//! What does merging resonant contexts cost a coder?
//!
//! The seam sweep merges once, after its model is built. A coder cannot:
//! symbol `t` is coded from a model built only from the bytes before it, and
//! a decoder holding the same prefix has to reach the same model. So the
//! merging here is causal - a context is placed using counts taken only from
//! already-coded bytes, and a decoder replays every decision.
//!
//! Reported in bits per byte, the same figure `trex compress --compare`
//! prints, so these rows compare directly with the established coders measured
//! on these corpora.
//!
//! Run: `cargo run --release --example ppm_vigilance -- <corpus> [bytes]`

use std::env;
use std::fs;

use trex::seam::{ppm_byte_bits, ppm_byte_bits_clustered, ppm_byte_bits_learned_orbit};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: ppm_vigilance <corpus-file> [bytes]");
        std::process::exit(2);
    });
    // An unreadable byte count is refused rather than defaulted: running at a
    // size nobody asked for and labelling the table with it is the failure
    // this whole exercise keeps turning up.
    let cap: usize = match env::args().nth(2) {
        None => 262_144,
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
    let order = 8;
    let n = input.len() as f64;
    println!("{path}: {} bytes, PPM order-{order}", input.len());

    let base_bpb = ppm_byte_bits(input, order, false) / n;
    println!(
        "{:>10} {:>10} {:>10} {:>10} {:>9}",
        "vigilance", "bits/byte", "classes", "placed", "vs off"
    );
    println!("{:>10} {base_bpb:>10.4} {:>10} {:>10} {:>9}", "off", "-", "-", "-");

    // The control is vigilance ABOVE one.
    //
    // Overlap is a sum of per-follower minima over two distributions, so it
    // cannot exceed one and nothing resonates: every context mints its own
    // class, and the coder must then give exactly the bits per byte it gives
    // unclustered.
    // That is what proves the machinery - the private-then-placed path, the
    // class lookup, the shared update - moves no reading on its own.
    //
    // Vigilance exactly at one is a real point on the curve here rather than
    // an identity, because a coder merges while it is still learning. Once
    // two contexts share a row, every later update to either lands on both,
    // so a pair that matched when they were placed stops matching as soon as
    // they are fed different bytes. An analysis pass has no such effect: its
    // model is final before anything merges, so identical distributions stay
    // identical and sharing a row is free. For a coder there is no free tier.
    let (ctl_bits, _, ctl_classes) = ppm_byte_bits_clustered(input, order, false, 1.001);
    let ctl_bpb = ctl_bits / n;
    let clean = (ctl_bpb - base_bpb).abs() < 1e-9;
    println!(
        "{:>10} {ctl_bpb:>10.4} {ctl_classes:>10} {:>10} {:>8.3}x",
        "none",
        "-",
        ctl_bpb / base_bpb
    );
    println!(
        "  control: unreachable vigilance {} the unclustered coder ({ctl_bpb:.6} vs {base_bpb:.6})",
        if clean { "reproduces" } else { "DOES NOT REPRODUCE" }
    );
    assert!(
        clean,
        "with vigilance above one nothing can resonate, so the clustered coder must give exactly \
         the plain one's bits per byte; it did not, so the machinery itself moves a reading and no \
         row below is worth anything"
    );

    for rho in [1.0f32, 0.95, 0.9, 0.8, 0.7, 0.5] {
        let (bits, placed, classes) = ppm_byte_bits_clustered(input, order, false, rho);
        let bpb = bits / n;
        println!("{rho:>10.2} {bpb:>10.4} {classes:>10} {placed:>10} {:>8.3}x", bpb / base_bpb);
    }

    // A learned orbit is the other thing ART can be here: it narrows the
    // alphabet the contexts are spelled in rather than making two contexts
    // share a row, so contexts keep their own futures.
    println!();
    println!("=== learned orbit: bytes clustered by what follows them ===");
    println!("the fixed orbit lowercases; this one asks the stream");
    let fixed = ppm_byte_bits(input, order, true) / n;
    println!("{:>10} {fixed:>10.4} {:>10} {:>10} {:>8.3}x", "lowercase", "-", "-", fixed / base_bpb);
    let (ctl, ctl_alpha) = ppm_byte_bits_learned_orbit(input, order, 1.001);
    let ctl_bpb = ctl / n;
    let clean = (ctl_bpb - base_bpb).abs() < 1e-9;
    println!(
        "{:>10} {ctl_bpb:>10.4} {ctl_alpha:>10} {:>10} {:>8.3}x",
        "none", "-", ctl_bpb / base_bpb
    );
    println!(
        "  control: unreachable vigilance {} the no-orbit coder ({ctl_bpb:.6} vs {base_bpb:.6})",
        if clean { "reproduces" } else { "DOES NOT REPRODUCE" }
    );
    assert!(
        clean,
        "with vigilance above one no two bytes can share a class, so the learned-orbit coder \
         must give exactly the plain one's bits per byte; it did not, so the folding itself moves \
         a reading and no row below is worth anything"
    );
    for rho in [0.95f32, 0.9, 0.8, 0.7, 0.5, 0.3] {
        let (bits, alpha) = ppm_byte_bits_learned_orbit(input, order, rho);
        let bpb = bits / n;
        println!("{rho:>10.2} {bpb:>10.4} {alpha:>10} {:>10} {:>8.3}x", "-", bpb / base_bpb);
    }
    println!();
    println!("the classes column is the size of the context alphabet, out of 256.");
}
