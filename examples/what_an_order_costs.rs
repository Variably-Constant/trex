//! Both ends of the seam field's context order, read from one run.
//!
//! The order decides how many context models the counting pass builds, and the
//! counting pass is `the seam field: counting the contexts` - 305.566 ms a scan
//! over `benches/engine_surface`, the largest leaf in the engine's phase table
//! that is neither an attempt nor the spectral field. Bumps go as `n * k * 2`,
//! so the order is the one term in that product anybody can move, and dropping
//! the default from three to two removes an order's whole work.
//!
//! What it costs to drop is segmentation quality, because the order is what the
//! backoff reads: a high-order context that has been seen enough is a sharper
//! estimate than a short one, and without it the field falls back sooner. That
//! is a trade rather than a saving, and a trade cannot be taken on one of its
//! two numbers. So this prints them together - boundary recovery against the
//! true word boundaries of the despaced text, beside the milliseconds the
//! counting took and the bumps it made.
//!
//! The reading is only as general as the corpus it is given, and it is given
//! one rather than defaulting to any: a table labeled with a corpus the caller
//! did not choose is the wrong answer arriving as the right one.
//!
//! The `ns a bump` column is for reading rows against each other in one table
//! and not against another table's. A despaced document is tens of kilobytes
//! where the engine's own surface is seven megabytes, so the models here are
//! built and thrown away over far fewer bumps and every bump carries a share of
//! that. It reads tens of nanoseconds here where the same counting over
//! `benches/engine_surface` reads about six.
//!
//! Run: `cargo run --release --example what_an_order_costs -- <corpus> [bytes]`

use std::env;
use std::fs;

use trex::seam::{SeamConfig, analyze_with, despaced_text, recovery};

/// The order the crate ships, which every row is read against.
const SHIPPED: usize = 3;

/// The named phase's milliseconds, or zero where the run did not open it.
fn phase_ms(phases: &[(&'static str, u64, std::time::Duration)], name: &str) -> f64 {
    phases.iter().find(|(n, _, _)| *n == name).map_or(0.0, |(_, _, d)| d.as_secs_f64() * 1e3)
}

/// The named count, or zero where the run did not report it.
fn count_of(counts: &[(&'static str, u64)], name: &str) -> u64 {
    counts.iter().find(|(n, _)| *n == name).map_or(0, |(_, v)| *v)
}

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: what_an_order_costs <corpus-file> [bytes]");
        std::process::exit(2);
    });
    // Refused rather than defaulted, as `seam_vigilance` refuses it: a sweep
    // run at a size the caller did not ask for is labeled with the size they
    // did.
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

    let (despaced, truth) = despaced_text(&raw, cap);
    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("{path}: {} despaced bytes, {} true boundaries", despaced.len(), truth.len());

    // Every row is read before any is printed, because each is shown against
    // the shipped order and that row is not the first one taken.
    let rows: Vec<Row> = (1..=5usize).map(|order| read(&despaced, &truth, order)).collect();
    let shipped = rows
        .iter()
        .find(|r| r.order == SHIPPED)
        .unwrap_or_else(|| panic!("the sweep covers the shipped order {SHIPPED}"));

    println!(
        "\n{:>6} {:>8} {:>8} {:>8} {:>9} {:>9} {:>12} {:>11} {:>10}",
        "order", "p", "r", "f1", "f1 vs", "ms vs", "bumps", "counting ms", "ns a bump"
    );
    for r in &rows {
        let ns = if r.bumps > 0 { r.ms * 1e6 / r.bumps as f64 } else { 0.0 };
        println!(
            "{:>6} {:>8.4} {:>8.4} {:>8.4} {:>9.4} {:>9.4} {:>12} {:>11.3} {:>10.2}",
            r.order,
            r.precision,
            r.recall,
            r.f1,
            f64::from(r.f1) / f64::from(shipped.f1),
            r.ms / shipped.ms,
            r.bumps,
            r.ms,
            ns
        );
    }

    println!(
        "\nthe shipped order is {SHIPPED}, at f1 {:.4} and {:.3} ms counting",
        shipped.f1, shipped.ms
    );
    println!("a row is worth taking only where both of its columns are, and neither alone");
}

/// One order's two ends: what it recovered and what its counting cost.
struct Row {
    order: usize,
    precision: f32,
    recall: f32,
    f1: f32,
    bumps: u64,
    ms: f64,
}

/// Segment `despaced` at `order` once, watched, and score it against `truth`.
fn read(despaced: &[u8], truth: &[usize], order: usize) -> Row {
    let cfg = SeamConfig { order, ..SeamConfig::default() };
    // An unwatched pass first: the models allocate, and a share of a first call
    // is the allocator rather than the counting.
    drop(analyze_with(despaced, &cfg));

    trex::trace::record();
    drop(trex::trace::take_phases());
    drop(trex::trace::take_counts());
    let field = analyze_with(despaced, &cfg);
    let phases = trex::trace::take_phases();
    let counts = trex::trace::take_counts();
    trex::trace::stop();

    let r = recovery(&field.cuts, truth, 2);
    Row {
        order,
        precision: r.precision,
        recall: r.recall,
        f1: r.f1,
        bumps: count_of(&counts, "the seam field: bumps into a direct table")
            + count_of(&counts, "the seam field: bumps into a map"),
        ms: phase_ms(&phases, "the seam field: counting the contexts"),
    }
}
