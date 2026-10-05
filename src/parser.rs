//! The surface parser: a trex pattern string to a [`Pattern`] AST.
//!
//! The grammar's precedence: ordered choice (loosest), concatenation,
//! binding, quantifier, atom (tightest). An assertion or content guard
//! (`~"lit"`, `~(P)`, `~<(P)`, `!~...`) is an atom: a zero-width item of a
//! concatenation, so a quantifier or binding after one applies to it.
//! Spaces in the pattern are
//! insignificant separators, matching token-mode authoring; to
//! match a literal space token, use `\S` or a quoted literal.

use crate::ast::{AltMode, AnchorKind, Atom, ByteClass, EmptyLoop, Greed, Look, Pattern};
use crate::token::{BracketKind, TokenKind};

/// A parse failure: the byte offset into the pattern and a reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset into the pattern string where parsing stopped.
    pub pos: usize,
    /// Human-readable reason.
    pub msg: String,
}

/// Parse a trex pattern string into a [`Pattern`].
///
/// # Errors
///
/// Returns a [`ParseError`] with a position and reason when the
/// pattern is malformed.
pub fn parse(src: &str) -> Result<Pattern, ParseError> {
    let (rest, _) = split_empty_loop(src)?;
    parse_with_shapes(rest, &crate::custom::ShapeSet::new())
}

/// Parse a pattern that may open with `(?empty:perl)` or
/// `(?empty:thompson)`, returning the tree beside the reading it named.
///
/// The directive is zero-width and governs the whole pattern, so it is read
/// off the front rather than compiled into a node: a reading carried in the
/// tree is one every consumer of the tree can forget to look at, and the
/// consequence of forgetting is a wrong match rather than a failure to build.
///
/// [`parse`] accepts the same directive and discards it, so a caller that does
/// not ask for the reading is not silently given a pattern whose reading it is
/// not honoring - it gets [`EmptyLoop::Thompson`], which is what it would
/// have got anyway.
///
/// # Errors
///
/// Returns a [`ParseError`] when the pattern is malformed, and when the
/// directive names a reading that does not exist.
pub fn parse_with_empty_loop(src: &str) -> Result<(Pattern, EmptyLoop), ParseError> {
    let (rest, mode) = split_empty_loop(src)?;
    Ok((parse_with_shapes(rest, &crate::custom::ShapeSet::new())?, mode))
}

/// Split a leading `(?empty:...)` directive off the front of a pattern.
pub(crate) fn split_empty_loop(src: &str) -> Result<(&str, EmptyLoop), ParseError> {
    const OPEN: &str = "(?empty:";
    let head = src.trim_start();
    let Some(rest) = head.strip_prefix(OPEN) else {
        return Ok((src, EmptyLoop::Thompson));
    };
    let Some(close) = rest.find(')') else {
        return Err(ParseError { pos: src.len(), msg: "unterminated (?empty:...) directive".into() });
    };
    let name = rest[..close].trim();
    let mode = match name {
        "thompson" => EmptyLoop::Thompson,
        "perl" => EmptyLoop::Perl,
        other => {
            return Err(ParseError {
                pos: src.len() - rest.len(),
                msg: format!(
                    "unknown empty-loop reading (?empty:{other}); the readings are `thompson` and `perl`"
                ),
            });
        }
    };
    Ok((&rest[close + 1..], mode))
}

/// Parse a pattern in which `\{name}` may also name a user-declared shape.
///
/// A built-in atom name resolves first, so a shape cannot shadow one; the
/// shape set refuses such a name at declaration anyway, and this ordering
/// keeps that true even for a set built another way.
///
/// # Errors
///
/// Returns a [`ParseError`] with a position and reason when the pattern is
/// malformed, including a `\{name}` matching neither a built-in atom nor a
/// declared shape.
pub fn parse_with_shapes(
    src: &str,
    shapes: &crate::custom::ShapeSet,
) -> Result<Pattern, ParseError> {
    parse_with_shapes_to_depth(src, shapes, NEST_LIMIT)
}

/// [`parse_with_shapes`] refusing a pattern that nests deeper than `limit`
/// groups.
///
/// The depth a caller can afford is a property of the stack it parses on, not
/// of the pattern language, which is why it is a parameter and not only a
/// constant.
pub fn parse_with_shapes_to_depth(
    src: &str,
    shapes: &crate::custom::ShapeSet,
    limit: u32,
) -> Result<Pattern, ParseError> {
    parse_full(src, shapes, &[], limit)
}

/// [`parse_with_shapes`] with the second inputs a join anchor may name
/// (`@echoed:@name`) supplied as bytes under their names, for a caller that
/// holds them rather than files; a name not in `inputs` is read as a path.
///
/// # Errors
///
/// As [`parse_with_shapes`].
pub fn parse_with_inputs(
    src: &str,
    shapes: &crate::custom::ShapeSet,
    inputs: &[(&str, &[u8])],
) -> Result<Pattern, ParseError> {
    parse_full(src, shapes, inputs, NEST_LIMIT)
}

fn parse_full(
    src: &str,
    shapes: &crate::custom::ShapeSet,
    inputs: &[(&str, &[u8])],
    limit: u32,
) -> Result<Pattern, ParseError> {
    let mut p = Parser {
        s: src.as_bytes(),
        i: 0,
        shapes,
        inputs,
        others: std::collections::HashMap::new(),
        depth: 0,
        limit,
        orbit: None,
    };
    let pat = p.parse_alt()?;
    p.skip_spaces();
    if p.i != p.s.len() {
        return Err(p.err("unexpected trailing input"));
    }
    if pat.resume_is_misplaced() {
        return Err(p.err("\\G may only begin a pattern, where it says a match abuts the previous one"));
    }
    // `\K` moves the reported start, which the single-pass engine does by
    // writing the match-start slot again. The set-reachability engine carries
    // no such slot - its start is the position the attempt was anchored at -
    // so a pattern needing that engine cannot also move the start, and saying
    // so is better than reporting a start the `\K` did not move.
    if pat.mentions_reset_start() && crate::nfa::needs_set_engine(&pat) {
        return Err(p.err(
            "\\K cannot be combined with a balanced group, a field anchor, a property anchor, \
             a sub-pattern assertion or a whitespace atom",
        ));
    }
    // Collapse nested quantifiers (Kleene identities) so a pattern like
    // `(.*)*` matches as `.*` rather than forcing the nested-quantifier
    // recompute. Capture-bearing nesting is left intact.
    Ok(crate::ast::normalize(pat))
}

/// How deep a pattern may nest groups before the parser refuses it.
///
/// The parser descends recursively, so nesting depth is stack depth and an
/// unbounded one is a crash rather than an error: a generated or hostile
/// pattern of a few thousand open brackets would overflow the stack with no
/// diagnostic and no way for a caller to recover. A refusal at a stated depth
/// is a `ParseError` a caller can handle.
///
/// The regex crate's `nest_limit` defaults to 250 for the same reason. A limit
/// only keeps its promise below the depth the stack actually holds, and that
/// depth is a property of the build as much as of the parser: an unoptimized
/// frame is several times an optimized one, so one number cannot serve both.
///
/// Measured by `examples/nest_ceiling`, which parses at rising depths in a
/// child process with the guard raised past anything under test, so what it
/// reads is the stack rather than the guard:
///
/// | build | first thread, 1 MB | spawned thread, 2 MB |
/// |---|---|---|
/// | release | 499 | deeper still |
/// | debug | 74 | 151 |
///
/// So 250 holds in release with room to spare and is unreachable in debug on
/// either kind of thread, which is the build every `cargo test` runs. The
/// debug figure is also measured on the cheapest shape there is, `((("a")))`,
/// and a level carrying an alternation or a repetition costs more than a bare
/// group, so the limit is well under even the 74 rather than just beneath
/// it. Neither number constrains a pattern anyone writes; nesting past a
/// handful of groups is already unreadable.
#[cfg(debug_assertions)]
pub const NEST_LIMIT: u32 = 32;

/// As above.
#[cfg(not(debug_assertions))]
pub const NEST_LIMIT: u32 = 250;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    shapes: &'a crate::custom::ShapeSet,
    /// Second inputs the caller supplied by name, read before any file.
    inputs: &'a [(&'a str, &'a [u8])],
    /// The second inputs this parse has read and lexed, by name, so a name
    /// two anchors write is lexed once.
    others: std::collections::HashMap<String, std::sync::Arc<crate::ast::OtherInput>>,
    /// Groups currently open. Every recursive descent passes through
    /// [`Parser::parse_alt`], so counting there counts all of them.
    depth: u32,
    limit: u32,
    /// The rung of the `(?orbit:G ...)` scope being parsed, where one is
    /// open. A back-reference whose own spelling names no rung takes it, and
    /// one that names a rung keeps what it names, which is a distinction only
    /// the parser can make - by the time [`set_orbit`] walks the tree the two
    /// spellings have become the same atom.
    orbit: Option<crate::orbit::OrbitGroup>,
}

/// Rewrite every literal in `pat` to compare under `group`.
///
/// A register-equality atom keeps whichever group its own `=shape x` spelling
/// gave it: that is a comparison the author already named, and a surrounding
/// scope silently widening it would make the pattern mean something other than
/// what was written.
pub(crate) fn set_orbit(pat: &mut Pattern, group: crate::orbit::OrbitGroup) {
    match pat {
        Pattern::Atom(a) => a.set_orbit(group),
        Pattern::Within(v, _) => {
            for e in v {
                e.atom.set_orbit(group);
            }
        }
        // An echo anchor counts recurrence at the rung the scope names, and
        // a join anchor keys the second input at it.
        Pattern::Anchor(crate::ast::AnchorKind::Echo(_, g)) => *g = group,
        Pattern::Anchor(crate::ast::AnchorKind::Joined { group: g, .. }) => *g = group,
        Pattern::Empty | Pattern::Guard(..) | Pattern::Anchor(_) => {}
        Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) | Pattern::Bind(_, _, p) => {
            set_orbit(p, group);
        }
        Pattern::Repeat(p, _, _, _)
        | Pattern::Balanced(_, p)
        | Pattern::Field(_, p)
        | Pattern::Atomic(p)
        | Pattern::Assert(p, _, _) => set_orbit(p, group),
        Pattern::Concat(v) | Pattern::Alt(v, _) => {
            for p in v {
                set_orbit(p, group);
            }
        }
    }
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> ParseError {
        ParseError { pos: self.i, msg: msg.to_string() }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.i += 1;
        }
        b
    }

    fn skip_spaces(&mut self) {
        while self.peek() == Some(b' ') {
            self.i += 1;
        }
    }

    fn parse_alt(&mut self) -> Result<Pattern, ParseError> {
        // Every recursion into a nested group arrives here, so the depth is
        // counted once rather than at each of the nine places that descend.
        self.depth += 1;
        if self.depth > self.limit {
            let limit = self.limit;
            // Unwound before returning, so a caller that recovers from this
            // error and parses again starts from a depth of zero.
            self.depth -= 1;
            return Err(self.err(&format!("pattern nests deeper than {limit} groups")));
        }
        let out = self.parse_alt_inner();
        self.depth -= 1;
        out
    }

    fn parse_alt_inner(&mut self) -> Result<Pattern, ParseError> {
        let mut alts = vec![self.parse_concat()?];
        let mut mode: Option<AltMode> = None;
        loop {
            self.skip_spaces();
            let Some(m) = self.peek_alt_op() else { break };
            let at = self.i;
            self.bump();
            if m != AltMode::First {
                self.bump();
            }
            // Two kinds at one level would have to resolve by precedence, and
            // any precedence chosen here is a rule the reader has to know to
            // predict the match. Parentheses say it instead.
            if let Some(prev) = mode
                && prev != m
            {
                return Err(ParseError {
                    pos: at,
                    msg: format!(
                        "mixed alternation kinds ({} then {}) at one level; parenthesize to say which binds tighter",
                        prev.glyph(),
                        m.glyph()
                    ),
                });
            }
            mode = Some(m);
            alts.push(self.parse_concat()?);
        }
        if alts.len() == 1 {
            Ok(alts.pop().expect("one alternative"))
        } else {
            Ok(Pattern::Alt(alts, mode.unwrap_or(AltMode::First)))
        }
    }

    /// The alternation operator at the cursor, without consuming it. All three
    /// start with `|`, so a bare `/` stays a literal -
    /// `</=t>` in a close-tag pattern must keep parsing as punctuation.
    fn peek_alt_op(&self) -> Option<AltMode> {
        if self.peek() != Some(b'|') {
            return None;
        }
        match self.s.get(self.i + 1) {
            Some(b'|') => Some(AltMode::Longest),
            Some(b'>') => Some(AltMode::Committed),
            _ => Some(AltMode::First),
        }
    }

    fn parse_concat(&mut self) -> Result<Pattern, ParseError> {
        let mut items: Vec<Pattern> = Vec::new();
        loop {
            self.skip_spaces();
            match self.peek() {
                None | Some(b'|') | Some(b')') | Some(b']') | Some(b'}') => break,
                _ => items.push(self.parse_postfix()?),
            }
        }
        if items.is_empty() {
            Ok(Pattern::Empty)
        } else if items.len() == 1 {
            Ok(items.pop().unwrap())
        } else {
            Ok(Pattern::Concat(items))
        }
    }

    fn parse_postfix(&mut self) -> Result<Pattern, ParseError> {
        let mut atom = self.parse_atom()?;
        // A predicate body is part of the atom, so a quantifier after it
        // applies to the predicated atom: `\N{>500}+` is one or more numbers
        // over five hundred. `{...}` holding only digits and a comma is a
        // repeat count and falls through to the quantifiers below.
        let kind = match &atom {
            Pattern::Atom(Atom::Kind(k)) => Some(*k),
            _ => None,
        };
        if let Some(k) = kind
            && self.peek() == Some(b'{')
            && self.peek_is_kind_predicate()
        {
            atom = self.parse_kind_predicate(k)?;
        } else if let Pattern::Atom(Atom::Class(c)) = &atom
            && Self::is_quantity_class(c)
            && self.peek() == Some(b'{')
            && self.peek_is_kind_predicate()
        {
            // `\{qty}{>5kg}`: the predicate is read once as a quantity's and
            // applied to every member kind, each read in its own units.
            let class = c.clone();
            atom = self.parse_class_predicate(class)?;
        }
        // Quantifier binds tighter than binding.
        let mut possessive = false;
        match self.peek() {
            Some(b'*') => {
                self.bump();
                let g = self.lean();
                possessive = self.possessive();
                atom = Pattern::Star(atom.boxed(), g);
            }
            Some(b'+') => {
                self.bump();
                let g = self.lean();
                possessive = self.possessive();
                atom = Pattern::Plus(atom.boxed(), g);
            }
            Some(b'?') => {
                self.bump();
                let g = self.lean();
                possessive = self.possessive();
                atom = Pattern::Opt(atom.boxed(), g);
            }
            Some(b'{') => {
                atom = self.parse_repeat(atom)?;
                possessive = self.possessive();
            }
            _ => {}
        }
        if possessive {
            atom = Pattern::Atomic(atom.boxed());
        }
        // Binding suffix. `:name` binds for the rest of the match; `::name`
        // is scoped to the enclosing balanced group (dropped when it closes).
        if self.peek() == Some(b':') {
            self.bump();
            let scoped = self.peek() == Some(b':');
            if scoped {
                self.bump();
            }
            let name = self.parse_name()?;
            // A register bound inside a bound pattern nests under it, as
            // `pair.k`, whether the inner bindings were written in place or
            // inlined from a let.
            if atom.binds_anything() {
                atom.nest_registers(&name);
            }
            atom = Pattern::Bind(name, scoped, atom.boxed());
        }
        Ok(atom)
    }

    /// Consume a trailing `?` marking the quantifier just read as lazy.
    ///
    /// `??` is therefore an optional that prefers to match nothing, which is
    /// what the same spelling means in every regex dialect.
    fn lean(&mut self) -> Greed {
        if self.peek() == Some(b'?') {
            self.bump();
            Greed::Lazy
        } else {
            Greed::Greedy
        }
    }

    /// Whether a trailing `+` marks the quantifier just read as possessive.
    ///
    /// A possessive quantifier is its greedy form with the other lengths
    /// discarded, so it is read here and wrapped in [`Pattern::Atomic`]
    /// rather than carried as a third [`Greed`]. Nothing skips whitespace
    /// first, so `\W+ +` is a plus followed by a punctuation literal, not a
    /// possessive one.
    fn possessive(&mut self) -> bool {
        if self.peek() == Some(b'+') {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Whether the `{...}` at the cursor is a predicate rather than a repeat
    /// count: a repeat holds only digits, a comma and spaces. Does not consume
    /// input.
    fn peek_is_kind_predicate(&self) -> bool {
        let mut j = self.i + 1;
        while j < self.s.len() && self.s[j] != b'}' {
            if !(self.s[j].is_ascii_digit() || self.s[j] == b',' || self.s[j] == b' ') {
                return true;
            }
            j += 1;
        }
        false
    }

    /// Consume a `{...}` predicate body on a kind atom (the cursor is at `{`).
    ///
    /// `mag` names the magnitude axis (`\N{mag>3}`), as does a signed
    /// threshold, which is relative to a context (`\N{>+1}`, `\N{<-1:k}`); a
    /// Number token is never negative, so a sign after the operator can mean
    /// nothing else. Every other body is a typed value predicate, read in the
    /// kind's own units by [`crate::typed::TypedPred::parse`].
    fn parse_kind_predicate(&mut self, kind: TokenKind) -> Result<Pattern, ParseError> {
        self.expect(b'{')?;
        let start = self.i;
        while !matches!(self.peek(), None | Some(b'}')) {
            self.bump();
        }
        if self.peek() != Some(b'}') {
            return Err(self.err("unterminated predicate"));
        }
        let raw = &self.s[start..self.i];
        self.bump();
        let body = String::from_utf8_lossy(raw).into_owned();
        let trimmed = body.trim();
        let after_op = trimmed.trim_start_matches(['>', '<', '=']).trim_start();
        let magnitude = trimmed.starts_with("mag")
            || (trimmed.starts_with(['>', '<']) && after_op.starts_with(['+', '-']));
        // `\T{>+1h:t}`: this instant read against the one the register holds,
        // which is the last instant bound to it. A written instant carries
        // colons of its own, so only a signed duration reads this way.
        if kind == TokenKind::Timestamp
            && trimmed.starts_with(['>', '<'])
            && after_op.starts_with(['+', '-'])
            && let Some((body, reg)) = trimmed.rsplit_once(':')
        {
            return Self::since_atom(body, reg).map_err(|m| ParseError { pos: start, msg: m });
        }
        if magnitude {
            let pred = Self::magnitude_pred(raw).map_err(|m| self.err(&m))?;
            return Ok(Pattern::Atom(Atom::KindMag(kind, pred)));
        }
        match crate::typed::TypedPred::parse_in(kind, trimmed, self.shapes) {
            Ok(pred) => Ok(Pattern::Atom(Atom::KindPred(kind, pred))),
            Err(msg) => Err(ParseError { pos: start, msg }),
        }
    }

    /// Whether a class holds only kinds a quantity predicate reads: the
    /// quantity, byte-size, duration and percentage kinds and declared or
    /// library kinds, with no intersection, subtraction or negation.
    fn is_quantity_class(c: &crate::ast::TokenClass) -> bool {
        !c.negated
            && c.all.is_empty()
            && c.none.is_empty()
            && c.any.iter().all(|m| {
                matches!(
                    m,
                    Atom::Kind(
                        TokenKind::Quantity
                            | TokenKind::ByteSize
                            | TokenKind::Duration
                            | TokenKind::Percent
                            | TokenKind::Custom(_)
                    )
                )
            })
    }

    /// The `{...}` after a quantity class: parsed as a predicate on the
    /// quantity kind, then placed on every member kind of the class.
    fn parse_class_predicate(
        &mut self,
        mut class: Box<crate::ast::TokenClass>,
    ) -> Result<Pattern, ParseError> {
        let on_quantity = self.parse_kind_predicate(TokenKind::Quantity)?;
        for member in &mut class.any {
            let Atom::Kind(k) = *member else {
                continue;
            };
            *member = match &on_quantity {
                Pattern::Atom(Atom::KindPred(_, pred)) => Atom::KindPred(k, pred.clone()),
                Pattern::Atom(Atom::KindMag(_, mag)) => Atom::KindMag(k, mag.clone()),
                _ => continue,
            };
        }
        Ok(Pattern::Atom(Atom::Class(class)))
    }

    /// The class `\{qty}` names: every kind a quantity predicate reads - the
    /// quantity, byte-size, duration and percentage kinds, and each unit kind
    /// the library reads from the context, or the declaration shadowing it.
    fn quantity_class(&self) -> crate::ast::TokenClass {
        let mut any = vec![
            Atom::Kind(TokenKind::Quantity),
            Atom::Kind(TokenKind::ByteSize),
            Atom::Kind(TokenKind::Duration),
            Atom::Kind(TokenKind::Percent),
        ];
        for name in crate::library::context_kind_names() {
            let id = match self.shapes.id_of(name) {
                Some(id) => Some(id),
                None if self.shapes.consults_library() => crate::library::id_of(name),
                None => None,
            };
            if let Some(id) = id {
                any.push(Atom::Kind(TokenKind::Custom(id)));
            }
        }
        crate::ast::TokenClass { any, all: Vec::new(), none: Vec::new(), negated: false }
    }

    /// The atom `\T{>+1h:t}` writes: an ordering, a signed duration and the
    /// register whose instant it is measured from.
    fn since_atom(body: &str, reg: &str) -> Result<Pattern, String> {
        use crate::ast::{Cmp, Signed};
        let body = body.trim();
        let (op, rest) = if let Some(r) = body.strip_prefix(">=") {
            (Cmp::Ge, r)
        } else if let Some(r) = body.strip_prefix("<=") {
            (Cmp::Le, r)
        } else if let Some(r) = body.strip_prefix('>') {
            (Cmp::Gt, r)
        } else if let Some(r) = body.strip_prefix('<') {
            (Cmp::Lt, r)
        } else {
            return Err("a stream-relative instant needs an ordering: \\T{>+1h:t}".to_string());
        };
        let rest = rest.trim();
        let (negative, mag) = match rest.strip_prefix('-') {
            Some(m) => (true, m),
            None => (false, rest.strip_prefix('+').unwrap_or(rest)),
        };
        let Some(nanos) = crate::typed::parse_duration(mag) else {
            return Err(format!("{mag:?} is not a duration; write the unit: 500ms, 2s, 1h30m"));
        };
        if reg.is_empty() || !reg.bytes().all(|b| b == b'_' || b.is_ascii_alphanumeric()) {
            return Err(format!("{reg:?} is not a register name"));
        }
        Ok(Pattern::Atom(Atom::Since(op, Signed { negative, nanos }, reg.to_string())))
    }

    fn parse_repeat(&mut self, inner: Pattern) -> Result<Pattern, ParseError> {
        // Consumes `{m}`, `{m,}`, or `{m,n}`.
        self.bump(); // '{'
        let m = self.parse_number()?;
        let n = if self.peek() == Some(b',') {
            self.bump();
            if self.peek() == Some(b'}') {
                None
            } else {
                Some(self.parse_number()?)
            }
        } else {
            Some(m)
        };
        if self.peek() != Some(b'}') {
            return Err(self.err("expected '}' to close a quantifier"));
        }
        self.bump();
        let g = self.lean();
        Ok(Pattern::Repeat(inner.boxed(), m, n, g))
    }

    fn parse_atom(&mut self) -> Result<Pattern, ParseError> {
        self.skip_spaces();
        match self.peek() {
            Some(b'\\') => {
                self.bump();
                let c = self.bump().ok_or_else(|| self.err("dangling backslash"))?;
                self.atom_from_escape(c)
            }
            Some(b'.') => {
                self.bump();
                Ok(Pattern::Atom(Atom::Any))
            }
            Some(b'(') => {
                self.bump();
                // `(?orbit:G P)` scopes a symmetry group over P; a bare `(P)`
                // is a logical group.
                if self.peek() == Some(b'?') {
                    return self.parse_modifier_group();
                }
                let inner = self.parse_alt()?;
                self.expect(b')')?;
                // `(A B C)~k`: a run of tokens within `k` token edits of the
                // group's atoms. The count is always written, as it is after
                // a literal, and a `~` before anything but a digit opens the
                // assertion that follows the group.
                if self.peek() == Some(b'~') && self.s.get(self.i + 1).is_some_and(u8::is_ascii_digit) {
                    self.bump();
                    let k = self.parse_number()?;
                    let k = u8::try_from(k)
                        .map_err(|e| self.err(&format!("an edit count fits one byte: {e}")))?;
                    return self.edit_group(inner, k);
                }
                Ok(inner)
            }
            Some(b'"') => {
                let lit = self.read_quoted()?;
                // `"lit"~k`: a token within `k` edits of the literal. The
                // count is always written; a `~` before anything but a digit
                // opens the assertion that follows the literal.
                if self.peek() == Some(b'~') && self.s.get(self.i + 1).is_some_and(u8::is_ascii_digit) {
                    self.bump();
                    let k = self.parse_number()?;
                    let k = u8::try_from(k)
                        .map_err(|e| self.err(&format!("an edit count fits one byte: {e}")))?;
                    return Ok(Pattern::Atom(Atom::LiteralWithin(
                        lit,
                        k,
                        crate::orbit::OrbitGroup::Identity,
                    )));
                }
                Ok(Pattern::Atom(Atom::literal(&lit)))
            }
            Some(b'=') => {
                self.bump();
                let first = self.parse_name()?;
                // `=editk x`: a later token within `k` edits of the bound
                // one. Read as a rung only when a register name follows, as
                // the groups and relations below are.
                if let Some(digits) = first.strip_prefix("edit")
                    && !digits.is_empty()
                    && digits.bytes().all(|b| b.is_ascii_digit())
                {
                    let save = self.i;
                    self.skip_spaces();
                    if matches!(self.peek(), Some(c) if c == b'_' || c.is_ascii_alphabetic()) {
                        let name = self.parse_name()?;
                        let k = digits
                            .parse::<u8>()
                            .map_err(|e| self.err(&format!("an edit count fits one byte: {e}")))?;
                        return Ok(Pattern::Atom(Atom::RegisterWithin(
                            name,
                            k,
                            crate::orbit::OrbitGroup::Identity,
                        )));
                    }
                    self.i = save;
                }
                // `=kin x` / `=kin:byte x` / `=kin:super x`: the token and the
                // bound span's unit share a type or a placed gravity class in
                // the input's pair field. As with the relations below, only a
                // register name after it makes it one, so `=kin` alone is a
                // register named kin.
                if first == "kin" {
                    let save = self.i;
                    let grain = self.gravity_grain()?;
                    self.skip_spaces();
                    if matches!(self.peek(), Some(c) if c == b'_' || c.is_ascii_alphabetic()) {
                        let name = self.parse_name()?;
                        return Ok(Pattern::Atom(Atom::RegisterKin(name, grain)));
                    }
                    self.i = save;
                }
                // `=subnet/24 x` / `=domain x` / `=day x`: the bound span and
                // the token compared through a typed relation. Only a
                // register name after the keyword makes it one, so a register
                // literally named `domain` still works as `=domain` on its
                // own.
                //
                // Read before the orbit groups because every relation is also
                // a rung of the same name, and the two differ over a
                // timestamp that writes no year: the relation does not hold
                // the absent year against a written one and the rung, which
                // is a key, cannot do that and stay an equivalence. What the
                // author wrote after `=` is a relation, so it is read as one.
                if let Some(mut relation) = crate::typed::Relation::parse(&first) {
                    let save = self.i;
                    if self.peek() == Some(b'/') {
                        self.bump();
                        let bits = self.parse_number()?;
                        if bits > 128 {
                            return Err(self.err("a prefix length is at most 128"));
                        }
                        relation = relation.with_prefix(bits as u8).ok_or_else(|| {
                            self.err("only =subnet takes a prefix length after a slash")
                        })?;
                    }
                    self.skip_spaces();
                    if matches!(self.peek(), Some(c) if c == b'_' || c.is_ascii_alphabetic()) {
                        let name = self.parse_name()?;
                        return Ok(Pattern::Atom(Atom::RegisterRelated(name, relation)));
                    }
                    self.i = save;
                }
                // `=shape x` / `=case x` / `=notation x`: compare the bound
                // span under an orbit group instead of byte-for-byte, so the
                // reference matches every token in the bound token's orbit.
                // Plain `=x` is exact equality (the Identity orbit), or the
                // rung of the scope it is in.
                if let Some(group) = crate::orbit::OrbitGroup::parse(&first) {
                    let save = self.i;
                    self.skip_spaces();
                    if matches!(self.peek(), Some(c) if c == b'_' || c.is_ascii_alphabetic()) {
                        let name = self.parse_name()?;
                        return Ok(Pattern::Atom(Atom::RegisterEq(name, group)));
                    }
                    self.i = save;
                }
                // A plain `=x` inside a scope compares at the scope's rung:
                // the author named no comparison of their own, so the one the
                // scope names is the one they wrote.
                Ok(Pattern::Atom(Atom::RegisterEq(
                    first,
                    self.orbit.unwrap_or(crate::orbit::OrbitGroup::Identity),
                )))
            }
            Some(b'~') => {
                self.bump();
                self.parse_assertion(false)
            }
            Some(b'!') => {
                // `!~"lit"`: a negative content guard - the forward window
                // must not contain `lit`. The `!` requires a `~` after it.
                self.bump();
                if self.peek() != Some(b'~') {
                    return Err(self.err("expected '~' after '!' for a negative assertion"));
                }
                self.bump();
                self.parse_assertion(true)
            }
            Some(b'`') => {
                self.bump();
                let start = self.i;
                while !matches!(self.peek(), None | Some(b'`')) {
                    self.bump();
                }
                if self.peek() != Some(b'`') {
                    return Err(self.err("unterminated byte-pattern"));
                }
                let raw = &self.s[start..self.i];
                self.bump();
                match crate::bytepat::parse(raw) {
                    Ok(bp) => Ok(Pattern::Atom(Atom::BytePattern(bp))),
                    Err(msg) => Err(self.err(&msg)),
                }
            }
            Some(b'[') => {
                self.bump();
                self.parse_class()
            }
            Some(b'#') => {
                // `#"W(W,W)"` is a silhouette: match by structural form
                // regardless of content. Each template letter is a token
                // class (`W` word, `N` number, `Q` quoted, `.` any, plus the
                // typed-atom letters), each other character a literal. A bare
                // `#` not followed by a quote is a literal `#`.
                self.bump();
                if self.peek() == Some(b'"') {
                    let tmpl = self.read_quoted()?;
                    self.expand_silhouette(&tmpl)
                } else {
                    Ok(Pattern::Atom(Atom::literal("#")))
                }
            }
            Some(b'@') => {
                self.bump();
                // `@` then a digit is a field anchor; `@` then a letter
                // is a cross-language structural lens.
                if matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    let k = self.parse_number()?;
                    // `@k` anchors the following atom or group to the k-th
                    // comma-delimited field. Use a group for multi-atom
                    // fields: `@3 (\W \W)`.
                    let inner = self.parse_atom()?;
                    Ok(Pattern::Field(k, inner.boxed()))
                } else {
                    self.parse_lens()
                }
            }
            // Positional anchors, spelled as regex spells them. They
            // constrain where a token is rather than what it is, which is
            // the axis the `@` property anchors do not cover.
            Some(b'^') => {
                self.bump();
                Ok(Pattern::Anchor(AnchorKind::LineStart))
            }
            Some(b'$') => {
                self.bump();
                Ok(Pattern::Anchor(AnchorKind::LineEnd))
            }
            Some(c) => {
                self.bump();
                Ok(Pattern::Atom(Atom::literal(&(c as char).to_string())))
            }
            None => Err(self.err("expected a pattern atom")),
        }
    }

    /// Expand a silhouette template (`#"W(W,W)"`) into a token sequence.
    /// A template letter that names a token class becomes that kind atom, `.`
    /// becomes any-token, and every other character becomes a literal of that
    /// character (so `(`, `,`, `)` match the punctuation they draw). Spaces in
    /// the template are insignificant separators. The result matches a span by
    /// its structural silhouette, whatever the identifiers or values are.
    fn expand_silhouette(&self, tmpl: &str) -> Result<Pattern, ParseError> {
        let mut items: Vec<Pattern> = Vec::new();
        for c in tmpl.chars() {
            if c == ' ' {
                continue;
            }
            let atom = match c {
                'W' => Atom::Kind(TokenKind::Word),
                'N' => Atom::Kind(TokenKind::Number),
                'Q' => Atom::Kind(TokenKind::Quoted),
                'I' => Atom::Kind(TokenKind::Ip),
                'U' => Atom::Kind(TokenKind::Url),
                'E' => Atom::Kind(TokenKind::Email),
                'T' => Atom::Kind(TokenKind::Timestamp),
                'P' => Atom::Kind(TokenKind::Punct),
                'V' => Atom::Kind(TokenKind::Version),
                'A' => Atom::Kind(TokenKind::Mac),
                'H' => Atom::Kind(TokenKind::HexColor),
                'C' => Atom::Kind(TokenKind::Cidr),
                '.' => Atom::Any,
                other => Atom::literal(&other.to_string()),
            };
            items.push(Pattern::Atom(atom));
        }
        match items.len() {
            0 => Err(self.err("empty silhouette template")),
            1 => Ok(items.pop().unwrap()),
            _ => Ok(Pattern::Concat(items)),
        }
    }

    fn atom_from_escape(&mut self, c: u8) -> Result<Pattern, ParseError> {
        let kind = match c {
            b'N' => Some(TokenKind::Number),
            b'W' => Some(TokenKind::Word),
            b'Q' => Some(TokenKind::Quoted),
            b'I' => Some(TokenKind::Ip),
            b'U' => Some(TokenKind::Url),
            b'E' => Some(TokenKind::Email),
            b'T' => Some(TokenKind::Timestamp),
            b'P' => Some(TokenKind::Punct),
            b'S' => Some(TokenKind::Whitespace),
            b'V' => Some(TokenKind::Version),
            b'H' => Some(TokenKind::HexColor),
            b'C' => Some(TokenKind::Cidr),
            b'Z' => Some(TokenKind::ByteSize),
            b'%' => Some(TokenKind::Percent),
            b'$' => Some(TokenKind::Money),
            b'D' => Some(TokenKind::HashDigest),
            b'R' => Some(TokenKind::Duration),
            b'L' => Some(TokenKind::Path),
            _ => None,
        };
        if let Some(k) = kind {
            return Ok(Pattern::Atom(Atom::Kind(k)));
        }
        match c {
            b'B' => self.parse_balanced(),
            // The input anchors, spelled as regex spells them. `\A` names the
            // start of the stream; `^` stays the start of a line, which is the
            // reading line-oriented input wants and so keeps the plain glyph.
            // A MAC address is `\{mac}`.
            b'A' => Ok(Pattern::Anchor(AnchorKind::InputStart)),
            b'z' => Ok(Pattern::Anchor(AnchorKind::InputEnd)),
            // `\G` says where a match may begin, which is decided when the
            // scan picks its non-overlapping run rather than while matching
            // one. Anywhere but the head of the pattern it would be asking
            // whether an interior token is where the previous match ended,
            // and for a non-overlapping run that is never true, so the parser
            // refuses it there rather than accepting a construct that can
            // only fail.
            b'G' => Ok(Pattern::Anchor(AnchorKind::Resume)),
            b'K' => Ok(Pattern::Anchor(AnchorKind::ResetStart)),
            b'd' => Ok(Pattern::Atom(Atom::Byte(ByteClass::Digit))),
            b'w' => Ok(Pattern::Atom(Atom::Byte(ByteClass::Word))),
            b's' => Ok(Pattern::Atom(Atom::Byte(ByteClass::Space))),
            b'h' => Ok(Pattern::Atom(Atom::Byte(ByteClass::Hex))),
            b'a' => Ok(Pattern::Atom(Atom::Byte(ByteClass::Alpha))),
            b'u' => Ok(Pattern::Atom(Atom::Byte(ByteClass::Upper))),
            b'l' => Ok(Pattern::Atom(Atom::Byte(ByteClass::Lower))),
            b'F' => self.parse_spectral(),
            b'M' => self.parse_magnitude(),
            b'{' => self.parse_named_atom(),
            // Any other escaped byte is a literal of that byte.
            other => Ok(Pattern::Atom(Atom::literal(&(other as char).to_string()))),
        }
    }

    /// Parse a `\{name}` named atom - the brace-delimited kind name (the opening
    /// brace is already consumed) mapped to its token kind. The named form is
    /// how atoms scale past the single-letter escapes: `\{jwt}`, `\{creditcard}`,
    /// and the letter atoms are also reachable by their full names (`\{ip}`).
    fn parse_named_atom(&mut self) -> Result<Pattern, ParseError> {
        fn kind_by_name(name: &str) -> Option<TokenKind> {
            Some(match name {
                "number" => TokenKind::Number,
                "word" => TokenKind::Word,
                "quoted" => TokenKind::Quoted,
                "ip" => TokenKind::Ip,
                "url" => TokenKind::Url,
                "email" => TokenKind::Email,
                "timestamp" => TokenKind::Timestamp,
                "punct" => TokenKind::Punct,
                "whitespace" => TokenKind::Whitespace,
                "version" => TokenKind::Version,
                "uuid" => TokenKind::Uuid,
                "mac" => TokenKind::Mac,
                "hexcolor" => TokenKind::HexColor,
                "cidr" => TokenKind::Cidr,
                "bytesize" => TokenKind::ByteSize,
                "percent" => TokenKind::Percent,
                "money" => TokenKind::Money,
                "hash" | "hashdigest" => TokenKind::HashDigest,
                "duration" => TokenKind::Duration,
                "path" => TokenKind::Path,
                "jwt" => TokenKind::Jwt,
                "creditcard" | "card" => TokenKind::CreditCard,
                "base64" | "b64" => TokenKind::Base64,
                "hex" => TokenKind::Hex,
                "geo" | "coord" => TokenKind::Geo,
                "phone" | "tel" => TokenKind::Phone,
                "quantity" => TokenKind::Quantity,
                _ => return None,
            })
        }
        let start = self.i;
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c == b'}' {
                break;
            }
            name.push(c as char);
            self.bump();
        }
        if self.peek() != Some(b'}') {
            return Err(ParseError { pos: start, msg: "unterminated \\{name}".to_string() });
        }
        self.bump();
        if let Some(k) = kind_by_name(&name) {
            return Ok(Pattern::Atom(Atom::Kind(k)));
        }
        if name == "qty" {
            return Ok(Pattern::Atom(Atom::Class(Box::new(self.quantity_class()))));
        }
        // A declaration first, then the shipped library, so a declaration
        // shadows an entry of its name; the library itself is parsed with a
        // set that does not consult it.
        if let Some(id) = self.shapes.id_of(&name) {
            return Ok(Pattern::Atom(Atom::Kind(TokenKind::Custom(id))));
        }
        if let Some(p) = self.shapes.let_of(&name) {
            return Ok(p.clone());
        }
        if self.shapes.consults_library() {
            if let Some(p) = crate::library::let_of(&name) {
                return Ok(p.clone());
            }
            if let Some(id) = crate::library::id_of(&name) {
                return Ok(Pattern::Atom(Atom::Kind(TokenKind::Custom(id))));
            }
        }
        Err(ParseError {
            pos: start,
            msg: format!(
                "unknown named atom \\{{{name}}}; it is neither a built-in, a declared shape, kind or sub-pattern, nor a library entry"
            ),
        })
    }

    /// Parse a spectral atom `\F{ <pred> }`: the token's pooled spectral
    /// signature read as a match condition.
    fn parse_spectral(&mut self) -> Result<Pattern, ParseError> {
        self.expect(b'{')?;
        let start = self.i;
        while !matches!(self.peek(), None | Some(b'}')) {
            self.bump();
        }
        if self.peek() != Some(b'}') {
            return Err(self.err("unterminated \\F{...} spectral predicate"));
        }
        let raw = &self.s[start..self.i];
        self.bump(); // consume '}'
        let pred = Self::spectral_pred(raw).map_err(|m| self.err(&m))?;
        Ok(Pattern::Atom(Atom::Spectral(pred)))
    }

    /// Parse the body of a `\F{...}` predicate.
    fn spectral_pred(raw: &[u8]) -> Result<crate::ast::SpectralPred, String> {
        use crate::ast::{SpecTexture, SpectralPred};
        let s = std::str::from_utf8(raw)
            .map_err(|_| "spectral predicate is not UTF-8".to_string())?
            .trim();
        if let Some(rest) = s.strip_prefix("entropy") {
            let rest = rest.trim();
            let (ge, num) = if let Some(n) = rest.strip_prefix(">=").or_else(|| rest.strip_prefix('>')) {
                (true, n)
            } else if let Some(n) = rest.strip_prefix("<=").or_else(|| rest.strip_prefix('<')) {
                (false, n)
            } else {
                return Err("entropy needs a > or < threshold, e.g. entropy>0.8".to_string());
            };
            let v: f32 = num.trim().parse().map_err(|_| format!("bad entropy threshold {num:?}"))?;
            let pct = (v * 100.0).round().clamp(0.0, 100.0) as u8;
            return Ok(if ge { SpectralPred::EntropyGe(pct) } else { SpectralPred::EntropyLe(pct) });
        }
        if let Some(rest) = s.strip_prefix("period") {
            let n = rest.trim().trim_start_matches([':', '=']).trim();
            // Empty, `any`, or the named `line` period all match any strong
            // detected periodicity; a number requires that exact period.
            if n.is_empty() || n == "any" || n == "line" {
                return Ok(SpectralPred::PeriodAny);
            }
            let p: u16 = n.parse().map_err(|_| format!("bad period {n:?}"))?;
            return Ok(SpectralPred::PeriodEq(p));
        }
        if let Some(rest) = s.strip_prefix("texture:").or_else(|| s.strip_prefix("texture=")) {
            return match rest.trim() {
                "prose" => Ok(SpectralPred::Texture(SpecTexture::Prose)),
                "code" => Ok(SpectralPred::Texture(SpecTexture::Code)),
                "math" => Ok(SpectralPred::Texture(SpecTexture::Math)),
                "data" => Ok(SpectralPred::Texture(SpecTexture::Data)),
                other => Err(format!("unknown texture {other:?} (prose|code|math|data)")),
            };
        }
        if s == "onset" {
            return Ok(SpectralPred::Onset);
        }
        Err(format!("unknown spectral predicate {s:?} (entropy|period|texture|onset)"))
    }

    /// Parse a magnitude atom `\M{ <pred> }`: the token's order of magnitude
    /// read as a match condition. `\M{>6}` matches a token whose magnitude
    /// exceeds 6 (for a Number, a value over ~1e6; for any other token, a
    /// byte length over ~64). `mag` is an optional readability prefix, so
    /// `\M{mag>6}` is the same predicate.
    fn parse_magnitude(&mut self) -> Result<Pattern, ParseError> {
        self.expect(b'{')?;
        let start = self.i;
        while !matches!(self.peek(), None | Some(b'}')) {
            self.bump();
        }
        if self.peek() != Some(b'}') {
            return Err(self.err("unterminated \\M{...} magnitude predicate"));
        }
        let raw = &self.s[start..self.i];
        self.bump(); // consume '}'
        let pred = Self::magnitude_pred(raw).map_err(|m| self.err(&m))?;
        Ok(Pattern::Atom(Atom::Magnitude(pred)))
    }

    /// Parse the body of a `\M{...}` predicate. `>=` and `>` both mean "at
    /// least"; `<=` and `<` both mean "at most", the same collapse the
    /// spectral entropy predicate uses.
    ///
    /// A signed threshold is relative: `>+1` is an order of magnitude above
    /// the context's mean, `>+2s` two standard deviations above it, `<-1` an
    /// order below. `:name` after it names the context - `window` (the
    /// default), `phase`, `regime`, `echo`, `enclosing`, or a register whose
    /// key's value history is the baseline.
    fn magnitude_pred(raw: &[u8]) -> Result<crate::ast::MagPred, String> {
        use crate::ast::{Delta, MagPred, Scope};
        let s = std::str::from_utf8(raw)
            .map_err(|e| format!("magnitude predicate is not UTF-8: {e}"))?
            .trim();
        // Optional `mag` readability prefix: `\M{mag>6}` == `\M{>6}`.
        let s = s.strip_prefix("mag").map_or(s, str::trim_start).trim();
        let (ge, num) = if let Some(n) = s.strip_prefix(">=").or_else(|| s.strip_prefix('>')) {
            (true, n)
        } else if let Some(n) = s.strip_prefix("<=").or_else(|| s.strip_prefix('<')) {
            (false, n)
        } else {
            return Err("magnitude needs a > or < threshold, e.g. \\M{>6}".to_string());
        };
        let num = num.trim();
        if num.starts_with('+') || num.starts_with('-') {
            let (body, scope) = match num.split_once(':') {
                Some((b, name)) => {
                    let name = name.trim();
                    if name.is_empty()
                        || !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
                    {
                        return Err(format!(
                            "bad context {name:?} after the relative threshold (use window, phase, regime, echo, enclosing, or a register name)"
                        ));
                    }
                    (b.trim(), Scope::parse(name))
                }
                None => (num, Scope::Window),
            };
            let (value, sigmas) = match body.strip_suffix('s') {
                Some(v) => (v.trim(), true),
                None => (body, false),
            };
            let v: f32 = value
                .parse()
                .map_err(|e| format!("bad relative magnitude threshold {body:?}: {e}"))?;
            let centi = (v * 100.0).round() as i32;
            let delta = if sigmas { Delta::Sigmas(centi) } else { Delta::Orders(centi) };
            return Ok(if ge { MagPred::Above(scope, delta) } else { MagPred::Below(scope, delta) });
        }
        let v: f32 = num.parse().map_err(|e| format!("bad magnitude threshold {num:?}: {e}"))?;
        let centi = (v * 100.0).round() as i32;
        Ok(if ge { MagPred::Ge(centi) } else { MagPred::Le(centi) })
    }

    /// `(A B C)~k` from the group already parsed: the elements as the atoms
    /// the walk aligns, each with the register it binds.
    ///
    /// The walk pairs one atom with one token, so every element has to be a
    /// single-token matcher; anything else is refused here, naming what the
    /// group takes, rather than read as something it is not. The count is
    /// less than the number of atoms, because at the atom count every atom can
    /// be deleted and the group would match a run that resembles none of them.
    fn edit_group(&mut self, inner: Pattern, k: u8) -> Result<Pattern, ParseError> {
        let parts: Vec<&Pattern> = match &inner {
            Pattern::Concat(v) => v.iter().collect(),
            one => vec![one],
        };
        let mut atoms = Vec::with_capacity(parts.len());
        for part in parts {
            let (bind, body) = match part {
                Pattern::Bind(name, scoped, p) => (Some((name.clone(), *scoped)), p.as_ref()),
                p => (None, p),
            };
            let Pattern::Atom(atom) = body else {
                return Err(self.err(
                    "a ~k group holds single-token atoms - a kind, a literal, a class, a \
                     predicate or a byte pattern, each optionally bound - and this one holds \
                     something that matches a run",
                ));
            };
            atoms.push(crate::ast::EditAtom { atom: atom.clone(), bind });
        }
        if usize::from(k) >= atoms.len() {
            return Err(self.err(&format!(
                "~{k} over {} atom(s) matches a run that resembles none of them; the count \
                 must be less than the number of atoms",
                atoms.len()
            )));
        }
        Ok(Pattern::Within(atoms, k))
    }

    /// Parse `(?orbit:G P)`, the `(` consumed and the cursor on `?`.
    ///
    /// Every literal inside `P` is rewritten to compare under `G`, so the
    /// scope resolves entirely at parse time and neither engine carries a
    /// modifier stack. `(?orbit:case ...)` is what regex spells `(?i)`; the
    /// other rungs have no regex counterpart, which is the point of naming the
    /// axis rather than adding a flag per equivalence.
    fn parse_modifier_group(&mut self) -> Result<Pattern, ParseError> {
        self.bump();
        // `(?>P)` is the atomic group. It takes no name, so it is read before
        // the named modifiers.
        if self.peek() == Some(b'>') {
            self.bump();
            let inner = self.parse_alt()?;
            self.expect(b')')?;
            return Ok(Pattern::Atomic(inner.boxed()));
        }
        let name = self.parse_name()?;
        if name != "orbit" {
            return Err(self.err(&format!(
                "unknown group modifier (?{name}...); the modifier axis is `orbit`"
            )));
        }
        self.expect(b':')?;
        let group_name = self.parse_name()?;
        let Some(mut group) = crate::orbit::OrbitGroup::parse(&group_name) else {
            return Err(self.err(&format!("unknown orbit group {group_name:?}")));
        };
        // `subnet/24`: the prefix length the rung folds at, written as the
        // `=subnet/24 x` reference writes it.
        if self.peek() == Some(b'/') {
            self.bump();
            let bits = self.parse_number()?;
            if bits > 128 {
                return Err(self.err("a prefix length is at most 128"));
            }
            group = group.with_prefix(bits as u8).ok_or_else(|| {
                self.err("only the subnet rung takes a prefix length after a slash")
            })?;
        }
        // One scope at a time. Nested, the outer one would rewrite the inner
        // one's literals on its way past and the inner rung would reach
        // nothing but the back-references, which is a pattern meaning neither
        // of the two things it is written to mean.
        if self.orbit.is_some() {
            return Err(self.err("an orbit scope cannot be nested inside another"));
        }
        self.orbit = Some(group);
        let inner = self.parse_alt();
        self.orbit = None;
        let mut inner = inner?;
        self.expect(b')')?;
        set_orbit(&mut inner, group);
        Ok(inner)
    }

    /// Parse the comparison at the cursor, or `None` where no operator is
    /// there.
    fn parse_cmp(&mut self) -> Option<crate::ast::Cmp> {
        use crate::ast::Cmp;
        let two = |p: &mut Self, c| {
            p.bump();
            p.bump();
            Some(c)
        };
        match (self.peek(), self.s.get(self.i + 1)) {
            (Some(b'>'), Some(b'=')) => two(self, Cmp::Ge),
            (Some(b'<'), Some(b'=')) => two(self, Cmp::Le),
            (Some(b'!'), Some(b'=')) => two(self, Cmp::Ne),
            (Some(b'>'), _) => {
                self.bump();
                Some(Cmp::Gt)
            }
            (Some(b'<'), _) => {
                self.bump();
                Some(Cmp::Lt)
            }
            (Some(b'='), _) => {
                self.bump();
                Some(Cmp::Eq)
            }
            _ => None,
        }
    }

    /// Parse the body of `@echo`: a bare anchor is content that recurs at
    /// all, an operator and a number compare the occurrence count, and
    /// `:nth` and `:period` read the other two fields.
    /// `@novel` / `@echoed`, and with `:@name` after them the same reading
    /// against a second input: the token's content nowhere in it, or
    /// somewhere in it. A bare name ends at whitespace or a closing bracket;
    /// a quoted one may hold either.
    fn echo_anchor(&mut self, recurs: bool) -> Result<Pattern, ParseError> {
        if self.peek() != Some(b':') {
            return Ok(Pattern::Anchor(if recurs { AnchorKind::Echoed } else { AnchorKind::Novel }));
        }
        let at = self.i;
        self.bump();
        if self.peek() != Some(b'@') {
            return Err(ParseError {
                pos: at,
                msg: "a second input is named after the colon: @echoed:@other.log".to_string(),
            });
        }
        self.bump();
        let name = if self.peek() == Some(b'"') {
            self.read_quoted()?
        } else {
            let start = self.i;
            while matches!(self.peek(), Some(c) if !c.is_ascii_whitespace() && c != b')') {
                self.bump();
            }
            String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
        };
        if name.is_empty() {
            return Err(self.err("a second input needs a name: @echoed:@other.log"));
        }
        let other = self.other_input(&name).map_err(|m| ParseError { pos: at, msg: m })?;
        Ok(Pattern::Anchor(AnchorKind::Joined {
            other,
            recurs,
            group: crate::orbit::OrbitGroup::Identity,
        }))
    }

    /// The second input `name` denotes: bytes the caller supplied under that
    /// name, else the file at that path, relative to the pattern file's
    /// directory when there is one; lexed once per parse however many
    /// anchors name it.
    fn other_input(
        &mut self,
        name: &str,
    ) -> Result<std::sync::Arc<crate::ast::OtherInput>, String> {
        use std::sync::Arc;
        if let Some(found) = self.others.get(name) {
            return Ok(Arc::clone(found));
        }
        let bytes: Vec<u8> = match self.inputs.iter().find(|(n, _)| *n == name) {
            Some((_, bytes)) => bytes.to_vec(),
            None => {
                let path = match self.shapes.base_dir() {
                    Some(dir) if std::path::Path::new(name).is_relative() => dir.join(name),
                    _ => std::path::PathBuf::from(name),
                };
                match std::fs::read(&path) {
                    Ok(b) => b,
                    Err(e) => {
                        return Err(format!(
                            "cannot read the second input {}: {e}",
                            path.display()
                        ));
                    }
                }
            }
        };
        let tokens = crate::lexer::lex(&bytes);
        let other = Arc::new(crate::ast::OtherInput { name: name.to_string(), bytes, tokens });
        self.others.insert(name.to_string(), Arc::clone(&other));
        Ok(other)
    }

    fn parse_echo_anchor(&mut self) -> Result<Pattern, ParseError> {
        use crate::ast::{AnchorKind, Cmp, EchoPred};
        use crate::orbit::OrbitGroup;
        let pred = if self.peek() == Some(b':') {
            self.bump();
            let name = self.parse_name()?;
            match name.as_str() {
                "nth" => {
                    let Some(op) = self.parse_cmp() else {
                        return Err(self.err("@echo:nth needs an operator, e.g. @echo:nth=3"));
                    };
                    let negative = self.peek() == Some(b'-');
                    if negative {
                        self.bump();
                    }
                    let k = self.parse_number()?;
                    if k == 0 {
                        return Err(self.err(
                            "@echo:nth counts from one, or from the last backward as -1; there is no zeroth occurrence",
                        ));
                    }
                    let k = i32::try_from(k)
                        .map_err(|e| self.err(&format!("an occurrence index fits four bytes: {e}")))?;
                    EchoPred::Nth(op, if negative { -k } else { k })
                }
                "period" => match self.parse_cmp() {
                    Some(op) => EchoPred::PeriodAt(op, self.parse_number()? as u32),
                    None => EchoPred::Period,
                },
                other => {
                    return Err(self.err(&format!(
                        "unknown echo reading {other:?} (use @echo, @echo>k, @echo:nth=k, @echo:period)"
                    )));
                }
            }
        } else {
            match self.parse_cmp() {
                Some(op) => EchoPred::Count(op, self.parse_number()? as u32),
                // A bare `@echo` is content that recurs at all, which is what
                // `@echoed` says.
                None => EchoPred::Count(Cmp::Ge, 2),
            }
        };
        Ok(Pattern::Anchor(AnchorKind::Echo(pred, OrbitGroup::Identity)))
    }

    /// Parse the body of an assertion, the leading `~` already consumed and
    /// `neg` saying whether a `!` preceded it.
    ///
    /// `~"lit"` stays the content guard: a presence question over the forward
    /// window that a prefilter can answer without a positional scan. `~(P)`,
    /// `~<(P)`, `~>k(P)` and `~#(P)` are the sub-pattern forms, positional and
    /// zero-width - `<` selecting the backward direction, `>k` a forward window
    /// of `k` significant tokens, `#` the balanced group opening at the
    /// position, whose extent the input decides rather than the pattern.
    fn parse_assertion(&mut self, neg: bool) -> Result<Pattern, ParseError> {
        let look = if self.peek() == Some(b'<') {
            self.bump();
            Look::Behind
        } else if self.peek() == Some(b'>') {
            self.bump();
            let window = self.parse_number()?;
            if window == 0 {
                return Err(self.err("a proximity window of zero tokens can never hold a match"));
            }
            let (at_least, at_most) = self.parse_count_range()?;
            if at_least > window {
                return Err(
                    self.err("a window cannot hold more occurrences than it holds tokens")
                );
            }
            Look::Within { window, at_least, at_most }
        } else if self.peek() == Some(b'#') {
            self.bump();
            let (at_least, at_most) = self.parse_count_range()?;
            Look::InGroup { at_least, at_most }
        } else {
            Look::Ahead
        };
        if self.peek() == Some(b'"') {
            match look {
                Look::Ahead => {
                    let lit = self.read_quoted()?;
                    return Ok(Pattern::Guard(lit, neg));
                }
                Look::Behind => {
                    return Err(self.err(
                        "a literal guard reads the forward window; use ~<(\"lit\") for a backward assertion",
                    ));
                }
                // A guard asks only whether the literal is anywhere ahead,
                // which is the question the window and the group were written
                // to narrow, so taking one here would answer a wider one under
                // the narrower spelling.
                Look::Within { .. } | Look::InGroup { .. } => {
                    return Err(self.err(
                        "a counted assertion takes a parenthesised sub-pattern; write ~>3{2}(\"lit\"), not ~>3{2}\"lit\"",
                    ));
                }
            }
        }
        self.expect(b'(')?;
        let inner = self.parse_alt()?;
        self.expect(b')')?;
        // Looking behind tries each start in a window sized by the
        // sub-pattern's longest match, so an unbounded one has no window and
        // would scan the whole prefix at every position.
        if look == Look::Behind && crate::nfa::bounded_max_len(&inner).is_none() {
            return Err(self.err(
                "a backward assertion needs a bounded sub-pattern; `*`, `+` and `{m,}` have no fixed length",
            ));
        }
        Ok(Pattern::Assert(inner.boxed(), neg, look))
    }

    /// Read the `{m}`, `{m,}` or `{m,n}` that follows a counting assertion's
    /// selector: how many occurrences the region must hold. `{m}` is an exact
    /// count, `{m,}` a floor with no ceiling, `{m,n}` both ends.
    ///
    /// With no brace the assertion asks only whether there is any, which is one
    /// occurrence and no ceiling.
    fn parse_count_range(&mut self) -> Result<(usize, Option<usize>), ParseError> {
        if self.peek() != Some(b'{') {
            return Ok((1, None));
        }
        self.bump();
        let lo = self.parse_number()?;
        let hi = if self.peek() == Some(b',') {
            self.bump();
            if self.peek() == Some(b'}') { None } else { Some(self.parse_number()?) }
        } else {
            Some(lo)
        };
        self.expect(b'}')?;
        if let Some(hi) = hi
            && hi < lo
        {
            return Err(self.err("a count whose top is below its bottom can never be met"));
        }
        Ok((lo, hi))
    }

    /// Parse a token class, the opening `[` already consumed.
    ///
    /// `[a b c]` unions, `[^a b]` complements, `[a && b]` intersects and
    /// `[a -- b]` subtracts. Members are ordinary single-token atoms, so a
    /// class composes with every atom the language already has. A literal
    /// hyphen or ampersand must be quoted (`["-"]`), since bare ones read as
    /// the operators.
    fn parse_class(&mut self) -> Result<Pattern, ParseError> {
        let mut cls = crate::ast::TokenClass {
            any: Vec::new(),
            all: Vec::new(),
            none: Vec::new(),
            negated: false,
        };
        self.skip_spaces();
        if self.peek() == Some(b'^') {
            self.bump();
            cls.negated = true;
        }
        // Which list the members being read belong to; `&&` and `--` move it.
        let mut target = 0u8;
        loop {
            self.skip_spaces();
            match self.peek() {
                None => return Err(self.err("unterminated token class")),
                Some(b']') => {
                    self.bump();
                    break;
                }
                Some(b'&') if self.s.get(self.i + 1) == Some(&b'&') => {
                    self.bump();
                    self.bump();
                    target = 1;
                    continue;
                }
                Some(b'-') if self.s.get(self.i + 1) == Some(&b'-') => {
                    self.bump();
                    self.bump();
                    target = 2;
                    continue;
                }
                _ => {}
            }
            let at = self.i;
            let member = match self.parse_atom()? {
                Pattern::Atom(a) => a,
                other => {
                    return Err(ParseError {
                        pos: at,
                        msg: format!(
                            "a token class holds single-token atoms; {other:?} is not one"
                        ),
                    });
                }
            };
            match target {
                1 => cls.all.push(member),
                2 => cls.none.push(member),
                _ => cls.any.push(member),
            }
        }
        if cls.any.is_empty() {
            return Err(self.err("a token class needs at least one member before && or --"));
        }
        Ok(Pattern::Atom(Atom::Class(Box::new(cls))))
    }

    fn parse_balanced(&mut self) -> Result<Pattern, ParseError> {
        match self.peek() {
            Some(b'(') => {
                self.bump();
                let inner = self.parse_alt()?;
                self.expect(b')')?;
                Ok(Pattern::Balanced(Some(BracketKind::Paren), inner.boxed()))
            }
            Some(b'[') => {
                self.bump();
                let inner = self.parse_alt()?;
                self.expect(b']')?;
                Ok(Pattern::Balanced(Some(BracketKind::Square), inner.boxed()))
            }
            Some(b'{') => {
                self.bump();
                let inner = self.parse_alt()?;
                self.expect(b'}')?;
                Ok(Pattern::Balanced(Some(BracketKind::Brace), inner.boxed()))
            }
            // Bare \B: any bracket kind, any interior.
            _ => Ok(Pattern::Balanced(
                None,
                Pattern::Star(Pattern::Atom(Atom::Any).boxed(), Greed::Greedy).boxed(),
            )),
        }
    }

    /// Expand a cross-language structural lens (`@call`, `@block`, ...)
    /// into a token pattern. Each lens is a convergent shape that holds
    /// across most languages because lexical structure (identifiers,
    /// balanced delimiters, string and number literals) is near-universal
    /// even where grammar and semantics differ.
    fn parse_lens(&mut self) -> Result<Pattern, ParseError> {
        let any_interior = || Pattern::Star(Pattern::Atom(Atom::Any).boxed(), Greed::Greedy).boxed();
        let name = self.parse_name()?;
        match name.as_str() {
            // identifier followed by a balanced paren group: a call.
            "call" => Ok(Pattern::Concat(vec![
                Pattern::Atom(Atom::Kind(TokenKind::Word)),
                Pattern::Balanced(Some(BracketKind::Paren), any_interior()),
            ])),
            // a balanced brace group: a block.
            "block" => Ok(Pattern::Balanced(Some(BracketKind::Brace), any_interior())),
            // a balanced bracket group of any kind.
            "nesting" => Ok(Pattern::Balanced(None, any_interior())),
            // a quoted string token.
            "string" => Ok(Pattern::Atom(Atom::Kind(TokenKind::Quoted))),
            // a number token.
            "number" => Ok(Pattern::Atom(Atom::Kind(TokenKind::Number))),
            // a word or identifier token.
            "ident" => Ok(Pattern::Atom(Atom::Kind(TokenKind::Word))),
            // an identifier immediately followed by `=`: an assignment
            // left-hand side (a heuristic that holds across C-family,
            // scripting, and config languages).
            "assignment" => Ok(Pattern::Concat(vec![
                Pattern::Atom(Atom::Kind(TokenKind::Word)),
                Pattern::Atom(Atom::literal("=")),
            ])),
            // an identifier bound to a value by `:` or `=`: the assignment
            // generalized to the two key/value separators that span config,
            // scripting, and data formats (`name: value`, `key=value`).
            "kv" => Ok(Pattern::Concat(vec![
                Pattern::Atom(Atom::Kind(TokenKind::Word)),
                Pattern::Alt(
                    vec![
                        Pattern::Atom(Atom::literal(":")),
                        Pattern::Atom(Atom::literal("=")),
                    ],
                    AltMode::First,
                ),
            ])),
            // a command-line flag: a leading `-` (or `--`) then a word. The
            // dash is one punctuation token per byte, so a long flag is two.
            "flag" => Ok(Pattern::Concat(vec![
                Pattern::Atom(Atom::literal("-")),
                Pattern::Opt(Pattern::Atom(Atom::literal("-")).boxed(), Greed::Greedy),
                Pattern::Atom(Atom::Kind(TokenKind::Word)),
            ])),
            // a comma-separated list: an element then one or more `, element`
            // groups (so a bare single item is not a list, a comma is).
            "list" => Ok(Pattern::Concat(vec![
                Pattern::Atom(Atom::Any),
                Pattern::Plus(
                    Pattern::Concat(vec![
                        Pattern::Atom(Atom::literal(",")),
                        Pattern::Atom(Atom::Any),
                    ])
                    .boxed(),
                    Greed::Greedy,
                ),
            ])),
            // a numeric range: two numbers joined by `..`, `-`, or `:` (the
            // separators that span slice, interval, and duration notations).
            // A `HH:MM` clock lexes as one timestamp token, so `:` here joins
            // only numbers that did not already fuse into a time.
            "range" => Ok(Pattern::Concat(vec![
                Pattern::Atom(Atom::Kind(TokenKind::Number)),
                Pattern::Alt(
                    vec![
                        Pattern::Concat(vec![
                            Pattern::Atom(Atom::literal(".")),
                            Pattern::Atom(Atom::literal(".")),
                        ]),
                        Pattern::Atom(Atom::literal("-")),
                        Pattern::Atom(Atom::literal(":")),
                    ],
                    AltMode::First,
                ),
                Pattern::Atom(Atom::Kind(TokenKind::Number)),
            ])),
            // a zero-width statistical anchor (not a token-consuming lens): the
            // current position must be at a predictive-segmentation cut.
            "seam" => {
                // `@seam` reads the token sequence; `@seam:byte` reads the
                // bytes under it and `@seam:super` the supertokens over it.
                //
                // The segmentation counts a context per symbol at every order
                // in both directions, so what it costs follows the length of
                // the sequence it reads: over 7.34 MB of source the byte grain
                // spends 1090.222 ms, of which 889.826 is that count, where
                // everything else in the field together is 138.7. The token
                // sequence is shorter than the bytes it was cut from and the
                // count falls with it.
                //
                // The two do not answer one question, and `@seam:byte` is how
                // the byte reading is asked for.
                //
                // A cut written after the grain, `@seam:byte>1.5`, is in
                // standard deviations above the input's mean seam strength.
                let grain = self.anchor_grain(crate::ast::Grain::Token)?;
                let cut = self.anchor_cut("seam")?;
                Ok(Pattern::Anchor(AnchorKind::Seam(grain, cut)))
            }
            // zero-width anchors on the input's pair field: the strain of the
            // unit here against what came before it, or the binding of the cut
            // before it, held against a percentile of the input's own readings
            // or, with `b` after the number, a value in bits.
            "strain" | "bound" => {
                use crate::ast::{GravityReading, Level, Real};
                let reading = if name == "strain" { GravityReading::Strain } else { GravityReading::Bound };
                let grain = self.gravity_grain()?;
                let cmp = self.parse_cmp().ok_or_else(|| {
                    self.err(&format!(
                        "@{name} needs a comparison: a percentile of the input, e.g. @{name}>90, or bits, e.g. @{name}<1.5b"
                    ))
                })?;
                let v = self.read_real()?;
                let level = if self.peek() == Some(b'b') {
                    self.bump();
                    Level::Bits(Real::new(v))
                } else if (0.0..=100.0).contains(&v) {
                    Level::Percentile(Real::new(v))
                } else {
                    return Err(self.err(&format!(
                        "{v} is not a percentile, which runs 0 to 100; write b after a value in bits, e.g. @{name}>{v}b"
                    )));
                };
                Ok(Pattern::Anchor(AnchorKind::Gravity(reading, grain, cmp, level)))
            }
            // a zero-width anchor on the pair field's geometry: the unit here
            // is the example's type or shares its gravity class.
            "kin" => {
                let grain = self.gravity_grain()?;
                self.expect(b'(')?;
                let example = self.read_quoted()?;
                self.expect(b')')?;
                if crate::gravity::example_key(grain, example.as_bytes()).is_none() {
                    return Err(self.err(&format!(
                        "@kin example {example:?} names no type at this grain: one byte at :byte, a token otherwise"
                    )));
                }
                Ok(Pattern::Anchor(AnchorKind::Kin(grain, example)))
            }
            // a zero-width structural-load anchor: the current token must be at
            // least (or more than) k brackets deep. `@nested>2` -> depth > 2;
            // `@nested>=2` -> depth >= 2. This is the counted-nesting predicate.
            "nested" => {
                if self.peek() != Some(b'>') {
                    return Err(self.err("@nested needs a > threshold, e.g. @nested>2"));
                }
                self.bump();
                let ge = self.peek() == Some(b'=');
                if ge {
                    self.bump();
                }
                let k = self.parse_number()? as u16;
                let min = if ge { k } else { k.saturating_add(1) };
                Ok(Pattern::Anchor(AnchorKind::Nested(min)))
            }
            // a zero-width observation-axis anchor: the current token's reading
            // depends on the observer's vantage (a contested / garden-path point).
            //
            // A cut written after the grain, `@ambiguous>0.3`, is the
            // disagreement a contested point must pass, which runs 0 to 1.
            "ambiguous" => {
                let grain = self.anchor_grain(crate::ast::Grain::Byte)?;
                let cut = self.anchor_cut("ambiguous")?;
                if let Some(c) = cut
                    && !(0.0..=1.0).contains(&c.value.get())
                {
                    return Err(self.err(&format!(
                        "@ambiguous{}{} names no disagreement: a disagreement runs 0 to 1",
                        c.cmp.glyph(),
                        c.value.get()
                    )));
                }
                Ok(Pattern::Anchor(AnchorKind::Ambiguous(grain, cut)))
            }
            // zero-width echo-axis (recurrence) anchors: the current token is
            // the first occurrence of its content (@novel), or its content
            // recurs elsewhere in the input (@echoed).
            "novel" => self.echo_anchor(false),
            "echoed" => self.echo_anchor(true),
            // the echo axis read as a number: how often the token's content
            // recurs, which occurrence this is, and whether the recurrence is
            // regularly spaced. The rung it counts at comes from an
            // `(?orbit:G ...)` scope around it.
            "echo" => self.parse_echo_anchor(),
            // a zero-width anchor on the rarity of the template of the token's
            // line, under the mean cut or a written one.
            "shape" => {
                if self.peek() != Some(b':') {
                    return Err(self.err("@shape needs a reading, e.g. @shape:rare"));
                }
                self.bump();
                let name = self.parse_name()?;
                if name != "rare" {
                    return Err(self.err(&format!("unknown shape reading {name:?} (use rare)")));
                }
                let cut = if self.peek() == Some(b'<') {
                    self.bump();
                    let start = self.i;
                    while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == b'.' || c == b'%')
                    {
                        self.bump();
                    }
                    let written = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                    crate::templates::Rarity::parse(&written).map_err(|m| self.err(&m))?
                } else {
                    crate::templates::Rarity::Mean
                };
                Ok(Pattern::Anchor(AnchorKind::Rare(cut)))
            }
            // a zero-width anchor on how this timestamp compares with the
            // timestamp token before it in the stream.
            "order" => {
                if self.peek() != Some(b':') {
                    return Err(self.err("@order needs a direction, e.g. @order:desc"));
                }
                self.bump();
                let name = self.parse_name()?;
                match crate::ast::TimeOrder::parse(&name) {
                    Some(o) => Ok(Pattern::Anchor(AnchorKind::Order(o))),
                    None => {
                        Err(self.err(&format!("unknown order {name:?} (use asc, desc)")))
                    }
                }
            }
            // a zero-width phase anchor: the current token is at column `k` of
            // the dominant token-kind period, counted in significant tokens.
            "phase" => {
                if self.peek() != Some(b':') {
                    return Err(self.err("@phase needs a column, e.g. @phase:2"));
                }
                self.bump();
                let max = crate::context::PHASE_MAX_PERIOD;
                let column = self.parse_number()?;
                let k = match u16::try_from(column) {
                    Ok(k) if k < max => k,
                    Ok(_) => {
                        return Err(self.err(&format!("@phase:{column} names no column: a period is at most {max} tokens")));
                    }
                    Err(e) => return Err(self.err(&format!("@phase:{column} names no column ({e})"))),
                };
                // `/p` names the period by its length, `#n` by its rank among
                // the stream's live periods; bare `@phase:k` is the strongest.
                let named = match self.peek() {
                    Some(b'/') => {
                        self.bump();
                        let p = self.parse_number()?;
                        match u16::try_from(p) {
                            Ok(p) if (2..=max).contains(&p) && k < p => Some(crate::ast::PeriodRef::Length(p)),
                            Ok(_) => {
                                return Err(self.err(&format!(
                                    "@phase:{k}/{p} names no column: a period runs 2 to {max} tokens and its columns 0 to one less"
                                )));
                            }
                            Err(e) => return Err(self.err(&format!("@phase:{k}/{p} names no period ({e})"))),
                        }
                    }
                    Some(b'#') => {
                        self.bump();
                        let n = self.parse_number()?;
                        match u16::try_from(n) {
                            Ok(n) if n >= 1 => Some(crate::ast::PeriodRef::Rank(n)),
                            Ok(_) => return Err(self.err(&format!("@phase:{k}#{n} names no period: the strongest is #1"))),
                            Err(e) => return Err(self.err(&format!("@phase:{k}#{n} names no period ({e})"))),
                        }
                    }
                    Some(_) | None => None,
                };
                Ok(Pattern::Anchor(match named {
                    Some(period) => AnchorKind::PhaseIn(k, period),
                    None => AnchorKind::Phase(k),
                }))
            }
            // zero-width supertoken anchors: the current token begins a
            // construct (`@super`), or the construct containing it has a role
            // (`@super:call`). The second is the containment test - what
            // encloses this token, rather than what it is.
            "super" => {
                if self.peek() != Some(b':') {
                    return Ok(Pattern::Anchor(AnchorKind::SuperStart));
                }
                self.bump();
                let name = self.parse_name()?;
                match crate::supertoken::Role::parse(&name) {
                    Some(role) => Ok(Pattern::Anchor(AnchorKind::SuperRole(role))),
                    None => Err(self.err(&format!(
                        "unknown supertoken role {name:?} (use call, assign, kv, list, numeric, plain)"
                    ))),
                }
            }
            other => Err(self.err(&format!(
                "unknown lens @{other} (use call, block, nesting, string, number, ident, assignment, kv, flag, list, range, seam, nested, ambiguous, novel, echoed, super, phase)"
            ))),
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), ParseError> {
        self.skip_spaces();
        if self.peek() == Some(b) {
            self.bump();
            Ok(())
        } else {
            Err(self.err(&format!("expected '{}'", b as char)))
        }
    }

    /// A register name: a letter or underscore, then letters, digits and
    /// underscores, and a `.` joining another such part, as a nested
    /// register is named (`pair.k`); a `.` not followed by a name part is
    /// the any-token atom after the name.
    fn parse_name(&mut self) -> Result<String, ParseError> {
        let start = self.i;
        if !matches!(self.peek(), Some(c) if c == b'_' || c.is_ascii_alphabetic()) {
            return Err(self.err("expected a register name"));
        }
        loop {
            while matches!(self.peek(), Some(c) if c == b'_' || c.is_ascii_alphanumeric()) {
                self.bump();
            }
            let joins = self.peek() == Some(b'.')
                && matches!(self.s.get(self.i + 1), Some(&c) if c == b'_' || c.is_ascii_alphabetic());
            if !joins {
                break;
            }
            self.bump();
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn parse_number(&mut self) -> Result<usize, ParseError> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.bump();
        }
        if self.i == start {
            return Err(self.err("expected a number"));
        }
        String::from_utf8_lossy(&self.s[start..self.i])
            .parse::<usize>()
            .map_err(|_| self.err("invalid number"))
    }

    /// A decimal number, optionally negative: `90`, `2.5`, `-3.25`.
    fn read_real(&mut self) -> Result<f64, ParseError> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.bump();
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.bump();
        }
        if self.peek() == Some(b'.') {
            self.bump();
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.bump();
            }
        }
        let text = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
        text.parse::<f64>().map_err(|e| self.err(&format!("expected a number, read {text:?} ({e})")))
    }

    /// The grain after `@strain`, `@bound` or `@kin`: `:byte`, `:token` or
    /// `:super`, and the token grain where none is written, as `@seam` reads.
    fn gravity_grain(&mut self) -> Result<crate::ast::Grain, ParseError> {
        self.anchor_grain(crate::ast::Grain::Token)
    }

    /// The grain after an anchor's name, `:byte`, `:token` or `:super`, or
    /// `default` where none is written.
    fn anchor_grain(&mut self, default: crate::ast::Grain) -> Result<crate::ast::Grain, ParseError> {
        if self.peek() != Some(b':') {
            return Ok(default);
        }
        self.bump();
        let name = self.parse_name()?;
        crate::ast::Grain::parse(&name)
            .ok_or_else(|| self.err(&format!("unknown grain {name:?} (use byte, token, super)")))
    }

    /// The `>k` or `>=k` written after `@seam` or `@ambiguous` and its grain,
    /// or `None` where no `>` is at the cursor.
    fn anchor_cut(&mut self, anchor: &str) -> Result<Option<crate::ast::AnchorCut>, ParseError> {
        use crate::ast::{AnchorCut, Cmp, Real};
        if self.peek() != Some(b'>') {
            return Ok(None);
        }
        let cmp = if self.s.get(self.i + 1) == Some(&b'=') { Cmp::Ge } else { Cmp::Gt };
        self.bump();
        if cmp == Cmp::Ge {
            self.bump();
        }
        if !matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == b'-' || c == b'.') {
            return Err(self.err(&format!("@{anchor}{} needs a number, e.g. @{anchor}{}0.5", cmp.glyph(), cmp.glyph())));
        }
        let v = self.read_real()?;
        Ok(Some(AnchorCut { cmp, value: Real::new(v) }))
    }

    fn read_quoted(&mut self) -> Result<String, ParseError> {
        if self.peek() != Some(b'"') {
            return Err(self.err("expected a quoted literal"));
        }
        self.bump();
        let mut out = String::new();
        loop {
            match self.bump() {
                Some(b'\\') => {
                    if let Some(c) = self.bump() {
                        out.push(c as char);
                    } else {
                        return Err(self.err("dangling backslash in literal"));
                    }
                }
                Some(b'"') => break,
                Some(c) => out.push(c as char),
                None => return Err(self.err("unterminated quoted literal")),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_number_atom() {
        assert_eq!(parse("\\N").unwrap(), Pattern::Atom(Atom::Kind(TokenKind::Number)));
    }

    #[test]
    fn parses_named_atom() {
        assert_eq!(parse("\\{jwt}").unwrap(), Pattern::Atom(Atom::Kind(TokenKind::Jwt)));
        assert_eq!(parse("\\{creditcard}").unwrap(), Pattern::Atom(Atom::Kind(TokenKind::CreditCard)));
        // A letter atom is also reachable by its full name.
        assert_eq!(parse("\\{ip}").unwrap(), Pattern::Atom(Atom::Kind(TokenKind::Ip)));
        assert!(parse("\\{bogus}").is_err());
        assert!(parse("\\{jwt").is_err());
    }

    #[test]
    fn parses_lowercase_byte_classes() {
        for (src, bc) in [
            ("\\h", ByteClass::Hex),
            ("\\a", ByteClass::Alpha),
            ("\\u", ByteClass::Upper),
            ("\\l", ByteClass::Lower),
        ] {
            assert_eq!(parse(src).unwrap(), Pattern::Atom(Atom::Byte(bc)));
        }
    }

    #[test]
    fn parses_matched_tag_pattern() {
        // <\W:t>.*</=t>
        let p = parse("<\\W:t>.*</=t>").unwrap();
        let Pattern::Concat(items) = p else {
            panic!("expected a concatenation, got {p:?}");
        };
        // < \W:t > .* < / =t >
        assert_eq!(items.len(), 8);
        assert_eq!(items[0], Pattern::Atom(Atom::literal("<")));
        assert_eq!(
            items[1],
            Pattern::Bind("t".to_string(), false, Pattern::Atom(Atom::Kind(TokenKind::Word)).boxed())
        );
        assert_eq!(items[3], Pattern::Star(Pattern::Atom(Atom::Any).boxed(), Greed::Greedy));
        assert_eq!(
            items[6],
            Pattern::Atom(Atom::RegisterEq("t".to_string(), crate::orbit::OrbitGroup::Identity))
        );
    }

    #[test]
    fn parses_balanced_with_interior() {
        // \W\B(.*)
        let p = parse("\\W\\B(.*)").unwrap();
        let Pattern::Concat(items) = p else {
            panic!("expected a concatenation, got {p:?}");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[1],
            Pattern::Balanced(
                Some(BracketKind::Paren),
                Pattern::Star(Pattern::Atom(Atom::Any).boxed(), Greed::Greedy).boxed()
            )
        );
    }

    #[test]
    fn reports_position_on_dangling_backslash() {
        let e = parse("\\").unwrap_err();
        assert_eq!(e.pos, 1);
    }

    #[test]
    fn a_brace_body_on_a_kind_atom_is_a_repeat_a_magnitude_or_a_typed_predicate() {
        // Digits and a comma are a repeat count.
        assert!(matches!(parse("\\W{2,3}").unwrap(), Pattern::Repeat(_, 2, Some(3), _)));
        assert!(matches!(parse("\\N{2}").unwrap(), Pattern::Repeat(_, 2, Some(2), _)));
        // A plain comparison compares the value.
        assert!(matches!(parse("\\N{>500}").unwrap(), Pattern::Atom(Atom::KindPred(TokenKind::Number, _))));
        assert!(matches!(parse("\\N{500..599}").unwrap(), Pattern::Atom(Atom::KindPred(TokenKind::Number, _))));
        assert!(matches!(parse("\\I{in:10.0.0.0/8}").unwrap(), Pattern::Atom(Atom::KindPred(TokenKind::Ip, _))));
        assert!(matches!(parse("\\{creditcard}{issuer:visa}").unwrap(), Pattern::Atom(Atom::KindPred(TokenKind::CreditCard, _))));
        // `mag` and a signed threshold are the magnitude axis.
        assert!(matches!(parse("\\N{mag>3}").unwrap(), Pattern::Atom(Atom::KindMag(TokenKind::Number, _))));
        assert!(matches!(parse("\\N{>+1}").unwrap(), Pattern::Atom(Atom::KindMag(TokenKind::Number, _))));
        assert!(matches!(parse("\\N{<-1:k}").unwrap(), Pattern::Atom(Atom::KindMag(TokenKind::Number, _))));
        assert!(matches!(parse("\\N{>+2s}").unwrap(), Pattern::Atom(Atom::KindMag(TokenKind::Number, _))));
        // A field the kind lacks, a unitless duration and a bare word
        // comparison are errors that name the problem.
        let e = parse("\\I{host:x}").unwrap_err();
        assert!(e.msg.contains("no field `host`"), "{}", e.msg);
        let e = parse("\\R{>500}").unwrap_err();
        assert!(e.msg.contains("unit"), "{}", e.msg);
        let e = parse("\\W{>5}").unwrap_err();
        assert!(e.msg.contains("name a field"), "{}", e.msg);
        assert!(parse("\\N{>500").is_err());
        // A predicate takes a quantifier and a binding like any atom.
        assert!(matches!(parse("\\N{>500}+").unwrap(), Pattern::Plus(..)));
        assert!(matches!(parse("\\N{>500}:big").unwrap(), Pattern::Bind(..)));
    }

    #[test]
    fn a_back_reference_reads_under_a_group_a_relation_or_neither() {
        use crate::orbit::OrbitGroup;
        use crate::typed::Relation;
        let atom = |src: &str| match parse(src).unwrap_or_else(|e| panic!("{src}: {e:?}")) {
            Pattern::Atom(a) => a,
            other => panic!("{src}: {other:?}"),
        };
        assert_eq!(atom("=ip a"), Atom::RegisterEq("a".to_string(), OrbitGroup::Ip));
        assert_eq!(atom("=fold w"), Atom::RegisterEq("w".to_string(), OrbitGroup::Fold));
        assert_eq!(atom("=subnet a"), Atom::RegisterRelated("a".to_string(), Relation::Subnet(None)));
        assert_eq!(atom("=subnet/16 a"), Atom::RegisterRelated("a".to_string(), Relation::Subnet(Some(16))));
        assert_eq!(atom("=domain e"), Atom::RegisterRelated("e".to_string(), Relation::Domain));
        assert_eq!(atom("=day t"), Atom::RegisterRelated("t".to_string(), Relation::Day));
        // A keyword with no register after it is a register of that name.
        assert_eq!(atom("=domain"), Atom::RegisterEq("domain".to_string(), OrbitGroup::Identity));
        assert_eq!(atom("=ip"), Atom::RegisterEq("ip".to_string(), OrbitGroup::Identity));
        assert!(parse("=domain/8 e").is_err());
        assert!(parse("=subnet/200 a").is_err());
        // The typed rungs scope over a group like the older ones.
        assert_eq!(atom("(?orbit:numeric \"1000\")"), Atom::Literal("1000".to_string(), OrbitGroup::Numeric));
    }

    #[test]
    fn an_orbit_scope_names_a_typed_rung_and_reaches_a_plain_back_reference() {
        use crate::orbit::OrbitGroup;
        use crate::typed::Relation;
        let atoms = |src: &str| match parse(src).unwrap_or_else(|e| panic!("{src}: {e:?}")) {
            Pattern::Concat(v) => v,
            other => panic!("{src}: {other:?}"),
        };
        let subnet = OrbitGroup::Typed(Relation::Subnet(Some(24)));
        assert_eq!(
            atoms("(?orbit:subnet/24 \"10.0.0.1\" =a)"),
            vec![
                Pattern::Atom(Atom::Literal("10.0.0.1".to_string(), subnet)),
                Pattern::Atom(Atom::RegisterEq("a".to_string(), subnet)),
            ]
        );
        // A bare rung takes the relation's own default width, and a
        // back-reference whose spelling names a comparison keeps it.
        assert_eq!(
            atoms("(?orbit:domain =a =case b =subnet c)"),
            vec![
                Pattern::Atom(Atom::RegisterEq(
                    "a".to_string(),
                    OrbitGroup::Typed(Relation::Domain)
                )),
                Pattern::Atom(Atom::RegisterEq("b".to_string(), OrbitGroup::Case)),
                Pattern::Atom(Atom::RegisterRelated("c".to_string(), Relation::Subnet(None))),
            ]
        );
        // Outside a scope a plain reference is exact.
        assert_eq!(
            parse("=a").unwrap_or_else(|e| panic!("{e:?}")),
            Pattern::Atom(Atom::RegisterEq("a".to_string(), OrbitGroup::Identity))
        );
        let e = parse("(?orbit:case (?orbit:numeric \"1\"))").unwrap_err();
        assert!(e.msg.contains("cannot be nested inside another"), "{}", e.msg);
        let e = parse("(?orbit:domain/24 \\E)").unwrap_err();
        assert!(e.msg.contains("only the subnet rung"), "{}", e.msg);
        let e = parse("(?orbit:nosuch \\W)").unwrap_err();
        assert!(e.msg.contains("unknown orbit group"), "{}", e.msg);
    }

    #[test]
    fn an_edit_group_holds_single_token_atoms_and_a_count_under_their_number() {
        use crate::ast::EditAtom;
        use crate::token::TokenKind;
        let plain = |a: Atom| EditAtom { atom: a, bind: None };
        assert_eq!(
            parse("(\\W \\N)~1").unwrap_or_else(|e| panic!("{e:?}")),
            Pattern::Within(
                vec![plain(Atom::Kind(TokenKind::Word)), plain(Atom::Kind(TokenKind::Number))],
                1
            )
        );
        // A binding rides on its atom rather than wrapping it, because the
        // walk aligns atoms and binds what each one took.
        assert_eq!(
            parse("(\\W:who \\N)~1").unwrap_or_else(|e| panic!("{e:?}")),
            Pattern::Within(
                vec![
                    EditAtom {
                        atom: Atom::Kind(TokenKind::Word),
                        bind: Some(("who".to_string(), false)),
                    },
                    plain(Atom::Kind(TokenKind::Number)),
                ],
                1
            )
        );
        assert_eq!(parse("(\\W \\N)~1").unwrap_or_else(|e| panic!("{e:?}")).capture_names(), Vec::<String>::new());
        assert_eq!(
            parse("(\\W:who \\N)~1").unwrap_or_else(|e| panic!("{e:?}")).capture_names(),
            vec!["who".to_string()]
        );
        // A `~` before anything but a digit is the assertion that follows the
        // group, as it is after a literal.
        assert!(matches!(
            parse("(\\W \\N) ~\"end\"").unwrap_or_else(|e| panic!("{e:?}")),
            Pattern::Concat(_)
        ));
        for (src, said) in [
            ("(\\W \\N*)~1", "holds single-token atoms"),
            ("(\\W (\\N \\W))~1", "holds single-token atoms"),
            ("(\\W \\N \\W)~3", "the count must be less than the number of atoms"),
            ("(\\W)~1", "the count must be less than the number of atoms"),
        ] {
            let e = parse(src).unwrap_err();
            assert!(e.msg.contains(said), "{src}: {}", e.msg);
        }
    }
}
