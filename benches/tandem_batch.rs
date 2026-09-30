//! CPU + GPU tandem dispatch over a batch of inputs.
//!
//! Builds a batch of inputs, scans them three ways -- all on the CPU,
//! all on the GPU, and split across both in tandem -- asserts the tandem
//! result is identical to the per-input CPU scan, and reports each
//! wall-clock so the overlap is visible.
//!
//!   cargo bench --bench tandem_batch
//!
//! Needs the `tandem` feature (which implies `gpu`) and a CUDA host.
//!
//! Each arm is read once a round over several rounds, in an order that
//! rotates, and reduced by its median. The speedup divides one arm's median
//! into another's, and a figure taken that way is only worth reading if the
//! box held still while all three were timed - which is what the spreads and
//! the closing control say.

use std::time::Instant;

use trex::engine::{Span, scan};
use trex::gpu::scan_gpu;
use trex::parse;
use trex::tandem::scan_batch_tandem;

/// How many times each arm is read.
const ROUNDS: usize = 5;

/// The middle of `v` and its spread, the largest reading over the smallest.
///
/// A median rather than a mean because one arm that met a neighbour's spike
/// moves a mean and does not move a middle. The spread is reported beside it
/// because a median hides how far the readings ranged.
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
    // A batch of medium inputs, each a device-eligible pattern target.
    let batch = 600;
    let pat = parse("\\W \\N").expect("pattern parses");
    let inputs: Vec<Vec<u8>> = (0..batch)
        .map(|i| {
            let mut s = String::with_capacity(20_000);
            for j in 0..1_000 {
                s.push_str(&format!("row {} val {} ", i * 1000 + j, j * 7));
            }
            s.into_bytes()
        })
        .collect();

    let total_bytes: usize = inputs.iter().map(Vec::len).sum();
    println!("batch of {batch} inputs, {total_bytes} bytes total, pattern \\W \\N");

    // Warm the device (JIT + context) and the CPU caches so every timed
    // path below is measured warm, not charged one-time setup.
    std::hint::black_box(scan_gpu(&pat, &inputs[0]));
    std::hint::black_box(scan(&pat, &inputs[0]));

    // The three arms take each other's places every round, so none of them is
    // always the one that runs first into whatever the box was doing.
    let (mut cpu, mut gpu, mut tan) = (Vec::new(), Vec::new(), Vec::new());
    let mut cpu_out: Vec<Vec<Span>> = Vec::new();
    let mut gpu_out: Vec<Vec<Span>> = Vec::new();
    let mut tan_out: Vec<Vec<Span>> = Vec::new();
    for round in 0..ROUNDS {
        for arm in 0..3 {
            match (round + arm) % 3 {
                0 => {
                    let (t, out) = timed(|| inputs.iter().map(|inp| scan(&pat, inp)).collect());
                    cpu.push(t);
                    cpu_out = out;
                }
                1 => {
                    let (t, out) = timed(|| {
                        inputs
                            .iter()
                            .map(|inp| scan_gpu(&pat, inp).unwrap_or_else(|| scan(&pat, inp)))
                            .collect()
                    });
                    gpu.push(t);
                    gpu_out = out;
                }
                _ => {
                    // `scan_batch_tandem` takes the batch by value, so the copy
                    // it needs is made before the clock starts. Inside it, this
                    // arm alone paid for twelve megabytes of clone that neither
                    // other arm pays, which is a difference between the arms
                    // and not between the backends.
                    let owned = inputs.clone();
                    let (t, out) = timed(|| scan_batch_tandem(&pat, owned));
                    tan.push(t);
                    tan_out = out;
                }
            }
        }
    }

    // The CPU arm once more, after all three are done: how far the box moved
    // across the whole run. A control far from that arm's own median says the
    // arms were not read under the same conditions, whatever each arm's own
    // spread says about its own rounds.
    let (control, _) = timed(|| -> Vec<Vec<Span>> { inputs.iter().map(|inp| scan(&pat, inp)).collect() });

    assert_eq!(tan_out, cpu_out, "tandem result must equal the per-input CPU scan");
    assert_eq!(gpu_out, cpu_out, "GPU result must equal the per-input CPU scan");

    let matches: usize = cpu_out.iter().map(Vec::len).sum();
    let (cpu_ms, cpu_spread) = median_and_spread(cpu);
    let (gpu_ms, gpu_spread) = median_and_spread(gpu);
    let (tan_ms, tan_spread) = median_and_spread(tan);
    println!("matches: {matches} (identical across all three paths)");
    println!("CPU only   : {cpu_ms:>8.2} ms  spread {cpu_spread:>5.2}x");
    println!("GPU only   : {gpu_ms:>8.2} ms  spread {gpu_spread:>5.2}x");
    println!("CPU+GPU    : {tan_ms:>8.2} ms  spread {tan_spread:>5.2}x  (tandem)");
    println!(
        "  control, the CPU arm re-read at the end: {control:>8.2} ms, {:.2}x its median",
        control / cpu_ms
    );
    let best_single = cpu_ms.min(gpu_ms);
    println!("speedup vs best single backend, median over median: {:.2}x", best_single / tan_ms);
}
