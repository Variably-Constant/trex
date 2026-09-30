//! Does moving back-references onto the device pay against the CPU engine?
//!
//! Binding patterns such as `\W:x =x` ran only on the CPU until the device
//! carried an orbit class per token and read a one-token bind at a fixed
//! offset. Correctness first: for every pattern the device scan must return
//! the CPU engine's matches exactly, captures included.
//!
//! Then two arms scan one text for every pattern: the CPU engine, and
//! `scan_gpu`, which lexes, computes classes and uploads per call and resolves
//! captures on the host at the selected matches. A third arm reruns the CPU
//! engine and its ratio against the first must read 1.00x. Arm order rotates
//! every round, and each ratio is the median of per-round ratios.
//!
//! Run on a CUDA host: `cargo run --release --features gpu --example
//! gpu_bind_scan -- <corpus> [bytes...] [--check]`, where `--check` runs the
//! correctness pass alone.

use std::env;
use std::fs;
use std::hint::black_box;
use std::time::Instant;

use trex::gpu::{gpu_eligible, scan_gpu};
use trex::{parse, scan};

const PATTERNS: [&str; 5] = ["\\W:x =x", "\\W:x =case x", "\\W:x \\W =case x", "(\\W:x =case x)+", "\\W:t \\N"];
const ROUNDS: usize = 9;

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let check_only = env::args().any(|a| a == "--check");
    let args: Vec<String> = env::args().skip(1).filter(|a| a != "--check").collect();
    let Some(path) = args.first() else {
        eprintln!("usage: gpu_bind_scan <corpus> [bytes...] [--check]");
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

        for (s, p) in PATTERNS.iter().zip(&patterns) {
            let cpu = scan(p, input);
            let Some(device) = scan_gpu(p, input) else {
                eprintln!("no usable device, or this build lacks the gpu feature");
                std::process::exit(1);
            };
            assert_eq!(device, cpu, "{s}: the device scan differs from the CPU engine on {size} bytes");
            println!("  {s}: {} matches, device agrees with the CPU, captures included", cpu.len());
        }
        println!("{size} bytes: all {} patterns agree", patterns.len());
        if check_only {
            continue;
        }

        let cpu_arm = || {
            let t = Instant::now();
            for p in &patterns {
                black_box(scan(p, input));
            }
            t.elapsed().as_secs_f64() * 1e3
        };
        let device_arm = || {
            let t = Instant::now();
            for p in &patterns {
                black_box(scan_gpu(p, input).expect("the device answered the correctness pass"));
            }
            t.elapsed().as_secs_f64() * 1e3
        };
        let arms: [&dyn Fn() -> f64; 3] = [&cpu_arm, &device_arm, &cpu_arm];

        let mut ms: [Vec<f64>; 3] = Default::default();
        let (mut control, mut cpu_vs_device) = (Vec::new(), Vec::new());
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
            cpu_vs_device.push(t[0] / t[1]);
        }
        println!(
            "  control cpu / cpu {:.3}x   cpu / scan_gpu {:.2}x",
            median(&mut control),
            median(&mut cpu_vs_device)
        );
        println!(
            "  median ms for {} scans: cpu {:.1}, scan_gpu {:.1}",
            patterns.len(),
            median(&mut ms[0]),
            median(&mut ms[1])
        );
    }
}
