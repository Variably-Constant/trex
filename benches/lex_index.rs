//! What a kept lex would save a repeated scan over a tree, measured before
//! anything is built on it.
//!
//! A scan of a file is read, lex, walk. The lex is the expensive middle and
//! it does not depend on the pattern, so a tree scanned again and again lexes
//! the same bytes to the same tokens every time. An index would lex once and
//! keep the tokens, and every later scan would read them instead.
//!
//! Whether that is worth having is an arithmetic question this bench answers
//! with measurements rather than with a guess, because the index is not free:
//! its tokens are wider than the bytes they describe, so every scan that
//! reads it reads MORE from the disk than the scan it replaces, and it only
//! pays if decoding those bytes is enough cheaper than lexing to cover the
//! difference. The four numbers that decide it:
//!
//! - what the lex costs per byte of input, which the index removes;
//! - what the index costs in bytes, which every later scan must read;
//! - what reading and decoding the index costs, which the index adds;
//! - what the walk costs, which happens either way and bounds the saving.
//!
//! The walk is held identical on both sides: the same pattern over the same
//! tokens through the same entry point, so the difference between the two
//! routes is the lex against the load and nothing else.

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use trex::token::{Token, TokenKind};

/// One file of the tree: its bytes, where they are, and where this file's
/// index is kept.
///
/// The index lives in a scratch directory of its own rather than beside the
/// file, so pointing the bench at a real tree cannot write into it.
struct Source {
    path: PathBuf,
    bytes: Vec<u8>,
    flat: PathBuf,
    packed: PathBuf,
}

/// `sources` with an index path each, under `store`.
fn with_index_paths(mut sources: Vec<Source>, store: &Path) -> Vec<Source> {
    std::fs::create_dir_all(store).unwrap_or_else(|e| panic!("create {}: {e}", store.display()));
    for (k, src) in sources.iter_mut().enumerate() {
        src.flat = store.join(format!("{k}.tokens"));
        src.packed = store.join(format!("{k}.packed"));
    }
    sources
}

/// Lines a tree's files are filled with: assignments, calls, records and
/// blocks, the shapes a configuration or a log holds.
fn corpus(bytes_wanted: usize, seed: usize) -> Vec<u8> {
    let mut s = String::new();
    let mut i = seed;
    while s.len() < bytes_wanted {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {i}) ;\n")),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{} ;\n", i + 1)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
        i += 1;
    }
    s.truncate(bytes_wanted);
    s.into_bytes()
}

/// Every file under `dir`, recursively, as sources read from where they
/// already are: the real-tree arm, which the synthetic corpus only stands in
/// for. A file holding a NUL is skipped, as a scan skips it.
fn real_tree(dir: &Path, out: &mut Vec<Source>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("entry under {}: {e}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            real_tree(&path, out);
        } else {
            // A file that cannot be read is named rather than skipped: the
            // corpus this builds is what every figure below is per-byte of, so
            // one dropped silently makes the rates wrong by an amount nothing
            // reports.
            let bytes =
                std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            if !bytes.contains(&0) {
                out.push(Source { path, bytes, flat: PathBuf::new(), packed: PathBuf::new() });
            }
        }
    }
}

/// A tree of `files` files of `each` bytes under `root`, written once.
fn tree(root: &Path, files: usize, each: usize) -> Vec<Source> {
    let mut out = Vec::with_capacity(files);
    for k in 0..files {
        let dir = root.join(format!("part{}", k % 8));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("create {}: {e}", dir.display()));
        let path = dir.join(format!("file{k}.txt"));
        let bytes = corpus(each, k * 1000);
        std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        out.push(Source { path, bytes, flat: PathBuf::new(), packed: PathBuf::new() });
    }
    out
}

/// One token as the index stores it: the kind's code, the span, and the
/// mate's index plus one, four little-endian words.
///
/// The code is [`TokenKind::code`], which is the encoding the crate already
/// defines and reads back with `from_code`, rather than the enum's own bytes:
/// a raw cast of the in-memory token would tie the file to one build's
/// layout, and the point of measuring the decode is to measure the decode a
/// real index could use.
const WORDS: usize = 4;
const TOKEN_BYTES: usize = WORDS * 4;

fn encode(toks: &[Token]) -> Vec<u8> {
    let mut out = Vec::with_capacity(toks.len() * TOKEN_BYTES);
    for t in toks {
        out.extend_from_slice(&t.kind.code().to_le_bytes());
        out.extend_from_slice(&t.start.to_le_bytes());
        out.extend_from_slice(&t.end.to_le_bytes());
        let mate = t.mate().map_or(0u32, |m| u32::try_from(m + 1).expect("a token index fits a word"));
        out.extend_from_slice(&mate.to_le_bytes());
    }
    out
}

fn decode(bytes: &[u8]) -> Vec<Token> {
    let mut out = Vec::with_capacity(bytes.len() / TOKEN_BYTES);
    for chunk in bytes.as_chunks::<TOKEN_BYTES>().0 {
        let word = |k: usize| {
            u32::from_le_bytes(
                chunk[k * 4..k * 4 + 4].try_into().expect("four bytes of a chunk sized in words"),
            )
        };
        let mut token = Token {
            kind: TokenKind::from_code(word(0)),
            start: word(1),
            end: word(2),
            mate_plus_one: None,
        };
        let mate = word(3);
        if mate > 0 {
            token.set_mate(Some(mate as usize - 1));
        }
        out.push(token);
    }
    out
}

/// The same tokens packed as tightly as their own shape allows: the kind's
/// code as two bytes, the token's length as two, and the mate as four.
///
/// A lex tiles the input - whitespace is a token, so every byte belongs to
/// one - which means a token's start is the one before it's end and no start
/// need be stored at all, which is half the flat form gone. The mate is
/// stored rather than rebuilt: a stack over the loaded stream pairs brackets
/// a real source file leaves unpaired differently from the lexer, and this
/// bench holds the packed form to giving the lexer's own tokens back, so a
/// cheaper pairing that is not the same pairing is not available to it.
///
/// This form is here because the flat form's size is the objection to it: if
/// the index is refused for being six times its input, the refusal has to be
/// measured against the smallest form that carries the same answer.
const PACKED_BYTES: usize = 8;

/// Whether `toks` tile `len` bytes end to end, which the packed form needs.
fn tiles(toks: &[Token], len: usize) -> bool {
    let mut at = 0u32;
    for t in toks {
        if t.start != at {
            return false;
        }
        at = t.end;
    }
    at as usize == len
}

fn pack(toks: &[Token]) -> Vec<u8> {
    let mut out = Vec::with_capacity(toks.len() * PACKED_BYTES);
    for t in toks {
        let code = u16::try_from(t.kind.code()).expect("a kind's code fits a half word");
        let len = u16::try_from(t.end - t.start).expect("a packed token is shorter than 64 kB");
        let mate = t.mate().map_or(0u32, |m| u32::try_from(m + 1).expect("a token index fits a word"));
        out.extend_from_slice(&code.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&mate.to_le_bytes());
    }
    out
}

/// The packed form read back, spans rebuilt by running sum over the lengths.
fn unpack(bytes: &[u8]) -> Vec<Token> {
    let mut out = Vec::with_capacity(bytes.len() / PACKED_BYTES);
    let mut at = 0u32;
    for chunk in bytes.as_chunks::<PACKED_BYTES>().0 {
        let half = |k: usize| {
            u16::from_le_bytes(chunk[k * 2..k * 2 + 2].try_into().expect("two bytes of a packed token"))
        };
        let end = at + u32::from(half(1));
        let mut token =
            Token { kind: TokenKind::from_code(u32::from(half(0))), start: at, end, mate_plus_one: None };
        let mate = u32::from_le_bytes(chunk[4..8].try_into().expect("four bytes of a packed token"));
        if mate > 0 {
            token.set_mate(Some(mate as usize - 1));
        }
        out.push(token);
        at = end;
    }
    out
}

/// The walk both routes share: the matches of `pattern` over tokens already
/// lexed, through the entry point a scan uses once its lex is done.
fn walk(pattern: &trex::ast::Pattern, input: &[u8], toks: &[Token]) -> usize {
    match trex::nfa::scan_nfa_over(pattern, input, toks) {
        Some(spans) => spans.len(),
        None => trex::engine::scan_tokens_from(pattern, input, toks, 0).len(),
    }
}

/// Each arm once a round, in an order that rotates, reduced by its median.
///
/// For rows that divide into each other. Timed one after another, a burst of
/// load during the first row is carried into every ratio the others are read
/// against; rotating puts each arm at each position instead, and the median
/// drops what lands on any one of them.
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

/// Time two routes against each other, alternating which runs first so the
/// warm-up bias lands on both rather than on whichever ran second.
fn time_pair(
    reps: u32,
    mut a: impl FnMut() -> usize,
    mut b: impl FnMut() -> usize,
) -> ((f64, f64, f64), (f64, f64, f64), f64) {
    black_box(a());
    black_box(b());
    let once = |f: &mut dyn FnMut() -> usize| {
        let t0 = Instant::now();
        black_box(f());
        t0.elapsed().as_secs_f64() * 1e3
    };
    let (mut sa, mut sb, mut ratios) = (Vec::new(), Vec::new(), Vec::new());
    for rep in 0..reps {
        let (ta, tb) = if rep % 2 == 0 {
            let ta = once(&mut a);
            (ta, once(&mut b))
        } else {
            let tb = once(&mut b);
            (once(&mut a), tb)
        };
        ratios.push(ta / tb);
        sa.push(ta);
        sb.push(tb);
    }
    let summarize = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        (v[v.len() / 2], v[0], v[v.len() - 1])
    };
    ratios.sort_by(f64::total_cmp);
    (summarize(sa), summarize(sb), ratios[ratios.len() / 2])
}

/// How many independent timings each cell takes, from `TREX_BENCH_REPS`.
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
    let root = std::env::temp_dir().join(format!("trex-lex-index-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap_or_else(|e| panic!("create {}: {e}", root.display()));
    // A path given on the command line is a real tree to measure instead of
    // the synthetic shapes: real text has its own token density, which is
    // what decides the index's size.
    let real = std::env::args().nth(1);

    // Patterns spread over what a scan of a tree actually asks: a kind
    // sequence the single-pass engine walks, a bound one it also walks, and
    // a balanced group only the set engine advances. All three need a lex,
    // which is the case an index is for; a byte-routable pattern never lexes
    // and an index would be dead weight to it.
    let patterns = [
        ("\\W \"=\" \\N", "a kind sequence"),
        ("\\W:x \"=\" \\N \";\" =x", "a bound sequence"),
        ("\\W \\B(\\W)", "a balanced group"),
    ];
    let shapes = [(64usize, 64 * 1024usize), (512, 8 * 1024)];

    println!("lex index: what a kept lex saves a repeated scan over a tree");
    println!("token: {TOKEN_BYTES} bytes stored, {} in memory", size_of::<Token>());
    println!();

    let arms: Vec<(String, Vec<Source>)> = match &real {
        Some(dir) => {
            let mut sources = Vec::new();
            real_tree(Path::new(dir), &mut sources);
            sources.sort_by(|a, b| a.path.cmp(&b.path));
            assert!(!sources.is_empty(), "{dir} holds no readable file");
            vec![(format!("{dir}, a real tree"), sources)]
        }
        None => shapes
            .iter()
            .map(|&(files, each)| (format!("{files} files of {} kB", each / 1024), tree(&root, files, each)))
            .collect(),
    };

    for (k, (what_tree, sources)) in arms.into_iter().enumerate() {
        let sources = with_index_paths(sources, &root.join(format!("index{k}")));
        let input_bytes: usize = sources.iter().map(|s| s.bytes.len()).sum();

        // The index, built once: every file lexed and its tokens written
        // beside it. This is the cost a tree pays before any scan reads one.
        let t0 = Instant::now();
        let mut index_bytes = 0usize;
        let mut packed_bytes = 0usize;
        let mut tokens = 0usize;
        let mut all_tile = true;
        for src in &sources {
            let toks = trex::lexer::lex(&src.bytes);
            let encoded = encode(&toks);
            std::fs::write(&src.flat, &encoded)
                .unwrap_or_else(|e| panic!("write {}: {e}", src.flat.display()));
            index_bytes += encoded.len();
            tokens += toks.len();
            all_tile &= tiles(&toks, src.bytes.len());
            let squeezed = pack(&toks);
            std::fs::write(&src.packed, &squeezed)
                .unwrap_or_else(|e| panic!("write {}: {e}", src.packed.display()));
            packed_bytes += squeezed.len();
            // The packed form must give the lexer's own tokens back, spans
            // and bracket pairing included, or its timing is of a different
            // answer.
            assert_eq!(unpack(&squeezed), toks, "the packed form must read back as the lex");
        }
        let build_ms = t0.elapsed().as_secs_f64() * 1e3;

        println!(
            "{what_tree}: {} files, {:.1} MB of input, {:.1} MB of index ({:.2}x), {tokens} tokens, {:.1} bytes of input a token",
            sources.len(),
            input_bytes as f64 / 1e6,
            index_bytes as f64 / 1e6,
            index_bytes as f64 / input_bytes as f64,
            input_bytes as f64 / tokens as f64,
        );
        println!(
            "  packed {:.1} MB ({:.2}x the input, {PACKED_BYTES} bytes a token); every file tiles: {all_tile}",
            packed_bytes as f64 / 1e6,
            packed_bytes as f64 / input_bytes as f64,
        );
        println!(
            "  both forms built in {build_ms:.1} ms ({:.0} MB/s of input)",
            input_bytes as f64 / 1e6 / (build_ms / 1e3)
        );

        // The two halves on their own, to say where a pass's time goes. Both
        // read the files from the page cache, which is where a repeated scan
        // of a tree reads them.
        // Read once a round in an order that rotates, because the two figures
        // below are divided by the first: timed one after another, whatever the
        // box was doing during the lex row becomes part of both ratios.
        let halves: Vec<Box<dyn Fn() -> usize>> = vec![
            Box::new(|| {
                let mut n = 0;
                for src in &sources {
                    let bytes = std::fs::read(&src.path).expect("read an input");
                    n += trex::lexer::lex(&bytes).len();
                }
                n
            }),
            Box::new(|| {
                let mut n = 0;
                for src in &sources {
                    let raw = std::fs::read(&src.flat).expect("read an index");
                    n += decode(&raw).len();
                }
                n
            }),
            Box::new(|| {
                let mut n = 0;
                for src in &sources {
                    let raw = std::fs::read(&src.packed).expect("read a packed index");
                    n += unpack(&raw).len();
                }
                n
            }),
        ];
        let taken = rotate(&halves, reps);
        let (lex_only, load_only, unpack_only) = (taken[0], taken[1], taken[2]);
        println!(
            "  read+lex {lex_only:.1} ms ({:.0} MB/s), read+decode {load_only:.1} ms ({:.2}x the lex), read+unpack {unpack_only:.1} ms ({:.2}x the lex)",
            input_bytes as f64 / 1e6 / (lex_only / 1e3),
            load_only / lex_only,
            unpack_only / lex_only,
        );

        for (src_pattern, what) in patterns {
            let pattern = trex::parse(src_pattern).expect("pattern parses");
            let (lexing, from_index, ratio) = time_pair(
                reps,
                || {
                    let mut n = 0;
                    for src in &sources {
                        let bytes = std::fs::read(&src.path).expect("read an input");
                        let toks = trex::lexer::lex(&bytes);
                        n += walk(&pattern, &bytes, &toks);
                    }
                    n
                },
                || {
                    let mut n = 0;
                    for src in &sources {
                        let bytes = std::fs::read(&src.path).expect("read an input");
                        let raw = std::fs::read(&src.flat).expect("read an index");
                        let toks = decode(&raw);
                        n += walk(&pattern, &bytes, &toks);
                    }
                    n
                },
            );
            // The two routes must agree on the matches, or the index is
            // answering a different question faster.
            let want: usize = sources
                .iter()
                .map(|s| walk(&pattern, &s.bytes, &trex::lexer::lex(&s.bytes)))
                .sum();
            let got: usize = sources
                .iter()
                .map(|s| {
                    let raw = std::fs::read(&s.flat).expect("read an index");
                    walk(&pattern, &s.bytes, &decode(&raw))
                })
                .sum();
            assert_eq!(want, got, "{src_pattern}: the index and the lex must report the same matches");
            let (_, from_packed, packed_ratio) = time_pair(
                reps,
                || {
                    let mut n = 0;
                    for src in &sources {
                        let bytes = std::fs::read(&src.path).expect("read an input");
                        let toks = trex::lexer::lex(&bytes);
                        n += walk(&pattern, &bytes, &toks);
                    }
                    n
                },
                || {
                    let mut n = 0;
                    for src in &sources {
                        let bytes = std::fs::read(&src.path).expect("read an input");
                        let raw = std::fs::read(&src.packed).expect("read a packed index");
                        let toks = unpack(&raw);
                        n += walk(&pattern, &bytes, &toks);
                    }
                    n
                },
            );
            let saved = lexing.0 - from_index.0;
            let passes = if saved > 0.0 { build_ms / saved } else { f64::INFINITY };
            println!(
                "  {what:<18} {src_pattern:<24} lex {:>7.1} ms  flat {:>7.1} ms ({ratio:.2}x)  packed {:>7.1} ms ({packed_ratio:.2}x)  {} matches; the flat build pays back after {}",
                lexing.0,
                from_index.0,
                from_packed.0,
                want,
                if passes.is_finite() { format!("{passes:.1} passes") } else { "never".to_string() },
            );
        }
        println!();
    }
    std::fs::remove_dir_all(&root).unwrap_or_else(|e| panic!("remove {}: {e}", root.display()));
}
