//! Matches taken one at a time, and the operations that stop before the end.
//!
//! [`crate::scan`] returns every match. A caller that wants the first, or the
//! first three, or the text between matches, computes all of them and discards
//! what it did not want. The regex crate divides the same surface into `find`,
//! `find_iter`, `split`, `replacen` and the rest; each is this cursor with a
//! different stopping rule, which is why they live together here rather than
//! as separate walks over the input.
//!
//! # What stopping early saves, and what it does not
//!
//! A match over bytes costs one pass, so stopping the pass early is the whole
//! saving. A match over tokens costs a lex and then a walk, the lex runs at a
//! pattern-independent rate, and the walk is the cheaper half. So a cursor
//! that stops the walk early and lexes the input in full saves the cheaper
//! half only: correct, and not yet fast.
//!
//! That is deliberate and it is the first of two steps. This cursor is the
//! surface and the correctness contract - every operation below agrees span
//! for span with the equivalent over [`crate::scan`], which is what its tests
//! assert. A source that lexes a block at a time goes underneath it next,
//! against this one as the control arm.
//!
//! The routes are the exception already: a pattern a byte route answers never
//! reaches the lexer, here or in [`crate::scan`], because both take the same
//! ladder in [`crate::engine::routed_spans`].

use crate::ast::Pattern;
use crate::engine::Span;

/// Where a cursor's matches come from.
enum Source {
    /// A route answered without the engine, and hands over every match at
    /// once. Position is an index into them.
    ///
    /// These routes do not lex the input, so taking all of their matches is
    /// not the cost that taking all of the engine's would be.
    Routed { spans: Vec<Span>, at: usize },
    /// The single-pass engine, held between matches so the next one resumes
    /// rather than restarts.
    ///
    /// Boxed because a held walk owns its token stream, its compiled program
    /// and two thread lists, and an enum is as large as its largest variant:
    /// inline, every routed cursor would carry the walk's size without ever
    /// holding one.
    Walk(Box<crate::nfa::SerialWalk<crate::nfa::OwnedStream>>),
}

/// The matches of one pattern over one input, taken in order and on demand.
///
/// Leftmost and non-overlapping, the same selection [`crate::scan`] makes:
/// collecting a cursor and scanning give the same spans in the same order.
pub struct Cursor<'h> {
    input: &'h [u8],
    source: Source,
    /// The start byte of every significant token, ascending, built only if a
    /// caller asks for a token extent from a source that did not lex.
    ///
    /// A byte route answers without the lexer, which is the whole of what it
    /// is for, so counting tokens for it costs the lex the route avoided.
    /// That cost comes with the first extent asked for and not before, and
    /// never at all for a caller that only wants spans.
    starts: Option<Vec<usize>>,
}

impl<'h> Cursor<'h> {
    /// A cursor over every match of `pattern` in `input`.
    #[must_use]
    pub fn new(pattern: &Pattern, input: &'h [u8]) -> Self {
        if let Some(shapes) = crate::library::shapes_for(pattern) {
            return Cursor::over_library_kinds(pattern, input, &shapes);
        }
        if let Some(spans) = crate::engine::routed_spans(pattern, input) {
            return Cursor { input, source: Source::Routed { spans, at: 0 }, starts: None };
        }
        Cursor::past_the_routes(pattern, input)
    }

    /// A cursor for a pattern naming a library kind, whose tokens only a lex
    /// under the library's shapes produces: the matches arrive together from
    /// that lex, with the token starts it saw.
    fn over_library_kinds(pattern: &Pattern, input: &'h [u8], shapes: &crate::custom::ShapeSet) -> Self {
        let blobs = crate::lexer::blob_runs(input);
        let toks = crate::lexer::lex_with_shapes(input, &blobs, shapes, 0);
        let spans = match crate::nfa::scan_nfa_over(pattern, input, &toks) {
            Some(spans) => spans,
            None => crate::engine::scan_tokens_from(pattern, input, &toks, 0),
        };
        let starts =
            toks.iter().filter(|t| t.is_significant()).map(crate::token::Token::start).collect();
        Cursor { input, source: Source::Routed { spans, at: 0 }, starts: Some(starts) }
    }

    /// A cursor over spans the scan found together, for a caller that will take
    /// all of them.
    ///
    /// [`Self::new`] holds a resumable walk where no route answers, which stops
    /// at each match and carries its state to the next. That is what a caller
    /// taking one match wants and it costs the cores: the scan dispatches its
    /// attempt per anchor across them and a walk yielding matches in order
    /// cannot. On the comparison's corpus a scan finds fifty thousand matches in
    /// 11.7 ms where the walk takes 44.1, and both lex once.
    ///
    /// So a caller that will exhaust the cursor takes the scan instead. The
    /// spans are the same and in the same order, which is this type's own
    /// contract.
    fn over_every_match(pattern: &Pattern, input: &'h [u8]) -> Self {
        let spans = crate::engine::scan(pattern, input);
        Cursor { input, source: Source::Routed { spans, at: 0 }, starts: None }
    }

    /// A cursor for a pattern whose routes have already been tried and
    /// declined, so the ladder is not walked twice.
    fn past_the_routes(pattern: &Pattern, input: &'h [u8]) -> Self {
        // The walk lexes only if it takes the pattern, so a refusal here costs
        // the compile and not the input.
        if let Some(walk) = crate::nfa::SerialWalk::over(pattern, input) {
            return Cursor { input, source: Source::Walk(Box::new(walk)), starts: None };
        }
        // A balanced group or a field node, which only the set-reachability
        // engine advances. It has no resumable form, so its matches arrive
        // together and the cursor hands them out.
        let spans = crate::engine::scan_set_reachability(pattern, input);
        Cursor { input, source: Source::Routed { spans, at: 0 }, starts: None }
    }

    /// The next match and how many significant tokens it spans.
    ///
    /// A match's length in tokens is the unit this engine actually works in -
    /// a match is bounded in tokens and a token is not bounded in bytes, so a
    /// blob run of several kilobytes is one of them. [`Span`] reports bytes
    /// and is deliberately eight of them, so the count rides here rather than
    /// on the span.
    ///
    /// The engine walk names a match by the token indices it runs between, so
    /// the count is free there. A byte route never lexed, so the first call
    /// against one builds an index of significant-token starts and later
    /// calls read it.
    pub fn next_extent(&mut self) -> Option<(Span, usize)> {
        if let Source::Walk(w) = &mut self.source {
            return w.next_span_and_extent(self.input);
        }
        let s = self.next()?;
        // Read out before the index is borrowed, so building it does not hold
        // the cursor while reading the cursor's own input.
        let input = self.input;
        let starts = self.starts.get_or_insert_with(|| {
            crate::parallel_lex::lex_parallel(input)
                .iter()
                .filter(|t| t.is_significant())
                .map(crate::token::Token::start)
                .collect()
        });
        let first = starts.partition_point(|&b| b < s.start());
        let past = starts.partition_point(|&b| b < s.end());
        Some((s, past - first))
    }
}

impl Iterator for Cursor<'_> {
    type Item = Span;

    fn next(&mut self) -> Option<Span> {
        match &mut self.source {
            Source::Routed { spans, at } => {
                let s = spans.get(*at).copied();
                if s.is_some() {
                    *at += 1;
                }
                s
            }
            Source::Walk(w) => w.next_span(self.input),
        }
    }
}

/// Where `pattern` first matches in `input`, or `None` where it does not.
///
/// The counterpart of the regex crate's `find`, and the answer
/// `scan(pattern, input).first().copied()` gives without finding the rest.
///
/// Three sources, in the order of what they cost. A byte route answers from
/// the input's bytes and never lexes. Failing that, a prefix is lexed and
/// widened until it settles the answer, which is the saving on an input whose
/// first match is not near its end. Failing that - an anchor that reads the
/// whole stream, or a pattern the single-pass engine does not take - the whole
/// input is lexed and the cursor takes its first match.
#[must_use]
pub fn find(pattern: &Pattern, input: &[u8]) -> Option<Span> {
    if crate::library::shapes_for(pattern).is_some() {
        crate::trace::rung("find", "a lex under the library's shapes", input.len());
        return Cursor::new(pattern, input).next();
    }
    // The routed answer read to the first match and no further, where the route
    // can do that. Taking every match and discarding all but one reads the
    // whole input to answer about a match that is often in its first bytes.
    if let Some(first) = crate::engine::routed_first(pattern, input) {
        // Named for what this rung knows, which is that neither the prefix
        // nor the cursor ran. Which route answered is the inner rung's to
        // report: the windows route lexes at each opening literal and reaches
        // here as well as the byte routes do.
        crate::trace::rung("find", "a route, not the cursor", input.len());
        return first;
    }
    if let Some(answer) = crate::prefilter::find_by_growing_prefix(pattern, input) {
        crate::trace::rung("find", "a widening prefix", input.len());
        return answer;
    }
    crate::trace::rung("find", "the cursor over a whole lex", input.len());
    Cursor::past_the_routes(pattern, input).next()
}

/// [`find`] with the whole input lexed up front and no widening prefix.
///
/// The arm [`find`] is measured against: same routes, same engine, same
/// selection, and the only difference is how much of the input reaches the
/// lexer. Keeping it callable is what lets the two be timed side by side on
/// one corpus instead of across two builds.
#[doc(hidden)]
#[must_use]
pub fn find_by_full_lex(pattern: &Pattern, input: &[u8]) -> Option<Span> {
    Cursor::new(pattern, input).next()
}

/// Every match of `pattern` in `input`, in order, taken as they are asked for.
///
/// The lazy form of [`crate::scan`]: the same spans in the same order, without
/// a vector holding all of them.
pub fn find_iter<'h>(pattern: &Pattern, input: &'h [u8]) -> Cursor<'h> {
    Cursor::new(pattern, input)
}

/// Where `pattern` first matches at or after byte `at`.
///
/// Not the same question as the first match of the whole input that happens to
/// start at or after `at`: the leftmost, non-overlapping selection from `at`
/// can hold a match that overlaps one the selection from zero preferred, so
/// this re-runs the selection from there rather than filtering.
///
/// Not for walking an input. Each call builds its own byte reader, whose quote
/// scan runs from the input's start to `at`, so a loop that advances `at`
/// through every match reads those bytes again for each one. A few calls cost
/// little and a walk is quadratic. [`find_iter`] and [`crate::captures_iter`]
/// hold their state between matches and are what a walk wants.
#[must_use]
pub fn find_at(pattern: &Pattern, input: &[u8], at: usize) -> Option<Span> {
    if let Some(shapes) = crate::library::shapes_for(pattern) {
        return crate::engine::scan_with_shapes_from(pattern, input, &shapes, at).into_iter().next();
    }
    // The routes whose matches are whole tokens of a fixed count cannot
    // overlap, so the ones at or after `at` are the ones that begin there.
    // Taking them costs no lex, and where the route has an anchored form the
    // scan begins at `at` rather than reading the bytes before it to find
    // matches the caller has already excluded.
    // The rung is named by routed_first_at itself, which knows whether a byte
    // route answered or the whole positional set was filtered.
    if let Some(first) = crate::engine::routed_first_at(pattern, input, at) {
        return first;
    }
    // A prefix lexed and widened until it settles the answer, as [`find`] takes
    // for the same question from zero. A caller asks from where it last
    // stopped, so the prefix that reaches past the offset is usually small.
    if let Some(answer) = crate::prefilter::find_at_by_growing_prefix(pattern, input, at) {
        crate::trace::rung("find_at", "a widening prefix from the offset", input.len());
        return answer;
    }
    if let Some(mut w) = crate::nfa::SerialWalk::over(pattern, input) {
        crate::trace::rung("find_at", "the held walk over a whole fused lex", input.len());
        w.seek(at);
        return w.next_span(input);
    }
    // A balanced or field pattern, which the walk declines and the set engine
    // takes. The walk lexes nothing when it declines, so this is the only lex
    // on this path.
    let toks = crate::parallel_lex::lex_parallel(input);
    let start = toks.partition_point(|t| t.start() < at);
    crate::engine::scan_tokens_from(pattern, input, &toks, start).into_iter().next()
}

/// Whether `pattern` matches anywhere at or after byte `at`.
#[must_use]
pub fn is_match_at(pattern: &Pattern, input: &[u8], at: usize) -> bool {
    find_at(pattern, input, at).is_some()
}

/// Where a match cursor's matches come from.
enum MatchSource<'h> {
    /// A route answered, so the spans arrived together and their registers
    /// were resolved together.
    ///
    /// Held as the vector's own iterator rather than a vector and an index: the
    /// cursor owns these matches and is the only thing that will ever read
    /// them, so handing one out is a move. Indexing meant cloning a match a
    /// caller was about to be given - a capture list and a name per register,
    /// measured at 4.058 ms of the 14.436 that `captures_iter` cost over fifty
    /// thousand matches.
    Resolved(std::vec::IntoIter<crate::engine::Match>),
    /// A route answered a pattern whose program is a flat run of atoms, so the
    /// registers came back inline and a match is built only when it is asked
    /// for.
    ///
    /// [`MatchSource::Resolved`] owns a vector a match before the caller has
    /// asked for any of them, which on fifty thousand matches is fifty thousand
    /// live allocations where one would do. These records allocate nothing, and
    /// the one match this hands out is freed before the next is built, so the
    /// allocator serves them all from the same block.
    Flat {
        ms: std::vec::IntoIter<crate::nfa::FlatMatch>,
        names: std::sync::Arc<[String]>,
    },
    /// A pattern that binds nothing, so every match carries no registers and
    /// there is nothing to resolve. Spans come from an ordinary cursor and
    /// each is widened to a match with an empty capture list.
    Plain(Box<Cursor<'h>>),
    /// The single-pass engine, held between matches. Its save slots are
    /// resolved as each match is taken. Boxed for the reason [`Source`] gives.
    Walk(Box<crate::nfa::SerialWalk<crate::nfa::OwnedStream>>),
}

/// The matches of one pattern over one input with the registers each bound,
/// taken in order and on demand.
///
/// [`Cursor`] over [`crate::engine::Match`] rather than [`Span`]: the same
/// selection, and each match carries what it bound.
pub struct MatchCursor<'h> {
    input: &'h [u8],
    source: MatchSource<'h>,
    /// The match [`MatchCursor::next_ref`] last took, held so the view it hands
    /// back has something to borrow. Untouched by the owning iterator.
    held: Option<Held>,
}

/// One match kept inside the cursor for a borrowed view to point at.
enum Held {
    /// A flat record, whose registers are inline and whose names belong to the
    /// source rather than to it.
    Flat(crate::nfa::FlatMatch),
    /// A match already built, whose names it owns a share of.
    Owned(crate::engine::Match),
    /// A span from a pattern that binds nothing.
    Span(Span),
}

/// One match, borrowing the cursor that produced it.
///
/// [`crate::Match`] owns a share of the pattern's register names, which costs
/// the two atomics of a clone and a drop and gives every match a destructor to
/// run. Over fifty thousand matches that is 30.2 nanoseconds each, measured
/// against the same loop building matches that bind nothing. A caller that
/// reads each match in turn and keeps none of them incurs neither here.
///
/// Borrowed from the cursor and not from the input, so a view is invalidated by
/// asking for the next match. [`MatchRef::to_match`] takes an owned copy where
/// one is wanted.
pub struct MatchRef<'c> {
    /// Inclusive start byte offset of the match in the input.
    pub start: usize,
    /// Exclusive end byte offset of the match in the input.
    pub end: usize,
    regs: &'c [Span],
    names: &'c [String],
}

impl<'c> MatchRef<'c> {
    /// The match's own span.
    #[must_use]
    pub fn span(&self) -> Span {
        Span { start: self.start as u32, end: self.end as u32 }
    }

    /// The spans this match's registers bound, in the order [`Self::names`]
    /// holds their names.
    #[must_use]
    pub fn captures(&self) -> &[Span] {
        self.regs
    }

    /// The register names, in the order [`Self::captures`] holds their spans.
    #[must_use]
    pub fn names(&self) -> &[String] {
        self.names
    }

    /// The bytes the register called `name` bound, or `None` where the pattern
    /// has no such register.
    #[must_use]
    pub fn group<'h>(&self, name: &str, input: &'h [u8]) -> Option<&'h [u8]> {
        let k = self.names.iter().position(|n| n == name)?;
        self.regs.get(k).map(|s| &input[s.range()])
    }

    /// This match as an owned one, which is where the share of the names and
    /// the destructor that come with it are incurred.
    #[must_use]
    pub fn to_match(&self) -> crate::engine::Match {
        if self.names.is_empty() {
            return crate::engine::Match::plain(self.start, self.end);
        }
        crate::engine::Match::bound(
            self.start,
            self.end,
            crate::engine::Regs::from_slice(self.regs),
            std::sync::Arc::from(self.names.to_vec()),
        )
    }
}

impl MatchCursor<'_> {
    /// The next match, borrowing this cursor rather than owning a share of the
    /// pattern's names.
    ///
    /// The lending counterpart of this cursor's [`Iterator`], for a caller that
    /// reads each match in turn and keeps none. A view borrows the cursor, so
    /// asking for the next match invalidates the one before it, which is why
    /// this is a method and not an `Iterator`.
    ///
    /// Where a route answered the pattern the registers are already inline and
    /// the names belong to the cursor, so no match is built and nothing is
    /// cloned. Where the engine ran, a match was built to resolve its save
    /// slots and this borrows that; the saving is the route's.
    pub fn next_ref(&mut self) -> Option<MatchRef<'_>> {
        let input = self.input;
        self.held = match &mut self.source {
            MatchSource::Flat { ms, .. } => ms.next().map(Held::Flat),
            MatchSource::Resolved(ms) => ms.next().map(Held::Owned),
            MatchSource::Plain(c) => c.next().map(Held::Span),
            MatchSource::Walk(w) => w.next_match(input).map(Held::Owned),
        };
        let flat_names: &[String] = match &self.source {
            MatchSource::Flat { names, .. } => names,
            _ => &[],
        };
        match self.held.as_ref()? {
            Held::Flat(m) => Some(MatchRef {
                start: m.span.start(),
                end: m.span.end(),
                regs: &m.regs[..flat_names.len()],
                names: flat_names,
            }),
            Held::Owned(m) => {
                Some(MatchRef { start: m.start, end: m.end, regs: m.captures(), names: m.names() })
            }
            Held::Span(s) => {
                Some(MatchRef { start: s.start(), end: s.end(), regs: &[], names: &[] })
            }
        }
    }
}

impl Iterator for MatchCursor<'_> {
    type Item = crate::engine::Match;

    fn next(&mut self) -> Option<crate::engine::Match> {
        match &mut self.source {
            MatchSource::Resolved(ms) => ms.next(),
            MatchSource::Flat { ms, names } => ms.next().map(|m| {
                crate::engine::Match::bound(
                    m.span.start(),
                    m.span.end(),
                    crate::engine::Regs::from_slice(&m.regs[..names.len()]),
                    names.clone(),
                )
            }),
            MatchSource::Plain(c) => c.next().map(crate::engine::Match::from),
            MatchSource::Walk(w) => w.next_match(self.input),
        }
    }
}

/// Every match of `pattern` in `input` with its captures, taken as they are
/// asked for.
///
/// The lazy form of [`crate::captures`], and the counterpart of the regex
/// crate's `captures_iter`. Where the engine runs, a match's registers are
/// resolved from the save slots the walk already carried, so taking the first
/// match resolves one match's worth rather than every match's.
pub fn captures_iter<'h>(pattern: &Pattern, input: &'h [u8]) -> MatchCursor<'h> {
    // A pattern that binds nothing has no registers to resolve, so the engine
    // has nothing to be run for. Spans come from an ordinary cursor and each
    // widens to a match with an empty capture list.
    if !pattern.binds() {
        let c = Cursor::new(pattern, input);
        return MatchCursor { input, source: MatchSource::Plain(Box::new(c)), held: None };
    }
    // A library kind lives only in a lex under the library's shapes, which
    // the scan and the resolution both take; the matches arrive together.
    if crate::library::shapes_for(pattern).is_some() {
        let spans = crate::engine::scan(pattern, input);
        let ms = crate::engine::captures(pattern, input, &spans);
        return MatchCursor { input, source: MatchSource::Resolved(ms.into_iter()), held: None };
    }
    // One pass over the windows, keeping what each attempt bound, with the
    // registers inline and the names held once here. Taking the spans from the
    // ladder and resolving them afterward lexes every window twice and runs
    // every attempt twice, because the first pass throws the save slots away to
    // report a span.
    //
    // Taken wherever `flat_shape` accepts the program, including shapes the
    // bytes do walk: for `"let" \W:v "="` this answers at 7.9091 ms against
    // 10.9338 for the byte route below, so a walk over a match's bytes being
    // cheaper than a lex does not settle which rung is cheaper.
    if let Some((ms, names)) = crate::prefilter::scan_flat_by_literal_windows(pattern, input) {
        crate::trace::rung("captures_iter", "windows, registers inline", input.len());
        return MatchCursor { input, source: MatchSource::Flat { ms: ms.into_iter(), names }, held: None };
    }
    // No such guard here, measured rather than assumed: this rung answers
    // `"let" \W:v "="` at 8.7636 ms where the byte-route rung below answers it
    // at 10.9338, so the vector of matches it builds is worth what it costs on
    // a shape the bytes do walk.
    if let Some(ms) = crate::prefilter::scan_captures_by_literal_windows(pattern, input) {
        crate::trace::rung("captures_iter", "windows, matched and resolved at once", input.len());
        return MatchCursor { input, source: MatchSource::Resolved(ms.into_iter()), held: None };
    }
    if let Some(spans) = crate::engine::routed_spans(pattern, input) {
        // The route never lexed, and the bytes of a match say where its atoms'
        // tokens are, so this resolves without lexing either.
        if let Some((ms, names)) =
            crate::prefilter::flat_captures_by_byte_bounds(pattern, input, &spans)
        {
            crate::trace::rung("captures_iter", "a byte route, registers off the bytes", input.len());
            return MatchCursor { input, source: MatchSource::Flat { ms: ms.into_iter(), names }, held: None };
        }
        if let Some((ms, names)) = crate::prefilter::flat_captures_by_windows(pattern, input, &spans) {
            crate::trace::rung("captures_iter", "a byte route, registers inline", input.len());
            return MatchCursor { input, source: MatchSource::Flat { ms: ms.into_iter(), names }, held: None };
        }
        let ms = crate::engine::captures(pattern, input, &spans);
        crate::trace::rung("captures_iter", "a byte route, resolved together", input.len());
        return MatchCursor { input, source: MatchSource::Resolved(ms.into_iter()), held: None };
    }
    if let Some(walk) = crate::nfa::SerialWalk::over(pattern, input) {
        crate::trace::rung("captures_iter", "the held walk over a whole fused lex", input.len());
        return MatchCursor { input, source: MatchSource::Walk(Box::new(walk)), held: None };
    }
    let spans = crate::engine::scan_set_reachability(pattern, input);
    let ms = crate::engine::captures(pattern, input, &spans);
    MatchCursor { input, source: MatchSource::Resolved(ms.into_iter()), held: None }
}

/// The captures of the first match of `pattern` in `input`.
///
/// The counterpart of the regex crate's `captures`, which resolves one
/// match's registers rather than every match's.
///
/// It takes the same widening prefix [`find`] does, and resolves the match's
/// registers over the tokens that prefix holds - the same tokens the match
/// was found over. Asking a full lex for them would cost the whole input to
/// answer about a match a sixty-fourth of a megabyte already settled.
#[must_use]
pub fn captures_first(pattern: &Pattern, input: &[u8]) -> Option<crate::engine::Match> {
    if crate::library::shapes_for(pattern).is_some() {
        let first = find(pattern, input)?;
        return crate::engine::captures(pattern, input, &[first]).into_iter().next();
    }
    if let Some(first) = crate::engine::routed_first(pattern, input) {
        let first = first?;
        return crate::engine::captures(pattern, input, &[first]).into_iter().next();
    }
    let settled = crate::prefilter::settle_first_from_a_prefix(pattern, input, |s, toks| {
        crate::nfa::captures_over(pattern, input, toks, &[s]).and_then(|v| v.into_iter().next())
    });
    match settled {
        // The prefix settled it and resolved what the match bound.
        Some(Some(Some(m))) => Some(m),
        // The prefix reached the whole input and found nothing.
        Some(None) => None,
        // Either no prefix can settle this pattern, or one found a match and
        // this engine would not resolve its registers. Both are questions for
        // the full lex rather than answers, and reporting no match for either
        // would report absence where a match was actually found.
        Some(Some(None)) | None => captures_iter(pattern, input).next(),
    }
}

/// The captures of the first match of `pattern` at or after byte `at`.
///
/// The counterpart of the regex crate's `captures_at`: it is to
/// [`captures_first`] what [`find_at`] is to [`find`]. The bytes before `at`
/// are still read, so a backward assertion and a start anchor see what precedes
/// the position; slicing the input instead would hide it from them and report a
/// different answer.
///
/// It takes the same ladder as [`find_at`] and resolves one match's registers
/// at the end of it, so no route is walked twice and nothing is lexed twice.
#[must_use]
pub fn captures_at(pattern: &Pattern, input: &[u8], at: usize) -> Option<crate::engine::Match> {
    // Nothing bound means nothing to resolve, so the span answer is the whole
    // answer and the engine has no reason to run.
    if !pattern.binds() {
        return find_at(pattern, input, at).map(crate::engine::Match::from);
    }
    if crate::library::shapes_for(pattern).is_some() {
        let first = find_at(pattern, input, at)?;
        return crate::engine::captures(pattern, input, &[first]).into_iter().next();
    }
    // Only the routes whose matches cannot overlap may be filtered by the
    // caller's position, which is why the ladder is split at that line. An
    // anchored ask takes the anchored form, which begins its scan at `at`.
    if let Some(first) = crate::engine::routed_first_at(pattern, input, at) {
        let first = first?;
        return crate::engine::captures(pattern, input, &[first]).into_iter().next();
    }
    // The prefix finds the span, and resolving one span costs the region around
    // it rather than a second reading of the input.
    if let Some(first) = crate::prefilter::find_at_by_growing_prefix(pattern, input, at) {
        crate::trace::rung("captures_at", "a widening prefix from the offset", input.len());
        let first = first?;
        return crate::engine::captures(pattern, input, &[first]).into_iter().next();
    }
    if let Some(mut w) = crate::nfa::SerialWalk::over(pattern, input) {
        crate::trace::rung("captures_at", "the held walk over a whole fused lex", input.len());
        w.seek(at);
        return w.next_match(input);
    }
    // A balanced or field pattern, which the walk declines and the set engine
    // takes. The walk lexes nothing when it declines, so this is the only lex.
    let toks = crate::parallel_lex::lex_parallel(input);
    let start = toks.partition_point(|t| t.start() < at);
    let span = crate::engine::scan_tokens_from(pattern, input, &toks, start).into_iter().next()?;
    crate::engine::captures_over(pattern, input, &toks, &[span]).into_iter().next()
}

/// How far the soonest-ending match of `pattern` in `input` reaches, as a
/// byte offset, or `None` where nothing matches.
///
/// The counterpart of the regex crate's `shortest_match`, with the same
/// relation to [`find`] there as here: the end reported can be earlier
/// than the match [`find`] returns, because the question is where a match is
/// first known to have occurred rather than which match the pattern prefers.
/// Where a pattern has one way to match, the two agree.
///
/// A pattern the single-pass engine does not take - a balanced group, a field
/// node - has no earliest-accept to report, so this answers from the ordinary
/// scan and the two ends are the same.
#[must_use]
pub fn shortest_match(pattern: &Pattern, input: &[u8]) -> Option<usize> {
    // A library kind is one token with one way to match, so the first match
    // under the library's lex ends where the soonest one does.
    if crate::library::shapes_for(pattern).is_some() {
        return find(pattern, input).map(|s| s.end());
    }
    // A byte route reports whole-token matches with one way to match each, so
    // its first match is also the soonest-ending one, and it is read to that
    // match and no further.
    if let Some(first) = crate::engine::routed_first_positional(pattern, input) {
        crate::trace::rung("shortest_match", "a positional byte route, no lex", input.len());
        return first.map(|s| s.end());
    }
    // The windows a required literal opens, each asked where a match is first
    // known to have occurred. The selective routes cannot answer this caller
    // from their matches, since a later match can end sooner than a selected
    // one, but a window answers the question directly over its own tokens.
    match crate::prefilter::shortest_end_in_windows(pattern, input) {
        Ok(end) => {
            crate::trace::rung("shortest_match", "the windows a literal opens", input.len());
            return end;
        }
        // Every refusal falls to the next rung, so the ladder itself needs no
        // more than that one was made. The reason is counted rather than
        // dropped: a pattern the windows can never take and one this input
        // happens to refuse are the same fall from here and different facts,
        // and the counter is what tells them apart afterward.
        Err(why) => {
            crate::trace::rung("shortest_match", &format!("the windows refuse: {why}"), input.len());
        }
    }
    // A prefix lexed and widened until it settles the answer, as [`find`] takes
    // for its own question. A match beginning past the cut ends past it, so an
    // end clear of the cut is already the soonest.
    if let Some(end) = crate::prefilter::shortest_end_by_growing_prefix(pattern, input) {
        crate::trace::rung("shortest_match", "a widening prefix", input.len());
        return end;
    }
    // The engine over the significant stream in the lexer's parts. It lexes
    // only once the pattern has compiled, so a pattern it declines costs the
    // compile and not the input, and the stitched stream below is reached only
    // by a pattern it does not take.
    if let Some(end) = crate::nfa::shortest_end_from_byte(pattern, input, 0) {
        crate::trace::rung("shortest_match", "the engine over a whole fused lex", input.len());
        return Some(end);
    }
    if crate::nfa::compile_pattern(pattern).is_some() {
        // The engine took the pattern and found nothing, which is an answer.
        crate::trace::rung("shortest_match", "the engine, which found nothing", input.len());
        return None;
    }
    crate::trace::rung("shortest_match", "the set engine over a stitched lex", input.len());
    let toks = crate::parallel_lex::lex_parallel(input);
    crate::engine::scan_tokens_from(pattern, input, &toks, 0).first().map(Span::end)
}

/// How far the soonest-ending match of `pattern` at or after byte `at` reaches.
///
/// The counterpart of the regex crate's `shortest_match_at`. The bytes before
/// `at` are read as [`captures_at`] reads them, so the position selects where a
/// match may begin and never what the input is.
#[must_use]
pub fn shortest_match_at(pattern: &Pattern, input: &[u8], at: usize) -> Option<usize> {
    if crate::library::shapes_for(pattern).is_some() {
        return find_at(pattern, input, at).map(|s| s.end());
    }
    if let Some(first) = crate::engine::routed_first_at(pattern, input, at) {
        return first.map(|s| s.end());
    }
    // A prefix widened until it settles the answer, as [`shortest_match`] takes
    // for the same question from zero.
    if let Some(end) = crate::prefilter::shortest_end_by_growing_prefix_from(pattern, input, at) {
        crate::trace::rung("shortest_match_at", "a widening prefix from the offset", input.len());
        return end;
    }
    if let Some(end) = crate::nfa::shortest_end_from_byte(pattern, input, at) {
        crate::trace::rung("shortest_match_at", "the engine over a whole fused lex", input.len());
        return Some(end);
    }
    if crate::nfa::compile_pattern(pattern).is_some() {
        // The engine took the pattern and found nothing, which is an answer.
        crate::trace::rung("shortest_match_at", "the engine, which found nothing", input.len());
        return None;
    }
    crate::trace::rung("shortest_match_at", "the set engine over a stitched lex", input.len());
    let toks = crate::parallel_lex::lex_parallel(input);
    let start = toks.partition_point(|t| t.start() < at);
    crate::engine::scan_tokens_from(pattern, input, &toks, start).first().map(Span::end)
}

/// The pieces of `input` between the matches of `pattern`.
///
/// The gaps a [`Cursor`] leaves, which is why it is the same walk: a piece
/// ends where the next match begins and the next piece starts where that match
/// ended. A match at the very start or the very end yields an empty piece on
/// that side, so the pieces always number one more than the matches and
/// rejoining them with the matched text gives back the input.
/// Where a split's separators come from.
enum Separators<'h> {
    /// A cursor, which either holds every match or resumes a walk.
    Cursor(Box<Cursor<'h>>),
    /// One separator at a time from an ascending offset, over a reader held
    /// across the pieces so the quote scan runs once for all of them.
    ///
    /// For a limited split whose pattern an anchored byte route takes: a cursor
    /// would find every match in the input to hand over three. The pattern is
    /// cloned rather than borrowed so [`Split`] keeps the one lifetime its
    /// callers already pass. The reader is boxed: it is most of this variant,
    /// and held inline it would make every other variant as large, a split
    /// allocating it once where a source moved by value copies it whole.
    Anchored { pattern: Pattern, reader: Box<Option<crate::prefilter::ByteReader<'h>>>, at: usize },
    /// A prefix of the input, lexed once and widened only when the matches it
    /// settled run out.
    ///
    /// For a pattern with no literal to anchor a window at and no anchored byte
    /// route: a split into four pieces of a pattern matching at the first token
    /// otherwise finds every match in the input to report three. Widening by
    /// doubling makes the whole walk a geometric series over the prefix sizes
    /// it actually needed, where asking a fresh prefix per piece is quadratic.
    Prefix {
        pattern: Pattern,
        settled: std::vec::IntoIter<Span>,
        covered: usize,
        /// The byte past which a match has not been handed out yet. A wider
        /// prefix re-reports everything a narrower one did, and this is what
        /// tells the new matches from the repeats - not where the narrow prefix
        /// ended, because a match starting inside it may have reached into the
        /// tokens that prefix held in reserve and so was never handed out.
        handed: usize,
    },
    /// The held walk, which the anchored source becomes where a route refuses
    /// part-way through.
    Walk(Box<crate::nfa::SerialWalk<crate::nfa::OwnedStream>>),
}

impl<'h> Separators<'h> {
    /// The next separator at or after wherever this source has reached.
    ///
    /// # Panics
    ///
    /// Never in practice: see the reasoning at the fallback below.
    fn next(&mut self, input: &'h [u8]) -> Option<Span> {
        // The refusal replaces the source it was read from, so the decision is
        // made inside the borrow and acted on outside it.
        let (pattern, from) = match self {
            Separators::Cursor(c) => return c.next(),
            Separators::Walk(w) => return w.next_span(input),
            Separators::Prefix { pattern, settled, covered, handed } => loop {
                if let Some(s) = settled.next() {
                    // An empty match would leave the offset where it is, so
                    // step past it as the walk does.
                    *handed = s.end().max(s.start() + 1);
                    return Some(s);
                }
                if *covered >= input.len() {
                    // The prefix reached the whole input and its matches are
                    // exhausted, so there are none left.
                    return None;
                }
                *covered = (*covered * 2).min(input.len());
                let Some((found, _)) =
                    crate::prefilter::settled_prefix_matches(pattern, input, *covered)
                else {
                    // The refusal is the same at every width, so this is the
                    // shape refusing rather than the widening: the walk below
                    // takes it from where this stopped.
                    break (pattern.clone(), *handed);
                };
                let fresh: Vec<Span> =
                    found.into_iter().filter(|s| s.start() >= *handed).collect();
                *settled = fresh.into_iter();
            },
            Separators::Anchored { pattern, reader, at } => {
                // The byte routes first, over the reader they share; then the
                // windows, which read the input only as far as the separator
                // they report. Both answer one ask at a time, and a refusal
                // from both is what the walk below is for.
                let answered = crate::engine::routed_first_at_reading(pattern, input, *at, reader)
                    .or_else(|| {
                        crate::prefilter::first_by_literal_windows_at(pattern, input, *at)
                    });
                match answered {
                    Some(found) => {
                        let Some(s) = found else {
                            // No match at or after the offset is a verdict over
                            // the rest of the input, so the offset goes to the
                            // end. Without that, a split of a pattern that is
                            // absent asks the route again for every piece and
                            // searches the whole input each time.
                            *at = input.len();
                            return None;
                        };
                        // An empty match would leave the offset where it is, so
                        // step past it as the walk does.
                        *at = s.end().max(s.start() + 1);
                        return Some(s);
                    }
                    // The bytes settled every offset before this one and cannot
                    // settle this one. Ending here would drop every separator
                    // after it, and a cursor's selection from zero is not the
                    // selection re-run from an offset, so neither will do.
                    None => (pattern.clone(), *at),
                }
            }
        };
        // The walk seeked to the offset is what find_at would give from here:
        // the same selection, re-run from the same place. It cannot decline,
        // because a pattern an anchored byte route answered is a literal, an
        // alternation of them, a line-anchored literal, a word then plain
        // punctuation, a byte pattern or a bare kind atom - and none of those
        // holds a balanced group, a field node or a named capture, which are
        // the only things SerialWalk::over turns away.
        let mut walk = crate::nfa::SerialWalk::over(&pattern, input)
            .expect("a pattern an anchored byte route answered compiles for this engine");
        walk.seek(from);
        let next = walk.next_span(input);
        *self = Separators::Walk(Box::new(walk));
        next
    }
}

pub struct Split<'h> {
    separators: Separators<'h>,
    input: &'h [u8],
    last: usize,
    /// Pieces this split may still yield. [`usize::MAX`] is no limit.
    left: usize,
    done: bool,
    /// The bracket depth a match must be at to separate, and the field that
    /// answers what depth a byte is at. `None` splits on every match.
    depth: Option<(crate::stress::StressField, u16)>,
}

impl<'h> Iterator for Split<'h> {
    type Item = &'h [u8];

    fn next(&mut self) -> Option<&'h [u8]> {
        if self.done || self.left == 0 {
            return None;
        }
        // The last piece a limit allows is the whole remainder, unsplit: that
        // is what makes `splitn(k)` yield `k` pieces rather than `k` pieces
        // and a silently discarded tail.
        if self.left == 1 {
            self.done = true;
            return Some(&self.input[self.last..]);
        }
        loop {
            match self.separators.next(self.input) {
                Some(s) => {
                    // A match at the wrong bracket depth is not a separator,
                    // so it stays inside the piece being built rather than
                    // ending it.
                    if let Some((field, want)) = &self.depth
                        && field.depth_at(s.start()) != *want
                    {
                        continue;
                    }
                    let piece = &self.input[self.last..s.start()];
                    self.last = s.end();
                    self.left = self.left.saturating_sub(1);
                    return Some(piece);
                }
                None => {
                    self.done = true;
                    return Some(&self.input[self.last..]);
                }
            }
        }
    }
}

/// `input` split on every match of `pattern`.
///
/// Every match is a separator, so the cursor is exhausted by definition and
/// takes [`Cursor::over_every_match`]: the resume a walk offers is worth nothing
/// to a caller that will ask for all of them, and it costs cores.
/// [`splitn`] keeps the walk, since a caller naming a limit may stop early.
pub fn split<'h>(pattern: &Pattern, input: &'h [u8]) -> Split<'h> {
    Split {
        separators: Separators::Cursor(Box::new(Cursor::over_every_match(pattern, input))),
        input,
        last: 0,
        left: usize::MAX,
        done: false,
        depth: None,
    }
}

/// [`split`] yielding at most `limit` pieces, the last being the unsplit
/// remainder. A `limit` of zero yields nothing.
/// The laziness is at construction and not only in the loop. A cursor finds
/// every match in the input before the first piece is handed over, which for a
/// limit of four is fifty thousand matches to report three. Where an anchored
/// byte route takes the pattern the separators are asked for one at a time
/// instead, over a reader held across them, so the quote scan runs once for the
/// whole split rather than from byte zero per piece.
/// The first prefix a split lexes when it has neither a route nor a window.
///
/// A sixty-fourth of a megabyte, which is what the widening prefix elsewhere in
/// the crate starts at, so a pattern matching early is settled by one lex of it
/// and a pattern matching late costs a doubling series rather than a fresh lex
/// an ask.
const PREFIX_FIRST_BYTES: usize = 64 * 1024;

pub fn splitn<'h>(pattern: &Pattern, input: &'h [u8], limit: usize) -> Split<'h> {
    // The ladder decides, not a second copy of its refusals: a route that
    // answers the first ask is asked again for each piece, and one that
    // declines at the outset declines for every offset. The answer here is
    // discarded rather than kept, which costs one route call and keeps the
    // source's state in one place.
    // The absent literal first, because routed_first_at_reading leaves it out:
    // it is a verdict over the whole input, and the n-gram filter settles it
    // without a search where the literal rung below would sweep every byte.
    let mut reader = None;
    // A split reports where the separators are and never what they bound, and
    // the routes are written against the shapes the language spells without
    // bindings, so the bare twin is what the source asks. Stripped once here
    // and carried: the source asks once a piece, and a pattern rebuilt per ask
    // would copy itself tens of thousands of times.
    let bare = pattern.without_bindings();
    let sought = bare.as_ref().unwrap_or(pattern);
    let separators = if crate::library::shapes_for(pattern).is_some() {
        crate::trace::rung("splitn", "a cursor over a lex under the library's shapes", input.len());
        Separators::Cursor(Box::new(Cursor::new(pattern, input)))
    } else if crate::prefilter::requires_absent(pattern, input) {
        crate::trace::rung("splitn", "a cursor over every match", input.len());
        Separators::Cursor(Box::new(Cursor::new(pattern, input)))
    } else if crate::engine::routed_first_at_reading(sought, input, 0, &mut reader).is_some()
        || crate::prefilter::first_by_literal_windows_at(sought, input, 0).is_some()
    {
        // Either the anchored byte routes or the stopping windows answer one
        // ask at a time, which is what a limited split wants; the source tries
        // both at each offset and becomes a walk where neither settles one.
        crate::trace::rung("splitn", "one separator at a time", input.len());
        Separators::Anchored { pattern: sought.clone(), reader: Box::new(reader), at: 0 }
    } else if let Some((settled, _)) =
        crate::prefilter::settled_prefix_matches(sought, input, PREFIX_FIRST_BYTES)
    {
        // No literal to window at and no anchored route, but a prefix settles
        // it: the separators come from one lexed prefix that widens only when
        // they run out.
        crate::trace::rung("splitn", "a prefix widened only when it runs out", input.len());
        Separators::Prefix {
            pattern: sought.clone(),
            settled: settled.into_iter(),
            covered: PREFIX_FIRST_BYTES.min(input.len()),
            handed: 0,
        }
    } else {
        crate::trace::rung("splitn", "a cursor over every match", input.len());
        Separators::Cursor(Box::new(Cursor::new(pattern, input)))
    };
    Split { separators, input, last: 0, left: limit, done: false, depth: None }
}

/// [`split`] separating only on matches that are `depth` brackets deep.
///
/// The operation a byte-level split cannot express. Splitting `f(a, g(b, c),
/// d)` on a comma gives five pieces over bytes, because the commas inside
/// `g(...)` look exactly like the ones outside it; a regular expression has no
/// way to tell them apart, since telling them apart requires counting brackets
/// and a regular language cannot count. At depth one it gives three, which is
/// the argument list.
///
/// A match at any other depth is not a separator and stays inside the piece
/// being built, so the pieces still rejoin to the input.
pub fn split_at_depth<'h>(pattern: &Pattern, input: &'h [u8], depth: u16) -> Split<'h> {
    Split {
        separators: Separators::Cursor(Box::new(Cursor::over_every_match(pattern, input))),
        input,
        last: 0,
        left: usize::MAX,
        done: false,
        depth: Some((crate::stress::analyze_bytes(input), depth)),
    }
}

/// `text` as a pattern matching exactly that text and nothing else.
///
/// The counterpart of the regex crate's `escape`, and not the same operation,
/// because the thing being escaped into is not the same. A regular expression
/// matches bytes, so making text inert there is a matter of putting a
/// backslash in front of the characters that would otherwise be syntax. A TREX
/// literal matches one token, so text that is several tokens cannot be one
/// literal at all, however it is quoted: `a+b` is a word, a punctuation mark
/// and a word, and a single literal atom asking for the three of them together
/// would never match anything.
///
/// So the text is lexed, and each significant token becomes its own quoted
/// literal in sequence. Escaping inside each is then the small part: within
/// `"..."` a backslash takes the next byte literally and an unescaped `"` ends
/// the literal, so those two are the only characters that need one.
///
/// Text that is empty or all whitespace escapes to the empty pattern, which
/// matches the empty token sequence.
#[must_use]
pub fn escape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 2);
    for t in crate::lexer::lex(bytes).iter().filter(|t| t.is_significant()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push('"');
        // Read through the bytes rather than slicing the string by the
        // lexer's offsets: the lexer works in bytes, and indexing a `str`
        // anywhere but a character boundary is a panic rather than an error.
        for c in String::from_utf8_lossy(&bytes[t.start()..t.end()]).chars() {
            if c == '\\' || c == '"' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
    }
    out
}

/// The names `pattern` binds, in the order it binds them, without repeats.
///
/// The regex crate's `capture_names` answers the same question over numbered
/// groups with optional names; every TREX binding is named, so there is no
/// unnamed slot to report and no index to report it under.
///
/// A name written only inside an assertion is not among them. An assertion
/// runs its sub-pattern as a filter and the probe's bindings are discarded, so
/// no match ever carries one, and a caller sizing a buffer by this would get a
/// slot that never fills.
#[must_use]
pub fn capture_names(pattern: &Pattern) -> Vec<String> {
    pattern.capture_names()
}

/// How many distinct names `pattern` binds.
#[must_use]
pub fn captures_len(pattern: &Pattern) -> usize {
    capture_names(pattern).len()
}


#[cfg(test)]
mod tests {
    use super::*;

    /// The borrowed view reports the match the owning iterator reports, over
    /// every source the cursor has.
    ///
    /// The two run different code: the owning form builds a match from each
    /// record, and the borrowed one points at the record. A view agreeing on
    /// the span but not on the registers, or reporting the names of a pattern
    /// in the wrong order against its spans, is a defect no caller of the
    /// owning form would ever see.
    ///
    /// The patterns reach different sources on purpose: one that binds nothing
    /// takes the plain cursor, and the binding ones take whichever rung the
    /// ladder gives them.
    #[test]
    fn the_borrowed_view_reports_what_the_owning_iterator_reports() {
        let mut text = String::new();
        for i in 0..400u32 {
            text.push_str(&format!("let value_{i} = {i} ; call_{i}(alpha, beta) ;\n"));
        }
        let input = text.as_bytes();
        for src in ["\"let\" \\W:v \"=\"", "\\W:w", "\"alpha\"", "\\W \"=\"", "\\N:n \";\""] {
            let p = crate::parse(src).expect("the test's own patterns parse");
            let owned: Vec<crate::engine::Match> = captures_iter(&p, input).collect();
            let mut cur = captures_iter(&p, input);
            let mut seen = 0usize;
            while let Some(m) = cur.next_ref() {
                let want = &owned[seen];
                assert_eq!((m.start, m.end), (want.start, want.end), "{src} span at {seen}");
                assert_eq!(m.captures(), want.captures(), "{src} registers at {seen}");
                assert_eq!(m.names(), want.names(), "{src} names at {seen}");
                assert_eq!(&m.to_match(), want, "{src} owned copy at {seen}");
                seen += 1;
            }
            assert_eq!(seen, owned.len(), "{src} reported a different number of matches");
        }
    }

    /// A split reports where the separators are, so a pattern that binds and
    /// its bare twin split identically - which is what lets the source ask the
    /// twin and reach the routes written against it.
    #[test]
    fn a_binding_separator_splits_where_its_bare_twin_splits() {
        let mut text = String::new();
        for i in 0..400u32 {
            text.push_str(&format!("let value_{i} = {i} ; call_{i}(alpha, beta) ;\n"));
        }
        let input = text.as_bytes();
        for (bound, bare) in [
            ("\\W:name \"=\"", "\\W \"=\""),
            ("\"let\" \\W:v \"=\"", "\"let\" \\W \"=\""),
            ("\\W:w", "\\W"),
        ] {
            let b = crate::parser::parse(bound).expect(bound);
            let u = crate::parser::parse(bare).expect(bare);
            for limit in [1usize, 2, 4, 50, usize::MAX] {
                let got: Vec<&[u8]> = splitn(&b, input, limit).collect();
                let want: Vec<&[u8]> = splitn(&u, input, limit).collect();
                assert_eq!(got, want, "{bound} against {bare}, limit {limit}");
            }
            let got: Vec<&[u8]> = split(&b, input).collect();
            let want: Vec<&[u8]> = split(&u, input).collect();
            assert_eq!(got, want, "{bound} against {bare}, unlimited");
        }
    }

    #[test]
    fn split_and_a_limitless_splitn_reach_the_same_pieces_by_different_routes() {
        // split takes the scan's spans together; splitn keeps the resumable
        // walk, because a caller naming a limit may stop early. They are now
        // different code and only a test says they agree, over every route the
        // cursor can take and an input that matches nothing.
        let mut text = String::new();
        for i in 0..3_000u32 {
            text.push_str(&format!("let value_{i} = {} ;\n", i * 7));
            text.push_str(&format!("call_{i}(alpha, beta, {i}) ;\n"));
            text.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n"));
        }
        for input in [text.as_bytes(), b"", b"nothing matches in here"] {
            for src in PATTERNS {
                let p = match crate::parse(src) {
                    Ok(p) => p,
                    Err(e) => panic!("{src} does not parse: {e:?}"),
                };
                let eager: Vec<&[u8]> = split(&p, input).collect();
                let walked: Vec<&[u8]> = splitn(&p, input, usize::MAX).collect();
                assert_eq!(eager, walked, "{src}");
                // The pieces rejoin to the input with the separators removed,
                // which is what makes a piece list a split rather than a list.
                assert!(
                    eager.iter().map(|p| p.len()).sum::<usize>() <= input.len(),
                    "{src}: the pieces outgrew the input"
                );
            }
        }
    }

    /// Patterns across every route the cursor can take: byte-routable
    /// literals, a word then punctuation, a byte pattern, a kind sequence,
    /// a windowed literal, the single-pass engine, and a balanced group that
    /// only the set engine advances.
    const PATTERNS: &[&str] = &[
        "\"alpha\"",
        "\"zzzqqq\"",
        "\\W",
        "\\N",
        "\\W \"=\"",
        "`cond_[0-9]+`",
        "(\"alpha\" | \"beta\")",
        "\"let\" \\W \"=\"",
        "\\W{2}",
        "^ \"let\"",
        "\\W:name \"=\"",
        "\\W:x \"=\" =x",
        "\\B(\\W)",
        "\\W \"=\" ~(\\N)",
    ];

    fn corpus() -> Vec<u8> {
        let mut s = String::new();
        for i in 0..400 {
            match i % 4 {
                0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
                1 => s.push_str(&format!("call_{i}(alpha, beta, {i}) ;\n")),
                2 => s.push_str(&format!("key_{i}: item_{i}, item_{} ;\n", i + 1)),
                _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
            }
        }
        s.into_bytes()
    }

    #[test]
    fn a_cursor_collects_to_what_the_scan_returns() {
        // The contract the lazy source is held to: the same spans, in the
        // same order. A cursor that stops early must not also select
        // differently, and only comparing the whole sequence shows that.
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            let want = crate::scan(&p, &input);
            let got: Vec<Span> = find_iter(&p, &input).collect();
            assert_eq!(got, want, "{src}");
        }
    }

    /// [`corpus`] with quoted strings through it, including a string holding an
    /// escaped quote and one written with single quotes.
    ///
    /// The corpus above holds no quote of either kind, so an ascending walk over
    /// it asks the quote scan only to report that there is nothing. A string
    /// opening between two asks is the other case, and the one a resumed scan
    /// can answer differently from a fresh one.
    fn quoted_corpus() -> Vec<u8> {
        let mut s = String::new();
        for i in 0..400 {
            match i % 5 {
                0 => s.push_str(&format!("let value_{i} = \"text {i}\" ;\n")),
                1 => s.push_str(&format!("call_{i}(alpha, \"beta {i}\", {i}) ;\n")),
                2 => s.push_str(&format!("key_{i}: \"a \\\"quoted\\\" {i}\" ;\n")),
                3 => s.push_str(&format!("note_{i} = 'c' ; other_{i} = {} ;\n", i * 37)),
                _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(\"x\") ; }}\n")),
            }
        }
        s.into_bytes()
    }

    /// A cursor holds its byte reader between matches and its quote scan reads
    /// quote bytes only as far as it has been asked, so what it carries from one
    /// ask to the next is state a fresh reader would not have. Over quoted text
    /// that state includes an open string, a closed one and an escaped quote
    /// inside one.
    ///
    /// The kind atoms are why this is asked over a whole pattern list rather
    /// than one pattern: a kind route probes at many candidate positions in
    /// ascending order, so it asks the quote scan far more often than a literal
    /// route, which asks only where a rare byte string occurs.
    #[test]
    fn a_cursor_collects_to_what_the_scan_returns_over_quoted_text() {
        let input = quoted_corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            let want = crate::scan(&p, &input);
            let got: Vec<Span> = find_iter(&p, &input).collect();
            assert_eq!(got, want, "{src}");
        }
    }

    #[test]
    fn find_is_the_first_match_the_scan_reports() {
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(find(&p, &input), crate::scan(&p, &input).first().copied(), "{src}");
        }
    }

    #[test]
    fn the_early_exit_reports_the_leftmost_match_not_the_first_literal_tried() {
        // The route reads each literal to its own first confirmation, so the
        // answer is the earliest of those. Taking whichever literal confirmed
        // first would report alpha at 11 over beta at 6, which is a different
        // match and not a slower way to the same one.
        let hay = b"gamma beta alpha gamma";
        let p = crate::parse("(\"alpha\" | \"beta\")").expect("pattern parses");
        assert_eq!(find(&p, hay).map(|s| s.start()), Some(6), "beta is leftmost");
        assert_eq!(find(&p, hay), crate::scan(&p, hay).first().copied());

        // And the same when the pattern names them the other way round, since
        // leftmost is a property of the input.
        let q = crate::parse("(\"beta\" | \"alpha\")").expect("pattern parses");
        assert_eq!(find(&q, hay), find(&p, hay));
    }

    #[test]
    fn the_widening_prefix_answers_what_the_whole_lex_answers() {
        // The two arms differ only in how much of the input is lexed, so they
        // must not differ in what they report. An input well past the first
        // prefix, so the widening actually runs more than one round.
        let mut input = corpus();
        while input.len() < 400_000 {
            let more = corpus();
            input.extend_from_slice(&more);
        }
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(find(&p, &input), find_by_full_lex(&p, &input), "{src}");
            assert_eq!(find(&p, &input), crate::scan(&p, &input).first().copied(), "{src}");
        }
    }

    #[test]
    fn a_match_only_at_the_end_is_still_found() {
        // The case the widening is worst at and must still get right: every
        // round before the last says nothing, and the last one lexes to the
        // end of the input.
        let mut input = corpus();
        while input.len() < 400_000 {
            let more = corpus();
            input.extend_from_slice(&more);
        }
        input.extend_from_slice(b"\nsentinel_token = 99 ;\n");
        let p = crate::parse("\"sentinel_token\" \"=\" \\N").expect("pattern parses");
        let want = crate::scan(&p, &input).first().copied();
        assert!(want.is_some(), "the sentinel must be there to be found");
        assert_eq!(find(&p, &input), want);
        assert_eq!(find_by_full_lex(&p, &input), want);
    }

    #[test]
    fn absence_over_a_widening_prefix_is_absence_over_the_input() {
        // A prefix that finds nothing proves nothing, so the widening has to
        // reach the end before it may answer no.
        let mut input = corpus();
        while input.len() < 400_000 {
            let more = corpus();
            input.extend_from_slice(&more);
        }
        for src in ["\"zzzqqq\"", "\"zzzqqq\" \"=\" \\N", "\\W \"@@\""] {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(find(&p, &input), None, "{src}");
            assert_eq!(crate::scan(&p, &input).first().copied(), None, "{src}");
        }
    }

    #[test]
    fn find_agrees_with_is_match_on_whether_there_is_one() {
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(find(&p, &input).is_some(), crate::is_match(&p, &input), "{src}");
        }
    }

    #[test]
    fn seeking_past_a_match_finds_the_next_one() {
        // Stepping the anchor to just past each match must walk the same
        // sequence the scan reports, which is what says the seek lands on a
        // token boundary and not inside one.
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            let want = crate::scan(&p, &input);
            let mut at = 0usize;
            for expected in &want {
                let got = find_at(&p, &input, at).expect("a match the scan found");
                assert_eq!(got, *expected, "{src} from {at}");
                at = got.end();
            }
            assert_eq!(find_at(&p, &input, at), None, "{src}: nothing past the last match");
        }
    }

    #[test]
    fn seeking_from_zero_is_finding_from_the_start() {
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(find_at(&p, &input, 0), find(&p, &input), "{src}");
            assert_eq!(is_match_at(&p, &input, 0), crate::is_match(&p, &input), "{src}");
        }
    }

    #[test]
    fn the_pieces_and_the_matches_rebuild_the_input() {
        // The property that says the split is right without asserting any
        // particular piece: pieces and matched text, interleaved in order,
        // are the input back. It catches an off-by-one at either edge of a
        // match, which comparing piece counts alone would not.
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            let spans = crate::scan(&p, &input);
            let pieces: Vec<&[u8]> = split(&p, &input).collect();
            assert_eq!(pieces.len(), spans.len() + 1, "{src}: one more piece than matches");
            let mut rebuilt: Vec<u8> = Vec::with_capacity(input.len());
            for (i, piece) in pieces.iter().enumerate() {
                rebuilt.extend_from_slice(piece);
                if let Some(s) = spans.get(i) {
                    rebuilt.extend_from_slice(&input[s.range()]);
                }
            }
            assert_eq!(rebuilt, input, "{src}");
        }
    }

    #[test]
    fn splitting_at_a_depth_ignores_the_separators_nested_deeper() {
        // The case a byte-level split cannot express: the commas inside
        // `g(...)` are the same bytes as the ones outside it, and telling
        // them apart needs counting, which a regular language cannot do.
        let input = b"f(a, g(b, c), d)" as &[u8];
        let comma = crate::parse("\",\"").expect("pattern parses");
        let flat: Vec<&[u8]> = split(&comma, input).collect();
        assert_eq!(flat.len(), 4, "every comma separates: {flat:?}");

        let args: Vec<&[u8]> = split_at_depth(&comma, input, 1).collect();
        assert_eq!(args.len(), 3, "only the top-level commas separate: {args:?}");
        assert_eq!(args[0], b"f(a");
        assert_eq!(args[1], b" g(b, c)", "the nested comma stayed inside its piece");
        assert_eq!(args[2], b" d)");

        // Whatever depth is asked for, the pieces and the separators that
        // were taken still rebuild the input.
        for d in 0..3u16 {
            let pieces: Vec<&[u8]> = split_at_depth(&comma, input, d).collect();
            let joined = pieces.join(b"," as &[u8]);
            assert_eq!(joined.len(), input.len(), "depth {d}: {pieces:?}");
        }
    }

    #[test]
    fn a_limited_split_keeps_the_rest_whole() {
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            let all: Vec<&[u8]> = split(&p, &input).collect();
            assert_eq!(splitn(&p, &input, 0).count(), 0, "{src}: a limit of zero yields nothing");
            for k in 1..=3.min(all.len()) {
                let some: Vec<&[u8]> = splitn(&p, &input, k).collect();
                assert_eq!(some.len(), k, "{src}: {k} pieces");
                assert_eq!(some[..k - 1], all[..k - 1], "{src}: the pieces before the last agree");
                // The last piece runs to the end of the input, whatever is in
                // it, which is what distinguishes a limit from a truncation.
                let tail = some[k - 1];
                assert_eq!(tail.as_ptr_range().end, input.as_ptr_range().end, "{src}");
            }
        }
    }

    #[test]
    fn rewriting_n_matches_rewrites_the_first_n() {
        let input = corpus();
        let names: Vec<String> = Vec::new();
        let tmpl = crate::Template::parse("X", &names).expect("the template parses");
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            let spans = crate::scan(&p, &input);
            // Past the match count, a partial rewrite is the whole rewrite.
            assert_eq!(
                crate::rewrite_n(&p, &tmpl, &input, spans.len()),
                crate::rewrite(&p, &tmpl, &input),
                "{src}"
            );
            assert_eq!(crate::rewrite_n(&p, &tmpl, &input, 0), input, "{src}: none is untouched");
            if let Some(first) = spans.first() {
                let once = crate::rewrite_first(&p, &tmpl, &input);
                assert_eq!(&once[..first.start()], &input[..first.start()], "{src}: before");
                assert_eq!(&once[first.start()..first.start() + 1], b"X", "{src}: the replacement");
                assert_eq!(&once[first.start() + 1..], &input[first.end()..], "{src}: after");
            }
        }
    }

    #[test]
    fn escaped_text_matches_exactly_itself() {
        // Each of these holds something the pattern language reads as syntax,
        // and each is more than one token, which is the case a byte-level
        // escape does not have to think about. Matched against the text
        // itself, so the surrounding bytes cannot change how it lexes.
        for text in ["alpha", "a+b", "x(y)", "let x = 1", "a\\b", "one|two", "p.q", "\\W"] {
            let pat = escape(text);
            let p = crate::parse(&pat).unwrap_or_else(|e| panic!("`{pat}` should parse: {e:?}"));
            let got = crate::find(&p, text.as_bytes());
            assert_eq!(
                got.map(|s| &text.as_bytes()[s.range()]),
                Some(text.as_bytes()),
                "`{pat}` over `{text}`"
            );
        }
    }

    #[test]
    fn escaped_text_parses_even_where_it_cannot_match() {
        // A lone quote opens a string the lexer never sees closed, and an
        // empty text has no token at all. Neither is a match question: the
        // contract here is that escaping never builds a pattern that fails
        // to parse, because a caller splices the result into a larger one.
        for text in ["\"", "\\", "", "   ", "'", "`"] {
            let pat = escape(text);
            crate::parse(&pat).unwrap_or_else(|e| panic!("`{pat}` from `{text}`: {e:?}"));
        }
    }

    #[test]
    fn escaping_text_that_is_not_ascii_does_not_panic() {
        // The lexer works in bytes and a `str` may only be indexed at a
        // character boundary, so reading a token out of the source by the
        // lexer's own offsets is a panic waiting for the first multi-byte
        // character. These carry two-, three- and four-byte ones.
        let texts =
            ["caf\u{e9}", "\u{3b8} = 1", "\u{4f60}\u{597d} world", "a \u{1f600} b", "na\u{ef}ve(x)"];
        for text in texts {
            let pat = escape(text);
            crate::parse(&pat).unwrap_or_else(|e| panic!("`{pat}` from `{text}`: {e:?}"));
        }
    }

    #[test]
    fn the_first_match_captures_the_same_over_a_prefix_as_over_the_whole_input() {
        // captures_first settles from a widening prefix and resolves the
        // registers over that prefix's tokens; the cursor resolves them over
        // a full lex. The two readings must agree, on an input well past the
        // first prefix so the two really do read different token streams.
        let mut input = corpus();
        while input.len() < 400_000 {
            let more = corpus();
            input.extend_from_slice(&more);
        }
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(captures_first(&p, &input), captures_iter(&p, &input).next(), "{src}");
        }
    }

    #[test]
    fn taking_captures_one_at_a_time_gives_what_taking_them_together_gives() {
        // The contract for the lazy form: same matches, same order, same
        // registers bound. Resolving from the walk's own save slots must not
        // differ from re-running an attempt at each span, which is what the
        // eager path does.
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            let spans = crate::scan(&p, &input);
            let want = crate::captures(&p, &input, &spans);
            let got: Vec<crate::engine::Match> = captures_iter(&p, &input).collect();
            assert_eq!(got, want, "{src}");
            assert_eq!(captures_first(&p, &input), want.first().cloned(), "{src}");
        }
    }

    #[test]
    fn the_names_a_pattern_binds_are_the_names_its_matches_carry() {
        // Introspection has to agree with what a match actually holds, or a
        // caller sizing a buffer from it is sized wrong.
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            let names = capture_names(&p);
            assert_eq!(captures_len(&p), names.len(), "{src}");
            let spans = crate::scan(&p, &input);
            for m in crate::captures(&p, &input, &spans) {
                let mut bound: Vec<&String> = m.names().iter().collect();
                bound.sort_unstable();
                let mut want: Vec<&String> = names.iter().collect();
                want.sort_unstable();
                assert_eq!(bound, want, "{src}: the match carries what the pattern binds");
            }
        }
    }

    #[test]
    fn a_matchs_token_extent_counts_the_tokens_it_spans() {
        // The two sources compute it differently - the walk subtracts the
        // indices it already ran between, a route indexes the significant
        // starts - so they are checked against one count neither of them
        // produced: the tokens of the whole input that are inside the span.
        let input = corpus();
        let all = crate::lexer::lex(&input);
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            let mut c = Cursor::new(&p, &input);
            let mut seen = 0usize;
            while let Some((s, extent)) = c.next_extent() {
                let want = all
                    .iter()
                    .filter(|t| t.is_significant() && t.start() >= s.start() && t.end() <= s.end())
                    .count();
                assert_eq!(extent, want, "{src} at {}..{}", s.start(), s.end());
                assert!(extent > 0, "{src}: a match spans at least one token");
                seen += 1;
            }
            assert_eq!(seen, crate::scan(&p, &input).len(), "{src}: every match was handed out");
        }
    }

    #[test]
    fn a_token_extent_is_not_a_byte_length() {
        // The point of counting in tokens: a token is not bounded in bytes,
        // so a run of several hundred characters is one of them and a span's
        // byte length says nothing about how many tokens it holds.
        let mut input = String::from("prefix = ");
        input.push_str(&"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789".repeat(8));
        input.push_str(" ;\n");
        let bytes = input.as_bytes();
        // `.` rather than `\W`: a long run with no whitespace in it reads as a
        // blob and not as a word, and which of the two it is does not bear on
        // the point. Either way it is one token.
        let p = crate::parse("\"prefix\" \"=\" .").expect("pattern parses");
        let mut c = Cursor::new(&p, bytes);
        let (s, extent) = c.next_extent().expect("the pattern matches this input");
        assert_eq!(extent, 3, "three tokens: the word, the equals and the run");
        assert!(
            s.end() - s.start() > 400,
            "three tokens spanning {} bytes",
            s.end() - s.start()
        );
    }

    #[test]
    fn the_shortest_end_never_reaches_past_the_first_match() {
        // The two answer the same question about whether there is a match,
        // and the shortest end is at or before the end of the match `find`
        // reports. It may be earlier, which is the whole point of having it,
        // so this bounds it rather than asserting equality.
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            let first = find(&p, &input);
            let end = shortest_match(&p, &input);
            assert_eq!(end.is_some(), first.is_some(), "{src}: they agree on whether");
            if let (Some(e), Some(f)) = (end, first) {
                assert!(e <= f.end(), "{src}: shortest end {e} past the first match's {}", f.end());
                assert!(e >= f.start(), "{src}: shortest end {e} before the match begins");
            }
        }
    }

    #[test]
    fn a_shorter_alternative_ends_sooner_than_the_preferred_match() {
        // Where a pattern has two ways to match from one place, `find`
        // reports the one the pattern prefers and this reports whichever ends
        // first. A greedy run of words prefers all of them; a match is known
        // to have occurred after the first.
        let input = b"alpha beta gamma delta ;" as &[u8];
        let p = crate::parse("\\W+").expect("pattern parses");
        let first = find(&p, input).expect("it matches");
        let end = shortest_match(&p, input).expect("it matches");
        assert_eq!(&input[first.range()], b"alpha beta gamma delta", "greedy takes all four");
        assert!(end < first.end(), "the soonest end is earlier: {end} against {}", first.end());
        assert_eq!(&input[..end], b"alpha", "and it is the end of the first word");
    }

    #[test]
    fn the_anchored_shortest_end_is_bounded_by_the_anchored_first_match() {
        // The anchored pair behaves as the unanchored pair does: they agree
        // on whether there is a match at or after the offset, and the end
        // reported is within the match `find_at` returns.
        let input = corpus();
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            for at in [0, 1, 37, input.len() / 3, input.len() / 2, input.len()] {
                let first = find_at(&p, &input, at);
                let end = shortest_match_at(&p, &input, at);
                assert_eq!(end.is_some(), first.is_some(), "{src} at {at}: agree on whether");
                if let (Some(e), Some(f)) = (end, first) {
                    assert!(e <= f.end(), "{src} at {at}: end {e} past the match's {}", f.end());
                    assert!(e >= f.start(), "{src} at {at}: end {e} before the match begins");
                }
            }
        }
    }

    #[test]
    fn a_limited_split_gives_what_an_unlimited_one_gives() {
        // splitn asks an anchored route for one separator at a time where
        // split takes the scan's spans together, so the two reach the same
        // pieces by different paths. The last input holds a quote the bytes
        // cannot settle partway through, which makes the route refuse there
        // and the source become a walk seeked to that offset - the transition
        // this is here to exercise rather than assume.
        for src in [
            "alpha beta alpha gamma alpha",
            "let a = 1 ; let b = 2 ; let c = 3 ;",
            "x alpha y alpha z",
            "no separator here at all",
            "alpha 'http://x/1' alpha beta alpha",
            "alpha \"q\" alpha 'http://y/2' alpha end",
        ] {
            let input = src.as_bytes();
            for pat in ["\"alpha\"", "^ \"let\"", "\\W \"=\"", "\\N"] {
                let p = crate::parse(pat).expect("pattern parses");
                let whole: Vec<&[u8]> = split(&p, input).collect();
                let unlimited: Vec<&[u8]> = splitn(&p, input, usize::MAX).collect();
                assert_eq!(unlimited, whole, "{pat} over {src:?}");
                // A limit takes a prefix of those pieces with the rest
                // unsplit, so every limit must agree with the whole split.
                for n in 1..=whole.len() + 1 {
                    let got: Vec<&[u8]> = splitn(&p, input, n).collect();
                    assert_eq!(got.len(), n.min(whole.len()), "{pat} over {src:?} at {n}");
                    for (i, piece) in got.iter().enumerate().take(got.len().saturating_sub(1)) {
                        assert_eq!(*piece, whole[i], "{pat} over {src:?} at {n}, piece {i}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_line_anchored_literal_stops_at_the_first_occurrence_that_leads_a_line() {
        // The route walks occurrences one at a time and stops at the first
        // that leads its line, where the spans route filters a whole set. Held
        // to the scan's first span over inputs where the earlier occurrences
        // are the rejected ones: indented leads a line, mid-line does not, and
        // an input whose every occurrence is mid-line has no match at all.
        let p = crate::parse("^ \"let\"").expect("pattern parses");
        for src in [
            "let a = 1 ;\nlet b = 2 ;\n",
            "x let a = 1 ;\n   let b = 2 ;\n",
            "x let a ;\ny let b ;\nz let c ;\n",
            "\n\n\tlet deep = 1 ;\n",
            "nothing here at all\n",
        ] {
            let input = src.as_bytes();
            let want = crate::scan(&p, input).first().copied();
            assert_eq!(find(&p, input), want, "{src:?}");
            assert_eq!(crate::is_match(&p, input), want.is_some(), "{src:?} is_match");
            // From an offset the answer is the first match at or after it, and
            // the occurrences the route steps over on the way are the ones
            // that do not lead a line.
            for at in 0..input.len() {
                let want_at = crate::scan(&p, input).into_iter().find(|s| s.start() >= at);
                assert_eq!(find_at(&p, input, at), want_at, "{src:?} from {at}");
            }
        }
    }

    #[test]
    fn the_quoted_route_names_the_token_the_lexer_makes() {
        // The quote scan applies the lexer's own char_literal_end and
        // single_quoted_end, so a span it reports is the token rather than a
        // guess at one. Held to the lexer directly over the shapes that decide
        // it: a lifetime and a contraction open nothing, a char literal holding
        // a double quote is one token, a doubled quote leaves one string, and a
        // quote a URL could have taken is refused rather than guessed - where
        // the engine then answers and must agree all the same.
        let p = crate::parse("\\Q").expect("pattern parses");
        for src in [
            "let s = \"hello world\" ;",
            "&'static T and don't stop",
            "c = '\"' ; d = 'x'",
            "q = 'foo''bar' ;",
            "the '90s and 5'10 tall",
            "u = 'http://x/1' ; v = \"w\"",
            "plain words and 12 numbers",
            "'n dag 'n beer met 'n karakter",
            "x = \"a\\\"b\" ; y = 2",
            // A string no quote closes runs to the input's end, where the scan
            // carries that length in place of a closing quote's index.
            "s = \"unterminated",
            "t = 'also unterminated",
        ] {
            let input = src.as_bytes();
            let want = crate::lexer::lex(input)
                .iter()
                .find(|t| t.kind == crate::token::TokenKind::Quoted)
                .map(|t| (t.start(), t.end()));
            assert_eq!(find(&p, input).map(|s| (s.start(), s.end())), want, "{src}");
            // From an offset, the token reported must be the first the lexer
            // makes that begins at or after it - a quote inside a string that
            // opened earlier names that string and not a token of its own.
            for at in 0..input.len() {
                let want_at = crate::lexer::lex(input)
                    .iter()
                    .find(|t| t.kind == crate::token::TokenKind::Quoted && t.start() >= at)
                    .map(|t| (t.start(), t.end()));
                let got = find_at(&p, input, at).map(|s| (s.start(), s.end()));
                assert_eq!(got, want_at, "{src} from {at}");
            }
        }
    }

    #[test]
    fn the_anchored_byte_pattern_route_agrees_with_the_walk() {
        // A byte pattern's match opens with its literal prefix, so bounding the
        // search at the offset is enough and the route needs no span filter.
        // Held to the walk at every offset the corpus reaches.
        let input = corpus();
        let p = crate::parse("`cond_[0-9]+`").expect("pattern parses");
        let mut compared = 0;
        for at in [0, 1, 37, input.len() / 3, input.len() / 2, input.len()] {
            let Some(routed) = crate::engine::routed_first_at(&p, &input, at) else {
                continue;
            };
            let Some(mut w) = crate::nfa::SerialWalk::over(&p, &input) else {
                continue;
            };
            w.seek(at);
            assert_eq!(routed, w.next_span(&input), "the byte pattern from {at}");
            compared += 1;
        }
        assert!(compared > 0, "the route never applied, so nothing was compared");
        // An input the route certainly matches in, so the agreement above is
        // not carried by a shared verdict of no match.
        let small = b"x cond_1 y cond_22 z cond_333" as &[u8];
        for at in [0, 1, 2, 9, 10, 19, 29] {
            let routed =
                crate::engine::routed_first_at(&p, small, at).expect("the route takes it");
            let mut w = crate::nfa::SerialWalk::over(&p, small).expect("the engine takes it");
            w.seek(at);
            assert_eq!(routed, w.next_span(small), "the byte pattern from {at} of the small input");
        }
        assert!(
            crate::engine::routed_first_at(&p, small, 0).expect("the route takes it").is_some(),
            "the small input must hold a match"
        );
    }

    #[test]
    fn the_anchored_word_then_punct_route_skips_a_word_that_began_before_the_offset() {
        // The route searches for the punctuation, and the word behind an
        // occurrence may begin before the offset. "alpha =" starts at 0, so an
        // ask from 1 must report "beta =" at 12 and not "alpha =" - the search
        // cannot be bounded at the offset, so the span is filtered instead.
        let input = b"alpha = 1 ; beta = 2 ; gamma = 3" as &[u8];
        let p = crate::parse("\\W \"=\"").expect("pattern parses");
        for (at, want) in [(0, (0, 7)), (1, (12, 18)), (12, (12, 18)), (13, (23, 30))] {
            let mut w = crate::nfa::SerialWalk::over(&p, input).expect("the engine takes it");
            w.seek(at);
            let walked = w.next_span(input);
            assert_eq!(
                walked.map(|s| (s.start(), s.end())),
                Some(want),
                "the walk from {at}"
            );
            assert_eq!(find_at(&p, input, at), walked, "the ladder from {at}");
        }
    }

    #[test]
    fn the_anchored_prefix_reruns_the_selection_rather_than_filtering_it() {
        // The leftmost, non-overlapping selection from an offset can hold a
        // match that overlaps one the selection from zero preferred. Over
        // "a b c d ..." with \W{2} the selection from zero is [a b] then
        // [c d]; asked from b, the answer is [b c], which no filter of that
        // selection contains. Both the walk and the prefix must give [b c].
        let input = b"a b c d e f g h" as &[u8];
        let p = crate::parse("\\W{2}").expect("pattern parses");
        let at = 2;
        let mut w = crate::nfa::SerialWalk::over(&p, input).expect("the engine takes it");
        w.seek(at);
        let by_walk = w.next_span(input);
        assert_eq!(
            by_walk.map(|s| (s.start(), s.end())),
            Some((2, 5)),
            "the walk re-runs the selection from the offset"
        );
        assert_eq!(find_at(&p, input, at), by_walk, "and the ladder gives the same");
    }

    #[test]
    fn the_anchored_prefix_settles_what_the_whole_input_settles() {
        // The prefix is only allowed to read less if it decides the same
        // thing, and the thing it must decide is the walk's answer from the
        // offset, not the whole scan's filtered.
        let input = corpus();
        let mut compared = 0;
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            for at in [0, 1, 37, input.len() / 3, input.len() / 2, input.len()] {
                let Some(from_prefix) = crate::prefilter::find_at_by_growing_prefix(&p, &input, at)
                else {
                    continue;
                };
                let Some(mut w) = crate::nfa::SerialWalk::over(&p, &input) else {
                    continue;
                };
                w.seek(at);
                assert_eq!(from_prefix, w.next_span(&input), "{src} at {at}: prefix against walk");
                compared += 1;
            }
        }
        assert!(compared > 0, "no pattern reached the prefix, so nothing was compared");
    }

    #[test]
    fn the_prefix_settles_the_same_shortest_end_the_whole_input_does() {
        // The prefix is only allowed to read less if it decides the same
        // thing. Where it answers at all - an end, or a proof that there is
        // none - that answer must be the whole input's.
        let input = corpus();
        let mut compared = 0;
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            for at in [0, 1, 37, input.len() / 3, input.len() / 2, input.len()] {
                let Some(from_prefix) =
                    crate::prefilter::shortest_end_by_growing_prefix_from(&p, &input, at)
                else {
                    continue;
                };
                let whole = crate::nfa::shortest_end_from_byte(&p, &input, at);
                assert_eq!(from_prefix, whole, "{src} at {at}: prefix and whole input differ");
                compared += 1;
            }
        }
        assert!(compared > 0, "no pattern reached the prefix, so nothing was compared");
    }

    #[test]
    fn an_anchor_holds_against_the_input_and_not_against_the_offset() {
        // `at` selects where a match may begin and never what the input is, so
        // the stream read is the whole one from `at` rather than the input cut
        // there. `\A` therefore holds only at the input's own first token, and
        // an ask from past it has no match - which is what `find_at` answers.
        let input = b"alpha beta gamma delta ;" as &[u8];
        let p = crate::parse("\\A \\W").expect("pattern parses");
        assert_eq!(shortest_match_at(&p, input, 0), Some(5), "at the start it matches");
        assert_eq!(find_at(&p, input, 6), None, "find_at reads the anchor this way");
        assert_eq!(shortest_match_at(&p, input, 6), None, "and so does this");
    }

    #[test]
    fn an_empty_input_has_no_match_to_take() {
        for src in PATTERNS {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(find(&p, b""), None, "{src}");
            assert_eq!(find_iter(&p, b"").count(), 0, "{src}");
            assert_eq!(find_at(&p, b"", 0), None, "{src}");
        }
    }
}
