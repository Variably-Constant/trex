//! The stress axis - trex's structural-load substrate.
//!
//! Material science reads a body by the internal FORCES it holds under load:
//! stress (force concentration), strain (deformation), fracture (sudden
//! release). A token stream holds an analogous load - the UNRESOLVED CONTEXT at
//! each point: how deeply nested it is, how long its open structures have been
//! held, how much total tension is outstanding. The stress axis reads it.
//!
//! Distinct from every other axis: `magnitude` reads a value's SCALE, `shape`
//! reads the structural FORM, `spectral` the temporal texture, `orbit` the
//! symmetry. Stress reads the structural load - the depth of nesting, the strain
//! of held-open spans, and the fracture points where a deep structure suddenly
//! closes. A flat stream carries zero stress; a deeply nested one carries a lot,
//! and the deepest / most-strained points are where the structure is most
//! loaded.
//!
//! Built on the lexer's bracket pairing (`TokenKind::Open` / `Close`): the open
//! stack at each token IS the load it holds. Documented in
//! `wiki/content/docs/reference/axes/stress.md`.

use crate::token::{Token, TokenKind};

/// Knobs for the stress reader.
#[derive(Clone, Copy, Debug)]
pub struct StressConfig {
    /// A stress peak (local depth maximum) must reach at least this depth.
    pub peak_min_depth: u16,
    /// A fracture releases from a level at least this deep (the cascade start).
    pub fracture_min_depth: u16,
}

impl Default for StressConfig {
    fn default() -> Self {
        Self {
            peak_min_depth: 2,
            fracture_min_depth: 2,
        }
    }
}

/// One token's reading on the stress axis.
#[derive(Clone, Copy, Debug, Default)]
pub struct StressFrame {
    /// Nesting depth: the number of brackets enclosing this token.
    pub depth: u16,
    /// Strain: how many tokens the innermost open bracket has been held open
    /// (its stretch at this point).
    pub strain: f32,
    /// Load: the total stretch of every currently-open bracket - the
    /// outstanding structural tension.
    pub load: f32,
}

/// The structural-load side table, keyed by byte offset.
#[derive(Clone, Debug, Default)]
pub struct StressField {
    /// Token count.
    pub n_tokens: usize,
    /// Byte span per token - the byte-offset key.
    pub spans: Vec<(usize, usize)>,
    /// One frame per token.
    pub frames: Vec<StressFrame>,
    /// Stress-peak byte offsets: local depth maxima at or above
    /// `peak_min_depth` (the most loaded points).
    pub peaks: Vec<usize>,
    /// Fracture byte offsets: where the depth starts to fall from a level at
    /// least `fracture_min_depth` deep - a deep structure beginning to close.
    pub fractures: Vec<usize>,
    /// The deepest nesting reached anywhere in the stream.
    pub max_depth: u16,
}

impl StressField {
    /// The token index covering `byte`, or `None` if the field is empty.
    fn token_at(&self, byte: usize) -> Option<usize> {
        if self.spans.is_empty() {
            return None;
        }
        let i = self.spans.partition_point(|&(s, _)| s <= byte);
        Some(i.saturating_sub(1))
    }

    /// The nesting depth at `byte`.
    #[must_use]
    pub fn depth_at(&self, byte: usize) -> u16 {
        self.token_at(byte)
            .and_then(|i| self.frames.get(i))
            .map_or(0, |f| f.depth)
    }


    /// The outstanding structural load at `byte`.
    #[must_use]
    pub fn load_at(&self, byte: usize) -> f32 {
        self.token_at(byte)
            .and_then(|i| self.frames.get(i))
            .map_or(0.0, |f| f.load)
    }
}

/// The nesting depth of every token, in token order.
///
/// This is the one reading the structural anchors take. It carries neither the
/// byte-offset side table that keys [`StressField`] nor the strain, load, peak
/// and fracture readings, so a caller that holds a token index and wants only
/// its depth reads a `u16` per token instead of a frame plus a span per token.
#[must_use]
pub fn depths(tokens: &[Token]) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::with_capacity(tokens.len());
    // The count of open brackets IS the depth, so the stack of indices that
    // `analyze_with` keeps for strain and load reduces to a counter here. A
    // close resolves its own level first, and only a bracket the lexer paired
    // carries structure.
    let mut open: usize = 0;
    for t in tokens {
        if matches!(t.kind, TokenKind::Close(_)) && t.mate().is_some() && open > 0 {
            open -= 1;
        }
        out.push(open as u16);
        if matches!(t.kind, TokenKind::Open(_)) && t.mate().is_some() {
            open += 1;
        }
    }
    out
}

/// Analyze a token stream with the default configuration.
#[must_use]
pub fn analyze(tokens: &[Token], bytes: &[u8]) -> StressField {
    analyze_with(tokens, bytes, &StressConfig::default())
}

/// Tokenize `bytes` and analyze - the convenience path for byte-only consumers.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> StressField {
    let toks = crate::tokutil::lex_sig(bytes);
    analyze(&toks, bytes)
}

/// The full one-pass reader: depth, strain, and load from the bracket stack,
/// plus stress peaks and fracture points.
#[must_use]
pub fn analyze_with(tokens: &[Token], _bytes: &[u8], cfg: &StressConfig) -> StressField {
    let n = tokens.len();
    let mut field = StressField {
        n_tokens: n,
        spans: Vec::with_capacity(n),
        frames: Vec::with_capacity(n),
        peaks: Vec::new(),
        fractures: Vec::new(),
        max_depth: 0,
    };
    if n == 0 {
        return field;
    }
    for t in tokens {
        field.spans.push((t.start(), t.end()));
    }

    // The open-bracket stack (token indices) IS the load held at each point,
    // and `stack_sum` is the sum of those indices, so the load is an identity
    // rather than a walk: `sum over open of (i - open) == depth * i -
    // stack_sum`. Every index on the stack is below `i`, so the difference is
    // non-negative and the u64 never underflows.
    let mut stack: Vec<usize> = Vec::new();
    let mut stack_sum: u64 = 0;
    let mut frames: Vec<StressFrame> = Vec::with_capacity(n);
    let mut prev_depth: u16 = 0;
    let mut was_decreasing = false;
    for (i, t) in tokens.iter().enumerate() {
        // A close bracket resolves its own level first, so it reads the outer
        // depth (and is the release point).
        // Only a bracket the lexer paired carries structure. An unpaired one is
        // a character in the text, and treating it as an open would hold every
        // later token under a level that never resolves.
        if matches!(t.kind, TokenKind::Close(_))
            && t.mate().is_some()
            && let Some(open) = stack.pop()
        {
            stack_sum -= open as u64;
        }
        let depth = stack.len() as u16;
        let strain = stack.last().map_or(0.0, |&open| (i - open) as f32);
        // Integer throughout: an f32 running sum is exact only below 2^24, and
        // a deep stream reaches loads two orders past that. The count comes
        // from the stack and not from `depth`, which is a `u16` and wraps at
        // 65536 where the stack and the load carry on.
        let load = (stack.len() as u64 * i as u64 - stack_sum) as f32;
        field.max_depth = field.max_depth.max(depth);

        // fracture: the START of a release cascade from a deep level - a depth
        // decrease whose prior depth was deep, not one already mid-cascade.
        let decreasing = depth < prev_depth;
        if decreasing && !was_decreasing && prev_depth >= cfg.fracture_min_depth {
            field.fractures.push(t.start());
        }
        was_decreasing = decreasing;
        prev_depth = depth;

        frames.push(StressFrame {
            depth,
            strain,
            load,
        });
        if matches!(t.kind, TokenKind::Open(_)) && t.mate().is_some() {
            stack.push(i);
            stack_sum += i as u64;
        }
    }

    // peaks: local depth maxima at or above the threshold.
    for i in 0..n {
        let d = frames[i].depth;
        if d < cfg.peak_min_depth {
            continue;
        }
        let left = i.checked_sub(1).map_or(0, |j| frames[j].depth);
        let right = frames.get(i + 1).map_or(0, |f| f.depth);
        if d >= left && d >= right && (d > left || d > right) {
            field.peaks.push(field.spans[i].0);
        }
    }

    field.frames = frames;
    field
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(s: &str) -> StressField {
        analyze_bytes(s.as_bytes())
    }

    /// The load at each token, summed over the open-bracket stack the slow way.
    fn summed_stack(toks: &[Token]) -> Vec<u64> {
        let mut stack: Vec<usize> = Vec::new();
        let mut out = Vec::with_capacity(toks.len());
        for (i, t) in toks.iter().enumerate() {
            if matches!(t.kind, TokenKind::Close(_)) && t.mate().is_some() {
                stack.pop();
            }
            out.push(stack.iter().map(|&open| (i - open) as u64).sum());
            if matches!(t.kind, TokenKind::Open(_)) && t.mate().is_some() {
                stack.push(i);
            }
        }
        out
    }

    #[test]
    fn the_load_identity_equals_the_summed_stack() {
        let src = b"a(b(c(d(e f) g) h) i) j (k (l m)) n";
        let toks = crate::tokutil::lex_sig(src);
        let f = analyze(&toks, src);
        for (i, want) in summed_stack(&toks).iter().enumerate() {
            assert_eq!(f.frames[i].load.to_bits(), (*want as f32).to_bits(), "token {i}");
        }
    }

    #[test]
    fn a_deep_stream_loads_past_what_an_f32_sum_holds() {
        // Six thousand nested pairs put the load an order past 2^24, where an
        // f32 running sum stops representing integers exactly. The identity is
        // integer, so the reading is the exact sum rounded once.
        let mut src = Vec::new();
        for _ in 0..6000 {
            src.extend_from_slice(b"( a ");
        }
        src.extend_from_slice(&b")".repeat(6000));
        let toks = crate::tokutil::lex_sig(&src);
        let f = analyze(&toks, &src);
        let want = summed_stack(&toks);
        let peak = want.iter().copied().max().expect("the fixture has tokens");
        assert!(peak > (1 << 24), "the fixture must reach past 2^24, got {peak}");
        for (i, w) in want.iter().enumerate() {
            assert_eq!(f.frames[i].load.to_bits(), (*w as f32).to_bits(), "token {i}");
        }
    }

    #[test]
    fn the_load_survives_a_stack_deeper_than_a_u16() {
        // `depth` is a u16 and wraps past 65535; the stack and the load do not.
        // A run of `n` openers puts `i` of them on the stack at token `i`, so
        // the load there is the sum of `i - k` over `k` below `i`, and the last
        // token's is `(n-1)n/2`. Real input reaches this: a JSONTestSuite
        // fixture opens 100000 arrays in 250 kB.
        const N: usize = 70_000;
        // Paired throughout, since an unpaired bracket carries no structure and
        // never reaches the stack.
        let mut src = b"[".repeat(N);
        src.extend_from_slice(&b"]".repeat(N));
        let toks = crate::tokutil::lex_sig(&src);
        assert_eq!(toks.len(), 2 * N, "one token per bracket");
        let f = analyze(&toks, &src);
        let want = ((N - 1) as u64 * N as u64 / 2) as f32;
        assert_eq!(
            f.frames[N - 1].load.to_bits(),
            want.to_bits(),
            "the deepest load reads {} against {want}",
            f.frames[N - 1].load
        );
        assert!(f.frames.iter().all(|fr| fr.load >= 0.0), "no frame's load may wrap negative");
    }

    #[test]
    fn an_unpaired_bracket_holds_nothing_open() {
        // Free-text data carries brackets that never close. Pushing one holds
        // every later token at a level that never resolves, so the depth never
        // returns and the load climbs to the end of the stream.
        let f = field("alpha [BETA,GAMMA, delta epsilon zeta eta");
        assert_eq!(f.max_depth, 0, "an unpaired opener is text, not a level");
        assert!(f.frames.iter().all(|fr| fr.load == 0.0), "so it holds nothing open");
        assert!(f.frames.iter().all(|fr| fr.depth == 0), "and nothing is under it");

        // A pair beside one still carries what it holds.
        let g = field("alpha (beta gamma) delta [EPSILON, zeta");
        assert!(g.max_depth >= 1, "the pair still nests its contents");
    }

    #[test]
    fn nesting_builds_stress() {
        // Deep nesting carries load that climbs to a peak then fractures.
        let f = field("a(b(c(d(e))))");
        assert!(f.max_depth >= 4, "should reach depth 4, got {}", f.max_depth);
        assert!(!f.peaks.is_empty(), "the deepest point is a stress peak");
        assert!(!f.fractures.is_empty(), "the cascade of closes is a fracture");
    }

    #[test]
    fn flat_stream_carries_no_stress() {
        let f = field("a b c d e f g");
        assert_eq!(f.max_depth, 0);
        assert!(f.peaks.is_empty());
        assert!(f.fractures.is_empty());
        assert!(f.frames.iter().all(|fr| fr.load == 0.0));
    }

    #[test]
    fn long_open_span_has_high_strain() {
        // A bracket held open across many tokens strains more than a short one.
        let long = field("( a b c d e f g h i j )");
        let short = field("( a )");
        let long_max = long.frames.iter().map(|f| f.strain).fold(0.0f32, f32::max);
        let short_max = short.frames.iter().map(|f| f.strain).fold(0.0f32, f32::max);
        assert!(
            long_max > short_max,
            "the long-held bracket should strain more ({long_max}) than the short ({short_max})"
        );
    }

    #[test]
    fn depth_at_byte_tracks_nesting() {
        let f = field("a(b(c)d)e");
        // Inside the inner bracket `c` is deeper than the trailing `e`.
        let c_pos = 4; // 'c'
        let e_pos = 8; // 'e'
        assert!(f.depth_at(c_pos) > f.depth_at(e_pos));
    }

    #[test]
    fn empty_is_safe() {
        let f = field("");
        assert_eq!(f.n_tokens, 0);
        assert!(f.frames.is_empty());
        assert!(f.peaks.is_empty());
        assert_eq!(f.max_depth, 0);
        assert_eq!(f.depth_at(0), 0);
    }
}
