//! What share of a device compress is sending the prior?
//!
//! The device coder reads a prior table on every slot miss. The host builds
//! that table once and caches it, then copies all of it to the device on every
//! call. Residency would pay that copy once; this measures what it costs per
//! call, so the saving is a number rather than an assumption.
//!
//! The phased call must return the same code length as the plain one, checked
//! before any timing is printed.
//!
//! Run on a CUDA host: `cargo run --release --features gpu --example
//! gpu_compress_phases -- <corpus>`

use std::env;
use std::fs;

use trex::gpu::{compress_gpu, compress_gpu_phases};

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: gpu_compress_phases <corpus>");
        std::process::exit(2);
    };
    let raw = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let Some(warm) = compress_gpu(&raw[..raw.len().min(65_536)]) else {
        eprintln!("no usable device, or this build lacks the gpu feature");
        std::process::exit(1);
    };
    std::hint::black_box(warm);

    println!("{path}: {} bytes", raw.len());
    println!(
        "{:>10} {:>7} {:>9} {:>9} {:>9} {:>10} {:>11} {:>7}",
        "bytes", "chunks", "prep us", "input us", "alloc us", "prior us", "kern+dn us", "prior%"
    );
    for cap in [65_536usize, 262_144, 1_048_576, 4_194_304] {
        if cap > raw.len() {
            break;
        }
        let input = &raw[..cap];
        let plain = compress_gpu(input).expect("device ran the warm-up");
        let mut prep = Vec::new();
        let mut up_in = Vec::new();
        let mut alloc = Vec::new();
        let mut up_prior = Vec::new();
        let mut kern = Vec::new();
        let mut last = None;
        for _ in 0..5 {
            let (bits, p) = compress_gpu_phases(input).expect("device ran the plain call");
            assert_eq!(
                bits.to_bits(),
                plain.to_bits(),
                "phased compress coded {cap} bytes to {bits} bits against {plain}"
            );
            prep.push(p.host_prep_us);
            up_in.push(p.upload_input_us);
            alloc.push(p.alloc_us);
            up_prior.push(p.upload_prior_us);
            kern.push(p.kernel_and_download_us);
            last = Some(p);
        }
        let p = last.expect("five rounds ran");
        let (a, b, c, d, e) =
            (median(&mut prep), median(&mut up_in), median(&mut alloc), median(&mut up_prior), median(&mut kern));
        let total = a + b + c + d + e;
        println!(
            "{cap:>10} {:>7} {a:>9.1} {b:>9.1} {c:>9.1} {d:>10.1} {e:>11.1} {:>6.1}%",
            p.chunks,
            100.0 * d / total
        );
        if cap == 65_536 {
            println!("{:>10} prior sent every call: {} bytes", "", p.prior_bytes);
        }
    }
}
