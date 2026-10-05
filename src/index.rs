//! An index over a tree that says which files a pattern cannot match, so a
//! scan never opens them.
//!
//! A scan of a tree spends most of its time on files that hold no match. trex
//! already refuses those cheaply - a required literal absent from the bytes
//! ends the file's scan, a pattern whose kinds the stream lacks fails fast -
//! but every one of those decisions is made from the file's bytes, so the
//! file is read and usually lexed before it is refused. A pattern led by a kind with no
//! literal to search for (`\I`, `\T`, `\{iban}`, `\N{>=1000}`) has no byte
//! route at all: without the index it lexes every file in the tree, though
//! almost none of them hold an address.
//!
//! This index keeps, for each file, a summary small enough to read for the
//! whole tree at once: which token kinds its lex made, a filter over the
//! words it holds, the range its numbers span and the range its timestamps
//! span. A pattern that needs a kind the file lacks, a word it does not
//! hold, a number larger than its largest or an instant later than its
//! latest cannot match it, and the file is skipped without being opened.
//! Measured over trex's own source (`benches/index_pruning.rs`): 50x for
//! `\E`, 23x for `\T`, 12x for `\I`, 9x for `\N{>=100000}`, 2x for a rare
//! word, and 0.99 to 1.02x where it prunes nothing, at 320 bytes a file.
//!
//! ## One-sidedness, which is the whole contract
//!
//! A summary may answer "cannot match" only when that is certain, and "might
//! match" in every other case. A wrong "might" costs one wasted read; a wrong
//! "cannot" is a missed match and a broken scan. So a kind mask is exact, a
//! word filter is a Bloom filter whose only error is a false present, the
//! ranges are exact, and every analysis of a pattern below is conservative:
//! where it cannot prove a file is needed it says it is.
//!
//! ## Staleness
//!
//! An entry records the file's size and modified time. A file that differs,
//! or that the index has never seen, is scanned normally: a stale index
//! slows a scan down and cannot change what it reports.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{Atom, Pattern};
use crate::token::TokenKind;

/// The file name of an index, at the root of the tree it covers.
pub const INDEX_FILE: &str = ".trex-index";

/// What the file begins with, so a file that is not an index, or is one an
/// older trex wrote, is refused rather than read as one.
const MAGIC: &[u8; 8] = b"trexidx\x02";

/// The number of 64-bit words in a kind mask: enough for every built-in
/// kind's code and the library and declared codes above them, which run to
/// 289.
const MASK_WORDS: usize = 5;

/// The bits in a summary's word filter, a power of two so a hash finds its
/// bit with a mask. 2048 bits is 256 bytes a file whatever the file's size;
/// a filter of that size answers most absences correctly, and the ones it
/// gets wrong cost a read and never a match.
const FILTER_BITS: usize = 2048;
const FILTER_WORDS: usize = FILTER_BITS / 64;

/// How many hashes a word sets in the filter.
const FILTER_HASHES: u64 = 4;

/// FNV-1a over a word, seeded, folded to ASCII lower case so a filter
/// answers a literal however it was capitalized.
fn hash(word: &[u8], seed: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for &b in word {
        h ^= u64::from(b.to_ascii_lowercase());
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// What one file's index holds.
///
/// Fixed size, and small: the summary is the same size however large the
/// file is, which is what lets a tree's index be read in one go.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// A bit per token kind the file's lex made.
    kinds: [u64; MASK_WORDS],
    /// A Bloom filter over the file's word and punctuation tokens.
    words: [u64; FILTER_WORDS],
    /// The least and greatest number the file holds.
    numbers: Option<(f64, f64)>,
    /// The earliest and latest instant the file holds, in seconds since the
    /// epoch.
    times: Option<(i64, i64)>,
}

/// The bytes one summary takes in the file: the mask, the filter, and each
/// range with a byte saying whether it is there.
const SUMMARY_BYTES: usize = MASK_WORDS * 8 + FILTER_WORDS * 8 + 1 + 16 + 1 + 16;

impl Summary {
    /// The summary of one file's bytes.
    ///
    /// The lex is the default one: a library kind is produced only for a
    /// pattern that names it, so the mask records the built-in kinds, and a
    /// pattern naming a library kind is pruned by the built-in kinds that
    /// kind is made of rather than by the kind itself.
    #[must_use]
    pub fn of(input: &[u8]) -> Summary {
        let mut kinds = [0u64; MASK_WORDS];
        let mut words = [0u64; FILTER_WORDS];
        let mut numbers: Option<(f64, f64)> = None;
        let mut times: Option<(i64, i64)> = None;
        let clock = crate::typed::Clock::current();
        for t in crate::lexer::lex(input) {
            let code = t.kind.code() as usize;
            if code < MASK_WORDS * 64 {
                kinds[code / 64] |= 1 << (code % 64);
            }
            let text = &input[t.start as usize..t.end as usize];
            match t.kind {
                TokenKind::Word | TokenKind::Punct => {
                    for seed in 0..FILTER_HASHES {
                        let bit = (hash(text, seed) as usize) % FILTER_BITS;
                        words[bit / 64] |= 1 << (bit % 64);
                    }
                }
                TokenKind::Number => {
                    // The float nearest each value: rounding never reverses
                    // an order, so the range of these floats holds the float
                    // of every value. A number with no value read here widens
                    // the range to every float rather than going unrecorded,
                    // since a range that left it out could refuse its file.
                    let value = match std::str::from_utf8(text) {
                        Ok(s) => crate::typed::number_value(s),
                        Err(_not_utf8) => None,
                    };
                    let (low, high) = match value {
                        Some(d) => (d.nearest_f64(), d.nearest_f64()),
                        None => (f64::NEG_INFINITY, f64::INFINITY),
                    };
                    numbers = Some(match numbers {
                        None => (low, high),
                        Some((lo, hi)) => (lo.min(low), hi.max(high)),
                    });
                }
                TokenKind::Timestamp => {
                    if let Some(secs) = std::str::from_utf8(text)
                        .ok()
                        .and_then(crate::typed::parse_civil)
                        .and_then(|c| c.epoch(clock))
                        .map(|(secs, _)| secs)
                    {
                        times = Some(match times {
                            None => (secs, secs),
                            Some((lo, hi)) => (lo.min(secs), hi.max(secs)),
                        });
                    }
                }
                _ => {}
            }
        }
        Summary { kinds, words, numbers, times }
    }

    /// Whether the file holds a token of `kind`. A code past the mask counts
    /// as present, since a summary that cannot record a kind must not refuse
    /// it.
    #[must_use]
    pub fn has_kind(&self, kind: TokenKind) -> bool {
        let code = kind.code() as usize;
        code >= MASK_WORDS * 64 || self.kinds[code / 64] & (1 << (code % 64)) != 0
    }

    /// Whether the file might hold `word`; false only where it certainly
    /// does not.
    #[must_use]
    pub fn might_hold(&self, word: &str) -> bool {
        (0..FILTER_HASHES).all(|seed| {
            let bit = (hash(word.as_bytes(), seed) as usize) % FILTER_BITS;
            self.words[bit / 64] & (1 << (bit % 64)) != 0
        })
    }

    /// Whether the file might hold a number at or above `least`: its largest
    /// number is at least `least`, or it holds no number at all, which
    /// leaves the refusal to the kind mask.
    #[must_use]
    pub fn any_number_from(&self, least: f64) -> bool {
        self.numbers.is_none_or(|(_, hi)| hi >= least)
    }

    /// Whether the file holds an instant at or after `earliest`.
    #[must_use]
    pub fn any_instant_from(&self, earliest: i64) -> bool {
        self.times.is_none_or(|(_, hi)| hi >= earliest)
    }

    fn write_into(&self, out: &mut Vec<u8>) {
        for w in self.kinds {
            out.extend_from_slice(&w.to_le_bytes());
        }
        for w in self.words {
            out.extend_from_slice(&w.to_le_bytes());
        }
        match self.numbers {
            Some((lo, hi)) => {
                out.push(1);
                out.extend_from_slice(&lo.to_le_bytes());
                out.extend_from_slice(&hi.to_le_bytes());
            }
            None => {
                out.push(0);
                out.extend_from_slice(&[0u8; 16]);
            }
        }
        match self.times {
            Some((lo, hi)) => {
                out.push(1);
                out.extend_from_slice(&lo.to_le_bytes());
                out.extend_from_slice(&hi.to_le_bytes());
            }
            None => {
                out.push(0);
                out.extend_from_slice(&[0u8; 16]);
            }
        }
    }

    fn read_from(bytes: &[u8]) -> Option<Summary> {
        if bytes.len() < SUMMARY_BYTES {
            return None;
        }
        let word = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().ok().unwrap_or([0; 8]));
        let mut kinds = [0u64; MASK_WORDS];
        for (k, slot) in kinds.iter_mut().enumerate() {
            *slot = word(k * 8);
        }
        let mut words = [0u64; FILTER_WORDS];
        let base = MASK_WORDS * 8;
        for (k, slot) in words.iter_mut().enumerate() {
            *slot = word(base + k * 8);
        }
        let at = base + FILTER_WORDS * 8;
        let pair = |at: usize| (f64::from_bits(word(at)), f64::from_bits(word(at + 8)));
        let numbers = (bytes[at] == 1).then(|| pair(at + 1));
        let at = at + 17;
        #[allow(clippy::cast_possible_wrap)]
        let times = (bytes[at] == 1).then(|| (word(at + 1) as i64, word(at + 9) as i64));
        Some(Summary { kinds, words, numbers, times })
    }
}

/// One file's entry: what it was when the index was built, and its summary.
#[derive(Clone, Debug)]
struct Entry {
    size: u64,
    /// The file's modified time in nanoseconds since the epoch, or `0` where
    /// the filesystem would not say.
    modified: i64,
    summary: Summary,
}

/// The index of one tree, as it is held while a scan consults it.
///
/// A file is keyed by its path under the tree's root rather than as the
/// walk spelled it, so an index built from one directory answers a scan run
/// from another. The root itself is where the index was found and is not in
/// the file.
#[derive(Clone, Debug, Default)]
pub struct Index {
    root: PathBuf,
    entries: HashMap<PathBuf, Entry>,
}

/// What a file was when a scan met it, for the staleness check: its length
/// and its modified time.
///
/// The time is in nanoseconds rather than seconds because a file rewritten
/// to the same length within one tick of its recorded time reads as
/// unchanged. At seconds that tick is a whole second, and an editor saving
/// twice or a test rewriting a file it has just indexed easily happens within
/// it; at nanoseconds it is the filesystem's own granularity, the narrowest
/// this check can be.
fn stat(path: &Path) -> Option<(u64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
    Some((meta.len(), modified))
}

impl Index {
    /// An empty index for the tree at `root`, to be filled as a scan reads
    /// its files.
    #[must_use]
    pub fn under(root: &Path) -> Index {
        Index { root: root.to_path_buf(), entries: HashMap::new() }
    }

    /// An index over the files at `paths`, each read and summarized, keyed
    /// under `root`.
    ///
    /// A file that cannot be read is left out rather than refused: a file
    /// with no entry is scanned, which is what an index says when it does not
    /// know.
    #[must_use]
    pub fn build(root: &Path, paths: &[PathBuf]) -> Index {
        let mut index = Index::under(root);
        for path in paths {
            if let Some(entry) = Self::entry_of(path) {
                index.insert(path, entry);
            }
        }
        index
    }

    fn entry_of(path: &Path) -> Option<Entry> {
        let (size, modified) = stat(path)?;
        let bytes = std::fs::read(path).ok()?;
        Some(Entry { size, modified, summary: Summary::of(&bytes) })
    }

    /// `path` as this index keys it: under the root, or as given where the
    /// root is empty or does not contain it.
    fn key(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root).unwrap_or(path).to_path_buf()
    }

    fn insert(&mut self, path: &Path, entry: Entry) {
        let key = self.key(path);
        self.entries.insert(key, entry);
    }

    /// Record `path`'s summary, computed from bytes a scan has already read,
    /// so an index built during a scan costs no second read.
    pub fn observe(&mut self, path: &Path, bytes: &[u8]) {
        if let Some((size, modified)) = stat(path) {
            self.insert(path, Entry { size, modified, summary: Summary::of(bytes) });
        }
    }

    /// The root this index is keyed under.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// How many files the index covers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it covers none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The summary for `path`, where the index holds one and the file is as
    /// it was when the entry was made. A file that has changed, or that the
    /// index has never seen, has no summary here and is scanned.
    #[must_use]
    pub fn summary_of(&self, path: &Path) -> Option<&Summary> {
        let entry = self.entries.get(&self.key(path))?;
        let (size, modified) = stat(path)?;
        (entry.size == size && entry.modified == modified).then_some(&entry.summary)
    }

    /// Whether `pattern` certainly cannot match `path`, by the index alone.
    ///
    /// False whenever the index does not know: a file it has not seen, one
    /// that has changed, or a pattern whose needs it cannot read.
    #[must_use]
    pub fn refuses(&self, pattern: &Pattern, path: &Path) -> bool {
        self.refuses_every(&[pattern], path)
    }

    /// Whether every one of `patterns` certainly cannot match `path`, which
    /// is when a scan of a set or a rule file may skip it: one member that
    /// might match is reason enough to open the file.
    #[must_use]
    pub fn refuses_every(&self, patterns: &[&Pattern], path: &Path) -> bool {
        if patterns.is_empty() {
            return false;
        }
        let needs: Vec<Needs> = patterns.iter().map(|p| Needs::of(p)).collect();
        if !needs.iter().all(Needs::are_any) {
            return false;
        }
        self.summary_of(path).is_some_and(|s| needs.iter().all(|n| n.refused_by(s)))
    }

    /// The index written as bytes: the magic, then one record per file with
    /// its path, size, modified time and summary.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAGIC.len() + 8 + self.entries.len() * (SUMMARY_BYTES + 64));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&u64::try_from(self.entries.len()).unwrap_or(0).to_le_bytes());
        // Written in path order, so an index of one tree is the same bytes
        // whatever order the walk met its files in.
        let mut paths: Vec<&PathBuf> = self.entries.keys().collect();
        paths.sort();
        for path in paths {
            let entry = &self.entries[path];
            let name = path.to_string_lossy();
            out.extend_from_slice(&u32::try_from(name.len()).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&entry.size.to_le_bytes());
            out.extend_from_slice(&entry.modified.to_le_bytes());
            entry.summary.write_into(&mut out);
        }
        out
    }

    /// The index read from `bytes`, or `None` for bytes that are not an
    /// index.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Index> {
        if bytes.len() < MAGIC.len() + 8 || &bytes[..MAGIC.len()] != MAGIC {
            return None;
        }
        let count = u64::from_le_bytes(bytes[MAGIC.len()..MAGIC.len() + 8].try_into().ok()?);
        let mut at = MAGIC.len() + 8;
        let mut entries = HashMap::with_capacity(usize::try_from(count).unwrap_or(0));
        for _ in 0..count {
            if at + 4 > bytes.len() {
                return None;
            }
            let len = u32::from_le_bytes(bytes[at..at + 4].try_into().ok()?) as usize;
            at += 4;
            if at + len + 16 + SUMMARY_BYTES > bytes.len() {
                return None;
            }
            let path = PathBuf::from(String::from_utf8_lossy(&bytes[at..at + len]).into_owned());
            at += len;
            let size = u64::from_le_bytes(bytes[at..at + 8].try_into().ok()?);
            at += 8;
            let modified = i64::from_le_bytes(bytes[at..at + 8].try_into().ok()?);
            at += 8;
            let summary = Summary::read_from(&bytes[at..])?;
            at += SUMMARY_BYTES;
            entries.insert(path, Entry { size, modified, summary });
        }
        Some(Index { root: PathBuf::new(), entries })
    }

    /// Write the index to [`INDEX_FILE`] under `root`.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        std::fs::write(root.join(INDEX_FILE), self.to_bytes())
    }

    /// The index at [`INDEX_FILE`] under `root`, or `None` where there is
    /// none or it is not one this trex wrote.
    #[must_use]
    pub fn load(root: &Path) -> Option<Index> {
        let mut index = Index::from_bytes(&std::fs::read(root.join(INDEX_FILE)).ok()?)?;
        index.root = root.to_path_buf();
        Some(index)
    }
}

/// What must be true of a file before a pattern is worth opening it.
///
/// Every field is a conservative reading of the pattern: a requirement is
/// recorded only where every match of the pattern must meet it, so a file
/// the needs refuse holds no match.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Needs {
    /// The token kinds every match includes.
    pub kinds: Vec<TokenKind>,
    /// The literal texts every match includes.
    pub words: Vec<String>,
    /// The least number a match's tokens must reach.
    pub least_number: Option<f64>,
    /// The earliest instant a match's timestamps must reach.
    pub earliest_instant: Option<i64>,
}

impl Needs {
    /// What a file must hold for `pattern` to match it.
    #[must_use]
    pub fn of(pattern: &Pattern) -> Needs {
        let mut needs = Needs::default();
        walk(pattern, &mut needs);
        needs
    }

    /// Whether `summary` refuses this outright, which is the only answer an
    /// index is allowed to give with certainty.
    #[must_use]
    pub fn refused_by(&self, summary: &Summary) -> bool {
        self.kinds.iter().any(|&k| !summary.has_kind(k))
            || self.words.iter().any(|w| !summary.might_hold(w))
            || self.least_number.is_some_and(|n| !summary.any_number_from(n))
            || self.earliest_instant.is_some_and(|t| !summary.any_instant_from(t))
    }

    /// Whether anything here can refuse a file, so a caller knows whether
    /// consulting the index is worth the lookup.
    #[must_use]
    pub fn are_any(&self) -> bool {
        !self.kinds.is_empty()
            || !self.words.is_empty()
            || self.least_number.is_some()
            || self.earliest_instant.is_some()
    }
}

/// Add what `pattern` requires to `needs`.
///
/// Only the constructs every match must pass through contribute: a
/// concatenation requires what each of its parts requires, a binding, a
/// field and a group require what they wrap, and a repetition requires its
/// inner pattern only where it must run at least once, which is `P+` and
/// `P{m,n}` with `m` above zero but not `P*` or `P?`. An alternation
/// requires nothing, since a match may take the branch that needs least; a
/// lookaround and a guard require nothing, since they constrain without
/// consuming and the text they read may be outside the match. Anything not
/// understood requires nothing, which is the safe answer.
fn walk(pattern: &Pattern, needs: &mut Needs) {
    match pattern {
        Pattern::Atom(atom) => atom_needs(atom, needs),
        Pattern::Concat(parts) => parts.iter().for_each(|p| walk(p, needs)),
        Pattern::Bind(_, _, inner)
        | Pattern::Balanced(_, inner)
        | Pattern::Field(_, inner)
        | Pattern::Plus(inner, _)
        | Pattern::Atomic(inner) => walk(inner, needs),
        Pattern::Repeat(inner, lo, _, _) if *lo > 0 => walk(inner, needs),
        _ => {}
    }
}

/// Add what one atom requires.
fn atom_needs(atom: &Atom, needs: &mut Needs) {
    let mut kind = |k: TokenKind| {
        // Whitespace is in every file worth scanning and a custom kind is
        // not in the default lex, so neither prunes.
        if !matches!(k, TokenKind::Whitespace | TokenKind::Custom(_)) && !needs.kinds.contains(&k) {
            needs.kinds.push(k);
        }
    };
    match atom {
        Atom::Kind(k) => kind(*k),
        Atom::KindMag(k, _) => kind(*k),
        Atom::KindPred(k, pred) => {
            kind(*k);
            if let Some(least) = pred.least_value() {
                needs.least_number = Some(needs.least_number.map_or(least, |n: f64| n.max(least)));
            }
            if *k == TokenKind::Timestamp
                && let Some(earliest) = pred.earliest(crate::typed::Clock::current())
            {
                needs.earliest_instant =
                    Some(needs.earliest_instant.map_or(earliest, |t: i64| t.max(earliest)));
            }
        }
        // A literal is a word the file must hold, but only under the
        // identity orbit: under a symmetry the text in the file may be
        // spelled differently from the text in the pattern.
        Atom::Literal(text, orbit)
            if *orbit == crate::orbit::OrbitGroup::Identity && !needs.words.contains(text) =>
        {
            needs.words.push(text.clone());
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(text: &str) -> Summary {
        Summary::of(text.as_bytes())
    }

    #[test]
    fn a_summary_records_the_kinds_words_and_ranges_of_its_file() {
        let s = summary("host = 10.0.0.1 port 8080 at 2026-09-15T10:00:00Z");
        assert!(s.has_kind(TokenKind::Ip));
        assert!(s.has_kind(TokenKind::Number));
        assert!(s.has_kind(TokenKind::Timestamp));
        assert!(!s.has_kind(TokenKind::Email));
        assert!(s.might_hold("host") && s.might_hold("port"));
        assert!(!s.might_hold("nowhere_at_all_xyzzy"));
        assert!(s.any_number_from(8080.0) && !s.any_number_from(99999.0));
    }

    #[test]
    fn what_a_pattern_needs_is_what_every_match_must_hold() {
        let needs = |src: &str| Needs::of(&crate::parse(src).expect("pattern parses"));
        assert_eq!(needs("\\I").kinds, [TokenKind::Ip]);
        assert_eq!(needs("\\W \"=\" \\N").words, ["="]);
        assert!(needs("\\W \"=\" \\N").kinds.contains(&TokenKind::Number));
        // An alternation may take the branch that needs least, so it needs
        // nothing; a repeat that may run zero times likewise.
        assert!(!needs("\\I | \\E").are_any());
        assert!(!needs("\\I*").are_any());
        assert_eq!(needs("\\I+").kinds, [TokenKind::Ip]);
        assert_eq!(needs("\\N{>=1000}").least_number, Some(1000.0));
    }

    #[test]
    fn a_summary_refuses_only_what_cannot_match() {
        let text = "host = 10.0.0.1 port 8080";
        let s = summary(text);
        let refuses = |src: &str| {
            let p = crate::parse(src).expect("pattern parses");
            let refused = Needs::of(&p).refused_by(&s);
            // Whatever the index says, it must never refuse a file that
            // matches: that is the one error it is not allowed.
            if refused {
                assert!(crate::scan(&p, text.as_bytes()).is_empty(), "{src} was refused but matches");
            }
            refused
        };
        assert!(refuses("\\E"));
        assert!(refuses("\\T"));
        assert!(refuses("\"nowhere_at_all_xyzzy\""));
        assert!(refuses("\\N{>=100000}"));
        assert!(!refuses("\\I"));
        assert!(!refuses("\\W \"=\" \\N"));
    }

    #[test]
    fn an_index_reads_back_as_it_was_written() {
        let dir = std::env::temp_dir().join(format!("trex/index-roundtrip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the temp dir");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "from 10.0.0.1 now").expect("write a");
        std::fs::write(&b, "nothing here at all").expect("write b");
        let index = Index::build(&dir, &[a.clone(), b.clone()]);
        assert_eq!(index.len(), 2);
        index.save(&dir).expect("save the index");
        let read = Index::load(&dir).expect("the index reads back");
        assert_eq!(read.len(), 2);
        assert_eq!(read.summary_of(&a), index.summary_of(&a));
        let ip = crate::parse("\\I").expect("pattern parses");
        assert!(!read.refuses(&ip, &a));
        assert!(read.refuses(&ip, &b));
        // A file that has changed since the index was built is not refused.
        std::fs::write(&b, "from 10.0.0.2 now").expect("rewrite b");
        assert!(!read.refuses(&ip, &b));
        assert!(Index::from_bytes(b"not an index at all").is_none());
        std::fs::remove_dir_all(&dir).expect("remove the temp dir");
    }
}
