//! The rolling context: every axis folded over a window that slides one unit
//! at a time, at the token rung and the unit rung, and the agreement of the
//! grains' own boundaries.
//!
//! [`crate::profile`] gives each axis a monoid and folds it up the tower; a
//! fold is a prefix, though, and a position's context is a window - the last
//! `w` tokens, the last `u` units. Refolding a window at every position is
//! `w` combines a step. A monoid has no inverse, so the window cannot subtract
//! the unit leaving it; what it can do is keep two stacks, one holding suffix
//! folds of the older half and one a running fold of the newer half, so the
//! window's fold is one combine and each unit is folded at most twice in its
//! life. That is [`Window`], and it needs nothing of an axis beyond
//! associativity.
//!
//! The context at a token is the fold of the token window ending there; the
//! context at a unit is the fold of the unit window ending there, where each
//! unit's own reading is the fold of its tokens - the same fold, one rung up,
//! which is what the tower promises.
//!
//! The grains also each place boundaries: the byte grain's spectral
//! change-points, the token grain's shape change-points and seam cuts, the
//! unit grain's unit starts. [`Agreement`] reads, per unit, which lower-grain
//! boundaries are at its start. It compares the same reading - a regime
//! change - re-grounded at three grains, never two different axes, so it has
//! units; a shuffled stream is the null it is tested against.

use crate::profile::{
    AxisCtx, AxisProfile, EchoProfile, FlowProfile, MagnitudeProfile, ObservationProfile,
    SeamProfile, SpectralProfile, StressProfile,
};
use crate::supertoken::{Role, SuperContext, SuperToken};
use crate::token::Token;

/// A sliding window over a monoid: the fold of the last `capacity` pushed
/// values, in push order, with each value folded at most twice.
///
/// `back` holds the newer values with `back_fold` their running fold; `front`
/// holds suffix folds of the older values, oldest last, so the oldest value's
/// entry is the fold of everything in `front`. When `front` runs out the
/// whole of `back` moves across as suffix folds. The window's fold is
/// `front.last() ⊕ back_fold`.
pub struct Window<P: AxisProfile> {
    capacity: usize,
    back: Vec<P>,
    back_fold: P,
    front: Vec<P>,
}

impl<P: AxisProfile> Window<P> {
    /// An empty window that holds at most `capacity` values.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Window {
            capacity,
            back: Vec::with_capacity(capacity),
            back_fold: P::identity(),
            front: Vec::with_capacity(capacity),
        }
    }

    /// Values in the window.
    #[must_use]
    pub fn len(&self) -> usize {
        self.front.len() + self.back.len()
    }

    /// Whether the window holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Push the newest value, dropping the oldest when the window is full.
    pub fn push(&mut self, value: P) {
        if self.len() == self.capacity {
            self.pop();
        }
        self.back_fold = self.back_fold.combine(&value);
        self.back.push(value);
    }

    /// Drop the oldest value.
    fn pop(&mut self) {
        if self.front.is_empty() {
            let mut suffix = P::identity();
            for value in self.back.iter().rev() {
                suffix = value.combine(&suffix);
                self.front.push(suffix.clone());
            }
            self.back.clear();
            self.back_fold = P::identity();
        }
        self.front.pop();
    }

    /// The fold of the window, oldest to newest.
    #[must_use]
    pub fn fold(&self) -> P {
        match self.front.last() {
            Some(older) => older.combine(&self.back_fold),
            None => self.back_fold.clone(),
        }
    }
}

/// The axes whose readings are a fixed number of words, bundled: what a
/// rolling window folds at a constant cost per step.
///
/// The shape axis is left out on purpose. Its monoid is the free monoid, a
/// span's silhouette being its tokens' silhouettes in order, so a window of
/// it is the silhouette slice itself and folding would copy it at every step.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowProfile {
    /// Scale.
    pub magnitude: MagnitudeProfile,
    /// Structural load.
    pub stress: StressProfile,
    /// Temporal texture.
    pub spectral: SpectralProfile,
    /// Recurrence.
    pub echo: EchoProfile,
    /// Vantage: what the past, the future and a centered view each said.
    pub observation: ObservationProfile,
    /// Segmentation, read in both directions.
    pub seam: SeamProfile,
    /// Dynamics.
    pub flow: FlowProfile,
}

impl AxisProfile for WindowProfile {
    fn identity() -> Self {
        WindowProfile {
            magnitude: MagnitudeProfile::identity(),
            stress: StressProfile::identity(),
            spectral: SpectralProfile::identity(),
            echo: EchoProfile::identity(),
            observation: ObservationProfile::identity(),
            seam: SeamProfile::identity(),
            flow: FlowProfile::identity(),
        }
    }

    fn combine(&self, other: &Self) -> Self {
        WindowProfile {
            magnitude: self.magnitude.combine(&other.magnitude),
            stress: self.stress.combine(&other.stress),
            spectral: self.spectral.combine(&other.spectral),
            echo: self.echo.combine(&other.echo),
            observation: self.observation.combine(&other.observation),
            seam: self.seam.combine(&other.seam),
            flow: self.flow.combine(&other.flow),
        }
    }

    fn of_token(idx: usize, tok: &Token, ctx: &AxisCtx<'_>) -> Self {
        WindowProfile {
            magnitude: MagnitudeProfile::of_token(idx, tok, ctx),
            stress: StressProfile::of_token(idx, tok, ctx),
            spectral: SpectralProfile::of_token(idx, tok, ctx),
            echo: EchoProfile::of_token(idx, tok, ctx),
            observation: ObservationProfile::of_token(idx, tok, ctx),
            seam: SeamProfile::of_token(idx, tok, ctx),
            flow: FlowProfile::of_token(idx, tok, ctx),
        }
    }
}

/// Window extents, in units of the rung each applies to.
#[derive(Clone, Copy, Debug)]
pub struct ContextConfig {
    /// Significant tokens the token-rung window spans, the current one
    /// included.
    pub token_window: usize,
    /// Units the unit-rung window spans, the current one included.
    pub unit_window: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        // The token window is the observation reader's, so the two readings
        // of a position span the same tokens. The unit window covers the
        // same stretch one rung up: the code corpus in `benches/axis_grain`
        // lexes to 16500 significant tokens in 4000 units, four to a unit.
        let token_window = crate::observation::ObservationConfig::default().window;
        ContextConfig { token_window, unit_window: (token_window / 4).max(1) }
    }
}

/// Where each lower grain's nearest boundary is relative to a unit's
/// start, in significant tokens: negative before it, zero at it, positive
/// after. `None` when that grain placed no boundary at all.
///
/// A distance rather than a hit: a grain's reader lands its boundary at a
/// fixed offset from the construct it reads - the token-grain seam cuts one
/// token into a statement on the code corpus, never at its first token - and
/// what says the grains are reading the same structure is that the offset
/// repeats, not that it is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Agreement {
    /// The unit's role, which is what its boundary offsets are read against:
    /// a grain reads a call and a binding at different places.
    pub role: Role,
    /// The byte grain's nearest spectral change-point.
    pub regime: Option<i32>,
    /// The token grain's nearest shape change-point.
    pub shape: Option<i32>,
    /// The token grain's nearest seam cut.
    pub seam: Option<i32>,
}

/// The rolling context of a stream.
#[derive(Clone, Debug, Default)]
pub struct ContextField {
    /// The token-rung window's fold at each token index. A whitespace token
    /// carries the reading of the significant token before it.
    pub at_token: Vec<WindowProfile>,
    /// Each unit's own reading: the fold of its tokens.
    pub of_unit: Vec<WindowProfile>,
    /// The unit-rung window's fold at each unit index.
    pub at_unit: Vec<WindowProfile>,
    /// Per unit, which lower grains place a boundary at its start.
    pub agreement: Vec<Agreement>,
}

impl ContextField {
    /// How consistently each lower grain's boundaries are at one offset from
    /// the starts of units of one role: the share of units whose nearest
    /// boundary of that grain is at the modal offset for their role, as
    /// `(regime, shape, seam)`, each in `[0, 1]`. A grain that reads the
    /// same construct the unit rung reads lands at one place per role; on a
    /// stream with no construct to read its offsets spread. Zeros for no
    /// units.
    #[must_use]
    pub fn alignment(&self) -> (f32, f32, f32) {
        let n = self.agreement.len();
        if n == 0 {
            return (0.0, 0.0, 0.0);
        }
        let modal_share = |offset: fn(&Agreement) -> Option<i32>| {
            let mut counts: std::collections::HashMap<(u32, i32), u32> =
                std::collections::HashMap::new();
            for a in &self.agreement {
                if let Some(o) = offset(a) {
                    *counts.entry((a.role.code(), o)).or_insert(0) += 1;
                }
            }
            let mut modal: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
            for (&(role, _), &c) in &counts {
                let m = modal.entry(role).or_insert(0);
                *m = (*m).max(c);
            }
            modal.values().sum::<u32>() as f32 / n as f32
        };
        (modal_share(|a| a.regime), modal_share(|a| a.shape), modal_share(|a| a.seam))
    }
}

/// Fold the rolling windows over an already-lexed stream.
///
/// `ctx` carries the axis fields the readings draw on; a field left out reads
/// as that axis's identity, as in [`crate::profile`]. `supers` is the unit
/// rung. The agreement is not read here: it needs the boundary fields, which
/// [`agreement`] takes.
#[must_use]
pub fn fold_windows(
    toks: &[Token],
    ctx: &AxisCtx<'_>,
    supers: &SuperContext,
    cfg: &ContextConfig,
) -> ContextField {
    fold_range(toks, ctx, supers, cfg, 0..toks.len(), 0)
}

/// Significant tokens a leaf of the parallel fold covers, at the least. A
/// leaf then takes about what a leaf of the parallel blob pass takes - 128
/// microseconds, at the 90 nanoseconds a token `benches/context_window`
/// measures - so the dispatch is amortised the same way.
const FOLD_MIN_LEAF_TOKENS: usize = 1024;

/// [`fold_windows`] across cores.
///
/// The stream is cut at unit starts into a few chunks per core, and each
/// chunk seeds its token window from the tokens before it and its unit
/// window from the units before it, so it reads what the one-thread walk
/// reads there. The readings agree with the serial fold up to the
/// association of a floating sum, which the seeded window brackets
/// differently.
#[must_use]
pub fn fold_windows_parallel(
    toks: &[Token],
    ctx: &AxisCtx<'_>,
    supers: &SuperContext,
    cfg: &ContextConfig,
) -> ContextField {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let n_units = supers.units.len();
    let units_per_chunk = n_units.div_ceil(cores * 4).max(1);
    // Chunks as (first unit, first token) pairs; a chunk runs to the next
    // chunk's first token. A chunk holds whole units, at least
    // `units_per_chunk` of them and at least a leaf's worth of tokens.
    let mut chunks: Vec<(usize, usize)> = Vec::new();
    let mut u = 0usize;
    while u < n_units {
        let tok_lo = first_token(toks, supers, u);
        chunks.push((u, tok_lo));
        let mut v = u + 1;
        while v < n_units
            && (v - u < units_per_chunk || first_token(toks, supers, v) - tok_lo < FOLD_MIN_LEAF_TOKENS)
        {
            v += 1;
        }
        u = v;
    }
    if chunks.len() <= 1 || cores <= 1 {
        return fold_windows(toks, ctx, supers, cfg);
    }
    let bounds: Vec<(usize, usize, usize)> = chunks
        .iter()
        .enumerate()
        .map(|(k, &(unit_lo, tok_lo))| {
            let tok_hi = chunks.get(k + 1).map_or(toks.len(), |&(_, t)| t);
            (unit_lo, tok_lo, tok_hi)
        })
        .collect();
    let mut parts: Vec<ContextField> = bounds.iter().map(|_| ContextField::default()).collect();
    // A leaf reads each token's bytes and axis frames once and writes a
    // reading per token: streaming work, and the shape is what says so. No
    // cost is named here because none was ever measured for it, and a named
    // one replaces the scheduler's own probe rather than informing it.
    let plan = JobPlan::new(0, bounds.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::Streaming);
    for_each_chunk_indexed_min_leaf(&plan, &mut parts, 1, |start, slots| {
        for (i, slot) in slots.iter_mut().enumerate() {
            let (unit_lo, tok_lo, tok_hi) = bounds[start + i];
            *slot = fold_range(toks, ctx, supers, cfg, tok_lo..tok_hi, unit_lo);
        }
    });
    // The first chunk starts at the first unit; tokens before it carry the
    // empty reading, as in the serial walk.
    let lead = bounds.first().map_or(0, |b| b.1);
    let mut out = ContextField {
        at_token: Vec::with_capacity(toks.len()),
        of_unit: Vec::with_capacity(n_units),
        at_unit: Vec::with_capacity(n_units),
        agreement: Vec::new(),
    };
    out.at_token.extend(std::iter::repeat_n(WindowProfile::identity(), lead));
    for part in parts {
        out.at_token.extend(part.at_token);
        out.of_unit.extend(part.of_unit);
        out.at_unit.extend(part.at_unit);
    }
    out
}

/// The index of unit `u`'s first token.
fn first_token(toks: &[Token], supers: &SuperContext, u: usize) -> usize {
    toks.partition_point(|t| t.start() < supers.units[u].start)
}

/// The fold over `tokens`, a range starting at a unit's first token (or at
/// the stream's start), with `unit_lo` the unit that starts there. The
/// windows are seeded from what precedes the range so the readings inside it
/// are those of the whole-stream walk.
fn fold_range(
    toks: &[Token],
    ctx: &AxisCtx<'_>,
    supers: &SuperContext,
    cfg: &ContextConfig,
    tokens: std::ops::Range<usize>,
    unit_lo: usize,
) -> ContextField {
    let mut window: Window<WindowProfile> = Window::new(cfg.token_window);
    // The token window holds the significant tokens before the range, as
    // many as it can, oldest first.
    let seed: Vec<usize> = (0..tokens.start)
        .rev()
        .filter(|&i| toks[i].is_significant())
        .take(cfg.token_window)
        .collect();
    for &i in seed.iter().rev() {
        window.push(WindowProfile::of_token(i, &toks[i], ctx));
    }
    let mut current = window.fold();
    // Each unit's tokens fold as the walk passes them, so a unit's reading is
    // ready when its last token is; the unit window takes it then. It is
    // seeded with the units before the range, each folded from its tokens.
    let mut unit_window: Window<WindowProfile> = Window::new(cfg.unit_window);
    for u in unit_lo.saturating_sub(cfg.unit_window)..unit_lo {
        let lo = first_token(toks, supers, u);
        let hi = first_token(toks, supers, u + 1);
        let reading = (lo..hi)
            .filter(|&i| toks[i].is_significant() && supers.index_of(i) == Some(u))
            .fold(WindowProfile::identity(), |acc, i| acc.combine(&WindowProfile::of_token(i, &toks[i], ctx)));
        unit_window.push(reading);
    }
    let mut at_token: Vec<WindowProfile> = Vec::with_capacity(tokens.len());
    let mut of_unit: Vec<WindowProfile> = Vec::new();
    let mut at_unit: Vec<WindowProfile> = Vec::new();
    let mut unit_fold = WindowProfile::identity();
    let mut open_unit: Option<usize> = None;
    let close_unit = |unit_fold: &mut WindowProfile,
                      of_unit: &mut Vec<WindowProfile>,
                      at_unit: &mut Vec<WindowProfile>,
                      unit_window: &mut Window<WindowProfile>| {
        let reading = std::mem::replace(unit_fold, WindowProfile::identity());
        unit_window.push(reading.clone());
        of_unit.push(reading);
        at_unit.push(unit_window.fold());
    };
    for i in tokens {
        let tok = &toks[i];
        if tok.is_significant() {
            let reading = WindowProfile::of_token(i, tok, ctx);
            window.push(reading.clone());
            current = window.fold();
            match (supers.index_of(i), open_unit) {
                (Some(u), Some(open)) if u == open => {
                    unit_fold = unit_fold.combine(&reading);
                }
                (Some(u), open) => {
                    if open.is_some() {
                        close_unit(&mut unit_fold, &mut of_unit, &mut at_unit, &mut unit_window);
                    }
                    open_unit = Some(u);
                    unit_fold = reading;
                }
                (None, Some(_)) => {
                    close_unit(&mut unit_fold, &mut of_unit, &mut at_unit, &mut unit_window);
                    open_unit = None;
                }
                (None, None) => {}
            }
        }
        at_token.push(current.clone());
    }
    if open_unit.is_some() {
        close_unit(&mut unit_fold, &mut of_unit, &mut at_unit, &mut unit_window);
    }
    ContextField { at_token, of_unit, at_unit, agreement: Vec::new() }
}

/// Per unit, where each lower grain's nearest boundary is relative to the
/// unit's start.
///
/// `regime_cuts` are the byte grain's change-points, `shape_cuts` the token
/// grain's, both as byte offsets ascending; `seam_cuts` are the token grain's
/// seam cuts as the byte offsets of token starts, ascending. Every boundary
/// is placed on the significant token holding it, so the three offsets are
/// in one unit whatever grain they came from.
#[must_use]
pub fn agreement(
    units: &[SuperToken],
    toks: &[Token],
    regime_cuts: &[usize],
    shape_cuts: &[usize],
    seam_cuts: &[usize],
) -> Vec<Agreement> {
    // Significant-token starts, so a byte offset maps to the significant
    // token holding it by one search.
    let starts: Vec<usize> =
        toks.iter().filter(|t| t.is_significant()).map(Token::start).collect();
    let sig_index = |byte: usize| starts.partition_point(|&s| s <= byte).saturating_sub(1);
    let place = |cuts: &[usize]| -> Vec<usize> { cuts.iter().map(|&c| sig_index(c)).collect() };
    let (regime, shape, seam) = (place(regime_cuts), place(shape_cuts), place(seam_cuts));
    let nearest = |placed: &[usize], at: usize| -> Option<i32> {
        if placed.is_empty() {
            return None;
        }
        let i = placed.partition_point(|&p| p < at);
        let after = placed.get(i).map(|&p| p as i64 - at as i64);
        let before = i.checked_sub(1).map(|j| placed[j] as i64 - at as i64);
        let offset = match (before, after) {
            (Some(b), Some(a)) => if a.abs() < b.abs() { a } else { b },
            (Some(b), None) => b,
            (None, Some(a)) => a,
            (None, None) => return None,
        };
        Some(offset.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
    };
    units
        .iter()
        .map(|u| {
            let at = sig_index(u.start);
            Agreement {
                role: u.role,
                regime: nearest(&regime, at),
                shape: nearest(&shape, at),
                seam: nearest(&seam, at),
            }
        })
        .collect()
}

/// The readings of a token's related context: one fold per relation that
/// admits earlier tokens, each over every token the relation names.
///
/// A window admits by distance. These admit by relation, and because a
/// reading is a monoid the context needs no members kept and none evicted:
/// each is a running fold, exact and unbounded in range, that costs one
/// combine when a token joins it.
#[derive(Clone, Debug, PartialEq)]
pub struct RelationReading {
    /// The heads of the brackets enclosing the token, outermost first: the
    /// path up the enclosure tree.
    pub enclosing: WindowProfile,
    /// Every earlier occurrence of the token's key, the token itself left
    /// out; identity for a first appearance or an unkeyed token.
    pub echoing: WindowProfile,
    /// The significant tokens since the byte grain's last regime change,
    /// the token itself left out.
    pub regime: WindowProfile,
    /// Every earlier significant token at the same phase of the dominant
    /// token-kind period, the token itself left out; identity when the stream
    /// has no period.
    pub phase: WindowProfile,
}

/// The related context at every token.
#[derive(Clone, Debug, Default)]
pub struct RelationContext {
    /// One reading per token index. A whitespace token carries the reading
    /// of the significant token before it.
    pub at_token: Vec<RelationReading>,
    /// Per token index, the token's phase of the dominant period - its index
    /// among the significant tokens modulo the period - or `u16::MAX` for an
    /// insignificant token or a stream with no period.
    pub phase_of: Vec<u16>,
    /// Per token index, the fold over the values bound to earlier occurrences
    /// of the token's key: for each earlier occurrence that is the left
    /// operand of a binding punctuation (`=`, `:`), the reading of the right
    /// operand. Identity for an unkeyed token, a first occurrence, or a key
    /// never bound before.
    pub value_history: Vec<WindowProfile>,
    /// The dominant token-kind period the phase fold is keyed on, when the
    /// stream has one.
    pub period: Option<u16>,
}

impl Default for RelationReading {
    fn default() -> Self {
        RelationReading {
            enclosing: WindowProfile::identity(),
            echoing: WindowProfile::identity(),
            regime: WindowProfile::identity(),
            phase: WindowProfile::identity(),
        }
    }
}

/// Fold the related context over an already-lexed stream.
///
/// `regime_cuts` are the byte grain's change-points ascending; `echo` is
/// the recurrence field, whose back lags name each token's previous
/// occurrence; `period` is the record period over shape classes, from
/// [`record_period`], which is what both call sites pass. The token readings
/// are made across cores; the folds themselves are chains and run in one pass
/// over them.
#[must_use]
pub fn relate(
    toks: &[Token],
    ctx: &AxisCtx<'_>,
    regime_cuts: &[usize],
    echo: &crate::echo::EchoField,
    period: Option<u16>,
) -> RelationContext {
    use crate::token::TokenKind;

    let n = toks.len();
    let readings = token_readings(toks, ctx);
    let mut at_token: Vec<RelationReading> = Vec::with_capacity(n);
    let mut phase_of: Vec<u16> = vec![u16::MAX; n];
    let mut value_history: Vec<WindowProfile> = vec![WindowProfile::identity(); n];
    // The enclosure path as a stack of prefix folds: an open bracket pushes
    // the fold so far combined with its head's reading, a close pops.
    let mut enclosure: Vec<WindowProfile> = Vec::new();
    let mut heads: Vec<bool> = Vec::new();
    // Each keyed token's fold over its earlier occurrences, by token index,
    // so the next occurrence extends it in one step; likewise the right
    // operand each token binds, so a later occurrence of its key extends the
    // key's value history in one step.
    let mut echo_fold: Vec<WindowProfile> = vec![WindowProfile::identity(); n];
    let mut operand_of: Vec<Option<usize>> = vec![None; n];
    let starts: Vec<usize> = toks.iter().map(Token::start).collect();
    let mut regime = WindowProfile::identity();
    let mut next_cut = regime_cuts.partition_point(|&c| c == 0);
    let p = period.map_or(0, usize::from);
    let mut phase_fold: Vec<WindowProfile> = vec![WindowProfile::identity(); p];
    let mut sig_index = 0usize;
    let mut current = RelationReading::default();
    let mut last_sig: Option<usize> = None;
    for (i, tok) in toks.iter().enumerate() {
        // The significant token before this one, which is what heads an
        // opening bracket here and what a binding punctuation binds.
        let prev_sig = last_sig;
        if matches!(tok.kind, TokenKind::Close(_)) && heads.pop().is_some() {
            enclosure.pop();
        }
        if tok.is_significant() {
            let reading = &readings[i];
            // A binding punctuation with an operand on each side binds the
            // right one to the left one, as the relation tier reads it.
            if tok.kind == TokenKind::Punct
                && matches!(&ctx.bytes[tok.span()], b"=" | b":")
                && let Some(l) = prev_sig
                && let Some(r) = (i + 1..n).find(|&j| toks[j].is_significant())
            {
                operand_of[l] = Some(r);
            }
            // The regime fold resets at a change-point the token start has
            // passed; the reading is what came before the token.
            while next_cut < regime_cuts.len() && regime_cuts[next_cut] <= tok.start() {
                regime = WindowProfile::identity();
                next_cut += 1;
            }
            let regime_before = regime.clone();
            regime = regime.combine(reading);
            let frame = echo.frames.get(i);
            let echoing = match frame.and_then(|f| f.back_lag) {
                Some(lag) => {
                    let prev = starts.partition_point(|&s| s < tok.start() - lag.get() as usize);
                    let fold = echo_fold[prev].combine(&readings[prev]);
                    echo_fold[i] = fold.clone();
                    let bound = &value_history[prev];
                    value_history[i] = match operand_of[prev] {
                        Some(r) => bound.combine(&readings[r]),
                        None => bound.clone(),
                    };
                    fold
                }
                None => WindowProfile::identity(),
            };
            let phase = if p > 0 {
                let k = sig_index % p;
                phase_of[i] = k as u16;
                let before = phase_fold[k].clone();
                phase_fold[k] = before.combine(reading);
                before
            } else {
                WindowProfile::identity()
            };
            sig_index += 1;
            current = RelationReading {
                enclosing: enclosure.last().cloned().unwrap_or_else(WindowProfile::identity),
                echoing,
                regime: regime_before,
                phase,
            };
            last_sig = Some(i);
        }
        if matches!(tok.kind, TokenKind::Open(_)) {
            // The head is the significant word just before the bracket, as
            // the relation tier reads it.
            let head = prev_sig.filter(|&j| matches!(toks[j].kind, TokenKind::Word));
            let below = enclosure.last().cloned().unwrap_or_else(WindowProfile::identity);
            enclosure.push(match head {
                Some(h) => below.combine(&readings[h]),
                None => below,
            });
            heads.push(true);
        }
        at_token.push(current.clone());
    }
    RelationContext { at_token, phase_of, value_history, period }
}

/// Every token's reading, made across cores; an insignificant token's is the
/// identity.
fn token_readings(toks: &[Token], ctx: &AxisCtx<'_>) -> Vec<WindowProfile> {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    let mut readings: Vec<WindowProfile> = vec![WindowProfile::identity(); toks.len()];
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let min_leaf = toks.len().div_ceil(cores * 4).max(FOLD_MIN_LEAF_TOKENS);
    let plan = JobPlan::new(0, toks.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::Streaming);
    for_each_chunk_indexed_min_leaf(&plan, &mut readings, min_leaf, |start, slots| {
        for (k, slot) in slots.iter_mut().enumerate() {
            let i = start + k;
            if toks[i].is_significant() {
                *slot = WindowProfile::of_token(i, &toks[i], ctx);
            }
        }
    });
    readings
}

/// The related context of `bytes` with every field it reads built.
#[must_use]
pub fn relate_bytes(bytes: &[u8]) -> RelationContext {
    let toks = crate::lexer::lex(bytes);
    let fields = Fields::build(&toks, bytes);
    relate(&toks, &fields.ctx(bytes), &fields.spectral.boundaries, &fields.echo, record_period(&toks, bytes))
}

/// Longest record period searched, in significant tokens: the bank the axis
/// benches read the token stream with.
pub const PHASE_MAX_PERIOD: u16 = 32;

/// How far above chance a lag's share must be before the reading is kept.
///
/// This gate applies alongside `spectral::PERIOD_FLOOR` rather than replacing it,
/// and they refuse different things: the floor demands that the stream repeat
/// at all, this demands that it repeat more than its own alphabet repeats by
/// coincidence. The floor alone cannot do the second, because the share a lag
/// reaches by chance is [`record_period_chance`], which moves with the
/// silhouette alphabet - 0.10 on code, 0.30 on prose, 0.41 on a log - so a
/// single share is above chance on one corpus and below it on another. At
/// 0.20 it is below the chance rate of prose, a log and a table, and on those
/// every lag clears it.
///
/// Measured over seven real corpora, whole and in slices at 64k, 256k and 1M
/// by `examples/what_the_resonant_period_changes`: prose reads 1.03 to 1.24 at
/// every size and offset, and every corpus with structure in it reads 1.61 and
/// up, with nothing between. This is in that gap. The gap is clean but it is
/// seven corpora, so a genre that reads between them is the thing that would
/// move it.
pub const PERIOD_LIFT_FLOOR: f32 = 1.4;

/// The period of the stream's records, in significant tokens, when it has
/// one: the phase fold's key.
///
/// The lag at which the stream of token silhouettes best repeats itself:
/// the share of positions whose silhouette equals the one a lag later, at
/// the smallest lag reaching the largest share. Silhouettes rather than kinds,
/// because in kinds a record `word , number , word ;` is an alternation of
/// content and punctuation and would read as two; the silhouette carries the
/// glyph. The smallest lag, because a record's multiples repeat as well as it
/// does. A resonator bank cannot make this reading: every divisor of the
/// record length is as coherent as the length itself.
///
/// The lag is kept only where the stream repeats at it more than chance would,
/// by [`PERIOD_LIFT_FLOOR`]. The share alone cannot order these corpora: prose
/// reaches 0.35 where code reaches 0.20, so an absolute floor rates prose the
/// more periodic of the two. Against its own chance rate prose reads 1.03 and
/// code 1.61.
#[must_use]
pub fn record_period(toks: &[Token], bytes: &[u8]) -> Option<u16> {
    gated_lag(&period_symbols(toks, bytes))
}

/// The strongest lag, kept only where the stream repeats at it more than
/// chance would. The reading every grain's period goes through.
fn gated_lag(symbols: &[u32]) -> Option<u16> {
    let (lag, share) = best_lag(symbols)?;
    let chance = chance_of(symbols);
    // No symbol repeats at all, so a share above zero is structure rather than
    // coincidence and there is no baseline to divide by.
    if chance <= 0.0 {
        return Some(lag);
    }
    (share / chance >= PERIOD_LIFT_FLOOR).then_some(lag)
}

/// The record period in supertokens, which is the grain a record is actually
/// found at.
///
/// A lag is counted in symbols and the search stops at [`PHASE_MAX_PERIOD`],
/// so the grain settles what can be found and not only what is read. A clippy
/// log runs to a median of 74 significant tokens a line against a ceiling of
/// 32, so no token-grain lag can be its record and [`record_period`] returns
/// the record's factors instead - 2, 4, 6 and 8, ranked by how short they are.
/// The same line is 1.64 supertokens.
///
/// The role alone does not carry it. Six roles put the chance rate at 0.609,
/// which is most of what a share can reach, and the strongest unit lag reads
/// 1.3 times chance - under the gate. Folding the unit's size in by its
/// magnitude widens the alphabet without keying on a length that varies run to
/// run, and separates them: on that log the chance rate falls to 0.099 and lag
/// 20 reads 3.7 times it, clear of the next reading at 1.9. Twenty units is
/// about twelve lines, which is a diagnostic block - the log's actual record.
///
/// Measured over the real corpora beside prose, which still refuses at 1.1,
/// and a table whose lines are too long to hold units at all, which refuses
/// too. So this reaches further rather than admitting more.
#[must_use]
pub fn unit_record_period(units: &[crate::supertoken::SuperToken]) -> Option<u16> {
    let symbols = unit_symbols(units);
    // Units that are all one symbol are period one, and no correlation can say
    // so: every lag reaches a share of 1.0 and the chance rate is 1.0 with
    // them, so the ratio is 1.0 and the gate refuses every lag. A uniform log,
    // where every line is the same shape at the same magnitude, is exactly
    // this - and its record is one line. The reading is exact rather than a
    // second floor: one distinct symbol IS a period of one.
    if symbols.len() > 1 && symbols.iter().all(|s| *s == symbols[0]) {
        return Some(1);
    }
    gated_lag(&symbols)
}

/// A supertoken's symbol: its role with its size folded in by magnitude.
///
/// The magnitude rather than the length, because a record repeats in shape and
/// not in exact width - two diagnostic blocks differ by a few characters and
/// must read as the same symbol, while a block and a bare continuation line
/// must not.
fn unit_symbols(units: &[crate::supertoken::SuperToken]) -> Vec<u32> {
    units
        .iter()
        .map(|u| {
            let len = u.end.saturating_sub(u.start).max(1);
            u.role.code() * 8 + len.ilog2().min(7)
        })
        .collect()
}

/// The token silhouettes the period readings are taken over.
fn period_symbols(toks: &[Token], bytes: &[u8]) -> Vec<u32> {
    toks.iter()
        .filter(|t| t.is_significant())
        .map(|t| crate::shape::shape_class(t.kind, &bytes[t.span()]))
        .collect()
}

/// The smallest lag reaching the largest share above
/// [`crate::spectral::PERIOD_FLOOR`].
///
/// The absolute floor and the lift gate refuse different things and both are
/// needed. The floor demands that the stream repeat at all; the lift demands
/// that it repeat more than its own alphabet repeats by coincidence. Dropping
/// the floor and keeping only the lift admits any short lag whose share is
/// above a low chance rate - code's lag of two reaches a share of 0.16 against a
/// chance rate of 0.10, which is 1.6 times chance and is still an alternation
/// of identifier and punctuation rather than a record.
fn best_lag(symbols: &[u32]) -> Option<(u16, f32)> {
    let n = symbols.len();
    let mut best: Option<(u16, f32)> = None;
    for lag in 2..=usize::from(PHASE_MAX_PERIOD) {
        if lag * 2 > n {
            break;
        }
        let matches = symbols.iter().zip(&symbols[lag..]).filter(|(a, b)| a == b).count();
        let share = matches as f32 / (n - lag) as f32;
        if share >= crate::spectral::PERIOD_FLOOR && best.is_none_or(|(_, b)| share > b) {
            best = Some((lag as u16, share));
        }
    }
    best
}

/// The chance two positions hold the same silhouette: the sum of the squared
/// silhouette frequencies.
fn chance_of(symbols: &[u32]) -> f32 {
    let n = symbols.len();
    if n == 0 {
        return 0.0;
    }
    let mut sorted = symbols.to_vec();
    sorted.sort_unstable();
    let mut chance = 0.0f64;
    let mut run = 0usize;
    for i in 0..n {
        run += 1;
        if i + 1 == n || sorted[i] != sorted[i + 1] {
            let p = run as f64 / n as f64;
            chance += p * p;
            run = 0;
        }
    }
    chance as f32
}

/// [`record_period`] with the share the lag reached, which is how strongly the
/// stream repeats at it.
///
/// The share is what decides the period and is then dropped, so a caller holds
/// a period and no way to tell a stream that repeats from one where some lag
/// merely beat the others. This is the raw winner, before
/// [`record_period`] gates it: the lag won its contest, which says nothing
/// about whether the contest was between lags that were all at chance.
///
/// Every consumer of the period wants this. `records::periods` cuts a record
/// every `period` significant tokens whatever the share, so a share that means
/// nothing still sets what `--count` answers, and `@phase:k` matches against a
/// period that may be noise.
#[must_use]
pub fn record_period_share(toks: &[Token], bytes: &[u8]) -> Option<(u16, f32)> {
    best_lag(&period_symbols(toks, bytes))
}

/// The share a lag reaches when the silhouettes carry no order at all: the
/// chance two positions a lag apart hold the same silhouette, which is the sum
/// of the squared silhouette frequencies.
///
/// This is the null [`record_period_share`] has never been read against. A
/// share is only evidence of repetition above what shuffling the same tokens
/// would give, and that baseline is a property of the corpus rather than a
/// constant: a stream of three distinct silhouettes has a chance collision
/// rate near a third, which clears a floor of 0.20 without any structure
/// whatever.
#[must_use]
pub fn record_period_chance(toks: &[Token], bytes: &[u8]) -> f32 {
    chance_of(&period_symbols(toks, bytes))
}

/// Every lag the reading considers, with the share it reaches, and the chance
/// rate they are all read against.
///
/// [`record_period`] returns one lag out of this profile - the smallest
/// reaching the largest share - and a caller cannot tell from it whether the
/// runner-up was a hair behind or half as good, nor whether a longer lag that
/// repeats just as well was passed over because a factor of it won. A log
/// whose line is its record repeats at the line and at every unit the line is
/// built from, so the profile is what distinguishes them and the single answer
/// is not.
///
/// Lags ascend from 2 to [`PHASE_MAX_PERIOD`], stopping where a lag has fewer
/// than two periods to compare.
#[must_use]
pub fn record_period_profile(toks: &[Token], bytes: &[u8]) -> (f32, Vec<(u16, f32)>) {
    period_profile_of(&period_symbols(toks, bytes))
}

/// Every lag the stream repeats at as [`record_period`] demands of the lag it
/// keeps - a share at the floor and [`PERIOD_LIFT_FLOOR`] times the chance
/// rate - strongest first by share, and on a tie the shorter, which is the
/// order [`record_period`] breaks ties in. The first is [`record_period`]'s.
#[must_use]
pub fn live_periods(toks: &[Token], bytes: &[u8]) -> Vec<u16> {
    live_lags(&period_symbols(toks, bytes))
}

fn live_lags(symbols: &[u32]) -> Vec<u16> {
    let (chance, profile) = period_profile_of(symbols);
    let mut live: Vec<(u16, f32)> = profile
        .into_iter()
        .filter(|&(_, share)| {
            share >= crate::spectral::PERIOD_FLOOR && (chance <= 0.0 || share / chance >= PERIOD_LIFT_FLOOR)
        })
        .collect();
    live.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    live.into_iter().map(|(lag, _)| lag).collect()
}

/// [`record_period_profile`] over symbols the caller chose, so the reading can
/// be taken at a grain other than the token's.
///
/// The grain decides what is reachable, not just what is read. A lag is
/// counted in symbols and the search stops at [`PHASE_MAX_PERIOD`], so a
/// record longer than that many symbols cannot be found at all - a log line
/// runs to a median of 74 significant tokens, so no token-grain lag can BE the
/// line and the reading returns the line's factors instead. The same line is a
/// unit or two of [`crate::supertoken`], which is inside the search. The
/// symbols must be dense, as they are for the resonator bank and for the same
/// reason.
#[must_use]
pub fn period_profile_of(symbols: &[u32]) -> (f32, Vec<(u16, f32)>) {
    let n = symbols.len();
    let mut out = Vec::new();
    for lag in 2..=usize::from(PHASE_MAX_PERIOD) {
        if lag * 2 > n {
            break;
        }
        let matches = symbols.iter().zip(&symbols[lag..]).filter(|(a, b)| a == b).count();
        out.push((lag as u16, matches as f32 / (n - lag) as f32));
    }
    (chance_of(symbols), out)
}

/// The rolling context of `bytes` with every field built: the axis fields the
/// windows read, the unit rung, and the three boundary sets the agreement
/// compares.
#[must_use]
pub fn analyze(bytes: &[u8]) -> ContextField {
    analyze_with(bytes, &ContextConfig::default())
}

/// [`analyze`] with the window extents given.
#[must_use]
pub fn analyze_with(bytes: &[u8], cfg: &ContextConfig) -> ContextField {
    let toks = crate::lexer::lex(bytes);
    let fields = Fields::build(&toks, bytes);
    let supers = SuperContext::build(&toks, bytes);
    let mut field = fold_windows_parallel(&toks, &fields.ctx(bytes), &supers, cfg);
    field.agreement = fields.agreement(&supers, &toks, bytes);
    field
}

/// Every axis field a fold reads, built over one lex.
struct Fields {
    spectral: crate::spectral::SpectralField,
    stress: crate::stress::StressField,
    echo: crate::echo::EchoField,
    observation: crate::observation::ObservationField,
    seam: crate::seam::SeamField,
    flow: crate::flow::FlowField,
}

impl Fields {
    fn build(toks: &[Token], bytes: &[u8]) -> Fields {
        Fields {
            spectral: crate::spectral::analyze(bytes),
            stress: crate::stress::analyze(toks, bytes),
            echo: crate::echo::analyze(toks, bytes),
            // The vantage and seam fields read ahead as well as behind, so a
            // fold without them carries only what the past said of a position.
            observation: crate::observation::analyze(bytes),
            seam: crate::seam::analyze(bytes),
            // Flow follows magnitude, the scale every fold carries, so the
            // dynamics beside a span's scale are that scale's.
            flow: crate::flow::analyze(toks, bytes, crate::flow::Signal::Magnitude),
        }
    }

    fn ctx<'a>(&'a self, bytes: &'a [u8]) -> AxisCtx<'a> {
        AxisCtx {
            spectral: Some(&self.spectral),
            stress: Some(&self.stress),
            echo: Some(&self.echo),
            observation: Some(&self.observation),
            seam: Some(&self.seam),
            flow: Some(&self.flow),
            ..AxisCtx::new(bytes)
        }
    }

    /// Where each lower grain's boundaries are relative to the units' starts.
    fn agreement(&self, supers: &SuperContext, toks: &[Token], bytes: &[u8]) -> Vec<Agreement> {
        let shape = crate::shape::analyze(toks, bytes);
        let seam_cuts = crate::seam::analyze_tokens(toks, &crate::seam::SeamConfig::default());
        agreement(&supers.units, toks, &self.spectral.boundaries, &shape.boundaries, &seam_cuts)
    }
}

/// The context `\N{>+1:F}` reads a token against, and the unit rung's two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    /// The window of significant tokens before the token: `\N{>+1}`.
    Window,
    /// The supertoken holding the token: the fold of its tokens.
    Unit,
    /// The window of supertokens ending at the one holding the token.
    Units,
    /// The heads of the brackets enclosing the token, outermost first:
    /// `\N{>+1:enclosing}`.
    Enclosing,
    /// Every earlier occurrence of the token's key: `\N{>+1:echo}`.
    Echo,
    /// The significant tokens since the last spectral change point:
    /// `\N{>+1:regime}`.
    Regime,
    /// Every earlier significant token at the token's column of the record
    /// period: `\N{>+1:phase}`.
    Phase,
    /// The values bound by `=` or `:` to earlier occurrences of the token's
    /// key: what `\N{>+1:k}` reads where `k` holds the token.
    Key,
}

impl Fold {
    /// Every fold's name, in the order of the variants.
    pub const NAMES: [&'static str; 8] = ["window", "unit", "units", "enclosing", "echo", "regime", "phase", "key"];

    /// The fold named `name`, as [`Fold::NAMES`] spells it.
    #[must_use]
    pub fn parse(name: &str) -> Option<Fold> {
        match name {
            "window" => Some(Fold::Window),
            "unit" => Some(Fold::Unit),
            "units" => Some(Fold::Units),
            "enclosing" => Some(Fold::Enclosing),
            "echo" => Some(Fold::Echo),
            "regime" => Some(Fold::Regime),
            "phase" => Some(Fold::Phase),
            "key" => Some(Fold::Key),
            _ => None,
        }
    }

    /// The fold's name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Fold::Window => "window",
            Fold::Unit => "unit",
            Fold::Units => "units",
            Fold::Enclosing => "enclosing",
            Fold::Echo => "echo",
            Fold::Regime => "regime",
            Fold::Phase => "phase",
            Fold::Key => "key",
        }
    }
}

/// Every context of one input over one lex: the rolling folds at the token
/// and unit rungs with the grains' agreement, the related folds with the
/// record period, and the live periods, every axis field the folds read
/// built. What [`Contexts::at`] returns is what a relative predicate compares
/// a token with.
pub struct Contexts {
    /// The rolling folds and the grains' agreement.
    pub field: ContextField,
    /// The related folds and the record period.
    pub related: RelationContext,
    /// Every lag the stream repeats at as the record period demands, strongest
    /// first; the first is the record period.
    pub live_periods: Vec<u16>,
    /// The supertokens and the one holding each token.
    pub supers: SuperContext,
    /// Each token's significance, for the window before it.
    significant: Vec<bool>,
}

impl Contexts {
    /// Every context of `bytes` lexed as `toks`, which may carry the
    /// declarations a scan reads.
    #[must_use]
    pub fn read(toks: &[Token], bytes: &[u8], cfg: &ContextConfig) -> Contexts {
        let fields = Fields::build(toks, bytes);
        let ctx = fields.ctx(bytes);
        let supers = SuperContext::build(toks, bytes);
        let mut field = fold_windows_parallel(toks, &ctx, &supers, cfg);
        field.agreement = fields.agreement(&supers, toks, bytes);
        let related = relate(toks, &ctx, &fields.spectral.boundaries, &fields.echo, record_period(toks, bytes));
        Contexts {
            field,
            related,
            live_periods: live_periods(toks, bytes),
            supers,
            significant: toks.iter().map(Token::is_significant).collect(),
        }
    }

    /// What `fold` holds at token `i`; the empty fold where it holds no token.
    #[must_use]
    pub fn at(&self, fold: Fold, i: usize) -> WindowProfile {
        match fold {
            // The window before the token is the window's fold at the
            // significant token before it, as `\N{>+1}` reads it.
            Fold::Window => match (0..i).rev().find(|&j| self.significant[j]) {
                Some(j) => self.field.at_token[j].clone(),
                None => WindowProfile::identity(),
            },
            Fold::Unit => match self.supers.index_of(i) {
                Some(u) => self.field.of_unit[u].clone(),
                None => WindowProfile::identity(),
            },
            Fold::Units => match self.supers.index_of(i) {
                Some(u) => self.field.at_unit[u].clone(),
                None => WindowProfile::identity(),
            },
            Fold::Enclosing => self.related.at_token[i].enclosing.clone(),
            Fold::Echo => self.related.at_token[i].echoing.clone(),
            Fold::Regime => self.related.at_token[i].regime.clone(),
            Fold::Phase => self.related.at_token[i].phase.clone(),
            Fold::Key => self.related.value_history[i].clone(),
        }
    }

    /// Token `i`'s column of the record period; `None` in a stream with no
    /// period, or at a token that is not significant.
    #[must_use]
    pub fn phase_of(&self, i: usize) -> Option<u16> {
        let p = self.related.phase_of[i];
        (p != u16::MAX).then_some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::fold_tokens;

    #[test]
    fn two_row_shapes_are_both_live_and_the_first_live_period_is_the_record_period() {
        let text = format!("{}{}", "k = 1 ;\n".repeat(150), "k = 1 , 2 ;\n".repeat(150));
        let toks = crate::lexer::lex(text.as_bytes());
        let live = live_periods(&toks, text.as_bytes());
        assert!(live.contains(&4) && live.contains(&6), "both row shapes repeat: {live:?}");
        assert_eq!(live.first().copied(), record_period(&toks, text.as_bytes()), "{live:?}");
        let prose = "the cat sat on a mat while it rained and then it stopped\n".repeat(3);
        let toks = crate::lexer::lex(prose.as_bytes());
        assert_eq!(live_periods(&toks, prose.as_bytes()).first().copied(), record_period(&toks, prose.as_bytes()));
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1.0)
    }

    fn same_reading(a: &WindowProfile, b: &WindowProfile) -> bool {
        close(a.magnitude.sum, b.magnitude.sum)
            && close(a.magnitude.sumsq, b.magnitude.sumsq)
            && a.magnitude.max == b.magnitude.max
            && a.magnitude.count == b.magnitude.count
            && a.stress == b.stress
            && close(a.spectral.entropy_sum, b.spectral.entropy_sum)
            && a.spectral.count == b.spectral.count
            && a.spectral.period == b.spectral.period
            && a.echo == b.echo
            && close(a.observation.anticausal_sum, b.observation.anticausal_sum)
            && a.observation.count == b.observation.count
            && close(a.seam.fwd_sum, b.seam.fwd_sum)
            && a.seam.cuts == b.seam.cuts
            && a.flow.reversals == b.flow.reversals
            && a.flow.count == b.flow.count
    }

    /// A field the rolling context builds but its bundle cannot read folds to
    /// zeros, and every other test here still passes. So the check is that the
    /// readings which look ahead arrive non-empty through the production path,
    /// not merely that the fold is self-consistent.
    #[test]
    fn the_rolling_context_reads_the_fields_it_builds() {
        let bytes = corpus(120);
        let field = analyze(&bytes);
        assert!(!field.at_token.is_empty(), "the corpus folds to something");

        let any = |f: fn(&WindowProfile) -> bool| field.at_token.iter().any(f);
        assert!(any(|w| w.observation.anticausal_sum > 0.0), "the future vantage arrived");
        assert!(any(|w| w.observation.centered_sum > 0.0), "the centered vantage arrived");
        assert!(any(|w| w.seam.fwd_sum > 0.0), "the boundary-after signal arrived");
        assert!(any(|w| w.seam.cuts > 0), "the stream is cut somewhere");
        assert!(any(|w| w.flow.count > 0), "the dynamics reading arrived");

        // And a rung up, which is the whole point of folding them.
        assert!(!field.at_unit.is_empty(), "the corpus has units");
        assert!(
            field.at_unit.iter().any(|u| u.observation.anticausal_sum > 0.0),
            "the future vantage reaches the unit rung, not only the token one"
        );
    }

    fn corpus(statements: usize) -> Vec<u8> {
        let mut s = String::new();
        for i in 0..statements {
            match i % 4 {
                0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
                1 => s.push_str(&format!("call_{i}(alpha, beta, {i}) ;\n")),
                2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
                _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
            }
        }
        s.into_bytes()
    }

    #[test]
    fn the_window_fold_is_the_direct_fold_at_every_position() {
        let bytes = corpus(60);
        let toks = crate::lexer::lex(&bytes);
        let spectral = crate::spectral::analyze(&bytes);
        let stress = crate::stress::analyze(&toks, &bytes);
        let echo = crate::echo::analyze(&toks, &bytes);
        let ctx =
            AxisCtx {
                spectral: Some(&spectral),
                stress: Some(&stress),
                echo: Some(&echo),
                ..AxisCtx::new(&bytes)
            };
        let supers = SuperContext::build(&toks, &bytes);
        for w in [1usize, 2, 3, 7, 32, 1000] {
            let cfg = ContextConfig { token_window: w, unit_window: 3 };
            let field = fold_windows(&toks, &ctx, &supers, &cfg);
            let sig: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].is_significant()).collect();
            for (k, &i) in sig.iter().enumerate() {
                let lo = k + 1 - w.min(k + 1);
                // The direct fold over the same significant tokens, in order.
                let direct = sig[lo..=k].iter().fold(WindowProfile::identity(), |acc, &j| {
                    acc.combine(&WindowProfile::of_token(j, &toks[j], &ctx))
                });
                assert!(
                    same_reading(&field.at_token[i], &direct),
                    "window {w} at token {i}: {:?} vs {:?}",
                    field.at_token[i],
                    direct
                );
            }
        }
    }

    #[test]
    fn a_units_reading_is_the_fold_of_its_tokens_and_the_unit_window_folds_them() {
        let bytes = corpus(40);
        let toks = crate::lexer::lex(&bytes);
        let ctx = AxisCtx::new(&bytes);
        let supers = SuperContext::build(&toks, &bytes);
        let cfg = ContextConfig { token_window: 8, unit_window: 3 };
        let field = fold_windows(&toks, &ctx, &supers, &cfg);
        assert_eq!(field.of_unit.len(), supers.units.len());
        for (u, unit) in supers.units.iter().enumerate() {
            let members: Vec<Token> = toks
                .iter()
                .enumerate()
                .filter(|(i, t)| t.is_significant() && supers.index_of(*i) == Some(u))
                .map(|(_, t)| *t)
                .collect();
            assert!(!members.is_empty(), "unit {u} {unit:?} holds tokens");
            let direct: WindowProfile = fold_tokens(0, &members, &ctx);
            // Magnitude reads the token's bytes only, so the index offset the
            // direct fold lacks changes nothing here.
            assert!(same_reading(&field.of_unit[u], &direct), "unit {u}");
            let lo = u + 1 - cfg.unit_window.min(u + 1);
            let windowed = crate::profile::fold_profiles(&field.of_unit[lo..=u]);
            assert!(same_reading(&field.at_unit[u], &windowed), "unit window at {u}");
        }
    }

    #[test]
    fn the_window_profile_is_a_monoid() {
        let bytes = b"alpha 12 (beta 3456) gamma alpha";
        let toks = crate::lexer::lex(bytes);
        let ctx = AxisCtx::new(bytes);
        let a = WindowProfile::of_token(0, &toks[0], &ctx);
        let b = WindowProfile::of_token(2, &toks[2], &ctx);
        let c = WindowProfile::of_token(4, &toks[4], &ctx);
        assert_eq!(WindowProfile::identity().combine(&a), a);
        assert_eq!(a.combine(&WindowProfile::identity()), a);
        assert!(same_reading(&a.combine(&b).combine(&c), &a.combine(&b.combine(&c))));
    }

    #[test]
    fn the_grains_agree_on_structure_and_not_on_a_shuffled_stream() {
        // Statements whose units start where the byte texture changes
        // (a keyword after a newline, a number after an operator). The same
        // tokens in a random order keep every grain's reader running but
        // leave nothing for their boundaries to agree on, so the shuffled
        // stream is the chance level the real one has to clear.
        let bytes = corpus(400);
        let real = analyze(&bytes);
        let toks = crate::lexer::lex(&bytes);
        let mut sig: Vec<&[u8]> = toks
            .iter()
            .filter(|t| t.is_significant())
            .map(|t| &bytes[t.span()])
            .collect();
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for i in (1..sig.len()).rev() {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            sig.swap(i, (x >> 33) as usize % (i + 1));
        }
        let shuffled: Vec<u8> = sig.join(&b' ');
        let null = analyze(&shuffled);
        let (_, _, r_seam) = real.alignment();
        let (_, _, n_seam) = null.alignment();
        assert!(real.agreement.len() > 100 && null.agreement.len() > 100, "enough units on both");
        // The seam is the one lower-grain reader dense enough to place a
        // boundary near every unit; the byte regime and shape change-points
        // are sparse on a uniform corpus, so their offsets spread on both.
        // On this corpus a call's nearest seam cut is one token before it,
        // a key's two before, a list's and a binding's one after: 0.875 of
        // units at their role's offset against 0.372 shuffled
        // (`benches/context_window`).
        assert!(
            r_seam > n_seam * 1.5 && r_seam > 0.6,
            "seam offset concentration by role: real {r_seam:.3} vs shuffled {n_seam:.3}"
        );
    }

    #[test]
    fn the_parallel_fold_reads_what_the_serial_fold_reads() {
        // Enough units and tokens for several chunks on any core count.
        let bytes = corpus(3000);
        let toks = crate::lexer::lex(&bytes);
        let spectral = crate::spectral::analyze(&bytes);
        let stress = crate::stress::analyze(&toks, &bytes);
        let echo = crate::echo::analyze(&toks, &bytes);
        let ctx =
            AxisCtx {
                spectral: Some(&spectral),
                stress: Some(&stress),
                echo: Some(&echo),
                ..AxisCtx::new(&bytes)
            };
        let supers = SuperContext::build(&toks, &bytes);
        let cfg = ContextConfig::default();
        let serial = fold_windows(&toks, &ctx, &supers, &cfg);
        let parallel = fold_windows_parallel(&toks, &ctx, &supers, &cfg);
        assert_eq!(parallel.at_token.len(), serial.at_token.len());
        assert_eq!(parallel.of_unit.len(), serial.of_unit.len());
        assert_eq!(parallel.at_unit.len(), serial.at_unit.len());
        for (i, (p, s)) in parallel.at_token.iter().zip(&serial.at_token).enumerate() {
            assert!(same_reading(p, s), "token {i}: {p:?} vs {s:?}");
        }
        for (u, (p, s)) in parallel.of_unit.iter().zip(&serial.of_unit).enumerate() {
            assert!(same_reading(p, s), "unit {u}");
        }
        for (u, (p, s)) in parallel.at_unit.iter().zip(&serial.at_unit).enumerate() {
            assert!(same_reading(p, s), "unit window {u}");
        }
    }

    #[test]
    fn each_relation_fold_is_the_direct_fold_over_what_the_relation_names() {
        let bytes = corpus(300);
        let toks = crate::lexer::lex(&bytes);
        let spectral = crate::spectral::analyze(&bytes);
        let stress = crate::stress::analyze(&toks, &bytes);
        let echo = crate::echo::analyze(&toks, &bytes);
        let ctx =
            AxisCtx {
                spectral: Some(&spectral),
                stress: Some(&stress),
                echo: Some(&echo),
                ..AxisCtx::new(&bytes)
            };
        // Four statement shapes cycle through more tokens than the period
        // search spans, so this stream has no record period; the phase fold
        // is checked on a periodic stream below.
        let period = record_period(&toks, &bytes);
        assert_eq!(period, None, "no record repeats within the search");
        let rel = relate(&toks, &ctx, &spectral.boundaries, &echo, period);
        let relation = crate::relation::analyze(&toks, &bytes);
        let reading = |i: usize| WindowProfile::of_token(i, &toks[i], &ctx);
        let fold = |ix: &[usize]| ix.iter().fold(WindowProfile::identity(), |a, &j| a.combine(&reading(j)));
        let sig: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].is_significant()).collect();
        let mut echoes_seen = 0usize;
        let mut enclosed_seen = 0usize;
        for (k, &i) in sig.iter().enumerate() {
            let r = &rel.at_token[i];
            // Enclosure: the heads of the enclosing brackets, outermost first.
            let heads = &relation.frames[i].enclosure;
            if !heads.is_empty() {
                enclosed_seen += 1;
            }
            assert!(same_reading(&r.enclosing, &fold(heads)), "enclosure at {i}");
            // Echo: every earlier token with the same text.
            let text = &bytes[toks[i].span()];
            let earlier: Vec<usize> = sig[..k]
                .iter()
                .copied()
                .filter(|&j| echo.frames[j].keyed && &bytes[toks[j].span()] == text)
                .collect();
            if echo.frames[i].keyed {
                if !earlier.is_empty() {
                    echoes_seen += 1;
                }
                assert!(same_reading(&r.echoing, &fold(&earlier)), "echo at {i}");
            }
            // Regime: the earlier significant tokens since the last change-point.
            let cut = spectral.boundaries.partition_point(|&c| c <= toks[i].start());
            let since = cut.checked_sub(1).map_or(0, |c| spectral.boundaries[c]);
            let members: Vec<usize> =
                sig[..k].iter().copied().filter(|&j| toks[j].start() >= since).collect();
            assert!(same_reading(&r.regime, &fold(&members)), "regime at {i}");
            assert_eq!(r.phase, WindowProfile::identity(), "no period, no phase fold at {i}");
            assert_eq!(rel.phase_of[i], u16::MAX, "no period, no phase at {i}");
        }
        assert!(echoes_seen > 100 && enclosed_seen > 100, "the relations were exercised");
    }

    #[test]
    fn the_phase_fold_is_the_direct_fold_over_the_column() {
        // Six significant tokens a row, no header.
        let mut rows = String::new();
        for i in 0..300 {
            rows.push_str(&format!("r{i} , {} , x{} ;\n", i * 3, i % 4));
        }
        let bytes = rows.as_bytes();
        let toks = crate::lexer::lex(bytes);
        let period = record_period(&toks, bytes);
        assert_eq!(period, Some(6), "the row is the period");
        let echo = crate::echo::analyze(&toks, bytes);
        let ctx = AxisCtx::new(bytes);
        let rel = relate(&toks, &ctx, &[], &echo, period);
        let reading = |i: usize| WindowProfile::of_token(i, &toks[i], &ctx);
        let sig: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].is_significant()).collect();
        for (k, &i) in sig.iter().enumerate() {
            let same_phase = (0..k).filter(|&m| m % 6 == k % 6).map(|m| sig[m]).fold(
                WindowProfile::identity(),
                |a, j| a.combine(&reading(j)),
            );
            assert!(same_reading(&rel.at_token[i].phase, &same_phase), "phase fold at {i}");
            assert_eq!(rel.phase_of[i], (k % 6) as u16, "phase at {i}");
        }
    }

    /// Lines binding values to a few recurring keys, so a key's value history
    /// has something in it.
    fn log_corpus(lines: usize) -> Vec<u8> {
        let mut s = String::new();
        for i in 0..lines {
            match i % 3 {
                0 => s.push_str(&format!("svc_{} latency = {} ms ;\n", i % 7, 90 + (i * 37) % 21)),
                1 => s.push_str(&format!("svc_{} size = {} ;\n", i % 7, 1_000_000 + i)),
                _ => s.push_str(&format!("state: {} ;\n", if i % 2 == 0 { "ready" } else { "busy" })),
            }
        }
        s.into_bytes()
    }

    #[test]
    fn the_value_history_is_the_fold_over_what_earlier_occurrences_bound() {
        let bytes = log_corpus(200);
        let toks = crate::lexer::lex(&bytes);
        let echo = crate::echo::analyze(&toks, &bytes);
        let ctx = AxisCtx::new(&bytes);
        let rel = relate(&toks, &ctx, &[], &echo, None);
        let reading = |i: usize| WindowProfile::of_token(i, &toks[i], &ctx);
        let next_sig = |i: usize| (i + 1..toks.len()).find(|&j| toks[j].is_significant());
        // What token `j` binds: the significant token after a `=` or `:`
        // that directly follows it.
        let bound_by = |j: usize| -> Option<usize> {
            let op = next_sig(j)?;
            let t = &toks[op];
            (t.kind == crate::token::TokenKind::Punct && matches!(&bytes[t.span()], b"=" | b":"))
                .then(|| next_sig(op))
                .flatten()
        };
        let mut histories_seen = 0usize;
        for i in 0..toks.len() {
            if !echo.frames[i].keyed {
                continue;
            }
            let text = &bytes[toks[i].span()];
            let values: Vec<usize> = (0..i)
                .filter(|&j| echo.frames[j].keyed && &bytes[toks[j].span()] == text)
                .filter_map(bound_by)
                .collect();
            if !values.is_empty() {
                histories_seen += 1;
            }
            let direct = values.iter().fold(WindowProfile::identity(), |a, &v| a.combine(&reading(v)));
            assert!(same_reading(&rel.value_history[i], &direct), "value history at {i}");
        }
        assert!(histories_seen > 100, "keys were bound more than once: {histories_seen}");
    }

    #[test]
    fn the_contexts_hold_the_folds_a_relative_predicate_reads() {
        let text = b"latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;";
        let toks = crate::lexer::lex(text);
        let c = Contexts::read(&toks, text, &ContextConfig::default());
        let sig: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].is_significant()).collect();
        // The window before the first token holds nothing, before the second
        // the first alone.
        assert_eq!(c.at(Fold::Window, sig[0]).magnitude.count, 0);
        assert_eq!(c.at(Fold::Window, sig[1]).magnitude.count, 1);
        // The last `latency` holds the three values its key was bound to
        // before: 100, 120 and 90, a mean of 2.01 orders.
        let last_key = sig.iter().rev().copied().find(|&i| &text[toks[i].span()] == b"latency").expect("a key");
        let history = c.at(Fold::Key, last_key);
        assert_eq!(history.magnitude.count, 3);
        assert!((history.magnitude.mean() - 2.01).abs() < 0.01, "{}", history.magnitude.mean());
        // Every axis is folded, not magnitude alone.
        assert_eq!(history.observation.count, 3);
        assert_eq!(history.flow.count, 3);
        for name in Fold::NAMES {
            assert_eq!(Fold::parse(name).map(Fold::name), Some(name));
        }
        assert_eq!(Fold::parse("windows"), None);
    }

    #[test]
    fn a_window_of_one_reads_the_token_alone() {
        let bytes = b"alpha 12 beta 3456";
        let toks = crate::lexer::lex(bytes);
        let ctx = AxisCtx::new(bytes);
        let supers = SuperContext::build(&toks, bytes);
        let field = fold_windows(&toks, &ctx, &supers, &ContextConfig { token_window: 1, unit_window: 1 });
        for (i, t) in toks.iter().enumerate() {
            if t.is_significant() {
                assert_eq!(field.at_token[i].magnitude.count, 1, "token {i}");
            }
        }
        // The whitespace after a token carries that token's reading.
        assert_eq!(field.at_token[1], field.at_token[0]);
    }
}
