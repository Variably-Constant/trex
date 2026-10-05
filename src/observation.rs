//! The observation axis - TREX's vantage substrate.
//!
//! Every reader has a vantage in time. `spectral` reads the stream from one
//! fixed vantage - a single causal forward pass, where the state at `t` is the
//! decayed past. `seam` reads bidirectionally. Neither makes the vantage itself
//! a parameter, nor measures how much the reading DEPENDS on it. The observation
//! axis does both: it reads the same temporal signal (local byte-class entropy)
//! from the causal (past-only), anti-causal (future-only), and centered windows,
//! and the disagreement between the past and future readings is the
//! observer-dependence - how much the future disambiguates the past.
//!
//! That disagreement is exactly the bleed / transition / garden-path signal. A
//! causal reader is committed to the past and cannot yet see a change a centered
//! reader would; where the two vantages disagree most is where observation
//! MATTERS - the boundary the causal vantage smears forward. The observation
//! axis LOCATES those contested points directly. Documented in
//! `wiki/content/docs/reference/axes/observation.md`.

/// The window each vantage reads over (bytes).
const WINDOW: usize = 32;

/// Knobs for the observation reader.
#[derive(Clone, Copy, Debug)]
pub struct ObservationConfig {
    /// Bytes each vantage's window spans.
    pub window: usize,
    /// A contested point's disagreement must reach this (normalized `[0,1]`).
    pub contested_threshold: f32,
    /// Minimum bytes between two contested points at the byte grain; of two
    /// closer maxima the stronger is kept.
    pub contested_min_gap: usize,
}

impl Default for ObservationConfig {
    fn default() -> Self {
        Self {
            window: WINDOW,
            contested_threshold: 0.2,
            contested_min_gap: 8,
        }
    }
}

/// One position's reading across the three vantages.
#[derive(Clone, Copy, Debug, Default)]
pub struct ObservationFrame {
    /// Local byte-class entropy of the past window (the causal vantage).
    pub causal: f32,
    /// ... of the future window (the anti-causal vantage).
    pub anticausal: f32,
    /// ... of the symmetric window (the centered, offline vantage).
    pub centered: f32,
    /// `|causal - anticausal|`: the observer-dependence at this position.
    pub disagreement: f32,
}

/// The vantage side table, keyed by byte offset.
#[derive(Clone, Debug, Default)]
pub struct ObservationField {
    /// Input length in bytes.
    pub len: usize,
    /// One frame per byte.
    pub frames: Vec<ObservationFrame>,
    /// Contested byte offsets: local disagreement maxima past the threshold -
    /// where past and future disagree most (the points the causal vantage
    /// smears).
    pub contested: Vec<usize>,
}

impl ObservationField {
    /// The causal (past-only) reading at `byte`.
    #[must_use]
    pub fn causal_at(&self, byte: usize) -> f32 {
        self.frames.get(byte).map_or(0.0, |f| f.causal)
    }

    /// The anti-causal (future-only) reading at `byte`.
    #[must_use]
    pub fn anticausal_at(&self, byte: usize) -> f32 {
        self.frames.get(byte).map_or(0.0, |f| f.anticausal)
    }

    /// The centered (symmetric, offline) reading at `byte`.
    #[must_use]
    pub fn centered_at(&self, byte: usize) -> f32 {
        self.frames.get(byte).map_or(0.0, |f| f.centered)
    }

    /// The observer-dependence at `byte`.
    #[must_use]
    pub fn disagreement_at(&self, byte: usize) -> f32 {
        self.frames.get(byte).map_or(0.0, |f| f.disagreement)
    }

    /// Is `byte` a contested point (high observer-dependence)?
    #[must_use]
    pub fn is_contested(&self, byte: usize) -> bool {
        self.contested.binary_search(&byte).is_ok()
    }

    /// Is any contested point in the half-open byte range `[lo, hi)`? The
    /// `contested` list is sorted, so this is a binary-search range probe - the
    /// query a dispatch gate uses to ask "does this function overlap a code/data
    /// bleed zone?" before editing it.
    #[must_use]
    pub fn any_contested_in(&self, lo: usize, hi: usize) -> bool {
        if lo >= hi {
            return false;
        }
        let i = self.contested.partition_point(|&c| c < lo);
        self.contested.get(i).is_some_and(|&c| c < hi)
    }
}

/// The byte-class of an ASCII byte: digit / alpha / space / punct. The byte
/// reader reads its input as UTF-8 (`spectral::class_symbols`): a letter
/// outside ASCII is alpha every byte, any other character counts once by its
/// class, and a byte not part of a well-formed character is class 4.
fn class(b: u8) -> usize {
    if b.is_ascii_digit() {
        0
    } else if b.is_ascii_alphabetic() {
        1
    } else if b.is_ascii_whitespace() {
        2
    } else if b < 128 {
        3
    } else {
        4
    }
}

/// The number of byte classes, the byte reader's alphabet.
const N_CLASSES: usize = 5;

/// One vantage's window over a symbol stream, slid a symbol at a time.
///
/// Every vantage is read at every position, and each window is the previous
/// position's window with one symbol added and one dropped, so it keeps its
/// counts and its `sum c*log2(c)` and updates both per symbol rather than
/// recounting the window. The sum is held in fixed point (see
/// [`crate::spectral::FIXED_LOG_SCALE`]) so it is exact however far it slides.
struct Vantage<'a> {
    counts: Vec<u32>,
    /// Symbols in the window, counted or not.
    len: u32,
    /// Symbols in the window that are inside the alphabet. A symbol past
    /// the alphabet was not counted in sizing it, so it is skipped rather than
    /// folded onto a real one, and it still takes its share of the window.
    counted: u32,
    sum_clog: i64,
    lut: &'a [i64],
    /// `log2(len)` for every window length the reader can reach.
    log2_len: &'a [f64],
}

impl<'a> Vantage<'a> {
    fn new(alphabet: usize, lut: &'a [i64], log2_len: &'a [f64]) -> Self {
        Self { counts: vec![0; alphabet], len: 0, counted: 0, sum_clog: 0, lut, log2_len }
    }

    #[inline]
    fn add(&mut self, sym: u32) {
        self.len += 1;
        if let Some(c) = self.counts.get_mut(sym as usize) {
            let old = *c;
            *c = old + 1;
            self.counted += 1;
            self.sum_clog +=
                crate::spectral::fixed_term(self.lut, old + 1) - crate::spectral::fixed_term(self.lut, old);
        }
    }

    #[inline]
    fn remove(&mut self, sym: u32) {
        self.len -= 1;
        if let Some(c) = self.counts.get_mut(sym as usize) {
            let old = *c;
            *c = old - 1;
            self.counted -= 1;
            self.sum_clog -=
                crate::spectral::fixed_term(self.lut, old) - crate::spectral::fixed_term(self.lut, old - 1);
        }
    }

    /// Shannon entropy of the window normalized by `log2(alphabet)`, given
    /// as `norm`; zero for an empty window or a one-symbol alphabet.
    ///
    /// By the count identity, `-sum p*log2(p)` over `p = c / len` is
    /// `(counted / len) * log2(len) - sum_clog / len`; with every symbol
    /// counted the first factor is one.
    #[inline]
    fn entropy(&self, norm: f64) -> f32 {
        if self.len == 0 || norm == 0.0 {
            return 0.0;
        }
        let len = f64::from(self.len);
        let log2_len = self.log2_len.get(self.len as usize).copied().unwrap_or_else(|| len.log2());
        let h = f64::from(self.counted) / len * log2_len
            - self.sum_clog as f64 / (crate::spectral::FIXED_LOG_SCALE * len);
        (h / norm).max(0.0) as f32
    }
}

/// The three vantages at every position of a stream of `n` symbols drawn
/// from `alphabet`, read through `sym`.
///
/// The causal window is the `w` symbols up to and including the position,
/// the anticausal window the `w` after it, and the centered window the
/// `w / 2` on either side of it; each is clipped by the ends of the stream.
fn read_vantages(n: usize, alphabet: usize, w: usize, sym: impl Fn(usize) -> u32) -> Vec<ObservationFrame> {
    let mut frames = Vec::with_capacity(n);
    if n == 0 {
        return frames;
    }
    let half = (w / 2).max(1);
    let norm = if alphabet > 1 { (alphabet as f64).log2() } else { 0.0 };
    let lut = crate::spectral::fixed_log_table();
    let log2_len: Vec<f64> = (0..=(w + 2).min(n + 1)).map(|l| (l as f64).log2()).collect();
    let mut causal = Vantage::new(alphabet, lut, &log2_len);
    let mut anticausal = Vantage::new(alphabet, lut, &log2_len);
    let mut centered = Vantage::new(alphabet, lut, &log2_len);
    // The two forward-looking windows hold their first position's symbols
    // before the walk; the causal window takes its first symbol in the walk.
    for j in 1..(1 + w).min(n) {
        anticausal.add(sym(j));
    }
    for j in 0..(1 + half).min(n) {
        centered.add(sym(j));
    }
    for t in 0..n {
        causal.add(sym(t));
        if t > w {
            causal.remove(sym(t - w - 1));
        }
        let c = causal.entropy(norm);
        let a = anticausal.entropy(norm);
        frames.push(ObservationFrame {
            causal: c,
            anticausal: a,
            centered: centered.entropy(norm),
            disagreement: (c - a).abs(),
        });
        // Slide the two forward-looking windows on to the next position.
        if t + 1 < n {
            anticausal.remove(sym(t + 1));
            if t + 1 + w < n {
                anticausal.add(sym(t + 1 + w));
            }
            if t + 1 + half < n {
                centered.add(sym(t + 1 + half));
            }
            if t >= half {
                centered.remove(sym(t - half));
            }
        }
    }
    frames
}

/// Read the three vantages over a stream of symbols.
///
/// The reading is that a position is contested when what the past says about
/// it and what the future says about it disagree. That is a property of a
/// sequence, so it lifts to the token and supertoken streams by running over
/// their symbols. Positions are in symbol space; a caller maps them through
/// the spans the symbols came from.
///
/// The lift changes what a disagreement means, not how it is measured. Over
/// bytes it finds the point where a word's reading is ambiguous; over tokens
/// it finds the point where the structure read forward and the structure read
/// backward do not agree, which is the garden-path reading at the grain
/// structure actually lives at.
#[must_use]
pub fn analyze_symbols(
    symbols: &[u32],
    alphabet: usize,
    cfg: &ObservationConfig,
) -> ObservationField {
    let n = symbols.len();
    let mut field = ObservationField {
        len: n,
        frames: read_vantages(n, alphabet, cfg.window.max(1), |j| symbols[j]),
        contested: Vec::new(),
    };
    if n == 0 {
        return field;
    }
    for t in 1..n.saturating_sub(1) {
        let d = field.frames[t].disagreement;
        if d >= cfg.contested_threshold
            && d >= field.frames[t - 1].disagreement
            && d >= field.frames[t + 1].disagreement
        {
            field.contested.push(t);
        }
    }
    field
}

/// Read the observation axis over the token stream, by token kind.
#[must_use]
pub fn analyze_tokens(toks: &[crate::token::Token], cfg: &ObservationConfig) -> ObservationField {
    let symbols: Vec<u32> =
        toks.iter().filter(|t| t.is_significant()).map(|t| t.kind.code()).collect();
    let alphabet = symbols.iter().copied().max().map_or(1, |m| m as usize + 1);
    analyze_symbols(&symbols, alphabet, cfg)
}

/// Read the observation axis over the supertoken stream, by unit role.
#[must_use]
pub fn analyze_supertokens(
    units: &[crate::supertoken::SuperToken],
    cfg: &ObservationConfig,
) -> ObservationField {
    let symbols: Vec<u32> = units.iter().map(|u| u.role.code()).collect();
    let alphabet = symbols.iter().copied().max().map_or(1, |m| m as usize + 1);
    analyze_symbols(&symbols, alphabet, cfg)
}

/// Contested points over the token stream, as byte offsets.
///
/// A contested point here is where the structure read forward and the
/// structure read backward disagree, which is the garden-path reading at the
/// grain structure lives at rather than the grain characters do.
#[must_use]
pub fn contested_tokens(toks: &[crate::token::Token], cfg: &ObservationConfig) -> Vec<usize> {
    let sig: Vec<&crate::token::Token> = toks.iter().filter(|t| t.is_significant()).collect();
    let symbols: Vec<u32> = sig.iter().map(|t| t.kind.code()).collect();
    let alphabet = symbols.iter().copied().max().map_or(1, |m| m as usize + 1);
    let mut out: Vec<usize> = analyze_symbols(&symbols, alphabet, cfg)
        .contested
        .into_iter()
        .filter_map(|i| sig.get(i).map(|t| t.start()))
        .collect();
    out.sort_unstable();
    out
}

/// Contested points over the supertoken stream, as byte offsets.
#[must_use]
pub fn contested_supertokens(
    units: &[crate::supertoken::SuperToken],
    cfg: &ObservationConfig,
) -> Vec<usize> {
    let symbols: Vec<u32> = units.iter().map(|u| u.role.code()).collect();
    let alphabet = symbols.iter().copied().max().map_or(1, |m| m as usize + 1);
    let mut out: Vec<usize> = analyze_symbols(&symbols, alphabet, cfg)
        .contested
        .into_iter()
        .filter_map(|i| units.get(i).map(|u| u.start))
        .collect();
    out.sort_unstable();
    out
}

/// Analyze `input` with the default configuration.
#[must_use]
pub fn analyze(input: &[u8]) -> ObservationField {
    analyze_with(input, &ObservationConfig::default())
}

/// The full reader: the three vantages and their disagreement per byte, plus the
/// contested points.
#[must_use]
pub fn analyze_with(input: &[u8], cfg: &ObservationConfig) -> ObservationField {
    let n = input.len();
    let w = cfg.window.max(1);
    let half = (w / 2).max(1);
    let symbols = crate::spectral::class_symbols(input, class);
    let mut field = ObservationField {
        len: n,
        frames: read_vantages(n, N_CLASSES, w, |j| symbols[j]),
        contested: Vec::new(),
    };
    if n == 0 {
        return field;
    }

    // contested: local disagreement maxima at or above the threshold, spaced
    // apart. Skip the incomplete-window edges (where a vantage's window runs
    // off the data and disagreement is an artifact of the missing past /
    // future) when the input is long enough to have a clean interior.
    let margin = if n > 2 * w { half } else { 0 };
    let mut peaks: Vec<(usize, f32)> = Vec::new();
    for t in margin..n.saturating_sub(margin) {
        let d = field.frames[t].disagreement;
        if d < cfg.contested_threshold {
            continue;
        }
        let left = t.checked_sub(1).map_or(0.0, |j| field.frames[j].disagreement);
        let right = field.frames.get(t + 1).map_or(0.0, |f| f.disagreement);
        if d >= left && d >= right && (d > left || d > right) {
            peaks.push((t, d));
        }
    }
    // Of two maxima closer than the gap the stronger is kept, a tie keeping
    // the earlier, so the strongest point is always kept and a higher
    // threshold removes only points below it.
    peaks.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let gap = cfg.contested_min_gap;
    let mut kept = std::collections::BTreeSet::new();
    for (t, _) in peaks {
        let crowded = gap > 0 && kept.range(t.saturating_sub(gap - 1)..=t.saturating_add(gap - 1)).next().is_some();
        if !crowded {
            kept.insert(t);
        }
    }
    field.contested = kept.into_iter().collect();
    field
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diacritics_inside_words_contest_no_more_than_their_ascii_folding() {
        // Letters outside ASCII are letters, so a word's accents are not
        // foreign bytes to disagree over.
        let accented = "Příliš žluťoučký kůň úpěl ďábelské ódy, když večer přicházel a v údolí se rozsvěcela světla nad řekou.";
        let folded = "Prilis zlutoucky kun upel dabelske ody, kdyz vecer prichazel a v udoli se rozsvecela svetla nad rekou.";
        let (a, f) = (analyze(accented.as_bytes()).contested.len(), analyze(folded.as_bytes()).contested.len());
        assert!(a <= f + 1, "{a} contested in the accented line against {f} in its folding");
    }

    #[test]
    fn any_contested_in_range_probe() {
        // a field with contested points at 10, 50, 90.
        let f = ObservationField { len: 100, frames: Vec::new(), contested: vec![10, 50, 90] };
        assert!(f.any_contested_in(0, 20), "10 is in [0,20)");
        assert!(f.any_contested_in(40, 60), "50 is in [40,60)");
        assert!(!f.any_contested_in(20, 50), "[20,50) excludes 10 and the exclusive 50");
        assert!(!f.any_contested_in(91, 100), "[91,100) is past 90");
        assert!(!f.any_contested_in(30, 30), "an empty range never matches");
        assert!(f.any_contested_in(90, 91), "90 is in [90,91) (inclusive lo)");
    }

    #[test]
    fn transition_is_contested() {
        // Code then a high-entropy blob: at the boundary the causal (past=code)
        // and anti-causal (future=blob) vantages disagree - a contested point.
        let mut input = b"let x = 5; let y = 6; let z = 7; ".to_vec();
        input.extend_from_slice(b"f8KZ3pQ9wX7mB2nL5vR1tY6uA4cE0dG8hJ3kP9sW7xZ2");
        let f = analyze(&input);
        assert!(
            !f.contested.is_empty(),
            "the code -> blob transition should be contested"
        );
        // the contested point is near the boundary, not at the very start.
        assert!(f.contested.iter().any(|&c| c > 20));
    }

    #[test]
    fn homogeneous_stream_has_low_disagreement() {
        // Uniform prose: in the interior (complete windows) past and future
        // agree, so nothing is contested - the contrast with a transition. The
        // edges legitimately disagree (a causal observer has no past at byte 0).
        let f = analyze(
            b"the quick brown fox jumps over the lazy dog again and again now and forever more",
        );
        assert!(f.contested.is_empty(), "homogeneous text has no contested point");
        let n = f.frames.len();
        let interior = if n > 32 { &f.frames[16..n - 16] } else { &f.frames[..] };
        let maxd = interior.iter().map(|fr| fr.disagreement).fold(0.0f32, f32::max);
        assert!(maxd < 0.4, "homogeneous interior should barely disagree, got {maxd}");
    }

    #[test]
    fn causal_lags_the_centered_reading_at_a_change() {
        // At a sharp change the causal reading (past only) differs from the
        // centered one - the observer-dependence the bleed exploited.
        let mut input = vec![b'a'; 40];
        input.extend(std::iter::repeat_n(b'9', 40));
        let f = analyze(&input);
        let at = 40; // the boundary
        assert!(
            (f.causal_at(at) - f.frames[at].centered).abs() > 0.0,
            "causal and centered should differ at a change"
        );
    }

    #[test]
    fn empty_is_safe() {
        let f = analyze(b"");
        assert_eq!(f.len, 0);
        assert!(f.frames.is_empty());
        assert!(f.contested.is_empty());
        assert_eq!(f.causal_at(0), 0.0);
        assert!(!f.is_contested(0));
    }
}
