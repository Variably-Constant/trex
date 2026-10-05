//! The spectral axis: TREX's temporal substrate.
//!
//! The byte grain (`crate::lexer`) and the token grain (`crate::engine`)
//! are both *segmentation* scales - they answer "where are the
//! boundaries?" and differ only in how finely the stream is cut. This
//! module adds a third, perpendicular scale that answers a different
//! question: "what is the temporal character of the stream *at this
//! position*, independent of any cut?"
//!
//! A spectral frame is one vector per position, read four ways at once:
//!
//! - **texture / time** - a bank of leaky integrators
//!   `x[t] = a*x[t-1] + (1-a)*u[t]` over byte-class indicators. Each
//!   decay `a` is one pole of a low-pass filter (a frequency band) and,
//!   equivalently, a clock with time-constant `tau = -1/ln a` (recency).
//!   Texture and time are the same numbers read two ways.
//! - **information density** - a rolling Shannon entropy band that
//!   separates compressed / packed / random regions from plain text.
//! - **periodicity** - a bounded autocorrelation that finds the dominant
//!   byte-period of the neighborhood (CSV row width, fixed-width
//!   records, base64 phase) with no delimiter knowledge.
//! - **novelty** - a rolling k-gram surprise band that flags repeated
//!   template / boilerplate against genuinely new content.
//!
//! On top of the per-position frame is an online **change-point**
//! detector: it records the byte offsets where the temporal signature
//! jumps. Those boundaries are "BPE with time" - a dictionary-free,
//! one-pass, scale-free segmentation that cuts where the stream's own
//! dynamics change rather than where a learned merge table says.
//!
//! Everything is a single causal forward pass, so the reader composes
//! with streaming and carries an honest arrow of time (the state at
//! position `t` is exactly the decayed past available at `t`).
//!
//! Documented in `wiki/content/docs/reference/axes/spectral.md`.

use std::collections::{HashMap, VecDeque};

/// Number of byte classes in the filterbank one-hot input.
pub const N_CLASSES: usize = 5;
/// Number of decay clocks in the filterbank.
pub const N_DECAYS: usize = 5;
/// Filterbank dimension: one EMA per (decay, class).
pub const N_BANDS: usize = N_CLASSES * N_DECAYS;

/// Time-constants of the filterbank clocks, in bytes, pow2-spaced from
/// byte-local to block scale. `a_d = exp(-1/tau_d)`.
pub const TAUS: [f32; N_DECAYS] = [2.0, 8.0, 32.0, 128.0, 512.0];

/// The class a byte contributes to the filterbank's `u[t]` read on its own:
/// digit 0, letter or underscore 1, whitespace 2, other printable ASCII 3,
/// and 4 for a control byte or any byte at or above `0x80`.
///
/// Exactly one class per byte, so the one-hot input keeps each band a
/// bounded leaky average in `[0, 1]`. The filterbank reads its input as
/// UTF-8: a byte of a well-formed multi-byte character takes the class of
/// the character ([`char_class`]) rather than 4.
#[must_use]
pub fn classify(b: u8) -> usize {
    if b.is_ascii_digit() {
        0
    } else if b == b'_' || b.is_ascii_alphabetic() {
        1
    } else if b.is_ascii_whitespace() {
        2
    } else if b.is_ascii_graphic() {
        3
    } else {
        4
    }
}

/// The class every byte of a character outside ASCII contributes: a letter
/// or a combining mark 1, a numeral 0, whitespace 2, anything else 3.
#[must_use]
pub fn char_class(ch: char) -> usize {
    let combining = matches!(ch as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F);
    if ch.is_alphabetic() || combining {
        1
    } else if ch.is_numeric() {
        0
    } else if ch.is_whitespace() {
        2
    } else {
        3
    }
}

/// Each byte of `input` as a symbol over the classes, for a reader that
/// counts symbols rather than injecting them: an ASCII byte its class by
/// `ascii`, every byte of a letter outside ASCII the letter class, the first
/// byte of any other character outside ASCII its [`char_class`] and its other
/// bytes [`N_CLASSES`], a code past the alphabet the readers skip, so such a
/// character counts once; a byte not part of a well-formed character is
/// class 4.
pub(crate) fn class_symbols(input: &[u8], ascii: impl Fn(u8) -> usize) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::with_capacity(input.len());
    for chunk in input.utf8_chunks() {
        for ch in chunk.valid().chars() {
            let k = ch.len_utf8();
            if k == 1 {
                out.push(ascii(ch as u8) as u32);
                continue;
            }
            let c = char_class(ch) as u32;
            if c == 1 {
                out.extend(std::iter::repeat_n(1, k));
            } else {
                out.push(c);
                out.extend(std::iter::repeat_n(N_CLASSES as u32, k - 1));
            }
        }
        out.extend(std::iter::repeat_n(4, chunk.invalid().len()));
    }
    out
}

/// `count` bytes of one class, the newest of which arrived `age` bytes
/// before the current byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Release {
    class: usize,
    count: usize,
    age: usize,
}

/// A byte stream's classes read as UTF-8 and released in arrival order: the
/// bytes of a multi-byte character are held until it completes and then take
/// its class together, and bytes that cannot complete a well-formed
/// character are class 4. Nothing is released before it has arrived.
#[derive(Clone, Copy, Debug, Default)]
struct Utf8Classes {
    held: [u8; 4],
    len: u8,
    need: u8,
}

impl Utf8Classes {
    /// Whether no character is part-way through.
    fn idle(&self) -> bool {
        self.len == 0
    }

    /// The held bytes, as class 4, aged from the byte before this one.
    fn abandon(&mut self) -> Option<Release> {
        let count = usize::from(self.len);
        self.len = 0;
        self.need = 0;
        (count > 0).then_some(Release { class: 4, count, age: 1 })
    }

    /// What byte `b` releases: first any held bytes it shows cannot complete
    /// a character, then itself or the character it completes.
    fn push(&mut self, b: u8) -> [Option<Release>; 2] {
        match b {
            0x80..=0xBF if self.len > 0 => {
                self.held[usize::from(self.len)] = b;
                self.len += 1;
                if self.len < self.need {
                    return [None, None];
                }
                let count = usize::from(self.len);
                let class = std::str::from_utf8(&self.held[..count])
                    .ok()
                    .and_then(|s| s.chars().next())
                    .map_or(4, char_class);
                self.len = 0;
                self.need = 0;
                [Some(Release { class, count, age: 0 }), None]
            }
            0xC2..=0xF4 => {
                let abandoned = self.abandon();
                self.held[0] = b;
                self.len = 1;
                self.need = match b {
                    0xC2..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    _ => 4,
                };
                [abandoned, None]
            }
            _ => {
                let abandoned = self.abandon();
                let class = if b < 0x80 { classify(b) } else { 4 };
                [abandoned, Some(Release { class, count: 1, age: 0 })]
            }
        }
    }

    /// The held bytes of a character the input ends inside, as class 4, the
    /// newest being the last byte.
    fn finish(&mut self) -> Option<Release> {
        self.abandon().map(|r| Release { age: 0, ..r })
    }
}

/// A coarse texture class read from a frame's band mixture and entropy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Texture {
    /// Alphabetic-dominated, low punctuation - natural-language prose.
    Prose,
    /// High punctuation / symbol energy alongside identifiers - code.
    Code,
    /// Mixed single-character symbols and digits - mathematics.
    Math,
    /// High entropy for the share of multi-byte characters it holds, or many
    /// bytes not part of a well-formed character - compressed / packed / binary.
    Data,
    /// No class dominates.
    Mixed,
}

impl Texture {
    /// A short, stable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Texture::Prose => "prose",
            Texture::Code => "code",
            Texture::Math => "math",
            Texture::Data => "data",
            Texture::Mixed => "mixed",
        }
    }
}

/// One position's spectral reading.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpectralFrame {
    /// Filterbank EMA: `bands[d * N_CLASSES + c]` is decay `d` x class `c`,
    /// each in `[0, 1]` (fraction of recent bytes in class `c` at
    /// timescale `tau_d`).
    pub bands: [f32; N_BANDS],
    /// Rolling Shannon entropy of the local window, normalized to `[0, 1]`.
    pub entropy: f32,
    /// Dominant byte-period of the neighborhood (`0` = none detected).
    pub period: u16,
    /// Normalized autocorrelation peak height `[0, 1]`.
    pub period_strength: f32,
    /// k-gram surprise `[0, 1]` (`1` = first sighting in window).
    pub novelty: f32,
    /// The share of recent bytes inside well-formed multi-byte UTF-8
    /// characters, at the medium clock `[0, 1]`, which the texture readings'
    /// entropy gates rise with.
    pub(crate) wide: f32,
}

impl SpectralFrame {
    /// The class mixture at the medium clock (decay index 2), the band
    /// most representative of word / line-scale texture.
    #[must_use]
    pub fn texture_mix(&self) -> [f32; N_CLASSES] {
        let base = 2 * N_CLASSES;
        let mut out = [0.0f32; N_CLASSES];
        out.copy_from_slice(&self.bands[base..base + N_CLASSES]);
        out
    }
}

/// Knobs for the reader. `Default` is tuned for general text + binary.
#[derive(Clone, Copy, Debug)]
pub struct SpectralConfig {
    /// Frame sampling stride: `frames[j]` summarizes bytes up to
    /// `(j + 1) * hop - 1`.
    pub hop: usize,
    /// Rolling entropy window, in bytes.
    pub entropy_window: usize,
    /// Autocorrelation window, in bytes.
    pub period_window: usize,
    /// Largest lag (period) the autocorrelation searches.
    pub max_lag: usize,
    /// Baseline stride between autocorrelation evaluations (raised
    /// adaptively for large inputs to bound total periodicity work).
    pub period_hop: usize,
    /// k-gram size for the novelty band.
    pub ngram: usize,
    /// Sliding window for novelty, in k-grams.
    pub novelty_window: usize,
    /// Change-point sensitivity: a boundary needs `dist > mean + k*mad`.
    pub cp_threshold: f32,
    /// How close to the mean the change-point threshold may be, as a fraction
    /// of the mean. The spread is the reader's own estimate of how noisy the
    /// divergence is, and a divergence that is constant rather than noisy
    /// drives it to zero, which puts the threshold on the signal. A floor is a
    /// share of the mean, so it holds at any scale and carries no constant
    /// tied to the data. At zero the threshold is `mean + k*mad` whatever the
    /// spread.
    pub cp_floor: f32,
    /// Minimum bytes between two recorded change-points.
    pub cp_min_gap: usize,
}

impl Default for SpectralConfig {
    fn default() -> Self {
        Self {
            hop: 16,
            entropy_window: 64,
            period_window: 512,
            max_lag: 128,
            period_hop: 64,
            ngram: 4,
            novelty_window: 4096,
            cp_threshold: 4.0,
            cp_floor: 0.05,
            cp_min_gap: 4,
        }
    }
}

/// The spectral side table: a byte-offset-keyed reading of the whole
/// input. Orthogonal to the token stream - bytes, subtokens, and tokens
/// all query it the same way via [`SpectralField::signature`].
#[derive(Clone, Debug, Default)]
pub struct SpectralField {
    /// Input byte length.
    pub len: usize,
    /// Frame sampling stride (see [`SpectralConfig::hop`]).
    pub hop: usize,
    /// Downsampled frames; `frames[j]` summarizes bytes up to
    /// `min((j + 1) * hop - 1, len - 1)`.
    pub frames: Vec<SpectralFrame>,
    /// Change-point byte offsets, ascending (the spectral subtoken cuts).
    pub boundaries: Vec<usize>,
    /// Which readings this field carries. One it does not carry holds the zero
    /// its frame started at, which is indistinguishable from a genuine zero,
    /// so a reader asserts against this rather than trusting that the field it
    /// was handed is a whole one.
    pub needs: Needs,
}

impl Default for Needs {
    /// Every reading, so a field built from a default carries all of them.
    fn default() -> Needs {
        Needs::all()
    }
}

impl SpectralField {
    /// Panic in a debug build where this field lacks a reading the caller is
    /// about to take.
    ///
    /// A field answers a reading it does not carry with zero, and zero is a
    /// value every reading can genuinely have, so nothing downstream can tell
    /// the two apart. This is what makes that a test failure rather than a
    /// wrong answer.
    pub fn assert_carries(&self, want: Needs, who: &str) {
        debug_assert!(
            !(want.entropy && !self.needs.entropy)
                && !(want.period && !self.needs.period)
                && !(want.bands && !self.needs.bands)
                && !(want.onset && !self.needs.onset)
                && !(want.novelty && !self.needs.novelty),
            "{who} reads {want:?} from a field built for {:?}",
            self.needs
        );
    }

    /// The frame index covering byte offset `byte`.
    fn idx_of(&self, byte: usize) -> usize {
        if self.frames.is_empty() {
            return 0;
        }
        (byte / self.hop.max(1)).min(self.frames.len() - 1)
    }

    /// The frame whose window covers `byte` (nearest sampled frame).
    #[must_use]
    pub fn frame_at(&self, byte: usize) -> SpectralFrame {
        if self.frames.is_empty() {
            return SpectralFrame::default();
        }
        self.frames[self.idx_of(byte)]
    }

    /// The pooled spectral signature of the span `[start, end)`: the mean
    /// of the covered frames' bands / entropy / novelty, with the period
    /// taken from the strongest-periodicity frame in the span.
    #[must_use]
    pub fn signature(&self, start: usize, end: usize) -> SpectralFrame {
        if self.frames.is_empty() || end <= start {
            return SpectralFrame::default();
        }
        let lo = self.idx_of(start);
        let hi = self.idx_of(end.saturating_sub(1));
        let mut acc = SpectralFrame::default();
        let mut n = 0.0f32;
        let mut best_strength = -1.0f32;
        for f in &self.frames[lo..=hi] {
            for (a, b) in acc.bands.iter_mut().zip(f.bands.iter()) {
                *a += *b;
            }
            acc.entropy += f.entropy;
            acc.novelty += f.novelty;
            acc.wide += f.wide;
            if f.period_strength > best_strength {
                best_strength = f.period_strength;
                acc.period = f.period;
                acc.period_strength = f.period_strength;
            }
            n += 1.0;
        }
        if n > 0.0 {
            for a in &mut acc.bands {
                *a /= n;
            }
            acc.entropy /= n;
            acc.novelty /= n;
            acc.wide /= n;
        }
        acc
    }

    /// Whether a change-point is within `tol` bytes of `byte`: the first
    /// change-point at or past `byte - tol`, found by binary search over the
    /// ascending offsets, is the only one that can.
    #[must_use]
    pub fn boundary_near(&self, byte: usize, tol: usize) -> bool {
        let i = self.boundaries.partition_point(|&b| b.saturating_add(tol) < byte);
        self.boundaries.get(i).is_some_and(|&b| b <= byte.saturating_add(tol))
    }

    /// Whether any change-point is inside the half-open span `[start, end)`:
    /// the first change-point at or past `start`, found by binary search over
    /// the ascending offsets, is the only one that can.
    #[must_use]
    pub fn boundary_in(&self, start: usize, end: usize) -> bool {
        let i = self.boundaries.partition_point(|&b| b < start);
        self.boundaries.get(i).is_some_and(|&b| b < end)
    }
}

/// Whether `f`'s entropy clears `gate` raised toward one by the frame's
/// share of bytes inside well-formed multi-byte characters, whose bytes
/// carry more entropy than the text they spell.
fn over_entropy_gate(f: &SpectralFrame, gate: f32) -> bool {
    f.entropy > gate + (1.0 - gate) * f.wide
}

/// The texture class of a frame, from its band mixture and entropy.
#[must_use]
pub fn texture_of(f: &SpectralFrame) -> Texture {
    let m = f.texture_mix();
    let (digit, alpha, _space, punct, high) = (m[0], m[1], m[2], m[3], m[4]);
    if over_entropy_gate(f, 0.85) || high > 0.30 {
        return Texture::Data;
    }
    let total = digit + alpha + punct + high + m[2] + 1e-6;
    let punct_frac = punct / total;
    let alpha_frac = alpha / total;
    let digit_frac = digit / total;
    if alpha_frac > 0.55 && punct_frac < 0.12 {
        Texture::Prose
    } else if punct_frac > 0.20 && alpha_frac > 0.12 {
        Texture::Code
    } else if punct_frac > 0.12 && digit_frac > 0.10 {
        Texture::Math
    } else {
        Texture::Mixed
    }
}

/// Code-tuned texture - a fork of [`texture_of`] for the construct/POS layer.
/// The generic classifier is built for the {prose, code, math, data} split and
/// reads alpha-heavy base64 as `Prose`; the code layer needs {code, blob,
/// prose, numeric}, where a packed / base64 / hex run is `Blob` (a lower 0.78
/// entropy gate, raised like the generic one by the share of multi-byte
/// characters) and a digit-dense span is `Numeric`.
/// This is the byte-region signal the grammar-free construct tagger folds into
/// its context-key so the same surface shape resolves differently inside a
/// comment / string / blob than it does in live code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CodeTexture {
    /// Punctuation-bearing live code.
    Code,
    /// Packed / base64 / hex / binary run (an opaque literal body).
    Blob,
    /// Alpha-dominant, low-punctuation prose - a comment or a natural-language
    /// string literal.
    Prose,
    /// Digit-dominant - a numeric literal run.
    Numeric,
    /// None of the above decisively.
    Mixed,
}

impl CodeTexture {
    /// A one-char context-key suffix for the decisively non-code textures, or
    /// `None` for `Code` / `Mixed` so the bulk of code keys stay byte-identical
    /// and the corpus does not re-key (only comment / string / blob / numeric
    /// regions earn a distinct key). `b` blob, `p` prose, `n` numeric.
    #[must_use]
    pub fn key_suffix(self) -> Option<char> {
        match self {
            CodeTexture::Code | CodeTexture::Mixed => None,
            CodeTexture::Blob => Some('b'),
            CodeTexture::Prose => Some('p'),
            CodeTexture::Numeric => Some('n'),
        }
    }
}

/// Classify a frame into a code-relevant texture (see [`CodeTexture`]).
#[must_use]
pub fn code_texture(f: &SpectralFrame) -> CodeTexture {
    let m = f.texture_mix();
    let (digit, alpha, _space, punct, high) = (m[0], m[1], m[2], m[3], m[4]);
    // Blob first: a packed / base64 / hex / binary run. The generic 0.85 gate
    // misses base64 (~0.83, alpha-heavy); the code gate is 0.78 plus the path
    // for bytes not part of a well-formed character, which is true binary.
    if over_entropy_gate(f, 0.78) || high > 0.30 {
        return CodeTexture::Blob;
    }
    let total = digit + alpha + punct + high + m[2] + 1e-6;
    let punct_frac = punct / total;
    let alpha_frac = alpha / total;
    let digit_frac = digit / total;
    if digit_frac > 0.45 && alpha_frac < 0.30 {
        CodeTexture::Numeric
    } else if alpha_frac > 0.60 && punct_frac < 0.08 {
        CodeTexture::Prose
    } else if punct_frac > 0.15 {
        CodeTexture::Code
    } else {
        CodeTexture::Mixed
    }
}

/// The `c * log2(c)` table, resolved once: the per-bin term of a windowed
/// entropy sum for every count up to the table's length, each entry computed
/// by the same formula [`log_term`] falls back to past it. Counts are bounded
/// by the entropy window (64 by default; the period windows are hundreds),
/// so the table covers the whole working domain. A caller inside a per-byte
/// loop takes this before the loop instead of running [`log_term`]'s lock
/// check on every lookup.
pub(crate) fn log_table() -> &'static [f64] {
    const N: usize = 4096;
    static LUT: std::sync::OnceLock<Vec<f64>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        (0..N as u32).map(|c| if c > 1 { f64::from(c) * f64::from(c).log2() } else { 0.0 }).collect()
    })
}

/// The scale of the fixed-point `c * log2(c)` table: one unit is `2^-32`.
///
/// A sliding entropy sum is four of these terms in and out per position, and
/// as floating-point adds they are one dependency chain - each add waits on
/// the last, so the position's cost is four add latencies whatever else the
/// core could overlap. As integers the two in-terms and the two out-terms
/// fold to one delta off the chain, the chain is a single integer add, and
/// the sum is exact at every position rather than drifting with the order the
/// terms happened to arrive in. The scale keeps sixteen bits of headroom
/// above the largest window sum the readers use (`4096 * 12 * 2^32 < 2^48`).
pub(crate) const FIXED_LOG_SCALE: f64 = 4_294_967_296.0;

/// `c * log2(c)` in units of [`FIXED_LOG_SCALE`], one table for every sliding
/// entropy reader.
pub(crate) fn fixed_log_table() -> &'static [i64] {
    const N: usize = 4096;
    static LUT: std::sync::OnceLock<Vec<i64>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| (0..N as u32).map(fixed_term_computed).collect())
}

/// `term(c + 1) - term(c)` in units of [`FIXED_LOG_SCALE`]: what a count
/// going from `c` to `c + 1` adds to a window's sum, one lookup where the
/// difference of two terms is two.
pub(crate) fn fixed_log_delta_table() -> &'static [i64] {
    const N: usize = 4096;
    static LUT: std::sync::OnceLock<Vec<i64>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| (0..N as u32).map(|c| fixed_term_computed(c + 1) - fixed_term_computed(c)).collect())
}

/// One fixed-point term from an already-resolved table, computed past its
/// end.
///
/// The table holds a few thousand entries, so it serves a count that a window
/// bounds by sliding, and not one that only ever rises. `observation`'s
/// vantages add and remove over a window of 32 and never leave it; `seam`'s
/// follower counts accumulate over the whole input, and over 7.34 MB of
/// source they reach about 48000, so 95.1% of such calls would be past the
/// end and compute a `log2`, nineteen million of them. A caller whose counts
/// are unbounded is asking this for a logarithm, whatever the table suggests.
#[inline]
pub(crate) fn fixed_term(lut: &[i64], c: u32) -> i64 {
    if let Some(&v) = lut.get(c as usize) {
        return v;
    }
    fixed_term_computed(c)
}

fn fixed_term_computed(c: u32) -> i64 {
    if c > 1 {
        let cf = f64::from(c);
        (cf * cf.log2() * FIXED_LOG_SCALE).round() as i64
    } else {
        0
    }
}

#[inline]
pub fn log_term(c: u32) -> f64 {
    let lut = log_table();
    if let Some(&v) = lut.get(c as usize) {
        return v;
    }
    if c > 1 {
        let cf = f64::from(c);
        cf * cf.log2()
    } else {
        0.0
    }
}

/// A sliding centered autocorrelation over a fixed-width window.
///
/// The frame loop advances the window by a fixed hop, so consecutive
/// evaluations share all but the hop: at the production settings a 512-byte
/// window moving 64 bytes repeats seven eighths of its work. This keeps the
/// raw cross-sums between calls and slides them, costing the hop rather than
/// the window.
///
/// The cross-sums are exact. A byte times a byte summed over a bounded window
/// is at most `255 * 255 * 512`, under `2^25`, so an `i32` holds it with
/// room to spare and adding the entering terms and subtracting the leaving
/// ones is exact arithmetic: no drift accumulates however long the stream
/// runs. A floating accumulator slid the same way would drift, which is what
/// makes the integer form the one worth having rather than merely the fast
/// one. The narrow lane is also what makes the slide vector work: sixteen
/// lags per 512-bit register.
///
/// The slide runs with the lag as the inner index. For one byte leaving or
/// entering, its products with every lag's partner are a contiguous run of
/// the input against a contiguous run of the sums, which is a vector
/// multiply-add; the other nesting made each lag a serial chain of scalar
/// products.
///
/// Centering does not block the slide once the sum is expanded. Writing
/// `sum (x[t+L] - m)(x[t] - m)` as `cross - m*(A + B) + (n - L)*m^2` leaves
/// only `cross` position-dependent; `A`, `B` and the mean come from prefix
/// sums in constant time, and `m` may move freely between windows.
struct PeriodScanner {
    win: usize,
    max_lag: usize,
    /// `cross[L - 2]` is `sum x[t] * x[t + L]` over the current window.
    cross: Vec<i32>,
    /// `prefix[i]` is the sum of the first `i` bytes; `prefix_sq` their squares.
    prefix: Vec<i64>,
    prefix_sq: Vec<i64>,
    /// Exclusive end of the window `cross` currently describes.
    at: Option<usize>,
}

impl PeriodScanner {
    fn new(input: &[u8], win: usize, max_lag: usize) -> Self {
        let mut prefix = Vec::with_capacity(input.len() + 1);
        let mut prefix_sq = Vec::with_capacity(input.len() + 1);
        let (mut s, mut q) = (0i64, 0i64);
        prefix.push(0);
        prefix_sq.push(0);
        for &b in input {
            let v = i64::from(b);
            s += v;
            q += v * v;
            prefix.push(s);
            prefix_sq.push(q);
        }
        let hi = max_lag.min(win / 2);
        PeriodScanner {
            win,
            max_lag,
            cross: vec![0; hi.saturating_sub(1)],
            prefix,
            prefix_sq,
            at: None,
        }
    }

    /// The lag range this scans, matching [`dominant_period`].
    fn hi(&self) -> usize {
        self.max_lag.min(self.win / 2)
    }

    /// Rebuild the cross-sums for the window ending at `end` from scratch.
    fn seed(&mut self, input: &[u8], end: usize) {
        let start = end - self.win;
        let hi = self.hi();
        self.cross.fill(0);
        // Every left index of the window, each against its partners at every
        // lag that stays inside the window.
        for t in start..end.saturating_sub(2) {
            let x = i32::from(input[t]);
            let last = hi.min(end - 1 - t);
            let partners = &input[t + 2..=t + last];
            for (c, &y) in self.cross.iter_mut().zip(partners) {
                *c += x * i32::from(y);
            }
        }
        self.at = Some(end);
    }

    /// Slide the cross-sums forward to the window ending at `end`.
    fn advance(&mut self, input: &[u8], end: usize) {
        let Some(prev) = self.at else {
            self.seed(input, end);
            return;
        };
        let step = end - prev;
        // Past a whole window there is nothing left to reuse, and the two
        // correction ranges would overlap; seeding is then both cheaper and
        // simpler than a special case.
        if step >= self.win {
            self.seed(input, end);
            return;
        }
        let old_start = prev - self.win;
        let new_start = end - self.win;
        let hi = self.hi();
        // Terms whose left index leaves the window: the leaving byte against
        // its partner at every lag, a contiguous run to its right.
        for t in old_start..new_start {
            let x = i32::from(input[t]);
            let partners = &input[t + 2..=t + hi];
            for (c, &y) in self.cross.iter_mut().zip(partners) {
                *c -= x * i32::from(y);
            }
        }
        // Terms whose right index enters at the far end: the entering byte
        // against its partner at every lag, a contiguous run to its left
        // read backward.
        for u in prev..end {
            let x = i32::from(input[u]);
            let partners = &input[u - hi..=u - 2];
            for (c, &y) in self.cross.iter_mut().zip(partners.iter().rev()) {
                *c += x * i32::from(y);
            }
        }
        self.at = Some(end);
    }

    /// The dominant period of the window ending at `end`, and its strength.
    fn eval(&mut self, input: &[u8], end: usize) -> (u16, f32) {
        if self.win < 4 || end < self.win {
            return (0, 0.0);
        }
        self.advance(input, end);
        let start = end - self.win;
        let n = self.win as f64;
        let total = (self.prefix[end] - self.prefix[start]) as f64;
        let m = total / n;
        let sumsq = (self.prefix_sq[end] - self.prefix_sq[start]) as f64;
        let denom = sumsq - n * m * m;
        if denom <= f64::EPSILON {
            return (0, 0.0);
        }
        let (mut best_lag, mut best) = (0usize, 0.0f64);
        for lag in 2..=self.hi() {
            // sum over t of (x[t+lag] - m)(x[t] - m), expanded so only the
            // cross term depends on where the window is.
            let a = (self.prefix[end - lag] - self.prefix[start]) as f64;
            let b = (self.prefix[end] - self.prefix[start + lag]) as f64;
            let len = (self.win - lag) as f64;
            let corr = f64::from(self.cross[lag - 2]) - m * (a + b) + len * m * m;
            let r = corr / denom;
            if r > best {
                best = r;
                best_lag = lag;
            }
        }
        let best = best as f32;
        if best < PERIOD_FLOOR { (0, best.max(0.0)) } else { (best_lag as u16, best) }
    }
}

/// Correlation a lag must reach to count as a period at all.
pub(crate) const PERIOD_FLOOR: f32 = 0.20;

/// Independent lane accumulators in the autocorrelation's inner loop.
///
/// Sixteen `f32` is one AVX-512 register and two AVX2 ones, so the wider
/// setting costs nothing where only AVX2 is available and uses the whole
/// register where 512 is. The count is a lane count rather than a tuning
/// constant: it is what the register holds.
const LANES: usize = 16;

/// The dominant period of a window by centered autocorrelation, and its
/// normalized strength. Returns `(0, _)` when no lag clears the floor.
pub fn dominant_period(win: &[u8], max_lag: usize) -> (u16, f32) {
    let n = win.len();
    if n < 4 {
        return (0, 0.0);
    }
    let mean = win.iter().map(|&b| f32::from(b)).sum::<f32>() / n as f32;
    // The window is centered once, so the autocorrelation below multiplies
    // precomputed deviations rather than converting and subtracting the mean
    // twice per (lag, t). Bit-exact: c[t] == f32(win[t]) - mean.
    let c: Vec<f32> = win.iter().map(|&b| f32::from(b) - mean).collect();
    let denom: f32 = c.iter().map(|&x| x * x).sum();
    if denom <= f32::EPSILON {
        return (0, 0.0);
    }
    let hi = max_lag.min(n / 2);
    let mut best_lag = 0usize;
    let mut best = 0.0f32;
    for lag in 2..=hi {
        // Autocorrelation at `lag` = dot(c[lag..], c[..n-lag]). Eight independent lane accumulators
        // break the serial add-chain so LLVM vectorizes the multiply-add (one AVX2 YMM step per
        // 8-wide chunk); a horizontal sum + scalar tail finish it. This reorders the f32 sum vs a
        // strictly serial accumulation - a ~1 ULP shift in the autocorrelation, immaterial to a
        // heuristic period detector whose output is the argmax over lags.
        let a = &c[lag..];
        let b = &c[..n - lag];
        let (a_ch, a_rest) = a.as_chunks::<LANES>();
        let (b_ch, b_rest) = b.as_chunks::<LANES>();
        let mut acc = [0.0f32; LANES];
        for (ca, cb) in a_ch.iter().zip(b_ch) {
            for ((acck, &x), &y) in acc.iter_mut().zip(ca).zip(cb) {
                *acck += x * y;
            }
        }
        let mut sum: f32 = acc.iter().sum();
        for (&x, &y) in a_rest.iter().zip(b_rest) {
            sum += x * y;
        }
        let r = sum / denom;
        if r > best {
            best = r;
            best_lag = lag;
        }
    }
    if best < PERIOD_FLOOR {
        (0, best.max(0.0))
    } else {
        (best_lag as u16, best)
    }
}

/// Which readings of the field a caller will take.
///
/// The pass is a step a byte, so a reading nobody asks for would cost a step
/// at every byte of the input. A `\F{entropy>0.7}` scan reads the entropy and nothing
/// else, and without this it would also decay a filterbank, hash and count a
/// k-gram through a map, and take a square root for a change-point, at each of
/// the input's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Needs {
    /// The rolling-entropy reading.
    pub entropy: bool,
    /// The dominant byte-period and its strength.
    pub period: bool,
    /// The class mix, which the texture is read off and the change-point
    /// measures its divergence over.
    pub bands: bool,
    /// The change-point boundaries.
    pub onset: bool,
    /// The k-gram surprise. No pattern atom reads it: [`SpectralPred`] names
    /// entropy, period, texture and onset, and nothing else reaches a frame's
    /// novelty. The `spectral` command reports it, and that is the whole of
    /// what asks for it.
    ///
    /// [`SpectralPred`]: crate::ast::SpectralPred
    pub novelty: bool,
}

impl Needs {
    /// Every reading, which is what the `spectral` command reports.
    #[must_use]
    pub fn all() -> Needs {
        Needs { entropy: true, period: true, bands: true, onset: true, novelty: true }
    }

    /// No reading at all.
    #[must_use]
    pub fn none() -> Needs {
        Needs { entropy: false, period: false, bands: false, onset: false, novelty: false }
    }

    /// The readings one atom takes, with what each rests on: a texture is read
    /// off the class mix and the entropy together, and a change-point measures
    /// how far the two have diverged, so naming either takes both.
    #[must_use]
    pub fn of(pred: &crate::ast::SpectralPred) -> Needs {
        use crate::ast::SpectralPred as P;
        let mut needs = Needs::none();
        match pred {
            P::EntropyGe(_) | P::EntropyLe(_) => needs.entropy = true,
            P::PeriodEq(_) | P::PeriodAny => needs.period = true,
            P::Texture(_) => {
                needs.entropy = true;
                needs.bands = true;
            }
            P::Onset => {
                needs.entropy = true;
                needs.bands = true;
                needs.onset = true;
            }
        }
        needs
    }

    /// Everything either takes, for a pattern naming several atoms.
    #[must_use]
    pub fn and(self, other: Needs) -> Needs {
        Needs {
            entropy: self.entropy || other.entropy,
            period: self.period || other.period,
            bands: self.bands || other.bands,
            onset: self.onset || other.onset,
            novelty: self.novelty || other.novelty,
        }
    }

    /// Whether any reading is asked for.
    #[must_use]
    pub fn any(self) -> bool {
        self != Needs::none()
    }
}

/// Analyze `input` with the default configuration, computing every reading.
#[must_use]
pub fn analyze(input: &[u8]) -> SpectralField {
    analyze_with(input, &SpectralConfig::default())
}

/// Analyze `input` into a [`SpectralField`] in one causal forward pass,
/// computing every reading.
#[must_use]
pub fn analyze_with(input: &[u8], cfg: &SpectralConfig) -> SpectralField {
    analyze_needing(input, cfg, Needs::all())
}

/// [`analyze_with`] computing only the readings `needs` names.
///
/// A reading left out is absent from every frame rather than wrong: it holds
/// the zero its frame was built with, so a caller that did not ask for one
/// must not read it. The guards are loop-invariant, so the branch a byte costs
/// what a predicted branch costs and saves the block it skips.
#[must_use]
pub fn analyze_needing(input: &[u8], cfg: &SpectralConfig, needs: Needs) -> SpectralField {
    let n = input.len();
    let hop = cfg.hop.max(1);
    if n == 0 {
        return SpectralField { len: 0, hop, frames: Vec::new(), boundaries: Vec::new(), needs };
    }

    // Filterbank decays: a_d = exp(-1/tau_d), and 1 - a_d (the input gain
    // that keeps each band a bounded leaky average).
    let mut a = [0.0f32; N_DECAYS];
    let mut oma = [0.0f32; N_DECAYS];
    for ((ad, od), &tau) in a.iter_mut().zip(oma.iter_mut()).zip(TAUS.iter()) {
        *ad = (-1.0 / tau).exp();
        *od = 1.0 - *ad;
    }
    let mut bands = [0.0f32; N_BANDS];
    // Each band's decay laid out as the bands are, so decaying the bank is
    // one multiply across the array - a lane per band - rather than a short
    // loop per clock the vectorizer cannot fill.
    let mut decay = [0.0f32; N_BANDS];
    for (d, &ad) in a.iter().enumerate() {
        decay[d * N_CLASSES..(d + 1) * N_CLASSES].fill(ad);
    }
    // `a_d^k` for the `k <= 4` bytes of one UTF-8 character: `count` bytes of
    // a class released together, the newest `age` bytes old, carry the
    // `a^age * (1 - a^count)` the same bytes would have accrued one at a time.
    let mut held_pow = [[1.0f32; 5]; N_DECAYS];
    for (row, &ad) in held_pow.iter_mut().zip(a.iter()) {
        for k in 1..row.len() {
            row[k] = row[k - 1] * ad;
        }
    }
    let mut utf8 = Utf8Classes::default();
    // The medium clock's share of bytes inside well-formed multi-byte
    // characters.
    let mut wide = 0.0f32;

    // Entropy: 256-bin sliding-window histogram with an incremental
    // `sum c*log2(c)`.
    let w = cfg.entropy_window.max(1);
    let mut hist = [0u32; 256];
    // The window's `sum c*log2(c)`, in fixed point (see `FIXED_LOG_SCALE`).
    let mut s = 0i64;
    let entropy_norm = (w.min(256) as f32).max(2.0).log2();

    // Novelty: counts of recent k-grams over a sliding window. The key is
    // already a mixed hash of the gram, so the table takes it as is instead
    // of hashing it again.
    let k = cfg.ngram.max(1);
    let nov_win = cfg.novelty_window.max(1);
    let mut ngram_counts: HashMap<u64, u32, std::hash::BuildHasherDefault<crate::tokutil::IdHash>> =
        HashMap::default();
    let mut ngram_ring: VecDeque<u64> = VecDeque::with_capacity(nov_win + 1);
    let mut cur_novelty = 0.0f32;

    // Periodicity: adaptive hop keeps total autocorrelation work bounded.
    let pwin = cfg.period_window.max(8);
    let max_lag = cfg.max_lag.clamp(2, pwin / 2);
    let per_eval = pwin.saturating_mul(max_lag).max(1);
    const PERIOD_OP_BUDGET: usize = 1_000_000_000;
    let max_evals = (PERIOD_OP_BUDGET / per_eval).max(1);
    let period_hop = (n / max_evals).max(cfg.period_hop).max(1);
    let mut period_evals = 0u64;
    let mut cur_period = 0u16;
    let mut cur_strength = 0.0f32;
    let mut period_scanner = PeriodScanner::new(input, pwin, max_lag);
    // Constants of the rolling-entropy window, hoisted out of the per-byte
    // loop; the reciprocal carries the fixed-point scale of the sum.
    let ent_lut = fixed_log_table();
    let ent_full_log2 = (w as f64).log2();
    let ent_full_recip = 1.0 / (w as f64 * FIXED_LOG_SCALE);

    // Change-point: how far the recent texture (fast clock) has diverged
    // from the established baseline (slow clock), with an adaptive EWMA
    // threshold and rising-edge hysteresis (one boundary per regime onset,
    // not a cluster while the slow clock catches up). The threshold's mean
    // and spread take each byte's deviation clipped at `CP_CLIP` spreads,
    // Huber's robust update, so a divergence rising over many bytes cannot
    // carry the threshold up ahead of itself. An onset also records the slow
    // clock's mix as the texture it left, and the regime ends where the fast
    // clock comes back to that texture: within `CP_RETURN_SHARE` of its mean
    // distance from it since the onset for `CP_RETURN_SUSTAIN` bytes in a
    // row, the boundary at the first of them.
    const CP_ALPHA: f32 = 0.02;
    const CP_CLIP: f32 = 3.0;
    // The rounding noise of a reading of order one in f32, about eight steps
    // of 2^-23: no spread the clip is computed from is taken as smaller.
    const CP_NOISE: f32 = 1e-6;
    const CP_WARMUP: usize = 32;
    const CP_RETURN_SHARE: f32 = 0.5;
    const CP_RETURN_SUSTAIN: u32 = 12;
    const ENT_SLOW_A: f32 = 0.992;
    const ENT_WEIGHT: f32 = 2.0;
    let mut ent_slow = 0.0f32;
    let mut pow_f = 1.0f32;
    let mut pow_s = 1.0f32;
    let mut pow_e = 1.0f32;
    // Whether every bias correction has reached exactly one, past which the
    // powers above are no longer read and the divisions they feed are skipped.
    let mut settled = false;
    let mut cp_mean = 0.0f32;
    let mut cp_mad = 0.0f32;
    let mut cp_armed = true;
    // The open regime: the mix its onset left, the summed distance of the
    // fast clock from it and the bytes summed, and how many bytes in a row
    // the distance has been under the return share of its mean.
    let mut regime: Option<([f32; N_CLASSES], f32, u32, u32)> = None;
    let mut last_boundary: isize = -(cfg.cp_min_gap as isize);
    let mut boundaries: Vec<usize> = Vec::new();

    let mut frames: Vec<SpectralFrame> = Vec::with_capacity(n / hop + 1);

    for i in 0..n {
        let b = input[i];

        // Filterbank: decay every band, then inject what this byte releases.
        // An ASCII byte outside a character releases itself; anything else
        // goes through the UTF-8 reader.
        if needs.bands {
            for (v, &dk) in bands.iter_mut().zip(decay.iter()) {
                *v *= dk;
            }
            wide *= a[2];
            if b < 0x80 && utf8.idle() {
                let c = classify(b);
                for (d, &od) in oma.iter().enumerate() {
                    bands[d * N_CLASSES + c] += od;
                }
            } else {
                let [first, second] = utf8.push(b);
                let last = if i + 1 == n { utf8.finish() } else { None };
                for r in [first, second, last].into_iter().flatten() {
                    if r.count > 1 && r.class != 1 && r.class != 4 {
                        // A mark: a character of more than one byte that is no
                        // letter, completing on this byte. Its first byte adds
                        // one unit of its class and its other bytes each add
                        // the current mix, which leaves the composition where that
                        // unit put it; the band's total grows by the `1 - a^k`
                        // any `k` bytes add.
                        let k = r.count;
                        for (d, p) in held_pow.iter().enumerate() {
                            let row = &mut bands[d * N_CLASSES..(d + 1) * N_CLASSES];
                            let target = row.iter().sum::<f32>() + (1.0 - p[k]);
                            let mut mix = [0.0f32; N_CLASSES];
                            for (c, v) in mix.iter_mut().enumerate() {
                                *v = row[c] / p[k - 1] + if c == r.class { 1.0 - p[1] } else { 0.0 };
                            }
                            let scale = target / mix.iter().sum::<f32>();
                            for (v, m) in row.iter_mut().zip(mix.iter()) {
                                *v = m * scale;
                            }
                        }
                    } else {
                        for (d, p) in held_pow.iter().enumerate() {
                            bands[d * N_CLASSES + r.class] += p[r.age] * (1.0 - p[r.count]);
                        }
                    }
                    if r.count > 1 && r.class != 4 {
                        let p = &held_pow[2];
                        wide += p[r.age] * (1.0 - p[r.count]);
                    }
                }
            }
        }

        // Entropy: add the new byte, evict the one leaving the window. The
        // window is full after `w` bytes and stays full, so its logarithm and
        // reciprocal are constants rather than a libm call per byte, and the
        // count table is resolved once above rather than per lookup.
        let cur_entropy = if needs.entropy {
            let nb = hist[b as usize];
            hist[b as usize] = nb + 1;
            let mut delta = fixed_term(ent_lut, nb + 1) - fixed_term(ent_lut, nb);
            if i >= w {
                let ob = input[i - w] as usize;
                let oc = hist[ob];
                hist[ob] = oc - 1;
                delta -= fixed_term(ent_lut, oc) - fixed_term(ent_lut, oc - 1);
            }
            s += delta;
            let (log_nwin, recip) = if i + 1 >= w {
                (ent_full_log2, ent_full_recip)
            } else {
                let nwin = f64::from((i + 1) as u32);
                (nwin.log2(), 1.0 / (nwin * FIXED_LOG_SCALE))
            };
            let hbits = (log_nwin - s as f64 * recip) as f32;
            (hbits / entropy_norm).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Novelty: surprise of the k-gram ending at this byte.
        if needs.novelty && i + 1 >= k {
            let mut h = 1_469_598_103_934_665_603u64;
            for &x in &input[i + 1 - k..=i] {
                h ^= u64::from(x);
                h = h.wrapping_mul(1_099_511_628_211);
            }
            // One lookup reads the count and bumps it.
            let count = ngram_counts.entry(h).or_insert(0);
            cur_novelty = 1.0 / (1.0 + *count as f32);
            *count += 1;
            ngram_ring.push_back(h);
            if ngram_ring.len() > nov_win
                && let Some(old) = ngram_ring.pop_front()
                && let Some(cc) = ngram_counts.get_mut(&old)
            {
                *cc -= 1;
                if *cc == 0 {
                    ngram_counts.remove(&old);
                }
            }
        }

        // Periodicity: recompute on the adaptive hop, hold between. The
        // scanner slides its cross-sums from the previous evaluation instead
        // of rebuilding them, so each one costs the hop rather than the window.
        if needs.period && i + 1 >= pwin && i % period_hop == 0 {
            // What the pass actually spends, carried in a register and handed
            // over once at the end. The cost of this block is the count of
            // these times what one costs, and a count is the same on a busy box
            // as on a quiet one where a clock is not: two identical
            // configurations of this scan timed 161.338 ms and 83.775 ms in one
            // run while another crate was building beside it.
            period_evals += 1;
            let (p, st) = period_scanner.eval(input, i + 1);
            cur_period = p;
            cur_strength = st;
        }

        // Change-point: divergence of the fast clock (tau=8, recent) from
        // the slow clock (tau=128, baseline) over the class mix, plus the
        // gap between the windowed entropy and its slow average. A regime
        // switch (prose -> packed data, code -> blob) drives both terms;
        // uniform text leaves them near zero.
        // Bias-correct each clock (x / (1 - a^(t+1))) so it reads its true
        // class fraction from byte 0 - no warmup transient inflating the
        // divergence while the slow clock is still filling.
        if needs.onset {
            // Each correction is `1 / (1 - a^(t+1))` over a decay below one, so
            // the power it subtracts shrinks geometrically until it falls under
            // the last bit of an f32 beside one. From that byte on the
            // subtraction is exactly one, the correction is exactly one, and
            // multiplying or dividing by it returns its operand unchanged - so
            // the settled reading is the same bits as the divided one, and the
            // longer the input the larger the share of it taken this way.
            let (corr_f, corr_s, corr_e);
            if settled {
                corr_f = 1.0;
                corr_s = 1.0;
                corr_e = 1.0;
            } else {
                pow_f *= a[1];
                pow_s *= a[3];
                pow_e *= ENT_SLOW_A;
                // The corrections divide every class, so they are inverted once
                // per byte and applied as multiplies rather than divided out per
                // class.
                corr_f = 1.0 / (1.0 - pow_f).max(1e-4);
                corr_s = 1.0 / (1.0 - pow_s).max(1e-4);
                corr_e = (1.0 - pow_e).max(1e-4);
                settled = corr_f == 1.0 && corr_s == 1.0 && corr_e == 1.0;
            }
            ent_slow += (1.0 - ENT_SLOW_A) * (cur_entropy - ent_slow);
            let ent_slow_c = if corr_e == 1.0 { ent_slow } else { ent_slow / corr_e };
            let fast_mix = &bands[N_CLASSES..2 * N_CLASSES];
            let slow_mix = &bands[3 * N_CLASSES..4 * N_CLASSES];
            let pairs = fast_mix.iter().zip(slow_mix.iter());
            let class_div: f32 = if settled {
                pairs.map(|(x, y)| (x - y).powi(2)).sum()
            } else {
                pairs.map(|(x, y)| (x * corr_f - y * corr_s).powi(2)).sum()
            };
            let dist = class_div.sqrt() + ENT_WEIGHT * (cur_entropy - ent_slow_c).abs();
            if i == 0 {
                cp_mean = dist;
                cp_mad = dist * 0.5 + 0.01;
            } else {
                // The spread the threshold is computed from, in the deviation's own
                // units: `cp_mad`, or the floor's share of the mean over
                // `cp_threshold` where that is larger, and never under
                // `CP_NOISE`. A uniform run's reading is rounding noise whose
                // spread falls toward nothing, and a clip at nothing would
                // cut the noise's own steps, settle the threshold a few of
                // them above the mean, and let the next step cross it.
                let floored = if cfg.cp_floor > 0.0 && cfg.cp_threshold > 0.0 {
                    cp_mad.max(cfg.cp_floor * cp_mean / cfg.cp_threshold)
                } else {
                    cp_mad
                };
                let unit = floored.max(CP_NOISE);
                // Before `CP_WARMUP` no boundary is placed and the clocks are
                // still filling, so the statistics take each deviation whole
                // and the threshold has caught up by the first byte it is
                // read at.
                let dev = if i < CP_WARMUP { dist - cp_mean } else { (dist - cp_mean).clamp(-CP_CLIP * unit, CP_CLIP * unit) };
                cp_mean += CP_ALPHA * dev;
                cp_mad += CP_ALPHA * (dev.abs() - cp_mad);
            }
            // `cp_mad` is the reader's estimate of how far `dist` usually
            // moves, so `k*mad` is a spread that goes to zero where `dist`
            // stops moving at all - and a threshold at the mean is one an ULP
            // of noise clears. The floor is a share of the mean rather than a
            // quantity in the signal's units, so it holds wherever the mean
            // is; at zero the branch is not taken and the threshold is the
            // spread's alone.
            let spread = cfg.cp_threshold * cp_mad;
            let thresh = cp_mean
                + if cfg.cp_floor > 0.0 { spread.max(cfg.cp_floor * cp_mean) } else { spread };
            if i >= CP_WARMUP {
                let spaced = i as isize - last_boundary >= cfg.cp_min_gap as isize;
                if cp_armed && dist > thresh && spaced {
                    boundaries.push(i);
                    last_boundary = i as isize;
                    cp_armed = false;
                    let mut left = [0.0f32; N_CLASSES];
                    for (l, &y) in left.iter_mut().zip(slow_mix.iter()) {
                        *l = y * corr_s;
                    }
                    regime = Some((left, 0.0, 0, 0));
                } else {
                    if !cp_armed && dist < thresh * 0.5 {
                        cp_armed = true;
                    }
                    if let Some((left, sum, count, under)) = regime.as_mut() {
                        let back: f32 =
                            fast_mix.iter().zip(left.iter()).map(|(x, y)| (x * corr_f - y).powi(2)).sum::<f32>().sqrt();
                        *sum += back;
                        *count += 1;
                        *under = if back < CP_RETURN_SHARE * (*sum / *count as f32) { *under + 1 } else { 0 };
                        if *under >= CP_RETURN_SUSTAIN && spaced {
                            // The run began `CP_RETURN_SUSTAIN - 1` bytes back,
                            // where the reading came back; the boundary is
                            // there unless that is inside the minimum gap.
                            let at = (i + 1 - CP_RETURN_SUSTAIN as usize)
                                .max((last_boundary + cfg.cp_min_gap as isize) as usize);
                            boundaries.push(at);
                            last_boundary = at as isize;
                            cp_armed = true;
                            regime = None;
                        }
                    }
                }
            }
        }

        // Sample a frame at the end of each hop window and at the input end.
        if (i + 1) % hop == 0 || i + 1 == n {
            frames.push(SpectralFrame {
                bands,
                entropy: cur_entropy,
                period: cur_period,
                period_strength: cur_strength,
                novelty: cur_novelty,
                wide,
            });
        }
    }

    // Handed over once rather than once an evaluation: `counted` takes a lock,
    // and a lock inside the byte loop would price the watch above the work it
    // watches.
    crate::trace::counted("the spectral field: period evaluations", period_evals);
    SpectralField { len: n, hop, frames, boundaries, needs }
}

/// Default entropy threshold (x100) above which the lexer treats a
/// sustained run as an opaque blob rather than shredding it into tokens.
pub const BLOB_ENTROPY_PCT: u8 = 85;
/// Minimum length, in bytes, of a high-entropy run before it collapses.
pub const BLOB_MIN_LEN: usize = 48;

/// Whitespace-bounded byte ranges of sustained high-entropy content - the
/// blobs (base64 / hex / packed data) the lexer collapses to one opaque
/// token instead of shredding into garbage word and number tokens.
///
/// A run qualifies when the rolling entropy stays at or above
/// `threshold_pct/100` for at least `min_len` bytes, the threshold raised
/// toward one by the share of the window's bytes inside well-formed
/// multi-byte UTF-8 characters; each qualifying run is then expanded outward
/// to the enclosing whitespace-free span (so the whole blob is one token, not
/// just its high-entropy core), and overlapping spans are merged. Returns
/// ascending, non-overlapping ranges.
///
/// A whitespace byte is never in a run, so a run is inside one
/// whitespace-free span, and a span shorter than `min_len` holds no run that
/// qualifies: the flag pass reads the spans at least `min_len` long and
/// nothing else, and the runs are the ones a pass over every byte finds.
#[must_use]
pub fn high_entropy_runs(input: &[u8], threshold_pct: u8, min_len: usize) -> Vec<(usize, usize)> {
    let n = input.len();
    if n < min_len {
        return Vec::new();
    }
    runs_over_spans(input, &long_spans(input, 0, n, min_len), threshold_pct, min_len)
}

/// [`high_entropy_runs`] inside one whitespace-free span `a..b` of `input`:
/// the runs the whole-input pass finds there, since the flag pass seeds its
/// window from the bytes before `a` and a run never crosses whitespace. A
/// span shorter than `min_len` holds none.
#[must_use]
pub fn high_entropy_runs_in_span(
    input: &[u8],
    a: usize,
    b: usize,
    threshold_pct: u8,
    min_len: usize,
) -> Vec<(usize, usize)> {
    if b - a < min_len {
        return Vec::new();
    }
    runs_over_spans(input, &[(a, b)], threshold_pct, min_len)
}

/// [`high_entropy_runs`] inside `input[from..to]`, a stretch that may hold
/// whitespace: the runs the whole-input pass finds there, read over the
/// whitespace-free spans inside the stretch one by one as that pass reads
/// them, each seeded from the bytes before it. A stretch shorter than
/// `min_len` holds none.
#[must_use]
pub fn high_entropy_runs_within(
    input: &[u8],
    from: usize,
    to: usize,
    threshold_pct: u8,
    min_len: usize,
) -> Vec<(usize, usize)> {
    if to - from < min_len {
        return Vec::new();
    }
    runs_over_spans(input, &long_spans(input, from, to, min_len), threshold_pct, min_len)
}

/// The qualifying runs inside `spans`, the long spans of `input` in order,
/// from the flag pass over each: maximal high-entropy runs of at least
/// `min_len`, closed as the scan leaves them rather than marked per byte and
/// collected in a second pass. A span's end closes the run reaching it, as
/// the whitespace byte there would.
fn runs_over_spans(
    input: &[u8],
    spans: &[(usize, usize)],
    threshold_pct: u8,
    min_len: usize,
) -> Vec<(usize, usize)> {
    let mut raw: Vec<(usize, usize)> = Vec::new();
    for &(a, b) in spans {
        // Every run inside this span expands to the same enclosing
        // whitespace-free span, because the span holds no whitespace to stop
        // the expansion anywhere inside it. Reading it once a span rather than
        // once a run is what keeps this linear: the walk is a byte at a time
        // to the nearest whitespace either side, so on an input that has none
        // it reaches the input's own ends, and doing that per run makes the
        // pass quadratic in the number of runs.
        if holds_a_run(input, a, b, threshold_pct, min_len) {
            raw.push(whitespace_free_span(input, a, b));
        }
    }
    merge_spans(raw)
}

/// Whether the flag pass over `input[from..to]`, seeded from the bytes
/// before `from`, meets a high-entropy run at least `min_len` long; a run
/// still open at `to` counts by the length it has reached there.
fn holds_a_run(input: &[u8], from: usize, to: usize, threshold_pct: u8, min_len: usize) -> bool {
    let mut qualifies = false;
    let mut run_start: Option<usize> = None;
    entropy_flags(input, from, to, threshold_pct, |i, hi| match (hi, run_start) {
        (true, None) => run_start = Some(i),
        (false, Some(rs)) => {
            qualifies |= i - rs >= min_len;
            run_start = None;
        }
        _ => {}
    });
    if let Some(rs) = run_start {
        qualifies |= to - rs >= min_len;
    }
    qualifies
}

/// [`runs_over_spans`] across the cores. The spans are cut into pieces of at
/// most `leaf_bytes` and consecutive pieces gathered into leaves of about that
/// many bytes between them; a piece reads its bytes and `min_len` past its
/// cut, so a run crossing a cut is seen at least `min_len` long by the piece
/// it starts in, and a span qualifies where any piece of it does. Every run
/// starts in some piece, so the spans found are the spans the pass over
/// whole spans finds. A single leaf runs that pass itself.
fn runs_over_spans_across(
    input: &[u8],
    spans: &[(usize, usize)],
    threshold_pct: u8,
    min_len: usize,
    least_a_leaf: usize,
) -> Vec<(usize, usize)> {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    let span_bytes: usize = spans.iter().map(|&(a, b)| b - a).sum();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let leaf_bytes = span_bytes.div_ceil(cores * 4).max(least_a_leaf).max(1);
    // A piece is a span's index and the part of it the piece reads before
    // its reach past the cut.
    let mut pieces: Vec<(usize, usize, usize)> = Vec::new();
    for (k, &(a, b)) in spans.iter().enumerate() {
        let mut p = a;
        while p < b {
            let q = (p + leaf_bytes).min(b);
            pieces.push((k, p, q));
            p = q;
        }
    }
    // A leaf is a range of consecutive pieces closed once it holds
    // `leaf_bytes`.
    let mut leaves: Vec<(usize, usize)> = Vec::new();
    let mut held = 0usize;
    let mut first = 0usize;
    for (i, &(_, p, q)) in pieces.iter().enumerate() {
        held += q - p;
        if held >= leaf_bytes {
            leaves.push((first, i + 1));
            first = i + 1;
            held = 0;
        }
    }
    if first < pieces.len() {
        leaves.push((first, pieces.len()));
    }
    if leaves.len() <= 1 {
        return runs_over_spans(input, spans, threshold_pct, min_len);
    }
    let mut found: Vec<Vec<usize>> = vec![Vec::new(); leaves.len()];
    // What a leaf costs, for the pool's own choice between the cores and
    // inline: the pass over whole spans read 0.315 ms over the 72,972 span
    // bytes of this crate's own src, 4.32 ns a byte.
    let per_leaf_ns = (leaf_bytes as u64 * 4320 / 1000).min(u64::from(u32::MAX)) as u32;
    let plan = JobPlan::new(0, leaves.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::Streaming)
        .with_estimated_per_item_ns(per_leaf_ns);
    for_each_chunk_indexed_min_leaf(&plan, &mut found, 1, |start, slots| {
        for (i, slot) in slots.iter_mut().enumerate() {
            let (lo, hi) = leaves[start + i];
            for &(k, p, q) in &pieces[lo..hi] {
                let reach = (q + min_len).min(spans[k].1);
                if slot.last() != Some(&k) && holds_a_run(input, p, reach, threshold_pct, min_len) {
                    slot.push(k);
                }
            }
        }
    });
    let mut qualifies = vec![false; spans.len()];
    for k in found.into_iter().flatten() {
        qualifies[k] = true;
    }
    let raw = spans
        .iter()
        .zip(qualifies)
        .filter(|&(_, q)| q)
        .map(|(&(a, b), _)| whitespace_free_span(input, a, b))
        .collect();
    merge_spans(raw)
}

/// Bytes a leaf of the span finder across the cores holds, at the least. The
/// per-byte work is a fraction of a nanosecond, so a leaf below this is
/// dispatch rather than work.
const ENTROPY_FLAGS_MIN_LEAF: usize = 64 * 1024;

/// Bytes a leaf of the runs over the spans across the cores holds, at the
/// least. Measured by `where_the_time_goes` on a 24-thread box, 64 kB
/// against 16 kB and 8 kB: over the 72,972 span bytes of this crate's own
/// src the runs over the spans read 0.400, 0.146 and 0.089 ms a scan, over
/// the 107,637 of real_code.txt 0.288, 0.120 and 0.103, and over the 400,754
/// of train_corpus.txt 0.488 and 0.226 at 64 kB and 16 kB; the walls of the
/// scans around them did not tell 16 kB from 8 kB.
const ENTROPY_RUNS_MIN_LEAF: usize = 8 * 1024;

/// [`high_entropy_runs`] across cores: the long spans found across the
/// cores, then the flag pass over them across the cores, reading only the
/// spans' bytes. A byte's flag depends on the sixty-four bytes up to it, the
/// three before those and the rest of the character it ends in, and nothing
/// else, so a piece seeded from the bytes before its start reads exactly what
/// the whole-stream pass reads there. Byte-identical to the serial form.
#[must_use]
pub fn high_entropy_runs_parallel(input: &[u8], threshold_pct: u8, min_len: usize) -> Vec<(usize, usize)> {
    let n = input.len();
    if n < min_len {
        return Vec::new();
    }
    // Each part is a phase entered once a call, so the table's cost reads by
    // part.
    let spanning = crate::trace::phase("the blob table: the long spans");
    let spans = long_spans_across(input, min_len, ENTROPY_FLAGS_MIN_LEAF);
    let span_bytes: usize = spans.iter().map(|&(a, b)| b - a).sum();
    drop(spanning);
    crate::trace::counted("the blob table: bytes in the long spans", span_bytes as u64);
    let _running = crate::trace::phase("the blob table: the runs over the spans");
    runs_over_spans_across(input, &spans, threshold_pct, min_len, ENTROPY_RUNS_MIN_LEAF)
}

/// The high-entropy flag of each byte of `input[from..to]`, in order, to
/// `emit`.
///
/// The rolling window is the sixty-four bytes up to and including the byte,
/// so the pass seeds it from the bytes before `from` and a range starting
/// mid-stream reads exactly what a whole-stream pass reads there; whether a
/// window byte is inside a multi-byte character is read from the bytes
/// around it, never from where the pass began.
///
/// The flag pass, compiled for whatever instruction set this CPU reports.
///
/// The body is a serial recurrence - a 256-bin histogram updated in place
/// and a running sum carried across bytes - so there is no vector loop here
/// to write by hand. What a wider target gains it is scalar: the newer
/// addressing and bit-manipulation forms, and a better lowering of the
/// float work per byte. `#[target_feature]` grants the whole instruction
/// set to the function it marks, so the same source compiled behind each
/// gate is enough to collect that, with no intrinsics.
///
/// Every rung runs the identical body and must agree flag for flag; the
/// gate is the only thing that differs between them, which is what
/// `entropy_flags_ladder_agrees` checks.
///
/// The ladder descends AVX-512, AVX2, SSE2, scalar, and every rung is
/// present in every binary: the build stays portable and the CPU decides.
/// SSE2 is architectural on x86-64 so that rung always applies there, and
/// the scalar body is what runs on every other architecture.
///
/// Calling a `#[target_feature]` function on a CPU without the feature is
/// undefined behavior. [`crate::isa::tier`] discharges that precondition,
/// reporting the rungs this CPU can run from a probe resolved once for the
/// process.
#[allow(unsafe_code)]
fn entropy_flags(
    input: &[u8],
    from: usize,
    to: usize,
    threshold_pct: u8,
    emit: impl FnMut(usize, bool),
) {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: `crate::isa::tier` reports the rungs this CPU
        // can run, and a rung implies every rung below it, so reaching an
        // arm is the guarantee that its features are present.
        match crate::isa::tier() {
            crate::isa::Tier::Avx512 => {
                return unsafe { entropy_flags_avx512(input, from, to, threshold_pct, emit) };
            }
            crate::isa::Tier::Avx2 => {
                return unsafe { entropy_flags_avx2(input, from, to, threshold_pct, emit) };
            }
            crate::isa::Tier::Sse2 => {
                return unsafe { entropy_flags_sse2(input, from, to, threshold_pct, emit) };
            }
            crate::isa::Tier::Scalar => {}
        }
    }
    entropy_flags_impl(input, from, to, threshold_pct, emit);
}

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
#[target_feature(enable = "avx512f", enable = "avx512bw")]
unsafe fn entropy_flags_avx512(
    input: &[u8],
    from: usize,
    to: usize,
    threshold_pct: u8,
    emit: impl FnMut(usize, bool),
) {
    entropy_flags_impl(input, from, to, threshold_pct, emit);
}

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
#[target_feature(enable = "avx2", enable = "bmi2")]
unsafe fn entropy_flags_avx2(
    input: &[u8],
    from: usize,
    to: usize,
    threshold_pct: u8,
    emit: impl FnMut(usize, bool),
) {
    entropy_flags_impl(input, from, to, threshold_pct, emit);
}

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
#[target_feature(enable = "sse2")]
unsafe fn entropy_flags_sse2(
    input: &[u8],
    from: usize,
    to: usize,
    threshold_pct: u8,
    emit: impl FnMut(usize, bool),
) {
    entropy_flags_impl(input, from, to, threshold_pct, emit);
}

// Inlined into each gated wrapper so the body is compiled once per target,
// rather than called across a boundary that would keep it at baseline.
#[inline(always)]
fn entropy_flags_impl(input: &[u8], from: usize, to: usize, threshold_pct: u8, mut emit: impl FnMut(usize, bool)) {
    let w = 64usize;
    let mut hist = [0u32; 256];
    // The window's `sum c*log2(c)`, in fixed point (see `FIXED_LOG_SCALE`).
    let mut s = 0i64;
    let entropy_norm = (w.min(256) as f32).max(2.0).log2();
    let thr = f32::from(threshold_pct) / 100.0;
    // The window is full after `w` bytes and stays full, so `log2(nwin)` is
    // one constant for all but the first `w` positions - it was a libm call
    // per byte computing the same number. The reciprocal goes with it, since
    // dividing by the window is the other per-byte constant; it carries the
    // fixed-point scale so the sum converts in the same multiply.
    let full_log2 = (w as f64).log2();
    let full_recip = 1.0 / (w as f64 * FIXED_LOG_SCALE);
    // The entropy of a full window is a function of `s` alone, and every
    // step of it - the product, the subtraction, the narrowing to f32, the
    // division - is monotone in `s`, so the positions where it clears the
    // threshold are exactly the `s` at or below one boundary. That boundary
    // is found once from the same expression the partial window evaluates,
    // and a full-window byte then compares `s` to it: no float work per byte,
    // and the same flag the expression would give.
    let full_high = |s: i64| (full_log2 - s as f64 * full_recip) as f32 / entropy_norm >= thr;
    let s_high_max: i64 = {
        // Sixty-four bytes give at most `64 * log2(64)` in sum, and no
        // count table sum is negative, so the boundary is in this range or
        // below it entirely.
        let (mut lo, mut hi) = (-1i64, (w as i64) * 6 * FIXED_LOG_SCALE as i64 + 1);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if full_high(mid) { lo = mid } else { hi = mid }
        }
        lo
    };
    // The tables resolve their locks once here rather than on each lookup
    // every byte makes.
    let lut = fixed_log_table();
    let dlut = fixed_log_delta_table();
    // Which window slots hold a byte inside a well-formed multi-byte
    // character, one bit a slot, and how many do. Text in a script written in
    // such characters spreads each one over continuation bytes that cycle
    // through sixty-four values, so the threshold rises toward one with their
    // share: `thr + (1 - thr) * share`. A window holding none compares as
    // the threshold alone does.
    let mut wide_bits = 0u64;
    let mut wide_count = 0u32;
    let mut wide_until = 0usize;
    let mut is_wide = |j: usize| -> bool {
        if j < wide_until {
            return true;
        }
        match multibyte_char_at(input, j) {
            Some((_, end)) => {
                wide_until = end;
                true
            }
            None => false,
        }
    };
    for (j, &b) in input.iter().enumerate().take(from).skip(from.saturating_sub(w)) {
        let nb = hist[b as usize];
        hist[b as usize] = nb + 1;
        s += fixed_term(lut, nb + 1) - fixed_term(lut, nb);
        if is_wide(j) {
            wide_bits |= 1 << (j % w);
            wide_count += 1;
        }
    }
    for i in from..to {
        let b = input[i] as usize;
        let nb = hist[b];
        hist[b] = nb + 1;
        // Counts stay within the window, so both deltas are in the table.
        let mut delta = dlut[nb as usize];
        let slot = 1u64 << (i % w);
        if i >= w {
            let ob = input[i - w] as usize;
            let oc = hist[ob];
            hist[ob] = oc - 1;
            delta -= dlut[(oc - 1) as usize];
            if wide_bits & slot != 0 {
                wide_bits &= !slot;
                wide_count -= 1;
            }
        }
        s += delta;
        if is_wide(i) {
            wide_bits |= slot;
            wide_count += 1;
        }
        let high = if wide_count == 0 {
            if i + 1 >= w {
                s <= s_high_max
            } else {
                let nwin = f64::from((i + 1) as u32);
                (nwin.log2() - s as f64 * (1.0 / (nwin * FIXED_LOG_SCALE))) as f32 / entropy_norm >= thr
            }
        } else {
            let nwin = (i + 1).min(w);
            let (log_nwin, recip) = if nwin == w {
                (full_log2, full_recip)
            } else {
                let nw = f64::from(nwin as u32);
                (nw.log2(), 1.0 / (nw * FIXED_LOG_SCALE))
            };
            let share = wide_count as f32 / nwin as f32;
            (log_nwin - s as f64 * recip) as f32 / entropy_norm >= thr + (1.0 - thr) * share
        };
        // A whitespace byte is never part of a run. The window trails the
        // cursor, so it stays blob-dominated for up to `w` bytes past a blob's
        // end; without this the run continues over the separator into the text
        // after it, and the returned span is neither whitespace-free nor
        // confined to one line.
        emit(i, high && !input[i].is_ascii_whitespace());
    }
}

/// The well-formed multi-byte UTF-8 character holding byte `j` of `input`,
/// as `[start, end)`, or `None` for an ASCII byte or a byte not part of a
/// well-formed character. Read from the at most three bytes before `j` and the
/// character's own bytes, so every pass over `input` reads the same answer
/// whatever byte it starts from.
fn multibyte_char_at(input: &[u8], j: usize) -> Option<(usize, usize)> {
    if input[j] < 0x80 {
        return None;
    }
    let mut start = j;
    while (0x80..=0xBF).contains(&input[start]) {
        if start == 0 || j - start == 3 {
            return None;
        }
        start -= 1;
    }
    let len = match input[start] {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    let end = start + len;
    (j < end && end <= input.len() && std::str::from_utf8(&input[start..end]).is_ok()).then_some((start, end))
}

/// The run `[rs, re)` expanded outward to the enclosing whitespace-free
/// span, so the whole blob is one token rather than just its high-entropy
/// core.
fn whitespace_free_span(input: &[u8], rs: usize, re: usize) -> (usize, usize) {
    let mut a = rs;
    while a > 0 && !input[a - 1].is_ascii_whitespace() {
        a -= 1;
    }
    let mut b = re;
    while b < input.len() && !input[b].is_ascii_whitespace() {
        b += 1;
    }
    (a, b)
}

/// The parts inside `[from, to)` of the whitespace-free spans of `input` at
/// least `min_len` bytes long, ascending. The span scan runs over the range
/// widened by `min_len` on each side, so a span crossing the range's edge is
/// measured to at least `min_len` before it is cut to the range, which
/// decides "at least `min_len`" exactly while reading a bounded distance
/// outside it.
fn long_spans(input: &[u8], from: usize, to: usize, min_len: usize) -> Vec<(usize, usize)> {
    let lo = from.saturating_sub(min_len);
    let hi = (to + min_len).min(input.len());
    crate::byte_simd::nonspace_spans_at_least(&input[lo..hi], min_len)
        .into_iter()
        .filter_map(|(a, b)| {
            let (a, b) = ((lo + a).max(from), (lo + b).min(to));
            (a < b).then_some((a, b))
        })
        .collect()
}

/// [`long_spans`] over the whole input across the cores: each leaf reads its
/// range as `long_spans` reads one - widened by `min_len` each side and cut
/// to the range - so a span crossing a cut arrives from both sides as two
/// pieces that touch, and the merge joins them back into the one span the
/// pass over the whole input reports. Every piece is a part of a span that
/// pass reports and every such span is covered, so the two agree exactly.
///
/// Each leaf takes at least `least_a_leaf` bytes; the input is read once
/// plus `min_len` twice a cut.
fn long_spans_across(input: &[u8], min_len: usize, least_a_leaf: usize) -> Vec<(usize, usize)> {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    let n = input.len();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let leaf_bytes = n.div_ceil(cores * 4).max(least_a_leaf).max(1);
    let leaves = n.div_ceil(leaf_bytes);
    if leaves <= 1 {
        return long_spans(input, 0, n, min_len);
    }
    let mut found: Vec<Vec<(usize, usize)>> = vec![Vec::new(); leaves];
    // What a leaf costs, for the pool's own choice between running the
    // leaves across the cores and inline: the pass over the whole input read
    // 539.74 us over 3,688,563 bytes of this crate's own src, 0.146 ns a byte.
    let per_leaf_ns = (leaf_bytes as u64 * 146 / 1000).min(u64::from(u32::MAX)) as u32;
    let plan = JobPlan::new(0, leaves as u32)
        .with_leaf_shape(flynnel::LeafShape::Streaming)
        .with_estimated_per_item_ns(per_leaf_ns);
    for_each_chunk_indexed_min_leaf(&plan, &mut found, 1, |start, slots| {
        for (i, slot) in slots.iter_mut().enumerate() {
            let from = (start + i) * leaf_bytes;
            let to = (from + leaf_bytes).min(n);
            *slot = long_spans(input, from, to, min_len);
        }
    });
    merge_spans(found.into_iter().flatten().collect())
}

/// Merge ascending spans that touch or overlap.
fn merge_spans(raw: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (a, b) in raw {
        if let Some(last) = runs.last_mut()
            && a <= last.1
        {
            last.1 = last.1.max(b);
        } else {
            runs.push((a, b));
        }
    }
    runs
}

/// The intrinsic region split of an analyzed field: the input segmented at
/// change-point boundaries, each segment classified by its pooled texture,
/// with adjacent same-texture segments merged. This is the dictionary-free,
/// fence-free alternative to marker-based region detection - the
/// prose/code/math/data split a literate-document consumer (`trimodal`,
/// `bundle`, `binpos`) needs, derived from the bytes alone.
#[must_use]
pub fn regions(field: &SpectralField) -> Vec<(usize, usize, Texture)> {
    if field.len == 0 {
        return Vec::new();
    }
    let mut cuts: Vec<usize> = Vec::with_capacity(field.boundaries.len() + 2);
    cuts.push(0);
    cuts.extend(field.boundaries.iter().copied());
    cuts.push(field.len);
    cuts.dedup();
    let mut out: Vec<(usize, usize, Texture)> = Vec::new();
    for w in cuts.windows(2) {
        let (s, e) = (w[0], w[1]);
        if e <= s {
            continue;
        }
        let tex = texture_of(&field.signature(s, e));
        if let Some(last) = out.last_mut()
            && last.2 == tex
        {
            last.1 = e;
        } else {
            out.push((s, e, tex));
        }
    }
    out
}

/// Code-tuned region classification - the code analog of [`regions`]. Splits
/// `input` at its spectral change-points and labels each span by its
/// [`CodeTexture`] (code / blob / prose / numeric), computed from a FRESH
/// spectral pass over only that span so a region's label cannot bleed across
/// its boundary the way the causal per-byte reader does. For consumers that
/// need the code {code, blob, prose, numeric} split rather than the generic
/// {prose, code, math, data} one - disassembly code-vs-data, packed-region
/// detection, and the construct layer's region texture. Adjacent same-texture
/// spans are merged.
#[must_use]
pub fn code_regions(input: &[u8]) -> Vec<(usize, usize, CodeTexture)> {
    code_regions_with(input, &SpectralConfig::default())
}

/// [`code_regions`] split at the change-points `cfg` places. Each span's
/// texture is still read by a fresh default pass over its own bytes.
#[must_use]
pub fn code_regions_with(input: &[u8], cfg: &SpectralConfig) -> Vec<(usize, usize, CodeTexture)> {
    let field = analyze_with(input, cfg);
    if field.len == 0 {
        return Vec::new();
    }
    let mut cuts: Vec<usize> = Vec::with_capacity(field.boundaries.len() + 4);
    cuts.push(0);
    cuts.extend(field.boundaries.iter().copied());
    // Isolate packed / base64 / hex runs as their own regions: the change-point
    // detector alone gives a coarse span that absorbs the surrounding code into
    // a blob-dominated region, so add the sharp `high_entropy_runs` bounds.
    for (s, e) in high_entropy_runs(input, BLOB_ENTROPY_PCT, BLOB_MIN_LEN) {
        cuts.push(s);
        cuts.push(e);
    }
    cuts.push(field.len);
    cuts.sort_unstable();
    cuts.dedup();
    let mut out: Vec<(usize, usize, CodeTexture)> = Vec::new();
    for w in cuts.windows(2) {
        let (s, e) = (w[0], w[1]);
        if e <= s {
            continue;
        }
        let tex = code_texture(&analyze(&input[s..e]).signature(0, e - s));
        if let Some(last) = out.last_mut()
            && last.2 == tex
        {
            last.1 = e;
        } else {
            out.push((s, e, tex));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sliding_scanner_agrees_with_the_one_shot_reading() {
        // Two implementations of one reading, so they are held against each
        // other rather than each against itself. The scanner slides exact
        // integer cross-sums; dominant_period rebuilds a centered f32 dot
        // product. They must pick the same lag.
        let csv: Vec<u8> = (0..300)
            .flat_map(|i| format!("{:03},{:03},{:03}\n", i % 1000, (i * 7) % 1000, (i * 13) % 1000).into_bytes())
            .collect();
        let prose: Vec<u8> = "it was the best of times it was the worst of times it was the age of wisdom "
            .bytes().cycle().take(9000).collect();
        let mut x = 0x1234_5678u32;
        let noise: Vec<u8> = (0..9000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x & 0xff) as u8
            })
            .collect();

        for (tag, data) in [("csv", csv), ("prose", prose), ("noise", noise)] {
            let (win, lag, hop) = (512usize, 128usize, 64usize);
            let mut scan = PeriodScanner::new(&data, win, lag);
            let mut end = win;
            let mut checked = 0u32;
            while end <= data.len() {
                let (want, want_st) = dominant_period(&data[end - win..end], lag);
                let (got, got_st) = scan.eval(&data, end);
                assert_eq!(got, want, "{tag} at {end}: lag disagrees");
                assert!(
                    (got_st - want_st).abs() < 1e-3,
                    "{tag} at {end}: strength {got_st} vs {want_st}"
                );
                checked += 1;
                end += hop;
            }
            assert!(checked > 10, "{tag}: the sweep must actually compare windows");
        }
    }

    #[test]
    fn the_scanner_reseeds_rather_than_sliding_past_a_whole_window() {
        // A jump wider than the window leaves nothing to reuse, and the two
        // correction ranges would overlap; it must rebuild instead.
        let data: Vec<u8> =
            (0..4000).map(|i: usize| b"abcd12"[i % 6]).collect();
        let win = 256usize;
        let mut scan = PeriodScanner::new(&data, win, 64);
        // Evaluate, then jump far past the window, then evaluate again.
        let a = scan.eval(&data, win);
        let far = scan.eval(&data, 3000);
        let fresh = dominant_period(&data[3000 - win..3000], 64);
        assert_eq!(far.0, fresh.0, "a reseeded window reads as a fresh one");
        assert_eq!(a.0, 6, "the six-byte cycle is found either way");
    }

    #[test]
    fn csv_rows_have_a_dominant_period() {
        // "a,b,c\n" is a 6-byte period; autocorrelation must recover it.
        let mut input = Vec::new();
        for _ in 0..200 {
            input.extend_from_slice(b"a,b,c\n");
        }
        let field = analyze(&input);
        assert!(
            field.frames.iter().any(|f| f.period == 6 && f.period_strength > 0.3),
            "expected a frame with period 6; got periods {:?}",
            field.frames.iter().map(|f| f.period).collect::<Vec<_>>()
        );
    }

    #[test]
    fn base64_blob_reads_high_entropy() {
        let alpha = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let blob: Vec<u8> = (0..800).map(|i| alpha[(i * 7 + i * i * 13) % 64]).collect();
        let field = analyze(&blob);
        let max_e = field.frames.iter().map(|f| f.entropy).fold(0.0f32, f32::max);
        assert!(max_e > 0.7, "base64 blob should read high entropy; max was {max_e}");
    }

    #[test]
    fn prose_and_code_classify_distinctly() {
        let prose = b"the quick brown fox jumps over the lazy dog and then the dog sleeps well into the warm afternoon while the fox wanders off across the wide green field again";
        let code = b"fn f(x){let y=x+1;return y*2;} fn g(a,b){if(a>b){a-=b;}else{b-=a;} return a;} struct P{x:i32,y:i32}";
        let pf = analyze(prose);
        let cf = analyze(code);
        let pt = texture_of(&pf.signature(0, prose.len()));
        let ct = texture_of(&cf.signature(0, code.len()));
        assert_eq!(pt, Texture::Prose, "prose misclassified as {}", pt.label());
        assert_eq!(ct, Texture::Code, "code misclassified as {}", ct.label());
    }

    #[test]
    fn repeated_template_drops_in_novelty() {
        let mut input = Vec::new();
        for _ in 0..200 {
            input.extend_from_slice(b"INFO: request handled ok\n");
        }
        let field = analyze(&input);
        let n = field.frames.len();
        assert!(n >= 4, "need several frames");
        let first: f32 = field.frames[..n / 4].iter().map(|f| f.novelty).sum::<f32>()
            / (n / 4).max(1) as f32;
        let last: f32 = field.frames[3 * n / 4..].iter().map(|f| f.novelty).sum::<f32>()
            / (n - 3 * n / 4).max(1) as f32;
        assert!(last < first, "novelty should drop on repetition: first {first}, last {last}");
    }

    #[test]
    fn regime_switch_records_a_change_point() {
        // Three 300-byte regimes: letters, digits, punctuation. The reader
        // should mark a boundary near each switch (300 and 600).
        let mut input = Vec::new();
        input.extend(std::iter::repeat_n(b'a', 300));
        input.extend(std::iter::repeat_n(b'7', 300));
        input.extend(std::iter::repeat_n(b'#', 300));
        let field = analyze(&input);
        assert!(field.boundary_near(300, 24), "no change-point near 300: {:?}", field.boundaries);
        assert!(field.boundary_near(600, 24), "no change-point near 600: {:?}", field.boundaries);
    }

    #[test]
    fn a_regime_switch_is_found_past_the_settling_of_the_bias_corrections() {
        // Each clock's bias correction is `1 / (1 - a^(t+1))`, and the power it
        // subtracts falls under the last bit of an f32 beside one a couple of
        // thousand bytes in; from there the correction is exactly one and the
        // reading takes the settled path. Five 4000-byte regimes put every
        // switch but the first past that point, which is where the longer part
        // of any real input is read.
        let mut input = Vec::new();
        for b in *b"a7#a7" {
            input.extend(std::iter::repeat_n(b, 4000));
        }
        let field = analyze(&input);
        for at in [4000usize, 8000, 12000, 16000] {
            assert!(
                field.boundary_near(at, 64),
                "no change-point near {at}: {:?}",
                field.boundaries
            );
        }
        // Every boundary is a switch, and the first switch is what this
        // covers. A regime is uniform, so the divergence inside it is
        // constant and the spread `cp_mad` estimates goes to zero;
        // `cp_floor` is what keeps the threshold off the signal there, and a
        // boundary inside a run would also hold the hysteresis closed
        // through the switch after it. Over six real corpora read in both
        // line-ending conventions, 69,118 boundaries as CRLF and 74,841 as
        // LF, the floor at its default moves none of them.
        for &b in &field.boundaries {
            assert!(
                [4000usize, 8000, 12000, 16000].iter().any(|&s| b.abs_diff(s) <= 64),
                "a change-point at {b} is inside a regime: {:?}",
                field.boundaries
            );
        }
    }

    #[test]
    fn boundary_lookups_answer_as_a_scan_over_every_change_point_does() {
        // The binary searches are held against the definitions they stand
        // in for, over every span and every byte of a field whose
        // change-points are at both ends and unevenly in between.
        let boundaries = vec![0usize, 3, 4, 10, 27, 28, 64, 100];
        let field = SpectralField {
            len: 101,
            hop: 1,
            frames: Vec::new(),
            boundaries: boundaries.clone(),
            needs: Needs::all(),
        };
        for start in 0..=104 {
            for end in 0..=104 {
                let scan = boundaries.iter().any(|&b| b >= start && b < end);
                assert_eq!(field.boundary_in(start, end), scan, "boundary_in({start}, {end})");
            }
        }
        for byte in 0..=104 {
            for tol in 0..8 {
                let scan = boundaries.iter().any(|&b| b.abs_diff(byte) <= tol);
                assert_eq!(field.boundary_near(byte, tol), scan, "boundary_near({byte}, {tol})");
            }
        }
        let none = SpectralField {
            len: 0,
            hop: 1,
            frames: Vec::new(),
            boundaries: Vec::new(),
            needs: Needs::all(),
        };
        assert!(!none.boundary_in(0, 10));
        assert!(!none.boundary_near(5, 5));
    }

    #[test]
    fn empty_input_is_safe() {
        let field = analyze(b"");
        assert_eq!(field.len, 0);
        assert!(field.frames.is_empty());
        assert_eq!(field.signature(0, 0), SpectralFrame::default());
    }

    /// A deterministic high-entropy base64-alphabet blob of `n` bytes (a
    /// fixed-seed PCG-style LCG, so no RNG and fully reproducible).
    fn hi_entropy_blob(n: usize) -> Vec<u8> {
        const ALPHA: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut x = 0x2545_f491_4f6c_dd1du64;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                ALPHA[((x >> 58) % 64) as usize]
            })
            .collect()
    }

    #[test]
    fn blob_runs_never_hold_whitespace() {
        // The entropy window trails the cursor, so it stays blob-dominated for
        // up to 64 bytes past a blob's end. A run must still stop at the
        // separator rather than continuing into the text that follows.
        //
        // The blob is 300 bytes: the window is 64 wide and a run must reach
        // BLOB_MIN_LEN, so a run only forms once the window holds mostly blob.
        // A blob shorter than roughly 112 bytes never sustains one, and is
        // classified by the base64 recognizer instead.
        let mut input = Vec::new();
        for i in 0..40 {
            input.extend_from_slice(format!("line {i} the quick brown fox jumps over\n").as_bytes());
            input.extend(hi_entropy_blob(300));
            input.push(b'\n');
        }
        let runs = high_entropy_runs(&input, BLOB_ENTROPY_PCT, BLOB_MIN_LEN);
        assert!(!runs.is_empty(), "blobs should be detected at all");
        for &(a, b) in &runs {
            assert!(
                !input[a..b].iter().any(u8::is_ascii_whitespace),
                "run [{a}..{b}] holds whitespace: {:?}",
                String::from_utf8_lossy(&input[a..b])
            );
        }
        // The prose after a blob still lexes into its own word tokens.
        let toks = crate::lexer::lex(&input);
        let words = toks
            .iter()
            .filter(|t| t.kind == crate::token::TokenKind::Word)
            .filter(|t| &input[t.span()] == b"quick")
            .count();
        assert_eq!(words, 40, "every line's prose survives the blob gate");
    }

    #[test]
    fn high_entropy_runs_find_a_base64_blob_not_prose() {
        let mut input = Vec::new();
        input.extend_from_slice(b"the quick brown fox jumps over the lazy dog near the old bridge today ");
        let blob_start = input.len();
        input.extend(hi_entropy_blob(300));
        let runs = high_entropy_runs(&input, BLOB_ENTROPY_PCT, BLOB_MIN_LEN);
        assert!(!runs.is_empty(), "a 300-byte base64 blob should be detected");
        let (a, b) = runs[0];
        assert!(b - a >= BLOB_MIN_LEN, "blob run too short: {:?}", runs[0]);
        assert!(a >= blob_start.saturating_sub(2), "blob should start at/after the prose: {a} vs {blob_start}");
    }

    /// The runs read from every byte's flag: what the span-gated pass must
    /// return.
    fn runs_from_every_flag(input: &[u8], threshold_pct: u8, min_len: usize) -> Vec<(usize, usize)> {
        let n = input.len();
        if n < min_len {
            return Vec::new();
        }
        let mut raw: Vec<(usize, usize)> = Vec::new();
        let mut run_start: Option<usize> = None;
        entropy_flags(input, 0, n, threshold_pct, |i, hi| match (hi, run_start) {
            (true, None) => run_start = Some(i),
            (false, Some(rs)) => {
                if i - rs >= min_len {
                    raw.push(whitespace_free_span(input, rs, i));
                }
                run_start = None;
            }
            _ => {}
        });
        if let Some(rs) = run_start
            && n - rs >= min_len
        {
            raw.push(whitespace_free_span(input, rs, n));
        }
        merge_spans(raw)
    }

    /// The spans found across leaves are the spans the pass over the whole
    /// input finds, with leaves small enough that spans cross the cuts many
    /// times over and at every length that decides a span.
    #[test]
    fn long_spans_across_leaves_are_the_one_pass_spans() {
        // This file's own bytes: real code, whitespace every few bytes, a few
        // runs long enough to be spans at the blob length.
        let text: &[u8] = include_bytes!("spectral.rs");
        let n = text.len();
        for min_len in [1usize, 3, 8, BLOB_MIN_LEN, 200] {
            let want = long_spans(text, 0, n, min_len);
            assert!(!want.is_empty() || min_len == 200, "min_len {min_len}: the file holds spans that long");
            for least in [1usize, 5, 64, 1000, 4096, ENTROPY_FLAGS_MIN_LEAF] {
                assert_eq!(long_spans_across(text, min_len, least), want, "min_len {min_len}, {least} bytes a leaf");
            }
        }
    }

    /// The runs found across leaves are the runs the pass over whole spans
    /// finds, with leaves small enough that a span is cut many times and a
    /// piece's reach ends inside runs, over this file's bytes and over blobs
    /// up to far longer than a leaf.
    #[test]
    fn runs_over_spans_across_leaves_are_the_whole_span_runs() {
        let text: &[u8] = include_bytes!("spectral.rs");
        let mut blobbed = Vec::new();
        for i in 0..12 {
            blobbed.extend_from_slice(b"the quick brown fox jumps over the lazy dog ");
            blobbed.extend(hi_entropy_blob(3000 + 700 * i));
            blobbed.push(b' ');
        }
        blobbed.extend(hi_entropy_blob(200_000));
        blobbed.push(b'\n');
        blobbed.extend_from_slice(text);
        for (name, input) in [("this file", text), ("blobs then this file", blobbed.as_slice())] {
            for min_len in [16usize, BLOB_MIN_LEN, 200] {
                let spans = long_spans(input, 0, input.len(), min_len);
                let want = runs_over_spans(input, &spans, BLOB_ENTROPY_PCT, min_len);
                assert!(
                    !want.is_empty() || name == "this file",
                    "{name}, min_len {min_len}: the blobs are runs to find"
                );
                for least in [1usize, 7, 100, 4096, ENTROPY_RUNS_MIN_LEAF] {
                    assert_eq!(
                        runs_over_spans_across(input, &spans, BLOB_ENTROPY_PCT, min_len, least),
                        want,
                        "{name}, min_len {min_len}, {least} bytes a leaf"
                    );
                }
            }
        }
    }

    #[test]
    fn long_spans_are_the_bytes_whose_span_reaches_min_len() {
        // Every byte of the range is in a reported part exactly when the
        // whitespace-free span holding it is at least min_len long, whether
        // that span begins before the range or ends past it.
        let mut input = Vec::new();
        for len in [1usize, 7, 47, 48, 49, 60, 100, 47, 48, 3] {
            input.extend(std::iter::repeat_n(b'x', len));
            input.push(if len % 2 == 0 { b' ' } else { b'\n' });
        }
        input.extend(std::iter::repeat_n(b'y', 130));
        let n = input.len();
        let in_long = |p: usize| {
            let mut a = p;
            while a > 0 && !input[a - 1].is_ascii_whitespace() {
                a -= 1;
            }
            let mut b = p;
            while b < n && !input[b].is_ascii_whitespace() {
                b += 1;
            }
            !input[p].is_ascii_whitespace() && b - a >= 48
        };
        for (from, to) in
            [(0, n), (0, 10), (5, 70), (60, 61), (100, 150), (120, n), (n - 20, n), (n - 1, n), (200, 200)]
        {
            let parts = long_spans(&input, from, to, 48);
            let mut covered = vec![false; n];
            for &(a, b) in &parts {
                assert!(from <= a && a < b && b <= to, "part [{a}, {b}) outside [{from}, {to})");
                for c in &mut covered[a..b] {
                    *c = true;
                }
            }
            for (p, &c) in covered.iter().enumerate().take(to).skip(from) {
                assert_eq!(c, in_long(p), "byte {p} of [{from}, {to})");
            }
        }
    }

    #[test]
    fn runs_over_long_spans_alone_equal_runs_over_every_byte() {
        // Blobs at both sides of min_len, at the input's ends, split by one
        // space, and buried in prose: the gated pass and a pass over every
        // byte must return the same runs, serially and across cores.
        let mut input = Vec::new();
        input.extend(hi_entropy_blob(300));
        for (i, len) in [40usize, 47, 48, 49, 60, 111, 112, 113, 200, 300].into_iter().enumerate() {
            input.extend_from_slice(b" the quick brown fox jumps over the lazy dog ");
            input.extend(hi_entropy_blob(len));
            if i % 3 == 1 {
                input.push(b' ');
                input.extend(hi_entropy_blob(len));
            }
            input.push(if i % 2 == 0 { b'\n' } else { b'\t' });
        }
        input.extend_from_slice(b"tail text");
        input.extend(hi_entropy_blob(300));
        for min_len in [16usize, 48, 64, 200] {
            assert_eq!(
                high_entropy_runs(&input, BLOB_ENTROPY_PCT, min_len),
                runs_from_every_flag(&input, BLOB_ENTROPY_PCT, min_len),
                "min_len {min_len}"
            );
        }

        // An input with no whitespace anywhere is one span, and every run
        // inside it expands to the same bounds. The expansion walks a byte at
        // a time to the nearest whitespace either side, so here it reaches the
        // input's own ends: reading it once a run rather than once a span made
        // the pass quadratic in the number of runs, and 19 MB of this shape
        // took minutes where the same bytes carrying newlines took
        // milliseconds.
        let mut unbroken = Vec::new();
        while unbroken.len() < 64 * 1024 {
            unbroken.extend(hi_entropy_blob(64));
            unbroken.extend_from_slice(b"plain.text.between.the.blobs");
        }
        assert!(
            !unbroken.iter().any(u8::is_ascii_whitespace),
            "the corpus must hold no whitespace, or it does not read this case"
        );
        for min_len in [16usize, 48, 200] {
            assert_eq!(
                high_entropy_runs(&unbroken, BLOB_ENTROPY_PCT, min_len),
                runs_from_every_flag(&unbroken, BLOB_ENTROPY_PCT, min_len),
                "no whitespace anywhere, min_len {min_len}"
            );
        }
        let mut big = Vec::new();
        while big.len() < 4 * ENTROPY_FLAGS_MIN_LEAF {
            big.extend_from_slice(&input);
        }
        assert_eq!(
            high_entropy_runs_parallel(&big, BLOB_ENTROPY_PCT, BLOB_MIN_LEN),
            runs_from_every_flag(&big, BLOB_ENTROPY_PCT, BLOB_MIN_LEN)
        );
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    #[allow(unsafe_code)]
    fn entropy_flags_ladder_agrees() {
        // Four rungs compiled from one body, so a divergence would mean the
        // instruction set changed the answer rather than the speed. Each is
        // run only where this CPU supports it; a rung the host cannot run
        // is reported as skipped rather than counted as agreeing, because a
        // ladder that silently tests one rung is a ladder nobody checked.
        let mut input = Vec::new();
        for i in 0..60 {
            input.extend_from_slice(b"the quick brown fox jumps over the lazy dog ");
            if i % 5 == 2 {
                input.extend(hi_entropy_blob(48 + i));
                input.push(b' ');
            }
        }
        /// One entropy-flag implementation: the input, the span, the
        /// threshold, and the sink each flagged position goes to.
        type FlagPass = dyn Fn(&[u8], usize, usize, u8, &mut dyn FnMut(usize, bool));
        let flags = |f: &FlagPass| {
            let mut v = Vec::new();
            f(&input, 0, input.len(), BLOB_ENTROPY_PCT, &mut |i, h| v.push((i, h)));
            v
        };
        let scalar = flags(&|inp, a, b, t, e| entropy_flags_impl(inp, a, b, t, e));
        assert!(!scalar.is_empty(), "the corpus must produce flags, or this asserts nothing");
        assert!(scalar.iter().any(|&(_, h)| h), "some byte must flag high, or this is vacuous");

        let mut ran = vec!["scalar"];
        if std::is_x86_feature_detected!("sse2") {
            // SAFETY: probed immediately above.
            let got = flags(&|inp, a, b, t, e| unsafe { entropy_flags_sse2(inp, a, b, t, e) });
            assert_eq!(got, scalar, "sse2 rung disagrees with the scalar body");
            ran.push("sse2");
        }
        if std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("bmi2") {
            let got = flags(&|inp, a, b, t, e| unsafe { entropy_flags_avx2(inp, a, b, t, e) });
            assert_eq!(got, scalar, "avx2 rung disagrees with the scalar body");
            ran.push("avx2");
        }
        if std::is_x86_feature_detected!("avx512f") && std::is_x86_feature_detected!("avx512bw") {
            let got = flags(&|inp, a, b, t, e| unsafe { entropy_flags_avx512(inp, a, b, t, e) });
            assert_eq!(got, scalar, "avx512 rung disagrees with the scalar body");
            ran.push("avx512");
        }
        eprintln!("entropy_flags ladder rungs exercised on this host: {}", ran.join(", "));
    }

    /// The flag pass written as the entropy expression per byte: the same
    /// window sum, converted through the same float steps at every position.
    /// The production pass compares a full window's sum to a boundary found
    /// once from this expression, and must agree with it flag for flag.
    /// The window's threshold rises with the share of its bytes inside
    /// well-formed multi-byte characters; `wide` false reads the threshold
    /// alone, which is what a pass blind to UTF-8 would flag.
    fn entropy_flags_by_expression(input: &[u8], threshold_pct: u8, wide: bool) -> Vec<(usize, bool)> {
        let w = 64usize;
        let mut hist = [0u32; 256];
        let mut s = 0i64;
        let entropy_norm = (w.min(256) as f32).max(2.0).log2();
        let thr = f32::from(threshold_pct) / 100.0;
        let lut = fixed_log_table();
        let inside = bytes_inside_characters(input);
        let mut out = Vec::with_capacity(input.len());
        for i in 0..input.len() {
            let b = input[i] as usize;
            let nb = hist[b];
            hist[b] = nb + 1;
            s += fixed_term(lut, nb + 1) - fixed_term(lut, nb);
            if i >= w {
                let ob = input[i - w] as usize;
                let oc = hist[ob];
                hist[ob] = oc - 1;
                s -= fixed_term(lut, oc) - fixed_term(lut, oc - 1);
            }
            let nwin = (i + 1).min(w);
            let share = if wide { inside[i + 1 - nwin..=i].iter().filter(|&&x| x).count() as f32 / nwin as f32 } else { 0.0 };
            let nw = f64::from(nwin as u32);
            let h = (nw.log2() - s as f64 * (1.0 / (nw * FIXED_LOG_SCALE))) as f32 / entropy_norm;
            let gate = if share == 0.0 { thr } else { thr + (1.0 - thr) * share };
            out.push((i, h >= gate && !input[i].is_ascii_whitespace()));
        }
        out
    }

    /// Which bytes of `input` are inside a well-formed multi-byte character,
    /// read by the standard library's own UTF-8 decoder.
    fn bytes_inside_characters(input: &[u8]) -> Vec<bool> {
        let mut out = vec![false; input.len()];
        let mut at = 0;
        for chunk in input.utf8_chunks() {
            for (o, ch) in chunk.valid().char_indices() {
                if ch.len_utf8() > 1 {
                    out[at + o..at + o + ch.len_utf8()].fill(true);
                }
            }
            at += chunk.valid().len() + chunk.invalid().len();
        }
        out
    }

    /// Text in scripts written in multi-byte characters, with the ways UTF-8
    /// breaks: a character cut short, an overlong form, a surrogate, a stray
    /// continuation byte, and a four-byte character.
    fn multibyte_text() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(CHINESE.as_bytes());
        v.extend_from_slice(b" \xE4\xB8 \xC0\x80 \xED\xA0\x80 \x80\xBF ");
        v.extend_from_slice(BULGARIAN.as_bytes());
        v.extend_from_slice(" \u{1F600}\u{1F680} ".as_bytes());
        v.extend_from_slice(JAPANESE.as_bytes());
        v
    }

    const BULGARIAN: &str = "Правителството обяви нови мерки за подкрепа на малкия бизнес и домакинствата, като министърът на финансите увери, че средствата ще бъдат изплатени до края на месеца. Опозицията поиска повече подробности за разходите.";
    const GREEK: &str = "Η κυβέρνηση ανακοίνωσε νέα μέτρα στήριξης για τις μικρές επιχειρήσεις και τα νοικοκυριά, ενώ ο υπουργός Οικονομικών διαβεβαίωσε ότι τα χρήματα θα καταβληθούν μέχρι το τέλος του μήνα. Η αντιπολίτευση ζήτησε περισσότερες λεπτομέρειες.";
    const CHINESE: &str = "政府宣布了支持小企业和家庭的新措施，财政部长表示，相关资金将在本月底之前发放到位。反对派要求公布更多关于支出的细节，并呼吁议会尽快展开辩论。专家认为，这些措施能够缓解物价上涨带来的压力，但长期效果仍有待观察。";
    const JAPANESE: &str = "政府は中小企業と家庭を支援する新たな対策を発表し、財務大臣は資金が今月末までに支給されると述べた。野党は支出についてさらに詳しい説明を求め、議会での早期の審議を呼びかけた。";

    #[test]
    fn the_full_window_boundary_flags_exactly_as_the_expression_does() {
        // Prose, blobs of every length around the window, runs of one byte,
        // and every byte value in order, so the window sum crosses the
        // threshold from both sides many times and equals it where it can.
        let mut input = Vec::new();
        for i in 0..80 {
            input.extend_from_slice(b"the quick brown fox jumps over the lazy dog ");
            match i % 4 {
                0 => input.extend(hi_entropy_blob(40 + i)),
                1 => input.extend(std::iter::repeat_n(b'x', i)),
                2 => input.extend((0..=255u8).skip(i % 7)),
                _ => input.extend(hi_entropy_blob(16).iter().chain(b"aaaa").copied()),
            }
            input.push(b' ');
            if i % 9 == 4 {
                input.extend(multibyte_text());
                input.push(b' ');
            }
        }
        for pct in [50u8, 70, BLOB_ENTROPY_PCT, 95, 100] {
            let want = entropy_flags_by_expression(&input, pct, true);
            let mut got = Vec::new();
            entropy_flags(&input, 0, input.len(), pct, |i, h| got.push((i, h)));
            assert_eq!(got, want, "threshold {pct}: the pass disagrees with the expression");
            let highs = want.iter().filter(|(_, h)| *h).count();
            assert!(pct == 100 || (highs > 0 && highs < want.len()), "threshold {pct} flags {highs} of {}, so the comparison asserts little", want.len());
        }
    }

    #[test]
    fn entropy_flags_from_mid_stream_read_what_the_whole_stream_reads() {
        // Prose with blobs dropped in, so the window crosses every kind of
        // edge; a pass started at any offset must flag each byte exactly as
        // the pass from the start does, which is the fact the parallel form
        // rests on.
        let mut input = Vec::new();
        for i in 0..40 {
            input.extend_from_slice(b"the quick brown fox jumps over the lazy dog ");
            if i % 7 == 3 {
                input.extend(hi_entropy_blob(60 + i));
                input.push(b' ');
            }
            if i % 11 == 5 {
                input.extend(multibyte_text());
                input.push(b' ');
            }
        }
        let n = input.len();
        let mut whole = vec![false; n];
        entropy_flags(&input, 0, n, BLOB_ENTROPY_PCT, |i, h| whole[i] = h);
        // Starts inside a multi-byte character, one per byte of the
        // character, besides the fixed offsets.
        let first_wide = input.iter().position(|&b| b >= 0xE0).expect("the text holds three-byte characters");
        for from in [1, 17, 63, 64, 65, 200, 777, n / 2, n - 70, n - 1, first_wide + 1, first_wide + 2, first_wide + 64, first_wide + 66] {
            let mut part = vec![false; n];
            entropy_flags(&input, from, n, BLOB_ENTROPY_PCT, |i, h| part[i] = h);
            assert_eq!(&part[from..], &whole[from..], "flags from {from}");
        }
        // The parallel runs over an input long enough to split are the
        // serial runs.
        let mut big = Vec::new();
        while big.len() < 4 * ENTROPY_FLAGS_MIN_LEAF {
            big.extend_from_slice(&input);
        }
        assert_eq!(
            high_entropy_runs_parallel(&big, BLOB_ENTROPY_PCT, BLOB_MIN_LEN),
            high_entropy_runs(&big, BLOB_ENTROPY_PCT, BLOB_MIN_LEN),
        );
    }

    #[test]
    fn well_formed_multibyte_text_is_never_a_blob_run() {
        // Lines of a real code of conduct in Chinese. The threshold alone
        // flags its middle sentence: a run past BLOB_MIN_LEN clears it, or the
        // test tests nothing.
        let zh = "份。\n可以通过 community@hasura.io 联系项目团队来报告辱骂、骚扰或其他不可接受的行为。\n所有投诉都将得到审查和调查".as_bytes();
        let blind = entropy_flags_by_expression(zh, BLOB_ENTROPY_PCT, false);
        let longest = blind.split(|&(_, h)| !h).map(<[(usize, bool)]>::len).max().unwrap_or(0);
        assert!(longest >= BLOB_MIN_LEN, "a run of {longest} bytes clears the bare threshold, so this asserts nothing");
        assert_eq!(high_entropy_runs(zh, BLOB_ENTROPY_PCT, BLOB_MIN_LEN), Vec::new(), "the paragraph is text");
        assert!(crate::lexer::lex(zh).iter().all(|t| t.kind != crate::token::TokenKind::Other), "no opaque token");
        eprintln!("longest run the bare threshold flags in the paragraph: {longest} bytes");
        // Bytes not part of a well-formed character stay a blob, and so does
        // base64.
        let packed = random_bytes(300);
        assert_eq!(high_entropy_runs(&packed, BLOB_ENTROPY_PCT, BLOB_MIN_LEN), vec![(0, packed.len())]);
        let b64 = hi_entropy_blob(300);
        assert_eq!(high_entropy_runs(&b64, BLOB_ENTROPY_PCT, BLOB_MIN_LEN), vec![(0, b64.len())]);
    }

    /// `n` pseudo-random bytes over the whole byte range but ASCII
    /// whitespace, so they make one run: packed or compressed data.
    fn random_bytes(n: usize) -> Vec<u8> {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut v = Vec::with_capacity(n);
        while v.len() < n {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let b = (x >> 56) as u8;
            if !b.is_ascii_whitespace() {
                v.push(b);
            }
        }
        v
    }

    #[test]
    fn prose_in_cyrillic_greek_and_cjk_reads_as_prose() {
        for (name, text) in [("bulgarian", BULGARIAN), ("greek", GREEK), ("chinese", CHINESE), ("japanese", JAPANESE)] {
            let sig = analyze(text.as_bytes()).signature(0, text.len());
            assert_eq!(texture_of(&sig), Texture::Prose, "{name}: {sig:?}");
            assert_eq!(code_texture(&sig), CodeTexture::Prose, "{name}: {sig:?}");
        }
        // The Chinese bytes carry more entropy than the code gate allows a
        // text of single bytes, so the share of characters is what passes it.
        let zh = analyze(CHINESE.as_bytes()).signature(0, CHINESE.len());
        assert!(zh.entropy > 0.78, "entropy {} is under the gate, so this asserts nothing", zh.entropy);
        // Packed bytes stay data.
        let bin = random_bytes(400);
        let sig = analyze(&bin).signature(0, bin.len());
        assert_eq!(texture_of(&sig), Texture::Data);
        assert_eq!(code_texture(&sig), CodeTexture::Blob);
    }

    #[test]
    fn a_mark_of_several_bytes_weighs_like_its_ascii_counterpart() {
        let typographic = "\u{201C}We\u{2019}ll ship it,\u{201D} she said \u{2014} and they did, though the report\u{2019}s authors \u{201C}doubted\u{201D} it would last the winter.";
        let ascii = "\"We'll ship it,\" she said - and they did, though the report's authors \"doubted\" it would last the winter.";
        let punct = |s: &str| {
            let m = analyze(s.as_bytes()).signature(0, s.len()).texture_mix();
            m[3] / m.iter().sum::<f32>()
        };
        // Each mark spelled as three ASCII marks: what counting a mark a unit a
        // byte would read.
        let tripled = "\"\"\"We'''ll ship it,\"\"\" she said --- and they did, though the report'''s authors \"\"\"doubted\"\"\" it would last the winter.";
        let (t, a, x) = (punct(typographic), punct(ascii), punct(tripled));
        eprintln!("punctuation share: typographic {t}, ascii {a}, tripled {x}");
        assert!((t - a).abs() * 3.0 < (x - a).abs(), "punctuation share {t} against {a}, where a unit a byte reads {x}");
    }

    #[test]
    fn the_utf8_reader_releases_each_character_whole_and_broken_bytes_as_class_4() {
        let mut r = Utf8Classes::default();
        let mut got = Vec::new();
        for &b in b"a\xC3\xA9\xE2\x82\xAC\xF0\x9F\x98\x80\xFF\xE4\xB8x" {
            got.extend(r.push(b).into_iter().flatten());
        }
        let rel = |class, count, age| Release { class, count, age };
        assert_eq!(
            got,
            [
                rel(1, 1, 0),
                rel(1, 2, 0),
                rel(3, 3, 0),
                rel(3, 4, 0),
                rel(4, 1, 0),
                rel(4, 2, 1),
                rel(1, 1, 0),
            ],
            "a, e-acute, euro sign, emoji, 0xFF, a character cut short by x"
        );
        r.push(0xE4);
        assert_eq!(r.finish(), Some(rel(4, 1, 0)), "a character the input ends inside");
        assert_eq!(char_class('\u{3002}'), 3, "an ideographic full stop is punctuation");
        assert_eq!(char_class('\u{00A0}'), 2, "a no-break space is a space");
        assert_eq!(char_class('\u{0301}'), 1, "a combining accent belongs to its letter");
    }

    #[test]
    fn a_short_span_of_code_mid_sentence_gets_an_entry_and_an_exit() {
        let text = "The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.";
        let entry = text.find("a[i]=").expect("the span");
        let exit = text.find(" on the payload").expect("the span's end");
        let cuts = analyze(text.as_bytes()).boundaries;
        assert!(cuts.iter().any(|&c| (entry..entry + 16).contains(&c)), "an entry near {entry}: {cuts:?}");
        // The exit is at the first of the twelve bytes the reading has
        // stayed back, so it is within twelve bytes of the span's end.
        assert!(cuts.iter().any(|&c| (exit..exit + 12).contains(&c)), "an exit near {exit}: {cuts:?}");
    }

    #[test]
    fn a_plain_sentence_holds_no_change_point() {
        // No boundary is placed before the clocks fill, and the threshold's
        // statistics take every deviation whole until then, so the first
        // byte a boundary may be at finds the threshold caught up.
        for text in [
            "The nightly report lists every host that answered within the window and the latency it measured for each one.",
            "The walk reports what it found rather than what it was asked for, and a filter that silently drops a file reads the same as a directory that never held one.",
        ] {
            assert_eq!(analyze(text.as_bytes()).boundaries, Vec::<usize>::new(), "{text}");
        }
    }

    #[test]
    fn regions_surface_distinct_textures() {
        // Prose then a high-bit binary blob: the class flip (alpha prose ->
        // high-byte data) plus the entropy rise reliably separates them into
        // distinct regions, the blob reading as Data.
        let mut doc = Vec::new();
        for _ in 0..3 {
            doc.extend_from_slice(b"the quick brown fox jumps over the lazy dog near the old stone bridge today ");
        }
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..300 {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            doc.push(0x80 | ((x >> 56) as u8 & 0x7f)); // 0x80..=0xFF, always high-bit
        }
        let field = analyze(&doc);
        let regs = regions(&field);
        let textures: std::collections::HashSet<Texture> = regs.iter().map(|r| r.2).collect();
        assert!(regs.len() >= 2, "expected multiple regions, got {regs:?}");
        assert!(textures.contains(&Texture::Data), "the blob region should read Data: {regs:?}");
    }
}
