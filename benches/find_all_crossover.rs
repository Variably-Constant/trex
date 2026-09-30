//! Where splitting a find-all across the cores starts paying, by input size.
//!
//! The literal routes walk every occurrence of a literal, and that walk is one
//! pass of the vector search over the whole input: 0.6526 ms of the 1.5473 an
//! anchor search costs on 7.34 MB. The search is embarrassingly parallel - an
//! occurrence depends on its own bytes and nothing else - but a split has a
//! cost of its own, and below some size it is the slower form.
//!
//! This finds that size. The threshold it set is `FIND_ALL_PARALLEL_THRESHOLD`
//! in `src/byte_simd.rs`, which carries this table beside it; re-run this and
//! re-set it when the per-byte cost of the search moves.
//!
//! Two needles, because the search's per-byte cost runs over an order of
//! magnitude between them: one whose first byte is common, so a candidate is
//! verified often, and one whose first byte is rare, so the scan runs at its
//! own rate. A crossover read off only the cheap needle would be set too high
//! for the other.
//!
//! Run: `cargo run --release --bench find_all_crossover`.

use std::hint::black_box;
use std::time::Instant;

/// The corpus `benches/vs_regex_full.rs` builds, cut to `bytes`.
fn corpus(bytes: usize) -> Vec<u8> {
    let mut s = String::new();
    let mut i = 0usize;
    while s.len() < bytes {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
        i += 1;
    }
    let mut v = s.into_bytes();
    v.truncate(bytes);
    v
}

const ROUNDS: usize = 7;
const BUDGET_MS: f64 = 20.0;

fn time_budget(f: &mut dyn FnMut() -> usize) -> f64 {
    black_box(f());
    let t0 = Instant::now();
    let mut iters = 0u32;
    loop {
        black_box(f());
        iters += 1;
        if iters >= 5 && t0.elapsed().as_secs_f64() * 1e3 >= BUDGET_MS {
            break;
        }
    }
    t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// One size and one needle, the two forms timed in an order that rotates so
/// neither reads its position as its own cost.
fn cell(input: &[u8], needle: &[u8]) -> (f64, f64, usize, f64) {
    let mut one = || trex::byte_simd::find_all_one_pass(input, needle).len();
    let mut across = || trex::byte_simd::find_all_across(input, needle).len();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for r in 0..ROUNDS {
        if r % 2 == 0 {
            a.push(time_budget(&mut one));
            b.push(time_budget(&mut across));
        } else {
            b.push(time_budget(&mut across));
            a.push(time_budget(&mut one));
        }
    }
    // The first form read once more, after every round of both. It is the same
    // work as the reading it repeats, so the distance between them is the box
    // moving under the cell, and a crossover reported closer than that
    // distance is the box rather than the two forms.
    let control = time_budget(&mut one);
    let n = trex::byte_simd::find_all_one_pass(input, needle).len();
    assert_eq!(trex::byte_simd::find_all_across(input, needle).len(), n, "the two forms disagree");
    assert_eq!(
        trex::byte_simd::occurrences(input, needle).count(),
        n,
        "the spanned walk the routes take disagrees with both"
    );
    let first = median(a);
    (first, median(b), n, control / first)
}

fn main() {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    println!("trex {}, {cores} cores", trex::version());
    println!("{:>10} {:>12} {:>11} {:>11} {:>9} {:>10}", "bytes", "needle", "one pass", "across", "ratio", "hits");
    // `let` opens a quarter of the statements, so its first byte is common and
    // every window verifies; `zzzqqq` occurs nowhere, so the scan runs at its
    // own rate and never verifies.
    for needle in [&b"let"[..], &b"zzzqqq"[..]] {
        for bytes in [4 * 1024, 16 * 1024, 64 * 1024, 128 * 1024, 512 * 1024, 2 * 1024 * 1024, 7_342_498] {
            let input = corpus(bytes);
            let (one, across, hits, drift) = cell(&input, needle);
            println!(
                "{bytes:>10} {:>12} {one:>11.5} {across:>11.5} {:>8.2}x {hits:>10} {drift:>8.3}x",
                String::from_utf8_lossy(needle),
                one / across
            );
        }
    }
    println!("\nthe crossover is the smallest size whose ratio exceeds 1.00 for both needles");
}
