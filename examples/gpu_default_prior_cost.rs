//! What does the default device prior cost a process to build?
//!
//! The first device compress in a process projects the compiled-in byte-ngram
//! prior into the device table; every later call reuses it. The device context
//! and the kernel are loaded first by a compress with no prior, so the first
//! timed call pays the build and nothing else a later call does not. The
//! difference between that call and the median of five later ones is the
//! build, and every later call must code to the first call's bits.
//!
//! One process measures one first call, so a script runs this several times.
//!
//! Run on a CUDA host: `cargo run --release --features gpu --example
//! gpu_default_prior_cost -- <corpus> [bytes]`

use std::env;
use std::fs;
use std::time::Instant;

use trex::gpu::{compress_gpu, compress_gpu_with_prior};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: gpu_default_prior_cost <corpus> [bytes]");
        std::process::exit(2);
    };
    let bytes: usize = match args.get(1) {
        None => 65_536,
        Some(s) => s.parse().unwrap_or_else(|e| panic!("bytes {s:?} does not parse: {e}")),
    };
    let raw = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    assert!(bytes <= raw.len(), "{path} holds {} bytes, fewer than {bytes}", raw.len());
    let input = &raw[..bytes];

    if compress_gpu_with_prior(input, &[0u16; 3], 0, false).is_none() {
        eprintln!("no usable device, or this build lacks the gpu feature");
        std::process::exit(1);
    }

    let t = Instant::now();
    let first = compress_gpu(input).expect("the device answered the warm-up");
    let first_ms = t.elapsed().as_secs_f64() * 1e3;

    let mut later = Vec::with_capacity(5);
    for _ in 0..5 {
        let t = Instant::now();
        let bits = compress_gpu(input).expect("the device answered the first call");
        later.push(t.elapsed().as_secs_f64() * 1e3);
        assert!(bits == first, "a later call coded {bits} bits against the first call's {first}");
    }
    later.sort_by(f64::total_cmp);
    let median = later[later.len() / 2];
    println!(
        "{bytes} bytes: first call {first_ms:.1} ms, later calls median {median:.1} ms, default prior build {:.1} ms",
        first_ms - median
    );
}
