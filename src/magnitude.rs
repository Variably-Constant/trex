//! The magnitude axis: each token's order of magnitude, with its energy and
//! gradient.
//!
//! Every other axis discards the scale of a value. A `Number` token is just a
//! span (`src/token.rs` carries no parsed value), so `5` and `5000000000` are
//! identical to the lexer, to `shape` (both `Number`, same silhouette), to
//! `spectral` (both made of digits), and to `orbit` (each its own orbit).
//! The magnitude axis reads the one thing they all drop: how big a value is.
//!
//! Each token's magnitude `m(t)` is `log10` of a Number's value, and `log2`
//! of any other token's length in bytes. Two readings are computed from it:
//!
//! - **energy**, the sum of `m^2` over a trailing window, which is high where
//!   large values cluster (and `total_energy`, the sum over the whole stream);
//! - **gradient**, `m[i] - m[i-1]`, the change from one token to the next (a
//!   steady ramp, a settling, a jump).
//!
//! A **jump** is a token whose gradient is at least a threshold in size: a
//! change of scale (a `5` next to a `5000000000`) no other axis can see.
//! Documented in `wiki/content/docs/reference/axes/magnitude.md`.

use crate::token::{Token, TokenKind};

/// Settings for the magnitude reader. `Default` suits general token input.
#[derive(Clone, Copy, Debug)]
pub struct MagnitudeConfig {
    /// How many trailing tokens the local energy `sum(m^2)` adds up.
    pub energy_window: usize,
    /// `|gradient|` at or past this (in orders of magnitude) is a scale
    /// discontinuity.
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

/// One token's magnitude, gradient and energy.
#[derive(Clone, Copy, Debug, Default)]
pub struct MagnitudeFrame {
    /// `m(t)`: a Number's `log10(|value|)`, any other token's
    /// `log2(byte_len)`.
    pub magnitude: f32,
    /// `dm/dt`: the change from the previous token's magnitude.
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
    /// The byte offsets of the jumps (`|gradient|` at or past the threshold).
    pub jumps: Vec<usize>,
    /// The energy `sum(m^2)` over the whole stream.
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

    /// The energy `sum(m^2)` of the tokens that overlap `[start, end)`.
    #[must_use]
    pub fn energy_of(&self, start: usize, end: usize) -> f32 {
        self.spans
            .iter()
            .zip(&self.frames)
            .filter(|((s, e), _)| *s < end && *e > start)
            .map(|(_, f)| f.magnitude * f.magnitude)
            .sum()
    }

    /// The scale outliers: token indices whose magnitude is at least
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

/// The base-ten order of a Number token's value: the logarithm of its
/// magnitude, read from any form the lexer reads as one number (underscores,
/// a radix prefix, an exponent) at any size. `None` for zero and for text
/// that is not a number.
///
/// Most numbers are plain decimal digits, which parse as an integer in place,
/// and digits with a point parse as a float. Every other form, and a value
/// past a float's range, is read by [`crate::typed::number_value`], whose
/// logarithm stays finite at any power of ten, so `1e999999` is 999999 and
/// never infinity.
fn number_order(s: &str) -> Option<f64> {
    let s = s.trim();
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        match s.parse::<u64>() {
            Ok(0) => return None,
            Ok(v) => return Some((v as f64).log10()),
            Err(too_long) => {
                debug_assert_eq!(*too_long.kind(), std::num::IntErrorKind::PosOverflow, "{s:?}");
            }
        }
    }
    if s.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        match s.parse::<f64>() {
            Ok(v) if v.is_finite() && v > 0.0 => return Some(v.log10()),
            Ok(_zero_or_past_a_float) => {}
            Err(_not_a_float) => {}
        }
    }
    crate::typed::number_value(s)?.log10()
}

/// `log2` of every byte length from 0 to 256, filled once from the same
/// expression a longer length computes, so a length in the table gets the
/// value the call would give. The table covers 99.9% of the tokens of every
/// corpus measured for it: tokens average three to five bytes, and the ones
/// past 256 are 0.003% of a 3.7 MB code corpus's and 0.056% of a 12 MB text
/// corpus's. A longer token calls `log2`.
static LOG2_OF_LENGTH: std::sync::LazyLock<[f32; 257]> =
    std::sync::LazyLock::new(|| std::array::from_fn(|n| (n.max(1) as f32).log2()));

/// `m(t)` for one token: a Number's decimal order of magnitude, any other
/// token's binary order of magnitude (its byte-size on a log2 scale).
#[must_use]
pub fn token_magnitude(kind: TokenKind, tok_bytes: &[u8]) -> f32 {
    match kind {
        TokenKind::Number => {
            let s = String::from_utf8_lossy(tok_bytes);
            match number_order(&s) {
                Some(order) => order as f32,
                None => 0.0,
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

/// Analyze a token stream with the default configuration.
#[must_use]
pub fn analyze(tokens: &[Token], bytes: &[u8]) -> MagnitudeField {
    analyze_with(tokens, bytes, &MagnitudeConfig::default())
}

/// Tokenize `bytes` (the significant-token lexer) and analyze - the convenience
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
        // A huge value among small ones: every other axis sees only `Number`
        // tokens, and here it is a magnitude outlier and a jump.
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

    /// Every length gets the magnitude `log2` gives it, inside the table's
    /// range and past its end.
    #[test]
    fn a_length_s_magnitude_is_its_log2_whether_the_table_holds_it_or_not() {
        let bytes = vec![b'x'; 2048];
        for n in 0..=2048usize {
            let got = token_magnitude(TokenKind::Word, &bytes[..n]);
            let want = (n.max(1) as f32).log2();
            assert_eq!(got.to_bits(), want.to_bits(), "length {n}");
        }
    }

    /// A number's order whichever path reads it: plain digits, digits past
    /// 2^53 and past a u64, a fraction, and every form the lexer reads as one
    /// number - underscores, a radix prefix in either case, an exponent - at
    /// sizes past a float's range, and text that is not a number.
    #[test]
    fn a_number_s_order_is_the_same_whichever_path_reads_it() {
        let close = |text: &str, want: f64| {
            let got = number_order(text).unwrap_or_else(|| panic!("{text:?} reads"));
            assert!((got - want).abs() < 1e-9, "{text:?}: {got} against {want}");
        };
        close("7", 7f64.log10());
        close("007", 7f64.log10());
        close(" 42 ", 42f64.log10());
        close("1000", 3.0);
        close("9007199254740993", 9_007_199_254_740_992f64.log10());
        close("18446744073709551616", 18_446_744_073_709_551_616f64.log10());
        close("99999999999999999999999", 23.0);
        close("3.25", 3.25f64.log10());
        close("1_000_000", 6.0);
        close("1e5", 5.0);
        close("2.5E-3", 2.5e-3f64.log10());
        close("6.02e23", 6.02e23f64.log10());
        close("0x1F", 31f64.log10());
        close("0X1f", 31f64.log10());
        close("0xffff_ffff", 4_294_967_295f64.log10());
        close("0b1010", 1.0);
        close("0o17", 15f64.log10());
        close("1e999999", 999_999.0);
        close("1e-999999", -999_999.0);
        close(&"9".repeat(400), 400.0);
        for none in ["0", "0.0", "0x0", "", "abc", "1..2", "١٢", "0x", "inf", "nan"] {
            assert_eq!(number_order(none), None, "{none:?}");
        }
    }

    #[test]
    fn number_value_drives_magnitude_not_length() {
        // `1000000` (1e6) outranks `9` though both are short: the value sets
        // the magnitude, not the length.
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
