//! Which of the parallel lexer's fixed costs scale with the chunk count, and
//! which with the input?
//!
//! `PARALLEL_LEX_THRESHOLD` exists to amortize what the parallel route pays
//! and the serial route does not. Its own doc names "the thread setup and the
//! boundary prescan", but 4 MB of input at the measured 6 ns a byte is about
//! 25 ms, against a scheduler dispatch measured on this host in microseconds -
//! so the constant cannot be amortizing the scheduler, and nothing has ever
//! measured what it IS amortizing.
//!
//! The buckets that matter are set by how a cost scales, not by which
//! subsystem owns it. A cost paid once per input is constant in the chunk
//! count, so it cannot move the optimal chunk count and belongs instead to the
//! serial-versus-parallel decision. A cost paid once per chunk is the term the
//! chunk-count formula minimises against. Assigning a phase to a bucket by
//! reading it has been wrong repeatedly, so this measures the scaling and lets
//! the numbers do the assigning.
//!
//! Two sweeps, each holding one variable still:
//!   chunks   input fixed, target chunk count doubling - flat means once per
//!            input, rising means once per chunk
//!   bytes    chunk count fixed, input doubling - flat means once per input is
//!            wrong too, and the cost is really per byte
//!
//!   cargo bench --bench lex_phase_scaling

use std::hint::black_box;
use std::time::Instant;

use trex::parallel_lex::safe_boundaries;

/// The bytes a sweep reads, repeated or truncated to the width it asks for.
///
/// A named file is read once and tiled to whatever width a sweep wants, so a
/// sweep over sizes reads the same token mix at every size. With none named
/// the generated statements stand, which is what every earlier reading here
/// was taken over.
///
/// The distinction is not cosmetic for this bench in particular. The blob
/// pre-pass runs a rolling entropy window over whitespace-free runs, and these
/// statements are short tokens separated by spaces where real source is full
/// of long unbroken runs - `crate::spectral::high_entropy_runs`, identifiers,
/// paths. Measured over the crate's own source the pass costs 316 us/MB
/// against 71 here, so a crossover derived from the statements is a crossover
/// for a token mix nobody wrote.
fn corpus_from(seed: Option<&[u8]>, bytes_wanted: usize) -> Vec<u8> {
    if let Some(bytes) = seed {
        if bytes.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(bytes_wanted);
        while out.len() < bytes_wanted {
            let take = (bytes_wanted - out.len()).min(bytes.len());
            out.extend_from_slice(&bytes[..take]);
        }
        return out;
    }
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

/// Median of `reps` timings, each the mean of `iters` calls, in microseconds.
///
/// A median rather than a mean across repetitions: this box is shared, and a
/// neighbour's burst lands on one repetition rather than on all of them.
fn median_us(iters: u32, reps: u32, mut f: impl FnMut() -> usize) -> f64 {
    black_box(f());
    let mut samples: Vec<f64> = (0..reps)
        .map(|_| {
            let t0 = Instant::now();
            for _ in 0..iters {
                black_box(f());
            }
            t0.elapsed().as_secs_f64() * 1e6 / f64::from(iters)
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn main() {
    let reps = 15;
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    // A path names the bytes every sweep below reads; with none the generated
    // statements stand, so an invocation that named nothing behaves as before.
    let seed: Option<Vec<u8>> = match std::env::args().nth(1) {
        Some(path) => match std::fs::read(&path) {
            Ok(bytes) => {
                println!("corpus {path}, {} bytes, tiled to each sweep's width", bytes.len());
                Some(bytes)
            }
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        },
        None => {
            println!("corpus the generated statements");
            None
        }
    };
    let corpus = |want: usize| corpus_from(seed.as_deref(), want);
    println!("host reports {cores} cores; trex asks safe_boundaries for cores * 4 = {} chunks", cores * 4);

    // Sweep 1: hold the input still, double the chunk count.
    let input = corpus(1024 * 1024);
    println!("\n=== chunk-count sweep, input fixed at {} bytes ===", input.len());
    println!("a cost that is flat here is paid once per input, whatever it is named");
    println!("{:>8} {:>14} {:>12} {:>10}", "chunks", "boundaries us", "per chunk", "vs C=1");
    let base = median_us(50, reps, || safe_boundaries(&input, 1).len());
    for c in [1usize, 2, 4, 8, 16, 32, 64, 96, 128, 256] {
        let us = median_us(50, reps, || safe_boundaries(&input, c).len());
        println!(
            "{c:>8} {us:>14.2} {:>12.3} {:>9.2}x",
            us / c as f64,
            us / base
        );
    }
    // The row this sweep divides by, read again once every other row has been.
    // It is the same work either way, so the distance between the two readings
    // is the box moving under the sweep, and it bounds the smallest ratio any
    // row above can carry.
    let control = median_us(50, reps, || safe_boundaries(&input, 1).len());
    println!("{:>8} {control:>14.2} {:>12} {:>9.3}x   the control", "C=1", "", control / base);

    // Sweep 2: hold the chunk count still, double the input.
    println!("\n=== input sweep, chunk count fixed at {} ===", cores * 4);
    println!("boundaries against the whole-buffer lex, so the prescan's share is readable");
    println!("{:>10} {:>14} {:>12} {:>10} {:>9}", "bytes", "boundaries us", "ns/byte", "lex us", "share");
    // Up to twelve megabytes because that is where the effect this sweep is
    // read for appears: the per-division cost of the boundaries phase is
    // 1.65 us at 4.79 MB and 5.54 at 12.07, and a sweep stopping at four
    // covers neither end of that. The two largest rows are slow and are the
    // point of the run rather than a cost to guard against.
    // 1536, 2048 and 3072 bracket a step this sweep found between 1 MB and
    // 4 MB, where the per-byte cost goes from 0.02 ns to 0.09 and stays there
    // through 12 MB. One core's L2 on this host is 1024 kB, so the step falls
    // where a serial prescan's working set leaves it; these sizes are what
    // say whether the knee sits at that boundary or somewhere else between.
    for kb in [64usize, 256, 1024, 1536, 2048, 3072, 4096, 8192, 12288] {
        let bytes = corpus(kb * 1024);
        let iters = if kb >= 1024 { 5 } else { 30 };
        let b_us = median_us(iters, reps, || safe_boundaries(&bytes, cores * 4).len());
        let lex_us = median_us(iters, reps, || trex::lexer::lex(&bytes).len());
        println!(
            "{:>10} {b_us:>14.2} {:>12.2} {lex_us:>10.1} {:>8.1}%",
            bytes.len(),
            b_us * 1e3 / bytes.len() as f64,
            100.0 * b_us / lex_us
        );
    }

    // Sweep 3: the stitch, which is the phase most likely to be misassigned.
    // It does a prefix sum over the chunks (per chunk) and then one linear
    // bracket-pairing pass over every token (per token). Which term dominates
    // decides whether trex has a once-per-chunk cost worth declaring at all.
    println!("\n=== stitch sweep, input fixed at {} bytes ===", input.len());
    println!("rising with chunks means a per-chunk cost; flat means it is really per token");
    println!("{:>8} {:>12} {:>12} {:>10}", "chunks", "stitch us", "per chunk", "vs C=2");
    let mut stitch_base = 0.0f64;
    for c in [2usize, 4, 8, 16, 32, 64, 96, 128, 256] {
        let bounds = safe_boundaries(&input, c);
        if bounds.len() <= 2 {
            println!("{c:>8}   only {} boundaries; skipped", bounds.len());
            continue;
        }
        let ranges: Vec<(usize, usize)> =
            bounds.windows(2).map(|w| (w[0], w[1])).collect();
        let blobs = trex::lexer::blob_runs(&input);
        let parts: Vec<(Vec<trex::token::Token>, trex::lexer::Seams)> = ranges
            .iter()
            .map(|&(s, e)| {
                trex::lexer::lex_chunk(
                    &input[s..e],
                    &trex::parallel_lex::chunk_blobs(&blobs, s, e),
                    (e - s) / 4,
                )
            })
            .collect();
        let us = median_us(20, reps, || trex::parallel_lex::stitch_parallel(&parts, &ranges).len());
        if stitch_base == 0.0 {
            stitch_base = us;
        }
        println!("{c:>8} {us:>12.2} {:>12.3} {:>9.2}x", us / c as f64, us / stitch_base);
    }

    // The blob table, which is the phase nothing has ever timed on its own.
    //
    // blob_runs_parallel runs over the entire input before a single chunk is
    // dispatched, so whatever it costs, the parallel route pays it and the
    // serial route does not. With safe_boundaries measured at two tenths of a
    // percent and the scheduler ruled out by arithmetic, this is the only
    // named phase left that could account for the recorded curve showing the
    // parallel route losing at a megabyte.
    //
    // It is timed against its serial counterpart, because the serial lexer
    // computes a blob table too - so only the difference between the two is a
    // cost the parallel route uniquely carries.
    println!("\n=== the blob pre-pass, serial against parallel ===");
    println!("only the difference is a cost the parallel route uniquely pays");
    println!(
        "{:>10} {:>12} {:>12} {:>10} {:>12}",
        "bytes", "serial us", "parallel us", "lex us", "para share"
    );
    // Dense between 64 kB and 256 kB: the parallel form measured slower than
    // the serial one at 64 kB and faster at 256 kB, so its own crossover is in
    // there. high_entropy_runs_parallel has no size gate of its own - it
    // dispatches at every size - and its declared 2 ns a byte is accurate
    // against the 2.12 measured here, so Flynnel's collapse gate sees real
    // work and dispatches, correctly by its own reckoning and wrongly for this
    // phase. Locating the crossover is what says whether the gate needs a
    // better number or this phase needs a gate.
    println!("{:>10} {:>12} {:>12} {:>10} {:>12} {:>9} {:>10} {:>9} {:>9}",
        "", "", "", "", "", "para/ser", "spans us", "spans/ser", "runs");
    for kb in [32usize, 64, 96, 128, 160, 192, 256, 512, 1024, 4096] {
        let bytes = corpus(kb * 1024);
        let iters = if kb >= 1024 { 5 } else { 30 };
        let s_us = median_us(iters, reps, || trex::lexer::blob_runs(&bytes).len());
        let p_us = median_us(iters, reps, || trex::lexer::blob_runs_parallel(&bytes).len());
        let lex_us = median_us(iters, reps, || trex::lexer::lex(&bytes).len());
        // The pass in two halves. Finding the whitespace-free runs at least
        // BLOB_MIN_LEN long is what `nonspace_spans_at_least` does and is all
        // the pass reads; measuring their entropy is the rest. The two have
        // different fixes - a cheaper span scan against a cheaper window - so
        // which dominates decides which is worth writing.
        let sp_us = median_us(iters, reps, || {
            trex::byte_simd::nonspace_spans_at_least(&bytes, trex::spectral::BLOB_MIN_LEN).len()
        });
        // How many runs the scan reports, which bounds what the vector holding
        // them can cost: a scan returning a handful cannot be spending its time
        // on the vector, and one returning hundreds of thousands might be.
        let spans = trex::byte_simd::nonspace_spans_at_least(&bytes, trex::spectral::BLOB_MIN_LEN).len();
        // Below 1.00x the parallel form is the slower one and this phase is
        // paying to dispatch work it should have run inline.
        println!(
            "{:>10} {s_us:>12.2} {p_us:>12.2} {lex_us:>10.1} {:>11.1}% {:>8.2}x {sp_us:>10.2} {:>8.1}% {spans:>9}",
            bytes.len(),
            100.0 * p_us / lex_us,
            s_us / p_us,
            100.0 * sp_us / s_us
        );
    }

    // Sweep 4: the whole dispatched fan-out against the chunk count.
    //
    // With the input fixed, the total is roughly W/C + O*C + K, where O is
    // every per-chunk cost there is - the scheduler's push, atomic and latch
    // init, plus any of trex's. Sweeps 1 and 3 measured trex's own per-chunk
    // costs and found them flat, so a rising limb here is the scheduler's
    // share, which is a figure this host has never had: the field doc offers
    // "~200-500ns" as a typical and nobody has measured it locally.
    //
    // Only the rising limb carries it. At small C the W/C term dominates and
    // swamps the slope, so the per-chunk column is worth reading only where
    // the total has stopped falling.
    println!("\n=== dispatched fan-out against chunk count, input fixed at {} bytes ===", input.len());
    println!("the slope of the rising limb is the per-chunk cost of everything");
    println!("{:>8} {:>12} {:>14} {:>12}", "chunks", "total us", "delta vs prev", "per extra chunk");
    let blobs = trex::lexer::blob_runs_parallel(&input);
    let mut prev: Option<(usize, f64)> = None;
    for c in [8usize, 16, 32, 64, 96, 128, 192, 256, 384, 512, 768, 1024] {
        let bounds = safe_boundaries(&input, c);
        if bounds.len() <= 2 {
            continue;
        }
        let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
        let actual = ranges.len();
        let us = median_us(5, reps, || {
            let mut parts: Vec<(Vec<trex::token::Token>, trex::lexer::Seams)> =
                ranges.iter().map(|_| (Vec::new(), trex::lexer::Seams::default())).collect();
            let per_chunk_ns =
                ((input.len() / actual.max(1)) as u64 * 6).min(u32::MAX as u64) as u32;
            let plan = flynnel::JobPlan::new(0, actual as u32)
                .with_leaf_shape(flynnel::LeafShape::Streaming)
                .with_estimated_per_item_ns(per_chunk_ns);
            flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(
                &plan,
                &mut parts,
                1,
                |start, slots| {
                    for (i, slot) in slots.iter_mut().enumerate() {
                        let (s, e) = ranges[start + i];
                        *slot = trex::lexer::lex_chunk(
                            &input[s..e],
                            &trex::parallel_lex::chunk_blobs(&blobs, s, e),
                            (e - s) / 4,
                        );
                    }
                },
            );
            parts.len()
        });
        // The chunk count safe_boundaries actually produced, which is not
        // always the one asked for - a boundary must land on a safe split.
        let delta = match prev {
            Some((pc, pus)) if actual > pc => {
                format!("{:>+9.2} {:>11.3}", us - pus, (us - pus) * 1e3 / (actual - pc) as f64)
            }
            _ => format!("{:>9} {:>11}", "-", "-"),
        };
        println!("{actual:>8} {us:>12.2} {delta}");
        prev = Some((actual, us));
    }
    println!("\nper-extra-chunk is in nanoseconds and is the figure to read off the rising limb");
}
