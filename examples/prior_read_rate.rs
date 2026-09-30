//! How often does the coder actually read the baked prior?
//!
//! The bit loop runs six orders by eight bits for every input byte, but those
//! are reads of the live adaptive slot table. The prior is reached only when a
//! slot holds a different context, so the rate that matters is the slot miss
//! rate, and it falls as the table warms.
//!
//! That rate is the multiplier on any per-lookup cost of changing how the
//! prior is stored. A mapped prior costs more per lookup and almost nothing at
//! startup; whether that trade wins depends on this number rather than on the
//! bit count.
//!
//! Reported per input length, so the decay is visible rather than averaged
//! away: a prior read on the first kilobyte and one on the thousandth are not
//! worth the same, because the coder that has seen less needs the prior more.
//!
//! Run: `cargo run --release --example prior_read_rate -- <corpus> [max-bytes]`

use std::env;
use std::fs;

use trex::seam::{baked_model, logistic_mix_bits, prior_reads, reset_prior_reads};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: prior_read_rate <corpus-file> [max-bytes]");
        std::process::exit(2);
    });
    let max: usize = match env::args().nth(2) {
        None => 524_288,
        Some(s) => match s.parse() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("second argument {s:?} is not a byte count: {e}");
                std::process::exit(2);
            }
        },
    };
    let raw = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let baked = baked_model();

    println!("{path}: {} bytes available", raw.len());
    println!(
        "{:>10} {:>14} {:>14} {:>10} {:>10} {:>11}",
        "bytes", "prior reads", "slot reads", "per byte", "miss rate", "bits/byte"
    );

    let mut largest = 0u64;
    for cap in [8_192usize, 32_768, 131_072, 262_144, 524_288] {
        if cap > raw.len() || cap > max {
            break;
        }
        let input = &raw[..cap];
        reset_prior_reads();
        let bits = logistic_mix_bits(input, true, &[], Some(baked));
        let reads = prior_reads();
        // Six orders by eight bits, which is what the bit loop runs.
        let slots = (cap as u64) * 6 * 8;
        println!(
            "{cap:>10} {reads:>14} {slots:>14} {:>10.2} {:>9.2}% {:>11.4}",
            reads as f64 / cap as f64,
            100.0 * reads as f64 / slots as f64,
            bits / cap as f64
        );
        largest = reads;
    }

    // The branch is taken whether or not a prior is loaded - without one it
    // yields the uniform (0, 0) instead of a row. So the count must not move,
    // and a difference would mean the two runs are not coding the same way and
    // no rate above describes the coder that ships.
    let n = max.min(raw.len());
    reset_prior_reads();
    let unprimed_bits = logistic_mix_bits(&raw[..n], true, &[], None);
    let unprimed = prior_reads();
    reset_prior_reads();
    let primed_bits = logistic_mix_bits(&raw[..n], true, &[], Some(baked));
    let primed = prior_reads();

    println!();
    println!(
        "control: the branch is taken {unprimed} times with no prior and {primed} times \
         with one, over {n} bytes"
    );
    assert_eq!(
        unprimed, primed,
        "the slot miss branch runs whether or not a prior is loaded, so the two counts must \
         agree; they did not, so the rate above does not describe one coder"
    );
    println!(
        "         and the prior is worth {:.3}x here ({:.4} bits/byte without it, {:.4} with)",
        unprimed_bits / primed_bits,
        unprimed_bits / n as f64,
        primed_bits / n as f64
    );
    println!("         largest size read the prior {largest} times");
    println!();
    println!("The miss rate is what multiplies any per-lookup cost of storing the");
    println!("prior differently. The bit loop's six-by-eight is the slot read count,");
    println!("not the prior read count.");
}
