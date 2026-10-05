//! What the fused significant lex spends its time on, on the corpus the full
//! comparison runs.
//!
//! Ranking that comparison by milliseconds rather than by ratio puts 91.9 of
//! the 244.5 ms trex loses in three bulk operations, and what a row costs above
//! the fused lex is what the engine costs - for a pattern that runs the fused
//! lex at all. Both terms have to be measured before either can be worked on,
//! and so does which of them a row is made of.
//!
//! The harness asks which rows the lex is under rather than assuming it. A
//! route that lexes a token's worth of bytes at each occurrence of a literal
//! never runs the whole-input lex, so subtracting the lex from its row would
//! report an engine cheaper than nothing, a negative share for a pattern the
//! lex is not under at all. A row names the engine that answered, and the
//! subtraction is made only where it means something.
//!
//! `benches/lex_phase_scaling` asks how each phase scales, over the token lex
//! and a synthetic megabyte; this asks where the time goes, over the fused lex
//! and the comparison's own 7.34 MB, so the shares can be read against the rows
//! they explain. `\W every match` is the fused lex plus one kind test a token,
//! which is why it is here as its own arm rather than as a reading of the lex.
//!
//! The phases, in the order the lex runs them:
//!
//!   boundaries    `safe_boundaries`, whose quoted-span prescan reads the whole
//!                 input before a chunk is dispatched
//!   blobs         `blob_runs_parallel`, the entropy prescan, likewise
//!                 whole-input, timed beside its serial form because the serial
//!                 lexer runs one too and only the difference is a cost the
//!                 parallel route uniquely carries
//!   leaves        the dispatched `lex_chunk_significant_into` over the chunks,
//!                 with the two prescans' results already in hand
//!   fused whole   `lex_significant_parts_held`, which is the three above plus
//!                 the dispatch itself
//!   kind route    `\W` through `trex::scan`, which is the fused whole plus one
//!                 kind test a token: the comparison's own row
//!
//! The three phases are timed separately and against the whole, so what the
//! subtraction leaves unattributed is printed rather than assumed.
//!
//! Run: `cargo run --release --example sig_lex_phases`.

use std::hint::black_box;
use std::time::Instant;

use trex::lexer::{Significant, blob_runs_parallel};
use trex::parallel_lex::{chunk_blobs, lex_significant_parts_held, safe_boundaries};

/// Statements of four shapes, the corpus `benches/vs_regex_full` builds.
fn corpus(statements: usize) -> String {
    let mut s = String::new();
    for i in 0..statements {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
    }
    s
}

/// The lexer's own bytes-per-token estimate, which sizes a chunk's buffer.
/// Private to the crate, so it is restated here; a buffer sized differently
/// would grow inside the timed leaves where the real lex's buffer does not.
const TOKEN_BYTES_ESTIMATE: usize = 2;

const ROUNDS: usize = 5;
const BUDGET_MS: f64 = 25.0;

/// Time `f` until `BUDGET_MS` has passed and at least five calls have run,
/// reporting the mean call and its answer - the comparison's own harness, so a
/// phase's milliseconds are comparable with the rows it explains.
fn time_budget(f: &mut dyn FnMut() -> usize) -> (f64, usize) {
    let n = f();
    let t0 = Instant::now();
    let mut iters = 0u32;
    loop {
        black_box(f());
        iters += 1;
        if iters >= 5 && t0.elapsed().as_secs_f64() * 1e3 >= BUDGET_MS {
            break;
        }
    }
    (t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters), n)
}

/// The middle of `v` and its spread, the largest reading over the smallest.
fn median_and_spread(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("a measured time is never NaN"));
    (v[v.len() / 2], v[v.len() - 1] / v[0])
}

/// Every arm timed once a round, in an order that rotates, so no arm always
/// runs first into a cold cache or last into a neighbor's burst. Returns each
/// arm's median in the order the arms were given.
///
/// The first arm is timed again at the end as a control: it is the same work
/// both times, so its drift is attributable to position in the run and to load,
/// and a run whose control drifts is a run whose arms moved whatever their own
/// spreads look like.
fn sweep(arms: &mut [(&str, &mut dyn FnMut() -> usize)]) -> Vec<f64> {
    let n = arms.len();
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); n];
    let mut answers: Vec<usize> = vec![0; n];
    for round in 0..ROUNDS {
        for step in 0..n {
            let i = (round + step) % n;
            let (t, got) = time_budget(&mut *arms[i].1);
            times[i].push(t);
            answers[i] = got;
        }
    }
    let (control, _) = time_budget(&mut *arms[0].1);

    println!("{:>16} {:>10} {:>9} {:>16}", "phase", "ms", "spread", "answer");
    let mut medians = Vec::with_capacity(n);
    for (i, (name, _)) in arms.iter().enumerate() {
        let (ms, spread) = median_and_spread(times[i].clone());
        medians.push(ms);
        println!("{name:>16} {ms:>10.4} {spread:>8.2}x {:>16}", answers[i]);
    }
    println!(
        "{:>16} {control:>10.4} {:>9} drift {:.3}x against the same work at the start of the run",
        "CONTROL", "", control / medians[0]
    );
    medians
}

fn main() {
    // The corpus is a parameter because a lex rate is a fact about the bytes
    // it was read over, and these two disagree by a factor of nearly three:
    // the generated statements lex at 2.763 GB/s and the crate's own source at
    // about 1.0. Four repeating statement shapes give the leaves phase a token
    // mix nothing wrote, so a share read only over them is the generator's
    // share. With no path named the statements are the corpus, and the header
    // says which was read.
    let (whose, text) = match std::env::args().nth(1) {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(bytes) => (path, bytes),
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        },
        None => ("the generated statements".to_string(), corpus(200_000)),
    };
    let input = text.as_bytes();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let bounds = safe_boundaries(input, cores * 4);
    let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
    let blobs = blob_runs_parallel(input);
    let tokens =
        lex_significant_parts_held(input, |parts| parts.iter().map(|p| p.kinds.len()).sum::<usize>());
    let kinds = trex::parse("\\W").expect("a kind atom parses");
    // The three patterns holding 208 of the comparison's 244 ms of loss. No
    // byte route takes any of them, so each runs the fused lex above and then
    // the single-pass engine over every anchor in it, and the difference
    // between their row and the lex is what the engine costs.
    let no_route: Vec<_> = ["\"let\" \\W \"=\"", "\"let\" \\W:v \"=\"", "\\W:name \"=\""]
        .iter()
        .map(|src| trex::parse(src).expect("the comparison's own pattern parses"))
        .collect();

    println!("trex {}", trex::version());
    println!(
        "corpus {whose}, {} bytes, {tokens} significant tokens, {cores} cores, {} chunks",
        input.len(),
        ranges.len()
    );
    println!(
        "the rows this explains: `\\W every match` at 6.352 ms, and the three patterns below, \
         which hold 208 of the comparison's 244 ms of loss"
    );

    // The chunks' own blob tables and buffers, built once: the real path keeps
    // both across calls in the thread's held workspace, so building them inside
    // the timed arm would add to the leaves an allocation the lex does not make.
    let chunk_tables: Vec<Vec<(usize, usize)>> =
        ranges.iter().map(|&(s, e)| chunk_blobs(&blobs, s, e)).collect();
    let mut parts: Vec<Significant> = ranges
        .iter()
        .map(|&(s, e)| Significant::with_base(s, (e - s) / TOKEN_BYTES_ESTIMATE))
        .collect();

    let mut boundaries_arm = || safe_boundaries(input, cores * 4).len();
    let mut blobs_par_arm = || blob_runs_parallel(input).len();
    let mut blobs_ser_arm = || trex::lexer::blob_runs(input).len();
    let mut leaves_arm = || {
        use flynnel::JobPlan;
        use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;
        let per_chunk_ns =
            ((input.len() / ranges.len().max(1)) as u64 * 6).min(u64::from(u32::MAX)) as u32;
        let plan = JobPlan::new(0, ranges.len() as u32)
            .with_leaf_shape(flynnel::LeafShape::Streaming)
            .with_estimated_per_item_ns(per_chunk_ns);
        for_each_chunk_indexed_min_leaf(&plan, parts.as_mut_slice(), 1, |start, slots| {
            for (i, slot) in slots.iter_mut().enumerate() {
                let (s, e) = ranges[start + i];
                slot.reset(s);
                trex::lexer::lex_chunk_significant_into(&input[s..e], &chunk_tables[start + i], slot);
            }
        });
        parts.iter().map(|p| p.kinds.len()).sum::<usize>()
    };
    let mut whole_arm =
        || lex_significant_parts_held(input, |p| p.iter().map(|q| q.kinds.len()).sum::<usize>());
    let mut route_arm = || trex::scan(&kinds, input).len();
    let mut plain_arm = || trex::scan(&no_route[0], input).len();
    let mut bound_arm = || trex::scan(&no_route[1], input).len();
    let mut named_arm = || trex::scan(&no_route[2], input).len();

    println!("\n=== the fused significant lex, phase by phase ===");
    let mut arms: [(&str, &mut dyn FnMut() -> usize); 9] = [
        ("boundaries", &mut boundaries_arm),
        ("blobs parallel", &mut blobs_par_arm),
        ("blobs serial", &mut blobs_ser_arm),
        ("leaves", &mut leaves_arm),
        ("fused whole", &mut whole_arm),
        ("kind route", &mut route_arm),
        ("\"let\" \\W \"=\"", &mut plain_arm),
        ("\"let\" \\W:v \"=\"", &mut bound_arm),
        ("\\W:name \"=\"", &mut named_arm),
    ];
    let m = sweep(&mut arms);
    let (boundaries, blobs_par, blobs_ser, leaves, whole, route) =
        (m[0], m[1], m[2], m[3], m[4], m[5]);

    println!("\n=== what the fused lex is made of ===");
    println!("each phase as a share of the fused whole, and what the three leave unattributed");
    for (name, ms) in [("boundaries", boundaries), ("blobs parallel", blobs_par), ("leaves", leaves)]
    {
        println!("{name:>16} {ms:>10.4} ms {:>8.1}% of the fused whole", 100.0 * ms / whole);
    }
    let named = boundaries + blobs_par + leaves;
    println!(
        "{:>16} {named:>10.4} ms {:>8.1}%  leaving {:.4} ms ({:.1}%) in the dispatch itself",
        "the three",
        100.0 * named / whole,
        whole - named,
        100.0 * (whole - named) / whole
    );
    println!(
        "{:>16} {:>10.4} ms  the parallel blob pre-pass over the serial one the serial lexer also \
         runs",
        "blobs para-ser",
        blobs_par - blobs_ser
    );
    println!(
        "{:>16} {whole:>10.4} ms  {:.3} GB/s over {} bytes",
        "fused whole",
        input.len() as f64 / (whole * 1e6),
        input.len()
    );
    println!(
        "{:>16} {route:>10.4} ms  leaving {:.4} ms ({:.1}%) for the kind test itself",
        "kind route",
        route - whole,
        100.0 * (route - whole) / route
    );

    println!("\n=== the three patterns, and whether the fused lex is under them ===");
    println!("the lex is a floor only for a pattern no route answers. A route that lexes a token's");
    println!("worth of bytes at each occurrence of a literal never runs the whole-input lex, so");
    println!("subtracting it there reports an engine cheaper than nothing.");
    println!(
        "{:>16} {:>9} {:>10} {:>10} {:>10} {:>9} {:>14}",
        "pattern", "answers", "scan ms", "lex ms", "engine ms", "engine", "ns an anchor"
    );
    for (i, name) in ["\"let\" \\W \"=\"", "\"let\" \\W:v \"=\"", "\\W:name \"=\""].iter().enumerate()
    {
        let scan = m[6 + i];
        let pat = trex::parse(name).expect("the harness's own patterns parse");
        // Which engine answers it, on the same input the arm was timed over.
        // A routed pattern's row is the route's whole cost and the
        // subtraction below does not apply to it.
        let routed = trex::engine::routed_spans_public(&pat, input).is_some();
        if routed {
            println!(
                "{name:>16} {:>9} {scan:>10.4} {:>10} {:>10} {:>9} {:>14}",
                "route", "-", "-", "-", "-"
            );
            continue;
        }
        let engine = scan - whole;
        println!(
            "{name:>16} {:>9} {scan:>10.4} {whole:>10.4} {engine:>10.4} {:>8.1}% {:>14.1}",
            "lex+engine",
            100.0 * engine / scan,
            engine * 1e6 / tokens as f64
        );
    }
    println!(
        "an anchor is one significant token, and every one of the {tokens} is attempted; the \
         figure is wall time, so it is already divided across the {cores} cores"
    );
}
