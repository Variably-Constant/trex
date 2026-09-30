//! What the chance gate refuses, read on real files.
//!
//! [`trex::context::record_period`] finds the record period by bounded
//! autocorrelation over shape classes: the lag whose exact-match share is
//! highest. The share alone cannot say whether that means anything, because
//! the share a lag reaches when the silhouettes carry no order at all is not a
//! constant - it is [`trex::context::record_period_chance`], the chance two
//! positions hold the same silhouette, and it moves with the corpus.
//!
//! So this reads the share beside its own null. `spectral::PERIOD_FLOOR` is an
//! absolute 0.20 and sits under the chance rate of prose, a log and a table,
//! where it therefore tests nothing and every lag clears it; it functions on
//! code only because code's chance rate falls below it by accident. The ratio
//! is what separates them, and `PERIOD_LIFT_FLOOR` is read off the gap this
//! harness measures.
//!
//! The `kept` column is the reading after that gate, which is what `@phase:k`
//! and the record cut actually see.
//!
//! This began as a comparison against a resonator-bank reading of the same
//! question, and that comparison concluded: the bank's fundamental was
//! measured unfit to source a period and removed, and the record cut moved to
//! the supertoken grain instead. What is left is the part that decided it.
//!
//! Every input is a real file somebody produced for another purpose; a corpus
//! built by repeating a motif would answer a question about itself.
//!
//! Run: `cargo run --release --example what_the_resonant_period_changes -- <file>...`

use std::io::Write;
use std::path::Path;

/// Slices are read as well as whole files, so a reading is not taken from one
/// length alone. A period found only at one size is a property of that window
/// rather than of the file.
const SLICE_SIZES: [usize; 3] = [64 * 1024, 256 * 1024, 1024 * 1024];

/// Slices at one size start this far apart, so two of them are not mostly the
/// same bytes.
const SLICE_STRIDE: usize = 2;

struct Reading {
    /// The raw winner, before the gate.
    raw: Option<(u16, f32)>,
    /// The share a lag reaches on silhouettes carrying no order, which is what
    /// the raw share has to beat before it is evidence of anything.
    chance: f32,
    /// The reading after the lift gate, which is what a scan sees.
    kept: Option<u16>,
    significant: usize,
}

fn read(bytes: &[u8]) -> Reading {
    let toks = trex::lexer::lex(bytes);
    Reading {
        raw: trex::context::record_period_share(&toks, bytes),
        chance: trex::context::record_period_chance(&toks, bytes),
        kept: trex::context::record_period(&toks, bytes),
        significant: toks.iter().filter(|t| t.is_significant()).count(),
    }
}

/// Records a period implies over a token count, which is how the token-grain
/// cut divided: one record per `period` significant tokens, and one whole
/// record where there is no period.
fn records(period: Option<u16>, significant: usize) -> usize {
    match period {
        Some(p) if p > 0 => significant.div_ceil(usize::from(p)),
        _ => usize::from(significant > 0),
    }
}

fn describe(r: &Reading) -> String {
    let (raw, share, lift) = match r.raw {
        Some((p, s)) => {
            // What the share is worth over the chance of two positions holding
            // the same silhouette at all. At or below 1.0 the reading is what
            // shuffling the same tokens would give.
            let l = if r.chance > 0.0 {
                format!("{:>6.2}", f64::from(s) / f64::from(r.chance))
            } else {
                "     -".to_string()
            };
            (format!("{p:>3}"), format!("{s:.3}"), l)
        }
        None => ("  -".to_string(), "    -".to_string(), "     -".to_string()),
    };
    let kept = match r.kept {
        Some(p) => format!("{p:>4}"),
        None => "   -".to_string(),
    };
    let rec_raw = records(r.raw.map(|(p, _)| p), r.significant);
    let rec_kept = records(r.kept, r.significant);
    let moved = if rec_raw == rec_kept {
        "same".to_string()
    } else if rec_raw > 0 {
        format!("{:+.1}%", (rec_kept as f64 - rec_raw as f64) / rec_raw as f64 * 100.0)
    } else {
        "from none".to_string()
    };
    format!(
        "{raw} {share} {:>6.3} {lift} {kept}  {rec_raw:>9} {rec_kept:>9}  {moved:>9}",
        r.chance
    )
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: what_the_resonant_period_changes <file>...");
        std::process::exit(2);
    }

    println!("raw = the lag with the highest exact-match share, before the gate");
    println!("chance = the share two positions share a silhouette with no order at all");
    println!("lift = share over chance; at 1.0 the reading is what a shuffle would give");
    println!("kept = the reading after the lift gate, which is what a scan sees");
    println!();
    println!(
        "{:<26} {:>9}  {:>3} {:>5} {:>6} {:>6} {:>4}  {:>9} {:>9}  {:>9}",
        "input", "sig toks", "raw", "share", "chance", "lift", "kept", "rec raw", "rec kept",
        "moved"
    );

    let mut total = 0usize;
    let mut dropped = 0usize;
    let mut records_moved = 0usize;
    let mut at_chance = 0usize;
    // An input that cannot be read is a row missing from the table, and a
    // table missing a row reads as complete. So a run with one exits failing.
    let mut unreadable = 0usize;

    for (n, arg) in args.iter().enumerate() {
        let path = Path::new(arg);
        let name = short(path);
        // stderr is unbuffered, so progress reaches a reader without a flush
        // even when stdout is redirected to a file.
        eprintln!("[{}/{}] {name}", n + 1, args.len());

        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{name:<26} unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };

        let mut takes: Vec<(String, &[u8])> = vec![(format!("{name} whole"), &bytes[..])];
        for size in SLICE_SIZES {
            if bytes.len() < size {
                continue;
            }
            let step = (bytes.len() - size) / SLICE_STRIDE.max(1);
            for i in 0..SLICE_STRIDE {
                let start = step.saturating_mul(i);
                let end = (start + size).min(bytes.len());
                if end > start {
                    takes.push((format!("{name} {}k@{i}", size / 1024), &bytes[start..end]));
                }
            }
        }

        for (label, slice) in takes {
            let r = read(slice);
            println!("{label:<26} {:>9}  {}", r.significant, describe(&r));
            // Redirected to a file, stdout is block-buffered, and a run this
            // long is read while it runs or not at all.
            std::io::stdout().flush()?;
            total += 1;
            if r.raw.is_some() && r.kept.is_none() {
                dropped += 1;
            }
            if records(r.raw.map(|(p, _)| p), r.significant) != records(r.kept, r.significant) {
                records_moved += 1;
            }
            // A reading at or below the chance rate is one a shuffle of the
            // same tokens would also produce.
            if let Some((_, s)) = r.raw
                && r.chance > 0.0
                && f64::from(s) / f64::from(r.chance) <= 1.0
            {
                at_chance += 1;
            }
        }
    }

    println!();
    println!(
        "{total} readings: the lift gate drops {dropped}, moving {records_moved} record counts"
    );
    println!(
        "{at_chance} of {total} raw readings sit at or below the chance rate, \
         so a shuffle of the same tokens reads the same period"
    );
    if unreadable > 0 {
        eprintln!("{unreadable} of {} inputs could not be read", args.len());
        std::process::exit(1);
    }
    Ok(())
}

fn short(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}
