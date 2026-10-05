//! Seam: bidirectional predictive segmentation.
//!
//! Byte-pair encoding segments by merging the globally most frequent
//! adjacent pair into a trained dictionary - a compression method
//! repurposed as a tokenizer. Seam segments by the opposite principle: it
//! cuts where the stream's own statistics say the past stops predicting
//! the future. A token boundary is a local minimum of the mutual
//! information `I(past; future)` across a seam; inside a unit the parts
//! predict each other, at a boundary they decouple.
//!
//! The signal is **bidirectional branching entropy**, read from the input
//! itself (no trained dictionary, no learned model):
//!
//! - forward: for the k-byte context ending at `t`, the Shannon entropy of
//!   the byte that follows it. High = many possible continuations = a
//!   boundary just after `t` (Harris successor variety).
//! - backward: for the k-byte context starting at `t`, the entropy of the
//!   byte that precedes it. High = a boundary just before `t`
//!   (predecessor variety).
//!
//! A seam between `t-1` and `t` is strong when both sides branch:
//! `boundary[t] = fwd_entropy[t-1] + bwd_entropy[t]`, plus a bonus where
//! the coarse multi-scale `crate::spectral` change-point signal agrees.
//! Cuts are at local maxima above an adaptive threshold.
//!
//! The forward-and-backward pairing is what carries signal BPE cannot
//! reach: BPE's per-pair frequency is a global, symmetric, single-scale,
//! count-based shadow of the bidirectional, position-specific, multi-scale
//! predictive surprise this computes. Documented in
//! `wiki/content/docs/reference/axes/seam.md`.

use std::collections::HashMap;

// The compressor behind `trex compress`, compiled with the `compress` feature;
// its public items keep their `trex::seam::` paths through the re-export.
#[cfg(feature = "compress")]
mod coder;
#[cfg(feature = "compress")]
pub use coder::*;

/// Knobs for the segmenter. `Default` suits general byte input.
#[derive(Clone, Copy, Debug)]
pub struct SeamConfig {
    /// Maximum context order. The branching-entropy estimate backs off to a
    /// shorter context where the high-order one is too rare to be reliable -
    /// the n-gram-backoff trick that extracts signal from a short input.
    pub order: usize,
    /// A cut needs `boundary >= mean + cut_threshold * std` over the input.
    pub cut_threshold: f32,
    /// Minimum bytes between two cuts.
    pub min_seg: usize,
    /// Fold the `crate::spectral` coarse change-point signal in as a prior.
    ///
    /// Off by default. Scored against the true word boundaries of despaced
    /// prose it is not worth its cost: F1 reads higher without it at four of
    /// five sizes from 8 kB to 1 MB and 0.0001 lower at the fifth, while the
    /// spectral pass costs 0.39 s on a 3.2 MB input and 0.72 s on a 7.0 MB
    /// one. Recall rises slightly without it and precision falls slightly, and
    /// the two cancel. A caller whose input has real regime changes rather
    /// than word boundaries may still want it.
    pub spectral_fusion: bool,
    /// Union the base lexer's structural boundaries in, so seam is never
    /// worse than the base tokenizer regardless of input length - the
    /// breadcrumbs trex always has, needing no statistics.
    pub lexer_fusion: bool,
    /// Backoff floor: an order's context must have at least this many
    /// observations before it is trusted over a shorter context.
    ///
    /// Two, and a floor this low is measured to be the wrong choice on any
    /// input large enough to have statistics. Scored against the true word
    /// boundaries of despaced text over seven languages at 131 kB and 524 kB,
    /// two wins none of the twelve cells: eight wins seven, four two and
    /// sixteen two, and the gap grows with how much a language inflects - on
    /// Czech at order 3 a floor of eight is worth 0.034 of F1.
    ///
    /// It stays at two because a floor only means something once the input can
    /// meet it. Over a couple of dozen bytes no context is seen eight times at
    /// any order, so nothing clears the floor, every entropy reading is zero
    /// and the segmentation falls back to the lexer's boundaries alone. What
    /// the right floor is at that end has not been measured.
    pub min_count: u32,
    /// Iterative confidence passes. `0` takes the count [`auto_passes`] gives
    /// for the input's length, one pass at every length; a nonzero value
    /// forces that many passes.
    pub passes: usize,
    /// Merge contexts whose follower distributions resonate, keeping one
    /// prototype for each class instead of a row per distinct context.
    ///
    /// `None` keeps every context its own row, which is the behavior every
    /// figure recorded for this module was measured under.
    ///
    /// `Some(rho)` places each context into the first class it resonates
    /// with, in the Adaptive Resonance sense: a class matches when the
    /// fuzzy intersection of the context's follower distribution with the
    /// class prototype covers at least `rho` of the context's own mass, and
    /// a context that resonates with none mints a class of its own. So
    /// `rho` is a floor on how much of a context's evidence its prototype
    /// has to reproduce, and `1.0` admits only exact agreement.
    ///
    /// The trade this makes is rows against fidelity, and both ends of it are
    /// already measurable here: rows are counted directly, and what the
    /// merging costs shows up in boundary recovery against known word
    /// boundaries. A sweep that does not reproduce the unclustered numbers
    /// at `Some(1.0)` has changed something other than sharing.
    pub vigilance: Option<f32>,
}

impl Default for SeamConfig {
    fn default() -> Self {
        Self {
            order: 3,
            cut_threshold: 0.6,
            min_seg: 1,
            spectral_fusion: false,
            lexer_fusion: true,
            min_count: 2,
            passes: 0,
            vigilance: None,
        }
    }
}

/// Auto-gated confidence-pass count. A length sweep measured single-pass vs
/// multi-pass boundary recovery across
/// corpus sizes: single-pass recovery, lifted by lexer fusion and order
/// backoff, climbs steadily and saturates near F1 0.94 by ~256 bytes (about a
/// few sentences), and multi-pass reinforcement does not reliably beat it at
/// any size - it is neutral to slightly harmful from roughly 30 to 340 bytes.
/// So the gate keeps a single pass everywhere; the `passes` knob forces more
/// for exploration. The byte-length argument is retained so a future,
/// stronger reinforcement rule can re-introduce a real length gate.
#[must_use]
pub fn auto_passes(_n: usize) -> usize {
    1
}

/// A recognized lexer token at or below this many bytes is treated as atomic:
/// weak entropy cuts inside it are suppressed, so seam never shreds a real
/// short unit (an identifier, a word). A larger token - the spaceless blob the
/// lexer collapses to one Word - is left open, since there the lexer fails and
/// entropy is the only segmentation signal.
const SMALL_TOKEN_BYTES: usize = 12;

/// The per-position bidirectional reading plus the cuts derived from it.
#[derive(Clone, Debug, Default)]
pub struct SeamField {
    /// Input byte length.
    pub len: usize,
    /// Forward branching entropy at each byte (the boundary-after signal).
    pub fwd_entropy: Vec<f32>,
    /// Backward branching entropy at each byte (the boundary-before signal).
    pub bwd_entropy: Vec<f32>,
    /// Seam strength for a cut before byte `t` (`boundary[0]` is unused).
    pub boundary: Vec<f32>,
    /// Byte offsets that start a segment, ascending, always including `0`.
    pub cuts: Vec<usize>,
}

impl SeamField {
    /// The forward branching entropy at `byte`: the boundary-after signal.
    /// `0.0` past the input, where there is nothing ahead to branch into.
    #[must_use]
    pub fn fwd_at(&self, byte: usize) -> f32 {
        self.fwd_entropy.get(byte).copied().unwrap_or(0.0)
    }

    /// The backward branching entropy at `byte`: the boundary-before signal.
    #[must_use]
    pub fn bwd_at(&self, byte: usize) -> f32 {
        self.bwd_entropy.get(byte).copied().unwrap_or(0.0)
    }

    /// The seam strength of a cut before `byte`.
    #[must_use]
    pub fn strength_at(&self, byte: usize) -> f32 {
        self.boundary.get(byte).copied().unwrap_or(0.0)
    }

    /// Does a segment start at `byte`? The cuts are ascending, so this is a
    /// binary search.
    #[must_use]
    pub fn starts_segment(&self, byte: usize) -> bool {
        self.cuts.binary_search(&byte).is_ok()
    }

    /// The segments as half-open `[start, end)` byte spans.
    #[must_use]
    pub fn segments(&self) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = self.cuts.windows(2).map(|w| (w[0], w[1])).collect();
        if let Some(&last) = self.cuts.last()
            && last < self.len
        {
            out.push((last, self.len));
        }
        out
    }

    /// The internal cut offsets (every segment start except `0`), the form
    /// the boundary-recovery scorer compares against truth.
    #[must_use]
    pub fn internal_cuts(&self) -> Vec<usize> {
        self.cuts.iter().copied().filter(|&c| c != 0 && c != self.len).collect()
    }

    /// The high-tier seam cuts (ascending): cut offsets whose bidirectional
    /// boundary strength is at least one standard deviation above the mean.
    /// These are the genuine predictability breaks - where the past most stops
    /// predicting the future, or the content regime shifts - as opposed to the
    /// ordinary delimiter boundaries that also populate [`SeamField::cuts`].
    /// This is the discriminating signal the `@seam` anchor fires on; a plain
    /// cut at every token start would make the anchor a no-op in spaced text.
    #[must_use]
    pub fn strong_cuts(&self) -> Vec<usize> {
        self.cuts_past(1.0, false)
    }

    /// The cuts, other than `0`, of positive strength at least `sigma`
    /// standard deviations above the mean strength, or past it when
    /// `strict`, ascending. The positions are this field's own: bytes for the
    /// byte reader, symbols for [`analyze_symbols`].
    #[must_use]
    pub(crate) fn cuts_past(&self, sigma: f32, strict: bool) -> Vec<usize> {
        if self.boundary.len() <= 1 {
            return Vec::new();
        }
        // Timed apart from the field's own phases because it is not inside
        // them: the caller wraps `analyze` and this together, and this walks
        // every boundary reading twice - once for the mean and once for the
        // deviation from it.
        let _reading = crate::trace::phase("the seam field: the strong cuts");
        let vals = &self.boundary[1..];
        let mean = vals.iter().sum::<f32>() / vals.len() as f32;
        let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / vals.len() as f32;
        let thr = mean + sigma * var.sqrt();
        self.cuts
            .iter()
            .copied()
            .filter(|&c| {
                c > 0
                    && c < self.boundary.len()
                    && self.boundary[c] > 0.0
                    && if strict { self.boundary[c] > thr } else { self.boundary[c] >= thr }
            })
            .collect()
    }
}

/// The followers of one context, how many times it was seen, and the entropy
/// sum over its counts.
///
/// A context's followers are held as pairs and scanned linearly rather than
/// hashed. Contexts overwhelmingly have few distinct followers, and for a
/// handful of entries a contiguous scan beats a hash table that allocates,
/// hashes and chases a pointer - and there is one of these per context, so
/// the allocation is the dominant part.
///
/// `total` is carried as the pairs are added, because the backoff reads it from
/// every context it tries and answers most of them without going further.
///
/// The entropy's `sum c*log2(c)` is neither carried as the counts arrive nor
/// retaken at every reading; both of those measure worse.
///
/// Carrying it costs two [`crate::spectral::fixed_term`] calls per bump, and
/// that table holds a few thousand entries where a follower's count climbs
/// without bound over a long input - so over 7.34 MB of source, 95.1% of those
/// calls are past its end and compute a `log2`, nineteen million of them.
/// Retaking it at every reading walks the pairs instead, and over the same
/// corpus the byte grain takes 14,684,996 readings walking 82,256,472 pairs,
/// which is worse again: the model is built to completion before the first
/// reading, and the symbol grain's 3,300,000 readings are spread over 82
/// contexts, so a context is asked for the same answer about forty thousand
/// times.
///
/// So it is computed at the first reading that wants it and kept until the list
/// changes, which `bump` and `rewrite_as` are the only two things that do. A
/// `Cell` rather than a field filled by a pass of its own, because the context
/// merge comes between the counting and the reading and rewrites lists - a value
/// filled before it would be stale after it, and one filled after it would have
/// to know whether it ran.
///
/// Every form is the same value, in units of
/// [`crate::spectral::FIXED_LOG_SCALE`]. Integer addition is exact and
/// associative, and the carried deltas telescope to `term(c) - term(0)` with
/// `term(0)` zero, so a sum taken afresh over the same counts is the same fixed
/// point.
#[derive(Clone)]
struct Followers {
    items: Vec<(u32, u32)>,
    total: u32,
    /// The entropy of `items`, or `NaN` where the list has changed since it was
    /// last answered. `NaN` rather than a sentinel value, because every real
    /// entropy including zero is a reading a context can genuinely have.
    settled: std::cell::Cell<f32>,
}

impl Default for Followers {
    fn default() -> Followers {
        Followers { items: Vec::new(), total: 0, settled: std::cell::Cell::new(f32::NAN) }
    }
}

impl Followers {
    /// Count one occurrence of `sym` after this context, answering how many
    /// pairs it walked to find it.
    ///
    /// The walk is what the layout rests on, and this reports it so the claim
    /// is checkable. The figure belongs to a grain as much as to a corpus: over
    /// `benches/engine_surface` it measures 1.55 pairs a bump on the token
    /// stream and 1.17 on the supertokens, and 3.49 on the same corpus's bytes,
    /// where the alphabet is 256 rather than the token kinds. The byte grain is
    /// also where the counting costs most - 305.566 ms against the token
    /// stream's 106.138 - so a layout read off the token figure is read away
    /// from where it costs.
    /// A caller carrying that in a register and handing it over once costs
    /// nothing; asking per call would cost more than the call.
    fn bump(&mut self, sym: u32) -> usize {
        self.settled.set(f32::NAN);
        self.total += 1;
        for (walked, (s, c)) in self.items.iter_mut().enumerate() {
            if *s == sym {
                *c += 1;
                return walked + 1;
            }
        }
        let walked = self.items.len();
        self.items.push((sym, 1));
        walked
    }

    /// Shannon entropy in bits, by the count identity
    /// `H = log2(t) - (sum_c c*log2(c)) / t` over the integer counts, so every
    /// term comes from a table rather than a libm call per follower where the
    /// count is inside it. The sum is taken here rather than carried: see the
    /// struct's own note for what carrying it costs. Equal to the p-form up to
    /// f32 association (the parity test holds it within 1e-4); the consumers
    /// threshold entropy at coarse cut points, far above that noise.
    fn entropy(&self, lut: &[i64]) -> f32 {
        let held = self.settled.get();
        if !held.is_nan() {
            return held;
        }
        if self.total == 0 {
            return 0.0;
        }
        let sum: i64 = self.items.iter().map(|(_, c)| crate::spectral::fixed_term(lut, *c)).sum();
        let t = f64::from(self.total);
        let h = f64::from(log2_count(self.total))
            - sum as f64 / (crate::spectral::FIXED_LOG_SCALE * t);
        let out = h.max(0.0) as f32;
        self.settled.set(out);
        out
    }
}

/// `log2(t)` for an integer total, from a one-time exact table: the identity
/// asks for it once per position and order, and totals are bounded by the
/// branching-context windows, far below the table size.
#[inline]
fn log2_count(t: u32) -> f32 {
    const N: usize = 4096;
    static LUT: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let lut = LUT.get_or_init(|| (0..N as u32).map(|t| (t.max(1) as f32).log2()).collect());
    if let Some(&v) = lut.get(t as usize) {
        return v;
    }
    (t as f32).log2()
}

/// Widest packed context key that indexes a table rather than a map: at
/// sixteen bits the table is 65536 followers, two megabytes, one load per
/// lookup where the map hashes, probes and compares. Past it the key space
/// outgrows what is worth zeroing per call, and the map takes over.
///
/// It also outgrows the cache, which is the larger effect while counting. A
/// key is `8 * order` bits, so the widths on offer step 8, 16, 24 and 32 -
/// 256 bytes, two megabytes, 512 megabytes, 128 gigabytes. Two megabytes and
/// 512 are neighbors here, with nothing between a table that fits in L3 and
/// one that is a main-memory touch a lookup. Giving order three a direct
/// table measures 8% to 32% slower than the map it replaces, over three
/// alternating rounds with the narrower orders as controls, so sixteen is the
/// only width both worth having and resident.
const DIRECT_TABLE_MAX_KEY_BITS: u32 = 16;

/// The buckets a follower scan's length is counted in, for the trace.
///
/// A mean cannot tell a list that is short in every context from one that is
/// short in most and long in a few, and the two want different answers - the
/// first a different layout, the second a different container on the heavy
/// contexts alone. Over the bytes the mean is 3.49 pairs a bump, which is
/// consistent with either.
const WALK_BUCKETS: [&str; 6] = [
    "the seam field: bumps walking 1 pair",
    "the seam field: bumps walking 2",
    "the seam field: bumps walking 3",
    "the seam field: bumps walking 4",
    "the seam field: bumps walking 5 to 8",
    "the seam field: bumps walking 9 or more",
];

/// The same buckets, counted over the contexts rather than over the bumps.
///
/// The walk histogram says a minority of bumps does most of the walking. It
/// cannot say whether those bumps are spread over a few contexts or many, and
/// that decides whether a container chosen per context has a handful of sites
/// or thousands.
const CONTEXT_BUCKETS: [&str; 6] = [
    "the seam field: contexts holding 1 follower",
    "the seam field: contexts holding 2",
    "the seam field: contexts holding 3",
    "the seam field: contexts holding 4",
    "the seam field: contexts holding 5 to 8",
    "the seam field: contexts holding 9 or more",
];

/// Which of [`WALK_BUCKETS`] a scan of `walked` pairs belongs to.
fn walk_bucket(walked: usize) -> usize {
    match walked {
        0 | 1 => 0,
        2 => 1,
        3 => 2,
        4 => 3,
        5..=8 => 4,
        _ => 5,
    }
}

/// One order's contexts and their followers, held as a table when the key
/// is short enough and as a map otherwise.
#[derive(Clone)]
enum ContextModel {
    Table(Vec<Followers>),
    Map(OrderModel),
}

impl ContextModel {
    fn new(key_bits: u32) -> Self {
        if key_bits <= DIRECT_TABLE_MAX_KEY_BITS {
            ContextModel::Table(vec![Followers::default(); 1 << key_bits])
        } else {
            ContextModel::Map(OrderModel::default())
        }
    }

    /// What the follower scans would have cost with every list ordered by how
    /// often its followers are asked for, which is the ceiling on what any
    /// reordering can save.
    ///
    /// A follower asked for `c` times, `p` deep in its list, costs `p * c` walking.
    /// The counts are known once the pass is done, so sorting each list by its
    /// counts and summing `rank * count` is the best an ordering could do even
    /// knowing the whole input in advance. Against the pairs actually walked it
    /// says whether the insertion order is already near that, in which case
    /// reordering gains nothing whatever it costs.
    ///
    /// It is near it. Over `benches/engine_surface`'s bytes a bump walks
    /// 153,688,678 pairs where the best order would walk 137,985,991, so every
    /// reordering there could ever be is worth a tenth of the scan. The same
    /// two numbers say more about the layout than about the order: a follower
    /// is a byte, so a context holding a direct table of 256 would walk
    /// 44,054,976 - one a bump - and what bounds that is the share of a bump
    /// spent walking rather than packing, indexing and touching memory, which
    /// nothing here has measured.
    fn ordered_walk(&self) -> u64 {
        let mut total = 0u64;
        let mut each = |f: &Followers| {
            let mut counts: Vec<u32> = f.items.iter().map(|&(_, c)| c).collect();
            counts.sort_unstable_by(|a, b| b.cmp(a));
            for (rank, c) in counts.iter().enumerate() {
                total += (rank as u64 + 1) * u64::from(*c);
            }
        };
        match self {
            ContextModel::Table(t) => t.iter().filter(|f| f.total > 0).for_each(&mut each),
            ContextModel::Map(m) => m.values().for_each(&mut each),
        }
        total
    }

    /// Answers what [`Followers::bump`] walked to find its pair.
    #[inline]
    fn bump(&mut self, key: u64, sym: u32) -> usize {
        match self {
            ContextModel::Table(t) => t[key as usize].bump(sym),
            ContextModel::Map(m) => m.entry(key).or_default().bump(sym),
        }
    }

    /// How many contexts hold an observation.
    ///
    /// A map knows its own occupancy; a table is sized to the whole key space
    /// and has to be walked to find out how much of it is in use, so this is
    /// for a caller reading the model rather than for the hot path.
    fn seen(&self) -> usize {
        match self {
            ContextModel::Table(t) => t.iter().filter(|f| f.total > 0).count(),
            ContextModel::Map(m) => m.len(),
        }
    }

    /// Add this model's contexts to `into`, bucketed by how many followers each
    /// holds, and answer how many follower pairs they hold between them. A
    /// context nothing was ever counted into is not a context.
    ///
    /// The total is what separates a deep scan that re-finds a symbol already
    /// in the list from one that appends a new one, and that decides whether
    /// any ordering could help at all: a pair is appended exactly once, so the
    /// pairs held are the appends, and every other bump found what it sought.
    fn follower_spread(&self, into: &mut [u64]) -> u64 {
        let mut pairs = 0u64;
        let mut add = |f: &Followers| {
            into[walk_bucket(f.items.len())] += 1;
            pairs += u64::try_from(f.items.len()).expect("a follower list within the width");
        };
        match self {
            ContextModel::Table(t) => t.iter().filter(|f| f.total > 0).for_each(&mut add),
            ContextModel::Map(m) => m.values().for_each(&mut add),
        }
        pairs
    }

    /// The followers of a context that has been seen at all.
    #[inline]
    fn get(&self, key: u64) -> Option<&Followers> {
        match self {
            ContextModel::Table(t) => {
                let f = &t[key as usize];
                (f.total > 0).then_some(f)
            }
            ContextModel::Map(m) => m.get(&key),
        }
    }

    /// Merge contexts whose follower distributions resonate, and report how
    /// many rows carried real observations before and how many classes they
    /// collapsed to.
    ///
    /// Each context is read as a distribution over followers rather than as
    /// counts, so two contexts predicting the same thing at different
    /// frequencies resonate. A class matches when the fuzzy intersection of
    /// the two distributions - the sum of their per-follower minima, which
    /// for two distributions is the mass they agree on - reaches `rho`. The
    /// first class to match takes the context and moves toward it by its
    /// share of the class mass; a context matching none mints a class.
    ///
    /// Each context then keeps its own `total`, so the backoff's
    /// `min_count` test still asks how often that context was seen, and
    /// takes the class's shape rescaled to that total. Merging is about what
    /// a context predicts, not about how often it was observed, and folding
    /// the two together would silently make rare contexts look frequent.
    ///
    /// At `rho >= 1.0` only exactly-agreeing distributions merge, so the
    /// field this produces is the unclustered one and the sweep has a
    /// control that costs nothing to run.
    fn cluster(&mut self, rho: f32) -> (usize, usize) {
        let rows: Vec<&mut Followers> = match self {
            ContextModel::Table(t) => t.iter_mut().filter(|f| f.total > 0).collect(),
            ContextModel::Map(m) => m.values_mut().filter(|f| f.total > 0).collect(),
        };
        let before = rows.len();
        // Prototypes as sorted (symbol, share) pairs, so the intersection is
        // a merge of two sorted runs rather than a lookup per follower.
        let mut protos: Vec<Vec<(u32, f32)>> = Vec::new();
        let mut masses: Vec<f32> = Vec::new();
        for f in rows {
            let total = f.total as f32;
            let mut x: Vec<(u32, f32)> =
                f.items.iter().map(|&(s, c)| (s, c as f32 / total)).collect();
            x.sort_unstable_by_key(|&(s, _)| s);

            let mut placed = None;
            for (i, w) in protos.iter().enumerate() {
                if fuzzy_overlap(&x, w) >= rho {
                    placed = Some(i);
                    break;
                }
            }
            let idx = match placed {
                Some(i) => {
                    // The prototype moves toward the context by the context's
                    // share of the class's mass, so an early member does not
                    // outweigh everything that joins later.
                    let beta = 1.0 / (masses[i] + 1.0);
                    blend_into(&mut protos[i], &x, beta);
                    masses[i] += 1.0;
                    i
                }
                None => {
                    protos.push(x);
                    masses.push(1.0);
                    protos.len() - 1
                }
            };
            rewrite_as(f, &protos[idx]);
        }
        (before, protos.len())
    }
}

/// The mass two distributions agree on: the sum of their per-follower minima.
///
/// Both runs are sorted by symbol, so this walks them together. For two
/// distributions the result is in `[0, 1]` and reaches one only when they are
/// identical, which is what makes it usable as a vigilance test directly.
fn fuzzy_overlap(x: &[(u32, f32)], w: &[(u32, f32)]) -> f32 {
    let (mut i, mut j, mut acc) = (0usize, 0usize, 0.0f32);
    while i < x.len() && j < w.len() {
        match x[i].0.cmp(&w[j].0) {
            std::cmp::Ordering::Equal => {
                acc += x[i].1.min(w[j].1);
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    acc
}

/// Move `w` a `beta` fraction toward `x`, keeping it sorted by symbol.
fn blend_into(w: &mut Vec<(u32, f32)>, x: &[(u32, f32)], beta: f32) {
    let (mut i, mut j) = (0usize, 0usize);
    let mut out: Vec<(u32, f32)> = Vec::with_capacity(w.len() + x.len());
    while i < w.len() || j < x.len() {
        let take_w = j >= x.len() || (i < w.len() && w[i].0 <= x[j].0);
        let take_x = i >= w.len() || (j < x.len() && x[j].0 <= w[i].0);
        match (take_w, take_x) {
            (true, true) => {
                out.push((w[i].0, w[i].1 * (1.0 - beta) + x[j].1 * beta));
                i += 1;
                j += 1;
            }
            (true, false) => {
                out.push((w[i].0, w[i].1 * (1.0 - beta)));
                i += 1;
            }
            _ => {
                out.push((x[j].0, x[j].1 * beta));
                j += 1;
            }
        }
    }
    *w = out;
}

/// Give `f` the class's shape at `f`'s own total, so the entropy the backoff
/// reads matches the counts that are actually there.
fn rewrite_as(f: &mut Followers, proto: &[(u32, f32)]) {
    f.settled.set(f32::NAN);
    let total = f.total;
    f.items.clear();
    let mut assigned = 0u32;
    for (k, &(s, share)) in proto.iter().enumerate() {
        // The last follower takes the remainder, so the counts sum to the
        // total exactly rather than to whatever rounding leaves.
        let c = if k + 1 == proto.len() {
            total.saturating_sub(assigned)
        } else {
            (share * total as f32).round() as u32
        };
        if c == 0 {
            continue;
        }
        assigned = assigned.saturating_add(c);
        f.items.push((s, c));
    }
    // A prototype whose shares all round to zero would leave the context with
    // no followers at all, which reads as unseen rather than as merged.
    if f.items.is_empty() && total > 0 {
        let s = proto.first().map_or(0, |&(s, _)| s);
        f.items.push((s, total));
    }
}

/// Pack a context of at most eight bytes into an integer key.
///
/// The models were keyed by the byte slice itself, so every insert and every
/// query hashed a slice. A context that fits a `u64` is hashed as one word
/// instead, and the order it belongs to is already the index of the map it
/// is in, so it needs no room in the key.
#[inline]
fn pack_context(ctx: &[u8]) -> u64 {
    let mut k = 0u64;
    for &b in ctx {
        k = (k << 8) | u64::from(b);
    }
    k
}

/// A multiply-mix hasher for the packed context keys.
///
/// The default hasher is built to resist collision attacks on untrusted keys;
/// these keys are bytes of the input being segmented, and the map is thrown
/// away at the end of the call, so that protection gains nothing and costs a
/// full hash per lookup where one multiply and a shift will do.
///
/// One multiply and a shift rather than nothing at all: [`pack_context`] and
/// [`pack_symbols`] lay their symbols into the low bits and leave the rest of
/// the word zero, and a table reads its control byte from the high bits, so a
/// key handed over unmixed would give every entry the same one. The coder's
/// `U64Map` is the other map and does pass its key through, because what
/// reaches it has been mixed before it arrives.
#[derive(Clone, Copy, Default)]
struct ContextHasher(u64);

impl std::hash::Hasher for ContextHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }

    fn write_u64(&mut self, n: u64) {
        // Fibonacci mixing: multiply by the 64-bit golden-ratio constant and
        // keep the high bits, which is where the mixing lands.
        self.0 = (self.0 ^ n).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.0 ^= self.0 >> 29;
    }
}

#[derive(Clone, Copy, Default)]
struct ContextHash;

impl std::hash::BuildHasher for ContextHash {
    type Hasher = ContextHasher;
    fn build_hasher(&self) -> ContextHasher {
        ContextHasher(0)
    }
}

/// One order's model: packed context to what followed it.
type OrderModel = HashMap<u64, Followers, ContextHash>;

/// Pack a context of symbols at `bits` per symbol into an integer key, the
/// symbol-stream counterpart of [`pack_context`]: the width is whatever the
/// alphabet in use needs rather than a byte, so a token-kind context packs
/// ten deep where a byte context packs eight.
#[inline]
fn pack_symbols(ctx: &[u32], bits: u32) -> u64 {
    let mut k = 0u64;
    for &s in ctx {
        k = (k << bits) | u64::from(s);
    }
    k
}

/// Segment a stream of symbols by bidirectional branching entropy.
///
/// The reading a seam is: a boundary is where the past most stops predicting
/// what comes next and the future most stops predicting what came before. That
/// is a statement about a sequence, not about bytes, so it lifts to the token
/// and supertoken streams by running over their symbols instead.
///
/// What does not lift is the byte path's fusion. [`analyze_with`] also folds in
/// the spectral coarse prior and the base lexer's structural boundaries, and
/// both are facts about bytes: there is no spectral frame over a token stream
/// and the lexer's boundaries are what produced the tokens in the first place.
/// So this is the entropy core alone, and the byte path keeps its extra
/// evidence rather than being reduced to match.
///
/// Positions are in symbol space. A caller wanting byte offsets maps them
/// through the spans of whatever the symbols came from.
#[must_use]
pub fn analyze_symbols(symbols: &[u32], cfg: &SeamConfig) -> SeamField {
    let n = symbols.len();
    // Three readings a symbol, written before the first phase opens, as the
    // byte grain writes three a byte. Fewer here only because there are fewer
    // symbols than bytes.
    let allotting = crate::trace::phase("the seam field, by symbol: allotting the readings");
    let mut field = SeamField {
        len: n,
        fwd_entropy: vec![0.0; n],
        bwd_entropy: vec![0.0; n],
        boundary: vec![0.0; n],
        cuts: Vec::new(),
    };
    drop(allotting);
    if n <= 1 {
        if n == 1 {
            field.cuts.push(0);
        }
        return field;
    }
    // A context packs into a `u64` key at the width of the alphabet in use,
    // and the order is clamped to what packs, by the byte path's rule: a
    // longer context would alias two contexts onto one key, which is a wrong
    // reading rather than a slow one. Token kinds are six bits, so ten deep
    // packs; a custom shape's sixteen-bit id leaves room for three.
    let bits = symbols.iter().max().map_or(1, |&m| (u32::BITS - m.leading_zeros()).max(1));
    let k = cfg.order.clamp(1, (64 / bits as usize).max(1));

    // One model per context order, so a short stream can back off to a context
    // that has been seen at all. The token stream is far shorter than the byte
    // stream it came from, which makes the backoff matter more here, not less.
    let mut fwd: Vec<ContextModel> = (1..=k).map(|o| ContextModel::new(bits * o as u32)).collect();
    let mut bwd: Vec<ContextModel> = (1..=k).map(|o| ContextModel::new(bits * o as u32)).collect();
    // The count table resolved once for the build: `bump` runs `n * k` times
    // per direction, which is too hot to re-resolve a `OnceLock` inside.
    let lut = crate::spectral::fixed_log_table();
    let counting = crate::trace::phase("the seam field, by symbol: counting the contexts");
    // What the follower scans walk, summed. A register add a bump, handed over
    // once, because the layout rests on those scans being short and nothing
    // here has ever said how short.
    let mut walked = 0u64;
    for t in 0..n - 1 {
        for o in 1..=k.min(t + 1) {
            walked +=
                fwd[o - 1].bump(pack_symbols(&symbols[t + 1 - o..=t], bits), symbols[t + 1]) as u64;
        }
    }
    for t in 1..n {
        for o in 1..=k.min(n - t) {
            walked += bwd[o - 1].bump(pack_symbols(&symbols[t..t + o], bits), symbols[t - 1]) as u64;
        }
    }
    crate::trace::counted("the seam field, by symbol: follower pairs walked", walked);
    drop(counting);

    // How many contexts the models hold once the count is in, and how many
    // bumps put them there. The two together say what a bump costs and how
    // much of the model a bump has to reach, which a duration on its own
    // cannot: the same pass over a stream carrying a handful of distinct
    // contexts and one carrying a million walks the same loop and touches very
    // different amounts of memory. Counted only where the trace is kept, since
    // a table reports its occupancy by scanning itself.
    if crate::trace::keeping() {
        let held: usize = fwd.iter().chain(bwd.iter()).map(ContextModel::seen).sum();
        let bumps: usize = (1..=k).map(|o| n.saturating_sub(o) * 2).sum();
        crate::trace::counted(
            "the seam field, by symbol: contexts held",
            u64::try_from(held).expect("a context count within the counter's width"),
        );
        crate::trace::counted(
            "the seam field, by symbol: bumps taken",
            u64::try_from(bumps).expect("a bump count within the counter's width"),
        );
    }

    let reading = crate::trace::phase("the seam field, by symbol: an entropy a position");
    let floor = cfg.min_count.max(1);
    // How many contexts this pass looks up. It packs a key and reaches a model
    // exactly as the count above does, and differs in reading the followers
    // rather than adding to them - so the two passes divided by their own
    // lookup counts say what the adding costs, which neither duration says
    // alone. Backing off stops at the first order that clears the floor, so
    // the count is not the count pass's and has to be carried rather than
    // derived. A register add a turn, reported once.
    // Beside the lookups: how many entropies the backoff then reads, and how
    // many follower pairs those reads walk between them. Together the three
    // divide this pass into the lookup, the call and the sum over the
    // followers. Whether rungs are kept is read once into a local, so a pass
    // nobody is watching carries a predictable branch and no counter at all.
    let watching = crate::trace::keeping();
    let (mut looked, mut read, mut pairs) = (0u64, 0u64, 0u64);
    for t in 0..n {
        // Longest context with enough observations to be trusted.
        for o in (1..=k.min(t + 1)).rev() {
            looked += u64::from(watching);
            if let Some(c) = fwd[o - 1].get(pack_symbols(&symbols[t + 1 - o..=t], bits))
                && c.total >= floor
            {
                if watching {
                    read += 1;
                    pairs += u64::try_from(c.items.len())
                        .expect("a follower list within the counter's width");
                }
                field.fwd_entropy[t] = c.entropy(lut);
                break;
            }
        }
        for o in (1..=k.min(n - t)).rev() {
            looked += u64::from(watching);
            if let Some(c) = bwd[o - 1].get(pack_symbols(&symbols[t..t + o], bits))
                && c.total >= floor
            {
                if watching {
                    read += 1;
                    pairs += u64::try_from(c.items.len())
                        .expect("a follower list within the counter's width");
                }
                field.bwd_entropy[t] = c.entropy(lut);
                break;
            }
        }
    }
    crate::trace::counted("the seam field, by symbol: contexts looked up", looked);
    crate::trace::counted("the seam field, by symbol: entropies read", read);
    crate::trace::counted("the seam field, by symbol: follower pairs the readings span", pairs);

    drop(reading);

    let _deriving = crate::trace::phase("the seam field, by symbol: deriving the cuts");
    // A boundary is where the forward reading rises going in and the backward
    // reading rises coming out, so the score is the pair of rises at one point.
    for t in 1..n {
        let fwd_rise = (field.fwd_entropy[t - 1] - field.fwd_entropy[t]).max(0.0);
        let bwd_rise = (field.bwd_entropy[t] - field.bwd_entropy[t - 1]).max(0.0);
        field.boundary[t] = fwd_rise + bwd_rise;
    }

    field.cuts.push(0);
    for t in 1..n {
        if field.boundary[t] > 0.0
            && field.boundary[t] >= field.boundary[t - 1]
            && (t + 1 >= n || field.boundary[t] >= field.boundary[t + 1])
        {
            field.cuts.push(t);
        }
    }
    field
}

/// Segment the token stream by branching entropy over token kinds.
///
/// A cut here is where the sequence of token kinds stops predicting itself,
/// which is a structural boundary - the end of a construct - rather than the
/// word boundary the byte reading finds. Cuts are returned as byte offsets so
/// they key the same way every other axis does.
#[must_use]
pub fn analyze_tokens(toks: &[crate::token::Token], cfg: &SeamConfig) -> Vec<usize> {
    let gathering = crate::trace::phase("the seam field, by symbol: the symbols it reads");
    let sig: Vec<&crate::token::Token> = toks.iter().filter(|t| t.is_significant()).collect();
    let symbols: Vec<u32> = sig.iter().map(|t| t.kind.code()).collect();
    drop(gathering);
    analyze_symbols(&symbols, cfg)
        .cuts
        .into_iter()
        .filter_map(|c| sig.get(c).map(|t| t.start()))
        .collect()
}

/// Segment the supertoken stream by branching entropy over unit roles.
#[must_use]
pub fn analyze_supertokens(units: &[crate::supertoken::SuperToken], cfg: &SeamConfig) -> Vec<usize> {
    let symbols: Vec<u32> = units.iter().map(|u| u.role.code()).collect();
    analyze_symbols(&symbols, cfg)
        .cuts
        .into_iter()
        .filter_map(|c| units.get(c).map(|u| u.start))
        .collect()
}

/// The cuts an anchor reads from a symbol field, never the first symbol:
/// every cut the reading finds where `past` is `None`, and otherwise those at
/// least `sigma` standard deviations above the mean strength, or past it when
/// `strict`.
fn anchor_cuts(field: &SeamField, past: Option<(f32, bool)>) -> Vec<usize> {
    match past {
        None => field.internal_cuts(),
        Some((sigma, strict)) => field.cuts_past(sigma, strict),
    }
}

/// The token-grain cuts `@seam:token` reads, as byte offsets: the cuts of
/// the token-kind stream other than its first token, all of them for a bare
/// anchor and those past `past` for a written cut.
#[must_use]
pub(crate) fn token_cuts(toks: &[crate::token::Token], cfg: &SeamConfig, past: Option<(f32, bool)>) -> Vec<usize> {
    let sig: Vec<&crate::token::Token> = toks.iter().filter(|t| t.is_significant()).collect();
    let symbols: Vec<u32> = sig.iter().map(|t| t.kind.code()).collect();
    anchor_cuts(&analyze_symbols(&symbols, cfg), past)
        .into_iter()
        .filter_map(|c| sig.get(c).map(|t| t.start()))
        .collect()
}

/// The supertoken-grain cuts `@seam:super` reads, as byte offsets, chosen as
/// [`token_cuts`] chooses them.
#[must_use]
pub(crate) fn supertoken_cuts(
    units: &[crate::supertoken::SuperToken],
    cfg: &SeamConfig,
    past: Option<(f32, bool)>,
) -> Vec<usize> {
    let symbols: Vec<u32> = units.iter().map(|u| u.role.code()).collect();
    anchor_cuts(&analyze_symbols(&symbols, cfg), past)
        .into_iter()
        .filter_map(|c| units.get(c).map(|u| u.start))
        .collect()
}

#[cfg(test)]
mod grain_tests {
    use super::*;

    #[test]
    fn the_token_grain_cuts_where_the_byte_grain_does_not() {
        // Two statements. At byte grain a seam is at word boundaries; at
        // token grain the sequence of kinds is what has to stop predicting
        // itself, so a cut means the construct changed.
        let src = b"let x = 1 ; let y = 2 ; print x ; print y ;";
        let toks = crate::lexer::lex(src);
        let cfg = SeamConfig::default();
        let tok_cuts = analyze_tokens(&toks, &cfg);
        let byte_cuts = analyze(src).cuts;
        assert!(!tok_cuts.is_empty(), "the token stream has structure to cut");
        // The two readings are not the same set: they answer different
        // questions over different sequences.
        assert_ne!(tok_cuts, byte_cuts, "a lift is a re-grounding, not a coarser view");
        // Every token-grain cut lands on a token start, since that is the
        // space it was computed in.
        let starts: Vec<usize> =
            toks.iter().filter(|t| t.is_significant()).map(crate::token::Token::start).collect();
        assert!(tok_cuts.iter().all(|c| starts.contains(c)), "cuts key to token starts");
    }

    #[test]
    fn the_supertoken_grain_reads_the_unit_sequence() {
        let src = b"a = 1;\nb = 2;\nfoo(3);\nc = 4;\n";
        let toks = crate::lexer::lex(src);
        let units = crate::supertoken::supertokens_from(&toks, src);
        let cuts = analyze_supertokens(&units, &SeamConfig::default());
        assert!(!cuts.is_empty(), "the unit stream is segmented");
        let starts: Vec<usize> = units.iter().map(|u| u.start).collect();
        assert!(cuts.iter().all(|c| starts.contains(c)), "cuts key to unit starts");
    }

    #[test]
    fn degenerate_symbol_streams_are_safe() {
        let cfg = SeamConfig::default();
        assert!(analyze_symbols(&[], &cfg).cuts.is_empty());
        assert_eq!(analyze_symbols(&[7u32], &cfg).cuts, vec![0]);
        // A constant stream predicts itself perfectly, so nothing inside it
        // is a boundary.
        let flat = analyze_symbols(&[1u32; 40], &cfg);
        assert!(flat.cuts.len() <= 1, "nothing to cut in a constant stream");
    }
}

/// Segment `input` with the default configuration.
#[must_use]
pub fn analyze(input: &[u8]) -> SeamField {
    analyze_with(input, &SeamConfig::default())
}

/// Segment `input` by bidirectional branching entropy (with order backoff)
/// fused with the spectral coarse prior and the base lexer's structural
/// boundaries, refined over iterative confidence passes.
#[must_use]
pub fn analyze_with(input: &[u8], cfg: &SeamConfig) -> SeamField {
    let n = input.len();
    // Timed, because it is three readings a byte and nothing reported it: over
    // a seven-megabyte input these are three `f32` vectors of seven million
    // elements, eighty-eight megabytes, and every one of them is written before
    // the first phase opens.
    let allotting = crate::trace::phase("the seam field: allotting the readings");
    let mut field = SeamField {
        len: n,
        fwd_entropy: vec![0.0; n],
        bwd_entropy: vec![0.0; n],
        boundary: vec![0.0; n],
        cuts: Vec::new(),
    };
    drop(allotting);
    if n == 0 {
        return field;
    }
    if n == 1 {
        field.cuts.push(0);
        return field;
    }
    // Contexts are packed into a `u64` key, so eight bytes is the longest one
    // that stays distinct. A longer order would alias two different contexts
    // onto one key, which is a wrong reading rather than a slow one, so it is
    // clamped rather than allowed. Eight is far past the default of three and
    // past where a longer context still has observations to back it.
    let k = cfg.order.clamp(1, 8);

    // Multi-order models: `fwd_models[o-1]` maps an o-byte context to the
    // byte that follows it; `bwd_models[o-1]` maps an o-byte context to the
    // byte that precedes it. Holding every order lets a short or thin input
    // back off to a context that actually has observations. The key is the
    // packed context itself; a content-prefixed key with a pre-mixed tag
    // measures slower at every order on this map shape (0.93x at order 8,
    // 0.80x at 16), its fatter buckets costing more cache than the tag saves.
    // Timed, because it is not free and nothing reported it: at the default
    // order the widest of these is a direct table of 65,536 followers, two
    // megabytes, and there is one per direction. `Followers::default()` is not
    // all-zeros, so the vector is built by cloning the element rather than by
    // zeroing a block.
    let building = crate::trace::phase("the seam field: building the models");
    let mut fwd_models: Vec<ContextModel> = (1..=k).map(|o| ContextModel::new(8 * o as u32)).collect();
    let mut bwd_models: Vec<ContextModel> = (1..=k).map(|o| ContextModel::new(8 * o as u32)).collect();
    drop(building);
    // The count table resolved once for the build: `bump` runs `n * k` times
    // per direction, which is too hot to re-resolve a `OnceLock` inside.
    let lut = crate::spectral::fixed_log_table();
    let counting = crate::trace::phase("the seam field: counting the contexts");
    // What the build does, for the trace: how many times a context is bumped,
    // and how many follower pairs those bumps walk to find their symbol. `bump`
    // already answers the second and the answer was being dropped. Divided into
    // the phase they separate the map operation from the follower scan, which
    // the duration does not. The symbol grain reports the same three.
    let watching = crate::trace::keeping();
    let (mut bumps, mut pairs) = (0u64, 0u64);
    let mut spread = [0u64; WALK_BUCKETS.len()];
    // An order whose key fits the direct table is an index; a wider one is a
    // hash and a probe. At the default order two of the three orders are tables
    // and one is a map, and the phase reads them as one number.
    let mut into_a_table = 0u64;
    for t in 0..n - 1 {
        for o in 1..=k.min(t + 1) {
            let walked = fwd_models[o - 1]
                .bump(pack_context(&input[t + 1 - o..=t]), u32::from(input[t + 1]));
            if watching {
                bumps += 1;
                pairs += u64::try_from(walked).expect("a follower scan within the counter's width");
                spread[walk_bucket(walked)] += 1;
                into_a_table += u64::from(8 * o as u32 <= DIRECT_TABLE_MAX_KEY_BITS);
            }
        }
    }
    for t in 1..n {
        for o in 1..=k.min(n - t) {
            let walked =
                bwd_models[o - 1].bump(pack_context(&input[t..t + o]), u32::from(input[t - 1]));
            if watching {
                bumps += 1;
                pairs += u64::try_from(walked).expect("a follower scan within the counter's width");
                spread[walk_bucket(walked)] += 1;
                into_a_table += u64::from(8 * o as u32 <= DIRECT_TABLE_MAX_KEY_BITS);
            }
        }
    }
    crate::trace::counted("the seam field: bumps into a direct table", into_a_table);
    crate::trace::counted("the seam field: bumps into a map", bumps - into_a_table);
    for (name, count) in WALK_BUCKETS.iter().zip(spread) {
        crate::trace::counted(name, count);
    }
    if watching {
        let held: usize = fwd_models.iter().chain(bwd_models.iter()).map(ContextModel::seen).sum();
        crate::trace::counted(
            "the seam field: contexts held",
            u64::try_from(held).expect("a context count within the counter's width"),
        );
        let mut by_followers = [0u64; CONTEXT_BUCKETS.len()];
        let mut appended = 0u64;
        for m in fwd_models.iter().chain(bwd_models.iter()) {
            appended += m.follower_spread(&mut by_followers);
        }
        for (name, count) in CONTEXT_BUCKETS.iter().zip(by_followers) {
            crate::trace::counted(name, count);
        }
        crate::trace::counted("the seam field: follower pairs held", appended);
        crate::trace::counted("the seam field: bumps that found their symbol", bumps - appended);
        let ideal: u64 =
            fwd_models.iter().chain(bwd_models.iter()).map(ContextModel::ordered_walk).sum();
        crate::trace::counted("the seam field: follower pairs a best order would walk", ideal);
    }
    crate::trace::counted("the seam field: bumps taken", bumps);
    crate::trace::counted("the seam field: follower pairs a bump walks", pairs);
    drop(counting);

    // Contexts merge once the counts are in, so a class is formed from what
    // a context finally predicts rather than from what it predicted partway
    // through the input, which would depend on where in the stream it first
    // appeared.
    let merging = crate::trace::phase("the seam field: merging the contexts");
    if let Some(rho) = cfg.vigilance {
        let mut before = 0usize;
        let mut after = 0usize;
        for m in fwd_models.iter_mut().chain(bwd_models.iter_mut()) {
            let (b, a) = m.cluster(rho);
            before += b;
            after += a;
        }
        if std::env::var_os("TREX_SEAM_CLUSTER_REPORT").is_some() {
            eprintln!("seam cluster: vigilance {rho} kept {after} classes of {before} contexts");
        }
    }
    drop(merging);

    let reading = crate::trace::phase("the seam field: an entropy a position");
    // What this pass does per position, for the trace: how many contexts the
    // backoff looks up before one clears the floor, how many entropies it then
    // reads, and how many follower pairs those reads walk between them. The
    // three divide the duration into the lookup, the call and the sum over the
    // followers, which the duration alone does not separate. Whether rungs are
    // kept is read once into a local, so a pass nobody is watching carries a
    // predictable branch rather than a lock.
    let watching = crate::trace::keeping();
    let (mut looked, mut read, mut pairs) = (0u64, 0u64, 0u64);
    // Per-position branching entropy with order backoff: take the longest
    // context whose observation count clears `min_count`, else shorten.
    for t in 0..n {
        let hf = k.min(t + 1);
        for o in (1..=hf).rev() {
            looked += u64::from(watching);
            if let Some(d) = fwd_models[o - 1].get(pack_context(&input[t + 1 - o..=t]))
                && d.total >= cfg.min_count
            {
                if watching {
                    read += 1;
                    pairs += u64::try_from(d.items.len())
                        .expect("a follower list within the counter's width");
                }
                field.fwd_entropy[t] = d.entropy(lut);
                break;
            }
        }
        let hb = k.min(n - t);
        for o in (1..=hb).rev() {
            looked += u64::from(watching);
            if let Some(d) = bwd_models[o - 1].get(pack_context(&input[t..t + o]))
                && d.total >= cfg.min_count
            {
                if watching {
                    read += 1;
                    pairs += u64::try_from(d.items.len())
                        .expect("a follower list within the counter's width");
                }
                field.bwd_entropy[t] = d.entropy(lut);
                break;
            }
        }
    }
    crate::trace::counted("the seam field: contexts looked up for an entropy", looked);
    crate::trace::counted("the seam field: entropies read", read);
    // What the readings span rather than what they walk, and the distinction is
    // the whole point: a reading whose context has settled since the list last
    // changed answers from the kept value and walks nothing. This is what the
    // pairs would cost with no value kept, which is what says whether keeping
    // one is worth it.
    crate::trace::counted("the seam field: follower pairs the readings span", pairs);

    drop(reading);

    // Base seam strength: both sides branch.
    for t in 1..n {
        field.boundary[t] = field.fwd_entropy[t - 1] + field.bwd_entropy[t];
    }

    // Coarse multi-scale regime prior: a spectral change-point nudges the
    // seams around it upward.
    let fusing = crate::trace::phase("the seam field: the spectral prior");
    if cfg.spectral_fusion {
        const FUSION_BONUS: f32 = 1.5;
        let sf = crate::spectral::analyze(input);
        for &b in &sf.boundaries {
            let lo = b.saturating_sub(1).max(1);
            let hi = (b + 1).min(n - 1);
            for slot in field.boundary.iter_mut().take(hi + 1).skip(lo) {
                *slot += FUSION_BONUS;
            }
        }
    }

    // The base lexer's structural boundaries: deterministic, statistics-free,
    // always available - the breadcrumbs that make seam never worse than the
    // base tokenizer. Two products: `lexer_cuts` forces every token start as a
    // cut (union); `protect` marks the interior bytes of every small recognized
    // token, where a weak entropy cut would only shred a real unit on a short,
    // statistics-thin input. A large token is left unprotected, so entropy
    // still segments inside it - the one place the lexer fails.
    drop(fusing);
    let breadcrumbs = crate::trace::phase("the seam field: the lexer's boundaries");
    let mut lexer_cuts: Vec<usize> = Vec::new();
    let mut protect = vec![false; n];
    if cfg.lexer_fusion {
        for tok in crate::lexer::lex(input) {
            if tok.start() > 0 && tok.start() < n {
                lexer_cuts.push(tok.start());
            }
            if tok.end().saturating_sub(tok.start()) <= SMALL_TOKEN_BYTES {
                for slot in protect.iter_mut().take(tok.end().min(n)).skip(tok.start() + 1) {
                    *slot = true;
                }
            }
        }
    }

    // Iterative confidence: derive cuts, then (for more than one pass)
    // reinforce the seams that keep bracketing a recurring segment and
    // re-derive, so a short input builds confidence from its own structure.
    drop(breadcrumbs);
    let _deriving = crate::trace::phase("the seam field: deriving the cuts");
    let passes = if cfg.passes == 0 { auto_passes(n) } else { cfg.passes.max(1) };
    let base_boundary = field.boundary.clone();
    field.cuts = derive_cuts(&field.boundary, &lexer_cuts, &protect, cfg, n);
    for _ in 1..passes {
        // Every pass starts from `base_boundary`, not from the previous pass's result: that is
        // what stops the reinforcement compounding into a runaway (point 2 of the coupling's
        // stability argument, and the reason the orbit coupling can ride the same loop).
        let mut boosted = base_boundary.clone();
        reinforce_recurrence(&mut boosted, input, &field.cuts);
        field.cuts = derive_cuts(&boosted, &lexer_cuts, &protect, cfg, n);
        field.boundary = boosted;
    }
    field
}

/// Place cuts at local maxima of `boundary` above the adaptive threshold,
/// unioned with the base lexer's structural boundaries, respecting
/// `min_seg`. Offset `0` is always a cut.
fn derive_cuts(boundary: &[f32], lexer_cuts: &[usize], protect: &[bool], cfg: &SeamConfig, n: usize) -> Vec<usize> {
    let mean = boundary[1..].iter().sum::<f32>() / (n - 1) as f32;
    let var = boundary[1..].iter().map(|v| (v - mean).powi(2)).sum::<f32>() / (n - 1) as f32;
    let thr = mean + cfg.cut_threshold * var.sqrt();
    let mut mark = vec![false; n];
    for t in 1..n {
        let b = boundary[t];
        let local_max = b >= boundary[t - 1] && (t + 1 >= n || b >= boundary[t + 1]);
        // An entropy cut is taken unless it is inside a small recognized
        // lexer token, where it would only shred a real short unit.
        if b >= thr && b > 0.0 && local_max && !protect.get(t).copied().unwrap_or(false) {
            mark[t] = true;
        }
    }
    // Lexer token starts are forced cuts regardless (they are boundaries, not
    // interiors, so they are never protected).
    for &c in lexer_cuts {
        if c < n {
            mark[c] = true;
        }
    }
    let mut cuts = vec![0usize];
    let mut last = 0usize;
    for (t, &m) in mark.iter().enumerate().skip(1) {
        if m && t - last >= cfg.min_seg {
            cuts.push(t);
            last = t;
        }
    }
    cuts
}

/// Reinforce the seams at both ends of any provisional segment whose exact
/// bytes recur elsewhere in the input - confidence that the boundary
/// produces a real, repeated unit.
///
/// Recurrence is exact-byte. Recurrence up to the E8 Weyl orbit measures a
/// tie against the single-pass default on despaced prose (Moby Dick F1 0.553
/// against 0.553 at 8K, 0.723 against 0.723 at 128K) and costs an orbit
/// canonicalization per span per pass; it gains only against multi-pass
/// reinforcement, whose own deficit is what it repairs.
fn reinforce_recurrence(boundary: &mut [f32], input: &[u8], cuts: &[usize]) {
    const RECUR_BONUS: f32 = 2.0;
    let mut spans: Vec<(usize, usize)> = cuts.windows(2).map(|w| (w[0], w[1])).collect();
    if let Some(&last) = cuts.last()
        && last < input.len()
    {
        spans.push((last, input.len()));
    }
    let mut freq: HashMap<&[u8], u32> = HashMap::new();
    for &(s, e) in &spans {
        *freq.entry(&input[s..e]).or_insert(0) += 1;
    }
    for &(s, e) in &spans {
        if freq.get(&input[s..e] as &[u8]).copied().unwrap_or(0) > 1 {
            if s > 0 && s < boundary.len() {
                boundary[s] += RECUR_BONUS;
            }
            if e > 0 && e < boundary.len() {
                boundary[e] += RECUR_BONUS;
            }
        }
    }
}

/// The boundary-recovery score of a predicted cut set against the truth,
/// matching each predicted cut to the nearest unused truth within `tol`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Recovery {
    /// Predicted internal cut count.
    pub predicted: usize,
    /// True internal boundary count.
    pub truth: usize,
    /// Predicted cuts matched to a distinct true boundary.
    pub hit: usize,
    /// `hit / predicted`.
    pub precision: f32,
    /// `hit / truth`.
    pub recall: f32,
    /// Harmonic mean of precision and recall.
    pub f1: f32,
}

/// Score `predicted` cut offsets against `truth` boundary offsets within
/// `tol` bytes (greedy nearest, each truth used at most once).
#[must_use]
pub fn recovery(predicted: &[usize], truth: &[usize], tol: usize) -> Recovery {
    let mut used = vec![false; truth.len()];
    let mut hit = 0usize;
    for &p in predicted {
        let mut best: Option<usize> = None;
        for (i, &tr) in truth.iter().enumerate() {
            if used[i] || p.abs_diff(tr) > tol {
                continue;
            }
            if best.is_none_or(|bi| p.abs_diff(tr) < p.abs_diff(truth[bi])) {
                best = Some(i);
            }
        }
        if let Some(i) = best {
            used[i] = true;
            hit += 1;
        }
    }
    let precision = if predicted.is_empty() { 0.0 } else { hit as f32 / predicted.len() as f32 };
    let recall = if truth.is_empty() { 0.0 } else { hit as f32 / truth.len() as f32 };
    let f1 = if precision + recall > 0.0 { 2.0 * precision * recall / (precision + recall) } else { 0.0 };
    Recovery { predicted: predicted.len(), truth: truth.len(), hit, precision, recall, f1 }
}

/// A deterministic spaceless corpus: `n_words` drawn from `words` (fixed
/// LCG seeded by `seed`), concatenated with no separators. Returns the
/// bytes and the true internal word-boundary offsets.
#[must_use]
pub fn spaceless_corpus(words: &[&str], n_words: usize, seed: u64) -> (Vec<u8>, Vec<usize>) {
    let mut out: Vec<u8> = Vec::new();
    let mut bounds: Vec<usize> = Vec::new();
    let mut x = seed;
    for _ in 0..n_words {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let w = words[((x >> 33) as usize) % words.len().max(1)];
        out.extend_from_slice(w.as_bytes());
        bounds.push(out.len());
    }
    bounds.pop(); // the final offset is end-of-input, not an internal boundary
    (out, bounds)
}

/// A despaced corpus built from real text: every whitespace-separated word concatenated, with
/// the true word boundaries recorded.
///
/// The synthetic [`spaceless_corpus`] draws from a fixed 15-word vocabulary, which makes it a
/// test of a tiny closed alphabet rather than of language - every "word" recurs constantly, so
/// any recurrence rule looks good on it. Real prose has a heavy tail: most words are rare, many
/// occur once, and morphology means near-repeats that are not exact repeats. A segmentation
/// claim that does not survive that has not been tested.
///
/// `max_bytes` bounds the despaced output so a caller can pick a slice size.
#[must_use]
pub fn despaced_text(text: &[u8], max_bytes: usize) -> (Vec<u8>, Vec<usize>) {
    let mut out: Vec<u8> = Vec::new();
    let mut bounds: Vec<usize> = Vec::new();
    for w in text.split(|b| b.is_ascii_whitespace()).filter(|w| !w.is_empty()) {
        if out.len() + w.len() > max_bytes {
            break;
        }
        out.extend_from_slice(w);
        bounds.push(out.len());
    }
    bounds.pop(); // the final offset is end-of-input, not an internal boundary
    (out, bounds)
}

/// The internal piece boundaries a count-BPE model trained on `text`
/// produces for the whole (spaceless) input - the baseline seam competes
/// against at boundary recovery.
#[must_use]
pub fn bpe_boundaries(text: &[u8], num_merges: usize) -> Vec<usize> {
    let bpe = crate::bpe::Bpe::train(text, num_merges);
    let pieces = bpe.encode(text);
    let mut bounds = Vec::new();
    let mut off = 0usize;
    for (i, piece) in pieces.iter().enumerate() {
        off += piece.trim_end_matches("</w>").len();
        if i + 1 < pieces.len() {
            bounds.push(off);
        }
    }
    bounds
}

/// A trained character n-gram **shape** model - the phonotactic prior that
/// lets seam segment a short, single-occurrence string from the language's
/// shape (which clusters are word-internal-legal) rather than from the
/// input's own statistics. A boundary is where the cross-seam cluster is
/// improbable under the model: `touch|me` cuts because `chm` rarely occurs
/// inside an English word. This is not a lexicon - it never stores words,
/// only sub-word transition counts, and it generalizes to unseen words via
/// order backoff.
#[derive(Clone, Debug, Default)]
pub struct SeamModel {
    order: usize,
    fwd: HashMap<Vec<u8>, HashMap<u8, u32>>,
    bwd: HashMap<Vec<u8>, HashMap<u8, u32>>,
}

impl SeamModel {
    /// Train an order-`order` forward/backward character model from `corpus`
    /// (a body of the language, spaces and all - the model learns the shape
    /// around word boundaries as well as inside words).
    #[must_use]
    pub fn train(corpus: &[u8], order: usize) -> SeamModel {
        let k = order.max(1);
        let n = corpus.len();
        let mut fwd: HashMap<Vec<u8>, HashMap<u8, u32>> = HashMap::new();
        let mut bwd: HashMap<Vec<u8>, HashMap<u8, u32>> = HashMap::new();
        for t in 0..n.saturating_sub(1) {
            for o in 1..=k.min(t + 1) {
                *fwd.entry(corpus[t + 1 - o..=t].to_vec()).or_default().entry(corpus[t + 1]).or_insert(0) += 1;
            }
        }
        for t in 1..n {
            for o in 1..=k.min(n - t) {
                *bwd.entry(corpus[t..t + o].to_vec()).or_default().entry(corpus[t - 1]).or_insert(0) += 1;
            }
        }
        SeamModel { order: k, fwd, bwd }
    }

    /// Surprisal (bits) of byte `c` following context `ctx`. High = an
    /// improbable forward cluster (a boundary cue).
    fn fwd_surprisal(&self, ctx: &[u8], c: u8) -> f32 {
        model_surprisal(&self.fwd, ctx, c, self.order)
    }

    /// Surprisal (bits) of byte `c` preceding context `ctx`. High = an
    /// improbable backward cluster.
    fn bwd_surprisal(&self, ctx: &[u8], c: u8) -> f32 {
        model_surprisal(&self.bwd, ctx, c, self.order)
    }
}

/// Backed-off, add-alpha-smoothed surprisal of `c` given `ctx` under a trained
/// count model: take the longest context with observations, else shorten, else
/// fall back to uniform (8 bits). Backoff is what lets the model score a
/// cluster it never saw at full order but did at a shorter one.
fn model_surprisal(model: &HashMap<Vec<u8>, HashMap<u8, u32>>, ctx: &[u8], c: u8, order: usize) -> f32 {
    const ALPHA: f32 = 0.5;
    let hi = ctx.len().min(order);
    for o in (1..=hi).rev() {
        if let Some(d) = model.get(&ctx[ctx.len() - o..]) {
            let total: u32 = d.values().sum();
            if total > 0 {
                let cc = d.get(&c).copied().unwrap_or(0) as f32;
                let p = (cc + ALPHA) / (total as f32 + ALPHA * 256.0);
                return -p.log2();
            }
        }
    }
    8.0
}

/// Segment `input` using a background shape `model` (the phonotactic prior),
/// so even a short, single-occurrence string is cut where a cross-seam cluster
/// is improbable under the language's shape. Lexer boundaries are unioned in,
/// but small tokens are not protected here: the model is reliable on a short
/// input, so it is allowed to cut inside a token (`touchme` -> `touch|me`).
#[must_use]
pub fn analyze_with_model(input: &[u8], model: &SeamModel, cfg: &SeamConfig) -> SeamField {
    let n = input.len();
    let mut field = SeamField {
        len: n,
        fwd_entropy: vec![0.0; n],
        bwd_entropy: vec![0.0; n],
        boundary: vec![0.0; n],
        cuts: Vec::new(),
    };
    if n == 0 {
        return field;
    }
    if n == 1 {
        field.cuts.push(0);
        return field;
    }
    let k = model.order.max(1);
    for t in 0..n {
        field.fwd_entropy[t] = model.fwd_surprisal(&input[t.saturating_sub(k)..t], input[t]);
        let be = (t + 1 + k).min(n);
        field.bwd_entropy[t] = model.bwd_surprisal(&input[(t + 1).min(n)..be], input[t]);
    }
    for t in 1..n {
        field.boundary[t] = field.fwd_entropy[t] + field.bwd_entropy[t - 1];
    }
    let mut lexer_cuts: Vec<usize> = Vec::new();
    if cfg.lexer_fusion {
        for tok in crate::lexer::lex(input) {
            if tok.start() > 0 && tok.start() < n {
                lexer_cuts.push(tok.start());
            }
        }
    }
    field.cuts = derive_cuts(&field.boundary, &lexer_cuts, &[], cfg, n);
    field
}

/// A compact body of common English, the default phonotactic shape corpus.
const ENGLISH_CORPUS: &[u8] = b"the quick brown fox jumps over the lazy dog. please touch me gently and tell me what you see when the calm river flows past the old stone bridge. hello there my friend, the world is wide and the deep blue sea is calm. a cat and a dog sat together in the warm sun while the birds sing softly in the tall green trees. people walk along the narrow streets and children play in the bright afternoon light. the wind moves gently through the open fields beyond the hills where the mountains rise against the far horizon. water falling over the rocks fills the air with a steady rhythm. flowers grow in bright colors among the grass and stones along the path that winds between the houses and gardens of the town. the shops open early and close late and the market holds fresh fruit and bread and cheese that people need each day to live and work and rest together in the place they call home.";

/// The default English shape model (order 3), trained from `ENGLISH_CORPUS`.
#[must_use]
pub fn english_model() -> SeamModel {
    SeamModel::train(ENGLISH_CORPUS, 3)
}

// --- Hierarchical grain separation (nested-scale segmentation) ---
//
// A binary instruction stream nests two grains: blocks (ending in a
// terminator) inside functions (each opening with a prologue). Every function
// start is also a block start, so one global threshold cannot separate them -
// it fires at every block edge (recall 1.0 on functions, far too many cuts) or
// collapses to nothing. Two techniques here do separate them, each on a
// different axis: multi-scale persistence (a function boundary survives heavy
// smoothing, a block edge washes out) and prologue-orbit recognition (the
// function prologue is a recurring rename-invariant marker, recovered directly).

/// Number of coarse instruction classes (the 13-class encoding).
pub const N_CLASSES: usize = 13;

/// The canonical function prologue, as class ids (push, mov, sub).
pub const PROLOGUE: [u8; 3] = [0, 1, 2];

/// A synthetic instruction-class stream with known structure: functions (each
/// opening with [`PROLOGUE`] and carrying a coherent class-mix style) contain
/// blocks (each ending in a terminator). `func_starts` is a subset of
/// `block_starts`.
#[derive(Clone, Debug, Default)]
pub struct Program {
    /// The instruction-class stream.
    pub classes: Vec<u8>,
    /// Offsets of function starts (prologue positions).
    pub func_starts: Vec<usize>,
    /// Offsets of block starts (every block, including each function's first).
    pub block_starts: Vec<usize>,
}

/// Generate a deterministic synthetic program of `n_funcs` functions. Each
/// function carries a per-function style (a class-mix bias), so distinct
/// functions have distinct coarse texture - the property multi-scale
/// separation relies on; blocks inside a function share its style.
#[must_use]
pub fn synth_program(n_funcs: usize, seed: u64) -> Program {
    let mut x = seed;
    let mut rng = || {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (x >> 33) as u32
    };
    let mut classes: Vec<u8> = Vec::new();
    let mut func_starts: Vec<usize> = Vec::new();
    let mut block_starts: Vec<usize> = Vec::new();
    for _ in 0..n_funcs {
        func_starts.push(classes.len());
        block_starts.push(classes.len());
        classes.extend_from_slice(&PROLOGUE);
        let style = rng() % 3;
        let n_blocks = 2 + (rng() % 4) as usize;
        for b in 0..n_blocks {
            if b > 0 {
                block_starts.push(classes.len());
            }
            let body = 3 + (rng() % 6) as usize;
            for _ in 0..body {
                let c = match style {
                    0 => 3 + (rng() % 2) as u8, // arithmetic: add / cmp
                    1 => 9 + (rng() % 3) as u8, // memory: load / store / lea
                    _ => 5 + (rng() % 2) as u8, // control-mix
                };
                classes.push(c);
            }
            let term = if b + 1 == n_blocks { 8 } else { 5 + (rng() % 2) as u8 };
            classes.push(term);
        }
    }
    Program { classes, func_starts, block_starts }
}

/// Per-position boundary **persistence**: the coarsest timescale index (1-based
/// into `taus`, `0` = none) at which a bidirectional class-distribution
/// change-point fires there. High = the boundary survives smoothing, the coarse
/// (function) grain; low = scale-local, the block grain.
#[must_use]
pub fn persistence(classes: &[u8], taus: &[f32], cp_k: f32) -> Vec<u8> {
    let n = classes.len();
    let mut pers = vec![0u8; n];
    if n < 2 {
        return pers;
    }
    for (si, &tau) in taus.iter().enumerate() {
        let a = (-1.0 / tau.max(1.0)).exp();
        let oma = 1.0 - a;
        let mut fwd = vec![[0f32; N_CLASSES]; n];
        let mut h = [0f32; N_CLASSES];
        let mut powa = 1.0f32;
        for t in 0..n {
            for v in &mut h {
                *v *= a;
            }
            h[classes[t] as usize] += oma;
            powa *= a;
            let corr = (1.0 - powa).max(1e-4);
            for (dst, &src) in fwd[t].iter_mut().zip(h.iter()) {
                *dst = src / corr;
            }
        }
        let mut sig = vec![0f32; n];
        let mut hb = [0f32; N_CLASSES];
        let mut powb = 1.0f32;
        for t in (0..n).rev() {
            for v in &mut hb {
                *v *= a;
            }
            hb[classes[t] as usize] += oma;
            powb *= a;
            let corr = (1.0 - powb).max(1e-4);
            if t >= 1 {
                let mut s = 0.0f32;
                for (c, &f) in fwd[t - 1].iter().enumerate() {
                    let d = f - hb[c] / corr;
                    s += d * d;
                }
                sig[t] = s.sqrt();
            }
        }
        let mean = sig[1..].iter().sum::<f32>() / (n - 1) as f32;
        let var = sig[1..].iter().map(|v| (v - mean).powi(2)).sum::<f32>() / (n - 1) as f32;
        let thr = mean + cp_k * var.sqrt();
        for t in 1..n {
            let local_max = sig[t] >= sig[t - 1] && (t + 1 >= n || sig[t] >= sig[t + 1]);
            if sig[t] >= thr && sig[t] > 0.0 && local_max {
                pers[t] = pers[t].max((si + 1) as u8);
            }
        }
    }
    pers
}

/// The offsets where the class stream matches the prologue orbit `prologue`.
/// In a class stream the registers are already abstracted, so the orbit is the
/// exact class pattern - the function grain recovered by symmetry, not by a
/// threshold.
#[must_use]
pub fn prologue_starts(classes: &[u8], prologue: &[u8]) -> Vec<usize> {
    if prologue.is_empty() || classes.len() < prologue.len() {
        return Vec::new();
    }
    (0..=classes.len() - prologue.len())
        .filter(|&t| &classes[t..t + prologue.len()] == prologue)
        .collect()
}

/// Positions whose persistence is at least `tier` (boundaries at or above a
/// given coarseness).
#[must_use]
pub fn boundaries_at_tier(pers: &[u8], tier: u8) -> Vec<usize> {
    (0..pers.len()).filter(|&t| pers[t] >= tier).collect()
}

/// The unified hierarchical grain: the two axes bootstrapping each other.
#[derive(Clone, Debug, Default)]
pub struct UnifiedGrain {
    /// Coarse-grain (function) starts - complete, via the discovered marker.
    pub func_starts: Vec<usize>,
    /// Fine-grain (block) starts - the function anchors plus the fine texture.
    pub block_starts: Vec<usize>,
    /// The coarse-grain marker discovered from the texture boundaries.
    pub marker: Vec<u8>,
}

/// The mode class-pattern of length `len` among the `coarse` boundary offsets.
/// Real coarse-grain starts share one pattern (the prologue) while texture
/// false-positives scatter, so the mode is the marker - discovered, not given.
fn discover_marker(classes: &[u8], coarse: &[usize], len: usize) -> Vec<u8> {
    if len == 0 {
        return Vec::new();
    }
    let mut counts: HashMap<Vec<u8>, u32> = HashMap::new();
    for &b in coarse {
        if b + len <= classes.len() {
            *counts.entry(classes[b..b + len].to_vec()).or_insert(0) += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, c)| *c).map(|(k, _)| k).unwrap_or_default()
}

/// The unified grain segmenter: multi-scale texture DISCOVERS the coarse-grain
/// symmetry marker (the mode pattern among the texture-detected coarse
/// boundaries), the marker COMPLETES the coarse grain (recovering the
/// same-texture-adjacent starts texture alone misses), and the fine texture
/// tier fills the inner grain. Texture is unsupervised but recall-limited;
/// symmetry is complete but needs a marker - here each supplies the other.
#[must_use]
pub fn unified_grain(classes: &[u8], taus: &[f32], cp_k: f32, marker_len: usize) -> UnifiedGrain {
    let pers = persistence(classes, taus, cp_k);
    let coarse = boundaries_at_tier(&pers, taus.len().max(1) as u8);
    let marker = discover_marker(classes, &coarse, marker_len);
    // Symmetry completes the coarse grain; fall back to the texture coarse
    // boundaries if no marker emerged.
    let func_starts = if marker.is_empty() { coarse } else { prologue_starts(classes, &marker) };
    // The inner grain: function anchors unioned with the fine-tier texture.
    let mut block_starts: Vec<usize> =
        func_starts.iter().copied().chain(boundaries_at_tier(&pers, 1)).collect();
    block_starts.sort_unstable();
    block_starts.dedup();
    UnifiedGrain { func_starts, block_starts, marker }
}

// --- Predictive compression (compression IS prediction) ---
//
// A symbol costs -log2 P(symbol | context) bits. BPE's implicit model is
// corpus-trained, single-scale, forward-only frequency, and ships a codebook
// the decoder needs. These functions code the stream with a corpus-free,
// adaptive, multi-scale (multi-order) predictor that ships no codebook - the
// decoder rebuilds the same online model - and optionally quotients the
// context by a symmetry orbit (case fold) so variants share statistics. The
// returned value is the code length in bits (the achievable arithmetic-coded
// size); bits/byte is that over the input length.
//
// Three measures live here in every build: `adaptive_byte_bits`,
// `ppm_byte_bits` and `ctw_bits`. The compressor built on them - the logistic
// mixer, the baked prior and the rest of `trex compress` - is the `coder`
// submodule, compiled with the `compress` feature.

/// Per-order context models for the byte coder: context -> (next-byte counts,
/// total seen).
type ByteModels = Vec<HashMap<Vec<u8>, (HashMap<u8, u32>, u32)>>;

/// The context key for the byte coder: the raw context, or the case-folded
/// context when `orbit_context` quotients `The` and `the` together.
fn context_key(ctx: &[u8], orbit_context: bool) -> Vec<u8> {
    if orbit_context {
        ctx.iter().map(u8::to_ascii_lowercase).collect()
    } else {
        ctx.to_vec()
    }
}

/// Adaptive multi-scale byte coder: at each position mix the next-byte
/// distributions from context orders `0..=max_order` (weighted by how much
/// each context has been seen), code the actual byte, then update every order
/// online. Corpus-free: the model starts empty and learns as it codes, so no
/// dictionary is transmitted. `max_order = 0` is the no-context (order-0)
/// baseline. Returns the total code length in bits.
#[must_use]
pub fn adaptive_byte_bits(input: &[u8], max_order: usize, orbit_context: bool) -> f64 {
    let mut models: ByteModels = (0..=max_order).map(|_| HashMap::new()).collect();
    let mut bits = 0.0f64;
    for t in 0..input.len() {
        let actual = input[t];
        let mut mix = [0.0f64; 256];
        let mut wsum = 0.0f64;
        let hi = max_order.min(t);
        for k in 0..=hi {
            let ctx = context_key(&input[t - k..t], orbit_context);
            if let Some((counts, total)) = models[k].get(&ctx)
                && *total > 0
            {
                let w = f64::from(*total) * (k as f64 + 1.0);
                let tf = f64::from(*total);
                for (&b, &c) in counts {
                    mix[b as usize] += w * (f64::from(c) / tf);
                }
                wsum += w;
            }
        }
        // Uniform floor (order -1) so an unseen byte still gets positive mass.
        for m in &mut mix {
            *m += 1.0 / 256.0;
        }
        wsum += 1.0;
        let p = (mix[actual as usize] / wsum).max(1e-12);
        bits += -p.log2();
        for k in 0..=hi {
            let ctx = context_key(&input[t - k..t], orbit_context);
            let e = models[k].entry(ctx).or_insert_with(|| (HashMap::new(), 0));
            *e.0.entry(actual).or_insert(0) += 1;
            e.1 += 1;
        }
    }
    bits
}

/// Adaptive PPM-C byte coder with full exclusions. Each byte is coded from the
/// highest available context order downward: a context that has seen the byte
/// codes it with `count / (total + distinct)`; otherwise the coder escapes
/// (`distinct / (total + distinct)`), excludes the symbols already seen, and
/// drops one order, finally bottoming out at a uniform over the not-yet-
/// excluded bytes. Every order updates online, so the coder is codebook-free
/// (the alphabet is the 256 bytes), and a high `max_order` captures the
/// long-range structure BPE otherwise packs into multi-character tokens.
/// `orbit_context` folds case into the context key so `The` and `the` pool.
#[must_use]
pub fn ppm_byte_bits(input: &[u8], max_order: usize, orbit_context: bool) -> f64 {
    let mut models: ByteModels = (0..=max_order).map(|_| HashMap::new()).collect();
    let mut bits = 0.0f64;
    for t in 0..input.len() {
        let actual = input[t];
        let hi = max_order.min(t);
        let mut excluded = [false; 256];
        let mut n_excluded = 0usize;
        let mut log_p = 0.0f64;
        let mut coded = false;
        for k in (0..=hi).rev() {
            let ctx = context_key(&input[t - k..t], orbit_context);
            if let Some((counts, _)) = models[k].get(&ctx) {
                let mut eff_total = 0u32;
                let mut eff_distinct = 0u32;
                for (&s, &c) in counts {
                    if !excluded[s as usize] {
                        eff_total += c;
                        eff_distinct += 1;
                    }
                }
                if eff_total > 0 {
                    let denom = f64::from(eff_total + eff_distinct);
                    let sym_c =
                        if excluded[actual as usize] { 0 } else { *counts.get(&actual).unwrap_or(&0) };
                    if sym_c > 0 {
                        log_p += (f64::from(sym_c) / denom).log2();
                        coded = true;
                        break;
                    }
                    log_p += (f64::from(eff_distinct) / denom).log2();
                    for &s in counts.keys() {
                        if !excluded[s as usize] {
                            excluded[s as usize] = true;
                            n_excluded += 1;
                        }
                    }
                }
            }
        }
        if !coded {
            let avail = (256 - n_excluded).max(1);
            log_p += (1.0 / avail as f64).log2();
        }
        bits += -log_p;
        for k in 0..=hi {
            let ctx = context_key(&input[t - k..t], orbit_context);
            let e = models[k].entry(ctx).or_insert_with(|| (HashMap::new(), 0));
            *e.0.entry(actual).or_insert(0) += 1;
            e.1 += 1;
        }
    }
    bits
}

/// Binary Context Tree Weighting (Willems/Shtarkov/Tjalkens): a Bayesian
/// mixture over every bounded-depth context tree, computed in closed form -
/// no training, so it extracts signal from sparse high-order contexts that a
/// gradient-trained mixer undertrains on. Each node holds Krichevsky-Trofimov
/// counts and the natural-log estimated (`log_pe`) and weighted (`log_pw`)
/// block probabilities. The next-bit conditional follows the context path only
/// (off-path subtrees cancel): `c1 = (kt1 + r*c1_child)/(1 + r)` with
/// `r = P_w(child0)*P_w(child1)/P_e`. Used standalone and as a mixer input.
pub struct Ctw {
    depth: usize,
    a: Vec<u32>,
    b: Vec<u32>,
    log_pe: Vec<f64>,
    log_pw: Vec<f64>,
    child: Vec<[u32; 2]>,
    hist: u64,
    path: Vec<u32>,
}

impl Ctw {
    /// A fresh context tree of the given context `depth` (root only).
    #[must_use]
    pub fn new(depth: usize) -> Ctw {
        Ctw {
            depth,
            a: vec![0],
            b: vec![0],
            log_pe: vec![0.0],
            log_pw: vec![0.0],
            child: vec![[u32::MAX, u32::MAX]],
            hist: 0,
            path: Vec::with_capacity(depth + 1),
        }
    }

    /// Descend the context path (history 0-padded at the start, creating nodes)
    /// and return `P(next bit = 1)`. Call once per bit, before [`Ctw::update`].
    pub fn predict(&mut self) -> f64 {
        self.path.clear();
        self.path.push(0);
        let mut node = 0u32;
        for d in 0..self.depth {
            let cb = ((self.hist >> d) & 1) as usize;
            let mut c = self.child[node as usize][cb];
            if c == u32::MAX {
                c = self.a.len() as u32;
                self.a.push(0);
                self.b.push(0);
                self.log_pe.push(0.0);
                self.log_pw.push(0.0);
                self.child.push([u32::MAX, u32::MAX]);
                self.child[node as usize][cb] = c;
            }
            node = c;
            self.path.push(node);
        }
        let leaf = self.path[self.path.len() - 1] as usize;
        let mut c1 = (f64::from(self.b[leaf]) + 0.5)
            / (f64::from(self.a[leaf]) + f64::from(self.b[leaf]) + 1.0);
        for di in (0..self.path.len() - 1).rev() {
            let nd = self.path[di] as usize;
            let kt1 = (f64::from(self.b[nd]) + 0.5)
                / (f64::from(self.a[nd]) + f64::from(self.b[nd]) + 1.0);
            let c0 = self.child[nd][0];
            let c1n = self.child[nd][1];
            let lpw0 = if c0 == u32::MAX { 0.0 } else { self.log_pw[c0 as usize] };
            let lpw1 = if c1n == u32::MAX { 0.0 } else { self.log_pw[c1n as usize] };
            let r = (lpw0 + lpw1 - self.log_pe[nd]).clamp(-700.0, 700.0).exp();
            c1 = (kt1 + r * c1) / (1.0 + r);
        }
        c1.clamp(1e-9, 1.0 - 1e-9)
    }

    /// Update the current path with the observed `bit` and shift the history.
    pub fn update(&mut self, bit: u8) {
        let ln2 = std::f64::consts::LN_2;
        for di in (0..self.path.len()).rev() {
            let nd = self.path[di] as usize;
            let tot = f64::from(self.a[nd]) + f64::from(self.b[nd]) + 1.0;
            let ktv = if bit == 1 {
                (f64::from(self.b[nd]) + 0.5) / tot
            } else {
                (f64::from(self.a[nd]) + 0.5) / tot
            };
            self.log_pe[nd] += ktv.ln();
            if bit == 1 {
                self.b[nd] += 1;
            } else {
                self.a[nd] += 1;
            }
            if di == self.path.len() - 1 {
                self.log_pw[nd] = self.log_pe[nd];
            } else {
                let c0 = self.child[nd][0];
                let c1n = self.child[nd][1];
                let lpw0 = if c0 == u32::MAX { 0.0 } else { self.log_pw[c0 as usize] };
                let lpw1 = if c1n == u32::MAX { 0.0 } else { self.log_pw[c1n as usize] };
                let x = self.log_pe[nd];
                let y = lpw0 + lpw1;
                let mx = x.max(y);
                self.log_pw[nd] = -ln2 + mx + ((x - mx).exp() + (y - mx).exp()).ln();
            }
        }
        self.hist = (self.hist << 1) | u64::from(bit);
    }
}

/// CTW code length in bits for `input` at the given context `depth`.
#[must_use]
pub fn ctw_bits(input: &[u8], depth: usize) -> f64 {
    let ln2 = std::f64::consts::LN_2;
    let mut ctw = Ctw::new(depth);
    let mut bits = 0.0f64;
    for &byte in input {
        for bit_idx in (0..8).rev() {
            let actual = (byte >> bit_idx) & 1;
            let p1 = ctw.predict();
            let p = if actual == 1 { p1 } else { 1.0 - p1 };
            bits += -p.ln() / ln2;
            ctw.update(actual);
        }
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_multiscale_beats_order0_and_under_raw() {
        let input = b"the cat sat on the mat and the cat ran to the cat and the mat sat";
        let o0 = adaptive_byte_bits(input, 0, false);
        let mo = adaptive_byte_bits(input, 4, false);
        assert!(mo < o0, "multi-scale {mo:.1} should beat order-0 {o0:.1}");
        assert!(mo < input.len() as f64 * 8.0, "multi-scale should be under raw 8 bpb");
    }

    #[test]
    fn ppm_high_order_beats_the_low_order_mix() {
        let input = b"the cat sat on the mat and the cat ran to the cat and the mat sat on the cat";
        let mix4 = adaptive_byte_bits(input, 4, false);
        let ppm8 = ppm_byte_bits(input, 8, false);
        assert!(ppm8 < mix4, "PPM order-8 {ppm8:.1} should beat the order-4 mix {mix4:.1}");
        assert!(ppm8 < input.len() as f64 * 8.0, "PPM should be under raw 8 bpb");
    }

    #[test]
    fn ctw_codes_under_order0() {
        // A longer, repetitive corpus so the depth-16 tree fills (a depth-D
        // tree is cold for its first ~D bits, so a 75-byte input under-tests it).
        let base = b"the cat sat on the mat and the cat ran to the cat and the mat sat. ";
        let mut input = Vec::new();
        for _ in 0..8 {
            input.extend_from_slice(base);
        }
        let ctw = ctw_bits(&input, 16);
        let o0 = adaptive_byte_bits(&input, 0, false);
        assert!(ctw < input.len() as f64 * 8.0, "ctw under raw: {ctw:.1}");
        assert!(ctw < o0, "ctw {ctw:.1} should beat order-0 {o0:.1}");
    }

    #[test]
    fn synth_grain_structure_is_nested() {
        let p = synth_program(20, 0x1234);
        assert!(p.func_starts.iter().all(|f| p.block_starts.contains(f)), "functions nest in blocks");
        assert!(p.block_starts.len() > p.func_starts.len(), "more blocks than functions");
        for &f in &p.func_starts {
            assert_eq!(&p.classes[f..f + 3], &PROLOGUE);
        }
    }

    #[test]
    fn prologue_orbit_recovers_functions_cleanly() {
        let p = synth_program(40, 0x9e37);
        let pro = prologue_starts(&p.classes, &PROLOGUE);
        let r = recovery(&pro, &p.func_starts, 0);
        assert!(r.f1 > 0.95, "prologue orbit should recover the function grain cleanly: {r:?}");
    }

    #[test]
    fn multiscale_separates_function_grain_from_flat_threshold() {
        let p = synth_program(40, 0x5eed);
        // Flat single-scale, low threshold: fires at every block edge (high
        // recall on functions, low precision).
        let flat = persistence(&p.classes, &[4.0], 0.3);
        let r_flat = recovery(&boundaries_at_tier(&flat, 1), &p.func_starts, 1);
        // Coarsest tier at a function-SIZED scale (~26 instructions): a
        // function boundary changes the class-mix style, a within-function
        // block edge does not, so the coarse tier is precise about functions.
        let taus = [4.0, 12.0, 32.0];
        let pers = persistence(&p.classes, &taus, 1.0);
        let r_func = recovery(&boundaries_at_tier(&pers, taus.len() as u8), &p.func_starts, 1);
        assert!(
            r_func.precision > r_flat.precision,
            "coarse-tier precision {:.3} should beat the flat threshold {:.3}",
            r_func.precision,
            r_flat.precision
        );
    }

    #[test]
    fn unified_grain_completes_the_coarse_grain_unsupervised() {
        let p = synth_program(40, 0x5eed);
        let taus = [4.0, 12.0, 32.0];
        // Multi-scale texture alone is recall-limited on the coarse grain.
        let tex = recovery(
            &boundaries_at_tier(&persistence(&p.classes, &taus, 1.0), taus.len() as u8),
            &p.func_starts,
            1,
        );
        // The unified pass discovers the recurring coarse marker (the
        // inter-function transition) from those few texture boundaries and
        // matches it everywhere, completing the grain - same axes, no marker
        // known in advance. Boundary recovery within one instruction.
        let ug = unified_grain(&p.classes, &taus, 1.0, PROLOGUE.len());
        assert!(!ug.marker.is_empty(), "a coarse marker should be discovered");
        let uni = recovery(&ug.func_starts, &p.func_starts, 1);
        assert!(
            uni.recall > tex.recall,
            "unified recall {:.3} should beat texture-alone {:.3}",
            uni.recall,
            tex.recall
        );
        assert!(uni.f1 > 0.9, "unified function grain should be near-complete: {uni:?}");
    }

    #[test]
    fn shape_model_segments_single_occurrence_compounds() {
        // The phonotactic shape model cuts a spaceless compound it never saw
        // as a unit, with no dictionary: the boundary is where the cross-seam
        // cluster is improbable under English shape (chm in touch|me, nf in
        // brown|fox). This is the case plain self-statistics cannot solve.
        let model = english_model();
        let cfg = SeamConfig::default();
        for (text, boundary) in [(&b"touchme"[..], 5usize), (&b"brownfox"[..], 5)] {
            let f = analyze_with_model(text, &model, &cfg);
            assert!(
                f.internal_cuts().iter().any(|&c| c.abs_diff(boundary) <= 1),
                "{:?} should cut near {boundary}: cuts {:?}",
                std::str::from_utf8(text).unwrap(),
                f.cuts
            );
        }
    }

    /// A varied vocabulary whose word-internal trigrams are mostly
    /// word-specific (low entropy) while word ends branch over the whole
    /// vocabulary (high entropy).
    const VOCAB: &[&str] = &[
        "data", "stream", "model", "token", "signal", "past", "future", "read", "byte", "scale",
        "field", "probe", "cortex", "lattice", "vector",
    ];

    #[test]
    fn empty_and_singleton_are_safe() {
        assert!(analyze(b"").segments().is_empty());
        let one = analyze(b"x");
        assert_eq!(one.cuts, vec![0]);
        assert_eq!(one.segments(), vec![(0, 1)]);
    }

    #[test]
    fn word_ends_branch_more_than_word_interiors() {
        // On spaceless concatenation, the seam strength at true word
        // boundaries should exceed the average elsewhere.
        let (bytes, truth) = spaceless_corpus(VOCAB, 400, 0x1234_5678);
        let field = analyze(&bytes);
        let at_boundary: f32 =
            truth.iter().map(|&t| field.boundary.get(t).copied().unwrap_or(0.0)).sum::<f32>()
                / truth.len() as f32;
        let overall: f32 =
            field.boundary[1..].iter().sum::<f32>() / (field.boundary.len() - 1) as f32;
        assert!(
            at_boundary > overall,
            "boundary seams ({at_boundary:.3}) should branch more than average ({overall:.3})"
        );
    }

    /// The real-text corpus builder: words concatenated, boundaries exact, size respected.
    ///
    /// This exists because measuring segmentation on the synthetic 15-word vocabulary is a test
    /// of a closed alphabet, not of language - it saturates at F1 0.999, where nothing can be
    /// compared. The same seam reads F1 0.665 on despaced Moby Dick, which is where a change is
    /// actually visible.
    #[test]
    fn despaced_text_concatenates_words_and_records_their_boundaries() {
        let (bytes, truth) = despaced_text(b"the quick  brown\nfox", usize::MAX);
        assert_eq!(bytes, b"thequickbrownfox", "runs of any whitespace collapse away");
        assert_eq!(truth, vec![3, 8, 13], "an internal boundary is after each word but the last");
        for &t in &truth {
            assert!(t > 0 && t < bytes.len(), "a boundary is internal, never an endpoint");
        }
        // The cap bounds the output and never splits a word across it.
        let (short, _) = despaced_text(b"the quick brown fox", 8);
        assert_eq!(short, b"thequick", "the cap stops on a word boundary");
        assert!(despaced_text(b"", usize::MAX).0.is_empty(), "empty input is safe");
    }

    #[test]
    fn recovers_word_boundaries_better_than_bpe() {
        let (bytes, truth) = spaceless_corpus(VOCAB, 400, 0x9e37_79b9);
        let seam = analyze(&bytes);
        let rec_seam = recovery(&seam.internal_cuts(), &truth, 1);
        let bpe_cuts = bpe_boundaries(&bytes, 200);
        let rec_bpe = recovery(&bpe_cuts, &truth, 1);
        assert!(
            rec_seam.f1 > rec_bpe.f1,
            "seam F1 {:.3} should beat BPE F1 {:.3}",
            rec_seam.f1,
            rec_bpe.f1
        );
        assert!(rec_seam.f1 >= 0.4, "seam F1 {:.3} should be non-trivial", rec_seam.f1);
    }

    #[test]
    fn entropy_count_identity_matches_probability_form() {
        // The table-served count identity H = log2(t) - sum(c*log2 c)/t must agree with the
        // direct p*log2(p) form within f32 association noise across representative and
        // adversarial count shapes (uniform, skewed, single-outcome, past-table counts).
        let p_form = |counts: &HashMap<u8, u32>| -> f32 {
            let total: u32 = counts.values().sum();
            if total == 0 {
                return 0.0;
            }
            let t = total as f32;
            let h: f32 = -counts
                .values()
                .map(|&c| {
                    let p = c as f32 / t;
                    if p > 0.0 { p * p.log2() } else { 0.0 }
                })
                .sum::<f32>();
            h.max(0.0)
        };
        let cases: Vec<Vec<(u8, u32)>> = vec![
            vec![(1, 1)],
            vec![(1, 32), (2, 32)],
            vec![(0, 1), (1, 2), (2, 4), (3, 8), (4, 16), (5, 33)],
            (0..=255u8).map(|b| (b, 1 + u32::from(b) % 7)).collect(),
            vec![(9, 5000), (10, 4096), (11, 3)], // counts past the table -> computed fallback
        ];
        for case in cases {
            let counts: HashMap<u8, u32> = case.iter().copied().collect();
            // The reading under test is the one the segmenter runs, so the
            // followers are built the way it builds them - by bumping - rather
            // than by handing the table a finished distribution.
            let lut = crate::spectral::fixed_log_table();
            let mut f = Followers::default();
            for &(sym, n) in &case {
                for _ in 0..n {
                    f.bump(u32::from(sym));
                }
            }
            let a = f.entropy(lut);
            let b = p_form(&counts);
            assert!((a - b).abs() < 1e-4, "count-identity {a} vs p-form {b} for {counts:?}");
            assert_eq!(f.total, counts.values().sum::<u32>(), "the running total is the count");

            // Real input interleaves its followers rather than delivering each
            // one's occurrences consecutively, and the sum has to read the same
            // either way: a context's entropy is a function of its counts, not
            // of the order they arrived in. Bit equality is the right bar here
            // rather than a tolerance, because the terms are summed in exact
            // integer arithmetic over the same counts.
            let mut g = Followers::default();
            let mut left = case.clone();
            while left.iter().any(|&(_, n)| n > 0) {
                for (sym, n) in &mut left {
                    if *n > 0 {
                        *n -= 1;
                        g.bump(u32::from(*sym));
                    }
                }
            }
            assert_eq!(
                g.entropy(lut).to_bits(),
                a.to_bits(),
                "the sum must not depend on follower arrival order, for {counts:?}"
            );
        }
    }

    #[test]
    fn the_widest_direct_table_costs_what_its_threshold_says() {
        // `DIRECT_TABLE_MAX_KEY_BITS` is chosen against what the table costs to
        // zero per call, and that cost is quoted where the threshold is set.
        // Any field on `Followers` moves it, in either direction, so the figure
        // is pinned here rather than left to be believed.
        let entries = 1usize << DIRECT_TABLE_MAX_KEY_BITS;
        let bytes = entries * std::mem::size_of::<Followers>();
        assert_eq!(entries, 65536, "the widest direct table is 65536 followers");
        assert_eq!(
            bytes, 2_097_152,
            "the widest direct table reads as {bytes} bytes, so the size quoted on \
             DIRECT_TABLE_MAX_KEY_BITS is no longer what it costs"
        );
    }
}
