//! Does a CPU scan gain from lexing into buffers kept across calls?
//!
//! Four arms scan one input for one pattern. The held arm is `trex::scan`,
//! which lexes into the calling thread's kept token workspace. The fresh arm
//! lexes into buffers of its own with `lex_parallel` and matches with
//! `nfa::scan_nfa_over`, the engine `trex::scan` routes these patterns to, so
//! the two differ only in where the tokens are written. The other two rerun
//! each of them, and a rerun's ratio against its arm must read 1.00x. Each
//! arm has a control of its own because only the fresh arm pays allocation
//! and first-touch page faults, so a neighbour moves the two by different
//! amounts and a repeat of one arm cannot speak for the other.
//!
//! Correctness first: both arms must return the same spans. Arm order rotates
//! every round, an arm's round is as many calls as fill about sixty
//! milliseconds, and each ratio is the median of per-round ratios with the
//! smallest and largest beside it. A fresh/held ratio above 1 means the held
//! buffers are faster.
//!
//! Run: `cargo run --release --example held_scan_ab -- [corpus [rounds]]`.
//! Without a corpus it scans the device phase sweep's tag-number lines at five
//! sizes; with one, the corpus's first 4 MB and the whole of it, over
//! `rounds` rounds when given.

use std::hint::black_box;
use std::time::Instant;

use trex::nfa::scan_nfa_over;
use trex::parallel_lex::lex_parallel;
use trex::{parse, scan};

const PATTERNS: [&str; 2] = ["\\W \\N", "\\W"];
const ROUNDS: usize = 11;
const ROUND_MS: f64 = 60.0;

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

/// The middle sample, sorting `v`.
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    // A second argument sets the rounds, for an input whose round is one call.
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
    let inputs: Vec<(String, Vec<u8>)> = match std::env::args().nth(1) {
        None => [8_192usize, 16_384, 131_072, 524_288, 2_097_152]
            .iter()
            .map(|&pairs| (format!("{pairs} tag lines"), tag_pairs(pairs)))
            .collect(),
        Some(path) => {
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("cannot read {path}: {e}");
                    std::process::exit(1);
                }
            };
            let head = bytes[..bytes.len().min(4 * 1024 * 1024)].to_vec();
            vec![(format!("{path}, first {} bytes", head.len()), head), (format!("{path}, whole"), bytes)]
        }
    };

    println!(
        "{:<8} {:>11} {:>7} {:>11} {:>15} {:>10} {:>10}",
        "pattern", "bytes", "calls", "fresh/held", "spread", "fresh ms", "held ms"
    );
    for (name, input) in &inputs {
        println!("{name}");
        for src in PATTERNS {
            let p = parse(src).unwrap_or_else(|e| panic!("{src} does not parse: {e:?}"));
            let held_spans = scan(&p, input);
            let fresh_spans = scan_nfa_over(&p, input, &lex_parallel(input))
                .expect("the single-pass engine takes this pattern");
            assert!(!held_spans.is_empty(), "{src} matches nothing in {name}, so the arms would time no work");
            assert_eq!(held_spans, fresh_spans, "{src}: the held and fresh scans differ on {name}");

            // Calls per round, from one warm call.
            let t = Instant::now();
            black_box(scan(&p, input));
            let warm_ms = t.elapsed().as_secs_f64() * 1e3;
            let calls = ((ROUND_MS / warm_ms.max(1e-3)).ceil() as usize).clamp(1, 5_000);

            let fresh = || {
                let t = Instant::now();
                for _ in 0..calls {
                    let toks = lex_parallel(input);
                    black_box(scan_nfa_over(&p, input, &toks));
                }
                t.elapsed().as_secs_f64() * 1e3 / calls as f64
            };
            let held = || {
                let t = Instant::now();
                for _ in 0..calls {
                    black_box(scan(&p, input));
                }
                t.elapsed().as_secs_f64() * 1e3 / calls as f64
            };
            let arms: [&dyn Fn() -> f64; 4] = [&fresh, &held, &fresh, &held];

            let mut ratio = Vec::new();
            let (mut fresh_control, mut held_control) = (Vec::new(), Vec::new());
            let (mut fresh_ms, mut held_ms) = (Vec::new(), Vec::new());
            for round in 0..rounds {
                let mut ms = [0.0f64; 4];
                for i in 0..4 {
                    let arm = (i + round) % 4;
                    ms[arm] = arms[arm]();
                }
                ratio.push(ms[0] / ms[1]);
                fresh_control.push(ms[0] / ms[2]);
                held_control.push(ms[1] / ms[3]);
                fresh_ms.push(ms[0]);
                held_ms.push(ms[1]);
            }
            let lo = ratio.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = ratio.iter().copied().fold(0.0, f64::max);
            println!(
                "{src:<8} {:>11} {calls:>7} {:>10.3}x {:>7.3}..{:<7.3} {:>10.3} {:>10.3}   control fresh {:.3}x held {:.3}x",
                input.len(),
                median(&mut ratio),
                lo,
                hi,
                median(&mut fresh_ms),
                median(&mut held_ms),
                median(&mut fresh_control),
                median(&mut held_control)
            );
        }
    }
}
