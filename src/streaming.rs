//! Streaming scan: feed the input in chunks and recover exactly the
//! matches a whole-input scan would produce.
//!
//! The contract is metamorphic: for any pattern and any way of cutting
//! the input into chunks, the streaming result equals
//! [`crate::scan`] on the concatenation. The scanner reaches that by
//! committing only what cannot change once committed.
//!
//! ## What can and cannot be committed early
//!
//! A match commits early only when no later byte can affect it. Two
//! things make a match depend on later bytes:
//!
//! - a quantifier or balanced group whose match could extend past the
//!   chunk seam (`.*`, `\B(...)`), and
//! - a content guard `~"lit"`, whose forward window can be satisfied by
//!   a literal that has not arrived yet.
//!
//! So the scanner commits a prefix only up to a boundary where no token
//! and no match straddles the cut, and it commits nothing early at all
//! while the pattern carries a guard (the guard's forward dependence is
//! unbounded). Whatever cannot be committed is retained and re-scanned
//! when the next chunk arrives; `finish` scans the final remainder. The
//! retained buffer is bounded by the longest unbroken run between safe
//! boundaries, not by the whole input, except for guarded or
//! boundary-spanning patterns, which retain until `finish` by
//! necessity, not by shortcut.

use crate::ast::Pattern;
use crate::engine::Span;
use crate::pattern_set::PatternSet;
use crate::token::Token;

/// What a stream scans: one pattern, or a set whose every match carries
/// its member.
enum Source {
    One(Pattern),
    Set(Box<PatternSet>),
}

/// A streaming scanner over one compiled pattern, or over a set.
pub struct StreamScanner {
    source: Source,
    /// Unfinalized bytes: everything from `base` to the current end.
    buf: Vec<u8>,
    /// Absolute offset of `buf[0]` in the whole input.
    base: usize,
    /// Matches already committed, in absolute offsets, each with the set
    /// member that made it, `0` for a stream over one pattern.
    out: Vec<(usize, Span)>,
    /// Whether the pattern depends on input beyond the lines its match spans,
    /// which forbids any early commit and means the whole input must be
    /// buffered. Every cut falls just after a newline, so a line anchor is
    /// settled once its line has ended. See
    /// [`Pattern::depends_on_more_than_its_lines`].
    defers_commit: bool,
    /// The most bytes retained at once so far.
    peak: usize,
    /// Whether the buffer's end is a boundary when it ends a line. The only
    /// token that could span such a cut is a run of whitespace, so it is
    /// unless the pattern reads whitespace itself.
    tail: bool,
    /// The most tokens a match can span, for a pattern whose length is
    /// bounded: such a match commits once the decided prefix holds that
    /// many tokens past its start, and only the last tokens of the prefix
    /// can still begin one that reaches past it.
    span_limit: Option<usize>,
    /// The absolute end of the last committed match. A scan of the retained
    /// buffer resumes there, so a committed match is never found twice
    /// where the buffer keeps its bytes for the lexer's sake.
    committed_through: usize,
    /// The shapes the retained buffer is lexed under, which for one pattern
    /// are the shapes its own scan would build: holding them here is what
    /// lets a push lex once and hand the tokens to the scan rather than have
    /// it lex the same bytes again. Empty for a set, whose members can need
    /// different shapes from one another and take their own lex.
    shapes: crate::custom::ShapeSet,
    /// The shapes a caller declared for one pattern, without the library's,
    /// which the scan at the stream's end is handed as a whole-input scan
    /// under them would be.
    declared: crate::custom::ShapeSet,
    /// The lexer's buffers, held from one push to the next so their pages stay
    /// mapped and the token vector is not rebuilt per chunk.
    lex_ws: crate::parallel_lex::TokenWorkspace,
    /// Every byte of the retained buffer a cut may fall on, ascending, from the
    /// one scan a push makes of it. Held rather than rebuilt for the same
    /// reason the token buffer is.
    bounds: Vec<usize>,
}

impl StreamScanner {
    /// Begin a streaming scan for `pattern`.
    #[must_use]
    pub fn new(pattern: Pattern) -> Self {
        Self::with_shapes(pattern, crate::custom::ShapeSet::new())
    }

    /// Begin a streaming scan for `pattern` under the shapes and kinds
    /// `shapes` declares, which decide token boundaries as they do for
    /// [`crate::scan_with_shapes`]: the stream finds the matches that scan
    /// finds over the whole input.
    #[must_use]
    pub fn with_shapes(pattern: Pattern, shapes: crate::custom::ShapeSet) -> Self {
        let span_limit = pattern.max_tokens();
        let defers_commit = pattern.depends_on_more_than_its_lines() || !settles(&pattern, span_limit);
        let tail = !pattern.reads_whitespace();
        let mut scanner = Self::over(Source::One(pattern), defers_commit, tail, span_limit);
        if !shapes.is_empty() {
            if let Source::One(pattern) = &scanner.source {
                scanner.shapes = shapes.with_library_shapes(&pattern.library_kinds());
            }
            scanner.declared = shapes;
        }
        scanner
    }

    /// Begin a streaming scan for every member of `set` at once, each match
    /// tagged with its member. The stream commits under its most
    /// conservative member: a match commits once the decided prefix holds
    /// the longest span any member can make, nothing commits before the
    /// end where any member depends on the whole input, and a line's end
    /// is no boundary where any member reads whitespace; one window serves
    /// them all.
    #[must_use]
    pub fn over_set(set: PatternSet) -> Self {
        let span_limit = set.max_tokens();
        let defers_commit = set.depends_on_more_than_its_lines()
            || !set.patterns().iter().all(|p| settles(p, span_limit));
        let tail = !set.reads_whitespace();
        Self::over(Source::Set(Box::new(set)), defers_commit, tail, span_limit)
    }

    fn over(source: Source, defers_commit: bool, tail: bool, span_limit: Option<usize>) -> Self {
        let shapes = match &source {
            Source::One(pattern) => {
                crate::custom::ShapeSet::new().with_library_shapes(&pattern.library_kinds())
            }
            Source::Set(_) => crate::custom::ShapeSet::new(),
        };
        Self {
            shapes,
            declared: crate::custom::ShapeSet::new(),
            lex_ws: crate::parallel_lex::TokenWorkspace::default(),
            bounds: Vec::new(),
            source,
            buf: Vec::new(),
            base: 0,
            out: Vec::new(),
            defers_commit,
            peak: 0,
            tail,
            span_limit,
            committed_through: 0,
        }
    }

    /// Whether a match can commit before the stream ends. A pattern that
    /// depends on the whole input retains everything until [`Self::finish`].
    #[must_use]
    pub fn commits_early(&self) -> bool {
        !self.defers_commit
    }

    /// The absolute offset of the first retained byte: everything before it
    /// is committed, and no later byte can reach it.
    #[must_use]
    pub fn base(&self) -> usize {
        self.base
    }

    /// The bytes retained now.
    #[must_use]
    pub fn retained(&self) -> usize {
        self.buf.len()
    }

    /// Count this stream's offsets from its first retained byte rather than
    /// from where it began, and give how far they moved; a reader adds that
    /// to every offset reported after. A span's offsets reach four GiB, so a
    /// stream that runs longer, as a file followed for days can, is rebased
    /// as it goes. Every committed match must have been drained first, since
    /// those carry the offsets from before.
    ///
    /// # Panics
    ///
    /// A committed match has not been drained.
    pub fn rebase(&mut self) -> usize {
        assert!(self.out.is_empty(), "a stream is rebased only once its committed matches are drained");
        let by = self.base;
        self.base = 0;
        self.committed_through = self.committed_through.saturating_sub(by);
        by
    }

    /// The most bytes retained at once so far.
    #[must_use]
    pub fn peak_retained(&self) -> usize {
        self.peak
    }

    /// Feed the next chunk of input. Matches that can no longer change
    /// are committed; the rest is retained for the following chunk.
    pub fn push(&mut self, chunk: &[u8]) {
        let taking = crate::trace::phase("streaming: taking the chunk in");
        self.buf.extend_from_slice(chunk);
        drop(taking);
        self.peak = self.peak.max(self.buf.len());
        if self.defers_commit {
            // A guard's forward window or a field's comma count reaches
            // outside the committable prefix, so no match is final until
            // `finish` sees the whole input.
            return;
        }
        match self.span_limit {
            Some(limit) => self.commit_bounded(limit),
            None => self.try_commit(),
        }
    }

    /// The matches of the retained buffer from the last committed match on,
    /// each with its member.
    fn scan_retained(&self) -> Vec<(usize, Span)> {
        let from = self.committed_through.saturating_sub(self.base);
        match &self.source {
            Source::One(pattern) => crate::engine::scan_with_shapes_from(pattern, &self.buf, &self.declared, from)
                .into_iter()
                .map(|s| (0, s))
                .collect(),
            Source::Set(set) => set.scan_from(&self.buf, from),
        }
    }

    /// Lex `buf` under `shapes` into `ws`, which for one pattern is the lex its
    /// own scan would take, so the scan can be handed these rather than repeat
    /// them.
    ///
    /// Serial, into a buffer held from the last push, which is the pairing
    /// [`crate::lexer::lex_into`] documents: a held vector has its pages
    /// already and lexes in three quarters of the time, but holding one across
    /// dispatched work costs more than the fresh pages save, because writing it
    /// again invalidates its lines in every core's cache. A push lexes and then
    /// walks the buffer serially, so the held half applies and the dispatched
    /// half does not.
    ///
    /// A declared shape decides token boundaries `lex_into` does not read, so
    /// that case takes the shaped lexer and a fresh vector.
    ///
    /// Takes the three pieces rather than `self` so the buffer, the shapes and
    /// the workspace are borrowed apart.
    fn lex_retained_into(
        buf: &[u8],
        shapes: &crate::custom::ShapeSet,
        ws: &mut crate::parallel_lex::TokenWorkspace,
    ) {
        if shapes.is_empty() {
            crate::lexer::lex_into(buf, &mut ws.toks);
        } else {
            let blobs = crate::lexer::blob_runs(buf);
            ws.toks = crate::lexer::lex_with_shapes(buf, &blobs, shapes, 0);
        }
    }

    /// [`Self::scan_retained`] over a lex of the buffer the caller holds
    /// already, where that lex is the one the scan would have taken itself.
    ///
    /// [`Self::lex_retained`] lexes under the shapes one pattern's scan builds
    /// from its own library kinds, so the tokens agree for every such pattern.
    /// A set is lent that same lex and spends it a member at a time: a member
    /// drawing no shape of its own reads these tokens, and one that does lexes
    /// under its shapes, since no single lex serves members that disagree.
    fn scan_retained_over(&self, toks: &[Token]) -> Vec<(usize, Span)> {
        let from = self.committed_through.saturating_sub(self.base);
        match &self.source {
            Source::One(pattern) => {
                // The routes over the retained buffer's own bytes, which answer
                // without walking its tokens where the pattern draws no shape.
                // The lex above is paid whatever they say - the drain reads it
                // to size what the buffer keeps - so what a route saves here is
                // the walk and not the lex.
                if self.shapes.is_empty()
                    && let Some(spans) = crate::engine::routed_spans_at(pattern, &self.buf, from)
                {
                    return spans.into_iter().map(|s| (0, s)).collect();
                }
                crate::engine::scan_over_tokens_from(pattern, &self.buf, toks, from)
                    .into_iter()
                    .map(|s| (0, s))
                    .collect()
            }
            Source::Set(set) => set.scan_from_over(&self.buf, from, toks),
        }
    }

    /// Commit for a pattern whose match spans at most `limit` tokens. The
    /// decided prefix runs to the last line boundary, or to the buffer's end
    /// when it ends a line; a match commits when the prefix holds `limit`
    /// tokens from its start, since every alternative at that start is then
    /// in view; and after the last commit only the prefix's last `limit - 1`
    /// tokens can still begin a match reaching past it, so the buffer keeps
    /// from the line boundary before them.
    fn commit_bounded(&mut self, limit: usize) {
        // The four acts of a push are timed apart, because the three that are
        // not the lex were inside no phase at all and a chunk size that moved
        // the whole could not be attributed to any of them.
        let deciding = crate::trace::phase("streaming: deciding the prefix");
        let end_held = scan_boundaries(&self.buf, &mut self.bounds);
        let decided = boundary_below(&self.buf, &self.bounds, end_held, self.buf.len(), self.tail);
        drop(deciding);
        let Some(decided) = decided else {
            return;
        };
        // The same re-lex as the unbounded path does, in the other function: a
        // pattern whose match spans a bounded number of tokens commits here and
        // never reaches `settled_through`, so an instrument on one path alone
        // reports nothing for half the patterns. The names are shared so the
        // lex reads as one quantity whichever path paid it.
        let lexing = crate::trace::phase("streaming: lexing the retained buffer");
        Self::lex_retained_into(&self.buf, &self.shapes, &mut self.lex_ws);
        let toks = &self.lex_ws.toks;
        drop(lexing);
        // What a push walks and re-walks: the retained bytes and the tokens made
        // of them. Four of a push's acts are linear in one or the other, so
        // these two summed over the pushes, against the pushes, are the working
        // set a chunk size asks a core to hold - which is a count rather than a
        // reading of any cache.
        crate::trace::counted(
            "streaming: bytes the buffer holds",
            u64::try_from(self.buf.len()).expect("a buffer within the counter's width"),
        );
        crate::trace::counted(
            "streaming: bytes the tokens occupy",
            u64::try_from(toks.len() * std::mem::size_of::<Token>())
                .expect("a token vector within the counter's width"),
        );
        // Where each significant token of the decided prefix begins, which is
        // the whole of what the drain below asks of them: how many start before
        // a byte, and where the k-th one starts. A token's end decides only
        // whether it lies inside the prefix, so nothing past this filter reads
        // one and the list carries starts alone.
        let gathering = crate::trace::phase("streaming: gathering the significant tokens");
        let sig: Vec<usize> = toks
            .iter()
            .filter(|t| t.is_significant() && t.end() <= decided)
            .map(crate::token::Token::start)
            .collect();
        let index_at = |byte: usize| sig.partition_point(|&s| s < byte);
        let mut after = index_at(self.committed_through.saturating_sub(self.base));
        drop(gathering);
        let scanning = crate::trace::phase("streaming: scanning the retained buffer");
        // What the scan is over, so its cost can be divided by its bytes. A
        // scan resuming at `committed_through` covers the buffer from there,
        // and that offset does not advance while nothing commits - so this says
        // whether the phase is many small scans or a few large ones without
        // anyone having to reason about which.
        crate::trace::counted(
            "streaming: bytes the scans cover",
            u64::try_from(self.buf.len() - self.committed_through.saturating_sub(self.base))
                .expect("a buffer within the counter's width"),
        );
        let found = self.scan_retained_over(toks);
        drop(scanning);
        for (member, m) in found {
            if m.end() > decided || index_at(m.start()) + limit > sig.len() {
                break;
            }
            self.out.push((member, shifted(m, self.base)));
            self.committed_through = self.base + m.end();
            after = index_at(m.end());
        }
        let dropping = crate::trace::phase("streaming: dropping the settled bytes");
        let retain = after.max((sig.len() + 1).saturating_sub(limit));
        let retain_byte = if retain < sig.len() { sig[retain] } else { decided };
        // The second limit answered from the first scan's boundaries rather
        // than from a second scan of the same bytes.
        let cut = if retain_byte >= self.buf.len() {
            self.buf.len()
        } else {
            boundary_below(&self.buf, &self.bounds, end_held, retain_byte + 1, self.tail)
                .unwrap_or(0)
        };
        // What the drain moves: the bytes it keeps, which it shifts down to the
        // buffer's start. A cost that tracks this rather than the chunk is one
        // that depends on how much of a buffer survives a push, which is the
        // shape a curve non-monotonic in chunk size would have.
        crate::trace::counted(
            "streaming: bytes the drain moves",
            u64::try_from(self.buf.len() - cut).expect("a buffer within the counter's width"),
        );
        self.buf.drain(..cut);
        self.base += cut;
        drop(dropping);
    }

    /// The matches committed so far, handed over once: a later call returns
    /// only what was committed after it, and `finish` returns what it
    /// commits plus whatever was never drained.
    pub fn drain_committed(&mut self) -> Vec<Span> {
        self.drain_committed_with_members().into_iter().map(|(_, s)| s).collect()
    }

    /// [`Self::drain_committed`], each match with the set member that made
    /// it; `0` throughout for a stream over one pattern.
    pub fn drain_committed_with_members(&mut self) -> Vec<(usize, Span)> {
        std::mem::take(&mut self.out)
    }

    /// Finish the stream: scan the retained buffer and return every match
    /// not yet drained, in absolute offsets.
    #[must_use]
    pub fn finish(self) -> Vec<Span> {
        self.finish_with_members().into_iter().map(|(_, s)| s).collect()
    }

    /// [`Self::finish`], each match with the set member that made it.
    #[must_use]
    pub fn finish_with_members(mut self) -> Vec<(usize, Span)> {
        for (member, m) in self.scan_retained() {
            self.out.push((member, shifted(m, self.base)));
        }
        self.out
    }

    /// Commit every match that ends at or before the largest boundary no
    /// live attempt can still reach back across, then drop the committed
    /// bytes.
    ///
    /// This is the path for a pattern whose match has no bounded length, so
    /// there is no count of tokens that puts a limit on how far back a match
    /// arriving with the next chunk could begin. The matches found in the
    /// buffer as it stands do not answer that: `\W+ "END"` over `alpha\n`
    /// holds no match at all, and cutting on that basis drops `alpha` from a
    /// match the next chunk completes. What answers it is the walk's own
    /// thread list at the end of the buffer - every attempt a further token
    /// could carry forward - and the earliest byte any of those began at is
    /// the first byte this buffer must keep.
    ///
    /// Where that cannot be computed, which is where the pattern needs the
    /// set-reachability engine, nothing is dropped: a stream that keeps too
    /// much is slow, and one that drops a live attempt is wrong.
    fn try_commit(&mut self) {
        // One lex of the retained buffer serves both the settling walk and the
        // scan that follows it, which read the same bytes under the same
        // shapes.
        let lexing = crate::trace::phase("streaming: lexing the retained buffer");
        Self::lex_retained_into(&self.buf, &self.shapes, &mut self.lex_ws);
        let toks = &self.lex_ws.toks;
        drop(lexing);
        let Some(settled) = self.settled_through(toks) else {
            return;
        };
        let scanning = crate::trace::phase("streaming: scanning the retained buffer");
        crate::trace::counted(
            "streaming: bytes the scans cover",
            u64::try_from(self.buf.len() - self.committed_through.saturating_sub(self.base))
                .expect("a buffer within the counter's width"),
        );
        let matches = self.scan_retained_over(toks);
        drop(scanning);
        // One scan of the buffer's boundaries, which both the empty case below
        // and the straddle loop under it read: the loop asks for the next
        // boundary down once per match that crosses a cut, and each of those
        // asks was a scan of the whole buffer.
        // Held to the end of the call rather than dropped at a point, because
        // every way out of this function from here drops the bytes it settled
        // or returns having settled none.
        let _dropping = crate::trace::phase("streaming: dropping the settled bytes");
        let end_held = scan_boundaries(&self.buf, &mut self.bounds);
        if matches.is_empty() {
            // No match yet, and no attempt reaching back past `settled`, so
            // the bytes before the last boundary under it are dead weight.
            if let Some(cut) = boundary_below(&self.buf, &self.bounds, end_held, settled, false) {
                self.buf.drain(..cut);
                self.base += cut;
            }
            return;
        }
        // The largest such boundary that no match straddles either. A match
        // straddling the cut would be split, so such a cut is rejected.
        let spans: Vec<Span> = matches.iter().map(|&(_, s)| s).collect();
        let Some(cut) = largest_uncrossed_boundary(
            &self.buf,
            &self.bounds,
            end_held,
            &spans,
            settled,
            false,
        ) else {
            return;
        };
        for &(member, m) in &matches {
            if m.end() <= cut {
                self.out.push((member, shifted(m, self.base)));
            }
        }
        self.buf.drain(..cut);
        self.base += cut;
    }

    /// The byte of the retained buffer before which no attempt is still
    /// running, so nothing below it can be reached by a match the next chunk
    /// completes; `None` where the engine cannot say and the buffer must be
    /// kept whole.
    ///
    /// A set is settled only as far as its least settled member: one member
    /// with a live attempt holds the buffer for all of them, which is the
    /// same rule as the one window the set commits under.
    fn settled_through(&self, toks: &[Token]) -> Option<usize> {
        // The walk consults the absent guard literals itself, so a
        // short-circuit above this one could save the lex and not the walk -
        // which is why the caller holds the lex and this phase times the walk
        // alone.
        let _walking = crate::trace::phase("streaming: the walk over the tokens");
        let of = |p: &Pattern| {
            crate::nfa::earliest_unsettled(p, &self.buf, toks).map(|at| at.unwrap_or(self.buf.len()))
        };
        match &self.source {
            Source::One(pattern) => of(pattern),
            Source::Set(set) => {
                let mut least = self.buf.len();
                for p in set.patterns() {
                    least = least.min(of(p)?);
                }
                Some(least)
            }
        }
    }
}

/// Where a held stream's offsets are counted from once its retained window
/// has moved this far: a span's offsets reach four GiB, so a stream longer
/// than that, as a file followed for days can be, counts from its retained
/// window rather than from its start and adds the difference back.
const REBASE_AT: usize = 1 << 30;

/// A stream scanner and the bytes from its retained window on, with where
/// they stand in the input: what a reader needs beside the scanner to read
/// each committed match's text and registers and to place it on its line.
///
/// The bytes a push settles are dropped at the next push, so the matches a
/// push commits are read against [`HeldStream::held`] until then.
pub struct HeldStream {
    scanner: StreamScanner,
    /// The input from the scanner's retained window on.
    held: Vec<u8>,
    /// Where `held` begins, in the scanner's offsets.
    held_base: usize,
    /// How far the input has been pushed to the scanner, in its offsets.
    pushed: usize,
    /// Where the scanner's offset zero stands in the input.
    origin: usize,
    /// The input's newlines before `held`, where the stream counts them.
    lines_before: Option<usize>,
}

/// What a held stream comes to at its input's end: the matches the scanner
/// held until then, as spans over `held`, with each member, and the bytes
/// and their place, as [`HeldStream`] gives them.
pub struct Ended {
    pub matches: Vec<(usize, Span)>,
    pub held: Vec<u8>,
    /// Where `held` begins in the input.
    pub base: usize,
    /// How many of the input's lines come before `held`, where the stream
    /// counted them.
    pub lines_before: Option<usize>,
}

impl HeldStream {
    /// `scanner` over an input whose first byte stands at `origin`, after
    /// `lines_before` of its lines; the lines are counted on as the stream
    /// moves where they were counted to its start, and left uncounted where
    /// they were not.
    #[must_use]
    pub fn new(scanner: StreamScanner, origin: usize, lines_before: Option<usize>) -> Self {
        HeldStream { scanner, held: Vec::new(), held_base: 0, pushed: 0, origin, lines_before }
    }

    /// Whether a match can commit before the input ends.
    #[must_use]
    pub fn commits_early(&self) -> bool {
        self.scanner.commits_early()
    }

    /// The bytes held: the input from the scanner's retained window on.
    #[must_use]
    pub fn held(&self) -> &[u8] {
        &self.held
    }

    /// Where the held bytes begin in the input.
    #[must_use]
    pub fn base(&self) -> usize {
        self.origin + self.held_base
    }

    /// How many of the input's lines come before the held bytes, where the
    /// stream counts them.
    #[must_use]
    pub fn lines_before(&self) -> Option<usize> {
        self.lines_before
    }

    /// How much of the held bytes is final: no match can still begin before
    /// this offset into them, and every match that ends before it has been
    /// committed.
    #[must_use]
    pub fn settled(&self) -> usize {
        self.scanner.base() - self.held_base
    }

    /// Push the input's next bytes: the matches they commit, each with the
    /// member that made it, as spans over the held bytes.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<(usize, Span)> {
        self.settle();
        self.held.extend_from_slice(bytes);
        let from = self.pushed - self.held_base;
        self.scanner.push(&self.held[from..]);
        self.pushed = self.held_base + self.held.len();
        let committed = self.scanner.drain_committed_with_members();
        local(committed, self.held_base)
    }

    /// End the input: the matches the scanner held until then, over the
    /// bytes still held.
    #[must_use]
    pub fn finish(mut self) -> Ended {
        self.settle();
        let HeldStream { scanner, held, held_base, origin, lines_before, .. } = self;
        let matches = local(scanner.finish_with_members(), held_base);
        Ended { matches, held, base: origin + held_base, lines_before }
    }

    /// Drop the bytes the last push settled, counting their newlines, and
    /// count the offsets from the retained window once it is far along.
    fn settle(&mut self) {
        let keep_from = self.scanner.base();
        if keep_from > self.held_base {
            let dropped = keep_from - self.held_base;
            if let Some(lines) = self.lines_before.as_mut() {
                *lines += crate::byte_simd::count_byte(&self.held[..dropped], b'\n');
            }
            self.held.drain(..dropped);
            self.held_base = keep_from;
        }
        if self.held_base >= REBASE_AT {
            let by = self.scanner.rebase();
            self.origin += by;
            self.held_base -= by;
            self.pushed -= by;
        }
    }
}

/// `committed`, spans in a scanner's offsets, as spans over the bytes held
/// from `held_base`.
fn local(committed: Vec<(usize, Span)>, held_base: usize) -> Vec<(usize, Span)> {
    let at = |offset: usize| {
        u32::try_from(offset - held_base).expect("a retained window is narrower than a span's width")
    };
    committed.into_iter().map(|(member, s)| (member, Span { start: at(s.start()), end: at(s.end()) })).collect()
}

/// Whether a stream over `pattern` can ever drop a byte before its end.
///
/// A bounded match settles by its own token count, whatever engine walks
/// it. An unbounded one settles only where the walk's thread list can be
/// read, which is where the pattern compiles to the single-pass program; a
/// pattern the set-reachability engine owns - a balanced group, a field
/// node - offers no such list, so nothing about it is ever settled and the
/// stream must keep every byte until it ends. Deciding that here rather
/// than per push is what lets the scanner report it: a caller asking
/// [`Self::commits_early`] is told before the bytes pile up, and the
/// command line says how much it retained when the stream closes.
fn settles(pattern: &Pattern, span_limit: Option<usize>) -> bool {
    span_limit.is_some() || crate::nfa::earliest_unsettled(pattern, b"", &[]).is_some()
}

/// The span at the whole input's offsets: the buffer's offsets plus `base`.
fn shifted(span: Span, base: usize) -> Span {
    let at = |offset: usize| {
        u32::try_from(offset + base)
            .expect("a stream's absolute offset fits the span width as an input's does")
    };
    Span { start: at(span.start()), end: at(span.end()) }
}

/// Scan `chunks` as a stream and return the whole-input matches. A
/// convenience over [`StreamScanner`] for callers that already hold an
/// iterator of byte slices.
#[must_use]
pub fn scan_chunked<'a>(pattern: &Pattern, chunks: impl IntoIterator<Item = &'a [u8]>) -> Vec<Span> {
    let mut s = StreamScanner::new(pattern.clone());
    for c in chunks {
        s.push(c);
    }
    s.finish()
}

/// The largest safe boundary in `buf` below `limit` that no match
/// straddles, or `None` when none qualifies (so nothing can be committed
/// yet).
/// Each rejected boundary sends this back for the next one down, and reading
/// that from the boundaries already collected makes the retry a search rather
/// than another scan of the buffer.
fn largest_uncrossed_boundary(
    buf: &[u8],
    bounds: &[usize],
    end_held: bool,
    matches: &[Span],
    limit: usize,
    tail: bool,
) -> Option<usize> {
    let mut cut = boundary_below(buf, bounds, end_held, limit, tail)?;
    loop {
        // A match straddles `cut` when it starts before and ends after.
        if matches.iter().any(|m| m.start() < cut && m.end() > cut) {
            cut = boundary_below(buf, bounds, end_held, cut, tail)?;
            continue;
        }
        return Some(cut);
    }
}

/// Whether a token of the lexer's may span the newline at `nl`, so that no cut
/// falls after it: a string, of either quote, carried over the line by a
/// backslash before the newline or before its CR, or a char literal holding
/// the newline itself, a quote on each side. Every other newline is a token
/// boundary - a string closes on its own line - and a cut after it needs no
/// quote state carried from the buffer's start.
///
/// A quote before the newline and none after it yet, at the buffer's end, may
/// be the first half of such a char literal, so it holds the newline too.
fn newline_held(buf: &[u8], nl: usize) -> bool {
    if nl == 0 {
        return false;
    }
    match buf[nl - 1] {
        b'\'' => nl + 1 >= buf.len() || crate::lexer::char_literal_end(buf, nl - 1) == Some(nl + 2),
        _ => crate::lexer::backslash_before_newline(buf, nl),
    }
}

/// Every byte of `buf` a cut may fall on, ascending, written to `out`, and
/// whether a token may run on past the buffer's end across its last newline.
///
/// A scan per limit would find the same newlines once per limit. This finds
/// them once and [`boundary_below`] answers each limit from what it wrote.
/// Whether a byte is a boundary depends on the bytes around its newline alone,
/// so a boundary below some limit is the same boundary a scan stopping at that
/// limit would have found, and the tests hold the two to that.
fn scan_boundaries(buf: &[u8], out: &mut Vec<usize>) -> bool {
    out.clear();
    let mut from = 0;
    while let Some(rel) = crate::byte_simd::find(&buf[from..], b"\n") {
        let nl = from + rel;
        from = nl + 1;
        if from < buf.len() && !buf[from].is_ascii_whitespace() && !newline_held(buf, nl) {
            out.push(from);
        }
    }
    buf.last() == Some(&b'\n') && newline_held(buf, buf.len() - 1)
}

/// The largest safe split boundary below `limit`, read from the boundaries
/// [`scan_boundaries`] wrote and whether a token may run past the buffer's
/// last newline.
///
/// `end_held` answers for the buffer's end, which only a limit reaching the end
/// reads - so the two agree wherever it is consulted.
fn boundary_below(
    buf: &[u8],
    bounds: &[usize],
    end_held: bool,
    limit: usize,
    tail: bool,
) -> Option<usize> {
    if tail && limit >= buf.len() && !end_held && buf.last() == Some(&b'\n') {
        return Some(buf.len());
    }
    let above = bounds.partition_point(|&b| b < limit);
    (above > 0).then(|| bounds[above - 1])
}

/// The largest safe split boundary in `buf` strictly below `limit`: a
/// significant byte directly after a newline no token spans. At such a point
/// the prefix lexes exactly as it would in the whole input. With `tail`, and
/// `limit` reaching the buffer's end, the end is a boundary too when the buffer
/// ends in a newline no token may span: the only token that could span that cut
/// is a run of whitespace, which the pattern then never reads, so a line's
/// match commits as soon as the line ends. Returns `None` when none exists
/// below `limit`.
///
/// Walks for one limit and answers it. Held as the reference the collected
/// form is checked against, a limit at a time over inputs that quote, escape
/// and leave a quote open: a stream reads the collected form, so the two
/// agreeing is what says the collected form cuts where this would.
#[cfg(test)]
fn last_safe_boundary(buf: &[u8], limit: usize, tail: bool) -> Option<usize> {
    let mut best: Option<usize> = None;
    for i in 1..limit.min(buf.len()) {
        if buf[i - 1] == b'\n' && !buf[i].is_ascii_whitespace() && !newline_held(buf, i - 1) {
            best = Some(i);
        }
    }
    if tail && limit >= buf.len() && buf.last() == Some(&b'\n') && !newline_held(buf, buf.len() - 1) {
        return Some(buf.len());
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::scan;
    use crate::parser::parse;

    #[test]
    fn one_scan_of_the_boundaries_answers_what_a_scan_per_limit_answers() {
        // `commit_bounded` reads the collected boundaries where it used to scan
        // the buffer again, and a disagreement between the two is a buffer cut
        // in the wrong place - which is the failure both reverted attempts on
        // this path made. Every limit, past the buffer's end as well, and both
        // tail flags, over inputs that put a newline after a quote its line
        // does not close, carry a string over a line by a backslash, hold a
        // newline in a char literal, end on a quote before the last newline,
        // and escape a quote inside a string.
        let cases: [&[u8]; 10] = [
            b"alpha\nbeta\ngamma\n",
            b"a \"quoted\nline\" b\nc\n",
            b"\"unclosed\nstill open\n",
            b"esc \"a\\\"b\nc\" d\ne\n",
            b"a \"carried \\\nover\" b\nc\n",
            b"c = '\n' ;\nnext\n",
            b"ends on a quote '\n",
            b"\n\n\n  \nx\n",
            b"no newline at all",
            b"",
        ];
        for buf in cases {
            let mut bounds = Vec::new();
            let in_quote = scan_boundaries(buf, &mut bounds);
            for limit in 0..=buf.len() + 2 {
                for tail in [false, true] {
                    assert_eq!(
                        boundary_below(buf, &bounds, in_quote, limit, tail),
                        last_safe_boundary(buf, limit, tail),
                        "{:?} at limit {limit}, tail {tail}",
                        String::from_utf8_lossy(buf)
                    );
                }
            }
        }
    }

    #[test]
    fn spectral_pattern_streams_equivalently() {
        // A spectral atom reads a field built over the whole buffer, so
        // draining a committed prefix moves the reading. The scanner must
        // defer every commit for such a pattern.
        let mut input = String::new();
        for i in 0..200 {
            input.push_str(&format!("fn f{i}(a,b){{let c=a+b;return c*2;}}\n"));
            input.push_str("the quick brown fox jumps over the lazy dog again and again\n");
        }
        for chunk in [64usize, 512, 4096] {
            assert_stream_equiv("\\F{texture:code}", &input, chunk);
        }
    }

    /// Feed `input` in fixed-size chunks and assert the streaming match
    /// set equals the whole-input scan, byte for byte.
    fn assert_stream_equiv(pattern_src: &str, input: &str, chunk: usize) {
        let pat = parse(pattern_src).expect("pattern parses");
        let whole = scan(&pat, input.as_bytes());
        let bytes = input.as_bytes();
        let chunks: Vec<&[u8]> = bytes.chunks(chunk.max(1)).collect();
        let streamed = scan_chunked(&pat, chunks);
        assert_eq!(
            streamed, whole,
            "pattern {pattern_src:?} chunk={chunk} differs from whole-input scan"
        );
    }

    #[test]
    fn committed_matches_drain_once_and_finish_returns_the_rest() {
        let pat = parse("\\N").expect("pattern parses");
        let whole = b"one 1 two 2\nthree 3 four 4\n";
        let mut s = StreamScanner::new(pat.clone());
        s.push(b"one 1 two 2\nthree 3 fo");
        let early = s.drain_committed();
        assert!(!early.is_empty(), "the line boundary settles the matches before it");
        assert!(s.drain_committed().is_empty(), "a drain hands each match over once");
        s.push(b"ur 4\n");
        let mut all = early;
        all.extend(s.drain_committed());
        all.extend(s.finish());
        assert_eq!(all, scan(&pat, whole));
    }

    /// A stream rebased between pushes reports its later matches from the new
    /// zero, and those plus the distance it moved are the whole input's.
    #[test]
    fn a_rebased_stream_finds_the_matches_the_whole_input_holds() {
        let pat = parse("\\N").expect("pattern parses");
        let whole = b"one 1 two 2\nthree 3 four 4\nfive 5\n";
        let mut s = StreamScanner::new(pat.clone());
        let mut all: Vec<(usize, usize)> = Vec::new();
        let mut origin = 0usize;
        for piece in whole.chunks(5) {
            s.push(piece);
            all.extend(s.drain_committed().into_iter().map(|m| (origin + m.start(), origin + m.end())));
            origin += s.rebase();
        }
        all.extend(s.finish().into_iter().map(|m| (origin + m.start(), origin + m.end())));
        let expected: Vec<(usize, usize)> = scan(&pat, whole).iter().map(|m| (m.start(), m.end())).collect();
        assert_eq!(all, expected);
    }

    /// Under a declared shape, a stream finds what a whole-input scan under
    /// the same shape finds, at every chunk size.
    #[test]
    fn a_stream_under_declared_shapes_finds_what_the_whole_scan_does() {
        let mut shapes = crate::custom::ShapeSet::new();
        shapes.declare("order = `[A-Z]{3}-[0-9]{4}`", crate::custom::Precedence::Before).expect("declare the shape");
        let pat = crate::parser::parse_with_shapes("\\{order}", &shapes).expect("pattern parses");
        let input = b"a ABC-1234 b\nXYZ-0007 c DEF-9000\nnone here\nQRS-0001\n";
        let expected = crate::engine::scan_with_shapes(&pat, input, &shapes);
        assert!(!expected.is_empty());
        for chunk in [1usize, 2, 3, 7, 64] {
            let mut s = StreamScanner::with_shapes(pat.clone(), shapes.clone());
            let mut got = Vec::new();
            for piece in input.chunks(chunk) {
                s.push(piece);
                got.extend(s.drain_committed());
            }
            got.extend(s.finish());
            assert_eq!(got, expected, "chunk {chunk}");
        }
    }

    #[test]
    fn metamorphic_equivalence_across_patterns_and_chunk_sizes() {
        let cases: &[(&str, &str)] = &[
            ("\\N \\W", "weight 12 kg\nlen 5 m\nmass 9 g\n"),
            ("\\W:x =x", "the the cat\ndog dog ran\nfoo bar baz\n"),
            ("<\\W:t>.*</=t>", "<a>x</a>\n<b>yy</b>\n<c>z</c>\n"),
            (". ~\"END\"", "begin here END\nmore lines END now\nlast END\n"),
            ("\\W\\B(.*)", "call f(g(x))\nrun h(k(y))\ntail\n"),
            (".*", "anything at all\ngoes here\n"),
            ("@2 \\W", "a, hello, c\nd, world, f\n"),
            ("\\I", "10.0.0.1 host\n192.168.1.1 ok\nfe80::1 v6\n"),
        ];
        for (pat, input) in cases {
            for chunk in [1usize, 2, 3, 5, 7, 13, 64, 1000] {
                assert_stream_equiv(pat, input, chunk);
            }
        }
    }

    /// Strings and char literals at every chunk size stream to the whole
    /// input's matches, with LF and with CRLF line endings: a quote its line
    /// does not close, a string carried over a line by a backslash, a char
    /// literal holding the quote the other kind would open with and one holding
    /// a newline, and a line ending on a quote.
    #[test]
    fn quoted_tokens_stream_as_the_whole_input_reads_them() {
        let input = concat!(
            "say \"no close on this line\n",
            "and \"a string\" here\n",
            "let s = \"carried \\\nover\" ;\n",
            "c = '\"' ; d = \"x\"\n",
            "e = '\n' ; f = \"y\"\n",
            "ends on a quote '\n",
            "'last' \"line\"\n",
        );
        let crlf = input.replace('\n', "\r\n");
        for text in [input, crlf.as_str()] {
            for pattern in ["\\Q", "\\W", "\\Q \\W"] {
                for chunk in [1usize, 2, 3, 5, 7, 11, 16, 64] {
                    assert_stream_equiv(pattern, text, chunk);
                }
            }
        }
    }

    #[test]
    fn guarded_pattern_with_forward_literal_across_a_seam() {
        // The guard literal lands in a later chunk than the match start;
        // a scanner that committed early would wrongly reject the match.
        let input = "alpha\nbeta\nthe needle is END\n";
        for chunk in [1usize, 3, 6, 9, 20] {
            assert_stream_equiv(". ~\"END\"", input, chunk);
        }
    }

    #[test]
    fn a_required_literal_arriving_late_still_finds_its_match() {
        // While a byte string every match must contain is absent, the scanner
        // skips the lex and keeps every byte. A match must contain that string
        // but need not begin at it, so one can start among the retained bytes
        // and reach a literal that arrives chunks later - and a scanner that
        // dropped bytes on the literal's own length would lose exactly those.
        // Small chunks put the literal's own bytes across a join as well.
        let input = "aaa bbb ccc\nddd eee fff\nneedle tail\nggg needle more\n";
        for chunk in [1usize, 2, 3, 7, 16, 64] {
            assert_stream_equiv("\"needle\" \\W", input, chunk);
        }
        // And where it never arrives, the stream must agree that there is
        // nothing: the skip must not invent a match any more than lose one.
        for chunk in [1usize, 3, 16] {
            assert_stream_equiv("\"needle\" \\W", "aaa bbb\nccc ddd\neee fff\n", chunk);
        }
    }

    #[test]
    fn match_spanning_many_chunks_is_not_split() {
        // A single `.*` match runs the length of a line that is far
        // longer than the chunk size; it must not be cut into pieces.
        let line = "x ".repeat(200);
        let input = format!("{line}\n{line}\n");
        for chunk in [1usize, 4, 16, 64] {
            assert_stream_equiv(".*", &input, chunk);
        }
    }

    #[test]
    fn no_newline_input_still_equivalent() {
        // No safe boundary exists, so everything retains until finish;
        // the result must still match the whole-input scan.
        assert_stream_equiv("\\W:x =x", "the the cat dog dog", 3);
    }
}
