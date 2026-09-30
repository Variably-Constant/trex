//! Does holding the device prior pay across many compresses?
//!
//! Correctness first: a compress against the held default prior must return
//! the bits `compress_gpu` returns for the same input, for every piece.
//!
//! Then two arms code the same pieces. `compress_gpu` sends the prior with
//! every call. The held arm uploads the prior once, inside its clock, and
//! codes every piece against it. A third arm reruns `compress_gpu` and its
//! ratio against the first must read 1.00x. Arm order rotates every round, and
//! each ratio is the median of per-round ratios.
//!
//! Run on a CUDA host: `cargo run --release --features gpu --example
//! gpu_held_prior -- <corpus> [piece bytes] [pieces] [--check]`, where
//! `--check` runs the correctness pass alone.

use std::env;
use std::fs;
use std::hint::black_box;
use std::time::Instant;

use trex::gpu::{compress_gpu, GpuPrior};

const ROUNDS: usize = 9;

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn arg_or(args: &[String], i: usize, default: usize) -> usize {
    match args.get(i) {
        None => default,
        Some(s) => s.parse().unwrap_or_else(|e| panic!("argument {i} {s:?} does not parse: {e}")),
    }
}

fn main() {
    let check_only = env::args().any(|a| a == "--check");
    let args: Vec<String> = env::args().skip(1).filter(|a| a != "--check").collect();
    let Some(path) = args.first() else {
        eprintln!("usage: gpu_held_prior <corpus> [piece bytes] [pieces] [--check]");
        std::process::exit(2);
    };
    let piece = arg_or(&args, 1, 262_144);
    let count = arg_or(&args, 2, 16);
    assert!(piece > 0 && count > 0, "pieces need a positive size and count");
    let raw = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    assert!(
        piece * count <= raw.len(),
        "{count} pieces of {piece} bytes need {} bytes; {path} holds {}",
        piece * count,
        raw.len()
    );
    let pieces: Vec<&[u8]> = raw[..piece * count].chunks(piece).collect();

    // The correctness pass holds its prior in this block, so the device's copy
    // is released before the timing arms hold their own.
    {
        let Some(held) = GpuPrior::default_prior() else {
            eprintln!("no usable device, or this build lacks the gpu feature");
            std::process::exit(1);
        };
        for (i, p) in pieces.iter().enumerate() {
            let plain = compress_gpu(p).expect("the device held the prior");
            let from_held = held.compress(p).expect("the device held the prior");
            assert_eq!(
                from_held.to_bits(),
                plain.to_bits(),
                "piece {i}: the held prior coded {from_held} bits against {plain}"
            );
        }
    }
    println!("{count} pieces of {piece} bytes: the held prior codes every piece to the same bits as compress_gpu");
    if check_only {
        return;
    }

    let per_call = || {
        let t = Instant::now();
        for p in &pieces {
            black_box(compress_gpu(p).expect("the device answered the correctness pass"));
        }
        t.elapsed().as_secs_f64() * 1e3
    };
    let held_arm = || {
        let t = Instant::now();
        let h = GpuPrior::default_prior().expect("the device answered the correctness pass");
        for p in &pieces {
            black_box(h.compress(p).expect("the device answered the correctness pass"));
        }
        t.elapsed().as_secs_f64() * 1e3
    };
    let arms: [&dyn Fn() -> f64; 3] = [&per_call, &held_arm, &per_call];

    let mut ms: [Vec<f64>; 3] = Default::default();
    let (mut control, mut call_vs_held) = (Vec::new(), Vec::new());
    for round in 0..ROUNDS {
        let mut t = [0.0f64; 3];
        for i in 0..3 {
            let arm = (i + round) % 3;
            t[arm] = arms[arm]();
        }
        for (arm, v) in t.iter().enumerate() {
            ms[arm].push(*v);
        }
        control.push(t[0] / t[2]);
        call_vs_held.push(t[0] / t[1]);
    }
    println!(
        "  control compress_gpu / compress_gpu {:.3}x   compress_gpu / held {:.3}x",
        median(&mut control),
        median(&mut call_vs_held)
    );
    println!(
        "  median ms for {count} pieces: compress_gpu {:.1}, held {:.1}",
        median(&mut ms[0]),
        median(&mut ms[1])
    );
}
