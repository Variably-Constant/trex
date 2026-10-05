//! A capture buffer a caller owns and refills, and the two pattern properties
//! that are fixed before any input is seen.
//!
//! A match carries two different kinds of thing. How many registers there are
//! and what they are called belongs to the pattern and is the same for every
//! match of it; where each one landed belongs to one match. [`crate::Match`]
//! carries both, so it allocates a vector and clones a name per register per
//! match. Splitting them lets the names be built once and the positions be
//! written into a buffer that outlives the match.
//!
//! This is the regex crate's `CaptureLocations` with two differences. The
//! buffer names its own slots, so reading one back does not need the pattern
//! again. And each slot carries a token extent beside its byte span: the
//! engine's save slots hold significant-token indices and convert them to bytes
//! on the way out, so the pair is already there and storing it costs nothing. A
//! byte matcher has no such pair to report.

use crate::ast::Pattern;
use crate::engine::Span;

/// Where each register landed in one match, in a buffer sized by the pattern
/// and refilled in place.
///
/// Built once with [`CaptureSlots::of`] and passed to [`captures_read`] or
/// [`captures_read_at`] for each match. Slot order is the order the pattern
/// binds its names, which is the order [`crate::capture_names`] reports; it is
/// not the order [`crate::Match::captures`] uses, which is sorted by name.
///
/// A register the match did not bind reads back as `None`, which is what
/// distinguishes it from one that bound an empty span.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureSlots {
    names: Vec<String>,
    spans: Vec<Option<Span>>,
    extents: Vec<Option<(usize, usize)>>,
    matched: Option<Span>,
    matched_extent: Option<(usize, usize)>,
}

impl CaptureSlots {
    /// A buffer shaped for `pattern`: one slot per name it binds.
    #[must_use]
    pub fn of(pattern: &Pattern) -> Self {
        let names = crate::cursor::capture_names(pattern);
        let n = names.len();
        Self {
            names,
            spans: vec![None; n],
            extents: vec![None; n],
            matched: None,
            matched_extent: None,
        }
    }

    /// How many registers the buffer holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether the pattern binds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The register names, in slot order.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The name of slot `i`.
    #[must_use]
    pub fn name(&self, i: usize) -> Option<&str> {
        self.names.get(i).map(String::as_str)
    }

    /// The slot `name` occupies.
    #[must_use]
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    /// The byte span slot `i` bound, or `None` where it bound nothing.
    #[must_use]
    pub fn get(&self, i: usize) -> Option<Span> {
        *self.spans.get(i)?
    }

    /// The byte span the register called `name` bound.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<Span> {
        self.get(self.index_of(name)?)
    }

    /// The significant tokens slot `i` bound, as a half-open index pair into
    /// the token stream.
    ///
    /// The engine names a match by the token indices it runs between, so this
    /// is what it had before it converted to bytes. A byte matcher reports no
    /// such pair, because a byte offset does not say which token it is in.
    #[must_use]
    pub fn extent(&self, i: usize) -> Option<(usize, usize)> {
        *self.extents.get(i)?
    }

    /// How many significant tokens slot `i` bound.
    #[must_use]
    pub fn tokens(&self, i: usize) -> Option<usize> {
        self.extent(i).map(|(a, b)| b - a)
    }

    /// The span of the whole match the buffer was last filled from.
    #[must_use]
    pub fn matched(&self) -> Option<Span> {
        self.matched
    }

    /// The significant tokens the whole match spanned.
    #[must_use]
    pub fn matched_extent(&self) -> Option<(usize, usize)> {
        self.matched_extent
    }

    /// How many significant tokens the whole match spanned.
    #[must_use]
    pub fn matched_tokens(&self) -> Option<usize> {
        self.matched_extent.map(|(a, b)| b - a)
    }

    /// Forget every position, keeping the names and the allocation.
    pub fn clear(&mut self) {
        self.spans.fill(None);
        self.extents.fill(None);
        self.matched = None;
        self.matched_extent = None;
    }

    /// Fill from a resolved match, for the routes that hand back whole matches.
    fn take_match(&mut self, m: &crate::engine::Match, extent: Option<(usize, usize)>) {
        self.clear();
        self.matched = Some(m.span());
        self.matched_extent = extent;
        for (name, span) in m.names().iter().zip(m.captures()) {
            if let Some(i) = self.index_of(name) {
                self.spans[i] = Some(*span);
            }
        }
    }

    /// Fill from a match whose registers arrived inline, `names` naming them in
    /// the order it holds them.
    ///
    /// The whole reason the inline form exists: this path copies the registers
    /// out and keeps nothing, so building a match to own them first is one
    /// allocation a match for a value discarded on the next call.
    fn take_flat(&mut self, m: &crate::nfa::FlatMatch, names: &[String]) {
        self.clear();
        self.matched = Some(m.span);
        for (i, name) in names.iter().enumerate() {
            if let Some(k) = self.index_of(name) {
                self.spans[k] = Some(m.regs[i]);
            }
        }
    }

    /// Take the walk's next match straight into these slots.
    ///
    /// The fields are destructured rather than reached through `self` so the
    /// name list and the two position lists are borrowed as the separate
    /// places they are: the walk reads the first and writes the other two.
    fn fill_from_walk(
        &mut self,
        w: &mut crate::nfa::SerialWalk<crate::nfa::OwnedStream>,
        input: &[u8],
    ) -> Option<Span> {
        let Self { names, spans, extents, matched, matched_extent } = self;
        let (span, ks, ke) =
            w.next_into(input, names.as_slice(), spans.as_mut_slice(), extents.as_mut_slice())?;
        *matched = Some(span);
        *matched_extent = Some((ks, ke));
        Some(span)
    }
}

/// The first match of `pattern` in `input`, with its registers written into
/// `slots`.
///
/// The counterpart of the regex crate's `captures_read`. Nothing is allocated
/// per match where the single-pass engine takes the pattern, which is the loop
/// this exists for: the walk already carries the save slots and writes them
/// straight through. A pattern the walk declines is answered by the set engine,
/// which builds a match before the slots are filled from it.
pub fn captures_read(pattern: &Pattern, input: &[u8], slots: &mut CaptureSlots) -> Option<Span> {
    captures_read_at(pattern, input, 0, slots)
}

/// [`captures_read`] anchored at byte `at`.
///
/// The counterpart of the regex crate's `captures_read_at`. The bytes before
/// `at` are still read, so a backward assertion and a start anchor see what
/// precedes the position.
///
/// It answers one anchored question and answers it from the start. The regex
/// crate's version RESUMES: it searches forward from `at`, so calling it in a
/// loop over every match costs one pass over the input in total. This one
/// RESTARTS: it takes the whole route ladder again, which is a fresh scan for a
/// byte route and a fresh lex for the walk, so the same loop costs one pass per
/// match. Over fifty thousand matches that is fifty thousand passes.
///
/// Use [`captures_read_iter`] to walk every match with one buffer. This is for
/// a single anchored ask.
pub fn captures_read_at(
    pattern: &Pattern,
    input: &[u8],
    at: usize,
    slots: &mut CaptureSlots,
) -> Option<Span> {
    slots.clear();
    if !pattern.binds() {
        let span = crate::cursor::find_at(pattern, input, at)?;
        slots.matched = Some(span);
        return Some(span);
    }
    if let Some(first) = crate::engine::routed_first_at(pattern, input, at) {
        let first = first?;
        let m = crate::engine::captures(pattern, input, &[first]).into_iter().next()?;
        slots.take_match(&m, None);
        return Some(m.span());
    }
    if let Some(mut w) = crate::nfa::SerialWalk::over(pattern, input) {
        w.seek(at);
        return slots.fill_from_walk(&mut w, input);
    }
    let toks = crate::parallel_lex::lex_parallel(input);
    let start = toks.partition_point(|t| t.start() < at);
    let span = crate::engine::scan_tokens_from(pattern, input, &toks, start).into_iter().next()?;
    let m = crate::engine::captures_over(pattern, input, &toks, &[span]).into_iter().next()?;
    slots.take_match(&m, None);
    Some(m.span())
}

/// Matches taken one at a time into a buffer the caller owns.
///
/// The other half of the reusing loop, and the half that makes it worth having.
/// The walk is held between matches, so taking every match costs one lex and
/// one allocation however many matches there are.
pub struct SlotCursor<'h> {
    input: &'h [u8],
    source: SlotSource,
}

/// Where a slot cursor's matches come from.
enum SlotSource {
    /// A route answered and the pattern binds nothing, so the spans arrived
    /// together and there is nothing to resolve. Nothing is allocated per
    /// match here either: the span is copied into the buffer's whole-match
    /// field and the register slots stay empty.
    Spans { spans: Vec<Span>, at: usize },
    /// A route answered a pattern that does bind, so the registers were
    /// resolved together with the spans and filling the buffer is a copy.
    Resolved { ms: Vec<crate::engine::Match>, at: usize },
    /// A route answered a pattern whose program is a flat run of atoms, so the
    /// registers came back inline. Nothing is allocated per match on this path
    /// at all: the record is copied into the caller's buffer and no match is
    /// ever built.
    Flat { ms: Vec<crate::nfa::FlatMatch>, names: std::sync::Arc<[String]>, at: usize },
    /// The single-pass engine, held between matches and writing its save slots
    /// straight into the caller's buffer.
    ///
    /// Boxed for the reason a held walk is always boxed here: it owns the
    /// tokens, the program and two thread lists, and an enum is as large as its
    /// largest variant.
    Walk(Box<crate::nfa::SerialWalk<crate::nfa::OwnedStream>>),
}

impl SlotCursor<'_> {
    /// The next match, with its registers written into `slots`, or `None` where
    /// there is none left.
    ///
    /// `slots` need not have come from the same call that made this cursor, but
    /// it must be shaped for the same pattern: a register the pattern binds and
    /// the buffer does not name is skipped rather than written to some other
    /// slot.
    pub fn next_into(&mut self, slots: &mut CaptureSlots) -> Option<Span> {
        match &mut self.source {
            SlotSource::Spans { spans, at } => {
                let s = *spans.get(*at)?;
                *at += 1;
                slots.clear();
                slots.matched = Some(s);
                Some(s)
            }
            SlotSource::Resolved { ms, at } => {
                let m = ms.get(*at)?;
                *at += 1;
                slots.take_match(m, None);
                Some(m.span())
            }
            SlotSource::Flat { ms, names, at } => {
                let m = ms.get(*at)?;
                *at += 1;
                slots.take_flat(m, names);
                Some(m.span)
            }
            SlotSource::Walk(w) => {
                slots.clear();
                slots.fill_from_walk(w, self.input)
            }
        }
    }
}

/// A cursor over every match of `pattern`, refilling one buffer, or `None` for
/// a pattern nothing here takes.
///
/// It takes the same ladder [`crate::captures_iter`] takes, for the same
/// reason: a pattern a byte route answers must not be walked, because that
/// route never lexes and the walk always does. Going straight to the walk would
/// make this the slowest way to ask rather than the fastest.
///
/// `None` is a refusal and never a verdict of no match. A caller that must take
/// every pattern falls back to [`crate::captures_iter`], which allocates a
/// capture list per match and accepts everything.
#[must_use]
pub fn captures_read_iter<'h>(pattern: &Pattern, input: &'h [u8]) -> Option<SlotCursor<'h>> {
    // A pattern that binds nothing has no registers to resolve, so the spans
    // are the whole answer and the windows below have nothing to keep. Without
    // this they run anyway and build a match a window to carry an empty capture
    // list, which measured 48 nanoseconds a match against the 5 the buffer
    // copy costs. [`crate::captures_iter`] takes the same exit first.
    if !pattern.binds()
        && let Some(spans) = crate::engine::routed_spans(pattern, input)
    {
        return Some(SlotCursor { input, source: SlotSource::Spans { spans, at: 0 } });
    }
    // One pass over the windows, keeping what each attempt bound. Taking the
    // spans from the ladder and resolving them afterward lexes every window
    // twice, because the first pass throws the save slots away to report a span.
    // Taken wherever `flat_shape` accepts the program, for the reason
    // [`crate::captures_iter`] gives: a walk over a match's bytes being cheaper
    // than a lex does not settle which rung is cheaper, and on the pattern
    // measured there the byte route below costs half as much again.
    if let Some((ms, names)) = crate::prefilter::scan_flat_by_literal_windows(pattern, input) {
        return Some(SlotCursor { input, source: SlotSource::Flat { ms, names, at: 0 } });
    }
    if let Some(ms) = crate::prefilter::scan_captures_by_literal_windows(pattern, input) {
        return Some(SlotCursor { input, source: SlotSource::Resolved { ms, at: 0 } });
    }
    if let Some(spans) = crate::engine::routed_spans(pattern, input) {
        let source = if !pattern.binds() {
            SlotSource::Spans { spans, at: 0 }
        } else if let Some((ms, names)) =
            crate::prefilter::flat_captures_by_byte_bounds(pattern, input, &spans)
        {
            SlotSource::Flat { ms, names, at: 0 }
        } else if let Some((ms, names)) =
            crate::prefilter::flat_captures_by_windows(pattern, input, &spans)
        {
            SlotSource::Flat { ms, names, at: 0 }
        } else {
            SlotSource::Resolved { ms: crate::engine::captures(pattern, input, &spans), at: 0 }
        };
        return Some(SlotCursor { input, source });
    }
    let walk = crate::nfa::SerialWalk::over(pattern, input)?;
    Some(SlotCursor { input, source: SlotSource::Walk(Box::new(walk)) })
}

/// How many registers every match of `pattern` binds, or `None` where that
/// number is not the same for every match.
///
/// The counterpart of the regex crate's `static_captures_len`. A name under a
/// star, an optional, a repeat that may run zero times, or only some branches
/// of an alternation is not bound by every match, and one under an assertion is
/// never bound at all: an assertion runs its sub-pattern as a filter and the
/// probe's own bindings are discarded.
#[must_use]
pub fn static_captures_len(pattern: &Pattern) -> Option<usize> {
    let all = crate::cursor::capture_names(pattern);
    let mut certain = Vec::new();
    certain_names(pattern, &mut certain);
    (certain.len() == all.len()).then_some(all.len())
}

/// The names every match of `pattern` binds.
fn certain_names(pattern: &Pattern, out: &mut Vec<String>) {
    match pattern {
        Pattern::Bind(name, _, inner) => {
            if !out.iter().any(|n| n == name) {
                out.push(name.clone());
            }
            certain_names(inner, out);
        }
        Pattern::Plus(p, _)
        | Pattern::Atomic(p)
        | Pattern::Balanced(_, p)
        | Pattern::Field(_, p) => certain_names(p, out),
        // A body that must run at least once binds what one run binds.
        Pattern::Repeat(p, lo, _, _) if *lo >= 1 => certain_names(p, out),
        Pattern::Concat(v) => {
            for p in v {
                certain_names(p, out);
            }
        }
        // Only a name every branch binds is bound by the alternation.
        Pattern::Alt(v, _) => {
            let Some((first, rest)) = v.split_first() else { return };
            let mut common = Vec::new();
            certain_names(first, &mut common);
            for p in rest {
                let mut theirs = Vec::new();
                certain_names(p, &mut theirs);
                common.retain(|n| theirs.contains(n));
            }
            for n in common {
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        // A body that may run zero times binds nothing, and an assertion's
        // bindings are discarded with the probe that made them.
        // An edit-distance group binds where its alignment matched, and an
        // alignment within `k` may leave any of its atoms out, so none of its
        // names is bound by every match.
        Pattern::Within(..)
        | Pattern::Star(..)
        | Pattern::Opt(..)
        | Pattern::Repeat(..)
        | Pattern::Assert(..)
        | Pattern::Empty
        | Pattern::Atom(_)
        | Pattern::Guard(..)
        | Pattern::Anchor(_) => {}
    }
}

/// How many significant tokens every match of `pattern` spans, or `None` where
/// that width is not the same for every match.
///
/// The token stream's counterpart of [`static_captures_len`], and a property no
/// byte matcher has: a regular expression's matches have no fixed width in
/// bytes because a token is not bounded in bytes, so a byte stream has no
/// answer to the same question.
///
/// It is what the route ladder already turns on. Matches that are whole tokens
/// of a fixed count cannot overlap, so a caller anchored at a position may take
/// such matches and keep the ones beginning at or after it, which is why
/// [`crate::find_at`] may filter one set of routes and not the other.
#[must_use]
pub fn static_token_extent(pattern: &Pattern) -> Option<usize> {
    match pattern {
        Pattern::Empty | Pattern::Guard(..) | Pattern::Anchor(_) | Pattern::Assert(..) => Some(0),
        Pattern::Atom(_) => Some(1),
        // Every run from `k` short of the atoms to `k` past them is offered,
        // so the width varies with the alignment unless the count is zero.
        Pattern::Within(v, 0) => Some(v.len()),
        Pattern::Within(..) => None,
        Pattern::Bind(_, _, p) | Pattern::Atomic(p) | Pattern::Field(_, p) => {
            static_token_extent(p)
        }
        // The open and the close are tokens of the match as well as the
        // interior, so a group is two wider than what it encloses.
        Pattern::Balanced(_, p) => static_token_extent(p)?.checked_add(2),
        Pattern::Concat(v) => {
            let mut total = 0usize;
            for p in v {
                total = total.checked_add(static_token_extent(p)?)?;
            }
            Some(total)
        }
        Pattern::Alt(v, _) => {
            let (first, rest) = v.split_first()?;
            let width = static_token_extent(first)?;
            for p in rest {
                if static_token_extent(p)? != width {
                    return None;
                }
            }
            Some(width)
        }
        Pattern::Repeat(p, lo, Some(hi), _) if lo == hi => {
            static_token_extent(p)?.checked_mul(*lo)
        }
        Pattern::Star(..) | Pattern::Plus(..) | Pattern::Opt(..) | Pattern::Repeat(..) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn a_refilled_buffer_answers_what_a_match_would_have() {
        // The regex crate's reusing loop, written the same way: take a match,
        // read the slots, call again from its end. The answers must be the
        // ones the allocating iterator gives, match for match and register for
        // register, or the cheap path is a different matcher.
        let hay = b"a 1 b 2 c 3";
        let p = parse("\\W:k \\N:v").expect("parses");
        let want: Vec<_> = crate::captures_iter(&p, hay).collect();
        assert_eq!(want.len(), 3, "three pairs: {want:?}");

        let mut slots = CaptureSlots::of(&p);
        let mut at = 0usize;
        let mut seen = 0usize;
        while let Some(span) = captures_read_at(&p, hay, at, &mut slots) {
            assert_eq!(span, want[seen].span(), "match {seen}");
            assert_eq!(
                slots.by_name("k"),
                want[seen].group_span("k"),
                "register k of match {seen}"
            );
            assert_eq!(
                slots.by_name("v"),
                want[seen].group_span("v"),
                "register v of match {seen}"
            );
            assert_eq!(slots.matched(), Some(span));
            at = span.end();
            seen += 1;
        }
        assert_eq!(seen, want.len(), "the loop took every match");
    }

    /// Both cursors report what the eager resolve reports on the patterns whose
    /// registers now arrive inline. The record holds spans in the program's name
    /// order and the buffer holds them in the pattern's, so a wrong pairing here
    /// is a register reported under another's name.
    #[test]
    fn the_inline_registers_are_the_ones_the_eager_resolve_reports() {
        let mut text = String::new();
        for i in 0..80u32 {
            text.push_str(&format!("let value_{i} = {} ; call_{i}(alpha, beta) ;\n", i * 37));
        }
        let hay = text.as_bytes();
        // Two registers named out of their positional order, so a path pairing
        // by position rather than by name reports them swapped.
        for src in ["\"let\" \\W:v \"=\"", "\\W:name \"=\"", "\"let\" \\W:z \"=\" \\N:a"] {
            let p = parse(src).expect("parses");
            let spans = crate::scan(&p, hay);
            let want = crate::captures(&p, hay, &spans);
            assert!(!want.is_empty(), "{src} matches the corpus");

            let got: Vec<_> = crate::captures_iter(&p, hay).collect();
            assert_eq!(got, want, "captures_iter {src}");

            let mut slots = CaptureSlots::of(&p);
            let mut c = captures_read_iter(&p, hay).expect("a route or the walk takes this");
            let mut seen = 0usize;
            while let Some(span) = c.next_into(&mut slots) {
                assert_eq!(span, want[seen].span(), "{src} match {seen}");
                for name in slots.names().to_vec() {
                    assert_eq!(
                        slots.by_name(&name),
                        want[seen].group_span(&name),
                        "{src} register {name} of match {seen}"
                    );
                }
                seen += 1;
            }
            assert_eq!(seen, want.len(), "{src} took every match");
        }
    }

    #[test]
    fn a_cursor_refilling_one_buffer_takes_the_same_matches() {
        // Holding the walk between matches must select exactly what taking
        // them one at a time selects. It is also what makes the loop linear:
        // captures_read_at restarts the route ladder on every call, so a loop
        // written with that costs one pass over the input per match.
        let hay = b"a 1 b 2 c 3 d 4";
        let p = parse("\\W:k \\N:v").expect("parses");
        let want: Vec<_> = crate::captures_iter(&p, hay).collect();
        assert_eq!(want.len(), 4, "four pairs: {want:?}");

        let mut slots = CaptureSlots::of(&p);
        let mut c = captures_read_iter(&p, hay).expect("the walk takes this pattern");
        let mut seen = 0usize;
        while let Some(span) = c.next_into(&mut slots) {
            assert_eq!(span, want[seen].span(), "match {seen}");
            assert_eq!(slots.by_name("k"), want[seen].group_span("k"), "k of match {seen}");
            assert_eq!(slots.by_name("v"), want[seen].group_span("v"), "v of match {seen}");
            seen += 1;
        }
        assert_eq!(seen, want.len(), "the cursor took every match");

        // A byte-routable pattern must take the route, which never lexes,
        // rather than the walk, which always does. The selection is the same
        // either way and that is what makes the choice free to make.
        let routed = parse("\"a\"").expect("parses");
        let mut rs = CaptureSlots::of(&routed);
        let mut rc = captures_read_iter(&routed, hay).expect("a route answers this");
        let mut got = Vec::new();
        while let Some(s) = rc.next_into(&mut rs) {
            got.push(s);
        }
        assert_eq!(got, crate::scan(&routed, hay), "the routed cursor selects what the scan does");
        assert!(rs.is_empty(), "the pattern binds nothing, so there are no slots");

        // A refusal is not a verdict of no match: the set engine takes this one
        // and a caller falls back rather than reading zero matches.
        let balanced = parse("\\B(\\N:v)").expect("parses");
        assert!(captures_read_iter(&balanced, b"(1) (2)").is_none());
        assert_eq!(crate::captures_iter(&balanced, b"(1) (2)").count(), 2);
    }

    #[test]
    fn binding_a_register_does_not_key_the_thread_list_but_reading_one_does() {
        // The engine keys its thread list on whether a register is read back,
        // not on whether one is written. Two threads at one counter holding
        // different bindings match identically from there unless something
        // reads a binding, so keying on a write keeps threads apart that
        // nothing can tell apart, and hashes the save array per thread per
        // step to do it.
        let bind_only = parse("\\W:k \"=\"").expect("parses");
        assert!(bind_only.binds(), "it writes a register");
        assert!(!bind_only.reads_registers(), "and never reads one back");

        let reads = parse("\\W:x \"=\" =x").expect("parses");
        assert!(reads.reads_registers(), "a back-reference reads one");

        // What the change must not alter: the captures reported. A pattern
        // that binds and never reads reports what it always did, and the
        // back-reference still matches only where the tokens agree.
        let hay = b"alpha = beta ; gamma = gamma ;";
        let got: Vec<_> = crate::captures_iter(&bind_only, hay)
            .map(|m| m.group_span("k").map(|s| (s.start(), s.end())))
            .collect();
        assert_eq!(got, vec![Some((0, 5)), Some((15, 20))], "both assignments, both keys");

        let back: Vec<_> = crate::captures_iter(&reads, hay)
            .map(|m| (m.span().start(), m.span().end()))
            .collect();
        assert_eq!(back.len(), 1, "only gamma = gamma repeats its token: {back:?}");
        assert_eq!(&hay[back[0].0..back[0].1], &b"gamma = gamma"[..]);
    }

    #[test]
    fn a_slot_carries_the_tokens_as_well_as_the_bytes() {
        // The save slots hold significant-token indices and every layer above
        // converts them to bytes and drops the pair. A byte matcher has no such
        // pair to keep: a byte offset does not say which token it is in.
        let hay = b"alpha 42 beta 7";
        let p = parse("\\W:k \\N:v").expect("parses");
        let mut slots = CaptureSlots::of(&p);
        assert!(captures_read(&p, hay, &mut slots).is_some(), "alpha 42 matches");

        let k = slots.index_of("k").expect("k is a register");
        let v = slots.index_of("v").expect("v is a register");
        assert_eq!(slots.tokens(k), Some(1), "a word is one token");
        assert_eq!(slots.tokens(v), Some(1), "a number is one token");
        assert_eq!(slots.matched_tokens(), Some(2), "the match spans both");

        // The extents index the significant-token stream, so the second
        // register begins where the first ends.
        let (ks, ke) = slots.extent(k).expect("k bound");
        let (vs, ve) = slots.extent(v).expect("v bound");
        assert_eq!(ke, vs, "adjacent tokens: k ends at {ke}, v starts at {vs}");
        assert_eq!(slots.matched_extent(), Some((ks, ve)));
    }

    #[test]
    fn clearing_keeps_the_names_and_forgets_the_positions() {
        let p = parse("\\W:k \\N:v").expect("parses");
        let mut slots = CaptureSlots::of(&p);
        assert!(captures_read(&p, b"alpha 42", &mut slots).is_some());
        slots.clear();
        assert_eq!(slots.len(), 2, "the shape is the pattern's, not the match's");
        assert_eq!(slots.get(0), None);
        assert_eq!(slots.matched(), None);
        assert_eq!(slots.names().to_vec(), vec!["k".to_string(), "v".to_string()]);
    }

    #[test]
    fn a_register_that_need_not_bind_has_no_static_count() {
        assert_eq!(static_captures_len(&parse("\\W:k \\N:v").expect("parses")), Some(2));
        assert_eq!(
            static_captures_len(&parse("\\W:k (\\N:v)?").expect("parses")),
            None,
            "v is bound by some matches and not others"
        );
        assert_eq!(
            static_captures_len(&parse("\\W:k | \\N:k").expect("parses")),
            Some(1),
            "every branch binds k"
        );
        assert_eq!(
            static_captures_len(&parse("\\W:k | \\N:v").expect("parses")),
            None,
            "neither name is bound by both branches"
        );
        // An assertion is a filter and the probe's bindings are discarded, so a
        // name only under one is never bound by any match. Every match of this
        // binds exactly none, which is a fixed count and not a varying one.
        let asserted = parse("\\W ~(\\N:v)").expect("parses");
        assert_eq!(crate::capture_names(&asserted), Vec::<String>::new(), "v cannot bind");
        assert_eq!(static_captures_len(&asserted), Some(0), "every match binds none");
    }

    #[test]
    fn a_fixed_token_width_is_a_property_no_byte_matcher_has() {
        assert_eq!(static_token_extent(&parse("\\W \\N").expect("parses")), Some(2));
        assert_eq!(static_token_extent(&parse("\\W{3}").expect("parses")), Some(3));
        assert_eq!(
            static_token_extent(&parse("\\B(\\N)").expect("parses")),
            Some(3),
            "the open and the close are tokens of the match"
        );
        assert_eq!(
            static_token_extent(&parse("\\W ~(\\N)").expect("parses")),
            Some(1),
            "an assertion consumes nothing"
        );
        assert_eq!(static_token_extent(&parse("\\W | \\N").expect("parses")), Some(1));
        assert_eq!(
            static_token_extent(&parse("\\W | \\N \\N").expect("parses")),
            None,
            "the branches are one token and two"
        );
        assert_eq!(static_token_extent(&parse("\\W*").expect("parses")), None);
        assert_eq!(static_token_extent(&parse("\\W{2,4}").expect("parses")), None);
    }

    #[test]
    fn anchoring_selects_where_a_match_may_begin_and_never_what_the_input_is() {
        let hay = b"a 1 b 2 c 3";
        let p = parse("\\W:k \\N:v").expect("parses");
        let all: Vec<_> = crate::captures_iter(&p, hay).collect();
        assert_eq!(all.len(), 3);

        let after_first = all[0].span().end();
        let got = crate::captures_at(&p, hay, after_first).expect("a match follows the first");
        assert_eq!(got.span(), all[1].span(), "the next match, not the first again");
        assert_eq!(got.group_span("k"), all[1].group_span("k"));

        assert_eq!(crate::shortest_match_at(&p, hay, 0), crate::shortest_match(&p, hay));
        assert_eq!(
            crate::shortest_match_at(&p, hay, after_first),
            Some(all[1].span().end()),
            "the soonest end at or after the position"
        );
        assert_eq!(crate::captures_at(&p, hay, hay.len()), None, "nothing begins past the end");
    }

    #[test]
    fn a_set_reports_where_each_member_matched() {
        // The regex crate's RegexSet reports which patterns match and states
        // that it does not report where: one automaton carrying every pattern
        // loses which of them reached an accepting state. A token set shares a
        // lex rather than an automaton, so each member is walked as itself and
        // keeps its span.
        let hay = b"alpha 42";
        let set = crate::PatternSet::new(vec![
            parse("\\N").expect("parses"),
            parse("\"zzzqqq\"").expect("parses"),
            parse("\\W").expect("parses"),
        ]);

        let hits = set.matches_with_spans(hay);
        assert_eq!(hits.iter().map(|&(i, _)| i).collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(&hay[hits[0].1.start()..hits[0].1.end()], &b"42"[..]);
        assert_eq!(&hay[hits[1].1.start()..hits[1].1.end()], &b"alpha"[..]);

        let m = set.matched(hay);
        assert!(m.matched(0) && !m.matched(1) && m.matched(2));
        assert!(m.matched_any(), "two of three");
        assert!(!m.matched_all(), "zzzqqq is nowhere in the input");
        assert_eq!(m.iter().collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(m.len(), 3, "every index carries a verdict, matched or not");

        // Anchored past the word: the number is still ahead and the word is not.
        let at = set.matches_at(hay, 5);
        assert!(at.matched(0), "42 begins at or after byte 5");
        assert!(!at.matched(2), "alpha ends before it");
        assert!(set.is_match_at(hay, 5));
        assert!(!set.is_match_at(hay, hay.len()), "nothing begins past the end");
    }
}
