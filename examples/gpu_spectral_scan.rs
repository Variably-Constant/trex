//! Do spectral patterns pay on the device, per call and on held tokens?
//!
//! A spectral pattern reads a field the CPU engine builds once per scan. On
//! the device the host builds the same field, pools a reading per token and
//! uploads it; tokens held with their reading pay that once for many scans.
//! Correctness first: for every pattern the device scan and the held scan
//! must return the CPU engine's matches exactly.
//!
//! Then three arms scan one text for every pattern: the CPU engine;
//! `scan_gpu` per pattern; and one upload with the spectral reading, inside
//! its clock, followed by a scan of the held tokens per pattern. A fourth arm
//! reruns the CPU engine and its ratio against the first must read 1.00x. Arm
//! order rotates every round, and each ratio is the median of per-round
//! ratios.
//!
//! Run on a CUDA host: `cargo run --release --features gpu --example
//! gpu_spectral_scan -- <corpus> [bytes...] [--check]`, where `--check` runs
//! the correctness pass alone.

use std::env;
use std::fs;
use std::hint::black_box;
use std::time::Instant;

use trex::gpu::{gpu_eligible, scan_gpu, GpuTokens};
use trex::orbit::OrbitGroup;
use trex::{parse, scan};

const PATTERNS: [&str; 5] =
    ["\\F{entropy>0.8}", "\\F{entropy<0.3} \\W", "\\F{period:any}", "\\F{texture:prose} \\N", "\\F{onset}"];
const ROUNDS: usize = 9;

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let check_only = env::args().any(|a| a == "--check");
    let args: Vec<String> = env::args().skip(1).filter(|a| a != "--check").collect();
    let Some(path) = args.first() else {
        eprintln!("usage: gpu_spectral_scan <corpus> [bytes...] [--check]");
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
        let Some(held) = GpuTokens::upload_with_spectral(input, OrbitGroup::Identity) else {
            eprintln!("no usable device, or this build lacks the gpu feature");
            std::process::exit(1);
        };
        for (s, p) in PATTERNS.iter().zip(&patterns) {
            let cpu = scan(p, input);
            let device = scan_gpu(p, input).expect("the device held the tokens");
            let from_held = held.scan(p).expect("the tokens were held with the spectral reading");
            assert_eq!(device, cpu, "{s}: the device scan differs from the CPU engine on {size} bytes");
            assert_eq!(from_held, cpu, "{s}: the held scan differs from the CPU engine on {size} bytes");
            println!("  {s}: {} matches, device and held agree with the CPU", cpu.len());
        }
        println!("{size} bytes, {} significant tokens: all {} patterns agree", held.len(), patterns.len());
        drop(held);
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
        let held_arm = || {
            let t = Instant::now();
            let h = GpuTokens::upload_with_spectral(input, OrbitGroup::Identity)
                .expect("the device answered the correctness pass");
            for p in &patterns {
                black_box(h.scan(p).expect("the device answered the correctness pass"));
            }
            t.elapsed().as_secs_f64() * 1e3
        };
        let arms: [&dyn Fn() -> f64; 4] = [&cpu_arm, &device_arm, &held_arm, &cpu_arm];
        let mut ms: [Vec<f64>; 4] = Default::default();
        let (mut control, mut cpu_vs_device, mut cpu_vs_held) = (Vec::new(), Vec::new(), Vec::new());
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
            cpu_vs_device.push(t[0] / t[1]);
            cpu_vs_held.push(t[0] / t[2]);
        }
        println!(
            "  control cpu / cpu {:.3}x   cpu / scan_gpu {:.2}x   cpu / held {:.2}x",
            median(&mut control),
            median(&mut cpu_vs_device),
            median(&mut cpu_vs_held)
        );
        println!(
            "  median ms for {} scans: cpu {:.1}, scan_gpu {:.1}, held {:.1}",
            patterns.len(),
            median(&mut ms[0]),
            median(&mut ms[1]),
            median(&mut ms[2])
        );
    }
}
