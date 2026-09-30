//! Lexing one input as sequential partitions, each across cores, against
//! lexing it whole: the lex a pipeline pays when it hands the device one
//! partition at a time. A partition count splits the input at
//! `safe_boundaries`, computes the whole input's blob table once, and lexes
//! the partitions in order with `lex_significant_range_into`. Before any
//! timing the joined partitions must equal the whole lex; the same split
//! lexed with a blob table per partition is compared too, and reported,
//! since a pipeline cannot use per-partition tables where they differ.
//!
//! Every round times the whole lex and each partition count, then each of
//! them a second time, once each in an order that rotates. Each ratio is the
//! median over rounds of partitioned milliseconds over whole milliseconds,
//! with the smallest and largest beside it. Each arm's control is the arm
//! against its own repeat, so it reads about 1.00x when the rounds have
//! absorbed the box's noise; a run beside another tenant says so arm by arm,
//! since a partitioned lex and the whole one need not move together under a
//! neighbour, and the run still needs a quiet host.
//!
//! Run: `cargo run --release --example partitioned_lex_timing -- [corpus [rounds]]`.
//! Without a corpus it lexes tag-number lines at 2 MB and 16.5 MB; with one,
//! the corpus whole.

use std::hint::black_box;
use std::time::Instant;

use trex::lexer::blob_runs_parallel;
use trex::parallel_lex::{
    SignificantWorkspace, lex_significant_parallel_into, lex_significant_range_into, safe_boundaries,
};

const PARTITIONS: [usize; 5] = [2, 4, 8, 16, 32];
const ROUNDS: usize = 21;

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

/// Milliseconds to lex `input` whole into `ws`.
fn lex_whole(input: &[u8], ws: &mut SignificantWorkspace) -> f64 {
    let t = Instant::now();
    lex_significant_parallel_into(input, ws);
    black_box(ws.kinds.len());
    t.elapsed().as_secs_f64() * 1e3
}

/// Milliseconds to compute `input`'s blob table and lex the partitions
/// `bounds` names, in order, into `ws`.
fn lex_partitions(input: &[u8], bounds: &[usize], ws: &mut SignificantWorkspace) -> f64 {
    let t = Instant::now();
    let blobs = blob_runs_parallel(input);
    for w in bounds.windows(2) {
        lex_significant_range_into(input, w[0], w[1], &blobs, ws);
        black_box(ws.kinds.len());
    }
    t.elapsed().as_secs_f64() * 1e3
}

/// `input` split at `bounds` and joined back: each partition lexed against
/// the whole input's blob table when `whole_table` is set, else lexed as an
/// input of its own, with its own table, and its spans shifted into place.
fn joined(input: &[u8], bounds: &[usize], whole_table: bool) -> (Vec<u32>, Vec<(u32, u32)>) {
    let blobs = blob_runs_parallel(input);
    let mut ws = SignificantWorkspace::default();
    let (mut kinds, mut spans) = (Vec::new(), Vec::new());
    for w in bounds.windows(2) {
        if whole_table {
            lex_significant_range_into(input, w[0], w[1], &blobs, &mut ws);
            spans.extend_from_slice(&ws.spans);
        } else {
            lex_significant_parallel_into(&input[w[0]..w[1]], &mut ws);
            let base = u32::try_from(w[0]).expect("an input offset within u32");
            spans.extend(ws.spans.iter().map(|&(s, e)| (s + base, e + base)));
        }
        kinds.extend_from_slice(&ws.kinds);
    }
    (kinds, spans)
}

/// The first significant token where `kinds` and `spans` differ from
/// `whole`'s, with both readings, or `None` when they are equal.
fn first_difference(kinds: &[u32], spans: &[(u32, u32)], whole: &SignificantWorkspace) -> Option<String> {
    let n = kinds.len().max(whole.kinds.len());
    (0..n)
        .find(|&i| kinds.get(i) != whole.kinds.get(i) || spans.get(i) != whole.spans.get(i))
        .map(|i| {
            format!(
                "token {i}: {:?} {:?} against the whole lex's {:?} {:?} ({} tokens against {})",
                kinds.get(i),
                spans.get(i),
                whole.kinds.get(i),
                whole.spans.get(i),
                kinds.len(),
                whole.kinds.len()
            )
        })
}

fn main() {
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
        None => [262_144usize, 2_097_152]
            .iter()
            .map(|&pairs| (format!("{pairs} tag lines"), tag_pairs(pairs)))
            .collect(),
        Some(path) => match std::fs::read(&path) {
            Ok(b) => vec![(path, b)],
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        },
    };

    for (name, input) in &inputs {
        println!("{name}, {} bytes", input.len());
        let mut whole = SignificantWorkspace::default();
        lex_significant_parallel_into(input, &mut whole);
        let splits: Vec<Vec<usize>> = PARTITIONS.iter().map(|&p| safe_boundaries(input, p)).collect();
        for (&asked, bounds) in PARTITIONS.iter().zip(&splits) {
            let made = bounds.len() - 1;
            let (kinds, spans) = joined(input, bounds, true);
            if let Some(d) = first_difference(&kinds, &spans, &whole) {
                panic!("{name}: {made} ranges lexed against the whole blob table differ from the whole lex at {d}");
            }
            let (kinds, spans) = joined(input, bounds, false);
            match first_difference(&kinds, &spans, &whole) {
                None => println!(
                    "  {asked} asked, {made} made: equal to the whole lex, and so are partitions with tables of their own"
                ),
                Some(d) => println!(
                    "  {asked} asked, {made} made: equal to the whole lex; partitions with tables of their own differ at {d}"
                ),
            }
        }

        let mut whole_ws = SignificantWorkspace::default();
        let mut part_ws = SignificantWorkspace::default();
        lex_whole(input, &mut whole_ws);
        for bounds in &splits {
            lex_partitions(input, bounds, &mut part_ws);
        }

        // The whole lex and one arm per partition count, then each of them
        // again: a rerun's ratio against its arm is the reading whose answer
        // is known.
        let half = splits.len() + 1;
        let arms = 2 * half;
        let mut whole_ms = Vec::with_capacity(rounds);
        let mut whole_controls = Vec::with_capacity(rounds);
        let mut part_ms: Vec<Vec<f64>> = vec![Vec::with_capacity(rounds); splits.len()];
        let mut ratios: Vec<Vec<f64>> = vec![Vec::with_capacity(rounds); splits.len()];
        let mut part_controls: Vec<Vec<f64>> = vec![Vec::with_capacity(rounds); splits.len()];
        for round in 0..rounds {
            let mut ms = vec![0.0f64; arms];
            for i in 0..arms {
                let arm = (i + round) % arms;
                let which = arm % half;
                ms[arm] = if which == 0 {
                    lex_whole(input, &mut whole_ws)
                } else {
                    lex_partitions(input, &splits[which - 1], &mut part_ws)
                };
            }
            whole_ms.push(ms[0]);
            whole_controls.push(ms[0] / ms[half]);
            for (k, ((part, ratio), control)) in
                part_ms.iter_mut().zip(ratios.iter_mut()).zip(part_controls.iter_mut()).enumerate()
            {
                part.push(ms[k + 1]);
                ratio.push(ms[k + 1] / ms[0]);
                control.push(ms[k + 1] / ms[half + k + 1]);
            }
        }
        println!(
            "  whole lex {:.3} ms, median of {rounds} rounds; control {:.3}x, the whole lex against its own repeat",
            median(&mut whole_ms),
            median(&mut whole_controls)
        );
        println!("  {:>10} {:>8} {:>11} {:>17} {:>9}", "partitions", "ms", "part/whole", "spread", "control");
        for (k, bounds) in splits.iter().enumerate() {
            let lo = ratios[k].iter().copied().fold(f64::INFINITY, f64::min);
            let hi = ratios[k].iter().copied().fold(0.0, f64::max);
            println!(
                "  {:>10} {:>8.3} {:>10.3}x {:>8.3}..{:<8.3} {:>8.3}x",
                bounds.len() - 1,
                median(&mut part_ms[k]),
                median(&mut ratios[k]),
                lo,
                hi,
                median(&mut part_controls[k])
            );
        }
    }
}
