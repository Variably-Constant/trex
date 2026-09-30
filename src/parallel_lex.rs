//! Parallel tokenization across cores.
//!
//! Lexing a large input is embarrassingly parallel once the input is
//! cut at points the serial lexer also treats as token boundaries. The
//! work that dominates lexing (typed-token recognition, which probes
//! every position for a URL, email, IP, or timestamp shape, plus
//! bracket pairing) then runs on one core per chunk, and the result is
//! stitched back into a single stream byte-identical to the serial
//! lexer, bracket mates included.
//!
//! ## Where a split is safe
//!
//! Only two token kinds can span a newline: a whitespace run (a
//! newline is whitespace) and a quoted string (it consumes bytes up to
//! the closing quote). Every other kind stops at the first whitespace.
//! So a byte position `p` is a guaranteed serial token boundary when
//!
//! - `input[p - 1]` is a newline, and
//! - `input[p]` is not whitespace, and
//! - `p` is not inside a quoted string.
//!
//! At such a `p` the whitespace run ends exactly at `p` (because the
//! next byte is significant) and no quoted string crosses it, so the
//! chunk before `p` and the chunk at `p` lex exactly as the serial
//! lexer would. Where a stretch of the input has no newline, the end of
//! any whitespace run serves instead, provided the byte before the run is
//! neither a quote nor a backslash (a char literal can hold a space), the
//! byte at `p` is neither a digit nor a sign, and the run is not one space
//! between a digit and a unit symbol: the recognizers that read across
//! whitespace - a card's digit groups, a phone's groups, a coordinate pair -
//! continue with a digit or a sign, and a quantity with its symbol one
//! space after its number, so none of them spans such a `p`. The quoted
//! strings are read from the quote bytes alone,
//! each found by the SIMD search, and the newline near each division by
//! the same search, so finding these points costs far less than the
//! classification it enables to run in parallel.
//!
//! ## Stitching
//!
//! Each chunk is lexed independently and its offsets shifted by the
//! chunk's base. A chunk's lex pairs its own brackets and hands back the
//! opens it left unclosed and the closes it met over an empty stack; those
//! alone run, in stream order, through the same open-stack the serial lexer
//! uses, so an open bracket in one chunk pairs with its close in another
//! exactly as it would serially.

use crate::lexer::{
    PairedSignificant, Seams, Significant, lex_chunk_into, lex_chunk_paired_into,
    lex_chunk_significant_into,
};
use crate::token::{Token, TokenKind};

/// The blob spans covering `[s, e)`, rebased to be relative to that chunk.
///
/// A blob is whitespace-free and every chunk boundary sits just after a
/// newline, so no blob straddles a boundary; a span not wholly inside the
/// chunk is therefore not this chunk's and is dropped.
/// Public alongside [`safe_boundaries`] and [`stitch_parallel`] so a bench can
/// rebuild one chunk's inputs and time the phases separately.
#[must_use]
pub fn chunk_blobs(blobs: &[(usize, usize)], s: usize, e: usize) -> Vec<(usize, usize)> {
    blobs
        .iter()
        .filter(|&&(a, b)| a >= s && b <= e)
        .map(|&(a, b)| (a - s, b - s))
        .collect()
}

/// Inputs shorter than this lex serially.
///
/// Measured by `benches/lex_scaling`, the parallel route against the serial
/// one on the same corpus, each cell the median of thirty-one paired
/// timings:
///
/// ```text
///    16kB  1.03x   256kB  1.28x     2MB  2.22x
///    64kB  1.26x   768kB  1.93x     4MB  2.75x
///   128kB  1.67x     1MB  1.88x     8MB  3.11x
/// ```
///
/// The parallel route is ahead at every size measured, so the threshold is
/// not where the routes cross in cost - it is where the win becomes larger
/// than the spread. At sixteen kilobytes the reading is 1.03x against a
/// spread of 4.90x between that cell's own fastest and slowest sample,
/// which does not establish a win; at sixty-four kilobytes it is 1.26x
/// against a spread of 1.77x, which does.
///
/// Two properties of the measurement decide whether a later reading can be
/// compared with this one. The speedup is the median of per-repetition
/// ratios, each pair timed adjacent, rather than the ratio of two medians:
/// on a shared machine the two arms drift differently and that difference
/// exceeds the effect. And a row below the threshold runs the serial lexer
/// down both columns, so it must read 1.00x; the bench flags it when it
/// does not, which is what catches a harness measuring the order a call ran
/// in rather than the work it did.
///
/// The blob table and the boundary prescan run ahead of the chunks, each a
/// pass over the whole input across the cores with its own crossover (see
/// `crate::spectral::high_entropy_runs_parallel` and [`quoted_spans_across`]);
/// what stays serial is the stitch's prefix sum and the final bracket pairing.
const PARALLEL_LEX_THRESHOLD: usize = 64 * 1024;

/// The chunks a core the parallel lexes divide an input into, at most. The
/// leaves are equal in bytes and unequal in cost, and a fan-out ends when the
/// last leaf started ends, so finer leaves shorten that tail and pay more
/// boundaries and dispatches for it. Measured by `where_the_time_goes` on a
/// 24-thread box, four against eight and sixteen a core: over 3,688,563
/// bytes of this crate's own src sixteen took the lex in parts to 0.86 of
/// four's and the set engine's lex to 0.95, over 4,787,795 bytes of
/// real_code.txt to 0.92 and 0.96, over 12,070,602 bytes of train_corpus.txt
/// to 0.92 and 0.99, with eight between at every size.
const CHUNKS_A_CORE: usize = 16;

/// The bytes a chunk holds, at the least, where the input is too small for
/// [`CHUNKS_A_CORE`] a core; and a chunk never holds more than a quarter of
/// a core's share of the input, so a leaf is always outnumbered by the
/// workers four to one. Over 746,496 bytes of code_tail.txt, where four a
/// core is 96 chunks of 7.8 kB, sixteen a core cost the set engine's lex 3%
/// and eight gained it nothing, so the least chunk holds that input at 96;
/// over 1,234,609 bytes of mobydick.txt its 150 chunks read the lex in parts
/// at 0.96 of four a core's and the set engine's lex level.
const LEAST_CHUNK_BYTES: usize = 8 * 1024;

/// The chunks `n` bytes divide into across `cores`.
///
/// Public so a bench asking [`safe_boundaries`] for boundaries asks for the
/// count the parallel lexes ask for, and times the scan they run.
#[must_use]
pub fn chunks_for(n: usize, cores: usize) -> usize {
    (n / LEAST_CHUNK_BYTES).clamp(cores * 4, cores * CHUNKS_A_CORE)
}

/// Bytes a piece of the quote scan across the cores holds, at the least:
/// [`quoted_spans_across`] under [`safe_boundaries`].
///
/// Measured by `benches/lex_scaling --phases-only` on pc2 over the 46 runs of
/// inputs of a megabyte and more, 8 to 21 foreign cores on the box, the whole
/// scan over the pieces from rotated medians: 16 kB a piece read 1.83 at the
/// median, 32 kB 1.95, 64 kB 1.77, 128 kB 1.68 and 256 kB 1.42, and 32 kB left
/// the fewest runs under 0.97, three, two of them the UTF-16 table whose
/// newlines are too far apart to cut at, which [`quote_piece_starts`] gives
/// back to be read whole.
///
/// Public so a bench reads the floor that ships beside the ones it sweeps.
pub const QUOTE_SCAN_MIN_LEAF: usize = 32 * 1024;

/// The input below which [`quoted_spans_across`] reads the quotes whole.
///
/// Measured by `benches/lex_scaling --phases-only` on pc2, the scan whole
/// against in pieces at 64 kB a piece, paired in one process with the whole
/// scan paired against itself as the floor, with 8 to 21 foreign cores on the
/// box: over 128 kB slices of six real files the pieces lost on every one, 0.34x
/// to 0.77x, where dispatch costs more than a scan of 4 to 21 us; over 512 kB
/// slices they lost on 5 of 12; over the comparison campaign's 1 MB heads and
/// tails, 1,047,718 to 1,048,574 bytes of code, data, logs, prose and wiki,
/// they won on all ten, 1.25x to 2.32x. The gate lies between those two sizes.
pub const QUOTE_SCAN_PARALLEL_BYTES: usize = 1_000_000;

/// The threshold in force, which is [`PARALLEL_LEX_THRESHOLD`] unless
/// `TREX_PARALLEL_LEX_THRESHOLD` names another.
///
/// The recorded table above compares the two routes at sizes the constant
/// now sends down the serial one, so re-measuring it needs the parallel
/// route reachable below the threshold. Without an override the bench
/// calls one function twice and reports the ratio of a function to itself,
/// which is 1.00x whatever the two routes would have cost.
///
/// A variable that is set but unreadable panics rather than falling back.
/// The fallback is the threshold being overridden, so a silent one runs the
/// measurement at the default and labels it with the override: the reading
/// would say the parallel route is exactly as fast as the serial route at
/// every size, which is what this function exists to stop being able to say
/// by accident.
///
/// Read once: the value cannot change within a process, and reading it per
/// call would take the environment lock on the lexer's entry.
fn parallel_lex_threshold() -> usize {
    static THRESHOLD: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *THRESHOLD.get_or_init(|| match std::env::var("TREX_PARALLEL_LEX_THRESHOLD") {
        Err(std::env::VarError::NotPresent) => PARALLEL_LEX_THRESHOLD,
        Err(e) => panic!("TREX_PARALLEL_LEX_THRESHOLD is set but unreadable: {e}"),
        Ok(v) => v.parse::<usize>().unwrap_or_else(|e| {
            panic!("TREX_PARALLEL_LEX_THRESHOLD is set to {v:?}, which is not a byte count: {e}")
        }),
    })
}

/// The byte count below which [`lex_parallel`] lexes serially, as this
/// process resolved it.
///
/// A caller measuring the two routes against each other reports this rather
/// than the value it tried to set, so a run whose override did not take
/// effect is labelled with the threshold that actually ran.
#[must_use]
pub fn active_parallel_threshold() -> usize {
    parallel_lex_threshold()
}

/// The buffers the pool's lexer writes: a token vector and seam record per
/// chunk, and the stitched stream. Kept across calls, their pages stay
/// mapped; they grow to the largest input lexed through them.
///
/// `examples/held_scan_ab` measured `trex::scan` through held buffers against
/// the same scan through fresh ones, in rotating rounds with a control and
/// another tenant on the box: 1.60x to 1.64x at 129 kB of tag lines, 1.35x at
/// 1 MB, 1.13x to 1.20x at 4.1 MB and 1.22x to 1.24x at 16.5 MB; 1.18x to
/// 1.22x on 4 MB of prose and 1.09x to 1.12x on 63.5 MB; and 0.99x to 1.03x
/// at 64 kB, where the lex is serial. The controls read 0.98x to 1.03x.
#[derive(Default)]
pub struct TokenWorkspace {
    parts: Vec<(Vec<Token>, Seams)>,
    /// The stitched stream of the last input.
    pub toks: Vec<Token>,
}

thread_local! {
    /// This thread's held lexer buffers, for callers that lex many inputs;
    /// empty while a lex on this thread has them out.
    static HELD: std::cell::Cell<TokenWorkspace> = std::cell::Cell::new(TokenWorkspace::default());
}

/// [`lex_parallel`] into this thread's held buffers, handing the stream to
/// `f`: a lex that takes the pool writes into pages an earlier one mapped.
/// Below the pool's threshold the lex is serial and takes a fresh vector,
/// where `examples/held_scan_ab` measured a held one at 0.99x to 1.02x.
///
/// The buffers are out for the call and back after it. A lex on this thread
/// while they are out - one nested in `f`, or a job the thread runs while its
/// own lex waits on the pool - finds none and lexes into buffers of its own.
pub fn lex_parallel_held<R>(input: &[u8], f: impl FnOnce(&[Token]) -> R) -> R {
    if input.len() < parallel_lex_threshold() {
        return f(&crate::lexer::lex(input));
    }
    let mut ws = HELD.take();
    lex_parallel_into(input, &mut ws);
    let out = f(&ws.toks);
    HELD.set(ws);
    out
}

/// Tokenize `input` across the available cores through the work-stealing
/// pool, returning a token stream identical to [`crate::lexer::lex`]. It
/// splits into several fine chunks per core so the pool steal-balances the
/// per-chunk density variance and amortizes the dispatch; small inputs,
/// inputs with no safe split point, and single-core hosts fall back to the
/// serial lexer.
#[must_use]
pub fn lex_parallel(input: &[u8]) -> Vec<Token> {
    let mut ws = TokenWorkspace::default();
    lex_parallel_into(input, &mut ws);
    ws.toks
}

/// [`lex_parallel`] into `ws`, whose `toks` then hold the stream.
pub fn lex_parallel_into(input: &[u8], ws: &mut TokenWorkspace) {
    use flynnel::JobPlan;

    let TokenWorkspace { parts, toks } = ws;
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    // Split into several chunks per core, not one. Fine chunks give the
    // work-stealing pool something to balance (chunks vary in token
    // density) and amortize the dispatch over more work units, the regime
    // the pool wins; one coarse chunk per core leaves nothing to steal.
    //
    // The spectral blob gate reads a trailing 64-byte window, so a chunk-local
    // call classifies its first 64 bytes against a truncated history and
    // diverges from the serial lexer. Computing the table once over the whole
    // input removes that dependence; a blob is whitespace-free and so lies
    // within one chunk, since every boundary sits just after a newline. The
    // boundaries and the table are each a pass over the whole input across the
    // cores, taken before any leaf runs and phased apart from the leaves and
    // the stitch.
    let (bounds, blobs) = bounds_and_blobs(
        input,
        cores,
        (Some("the lex: the boundaries"), Some("the lex: the blob table")),
    );
    if bounds.len() <= 2 {
        toks.clear();
        toks.reserve(input.len() / crate::lexer::TOKEN_BYTES_ESTIMATE);
        lex_chunk_into(input, &blobs, toks, &mut Seams::default());
        return;
    }
    let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
    if parts.len() < ranges.len() {
        parts.resize_with(ranges.len(), Default::default);
    }
    let parts = &mut parts[..ranges.len()];
    for (&(s, e), (chunk, seams)) in ranges.iter().zip(parts.iter_mut()) {
        chunk.clear();
        chunk.reserve((e - s) / crate::lexer::TOKEN_BYTES_ESTIMATE);
        seams.open.clear();
        seams.close.clear();
    }

    // Each leaf lexes a whole chunk, and `benches/lex_scaling` measures that
    // at 5.5 to 6.4 ns a byte, flat from 16 kB to 8 MB. The estimate makes the
    // pool's serial-vs-parallel choice depend on total work, not on K_outer
    // (which a lexer does not have), and the pool compares this figure times
    // the item count against its own collapse threshold - so a low estimate
    // reads as little work and argues for staying serial. Six is the measured
    // rate rounded down within its own spread, which errs toward dispatching.
    let avg_chunk_bytes = input.len() / ranges.len().max(1);
    let per_chunk_ns = (avg_chunk_bytes as u64 * 6).min(u32::MAX as u64) as u32;
    // A leaf here walks a chunk of bytes and writes tokens out, so its
    // bottleneck is per-core memory bandwidth rather than an issue port.
    // Flynnel names that shape `Streaming` and gives byte scans and parsing as
    // its examples; `PortCompute` is for work that saturates an execution unit
    // (integer multiply, FMA), which a lexer does not. The two agree that SMT
    // siblings should be parked, but for opposite reasons - contention for
    // bandwidth here, contention for a port there - and the class also sets
    // the gating and leaf-sizing knobs, so declaring the wrong one gets those
    // tuned for a workload this is not.
    let plan = JobPlan::new(0, ranges.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::Streaming)
        .with_estimated_per_item_ns(per_chunk_ns);
    let lexing = crate::trace::phase("the lex: a chunk a leaf");
    over_ranges_longest_first(&plan, &ranges, parts, |k, (chunk, seams)| {
        let (s, e) = ranges[k];
        lex_chunk_into(&input[s..e], &chunk_blobs(&blobs, s, e), chunk, seams);
    });
    drop(lexing);

    stitch_parallel_into(parts, &ranges, toks);
}

/// The buffers a paired lex writes: one significant stream per chunk with the
/// brackets it could not pair beside it. Kept across calls, their pages stay
/// mapped.
///
/// A chunk and its seams travel together, as they do for the token path, so
/// one pass over the chunks reaches both.
#[derive(Default)]
pub struct PairedWorkspace {
    parts: Vec<(PairedSignificant, Seams)>,
}

/// The significant stream with its brackets paired, in the lexer's parts,
/// handed to `f` where the chunks were lexed: the chunks are never copied into
/// one pair of arrays, which is the copy this path exists to avoid.
///
/// Every mate is an index into the whole stream, so a reader walks the parts
/// without tracking which chunk a mate landed in.
///
/// The buffers are out for the call and back after it, as they are for
/// [`lex_significant_parts_held`].
pub fn lex_paired_parts_held<R>(
    input: &[u8],
    f: impl FnOnce(&[(PairedSignificant, Seams)]) -> R,
) -> R {
    let mut ws = HELD_PAIRED.take();
    let n = fill_paired_parts(input, &mut ws);
    let out = f(&ws.parts[..n]);
    HELD_PAIRED.set(ws);
    out
}

/// Lex `input` into `ws` as paired significant parts, answering how many parts
/// hold it, with every bracket paired and every mate in the whole stream's
/// index space.
fn fill_paired_parts(input: &[u8], ws: &mut PairedWorkspace) -> usize {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    // The boundaries and the blob table are each a pass over the whole input
    // across the cores, taken before any leaf runs and phased apart from the
    // leaves and the join.
    let (bounds, blobs) =
        bounds_and_blobs(input, cores, (Some("the paired lex: the boundaries"), None));
    let ranges: Vec<(usize, usize)> = if bounds.len() <= 2 {
        vec![(0, input.len())]
    } else {
        bounds.windows(2).map(|w| (w[0], w[1])).collect()
    };
    lex_paired_leaves(input, &ranges, &blobs, ws);
    join_paired_seams(&mut ws.parts[..ranges.len()]);
    ranges.len()
}

/// Lex each range into its own paired stream, across cores, keeping what each
/// chunk's pairing left for the walk between chunks.
fn lex_paired_leaves(
    input: &[u8],
    ranges: &[(usize, usize)],
    blobs: &[(usize, usize)],
    ws: &mut PairedWorkspace,
) {
    use flynnel::JobPlan;

    if ws.parts.len() < ranges.len() {
        ws.parts
            .resize_with(ranges.len(), || (PairedSignificant::with_base(0, 0), Seams::default()));
    }
    let _lexing = crate::trace::phase("the paired lex, a mate a token");
    let parts = &mut ws.parts[..ranges.len()];
    for (&(s, e), (part, seam)) in ranges.iter().zip(parts.iter_mut()) {
        part.reset(s);
        part.reserve((e - s) / crate::lexer::TOKEN_BYTES_ESTIMATE);
        seam.open.clear();
        seam.close.clear();
    }

    // One chunk is the whole input, and dispatching a plan of one leaf would
    // price the scheduler into every input too small to be split.
    if let [(s, e)] = *ranges {
        let (part, seam) = &mut parts[0];
        lex_chunk_paired_into(&input[s..e], &chunk_blobs(blobs, s, e), part, seam);
        return;
    }

    // The leaf is the significant leaf writing one more word a token, so that
    // leaf's per-byte estimate carries over.
    let lexed: usize = ranges.iter().map(|&(s, e)| e - s).sum();
    let avg_chunk_bytes = lexed / ranges.len().max(1);
    let per_chunk_ns = (avg_chunk_bytes as u64 * 6).min(u32::MAX as u64) as u32;
    let plan = JobPlan::new(0, ranges.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::Streaming)
        .with_estimated_per_item_ns(per_chunk_ns);
    over_ranges_longest_first(&plan, ranges, &mut *parts, |k, (part, seam)| {
        let (s, e) = ranges[k];
        lex_chunk_paired_into(&input[s..e], &chunk_blobs(blobs, s, e), part, seam);
    });
}

/// Put every mate in the whole stream's index space, and pair the brackets that
/// cross a chunk boundary.
///
/// A chunk pairs what it can as it lexes, in its own indices, because no chunk
/// knows where it begins until every earlier one has been lexed. This is the
/// first point that does, so it does both things that need knowing: it adds
/// each chunk's base to the mates that chunk wrote, and it walks the seams -
/// each chunk's unclosed opens and the closes it met over an empty stack, in
/// stream order, through the stack the serial lexer holds there - writing the
/// pairs that span chunks in those same whole-stream indices. Every other
/// bracket was paired inside its chunk or pairs with nothing serially.
///
/// Running this over a single part is correct and pairs nothing: in one chunk a
/// close that met an empty stack precedes every open left unclosed, or it would
/// have closed one, so the walk meets those closes with an empty stack too.
fn join_paired_seams(parts: &mut [(PairedSignificant, Seams)]) {
    let _joining = crate::trace::phase("the seam join");
    let mut bases: Vec<u32> = Vec::with_capacity(parts.len());
    let mut acc: u32 = 0;
    for (part, _) in parts.iter() {
        bases.push(acc);
        acc = acc
            .checked_add(u32::try_from(part.mates.len()).expect("a token index within the stored width"))
            .expect("a token index within the stored width");
    }
    let shifting = crate::trace::phase("the seam join: every mate shifted by its base");
    for ((part, _), &base) in parts.iter_mut().zip(bases.iter()) {
        if base == 0 {
            continue;
        }
        for m in &mut part.mates {
            if *m != crate::lexer::NO_MATE {
                *m += base;
            }
        }
    }
    drop(shifting);

    // The seam brackets of every chunk, in stream order, each carrying where it
    // sits in the whole stream and where to write its mate.
    let mut events: Vec<(u32, usize, usize, bool, crate::token::BracketKind)> = Vec::new();
    for (ci, (part, seam)) in parts.iter().enumerate() {
        let base = bases[ci];
        let (mut oi, mut cj) = (0usize, 0usize);
        while oi < seam.open.len() || cj < seam.close.len() {
            let take_open =
                seam.open.get(oi).is_some_and(|&o| cj >= seam.close.len() || o < seam.close[cj]);
            let local = if take_open { seam.open[oi] } else { seam.close[cj] };
            if let Some((is_open, bk)) = TokenKind::bracket_of_code(part.parts.kinds[local]) {
                events.push((base + local as u32, ci, local, is_open, bk));
            }
            if take_open {
                oi += 1;
            } else {
                cj += 1;
            }
        }
    }

    let mut open_stack: Vec<(u32, usize, usize, crate::token::BracketKind)> = Vec::new();
    let mut paired: Vec<(usize, usize, usize, usize, u32, u32)> = Vec::new();
    for (at, ci, local, is_open, bk) in events {
        if is_open {
            open_stack.push((at, ci, local, bk));
            continue;
        }
        // A close pairs with the open on top only when that open is its own
        // bracket; a mismatched one pairs with nothing serially either.
        if let Some(&(open_at, open_ci, open_local, open_bk)) = open_stack.last()
            && open_bk == bk
        {
            open_stack.pop();
            paired.push((open_ci, open_local, ci, local, at, open_at));
        }
    }
    for (open_ci, open_local, close_ci, close_local, close_at, open_at) in paired {
        parts[open_ci].0.mates[open_local] = close_at;
        parts[close_ci].0.mates[close_local] = open_at;
    }
}

thread_local! {
    /// This thread's held paired-lex buffers; empty while a lex on this thread
    /// has them out.
    static HELD_PAIRED: std::cell::Cell<PairedWorkspace> =
        std::cell::Cell::new(PairedWorkspace::default());
}

/// The buffers a fused lex writes: one per chunk, and the two it fills for
/// the caller. Kept across calls, their pages stay mapped.
#[derive(Default)]
pub struct SignificantWorkspace {
    parts: Vec<Significant>,
    /// Kind codes, one per significant token of the last input.
    pub kinds: Vec<u32>,
    /// Byte spans, one per significant token of the last input.
    pub spans: Vec<(u32, u32)>,
}

/// [`lex_significant_parallel_into`] with buffers taken and dropped here.
#[must_use]
pub fn lex_significant_parallel(input: &[u8]) -> (Vec<u32>, Vec<(u32, u32)>) {
    let mut ws = SignificantWorkspace::default();
    lex_significant_parallel_into(input, &mut ws);
    (ws.kinds, ws.spans)
}

thread_local! {
    /// This thread's held fused-lex buffers; empty while a lex on this thread
    /// has them out.
    static HELD_SIGNIFICANT: std::cell::Cell<SignificantWorkspace> =
        std::cell::Cell::new(SignificantWorkspace::default());
}

/// [`lex_significant_parallel`] into this thread's held buffers, handing the
/// kinds and spans to `f`. The device phase sweep measured its 16.5 MB lex at
/// 9.0 ms into held buffers against 14.4 ms into fresh ones.
///
/// The buffers are out for the call and back after it; a lex on this thread
/// while they are out finds none and lexes into buffers of its own.
pub fn lex_significant_parallel_held<R>(
    input: &[u8],
    f: impl FnOnce(&[u32], &[(u32, u32)]) -> R,
) -> R {
    let mut ws = HELD_SIGNIFICANT.take();
    lex_significant_parallel_into(input, &mut ws);
    let out = f(&ws.kinds, &ws.spans);
    HELD_SIGNIFICANT.set(ws);
    out
}

/// [`lex_significant_parallel_held`]'s lex handing `f` each chunk's kinds and
/// absolute spans, in input order, where the chunk was lexed: the chunks are
/// never copied into one pair of arrays. Joined, the parts are the kinds and
/// spans [`lex_significant_parallel_into`] writes; below the parallel
/// threshold, or on one core, the input is one part.
///
/// The buffers are out for the call and back after it, as they are for
/// [`lex_significant_parallel_held`].
pub fn lex_significant_parts_held<R>(input: &[u8], f: impl FnOnce(&[Significant]) -> R) -> R {
    let mut ws = HELD_SIGNIFICANT.take();
    let n = fill_significant_parts(input, &mut ws);
    let out = f(&ws.parts[..n]);
    HELD_SIGNIFICANT.set(ws);
    out
}

/// The significant stream in the lexer's parts, with this thread's held
/// workspace moved to the caller rather than lent for a call.
///
/// [`lex_significant_parts_held`] puts the workspace back before it returns, so
/// nothing it lends can outlive the closure. A cursor hands matches out for as
/// long as it is kept, which is longer, and this is what it holds instead.
///
/// Dropping it returns the workspace, and scans on this thread after that find
/// their buffers already mapped again. While one is alive they do not: the
/// thread's other scans lex into buffers of their own, as they already do for
/// any lex that has the workspace out.
///
/// That is the price, and it is measured: a scan into held buffers beat the
/// same scan into fresh ones by 1.16x to 1.70x from 129 kB to 16.5 MB of tag
/// lines, and 1.11x to 1.32x on 4 MB of prose. So a cursor parked in a
/// long-lived structure costs its neighbours up to seven tenths again on their
/// lex, for exactly as long as it lives.
pub struct OwnedParts {
    /// `None` only between [`Drop`] taking it and the value going away.
    ws: Option<SignificantWorkspace>,
    /// How many of the workspace's parts hold this input's stream.
    n: usize,
}

impl OwnedParts {
    /// The parts holding the significant stream, in input order.
    #[must_use]
    pub fn parts(&self) -> &[Significant] {
        let ws = self.ws.as_ref().expect("the workspace is taken only as this is dropped");
        &ws.parts[..self.n]
    }
}

impl Drop for OwnedParts {
    fn drop(&mut self) {
        if let Some(ws) = self.ws.take() {
            HELD_SIGNIFICANT.set(ws);
        }
    }
}

/// [`lex_significant_parts_held`] with the workspace moved to the caller. See
/// [`OwnedParts`] for what that costs the thread's other scans while it lives.
#[must_use]
pub fn lex_significant_parts_owned(input: &[u8]) -> OwnedParts {
    let mut ws = HELD_SIGNIFICANT.take();
    let n = fill_significant_parts(input, &mut ws);
    OwnedParts { ws: Some(ws), n }
}

/// Lex `input`'s significant stream into `ws.parts`, returning how many of them
/// hold it. The chunks are never copied into one pair of arrays.
///
/// Held apart from the scope that lends the workspace because the two have
/// different lifetimes to serve. A caller that finishes inside the scope takes
/// [`lex_significant_parts_held`]; one that must outlive it - a cursor, which
/// hands matches out for as long as it is kept - needs this same fill with the
/// workspace moved rather than lent.
fn fill_significant_parts(input: &[u8], ws: &mut SignificantWorkspace) -> usize {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    // Three phases a scan, dividing the fill where its work divides: the
    // boundaries and the blob table are each one pass over the whole input
    // across the cores, taken before any leaf runs, and the leaves go across
    // the cores after them.
    let (bounds, blobs) = bounds_and_blobs(
        input,
        cores,
        (Some("the lex in parts: the boundaries"), Some("the lex in parts: the blob table")),
    );
    if bounds.len() <= 2 {
        if ws.parts.is_empty() {
            ws.parts.push(Significant::with_base(0, 0));
        }
        let part = &mut ws.parts[0];
        part.reset(0);
        part.reserve(input.len() / crate::lexer::TOKEN_BYTES_ESTIMATE);
        lex_chunk_significant_into(input, &chunk_blobs(&blobs, 0, input.len()), part);
        return 1;
    }
    let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
    let _leafing = crate::trace::phase("the lex in parts: a chunk a leaf");
    lex_significant_leaves(input, &ranges, &blobs, &mut ws.parts).len()
}

/// The device path's input, the significant kind codes and byte spans of
/// [`lex_parallel`]'s stream, lexed across cores straight into `ws`: no
/// token vector is built, stitched or walked again.
///
/// `ws.kinds` and `ws.spans` hold the result, equal to
/// `crate::gpu::significant_stream(&lex_parallel(input))`. The chunk
/// boundaries, blob table and serial fallback are [`lex_parallel`]'s; each
/// leaf writes its chunk's kinds and absolute spans into one of the
/// workspace's buffers, and the chunks' arrays are then copied to their slots
/// of the output across cores, as [`stitch_parallel`] copies tokens.
pub fn lex_significant_parallel_into(input: &[u8], ws: &mut SignificantWorkspace) {
    ws.kinds.clear();
    ws.spans.clear();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    // The whole-input blob table, for the reason `lex_parallel` computes it
    // once: a chunk-local gate reads a truncated window at the chunk's start.
    // It and the boundaries are taken as they are there.
    let (bounds, blobs) = bounds_and_blobs(input, cores, (None, None));
    if bounds.len() <= 2 {
        lex_significant_serial_into(input, 0, input.len(), &blobs, ws);
        return;
    }
    let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
    lex_significant_chunks_into(input, &ranges, &blobs, ws);
}

/// The fused lex of `input[s..e]` into `ws`: the significant kind codes of
/// that range, and their byte spans absolute in `input`. `blobs` is the blob
/// table of the whole of `input`, from [`crate::lexer::blob_runs_parallel`],
/// so a range reads the entropy history a whole-input lex reads. A range whose
/// ends are two of the input's [`safe_boundaries`] lexes to exactly its slice
/// of [`lex_significant_parallel_into`]'s stream; above the parallel threshold
/// it is lexed across cores, as a whole input is.
pub fn lex_significant_range_into(
    input: &[u8],
    s: usize,
    e: usize,
    blobs: &[(usize, usize)],
    ws: &mut SignificantWorkspace,
) {
    ws.kinds.clear();
    ws.spans.clear();
    lex_significant_range_onto(input, s, e, blobs, ws);
}

/// [`lex_significant_range_into`] appending to what `ws` already holds rather
/// than replacing it, so a caller handing this range to another thread joined
/// to tokens before it writes the join on the cores whose caches hold what
/// they just lexed.
pub fn lex_significant_range_onto(
    input: &[u8],
    s: usize,
    e: usize,
    blobs: &[(usize, usize)],
    ws: &mut SignificantWorkspace,
) {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let bounds = if e - s < parallel_lex_threshold() || cores <= 1 {
        Vec::new()
    } else {
        safe_boundaries(&input[s..e], chunks_for(e - s, cores))
    };
    if bounds.len() <= 2 {
        lex_significant_serial_into(input, s, e, blobs, ws);
        return;
    }
    let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (s + w[0], s + w[1])).collect();
    lex_significant_chunks_into(input, &ranges, blobs, ws);
}

/// The serial route of a fused lex: `input[s..e]` lexed on this thread into
/// the workspace's first chunk buffer, against its part of `blobs`, the whole
/// input's blob table, then copied to `ws.kinds` and `ws.spans`.
fn lex_significant_serial_into(
    input: &[u8],
    s: usize,
    e: usize,
    blobs: &[(usize, usize)],
    ws: &mut SignificantWorkspace,
) {
    let SignificantWorkspace { parts, kinds, spans } = ws;
    if parts.is_empty() {
        parts.push(Significant::with_base(0, 0));
    }
    let part = &mut parts[0];
    part.reset(s);
    part.reserve((e - s) / crate::lexer::TOKEN_BYTES_ESTIMATE);
    lex_chunk_significant_into(&input[s..e], &chunk_blobs(blobs, s, e), part);
    kinds.extend_from_slice(&part.kinds);
    spans.extend_from_slice(&part.spans);
}

/// The indices of `ranges` longest first, ranges of one length in their
/// order. A range no boundary could cut - a whitespace-free span longer than
/// a chunk - is the leaf that sets a fan-out's end, and the pool starts the
/// first index on the caller before any leaf is stolen.
fn longest_first(ranges: &[(usize, usize)]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&k| std::cmp::Reverse(ranges[k].1 - ranges[k].0));
    order
}

/// Run `lex` over `parts[k]` for every range `k` across the cores under
/// `plan`, the ranges taken in [`longest_first`] order; each part is written
/// by one leaf, once.
#[allow(unsafe_code)]
fn over_ranges_longest_first<T: Send>(
    plan: &flynnel::JobPlan,
    ranges: &[(usize, usize)],
    parts: &mut [T],
    lex: impl Fn(usize, &mut T) + Sync,
) {
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    assert_eq!(parts.len(), ranges.len(), "a part a range");
    let mut order = longest_first(ranges);
    let parts_at = parts.as_mut_ptr() as usize;
    for_each_chunk_indexed_min_leaf(plan, &mut order, 1, |_, slots| {
        for &k in slots.iter() {
            // SAFETY: `order` is a permutation of `0..parts.len()` and the
            // leaves take disjoint slices of it, so each part is reached by
            // one leaf once, and `parts` outlives the dispatch, which joins
            // before it returns.
            let part = unsafe { &mut *(parts_at as *mut T).add(k) };
            lex(k, part);
        }
    });
}

/// Lex the `ranges` of `input` across cores, each into its own chunk buffer of
/// `parts` against its part of `blobs`, the whole input's blob table, growing
/// `parts` to a buffer a range. Returns the buffers written, in range order.
#[allow(unsafe_code)]
fn lex_significant_leaves<'a>(
    input: &[u8],
    ranges: &[(usize, usize)],
    blobs: &[(usize, usize)],
    parts: &'a mut Vec<Significant>,
) -> &'a mut [Significant] {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;
    use std::sync::atomic::{AtomicU64, Ordering};

    if parts.len() < ranges.len() {
        parts.resize_with(ranges.len(), || Significant::with_base(0, 0));
    }
    let parts = &mut parts[..ranges.len()];
    for (&(s, e), part) in ranges.iter().zip(parts.iter_mut()) {
        part.reset(s);
        part.reserve((e - s) / crate::lexer::TOKEN_BYTES_ESTIMATE);
    }

    // The leaf is `lex_parallel`'s leaf writing three words a significant
    // token instead of four a token, so its per-byte estimate carries over.
    let lexed: usize = ranges.iter().map(|&(s, e)| e - s).sum();
    let avg_chunk_bytes = lexed / ranges.len().max(1);
    let per_chunk_ns = (avg_chunk_bytes as u64 * 6).min(u32::MAX as u64) as u32;
    let plan = JobPlan::new(0, ranges.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::Streaming)
        .with_estimated_per_item_ns(per_chunk_ns);
    // Each leaf clocks itself where the rungs are being kept, and the four
    // counts a scan say how the fan-out spent the phase around it: the leaves'
    // summed time over the phase is the cores it held, the longest leaf over
    // the phase is how far one leaf set the phase's length, and the latest
    // start is how long the pool took to have every leaf running.
    let dispatched = crate::trace::keeping().then(std::time::Instant::now);
    let leaves = AtomicU64::new(0);
    let leaf_nanos = AtomicU64::new(0);
    let longest = AtomicU64::new(0);
    let latest_start = AtomicU64::new(0);
    let mut order = longest_first(ranges);
    let parts_at = parts.as_mut_ptr() as usize;
    for_each_chunk_indexed_min_leaf(&plan, &mut order, 1, |_, slots| {
        let clock = dispatched.map(|dispatched| (dispatched.elapsed(), std::time::Instant::now()));
        for &k in slots.iter() {
            let (s, e) = ranges[k];
            // SAFETY: `order` is a permutation of `0..parts.len()` and the
            // leaves take disjoint slices of it, so each part is reached by
            // one leaf once, and `parts` outlives the dispatch, which joins
            // before it returns.
            let slot = unsafe { &mut *(parts_at as *mut Significant).add(k) };
            lex_chunk_significant_into(&input[s..e], &chunk_blobs(blobs, s, e), slot);
        }
        if let Some((since_dispatch, began)) = clock {
            let took = nanos(began.elapsed());
            leaves.fetch_add(1, Ordering::Relaxed);
            leaf_nanos.fetch_add(took, Ordering::Relaxed);
            longest.fetch_max(took, Ordering::Relaxed);
            latest_start.fetch_max(nanos(since_dispatch), Ordering::Relaxed);
        }
    });
    if dispatched.is_some() {
        crate::trace::counted("the lex in parts: leaves", leaves.into_inner());
        crate::trace::counted("the lex in parts: leaf nanoseconds, summed", leaf_nanos.into_inner());
        crate::trace::counted("the lex in parts: the longest leaf, nanoseconds", longest.into_inner());
        crate::trace::counted(
            "the lex in parts: the latest leaf start, nanoseconds",
            latest_start.into_inner(),
        );
    }
    parts
}

/// `d` in nanoseconds, in the width the counts hold.
fn nanos(d: std::time::Duration) -> u64 {
    u64::try_from(d.as_nanos()).expect("a leaf shorter than the five hundred years the counter holds")
}

/// Lex the `ranges` of `input` across cores with [`lex_significant_leaves`],
/// then copy the chunks' kinds and absolute spans to their slots of `ws.kinds`
/// and `ws.spans`, which the caller cleared, across cores in range order.
#[allow(unsafe_code, clippy::uninit_vec)]
fn lex_significant_chunks_into(
    input: &[u8],
    ranges: &[(usize, usize)],
    blobs: &[(usize, usize)],
    ws: &mut SignificantWorkspace,
) {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    let SignificantWorkspace { parts, kinds, spans } = ws;
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let parts = lex_significant_leaves(input, ranges, blobs, parts);

    // Prefix sum: chunk `i`'s tokens occupy `offsets[i] .. offsets[i+1]` of
    // both arrays.
    let total: usize = parts.iter().map(|p| p.kinds.len()).sum();
    if total == 0 {
        return;
    }
    // What the arrays already hold keeps its place; the chunks fill from
    // there on.
    let base = kinds.len();
    let mut offsets = Vec::with_capacity(parts.len());
    let mut acc = base;
    for p in parts.iter() {
        offsets.push(acc);
        acc += p.kinds.len();
    }
    kinds.reserve(total);
    spans.reserve(total);
    // SAFETY: every index in `base .. base + total` of both arrays is
    // written exactly once below, by a copy that does not read the
    // uninitialized slot, before `set_len` exposes the buffers to any read.
    // Each chunk writes its own range `offsets[i] .. offsets[i] + len_i` and
    // those ranges partition `base .. base + total`. A serial fill would
    // defeat the parallel write, so the uninit-then-fill pattern is
    // deliberate (clippy::uninit_vec).
    unsafe {
        kinds.set_len(base + total);
        spans.set_len(base + total);
    }
    let kinds_addr = kinds.as_mut_ptr() as usize;
    let spans_addr = spans.as_mut_ptr() as usize;
    let parts: &[Significant] = parts;
    let mut fan: Vec<u8> = vec![0; parts.len()];
    let min_leaf = parts.len().div_ceil(cores * 4).max(1);
    let plan = JobPlan::new(0, parts.len() as u32).with_leaf_shape(flynnel::LeafShape::Streaming);
    for_each_chunk_indexed_min_leaf(&plan, &mut fan, min_leaf, |start, slots| {
        let dst_kinds = kinds_addr as *mut u32;
        let dst_spans = spans_addr as *mut (u32, u32);
        for k in 0..slots.len() {
            let ci = start + k;
            let off = offsets[ci];
            let part = &parts[ci];
            // SAFETY: `off .. off + len` is chunk `ci`'s disjoint range in
            // both arrays, and the sources are the chunk's own vectors.
            unsafe {
                std::ptr::copy_nonoverlapping(part.kinds.as_ptr(), dst_kinds.add(off), part.kinds.len());
                std::ptr::copy_nonoverlapping(part.spans.as_ptr(), dst_spans.add(off), part.spans.len());
            }
        }
    });
}

/// Stitch the chunk token lists into one stream cooperatively: a prefix
/// sum assigns each chunk its slot in the output, the chunks write their
/// (offset-shifted, mate-rebased) tokens to those disjoint slots across
/// cores, and the brackets that cross a chunk boundary - the opens a chunk
/// left unclosed and the closes it met over an empty stack, which its
/// [`Seams`] name - run through the stack the serial lexer would hold there,
/// in stream order. Byte-identical to the serial lexer. Where the per-chunk
/// lexing is independent fan-out, this coordinates through the prefix sum,
/// so the O(tokens) offset and copy work parallelizes instead of running
/// serially, and the pairing pass reads the seam brackets alone.
#[must_use]
pub fn stitch_parallel(parts: &[(Vec<Token>, Seams)], ranges: &[(usize, usize)]) -> Vec<Token> {
    let mut out = Vec::new();
    stitch_parallel_into(parts, ranges, &mut out);
    out
}

/// [`stitch_parallel`] into `out`, cleared first and holding the stream
/// after.
#[allow(unsafe_code, clippy::uninit_vec)]
pub fn stitch_parallel_into(
    parts: &[(Vec<Token>, Seams)],
    ranges: &[(usize, usize)],
    out: &mut Vec<Token>,
) {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    // What a caller taking the token stream pays over one taking the parts
    // where they were lexed, named so a reading says what the copy costs the
    // engine that asked for it.
    //
    // It costs what the memory costs and no more: 2650000 tokens of sixteen
    // bytes move in 1.696 ms over `benches/engine_surface`, which is 42.4 MB
    // copied and 84.8 MB of traffic counting both sides - around fifty
    // gigabytes a second. There is no arrangement of this loop that beats the
    // bus, so the only way past it is for a caller not to ask for one array.
    let _copying = crate::trace::phase("the stitch into one array");
    out.clear();
    let nchunks = parts.len();
    let total: usize = parts.iter().map(|(toks, _)| toks.len()).sum();
    if total == 0 {
        return;
    }
    // Prefix sum: chunk `i`'s tokens occupy `out[offsets[i] .. offsets[i+1]]`.
    let mut offsets = Vec::with_capacity(nchunks);
    let mut acc = 0usize;
    for (toks, _) in parts {
        offsets.push(acc);
        acc += toks.len();
    }

    out.reserve(total);
    // SAFETY: every index in `0..total` is written exactly once below, via
    // `ptr::write` (which does not read the uninitialized slot), before
    // `set_len` exposes the buffer and before any read. Each chunk writes
    // its own range `out[offsets[i] .. offsets[i]+len_i]` and those ranges
    // partition `0..total`. A serial fill would defeat the parallel write,
    // so the uninit-then-fill pattern is deliberate (clippy::uninit_vec).
    unsafe {
        out.set_len(total);
    }
    let out_addr = out.as_mut_ptr() as usize;

    // The chunks write their disjoint slots in parallel: shift byte offsets
    // to absolute and re-base each intra-chunk mate index to global. A small
    // index array drives the fan-out; the writes never alias because the
    // ranges are disjoint.
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let mut fan: Vec<u8> = vec![0; nchunks];
    let min_leaf = nchunks.div_ceil(cores * 4).max(1);
    // Rebasing walks each chunk's tokens and writes them into the output, so
    // it is streaming for the same reason the lex itself is.
    let plan = JobPlan::new(0, nchunks as u32).with_leaf_shape(flynnel::LeafShape::Streaming);
    for_each_chunk_indexed_min_leaf(&plan, &mut fan, min_leaf, |start, slots| {
        let dst = out_addr as *mut Token;
        for k in 0..slots.len() {
            let ci = start + k;
            let base = ranges[ci].0;
            let off = offsets[ci];
            for (j, t) in parts[ci].0.iter().enumerate() {
                let mut tok = *t;
                tok.shift(base);
                tok.set_mate(tok.mate().map(|m| off + m));
                // SAFETY: `off + j` lies in chunk `ci`'s disjoint range.
                unsafe {
                    dst.add(off + j).write(tok);
                }
            }
        }
    });

    // Pair the brackets that cross a chunk boundary: each chunk's unclosed
    // opens and the closes it met over an empty stack, in stream order,
    // through the stack the serial lexer holds there. Every other bracket
    // was paired inside its chunk or pairs with nothing serially.
    let mut open_stack: Vec<usize> = Vec::new();
    for (ci, (_, seam)) in parts.iter().enumerate() {
        let off = offsets[ci];
        let (mut oi, mut cj) = (0, 0);
        while oi < seam.open.len() || cj < seam.close.len() {
            let open_next = seam.open.get(oi).is_some_and(|&o| cj >= seam.close.len() || o < seam.close[cj]);
            if open_next {
                open_stack.push(off + seam.open[oi]);
                oi += 1;
            } else {
                let idx = off + seam.close[cj];
                if let TokenKind::Close(bk) = out[idx].kind
                    && let Some(&open_idx) = open_stack.last()
                    && out[open_idx].kind == TokenKind::Open(bk)
                {
                    open_stack.pop();
                    out[open_idx].set_mate(Some(idx));
                    out[idx].set_mate(Some(open_idx));
                }
                cj += 1;
            }
        }
    }
}

/// The chunk boundaries and the whole-input blob table: the two passes a
/// parallel lex makes over the whole input before any leaf runs.
///
/// They run one after the other. Forked under one join, measured on pc2 over
/// 40 slices and whole files of six real corpora, the pair lost to the same two
/// passes in turn on 6 of 12 inputs of 128 kB, by 4 to 13 percent, and on a
/// 2 MB log tail by 8 to 10: where one pass is short, the fork's dispatch costs
/// more than overlapping it saves.
///
/// Below the parallel threshold, or on one core, there are no boundaries; and
/// where no boundary is found the lex that follows is serial, so the blob
/// table is the serial one.
///
/// `phases` names each pass in the trace as its caller names it, and a pass its
/// caller never timed stays untimed.
fn bounds_and_blobs(
    input: &[u8],
    cores: usize,
    phases: (Option<&'static str>, Option<&'static str>),
) -> (Vec<usize>, Vec<(usize, usize)>) {
    if input.len() < parallel_lex_threshold() || cores <= 1 {
        return (Vec::new(), crate::lexer::blob_runs(input));
    }
    let (bounding, tabling) = phases;
    let bounds = {
        let _timed = bounding.map(crate::trace::phase);
        safe_boundaries(input, chunks_for(input.len(), cores))
    };
    let blobs = {
        let _timed = tabling.map(crate::trace::phase);
        if bounds.len() <= 2 {
            crate::lexer::blob_runs(input)
        } else {
            crate::lexer::blob_runs_parallel(input)
        }
    };
    (bounds, blobs)
}

/// The chunk boundaries `[0, b1, b2, ..., len]` for `target_chunks`
/// roughly even chunks, each interior boundary a guaranteed serial token
/// boundary: near each ideal division, a SIMD newline search finds the next
/// newline outside every double-quoted string, and the boundary is the first
/// significant byte after it. The whitespace run ends exactly there and no
/// string crosses it, so the chunk before and the chunk after lex as the
/// serial lexer would.
///
/// Public so the cost of this prescan can be timed on its own. It runs
/// before any chunk is dispatched and reads the whole input for its quotes,
/// across the cores through [`quoted_spans_across`], so it is one of the fixed
/// costs the parallel route pays and the serial route does not.
#[must_use]
pub fn safe_boundaries(input: &[u8], target_chunks: usize) -> Vec<usize> {
    let n = input.len();
    let quotes = quoted_spans_across(input, QUOTE_SCAN_MIN_LEAF);
    // The string holding `p`, when one does.
    let holding = |p: usize| {
        let k = quotes.partition_point(|&(open, _)| open <= p);
        (k > 0 && quotes[k - 1].1 >= p).then(|| quotes[k - 1])
    };
    let mut picked: Vec<usize> = Vec::new();
    // Where the divisions' cuts came from, counted locally and handed to the
    // trace once: a division cut at a newline, one cut at a whitespace run,
    // one left uncut, and the whitespace runs the fallback walked to find out.
    let (mut at_newline, mut at_space, mut uncut, mut runs_walked) = (0u64, 0u64, 0u64, 0u64);
    for k in 1..target_chunks {
        let mut from = k * n / target_chunks;
        // The search stops at the next division, whose own search starts
        // there, so the divisions together read the input once even when no
        // newline lies ahead of them.
        let limit = ((k + 1) * n / target_chunks).min(n);
        let before = picked.len();
        while from < limit {
            let Some(rel) = crate::byte_simd::find(&input[from..limit], b"\n") else {
                break;
            };
            let newline = from + rel;
            // A newline can be the char a char literal holds - a quote on each
            // side, as wiki bold markup puts them, or a quote and a backslash
            // before it - and the lexer reads the literal as one token across
            // the line. The quote scan passes char literals over without
            // recording them, so `holding` cannot see one, and the cut after
            // such a newline is refused here.
            let held = |at: usize| crate::lexer::char_literal_end(input, at) == Some(newline + 2);
            if (newline >= 1 && held(newline - 1)) || (newline >= 2 && held(newline - 2)) {
                from = newline + 1;
                continue;
            }
            let mut b = newline + 1;
            while b < n && input[b].is_ascii_whitespace() {
                b += 1;
            }
            if b >= n {
                break;
            }
            // A newline inside a string, or a boundary that would open one,
            // is passed over for the next newline after that string.
            match holding(newline).or_else(|| holding(b)) {
                Some((_, close)) => from = close + 1,
                None => {
                    picked.push(b);
                    at_newline += 1;
                    break;
                }
            }
        }
        if picked.len() == before {
            // No newline in the window: a whitespace run ends the chunk
            // instead, where the byte before the run is neither a quote nor
            // a backslash (a char literal holding a space), the significant
            // byte after it is neither a digit nor a sign, which is what
            // every recognizer that crosses whitespace - a card's groups, a
            // phone's groups, a coordinate pair - continues with, and the
            // run is not the one space a quantity's unit symbol stands
            // after its number.
            let mut from = k * n / target_chunks;
            while from < limit {
                let w = from + crate::byte_simd::nonspace_run(&input[from..limit]);
                if w >= limit {
                    break;
                }
                runs_walked += 1;
                let p = w + crate::byte_simd::space_run(&input[w..]);
                // A run inside a string, or one a string opens after, is never
                // a cut, and neither is any run before that string closes, so
                // the search goes on past the close. Refusing the runs inside
                // one by one instead costs a quarter of a million runs on 4 MB
                // of code the lexer reads as 69% string.
                if let Some((_, close)) = holding(w).or_else(|| holding(p)) {
                    from = close + 1;
                    continue;
                }
                let unit_after_a_number = p == w + 1
                    && input[w] == b' '
                    && input[w - 1].is_ascii_digit()
                    && crate::quantity::spaced_symbol_len(input, p).is_some();
                let safe = p < n
                    && w > 0
                    && !matches!(input[w - 1], b'\'' | b'\\')
                    && !input[p].is_ascii_digit()
                    && !matches!(input[p], b'-' | b'+')
                    && !unit_after_a_number;
                if safe {
                    picked.push(p);
                    at_space += 1;
                    break;
                }
                from = p;
            }
            if picked.len() == before {
                uncut += 1;
            }
        }
    }
    crate::trace::counted("the boundaries: divisions cut at a newline", at_newline);
    crate::trace::counted("the boundaries: divisions cut at a whitespace run", at_space);
    crate::trace::counted("the boundaries: divisions left uncut", uncut);
    crate::trace::counted("the boundaries: whitespace runs the fallback walked", runs_walked);
    picked.sort_unstable();
    picked.dedup();

    let mut bounds = Vec::with_capacity(picked.len() + 2);
    bounds.push(0);
    bounds.extend(picked);
    bounds.push(n);
    bounds.dedup();
    bounds
}

/// The strings of `input` as inclusive `[open, close]` byte ranges,
/// ascending, read as the lexer reads them. A double quote outside a string
/// opens one under [`crate::lexer::double_quoted_end`]'s rule, closed on its
/// line with a backslash escaping the byte or the CRLF after it, or is nothing.
/// A single quote outside a string opens one under
/// [`crate::lexer::single_quoted_end`]'s rule, closed on its line, or is a char
/// literal passed over whole with the quote it may hold, or is nothing. Only
/// the quote bytes are visited, each read off a SIMD classification of the
/// input a block at a time, and a double-quoted string's close is read the
/// same way, off its double quotes and newlines
/// ([`crate::lexer::double_quoted_end_at`]), rather than walked to.
///
/// Public so a bench can time it against [`quoted_spans_across`].
#[must_use]
pub fn quoted_spans(input: &[u8]) -> Vec<(usize, usize)> {
    match quoted_spans_where(input, |_| Some(true)) {
        Some(spans) => spans,
        None => unreachable!("a rule that settles every quote abandons no reading"),
    }
}

/// [`quoted_spans`] where a quote outside a string is read only if `opens`
/// says so of its position: `Some(false)` where the lexer takes that quote
/// inside another token, so it is passed over as a plain byte, `None` where
/// it cannot be settled, which abandons the whole reading.
pub(crate) fn quoted_spans_where(
    input: &[u8],
    mut opens: impl FnMut(usize) -> Option<bool>,
) -> Option<Vec<(usize, usize)>> {
    let read = quote_scan_where(input, 0, input.len(), &mut opens)?;
    Some(read.spans)
}

/// What the quote scan reads over one stretch of the input.
#[derive(Clone, Default)]
struct QuoteRead {
    /// The strings it read, in order, as inclusive `[open, close]` ranges.
    spans: Vec<(usize, usize)>,
    /// Where the search for the next quote resumes: past the stretch's end
    /// where a string or char literal passed over whole reaches beyond it.
    from: usize,
}

/// The scan [`quoted_spans_where`] makes, over the quotes at `from..to` of
/// `input`, begun outside every string. It reads bytes past `to` as the whole
/// scan would, for a string or a char literal opening before it, and visits no
/// quote at or past it.
fn quote_scan_where(
    input: &[u8],
    from: usize,
    to: usize,
    opens: &mut impl FnMut(usize) -> Option<bool>,
) -> Option<QuoteRead> {
    let mut spans = Vec::new();
    // The next quote byte of either kind at or past `from` and below `to`, read
    // off a mask a block at a time: a string or a char literal passed over
    // moves `from` to its end, and the quotes inside it are never visited. The
    // search is over the input up to `to` and no further, since a search that
    // runs on past it finds nothing this stretch reads and, on input holding
    // no quote after it, walks every block to the input's end.
    let mut quotes = crate::byte_simd::QuotePositions::new(&input[..to]);
    // Where a double-quoted string closes or ends its line, read the same way
    // over the whole input, since a string opening before `to` is read to its
    // close wherever that lies.
    let mut closes = crate::byte_simd::CloseOrNewlinePositions::new(input);
    let mut from = from;
    while let Some(p) = quotes.next_at_or_after(from) {
        from = p + 1;
        if input[p] == b'\'' {
            if let Some(end) = crate::lexer::char_literal_end(input, p) {
                if opens(p)? {
                    from = end;
                }
            } else if let Some(end) = crate::lexer::single_quoted_end(input, p)
                && opens(p)?
            {
                spans.push((p, end - 1));
                from = end;
            }
        } else if let Some(end) = crate::lexer::double_quoted_end_at(input, p, &mut closes)
            && opens(p)?
        {
            spans.push((p, end - 1));
            from = end;
        }
    }
    Some(QuoteRead { spans, from })
}

/// [`quote_scan_where`] under [`quoted_spans`]'s rule, which reads every quote.
fn quote_scan(input: &[u8], from: usize, to: usize) -> QuoteRead {
    match quote_scan_where(input, from, to, &mut |_| Some(true)) {
        Some(read) => read,
        None => unreachable!("a rule that settles every quote abandons no reading"),
    }
}

/// Where the pieces of [`quoted_spans_across`] start: `0`, then, from every
/// `leaf_bytes` on, the byte after the first newline no backslash stands
/// before, directly or before its CR - or `None` where one of those newlines
/// is not within [`QUOTE_SCAN_MIN_LEAF`] of where it was looked for, or
/// `leaf_bytes` where that is shorter.
///
/// No string is open across such a newline. A string of either quote ends on
/// its line - a backslash is all that carries one past a newline - and a char
/// literal reaches past it only where the newline is the char it holds, which
/// [`quoted_spans_across`] finds as the scan resuming past the cut.
///
/// An input whose newlines are further apart than a piece divides into pieces
/// few and uneven, and finding each cut is a search as long as the gap: the
/// UTF-16 table in the measured set holds 29 newlines in 31 MB, and on 2 MB of
/// it the pieces lost to the whole scan. So the first search to come up empty
/// gives the input back to be read whole. Each search reaches no further than
/// the least a piece holds: one the length of a whole piece, 325 kB on that
/// table, finds a few of its newlines before one misses and costs the scan 4
/// percent, where one this long costs it a microsecond. A file with a line
/// longer than that is read whole too - the measured code and data hold lines
/// of 108 kB and 294 kB, which a piece's length misses as well.
fn quote_piece_starts(input: &[u8], leaf_bytes: usize) -> Option<Vec<usize>> {
    let n = input.len();
    let reach = leaf_bytes.min(QUOTE_SCAN_MIN_LEAF);
    let mut starts = vec![0usize];
    let mut at = leaf_bytes;
    while at < n {
        let window_end = (at + reach).min(n);
        let mut from = at;
        let mut start = None;
        while let Some(rel) = crate::byte_simd::find(&input[from..window_end], b"\n") {
            let newline = from + rel;
            if !crate::lexer::backslash_before_newline(input, newline) {
                start = Some(newline + 1);
                break;
            }
            from = newline + 1;
        }
        match start {
            Some(s) if s < n => {
                starts.push(s);
                at = s + leaf_bytes;
            }
            // The last newline in reach ends the input: nothing is left to
            // cut, and the pieces found so far are the whole of it.
            Some(_) => break,
            None if window_end == n => break,
            None => return None,
        }
    }
    Some(starts)
}

/// [`quoted_spans`] read across the cores: the same spans.
///
/// The input is cut after newlines no backslash stands before
/// ([`quote_piece_starts`]), where the scan is outside every string, and the
/// pieces are read at once. They are then taken in order; one the scan enters
/// past its first byte, through a char literal holding the newline it was cut
/// at, is read again from there.
///
/// No piece holds fewer than `least_a_leaf` bytes. An input under
/// [`QUOTE_SCAN_PARALLEL_BYTES`], one that does not divide into two pieces, and
/// one whose newlines lie too far apart to cut at are read as [`quoted_spans`]
/// reads them.
///
/// Public so a bench can time it against [`quoted_spans`], and sweep the least
/// a piece holds.
#[must_use]
pub fn quoted_spans_across(input: &[u8], least_a_leaf: usize) -> Vec<(usize, usize)> {
    if input.len() < QUOTE_SCAN_PARALLEL_BYTES {
        return quoted_spans(input);
    }
    quoted_spans_in_pieces(input, quote_piece_bytes(input.len(), least_a_leaf))
}

/// How many pieces [`quoted_spans_across`] reads `input` in, one where it reads
/// it whole. Public so a bench states the route each timing took.
#[must_use]
pub fn quote_scan_pieces(input: &[u8], least_a_leaf: usize) -> usize {
    if input.len() < QUOTE_SCAN_PARALLEL_BYTES {
        return 1;
    }
    quote_piece_starts(input, quote_piece_bytes(input.len(), least_a_leaf)).map_or(1, |s| s.len())
}

/// The length a piece of `n` bytes' quote scan aims at: four pieces a core,
/// and never under `least_a_leaf`.
fn quote_piece_bytes(n: usize, least_a_leaf: usize) -> usize {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    n.div_ceil(cores * 4).max(least_a_leaf).max(1)
}

/// [`quoted_spans_across`] over pieces of at least `leaf_bytes`, whatever the
/// core count and the input's size, so a test can cut at every newline the rule
/// allows.
fn quoted_spans_in_pieces(input: &[u8], leaf_bytes: usize) -> Vec<(usize, usize)> {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    let n = input.len();
    let Some(starts) = quote_piece_starts(input, leaf_bytes) else {
        return quoted_spans(input);
    };
    if starts.len() <= 1 {
        return quoted_spans(input);
    }
    let ends: Vec<usize> = starts[1..].iter().copied().chain(std::iter::once(n)).collect();
    let mut read: Vec<QuoteRead> = vec![QuoteRead::default(); starts.len()];
    // What a piece costs, for the pool's own choice between the cores and
    // inline: one reading of its bytes at 0.15 ns a byte, the rate this quote
    // scan reads the 4,787,795 bytes of real_code.txt at whole, 0.724 ms.
    let per_piece_ns = (leaf_bytes as u64 * 150 / 1000).min(u64::from(u32::MAX)) as u32;
    let pieces = u32::try_from(starts.len()).expect("a piece count of at most four a core, and one");
    let plan = JobPlan::new(0, pieces)
        .with_leaf_shape(flynnel::LeafShape::Streaming)
        .with_estimated_per_item_ns(per_piece_ns);
    for_each_chunk_indexed_min_leaf(&plan, &mut read, 1, |base, slots| {
        for (i, slot) in slots.iter_mut().enumerate() {
            let k = base + i;
            *slot = quote_scan(input, starts[k], ends[k]);
        }
    });

    let mut spans = Vec::new();
    let mut from = 0usize;
    for (k, piece) in read.iter().enumerate() {
        let entered_past_start;
        let piece = if from > starts[k] {
            entered_past_start = quote_scan(input, from, ends[k]);
            &entered_past_start
        } else {
            piece
        };
        spans.extend_from_slice(&piece.spans);
        from = piece.from;
    }
    spans
}

/// [`quoted_spans_where`] paused and resumed, so a caller that asks about one
/// position pays for the bytes up to it and not for the whole input.
///
/// The byte routes need to know whether an occurrence sits inside a string, and
/// a string's extent is only knowable by reading from the start. Reading to the
/// end as well is what this avoids: a literal thirty bytes in costs thirty
/// bytes of quote scanning, where the eager form costs the input.
///
/// The loop is the one above, stopped between iterations. Resuming it continues
/// from the same state, so the spans it yields for a prefix are the spans the
/// eager scan yields for that prefix, which a test asserts directly.
pub(crate) struct QuoteScan<'a> {
    input: &'a [u8],
    spans: Vec<(usize, usize)>,
    /// The next quote of each kind at or after the scan's position and below
    /// [`Self::searched_to`], or none where there is none in that range.
    next_double: Option<usize>,
    next_single: Option<usize>,
    /// Bytes searched for quote bytes, or passed over inside a string or char
    /// literal read whole. Both fields above are decided over everything below
    /// this and nothing above it.
    ///
    /// Searching to the end instead is what the resumable form exists to avoid,
    /// and looking for the leftmost quote is still a search: on an input holding
    /// no quote at all, an unbounded search for each kind reads every byte twice
    /// to report that there is nothing, which every anchored route then pays
    /// before asking its first question.
    searched_to: usize,
    /// Bytes whose quoting is decided: no span starting below this is missing
    /// from `spans`.
    settled_to: usize,
    /// Where each double-quoted string closes or ends its line, read a block
    /// at a time from the string's opening quote and no further than its close.
    closes: crate::byte_simd::CloseOrNewlinePositions<'a>,
    /// The scan reached a quote it could not settle, which is the reader's
    /// answer to everything.
    unsettled: bool,
    done: bool,
}

impl<'a> QuoteScan<'a> {
    pub(crate) fn new(input: &'a [u8]) -> Self {
        QuoteScan {
            input,
            spans: Vec::new(),
            next_double: None,
            next_single: None,
            searched_to: 0,
            settled_to: 0,
            closes: crate::byte_simd::CloseOrNewlinePositions::new(input),
            unsettled: false,
            done: false,
        }
    }

    /// Whether a quote the rule could not settle has been reached so far.
    pub(crate) fn unsettled(&self) -> bool {
        self.unsettled
    }

    /// The spans decided so far, in order.
    pub(crate) fn spans(&self) -> &[(usize, usize)] {
        &self.spans
    }

    /// How far the scan has searched for quote bytes, which is what a test
    /// holding it to reading no more than it was asked about reads.
    #[cfg(test)]
    pub(crate) fn searched_to(&self) -> usize {
        self.searched_to
    }

    /// Read far enough to decide every span starting at or before `upto`.
    ///
    /// A string opening at or before `upto` is read to its close before
    /// returning, which lies on its own line.
    pub(crate) fn ensure(&mut self, upto: usize, opens: &mut impl FnMut(usize) -> Option<bool>) {
        let n = self.input.len();
        let input = self.input;
        let next = |from: usize, to: usize, quote: u8| {
            (from < to)
                .then(|| crate::byte_simd::find(&input[from..to], &[quote]).map(|r| from + r))
                .flatten()
        };
        while !self.done && self.settled_to <= upto {
            // How far this ask needs read: to the position asked about, and
            // never less than twice what is already read. One ask wants the
            // least, but a scan asks at every occurrence of its literal and
            // wants the fewest: bounding each of fifty thousand ascending asks
            // to its own position turned two searches of the input into a
            // hundred thousand small ones, and measured the anchor search at
            // 3.3351 ms against 1.5964. Doubling reads the same bytes in a
            // logarithmic number of searches, and still stops within one
            // doubling of a lone ask.
            let horizon = upto.saturating_add(1).max(self.searched_to.saturating_mul(2)).min(n);
            // Neither kind occurs below what has been searched, so searching
            // further is the only way to find the next quote - or to learn that
            // the horizon holds none.
            if self.next_double.is_none() && self.next_single.is_none() && self.searched_to < horizon
            {
                self.next_double = next(self.searched_to, horizon, b'"');
                self.next_single = next(self.searched_to, horizon, b'\'');
                self.searched_to = horizon;
            }
            let p = match (self.next_double, self.next_single) {
                (Some(d), Some(s)) => d.min(s),
                (Some(q), None) | (None, Some(q)) => q,
                (None, None) => {
                    // No quote of either kind below the horizon, so every span
                    // starting below it is already held. Only a horizon at the
                    // end finishes the scan; a shorter one leaves it resumable.
                    if horizon == n {
                        self.done = true;
                    }
                    self.settled_to = horizon;
                    break;
                }
            };
            // The search may run past the ask, because it doubles; deciding
            // must not. A quote above `upto` is left where it was found, so a
            // later ask processes it - which is what lets the reader say a
            // quote past the last position asked about has not been reached,
            // and cannot spoil an answer that never depended on it. Nothing
            // below `p` is quoted, so the ask is answered.
            if p > upto {
                self.settled_to = p;
                break;
            }
            let mut from = p + 1;
            // What the quote at `p` reads as: the byte past the string or char
            // literal it opens, and whether that is a string, or nothing.
            let read = if input[p] == b'\'' {
                crate::lexer::char_literal_end(input, p)
                    .map(|end| (end, false))
                    .or_else(|| crate::lexer::single_quoted_end(input, p).map(|end| (end, true)))
            } else {
                crate::lexer::double_quoted_end_at(input, p, &mut self.closes).map(|end| (end, true))
            };
            if let Some((end, string)) = read {
                match opens(p) {
                    Some(true) => {
                        if string {
                            self.spans.push((p, end - 1));
                        }
                        from = end;
                    }
                    Some(false) => {}
                    None => {
                        self.unsettled = true;
                        self.done = true;
                        return;
                    }
                }
            }
            self.settled_to = from;
            // A string or char literal read whole can reach past the bytes
            // searched, and a quote inside it is no quote the scan reads: the
            // next search starts past it, not where the last one stopped.
            if from > self.searched_to {
                self.searched_to = from;
            }
            if self.next_double.is_some_and(|q| q < from) {
                self.next_double = next(from, self.searched_to, b'"');
            }
            if self.next_single.is_some_and(|q| q < from) {
                self.next_single = next(from, self.searched_to, b'\'');
            }
        }
    }
}

/// [`quoted_spans`] read through the resumable scan the byte routes ask, asked
/// about every `step` bytes in ascending order and then about the input's end:
/// the same spans.
///
/// Public so a bench can time the scan a route pays for beside the eager one.
#[must_use]
pub fn quoted_spans_resumed(input: &[u8], step: usize) -> Vec<(usize, usize)> {
    let mut scan = QuoteScan::new(input);
    let mut always = |_: usize| Some(true);
    for upto in (0..input.len()).step_by(step.max(1)) {
        scan.ensure(upto, &mut always);
    }
    scan.ensure(input.len(), &mut always);
    scan.spans().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{lex, lex_chunk};

    #[test]
    fn owning_the_parts_gives_the_lent_stream_and_hands_the_workspace_back() {
        // The owned form moves this thread's workspace where the held form
        // lends it. Two things have to hold: the stream is the same, and the
        // workspace comes back - a Drop that fails to return it strands the
        // buffers, which no other test would notice because every scan stays
        // correct and only gets slower.
        fn flat(parts: &[Significant]) -> Vec<(u32, u32, u32)> {
            parts
                .iter()
                .flat_map(|p| p.kinds.iter().zip(&p.spans).map(|(k, s)| (*k, s.0, s.1)))
                .collect()
        }
        let mut big = String::new();
        for i in 0..20_000u32 {
            big.push_str(&format!("let value_{i} = {} ; call_{i}(a, \"q\") ;\n", i * 7));
        }
        for input in [b"" as &[u8], b"x", b"a 1 b 2\n", big.as_bytes()] {
            let lent = lex_significant_parts_held(input, flat);
            let owned = lex_significant_parts_owned(input);
            assert_eq!(flat(owned.parts()), lent, "{} bytes", input.len());
            drop(owned);
            // The workspace is back, so the held form finds it and answers the
            // same again. A stranded workspace still answers correctly, so this
            // pins the return by asking twice rather than by timing.
            assert_eq!(lex_significant_parts_held(input, flat), lent);
        }
        // Nested: the inner call finds the workspace out and lexes into its own
        // buffers, which is the documented cost rather than a failure.
        let outer = lex_significant_parts_owned(b"a 1 b 2\n");
        let inner = lex_significant_parts_owned(b"a 1 b 2\n");
        assert_eq!(flat(outer.parts()), flat(inner.parts()));
    }

    /// The quote scan read in pieces is the scan read whole, cut at every
    /// newline the rule allows and at coarser divisions, over real source and
    /// over every construct that carries the scan's state across a newline.
    #[test]
    fn the_quote_scan_in_pieces_is_the_whole_scan() {
        let own: &[u8] = include_bytes!("parallel_lex.rs");
        let lexer_src: &[u8] = include_bytes!("lexer.rs");
        // Each construct repeated, so pieces start inside every one of them.
        let mut edges = String::new();
        for i in 0..40 {
            // A double-quoted string carried over three lines by backslashes,
            // holding an escaped quote and closing after an escaped backslash.
            edges.push_str(&format!("let s{i} = \"first line\\\nsecond \\\" line\\\nthird \\\\\";\n"));
            // A char literal holding the newline itself, the one reach past a
            // cut that no backslash marks.
            edges.push_str("let c = '\n';\n");
            // A single-quoted string carried past its line by a backslash.
            edges.push_str("say 'held \\\nover' done\n");
            // Both kinds of string carried past a line by a backslash before a
            // CRLF.
            edges.push_str(&format!("let r{i} = \"crlf\\\r\nnext\";\r\n"));
            edges.push_str("say 'crlf \\\r\nheld' done\r\n");
            // A backslash before a newline outside every string.
            edges.push_str("macro \\\nnext\n");
            // An apostrophe that opens nothing, and a double quote whose line
            // holds no close, which opens nothing either.
            edges.push_str("don't \"quote runs\n on here\n");
        }
        let mut open_end = edges.clone();
        open_end.push_str("tail \"never closed\nat all\n");
        // A piece's search for its cut reaches one piece's length, so the
        // pieces here are longer than any line of the input they cut.
        for (name, input, leaves) in [
            ("this file", own, &[256usize, 1000, 4096, 1 << 20][..]),
            ("lexer.rs", lexer_src, &[256, 1000, 4096, 1 << 20]),
            ("the edges", edges.as_bytes(), &[48, 64, 256, 1000]),
            ("a quote left open at the end", open_end.as_bytes(), &[48, 64, 256]),
            ("no newline", b"a \"b\" 'c' \"d" as &[u8], &[1, 4, 64]),
            ("empty", b"", &[1, 64]),
        ] {
            let whole = quoted_spans(input);
            for &leaf in leaves {
                assert_eq!(quoted_spans_in_pieces(input, leaf), whole, "{name}, {leaf} bytes a piece");
            }
            assert_eq!(quoted_spans_across(input, 1), whole, "{name}, across the cores");
        }
        // Every piece length from 40 to 104 over the edges, whose lines are all
        // shorter than 40 bytes: the cuts move a byte at a time through the
        // repeating block, so across the sweep one falls after every newline in
        // it, the one a char literal holds among them.
        let input = edges.as_bytes();
        let whole = quoted_spans(input);
        let mut cut_after = std::collections::BTreeSet::new();
        for leaf in 40..=104 {
            assert_eq!(quoted_spans_in_pieces(input, leaf), whole, "the edges, {leaf} bytes a piece");
            let starts = quote_piece_starts(input, leaf).expect("every line of the edges is shorter than a piece");
            // The cut rule itself: no piece starts after a newline a backslash
            // stands before, directly or before its CR, and every piece but the
            // first starts after one.
            for &s in &starts[1..] {
                assert_eq!(input[s - 1], b'\n', "a piece at {s} starts after a newline");
                assert!(
                    !crate::lexer::backslash_before_newline(input, s - 1),
                    "a piece at {s} starts after an escaped newline"
                );
                cut_after.insert(s - 1);
            }
        }
        let held = input.windows(3).position(|w| w == b"'\n'").expect("the edges hold a char literal holding a newline");
        assert!(cut_after.contains(&(held + 1)), "no cut fell after the newline a char literal holds");
    }

    /// Newlines further apart than a piece give the input back to be read
    /// whole, having searched no further than one piece's length for a cut.
    #[test]
    fn newlines_too_far_apart_to_cut_at_are_read_whole() {
        let mut sparse = "x".repeat(5000);
        sparse.push_str(" \"a\" 'b'\n");
        sparse.push_str(&"y".repeat(5000));
        let input = sparse.as_bytes();
        assert_eq!(quote_piece_starts(input, 1000), None, "a piece's search came up empty and was not refused");
        assert_eq!(quoted_spans_in_pieces(input, 1000), quoted_spans(input));
        // The same bytes with a newline every forty cut into pieces.
        let lines: String = (0..500).map(|i| format!("line {i} \"q\" 'c' ok\n")).collect();
        let cut = quote_piece_starts(lines.as_bytes(), 1000).expect("newlines every line are within a piece");
        assert!(cut.len() >= 5, "a dense input divides into pieces: {} starts", cut.len());
    }

    #[test]
    fn the_resumable_quote_scan_agrees_with_the_eager_one() {
        // The point of the resumable form is that it reads less, and it is
        // only allowed to read less if it decides the same thing. This holds
        // it to the eager scan: the spans it has decided must be that scan's
        // spans, in order, at every point along the way and at the end.
        let mut quoted = String::new();
        for i in 0..400 {
            match i % 5 {
                0 => quoted.push_str(&format!("let a_{i} = \"str {i}\" ;\n")),
                1 => quoted.push_str(&format!("call_{i}('c', \"esc \\\" still in\", {i}) ;\n")),
                2 => quoted.push_str(&format!("url_{i} = 'http://x/{i}' ;\n")),
                3 => quoted.push_str(&format!("plain_{i} = {i} ;\n")),
                _ => quoted.push_str(&format!("q_{i} = \"trailing \\\\\" ;\n")),
            }
        }
        // An input holding no quote at all, and one holding none until its last
        // line. The scan reads to a horizon rather than to the end, so these
        // are where a horizon that stopped too early would report a quote it
        // had not reached as absent.
        let mut bare = String::new();
        for i in 0..400 {
            bare.push_str(&format!("let value_{i} = {i} ; call_{i}(alpha, beta) ;\n"));
        }
        let mut late = bare.clone();
        late.push_str("tail = \"only string\" ;\n");

        for (name, src) in [("quoted", &quoted), ("quote-free", &bare), ("quote-late", &late)] {
            let input = src.as_bytes();
            let eager = quoted_spans(input);
            assert_eq!(
                eager.is_empty(),
                name == "quote-free",
                "{name}: the corpus must exercise what it is for"
            );

            let mut scan = QuoteScan::new(input);
            let mut always = |_: usize| Some(true);
            for upto in [0usize, 1, 40, 200, 1000, 5000, input.len() / 2, input.len() - 1, input.len()]
            {
                scan.ensure(upto, &mut always);
                assert!(!scan.unsettled(), "{name}: this corpus settles every quote");
                let got = scan.spans();
                assert!(got.len() <= eager.len(), "{name}: decided more spans than exist at {upto}");
                assert_eq!(
                    got,
                    &eager[..got.len()],
                    "{name}: the decided prefix at {upto} is not the eager scan's"
                );
                // Everything the eager scan starts at or before the decided
                // point must already be here, or the caller would read a
                // position this scan wrongly calls unquoted.
                let owed = eager.iter().filter(|s| s.0 <= upto).count();
                assert!(
                    got.len() >= owed,
                    "{name} at {upto}: decided {} of the {owed} spans owed",
                    got.len()
                );
            }
            scan.ensure(input.len(), &mut always);
            assert_eq!(scan.spans(), &eager[..], "{name}: the finished scans differ");
        }
    }

    /// A scan asked about one position has read no further than it needs to
    /// decide that position, which is the whole reason it is resumable.
    ///
    /// Asserted on an input holding no quote, because that is where an
    /// unbounded search costs the most and shows the least: it reads every byte
    /// to report that there is nothing.
    #[test]
    fn the_resumable_quote_scan_reads_no_further_than_it_was_asked() {
        let mut src = String::new();
        for i in 0..4000 {
            src.push_str(&format!("let value_{i} = {i} ; call_{i}(alpha, beta) ;\n"));
        }
        let input = src.as_bytes();
        assert!(input.len() > 100_000, "the input is long enough for the ask to be a small part");
        let mut scan = QuoteScan::new(input);
        let mut always = |_: usize| Some(true);
        scan.ensure(64, &mut always);
        assert!(
            scan.searched_to() <= 65,
            "an ask about byte 64 read {} bytes of {}",
            scan.searched_to(),
            input.len()
        );
        // And it still answers: nothing below the ask is quoted.
        assert!(scan.spans().is_empty());
        // The next ask reads at least twice as far, so a scan asking at every
        // occurrence of its literal costs a logarithmic number of searches
        // rather than one each.
        scan.ensure(65, &mut always);
        assert!(
            scan.searched_to() >= 130,
            "a second ask read only to {}, so ascending asks do not double",
            scan.searched_to()
        );
    }

    /// A quote the rule cannot settle, far past the ask, must not be reached.
    ///
    /// The search doubles and so runs ahead of the ask; deciding must not. A
    /// reader that reached such a quote would report itself unsettled, and
    /// every route over that input would refuse - for a quote no caller asked
    /// about.
    #[test]
    fn a_quote_past_the_ask_is_not_reached_however_far_the_search_ran() {
        let mut src = String::new();
        for i in 0..4000 {
            src.push_str(&format!("let value_{i} = {i} ;\n"));
        }
        let tail = src.len();
        // A single quote with `://` before it in its run, which is the one
        // shape the rule declines: it may be a URL's byte or a token's start.
        src.push_str("x = 'http://unsettled' ;\n");
        let input = src.as_bytes();
        let mut scan = QuoteScan::new(input);
        let mut rule = |q: usize| (q < tail).then_some(true);
        for upto in [0usize, 64, 1000, tail / 2, tail - 1] {
            scan.ensure(upto, &mut rule);
            assert!(!scan.unsettled(), "an ask about {upto} reached the quote at {tail}");
        }
        scan.ensure(input.len(), &mut rule);
        assert!(scan.unsettled(), "the quote is reached once it is asked about");
    }

    /// Every position outside a double-quoted string that directly follows a
    /// newline, and every one that directly follows a whitespace run where the
    /// whitespace rule holds, by a byte-at-a-time walk with the lexer's rule -
    /// a string closes at the first unescaped quote on its line, and a quote
    /// whose line holds none opens nothing: what a boundary must be. A quote
    /// that opens a string is no boundary, and one that opens nothing may be.
    fn candidates_by_walk(input: &[u8]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < input.len() {
            let b = input[i];
            if let Some(end) = crate::lexer::double_quoted_end(input, i) {
                i = end;
                continue;
            }
            if i > 0 && input[i - 1].is_ascii_whitespace() && !b.is_ascii_whitespace() {
                let mut w = i;
                while w > 0 && input[w - 1].is_ascii_whitespace() {
                    w -= 1;
                }
                let after_newline = input[i - 1] == b'\n';
                let by_whitespace = w > 0
                    && !matches!(input[w - 1], b'\'' | b'\\')
                    && !b.is_ascii_digit()
                    && !matches!(b, b'-' | b'+');
                if after_newline || by_whitespace {
                    out.push(i);
                }
            }
            i += 1;
        }
        out
    }

    #[test]
    fn single_quoted_strings_lex_the_same_chunked_and_whole() {
        // Strings with spaces inside them, quotes of the other kind inside
        // each, char literals, lifetimes, contractions and a string that a
        // newline leaves open: every chunking must read them as the whole lex
        // does, and the spans must be the ones the lexer makes tokens of.
        let corpus = concat!(
            "let a = 'one two' ;\n",
            "let b = 'has \"double\" inside' ; x\n",
            "let c = \"has 'single' inside\" ;\n",
            "fn f<'a>(x: &'a str) -> &'a str { x }\n",
            "don't stop, it's the '90s ;\n",
            "let d = 'never closed\n",
            "rock 'n' roll and '\\'' ;\n",
            "y = '\\u{10000}\\u{10001}' 'z' ;\n",
            "plain line at the end\n",
        );
        let input = corpus.as_bytes();
        assert_identical(input);
        let spans = quoted_spans(input);
        let quoted: Vec<(usize, usize)> = lex(input)
            .iter()
            .filter(|t| t.kind == crate::token::TokenKind::Quoted && t.end() - t.start() > 4)
            .map(|t| (t.start(), t.end() - 1))
            .collect();
        for span in &quoted {
            assert!(spans.contains(span), "the lexer's string {span:?} is not among the reader's {spans:?}");
        }
    }

    #[test]
    fn boundaries_fall_only_where_the_walk_allows() {
        // Quotes whose lines hold no close, strings carried over lines by a
        // backslash before an LF and before a CRLF, escaped quotes behind odd
        // and even backslash runs, a quote right after a newline, and lines of
        // plain text between: every boundary chosen at every chunk count must
        // be a position the walk allows, and the stitched lex must be the
        // serial one.
        let corpus = concat!(
            "let a = \"one\\\"two\" ;\n",
            "let b = \"spans\nlines\nhere\" ; x\n",
            "let c = \"ends with backslashes\\\\\" ;\n",
            "let d = \"odd\\\\\\\" still open\n",
            "not closed yet ;\n",
            "\" closed now\n",
            "\"opens the line\" ok\n",
            "let e = \"carried \\\nover\" ;\n",
            "let f = \"carried \\\r\nover crlf\" ;\r\n",
            "plain line one\n",
            "plain line two\n",
            "y = 'q' \"tail without close\n",
            "last line\n",
        );
        let input = corpus.as_bytes();
        let allowed = candidates_by_walk(input);
        for chunks in [2usize, 3, 5, 8, 13, 40] {
            let bounds = safe_boundaries(input, chunks);
            assert_eq!(bounds[0], 0);
            assert_eq!(*bounds.last().expect("a last bound"), input.len());
            for &b in &bounds[1..bounds.len() - 1] {
                assert!(allowed.contains(&b), "boundary {b} at {chunks} chunks is not after a newline outside a string");
            }
            let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
            let blobs = crate::lexer::blob_runs(input);
            let parts: Vec<(Vec<Token>, Seams)> = ranges
                .iter()
                .map(|&(s, e)| lex_chunk(&input[s..e], &chunk_blobs(&blobs, s, e), 16))
                .collect();
            assert_eq!(stitch_parallel(&parts, &ranges), lex(input), "{chunks} chunks");
        }
        let one_string = b"\"a\\\\\" b\nc\n";
        assert_eq!(quoted_spans(one_string), vec![(0, 4)]);
        // An escaped quote leaves the string waiting on a close its line does
        // not hold, so it is no string; a backslash before the newline carries
        // one over to its close.
        assert_eq!(quoted_spans(b"\"\\\"\nx"), Vec::new());
        assert_eq!(quoted_spans(b"\"a\\\nb\" c"), vec![(0, 5)]);
        assert_eq!(quoted_spans(b"no quotes\n"), Vec::new());
    }

    /// A string the resumable scan reads past the bytes it has searched holds
    /// quotes it must not read again as openers when a later ask searches
    /// further: its spans stay the eager scan's.
    #[test]
    fn a_string_read_past_the_search_hides_the_quotes_inside_it() {
        let mut src = String::new();
        for i in 0..200 {
            src.push_str(&format!("let value_{i} = {i} ;\n"));
        }
        let open = src.len() + 4;
        // The string opens at the first ask's position and closes past what
        // that ask searched, holding a single-quoted string and a char literal.
        src.push_str("x = \"a 'held inside' and 'c' still in the string\" ;\n");
        for i in 0..200 {
            src.push_str(&format!("let later_{i} = 'z' ;\n"));
        }
        let input = src.as_bytes();
        assert_eq!(input[open], b'"');
        let eager = quoted_spans(input);
        let mut scan = QuoteScan::new(input);
        let mut always = |_: usize| Some(true);
        scan.ensure(open, &mut always);
        assert!(scan.searched_to() > open, "the string was read past the first search");
        scan.ensure(input.len(), &mut always);
        assert_eq!(scan.spans(), &eager[..], "a quote inside the string was read as an opener");
    }

    #[test]
    fn unicode_text_stitches_byte_identical() {
        // Chunk boundaries are newlines (ASCII), which never fall inside a
        // multi-byte sequence, so unicode-heavy text must stitch exactly.
        let line = "caf\u{E9} Gr\u{FC}\u{DF}e \u{03B8} na\u{EF}ve\u{2014}done \u{A0} ok\n";
        let big = line.repeat(64);
        assert_eq!(lex_chunked_at_every_boundary(big.as_bytes()), lex(big.as_bytes()));
    }

    /// Force parallelism regardless of input size, so the chunking and
    /// stitching are exercised even by small test inputs: split at every
    /// safe boundary and stitch, then compare to the serial lexer.
    fn lex_chunked_at_every_boundary(input: &[u8]) -> Vec<Token> {
        // Reuse the same boundary finder, but ask for a chunk per line
        // so even a few-line input splits, then stitch exactly as the
        // parallel path does.
        let bounds = safe_boundaries(input, input.len().max(1));
        if bounds.len() <= 2 {
            return lex(input);
        }
        let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
        // Mirror the production path: one blob table over the whole input,
        // sliced per chunk. Calling `lex` per chunk here would give each chunk
        // a truncated entropy window, which is the divergence this checks for.
        let blobs = crate::lexer::blob_runs(input);
        let parts: Vec<(Vec<Token>, Seams)> = ranges
            .iter()
            .map(|&(s, e)| lex_chunk(&input[s..e], &chunk_blobs(&blobs, s, e), 0))
            .collect();
        stitch_parallel(&parts, &ranges)
    }

    #[test]
    fn whitespace_boundaries_keep_every_recognizer_whole() {
        // No newline anywhere, so every boundary comes from the whitespace
        // rule: chunked at every boundary it allows, the stream must be the
        // serial lexer's, so no card, phone, coordinate pair, quoted
        // string or char literal holding a space is cut, and plain words,
        // numbers, paths, sizes and dates lex as before.
        let corpus = concat!(
            "card 4111 1111 1111 1111 paid 4111-1111-1111-1111 also 12345678901234567 sum ",
            "call +44 20 7123 4567 or +1 555-123-4567 or 212-555-1234 now ",
            "at 37.7749, -122.4194 and 55.7558, 37.6173 or 12.5,34.7 here ",
            "c = ' ' d = '\\ ' e = 'x' f = \"a string with spaces\" g = \"esc \\\" quote\" ",
            "size 10MB and 1.5GiB took 1500ms or 3h20m for $1,234.56 at 50% ",
            "mass 5 kg at 3.2 GHz and 40 % or -40\u{b0}C over 5 m/s with 5 items and 3 in a row ",
            "on 2024-01-02 at 12:30:45 or 2024-01-02T12:30:00 see /usr/bin and C:\\Users\\x ",
            "mail user@example.com http://example.com/a?b=1 v1.2.3 #fff 00:1a:2b:3c:4d:5e ",
            "hash d41d8cd98f00b204e9800998ecf8427e blob SGVsbG8gV29ybGQhIQ== end",
        );
        let input = corpus.as_bytes();
        assert!(!input.contains(&b'\n'));
        let bounds = safe_boundaries(input, input.len());
        assert!(bounds.len() > 20, "the whitespace rule should split a newline-free input: {bounds:?}");
        assert_eq!(lex_chunked_at_every_boundary(input), lex(input));
    }

    #[test]
    fn seam_brackets_pair_as_the_serial_lexer_pairs_them() {
        // A close over a mismatched open in its own chunk stays unpaired
        // even when an earlier chunk's open would match it; a close over an
        // empty chunk stack pairs with the nearest earlier open of its kind
        // or with nothing; opens nest across many chunks; and a chunk's own
        // pairs are never disturbed. One chunk per line.
        for input in [
            b"( a\n] b\n) c\n".as_slice(),
            b"[ x\n( y\n] z\n) w\n",
            b"a )\n( b\n) c\n",
            b"{\n[\n(\n)\n]\n}\n",
            b"( ]\n)\n",
            b") (\n) ]\n",
            b"[ ( x\n) y ]\n} z\n",
            b"( ( )\n) )\n( \n",
        ] {
            assert_eq!(
                lex_chunked_at_every_boundary(input),
                lex(input),
                "chunked pairing differs from serial on {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    /// Prose lines alternating with base64-shaped blobs: the input whose
    /// entropy history differs between a whole-input and a per-chunk lex.
    /// The device path's fused lex must equal the significant part of the
    /// serial lexer's stream, kind for kind and span for span: past the
    /// parallel threshold, so the chunked dispatch runs, on prose with blobs
    /// and on brackets across lines, and below it on a line of recognizers.
    #[test]
    fn significant_lex_equals_the_serial_streams_significant_part() {
        let grow = |make: &dyn Fn(usize) -> Vec<u8>| {
            let mut units = 4000;
            loop {
                let candidate = make(units);
                if candidate.len() > PARALLEL_LEX_THRESHOLD {
                    break candidate;
                }
                units *= 2;
            }
        };
        let prose = grow(&prose_and_blobs);
        let brackets = grow(&|n| {
            let mut v = Vec::new();
            for i in 0..n {
                v.extend_from_slice(format!("a (b [c {i}] d) e\n(f\ng) {{h}}\n").as_bytes());
            }
            v
        });
        let small = b"call +1 555-123-4567 now 4111 1111 1111 1111 pin 37.7749,-122.4194 x".to_vec();
        for (name, input) in [("prose and blobs", &prose), ("brackets across lines", &brackets), ("small", &small)]
        {
            let want = crate::gpu::significant_stream(&lex(input));
            let got = lex_significant_parallel(input);
            assert!(!want.0.is_empty(), "{name}: nothing to compare");
            assert_eq!(got.0, want.0, "{name}: kind codes differ from the serial lexer's");
            assert_eq!(got.1, want.1, "{name}: spans differ from the serial lexer's");
        }
    }

    /// The leaves take the ranges longest first, ranges of one length in
    /// their order, and every part holds what its own range lexes to
    /// whatever order the leaves ran in.
    #[test]
    fn the_leaves_take_the_longest_range_first_and_write_each_part_once() {
        assert_eq!(longest_first(&[(0, 10), (10, 50), (50, 55), (55, 95)]), vec![1, 3, 0, 2]);
        assert_eq!(longest_first(&[(0, 4), (4, 8), (8, 12)]), vec![0, 1, 2]);
        assert_eq!(longest_first(&[]), Vec::<usize>::new());
        let input: &[u8] = include_bytes!("parallel_lex.rs");
        let n = input.len();
        let cuts = [0, n / 20, n / 20 + 2000, n / 3, n / 3 + 100, n / 2, 3 * n / 4, n];
        let ranges: Vec<(usize, usize)> = cuts.windows(2).map(|w| (w[0], w[1])).collect();
        let blobs = crate::lexer::blob_runs(input);
        let mut parts = Vec::new();
        let got = lex_significant_leaves(input, &ranges, &blobs, &mut parts);
        assert_eq!(got.len(), ranges.len());
        for (k, &(s, e)) in ranges.iter().enumerate() {
            let mut want = Significant::with_base(0, 0);
            want.reset(s);
            lex_chunk_significant_into(&input[s..e], &chunk_blobs(&blobs, s, e), &mut want);
            assert!(!want.kinds.is_empty(), "range {k} holds tokens");
            assert_eq!(got[k].kinds, want.kinds, "range {k}");
            assert_eq!(got[k].spans, want.spans, "range {k}");
        }
    }

    /// A chunk's fused lex at every boundary the walk allows, joined, equals
    /// the serial stream's significant part, so a boundary inside a
    /// newline-free stretch keeps every recognizer whole on this path too.
    #[test]
    fn significant_chunks_at_every_boundary_join_to_the_serial_stream() {
        use crate::lexer::lex_chunk_significant;
        let input: &[u8] = b"call +1 555-123-4567 now 4111 1111 1111 1111 pin 37.7749,-122.4194 x 'a b' \"q r\" (p [q] r) 10.0.0.1 v1.2.3 end";
        let bounds = safe_boundaries(input, input.len());
        assert!(bounds.len() > 2, "the walk must split this input: {bounds:?}");
        let blobs = crate::lexer::blob_runs(input);
        let (mut kinds, mut spans) = (Vec::new(), Vec::new());
        for w in bounds.windows(2) {
            let (s, e) = (w[0], w[1]);
            let part = lex_chunk_significant(&input[s..e], &chunk_blobs(&blobs, s, e), s, 0);
            kinds.extend_from_slice(&part.kinds);
            spans.extend_from_slice(&part.spans);
        }
        assert_eq!((kinds, spans), crate::gpu::significant_stream(&lex(input)));
    }

    /// The fused lex of each range between the input's safe boundaries,
    /// against the whole input's blob table, joins to the whole input's fused
    /// lex: on prose with blobs, whose entropy history a range-local table
    /// would cut, at two to sixty-four partitions, so ranges above the parallel
    /// threshold and ranges below it both run.
    #[test]
    fn significant_ranges_between_safe_boundaries_join_to_the_whole_lex() {
        let mut units = 4000;
        let prose = loop {
            let candidate = prose_and_blobs(units);
            if candidate.len() > 8 * PARALLEL_LEX_THRESHOLD {
                break candidate;
            }
            units *= 2;
        };
        let whole = lex_significant_parallel(&prose);
        let blobs = crate::lexer::blob_runs_parallel(&prose);
        let mut ws = SignificantWorkspace::default();
        for partitions in [2, 3, 8, 64] {
            let bounds = safe_boundaries(&prose, partitions);
            assert!(bounds.len() > 2, "{partitions} partitions: the walk must split the input");
            let (mut kinds, mut spans) = (Vec::new(), Vec::new());
            for w in bounds.windows(2) {
                lex_significant_range_into(&prose, w[0], w[1], &blobs, &mut ws);
                kinds.extend_from_slice(&ws.kinds);
                spans.extend_from_slice(&ws.spans);
            }
            assert_eq!(kinds, whole.0, "{partitions} partitions: kind codes differ from the whole lex");
            assert_eq!(spans, whole.1, "{partitions} partitions: spans differ from the whole lex");
        }
    }

    /// A held token workspace lexes a second, smaller input to the serial
    /// lexer's stream and the first again after it, so nothing a call leaves
    /// in the buffers reaches the next; the thread's own workspace answers
    /// the same.
    #[test]
    fn held_token_buffers_carry_nothing_between_inputs() {
        let mut units = 4000;
        let big = loop {
            let candidate = prose_and_blobs(units);
            if candidate.len() > PARALLEL_LEX_THRESHOLD {
                break candidate;
            }
            units *= 2;
        };
        let small = b"a (b [c] d) e\n".to_vec();
        let mut ws = TokenWorkspace::default();
        lex_parallel_into(&big, &mut ws);
        assert_eq!(ws.toks, lex(&big));
        lex_parallel_into(&small, &mut ws);
        assert_eq!(ws.toks, lex(&small));
        lex_parallel_into(&big, &mut ws);
        assert_eq!(ws.toks, lex(&big));
        assert_eq!(lex_parallel_held(&small, <[Token]>::to_vec), lex(&small));
        assert_eq!(lex_parallel_held(&big, <[Token]>::len), lex(&big).len());
    }

    /// The same for the device path's workspace: its kinds and spans after
    /// a smaller input, and after the larger one again, are the serial
    /// lexer's significant part.
    #[test]
    fn held_significant_buffers_carry_nothing_between_inputs() {
        let mut units = 4000;
        let big = loop {
            let candidate = prose_and_blobs(units);
            if candidate.len() > PARALLEL_LEX_THRESHOLD {
                break candidate;
            }
            units *= 2;
        };
        let small = b"a (b [c] d) e\n".to_vec();
        let mut ws = SignificantWorkspace::default();
        for input in [&big, &small, &big] {
            lex_significant_parallel_into(input, &mut ws);
            let want = crate::gpu::significant_stream(&lex(input));
            assert_eq!((ws.kinds.clone(), ws.spans.clone()), want);
        }
    }

    /// A held lex nested in a held lex's closure lexes into buffers of its
    /// own: both streams are the serial lexer's, and nothing panics.
    #[test]
    fn a_held_lex_inside_a_held_lex_gets_buffers_of_its_own() {
        let outer = b"a (b) c\n".repeat(20_000);
        let inner = b"x [y] z\n".repeat(10_000);
        assert!(
            outer.len() > PARALLEL_LEX_THRESHOLD && inner.len() > PARALLEL_LEX_THRESHOLD,
            "both lexes must take the pool route, where the buffers are held"
        );
        let (outer_toks, inner_toks) =
            lex_parallel_held(&outer, |o| (o.to_vec(), lex_parallel_held(&inner, <[Token]>::to_vec)));
        assert_eq!(outer_toks, lex(&outer));
        assert_eq!(inner_toks, lex(&inner));
        let (outer_sig, inner_sig) = lex_significant_parallel_held(&outer, |ok, os| {
            let inner_sig = lex_significant_parallel_held(&inner, |ik, is| (ik.to_vec(), is.to_vec()));
            ((ok.to_vec(), os.to_vec()), inner_sig)
        });
        assert_eq!(outer_sig, crate::gpu::significant_stream(&lex(&outer)));
        assert_eq!(inner_sig, crate::gpu::significant_stream(&lex(&inner)));
    }

    fn prose_and_blobs(lines: usize) -> Vec<u8> {
        const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let mut x = 0x1234_5678u64;
        for i in 0..lines {
            out.extend_from_slice(
                format!("the quick brown fox jumps over the lazy dog line {i}\n").as_bytes(),
            );
            for _ in 0..64 {
                x = x
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                out.push(ALPHA[((x >> 58) % 64) as usize]);
            }
            out.push(b'\n');
        }
        out
    }

    #[test]
    fn blob_bearing_input_stitches_byte_identical() {
        assert_identical(&prose_and_blobs(200));
    }

    #[test]
    fn lex_parallel_matches_serial_over_the_threshold() {
        // Past PARALLEL_LEX_THRESHOLD the real dispatch runs, so this covers
        // the production path rather than the test harness's own chunking.
        //
        // The corpus grows until it clears the threshold, so the size is
        // derived from it rather than chosen to match it. A fixed size is a
        // test of the serial path for any threshold above it, and it passes
        // either way, so nothing would say it had stopped covering this one.
        let mut units = 4000;
        let input = loop {
            let candidate = prose_and_blobs(units);
            if candidate.len() > PARALLEL_LEX_THRESHOLD {
                break candidate;
            }
            units *= 2;
        };
        assert_eq!(lex_parallel(&input), lex(&input));
    }

    fn assert_identical(input: &[u8]) {
        assert_eq!(
            lex_chunked_at_every_boundary(input),
            lex(input),
            "chunked lex differs from serial on {:?}",
            String::from_utf8_lossy(input)
        );
    }

    #[test]
    fn matches_serial_on_line_oriented_input() {
        assert_identical(b"alpha beta\n12 34\nword (group)\n");
    }

    /// A newline the lexer reads as the char of a char literal is one token
    /// with the quotes around it, so no cut falls after it. The first input is
    /// the bytes of `wiki_16mb_tail.txt` where the pool's lexer and the serial
    /// one parted: wiki bold markup ending one line and opening the next.
    #[test]
    fn a_newline_held_in_a_char_literal_is_never_a_cut() {
        assert_identical(b"see [[Hummingbird]].''\n'''Hummer''' is a [[marque]] of vehicles\n");
        assert_identical(b"let c = '\\\n' ;\nnext line\n");
        assert_identical(b"a '\n' b\n'\n'\nc\n");
    }

    #[test]
    fn reconciles_brackets_across_a_chunk_seam() {
        // The open paren is on the first line, the close on the third,
        // so the pair spans two interior boundaries; the global repair
        // must still find it.
        let input = b"open (\nmiddle line here\n) close\ntail\n";
        assert_identical(input);
        // Spot-check the mate is actually set across the seam.
        let toks = lex_chunked_at_every_boundary(input);
        let open = toks.iter().position(|t| matches!(t.kind, TokenKind::Open(_))).unwrap();
        let close = toks.iter().position(|t| matches!(t.kind, TokenKind::Close(_))).unwrap();
        assert_eq!(toks[open].mate(), Some(close));
        assert_eq!(toks[close].mate(), Some(open));
    }

    #[test]
    fn parallel_stitch_matches_serial() {
        // The parallel-prefix stitch must reproduce the serial lexer
        // byte-for-byte, including mates, on nested and seam-crossing
        // brackets and on bracket-free text.
        for input in [
            b"alpha beta\n12 34\nword (group)\n".as_slice(),
            b"open (\nmiddle line here\n) close\ntail\n",
            b"a (b (c) d) e\nf (g) h\n(i\nj)\nk\n",
            b"no brackets here\njust words and 123 numbers\n",
            b"[x]\n{y}\n(z)\nmix [a (b) c]\n",
        ] {
            let bounds = safe_boundaries(input, input.len().max(1));
            if bounds.len() <= 2 {
                continue;
            }
            let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
            let parts: Vec<(Vec<Token>, Seams)> =
                ranges.iter().map(|&(s, e)| lex_chunk(&input[s..e], &[], 0)).collect();
            assert_eq!(
                stitch_parallel(&parts, &ranges),
                lex(input),
                "parallel stitch differs from serial lex on {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn never_splits_inside_a_quoted_string_with_newlines() {
        // A quoted string contains newlines; a naive newline split would
        // break it. The boundary finder must skip newlines inside quotes.
        let input = b"before\n\"a quoted\nstring with\nnewlines\"\nafter\n";
        assert_identical(input);
    }

    #[test]
    fn handles_blank_lines_and_leading_whitespace() {
        let input = b"a\n\n\n   b\nc\n";
        assert_identical(input);
    }

    #[test]
    fn handles_no_trailing_newline() {
        assert_identical(b"x\ny\nz");
    }

    #[test]
    fn handles_typed_tokens_per_line() {
        let input =
            b"2026-06-16 192.168.0.1 a@b.com\nhttp://x.com/p 12:30:45\nfe80::1 plain word\n";
        assert_identical(input);
    }

    #[test]
    fn empty_and_tiny_inputs() {
        assert_identical(b"");
        assert_identical(b"a");
        assert_identical(b"\n");
    }

    /// The paired parts pair a bracket exactly where the stitched token stream
    /// does, over an input long enough to be split across chunks.
    ///
    /// The outer brackets open in the first chunk and close in the last, so
    /// they are paired by the seam walk and by nothing else; the inner ones
    /// close inside the chunk that opened them. Both readings have to agree, and
    /// the mates the walk writes are in the whole stream's indices while the
    /// ones a chunk wrote began in its own, so this is also what holds the two
    /// index spaces to each other.
    #[test]
    fn the_paired_parts_pair_what_the_stitched_tokens_pair() {
        let mut src = String::from("( {\n");
        for i in 0..40_000 {
            src.push_str(&format!("f_{i} [ a_{i} ] b_{i} ( c_{i} )\n"));
        }
        src.push_str("} )\n");
        let input = src.as_bytes();

        let toks = lex_parallel(input);
        // Where each token sits in the significant stream, and which token each
        // significant slot came from.
        let mut sig_of = vec![usize::MAX; toks.len()];
        let mut tok_of: Vec<usize> = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            if t.kind != TokenKind::Whitespace {
                sig_of[i] = tok_of.len();
                tok_of.push(i);
            }
        }

        lex_paired_parts_held(input, |parts| {
            assert!(parts.len() > 1, "the input is long enough to be split");
            let mut mates: Vec<u32> = Vec::new();
            for (part, _) in parts {
                mates.extend_from_slice(&part.mates);
            }
            assert_eq!(
                mates.len(),
                tok_of.len(),
                "one mate slot per significant token"
            );
            let mut crossed = 0usize;
            for (s, &t) in tok_of.iter().enumerate() {
                let want = toks[t].mate().map(|m| sig_of[m]);
                let got = (mates[s] != crate::lexer::NO_MATE).then(|| mates[s] as usize);
                assert_eq!(got, want, "significant {s} (token {t})");
                if let Some(m) = want
                    && m.abs_diff(s) > 4
                {
                    crossed += 1;
                }
            }
            assert!(crossed >= 4, "the outer brackets pair across chunks");
        });
    }
}
