//! What it costs to answer a run-length question over a byte class with two
//! nibble lookups, against the automaton and the lex that answer it today.
//!
//! The kinds trex has no cheap refusal for are the ones with a counting
//! constraint and no required literal: a base64 blob is a run of at least
//! sixteen base64 bytes, a hash digest a hex run of exactly thirty-two, forty
//! or sixty-four. Unanchored, their automata are combinatorial - every live
//! starting position is at a different phase of the counter and the
//! determinized subsets multiply. Anchored, they sweep at a restart a byte,
//! which is the time spent exactly when the kind is absent and the refusal
//! would have been worth having.
//!
//! Both conditions are necessary rather than sufficient, and both are run
//! lengths over a byte class. A class is what two nibble lookups answer.
//!
//! ## Why the nibble
//!
//! `vpshufb` looks up thirty-two bytes against a sixteen-entry table in one
//! instruction, and sixteen entries is four bits of index. So a byte's two
//! nibbles are two lookups, and membership in an arbitrary set of ASCII bytes
//! is their conjunction: the low nibble selects which high nibbles it pairs
//! with, the high nibble selects its own bit, and a non-zero meet is
//! membership. Exact, not a filter over the class - the false positives are in
//! what a class says about a token, not in what the tables say about a byte.
//!
//! The reason this is a different order and not a better constant is the
//! dependency. [`trex::byte_dfa::ByteDfa`] steps through a two-hundred-and
//! -fifty-six entry table whose next index is the value just loaded, so the
//! walk is a dependent load chain and cannot vectorize however it is written.
//! A nibble classifier carries no state between bytes.
//!
//! ## What is measured
//!
//! `scalar` is the same predicate over [`trex::byte_nfa::ByteClass::has`], one
//! byte at a time, which is what the class costs without the tables. It is the
//! control: a reading where the vector path does not beat it says the tables
//! are wrong rather than the idea.
//!
//! The two paths are asserted to agree on every corpus before either is
//! reported. A faster answer that differs is not an answer.
//!
//! Run: `cargo run --release --example what_a_nibble_class_scan_costs -- <file>...`

#![allow(unsafe_code)]

use std::time::Instant;

use trex::byte_nfa::ByteClass;

/// A byte class with the run length its kind requires, and what that kind is.
struct Predicate {
    /// What the run is for.
    label: &'static str,
    /// The bytes a run of this kind is made of.
    class: ByteClass,
    /// The shortest run the kind could be, so a shorter longest run in the
    /// input means no token of it is there.
    least: usize,
}

/// The classes the kinds with no literal and no workable automaton need.
///
/// Spelled here rather than taken from `byte_lex`, whose pieces are private to
/// it, and named against the recognizer each one is the necessary condition of.
fn predicates() -> Vec<Predicate> {
    let digits = ByteClass::range(b'0', b'9');
    let lower = ByteClass::range(b'a', b'z');
    let upper = ByteClass::range(b'A', b'Z');

    // `try_base64` measures a length that INCLUDES up to two `=` of padding
    // and refuses below sixteen, so the run of alphabet bytes alone can be as
    // short as fourteen. Fourteen is therefore the shortest run that could
    // open a base64 token, and sixteen - the number the refusal names - would
    // refuse an input holding a fourteen-byte body with its padding.
    let base64 = digits
        .union(lower)
        .union(upper)
        .union(ByteClass::just(b'+'))
        .union(ByteClass::just(b'/'));

    // `try_hash` is gated on a word run of exactly thirty-two, forty or
    // sixty-four and takes no padding, so thirty-two is the shortest hex run
    // that could be one.
    let hex = digits.union(ByteClass::range(b'a', b'f')).union(ByteClass::range(b'A', b'F'));

    // A word run, which is the control: the engine already routes this kind
    // and its cost here says what the tables do on a class that is everywhere
    // rather than one that is rare.
    let word = digits.union(lower).union(upper).union(ByteClass::just(b'_'));

    vec![
        Predicate { label: "base64 >= 14", class: base64, least: 14 },
        Predicate { label: "hex >= 32", class: hex, least: 32 },
        Predicate { label: "word >= 1", class: word, least: 1 },
    ]
}

/// The two sixteen-entry tables that answer membership in `class` for an ASCII
/// byte, or `None` where the class holds a byte at or above `0x80`.
///
/// `lo[l]` carries a bit for each high nibble `h` under eight such that the
/// byte `h << 4 | l` is in the class, and `hi[h]` carries that one bit. A byte
/// is in the class where the two meet, and a byte at or above `0x80` has no
/// bit in `hi` and so is never claimed.
///
/// A class holding a byte at or above `0x80` cannot be answered this way with
/// one pair of tables, and is refused rather than answered wrongly: every
/// class below is ASCII, and a later one that is not must be seen to fail here
/// rather than silently lose its upper half.
fn nibble_tables(class: &ByteClass) -> Option<([u8; 16], [u8; 16])> {
    let mut lo = [0u8; 16];
    for b in 0..=255u8 {
        if !class.has(b) {
            continue;
        }
        if b >= 0x80 {
            return None;
        }
        lo[(b & 0x0F) as usize] |= 1 << (b >> 4);
    }
    let mut hi = [0u8; 16];
    for (h, slot) in hi.iter_mut().enumerate() {
        *slot = if h < 8 { 1 << h } else { 0 };
    }
    Some((lo, hi))
}

/// The longest run of bytes of `class` in `input`, read one byte at a time.
fn longest_run_scalar(class: &ByteClass, input: &[u8]) -> usize {
    let mut longest = 0usize;
    let mut run = 0usize;
    for &b in input {
        if class.has(b) {
            run += 1;
            if run > longest {
                longest = run;
            }
        } else {
            run = 0;
        }
    }
    longest
}

/// The longest run of bytes of the class the tables describe, read thirty-two
/// bytes at a time where the target has AVX2 and one at a time where it does
/// not.
///
/// The run is carried across vectors, so a run spanning a boundary is one run.
fn longest_run_nibble(lo: &[u8; 16], hi: &[u8; 16], input: &[u8]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: `trex::isa::tier` reports the rungs this CPU can run,
        // resolved once for the process, and a rung implies every rung below
        // it - so reaching this arm is the guarantee that the features the
        // body was compiled for are present. The same discharge `byte_simd`
        // makes for its own paths.
        if matches!(trex::isa::tier(), trex::isa::Tier::Avx2 | trex::isa::Tier::Avx512) {
            return unsafe { longest_run_avx2(lo, hi, input) };
        }
    }
    longest_run_tables_scalar(lo, hi, input)
}

/// [`longest_run_nibble`]'s answer computed from the same tables without any
/// vector instruction, which is what a target without AVX2 runs and what the
/// vector path is checked against.
fn longest_run_tables_scalar(lo: &[u8; 16], hi: &[u8; 16], input: &[u8]) -> usize {
    let mut longest = 0usize;
    let mut run = 0usize;
    for &b in input {
        let member = lo[(b & 0x0F) as usize] & hi[((b >> 4) & 0x0F) as usize] != 0;
        if member {
            run += 1;
            if run > longest {
                longest = run;
            }
        } else {
            run = 0;
        }
    }
    longest
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn longest_run_avx2(lo: &[u8; 16], hi: &[u8; 16], input: &[u8]) -> usize {
    use core::arch::x86_64::{
        __m256i, _mm256_and_si256, _mm256_broadcastsi128_si256, _mm256_cmpeq_epi8,
        _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8, _mm256_setzero_si256,
        _mm256_shuffle_epi8, _mm256_srli_epi16, _mm_loadu_si128,
    };

    // One sixteen-byte table broadcast to both lanes, because `shuffle_epi8`
    // indexes within each lane and the same table answers both.
    let lo_tbl = _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(lo.as_ptr().cast()) });
    let hi_tbl = _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(hi.as_ptr().cast()) });
    let low_nibble = _mm256_set1_epi8(0x0F);
    let zero = _mm256_setzero_si256();

    let mut longest = 0usize;
    let mut run = 0usize;
    let mut at = 0usize;

    while at + 32 <= input.len() {
        let v: __m256i = unsafe { _mm256_loadu_si256(input.as_ptr().add(at).cast()) };
        let lo_idx = _mm256_and_si256(v, low_nibble);
        // `srli_epi16` shifts sixteen bits at a time, so the byte below each
        // pair brings its top four bits down into this one; the mask removes
        // them and leaves the high nibble alone.
        let hi_idx = _mm256_and_si256(_mm256_srli_epi16::<4>(v), low_nibble);
        let met = _mm256_and_si256(
            _mm256_shuffle_epi8(lo_tbl, lo_idx),
            _mm256_shuffle_epi8(hi_tbl, hi_idx),
        );
        // A lane meets where the conjunction is non-zero, so compare against
        // zero and take the complement of the mask.
        let absent = _mm256_movemask_epi8(_mm256_cmpeq_epi8(met, zero)) as u32;
        let present = !absent;

        // Bit `i` is byte `i` of this vector, so a run has three parts: the
        // ones at the bottom continue the run carried in, the ones at the top
        // become the run carried out, and every run between them is whole
        // inside this word. A word that is all ones is none of the three and
        // simply extends the carry.
        if present == u32::MAX {
            run += 32;
            if run > longest {
                longest = run;
            }
        } else {
            let head = present.trailing_ones() as usize;
            run += head;
            if run > longest {
                longest = run;
            }
            let tail = present.leading_ones() as usize;
            let mut rest = present >> head;
            let mut seen = head;
            while rest != 0 {
                let gap = rest.trailing_zeros() as usize;
                rest >>= gap;
                seen += gap;
                if rest == 0 {
                    break;
                }
                let ones = rest.trailing_ones() as usize;
                // A run reaching the top of the word is the tail, counted
                // once below as the carry rather than twice here.
                if seen + ones < 32 && ones > longest {
                    longest = ones;
                }
                rest >>= ones;
                seen += ones;
            }
            run = tail;
            if run > longest {
                longest = run;
            }
        }
        at += 32;
    }

    // The tail, by the same rule as the body so a run crossing into it is one
    // run and not two.
    for &b in &input[at..] {
        let member = lo[(b & 0x0F) as usize] & hi[((b >> 4) & 0x0F) as usize] != 0;
        if member {
            run += 1;
            if run > longest {
                longest = run;
            }
        } else {
            run = 0;
        }
    }
    longest
}

/// Milliseconds one call takes, with what it returned.
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let started = Instant::now();
    let out = f();
    (out, started.elapsed().as_secs_f64() * 1000.0)
}

/// One corpus against every predicate.
fn report(path: &str, input: &[u8]) {
    let bytes = input.len() as f64;
    let ns_b = |ms: f64| ms * 1e6 / bytes;

    let (_, lex_par_ms) = timed(|| trex::parallel_lex::lex_parallel(input));

    println!(
        "\ncorpus {path}, {} bytes, lex par {lex_par_ms:.1} ms {:.2} ns/B",
        input.len(),
        ns_b(lex_par_ms)
    );
    println!(
        "{:>14} {:>11} {:>8} {:>11} {:>8} {:>7} {:>9} {:>11}",
        "predicate", "scalar", "ns/B", "nibble", "ns/B", "gain", "longest", "vs lex par"
    );

    for p in predicates() {
        let Some((lo, hi)) = nibble_tables(&p.class) else {
            println!("{:>14}  holds a byte at or above 0x80: two tables cannot answer it", p.label);
            continue;
        };

        let (want, scalar_ms) = timed(|| longest_run_scalar(&p.class, input));
        let (got, nibble_ms) = timed(|| longest_run_nibble(&lo, &hi, input));
        assert_eq!(
            want, got,
            "{}: the tables answered {got} where the class answers {want} on {path}",
            p.label
        );

        // What the predicate is for: a longest run shorter than the kind's
        // shortest possible token means no token of that kind is in the input.
        let refuses = want < p.least;
        println!(
            "{:>14} {scalar_ms:>8.1} ms {:>8.2} {nibble_ms:>8.1} ms {:>8.2} {:>6.1}x {want:>9} {:>10.2}x{}",
            p.label,
            ns_b(scalar_ms),
            ns_b(nibble_ms),
            scalar_ms / nibble_ms,
            lex_par_ms / nibble_ms,
            if refuses { "  refuses" } else { "" }
        );
    }
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("name one or more corpus files: a cost is a fact about the bytes it was read over");
        std::process::exit(2);
    }

    println!("trex {}", env!("CARGO_PKG_VERSION"));
    // Which path the readings below were taken on, because a scalar fallback
    // reporting a gain near one is a different fact from tables that do not
    // work.
    #[cfg(target_arch = "x86_64")]
    println!("isa tier {:?}", trex::isa::tier());

    for (i, path) in paths.iter().enumerate() {
        eprintln!("[{}/{}] {path}", i + 1, paths.len());
        match std::fs::read(path) {
            Ok(input) => report(path, &input),
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        }
    }

    println!(
        "\n`gain` is the class read one byte at a time over the same class read through the\n\
         nibble tables. The claim being tested is an order rather than a margin, so a gain\n\
         near one means the tables are wrong and not that the idea is."
    );
}
