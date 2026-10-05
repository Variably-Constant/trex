//! The significant lex on held buffers, timed per call, for comparing two
//! builds of this crate. One process times each cell once: after a warm call
//! it runs as many calls as fill about sixty milliseconds, and prints the
//! median, smallest and largest milliseconds per call beside the token count
//! and FNV-1a hashes of the kinds and spans lexed, so two builds that lex
//! differently are told apart before their times are compared.
//!
//! Cells: tag-number lines at 16,384, 131,072, 524,288 and 2,097,152 pairs,
//! the corpus `benches/gpu_throughput.rs --phases` builds, then each corpus
//! file named on the command line, whole, under its file stem.
//!
//! Run: `cargo run --release --example sig_lex_timing -- [corpus ...]`.

use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use trex::parallel_lex::{SignificantWorkspace, lex_significant_parallel_into};

const TAG_PAIRS: [usize; 4] = [16_384, 131_072, 524_288, 2_097_152];
const ROUND_MS: f64 = 60.0;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

/// `pairs` lines of a word and a number: the corpus
/// `benches/gpu_throughput.rs --phases` builds.
fn tag_pairs(pairs: usize) -> Vec<u8> {
    let mut input = String::new();
    for i in 0..pairs {
        input.push_str("tag ");
        input.push_str(&(i % 1000).to_string());
        input.push('\n');
    }
    input.into_bytes()
}

/// FNV-1a over `bytes`, continuing from `h`.
fn fnv1a(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

fn main() {
    let mut cells: Vec<(String, Vec<u8>)> =
        TAG_PAIRS.iter().map(|&pairs| (format!("tag-lines-{pairs}"), tag_pairs(pairs))).collect();
    for path in std::env::args().skip(1) {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        };
        let stem = match Path::new(&path).file_stem() {
            Some(s) => s.to_string_lossy().into_owned(),
            None => {
                eprintln!("{path} names no file");
                std::process::exit(1);
            }
        };
        cells.push((stem, bytes));
    }

    let mut ws = SignificantWorkspace::default();
    for (cell, input) in &cells {
        lex_significant_parallel_into(input, &mut ws);
        let tokens = ws.kinds.len();
        let kinds_hash = ws.kinds.iter().fold(FNV_OFFSET, |h, k| fnv1a(h, &k.to_le_bytes()));
        let spans_hash =
            ws.spans.iter().fold(FNV_OFFSET, |h, &(s, e)| fnv1a(fnv1a(h, &s.to_le_bytes()), &e.to_le_bytes()));

        let mut ms = Vec::new();
        let round = Instant::now();
        while ms.is_empty() || round.elapsed().as_secs_f64() * 1e3 < ROUND_MS {
            let t = Instant::now();
            lex_significant_parallel_into(input, &mut ws);
            black_box(ws.kinds.len());
            ms.push(t.elapsed().as_secs_f64() * 1e3);
        }
        ms.sort_by(f64::total_cmp);
        println!(
            "size={} cell={cell} calls={} ms={:.4} lo={:.4} hi={:.4} tokens={tokens} kinds_hash={kinds_hash:016x} spans_hash={spans_hash:016x}",
            input.len(),
            ms.len(),
            ms[ms.len() / 2],
            ms[0],
            ms[ms.len() - 1]
        );
    }
}
