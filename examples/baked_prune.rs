//! What does dropping redundant rows take off the shipped prior?
//!
//! trex compiles a byte-n-gram prior into the binary. Its cost is bytes on
//! disk, and those bytes are mostly the contexts' own literal keys, so
//! collapsing counts cannot reach them - a row has to go entirely.
//!
//! A row earns its place by disagreeing with the shorter context the coder
//! would fall back to. This drops the ones that agree, re-serializes through
//! the same encoder that produced the shipped blob, and reports the blob
//! size that results beside what the thinner prior costs in bits per byte.
//!
//! Vigilance above one cannot be reached by an overlap, so nothing is
//! dropped and the blob round-trips: that row is the control, and it also
//! shows what the decode and re-encode cost on their own.
//!
//! Run: `cargo run --release --example baked_prune -- <corpus> [bytes]`

use std::env;
use std::fs;

use trex::seam::{load_byte_ngram, logistic_mix_bits, prune_baked};

/// The shipped blob, as compiled into the binary.
const BAKED: &[u8] = include_bytes!("../_corpus/baked_model.br");

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: baked_prune <corpus-file> [bytes]");
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
    let n = input.len() as f64;

    println!("{path}: {} bytes", input.len());
    println!("shipped blob: {} bytes", BAKED.len());
    println!(
        "{:>10} {:>10} {:>10} {:>12} {:>10} {:>9}",
        "vigilance", "rows in", "rows out", "blob bytes", "bits/byte", "vs shipped"
    );

    let shipped = load_byte_ngram(BAKED);
    let base = logistic_mix_bits(input, true, &[], Some(&shipped)) / n;

    // How much the prior is worth on this input at all.
    //
    // What corpus trained the prior is not recorded anywhere in the tree -
    // it was an argument to an offline `--build-model` run - so its fit has
    // to be measured rather than looked up. A prior helps most on text like
    // the text it saw, so this figure says how close the input is to that
    // corpus, and it bounds what any overlap between the two could be worth:
    // where the prior barely helps, an overlap cannot have inflated much.
    let unprimed = logistic_mix_bits(input, true, &[], None) / n;
    println!(
        "prior is worth {:.3}x here ({unprimed:.4} bits/byte without it, {base:.4} with)",
        unprimed / base
    );
    println!(
        "{:>10} {:>10} {:>10} {:>12} {base:>10.4} {:>9}",
        "shipped",
        "-",
        "-",
        BAKED.len(),
        "-"
    );

    let (ctl_blob, ctl_in, ctl_out) = prune_baked(BAKED, 1.001);
    let ctl_tables = load_byte_ngram(&ctl_blob);
    let ctl_bpb = logistic_mix_bits(input, true, &[], Some(&ctl_tables)) / n;
    let clean = (ctl_bpb - base).abs() < 1e-9;
    println!(
        "{:>10} {ctl_in:>10} {ctl_out:>10} {:>12} {ctl_bpb:>10.4} {:>8.3}x",
        "none",
        ctl_blob.len(),
        ctl_bpb / base
    );
    println!(
        "  control: unreachable vigilance {} the shipped prior ({ctl_bpb:.6} vs {base:.6}), \
         rows {ctl_in} -> {ctl_out}",
        if clean { "reproduces" } else { "DOES NOT REPRODUCE" }
    );
    assert!(
        clean,
        "with vigilance above one no row can agree with its backoff, so nothing is dropped and \
         the prior must predict exactly as the shipped one does; it did not, so the decode and \
         re-encode are not lossless and no row below is worth anything"
    );

    for rho in [0.99f32, 0.95, 0.9, 0.8, 0.7] {
        let (blob, rin, rout) = prune_baked(BAKED, rho);
        let tables = load_byte_ngram(&blob);
        let bpb = logistic_mix_bits(input, true, &[], Some(&tables)) / n;
        println!(
            "{rho:>10.2} {rin:>10} {rout:>10} {:>12} {bpb:>10.4} {:>8.3}x",
            blob.len(),
            bpb / base
        );
    }
    println!();
    println!("blob bytes is what would ship; bits/byte is what the thinner prior costs.");
}
