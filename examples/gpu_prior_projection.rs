//! Does the device coder gain from a prior, and seeded how?
//!
//! The device coder's default prior is the compiled-in byte-ngram prior, trained
//! on 63 MB across eight languages and source code, projected into its table.
//! This projects that blob at several table sizes and slot caps and codes every
//! holdout with each, beside no prior at all and the default.
//!
//! Every projected column, marked `+p`, predicts from the prior when a context
//! model's slot misses, as the CPU coder does and as the default does.
//!
//! The control comes first: the default device call, run twice on the same
//! input, must give the same bits, or no difference between columns means
//! anything.
//!
//! Run on a CUDA host: `cargo run --release --features gpu --example
//! gpu_prior_projection -- <baked_model.br> <holdout>...`

use std::env;
use std::fs;
use std::time::Instant;

use trex::gpu::{compress_gpu, compress_gpu_with_prior, project_device_prior, GpuPrior};

fn read(path: &str) -> Vec<u8> {
    match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: gpu_prior_projection <baked_model.br> <holdout>...");
        std::process::exit(2);
    }
    let blob = read(&args[0]);
    let holdouts: Vec<(String, Vec<u8>)> = args[1..].iter().map(|p| (p.clone(), read(p))).collect();

    let (first_name, first) = (&holdouts[0].0, &holdouts[0].1);
    let Some(once) = compress_gpu(first) else {
        eprintln!("no usable device, or this build lacks the gpu feature");
        std::process::exit(1);
    };
    let twice = compress_gpu(first).expect("the device answered the first call");
    assert_eq!(once.to_bits(), twice.to_bits(), "the device coded {first_name} to {once} bits, then {twice}");
    println!("control: the default device prior codes {first_name} to {once} bits on both calls");

    let mut columns: Vec<(String, Vec<Option<f64>>)> = vec![
        (
            "none".to_string(),
            holdouts.iter().map(|(_, h)| compress_gpu_with_prior(h, &[0u16; 3], 0, false)).collect(),
        ),
        ("default".to_string(), holdouts.iter().map(|(_, h)| compress_gpu(h)).collect()),
    ];

    for (bits, cap) in [(22usize, 8u64), (22, 4), (23, 8), (23, 4), (24, 8), (24, 4)] {
        let t = Instant::now();
        let Some((table, stats)) = project_device_prior(&blob, bits, cap) else {
            eprintln!("this build cannot project a device prior");
            std::process::exit(1);
        };
        println!(
            "projected at {bits} bits, cap {cap}: {} bytes in {:.1} s",
            table.len() * 2,
            t.elapsed().as_secs_f64()
        );
        for s in &stats {
            println!(
                "  model {:>2}: {:>10} nodes, {:>10} placed, {:.4} of the count kept",
                s.model, s.nodes, s.placed, s.mass_kept
            );
        }
        let Some(held) = GpuPrior::upload(&table, bits, true) else {
            eprintln!("the device would not hold a {bits}-bit prior");
            std::process::exit(1);
        };
        let cells = holdouts.iter().map(|(_, h)| held.compress(h)).collect();
        columns.push((format!("p{bits}/{cap}+p"), cells));
    }

    println!();
    print!("{:<22}", "bits per byte");
    for (name, _) in &columns {
        print!(" {name:>10}");
    }
    println!();
    for (i, (path, h)) in holdouts.iter().enumerate() {
        let name = path.rsplit(['\\', '/']).next().expect("a split yields at least one piece");
        print!("{name:<22}");
        for (_, cells) in &columns {
            match cells[i] {
                Some(bits) => print!(" {:>10.4}", bits / h.len() as f64),
                None => print!(" {:>10}", "failed"),
            }
        }
        println!();
    }
}
