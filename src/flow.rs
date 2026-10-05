//! The flow axis - trex's dynamics substrate (direction + rate of change).
//!
//! The other axes read a STATE at each token: its scale (`magnitude`), its
//! structural load (`stress`), its texture (`spectral`), its form (`shape`).
//! Flow reads the dynamics - which way the stream is heading and how fast. It
//! is the directional generalization of `magnitude`'s `dm/dt`: where that is
//! one signal's derivative, flow is the windowed slope, direction, momentum,
//! and reversal structure of any scalar signal.
//!
//! That is what makes it a meta axis: `analyze_signal` takes a precomputed
//! per-token scalar and reads its flow, so the same dynamics machinery composes
//! over the magnitude field, the stress depth, or the raw token length - "the
//! flow of any axis." Change-points (spectral / shape) find where a signal
//! shifts; flow finds which way and how strongly, and where the trend reverses.
//!
//! ## The analytic reading, and what it cost to keep time pointing forward
//!
//! [`analytic_signal`] is the complex counterpart of the same meta-axis idea:
//! for a real signal `s` it reads the instantaneous amplitude and phase, so
//! where the flow reading gives the slope over a window, this gives the local
//! envelope, the position inside the oscillation, and the rate the
//! oscillation itself is running at. The envelope is the part a slope cannot
//! reach: a signal swinging with a growing amplitude has a slope that turns
//! around at every peak while the envelope only climbs.
//!
//! The transform behind it, the Hilbert transform, is not causal in its
//! standard form - it wants the whole signal, and [`crate::spectral`] states
//! an arrow of time the rest of the crate holds to. Three routes were
//! available: approximate it causally, build a one-sided filter from the
//! resonator bank, or declare the reading dependent on the whole input and
//! let a chunked scanner defer it.
//!
//! It approximates. The kernel is truncated and shifted so it reads only
//! backward, which costs a fixed delay of half the filter length and band
//! limits the result, and in return gives a reading that a streaming scanner
//! can emit as it goes. Appending input never changes a value already emitted with full
//! support, which a test pins. The alternative that keeps the transform exact
//! would have made this the only axis that cannot stream, and the delay is
//! the cheaper price.
//!
//! Documented in `wiki/content/docs/reference/axes/flow.md`.

use crate::token::Token;

/// Which scalar signal the flow is read over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// The `magnitude` field's order-of-magnitude per token.
    Magnitude,
    /// The `stress` field's nesting depth per token.
    StressDepth,
    /// The token's byte length (self-contained, no other axis).
    Length,
}

impl Signal {
    /// Parse a `--signal` argument.
    #[must_use]
    pub fn parse(name: &str) -> Option<Signal> {
        match name {
            "magnitude" => Some(Signal::Magnitude),
            "stress" => Some(Signal::StressDepth),
            "length" => Some(Signal::Length),
            _ => None,
        }
    }

    /// A short, stable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Signal::Magnitude => "magnitude",
            Signal::StressDepth => "stress",
            Signal::Length => "length",
        }
    }
}

/// Knobs for the flow reader.
#[derive(Clone, Copy, Debug)]
pub struct FlowConfig {
    /// Trailing token window the slope is averaged over.
    pub window: usize,
    /// `|slope|` at or below this reads as steady (no trend).
    pub steady_band: f32,
}

impl Default for FlowConfig {
    fn default() -> Self {
        Self {
            window: 4,
            steady_band: 0.05,
        }
    }
}

/// One token's reading on the flow axis.
#[derive(Clone, Copy, Debug, Default)]
pub struct FlowFrame {
    /// Windowed rate of change: average per-step slope over the trailing window.
    pub slope: f32,
    /// Trend direction: `-1` falling, `0` steady, `+1` rising.
    pub direction: i8,
    /// Trend persistence: consecutive tokens holding the same non-steady
    /// direction.
    pub momentum: u16,
}

/// The dynamics side table, keyed by byte offset.
#[derive(Clone, Debug, Default)]
pub struct FlowField {
    /// Token count.
    pub n_tokens: usize,
    /// Byte span per token - the byte-offset key.
    pub spans: Vec<(usize, usize)>,
    /// One frame per token.
    pub frames: Vec<FlowFrame>,
    /// Reversal byte offsets: where the trend flips between rising and falling.
    pub reversals: Vec<usize>,
}

impl FlowField {
    /// The token index covering `byte`, or `None` if the field is empty.
    fn token_at(&self, byte: usize) -> Option<usize> {
        if self.spans.is_empty() {
            return None;
        }
        let i = self.spans.partition_point(|&(s, _)| s <= byte);
        Some(i.saturating_sub(1))
    }

    /// The flow slope at `byte`.
    #[must_use]
    pub fn slope_at(&self, byte: usize) -> f32 {
        self.token_at(byte)
            .and_then(|i| self.frames.get(i))
            .map_or(0.0, |f| f.slope)
    }

    /// The trend direction at `byte` (`-1` / `0` / `+1`).
    #[must_use]
    pub fn direction_at(&self, byte: usize) -> i8 {
        self.token_at(byte)
            .and_then(|i| self.frames.get(i))
            .map_or(0, |f| f.direction)
    }

    /// The trend persistence at `byte`: consecutive tokens up to it holding the
    /// same non-steady direction. Counts back past the caller's span, because
    /// it is a property of the position rather than of any window over it.
    #[must_use]
    pub fn momentum_at(&self, byte: usize) -> u16 {
        self.token_at(byte)
            .and_then(|i| self.frames.get(i))
            .map_or(0, |f| f.momentum)
    }
}

/// The per-token scalar signal a `Signal` selects.
#[must_use]
pub fn signal_values(tokens: &[Token], bytes: &[u8], signal: Signal) -> Vec<f32> {
    match signal {
        Signal::Magnitude => crate::magnitude::analyze(tokens, bytes)
            .frames
            .iter()
            .map(|f| f.magnitude)
            .collect(),
        Signal::StressDepth => crate::stress::analyze(tokens, bytes)
            .frames
            .iter()
            .map(|f| f32::from(f.depth))
            .collect(),
        Signal::Length => tokens
            .iter()
            .map(|t| (t.end - t.start) as f32)
            .collect(),
    }
}

/// Read the flow of a `Signal` over a token stream (default config).
#[must_use]
pub fn analyze(tokens: &[Token], bytes: &[u8], signal: Signal) -> FlowField {
    analyze_with(tokens, bytes, signal, &FlowConfig::default())
}

/// Read the flow of a `Signal` over a token stream with `cfg`'s window and
/// steady band.
#[must_use]
pub fn analyze_with(tokens: &[Token], bytes: &[u8], signal: Signal, cfg: &FlowConfig) -> FlowField {
    let sig = signal_values(tokens, bytes, signal);
    analyze_signal(tokens, &sig, cfg)
}

/// Tokenize `bytes` and read the flow of a `Signal` - the byte-only path.
#[must_use]
pub fn analyze_bytes(bytes: &[u8], signal: Signal) -> FlowField {
    let toks = crate::tokutil::lex_sig(bytes);
    analyze(&toks, bytes, signal)
}

/// The core: read the flow (slope, direction, momentum, reversals) of an
/// arbitrary precomputed per-token scalar `signal`. The meta-axis entry point -
/// every other axis exposes a signal this composes over.
#[must_use]
pub fn analyze_signal(tokens: &[Token], signal: &[f32], cfg: &FlowConfig) -> FlowField {
    let spans: Vec<(usize, usize)> = tokens.iter().map(|t| (t.start(), t.end())).collect();
    analyze_spans(&spans, signal, cfg)
}

/// The same reading over units of any size, given their byte spans.
///
/// The spans are all this needs of the layer it is running over, so one
/// implementation serves tokens and supertokens rather than each grain
/// carrying a copy. What changes with the grain is what a step means: over
/// tokens the slope is per token, over supertokens it is per construct, and
/// the second is not derivable from the first.
#[must_use]
pub fn analyze_spans(spans: &[(usize, usize)], signal: &[f32], cfg: &FlowConfig) -> FlowField {
    let n = spans.len();
    let mut field = FlowField {
        n_tokens: n,
        spans: Vec::with_capacity(n),
        frames: Vec::with_capacity(n),
        reversals: Vec::new(),
    };
    if n == 0 {
        return field;
    }
    field.spans.extend_from_slice(spans);

    let mut frames: Vec<FlowFrame> = Vec::with_capacity(n);
    let mut momentum: u16 = 0;
    let mut prev_dir: i8 = 0;
    for i in 0..n.min(signal.len()) {
        // slope: average per-step change over the trailing window.
        let lo = i.saturating_sub(cfg.window);
        let span = (i - lo).max(1) as f32;
        let slope = (signal[i] - signal[lo]) / span;

        let direction = if slope > cfg.steady_band {
            1
        } else if slope < -cfg.steady_band {
            -1
        } else {
            0
        };

        // momentum: run length of the same non-steady direction.
        if direction != 0 && direction == prev_dir {
            momentum = momentum.saturating_add(1);
        } else {
            momentum = u16::from(direction != 0);
        }

        // reversal: a flip between rising and falling (steady does not count).
        if direction != 0 && prev_dir != 0 && direction != prev_dir {
            field.reversals.push(spans[i].0);
        }
        if direction != 0 {
            prev_dir = direction;
        }

        frames.push(FlowFrame {
            slope,
            direction,
            momentum,
        });
    }
    // pad if the signal was short (defensive; signal is normally per-token).
    while frames.len() < n {
        frames.push(FlowFrame::default());
    }
    field.frames = frames;
    field
}

/// Knobs for the analytic reader.
#[derive(Clone, Copy, Debug)]
pub struct AnalyticConfig {
    /// Filter length in tokens. Odd; the group delay is half of it.
    pub taps: usize,
    /// Trailing window the running mean is taken over before the transform.
    pub detrend: usize,
}

impl Default for AnalyticConfig {
    fn default() -> Self {
        Self { taps: 31, detrend: 16 }
    }
}

/// The analytic reading of a scalar signal: its envelope and its current
/// position inside that envelope.
///
/// [`FlowField`] answers which way a signal is heading. This answers how
/// widely it is swinging and how fast it is swinging, which a slope cannot
/// separate: a signal oscillating with a growing envelope has a slope that
/// keeps changing sign while the envelope only rises.
#[derive(Clone, Debug, Default)]
pub struct AnalyticField {
    /// Instantaneous amplitude, the local envelope of the signal.
    pub amplitude: Vec<f32>,
    /// Instantaneous phase in `(-pi, pi]`, position within the oscillation.
    pub phase: Vec<f32>,
    /// Instantaneous frequency in radians per token, the phase advance.
    ///
    /// Unlike the resonator bank's, this is not prescribed: the bank rotates
    /// at the frequency it was tuned to whatever the input, while this is
    /// read off the signal and changes with it.
    pub frequency: Vec<f32>,
    /// Tokens of group delay. Index `i` reads the signal around `i`, using
    /// input up to `i + latency`, so the reading is causal with latency
    /// rather than centered on unseen input.
    pub latency: usize,
}

impl AnalyticField {
    /// Envelope at a token index, or `0.0` past the end.
    #[must_use]
    pub fn amplitude_at(&self, i: usize) -> f32 {
        self.amplitude.get(i).copied().unwrap_or(0.0)
    }

    /// Instantaneous frequency at a token index, or `0.0` past the end.
    #[must_use]
    pub fn frequency_at(&self, i: usize) -> f32 {
        self.frequency.get(i).copied().unwrap_or(0.0)
    }
}

/// The windowed Hilbert kernel, causal: `h[k]` weights `signal[i - k]`.
///
/// The ideal transform is `2 / (pi * n)` on odd offsets from the center and
/// zero on even ones, over an infinite two-sided support. Truncating it to
/// `taps` and shifting so the center lands at `taps / 2` is what makes it
/// causal, at the cost of a delay of exactly that much. The Hamming window
/// is there because a bare truncation rings: the sharp cutoff puts ripple
/// across the passband and the envelope inherits it.
fn hilbert_kernel(taps: usize) -> Vec<f32> {
    let n = taps | 1;
    let mid = (n / 2) as isize;
    (0..n)
        .map(|k| {
            let d = k as isize - mid;
            if d == 0 || d % 2 == 0 {
                return 0.0;
            }
            let ideal = 2.0 / (std::f32::consts::PI * d as f32);
            let w = 0.54
                - 0.46
                    * (std::f32::consts::TAU * k as f32 / (n - 1) as f32).cos();
            ideal * w
        })
        .collect()
}

/// Read the analytic signal of an arbitrary per-token scalar.
///
/// Composes the same way [`analyze_signal`] does, over any axis that can
/// produce a scalar per token.
///
/// The transform is applied causally. A Hilbert transform is not causal in
/// its standard form, and [`crate::spectral`] states an arrow of time this
/// crate holds to, so the kernel is truncated and delayed rather than
/// centered: the value at index `i` uses input through `i + latency` only.
/// What that costs is stated on [`AnalyticField::latency`], and the last
/// `latency` tokens have no full-support reading.
///
/// The signal is detrended first. The transform of a constant is zero, so a
/// signal on a large offset would report an envelope that is mostly
/// that offset and a phase pinned near zero; subtracting a trailing mean
/// leaves the part that actually oscillates, and keeps the pass causal.
#[must_use]
pub fn analytic_signal(signal: &[f32], cfg: &AnalyticConfig) -> AnalyticField {
    let n = signal.len();
    let taps = cfg.taps.max(3) | 1;
    let latency = taps / 2;
    let mut out = AnalyticField {
        amplitude: vec![0.0; n],
        phase: vec![0.0; n],
        frequency: vec![0.0; n],
        latency,
    };
    if n == 0 {
        return out;
    }

    let win = cfg.detrend.max(1);
    let detrended: Vec<f32> = {
        let mut acc = 0.0f32;
        signal
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                acc += x;
                if i >= win {
                    acc -= signal[i - win];
                }
                x - acc / (i + 1).min(win) as f32
            })
            .collect()
    };

    let h = hilbert_kernel(taps);
    for i in 0..n {
        // The reading is placed at `at`, using input up to `i`: the delay is
        // what makes it causal.
        let Some(at) = i.checked_sub(latency) else { continue };
        let mut im = 0.0f32;
        for (k, &hk) in h.iter().enumerate() {
            if hk == 0.0 {
                continue;
            }
            let Some(j) = i.checked_sub(k) else { break };
            im += hk * detrended[j];
        }
        let re = detrended[at];
        out.amplitude[at] = (re * re + im * im).sqrt();
        out.phase[at] = im.atan2(re);
    }

    for i in 1..n {
        let mut d = out.phase[i] - out.phase[i - 1];
        while d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        while d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        out.frequency[i] = d;
    }
    out
}

/// Read the analytic signal of a named [`Signal`] over a token stream.
#[must_use]
pub fn analytic(tokens: &[Token], bytes: &[u8], signal: Signal) -> AnalyticField {
    let sig = signal_values(tokens, bytes, signal);
    analytic_signal(&sig, &AnalyticConfig::default())
}

/// One value per supertoken, and the spans they occupy.
///
/// The value is a fold: a unit's scalar is its tokens' combined, through the
/// [`crate::profile`] monoid, so it is determined by the layer below. What is
/// not determined is what the reading over these values says, because a step
/// is now a construct rather than a token - the sequence being read is a
/// different sequence, not a coarser view of the same one.
fn supertoken_signal(
    toks: &[Token],
    bytes: &[u8],
    signal: Signal,
) -> (Vec<(usize, usize)>, Vec<f32>) {
    let stress = crate::stress::analyze(toks, bytes);
    let units = crate::supertoken::supertokens_from(toks, bytes);

    let mut spans = Vec::with_capacity(units.len());
    let mut values = Vec::with_capacity(units.len());
    let mut cursor = 0usize;
    for u in &units {
        // Unit starts are non-decreasing, so one cursor walks the left ends.
        // The right end is found by search rather than by scanning from the
        // left one: a nested unit ends before its parent, so a span walk costs
        // the sum of every unit's span rather than the token count.
        while cursor < toks.len() && toks[cursor].start() < u.start {
            cursor += 1;
        }
        let lo = cursor;
        let hi = lo + toks[lo..].partition_point(|t| t.end() <= u.end);
        spans.push((u.start, u.end));
        // Only the one scalar the signal reads is taken over the span. Folding
        // a whole `Profile` here would build the free monoid in `shape` for
        // every unit, and its combine returns a fresh sequence, so a span of
        // `L` costs `L(L+1)/2` copies and `L` allocations. Over a stream whose
        // units are few and long - which is what the significant-token stream
        // gives on machine-generated input - that is the sum of each span
        // squared rather than the sum of the spans.
        //
        // The depth is read by index because the field was built from these
        // tokens and their starts strictly increase, so the byte-offset lookup
        // resolves to frame `i`.
        values.push(match signal {
            Signal::Magnitude => {
                if hi > lo {
                    let sum: f32 = toks[lo..hi]
                        .iter()
                        .map(|t| crate::magnitude::token_magnitude(t.kind, &bytes[t.span()]))
                        .sum();
                    sum / (hi - lo) as f32
                } else {
                    0.0
                }
            }
            Signal::StressDepth => {
                stress.frames[lo..hi].iter().map(|f| f.depth).max().map_or(0.0, f32::from)
            }
            Signal::Length => (u.end - u.start) as f32,
        });
    }
    (spans, values)
}

/// Read the flow of a `Signal` over the supertoken stream.
///
/// A step here is one construct, so this reads how a document moves from
/// construct to construct rather than from token to token. The two are
/// different readings of the same input and neither follows from the other.
#[must_use]
pub fn analyze_supertokens(toks: &[Token], bytes: &[u8], signal: Signal) -> FlowField {
    analyze_supertokens_with(toks, bytes, signal, &FlowConfig::default())
}

/// Read the flow of a `Signal` over the supertoken stream with `cfg`'s
/// window and steady band, counted in constructs.
#[must_use]
pub fn analyze_supertokens_with(toks: &[Token], bytes: &[u8], signal: Signal, cfg: &FlowConfig) -> FlowField {
    let (spans, values) = supertoken_signal(toks, bytes, signal);
    analyze_spans(&spans, &values, cfg)
}

/// Read the analytic signal over the supertoken stream.
#[must_use]
pub fn analytic_supertokens(toks: &[Token], bytes: &[u8], signal: Signal) -> AnalyticField {
    let (_, values) = supertoken_signal(toks, bytes, signal);
    analytic_signal(&values, &AnalyticConfig::default())
}

#[cfg(test)]
mod grain_tests {
    use super::*;

    #[test]
    fn the_supertoken_signal_reads_what_the_folded_profile_reads() {
        // Reading one scalar over each span replaces folding a whole profile
        // over it, so what it must equal is that fold, signal by signal and
        // unit by unit. Both readings are taken left to right over the same
        // tokens, so the bar is bit equality rather than a tolerance.
        use crate::profile::{AxisCtx, Profile, fold_tokens};
        for src in [
            &b"{module, [{a, [1, 2, {b, [3, 4]}]}, {c, [5, {d, [6, 7]}]}]}. {x, [{y, 8}]}."[..],
            &b"alpha beta gamma delta"[..],
            &b"f(x) g(yy) h(zzz) i(wwww)"[..],
        ] {
            let toks = crate::lexer::lex(src);
            let stress = crate::stress::analyze(&toks, src);
            let ctx = AxisCtx { stress: Some(&stress), ..AxisCtx::new(src) };
            let units = crate::supertoken::supertokens_from(&toks, src);

            for signal in [Signal::Magnitude, Signal::StressDepth, Signal::Length] {
                let (spans, values) = supertoken_signal(&toks, src, signal);
                assert_eq!(spans.len(), units.len(), "one span per unit, {signal:?} {src:?}");

                let mut cursor = 0usize;
                for (k, u) in units.iter().enumerate() {
                    while cursor < toks.len() && toks[cursor].start() < u.start {
                        cursor += 1;
                    }
                    let lo = cursor;
                    let mut hi = cursor;
                    while hi < toks.len() && toks[hi].end() <= u.end {
                        hi += 1;
                    }
                    let p: Profile = fold_tokens(lo, &toks[lo..hi], &ctx);
                    let want = match signal {
                        Signal::Magnitude => p.magnitude.mean(),
                        Signal::StressDepth => f32::from(p.stress.max_depth),
                        Signal::Length => (u.end - u.start) as f32,
                    };
                    assert_eq!(
                        values[k].to_bits(),
                        want.to_bits(),
                        "unit {k} under {signal:?} reads {} against the fold's {want}, {src:?}",
                        values[k]
                    );
                }
            }
        }
    }

    #[test]
    fn the_supertoken_reading_is_not_the_token_reading_coarsened() {
        // Constructs whose scale rises across the document while the tokens
        // inside each one keep the same shape. Over tokens the reading keeps
        // turning around at every construct boundary; over constructs it is a
        // trend, because a step there is a whole unit.
        let src = b"a = 1;\nb = 22;\nc = 333;\nd = 4444;\ne = 55555;\nf = 666666;\n";
        let toks = crate::lexer::lex(src);
        let by_token = analyze(&toks, src, Signal::Magnitude);
        let by_unit = analyze_supertokens(&toks, src, Signal::Magnitude);
        assert!(!by_unit.frames.is_empty(), "there are constructs to read");
        assert!(by_unit.frames.len() < by_token.frames.len(), "fewer units than tokens");

        // The unit reading rises, because each construct carries a larger
        // number than the one before.
        let rising = by_unit.frames.iter().filter(|f| f.direction > 0).count();
        let falling = by_unit.frames.iter().filter(|f| f.direction < 0).count();
        assert!(rising > falling, "the construct sequence trends up: {rising} vs {falling}");
    }

    #[test]
    fn the_analytic_reading_lifts_the_same_way() {
        let src = b"a = 1;\nb = 22;\nc = 333;\nd = 4444;\ne = 55555;\nf = 666666;\n";
        let toks = crate::lexer::lex(src);
        let a = analytic_supertokens(&toks, src, Signal::Length);
        assert!(!a.amplitude.is_empty(), "an envelope over constructs");
    }

    #[test]
    fn spans_are_all_the_reading_needs_of_its_layer() {
        // The token entry and the span entry are the same computation, which
        // is what lets one implementation serve both grains.
        let src = b"a 1 bb 22 ccc 333";
        let toks = crate::tokutil::lex_sig(src);
        let sig: Vec<f32> = toks.iter().map(|t| t.len() as f32).collect();
        let spans: Vec<(usize, usize)> = toks.iter().map(|t| (t.start(), t.end())).collect();
        let cfg = FlowConfig::default();
        assert_eq!(
            analyze_signal(&toks, &sig, &cfg).frames.len(),
            analyze_spans(&spans, &sig, &cfg).frames.len()
        );
    }
}

#[cfg(test)]
mod analytic_tests {
    use super::*;

    fn cfg() -> AnalyticConfig {
        AnalyticConfig::default()
    }

    /// Mean of a slice, ignoring the filter's warm-up and tail.
    fn mid_mean(v: &[f32], lat: usize) -> f32 {
        let lo = lat * 2;
        let hi = v.len().saturating_sub(lat);
        if hi <= lo {
            return 0.0;
        }
        v[lo..hi].iter().sum::<f32>() / (hi - lo) as f32
    }

    #[test]
    fn a_constant_amplitude_sinusoid_reads_its_amplitude_and_frequency() {
        // The case where the answer is known exactly: amplitude 3, period 8,
        // so the envelope should read 3 and the frequency 2*pi/8.
        let w = std::f32::consts::TAU / 8.0;
        let s: Vec<f32> = (0..400).map(|i| 3.0 * (w * i as f32).sin()).collect();
        let a = analytic_signal(&s, &cfg());
        let env = mid_mean(&a.amplitude, a.latency);
        assert!((env - 3.0).abs() < 0.3, "envelope reads the amplitude: {env}");
        let f = mid_mean(&a.frequency, a.latency);
        assert!((f - w).abs() < 0.05, "frequency reads the rate: {f} vs {w}");
    }

    #[test]
    fn the_envelope_rises_where_the_slope_only_keeps_changing_sign() {
        // What the analytic reading separates that a slope cannot. The signal
        // oscillates the whole way with a linearly growing amplitude: its
        // slope reverses every few tokens throughout, so flow reports the
        // same churn at the start as at the end, while the envelope rises
        // monotonically because that is the thing actually changing.
        let w = std::f32::consts::TAU / 8.0;
        let s: Vec<f32> =
            (0..400).map(|i| (0.5 + i as f32 * 0.02) * (w * i as f32).sin()).collect();
        let a = analytic_signal(&s, &cfg());
        let lat = a.latency;
        let early = a.amplitude[lat * 2..100].iter().sum::<f32>() / (100 - lat * 2) as f32;
        let late = a.amplitude[300..380].iter().sum::<f32>() / 80.0;
        assert!(late > early * 3.0, "the envelope tracks the growth: {early} -> {late}");

        // The envelope climbs almost without turning back, because growth is
        // the only thing happening to it.
        let env_flips = (lat * 2 + 1..380)
            .filter(|&i| {
                (a.amplitude[i] - a.amplitude[i - 1]) * (a.amplitude[i - 1] - a.amplitude[i - 2])
                    < 0.0
            })
            .count();

        // Flow over the same signal, with a real token stream behind it so
        // this is a comparison and not an empty one. Its slope turns around
        // constantly, because at every peak of the oscillation the signal is
        // momentarily flat and then heads the other way. Both readings are
        // correct; they answer different questions, and only one of them
        // recovers the growth.
        let src: Vec<u8> = std::iter::repeat_n(b"a ", s.len()).flatten().copied().collect();
        let toks = crate::tokutil::lex_sig(&src);
        assert_eq!(toks.len(), s.len(), "one token per sample");
        let f = analyze_signal(&toks, &s, &FlowConfig::default());
        let slope_flips = (lat * 2 + 1..380)
            .filter(|&i| f.frames[i].slope * f.frames[i - 1].slope < 0.0)
            .count();

        assert!(
            slope_flips > env_flips * 4,
            "the slope keeps reversing where the envelope does not: {slope_flips} vs {env_flips}"
        );
    }

    #[test]
    fn a_chirp_is_read_as_a_rising_frequency() {
        // Instantaneous frequency is read off the signal rather than
        // prescribed, so a signal that speeds up reads as speeding up. This
        // is the property the resonator bank cannot have: its phase advances
        // at whatever theta it was tuned to, whatever the input does.
        let s: Vec<f32> = (0..600)
            .map(|i| {
                let t = i as f32;
                let phase = 0.15 * t + 0.0004 * t * t;
                phase.sin()
            })
            .collect();
        let a = analytic_signal(&s, &cfg());
        let lat = a.latency;
        let early = a.frequency[lat * 2..200].iter().sum::<f32>() / (200 - lat * 2) as f32;
        let late = a.frequency[400..560].iter().sum::<f32>() / 160.0;
        assert!(late > early + 0.05, "the frequency rises with the chirp: {early} -> {late}");
    }

    #[test]
    fn the_reading_is_causal_with_latency() {
        // The contract that keeps this axis compatible with the crate's
        // arrow of time: appending input must not change any reading already
        // emitted with full support.
        let w = std::f32::consts::TAU / 7.0;
        let long: Vec<f32> = (0..300).map(|i| (w * i as f32).sin()).collect();
        let short = &long[..200];
        let a = analytic_signal(&long, &cfg());
        let b = analytic_signal(short, &cfg());
        let lat = a.latency;
        // Everything the short read with full support must survive unchanged.
        for i in lat..(200 - lat) {
            assert!(
                (a.amplitude[i] - b.amplitude[i]).abs() < 1e-4,
                "index {i} moved when later input arrived: {} vs {}",
                b.amplitude[i],
                a.amplitude[i]
            );
        }
    }

    #[test]
    fn a_constant_signal_has_no_envelope() {
        // Detrending is what makes this true: without it the envelope would
        // report the offset the signal happens to be on.
        let s = vec![7.5f32; 200];
        let a = analytic_signal(&s, &cfg());
        assert!(mid_mean(&a.amplitude, a.latency) < 0.05, "a flat signal does not oscillate");
    }

    #[test]
    fn degenerate_inputs_are_safe() {
        assert!(analytic_signal(&[], &cfg()).amplitude.is_empty());
        let one = analytic_signal(&[1.0], &cfg());
        assert_eq!(one.amplitude.len(), 1);
        assert_eq!(one.frequency_at(99), 0.0);
        assert_eq!(one.amplitude_at(99), 0.0);
    }

    #[test]
    fn it_composes_over_a_named_axis_the_way_flow_does() {
        let src = b"a 1 bb 22 ccc 333 dddd 4444 eeeee 55555 f 6 gg 77 hhh 888";
        let toks = crate::tokutil::lex_sig(src);
        let a = analytic(&toks, src, Signal::Length);
        assert_eq!(a.amplitude.len(), toks.len());
        assert!(a.amplitude.iter().any(|&x| x > 0.0), "the length signal swings");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_flows_upward_with_momentum() {
        // Orders of magnitude climbing: flow direction is +1 and momentum builds.
        let f = analyze_bytes(b"1 10 100 1000 10000 100000", Signal::Magnitude);
        let last = f.frames.last().copied().unwrap_or_default();
        assert_eq!(last.direction, 1, "a rising ramp flows upward");
        assert!(last.momentum >= 3, "sustained trend builds momentum, got {}", last.momentum);
        assert!(f.reversals.is_empty(), "a monotone ramp has no reversal");
    }

    #[test]
    fn valley_has_a_reversal_at_the_bottom() {
        // Magnitude falls then rises - a turning point at the trough.
        let f = analyze_bytes(b"100000 1000 10 1 10 1000 100000", Signal::Magnitude);
        assert!(
            !f.reversals.is_empty(),
            "a fall-then-rise should register a reversal"
        );
    }

    #[test]
    fn flow_composes_over_stress_depth() {
        // Flow over the STRESS signal: nesting deepens (rising) then releases.
        let f = analyze_bytes(b"a(b(c(d(e))))", Signal::StressDepth);
        assert!(f.frames.iter().any(|fr| fr.direction == 1), "deepening rises");
        assert!(
            f.frames.iter().any(|fr| fr.direction == -1),
            "the release falls"
        );
        assert!(!f.reversals.is_empty(), "deepen-then-release reverses");
    }

    #[test]
    fn flat_signal_is_steady() {
        let f = analyze_bytes(b"a b c d e f g", Signal::Length);
        assert!(f.frames.iter().all(|fr| fr.direction == 0), "equal lengths are steady");
        assert!(f.reversals.is_empty());
    }

    #[test]
    fn empty_is_safe() {
        let f = analyze_bytes(b"", Signal::Magnitude);
        assert_eq!(f.n_tokens, 0);
        assert!(f.frames.is_empty());
        assert!(f.reversals.is_empty());
        assert_eq!(f.direction_at(0), 0);
    }
}
