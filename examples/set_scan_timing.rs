//! Timing of scans the set engine runs, for one build: `trex::scan` over a
//! balanced group and a field anchor, each with a register bind and without,
//! at three sizes. Each cell prints its median milliseconds per call with the
//! fastest and slowest round, and hashes of its spans and of the captures they
//! resolve to, so two builds of this example run alternately compare both the
//! time and the result.
//!
//! Run: `cargo run --release --example set_scan_timing -- <code corpus> [rounds]`.
//! The balanced cells scan the corpus repeated to each size; the field cells
//! scan rows of four comma-separated fields.

use std::hint::black_box;
use std::time::Instant;

use trex::{captures, parse, scan};

const SIZES: [usize; 3] = [8 * 1024, 256 * 1024, 4 * 1024 * 1024];
const ROUNDS: usize = 7;
const ROUND_MS: f64 = 60.0;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

/// A cell's name, its pattern, and whether it scans the code corpus rather
/// than the comma-separated rows.
const CELLS: [(&str, &str, bool); 4] = [
    ("balanced-bind", "\\B:x", true),
    ("balanced", "\\B", true),
    ("field-bind", "@3 \\W:x", false),
    ("field", "@3 \\W", false),
];

/// `code` repeated and cut to `len` bytes.
fn repeated(code: &[u8], len: usize) -> Vec<u8> {
    code.iter().copied().cycle().take(len).collect()
}

/// Rows of four comma-separated fields, cut to `len` bytes.
fn rows(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 32);
    let mut i = 0usize;
    while out.len() < len {
        out.extend_from_slice(format!("alpha,{i},gamma,delta\n").as_bytes());
        i += 1;
    }
    out.truncate(len);
    out
}

/// FNV-1a over `bytes`, continuing from `h`.
fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// The middle sample, sorting `v`.
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: set_scan_timing <code corpus> [rounds]");
        std::process::exit(2);
    };
    let code = match std::fs::read(&path) {
        Ok(b) if !b.is_empty() => b,
        Ok(_) => {
            eprintln!("{path} is empty");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let rounds = match std::env::args().nth(2) {
        None => ROUNDS,
        Some(v) => match v.parse::<usize>() {
            Ok(r) if r > 0 => r,
            Ok(_) => {
                eprintln!("rounds must be at least 1");
                std::process::exit(2);
            }
            Err(e) => {
                eprintln!("rounds {v:?} is not a count: {e}");
                std::process::exit(2);
            }
        },
    };
    for (name, src, over_code) in CELLS {
        println!("cell {name} is {src} over {}", if over_code { "the code corpus" } else { "field rows" });
    }
    for len in SIZES {
        for (name, src, over_code) in CELLS {
            let input = if over_code { repeated(&code, len) } else { rows(len) };
            let p = parse(src).unwrap_or_else(|e| panic!("{src} does not parse: {e:?}"));
            let spans = scan(&p, &input);
            assert!(!spans.is_empty(), "{name} matches nothing at {len} bytes, so it would time no work");
            let span_hash = spans
                .iter()
                .fold(FNV_OFFSET, |h, s| fnv(&s.end.to_le_bytes(), fnv(&s.start.to_le_bytes(), h)));
            let capture_hash = captures(&p, &input, &spans).iter().fold(FNV_OFFSET, |h, m| {
                m.names().iter().zip(m.captures()).fold(h, |h, (reg, s)| {
                    fnv(&input[s.range()], fnv(reg.as_bytes(), h))
                })
            });

            // Calls per round, from one warm call.
            let t = Instant::now();
            black_box(scan(&p, &input));
            let warm_ms = t.elapsed().as_secs_f64() * 1e3;
            let calls = ((ROUND_MS / warm_ms.max(1e-3)).ceil() as usize).clamp(1, 5_000);

            let mut ms = Vec::with_capacity(rounds);
            for _ in 0..rounds {
                let t = Instant::now();
                for _ in 0..calls {
                    black_box(scan(&p, &input));
                }
                ms.push(t.elapsed().as_secs_f64() * 1e3 / calls as f64);
            }
            let lo = ms.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = ms.iter().copied().fold(0.0, f64::max);
            println!(
                "size={len} cell={name} calls={calls} ms={:.4} lo={lo:.4} hi={hi:.4} spans={} span_hash={span_hash:016x} capture_hash={capture_hash:016x}",
                median(&mut ms),
                spans.len()
            );
        }
    }
}
