//! What does a cheaper baked prior give up?
//!
//! trex compiles a byte-n-gram prior into the binary - about nine and a
//! third megabytes, brotli-compressed. Unlike the model a coder builds as it
//! goes, this one is stored and shipped, so making it cheaper is worth
//! something and the question is what it costs in prediction.
//!
//! Quantizing the prior's counts to the classes its rows resonate in leaves
//! every key in place and collapses the values, so what shrinks is the
//! entropy of the value stream. The saving is reported as the count of
//! distinct values, which is what a general compressor has to spend bits on;
//! the cost is reported in bits per byte through the same mixer the shipped
//! prior feeds.
//!
//! Vigilance above one cannot be reached by an overlap, so nothing merges
//! and the prior is the shipped one. That row is the control.
//!
//! Run: `cargo run --release --example baked_vigilance -- <corpus> [bytes]`

use std::collections::HashSet;
use std::env;
use std::fs;

use trex::seam::{PreloadTable, baked_model, cluster_preload, logistic_mix_bits};

/// Distinct count pairs across every order: what the value stream costs a
/// compressor, as opposed to how many rows there are.
fn distinct_values(tables: &[PreloadTable]) -> usize {
    let mut seen: HashSet<(u32, u32)> = HashSet::new();
    for (_, m) in tables {
        seen.extend(m.values().copied());
    }
    seen.len()
}

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: baked_vigilance <corpus-file> [bytes]");
        std::process::exit(2);
    });
    let cap: usize = match env::args().nth(2) {
        None => 131_072,
        Some(s) => s.parse().unwrap_or_else(|e| {
            eprintln!("second argument {s:?} is not a byte count: {e}");
            std::process::exit(2);
        }),
    };
    let raw = fs::read(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(1);
    });
    let input = &raw[..cap.min(raw.len())];

    let baked = baked_model();
    let rows: usize = baked.iter().map(|(_, m)| m.len()).sum();
    println!("{path}: {} bytes", input.len());
    println!("baked prior: {rows} rows over {} orders", baked.len());
    println!(
        "{:>10} {:>10} {:>12} {:>10} {:>9}",
        "vigilance", "classes", "distinct vals", "bits/byte", "vs shipped"
    );

    let base = logistic_mix_bits(input, true, &[], Some(baked)) / input.len() as f64;
    println!(
        "{:>10} {:>10} {:>12} {base:>10.4} {:>9}",
        "shipped",
        "-",
        distinct_values(baked),
        "-"
    );

    let (ctl, _, ctl_classes) = cluster_preload(baked, 1.001);
    let ctl_bpb = logistic_mix_bits(input, true, &[], Some(&ctl)) / input.len() as f64;
    let clean = (ctl_bpb - base).abs() < 1e-9;
    println!(
        "{:>10} {ctl_classes:>10} {:>12} {ctl_bpb:>10.4} {:>8.3}x",
        "none",
        distinct_values(&ctl),
        ctl_bpb / base
    );
    println!(
        "  control: unreachable vigilance {} the shipped prior ({ctl_bpb:.6} vs {base:.6})",
        if clean { "reproduces" } else { "DOES NOT REPRODUCE" }
    );
    assert!(
        clean,
        "with vigilance above one no two rows can share a class, so the prior must be the \
         shipped one and the mixer must pay exactly what it pays with it; it did not, so the \
         quantizing itself moves a reading and no row below is worth anything"
    );

    for rho in [0.999f32, 0.99, 0.95, 0.9, 0.8] {
        let (q, _, classes) = cluster_preload(baked, rho);
        let bpb = logistic_mix_bits(input, true, &[], Some(&q)) / input.len() as f64;
        println!(
            "{rho:>10.3} {classes:>10} {:>12} {bpb:>10.4} {:>8.3}x",
            distinct_values(&q),
            bpb / base
        );
    }
}
