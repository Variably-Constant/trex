//! What an index over a tree saves when it decides which files to open,
//! rather than how fast an opened file is scanned.
//!
//! The companion bench, `lex_index`, measures keeping the lexer's tokens and
//! loading them instead of lexing. Its answer is a ceiling of about 1.4x and
//! usually a loss, and the reason is structural: a scan must read the input
//! whatever it stores beside it, because literals and typed predicates
//! compare against the bytes, so an indexed scan reads the input AND the
//! index and can only ever save the lex, which is under a third of a pass.
//!
//! This bench asks the other question. Over a tree, the expensive thing is
//! not one file's scan but the files scanned for nothing. trex prunes well
//! already - a required literal absent from the bytes refuses a file, a
//! pattern whose kinds the stream lacks fails fast - but every one of those
//! decisions is made FROM THE FILE'S BYTES, so the file is read first. An
//! index that answers "can this pattern match here?" from a few bytes held
//! per file skips the read, the lex and the walk together, and what it saves
//! is not a ratio on one pass but the fraction of the tree that never had to
//! be opened.
//!
//! The summary must be one-sided. It may answer "cannot match" only when
//! that is certain, and "might match" in every other case: a wrong "might"
//! costs one wasted read, a wrong "cannot" is a missed match and a broken
//! scan. A kind mask is exact - a kind absent from the lex is absent. A
//! Bloom filter over the file's words is one-sided in the safe direction. A
//! numeric range is exact. Every pruned run below is checked against the
//! unpruned one for the same matches, which is the only way to hold that.

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use trex::token::TokenKind;

/// The bits a kind mask holds: every built-in kind's code, and the library
/// and declared codes above them, which run to 288.
const MASK_WORDS: usize = 5;

/// The bits of the word filter a summary carries, a power of two so the
/// index is a mask. 2048 bits is 256 bytes a file whatever the file's size,
/// and a filter that full answers most absences; the ones it gets wrong cost
/// a read and never a match.
const FILTER_BITS: usize = 2048;
const FILTER_WORDS: usize = FILTER_BITS / 64;

/// What one file's index holds: which kinds its lex made, a filter over the
/// words it holds, and the range its numbers span.
///
/// Fixed size, and small: the whole summary is 296 bytes however large the
/// file is, where a kept token stream is one and a half times the file.
#[derive(Clone)]
struct Summary {
    kinds: [u64; MASK_WORDS],
    words: [u64; FILTER_WORDS],
    numbers: Option<(f64, f64)>,
}

const SUMMARY_BYTES: usize = MASK_WORDS * 8 + FILTER_WORDS * 8 + 8 + 2 * 8;

/// FNV-1a over a word, seeded, for the filter's hashes.
fn hash(word: &[u8], seed: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for &b in word {
        h ^= u64::from(b.to_ascii_lowercase());
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl Summary {
    /// The summary of one file, from its lex.
    fn of(input: &[u8]) -> Summary {
        let mut kinds = [0u64; MASK_WORDS];
        let mut words = [0u64; FILTER_WORDS];
        let mut numbers: Option<(f64, f64)> = None;
        for t in trex::lexer::lex(input) {
            let code = t.kind.code() as usize;
            if code < MASK_WORDS * 64 {
                kinds[code / 64] |= 1 << (code % 64);
            }
            let text = &input[t.start as usize..t.end as usize];
            match t.kind {
                TokenKind::Word | TokenKind::Punct => {
                    for seed in 0..4u64 {
                        let bit = (hash(text, seed) as usize) % FILTER_BITS;
                        words[bit / 64] |= 1 << (bit % 64);
                    }
                }
                TokenKind::Number => {
                    if let Ok(v) = std::str::from_utf8(text).unwrap_or("").parse::<f64>() {
                        numbers = Some(match numbers {
                            None => (v, v),
                            Some((lo, hi)) => (lo.min(v), hi.max(v)),
                        });
                    }
                }
                _ => {}
            }
        }
        Summary { kinds, words, numbers }
    }

    /// Whether the file holds a token of `kind`.
    fn has_kind(&self, kind: TokenKind) -> bool {
        let code = kind.code() as usize;
        code >= MASK_WORDS * 64 || self.kinds[code / 64] & (1 << (code % 64)) != 0
    }

    /// Whether the file might hold `word`. False only where it certainly
    /// does not.
    fn might_hold(&self, word: &str) -> bool {
        (0..4u64).all(|seed| {
            let bit = (hash(word.as_bytes(), seed) as usize) % FILTER_BITS;
            self.words[bit / 64] & (1 << (bit % 64)) != 0
        })
    }

    /// Whether the file holds a number at or above `least`.
    fn any_number_at_least(&self, least: f64) -> bool {
        self.numbers.is_some_and(|(_, hi)| hi >= least)
    }
}

/// What a pattern needs of a file before it is worth opening: the kinds its
/// stream must carry, the words it must hold, and the number it must reach.
///
/// Declared here per pattern rather than derived from the AST, because this
/// bench measures what such an analysis would be WORTH; deriving it is the
/// work the measurement either earns or does not.
struct Needs {
    kinds: &'static [TokenKind],
    words: &'static [&'static str],
    least_number: Option<f64>,
}

impl Needs {
    /// Whether `s` refuses this pattern outright, which is the only answer
    /// the index is allowed to give with certainty.
    fn refused_by(&self, s: &Summary) -> bool {
        self.kinds.iter().any(|&k| !s.has_kind(k))
            || self.words.iter().any(|w| !s.might_hold(w))
            || self.least_number.is_some_and(|n| !s.any_number_at_least(n))
    }
}

/// Every file under `dir`, recursively, with its bytes.
fn tree(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("entry under {}: {e}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            tree(&path, out);
        } else {
            // A file present but unreadable fails the walk, as the directory
            // read and each entry above do. A corpus quietly missing the files
            // it could not open is a corpus smaller than the one reported.
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            if !bytes.contains(&0) {
                out.push((path, bytes));
            }
        }
    }
}

/// Each arm once a round, in an order that rotates, reduced by its median.
///
/// For rows that divide into each other. Taking one arm's repeats together and
/// then the other's puts a burst of load entirely into whichever arm it landed
/// on, and from there into the saving, which is the only figure this bench
/// exists to report.
fn rotate<'a>(arms: &[Box<dyn Fn() -> usize + 'a>], reps: u32) -> Vec<f64> {
    for f in arms {
        black_box(f());
    }
    let mut taken: Vec<Vec<f64>> = vec![Vec::new(); arms.len()];
    for rep in 0..reps as usize {
        for step in 0..arms.len() {
            let i = (step + rep) % arms.len();
            let t0 = Instant::now();
            black_box(arms[i]());
            taken[i].push(t0.elapsed().as_secs_f64() * 1e3);
        }
    }
    taken
        .into_iter()
        .map(|mut v| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        })
        .collect()
}


fn reps() -> u32 {
    match std::env::var("TREX_BENCH_REPS") {
        Err(std::env::VarError::NotPresent) => 9,
        Err(e) => panic!("TREX_BENCH_REPS is set but unreadable: {e}"),
        Ok(v) => v
            .parse::<u32>()
            .unwrap_or_else(|e| panic!("TREX_BENCH_REPS is set to {v:?}, not a count: {e}")),
    }
}

fn main() {
    let reps = reps();
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let mut files = Vec::new();
    tree(Path::new(&root), &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!files.is_empty(), "{root} holds no readable file");
    let total_bytes: usize = files.iter().map(|(_, b)| b.len()).sum();

    // The index, built once over the tree.
    let t0 = Instant::now();
    let summaries: Vec<Summary> = files.iter().map(|(_, b)| Summary::of(b)).collect();
    let build_ms = t0.elapsed().as_secs_f64() * 1e3;
    let index_bytes = summaries.len() * SUMMARY_BYTES;

    println!("index pruning: what a summary a file wide saves a scan of a tree");
    println!(
        "{}: {} files, {:.1} MB, {:.1} kB of index ({:.4}x the tree, {SUMMARY_BYTES} bytes a file), built in {build_ms:.0} ms",
        root,
        files.len(),
        total_bytes as f64 / 1e6,
        index_bytes as f64 / 1e3,
        index_bytes as f64 / total_bytes as f64,
    );
    println!();
    println!(
        "{:<26} {:>7} {:>8} {:>9} {:>9} {:>7}  what it is",
        "pattern", "files", "opened", "scan ms", "pruned ms", "gain"
    );

    let cases: &[(&str, Needs, &str)] = &[
        // Kind-led, with no literal to search for: today every file must be
        // read and lexed to learn it has no timestamp.
        ("\\T", Needs { kinds: &[TokenKind::Timestamp], words: &[], least_number: None }, "a timestamp"),
        ("\\I", Needs { kinds: &[TokenKind::Ip], words: &[], least_number: None }, "an address"),
        (
            "\\E",
            Needs { kinds: &[TokenKind::Email], words: &[], least_number: None },
            "an email",
        ),
        // A typed predicate over a kind the tree does hold: the range prunes
        // where the kind alone would not.
        (
            "\\N{>=100000}",
            Needs { kinds: &[TokenKind::Number], words: &[], least_number: Some(100_000.0) },
            "a large number",
        ),
        // Literal-led: trex already refuses these from the bytes, so the
        // index's saving here is the read itself.
        (
            "\"unsafe\" \\W",
            Needs { kinds: &[TokenKind::Word], words: &["unsafe"], least_number: None },
            "a rare word",
        ),
        (
            "\"fn\" \\W \\B(.*)",
            Needs { kinds: &[TokenKind::Word], words: &["fn"], least_number: None },
            "a common word",
        ),
        // The control: present everywhere, so the index can prune nothing
        // and its cost is all that is measured.
        (
            "\\W \"=\" \\N",
            Needs { kinds: &[TokenKind::Word, TokenKind::Number], words: &["="], least_number: None },
            "everywhere (control)",
        ),
    ];

    for (src, needs, what) in cases {
        let pattern = trex::parse(src).expect("pattern parses");
        let opened = files.iter().zip(&summaries).filter(|(_, s)| !needs.refused_by(s)).count();

        // The two arms alternate which runs first, because the saving printed
        // below is one divided by the other: run in a fixed order, a burst of
        // load during either one becomes the saving.
        let arms: [Box<dyn Fn() -> usize>; 2] = [
            Box::new(|| {
                let mut n = 0;
                for (path, _) in &files {
                    let bytes = std::fs::read(path).expect("read an input");
                    n += trex::scan(&pattern, &bytes).len();
                }
                n
            }),
            Box::new(|| {
                let mut n = 0;
                for ((path, _), summary) in files.iter().zip(&summaries) {
                    if needs.refused_by(summary) {
                        continue;
                    }
                    let bytes = std::fs::read(path).expect("read an input");
                    n += trex::scan(&pattern, &bytes).len();
                }
                n
            }),
        ];
        let taken = rotate(&arms, reps);
        let (plain, pruned) = (taken[0], taken[1]);

        // The prune must not lose a match: the whole contract is that a file
        // it refuses could not have matched.
        let want: usize = files.iter().map(|(_, b)| trex::scan(&pattern, b).len()).sum();
        let got: usize = files
            .iter()
            .zip(&summaries)
            .filter(|(_, s)| !needs.refused_by(s))
            .map(|((_, b), _)| trex::scan(&pattern, b).len())
            .sum();
        assert_eq!(want, got, "{src}: a pruned file held {} matches the index refused", want - got);

        println!(
            "{src:<26} {:>7} {:>8} {plain:>9.1} {pruned:>9.1} {:>6.2}x  {what}, {want} matches",
            files.len(),
            opened,
            plain / pruned,
        );
    }
}
