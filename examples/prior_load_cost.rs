//! What does loading the prior cost, in time and in memory?
//!
//! The shipped prior is a brotli blob that `load_byte_ngram` decodes and
//! expands into one bit-tree count table per order. The expansion is the cost
//! this measures: every row becomes a hash entry, and the blob's compressed
//! bytes bear no relation to what the expansion occupies.
//!
//! Time is reported here. Peak working set is reported by the caller, because
//! a process cannot watch its own high-water mark cheaply on Windows; the
//! wrapper reads `PeakWorkingSet64` after this exits.
//!
//! Prints the row count per order so the expansion factor can be computed
//! against the blob rather than assumed.
//!
//! Run: `cargo run --release --example prior_load_cost -- [blob]`

use std::env;
use std::fs;
use std::time::Instant;

use trex::seam::{baked_model, load_byte_ngram};

fn main() {
    match env::args().nth(1) {
        Some(path) => {
            let blob = match fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("cannot read {path}: {e}");
                    std::process::exit(1);
                }
            };
            let t = Instant::now();
            let tables = load_byte_ngram(&blob);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            report(&path, blob.len(), &tables, ms);
        }
        None => {
            // The compiled-in blob decodes on first use, so the clock around
            // the first call is the decode and expansion.
            let t = Instant::now();
            let tables = baked_model();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            report("compiled-in", 0, tables, ms);
        }
    }
}

fn report(name: &str, blob_bytes: usize, tables: &[trex::seam::PreloadTable], ms: f64) {
    let rows: usize = tables.iter().map(|(_, m)| m.len()).sum();
    println!("prior: {name}");
    println!("blob bytes: {blob_bytes}");
    println!("load+expand: {ms:.1} ms");
    println!("orders: {}", tables.len());
    for (k, m) in tables {
        println!("  order {k:>2}: {:>12} rows", m.len());
    }
    println!("total rows: {rows}");
    // A row is a u64 key and a (u32, u32) value, so sixteen bytes of payload
    // before the table's own load factor and control bytes.
    println!("payload floor: {} bytes ({} MB)", rows * 16, rows * 16 / (1 << 20));
}
