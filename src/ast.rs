//! The pattern AST: the intermediate form the parser emits and
//! the engine consumes.
//!
//! Every construct in the surface language (the wiki's pattern syntax
//! reference) has a node here. The engine matches by folding the AST over
//! the token stream; the parser is the only producer.

use crate::orbit::OrbitGroup;
use crate::token::{BracketKind, TokenKind};

/// A byte-grain character class, used when a pattern drops below the token
/// grain to constrain a token's bytes (`\d`, `\w`, `\s`, `\h`, `\a`, `\u`,
/// `\l`). Uppercase escapes are whole typed tokens; these lowercase ones are
/// the regex-style byte classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteClass {
    /// `\d`: every byte is an ASCII digit.
    Digit,
    /// `\w`: every byte is an ASCII word byte (alphanumeric or `_`).
    Word,
    /// `\s`: every byte is ASCII whitespace.
    Space,
    /// `\h`: every byte is an ASCII hex digit (`0-9a-fA-F`).
    Hex,
    /// `\a`: every byte is an ASCII letter.
    Alpha,
    /// `\u`: every byte is an ASCII uppercase letter.
    Upper,
    /// `\l`: every byte is an ASCII lowercase letter.
    Lower,
}

/// How a reading compares against the number written beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl Cmp {
    /// Whether an ordering of the reading against the written number
    /// satisfies this comparison.
    #[must_use]
    pub fn holds(self, o: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering;
        match self {
            Cmp::Lt => o == Ordering::Less,
            Cmp::Le => o != Ordering::Greater,
            Cmp::Gt => o == Ordering::Greater,
            Cmp::Ge => o != Ordering::Less,
            Cmp::Eq => o == Ordering::Equal,
            Cmp::Ne => o != Ordering::Equal,
        }
    }

    /// The surface operator, for diagnostics.
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
            Cmp::Eq => "=",
            Cmp::Ne => "!=",
        }
    }
}

/// A reading of the echo axis at one token: how often its content recurs in
/// the input, which occurrence this one is, and whether the recurrence is
/// regularly spaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EchoPred {
    /// `@echo>5`: occurrences of this token's content in the input, one for
    /// a token whose content appears once.
    Count(Cmp, u32),
    /// `@echo:nth=3`: this occurrence's place among them, counting from one,
    /// or from the last backwards where the index is negative. Never zero.
    Nth(Cmp, i32),
    /// `@echo:period`: the occurrences are regularly spaced.
    Period,
    /// `@echo:period=k`: regularly spaced, at `k` bytes to the nearest byte.
    PeriodAt(Cmp, u32),
}

impl AnchorKind {
    /// Whether the anchor reads the input beyond the bytes beside one
    /// token, so a chunked scanner cannot commit a match under it before
    /// the input ends: the input's ends, which a retained buffer would
    /// misjudge at its own; and every field built over the whole stream.
    /// A line anchor reads to the nearest newline, and a join reads the
    /// other input, so neither does.
    #[must_use]
    pub fn reads_whole_input(&self) -> bool {
        !matches!(
            self,
            AnchorKind::LineStart
                | AnchorKind::LineEnd
                | AnchorKind::Resume
                | AnchorKind::ResetStart
                | AnchorKind::Joined { .. }
        )
    }
}

/// Which way a timestamp stands against the one before it in the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeOrder {
    /// At or after it: the records run forward.
    Asc,
    /// Before it: the records run backward, a clock skew or an out-of-order
    /// record.
    Desc,
}

impl TimeOrder {
    /// The direction a name denotes.
    #[must_use]
    pub fn parse(name: &str) -> Option<TimeOrder> {
        match name {
            "asc" => Some(TimeOrder::Asc),
            "desc" => Some(TimeOrder::Desc),
            _ => None,
        }
    }

    /// The name it is written under.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            TimeOrder::Asc => "asc",
            TimeOrder::Desc => "desc",
        }
    }
}

/// A texture class a spectral predicate can match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecTexture {
    /// Natural-language prose.
    Prose,
    /// Source code.
    Code,
    /// Mathematics.
    Math,
    /// Compressed / packed / binary data.
    Data,
}

/// A spectral-axis predicate over a token's pooled field signature
/// (`\F{...}`): the temporal substrate read as a match condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpectralPred {
    /// Pooled entropy (x100) is at least this (high-entropy / packed).
    EntropyGe(u8),
    /// Pooled entropy (x100) is at most this (plain / low-information).
    EntropyLe(u8),
    /// The dominant byte-period equals this many bytes.
    PeriodEq(u16),
    /// Any dominant byte-period was detected.
    PeriodAny,
    /// The pooled texture class matches.
    Texture(SpecTexture),
    /// A change-point lies inside the token's span.
    Onset,
}

/// A magnitude-axis predicate over a token's order of magnitude
/// (`\M{...}`): the scale substrate read as a match condition. A
/// Number's magnitude is `log10(|value|)`, any other token's is
/// `log2(byte length)`, so a threshold of `6` is a numeric value over
/// ~1e6 or a token over ~64 bytes. The threshold is stored as the
/// magnitude times 100, an integer, so [`Atom`] keeps deriving `Eq` and
/// `Hash` (a raw `f32` implements neither), the same fixed-point trick
/// [`SpectralPred::EntropyGe`] uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MagPred {
    /// The token's magnitude (x100) is at least this: `\M{>6}` / `\M{>=6}`.
    Ge(i32),
    /// The token's magnitude (x100) is at most this: `\M{<3}` / `\M{<=3}`.
    Le(i32),
    /// The token's magnitude is at least the mean magnitude of a context
    /// plus a delta: `\N{>+1}` is an order of magnitude above the mean of
    /// the rolling window before the token, `\N{>+2s}` two of the window's
    /// standard deviations above it, `\N{>+1:phase}` an order above the
    /// token's column, `\N{>+1:k}` an order above the values bound to
    /// earlier occurrences of the key register `k` holds. A threshold read
    /// from the stream rather than written into the pattern, which a regular
    /// expression has no way to state.
    Above(Scope, Delta),
    /// The token's magnitude is at most a context's mean plus a delta:
    /// `\N{<-1}` is an order of magnitude below the window's mean.
    Below(Scope, Delta),
}

/// The context a relative magnitude predicate takes its baseline from.
///
/// Each is a fold the rolling context keeps at every token
/// ([`crate::context`]); the predicate reads its magnitude and compares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The rolling window of significant tokens before the token. The
    /// default when no scope is named.
    Window,
    /// The earlier tokens at the token's phase of the dominant token-kind
    /// period: its column, in periodic records.
    Phase,
    /// The tokens since the byte grain's last regime change.
    Regime,
    /// The earlier occurrences of the token's own key.
    Echo,
    /// The heads of the brackets enclosing the token.
    Enclosing,
    /// The values bound by `=` or `:` to earlier occurrences of the token
    /// the named register holds: the history of that key's values.
    Key(String),
}

impl Scope {
    /// The scope a name denotes. The five context names are reserved; any
    /// other name is a register.
    #[must_use]
    pub fn parse(name: &str) -> Scope {
        match name {
            "window" => Scope::Window,
            "phase" => Scope::Phase,
            "regime" => Scope::Regime,
            "echo" => Scope::Echo,
            "enclosing" => Scope::Enclosing,
            other => Scope::Key(other.to_string()),
        }
    }
}

/// The contexts a pattern reads, one flag per [`Scope`] (the phase anchor
/// counts as reading the phase).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextUses {
    /// The rolling window (`\N{>+1}`).
    pub window: bool,
    /// The phase fold or anchor (`\N{>+1:phase}`, `@phase:2`).
    pub phase: bool,
    /// The regime fold (`\N{>+1:regime}`).
    pub regime: bool,
    /// The echo fold (`\N{>+1:echo}`).
    pub echo: bool,
    /// The enclosure fold (`\N{>+1:enclosing}`).
    pub enclosing: bool,
    /// A key's value history (`\N{>+1:k}`).
    pub key: bool,
}

impl ContextUses {
    /// Whether any relation-admitted context or the phase is read.
    #[must_use]
    pub fn any_related(&self) -> bool {
        self.phase || self.regime || self.echo || self.enclosing || self.key
    }
}

/// How far from the context's mean a relative predicate's threshold sits,
/// signed, stored times 100 like the absolute thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delta {
    /// Orders of magnitude.
    Orders(i32),
    /// Standard deviations of the context's magnitudes.
    Sigmas(i32),
}

impl MagPred {
    /// Whether a token whose order of magnitude is `mag` satisfies this
    /// predicate, given the magnitude fold of the predicate's context for a
    /// relative form. `None`, or an empty fold, matches nothing: there is no
    /// baseline to be above. A sigma form also needs a spread to measure in,
    /// so a context of one value, or of equal values, matches nothing
    /// either. Both engines resolve the token's magnitude with
    /// [`crate::magnitude::token_magnitude`] and call this, so the compare
    /// lives in one place.
    #[must_use]
    pub fn matches(&self, mag: f32, context: Option<&crate::profile::MagnitudeProfile>) -> bool {
        match self {
            MagPred::Ge(centi) => mag * 100.0 >= *centi as f32,
            MagPred::Le(centi) => mag * 100.0 <= *centi as f32,
            MagPred::Above(_, delta) | MagPred::Below(_, delta) => {
                let Some(c) = context.filter(|c| c.count > 0) else { return false };
                let offset = match delta {
                    Delta::Orders(centi) => *centi as f32 / 100.0,
                    Delta::Sigmas(centi) => {
                        let sigma = c.std_dev();
                        if c.count < 2 || sigma <= 0.0 {
                            return false;
                        }
                        *centi as f32 / 100.0 * sigma
                    }
                };
                let threshold = c.mean() + offset;
                if matches!(self, MagPred::Above(..)) { mag >= threshold } else { mag <= threshold }
            }
        }
    }

    /// The context a relative form reads; `None` for an absolute one.
    #[must_use]
    pub fn scope(&self) -> Option<&Scope> {
        match self {
            MagPred::Ge(_) | MagPred::Le(_) => None,
            MagPred::Above(s, _) | MagPred::Below(s, _) => Some(s),
        }
    }
}

/// Which stream a grain-qualified reading runs over.
///
/// An axis that reads a sequence can read any of trex's three, and they are
/// different sequences rather than coarser views of one: a byte-grain seam is
/// a break in the bytes, a token-grain seam a break in the sequence of kinds,
/// a supertoken-grain seam a break in the sequence of roles. A pattern names
/// the one it means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Grain {
    /// The byte stream. The unqualified reading, and the default.
    #[default]
    Byte,
    /// The significant-token stream, read by kind.
    Token,
    /// The supertoken stream, read by role.
    Super,
}

impl Grain {
    /// The grain a name denotes, or `None` when it names none.
    #[must_use]
    pub fn parse(name: &str) -> Option<Grain> {
        Some(match name {
            "byte" => Grain::Byte,
            "token" => Grain::Token,
            "super" => Grain::Super,
            _ => return None,
        })
    }
}

/// A second input a join anchor reads: its bytes and their lex, taken once
/// when the pattern is parsed, so every anchor naming it shares one lex.
#[derive(Debug, PartialEq, Eq)]
pub struct OtherInput {
    /// The name the pattern wrote after `@`: a path, or a name the caller
    /// supplied the bytes for.
    pub name: String,
    pub bytes: Vec<u8>,
    pub tokens: Vec<crate::token::Token>,
}

/// An `f64` held by its bit pattern, so a pattern node carrying one keeps
/// `Eq`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Real(u64);

impl Real {
    /// The real `v`.
    #[must_use]
    pub fn new(v: f64) -> Real {
        Real(v.to_bits())
    }

    /// The value held.
    #[must_use]
    pub fn get(self) -> f64 {
        f64::from_bits(self.0)
    }
}

/// What a gravity reading is held against: a percentile of the input's own
/// readings at the grain, or a value in bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// `@strain>90`: the reading at that percentile, `0..=100`, of this
    /// input's readings.
    Percentile(Real),
    /// `@strain>2.5b`: that many bits.
    Bits(Real),
}

/// Which reading of the input's pair field an anchor holds against a level
/// (see [`crate::gravity`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GravityReading {
    /// `@strain`: a unit's mean potential against the units before it.
    Strain,
    /// `@bound`: the attraction across the cut before a unit.
    Bound,
}

/// A zero-width position anchor: it asserts a property of the current
/// position (a boundary in a precomputed axis field) without consuming a
/// token, the statistical analog of regex's `^` / `$` / `\b`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorKind {
    /// `@seam`: the current token starts at a predictive-segmentation cut -
    /// a point where the past stops predicting the future (branching-entropy
    /// boundary), with no delimiter to anchor on.
    ///
    /// The grain says which sequence has to stop predicting itself.
    /// `@seam` reads the sequence of token kinds, `@seam:byte` the bytes,
    /// `@seam:super` the sequence of construct roles. A byte-grain cut falls
    /// at a word boundary; a token-grain cut falls where the shape of the
    /// statement changes, which is a different question about the same input.
    Seam(Grain),
    /// `@strain>90`, `@bound<10`, `@strain:byte>2.5b`, `@bound:super<=5`: a
    /// reading of the input's pair field at the current token, held against a
    /// percentile of the input's own readings or a value in bits.
    ///
    /// The grain says which units the field is learned over and read at:
    /// `@strain` reads the token, `:byte` the token's first byte, `:super` the
    /// supertoken holding the token. Strain is that unit's; bound is the cut
    /// before it, so at the supertoken grain it holds only at a token that
    /// opens its supertoken.
    Gravity(GravityReading, Grain, Cmp, Level),
    /// `@kin("x")`, `@kin:byte("e")`, `@kin:super("f(x)")`: the current unit's
    /// type is `x`'s type or shares its gravity class in this input - a type
    /// the input's pair field treats like the one `x` lexes to.
    ///
    /// The unit is read at the grain as [`Self::Gravity`] reads it. `x` names
    /// its type by example: at the byte grain it is one byte, at the token
    /// grain the first token it lexes to, at the supertoken grain the first
    /// supertoken it forms. An example the input never holds matches nothing.
    Kin(Grain, String),
    /// `@nested>k` / `@nested>=k`: the current token is at least this many
    /// brackets deep (the stress-axis structural-load regime). `@nested>2`
    /// stores `3`; `@nested>=2` stores `2`.
    Nested(u16),
    /// `@ambiguous`: the current token's span contains a contested point - a
    /// position whose reading depends on the observer's vantage (the causal
    /// and anticausal readings disagree), the observation axis.
    ///
    /// The grain says which sequence's two readings have to disagree.
    /// `@ambiguous` reads the bytes, where the contest is over how characters
    /// group; `@ambiguous:token` and `@ambiguous:super` read the sequences
    /// above, where it is over how structure groups.
    Ambiguous(Grain),
    /// `@novel`: the current token is the first occurrence of its content in
    /// the input - a keyed token with no prior echo (the echo axis).
    Novel,
    /// `@echoed`: the current token's content recurs elsewhere in the input
    /// (before or after) - a keyed token whose echo count is at least two.
    Echoed,
    /// `@echo>5`, `@echo:nth=3`, `@echo:period`: a reading of the echo axis
    /// at the current token, counted at the orbit rung an `(?orbit:G ...)`
    /// scope around it names.
    Echo(EchoPred, OrbitGroup),
    /// `@order:asc` / `@order:desc`: the timestamp here stands at or after
    /// the timestamp token before it in the stream, or before it. A token
    /// that is not a timestamp, and the first timestamp of an input, satisfy
    /// neither: there is no pair to order.
    Order(TimeOrder),
    /// `@shape:rare`, `@shape:rare<5`, `@shape:rare<1%`: the line this
    /// token stands on has a template rarer than the cut, the templates
    /// being the input's lines grouped by token-kind silhouette.
    Rare(crate::templates::Rarity),
    /// `@echoed:@other.log` / `@novel:@other.log`: the current token's
    /// content occurs somewhere in a second input, or nowhere in it, keyed
    /// at the orbit rung an `(?orbit:G ...)` scope around it names. A token
    /// the echo axis does not key satisfies neither.
    Joined { other: std::sync::Arc<OtherInput>, recurs: bool, group: OrbitGroup },
    /// `^`: the current token is the first significant token of a line,
    /// or of the input.
    ///
    /// The positional counterpart to the property anchors above. A
    /// regex-shaped language is expected to be able to say where a match
    /// sits, and until this every anchor described what a token IS rather
    /// than where it stands. Line-leading position is the structural
    /// distinction that separates a token being USED from a token being
    /// mentioned: in most line-oriented input the first token of a line is
    /// the thing acting, and the same word later in the line is an
    /// argument or prose.
    ///
    /// Whitespace does not count as significant, so an indented token
    /// still leads its line.
    LineStart,
    /// `$`: the current token is the last significant token of a line, or
    /// of the input. The mirror of [`Self::LineStart`].
    LineEnd,
    /// `@super`: the current token is the first of its supertoken - a
    /// construct boundary.
    ///
    /// The upper-grain counterpart of `@seam`. Where a seam is a statistical
    /// break in the byte stream, this is a structural one in the tower: the
    /// point where one construct ends and the next begins.
    SuperStart,
    /// `@super:role`: the supertoken containing the current token has this
    /// role.
    ///
    /// The one thing a regular expression cannot state at all, because it has
    /// no notion of a container: an atom conditioned on what encloses it. A
    /// number is a number wherever it sits, but a number inside an assignment
    /// is a value and a number inside a call is an argument, and this is what
    /// tells them apart without a grammar for the language.
    SuperRole(crate::supertoken::Role),
    /// `\K`: report the match as beginning here, discarding what was matched
    /// before it.
    ///
    /// Everything to its left is still required, so it reads as a lookbehind
    /// whose width need not be known: `"key" ":" \K \W` requires the key and
    /// colon and reports only the value. [`Look::Behind`] is the other way to
    /// say that and needs a bounded sub-pattern, since it tries start
    /// positions in a fixed window; this has no such limit because it
    /// consumes what it looks at rather than searching backwards for it.
    ///
    /// It moves the reported start only. The scan resumes from the match's
    /// real end, so a run does not overlap the part a `\K` hid.
    ResetStart,
    /// `\G`: the current token is exactly where the previous match ended, so
    /// this match abuts it with nothing skipped.
    ///
    /// A scan reports the leftmost non-overlapping matches, which lets it
    /// skip whatever fails to match. This refuses that: a run of matches
    /// carrying `\G` covers a contiguous stretch, and the run stops at the
    /// first token the pattern cannot take. That is the difference between
    /// finding occurrences and tokenizing, and it is the one thing a scan
    /// cannot express by filtering its results afterwards - the gap has to
    /// forbid the match rather than be noticed once the match exists.
    ///
    /// At the first attempt there is no previous match, so it holds at the
    /// start of the input.
    Resume,
    /// `\A`: the current token is the first significant token of the whole
    /// input.
    ///
    /// Distinct from [`Self::LineStart`], which holds at the head of every
    /// line. The two are separate because trex's input is usually
    /// line-oriented, so the line reading is the useful default and keeps the
    /// unmarked spelling; a pattern that means the start of the stream says so.
    /// The scope is the input, not any enclosing balanced group, so `\A` inside
    /// `\B(...)` still asks about the stream.
    InputStart,
    /// `\z`: the current token is the last significant token of the whole
    /// input. The mirror of [`Self::InputStart`].
    InputEnd,
    /// `@phase:k`: the current token sits at phase `k` of the dominant
    /// token-kind period - column `k` of a periodic record, counted in
    /// significant tokens from the stream start, with no delimiter named.
    /// Holds nowhere when the stream has no period.
    Phase(u16),
    /// `@phase:k/p`, `@phase:k#n`: the current token sits at column `k` of a
    /// named period - the `p`-token period, or the `n`-th strongest - where
    /// that period is live in the stream by the gate [`Self::Phase`]'s period
    /// clears ([`crate::context::live_periods`]). Holds nowhere when it is not.
    PhaseIn(u16, PeriodRef),
}

/// Which of a stream's live periods a phase anchor counts against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PeriodRef {
    /// The period of this many significant tokens.
    Length(u16),
    /// The `n`-th strongest live period, counting from one. Two periods of
    /// nearly one strength can trade ranks between slices of one input.
    Rank(u16),
}

/// A set expression over token matchers: `[\N \W]`, `[^\N]`, `[\W && \h]`,
/// `[\W -- \u]`.
///
/// A token is a member when it matches one of `any`, and also one of `all`
/// when that is non-empty, and none of `none`; `negated` then flips the
/// answer. The three lists are flat rather than a nested expression tree,
/// which covers union, complement, intersection and difference without a
/// precedence rule for the reader to learn.
///
/// Complement is what makes the token grain closed under the regular
/// operations, and it is cheaper here than in a byte regex: the alphabet is a
/// handful of typed kinds rather than 256 byte values, so a class is a
/// predicate over members and negation is one flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenClass {
    /// Unioned members; a token must match one of these.
    pub any: Vec<Atom>,
    /// Intersected members (`&&`); when non-empty a token must also match one.
    pub all: Vec<Atom>,
    /// Subtracted members (`--`); a token must match none of these.
    pub none: Vec<Atom>,
    /// `[^...]`: the whole membership answer is inverted.
    pub negated: bool,
}

impl TokenClass {
    /// Every member atom across the three lists.
    pub fn members(&self) -> impl Iterator<Item = &Atom> {
        self.any.iter().chain(&self.all).chain(&self.none)
    }

    /// Every member atom, mutably.
    pub fn members_mut(&mut self) -> impl Iterator<Item = &mut Atom> {
        self.any.iter_mut().chain(&mut self.all).chain(&mut self.none)
    }
}

impl Atom {
    /// A literal compared byte-for-byte, the default reading.
    #[must_use]
    pub fn literal(text: &str) -> Atom {
        Atom::Literal(text.to_string(), OrbitGroup::Identity)
    }

    /// Rewrite every literal in this atom to compare under `group`, recursing
    /// into class members. Applied once at parse time by an `(?orbit:G ...)`
    /// scope, so the engines see a literal that already knows how it compares
    /// and neither has to carry a scope stack. The single-pass engine bakes
    /// atoms into instructions at compile time and could not carry one.
    pub fn set_orbit(&mut self, group: OrbitGroup) {
        match self {
            Atom::Literal(_, g) | Atom::LiteralWithin(_, _, g) | Atom::RegisterWithin(_, _, g) => {
                *g = group;
            }
            Atom::Class(c) => {
                for m in c.members_mut() {
                    m.set_orbit(group);
                }
            }
            _ => {}
        }
    }
}

/// A single-token matcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Atom {
    /// A token of a specific typed class (`\N`, `\W`, `\Q`, ...).
    Kind(TokenKind),
    /// Any one significant token (`.`).
    Any,
    /// A literal token whose text equals this string (`"lit"` or a bare
    /// punctuation character), compared under a symmetry group.
    ///
    /// [`OrbitGroup::Identity`] is byte equality, the default everywhere. A
    /// `(?orbit:G ...)` scope rewrites the literals inside it to carry `G`, so
    /// `(?orbit:case "Cat")` matches `cat`, and the notation rung matches a
    /// Greek glyph against its TeX name. This is the generalisation regex
    /// spells `(?i)`, which has only the one rung.
    Literal(String, OrbitGroup),
    /// A token whose text matches the value bound to a register. Plain
    /// `=name` compares byte-for-byte (the [`OrbitGroup::Identity`] orbit);
    /// `=shape name` / `=case name` / `=notation name` compare under a
    /// symmetry group, so the reference matches every token in the bound
    /// token's orbit - a fuzzy backreference, not just an exact repeat.
    RegisterEq(String, OrbitGroup),
    /// A token standing in a typed relation to the value bound to a register:
    /// `=subnet a` the same network, `=domain e` the same mail domain,
    /// `=day t` the same calendar day, `=major v` the same major version.
    /// Both sides are projected through the relation's typed field and the
    /// projections compared, so the relation is decided by the values the
    /// lexer recognized rather than by the bytes.
    RegisterRelated(String, crate::typed::Relation),
    /// A literal token within `k` edits of this string (`"lit"~k`): an
    /// insertion, a deletion or a substitution of one character each, over
    /// the two texts as the group canonicalizes them.
    LiteralWithin(String, u8, OrbitGroup),
    /// A token within `k` edits of the value bound to a register
    /// (`=editk name`), over the two texts as the group canonicalizes them.
    RegisterWithin(String, u8, OrbitGroup),
    /// `=kin a`, `=kin:byte a`, `=kin:super a`: a token whose unit the input's
    /// pair field places with the unit the value bound to the register starts
    /// in - the same type, or one placed gravity class ([`crate::gravity`]).
    /// The grain is read as [`AnchorKind::Kin`] reads it.
    RegisterKin(String, Grain),
    /// A token whose bytes all satisfy a byte class (`\d`, `\w`,
    /// `\s`).
    Byte(ByteClass),
    /// A token whose bytes match a whole-anchored byte-pattern
    /// (`` `[A-Z][a-z]+` ``). The low grain composed into the token
    /// grain: regex-style byte matching inside one token.
    BytePattern(crate::bytepat::BytePat),
    /// A token whose pooled spectral signature satisfies a predicate
    /// (`\F{...}`): the temporal substrate (entropy, period, texture,
    /// change-point) composed into the token grain.
    Spectral(SpectralPred),
    /// A token whose order of magnitude satisfies a predicate (`\M{...}`):
    /// the scale substrate composed into the token grain. Stateless (a pure
    /// function of the token's kind and bytes), so it matches in the linear
    /// single-pass engine with no side field, unlike [`Atom::Spectral`].
    Magnitude(MagPred),
    /// A token satisfying a class set-expression (`[\N \W]`, `[^\N]`).
    Class(Box<TokenClass>),
    /// A token of a specific kind whose magnitude also satisfies a predicate
    /// (`\N{>6}`): the kind atom intersected with the magnitude axis on the
    /// same token ("a number, specifically, over a million"). Stateless, like
    /// [`Atom::Magnitude`]; the `{...}` here is a predicate, not a repeat
    /// count, which the parser tells apart by the comparison operator.
    KindMag(TokenKind, MagPred),
    /// A token of a specific kind whose value, read in the kind's own units,
    /// satisfies a typed predicate (`\I{in:10.0.0.0/8}`, `\V{>=2.0,<3}`,
    /// `\T{age<24h}`): the kind atom intersected with a comparison the
    /// lexer's recognition of the token makes possible. A function of the
    /// token's bytes, and of the clock for a timestamp clause, which each
    /// engine reads once per scan.
    KindPred(TokenKind, crate::typed::TypedPred),
    /// A timestamp standing a written distance from the one a register holds
    /// (`\T{>+1h:t}`): the difference between this instant and the bound one,
    /// compared against a signed duration. The threshold is read from the
    /// stream rather than written into the pattern, so a gap and a burst are
    /// the same construct with two operators.
    Since(Cmp, Signed, String),
}

/// A duration written with a sign, in nanoseconds.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Signed {
    /// Whether the duration stands before rather than after.
    pub negative: bool,
    /// Its magnitude in nanoseconds.
    pub nanos: crate::typed::Decimal,
}

impl Atom {
    /// The magnitude predicate this atom carries, if any; a class answers
    /// for its first member that carries one.
    #[must_use]
    pub fn magnitude_pred(&self) -> Option<&MagPred> {
        match self {
            Atom::Magnitude(p) | Atom::KindMag(_, p) => Some(p),
            Atom::Class(c) => c.members().find_map(Atom::magnitude_pred),
            _ => None,
        }
    }

    /// The register a relative magnitude predicate on this atom reads its
    /// baseline through, if it reads one.
    #[must_use]
    pub fn key_scope(&self) -> Option<&str> {
        match self.magnitude_pred().and_then(MagPred::scope) {
            Some(Scope::Key(name)) => Some(name),
            _ => None,
        }
    }

    /// `rename` applied to every register name the atom reads: a
    /// back-reference, a typed relation, an edit distance, a timestamp gap,
    /// a magnitude history, and the members of a class.
    pub(crate) fn rename_registers(&mut self, rename: &dyn Fn(&mut String)) {
        match self {
            Atom::RegisterEq(name, _)
            | Atom::RegisterRelated(name, _)
            | Atom::RegisterWithin(name, _, _)
            | Atom::RegisterKin(name, _)
            | Atom::Since(_, _, name) => rename(name),
            Atom::Magnitude(p) | Atom::KindMag(_, p) => p.rename_key(rename),
            Atom::Class(c) => {
                for member in c.members_mut() {
                    member.rename_registers(rename);
                }
            }
            _ => {}
        }
    }
}

impl MagPred {
    /// `rename` applied to the register a relative predicate reads its
    /// history through, where it reads one.
    fn rename_key(&mut self, rename: &dyn Fn(&mut String)) {
        if let MagPred::Above(Scope::Key(name), _) | MagPred::Below(Scope::Key(name), _) = self {
            rename(name);
        }
    }
}

/// How an alternation chooses among its branches.
///
/// The three are genuinely different answers, not shades of one. For
/// `("a" | "a" "b") "c"` over `a b c`: [`Self::First`] and [`Self::Longest`]
/// both match the whole span, [`Self::Committed`] matches nothing, because it
/// takes the short branch and never reconsiders when `"c"` fails. For
/// `\W | \W \W` over `a b`: [`Self::First`] and [`Self::Committed`] match `a`
/// then `b`, [`Self::Longest`] matches `a b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AltMode {
    /// `|`: leftmost-first, the default. Branches are preferred in order, but
    /// a branch whose continuation fails yields to the next. Perl, PCRE and
    /// Rust's `regex` all match this way, so a pattern ported from one of them
    /// keeps its meaning.
    First,
    /// `||`: leftmost-longest. A true regular union with no preference; the
    /// longest overall match wins. POSIX and RE2 match this way.
    Longest,
    /// `|>`: committed choice. The first branch that matches wins outright and
    /// the rest are never tried, whatever follows. PEG semantics, and what the
    /// grammar engine uses internally.
    Committed,
}

impl AltMode {
    /// The surface operator, for diagnostics.
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            AltMode::First => "|",
            AltMode::Longest => "||",
            AltMode::Committed => "|>",
        }
    }
}

/// Which way a zero-width assertion reads from the current position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// `~(P)`: `P` matches starting here.
    Ahead,
    /// `~<(P)`: `P` matches ending here. Requires a bounded `P`, so the
    /// start positions to try are a fixed window rather than the whole prefix.
    Behind,
    /// `~>k(P)` and `~>k{m,n}(P)`: of the `k` significant tokens starting at
    /// the position the assertion holds at, between `m` and `n` are positions
    /// where `P` starts a match. `~>k(P)` is `m` of one and no `n`.
    ///
    /// The position is the one reached, not the one last consumed, so in
    /// `\W ~>2(P)` the window opens on the token after the word rather than on
    /// the word. Every assertion reads from where it stands and this is no
    /// different.
    ///
    /// Two questions a regular expression cannot ask, in one shape. Distance
    /// in tokens is the unit a token stream has and a byte stream does not -
    /// `.{0,n}` over bytes is a different question, since a token is not
    /// bounded in bytes. Counting how many times something occurs nearby is
    /// not a regular property at all.
    ///
    /// `window` is never zero and `at_least` never exceeds `at_most`; the
    /// parser refuses a window that can hold nothing and a range that nothing
    /// can satisfy.
    ///
    /// The window is why the assertion is bounded, and a bounded assertion
    /// does not make a match depend on the whole input the way `~(P)` and a
    /// content guard do.
    Within { window: usize, at_least: usize, at_most: Option<usize> },
    /// `~#(P)` and `~#{m,n}(P)`: of the significant tokens inside the balanced
    /// group that OPENS at the position the assertion holds at, between `m` and
    /// `n` are positions where `P` starts a match. `~#(P)` is `m` of one and no
    /// `n`.
    ///
    /// The region is the group, so `P` is confined to it: a match cannot run
    /// past the closing bracket, and no token after it is a position `P` is
    /// tried at. Nested groups lie inside the region, so their tokens count;
    /// counting at one bracket depth only is what `split_at_depth` asks.
    ///
    /// A position that opens no group has an empty region and a count of zero,
    /// which leaves the two polarities exact complements of each other at every
    /// position rather than only where a bracket stands.
    ///
    /// [`Self::Within`] bounds the same count by a token distance the pattern
    /// names; this one bounds it by a structure the input has. That is the
    /// difference that puts it past a regular language: the extent is the
    /// matching bracket, which is found by counting brackets, and a regular
    /// language cannot count them.
    ///
    /// The reach is therefore not a number the pattern carries, so no scanner
    /// can hold enough tokens past a match to finalize one.
    /// [`Pattern::has_assert`] reports it for that reason, which makes
    /// [`Pattern::depends_on_whole_input`] true and the prefix machinery
    /// decline the pattern rather than read a group a cut truncated.
    InGroup { at_least: usize, at_most: Option<usize> },
}

impl Look {
    /// Whether absence satisfies the assertion: a count whose floor is zero is
    /// met by a region holding nothing, so the sub-pattern under it requires
    /// nothing of the input at all.
    ///
    /// A shortcut that reads such a sub-pattern's literals as required refuses
    /// an input unscanned that would have matched. The counting forms are the
    /// only ones that can be satisfied this way: every other assertion runs its
    /// sub-pattern and needs it to succeed.
    #[must_use]
    pub fn satisfied_by_absence(self) -> bool {
        matches!(self, Self::Within { at_least: 0, .. } | Self::InGroup { at_least: 0, .. })
    }
}

/// Which way a quantifier leans when several match lengths are possible.
///
/// The language a quantifier accepts is the same either way; the difference is
/// which accepted length is reported. `\W*? \N` and `\W* \N` accept the same
/// inputs and return different spans on most of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Greed {
    /// `*` `+` `?` `{m,n}`: prefer the longest match, the default everywhere.
    Greedy,
    /// `*?` `+?` `??` `{m,n}?`: prefer the shortest.
    Lazy,
}

/// Which reading a repetition takes when its body can match without
/// consuming a token.
///
/// Allowing an empty iteration to repeat leaves a greedy star with no
/// most-preferred derivation at all, so every engine removes something to make
/// an answer exist, and the two families remove different things. The
/// difference is visible only where the body's highest-priority alternation
/// branch is nullable and a lower-priority one consumes; reverse those two and
/// the readings agree.
///
/// This is a property of the whole pattern rather than of one node. A pattern
/// names it with a leading `(?empty:perl)` or `(?empty:thompson)`, and
/// [`crate::parser::parse_with_empty_loop`] hands it back beside the tree;
/// the tree itself does not carry it, so no consumer can forget to unwrap it
/// and read the wrong one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EmptyLoop {
    /// The crate's reading, and the default. A thread whose body matched empty
    /// returns to a state it already stood at, and a simulation that carries
    /// one thread per state drops it, so the branch that consumed wins.
    #[default]
    Thompson,
    /// The backtracking reading. The empty iteration is taken and the loop
    /// then breaks, so the branch that matched empty is on the path and the
    /// match ends earlier.
    Perl,
}

/// A pattern node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// Matches the empty token sequence.
    Empty,
    /// One token.
    Atom(Atom),
    /// `P:name` / `P::name`: match `P` and bind the matched span text to
    /// `name`. The flag is scope: `false` (`:name`) binds for the rest of the
    /// match; `true` (`::name`) is scoped to the enclosing balanced group, so
    /// the binding is dropped when that group closes and a reference cannot
    /// leak out of it.
    Bind(String, bool, Box<Pattern>),
    /// `\B(P)`, `\B[P]`, `\B{P}`, or bare `\B`: a balanced bracket
    /// group whose interior matches `P`. `None` accepts any bracket
    /// kind.
    Balanced(Option<BracketKind>, Box<Pattern>),
    /// `~"lit"` / `!~"lit"`: a zero-width assertion on `lit` in the
    /// forward window. The flag is the negation: `false` asserts `lit`
    /// occurs (`~"lit"`), `true` asserts it does not (`!~"lit"`, a
    /// negative lookahead that stays linear and ReDoS-free).
    Guard(String, bool),
    /// `~(P)` / `!~(P)` / `~<(P)` / `!~<(P)`: a zero-width assertion that `P`
    /// matches at the current position, consuming nothing. The flag is the
    /// negation. This is the sub-pattern generalisation of [`Self::Guard`],
    /// which asks only whether a literal occurs somewhere in the forward
    /// window; the literal form is kept because a prefilter can answer it
    /// without a positional scan.
    Assert(Box<Pattern>, bool, Look),
    /// `@seam` (and future axis anchors): a zero-width assertion that the
    /// current position sits at a boundary in a precomputed axis field. It
    /// consumes no token; it filters the reachable set by position.
    Anchor(AnchorKind),
    /// `@k P`: position at the k-th field, then match `P`.
    Field(usize, Box<Pattern>),
    /// `P Q ...`: a sequence matched left to right.
    Concat(Vec<Pattern>),
    /// `P | Q ...`: alternation, with the mode fixing how a branch is chosen.
    Alt(Vec<Pattern>, AltMode),
    /// `P*` / `P*?`.
    Star(Box<Pattern>, Greed),
    /// `P+` / `P+?`.
    Plus(Box<Pattern>, Greed),
    /// `P?` / `P??`.
    Opt(Box<Pattern>, Greed),
    /// `P{m,n}` / `P{m,n}?`; `n` is `None` for an open upper bound (`P{m,}`).
    Repeat(Box<Pattern>, usize, Option<usize>, Greed),
    /// `(?>P)`, and the possessive quantifiers that expand to it: match `P`,
    /// keep only the length `P` itself preferred, and never offer another.
    ///
    /// In a backtracking engine this exists to stop catastrophic
    /// backtracking. That reason does not apply here - neither engine
    /// backtracks - so what is left is the meaning: `(?>\W*) \W` cannot
    /// match, because the star takes every word and the atom is then offered
    /// nothing, where `\W* \W` would hand back one.
    ///
    /// It is a cut over lengths, as `|>` is a cut over branches, and it goes
    /// to the same engine for the same reason: the single-pass engine's
    /// thread priority expresses a preference and has no way to discard the
    /// alternatives it has already queued.
    Atomic(Box<Pattern>),
    /// `(A B C)~k`: a run of tokens within `k` token edits of the atom
    /// sequence, an edit being a token the run lacks, a token it has extra,
    /// or a token that matches no atom in its place. The token-grain form of
    /// [`Atom::LiteralWithin`], which counts characters within one token.
    ///
    /// Every run within `k` is offered, ranked by edit cost and then by
    /// length, so what follows the group chooses among the alignments the
    /// way it chooses among an alternation's branches.
    Within(Vec<EditAtom>, u8),
}

/// One element of a `(A B C)~k` group.
///
/// A single-token atom, because the walk aligns one atom against one token
/// and a cell of it is one atom test; the parser refuses anything else
/// inside the group rather than reading it as something it is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditAtom {
    /// The atom, matched against its aligned token as it would be alone.
    pub atom: Atom,
    /// The register the token this atom aligned with binds to, and whether
    /// the binding is scoped to the enclosing balanced group, as
    /// [`Pattern::Bind`] carries them. An atom the alignment deleted binds
    /// nothing, as a register under an untaken optional does.
    pub bind: Option<(String, bool)>,
}

impl Pattern {
    /// Wrap a pattern in a box. Small helper to keep the parser
    /// terse.
    #[must_use]
    pub fn boxed(self) -> Box<Pattern> {
        Box::new(self)
    }

    /// Whether any atom reads a register back, as `=name` does.
    ///
    /// This is what decides whether a binding can be erased without changing
    /// which spans match: a binding records what a match consumed and
    /// constrains nothing, so a pattern nothing reads back matches the same
    /// spans with every binding removed.
    #[must_use]
    pub fn reads_a_register(&self) -> bool {
        fn atom_reads(a: &Atom) -> bool {
            match a {
                Atom::RegisterEq(..)
                | Atom::RegisterRelated(..)
                | Atom::RegisterWithin(..)
                | Atom::RegisterKin(..)
                | Atom::Since(..) => true,
                Atom::Class(c) => {
                    c.any.iter().chain(&c.all).chain(&c.none).any(atom_reads)
                }
                _ => false,
            }
        }
        match self {
            Pattern::Atom(a) => atom_reads(a),
            Pattern::Within(v, _) => v.iter().any(|e| atom_reads(&e.atom)),
            Pattern::Empty | Pattern::Guard(..) | Pattern::Anchor(_) => false,
            Pattern::Bind(_, _, p)
            | Pattern::Balanced(_, p)
            | Pattern::Field(_, p)
            | Pattern::Star(p, _)
            | Pattern::Plus(p, _)
            | Pattern::Opt(p, _)
            | Pattern::Repeat(p, _, _, _)
            | Pattern::Atomic(p)
            | Pattern::Assert(p, _, _) => p.reads_a_register(),
            Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(Pattern::reads_a_register),
        }
    }

    /// Whether the pattern binds anything at all.
    #[must_use]
    pub fn binds_anything(&self) -> bool {
        match self {
            Pattern::Bind(..) => true,
            Pattern::Within(v, _) => v.iter().any(|e| e.bind.is_some()),
            Pattern::Empty | Pattern::Atom(_) | Pattern::Guard(..) | Pattern::Anchor(_) => false,
            Pattern::Balanced(_, p)
            | Pattern::Field(_, p)
            | Pattern::Star(p, _)
            | Pattern::Plus(p, _)
            | Pattern::Opt(p, _)
            | Pattern::Repeat(p, _, _, _)
            | Pattern::Atomic(p)
            | Pattern::Assert(p, _, _) => p.binds_anything(),
            Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(Pattern::binds_anything),
        }
    }

    /// The same pattern with every binding removed, or `None` where it binds
    /// nothing or an atom reads a binding back.
    ///
    /// The spans are the same, which is the whole of what a scan reports: the
    /// routes are written against the shapes the language spells without
    /// bindings, so `\W:name "="` reaches the route `\W "="` takes only once
    /// its binding is off. A caller wanting the registers resolves them against
    /// the original pattern over these spans.
    #[must_use]
    pub fn without_bindings(&self) -> Option<Pattern> {
        if !self.binds_anything() || self.reads_a_register() {
            return None;
        }
        fn strip(p: &Pattern) -> Pattern {
            match p {
                Pattern::Bind(_, _, inner) => strip(inner),
                Pattern::Balanced(k, p) => Pattern::Balanced(*k, strip(p).boxed()),
                Pattern::Field(k, p) => Pattern::Field(*k, strip(p).boxed()),
                Pattern::Star(p, g) => Pattern::Star(strip(p).boxed(), *g),
                Pattern::Plus(p, g) => Pattern::Plus(strip(p).boxed(), *g),
                Pattern::Opt(p, g) => Pattern::Opt(strip(p).boxed(), *g),
                Pattern::Repeat(p, lo, hi, g) => Pattern::Repeat(strip(p).boxed(), *lo, *hi, *g),
                Pattern::Atomic(p) => Pattern::Atomic(strip(p).boxed()),
                Pattern::Assert(p, neg, look) => Pattern::Assert(strip(p).boxed(), *neg, *look),
                Pattern::Concat(v) => Pattern::Concat(v.iter().map(strip).collect()),
                Pattern::Alt(v, mode) => Pattern::Alt(v.iter().map(strip).collect(), *mode),
                Pattern::Within(v, k) => Pattern::Within(
                    v.iter().map(|e| EditAtom { atom: e.atom.clone(), bind: None }).collect(),
                    *k,
                ),
                other => other.clone(),
            }
        }
        Some(strip(self))
    }

    /// Whether the pattern begins with `\G`, so its matches must form a
    /// contiguous run rather than the leftmost non-overlapping selection.
    ///
    /// Only the head position counts, and [`Pattern::resume_is_misplaced`]
    /// rejects any other, so this is the whole of what the scan has to ask.
    #[must_use]
    pub fn starts_with_resume(&self) -> bool {
        match self {
            Pattern::Anchor(AnchorKind::Resume) => true,
            Pattern::Concat(v) => v.first().is_some_and(Pattern::starts_with_resume),
            Pattern::Bind(_, _, p) => p.starts_with_resume(),
            _ => false,
        }
    }

    /// Whether `\G` appears anywhere it cannot mean anything: that is, at all
    /// except the head of the pattern.
    #[must_use]
    pub fn resume_is_misplaced(&self) -> bool {
        // The head occurrence is the legal one, so it is not searched.
        match self {
            Pattern::Concat(v) => {
                let head_ok = v.first().is_some_and(Pattern::starts_with_resume);
                let skip = usize::from(head_ok);
                v.iter().skip(skip).any(Pattern::mentions_resume)
                    || v.first().is_some_and(|p| !head_ok && p.mentions_resume())
            }
            Pattern::Anchor(AnchorKind::Resume) => false,
            other => other.mentions_resume(),
        }
    }

    /// Whether an anchor reading the whole stream's ends - `\A` or `\z` -
    /// occurs anywhere in the pattern.
    ///
    /// The two read whether any significant token precedes or follows the
    /// one at hand, which a caller scanning a slice of the token stream
    /// cannot answer: the slice's first token looks like the input's.
    /// `^` and `$` are not here, reading the bytes either side of a token
    /// rather than the stream, so a slice answers them as the whole does.
    #[must_use]
    pub fn mentions_stream_end_anchor(&self) -> bool {
        match self {
            Pattern::Anchor(AnchorKind::InputStart | AnchorKind::InputEnd) => true,
            Pattern::Empty
            | Pattern::Atom(_)
            | Pattern::Within(..)
            | Pattern::Anchor(_)
            | Pattern::Guard(..) => false,
            Pattern::Star(p, _)
            | Pattern::Plus(p, _)
            | Pattern::Opt(p, _)
            | Pattern::Bind(_, _, p)
            | Pattern::Balanced(_, p)
            | Pattern::Field(_, p)
            | Pattern::Repeat(p, _, _, _)
            | Pattern::Atomic(p)
            | Pattern::Assert(p, _, _) => p.mentions_stream_end_anchor(),
            Pattern::Concat(v) | Pattern::Alt(v, _) => {
                v.iter().any(Pattern::mentions_stream_end_anchor)
            }
        }
    }

    /// Whether `\K` occurs anywhere in the pattern.
    #[must_use]
    pub fn mentions_reset_start(&self) -> bool {
        match self {
            Pattern::Anchor(AnchorKind::ResetStart) => true,
            Pattern::Empty
            | Pattern::Atom(_)
            | Pattern::Within(..)
            | Pattern::Anchor(_)
            | Pattern::Guard(..) => false,
            Pattern::Star(p, _)
            | Pattern::Plus(p, _)
            | Pattern::Opt(p, _)
            | Pattern::Bind(_, _, p)
            | Pattern::Balanced(_, p)
            | Pattern::Field(_, p)
            | Pattern::Repeat(p, _, _, _) | Pattern::Atomic(p)
            | Pattern::Assert(p, _, _) => p.mentions_reset_start(),
            Pattern::Concat(v) | Pattern::Alt(v, _) => {
                v.iter().any(Pattern::mentions_reset_start)
            }
        }
    }

    /// Whether `\G` occurs anywhere at all in this sub-pattern.
    fn mentions_resume(&self) -> bool {
        match self {
            Pattern::Anchor(AnchorKind::Resume) => true,
            Pattern::Empty
            | Pattern::Atom(_)
            | Pattern::Within(..)
            | Pattern::Anchor(_)
            | Pattern::Guard(..) => false,
            Pattern::Star(p, _)
            | Pattern::Plus(p, _)
            | Pattern::Opt(p, _)
            | Pattern::Bind(_, _, p)
            | Pattern::Balanced(_, p)
            | Pattern::Field(_, p)
            | Pattern::Repeat(p, _, _, _) | Pattern::Atomic(p)
            | Pattern::Assert(p, _, _) => p.mentions_resume(),
            Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(Pattern::mentions_resume),
        }
    }

    /// Whether the pattern contains a content guard (`~"lit"`) anywhere.
    /// A guard's forward window is unbounded, so a streaming or pipelined
    /// scanner cannot finalize a match carrying one until the whole input
    /// is seen.
    #[must_use]
    pub fn contains_guard(&self) -> bool {
        match self {
            Pattern::Guard(..) => true,
            Pattern::Assert(p, _, _) => p.contains_guard(),
            Pattern::Empty | Pattern::Atom(_) | Pattern::Within(..) | Pattern::Anchor(_) => false,
            Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) | Pattern::Bind(_, _, p) => {
                p.contains_guard()
            }
            Pattern::Repeat(p, _, _, _) | Pattern::Atomic(p) | Pattern::Balanced(_, p) | Pattern::Field(_, p) => {
                p.contains_guard()
            }
            Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(Pattern::contains_guard),
        }
    }

    /// Whether the pattern needs whole-input, non-droppable context: a field
    /// anchor (`@k`, which counts commas from the input start) or a statistical
    /// anchor (`@seam`, whose axis field is computed over the whole stream). In
    /// either case a committed prefix cannot be dropped, so a streaming or
    /// pipelined scanner must defer finalizing a match that carries one.
    #[must_use]
    pub fn contains_field(&self) -> bool {
        match self {
            Pattern::Field(..) | Pattern::Anchor(_) => true,
            Pattern::Assert(p, _, _) => p.contains_field(),
            Pattern::Empty | Pattern::Atom(_) | Pattern::Within(..) | Pattern::Guard(..) => false,
            Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) | Pattern::Bind(_, _, p) => {
                p.contains_field()
            }
            Pattern::Repeat(p, _, _, _) | Pattern::Atomic(p) | Pattern::Balanced(_, p) => p.contains_field(),
            Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(Pattern::contains_field),
        }
    }

    /// Whether a match's outcome can depend on input outside its own span, so
    /// a chunked scanner must not commit it before the whole input is seen.
    ///
    /// Three sources: a content guard reads an unbounded forward window; a
    /// field anchor counts commas from the input start; an axis atom or anchor
    /// reads a field computed over the whole stream, and truncating the input
    /// moves that reading. A new axis belongs in this predicate, which is the
    /// single place a chunked surface consults.
    #[must_use]
    pub fn depends_on_whole_input(&self) -> bool {
        self.contains_guard()
            || self.contains_field()
            || self.has_spectral()
            || self.has_assert()
            || self.reads_context_window()
            || self.reads_related_context()
            || self.any_node(&|p| matches!(p, Pattern::Anchor(k) if k.reads_whole_input()))
    }

    /// Whether a match's outcome can depend on input beyond the lines it
    /// spans, so a scanner that cuts its input only just after a newline
    /// must not commit it before the whole input is seen:
    /// [`Self::depends_on_whole_input`] less the line anchors, which read no
    /// further than the newlines around the match, and less a lookbehind
    /// that reads only tokens of the match itself.
    #[must_use]
    pub fn depends_on_more_than_its_lines(&self) -> bool {
        self.contains_guard()
            || self.any_node(&|p| match p {
                Pattern::Field(..) => true,
                Pattern::Anchor(k) => !matches!(k, AnchorKind::LineStart | AnchorKind::LineEnd),
                Pattern::Assert(_, _, look) => !matches!(look, Look::Within { .. } | Look::Behind),
                _ => false,
            })
            || self.looks_behind_its_match()
            || self.has_spectral()
            || self.reads_context_window()
            || self.reads_related_context()
    }

    /// Whether a lookbehind can read a token before its match's start: one
    /// standing where the match may have taken fewer tokens than its
    /// sub-pattern can span, or one whose sub-pattern is unbounded.
    #[must_use]
    pub fn looks_behind_its_match(&self) -> bool {
        self.looks_behind_from(0)
    }

    /// [`Self::looks_behind_its_match`] for a node the match reaches having
    /// taken at least `before` tokens.
    fn looks_behind_from(&self, before: usize) -> bool {
        match self {
            Pattern::Assert(p, _, Look::Behind) => match p.max_tokens() {
                Some(reach) => reach > before || p.looks_behind_from(0),
                None => true,
            },
            Pattern::Assert(p, _, _) => p.looks_behind_from(0),
            Pattern::Empty | Pattern::Atom(_) | Pattern::Guard(..) | Pattern::Anchor(_) | Pattern::Within(..) => false,
            Pattern::Bind(_, _, p)
            | Pattern::Atomic(p)
            | Pattern::Opt(p, _)
            | Pattern::Star(p, _)
            | Pattern::Plus(p, _)
            | Pattern::Repeat(p, ..)
            | Pattern::Field(_, p)
            | Pattern::Balanced(_, p) => p.looks_behind_from(before),
            Pattern::Concat(v) => {
                let mut taken = before;
                for p in v {
                    if p.looks_behind_from(taken) {
                        return true;
                    }
                    taken = taken.saturating_add(p.min_tokens());
                }
                false
            }
            Pattern::Alt(v, _) => v.iter().any(|p| p.looks_behind_from(before)),
        }
    }

    /// The fewest tokens a match of the pattern can span. A lower bound: where
    /// the count is not known exactly it is too small, never too large, so a
    /// reader asking what a match has taken errs towards less.
    #[must_use]
    pub fn min_tokens(&self) -> usize {
        match self {
            Pattern::Empty | Pattern::Guard(..) | Pattern::Anchor(_) | Pattern::Assert(..) => 0,
            Pattern::Atom(_) => 1,
            // A run within `k` of `m` atoms drops at most `k` of them and is
            // never empty.
            Pattern::Within(v, k) => v.len().saturating_sub(usize::from(*k)).max(1),
            Pattern::Bind(_, _, p) | Pattern::Atomic(p) | Pattern::Field(_, p) | Pattern::Plus(p, _) => {
                p.min_tokens()
            }
            Pattern::Opt(..) | Pattern::Star(..) | Pattern::Balanced(..) => 0,
            Pattern::Repeat(p, least, _, _) => p.min_tokens().saturating_mul(*least),
            Pattern::Concat(v) => v.iter().map(Pattern::min_tokens).fold(0, usize::saturating_add),
            // An alternation with no branch takes no token.
            Pattern::Alt(v, _) if v.is_empty() => 0,
            Pattern::Alt(v, _) => v.iter().map(Pattern::min_tokens).fold(usize::MAX, usize::min),
        }
    }

    /// Whether the pattern compares a token to the rolling window before it
    /// (`\N{>+1}`), so the set engine folds the window field once per scan.
    #[must_use]
    pub fn reads_context_window(&self) -> bool {
        self.context_uses().window
    }

    /// Whether the pattern reads a relation-admitted context or a phase
    /// (`\N{>+1:phase}`, `\N{>+1:k}`, `@phase:2`), so the set engine builds
    /// the related-context field once per scan.
    #[must_use]
    pub fn reads_related_context(&self) -> bool {
        self.context_uses().any_related()
    }

    /// Which contexts the pattern reads, so a scan builds each one's inputs
    /// only when something asks for it.
    #[must_use]
    pub fn context_uses(&self) -> ContextUses {
        // The walk takes a `Fn`, so the flags are gathered through a cell.
        let uses = std::cell::Cell::new(ContextUses::default());
        self.any_atom(&|a| {
            if let Some(scope) = a.magnitude_pred().and_then(MagPred::scope) {
                let mut u = uses.get();
                match scope {
                    Scope::Window => u.window = true,
                    Scope::Phase => u.phase = true,
                    Scope::Regime => u.regime = true,
                    Scope::Echo => u.echo = true,
                    Scope::Enclosing => u.enclosing = true,
                    Scope::Key(_) => u.key = true,
                }
                uses.set(u);
            }
            false
        });
        let mut u = uses.get();
        if self.any_node(&|p| matches!(p, Pattern::Anchor(AnchorKind::Phase(_)))) {
            u.phase = true;
        }
        u
    }

    /// Whether any atom in the pattern satisfies `pred`, class members
    /// included.
    /// Whether the pattern reads a register back, as `=name` and
    /// `=shape name` do.
    ///
    /// Distinct from [`Self::binds`], which says only that a register is
    /// written. A thread's future depends on what it has bound only where
    /// something reads it: with a back-reference, two threads at one counter
    /// holding different bindings go on to match different tokens and are both
    /// live. Without one, they match identically from here and the
    /// higher-priority thread settles which captures are reported.
    ///
    /// The engine's thread list keys on this. Keying on a binding instead
    /// makes a pattern that only binds pay a hash of its whole save array per
    /// thread per step to keep threads apart that nothing can tell apart.
    #[must_use]
    pub fn reads_registers(&self) -> bool {
        self.any_atom(&|a| {
            matches!(
                a,
                Atom::RegisterEq(..)
                    | Atom::RegisterRelated(..)
                    | Atom::RegisterWithin(..)
                    | Atom::RegisterKin(..)
                    | Atom::Since(..)
            )
        })
    }

    /// Whether the pattern orders timestamps against the one before them
    /// (`@order:asc` / `@order:desc`), so the scan reads each timestamp once
    /// and records which way it stands.
    #[must_use]
    pub fn has_order(&self) -> bool {
        self.any_node(&|p| matches!(p, Pattern::Anchor(AnchorKind::Order(_))))
    }

    /// Whether the pattern reads a whitespace token itself (`\S`, or the
    /// `\s` byte class), so a run of whitespace split at a cut would read
    /// differently from the same run whole.
    #[must_use]
    pub fn reads_whitespace(&self) -> bool {
        self.any_atom(&|a| {
            matches!(a, Atom::Kind(TokenKind::Whitespace) | Atom::Byte(ByteClass::Space))
        })
    }

    /// The most tokens a match of the pattern can span, or `None` when an
    /// open repeat or a balanced group leaves it unbounded. A chunked
    /// scanner commits a match of a bounded pattern once the input holds
    /// that many tokens past the match's start, since no alternative at
    /// that start can reach further; an assertion consumes nothing, so it
    /// spans nothing here, and the assertions that read past a match defer
    /// every commit on their own.
    #[must_use]
    pub fn max_tokens(&self) -> Option<usize> {
        match self {
            Pattern::Empty | Pattern::Guard(..) | Pattern::Anchor(_) | Pattern::Assert(..) => Some(0),
            Pattern::Atom(_) => Some(1),
            // The longest run within `k` of `m` atoms is `m + k` tokens: every
            // atom aligned and `k` more the run carried extra.
            Pattern::Within(v, k) => v.len().checked_add(usize::from(*k)),
            Pattern::Bind(_, _, p) | Pattern::Atomic(p) | Pattern::Opt(p, _) | Pattern::Field(_, p) => {
                p.max_tokens()
            }
            Pattern::Balanced(..) | Pattern::Star(..) | Pattern::Plus(..) => None,
            Pattern::Repeat(p, _, most, _) => {
                most.and_then(|n| p.max_tokens().and_then(|w| w.checked_mul(n)))
            }
            Pattern::Concat(v) => {
                v.iter().try_fold(0usize, |acc, p| p.max_tokens().and_then(|w| acc.checked_add(w)))
            }
            Pattern::Alt(v, _) => {
                v.iter().try_fold(0usize, |acc, p| p.max_tokens().map(|w| acc.max(w)))
            }
        }
    }

    /// Whether a match of the pattern can span no tokens at all.
    ///
    /// The opening-kind route reads this. A pattern that can match nothing
    /// matches at every anchor, so it forces no opening kind; and inside a
    /// concatenation, a leading element that can take nothing leaves the match
    /// opening on whatever follows it, so the kinds it begins with are its own
    /// and the next element's together.
    ///
    /// Answering `false` for a pattern that can in fact take nothing is the
    /// dangerous direction: the route would then refuse anchors where a
    /// zero-width match begins. Every variant is matched rather than defaulted,
    /// so a new one has to be decided rather than inheriting an answer.
    #[must_use]
    pub fn takes_no_tokens(&self) -> bool {
        match self {
            // An assertion runs a sub-pattern as a filter and consumes none of
            // what it reads; a guard and an anchor are positions.
            Pattern::Empty | Pattern::Guard(..) | Pattern::Anchor(_) | Pattern::Assert(..) => true,
            // Every atom covers exactly one token.
            Pattern::Atom(_) => false,
            // The shortest run within `k` of `m` atoms deletes `k` of them, so
            // it is empty only where the budget covers every atom.
            Pattern::Within(v, k) => v.len() <= usize::from(*k),
            Pattern::Bind(_, _, p) | Pattern::Atomic(p) | Pattern::Field(_, p) => p.takes_no_tokens(),
            Pattern::Opt(..) | Pattern::Star(..) => true,
            Pattern::Plus(p, _) => p.takes_no_tokens(),
            // A balanced group is its brackets at least.
            Pattern::Balanced(..) => false,
            Pattern::Repeat(p, least, _, _) => *least == 0 || p.takes_no_tokens(),
            Pattern::Concat(v) => v.iter().all(Pattern::takes_no_tokens),
            Pattern::Alt(v, _) => v.iter().any(Pattern::takes_no_tokens),
        }
    }

    /// Whether the pattern reads the rarity of a line's template
    /// (`@shape:rare`), so the scan mines the input's templates once.
    #[must_use]
    pub fn has_rare(&self) -> bool {
        self.any_node(&|p| matches!(p, Pattern::Anchor(AnchorKind::Rare(_))))
    }

    /// The second inputs the pattern's join anchors read, each with the rung
    /// it is keyed at, once per distinct pair, so the scan keys each once.
    #[must_use]
    pub fn joins(&self) -> Vec<(std::sync::Arc<OtherInput>, OrbitGroup)> {
        use std::sync::Arc;
        let out = std::cell::RefCell::new(Vec::<(Arc<OtherInput>, OrbitGroup)>::new());
        self.any_node(&|p| {
            if let Pattern::Anchor(AnchorKind::Joined { other, group, .. }) = p {
                let mut seen = out.borrow_mut();
                if !seen.iter().any(|(o, g)| Arc::ptr_eq(o, other) && g == group) {
                    seen.push((Arc::clone(other), *group));
                }
            }
            false
        });
        out.into_inner()
    }

    fn any_atom(&self, pred: &impl Fn(&Atom) -> bool) -> bool {
        fn atom_has(a: &Atom, pred: &impl Fn(&Atom) -> bool) -> bool {
            pred(a)
                || match a {
                    Atom::Class(c) => c.members().any(|m| atom_has(m, pred)),
                    _ => false,
                }
        }
        self.any_node(&|p| matches!(p, Pattern::Atom(a) if atom_has(a, pred)))
    }

    /// The most significant tokens past a match's own end that deciding the
    /// match can read, for the bounded assertions that read forward at all.
    ///
    /// A `~>k(P)` assertion tries `P` at each of `k` positions from where it
    /// stands, and `P` itself can run on, so its reach is `k` plus `P`'s own
    /// token length. Zero where the pattern has no such assertion.
    ///
    /// A caller working from a prefix must hold this many tokens beyond a
    /// match before it can trust the verdict: reserving only the match's own
    /// length leaves the assertion reading a stream the cut truncated, and
    /// that reports no match where the whole input has one.
    ///
    /// `~#{m,n}(P)` reads to a closing bracket rather than a token count, so no
    /// number here describes its reach and it contributes none. It is reported
    /// by [`Self::has_assert`] instead, which declines the prefix outright.
    #[must_use]
    pub fn widest_forward_window(&self) -> usize {
        // A cell because the walker takes an `Fn`: it visits every node and
        // never needs to mutate, so the accumulator carries its own.
        let widest = std::cell::Cell::new(0usize);
        self.any_node(&|p| {
            if let Pattern::Assert(inner, _, Look::Within { window, .. }) = p {
                let inner_len = crate::nfa::bounded_max_len(inner).unwrap_or(0);
                widest.set(widest.get().max(window.saturating_add(inner_len)));
            }
            false
        });
        widest.get()
    }

    /// Whether the pattern carries a zero-width sub-pattern assertion whose
    /// reach is not bounded. Such an assertion reads input outside the match
    /// span in either direction and as far as it needs to, so a chunked
    /// scanner cannot finalize a match carrying one.
    ///
    /// A `~>k(P)` assertion reads at most `k` significant tokens past the
    /// position and is not counted here: its reach is part of the pattern, so
    /// a scanner holding that many tokens beyond a match can finalize it.
    ///
    /// `~#{m,n}(P)` is counted, because its reach is the enclosing bracket and
    /// the input decides where that is. A cut falling inside the group leaves
    /// the opening token with no mate, which reads as an empty region and a
    /// count of zero - a verdict of no match on an input that has one.
    #[must_use]
    pub fn has_assert(&self) -> bool {
        self.any_node(&|p| {
            matches!(p, Pattern::Assert(_, _, look) if !matches!(look, Look::Within { .. }))
        })
    }

    /// Whether the pattern reads the supertoken tower, so a chunked scanner
    /// must not commit a match carrying it before the whole input is seen.
    ///
    /// A unit is a run of tokens, so one cut by a chunk boundary is the same
    /// hazard [`Self::depends_on_whole_input`] already declares for the other
    /// whole-stream readings. This is covered there through
    /// [`Self::contains_field`], which answers true for every anchor rather
    /// than enumerating them, and the test on this module pins that so the
    /// coverage is not accidental.
    #[must_use]
    pub fn reads_supertokens(&self) -> bool {
        self.has_super()
    }

    /// Whether the pattern queries the seam axis (`@seam`) anywhere, so the
    /// set engine builds the seam field once and threads it read-only.
    #[must_use]
    pub fn has_seam(&self) -> bool {
        self.has_seam_at(Grain::Byte)
    }

    /// Whether the pattern reads the seam axis at one particular grain, so
    /// only the streams a pattern names are segmented.
    #[must_use]
    pub fn has_seam_at(&self, grain: Grain) -> bool {
        self.any_node(&|p| matches!(p, Pattern::Anchor(AnchorKind::Seam(g)) if *g == grain))
    }

    /// Whether any anchor counts a column against a named period
    /// (`@phase:k/p`, `@phase:k#n`), so the live periods are read once.
    #[must_use]
    pub fn has_phase_in(&self) -> bool {
        self.any_node(&|p| matches!(p, Pattern::Anchor(AnchorKind::PhaseIn(..))))
    }

    /// Whether any anchor reads the pair field at `grain` (`@strain`,
    /// `@bound`, `@kin`), so the field is learned only at the grains a
    /// pattern names.
    #[must_use]
    pub fn has_gravity_at(&self, grain: Grain) -> bool {
        self.any_node(&|p| {
            matches!(
                p,
                Pattern::Anchor(AnchorKind::Gravity(_, g, ..) | AnchorKind::Kin(g, _)) if *g == grain
            )
        }) || self.any_atom(&|a| matches!(a, Atom::RegisterKin(_, g) if *g == grain))
            || self.any_node(&|p| {
                matches!(p, Pattern::Within(v, _)
                    if v.iter().any(|e| matches!(&e.atom, Atom::RegisterKin(_, g) if *g == grain)))
            })
    }

    /// Every `@kin` anchor's grain and example, each once, in the order met.
    #[must_use]
    pub fn kin_examples(&self) -> Vec<(Grain, String)> {
        let found = std::cell::RefCell::new(Vec::new());
        self.any_node(&|p| {
            if let Pattern::Anchor(AnchorKind::Kin(g, x)) = p {
                let mut found = found.borrow_mut();
                if !found.iter().any(|(fg, fx)| fg == g && fx == x) {
                    found.push((*g, x.clone()));
                }
            }
            false
        });
        found.into_inner()
    }

    /// Whether this subtree makes a preference-bearing choice of its own: a
    /// leftmost-first alternation, or a quantifier.
    ///
    /// A quantifier whose body answers `false` is fully described by how many
    /// times it ran, so the set engine records one number for it. A body that
    /// answers `true` made choices inside each iteration, and those have to be
    /// comparable position by position against another derivation's, so that
    /// quantifier records one entry per iteration instead.
    #[must_use]
    pub fn has_choice(&self) -> bool {
        self.any_node(&|p| {
            matches!(
                p,
                Pattern::Alt(_, AltMode::First)
                    | Pattern::Star(..)
                    | Pattern::Plus(..)
                    | Pattern::Opt(..)
                    | Pattern::Repeat(..)
            )
        })
    }

    /// Whether the pattern carries a field anchor (`@k`), so the set engine
    /// builds the per-token field index once. Narrower than
    /// [`Self::contains_field`], which also answers true for an axis anchor.
    #[must_use]
    pub fn has_field_anchor(&self) -> bool {
        self.any_node(&|p| matches!(p, Pattern::Field(..)))
    }

    /// Whether the pattern queries the stress axis (`@nested>k`) anywhere, so
    /// the set engine builds the stress field once and threads it read-only.
    #[must_use]
    pub fn has_stress(&self) -> bool {
        self.any_node(&|p| matches!(p, Pattern::Anchor(AnchorKind::Nested(_))))
    }

    /// Whether the pattern queries the observation axis (`@ambiguous`)
    /// anywhere, so the set engine builds the observation field once.
    #[must_use]
    pub fn has_observation(&self) -> bool {
        self.has_observation_at(Grain::Byte)
    }

    /// Whether the pattern reads the observation axis at one grain, so only
    /// the streams a pattern names are read.
    #[must_use]
    pub fn has_observation_at(&self, grain: Grain) -> bool {
        self.any_node(&|p| matches!(p, Pattern::Anchor(AnchorKind::Ambiguous(g)) if *g == grain))
    }

    /// Whether the pattern queries the echo axis (`@novel` / `@echoed` /
    /// `@echo...`) anywhere, so the set engine builds the recurrence field.
    #[must_use]
    pub fn has_echo(&self) -> bool {
        self.any_node(&|p| {
            matches!(p, Pattern::Anchor(AnchorKind::Novel | AnchorKind::Echoed | AnchorKind::Echo(..)))
        })
    }

    /// The orbit rungs the pattern counts recurrence at, in the order they
    /// first appear and each once. A scan builds one recurrence field per
    /// rung; `Identity` is the rung `@novel` and `@echoed` read and the one
    /// an `@echo` under no orbit scope reads.
    #[must_use]
    pub fn echo_orbits(&self) -> Vec<OrbitGroup> {
        let out = std::cell::RefCell::new(Vec::new());
        self.any_node(&|p| {
            let group = match p {
                Pattern::Anchor(AnchorKind::Novel | AnchorKind::Echoed) => OrbitGroup::Identity,
                Pattern::Anchor(AnchorKind::Echo(_, g)) => *g,
                _ => return false,
            };
            let mut seen = out.borrow_mut();
            if !seen.contains(&group) {
                seen.push(group);
            }
            false
        });
        out.into_inner()
    }

    /// Whether the pattern queries the supertoken tower (`@super` /
    /// `@super:role`) anywhere, so the set engine builds the upper-grain
    /// window once.
    #[must_use]
    pub fn has_super(&self) -> bool {
        self.any_node(&|p| {
            matches!(p, Pattern::Anchor(AnchorKind::SuperStart | AnchorKind::SuperRole(_)))
        })
    }

    /// Whether any node in the pattern satisfies `pred`. The shared subtree
    /// walk the per-axis `has_*` queries delegate to, so each is a one-line
    /// predicate rather than a repeated match.
    fn any_node(&self, pred: &impl Fn(&Pattern) -> bool) -> bool {
        if pred(self) {
            return true;
        }
        match self {
            Pattern::Assert(p, _, _) => p.any_node(pred),
            // An edit-distance group's elements are atoms rather than
            // patterns, so there is no node beneath it for a predicate over
            // nodes to reach.
            Pattern::Empty
            | Pattern::Atom(_)
            | Pattern::Within(..)
            | Pattern::Guard(..)
            | Pattern::Anchor(_) => false,
            Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) | Pattern::Bind(_, _, p) => {
                p.any_node(pred)
            }
            Pattern::Repeat(p, _, _, _) | Pattern::Atomic(p) | Pattern::Balanced(_, p) | Pattern::Field(_, p) => {
                p.any_node(pred)
            }
            Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(|c| c.any_node(pred)),
        }
    }

    /// The capture names the pattern can bind (`:name` suffixes), in
    /// first-seen order with duplicates removed. A rewrite template is
    /// validated against this set so a reference to an unbound name is a
    /// template error rather than a silent empty substitution.
    #[must_use]
    pub fn capture_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_capture_names(&mut out);
        out
    }

    /// Nest every register bound inside this pattern under `prefix`: each is
    /// renamed `prefix.name`, and every reference to one of them inside the
    /// pattern follows, so a back-reference, a typed relation, an edit
    /// distance, a timestamp gap or a magnitude history keyed by the register
    /// still reads it. A reference to a register bound outside is left as it
    /// is.
    pub fn nest_registers(&mut self, prefix: &str) {
        let inner = self.capture_names();
        let rename = |name: &mut String| {
            if inner.iter().any(|n| n == name) {
                *name = format!("{prefix}.{name}");
            }
        };
        self.rename_registers(&rename);
    }

    /// `rename` applied to every register name the pattern binds or reads.
    fn rename_registers(&mut self, rename: &dyn Fn(&mut String)) {
        match self {
            Pattern::Bind(name, _, p) => {
                rename(name);
                p.rename_registers(rename);
            }
            Pattern::Atom(a) => a.rename_registers(rename),
            Pattern::Within(v, _) => {
                for e in v {
                    e.atom.rename_registers(rename);
                    if let Some((name, _)) = &mut e.bind {
                        rename(name);
                    }
                }
            }
            Pattern::Empty | Pattern::Guard(..) | Pattern::Anchor(_) => {}
            Pattern::Balanced(_, p)
            | Pattern::Field(_, p)
            | Pattern::Star(p, _)
            | Pattern::Plus(p, _)
            | Pattern::Opt(p, _)
            | Pattern::Repeat(p, _, _, _)
            | Pattern::Atomic(p)
            | Pattern::Assert(p, _, _) => p.rename_registers(rename),
            Pattern::Concat(v) | Pattern::Alt(v, _) => {
                for p in v {
                    p.rename_registers(rename);
                }
            }
        }
    }

    /// The names bound under a repetition, in first-seen order: the
    /// registers that hold every binding a match made, in order, rather than
    /// the last.
    #[must_use]
    pub fn list_registers(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_list_registers(false, &mut out);
        out
    }

    /// Whether any register is bound under a repetition.
    #[must_use]
    pub fn has_list_registers(&self) -> bool {
        !self.list_registers().is_empty()
    }

    /// The registers bound inside each repetition, one list per repetition
    /// in the order they open, each in first-seen order; a repetition
    /// inside another gives its own list as well as adding to the outer's.
    #[must_use]
    pub fn repetition_registers(&self) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        self.collect_repetition_registers(&mut out);
        out
    }

    fn collect_repetition_registers(&self, out: &mut Vec<Vec<String>>) {
        let repetition = |p: &Pattern, out: &mut Vec<Vec<String>>| {
            out.push(p.capture_names());
            p.collect_repetition_registers(out);
        };
        match self {
            Pattern::Star(p, _) | Pattern::Plus(p, _) => repetition(p, out),
            Pattern::Repeat(p, _, hi, _) if *hi != Some(1) => repetition(p, out),
            Pattern::Bind(_, _, p)
            | Pattern::Repeat(p, _, _, _)
            | Pattern::Opt(p, _)
            | Pattern::Balanced(_, p)
            | Pattern::Field(_, p)
            | Pattern::Atomic(p)
            | Pattern::Assert(p, _, _) => p.collect_repetition_registers(out),
            Pattern::Concat(v) | Pattern::Alt(v, _) => {
                for p in v {
                    p.collect_repetition_registers(out);
                }
            }
            Pattern::Within(..) | Pattern::Empty | Pattern::Atom(_) | Pattern::Guard(..) | Pattern::Anchor(_) => {}
        }
    }

    fn collect_list_registers(&self, repeated: bool, out: &mut Vec<String>) {
        match self {
            Pattern::Bind(name, _, p) => {
                if repeated && !out.contains(name) {
                    out.push(name.clone());
                }
                p.collect_list_registers(repeated, out);
            }
            Pattern::Star(p, _) | Pattern::Plus(p, _) => p.collect_list_registers(true, out),
            Pattern::Repeat(p, _, hi, _) => p.collect_list_registers(repeated || *hi != Some(1), out),
            Pattern::Opt(p, _)
            | Pattern::Balanced(_, p)
            | Pattern::Field(_, p)
            | Pattern::Atomic(p)
            | Pattern::Assert(p, _, _) => p.collect_list_registers(repeated, out),
            Pattern::Concat(v) | Pattern::Alt(v, _) => {
                for p in v {
                    p.collect_list_registers(repeated, out);
                }
            }
            // An edit-distance group holds no repetition of its own, so its
            // bindings are a list only where one encloses it.
            Pattern::Within(v, _) => {
                for name in v.iter().filter_map(|e| e.bind.as_ref()).map(|(n, _)| n) {
                    if repeated && !out.contains(name) {
                        out.push(name.clone());
                    }
                }
            }
            Pattern::Empty | Pattern::Atom(_) | Pattern::Guard(..) | Pattern::Anchor(_) => {}
        }
    }

    fn collect_capture_names(&self, out: &mut Vec<String>) {
        match self {
            // An assertion consumes nothing and its bindings are discarded
            // with the probe, so it contributes no capture a template could
            // reference.
            Pattern::Assert(..) => {}
            Pattern::Bind(name, _, p) => {
                if !out.contains(name) {
                    out.push(name.clone());
                }
                p.collect_capture_names(out);
            }
            Pattern::Within(v, _) => {
                for name in v.iter().filter_map(|e| e.bind.as_ref()).map(|(n, _)| n) {
                    if !out.contains(name) {
                        out.push(name.clone());
                    }
                }
            }
            Pattern::Empty | Pattern::Atom(_) | Pattern::Guard(..) | Pattern::Anchor(_) => {}
            Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) => {
                p.collect_capture_names(out);
            }
            Pattern::Repeat(p, _, _, _) | Pattern::Atomic(p) | Pattern::Balanced(_, p) | Pattern::Field(_, p) => {
                p.collect_capture_names(out);
            }
            Pattern::Concat(v) | Pattern::Alt(v, _) => {
                for c in v {
                    c.collect_capture_names(out);
                }
            }
        }
    }

    /// The token kind each register binds, in the order
    /// [`Pattern::capture_names`] reports, `None` where the register binds
    /// anything but a single kind atom.
    ///
    /// A register over a concatenation, an alternation or a repetition binds
    /// text that may span several kinds, and there is no one typed value to
    /// read from it, so it answers `None` rather than the first kind it
    /// happens to contain. A magnitude or a predicate on the atom does not
    /// change which kind it is, so those bind their kind like a bare one.
    #[must_use]
    pub fn capture_kinds(&self) -> Vec<(String, Option<TokenKind>)> {
        let mut out = Vec::new();
        self.collect_capture_kinds(&mut out);
        out
    }

    fn collect_capture_kinds(&self, out: &mut Vec<(String, Option<TokenKind>)>) {
        match self {
            Pattern::Assert(..) => {}
            Pattern::Bind(name, _, p) => {
                if !out.iter().any(|(n, _)| n == name) {
                    out.push((name.clone(), p.one_kind()));
                }
                p.collect_capture_kinds(out);
            }
            // Each element is one atom, so a binding on one carries that
            // atom's kind exactly as a bare `\N:n` does.
            Pattern::Within(v, _) => {
                for e in v {
                    if let Some((name, _)) = &e.bind
                        && !out.iter().any(|(n, _)| n == name)
                    {
                        out.push((name.clone(), Pattern::Atom(e.atom.clone()).one_kind()));
                    }
                }
            }
            Pattern::Empty | Pattern::Atom(_) | Pattern::Guard(..) | Pattern::Anchor(_) => {}
            Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) => {
                p.collect_capture_kinds(out);
            }
            Pattern::Repeat(p, _, _, _)
            | Pattern::Atomic(p)
            | Pattern::Balanced(_, p)
            | Pattern::Field(_, p) => {
                p.collect_capture_kinds(out);
            }
            Pattern::Concat(v) | Pattern::Alt(v, _) => {
                for c in v {
                    c.collect_capture_kinds(out);
                }
            }
        }
    }

    /// The kind this pattern is one atom of, or `None` for anything else.
    ///
    /// A sequence or an alternation of exactly one member is that member, so
    /// a register reads its kind through either. Without that, a lone atom
    /// the parser happened to wrap would report no value at all, which reads
    /// identically to a register that genuinely has none.
    ///
    /// Every other shape answers `None` deliberately. A run, a repetition and
    /// an alternation of two or more all bind text whose kind varies between
    /// matches or spans several kinds at once, and there is no single value
    /// to read from them. An unsure shape answers `None`, because a value
    /// printed for text that is not that kind is worse than no value.
    fn one_kind(&self) -> Option<TokenKind> {
        match self {
            Pattern::Atom(Atom::Kind(k) | Atom::KindMag(k, _) | Atom::KindPred(k, _)) => Some(*k),
            Pattern::Atomic(p) => p.one_kind(),
            Pattern::Concat(v) | Pattern::Alt(v, _) if v.len() == 1 => v[0].one_kind(),
            _ => None,
        }
    }

    /// The ids of every declared or library kind the pattern names, each
    /// once, in order of first mention.
    #[must_use]
    pub fn custom_kinds(&self) -> Vec<u8> {
        fn atom_into(a: &Atom, out: &mut Vec<u8>) {
            match a {
                Atom::Kind(TokenKind::Custom(id))
                | Atom::KindMag(TokenKind::Custom(id), _)
                | Atom::KindPred(TokenKind::Custom(id), _) => {
                    if !out.contains(id) {
                        out.push(*id);
                    }
                }
                Atom::Class(c) => c.members().for_each(|m| atom_into(m, out)),
                _ => {}
            }
        }
        fn into(p: &Pattern, out: &mut Vec<u8>) {
            match p {
                Pattern::Atom(a) => atom_into(a, out),
                Pattern::Within(v, _) => v.iter().for_each(|e| atom_into(&e.atom, out)),
                Pattern::Empty | Pattern::Guard(..) | Pattern::Anchor(_) => {}
                Pattern::Assert(p, _, _)
                | Pattern::Star(p, _)
                | Pattern::Plus(p, _)
                | Pattern::Opt(p, _)
                | Pattern::Bind(_, _, p)
                | Pattern::Repeat(p, _, _, _)
                | Pattern::Atomic(p)
                | Pattern::Balanced(_, p)
                | Pattern::Field(_, p) => into(p, out),
                Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().for_each(|p| into(p, out)),
            }
        }
        let mut out = Vec::new();
        into(self, &mut out);
        out
    }

    /// The ids of the shipped library's kinds the pattern names, which a lex
    /// has to be told to produce.
    #[must_use]
    pub fn library_kinds(&self) -> Vec<u8> {
        self.custom_kinds().into_iter().filter(|&id| id >= crate::custom::LIBRARY_ID_BASE).collect()
    }

    /// Whether the pattern drops to the byte grain anywhere: a byte
    /// pattern (`` `...` ``), a byte class (`\d` / `\w` / `\s`), or a
    /// content guard (a byte-level forward-window assertion). This is the
    /// sub-token byte constraint that the dual-grain split routes to the
    /// byte grain.
    #[must_use]
    pub fn has_byte_constraint(&self) -> bool {
        fn atom_has(a: &Atom) -> bool {
            match a {
                Atom::Byte(_) | Atom::BytePattern(_) => true,
                Atom::Class(c) => c.members().any(atom_has),
                _ => false,
            }
        }
        match self {
            Pattern::Atom(a) if atom_has(a) => true,
            Pattern::Within(v, _) if v.iter().any(|e| atom_has(&e.atom)) => true,
            Pattern::Guard(..) => true,
            Pattern::Assert(p, _, _) => p.has_byte_constraint(),
            Pattern::Empty | Pattern::Atom(_) | Pattern::Within(..) | Pattern::Anchor(_) => false,
            Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) | Pattern::Bind(_, _, p) => {
                p.has_byte_constraint()
            }
            Pattern::Repeat(p, _, _, _) | Pattern::Atomic(p) | Pattern::Balanced(_, p) | Pattern::Field(_, p) => {
                p.has_byte_constraint()
            }
            Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(Pattern::has_byte_constraint),
        }
    }

    /// Whether the pattern queries the spectral axis (`\F{...}`) anywhere.
    /// Such a pattern routes to the set-reachability engine, which builds
    /// the spectral field once and threads it read-only to the matcher.
    #[must_use]
    pub fn has_spectral(&self) -> bool {
        fn atom_has(a: &Atom) -> bool {
            match a {
                Atom::Spectral(_) => true,
                Atom::Class(c) => c.members().any(atom_has),
                _ => false,
            }
        }
        match self {
            Pattern::Atom(a) if atom_has(a) => true,
            Pattern::Within(v, _) if v.iter().any(|e| atom_has(&e.atom)) => true,
            Pattern::Assert(p, _, _) => p.has_spectral(),
            Pattern::Empty
            | Pattern::Atom(_)
            | Pattern::Within(..)
            | Pattern::Guard(..)
            | Pattern::Anchor(_) => false,
            Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) | Pattern::Bind(_, _, p) => {
                p.has_spectral()
            }
            Pattern::Repeat(p, _, _, _) | Pattern::Atomic(p) | Pattern::Balanced(_, p) | Pattern::Field(_, p) => {
                p.has_spectral()
            }
            Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(Pattern::has_spectral),
        }
    }

    /// The readings the pattern's `\F{...}` atoms take, so a scan builds the
    /// spectral field with those and no others.
    ///
    /// The field is a step a byte over the whole input, so a reading nothing
    /// asks for is paid at every byte: an atom naming the entropy alone would
    /// otherwise also decay a filterbank, count a k-gram through a map and
    /// take a square root, at each of them.
    #[must_use]
    pub fn spectral_needs(&self) -> crate::spectral::Needs {
        fn atom_into(a: &Atom, out: &mut crate::spectral::Needs) {
            match a {
                Atom::Spectral(p) => *out = out.and(crate::spectral::Needs::of(p)),
                Atom::Class(c) => c.members().for_each(|m| atom_into(m, out)),
                _ => {}
            }
        }
        let out = std::cell::RefCell::new(crate::spectral::Needs::none());
        self.any_node(&|p| {
            match p {
                Pattern::Atom(a) => atom_into(a, &mut out.borrow_mut()),
                Pattern::Within(v, _) => {
                    for e in v {
                        atom_into(&e.atom, &mut out.borrow_mut());
                    }
                }
                _ => {}
            }
            false
        });
        out.into_inner()
    }

    /// Whether a bind appears anywhere in the pattern, so a scan of it has
    /// captures to resolve.
    #[must_use]
    pub fn binds(&self) -> bool {
        match self {
            Pattern::Bind(..) => true,
            Pattern::Within(v, _) => v.iter().any(|e| e.bind.is_some()),
            Pattern::Empty | Pattern::Atom(_) | Pattern::Guard(..) | Pattern::Anchor(_) => false,
            Pattern::Assert(p, _, _)
            | Pattern::Star(p, _)
            | Pattern::Plus(p, _)
            | Pattern::Opt(p, _)
            | Pattern::Repeat(p, _, _, _)
            | Pattern::Atomic(p)
            | Pattern::Balanced(_, p)
            | Pattern::Field(_, p) => p.binds(),
            Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(Pattern::binds),
        }
    }
}

/// Collapse nested quantifiers using Kleene-algebra identities that
/// preserve the matched language:
/// `(r*)* = (r+)* = (r?)* = r*`, `(r*)+ = r*`, `(r+)+ = r+`,
/// `(r?)+ = r*`, `(r*)? = (r+)? = r*`, `(r?)? = r?`.
///
/// The collapse fires only when the inner quantified subtree carries
/// no capture (binding, register-equality, or guard), so the reported
/// capture values are never changed. This removes the canonical
/// nested-quantifier blowup: `(.*)*` becomes `.*`, which the engine
/// scans without the per-position recompute the nested form forces.
///
/// It also fires only when the two quantifiers lean the same way. The
/// identities hold for the language a pattern accepts, not for which of the
/// accepted lengths it reports, so collapsing `(r*?)*` to `r*` would keep the
/// same inputs matching and change the spans returned for them. Three lazy
/// pairs stay nested even so, for the reason [`foldable`] gives.
#[must_use]
pub fn normalize(p: Pattern) -> Pattern {
    match p {
        Pattern::Star(inner, g) => collapse_outer_star(normalize(*inner), g),
        Pattern::Plus(inner, g) => collapse_outer_plus(normalize(*inner), g),
        Pattern::Opt(inner, g) => collapse_outer_opt(normalize(*inner), g),
        Pattern::Concat(v) => Pattern::Concat(v.into_iter().map(normalize).collect()),
        Pattern::Alt(v, m) => Pattern::Alt(v.into_iter().map(normalize).collect(), m),
        Pattern::Bind(n, s, x) => Pattern::Bind(n, s, normalize(*x).boxed()),
        Pattern::Balanced(k, x) => Pattern::Balanced(k, normalize(*x).boxed()),
        Pattern::Repeat(x, lo, hi, g) => Pattern::Repeat(normalize(*x).boxed(), lo, hi, g),
        Pattern::Field(k, x) => Pattern::Field(k, normalize(*x).boxed()),
        Pattern::Assert(x, neg, look) => Pattern::Assert(normalize(*x).boxed(), neg, look),
        // The body normalizes, but the cut is opaque to the identities above:
        // an outer quantifier folding through it would restore the lengths
        // the atomic group exists to discard.
        Pattern::Atomic(x) => Pattern::Atomic(normalize(*x).boxed()),
        leaf @ (Pattern::Empty
        | Pattern::Atom(_)
        | Pattern::Within(..)
        | Pattern::Guard(..)
        | Pattern::Anchor(_)) => leaf,
    }
}

/// The three quantifier shapes a fold pairs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quant {
    Star,
    Plus,
    Opt,
}

/// Whether an inner quantifier may fold into an outer one: the same lean, no
/// capture inside to have its reported value changed, and, for a lazy pair,
/// the same standing after the body has matched.
///
/// A greedy pair offers the body's next token first in either form. A lazy
/// pair offers an enclosing loop's exit first, and where the form stands
/// after the body has matched decides what that loop's re-entry reaches: an
/// optional or a plus stands past its body, so the re-entry reaches the body
/// afresh and offers its next token before the exit; a star stands at its
/// own head, which the re-entry reaches a second time and drops, so the
/// exit comes first. `(r??)*?`, `(r??)+?` and `(r+?)??` each fold to a form
/// standing at a head where the original stood past the body, and inside a
/// further loop the two rank the body's next token and the exit in opposite
/// orders. The regex crate ranks the nested form's way, so those stay as
/// written.
fn foldable(inner: Quant, i: Greed, outer: Quant, g: Greed, x: &Pattern) -> bool {
    if i != g || has_capture(x) {
        return false;
    }
    match g {
        Greed::Greedy => true,
        Greed::Lazy => !matches!(
            (inner, outer),
            (Quant::Opt, Quant::Star | Quant::Plus) | (Quant::Plus, Quant::Opt)
        ),
    }
}

fn collapse_outer_star(inner: Pattern, g: Greed) -> Pattern {
    match inner {
        Pattern::Star(x, i) if foldable(Quant::Star, i, Quant::Star, g, &x) => Pattern::Star(x, g),
        Pattern::Plus(x, i) if foldable(Quant::Plus, i, Quant::Star, g, &x) => Pattern::Star(x, g),
        Pattern::Opt(x, i) if foldable(Quant::Opt, i, Quant::Star, g, &x) => Pattern::Star(x, g),
        other => Pattern::Star(other.boxed(), g),
    }
}

fn collapse_outer_plus(inner: Pattern, g: Greed) -> Pattern {
    match inner {
        Pattern::Star(x, i) if foldable(Quant::Star, i, Quant::Plus, g, &x) => Pattern::Star(x, g),
        Pattern::Plus(x, i) if foldable(Quant::Plus, i, Quant::Plus, g, &x) => Pattern::Plus(x, g),
        Pattern::Opt(x, i) if foldable(Quant::Opt, i, Quant::Plus, g, &x) => Pattern::Star(x, g),
        other => Pattern::Plus(other.boxed(), g),
    }
}

fn collapse_outer_opt(inner: Pattern, g: Greed) -> Pattern {
    match inner {
        Pattern::Star(x, i) if foldable(Quant::Star, i, Quant::Opt, g, &x) => Pattern::Star(x, g),
        Pattern::Plus(x, i) if foldable(Quant::Plus, i, Quant::Opt, g, &x) => Pattern::Star(x, g),
        Pattern::Opt(x, i) if foldable(Quant::Opt, i, Quant::Opt, g, &x) => Pattern::Opt(x, g),
        other => Pattern::Opt(other.boxed(), g),
    }
}

/// Whether `p` carries any capture-affecting node (binding, register-
/// equality, or content guard) in its subtree. Used to keep the
/// quantifier-collapse from changing reported captures.
fn has_capture(p: &Pattern) -> bool {
    match p {
        // An assertion is a condition on the match, so collapsing a quantifier
        // around one could change which spans are reported.
        Pattern::Bind(..) | Pattern::Guard(..) | Pattern::Assert(..) => true,
        Pattern::Atom(
            Atom::RegisterEq(..) | Atom::RegisterRelated(..) | Atom::RegisterWithin(..) | Atom::RegisterKin(..),
        ) => true,
        Pattern::Within(v, _) => v.iter().any(|e| {
            e.bind.is_some()
                || matches!(
                    e.atom,
                    Atom::RegisterEq(..) | Atom::RegisterRelated(..) | Atom::RegisterWithin(..) | Atom::RegisterKin(..)
                )
        }),
        Pattern::Atom(_) | Pattern::Empty | Pattern::Anchor(_) => false,
        Pattern::Star(x, _)
        | Pattern::Plus(x, _)
        | Pattern::Opt(x, _)
        | Pattern::Balanced(_, x)
        | Pattern::Repeat(x, _, _, _)
        | Pattern::Atomic(x)
        | Pattern::Field(_, x) => has_capture(x),
        Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(has_capture),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_supertoken_pattern_is_whole_input_dependent() {
        // A unit is a run of tokens, so a chunk boundary can cut one. A
        // chunked scanner must therefore not commit a match that read the
        // tower before it has seen the whole input.
        let p = crate::parser::parse("@super:assign \\N").expect("parses");
        assert!(p.reads_supertokens());
        assert!(
            p.depends_on_whole_input(),
            "the tower is a whole-stream reading and has to be declared as one"
        );
        // A pattern that does not read the tower is not dragged in with it.
        let plain = crate::parser::parse("\\N").expect("parses");
        assert!(!plain.reads_supertokens());
        assert!(!plain.depends_on_whole_input());
    }

    #[test]
    fn constructs_one_of_every_variant() {
        // Acceptance: the AST represents every construct in the
        // spec; build one instance of each node.
        let nodes = vec![
            Pattern::Empty,
            Pattern::Atom(Atom::Kind(TokenKind::Word)),
            Pattern::Atom(Atom::Any),
            Pattern::Atom(Atom::literal("<")),
            Pattern::Atom(Atom::RegisterEq("t".to_string(), OrbitGroup::Identity)),
            Pattern::Atom(Atom::Byte(ByteClass::Digit)),
            Pattern::Bind("t".to_string(), false, Pattern::Atom(Atom::Kind(TokenKind::Word)).boxed()),
            Pattern::Balanced(
                Some(BracketKind::Paren),
                Pattern::Star(Pattern::Atom(Atom::Any).boxed(), Greed::Greedy).boxed(),
            ),
            Pattern::Balanced(None, Pattern::Empty.boxed()),
            Pattern::Guard("ERROR".to_string(), false),
            Pattern::Field(3, Pattern::Empty.boxed()),
            Pattern::Concat(vec![Pattern::Empty, Pattern::Empty]),
            Pattern::Alt(vec![Pattern::Empty, Pattern::Empty], AltMode::First),
            Pattern::Star(Pattern::Empty.boxed(), Greed::Greedy),
            Pattern::Plus(Pattern::Empty.boxed(), Greed::Greedy),
            Pattern::Opt(Pattern::Empty.boxed(), Greed::Lazy),
            Pattern::Repeat(Pattern::Empty.boxed(), 1, Some(3), Greed::Greedy),
        ];
        // Debug renders each node without panicking.
        for n in &nodes {
            assert!(!format!("{n:?}").is_empty());
        }
        assert_eq!(nodes.len(), 17);
    }

    #[test]
    fn nested_quantifiers_collapse() {
        let g = Greed::Greedy;
        let any = || Pattern::Atom(Atom::Any).boxed();
        let star = Pattern::Star(any(), g);
        // (.*)* -> .*
        assert_eq!(normalize(Pattern::Star(star.clone().boxed(), g)), star);
        // (.+)* -> .*
        let plus = Pattern::Plus(any(), g);
        assert_eq!(normalize(Pattern::Star(plus.boxed(), g)), star);
        // (.?)+ -> .*
        let opt = Pattern::Opt(any(), g);
        assert_eq!(normalize(Pattern::Plus(opt.clone().boxed(), g)), star);
        // (.?)? -> .?
        assert_eq!(normalize(Pattern::Opt(opt.clone().boxed(), g)), opt);
    }

    #[test]
    fn quantifiers_leaning_opposite_ways_do_not_collapse() {
        // The Kleene identities hold for the accepted language, not for which
        // accepted length is reported, so folding a lazy inner into a greedy
        // outer would keep the same inputs matching and change their spans.
        let lazy_star = Pattern::Star(Pattern::Atom(Atom::Any).boxed(), Greed::Lazy);
        let nested = Pattern::Star(lazy_star.clone().boxed(), Greed::Greedy);
        assert_eq!(normalize(nested.clone()), nested, "(.*?)* keeps its nesting");

        let greedy_star = Pattern::Star(Pattern::Atom(Atom::Any).boxed(), Greed::Greedy);
        let nested = Pattern::Star(greedy_star.boxed(), Greed::Lazy);
        assert_eq!(normalize(nested.clone()), nested, "(.*)*? keeps its nesting");

        // Matching leans still collapse.
        let lazy = Pattern::Star(Pattern::Atom(Atom::Any).boxed(), Greed::Lazy);
        assert_eq!(normalize(Pattern::Star(lazy.clone().boxed(), Greed::Lazy)), lazy);
    }

    #[test]
    fn a_lazy_fold_that_changes_where_the_form_stands_is_not_made() {
        // `(.??)+?`, `(.??)*?` and `(.+?)??` each fold to a star, which
        // stands at its head after the body has matched where they stand
        // past it, and inside a further loop that ranks the next token and
        // the exit the other way round. They keep their nesting.
        let any = || Pattern::Atom(Atom::Any).boxed();
        let lazy_opt = Pattern::Opt(any(), Greed::Lazy);
        let lazy_plus = Pattern::Plus(any(), Greed::Lazy);
        for nested in [
            Pattern::Plus(lazy_opt.clone().boxed(), Greed::Lazy),
            Pattern::Star(lazy_opt.clone().boxed(), Greed::Lazy),
            Pattern::Opt(lazy_plus.clone().boxed(), Greed::Lazy),
        ] {
            assert_eq!(normalize(nested.clone()), nested, "{nested:?} keeps its nesting");
        }
        // The lazy folds that keep the standing still collapse.
        let lazy_star = Pattern::Star(any(), Greed::Lazy);
        assert_eq!(normalize(Pattern::Plus(lazy_star.clone().boxed(), Greed::Lazy)), lazy_star);
        assert_eq!(normalize(Pattern::Star(lazy_plus.clone().boxed(), Greed::Lazy)), lazy_star);
        assert_eq!(normalize(Pattern::Plus(lazy_plus.clone().boxed(), Greed::Lazy)), lazy_plus);
        assert_eq!(normalize(Pattern::Opt(lazy_star.clone().boxed(), Greed::Lazy)), lazy_star);
        assert_eq!(normalize(Pattern::Opt(lazy_opt.clone().boxed(), Greed::Lazy)), lazy_opt);
        // And every greedy fold.
        let greedy_opt = Pattern::Opt(any(), Greed::Greedy);
        let greedy_plus = Pattern::Plus(any(), Greed::Greedy);
        let star = Pattern::Star(any(), Greed::Greedy);
        assert_eq!(normalize(Pattern::Plus(greedy_opt.boxed(), Greed::Greedy)), star);
        assert_eq!(normalize(Pattern::Opt(greedy_plus.boxed(), Greed::Greedy)), star);
    }

    #[test]
    fn nested_quantifier_with_capture_is_preserved() {
        // ((\W:t))* keeps its nesting so reported captures are unchanged.
        let bind =
            Pattern::Bind("t".to_string(), false, Pattern::Atom(Atom::Kind(TokenKind::Word)).boxed());
        let g = Greed::Greedy;
        let inner_star = Pattern::Star(bind.boxed(), g);
        let nested = Pattern::Star(inner_star.clone().boxed(), g);
        assert_eq!(normalize(nested), Pattern::Star(inner_star.boxed(), g));
    }
}
