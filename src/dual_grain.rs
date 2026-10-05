//! Dual-grain producer-consumer pipeline.
//!
//! A token is a span of bytes, so lexing (the byte grain) and
//! structural matching (the token grain) are two stages over the same
//! input. This module runs them concurrently as a producer and a
//! consumer: a byte-grain thread lexes the input chunk by chunk and
//! streams each chunk's tokens ahead, while a token-grain thread matches
//! the structure of the tokens already produced. The byte grain
//! tokenizes ahead while the token grain consumes the emerging stream,
//! so the two overlap in time.
//!
//! The result is identical to a single-grain [`crate::scan`]. The
//! consumer commits a match only once no later byte can change it: it
//! resumes the leftmost scan from the last committed token
//! ([`crate::engine::scan_tokens_from`]) and commits up to the largest
//! chunk boundary no match straddles. A pattern whose dependence reaches
//! outside one match span (a content guard's forward window, a field's
//! comma count from the input start) commits once at the end; the byte
//! grain still lexes ahead of the token grain, so the two still overlap.
//!
//! ## Observing the overlap
//!
//! Each grain accumulates only its own compute time, excluding the time
//! it blocks on the channel. If the two compute totals sum to more than
//! the wall-clock span, the grains must have run at the same time for at
//! least the difference. [`GrainTiming::overlap`] reports that lower
//! bound.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::ast::Pattern;
use crate::engine::{Span, scan_tokens_from};
use crate::lexer::lex_with_blobs;
use crate::token::{Token, TokenKind};

/// Target chunk size for the byte grain. Small enough that the consumer
/// starts matching while most of the input is still being lexed.
const CHUNK_BYTES: usize = 32 * 1024;

/// Bound on the number of chunks: enough to pipeline, few enough that
/// the consumer's resume-scan stays linear overall.
const MAX_CHUNKS: usize = 64;

/// Per-grain timing for one dual-grain scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrainTiming {
    /// Compute time spent in the byte grain (lexing), excluding channel
    /// blocking.
    pub producer_busy: Duration,
    /// Compute time spent in the token grain (matching), excluding
    /// channel blocking.
    pub consumer_busy: Duration,
    /// Wall-clock span of the whole pipeline.
    pub wall: Duration,
    /// Number of chunks the input was split into.
    pub chunks: usize,
}

impl GrainTiming {
    /// A lower bound on the time the two grains ran concurrently: when
    /// their compute totals exceed the wall clock, the excess is time
    /// they were both busy at once.
    #[must_use]
    pub fn overlap(&self) -> Duration {
        (self.producer_busy + self.consumer_busy).saturating_sub(self.wall)
    }

    /// Whether the two grains observably overlapped in time.
    #[must_use]
    pub fn overlapped(&self) -> bool {
        self.overlap() > Duration::ZERO
    }
}

/// Scan `input` for `pattern` with the byte grain and token grain
/// pipelined across two threads. Returns the matches (identical to
/// [`crate::scan`]) and the timing that shows the two grains overlapped.
#[must_use]
pub fn scan_dual_grain(pattern: &Pattern, input: &[u8]) -> (Vec<Span>, GrainTiming) {
    let defers = pattern.depends_on_whole_input();

    let target = (input.len() / CHUNK_BYTES).clamp(2, MAX_CHUNKS);
    let bounds = crate::parallel_lex::safe_boundaries(input, target);
    let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();

    // Computed once over the whole input: the spectral blob gate reads a
    // trailing window, so a chunk-local call would classify a chunk's opening
    // bytes against a truncated history and diverge from a whole-input lex.
    let blobs = crate::lexer::blob_runs(input);

    let (tx, rx) = mpsc::channel::<ChunkMsg>();
    let wall_start = Instant::now();

    let (matches, producer_busy, consumer_busy) = std::thread::scope(|sc| {
        let ranges_ref = &ranges;
        let blobs_ref = &blobs;
        let producer = sc.spawn(move || {
            let mut busy = Duration::ZERO;
            for &(s, e) in ranges_ref {
                let t = Instant::now();
                // Byte grain: classify this chunk's bytes into tokens and
                // shift to absolute offsets. Mates are rebuilt by the
                // consumer as the stream is stitched, so they are cleared.
                let chunk_blobs = crate::parallel_lex::chunk_blobs(blobs_ref, s, e);
                let mut toks = lex_with_blobs(
                    &input[s..e],
                    &chunk_blobs,
                    (e - s) / crate::lexer::TOKEN_BYTES_ESTIMATE,
                );
                for tk in &mut toks {
                    tk.shift(s);
                    tk.set_mate(None);
                }
                busy += t.elapsed();
                if tx.send(ChunkMsg { byte_end: e, toks }).is_err() {
                    break;
                }
            }
            busy
        });

        let consumer = sc.spawn(move || consume(pattern, input, defers, &rx));

        let producer_busy = producer.join().expect("byte grain panicked");
        let (matches, consumer_busy) = consumer.join().expect("token grain panicked");
        (matches, producer_busy, consumer_busy)
    });

    let wall = wall_start.elapsed();
    (matches, GrainTiming { producer_busy, consumer_busy, wall, chunks: ranges.len() })
}

/// One chunk handed from the byte grain to the token grain.
struct ChunkMsg {
    /// Absolute byte offset just past this chunk: a safe boundary.
    byte_end: usize,
    /// The chunk's tokens, already at absolute offsets with mates clear.
    toks: Vec<Token>,
}

/// The token grain: receive chunks, stitch them into one stream, and
/// commit matches that can no longer change. Returns the matches and the
/// compute time spent (excluding the time blocked waiting for a chunk).
fn consume(
    pattern: &Pattern,
    input: &[u8],
    defers: bool,
    rx: &mpsc::Receiver<ChunkMsg>,
) -> (Vec<Span>, Duration) {
    let mut busy = Duration::ZERO;
    let mut acc: Vec<Token> = Vec::new();
    let mut open_stack: Vec<usize> = Vec::new();
    let mut committed_tok = 0usize;
    let mut committed_byte = 0usize;
    let mut out: Vec<Span> = Vec::new();
    // (byte boundary, token count up to it) for every chunk seen.
    let mut marks: Vec<(usize, usize)> = Vec::new();

    for msg in rx {
        let t = Instant::now();
        stitch_chunk(&mut acc, &mut open_stack, msg.toks);
        marks.push((msg.byte_end, acc.len()));

        if !defers {
            let found = resume(pattern, input, &acc, committed_tok);
            // Commit up to the largest chunk boundary that (a) no match
            // straddles and (b) has accumulated tokens after it. The
            // second condition excludes the most recent boundary: a
            // greedy match ending exactly at the current buffer end could
            // still be extended by a later chunk, so it is not final. A
            // boundary with tokens already past it is safe, because a
            // greedy match that stopped there did so with those tokens
            // available, and a later chunk is further still.
            let candidates = marks.len().saturating_sub(1);
            if let Some((cut_byte, cut_tok)) = marks
                .iter()
                .take(candidates)
                .rev()
                .find(|&&(b, _)| b > committed_byte && !found.iter().any(|m| m.start() < b && m.end() > b))
                .copied()
            {
                for m in &found {
                    if m.end() <= cut_byte {
                        out.push(*m);
                    }
                }
                committed_byte = cut_byte;
                committed_tok = cut_tok;
            }
        }
        busy += t.elapsed();
    }

    // Drain whatever was not committed.
    let t = Instant::now();
    out.extend(resume(pattern, input, &acc, committed_tok));
    busy += t.elapsed();
    (out, busy)
}

/// A leftmost scan resuming at token `from`, routed the way [`crate::scan`]
/// routes a whole input: the single-pass engine where it takes the pattern,
/// the set-reachability fold where it declines.
///
/// The routing is what makes this pipeline agree with [`crate::scan`]. The two
/// engines read an iteration that matched empty differently, so a pipeline
/// that always resumed on the fold would answer the regular subset with the
/// fold's reading while `scan` answered it with the single-pass engine's.
/// The serial variant is the right one here: the byte grain already has a
/// thread, so a per-anchor dispatch would contend with it.
fn resume(pattern: &Pattern, input: &[u8], toks: &[Token], from: usize) -> Vec<Span> {
    crate::nfa::scan_nfa_over_serial_from(pattern, input, toks, from)
        .unwrap_or_else(|| scan_tokens_from(pattern, input, toks, from))
}

/// Append a chunk's tokens to the accumulated stream, pairing brackets
/// with the running open-stack so a pair split across chunks is matched
/// exactly as the serial lexer would.
fn stitch_chunk(acc: &mut Vec<Token>, open_stack: &mut Vec<usize>, toks: Vec<Token>) {
    for mut tk in toks {
        let idx = acc.len();
        match tk.kind {
            TokenKind::Open(_) => open_stack.push(idx),
            TokenKind::Close(bk) => {
                if let Some(&open_idx) = open_stack.last()
                    && acc[open_idx].kind == TokenKind::Open(bk)
                {
                    open_stack.pop();
                    acc[open_idx].set_mate(Some(idx));
                    tk.set_mate(Some(open_idx));
                }
            }
            _ => {}
        }
        acc.push(tk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::scan;

    fn assert_dual_equals_single(pattern_src: &str, input: &str) {
        let pat = parse(pattern_src).expect("pattern parses");
        let single = scan(&pat, input.as_bytes());
        let (dual, _) = scan_dual_grain(&pat, input.as_bytes());
        assert_eq!(dual, single, "dual-grain differs from single-grain on {pattern_src:?}");
    }

    /// The two engines read an iteration that matched empty differently, so a
    /// pipeline that resumed on the set-reachability fold unconditionally
    /// answered the regular subset with the fold's reading while [`scan`]
    /// answered it with the single-pass engine's. These are the shapes that
    /// separate the readings: a nullable branch at higher priority than one
    /// that consumes, inside a repetition.
    #[test]
    fn dual_equals_single_where_the_two_engines_read_an_empty_iteration_apart() {
        const INPUT: &str = "bar 771 baz baz ";
        for pattern in [
            r"(\N? | \W)+ .",
            r"(\W? | \N)+ .",
            r"(\N? | \W?)+ .",
            r"(\W | \N?)+ .",
            r"(\N | \W)+ .",
            r"(\N?)+ .",
        ] {
            assert_dual_equals_single(pattern, INPUT);
        }
    }

    #[test]
    fn dual_equals_single_on_blob_bearing_input() {
        // The byte grain lexes chunk by chunk, so a per-chunk entropy window
        // would classify each chunk's opening bytes against a truncated
        // history. The pattern is ordinary: no axis, no guard, no balance.
        const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut input = Vec::new();
        let mut x = 0x1234_5678u64;
        for i in 0..4000 {
            input.extend_from_slice(
                format!("the quick brown fox jumps over the lazy dog line {i}\n").as_bytes(),
            );
            for _ in 0..64 {
                x = x
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                input.push(ALPHA[((x >> 58) % 64) as usize]);
            }
            input.push(b'\n');
        }
        let pat = parse("\\W \\W").expect("pattern parses");
        let single = scan(&pat, &input);
        let (dual, timing) = scan_dual_grain(&pat, &input);
        assert!(timing.chunks > 1, "the input must actually split");
        assert_eq!(dual, single, "dual-grain differs from single-grain on blob input");
    }

    #[test]
    fn dual_equals_single_over_mixed_patterns() {
        // Each case mixes a structural constraint with a sub-token byte
        // constraint (a byte pattern, a byte class, or a content guard).
        let cases: &[(&str, &str)] = &[
            ("<`[a-z]+`:t>.*</=t>", "<div>hi</div>\n<span>yo</span>\n<DIV>no</DIV>\n"),
            ("`\\d+`:n =n", "12 12 ok\n34 56 no\n77 77 yes\n"),
            ("`[A-Z]+` \\N", "ABC 12\nxy 9\nDEF 34\n"),
            ("\\W ~\"END\"", "begin END\nmiddle here\nlast END now\n"),
            ("\\d \\W", "12 kg\nab cd\n9 m\n"),
        ];
        for (pat, input) in cases {
            assert_dual_equals_single(pat, input);
        }
    }

    #[test]
    fn dual_equals_single_on_structural_only() {
        assert_dual_equals_single("\\N \\W", "weight 12 kg\nlen 5 m\nmass 9 g\n");
        assert_dual_equals_single("\\W\\B(.*)", "call f(g(x))\nrun h(k)\ntail\n");
    }

    #[test]
    fn greedy_match_spanning_a_chunk_seam_is_not_truncated() {
        // A single greedy `.*` match runs across the chunk boundary; the
        // consumer must not commit a truncated match at the seam (it did
        // before the most-recent-boundary exclusion landed).
        assert_dual_equals_single(".*", "a b c d e f\ng h i j k l\nm n o p q r\n");
        // A matched tag whose close is in a later chunk than its open,
        // with the greedy interior spanning the seam.
        assert_dual_equals_single("<\\W:t>.*</=t>", "<x>\naaa bbb ccc\nddd eee fff\n</x>\n");
    }

    #[test]
    fn grains_agree_on_a_larger_input() {
        // A larger input gives the pipeline enough chunks that the byte
        // grain and the token grain are both busy at once. Whether they
        // measurably overlapped in wall time is a fact about the host's
        // scheduling under whatever else is running, not about the
        // pipeline, so it is reported by the CLI's timing rather than
        // asserted here; what the pipeline owes is the same matches.
        let mut input = String::new();
        for i in 0..20_000 {
            input.push_str(&format!("row {i} val {} tag t{}\n", i * 7, i % 5));
        }
        let pat = parse("`[a-z]+` \\N").expect("pattern parses");
        let (dual, timing) = scan_dual_grain(&pat, input.as_bytes());
        let single = scan(&pat, input.as_bytes());
        assert_eq!(dual, single, "dual-grain must match single-grain");
        assert!(timing.chunks >= 2, "input should split into several chunks");
    }
}
