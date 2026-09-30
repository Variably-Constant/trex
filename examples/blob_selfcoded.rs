//! Does trex's own coder beat brotli on trex's own prior?
//!
//! The blob ships brotli-compressed. But the thing being compressed is highly
//! structured - contexts sorted by their bytes, then counts drawn from about a
//! hundred thousand distinct pairs - and a context-mixing coder is built for
//! exactly that shape, where a general-purpose compressor is not.
//!
//! Decode cost is what has always argued against it: trex's coder runs at a
//! fraction of brotli's speed, and a prior decoded on every invocation of a
//! one-shot CLI cannot afford it. That objection weakens if the blob is decoded
//! ONCE per machine into a mapped table and read from there afterwards, which
//! is what the mapped-prior work makes possible. Then the shipped artifact
//! wants the strongest compression available and the decode is amortized.
//!
//! Reported as bits per byte on a sample, against brotli's rate over the whole
//! blob. A sample is not the whole: the rate is measured on a prefix of the
//! decoded stream, which is the low orders, and those have shorter keys and
//! denser counts than orders six and eight. Treat the figure as indicative of
//! that region rather than of the blob.
//!
//! Run: `cargo run --release --example blob_selfcoded -- <blob> [sample-bytes]`

use std::env;
use std::fs;
use std::time::Instant;

use trex::seam::logistic_mix_bits;

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: blob_selfcoded <blob> [sample-bytes]");
        std::process::exit(2);
    };
    let sample: usize = match env::args().nth(2) {
        None => 2_097_152,
        Some(s) => match s.parse() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("second argument {s:?} is not a byte count: {e}");
                std::process::exit(2);
            }
        },
    };
    let blob = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };

    // The bytes brotli was given, recovered by undoing it.
    let mut raw = Vec::new();
    {
        let mut d = brotli::Decompressor::new(&blob[..], 4096);
        if let Err(e) = std::io::Read::read_to_end(&mut d, &mut raw) {
            eprintln!("cannot decompress {path}: {e}");
            std::process::exit(1);
        }
    }
    let brotli_bpb = 8.0 * blob.len() as f64 / raw.len() as f64;
    println!("{path}");
    println!("uncompressed {} bytes, brotli {} bytes", raw.len(), blob.len());
    println!("brotli over the whole blob: {brotli_bpb:.4} bits/byte");

    // Orders are serialized in MODEL_ORDERS sequence, so a prefix is the low
    // orders: short keys, dense counts. Orders six and eight hold most of the
    // rows and all the long keys, and they sit at the end. An offset is what
    // lets the same question be asked of that region rather than only of the
    // easy one.
    let offset: usize = match env::args().nth(3) {
        None => 0,
        Some(s) => match s.parse() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("third argument {s:?} is not a byte offset: {e}");
                std::process::exit(2);
            }
        },
    };
    let start = offset.min(raw.len());
    let n = sample.min(raw.len() - start);
    let input = &raw[start..start + n];
    println!();
    println!(
        "sample: {n} bytes at offset {start} ({:.0}% into the decoded stream)",
        100.0 * start as f64 / raw.len() as f64
    );

    // Both figures on the same bytes, so the comparison is not between a rate
    // over the whole blob and a rate over a prefix.
    let mut sample_br = Vec::new();
    {
        let mut w = brotli::CompressorWriter::new(&mut sample_br, 4096, 11, 24);
        std::io::Write::write_all(&mut w, input).expect("brotli encode");
    }
    let sample_br_bpb = 8.0 * sample_br.len() as f64 / n as f64;

    let t = Instant::now();
    let bits = logistic_mix_bits(input, true, &[], None);
    let secs = t.elapsed().as_secs_f64();
    let self_bpb = bits / n as f64;

    println!("{:>22} {:>12} {:>12}", "coder", "bits/byte", "bytes");
    println!("{:>22} {sample_br_bpb:>12.4} {:>12}", "brotli q11", sample_br.len());
    println!(
        "{:>22} {self_bpb:>12.4} {:>12}",
        "trex, no prior",
        (bits / 8.0).round() as u64
    );
    println!(
        "{:>22} {:>12.3} {:>12}",
        "ratio",
        self_bpb / sample_br_bpb,
        ""
    );
    println!();
    println!(
        "trex coded {n} bytes in {secs:.1}s, {:.2} MB/s",
        n as f64 / secs / (1 << 20) as f64
    );
    println!("Below 1.000 trex's coder is the stronger one on this data. The rate is");
    println!("what decides whether it can be afforded: a blob decoded once per machine");
    println!("into a mapped table can pay it, one decoded per invocation cannot.");
}
