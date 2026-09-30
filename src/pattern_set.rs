//! Many patterns asked of one input, over one lex.
//!
//! The regex crate's `RegexSet` answers which of its patterns match in a
//! single pass, and the saving there is that one automaton carries all of
//! them: the input is read once instead of once per pattern.
//!
//! Over tokens the saving is larger and comes from somewhere else. Matching
//! here is a lex and then a walk, the lex runs at a pattern-independent rate
//! and dominates, and the walk is the cheap half. So a set does not need one
//! automaton to win - it needs one lex. Twenty patterns over a shared token
//! stream cost one lex and twenty walks, where twenty separate calls cost
//! twenty of each.
//!
//! A pattern that a byte route answers costs neither: those routes read the
//! input's bytes and never reach the lexer, so a set whose patterns are all
//! byte-routable never lexes at all, and one with a mix lexes once for the
//! rest.
//!
//! The lex the rest share is a prefix that widens, not the whole input. That
//! is the difference between a set being worth having and being worse than
//! not having one: a member asked on its own settles from a prefix, so a set
//! that lexes everything up front would lose to the same members asked one at
//! a time. Members drop out of the widening as they are settled, so the
//! prefix reached is the one the hardest member needed rather than the sum of
//! what each needed.

use crate::ast::Pattern;
use crate::token::Token;

/// A set of patterns, asked together.
pub struct PatternSet {
    pats: Vec<Pattern>,
    /// The members' names, one each, where the set was built from a pattern
    /// file: a `let` under its name, a bare line under its line number.
    /// Empty for a set built from patterns alone, whose members go by index.
    names: Vec<String>,
    /// The shapes and kinds a pattern file declared for the members, which
    /// a member naming one is lexed under; empty for a set built from
    /// patterns alone.
    shapes: crate::custom::ShapeSet,
    /// Whether the members the single-pass engine takes are walked as one
    /// program over the shared lex, or each as itself.
    as_one: bool,
    /// Whether an input's literals are probed once for every member through
    /// a filter, or searched for once per member.
    probed: bool,
    /// Those members as one program, built on first use.
    together: std::sync::OnceLock<Together>,
}

impl std::fmt::Debug for PatternSet {
    /// The members and their names; the union built over them is not shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatternSet").field("pats", &self.pats).field("names", &self.names).finish_non_exhaustive()
    }
}

impl Clone for PatternSet {
    /// The same members, names and declarations; the union is built again
    /// on first use.
    fn clone(&self) -> Self {
        PatternSet {
            pats: self.pats.clone(),
            names: self.names.clone(),
            shapes: self.shapes.clone(),
            as_one: self.as_one,
            probed: self.probed,
            together: std::sync::OnceLock::new(),
        }
    }
}

/// The members walked as one: their union, and each set index's place in
/// it, `None` for a member walked as itself.
struct Together {
    union: Option<crate::nfa::Union>,
    place: Vec<Option<u32>>,
    /// How many literals the members require between them, which decides
    /// whether an input is filtered once or searched once per literal.
    required_literals: usize,
}

/// How a member of a set was read, which is how its matches' registers are
/// resolved: over the bytes a route read, over the lex the members shared,
/// or over a lex of its own under its library kinds.
enum Lexed<'t> {
    Bytes,
    Shared(&'t [crate::token::Token]),
    Own,
}

/// What one input answers for every member at once: a filter over its
/// n-grams, built once, that says which literals are absent with no search.
/// Absent when the members require few literals, which one search each
/// answers for less than the filter costs to build.
struct Probe {
    filter: Option<crate::prefilter::BloomFilter>,
}

impl Probe {
    /// Whether `lits` are all absent from the input by the filter alone.
    fn refuses_all(&self, lits: &[&str]) -> bool {
        self.filter.as_ref().is_some_and(|f| crate::prefilter::all_absent_by(f, lits))
    }
}

/// Which patterns of a set matched, one verdict per index.
///
/// The counterpart of the regex crate's `SetMatches`. Every index carries a
/// verdict, so asking about one pattern is a lookup rather than a search
/// through the list [`PatternSet::matches`] returns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetMatches {
    bits: Vec<bool>,
}

impl SetMatches {
    /// Whether the pattern at `i` matched. An index past the set reads false.
    #[must_use]
    pub fn matched(&self, i: usize) -> bool {
        self.bits.get(i).copied().unwrap_or(false)
    }

    /// Whether any pattern matched.
    #[must_use]
    pub fn matched_any(&self) -> bool {
        self.bits.iter().any(|b| *b)
    }

    /// Whether every pattern matched.
    #[must_use]
    pub fn matched_all(&self) -> bool {
        self.bits.iter().all(|b| *b)
    }

    /// How many patterns the set held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bits.len()
    }

    /// Whether the set held none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bits.is_empty()
    }

    /// The indices that matched, in order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.bits.iter().enumerate().filter_map(|(i, b)| b.then_some(i))
    }
}

/// How one member of a set is to be answered.
enum Plan {
    /// A byte route settled it without reaching the lexer.
    Settled(bool),
    /// It needs a lex of its own: an anchor no prefix can settle, or a
    /// pattern the single-pass engine does not take.
    Alone,
    /// It can be walked over the prefix the rest of the set shares, with the
    /// program already compiled and the token bound a match cannot exceed.
    Shared(crate::nfa::Compiled, usize),
    /// It is walked over the shared prefix as a member of the union, at this
    /// place in it, with the token bound a match cannot exceed.
    Together(u32, usize),
}

impl PatternSet {
    /// A set over `pats`, in the order given. That order is the index every
    /// answer is reported under.
    #[must_use]
    pub fn new(pats: Vec<Pattern>) -> Self {
        PatternSet {
            pats,
            names: Vec::new(),
            shapes: crate::custom::ShapeSet::new(),
            as_one: true,
            probed: false,
            together: std::sync::OnceLock::new(),
        }
    }

    /// A set over `pats` with a name each, in the order given; a name short
    /// of the count is the member's index as text.
    #[must_use]
    pub fn named(pats: Vec<Pattern>, mut names: Vec<String>) -> Self {
        let count = pats.len();
        names.truncate(count);
        while names.len() < count {
            names.push(names.len().to_string());
        }
        let mut set = PatternSet::new(pats);
        set.names = names;
        set
    }

    /// The members a pattern file declares, into `shapes`: a `let NAME =
    /// PATTERN` line is a member under its name, and any line that is not a
    /// declaration is a member under its line number, parsed against the
    /// declarations so far; `kind`, `shape`, `shape-after` and `test` lines
    /// are declared as `ShapeSet::declare_text` declares them, so the file's
    /// kinds and shapes serve the members and `lib --test` checks them.
    /// Every member is lexed under the file's declarations, as a scan under
    /// `--lib` is.
    ///
    /// # Errors
    ///
    /// The first line that is neither a declaration nor a pattern, or whose
    /// declaration is refused, with its line number in the message.
    pub fn from_text(
        text: &str,
        shapes: &mut crate::custom::ShapeSet,
    ) -> Result<PatternSet, crate::custom::ShapeError> {
        let mut members = Vec::new();
        shapes.declare_lines(text, Some(&mut members))?;
        Ok(PatternSet::of_members(members, shapes))
    }

    /// [`Self::from_text`] over the pattern file at `path`, a relative
    /// `@file` set in it read from beside the file.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or a line of it is refused.
    pub fn from_file(
        path: &std::path::Path,
        shapes: &mut crate::custom::ShapeSet,
    ) -> Result<PatternSet, crate::custom::ShapeError> {
        let members = shapes.declare_file_members(path)?;
        Ok(PatternSet::of_members(members, shapes))
    }

    /// A set over named members, lexed under `shapes`.
    fn of_members(members: Vec<(String, Pattern)>, shapes: &crate::custom::ShapeSet) -> PatternSet {
        let (names, pats): (Vec<String>, Vec<Pattern>) = members.into_iter().unzip();
        PatternSet::named(pats, names).under(shapes.clone())
    }

    /// This set with its members lexed under `shapes`, as a set built from a
    /// pattern file is lexed under the file's declarations.
    #[must_use]
    pub fn under(mut self, shapes: crate::custom::ShapeSet) -> Self {
        self.shapes = shapes;
        self
    }

    /// The members' names, one each, where the set was built from a pattern
    /// file, and nothing where it was built from patterns alone.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The name the member at `i` goes by: its name where the set has them,
    /// else its index as text.
    #[must_use]
    pub fn name(&self, i: usize) -> String {
        self.names.get(i).cloned().unwrap_or_else(|| i.to_string())
    }

    /// The declarations the members are lexed under.
    #[must_use]
    pub fn shapes(&self) -> &crate::custom::ShapeSet {
        &self.shapes
    }

    /// Every match of every member: each member's leftmost, non-overlapping
    /// matches over `input`, tagged with the member's index and ordered by
    /// position, then by member. The members a byte route answers never
    /// reach the lexer; the rest share one lex and each is walked over it as
    /// itself, so every span keeps the member that made it. A set built from
    /// a pattern file lexes every member under the file's declarations,
    /// since a shape decides boundaries a byte route never sees.
    #[must_use]
    pub fn scan(&self, input: &[u8]) -> Vec<(usize, crate::engine::Span)> {
        self.scan_from(input, 0)
    }

    /// [`Self::scan`] from the first token starting at or after byte `at`,
    /// each member's leftmost, non-overlapping selection re-run from there.
    #[must_use]
    pub fn scan_from(&self, input: &[u8], at: usize) -> Vec<(usize, crate::engine::Span)> {
        let mut out: Vec<(usize, crate::engine::Span)> = Vec::new();
        self.each_member(input, at, false, None, |i, spans, _| {
            out.extend(spans.into_iter().map(|s| (i, s)));
        });
        out.sort_unstable_by_key(|&(i, s)| (s.start(), s.end(), i));
        out
    }

    /// [`Self::scan_from`] over a lex of `input` the caller already holds.
    ///
    /// A streaming push lexes its retained buffer before it scans, and a set
    /// scanning the same bytes would otherwise lex them again - measured at
    /// 7.3 MB in 57 pushes, the stream's lex ran 57 times and the set's 56
    /// more over the same buffer. Lending the tokens removes the second.
    ///
    /// `toks` must be a lex of the whole of `input` taken under no declared
    /// shapes, which is what [`crate::lexer::lex_into`] gives. A set that
    /// declares shapes ignores them and lexes under its own, since those
    /// decide boundaries the caller's lex never saw.
    #[must_use]
    pub fn scan_from_over(
        &self,
        input: &[u8],
        at: usize,
        toks: &[Token],
    ) -> Vec<(usize, crate::engine::Span)> {
        let mut out: Vec<(usize, crate::engine::Span)> = Vec::new();
        self.each_member(input, at, false, Some(toks), |i, spans, _| {
            out.extend(spans.into_iter().map(|s| (i, s)));
        });
        out.sort_unstable_by_key(|&(i, s)| (s.start(), s.end(), i));
        out
    }

    /// [`Self::scan`] with each match's registers resolved under its own
    /// member's names, over the lex the member was found on where it shared
    /// one, so a set of binding members pays no lex beyond the scan's; under
    /// `lists`, with every binding a register made under a repetition, as
    /// [`crate::captures_with_lists`] resolves them.
    #[must_use]
    pub fn scan_matches(&self, input: &[u8], lists: bool) -> Vec<(usize, crate::engine::Match)> {
        self.resolved(input, false, lists)
    }

    /// Each member's first match over `input`, with its registers resolved
    /// as [`Self::scan_matches`] resolves them, ordered by position, then by
    /// member. A member stops at its first match and the lexer stops with
    /// it: a byte route reads no token, a member that lexes alone reads
    /// through a cursor that lexes as it goes, and the rest share one prefix
    /// that widens only while a member is still open. A set built from a
    /// pattern file with declarations lexes every member under them.
    #[must_use]
    pub fn first_matches(&self, input: &[u8], lists: bool) -> Vec<(usize, crate::engine::Match)> {
        let shaped = !self.shapes.is_empty();
        let spans = if shaped { self.first_spans_shaped(input, 0, false) } else { self.first_spans(input) };
        let mut out = Vec::with_capacity(spans.len());
        for (i, s) in spans {
            let p = &self.pats[i];
            let resolved = match (lists, shaped) {
                (true, false) => crate::engine::captures_with_lists(p, input, &[s]),
                (false, false) => crate::engine::captures(p, input, &[s]),
                (true, true) => crate::engine::captures_with_shapes_and_lists(p, input, &self.shapes, &[s]),
                (false, true) => crate::engine::captures_with_shapes(p, input, &self.shapes, &[s]),
            };
            out.extend(resolved.into_iter().map(|m| (i, m)));
        }
        out.sort_by_key(|(i, m)| (m.start, m.end, *i));
        out
    }

    /// Each member's first match, found as [`Self::matches`] finds whether
    /// there is one: a route over the bytes where one answers, a cursor that
    /// lexes as it goes for a member that lexes alone, and one widening
    /// prefix for the rest, so no member reads past what its first match
    /// needs.
    fn first_spans(&self, input: &[u8]) -> Vec<(usize, crate::engine::Span)> {
        let mut out = Vec::new();
        let mut shared = Vec::new();
        let mut together = Vec::new();
        let probe = self.probe(input);
        for (i, p) in self.pats.iter().enumerate() {
            match self.plan(i, p, input, &probe) {
                Plan::Settled(false) => {}
                // A route said there is a match: the route that reports
                // where answers, and the cursor where none of them does.
                Plan::Settled(true) => {
                    let first = match crate::engine::routed_first(p, input) {
                        Some(first) => first,
                        None => crate::cursor::find(p, input),
                    };
                    if let Some(s) = first {
                        out.push((i, s));
                    }
                }
                Plan::Alone => {
                    if let Some(s) = crate::cursor::find(p, input) {
                        out.push((i, s));
                    }
                }
                Plan::Shared(c, max_len) => shared.push((i, c, max_len)),
                Plan::Together(j, max_len) => together.push((i, j, max_len)),
            }
        }
        out.extend(self.over_a_shared_prefix(input, &shared, &together, false));
        out
    }

    /// The matches of every member, or its first alone under `first`, each
    /// resolved over the lex it was found on.
    fn resolved(&self, input: &[u8], first: bool, lists: bool) -> Vec<(usize, crate::engine::Match)> {
        let mut out: Vec<(usize, crate::engine::Match)> = Vec::new();
        self.each_member(input, 0, first, None, |i, spans, lexed| {
            let p = &self.pats[i];
            let matches = match lexed {
                Lexed::Bytes if lists => crate::engine::captures_with_lists(p, input, &spans),
                Lexed::Bytes => crate::engine::captures(p, input, &spans),
                Lexed::Shared(toks) if lists => crate::engine::captures_over_with_lists(p, input, toks, &spans),
                Lexed::Shared(toks) => crate::engine::captures_over(p, input, toks, &spans),
                Lexed::Own if lists => crate::engine::captures_with_shapes_and_lists(p, input, &self.shapes, &spans),
                Lexed::Own => crate::engine::captures_with_shapes(p, input, &self.shapes, &spans),
            };
            out.extend(matches.into_iter().map(|m| (i, m)));
        });
        out.sort_by_key(|(i, m)| (m.start, m.end, *i));
        out
    }

    /// Hand `emit` each member's matches at or after byte `at`, or its
    /// first alone under `first`, with how the member was read: the members
    /// a byte route answers never reach the lexer, the rest share one lex
    /// and each is walked over it as itself, and a member naming a library
    /// kind takes a lex of its own under that kind's shapes. Under the
    /// declared shapes of a set built from a pattern file every member is
    /// lexed, once, since a shape decides boundaries a byte route never
    /// sees and a literal's absence from the bytes settles nothing it could
    /// have fused.
    fn each_member<F>(
        &self,
        input: &[u8],
        at: usize,
        first: bool,
        reuse: Option<&[Token]>,
        mut emit: F,
    ) where
        F: FnMut(usize, Vec<crate::engine::Span>, Lexed<'_>),
    {
        let shaped = !self.shapes.is_empty();
        let mut shared = Vec::new();
        let mut own = Vec::new();
        let probe = self.probe(input);
        for (i, p) in self.pats.iter().enumerate() {
            if !p.library_kinds().is_empty() {
                own.push(i);
                continue;
            }
            if shaped {
                shared.push(i);
                continue;
            }
            if crate::prefilter::requires_absent_with(p, input, probe.filter.as_ref()) {
                continue;
            }
            // From the start any route answers; from a later position only
            // the routes whose matches cannot overlap may be cut at it.
            let routed = if first {
                crate::engine::routed_first(p, input).map(|s| s.into_iter().collect())
            } else if at == 0 {
                crate::engine::routed_spans(p, input)
            } else {
                crate::engine::routed_spans_positional(p, input)
            };
            match routed {
                Some(spans) => emit(i, spans.into_iter().filter(|s| s.start() >= at).collect(), Lexed::Bytes),
                None => shared.push(i),
            }
        }
        if !shared.is_empty() {
            // One lex for every member that needs one, read by each rather
            // than copied to it.
            // A caller holding a lex of these same bytes can lend it, and a
            // streaming set is handed the one its push already took. The
            // offer is refused where this set declares shapes, because those
            // decide boundaries the caller's lex never saw, and a set must
            // read its members under its own declarations whatever it is
            // given.
            let lexed;
            let toks: &[Token] = match (shaped, reuse) {
                (true, _) => {
                    let blobs = crate::lexer::blob_runs(input);
                    lexed = crate::lexer::lex_with_shapes(input, &blobs, &self.shapes, 0);
                    &lexed
                }
                (false, Some(lent)) => lent,
                (false, None) => {
                    lexed = crate::parallel_lex::lex_parallel(input);
                    &lexed
                }
            };
            let start = toks.partition_point(|t| t.start() < at);
            let sig = crate::nfa::Stitched::significant_of(toks);
            for i in shared {
                let p = &self.pats[i];
                let stream = crate::nfa::Stitched::new(toks, &sig);
                let spans = if let Some(mut w) = crate::nfa::SerialWalk::over_stream(p, input, stream) {
                    w.seek(at);
                    let mut spans = Vec::new();
                    while let Some(s) = w.next_span(input) {
                        spans.push(s);
                        if first {
                            break;
                        }
                    }
                    spans
                } else {
                    let mut spans = crate::engine::scan_tokens_from(p, input, toks, start);
                    if first {
                        spans.truncate(1);
                    }
                    spans
                };
                emit(i, spans, Lexed::Shared(toks));
            }
        }
        for i in own {
            let mut spans = crate::engine::scan_with_shapes_from(&self.pats[i], input, &self.shapes, at);
            if first {
                spans.truncate(1);
            }
            emit(i, spans, Lexed::Own);
        }
    }

    /// Each member's first match at or after byte `at` under the declared
    /// shapes of a set built from a pattern file, over one lex under them,
    /// in member order; the first of them alone under `any`.
    fn first_spans_shaped(&self, input: &[u8], at: usize, any: bool) -> Vec<(usize, crate::engine::Span)> {
        let mut out = Vec::new();
        self.each_member(input, at, true, None, |i, spans, _| {
            if let Some(&s) = spans.first() {
                out.push((i, s));
            }
        });
        out.sort_unstable_by_key(|&(i, _)| i);
        if any {
            out.truncate(1);
        }
        out
    }

    /// The most tokens a match of any member can span, for a set every
    /// member of which is bounded: what a stream over the set commits under.
    #[must_use]
    pub fn max_tokens(&self) -> Option<usize> {
        self.pats.iter().map(Pattern::max_tokens).try_fold(0usize, |best, m| m.map(|m| best.max(m)))
    }

    /// Whether any member depends on input outside a single match span, so a
    /// stream over the set commits nothing before its end.
    #[must_use]
    pub fn depends_on_whole_input(&self) -> bool {
        self.pats.iter().any(Pattern::depends_on_whole_input)
    }

    /// Whether any member depends on input beyond the lines its match spans,
    /// so a stream over the set, which cuts only just after a newline,
    /// commits nothing before its end.
    #[must_use]
    pub fn depends_on_more_than_its_lines(&self) -> bool {
        self.pats.iter().any(Pattern::depends_on_more_than_its_lines)
    }

    /// Whether any member reads whitespace, so a stream over the set cannot
    /// take a line's end as a boundary.
    #[must_use]
    pub fn reads_whitespace(&self) -> bool {
        self.pats.iter().any(Pattern::reads_whitespace)
    }

    /// Whether the literals the members require are probed once per input
    /// through a filter over its n-grams, or searched for once per member,
    /// which is how a set is built: measured on a thousand members over
    /// 2 MiB, the filter cost 40 ms more than the searches when the literals
    /// were present and saved 13 ms when they were absent. The answers are
    /// the same either way; a caller whose lists are mostly absent turns the
    /// probe on.
    #[must_use]
    pub fn probed(mut self, on: bool) -> Self {
        self.probed = on;
        self
    }

    /// Whether the members the single-pass engine takes are walked as one
    /// program over the shared lex, which is how a set is built, or each as
    /// itself. The answers are the same either way; this is the switch the
    /// two forms are timed against each other through.
    #[must_use]
    pub fn walked_as_one(mut self, on: bool) -> Self {
        self.as_one = on;
        self.together = std::sync::OnceLock::new();
        self
    }

    /// This set with the members' names replaced.
    #[must_use]
    pub fn with_names(mut self, names: Vec<String>) -> Self {
        let count = self.pats.len();
        self.names = names;
        self.names.truncate(count);
        while self.names.len() < count {
            self.names.push(self.names.len().to_string());
        }
        self
    }

    /// The union of the members it takes, built once.
    fn together(&self) -> &Together {
        self.together.get_or_init(|| {
            let mut place = vec![None; self.pats.len()];
            let mut members: Vec<&Pattern> = Vec::new();
            if self.as_one {
                for (i, p) in self.pats.iter().enumerate() {
                    if crate::nfa::union_eligible(p) {
                        place[i] = Some(u32::try_from(members.len()).expect("a set holds fewer than four billion patterns"));
                        members.push(p);
                    }
                }
            }
            let union = (!members.is_empty()).then(|| crate::nfa::Union::of(&members));
            let required_literals =
                self.pats.iter().map(crate::prefilter::required_literal_count).sum();
            Together { union, place, required_literals }
        })
    }

    /// The probe of `input` the members share: a filter over the input's
    /// n-grams where the members require more literals than one search each
    /// is worth, and nothing otherwise.
    fn probe(&self, input: &[u8]) -> Probe {
        let many = self.probed
            && self.together().required_literals > crate::prefilter::direct_search_max_literals();
        Probe { filter: many.then(|| crate::prefilter::BloomFilter::build(input)) }
    }

    /// The guard literals a prefilter proves absent from `input` for every
    /// member in `indices`, so one union walk can carry them all.
    fn absent_for(&self, indices: impl Iterator<Item = usize>, input: &[u8]) -> std::collections::HashSet<Vec<u8>> {
        let mut absent = std::collections::HashSet::new();
        for i in indices {
            absent.extend(crate::prefilter::absent_guard_literals(&self.pats[i], input));
        }
        absent
    }

    /// Each member of `asked` (a set index and its place in the union) with
    /// its first match at or after token `from`, from one walk of the union.
    fn first_spans_together(
        &self,
        asked: &[(usize, u32)],
        input: &[u8],
        toks: &[crate::token::Token],
        from: usize,
    ) -> Vec<(usize, Option<crate::engine::Span>)> {
        if asked.is_empty() {
            return Vec::new();
        }
        let Some(u) = self.together().union.as_ref() else {
            return Vec::new();
        };
        let mut active = vec![false; u.len()];
        for &(_, j) in asked {
            active[j as usize] = true;
        }
        let absent = self.absent_for(asked.iter().map(|&(i, _)| i), input);
        let firsts = crate::nfa::first_spans_union(u, &active, input, toks, from, &absent);
        asked.iter().map(|&(i, j)| (i, firsts[j as usize])).collect()
    }

    /// How many patterns the set holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pats.len()
    }

    /// Whether the set holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pats.is_empty()
    }

    /// The patterns themselves, in index order.
    #[must_use]
    pub fn patterns(&self) -> &[Pattern] {
        &self.pats
    }

    /// The indices of the patterns that match `input`, in index order.
    ///
    /// Every pattern is asked. A route that answers without the lexer answers
    /// first and costs nothing; what is left shares one lex.
    #[must_use]
    pub fn matches(&self, input: &[u8]) -> Vec<usize> {
        if !self.shapes.is_empty() {
            return self.first_spans_shaped(input, 0, false).into_iter().map(|(i, _)| i).collect();
        }
        let mut out = Vec::new();
        let mut shared = Vec::new();
        let mut together = Vec::new();
        let probe = self.probe(input);
        for (i, p) in self.pats.iter().enumerate() {
            match self.plan(i, p, input, &probe) {
                Plan::Settled(true) => out.push(i),
                Plan::Settled(false) => {}
                Plan::Alone => {
                    if crate::engine::is_match(p, input) {
                        out.push(i);
                    }
                }
                Plan::Shared(c, max_len) => shared.push((i, c, max_len)),
                Plan::Together(j, max_len) => together.push((i, j, max_len)),
            }
        }
        out.extend(self.over_a_shared_prefix(input, &shared, &together, false).into_iter().map(|(i, _)| i));
        out.sort_unstable();
        out
    }

    /// Whether any pattern in the set matches `input`.
    ///
    /// Stops at the first that does, so a set whose early members are
    /// byte-routable can answer without lexing even when later ones would
    /// have needed it, and a set that must lex stops widening its prefix the
    /// moment one member matches.
    #[must_use]
    pub fn is_match(&self, input: &[u8]) -> bool {
        if !self.shapes.is_empty() {
            return !self.first_spans_shaped(input, 0, true).is_empty();
        }
        let mut shared = Vec::new();
        let mut together = Vec::new();
        let mut alone = Vec::new();
        let probe = self.probe(input);
        for (i, p) in self.pats.iter().enumerate() {
            match self.plan(i, p, input, &probe) {
                Plan::Settled(true) => return true,
                Plan::Settled(false) => {}
                Plan::Alone => alone.push(p),
                Plan::Shared(c, max_len) => shared.push((i, c, max_len)),
                Plan::Together(j, max_len) => together.push((i, j, max_len)),
            }
        }
        // The members that need a lex of their own are asked after the ones a
        // byte route settled and before the shared prefix runs, so a set that
        // one of them answers never widens a prefix at all.
        if alone.iter().any(|p| crate::engine::is_match(p, input)) {
            return true;
        }
        !self.over_a_shared_prefix(input, &shared, &together, true).is_empty()
    }

    /// Which patterns match `input`, as a bitset over the set's indices.
    ///
    /// The counterpart of the regex crate's `matches`, which returns its
    /// `SetMatches`. [`Self::matches`] answers the same question as a list of
    /// the indices that matched; this reports every index with its verdict, so
    /// a caller asking about one pattern does not scan a list to find it.
    #[must_use]
    pub fn matched(&self, input: &[u8]) -> SetMatches {
        let mut bits = vec![false; self.pats.len()];
        for i in self.matches(input) {
            bits[i] = true;
        }
        SetMatches { bits }
    }

    /// Which patterns match at or after byte `at`, as a bitset.
    ///
    /// The counterpart of the regex crate's `matches_at`. The bytes before `at`
    /// are still read, so an assertion that looks back sees them.
    ///
    /// This lexes once and whole rather than widening the prefix
    /// [`Self::matches`] shares. A prefix grows forward from the start of the
    /// input, which is the wrong shape for a question anchored partway through
    /// it: the bytes a member would settle from are the ones already passed.
    #[must_use]
    pub fn matches_at(&self, input: &[u8], at: usize) -> SetMatches {
        let mut bits = vec![false; self.pats.len()];
        if !self.shapes.is_empty() {
            for (i, _) in self.first_spans_shaped(input, at, false) {
                bits[i] = true;
            }
            return SetMatches { bits };
        }
        let mut open = Vec::new();
        let probe = self.probe(input);
        for (i, p) in self.pats.iter().enumerate() {
            if !p.library_kinds().is_empty() {
                open.push(i);
                continue;
            }
            if crate::prefilter::requires_absent_with(p, input, probe.filter.as_ref()) {
                continue;
            }
            // Only the routes whose matches cannot overlap may be filtered by
            // the caller's position, and they answer without any lex at all.
            if let Some(spans) = crate::engine::routed_spans_positional(p, input) {
                bits[i] = spans.iter().any(|s| s.start() >= at);
            } else {
                open.push(i);
            }
        }
        if open.is_empty() {
            return SetMatches { bits };
        }
        let toks = crate::parallel_lex::lex_parallel(input);
        let start = toks.partition_point(|t| t.start() < at);
        // One lex for every member, read by each rather than copied to it: the
        // walk borrows this stream, so a set of eight pays one lex and no
        // member's tokens are its own.
        let sig = crate::nfa::Stitched::significant_of(&toks);
        let (asked, apart) = self.split_together(open);
        for (i, first) in self.first_spans_together(&asked, input, &toks, start) {
            bits[i] = first.is_some();
        }
        for i in apart {
            let p = &self.pats[i];
            let stream = crate::nfa::Stitched::new(&toks, &sig);
            if !p.library_kinds().is_empty() {
                bits[i] = crate::cursor::find_at(p, input, at).is_some();
            } else if let Some(mut w) = crate::nfa::SerialWalk::over_stream(p, input, stream) {
                w.seek(at);
                bits[i] = w.next_span(input).is_some();
            } else {
                bits[i] = !crate::engine::scan_tokens_from(p, input, &toks, start).is_empty();
            }
        }
        SetMatches { bits }
    }

    /// `open` split into the members the union walks, each with its place,
    /// and the members walked as themselves.
    fn split_together(&self, open: Vec<usize>) -> (Vec<(usize, u32)>, Vec<usize>) {
        let place = &self.together().place;
        let mut asked = Vec::new();
        let mut apart = Vec::new();
        for i in open {
            match place[i] {
                Some(j) => asked.push((i, j)),
                None => apart.push(i),
            }
        }
        (asked, apart)
    }

    /// Whether any pattern in the set matches at or after byte `at`.
    #[must_use]
    pub fn is_match_at(&self, input: &[u8], at: usize) -> bool {
        self.matches_at(input, at).matched_any()
    }

    /// Which patterns match `input`, and where each one first does.
    ///
    /// The regex crate's `RegexSet` cannot answer this: it reports which
    /// patterns match and states that it does not report where. The reason is
    /// that its saving comes from carrying every pattern in one automaton,
    /// which loses the identity of the pattern that reached an accepting state.
    ///
    /// A token set's saving is the shared lex rather than a shared automaton,
    /// so each member is still walked as itself and its match keeps its span.
    /// The position costs nothing beyond the walk that decided the verdict.
    #[must_use]
    pub fn matches_with_spans(&self, input: &[u8]) -> Vec<(usize, crate::engine::Span)> {
        if !self.shapes.is_empty() {
            return self.first_spans_shaped(input, 0, false);
        }
        let mut out = Vec::new();
        let mut open = Vec::new();
        let probe = self.probe(input);
        for (i, p) in self.pats.iter().enumerate() {
            if !p.library_kinds().is_empty() {
                open.push(i);
                continue;
            }
            if crate::prefilter::requires_absent_with(p, input, probe.filter.as_ref()) {
                continue;
            }
            // The first-match form, because that is the question: the whole-set
            // form reads the input to the end to report matches this discards.
            // A route answering with no span is a verdict of no match, which is
            // not the same as no route answering, so these cannot collapse.
            match crate::engine::routed_first(p, input) {
                Some(first) => {
                    if let Some(s) = first {
                        out.push((i, s));
                    }
                }
                None => open.push(i),
            }
        }
        if !open.is_empty() {
            let toks = crate::parallel_lex::lex_parallel(input);
            // Every member reads this one lex rather than taking a copy of it.
            let sig = crate::nfa::Stitched::significant_of(&toks);
            let (asked, apart) = self.split_together(open);
            for (i, first) in self.first_spans_together(&asked, input, &toks, 0) {
                if let Some(s) = first {
                    out.push((i, s));
                }
            }
            for i in apart {
                let p = &self.pats[i];
                let stream = crate::nfa::Stitched::new(&toks, &sig);
                let found = if !p.library_kinds().is_empty() {
                    crate::cursor::find(p, input)
                } else if let Some(mut w) =
                    crate::nfa::SerialWalk::over_stream(p, input, stream)
                {
                    w.next_span(input)
                } else {
                    crate::engine::scan_tokens_from(p, input, &toks, 0).into_iter().next()
                };
                if let Some(s) = found {
                    out.push((i, s));
                }
            }
        }
        out.sort_unstable_by_key(|&(i, _)| i);
        out
    }

    /// How a member is to be answered, decided once so the compile that
    /// decides it is also the compile that runs.
    fn plan(&self, index: usize, pattern: &Pattern, input: &[u8], probe: &Probe) -> Plan {
        // A library kind lives only in a lex under the library's shapes, which
        // neither a route nor the shared lex produces.
        if !pattern.library_kinds().is_empty() {
            return Plan::Alone;
        }
        if let Some(settled) = self.without_a_lex(pattern, input, probe) {
            return Plan::Settled(settled);
        }
        if !crate::prefilter::settles_from_a_prefix(pattern) {
            return Plan::Alone;
        }
        if let Some(j) = self.together().place[index] {
            return Plan::Together(j, crate::nfa::bounded_max_len(pattern).unwrap_or(1).max(1));
        }
        match crate::nfa::compile_pattern(pattern) {
            Some(c) => {
                Plan::Shared(c, crate::nfa::bounded_max_len(pattern).unwrap_or(1).max(1))
            }
            // A balanced group or a field node, which only the
            // set-reachability engine advances and which lexes its own stream.
            None => Plan::Alone,
        }
    }

    /// The members of `open` that match, found over one prefix that widens
    /// until every one of them is settled.
    ///
    /// This is where a set pays for itself. Asked separately, each member
    /// widens a prefix of its own and lexes those bytes again; asked together
    /// they widen one prefix and each round's tokens are walked once per
    /// member still open. A member that matches early drops out and the
    /// widening continues only for the rest, so the prefix reached is the one
    /// the hardest member needed and not the sum of what each needed.
    ///
    /// `stop_at_the_first` ends the whole walk as soon as any member matches,
    /// for a caller asking whether rather than which. Each member found is
    /// handed back with its first match, which a prefix's edge cannot have
    /// cut short: the first span clear of the edge is the leftmost, since a
    /// leftmost match reaching past the edge leaves none clear behind it.
    fn over_a_shared_prefix(
        &self,
        input: &[u8],
        shared: &[(usize, crate::nfa::Compiled, usize)],
        together: &[(usize, u32, usize)],
        stop_at_the_first: bool,
    ) -> Vec<(usize, crate::engine::Span)> {
        let mut found: Vec<(usize, crate::engine::Span)> = Vec::new();
        if shared.is_empty() && together.is_empty() {
            return found;
        }
        let seen = |found: &[(usize, crate::engine::Span)], i: usize| found.iter().any(|&(k, _)| k == i);
        let asked: Vec<(usize, u32)> = together.iter().map(|&(i, j, _)| (i, j)).collect();
        let bound: std::collections::HashMap<usize, usize> =
            together.iter().map(|&(i, _, max_len)| (i, max_len)).collect();
        crate::prefilter::over_widening_prefixes(input, |toks, whole| {
            // The union's members still open, walked as one over this
            // prefix; a member found drops out of the next round's walk.
            let still: Vec<(usize, u32)> =
                asked.iter().filter(|(i, _)| !seen(&found, *i)).copied().collect();
            for (i, first) in self.first_spans_together(&still, input, toks, 0) {
                let hit = if whole {
                    first
                } else {
                    first.and_then(|s| {
                        crate::prefilter::settled_clear_of_the_cut(toks, &[s], bound[&i])
                    })
                };
                if let Some(s) = hit {
                    found.push((i, s));
                    if stop_at_the_first {
                        return false;
                    }
                }
            }
            for (i, c, max_len) in shared {
                if seen(&found, *i) {
                    continue;
                }
                let spans = crate::nfa::scan_nfa_over_compiled(c, &self.pats[*i], input, toks);
                // On the last round the prefix is the whole input, so any
                // match is a match and none means none. Before that only a
                // match clear of the cut is one the cut cannot have made.
                let hit = if whole {
                    spans.first().copied()
                } else {
                    crate::prefilter::settled_clear_of_the_cut(toks, &spans, *max_len)
                };
                if let Some(s) = hit {
                    found.push((*i, s));
                    if stop_at_the_first {
                        return false;
                    }
                }
            }
            found.len() < shared.len() + together.len()
        });
        found
    }

    /// Whether every pattern in the set matches `input`.
    #[must_use]
    pub fn matched_all(&self, input: &[u8]) -> bool {
        self.matches(input).len() == self.pats.len()
    }

    /// Whether `pattern` matches `input` by a route that never lexes, or
    /// `None` where only the lexer can say.
    ///
    /// The absent-literal refusal is the one that pays here: a set of many
    /// patterns over one input usually has most of them absent, and each
    /// absence is settled by a byte search rather than by a share of a lex.
    fn without_a_lex(&self, pattern: &Pattern, input: &[u8], probe: &Probe) -> Option<bool> {
        if crate::prefilter::requires_absent_with(pattern, input, probe.filter.as_ref()) {
            return Some(false);
        }
        if let Some(lits) = crate::prefilter::byte_routable_literals(pattern) {
            if probe.refuses_all(&lits) {
                return Some(false);
            }
            if let Some(found) = crate::prefilter::byte_route_any_word_literal(&lits, input) {
                return Some(found);
            }
        }
        if let Some(punct) = crate::prefilter::byte_routable_word_then_punct(pattern)
            && let Some(found) = crate::prefilter::byte_route_any_word_then_punct(punct, input)
        {
            return Some(found);
        }
        if let Some((bp, prefix)) = crate::prefilter::byte_routable_byte_pattern(pattern)
            && let Some(found) = crate::prefilter::byte_route_any_byte_pattern(bp, &prefix, input)
        {
            return Some(found);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spread over every route a set member can take: byte-routable
    /// literals present and absent, a word then punctuation, a byte pattern,
    /// a kind sequence, the single-pass engine, and a balanced group that
    /// only the set engine advances.
    const SOURCES: &[&str] = &[
        "\"alpha\"",
        "\"zzzqqq\"",
        "\\W",
        "\\N",
        "\\W \"=\"",
        "`cond_[0-9]+`",
        "\"let\" \\W \"=\"",
        "\\W:x \"=\" =x",
        "\\B(\\W)",
        "\"nowhere_at_all\" \"=\" \\N",
    ];

    fn corpus() -> Vec<u8> {
        let mut s = String::new();
        for i in 0..300 {
            match i % 4 {
                0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
                1 => s.push_str(&format!("call_{i}(alpha, beta, {i}) ;\n")),
                2 => s.push_str(&format!("key_{i}: item_{i}, item_{} ;\n", i + 1)),
                _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
            }
        }
        s.into_bytes()
    }

    fn built() -> PatternSet {
        PatternSet::new(
            SOURCES.iter().map(|s| crate::parse(s).expect("pattern parses")).collect(),
        )
    }

    #[test]
    fn the_set_reports_what_each_pattern_reports_alone() {
        // The whole contract: sharing a lex must not change any answer. Asked
        // one at a time through the ordinary entry point, and together.
        let input = corpus();
        let set = built();
        let want: Vec<usize> = SOURCES
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                let p = crate::parse(s).expect("pattern parses");
                crate::is_match(&p, &input)
            })
            .map(|(i, _)| i)
            .collect();
        assert_eq!(set.matches(&input), want);
        assert_eq!(set.is_match(&input), !want.is_empty());
        assert_eq!(set.matched_all(&input), want.len() == SOURCES.len());
    }

    #[test]
    fn the_position_row_reports_where_each_pattern_first_matches_alone() {
        // The spans must be the ones each pattern's own first-match path gives.
        // The rung that answers inside the set is not always the one that
        // answers a lone ask - the set takes the first-match ladder and shares
        // a lex for what falls through it - and the span must not depend on
        // which of them answered.
        let input = corpus();
        let set = built();
        let want: Vec<(usize, crate::engine::Span)> = SOURCES
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let p = crate::parse(s).expect("pattern parses");
                crate::find(&p, &input).map(|span| (i, span))
            })
            .collect();
        assert_eq!(set.matches_with_spans(&input), want);
        // The first-match row takes a ladder that stops the lexer rather
        // than walking a whole lex, and must reach the same spans.
        let mut firsts: Vec<(usize, crate::engine::Span)> = set
            .first_matches(&input, false)
            .into_iter()
            .map(|(i, m)| {
                let at = |o: usize| u32::try_from(o).expect("a corpus offset fits a span");
                (i, crate::engine::Span { start: at(m.start), end: at(m.end) })
            })
            .collect();
        firsts.sort_unstable_by_key(|&(i, _)| i);
        assert_eq!(firsts, want);
    }

    /// `n` patterns spread over every route: literal-led sequences the
    /// single-pass engine walks, kind-led ones with a typed predicate,
    /// literal runs a byte route settles, balanced groups the set engine owns,
    /// and a guard that keeps a member whole.
    fn generated(n: usize) -> Vec<Pattern> {
        (0..n)
            .map(|i| {
                let src = match i % 8 {
                    0 => format!("\"value_{i}\" \"=\" \\N"),
                    1 => format!("\"call_{i}\" \\B(\\W \",\" \\W \",\" \\N)"),
                    2 => format!("\"key_{i}\" \":\" \\W"),
                    3 => format!("\"cond_{i}\" \")\" \"{{\""),
                    4 => format!("\\W \"=\" \\N{{={}}}", i * 37),
                    5 => format!("\"item_{i}\" ~\"alpha\""),
                    6 => format!("\\N{{>={i}}} \";\""),
                    _ => format!("\"do_{i}\" \\B(\\W)"),
                };
                crate::parse(&src).expect("pattern parses")
            })
            .collect()
    }

    #[test]
    fn walked_as_one_agrees_with_walked_apart_at_every_size() {
        // The union changes how the members the single-pass engine takes are
        // walked and nothing about what they answer: every verdict, first
        // span and positional verdict must equal the per-member walk's, over
        // sets small enough to read and large enough to matter.
        let input = corpus();
        for n in [10usize, 100, 1000] {
            let together = PatternSet::new(generated(n));
            let apart = PatternSet::new(generated(n)).walked_as_one(false);
            assert_eq!(together.matches(&input), apart.matches(&input), "matches, {n} patterns");
            assert!(!together.matches(&input).is_empty(), "the generated set has members that match");
            assert_eq!(
                together.matches_with_spans(&input),
                apart.matches_with_spans(&input),
                "first spans, {n} patterns"
            );
            assert_eq!(together.is_match(&input), apart.is_match(&input), "is_match, {n} patterns");
            for at in [0usize, 1000, input.len() / 2, input.len()] {
                assert_eq!(together.matches_at(&input, at), apart.matches_at(&input, at), "at {at}, {n} patterns");
            }
        }
    }

    #[test]
    fn an_empty_set_matches_nothing_and_matches_all_of_it() {
        // `matched_all` over no patterns is vacuously true, which is the same
        // reading the regex crate takes and worth pinning so it cannot drift.
        let set = PatternSet::new(Vec::new());
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
        assert_eq!(set.matches(b"anything"), Vec::<usize>::new());
        assert!(!set.is_match(b"anything"));
        assert!(set.matched_all(b"anything"));
    }

    #[test]
    fn a_set_of_only_absent_patterns_matches_none() {
        let input = corpus();
        let set = PatternSet::new(
            ["\"zzzqqq\"", "\"nowhere_at_all\"", "\"absent_word\" \"=\""]
                .iter()
                .map(|s| crate::parse(s).expect("pattern parses"))
                .collect(),
        );
        assert_eq!(set.matches(&input), Vec::<usize>::new());
        assert!(!set.is_match(&input));
        assert!(!set.matched_all(&input));
    }

    #[test]
    fn the_index_follows_the_order_the_set_was_built_in() {
        // The index is the only handle a caller has on which pattern matched,
        // so it must be the position given and not the order answers arrive
        // in - and answers do not arrive in order here, since the routed
        // patterns are settled before the lexed ones.
        let input = corpus();
        let set = PatternSet::new(
            ["\\B(\\W)", "\"alpha\"", "\"zzzqqq\"", "\\W \"=\""]
                .iter()
                .map(|s| crate::parse(s).expect("pattern parses"))
                .collect(),
        );
        assert_eq!(set.matches(&input), vec![0, 1, 3]);
        assert_eq!(set.patterns().len(), 4);
    }

    #[test]
    fn a_shared_prefix_that_widens_answers_what_a_single_ask_answers() {
        // An input well past the first prefix, so the widening runs more than
        // one round and members drop out of it at different rounds. Members
        // that match nowhere force it all the way to the end, which is the
        // case where sharing has to still be correct rather than merely fast.
        let mut input = corpus();
        while input.len() < 400_000 {
            let more = corpus();
            input.extend_from_slice(&more);
        }
        input.extend_from_slice(b"\nonly_at_the_very_end = 7 ;\n");
        let sources = [
            "\"alpha\"",
            "\"nowhere_at_all\"",
            "\\W \"=\" \\N",
            "\"only_at_the_very_end\" \"=\" \\N",
            "\\B(\\W)",
            "\"zzzqqq\" \"=\"",
        ];
        let set = PatternSet::new(
            sources.iter().map(|s| crate::parse(s).expect("pattern parses")).collect(),
        );
        let want: Vec<usize> = sources
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                let p = crate::parse(s).expect("pattern parses");
                crate::is_match(&p, &input)
            })
            .map(|(i, _)| i)
            .collect();
        assert_eq!(set.matches(&input), want);
        assert_eq!(set.is_match(&input), !want.is_empty());
        assert!(want.contains(&3), "the member that matches only at the end is found");
        assert!(!want.contains(&1), "the member that matches nowhere is not");
    }

    #[test]
    fn an_empty_input_matches_nothing() {
        let set = built();
        assert_eq!(set.matches(b""), Vec::<usize>::new());
        assert!(!set.is_match(b""));
    }
}
