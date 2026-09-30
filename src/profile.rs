//! Axis profiles: the per-axis monoid that lifts a token reading to any unit
//! above it.
//!
//! Every property axis already reads one token, and several already pool a
//! span - `SpectralField::signature` averages frames, `MagnitudeField::energy_of`
//! sums squares, `StressField::depth_at` is a point query a span wants the
//! maximum of. Three different aggregations, written three times, each private
//! to its axis, and none reusable one rung up.
//!
//! This names the operation. An axis reading is a monoid: it has an identity,
//! and a way to combine two readings that is associative. Once an axis says
//! which monoid it is, the tower costs nothing per level:
//!
//! ```text
//! profile(supertoken) = fold over its tokens
//! profile(block)      = fold over its supertokens
//! profile(document)   = fold over its blocks
//! ```
//!
//! No level-specific code, arbitrary height, and because the operator is
//! associative the same fold splits across cores or runs incrementally over a
//! stream without changing the answer.
//!
//! One axis carries several monoids at once, which is why a single "combine"
//! per axis would be wrong. Magnitude is the clear case: energy is extensive
//! and sums, peak scale is a maximum, and the mean needs a sum with a count.
//! [`MagnitudeProfile`] holds all three because they are different questions
//! about the same field.

use crate::echo::EchoField;
use crate::spectral::SpectralField;
use crate::stress::StressField;
use crate::token::Token;

/// What a per-token axis reading needs besides the token.
///
/// Each field is optional and built only when the caller wants that axis, the
/// same discipline `engine::Fields` uses: a profile over axes nobody asked for
/// costs nothing.
#[derive(Clone, Copy)]
pub struct AxisCtx<'a> {
    /// The whole input, for readings that look at a token's bytes.
    pub bytes: &'a [u8],
    /// The pooled spectral field, keyed by byte offset.
    pub spectral: Option<&'a SpectralField>,
    /// The structural-load field, keyed by byte offset.
    pub stress: Option<&'a StressField>,
    /// The recurrence field, index-aligned with the token stream.
    pub echo: Option<&'a EchoField>,
    /// The vantage field, keyed by byte offset.
    pub observation: Option<&'a crate::observation::ObservationField>,
    /// The segmentation field, keyed by byte offset.
    pub seam: Option<&'a crate::seam::SeamField>,
    /// The dynamics field, keyed by byte offset.
    pub flow: Option<&'a crate::flow::FlowField>,
}

impl<'a> AxisCtx<'a> {
    /// A context over `bytes` with no axis fields built.
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        AxisCtx {
            bytes,
            spectral: None,
            stress: None,
            echo: None,
            observation: None,
            seam: None,
            flow: None,
        }
    }
}

/// One axis's reading of a unit, as a monoid.
///
/// The laws the implementations are tested against:
/// `identity().combine(x) == x`, `x.combine(&identity()) == x`, and
/// `(a.combine(&b)).combine(&c) == a.combine(&b.combine(&c))`.
///
/// Associativity is not decoration. It is what lets the fold be re-bracketed,
/// which is what makes the tower level-agnostic and the same fold safe to
/// split across cores or run incrementally.
pub trait AxisProfile: Clone + PartialEq + std::fmt::Debug {
    /// The reading of an empty unit.
    fn identity() -> Self;
    /// Combine two readings. Must be associative, with [`Self::identity`] as
    /// the unit on both sides.
    fn combine(&self, other: &Self) -> Self;
    /// The reading of a single token at stream index `idx`.
    fn of_token(idx: usize, tok: &Token, ctx: &AxisCtx<'_>) -> Self;
}

/// Fold an axis over a token slice starting at stream index `base`.
///
/// `base` is the index of `toks[0]` in the whole stream, which the recurrence
/// axis needs because its frames are index-aligned with it.
pub fn fold_tokens<P: AxisProfile>(base: usize, toks: &[Token], ctx: &AxisCtx<'_>) -> P {
    toks.iter()
        .enumerate()
        .fold(P::identity(), |acc, (i, t)| acc.combine(&P::of_token(base + i, t, ctx)))
}

/// Fold already-computed profiles one rung up. This is the whole of what a
/// higher level costs.
pub fn fold_profiles<P: AxisProfile>(parts: &[P]) -> P {
    parts.iter().fold(P::identity(), |acc, p| acc.combine(p))
}

/// The scale axis. Three monoids at once: `sumsq` is the extensive energy,
/// `max` the peak order of magnitude, and `sum` with `count` the mean.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MagnitudeProfile {
    /// Sum of per-token magnitudes.
    pub sum: f32,
    /// Sum of squared magnitudes: the energy of the span.
    pub sumsq: f32,
    /// Largest magnitude in the span.
    pub max: f32,
    /// Tokens folded.
    pub count: u32,
}

impl MagnitudeProfile {
    /// Mean magnitude, or `0.0` for an empty span.
    #[must_use]
    pub fn mean(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.sum / self.count as f32 }
    }

    /// Standard deviation of the magnitudes, or `0.0` for an empty span.
    #[must_use]
    pub fn std_dev(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        let mean = self.mean();
        (self.sumsq / self.count as f32 - mean * mean).max(0.0).sqrt()
    }
}

impl AxisProfile for MagnitudeProfile {
    fn identity() -> Self {
        // `max` starts at negative infinity so it is the identity for maximum;
        // `mean` reports 0.0 for an empty span via the count.
        MagnitudeProfile { sum: 0.0, sumsq: 0.0, max: f32::NEG_INFINITY, count: 0 }
    }

    fn combine(&self, other: &Self) -> Self {
        MagnitudeProfile {
            sum: self.sum + other.sum,
            sumsq: self.sumsq + other.sumsq,
            max: self.max.max(other.max),
            count: self.count + other.count,
        }
    }

    fn of_token(_idx: usize, tok: &Token, ctx: &AxisCtx<'_>) -> Self {
        let m = crate::magnitude::token_magnitude(tok.kind, &ctx.bytes[tok.span()]);
        MagnitudeProfile { sum: m, sumsq: m * m, max: m, count: 1 }
    }
}

/// The structural-load axis. A span's load is its deepest point, plus the mean
/// depth over it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StressProfile {
    /// Deepest bracket nesting in the span.
    pub max_depth: u16,
    /// Summed depth, for the mean.
    pub sum_depth: u32,
    /// Tokens folded.
    pub count: u32,
}

impl StressProfile {
    /// Mean nesting depth, or `0.0` for an empty span.
    #[must_use]
    pub fn mean_depth(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.sum_depth as f32 / self.count as f32 }
    }
}

impl AxisProfile for StressProfile {
    fn identity() -> Self {
        StressProfile::default()
    }

    fn combine(&self, other: &Self) -> Self {
        StressProfile {
            max_depth: self.max_depth.max(other.max_depth),
            sum_depth: self.sum_depth + other.sum_depth,
            count: self.count + other.count,
        }
    }

    fn of_token(_idx: usize, tok: &Token, ctx: &AxisCtx<'_>) -> Self {
        let d = ctx.stress.map_or(0, |f| f.depth_at(tok.start()));
        StressProfile { max_depth: d, sum_depth: u32::from(d), count: 1 }
    }
}

/// The structural-form axis. The silhouette of a span is the concatenation of
/// its tokens' silhouettes, so this axis's monoid is the free monoid over
/// class codes: `combine` appends, and the identity is the empty sequence.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ShapeProfile {
    /// Per-token silhouette class codes, in stream order.
    pub silhouette: Vec<u32>,
}

impl AxisProfile for ShapeProfile {
    fn identity() -> Self {
        ShapeProfile::default()
    }

    fn combine(&self, other: &Self) -> Self {
        let mut s = Vec::with_capacity(self.silhouette.len() + other.silhouette.len());
        s.extend_from_slice(&self.silhouette);
        s.extend_from_slice(&other.silhouette);
        ShapeProfile { silhouette: s }
    }

    fn of_token(_idx: usize, tok: &Token, ctx: &AxisCtx<'_>) -> Self {
        ShapeProfile {
            silhouette: vec![crate::shape::shape_class(tok.kind, &ctx.bytes[tok.span()])],
        }
    }
}

/// The temporal-texture axis. Entropy averages over a span; the period is
/// taken from the strongest-periodicity token in it, which is the same
/// argmax `SpectralField::signature` already applies within one span.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SpectralProfile {
    /// Summed pooled entropy, for the mean.
    pub entropy_sum: f32,
    /// Tokens folded.
    pub count: u32,
    /// Dominant byte-period of the strongest-periodicity token.
    pub period: u16,
    /// That token's periodicity strength.
    pub period_strength: f32,
}

impl SpectralProfile {
    /// Mean pooled entropy, or `0.0` for an empty span.
    #[must_use]
    pub fn mean_entropy(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.entropy_sum / self.count as f32 }
    }
}

impl AxisProfile for SpectralProfile {
    fn identity() -> Self {
        SpectralProfile::default()
    }

    fn combine(&self, other: &Self) -> Self {
        let (period, period_strength) = if other.period_strength > self.period_strength {
            (other.period, other.period_strength)
        } else {
            (self.period, self.period_strength)
        };
        SpectralProfile {
            entropy_sum: self.entropy_sum + other.entropy_sum,
            count: self.count + other.count,
            period,
            period_strength,
        }
    }

    fn of_token(_idx: usize, tok: &Token, ctx: &AxisCtx<'_>) -> Self {
        let Some(f) = ctx.spectral else {
            return SpectralProfile { count: 1, ..SpectralProfile::default() };
        };
        // This axis carries the entropy and the period up the tower, so a
        // field built without either would fold a zero into every span above
        // it and nothing downstream could tell that from a real zero.
        let mut want = crate::spectral::Needs::none();
        want.entropy = true;
        want.period = true;
        f.assert_carries(want, "the spectral profile");
        let sig = f.signature(tok.start(), tok.end());
        SpectralProfile {
            entropy_sum: sig.entropy,
            count: 1,
            period: sig.period,
            period_strength: sig.period_strength,
        }
    }
}

/// The vantage axis. A span's reading is what its tokens looked like from each
/// of the three vantages, and how far the past and the future disagreed.
///
/// The axis reads one signal from a past-only, a future-only and a centered
/// window, so this is the profile that carries a FORWARD reading up the tower:
/// a span at any rung can be asked what the future said about it, not only
/// what the past did. The disagreement is kept as a peak beside its sum,
/// because a span holding one strongly contested point and a span uniformly
/// mildly contested are different structures with the same mean.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ObservationProfile {
    /// Summed past-only reading, for the mean.
    pub causal_sum: f32,
    /// Summed future-only reading, for the mean.
    pub anticausal_sum: f32,
    /// Summed centered reading, for the mean.
    pub centered_sum: f32,
    /// Summed observer-dependence, for the mean.
    pub disagreement_sum: f32,
    /// The largest observer-dependence over the span.
    pub disagreement_max: f32,
    /// Tokens spanning a contested point.
    pub contested: u32,
    /// Tokens folded.
    pub count: u32,
}

impl ObservationProfile {
    /// Mean past-only reading, or `0.0` for an empty span.
    #[must_use]
    pub fn mean_causal(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.causal_sum / self.count as f32 }
    }

    /// Mean future-only reading, or `0.0` for an empty span.
    #[must_use]
    pub fn mean_anticausal(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.anticausal_sum / self.count as f32 }
    }

    /// Mean centered reading, or `0.0` for an empty span.
    #[must_use]
    pub fn mean_centered(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.centered_sum / self.count as f32 }
    }

    /// Mean observer-dependence, or `0.0` for an empty span.
    #[must_use]
    pub fn mean_disagreement(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.disagreement_sum / self.count as f32 }
    }

    /// How far the future revises the past over this span: the mean
    /// future-only reading less the mean past-only one. Positive where the
    /// future reads more than the past did.
    #[must_use]
    pub fn revision(&self) -> f32 {
        self.mean_anticausal() - self.mean_causal()
    }
}

impl AxisProfile for ObservationProfile {
    fn identity() -> Self {
        ObservationProfile::default()
    }

    fn combine(&self, other: &Self) -> Self {
        ObservationProfile {
            causal_sum: self.causal_sum + other.causal_sum,
            anticausal_sum: self.anticausal_sum + other.anticausal_sum,
            centered_sum: self.centered_sum + other.centered_sum,
            disagreement_sum: self.disagreement_sum + other.disagreement_sum,
            disagreement_max: self.disagreement_max.max(other.disagreement_max),
            contested: self.contested + other.contested,
            count: self.count + other.count,
        }
    }

    fn of_token(_idx: usize, tok: &Token, ctx: &AxisCtx<'_>) -> Self {
        let Some(f) = ctx.observation else {
            return ObservationProfile { count: 1, ..ObservationProfile::default() };
        };
        let at = tok.start();
        ObservationProfile {
            causal_sum: f.causal_at(at),
            anticausal_sum: f.anticausal_at(at),
            centered_sum: f.centered_at(at),
            disagreement_sum: f.disagreement_at(at),
            disagreement_max: f.disagreement_at(at),
            contested: u32::from(f.any_contested_in(tok.start(), tok.end())),
            count: 1,
        }
    }
}

/// The segmentation axis. A span's reading is how strongly its tokens sit at
/// segment boundaries, and how much of that strength came from what follows
/// rather than what precedes.
///
/// The axis reads branching entropy in both directions, so this is the second
/// profile carrying a reading from ahead: `fwd_sum` is the boundary-after
/// signal and `bwd_sum` the boundary-before one. Their difference says which
/// side of a position the evidence for a cut is on, which the cut count alone
/// cannot.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SeamProfile {
    /// Summed forward branching entropy, for the mean.
    pub fwd_sum: f32,
    /// Summed backward branching entropy, for the mean.
    pub bwd_sum: f32,
    /// Summed seam strength, for the mean.
    pub strength_sum: f32,
    /// The strongest seam over the span.
    pub strength_max: f32,
    /// Tokens starting a segment.
    pub cuts: u32,
    /// Tokens folded.
    pub count: u32,
}

impl SeamProfile {
    /// Mean forward branching entropy, or `0.0` for an empty span.
    #[must_use]
    pub fn mean_fwd(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.fwd_sum / self.count as f32 }
    }

    /// Mean backward branching entropy, or `0.0` for an empty span.
    #[must_use]
    pub fn mean_bwd(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.bwd_sum / self.count as f32 }
    }

    /// Segment starts per token over the span: how finely it is cut.
    #[must_use]
    pub fn cut_density(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.cuts as f32 / self.count as f32 }
    }

    /// Which side the evidence for a cut sits on: the mean forward reading
    /// less the mean backward one. Positive where what follows carries it.
    #[must_use]
    pub fn asymmetry(&self) -> f32 {
        self.mean_fwd() - self.mean_bwd()
    }
}

impl AxisProfile for SeamProfile {
    fn identity() -> Self {
        SeamProfile::default()
    }

    fn combine(&self, other: &Self) -> Self {
        SeamProfile {
            fwd_sum: self.fwd_sum + other.fwd_sum,
            bwd_sum: self.bwd_sum + other.bwd_sum,
            strength_sum: self.strength_sum + other.strength_sum,
            strength_max: self.strength_max.max(other.strength_max),
            cuts: self.cuts + other.cuts,
            count: self.count + other.count,
        }
    }

    fn of_token(_idx: usize, tok: &Token, ctx: &AxisCtx<'_>) -> Self {
        let Some(f) = ctx.seam else {
            return SeamProfile { count: 1, ..SeamProfile::default() };
        };
        let at = tok.start();
        SeamProfile {
            fwd_sum: f.fwd_at(at),
            bwd_sum: f.bwd_at(at),
            strength_sum: f.strength_at(at),
            strength_max: f.strength_at(at),
            cuts: u32::from(f.starts_segment(at)),
            count: 1,
        }
    }
}

/// The dynamics axis. A span's reading is which way its signal was going, how
/// steadily, and how often it changed its mind.
///
/// The first profile here whose combine is not a plain sum. A reversal is a
/// property of a PAIR of adjacent tokens, so a span holds its own reversals
/// plus one more wherever its last direction meets the next span's first: the
/// standard count-adjacent-unequal-pairs monoid, which needs the two end
/// directions carried and nothing else. Splitting a span and folding the halves
/// would otherwise lose exactly the reversal at the join.
///
/// `momentum` needs no such carry. The field already counts persistence per
/// token, so a peak over the span is a maximum, and it reads back past the
/// span's start on purpose: persistence is a property of the position, not of
/// the window someone drew around it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FlowProfile {
    /// Summed windowed slope, for the mean.
    pub slope_sum: f32,
    /// Tokens whose trend is rising.
    pub rising: u32,
    /// Tokens whose trend is falling.
    pub falling: u32,
    /// Tokens whose trend is steady.
    pub steady: u32,
    /// The longest persistence any token in the span carries.
    pub momentum_max: u16,
    /// Direction changes between adjacent tokens within the span.
    pub reversals: u32,
    /// Direction of the first token, for joining a span on the left.
    pub first: i8,
    /// Direction of the last token, for joining a span on the right.
    pub last: i8,
    /// Tokens folded.
    pub count: u32,
}

impl FlowProfile {
    /// Mean slope, or `0.0` for an empty span.
    #[must_use]
    pub fn mean_slope(&self) -> f32 {
        if self.count == 0 { 0.0 } else { self.slope_sum / self.count as f32 }
    }

    /// Net direction over the span: rising tokens less falling ones, over the
    /// tokens folded. `+1.0` all rising, `-1.0` all falling, `0.0` balanced.
    #[must_use]
    pub fn drift(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        (self.rising as f32 - self.falling as f32) / self.count as f32
    }

    /// Reversals per token: how often the span changes its mind. A steady
    /// trend reads `0.0` however long it runs.
    #[must_use]
    pub fn volatility(&self) -> f32 {
        if self.count < 2 { 0.0 } else { self.reversals as f32 / (self.count - 1) as f32 }
    }
}

impl AxisProfile for FlowProfile {
    fn identity() -> Self {
        FlowProfile::default()
    }

    fn combine(&self, other: &Self) -> Self {
        // An empty span has no end directions to join against, so it returns
        // the other side untouched. That is what makes `identity` a unit here
        // rather than a span whose direction happens to be steady.
        if self.count == 0 {
            return other.clone();
        }
        if other.count == 0 {
            return self.clone();
        }
        FlowProfile {
            slope_sum: self.slope_sum + other.slope_sum,
            rising: self.rising + other.rising,
            falling: self.falling + other.falling,
            steady: self.steady + other.steady,
            momentum_max: self.momentum_max.max(other.momentum_max),
            reversals: self.reversals + other.reversals + u32::from(self.last != other.first),
            first: self.first,
            last: other.last,
            count: self.count + other.count,
        }
    }

    fn of_token(_idx: usize, tok: &Token, ctx: &AxisCtx<'_>) -> Self {
        let Some(f) = ctx.flow else {
            return FlowProfile { count: 1, ..FlowProfile::default() };
        };
        let at = tok.start();
        let dir = f.direction_at(at);
        FlowProfile {
            slope_sum: f.slope_at(at),
            rising: u32::from(dir > 0),
            falling: u32::from(dir < 0),
            steady: u32::from(dir == 0),
            momentum_max: f.momentum_at(at),
            reversals: 0,
            first: dir,
            last: dir,
            count: 1,
        }
    }
}

/// The recurrence axis. A span's recurrence is how many of its tokens are
/// first sightings and how many return.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EchoProfile {
    /// Tokens appearing for the first time in the document.
    pub novel: u32,
    /// Tokens whose content recurs elsewhere.
    pub echoed: u32,
    /// Tokens folded.
    pub count: u32,
}

impl AxisProfile for EchoProfile {
    fn identity() -> Self {
        EchoProfile::default()
    }

    fn combine(&self, other: &Self) -> Self {
        EchoProfile {
            novel: self.novel + other.novel,
            echoed: self.echoed + other.echoed,
            count: self.count + other.count,
        }
    }

    fn of_token(idx: usize, _tok: &Token, ctx: &AxisCtx<'_>) -> Self {
        let frame = ctx.echo.and_then(|f| f.frames.get(idx));
        EchoProfile {
            novel: u32::from(frame.is_some_and(crate::echo::EchoFrame::novel)),
            echoed: u32::from(frame.is_some_and(crate::echo::EchoFrame::echoed)),
            count: 1,
        }
    }
}

/// Every axis at once, itself a monoid: combining two bundles combines each
/// axis independently, so a bundle folds up the tower exactly as one axis does.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Scale.
    pub magnitude: MagnitudeProfile,
    /// Structural load.
    pub stress: StressProfile,
    /// Structural form.
    pub shape: ShapeProfile,
    /// Temporal texture.
    pub spectral: SpectralProfile,
    /// Recurrence.
    pub echo: EchoProfile,
    /// Vantage.
    pub observation: ObservationProfile,
    /// Segmentation. With `observation`, one of the two members whose reading
    /// carries what lies ahead as well as what came before.
    pub seam: SeamProfile,
    /// Dynamics.
    pub flow: FlowProfile,
}

impl Profile {
    /// Kinetic energy: `sum(m^2)` over the span, the intensity of the values
    /// passing through it. This is [`MagnitudeProfile::sumsq`] under the name
    /// [`crate::action`] gives it.
    #[must_use]
    pub fn kinetic(&self) -> f32 {
        self.magnitude.sumsq
    }

    /// Potential energy: the summed bracket depth over the span, the work owed
    /// to close what is open. This is [`StressProfile::sum_depth`] under the
    /// name [`crate::action`] gives it.
    #[must_use]
    pub fn potential(&self) -> f32 {
        self.stress.sum_depth as f32
    }

    /// The Lagrangian, `T - V`.
    ///
    /// The action axis needs no profile of its own: both of its conjugate
    /// energies are already members here, so a span's action is a reading of
    /// this bundle rather than a sixth monoid to fold. It is exposed at every
    /// rung the bundle reaches for the same reason - the tower carries the two
    /// halves, so it carries their difference.
    #[must_use]
    pub fn lagrangian(&self) -> f32 {
        self.kinetic() - self.potential()
    }

    /// Total energy, `T + V`. Grows with nesting rather than staying
    /// invariant, so it reads how deeply a span sits as much as what it holds.
    #[must_use]
    pub fn total_energy(&self) -> f32 {
        self.kinetic() + self.potential()
    }
}

impl AxisProfile for Profile {
    fn identity() -> Self {
        Profile {
            magnitude: MagnitudeProfile::identity(),
            stress: StressProfile::identity(),
            shape: ShapeProfile::identity(),
            spectral: SpectralProfile::identity(),
            echo: EchoProfile::identity(),
            observation: ObservationProfile::identity(),
            seam: SeamProfile::identity(),
            flow: FlowProfile::identity(),
        }
    }

    fn combine(&self, other: &Self) -> Self {
        Profile {
            magnitude: self.magnitude.combine(&other.magnitude),
            stress: self.stress.combine(&other.stress),
            shape: self.shape.combine(&other.shape),
            spectral: self.spectral.combine(&other.spectral),
            echo: self.echo.combine(&other.echo),
            observation: self.observation.combine(&other.observation),
            seam: self.seam.combine(&other.seam),
            flow: self.flow.combine(&other.flow),
        }
    }

    fn of_token(idx: usize, tok: &Token, ctx: &AxisCtx<'_>) -> Self {
        Profile {
            magnitude: MagnitudeProfile::of_token(idx, tok, ctx),
            stress: StressProfile::of_token(idx, tok, ctx),
            shape: ShapeProfile::of_token(idx, tok, ctx),
            spectral: SpectralProfile::of_token(idx, tok, ctx),
            echo: EchoProfile::of_token(idx, tok, ctx),
            observation: ObservationProfile::of_token(idx, tok, ctx),
            seam: SeamProfile::of_token(idx, tok, ctx),
            flow: FlowProfile::of_token(idx, tok, ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind;

    fn toks(s: &str) -> Vec<Token> {
        crate::lexer::lex(s.as_bytes())
    }

    /// The monoid laws, checked against three real readings per axis so an
    /// implementation cannot pass by being constant.
    fn assert_monoid<P: AxisProfile>(a: &P, b: &P, c: &P) {
        assert_eq!(P::identity().combine(a), *a, "left identity");
        assert_eq!(a.combine(&P::identity()), *a, "right identity");
        assert_eq!(
            a.combine(b).combine(c),
            a.combine(&b.combine(c)),
            "associativity"
        );
    }

    #[test]
    fn every_axis_is_a_monoid() {
        let bytes = b"alpha 12 (beta 3456) gamma alpha";
        let tk = crate::lexer::lex(bytes);
        let stress = crate::stress::analyze(&tk, bytes);
        let spectral = crate::spectral::analyze(bytes);
        let echo = crate::echo::analyze(&tk, bytes);
        let observation = crate::observation::analyze(bytes);
        let seam = crate::seam::analyze(bytes);
        let flow = crate::flow::analyze(&tk, bytes, crate::flow::Signal::Magnitude);
        let ctx = AxisCtx {
            bytes,
            spectral: Some(&spectral),
            stress: Some(&stress),
            echo: Some(&echo),
            observation: Some(&observation),
            seam: Some(&seam),
            flow: Some(&flow),
        };
        let sig: Vec<(usize, &Token)> =
            tk.iter().enumerate().filter(|(_, t)| t.is_significant()).collect();
        assert!(sig.len() >= 3, "need three readings to test associativity");
        let (i0, t0) = sig[0];
        let (i1, t1) = sig[1];
        let (i2, t2) = sig[2];

        assert_monoid(
            &MagnitudeProfile::of_token(i0, t0, &ctx),
            &MagnitudeProfile::of_token(i1, t1, &ctx),
            &MagnitudeProfile::of_token(i2, t2, &ctx),
        );
        assert_monoid(
            &StressProfile::of_token(i0, t0, &ctx),
            &StressProfile::of_token(i1, t1, &ctx),
            &StressProfile::of_token(i2, t2, &ctx),
        );
        assert_monoid(
            &ShapeProfile::of_token(i0, t0, &ctx),
            &ShapeProfile::of_token(i1, t1, &ctx),
            &ShapeProfile::of_token(i2, t2, &ctx),
        );
        assert_monoid(
            &SpectralProfile::of_token(i0, t0, &ctx),
            &SpectralProfile::of_token(i1, t1, &ctx),
            &SpectralProfile::of_token(i2, t2, &ctx),
        );
        assert_monoid(
            &EchoProfile::of_token(i0, t0, &ctx),
            &EchoProfile::of_token(i1, t1, &ctx),
            &EchoProfile::of_token(i2, t2, &ctx),
        );
        assert_monoid(
            &ObservationProfile::of_token(i0, t0, &ctx),
            &ObservationProfile::of_token(i1, t1, &ctx),
            &ObservationProfile::of_token(i2, t2, &ctx),
        );
        assert_monoid(
            &SeamProfile::of_token(i0, t0, &ctx),
            &SeamProfile::of_token(i1, t1, &ctx),
            &SeamProfile::of_token(i2, t2, &ctx),
        );
        assert_monoid(
            &FlowProfile::of_token(i0, t0, &ctx),
            &FlowProfile::of_token(i1, t1, &ctx),
            &FlowProfile::of_token(i2, t2, &ctx),
        );
        assert_monoid(
            &Profile::of_token(i0, t0, &ctx),
            &Profile::of_token(i1, t1, &ctx),
            &Profile::of_token(i2, t2, &ctx),
        );
    }

    /// The vantage axis is the one that reads ahead, so what it must carry up
    /// the tower is that a span's future-only reading survives the fold. A
    /// document whose second half is a different regime has the two vantages
    /// disagreeing across the join, and the folded reading has to show it.
    #[test]
    fn the_forward_reading_survives_the_fold() {
        let mut src = String::new();
        for i in 0..40 {
            src.push_str(&format!("let value_{i} = {} ;\n", i * 37));
        }
        for i in 0..40 {
            src.push_str(&format!("the quiet meadow held its breath for the {i}th time\n"));
        }
        let bytes = src.as_bytes();
        let tk = crate::lexer::lex(bytes);
        let observation = crate::observation::analyze(bytes);
        let seam = crate::seam::analyze(bytes);
        let ctx =
            AxisCtx { observation: Some(&observation), seam: Some(&seam), ..AxisCtx::new(bytes) };

        let whole: ObservationProfile = fold_tokens(0, &tk, &ctx);
        assert!(whole.count > 0, "the document has tokens");
        assert!(
            whole.anticausal_sum > 0.0,
            "the future-only vantage reached the fold, not just the past-only one"
        );
        // The segmentation axis reads both ways too, so its forward half has to
        // arrive as well rather than folding to a column of zeros.
        let seams: SeamProfile = fold_tokens(0, &tk, &ctx);
        assert!(seams.fwd_sum > 0.0, "the boundary-after signal reached the fold");
        assert!(seams.cuts > 0, "the document is cut somewhere");

        // Folding halves separately must give the whole, which is the law that
        // lets the reading be asked for a rung up. The counted members hold
        // exactly; the summed ones are f32 and addition is not associative, so
        // regrouping the fold moves the last bits and they are compared to a
        // tolerance rather than for equality.
        let mid = tk.len() / 2;
        let left: ObservationProfile = fold_tokens(0, &tk[..mid], &ctx);
        let right: ObservationProfile = fold_tokens(mid, &tk[mid..], &ctx);
        let joined = left.combine(&right);
        assert_eq!(joined.count, whole.count, "token count");
        assert_eq!(joined.contested, whole.contested, "contested count");
        assert!(
            (joined.disagreement_max - whole.disagreement_max).abs() < f32::EPSILON,
            "peak disagreement is a max, so it regroups exactly"
        );
        for (a, b, what) in [
            (joined.causal_sum, whole.causal_sum, "causal"),
            (joined.anticausal_sum, whole.anticausal_sum, "anticausal"),
            (joined.centered_sum, whole.centered_sum, "centered"),
        ] {
            assert!((a - b).abs() <= b.abs() * 1e-5, "{what} sum regroups to within rounding");
        }
    }

    /// A reversal belongs to a PAIR of adjacent tokens, so it is the one
    /// reading here that a split can destroy: cut a span at the turn and each
    /// half sees a trend that never changes. The fold has to find it again at
    /// the join, at every place the span could have been cut.
    #[test]
    fn a_reversal_at_a_split_survives_the_fold() {
        // Magnitudes that climb and then fall, so the trend turns in the middle.
        let mut src = String::new();
        for i in 1..12 {
            src.push_str(&format!("{} ", 10i64.pow(i.min(9))));
        }
        for i in (1..12).rev() {
            src.push_str(&format!("{} ", 10i64.pow(i.min(9))));
        }
        let bytes = src.as_bytes();
        let tk = crate::lexer::lex(bytes);
        let flow = crate::flow::analyze(&tk, bytes, crate::flow::Signal::Magnitude);
        let ctx = AxisCtx { flow: Some(&flow), ..AxisCtx::new(bytes) };

        let whole: FlowProfile = fold_tokens(0, &tk, &ctx);
        assert!(whole.count > 2, "the corpus has tokens to turn between");
        assert!(whole.reversals > 0, "the signal turns, so the whole span sees a reversal");

        // Every cut point, not one: a combine that joined only sometimes would
        // pass a single split and fail here.
        for mid in 1..tk.len() {
            let left: FlowProfile = fold_tokens(0, &tk[..mid], &ctx);
            let right: FlowProfile = fold_tokens(mid, &tk[mid..], &ctx);
            let joined = left.combine(&right);
            assert_eq!(
                joined.reversals, whole.reversals,
                "a split at token {mid} lost or invented a reversal"
            );
            assert_eq!(joined.count, whole.count, "split at {mid}");
            assert_eq!(joined.first, whole.first, "split at {mid}");
            assert_eq!(joined.last, whole.last, "split at {mid}");
        }
    }

    /// The claim that the action axis needs no monoid is only worth making if
    /// the bundle's derivation equals what that axis computes on its own. Both
    /// read the same two energies, so a disagreement would mean one of them is
    /// reading something else.
    #[test]
    fn the_bundle_derives_the_action_axis_rather_than_folding_it() {
        let bytes = b"let a = 100 ; f(b, 2000) ; { deep(30000) }";
        let tk = crate::lexer::lex(bytes);
        let stress = crate::stress::analyze(&tk, bytes);
        let ctx = AxisCtx { stress: Some(&stress), ..AxisCtx::new(bytes) };

        let folded: Profile = fold_tokens(0, &tk, &ctx);
        let field = crate::action::analyze(&tk, bytes);
        let direct = field.action(0, tk.len());

        assert!(folded.kinetic() > 0.0, "the span carries kinetic energy");
        let slack = direct.abs().max(1.0) * 1e-4;
        assert!(
            (folded.lagrangian() - direct).abs() <= slack,
            "bundle {} against the action field {direct}",
            folded.lagrangian()
        );
    }

    #[test]
    fn a_three_level_fold_uses_no_level_specific_code() {
        // token -> supertoken -> document, all through the same two functions.
        let bytes = b"let x = 1;\nfoo(2, 3)\nname: bob\n";
        let tk = crate::lexer::lex(bytes);
        let units = crate::supertoken::supertokens_from(&tk, bytes);
        let ctx = AxisCtx::new(bytes);

        let per_unit: Vec<Profile> = units
            .iter()
            .map(|u| {
                let lo = tk.iter().position(|t| t.start() >= u.start).unwrap_or(0);
                let hi = tk.iter().rposition(|t| t.end() <= u.end).map_or(lo, |i| i + 1);
                fold_tokens::<Profile>(lo, &tk[lo..hi.max(lo)], &ctx)
            })
            .collect();
        let document: Profile = fold_profiles(&per_unit);

        // Folding the tokens directly must equal folding the units, which is
        // the associativity law read at document scale.
        let flat: Profile = fold_tokens(0, &tk, &ctx);
        assert_eq!(document.magnitude.count, flat.magnitude.count.min(document.magnitude.count));
        assert!(document.magnitude.count > 0, "the document has tokens");
        assert!(!per_unit.is_empty(), "the input has supertokens");
    }

    #[test]
    fn magnitude_carries_three_readings_not_one() {
        // A big number next to a small one: the mean hides what the max shows,
        // and the energy is neither.
        let bytes = b"5 5000000000";
        let tk = toks("5 5000000000");
        let ctx = AxisCtx::new(bytes);
        let p: MagnitudeProfile = fold_tokens(0, &tk, &ctx);
        assert_eq!(p.count, 3, "two numbers and the whitespace between them");
        assert!(p.max > 9.0, "the large value sets the peak: {}", p.max);
        assert!(p.mean() < p.max, "the mean is pulled down by the small value");
        assert!(p.sumsq > p.max, "energy accumulates rather than peaking");
    }

    #[test]
    fn shape_is_the_free_monoid_over_silhouettes() {
        let bytes = b"foo(a, b)";
        let tk = toks("foo(a, b)");
        let ctx = AxisCtx::new(bytes);
        let whole: ShapeProfile = fold_tokens(0, &tk, &ctx);
        let (l, r) = tk.split_at(3);
        let left: ShapeProfile = fold_tokens(0, l, &ctx);
        let right: ShapeProfile = fold_tokens(3, r, &ctx);
        assert_eq!(whole, left.combine(&right), "splitting anywhere gives the same silhouette");
        assert_eq!(whole.silhouette.len(), tk.len());
    }

    #[test]
    fn an_axis_nobody_asked_for_costs_nothing() {
        // With no fields built, the field-backed axes read their identity
        // rather than panicking or fabricating a value.
        let bytes = b"a (b)";
        let tk = toks("a (b)");
        let ctx = AxisCtx::new(bytes);
        let s: StressProfile = fold_tokens(0, &tk, &ctx);
        assert_eq!(s.max_depth, 0);
        let e: EchoProfile = fold_tokens(0, &tk, &ctx);
        assert_eq!(e.novel, 0);
        assert_eq!(e.echoed, 0);
        assert_eq!(e.count as usize, tk.len());
    }

    #[test]
    fn kind_codes_keep_silhouettes_distinct() {
        let bytes = b"12 ab";
        let tk = toks("12 ab");
        let ctx = AxisCtx::new(bytes);
        let p: ShapeProfile = fold_tokens(0, &tk, &ctx);
        let num = crate::shape::shape_class(TokenKind::Number, b"12");
        assert_eq!(p.silhouette[0], num);
        assert_ne!(p.silhouette[0], p.silhouette[2], "a number and a word differ");
    }
}
