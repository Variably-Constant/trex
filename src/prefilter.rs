//! Byte-grain presence prefilter: approximate-membership filters over
//! the n-grams of a corpus, answering "might this literal occur here?"
//! with one-sided error and no false negatives.
//!
//! The content guard `~"lit"` asks whether a literal occurs in the
//! forward window. A positional scan answers it in time proportional
//! to the window, and the guard is checked at many positions, so an
//! absent literal costs the whole window once per position. A
//! membership filter built once over the input collapses every later
//! check to a few hash probes: if any of the literal's n-grams is
//! absent from the corpus, the literal cannot occur, and the scan
//! short-circuits without touching the window again.
//!
//! Three filters share the contract, matching the literature they come
//! from: a Bloom filter (Bloom 1970), a Cuckoo filter (Fan, Andersen,
//! Kaminsky, Mitzenmacher 2014), and an Xor filter (Graf and Lemire
//! 2020). All three answer membership with no false negatives; they
//! trade space against false-positive rate and construction cost. The
//! engine wires the Bloom filter (cheapest to build per scan); the
//! Cuckoo and Xor filters serve the static-corpus case, where one
//! build amortizes over many queries.
//!
//! ## No false negatives, end to end
//!
//! A filter answers an n-gram query with possible false positives but
//! never a false negative: a present n-gram always reads as present.
//! A literal that truly occurs in the corpus has every one of its
//! n-grams in the corpus, so each reads as present, so the literal
//! reads as "might occur". The reverse, "all n-grams present" implies
//! "literal occurs", does not hold (the n-grams may come from
//! different places), which is the permitted false positive: the exact
//! scan resolves it.

use std::collections::HashSet;

/// The n-gram width. Four bytes is long enough that a random four-byte
/// window rarely collides across unrelated text, and short enough that
/// most useful literals decompose into several of them.
pub const NGRAM: usize = 4;

/// A 64-bit hash of a byte slice (SplitMix64 finalizer over a
/// multiply-accumulate). Deterministic, so a build and a query agree
/// across runs and across machines.
fn hash_bytes(bytes: &[u8], seed: u64) -> u64 {
    let mut z = seed ^ 0x9E37_79B9_7F4A_7C15;
    for &b in bytes {
        z = (z ^ u64::from(b)).wrapping_mul(0x1000_0000_01B3);
    }
    // SplitMix64 finalizer.
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Lemire's fast range reduction: map a 32-bit value into `[0, n)`
/// without a division.
fn reduce(x: u32, n: usize) -> usize {
    ((u64::from(x) * n as u64) >> 32) as usize
}

/// The distinct n-gram hashes of a corpus; every filter is built from this
/// key set. A literal shorter than one n-gram cannot be decomposed,
/// so it is never rejected (the filter returns "might occur" and the
/// exact scan decides).
fn ngram_keys(corpus: &[u8]) -> Vec<u64> {
    if corpus.len() < NGRAM {
        return Vec::new();
    }
    let mut set: HashSet<u64> = HashSet::new();
    for w in corpus.windows(NGRAM) {
        set.insert(hash_bytes(w, 0));
    }
    set.into_iter().collect()
}

/// Whether every n-gram of `literal` is a key in `present`. A literal
/// shorter than the n-gram width returns `true` (cannot be rejected).
fn all_ngrams_present(literal: &[u8], present: impl Fn(u64) -> bool) -> bool {
    if literal.len() < NGRAM {
        return true;
    }
    literal.windows(NGRAM).all(|w| present(hash_bytes(w, 0)))
}

/// The membership query every filter exposes: whether `literal` might
/// occur in the filter's corpus. `false` is exact
/// (the literal definitely does not occur); `true` is approximate.
pub trait Membership {
    /// Whether `literal` might occur in the corpus. Never a false
    /// negative: a literal that occurs always returns `true`.
    fn might_contain(&self, literal: &[u8]) -> bool;
    /// A short name for diagnostics and the CLI report.
    fn name(&self) -> &'static str;
}

/// A Bloom filter over the corpus n-grams (Bloom 1970). Each key sets
/// `k` bits chosen by `k` independent hashes; a query reads the same
/// bits. An unset bit proves absence, so there are no false negatives.
pub struct BloomFilter {
    bits: Vec<u64>,
    /// Number of addressable bits, a power of two so the index is a mask.
    nbits: usize,
    k: usize,
}

impl BloomFilter {
    /// Build a Bloom filter sized for the corpus n-gram count at roughly
    /// a 1% false-positive rate (about 10 bits per key, 7 hashes).
    #[must_use]
    pub fn build(corpus: &[u8]) -> Self {
        let keys = ngram_keys(corpus);
        let n = keys.len().max(1);
        // 10 bits per key, rounded up to a power of two.
        let nbits = (n * 10).next_power_of_two().max(64);
        let mut f = Self { bits: vec![0u64; nbits / 64], nbits, k: 7 };
        for key in keys {
            f.insert(key);
        }
        f
    }

    fn positions(&self, key: u64) -> impl Iterator<Item = usize> + '_ {
        let h1 = key;
        let h2 = hash_bytes(&key.to_le_bytes(), 0xD1B5_4A32);
        (0..self.k).map(move |i| {
            // Kirsch-Mitzenmacher double hashing: h1 + i*h2.
            let h = h1.wrapping_add((i as u64).wrapping_mul(h2));
            (h as usize) & (self.nbits - 1)
        })
    }

    fn insert(&mut self, key: u64) {
        for p in self.positions(key).collect::<Vec<_>>() {
            self.bits[p >> 6] |= 1u64 << (p & 63);
        }
    }

    fn has(&self, key: u64) -> bool {
        self.positions(key).all(|p| self.bits[p >> 6] & (1u64 << (p & 63)) != 0)
    }
}

impl Membership for BloomFilter {
    fn might_contain(&self, literal: &[u8]) -> bool {
        all_ngrams_present(literal, |k| self.has(k))
    }
    fn name(&self) -> &'static str {
        "bloom"
    }
}

/// A Cuckoo filter over the corpus n-grams (Fan et al. 2014). Each key
/// is reduced to a one-byte fingerprint stored in one of two candidate
/// buckets; a query reads both buckets. Insertion grows the table and
/// rebuilds on overflow so no key is ever dropped, which is what keeps
/// the no-false-negative guarantee.
pub struct CuckooFilter {
    buckets: Vec<[u8; 4]>,
    /// Bucket count, a power of two.
    nbuckets: usize,
}

impl CuckooFilter {
    /// Build a Cuckoo filter over the corpus n-grams, growing the table
    /// until every key is placed.
    #[must_use]
    pub fn build(corpus: &[u8]) -> Self {
        let keys = ngram_keys(corpus);
        let mut nbuckets = ((keys.len() / 4) + 1).next_power_of_two().max(4);
        loop {
            if let Some(f) = Self::try_build(&keys, nbuckets) {
                return f;
            }
            nbuckets *= 2;
        }
    }

    fn fingerprint(key: u64) -> u8 {
        // Clamp to a nonzero byte; zero marks an empty slot.
        ((key & 0xFF) as u8).max(1)
    }

    fn index1(key: u64, nbuckets: usize) -> usize {
        ((key >> 32) as usize) & (nbuckets - 1)
    }

    fn alt_index(i: usize, fp: u8, nbuckets: usize) -> usize {
        // Partial-key cuckoo: the alternate bucket is derived from the
        // fingerprint alone, so a relocation needs no stored key.
        let h = hash_bytes(&[fp], 0x9E37_79B1);
        (i ^ (h as usize)) & (nbuckets - 1)
    }

    fn try_build(keys: &[u64], nbuckets: usize) -> Option<Self> {
        let mut f = Self { buckets: vec![[0u8; 4]; nbuckets], nbuckets };
        for &key in keys {
            if !f.insert(key) {
                return None;
            }
        }
        Some(f)
    }

    fn place(&mut self, i: usize, fp: u8) -> bool {
        for slot in &mut self.buckets[i] {
            if *slot == 0 {
                *slot = fp;
                return true;
            }
        }
        false
    }

    fn insert(&mut self, key: u64) -> bool {
        let fp = Self::fingerprint(key);
        let i1 = Self::index1(key, self.nbuckets);
        if self.place(i1, fp) {
            return true;
        }
        let i2 = Self::alt_index(i1, fp, self.nbuckets);
        if self.place(i2, fp) {
            return true;
        }
        // Evict: kick fingerprints along a cuckoo path. A deterministic
        // victim slot keeps the build reproducible.
        let mut i = i2;
        let mut carry = fp;
        for kick in 0..500usize {
            let victim_slot = kick & 3;
            std::mem::swap(&mut carry, &mut self.buckets[i][victim_slot]);
            i = Self::alt_index(i, carry, self.nbuckets);
            if self.place(i, carry) {
                return true;
            }
        }
        false
    }

    fn has(&self, key: u64) -> bool {
        let fp = Self::fingerprint(key);
        let i1 = Self::index1(key, self.nbuckets);
        if self.buckets[i1].contains(&fp) {
            return true;
        }
        let i2 = Self::alt_index(i1, fp, self.nbuckets);
        self.buckets[i2].contains(&fp)
    }
}

impl Membership for CuckooFilter {
    fn might_contain(&self, literal: &[u8]) -> bool {
        all_ngrams_present(literal, |k| self.has(k))
    }
    fn name(&self) -> &'static str {
        "cuckoo"
    }
}

/// An Xor filter over the corpus n-grams (Graf and Lemire 2020). Each
/// key maps to three slots whose stored bytes xor to the key's
/// fingerprint; a query recomputes that xor. The table is filled by
/// peeling, which assigns each key to a slot no other key has yet
/// claimed, so the construction is exact and there are no false
/// negatives.
pub struct XorFilter {
    fingerprints: Vec<u8>,
    block: usize,
    seed: u64,
}

impl XorFilter {
    /// Build an Xor filter over the corpus n-grams. Peeling is retried
    /// across seeds, and the table is grown if every seed hits a cycle,
    /// so construction always succeeds with no false negatives.
    #[must_use]
    pub fn build(corpus: &[u8]) -> Self {
        let keys = ngram_keys(corpus);
        let mut block = (((1.23 * keys.len() as f64).ceil() as usize + 32) / 3).max(1) + 1;
        loop {
            for seed in 0..64u64 {
                if let Some(f) = Self::try_build(&keys, block, seed) {
                    return f;
                }
            }
            // A cycle survived every seed at this size; a larger table
            // lowers the load until peeling succeeds.
            block += block / 2 + 1;
        }
    }

    fn fp(hash: u64) -> u8 {
        // A fingerprint that is never zero would waste a bit; zero is a
        // fine fingerprint here because a query xors three slots.
        (hash ^ (hash >> 32)) as u8
    }

    fn slots(hash: u64, block: usize) -> [usize; 3] {
        // Rotations, not shifts: each rotation keeps all 32 bits
        // populated, so the three positions are independent. A shift
        // would leave the high bits of the later words zero, collapsing
        // a position onto one slot and leaving peeling a huge 2-core.
        let r0 = hash as u32;
        let r1 = hash.rotate_left(21) as u32;
        let r2 = hash.rotate_left(42) as u32;
        [reduce(r0, block), block + reduce(r1, block), 2 * block + reduce(r2, block)]
    }

    fn try_build(keys: &[u64], block: usize, seed: u64) -> Option<Self> {
        let size = block * 3;
        // Per-slot xor of mapped key hashes and count of mapped keys.
        let mut h_xor = vec![0u64; size];
        let mut h_count = vec![0u32; size];
        let hashes: Vec<u64> = keys.iter().map(|&k| hash_bytes(&k.to_le_bytes(), seed | 1)).collect();
        for &h in &hashes {
            for s in Self::slots(h, block) {
                h_xor[s] ^= h;
                h_count[s] += 1;
            }
        }
        // Peel: repeatedly take a slot with exactly one key, record it,
        // and remove that key from its other two slots.
        let mut stack: Vec<(usize, u64)> = Vec::with_capacity(keys.len());
        let mut queue: Vec<usize> = (0..size).filter(|&s| h_count[s] == 1).collect();
        while let Some(s) = queue.pop() {
            if h_count[s] != 1 {
                continue;
            }
            let h = h_xor[s];
            stack.push((s, h));
            for t in Self::slots(h, block) {
                h_xor[t] ^= h;
                h_count[t] -= 1;
                if h_count[t] == 1 {
                    queue.push(t);
                }
            }
        }
        if stack.len() != keys.len() {
            return None;
        }
        // Fill in reverse peel order so each key's chosen slot is the one
        // not yet assigned by the keys peeled after it.
        let mut fingerprints = vec![0u8; size];
        while let Some((s, h)) = stack.pop() {
            let [a, b, c] = Self::slots(h, block);
            let other = fingerprints[a] ^ fingerprints[b] ^ fingerprints[c];
            fingerprints[s] = Self::fp(h) ^ other;
        }
        Some(Self { fingerprints, block, seed: seed | 1 })
    }

    fn has(&self, key: u64) -> bool {
        let h = hash_bytes(&key.to_le_bytes(), self.seed);
        let [a, b, c] = Self::slots(h, self.block);
        Self::fp(h) == (self.fingerprints[a] ^ self.fingerprints[b] ^ self.fingerprints[c])
    }
}

impl Membership for XorFilter {
    fn might_contain(&self, literal: &[u8]) -> bool {
        all_ngrams_present(literal, |k| self.has(k))
    }
    fn name(&self) -> &'static str {
        "xor"
    }
}

/// How one filter kept its contract over a corpus, as [`verify`] probed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterCheck {
    /// The filter's name: `bloom`, `cuckoo` or `xor`.
    pub name: &'static str,
    /// The substrings of the corpus probed, each present by construction.
    pub present: usize,
    /// The present probes the filter reported absent, which its contract
    /// forbids.
    pub false_negatives: usize,
    /// The byte strings probed that an exact search finds nowhere in the
    /// corpus.
    pub absent: usize,
    /// The absent probes the filter rejected, each a scan it saves.
    pub rejected: usize,
}

/// Probes every filter's contract over `corpus`: each substring taken from
/// it must read as one that might occur, and the share of byte strings found
/// nowhere in it that a filter rejects is the scanning it saves.
///
/// The present probes are substrings of 4, 6, 8, 12 and 16 bytes at about
/// 200 evenly spaced offsets each; the absent ones are 500 printable byte
/// strings of 6 to 21 bytes from a fixed generator, each kept only once an
/// exact search finds it nowhere, so the same corpus always gets the same
/// probes.
#[must_use]
pub fn verify(corpus: &[u8]) -> Vec<FilterCheck> {
    let filters: [Box<dyn Membership>; 3] = [
        Box::new(BloomFilter::build(corpus)),
        Box::new(CuckooFilter::build(corpus)),
        Box::new(XorFilter::build(corpus)),
    ];
    let mut present: Vec<&[u8]> = Vec::new();
    for len in [4usize, 6, 8, 12, 16] {
        if len > corpus.len() {
            continue;
        }
        let step = (corpus.len() / 200).max(1);
        let mut p = 0;
        while p + len <= corpus.len() {
            present.push(&corpus[p..p + len]);
            p += step;
        }
    }
    let mut absent: Vec<Vec<u8>> = Vec::new();
    let mut state: u64 = 0x1234_5678_9ABC_DEF0;
    while absent.len() < 500 {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let len = 6 + (state >> 60) as usize;
        let probe: Vec<u8> = (0..len)
            .map(|j| {
                let r = state.rotate_left(j as u32 * 7);
                b'!' + (r % 93) as u8
            })
            .collect();
        if !crate::byte_simd::contains(corpus, &probe) {
            absent.push(probe);
        }
    }
    filters
        .iter()
        .map(|f| FilterCheck {
            name: f.name(),
            present: present.len(),
            false_negatives: present.iter().filter(|l| !f.might_contain(l)).count(),
            absent: absent.len(),
            rejected: absent.iter().filter(|l| !f.might_contain(l)).count(),
        })
        .collect()
}

/// Guard literals up to this many are each searched for directly; past it
/// the Bloom filter is built once over the input and every literal asks it.
///
/// A direct search is one SIMD pass over the input per literal and proves
/// absence exactly, where the filter build hashes and inserts every n-gram of
/// the input before the first question is asked. Measured by
/// `benches/vs_regex` on a 63kB corpus: 1 microsecond for a direct search of
/// an absent literal against 1150 for the filter build, both linear in the
/// input, so the build is only worth it past a thousand literals. A pattern
/// has a handful.
const DIRECT_SEARCH_MAX_LITERALS: usize = 1024;

/// The literals in `pattern`'s content guards that are provably absent from
/// `input`. Each is definitely not in the input, so a guard requiring it can
/// short-circuit to "no match" with no scan. Empty when the pattern has no
/// guards, so a guardless scan costs only a cheap pattern walk.
#[must_use]
pub fn absent_guard_literals(pattern: &crate::ast::Pattern, input: &[u8]) -> HashSet<Vec<u8>> {
    let mut lits: Vec<Vec<u8>> = Vec::new();
    collect_guard_literals(pattern, &mut lits);
    if lits.is_empty() {
        return HashSet::new();
    }
    if lits.len() <= DIRECT_SEARCH_MAX_LITERALS {
        return lits.into_iter().filter(|l| !crate::byte_simd::contains(input, l)).collect();
    }
    let filter = BloomFilter::build(input);
    lits.into_iter().filter(|l| !filter.might_contain(l)).collect()
}

/// Whether `pattern` requires something that `input` does not hold, so no
/// match can exist anywhere and the caller can answer without lexing.
///
/// The lex runs at 217 to 513 MB/s on the corpora measured, and a scan that
/// cannot match still runs all of it. Two things are answerable from
/// the bytes alone and both are asked here: a byte string every match must
/// contain, at a literal search each, and - for the kinds that name no
/// literal - a run of one byte class every token of the kind must hold.
#[must_use]
pub fn requires_absent(pattern: &crate::ast::Pattern, input: &[u8]) -> bool {
    requires_absent_with(pattern, input, None)
}

/// [`requires_absent`] asking `filter` first where one is given: a literal the
/// filter refuses is absent with no search, and one it admits is searched. A
/// caller asking about many patterns over one input builds the filter once and
/// searches only for the literals it admits.
///
/// The run condition takes no filter and is asked last, because it is the only
/// one of the two that reads the input rather than a summary of it. A pattern
/// naming no run costs a match on its kind to find that out.
#[must_use]
pub fn requires_absent_with(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    filter: Option<&BloomFilter>,
) -> bool {
    let mut lits: Vec<Vec<u8>> = Vec::new();
    collect_required_literals(pattern, &mut lits);
    let literal_absent = lits.iter().any(|l| match filter {
        Some(f) if !f.might_contain(l) => true,
        Some(_) | None => !crate::byte_simd::contains(input, l),
    });
    literal_absent || requires_absent_run(pattern, input)
}

/// How many literals `pattern` requires, so a caller over many patterns
/// can size the question before building a filter for it.
#[must_use]
pub fn required_literal_count(pattern: &crate::ast::Pattern) -> usize {
    let mut lits: Vec<Vec<u8>> = Vec::new();
    collect_required_literals(pattern, &mut lits);
    lits.len()
}

/// Whether every literal in `lits` is one `filter` refuses, so no route
/// need search for any of them.
#[must_use]
pub fn all_absent_by(filter: &BloomFilter, lits: &[&str]) -> bool {
    !lits.is_empty() && lits.iter().all(|l| !filter.might_contain(l.as_bytes()))
}

/// The number of literals across many patterns above which one filter over
/// the input is built and asked, rather than each literal searched for.
#[must_use]
pub fn direct_search_max_literals() -> usize {
    DIRECT_SEARCH_MAX_LITERALS
}

/// The text of `pattern` when it is one literal word token and nothing else,
/// so its matches are answerable by [`byte_route_word_literal`].
///
/// The shape is checked as well as the orbit, and only an ASCII identifier
/// that does not open with a digit is accepted: a leading digit opens a
/// number or a typed token, punctuation lexes as tokens of its own, and a
/// letter beyond ASCII continues a word run through a UTF-8 decode the byte
/// tests do not make.
///
/// This says the pattern can be answered from the bytes. Whether an
/// occurrence is the token is settled against the input by the route, which
/// reads the lexer over the occurrence's run where the bytes alone cannot
/// say, and declines the input where even that cannot.
#[must_use]
pub fn byte_routable_literal(pattern: &crate::ast::Pattern) -> Option<&str> {
    use crate::ast::{Atom, Pattern};
    use crate::orbit::OrbitGroup;
    let Pattern::Atom(Atom::Literal(lit, OrbitGroup::Identity)) = pattern else {
        return None;
    };
    let b = lit.as_bytes();
    let opens = b.first().is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_');
    let word = b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_');
    (opens && word).then_some(lit.as_str())
}

/// Whether the digit run `input[s..e]` is certainly a whole `Number` token.
///
/// The lexer reaches [`crate::token::TokenKind::Number`] only once every typed
/// recognizer has declined, and those are not local to the run. `Money` opens
/// at a `$` before the digits; `ByteSize`, `Percent` and `Duration` close on a
/// letter or `%` after them; `Quantity` opens at a sign before them and closes
/// on a unit symbol after them or one space after them; `Ip`, `Version`,
/// `Timestamp`, `Mac` and `Uuid` join runs across `.`, `:` and `-`;
/// `CreditCard` joins four-digit groups across single spaces under a Luhn
/// check; and the number itself reads on through an exponent, a `0x`, `0b` or
/// `0o` prefix or a `_` group, each led by a letter, `_` or `.` after the
/// run. So a digit run settles neither the kind, nor the token's start, nor
/// its end, and this refuses wherever any of them can reach.
///
/// One-directional: a refusal costs the lex that would run anyway, and a
/// wrong acceptance reports a token the lexer does not make.
pub(crate) fn settles_number(input: &[u8], s: usize, e: usize) -> bool {
    if s >= e || !input[s..e].iter().all(u8::is_ascii_digit) {
        return false;
    }
    // A byte that lets a typed recognizer continue across the run's edge.
    let joins = |c: u8| {
        c.is_ascii_alphanumeric()
            || matches!(c, b'.' | b':' | b'-' | b'_' | b'%' | b'$' | b',' | b'/' | b'+' | b'#')
            || c >= 0x80
    };
    if s > 0 && joins(input[s - 1]) {
        return false;
    }
    if input.get(e).copied().is_some_and(joins) {
        return false;
    }
    // One space then a unit symbol makes the run a quantity's number, by the
    // test the lexer applies there.
    if input.get(e) == Some(&b' ') && crate::quantity::spaced_symbol_len(input, e + 1).is_some() {
        return false;
    }
    // A card is digit groups joined by single spaces, so a group-sized run with
    // another group one space away may be inside one. The Luhn check that would
    // settle it reads up to nineteen digits, which is not a neighbor test, so
    // the shape alone is refused.
    if (3..=5).contains(&(e - s)) {
        let after = input.get(e) == Some(&b' ')
            && input.get(e + 1).is_some_and(u8::is_ascii_digit);
        let before = s >= 2 && input[s - 1] == b' ' && input[s - 2].is_ascii_digit();
        if after || before {
            return false;
        }
    }
    true
}

/// The leftmost `Number` token in `input`, read from the bytes, or `None` where
/// the first digit run reached is one the bytes cannot settle.
///
/// A run that does not settle cannot be stepped over: it may be the match, and
/// reporting the next one would report the wrong match. So an unsettled run
/// hands the whole input back and the lex answers instead.
///
/// A digit inside a string opens no token, so those are passed over rather than
/// refused. Whether the quote scan met something it could not settle is asked
/// after the reading rather than at the start, since a quote beyond the first
/// number never bore on the answer.
#[must_use]
pub fn byte_route_first_number(input: &[u8]) -> Option<Option<crate::engine::Span>> {
    byte_route_first_number_from(input, 0)
}

/// [`byte_route_first_number`] beginning the search at byte `from`.
///
/// An anchored caller wants the leftmost match at or after a position and the
/// bytes before it hold no such match by definition. A `from` inside a run is
/// not a special case: the run's own first byte is behind it, so the run
/// is stepped over by the same test that steps over a digit inside a word.
#[must_use]
pub fn byte_route_first_number_from(
    input: &[u8],
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    byte_route_first_number_reading(&ByteReader::new(input), from)
}

/// [`byte_route_first_number_from`] over a reader the caller holds.
///
/// A reader's quote scan advances to the position it is asked about and no
/// further, so one held across a run of anchored asks reads the input once
/// where one built per ask restarts that scan from byte zero.
///
/// Asks must ascend. A reader carries `unsettled` for its life once its scan
/// meets a quote the rule cannot settle, so an ask behind a position already
/// read can hand the input back where a fresh reader would have answered it.
/// That is a refusal and never a wrong span, and an ascending caller never
/// meets it: the flag is then set exactly where a fresh reader would set it.
#[must_use]
pub(crate) fn byte_route_first_number_reading(
    reader: &ByteReader<'_>,
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    let input = reader.input;
    let mut from = from.min(input.len());
    while let Some(rel) = crate::byte_simd::digit_find(&input[from..]) {
        let s = from + rel;
        if reader.string_end(s).is_some() {
            from = s + 1;
            continue;
        }
        if reader.unsettled() {
            return None;
        }
        // A digit after a word byte is inside that run, so no token opens
        // at it whatever the run turns out to be. Stepping over the run is not
        // the refusal below: nothing about this position is unsettled, and
        // handing the input back for it would decline every corpus whose first
        // digit is in an identifier.
        if s > 0 && (input[s - 1].is_ascii_alphanumeric() || input[s - 1] == b'_') {
            let mut w = s;
            while input.get(w).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                w += 1;
            }
            from = w;
            continue;
        }
        let mut e = s;
        while input.get(e).is_some_and(|c| c.is_ascii_digit()) {
            e += 1;
        }
        if !settles_number(input, s, e) {
            return None;
        }
        return Some(Some(crate::engine::Span { start: s as u32, end: e as u32 }));
    }
    if reader.unsettled() {
        return None;
    }
    Some(None)
}

/// The leftmost `Word` token in `input`, read from the bytes, or `None` where
/// the first run that could open one is not settled by them.
///
/// The counterpart of [`byte_route_first_number`] and the easier half: the
/// probe already settles a word run, and [`ByteReader::settles_word`] is the
/// gate that refuses a run a dot, slash or colon joins into a typed token, so
/// the kind it reports is the lexer's for a run it accepts.
///
/// A letter after a word byte is inside that run and opens no token, so it
/// is stepped over. A run the probe defers to the lexer is not: it may be the
/// match, and the whole input is handed back rather than the next run reported.
#[must_use]
pub fn byte_route_first_word(input: &[u8]) -> Option<Option<crate::engine::Span>> {
    byte_route_first_word_from(input, 0)
}

/// [`byte_route_first_word`] beginning the search at byte `from`, for the same
/// reason [`byte_route_first_number_from`] takes one.
#[must_use]
pub fn byte_route_first_word_from(
    input: &[u8],
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    byte_route_first_word_reading(&ByteReader::new(input), from)
}

/// [`byte_route_first_word_from`] over a reader the caller holds, under the
/// ascending-ask constraint [`byte_route_first_number_reading`] states.
#[must_use]
pub(crate) fn byte_route_first_word_reading(
    reader: &ByteReader<'_>,
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    let input = reader.input;
    let mut from = from.min(input.len());
    while let Some(rel) =
        input[from..].iter().position(|c| c.is_ascii_alphabetic() || *c == b'_')
    {
        let s = from + rel;
        if reader.string_end(s).is_some() {
            from = s + 1;
            continue;
        }
        if reader.unsettled() {
            return None;
        }
        if s > 0 && (input[s - 1].is_ascii_alphanumeric() || input[s - 1] == b'_') {
            let mut w = s;
            while input.get(w).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                w += 1;
            }
            from = w;
            continue;
        }
        match reader.probe_starting_at(s) {
            Probe::Settled(Read::TokenEndingAt(crate::token::TokenKind::Word, e)) => {
                return Some(Some(crate::engine::Span { start: s as u32, end: e as u32 }));
            }
            // A run the bytes settle as something other than a word, which the
            // probe does not report and which would have to be a token this
            // route cannot name.
            Probe::Settled(
                Read::TokenEndingAt(..) | Read::TokenStartingAt(..) | Read::TokenAt(..),
            ) => return None,
            Probe::Settled(Read::NoToken) => {
                from = s + 1;
                continue;
            }
            Probe::Settled(Read::Unsettled) | Probe::Lex => return None,
        }
    }
    if reader.unsettled() {
        return None;
    }
    Some(None)
}

/// The leftmost match of a bare kind atom at or after `at`, read from the
/// bytes, or `None` where `pattern` is not one of the kinds routed here or the
/// first candidate is one the bytes cannot settle.
///
/// One place rather than one per ladder. Whether a pattern matches, where it
/// first matches, and where it first matches at or after a position are three
/// entry points with three ladders, because they differ in what they may
/// assume; a bare kind atom answers all three the same way, and this is what
/// they share. A new kind belongs in the match below and reaches every caller.
#[must_use]
pub fn byte_route_kind_from(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    at: usize,
) -> Option<Option<crate::engine::Span>> {
    byte_route_kind_reading(pattern, input, &mut None, at)
}

/// The leftmost `Quoted` token beginning at or after byte `from`, read from the
/// bytes, or `None` where the scan met a quote it could not settle first.
///
/// The other kind routes are refusals built on a probe, because a word or digit
/// run is not settled as a token by the run that opens it. A quoted token is
/// different in kind: nothing reaches across its edges, and the rules that
/// decide it are the lexer's own.
///
/// The scan records strings, which is a smaller set than the quoted tokens.
/// [`crate::parallel_lex::quoted_spans_where`] exists to say what is inside
/// one, so a char literal is passed over whole - which stops the quote it may
/// hold from opening a string - and no span is kept for it. A route reading
/// only the scan's spans therefore reports no match on `c = '"' ; d = 'x'`,
/// where the lexer makes two tokens. So the string branches come from the scan
/// and the char-literal branch from [`crate::lexer::char_literal_end`], the
/// same function the scan uses to pass it over.
///
/// All three of the lexer's quote branches emit the same kind, so no second
/// kind has to be told apart.
///
/// A quote byte that does not open its span is inside a string that began
/// before `from`, so that token is not one an anchored caller asked for and the
/// search steps past the string rather than reporting it.
#[must_use]
pub(crate) fn byte_route_first_quoted_reading(
    reader: &ByteReader<'_>,
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    let input = reader.input;
    let next = |at: usize, q: u8| crate::byte_simd::find(&input[at..], &[q]).map(|r| at + r);
    let mut from = from.min(input.len());
    loop {
        // The two quote bytes are searched for separately because the search is
        // for a substring, not a set; the nearer of the two is the next quote.
        let q = match (next(from, b'"'), next(from, b'\'')) {
            (Some(d), Some(s)) => d.min(s),
            (Some(one), None) | (None, Some(one)) => one,
            (None, None) => break,
        };
        let span = reader.string_span(q);
        // Asked after the reading rather than before it, as the other routes
        // ask: a quote beyond the first token never bore on the answer.
        if reader.unsettled() {
            return None;
        }
        match span {
            // The quote opens its own string, so the span is the token. A
            // string no quote closes runs to the input's end, and the scan
            // carries that length where a closing quote's index would be, so
            // the token ends there rather than a byte past it.
            Some((a, b)) if a == q => {
                return Some(Some(crate::engine::Span {
                    start: a as u32,
                    end: (b + 1).min(input.len()) as u32,
                }));
            }
            // Inside a string that opened before `from`; step past its close.
            Some((_, b)) => from = b + 1,
            None => {
                // The scan records strings, not every quoted token: a char
                // literal is passed over whole so the quote it may hold opens
                // nothing, and no span is kept for it. So the lexer's own rule
                // is asked here, which is the same function the scan uses to
                // pass it over. Inside a blob run the lexer opens neither a string
                // nor a char literal, taking the run whole.
                if !reader.run_of(q, q + 1, false).blob
                    && let Some(end) = crate::lexer::char_literal_end(input, q)
                {
                    return Some(Some(crate::engine::Span {
                        start: q as u32,
                        end: end as u32,
                    }));
                }
                from = q + 1;
            }
        }
    }
    Some(None)
}

/// [`byte_route_kind_from`] over a reader the caller holds, under the
/// ascending-ask constraint [`byte_route_first_number_reading`] states.
///
/// The reader is made here on the first ask that needs one, so a pattern this
/// declines costs the match below and not the two searches that opening a
/// reader costs.
#[must_use]
pub(crate) fn byte_route_kind_reading<'i>(
    pattern: &crate::ast::Pattern,
    input: &'i [u8],
    reader: &mut Option<ByteReader<'i>>,
    at: usize,
) -> Option<Option<crate::engine::Span>> {
    use crate::ast::{Atom, Pattern};
    use crate::token::TokenKind;
    match pattern {
        Pattern::Atom(Atom::Kind(TokenKind::Number)) => byte_route_first_number_reading(
            reader.get_or_insert_with(|| ByteReader::new(input)),
            at,
        ),
        Pattern::Atom(Atom::Kind(TokenKind::Word)) => byte_route_first_word_reading(
            reader.get_or_insert_with(|| ByteReader::new(input)),
            at,
        ),
        Pattern::Atom(Atom::Kind(TokenKind::Quoted)) => byte_route_first_quoted_reading(
            reader.get_or_insert_with(|| ByteReader::new(input)),
            at,
        ),
        // A run of word atoms - `\W \W`, or `\W{2}` written shorter. The
        // sequence is read through [`crate::kind_route::kind_sequence`], which
        // is where the shape of a kind sequence is already decided, so a repeat
        // and its written-out form reach this by one reading and not two.
        _ => {
            let codes = crate::kind_route::kind_sequence(pattern)?;
            let word = TokenKind::Word.code();
            if codes.len() < 2 || codes.iter().any(|&c| c != word) {
                return None;
            }
            byte_route_first_word_run_reading(
                reader.get_or_insert_with(|| ByteReader::new(input)),
                codes.len(),
                at,
            )
        }
    }
}

/// The leftmost run of `times` consecutive word tokens at or after `at`, read
/// from the bytes, or `None` where the bytes cannot settle one of the tokens it
/// reaches.
///
/// [`byte_route_first_word_reading`] answers one word token, and a run of them
/// matches wherever that many consecutive significant tokens are words. So this
/// finds a word and then asks about the token beginning at each following
/// significant byte, restarting past the first word wherever the run ends too
/// soon. Whitespace is stepped over, as the engine skips it.
///
/// Every ask is a probe over that token's own run, so reaching a match costs one
/// probe per token and no lex. A run the probe defers to the lexer is refused
/// rather than lexed: the caller falls back to the prefix, which lexes anyway.
///
/// The leftmost run is also the soonest-ending one, because every match is the
/// same number of whole tokens and a later match closes later - so the callers
/// that want the soonest end may take this too.
#[must_use]
pub(crate) fn byte_route_first_word_run_reading(
    reader: &ByteReader<'_>,
    times: usize,
    at: usize,
) -> Option<Option<crate::engine::Span>> {
    let input = reader.input;
    let mut at = at;
    loop {
        let first = byte_route_first_word_reading(reader, at)?;
        let Some(first) = first else {
            return Some(None);
        };
        let mut end = first.end as usize;
        let mut held = 1usize;
        while held < times {
            let Some(rel) = input[end..].iter().position(|c| !c.is_ascii_whitespace()) else {
                break;
            };
            let p = end + rel;
            // [`ByteReader::probe_starting_at`] is written for a letter or
            // underscore that no letter or underscore precedes, which the
            // whitespace before `p` guarantees. Any other byte opens a token that
            // is not a word, and the run ends here.
            if !(input[p].is_ascii_alphabetic() || input[p] == b'_')
                || reader.string_end(p).is_some()
            {
                break;
            }
            match reader.probe_starting_at(p) {
                Probe::Settled(Read::TokenEndingAt(crate::token::TokenKind::Word, e)) => {
                    end = e;
                    held += 1;
                }
                Probe::Settled(Read::Unsettled) | Probe::Lex => return None,
                Probe::Settled(_) => break,
            }
        }
        if held == times {
            return Some(Some(crate::engine::Span { start: first.start, end: end as u32 }));
        }
        // A match begins at a word token, so the next place one can begin is the
        // next word at or after this one's end.
        at = first.end as usize;
    }
}

/// The text of `pattern` when it is one routable word literal behind `^`, so
/// its matches are the route's occurrences that lead their lines.
#[must_use]
pub fn byte_routable_line_anchored_literal(pattern: &crate::ast::Pattern) -> Option<&str> {
    use crate::ast::{AnchorKind, Pattern};
    let Pattern::Concat(v) = pattern else {
        return None;
    };
    let [Pattern::Anchor(AnchorKind::LineStart), rest] = v.as_slice() else {
        return None;
    };
    byte_routable_literal(rest)
}

/// Where a line-anchored word literal first matches at or after byte `from`,
/// read to that match and no further, or `None` where the bytes cannot settle
/// an occurrence it reaches.
///
/// [`byte_route_word_literals`] finds every occurrence because a scan must, and
/// the spans route then filters them by [`leads_its_line`]. Taking that and
/// discarding the rest reads the whole input to answer about a match that is
/// often in its first bytes, so this walks the occurrences one at a time and
/// stops at the first that leads its line.
///
/// An occurrence that does not lead its line costs the search that found it and
/// no lex, which is why stepping past it is cheap enough to do one at a time.
#[must_use]
pub fn byte_route_first_line_anchored_literal(
    lit: &str,
    input: &[u8],
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    byte_route_first_line_anchored_literal_reading(lit, &mut ByteReader::new(input), from)
}

/// [`byte_route_first_line_anchored_literal`] over a reader the caller holds,
/// under the ascending-ask constraint [`byte_route_first_number_reading`]
/// states.
///
/// The occurrences it steps over ascend, so one reader serves the whole walk
/// even though the walk is a loop: each ask starts past the last occurrence
/// rejected.
#[must_use]
pub(crate) fn byte_route_first_line_anchored_literal_reading(
    lit: &str,
    reader: &mut ByteReader<'_>,
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    let input = reader.input;
    let mut from = from.min(input.len());
    loop {
        let Some(found) = byte_route_first_word_literal_reading(&[lit], reader, from)? else {
            return Some(None);
        };
        if leads_its_line(input, found.start()) {
            return Some(Some(found));
        }
        from = found.start() + 1;
    }
}

/// Whether the token beginning at `at` is the first significant one on its
/// line, read from the bytes.
///
/// Only whitespace is insignificant, so the test is that every byte back to the
/// previous newline, or to the input's start, is whitespace. An indented token
/// still leads its line, which a test for a newline directly before it would
/// refuse.
///
/// The walk back stops at the first byte that is not whitespace, so it reads
/// the line's indentation and not its length.
#[must_use]
pub(crate) fn leads_its_line(input: &[u8], at: usize) -> bool {
    input[..at].iter().rev().take_while(|&&c| c != b'\n').all(u8::is_ascii_whitespace)
}

/// The literals of `pattern` when it is one routable word literal or a flat
/// alternation of them, so a search for each answers the whole pattern.
///
/// An alternation qualifies because each alternative is one whole token. Two
/// different literals cannot both be whole tokens at one position, since the
/// shorter's end neighbor would be a byte of the longer, an identifier byte.
/// So at most one matches anywhere, matches never overlap, and a merge in
/// start order is exact whichever reading the alternation carries. Repeated
/// literals are folded so a match is not reported twice.
#[must_use]
pub fn byte_routable_literals(pattern: &crate::ast::Pattern) -> Option<Vec<&str>> {
    use crate::ast::Pattern;
    if let Some(lit) = byte_routable_literal(pattern) {
        return Some(vec![lit]);
    }
    let Pattern::Alt(branches, _) = pattern else {
        return None;
    };
    let mut lits: Vec<&str> = Vec::with_capacity(branches.len());
    for branch in branches {
        let lit = byte_routable_literal(branch)?;
        if !lits.contains(&lit) {
            lits.push(lit);
        }
    }
    (!lits.is_empty()).then_some(lits)
}

/// Inputs at or above this take the every-match byte routes cut across the
/// cores; below it they run in one pass.
///
/// Measured by `examples/chunked_route_ab` on a 24-core host, the cut route
/// against the one-pass form on the comparison's corpus, medians of seven
/// rotated rounds with a control, the same on all three routes it serves:
///
/// ```text
///   130 KB    cut 0.0393 ms   one pass 0.0356   the cut loses, 1.10x
///   163 KB    cut 0.0425      one pass 0.0522   the cut wins,  0.81x
///   329 KB    cut 0.0593      one pass 0.1018   0.58x
///   862 KB    cut 0.0944      one pass 0.2180   0.43x
///   7.34 MB   cut 0.5111      one pass 2.0243   0.25x
///   14.97 MB  cut 1.1759      one pass 3.8653   0.30x
/// ```
///
/// The crossing is between the first two rows.
pub const CUT_ROUTE_THRESHOLD: usize = 160 * 1024;

/// `route` over each chunk of `input` cut at the line boundaries among
/// [`crate::parallel_lex::safe_boundaries`], on the cores, the spans rebased
/// and joined in chunk order.
///
/// A safe boundary has a significant byte at it and no string across it, and
/// is either the first significant byte after a newline or, where a division's
/// search finds no newline, the end of a whitespace run. The lexer may cut at
/// either. The routes here may not: a route that reads across whitespace - the
/// three tokens of a literal, a word and punctuation, or the walk back to the
/// previous newline that says a token leads its line - would find its match
/// split over two chunks at a whitespace-run cut. So only the boundaries with
/// a newline before them are kept. Those are line boundaries, and no run, no
/// string, no literal and no match of these routes crosses a line boundary: a
/// chunk's reader answers from the chunk's bytes alone, and its spans,
/// rebased, are the one-pass route's for that stretch. Chunk order is start
/// order because a chunk owns exactly the occurrences that start inside it.
/// Input with no newline in a stretch gets fewer chunks there, down to one.
///
/// `None` from any chunk is `None` for the whole. The one-pass route hands the
/// whole input to the lexer at the first construct the bytes refuse, and a cut
/// form that answered around it would answer a different question.
///
/// Splitting only the search and walking the probes serially parallelizes the
/// cheap half; cutting the input puts the reader's quote scan and every probe
/// on the cores as well, which is where the route's time is.
fn cut_across_cores<F>(input: &[u8], route: F) -> Option<Vec<crate::engine::Span>>
where
    F: Fn(&[u8]) -> Option<Vec<crate::engine::Span>> + Sync,
{
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let n = input.len();
    let mut bounds = crate::parallel_lex::safe_boundaries(input, cores * 4);
    bounds.retain(|&b| b == 0 || b == n || input[b - 1] == b'\n');
    let leaves = bounds.len() - 1;
    crate::trace::rung("byte route across", "leaves", leaves);
    let mut found: Vec<Option<Vec<crate::engine::Span>>> = vec![None; leaves];
    let plan = flynnel::JobPlan::set_profile(
        0,
        u32::try_from(leaves).unwrap_or(u32::MAX),
        flynnel::DispatchProfile::Streaming,
    );
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut found, 1, |base, slots| {
        for (k, slot) in slots.iter_mut().enumerate() {
            let lo = bounds[base + k];
            let hi = bounds[base + k + 1];
            let rebase = u32::try_from(lo).expect("an input this route takes fits a u32 span");
            *slot = route(&input[lo..hi]).map(|spans| {
                spans
                    .into_iter()
                    .map(|s| crate::engine::Span { start: s.start + rebase, end: s.end + rebase })
                    .collect()
            });
        }
    });
    let mut out: Vec<crate::engine::Span> = Vec::new();
    for leaf in found {
        out.extend(leaf?);
    }
    Some(out)
}

/// [`cut_across_cores`] for a route whose match may run past the chunk that
/// owns its start.
///
/// `route` is given the input from its chunk's start to the input's end, and
/// how many of those bytes the chunk owns: it takes the matches whose start
/// is inside that many and may read past them to finish one. The reader
/// it builds scans quotes lazily, so reading past costs the bytes asked for
/// and not the rest of the input. The chunks are the same line boundaries
/// [`cut_across_cores`] keeps, so a chunk's reader starts outside every
/// string, as the one-pass reader would be at that position.
fn cut_across_cores_reading_past<F>(input: &[u8], route: F) -> Option<Vec<crate::engine::Span>>
where
    F: Fn(&[u8], usize) -> Option<Vec<crate::engine::Span>> + Sync,
{
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let n = input.len();
    let mut bounds = crate::parallel_lex::safe_boundaries(input, cores * 4);
    bounds.retain(|&b| b == 0 || b == n || input[b - 1] == b'\n');
    let leaves = bounds.len() - 1;
    crate::trace::rung("byte route across", "leaves, reading past", leaves);
    let mut found: Vec<Option<Vec<crate::engine::Span>>> = vec![None; leaves];
    let plan = flynnel::JobPlan::set_profile(
        0,
        u32::try_from(leaves).unwrap_or(u32::MAX),
        flynnel::DispatchProfile::Streaming,
    );
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut found, 1, |base, slots| {
        for (k, slot) in slots.iter_mut().enumerate() {
            let lo = bounds[base + k];
            let hi = bounds[base + k + 1];
            let rebase = u32::try_from(lo).expect("an input this route takes fits a u32 span");
            *slot = route(&input[lo..], hi - lo).map(|spans| {
                spans
                    .into_iter()
                    .map(|s| crate::engine::Span { start: s.start + rebase, end: s.end + rebase })
                    .collect()
            });
        }
    });
    let mut out: Vec<crate::engine::Span> = Vec::new();
    for leaf in found {
        out.extend(leaf?);
    }
    Some(out)
}

/// [`byte_route_word_literal`] over several literals, merged in start order,
/// or `None` when any of them makes the bytes insufficient.
///
/// Cut across the cores from [`CUT_ROUTE_THRESHOLD`], in one pass below it.
#[must_use]
pub fn byte_route_word_literals(lits: &[&str], input: &[u8]) -> Option<Vec<crate::engine::Span>> {
    if input.len() >= CUT_ROUTE_THRESHOLD {
        crate::trace::rung("byte route", "word literals, cut across the cores", input.len());
        return cut_across_cores(input, |chunk| byte_route_word_literals_one_pass(lits, chunk));
    }
    crate::trace::rung("byte route", "word literals, one pass", input.len());
    byte_route_word_literals_one_pass(lits, input)
}

/// The matches of `lit` as a whole word token that leads its line, or `None`
/// when the bytes cannot settle one: `^ "let"`.
///
/// The word-literal route with [`leads_its_line`] applied to each occurrence.
/// Cut across the cores from [`CUT_ROUTE_THRESHOLD`] with the filter inside
/// each chunk, since a safe boundary is the first significant byte after a
/// newline and so leads its line by construction: the filter reads back to the
/// previous newline and never crosses a cut.
#[must_use]
pub fn byte_route_line_anchored_literal(lit: &str, input: &[u8]) -> Option<Vec<crate::engine::Span>> {
    if input.len() >= CUT_ROUTE_THRESHOLD {
        crate::trace::rung("byte route", "line-anchored literal, cut across the cores", input.len());
        return cut_across_cores(input, |chunk| byte_route_line_anchored_literal_one_pass(lit, chunk));
    }
    crate::trace::rung("byte route", "line-anchored literal, one pass", input.len());
    byte_route_line_anchored_literal_one_pass(lit, input)
}

/// [`byte_route_line_anchored_literal`] in one pass whatever the size.
#[must_use]
pub fn byte_route_line_anchored_literal_one_pass(
    lit: &str,
    input: &[u8],
) -> Option<Vec<crate::engine::Span>> {
    let spans = byte_route_word_literals_one_pass(&[lit], input)?;
    Some(spans.into_iter().filter(|s| leads_its_line(input, s.start as usize)).collect())
}

/// [`byte_route_word_literals`] in one pass whatever the size: the form the
/// cut route runs over each chunk, and the A/B's baseline.
#[must_use]
pub fn byte_route_word_literals_one_pass(
    lits: &[&str],
    input: &[u8],
) -> Option<Vec<crate::engine::Span>> {
    let mut reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    let mut out: Vec<crate::engine::Span> = Vec::new();
    for lit in lits {
        out.extend(route_word_literal(&mut reader, lit)?);
    }
    out.sort_by_key(|s| s.start);
    Some(out)
}

/// Whether any occurrence of one of `lits` is a whole token, or `None` when
/// the bytes cannot settle one and the lexer must run over the whole input.
///
/// [`byte_route_word_literals`] walks every occurrence in the input because
/// it must report them all. A caller asking only whether a match exists needs
/// the first occurrence the bytes confirm and nothing after it, which, when the
/// literal occurs early in the input, is the difference between reading a few
/// bytes and reading all of them.
#[must_use]
pub fn byte_route_any_word_literal(lits: &[&str], input: &[u8]) -> Option<bool> {
    let mut reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    for lit in lits {
        if first_word_literal(&mut reader, lit, 0)?.is_some() {
            return Some(true);
        }
    }
    Some(false)
}

/// Where the leftmost match of any of `lits` is, read to that match and no
/// further, or `None` where the bytes cannot settle it.
///
/// The span form of [`byte_route_any_word_literal`], and the reason both exist:
/// [`byte_route_word_literals`] walks every occurrence in the input because it
/// must report them all, and when the literal occurs early in the input that is
/// the difference between reading a few bytes and reading every one of them.
///
/// Each literal is read to its own first confirmation and the earliest of those
/// wins, because leftmost is a property of the input and not of the order the
/// pattern names its alternatives.
#[must_use]
pub fn byte_route_first_word_literal(
    lits: &[&str],
    input: &[u8],
) -> Option<Option<crate::engine::Span>> {
    byte_route_first_word_literal_from(lits, input, 0)
}

/// [`byte_route_first_word_literal`] beginning the search at byte `from`.
///
/// An anchored caller wants the leftmost match at or after a position, and the
/// bytes before it hold no such match by definition. Starting the scan there
/// rather than filtering a whole input's worth of matches is what the anchored
/// form is for.
#[must_use]
pub fn byte_route_first_word_literal_from(
    lits: &[&str],
    input: &[u8],
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    byte_route_first_word_literal_reading(lits, &mut ByteReader::new(input), from)
}

/// [`byte_route_first_word_literal_from`] over a reader the caller holds, under
/// the ascending-ask constraint [`byte_route_first_number_reading`] states.
#[must_use]
pub(crate) fn byte_route_first_word_literal_reading(
    lits: &[&str],
    reader: &mut ByteReader<'_>,
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    if reader.unsettled() {
        return None;
    }
    let mut best: Option<crate::engine::Span> = None;
    for lit in lits {
        if let Some(s) = first_word_literal(reader, lit, from)?
            && best.is_none_or(|b| s.start < b.start)
        {
            best = Some(s);
        }
    }
    Some(best)
}

/// [`route_word_literal`] stopping at the first occurrence it confirms, and
/// reporting where it is.
///
/// An occurrence the bytes defer is lexed as soon as it is found, not batched
/// until the end of the pass as the full route does: this walk stops at the
/// first confirmation, so it may never reach the end of the pass.
fn first_word_literal(
    reader: &mut ByteReader<'_>,
    lit: &str,
    start_at: usize,
) -> Option<Option<crate::engine::Span>> {
    let pat = lit.as_bytes();
    if pat.is_empty() {
        return None;
    }
    let input = reader.input;
    let mut from = start_at.min(input.len());
    while let Some(rel) = crate::byte_simd::find(&input[from..], pat) {
        let s = from + rel;
        let e = s + pat.len();
        from = s + 1;
        // A word byte after the occurrence: no token ends there. A letter or
        // underscore before it: the token holding that byte runs on over it.
        if input.get(e).is_some_and(|&c| word_byte(c)) {
            continue;
        }
        if s > 0 && (input[s - 1].is_ascii_alphabetic() || input[s - 1] == b'_') {
            continue;
        }
        let settled = match reader.probe_ending_at(e) {
            Probe::Settled(Read::TokenStartingAt(_, start)) => start == s,
            Probe::Settled(Read::TokenEndingAt(..) | Read::TokenAt(..) | Read::NoToken) => false,
            Probe::Settled(Read::Unsettled) => return None,
            Probe::Lex => match reader.lex_ending_at(e) {
                Read::TokenStartingAt(_, start) => start == s,
                Read::TokenEndingAt(..) | Read::TokenAt(..) | Read::NoToken => false,
                Read::Unsettled => return None,
            },
        };
        if settled {
            return Some(Some(crate::engine::Span { start: s as u32, end: e as u32 }));
        }
    }
    Some(None)
}

/// What the bytes say about a token, read without lexing the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Read {
    /// The lexer makes a token of exactly the span asked about, of this kind,
    /// and starting at this offset. Reported by [`ByteReader::probe_ending_at`]
    /// and [`ByteReader::lex_ending_at`], which are given the token's end.
    ///
    /// The kind is only as specific as the reader that settled it. The fast
    /// path in [`ByteReader::probe_ending_at`] reports
    /// [`crate::token::TokenKind::Word`] for any run it settles by shape,
    /// whatever the lexer would call that run, because its caller decides what
    /// matched and reads only the offset. A caller that needs the true kind
    /// takes the lexing form instead.
    TokenStartingAt(crate::token::TokenKind, usize),
    /// As [`Self::TokenStartingAt`], with the offset being the token's end.
    /// Reported by [`ByteReader::probe_starting_at`] and
    /// [`ByteReader::lex_starting_at`], which are given the token's start; its
    /// kind too is only as specific as the reader that settled it.
    TokenEndingAt(crate::token::TokenKind, usize),
    /// As [`Self::TokenStartingAt`], with the offset being the position asked
    /// about. Reported by [`ByteReader::probe_punct_at`] and
    /// [`ByteReader::lex_punct_at`]: a punctuation token is one byte, so its
    /// start and its end coincide and the distinction does not arise.
    TokenAt(crate::token::TokenKind, usize),
    /// No token spans exactly it.
    NoToken,
    /// The bytes cannot settle it, and the lexer must run over the input.
    Unsettled,
}

// Each reader produces one token variant and no other. A match lists the
// variant its own reader makes and groups the other two with `NoToken`, which
// is how a reader's offset cannot be read from the wrong end: naming the wrong
// variant leaves the right one in the no-token arm, so the route reports no
// match and its tests fail rather than a start being used as an end.

/// A reading of a token from the bytes alone, or the finding that the run
/// around it has to be lexed to make one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Probe {
    Settled(Read),
    Lex,
}

/// The bounds within which a token ending at a word byte is read.
struct WordRun {
    /// Start of the word run ending at the byte asked about.
    ws: usize,
    /// The non-whitespace around the word run.
    run_start: usize,
    run_end: usize,
    /// End of the word run's leading digits.
    digits_end: usize,
    /// [`RUN_BYTE`] over the whole run, when the scan that found the bounds
    /// covered it.
    seen: Option<u8>,
}

/// An identifier byte.
const fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// A punctuation byte that lexes as one token of its own and that no typed
/// token is joined through.
const fn plain_punct(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b',' | b';' | b'=' | b'<' | b'>' | b'!')
}

/// A byte of a run the reader settles from the bytes: a word byte, plain
/// punctuation, or a dot, a slash or a colon, which join only typed tokens
/// the bytes beside them give away.
const fn settled_byte(c: u8) -> bool {
    word_byte(c) || plain_punct(c) || c == b'.' || c == b'/' || c == b':'
}

/// A byte that keeps the reader from settling a run.
const RUN_UNSETTLED: u8 = 1;

/// A byte that joins a word run into a typed token: a dot, a slash or a colon.
const RUN_JOINS: u8 = 2;

/// [`RUN_UNSETTLED`] and [`RUN_JOINS`] for each byte, so a run answers both in
/// one pass with a load and an or.
static RUN_BYTE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut c = 0usize;
    while c < 256 {
        #[allow(clippy::cast_possible_truncation)]
        let b = c as u8;
        if !settled_byte(b) {
            t[c] |= RUN_UNSETTLED;
        }
        if b == b'.' || b == b'/' || b == b':' {
            t[c] |= RUN_JOINS;
        }
        c += 1;
    }
    t
};

/// Whether every byte of `run` is settled and no `:/` opens a URL or a
/// drive path in it.
fn settled_run(run: &[u8]) -> bool {
    run.iter().all(|&c| settled_byte(c)) && !run.windows(2).any(|w| w == b":/")
}

/// A byte of a base64 body among the settled bytes.
fn b64_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'/'
}

/// An input read for the tokens at chosen spans without a lex.
///
/// The engine matches a literal by text against a token of any kind, so a
/// route's question is whether the lexer makes a token of exactly a span. The
/// bytes settle most spans on their own. A word byte after a word run means no
/// token ends there: a word token runs on through it, and every typed token
/// that could end before a word byte refuses to. A run of word bytes and plain
/// punctuation - the non-whitespace around a span - holds no typed token
/// shorter than [`crate::lexer::TYPED_WORD_RUN`], each punctuation byte in it
/// is its own token, and the recognizers that step over whitespace do so only
/// into a digit; so a token starts where a word run does and, short of that
/// length, it is the word token the lexer's gate reads it as, unless a digit
/// opens the run: `2alpha` is a number then a word, `2ms` a duration. There,
/// and wherever another byte in the run could join the span into a wider
/// token, as `alpha` is joined inside `http://alpha.com`, the lexer decides
/// over the run alone, which reads as the whole input does but for two
/// constructs. A string closing inside the run would open one in the run's
/// own lex, so the lex starts after it; [`crate::parallel_lex::quoted_spans`]
/// is the lexer's own quote reader, escapes included, and also settles that a
/// span inside a string is no token. And a phone number spanning a space into
/// the run's leading digits ends where they do in the whole input's lex, where
/// the run's own lex may fuse them into what follows; when it has, the answer
/// is unsettled.
///
/// A high-entropy run is one token whatever its bytes read as, and the flag
/// that decides it depends on the sixty-four bytes before each byte and on
/// nothing else, so the whitespace-free run around a span is read for blobs
/// on its own, seeded as the whole input's pass is seeded, and a run the
/// lexer takes whole holds no token but itself. The strings are read as the
/// lexer reads them: a quote inside such a run opens none, nor does the quote
/// inside a char literal `'"'`; a quote a URL could have taken cannot be
/// settled from the bytes, and then nothing is answered.
pub(crate) struct ByteReader<'a> {
    input: &'a [u8],
    /// The double-quoted strings, ascending, inclusive of both quote bytes,
    /// read only as far as a position asked about.
    ///
    /// Reading them all up front puts the whole input's cost under every
    /// answer, so a literal thirty bytes in costs what one at the far end
    /// costs. The routes ask about positions they have already found, which is
    /// the first match for a caller that stops at one, so the scan reaches that
    /// match and stops with it.
    quotes: std::cell::RefCell<crate::parallel_lex::QuoteScan<'a>>,
    /// The number of strings opening at or before the last position asked
    /// about. A route asks about positions that ascend, so the next answer is
    /// a step from here rather than a search over every string.
    quote_cursor: std::cell::Cell<usize>,
    /// The whitespace-free run last asked about.
    run: std::cell::Cell<RunCache>,
    /// The maximal word run last asked about.
    word: std::cell::Cell<(usize, usize)>,
    /// Each lexed run's tokens go into this buffer; `lexed` is the byte range
    /// they cover.
    scratch: Vec<crate::token::Token>,
    lexed: (usize, usize),
    /// Whether a quote the rule could not settle has been reached by the scan,
    /// so that nothing decided after it can be trusted.
    ///
    /// A cell because the scan reaches it while a probe is reading, and the
    /// probes take the reader by shared reference. It is discovered rather
    /// than decided up front: a quote past the last position asked about has
    /// not been reached, and cannot spoil an answer that never depended on it.
    unsettled: std::cell::Cell<bool>,
}

/// A whitespace-free run: its bounds, whether the lexer takes it whole as a
/// blob, and the [`RUN_BYTE`] flags of the bytes the bound scans read.
#[derive(Clone, Copy, Default)]
struct RunCache {
    start: usize,
    end: usize,
    blob: bool,
    /// [`RUN_BYTE`] over `start..s` and `e..end`, which is what the two bound
    /// scans walk, for the `s` and `e` from which the bounds were found.
    seen: u8,
    /// Whether `seen` covers the whole run rather than only the two ends,
    /// which it does when the caller vouched for the gap between its `s` and
    /// `e`.
    seen_whole: bool,
}

impl RunCache {
    /// The run holding `s..e`, from the cache when it holds it.
    ///
    /// `gap_is_clear` says every byte of `s..e` has a zero [`RUN_BYTE`] entry.
    /// The bound scans do not read that gap, so a caller able to vouch for it
    /// gets flags covering the whole run for an or per byte the scan already
    /// loaded; a caller that cannot vouch passes `false` and the flags stand
    /// for the two ends alone. The caller must justify vouching, and the two
    /// that do justify it differently: over a word run, because no word byte
    /// carries a flag, and over a single byte, by reading that byte's entry.
    fn run_of(&mut self, input: &[u8], s: usize, e: usize, gap_is_clear: bool) -> RunCache {
        if !(self.start < self.end && self.start <= s && e <= self.end) {
            let mut seen = 0u8;
            let mut start = s;
            for &c in input[..s].iter().rev() {
                if c.is_ascii_whitespace() {
                    break;
                }
                seen |= RUN_BYTE[c as usize];
                start -= 1;
            }
            let mut end = e;
            for &c in &input[e..] {
                if c.is_ascii_whitespace() {
                    break;
                }
                seen |= RUN_BYTE[c as usize];
                end += 1;
            }
            let blob = end - start >= crate::spectral::BLOB_MIN_LEN
                && !crate::lexer::blob_runs_in_span(input, start, end).is_empty();
            *self = RunCache { start, end, blob, seen, seen_whole: gap_is_clear };
        }
        *self
    }
}

impl<'a> ByteReader<'a> {
    pub(crate) fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            quotes: std::cell::RefCell::new(crate::parallel_lex::QuoteScan::new(input)),
            quote_cursor: std::cell::Cell::new(0),
            run: std::cell::Cell::new(RunCache::default()),
            word: std::cell::Cell::new((0, 0)),
            scratch: Vec::new(),
            lexed: (0, 0),
            unsettled: std::cell::Cell::new(false),
        }
    }

    /// Read the quote scan far enough to decide `p`.
    ///
    /// The rule it reads under: the lexer opens no string and no char literal
    /// at a quote inside a blob run, which it takes whole. A URL runs up to a
    /// double quote and over a single one, so a single quote with `://` before
    /// it in its run may be a URL's byte instead of a token's start, and that
    /// is not settled.
    ///
    /// The run cache the rule consults is this reader's own, so a position
    /// asked about twice costs its run once.
    fn quotes_through(&self, p: usize) {
        let input = self.input;
        self.quotes.borrow_mut().ensure(p, &mut |q: usize| {
            // The byte at `q` is a quote, which no scan here reads, so this
            // vouches for nothing.
            //
            // The run cache is read through its cell rather than through a copy
            // carried across this call, so it is touched where a quote is
            // settled and not once at every position asked about.
            //
            // This runs while the quote scan is mutably borrowed, so whatever
            // it calls must not reach that scan: `run_of` reads `run` and
            // `input` and nothing else, and a later edit giving it a quote
            // question would be a borrow panic at run time with nothing at
            // compile time to say so.
            let run = self.run_of(q, q + 1, false);
            if run.blob {
                return Some(false);
            }
            let in_url = input[q] == b'\'' && input[run.start..q].windows(3).any(|w| w == b"://");
            (!in_url).then_some(true)
        });
        if self.quotes.borrow().unsettled() {
            self.unsettled.set(true);
        }
    }

    /// The whitespace-free run holding `s..e`, where `gap_is_clear` says every
    /// byte of `s..e` has a zero [`RUN_BYTE`] entry.
    fn run_of(&self, s: usize, e: usize, gap_is_clear: bool) -> RunCache {
        let mut runs = self.run.get();
        let run = runs.run_of(self.input, s, e, gap_is_clear);
        self.run.set(runs);
        run
    }

    /// The maximal word run holding `p`, a word byte.
    fn word_run_at(&self, p: usize) -> (usize, usize) {
        let (ws, we) = self.word.get();
        if (ws..we).contains(&p) {
            return (ws, we);
        }
        let input = self.input;
        let ws = p - input[..p].iter().rev().take_while(|&&c| word_byte(c)).count();
        let we = p + input[p..].iter().take_while(|&&c| word_byte(c)).count();
        self.word.set((ws, we));
        (ws, we)
    }

    /// The closing quote of the string holding `p`.
    ///
    /// Reads the quote scan up to `p` first, so a caller that never asks about
    /// the far end of the input never scans it.
    fn string_end(&self, p: usize) -> Option<usize> {
        self.string_span(p).map(|(_, e)| e)
    }

    /// The string holding `p`, opening and closing quote included.
    ///
    /// [`Self::string_end`] reports where such a string closes, which is all a
    /// caller stepping over one needs. A caller naming the token needs where it
    /// opens too: a quote byte that is not the opener is inside a string that
    /// began earlier, and the token is that string rather than one starting
    /// here.
    fn string_span(&self, p: usize) -> Option<(usize, usize)> {
        self.quotes_through(p);
        let scan = self.quotes.borrow();
        let quotes = scan.spans();
        let mut k = self.quote_cursor.get();
        while k > 0 && quotes[k - 1].0 > p {
            k -= 1;
        }
        while k < quotes.len() && quotes[k].0 <= p {
            k += 1;
        }
        self.quote_cursor.set(k);
        (k > 0 && quotes[k - 1].1 >= p).then(|| quotes[k - 1])
    }

    /// Whether the quote scan has met a quote it could not settle.
    ///
    /// Asked after reading rather than at construction, because the scan
    /// reaches a position only when a caller asks about it: a quote the rule cannot
    /// settle past the last position asked about has not been met and does not
    /// spoil an answer that never depended on it.
    fn unsettled(&self) -> bool {
        self.unsettled.get()
    }

    /// Whether the bytes of `start..end` settle the word run `ws..e` inside
    /// it as the word token the lexer's gate reads it as: the run holds only
    /// settled bytes, and no dot, slash or colon in it joins the word run
    /// into a typed token. A JWT opens with `eyJ`; a measure with its unit
    /// opens with a digit, then digits, a dot and digits before the unit; a
    /// version is a dot and a digit after its `v1`; a path opens with a
    /// slash at the run's start or after an opening bracket, or with `./`; a
    /// base64 blob is a run of word bytes other than underscore and slashes
    /// of at least TYPED_WORD_RUN; an address or a MAC is groups of at most
    /// four hex digits about colons, ending before no word byte. Every other
    /// token a dot, a slash or a colon joins carries a byte outside the set,
    /// or the `:/` the set refuses.
    fn settles_word(&self, word: &WordRun, e: usize) -> bool {
        let (start, end, ws) = (word.run_start, word.run_end, word.ws);
        let input = self.input;
        if self.quantity_reaches(word) != Some(false) {
            return false;
        }
        let run = &input[start..end];
        // Both per-byte questions at once, off a table: is every byte settled,
        // and does a dot, a slash or a colon occur. A run with none joins
        // nothing, and the rest of this function is about what a join could
        // have made, so that run is settled here - which is almost every run,
        // and this runs at every occurrence of a literal.
        //
        // The bound scan carries these flags when it vouched for its gap, and
        // this pass runs only when it did not.
        //
        // The loop below answers the same two and four more that are about
        // where a byte is rather than which byte it is. It runs for the runs
        // that do join.
        let seen = word.seen.unwrap_or_else(|| {
            let mut seen = 0u8;
            for &c in run {
                seen |= RUN_BYTE[c as usize];
            }
            seen
        });
        if seen & RUN_UNSETTLED != 0 {
            return false;
        }
        if seen & RUN_JOINS == 0 {
            return true;
        }
        let (mut joins, mut jwt, mut path, mut colon) = (false, false, false, false);
        let mut prev = 0u8;
        for (i, &c) in run.iter().enumerate() {
            if !settled_byte(c) {
                return false;
            }
            match c {
                b'/' => {
                    if prev == b':' {
                        return false;
                    }
                    joins = true;
                    path |= i == 0 || matches!(prev, b'(' | b'[' | b'{' | b'.');
                }
                b'.' => joins = true,
                b':' => {
                    joins = true;
                    colon = true;
                }
                b'J' => jwt |= i >= 2 && run[i - 2] == b'e' && prev == b'y',
                _ => {}
            }
            prev = c;
        }
        if !joins {
            return true;
        }
        // Whether the word run before a dot before `ws` opens with a digit, as
        // a measure's does.
        let number_before_dot = ws >= 2 && input[ws - 1] == b'.' && {
            let dot = ws - 1;
            let before = dot - input[..dot].iter().rev().take_while(|&&c| word_byte(c)).count();
            before < dot && input[before].is_ascii_digit()
        };
        if jwt
            || path
            || number_before_dot
            || (input.get(e) == Some(&b'.') && input.get(e + 1).is_some_and(u8::is_ascii_digit))
            || (colon && e - ws <= 4 && input[ws..e].iter().all(u8::is_ascii_hexdigit))
        {
            return false;
        }
        let body = input[..e].iter().rev().take_while(|&&c| b64_byte(c)).count()
            + input[e..end].iter().take_while(|&&c| b64_byte(c)).count();
        body < crate::lexer::TYPED_WORD_RUN
    }

    /// The bounds within which the token ending at `e` is read, where the byte
    /// before `e` is a word byte and the byte at `e` is not; `None` when the
    /// run around it is a blob, which is the only token there.
    fn word_run_ending_at(&self, e: usize) -> Option<WordRun> {
        let input = self.input;
        let (ws, _) = self.word_run_at(e - 1);
        // `ws..e` is inside the maximal word run holding `e - 1`, so every
        // byte of it is a word byte and the bound scans carry the whole run's
        // flags.
        let run = self.run_of(ws, e, true);
        if run.blob {
            return None;
        }
        let digits_end = ws + input[ws..e].iter().take_while(|&&c| c.is_ascii_digit()).count();
        Some(WordRun {
            ws,
            run_start: run.start,
            run_end: run.end,
            digits_end,
            seen: run.seen_whole.then_some(run.seen),
        })
    }

    /// The lexer over `start..end`, a run holding no blob, read from after
    /// any string that closes inside it into the scratch buffer unless the
    /// buffer holds that lex already, returning the offset from which the
    /// token positions count.
    fn lex_run(&mut self, start: usize, end: usize) -> usize {
        let from = self.string_end(start).map_or(start, |close| close + 1);
        if self.lexed != (from, end) {
            crate::lexer::lex_run_into(&self.input[from..end], &mut self.scratch);
            self.lexed = (from, end);
        }
        from
    }

    /// The token ending at `e` as the bytes read it, where the byte before
    /// `e` is a word byte and the byte at `e` is not, or that the run has to
    /// be lexed by [`Self::lex_ending_at`].
    fn probe_ending_at(&self, e: usize) -> Probe {
        self.probe_ending_at_carrying(e, true)
    }

    /// [`Self::probe_ending_at`], with `carry` saying whether the settle may
    /// read the flags the bound scan carried or has to make its own pass.
    ///
    /// `carry` is a constant at both call sites. `false` reaches it only from
    /// the pricing arm that exists to time the two against each other inside
    /// one sweep, where a difference of a percent is readable; between
    /// processes this route has read 3.99, 4.26 and 4.73 ms for the same work,
    /// and a percent is not readable there at all.
    fn probe_ending_at_carrying(&self, e: usize, carry: bool) -> Probe {
        let quoted = self.string_end(e - 1).is_some();
        // The scan met a quote its rule cannot settle on the way here, so what
        // it reports about this position is not an answer.
        if self.unsettled() {
            return Probe::Settled(Read::Unsettled);
        }
        if quoted {
            return Probe::Settled(Read::NoToken);
        }
        let Some(mut run) = self.word_run_ending_at(e) else {
            return Probe::Settled(Read::NoToken);
        };
        if !carry {
            run.seen = None;
        }
        if run.digits_end == run.ws
            && e - run.ws < crate::lexer::TYPED_WORD_RUN
            && self.settles_word(&run, e)
        {
            return Probe::Settled(Read::TokenStartingAt(crate::token::TokenKind::Word, run.ws));
        }
        Probe::Lex
    }

    /// What a quantity from the run before does to `run`: `Some(true)` when
    /// the run is the unit symbol one separator after a number, so the token
    /// the lexer makes there covers the number and this run together and
    /// neither the run's own bytes nor its own lex is that token; `Some(false)`
    /// when no quantity reaches it; `None` when only the lexer can say.
    ///
    /// The byte before the run decides almost every call, which is why it is
    /// asked first: only a separator can come between a quantity's number and
    /// its unit symbol, and its last byte is a space or a byte of the three
    /// unicode spaces SI writes.
    fn quantity_reaches(&self, run: &WordRun) -> Option<bool> {
        let input = self.input;
        if run.ws < 2 || !(input[run.ws - 1] == b' ' || input[run.ws - 1] >= 0x80) {
            return Some(false);
        }
        crate::quantity::joins_the_number_before(input, run.ws)
    }

    /// Whether a phone number from the run before reaches into `run`, whose
    /// lex in the scratch buffer counts positions from `from`. A phone number
    /// ends its last digit group at the first byte that is not a digit,
    /// whatever that byte is, and steps over one space to reach the group.
    /// One reaching into this run covers exactly the run's leading digits in
    /// the whole input's lex, so a token of the run's own lex that runs on
    /// past them is not that lex's.
    fn phone_reaches(&self, run: &WordRun, from: usize) -> bool {
        let input = self.input;
        run.digits_end > run.ws
            && run.run_start == run.ws
            && run.run_start >= 2
            && input[run.run_start - 1] == b' '
            && input[run.run_start - 2].is_ascii_digit()
            && self.scratch.first().is_some_and(|t| from + t.end() > run.digits_end)
    }

    /// The token ending at `e` as the lexer reads its run, after
    /// [`Self::probe_ending_at`] found the run had to be lexed.
    fn lex_ending_at(&mut self, e: usize) -> Read {
        let Some(run) = self.word_run_ending_at(e) else {
            return Read::NoToken;
        };
        match self.quantity_reaches(&run) {
            Some(true) => return Read::NoToken,
            None => return Read::Unsettled,
            Some(false) => {}
        }
        let from = self.lex_run(run.run_start, run.run_end);
        if self.phone_reaches(&run, from) {
            return Read::Unsettled;
        }
        match self.scratch.iter().find(|t| from + t.end() == e) {
            Some(t) => Read::TokenStartingAt(t.kind, from + t.start()),
            None => Read::NoToken,
        }
    }

    /// The end of the word run opening at `s`, a word byte.
    fn word_run_end(&self, s: usize) -> usize {
        self.word_run_at(s).1
    }

    /// The token starting at `s`, a letter or underscore that no letter or
    /// underscore precedes, as the bytes read it - its kind and its end - or
    /// that the run has to be lexed by [`Self::lex_starting_at`]. A digit
    /// before `s` may end a number there or hold `s` inside a wider token,
    /// which the lex tells apart.
    fn probe_starting_at(&self, s: usize) -> Probe {
        let quoted = self.string_end(s).is_some();
        if self.unsettled() {
            return Probe::Settled(Read::Unsettled);
        }
        if quoted {
            return Probe::Settled(Read::NoToken);
        }
        let e = self.word_run_end(s);
        let Some(run) = self.word_run_ending_at(e) else {
            return Probe::Settled(Read::NoToken);
        };
        // `run.ws == s` first, so the run this settles opens where the token
        // asked about does and its `ws` is the `s` this reads under.
        if run.ws == s
            && e - s < crate::lexer::TYPED_WORD_RUN
            && self.settles_word(&run, e)
        {
            return Probe::Settled(Read::TokenEndingAt(crate::token::TokenKind::Word, e));
        }
        Probe::Lex
    }

    /// The token starting at `s` as the lexer reads its run, its kind and its
    /// end, after [`Self::probe_starting_at`] found the run had to be lexed.
    fn lex_starting_at(&mut self, s: usize) -> Read {
        let e = self.word_run_end(s);
        let Some(run) = self.word_run_ending_at(e) else {
            return Read::NoToken;
        };
        match self.quantity_reaches(&run) {
            Some(true) => return Read::NoToken,
            None => return Read::Unsettled,
            Some(false) => {}
        }
        let from = self.lex_run(run.run_start, run.run_end);
        if self.phone_reaches(&run, from) {
            return Read::Unsettled;
        }
        match self.scratch.iter().find(|t| from + t.start() == s) {
            Some(t) => Read::TokenEndingAt(t.kind, from + t.end()),
            None => Read::NoToken,
        }
    }

    /// The token at the plain punctuation byte at `q` as the bytes read it,
    /// or that the run has to be lexed by [`Self::lex_punct_at`].
    fn probe_punct_at(&self, q: usize) -> Probe {
        let input = self.input;
        let quoted = self.string_end(q).is_some();
        if self.unsettled() {
            return Probe::Settled(Read::Unsettled);
        }
        if quoted {
            return Probe::Settled(Read::NoToken);
        }
        // The gap the bound scans skip is the single byte at `q`, so its entry
        // is read rather than argued.
        let run = self.run_of(q, q + 1, RUN_BYTE[input[q] as usize] == 0);
        let (run_start, run_end) = (run.start, run.end);
        if run.blob {
            return Probe::Settled(Read::NoToken);
        }
        // A run carrying neither flag holds no unsettled byte and no colon, so
        // it holds no `:/` either and the two passes below are already
        // answered.
        let settled = (run.seen_whole && run.seen & (RUN_UNSETTLED | RUN_JOINS) == 0)
            || settled_run(&input[run_start..run_end]);
        // In a run of settled bytes the byte is its own token, unless it is
        // `=` padding a base64 blob, whose body is at least TYPED_WORD_RUN
        // base64 bytes before the padding.
        if settled {
            let body = input[run_start..q]
                .iter()
                .rev()
                .skip_while(|&&c| c == b'=')
                .take_while(|&&c| b64_byte(c))
                .count();
            if input[q] != b'=' || body < crate::lexer::TYPED_WORD_RUN {
                return Probe::Settled(Read::TokenAt(crate::lexer::punct_kind(input[q]), q));
            }
        }
        Probe::Lex
    }

    /// The token at the plain punctuation byte at `q` as the lexer reads its
    /// run, after [`Self::probe_punct_at`] found the run had to be lexed.
    fn lex_punct_at(&mut self, q: usize) -> Read {
        let RunCache { start: run_start, end: run_end, .. } =
            self.run_of(q, q + 1, RUN_BYTE[self.input[q] as usize] == 0);
        let from = self.lex_run(run_start, run_end);
        match self.scratch.iter().find(|t| from + t.start() == q && from + t.end() == q + 1) {
            Some(t) => Read::TokenAt(t.kind, q),
            None => Read::NoToken,
        }
    }
}

/// Remove from `out` the entries marked in `dropped`.
fn drop_marked(out: &mut Vec<crate::engine::Span>, dropped: &[bool]) {
    let mut i = 0;
    out.retain(|_| {
        i += 1;
        !dropped[i - 1]
    });
}

/// The matches of `lit` as a whole token, decided as [`ByteReader`] decides,
/// or `None` when the bytes cannot settle one of them and the lexer must run
/// over the whole input.
#[must_use]
pub fn byte_route_word_literal(lit: &str, input: &[u8]) -> Option<Vec<crate::engine::Span>> {
    let mut reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    route_word_literal(&mut reader, lit)
}

/// [`byte_route_word_literal`] reaching each occurrence when it asks for it,
/// rather than searching a span of the input ahead.
///
/// The two answer alike and cost differently, and this exists so a bench can
/// hold both in one process and one rotation. Every decision about an
/// occurrence is shared between them, so the only thing that differs is where
/// the occurrences come from, and a reading of the two is a reading of that.
#[must_use]
pub fn byte_route_word_literal_unsplit(
    lit: &str,
    input: &[u8],
) -> Option<Vec<crate::engine::Span>> {
    let mut reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    let pat = lit.as_bytes();
    if pat.is_empty() {
        return None;
    }
    route_word_literal_over(&mut reader, pat, crate::byte_simd::occurrences_unsplit(input, pat))
}

/// How many occurrences of `lit` survive the two byte tests, without asking the
/// reader whether a token ends at any of them.
///
/// A cost and never a result: the reader's probe is what decides whether an
/// occurrence is a whole token, so leaving it out over-reports by every
/// occurrence inside a string, inside a blob run, or under a typed
/// recognizer that reaches across its edge. It returns a count rather than
/// spans so nothing can mistake it for an answer.
///
/// It exists because the walk over occurrences is 3.7360 ms of this route's
/// 4.2625 on 7.34 MB - seventy-four nanoseconds per occurrence, which is more
/// than everything above it put together - and that figure says nothing about
/// which half is the byte tests and which the probe.
#[must_use]
pub fn byte_route_word_literal_unprobed(lit: &str, input: &[u8]) -> Option<usize> {
    // Built and its settling checked even though nothing here probes it, so this
    // arm carries the reader's construction cost exactly as the route does and
    // the subtraction between them is the probing alone.
    let reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    let pat = lit.as_bytes();
    if pat.is_empty() {
        return None;
    }
    let mut kept = 0usize;
    for s in crate::byte_simd::occurrences_unsplit(input, pat) {
        let e = s + pat.len();
        if input.get(e).is_some_and(|&c| word_byte(c)) {
            continue;
        }
        if s > 0 && (input[s - 1].is_ascii_alphabetic() || input[s - 1] == b'_') {
            continue;
        }
        kept += 1;
    }
    Some(kept)
}

/// [`byte_route_word_literal_unprobed`] with the probe's quote question asked
/// and its word-run question left out.
///
/// A cost and never a result, for the same reason and returning a count for the
/// same reason. What it adds over the unprobed arm is `string_end` at every
/// occurrence, which is what the probe asks first; what it still leaves out is
/// finding the word run and settling it.
///
/// The probe is 74.2% of this route at seventy nanoseconds per occurrence, and
/// the two questions inside it have different answers if they need fixing: one
/// reads a scan of the input's quoting, the other reads the run's own bytes.
#[must_use]
pub fn byte_route_word_literal_quote_only(lit: &str, input: &[u8]) -> Option<usize> {
    let reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    let pat = lit.as_bytes();
    if pat.is_empty() {
        return None;
    }
    let mut kept = 0usize;
    for s in crate::byte_simd::occurrences_unsplit(input, pat) {
        let e = s + pat.len();
        if input.get(e).is_some_and(|&c| word_byte(c)) {
            continue;
        }
        if s > 0 && (input[s - 1].is_ascii_alphabetic() || input[s - 1] == b'_') {
            continue;
        }
        if reader.string_end(e - 1).is_some() || reader.unsettled() {
            continue;
        }
        kept += 1;
    }
    Some(kept)
}

/// [`byte_route_word_literal_quote_only`] with the word run found and not
/// settled.
///
/// A cost and never a result, for the same reason and returning a count for the
/// same reason. It counts the occurrences whose run is not a blob, which is what
/// [`ByteReader::word_run_ending_at`] answers, so it runs the scans that find
/// the run and not `settles_word`.
///
/// It exists because the run question is forty-eight nanoseconds per
/// occurrence and two different things are inside it. Finding the run is four
/// scans over the bytes around the occurrence, and the one-entry caches miss on
/// nearly all of them because consecutive occurrences of a literal are in
/// different runs.
/// Settling it is one table pass over the run, which returns on the first test
/// for a run holding no dot, slash or colon. Either could be the forty-eight,
/// and they have nothing in common: the first is answered by asking once per run
/// instead of once per occurrence, the second by asking a cheaper question.
#[must_use]
pub fn byte_route_word_literal_run_only(lit: &str, input: &[u8]) -> Option<usize> {
    let reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    let pat = lit.as_bytes();
    if pat.is_empty() {
        return None;
    }
    let mut kept = 0usize;
    for s in crate::byte_simd::occurrences_unsplit(input, pat) {
        let e = s + pat.len();
        if input.get(e).is_some_and(|&c| word_byte(c)) {
            continue;
        }
        if s > 0 && (input[s - 1].is_ascii_alphabetic() || input[s - 1] == b'_') {
            continue;
        }
        if reader.string_end(e - 1).is_some() || reader.unsettled() {
            continue;
        }
        if reader.word_run_ending_at(e).is_some() {
            kept += 1;
        }
    }
    Some(kept)
}

/// The occurrences of `lit` the reader settles as a whole word token, with
/// `carry` saying whether the settle reads the flags the bound scan carried.
///
/// A cost and never a result, returning a count for the reason the arms above
/// do. The two wrappers below differ in `carry` and in nothing else, so the
/// difference between them inside one sweep is the carried pass and nothing
/// else - which is the only way to read it, this route having moved 18% between
/// processes doing identical work.
fn route_word_literal_priced(input: &[u8], pat: &[u8], carry: bool) -> Option<usize> {
    let reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    if pat.is_empty() {
        return None;
    }
    let mut kept = 0usize;
    for s in crate::byte_simd::occurrences_unsplit(input, pat) {
        let e = s + pat.len();
        if input.get(e).is_some_and(|&c| word_byte(c)) {
            continue;
        }
        if s > 0 && (input[s - 1].is_ascii_alphabetic() || input[s - 1] == b'_') {
            continue;
        }
        if let Probe::Settled(Read::TokenStartingAt(_, start)) =
            reader.probe_ending_at_carrying(e, carry)
            && start == s
        {
            kept += 1;
        }
    }
    Some(kept)
}

/// [`route_word_literal_priced`] with the bound scan's flags read.
#[must_use]
pub fn byte_route_word_literal_carried(lit: &str, input: &[u8]) -> Option<usize> {
    route_word_literal_priced(input, lit.as_bytes(), true)
}

/// [`route_word_literal_priced`] with the settle making its own pass over the
/// run, which is the work the carried flags replace.
#[must_use]
pub fn byte_route_word_literal_rescanned(lit: &str, input: &[u8]) -> Option<usize> {
    route_word_literal_priced(input, lit.as_bytes(), false)
}

fn route_word_literal(reader: &mut ByteReader<'_>, lit: &str) -> Option<Vec<crate::engine::Span>> {
    let pat = lit.as_bytes();
    if pat.is_empty() {
        return None;
    }
    let input = reader.input;
    // Each occurrence reached when this asks for one. Searching a span ahead and
    // splitting it across the cores halves the search and costs more than that
    // to hold the result: on 7.34 MB the split saves 0.4487 ms and the walk over
    // it gives back 0.5318, so the route reads 4.7031 ms against 4.2625 for this
    // form. `byte_route_word_literal_unsplit` is the other arm of that reading.
    route_word_literal_over(reader, pat, crate::byte_simd::occurrences_unsplit(input, pat))
}

/// The matches of `pat` as a whole token, decided at each position `at` yields.
///
/// `at` supplies the occurrences of `pat` in ascending order and nothing else.
/// What is decided about each one lives here, and stays serial: the reader is
/// asked in ascending order and carries what it has settled between asks.
fn route_word_literal_over(
    reader: &mut ByteReader<'_>,
    pat: &[u8],
    at: impl Iterator<Item = usize>,
) -> Option<Vec<crate::engine::Span>> {
    let input = reader.input;
    let mut out: Vec<crate::engine::Span> = Vec::new();
    // The occurrences whose run the lexer reads, as indices into `out`, read
    // after the pass: a run the bytes refuse hands the whole input to the
    // lexer, and the pass reaches it with no run lexed.
    let mut deferred: Vec<usize> = Vec::new();
    for s in at {
        let e = s + pat.len();
        // A word byte after the occurrence: no token ends there. A letter or
        // underscore before it: the token holding that byte runs on over it.
        if input.get(e).is_some_and(|&c| word_byte(c)) {
            continue;
        }
        if s > 0 && (input[s - 1].is_ascii_alphabetic() || input[s - 1] == b'_') {
            continue;
        }
        let span = crate::engine::Span { start: s as u32, end: e as u32 };
        match reader.probe_ending_at(e) {
            Probe::Settled(Read::TokenStartingAt(_, start)) if start == s => out.push(span),
            Probe::Settled(
                Read::TokenStartingAt(..)
                | Read::TokenEndingAt(..)
                | Read::TokenAt(..)
                | Read::NoToken,
            ) => {}
            Probe::Settled(Read::Unsettled) => return None,
            Probe::Lex => {
                deferred.push(out.len());
                out.push(span);
            }
        }
    }
    if !deferred.is_empty() {
        let mut dropped = vec![false; out.len()];
        for &i in &deferred {
            let span = out[i];
            dropped[i] = match reader.lex_ending_at(span.end as usize) {
                Read::TokenStartingAt(_, start) => start != span.start as usize,
                Read::TokenEndingAt(..) | Read::TokenAt(..) | Read::NoToken => true,
                Read::Unsettled => return None,
            };
        }
        drop_marked(&mut out, &dropped);
    }
    Some(out)
}

/// The atoms of a pattern that is a flat sequence of single-token atoms, some
/// of them bound, or `None` for any other shape.
///
/// This is the shape a window around one anchor can answer: every atom consumes
/// exactly one token, so a match spans exactly as many tokens as there are
/// atoms and a window holding that many past its anchor holds the whole match.
/// A repeat, an assertion, an anchor, a guard and a balanced group each break
/// that, and each is refused here rather than handled.
///
/// A group-scoped bind (`P::name`) is refused with them: its scope is the
/// enclosing balanced group, and there is none in this shape.
fn flat_atom_sequence(pattern: &crate::ast::Pattern) -> Option<Vec<&crate::ast::Atom>> {
    use crate::ast::Pattern;
    fn atom_of(p: &Pattern) -> Option<&crate::ast::Atom> {
        match p {
            Pattern::Atom(a) => Some(a),
            Pattern::Bind(_, false, inner) => atom_of(inner),
            _ => None,
        }
    }
    let atoms: Vec<&crate::ast::Atom> = match pattern {
        Pattern::Concat(parts) => parts.iter().map(atom_of).collect::<Option<Vec<_>>>()?,
        other => vec![atom_of(other)?],
    };
    atoms.iter().all(|a| decides_from_its_own_token(a)).then_some(atoms)
}

/// Whether an atom's answer depends only on the token being tested.
///
/// A window holds the match's own tokens and no more, so an atom reading
/// anything else - a register bound by another match, an axis computed over
/// the whole stream, a role read from an enclosing supertoken - is asking a
/// question the window cannot answer, and a window that answered it anyway
/// would be answering a different question from the scan's.
fn decides_from_its_own_token(atom: &crate::ast::Atom) -> bool {
    use crate::ast::Atom;
    match atom {
        Atom::Literal(_, crate::orbit::OrbitGroup::Identity)
        | Atom::Kind(_)
        | Atom::Any
        | Atom::Byte(_)
        | Atom::BytePattern(_) => true,
        Atom::Class(c) => {
            c.any.iter().chain(&c.all).chain(&c.none).all(decides_from_its_own_token)
        }
        _ => false,
    }
}

/// Whether `at` is inside one of `runs`, which are sorted and disjoint.
fn inside_a_run(runs: &[(usize, usize)], at: usize) -> bool {
    let i = runs.partition_point(|&(s, _)| s <= at);
    i > 0 && runs[i - 1].1 > at
}

/// The matches of `pattern` found by lexing only around the occurrences of its
/// opening literal word, or `None` where the bytes cannot settle it and the
/// lexer must run over the whole input.
///
/// A TREX atom is a whole token and this shape's match spans one token per
/// atom, so a match cannot begin anywhere but at an occurrence of the opening
/// literal - and [`route_word_literal`] already reports exactly those, deciding
/// whole-token-ness from the bytes and deferring to the lexer only over the
/// runs the bytes cannot settle. What is left is to read a few tokens at each
/// of them instead of every token in the input.
///
/// On the comparison's corpus that is about 200000 tokens against 1650000: the
/// lex is 3.032 ms for the whole input and the literal search 0.684 for all
/// 50000 occurrences of `let`.
///
/// Each window is grown until it holds one significant token more than the
/// pattern can span and the match ends clear of that token, so no answer here
/// rests on a token the window's edge may have cut short. A window that reaches
/// the input's end needs no such room: nothing was cut.
#[must_use]
pub fn scan_by_literal_windows(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Option<Vec<crate::engine::Span>> {
    scan_by_literal_windows_at(pattern, input, 0)
}

/// [`scan_by_literal_windows`] reporting only the matches at or after byte
/// `at`, which are the matches a scan resuming there reports.
///
/// A match begins at an occurrence of the opening literal and the occurrences
/// come in order, so the ones below `at` cannot open a match this reports and
/// are dropped before any window is read. The non-overlap cursor starts at
/// `at` for the same reason: a match reaching back across `at` belongs to the
/// scan that already passed it.
///
/// A resuming scan reports a match that starts exactly at `at`, so the
/// cursor's comparison has to admit it and the anchor cut has to keep it - both
/// are `>=`, and `>` at either place would lose the match at the boundary. A
/// pattern the route cannot settle still refuses with `None` rather than
/// reporting no matches, because an empty answer here and a declined route
/// mean opposite things to the caller.
#[must_use]
pub fn scan_by_literal_windows_at(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    at: usize,
) -> Option<Vec<crate::engine::Span>> {
    // The route's phases, named so a trace shows each one's share of the time.
    // The route's cost rises with how densely the literal occurs rather than
    // with the input's length - ten times the occurrences in the same bytes
    // measured five thousand times as long - and the trace shows which phase
    // takes the extra time.
    let finding = crate::trace::phase("the literal route: its anchors");
    let (anchors, compiled, atoms) = literal_window_anchors(pattern, input)?;
    drop(finding);
    let anchors = &anchors[anchors.partition_point(|a| (a.start as usize) < at)..];
    if anchors.is_empty() {
        return Some(Vec::new());
    }

    // The whole input's blob table, computed once for the same reason the
    // parallel lexer computes it once: a window-local gate reads a truncated
    // entropy history and can find a blob where a lex of the whole input finds
    // none.
    let tabling = crate::trace::phase("the literal route: the blob table");
    let blobs = crate::lexer::blob_runs_parallel(input);
    drop(tabling);
    let reserve = atoms + 1;
    let guarding = crate::trace::phase("the literal route: the absent guards");
    let absent = absent_guard_literals(pattern, input);
    drop(guarding);

    // One window is independent of every other, so they are dispatched the way
    // the per-anchor scan dispatches its attempts, with a leaf's buffers made
    // once and reused across its own anchors.
    let starts: Vec<usize> = anchors.iter().map(|a| a.start as usize).collect();
    // The span alone: a scan reports where its matches are, and the pattern
    // may bind nothing at all.
    let scanning = crate::trace::phase("the literal route: the windows");
    let found = windows_matching(input, &starts, &blobs, &compiled, &absent, reserve, start_width(reserve));
    drop(scanning);

    // Leftmost, non-overlapping, as the scan reports them. Every match begins
    // at an anchor and the anchors are in order, so taking each one that starts
    // at or after the last end is the same selection the whole-input scan makes.
    let mut kept: Vec<crate::engine::Span> = Vec::with_capacity(found.len());
    let mut from = u32::try_from(at).expect("an offset within the span's width");
    for (s, sp) in found.into_iter().flatten() {
        let sp = crate::engine::Span {
            start: sp.start + s as u32,
            end: sp.end + s as u32,
        };
        if sp.start >= from {
            from = sp.end;
            kept.push(sp);
        }
    }
    Some(kept)
}

/// The matches of `pattern` in `input[..upto]` that the cut at `upto` cannot
/// have made or broken, and whether that prefix is the whole input.
///
/// For a caller taking matches one at a time from a growing prefix: it holds
/// these until they run out, then asks again with a wider prefix, so the prefix
/// widens once for the whole walk rather than once per request. Calling
/// [`find_at_by_growing_prefix`] for each request re-lexes from byte zero every
/// time, which is quadratic in the requests: the lib suite measured 10.05
/// seconds that way against 0.54 with this.
///
/// `None` is the refusal, for the patterns [`settles_from_a_prefix`] rejects
/// and for those the single-pass engine declines.
#[must_use]
pub fn settled_prefix_matches(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    upto: usize,
) -> Option<(Vec<crate::engine::Span>, bool)> {
    if !settles_from_a_prefix(pattern) {
        return None;
    }
    let upto = upto.min(input.len());
    let whole = upto == input.len();
    let prefix = &input[..upto];
    let toks = crate::lexer::lex(prefix);
    let found = crate::nfa::scan_nfa_over_serial(pattern, prefix, &toks)?;
    if whole {
        return Some((found, true));
    }
    // A match reaching into the last `reserve` significant tokens may have been
    // cut short by the prefix's edge, so it is left for a wider one. `reserve`
    // is the match's own token length plus how far past it a bounded assertion
    // can read, which is what `settle_first_from_a_prefix` reserves.
    let reserve = crate::nfa::bounded_max_len(pattern).unwrap_or(1).max(1)
        + pattern.widest_forward_window();
    let settled: Vec<crate::engine::Span> =
        found.into_iter().filter(|s| end_clear_of_the_cut(&toks, s.end(), reserve)).collect();
    Some((settled, false))
}

/// The first match of `pattern`, found by windowing the occurrences of its
/// opening literal in order and stopping at the first that matches, or `None`
/// where the bytes cannot settle it.
///
/// [`scan_by_literal_windows`] windows every occurrence because a scan reports
/// every match. A caller asking only whether there is one, or where the first
/// is, wants the first anchor that answers and nothing after it - which, when
/// the literal occurs early in the input, is a handful of bytes read instead
/// of seven megabytes.
///
/// `Some(None)` is a verdict of no match over the whole input: every anchor was
/// windowed and none matched. `None` is a refusal, and the ladder goes on.
#[must_use]
pub fn first_by_literal_windows(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Option<Option<crate::engine::Span>> {
    first_by_literal_windows_at(pattern, input, 0)
}

/// [`first_by_literal_windows`] for the first match at or after byte `at`.
///
/// The occurrences are searched from `at`, so a caller walking an input
/// searches only the stretch it is asking about, not the whole input. The
/// quote check still starts at byte zero, because a string opening before `at`
/// encloses what follows it whatever the caller asked.
#[must_use]
pub fn first_by_literal_windows_at(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    at: usize,
) -> Option<Option<crate::engine::Span>> {
    use crate::ast::Atom;
    if !settles_from_a_prefix(pattern) {
        crate::trace::rung("first windows", "no: a prefix cannot settle it", input.len());
        return None;
    }
    let Some(atoms) = flat_atom_sequence(pattern) else {
        crate::trace::rung("first windows", "no: not a flat sequence of plain atoms", input.len());
        return None;
    };
    let Some(Atom::Literal(lit, crate::orbit::OrbitGroup::Identity)) = atoms.first() else {
        crate::trace::rung("first windows", "no: it opens with no plain literal", input.len());
        return None;
    };
    if byte_routable_literal(&crate::ast::Pattern::Atom(Atom::Literal(
        (*lit).clone(),
        crate::orbit::OrbitGroup::Identity,
    )))
    .is_none()
    {
        crate::trace::rung("first windows", "no: the literal is not a word token", input.len());
        return None;
    }
    let Some(compiled) = crate::nfa::compile_pattern(pattern) else {
        crate::trace::rung("first windows", "no: the engine declines it", input.len());
        return None;
    };

    // Nothing here reads the whole input before the first answer. The occurrences
    // come one at a time from the byte search, whole-token-ness is decided from
    // the bytes either side, and the entropy table is computed over the region
    // the window can reach rather than over the input: a blob never holds
    // whitespace, so a run that touches the window is wholly inside that region
    // and nothing outside the region can change whether it is a blob.
    let pat = lit.as_bytes();
    let absent = absent_guard_literals(pattern, input);
    let reserve = atoms.len() + 1;
    let mut width = 4 * reserve + 8;
    let mut toks: Vec<crate::token::Token> = Vec::new();
    let mut seams = crate::lexer::Seams::default();
    let mut chunk: Vec<(usize, usize)> = Vec::new();
    let mut scratch = crate::nfa::AttemptScratch::for_program(&compiled);
    let mut tried = 0usize;
    let mut from = at.min(input.len());
    // How far the input has been shown to hold no quote byte. A string encloses
    // its contents in one token, so an occurrence inside one is not a token at
    // all and a window there would read a word where the lexer reads a string.
    // ByteReader settles that by mapping every literal, which is a pass over the
    // whole input; this route needs only that no string has opened before the
    // window it is about to read, so it looks at the bytes it has already walked
    // past and no others.
    let mut quote_free = 0usize;
    while let Some(rel) = crate::byte_simd::find(&input[from..], pat) {
        let s = from + rel;
        from = s + 1;
        if quote_free < s {
            if input[quote_free..s].iter().any(|&c| c == b'"' || c == b'\'') {
                crate::trace::rung("first windows", "no: a quote opens before the match", s);
                return None;
            }
            quote_free = s;
        }
        // The occurrence is a whole token only where no word byte abuts it: one
        // after means the token runs on, and a letter or underscore before means
        // the token holding that byte started earlier. The same two tests
        // `route_word_literal` makes before it consults the reader.
        if input.get(s + pat.len()).is_some_and(|&c| word_byte(c)) {
            continue;
        }
        if s > 0 && (input[s - 1].is_ascii_alphabetic() || input[s - 1] == b'_') {
            continue;
        }
        // The entropy table over the region the window can reach, widened at
        // both ends to the edges of the whitespace-free runs there. A blob never
        // holds whitespace, so one touching the window is wholly inside this
        // region, and nothing outside it can change whether it is a blob.
        let lo = run_start(input, s);
        let hi = run_end(input, (s + LOCAL_BLOB_REACH).min(input.len()));
        let local = crate::lexer::blob_runs(&input[lo..hi]);
        let blobs: Vec<(usize, usize)> = local.iter().map(|&(a, b)| (lo + a, lo + b)).collect();
        tried += 1;
        let Some((s, e)) =
            window_tokens(input, s, &blobs, reserve, &mut width, &mut toks, &mut seams, &mut chunk)
        else {
            continue;
        };
        if e > hi {
            // The window outgrew the region the table covers, so a blob beyond
            // it could be one this lex did not see. Refused rather than
            // answered: the ladder below reads the whole input and is right.
            crate::trace::rung("first windows", "no: a window outgrew its blob region", e - s);
            return None;
        }
        if let Some(sp) =
            crate::nfa::match_at_first_token(&compiled, &input[s..e], &toks, &mut scratch, &absent)
        {
            crate::trace::rung("first windows", "yes: found it", tried);
            return Some(Some(crate::engine::Span {
                start: sp.start + s as u32,
                end: sp.end + s as u32,
            }));
        }
    }
    crate::trace::rung("first windows", "yes: no match anywhere", tried);
    Some(None)
}

/// How far past an anchor the first-match route computes its entropy table.
///
/// A window starts at about a token's worth of bytes per atom and grows by
/// doubling, so this covers one that has doubled several times over a
/// long-token corpus. Past it the route refuses rather than lexing against a
/// table that may miss a blob inside the window.
const LOCAL_BLOB_REACH: usize = 4096;

/// The first byte of the whitespace-free run containing `at`.
fn run_start(input: &[u8], at: usize) -> usize {
    let mut lo = at.min(input.len());
    while lo > 0 && !input[lo - 1].is_ascii_whitespace() {
        lo -= 1;
    }
    lo
}

/// One past the last byte of the whitespace-free run containing `at`.
fn run_end(input: &[u8], at: usize) -> usize {
    let at = at.min(input.len());
    at + input[at..].iter().take_while(|&&c| !c.is_ascii_whitespace()).count()
}

/// The anchors where a match of `pattern` can begin, its compiled program, and
/// how many atoms it spans - or `None` where the window scans refuse it.
///
/// The check both window scans run, kept in one place: two copies of a refusal
/// could give two different answers to the same question.
fn literal_window_anchors(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Option<(Vec<crate::engine::Span>, crate::nfa::Compiled, usize)> {
    use crate::ast::Atom;
    if !settles_from_a_prefix(pattern) {
        crate::trace::rung("windows", "no: a prefix cannot settle this pattern", input.len());
        return None;
    }
    let Some(atoms) = flat_atom_sequence(pattern) else {
        crate::trace::rung("windows", "no: not a flat sequence of plain atoms", input.len());
        return None;
    };
    let Some(Atom::Literal(lit, crate::orbit::OrbitGroup::Identity)) = atoms.first() else {
        crate::trace::rung("windows", "no: it does not open with a plain literal", input.len());
        return None;
    };
    byte_routable_literal(&crate::ast::Pattern::Atom(Atom::Literal(
        (*lit).clone(),
        crate::orbit::OrbitGroup::Identity,
    )))?;
    // The byte literal route, ahead of the windows on the ladder, answers a
    // pattern that is one literal atom: each anchor is a whole match, found by
    // this same search with no lex, and there is no further token to confirm.
    if atoms.len() == 1 && byte_routable_literals(pattern).is_some() {
        crate::trace::rung("windows", "no: the byte literal route answers it whole", input.len());
        return None;
    }
    let compiled = crate::nfa::compile_pattern(pattern)?;
    let mut reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    let anchors = route_word_literal(&mut reader, lit)?;
    Some((anchors, compiled, atoms.len()))
}

/// Every match of `pattern` with the registers it bound, in one pass over the
/// windows, or `None` where the bytes cannot settle it.
///
/// [`scan_by_literal_windows`] and [`captures_by_windows`] together lex each
/// window twice and run each attempt twice: the first throws the save slots
/// away to report a span and the second rebuilds them from the same bytes. A
/// caller wanting the registers of every match asks for both at once here.
#[must_use]
pub fn scan_captures_by_literal_windows(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Option<Vec<crate::engine::Match>> {
    let (anchors, compiled, atoms) = literal_window_anchors(pattern, input)?;
    let blobs = crate::lexer::blob_runs_parallel(input);
    let absent = absent_guard_literals(pattern, input);
    let reserve = atoms + 1;
    let starts: Vec<usize> = anchors.iter().map(|a| a.start as usize).collect();
    let found =
        windows_capturing(input, &starts, &blobs, &compiled, &absent, reserve, start_width(reserve));
    // Leftmost, non-overlapping, as the scan reports them.
    let mut kept: Vec<crate::engine::Match> = Vec::with_capacity(found.len());
    let mut from = 0usize;
    for (s, mut m) in found.into_iter().flatten() {
        m.start += s;
        m.end += s;
        if m.start < from {
            continue;
        }
        from = m.end;
        for sp in m.captures_mut() {
            *sp = crate::engine::Span { start: sp.start + s as u32, end: sp.end + s as u32 };
        }
        kept.push(m);
    }
    Some(kept)
}

/// How many tokens the route's windows hold, lexed and never scanned.
///
/// A measure of cost, never a result: it returns a token count so nothing can
/// mistake it for matches. It takes the same anchors and the same dispatch as
/// [`scan_captures_by_literal_windows`] and hands `windows_over` an attempt
/// that only counts each window's tokens, so the only difference between the
/// two is the engine running over each window's tokens.
///
/// There is one window per anchor, each about [`start_width`] bytes, and the
/// comparison's corpus has fifty thousand of them, so this shows which of the
/// two costs - lexing that many small windows, or attempting a match in each -
/// is worth attacking.
#[doc(hidden)]
#[must_use]
pub fn anchor_windows_unscanned(pattern: &crate::ast::Pattern, input: &[u8]) -> Option<usize> {
    let (anchors, compiled, atoms) = literal_window_anchors(pattern, input)?;
    let blobs = crate::lexer::blob_runs_parallel(input);
    let absent = absent_guard_literals(pattern, input);
    let reserve = atoms + 1;
    let starts: Vec<usize> = anchors.iter().map(|a| a.start as usize).collect();
    let found = windows_over(
        input,
        &starts,
        &blobs,
        &compiled,
        &absent,
        reserve,
        start_width(reserve),
        |_, _, toks, _, _| Some(toks.len()),
    );
    Some(found.iter().flatten().map(|&(_, n)| n).sum())
}

/// Every match of `pattern` with its registers held inline, and the register
/// names in the same order, or `None` where the bytes cannot settle it or the
/// program is not the atom-a-token shape.
///
/// [`scan_captures_by_literal_windows`] reports the same matches as owned
/// [`crate::engine::Match`]es, which allocates one vector per match before the
/// caller has asked for any of them. A cursor handing matches out one at a time
/// takes these instead and builds at most one.
#[must_use]
pub fn scan_flat_by_literal_windows(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Option<(Vec<crate::nfa::FlatMatch>, std::sync::Arc<[String]>)> {
    let (anchors, compiled, atoms) = literal_window_anchors(pattern, input)?;
    let shape = crate::nfa::flat_shape(&compiled)?;
    let blobs = crate::lexer::blob_runs_parallel(input);
    let absent = absent_guard_literals(pattern, input);
    let reserve = atoms + 1;
    let starts: Vec<usize> = anchors.iter().map(|a| a.start as usize).collect();
    let found = windows_over(
        input,
        &starts,
        &blobs,
        &compiled,
        &absent,
        reserve,
        start_width(reserve),
        |c, i, t, s, _| crate::nfa::flat_regs_at_first_token(c, &shape, i, t, s),
    );
    // Leftmost, non-overlapping, as the scan reports them.
    let mut kept: Vec<crate::nfa::FlatMatch> = Vec::with_capacity(found.len());
    let mut from = 0u32;
    for (s, m) in found.into_iter().flatten() {
        let m = rebased(m, s as u32);
        if m.span.start < from {
            continue;
        }
        from = m.span.end;
        kept.push(m);
    }
    Some((kept, crate::nfa::names_of(&compiled)))
}

/// Whether [`flat_captures_by_byte_bounds`] can walk this pattern's shape, so a
/// caller can take the cheaper scan and resolve rather than a window scan that
/// lexes each window to recover what the bytes already say.
///
/// The shape alone. Whether a particular match walks is settled against its own
/// bytes by the resolve itself.
#[must_use]
pub fn bytes_walk_the_shape(pattern: &crate::ast::Pattern) -> bool {
    crate::nfa::compile_pattern(pattern)
        .and_then(|c| crate::nfa::flat_shape(&c))
        .is_some_and(|s| s.walks_from_bytes())
}

/// The registers each of `spans` bound, read off the bytes with no lex at all,
/// or `None` where the pattern's program is not the atom-a-token shape or any
/// span's atoms do not reconstruct it.
///
/// The route that found these spans never lexed, and every other resolve here
/// lexes something to say what they bound - a window per span in
/// [`flat_captures_by_windows`], the whole input below that. For a shape whose
/// atoms are literals and word tokens the bytes say it directly: the atoms walk
/// forward from each span's start and land on the registers' tokens.
///
/// All or none, for the reason [`captures_by_windows`] gives.
#[must_use]
pub fn flat_captures_by_byte_bounds(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    spans: &[crate::engine::Span],
) -> Option<(Vec<crate::nfa::FlatMatch>, std::sync::Arc<[String]>)> {
    let compiled = crate::nfa::compile_pattern(pattern)?;
    let shape = crate::nfa::flat_shape(&compiled)?;
    let out: Option<Vec<crate::nfa::FlatMatch>> =
        spans.iter().map(|&s| shape.regs_from_bytes(input, s)).collect();
    Some((out?, crate::nfa::names_of(&compiled)))
}

/// The registers each of `spans` bound, held inline, or `None` where no window
/// answers for the pattern or a window at any span refuses.
///
/// The inline form of [`captures_by_windows`], for the same caller as
/// [`scan_flat_by_literal_windows`]: a cursor reading out, one at a time, the
/// registers of matches a route found without lexing.
#[must_use]
pub fn flat_captures_by_windows(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    spans: &[crate::engine::Span],
) -> Option<(Vec<crate::nfa::FlatMatch>, std::sync::Arc<[String]>)> {
    if !settles_from_a_prefix(pattern) {
        return None;
    }
    let atoms = flat_atom_sequence(pattern)?;
    let compiled = crate::nfa::compile_pattern(pattern)?;
    let shape = crate::nfa::flat_shape(&compiled)?;
    let blobs = crate::lexer::blob_runs_parallel(input);
    let absent = absent_guard_literals(pattern, input);
    let reserve = atoms.len() + 1;
    let starts: Vec<usize> = spans.iter().map(|s| s.start()).collect();
    let found = windows_over(
        input,
        &starts,
        &blobs,
        &compiled,
        &absent,
        reserve,
        start_width(reserve),
        |c, i, t, s, _| crate::nfa::flat_regs_at_first_token(c, &shape, i, t, s),
    );
    // All or none, for the reason `captures_by_windows` gives: a span with no
    // window means the whole-input path must answer for every span.
    let out: Option<Vec<crate::nfa::FlatMatch>> =
        found.into_iter().map(|slot| slot.map(|(s, m)| rebased(m, s as u32))).collect();
    let out = out?;
    (out.len() == spans.len() && out.iter().zip(spans).all(|(m, s)| m.span.start() == s.start()))
        .then_some((out, crate::nfa::names_of(&compiled)))
}

/// `m`, whose positions count from its window's start, moved to count from the
/// input's start.
fn rebased(mut m: crate::nfa::FlatMatch, at: u32) -> crate::nfa::FlatMatch {
    let shift = |s: crate::engine::Span| crate::engine::Span { start: s.start + at, end: s.end + at };
    m.span = shift(m.span);
    for r in &mut m.regs {
        *r = shift(*r);
    }
    m
}

/// The registers each of `spans` bound, resolved over a window at each, or
/// `None` where the pattern's shape has no window or a window at any span
/// refuses.
///
/// The spans are a scan's own matches, and a route found them without lexing
/// the input; reading a whole lex back to say what they bound costs exactly
/// what the route saved. A window is about a token's worth of bytes per atom,
/// which bounds the cost per span where settling a region for each would not.
#[must_use]
pub fn captures_by_windows(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    spans: &[crate::engine::Span],
) -> Option<Vec<crate::engine::Match>> {
    if !settles_from_a_prefix(pattern) {
        return None;
    }
    let atoms = flat_atom_sequence(pattern)?;
    let compiled = crate::nfa::compile_pattern(pattern)?;
    let blobs = crate::lexer::blob_runs_parallel(input);
    let absent = absent_guard_literals(pattern, input);
    let reserve = atoms.len() + 1;
    let starts: Vec<usize> = spans.iter().map(|s| s.start()).collect();
    let found =
        windows_capturing(input, &starts, &blobs, &compiled, &absent, reserve, start_width(reserve));
    // All or none: a span with no window means the whole-input path must answer
    // for every span, rather than this handing back a list to be merged with it.
    let out: Option<Vec<crate::engine::Match>> = found
        .into_iter()
        .map(|slot| {
            slot.map(|(s, mut m)| {
                m.start += s;
                m.end += s;
                for sp in m.captures_mut() {
                    *sp = crate::engine::Span {
                        start: sp.start + s as u32,
                        end: sp.end + s as u32,
                    };
                }
                m
            })
        })
        .collect();
    let out = out?;
    (out.len() == spans.len() && out.iter().zip(spans).all(|(m, s)| m.start == s.start()))
        .then_some(out)
}

/// [`windows_over`] reporting each window's match, taking the atom-a-token
/// attempt where the compiled program is that shape and the simulation
/// otherwise.
#[allow(clippy::too_many_arguments)]
fn windows_matching(
    input: &[u8],
    starts: &[usize],
    blobs: &[(usize, usize)],
    compiled: &crate::nfa::Compiled,
    absent: &std::collections::HashSet<Vec<u8>>,
    reserve: usize,
    width: usize,
) -> Vec<Option<(usize, crate::engine::Span)>> {
    match crate::nfa::flat_shape(compiled) {
        Some(shape) => windows_over(input, starts, blobs, compiled, absent, reserve, width, |c, i, t, s, _| {
            crate::nfa::flat_match_at_first_token(c, &shape, i, t, s)
        }),
        None => windows_over(
            input,
            starts,
            blobs,
            compiled,
            absent,
            reserve,
            width,
            crate::nfa::match_at_first_token,
        ),
    }
}

/// [`windows_matching`] reporting what each window's match bound with it.
#[allow(clippy::too_many_arguments)]
fn windows_capturing(
    input: &[u8],
    starts: &[usize],
    blobs: &[(usize, usize)],
    compiled: &crate::nfa::Compiled,
    absent: &std::collections::HashSet<Vec<u8>>,
    reserve: usize,
    width: usize,
) -> Vec<Option<(usize, crate::engine::Match)>> {
    match crate::nfa::flat_shape(compiled) {
        Some(shape) => windows_over(input, starts, blobs, compiled, absent, reserve, width, |c, i, t, s, _| {
            crate::nfa::flat_captures_at_first_token(c, &shape, i, t, s)
        }),
        None => windows_over(
            input,
            starts,
            blobs,
            compiled,
            absent,
            reserve,
            width,
            crate::nfa::captures_at_first_token,
        ),
    }
}

/// A window's starting byte width, for a shape whose match spans
/// `reserve - 1` tokens.
///
/// Four bytes per token, plus eight. The windows are lexed serially and their
/// bytes add up, so a wider start costs more than it saves: fifty thousand
/// windows of 160 bytes is eight megabytes against a 7.34 MB input, and
/// measures 27.878 ms against 2.941 for one parallel lex of the whole input. A
/// window too narrow for its shape doubles, and its leaf carries the settled
/// width to the next anchor.
fn start_width(reserve: usize) -> usize {
    4 * reserve + 8
}

/// Whether `toks` already holds one significant token past what a match can
/// read, ending strictly before `edge` bytes into the window.
///
/// The confirmation in `windows_over` widens a window until its tokens stop
/// changing, and this is what lets it stop at the pattern's reach instead of
/// the input's end. A match reads at most `reserve` significant tokens from the
/// anchor, and the only token a wider lex can read differently is one the edge
/// cut, so a whole token past the match's last one, inside the window, settles
/// every token the match read. A window holding fewer than that answers false
/// and is widened.
fn settled_past_the_match(toks: &[crate::token::Token], reserve: usize, edge: usize) -> bool {
    toks.iter()
        .filter(|t| t.is_significant())
        .nth(reserve)
        .is_some_and(|t| t.end() < edge)
}

/// A window at each of `starts`, with `attempt` run over each one's tokens,
/// across the cores: one window is independent of every other, and a leaf's
/// buffers are made once and reused across its own windows.
///
/// Each answer comes back with the byte where its window started, because
/// what the attempt reports is in the window's own offsets and only the caller
/// knows what it wants rebased.
#[allow(clippy::too_many_arguments)]
fn windows_over<T: Send>(
    input: &[u8],
    starts: &[usize],
    blobs: &[(usize, usize)],
    compiled: &crate::nfa::Compiled,
    absent: &std::collections::HashSet<Vec<u8>>,
    reserve: usize,
    start_width: usize,
    attempt: impl Fn(
        &crate::nfa::Compiled,
        &[u8],
        &[crate::token::Token],
        &mut crate::nfa::AttemptScratch,
        &std::collections::HashSet<Vec<u8>>,
    ) -> Option<T>
    + Sync,
) -> Vec<Option<(usize, T)>> {
    let mut found: Vec<Option<(usize, T)>> = Vec::new();
    found.resize_with(starts.len(), || None);
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let min_leaf = found.len().div_ceil(cores * 4).max(64);
    let plan = flynnel::JobPlan::new(0, found.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::Streaming)
        .with_estimated_per_item_ns(WINDOW_NS_ESTIMATE);
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(
        &plan,
        &mut found,
        min_leaf,
        |base, slots| {
            let mut width = start_width;
            let mut toks: Vec<crate::token::Token> = Vec::new();
            let mut seams = crate::lexer::Seams::default();
            let mut chunk: Vec<(usize, usize)> = Vec::new();
            // A second set, for confirming a window that matched against a
            // wider one. Made once per leaf like the first, and only touched
            // by the windows that match.
            let mut wide_toks: Vec<crate::token::Token> = Vec::new();
            let mut wide_seams = crate::lexer::Seams::default();
            let mut wide_chunk: Vec<(usize, usize)> = Vec::new();
            let mut scratch = crate::nfa::AttemptScratch::for_program(compiled);
            for (i, slot) in slots.iter_mut().enumerate() {
                let Some((s, e)) = window_tokens(
                    input,
                    starts[base + i],
                    blobs,
                    reserve,
                    &mut width,
                    &mut toks,
                    &mut seams,
                    &mut chunk,
                ) else {
                    continue;
                };
                // The same slice the lexer saw, so an atom reading a token's
                // bytes and a guard reading the forward window both see the
                // window and not the rest of the input.
                let Some(answer) = attempt(compiled, &input[s..e], &toks, &mut scratch, absent)
                else {
                    continue;
                };
                // A window that matched is confirmed against a wider one
                // before it is believed. Counting the tokens cannot show that
                // a window is wide enough: a cut edge can split one token
                // into two that are both valid, which raises the count while
                // changing what the match reads. Only a wider lex can say.
                //
                // This runs where a window matched, never where it missed, so
                // a scan runs it once per match rather than once per window.
                let mut settled = e;
                while settled < input.len() {
                    // A match reads at most `reserve` significant tokens from
                    // the anchor. Once the window holds one past that and it
                    // ends before the edge, no wider lex can change what the
                    // match read: widening moves the right edge, and the only
                    // token a wider lex can read differently is one the edge
                    // cut. Without this the doubling is bounded by the input
                    // rather than by the pattern's reach, so one match can lex
                    // its way out to the whole input - measured at 4753 ms of a
                    // 4756 ms scan over 7.34 MB, against 1.5 ms to find every
                    // anchor in it.
                    if settled_past_the_match(&toks, reserve, settled - s) {
                        break;
                    }
                    let wider = (s + (settled - s) * 2).min(input.len());
                    lex_window_into(
                        input,
                        s,
                        wider,
                        blobs,
                        &mut wide_toks,
                        &mut wide_seams,
                        &mut wide_chunk,
                    );
                    if same_tokens_within(&toks, &wide_toks, settled - s) {
                        break;
                    }
                    std::mem::swap(&mut toks, &mut wide_toks);
                    settled = wider;
                }
                if settled == e {
                    *slot = Some((s, answer));
                    continue;
                }
                // The lex changed under a wider window, so the answer taken
                // from the narrower one is not this input's answer. The carried
                // width grows to the wider window, since a shape that needed
                // the room at one anchor needs it at the next.
                width = width.max(settled - s);
                *slot = attempt(compiled, &input[s..settled], &toks, &mut scratch, absent)
                    .map(|t| (s, t));
            }
        },
    );
    found
}

/// The per-item cost estimate the window dispatch hands the scheduler. A window
/// lexes about a token's worth of bytes per atom and runs one anchored attempt
/// over them, measured at about 170 ns when its buffers are held.
///
/// The measurement itself, not a rounded figure: naming a cost replaces the
/// scheduler's own probe, so the number has to be a measured one. The lexer's
/// estimate rounds the other way and says why.
const WINDOW_NS_ESTIMATE: u32 = 170;

/// Lex `input[s..e]` into `toks`, with the blob runs that are wholly inside it.
///
/// The tokens count from the window's start, as [`window_tokens`] produces
/// them, so the two can be compared token for token.
fn lex_window_into(
    input: &[u8],
    s: usize,
    e: usize,
    blobs: &[(usize, usize)],
    toks: &mut Vec<crate::token::Token>,
    seams: &mut crate::lexer::Seams,
    chunk: &mut Vec<(usize, usize)>,
) {
    toks.clear();
    seams.open.clear();
    seams.close.clear();
    chunk.clear();
    let lo = blobs.partition_point(|&(a, _)| a < s);
    let hi = blobs.partition_point(|&(a, _)| a < e);
    chunk.extend(blobs[lo..hi].iter().filter(|&&(_, b)| b <= e).map(|&(a, b)| (a - s, b - s)));
    crate::lexer::lex_chunk_into(&input[s..e], chunk, toks, seams);
}

/// Whether `narrow` and `wide` agree about every token that is wholly within
/// the first `upto` bytes, both being lexes of windows that begin at the same
/// byte.
///
/// A token count cannot answer this. A narrow window's edge can end a token
/// early, and the bytes it left behind then lex as further tokens of their own,
/// so the narrow window holds one token more than the wide one over the same
/// bytes while describing them wrongly: the count rises while the tokens are
/// wrong. Only comparing the tokens themselves catches it.
fn same_tokens_within(
    narrow: &[crate::token::Token],
    wide: &[crate::token::Token],
    upto: usize,
) -> bool {
    let within = |t: &&crate::token::Token| t.end() <= upto;
    let mut a = narrow.iter().filter(within);
    let mut b = wide.iter().filter(within);
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(x), Some(y))
                if x.kind == y.kind && x.start() == y.start() && x.end() == y.end() => {}
            _ => return false,
        }
    }
}

/// Lex a window at the token starting at byte `s` into `toks`, reporting the
/// window's own bounds, or `None` where no window there can be trusted.
///
/// The window grows until it holds `reserve` significant tokens, which is one
/// more than the shape's match can span - so the match ends at or before the
/// last of them and no answer rests on a token the window's edge may have cut
/// short. A window reaching the input's end needs no such room. `width` is
/// carried in and out, so a leaf's later anchors start at the width its earlier
/// ones reached rather than growing again at each.
#[allow(clippy::too_many_arguments)]
fn window_tokens(
    input: &[u8],
    s: usize,
    blobs: &[(usize, usize)],
    reserve: usize,
    width: &mut usize,
    toks: &mut Vec<crate::token::Token>,
    seams: &mut crate::lexer::Seams,
    chunk: &mut Vec<(usize, usize)>,
) -> Option<(usize, usize)> {
    // To a whole lex, an occurrence inside a high-entropy run is part of that
    // run's single token, and a window starting at it reads a word instead: the
    // blob slice below keeps only the runs wholly inside the window, so a run
    // straddling its start is not there to say otherwise.
    if inside_a_run(blobs, s) {
        return None;
    }
    loop {
        let e = (s + *width).min(input.len());
        let whole = e == input.len();
        // A run straddling the far edge is dropped for the same reason, so the
        // window grows past it rather than being lexed without it.
        if !whole && inside_a_run(blobs, e) {
            *width *= 2;
            continue;
        }
        let slice = &input[s..e];
        toks.clear();
        // Cleared rather than replaced: the leaf keeps these across its own
        // anchors, and assigning a default drops the capacity the last window's
        // brackets allocated, so the next window with brackets allocates again.
        seams.open.clear();
        seams.close.clear();
        chunk.clear();
        // The runs of the whole input's table that are wholly inside the
        // window. Runs ascend and do not overlap, so those starting in it are
        // one contiguous part of the table and only the last can reach past
        // the far edge; bisecting for that part keeps a window's cost
        // independent of how many runs the input holds.
        let lo = blobs.partition_point(|&(a, _)| a < s);
        let hi = blobs.partition_point(|&(a, _)| a < e);
        chunk.extend(blobs[lo..hi].iter().filter(|&&(_, b)| b <= e).map(|&(a, b)| (a - s, b - s)));
        crate::lexer::lex_chunk_into(slice, chunk, toks, seams);
        if !whole && toks.iter().filter(|t| t.is_significant()).count() < reserve {
            *width *= 2;
            continue;
        }
        // The window holds one significant token more than the shape's match
        // can span, and this shape spans exactly one token per atom, so the
        // match ends at or before the last token's start whatever it is - which
        // is what `end_clear_of_the_cut` asks, already answered by the count.
        return Some((s, e));
    }
}

#[cfg(test)]
mod literal_window_tests {
    use super::scan_by_literal_windows;

    fn spans(v: &[crate::engine::Span]) -> Vec<(usize, usize)> {
        v.iter().map(|s| (s.start(), s.end())).collect()
    }

    /// The window path answers what the whole-input scan answers, or refuses.
    /// A wrong answer is the failure; a refusal is not.
    fn agrees(src: &str, text: &str) {
        let p = crate::parser::parse(src).expect("parses");
        let input = text.as_bytes();
        let Some(got) = scan_by_literal_windows(&p, input) else {
            return;
        };
        assert_eq!(spans(&got), spans(&crate::engine::scan(&p, input)), "{src}");
    }

    #[test]
    fn a_window_answers_what_the_whole_scan_answers() {
        let mut plain = String::new();
        for i in 0..400u32 {
            plain.push_str(&format!("let value_{i} = {} ; call_{i}(a, b) ;\n", i * 37));
        }
        // A blob the entropy gate takes as one token, an unclosed quote that
        // encloses the rest of its file, a literal glued to other word bytes,
        // and the literal as the very last token with nothing after it.
        let mut blob = String::from("let a = 1 ;\nx ");
        for i in 0..900u32 {
            blob.push_str(&format!("aZ9+let+kQ2mX7pL4vB8n{}", i % 7));
        }
        blob.push_str(" ;\nlet b = 2 ;\n");
        for (src, text) in [
            ("\"let\" \\W \"=\"", plain.as_str()),
            ("\"let\" \\W:v \"=\"", plain.as_str()),
            ("\"let\"", plain.as_str()),
            ("\"let\" \\W", plain.as_str()),
            ("\"let\" \\W \"=\"", blob.as_str()),
            ("\"let\"", blob.as_str()),
            ("\"let\" \\W \"=\"", "let a = 1 ; let b = \"never closed ; let c = 3 ;\n"),
            ("\"let\"", "let a = 1 ; let b = \"never closed ; let c = 3 ;\n"),
            ("\"let\" \\W \"=\"", "xlet y = 1 ; letx z = 2 ; let w = 3 ;\n"),
            ("\"let\"", "a b let"),
            ("\"let\" \\W \"=\"", "a b let"),
            ("\"let\"", ""),
            ("\"let\" \\W \"=\"", "let a = 1 ; let a = 2 ; let a = 3 ;\n"),
        ] {
            agrees(src, text);
        }
    }

    /// A pattern that is one literal atom is left to the byte literal route,
    /// which reports the same spans from the same search without lexing a
    /// window at any of them. Both window scans refuse it, and the scan that
    /// takes over after the refusal still finds every match.
    #[test]
    fn one_literal_atom_is_left_to_the_route_that_answers_it() {
        let mut text = String::new();
        for i in 0..400u32 {
            text.push_str(&format!("let value_{i} = {} ; call_{i}(alpha, beta) ;\n", i * 37));
        }
        let input = text.as_bytes();
        for src in ["\"let\"", "\"alpha\""] {
            let p = crate::parser::parse(src).expect("parses");
            assert!(scan_by_literal_windows(&p, input).is_none(), "{src} took the windows");
            assert!(
                super::scan_captures_by_literal_windows(&p, input).is_none(),
                "{src} took the capturing windows"
            );
            let found = crate::engine::scan(&p, input);
            let lit = src.trim_matches('"');
            assert_eq!(
                spans(&found),
                spans(&super::byte_route_word_literals(&[lit], input).expect("the bytes settle it")),
                "{src}"
            );
            assert_eq!(found.len(), 400, "{src}");
        }
    }

    /// When asked from an offset, the stopping window must give the first match
    /// at or after it - the same answer filtering the whole scan gives.
    #[test]
    fn stopping_from_an_offset_gives_the_scans_first_match_there() {
        let mut text = String::new();
        for i in 0..400u32 {
            text.push_str(&format!("let value_{i} = {} ; call_{i}(a, b) ;\n", i * 37));
        }
        let input = text.as_bytes();
        for src in ["\"let\" \\W \"=\"", "\"let\" \\W:v \"=\"", "\"let\""] {
            let p = crate::parser::parse(src).expect("parses");
            let all = crate::engine::scan(&p, input);
            // Every match's start and end, one past the last, and the far end:
            // the offsets a caller walking an input actually uses.
            let mut offsets: Vec<usize> = all.iter().flat_map(|s| [s.start(), s.end()]).collect();
            offsets.push(0);
            offsets.push(input.len());
            offsets.push(input.len() + 1);
            for at in offsets.into_iter().take(64) {
                let Some(got) = super::first_by_literal_windows_at(&p, input, at) else {
                    continue;
                };
                let want = all.iter().copied().find(|s| s.start() >= at);
                assert_eq!(
                    got.map(|s| (s.start(), s.end())),
                    want.map(|s| (s.start(), s.end())),
                    "{src} from {at}"
                );
            }
        }
    }

    /// Stopping at the first anchor that matches must give the first match the
    /// whole scan gives, and a no-match must be a no-match - never a refusal
    /// reported as no match, which is the contract a prefilter breaks first.
    #[test]
    fn stopping_at_the_first_window_gives_the_scans_first_match() {
        let mut plain = String::new();
        for i in 0..400u32 {
            plain.push_str(&format!("let value_{i} = {} ; call_{i}(a, b) ;\n", i * 37));
        }
        // The literal late, the literal absent as a token though present as
        // bytes, an unclosed quote, and the literal at the very end.
        let late = format!("{}\nlet z = 1 ;\n", "x y ; ".repeat(4000));
        let bytes_only = "xlet y = 1 ; letx z = 2 ; letter w = 3 ;\n".repeat(200);
        let unclosed = "a = 1 ; b = \"never closed ; let c = 3 ;\n".to_string();
        for (src, text) in [
            ("\"let\" \\W \"=\"", &plain),
            ("\"let\" \\W:v \"=\"", &plain),
            ("\"let\"", &plain),
            ("\"let\" \\W \"=\"", &late),
            ("\"let\" \\W \"=\"", &bytes_only),
            ("\"let\"", &bytes_only),
            ("\"let\" \\W \"=\"", &unclosed),
            ("\"let\" \\W \"=\"", &String::new()),
        ] {
            let p = crate::parser::parse(src).expect("parses");
            let input = text.as_bytes();
            let Some(got) = super::first_by_literal_windows(&p, input) else {
                continue;
            };
            let want = crate::engine::scan(&p, input).into_iter().next();
            assert_eq!(
                got.map(|s| (s.start(), s.end())),
                want.map(|s| (s.start(), s.end())),
                "{src} over {} bytes",
                input.len()
            );
        }
    }

    /// One pass that matches and resolves together must give what two passes
    /// give: the same spans as the scan, and the same registers as resolving
    /// those spans afterward.
    #[test]
    fn one_pass_over_the_windows_gives_what_two_passes_give() {
        let mut text = String::new();
        for i in 0..400u32 {
            text.push_str(&format!("let value_{i} = {} ; call_{i}(a, b) ;\n", i * 37));
        }
        let input = text.as_bytes();
        for src in ["\"let\" \\W:v \"=\"", "\"let\" \\W \"=\"", "\"let\" \\W:v \"=\" \\N:n"] {
            let p = crate::parser::parse(src).expect("parses");
            let Some(got) = super::scan_captures_by_literal_windows(&p, input) else {
                continue;
            };
            let spans = crate::engine::scan(&p, input);
            let want = crate::engine::captures(&p, input, &spans);
            assert_eq!(got.len(), want.len(), "{src}: a different number of matches");
            for (g, w) in got.iter().zip(&want) {
                assert_eq!((g.start, g.end), (w.start, w.end), "{src}: a different span");
                assert_eq!(g.captures(), w.captures(), "{src}: different registers at {}", g.start);
            }
        }
    }

    /// A window at each span must bind what a whole lex of the input binds:
    /// both the names and the spans, on patterns that bind and on the corpora
    /// that break a naive window.
    #[test]
    fn a_window_at_each_span_binds_what_the_whole_input_binds() {
        let mut plain = String::new();
        for i in 0..400u32 {
            plain.push_str(&format!("let value_{i} = {} ; call_{i}(a, b) ;\n", i * 37));
        }
        let unclosed = "let a = 1 ; let b = \"never closed ; let c = 3 ;\n".to_string();
        for (src, text) in [
            ("\"let\" \\W:v \"=\"", &plain),
            ("\"let\" \\W:v \"=\" \\N:n", &plain),
            ("\\W:name \"=\"", &plain),
            ("\"let\" \\W:v \"=\"", &unclosed),
            ("\\W:a \\W:b", &plain),
        ] {
            let p = crate::parser::parse(src).expect("parses");
            let input = text.as_bytes();
            let all = crate::engine::scan(&p, input);
            let Some(got) = super::captures_by_windows(&p, input, &all) else {
                continue;
            };
            let want = crate::nfa::captures_over_parts(&p, input, &all)
                .expect("the single-pass engine takes these");
            assert_eq!(got.len(), want.len(), "{src}: a different number of matches");
            for (g, w) in got.iter().zip(&want) {
                assert_eq!((g.start, g.end), (w.start, w.end), "{src}: a different span");
                assert_eq!(g.captures(), w.captures(), "{src}: different registers at {}", g.start);
            }
        }
    }

    /// A binding records what a match consumed and constrains nothing, so a
    /// pattern whose bindings nothing reads back matches exactly the spans the
    /// same pattern without its bindings matches - which is what lets the
    /// routes answer it. A pattern that reads one back is a different matcher
    /// and must not be stripped.
    #[test]
    fn an_unread_binding_changes_no_span() {
        let mut text = String::new();
        for i in 0..400u32 {
            text.push_str(&format!("let value_{i} = {} ; call_{i}(a, b) ;\n", i * 37));
        }
        let input = text.as_bytes();
        for (bound, bare) in [
            ("\\W:name \"=\"", "\\W \"=\""),
            ("\"let\" \\W:v \"=\"", "\"let\" \\W \"=\""),
            ("\\W:a \\W:b", "\\W \\W"),
            ("(\\W:x | \\N:y) \"=\"", "(\\W | \\N) \"=\""),
            ("\\N:n", "\\N"),
        ] {
            let bp = crate::parser::parse(bound).expect("parses");
            let rp = crate::parser::parse(bare).expect("parses");
            assert_eq!(bp.without_bindings().as_ref(), Some(&rp), "{bound} strips to {bare}");
            assert_eq!(
                spans(&crate::engine::scan(&bp, input)),
                spans(&crate::engine::scan(&rp, input)),
                "{bound} and {bare} match the same spans"
            );
        }
        // A back-reference reads what was bound, so the binding is part of the
        // matcher and stripping it would change the answer.
        for src in ["\\W:x \"=\" =x", "\\W:x [\\N =x]", "\\W:x ~(=x)"] {
            let p = crate::parser::parse(src).expect("parses");
            assert!(p.without_bindings().is_none(), "{src} reads a register back");
        }
        // Nothing to strip is also a refusal, so the ladder does not run twice.
        assert!(crate::parser::parse("\\W \"=\"").expect("parses").without_bindings().is_none());
    }

    #[test]
    fn it_refuses_what_a_window_cannot_hold() {
        // Each of these reads something the match's own tokens do not carry, or
        // spans a number of tokens a window cannot bound: a repeat, an
        // assertion, an anchor, a guard, a back-reference, a balanced group,
        // and an opening atom that is not a plain literal word.
        for src in [
            "\"let\" \\W*",
            "\"let\" ~(\\N)",
            "^ \"let\" \\W",
            "\"let\" ~\"zzz\"",
            "\"let\" \\W:x =x",
            "\\B(\"let\")",
            "\\W \"=\"",
            "\"let\" \\K \\W",
            "(?orbit:case \"let\") \\W",
        ] {
            let p = crate::parser::parse(src).expect("parses");
            assert!(
                scan_by_literal_windows(&p, b"let a = 1 ;").is_none(),
                "{src} must be refused"
            );
        }
    }
}

/// The byte pattern of `pattern` and its literal prefix, when `pattern` is one
/// byte-pattern atom whose literal prefix begins with a letter or an
/// underscore, so the matches can be found from every occurrence of the prefix.
#[must_use]
pub fn byte_routable_byte_pattern(
    pattern: &crate::ast::Pattern,
) -> Option<(&crate::bytepat::BytePat, Vec<u8>)> {
    use crate::ast::{Atom, Pattern};
    let Pattern::Atom(Atom::BytePattern(bp)) = pattern else {
        return None;
    };
    let prefix = bp.literal_prefix();
    let opens = prefix.first().is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_');
    opens.then_some((bp, prefix))
}

/// Whether the byte pattern `bp` matches a whole token anywhere, or `None`
/// when the bytes cannot settle the first occurrence they reach.
///
/// [`byte_route_byte_pattern`] walks every occurrence of the prefix because
/// it must report them all. This stops at the first that is a match, and
/// lexes a deferred occurrence as soon as it is found rather than batching it
/// until the end of the pass, which this may never reach.
#[must_use]
pub fn byte_route_any_byte_pattern(
    bp: &crate::bytepat::BytePat,
    prefix: &[u8],
    input: &[u8],
) -> Option<bool> {
    Some(byte_route_first_byte_pattern(bp, prefix, input)?.is_some())
}

/// Where the byte pattern `bp` first matches a whole token, read to that match
/// and no further, or `None` where the bytes cannot settle it.
///
/// The span form of [`byte_route_any_byte_pattern`]. [`byte_route_byte_pattern`]
/// walks every occurrence of the prefix in the input because it must report
/// them all, which a caller wanting only the first match does not need.
#[must_use]
pub fn byte_route_first_byte_pattern(
    bp: &crate::bytepat::BytePat,
    prefix: &[u8],
    input: &[u8],
) -> Option<Option<crate::engine::Span>> {
    byte_route_first_byte_pattern_reading(bp, prefix, &mut ByteReader::new(input), 0)
}

/// [`byte_route_first_byte_pattern`] beginning the search at byte `from`, over
/// a reader the caller holds, under the ascending-ask constraint
/// [`byte_route_first_number_reading`] states.
///
/// A match opens with `prefix` and the span starts where that occurrence does,
/// so bounding the search at `from` is enough here and no span needs filtering
/// out - unlike [`byte_route_first_word_then_punct_reading`], where the byte
/// searched for is the match's last and its word may begin earlier.
#[must_use]
pub(crate) fn byte_route_first_byte_pattern_reading(
    bp: &crate::bytepat::BytePat,
    prefix: &[u8],
    reader: &mut ByteReader<'_>,
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    let input = reader.input;
    if reader.unsettled() {
        return None;
    }
    let mut from = from.min(input.len());
    while let Some(rel) = crate::byte_simd::find(&input[from..], prefix) {
        let s = from + rel;
        from = s + 1;
        // A letter or underscore before the occurrence: the token holding
        // that byte runs on over it, so no token opens here.
        if s > 0 && (input[s - 1].is_ascii_alphabetic() || input[s - 1] == b'_') {
            continue;
        }
        let end = match reader.probe_starting_at(s) {
            Probe::Settled(Read::TokenEndingAt(_, e)) => e,
            Probe::Settled(Read::TokenStartingAt(..) | Read::TokenAt(..) | Read::NoToken) => {
                continue;
            }
            Probe::Settled(Read::Unsettled) => return None,
            Probe::Lex => match reader.lex_starting_at(s) {
                Read::TokenEndingAt(_, e) => e,
                Read::TokenStartingAt(..) | Read::TokenAt(..) | Read::NoToken => continue,
                Read::Unsettled => return None,
            },
        };
        if bp.matches_whole(&input[s..end]) {
            return Some(Some(crate::engine::Span { start: s as u32, end: end as u32 }));
        }
    }
    Some(None)
}

/// The matches of the byte pattern `bp`, each a whole token, or `None` when
/// the bytes cannot settle one of them. They are found from the occurrences of
/// `prefix`, the bytes every match opens with, that open a token as
/// [`ByteReader`] reads tokens.
///
/// Cut across the cores from [`CUT_ROUTE_THRESHOLD`], in one pass below it.
#[must_use]
pub fn byte_route_byte_pattern(
    bp: &crate::bytepat::BytePat,
    prefix: &[u8],
    input: &[u8],
) -> Option<Vec<crate::engine::Span>> {
    if input.len() >= CUT_ROUTE_THRESHOLD {
        crate::trace::rung("byte route", "byte pattern, cut across the cores", input.len());
        return cut_across_cores(input, |chunk| byte_route_byte_pattern_one_pass(bp, prefix, chunk));
    }
    crate::trace::rung("byte route", "byte pattern, one pass", input.len());
    byte_route_byte_pattern_one_pass(bp, prefix, input)
}

/// [`byte_route_byte_pattern`] in one pass whatever the size: the form the cut
/// route runs over each chunk, and the A/B's baseline.
#[must_use]
pub fn byte_route_byte_pattern_one_pass(
    bp: &crate::bytepat::BytePat,
    prefix: &[u8],
    input: &[u8],
) -> Option<Vec<crate::engine::Span>> {
    let mut reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    // The byte machine where the pattern has one, built once for the pass: the
    // tree walk it replaces is 47 nanoseconds per token and runs at every
    // occurrence of the prefix.
    let machine = bp.automaton();
    let whole = |t: &[u8]| machine.as_ref().map_or_else(|| bp.matches_whole(t), |m| m.matches_whole(t));
    let mut out: Vec<crate::engine::Span> = Vec::new();
    // The occurrences whose run the lexer reads, as indices into `out`, their
    // ends filled in after the pass.
    let mut deferred: Vec<usize> = Vec::new();
    // Each occurrence is found when this asks for the next one, for the reason
    // given at `route_word_literal`: holding a span's worth costs more than
    // splitting the search over it saves.
    for s in crate::byte_simd::occurrences_unsplit(input, prefix) {
        // A letter or underscore before the occurrence: the token holding
        // that byte runs on over it, so no token opens here.
        if s > 0 && (input[s - 1].is_ascii_alphabetic() || input[s - 1] == b'_') {
            continue;
        }
        match reader.probe_starting_at(s) {
            Probe::Settled(Read::TokenEndingAt(_, e)) => {
                if whole(&input[s..e]) {
                    out.push(crate::engine::Span { start: s as u32, end: e as u32 });
                }
            }
            Probe::Settled(Read::TokenStartingAt(..) | Read::TokenAt(..) | Read::NoToken) => {}
            Probe::Settled(Read::Unsettled) => return None,
            Probe::Lex => {
                deferred.push(out.len());
                out.push(crate::engine::Span { start: s as u32, end: s as u32 });
            }
        }
    }
    if !deferred.is_empty() {
        let mut dropped = vec![false; out.len()];
        for &i in &deferred {
            let s = out[i].start as usize;
            dropped[i] = match reader.lex_starting_at(s) {
                Read::TokenEndingAt(_, e) => {
                    out[i].end = e as u32;
                    !whole(&input[s..e])
                }
                Read::TokenStartingAt(..) | Read::TokenAt(..) | Read::NoToken => true,
                Read::Unsettled => return None,
            };
        }
        drop_marked(&mut out, &dropped);
    }
    Some(out)
}

/// The punctuation byte of `pattern` when it is a word token then a one-byte
/// literal of plain punctuation, `\W "="`, so the matches can be found from
/// every occurrence of that byte.
#[must_use]
pub fn byte_routable_word_then_punct(pattern: &crate::ast::Pattern) -> Option<u8> {
    use crate::ast::{Atom, Pattern};
    use crate::orbit::OrbitGroup;
    let Pattern::Concat(v) = pattern else {
        return None;
    };
    let [Pattern::Atom(Atom::Kind(crate::token::TokenKind::Word)), Pattern::Atom(Atom::Literal(lit, OrbitGroup::Identity))] =
        v.as_slice()
    else {
        return None;
    };
    let &[c] = lit.as_bytes() else {
        return None;
    };
    plain_punct(c).then_some(c)
}

/// Whether a word token followed by `punct` as a token of its own occurs at
/// all, or `None` when the bytes cannot settle the first one they reach.
///
/// [`byte_route_word_then_punct`] walks every occurrence of the punctuation
/// byte because it must report them all. This stops at the first that is a
/// match, and lexes a deferred occurrence as soon as it is found rather than
/// batching it until the end of the pass, which this may never reach.
#[must_use]
pub fn byte_route_any_word_then_punct(punct: u8, input: &[u8]) -> Option<bool> {
    Some(byte_route_first_word_then_punct(punct, input)?.is_some())
}

/// Where a word token followed by `punct` first matches, read to that match and
/// no further, or `None` where the bytes cannot settle it.
///
/// The span form of [`byte_route_any_word_then_punct`]. The match runs from the
/// word's start to past the punctuation, so this keeps the word's start where
/// the verdict form has no use for it.
#[must_use]
pub fn byte_route_first_word_then_punct(
    punct: u8,
    input: &[u8],
) -> Option<Option<crate::engine::Span>> {
    byte_route_first_word_then_punct_reading(punct, &mut ByteReader::new(input), 0)
}

/// [`byte_route_first_word_then_punct`] beginning the search at byte `from`,
/// over a reader the caller holds, under the ascending-ask constraint
/// [`byte_route_first_number_reading`] states.
///
/// A match runs from the word's start to past the punctuation, and the search
/// is for the punctuation, so the byte searched for is always past `from` when
/// the match begins at or after it. The word behind it may begin before `from`,
/// which is why the span is filtered rather than the search bounded.
#[must_use]
pub(crate) fn byte_route_first_word_then_punct_reading(
    punct: u8,
    reader: &mut ByteReader<'_>,
    from: usize,
) -> Option<Option<crate::engine::Span>> {
    use crate::token::TokenKind;
    let input = reader.input;
    if reader.unsettled() {
        return None;
    }
    let anchor = from.min(input.len());
    let mut from = anchor;
    while let Some(rel) = crate::byte_simd::find(&input[from..], &[punct]) {
        let q = from + rel;
        from = q + 1;
        let punct_lex = match reader.probe_punct_at(q) {
            Probe::Settled(Read::TokenAt(..)) => false,
            Probe::Settled(
                Read::TokenStartingAt(..) | Read::TokenEndingAt(..) | Read::NoToken,
            ) => continue,
            Probe::Settled(Read::Unsettled) => return None,
            Probe::Lex => true,
        };
        let Some(p) = input[..q].iter().rposition(|c| !c.is_ascii_whitespace()) else {
            continue;
        };
        if input[p] >= 0x80 {
            return None;
        }
        if !word_byte(input[p]) {
            continue;
        }
        let e = p + 1;
        // The word's start, which the span needs and the verdict does not.
        let mut word_start = match reader.probe_ending_at(e) {
            Probe::Settled(Read::TokenStartingAt(TokenKind::Word, ws)) => Some(ws),
            Probe::Settled(
                Read::TokenStartingAt(..)
                | Read::TokenEndingAt(..)
                | Read::TokenAt(..)
                | Read::NoToken,
            ) => continue,
            Probe::Settled(Read::Unsettled) => return None,
            Probe::Lex => None,
        };
        if punct_lex {
            match reader.lex_punct_at(q) {
                Read::TokenAt(..) => {}
                Read::TokenStartingAt(..) | Read::TokenEndingAt(..) | Read::NoToken => continue,
                Read::Unsettled => return None,
            }
        }
        if word_start.is_none() {
            match reader.lex_ending_at(e) {
                Read::TokenStartingAt(TokenKind::Word, ws) => word_start = Some(ws),
                Read::TokenStartingAt(..)
                | Read::TokenEndingAt(..)
                | Read::TokenAt(..)
                | Read::NoToken => continue,
                Read::Unsettled => return None,
            }
        }
        let Some(ws) = word_start else { continue };
        // A match whose word begins before the offset is outside what an
        // anchored caller asked for, and a later occurrence of the punctuation
        // may still give one inside it.
        if ws < anchor {
            continue;
        }
        return Some(Some(crate::engine::Span { start: ws as u32, end: (q + 1) as u32 }));
    }
    Some(None)
}

/// The matches of a word token followed by `punct` as a token of its own,
/// decided as [`ByteReader`] decides, or `None` when the bytes cannot settle
/// one of them.
///
/// The token before the punctuation is the one ending at the last
/// non-whitespace byte before it, since the engine skips whitespace tokens.
/// Only ASCII bytes are read as whitespace, and whether a byte beyond ASCII
/// belongs to a word depends on a UTF-8 decode this route does not do, so a
/// byte beyond ASCII there is unsettled.
#[must_use]
pub fn byte_route_word_then_punct(punct: u8, input: &[u8]) -> Option<Vec<crate::engine::Span>> {
    use crate::token::TokenKind;
    /// A match at this index of the output whose punctuation run, word run or
    /// both are left for the lexer to read after the pass.
    struct Deferred {
        at: usize,
        q: usize,
        e: usize,
        punct: bool,
        word: bool,
    }
    let mut reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    let mut out: Vec<crate::engine::Span> = Vec::new();
    let mut deferred: Vec<Deferred> = Vec::new();
    // Each occurrence is found when this asks for the next one, for the reason
    // given at `route_word_literal`.
    for q in crate::byte_simd::occurrences_unsplit(input, &[punct]) {
        let punct_lex = match reader.probe_punct_at(q) {
            Probe::Settled(Read::TokenAt(..)) => false,
            Probe::Settled(
                Read::TokenStartingAt(..) | Read::TokenEndingAt(..) | Read::NoToken,
            ) => continue,
            Probe::Settled(Read::Unsettled) => return None,
            Probe::Lex => true,
        };
        let Some(p) = input[..q].iter().rposition(|c| !c.is_ascii_whitespace()) else {
            continue;
        };
        if input[p] >= 0x80 {
            return None;
        }
        if !word_byte(input[p]) {
            continue;
        }
        let e = p + 1;
        let (start, word_lex) = match reader.probe_ending_at(e) {
            Probe::Settled(Read::TokenStartingAt(TokenKind::Word, start)) => (start, false),
            Probe::Settled(
                Read::TokenStartingAt(..)
                | Read::TokenEndingAt(..)
                | Read::TokenAt(..)
                | Read::NoToken,
            ) => continue,
            Probe::Settled(Read::Unsettled) => return None,
            Probe::Lex => (0, true),
        };
        if punct_lex || word_lex {
            deferred.push(Deferred { at: out.len(), q, e, punct: punct_lex, word: word_lex });
        }
        out.push(crate::engine::Span { start: start as u32, end: (q + 1) as u32 });
    }
    if !deferred.is_empty() {
        let mut dropped = vec![false; out.len()];
        for d in &deferred {
            if d.punct {
                match reader.lex_punct_at(d.q) {
                    Read::TokenAt(..) => {}
                    Read::TokenStartingAt(..) | Read::TokenEndingAt(..) | Read::NoToken => {
                        dropped[d.at] = true;
                        continue;
                    }
                    Read::Unsettled => return None,
                }
            }
            if d.word {
                match reader.lex_ending_at(d.e) {
                    Read::TokenStartingAt(TokenKind::Word, start) => {
                        out[d.at].start = start as u32;
                    }
                    Read::TokenStartingAt(..)
                    | Read::TokenEndingAt(..)
                    | Read::TokenAt(..)
                    | Read::NoToken => dropped[d.at] = true,
                    Read::Unsettled => return None,
                }
            }
        }
        drop_marked(&mut out, &dropped);
    }
    Some(out)
}

/// The literal and the punctuation byte of `pattern` when it is a plain
/// literal, a word token, then a one-byte literal of plain punctuation -
/// `"let" \W "="` - and the literal is itself routable as a whole word token.
#[must_use]
pub fn byte_routable_literal_word_punct(pattern: &crate::ast::Pattern) -> Option<(&str, u8)> {
    use crate::ast::{Atom, Pattern};
    use crate::orbit::OrbitGroup;
    let Pattern::Concat(v) = pattern else {
        return None;
    };
    let [head, Pattern::Atom(Atom::Kind(crate::token::TokenKind::Word)), Pattern::Atom(Atom::Literal(p, OrbitGroup::Identity))] =
        v.as_slice()
    else {
        return None;
    };
    let &[c] = p.as_bytes() else {
        return None;
    };
    if !plain_punct(c) {
        return None;
    }
    let lit = byte_routable_literal(head)?;
    Some((lit, c))
}

/// The matches of a whole-token `lit`, then a word token, then `punct` as a
/// token of its own, leftmost and non-overlapping, decided as [`ByteReader`]
/// decides - or `None` when the bytes cannot settle one of them.
///
/// Cut across the cores from [`CUT_ROUTE_THRESHOLD`], in one pass below it.
///
/// A match may span lines: the whitespace between its tokens can hold a
/// newline, since the engine skips whitespace. So a chunk owns the
/// anchors that start inside it and reads past its end for the word and the
/// punctuation, which is what [`cut_across_cores_reading_past`] provides. No
/// anchor is owned by two chunks, and a match crossing a cut is counted once:
/// the next chunk's match from that anchor would need a word where this one
/// found the punctuation.
#[must_use]
pub fn byte_route_literal_word_punct(
    lit: &str,
    punct: u8,
    input: &[u8],
) -> Option<Vec<crate::engine::Span>> {
    if input.len() >= CUT_ROUTE_THRESHOLD {
        crate::trace::rung("byte route", "literal, word, punctuation, cut across the cores", input.len());
        return cut_across_cores_reading_past(input, |tail, owns| {
            byte_route_literal_word_punct_owning(lit, punct, tail, owns)
        });
    }
    crate::trace::rung("byte route", "literal, word, punctuation, one pass", input.len());
    byte_route_literal_word_punct_one_pass(lit, punct, input)
}

/// [`byte_route_literal_word_punct`] in one pass whatever the size.
#[must_use]
pub fn byte_route_literal_word_punct_one_pass(
    lit: &str,
    punct: u8,
    input: &[u8],
) -> Option<Vec<crate::engine::Span>> {
    byte_route_literal_word_punct_owning(lit, punct, input, input.len())
}

/// The matches whose literal starts below `owns`, read from all of `input`.
///
/// Each anchor is decided as it is found, and its word and punctuation are
/// probed before the next anchor is sought, because the reader answers
/// ascending positions only. The word's kind is what the bytes settle by
/// shape, which is what `\W` asks, exactly as [`byte_route_word_then_punct`]
/// reads its word; a run the bytes cannot settle is lexed in place.
///
/// A match's word may itself be the literal, so two candidate matches can
/// share a token; an anchor inside the last match taken is passed over, which
/// is the leftmost non-overlapping selection the engine makes.
///
/// An anchor at or past `owns` belongs to another chunk and ends the walk. The
/// word and the punctuation of an owned anchor may be past `owns`, and are
/// read there. Public so the A/B can cut the route the way the shipped form
/// cuts it and hold the two equal.
#[must_use]
pub fn byte_route_literal_word_punct_owning(
    lit: &str,
    punct: u8,
    input: &[u8],
    owns: usize,
) -> Option<Vec<crate::engine::Span>> {
    use crate::token::TokenKind;
    let mut reader = ByteReader::new(input);
    if reader.unsettled() {
        return None;
    }
    let mut out: Vec<crate::engine::Span> = Vec::new();
    let mut from = 0usize;
    while from < owns {
        let Some(anchor) = byte_route_first_word_literal_reading(&[lit], &mut reader, from)? else {
            break;
        };
        let (s, e) = (anchor.start as usize, anchor.end as usize);
        if s >= owns {
            break;
        }
        // Whatever this anchor turns out to be, the next is sought past it.
        from = e;
        // The word: the first significant byte after the literal must open a
        // word token. Whether a byte beyond ASCII there opens a word depends on
        // a UTF-8 decode this route does not do, so it is unsettled.
        let ws = e + input[e..].iter().take_while(|c| c.is_ascii_whitespace()).count();
        if ws >= input.len() {
            break;
        }
        if input[ws] >= 0x80 {
            return None;
        }
        if !word_byte(input[ws]) {
            continue;
        }
        let we = match reader.probe_starting_at(ws) {
            Probe::Settled(Read::TokenEndingAt(TokenKind::Word, we)) => we,
            Probe::Settled(
                Read::TokenEndingAt(..) | Read::TokenStartingAt(..) | Read::TokenAt(..) | Read::NoToken,
            ) => continue,
            Probe::Settled(Read::Unsettled) => return None,
            Probe::Lex => match reader.lex_starting_at(ws) {
                Read::TokenEndingAt(TokenKind::Word, we) => we,
                Read::TokenEndingAt(..) | Read::TokenStartingAt(..) | Read::TokenAt(..) | Read::NoToken => {
                    continue;
                }
                Read::Unsettled => return None,
            },
        };
        // The punctuation: the first significant byte after the word is
        // `punct`, as a token of its own.
        let q = we + input[we..].iter().take_while(|c| c.is_ascii_whitespace()).count();
        if q >= input.len() || input[q] != punct {
            continue;
        }
        match reader.probe_punct_at(q) {
            Probe::Settled(Read::TokenAt(..)) => {}
            Probe::Settled(Read::TokenStartingAt(..) | Read::TokenEndingAt(..) | Read::NoToken) => {
                continue;
            }
            Probe::Settled(Read::Unsettled) => return None,
            Probe::Lex => match reader.lex_punct_at(q) {
                Read::TokenAt(..) => {}
                Read::TokenStartingAt(..) | Read::TokenEndingAt(..) | Read::NoToken => continue,
                Read::Unsettled => return None,
            },
        }
        out.push(crate::engine::Span { start: s as u32, end: (q + 1) as u32 });
        from = q + 1;
    }
    Some(out)
}

#[cfg(test)]
mod literal_word_punct_tests {
    use super::*;

    /// The comparison's four statement shapes, at a size that crosses the cut
    /// threshold so both forms of the route are exercised.
    fn corpus(statements: usize) -> Vec<u8> {
        let mut s = String::new();
        for i in 0..statements {
            match i % 4 {
                0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
                1 => s.push_str(&format!("call_{i}(alpha, beta, {i}) ;\n")),
                2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
                _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
            }
        }
        s.into_bytes()
    }

    /// Where the route answers, it answers what the windows and the engine
    /// answer; where it declines, the engine still answers and the caller
    /// falls through to it. The cut form equals the one-pass form always.
    fn holds_on(input: &[u8], src: &str) {
        let p = crate::parser::parse(src).expect("parses");
        let (lit, punct) = byte_routable_literal_word_punct(&p).expect("the shape is routable");
        let one = byte_route_literal_word_punct_one_pass(lit, punct, input);
        let cut = cut_across_cores_reading_past(input, |tail, owns| {
            byte_route_literal_word_punct_owning(lit, punct, tail, owns)
        });
        assert_eq!(cut, one, "{src}: the cut form disagrees with one pass");
        let chosen = byte_route_literal_word_punct(lit, punct, input);
        assert_eq!(chosen, one, "{src}: the shipped route disagrees with one pass");
        if let Some(spans) = one {
            let windows = scan_by_literal_windows(&p, input).expect("the windows take this shape");
            assert_eq!(spans, windows, "{src}: the byte route disagrees with the windows");
            let engine = crate::nfa::scan_nfa(&p, input).expect("the single-pass engine takes this shape");
            assert_eq!(spans, engine, "{src}: the byte route disagrees with the engine");
        }
    }

    #[test]
    fn matches_the_windows_and_the_engine_on_the_comparison_corpus() {
        // 8000 statements is 263 KB, past the 160 KB cut threshold.
        holds_on(&corpus(8000), "\"let\" \\W \"=\"");
        holds_on(&corpus(8000), "\"alpha\" \\W \")\"");
        holds_on(&corpus(2000), "\"let\" \\W \"=\"");
    }

    #[test]
    fn the_word_may_be_the_literal_and_the_selection_is_leftmost_non_overlapping() {
        // `let let = x`: one match, `let let =`, and the second `let` is
        // inside it, not a second anchor.
        let input = b"let let = x ;\nlet a = 1 ;\nlet let let = 2 ;\n";
        holds_on(input, "\"let\" \\W \"=\"");
        let p = crate::parser::parse("\"let\" \\W \"=\"").expect("parses");
        let (lit, punct) = byte_routable_literal_word_punct(&p).expect("routable");
        let spans = byte_route_literal_word_punct_one_pass(lit, punct, input).expect("the bytes settle it");
        let text: Vec<&str> = spans.iter().map(|s| std::str::from_utf8(&input[s.start as usize..s.end as usize]).unwrap()).collect();
        assert_eq!(text, ["let let =", "let a =", "let let ="]);
    }

    #[test]
    fn a_quote_the_bytes_cannot_settle_refuses_the_whole_in_both_forms() {
        // A single quote after `://` in one run is the quote `quotes_through`
        // cannot settle. Both forms must decline. The cut form declines
        // because a single chunk did, and that chunk straddles none of the
        // others, so this is the path where one chunk's refusal has to reach
        // the whole answer on its own.
        // A char literal `'"'` after `://` in one run: the scan reads the
        // char literal whole and asks whether it opens, and `quotes_through`
        // cannot say, since a URL runs over a single quote and up to a double
        // one. The scan ignores a lone single quote that never closes on its
        // line and never asks about it, which is why a bare `'` after `://`
        // does not reach the rule.
        let mut s = String::new();
        for i in 0..6000 {
            s.push_str(&format!("let v_{i} = {i} ; see http://x/'\"' here\n"));
        }
        let input = s.into_bytes();
        assert!(input.len() >= CUT_ROUTE_THRESHOLD, "the refusing input must reach the cut form");
        let p = crate::parser::parse("\"let\" \\W \"=\"").expect("parses");
        let (lit, punct) = byte_routable_literal_word_punct(&p).expect("routable");
        assert_eq!(byte_route_literal_word_punct_one_pass(lit, punct, &input), None);
        assert_eq!(byte_route_literal_word_punct(lit, punct, &input), None);
        assert_eq!(byte_route_word_literals(&[lit], &input), None);
        // The engine still answers; the caller falls through to it.
        assert!(crate::nfa::scan_nfa(&p, &input).is_some());
    }

    #[test]
    fn a_match_spanning_a_line_is_not_split_by_a_cut() {
        // The literal at a line's end, its word and punctuation on the next:
        // the engine skips whitespace, so this is one match, and the chunk
        // owning the literal must read past its line to finish it.
        let mut s = String::new();
        for i in 0..3000 {
            s.push_str(&format!("let\nvalue_{i} = {i} ;\nlet\n  x_{i}\n  = 2 ;\ncall_{i}(a) ;\n"));
        }
        let input = s.into_bytes();
        assert!(input.len() >= CUT_ROUTE_THRESHOLD);
        holds_on(&input, "\"let\" \\W \"=\"");
        let p = crate::parser::parse("\"let\" \\W \"=\"").expect("parses");
        let (lit, punct) = byte_routable_literal_word_punct(&p).expect("routable");
        let spans = byte_route_literal_word_punct(lit, punct, &input).expect("settled");
        assert_eq!(spans.len(), 6000, "two spanning matches a block, none split");
    }

    #[test]
    fn a_line_anchored_literal_cut_at_lines_rejects_a_second_literal_on_the_line() {
        // `let let = 1`: the first leads its line, the second does not. A cut
        // at a whitespace-run end would put the second at a chunk's start and
        // read it as leading; the cut keeps line boundaries only.
        let mut s = String::new();
        for i in 0..6000 {
            s.push_str(&format!("let let = {i} ;\n  let a_{i} = 1 ; let b = 2 ;\n"));
        }
        let input = s.into_bytes();
        assert!(input.len() >= CUT_ROUTE_THRESHOLD);
        let one = byte_route_line_anchored_literal_one_pass("let", &input).expect("settled");
        let cut = byte_route_line_anchored_literal("let", &input).expect("settled");
        assert_eq!(cut, one, "the cut line-anchored route disagrees with one pass");
        assert_eq!(one.len(), 12000, "one leading `let` a line, two lines a block");
        let p = crate::parser::parse("^ \"let\"").expect("parses");
        assert_eq!(one, crate::nfa::scan_nfa(&p, &input).expect("the engine takes it"));
    }

    #[test]
    fn strings_across_every_candidate_boundary_do_not_move_a_cut() {
        let mut s = String::new();
        for i in 0..3000 {
            s.push_str(&format!("let a_{i} = \"open {i}\nlet b = 2 ; still in the string\n\" ; let c_{i} = 3 ;\n"));
        }
        holds_on(&s.into_bytes(), "\"let\" \\W \"=\"");
    }
}

/// Bytes either side of a literal hit the first window tries, per token the
/// longest match can span. A token averages two to six bytes across the
/// corpora `benches/lex_scaling` reads, so this is generous by an order of
/// magnitude and the widening below is the guard rather than the rule.
const WINDOW_BYTES_PER_TOKEN: usize = 256;

/// How many times a window may widen before the scan gives up on windowing
/// and lexes the whole input. Each try doubles.
const WINDOW_WIDENINGS: usize = 3;

/// What one byte of the whole-input lex costs, in per mille of what one byte
/// of a window's own lex costs. A window is lexed serially where the scan it
/// replaces lexes across cores, so a byte inside a window usually costs more
/// than a byte of the whole input, and windows covering this share of the
/// input cost what the whole lex costs.
///
/// `benches/vs_regex`'s lex-ratio sweep measures it per corpus shape, at about
/// sixteen megabytes each, and the shapes do not agree:
///
/// ```text
///   plain statements          413      two-hundred-byte tokens   491
///   typed statements          389      one run, no whitespace   1050
///   typed, a long run last    378
/// ```
///
/// The lowest is taken, which keeps the route furthest from the crossover.
/// The one-run shape has no whitespace where the chunked lexer can split, so
/// it lexes serially in both arms and the dispatch is wasted: a window's
/// bytes cost the same there as the whole input's, and the gate is slack
/// rather than tight. A corpus whose tokens are long is what moves this.
const WINDOW_LEX_SHARE: usize = 378;

/// The share of the input, in per mille, that the windows may cover, both as
/// the literal's hits open them and as they settle onto the lexer's splits.
/// Past [`WINDOW_LEX_SHARE`] their own lex costs more than the lex they
/// replace whatever the window count, and the margin holds the route back
/// from the crossover as it does for the count.
///
/// On the windows as they open, nothing in `benches/vs_regex` reaches it:
/// window width grows with the pattern's token bound, and the count gate
/// binds first for every pattern spanning fewer than about thirty tokens. On
/// the settled spans it binds where splits are sparse, since a split wants a
/// newline before a significant byte and indented text has few.
const WINDOW_COVERAGE_LIMIT: usize = WINDOW_LEX_SHARE / WINDOW_MARGIN;

/// Input bytes the whole-input lex and match read in the time one window
/// costs to settle, lex and scan.
///
/// A window is a fixed cost repeated, and on the corpora `benches/vs_regex`
/// windows it is the cost that decides, not the bytes inside the windows: at
/// 19.6 MB the route takes 0.7 ms of whole-input work plus 25 to 33 us per
/// window, where a thousand windows hold 1.5 MB that a lex reads in 4 ms.
/// Against 32 ms for the whole input the two meet between 1,000 and 2,000
/// windows, which is 16 kB of input per window.
const WINDOW_COST_IN_INPUT_BYTES: usize = 16_000;

/// How far ahead the route must be before it is taken, rather than being
/// taken wherever it ties.
///
/// The cost above is one corpus's, and a corpus whose windows hold longer
/// tokens or a blob costs more per window. The two sides of the crossover are
/// not alike either: past it the route runs 1.6x the scan it replaces at
/// 2,000 windows and 5.7x at 3,500, where below it the gain is 0.17x at a
/// hundred windows and 0.02x at one. So the route gives up the narrow wins
/// either side of the crossover to keep away from the losses.
const WINDOW_MARGIN: usize = 2;

/// The leftmost, non-overlapping matches of `pattern` found by lexing only
/// the regions where a required literal can put a match, or `None` where that
/// cannot be done or would not be cheaper.
///
/// Every match contains every literal [`collect_required_literals`]
/// collects, and a match spans at most [`crate::nfa::bounded_max_len`] tokens.
/// So a match containing an occurrence of such a literal begins at most that
/// many tokens before it and ends at most that many after, and no match exists
/// anywhere else. Finding the occurrences costs one SIMD pass; lexing the
/// windows around them costs the lex of those regions rather than of the
/// input, which is the whole of the saving - the lex runs at 217 to 513 MB/s
/// depending on the input, and a scan that can only match in a few places
/// otherwise runs all of it.
///
/// Each window is taken twice as wide as a match can be and settled onto the
/// splits the chunked lexer takes, with `2 * max_len` significant tokens
/// beyond it on either side. Windows whose settled spans overlap or touch are
/// gathered into one span, so no byte is lexed twice, no match is reported
/// twice, and no match crosses from one span into the next. A match ending
/// within a match's length of a span's end may have been cut short there, so
/// it is refused.
///
/// `None`, and the caller runs the whole scan, where: the pattern requires no
/// literal, so there is nothing to anchor on; its longest match is unbounded,
/// so no window can hold one; it carries `\G`, whose matches must abut, or
/// `\K`, which moves a match's start off its anchor; it carries `\A` or `\z`,
/// which read whether any token precedes or follows in the whole stream and
/// which a window cannot answer; a window will not widen far enough to hold a
/// match; or the windows, as the literal opens them or as they settle, cover
/// so much of the input that lexing them is not cheaper than scanning it.
#[must_use]
pub fn scan_required_windows(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Option<Vec<crate::engine::Span>> {
    required_window_spans(pattern, input, false, true)
}

/// How the window route decides whether its windows are worth taking.
///
/// A window's cost is dominated by the tokens inside it, and token density
/// varies by about a hundredfold between corpus shapes: a 1,536-byte window
/// holds some three hundred tokens of statements and some seven of
/// two-hundred-byte text. A gate reading only bytes cannot see that, which is
/// what these exist to measure against each other.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowGate {
    /// Windows counted against the input's length in bytes, and the share of
    /// the input they cover held under [`WINDOW_COVERAGE_LIMIT`].
    ByteCount,
    /// The coverage share alone, with no bound on the window count.
    CoverageOnly,
    /// The first window settled and lexed, the rest priced from the tokens it
    /// held: the density is measured on the input rather than assumed.
    PricedWindow,
    /// Tokens per byte predicted from how many bytes end a token - whitespace
    /// and punctuation - before anything is lexed.
    PredictedDensity,
    /// No gate at all, for pricing the route where a gate refuses it.
    Open,
}

/// Nanoseconds a significant token costs inside a window, against what the
/// same token costs in the whole-input lex the route replaces.
///
/// The route lexes a window serially where the scan it replaces lexes across
/// cores, so a token inside a window is dearer than a token outside one, and
/// this is that ratio in per mille. It is [`WINDOW_LEX_SHARE`] restated per
/// token rather than per byte, which is the quantity that holds across corpus
/// shapes when the byte one does not.
const WINDOW_TOKEN_SHARE: usize = WINDOW_LEX_SHARE;

/// Bytes of the input a prefix probe lexes before giving up on it.
///
/// A pattern that matches at all usually matches early, and the probe costs
/// the lex of this much where the scan it replaces costs the lex of
/// everything. Big enough to hold a match of any bounded pattern many times
/// over, small enough that reading it and then scanning anyway is a rounding
/// error against a multi-megabyte lex.
const PREFIX_PROBE_BYTES: usize = 64 * 1024;

/// Whether `pattern` matches inside a prefix of `input`, or `None` where the
/// prefix does not settle it.
///
/// `Some(true)` is a match of the whole input: the prefix ends where the
/// chunked lexer would cut, so every token in it is the token the whole-input
/// lex makes, and the engine reads the input's own bytes for anchors and
/// guards. `None` proves nothing and the caller runs the whole scan - a match
/// may straddle the cut or be past it.
///
/// This is the existence short circuit for patterns no byte route answers: a
/// bare token kind has no literal to search for, so the only way to know
/// whether one occurs is to lex until one does.
#[must_use]
pub fn any_in_prefix(pattern: &crate::ast::Pattern, input: &[u8]) -> Option<bool> {
    if input.len() <= PREFIX_PROBE_BYTES {
        return None;
    }
    // An anchor that reads whether any token follows in the whole stream
    // cannot be answered from a prefix, and `\G` and `\K` are sequential.
    if pattern.starts_with_resume()
        || pattern.mentions_reset_start()
        || pattern.mentions_stream_end_anchor()
    {
        return None;
    }
    let compiled = crate::nfa::compile_pattern(pattern)?;
    let quotes = crate::parallel_lex::quoted_spans(input);
    let cut = boundary_at_or_after(input, &quotes, PREFIX_PROBE_BYTES);
    if cut == 0 || cut >= input.len() {
        return None;
    }
    let toks = lex_window(input, 0, cut);
    let spans = crate::nfa::scan_nfa_over_compiled(&compiled, pattern, input, &toks);
    // A match reaching the cut may have been cut short, so only one ending
    // clear of it is a match of the whole input.
    // The match's own token length plus how far past it a bounded assertion
    // can read. Reserving only the first would let a cut truncate the tokens
    // an assertion reads.
    let max_len = crate::nfa::bounded_max_len(pattern).unwrap_or(1).max(1)
        + pattern.widest_forward_window();
    let edge = toks.iter().filter(|t| t.is_significant()).nth_back(max_len - 1)?.start();
    spans.iter().any(|s| s.end() <= edge).then_some(true)
}

/// How many prefixes are tried before the whole input is taken.
///
/// One. Measured over four patterns no route answers, five positions for the
/// first match and two sizes, against a whole-input lex using the same
/// parallel lexer: one probe then the whole input measures 0.60x to 0.71x
/// where the match is late or absent, and 0.02x to 0.08x where it is early. No
/// cell was worse than the whole lex, so there is no crossover to trade
/// against and no growth factor left to choose.
///
/// Widening further wins in the middle of a corpus - 0.20x where one probe
/// measures 0.61x - and loses at the end: a back-reference over an input
/// holding no match measures 4.06x. One probe does not have that tail.
const PREFIX_TRIES: usize = 1;

/// The first match of `pattern` in `input`, found by lexing a prefix and
/// widening it until the answer is settled, or `None` where no prefix can
/// settle it.
///
/// [`any_in_prefix`] asks whether a match exists and stops at the first
/// prefix that says yes. This asks where the first one is, which needs the
/// same prefix to be able to say no as well - so the widening continues to
/// the whole input, and the answer is then definitive either way.
///
/// `Some(None)` is a proof of absence over the whole input, not over a
/// prefix. `None` is the refusal: an anchor that reads the whole stream, or a
/// pattern the single-pass engine does not take.
///
/// The saving is the lex and not the walk. A pattern matching in the first
/// kilobyte lexes a sixty-fourth of a megabyte where a whole-input scan lexes
/// every byte, and the lex is the pattern-independent cost that dominates.
/// A pattern that matches only at the end, or not at all, lexes everything
/// and has saved nothing - which is the trade this makes deliberately, since
/// a first match is usually not at the end.
#[must_use]
pub fn find_by_growing_prefix(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Option<Option<crate::engine::Span>> {
    settle_first_from_a_prefix(pattern, input, |s, _| s)
}

/// [`find_by_growing_prefix`] widening by `growth` each round and taking the
/// whole input after `cap` prefixes, for the sweep that sets the policy.
#[doc(hidden)]
#[must_use]
pub fn find_by_growing_prefix_at(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    growth: usize,
    cap: usize,
) -> Option<Option<crate::engine::Span>> {
    find_by_growing_prefix_lexed(pattern, input, growth, cap, false)
}

/// [`find_by_growing_prefix_at`] lexing each prefix with the multi-core lexer
/// when `parallel` is set, for the arm that says whether the prefix path's
/// cost is the widening or its lexer.
#[doc(hidden)]
#[must_use]
pub fn find_by_growing_prefix_lexed(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    growth: usize,
    cap: usize,
    parallel: bool,
) -> Option<Option<crate::engine::Span>> {
    if !settles_from_a_prefix(pattern) {
        return None;
    }
    let compiled = crate::nfa::compile_pattern(pattern)?;
    // The match's own token length plus how far past it a bounded assertion
    // can read. Reserving only the first would let a cut truncate the tokens
    // an assertion reads.
    let max_len = crate::nfa::bounded_max_len(pattern).unwrap_or(1).max(1)
        + pattern.widest_forward_window();
    let mut answer = None;
    over_widening_prefixes_lexed(input, growth, cap, parallel, |toks, whole| {
        let spans = crate::nfa::scan_nfa_over_compiled(&compiled, pattern, input, toks);
        let hit = if whole {
            spans.first().copied()
        } else {
            settled_clear_of_the_cut(toks, &spans, max_len)
        };
        match hit {
            Some(s) => {
                answer = Some(Some(s));
                false
            }
            None => {
                if whole {
                    answer = Some(None);
                }
                true
            }
        }
    });
    answer
}

/// [`find_by_growing_prefix`], handing `take` the prefix's tokens beside the
/// match so a caller can read more out of it than the span.
///
/// The tokens are the prefix's, not the whole input's, and the match was found
/// over them - so anything derived from them is derived from the same reading
/// that found it. A caller wanting the registers a match
/// bound resolves them here rather than lexing the input again to ask.
///
/// `None` is the refusal: an anchor no prefix can settle, or a pattern the
/// single-pass engine does not take. `Some(None)` is a proof of absence over
/// the whole input.
pub fn settle_first_from_a_prefix<T>(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    mut take: impl FnMut(crate::engine::Span, &[crate::token::Token]) -> T,
) -> Option<Option<T>> {
    if !settles_from_a_prefix(pattern) {
        return None;
    }
    let compiled = crate::nfa::compile_pattern(pattern)?;
    // The match's own token length plus how far past it a bounded assertion
    // can read. Reserving only the first would let a cut truncate the tokens
    // an assertion reads.
    let max_len = crate::nfa::bounded_max_len(pattern).unwrap_or(1).max(1)
        + pattern.widest_forward_window();
    let mut answer = None;
    over_widening_prefixes(input, |toks, whole| {
        let spans = crate::nfa::scan_nfa_over_compiled(&compiled, pattern, input, toks);
        // On the last round the prefix is the whole input, so any match is a
        // match and none means none. Before that only a match clear of the
        // cut is one the cut cannot have made.
        let hit = if whole {
            spans.first().copied()
        } else {
            settled_clear_of_the_cut(toks, &spans, max_len)
        };
        match hit {
            Some(s) => {
                answer = Some(Some(take(s, toks)));
                false
            }
            None => {
                if whole {
                    answer = Some(None);
                }
                true
            }
        }
    });
    answer
}

/// The leftmost match of `pattern` at or after byte `at`, found by lexing a
/// prefix and widening it until the answer is settled, or `None` where no
/// prefix can settle it.
///
/// [`find_by_growing_prefix`] with the selection re-run from `at` rather than
/// filtered from zero, which is the question [`crate::find_at`] answers: the
/// leftmost, non-overlapping selection from an offset can hold a match that
/// overlaps one the selection from zero preferred, so a filter of that
/// selection is a different answer.
///
/// A prefix stopping before `at` has decided nothing, so the widening carries
/// on until one reaches past it. That is also where the saving is: a caller
/// walking an input asks from where it last stopped, which is early.
///
/// `Some(None)` is a proof of absence over the whole input. `None` is the
/// refusal, for the patterns [`settles_from_a_prefix`] rejects.
#[must_use]
pub fn find_at_by_growing_prefix(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    at: usize,
) -> Option<Option<crate::engine::Span>> {
    if !settles_from_a_prefix(pattern) {
        return None;
    }
    let compiled = crate::nfa::compile_pattern(pattern)?;
    let max_len = crate::nfa::bounded_max_len(pattern).unwrap_or(1).max(1)
        + pattern.widest_forward_window();
    let mut answer = None;
    over_widening_prefixes(input, |toks, whole| {
        if !whole && toks.last().is_none_or(|t| t.end() <= at) {
            return true;
        }
        let start = toks.partition_point(|t| t.start() < at);
        let spans =
            crate::nfa::scan_nfa_over_serial_from_compiled(&compiled, pattern, input, toks, start);
        let hit = if whole {
            spans.first().copied()
        } else {
            settled_clear_of_the_cut(toks, &spans, max_len)
        };
        match hit {
            Some(s) => {
                answer = Some(Some(s));
                false
            }
            None => {
                if whole {
                    answer = Some(None);
                }
                true
            }
        }
    });
    answer
}

/// How far the soonest-ending match of `pattern` in `input` reaches, found by
/// lexing a prefix and widening it until the answer is settled, or `None` where
/// no prefix can settle it.
///
/// The counterpart of [`find_by_growing_prefix`] for the earliest-accept
/// question, and it settles for the same reason: a match beginning at or after
/// the cut ends at or after the cut, so no wider prefix can find an earlier end
/// than one already found clear of the cut.
///
/// `Some(None)` is a proof of absence over the whole input, not over a prefix.
/// `None` is the refusal, for the patterns [`settles_from_a_prefix`] rejects.
#[must_use]
pub fn shortest_end_by_growing_prefix(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Option<Option<usize>> {
    shortest_end_by_growing_prefix_from(pattern, input, 0)
}

/// [`shortest_end_by_growing_prefix`] for a match beginning at or after byte
/// `at`, as [`find_at_by_growing_prefix`] is for the leftmost one.
///
/// A prefix stopping before `at` has decided nothing, so the widening carries
/// on until one reaches past it.
#[must_use]
pub fn shortest_end_by_growing_prefix_from(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    at: usize,
) -> Option<Option<usize>> {
    if !settles_from_a_prefix(pattern) {
        return None;
    }
    let compiled = crate::nfa::compile_pattern(pattern)?;
    // The match's own token length plus how far past it a bounded assertion can
    // read, as [`settle_first_from_a_prefix`] reserves it and for the same
    // reason.
    let max_len = crate::nfa::bounded_max_len(pattern).unwrap_or(1).max(1)
        + pattern.widest_forward_window();
    let mut answer = None;
    over_widening_prefixes(input, |toks, whole| {
        if !whole && toks.last().is_none_or(|t| t.end() <= at) {
            return true;
        }
        let start = toks.partition_point(|t| t.start() < at);
        let end = crate::nfa::shortest_end_over_compiled(&compiled, pattern, input, toks, start);
        // On the last round the prefix is the whole input, so any end is the
        // end and none means none. Before that only an end clear of the cut is
        // one the cut cannot have made.
        let hit = match end {
            Some(e) if whole || end_clear_of_the_cut(toks, e, max_len) => Some(e),
            _ => None,
        };
        match hit {
            Some(e) => {
                answer = Some(Some(e));
                false
            }
            None => {
                if whole {
                    answer = Some(None);
                }
                true
            }
        }
    });
    answer
}

/// Whether `end`, the byte a match reaches, is clear of the last `reserve`
/// significant tokens of `toks`, and so cannot have been cut short by the
/// prefix's edge.
///
/// The [`settled_clear_of_the_cut`] test for a caller holding one end rather
/// than a list of spans. `false` for a prefix holding fewer than `reserve`
/// significant tokens: one that short settles nothing.
#[must_use]
fn end_clear_of_the_cut(toks: &[crate::token::Token], end: usize, reserve: usize) -> bool {
    toks.iter()
        .filter(|t| t.is_significant())
        .nth_back(reserve.max(1) - 1)
        .is_some_and(|edge| end <= edge.start())
}

/// Whether a prefix of the input can settle `pattern` at all.
///
/// An anchor that reads whether any token follows in the whole stream cannot
/// be answered from a prefix, and `\G` and `\K` are sequential.
#[must_use]
pub fn settles_from_a_prefix(pattern: &crate::ast::Pattern) -> bool {
    !pattern.starts_with_resume()
        && !pattern.mentions_reset_start()
        && !pattern.mentions_stream_end_anchor()
        // A match whose outcome can turn on input outside its own span cannot
        // be settled by a prefix that truncated that input. A content guard
        // reads an unbounded forward window, an unbounded assertion reads as
        // far as its sub-pattern runs, a field anchor counts from the input's
        // start, and an axis field is computed over the whole stream. Each
        // would report no match from a prefix where the whole input has one.
        && !pattern.depends_on_whole_input()
}

/// The first of `spans` that ends clear of the last `reserve` significant
/// tokens of `toks`, and so cannot have been cut short by the prefix's edge.
///
/// `None` where there is no such match, which includes a prefix holding fewer
/// than `max_len` significant tokens: one that short settles nothing.
#[must_use]
pub fn settled_clear_of_the_cut(
    toks: &[crate::token::Token],
    spans: &[crate::engine::Span],
    reserve: usize,
) -> Option<crate::engine::Span> {
    let edge = toks.iter().filter(|t| t.is_significant()).nth_back(reserve.max(1) - 1)?;
    spans.iter().find(|s| s.end() <= edge.start()).copied()
}

/// Lex `input` in prefixes that widen until `round` is satisfied or the whole
/// input is reached.
///
/// `round` is handed every token lexed so far and whether the prefix is now
/// the whole input, and returns whether to widen again. On the call where the
/// prefix is the whole input there is nothing left to widen to, so the return
/// is ignored and the walk ends.
///
/// Each cut is a place the chunked lexer would split, so the tokens either
/// side of it are the tokens a whole-input lex makes and each round appends
/// to the last rather than redoing it. That is what keeps the lexing linear
/// in the bytes reached however many rounds it takes.
///
/// The widening is shared rather than written per caller because the callers
/// that want it - a first match, a set of patterns asked together - differ
/// only in what they do with each round's tokens, and a second copy of the
/// cut-and-append arithmetic would be a second chance to get a boundary
/// wrong.
pub fn over_widening_prefixes(
    input: &[u8],
    round: impl FnMut(&[crate::token::Token], bool) -> bool,
) {
    over_widening_prefixes_lexed(input, 2, PREFIX_TRIES, true, round);
}

/// [`over_widening_prefixes`] widening by `growth` each round and never
/// stopping at [`PREFIX_TRIES`] prefixes, so the factor can be swept and the
/// policy read off the curve instead of reasoned about.
///
/// A `growth` below two is raised to two: a prefix that does not widen never
/// reaches the end of the input, and a caller asking for that is asking for a
/// loop that does not terminate.
#[doc(hidden)]
pub fn over_widening_prefixes_by(
    input: &[u8],
    growth: usize,
    round: impl FnMut(&[crate::token::Token], bool) -> bool,
) {
    over_widening_prefixes_capped(input, growth, usize::MAX, round);
}

/// [`over_widening_prefixes_by`] taking the whole input once `cap` prefixes
/// have failed to settle it, rather than widening again.
///
/// Each round re-walks everything lexed so far, so the walk's cost is the sum
/// over rounds rather than one pass, and a cap trades the chance of settling
/// in a later prefix against incurring that sum. A cap of one is probe-then-lex:
/// one prefix, and the whole input if it says nothing.
#[doc(hidden)]
pub fn over_widening_prefixes_capped(
    input: &[u8],
    growth: usize,
    cap: usize,
    round: impl FnMut(&[crate::token::Token], bool) -> bool,
) {
    over_widening_prefixes_lexed(input, growth, cap, false, round);
}

/// [`over_widening_prefixes_capped`] lexing each appended range with the
/// multi-core lexer when `parallel` is set, rather than the serial one.
///
/// Which lexer runs is most of what separates a prefix from a whole-input
/// lex on a many-core box, so it is a parameter while that is being measured
/// rather than a property of the path.
///
/// A cut is a place the chunked lexer would split, so the range between two
/// of them lexes the same whichever lexer reads it: no blob and no quoted
/// string crosses a cut.
#[doc(hidden)]
pub fn over_widening_prefixes_lexed(
    input: &[u8],
    growth: usize,
    cap: usize,
    parallel: bool,
    mut round: impl FnMut(&[crate::token::Token], bool) -> bool,
) {
    let quotes = crate::parallel_lex::quoted_spans(input);
    let mut toks: Vec<crate::token::Token> = Vec::new();
    let mut lo = 0usize;
    let mut want = PREFIX_PROBE_BYTES;
    let mut done = 0usize;
    loop {
        // Past the cap the only prefix left to try is the whole input, so
        // reaching for it costs one more round rather than several.
        let capped = done >= cap;
        let cut = if capped || want >= input.len() {
            input.len()
        } else {
            boundary_at_or_after(input, &quotes, want)
        };
        // No cut between here and the end, so the prefix is the whole input
        // and this round is the last one there is.
        let whole = cut >= input.len() || cut <= lo;
        let cut = if whole { input.len() } else { cut };
        if parallel {
            let mut part = crate::parallel_lex::lex_parallel(&input[lo..cut]);
            for t in &mut part {
                t.start += lo as u32;
                t.end += lo as u32;
            }
            toks.extend(part);
        } else {
            toks.extend(lex_window(input, lo, cut));
        }
        lo = cut;
        done += 1;
        let widen = round(&toks, whole);
        if whole || !widen {
            return;
        }
        want = want.saturating_mul(growth.max(2));
    }
}

/// Which stage of the window route turned `pattern` over `input` away, or
/// `"taken"` where none did.
///
/// The route hands back `None` from six places and a caller cannot tell them
/// apart, so a corpus it refuses reads exactly like a corpus its gate refuses
/// - and a gate cannot be measured on a route that never reaches it. This says
/// which.
#[doc(hidden)]
#[must_use]
pub fn required_window_reason(pattern: &crate::ast::Pattern, input: &[u8]) -> String {
    let Some(max_len) = crate::nfa::bounded_max_len(pattern) else {
        return "unbounded".into();
    };
    if max_len == 0 {
        return "empty match".into();
    }
    if pattern.starts_with_resume() {
        return "resume anchor".into();
    }
    if pattern.mentions_reset_start() {
        return "reset start".into();
    }
    if pattern.mentions_stream_end_anchor() {
        return "stream anchor".into();
    }
    let mut lits: Vec<Vec<u8>> = Vec::new();
    collect_contained_literals(pattern, &mut lits);
    if lits.iter().all(Vec::is_empty) {
        return "no literal".into();
    }
    if crate::nfa::compile_pattern(pattern).is_none() {
        return "set engine".into();
    }
    let reach = max_len.saturating_mul(WINDOW_BYTES_PER_TOKEN);
    let Some(rough) = windows_under_limit(input, &mut lits, reach, WindowGate::ByteCount) else {
        return "gate".into();
    };
    if rough.is_empty() {
        return "taken, no hit".into();
    }
    let budget = settled_budget(input.len(), WindowGate::ByteCount);
    if gather_windows(input, None, &rough, budget).is_none() {
        return format!("settled past the budget of {budget} bytes before the strings are read");
    }
    let quotes = crate::parallel_lex::quoted_spans(input);
    let Some(gathered) = gather_windows(input, Some(&quotes), &rough, budget) else {
        return format!("settled past the budget of {budget} bytes");
    };
    match settle_gathered(input, &quotes, &gathered, max_len, budget, |_, _, _| false) {
        Ok(_) => "taken".into(),
        Err(Unsettled::Widening(report)) => report.to_string(),
        Err(Unsettled::Budget(bytes)) => format!("settling lexed {bytes} bytes, past the budget of {budget}"),
    }
}

/// What the widest window [`settle_window`] tried actually held, when it gave
/// up.
///
/// "The window will not settle" names a stage and not a cause. The settling
/// wants `enough` significant tokens each side of the span it was handed and
/// widens by `widest` bytes to get them, so the counts it reached, the window
/// where it reached them, and the span it was handed show whether a bound is
/// too high or a boundary is in the wrong place.
///
/// It is returned by the settling rather than re-derived beside it: a second
/// copy of the widening arithmetic can disagree with the first, and then the
/// diagnostic describes a window that was never tried.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SettleReport {
    span: (usize, usize),
    lo: usize,
    hi: usize,
    before: usize,
    after: usize,
    enough: usize,
    widest: usize,
}

impl std::fmt::Display for SettleReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "will not settle: {} before and {} after, wanted {}, in {}..{} widened {} around {}..{}",
            self.before,
            self.after,
            self.enough,
            self.lo,
            self.hi,
            self.widest,
            self.span.0,
            self.span.1
        )
    }
}

/// [`scan_required_windows`] under a named gate, so the gates can be measured
/// against each other rather than chosen between.
#[doc(hidden)]
#[must_use]
pub fn scan_required_windows_gated(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    gate: WindowGate,
) -> Option<Vec<crate::engine::Span>> {
    required_window_spans_gated(pattern, input, false, gate)
}

/// [`scan_required_windows`] with the gate open, so the route can be priced
/// at densities the gate refuses.
///
/// The gate is a cut point on a curve, and a caller that only ever sees the
/// accepted side of it cannot tell where the curve crosses or how far the cut
/// is from the crossing. `benches/vs_regex` reads both sides through this
/// and the gate's own constants follow from the curve.
#[doc(hidden)]
#[must_use]
pub fn scan_required_windows_ungated(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Option<Vec<crate::engine::Span>> {
    required_window_spans_gated(pattern, input, false, WindowGate::Open)
}

/// How many bytes of `input` end a token - ASCII whitespace, or punctuation
/// that is not a word byte - as a stand-in for how many tokens it holds.
///
/// Read from the first `sample` bytes and scaled, since the question is a
/// density rather than a count. A token ends at one of these or at the
/// input's end, so the count is within one of the token count on a run of
/// ordinary text and over-counts a run of punctuation.
fn token_enders_per_mille(input: &[u8], sample: usize) -> usize {
    let head = &input[..sample.min(input.len())];
    if head.is_empty() {
        return 0;
    }
    let enders = head.iter().filter(|&&c| !word_byte(c)).count();
    enders * 1000 / head.len()
}

/// Whether `pattern` matches anywhere, answered from the windows and stopping
/// at the first that holds a match, or `None` where
/// [`scan_required_windows`] hands the input back.
///
/// A caller asking only whether a match exists needs no span past the first
/// that answers, and needs no leftmost ordering among them, so no span past
/// the one after it is lexed: that one is lexed to learn whether it joins the
/// first.
#[must_use]
pub fn any_required_window(pattern: &crate::ast::Pattern, input: &[u8]) -> Option<bool> {
    required_window_spans_gated(pattern, input, true, WindowGate::ByteCount)
        .map(|spans| !spans.is_empty())
}

/// The leftmost match of `pattern`, answered from the windows and stopping at
/// the first that holds one, or `None` where [`scan_required_windows`] hands
/// the input back.
///
/// The first span holding a match holds the leftmost one. Every match holds
/// one of the required literals, the spans are disjoint and in order, and each
/// holds `2 * max_len` significant tokens beyond its windows on either side,
/// so a match from a later span begins after the end of every match an
/// earlier span can hold.
///
/// That count is in tokens because a token long enough can carry a match
/// further than `reach` bytes from its literal. The settling widens for
/// exactly that and declines where widening will not reach, so the route hands
/// the input back rather than answering from a span that cut a match short.
/// The agreement with the full route over such input is held by a test rather
/// than by the argument alone.
#[must_use]
pub fn first_required_window(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Option<Option<crate::engine::Span>> {
    required_window_spans_gated(pattern, input, true, WindowGate::ByteCount)
        .map(|spans| spans.into_iter().next())
}

/// Why the windows a literal opens cannot answer a shortest match.
///
/// A caller reading one `None` for all of these cannot tell a pattern the
/// route can never take from one this input happens to refuse, and a router
/// needs exactly that: the first is decided once for a pattern and held, the
/// second is a property of the bytes and has to be asked again.
#[derive(Debug)]
pub enum WindowRefusal {
    /// The pattern's length is unbounded or empty, or it resumes, resets the
    /// start or names the stream's end, so no window width follows from it.
    NoWindowForm,
    /// The pattern does not compile, so there is nothing to run over a window.
    Uncompiled,
    /// No literal that every match contains, so nothing anchors a window.
    NoLiteral,
    /// A literal anchors windows and they cost more than the input justifies:
    /// by count or coverage as the literal opens them, or by the bytes they
    /// settle to on the lexer's splits.
    TooDense,
    /// Widening stopped before a window held its match whole.
    ///
    /// The report itself stays inside: `SettleReport` is crate-private and
    /// this enum is not, and which refusal was made is the distinction a
    /// caller acts on. The report is still carried by the debug assertion at
    /// the refusal site, where its detail is read.
    Widening,
}

impl std::fmt::Display for WindowRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WindowRefusal::NoWindowForm => f.write_str("no window form"),
            WindowRefusal::Uncompiled => f.write_str("the pattern does not compile"),
            WindowRefusal::NoLiteral => f.write_str("no literal anchors a window"),
            WindowRefusal::TooDense => f.write_str("the windows cost more than the whole lex"),
            WindowRefusal::Widening => f.write_str("a window would not settle even widened"),
        }
    }
}

impl std::error::Error for WindowRefusal {}

/// How far the soonest-ending match of `pattern` reaches, read from the windows
/// a required literal opens, or the reason those windows cannot answer.
///
/// `Ok(None)` is the route running and finding no match, which is an answer.
/// `Err` is the route declining, which is not, and [`WindowRefusal`] says which
/// decline it was: a caller routing on the result needs to tell a pattern this
/// route can never take from one these bytes happen to refuse.
///
/// Every span the windows settle to is visited. A later span can hold a
/// sooner-ending match, so this cannot stop at the first the way
/// [`first_required_window`] does, and the answer is the least end any span
/// reports.
///
/// The least end among the matches a span selects is the wrong answer:
/// `\W{1,2}` over three words selects the first two and the third, whose ends
/// are after the second word and after the third, while the soonest-ending match
/// ends after the first. So each span is asked [`crate::nfa::shortest_end`]
/// directly rather than being asked for its matches.
///
/// A span that cuts a match short does not report it early - its tokens end
/// before the match completes, so it is not found there at all - and every
/// match is whole inside the span holding its literal, which is what makes the
/// least of the spans' ends the true answer.
pub fn shortest_end_in_windows(
    pattern: &crate::ast::Pattern,
    input: &[u8],
) -> Result<Option<usize>, WindowRefusal> {
    let Some(max_len) = crate::nfa::bounded_max_len(pattern) else {
        return Err(WindowRefusal::NoWindowForm);
    };
    if max_len == 0
        || pattern.starts_with_resume()
        || pattern.mentions_reset_start()
        || pattern.mentions_stream_end_anchor()
    {
        return Err(WindowRefusal::NoWindowForm);
    }
    if crate::nfa::compile_pattern(pattern).is_none() {
        return Err(WindowRefusal::Uncompiled);
    }
    let mut lits: Vec<Vec<u8>> = Vec::new();
    collect_contained_literals(pattern, &mut lits);
    // Asked before the gates, so "nothing to anchor on" and "the anchors cost
    // too much" are two answers rather than one. `windows_under_limit` filters
    // the empty literals out itself and then has nothing to try, which reads
    // the same as every literal being refused.
    if lits.iter().all(|l| l.is_empty()) {
        return Err(WindowRefusal::NoLiteral);
    }
    let reach = max_len.saturating_mul(WINDOW_BYTES_PER_TOKEN);
    let Some(rough) = windows_under_limit(input, &mut lits, reach, WindowGate::ByteCount) else {
        return Err(WindowRefusal::TooDense);
    };
    if rough.is_empty() {
        return Ok(None);
    }
    let budget = settled_budget(input.len(), WindowGate::ByteCount);
    if gather_windows(input, None, &rough, budget).is_none() {
        return Err(WindowRefusal::TooDense);
    }
    let quotes = crate::parallel_lex::quoted_spans(input);
    let Some(gathered) = gather_windows(input, Some(&quotes), &rough, budget) else {
        return Err(WindowRefusal::TooDense);
    };
    let mut best: Option<usize> = None;
    let settled = settle_gathered(input, &quotes, &gathered, max_len, budget, |_, _, toks| {
        if let Some(end) = crate::nfa::shortest_end(pattern, input, toks, 0)
            && best.is_none_or(|b| end < b)
        {
            best = Some(end);
        }
        false
    });
    match settled {
        Ok(_) => Ok(best),
        Err(Unsettled::Widening(report)) => {
            debug_assert!(report.widest > 0, "a window was refused unwidened: {report}");
            Err(WindowRefusal::Widening)
        }
        Err(Unsettled::Budget(bytes)) => {
            if crate::trace::on() {
                crate::trace::rung(
                    "windows",
                    &format!("declining: settling lexed {bytes} bytes, past the budget of {budget}"),
                    input.len(),
                );
            }
            Err(WindowRefusal::TooDense)
        }
    }
}

/// Why a region around one match cannot answer what that match bound.
pub(crate) enum RegionRefusal {
    /// The pattern's reach is unbounded or empty, or it reads the whole
    /// stream's ends, so no region of the input answers for it.
    NoRegionForm,
    /// Widening stopped before the region held the match whole.
    Widening(SettleReport),
}

impl std::fmt::Display for RegionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegionRefusal::NoRegionForm => f.write_str("no region form"),
            RegionRefusal::Widening(report) => report.fmt(f),
        }
    }
}

/// The tokens around `span`, settled onto boundaries where the chunked lexer
/// would cut, for resolving what that one match bound without lexing the
/// input.
///
/// The preconditions are the window route's: an unbounded reach has no region
/// to settle, and `\A` or `\z` read whether any token precedes or follows in
/// the whole stream, which a slice answers wrongly because its first token
/// looks like the input's.
///
/// The quote scan reads to the region's far edge and no further. A quote before
/// a position decides whether that position is inside a string, and one after it
/// decides nothing, so the scan is bounded by where the settle probes rather
/// than by the input's end - and a string still open at that edge is carried to
/// its close, because a position inside an unclosed string is not decided by
/// knowing where the string began.
///
/// That bound is the whole value of this path. A route found this match without
/// lexing, and scanning megabytes for quotes to settle a few dozen bytes around
/// it costs back more than the route saved.
pub(crate) fn tokens_around(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    span: crate::engine::Span,
) -> Result<Vec<crate::token::Token>, RegionRefusal> {
    let Some(max_len) = crate::nfa::bounded_max_len(pattern) else {
        return Err(RegionRefusal::NoRegionForm);
    };
    if max_len == 0
        || pattern.starts_with_resume()
        || pattern.mentions_reset_start()
        || pattern.mentions_stream_end_anchor()
    {
        return Err(RegionRefusal::NoRegionForm);
    }
    let mut scan = crate::parallel_lex::QuoteScan::new(input);
    match settle_window(
        input,
        &mut Quoting::AsProbed(&mut scan),
        span.start(),
        span.end(),
        max_len,
    ) {
        Ok((_, _, toks)) => Ok(toks),
        Err(report) => Err(RegionRefusal::Widening(report)),
    }
}

fn required_window_spans(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    first_only: bool,
    gated: bool,
) -> Option<Vec<crate::engine::Span>> {
    let gate = if gated { WindowGate::ByteCount } else { WindowGate::Open };
    required_window_spans_gated(pattern, input, first_only, gate)
}

fn required_window_spans_gated(
    pattern: &crate::ast::Pattern,
    input: &[u8],
    first_only: bool,
    gate: WindowGate,
) -> Option<Vec<crate::engine::Span>> {
    let max_len = crate::nfa::bounded_max_len(pattern)?;
    if max_len == 0
        || pattern.starts_with_resume()
        || pattern.mentions_reset_start()
        || pattern.mentions_stream_end_anchor()
    {
        return None;
    }
    // A pattern holding no literal anchors nothing, and it is handed back
    // before the program is built: the scan's ladder tries this route ahead
    // of the ones that lex the whole input, so a pattern of bare kinds costs
    // only this walk of its own tree here.
    let mut lits: Vec<Vec<u8>> = Vec::new();
    collect_contained_literals(pattern, &mut lits);
    if lits.iter().all(Vec::is_empty) {
        return None;
    }
    // One program for every window: the engine is entered once per window,
    // and a pattern that needs the set-reachability engine is declined here,
    // before the input is passed over at all.
    let compiled = crate::nfa::compile_pattern(pattern)?;
    // The reach in bytes, and the coverage gate on it, before a single window
    // is lexed: the gate exists to refuse a dense literal, and lexing to
    // decide would cost what it is refusing. A window is verified in tokens
    // once, after merging, where a dense literal has left one window rather
    // than one per hit.
    let reach = max_len.saturating_mul(WINDOW_BYTES_PER_TOKEN);
    let rough = windows_under_limit(input, &mut lits, reach, gate)?;
    if rough.is_empty() {
        return Some(Vec::new());
    }
    // How many windows the route is about to lex and how much of the input they
    // cover. The count is what decides whether there is anything to spread over
    // the cores, and it is not readable from the outside: the occurrences merge,
    // so a literal on every line can leave one window and a rare one thousands.
    if crate::trace::on() {
        let covered: usize = rough.iter().map(|&(x, y)| y - x).sum();
        crate::trace::rung(
            "windows",
            &format!(
                "taking {} window(s) covering {covered} bytes, {} per mille of the input",
                rough.len(),
                covered.saturating_mul(1000) / input.len().max(1)
            ),
            input.len(),
        );
    }

    let budget = settled_budget(input.len(), gate);
    // With every newline before a significant byte read as a split, the spans
    // are inside the lexer's settled spans, so a total already past the budget
    // there is declined before the strings are read.
    if gate != WindowGate::Open && gather_windows(input, None, &rough, budget).is_none() {
        if crate::trace::on() {
            crate::trace::rung(
                "windows",
                &format!("declining: the windows settle past the budget of {budget} bytes before the strings are read"),
                input.len(),
            );
        }
        return None;
    }
    let quotes = crate::parallel_lex::quoted_spans(input);
    // Priced from one window rather than predicted from the bytes: settle and
    // lex the first, count the significant tokens in it, and read the share
    // of the input's tokens all the windows would carry from that. A window's
    // cost is its tokens, and how many a window holds is a property of the
    // corpus that no count of bytes reports.
    if gate == WindowGate::PricedWindow {
        let &(a, b) = rough.first().expect("the windows are not empty here");
        let (lo, hi, toks) =
            match settle_window(input, &mut Quoting::Scanned(&quotes), a, b, max_len) {
                Ok(settled) => settled,
                Err(report) => {
                    debug_assert!(report.widest > 0, "a window was refused unwidened: {report}");
                    return None;
                }
            };
        let seen = toks.iter().filter(|t| t.is_significant()).count();
        let covered: usize = rough.iter().map(|&(x, y)| y - x).sum();
        // Tokens in the windows over tokens in the input, at the density this
        // one window actually showed: the per-token densities cancel and what
        // is left is the covered share, which is why this reads the same as the
        // coverage bound where density is uniform and differently where the
        // windows cover denser text. A window of no width showed no density
        // and prices nothing.
        let tokens_in = |bytes: usize| bytes.saturating_mul(seen).checked_div(hi - lo);
        if let (Some(in_windows), Some(in_input)) = (tokens_in(covered), tokens_in(input.len()))
            && (in_input == 0 || in_windows.saturating_mul(1000) / in_input > WINDOW_TOKEN_SHARE)
        {
            return None;
        }
    }
    let Some(gathered) = gather_windows(input, Some(&quotes), &rough, budget) else {
        if crate::trace::on() {
            crate::trace::rung(
                "windows",
                &format!("declining: the windows settle past the budget of {budget} bytes"),
                input.len(),
            );
        }
        return None;
    };
    if crate::trace::on() {
        let spans: usize = gathered.iter().map(|g| g.hi - g.lo).sum();
        crate::trace::rung(
            "windows",
            &format!(
                "gathered into {} span(s) covering {spans} bytes, {} per mille of the input",
                gathered.len(),
                spans.saturating_mul(1000) / input.len().max(1)
            ),
            input.len(),
        );
    }

    let mut out: Vec<crate::engine::Span> = Vec::new();
    let settled = settle_gathered(input, &quotes, &gathered, max_len, budget, |lo, hi, toks| {
        if crate::trace::on() {
            crate::trace::rung(
                "windows",
                &format!("span {lo}..{hi} lexed to {} tokens", toks.len()),
                input.len(),
            );
        }
        let spans = crate::nfa::scan_nfa_over_compiled(&compiled, pattern, input, toks);
        // A match ending within a match's length of the span's end may have
        // been cut short there. The input's own end cuts nothing short.
        if hi == input.len() {
            out.extend(spans);
        } else {
            let edge = match toks.iter().filter(|t| t.is_significant()).nth_back(max_len - 1) {
                Some(t) => t.start(),
                None => lo,
            };
            out.extend(spans.into_iter().filter(|s| s.end() <= edge));
        }
        // A caller asking only whether a match exists has its answer, and no
        // span past the next is lexed.
        first_only && !out.is_empty()
    });
    match settled {
        Ok(s) => {
            if crate::trace::on() {
                crate::trace::rung(
                    "windows",
                    &format!(
                        "settled {} span(s), lexing {} bytes, {} per mille of the input, the widest {}",
                        s.spans,
                        s.bytes,
                        s.bytes.saturating_mul(1000) / input.len().max(1),
                        s.widest
                    ),
                    input.len(),
                );
            }
        }
        Err(Unsettled::Widening(report)) => {
            debug_assert!(report.widest > 0, "a window was refused unwidened: {report}");
            return None;
        }
        Err(Unsettled::Budget(bytes)) => {
            if crate::trace::on() {
                crate::trace::rung(
                    "windows",
                    &format!("declining: settling lexed {bytes} bytes, past the budget of {budget}"),
                    input.len(),
                );
            }
            return None;
        }
    }
    out.sort_by_key(crate::engine::Span::start);
    out.dedup();
    Some(out)
}

/// The merged windows around the occurrences of one of `lits`, which hold
/// every match of the pattern, for the first literal whose windows cover at
/// most [`WINDOW_COVERAGE_LIMIT`] of `input`, and `None` where none of them
/// does.
///
/// Every match holds every one of these literals inside its own span, so any
/// one of them is a sound anchor and the first that fits is taken. They are
/// tried longest first, since a longer needle is more often the rarer one.
/// `lits` is left sorted by that order.
fn windows_under_limit(
    input: &[u8],
    lits: &mut [Vec<u8>],
    reach: usize,
    gate: WindowGate,
) -> Option<Vec<(usize, usize)>> {
    lits.sort_by_key(|l| std::cmp::Reverse(l.len()));
    lits.iter()
        .filter(|l| !l.is_empty())
        .find_map(|lit| windows_of_literal(input, lit, reach, gate))
}

/// The merged windows around every occurrence of `lit` in `input`, or `None`
/// as soon as there are more of them than the input's length justifies, or as
/// soon as they cover more than [`WINDOW_COVERAGE_LIMIT`] of it.
///
/// Occurrences arrive ascending, so each one either extends the last window or
/// opens a new one, and both the count and the coverage grow with them. A
/// literal on every line is abandoned at whichever bound it reaches rather
/// than collected first.
fn windows_of_literal(
    input: &[u8],
    lit: &[u8],
    reach: usize,
    gate: WindowGate,
) -> Option<Vec<(usize, usize)>> {
    // The count bound is the one that reads bytes and cannot see token
    // density; the coverage bound is the one every gate but `Open` keeps.
    // `PredictedDensity` scales the coverage it allows by how token-dense the
    // input looks, so a corpus whose windows hold few tokens is allowed more
    // of them - which is the quantity the byte bound cannot see.
    let (most, share) = match gate {
        WindowGate::ByteCount => (
            input.len() / (WINDOW_COST_IN_INPUT_BYTES * WINDOW_MARGIN),
            WINDOW_COVERAGE_LIMIT,
        ),
        WindowGate::CoverageOnly | WindowGate::PricedWindow => {
            (usize::MAX, WINDOW_COVERAGE_LIMIT)
        }
        WindowGate::PredictedDensity => {
            // Statements end a token about every four bytes; two-hundred-byte
            // text about every two hundred. The allowance rises with the
            // second over the first, capped at the whole input.
            let enders = token_enders_per_mille(input, 1 << 16).max(1);
            (usize::MAX, (WINDOW_COVERAGE_LIMIT * 250 / enders).min(1000))
        }
        WindowGate::Open => (usize::MAX, usize::MAX),
    };
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut covered = 0usize;
    let mut from = 0usize;
    while let Some(rel) = crate::byte_simd::find(&input[from..], lit) {
        let p = from + rel;
        from = p + 1;
        let (a, b) = (p.saturating_sub(reach), (p + reach).min(input.len()));
        match out.last_mut() {
            Some(last) if a <= last.1 => {
                if b > last.1 {
                    covered += b - last.1;
                    last.1 = b;
                }
            }
            _ => {
                covered += b - a;
                out.push((a, b));
            }
        }
        // A hit means the needle fit, so `input` is not empty here. Both
        // quantities only grow, so a verdict here is the verdict the whole
        // run would reach.
        if out.len() > most || covered * 1000 / input.len() > share {
            return None;
        }
    }
    Some(out)
}

/// The byte span `a..b` settled onto boundaries where the chunked lexer would
/// cut, widened until it holds twice the longest match in tokens beyond each
/// edge of `a..b`. `None` where widening will not reach.
///
/// The check is in tokens because a match is bounded in tokens and a token is
/// not bounded in bytes: a blob run can be thousands. The widening is the
/// guard for that case rather than the ordinary path - a token averages two
/// to six bytes, so the first try is generous by an order of magnitude.
fn settle_window(
    input: &[u8],
    quoting: &mut Quoting<'_, '_>,
    a: usize,
    b: usize,
    max_len: usize,
) -> Result<(usize, usize, Vec<crate::token::Token>), SettleReport> {
    let enough = 2 * max_len;
    let mut out = 0usize;
    let mut last = SettleReport {
        span: (a, b),
        lo: a,
        hi: b,
        before: 0,
        after: 0,
        enough,
        widest: 0,
    };
    for _ in 0..WINDOW_WIDENINGS {
        let (lo, hi) = match quoting {
            Quoting::Scanned(quotes) => (
                boundary_at_or_before(input, quotes, a.saturating_sub(out)),
                boundary_at_or_after(input, quotes, (b + out).min(input.len())),
            ),
            Quoting::AsProbed(scan) => {
                // Every position the backward walk probes is at or before the
                // span's start, so one ask settles all of them; the forward walk
                // asks as it goes, because where it stops is what it is finding.
                scan.ensure(a, &mut |_| Some(true));
                let lo = boundary_at_or_before(input, scan.spans(), a.saturating_sub(out));
                let hi = boundary_at_or_after_scanned(input, scan, (b + out).min(input.len()));
                (lo, hi)
            }
        };
        let toks = lex_window(input, lo, hi);
        let before = toks.iter().filter(|t| t.is_significant() && t.end() <= a).count();
        let after = toks.iter().filter(|t| t.is_significant() && t.start() >= b).count();
        if (before >= enough || lo == 0) && (after >= enough || hi == input.len()) {
            return Ok((lo, hi, toks));
        }
        last = SettleReport { span: (a, b), lo, hi, before, after, enough, widest: out };
        out = if out == 0 { max_len * WINDOW_BYTES_PER_TOKEN } else { out.saturating_mul(2) };
    }
    Err(last)
}

/// Bytes the settled spans may lex before the window route costs more than
/// the lex it replaces: [`WINDOW_COVERAGE_LIMIT`] of the input, and no bound
/// where the gate is open.
fn settled_budget(input_len: usize, gate: WindowGate) -> usize {
    match gate {
        WindowGate::Open => usize::MAX,
        WindowGate::ByteCount
        | WindowGate::CoverageOnly
        | WindowGate::PricedWindow
        | WindowGate::PredictedDensity => input_len.saturating_mul(WINDOW_COVERAGE_LIMIT) / 1000,
    }
}

/// Rough windows gathered into one span of the input: `lo..hi` on splits the
/// chunked lexer takes, holding every rough window from `a` to `b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Gathered {
    lo: usize,
    hi: usize,
    a: usize,
    b: usize,
}

/// The rough windows gathered by the spans they settle to on a first try:
/// each taken out to the nearest split before and after it, and those whose
/// spans overlap or touch gathered into one. `None` as soon as the spans cover
/// more than `budget` bytes.
///
/// A walk back stops at the span before it and a walk forward starts past
/// that span's end, so no byte is walked twice, and neither walks further than
/// the budget left.
///
/// Without `quotes` every newline before a significant byte is a split. That
/// puts each split at or nearer than the one the lexer takes, so the spans are
/// inside the true ones and their total bounds the true total from below.
fn gather_windows(
    input: &[u8],
    quotes: Option<&[(usize, usize)]>,
    rough: &[(usize, usize)],
    budget: usize,
) -> Option<Vec<Gathered>> {
    let mut out: Vec<Gathered> = Vec::new();
    let mut covered = 0usize;
    for &(a, b) in rough {
        let left = budget - covered;
        let floor = match out.last() {
            Some(last) => last.hi,
            None => 0,
        };
        let reach = a.saturating_sub(left).max(floor);
        if let Some(lo) = cut_back(input, quotes, a, reach) {
            let hi = cut_forward(input, quotes, b, lo.saturating_add(left))?;
            covered += hi - lo;
            out.push(Gathered { lo, hi, a, b });
            continue;
        }
        // The budget stopped the walk short of a split and of the span
        // before, so this window's span alone would pass it.
        if reach > floor {
            return None;
        }
        match out.last_mut() {
            Some(last) => {
                if b > last.hi {
                    let hi = cut_forward(input, quotes, b, last.hi.saturating_add(left))?;
                    covered += hi - last.hi;
                    last.hi = hi;
                }
                last.b = b;
            }
            None => {
                let hi = cut_forward(input, quotes, b, left)?;
                covered += hi;
                out.push(Gathered { lo: 0, hi, a, b });
            }
        }
    }
    Some(out)
}

/// What settling gathered windows lexed: the spans handed on, their bytes, the
/// widest, and the bytes the lexer read in all, which exceed the spans' only
/// by a span lexed and not handed on after an early stop.
#[derive(Clone, Copy, Debug, Default)]
struct SettledSpans {
    spans: usize,
    bytes: usize,
    widest: usize,
    lexed: usize,
}

impl SettledSpans {
    fn add(&mut self, lo: usize, hi: usize) {
        self.spans += 1;
        self.bytes += hi - lo;
        self.widest = self.widest.max(hi - lo);
    }
}

/// Why gathered windows did not settle.
enum Unsettled {
    /// A side would not widen far enough to hold its matches whole.
    Widening(SettleReport),
    /// The bytes lexed passed the budget, having reached this many.
    Budget(usize),
}

/// A span being settled: its bounds, the first and last rough window it
/// holds, and the tokens lexed in it so far.
struct OpenSpan {
    lo: usize,
    hi: usize,
    a: usize,
    b: usize,
    toks: Vec<crate::token::Token>,
}

impl OpenSpan {
    /// Significant tokens that end at or before the span's first window.
    fn before(&self) -> usize {
        self.toks.iter().filter(|t| t.is_significant() && t.end() <= self.a).count()
    }

    /// Significant tokens that start at or after the span's last window ends.
    fn after(&self) -> usize {
        self.toks.iter().filter(|t| t.is_significant() && t.start() >= self.b).count()
    }

    fn report(&self, enough: usize, widest: usize) -> SettleReport {
        SettleReport {
            span: (self.a, self.b),
            lo: self.lo,
            hi: self.hi,
            before: self.before(),
            after: self.after(),
            enough,
            widest,
        }
    }
}

/// Spans of one input lexed against a budget of bytes.
struct BudgetLexer<'i> {
    input: &'i [u8],
    lexed: usize,
    budget: usize,
}

impl BudgetLexer<'_> {
    fn lex(&mut self, lo: usize, hi: usize) -> Result<Vec<crate::token::Token>, Unsettled> {
        self.lexed += hi - lo;
        if self.lexed > self.budget {
            return Err(Unsettled::Budget(self.lexed));
        }
        Ok(lex_window(self.input, lo, hi))
    }
}

/// Widens `open` forward until `enough` significant tokens follow its last
/// window, stepping as [`settle_window`] steps. `Ok(true)` where a step
/// reaches `next`, the start of the span after it, which the caller joins.
fn widen_forward(
    open: &mut OpenSpan,
    quotes: &[(usize, usize)],
    enough: usize,
    max_len: usize,
    next: usize,
    lexer: &mut BudgetLexer<'_>,
) -> Result<bool, Unsettled> {
    let n = lexer.input.len();
    let mut out = 0usize;
    for _ in 1..WINDOW_WIDENINGS {
        if open.hi == n || open.after() >= enough {
            return Ok(false);
        }
        out = if out == 0 { max_len * WINDOW_BYTES_PER_TOKEN } else { out.saturating_mul(2) };
        let hi = boundary_at_or_after(lexer.input, quotes, open.b.saturating_add(out).min(n));
        if hi >= next {
            return Ok(true);
        }
        if hi > open.hi {
            let more = lexer.lex(open.hi, hi)?;
            open.toks.extend(more);
            open.hi = hi;
        }
    }
    if open.hi == n || open.after() >= enough {
        return Ok(false);
    }
    Err(Unsettled::Widening(open.report(enough, out)))
}

/// Widens `open` back until `enough` significant tokens precede its first
/// window. `Ok(true)` where a step reaches `prev`, the end of the span before
/// it, which the caller joins; the first span has none.
fn widen_back(
    open: &mut OpenSpan,
    quotes: &[(usize, usize)],
    enough: usize,
    max_len: usize,
    prev: Option<usize>,
    lexer: &mut BudgetLexer<'_>,
) -> Result<bool, Unsettled> {
    let mut out = 0usize;
    for _ in 1..WINDOW_WIDENINGS {
        if open.lo == 0 || open.before() >= enough {
            return Ok(false);
        }
        out = if out == 0 { max_len * WINDOW_BYTES_PER_TOKEN } else { out.saturating_mul(2) };
        let lo = boundary_at_or_before(lexer.input, quotes, open.a.saturating_sub(out));
        if prev.is_some_and(|p| lo <= p) {
            return Ok(true);
        }
        if lo < open.lo {
            let mut toks = lexer.lex(lo, open.lo)?;
            toks.append(&mut open.toks);
            open.toks = toks;
            open.lo = lo;
        }
    }
    if open.lo == 0 || open.before() >= enough {
        return Ok(false);
    }
    Err(Unsettled::Widening(open.report(enough, out)))
}

/// The gathered spans lexed once each and handed to `each` with their bounds
/// and tokens, in order, until `each` returns `true`.
///
/// A span wants `2 * max_len` significant tokens between each edge and the
/// windows it holds, as [`settle_window`] does, and a side short of them
/// widens as it widens there. A widening that reaches the neighboring span
/// joins the two: the bytes between are lexed and the runs laid end to end,
/// which relies on a span cut at a chunked-lexer split lexing alone exactly as
/// it lexes in place. So the spans handed on are disjoint and no byte is lexed
/// twice, and a span is handed on only once the span after it is lexed and
/// can no longer join it.
fn settle_gathered(
    input: &[u8],
    quotes: &[(usize, usize)],
    gathered: &[Gathered],
    max_len: usize,
    budget: usize,
    mut each: impl FnMut(usize, usize, &[crate::token::Token]) -> bool,
) -> Result<SettledSpans, Unsettled> {
    let enough = 2 * max_len;
    let mut lexer = BudgetLexer { input, lexed: 0, budget };
    let mut settled = SettledSpans::default();
    let mut open: Option<OpenSpan> = None;
    for g in gathered {
        let mut next = OpenSpan { lo: g.lo, hi: g.hi, a: g.a, b: g.b, toks: lexer.lex(g.lo, g.hi)? };
        let join = match open.as_mut() {
            Some(cur) => {
                widen_forward(cur, quotes, enough, max_len, next.lo, &mut lexer)?
                    || widen_back(&mut next, quotes, enough, max_len, Some(cur.hi), &mut lexer)?
            }
            None => widen_back(&mut next, quotes, enough, max_len, None, &mut lexer)?,
        };
        if join {
            let cur = open.as_mut().expect("only a span with one before it joins");
            let gap = lexer.lex(cur.hi, next.lo)?;
            cur.toks.extend(gap);
            cur.toks.append(&mut next.toks);
            cur.hi = next.hi;
            cur.b = next.b;
        } else if let Some(done) = open.replace(next) {
            settled.add(done.lo, done.hi);
            if each(done.lo, done.hi, &done.toks) {
                settled.lexed = lexer.lexed;
                return Ok(settled);
            }
        }
    }
    if let Some(mut last) = open {
        let joined = widen_forward(&mut last, quotes, enough, max_len, usize::MAX, &mut lexer)?;
        debug_assert!(!joined, "no span follows the last for it to join");
        settled.add(last.lo, last.hi);
        each(last.lo, last.hi, &last.toks);
    }
    settled.lexed = lexer.lexed;
    Ok(settled)
}

/// Where a settle reads quoting from.
///
/// The two differ in what they are settling. A caller settling windows all over
/// the input scans it once for all of them, and the scan is a fraction of what
/// the windows themselves cost. A caller settling a single region has no such
/// amortization: a route found that match without lexing, and a whole-input
/// quote scan to settle thirty bytes around it costs back more than the route
/// saved. Only quotes at or before a position decide it, so that caller reads
/// the input as far as it probes and no further.
enum Quoting<'q, 'i> {
    /// Spans for the whole input, scanned once and shared by many settles.
    Scanned(&'q [(usize, usize)]),
    /// A scan carried forward only as far as this settle probes.
    AsProbed(&'q mut crate::parallel_lex::QuoteScan<'i>),
}

/// Whether byte `p` is inside a string, read off spans that decide every
/// position at or before it.
fn quote_holds(quotes: &[(usize, usize)], p: usize) -> bool {
    let k = quotes.partition_point(|&(open, _)| open <= p);
    k > 0 && quotes[k - 1].1 >= p
}

/// The nearest position at or before `at` where the chunked lexer would split:
/// a significant byte right after a newline, with neither inside a string.
/// Falls back to the input's start.
fn boundary_at_or_before(input: &[u8], quotes: &[(usize, usize)], at: usize) -> usize {
    cut_back(input, Some(quotes), at, 0).unwrap_or(0)
}

/// The mirror of [`boundary_at_or_before`], falling back to the input's end.
fn boundary_at_or_after(input: &[u8], quotes: &[(usize, usize)], at: usize) -> usize {
    cut_forward(input, Some(quotes), at, input.len()).unwrap_or(input.len())
}

/// Whether the chunked lexer may split before byte `i`: a significant byte
/// right after a newline and, where `quotes` are given, with neither it nor
/// the newline inside a string. Without them every such byte counts.
fn cuts_at(input: &[u8], quotes: Option<&[(usize, usize)]>, i: usize) -> bool {
    i > 0
        && i < input.len()
        && input[i - 1] == b'\n'
        && !input[i].is_ascii_whitespace()
        && quotes.is_none_or(|q| !quote_holds(q, i - 1) && !quote_holds(q, i))
}

/// The nearest split in `floor + 1..=at`, or `None` where there is none. Only
/// a byte after a newline can be one, so the walk steps between newlines.
fn cut_back(input: &[u8], quotes: Option<&[(usize, usize)]>, at: usize, floor: usize) -> Option<usize> {
    let mut i = at.min(input.len());
    while i > floor {
        if cuts_at(input, quotes, i) {
            return Some(i);
        }
        i = floor + input[floor..i - 1].iter().rposition(|&c| c == b'\n')? + 1;
    }
    None
}

/// The nearest split in `at..=limit`, the input's end counting as one, or
/// `None` where neither is within `limit`.
fn cut_forward(input: &[u8], quotes: Option<&[(usize, usize)]>, at: usize, limit: usize) -> Option<usize> {
    let n = input.len();
    let mut i = at.min(n);
    while i <= limit {
        if i == n || cuts_at(input, quotes, i) {
            return Some(i);
        }
        i = match crate::byte_simd::find(&input[i..], b"\n") {
            Some(k) => i + k + 1,
            None => n,
        };
    }
    None
}

/// [`boundary_at_or_after`] reading a scan that is carried forward only as far
/// as the walk actually probes.
///
/// The walk asks about quoting at the positions where a newline is followed by
/// a significant byte, and nowhere else, so the scan advances at those and not
/// a byte sooner. A region near the start of a large input therefore costs a
/// search of its own neighborhood rather than of the whole input, which is the
/// whole point of settling a region instead of lexing everything.
fn boundary_at_or_after_scanned(
    input: &[u8],
    scan: &mut crate::parallel_lex::QuoteScan<'_>,
    at: usize,
) -> usize {
    let n = input.len();
    let mut i = at.min(n);
    while i < n {
        if i > 0 && input[i - 1] == b'\n' && !input[i].is_ascii_whitespace() {
            scan.ensure(i, &mut |_| Some(true));
            let quotes = scan.spans();
            if !quote_holds(quotes, i - 1) && !quote_holds(quotes, i) {
                return i;
            }
        }
        i += 1;
    }
    n
}

/// [`lex_window`] for a probe outside the crate: `input[lo..hi]` lexed as a
/// settled span is, its tokens' spans absolute in `input`.
#[doc(hidden)]
#[must_use]
pub fn lex_span_as_a_window(input: &[u8], lo: usize, hi: usize) -> Vec<crate::token::Token> {
    lex_window(input, lo, hi)
}

/// `input[lo..hi]` lexed, its tokens carrying spans absolute in `input`, so
/// the engine reading them reads the whole input for a guard's forward search
/// and for the bytes a line anchor tests, exactly as it would over the whole
/// stream's tokens.
///
/// The blob table is the window's own, each whitespace-free span inside the
/// window judged on its own bytes and seeded from the bytes before it, so a
/// high-entropy run reads here as it reads in a whole-input lex; a blob holds
/// no whitespace and a window edge is a significant byte after a newline, so
/// no blob crosses one.
fn lex_window(input: &[u8], lo: usize, hi: usize) -> Vec<crate::token::Token> {
    let blobs = crate::lexer::blob_runs_within(input, lo, hi);
    let local: Vec<(usize, usize)> = blobs.iter().map(|&(a, b)| (a - lo, b - lo)).collect();
    let mut toks = crate::lexer::lex_with_blobs(
        &input[lo..hi],
        &local,
        (hi - lo) / crate::lexer::TOKEN_BYTES_ESTIMATE,
    );
    for t in &mut toks {
        t.start += lo as u32;
        t.end += lo as u32;
    }
    toks
}

/// The byte strings every match of `pattern` must hold inside its own span.
///
/// [`collect_required_literals`] collects what an input must hold for a match
/// to exist anywhere in it, which is a weaker claim and a different one: a
/// positive guard's literal need only be at or after the match, and a positive
/// assertion is zero width, so neither is inside the span. A caller reasoning
/// about where a match can be needs this one; a caller asking only whether a
/// match can exist wants the other.
///
/// Soundness runs one way only, as it does there: a literal named here must be
/// inside every match, so naming too few costs an optimization and naming one
/// too many silently loses matches.
fn collect_contained_literals(pattern: &crate::ast::Pattern, out: &mut Vec<Vec<u8>>) {
    use crate::ast::{Atom, Pattern};
    use crate::orbit::OrbitGroup;
    match pattern {
        Pattern::Atom(Atom::Literal(lit, OrbitGroup::Identity)) if !lit.is_empty() => {
            out.push(lit.as_bytes().to_vec());
        }
        // A typed token holds its recognizer's required literal inside its own
        // bytes, so a match of the atom does too.
        Pattern::Atom(Atom::Kind(kind)) => {
            if let Some(lit) = kind_required_literal(*kind) {
                out.push(lit.to_vec());
            }
        }
        Pattern::Plus(inner, _)
        | Pattern::Bind(_, _, inner)
        | Pattern::Balanced(_, inner)
        | Pattern::Field(_, inner)
        | Pattern::Atomic(inner) => collect_contained_literals(inner, out),
        // A repeat that must run at least once holds its body's literals in
        // every match, read the same way as `Plus`; one that may run no times
        // holds nothing.
        Pattern::Repeat(inner, min, _, _) if *min >= 1 => {
            collect_contained_literals(inner, out);
        }
        Pattern::Concat(v) => {
            for p in v {
                collect_contained_literals(p, out);
            }
        }
        // The alternation holds only the literals common to every branch.
        Pattern::Alt(v, _) => {
            let Some((first, rest)) = v.split_first() else { return };
            let mut common: Vec<Vec<u8>> = Vec::new();
            collect_contained_literals(first, &mut common);
            for p in rest {
                let mut theirs: Vec<Vec<u8>> = Vec::new();
                collect_contained_literals(p, &mut theirs);
                common.retain(|l| theirs.contains(l));
            }
            out.append(&mut common);
        }
        // A guard is satisfied by a literal at or after the match, which may
        // be any distance from it, and an assertion consumes nothing: neither
        // puts bytes inside the span. An edit-distance group's alignment may
        // leave any of its atoms out, so none of its literals is in every
        // match of the group.
        Pattern::Guard(..)
        | Pattern::Assert(..)
        | Pattern::Atom(_)
        | Pattern::Within(..)
        | Pattern::Star(..)
        | Pattern::Opt(..)
        | Pattern::Repeat(..)
        | Pattern::Empty
        | Pattern::Anchor(_) => {}
    }
}

#[cfg(test)]
mod contained_literal_tests {
    use super::collect_contained_literals;

    fn contained(src: &str) -> Vec<String> {
        let p = crate::parse(src).expect("pattern parses");
        let mut out = Vec::new();
        collect_contained_literals(&p, &mut out);
        out.iter().map(|l| String::from_utf8_lossy(l).into_owned()).collect()
    }

    #[test]
    fn a_repeat_holds_its_body_only_when_it_must_run() {
        assert_eq!(contained("\"alpha\"{1,2}"), ["alpha"], "one or two hold one");
        assert_eq!(contained("\"alpha\"{2,4}"), ["alpha"], "two or more hold one");
        assert_eq!(contained("\"alpha\"{3}"), ["alpha"], "an exact count holds one");
        assert!(contained("\"alpha\"{0,2}").is_empty(), "none or two hold none");
        assert!(contained("\"alpha\"*").is_empty(), "a star holds none");
        assert!(contained("\"alpha\"?").is_empty(), "an option holds none");
        assert_eq!(contained("\"alpha\"+"), ["alpha"], "a plus holds one");
    }

    #[test]
    fn a_typed_atom_holds_its_kinds_required_literal() {
        assert_eq!(contained("\\E"), ["@"]);
        assert_eq!(contained("\\U"), ["://"]);
        assert_eq!(contained("\\E \\W"), ["@"], "a sequence holds each atom's");
        assert!(contained("(\\E | \\U)").is_empty(), "the two branches share no literal");
        assert!(contained("\\E?").is_empty(), "an optional atom holds none");
        assert!(contained("\\I").is_empty(), "an address has two forms and requires neither");
    }

    /// The window route relies on this containment, read off the lexer: every
    /// token of a kind holds its required literal inside its own span.
    #[test]
    fn every_token_of_a_kind_holds_its_literal_inside_its_span() {
        use crate::token::TokenKind;
        let samples: [(TokenKind, &str); 10] = [
            (TokenKind::Email, "write to ops.team+alerts@example.co.uk today"),
            (TokenKind::Url, "see https://example.org/a?b=c#d for more"),
            (TokenKind::Jwt, "token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl ok"),
            (TokenKind::Percent, "load at 97.5% then 12%"),
            (TokenKind::HexColor, "color: #fff; border: #a0b1c2;"),
            (TokenKind::Version, "upgraded v1.2.3 to 2.0.0-rc.1"),
            (TokenKind::Cidr, "allow 10.0.0.0/8 and 192.168.1.0/24"),
            (TokenKind::Uuid, "id 123e4567-e89b-12d3-a456-426614174000 done"),
            (TokenKind::Money, "paid $1,234.56 and $5"),
            (TokenKind::Geo, "at 40.7128, -74.0060 now"),
        ];
        for (kind, text) in samples {
            let lit = super::kind_required_literal(kind).expect("each of these kinds names a literal");
            let toks: Vec<_> = crate::lexer::lex(text.as_bytes()).into_iter().filter(|t| t.kind == kind).collect();
            assert!(!toks.is_empty(), "{kind:?}: the sample {text:?} lexes no token of the kind");
            for t in toks {
                let span = &text.as_bytes()[t.start as usize..t.end as usize];
                assert!(
                    span.windows(lit.len()).any(|w| w == lit),
                    "{kind:?} token {:?} does not hold {:?}",
                    String::from_utf8_lossy(span),
                    String::from_utf8_lossy(lit)
                );
            }
        }
    }
}

/// The bytes a token of `kind` must contain, where its recognizer requires
/// them unconditionally. Each is inside the token's own span rather than
/// beside it, which [`collect_contained_literals`] relies on to anchor windows.
///
/// Soundness is the same one way as the caller's: a kind named here must be
/// impossible in an input lacking the literal, so naming one too many
/// silently loses matches. Each entry is read off the recognizer's own
/// refusal rather than off the gate that admits it - the gates are wider
/// than the recognizers, and `Email`'s admits `.` `%` `+` `-` beside `@`.
///
/// Absent on purpose, for one reason only: a kind with alternative forms
/// requires none of them singly. An `Ip` is v4 with dots or v6 with colons, a
/// `Mac` takes either separator, a `Path` takes either slash, a `Phone` is
/// `+`-led or hyphenated, and a `Timestamp` is dated or clocked. Naming one
/// literal for any of these refuses an input holding the other form. They need
/// every alternative absent, which is a different question from this one.
///
/// How common a literal is, is not a reason to leave its kind out. The search
/// this feeds is [`crate::byte_simd::contains`], which stops at the first
/// occurrence: a literal on the corpus's first line costs that line, and a
/// literal the corpus lacks saves the whole lex. A dot is cheap to look for
/// because it is everywhere.
fn kind_required_literal(kind: crate::token::TokenKind) -> Option<&'static [u8]> {
    use crate::token::TokenKind;
    match kind {
        // `try_email` returns None unless the byte ending the local part is
        // `@`, so no Email token exists in an input without one.
        TokenKind::Email => Some(b"@"),
        // `try_url` returns None unless the three bytes after the scheme
        // are `://`.
        TokenKind::Url => Some(b"://"),
        // `try_jwt` opens by refusing anything that does not start `eyJ`,
        // which is the base64url of a JSON object's `{"`. Three bytes and
        // the rarest of these in ordinary text.
        TokenKind::Jwt => Some(b"eyJ"),
        // `try_percent` ends by requiring the byte after the digits to be
        // `%`, and `try_hexcolor` opens by requiring `#`.
        TokenKind::Percent => Some(b"%"),
        TokenKind::HexColor => Some(b"#"),
        // `try_semver` reads exactly three dotted numeric parts, so a version
        // string holds a `.`. `try_cidr` requires the byte after the address
        // to be `/`; `try_uuid` requires a `-` closing each of its first four
        // groups; `try_money` opens by requiring `$`; and `try_geo` requires
        // the `,` between its two degrees.
        TokenKind::Version => Some(b"."),
        TokenKind::Cidr => Some(b"/"),
        TokenKind::Uuid => Some(b"-"),
        TokenKind::Money => Some(b"$"),
        TokenKind::Geo => Some(b","),
        _ => None,
    }
}

/// The byte class whose runs make up a token of `kind`, and the shortest run
/// that could be one, where its recognizer requires a run at all.
///
/// The counterpart of [`kind_required_literal`] for the kinds that name no
/// literal. A base64 blob, a hash digest and a hex run are each a run of one
/// class with a length condition, which is a fact about the bytes a scan can
/// read without lexing - and none is reachable any other way: none requires a
/// fixed byte string, and an unanchored automaton for any of them is
/// combinatorial in live starting positions, because every position is at a
/// different phase of its length counter.
///
/// Soundness runs one way, as it does for the literal table: every token of
/// the kind must contain the run named here, so naming a length above the
/// shortest such run loses matches. Each is read off what the recognizer
/// measures rather than off the number it states, and those differ for base64.
fn kind_required_run(kind: crate::token::TokenKind) -> Option<(crate::byte_nfa::ByteClass, usize)> {
    use crate::byte_nfa::ByteClass;
    use crate::token::TokenKind;
    let digits = ByteClass::range(b'0', b'9');
    match kind {
        // `try_base64` measures a length that includes up to two bytes of `=`
        // padding and refuses below sixteen, so the run of alphabet bytes
        // alone can be as short as fourteen. Sixteen - the number the refusal
        // states - would refuse an input holding a fourteen-byte body with its
        // padding.
        TokenKind::Base64 => Some((
            digits
                .union(ByteClass::range(b'a', b'z'))
                .union(ByteClass::range(b'A', b'Z'))
                .union(ByteClass::just(b'+'))
                .union(ByteClass::just(b'/')),
            14,
        )),
        // `try_hash` is gated on a word run of exactly 32, 40 or 64 and takes
        // no padding, so the shortest hex run that could be one is 32.
        TokenKind::HashDigest => Some((
            digits.union(ByteClass::range(b'a', b'f')).union(ByteClass::range(b'A', b'F')),
            32,
        )),
        // `try_hex` takes a hex run longer than 32 at a length no digest has,
        // so the shortest hex run that could be one is 33.
        TokenKind::Hex => Some((
            digits.union(ByteClass::range(b'a', b'f')).union(ByteClass::range(b'A', b'F')),
            33,
        )),
        // `try_mac` reads exactly six pairs of hex digits joined by five
        // copies of one separator, which is `:` or `-` and the same one
        // throughout, so a Mac token is always seventeen bytes and every one
        // of them is a hex digit or a separator. The class admits both
        // separators where the token holds only one, and admits runs no Mac
        // could be - a UUID is thirty-six bytes of the class - which is the
        // safe direction for a necessary condition to err.
        //
        // The kinds beside this one that carry no required literal - Ip, Path,
        // Phone, Timestamp - carry no required run either, because each has
        // alternative forms and the shortest is too short or too broad to
        // justify a refusal: an IPv6 address may be `::1`, three bytes of a
        // class that most text contains.
        TokenKind::Mac => Some((
            digits
                .union(ByteClass::range(b'a', b'f'))
                .union(ByteClass::range(b'A', b'F'))
                .union(ByteClass::just(b':'))
                .union(ByteClass::just(b'-')),
            17,
        )),
        _ => None,
    }
}

/// Whether `pattern` is a bare typed atom whose kind must be a run of one byte
/// class, and `input` holds no run of that class long enough, so no match can
/// exist and the caller can answer without lexing.
///
/// Only a bare atom. A run condition is necessary for the kind rather than for
/// the pattern, and carrying it through a sequence or an alternation would
/// need the same case analysis [`collect_required_literals`] does; until that
/// is written, a pattern that is anything else is admitted rather than read
/// wrongly.
///
/// The scan costs one pass at about 0.15 to 0.41 ns per byte through
/// [`crate::byte_simd::ClassTables`], against a parallel lex of 1.16 to 4.11,
/// and it is reached only by a pattern whose kind names a run - so a pattern
/// that names none costs a match on its kind and nothing else.
#[must_use]
pub fn requires_absent_run(pattern: &crate::ast::Pattern, input: &[u8]) -> bool {
    use crate::ast::{Atom, Pattern};
    let Pattern::Atom(Atom::Kind(kind)) = pattern else {
        return false;
    };
    let Some((class, least)) = kind_required_run(*kind) else {
        return false;
    };
    let Some(tables) = crate::byte_simd::ClassTables::for_class(&class) else {
        return false;
    };
    !tables.has_run_of(input, least)
}

/// The byte strings an input must hold for `pattern` to match anywhere in it.
///
/// Soundness runs one way only: a literal named here must appear in any input
/// with a match, so naming too few costs a skipped optimization and naming one
/// too many silently loses matches. Every case that is not certain contributes
/// nothing.
///
/// This says the input holds the literal, not that a match does: a positive
/// guard's literal can be far past the match it admits. A caller reasoning
/// about where a match can be wants [`collect_contained_literals`].
///
/// What that rules out is not obvious from the node names. A negated guard
/// (`!~"lit"`) is satisfied by absence, so its literal is not required. A
/// negated assertion succeeds when its body fails, so nothing inside it is
/// required either. A literal atom under a non-identity orbit compares by
/// symmetry rather than bytes - `(?orbit:case "Cat")` matches `cat` - so its
/// own bytes need not appear. A body that may match zero times requires
/// nothing, and a counted repetition is left out rather than read wrongly.
fn collect_required_literals(pattern: &crate::ast::Pattern, out: &mut Vec<Vec<u8>>) {
    use crate::ast::{Atom, Pattern};
    use crate::orbit::OrbitGroup;
    match pattern {
        Pattern::Guard(lit, false) => out.push(lit.as_bytes().to_vec()),
        Pattern::Atom(Atom::Literal(lit, OrbitGroup::Identity)) if !lit.is_empty() => {
            out.push(lit.as_bytes().to_vec());
        }
        // A typed atom carries no literal of its own, so what it requires is
        // read off its recognizer: `\E` over a corpus holding no `@` is
        // refused from the bytes rather than lexed to the end to say so.
        Pattern::Atom(Atom::Kind(kind)) => {
            if let Some(lit) = kind_required_literal(*kind) {
                out.push(lit.to_vec());
            }
        }
        // A positive assertion must hold at its own position, so what it requires
        // the whole pattern requires. A count whose floor is zero is the
        // exception: absence satisfies it, so it requires nothing, and treating
        // its literal as required would refuse an input that matches.
        Pattern::Assert(inner, false, look) => {
            if !look.satisfied_by_absence() {
                collect_required_literals(inner, out);
            }
        }
        Pattern::Plus(inner, _)
        | Pattern::Bind(_, _, inner)
        | Pattern::Balanced(_, inner)
        | Pattern::Field(_, inner)
        | Pattern::Atomic(inner) => collect_required_literals(inner, out),
        Pattern::Concat(v) => {
            for p in v {
                collect_required_literals(p, out);
            }
        }
        // The alternation requires only the literals that every branch requires.
        Pattern::Alt(v, _) => {
            let Some((first, rest)) = v.split_first() else { return };
            let mut common: Vec<Vec<u8>> = Vec::new();
            collect_required_literals(first, &mut common);
            for p in rest {
                let mut theirs: Vec<Vec<u8>> = Vec::new();
                collect_required_literals(p, &mut theirs);
                common.retain(|l| theirs.contains(l));
            }
            out.append(&mut common);
        }
        Pattern::Guard(_, true)
        | Pattern::Assert(_, true, _)
        | Pattern::Atom(_)
        | Pattern::Within(..)
        | Pattern::Star(..)
        | Pattern::Opt(..)
        | Pattern::Repeat(..)
        | Pattern::Empty
        | Pattern::Anchor(_) => {}
    }
}

fn collect_guard_literals(pattern: &crate::ast::Pattern, out: &mut Vec<Vec<u8>>) {
    use crate::ast::Pattern;
    match pattern {
        Pattern::Guard(lit, _) => out.push(lit.as_bytes().to_vec()),
        // A guard under a count whose floor is zero need not hold at all, so
        // its literal's absence prunes nothing and collecting it would kill
        // threads that were going to match.
        Pattern::Assert(p, _, look) => {
            if !look.satisfied_by_absence() {
                collect_guard_literals(p, out);
            }
        }
        // An edit-distance group holds single-token atoms and no guard.
        Pattern::Empty | Pattern::Atom(_) | Pattern::Within(..) | Pattern::Anchor(_) => {}
        Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) | Pattern::Bind(_, _, p) => {
            collect_guard_literals(p, out);
        }
        Pattern::Repeat(p, _, _, _)
        | Pattern::Balanced(_, p)
        | Pattern::Field(_, p)
        | Pattern::Atomic(p) => {
            collect_guard_literals(p, out);
        }
        Pattern::Concat(v) | Pattern::Alt(v, _) => {
            for p in v {
                collect_guard_literals(p, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The matches by the lexing engine and nothing else. `crate::scan` takes
    /// the byte route for the patterns under test here, so comparing a route
    /// against it would compare the route against itself.
    fn engine(p: &crate::ast::Pattern, input: &[u8]) -> Vec<crate::engine::Span> {
        crate::nfa::scan_nfa(p, input).expect("the single-pass engine takes these patterns")
    }

    const CORPUS: &[u8] =
        b"the quick brown fox jumps over the lazy dog while 192.168.0.1 logs an ERROR at noon";

    /// A run whose flags carry neither an unsettled byte nor a joining one is a
    /// settled run, which is what lets `probe_punct_at` skip the two passes
    /// `settled_run` makes over its bytes.
    ///
    /// Both halves of that function, against the table: no byte with a clear
    /// entry is unsettled, and none of them is a colon, so no `:/` can open in
    /// a run whose flags are clear.
    #[test]
    fn a_run_with_clear_flags_is_a_settled_run() {
        for c in 0..=u8::MAX {
            if RUN_BYTE[c as usize] & (RUN_UNSETTLED | RUN_JOINS) == 0 {
                assert!(settled_byte(c), "{:?} has clear flags and is not settled", c as char);
                assert_ne!(c, b':', "a colon must carry RUN_JOINS");
            }
        }
    }

    /// No word byte carries a run flag, which is what lets the scan that finds
    /// a run's bounds carry the flags of the whole run without reading the word
    /// run between them.
    ///
    /// `RunCache::run_of` walks the bytes outside `s..e` and never those inside
    /// it, and `word_run_ending_at` hands it a gap that is part of the maximal
    /// word run around the position asked about. What makes the flags it
    /// carries stand for the whole run is that every byte of such a gap ors in
    /// zero. A word byte gaining a flag would make `settles_word` read a run as
    /// settled that is not, which is a false positive rather than a refusal,
    /// and the byte tests around it would not catch it.
    #[test]
    fn a_word_byte_carries_no_run_flag() {
        for c in 0..=u8::MAX {
            if word_byte(c) {
                assert_eq!(
                    RUN_BYTE[c as usize], 0,
                    "{:?} is a word byte and carries flags {:#04b}",
                    c as char, RUN_BYTE[c as usize]
                );
            }
        }
    }

    /// A settle reading quoting as it probes reaches the same region as one
    /// reading a scan of the whole input, at every region of an input whose
    /// strings are before, around and after them.
    ///
    /// The case that decides it is a string opening before a region and closing
    /// after it. A scan bounded at the region's edge meets the open and not the
    /// close, and a reading that stopped there would call every later position
    /// quoted; the scan has to carry an open string to its close, which is what
    /// `QuoteScan::ensure` promises and what this holds it to.
    ///
    /// A refusal is compared by the report it carries, not only by whether it
    /// happened:
    /// the two readings must refuse for the same reason, which is a stronger
    /// statement than both refusing.
    #[test]
    fn a_probed_quote_scan_settles_what_a_whole_scan_settles() {
        let mut text = String::new();
        for i in 0..40u32 {
            match i % 4 {
                // A string that closes on its own line.
                0 => text.push_str(&format!("let a{i} = \"s{i}\" ;\n")),
                // A string holding a newline, so it opens on one line and closes
                // several lines later: the straddling case.
                1 => text.push_str(&format!("let b{i} = \"open{i}\nstill in it\nclose{i}\" ;\n")),
                // No quote at all.
                2 => text.push_str(&format!("let c{i} = {} ;\n", i * 37)),
                // A quote the lexer takes inside a word rather than as an opener.
                _ => text.push_str(&format!("let d{i} = don't_{i} ;\n")),
            }
        }
        let input = text.as_bytes();
        let n = input.len();
        let quotes = crate::parallel_lex::quoted_spans(input);
        let read = |r: Result<(usize, usize, Vec<crate::token::Token>), SettleReport>| match r {
            Ok((lo, hi, toks)) => Ok((lo, hi, toks.len())),
            Err(report) => Err(report.to_string()),
        };
        // Every region an eighth of the way along, plus both edges, at three
        // widths and three match lengths.
        for k in 0..=8usize {
            for width in [1usize, 9, 40] {
                let a = (n * k / 8).min(n.saturating_sub(width));
                let b = (a + width).min(n);
                for max_len in [1usize, 2, 4] {
                    let whole =
                        read(settle_window(input, &mut Quoting::Scanned(&quotes), a, b, max_len));
                    let mut scan = crate::parallel_lex::QuoteScan::new(input);
                    let probed = read(settle_window(
                        input,
                        &mut Quoting::AsProbed(&mut scan),
                        a,
                        b,
                        max_len,
                    ));
                    assert_eq!(
                        whole, probed,
                        "region {a}..{b} at max_len {max_len} over {n} bytes"
                    );
                }
            }
        }
    }

    /// A settle reads the input as far as it probes, on an input that has a
    /// bound to find.
    ///
    /// Every string here closes on its own line, so the far edge of a region is
    /// where the reading stops. Whether a bound exists at all is a property of
    /// the input and not of the reading: a string that never closes has to be
    /// carried to the input's end, because a position inside an unclosed string
    /// is not decided by knowing where the string began. That case is held for
    /// agreement by the test above and cannot be held for a bound here.
    #[test]
    fn a_probed_quote_scan_reads_no_further_than_it_probes() {
        let mut text = String::new();
        for i in 0..4000u32 {
            if i % 3 == 0 {
                text.push_str(&format!("let a{i} = \"s{i}\" ;\n"));
            } else {
                text.push_str(&format!("let c{i} = {} ;\n", i * 37));
            }
        }
        let input = text.as_bytes();
        let n = input.len();
        assert!(n > 60_000, "the input must be large enough for a bound to show ({n} bytes)");
        for (a, b) in [(0usize, 9usize), (200, 240), (1_000, 1_040)] {
            for max_len in [1usize, 2, 4] {
                let mut scan = crate::parallel_lex::QuoteScan::new(input);
                let settled =
                    settle_window(input, &mut Quoting::AsProbed(&mut scan), a, b, max_len);
                assert!(settled.is_ok(), "region {a}..{b} at max_len {max_len} did not settle");
                // Proportional to where the region is, not to how long the
                // input happens to be.
                assert!(
                    scan.searched_to() < b + 4096,
                    "region {a}..{b} at max_len {max_len} searched to {} of {n}",
                    scan.searched_to()
                );
            }
        }
    }

    /// The byte table says of every one of the 256 bytes exactly what the
    /// predicates it stands in for say. A table that disagreed about one byte
    /// would settle a run the reader must decline, or decline one it settles.
    #[test]
    fn the_run_byte_table_is_the_predicates_it_replaces() {
        for c in 0..=u8::MAX {
            let t = RUN_BYTE[c as usize];
            assert_eq!(
                t & RUN_UNSETTLED != 0,
                !settled_byte(c),
                "byte {c:#04x} settles differently"
            );
            assert_eq!(
                t & RUN_JOINS != 0,
                c == b'.' || c == b'/' || c == b':',
                "byte {c:#04x} joins differently"
            );
        }
    }

    /// Reading the registers off a match's own bytes reports what lexing a
    /// window at it reports - the same spans, under the same names, in the same
    /// order. A shape whose atoms the bytes cannot walk is refused, and the
    /// window answers it instead.
    #[test]
    fn the_registers_read_off_the_bytes_are_the_ones_a_window_resolves() {
        let mut text = String::new();
        for i in 0..300u32 {
            text.push_str(&format!("let value_{i} = {} ; call_{i}(alpha, beta) ;\n", i * 7));
        }
        let input = text.as_bytes();
        // `z` binds the first atom and `a` the second, so the name order is the
        // reverse of the positional one: a path pairing by position reports each
        // register under the other's name.
        for src in ["\\W:name \"=\"", "\"let\" \\W:v \"=\"", "\"let\" \\W \"=\"", "\\W:z \\W:a"] {
            let p = crate::parser::parse(src).expect("parses");
            let spans = crate::engine::scan(&p, input);
            assert!(!spans.is_empty(), "{src} matches the corpus");
            // Every atom of these is a literal or a word token over an ASCII
            // corpus, so the walk must take them: a refusal here would leave the
            // rest of this test asserting nothing at all.
            let (got, names) = flat_captures_by_byte_bounds(&p, input, &spans)
                .unwrap_or_else(|| panic!("the bytes walk {src}"));
            let want = captures_by_windows(&p, input, &spans).expect("a window answers these");
            assert_eq!(got.len(), want.len(), "{src}");
            assert_eq!(names.as_ref(), want[0].names(), "{src} names");
            for (g, w) in got.iter().zip(&want) {
                assert_eq!((g.span.start(), g.span.end()), (w.start, w.end), "{src} span");
                assert_eq!(&g.regs[..names.len()], w.captures(), "{src} registers");
            }
        }
        // A byte-pattern atom is a shape the walk does not read, so it refuses
        // and the window keeps it.
        let bp = crate::parser::parse("`cond_[0-9]+`:c").expect("parses");
        let bs = crate::engine::scan(&bp, b"if (cond_12) { }");
        assert!(flat_captures_by_byte_bounds(&bp, b"if (cond_12) { }", &bs).is_none());

        // The byte walk stops short of a word carrying a letter outside ASCII,
        // so it does not reconstruct the match and refuses it rather than
        // reporting a register cut in two. One such match refuses the whole
        // input, which is the same all-or-none the windows take.
        let accented = "let caf\u{e9}_x = 1 ; let plain = 2 ;\n".as_bytes();
        let p = crate::parser::parse("\"let\" \\W:v \"=\"").expect("parses");
        let spans = crate::engine::scan(&p, accented);
        assert_eq!(spans.len(), 2, "both statements match");
        assert!(
            flat_captures_by_byte_bounds(&p, accented, &spans).is_none(),
            "a word the bytes cannot walk refuses the walk"
        );
        // And the window still answers it, so nothing is lost by refusing.
        let want = captures_by_windows(&p, accented, &spans).expect("a window answers it");
        assert_eq!(want.len(), 2);
    }

    /// The word-run route reports the leftmost run of that many consecutive
    /// word tokens at or after the offset, which is what the lexed tokens say
    /// it is. Asked from every byte of each text, for runs of two and three.
    ///
    /// A refusal is not a failure and a wrong span is: the last two texts hold
    /// an unclosed quote and an address, which the bytes may decline to settle.
    #[test]
    fn the_word_run_route_reports_the_run_the_tokens_hold() {
        use crate::token::TokenKind;
        for text in [
            "",
            "a",
            "aa bb",
            "aa bb cc dd",
            "aa 1 bb cc dd",
            "aa, bb cc",
            "aa\n\nbb  cc\tdd",
            "x1 y2 z3 w4",
            "let value_0 = 0 ; call_0(alpha, beta) ;",
            "aa \"bb cc\" dd ee ff",
            "aa bb \"never closed ; cc dd",
            "aa bb 192.168.0.1 cc dd",
        ] {
            let input = text.as_bytes();
            let toks = crate::lexer::lex(input);
            let sig: Vec<&crate::token::Token> =
                toks.iter().filter(|t| t.is_significant()).collect();
            for times in [2usize, 3] {
                for at in 0..=input.len() {
                    let want = sig
                        .windows(times)
                        .find(|w| {
                            w[0].start() >= at && w.iter().all(|t| t.kind == TokenKind::Word)
                        })
                        .map(|w| (w[0].start(), w[times - 1].end()));
                    let reader = ByteReader::new(input);
                    let Some(got) = byte_route_first_word_run_reading(&reader, times, at) else {
                        continue;
                    };
                    assert_eq!(
                        got.map(|s| (s.start(), s.end())),
                        want,
                        "{text:?}, a run of {times} at or after {at}"
                    );
                }
            }
        }
    }

    fn present_literals() -> Vec<&'static [u8]> {
        vec![b"quick", b"brown fox", b"ERROR", b"192.168.0.1", b"lazy dog", b"the"]
    }

    fn absent_literals() -> Vec<&'static [u8]> {
        vec![b"ZZZZ", b"wombat", b"CRITICAL", b"10.0.0.255", b"xylophone", b"qwxz"]
    }

    #[test]
    fn only_a_bare_identity_word_literal_is_byte_routable() {
        for src in ["\"alpha\"", "\"_x9\"", "\"Cat\""] {
            let p = crate::parse(src).expect("pattern parses");
            assert!(byte_routable_literal(&p).is_some(), "{src} is one identifier token");
        }
        // Each of these has a boundary rule the word test does not describe, or
        // is more than the one atom the route answers. A digit opens a number
        // token, punctuation carries its own boundaries, and a concatenation or
        // a kind atom is not a literal search at all.
        for src in ["\"9lives\"", "\"=\"", "\"a b\"", "\\W", "\"alpha\" \\N", "(\"alpha\" | \"beta\")"]
        {
            let p = crate::parse(src).expect("pattern parses");
            assert!(byte_routable_literal(&p).is_none(), "{src} must not route");
        }
    }

    #[test]
    fn an_alternation_of_word_literals_routes_and_a_mixed_one_does_not() {
        let routes = crate::parse("(\"alpha\" | \"beta\")").expect("pattern parses");
        assert_eq!(byte_routable_literals(&routes), Some(vec!["alpha", "beta"]));
        let folded = crate::parse("(\"alpha\" | \"alpha\")").expect("pattern parses");
        assert_eq!(byte_routable_literals(&folded), Some(vec!["alpha"]));
        // One branch that is not a word literal keeps the whole pattern on the
        // lexer, since the route answers all of it or none of it.
        for src in ["(\"alpha\" | \\N)", "(\"alpha\" | \"=\")", "(\"alpha\" | \"9x\")"] {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(byte_routable_literals(&p), None, "{src} must not route");
        }
    }

    #[test]
    fn a_routed_alternation_answers_what_the_engine_answers() {
        // The merge in start order is exact only because two different word
        // literals cannot both be whole tokens at one position, so the inputs
        // put them adjacent, repeated and as prefixes of longer words.
        let p = crate::parse("(\"alpha\" | \"beta\")").expect("pattern parses");
        let inputs: &[&[u8]] = &[
            b"alpha beta",
            b"beta alpha beta",
            b"alphabet beta alpha betas",
            b"say \"alpha beta\" then beta",
            b"alpha,beta;alpha",
            b"nothing here",
        ];
        for input in inputs {
            let routed = byte_route_word_literals(&["alpha", "beta"], input);
            let engine = engine(&p, input);
            assert_eq!(routed, Some(engine), "route and engine differ on {input:?}");
        }
    }

    #[test]
    fn the_byte_route_answers_what_the_engine_answers() {
        // Every input the route accepts must give the engine's spans exactly.
        // A disagreement here is the failure this route can have that does not
        // announce itself: wrong matches rather than an error.
        let p = crate::parse("\"alpha\"").expect("pattern parses");
        let inputs: &[&[u8]] = &[
            b"alpha",
            b"say alpha now",
            b"alphabet alpha alphas",
            b"alpha, alpha; (alpha)",
            b"say \"alpha beta\" and alpha",
            b"no match here",
            b"",
            b"ALPHA alpha Alpha",
            // Inside a typed token the bytes read as a word with non-word
            // neighbors and are not one: the lexer makes a URL, an email, an
            // IP-like run its own token, and only the lexer can say so.
            b"visit http://alpha.com now",
            b"mail alpha@example.org today",
            b"see alpha.beta.gamma here",
            b"alpha:80 and alpha/2",
            // Plain punctuation either side, where the bytes alone decide.
            b"if (alpha==beta) {alpha} else [alpha]; <alpha> !alpha, alpha;",
            // A digit before the occurrence, where the lexer decides.
            b"2alpha x2alpha alpha2 _2alpha 42alpha=1 2.5alpha 0xalpha",
            // A string closing inside the occurrence's run, and one that no
            // quote closes.
            b"say \"x y\"alpha and \"z\"alpha too",
            b"\"open alpha",
            // Single-quoted strings, which hold the occurrence or close before
            // it, and the quotes that open none: a lifetime, a contraction.
            b"say 'alpha beta' and 'x'alpha and alpha",
            b"fn f<'alpha>(x: &'alpha str) -> &'alpha str",
            b"don'talpha alpha don't",
            // A letter beyond ASCII joins the word as a letter does.
            "\u{e9}alpha alpha\u{e9} alpha".as_bytes(),
            // Dots, which the bytes settle unless a JWT header, a decimal
            // measure or a version could be at hand.
            b"obj.alpha alpha.len() a.alpha.b x.alpha.y. .alpha alpha. 2.5alpha x.5alpha",
            b"eyJhbGciOi.alpha.c v1.2.3 item_1.alpha x1.alpha 1.alpha 1x.alpha 1_.alpha",
            // Colons, which the bytes settle for a word that is no hex group
            // of four, and hand to the lexer before a slash.
            b"std::alpha alpha::beta x:alpha alpha:80 ::alpha a:b:alpha 12:30alpha",
            b"http://alpha.com/x C:/dir/alpha a::alpha alpha::1",
            // Slashes, which the bytes settle unless a path or a base64 blob
            // could be at hand.
            b"dir/alpha (/usr/alpha) ./alpha a/alpha/b x=/alpha abcdefghij/alpha/ 1.2.3.4/alpha /alpha",
        ];
        for input in inputs {
            let routed = byte_route_word_literal("alpha", input);
            let engine = engine(&p, input);
            assert_eq!(routed, Some(engine), "route and engine differ on {input:?}");
        }
    }

    /// A run of printable bytes with `mid` inside it, high in entropy at every
    /// window, which the lexer takes whole as a blob.
    fn blob_around(mid: &str) -> String {
        let mut x: u32 = 0x9e37_79b9;
        let mut side = || {
            let mut s = String::new();
            for _ in 0..80 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                // Printable, and neither quote, so the run reads as a blob
                // and nothing else.
                let c = b'!' + (x >> 25) as u8 % 94;
                if !matches!(c, b'"' | b'\'') {
                    s.push(c as char);
                }
            }
            s
        };
        format!("{}{mid}{}", side(), side())
    }

    #[test]
    fn a_long_run_is_read_by_its_own_lex_and_a_blob_holds_no_token() {
        // A run of dots is long but low in entropy, so the lexer makes tokens
        // of it and the occurrence among them is a word; a run of high
        // entropy is one blob token to the lexer, whatever bytes it holds.
        // The reader reads each run for blobs on its own and answers both as
        // the engine does.
        let p = crate::parse("\"alpha\"").expect("pattern parses");
        let pad = ".".repeat(30);
        let dotted = format!("data {pad}alpha{pad} end");
        assert!(crate::lexer::blob_runs(dotted.as_bytes()).is_empty(), "dots are not a blob");
        let blob = format!("data {} end", blob_around(".alpha."));
        assert!(!crate::lexer::blob_runs(blob.as_bytes()).is_empty(), "the run must be a blob");
        let lettered = format!("{}.alpha.{}", "a".repeat(30), "b".repeat(30));
        for input in [dotted, blob, lettered] {
            let input = input.as_bytes();
            assert_eq!(
                byte_route_word_literal("alpha", input),
                Some(engine(&p, input)),
                "route and engine differ on {input:?}"
            );
        }
    }

    #[test]
    fn a_quote_inside_a_blob_or_a_char_literal_opens_no_string() {
        // The lexer takes a blob whole and a char literal `'"'` as one token,
        // so a quote inside either opens no string, and the word after it is
        // a word. A URL runs up to a quote, so the quote before `'"'` may end
        // a URL instead, which the bytes cannot settle: the route declines.
        let p = crate::parse("\"alpha\"").expect("pattern parses");
        let blob = format!("x {} alpha", blob_around("\""));
        assert!(!crate::lexer::blob_runs(blob.as_bytes()).is_empty(), "the run must be a blob");
        let inputs: Vec<Vec<u8>> = vec![
            blob.into_bytes(),
            b"x = '\"'; alpha".to_vec(),
            b"y = '\\\"'; alpha".to_vec(),
            b"z = 'a \"b\" c'; alpha".to_vec(),
        ];
        for input in &inputs {
            assert_eq!(engine(&p, input).len(), 1, "the engine sees the word on {input:?}");
            assert_eq!(byte_route_word_literal("alpha", input), Some(engine(&p, input)), "on {input:?}");
        }
        assert_eq!(byte_route_word_literal("alpha", b"see http://x/'\"' alpha"), None);
    }

    #[test]
    fn a_digit_before_the_occurrence_is_the_lexers_call() {
        // In a run of word bytes and plain punctuation the neighbors decide,
        // except after a digit: `2ms` is one duration token and `2alpha` a
        // number then a word, and only the lexer tells them apart.
        let cases: &[(&str, &[u8])] = &[
            ("ms", b"took 15ms and 3h20m then ms alone" as &[u8]),
            // A unit symbol one space after a number is that quantity's, so
            // the second `kb` is inside a token and the first is not.
            ("kb", b"2kb of 2 kb"),
            ("kg", b"5 kg and kg alone and x kg"),
            ("alpha", b"2alpha 2.5alpha 0xalpha 1_alpha"),
            // A unit after a decimal, and a version's `v1` before its dot.
            ("ms", b"2.5ms 2.ms 2 . ms x.ms x1.ms 1x.ms"),
            ("v1", b"v1.2.3 v1.x v1 v1.2 a.v1"),
            // A word of four hex digits, which a colon can turn into an
            // address group.
            ("beef", b"dead::beef x::beef beef::1 beef:cafe::1 (beef) beef:x beef beef:"),
        ];
        for (lit, input) in cases {
            let p = crate::parse(&format!("\"{lit}\"")).expect("pattern parses");
            assert_eq!(
                byte_route_word_literal(lit, input),
                Some(engine(&p, input)),
                "route and engine differ on {input:?}"
            );
        }
    }

    #[test]
    fn a_phone_number_reaching_into_the_run_hands_the_input_back() {
        // A phone number ends its last group at the first non-digit and steps
        // over a space to reach the group, so in the whole input `2671ms` is
        // the end of the number and then a word, where the run alone reads as
        // a duration. The route cannot tell from the run and declines.
        let p = crate::parse("\"ms\"").expect("pattern parses");
        let input = b"call +1 415 555 2671ms now";
        assert_eq!(engine(&p, input).len(), 1, "the engine sees the word after the number");
        assert_eq!(byte_route_word_literal("ms", input), None);
        // With no digits before the space nothing reaches in, and the run's
        // duration is the whole input's.
        let input = b"call x 2671ms now";
        assert_eq!(byte_route_word_literal("ms", input), Some(engine(&p, input)));
        assert!(engine(&p, input).is_empty(), "a duration is not the word");
    }

    #[test]
    fn a_long_literal_is_a_token_of_whatever_kind_the_lexer_makes() {
        // The engine matches a literal by text on a token of any kind, so a
        // literal shaped as a hash digest matches the digest token, and one
        // that base64 padding joins into a blob does not match there. Both
        // are as long as the runs the bytes cannot settle, so the lexer reads
        // them.
        let digest = "d41d8cd98f00b204e9800998ecf8427e";
        let p = crate::parse(&format!("\"{digest}\"")).expect("pattern parses");
        for input in [format!("x {digest} y"), format!("x.{digest}"), format!("{digest}=")] {
            let input = input.as_bytes();
            assert_eq!(engine(&p, input).len(), 1, "the digest token matches by text on {input:?}");
            assert_eq!(byte_route_word_literal(digest, input), Some(engine(&p, input)), "on {input:?}");
        }
        let p = crate::parse("\"AlphaBetaGamm0\"").expect("pattern parses");
        let input = b"AlphaBetaGamm0== and AlphaBetaGamm0 here";
        assert_eq!(engine(&p, input).len(), 1, "the padded one is a base64 token");
        assert_eq!(byte_route_word_literal("AlphaBetaGamm0", input), Some(engine(&p, input)));
    }

    #[test]
    fn a_word_then_plain_punctuation_is_byte_routable() {
        for (src, byte) in [("\\W \"=\"", b'='), ("\\W \"(\"", b'('), ("\\W \";\"", b';')] {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(byte_routable_word_then_punct(&p), Some(byte), "{src}");
        }
        // The word comes first, the literal is one byte of plain punctuation,
        // and nothing else is in the pattern. A dot joins typed tokens, so it
        // is not plain.
        for src in ["\"=\" \\W", "\\N \"=\"", "\\W \"==\"", "\\W \".\"", "\\W \"=\" \\W", "\\W"] {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(byte_routable_word_then_punct(&p), None, "{src} must not route");
        }
    }

    #[test]
    fn the_word_then_punctuation_route_answers_what_the_engine_answers() {
        // Each `=` is its own token or not, and the token before it is a
        // word or not; every combination the bytes settle is here, with the
        // number, the duration, the string, the bracket and the base64
        // padding that make a byte before `=` no word and a `=` no token.
        let p = crate::parse("\\W \"=\"").expect("pattern parses");
        let inputs: &[&[u8]] = &[
            b"a = b == c; x=1; y =2; (z) = 3; \"q\" = 4; 42 = x; 2alpha = 1; 2ms = 5",
            b"AlphaBetaGamm0== x = 1",
            b"fetch(https://example.com/a?b=1) ; c = 2",
            b"x==y ==x x= =y",
            b"\"open = 1",
            b"= a =\n\tb\t=\r\n",
            b"",
            b"if (alpha==beta) {alpha} else [alpha]; <alpha> !alpha, alpha;",
        ];
        for input in inputs {
            assert_eq!(
                byte_route_word_then_punct(b'=', input),
                Some(engine(&p, input)),
                "route and engine differ on {input:?}"
            );
        }
    }

    #[test]
    fn the_word_then_punctuation_route_declines_what_the_bytes_cannot_settle() {
        // A phone number reaching into the word's run, a letter beyond ASCII
        // ending the word, and whitespace beyond ASCII before the byte: each
        // has a match the bytes cannot see, and the route hands the input to
        // the lexer rather than miss it. A long run is read by its own lex.
        let p = crate::parse("\\W \"=\"").expect("pattern parses");
        let phone = b"+1 415 555 2671ms = 3";
        assert_eq!(engine(&p, phone).len(), 1, "the word after the number then `=`");
        assert_eq!(byte_route_word_then_punct(b'=', phone), None);
        let accented = "caf\u{e9} = 1".as_bytes();
        assert_eq!(engine(&p, accented).len(), 1, "the accented word then `=`");
        assert_eq!(byte_route_word_then_punct(b'=', accented), None);
        let nbsp = "foo\u{a0}= 1".as_bytes();
        assert_eq!(engine(&p, nbsp).len(), 1, "the word, unicode whitespace, then `=`");
        assert_eq!(byte_route_word_then_punct(b'=', nbsp), None);
        let long = format!("{}=1", "x".repeat(60));
        assert_eq!(engine(&p, long.as_bytes()).len(), 1, "the long word then `=`");
        assert_eq!(byte_route_word_then_punct(b'=', long.as_bytes()), Some(engine(&p, long.as_bytes())));
        // Sixty hex letters are a hex run rather than a word, which only the
        // lex of the run says.
        let hex = format!("{}=1", "a".repeat(60));
        assert!(engine(&p, hex.as_bytes()).is_empty(), "a hex run then `=`");
        assert_eq!(byte_route_word_then_punct(b'=', hex.as_bytes()), Some(Vec::new()));
    }

    #[test]
    fn a_byte_pattern_with_a_literal_word_prefix_is_byte_routable() {
        let p = crate::parse("`cond_[0-9]+`").expect("pattern parses");
        let (_, prefix) = byte_routable_byte_pattern(&p).expect("opens with literal bytes");
        assert_eq!(prefix, b"cond_");
        let p = crate::parse("`x`").expect("pattern parses");
        assert!(byte_routable_byte_pattern(&p).is_some());
        // No literal to search for, a prefix opening with punctuation, and
        // more than the one atom.
        for src in ["`[0-9]+`", "`=x`", "`cond_[0-9]+` \\N"] {
            let p = crate::parse(src).expect("pattern parses");
            assert!(byte_routable_byte_pattern(&p).is_none(), "{src} must not route");
        }
    }

    #[test]
    fn the_byte_pattern_route_answers_what_the_engine_answers() {
        let src = "`cond_[0-9]+`";
        let p = crate::parse(src).expect("pattern parses");
        let (bp, prefix) = byte_routable_byte_pattern(&p).expect("routable");
        let inputs: &[&[u8]] = &[
            b"cond_1 cond_22 cond_x xcond_3 2cond_4 cond_5.x cond_6:7 (cond_8) \"cond_9\" cond_",
            b"if (cond_10) { do_1(cond_11) ; } cond_cond_12 _cond_13 cond_14_ cond_15",
            b"cond_16@example.org http://cond_17.com cond_18/19 cond_20",
            b"",
            b"cond_",
        ];
        for input in inputs {
            let engine = crate::nfa::scan_nfa(&p, input).expect("the single-pass engine takes this pattern");
            assert_eq!(byte_route_byte_pattern(bp, &prefix, input), Some(engine), "route and engine differ on {input:?}");
        }
    }

    #[test]
    fn the_byte_pattern_route_declines_what_the_bytes_cannot_settle() {
        // A phone number reaching into the run before the token. A long run
        // is read by its own lex, and a blob holds no token.
        let p = crate::parse("`ms`").expect("pattern parses");
        let (bp, prefix) = byte_routable_byte_pattern(&p).expect("routable");
        let phone = b"call +1 415 555 2671ms now";
        assert_eq!(crate::nfa::scan_nfa(&p, phone).expect("engine").len(), 1);
        assert_eq!(byte_route_byte_pattern(bp, &prefix, phone), None);
        let long = format!("{}.ms.{}", "a".repeat(30), "b".repeat(30));
        let blob = format!("x {} ms", blob_around(".ms."));
        for input in [long, blob] {
            let input = input.as_bytes();
            let engine = crate::nfa::scan_nfa(&p, input).expect("engine");
            assert_eq!(byte_route_byte_pattern(bp, &prefix, input), Some(engine), "on {input:?}");
        }
    }

    #[test]
    fn the_word_boundary_rule_alone_does_not_decide_a_token() {
        // What a byte route would have to reproduce, pinned rather than
        // assumed. Finding the bytes with non-identifier neighbors is the
        // right rule only where the lexer is making word tokens; a quoted
        // string is one token whatever its interior looks like, so a literal
        // inside one is not a word token and does not match. A route that
        // tested boundaries alone would answer this input differently from
        // the engine, silently.
        let p = crate::parse("\"alpha\"").expect("pattern parses");
        assert_eq!(engine(&p, b"say alpha now").len(), 1, "a bare word is its own token");
        assert_eq!(
            engine(&p, b"say \"alpha beta\" now").len(),
            0,
            "inside a quoted string there is no word token to match"
        );
    }

    #[test]
    fn the_windows_a_required_literal_opens_answer_what_the_whole_scan_answers() {
        // The route must give the engine's spans exactly, on inputs built so
        // the windowing's own failures would show: a hit at the very start and
        // at the very end, hits close enough that their windows merge, hits
        // far enough apart that they do not, a match spanning a line, and a
        // pattern whose matches are longer than one token so an edge can cut
        // one short.
        // The literal is rare enough, and the input long enough, that the
        // windows are worth their cost: the route must answer these rather
        // than hand them back, or the comparison below compares nothing.
        let mut text = String::new();
        text.push_str("alpha = 1 ;\n");
        for i in 0..40_000u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i % 8_000 == 0 {
                text.push_str("alpha = 2 ;\nalpha = 3 ;\n");
            }
            if i % 13_997 == 0 {
                text.push_str("key : alpha\n= 4 ;\n");
            }
        }
        text.push_str("alpha = 5 ;\n");
        let input = text.as_bytes();
        for src in ["\"alpha\" \"=\"", "\"alpha\" \"=\" \\N", "\"alpha\"", "\"alpha\"{1,2}"] {
            let p = crate::parse(src).expect("pattern parses");
            let want = engine(&p, input);
            let got = scan_required_windows(&p, input);
            assert_eq!(got.as_ref(), Some(&want), "{src}");
            assert_eq!(crate::scan(&p, input), want, "{src} through the scan's own route");
        }
        // A guard's literal is the only one here and it is on every line, so
        // this one is handed back; the scan must still answer it.
        let p = crate::parse("\\W \"=\" \\N ~\"alpha\"").expect("pattern parses");
        let want = engine(&p, input);
        assert_eq!(scan_required_windows(&p, input), None, "a dense guard literal declines");
        assert_eq!(crate::scan(&p, input), want, "and the scan answers it anyway");
    }

    #[test]
    fn the_first_window_holding_a_match_holds_the_leftmost_one() {
        // A caller wanting one match stops at the first window that holds one;
        // a caller wanting all of them lexes every window. The two must name
        // the same match, or one rung of the ladder answers one question two
        // ways.
        //
        // Three corpora, each of a kind where the early exit could disagree
        // with the full route.
        // Hits far enough apart that their windows do not merge, so a later
        // window could give a wrong answer. An input holding the literal that
        // the pattern never matches, so every window is lexed and the answer
        // is still no match. And tokens longer than the 512 bytes a two-token
        // window reaches, which puts a match's start outside the window its
        // own literal opened - the one case the leftmost argument does not
        // cover, and the reason this is a test and not a comment.
        let mut spread = String::new();
        spread.push_str("alpha = 1 ;\n");
        for i in 0..40_000u32 {
            spread.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i == 20_000 {
                spread.push_str("alpha = 2 ;\n");
            }
        }
        spread.push_str("alpha = 3 ;\n");

        let mut never = String::new();
        for i in 0..40_000u32 {
            never.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i % 10_000 == 0 {
                never.push_str("alpha ;\n");
            }
        }

        let mut blob = String::new();
        for i in 0..40_000u32 {
            blob.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i % 2_700 == 0 {
                blob.push_str(&format!("{} alpha = {i} ;\n", "n".repeat(700)));
            }
        }

        let mut answered = 0usize;
        for (label, text) in [("spread", &spread), ("never", &never), ("blob", &blob)] {
            let input = text.as_bytes();
            for src in ["\"alpha\" \"=\"", "\"alpha\" \"=\" \\N", "\\W \"alpha\""] {
                let p = crate::parse(src).expect("pattern parses");
                let Some(got) = first_required_window(&p, input) else {
                    continue;
                };
                answered += 1;
                let want = engine(&p, input).first().copied();
                assert_eq!(got, want, "{label} / {src}: the early exit against the engine");
                assert_eq!(
                    got,
                    scan_required_windows(&p, input).and_then(|s| s.first().copied()),
                    "{label} / {src}: the early exit against the full route"
                );
            }
        }
        assert!(answered > 0, "no case reached the route, so this test checked nothing");
    }

    /// The splits `cuts_at` finds, one byte at a time, so the walks that step
    /// between newlines are held to the byte walk.
    fn every_split(input: &[u8], quotes: Option<&[(usize, usize)]>) -> Vec<usize> {
        (0..=input.len()).filter(|&i| cuts_at(input, quotes, i)).collect()
    }

    #[test]
    fn the_split_walks_find_the_split_the_byte_walk_finds() {
        // Strings carried across a line by a backslash, indented lines, blank
        // lines, a char literal and a tail with no newline: a candidate is
        // refused for each reason there is, with and without the strings read.
        let mut text = String::new();
        for i in 0..400u32 {
            match i % 7 {
                0 => text.push_str(&format!("let s_{i} = \"open {i} \\\nx = 1 ; still in the string\" ;\n")),
                1 => text.push_str("    indented = 1 ;\n"),
                2 => text.push_str("\n\n"),
                3 => text.push_str(&format!("x_{i} = {i} ; y = 'c' ;\n")),
                _ => text.push_str(&format!("let v_{i} = {} ;\n", i * 3)),
            }
        }
        text.push_str("tail with no newline");
        let input = text.as_bytes();
        let n = input.len();
        let quotes = crate::parallel_lex::quoted_spans(input);
        for q in [None, Some(quotes.as_slice())] {
            let splits = every_split(input, q);
            assert!(splits.len() > 100, "the corpus holds {} splits, too few to walk", splits.len());
            for at in (0..=n).step_by(7) {
                for floor in [0usize, at / 2, at.saturating_sub(40)] {
                    let want = splits.iter().rev().find(|&&s| s > floor && s <= at).copied();
                    assert_eq!(cut_back(input, q, at, floor), want, "back from {at} over {floor}");
                }
                for limit in [n, at + 30, (at + n) / 2] {
                    let want = (at..=limit.min(n)).find(|&i| i == n || splits.binary_search(&i).is_ok());
                    assert_eq!(cut_forward(input, q, at, limit), want, "forward from {at} to {limit}");
                }
            }
        }
        let bare = every_split(input, None);
        let read = every_split(input, Some(&quotes));
        assert!(read.iter().all(|s| bare.binary_search(s).is_ok()), "a split read with the strings is one without");
        assert!(read.len() < bare.len(), "no string hid a split, so the two readings were not told apart");
    }

    /// Hits with lines of one long token between them, at two spacings. Four
    /// such lines put the spans the two windows first settle to end to end,
    /// and the gather joins them; five leave a line between, and only a
    /// widening reaches across. The filler settles at the first try.
    fn corpus_with_joining_spans() -> Vec<u8> {
        let mut text = String::new();
        for i in 0..30_000u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i % 6_000 == 3_000 {
                let runs = if i % 12_000 == 3_000 { 4 } else { 5 };
                text.push_str("alpha = 1 ;\n");
                for k in 0..runs {
                    text.push_str(&"nmopq"[k..=k].repeat(300));
                    text.push('\n');
                }
                text.push_str("alpha = 2 ;\n");
            }
        }
        text.into_bytes()
    }

    #[test]
    fn gathered_spans_are_apart_lexed_once_and_lex_as_the_whole_input_does() {
        let input = corpus_with_joining_spans();
        let input = input.as_slice();
        let p = crate::parse("\"alpha\" \"=\"").expect("pattern parses");
        let max_len = crate::nfa::bounded_max_len(&p).expect("two tokens");
        let mut lits: Vec<Vec<u8>> = Vec::new();
        collect_contained_literals(&p, &mut lits);
        let reach = max_len * WINDOW_BYTES_PER_TOKEN;
        let rough = windows_under_limit(input, &mut lits, reach, WindowGate::Open).expect("the gate is open");
        assert_eq!(rough.len(), 10, "each hit opens a window of its own");
        let quotes = crate::parallel_lex::quoted_spans(input);
        let gathered = gather_windows(input, Some(&quotes), &rough, usize::MAX).expect("no budget to pass");
        // Every span ends on splits, holds its windows, and is separate from
        // the next; every window is in one.
        for g in &gathered {
            assert!(g.lo == 0 || cuts_at(input, Some(&quotes), g.lo), "{g:?} opens off a split");
            assert!(g.hi == input.len() || cuts_at(input, Some(&quotes), g.hi), "{g:?} closes off a split");
            assert!(g.lo <= g.a && g.b <= g.hi, "{g:?} does not hold its windows");
        }
        for w in gathered.windows(2) {
            assert!(w[0].hi < w[1].lo, "spans touch or overlap: {w:?}");
        }
        for &(a, b) in &rough {
            assert!(gathered.iter().any(|g| g.lo <= a && b <= g.hi), "window {a}..{b} is in no span");
        }
        assert_eq!(gathered.len(), 7, "the gather joins the three pairs whose spans meet");

        // Settling hands on separate spans, each lexed once, whose
        // tokens are the whole input's over the same bytes.
        let key = |t: &crate::token::Token| (t.kind, t.start, t.end);
        let whole: Vec<_> = crate::parallel_lex::lex_parallel(input).iter().map(key).collect();
        let mut handed: Vec<(usize, usize)> = Vec::new();
        let settled = settle_gathered(input, &quotes, &gathered, max_len, usize::MAX, |lo, hi, toks| {
            handed.push((lo, hi));
            let mine: Vec<_> = toks.iter().map(key).collect();
            let theirs: Vec<_> =
                whole.iter().filter(|t| t.1 as usize >= lo && t.2 as usize <= hi).copied().collect();
            assert_eq!(mine, theirs, "span {lo}..{hi} lexes apart from the whole input");
            false
        });
        let settled = match settled {
            Ok(s) => s,
            Err(Unsettled::Widening(report)) => panic!("a span would not settle: {report}"),
            Err(Unsettled::Budget(bytes)) => panic!("no budget was set and {bytes} bytes passed it"),
        };
        assert_eq!(handed.len(), 5, "the two pairs a line apart join by widening, the rest stay separate");
        for w in handed.windows(2) {
            assert!(w[0].1 < w[1].0, "handed spans touch or overlap: {w:?}");
        }
        assert_eq!(settled.spans, handed.len());
        assert_eq!(settled.bytes, handed.iter().map(|&(lo, hi)| hi - lo).sum::<usize>());
        assert_eq!(settled.lexed, settled.bytes, "a byte was lexed twice, or lexed and not handed on");

        // And the route answers what the engine answers over it, gated and not.
        let want = engine(&p, input);
        assert_eq!(want.len(), 10);
        assert_eq!(scan_required_windows_ungated(&p, input), Some(want.clone()));
        assert_eq!(scan_required_windows(&p, input), Some(want.clone()));
        assert_eq!(crate::scan(&p, input), want, "through the scan's own route");
    }

    #[test]
    fn a_gather_reading_no_strings_is_inside_the_one_that_does() {
        // Strings carried across a line by a backslash hide splits from the
        // settling; without them every candidate is a split, so the spans can
        // only be narrower, and a total past the budget without the strings
        // is past it with them.
        let mut text = String::new();
        for i in 0..20_000u32 {
            text.push_str(&format!("let value_{i} = \"open {i} \\\nx = 1 ; still in the string\" ;\n"));
            if i % 3_000 == 1_500 {
                text.push_str("alpha = 2 ;\n");
            }
        }
        let input = text.as_bytes();
        let p = crate::parse("\"alpha\" \"=\"").expect("pattern parses");
        let max_len = crate::nfa::bounded_max_len(&p).expect("two tokens");
        let mut lits: Vec<Vec<u8>> = Vec::new();
        collect_contained_literals(&p, &mut lits);
        let rough = windows_under_limit(input, &mut lits, max_len * WINDOW_BYTES_PER_TOKEN, WindowGate::Open)
            .expect("the gate is open");
        let quotes = crate::parallel_lex::quoted_spans(input);
        let bare = gather_windows(input, None, &rough, usize::MAX).expect("no budget to pass");
        let read = gather_windows(input, Some(&quotes), &rough, usize::MAX).expect("no budget to pass");
        for g in &bare {
            assert!(read.iter().any(|r| r.lo <= g.lo && g.hi <= r.hi), "{g:?} is in no span read with the strings");
        }
        let total = |gs: &[Gathered]| gs.iter().map(|g| g.hi - g.lo).sum::<usize>();
        assert!(total(&bare) < total(&read), "the strings hid no split, so this checked nothing");
        for budget in [total(&bare) / 4, total(&bare) - 1, total(&bare), total(&read) - 1, total(&read)] {
            if gather_windows(input, None, &rough, budget).is_none() {
                assert!(gather_windows(input, Some(&quotes), &rough, budget).is_none(), "budget {budget}");
            }
        }
        assert!(gather_windows(input, None, &rough, total(&bare) - 1).is_none(), "a byte under its total declines");
        assert!(gather_windows(input, Some(&quotes), &rough, total(&read)).is_some(), "its total is within budget");
    }

    /// A deterministic run of `n` base64-alphabet bytes, high in entropy.
    fn dense_run(n: usize) -> String {
        const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut x = 0x2545_f491_4f6c_dd1du64;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                ALPHA[(x >> 58) as usize] as char
            })
            .collect()
    }

    #[test]
    fn a_window_holding_a_blob_line_lexes_as_the_whole_input_does() {
        // Lines of statements, one line whose run the entropy pass reads as a
        // blob, then lines holding URLs. A window read as one whitespace-free
        // span would be qualified whole by that one run and the URLs after it
        // would vanish into a single token.
        let run = dense_run(200);
        let mut text = String::new();
        for i in 0..40u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
        }
        text.push_str(&format!("let blob = {run} ;\n"));
        for i in 0..40u32 {
            text.push_str(&format!("see https://example.org/page/{i} for value_{i} ;\n"));
        }
        let input = text.as_bytes();
        let key = |t: &crate::token::Token| (t.kind, t.start, t.end);
        let whole: Vec<_> = crate::parallel_lex::lex_parallel(input).iter().map(key).collect();
        let window: Vec<_> = lex_window(input, 0, input.len()).iter().map(key).collect();
        assert_eq!(window, whole, "the window's lex differs from the whole input's");
        assert_eq!(window.iter().filter(|t| t.0 == crate::token::TokenKind::Url).count(), 40);
        let at = text.find(&run).expect("the run is in the text");
        assert!(
            window.iter().any(|t| t.1 as usize == at && t.2 as usize == at + run.len()),
            "the blob line's run is one token"
        );
        // A window cut inside the input, at the splits either side of the
        // blob line, lexes as the whole input does over the same bytes.
        let lo = text.find("let value_20").expect("the line is there");
        let hi = text.find("see https://example.org/page/20").expect("the line is there");
        let inner: Vec<_> = lex_window(input, lo, hi).iter().map(key).collect();
        let theirs: Vec<_> = whole.iter().filter(|t| t.1 as usize >= lo && t.2 as usize <= hi).copied().collect();
        assert_eq!(inner, theirs, "the inner window's lex differs from the whole input's");
    }

    #[test]
    fn a_typed_atom_scans_as_the_engine_does_where_the_windows_take_it() {
        // Statements with a few emails, URLs and money amounts spread through
        // them: each atom's required literal opens a window at each, the gate
        // takes them, and the scan, whose ladder tries the windows before the
        // whole-input kind route, must answer what the engine answers.
        let mut text = String::new();
        for i in 0..40_000u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i % 5_000 == 1_000 {
                text.push_str(&format!("mail ops{i}@example.org now\n"));
            }
            if i % 7_000 == 2_000 {
                text.push_str(&format!("see https://example.org/{i} today\n"));
            }
            if i % 9_000 == 3_000 {
                text.push_str(&format!("paid $1,{i:03}.50 here\n"));
            }
        }
        let input = text.as_bytes();
        for src in ["\\E", "\\U", "\\$"] {
            let p = crate::parse(src).expect("pattern parses");
            let want = engine(&p, input);
            assert!(!want.is_empty(), "{src} matches nothing, so this tests nothing");
            assert!(scan_required_windows(&p, input).is_some(), "{src}: the gate declines, so the ladder change is not exercised");
            assert_eq!(crate::scan(&p, input), want, "{src} through the scan's own ladder");
        }
    }

    #[test]
    fn the_route_declines_windows_that_settle_past_the_budget() {
        // Every line is indented, so the lexer has no split to cut at and every
        // window settles to the whole input.
        let mut text = String::new();
        for i in 0..40_000u32 {
            text.push_str(&format!("  let value_{i} = {} ;\n", i * 7));
            if i % 9_000 == 0 {
                text.push_str("  alpha = 2 ;\n");
            }
        }
        let input = text.as_bytes();
        let p = crate::parse("\"alpha\" \"=\" \\N").expect("pattern parses");
        let want = engine(&p, input);
        assert_eq!(want.len(), 5);
        assert_eq!(scan_required_windows(&p, input), None, "the settled spans are the input");
        let why = required_window_reason(&p, input);
        assert!(why.starts_with("settled past the budget"), "the reason is `{why}`");
        assert!(matches!(shortest_end_in_windows(&p, input), Err(WindowRefusal::TooDense)));
        assert_eq!(crate::scan(&p, input), want, "the scan answers it anyway");
        // With no budget the one span is the input, lexed once, and it answers.
        assert_eq!(scan_required_windows_ungated(&p, input), Some(want));
    }

    #[test]
    fn asking_whether_a_kind_matches_agrees_with_the_scan() {
        // is_match carries its own ladder, so a route reaching find says
        // nothing about whether it reaches this. Inputs that match, that hold
        // the kind only inside a token the lexer names something else, and
        // that hold nothing at all.
        for text in [
            "let value_0 = 41 ;\n",
            "only_ident_0 and_1 no_number\n",
            "1.2.3.4 and 512KB and $5\n",
            "\"42 quoted\"\n",
            "!@#$%^&*\n",
            "",
            "7",
            "word",
        ] {
            let input = text.as_bytes();
            for src in ["\\W", "\\N"] {
                let p = crate::parse(src).expect("pattern parses");
                let want = !engine(&p, input).is_empty();
                assert_eq!(crate::engine::is_match(&p, input), want, "{src} on {text:?}");
            }
        }
    }

    #[test]
    fn the_kind_routes_answer_an_anchored_ask_as_the_engine_does() {
        // find_at over every offset in the input against the engine's own
        // answer for that offset. An offset landing inside a token, on its last
        // byte, on whitespace, and one past the end are all covered by walking
        // every one rather than choosing a few.
        let text = "let value_0 = 41 ; call_1(alpha, 7) ; x9 y8 = 512KB 3.14% ;\n";
        let input = text.as_bytes();
        let toks = crate::parallel_lex::lex_parallel(input);
        for src in ["\\W", "\\N"] {
            let p = crate::parse(src).expect("pattern parses");
            let all = crate::nfa::scan_nfa_over_serial(&p, input, &toks)
                .expect("the single-pass engine takes a kind atom");
            for at in 0..=input.len() {
                let want = all.iter().copied().find(|s| s.start() >= at);
                assert_eq!(crate::cursor::find_at(&p, input, at), want, "{src} at {at}");
            }
        }
    }

    #[test]
    fn the_word_route_agrees_with_the_lexer_on_every_typed_near_miss() {
        // A letter run is a Word only once the typed recognizers decline, and
        // those open on a letter too: an email, a URL, a JWT, a hostname, a
        // version behind a `v`, a byte size behind its digits. Each is here
        // with a plain word, a word behind an identifier holding digits, a
        // quoted one, and an input holding no letter at all.
        for text in [
            "value = 42 ;\n",
            "a@b.com sent it\n",
            "see http://x.com/p now\n",
            "host api.example.com up\n",
            "ver v1.2.3 ok\n",
            "size 512KB free\n",
            "id 550e8400-e29b-41d4-a716-446655440000\n",
            "\"quoted word inside\" then tail\n",
            "12345 then word\n",
            "value_0 = alpha ;\n",
            "  indented word\n",
            "1234567890\n",
            "!@#$%^&*()\n",
            "word\n",
            "3.14% done\n",
        ] {
            let input = text.as_bytes();
            let p = crate::parse("\\W").expect("pattern parses");
            let want = engine(&p, input).into_iter().next();
            assert_eq!(crate::cursor::find(&p, input), want, "{text:?}");
        }
    }

    #[test]
    fn the_number_route_agrees_with_the_lexer_on_every_typed_near_miss() {
        // A digit run is a Number only once every typed recognizer has
        // declined, and those reach outside the run: Money opens before it,
        // ByteSize and Percent and Duration close after it, Quantity opens at
        // a sign before it and closes on a symbol after it or one space
        // after it, Ip and Version and Timestamp and Mac and Uuid join across
        // punctuation, and CreditCard joins four-digit groups across single
        // spaces. Each of those is here, with a plain number, a quoted one
        // and an input holding none.
        for text in [
            "value = 42 ;\n",
            "ip 1.2.3.4 here\n",
            "at 12:30:45 today\n",
            "ver 1.2.3 ok\n",
            "card 4111 1111 1111 1111 done\n",
            "size 512KB free\n",
            "pct 3.14% done\n",
            "cost $5 each\n",
            "took 1500ms\n",
            "mass 5 kg here\n",
            "mass 5kg here\n",
            "temp -40\u{b0}C now\n",
            "temp 20\u{b0}C now\n",
            "clock 3.2 GHz\n",
            "share 40 % done\n",
            "count 5 items\n",
            "range 10-20kg\n",
            "three 3 in a row\n",
            "uuid 550e8400-e29b-41d4-a716-446655440000\n",
            "date 2026-06-16\n",
            "mac 01:23:45:67:89:ab\n",
            "\"quoted 42 inside\" then 7\n",
            "no digits at all here\n",
            "7\n",
            "a 7\n",
            // The corpus's first digit is inside an identifier, where no token
            // starts, rather than at a position the bytes cannot settle.
            // Stepping over it must reach the number after the equals, and
            // refusing there would decline the whole input.
            "let value_0 = 41 ;\n",
            "let a1b2c3 = 7 ;\n",
            "x9 y8 z7 = 5 ;\n",
            "only_ident_0 and_1 no_number_here\n",
        ] {
            let input = text.as_bytes();
            let p = crate::parse("\\N").expect("pattern parses");
            let want = engine(&p, input).into_iter().next();
            assert_eq!(crate::cursor::find(&p, input), want, "{text:?}");
        }
    }

    #[test]
    fn a_line_anchored_literal_reads_from_the_bytes_as_the_engine_reads_it() {
        // The four places an occurrence of "let" can be: leading its line,
        // leading it behind indentation, inside a longer word, and on a line
        // some other token leads. Only the first two are matches, and an
        // indented one is the case a test for a newline directly before the
        // occurrence would wrongly refuse.
        let mut text = String::new();
        text.push_str("let a = 1 ;\n");
        for i in 0..2_000u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            text.push_str(&format!("    let indented_{i} = {i} ;\n"));
            text.push_str(&format!("\tlet tabbed_{i} = {i} ;\n"));
            text.push_str(&format!("call_{i}(let_not_a_token, {i}) ;\n"));
            text.push_str(&format!("x_{i} = let ;\n"));
        }
        let input = text.as_bytes();
        let p = crate::parse("^ \"let\"").expect("pattern parses");
        let want = engine(&p, input);
        assert!(!want.is_empty(), "nothing matched, so this test checked nothing");
        assert_eq!(crate::scan(&p, input), want, "the route against the engine");
        // The route must be the one answering, or this compares the engine
        // with itself and would pass with no route at all.
        assert_eq!(
            crate::engine::routed_spans_public(&p, input).as_ref(),
            Some(&want),
            "the byte route must answer a line-anchored literal"
        );
    }

    #[test]
    fn the_windows_soonest_end_is_the_whole_lexs_soonest_end() {
        // shortest_match asks where a match is first known to have occurred,
        // which is not the end of any selected match: `\W{1,2}` over three
        // words selects the first two and then the third, and the soonest end
        // is after the first. The windows must answer what the whole lex
        // answers, or a routed caller and an unrouted one read different ends
        // out of one input.
        let mut text = String::new();
        text.push_str("alpha beta gamma ;\n");
        for i in 0..40_000u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i % 9_000 == 0 {
                text.push_str("alpha = 41 ;\nalpha beta ;\n");
            }
        }
        let input = text.as_bytes();
        let toks = crate::parallel_lex::lex_parallel(input);
        let mut answered = 0usize;
        for src in ["\"alpha\" \\W{1,2}", "\"alpha\" \"=\"", "\"alpha\" \"=\" \\N", "\"alpha\" \\W"] {
            let p = crate::parse(src).expect("pattern parses");
            let want = crate::nfa::shortest_end(&p, input, &toks, 0);
            let got = match shortest_end_in_windows(&p, input) {
                Ok(end) => end,
                Err(why) => {
                    // A declined pattern is not a failure here - this asserts
                    // the windows agree with the whole lex wherever they
                    // answer - but the reason is printed so that a run where
                    // every pattern declines says why, rather than only that
                    // the assertion below checked nothing.
                    eprintln!("{src}: the windows declined: {why}");
                    continue;
                }
            };
            answered += 1;
            assert_eq!(got, want, "{src}: the windows against the whole lex");
        }
        assert!(answered > 0, "no pattern reached the route, so this test checked nothing");
    }

    #[test]
    fn one_span_resolves_over_its_region_as_it_does_over_a_whole_lex() {
        // captures on a single span resolves over the tokens around it and on
        // several over a lex of the input. The two must bind the same
        // registers, or a caller wanting one match reads different values out
        // of it than a scan does out of the same match.
        let mut text = String::new();
        for i in 0..40_000u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i % 9_000 == 0 {
                text.push_str("alpha = 41 ;\n");
            }
        }
        let input = text.as_bytes();
        for src in ["\\W:name \"=\"", "\"alpha\" \"=\" \\N:num", "\"let\" \\W:v \"=\""] {
            let p = crate::parse(src).expect("pattern parses");
            let spans = crate::scan(&p, input);
            assert!(!spans.is_empty(), "{src} matches nothing, so this tests nothing");
            let whole = crate::engine::captures(&p, input, &spans);
            assert_eq!(whole.len(), spans.len(), "{src}: one match per span");
            // The first, one in the middle and the last: a region at the input's
            // start has no tokens before it to settle onto and one at its end
            // none after, which are the two edges the widening treats separately.
            for i in [0, spans.len() / 2, spans.len() - 1] {
                let one = crate::engine::captures(&p, input, &[spans[i]]);
                assert_eq!(one.len(), 1, "{src}: one span must give one match");
                assert_eq!(one[0], whole[i], "{src}: span {i} resolved differently alone");
            }
        }
    }

    #[test]
    fn a_dense_literal_gives_way_to_a_rarer_one_in_the_same_pattern() {
        // Both literals are inside every match, so either anchors the windows
        // soundly. The longer one is on every line and its windows cover the
        // input; the shorter is in one line only. The route must reach the
        // second and answer, not stop at the first and hand the input back.
        let mut text = String::new();
        for i in 0..40_000u32 {
            text.push_str(&format!("value = {} ;\n", i * 7));
            if i == 20_000 {
                text.push_str("value @ 41 ;\n");
            }
        }
        let input = text.as_bytes();
        let p = crate::parse("\"value\" \"@\" \\N").expect("pattern parses");
        let want = engine(&p, input);
        assert_eq!(want.len(), 1, "the corpus holds the match once");
        let got = scan_required_windows(&p, input);
        assert_eq!(got.as_ref(), Some(&want), "the rarer literal anchors the windows");
        assert_eq!(crate::scan(&p, input), want, "and through the scan's own route");
    }

    #[test]
    fn asking_only_whether_a_match_exists_agrees_with_the_scan() {
        // The short circuit must answer what the scan answers on every route
        // it can take: refused before the lex, answered from the first window,
        // and handed to the scan. The corpus is long enough that the window
        // route is worth taking on a rare literal and gives up on a dense one.
        let mut text = String::new();
        text.push_str("alpha = 1 ;\n");
        for i in 0..40_000u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i % 9_000 == 0 {
                text.push_str("alpha = 2 ;\n");
            }
        }
        let input = text.as_bytes();
        for src in [
            // One literal word, which the byte route answers from the first
            // occurrence it confirms rather than from all of them.
            "\"alpha\"",
            "(\"alpha\" | \"beta\")",
            "\"zzzqqq\"",
            // Present, and the windows are worth taking.
            "\"alpha\" \"=\" \\N",
            "\"alpha\"{1,2}",
            // Absent, so refused from the bytes before any lex.
            "\"zzzqqq\" \"=\"",
            // Present as a literal but with no match, so the windows are lexed
            // and answer nothing.
            "\"alpha\" \";\" \\N",
            // On every line, so the route hands it back and the scan answers.
            "\"value_1\" \"=\"",
            // A byte pattern, answered from the first occurrence of its
            // literal prefix that opens a token it matches whole.
            "`value_[0-9]+`",
            "`zzz_[0-9]+`",
            // A word token then one byte of punctuation, which its own byte
            // route answers from the first occurrence that is a match.
            "\\W \"=\"",
            "\\W \";\"",
            "\\W \"@\"",
            // No contained literal at all, so only the prefix probe or the
            // scan itself can answer these.
            "\\W \"=\" \\N",
            "\\N",
            "\\W",
            "\\N \\N \\N \\N",
            "\\W \"~\" \\N",
        ] {
            let p = crate::parse(src).expect("pattern parses");
            let want = !crate::scan(&p, input).is_empty();
            assert_eq!(crate::engine::is_match(&p, input), want, "{src}");
            // The engine's own early-stopping walk answers the same, whether
            // or not the scan routes through it.
            assert_eq!(crate::nfa::any_nfa(&p, input), Some(want), "{src} through the engine");
        }
    }

    #[test]
    fn the_reason_the_route_declines_agrees_with_whether_it_declines() {
        // A second implementation of the route's own refusals drifts from it
        // silently, and a wrong reason is worse than none: it sends the next
        // reader to the wrong stage. So the two are held together - "taken"
        // exactly where the route answers.
        let mut text = String::new();
        text.push_str("alpha = 1 ;\n");
        for i in 0..20_000u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i % 4_000 == 0 {
                text.push_str("alpha = 2 ;\n");
            }
        }
        let input = text.as_bytes();
        for src in [
            "\"alpha\" \"=\" \\N",
            "\"alpha\"",
            "\"alpha\"{1,2}",
            "\"value_1\" \"=\"",
            "\\W \"=\" \\N",
            "\\W \"=\" \\N ~\"alpha\"",
            "\"alpha\" \\N*",
            "\\G \"alpha\"",
            "\"alpha\" \\K \"=\"",
            "\\A \"alpha\"",
            "\"alpha\" \"=\" \\N \";\" \\z",
        ] {
            let p = crate::parse(src).expect("pattern parses");
            let why = required_window_reason(&p, input);
            let took = scan_required_windows(&p, input).is_some();
            assert_eq!(
                took,
                why.starts_with("taken"),
                "{src}: the route {} but the reason is `{why}`",
                if took { "answered" } else { "declined" }
            );
        }
        // A corpus whose tokens are long is where the settling runs out, and
        // the reason must name a side rather than only the stage.
        let mut long = String::new();
        for i in 0..20_000u32 {
            let run: String = (0..200usize)
                .map(|j| b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
                    [(i as usize * 31 + j * 17) % 64] as char)
                .collect();
            long.push_str(&format!("blob_{i} = {run} ;\n"));
            if i == 10_000 {
                long.push_str("rare_value = 4242 ;\n");
            }
        }
        let long = long.as_bytes();
        let p = crate::parse("\"rare_value\" \"=\" \\N").expect("pattern parses");
        let why = required_window_reason(&p, long);
        assert_eq!(
            scan_required_windows(&p, long).is_some(),
            why.starts_with("taken"),
            "long tokens: the reason is `{why}`"
        );
    }

    #[test]
    fn a_wider_match_bound_gives_the_route_up_at_the_same_literal_density() {
        // A window is as wide as a match can be, so the same occurrences cover
        // more of the input as the pattern's token bound grows. The route must
        // give up somewhere along that axis and stay given up, whatever the
        // gate's own numbers are: the property is monotone, the crossover is
        // measured.
        let mut text = String::new();
        for i in 0..40_000u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            if i % 7_000 == 0 {
                text.push_str("alpha = 2 ;\n");
            }
        }
        let input = text.as_bytes();
        let mut answered = Vec::new();
        for k in [1usize, 10, 40, 200] {
            let p = crate::parse(&format!("\"alpha\" \\W{{{k}}}")).expect("pattern parses");
            let got = scan_required_windows(&p, input);
            if let Some(spans) = &got {
                assert_eq!(spans, &engine(&p, input), "the route answered wrongly at {k}");
            }
            answered.push(got.is_some());
        }
        assert_eq!(answered.first(), Some(&true), "the narrowest bound is worth windowing");
        assert_eq!(answered.last(), Some(&false), "the widest is not");
        assert!(
            answered.windows(2).all(|w| w[0] >= w[1]),
            "the route took a wider bound after giving up on a narrower one: {answered:?}"
        );
    }

    #[test]
    fn the_window_route_declines_what_it_cannot_answer() {
        // Each of these has a reason the windows cannot hold it, and the
        // route hands the input back rather than answering wrongly.
        let input = b"alpha = 1 ; beta = 2 ; alpha = 3 ;" as &[u8];
        for src in [
            // Nothing required, so no literal anchors a window.
            "\\W \"=\"",
            // Unbounded, so no window is wide enough for a match.
            "\"alpha\" \\N*",
            // The matches must abut, which a window cannot know.
            "\\G \"alpha\"",
            // The reported start moves off the anchor.
            "\"alpha\" \\K \"=\"",
            // These read whether any token precedes or follows in the whole
            // stream, which a window's own tokens cannot say.
            "\\A \"alpha\"",
            "\"alpha\" \"=\" \\N \";\" \\z",
            // A guard's literal is satisfied by an occurrence at or after the
            // match, at any distance, so it anchors nothing; this pattern
            // holds no literal of its own either.
            "\\W \\N ~\"alpha\"",
            // A positive assertion consumes nothing, so its literal is beside
            // the match rather than inside it.
            "\\W ~(\"alpha\")",
        ] {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(scan_required_windows(&p, input), None, "{src} must decline");
        }
    }

    #[test]
    fn a_guards_literal_is_required_of_the_input_and_not_of_the_match() {
        // The two collectors answer different questions, and reading one for
        // the other would look for matches only near the guard's literal when
        // they are spread across the whole input.
        let guarded = crate::parse("\\W \\N ~\"alpha\"").expect("pattern parses");
        let mut required: Vec<Vec<u8>> = Vec::new();
        collect_required_literals(&guarded, &mut required);
        assert_eq!(required, vec![b"alpha".to_vec()], "the input must hold it");
        let mut contained: Vec<Vec<u8>> = Vec::new();
        collect_contained_literals(&guarded, &mut contained);
        assert!(contained.is_empty(), "no match holds it inside its own span");
        // With a literal of its own beside the guard, the match holds that one
        // and the windows anchor on it; the guard's literal is still not held.
        let both = crate::parse("\\W \"=\" \\N ~\"alpha\"").expect("pattern parses");
        let mut required: Vec<Vec<u8>> = Vec::new();
        collect_required_literals(&both, &mut required);
        assert_eq!(required, vec![b"=".to_vec(), b"alpha".to_vec()]);
        let mut contained: Vec<Vec<u8>> = Vec::new();
        collect_contained_literals(&both, &mut contained);
        assert_eq!(contained, vec![b"=".to_vec()]);
        // A literal atom is held by the match, and both collectors name it.
        let plain = crate::parse("\"alpha\" \"=\"").expect("pattern parses");
        let mut contained: Vec<Vec<u8>> = Vec::new();
        collect_contained_literals(&plain, &mut contained);
        assert_eq!(contained, vec![b"alpha".to_vec(), b"=".to_vec()]);
    }

    #[test]
    fn a_window_answers_the_same_at_every_hit_density() {
        // The coverage gate decides between windowing and the whole scan, and
        // whichever it picks the answer is the engine's. Densities from one
        // hit in a large input to a hit on every line.
        for (label, text) in [
            ("one hit", {
                let mut s = "x = 1 ;\n".repeat(6000);
                s.push_str("alpha = 9 ;\n");
                s
            }),
            ("every line", "alpha = 1 ;\n".repeat(3000)),
            ("every other line", "alpha = 1 ;\nx = 2 ;\n".repeat(1500)),
        ] {
            let input = text.as_bytes();
            let p = crate::parse("\"alpha\" \"=\"").expect("pattern parses");
            let want = engine(&p, input);
            if let Some(got) = scan_required_windows(&p, input) {
                assert_eq!(got, want, "{label}");
            }
            assert_eq!(crate::scan(&p, input), want, "{label} through the scan's own route");
        }
    }

    #[test]
    fn a_required_literal_absent_means_no_match_exists() {
        // The short-circuit relies on this contract. Each case is checked
        // against the engine as well, because a wrong answer here does not
        // fail loudly - it returns no matches for an input that has some.
        let cases: &[(&str, &[u8])] = &[
            ("\"alpha\"", b"beta gamma delta" as &[u8]),
            ("\\W ~\"zzz\"", b"one two three"),
            ("\"alpha\" \\N", b"beta 42"),
        ];
        for (src, input) in cases {
            let p = crate::parse(src).expect("pattern parses");
            assert!(requires_absent(&p, input), "{src} should read as unmatchable");
            assert!(engine(&p, input).is_empty(), "{src} must have no match to lose");
        }
    }

    #[test]
    fn a_literal_that_is_not_required_does_not_short_circuit() {
        // Each input lacks a literal the pattern mentions, and each must still
        // be scanned. A negated guard is satisfied by that absence, and an
        // alternation requires only what every branch requires - so a branch
        // that can match without the missing literal keeps the pattern alive.
        let cases: &[(&str, &[u8])] = &[
            ("\\W !~\"zzz\"", b"one two three" as &[u8]),
            ("(\"alpha\" | \"beta\")", b"beta gamma"),
            ("(\"alpha\" | \\N)", b"beta 42"),
        ];
        for (src, input) in cases {
            let p = crate::parse(src).expect("pattern parses");
            assert!(!requires_absent(&p, input), "{src} must not short-circuit");
            assert!(!engine(&p, input).is_empty(), "{src} has a match the engine must find");
        }
    }

    #[test]
    fn a_typed_atom_short_circuits_on_the_marker_its_recognizer_requires() {
        // `try_email` refuses without an `@` and `try_url` without a `://`,
        // so an input holding neither holds no token of that kind and the
        // whole lex is skipped. Without this a `\E` over a megabyte of log
        // would tokenize every byte of it to report that it holds no address.
        let cases: &[(&str, &[u8])] = &[
            ("\\E", b"no address here, only words and 12 numbers" as &[u8]),
            ("\\U", b"http and https are named but no scheme separator is"),
            ("\\E \\W", b"plain words carrying no at sign at all"),
            ("\\{jwt}", b"a token would open with the base64 of a brace and quote"),
            ("\\{percent}", b"ninety nine of a hundred, written out"),
            ("\\{hexcolor}", b"colors named rather than given in hex"),
            ("\\V", b"deploy build one two three shipped"),
            ("\\C", b"route 192.168.0.0 added"),
            ("\\{uuid}", b"id 550e8400e29b41d4a716446655440000 seen"),
            ("\\$", b"cost 1234.56 today"),
            ("\\{geo}", b"at 37.7749 and -122.4194"),
        ];
        for (src, input) in cases {
            let p = crate::parse(src).expect("pattern parses");
            assert!(requires_absent(&p, input), "{src} should read as unmatchable");
            assert!(engine(&p, input).is_empty(), "{src} must have no match to lose");
        }
    }

    #[test]
    fn a_typed_atom_keeps_an_input_that_holds_its_marker() {
        // The false-negative side, which is the one that loses matches. The
        // alternation is the case to watch: only a literal that each branch
        // requires is required of the whole, so a branch matching without
        // the marker has to keep the input alive.
        let cases: &[(&str, &[u8])] = &[
            ("\\E", b"write to bob@example.com today" as &[u8]),
            ("\\U", b"see http://example.com/p for more"),
            ("\\{percent}", b"cpu at 97% and climbing"),
            ("\\{hexcolor}", b"background #ff8800 today"),
            ("\\V", b"deploy 1.2.3 shipped"),
            ("\\C", b"route 192.168.0.0/24 added"),
            ("\\{uuid}", b"id 550e8400-e29b-41d4-a716-446655440000 seen"),
            ("\\$", b"cost $1,234.56 today"),
            ("\\{geo}", b"at 37.7749,-122.4194 exactly"),
            ("(\\E | \"error\")", b"error with no at sign anywhere"),
        ];
        for (src, input) in cases {
            let p = crate::parse(src).expect("pattern parses");
            assert!(!requires_absent(&p, input), "{src} must not short-circuit");
            assert!(!engine(&p, input).is_empty(), "{src} has a match the engine must find");
        }
    }

    #[test]
    fn a_kind_with_alternative_forms_contributes_no_required_literal() {
        // An Ip is v4 with dots or v6 with colons, a Mac takes either
        // separator, a Path either slash, a Phone is `+`-led or hyphenated,
        // and a Timestamp is dated or clocked. No single literal is required
        // of any of them, and naming one would refuse an input holding the
        // other form. A Word and a Number require no bytes at all.
        for src in ["\\I", "\\T", "\\A", "\\L", "\\{phone}", "\\W", "\\N"] {
            let p = crate::parse(src).expect("pattern parses");
            let mut lits: Vec<Vec<u8>> = Vec::new();
            collect_required_literals(&p, &mut lits);
            assert!(lits.is_empty(), "{src} must contribute no required literal, got {lits:?}");
        }
    }

    /// The run predicate's deciding boundary, and the one it is easy to get
    /// wrong: `try_base64` refuses below sixteen but counts up to two bytes of
    /// padding inside that, so a token's run of alphabet bytes can be
    /// fourteen. A predicate reading the stated sixteen refuses an input the
    /// recognizer accepts, which is a lost match.
    #[test]
    fn the_run_a_base64_token_needs_is_fourteen_because_its_padding_counts() {
        let p = crate::parse("\\{base64}").expect("pattern parses");

        // Fourteen alphabet bytes reaching sixteen on two `=`.
        let padded: &[u8] = b"blob aB3dEfGhIjKlM1== here";
        assert!(
            !engine(&p, padded).is_empty(),
            "the recognizer takes a fourteen-byte body with its padding"
        );
        assert!(!requires_absent_run(&p, padded), "so the filter must not refuse it");

        // No run of the class reaches fourteen.
        let none: &[u8] = b"short words only, and none of them long";
        assert!(requires_absent_run(&p, none));
        assert!(engine(&p, none).is_empty(), "nothing to lose by refusing it");
    }

    /// A digest is a hex run of exactly 32, 40 or 64 with no padding, so 32 is
    /// the shortest that could be one and 31 cannot.
    #[test]
    fn the_run_a_hash_digest_needs_is_thirty_two() {
        let p = crate::parse("\\D").expect("pattern parses");
        let held: &[u8] = b"sum d41d8cd98f00b204e9800998ecf8427e end";
        assert!(!engine(&p, held).is_empty(), "an md5 is a digest");
        assert!(!requires_absent_run(&p, held));

        // Thirty-one hex bytes: no digest can be there.
        let short: &[u8] = b"sum d41d8cd98f00b204e9800998ecf842 end";
        assert!(requires_absent_run(&p, short));
        assert!(engine(&p, short).is_empty());
    }

    /// A hex run is longer than 32 at a length no digest has, so 33 is the
    /// shortest that could be one, and a digest's 32 cannot.
    #[test]
    fn the_run_a_hex_run_needs_is_thirty_three() {
        let p = crate::parse("\\{hex}").expect("pattern parses");
        let held: &[u8] = b"key d41d8cd98f00b204e9800998ecf8427e0 end";
        assert!(!engine(&p, held).is_empty(), "thirty-three hex digits are a hex run");
        assert!(!requires_absent_run(&p, held), "so the filter must not refuse it");

        // Thirty-two hex bytes: a digest, and no hex run can be there.
        let digest: &[u8] = b"sum d41d8cd98f00b204e9800998ecf8427e end";
        assert!(requires_absent_run(&p, digest));
        assert!(engine(&p, digest).is_empty(), "nothing to lose by refusing it");
    }

    /// A Mac is six pairs of hex digits joined by five copies of one
    /// separator, so it is always seventeen bytes and every one of them is a
    /// hex digit or a separator. It is the only kind carrying no required
    /// literal that carries a required run.
    #[test]
    fn the_run_a_mac_needs_is_seventeen_because_its_shape_is_fixed() {
        let p = crate::parse("\\{mac}").expect("pattern parses");

        let colons: &[u8] = b"host 01:23:45:67:89:ab up";
        assert!(!engine(&p, colons).is_empty(), "that is a mac");
        assert!(!requires_absent_run(&p, colons), "so the filter must not refuse it");

        // The class admits both separators where a token holds only one, which
        // is the safe direction for a necessary condition to err.
        let dashes: &[u8] = b"host 01-23-45-67-89-ab up";
        assert!(!engine(&p, dashes).is_empty(), "the hyphen form is a mac too");
        assert!(!requires_absent_run(&p, dashes));

        // Fourteen bytes of the class: a pair short, so no mac can be there.
        let short: &[u8] = b"host 01:23:45:67:89 up";
        assert!(requires_absent_run(&p, short));
        assert!(engine(&p, short).is_empty(), "nothing to lose by refusing it");
    }

    /// A kind naming no run is never refused this way, and neither is a
    /// pattern that is more than a bare atom - the condition is necessary for
    /// the kind rather than for the pattern.
    #[test]
    fn a_kind_or_a_shape_that_names_no_run_is_never_refused_by_one() {
        let hay: &[u8] = b"short words only, and none of them long";
        for src in ["\\W", "\\N", "\\E", "\\U", "\\{uuid}", "\\I", "\\A"] {
            let p = crate::parse(src).expect("pattern parses");
            assert!(!requires_absent_run(&p, hay), "{src} names no run");
        }
        // A base64 atom inside a larger shape is admitted rather than read
        // through a condition that was only ever necessary for the atom.
        for src in ["\\{base64} \\W", "(\\{base64} | \"x\")", "\\{base64}*"] {
            let p = crate::parse(src).expect("pattern parses");
            assert!(!requires_absent_run(&p, hay), "{src} is not a bare atom");
        }
    }

    /// No filter may ever reject a literal that truly occurs in the
    /// corpus. This is the load-bearing contract: zero false negatives.
    fn assert_no_false_negatives(f: &dyn Membership) {
        for lit in present_literals() {
            assert!(
                f.might_contain(lit),
                "{} false-negatived present literal {:?}",
                f.name(),
                std::str::from_utf8(lit).unwrap()
            );
        }
    }

    #[test]
    fn bloom_has_no_false_negatives() {
        assert_no_false_negatives(&BloomFilter::build(CORPUS));
    }

    #[test]
    fn cuckoo_has_no_false_negatives() {
        assert_no_false_negatives(&CuckooFilter::build(CORPUS));
    }

    #[test]
    fn xor_has_no_false_negatives() {
        assert_no_false_negatives(&XorFilter::build(CORPUS));
    }

    #[test]
    fn absent_literals_are_mostly_rejected() {
        // Absence is the prefilter's value: an absent literal should be
        // rejected (returns false) so the exact scan never runs. A rare
        // false positive is permitted, so require the strong majority.
        for f in [
            Box::new(BloomFilter::build(CORPUS)) as Box<dyn Membership>,
            Box::new(CuckooFilter::build(CORPUS)),
            Box::new(XorFilter::build(CORPUS)),
        ] {
            let rejected = absent_literals().iter().filter(|l| !f.might_contain(l)).count();
            assert!(
                rejected >= 5,
                "{} rejected only {rejected}/6 absent literals",
                f.name()
            );
        }
    }

    #[test]
    fn short_literal_is_never_rejected() {
        // A literal shorter than the n-gram width cannot be decomposed,
        // so every filter must pass it through to the exact scan.
        let f = BloomFilter::build(CORPUS);
        assert!(f.might_contain(b"zz"));
    }

    #[test]
    fn absent_guard_literals_finds_the_missing_one() {
        let pattern = crate::parser::parse(". ~\"CRITICAL\"").unwrap();
        let absent = absent_guard_literals(&pattern, CORPUS);
        assert!(absent.contains(b"CRITICAL".as_slice()));

        let pattern2 = crate::parser::parse(". ~\"ERROR\"").unwrap();
        let absent2 = absent_guard_literals(&pattern2, CORPUS);
        assert!(absent2.is_empty(), "ERROR is present, so nothing is absent");
    }
}
