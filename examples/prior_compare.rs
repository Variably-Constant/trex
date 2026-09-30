//! What does a prior cost, and is a given one better than the shipped one?
//!
//! A prior earns its bytes by predicting the input in fewer bits than no prior
//! at all. The shipped one predicts English prose; whether it predicts anything
//! else is a measurement, and a prior trained on other text has to be measured
//! the same way rather than assumed better.
//!
//! Reports bits per byte through the mixer for no prior, the compiled-in prior,
//! and each blob named on the command line. The ratio against no prior is what
//! says whether a prior is worth loading at all: below one it pays for itself,
//! above one the input compresses better without it.
//!
//! The input must not be in the blob's training corpus. A prior measured on its
//! own training text reports how well it memorized, not how well it predicts.
//!
//! Run: `cargo run --release --example prior_compare -- <corpus> [bytes] [blob...]`

use std::env;
use std::fs;

use trex::seam::{baked_model, load_byte_ngram, logistic_mix_bits};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: prior_compare <corpus-file> [bytes] [blob...]");
        std::process::exit(2);
    };
    let cap: usize = match args.get(1) {
        None => 131_072,
        Some(s) => match s.parse() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("second argument {s:?} is not a byte count: {e}");
                std::process::exit(2);
            }
        },
    };
    let raw = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let input = &raw[..cap.min(raw.len())];
    let n = input.len() as f64;

    println!("{path}: {} bytes", input.len());
    println!("{:>28} {:>12} {:>10} {:>12}", "prior", "blob bytes", "bits/byte", "vs no prior");

    let none = logistic_mix_bits(input, true, &[], None) / n;
    println!("{:>28} {:>12} {none:>10.4} {:>12}", "none", 0, "1.000x");

    let baked = baked_model();
    let rows: usize = baked.iter().map(|(_, m)| m.len()).sum();
    let bpb = logistic_mix_bits(input, true, &[], Some(baked)) / n;
    println!(
        "{:>28} {:>12} {bpb:>10.4} {:>11.3}x",
        format!("shipped ({rows} rows)"),
        "compiled in",
        bpb / none
    );

    for p in args.iter().skip(2) {
        let blob = match fs::read(p) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("cannot read {p}: {e}");
                std::process::exit(1);
            }
        };
        let tables = load_byte_ngram(&blob);
        let rows: usize = tables.iter().map(|(_, m)| m.len()).sum();
        let bpb = logistic_mix_bits(input, true, &[], Some(&tables)) / n;
        let name = p.rsplit(['\\', '/']).next().unwrap_or(p);
        println!(
            "{:>28} {:>12} {bpb:>10.4} {:>11.3}x",
            format!("{name} ({rows} rows)"),
            blob.len(),
            bpb / none
        );
    }

    println!();
    println!("Below 1.000x the prior pays for itself; above it the input codes");
    println!("better with no prior loaded.");
}
