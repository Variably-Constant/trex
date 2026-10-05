//! One workload, repeated, for a sampling profiler to look at.
//!
//! The scaling benches run a ladder of sizes and both lex routes, which is
//! what a timing needs and the opposite of what a profile needs: samples
//! spread across everything say little about anything. This runs a single
//! workload chosen by its first argument for long enough to collect a few
//! thousand samples, and nothing else.
//!
//! A second argument names a file to use in place of the generated corpus,
//! its first 4MB.
//!
//! ```text
//!   profile_target lex       serial lex of a 4MB corpus
//!   profile_target lex-held  the same lex into a buffer kept across calls
//!   profile_target lex-par   the pool's lexer on the device phase sweep's
//!                            largest corpus (16.5 MB of tag-number pairs)
//!   profile_target lex-sig   the device path's fused lex on that corpus
//!   profile_target lex-sig-held  the same into a workspace kept across calls
//!   profile_target scan      plain \W scan, lex included
//!   profile_target match   the single-pass engine over pre-lexed tokens
//!   profile_target seam    byte-grain seam, both fusions
//!   profile_target axes    the token- and supertoken-grain readings
//! ```

use std::hint::black_box;
use std::time::Instant;

fn corpus(bytes_wanted: usize) -> Vec<u8> {
    let mut s = String::new();
    let mut i = 0usize;
    while s.len() < bytes_wanted {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
        i += 1;
    }
    s.truncate(bytes_wanted);
    s.into_bytes()
}

/// The corpus `benches/gpu_throughput.rs --phases` builds: `pairs` lines of
/// a word and a number.
fn tag_pairs(pairs: usize) -> Vec<u8> {
    let mut input = String::new();
    for i in 0..pairs {
        input.push_str("tag ");
        input.push_str(&(i % 1000).to_string());
        input.push('\n');
    }
    input.into_bytes()
}

/// Repeat `f` until about three seconds have gone by, so the sample count is
/// set by wall time rather than by how fast the workload happens to be.
fn run_for(secs: f64, mut f: impl FnMut()) -> u32 {
    let t0 = Instant::now();
    let mut iters = 0u32;
    while t0.elapsed().as_secs_f64() < secs {
        f();
        iters += 1;
    }
    iters
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "lex".to_string());
    // A second argument names a file whose first 4MB replace the generated
    // corpus, so a workload can be profiled on real text.
    let bytes = match std::env::args().nth(2) {
        // The parallel-lex workloads read the device phase sweep's largest
        // corpus, so a profile of them is read against that sweep's lex column.
        None if which.starts_with("lex-par") || which.starts_with("lex-sig") => tag_pairs(2_097_152),
        None => corpus(4 * 1024 * 1024),
        Some(path) => match std::fs::read(&path) {
            Ok(mut b) => {
                b.truncate(4 * 1024 * 1024);
                b
            }
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        },
    };
    let iters = match which.as_str() {
        "lex" => run_for(3.0, || {
            black_box(trex::lexer::lex(&bytes));
        }),
        "lex-held" => {
            // The same lex into one buffer kept across calls, so the share of
            // a lex that is the fresh output buffer's page faults is the
            // difference from `lex`.
            let mut held = Vec::new();
            run_for(3.0, || {
                trex::lexer::lex_into(&bytes, &mut held);
                black_box(held.len());
            })
        }
        "lex-par" => {
            // The pool's lexer, every thread sampled.
            run_for(3.0, || {
                black_box(trex::parallel_lex::lex_parallel(&bytes));
            })
        }
        "lex-sig" => {
            // The device path's fused lex into fresh buffers each call.
            run_for(3.0, || {
                black_box(trex::parallel_lex::lex_significant_parallel(&bytes));
            })
        }
        "lex-sig-held" => {
            // The fused lex into a workspace kept across calls, so the share
            // of a lex that is fresh pages is the difference from `lex-sig`.
            let mut ws = trex::parallel_lex::SignificantWorkspace::default();
            run_for(3.0, || {
                trex::parallel_lex::lex_significant_parallel_into(&bytes, &mut ws);
                black_box(ws.kinds.len());
            })
        }
        "scan" => {
            let p = trex::parse("\\W").expect("pattern parses");
            run_for(3.0, || {
                black_box(trex::scan(&p, &bytes));
            })
        }
        "match" => {
            let p = trex::parse("\\W \"=\"").expect("pattern parses");
            let toks = trex::lexer::lex(&bytes);
            run_for(3.0, || {
                black_box(trex::nfa::scan_nfa_over(&p, &bytes, &toks));
            })
        }
        "seam" => {
            // A quarter of the corpus: the byte-grain seam is the slowest
            // thing here and a full 4MB pass would give one sample-set per
            // several seconds.
            let slice = &bytes[..bytes.len() / 4];
            run_for(3.0, || {
                black_box(trex::seam::analyze(slice));
            })
        }
        "axes" => {
            let toks = trex::lexer::lex(&bytes);
            let units = trex::supertoken::supertokens_from(&toks, &bytes);
            let seam_cfg = trex::seam::SeamConfig::default();
            let obs_cfg = trex::observation::ObservationConfig::default();
            run_for(3.0, || {
                black_box(trex::seam::analyze_tokens(&toks, &seam_cfg));
                black_box(trex::observation::contested_tokens(&toks, &obs_cfg));
                black_box(trex::resonator::over_tokens(&toks, 32));
                black_box(trex::resonator::over_supertokens(&units, 32));
                black_box(trex::supertoken::SuperContext::build(&toks, &bytes));
            })
        }
        other => {
            eprintln!(
                "unknown workload {other:?}; use lex, lex-held, lex-par, lex-sig, lex-sig-held, scan, match, seam or axes"
            );
            std::process::exit(2);
        }
    };
    println!("{which}: {iters} iterations over {} bytes", bytes.len());
}
