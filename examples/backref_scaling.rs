//! Does a bound register stay linear as the input grows?
//!
//! trex answers a back-reference with a register lookup rather than by
//! backtracking, but a pattern that binds a register also changes the
//! thread-dedup key: `src/nfa.rs` keys those threads on the counter and the
//! save slots together, because two threads at one counter with different
//! bindings are not interchangeable. The register-free claim of one visit per
//! counter-position pair therefore does not transfer to this case by
//! argument, and it is measured here instead.
//!
//! The reading is the shape of the curve rather than any single time.
//! Doubling the input should double the time if the scan is linear in it; a
//! super-linear cost shows as a ratio above one that grows.

use std::hint::black_box;
use std::time::Instant;

use trex::{parse, scan};

/// How long one cell is timed for, in milliseconds.
///
/// A short input read once is mostly the clock's own resolution and the cost
/// of touching a buffer nobody has touched yet. Reading it from as many passes
/// as this target needs puts every size on the same footing, so a size leaves
/// the ladder only when it answers nothing, never for being quick.
const INNER_TARGET_MS: f64 = 50.0;

/// Milliseconds one pass takes, read from as many passes as
/// [`INNER_TARGET_MS`] needs.
///
/// The first pass is discarded. A fresh buffer pays its page faults once, and
/// at the foot of a ladder the core is still at its idle clock, which lands on
/// whichever size happens to be first and reads as that size being slow.
fn timed_ms(run: &mut dyn FnMut() -> usize) -> f64 {
    black_box(run());
    let mut reps: u32 = 0;
    let t0 = Instant::now();
    let ms = loop {
        black_box(run());
        reps += 1;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if ms >= INNER_TARGET_MS {
            break ms;
        }
    };
    ms / f64::from(reps)
}

fn main() {
    // `\W:x =x` binds a word and demands the next token equal it - the
    // back-reference shape, and the one a backtracking engine answers with
    // (\w+)\s+\1.
    let pattern = parse(r"\W:x =x").expect("pattern parses");

    println!("pattern  \\W:x =x   (bind a word, require the next token to equal it)");
    println!("{:>10} {:>12} {:>10} {:>9}", "tokens", "bytes", "ms", "ratio");

    let mut prev: Option<(usize, f64)> = None;
    // The ladder spans a 256-fold range, and a short cell is read from many
    // passes rather than dropped: a size that runs quickly is still a size the
    // curve has to pass through, and a non-linearity that only appears at the
    // foot is invisible to a ladder that starts above it.
    for n in [
        25_000usize, 50_000, 100_000, 200_000, 400_000, 800_000, 1_600_000, 3_200_000, 6_400_000,
    ] {
        // No two adjacent words are equal, so nothing matches and the engine
        // cannot stop early: this is the full scan, not a lucky first hit.
        let mut input = String::with_capacity(n * 7);
        for i in 0..n {
            input.push('w');
            input.push_str(&(i % 9973).to_string());
            input.push(' ');
        }
        let bytes = input.as_bytes();

        assert!(
            scan(&pattern, bytes).is_empty(),
            "no two adjacent words are equal, so nothing should match"
        );
        let ms = timed_ms(&mut || scan(&pattern, bytes).len());

        let ratio = match prev {
            Some((pn, pms)) if pms > 0.0 && pn > 0 => {
                format!("{:.2}x", (ms / pms) / (n as f64 / pn as f64))
            }
            _ => "-".to_string(),
        };
        println!("{:>10} {:>12} {:>10.1} {:>9}", n, bytes.len(), ms, ratio);
        prev = Some((n, ms));
    }

    println!();
    println!("ratio is time-growth divided by input-growth: 1.00x is linear in the input.");

    // The bind above holds exactly one token, so one binding is alive per
    // thread and the register key collapses back to the counter. A bind of
    // unbounded length does not: `src/nfa.rs` keys a thread on the register
    // slots, so every distinct span the plus could have covered is a
    // separate live thread. That is the case where the cost has to grow, and
    // the shape of the growth is the whole reading.
    let wide = parse(r"\W+:x =x").expect("pattern parses");
    println!();
    println!("pattern  \\W+:x =x   (bind an unbounded run of words, then require equality)");
    println!("{:>10} {:>12} {:>10} {:>9}", "tokens", "bytes", "ms", "ratio");

    let mut prev: Option<(usize, f64)> = None;
    for n in [500usize, 1_000, 2_000, 4_000, 8_000] {
        let mut input = String::with_capacity(n * 7);
        for i in 0..n {
            input.push('w');
            input.push_str(&(i % 9973).to_string());
            input.push(' ');
        }
        let bytes = input.as_bytes();

        let matched = scan(&wide, bytes).len();
        let ms = timed_ms(&mut || scan(&wide, bytes).len());

        let ratio = match prev {
            Some((pn, pms)) if pms > 0.0 && pn > 0 => {
                format!("{:.2}x", (ms / pms) / (n as f64 / pn as f64))
            }
            _ => "-".to_string(),
        };
        println!(
            "{:>10} {:>12} {:>10.1} {:>9}   {} matches",
            n,
            bytes.len(),
            ms,
            ratio,
            matched
        );
        prev = Some((n, ms));
    }

    println!();
    println!("a ratio that holds near 2.00x across the sweep is quadratic in the input.");
}
