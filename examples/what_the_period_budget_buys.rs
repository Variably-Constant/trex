//! What the periodicity budget buys, read on real files and against no invented
//! truth.
//!
//! The spectral field's period block costs 133.831 ms a scan over
//! `benches/engine_surface` and that cost is a cap rather than slack:
//! `per_eval` is `period_window * max_lag`, `max_evals` is
//! `PERIOD_OP_BUDGET / per_eval`, and the evaluation hop is
//! `(n / max_evals).max(cfg.period_hop)`. On a seven-megabyte input the budget
//! puts the hop at 481 against a configured 64, so the budget is what binds.
//! Lowering it raises the hop and cuts the cost in proportion.
//!
//! What it costs in return needs no ground truth, which is the point of reading
//! it this way. A first attempt planted periods - a motif of `p` bytes
//! repeated - and scored whether the field found them. It read 1.0000 at every
//! hop but the widest, because a repeated random motif is the easiest periodic
//! signal there is: no noise, no drift, nothing competing. A score that never
//! falls prices nothing, and worse, a synthetic corpus answers a question
//! about itself rather than about the data anyone runs this on.
//!
//! So the question asked here is what a smaller budget changes. The shipped
//! budget's readings over a real file are the reference; a larger hop is
//! compared against them offset by offset. Where they agree the cut is free on
//! that input, and where they differ is exactly what it costs. No planted
//! answer, no claim about which reading is right - only how far the cheaper one
//! departs from the one that ships.
//!
//! Run: `cargo run --release --example what_the_period_budget_buys -- <file>...`

use trex::spectral::{Needs, SpectralConfig, analyze_needing};

/// The hops compared against the shipped configuration. Each is what a budget
/// of some fraction of the shipped one would force.
const HOPS: [usize; 6] = [128, 256, 512, 1024, 2048, 4096];

/// Offsets are sampled this far apart. Far enough that two samples are not the
/// same evaluation held between hops, close enough to cover the file.
const STRIDE: usize = 1024;

/// Only the period reading, so what is timed is the block under test and not
/// the chain the other three readings share.
fn only_period() -> Needs {
    Needs { entropy: false, period: true, bands: false, onset: false, novelty: false }
}

/// The period at every sampled offset, and the evaluations the pass made.
fn read(input: &[u8], hop: usize) -> (Vec<u16>, u64) {
    read_with(input, SpectralConfig { period_hop: hop, ..SpectralConfig::default() })
}

/// The evaluations a run made, which is what the block costs.
///
/// A count rather than a clock, because this box is shared and a clock says so:
/// two identical configurations of this very scan timed 161.338 ms and 83.775
/// ms in one run. The evaluations are the same on a busy box as on a quiet one,
/// and the pass is that count times what one evaluation costs.
fn evals_of(counts: &[(&'static str, u64)]) -> u64 {
    counts.iter().find(|(n, _)| *n == "the spectral field: period evaluations").map_or(0, |(_, v)| *v)
}

fn read_with(input: &[u8], cfg: SpectralConfig) -> (Vec<u16>, u64) {
    trex::trace::record();
    drop(trex::trace::take_counts());
    let field = analyze_needing(input, &cfg, only_period());
    let counts = trex::trace::take_counts();
    trex::trace::stop();
    let mut got = Vec::new();
    let mut at = 0;
    while at < input.len() {
        got.push(field.frame_at(at).period);
        at += STRIDE;
    }
    (got, evals_of(&counts))
}

/// The hop the shipped budget forces on an input of `n` bytes.
///
/// The same arithmetic `analyze_needing` does, so the table can say which of
/// its rows the crate actually runs today.
fn shipped_hop(n: usize) -> usize {
    effective_hop(n, SpectralConfig::default().period_hop)
}

/// The hop an input of `n` bytes actually runs at when `requested` is
/// configured: the budget's own hop, or the request where that is coarser.
///
/// The two meet under a `max`, so a request below the budget's hop changes
/// nothing at all - which is the difference between asking for a hop and
/// getting one.
fn effective_hop(n: usize, requested: usize) -> usize {
    let cfg = SpectralConfig::default();
    let max_lag = cfg.max_lag.clamp(2, cfg.period_window / 2);
    let per_eval = cfg.period_window.saturating_mul(max_lag).max(1);
    let max_evals = (1_000_000_000usize / per_eval).max(1);
    (n / max_evals).max(requested).max(1)
}

/// The largest hop whose readings are identical to the most expensive one's,
/// and what it would cost, over a prefix of `input`.
///
/// Identical rather than close: a hop that changes nothing is free on this
/// input by definition, and one that changes a reading has a price that only
/// the caller of the axis can value. So the ladder answers where free stops and
/// says nothing about what lies past it.
fn free_hop(input: &[u8]) -> (usize, u64, u64) {
    let base = effective_hop(input.len(), SpectralConfig::default().period_hop);
    let (reference, base_evals) = read(input, SpectralConfig::default().period_hop);
    let mut best = (base, base_evals);
    for hop in HOPS {
        // A requested hop under the budget's own is absorbed by it and runs the
        // reference again, so it agrees with itself. Counting that as free
        // reports the budget term as a saving, which is how the first cut of
        // this ladder came to claim a free hop of 128 on an input whose hop was
        // already 165.
        if effective_hop(input.len(), hop) <= best.0 {
            continue;
        }
        let (got, evals) = read(input, hop);
        if reference == got {
            best = (effective_hop(input.len(), hop), evals);
        } else {
            break;
        }
    }
    (best.0, best.1, base_evals)
}

/// Read the free hop against input size, over prefixes of one real file.
///
/// The setting that suits a quarter-megabyte need not suit four, and the crate
/// already knows it: the hop is `(n / max_evals).max(cfg.period_hop)`, a term
/// that grows with the input under a floor that does not. A ladder says whether
/// that floor is in the right place, which a single file cannot.
fn ladder(path: &str, input: &[u8]) {
    println!("\n{path}: {} bytes, by input size", input.len());
    println!(
        "  {:>10} {:>10} {:>10} {:>12} {:>10}",
        "bytes", "shipped", "free hop", "evaluations", "vs shipped"
    );
    // Half again a rung rather than double. The free hop turned out to fall off
    // a cliff somewhere between a half megabyte and one, and a ladder that
    // doubles brackets that knee two apart - which is not a reading anyone
    // should set a constant from.
    let mut size = 65_536;
    while size <= input.len() {
        let (hop, evals, base_evals) = free_hop(&input[..size]);
        println!(
            "  {size:>10} {:>10} {hop:>10} {evals:>12} {:>10.4}",
            shipped_hop(size),
            evals as f64 / base_evals.max(1) as f64
        );
        size = size * 3 / 2;
    }
}

fn main() {
    let mut files: Vec<String> = std::env::args().skip(1).collect();
    // A ladder over prefixes of one real file, rather than one reading of each
    // of several. Size is the axis the setting has to answer on, and a handful
    // of files at whatever sizes they happen to be does not trace it.
    let by_size = files.first().is_some_and(|a| a == "--by-size");
    if by_size {
        files.remove(0);
    }
    if files.is_empty() {
        eprintln!("usage: what_the_period_budget_buys [--by-size] <file>...");
        eprintln!("real files only - a corpus written for this measures the writer");
        std::process::exit(2);
    }
    println!("trex {}", env!("CARGO_PKG_VERSION"));

    if by_size {
        for path in &files {
            match std::fs::read(path) {
                Ok(input) => ladder(path, &input),
                Err(e) => {
                    eprintln!("cannot read {path}: {e}");
                    std::process::exit(1);
                }
            }
        }
        return;
    }

    for path in &files {
        let input = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        };
        let (reference, base_evals) = read(&input, SpectralConfig::default().period_hop);
        println!(
            "\n{path}: {} bytes, {} sampled offsets, the shipped budget's hop here is {}",
            input.len(),
            reference.len(),
            shipped_hop(input.len())
        );
        // A file whose period reading is zero everywhere has nothing for a
        // budget to lose, and saying so is worth more than a column of 1.0000.
        let sounding = reference.iter().filter(|&&p| p > 0).count();
        println!(
            "  the shipped reading finds a period at {sounding} of {} offsets, over {base_evals} evaluations",
            reference.len()
        );
        if sounding == 0 {
            println!("  so nothing here can be lost by spending less on it");
            continue;
        }

        println!(
            "  {:>8} {:>6} {:>10} {:>12} {:>12} {:>9}",
            "stride", "at", "same", "same where", "evaluations", "vs base"
        );
        // A stride widening where the reading held still and falling back where
        // it moved was built and measured here, and it lost: on 4.79 MB of real
        // source a fixed stride reached 0.8325 agreement in 4675 evaluations
        // where the adapting one needed 4925 to reach 0.7695, and on a real
        // code tail a fixed stride read identically in 728 against the adapting
        // one's 0.7846 in 1968. Part of that is the metric - the reference is
        // itself a fixed stride, so a coarser fixed one samples a subset of its
        // positions and reproduces it exactly - and that bias is the right one
        // to keep, because the question is what can be spent less while
        // reporting the same thing. It is not kept in the code.
        let mut rows: Vec<(&str, usize, f64, f64, u64)> = Vec::new();
        for hop in HOPS {
            let (got, evals) = read(&input, hop);
            let same = reference.iter().zip(&got).filter(|(a, b)| a == b).count();
            // Agreement over the offsets where the shipped reading found
            // anything at all. The zeros agree trivially and would carry the
            // figure on a file that is mostly silent.
            let (mut sounded, mut sounded_same) = (0usize, 0usize);
            for (a, b) in reference.iter().zip(&got) {
                if *a > 0 {
                    sounded += 1;
                    sounded_same += usize::from(a == b);
                }
            }
            rows.push((
                "fixed",
                hop,
                same as f64 / reference.len().max(1) as f64,
                sounded_same as f64 / sounded.max(1) as f64,
                evals,
            ));
        }
        for (kind, at, same, where_same, evals) in &rows {
            println!(
                "  {kind:>8} {at:>6} {same:>10.4} {where_same:>12.4} {evals:>12} {:>9.4}",
                *evals as f64 / base_evals.max(1) as f64
            );
        }
    }
}
