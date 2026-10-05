//! What does merging resonant contexts cost, and what does it save?
//!
//! seam keeps one row per distinct context per order. Each row is a
//! distribution over followers, which is the same object an Adaptive
//! Resonance class holds as a prototype, so contexts that predict the same
//! thing can share one. Vigilance is the floor on how much of a context's
//! own evidence its prototype has to reproduce.
//!
//! The sweep reports classes kept against contexts seen, and boundary
//! recovery against the true word boundaries of the same despaced text, so
//! the two ends of the trade are read from one run.
//!
//! Vigilance 1.0 admits only exactly-agreeing distributions, so its row has
//! to reproduce the unclustered F1. A run where it does not has changed
//! something other than sharing, and no other row in the table is worth
//! reading.
//!
//! Run: `cargo run --release --example seam_vigilance -- <corpus>`

use std::env;
use std::fs;

use trex::seam::{SeamConfig, analyze_with, despaced_text, recovery};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: seam_vigilance <corpus-file> [bytes]");
        std::process::exit(2);
    });
    // An unreadable byte count is refused rather than defaulted. Silently
    // falling back would run the sweep at a size the caller did not ask for
    // and label the table with it, which is the same shape as every other
    // measurement failure here: the wrong answer arrives looking like the
    // right one.
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
    let (despaced, truth) = despaced_text(&raw, cap);
    println!("{path}: {} despaced bytes, {} true boundaries", despaced.len(), truth.len());
    println!("{:>10} {:>8} {:>8} {:>8} {:>9} {:>9}", "vigilance", "p", "r", "f1", "cuts", "vs off");

    // The unclustered run first: every later row is read against it, and the
    // 1.0 row has to reproduce it.
    let base = analyze_with(&despaced, &SeamConfig::default());
    let b = recovery(&base.cuts, &truth, 2);
    println!(
        "{:>10} {:>8.4} {:>8.4} {:>8.4} {:>9} {:>9}",
        "off", b.precision, b.recall, b.f1, b.predicted, "-"
    );

    for rho in [1.0f32, 0.99, 0.95, 0.9, 0.8, 0.7, 0.5, 0.3] {
        let cfg = SeamConfig { vigilance: Some(rho), ..SeamConfig::default() };
        let field = analyze_with(&despaced, &cfg);
        let r = recovery(&field.cuts, &truth, 2);
        println!(
            "{rho:>10.2} {:>8.4} {:>8.4} {:>8.4} {:>9} {:>8.3}x",
            r.precision,
            r.recall,
            r.f1,
            r.predicted,
            f64::from(r.f1) / f64::from(b.f1)
        );
        if (rho - 1.0).abs() < f32::EPSILON {
            let exact = r.f1 == b.f1 && r.predicted == b.predicted;
            println!(
                "  control: vigilance 1.0 {} the unclustered run \
                 (f1 {:.6} vs {:.6}, cuts {} vs {})",
                if exact { "reproduces" } else { "DOES NOT REPRODUCE" },
                r.f1,
                b.f1,
                r.predicted,
                b.predicted
            );
            assert!(
                exact,
                "vigilance 1.0 admits only identical distributions, so it must merge nothing \
                 that changes a reading; it moved one, which means the clustering is doing \
                 something other than sharing and no other row in this table is worth reading"
            );
        }
    }
    println!();
    println!("set TREX_SEAM_CLUSTER_REPORT=1 for the class and context counts per run.");
}
