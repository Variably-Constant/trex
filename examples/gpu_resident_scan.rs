//! Does holding the token stream on the device save time across many scans?
//!
//! Three arms scan one text for the same six patterns. `scan_gpu` lexes and
//! uploads on every call. The re-upload arm lexes once and uploads the kinds
//! again for every pattern. The held arm lexes and uploads once. Each arm's
//! clock includes its own lexing, so each times the whole job of scanning the
//! text for every pattern, and the held arm against the re-upload arm is the
//! upload alone.
//!
//! Correctness first: for every pattern the held scan must return the plain
//! device scan's matches, and those must equal the CPU engine's. A fourth arm
//! reruns `scan_gpu`, and its ratio against the first must read 1.00x. Arm
//! order rotates every round, and each ratio is the median of per-round ratios.
//!
//! Run on a CUDA host: `cargo run --release --features gpu --example
//! gpu_resident_scan -- <corpus> [bytes...] [--check]`, where `--check` runs
//! the correctness pass alone.

use std::env;
use std::fs;
use std::hint::black_box;
use std::time::Instant;

use trex::gpu::{gpu_eligible, scan_gpu, GpuTokens};
use trex::parallel_lex::lex_parallel;
use trex::{parse, scan};

const PATTERNS: [&str; 6] = ["\\W \\N", "\\N+", "\\N{2,4}", "(\\N \\W)+", ". .", "\\W \\W \\W"];
const ROUNDS: usize = 9;

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let check_only = env::args().any(|a| a == "--check");
    let args: Vec<String> = env::args().skip(1).filter(|a| a != "--check").collect();
    let Some(path) = args.first() else {
        eprintln!("usage: gpu_resident_scan <corpus> [bytes...] [--check]");
        std::process::exit(2);
    };
    let sizes: Vec<usize> = if args.len() > 1 {
        args[1..]
            .iter()
            .map(|s| s.parse().unwrap_or_else(|e| panic!("size {s:?} does not parse: {e}")))
            .collect()
    } else {
        vec![2_097_152, 16_777_216]
    };
    let raw = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let patterns: Vec<_> = PATTERNS
        .iter()
        .map(|s| {
            let p = parse(s).unwrap_or_else(|e| panic!("{s} does not parse: {e:?}"));
            assert!(gpu_eligible(&p), "{s} is outside the device subset");
            p
        })
        .collect();

    for &size in &sizes {
        if size > raw.len() {
            println!("{size} bytes: the corpus holds only {} bytes", raw.len());
            continue;
        }
        let input = &raw[..size];

        let Some(held) = GpuTokens::upload(input) else {
            eprintln!("no usable device, or this build lacks the gpu feature");
            std::process::exit(1);
        };
        for (s, p) in PATTERNS.iter().zip(&patterns) {
            let cpu = scan(p, input);
            let plain = scan_gpu(p, input).expect("the device held the tokens");
            let resident = held.scan(p).expect("the device held the tokens");
            assert_eq!(plain, cpu, "{s}: the device scan differs from the CPU engine on {size} bytes");
            assert_eq!(resident, plain, "{s}: the held scan differs from the device scan on {size} bytes");
        }
        println!(
            "{size} bytes, {} significant tokens: held, device and CPU agree on all {} patterns",
            held.len(),
            patterns.len()
        );
        drop(held);
        if check_only {
            continue;
        }

        let per_call = || {
            let t = Instant::now();
            for p in &patterns {
                black_box(scan_gpu(p, input).expect("the device answered the correctness pass"));
            }
            t.elapsed().as_secs_f64() * 1e3
        };
        let reupload = || {
            let t = Instant::now();
            let toks = lex_parallel(input);
            for p in &patterns {
                let h = GpuTokens::from_tokens(&toks).expect("the device answered the correctness pass");
                black_box(h.scan(p).expect("the device answered the correctness pass"));
            }
            t.elapsed().as_secs_f64() * 1e3
        };
        let held_arm = || {
            let t = Instant::now();
            let h = GpuTokens::upload(input).expect("the device answered the correctness pass");
            for p in &patterns {
                black_box(h.scan(p).expect("the device answered the correctness pass"));
            }
            t.elapsed().as_secs_f64() * 1e3
        };
        let arms: [&dyn Fn() -> f64; 4] = [&per_call, &reupload, &held_arm, &per_call];

        let mut ms: [Vec<f64>; 4] = Default::default();
        let (mut control, mut call_vs_held, mut upload_vs_held) = (Vec::new(), Vec::new(), Vec::new());
        for round in 0..ROUNDS {
            let mut t = [0.0f64; 4];
            for i in 0..4 {
                let arm = (i + round) % 4;
                t[arm] = arms[arm]();
            }
            for (arm, v) in t.iter().enumerate() {
                ms[arm].push(*v);
            }
            control.push(t[0] / t[3]);
            call_vs_held.push(t[0] / t[2]);
            upload_vs_held.push(t[1] / t[2]);
        }
        println!(
            "  control scan_gpu / scan_gpu {:.3}x   scan_gpu / held {:.2}x   re-upload / held {:.2}x",
            median(&mut control),
            median(&mut call_vs_held),
            median(&mut upload_vs_held)
        );
        println!(
            "  median ms for {} scans: scan_gpu {:.1}, re-upload {:.1}, held {:.1}",
            patterns.len(),
            median(&mut ms[0]),
            median(&mut ms[1]),
            median(&mut ms[2])
        );
    }
}
