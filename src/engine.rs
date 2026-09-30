//! The matching engine.
//!
//! The engine folds the pattern over the token stream as a set of
//! reachable states rather than a backtracking search. Each state
//! is a token position plus a register environment, deduplicated
//! through a HashSet. Because it explores positions as a set
//! instead of trying paths one at a time, it never backtracks: an
//! input that drives a backtracking matcher to exponential time
//! runs in polynomial time here, with no catastrophic-backtracking
//! cliff.
//!
//! This set-reachability form recomputes a nested quantifier's
//! closure once per starting position, so its worst case is
//! polynomial rather than linear. The single-pass NFA simulation in
//! [`crate::nfa`] visits each state once per position for a
//! worst-case-linear bound, and [`scan`] tries it first; this engine
//! handles what that one declines - balanced groups, field anchors,
//! and the axis predicates - and is the oracle the single-pass engine
//! is differential-tested against.
//!
//! The active set is the operational form of the partial-derivative
//! set of the pattern with respect to the consumed prefix: each
//! reachable position is one residual. Registers ride alongside,
//! making the machine a register-set automaton, which is what gives
//! `:name` / `=name` long-distance binding without the exponential
//! blowup of a backtracking backreference.
//!
//! The scan over a large token stream is data-parallel: the match
//! attempt anchored at each position is an independent, pure call, so
//! the attempts fan out across the available cores and only the cheap
//! leftmost, non-overlapping selection runs on one thread. Small
//! streams stay inline, where a thread spawn would cost more than the
//! scan.

use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

use crate::ast::{Atom, ByteClass, EmptyLoop, Greed, Pattern};
use crate::lexer::text;
use crate::spectral::SpectralField;
use crate::token::{Token, TokenKind};

/// The precomputed axis fields threaded read-only through the set engine.
/// Each is built once per scan, and only when the pattern queries that axis
/// (`\F{...}` builds the spectral field, `@seam` the seam field); a pattern
/// that queries no axis carries all-`None` and pays nothing. `Copy`, so
/// threading it through the recursion costs no more than the single option it
/// replaced, and a new axis is one more field here rather than a new param at
/// every call site.
#[derive(Clone, Copy, Default)]
struct Fields<'a> {
    /// The pooled spectral signature field (`\F{...}`).
    spectral: Option<&'a SpectralField>,
    /// The high-tier seam-cut byte offsets, ascending (`@seam`), precomputed
    /// once via [`crate::seam::SeamField::strong_cuts`] so the anchor is an O(log n) lookup.
    seam_cuts: Option<&'a [usize]>,
    /// The nesting depth of every token (`@nested>k`), in the order of the slice
    /// the engine walks, so the anchor reads a token's depth by its own index.
    depths: Option<&'a [u16]>,
    /// The contested (vantage-dependent) byte offsets, ascending (`@ambiguous`),
    /// taken from the observation field so the anchor is an O(log n) lookup.
    obs_contested: Option<&'a [usize]>,
    /// The recurrence fields (`@novel` / `@echoed` / `@echo...`), one per
    /// orbit rung the pattern counts at. Frames are index-aligned with the
    /// token stream, so an anchor is a rung lookup and an O(1) index.
    echo: &'a [(crate::orbit::OrbitGroup, crate::echo::EchoField)],
    /// The upper-grain window (`@super`): which construct each token sits in.
    supers: Option<&'a crate::supertoken::SuperContext>,
    /// Which way each timestamp stands against the timestamp before it
    /// (`@order`): `Some(true)` at or after, `Some(false)` before, `None`
    /// where the token is no timestamp or is the input's first.
    order: Option<&'a [Option<bool>]>,
    /// The input's line templates (`@shape:rare`): which template each
    /// token's line has, and how many lines each template covers.
    templates: Option<&'a crate::templates::Mining>,
    /// The second inputs the join anchors read (`@echoed:@other`), each
    /// keyed once at its rung against every token of this input.
    joins: &'a [Join],
    /// Seam cuts read over the token stream (`@seam:token`), as byte offsets.
    seam_token: Option<&'a [usize]>,
    /// Seam cuts read over the supertoken stream (`@seam:super`).
    seam_super: Option<&'a [usize]>,
    /// Contested points over the token stream (`@ambiguous:token`).
    obs_token: Option<&'a [usize]>,
    /// Contested points over the supertoken stream (`@ambiguous:super`).
    obs_super: Option<&'a [usize]>,
    /// The pair field's readings at each grain an anchor names (`@strain`,
    /// `@bound`, `@kin`), in the slot [`grain_slot`] gives the grain.
    gravity: [Option<&'a crate::gravity::Readings>; 3],
    /// Each `@kin` anchor's grain and example, with the type the example names
    /// in this input, `None` where the input holds no such type.
    kin: &'a [(crate::ast::Grain, String, Option<u32>)],
    /// The stream's live periods and each token's index among the significant
    /// tokens (`@phase:k/p`, `@phase:k#n`).
    bands: Option<&'a Bands>,
    /// Per token index, the 1-based comma-delimited field it starts, or `0`
    /// when it does not start one (`@k`). Index-aligned with the token stream,
    /// so the anchor is an O(1) lookup.
    field_starts: Option<&'a [u32]>,
    /// The rolling window's fold at each token (`\N{>+1}`); index-aligned.
    context: Option<&'a crate::context::ContextField>,
    /// The relation-admitted folds and the phase at each token
    /// (`\N{>+1:phase}`, `\N{>+1:k}`, `@phase:2`); index-aligned.
    relation: Option<&'a crate::context::RelationContext>,
    /// The clock a typed predicate's timestamp clause reads (`\T{age<24h}`),
    /// one reading per scan.
    clock: crate::typed::Clock,
    /// The ids of the registers bound under a repetition, whose every
    /// binding a state keeps rather than the last.
    lists: &'a [u16],
}

/// The context fields a pattern reads, built once per scan like the axis
/// fields and only when the pattern asks for them.
struct ContextBuild {
    window: Option<crate::context::ContextField>,
    relation: Option<crate::context::RelationContext>,
}

/// Build the context fields `pattern` reads over `toks`. The spectral and
/// echo fields already built for other atoms are reused; missing ones are
/// built here.
fn build_context(
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    spectral: Option<&SpectralField>,
    echo: Option<&crate::echo::EchoField>,
) -> ContextBuild {
    use crate::context::{ContextConfig, fold_windows_parallel, record_period, relate};
    use crate::profile::AxisCtx;
    let uses = pattern.context_uses();
    // The predicates read magnitude, which is a function of the token's
    // bytes, so the readings need no axis field behind them.
    let ctx = AxisCtx::new(input);
    let window = uses.window.then(|| {
        let supers = crate::supertoken::SuperContext::build(toks, input);
        fold_windows_parallel(toks, &ctx, &supers, &ContextConfig::default())
    });
    // Each input of the related context is built only for the scope that
    // reads it: the regime cuts come from a spectral pass, the echo and key
    // folds from the recurrence field, the phase from the record period.
    let relation = uses.any_related().then(|| {
        let own_spectral;
        let cuts: &[usize] = if uses.regime {
            match spectral {
                Some(s) => &s.boundaries,
                None => {
                    // Only the change-point boundaries are read here, so the
                    // field is built for those alone: the period scan and the
                    // k-gram novelty would each be work at every byte of the
                    // input for a reading nothing takes.
                    let mut needs = crate::spectral::Needs::none();
                    needs.onset = true;
                    needs.entropy = true;
                    needs.bands = true;
                    own_spectral =
                        crate::spectral::analyze_needing(input, &Default::default(), needs);
                    &own_spectral.boundaries
                }
            }
        } else {
            &[]
        };
        let own_echo;
        let echo = if uses.echo || uses.key {
            match echo {
                Some(e) => e,
                None => {
                    own_echo = crate::echo::analyze(toks, input);
                    &own_echo
                }
            }
        } else {
            own_echo =
                crate::echo::EchoField { frames: Vec::new(), keyed: 0, distinct: 0, novel: 0, echoed: 0 };
            &own_echo
        };
        let period = if uses.phase { record_period(toks, input) } else { None };
        relate(toks, &ctx, cuts, echo, period)
    });
    ContextBuild { window, relation }
}

/// For each token index, the 1-based comma-delimited field that token begins,
/// or `0` when it begins none. A token begins field `k` when exactly `k - 1`
/// significant commas precede it and it sits at the input start or directly
/// after a comma.
pub(crate) fn field_start_index(input: &[u8], toks: &[Token]) -> Vec<u32> {
    let mut out = vec![0u32; toks.len()];
    let mut commas = 0u32;
    let mut any_significant = false;
    let mut prev_was_comma = false;
    for (p, t) in toks.iter().enumerate() {
        if t.kind == TokenKind::Whitespace {
            // Insignificant: it neither opens a field nor closes one.
            if !any_significant || prev_was_comma {
                out[p] = commas + 1;
            }
            continue;
        }
        if !any_significant || prev_was_comma {
            out[p] = commas + 1;
        }
        any_significant = true;
        if t.kind == TokenKind::Punct && text(input, t) == b"," {
            commas += 1;
            prev_was_comma = true;
        } else {
            prev_was_comma = false;
        }
    }
    out
}

/// A register environment: register *id* to the bound value's byte span
/// `(start, end)` in the input, resolved to text only when a register-
/// equality atom compares it or a completed match reports it.
///
/// Three layers keep this off the per-state hot path, each removing a cost
/// the measurement found scaled per register:
///
/// - **Interned key.** Register names are interned to a dense `u16` id once
///   per scan (see [`collect_registers`]); the map is keyed by id, not by
///   `String`. The active-set dedup hashes the whole state per reachable
///   position, so a `String` key meant hashing register names on the hot
///   path -- a cost that grew linearly with register count (+72% for one
///   register, +120% for two). A `u16` key hashes in constant time and
///   keeps the hash well-distributed, so the dedup never degrades.
/// - **Span value.** The value is a byte span, not a `String`, so a bind
///   stores two integers instead of allocating and copying the matched
///   text; the text is resolved lazily at a register-equality check or at
///   final capture output.
/// - **Copy-on-write map.** The map is wrapped in `Rc`: a reachable state
///   is *carried* far more often than its registers are *bound* (every
///   atom match clones the state to advance its position, but a `:name`
///   bind happens only at the few positions that capture), so sharing the
///   map through `Rc` makes each carry-clone a refcount bump, and only a
///   bind pays a real clone, via `Rc::make_mut`.
///
/// `Rc<T>` derives `Hash`/`Eq`/`Ord` from the pointee, so the dedup
/// compares register contents, not pointers, and stays correct. Measured:
/// carrying one register entry cost +112% with a cloned
/// `BTreeMap<String, String>`; the three layers cut that to single digits.
type Env = Rc<BTreeMap<u16, (usize, usize)>>;

/// The distinct register names referenced by `pat`, in first-encounter
/// order. A register's id is its index here; the set engine keys its
/// environment by that id rather than by name, so names are compared once,
/// at scan setup, instead of on every dedup of a reachable state.
fn collect_registers(pat: &Pattern) -> Vec<String> {
    fn walk(pat: &Pattern, out: &mut Vec<String>) {
        let push = |name: &str, out: &mut Vec<String>| {
            if !out.iter().any(|r| r == name) {
                out.push(name.to_string());
            }
        };
        match pat {
            Pattern::Empty | Pattern::Guard(..) | Pattern::Anchor(_) => {}
            Pattern::Assert(p, _, _) => walk(p, out),
            Pattern::Atom(
                Atom::RegisterEq(name, _)
                | Atom::RegisterRelated(name, _)
                | Atom::RegisterWithin(name, _, _)
                | Atom::RegisterKin(name, _),
            ) => {
                push(name, out);
            }
            // A relative magnitude predicate keyed on a register reads that
            // register, so it is referenced like an equality would be.
            Pattern::Atom(a) => {
                if let Some(name) = a.key_scope() {
                    push(name, out);
                }
            }
            Pattern::Concat(ps) | Pattern::Alt(ps, _) => ps.iter().for_each(|p| walk(p, out)),
            Pattern::Opt(p, _)
            | Pattern::Star(p, _)
            | Pattern::Plus(p, _)
            | Pattern::Repeat(p, _, _, _)
            | Pattern::Atomic(p)
            | Pattern::Balanced(_, p)
            | Pattern::Field(_, p) => walk(p, out),
            Pattern::Bind(name, _, p) => {
                push(name, out);
                walk(p, out);
            }
            Pattern::Within(v, _) => {
                for e in v {
                    if let Some((name, _)) = &e.bind {
                        push(name, out);
                    }
                    walk(&Pattern::Atom(e.atom.clone()), out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(pat, &mut out);
    out
}

/// The interned id of register `name`, or `None` when the pattern never
/// references it (a possibility only for a malformed register-equality with
/// no matching bind, which then simply never matches).
fn reg_id(regs: &[String], name: &str) -> Option<u16> {
    regs.iter().position(|r| r == name).map(|i| i as u16)
}

/// The branch taken at each enclosing [`crate::ast::AltMode::First`],
/// outermost first.
///
/// Compared lexicographically: the smallest path is the preferred derivation,
/// which is what makes leftmost-first expressible in an engine that explores a
/// set, where there is otherwise no preference order. `None` is the empty path
/// and beats every non-empty one, so a pattern with no such alternation pays
/// nothing. Shared through `Rc` for the same reason [`Env`] is: a state is
/// carried far more often than a branch is taken.
///
/// Held in the state where it is short enough, and on the heap past that.
///
/// Measured over the crate's own source and over prose, every extension a scan
/// makes produces a path of exactly one byte and none exceeds four, so the heap
/// form was taking a block with a sixteen byte refcount header to carry a
/// single byte three to four million times a scan. The inline form carries
/// seven bytes and a length, which is what fits beside it in one word.
///
/// The heap arm is what keeps this a choice about speed rather than a bound on
/// the language: a pattern whose first-match alternations nest deeply enough
/// still gets a path as long as it needs, and neither corpus measured has one.
///
/// It costs width. `Option<Rc<[u8]>>` was sixteen bytes because the null
/// pointer niche carried the `None`; two variants sharing no niche need a
/// discriminant, so this is twenty-four and a `State` is that much wider. The
/// sweep carries millions of states in vectors a fan-out grows, so that is paid
/// against the allocations removed rather than on top of them.
#[derive(Clone, Debug)]
enum Rank {
    /// A path that fits beside its length in a word.
    Inline { len: u8, bytes: [u8; 7] },
    /// A path past the inline room.
    Heap(Rc<[u8]>),
}

/// How many bytes a path holds before it goes to the heap.
const RANK_INLINE: usize = 7;

impl Default for Rank {
    fn default() -> Self {
        Rank::Inline { len: 0, bytes: [0; RANK_INLINE] }
    }
}

impl Rank {
    /// The path as bytes, whichever form holds it.
    fn as_slice(&self) -> &[u8] {
        match self {
            Rank::Inline { len, bytes } => &bytes[..*len as usize],
            Rank::Heap(path) => path,
        }
    }

    /// This path with `branch` appended.
    fn extended(&self, branch: u8) -> Rank {
        let path = self.as_slice();
        let n = path.len();
        if n < RANK_INLINE {
            let mut bytes = [0u8; RANK_INLINE];
            bytes[..n].copy_from_slice(path);
            bytes[n] = branch;
            return Rank::Inline { len: (n + 1) as u8, bytes };
        }
        // One allocation, and the iterator's shape is what makes it one: a
        // slice collects in a single sized allocation only where the length can
        // be trusted, which a map over a range reports and a chain does not.
        Rank::Heap((0..n + 1).map(|i| if i < n { path[i] } else { branch }).collect())
    }

    /// The path these bytes spell, inline where they fit.
    fn of(path: &[u8]) -> Rank {
        if path.len() <= RANK_INLINE {
            let mut bytes = [0u8; RANK_INLINE];
            bytes[..path.len()].copy_from_slice(path);
            return Rank::Inline { len: path.len() as u8, bytes };
        }
        Rank::Heap(Rc::from(path))
    }
}

/// One binding of a register bound under a repetition, and the bindings
/// before it: a list shared by every state that forked after the binding, so
/// a fork copies one pointer and a binding adds one node.
#[derive(Debug)]
struct HistNode {
    id: u16,
    span: (usize, usize),
    prev: Hist,
}

/// The bindings a state's registers under a repetition have made, newest
/// first; nothing for a pattern that binds none under one.
type Hist = Option<Rc<HistNode>>;

/// One reachable matcher state.
///
/// `Eq` and `Hash` cover `pos` and `env` only, not `rank` or `hist`. Two
/// derivations reaching the same position with the same registers have
/// identical futures, so they are one state and the dedup collapses them; the
/// branch path that got there decides only which of the two is preferred, and
/// the bindings it made under a repetition ride with the one kept. Keeping
/// rank out of the identity is what stops a preferred and a non-preferred
/// derivation coexisting and doubling the set at every alternation.
///
/// Deduplicating with a linear scan would make each star fixpoint quadratic
/// and the whole scan cubic, which is the difference between a matcher that
/// shrugs off adversarial input and one that stalls on it.
#[derive(Clone, Debug)]
struct State {
    /// Token index reached.
    pos: usize,
    /// Registers bound so far.
    env: Env,
    /// Preference path; see [`Rank`].
    rank: Rank,
    /// Every binding made so far under a repetition; see [`Hist`].
    hist: Hist,
}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.pos == other.pos && self.env == other.env
    }
}

impl Eq for State {}

impl std::hash::Hash for State {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.pos.hash(h);
        self.env.hash(h);
    }
}

/// The empty environment, shared rather than built.
///
/// [`State::start`] runs once at every anchor the sweep offers, and building a
/// map there allocated an empty `BTreeMap` behind an `Rc` for every significant
/// token in the input whether the pattern bound a register or not. Sampling the
/// callers of a scan's allocations put `State::start` among the largest.
///
/// Sharing one is safe because the map is already copy-on-write: every write
/// goes through `Rc::make_mut`, which clones when the count is above one, so a
/// state that binds a register gets a map of its own and the shared empty one
/// is never written through.
///
/// Per thread, because an `Rc` is not shared across threads - which is also
/// where the parallel sweep wants it, each worker bumping a count no other
/// worker's cache line holds.
fn empty_env() -> Env {
    thread_local! {
        static EMPTY: Env = Rc::new(BTreeMap::new());
    }
    EMPTY.with(Clone::clone)
}

impl State {
    /// A state at `pos` with no registers and no branch choices.
    fn start(pos: usize) -> Self {
        State { pos, env: empty_env(), rank: Rank::default(), hist: None }
    }

    /// The bindings under a repetition, oldest first, as register id and
    /// byte span.
    fn history(&self) -> Vec<(u16, (usize, usize))> {
        let mut out = Vec::new();
        let mut cur = self.hist.as_ref();
        while let Some(node) = cur {
            out.push((node.id, node.span));
            cur = node.prev.as_ref();
        }
        out.reverse();
        out
    }

    /// The preference path as a slice.
    fn rank_slice(&self) -> &[u8] {
        self.rank.as_slice()
    }

    /// This state's path extended by taking branch `branch`.
    ///
    /// This runs for every state on every branch of a first-match alternation
    /// and for every state on every iteration of a quantifier's fixpoint, which
    /// is the hottest place in this engine that allocates at all: sampling the
    /// callers of a scan's allocations put this function in two of the three
    /// largest rows, once for the `Rc` and once for the buffer beneath it.
    ///
    /// One allocation, and the shape of the iterator is what makes it one.
    /// `Rc<[u8]>` collects in a single sized allocation only where the iterator
    /// reports an exact length it can be trusted on; a chain of the path and
    /// one more byte does not, and would collect into a vector first and copy
    /// again. A map over a range does, so the extension is written as an index
    /// walk that yields the path and then the branch.
    fn with_branch(&self, branch: u8) -> Self {
        // The length the extension produces, which is what the inline room has
        // to hold before this reaches the heap at all.
        note_extended(self.rank_slice().len() + 1);
        State {
            pos: self.pos,
            env: self.env.clone(),
            rank: self.rank.extended(branch),
            hist: self.hist.clone(),
        }
    }
}

/// Order two preference paths, lower being preferred.
///
/// Every choice a derivation makes writes an entry, and each entry is already
/// oriented so that the preferred option sorts lower: an earlier alternation
/// branch, another iteration of a greedy quantifier, an earlier stop for a
/// lazy one. Two derivations therefore agree entry by entry until the first
/// point where they chose differently, and that entry decides, which is a
/// plain lexicographic compare.
fn cmp_rank(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    note_compared();
    a.cmp(b)
}

/// Where the preferred derivation sits in `states`, by the rank order, or
/// `None` for an empty set.
///
/// The index rather than the state, so a caller holding the vector can keep
/// the one it names and drop the rest in place. Ties go to the earliest, which
/// is the order `min_by` takes over the states themselves.
fn best_index(states: &[State]) -> Option<usize> {
    states
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| cmp_rank(a.rank_slice(), b.rank_slice()))
        .map(|(at, _)| at)
}

/// The single entry a choiceless quantifier writes for having run `iters`
/// times, oriented so the preferred count sorts lower.
fn count_key(g: Greed, iters: usize) -> u8 {
    let k = u8::try_from(iters.min(u8::MAX as usize)).expect("clamped to a u8");
    match g {
        // More iterations is preferred, so a higher count must sort lower.
        Greed::Greedy => u8::MAX - k,
        Greed::Lazy => k,
    }
}

/// How many register spans a match holds without allocating.
///
/// Two spans are sixteen bytes, which is what the fat pointer of the shared
/// form already costs, so carrying them inline widens nothing: [`Regs`] is the
/// size of the `Vec` it stands in for, and a match that binds nothing pays the
/// same as it did.
pub const INLINE_REGS: usize = 2;

/// The register spans of one match.
///
/// A match's register count is a property of its pattern, not of the match, so
/// every match of one pattern carries the same number. What changes with that
/// number is which way of holding it is cheapest, and the two costs are both
/// measured here: a heap allocation and its free is 60.5 nanoseconds a match,
/// and a reference count's clone and drop is 10.
///
/// So neither form wins everywhere. Up to [`INLINE_REGS`] the spans ride in the
/// match and cost neither. Beyond it they are shared by reference count, which
/// is six times cheaper than allocating per match and lets a match cross a
/// thread without copying them.
#[derive(Clone)]
pub enum Regs {
    /// No registers, which is what every match of a pattern that binds none
    /// carries.
    ///
    /// A variant of its own rather than an empty `Inline`, because this is the
    /// commonest match in the crate and it is built by a path that does nothing
    /// else: `captures` over a non-binding pattern is `Match::from` per span and
    /// no work besides. Filling a zeroed inline array there measured 6.8
    /// nanoseconds a match against writing a discriminant, which over eight rows
    /// of the comparison was 3.41 ms - more than holding the registers inline
    /// won back on the two rows that bind.
    None,
    /// The spans a match holds itself, and how many of them are live.
    Inline(u8, [Span; INLINE_REGS]),
    /// More spans than ride inline, counted rather than copied.
    Shared(std::sync::Arc<[Span]>),
}

impl Regs {
    /// No registers.
    #[must_use]
    #[inline]
    pub const fn none() -> Self {
        Regs::None
    }

    /// The spans of `from`, inline where they fit and shared where they do not.
    #[must_use]
    #[inline]
    pub fn from_slice(from: &[Span]) -> Self {
        if from.is_empty() {
            return Regs::None;
        }
        if from.len() <= INLINE_REGS {
            let mut held = [Span { start: 0, end: 0 }; INLINE_REGS];
            held[..from.len()].copy_from_slice(from);
            let n = u8::try_from(from.len()).expect("at most INLINE_REGS");
            Regs::Inline(n, held)
        } else {
            Regs::Shared(from.into())
        }
    }

    /// The spans, whichever way they are held.
    #[must_use]
    #[inline]
    pub fn as_slice(&self) -> &[Span] {
        match self {
            Regs::None => &[],
            Regs::Inline(n, spans) => &spans[..*n as usize],
            Regs::Shared(spans) => spans,
        }
    }

    /// The spans to write through, for a caller rebasing them onto another
    /// region of the input.
    ///
    /// Spans another match still holds are copied before the first such write,
    /// so rebasing one match never rewrites another's registers.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [Span] {
        match self {
            Regs::None => &mut [],
            Regs::Inline(n, spans) => &mut spans[..*n as usize],
            Regs::Shared(spans) => {
                if std::sync::Arc::get_mut(spans).is_none() {
                    *spans = spans.to_vec().into();
                }
                std::sync::Arc::get_mut(spans).expect("no other holder remains")
            }
        }
    }
}

impl std::ops::Deref for Regs {
    type Target = [Span];
    #[inline]
    fn deref(&self) -> &[Span] {
        self.as_slice()
    }
}

impl From<Vec<Span>> for Regs {
    #[inline]
    fn from(v: Vec<Span>) -> Self {
        Regs::from_slice(&v)
    }
}

// Filled straight into the form the match will hold, so a caller that has an
// iterator and not a slice also allocates nothing for the counts that ride
// inline. The first spans past the inline width are what says a vector is
// needed, and it is built only then.
impl FromIterator<Span> for Regs {
    fn from_iter<I: IntoIterator<Item = Span>>(iter: I) -> Self {
        let mut it = iter.into_iter();
        let mut held = [Span { start: 0, end: 0 }; INLINE_REGS];
        let mut n = 0usize;
        for slot in &mut held {
            let Some(span) = it.next() else { break };
            *slot = span;
            n += 1;
        }
        match it.next() {
            None if n == 0 => Regs::None,
            None => Regs::Inline(u8::try_from(n).expect("at most INLINE_REGS"), held),
            Some(over) => {
                let mut all: Vec<Span> = Vec::with_capacity(n + 2);
                all.extend_from_slice(&held[..n]);
                all.push(over);
                all.extend(it);
                Regs::Shared(all.into())
            }
        }
    }
}

// By the spans and not by how they are held: the same registers held inline and
// held shared are the same registers, and a caller comparing two matches is
// asking about the registers.
impl PartialEq for Regs {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Regs {}

impl std::fmt::Debug for Regs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}

/// A reported match: a half-open byte span plus the registers bound
/// when the match completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// Inclusive start byte offset of the match in the input.
    pub start: usize,
    /// Exclusive end byte offset of the match in the input.
    pub end: usize,
    /// Captured registers as spans, in the order [`Self::names`] holds their
    /// names, which is sorted by name.
    ///
    /// Spans rather than text, because the engine binds positions and the
    /// text is a slice of the input the caller already holds. A register
    /// that bound nothing carries an empty span.
    ///
    /// Positions rather than pairs, because the names belong to the pattern and
    /// not to any one of its matches: carrying an owned name in each cost 58.4
    /// nanoseconds a match, half of what resolving a match cost at all.
    ///
    /// Read through [`Self::captures`], as the names are read through
    /// [`Self::names`], so that how the spans are held stays this type's to
    /// choose. Holding them in a `Vec` cost a heap allocation and its free per
    /// match, 60.5 nanoseconds, which on a pattern binding one register was
    /// more than the whole rest of resolving it.
    pub(crate) captures: Regs,
    /// The register names, in the order `captures` holds their spans, and
    /// nothing where the pattern names none.
    ///
    /// Shared with every other match of the same pattern, so a match carries a
    /// reference count rather than a copy of them - and a match of a pattern
    /// that names no register carries neither. A shared empty list still costs
    /// the two atomics of a clone and a drop a match, which on a scan reporting
    /// fifty thousand is half a millisecond to hand back nothing.
    names: Option<std::sync::Arc<[String]>>,
    /// Every binding of each register bound under a repetition, oldest first,
    /// keyed by the register's index in `names`; nothing for a pattern that
    /// binds none under one, which is one word a match.
    lists: Option<Box<Bindings>>,
}

/// The bindings the registers under a repetition made in one match: each
/// register's index in the match's names with its spans, oldest first.
type Bindings = Vec<(usize, Vec<Span>)>;

impl Match {
    /// A match with no registers bound.
    ///
    /// Inlined because a cursor builds one a match: it is five field writes,
    /// and a call to it across the module boundary costs more than the writes.
    #[must_use]
    #[inline]
    pub fn plain(start: usize, end: usize) -> Self {
        Match { start, end, captures: Regs::none(), names: None, lists: None }
    }

    /// This match carrying `lists`: every binding of each register bound
    /// under a repetition, keyed by its index in [`Self::names`].
    #[must_use]
    pub(crate) fn with_lists(mut self, lists: Bindings) -> Self {
        self.lists = (!lists.is_empty()).then(|| Box::new(lists));
        self
    }

    /// Every binding the register called `name` made in this match, oldest
    /// first, where the register is bound under a repetition; `None` for a
    /// register that is not, whose one binding [`Self::group_span`] holds.
    #[must_use]
    pub fn list(&self, name: &str) -> Option<&[Span]> {
        let i = self.names().iter().position(|n| n == name)?;
        self.lists.as_ref()?.iter().find(|(k, _)| *k == i).map(|(_, spans)| spans.as_slice())
    }

    /// Every register bound under a repetition, with its bindings.
    pub fn lists(&self) -> impl Iterator<Item = (&str, &[Span])> {
        self.lists
            .as_deref()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|(k, spans)| (self.names()[*k].as_str(), spans.as_slice()))
    }

    /// A match with `captures`, named by `names` in the same order.
    ///
    /// An empty `names` is held as none, so a match of a pattern naming no
    /// register is the same match however it was built.
    #[must_use]
    #[inline]
    pub fn bound(
        start: usize,
        end: usize,
        captures: Regs,
        names: std::sync::Arc<[String]>,
    ) -> Self {
        Match { start, end, captures, names: (!names.is_empty()).then_some(names), lists: None }
    }

    /// The register names, in the order [`Self::captures`] holds their spans.
    #[must_use]
    pub fn names(&self) -> &[String] {
        self.names.as_deref().unwrap_or(&[])
    }

    /// The spans this match's registers bound, in the order [`Self::names`]
    /// holds their names.
    #[must_use]
    #[inline]
    pub fn captures(&self) -> &[Span] {
        self.captures.as_slice()
    }

    /// The register spans to write through, for a caller rebasing this match
    /// onto another region of the input.
    #[inline]
    pub fn captures_mut(&mut self) -> &mut [Span] {
        self.captures.as_mut_slice()
    }

    /// This match found over a window of a longer input, as a match of that
    /// input: its span, its registers and every binding made under a
    /// repetition moved by `by`, where the window begins in it.
    ///
    /// # Panics
    ///
    /// A register moved past the widest offset a span holds.
    #[must_use]
    pub fn shifted(mut self, by: usize) -> Match {
        let by32 = u32::try_from(by).expect("a window begins within the widest offset a span holds");
        let shift = |s: &mut Span| {
            let moved = |at: u32| at.checked_add(by32).expect("a register ends within the widest offset a span holds");
            s.start = moved(s.start);
            s.end = moved(s.end);
        };
        self.start += by;
        self.end += by;
        self.captures.as_mut_slice().iter_mut().for_each(shift);
        if let Some(lists) = self.lists.as_mut() {
            lists.iter_mut().flat_map(|(_, spans)| spans.iter_mut()).for_each(shift);
        }
        self
    }
    /// The bytes the register called `name` bound, or `None` where the
    /// pattern has no such register.
    ///
    /// A register the pattern has but this match did not bind returns an
    /// empty slice, which is the same answer as binding nothing and is not
    /// the same as the pattern never naming it.
    #[must_use]
    pub fn group<'h>(&self, name: &str, input: &'h [u8]) -> Option<&'h [u8]> {
        self.group_span(name).map(|s| &input[s.range()])
    }

    /// The span the register called `name` bound.
    #[must_use]
    pub fn group_span(&self, name: &str) -> Option<Span> {
        let i = self.names().iter().position(|n| n == name)?;
        self.captures.get(i).copied()
    }

    /// The bytes of the `i`th register, in the order this match carries them.
    ///
    /// Positional access exists because a caller that knows the pattern knows
    /// the positions; a caller that does not should ask by name, which is the
    /// handle the pattern actually gave the register.
    #[must_use]
    pub fn group_at<'h>(&self, i: usize, input: &'h [u8]) -> Option<&'h [u8]> {
        self.captures.get(i).map(|s| &input[s.range()])
    }

    /// The whole match's own span.
    #[must_use]
    #[inline]
    pub fn span(&self) -> Span {
        Span { start: self.start as u32, end: self.end as u32 }
    }

    /// The whole match and every register it bound, as a fixed-size array.
    ///
    /// The counterpart of the regex crate's `Captures::extract`, which exists
    /// to destructure a match in one binding rather than unwrapping a group at
    /// a time. Registers arrive in the order [`Self::captures`] holds them,
    /// sorted by name: every trex binding is named and there are no numbered
    /// groups, so there is no group number to order them by.
    ///
    /// [`crate::static_captures_len`] is what says `N` is right for a pattern
    /// before any input is seen.
    ///
    /// # Panics
    ///
    /// When `N` is not the number of registers this match bound. That is the
    /// same contract regex's has, and the reason both are checked rather than
    /// truncated: a silently short array would bind the wrong register to the
    /// wrong name.
    #[must_use]
    pub fn extract<'h, const N: usize>(&self, input: &'h [u8]) -> (&'h [u8], [&'h [u8]; N]) {
        assert_eq!(
            self.captures.len(),
            N,
            "extract::<{N}> on a match that bound {} registers",
            self.captures.len()
        );
        let mut out = [&input[0..0]; N];
        for (slot, span) in out.iter_mut().zip(self.captures()) {
            *slot = &input[span.range()];
        }
        (&input[self.start..self.end], out)
    }
}

/// A match's byte span alone, for the paths that carry no captures: eight
/// bytes where a [`Match`] is fifty-six, twenty-four of them the registers a
/// plain scan never fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    /// Inclusive start byte offset of the match in the input.
    pub start: u32,
    /// Exclusive end byte offset of the match in the input.
    pub end: u32,
}

impl Span {
    /// The start offset as an index.
    #[must_use]
    pub fn start(&self) -> usize {
        self.start as usize
    }

    /// The end offset as an index.
    #[must_use]
    pub fn end(&self) -> usize {
        self.end as usize
    }

    /// How many bytes the span covers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.end().saturating_sub(self.start())
    }

    /// Whether the span covers no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end() <= self.start()
    }

    /// The matched bytes' range in the input.
    #[must_use]
    pub fn range(&self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

impl From<Span> for Match {
    /// The span with no captures.
    #[inline]
    fn from(s: Span) -> Self {
        Match::plain(s.start as usize, s.end as usize)
    }
}

/// Whether `pattern` matches anywhere in `input`.
///
/// Equal to `!scan(pattern, input).is_empty()`, and cheaper where the answer
/// can be reached before every match is found. The absent-literal refusal
/// answers without lexing at all; the window route stops at the first window
/// holding a match and never lexes the rest. Everywhere else the scan runs as
/// it would have, since its routes read the input once and produce their
/// matches together.
#[must_use]
pub fn is_match(pattern: &Pattern, input: &[u8]) -> bool {
    if let Some(shapes) = crate::library::shapes_for(pattern) {
        return !scan_with_shapes(pattern, input, &shapes).is_empty();
    }
    if let Some(found) = routed_is_match_early(pattern, input) {
        return found;
    }
    // The same routes over the pattern with its bindings off, for the reason
    // [`routed_spans`] takes them that way: the routes are written against the
    // shapes the language spells without bindings, and a binding constrains
    // nothing, so `\W:name "="` reaches the route `\W "="` takes only once its
    // binding is off. Whether a match exists is the same question of both.
    if let Some(bare) = pattern.without_bindings()
        && let Some(found) = routed_is_match_early(&bare, input)
    {
        crate::trace::rung("is_match", "a route, with the bindings off", input.len());
        return found;
    }
    if let Some(found) = crate::prefilter::any_required_window(pattern, input) {
        crate::trace::rung("is_match", "the windows a literal opens", input.len());
        return found;
    }
    // Nothing above answered, so there is no literal to search for and the
    // only way to know whether a match exists is to lex until one does. A
    // prefix settles it where the pattern matches early, which is the common
    // case; where it does not, the scan runs as before and the prefix cost a
    // sixty-fourth of a megabyte.
    if crate::prefilter::any_in_prefix(pattern, input) == Some(true) {
        crate::trace::rung("is_match", "a prefix that found one", input.len());
        return true;
    }
    // The held walk, which stops at the first match rather than resuming past
    // it. Nothing above answered and the prefix found nothing, so a match here
    // is late or absent - and both are what the windowed walk is for, since it
    // puts the anchors it has not reached across the cores.
    if let Some(found) = crate::nfa::SerialWalk::any_match(pattern, input) {
        crate::trace::rung("is_match", "the held walk, windowed from the first ask", input.len());
        return found;
    }
    crate::trace::rung("is_match", "every match of the whole scan", input.len());
    !scan(pattern, input).is_empty()
}

/// Whether `pattern` matches anywhere in `input`, from a route that reads to
/// the first match and stops, or `None` where every such route declined.
///
/// The half of [`is_match`]'s ladder whose answer is a byte route's, held as
/// its own function so the bindings-off form asks it and not the rungs below,
/// which lex. The counterpart of [`routed_first_early`], rung for rung.
fn routed_is_match_early(pattern: &Pattern, input: &[u8]) -> Option<bool> {
    if crate::prefilter::requires_absent(pattern, input) {
        crate::trace::rung("is_match", "an absent literal, no lex", input.len());
        return Some(false);
    }
    // One literal word token, answered from the first occurrence the bytes
    // confirm rather than from all of them. The full scan has to walk every
    // occurrence to report them; this reads to the first and stops.
    if let Some(lits) = crate::prefilter::byte_routable_literals(pattern)
        && let Some(found) = crate::prefilter::byte_route_any_word_literal(&lits, input)
    {
        crate::trace::rung("is_match", "a word literal route, no lex", input.len());
        return Some(found);
    }
    // The same literal behind `^`, read to the first occurrence that leads its
    // line. Without this rung the pattern falls to a lexed prefix, which is a
    // lex to answer what the bytes answer on every other ladder.
    if let Some(lit) = crate::prefilter::byte_routable_line_anchored_literal(pattern)
        && let Some(first) = crate::prefilter::byte_route_first_line_anchored_literal(lit, input, 0)
    {
        crate::trace::rung("is_match", "a line-anchored literal route, no lex", input.len());
        return Some(first.is_some());
    }
    // A word token then one byte of plain punctuation, answered the same way
    // from the first occurrence of that byte that is a match.
    if let Some(punct) = crate::prefilter::byte_routable_word_then_punct(pattern)
        && let Some(found) = crate::prefilter::byte_route_any_word_then_punct(punct, input)
    {
        crate::trace::rung("is_match", "a word-then-punct route, no lex", input.len());
        return Some(found);
    }
    // A byte pattern, answered from the first occurrence of its literal
    // prefix that opens a token the pattern matches whole.
    if let Some((bp, prefix)) = crate::prefilter::byte_routable_byte_pattern(pattern)
        && let Some(found) = crate::prefilter::byte_route_any_byte_pattern(bp, &prefix, input)
    {
        crate::trace::rung("is_match", "a byte-pattern route, no lex", input.len());
        return Some(found);
    }
    // A bare kind atom, answered from the first run the bytes settle as that
    // token. The route reports the leftmost match or that there is none, which
    // is this question with the span thrown away.
    if let Some(first) = crate::prefilter::byte_route_kind_from(pattern, input, 0) {
        crate::trace::rung("is_match", "a kind route, no lex", input.len());
        return Some(first.is_some());
    }
    // The windows, stopping at the first occurrence of the opening literal that
    // matches, which reads the input only as far as its answer. Above the two
    // rungs below because both read more of it than that: the required-window
    // route scans for every window a literal opens, and a prefix lexes a
    // sixty-fourth of a megabyte to settle a match that may be twenty bytes in.
    // The trace put is_match at 0.291 ms here where find, which reaches this
    // rung sooner, read 0.00172 for the same question.
    if let Some(found) = crate::prefilter::first_by_literal_windows(pattern, input) {
        crate::trace::rung("is_match", "windows to the first match", input.len());
        return Some(found.is_some());
    }
    None
}

/// Tokenize `input` and scan it for `pattern`, returning the
/// leftmost, non-overlapping matches.
#[must_use]
pub fn scan(pattern: &Pattern, input: &[u8]) -> Vec<Span> {
    // A library kind exists only in a lex told to produce it, so a pattern
    // naming one takes the shaped lex before any route reads the bytes.
    if let Some(shapes) = crate::library::shapes_for(pattern) {
        return scan_with_shapes(pattern, input, &shapes);
    }
    if let Some(spans) = routed_spans(pattern, input) {
        // Named for what this rung knows, which is that the engine did not run.
        // Which route answered is the inner rung's to report, and only some of
        // them skip the lex: the window route lexes a token's worth of bytes at
        // every occurrence of the opening literal and reaches here too.
        crate::trace::rung("scan", "a route, not the engine", input.len());
        return spans;
    }
    // The single-pass engine handles the regular core plus binding,
    // register-equality, and the content guard in linear time, over the
    // significant stream in the lexer's parts. It returns `None` for
    // patterns with a balanced group or field node, whose variable-length
    // advance the set-reachability engine below handles instead.
    //
    // The two are named apart in the trace because they are different engines
    // with different costs, and a reader attributing a pattern to "the engine"
    // cannot tell which one answered it. Measured over one corpus: a balanced
    // group takes the set engine at 44.6203 ms and a star the single-pass one
    // at 37.3523, so a mechanism for either would be sized wrongly by a rung
    // that named only "an engine".
    if let Some(matches) = crate::nfa::scan_nfa(pattern, input) {
        crate::trace::rung("scan", "the single-pass engine over a whole lex", input.len());
        return matches;
    }
    crate::trace::rung("scan", "the set engine over a whole lex", input.len());
    scan_set_reachability(pattern, input)
}

/// The matches of `pattern` from a route that answers without the engine, or
/// `None` where every route declined and the engine must run.
///
/// This is the front of [`scan`] held as its own function because a cursor
/// takes the same routes and must reach the same verdict: a route that
/// answers here answers for both, and only what falls through is walked one
/// match at a time. A second copy of the ladder would be a second set of
/// answers to the same question.
pub(crate) fn routed_spans(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
    // A binding records what a match consumed and constrains nothing, so where
    // no atom reads one back the same spans match without it - and the routes
    // are written against the shapes the language spells without bindings, so
    // `\W:name "="` reaches the route `\W "="` takes only once its binding is
    // off. A caller wanting the registers resolves them against the pattern it
    // was given, over these spans.
    if let Some(bare) = pattern.without_bindings()
        && let Some(spans) = routed_spans(&bare, input)
    {
        crate::trace::rung("scan", "a route, with the bindings off", input.len());
        return Some(spans);
    }
    routed_spans_positional(pattern, input)
        .or_else(|| routed_spans_selective(pattern, input))
        .or_else(|| {
            // The literal windows, last because they do lex - a token's worth
            // of bytes an atom, at the occurrences of the literal the pattern
            // opens with, rather than the whole input. The routes above lex
            // nothing at all and answer their own shapes more cheaply.
            let spans = crate::prefilter::scan_by_literal_windows(pattern, input)?;
            crate::trace::rung("scan", "windows around the opening literal", input.len());
            Some(spans)
        })
}

/// The leftmost match a route can answer, read to that match and no further,
/// or `None` where every route declined.
///
/// [`routed_spans`] reports every match because a scan must. A caller wanting
/// only the first had to take that and discard the rest, which reads the whole
/// input to answer about a match that may be in its first hundred bytes. The
/// routes that can stop early do so here; the ones that cannot yet fall through
/// to the full set, so this is never wrong and only sometimes fast.
///
/// The absent-literal refusal comes first for the same reason it does in
/// [`routed_spans_positional`]: it is a verdict of no match that costs no lex
/// and no positional scan at all.
///
/// The rungs are taken in the order [`routed_spans`] takes them, so a pattern
/// answers here from the same route that answers a scan. Two of them read no
/// further than the match they report - the byte routes, which stop at the
/// first occurrence the bytes confirm, and the windows, which stop at the first
/// window holding a match and lex none of the rest.
///
/// The kind route is deliberately not among them. It reads the whole
/// significant stream and returns every match, so taking its first is a scan
/// spent on one answer: `\W{2}` read 4.782 ms that way against 0.551 from the
/// widening prefix [`crate::find`] falls to when this declines. A caller
/// wanting one match is better served by a path that stops at it, and the scan
/// still takes the kind route through [`routed_spans_selective`].
pub(crate) fn routed_first(pattern: &Pattern, input: &[u8]) -> Option<Option<Span>> {
    routed_first_early(pattern, input)
        .or_else(|| routed_spans_positional(pattern, input).map(|v| v.into_iter().next()))
        .or_else(|| {
            // The windows, stopping at the first occurrence of the opening
            // literal that matches. The scan's form of this builds every match
            // because a scan reports every match; a caller wanting one reads
            // the bytes up to the first and no further.
            let first = crate::prefilter::first_by_literal_windows(pattern, input)?;
            crate::trace::rung("routed_first", "windows to the first match", input.len());
            Some(first)
        })
        .or_else(|| crate::prefilter::first_required_window(pattern, input))
}

/// [`routed_first`] over the positional half of the ladder only.
///
/// For a caller whose answer depends on the matches not overlapping - the
/// soonest-ending match is the first one only when no later match can end
/// earlier, which whole tokens of a fixed count guarantee and the selective
/// routes do not.
pub(crate) fn routed_first_positional(pattern: &Pattern, input: &[u8]) -> Option<Option<Span>> {
    routed_first_early(pattern, input)
        .or_else(|| routed_spans_positional(pattern, input).map(|v| v.into_iter().next()))
        .or_else(|| {
            // The windows, which this half of the ladder may take for the same
            // reason the routes above it may: their shape is a flat sequence of
            // atoms, each consuming one token, so every match spans the same
            // number of whole tokens and a later match closes later. The
            // leftmost is therefore also the soonest-ending, which is what a
            // caller here is asking for.
            let first = crate::prefilter::first_by_literal_windows(pattern, input)?;
            crate::trace::rung("routed_first_positional", "windows to the first match", input.len());
            Some(first)
        })
}

/// [`routed_first`] for a caller anchored at byte `at`.
///
/// The literal route begins its scan there rather than at the start, because
/// the bytes before `at` hold no match the caller wants by definition. The
/// other routes have no anchored form yet and fall through to the positional
/// set, filtered - which is what every anchored entry point did for all of
/// them before.
pub(crate) fn routed_first_at(pattern: &Pattern, input: &[u8], at: usize) -> Option<Option<Span>> {
    if crate::prefilter::requires_absent(pattern, input) {
        return Some(None);
    }
    if let Some(first) = routed_first_at_reading(pattern, input, at, &mut None) {
        crate::trace::rung("routed_first_at", "an anchored byte route, no lex", input.len());
        return Some(first);
    }
    // The same routes over the pattern with its bindings off, for the reason
    // [`routed_spans`] takes them that way. Stripped here and not inside
    // [`routed_first_at_reading`], whose caller asks once a piece: a pattern
    // rebuilt per ask would cost a copy of itself tens of thousands of times.
    if let Some(bare) = pattern.without_bindings()
        && let Some(first) = routed_first_at_reading(&bare, input, at, &mut None)
    {
        crate::trace::rung("routed_first_at", "an anchored route, with the bindings off", input.len());
        return Some(first);
    }
    // The windows, searched from `at` and stopping at the first occurrence that
    // matches. Above the whole-set fallback because that finds every match in
    // the input and throws away the ones before the offset.
    if let Some(first) = crate::prefilter::first_by_literal_windows_at(pattern, input, at) {
        crate::trace::rung("routed_first_at", "windows from the offset", input.len());
        return Some(first);
    }
    let whole = routed_spans_positional(pattern, input)?;
    // Not a byte route from the offset: every match found, then filtered. The
    // rung is named apart from the one above because they cost differently and
    // a reader attributing a row needs to know which answered.
    crate::trace::rung("routed_first_at", "the whole positional set, filtered", input.len());
    Some(whole.into_iter().find(|s| s.start() >= at))
}

/// Every match of `pattern` in `input` at or after byte `at`, from a route that
/// answers without walking the whole token stream, or `None` where every route
/// declined and the walk must run.
///
/// The anchored counterpart of [`routed_spans`], for a caller resuming a scan.
/// Both routes below begin at `at` themselves rather than answering for the
/// whole input and being filtered afterwards, which is the distinction
/// [`routed_first_at`] draws when it restricts its own filter to the positional
/// set: a selected match reaching back across `at` suppresses one starting at
/// `at`, so filtering a whole-input selection would lose that second match.
///
/// An empty vector is a scan that found nothing; `None` is a route that
/// declined to answer. They are not the same and a caller must not read one as
/// the other.
#[must_use]
pub(crate) fn routed_spans_at(pattern: &Pattern, input: &[u8], at: usize) -> Option<Vec<Span>> {
    if crate::prefilter::requires_absent(pattern, input) {
        crate::trace::rung("routed_spans_at", "a literal the input does not hold", input.len());
        return Some(Vec::new());
    }
    let spans = crate::prefilter::scan_by_literal_windows_at(pattern, input, at)?;
    crate::trace::rung("routed_spans_at", "windows from the offset", input.len());
    Some(spans)
}

/// The byte routes of [`routed_first_at`] over a reader the caller holds, and
/// without its whole-set fallback.
///
/// For a caller asking at one ascending position after another - a split taking
/// its pieces one at a time. Those two rungs are the reader's, and repeating
/// them over a reader built per ask costs a fresh quote scan from byte zero
/// every time, which makes such a loop quadratic.
///
/// `None` means no byte route answered. It is the same refusal for every ask,
/// so a caller meeting it falls back once rather than once a piece; the absent
/// literal is left out here for the same reason, being a verdict on the whole
/// input that a caller asks for once.
///
/// The reader is made on the first ask that reaches a route needing one. A
/// reader opens with a search for each quote byte over the whole input, which a
/// pattern no route takes must not be charged for.
pub(crate) fn routed_first_at_reading<'i>(
    pattern: &Pattern,
    input: &'i [u8],
    at: usize,
    reader: &mut Option<crate::prefilter::ByteReader<'i>>,
) -> Option<Option<Span>> {
    if let Some(lits) = crate::prefilter::byte_routable_literals(pattern) {
        let held = reader.get_or_insert_with(|| crate::prefilter::ByteReader::new(input));
        if let Some(first) =
            crate::prefilter::byte_route_first_word_literal_reading(&lits, held, at)
        {
            return Some(first);
        }
    }
    // The same literal behind `^`, read to the first occurrence at or after
    // `at` that leads its line.
    if let Some(lit) = crate::prefilter::byte_routable_line_anchored_literal(pattern) {
        let held = reader.get_or_insert_with(|| crate::prefilter::ByteReader::new(input));
        if let Some(first) =
            crate::prefilter::byte_route_first_line_anchored_literal_reading(lit, held, at)
        {
            return Some(first);
        }
    }
    // A word token then one byte of plain punctuation, which the unanchored
    // ladder already takes and this one did not, so the same pattern answered
    // from the bytes at zero and paid a lex from an offset.
    if let Some(punct) = crate::prefilter::byte_routable_word_then_punct(pattern) {
        let held = reader.get_or_insert_with(|| crate::prefilter::ByteReader::new(input));
        if let Some(first) =
            crate::prefilter::byte_route_first_word_then_punct_reading(punct, held, at)
        {
            return Some(first);
        }
    }
    // A byte pattern, whose match opens with a literal prefix, so the search
    // starts at `at` and every match it reaches begins there or later.
    if let Some((bp, prefix)) = crate::prefilter::byte_routable_byte_pattern(pattern) {
        let held = reader.get_or_insert_with(|| crate::prefilter::ByteReader::new(input));
        if let Some(first) =
            crate::prefilter::byte_route_first_byte_pattern_reading(bp, &prefix, held, at)
        {
            return Some(first);
        }
    }
    // The kind route takes an offset for the same reason the literal one does,
    // so an anchored ask is answered from `at` rather than from a whole set
    // filtered. These are the rows the ladder was losing worst: every "from an
    // offset" operation reached no route at all.
    crate::prefilter::byte_route_kind_reading(pattern, input, reader, at)
}

/// The leftmost match from a route that can stop at it, or `None` where no
/// such route applies and a caller must fall back to a whole set.
///
/// Every route here is positional, so both callers above may take it.
fn routed_first_early(pattern: &Pattern, input: &[u8]) -> Option<Option<Span>> {
    if crate::prefilter::requires_absent(pattern, input) {
        return Some(None);
    }
    // The same routes over the pattern with its bindings off, for the reason
    // [`routed_spans`] takes them that way. Where the first match is does not
    // depend on what it records, so the span these report is the span the
    // pattern's own first match has; a caller wanting the registers resolves
    // them against the pattern it was given, over this span.
    if let Some(bare) = pattern.without_bindings()
        && let Some(first) = routed_first_early(&bare, input)
    {
        crate::trace::rung("routed_first", "a route, with the bindings off", input.len());
        return Some(first);
    }
    if let Some(lits) = crate::prefilter::byte_routable_literals(pattern)
        && let Some(first) = crate::prefilter::byte_route_first_word_literal(&lits, input)
    {
        return Some(first);
    }
    // `^ "let"` is the literal route with one more test on each occurrence, and
    // it stops at the first that passes. The spans route filters a whole set
    // instead, because a scan must report them all.
    if let Some(lit) = crate::prefilter::byte_routable_line_anchored_literal(pattern)
        && let Some(first) = crate::prefilter::byte_route_first_line_anchored_literal(lit, input, 0)
    {
        return Some(first);
    }
    if let Some(punct) = crate::prefilter::byte_routable_word_then_punct(pattern)
        && let Some(first) = crate::prefilter::byte_route_first_word_then_punct(punct, input)
    {
        return Some(first);
    }
    if let Some((bp, prefix)) = crate::prefilter::byte_routable_byte_pattern(pattern)
        && let Some(first) = crate::prefilter::byte_route_first_byte_pattern(bp, &prefix, input)
    {
        return Some(first);
    }
    // A bare kind atom, from the first run the bytes settle as that token. One
    // token of a fixed count, so it belongs on this half of the ladder: its
    // matches cannot overlap and the leftmost also ends first.
    if let Some(first) = crate::prefilter::byte_route_kind_from(pattern, input, 0) {
        return Some(first);
    }
    None
}

/// Whether any route answers `pattern` over `input`, and with what.
///
/// For a measurement that means to time the engine: a route answering the
/// pattern makes every reading a reading of that route instead, and this is
/// how a bench says so rather than assuming.
#[doc(hidden)]
#[must_use]
pub fn routed_spans_public(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
    routed_spans(pattern, input)
}

/// The part of the route ladder whose answer does not depend on where the
/// search began.
///
/// Each of these routes matches whole tokens of a fixed count - one for a
/// literal or a byte pattern, two for a word then punctuation - and two of its
/// matches can never share a token: the second token of a `\W "="` match is
/// punctuation, so no match can begin there. With no overlap possible, the
/// leftmost non-overlapping selection from a later position is exactly the
/// tail of the selection from the start, and a caller anchored at `at` may
/// take these matches and keep the ones that begin at or after it.
///
/// That is what [`routed_spans_selective`] cannot promise, which is why the
/// ladder is cut here rather than kept whole.
pub(crate) fn routed_spans_positional(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
    // Something every match must hold and the input does not: a byte string,
    // or - for the kinds that name no byte string - a run of one byte class
    // long enough to be a token. There is nothing to find, and answering here
    // skips the lex, which a scan that cannot match otherwise pays in full and
    // which costs 217 to 513 MB/s over the corpora it has been read on.
    if crate::prefilter::requires_absent(pattern, input) {
        return Some(Vec::new());
    }
    // One literal word token is answerable without lexing the input: a SIMD
    // search for it, the lexer's own quote reader, a neighbour test, and the
    // lexer over an occurrence's own run where the neighbours do not settle
    // it. The route hands back `None` where even that cannot, and then the
    // lex runs as before.
    if let Some(lits) = crate::prefilter::byte_routable_literals(pattern)
        && let Some(spans) = crate::prefilter::byte_route_word_literals(&lits, input)
    {
        crate::trace::rung("scan", "a word literal route, no lex", input.len());
        return Some(spans);
    }
    // `^ "let"` is the literal route with one more test on each occurrence:
    // every byte back to the previous newline must be whitespace, which is what
    // makes the token the first significant one on its line. The occurrences it
    // rejects cost the search that found them and no lex.
    if let Some(lit) = crate::prefilter::byte_routable_line_anchored_literal(pattern)
        && let Some(spans) = crate::prefilter::byte_route_line_anchored_literal(lit, input)
    {
        crate::trace::rung("scan", "a line-anchored literal route, no lex", input.len());
        return Some(spans);
    }
    // A word token then one byte of plain punctuation, `\W "="`, is answered
    // the same way from every occurrence of that byte.
    if let Some(punct) = crate::prefilter::byte_routable_word_then_punct(pattern)
        && let Some(spans) = crate::prefilter::byte_route_word_then_punct(punct, input)
    {
        crate::trace::rung("scan", "a word-then-punctuation route, no lex", input.len());
        return Some(spans);
    }
    // A byte pattern is a whole-token match, and every match opens with the
    // pattern's literal prefix, so each occurrence of that prefix that opens
    // a token is the only place to try it.
    if let Some((bp, prefix)) = crate::prefilter::byte_routable_byte_pattern(pattern)
        && let Some(spans) = crate::prefilter::byte_route_byte_pattern(bp, &prefix, input)
    {
        crate::trace::rung("scan", "a byte-pattern route, no lex", input.len());
        return Some(spans);
    }
    None
}

/// The part of the route ladder that selects leftmost non-overlapping matches
/// whose length is not fixed, so the selection from a later start can differ
/// from the tail of the selection from the start.
///
/// `\W{2}` over three words matches the first two; from the second word it
/// matches the second and third, which the first selection rejected as
/// overlapping. A caller anchored partway through must therefore run the
/// engine rather than filter these.
pub(crate) fn routed_spans_selective(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
    // A plain literal, a word token and one byte of plain punctuation,
    // `"let" \W "="`, answered from the bytes: the literal by the word
    // literal route, the word and the punctuation by the reader's probes
    // at each anchor. First because it lexes nothing where the windows lex a
    // token's worth of bytes at every occurrence of the literal, and
    // selective rather than positional because a match's word may itself
    // be the literal, so the selection from a later start can differ.
    let literal_word_punct = || {
        let (lit, punct) = crate::prefilter::byte_routable_literal_word_punct(pattern)?;
        let spans = crate::prefilter::byte_route_literal_word_punct(lit, punct, input)?;
        crate::trace::rung("scan", "a literal, word, punctuation route, no lex", input.len());
        Some(spans)
    };
    literal_word_punct()
        // Every match holds the literals the pattern requires, and a match is
        // at most so many tokens long, so the spans around those literals'
        // occurrences are the only places a match can be. Lexing those rather
        // than the input is the saving, so the windows come before the routes
        // that lex the whole input; a pattern holding no literal to anchor on
        // is handed back before a byte is read, and where the spans would cost
        // what the whole lex costs the route declines and the scan runs on.
        .or_else(|| {
            let spans = crate::prefilter::scan_required_windows(pattern, input)?;
            crate::trace::rung("scan", "the windows a literal opens", input.len());
            Some(spans)
        })
        .or_else(|| routed_spans_kind(pattern, input))
        .or_else(|| routed_spans_balanced(pattern, input))
}

/// A balanced group with an unconstrained interior, read off the bracket
/// pairing the lexer records.
///
/// The lexer pairs each bracket as it reads, so a group that asks only where
/// the brackets are has already been answered when the lex ends, and the scan
/// is one pass over the tokens. This shape reaches the set-reachability engine
/// otherwise, which read 44.6203 ms over 7.34 MB against a lex of 2.9.
///
/// The significant parts rather than the full token vector: the parts carry
/// mates of their own, so the route reads them where the chunks were lexed and
/// no chunk is copied into one array first.
fn routed_spans_balanced(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
    let kind = crate::kind_route::bare_balanced(pattern)?;
    Some(crate::parallel_lex::lex_paired_parts_held(input, |parts| {
        crate::kind_route::scan_balanced_parts(kind, parts)
    }))
}

/// A fixed sequence of token kinds - `\W`, `\N`, `\W \N` - read as a window
/// over the significant stream from the lexer's chunk parts in place.
///
/// Held apart from the rest of [`routed_spans_selective`] because the ladder
/// has two orders to keep in step: a scan takes the rungs in this order, and
/// so must a caller wanting one match, which reaches the windows after the
/// positional set as a scan does and leaves this rung out, as
/// [`routed_first`] says.
fn routed_spans_kind(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
    if let Some(codes) = crate::kind_route::kind_sequence(pattern) {
        return Some(crate::parallel_lex::lex_significant_parts_held(input, |parts| {
            crate::kind_route::scan_kind_sequence(&codes, parts)
        }));
    }
    // A repeat whose bounds differ is a run rather than a fixed window, and the
    // same stream answers it: how many of one kind follow, capped at the upper
    // bound. `\W{2,4}` reached the per-anchor engine for that and read 11.3008
    // ms over 7.34 MB.
    let (code, lo, hi) = crate::kind_route::kind_run(pattern)?;
    Some(crate::parallel_lex::lex_significant_parts_held(input, |parts| {
        crate::kind_route::scan_kind_run(code, lo, hi, parts)
    }))
}

/// Tokenize `input` and scan it for `pattern` under a named empty-loop
/// reading.
///
/// [`scan`] is this with [`EmptyLoop::Thompson`], which is the crate's reading
/// and what a pattern gets when it names none.
///
/// The reading is honoured by choosing the engine that holds it, not by
/// teaching either engine the other's. The single-pass engine expresses
/// preference by the order it queues threads and cannot withdraw one it has
/// already queued, which is what the backtracking reading needs when a body
/// turns out to match empty; the set engine carries each derivation's path and
/// can compare them, so it holds that reading natively. `\|>` and an atomic
/// group route by the same rule and for the same reason.
///
/// Only a repetition whose body can match without consuming a token is
/// affected. Everywhere else the two readings agree, and the pattern stays on
/// the single-pass engine whichever it names.
#[must_use]
pub fn scan_with_empty_loop(pattern: &Pattern, input: &[u8], empty: EmptyLoop) -> Vec<Span> {
    if empty == EmptyLoop::Perl && crate::nfa::empty_loop_needs_set_engine(pattern) {
        return scan_set_reachability(pattern, input);
    }
    scan(pattern, input)
}

/// Tokenize `input` with `shapes` in force and scan it for `pattern`.
///
/// The shapes decide token boundaries, so they belong to the lex rather than
/// the match: a `\{name}` atom in the pattern is matching a token the shape
/// created. Pass the same set to [`crate::parser::parse_with_shapes`], or the
/// atom naming a shape has no id to resolve.
#[must_use]
pub fn scan_with_shapes(
    pattern: &Pattern,
    input: &[u8],
    shapes: &crate::custom::ShapeSet,
) -> Vec<Span> {
    // The library kinds the pattern names join the caller's shapes, so a
    // `\{iban}` beside a declared shape is lexed as both.
    let shapes = shapes.with_library_shapes(&pattern.library_kinds());
    if shapes.is_empty() {
        return scan(pattern, input);
    }
    crate::trace::rung("scan", "the engines over a lex under the declared shapes", input.len());
    let blobs = crate::lexer::blob_runs(input);
    let toks = crate::lexer::lex_with_shapes(input, &blobs, &shapes, 0);
    // Same routing as `scan`: the single-pass engine first, the
    // set-reachability fold for what it declines. Handing the tokens over
    // rather than re-lexing is what keeps the shapes' boundaries.
    if let Some(matches) = crate::nfa::scan_nfa_over(pattern, input, &toks) {
        return matches;
    }
    scan_tokens_from(pattern, input, &toks, 0)
}

/// [`scan_with_shapes`] from the first token starting at or after byte
/// `at`, the leftmost non-overlapping selection re-run from there.
#[must_use]
pub(crate) fn scan_with_shapes_from(
    pattern: &Pattern,
    input: &[u8],
    shapes: &crate::custom::ShapeSet,
    at: usize,
) -> Vec<Span> {
    // This entry point lexes `input` whole however small `at` leaves the walk,
    // so the three phases divide a cost that reads as one. A caller holding a
    // lex of the same bytes already - the streaming commit is one - pays the
    // first two over again, and the division is what says how much that is.
    let preparing = crate::trace::phase("the resumed scan: its shapes and blob runs");
    let shapes = shapes.with_library_shapes(&pattern.library_kinds());
    drop(preparing);
    // The routes read the input's own bytes and know nothing of a declared
    // shape, which is the line `scan` draws above [`routed_spans`] and
    // `pattern_set` draws before it routes a member. Asked above the lex
    // because a route that answers needs no tokens at all.
    if shapes.is_empty()
        && let Some(spans) = routed_spans_at(pattern, input, at)
    {
        return spans;
    }
    let tabling = crate::trace::phase("the resumed scan: its blob runs");
    let blobs = crate::lexer::blob_runs(input);
    drop(tabling);
    let lexing = crate::trace::phase("the resumed scan: its own lex");
    let toks = crate::lexer::lex_with_shapes(input, &blobs, &shapes, 0);
    drop(lexing);
    crate::trace::counted(
        "the resumed scan: bytes it lexes",
        u64::try_from(input.len()).expect("an input within the counter's width"),
    );
    scan_over_tokens_from(pattern, input, &toks, at)
}

/// [`scan_with_shapes_from`] over a lex of `input` the caller already holds.
///
/// `toks` must be the lex this call would have taken itself. For a pattern
/// drawing no library kinds that is what [`crate::lexer::lex`] returns:
/// `lex` is [`crate::lexer::lex_with_shapes`] over an empty shape set, and the
/// set this builds is empty exactly then, so the two agree token for token.
#[must_use]
pub(crate) fn scan_over_tokens_from(
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    at: usize,
) -> Vec<Span> {
    let walking = crate::trace::phase("the resumed scan: its walk from the anchor");
    let start = toks.partition_point(|t| t.start() < at);
    let found = match crate::nfa::scan_nfa_over_serial_from(pattern, input, toks, start) {
        Some(matches) => matches,
        None => scan_tokens_from(pattern, input, toks, start),
    };
    drop(walking);
    found
}

/// The registers each of `spans` bound, one [`Match`] per span in order.
///
/// `spans` are the matches a scan of `pattern` over `input` returned. Each is
/// resolved by re-running the attempt anchored where it starts, which is how
/// the engines resolve a selected match; a pattern that binds nothing gets
/// its spans back with empty captures and no lex.
///
/// # Panics
///
/// When a span is not a match of `pattern` over `input`.
#[must_use]
pub fn captures(pattern: &Pattern, input: &[u8], spans: &[Span]) -> Vec<Match> {
    if !pattern.binds() {
        crate::trace::rung("captures", "nothing bound, the spans back", input.len());
        return spans.iter().copied().map(Match::from).collect();
    }
    if let Some(shapes) = crate::library::shapes_for(pattern) {
        return captures_with_shapes(pattern, input, &shapes, spans);
    }
    // One span is what every first-match caller asks for, and a route found it
    // without lexing. Lexing the whole input to resolve it costs back exactly
    // what the route saved, so the tokens around the match answer it instead.
    if let [span] = spans
        && let Some(m) = captures_in_region(pattern, input, *span)
    {
        crate::trace::rung("captures", "one span, over its own region", input.len());
        return vec![m];
    }
    // A window at each span, for the reason the single-span case above gives: a
    // route found these without lexing, so reading a whole lex back to say what
    // they bound costs exactly what the route saved. A window is bounded - a
    // token's worth of bytes an atom - which is what a region settled per span
    // is not, and that difference is why this generalises where the region does
    // not.
    // The bytes of a match say where its atoms' tokens are, for the shape whose
    // atoms are literals and word tokens, so this resolves without lexing at
    // all - which is what the route that found these spans also did.
    if let Some((ms, names)) = crate::prefilter::flat_captures_by_byte_bounds(pattern, input, spans) {
        crate::trace::rung("captures", "the registers off each match's own bytes", input.len());
        return ms
            .into_iter()
            .map(|m| {
                Match::bound(
                    m.span.start(),
                    m.span.end(),
                    Regs::from_slice(&m.regs[..names.len()]),
                    names.clone(),
                )
            })
            .collect();
    }
    if let Some(ms) = crate::prefilter::captures_by_windows(pattern, input, spans) {
        crate::trace::rung("captures", "a window at each span", input.len());
        return ms;
    }
    // The single-pass engine over the significant parts, which is the engine
    // the scan that produced these spans ran on. It declines exactly what that
    // scan routes to the set engine, and only those reach the stitched lex.
    if let Some(ms) = crate::nfa::captures_over_parts(pattern, input, spans) {
        crate::trace::rung("captures", "an attempt a span, over the lexer's parts", input.len());
        return ms;
    }
    crate::trace::rung("captures", "the set engine over a whole stitched lex", input.len());
    crate::parallel_lex::lex_parallel_held(input, |toks| {
        captures_routed(pattern, input, toks, spans, EmptyLoop::Thompson, false)
    })
}

/// [`captures`], with every binding a register bound under a repetition
/// made kept beside its last, read back through [`Match::list`].
///
/// The lists live in the set engine's states, so the spans are resolved
/// there whatever engine found them; a caller that reads only the last
/// binding takes [`captures`] and its cheaper rungs. A pattern that binds
/// nothing under a repetition resolves as [`captures`] does.
#[must_use]
pub fn captures_with_lists(pattern: &Pattern, input: &[u8], spans: &[Span]) -> Vec<Match> {
    if !pattern.has_list_registers() {
        return captures(pattern, input, spans);
    }
    if let Some(shapes) = crate::library::shapes_for(pattern) {
        return captures_with_shapes_and_lists(pattern, input, &shapes, spans);
    }
    crate::trace::rung("captures", "the set engine, keeping every binding under a repetition", input.len());
    crate::parallel_lex::lex_parallel_held(input, |toks| {
        captures_routed(pattern, input, toks, spans, EmptyLoop::Thompson, true)
    })
}

/// [`captures_with_shapes`], keeping every binding under a repetition as
/// [`captures_with_lists`] does.
#[must_use]
pub fn captures_with_shapes_and_lists(
    pattern: &Pattern,
    input: &[u8],
    shapes: &crate::custom::ShapeSet,
    spans: &[Span],
) -> Vec<Match> {
    if !pattern.has_list_registers() {
        return captures_with_shapes(pattern, input, shapes, spans);
    }
    let shapes = shapes.with_library_shapes(&pattern.library_kinds());
    if shapes.is_empty() {
        return captures_with_lists(pattern, input, spans);
    }
    let blobs = crate::lexer::blob_runs(input);
    let toks = crate::lexer::lex_with_shapes(input, &blobs, &shapes, 0);
    captures_routed(pattern, input, &toks, spans, EmptyLoop::Thompson, true)
}

/// What `span` bound, resolved over the tokens around it, or `None` where no
/// region answers for the pattern and the whole input must be lexed.
///
/// The attempt anchored at the span's start is how both engines resolve a
/// selected match, and it reads no token outside the match except through the
/// assertions [`crate::prefilter::tokens_around`] refuses to settle a region
/// for.
fn captures_in_region(pattern: &Pattern, input: &[u8], span: Span) -> Option<Match> {
    match crate::prefilter::tokens_around(pattern, input, span) {
        Ok(toks) => crate::nfa::captures_over(pattern, input, &toks, &[span])
            .and_then(|v| v.into_iter().next()),
        Err(why) => {
            debug_assert!(
                !matches!(why, crate::prefilter::RegionRefusal::NoRegionForm)
                    || crate::nfa::bounded_max_len(pattern).is_none_or(|n| n == 0)
                    || pattern.starts_with_resume()
                    || pattern.mentions_reset_start()
                    || pattern.mentions_stream_end_anchor(),
                "a region was refused as `{why}` for a pattern that has one"
            );
            None
        }
    }
}

/// [`captures`] for spans a [`scan_with_empty_loop`] under `empty` returned.
///
/// # Panics
///
/// When a span is not a match of `pattern` over `input` under that reading.
#[must_use]
pub fn captures_with_empty_loop(
    pattern: &Pattern,
    input: &[u8],
    empty: EmptyLoop,
    spans: &[Span],
) -> Vec<Match> {
    if !pattern.binds() {
        return spans.iter().copied().map(Match::from).collect();
    }
    crate::parallel_lex::lex_parallel_held(input, |toks| {
        captures_routed(pattern, input, toks, spans, empty, false)
    })
}

/// [`captures`] for spans a [`scan_with_shapes`] under `shapes` returned: the
/// input is lexed with the same shapes, so the attempts see the boundaries
/// the scan saw.
///
/// # Panics
///
/// When a span is not a match of `pattern` over `input` under those shapes.
#[must_use]
pub fn captures_with_shapes(
    pattern: &Pattern,
    input: &[u8],
    shapes: &crate::custom::ShapeSet,
    spans: &[Span],
) -> Vec<Match> {
    let shapes = shapes.with_library_shapes(&pattern.library_kinds());
    if shapes.is_empty() {
        return captures(pattern, input, spans);
    }
    if !pattern.binds() {
        return spans.iter().copied().map(Match::from).collect();
    }
    let blobs = crate::lexer::blob_runs(input);
    let toks = crate::lexer::lex_with_shapes(input, &blobs, &shapes, 0);
    captures_routed(pattern, input, &toks, spans, EmptyLoop::Thompson, false)
}

/// [`captures`] over the token stream the spans were scanned on.
///
/// # Panics
///
/// When a span is not a match of `pattern` over `toks`.
#[must_use]
pub fn captures_over(pattern: &Pattern, input: &[u8], toks: &[Token], spans: &[Span]) -> Vec<Match> {
    if !pattern.binds() {
        return spans.iter().copied().map(Match::from).collect();
    }
    captures_routed(pattern, input, toks, spans, EmptyLoop::Thompson, false)
}

/// [`captures_over`], keeping every binding under a repetition as
/// [`captures_with_lists`] does; a pattern binding nothing under one
/// resolves as [`captures_over`] does.
///
/// # Panics
///
/// When a span is not a match of `pattern` over `toks`.
#[must_use]
pub fn captures_over_with_lists(
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    spans: &[Span],
) -> Vec<Match> {
    if !pattern.has_list_registers() {
        return captures_over(pattern, input, toks, spans);
    }
    captures_routed(pattern, input, toks, spans, EmptyLoop::Thompson, true)
}

/// Resolve `spans` on the engine a scan under `empty` ran them on: the
/// single-pass engine where it takes the pattern, else the set engine, whose
/// attempt at a span's start token reproduces the match with its registers.
/// Under `keep_lists` the set engine resolves every span, since only its
/// states keep every binding a register makes under a repetition.
fn captures_routed(
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    spans: &[Span],
    empty: EmptyLoop,
    keep_lists: bool,
) -> Vec<Match> {
    let set_engine = (empty == EmptyLoop::Perl && crate::nfa::empty_loop_needs_set_engine(pattern))
        || (keep_lists && pattern.has_list_registers());
    if !set_engine && let Some(matches) = crate::nfa::captures_over(pattern, input, toks, spans) {
        return matches;
    }
    let n = toks.len();
    let analyses = Analyses::build(pattern, input, toks);
    let fields = analyses.fields();
    // The registers in name order, which is the order the single-pass engine
    // reports them in: a match from either engine carries the same registers at
    // the same positions, and the names are built once for the whole scan.
    let mut order: Vec<usize> = (0..analyses.regs.len()).collect();
    order.sort_by(|&a, &b| analyses.regs[a].cmp(&analyses.regs[b]));
    let names: std::sync::Arc<[String]> =
        order.iter().map(|&i| analyses.regs[i].clone()).collect();
    spans
        .iter()
        .map(|&span| {
            let i = toks.partition_point(|t| t.start < span.start);
            let (env, history) = (i < n)
                .then(|| {
                    best_at(pattern, input, toks, n, i, &analyses.absent, &analyses.regs, fields, &mut 0, |best| {
                        (best.pos, best.env.clone(), best.history())
                    })
                })
                .flatten()
                .filter(|&(end, _, _)| toks[end - 1].end == span.end)
                .map(|(_, env, history)| (env, history))
                .expect("a span a scan of this pattern returned is reproduced by the attempt at its start");
            // Every register the pattern names, not only the ones this match
            // bound: one that bound nothing carries an empty span, which is
            // what tells it from a register the pattern never named and is how
            // the single-pass engine reports it too.
            let captures = order
                .iter()
                .map(|&id| {
                    env.iter()
                        .find(|entry| *entry.0 as usize == id)
                        .map_or(Span { start: 0, end: 0 }, |entry| Span {
                            start: entry.1.0 as u32,
                            end: entry.1.1 as u32,
                        })
                })
                .collect();
            if !keep_lists {
                return Match::bound(span.start(), span.end(), captures, names.clone());
            }
            // Every register bound under a repetition, with each binding it
            // made in this match, oldest first, at its place in name order.
            let mut lists: Bindings = analyses
                .lists
                .iter()
                .filter_map(|&id| order.iter().position(|&o| o == usize::from(id)).map(|at| (at, Vec::new())))
                .collect();
            for (id, (s, e)) in history {
                if let Some(at) = order.iter().position(|&o| o == usize::from(id))
                    && let Some(slot) = lists.iter_mut().find(|(k, _)| *k == at)
                {
                    slot.1.push(Span { start: s as u32, end: e as u32 });
                }
            }
            Match::bound(span.start(), span.end(), captures, names.clone()).with_lists(lists)
        })
        .collect()
}

/// The per-scan analyses the set engine reads through [`Fields`]: the guard
/// literals a prefilter proved absent, the interned register names, and each
/// axis field the pattern queries. Owned here, so an anchored attempt can be
/// run at any position after the scan that built them.
struct Analyses {
    absent: HashSet<Vec<u8>>,
    clock: crate::typed::Clock,
    regs: Vec<String>,
    spectral: Option<SpectralField>,
    seam_cuts: Option<Vec<usize>>,
    depths: Option<Vec<u16>>,
    obs_contested: Option<Vec<usize>>,
    echo: Vec<(crate::orbit::OrbitGroup, crate::echo::EchoField)>,
    supers: Option<crate::supertoken::SuperContext>,
    order: Option<Vec<Option<bool>>>,
    templates: Option<crate::templates::Mining>,
    joins: Vec<Join>,
    seam_token: Option<Vec<usize>>,
    seam_super: Option<Vec<usize>>,
    obs_token: Option<Vec<usize>>,
    obs_super: Option<Vec<usize>>,
    gravity: [Option<crate::gravity::Readings>; 3],
    kin: Vec<(crate::ast::Grain, String, Option<u32>)>,
    bands: Option<Bands>,
    field_starts: Option<Vec<u32>>,
    context: ContextBuild,
    /// The ids of the registers bound under a repetition.
    lists: Vec<u16>,
}

/// The periods a named phase anchor counts against, read once per scan.
struct Bands {
    /// The stream's live periods, strongest first.
    live: Vec<u16>,
    /// Per token, its index among the significant tokens, `u32::MAX` for an
    /// insignificant one.
    sig_index: Vec<u32>,
}

impl Bands {
    fn build(toks: &[Token], input: &[u8]) -> Bands {
        let mut next = 0u32;
        let sig_index = toks
            .iter()
            .map(|t| {
                if t.is_significant() {
                    next += 1;
                    next - 1
                } else {
                    u32::MAX
                }
            })
            .collect();
        Bands { live: crate::context::live_periods(toks, input), sig_index }
    }

    /// Whether token `p` sits at column `k` of the period `period` names,
    /// where that period is live.
    fn at(&self, p: usize, k: u16, period: crate::ast::PeriodRef) -> bool {
        let named = match period {
            crate::ast::PeriodRef::Length(len) => self.live.contains(&len).then_some(len),
            crate::ast::PeriodRef::Rank(n) => usize::from(n).checked_sub(1).and_then(|i| self.live.get(i).copied()),
        };
        match (named, self.sig_index.get(p)) {
            (Some(len), Some(&i)) if i != u32::MAX => i % u32::from(len) == u32::from(k),
            (Some(_) | None, Some(_) | None) => false,
        }
    }
}

/// The slot a grain's readings take in [`Fields::gravity`].
fn grain_slot(grain: crate::ast::Grain) -> usize {
    match grain {
        crate::ast::Grain::Byte => 0,
        crate::ast::Grain::Token => 1,
        crate::ast::Grain::Super => 2,
    }
}

impl Analyses {
    /// Build what `pattern` reads over `toks`. Each axis field is built only
    /// when the pattern queries it (`\F{...}` the spectral field, `@seam` the
    /// seam field); a pattern that queries no axis pays nothing.
    fn build(pattern: &Pattern, input: &[u8], toks: &[Token]) -> Self {
        let absent = crate::prefilter::absent_guard_literals(pattern, input);
        let regs = collect_registers(pattern);
        let lists: Vec<u16> =
            pattern.list_registers().iter().filter_map(|name| reg_id(&regs, name)).collect();
        // Built for the readings this pattern's atoms name and no others: the
        // field is a step a byte, so one it never reads is paid for at every
        // byte of the input.
        let spectral = pattern.has_spectral().then(|| {
            let _building = crate::trace::phase("the spectral field");
            crate::spectral::analyze_needing(input, &Default::default(), pattern.spectral_needs())
        });
        let seam_cuts = pattern.has_seam().then(|| {
            let _building = crate::trace::phase("the seam cuts");
            crate::seam::analyze(input).strong_cuts()
        });
        let depths = pattern.has_stress().then(|| {
            let _building = crate::trace::phase("the nesting depths");
            crate::stress::depths(toks)
        });
        let obs_contested = pattern.has_observation().then(|| {
            let _building = crate::trace::phase("the contested points");
            crate::observation::analyze(input).contested
        });
        // One recurrence field per rung the pattern counts at, so a pattern
        // naming no orbit builds the one `@novel` and `@echoed` read.
        let echo: Vec<(crate::orbit::OrbitGroup, crate::echo::EchoField)> = {
            let _building = crate::trace::phase("the recurrence fields");
            pattern
                .echo_orbits()
                .into_iter()
                .map(|g| {
                    let cfg = crate::echo::EchoConfig { orbit: g, ..Default::default() };
                    (g, crate::echo::analyze_with(toks, input, &cfg))
                })
                .collect()
        };
        let supers = pattern.has_super().then(|| {
            let _building = crate::trace::phase("the supertoken tower");
            crate::supertoken::SuperContext::build(toks, input)
        });
        let clock = crate::typed::Clock::current();
        let order = pattern.has_order().then(|| {
            let _building = crate::trace::phase("the timestamp order");
            timestamp_order(toks, input, clock)
        });
        let templates = pattern.has_rare().then(|| {
            let _building = crate::trace::phase("the mined templates");
            crate::templates::Mining::mine_tokens(toks, input)
        });
        // Timed whether or not the pattern names an input to join against: a
        // shape with none reads this as the cost of asking, which is the figure
        // that says whether the question is worth skipping.
        let joins: Vec<Join> = {
            let _building = crate::trace::phase("the joins, one a named input");
            pattern
                .joins()
                .into_iter()
                .map(|(other, group)| Join::build(other, group, toks, input))
                .collect()
        };
        let obs_cfg = crate::observation::ObservationConfig::default();
        let seam_cfg = crate::seam::SeamConfig::default();
        let seam_token = pattern.has_seam_at(crate::ast::Grain::Token).then(|| {
            let _building = crate::trace::phase("the seam cuts, over the tokens");
            crate::seam::analyze_tokens(toks, &seam_cfg)
        });
        let seam_super = pattern.has_seam_at(crate::ast::Grain::Super).then(|| {
            let _building = crate::trace::phase("the seam cuts, over the supertokens");
            crate::seam::analyze_supertokens(
                &crate::supertoken::supertokens_from(toks, input),
                &seam_cfg,
            )
        });
        let obs_token = pattern.has_observation_at(crate::ast::Grain::Token).then(|| {
            let _building = crate::trace::phase("the contested tokens");
            crate::observation::contested_tokens(toks, &obs_cfg)
        });
        let obs_super = pattern.has_observation_at(crate::ast::Grain::Super).then(|| {
            let _building = crate::trace::phase("the contested supertokens");
            crate::observation::contested_supertokens(
                &crate::supertoken::supertokens_from(toks, input),
                &obs_cfg,
            )
        });
        let gravity = [crate::ast::Grain::Byte, crate::ast::Grain::Token, crate::ast::Grain::Super].map(|g| {
            pattern.has_gravity_at(g).then(|| {
                let _building = crate::trace::phase(match g {
                    crate::ast::Grain::Byte => "the pair field, over the bytes",
                    crate::ast::Grain::Token => "the pair field, over the tokens",
                    crate::ast::Grain::Super => "the pair field, over the supertokens",
                });
                crate::gravity::Readings::read(g, input, toks)
            })
        });
        let kin = pattern
            .kin_examples()
            .into_iter()
            .map(|(g, x)| {
                let named = gravity[grain_slot(g)]
                    .as_ref()
                    .and_then(|r| crate::gravity::example_key(g, x.as_bytes()).and_then(|k| r.type_named(&k)));
                (g, x, named)
            })
            .collect();
        let bands = pattern.has_phase_in().then(|| {
            let _building = crate::trace::phase("the live periods");
            Bands::build(toks, input)
        });
        let field_starts = pattern.has_field_anchor().then(|| {
            let _building = crate::trace::phase("the field starts");
            field_start_index(input, toks)
        });
        let context = {
            let _building = crate::trace::phase("the context field");
            build_context(pattern, input, toks, spectral.as_ref(), echo_at(&echo, crate::orbit::OrbitGroup::Identity))
        };
        Analyses {
            absent,
            clock,
            regs,
            order,
            templates,
            joins,
            spectral,
            seam_cuts,
            depths,
            obs_contested,
            echo,
            supers,
            seam_token,
            seam_super,
            obs_token,
            obs_super,
            gravity,
            kin,
            bands,
            field_starts,
            context,
            lists,
        }
    }

    /// The fields as the matcher reads them.
    fn fields(&self) -> Fields<'_> {
        Fields {
            spectral: self.spectral.as_ref(),
            seam_cuts: self.seam_cuts.as_deref(),
            depths: self.depths.as_deref(),
            obs_contested: self.obs_contested.as_deref(),
            echo: &self.echo,
            supers: self.supers.as_ref(),
            order: self.order.as_deref(),
            templates: self.templates.as_ref(),
            joins: &self.joins,
            seam_token: self.seam_token.as_deref(),
            seam_super: self.seam_super.as_deref(),
            obs_token: self.obs_token.as_deref(),
            obs_super: self.obs_super.as_deref(),
            gravity: self.gravity.each_ref().map(Option::as_ref),
            kin: &self.kin,
            bands: self.bands.as_ref(),
            field_starts: self.field_starts.as_deref(),
            context: self.context.window.as_ref(),
            relation: self.context.relation.as_ref(),
            clock: self.clock,
            lists: &self.lists,
        }
    }
}

/// Scan using only the set-reachability engine, bypassing the
/// single-pass engine. This is the fallback path for balanced and
/// field patterns, and the differential oracle the single-pass engine
/// is checked against.
pub(crate) fn scan_set_reachability(pattern: &Pattern, input: &[u8]) -> Vec<Span> {
    // The three phases a scan here spends its time in, named so a reading says
    // which one a change reached. The lex is the whole of what happens before
    // the closure runs, so its phase ends as the closure begins.
    let _whole = crate::trace::phase("the set engine");
    let lexing = crate::trace::phase("its lex, stitched");
    // Parallel lexer (serial under its own threshold) into this thread's
    // held buffers, so large balanced and field scans pay neither a serial
    // tokenization prefix nor fresh pages.
    crate::parallel_lex::lex_parallel_held(input, |toks| {
        drop(lexing);
        let building = crate::trace::phase("its axis fields");
        let analyses = Analyses::build(pattern, input, toks);
        drop(building);
        let _sweeping = crate::trace::phase("its sweep over the anchors");
        scan_tokens(pattern, input, toks, &analyses)
    })
}

/// Scan a pre-lexed token stream, beginning the leftmost,
/// non-overlapping selection at token index `start` and computing match
/// attempts only at or after it. Token byte offsets are absolute, so a
/// caller streaming a growing token vector can resume here without
/// rescanning the committed prefix. Used by the dual-grain pipeline,
/// where the byte grain lexes ahead and the token grain matches behind.
#[must_use]
pub fn scan_tokens_from(
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    start: usize,
) -> Vec<Span> {
    let n = toks.len();
    let analyses = Analyses::build(pattern, input, toks);
    let fields = analyses.fields();
    let results: Vec<StartMatch> = (0..n)
        .map(|i| {
            if i < start {
                None
            } else {
                attempt_at(
                    pattern,
                    input,
                    toks,
                    n,
                    i,
                    &analyses.absent,
                    &analyses.regs,
                    fields,
                    &mut 0,
                )
            }
        })
        .collect();
    let mut matches = Vec::new();
    let mut s = start;
    while s < n {
        let s0 = skip_ws(toks, s, n);
        if s0 >= n {
            break;
        }
        if let Some(best_pos) = results[s0] {
            matches.push(Span { start: toks[s0].start, end: toks[best_pos - 1].end });
            s = best_pos;
        } else {
            s = s0 + 1;
        }
    }
    matches
}

/// Token count below which the per-start scan runs on one thread while this
/// process has not yet dispatched across cores.
///
/// The pool spawns on its first dispatch, and a scan that only happens once
/// pays that spawn out of its own time - about six and a half milliseconds of
/// it, which is more than a narrow scan of anything under twenty thousand
/// tokens costs in total. Over prefixes of real source, each reading the median
/// of nine processes that scan once and exit:
///
///    5890 tokens    1.0362 ms on one thread    7.0672 ms across cores
///   12425           2.2382                     7.3657
///   15748           2.6766                     8.0069
///   19025           3.3565                     7.3222
///   22496           3.9576                     7.5519
///   25473          11.7819                     9.6438
///   28484          12.2419                    10.1085
///   50005          18.3677                    13.1723
///  184329          39.4007                    17.3067
///
/// One thread wins by three times at 15748 and is still ahead at 22496; the
/// cores are ahead from 25473 on. The narrow reading climbs from 3.96 to 11.78
/// ms between those two, three times the cost for an eighth more work, and that
/// step is where the two curves cross. This bound sits inside it.
///
/// Across cores the figure barely moves over the whole range - 7.07 to 10.11 ms
/// from 5890 tokens to 28484 - because what a cold wide scan costs is mostly
/// the spawn and hardly at all the scanning.
const PARALLEL_SCAN_THRESHOLD_COLD: usize = 24_576;

/// The same bound once the pool is up, when the spawn is already paid.
///
/// Warm, on the same file and sizes, the median of five rounds with the
/// spreads disjoint except where this says they meet:
///
///    192 tokens   0.0100 ms on one thread   0.0173 ms across cores
///    400          0.0227                    0.0296
///    809          0.0448                    0.0438   (the spreads meet)
///   1259          0.0888                    0.0874   (the spreads overlap)
///   1672          0.1278                    0.0970
///   3337          0.4020                    0.2591
///
/// One thread wins below about four hundred tokens, the two are worth the same
/// from about eight hundred to about thirteen hundred, and the cores win by a
/// quarter at 1672 and better than a third at 3337. This bound sits inside the
/// band where they are worth the same, so crossing it can cost nothing and
/// everything above it is a reading where the cores are ahead.
///
/// The figure this replaced said the parallel path wins "from a few hundred
/// tokens up". At four hundred tokens it loses by thirty percent.
const PARALLEL_SCAN_THRESHOLD_WARM: usize = 1024;

/// Whether this process has already dispatched the sweep across cores.
///
/// The bound above depends on it, and nothing else can answer it: flynnel's
/// `global_local_arena` builds the arena when it is asked for, so there is no
/// way to read whether the pool is up without starting one. This records what
/// trex itself has done, which is the fact that decides whether the spawn is
/// still to pay.
///
/// A process where something else warmed the pool reads `false` here and stays
/// on one thread, which forgoes the warm saving and never risks the cold cost.
static POOL_WARM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Tokens this process has swept, until the pool is up.
///
/// A scan under the cold bound never spawns the pool, so a process that only
/// ever scans small inputs never reaches the warm bound however many it does -
/// and the warm bound is where the cores are ahead by a quarter. Counting what
/// has been swept lets the spawn be earned by the work in aggregate rather than
/// by one input being large enough on its own.
static TOKENS_SWEPT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Tokens a process must have swept before it spawns the pool for work that no
/// single scan would justify.
///
/// The spawn is about six and a half milliseconds. What warming buys is the gap
/// between the two warm paths, which is 0.0306 ms over 1672 tokens and 0.1346
/// over 3337 - 18.3 and 40.3 nanoseconds a token. So the spawn is repaid
/// somewhere between 160,000 and 355,000 tokens of sweeping, and this sits at
/// the far end of that: a process that stops short of it has lost nothing, and
/// one that carries on past it was going to repay the spawn several times over.
const TOKENS_BEFORE_WARMING: usize = 350_000;

/// The end position of the longest match anchored at one token index, or
/// `None` when nothing matches there.
type StartMatch = Option<usize>;

fn scan_tokens(pattern: &Pattern, input: &[u8], toks: &[Token], analyses: &Analyses) -> Vec<Span> {
    let n = toks.len();
    // The two halves of the sweep, named apart. The attempts fan out across
    // cores and the selection is one serial walk, so a share that is the whole
    // sweep cannot say whether a change belongs in the anchor's work or in the
    // walk that reads it back. The phase wraps the fan-out rather than sitting
    // inside its leaf: a leaf takes a lock on the way out, and a lock per
    // anchor over millions of anchors would price the watch above the work.
    let attempting = crate::trace::phase("its sweep: an attempt a start");
    // `results[i]`: the longest match anchored at token `i`. Each attempt is
    // an independent, pure `advance` call, so they fan out across cores with
    // no shared state.
    let results = per_start_matches(
        pattern,
        input,
        toks,
        n,
        &analyses.absent,
        &analyses.regs,
        analyses.fields(),
    );
    drop(attempting);

    let _selecting = crate::trace::phase("its sweep: the leftmost selection");

    // The leftmost, non-overlapping selection over the precomputed per-start
    // matches: a serial pass identical to the one-pass scan, so only the
    // expensive per-start attempts were parallelized.
    // `\G` requires each match to begin where the previous ended. Every
    // continuation already does, since the loop resumes at the next
    // significant token after a match; what it forbids is the other branch,
    // where a token that cannot match is stepped over. So the run covers a
    // contiguous stretch and ends at the first token the pattern cannot take.
    let contiguous = pattern.starts_with_resume();
    // Unreserved on purpose. Half of what a match costs here is the growth this
    // would remove - 1300721 matches read 4.912 ms above the walk unreserved
    // and 2.322 reserved - but the only capacity that removes it is one
    // proportional to the token count, and a `Span` is sixteen bytes: reserving
    // `n` of them is about six bytes for every input byte, so a half-gigabyte
    // input would reserve gigabytes to hold what it actually matches. A fixed
    // cap removes the early doublings, which are the cheap ones. Counting the
    // matches first costs a pass over `results` worth more than the growth.
    let mut matches = Vec::new();
    let mut start = 0;
    while start < n {
        let s0 = skip_ws(toks, start, n);
        if s0 >= n {
            break;
        }
        if let Some(best_pos) = results[s0] {
            matches.push(Span { start: toks[s0].start, end: toks[best_pos - 1].end });
            start = best_pos;
        } else if contiguous {
            break;
        } else {
            start = s0 + 1;
        }
    }
    matches
}

/// For each token index, the longest match anchored there. Large
/// streams split the work across the available cores; small streams
/// stay on one thread.
#[allow(clippy::too_many_arguments)]
fn per_start_matches(
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    n: usize,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> Vec<StartMatch> {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    // Read once rather than once a leaf: it walks the pattern, and the pattern
    // does not change under the sweep.
    let opens = crate::kind_route::first_kinds(pattern);
    // Which bound applies is a fact about this process rather than this scan:
    // the spawn is paid once and every scan after it is warm. Until it is paid,
    // what this process has already swept stands in for it - enough small scans
    // earn the spawn between them that no one of them would earn alone.
    let warm = POOL_WARM.load(std::sync::atomic::Ordering::Relaxed)
        || TOKENS_SWEPT.fetch_add(n, std::sync::atomic::Ordering::Relaxed) + n
            >= TOKENS_BEFORE_WARMING;
    let threshold =
        if warm { PARALLEL_SCAN_THRESHOLD_WARM } else { PARALLEL_SCAN_THRESHOLD_COLD };
    if n < threshold || cores <= 1 {
        let mut advanced = 0u64;
        let got: Vec<StartMatch> = (0..n)
            .map(|i| {
                if worth_attempting(opens, toks[i].kind) {
                    attempt_at(pattern, input, toks, n, i, absent, regs, fields, &mut advanced)
                } else {
                    None
                }
            })
            .collect();
        count_starts(opens, toks, 0, &got, advanced);
        return got;
    }
    // Each position's attempt is independent, so they run across cores
    // through the work-stealing pool. The unanchored attempt has a
    // triangular cost profile -- a low index attempts a longer match than a
    // high index -- so the leaf is kept fine (several per core) and the
    // pool steals the expensive low-index leaves off the workers that drew
    // them, instead of one contiguous chunk piling them onto one worker.
    // Set before the dispatch rather than after it: the pool is up from the
    // moment this call asks for it, and a scan on another thread reading the
    // flag while this one waits should see what it will find.
    POOL_WARM.store(true, std::sync::atomic::Ordering::Relaxed);

    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    let mut results: Vec<StartMatch> = (0..n).map(|_| None).collect();
    let min_leaf = n.div_ceil(cores * 8).max(64);
    let plan = JobPlan::new(0, n as u32)
        .with_leaf_shape(flynnel::LeafShape::PortCompute);
    for_each_chunk_indexed_min_leaf(&plan, &mut results, min_leaf, |start, slots| {
        let mut advanced = 0u64;
        for (j, slot) in slots.iter_mut().enumerate() {
            // Every slot arrives holding `None`, which is what an anchor no
            // attempt is made at answers, so one left alone is the same vector.
            let i = start + j;
            if worth_attempting(opens, toks[i].kind) {
                *slot = attempt_at(pattern, input, toks, n, i, absent, regs, fields, &mut advanced);
            }
        }
        count_starts(opens, toks, start, slots, advanced);
    });
    results
}

/// Whether an attempt anchored at a token of `kind` could match at all, given
/// the kind `opens` says the pattern must begin with.
///
/// One place, because the serial path, the parallel path and the counter all
/// ask it, and three spellings of one predicate come apart. `best_at` answers
/// `None` at a whitespace anchor and the selection pass only queries
/// significant positions, so an attempt there computes a value defined to be
/// absent that nothing reads; `opens` refuses a significant anchor whose token
/// the pattern's first atom cannot be.
///
/// `None` from `opens` is a pattern that does not force its first token, and it
/// admits every significant anchor - which is the sweep exactly as it was.
fn worth_attempting(opens: Option<crate::kind_route::OpeningKinds>, kind: TokenKind) -> bool {
    kind != TokenKind::Whitespace && opens.is_none_or(|set| set.admits(kind))
}

/// What a run of attempts reached, for the trace.
///
/// Handed over once a run rather than once an attempt, and read in a pass of
/// its own after the attempts rather than inside them: `counted` takes a lock,
/// and a lock per anchor over millions of anchors would price the watch above
/// the work it watches. Where rungs are not being kept this is one atomic load
/// a leaf and nothing else, so the attempts themselves carry no counter.
fn count_starts(
    opens: Option<crate::kind_route::OpeningKinds>,
    toks: &[Token],
    start: usize,
    got: &[StartMatch],
    advanced: u64,
) {
    if !crate::trace::keeping() {
        return;
    }
    let (mut skipped, mut refused, mut matched) = (0u64, 0u64, 0u64);
    for (j, slot) in got.iter().enumerate() {
        // Both counts read `worth_attempting`, which is the predicate the
        // attempts themselves are made under, so a count is of what happened
        // rather than of what some other rule would have done.
        let kind = toks[start + j].kind;
        if kind == TokenKind::Whitespace {
            skipped += 1;
        } else if !worth_attempting(opens, kind) {
            refused += 1;
        }
        matched += u64::from(slot.is_some());
    }
    let walked = u64::try_from(got.len()).expect("a leaf shorter than the counter's width");
    crate::trace::counted("the sweep: starts the fan-out walks", walked);
    crate::trace::counted("the sweep: starts skipped as whitespace", skipped);
    // A significant anchor whose token the pattern's first atom cannot be. The
    // sweep costs the same to enter an attempt that dies as one that matches,
    // so these are attempts whose whole cost is gone rather than shortened.
    crate::trace::counted("the sweep: starts the opening kind refuses", refused);
    crate::trace::counted("the sweep: attempts made", walked - skipped - refused);
    // The two ways an attempt fails, which a duration reads as one. An attempt
    // whose state set came back empty died on the way in and a cheaper
    // rejection would have caught it; one that came back with states and still
    // answered nothing did the work and was refused by the length filter, and
    // no prefilter reaches that.
    crate::trace::counted("the sweep: attempts whose state set survived", advanced);
    crate::trace::counted("the sweep: attempts that match", matched);
    report_lent();
}

/// The end position of the longest match anchored at token index `i`, or
/// `None`.
#[allow(clippy::too_many_arguments)]
fn attempt_at(
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    n: usize,
    i: usize,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
    advanced: &mut u64,
) -> StartMatch {
    best_at(pattern, input, toks, n, i, absent, regs, fields, advanced, |best| best.pos)
}

/// The longest match anchored at token index `i`, read through `read` from
/// the state it ends in, or `None` when nothing matches there. Whitespace
/// anchors return `None`: the selection pass only queries significant
/// positions (the output of `skip_ws`).
#[allow(clippy::too_many_arguments)]
fn best_at<R>(
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    n: usize,
    i: usize,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
    advanced: &mut u64,
    read: impl FnOnce(&State) -> R,
) -> Option<R> {
    if toks[i].kind == TokenKind::Whitespace {
        return None;
    }
    // The state vector, carried from the last attempt rather than built again.
    //
    // `advance` takes a vector by value and gives one back, and it is the same
    // allocation: the atom arm collects `State` into `State`, which reuses the
    // buffer it consumed. So one vector already serves a whole attempt - it was
    // just being built and dropped at each, which sampling put among the
    // largest allocating sites in a scan.
    //
    // Keeping it also keeps its capacity, so the doublings a fan-out pays are
    // paid by the first few attempts instead of by every one.
    //
    // Taking it out leaves an empty vector behind, so an attempt that somehow
    // began inside another gets a fresh one rather than the outer attempt's
    // states, and whichever finishes last leaves a cleared vector for the next.
    let mut init = SCRATCH.with(|s| std::mem::take(&mut *s.borrow_mut()));
    init.clear();
    let lent = init.capacity();
    init.push(State::start(i));
    let mut results = advance(pattern, input, toks, n, init, absent, regs, fields);
    note_lent(lent, results.capacity());
    // Whether the state set survived at all, carried in a register the caller
    // owns and handed to the trace once a leaf. An attempt that comes back with
    // nothing died on the way in; one that comes back with states and still
    // answers `None` did the work and was refused by the `pos > i` filter
    // below. The two are the same row in a duration and different questions.
    *advanced += u64::from(!results.is_empty());
    // Best rank first, then longest. Rank orders the leftmost-first branches
    // that survived the whole pattern, so the preference is applied after the
    // continuation has pruned rather than before it has been consulted; length
    // breaks ties, which is the greedy reading every quantifier here takes.
    let answer = results
        .iter()
        .min_by(|a, b| cmp_rank(a.rank_slice(), b.rank_slice()).then(b.pos.cmp(&a.pos)))
        .filter(|best| best.pos > i)
        .map(read);
    // Handed back emptied, so the next attempt on this thread pushes into a
    // buffer that is already there. Nothing read from it outlives this: `read`
    // takes a state by reference and returns a value of its own.
    results.clear();
    SCRATCH.with(|s| *s.borrow_mut() = results);
    answer
}

thread_local! {
    /// The state vector [`best_at`] lends to each attempt; see the note there.
    static SCRATCH: std::cell::RefCell<Vec<State>> =
        const { std::cell::RefCell::new(Vec::new()) };

    /// What the lent vector was worth, summed over the attempts this thread has
    /// made and reported once a leaf by [`count_starts`].
    ///
    /// Sampling the callers of a scan's allocations says the state vector
    /// growing is what is left to remove, and that its count ROSE as the
    /// attempts around it were made cheaper. Two capacities say why: the room
    /// the attempt was handed, and the room it gave back. A buffer that is
    /// carrying the work comes back holding what the fan-out reached, so the
    /// second runs far ahead of the first and settles after a few attempts; one
    /// that is only riding along comes back the size of a result set while the
    /// vectors doing the work were grown and dropped inside.
    ///
    /// Summed rather than counted per borrow, because a lock per anchor over
    /// millions of anchors would price the watch above the thing it watches -
    /// the same reason the sweep's own counters are folded a leaf at a time.
    ///
    /// A borrow is not an attempt and the count must not be read as one.
    /// [`best_at`] is reached from the sweep and again from the capture pass
    /// that resolves each span's registers, the thread keeps its tally between
    /// leaves, and the capture pass runs outside any leaf - so a borrow made
    /// there lands in whichever leaf reports next, possibly of a later scan.
    /// What survives that is the RATIO of the two capacities, which is a
    /// property of each borrow and not of how they were grouped.
    static LENT: std::cell::Cell<(u64, u64, u64)> = const { std::cell::Cell::new((0, 0, 0)) };

    /// What the quantifier fixpoint spends on vectors, summed the same way.
    ///
    /// [`unroll`] builds a frontier from empty at every iteration, and a second
    /// one where the body makes choices of its own. Sampling the callers of a
    /// scan's allocations puts this growth at 40% of what is left, so what
    /// decides whether carrying those buffers is worth anything is how many
    /// times the loop goes round: twice and a swap saves one vector, hundreds
    /// of times and it saves hundreds.
    static UNROLLED: std::cell::Cell<(u64, u64, u64)> = const { std::cell::Cell::new((0, 0, 0)) };

    /// How often a preference path is extended against how often one is read.
    ///
    /// Extending is an allocation - `with_branch` is the second largest site a
    /// scan allocates from - and a cons-list would make it a fixed node shared
    /// with the path it grew from, costing nothing to extend and keeping
    /// `State` a thin pointer wide. What it would cost is the reading:
    /// `cmp_rank` compares two slices today and would walk two lists instead.
    ///
    /// So the trade is one count against the other, and it is not obvious which
    /// way round it falls - a path is extended once a branch and read once a
    /// dedup, and nothing says which a scan does more of.
    ///
    /// The lengths ride along because they decide a different question. Holding
    /// a short path inside the state rather than on the heap would remove this
    /// site's allocations outright, which a cons-list would not, and what makes
    /// that possible or not is how long a path actually gets.
    ///
    /// The tail is counted in buckets rather than carried as a maximum, because
    /// the trace SUMS what it is handed: a maximum reported through it becomes
    /// the sum of each leaf's maximum, which is a number about how the work was
    /// divided and not about any path. Counting the extensions that exceed a
    /// length is a sum and survives that, and three bounds give the shape of
    /// the tail without one of them having to be chosen in advance.
    static RANKED: std::cell::Cell<(u64, u64, u64, u64, u64, u64)> =
        const { std::cell::Cell::new((0, 0, 0, 0, 0, 0)) };
}

thread_local! {
    /// How many entries a quantifier fixpoint's seen-map held when it
    /// finished, and how often it held more than a few.
    ///
    /// The map is built once per call to [`unroll`] and a call is an anchor,
    /// so its size is what decides whether a map is the right structure there
    /// at all: a handful of entries is a linear scan's territory, and a scan
    /// over a slice already in hand allocates nothing.
    ///
    /// Counted in buckets rather than as a maximum, for the reason the
    /// preference paths are: the trace SUMS what it is handed, so a maximum
    /// through it becomes the sum of every leaf's maximum, which describes how
    /// the work was divided and not any one call.
    static FIXPOINT_MAP: std::cell::Cell<(u64, u64, u64, u64, u64, u64, u64)> =
        const { std::cell::Cell::new((0, 0, 0, 0, 0, 0, 0)) };
}

/// Record that a preference path was extended to `len` bytes.
fn note_extended(len: usize) {
    RANKED.with(|r| {
        let (extended, compared, sum, over4, over16, over64) = r.get();
        r.set((
            extended + 1,
            compared,
            sum + len as u64,
            over4 + u64::from(len > 4),
            over16 + u64::from(len > 16),
            over64 + u64::from(len > 64),
        ));
    });
}

/// Record that two preference paths were compared.
fn note_compared() {
    RANKED.with(|r| {
        let (extended, compared, sum, over4, over16, over64) = r.get();
        r.set((extended, compared + 1, sum, over4, over16, over64));
    });
}

/// Record that an attempt was handed `given` slots and gave `back` slots.
fn note_lent(given: usize, back: usize) {
    LENT.with(|l| {
        let (attempts, sum_given, sum_back) = l.get();
        l.set((attempts + 1, sum_given + given as u64, sum_back + back as u64));
    });
}

/// Record that a quantifier fixpoint finished with `entries` in its seen-map.
fn note_fixpoint_map(entries: usize) {
    FIXPOINT_MAP.with(|m| {
        let (calls, sum, over4, over16, over64, over256, over1024) = m.get();
        m.set((
            calls + 1,
            sum + entries as u64,
            over4 + u64::from(entries > 4),
            over16 + u64::from(entries > 16),
            over64 + u64::from(entries > 64),
            over256 + u64::from(entries > 256),
            over1024 + u64::from(entries > 1024),
        ));
    });
}

/// Record one iteration of a quantifier fixpoint, which built a frontier of
/// `grown` slots and handed `marked` slots to the body.
fn note_unrolled(grown: usize, marked: usize) {
    UNROLLED.with(|u| {
        let (iterations, sum_grown, sum_marked) = u.get();
        u.set((iterations + 1, sum_grown + grown as u64, sum_marked + marked as u64));
    });
}

/// Hand the lending figures to the trace and zero them for the next leaf.
fn report_lent() {
    let (attempts, given, back) = LENT.with(|l| l.replace((0, 0, 0)));
    if attempts == 0 {
        return;
    }
    crate::trace::counted("the state vector: times it was borrowed", attempts);
    crate::trace::counted("the state vector: slots lent out", given);
    crate::trace::counted("the state vector: slots handed back", back);

    let (iterations, grown, marked) = UNROLLED.with(|u| u.replace((0, 0, 0)));
    if iterations == 0 {
        return;
    }
    crate::trace::counted("the quantifier fixpoint: iterations", iterations);
    crate::trace::counted("the quantifier fixpoint: slots its frontier grew to", grown);
    crate::trace::counted("the quantifier fixpoint: slots handed to the body", marked);

    let (calls, entries, over4, over16, over64, over256, over1024) =
        FIXPOINT_MAP.with(|m| m.replace((0, 0, 0, 0, 0, 0, 0)));
    if calls > 0 {
        crate::trace::counted("the fixpoint's seen-map: times one was built", calls);
        crate::trace::counted("the fixpoint's seen-map: entries they held", entries);
        crate::trace::counted("the fixpoint's seen-map: those past four entries", over4);
        crate::trace::counted("the fixpoint's seen-map: those past sixteen entries", over16);
        crate::trace::counted("the fixpoint's seen-map: those past sixty-four entries", over64);
        crate::trace::counted("the fixpoint's seen-map: those past two hundred and fifty-six", over256);
        crate::trace::counted("the fixpoint's seen-map: those past one thousand and twenty-four", over1024);
    }

    let (extended, compared, sum, over4, over16, over64) =
        RANKED.with(|r| r.replace((0, 0, 0, 0, 0, 0)));
    if extended == 0 && compared == 0 {
        return;
    }
    crate::trace::counted("the preference path: times one was extended", extended);
    crate::trace::counted("the preference path: times two were compared", compared);
    crate::trace::counted("the preference path: bytes those extensions produced", sum);
    crate::trace::counted("the preference path: extensions past four bytes", over4);
    crate::trace::counted("the preference path: extensions past sixteen bytes", over16);
    crate::trace::counted("the preference path: extensions past sixty-four bytes", over64);
}

/// Skip insignificant whitespace tokens, bounded by `end`.
fn skip_ws(toks: &[Token], mut pos: usize, end: usize) -> usize {
    while pos < end && toks[pos].kind == TokenKind::Whitespace {
        pos += 1;
    }
    pos
}

/// Advance the active state set by matching `pat`. `absent` carries the
/// guard literals a prefilter has proven are nowhere in the input, so a
/// guard requiring one fails with no positional scan.
#[allow(clippy::too_many_arguments)]
fn advance(
    pat: &Pattern,
    input: &[u8],
    toks: &[Token],
    end: usize,
    states: Vec<State>,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> Vec<State> {
    match pat {
        Pattern::Empty => states,
        Pattern::Atom(a) => states
            .into_iter()
            .filter_map(|s| match_atom(a, input, toks, end, &s, regs, fields))
            .collect(),
        Pattern::Concat(parts) => {
            // Every arm of this function maps the set it is given forward -
            // filtering it, walking it, or handing it on - so an empty set in
            // is an empty set out, whatever the pattern. A concatenation whose
            // set has emptied therefore has its answer already, and the parts
            // after it would each be dispatched to do nothing.
            //
            // It is worth the check because most anchors fail: 86% of the
            // attempts a scan makes die on the way in, and a pattern of several
            // parts dies at one of them rather than at the last.
            let mut acc = states;
            for p in parts {
                if acc.is_empty() {
                    break;
                }
                acc = advance(p, input, toks, end, acc, absent, regs, fields);
            }
            acc
        }
        Pattern::Alt(parts, mode) => {
            alt(parts, *mode, input, toks, end, states, absent, regs, fields)
        }
        // An optional is a repetition bounded at one, so it ranks the same way
        // rather than through a second rule.
        Pattern::Opt(p, g) => {
            repeat(p, (0, Some(1)), *g, input, toks, end, states, absent, regs, fields)
        }
        Pattern::Star(p, g) => star(p, *g, input, toks, end, states, absent, regs, fields),
        Pattern::Plus(p, g) => {
            let once = advance(p, input, toks, end, states, absent, regs, fields);
            star(p, *g, input, toks, end, once, absent, regs, fields)
        }
        Pattern::Repeat(p, m, n, g) => {
            repeat(p, (*m, *n), *g, input, toks, end, states, absent, regs, fields)
        }
        // Atomic keeps only the length the body itself preferred. Every
        // derivation is already ranked, so the cut is taking the best one and
        // dropping the rest - the alternatives never reach what follows, and
        // an atom that needed one of them finds nothing to take.
        //
        // Each start is cut on its own. Collapsing the whole set to one state
        // would instead pick a single winner across unrelated starting
        // positions, which is a different and wrong thing: a scan explores
        // several starts at once here.
        Pattern::Atomic(p) => {
            // A single start is its own cut: with no second start to keep
            // apart, cutting the set and cutting each start are one operation,
            // so the set goes through as it stands. A scan anchors one state at
            // a time, which is what makes this the usual arrival.
            if states.len() == 1 {
                let mut out = advance(p, input, toks, end, states, absent, regs, fields);
                // The cut keeps one derivation, and that derivation is already
                // in the vector the body returned: moving it to the front and
                // dropping the rest answers with the allocation the body made
                // rather than with a second one holding a copy of it.
                match best_index(&out) {
                    Some(at) => {
                        out.swap(0, at);
                        out.truncate(1);
                    }
                    None => out.clear(),
                }
                return out;
            }
            let mut kept = Vec::with_capacity(states.len());
            for s in states {
                let out = advance(p, input, toks, end, vec![s], absent, regs, fields);
                if let Some(best) =
                    out.into_iter().min_by(|a, b| cmp_rank(a.rank_slice(), b.rank_slice()))
                {
                    kept.push(best);
                }
            }
            dedup(kept)
        }
        Pattern::Bind(name, _, p) => bind(name, p, input, toks, end, states, absent, regs, fields),
        Pattern::Balanced(kind, p) => balanced(*kind, p, input, toks, end, states, absent, regs, fields),
        Pattern::Guard(lit, neg) => guard(lit, *neg, input, toks, end, states, absent),
        Pattern::Assert(inner, neg, look) => {
            assert_zero_width(inner, *neg, *look, input, toks, end, states, absent, regs, fields)
        }
        Pattern::Field(k, inner) => field_match(*k, inner, input, toks, end, states, absent, regs, fields),
        Pattern::Anchor(kind) => anchor(kind, input, toks, end, states, fields),
        Pattern::Within(atoms, k) => {
            // `out` is the step's answer and leaves with it, so it is built
            // here where the scratch is carried: `dedup` hands the caller
            // either this vector or one it builds from it.
            let mut out = Vec::new();
            let mut scratch = EDITS.with(|e| std::mem::take(&mut *e.borrow_mut()));
            for s in states {
                within_edits_of(
                    atoms,
                    *k,
                    input,
                    toks,
                    end,
                    &s,
                    regs,
                    fields,
                    &mut scratch,
                    &mut out,
                );
            }
            EDITS.with(|e| *e.borrow_mut() = scratch);
            dedup(out)
        }
    }
}

/// Append every run of tokens from `s` within `k` token edits of `atoms` to
/// `out`, as one state per run length, preferring fewer edits and then the
/// longer run.
///
/// The walk is the Levenshtein table with the atoms down one side and the
/// tokens along the other: a cell is one atom tested against one token, a
/// step down is an atom the run lacks, a step right a token it has extra, and
/// a step diagonal a match or a substitution. Only the band `k` wide either
/// side of the diagonal can hold a cost at or under `k`, so the work is the
/// atom count times `2k + 1` however long the input is.
///
/// Every alignment the last row admits is offered rather than one, because a
/// run that costs more can be the one the rest of the pattern needs - the
/// same reason an alternation offers every branch. The rank is the edit cost
/// then the run's shortfall, so a state that spent fewer edits, and among
/// those the one reaching furthest, is preferred.
///
/// Where two alignments cost the same the diagonal wins, so an atom that
/// could match the token beside it is recorded as having matched it. Several
/// minimum-cost alignments can exist and a register binds what the one taken
/// gave it.
#[allow(clippy::too_many_arguments)]
fn within_edits_of(
    atoms: &[crate::ast::EditAtom],
    k: u8,
    input: &[u8],
    toks: &[Token],
    end: usize,
    s: &State,
    regs: &[String],
    fields: Fields<'_>,
    scratch: &mut EditScratch,
    out: &mut Vec<State>,
) {
    let k = usize::from(k);
    let m = atoms.len();
    if m == 0 {
        return;
    }
    // The significant tokens the run could reach, at most `m + k` of them.
    let run = &mut scratch.run;
    run.clear();
    let mut p = skip_ws(toks, s.pos, end);
    while run.len() < m + k && p < end {
        run.push(p);
        p = skip_ws(toks, p + 1, end);
    }
    let n = run.len();
    // Cost, and the step that reached it, for every cell of the table. A cell
    // records where it came from rather than the alignment that reached it, so
    // the walk writes two words a cell where carrying the alignment forward
    // copied a vector of every atom's token into each one.
    let past = k + 1;
    let w = n + 1;
    let table = &mut scratch.table;
    table.clear();
    table.resize((m + 1) * w, (past, Step::Start));
    // The first row, where no atom has been consumed yet: reaching token `j`
    // costs the `j` tokens skipped to get there, and past `k` it is out of
    // budget.
    for (j, cell) in table.iter_mut().take(w).enumerate() {
        cell.0 = if j <= k { j } else { past };
    }
    for i in 1..=m {
        let lo = i.saturating_sub(k);
        let hi = (i + k).min(n);
        if lo == 0 {
            table[i * w] = (i, Step::AtomDeleted);
        }
        for j in lo.max(1)..=hi {
            let matched = atom_test(&atoms[i - 1].atom, input, toks, run[j - 1], s, regs, fields);
            let diagonal = table[(i - 1) * w + j - 1].0 + usize::from(!matched);
            let atom_deleted = table[(i - 1) * w + j].0 + 1;
            let token_extra = table[i * w + j - 1].0 + 1;
            let cost = diagonal.min(atom_deleted).min(token_extra);
            if cost > k {
                continue;
            }
            // The diagonal wins a tie, so an atom that matched its token is
            // recorded as having matched it rather than deleted beside an
            // equally cheap insertion.
            let step = if cost == diagonal {
                if matched { Step::Matched } else { Step::Substituted }
            } else if cost == atom_deleted {
                Step::AtomDeleted
            } else {
                Step::TokenExtra
            };
            table[i * w + j] = (cost, step);
        }
    }
    // The run must hold at least one token, so the empty alignment is never
    // offered even where every atom could be deleted.
    let taken = &mut scratch.taken;
    taken.clear();
    taken.resize(m, None);
    for j in 1..=n {
        let cost = table[m * w + j].0;
        if cost > k {
            continue;
        }
        // The path back to row zero is the alignment: each step says which cell
        // it came from, and a step that matched names the token its atom took.
        taken.iter_mut().for_each(|t| *t = None);
        let (mut i, mut col) = (m, j);
        while i > 0 {
            match table[i * w + col].1 {
                Step::Matched => {
                    taken[i - 1] = Some(run[col - 1]);
                    i -= 1;
                    col -= 1;
                }
                Step::Substituted => {
                    i -= 1;
                    col -= 1;
                }
                Step::AtomDeleted => i -= 1,
                Step::TokenExtra => col -= 1,
                // Row zero alone starts an alignment, and every cell a path
                // reaches was written with the step that reached it.
                Step::Start => {
                    debug_assert!(false, "an alignment stepped through an unreached cell");
                    break;
                }
            }
        }
        let mut env = s.env.clone();
        for (e, at) in atoms.iter().zip(taken.iter()) {
            if let (Some((name, _)), Some(at)) = (&e.bind, at)
                && let Some(id) = reg_id(regs, name)
            {
                let span = (toks[*at].start(), toks[*at].end());
                Rc::make_mut(&mut env).insert(id, span);
            }
        }
        // Fewer edits first, then the longer run: both oriented so the
        // preferred option sorts lower, as every other rank entry is.
        let mut rank = s.rank_slice().to_vec();
        rank.push(u8::try_from(cost).unwrap_or(u8::MAX));
        rank.push(u8::try_from(n - j).unwrap_or(u8::MAX));
        out.push(State {
            pos: run[j - 1] + 1,
            env,
            rank: Rank::of(&rank),
            hist: s.hist.clone(),
        });
    }
}

/// Which cell an alignment stepped from, so the tokens an endpoint's atoms took
/// are walked back out of the cost table.
#[derive(Clone, Copy)]
enum Step {
    /// Row zero, where an alignment begins, and the state of a cell no
    /// alignment reached.
    Start,
    /// From the cell up and left, the atom matching its token.
    Matched,
    /// From the cell up and left, the atom standing in for its token.
    Substituted,
    /// From the cell above: an atom the run lacks.
    AtomDeleted,
    /// From the cell to the left: a token the run has spare.
    TokenExtra,
}

/// The buffers the edit-distance walk runs in: the run of tokens it reads, its
/// cost table, and the tokens one alignment took. The walk runs once per state,
/// so each of these is an allocation a state otherwise.
///
/// Each field is cleared where the walk reaches it, so a scratch carries no
/// meaning between uses and one arriving full is the same as one arriving
/// empty.
#[derive(Default)]
struct EditScratch {
    run: Vec<usize>,
    table: Vec<(usize, Step)>,
    taken: Vec<Option<usize>>,
}

impl EditScratch {
    /// An empty scratch, holding no capacity until a walk asks for some.
    const fn new() -> Self {
        Self { run: Vec::new(), table: Vec::new(), taken: Vec::new() }
    }
}

thread_local! {
    /// The scratch an edit-distance walk runs in, held past the call that
    /// filled it; see [`EditScratch`].
    ///
    /// A `Within` pattern builds one scratch per step, and a scan steps once
    /// per anchor, so its three vectors reached the heap once an anchor each -
    /// the largest single source of allocation a scan has. Held here they
    /// reach a capacity the corpus needs and stop asking for more.
    ///
    /// Taking it out leaves an empty scratch behind, so a `Within` reached
    /// from inside another gets its own rather than the outer walk's table.
    static EDITS: std::cell::RefCell<EditScratch> =
        const { std::cell::RefCell::new(EditScratch::new()) };
}

/// Filter the reachable set to states whose next significant token sits at a
/// boundary in a precomputed axis field. Zero-width: a surviving state keeps
/// its position (the anchor consumes no token), so a following atom matches
/// from the same place. `@seam` keeps a state whose token starts at a seam
/// cut - the predictive-segmentation boundary the set engine built once.
/// Is token `p` the first significant token of its line, or of the input?
///
/// Significance skips whitespace, so an indented token still leads its
/// line. Scans back from the token start over whitespace only: reaching
/// the start of input, or a newline, means nothing precedes it on this
/// line.
fn leads_its_line(input: &[u8], start: usize) -> bool {
    let mut i = start;
    while i > 0 {
        let b = input[i - 1];
        if b == b'\n' {
            return true;
        }
        if !b.is_ascii_whitespace() {
            return false;
        }
        i -= 1;
    }
    true
}

/// Is token `p` the last significant token of its line, or of the input?
/// The mirror of [`leads_its_line`], scanning forward from the token end.
fn ends_its_line(input: &[u8], end: usize) -> bool {
    let mut i = end;
    while i < input.len() {
        let b = input[i];
        if b == b'\n' {
            return true;
        }
        if !b.is_ascii_whitespace() {
            return false;
        }
        i += 1;
    }
    true
}

/// Whether a positional anchor holds at token index `p`.
///
/// These four read the input either side of a token, so they are functions of
/// the tokens, the bytes and the position, with no precomputed field behind
/// them. Both engines call this, which is what keeps them from drifting: the
/// single-pass engine evaluates it as an epsilon instruction and the set
/// engine as a filter over the reachable set, and neither has its own copy of
/// the predicate.
///
/// The property anchors (`@seam` and the rest) are not here. Each reads a
/// field built once per scan, which only the set engine carries.
pub(crate) fn positional_anchor_holds(
    kind: &crate::ast::AnchorKind,
    input: &[u8],
    toks: &[Token],
    p: usize,
) -> bool {
    let t = &toks[p];
    anchor_holds_at(
        kind,
        input,
        t.start(),
        t.end(),
        !toks[..p].iter().any(Token::is_significant),
        !toks[p + 1..].iter().any(Token::is_significant),
    )
}

/// [`positional_anchor_holds`] of a token given by its byte span alone, with
/// whether it is the first significant token of the input and whether it is
/// the last: what a stream that holds only the significant tokens can say.
pub(crate) fn anchor_holds_at(
    kind: &crate::ast::AnchorKind,
    input: &[u8],
    start: usize,
    end: usize,
    first: bool,
    last: bool,
) -> bool {
    use crate::ast::AnchorKind;
    match kind {
        AnchorKind::LineStart => leads_its_line(input, start),
        AnchorKind::LineEnd => ends_its_line(input, end),
        AnchorKind::InputStart => first,
        AnchorKind::InputEnd => last,
        // `\G` constrains which match a scan may keep, not which tokens a
        // match may take, so it is an epsilon here and the non-overlapping
        // selection applies it. A match attempt in isolation has no previous
        // match to abut.
        AnchorKind::Resume => true,
        // `\K` moves the reported start rather than testing the position, and
        // the parser refuses it on any pattern that reaches the set engine,
        // so it consumes nothing and asserts nothing wherever it is seen.
        AnchorKind::ResetStart => true,
        _ => false,
    }
}

/// Whether the pair field's `reading` at the token starting at byte `start`
/// stands in `cmp` to `level`. Strain is read at the unit holding that byte;
/// the binding of the cut before a unit is read only where a unit starts
/// there. A percentile is of this input's readings at the grain.
fn gravity_holds(
    r: &crate::gravity::Readings,
    reading: crate::ast::GravityReading,
    cmp: crate::ast::Cmp,
    level: crate::ast::Level,
    start: usize,
) -> bool {
    use crate::ast::{GravityReading, Level};
    let unit = match reading {
        GravityReading::Strain => r.unit_at(start),
        GravityReading::Bound => r.unit_starting_at(start),
    };
    let Some(u) = unit else { return false };
    let value = match reading {
        GravityReading::Strain => r.strain[u],
        GravityReading::Bound => r.binding[u],
    };
    let bar = match (level, reading) {
        (Level::Bits(v), _) => Some(v.get()),
        (Level::Percentile(q), GravityReading::Strain) => r.strain_percentile(q.get()).map(f64::from),
        (Level::Percentile(q), GravityReading::Bound) => r.binding_percentile(q.get()).map(f64::from),
    };
    bar.is_some_and(|b| cmp.holds(f64::from(value).total_cmp(&b)))
}

fn anchor(
    kind: &crate::ast::AnchorKind,
    input: &[u8],
    toks: &[Token],
    end: usize,
    states: Vec<State>,
    fields: Fields<'_>,
) -> Vec<State> {
    use crate::ast::AnchorKind;
    states
        .into_iter()
        .filter(|s| {
            let p = skip_ws(toks, s.pos, end);
            if p >= end {
                return false;
            }
            match kind {
                // The positional four share one predicate with the
                // single-pass engine, so the two cannot drift.
                AnchorKind::LineStart
                | AnchorKind::LineEnd
                | AnchorKind::InputStart
                | AnchorKind::InputEnd
                | AnchorKind::Resume => positional_anchor_holds(kind, input, toks, p),
                // The strong-cut offsets are ascending, so a binary search
                // resolves membership in O(log n).
                // Each grain reads its own cut set, and only the grains a
                // pattern names are segmented at all. The cuts are ascending
                // byte offsets whatever stream produced them, so one lookup
                // shape serves all three.
                AnchorKind::Seam(grain) => {
                    let cuts = match grain {
                        crate::ast::Grain::Byte => fields.seam_cuts,
                        crate::ast::Grain::Token => fields.seam_token,
                        crate::ast::Grain::Super => fields.seam_super,
                    };
                    cuts.is_some_and(|c| c.binary_search(&toks[p].start()).is_ok())
                }
                // A reading of the pair field at the unit this token's start
                // falls in, held against the input's own percentile or bits.
                AnchorKind::Gravity(reading, grain, cmp, level) => fields.gravity[grain_slot(*grain)]
                    .is_some_and(|r| gravity_holds(r, *reading, *cmp, *level, toks[p].start())),
                // The unit's type is the example's, or shares its placed class.
                AnchorKind::Kin(grain, example) => fields.gravity[grain_slot(*grain)].is_some_and(|r| {
                    let named = fields
                        .kin
                        .iter()
                        .find(|(g, x, _)| g == grain && x == example)
                        .and_then(|(_, _, t)| *t);
                    match (named, r.unit_at(toks[p].start())) {
                        (Some(want), Some(unit)) => r.kin(r.type_of(unit), want),
                        (None, _) | (_, None) => false,
                    }
                }),
                // The token is at least `min` brackets deep.
                AnchorKind::Nested(min) => fields.depths.is_some_and(|d| d[p] >= *min),
                // The token's span contains a contested (vantage-dependent)
                // point: the first contested offset at or after the token start
                // still falls before the token end.
                AnchorKind::Ambiguous(grain) => {
                    let contested = match grain {
                        crate::ast::Grain::Byte => fields.obs_contested,
                        crate::ast::Grain::Token => fields.obs_token,
                        crate::ast::Grain::Super => fields.obs_super,
                    };
                    contested.is_some_and(|c| {
                        let i = c.partition_point(|&x| x < toks[p].start());
                        c.get(i).is_some_and(|&x| x < toks[p].end())
                    })
                }
                // Echo-axis anchors: the frames are index-aligned with the
                // token stream, so each is a rung lookup and an O(1) index.
                AnchorKind::Novel => echo_at(fields.echo, crate::orbit::OrbitGroup::Identity)
                    .is_some_and(|f| f.frames.get(p).is_some_and(crate::echo::EchoFrame::novel)),
                AnchorKind::Echoed => echo_at(fields.echo, crate::orbit::OrbitGroup::Identity)
                    .is_some_and(|f| f.frames.get(p).is_some_and(crate::echo::EchoFrame::echoed)),
                AnchorKind::Echo(pred, group) => echo_at(fields.echo, *group)
                    .and_then(|f| f.frames.get(p))
                    .is_some_and(|fr| echo_pred_holds(pred, fr)),
                // Which way this timestamp stands against the one before it,
                // read once per scan into a table the anchor indexes.
                AnchorKind::Order(want) => fields
                    .order
                    .and_then(|o| o.get(p).copied())
                    .flatten()
                    .is_some_and(|forward| forward == (*want == crate::ast::TimeOrder::Asc)),
                // Whether the line this token stands on has a template rarer
                // than the cut, over templates mined once per scan.
                AnchorKind::Rare(cut) => {
                    fields.templates.is_some_and(|m| m.token_is_rare(p, *cut))
                }
                // Whether this token's content occurs in the second input,
                // from a table keyed once per scan at the anchor's rung; a
                // token the echo axis does not key holds for neither reading.
                AnchorKind::Joined { other, recurs, group } => fields
                    .joins
                    .iter()
                    .find(|j| std::sync::Arc::ptr_eq(&j.other, other) && j.group == *group)
                    .and_then(|j| j.recurs.get(p).copied().flatten())
                    .is_some_and(|in_other| in_other == *recurs),
                // The parser refuses `\K` on any pattern that reaches this
                // engine, since there is no match-start slot here to move.
                AnchorKind::ResetStart => true,
                // The upper-grain window: is this token where a construct
                // begins, and what is the construct it sits in. Both are O(1)
                // lookups into a table built once for the scan.
                AnchorKind::SuperStart => fields.supers.is_some_and(|s| s.starts_unit(p)),
                AnchorKind::SuperRole(want) => {
                    fields.supers.is_some_and(|s| s.unit_of(p).is_some_and(|u| u.role == *want))
                }
                // The token's phase of the dominant token-kind period, read
                // from the related context; a stream with no period holds
                // no phase.
                AnchorKind::Phase(k) => {
                    fields.relation.is_some_and(|r| r.phase_of.get(p) == Some(k))
                }
                // The token's column of a named period, where it is live.
                AnchorKind::PhaseIn(k, period) => fields.bands.is_some_and(|b| b.at(p, *k, *period)),
            }
        })
        .collect()
}

fn match_atom(
    a: &Atom,
    input: &[u8],
    toks: &[Token],
    end: usize,
    s: &State,
    regs: &[String],
    fields: Fields<'_>,
) -> Option<State> {
    // Whitespace is insignificant between atoms, so it is skipped before
    // matching - EXCEPT when the atom itself asks for whitespace (`\S`, or
    // the `\s` byte class), which must see the token the skip would jump
    // over.
    let wants_ws = matches!(
        a,
        Atom::Kind(TokenKind::Whitespace) | Atom::Byte(crate::ast::ByteClass::Space)
    );
    let p = if wants_ws { s.pos } else { skip_ws(toks, s.pos, end) };
    if p >= end {
        return None;
    }
    let ok = atom_test(a, input, toks, p, s, regs, fields);
    if ok {
        Some(State { pos: p + 1, env: s.env.clone(), rank: s.rank.clone(), hist: s.hist.clone() })
    } else {
        None
    }
}

/// Whether the token at index `p` satisfies an atom, with no position
/// advance.
///
/// Separated from [`match_atom`] so a class can test its members against the
/// same token: membership recurses here, and every atom the language has is a
/// possible member for free.
fn atom_test(
    a: &Atom,
    input: &[u8],
    toks: &[Token],
    p: usize,
    s: &State,
    regs: &[String],
    fields: Fields<'_>,
) -> bool {
    let t = &toks[p];
    let txt = text(input, t);
    match a {
        Atom::Kind(k) => t.kind == *k,
        Atom::Any => true,
        Atom::Literal(lit, group) => register_eq_matches(lit.as_bytes(), txt, *group),
        Atom::RegisterEq(name, group) => reg_id(regs, name)
            .and_then(|id| s.env.get(&id))
            .is_some_and(|&(a, b)| register_eq_matches(&input[a..b], txt, *group)),
        // The unit the bound value starts in and this token's unit, at the
        // atom's grain, share a type or a placed gravity class.
        Atom::RegisterKin(name, grain) => reg_id(regs, name).and_then(|id| s.env.get(&id)).is_some_and(|&(a, _)| {
            fields.gravity[grain_slot(*grain)].is_some_and(|r| match (r.unit_at(a), r.unit_at(t.start())) {
                (Some(bound), Some(here)) => r.kin(r.type_of(bound), r.type_of(here)),
                (None, _) | (_, None) => false,
            })
        }),
        Atom::RegisterRelated(name, relation) => reg_id(regs, name)
            .and_then(|id| s.env.get(&id))
            .is_some_and(|&(a, b)| relation.related(&input[a..b], txt)),
        Atom::LiteralWithin(lit, k, group) => within_edits(lit.as_bytes(), txt, *k, *group),
        Atom::RegisterWithin(name, k, group) => reg_id(regs, name)
            .and_then(|id| s.env.get(&id))
            .is_some_and(|&(a, b)| within_edits(&input[a..b], txt, *k, *group)),
        Atom::Byte(bc) => byte_class_matches(*bc, txt),
        Atom::BytePattern(bp) => bp.matches_whole(txt),
        Atom::Spectral(pred) => spectral_pred_matches(pred, fields.spectral, t.start(), t.end()),
        Atom::Magnitude(pred) => magnitude_matches(pred, toks, p, txt, s, regs, fields),
        Atom::KindMag(k, pred) => {
            t.kind == *k && magnitude_matches(pred, toks, p, txt, s, regs, fields)
        }
        Atom::KindPred(k, pred) => t.kind == *k && pred.matches_as(t.kind, txt, || fields.clock),
        Atom::Since(op, signed, name) => {
            t.kind == crate::token::TokenKind::Timestamp
                && reg_id(regs, name).and_then(|id| s.env.get(&id)).is_some_and(|&(a, b)| {
                    crate::typed::since_holds(*op, signed, &input[a..b], txt, fields.clock)
                })
        }
        Atom::Class(c) => {
            let hit = c.any.iter().any(|m| atom_test(m, input, toks, p, s, regs, fields))
                && (c.all.is_empty()
                    || c.all.iter().any(|m| atom_test(m, input, toks, p, s, regs, fields)))
                && !c.none.iter().any(|m| atom_test(m, input, toks, p, s, regs, fields));
            hit != c.negated
        }
    }
}

/// Whether the token at `p` satisfies a magnitude predicate, absolute or
/// relative to the context the predicate names.
fn magnitude_matches(
    pred: &crate::ast::MagPred,
    toks: &[Token],
    p: usize,
    txt: &[u8],
    s: &State,
    regs: &[String],
    fields: Fields<'_>,
) -> bool {
    let mag = crate::magnitude::token_magnitude(toks[p].kind, txt);
    let context = pred.scope().and_then(|scope| magnitude_context(scope, toks, p, s, regs, fields));
    pred.matches(mag, context)
}

/// The magnitude fold a relative predicate at token `p` compares against:
/// the rolling window before the token, one of its relation-admitted
/// contexts, or the value history of the key a register holds. `None` when
/// the field was not built or the token has no such context.
fn magnitude_context<'a>(
    scope: &crate::ast::Scope,
    toks: &[Token],
    p: usize,
    s: &State,
    regs: &[String],
    fields: Fields<'a>,
) -> Option<&'a crate::profile::MagnitudeProfile> {
    use crate::ast::Scope;
    match scope {
        // The window before the token is the window's fold at the
        // significant token before it.
        Scope::Window => {
            let field = fields.context?;
            let prev = (0..p).rev().find(|&j| toks[j].is_significant())?;
            Some(&field.at_token.get(prev)?.magnitude)
        }
        Scope::Phase => Some(&fields.relation?.at_token.get(p)?.phase.magnitude),
        Scope::Regime => Some(&fields.relation?.at_token.get(p)?.regime.magnitude),
        Scope::Echo => Some(&fields.relation?.at_token.get(p)?.echoing.magnitude),
        Scope::Enclosing => Some(&fields.relation?.at_token.get(p)?.enclosing.magnitude),
        // The register holds a span; the token starting there is the key
        // whose value history the predicate reads.
        Scope::Key(name) => {
            let field = fields.relation?;
            let id = reg_id(regs, name)?;
            let &(start, _) = s.env.get(&id)?;
            let key = toks.partition_point(|t| t.start() < start);
            Some(&field.value_history.get(key)?.magnitude)
        }
    }
}

/// Evaluate a spectral predicate against the pooled field signature of the
/// token span `[start, end)`. A pattern carrying a spectral atom always
/// builds the field (see `scan_set_reachability`); the `None` guard simply
/// declines to match when no field was built.
/// A second input a join anchor reads, keyed at one rung: per token of the
/// scanned input, whether its key occurs anywhere in the other input, and
/// `None` for a token the echo axis does not key.
pub(crate) struct Join {
    other: std::sync::Arc<crate::ast::OtherInput>,
    group: crate::orbit::OrbitGroup,
    recurs: Vec<Option<bool>>,
}

impl Join {
    /// Key the other input's tokens at `group` once, then read every token
    /// of this input against them.
    fn build(
        other: std::sync::Arc<crate::ast::OtherInput>,
        group: crate::orbit::OrbitGroup,
        toks: &[Token],
        input: &[u8],
    ) -> Join {
        let keys: std::collections::HashSet<String> = other
            .tokens
            .iter()
            .filter(|t| crate::echo::keyed_kind(t.kind))
            .map(|t| crate::orbit::canonical(&other.bytes[t.span()], group))
            .collect();
        let recurs = toks
            .iter()
            .map(|t| {
                crate::echo::keyed_kind(t.kind)
                    .then(|| keys.contains(&crate::orbit::canonical(&input[t.span()], group)))
            })
            .collect();
        Join { other, group, recurs }
    }
}

/// Which way each timestamp token stands against the timestamp before it:
/// `Some(true)` at or after it, `Some(false)` before it, and `None` for a
/// token that is no timestamp, one whose text does not read as an instant,
/// one the clock cannot place, and the first timestamp of an input, which
/// has nothing to stand against.
pub(crate) fn timestamp_order(
    toks: &[Token],
    input: &[u8],
    clock: crate::typed::Clock,
) -> Vec<Option<bool>> {
    let mut out = vec![None; toks.len()];
    let mut prev: Option<(i64, u32)> = None;
    for (i, t) in toks.iter().enumerate() {
        if t.kind != crate::token::TokenKind::Timestamp {
            continue;
        }
        let text = String::from_utf8_lossy(&input[t.span()]);
        let Some(here) = crate::typed::parse_civil(&text).and_then(|c| c.epoch(clock)) else {
            continue;
        };
        if let Some(before) = prev {
            out[i] = Some(here >= before);
        }
        prev = Some(here);
    }
    out
}

/// The recurrence field counted at `group`, or `None` where the pattern
/// names no anchor at that rung.
fn echo_at(
    fields: &[(crate::orbit::OrbitGroup, crate::echo::EchoField)],
    group: crate::orbit::OrbitGroup,
) -> Option<&crate::echo::EchoField> {
    fields.iter().find(|(g, _)| *g == group).map(|(_, f)| f)
}

/// Whether one token's echo frame satisfies a reading of the axis. An
/// unkeyed token carries no recurrence and satisfies none of them.
fn echo_pred_holds(pred: &crate::ast::EchoPred, fr: &crate::echo::EchoFrame) -> bool {
    use crate::ast::EchoPred;
    if !fr.keyed {
        return false;
    }
    match pred {
        EchoPred::Count(op, k) => op.holds(fr.count.cmp(k)),
        EchoPred::Nth(op, k) => {
            // A negative index counts back from the last occurrence, so -1 is
            // the last and -2 the one before it.
            let want = if *k < 0 {
                let from_end = k.unsigned_abs();
                if from_end > fr.count {
                    return false;
                }
                fr.count - from_end + 1
            } else {
                k.unsigned_abs()
            };
            op.holds(fr.nth.cmp(&want))
        }
        // A frame with no period carries zero, which no regular recurrence can
        // read as its own, so the test for having one is the test for it being
        // above zero.
        EchoPred::Period => fr.period > 0.0,
        EchoPred::PeriodAt(op, k) => {
            fr.period > 0.0 && op.holds((fr.period.round() as i64).cmp(&i64::from(*k)))
        }
    }
}

fn spectral_pred_matches(
    pred: &crate::ast::SpectralPred,
    field: Option<&SpectralField>,
    start: usize,
    end: usize,
) -> bool {
    use crate::ast::{SpecTexture, SpectralPred};
    use crate::spectral::Texture;
    let Some(f) = field else { return false };
    f.assert_carries(crate::spectral::Needs::of(pred), "a spectral atom");
    // Pooled inside the arms that read a frame rather than before the match.
    // Pooling sums a band a frame across the token's span, and the onset reads
    // the change-points instead, so taking it first hands one predicate a
    // frame it discards at every anchor it is asked about.
    match pred {
        SpectralPred::Onset => f.boundary_in(start, end),
        SpectralPred::EntropyGe(p) => f.signature(start, end).entropy * 100.0 >= f32::from(*p),
        SpectralPred::EntropyLe(p) => f.signature(start, end).entropy * 100.0 <= f32::from(*p),
        SpectralPred::PeriodEq(n) => f.signature(start, end).period == *n,
        SpectralPred::PeriodAny => f.signature(start, end).period != 0,
        SpectralPred::Texture(want) => matches!(
            (want, crate::spectral::texture_of(&f.signature(start, end))),
            (SpecTexture::Prose, Texture::Prose)
                | (SpecTexture::Code, Texture::Code)
                | (SpecTexture::Math, Texture::Math)
                | (SpecTexture::Data, Texture::Data)
        ),
    }
}

/// Whether a token `txt` matches a register's bound span `bound` under an
/// orbit `group`. The Identity orbit is a byte-for-byte compare (the exact-
/// repeat backreference, kept bit-identical to the original behavior); any
/// other group compares the two spans' canonical orbit representatives, so
/// the reference matches every token in the bound token's orbit (a fuzzy
/// backreference). Shared by both engines so the compare lives in one place.
pub(crate) fn register_eq_matches(bound: &[u8], txt: &[u8], group: crate::orbit::OrbitGroup) -> bool {
    use crate::orbit::{OrbitGroup, canonical};
    match group {
        OrbitGroup::Identity => bound == txt,
        g => canonical(bound, g) == canonical(txt, g),
    }
}

/// Whether `bound` and `txt` lie within `k` edits of each other, over the
/// two as `group` canonicalizes them.
pub(crate) fn within_edits(bound: &[u8], txt: &[u8], k: u8, group: crate::orbit::OrbitGroup) -> bool {
    use crate::orbit::{OrbitGroup, canonical};
    match group {
        OrbitGroup::Identity => crate::edit::within(bound, txt, k),
        g => crate::edit::within(canonical(bound, g).as_bytes(), canonical(txt, g).as_bytes(), k),
    }
}

pub(crate) fn byte_class_matches(bc: ByteClass, txt: &[u8]) -> bool {
    if txt.is_empty() {
        return false;
    }
    match bc {
        ByteClass::Digit => txt.iter().all(u8::is_ascii_digit),
        ByteClass::Word => txt.iter().all(|b| *b == b'_' || b.is_ascii_alphanumeric()),
        ByteClass::Space => txt.iter().all(u8::is_ascii_whitespace),
        ByteClass::Hex => txt.iter().all(u8::is_ascii_hexdigit),
        ByteClass::Alpha => txt.iter().all(u8::is_ascii_alphabetic),
        ByteClass::Upper => txt.iter().all(u8::is_ascii_uppercase),
        ByteClass::Lower => txt.iter().all(u8::is_ascii_lowercase),
    }
}

/// Alternation, per [`crate::ast::AltMode`].
///
/// `Committed` takes the first branch that yields any state and never consults
/// what follows, so a branch the continuation later contradicts kills the
/// pattern. `Longest` unions every branch, leaving the choice to the final
/// longest-match selection. `First` also unions, but tags each branch's states
/// with their index, so the selection prefers the earliest branch that
/// survives the continuation - the fallback regex gives and commitment does
/// not.
#[allow(clippy::too_many_arguments)]
fn alt(
    parts: &[Pattern],
    mode: crate::ast::AltMode,
    input: &[u8],
    toks: &[Token],
    end: usize,
    mut states: Vec<State>,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> Vec<State> {
    use crate::ast::AltMode;
    // The last branch to be tried takes the set rather than a copy of it: no
    // branch after it needs one, and a copy left standing holds a second
    // reference to every register map its results carry, which makes the next
    // write to one copy a map it could have written in place.
    let last = parts.len().saturating_sub(1);
    match mode {
        AltMode::Committed => {
            for (i, p) in parts.iter().enumerate() {
                let taken =
                    if i == last { std::mem::take(&mut states) } else { states.clone() };
                let out = advance(p, input, toks, end, taken, absent, regs, fields);
                if !out.is_empty() {
                    return dedup(out);
                }
            }
            Vec::new()
        }
        AltMode::Longest => {
            let mut out = Vec::new();
            for (i, p) in parts.iter().enumerate() {
                let taken =
                    if i == last { std::mem::take(&mut states) } else { states.clone() };
                out.extend(advance(p, input, toks, end, taken, absent, regs, fields));
            }
            dedup(out)
        }
        AltMode::First => {
            let mut out = Vec::new();
            for (i, p) in parts.iter().enumerate() {
                // Saturating at 255 only collapses the preference between
                // branch 255 and beyond, which no readable pattern reaches.
                let branch = u8::try_from(i).unwrap_or(u8::MAX);
                let tagged: Vec<State> = states.iter().map(|s| s.with_branch(branch)).collect();
                out.extend(advance(p, input, toks, end, tagged, absent, regs, fields));
            }
            // Branches were walked in order, so the first arrival at a given
            // (pos, env) came from the earliest branch and carries the best
            // rank; `dedup` keeps the first, which is that one.
            dedup(out)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn star(
    p: &Pattern,
    g: Greed,
    input: &[u8],
    toks: &[Token],
    end: usize,
    states: Vec<State>,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> Vec<State> {
    unroll(p, g, None, input, toks, end, states, absent, regs, fields)
}

/// Closure under repetition, recording which way the quantifier leaned.
///
/// Adds states reachable by one more application of `p` until an iteration
/// reaches nothing it has not already reached as well or better, or until
/// `bound` iterations. Each state is held once, at its preferred derivation,
/// in a `HashMap` keyed on the state's identity, so the fixpoint is linear in
/// the number of distinct states rather than quadratic.
///
/// Two judgements, made separately. What a state records is decided on the
/// entry, so a derivation arriving at a state it cannot improve still offers
/// the entry it would write. Whether that state iterates again is decided on
/// the arrival, and only a strictly preferred arrival goes round.
///
/// The second is what terminates the fixpoint: a preference path is only ever
/// extended, and an extension compares larger than the path it grew from, so a
/// body that matched empty hands its state back ranked below the one it
/// arrived with and is refused.
///
/// Every state that stops here records the choice, in one of two forms. A body
/// that makes no choice of its own is fully described by its iteration count,
/// so one entry carries it. A body that does make choices needs its per
/// iteration entries to line up against another derivation's, so each
/// iteration writes a continue marker and stopping writes a stop marker; the
/// two markers are ordered by `g`, which is the whole of what lazy means here.
/// How many states a fixpoint's seen-set holds before it stops being a slice
/// and becomes a map.
///
/// Measured over the crate's own source: of the five shapes that reach a
/// fixpoint, four hold at most sixty-four states in every call they make - zero
/// calls past it - and the fifth exceeds sixty-four in 5,352 of its 864,634
/// calls, two hundred and fifty-six in 1,467 and a thousand in 333. So a slice
/// answers almost every call and the map is there for the one shape that
/// reaches hundreds.
const SEEN_LINEAR: usize = 16;

/// The states a fixpoint has already reached, each with where its entry sits in
/// the result and the rank it arrived by.
///
/// A slice beats a map at these sizes twice over. The allocation is the first:
/// a map asks the heap on its first insert and a fixpoint runs once an anchor,
/// where the slice is held per thread and cleared. The comparison is the
/// second: [`State`] rejects on `pos` before it reads the register map, and
/// hashing must read both, so a short scan asks less of a state than one hash
/// does.
///
/// It is a slice only while it is short. Past [`SEEN_LINEAR`] the states move
/// into a map, which bounds what a scan costs on a shape that reaches hundreds
/// of them.
struct Seen {
    few: Vec<(State, usize, Rank)>,
    many: Option<std::collections::HashMap<State, (usize, Rank), crate::fxhash::FxBuild>>,
}

impl Seen {
    /// Where `s` sits and the rank it arrived by, or `None` for a state this
    /// fixpoint has not reached.
    fn get(&self, s: &State) -> Option<(usize, Rank)> {
        match &self.many {
            Some(m) => m.get(s).map(|(at, rank)| (*at, rank.clone())),
            None => self
                .few
                .iter()
                .find(|(held, _, _)| held == s)
                .map(|(_, at, rank)| (*at, rank.clone())),
        }
    }

    /// Record that `s` sits at `at` and arrived by `rank`, replacing whatever
    /// the set held for it.
    fn insert(&mut self, s: State, at: usize, rank: Rank) {
        if let Some(m) = &mut self.many {
            m.insert(s, (at, rank));
            return;
        }
        if let Some(slot) = self.few.iter_mut().find(|(held, _, _)| *held == s) {
            slot.1 = at;
            slot.2 = rank;
            return;
        }
        self.few.push((s, at, rank));
        if self.few.len() > SEEN_LINEAR {
            let mut m = std::collections::HashMap::with_capacity_and_hasher(
                self.few.len() * 2,
                crate::fxhash::FxBuild::process(),
            );
            for (state, at, rank) in self.few.drain(..) {
                m.insert(state, (at, rank));
            }
            self.many = Some(m);
        }
    }

    /// How many states the set holds.
    fn held(&self) -> usize {
        match &self.many {
            Some(m) => m.len(),
            None => self.few.len(),
        }
    }
}

thread_local! {
    /// The slice a fixpoint's seen-set scans, held past the call that filled
    /// it; see [`Seen`].
    ///
    /// A vector clears in its length where a map clears in its capacity, which
    /// is what makes this one safe to hold where the map at [`dedup`] was
    /// measured worse for being held.
    ///
    /// Taking it out leaves an empty vector behind, so a fixpoint reached from
    /// inside another gets its own rather than the outer one's states.
    static SEEN: std::cell::RefCell<Vec<(State, usize, Rank)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[allow(clippy::too_many_arguments)]
fn unroll(
    p: &Pattern,
    g: Greed,
    bound: Option<usize>,
    input: &[u8],
    toks: &[Token],
    end: usize,
    states: Vec<State>,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> Vec<State> {
    let choiceful = p.has_choice();
    let (cont, stop) = match g {
        Greed::Greedy => (0u8, 1u8),
        Greed::Lazy => (1u8, 0u8),
    };
    // The rank each state was last reached at, and where its entry sits in
    // `result`, so a preferred arrival replaces the entry rather than adding a
    // second one for the same state. Looked up once per state per iteration,
    // which is what makes the shape of [`Seen`] worth its lines: a slice while
    // it is short and a map past that. `result` holds the entries in first-seen
    // order, so neither how the set is held nor where a state sits inside it
    // can reach the answer.
    let mut best =
        Seen { few: SEEN.with(|s| std::mem::take(&mut *s.borrow_mut())), many: None };
    let mut result: Vec<State> = Vec::new();
    let mut frontier: Vec<State> = states;
    let mut iters: usize = 0;
    // The next round's frontier, built once and recycled rather than at each
    // iteration.
    //
    // Measured over the crate's own source, this loop goes round 3.1 million
    // times for `(?>\W*) "="` and 4.5 million for `\W \B`, and its frontier
    // reaches four states each time - a vector of 160 bytes, which is the size
    // bucket the sampled allocation sites kept naming. Building one an
    // iteration was the largest remaining allocation in a scan.
    //
    // The recycling is the swap below: draining the frontier leaves a vector
    // that is empty and still holds its capacity, and that is what next round
    // pushes into. A body that makes choices of its own still collects a marked
    // copy, which is a second vector this does not remove.
    let mut fresh: Vec<State> = Vec::new();
    loop {
        fresh.clear();
        for s in frontier.drain(..) {
            let entry = if choiceful {
                s.with_branch(stop)
            } else {
                s.with_branch(count_key(g, iters))
            };
            let Some((i, held)) = best.get(&s) else {
                best.insert(s.clone(), result.len(), s.rank.clone());
                result.push(entry);
                fresh.push(s);
                continue;
            };
            // What a derivation records is judged on the entry itself, so a
            // pass that ends where it began still offers the loop the single
            // empty iteration leftmost-first allows it before stopping.
            if cmp_rank(entry.rank_slice(), result[i].rank_slice()) == std::cmp::Ordering::Less {
                result[i] = entry;
            }
            // Whether to go round again is judged on the arrival instead, and
            // an extended path never beats the one it grew from. That is what
            // separates the one empty iteration from a second.
            let held = held.as_slice();
            if cmp_rank(s.rank_slice(), held) == std::cmp::Ordering::Less {
                best.insert(s.clone(), i, s.rank.clone());
                fresh.push(s);
            }
        }
        if fresh.is_empty() || bound.is_some_and(|nn| iters >= nn) {
            note_fixpoint_map(best.held());
            // The slice goes back whether or not the states moved into a map:
            // promotion drains it, so what returns is empty either way and
            // still holds the capacity the next call pushes into.
            let mut few = best.few;
            few.clear();
            SEEN.with(|s| *s.borrow_mut() = few);
            return result;
        }
        let grown = fresh.capacity();
        let marked: Vec<State> = if choiceful {
            fresh.iter().map(|s| s.with_branch(cont)).collect()
        } else {
            std::mem::take(&mut fresh)
        };
        note_unrolled(grown, marked.capacity());
        // The drained frontier is empty and still holds its capacity, so it
        // becomes next round's buffer before `advance` replaces it. Where the
        // body made no choices `fresh` was just handed away, and this is what
        // it gets back in place of a fresh allocation.
        std::mem::swap(&mut fresh, &mut frontier);
        frontier = advance(p, input, toks, end, marked, absent, regs, fields);
        iters += 1;
    }
}

// Threads the scan context (input, tokens, end, state set, prefilter,
// register table) of a recursive descent; bundling it would not change the
// generated code.
#[allow(clippy::too_many_arguments)]
fn repeat(
    p: &Pattern,
    bounds: (usize, Option<usize>),
    g: Greed,
    input: &[u8],
    toks: &[Token],
    end: usize,
    states: Vec<State>,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> Vec<State> {
    let (m, n) = bounds;
    // The first `m` copies are mandatory, so they carry no choice and write
    // no rank entry. Only the optional tail leans.
    let mut cur = states;
    for _ in 0..m {
        cur = advance(p, input, toks, end, cur, absent, regs, fields);
        if cur.is_empty() {
            return cur;
        }
    }
    let optional = n.map(|nn| nn.saturating_sub(m));
    unroll(p, g, optional, input, toks, end, cur, absent, regs, fields)
}

#[allow(clippy::too_many_arguments)]
fn bind(
    name: &str,
    p: &Pattern,
    input: &[u8],
    toks: &[Token],
    end: usize,
    states: Vec<State>,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> Vec<State> {
    // The name is interned once here, not per produced state: every bound
    // register was collected into `regs` at scan setup.
    let id = reg_id(regs, name).expect("bind register is collected");
    // A register bound under a repetition keeps every binding: each is one
    // node on the state's list, shared with every fork after it.
    let listed = fields.lists.contains(&id);
    let record = |r: &mut State, span: (usize, usize)| {
        if listed {
            r.hist = Some(Rc::new(HistNode { id, span, prev: r.hist.take() }));
        }
    };

    // Fast path: binding a single atom. An atom consumes exactly the one
    // significant token at `r.pos - 1`, so its bound span is that token and
    // each input state maps to at most one result. This skips the general
    // path's per-state one-element vector and `advance` dispatch, which the
    // flame graph showed dominate a register-binding scan.
    if let Pattern::Atom(atom) = p {
        let out: Vec<State> = states
            .into_iter()
            .filter_map(|s| {
                let mut r = match_atom(atom, input, toks, end, &s, regs, fields)?;
                let tok = r.pos - 1;
                let span = (toks[tok].start(), toks[tok].end());
                Rc::make_mut(&mut r.env).insert(id, span);
                record(&mut r, span);
                Some(r)
            })
            .collect();
        return dedup(out);
    }

    let mut out = Vec::new();
    for s in states {
        let start_pos = skip_ws(toks, s.pos, end);
        // The state moves into the call rather than being copied beside it. A
        // copy left standing here holds a second reference to the register map
        // the results share, which is what makes the `make_mut` below copy that
        // map where it could have written it in place.
        for mut r in advance(p, input, toks, end, vec![s], absent, regs, fields) {
            let span = if r.pos > start_pos {
                (toks[start_pos].start(), toks[r.pos - 1].end())
            } else {
                (0, 0)
            };
            // Copy-on-write: clones the shared map only here, at the bind,
            // not on every carry-clone of a state. The value is the span,
            // resolved to text lazily, so no text is allocated here.
            Rc::make_mut(&mut r.env).insert(id, span);
            record(&mut r, span);
            out.push(r);
        }
    }
    dedup(out)
}

#[allow(clippy::too_many_arguments)]
fn balanced(
    kind: Option<crate::token::BracketKind>,
    p: &Pattern,
    input: &[u8],
    toks: &[Token],
    end: usize,
    states: Vec<State>,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> Vec<State> {
    // Registers bound with `::name` inside this group are scoped to it: their
    // values are restored to the pre-group state when the group closes, so a
    // reference cannot leak out. Computed once from the interior pattern.
    let mut scoped_ids: Vec<u16> = Vec::new();
    collect_scoped_ids(p, regs, &mut scoped_ids);

    let mut out = Vec::new();
    for s in states {
        let p0 = skip_ws(toks, s.pos, end);
        if p0 >= end {
            continue;
        }
        let TokenKind::Open(bk) = toks[p0].kind else {
            continue;
        };
        if let Some(want) = kind
            && want != bk
        {
            continue;
        }
        let Some(m) = toks[p0].mate() else {
            continue;
        };
        if m >= end {
            continue;
        }
        // The interior must consume exactly the enclosed span.
        let pre_env = s.env.clone();
        let inner_start =
            vec![State { pos: p0 + 1, env: s.env.clone(), rank: s.rank.clone(), hist: s.hist.clone() }];
        for ist in advance(p, input, toks, m, inner_start, absent, regs, fields) {
            if skip_ws(toks, ist.pos, m) == m {
                let rank = ist.rank.clone();
                let hist = ist.hist.clone();
                let env = restore_scoped(ist.env, &pre_env, &scoped_ids);
                out.push(State { pos: m + 1, env, rank, hist });
            }
        }
    }
    dedup(out)
}

/// The register ids bound with a scoped `::name` anywhere in `pat`, deduplicated.
/// A balanced group uses this to know which bindings to drop when it closes.
fn collect_scoped_ids(pat: &Pattern, regs: &[String], out: &mut Vec<u16>) {
    match pat {
        Pattern::Bind(name, scoped, p) => {
            if *scoped
                && let Some(id) = reg_id(regs, name)
                && !out.contains(&id)
            {
                out.push(id);
            }
            collect_scoped_ids(p, regs, out);
        }
        Pattern::Within(v, _) => {
            for (name, scoped) in v.iter().filter_map(|e| e.bind.as_ref()) {
                if *scoped
                    && let Some(id) = reg_id(regs, name)
                    && !out.contains(&id)
                {
                    out.push(id);
                }
            }
        }
        Pattern::Empty | Pattern::Atom(_) | Pattern::Guard(..) | Pattern::Anchor(_) => {}
        Pattern::Assert(p, _, _) | Pattern::Atomic(p) => collect_scoped_ids(p, regs, out),
        Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) => {
            collect_scoped_ids(p, regs, out);
        }
        Pattern::Repeat(p, _, _, _) | Pattern::Balanced(_, p) | Pattern::Field(_, p) => {
            collect_scoped_ids(p, regs, out);
        }
        Pattern::Concat(v) | Pattern::Alt(v, _) => {
            for c in v {
                collect_scoped_ids(c, regs, out);
            }
        }
    }
}

/// Restore each scoped register id in `env` to its value in `pre` (the state
/// before the group was entered), removing it when it was unbound there. This
/// drops any binding a `::name` made inside the group so it cannot leak out.
/// When nothing is scoped, `env` is returned untouched (no clone).
fn restore_scoped(env: Env, pre: &Env, scoped_ids: &[u16]) -> Env {
    if scoped_ids.is_empty() {
        return env;
    }
    let mut e = env;
    let map = Rc::make_mut(&mut e);
    for &id in scoped_ids {
        match pre.get(&id) {
            Some(&v) => {
                map.insert(id, v);
            }
            None => {
                map.remove(&id);
            }
        }
    }
    e
}

thread_local! {
    /// The vector an assertion's probe runs from; see [`probe_matches`].
    static PROBE: std::cell::RefCell<Vec<State>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Whether `inner` matches starting at token `at`, under `env`.
///
/// An assertion asks this once per state it filters, and at each position in a
/// window for the counting forms, so a scan runs it far more often than it runs
/// the assertion itself. Each run needs a vector holding the one state it
/// starts from, and building one at each was among the sites a sampled scan
/// allocates from.
///
/// The vector is carried instead, on the same argument that lets `best_at`
/// carry its own: `advance` takes one by value and gives one back, and what
/// comes back is read for emptiness and dropped here rather than escaping.
/// Taking it out leaves an empty vector behind, so an assertion nested inside
/// another gets a fresh one rather than the outer probe's states.
#[allow(clippy::too_many_arguments)]
fn probe_matches(
    inner: &Pattern,
    at: usize,
    env: &Env,
    input: &[u8],
    toks: &[Token],
    end: usize,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> bool {
    let mut probe = PROBE.with(|p| std::mem::take(&mut *p.borrow_mut()));
    probe.clear();
    probe.push(State { pos: at, env: env.clone(), rank: Rank::default(), hist: None });
    let mut out = advance(inner, input, toks, end, probe, absent, regs, fields);
    let hit = !out.is_empty();
    out.clear();
    PROBE.with(|p| *p.borrow_mut() = out);
    hit
}

/// A zero-width sub-pattern assertion: keep a state when `inner` matches at
/// the position, without advancing it.
///
/// Looking ahead runs `inner` from the position and asks only whether any
/// state survives. Looking behind asks a different question - whether `inner`
/// matches with its end at the position, not its start - so it tries each
/// start in a window and requires the run to land exactly there. The window is
/// bounded by the sub-pattern's longest match, which is why an unbounded one
/// is rejected at parse time rather than scanning the whole prefix.
///
/// The probe's own bindings are discarded: the surviving state keeps the
/// registers and position it arrived with, so an assertion is a filter and
/// never a capture.
#[allow(clippy::too_many_arguments)]
fn assert_zero_width(
    inner: &Pattern,
    neg: bool,
    look: crate::ast::Look,
    input: &[u8],
    toks: &[Token],
    end: usize,
    states: Vec<State>,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> Vec<State> {
    use crate::ast::Look;
    let window = crate::nfa::bounded_max_len(inner).unwrap_or(0);
    states
        .into_iter()
        .filter(|s| {
            let p = skip_ws(toks, s.pos, end);
            let hit = match look {
                Look::Ahead => probe_matches(inner, p, &s.env, input, toks, end, absent, regs, fields),
                Look::Within { window, at_least, at_most } => {
                    // The next `window` significant tokens, this one first.
                    // Each is a start position the sub-pattern is tried at, so
                    // the question is how many of them it matches at rather
                    // than whether it matches exactly here.
                    let mut tried = 0usize;
                    let mut found = 0usize;
                    let mut j = p;
                    // One past the top is already a failure, so counting stops
                    // there; with no top, the first hit that meets the bottom
                    // settles it.
                    let enough = at_most.map_or(at_least, |hi| hi + 1);
                    while j < end && tried < window && found < enough {
                        if toks[j].kind != TokenKind::Whitespace {
                            tried += 1;
                            if probe_matches(
                                inner, j, &s.env, input, toks, end, absent, regs, fields,
                            ) {
                                found += 1;
                            }
                        }
                        j += 1;
                    }
                    found >= at_least && at_most.is_none_or(|hi| found <= hi)
                }
                Look::InGroup { at_least, at_most } => {
                    // The region is the group opening here, so the lexer's
                    // bracket pairing gives its extent and nothing in the
                    // pattern does. A position opening no group holds an empty
                    // region, whose count is zero under either polarity.
                    let mut interior = p..p;
                    if p < end
                        && matches!(toks[p].kind, TokenKind::Open(_))
                        && let Some(m) = toks[p].mate()
                        && m < end
                    {
                        interior = p + 1..m;
                    }
                    let close = interior.end;
                    let mut found = 0usize;
                    // One past the top is already a failure, so counting stops
                    // there; with no top, the first hit that meets the bottom
                    // settles it.
                    let enough = at_most.map_or(at_least, |hi| hi + 1);
                    for j in interior {
                        if found >= enough {
                            break;
                        }
                        if toks[j].kind == TokenKind::Whitespace {
                            continue;
                        }
                        // The region's close is the sub-pattern's end, so a
                        // counted match cannot run past the bracket bounding
                        // the region it is counted in.
                        if probe_matches(
                            inner, j, &s.env, input, toks, close, absent, regs, fields,
                        ) {
                            found += 1;
                        }
                    }
                    found >= at_least && at_most.is_none_or(|hi| found <= hi)
                }
                Look::Behind => {
                    // Every token index that could start a match ending at p.
                    // `window` counts significant tokens, and whitespace only
                    // widens the span, so scanning back twice that many token
                    // slots covers it.
                    let lo = p.saturating_sub(window.saturating_mul(2));
                    (lo..p).any(|j| {
                        if toks[j].kind == TokenKind::Whitespace {
                            return false;
                        }
                        // Carried like the other probes, but read rather than
                        // only tested: looking behind asks where the run ended
                        // and not merely whether it ran, so the states are
                        // needed and `probe_matches` cannot answer it.
                        let mut probe = PROBE.with(|b| std::mem::take(&mut *b.borrow_mut()));
                        probe.clear();
                        probe.push(State {
                            pos: j,
                            env: s.env.clone(),
                            rank: Rank::default(),
                            hist: None,
                        });
                        let mut out = advance(inner, input, toks, p, probe, absent, regs, fields);
                        let hit = out.iter().any(|r| skip_ws(toks, r.pos, p) == p);
                        out.clear();
                        PROBE.with(|b| *b.borrow_mut() = out);
                        hit
                    })
                }
            };
            hit != neg
        })
        .collect()
}

fn guard(
    lit: &str,
    neg: bool,
    input: &[u8],
    toks: &[Token],
    end: usize,
    states: Vec<State>,
    absent: &HashSet<Vec<u8>>,
) -> Vec<State> {
    // A literal the prefilter proved is nowhere in the input resolves every
    // state at once with no positional scan: a positive guard fails (the
    // literal it requires cannot appear), a negative guard passes (the
    // literal it forbids is confirmed absent).
    if absent.contains(lit.as_bytes()) {
        return if neg { states } else { Vec::new() };
    }
    states
        .into_iter()
        .filter(|s| {
            let p = skip_ws(toks, s.pos, end);
            let from = if p < toks.len() { toks[p].start() } else { input.len() };
            // Positive guard keeps a state where the literal is present ahead;
            // negative guard keeps it where the literal is absent.
            crate::byte_simd::contains(&input[from..], lit.as_bytes()) != neg
        })
        .collect()
}

/// Collapse states sharing a position and register environment, keeping the
/// preferred one.
///
/// Identity is `(pos, env)`, so two derivations that arrived the same way are
/// one state with one future; which of them is kept decides only the
/// preference carried forward, and [`cmp_rank`] picks it. Keeping whichever
/// arrived first instead would be right only while branches are walked in
/// order, and is wrong as soon as a greedy repetition reaches a position by
/// two paths of different length.
fn dedup(states: Vec<State>) -> Vec<State> {
    // A set of zero or one state has no duplicate to remove, so skip the
    // allocation. Bind and the atom-bind path produce one-state sets
    // constantly, so this removes a per-call allocation from the hot path of
    // every register-binding scan.
    if states.len() <= 1 {
        return states;
    }
    // The crate's own hash rather than SipHash, for the reason the single-pass
    // engine's register-dedup set uses it: a state's key carries its whole
    // register map, and this runs on every arm of every step. The map is an
    // index only - `out` holds the states in first-seen order - so which
    // bucket a state lands in cannot reach the answer.
    //
    // The map is built here and sized to this call. Holding one per thread and
    // clearing it instead was measurably worse: a hash map clears in the size
    // of its capacity rather than its length, so one pattern that briefly
    // reaches a wide set leaves every later two-state dedup clearing the whole
    // retained table.
    let mut at: std::collections::HashMap<State, usize, crate::fxhash::FxBuild> =
        std::collections::HashMap::with_capacity_and_hasher(
            states.len(),
            crate::fxhash::FxBuild::process(),
        );
    let mut out: Vec<State> = Vec::with_capacity(states.len());
    for s in states {
        match at.get(&s) {
            Some(&i) => {
                if cmp_rank(s.rank_slice(), out[i].rank_slice()) == std::cmp::Ordering::Less {
                    out[i] = s;
                }
            }
            None => {
                at.insert(s.clone(), out.len());
                out.push(s);
            }
        }
    }
    out
}

/// Match the inner pattern only at the start of the k-th comma-
/// delimited field. `@k` anchors to the input's field structure, so a
/// state advances only when its position is that field's first token.
#[allow(clippy::too_many_arguments)]
fn field_match(
    k: usize,
    inner: &Pattern,
    input: &[u8],
    toks: &[Token],
    end: usize,
    states: Vec<State>,
    absent: &HashSet<Vec<u8>>,
    regs: &[String],
    fields: Fields<'_>,
) -> Vec<State> {
    let mut out = Vec::new();
    for s in states {
        let p = skip_ws(toks, s.pos, end);
        if is_field_start(end, p, k, fields) {
            let init = vec![State { pos: p, env: s.env, rank: s.rank, hist: s.hist }];
            out.extend(advance(inner, input, toks, end, init, absent, regs, fields));
        }
    }
    dedup(out)
}

/// Whether token position `p` begins the k-th comma-delimited field. `@0` and
/// `@1` both name the first field.
fn is_field_start(end: usize, p: usize, k: usize, fields: Fields<'_>) -> bool {
    if p >= end {
        return false;
    }
    fields.field_starts.is_some_and(|f| f.get(p).copied() == Some(k.max(1) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    /// Holding the registers inline must not widen a match, because every match
    /// of every pattern carries them and most patterns bind none. Two spans are
    /// sixteen bytes and the shared form's fat pointer is sixteen, so the two
    /// variants are the same width and the enum is the width of the `Vec` it
    /// replaced.
    #[test]
    fn holding_registers_inline_does_not_widen_a_match() {
        assert_eq!(
            size_of::<Regs>(),
            size_of::<Vec<Span>>(),
            "Regs is {} bytes against a Vec's {}",
            size_of::<Regs>(),
            size_of::<Vec<Span>>()
        );
        // The bindings under a repetition are one word, a null pointer for
        // every match of a pattern that binds none under one.
        assert_eq!(size_of::<Match>(), 64, "a match is {} bytes", size_of::<Match>());
    }

    /// The two ways of holding registers answer alike, at every count either
    /// can hold, and a match compares by its registers and not by how it holds
    /// them.
    #[test]
    fn registers_read_the_same_inline_and_shared() {
        let spans: Vec<Span> =
            (0..8u32).map(|i| Span { start: i * 10, end: i * 10 + 4 }).collect();
        for n in 0..=spans.len() {
            let regs = Regs::from_slice(&spans[..n]);
            assert_eq!(regs.as_slice(), &spans[..n], "{n} registers read back");
            assert_eq!(regs.len(), n, "{n} registers counted");
            let shared = Regs::Shared(spans[..n].into());
            assert_eq!(regs, shared, "{n} registers compare alike however held");
            let held = match n {
                0 => matches!(regs, Regs::None),
                1..=INLINE_REGS => matches!(regs, Regs::Inline(..)),
                _ => matches!(regs, Regs::Shared(_)),
            };
            assert!(held, "{n} registers held the way its count calls for");
            // No registers is the commonest match in the crate and is built by
            // a path that does nothing else, so it must cost a discriminant
            // and not a zeroed array.
            assert!(matches!(Regs::none(), Regs::None), "no registers holds nothing");
            // Writing through the shared form must not reach another holder of
            // the same spans.
            let mut mine = shared.clone();
            let theirs = Regs::Shared(spans[..n].into());
            for sp in mine.as_mut_slice() {
                sp.start += 1;
            }
            assert_eq!(theirs.as_slice(), &spans[..n], "{n} registers left alone");
            assert!(
                mine.iter().zip(&spans[..n]).all(|(a, b)| a.start == b.start + 1),
                "{n} registers written through"
            );
        }
    }

    /// A scan with its captures resolved, so a test reads a match the way a
    /// caller wanting the registers does.
    fn run(pattern: &str, input: &str) -> Vec<Match> {
        let p = parse(pattern).unwrap();
        let spans = scan(&p, input.as_bytes());
        captures(&p, input.as_bytes(), &spans)
    }

    /// A binding constrains nothing, so the anchored ladders answer a pattern
    /// that binds exactly as they answer its unbound twin, and both agree with
    /// the scan. The rung that makes them fast must not make them differ.
    #[test]
    fn the_anchored_ladders_answer_a_binding_pattern_as_they_answer_its_twin() {
        let mut text = String::new();
        for i in 0..300u32 {
            text.push_str(&format!("let value_{i} = {} ; call_{i}(alpha, beta) ;\n", i * 37));
        }
        let input = text.as_bytes();
        // The last pair matches nowhere - the corpus holds no `!` - so the
        // ladders are held to a verdict of no match as well as to a span.
        for (bound, bare) in [
            ("\\W:name \"=\"", "\\W \"=\""),
            ("\"let\" \\W:v \"=\"", "\"let\" \\W \"=\""),
            ("\\W:w", "\\W"),
            ("\\N:n", "\\N"),
            ("\\W:w \"!\"", "\\W \"!\""),
        ] {
            let (b, u) = (parse(bound).expect(bound), parse(bare).expect(bare));
            let spans = scan(&u, input);
            assert_eq!(scan(&b, input), spans, "scan {bound}");
            assert_eq!(is_match(&b, input), !spans.is_empty(), "is_match {bound}");
            assert_eq!(is_match(&u, input), !spans.is_empty(), "is_match {bare}");
            let first = spans.first().copied();
            assert_eq!(routed_first(&b, input).flatten(), first, "first {bound}");
            assert_eq!(routed_first(&u, input).flatten(), first, "first {bare}");
            // From past the first match, so the ladder is answering about the
            // second rather than repeating the first.
            let at = first.map_or(0, |s| s.end());
            let next = spans.iter().find(|s| s.start() >= at).copied();
            assert_eq!(routed_first_at(&b, input, at).flatten(), next, "at {bound}");
            assert_eq!(routed_first_at(&u, input, at).flatten(), next, "at {bare}");
        }
    }

    /// The soonest-ending match of a flat shape is its leftmost, so the
    /// positional half of the ladder may take the windows - and must report
    /// what the whole scan's first match is when it does.
    #[test]
    fn the_positional_ladder_takes_the_windows_and_answers_what_the_scan_does() {
        let mut text = String::new();
        for i in 0..300u32 {
            text.push_str(&format!("call_{i}(alpha) ; let value_{i} = {i} ;\n"));
        }
        let input = text.as_bytes();
        // Each opens with a literal, so the windows answer them, and none has a
        // byte route of its own: this is the rung under test.
        for src in ["\"let\" \\W \"=\"", "\"let\" \\W:v \"=\"", "\"let\" \\W"] {
            let p = parse(src).expect(src);
            let first = scan(&p, input).first().copied();
            assert!(first.is_some(), "{src} matches the corpus");
            assert_eq!(routed_first_positional(&p, input).flatten(), first, "{src}");
            assert_eq!(crate::shortest_match(&p, input), first.map(|s| s.end()), "{src}");
        }
    }

    #[test]
    fn a_pattern_names_the_reading_its_empty_loops_take() {
        use crate::parser::parse_with_empty_loop;
        let spans = |ms: Vec<Span>| -> Vec<(usize, usize)> {
            ms.iter().map(|m| (m.start(), m.end())).collect()
        };
        let input = b"42 baz bar ";

        // The body's highest-priority branch is nullable and its sibling
        // consumes, which is the whole of where the two families differ. The
        // crate's reading drops the thread that matched empty, so the branch
        // that consumed wins and the loop runs on. The backtracking reading
        // takes that iteration and leaves the loop, so the match ends earlier
        // and a second one follows it.
        for src in [r"(\N? | \W)+ .", r"(\N? | \W)* ."] {
            let p = parse(src).expect("parses");
            assert_eq!(
                spans(scan_with_empty_loop(&p, input, EmptyLoop::Thompson)),
                vec![(0, 10)],
                "{src} under the crate's reading"
            );
            assert_eq!(
                spans(scan_with_empty_loop(&p, input, EmptyLoop::Perl)),
                vec![(0, 6), (7, 10)],
                "{src} under the backtracking reading"
            );
            // Naming no reading is naming the crate's.
            assert_eq!(spans(scan(&p, input)), vec![(0, 10)], "{src} unnamed");
        }

        // Two controls. Putting the consuming branch first leaves no choice
        // for a reading to make, and a body with no alternation offers no
        // branch to prefer; both read the same either way.
        for src in [r"(\W | \N?)+ .", r"(\N?)+ \W"] {
            let p = parse(src).expect("parses");
            assert_eq!(
                spans(scan_with_empty_loop(&p, input, EmptyLoop::Thompson)),
                spans(scan_with_empty_loop(&p, input, EmptyLoop::Perl)),
                "{src} reads the same either way"
            );
        }

        // A pattern with no nullable loop names a reading and keeps the
        // single-pass engine, because there is nothing for the readings to
        // disagree about.
        let flat = parse(r"\W+ \N").expect("parses");
        assert!(!crate::nfa::empty_loop_needs_set_engine(&flat));

        // The directive is read off the front and does not enter the tree.
        let (p, mode) = parse_with_empty_loop(r"(?empty:perl)(\N? | \W)+ .").expect("parses");
        assert_eq!(mode, EmptyLoop::Perl);
        assert_eq!(p, parse(r"(\N? | \W)+ .").expect("parses"));
        assert_eq!(spans(scan_with_empty_loop(&p, input, mode)), vec![(0, 6), (7, 10)]);

        let (_, mode) = parse_with_empty_loop(r"\W+").expect("parses");
        assert_eq!(mode, EmptyLoop::Thompson, "the crate's reading is the default");

        let e = parse_with_empty_loop(r"(?empty:pcre)\W+").expect_err("names no reading");
        assert!(e.msg.contains("thompson"), "the error names the readings: {}", e.msg);
    }

    /// Lines binding values to a few recurring keys: latencies near a
    /// hundred, sizes near a million, and one latency planted four orders
    /// above its kind.
    fn log_with_an_outlier() -> (String, usize) {
        let mut s = String::new();
        let mut planted = 0;
        for i in 0..240 {
            match i % 3 {
                0 => {
                    let v = if i == 150 { 5_000_000 } else { 90 + (i * 37) % 21 };
                    if i == 150 {
                        planted = s.len() + format!("svc_{} latency = ", i % 7).len();
                    }
                    // The value stands without a unit: a unit symbol one space
                    // after a number is that quantity's, and the number then
                    // belongs to a quantity token rather than to `\N`.
                    s.push_str(&format!("svc_{} latency = {v} ;\n", i % 7));
                }
                1 => s.push_str(&format!("svc_{} size = {} ;\n", i % 7, 1_000_000 + i)),
                _ => s.push_str(&format!("state: {} ;\n", if i % 2 == 0 { "ready" } else { "busy" })),
            }
        }
        (s, planted)
    }

    #[test]
    fn a_relative_magnitude_predicate_reads_its_threshold_from_the_window() {
        let (log, planted) = log_with_an_outlier();
        // Every size is six orders; every latency two; words one or two. A
        // window mean sits between, so the planted latency at six and a
        // half orders clears it by two and the sizes do too - the window
        // knows neighbourhoods, not keys.
        let m = run("\\N{>+2}", &log);
        assert!(m.iter().any(|m| m.start == planted), "the planted latency is found: {m:?}");
        assert!(m.iter().all(|m| log[m.start..m.end].len() >= 7), "only the large numbers: {m:?}");
        // Two standard deviations above the window, the same reading in the
        // window's own units.
        let m = run("\\N{>+2s}", &log);
        assert!(m.iter().any(|m| m.start == planted), "the planted latency clears two sigmas: {m:?}");
        // An absolute threshold still routes and reads as before.
        let m = run("\\N{mag>6}", &log);
        assert!(m.iter().all(|m| log[m.start..m.end].len() >= 7) && m.len() > 70, "{}", m.len());
    }

    #[test]
    fn a_keyed_relative_predicate_reads_the_history_of_that_keys_values() {
        let (log, planted) = log_with_an_outlier();
        // The baseline is the values bound to earlier `latency` keys, so the
        // sizes, which are large but bound to another key, are not outliers
        // and the planted latency is the one match.
        let m = run("\"latency\":k \"=\" \\N{>+1:k}", &log);
        assert_eq!(m.len(), 1, "{m:?}");
        assert!(log[m[0].start..m[0].end].starts_with("latency = 5000000"), "{m:?}");
        assert_eq!(m[0].end, planted + "5000000".len(), "the match ends at the planted value");
        // The first occurrence of a key has no history, so nothing at all is
        // an outlier against it.
        let m = run("\"size\":k \"=\" \\N{>+0:k}", &log);
        assert!(m.iter().all(|m| !log[..m.start].is_empty()), "the first size never matches: {m:?}");
        assert!(m.len() > 30, "later sizes sit at the mean, so `>+0` holds where a value repeats: {}", m.len());
    }

    #[test]
    fn a_phase_anchor_selects_a_column_of_a_periodic_record() {
        // Six significant tokens a row, no header: word, comma, number,
        // comma, word, semicolon.
        let mut rows = String::new();
        for i in 0..200 {
            rows.push_str(&format!("r{i} , {} , x{} ;\n", i * 3, i % 4));
        }
        let toks = crate::lexer::lex(rows.as_bytes());
        assert_eq!(crate::context::record_period(&toks, rows.as_bytes()), Some(6), "the row is the period");
        let m = run("@phase:2 \\N", &rows);
        assert_eq!(m.len(), 200, "every row's number sits at phase two: {}", m.len());
        assert!(run("@phase:0 \\N", &rows).is_empty(), "no number sits at phase zero");
        assert_eq!(run("@phase:4 \\W", &rows).len(), 200, "the second word is at phase four");
    }

    #[test]
    fn a_declared_shape_becomes_a_token_the_default_lexer_would_split() {
        use crate::custom::{Precedence, ShapeSet};
        let input = b"ticket ABC-1234 done";
        // The default lexer splits this into a word, a hyphen and a number.
        let split = crate::lexer::lex(input);
        assert!(
            split.iter().filter(|t| t.is_significant()).count() > 3,
            "the default lexer splits ABC-1234"
        );

        let mut shapes = ShapeSet::new();
        shapes.declare("order = `[A-Z]{3}-[0-9]{4}`", Precedence::Before).expect("declares");
        let pat = crate::parser::parse_with_shapes("\\{order}", &shapes).expect("parses");
        let m = crate::engine::scan_with_shapes(&pat, input, &shapes);
        assert_eq!(m.len(), 1, "the shape matches once");
        assert_eq!(&input[m[0].range()], b"ABC-1234");
    }

    #[test]
    fn both_engines_agree_on_custom_kinds() {
        use crate::custom::{Precedence, ShapeSet};
        let mut shapes = ShapeSet::new();
        shapes.declare("order = `[A-Z]{3}-[0-9]{4}`", Precedence::Before).expect("declares");
        shapes.declare("level = `(DEBUG|INFO|WARN|ERROR)`", Precedence::Before).expect("declares");

        let inputs: &[&str] = &[
            "",
            "ABC-1234",
            "ticket ABC-1234 done",
            "ERROR ABC-1234 and WARN XYZ-9999 here",
            "no shapes at all in this line",
            "ABC-1234 ABC-1234 ABC-1234",
            "INFO 12 ABC-1234 (nested DEF-5678) tail",
        ];
        let patterns: &[&str] = &[
            "\\{order}",
            "\\{level}",
            "\\{level} \\{order}",
            "\\{order}+",
            "\\{order} | \\{level}",
            "\\{level} .* \\{order}",
            "\\W \\{order}",
            "\\{order}:x =x",
        ];
        for pat_src in patterns {
            let pat = crate::parser::parse_with_shapes(pat_src, &shapes).expect("parses");
            for inp in inputs {
                let bytes = inp.as_bytes();
                // One lex, both engines, so the comparison is of matching and
                // not of two different token streams.
                let toks = crate::lexer::lex_with_shapes(bytes, &[], &shapes, 0);
                let set = scan_tokens_from(&pat, bytes, &toks, 0);
                if let Some(nfa) = crate::nfa::scan_nfa_over(&pat, bytes, &toks) {
                    assert_eq!(nfa, set, "engines differ on {pat_src:?} over {inp:?}");
                }
                // The public entry must agree with the set engine too.
                assert_eq!(
                    crate::engine::scan_with_shapes(&pat, bytes, &shapes),
                    set,
                    "scan_with_shapes differs on {pat_src:?} over {inp:?}"
                );
            }
        }
    }

    #[test]
    fn the_device_gate_declines_a_custom_kind() {
        use crate::custom::{Precedence, ShapeSet};
        let mut shapes = ShapeSet::new();
        shapes.declare("order = `[A-Z]{3}-[0-9]{4}`", Precedence::Before).expect("declares");
        let pat = crate::parser::parse_with_shapes("\\{order}", &shapes).expect("parses");
        // The device lexes for itself with no shape set, so its stream holds
        // no such token; the gate must refuse rather than return empty.
        assert!(!crate::gpu::gpu_eligible(&pat));
        // A pattern of only built-in kinds is still eligible.
        assert!(crate::gpu::gpu_eligible(&parse("\\W \\N").expect("parses")));
    }

    #[test]
    fn a_shape_atom_is_unknown_without_its_set() {
        // The same pattern text with no shapes declared names nothing.
        assert!(parse("\\{order}").is_err());
    }

    /// A state's width, pinned, because the sweep carries millions of them.
    ///
    /// A scan holds states in vectors that a fan-out grows, so every byte here
    /// is multiplied by the state set and paid again at each growth. It is not
    /// a number to change without knowing: widening the rank from a thin
    /// pointer to a slice's fat one cost eight bytes and was worth it for
    /// halving an allocation, but that was weighed rather than discovered
    /// afterwards, and a later change should get to weigh it too.
    ///
    /// The parts are asserted beside the whole so a failure says which field
    /// moved rather than only that something did.
    #[test]
    fn a_state_is_the_width_the_sweep_was_tuned_for() {
        use std::mem::size_of;
        // Sixteen, not twenty-four: the heap arm's pointer cannot be null, and
        // that niche carries the discriminant, so the inline arm's seven bytes
        // and length sit in the room the fat pointer already took. Holding the
        // path inline therefore costs no width at all, which two readings of
        // this layout in prose got wrong in both directions before the compiler
        // was asked.
        assert_eq!(size_of::<Rank>(), 16, "the preference path: seven bytes inline, or a slice");
        assert_eq!(size_of::<Env>(), 8, "the register map, a thin pointer behind an Rc");
        assert_eq!(size_of::<Hist>(), 8, "the binding list, a thin pointer behind an Rc");
        assert_eq!(size_of::<State>(), 40, "a position and those three");
    }

    #[test]
    fn shape_precedence_decides_an_overlap_with_a_builtin() {
        use crate::custom::{Precedence, ShapeSet};
        let input = b"10.0.0.1";
        let mut before = ShapeSet::new();
        before.declare("quad = `[0-9.]{8}`", Precedence::Before).expect("declares");
        let toks = crate::lexer::lex_with_shapes(input, &[], &before, 0);
        assert_eq!(toks[0].kind, crate::token::TokenKind::Custom(0), "Before wins the overlap");

        let mut after = ShapeSet::new();
        after.declare("quad = `[0-9.]{8}`", Precedence::After).expect("declares");
        let toks = crate::lexer::lex_with_shapes(input, &[], &after, 0);
        assert_eq!(toks[0].kind, crate::token::TokenKind::Ip, "After leaves the IP reading");
    }

    #[test]
    fn an_orbit_scope_makes_literals_compare_under_a_symmetry() {
        // Identity is the default: byte equality.
        assert_eq!(run("\"Cat\"", "Cat cat CAT").len(), 1);
        // Case: the whole orbit of the literal matches.
        assert_eq!(run("(?orbit:case \"Cat\")", "Cat cat CAT").len(), 3);
        // The scope reaches every literal inside it, at any depth.
        assert_eq!(run("(?orbit:case (\"cat\" | \"dog\"))", "CAT Dog bird").len(), 2);
        // And leaves atoms that are not literals alone.
        assert_eq!(run("(?orbit:case \\N)", "12 ab").len(), 1);
    }

    #[test]
    fn an_orbit_scope_reaches_into_a_token_class() {
        assert_eq!(run("(?orbit:case [\"cat\" \"dog\"])", "CAT Dog bird").len(), 2);
    }

    #[test]
    fn a_bad_orbit_modifier_is_a_parse_error() {
        assert!(parse("(?bogus:case \"x\")").is_err());
        assert!(parse("(?orbit:nosuch \"x\")").is_err());
        assert!(parse("(?orbit \"x\")").is_err());
        // A plain group is untouched by the modifier syntax.
        assert!(parse("(\"x\" | \"y\")").is_ok());
    }

    #[test]
    fn lookahead_is_zero_width_and_both_polarities_work() {
        // `~(P)` consumes nothing: the following atom matches from the same
        // place, so the reported span is the word alone.
        let m = run("~(\\W) \\W", "cat 12 dog");
        assert_eq!(m.len(), 2);
        assert_eq!(&"cat 12 dog"[m[0].start..m[0].end], "cat");
        // A word that is followed by a number.
        let followed: Vec<_> = run("\\W ~(\\N)", "cat 12 dog 34 end")
            .iter()
            .map(|m| "cat 12 dog 34 end"[m.start..m.end].to_string())
            .collect();
        assert_eq!(followed, vec!["cat", "dog"]);
        // Negative: a word not followed by a number.
        let unfollowed: Vec<_> = run("\\W !~(\\N)", "cat 12 dog 34 end")
            .iter()
            .map(|m| "cat 12 dog 34 end"[m.start..m.end].to_string())
            .collect();
        assert_eq!(unfollowed, vec!["end"]);
    }

    #[test]
    fn lookbehind_reads_the_direction_the_matcher_never_exposed() {
        // A number preceded by a word.
        let after_word: Vec<_> = run("~<(\\W) \\N", "cat 12 34 dog 56")
            .iter()
            .map(|m| "cat 12 34 dog 56"[m.start..m.end].to_string())
            .collect();
        assert_eq!(after_word, vec!["12", "56"]);
        // A number not preceded by a word.
        let not_after_word: Vec<_> = run("!~<(\\W) \\N", "cat 12 34 dog 56")
            .iter()
            .map(|m| "cat 12 34 dog 56"[m.start..m.end].to_string())
            .collect();
        assert_eq!(not_after_word, vec!["34"]);
    }

    #[test]
    fn an_unbounded_backward_assertion_is_a_parse_error() {
        assert!(parse("~<(\\W*) \\N").is_err());
        assert!(parse("~<(\\W+) \\N").is_err());
        assert!(parse("~<(\\W{2,}) \\N").is_err());
        assert!(parse("~<(\\W{2,3}) \\N").is_ok());
    }

    #[test]
    fn the_literal_guard_still_works_and_stays_forward() {
        // The guard reads the window after the atom it follows, so `END`
        // itself does not match: nothing beyond it contains `END`.
        let present: Vec<_> = run(". ~\"END\"", "a b END c")
            .iter()
            .map(|m| "a b END c"[m.start..m.end].to_string())
            .collect();
        assert_eq!(present, vec!["a", "b"]);
        let absent: Vec<_> = run(". !~\"END\"", "a b END c")
            .iter()
            .map(|m| "a b END c"[m.start..m.end].to_string())
            .collect();
        assert_eq!(absent, vec!["END", "c"]);
        // The literal form has no backward reading.
        assert!(parse("~<\"END\"").is_err());
    }

    #[test]
    fn a_proximity_window_asks_within_how_many_tokens() {
        // Distance in tokens, which is the unit a token stream has. The
        // window counts significant tokens from the position outward, the
        // one under test counted first.
        // The window opens at the position the assertion holds at, which is
        // the token after the word the pattern consumed. So a word matches
        // when END is among the next k tokens, not counting itself.
        let hay = "alpha one two three END beta";
        let near: Vec<_> = run("\\W ~>2(\"END\")", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(near, vec!["two", "three"], "END is within two tokens after each");

        let wider: Vec<_> = run("\\W ~>3(\"END\")", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(wider, vec!["one", "two", "three"], "one more token of reach");
    }

    #[test]
    fn a_bounded_gap_between_two_tokens_spans_both_of_them() {
        // The subsequence question - these two in this order, anything
        // between, within so many tokens - asked by a bounded repeat of any
        // token. The match covers both ends and the gap, where a proximity
        // assertion consumes nothing and covers only the token it sits on.
        let hay = "alpha p q beta gamma alpha r s t u beta";
        let near: Vec<_> = run("\"alpha\" .{0,2} \"beta\"", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(near, vec!["alpha p q beta"], "the far pair has four tokens between");

        let wider: Vec<_> = run("\"alpha\" .{0,4} \"beta\"", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(wider.len(), 2, "both pairs reach at four: {wider:?}");
    }

    #[test]
    fn a_proximity_window_and_its_negation_partition_the_matches() {
        // Every token either has the target in its window or does not, so the
        // two readings together are what the bare pattern matches, with no
        // token in both.
        let hay = "a b c END d e f";
        let all = run("\\W", hay).len();
        let inside = run("\\W ~>2(\"END\")", hay).len();
        let outside = run("\\W !~>2(\"END\")", hay).len();
        assert_eq!(inside + outside, all, "{inside} within and {outside} not, of {all}");
        assert!(inside > 0 && outside > 0, "the corpus must exercise both sides");
    }

    #[test]
    fn a_window_can_be_asked_how_many_and_not_only_whether() {
        // Counting occurrences nearby is not a regular property, so no
        // regular expression asks it. Six words, then three numbers.
        // Each window opens on the token after the word, so e sees f 1 2 3.
        let hay = "a b c d e f 1 2 3";
        let three: Vec<_> = run("\\W ~>4{3,}(\\N)", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(three, vec!["e", "f"], "all three numbers are within four of each");

        let two: Vec<_> = run("\\W ~>4{2,}(\\N)", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(two, vec!["d", "e", "f"], "d reaches two of them");

        // A top as well as a bottom: at most one number in the window.
        let sparse: Vec<_> = run("\\W ~>4{0,1}(\\N)", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(sparse, vec!["a", "b", "c"], "d is the first to see two");
    }

    #[test]
    fn a_count_over_a_region_is_bounded_by_the_bracket_not_by_a_distance() {
        // "at least three numbers inside this group". The region is the group
        // opening at the position, so its extent is the input's and not a
        // number the pattern carries.
        let hay = "f(1, 2, 3) g(4, 5) h(6, 7, 8, 9)";
        let full: Vec<_> = run("\\W ~#{3,}(\\N) \\B", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(full, vec!["f(1, 2, 3)", "h(6, 7, 8, 9)"], "g holds two");

        // The same count over a window instead of a region, on an input where
        // the two disagree: the group holds two numbers and three more follow
        // it. A window reaches past the bracket and a region does not, which is
        // the whole of the difference between the two forms.
        let spill = "f(1, 2) 3 4 5";
        let by_region: Vec<_> = run("\\W ~#{3,}(\\N) \\B", spill)
            .iter()
            .map(|m| spill[m.start..m.end].to_string())
            .collect();
        assert!(by_region.is_empty(), "the region stops at `)`: {by_region:?}");

        let by_window: Vec<_> = run("\\W ~>9{3,}(\\N) \\B", spill)
            .iter()
            .map(|m| spill[m.start..m.end].to_string())
            .collect();
        assert_eq!(by_window, vec!["f(1, 2)"], "nine tokens of reach cross the bracket");
    }

    #[test]
    fn a_region_reaches_its_own_close_through_nesting() {
        // What puts this past a regular language: finding the region's end
        // means counting brackets. Stopping at the first `)` would read f's
        // region as `a(1, 2` and count two, so this input separates a matcher
        // that counts brackets from one that cannot.
        let hay = "f(a(1, 2), 3) g(b(1), 2)";
        let three: Vec<_> = run("\\W ~#{3,}(\\N) \\B", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(three, vec!["f(a(1, 2), 3)"], "only f's region holds three");

        // Nested tokens are inside the region, so the inner group's numbers
        // count toward the outer group's total.
        let two: Vec<_> = run("\\W ~#{2}(\\N) \\B", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(two, vec!["a(1, 2)", "g(b(1), 2)"], "exactly two, counting through nesting");
    }

    #[test]
    fn a_region_count_and_its_negation_partition_every_position() {
        // A position opening no group has an empty region, so its count is
        // zero rather than undefined. That is what keeps the two polarities
        // complements everywhere instead of only where a bracket stands.
        let hay = "f(1, 2) x y g(3)";
        let all = run("\\W", hay).len();
        let inside = run("\\W ~#{1,}(\\N)", hay).len();
        let outside = run("\\W !~#{1,}(\\N)", hay).len();
        assert_eq!(inside + outside, all, "{inside} over a region and {outside} not, of {all}");
        assert_eq!(inside, 2, "f and g open a group holding a number");
        assert_eq!(outside, 2, "x and y open no group at all");
    }

    #[test]
    fn a_region_count_satisfied_by_absence_does_not_make_its_literal_required() {
        // The same shortcut the window form can be wrong about. A region
        // asking for at most one of something is satisfied by none of it, so
        // its literal is not required, and treating it as required would
        // refuse an input, unscanned, that matches.
        let hay = "f(alpha) g(beta)";
        let found: Vec<_> = run("\\W ~#{0,1}(\"zzzqqq\") \\B", hay)
            .iter()
            .map(|m| hay[m.start..m.end].to_string())
            .collect();
        assert_eq!(found, vec!["f(alpha)", "g(beta)"], "every region holds at most one: none");
        assert!(run("\\W ~#{1,}(\"zzzqqq\") \\B", hay).is_empty(), "a floor above zero needs it");

        let none = parse("\\W ~#{0,1}(\"zzzqqq\")").expect("parses");
        let some = parse("\\W ~#{1,}(\"zzzqqq\")").expect("parses");
        assert!(!crate::prefilter::requires_absent(&none, hay.as_bytes()));
        assert!(crate::prefilter::requires_absent(&some, hay.as_bytes()));
    }

    #[test]
    fn a_region_count_keeps_the_pattern_off_the_prefix_path() {
        // A cut inside the group leaves the opening token with no mate, which
        // reads as an empty region and a count of zero - no match reported on
        // an input that has one. No token count describes the reach, so there
        // is no reserve to hold and the pattern declines the prefix instead.
        let region = parse("\\W ~#{1,}(\\N)").expect("parses");
        assert!(region.has_assert(), "the reach is the input's, not the pattern's");
        assert_eq!(region.widest_forward_window(), 0, "no token count describes it");
        assert!(!crate::prefilter::settles_from_a_prefix(&region));

        // Whatever route it takes, the answer is the scan's answer.
        let mut hay = String::new();
        for i in 0..40_000 {
            hay.push_str(&format!("key_{i} : {i} ;\n"));
        }
        hay.push_str("alpha ( 7 ) ;\n");
        assert_eq!(
            crate::find(&region, hay.as_bytes()),
            crate::scan(&region, hay.as_bytes()).first().copied()
        );
    }

    #[test]
    fn a_counting_selector_refuses_what_it_cannot_mean() {
        assert!(parse("\\W ~#{3,2}(\\N)").is_err(), "a top below its bottom");
        assert!(parse("\\W ~#{,2}(\\N)").is_err(), "the bottom is not optional");

        // A bare literal is the content guard, which asks whether the literal
        // is anywhere ahead - a wider question than either selector narrowed
        // to, so it is refused rather than silently answered.
        assert!(parse("\\W ~#{2,}\"END\"").is_err(), "the region form needs parentheses");
        assert!(parse("\\W ~>3\"END\"").is_err(), "the window form needs parentheses");
        assert!(parse("\\W ~\"END\"").is_ok(), "the plain guard is still the literal form");
    }

    #[test]
    fn a_prefix_does_not_settle_a_pattern_that_reads_past_its_match() {
        // A prefix truncates the input, so anything deciding a match from
        // outside the match's own span can be decided against a stream that
        // is not there. The reserve covers a bounded assertion's reach; an
        // unbounded one keeps the pattern off the prefix path entirely.
        let bounded = parse("\\W ~>3(\\N)").expect("parses");
        let unbounded = parse("\\W ~(\\N \\N)").expect("parses");
        let guarded = parse("\\W ~\"END\"").expect("parses");
        assert!(crate::prefilter::settles_from_a_prefix(&bounded));
        assert!(!crate::prefilter::settles_from_a_prefix(&unbounded));
        assert!(!crate::prefilter::settles_from_a_prefix(&guarded));

        // The reserve is the match's own length plus the assertion's reach,
        // so a cut cannot fall inside what the assertion is reading.
        assert_eq!(bounded.widest_forward_window(), 4, "three positions and a one-token inner");
        assert_eq!(unbounded.widest_forward_window(), 0, "no bounded window to reserve for");

        // Whatever route it takes, the answer is the scan's answer.
        let mut hay = String::new();
        for i in 0..40_000 {
            hay.push_str(&format!("key_{i} : {i} ;\n"));
        }
        hay.push_str("alpha 7 ;\n");
        for p in [&bounded, &unbounded, &guarded] {
            assert_eq!(crate::find(p, hay.as_bytes()), crate::scan(p, hay.as_bytes()).first().copied());
        }
    }

    #[test]
    fn a_count_satisfied_by_absence_does_not_make_its_literal_required() {
        // The prefilter refuses an input that cannot hold a literal every
        // match needs. A window asking for at most one of something is
        // satisfied by none of it, so its literal is not one of those - and
        // treating it as one would refuse an input that matches, which is the
        // false negative the prefilter exists never to produce.
        let hay = "alpha beta gamma delta";
        let found = run("\\W ~>4{0,1}(\"zzzqqq\")", hay);
        assert_eq!(found.len(), 4, "every word has at most one zzzqqq nearby: none");

        // A floor above zero does require it, and the input does not hold it.
        assert!(run("\\W ~>4{1,}(\"zzzqqq\")", hay).is_empty());

        // The same question asked of the prefilter directly, since that is
        // where the refusal would happen.
        let none = parse("\\W ~>4{0,1}(\"zzzqqq\")").expect("parses");
        let some = parse("\\W ~>4{1,}(\"zzzqqq\")").expect("parses");
        assert!(!crate::prefilter::requires_absent(&none, hay.as_bytes()));
        assert!(crate::prefilter::requires_absent(&some, hay.as_bytes()));
    }

    #[test]
    fn a_count_that_nothing_can_satisfy_is_refused() {
        assert!(parse("\\W ~>4{3,2}(\\N)").is_err(), "a top below its bottom");
        assert!(parse("\\W ~>2{5,}(\\N)").is_err(), "more occurrences than the window holds");
        assert!(parse("\\W ~>4{,2}(\\N)").is_err(), "the bottom is not optional");
    }

    #[test]
    fn a_window_of_zero_tokens_is_refused_rather_than_matched() {
        // A window that can hold nothing is a pattern that can never be
        // satisfied, which is worth a parse error rather than silence.
        assert!(parse("\\W ~>0(\"END\")").is_err());
        assert!(parse("\\W ~>(\"END\")").is_err(), "the count is not optional");
    }

    #[test]
    fn a_bounded_assertion_does_not_depend_on_the_whole_input() {
        // The reach is part of the pattern, so a chunked scanner holding that
        // many tokens past a match can finalize it. An unbounded one cannot.
        let bounded = parse("\\W ~>3(\"END\")").expect("parses");
        let unbounded = parse("\\W ~(\"END\")").expect("parses");
        assert!(!bounded.has_assert(), "a bounded assertion is not an unbounded one");
        assert!(unbounded.has_assert());
        assert!(!bounded.depends_on_whole_input(), "so it can be finalized from a chunk");
        assert!(unbounded.depends_on_whole_input());
    }

    #[test]
    fn an_assertion_binds_nothing_that_escapes_it() {
        // The probe's registers are discarded, so the template surface sees no
        // capture from inside an assertion.
        let p = parse("~(\\W:inner) \\W:outer").expect("parses");
        assert_eq!(p.capture_names(), vec!["outer".to_string()]);
    }

    #[test]
    fn token_classes_union_complement_intersect_and_subtract() {
        // Union: a number or a word, not the punctuation between them.
        assert_eq!(run("[\\N \\W]", "ab 12 , cd").len(), 3);
        // Complement: everything that is not a number.
        let not_num: Vec<_> = run("[^\\N]", "ab 12 cd")
            .iter()
            .map(|m| "ab 12 cd"[m.start..m.end].to_string())
            .collect();
        assert_eq!(not_num, vec!["ab", "cd"]);
        // Intersection: a word whose bytes are all hex.
        let hex: Vec<_> = run("[\\W && \\h]", "deadbeef zzz cafe")
            .iter()
            .map(|m| "deadbeef zzz cafe"[m.start..m.end].to_string())
            .collect();
        assert_eq!(hex, vec!["deadbeef", "cafe"]);
        // Difference: a word that is not all uppercase.
        let lower: Vec<_> = run("[\\W -- \\u]", "ABC def GHI jkl")
            .iter()
            .map(|m| "ABC def GHI jkl"[m.start..m.end].to_string())
            .collect();
        assert_eq!(lower, vec!["def", "jkl"]);
    }

    #[test]
    fn a_class_composes_with_literals_and_byte_patterns() {
        assert_eq!(run("[\"cat\" \"dog\"]", "cat bird dog").len(), 2);
        assert_eq!(run("[`[a-z]+` \\N]", "abc DEF 12").len(), 2);
    }

    #[test]
    fn malformed_classes_are_parse_errors() {
        for bad in ["[", "[\\N", "[]", "[&& \\N]"] {
            assert!(parse(bad).is_err(), "should reject: {bad}");
        }
    }

    #[test]
    fn a_balanced_square_group_still_parses_after_classes() {
        // `\B[...]` consumes its own bracket, so a bare `[` becoming a class
        // opener must not disturb it.
        assert_eq!(run("\\B[.*]", "x [a b] y").len(), 1);
    }

    #[test]
    fn input_anchors_are_distinct_from_line_anchors() {
        // Every anchor here is a prefix: it constrains the token the following
        // atom consumes, so `\z \W` reads "a word that ends the input".
        let two_lines = "alpha beta\ngamma delta\n";
        // `^` heads every line; `\A` heads only the stream.
        assert_eq!(run("^ \\W", two_lines).len(), 2);
        let at_start = run("\\A \\W", two_lines);
        assert_eq!(at_start.len(), 1);
        assert_eq!(&two_lines[at_start[0].start..at_start[0].end], "alpha");
        // `$` ends every line; `\z` ends only the stream.
        assert_eq!(run("$ \\W", two_lines).len(), 2);
        let at_end = run("\\z \\W", two_lines);
        assert_eq!(at_end.len(), 1);
        assert_eq!(&two_lines[at_end[0].start..at_end[0].end], "delta");
        // On a single line the two readings coincide.
        assert_eq!(run("^ \\W", "only line\n").len(), run("\\A \\W", "only line\n").len());
        assert_eq!(run("$ \\W", "only line\n").len(), run("\\z \\W", "only line\n").len());
    }

    #[test]
    fn resume_anchor_takes_a_contiguous_run_instead_of_finding_occurrences() {
        // The difference between tokenizing and searching. Plain `\N` finds
        // every number wherever it sits; `\G \N` takes numbers only while
        // they keep coming and stops at the word.
        let input = "1 2 3 stop 4 5";
        assert_eq!(run("\\N", input).len(), 5, "a search finds all five");
        let contiguous = run("\\G \\N", input);
        assert_eq!(contiguous.len(), 3, "the run ends at the word");
        assert_eq!(&input[contiguous[2].start..contiguous[2].end], "3");

        // It has to start at the beginning, so a stream that opens with a
        // non-match yields nothing at all rather than skipping ahead.
        assert!(run("\\G \\N", "stop 1 2 3").is_empty(), "no run to take");
        assert_eq!(run("\\N", "stop 1 2 3").len(), 3, "and searching still finds them");
    }

    #[test]
    fn resume_anchor_is_refused_where_it_could_only_fail() {
        // Anywhere but the head it asks whether an interior token is where
        // the previous match ended, which a non-overlapping run never makes
        // true, so it is a parse error rather than a silent never-match.
        assert!(crate::parser::parse("\\G \\N").is_ok(), "the head is where it belongs");
        for bad in ["\\N \\G", "\\N (\\G \\W)", "(\\W | \\G \\N)", "\\G \\N \\G"] {
            let err = crate::parser::parse(bad).expect_err("refused: {bad}");
            assert!(format!("{err:?}").contains("\\\\G"), "names the construct: {err:?}");
        }
    }

    #[test]
    fn reset_start_reports_only_what_follows_it() {
        // A lookbehind with no width limit: the key and colon are required
        // and not reported.
        let input = "name: alice age: bob";
        let got = run("\\W \":\" \\K \\W", input);
        assert_eq!(got.len(), 2);
        assert_eq!(&input[got[0].start..got[0].end], "alice");
        assert_eq!(&input[got[1].start..got[1].end], "bob");
        // Without it the whole construct is reported.
        let whole = run("\\W \":\" \\W", input);
        assert_eq!(&input[whole[0].start..whole[0].end], "name: alice");

        // The scan still resumes past what was matched, not past what was
        // reported, so a run does not re-read the hidden part.
        assert_eq!(got.len(), whole.len(), "same matches, different spans");
    }

    #[test]
    fn reset_start_is_refused_where_the_engine_cannot_carry_it() {
        assert!(crate::parser::parse("\\W \":\" \\K \\W").is_ok());
        // Each of these routes to the set engine, whose match start is the
        // position the attempt was anchored at and cannot be moved.
        for bad in ["\\B( \\K \\W )", "@seam \\K \\W", "\\K \\S", "~(\\W) \\K \\W"] {
            assert!(crate::parser::parse(bad).is_err(), "refused: {bad}");
        }
    }

    #[test]
    fn an_atomic_group_keeps_only_the_length_its_body_preferred() {
        // The whole of what atomic means: the star takes every word, the cut
        // discards the shorter lengths, and the atom after it is offered
        // nothing. Without the cut the star hands one back.
        let input = "a b c";
        assert_eq!(run("\\W* \\W", input).len(), 1, "greedy hands one back");
        assert!(run("(?>\\W*) \\W", input).is_empty(), "atomic does not");
        assert!(run("\\W*+ \\W", input).is_empty(), "and the possessive form is the same");

        // With something the body cannot take, the cut leaves it there.
        assert_eq!(run("(?>\\N*) \\W", "1 2 end").len(), 1, "numbers stop at the word");
        // A cut over a body that had one length to give changes nothing.
        assert_eq!(run("(?>\\W) \\W", input).len(), 1);
    }

    #[test]
    fn possessive_quantifiers_read_as_the_atomic_form() {
        let atomic = crate::parser::parse("(?>\\W*)").expect("parses");
        let possessive = crate::parser::parse("\\W*+").expect("parses");
        assert_eq!(atomic, possessive, "the spellings agree");
        for (poss, group) in
            [("\\N++", "(?>\\N+)"), ("\\N?+", "(?>\\N?)"), ("\\N{2,4}+", "(?>\\N{2,4})")]
        {
            assert_eq!(
                crate::parser::parse(poss).expect("parses"),
                crate::parser::parse(group).expect("parses"),
                "{poss} is {group}"
            );
        }
        // Nothing skips whitespace before the `+`, so a spaced one is still a
        // punctuation literal rather than a possessive marker.
        let spaced = crate::parser::parse("\\W+ \"+\"").expect("parses");
        assert_ne!(spaced, crate::parser::parse("\\W++").expect("parses"));
    }

    #[test]
    fn an_atom_can_be_conditioned_on_the_construct_containing_it() {
        // The reading regex has no way to state, because it has no container.
        // The same token kind means different things by where it sits: a
        // number in an assignment is a value, a number in a call is an
        // argument, and nothing about the token itself separates them.
        // x = 41  lexes to one assign unit holding the number; f(42) to a
        // call unit for the name and a separate numeric unit, one bracket
        // deeper, holding the argument.
        let input = "x = 41\nf(42)\ny = 43\n";
        assert_eq!(run("\\N", input).len(), 3, "three numbers, taken plainly");

        let assigned = run("@super:assign \\N", input);
        assert_eq!(assigned.len(), 2, "two of them are values in a binding");
        assert_eq!(&input[assigned[0].start..assigned[0].end], "41");
        assert_eq!(&input[assigned[1].start..assigned[1].end], "43");

        let argument = run("@super:numeric \\N", input);
        assert_eq!(argument.len(), 1, "and one stands alone as an argument");
        assert_eq!(&input[argument[0].start..argument[0].end], "42");

        // The name being called is reachable the same way.
        let callee = run("@super:call \\W", input);
        assert_eq!(callee.len(), 1);
        assert_eq!(&input[callee[0].start..callee[0].end], "f");
    }

    #[test]
    fn the_construct_boundary_is_an_anchor_of_its_own() {
        // The upper-grain counterpart of @seam: where one construct ends and
        // the next begins, which is a structural fact rather than a
        // statistical one.
        // A key-value entry and a list, each several words wide, so heads are
        // a strict subset of words rather than coinciding with them.
        let input = "k: v\na, b, c\n";
        let heads = run("@super \\W", input);
        assert_eq!(heads.len(), 2, "one head per construct");
        for (got, want) in heads.iter().zip(["k", "a"]) {
            assert_eq!(&input[got.start..got.end], want);
        }
        // Without the anchor every word matches, not only the heads.
        assert_eq!(run("\\W", input).len(), 5, "five words, two of them heads");
    }

    #[test]
    fn a_seam_names_which_stream_has_to_stop_predicting_itself() {
        // The same input, three sequences, three different questions. A
        // byte-grain cut falls where the characters stop predicting each
        // other; a token-grain cut where the sequence of kinds does; a
        // supertoken-grain cut where the sequence of roles does.
        let input = "let x = 1 ; let y = 2 ; print x ; print y ;";
        let by_byte = run("@seam:byte \\W", input);
        let by_token = run("@seam:token \\W", input);
        let by_super = run("@seam:super \\W", input);
        // The unqualified spelling is one of the three rather than a fourth
        // reading, and it is the token grain.
        assert_eq!(
            run("@seam \\W", input).iter().map(|m| m.start).collect::<Vec<_>>(),
            by_token.iter().map(|m| m.start).collect::<Vec<_>>(),
            "an unqualified seam reads the token stream"
        );

        // Each grain finds something, and they are not the same set - a
        // qualified reading is a different reading, not a filter on one.
        assert!(!by_token.is_empty(), "the token stream is segmented");
        assert_ne!(
            by_byte.iter().map(|m| m.start).collect::<Vec<_>>(),
            by_token.iter().map(|m| m.start).collect::<Vec<_>>(),
            "byte and token grain disagree about where the breaks are"
        );
        // A supertoken cut can only land where a construct begins.
        let units = crate::supertoken::supertokens_from(&crate::lexer::lex(input.as_bytes()), input.as_bytes());
        let heads: Vec<usize> = units.iter().map(|u| u.start).collect();
        assert!(
            by_super.iter().all(|m| heads.contains(&m.start)),
            "a construct-grain seam keys to construct starts"
        );
    }

    #[test]
    fn only_the_grains_a_pattern_names_are_segmented() {
        use crate::ast::Grain;
        let p = crate::parser::parse("@seam:token \\W").expect("parses");
        assert!(p.has_seam_at(Grain::Token));
        assert!(!p.has_seam_at(Grain::Byte), "the byte stream is not segmented for this");
        assert!(!p.has_seam_at(Grain::Super));
        // The unqualified spelling is the token grain, and the byte stream is
        // segmented only where `@seam:byte` asks for it.
        let plain = crate::parser::parse("@seam \\W").expect("parses");
        assert!(plain.has_seam_at(Grain::Token));
        assert!(!plain.has_seam_at(Grain::Byte), "the bytes are segmented only when named");
        let bytes = crate::parser::parse("@seam:byte \\W").expect("parses");
        assert!(bytes.has_seam_at(Grain::Byte));
        assert!(!bytes.has_seam_at(Grain::Token));
    }

    #[test]
    fn the_observation_axis_takes_a_grain_the_same_way() {
        use crate::ast::Grain;
        let p = crate::parser::parse("@ambiguous:token \\W").expect("parses");
        assert!(p.has_observation_at(Grain::Token));
        assert!(!p.has_observation_at(Grain::Byte), "only the named stream is read");
        let plain = crate::parser::parse("@ambiguous \\W").expect("parses");
        assert!(plain.has_observation_at(Grain::Byte));
        assert!(!plain.has_observation_at(Grain::Token));

        // Every grain scans, and a contested point is a subset of the words,
        // so the anchor is selective rather than a no-op at any of them.
        let input = "let x = 1 ; print x ; let yy = 22 ; print yy ;";
        let all = run("\\W", input).len();
        for pat in ["@ambiguous \\W", "@ambiguous:token \\W", "@ambiguous:super \\W"] {
            assert!(run(pat, input).len() <= all, "{pat} selects from the words");
        }
        assert!(crate::parser::parse("@ambiguous:nonesuch \\W").is_err());
    }

    #[test]
    fn an_unknown_grain_is_refused() {
        assert!(crate::parser::parse("@seam:token \\W").is_ok());
        assert!(crate::parser::parse("@seam:super \\W").is_ok());
        assert!(crate::parser::parse("@seam:byte \\W").is_ok());
        let err = crate::parser::parse("@seam:nonesuch \\W").expect_err("refused");
        assert!(format!("{err:?}").contains("nonesuch"), "names it: {err:?}");
    }

    #[test]
    fn an_unknown_supertoken_role_is_refused() {
        assert!(crate::parser::parse("@super:call \\N").is_ok());
        assert!(crate::parser::parse("@super").is_ok());
        let err = crate::parser::parse("@super:nonesuch \\N").expect_err("refused");
        assert!(format!("{err:?}").contains("nonesuch"), "names it: {err:?}");
    }

    #[test]
    fn uuid_is_spelled_out_and_g_is_the_anchor() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(run("\\{uuid}", id).len(), 1, "the long spelling reads a uuid");
        // `\G` is the anchor now, so against a uuid it takes the run from the
        // start rather than naming the token kind.
        assert!(crate::parser::parse("\\G").is_ok());
    }

    #[test]
    fn lazy_quantifiers_prefer_the_shorter_match() {
        // Laziness shows only where several lengths match from one start. In
        // `\W*? \N` the start already fixes the length, so both leans agree;
        // a repeated terminator is what gives the quantifier a real choice.
        let input = "a q b q";
        let greedy = run(".* \"q\"", input);
        let lazy = run(".*? \"q\"", input);
        assert_eq!(greedy.len(), 1, "greedy runs to the last terminator");
        assert_eq!(&input[greedy[0].start..greedy[0].end], "a q b q");
        assert_eq!(lazy.len(), 2, "lazy stops at the first, then resumes");
        assert_eq!(&input[lazy[0].start..lazy[0].end], "a q");
        assert_eq!(&input[lazy[1].start..lazy[1].end], "b q");

        // A lazy optional prefers to match nothing, which needs both lengths
        // to succeed from the same start: leftmost outranks either lean, so
        // `\W?? \N` over `x 1` still takes the word rather than failing.
        let opt_in = "a b";
        let g_opt = run("\\W? \\W", opt_in);
        let l_opt = run("\\W?? \\W", opt_in);
        assert_eq!(&opt_in[g_opt[0].start..g_opt[0].end], "a b");
        assert_eq!(&opt_in[l_opt[0].start..l_opt[0].end], "a");
    }

    #[test]
    fn a_lazy_quantifier_over_a_branching_body_still_prefers_shorter() {
        // The body makes a choice per iteration, so this quantifier takes the
        // per-iteration rank encoding rather than the single-count one.
        let input = "a 1 q b 2 q";
        let greedy = run("(\\W | \\N)* \"q\"", input);
        let lazy = run("(\\W | \\N)*? \"q\"", input);
        assert_eq!(greedy.len(), 1);
        assert_eq!(&input[greedy[0].start..greedy[0].end], "a 1 q b 2 q");
        assert_eq!(lazy.len(), 2);
        assert_eq!(&input[lazy[0].start..lazy[0].end], "a 1 q");
        assert_eq!(&input[lazy[1].start..lazy[1].end], "b 2 q");
    }

    #[test]
    fn lazy_and_greedy_accept_the_same_inputs() {
        // Laziness changes the span reported, never whether there is one.
        for input in ["a b c 1", "1", "a 1 b 2", "no number here", ""] {
            assert_eq!(
                run("\\W* \\N", input).is_empty(),
                run("\\W*? \\N", input).is_empty(),
                "greedy and lazy must agree on acceptance for {input:?}"
            );
        }
    }

    #[test]
    fn anchors_route_by_what_they_read() {
        // A positional anchor is a function of the tokens, the bytes and the
        // position, so the single-pass engine evaluates it as an epsilon.
        for src in ["\\A \\W", "\\z \\W", "^ \\W", "$ \\W", "^ $ \\W"] {
            let p = parse(src).expect("parses");
            assert!(
                crate::nfa::scan_nfa(&p, b"a b\nc d\n").is_some(),
                "the single-pass engine must handle {src:?}"
            );
        }
        // A property anchor reads a field built once per scan, which only the
        // set engine carries, so it still routes away.
        for src in ["@seam \\W", "@nested>1 \\W", "@novel \\W", "@echoed \\W"] {
            let p = parse(src).expect("parses");
            assert!(
                crate::nfa::scan_nfa(&p, b"a b\nc d\n").is_none(),
                "the single-pass engine must decline {src:?}"
            );
        }
    }

    #[test]
    fn both_engines_agree_on_positional_anchors() {
        // Now that both run them, they can disagree, so this checks they do
        // not. One lex, both engines, over inputs where line and input
        // readings come apart.
        for src in ["\\A \\W", "\\z \\W", "^ \\W", "$ \\W", "^ $ \\W", "^ \\W | \\z \\N"] {
            let p = parse(src).expect("parses");
            for inp in [
                "",
                "a",
                "alpha beta\ngamma delta\n",
                "  indented\nplain\n",
                "solo\n",
                "a b c\n\n d e\n",
                "1\nx 2\n",
            ] {
                let bytes = inp.as_bytes();
                let nfa = crate::nfa::scan_nfa(&p, bytes).expect("the linear engine handles it");
                let set = scan_set_reachability(&p, bytes);
                assert_eq!(nfa, set, "engines differ on {src:?} over {inp:?}");
            }
        }
    }

    #[test]
    fn an_input_anchor_ignores_an_enclosing_group() {
        // `\A` asks about the stream, so it holds at a group that opens the
        // input and not at one further in.
        assert_eq!(run("\\A \\B(\\W)", "(a) x").len(), 1);
        assert!(run("\\A \\B(\\W)", "x (a)").is_empty());
        // Inside the group it still asks about the input, where the open
        // bracket is a significant token preceding the interior, so it cannot
        // hold there at all. A group-scoped reading would have matched.
        assert!(run("\\B(\\A \\W)", "(a) x").is_empty());
    }

    #[test]
    fn mac_keeps_its_named_atom_after_a_lost_its_letter() {
        let m = run("\\{mac}", "nic 01:23:45:67:89:ab up");
        assert_eq!(m.len(), 1);
        assert_eq!(&"nic 01:23:45:67:89:ab up"[m[0].start..m[0].end], "01:23:45:67:89:ab");
    }

    #[test]
    fn the_three_alternation_modes_differ_as_documented() {
        // `|` leftmost-first: the short branch is preferred, but yields when
        // the continuation contradicts it.
        assert_eq!(run("(\"a\" | \"a\" \"b\") \"c\"", "a b c").len(), 1);
        assert_eq!(run("\\W | \\W \\W", "a b").len(), 2);
        // `||` leftmost-longest: no preference, the longest overall wins.
        let longest = run("\\W || \\W \\W", "a b");
        assert_eq!(longest.len(), 1);
        assert_eq!(&"a b"[longest[0].start..longest[0].end], "a b");
        // `|>` committed: the first branch that matches wins outright, so the
        // contradicted continuation kills the match instead of falling back.
        assert!(run("(\"a\" |> \"a\" \"b\") \"c\"", "a b c").is_empty());
        assert_eq!(run("(\"a\" \"b\" |> \"a\") \"c\"", "a b c").len(), 1);
    }

    #[test]
    fn alternation_agrees_across_the_router_inside_a_balanced_group() {
        // A balanced group forces the set engine and requires the interior to
        // consume exactly to the close, so a committed short branch leaves a
        // token over and fails having never tried the branch that fits.
        assert_eq!(run("\\B(\\W | \\W \\W)", "(a b)").len(), 1);
        assert_eq!(run("\\B((\"a\" | \"a\" \"b\") \"c\")", "(a b c)").len(), 1);
        assert!(run("\\B(\\W |> \\W \\W)", "(a b)").is_empty());
    }

    #[test]
    fn mixing_alternation_kinds_at_one_level_is_a_parse_error() {
        let e = parse("\\N | \\W || \\Q").unwrap_err();
        assert!(e.msg.contains("mixed alternation kinds"), "got {:?}", e.msg);
        // Parenthesizing says which binds tighter, and parses.
        assert!(parse("\\N | (\\W || \\Q)").is_ok());
        assert!(parse("(\\N | \\W) || \\Q").is_ok());
    }

    #[test]
    fn a_bare_slash_is_still_a_literal() {
        // `/` was considered for committed choice and rejected: the close-tag
        // pattern needs it as punctuation.
        let hay = "<div>hi</div>";
        let m = run("<\\W:t>.*</=t>", hay);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].group("t", hay.as_bytes()), Some(b"div" as &[u8]));
    }

    #[test]
    fn explicit_whitespace_atoms_match() {
        // \S (the whitespace token kind) and \s (the space byte class) must
        // see the whitespace token the inter-atom skip would jump over. They
        // cannot start a pattern (matches anchor at significant tokens).
        assert_eq!(run(r"\W \S \W", "a b").len(), 1, "\\S between atoms");
        assert_eq!(run(r"\W \s \W", "a  b").len(), 1, "\\s between atoms");
        // Constraining: no whitespace between the atoms = no match.
        assert_eq!(run(r"\W \S \W \S \W", "a b").len(), 0);
    }

    #[test]
    fn lowercase_byte_class_atoms() {
        // \h hex, \a alpha, \u upper, \l lower each match a whole token whose
        // bytes all satisfy the class, regardless of the token's kind.
        let hex: Vec<_> = run("\\h", "deadbeef 123 xyz").iter().map(|m| m.end - m.start).collect();
        assert_eq!(hex.len(), 2, "deadbeef and 123 are all-hex; xyz is not");
        assert_eq!(run("\\u", "ABC def GHI").len(), 2);
        assert_eq!(run("\\l", "ABC def GHI").len(), 1);
        assert_eq!(run("\\a", "abc d3f ghi").iter().map(|m| m.end - m.start).sum::<usize>(), 6);
    }

    #[test]
    fn spectral_atom_parses_every_predicate_form() {
        for ok in ["\\F{entropy>0.8}", "\\F{entropy<0.3}", "\\F{period=4}", "\\F{period:line}", "\\F{texture:code}", "\\F{texture:prose}", "\\F{onset}"] {
            assert!(parse(ok).is_ok(), "should parse: {ok}");
        }
        for bad in ["\\F{bogus}", "\\F{entropy}", "\\F{texture:nope}", "\\F{period=x}"] {
            assert!(parse(bad).is_err(), "should reject: {bad}");
        }
    }

    #[test]
    fn spectral_texture_atom_discriminates_code_from_prose() {
        // A spectral atom routes to the set engine, which builds the field
        // once and threads it to the matcher. Code carries many
        // code-textured tokens; prose carries essentially none.
        let code = "fn add(a,b){let c=a+b;return c*2;} impl P{fn n(&self){self.x*self.x+self.y*self.y}}";
        let prose = "the quick brown fox jumps over the lazy dog near the old stone bridge in the cool air";
        let code_hits = run("\\F{texture:code}", code).len();
        let prose_hits = run("\\F{texture:code}", prose).len();
        assert!(code_hits > prose_hits, "code {code_hits} should exceed prose {prose_hits}");
        assert!(code_hits > 0, "code texture should match in code");
    }

    #[test]
    fn matches_number_then_word() {
        // A unit symbol one space after a number is that quantity's, so the
        // word this reads is one no unit table holds.
        let m = run("\\N \\W", "weight 12 items here");
        assert_eq!(m.len(), 1);
        assert_eq!(&"weight 12 items here"[m[0].start..m[0].end], "12 items");
        assert!(run("\\N \\W", "weight 12 kg here").is_empty(), "`12 kg` is one quantity token");
    }

    #[test]
    fn matched_tag_binds_and_checks() {
        let hay = "<div>hi</div>";
        let ok = run("<\\W:t>.*</=t>", hay);
        assert_eq!(ok.len(), 1);
        assert_eq!(&hay[ok[0].start..ok[0].end], "<div>hi</div>");
        assert_eq!(ok[0].group("t", hay.as_bytes()), Some(b"div" as &[u8]));

        let bad = run("<\\W:t>.*</=t>", "<div>hi</span>");
        assert!(bad.is_empty(), "mismatched tag must not match");
    }

    #[test]
    fn atom_bind_through_set_engine_captures() {
        // A balanced group forces the set-reachability engine; the leading
        // `\W:t` is a single-atom bind, which takes the batched fast path
        // inside that engine. The capture must still be exact.
        let hay = "tag (5) rest";
        let m = run("\\W:t \\B(\\N)", hay);
        assert_eq!(m.len(), 1);
        assert_eq!(&hay[m[0].start..m[0].end], "tag (5)");
        assert_eq!(m[0].group("t", hay.as_bytes()), Some(b"tag" as &[u8]));

        // Two atom binds: the second register interns to a distinct id, so
        // this also covers the multi-register environment in the set engine.
        let hay2 = "alpha beta (7)";
        let m2 = run("\\W:t \\W:u \\B(\\N)", hay2);
        assert_eq!(m2.len(), 1);
        assert_eq!(m2[0].group("t", hay2.as_bytes()), Some(b"alpha" as &[u8]));
        assert_eq!(m2[0].group("u", hay2.as_bytes()), Some(b"beta" as &[u8]));
    }

    #[test]
    fn repeated_token_matches_only_a_repeat() {
        let ok = run("\\W:x =x", "the the cat");
        assert_eq!(ok.len(), 1);
        assert_eq!(&"the the cat"[ok[0].start..ok[0].end], "the the");

        let none = run("\\W:x =x", "the cat sat");
        assert!(none.is_empty());
    }

    #[test]
    fn scoped_binding_does_not_leak_out_of_its_group() {
        // A global `:x` bound inside a group leaks out, so a following `=x`
        // matches the repeat outside the group.
        let global = run("\\B(\\W:x) =x", "(cat) cat");
        assert_eq!(global.len(), 1);
        assert_eq!(&"(cat) cat"[global[0].start..global[0].end], "(cat) cat");
        // A scoped `::x` is dropped when the group closes, so the outside `=x`
        // has no binding and cannot match.
        assert!(
            run("\\B(\\W::x) =x", "(cat) cat").is_empty(),
            "a scoped bind must not leak out of its group"
        );
        // Inside the group a scoped reference still resolves normally.
        assert_eq!(run("\\B(\\W::x =x)", "(the the)").len(), 1);
        assert!(run("\\B(\\W::x =x)", "(the cat)").is_empty());
    }

    #[test]
    fn balanced_group_handles_nesting() {
        let m = run("\\W\\B(.*)", "call f(g(x)) end");
        assert_eq!(m.len(), 1);
        assert_eq!(&"call f(g(x)) end"[m[0].start..m[0].end], "f(g(x))");

        let none = run("\\W\\B(.*)", "bare word");
        assert!(none.is_empty());
    }

    #[test]
    fn ordered_choice_matches_either() {
        let a = run("\\N | \\W", "12");
        assert_eq!(a.len(), 1);
        let b = run("\\N | \\W", "hi");
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn ordered_choice_is_committed() {
        // The first alternative (a single word) wins, so the match
        // is "a", not the longer "a b" the second alternative would
        // produce. Union semantics would take the longer span.
        let m = run("\\W | \\W \\W", "a b");
        assert_eq!(m.len(), 2);
        assert_eq!(&"a b"[m[0].start..m[0].end], "a");
    }

    #[test]
    fn guard_requires_forward_literal() {
        let hit = run(". ~\"END\"", "begin END");
        assert!(!hit.is_empty());
        let miss = run(". ~\"END\"", "begin only");
        assert!(miss.is_empty());
    }

    #[test]
    fn parallel_scan_over_large_input_is_correct() {
        // 2000 word tokens exceed PARALLEL_SCAN_THRESHOLD, so the
        // per-start attempts run across cores. The repeated-token
        // pattern pairs them up: 2000 words yield 1000 non-overlapping
        // matches, and the parallel result must equal that exactly.
        let input = vec!["x"; 2000].join(" ");
        let m = run("\\W:p =p", &input);
        assert_eq!(m.len(), 1000);
        assert_eq!(&input[m[0].start..m[0].end], "x x");
    }

    #[test]
    fn ambiguous_anchor_fires_at_contested_points() {
        // `@ambiguous` fires on tokens overlapping a vantage-dependent point.
        // A garden-path sentence has such points, and the anchor is selective:
        // it matches some tokens but not every one, unlike a bare `.`.
        let src = "the old man the boats";
        let garden = run("@ambiguous .", src);
        assert!(!garden.is_empty(), "a garden-path sentence has contested points");
        let all = run(".", src).len();
        assert!(garden.len() < all, "@ambiguous is selective, not every token");
    }

    #[test]
    fn nested_anchor_matches_by_depth() {
        // `@nested>1` fires only on tokens more than one bracket deep. In
        // `f(g(x))`, `x` sits two parens deep; nothing else does.
        let deep = run("@nested>1 .", "a f(g(x)) b");
        let got: Vec<&str> = deep.iter().map(|h| &"a f(g(x)) b"[h.start..h.end]).collect();
        assert_eq!(got, vec!["x"]);
        // `@nested>=1` fires on every token at least one bracket deep.
        let one = run("@nested>=1 .", "a (b c) d");
        let g1: Vec<&str> = one.iter().map(|h| &"a (b c) d"[h.start..h.end]).collect();
        assert_eq!(g1, vec!["b", "c"]);
    }

    #[test]
    fn seam_anchor_is_selective() {
        // `@seam` fires only where the seam strength is in the field's upper
        // tier - a genuine predictability break - not at every token start, so
        // it matches a strict, non-empty subset of the tokens a bare `.` takes.
        let src = "the the the the cat sat";
        let all = run(".", src).len();
        let seams = run("@seam .", src).len();
        assert!(all >= 5, "the bare-dot baseline should match every token");
        assert!(
            seams >= 1 && seams < all,
            "@seam should be selective: {seams} of {all} token(s), not none and not all"
        );
    }

    #[test]
    fn a_strain_anchor_takes_the_units_above_its_percentile() {
        // A stream that repeats one word, with a word of unseen bytes once in
        // the middle: its first byte is the most strained in the input.
        let src = format!("{}zqxj {}", "abcd ".repeat(400), "abcd ".repeat(400));
        let all = run(".", &src).len();
        let high = run("@strain:byte>99.5 .", &src);
        let got: Vec<&str> = high.iter().map(|m| &src[m.start..m.end]).collect();
        assert!(got.contains(&"zqxj"), "the unseen word is among the most strained: {got:?}");
        assert!(high.len() < all / 20, "a high percentile is selective: {} of {all}", high.len());
        assert!(run("@strain:byte>1000b .", &src).is_empty(), "no byte is strained a thousand bits");
    }

    #[test]
    fn a_bound_anchor_takes_the_weakest_cuts() {
        // Two runs of different bytes: the cut between them binds least.
        let src = format!("{}{}", "ab ab ".repeat(300), "xy xy ".repeat(300));
        let first_x = src.find('x').expect("the second run");
        let weakest = run("@bound:byte<=0.5 .", &src);
        assert!(
            weakest.iter().any(|m| m.start == first_x),
            "the seam between the runs is among the weakest cuts: {:?}",
            weakest.iter().map(|m| m.start).collect::<Vec<_>>()
        );
        let all = run(".", &src).len();
        assert!(weakest.len() < all / 10, "a low percentile is selective: {} of {all}", weakest.len());
    }

    #[test]
    fn a_kin_anchor_takes_its_example_and_what_the_field_places_with_it() {
        let src = "alpha beta; gamma(delta)\n".repeat(200);
        let a_words = run("@kin:byte(\"a\") .", &src);
        let got: Vec<&str> = a_words.iter().map(|m| &src[m.start..m.end]).collect();
        assert!(got.contains(&"alpha"), "a token opening with the example's byte is kin to it: {got:?}");
        assert!(run("@kin:byte(\"~\") .", &src).is_empty(), "an example the input never holds matches nothing");
    }

    #[test]
    fn a_kin_reference_takes_a_token_the_field_places_with_the_bound_one() {
        use crate::ast::{Atom, Grain, Pattern};
        assert_eq!(crate::parse("=kin a").expect("parses"), Pattern::Atom(Atom::RegisterKin("a".into(), Grain::Token)));
        assert_eq!(
            crate::parse("=kin:super a").expect("parses"),
            Pattern::Atom(Atom::RegisterKin("a".into(), Grain::Super))
        );
        assert!(
            matches!(crate::parse("=kin").expect("parses"), Pattern::Atom(Atom::RegisterEq(name, _)) if name == "kin"),
            "with no register after it, kin is a register's name"
        );
        // Every word is one type at the token grain, so a word is kin to a word.
        assert_eq!(run("(\\W):a =kin a", "x y").len(), 1);
        // Too little input places no class, so only the same type is kin.
        assert!(run("(\\W):a =kin a", "x ;").is_empty(), "a word and a semicolon are not kin");
    }

    #[test]
    fn a_phase_anchor_names_a_period_by_length_or_by_rank() {
        use crate::ast::{AnchorKind, Pattern, PeriodRef};
        assert_eq!(crate::parse("@phase:2/6").expect("parses"), Pattern::Anchor(AnchorKind::PhaseIn(2, PeriodRef::Length(6))));
        assert_eq!(crate::parse("@phase:1#2").expect("parses"), Pattern::Anchor(AnchorKind::PhaseIn(1, PeriodRef::Rank(2))));
        assert!(crate::parse("@phase:6/6").is_err(), "a column lies below its period");
        assert!(crate::parse("@phase:0/40").is_err(), "a period lies within the lags searched");
        assert!(crate::parse("@phase:0#0").is_err(), "the strongest period is #1");
        assert!(crate::parse("@phase:40").is_err(), "no column lies past the longest period");
        // Rows of two shapes, four tokens and six: both periods are live.
        let src = format!("{}{}", "k = 1 ;\n".repeat(150), "k = 1 , 2 ;\n".repeat(150));
        assert!(!run("@phase:0/4 .", &src).is_empty(), "column 0 of the 4-token period holds");
        assert!(!run("@phase:0/6 .", &src).is_empty(), "column 0 of the 6-token period holds");
        assert!(run("@phase:0/5 .", &src).is_empty(), "no 5-token period lives here");
        assert_eq!(
            run("@phase:0#1 .", &src).len(),
            run("@phase:0 .", &src).len(),
            "#1 is the strongest period, the one plain @phase counts in"
        );
    }

    #[test]
    fn the_gravity_anchors_parse_their_thresholds_and_refuse_what_names_nothing() {
        use crate::ast::{AnchorKind, Cmp, Grain, GravityReading, Level, Pattern, Real};
        let p = crate::parse("@strain>90").expect("a percentile");
        assert_eq!(
            p,
            Pattern::Anchor(AnchorKind::Gravity(
                GravityReading::Strain,
                Grain::Token,
                Cmp::Gt,
                Level::Percentile(Real::new(90.0))
            ))
        );
        let p = crate::parse("@bound:super<=-1.5b").expect("a negative value in bits");
        assert_eq!(
            p,
            Pattern::Anchor(AnchorKind::Gravity(GravityReading::Bound, Grain::Super, Cmp::Le, Level::Bits(Real::new(-1.5))))
        );
        assert!(crate::parse("@strain").is_err(), "a comparison is required");
        assert!(crate::parse("@strain>150").is_err(), "a percentile runs 0 to 100");
        assert!(crate::parse("@strain>150b").is_ok(), "bits are not a percentile");
        assert!(crate::parse("@strain:bytes>1").is_err(), "an unknown grain is refused");
        assert!(crate::parse("@kin:byte(\"ab\")").is_err(), "a byte-grain example is one byte");
        assert!(crate::parse("@kin(\"   \")").is_err(), "an example of whitespace names no token");
    }

    #[test]
    fn silhouette_matches_by_structural_form() {
        // `#"W(W,W)"` matches a two-argument call whatever the identifiers
        // are, and rejects a one-arg or three-arg call: structure, not content.
        let src = "foo(a,b) and bar(x,y) but baz(1) and qux(p,q,r)";
        let m = run("#\"W(W,W)\"", src);
        let got: Vec<&str> = m.iter().map(|h| &src[h.start..h.end]).collect();
        assert_eq!(got, vec!["foo(a,b)", "bar(x,y)"]);
    }

    #[test]
    fn negative_guard_requires_absence() {
        // `!~"lit"` keeps a match only when its forward window does not
        // contain the literal - negative lookahead that stays linear.
        let src = "run 7 fail 9 pass";
        let neg = run("\\N !~\"fail\"", src);
        let got: Vec<&str> = neg.iter().map(|h| &src[h.start..h.end]).collect();
        assert_eq!(got, vec!["9"], "only the number with no 'fail' after it");
        // The positive guard is unchanged: a number followed by 'fail'.
        let pos = run("\\N ~\"fail\"", src);
        let gotp: Vec<&str> = pos.iter().map(|h| &src[h.start..h.end]).collect();
        assert_eq!(gotp, vec!["7"]);
    }

    #[test]
    fn shape_backreference_is_a_fuzzy_repeat() {
        // `=shape x` matches any later token in the bound token's shape orbit,
        // not just an exact repeat: `cat dog` and `pin bad` are CVC CVC pairs.
        let src = "cat dog and pin bad the sky";
        let m = run("\\W:x =shape x", src);
        let got: Vec<&str> = m.iter().map(|h| &src[h.start..h.end]).collect();
        assert_eq!(got, vec!["cat dog", "pin bad"]);
        // The plain reference stays an exact repeat (no orbit widening).
        let exact = run("\\W:x =x", src);
        assert!(exact.is_empty(), "no adjacent exact word repeat in {src:?}");
    }

    #[test]
    fn case_backreference_matches_across_case() {
        let src = "Foo foo bar BAZ baz";
        let m = run("\\W:x =case x", src);
        let got: Vec<&str> = m.iter().map(|h| &src[h.start..h.end]).collect();
        assert_eq!(got, vec!["Foo foo", "BAZ baz"]);
    }

    #[test]
    fn magnitude_atom_parses_every_predicate_form() {
        for ok in ["\\M{>6}", "\\M{>=6}", "\\M{<3}", "\\M{<=3}", "\\M{mag>6}", "\\M{ mag >= 9 }"] {
            assert!(parse(ok).is_ok(), "should parse: {ok}");
        }
        for bad in ["\\M{}", "\\M{6}", "\\M{>x}", "\\M{bogus}"] {
            assert!(parse(bad).is_err(), "should reject: {bad}");
        }
    }

    #[test]
    fn magnitude_matches_numbers_by_scale() {
        // The scale substrate: `5` and `5000000000` are indistinguishable to
        // every other axis (both Number tokens), but the magnitude predicate
        // separates them - the capability regex structurally lacks.
        let src = "a 5 b 5000000000 c 42 d 999999999999";
        let m = run("\\M{>6}", src);
        let got: Vec<&str> = m.iter().map(|h| &src[h.start..h.end]).collect();
        assert_eq!(got, vec!["5000000000", "999999999999"]);
    }

    #[test]
    fn kind_magnitude_intersects_kind_and_scale() {
        // `\N{mag>6}` is a Number and magnitude > 6. The big number matches; a
        // word long enough to have magnitude > 6 (len > 64) does not, because
        // the kind must be Number - where kind-blind `\M{>6}` would take it.
        let longword = "a".repeat(70);
        let src = format!("5000000000 42 {longword}");
        let n: Vec<String> =
            run("\\N{mag>6}", &src).iter().map(|h| src[h.start..h.end].to_string()).collect();
        assert_eq!(n, vec!["5000000000".to_string()]);
        // `\M{>6}` is kind-blind, so it also takes the long word.
        let m: Vec<String> =
            run("\\M{>6}", &src).iter().map(|h| src[h.start..h.end].to_string()).collect();
        assert!(m.contains(&longword), "\\M{{>6}} is kind-blind and takes the long word");
        // The repeat form on a kind atom is unchanged: `\N{2}` is two numbers.
        let rep = run("\\N{2}", "a 1 2 3 b");
        assert_eq!(&"a 1 2 3 b"[rep[0].start..rep[0].end], "1 2");
    }

    #[test]
    fn magnitude_le_matches_small_tokens() {
        // A low-magnitude predicate keeps the small numbers and short words
        // and drops the huge value.
        let m = run("\\M{<3}", "tiny 5 huge 5000000000 mid 900");
        assert!(m.iter().all(|h| &"tiny 5 huge 5000000000 mid 900"[h.start..h.end] != "5000000000"));
        assert!(m.iter().any(|h| &"tiny 5 huge 5000000000 mid 900"[h.start..h.end] == "900"));
    }

    #[test]
    fn kv_lens_matches_both_separators() {
        // `@kv` generalizes assignment to `:` and `=`; a word with no
        // separator after it is not a key/value pair.
        let m = run("@kv", "name: value  x=5");
        let got: Vec<&str> = m.iter().map(|h| &"name: value  x=5"[h.start..h.end]).collect();
        assert_eq!(got, vec!["name:", "x="]);
    }

    #[test]
    fn flag_lens_matches_short_and_long() {
        // `-x` and `--verbose` are flags; `-5` is a dash then a number, not a
        // flag (the word requirement rejects it).
        let m = run("@flag", "run -x --verbose -5 end");
        let got: Vec<&str> = m.iter().map(|h| &"run -x --verbose -5 end"[h.start..h.end]).collect();
        assert_eq!(got, vec!["-x", "--verbose"]);
    }

    #[test]
    fn list_lens_requires_a_comma() {
        // A comma-separated run is a list; a bare word is not.
        let m = run("@list", "items a, b, c but lone stands");
        assert_eq!(m.len(), 1);
        assert_eq!(&"items a, b, c but lone stands"[m[0].start..m[0].end], "a, b, c");
    }

    #[test]
    fn range_lens_joins_numbers_not_clocks() {
        // `..`, `-`, and `:` all join two numbers; a `HH:MM` clock lexes as a
        // single timestamp token, so it is not a range.
        let src = "span 1..10 and 3-7 and 2:9 but 12:30 clock";
        let m = run("@range", src);
        let got: Vec<&str> = m.iter().map(|h| &src[h.start..h.end]).collect();
        assert_eq!(got, vec!["1..10", "3-7", "2:9"]);
    }

    #[test]
    fn field_addressing_anchors_to_csv_field() {
        let m = run("@3 \"ERROR\"", "a, b, ERROR");
        assert_eq!(m.len(), 1);
        assert_eq!(&"a, b, ERROR"[m[0].start..m[0].end], "ERROR");
        // Field 3 holds OK, not ERROR, so nothing matches.
        assert!(run("@3 \"ERROR\"", "a, b, OK").is_empty());
        // @2 selects the second field's word.
        let m2 = run("@2 \\W", "a, hello, c");
        assert_eq!(m2.len(), 1);
        assert_eq!(&"a, hello, c"[m2[0].start..m2[0].end], "hello");
    }

    /// `^` selects the token that leads its line, which separates a word
    /// being used from the same word mentioned later in the line.
    #[test]
    fn line_start_anchor_selects_only_line_leading_tokens() {
        let input = "cat file\nrun cat\ncat again";
        let m = run(r#"^ "cat""#, input);
        assert_eq!(m.len(), 2, "two lines LEAD with cat, one merely mentions it");
        assert_eq!(m[0].start, 0);
        assert_eq!(&input[m[1].start..m[1].end], "cat");
        assert!(m[1].start > input.find("run").unwrap(), "the third line's cat");

        // Indentation does not stop a token leading its line.
        assert_eq!(run(r#"^ "cat""#, "   \n\t cat x").len(), 1);
        // Without the anchor every occurrence matches.
        assert_eq!(run(r#""cat""#, input).len(), 3);
        // A token that never leads a line is never selected.
        assert_eq!(run(r#"^ "cat""#, "run cat here").len(), 0);
    }

    /// `$` is the mirror: the token that ends its line.
    #[test]
    fn line_end_anchor_selects_only_line_trailing_tokens() {
        let input = "run cat\ncat file\nx cat";
        let m = run(r#"$ "cat""#, input);
        assert_eq!(m.len(), 2, "two lines END with cat");
        assert_eq!(run(r#"$ "cat""#, "cat trailing spaces   ").len(), 0);
        assert_eq!(run(r#"$ "cat""#, "ends with cat   ").len(), 1, "trailing space still ends the line");
    }

    /// Composed with alternation, the anchors express "at a position where
    /// a command may begin" for line-oriented input, without the pattern
    /// language needing to know any particular shell's syntax.
    #[test]
    fn anchors_compose_with_alternation_into_a_position_class() {
        let pat = r#"(^ | "|" | ";") "cat""#;
        assert_eq!(run(pat, "cat x").len(), 1, "start of input");
        assert_eq!(run(pat, "a | cat x").len(), 1, "after a pipe");
        assert_eq!(run(pat, "a ; cat x").len(), 1, "after a separator");
        assert_eq!(run(pat, "echo the cat sat").len(), 0, "mid-line mention");
    }

    /// Both anchors hold at once for a token that is alone on its line.
    #[test]
    fn a_token_alone_on_its_line_both_leads_and_ends_it() {
        assert_eq!(run(r#"^ $ "cat""#, "x\ncat\ny").len(), 1);
        assert_eq!(run(r#"^ $ "cat""#, "x\ncat y\nz").len(), 0);
    }

    /// Byte spans of a match list, for comparing one engine against another
    /// without asserting on the registers each happened to bind.
    fn byte_spans(ms: &[Span]) -> Vec<(usize, usize)> {
        ms.iter().map(|m| (m.start(), m.end())).collect()
    }

    /// A greedy repetition over a lazily quantified body reaches a position
    /// twice: once inside a single long body match, and again as two short
    /// ones. The second is the derivation leftmost-first prefers, so the
    /// repetition must keep it rather than whichever iteration arrived first.
    #[test]
    fn a_greedy_loop_over_a_lazy_body_keeps_the_preferred_derivation() {
        let p = parse("(.+?)+").unwrap();
        let input = "956 116 bar";
        let set = scan(&p, input.as_bytes());
        let single = crate::nfa::scan_nfa(&p, input.as_bytes())
            .expect("the single-pass engine takes this pattern");
        assert_eq!(byte_spans(&set), byte_spans(&single), "the two engines rank this alike");
        assert_eq!(set.len(), 1, "one match spanning the input, not one per token");
    }

    /// Taking a greedy option and letting a nullable body match empty
    /// outranks declining the option, because continuing sorts below stopping
    /// where the two differ. Each match is then the single token the leading
    /// atom consumes.
    #[test]
    fn a_greedy_option_prefers_a_nullable_body_matching_empty() {
        let p = parse(". (.{0,2}?)?").unwrap();
        let input = "488 foo 786 qux ";
        let set = scan(&p, input.as_bytes());
        let single = crate::nfa::scan_nfa(&p, input.as_bytes())
            .expect("the single-pass engine takes this pattern");
        assert_eq!(byte_spans(&set), byte_spans(&single), "the two engines rank this alike");
        assert_eq!(set.len(), 4, "one match per token, not one per two");
    }
}
