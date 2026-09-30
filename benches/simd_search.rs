//! What the hand-rolled byte search and the run functions cost, against their
//! scalar baselines and against the standard library's substring search.
//!
//! Three axes, because one number hides what decides these.
//!
//! A haystack whose needle is absent and whose first byte never occurs is the
//! pure-throughput case: no candidate is ever verified, so it reads the scan
//! itself. `std::str::find` belongs beside it, since that is what a caller
//! gets without this module and is the baseline it has to beat to be worth
//! routing a literal through.
//!
//! The needle ladder varies what the search is looking for over one haystack.
//! The search probes the needle's first byte across a window and verifies at
//! each candidate, so a needle whose first byte is common pays a verify the
//! rare one never does, and the per-byte cost runs over an order of magnitude
//! between them.
//!
//! The run ladder varies token length. A run function is called with the rest
//! of the input and stops at the first byte outside its class, so a token
//! shorter than a SIMD window should be answered by one window whatever the
//! window is wide - runs of 8 and 24 bytes sit inside a 32-byte window, 40 and
//! 56 need a second 32 but still a single 64, and 200 needs several of either.

use std::hint::black_box;
use std::time::Instant;

use trex::byte_simd::{find, find_scalar, word_run};

/// Statements of four shapes, the corpus `benches/vs_regex` sweeps.
fn corpus(statements: usize) -> String {
    let mut s = String::new();
    for i in 0..statements {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
    }
    s
}

/// The middle of `v` and its spread, the largest reading over the smallest.
fn median_and_spread(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("a measured time is never NaN"));
    (v[v.len() / 2], v[v.len() - 1] / v[0])
}

const ROUNDS: usize = 5;

/// `f` timed once a round, reporting the middle round and the spread.
fn rounds(mut f: impl FnMut() -> usize) -> (f64, f64, usize) {
    let mut v = Vec::with_capacity(ROUNDS);
    let mut n = 0usize;
    for _ in 0..ROUNDS {
        let t0 = Instant::now();
        n = black_box(f());
        v.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    let (t, s) = median_and_spread(v);
    (t, s, n)
}

/// Every position at which `needle` occurs, the shape `prefilter` searches in:
/// the whole haystack is walked whatever the count.
fn hits(haystack: &[u8], needle: &[u8]) -> usize {
    let mut n = 0usize;
    let mut from = 0usize;
    while let Some(rel) = find(&haystack[from..], needle) {
        n += 1;
        from = from + rel + 1;
    }
    n
}

/// How often each byte of `needle` occurs in `haystack`, per thousand bytes,
/// for the one the search probes and the one it rejects on.
fn probe_density(haystack: &[u8], needle: &[u8]) -> (usize, usize) {
    let count = |b: u8| haystack.iter().filter(|&&c| c == b).count() * 1000 / haystack.len();
    (count(needle[0]), count(needle[needle.len() - 1]))
}

/// The three searches over one absent needle, rotating so none of them always
/// runs on a cold cache.
fn absent_needle_throughput() {
    let bytes = 32 * 1024 * 1024usize;
    let haystack: Vec<u8> = (0..bytes).map(|i| b'a' + (i % 23) as u8).collect();
    let needle = b"NEEDLE_NOT_PRESENT";
    let hay_str = std::str::from_utf8(&haystack).expect("ascii");
    let needle_str = std::str::from_utf8(needle).expect("ascii");

    assert_eq!(find(&haystack, needle), find_scalar(&haystack, needle));
    assert_eq!(find(&haystack, needle), None);
    assert_eq!(hay_str.find(needle_str), find(&haystack, needle));

    let (mut sca, mut std_, mut sim) = (Vec::new(), Vec::new(), Vec::new());
    for round in 0..ROUNDS {
        for arm in 0..3 {
            let t0 = Instant::now();
            match (round + arm) % 3 {
                0 => {
                    black_box(find_scalar(black_box(&haystack), black_box(needle)));
                    sca.push(t0.elapsed().as_secs_f64() * 1e3);
                }
                1 => {
                    black_box(black_box(hay_str).find(black_box(needle_str)));
                    std_.push(t0.elapsed().as_secs_f64() * 1e3);
                }
                _ => {
                    black_box(find(black_box(&haystack), black_box(needle)));
                    sim.push(t0.elapsed().as_secs_f64() * 1e3);
                }
            }
        }
    }
    let (t_sca, s_sca) = median_and_spread(sca);
    let (t_std, s_std) = median_and_spread(std_);
    let (t_sim, s_sim) = median_and_spread(sim);
    let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    println!("\n=== one absent needle over 32 MiB, no candidate ever verified ===");
    println!("  {:<16} {:>9} {:>9} {:>8} {:>7}", "search", "ms", "GiB/s", "vs simd", "spread");
    for (name, t, s) in
        [("scalar", t_sca, s_sca), ("std::str::find", t_std, s_std), ("byte_simd::find", t_sim, s_sim)]
    {
        println!(
            "  {name:<16} {t:>9.3} {:>9.2} {:>7.2}x {s:>6.2}x",
            gib / (t / 1e3),
            t / t_sim
        );
    }
}

/// What the search costs per byte of haystack, by needle.
fn needle_ladder(bytes: &[u8]) {
    let mb = bytes.len() as f64 / (1024.0 * 1024.0);
    println!("\n=== the search over needle, one haystack of {} bytes ===", bytes.len());
    println!(
        "  {:<28} {:>4} {:>7} {:>7} {:>9} {:>9} {:>9} {:>7}",
        "needle", "len", "first", "last", "hits", "ms", "MB/s", "spread"
    );
    for needle in [
        &b"alpha"[..],
        &b"value_"[..],
        &b"item_"[..],
        &b"cond_"[..],
        &b"zzzqqq"[..],
        &b"a"[..],
        &b"=="[..],
        &b"call_199999(alpha, beta, 199999) ;"[..],
    ] {
        let (first, last) = probe_density(bytes, needle);
        let (ms, spread, found) = rounds(|| hits(bytes, needle));
        let name = String::from_utf8_lossy(&needle[..needle.len().min(28)]).to_string();
        println!(
            "  {name:<28} {:>4} {first:>5}pm {last:>5}pm {found:>9} {ms:>9.3} {:>9.1} {spread:>6.2}x",
            needle.len(),
            mb / (ms / 1e3)
        );
    }
}

/// A haystack of word runs of exactly `run` bytes, each followed by a space.
fn runs_of(run: usize, bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + run + 1);
    while out.len() < bytes {
        out.extend(std::iter::repeat_n(b'w', run));
        out.push(b' ');
    }
    out
}

/// What `word_run` costs per byte at token lengths either side of both SIMD
/// window widths.
fn run_ladder() {
    println!("\n=== word_run over token length, 4 MiB a row ===");
    println!(
        "  {:>8} {:>10} {:>10} {:>9} {:>9} {:>7}",
        "run", "bytes", "calls", "ms", "MB/s", "spread"
    );
    for run in [8usize, 24, 40, 56, 200] {
        let hay = runs_of(run, 4 << 20);
        let mb = hay.len() as f64 / (1024.0 * 1024.0);
        let (ms, spread, calls) = rounds(|| {
            let mut at = 0usize;
            let mut n = 0usize;
            while at < hay.len() {
                at += word_run(&hay[at..]).max(1);
                n += 1;
            }
            n
        });
        println!(
            "  {run:>8} {:>10} {calls:>10} {ms:>9.3} {:>9.1} {spread:>6.2}x",
            hay.len(),
            mb / (ms / 1e3)
        );
    }
}

fn main() {
    // The rung the ladders will exercise. Every function below picks its
    // widest available implementation at run time, so a curve that does not
    // bend at a vector width means either that the width buys nothing or that
    // the host has not got it, and only this line tells the two apart.
    println!("trex {}   widest instruction set: {}", trex::version(), trex::isa::tier().name());
    absent_needle_throughput();
    let text = corpus(200_000);
    needle_ladder(text.as_bytes());
    run_ladder();
    println!("\nDONE");
}
