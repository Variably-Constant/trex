//! What the lexer costs, and where handing it to the pool starts to save time.
//!
//! The lex is roughly four fifths of what a plain scan costs, so it is the
//! thing worth knowing precisely. Two questions: how the serial lexer scales,
//! and whether the parallel threshold matches the measured crossover rather
//! than a guess.

use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::time::Instant;

/// FNV-1a over whatever a `Hash` impl writes, so a token stream folds to one
/// word the same way in every build of this bench.
struct Fnv(u64);

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

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

/// One timing: the mean of `iters` calls, in milliseconds.
fn time_once(iters: u32, f: &mut impl FnMut() -> usize) -> f64 {
    let t0 = Instant::now();
    for _ in 0..iters {
        black_box(f());
    }
    t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters)
}

/// Each arm once a round, in an order that rotates, reduced by its median.
///
/// For rows that are read against each other: the pool's pre-pass against the
/// serial one, the pool's lexer against the whole lex, the split scan against
/// both. Taking one arm's repeats together and then the next arm's puts a burst
/// of load entirely into whichever arm it happened to land on, and from there
/// into every ratio that arm appears in. Rotating puts each arm at each position
/// instead, and the median drops what still lands on one.
fn rotate<'a>(arms: &[Box<dyn Fn() -> usize + 'a>], iters: u32, reps: u32) -> Vec<f64> {
    for f in arms {
        black_box(f());
    }
    let mut taken: Vec<Vec<f64>> = vec![Vec::new(); arms.len()];
    for rep in 0..reps as usize {
        for step in 0..arms.len() {
            let i = (step + rep) % arms.len();
            let t0 = Instant::now();
            for _ in 0..iters {
                black_box(arms[i]());
            }
            taken[i].push(t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters));
        }
    }
    taken
        .into_iter()
        .map(|mut v| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        })
        .collect()
}

fn summarize(mut samples: Vec<f64>) -> (f64, f64, f64) {
    samples.sort_by(f64::total_cmp);
    (samples[samples.len() / 2], samples[0], samples[samples.len() - 1])
}

/// Time two routes against each other, alternating which one goes first.
///
/// Timing one route to completion and then the other gives the second a
/// systematically different machine: the first has already warmed the clocks
/// and left the allocator grown. Measured here, that bias was 20 percent and
/// it did not show up in the spread, because it is not noise - every
/// repetition carried it in the same direction. It was caught only because
/// the control pass compares the serial lexer with itself, where the answer
/// has to be 1.00x and read 0.83x.
///
/// Alternating the order each repetition puts half of each route's samples in
/// each position, so the bias lands on both sides instead of on whichever ran
/// second.
fn time_pair(
    iters: u32,
    reps: u32,
    mut a: impl FnMut() -> usize,
    mut b: impl FnMut() -> usize,
) -> ((f64, f64, f64), (f64, f64, f64), f64) {
    black_box(a());
    black_box(b());
    let (mut sa, mut sb, mut ratios) = (Vec::new(), Vec::new(), Vec::new());
    for rep in 0..reps {
        // The two samples of a repetition are taken next to each other in
        // time, so whatever the box was doing during one it was mostly doing
        // during the other.
        let (ta, tb) = if rep % 2 == 0 {
            let ta = time_once(iters, &mut a);
            let tb = time_once(iters, &mut b);
            (ta, tb)
        } else {
            let tb = time_once(iters, &mut b);
            let ta = time_once(iters, &mut a);
            (ta, tb)
        };
        // The ratio of the two medians divides one number that drifted by one
        // that drifted differently, and on a shared box that difference is the
        // whole reading. The ratio WITHIN a repetition cancels any drift the
        // pair shared, and the median over those ratios then discards the
        // repetitions where it did not. This is the figure to quote.
        ratios.push(ta / tb);
        sa.push(ta);
        sb.push(tb);
    }
    ratios.sort_by(f64::total_cmp);
    (summarize(sa), summarize(sb), ratios[ratios.len() / 2])
}

/// How many independent timings each cell takes, from `TREX_BENCH_REPS`.
///
/// A value set but unparseable panics rather than falling back, so a sweep
/// asking for more repetitions than the default cannot quietly run at the
/// default and be read as though it had not.
fn reps() -> u32 {
    match std::env::var("TREX_BENCH_REPS") {
        Err(std::env::VarError::NotPresent) => 9,
        Err(e) => panic!("TREX_BENCH_REPS is set but unreadable: {e}"),
        Ok(v) => v
            .parse::<u32>()
            .unwrap_or_else(|e| panic!("TREX_BENCH_REPS is set to {v:?}, not a count: {e}")),
    }
}

/// The lex's phases over one input, serial and pooled, and nothing else.
///
/// The rows are read in rotation in one process, so a burst of load lands
/// across them rather than on one, and the serial rows are the control the
/// pooled ones are read against. The input is the file at `path`, or 256 kB
/// of the generated corpus without one. The token count and a hash of every
/// token's kind and span follow the timings, so two arms are held to one
/// answer as well as compared on time, and the pooled lexer is held to the
/// serial one's answer.
///
/// `load` threads spin in this process from before the first timing to after
/// the last, so the same arms are read with the cores contended as well as
/// idle: an arm that wins only on an idle box is not one the pool's callers
/// get.
fn phases_only(path: Option<&str>, load: usize) {
    let reps = reps();
    let bytes = match path {
        Some(p) => std::fs::read(p).unwrap_or_else(|e| panic!("cannot read {p}: {e}")),
        None => corpus(256 * 1024),
    };
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let spinners: Vec<std::thread::JoinHandle<()>> = (0..load)
        .map(|_| {
            let stop = std::sync::Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut x = 0x9e37_79b9_7f4a_7c15u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    x = black_box(x.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1));
                }
            })
        })
        .collect();
    println!("  load: {load} threads spinning in this process through every timing");
    // Enough calls that a timing is milliseconds, and one call once the
    // input alone takes that long.
    let iters = u32::try_from(((4 << 20) / bytes.len().max(1)).clamp(1, 20)).expect("at most 20");
    let blobs = trex::lexer::blob_runs(&bytes);
    // The pool's split scan alone, at the chunk count the pool asks for.
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let chunks = trex::parallel_lex::chunks_for(bytes.len(), cores);
    // The quote scan under the boundaries, read whole and read in pieces
    // across the cores: the same spans, so the one thing between the two rows
    // is how the scan is divided. The pieces are read at the floor that ships
    // and at the floors either side of it, from which that floor is chosen.
    let shipped = trex::parallel_lex::QUOTE_SCAN_MIN_LEAF;
    let sweep: Vec<usize> =
        [16usize, 32, 64, 128, 256].iter().map(|k| k << 10).filter(|&f| f != shipped).collect();
    let whole_quotes = || trex::parallel_lex::quoted_spans(&bytes).len();
    let pieced_quotes = || trex::parallel_lex::quoted_spans_across(&bytes, shipped).len();
    // The resumable scan the byte routes ask as they reach each occurrence,
    // asked every 128 bytes: the anchor search asks fifty thousand times over
    // 7.34 MB, one ask in 147 bytes.
    let resumed_step = 128;
    // Read one a round in a rotating order, because every row here is quoted
    // against another: the pool's pre-pass against the serial one, the pool's
    // lexer against the whole lex, the quote scan in pieces against it whole.
    let mut phases: Vec<Box<dyn Fn() -> usize>> = vec![
        Box::new(|| trex::lexer::blob_runs(&bytes).len()),
        Box::new(|| trex::lexer::blob_runs_parallel(&bytes).len()),
        Box::new(|| trex::lexer::lex_with_blobs(&bytes, &blobs, bytes.len() / 4).len()),
        Box::new(|| trex::lexer::lex(&bytes).len()),
        Box::new(|| trex::parallel_lex::lex_parallel(&bytes).len()),
        Box::new(|| trex::parallel_lex::safe_boundaries(&bytes, chunks).len()),
        Box::new(whole_quotes),
        Box::new(pieced_quotes),
        Box::new(|| trex::parallel_lex::quoted_spans_resumed(&bytes, resumed_step).len()),
    ];
    for &least in &sweep {
        let bytes = &bytes;
        phases.push(Box::new(move || trex::parallel_lex::quoted_spans_across(bytes, least).len()));
    }
    let taken = rotate(&phases, iters, reps);
    let (blob_ms, par_blob_ms, body_ms) = (taken[0], taken[1], taken[2]);
    let (whole_ms, par_lex_ms, bounds_ms) = (taken[3], taken[4], taken[5]);
    let (whole_quotes_ms, pieced_quotes_ms, resumed_quotes_ms) = (taken[6], taken[7], taken[8]);
    println!("  {} bytes, {iters} calls a timing, median of {reps} rotating timings", bytes.len());
    println!("  blob_runs (entropy pre-pass)             {blob_ms:>8.3} ms");
    println!("  parallel blob_runs (the pool's pre-pass) {par_blob_ms:>8.3} ms");
    println!("  lex_with_blobs (the token loop)          {body_ms:>8.3} ms");
    println!("  lex (both)                               {whole_ms:>8.3} ms");
    println!("  parallel lex (the pool's lexer)          {par_lex_ms:>8.3} ms");
    println!("  boundaries (the pool's split scan)       {bounds_ms:>8.3} ms  at {chunks} chunks");
    println!("  quote scan, whole                        {whole_quotes_ms:>8.3} ms");
    println!("  quote scan in pieces, {:>3} kB a piece    {pieced_quotes_ms:>8.3} ms  (ships)", shipped >> 10);
    for (i, &least) in sweep.iter().enumerate() {
        println!("  quote scan in pieces, {:>3} kB a piece    {:>8.3} ms", least >> 10, taken[9 + i]);
    }
    println!("  quote scan resumed, asked every {resumed_step} B    {resumed_quotes_ms:>8.3} ms  (the byte routes')");
    // The shipped pair again as a pair, each repetition's two readings taken
    // side by side and divided there, beside the whole scan divided by itself:
    // the floor a gap has to clear before it is the division's rather than the
    // position the arm ran in.
    let (_, _, divided) = time_pair(iters, reps, whole_quotes, pieced_quotes);
    let (_, _, floor) = time_pair(iters, reps, whole_quotes, whole_quotes);
    let pieces = trex::parallel_lex::quote_scan_pieces(&bytes, shipped);
    let route = if pieces == 1 { "whole".to_string() } else { format!("{pieces} pieces") };
    println!("  whole over in pieces, paired             {divided:>8.3}x  floor {floor:.3}x  ({route})");
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for spinner in spinners {
        spinner.join().expect("a spinning thread ends once told to");
    }
    let toks = trex::lexer::lex(&bytes);
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    for t in &toks {
        t.kind.hash(&mut h);
        h.write_u32(t.start);
        h.write_u32(t.end);
    }
    println!("  tokens {} hash {:016x}", toks.len(), h.finish());
    // The pool's lexer is held to the serial one's answer over the file timed,
    // not only over the generated statements the sweep checks.
    let pooled = trex::parallel_lex::lex_parallel(&bytes);
    assert_eq!(toks.len(), pooled.len(), "the two routes disagree on token count");
    assert!(
        toks.iter().zip(&pooled).all(|(x, y)| x.start == y.start && x.end == y.end && x.kind == y.kind),
        "the two routes disagree on the tokens themselves"
    );
    println!("  serial and parallel agree token for token");
    // The quote scan in pieces is held to the whole scan over the file timed,
    // at every floor it was read at.
    let whole = trex::parallel_lex::quoted_spans(&bytes);
    for least in std::iter::once(shipped).chain(sweep) {
        assert_eq!(
            trex::parallel_lex::quoted_spans_across(&bytes, least),
            whole,
            "the quote scan in pieces of at least {least} bytes disagrees with the whole scan"
        );
    }
    println!("  the quote scan in pieces agrees with it whole at every floor read");
    assert_eq!(
        trex::parallel_lex::quoted_spans_resumed(&bytes, resumed_step),
        whole,
        "the resumable quote scan disagrees with the whole scan"
    );
    println!("  the resumable quote scan agrees with it whole");
    // Which parker the pool's arms ran on, since a run that fell back to the
    // kernel park is not comparable with one that did not.
    println!(
        "  the parallel arms' parker: monitor wait {}",
        if flynnel::sched::sleep::monitor_wait_held() {
            "held throughout"
        } else {
            "did not hold, so this run fell back to the kernel park"
        }
    );
}

fn main() {
    // `--phases-only [file]` prints the phase block and stops. Comparing
    // build arms needs that block from each arm many times over,
    // interleaved; running the whole size sweep to reach it costs twenty
    // times the work and lengthens the window over which the machine can
    // drift between the arms being compared, which is the thing such a
    // comparison is trying to hold still.
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--phases-only") {
        // `--load <threads>` beside it reads the block with that many threads
        // spinning in this process. A count that does not parse is refused,
        // since a loaded run silently read idle is labeled with a load it
        // never carried.
        let load = match args.iter().position(|a| a == "--load") {
            None => 0,
            Some(l) => {
                let v = args.get(l + 1).expect("--load needs a thread count");
                v.parse::<usize>()
                    .unwrap_or_else(|e| panic!("--load {v} is not a thread count: {e}"))
            }
        };
        let path = args.get(at + 1).filter(|a| !a.starts_with("--"));
        phases_only(path.map(String::as_str), load);
        return;
    }
    // `--dump <file> <out>` writes every token of the file's serial lex to
    // `out` as `start end kind` lines and stops, so two builds' streams can
    // be diffed at every site they differ where their hashes only say that
    // they do.
    if let Some(at) = args.iter().position(|a| a == "--dump") {
        use std::io::Write;
        let path = args.get(at + 1).expect("--dump needs the input file");
        let out = args.get(at + 2).expect("--dump needs the output file");
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
        let toks = trex::lexer::lex(&bytes);
        let file = std::fs::File::create(out).unwrap_or_else(|e| panic!("cannot create {out}: {e}"));
        let mut w = std::io::BufWriter::new(file);
        for t in &toks {
            writeln!(w, "{} {} {:?}", t.start, t.end, t.kind).expect("the dump is written");
        }
        w.flush().expect("the dump is flushed");
        println!("  dumped {} tokens to {out}", toks.len());
        return;
    }
    // Every row at or below this size runs the serial lexer down both
    // columns, so its speedup is the ratio of a function to itself and says
    // nothing about the two routes. Printing the threshold the process
    // actually resolved, rather than the one the caller meant to set, is
    // what separates a real sweep from one whose override never took.
    let threshold = trex::parallel_lex::active_parallel_threshold();
    println!(
        "parallel threshold in force: {threshold} bytes  (rows under it are serial vs serial)"
    );
    let reps = reps();
    println!("each cell is the median of {reps} timings; spread is max/min of those timings");
    println!(
        "{:>10} {:>9} {:>9} {:>9} {:>8} {:>8} {:>7}",
        "bytes", "serial", "parallel", "ns/byte", "speedup", "spread", "route"
    );
    // Denser around a megabyte than the powers of two alone, because the
    // recorded curve dips at 1 MB and recovers by 2 MB. A single reading there
    // cannot tell a real trough from one long sample, and the two readings
    // that separate them are 1 MB repeated and the sizes either side of it.
    for kb in [16usize, 64, 128, 256, 512, 768, 1024, 1536, 2048, 3072, 4096, 8192] {
        let bytes = corpus(kb * 1024);
        // Enough calls that a cell is milliseconds rather than microseconds,
        // so the clock's own resolution is not part of the reading.
        let iters = if kb >= 2048 {
            5
        } else if kb >= 512 {
            20
        } else {
            60
        };
        let ((serial, s_lo, s_hi), (parallel, p_lo, p_hi), paired) = time_pair(
            iters,
            reps,
            || trex::lexer::lex(&bytes).len(),
            || trex::parallel_lex::lex_parallel(&bytes).len(),
        );
        // The worse of the two spreads: a tight serial column next to a
        // scattered parallel one is still a reading nobody should quote.
        let spread = (s_hi / s_lo).max(p_hi / p_lo);
        // The paired figure, not the ratio of the two medians: the pair was
        // taken adjacent in time, so drift the box imposed on one sample it
        // mostly imposed on the other, and dividing inside the pair cancels
        // it. On a contended box the two differ by more than the effect.
        let speedup = paired;
        let serial_route = bytes.len() < threshold;
        // A row below the threshold ran the serial lexer down both columns,
        // so its speedup is 1.00x by construction. Anything else is the
        // harness measuring the position a call ran in rather than the work
        // it did, and every other row in the table carries the same error.
        // Flagging it here is what makes the whole sweep falsifiable.
        let flag = if serial_route && (speedup - 1.0).abs() > 0.05 {
            "  <-- HARNESS BIAS: same function both columns, speedup must be 1.00x"
        } else {
            ""
        };
        println!(
            "{:>10} {serial:>9.3} {parallel:>9.3} {:>9.1} {:>7.2}x {:>7.2}x {:>7}{flag}",
            bytes.len(),
            serial * 1e6 / bytes.len() as f64,
            speedup,
            spread,
            if serial_route { "serial" } else { "pool" }
        );
    }

    // Where the serial lex actually spends itself. Guessing has been wrong
    // three times running, so the phases are timed rather than reasoned about.
    let bytes = corpus(256 * 1024);
    println!("\nphases of one serial lex over {} bytes", bytes.len());
    let blobs = trex::lexer::blob_runs(&bytes);
    // Rotating, because the share printed below divides the first of these by
    // the third: read one after another, whatever the box did during either row
    // lands in the percentage.
    let phases: Vec<Box<dyn Fn() -> usize>> = vec![
        Box::new(|| trex::lexer::blob_runs(&bytes).len()),
        Box::new(|| trex::lexer::lex_with_blobs(&bytes, &blobs, bytes.len() / 4).len()),
        Box::new(|| trex::lexer::lex(&bytes).len()),
    ];
    let taken = rotate(&phases, 20, reps);
    let (blobs_ms, body_ms, whole_ms) = (taken[0], taken[1], taken[2]);
    println!("  blob_runs (entropy pre-pass)             {blobs_ms:>8.3} ms");
    println!("  lex_with_blobs (the token loop)          {body_ms:>8.3} ms");
    println!("  lex (both)                               {whole_ms:>8.3} ms");
    println!(
        "  the pre-pass is {:.0}% of the lex",
        100.0 * blobs_ms / whole_ms
    );

    // The two must agree exactly, or the threshold is choosing between two
    // different answers rather than two routes to one.
    let bytes = corpus(256 * 1024);
    let a = trex::lexer::lex(&bytes);
    let b = trex::parallel_lex::lex_parallel(&bytes);
    assert_eq!(a.len(), b.len(), "the two routes disagree on token count");
    assert!(
        a.iter().zip(&b).all(|(x, y)| x.start == y.start && x.end == y.end && x.kind == y.kind),
        "the two routes disagree on the tokens themselves"
    );
    println!("\nserial and parallel agree token for token at 256kB");
}
