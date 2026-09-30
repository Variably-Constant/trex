//! The magnitude axis - trex's scale substrate, and the energy / gradient stack
//! that derives from it.
//!
//! Every other axis discards the scale of a value. A `Number` token is just a
//! span (`src/token.rs` carries no parsed value), so `5` and `5000000000` are
//! identical to the lexer, to `shape` (both `Number`, same silhouette), to
//! `spectral` (both digit-textured), and to `orbit` (both their own identity).
//! The magnitude axis reads the one thing they all drop: *how big*.
//!
//! It is an INTENSIVE scalar field `m(t)` over the token stream - the order of
//! magnitude of each token's natural scalar (a Number's numeric value, any
//! other token's byte-size). Two physical quantities derive from it by calculus,
//! exactly as energy and momentum derive from a field in physics:
//!
//! - **energy** = the running `sum(m^2)` over a window - the extensive
//!   accumulated intensity, locating the "heavy" regions where scale
//!   concentrates (and `total_energy`, the whole-stream integral);
//! - **gradient** = `m[i] - m[i-1]` - the per-token derivative, the directional
//!   FLOW (a steady ramp, a settling, a discontinuity).
//!
//! A **jump** is a gradient spike past a threshold - a scale discontinuity (a
//! `5` next to a `5000000000`) no other axis can see. Documented in
//! `wiki/content/docs/reference/axes/magnitude.md`.

use crate::token::{Token, TokenKind};

/// Knobs for the magnitude reader. `Default` suits general token input.
#[derive(Clone, Copy, Debug)]
pub struct MagnitudeConfig {
    /// Trailing token window the local energy `sum(m^2)` runs over.
    pub energy_window: usize,
    /// `|gradient|` past this (in orders of magnitude) is a scale discontinuity.
    pub jump_threshold: f32,
    /// A token whose magnitude is this many standard deviations from the stream
    /// mean is a scale outlier.
    pub outlier_sigma: f32,
}

impl Default for MagnitudeConfig {
    fn default() -> Self {
        Self {
            energy_window: 16,
            jump_threshold: 3.0,
            outlier_sigma: 2.5,
        }
    }
}

/// One token's reading on the magnitude stack.
#[derive(Clone, Copy, Debug, Default)]
pub struct MagnitudeFrame {
    /// `m(t)`: the order of magnitude of the token's natural scalar (a Number's
    /// `log10(|value|)`, any other token's `log2(byte_len)`).
    pub magnitude: f32,
    /// `dm/dt`: the change from the previous token's magnitude (the flow).
    pub gradient: f32,
    /// Local energy: `sum(m^2)` over the trailing `energy_window` tokens.
    pub energy: f32,
}

/// The scale side table, keyed by byte offset (like `SpectralField` /
/// `ShapeField`).
#[derive(Clone, Debug, Default)]
pub struct MagnitudeField {
    /// Token count.
    pub n_tokens: usize,
    /// Byte span per token - the byte-offset key.
    pub spans: Vec<(usize, usize)>,
    /// One frame per token.
    pub frames: Vec<MagnitudeFrame>,
    /// Scale-discontinuity byte offsets (`|gradient|` past the threshold).
    pub jumps: Vec<usize>,
    /// The whole-stream energy integral `sum(m^2)` - the extensive total.
    pub total_energy: f32,
    /// Stream magnitude mean and standard deviation (for outlier queries).
    mean: f32,
    std: f32,
    outlier_sigma: f32,
}

impl MagnitudeField {
    /// The token index covering `byte`, or `None` if the field is empty.
    fn token_at(&self, byte: usize) -> Option<usize> {
        if self.spans.is_empty() {
            return None;
        }
        let i = self.spans.partition_point(|&(s, _)| s <= byte);
        Some(i.saturating_sub(1))
    }

    /// The magnitude `m(t)` of the token covering `byte` (`0.0` if empty).
    #[must_use]
    pub fn magnitude_at(&self, byte: usize) -> f32 {
        self.token_at(byte)
            .and_then(|i| self.frames.get(i))
            .map_or(0.0, |f| f.magnitude)
    }

    /// The gradient `dm/dt` at `byte`.
    #[must_use]
    pub fn gradient_at(&self, byte: usize) -> f32 {
        self.token_at(byte)
            .and_then(|i| self.frames.get(i))
            .map_or(0.0, |f| f.gradient)
    }

    /// The energy `sum(m^2)` over `[start, end)` - the extensive intensity of a
    /// span, computed directly from the per-token magnitudes it covers.
    #[must_use]
    pub fn energy_of(&self, start: usize, end: usize) -> f32 {
        self.spans
            .iter()
            .zip(&self.frames)
            .filter(|((s, e), _)| *s < end && *e > start)
            .map(|(_, f)| f.magnitude * f.magnitude)
            .sum()
    }

    /// The scale outliers: token indices whose magnitude is more than
    /// `outlier_sigma` standard deviations from the stream mean. The anomalies
    /// no other axis can see - a huge value among small ones.
    #[must_use]
    pub fn outliers(&self) -> Vec<usize> {
        if self.std <= f32::EPSILON {
            return Vec::new();
        }
        self.frames
            .iter()
            .enumerate()
            .filter(|(_, f)| ((f.magnitude - self.mean) / self.std).abs() >= self.outlier_sigma)
            .map(|(i, _)| i)
            .collect()
    }
}

/// The numeric value of a Number token's text, handling underscores, hex
/// (`0x..`), and scientific notation. `None` when nothing parses.
///
/// Most numbers are plain decimal digits, which parse as an integer in
/// place: below 2^53 the double is exact, and above it the decimal parse and
/// the conversion both round the same integer to the nearest double, so the
/// value is the general parse's either way. The copy that drops underscores
/// is made only where one is present.
fn numeric_value(s: &str) -> Option<f64> {
    let s = s.trim();
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        match s.parse::<u64>() {
            Ok(v) => return Some(v as f64),
            // A run of digits too long for a u64 is a number all the same,
            // and the float parse below is what reads it.
            Err(too_long) => {
                debug_assert_eq!(*too_long.kind(), std::num::IntErrorKind::PosOverflow, "{s:?}");
            }
        }
    }
    let t: std::borrow::Cow<'_, str> = if s.contains('_') {
        std::borrow::Cow::Owned(s.chars().filter(|&c| c != '_').collect())
    } else {
        std::borrow::Cow::Borrowed(s)
    };
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).ok().map(|v| v as f64);
    }
    t.parse::<f64>().ok()
}

/// `log2` of every byte length up to the table's, filled once from the
/// expression the fall-through evaluates, so a length inside it reads the
/// value it would have computed. The table holds the lengths of 99.9% of the
/// tokens of every corpus read for it - tokens average three to five bytes,
/// and the ones past 256 are 0.003% of a 3.7 MB code corpus's and 0.056% of a
/// 12 MB text one's - and the rest take the call.
static LOG2_OF_LENGTH: std::sync::LazyLock<[f32; 257]> =
    std::sync::LazyLock::new(|| std::array::from_fn(|n| (n.max(1) as f32).log2()));

/// `m(t)` for one token: a Number's decimal order of magnitude, any other
/// token's binary order of magnitude (its byte-size on a log2 scale).
#[must_use]
pub fn token_magnitude(kind: TokenKind, tok_bytes: &[u8]) -> f32 {
    match kind {
        TokenKind::Number => {
            let s = String::from_utf8_lossy(tok_bytes);
            match numeric_value(&s) {
                Some(v) if v.abs() > 0.0 => v.abs().log10() as f32,
                _ => 0.0,
            }
        }
        _ => {
            let n = tok_bytes.len();
            match LOG2_OF_LENGTH.get(n) {
                Some(&m) => m,
                None => (n as f32).log2(),
            }
        }
    }
}

/// Analyse a token stream with the default configuration.
#[must_use]
pub fn analyze(tokens: &[Token], bytes: &[u8]) -> MagnitudeField {
    analyze_with(tokens, bytes, &MagnitudeConfig::default())
}

/// Tokenize `bytes` (the significant-token lexer) and analyse - the convenience
/// path for consumers that hold only bytes.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> MagnitudeField {
    let toks = crate::tokutil::lex_sig(bytes);
    analyze(&toks, bytes)
}

/// The full one-pass reader: magnitude, its derivative (gradient), and its
/// windowed integral (energy), plus jumps and outlier statistics.
#[must_use]
pub fn analyze_with(tokens: &[Token], bytes: &[u8], cfg: &MagnitudeConfig) -> MagnitudeField {
    let n = tokens.len();
    let mut field = MagnitudeField {
        n_tokens: n,
        spans: Vec::with_capacity(n),
        frames: Vec::with_capacity(n),
        jumps: Vec::new(),
        total_energy: 0.0,
        mean: 0.0,
        std: 0.0,
        outlier_sigma: cfg.outlier_sigma,
    };
    if n == 0 {
        return field;
    }

    // m(t): the magnitude scalar field.
    let mags: Vec<f32> = tokens
        .iter()
        .map(|t| token_magnitude(t.kind, &bytes[t.span()]))
        .collect();
    for t in tokens {
        field.spans.push((t.start(), t.end()));
    }

    // mean / std of the field (for outlier queries) + the total energy integral.
    let sum: f32 = mags.iter().copied().sum();
    let mean = sum / n as f32;
    let var: f32 = mags.iter().map(|&m| (m - mean) * (m - mean)).sum::<f32>() / n as f32;
    field.mean = mean;
    field.std = var.sqrt();
    field.total_energy = mags.iter().map(|&m| m * m).sum();

    let mut frames: Vec<MagnitudeFrame> = Vec::with_capacity(n);
    for i in 0..n {
        // gradient dm/dt: change from the previous token.
        let gradient = if i > 0 { mags[i] - mags[i - 1] } else { 0.0 };

        // a jump: |gradient| past the threshold is a scale discontinuity.
        if i > 0 && gradient.abs() >= cfg.jump_threshold {
            field.jumps.push(tokens[i].start());
        }

        // local energy: sum(m^2) over the trailing window.
        let lo = i.saturating_sub(cfg.energy_window.saturating_sub(1));
        let energy: f32 = mags[lo..=i].iter().map(|&m| m * m).sum();

        frames.push(MagnitudeFrame {
            magnitude: mags[i],
            gradient,
            energy,
        });
    }
    field.frames = frames;
    field
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(s: &str) -> MagnitudeField {
        analyze_bytes(s.as_bytes())
    }

    #[test]
    fn scale_outlier_is_detected() {
        // A huge value among small ones - invisible to every other axis (all
        // are `Number` tokens), a clear magnitude outlier + jump here.
        let f = field("x = 5 ; y = 6 ; z = 5000000000 ; w = 7");
        assert!(!f.jumps.is_empty(), "the 5 -> 5e9 jump should register");
        assert!(
            !f.outliers().is_empty(),
            "the huge value should be a scale outlier"
        );
    }

    #[test]
    fn magnitude_ramp_has_steady_positive_gradient() {
        // Orders of magnitude climbing: 1 10 100 1000 10000 - the gradient is
        // steadily positive (a ramp in log space).
        let f = field("1 10 100 1000 10000 100000");
        let num_grads: Vec<f32> = f
            .frames
            .iter()
            .filter(|fr| fr.gradient.abs() > 0.01)
            .map(|fr| fr.gradient)
            .collect();
        assert!(
            !num_grads.is_empty() && num_grads.iter().all(|&g| g > 0.0),
            "a magnitude ramp should show positive gradient, got {num_grads:?}"
        );
    }

    #[test]
    fn energy_is_higher_in_the_heavy_region() {
        // A region of large numbers carries more energy than small ones.
        let f = field("1 2 3 4 999999999 888888888 777777777 666666666");
        let light = f.energy_of(0, 7); // "1 2 3 4" region (small)
        let heavy = f.energy_of(8, 50); // the big-number region
        assert!(
            heavy > light,
            "the large-number region should carry more energy ({heavy}) than the small ({light})"
        );
    }

    /// Every length reads the magnitude the expression the table was filled
    /// from gives it, through the table's whole range and past its end.
    #[test]
    fn a_length_s_magnitude_is_its_log2_whether_the_table_holds_it_or_not() {
        let bytes = vec![b'x'; 2048];
        for n in 0..=2048usize {
            let got = token_magnitude(TokenKind::Word, &bytes[..n]);
            let want = (n.max(1) as f32).log2();
            assert_eq!(got.to_bits(), want.to_bits(), "length {n}");
        }
    }

    /// A number's value whichever path parses it: plain digits, digits past
    /// 2^53 and past u64, underscores, hex in either case, signs, fractions,
    /// exponents, the two special floats, and text no number is.
    #[test]
    fn a_number_s_value_is_the_same_whichever_path_parses_it() {
        let cases: [(&str, Option<f64>); 24] = [
            ("0", Some(0.0)),
            ("7", Some(7.0)),
            ("007", Some(7.0)),
            (" 42 ", Some(42.0)),
            ("1000", Some(1000.0)),
            ("1_000_000", Some(1_000_000.0)),
            ("9007199254740993", Some(9_007_199_254_740_992.0)),
            ("18446744073709551615", Some(18_446_744_073_709_551_615.0)),
            ("18446744073709551616", Some(18_446_744_073_709_551_616.0)),
            ("99999999999999999999999", Some(1e23)),
            ("3.25", Some(3.25)),
            ("-2", Some(-2.0)),
            ("+7", Some(7.0)),
            ("1e5", Some(1e5)),
            ("2.5E-3", Some(2.5e-3)),
            ("0x1F", Some(31.0)),
            ("0X1f", Some(31.0)),
            ("0xffff_ffff", Some(4_294_967_295.0)),
            ("inf", Some(f64::INFINITY)),
            ("", None),
            ("abc", None),
            ("1..2", None),
            ("١٢", None),
            ("0x", None),
        ];
        for (text, want) in cases {
            assert_eq!(numeric_value(text).map(f64::to_bits), want.map(f64::to_bits), "{text:?}");
        }
        assert!(numeric_value("nan").is_some_and(f64::is_nan));
    }

    #[test]
    fn number_value_drives_magnitude_not_length() {
        // `1000000` (1e6) outranks `9` even though both are short-ish: the
        // The value drives magnitude, which is the whole point.
        let f = field("9 1000000");
        assert!(
            f.frames[1].magnitude > f.frames[0].magnitude + 4.0,
            "1e6 should be ~6 orders, 9 ~0.95"
        );
    }

    #[test]
    fn empty_is_safe() {
        let f = field("");
        assert_eq!(f.n_tokens, 0);
        assert!(f.frames.is_empty());
        assert!(f.jumps.is_empty());
        assert!(f.outliers().is_empty());
        assert_eq!(f.magnitude_at(0), 0.0);
    }
}
