//! GPU-matched rewrite throughput against the CPU rewrite.
//!
//! A rewrite is match then render then splice. The match is the
//! expensive phase over a large input, and it is the phase the device
//! parallelizes; the render and splice run on the host either way. This
//! bench rewrites a large input with a device-eligible pattern on the
//! GPU and on the CPU, asserts the two outputs are byte-identical, and
//! reports the wall-clock of each whole rewrite.
//!
//!   cargo bench --bench gpu_rewrite_throughput
//!
//! Without a device the GPU path is unavailable and the run reports that it
//! was CPU-only.
//!
//! Each arm is read once a round over several rounds, in an order that
//! swaps every round, and reduced by its median. The speedup divides one
//! arm's median by the other's, and a figure taken that way is only worth
//! reading if the box held still while both were timed - which is what the
//! spreads and the closing control say.

use std::time::Instant;

use trex::parse;
use trex::rewrite::{Template, rewrite, rewrite_gpu};

/// How many times each arm is read.
const ROUNDS: usize = 5;

/// The middle of `v` and its spread, the largest reading over the smallest.
///
/// A median rather than a mean because one arm that met a neighbor's spike
/// moves a mean and does not move a middle. The spread is reported beside it
/// because a median hides how far the readings ranged, and a pair of arms that
/// are both near 1.00 was read on a box that held still.
fn median_and_spread(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("a measured time is never NaN"));
    (v[v.len() / 2], v[v.len() - 1] / v[0])
}

/// Milliseconds `f` takes, with what it answered.
fn timed<T>(f: impl FnOnce() -> T) -> (f64, T) {
    let t = Instant::now();
    let got = f();
    (t.elapsed().as_secs_f64() * 1000.0, got)
}

fn main() {
    // Wrap every number token in brackets: a device-eligible pattern
    // (`\N`, no binding) with a whole-match template, over a large input.
    let count = 2_000_000;
    let mut input = String::with_capacity(count * 10);
    for i in 0..count {
        input.push_str("tag ");
        input.push_str(&(i % 1000).to_string());
        input.push(' ');
    }
    let bytes = input.as_bytes();
    let pat = parse("\\N").expect("pattern parses");
    let tpl = Template::parse("[${0}]", &pat.capture_names()).expect("template parses");

    println!("input: {} bytes, ~{} number tokens, rewrite \\N -> [${{0}}]", bytes.len(), count);

    // One of each before the clock runs, so neither arm is charged the
    // device's context and JIT or the first touch of the output buffer.
    std::hint::black_box(rewrite(&pat, &tpl, bytes));
    std::hint::black_box(rewrite_gpu(&pat, &tpl, bytes));

    // The arms swap places every round, so neither is always the one that
    // follows the other into whatever the box was doing.
    let (mut cpu, mut gpu) = (Vec::new(), Vec::new());
    let mut cpu_out = Vec::new();
    let mut gpu_out = None;
    for round in 0..ROUNDS {
        for arm in 0..2 {
            if (round + arm) % 2 == 0 {
                let (t, out) = timed(|| rewrite(&pat, &tpl, bytes));
                cpu.push(t);
                cpu_out = out;
            } else {
                let (t, out) = timed(|| rewrite_gpu(&pat, &tpl, bytes));
                gpu.push(t);
                gpu_out = out;
            }
        }
    }

    // The CPU arm once more, after both arms are done: how far the box moved
    // across the whole run. A control far from that arm's own median says the
    // two arms were not read under the same conditions, whatever each arm's
    // own spread says about its own rounds.
    let (control, _) = timed(|| rewrite(&pat, &tpl, bytes));

    let (cpu_ms, cpu_spread) = median_and_spread(cpu);
    println!(
        "CPU rewrite: {cpu_ms:>8.2} ms  spread {cpu_spread:>5.2}x  ({} bytes out)",
        cpu_out.len()
    );
    println!(
        "  control, the CPU arm re-read at the end: {control:>8.2} ms, {:.2}x its median",
        control / cpu_ms
    );
    match gpu_out {
        Some(g) => {
            assert_eq!(g, cpu_out, "GPU and CPU rewrite output must be identical");
            let (gpu_ms, gpu_spread) = median_and_spread(gpu);
            println!("GPU rewrite: {gpu_ms:>8.2} ms  spread {gpu_spread:>5.2}x  ({} bytes out)", g.len());
            println!("identical: yes");
            println!("speedup (CPU / GPU), median over median: {:.2}x", cpu_ms / gpu_ms);
        }
        None => {
            println!("GPU rewrite: not available (no device, or built without --features gpu)");
        }
    }
}
