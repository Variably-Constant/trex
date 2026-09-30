//! Single-pass NFA simulation (a Pike-style virtual machine) over the
//! token stream.
//!
//! This is the worst-case-linear matching engine. A pattern compiles
//! to a small instruction program; the scan then sweeps the
//! significant-token subsequence once, carrying a priority-ordered set
//! of threads deduplicated by program counter at each position. Each
//! `(program counter, position)` pair is reached at most once per
//! step, so a match runs in time linear in the token count for a fixed
//! pattern: no per-start recompute, and the per-thread state is a small
//! array of token indices rather than a cloned map.
//!
//! Whitespace never appears in the simulation: the machine runs over
//! the indices of the significant (non-whitespace) tokens, so an atom
//! always consumes the next real token and the engine carries no
//! whitespace-skipping logic.
//!
//! A pattern that contains a balanced-bracket group or a field node
//! returns `None` from [`scan_nfa`]; the caller routes those to the
//! set-reachability engine, whose variable-length advance handles the
//! span a balanced group consumes.

use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};

use crate::ast::{Atom, Greed, Pattern};
use crate::engine::{Match, Span, byte_class_matches, register_eq_matches};
use crate::lexer::Significant;
use crate::token::{Token, TokenKind};

/// The significant token stream an engine reads: each token's kind and byte
/// span by significant index, in whichever form the lexer left them.
///
/// Implemented here for the stitched token vector, for the lexer's chunk
/// parts and for a split's kind codes and spans. A caller holding its tokens
/// in some other shape implements this and runs the engine over them through
/// [`anchor_ends_into`] or [`walk_prefix_into`] without building a stream of
/// the crate's own.
pub trait SigStream {
    /// How many significant tokens there are.
    fn len(&self) -> usize;
    /// Whether there are none.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The kind of token `k`.
    fn kind(&self, k: usize) -> TokenKind;
    /// The byte span of token `k`.
    fn span(&self, k: usize) -> (usize, usize);
    /// The longest run of consecutive tokens of `kind`, or `None` once a run
    /// passes `ceiling`, past which the rest of the stream is not read.
    ///
    /// Read a token at a time through [`Self::kind`] here. A stream holding
    /// its kind codes in slices walks those instead, which is the same answer
    /// at a fraction of the cost: the run is what the dispatch's bound on a
    /// match is read from, once a scan, over every token.
    fn longest_run(&self, kind: TokenKind, ceiling: usize) -> Option<usize> {
        let (mut best, mut run) = (0usize, 0usize);
        for k in 0..self.len() {
            if self.kind(k) == kind {
                run += 1;
                if run > best {
                    best = run;
                    if best > ceiling {
                        return None;
                    }
                }
            } else {
                run = 0;
            }
        }
        Some(best)
    }
    /// The first token starting at or after byte `at`, or the stream's length
    /// where none does.
    ///
    /// Token starts ascend, so this is a partition over them, read a token at
    /// a time through [`Self::span`] here. A stream holding its spans in
    /// slices partitions those, part by part: a probe here over the lexer's
    /// parts is a search for the part before the span is read, and a walk
    /// mapping thousands of byte hits onto anchors pays that at every probe.
    fn first_at_or_after(&self, at: usize) -> usize {
        let (mut lo, mut hi) = (0usize, self.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.span(mid).0 < at {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

impl<S: SigStream + ?Sized> SigStream for &S {
    #[inline]
    fn len(&self) -> usize {
        (**self).len()
    }
    #[inline]
    fn kind(&self, k: usize) -> TokenKind {
        (**self).kind(k)
    }
    #[inline]
    fn span(&self, k: usize) -> (usize, usize) {
        (**self).span(k)
    }
    #[inline]
    fn longest_run(&self, kind: TokenKind, ceiling: usize) -> Option<usize> {
        (**self).longest_run(kind, ceiling)
    }
    #[inline]
    fn first_at_or_after(&self, at: usize) -> usize {
        (**self).first_at_or_after(at)
    }
}

/// [`SigStream::first_at_or_after`] over the parts of a stream: the part is
/// found by a partition over the parts' first token starts, then the token by
/// a partition over that part's spans, each a slice read in place. A part
/// holding no token has no first start and reads as the next part that has
/// one, which keeps the partition over the parts monotone.
fn first_at_or_after_over_parts(parts: &[Significant], bases: &[usize], at: usize) -> usize {
    let (mut lo, mut hi) = (0usize, parts.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let mut q = mid;
        while q < parts.len() && parts[q].spans.is_empty() {
            q += 1;
        }
        if q == parts.len() || (parts[q].spans[0].0 as usize) >= at {
            hi = mid;
        } else {
            lo = q + 1;
        }
    }
    // `lo` is the first part whose first token starts at or after `at`, or
    // the count. The part before it, when there is one, holds a token and
    // starts before `at`, so the answer is inside it unless every token of
    // it starts before `at`; then it is the first token of the first part
    // from `lo` on that holds one.
    if lo > 0 {
        let prev = &parts[lo - 1];
        let i = prev.spans.partition_point(|&(s, _)| (s as usize) < at);
        if i < prev.spans.len() {
            return bases[lo - 1] + i;
        }
    }
    let mut q = lo;
    while q < parts.len() && parts[q].spans.is_empty() {
        q += 1;
    }
    bases[q]
}

/// The longest run of tokens coded `code` in `kinds`, continuing the `run`
/// of them that ended just before the slice and the `best` seen so far, or
/// `false` once a run passes `ceiling`.
///
/// Reads the codes where they lie. The run is a select and the best a max, so
/// a token's own kind costs no branch; the ceiling is the one branch, and it
/// is taken once at most.
#[inline]
fn longest_run_in(kinds: &[u32], code: u32, ceiling: usize, run: &mut usize, best: &mut usize) -> bool {
    let (mut r, mut b) = (*run, *best);
    for &k in kinds {
        r = if k == code { r + 1 } else { 0 };
        b = b.max(r);
        if b > ceiling {
            return false;
        }
    }
    *run = r;
    *best = b;
    true
}

/// [`longest_run_in`] over the parts of a stream in order, a run crossing from
/// one part into the next as it does in the stream they hold.
fn longest_run_over_parts(parts: &[Significant], kind: TokenKind, ceiling: usize) -> Option<usize> {
    let code = kind.code();
    let (mut run, mut best) = (0usize, 0usize);
    for part in parts {
        if !longest_run_in(&part.kinds, code, ceiling, &mut run, &mut best) {
            return None;
        }
    }
    Some(best)
}

/// A stream the per-anchor dispatch runs over, and the reader one leaf of that
/// dispatch holds.
///
/// A leaf takes a contiguous run of anchors and every attempt inside it steps
/// forward from its own, so a reader that remembers where it last read answers
/// the next read with a compare where the shared form answers it with a search
/// over every part. That reader belongs to one leaf, so it is free to be
/// `!Sync`, which is what lets it carry the position at all.
///
/// A stream a search costs nothing over reads as itself.
pub(crate) trait LeafRead: SigStream + Sync {
    /// What one leaf of the dispatch reads through.
    type Leaf<'a>: SigStream
    where
        Self: 'a;

    /// A reader positioned at the start of the stream.
    fn leaf(&self) -> Self::Leaf<'_>;
}

/// The stitched form: every token in one vector, and the indices of the
/// significant ones.
pub(crate) struct Stitched<'a> {
    toks: &'a [Token],
    sig: &'a [usize],
}

impl<'a> Stitched<'a> {
    /// A stream over tokens and the indices of the significant ones, both owned
    /// by the caller.
    ///
    /// For a caller running several walks over one lex - a pattern set asks its
    /// members the same input - which wants the stream borrowed rather than
    /// owned, so the lex is paid once and no member copies it.
    pub(crate) fn new(toks: &'a [Token], sig: &'a [usize]) -> Self {
        Stitched { toks, sig }
    }

    /// The indices of the significant tokens of `toks`, which [`Self::new`]
    /// borrows.
    pub(crate) fn significant_of(toks: &[Token]) -> Vec<usize> {
        (0..toks.len()).filter(|&i| toks[i].kind != TokenKind::Whitespace).collect()
    }
}

impl SigStream for Stitched<'_> {
    #[inline]
    fn len(&self) -> usize {
        self.sig.len()
    }
    #[inline]
    fn kind(&self, k: usize) -> TokenKind {
        self.toks[self.sig[k]].kind
    }
    #[inline]
    fn span(&self, k: usize) -> (usize, usize) {
        let t = &self.toks[self.sig[k]];
        (t.start(), t.end())
    }
}

impl<'a> LeafRead for Stitched<'a> {
    // Two index reads answer any token here, so a leaf reads the stream itself.
    type Leaf<'l>
        = &'l Stitched<'a>
    where
        Self: 'l;

    #[inline]
    fn leaf(&self) -> &Stitched<'a> {
        self
    }
}

/// The parts form: the significant stream one part per lexer chunk, read
/// where the chunks were lexed with no stream stitched. A token is found by a
/// search over the parts' starting indices, so this is the shared form, which
/// is `Sync` and carries no position. Every walk over it - the serial one and
/// each leaf of the parallel scan - reads through a [`PartsCursor`] of its own,
/// which steps between parts as its index does.
pub(crate) struct Parts<'a> {
    parts: &'a [Significant],
    /// The significant index each part begins at, then the total.
    bases: &'a [usize],
}

impl<'a> Parts<'a> {
    /// The index each part begins at, then the total, which [`Self::new`]
    /// borrows.
    ///
    /// Held by the caller rather than by the stream so that a caller owning its
    /// parts can own these beside them: a stream that owned its bases and were
    /// stored next to the parts it borrows would be self-referential, and one
    /// built per call would pay a push a part on every step of a walk.
    pub(crate) fn bases_of(parts: &[Significant]) -> Vec<usize> {
        let mut bases = Vec::with_capacity(parts.len() + 1);
        let mut total = 0;
        for part in parts {
            bases.push(total);
            total += part.kinds.len();
        }
        bases.push(total);
        bases
    }

    pub(crate) fn new(parts: &'a [Significant], bases: &'a [usize]) -> Self {
        debug_assert_eq!(bases.len(), parts.len() + 1, "one base a part, then the total");
        Parts { parts, bases }
    }

    /// The part holding token `k` and the token's index in it.
    #[inline]
    fn locate(&self, k: usize) -> (usize, usize) {
        let p = self.bases.partition_point(|&b| b <= k) - 1;
        (p, k - self.bases[p])
    }

    /// Whether `k` lies in part `p`.
    #[inline]
    fn holds(&self, p: usize, k: usize) -> bool {
        p + 1 < self.bases.len() && self.bases[p] <= k && k < self.bases[p + 1]
    }

    fn cursor(&self) -> PartsCursor<'_> {
        PartsCursor { parts: self, at: Cell::new(0) }
    }
}

impl<'a> LeafRead for Parts<'a> {
    // A token is found here by a search over every part's starting index, and a
    // leaf reads ascending, so it reads through the part it last read from.
    type Leaf<'l>
        = PartsCursor<'l>
    where
        Self: 'l;

    #[inline]
    fn leaf(&self) -> PartsCursor<'_> {
        self.cursor()
    }
}

impl SigStream for Parts<'_> {
    #[inline]
    fn len(&self) -> usize {
        self.bases[self.bases.len() - 1]
    }
    #[inline]
    fn kind(&self, k: usize) -> TokenKind {
        let (p, i) = self.locate(k);
        TokenKind::from_code(self.parts[p].kinds[i])
    }
    #[inline]
    fn span(&self, k: usize) -> (usize, usize) {
        let (p, i) = self.locate(k);
        let (a, b) = self.parts[p].spans[i];
        (a as usize, b as usize)
    }
    fn longest_run(&self, kind: TokenKind, ceiling: usize) -> Option<usize> {
        longest_run_over_parts(self.parts, kind, ceiling)
    }
    fn first_at_or_after(&self, at: usize) -> usize {
        first_at_or_after_over_parts(self.parts, self.bases, at)
    }
}

/// [`Parts`] read through the part last read from, so a walk that steps
/// one token at a time searches only where it crosses into the next part.
pub(crate) struct PartsCursor<'a> {
    parts: &'a Parts<'a>,
    at: Cell<usize>,
}

impl PartsCursor<'_> {
    #[inline]
    fn locate(&self, k: usize) -> (usize, usize) {
        let mut p = self.at.get();
        if !self.parts.holds(p, k) {
            p = self.parts.locate(k).0;
            self.at.set(p);
        }
        (p, k - self.parts.bases[p])
    }
}

impl SigStream for PartsCursor<'_> {
    #[inline]
    fn len(&self) -> usize {
        self.parts.len()
    }
    #[inline]
    fn kind(&self, k: usize) -> TokenKind {
        let (p, i) = self.locate(k);
        TokenKind::from_code(self.parts.parts[p].kinds[i])
    }
    #[inline]
    fn span(&self, k: usize) -> (usize, usize) {
        let (p, i) = self.locate(k);
        let (a, b) = self.parts.parts[p].spans[i];
        (a as usize, b as usize)
    }
    fn longest_run(&self, kind: TokenKind, ceiling: usize) -> Option<usize> {
        self.parts.longest_run(kind, ceiling)
    }
    fn first_at_or_after(&self, at: usize) -> usize {
        self.parts.first_at_or_after(at)
    }
}

/// The parts form with the lexer's workspace owned rather than borrowed, for a
/// walk that hands matches out for longer than any scope can lend them.
///
/// [`Parts`] borrows both the parts and their bases, which suits a caller
/// running inside the closure that lends them. A cursor outlives that closure,
/// so it holds this instead: the workspace moved to it by
/// [`crate::parallel_lex::lex_significant_parts_owned`] and returned when this
/// drops, with the bases beside the parts rather than inside a stream that
/// would then be self-referential.
///
/// The part last read from is cached, as [`PartsCursor`] caches it and for the
/// same reason: a walk steps one token at a time and should search only where
/// it crosses into the next part.
pub(crate) struct OwnedStream {
    parts: crate::parallel_lex::OwnedParts,
    bases: Vec<usize>,
    at: Cell<usize>,
}

impl OwnedStream {
    /// Lex `input`'s significant stream and hold it, taking this thread's
    /// workspace until this drops.
    pub(crate) fn over(input: &[u8]) -> Self {
        let parts = crate::parallel_lex::lex_significant_parts_owned(input);
        let bases = Parts::bases_of(parts.parts());
        OwnedStream { parts, bases, at: Cell::new(0) }
    }

    /// The form a leaf of the per-anchor dispatch reads: two borrowed slices,
    /// and so `Sync`, where this stream's own cache is not.
    pub(crate) fn parts_view(&self) -> Parts<'_> {
        Parts::new(self.parts.parts(), &self.bases)
    }

    #[inline]
    fn locate(&self, k: usize) -> (usize, usize) {
        let mut p = self.at.get();
        if !(p + 1 < self.bases.len() && self.bases[p] <= k && k < self.bases[p + 1]) {
            p = self.bases.partition_point(|&b| b <= k) - 1;
            self.at.set(p);
        }
        (p, k - self.bases[p])
    }
}

impl SigStream for OwnedStream {
    #[inline]
    fn len(&self) -> usize {
        self.bases[self.bases.len() - 1]
    }
    #[inline]
    fn kind(&self, k: usize) -> TokenKind {
        let (p, i) = self.locate(k);
        TokenKind::from_code(self.parts.parts()[p].kinds[i])
    }
    #[inline]
    fn span(&self, k: usize) -> (usize, usize) {
        let (p, i) = self.locate(k);
        let (a, b) = self.parts.parts()[p].spans[i];
        (a as usize, b as usize)
    }
    fn longest_run(&self, kind: TokenKind, ceiling: usize) -> Option<usize> {
        longest_run_over_parts(self.parts.parts(), kind, ceiling)
    }
    fn first_at_or_after(&self, at: usize) -> usize {
        first_at_or_after_over_parts(self.parts.parts(), &self.bases, at)
    }
}

/// A stream a walk over it can put across cores.
///
/// A [`SigStream`] is read one token at a time and may cache where it last
/// read, which a leaf of the per-anchor dispatch cannot share. This hands back
/// the form that can be shared, and `None` where the stream has none, in which
/// case every walk over it runs on one thread.
pub(crate) trait Shardable: SigStream {
    fn shard(&self) -> Option<Parts<'_>> {
        None
    }
}

impl Shardable for Stitched<'_> {}

impl Shardable for OwnedStream {
    fn shard(&self) -> Option<Parts<'_>> {
        Some(self.parts_view())
    }
}

/// The bytes of token `k` of `s`.
#[inline]
fn text_of<'i, S: SigStream + ?Sized>(input: &'i [u8], s: &S, k: usize) -> &'i [u8] {
    let (a, b) = s.span(k);
    &input[a..b]
}

/// The bytes from the start of token `ks` to the end of token `ke - 1`.
#[inline]
fn bytes_between<'i, S: SigStream + ?Sized>(input: &'i [u8], s: &S, ks: usize, ke: usize) -> &'i [u8] {
    &input[s.span(ks).0..s.span(ke - 1).1]
}

/// The byte span from the start of token `ks` to the end of token `ke - 1`.
#[inline]
fn span_between<S: SigStream + ?Sized>(s: &S, ks: usize, ke: usize) -> Span {
    Span { start: s.span(ks).0 as u32, end: s.span(ke - 1).1 as u32 }
}

/// A compiled instruction. Save slots hold significant-token indices;
/// slots 0 and 1 are the whole-match start and end, and each register
/// name owns the next free even/odd pair.
#[derive(Clone, Debug, PartialEq)]
enum Inst {
    /// Consume one significant token matching the atom, advancing to
    /// `pc + 1`.
    Atom(Atom),
    /// Epsilon fork; `.0` is the higher-priority branch.
    ///
    /// A quantifier's loop head is a split like any other. A thread that
    /// comes back to one it already reached at this position is a duplicate
    /// and is dropped; the compiler lays a loop out so that the drop loses
    /// nothing, see [`Compiler::emit_star`].
    Split(usize, usize),
    /// Epsilon jump.
    Jmp(usize),
    /// Epsilon: record the current position into a save slot.
    Save(usize),
    /// Epsilon zero-width assertion on a literal ahead. The flag is the
    /// negation: `false` requires the literal to occur, `true` requires it
    /// to be absent (`!~"lit"`).
    Guard(Vec<u8>, bool),
    /// Epsilon zero-width assertion on where the position stands: `^`, `$`,
    /// `\A`, `\z`. Only the positional anchors compile here; the ones reading
    /// an axis field route to the set-reachability engine.
    Anchor(crate::ast::AnchorKind),
    /// Accept.
    Match,
}

/// A compiled pattern.
struct Program {
    insts: Vec<Inst>,
    /// Number of save slots (2 for the whole match plus 2 per register).
    nslots: usize,
    /// Register name to its base save slot (the start; base + 1 is end).
    slots: BTreeMap<String, usize>,
    /// The register names in `slots` order, held once so a match carries a
    /// reference count rather than a copy of them.
    names: std::sync::Arc<[String]>,
    /// Whether the pattern reads a register back, as `=name` does.
    ///
    /// This decides whether the thread list may dedup by counter alone. Two
    /// threads at one counter differ only in what they have bound; where
    /// nothing reads a binding, their futures are identical and the
    /// higher-priority one settles which captures are reported, so the counter
    /// is the whole key. Where a back-reference reads one, the bindings decide
    /// what each thread matches next, and two such threads are genuinely
    /// different runs that both have to be carried.
    ///
    /// Binding alone does not need it. A pattern that binds and never reads
    /// pays a hash of its whole save array per thread per step for a
    /// distinction nothing consumes.
    reads_registers: bool,
    /// The clock the typed predicates read, captured when the program is
    /// built and refreshed where a held program begins another scan.
    clock: crate::typed::ClockCell,
}

/// Whether a pattern needs the set-reachability engine because it has a
/// balanced group (variable-length advance) or a field node.
pub(crate) fn needs_set_engine(pat: &Pattern) -> bool {
    needs_set_engine_unless(pat, false)
}

/// [`needs_set_engine`], with a spectral atom counted as at home when
/// `spectral_ok`: the device kernel carries a pooled spectral reading per
/// token, so a program compiled for it may hold the atom this engine cannot
/// evaluate.
fn needs_set_engine_unless(pat: &Pattern, spectral_ok: bool) -> bool {
    let needs = |p: &Pattern| needs_set_engine_unless(p, spectral_ok);
    match pat {
        // A balanced or field node needs the set engine: a variable-length
        // advance, or a comma-field count. An edit-distance group is the same
        // shape - it offers a run of several lengths at once, each with its
        // own preference, and this engine has no way to rank them.
        Pattern::Balanced(..) | Pattern::Field(..) | Pattern::Within(..) => true,
        // A positional anchor is a function of the tokens, the bytes and the
        // position, so it compiles to an epsilon here. The property anchors
        // each read a field built once per scan, which only the set engine
        // carries, so those still route away.
        Pattern::Anchor(k) => !matches!(
            k,
            crate::ast::AnchorKind::LineStart
                | crate::ast::AnchorKind::LineEnd
                | crate::ast::AnchorKind::InputStart
                | crate::ast::AnchorKind::InputEnd
                | crate::ast::AnchorKind::Resume
                | crate::ast::AnchorKind::ResetStart
        ),
        // An explicit whitespace atom (`\S`, or the `\s` byte class) needs the
        // set engine too: this engine runs over the significant-token
        // subsequence and never sees a whitespace token at all.
        Pattern::Atom(a) => atom_needs_set_engine(a, spectral_ok),
        // A zero-width sub-pattern probe has no instruction form here; the set
        // engine runs the sub-pattern and filters instead.
        Pattern::Assert(..) => true,
        // Atomic discards the lengths the body did not prefer. This engine's
        // priority order expresses which length is preferred and has no way
        // to withdraw the others once they are queued, which is the same
        // reason `|>` routes away.
        Pattern::Atomic(..) => true,
        Pattern::Empty | Pattern::Guard(..) => false,
        Pattern::Star(p, _) | Pattern::Plus(p, _) | Pattern::Opt(p, _) | Pattern::Bind(_, _, p) => needs(p),
        Pattern::Repeat(p, _, _, _) => needs(p),
        // This engine's thread priority already expresses leftmost-first, so
        // `|` compiles here directly and needs nothing added. The other two
        // modes are global properties of the
        // scan rather than of one node - `||` would have to stop cutting
        // lower-priority threads on a match, `|>` would need a commit
        // instruction - and a pattern may mix them, so they route to the set
        // engine, which decides per node.
        Pattern::Alt(v, mode) => *mode != crate::ast::AltMode::First || v.iter().any(needs),
        Pattern::Concat(v) => v.iter().any(needs),
    }
}

/// Whether an atom needs the set engine: a spectral predicate reads a
/// precomputed axis field this engine does not carry (unless `spectral_ok`
/// says the caller carries one), and a whitespace atom asks for a token this
/// engine's significant-token subsequence never contains. A class needs it
/// when any member does.
fn atom_needs_set_engine(a: &Atom, spectral_ok: bool) -> bool {
    match a {
        Atom::Spectral(_) => !spectral_ok,
        Atom::Kind(TokenKind::Whitespace) | Atom::Byte(crate::ast::ByteClass::Space) => true,
        // A relative magnitude predicate reads a context fold this engine
        // does not carry; an absolute one is a function of the token alone.
        Atom::Magnitude(p) | Atom::KindMag(_, p) => p.scope().is_some(),
        // A timestamp read against the one a register holds needs both spans
        // resolved as instants, which the set engine's environment carries.
        Atom::Since(..) => true,
        // Kinship is read off the input's pair field, which the set engine
        // builds once per scan and this engine does not carry.
        Atom::RegisterKin(..) => true,
        Atom::Class(c) => c.members().any(|m| atom_needs_set_engine(m, spectral_ok)),
        _ => false,
    }
}

/// Whether the backtracking empty-loop reading needs the set engine for this
/// pattern: whether it holds a repetition whose body can match without
/// consuming a token.
///
/// That reading takes an empty iteration and then BREAKS the loop, which means
/// withdrawing the re-entry this engine has already queued by the time the
/// body's descent reveals the body matched empty. Preference here is expressed
/// by the order threads are added and there is no way to take one back - the
/// same limitation that sends [`Pattern::Atomic`] and `|>` to the set engine,
/// which carries each derivation's path explicitly and can compare them.
///
/// Only a nullable body is affected. Everywhere else the two readings agree
/// instruction for instruction, so a pattern that names the backtracking
/// reading and contains no nullable loop stays on this engine.
pub(crate) fn empty_loop_needs_set_engine(pat: &Pattern) -> bool {
    match pat {
        Pattern::Star(p, _) | Pattern::Plus(p, _) => {
            nullable(p) || empty_loop_needs_set_engine(p)
        }
        Pattern::Repeat(p, _, hi, _) => {
            (hi.is_none() && nullable(p)) || empty_loop_needs_set_engine(p)
        }
        Pattern::Opt(p, _)
        | Pattern::Bind(_, _, p)
        | Pattern::Atomic(p)
        | Pattern::Field(_, p)
        | Pattern::Balanced(_, p)
        | Pattern::Assert(p, _, _) => empty_loop_needs_set_engine(p),
        Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(empty_loop_needs_set_engine),
        Pattern::Empty
        | Pattern::Atom(_)
        | Pattern::Within(..)
        | Pattern::Guard(..)
        | Pattern::Anchor(_) => false,
    }
}

/// The split a quantifier compiles to, with the preferred branch first.
///
/// This is the whole of laziness on this engine: `Split.0` is the
/// higher-priority thread, so a greedy quantifier prefers to enter the body
/// and a lazy one prefers to leave it, and the existing priority-ordered
/// thread list carries the rest.
fn lean(g: Greed, body: usize, exit: usize) -> Inst {
    match g {
        Greed::Greedy => Inst::Split(body, exit),
        Greed::Lazy => Inst::Split(exit, body),
    }
}

/// Whether a pattern can match without consuming a token. A zero-width
/// node can, an atom cannot, and the rest follow from their parts. Decides
/// a star's layout; see [`Compiler::emit_star`].
fn nullable(pat: &Pattern) -> bool {
    match pat {
        Pattern::Empty | Pattern::Guard(..) | Pattern::Anchor(_) | Pattern::Assert(..) => true,
        // An edit-distance group always consumes a token: the empty run is
        // not offered even where every atom could be deleted.
        Pattern::Atom(_) | Pattern::Balanced(..) | Pattern::Within(..) => false,
        Pattern::Bind(_, _, p) | Pattern::Atomic(p) | Pattern::Field(_, p) | Pattern::Plus(p, _) => {
            nullable(p)
        }
        Pattern::Concat(v) => v.iter().all(nullable),
        Pattern::Alt(v, _) => v.iter().any(nullable),
        Pattern::Star(..) | Pattern::Opt(..) => true,
        Pattern::Repeat(p, m, _, _) => *m == 0 || nullable(p),
    }
}

struct Compiler {
    insts: Vec<Inst>,
    slots: BTreeMap<String, usize>,
}

impl Compiler {
    fn here(&self) -> usize {
        self.insts.len()
    }

    fn push(&mut self, inst: Inst) -> usize {
        let at = self.here();
        self.insts.push(inst);
        at
    }

    fn slot_of(&mut self, name: &str) -> usize {
        if let Some(&base) = self.slots.get(name) {
            return base;
        }
        let base = 2 + self.slots.len() * 2;
        self.slots.insert(name.to_string(), base);
        base
    }

    fn emit(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Empty => {}
            Pattern::Atom(a) => {
                self.push(Inst::Atom(a.clone()));
            }
            // `needs_set_engine` routes an edit-distance group away before
            // compilation, so reaching it here would be a routing fault
            // rather than a pattern this engine could run.
            Pattern::Within(..) => unreachable!("an edit-distance group routes to the set engine"),
            Pattern::Guard(lit, neg) => {
                self.push(Inst::Guard(lit.as_bytes().to_vec(), *neg));
            }
            Pattern::Concat(parts) => {
                for p in parts {
                    self.emit(p);
                }
            }
            // Only `AltMode::First` reaches here; `needs_set_engine` routed
            // the others away before compilation.
            Pattern::Alt(parts, _) => self.emit_alt(parts),
            Pattern::Star(p, g) => self.emit_star(p, *g),
            Pattern::Plus(p, g) => self.emit_plus(p, *g),
            Pattern::Opt(p, g) => {
                let split = self.push(Inst::Split(0, 0));
                let body = self.here();
                self.emit(p);
                let exit = self.here();
                self.insts[split] = lean(*g, body, exit);
            }
            Pattern::Repeat(p, m, n, g) => self.emit_repeat(p, *m, *n, *g),
            // The scope flag matters only inside a balanced group, which routes
            // to the set engine; the linear engine has no group context, so a
            // scoped bind behaves like a plain one here.
            Pattern::Bind(name, _, p) => {
                let base = self.slot_of(name);
                self.push(Inst::Save(base));
                self.emit(p);
                self.push(Inst::Save(base + 1));
            }
            // `\K` moves where the match is reported to start, and slot 0 is
            // exactly that, so it writes the slot again rather than asserting
            // anything. Every other anchor is a predicate on the position.
            Pattern::Anchor(crate::ast::AnchorKind::ResetStart) => {
                self.push(Inst::Save(0));
            }
            // Only the positional anchors reach here; `needs_set_engine`
            // routed the field-reading ones away.
            Pattern::Anchor(k) => {
                self.push(Inst::Anchor(k.clone()));
            }
            // Routed to the set-reachability engine before compilation.
            Pattern::Balanced(..)
            | Pattern::Field(..)
            | Pattern::Assert(..)
            | Pattern::Atomic(..) => unreachable!(),
        }
    }

    fn emit_alt(&mut self, alts: &[Pattern]) {
        if alts.len() == 1 {
            self.emit(&alts[0]);
            return;
        }
        let split = self.push(Inst::Split(0, 0));
        let first = self.here();
        self.emit(&alts[0]);
        let jmp = self.push(Inst::Jmp(0));
        let rest = self.here();
        self.emit_alt(&alts[1..]);
        let end = self.here();
        self.insts[split] = Inst::Split(first, rest);
        self.insts[jmp] = Inst::Jmp(end);
    }

    /// `P*`, in one of two layouts by whether `P` can match empty.
    ///
    /// A body that always consumes gets one split at the head: the body,
    /// then a jump back to the split, which is reached again only at a later
    /// position. A body that can match empty would come back to that head
    /// within the same position, where it is a duplicate and dropped, and
    /// the exit it was about to take goes with it, leaving the body's
    /// consuming derivations ranked above an exit a regular expression
    /// prefers. So that star is laid out as `(P+)?`: a split into the body
    /// or past it, the body, then a second split back into the body or out.
    /// A path that matched the body empty reaches the second split fresh and
    /// leaves through it at the priority it arrived with; only its attempt
    /// to re-enter the body is the duplicate. This is the layout the `regex`
    /// crate gives such a star, so the two rank the same derivations the
    /// same way.
    fn emit_star(&mut self, p: &Pattern, g: Greed) {
        let head = self.push(Inst::Split(0, 0));
        let body = self.here();
        self.emit(p);
        if nullable(p) {
            let back = self.push(Inst::Split(0, 0));
            let exit = self.here();
            self.insts[head] = lean(g, body, exit);
            self.insts[back] = lean(g, body, exit);
        } else {
            self.push(Inst::Jmp(head));
            let exit = self.here();
            self.insts[head] = lean(g, body, exit);
        }
    }

    /// `P+`: the body, then a split back into it or out. The body's first
    /// run is the loop's own, so a body that can match empty needs no second
    /// layout: an empty run reaches the split fresh and leaves through it.
    fn emit_plus(&mut self, p: &Pattern, g: Greed) {
        let body = self.here();
        self.emit(p);
        let back = self.push(Inst::Split(0, 0));
        let exit = self.here();
        self.insts[back] = lean(g, body, exit);
    }

    fn emit_repeat(&mut self, p: &Pattern, m: usize, n: Option<usize>, g: Greed) {
        match n {
            // `m` or more. With no mandatory copy this is a star; otherwise
            // the last mandatory copy is the loop's own body, as in `P+`, so
            // `P{1,}` and `P+` are the same instructions and rank the same
            // derivations the same way.
            None => {
                if m == 0 {
                    self.emit_star(p, g);
                } else {
                    for _ in 1..m {
                        self.emit(p);
                    }
                    self.emit_plus(p, g);
                }
            }
            Some(nn) => {
                for _ in 0..m {
                    self.emit(p);
                }
                // Up to `nn - m` further optional copies, each able to
                // skip the rest by jumping to the end.
                let mut splits = Vec::new();
                for _ in m..nn {
                    let split = self.push(Inst::Split(0, 0));
                    splits.push(split);
                    let body = self.here();
                    self.emit(p);
                    self.insts[split] = lean(g, body, 0);
                }
                let end = self.here();
                // The exit target is backpatched into whichever arm `lean`
                // left for it: greedy put the body first, lazy put it second.
                for split in splits {
                    if let Inst::Split(a, b) = self.insts[split] {
                        self.insts[split] = match g {
                            Greed::Greedy => Inst::Split(a, end),
                            Greed::Lazy => Inst::Split(end, b),
                        };
                    }
                }
            }
        }
    }
}

fn compile(pat: &Pattern) -> Option<Program> {
    compile_unless(pat, false)
}

/// [`compile`], keeping a spectral atom as an instruction when `spectral_ok`;
/// only a runner that carries a spectral reading per token may run the
/// result.
fn compile_unless(pat: &Pattern, spectral_ok: bool) -> Option<Program> {
    if needs_set_engine_unless(pat, spectral_ok) {
        return None;
    }
    let mut c = Compiler { insts: Vec::new(), slots: BTreeMap::new() };
    c.push(Inst::Save(0));
    c.emit(pat);
    c.push(Inst::Save(1));
    c.push(Inst::Match);
    let nslots = 2 + c.slots.len() * 2;
    let reads_registers = pat.reads_registers();
    let names: std::sync::Arc<[String]> = c.slots.keys().cloned().collect();
    Some(Program {
        insts: c.insts,
        nslots,
        slots: c.slots,
        names,
        reads_registers,
        clock: crate::typed::ClockCell::now(),
    })
}

/// Largest save-slot count handled inline. Slots are `2 + 2 * registers`,
/// so this covers a whole-match span plus three named captures. A pattern
/// needing more falls back to the set-reachability engine, which carries
/// its registers in a map.
const MAX_SAVE_SLOTS: usize = 8;

/// The save slots for one matcher thread: the whole-match start/end and a
/// start/end per captured register. Inline and `Copy`, so a thread fork
/// (one per epsilon split, one per surviving anchor) is a stack copy
/// rather than a heap allocation. The single-pass scan forks on the
/// hottest path, so this is where the per-token allocation cost was.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Saves {
    slots: [usize; MAX_SAVE_SLOTS],
    len: usize,
}

/// Hashes the slots this thread actually has, not the whole array.
///
/// A derived implementation feeds all [`MAX_SAVE_SLOTS`] of them - sixty-four
/// bytes - to the hasher on every insert into the register-keyed dedup set,
/// which happens once per thread per step. `len` is two plus two per
/// register, so a pattern binding one register carries four slots, and
/// `register_key` zeroes the first two before the key is built.
///
/// Hashing has to agree with equality or two equal keys could land in
/// different buckets and a duplicate thread would survive, which decides
/// which alternative the scan keeps. `PartialEq` is derived over `slots` and
/// `len` together; hashing `len` first and then that many slots covers the
/// same information, and including `len` is what keeps a shorter thread from
/// colliding with a longer one that shares its prefix.
impl std::hash::Hash for Saves {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.len.hash(state);
        self.slots[..self.len].hash(state);
    }
}

impl Saves {
    /// A fresh slot set of `len` unset (`usize::MAX`) slots. `len` is the
    /// program's `nslots`, which the scan caps at `MAX_SAVE_SLOTS`.
    #[inline]
    fn unset(len: usize) -> Self {
        Saves { slots: [usize::MAX; MAX_SAVE_SLOTS], len }
    }

    #[inline]
    fn get(&self, i: usize) -> usize {
        self.slots[i]
    }

    #[inline]
    fn set(&mut self, i: usize, v: usize) {
        self.slots[i] = v;
    }

    #[inline]
    fn as_slice(&self) -> &[usize] {
        &self.slots[..self.len]
    }

    /// The register slots only, with the whole-match start and end cleared.
    ///
    /// These are the slots a [`Atom::RegisterEq`] can read, so they belong in
    /// the thread-dedup key. Slots 0 and 1 are output-only; keeping them would
    /// stop threads with different start positions from ever collapsing, which
    /// is what makes the single-pass scan linear.
    #[inline]
    fn register_key(&self) -> Saves {
        let mut k = *self;
        k.slots[0] = 0;
        k.slots[1] = 0;
        k
    }
}

/// A running thread: a program counter plus its save slots.
#[derive(Clone, Copy)]
struct Thread {
    pc: usize,
    saves: Saves,
}

/// One slot of [`RegisterSeen`]: it holds a key while its stamp is the
/// table's, and is free otherwise.
#[derive(Clone)]
struct RegisterSlot {
    stamp: u32,
    pc: u32,
    saves: Saves,
}

/// The register-keyed thread set: counter and register slots in an
/// open-addressed table, probed linearly, with the key compared inline.
///
/// The step clears this once per anchor, so clearing is a new stamp rather
/// than a walk: a slot whose stamp is not the table's reads as free. The
/// stamp wrapping is the only walk, and it writes one word a slot.
struct RegisterSeen {
    slots: Vec<RegisterSlot>,
    /// Keys held under the current stamp. The table doubles when they would
    /// pass three quarters of the slots, so a probe stays short.
    live: usize,
    stamp: u32,
    build: crate::fxhash::FxBuild,
}

impl RegisterSeen {
    /// Slots a fresh table holds, a power of two so the mask is one less.
    const INITIAL: usize = 64;

    fn new() -> Self {
        Self {
            slots: vec![RegisterSlot { stamp: 0, pc: 0, saves: Saves::unset(0) }; Self::INITIAL],
            live: 0,
            stamp: 1,
            build: crate::fxhash::FxBuild::process(),
        }
    }

    fn clear(&mut self) {
        self.live = 0;
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            for slot in &mut self.slots {
                slot.stamp = 0;
            }
            self.stamp = 1;
        }
    }

    /// Record the thread at `pc` with `saves`, reporting whether it was
    /// already held under this stamp.
    #[inline]
    fn insert(&mut self, pc: usize, saves: Saves) -> bool {
        if (self.live + 1) * 4 > self.slots.len() * 3 {
            self.grow();
        }
        let mask = self.slots.len() - 1;
        let mut i = self.hash(pc, &saves) as usize & mask;
        loop {
            if self.slots[i].stamp != self.stamp {
                self.slots[i] = RegisterSlot { stamp: self.stamp, pc: pc as u32, saves };
                self.live += 1;
                return true;
            }
            if self.slots[i].pc as usize == pc && self.slots[i].saves == saves {
                return false;
            }
            i = (i + 1) & mask;
        }
    }

    /// One call per insert, on the engine's hottest path.
    #[inline(always)]
    fn hash(&self, pc: usize, saves: &Saves) -> u64 {
        use std::hash::BuildHasher;
        self.build.hash_one((pc, saves))
    }

    /// Twice the slots, every key held under this stamp placed again.
    fn grow(&mut self) {
        let capacity = self.slots.len() * 2;
        let free = RegisterSlot { stamp: 0, pc: 0, saves: Saves::unset(0) };
        let old = std::mem::replace(&mut self.slots, vec![free; capacity]);
        let stamp = self.stamp;
        self.live = 0;
        for slot in old {
            if slot.stamp == stamp {
                self.insert(slot.pc as usize, slot.saves);
            }
        }
    }
}

#[cfg(test)]
mod register_seen_tests {
    use super::{RegisterSeen, Saves};

    fn key(a: usize, b: usize) -> Saves {
        let mut s = Saves::unset(4);
        s.set(2, a);
        s.set(3, b);
        s
    }

    #[test]
    fn a_key_is_new_once_and_held_after() {
        let mut seen = RegisterSeen::new();
        assert!(seen.insert(7, key(1, 2)));
        assert!(!seen.insert(7, key(1, 2)));
        assert!(seen.insert(8, key(1, 2)), "another counter is another key");
        assert!(seen.insert(7, key(1, 3)), "another binding is another key");
    }

    #[test]
    fn clearing_frees_every_key_without_walking() {
        let mut seen = RegisterSeen::new();
        assert!(seen.insert(3, key(4, 5)));
        seen.clear();
        assert!(seen.insert(3, key(4, 5)), "the stamp moved, so the slot reads free");
        assert_eq!(seen.live, 1);
    }

    #[test]
    fn it_grows_past_its_first_slots_and_keeps_every_key() {
        let mut seen = RegisterSeen::new();
        for i in 0..500usize {
            assert!(seen.insert(i, key(i, i + 1)), "key {i} is new");
        }
        assert!(seen.slots.len() >= 512, "the table doubled to hold them");
        for i in 0..500usize {
            assert!(!seen.insert(i, key(i, i + 1)), "key {i} is still held");
        }
    }

    #[test]
    fn a_wrapped_stamp_frees_the_table() {
        let mut seen = RegisterSeen::new();
        seen.stamp = u32::MAX;
        assert!(seen.insert(1, key(9, 9)));
        seen.clear();
        assert_eq!(seen.stamp, 1, "the stamp restarts past a wrap");
        assert!(seen.insert(1, key(9, 9)), "and every slot reads free");
    }
}

/// A priority-ordered, deduplicated thread list for one step.
///
/// Two threads are the same when they will behave identically from here on.
/// For a program with no registers that is just the program counter, and
/// `seen` is a bool per counter. A program that binds registers also has
/// [`Atom::RegisterEq`] instructions reading those slots, so two threads at
/// one counter with different bindings are not interchangeable and the key
/// takes the register slots too; `seen_regs` carries that heavier key, and is
/// used only by patterns that need it. It is an open-addressed table hashed
/// with [`crate::fxhash`], from a key this process drew once.
struct ThreadList {
    dense: Vec<Thread>,
    /// The stamp of the step that last visited each counter. A counter is
    /// visited this step when its stamp is the current one, so clearing the
    /// list is a new stamp rather than a write per counter; the list is
    /// cleared once per step per anchor, the hottest path the engine has.
    seen: Vec<u32>,
    stamp: u32,
    seen_regs: RegisterSeen,
    keyed_by_registers: bool,
    /// What the closure walk did over this list's life: calls into `add`,
    /// calls that stopped at `mark` because the counter was already reached
    /// this step, and threads pushed. Plain fields rather than trace calls,
    /// because `add` runs once an epsilon step per thread per anchor and a
    /// trace call there would cost more than the step; a field costs one
    /// increment, and the totals are handed over once when the list goes.
    closure_steps: u64,
    steps_already_seen: u64,
    threads_queued: u64,
}

impl ThreadList {
    fn new(np: usize, keyed_by_registers: bool) -> Self {
        Self {
            dense: Vec::new(),
            seen: vec![0; np],
            stamp: 1,
            seen_regs: RegisterSeen::new(),
            keyed_by_registers,
            closure_steps: 0,
            steps_already_seen: 0,
            threads_queued: 0,
        }
    }

    fn clear(&mut self) {
        self.dense.clear();
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            // The stamp wrapped, so a counter last visited under stamp zero
            // would read as visited now; zero them and start over.
            self.seen.fill(0);
            self.stamp = 1;
        }
        // The register-keyed set is only ever written by a keyed list, so an
        // unkeyed one has nothing to clear.
        if self.keyed_by_registers {
            self.seen_regs.clear();
        }
    }

    /// Record this thread as visited, reporting whether it already was.
    fn mark(&mut self, pc: usize, saves: &Saves) -> bool {
        if self.keyed_by_registers {
            !self.seen_regs.insert(pc, saves.register_key())
        } else {
            let was = self.seen[pc] == self.stamp;
            self.seen[pc] = self.stamp;
            was
        }
    }

    /// Follow epsilon transitions from `pc`, recording consuming and
    /// accepting instructions into the list in priority order. `pos` is
    /// the significant-token index the thread currently sits at; saves
    /// record it. A counter reached twice within one step is a duplicate:
    /// the thread that reached it first has every continuation this one
    /// would, at a higher priority, so the second is dropped.
    #[allow(clippy::too_many_arguments)]
    fn add<S: SigStream + ?Sized>(
        &mut self,
        prog: &Program,
        pc: usize,
        saves: &Saves,
        pos: usize,
        input: &[u8],
        stream: &S,
        absent: &HashSet<Vec<u8>>,
    ) {
        self.closure_steps += 1;
        if self.mark(pc, saves) {
            self.steps_already_seen += 1;
            return;
        }
        match &prog.insts[pc] {
            Inst::Jmp(t) => self.add(prog, *t, saves, pos, input, stream, absent),
            Inst::Split(a, b) => {
                self.add(prog, *a, saves, pos, input, stream, absent);
                self.add(prog, *b, saves, pos, input, stream, absent);
            }
            Inst::Save(slot) => {
                let mut s = *saves;
                s.set(*slot, pos);
                self.add(prog, pc + 1, &s, pos, input, stream, absent);
            }
            Inst::Guard(lit, neg) => {
                // Positive guard advances when the literal is present ahead;
                // negative guard advances when it is absent.
                if guard_present(input, stream, pos, lit, absent) != *neg {
                    self.add(prog, pc + 1, saves, pos, input, stream, absent);
                }
            }
            Inst::Anchor(kind) => {
                // An anchor constrains the token the next atom consumes, so
                // it fails where there is no such token, matching the set
                // engine's reading at the end of the stream.
                let m = stream.len();
                let holds = pos < m && {
                    let (a, b) = stream.span(pos);
                    crate::engine::anchor_holds_at(kind, input, a, b, pos == 0, pos + 1 == m)
                };
                if holds {
                    self.add(prog, pc + 1, saves, pos, input, stream, absent);
                }
            }
            Inst::Atom(_) | Inst::Match => {
                self.threads_queued += 1;
                self.dense.push(Thread { pc, saves: *saves });
            }
        }
    }
}

impl Drop for ThreadList {
    /// The walk's counts, handed over once when the list goes. A list lives a
    /// leaf or a scan rather than a step, so this runs once per leaf, and
    /// `counted` returns at once where nothing is recording.
    ///
    /// A list dropped by an unwind hands over nothing: its counts stop wherever
    /// the panic found them, and a partial total merged into the rest would
    /// read as a whole one with nothing to say otherwise.
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        crate::trace::counted("the walk: closure steps", self.closure_steps);
        crate::trace::counted("the walk: closure steps already seen", self.steps_already_seen);
        crate::trace::counted("the walk: threads queued", self.threads_queued);
    }
}

/// Whether `lit` occurs in the input at or after the current position.
/// `absent` holds guard literals a prefilter has proven are nowhere in
/// the input; one of those fails the guard with no positional scan.
fn guard_present<S: SigStream + ?Sized>(
    input: &[u8],
    stream: &S,
    pos: usize,
    lit: &[u8],
    absent: &HashSet<Vec<u8>>,
) -> bool {
    if absent.contains(lit) {
        return false;
    }
    let from = if pos < stream.len() { stream.span(pos).0 } else { input.len() };
    crate::byte_simd::contains(&input[from..], lit)
}

fn atom_matches<S: SigStream + ?Sized>(
    a: &Atom,
    prog: &Program,
    input: &[u8],
    stream: &S,
    k: usize,
    saves: &[usize],
) -> bool {
    let kind = stream.kind(k);
    // The token's bytes are read only by the atoms below that ask for them. A
    // kind atom is the common one and reads none, and finding the bytes costs
    // a span lookup, which over the parts is a search or a cursor step.
    let txt = || text_of(input, stream, k);
    match a {
        Atom::Kind(want) => kind == *want,
        Atom::Any => true,
        Atom::Literal(lit, group) => {
            crate::engine::register_eq_matches(lit.as_bytes(), txt(), *group)
        }
        Atom::Byte(bc) => byte_class_matches(*bc, txt()),
        Atom::BytePattern(bp) => bp.matches_whole(txt()),
        // A spectral atom reaches this engine only in a program compiled for
        // the device, which evaluates it itself; here it matches nothing. An
        // instant read against a register's, and a kinship read off the pair
        // field, are routed away before compiling, so they reach this engine
        // no more than a whitespace atom does.
        Atom::Spectral(_) | Atom::Since(..) | Atom::RegisterKin(..) => false,
        // Magnitude and kind-magnitude are stateless (pure functions of the
        // token's kind and bytes), so the linear engine evaluates them directly.
        // Only absolute forms reach this engine; the relative ones route to
        // the set engine, which carries their context.
        Atom::Magnitude(pred) => pred.matches(crate::magnitude::token_magnitude(kind, txt()), None),
        Atom::KindMag(want, pred) => {
            kind == *want && pred.matches(crate::magnitude::token_magnitude(kind, txt()), None)
        }
        // A typed predicate reads the token's bytes in its kind's own units;
        // the clock a timestamp clause needs is the program's, one reading per
        // scan.
        Atom::KindPred(want, pred) => kind == *want && pred.matches_as(kind, txt(), || prog.clock.get()),
        Atom::Class(c) => {
            let hit = c.any.iter().any(|m| atom_matches(m, prog, input, stream, k, saves))
                && (c.all.is_empty() || c.all.iter().any(|m| atom_matches(m, prog, input, stream, k, saves)))
                && !c.none.iter().any(|m| atom_matches(m, prog, input, stream, k, saves));
            hit != c.negated
        }
        Atom::RegisterEq(name, group) => match prog.slots.get(name) {
            Some(&base) => {
                let (start, end) = (saves[base], saves[base + 1]);
                if start == usize::MAX || end == usize::MAX || end <= start {
                    return false;
                }
                let bound = bytes_between(input, stream, start, end);
                register_eq_matches(bound, txt(), *group)
            }
            None => false,
        },
        Atom::RegisterRelated(name, relation) => match prog.slots.get(name) {
            Some(&base) => {
                let (start, end) = (saves[base], saves[base + 1]);
                if start == usize::MAX || end == usize::MAX || end <= start {
                    return false;
                }
                relation.related(bytes_between(input, stream, start, end), txt())
            }
            None => false,
        },
        Atom::LiteralWithin(lit, k, group) => {
            crate::engine::within_edits(lit.as_bytes(), txt(), *k, *group)
        }
        Atom::RegisterWithin(name, k, group) => match prog.slots.get(name) {
            Some(&base) => {
                let (start, end) = (saves[base], saves[base + 1]);
                if start == usize::MAX || end == usize::MAX || end <= start {
                    return false;
                }
                crate::engine::within_edits(bytes_between(input, stream, start, end), txt(), *k, *group)
            }
            None => false,
        },
    }
}

/// The atoms an attempt can consume its first token with: those reachable from
/// the program's start without consuming anything.
///
/// A match anchored at `k` has to consume the token at `k` with one of these,
/// so an anchor none of them accepts has no match beginning at it and the
/// attempt there can be skipped. Guards and anchors are followed rather than
/// evaluated, which makes the set a superset of what any particular position
/// can really reach - and a superset is the safe direction, because it admits
/// anchors that turn out to have no match rather than rejecting ones that do.
///
/// `None` where no such test can reject anything, which the caller reads as
/// "attempt every anchor":
///
///   - a start closure reaching `Match` admits an empty match, which consumes
///     no token and so is not conditional on the one at `k`;
///   - a program that reads a register back has atoms whose answer depends on
///     bindings no thread has made yet at an anchor, and they cannot be
///     evaluated before the attempt that makes them;
///   - a program whose start consumes nothing at all has no opening atom to
///     test.
fn opening_atoms(prog: &Program) -> Option<Vec<&Atom>> {
    let pcs = opening_counters(prog)?;
    let mut opens = Vec::with_capacity(pcs.len());
    for pc in pcs {
        if let Inst::Atom(a) = &prog.insts[pc] {
            opens.push(a);
        }
    }
    Some(opens)
}

/// The opening atoms as the bytes a search finds them by, where every one of
/// them is a literal compared byte for byte; `None` where any is not, since a
/// kind, a class, or a literal under an orbit names no bytes a search could
/// look for.
fn opening_literals<'a>(opens: &[&'a Atom]) -> Option<Vec<&'a [u8]>> {
    let mut lits = Vec::with_capacity(opens.len());
    for a in opens {
        match a {
            Atom::Literal(lit, crate::orbit::OrbitGroup::Identity) if !lit.is_empty() => {
                lits.push(lit.as_bytes());
            }
            _ => return None,
        }
    }
    (!lits.is_empty()).then_some(lits)
}

/// The counters of the [`opening_atoms`], under the same refusals.
fn opening_counters(prog: &Program) -> Option<Vec<usize>> {
    if prog.reads_registers {
        return None;
    }
    let mut seen = vec![false; prog.insts.len()];
    let mut stack = vec![0usize];
    let mut opens = Vec::new();
    while let Some(pc) = stack.pop() {
        if std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        match &prog.insts[pc] {
            Inst::Atom(_) => opens.push(pc),
            Inst::Match => return None,
            Inst::Split(a, b) => {
                stack.push(*a);
                stack.push(*b);
            }
            Inst::Jmp(t) => stack.push(*t),
            Inst::Save(_) | Inst::Guard(..) | Inst::Anchor(_) => stack.push(pc + 1),
        }
    }
    (!opens.is_empty()).then_some(opens)
}

/// Several programs walked as one. The members' leading instructions are
/// laid out as a trie, so members that open the same way share one thread
/// until they part, and the rest of each member follows on counters of its
/// own, its `Match` among them; one thread list carries every member's
/// threads and a step reads each token once for all of them. A counter on a
/// shared prefix belongs to every member below it and any other to one
/// member, and the dedup is by counter, so past the fork a member's threads
/// never meet another's.
///
/// Members that read a register back are not taken, since their atoms read
/// a slot map that is theirs alone; nor are members that can match empty,
/// that carry `\G` or `\K`, whose walks are sequential, or that name a
/// library kind, whose tokens only a shaped lex makes.
pub(crate) struct Union {
    prog: Program,
    /// How many members the union holds.
    members: usize,
    /// The member each counter belongs to, and `None` on a shared prefix.
    owner: Vec<Option<u32>>,
    /// The counter each entry begins at: one entry per way the members
    /// open, seeded at every token its opening atoms may accept.
    entries: Vec<usize>,
    /// Each entry's opening atoms, as the counters of the atoms reachable
    /// from it without consuming.
    opens: Vec<Vec<usize>>,
    /// The entries whose opening atoms are tested at every token.
    always: Vec<u32>,
    /// The entries with a kind atom among their opening atoms, by kind.
    by_kind: std::collections::HashMap<TokenKind, Vec<u32>>,
    /// The entries with an exact literal among their opening atoms, by
    /// literal.
    by_literal: std::collections::HashMap<Vec<u8>, Vec<u32>>,
}

/// A node of the trie over the members' leading instructions.
struct Trie {
    inst: Inst,
    children: Vec<Trie>,
    /// The members whose own instructions begin after this node.
    ends: Vec<u32>,
}

/// Where a program's leading linear run ends: the index of its first
/// `Split`, `Jmp` or `Match`, or of the first instruction one of them
/// targets, whichever comes first. At least one, past the opening `Save`.
fn linear_prefix_len(insts: &[Inst]) -> usize {
    let mut targeted = vec![false; insts.len()];
    for inst in insts {
        match inst {
            Inst::Split(a, b) => {
                targeted[*a] = true;
                targeted[*b] = true;
            }
            Inst::Jmp(t) => targeted[*t] = true,
            Inst::Atom(_) | Inst::Save(_) | Inst::Guard(..) | Inst::Anchor(_) | Inst::Match => {}
        }
    }
    let end = insts
        .iter()
        .enumerate()
        .skip(1)
        .find(|(pc, inst)| {
            targeted[*pc] || matches!(inst, Inst::Split(..) | Inst::Jmp(_) | Inst::Match)
        })
        .map_or(insts.len(), |(pc, _)| pc);
    end.max(1)
}

/// Insert member `m`'s leading run `prefix`, which is not empty, under
/// `nodes`, recording the member at the node its run ends on.
fn trie_insert(nodes: &mut Vec<Trie>, prefix: &[Inst], m: u32) {
    let Some((first, rest)) = prefix.split_first() else {
        return;
    };
    let at = match nodes.iter().position(|n| n.inst == *first) {
        Some(at) => at,
        None => {
            nodes.push(Trie { inst: first.clone(), children: Vec::new(), ends: Vec::new() });
            nodes.len() - 1
        }
    };
    if rest.is_empty() {
        nodes[at].ends.push(m);
    } else {
        trie_insert(&mut nodes[at].children, rest, m);
    }
}

/// The counters of the atoms reachable from `entry` without consuming.
fn opening_from(insts: &[Inst], entry: usize) -> Vec<usize> {
    let mut seen = vec![false; insts.len()];
    let mut stack = vec![entry];
    let mut opens = Vec::new();
    while let Some(pc) = stack.pop() {
        if std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        match &insts[pc] {
            Inst::Atom(_) => opens.push(pc),
            Inst::Match => {}
            Inst::Split(a, b) => {
                stack.push(*a);
                stack.push(*b);
            }
            Inst::Jmp(t) => stack.push(*t),
            Inst::Save(_) | Inst::Guard(..) | Inst::Anchor(_) => stack.push(pc + 1),
        }
    }
    opens
}

/// Whether `pattern` can be a member of a [`Union`].
#[must_use]
pub(crate) fn union_eligible(pattern: &Pattern) -> bool {
    if pattern.starts_with_resume()
        || pattern.mentions_reset_start()
        || !pattern.library_kinds().is_empty()
    {
        return false;
    }
    match compile(pattern) {
        Some(prog) => {
            prog.nslots <= MAX_SAVE_SLOTS
                && !prog.reads_registers
                && opening_counters(&prog).is_some()
        }
        None => false,
    }
}

impl Union {
    /// The union of `patterns`, each of which [`union_eligible`] accepts, in
    /// the order given, which is the member index every answer is under.
    pub(crate) fn of(patterns: &[&Pattern]) -> Union {
        // Each member's instructions past its leading run, with the index the
        // run ended at, so a jump inside the rest can be rebased.
        let mut rests: Vec<(usize, Vec<Inst>)> = Vec::with_capacity(patterns.len());
        let mut root: Vec<Trie> = Vec::new();
        let mut unshared: Vec<u32> = Vec::new();
        let mut nslots = 0;
        for (i, p) in patterns.iter().enumerate() {
            let m = u32::try_from(i).expect("a union holds fewer than four billion members");
            let prog = compile(p).expect("every member passed union_eligible");
            nslots = nslots.max(prog.nslots);
            let split = linear_prefix_len(&prog.insts);
            // Every program opens with the whole-match save, which each entry
            // restores, so the trie holds what follows it.
            if split > 1 {
                trie_insert(&mut root, &prog.insts[1..split], m);
            } else {
                unshared.push(m);
            }
            rests.push((split, prog.insts[split..].to_vec()));
        }
        let mut insts: Vec<Inst> = Vec::new();
        let mut owner: Vec<Option<u32>> = Vec::new();
        let mut entries = Vec::with_capacity(root.len() + unshared.len());
        for node in &root {
            entries.push(insts.len());
            insts.push(Inst::Save(0));
            owner.push(None);
            emit_trie(node, &mut insts, &mut owner, &rests);
        }
        for &m in &unshared {
            entries.push(insts.len());
            insts.push(Inst::Save(0));
            owner.push(None);
            emit_rest(m, &mut insts, &mut owner, &rests);
        }
        let mut opens = Vec::with_capacity(entries.len());
        let mut always = Vec::new();
        let mut by_kind: std::collections::HashMap<TokenKind, Vec<u32>> =
            std::collections::HashMap::new();
        let mut by_literal: std::collections::HashMap<Vec<u8>, Vec<u32>> =
            std::collections::HashMap::new();
        for (e, &entry) in entries.iter().enumerate() {
            let e = u32::try_from(e).expect("fewer entries than members");
            let opening = opening_from(&insts, entry);
            let mut in_always = false;
            for &pc in &opening {
                match &insts[pc] {
                    Inst::Atom(Atom::Kind(k) | Atom::KindMag(k, _) | Atom::KindPred(k, _)) => {
                        let members = by_kind.entry(*k).or_default();
                        if members.last() != Some(&e) {
                            members.push(e);
                        }
                    }
                    Inst::Atom(Atom::Literal(lit, crate::orbit::OrbitGroup::Identity)) => {
                        let members = by_literal.entry(lit.as_bytes().to_vec()).or_default();
                        if members.last() != Some(&e) {
                            members.push(e);
                        }
                    }
                    Inst::Atom(_) => {
                        if !in_always {
                            always.push(e);
                            in_always = true;
                        }
                    }
                    Inst::Split(..) | Inst::Jmp(_) | Inst::Save(_) | Inst::Guard(..) | Inst::Anchor(_) | Inst::Match => {}
                }
            }
            opens.push(opening);
        }
        Union {
            prog: Program {
                insts,
                nslots,
                slots: BTreeMap::new(),
                names: std::sync::Arc::from(Vec::<String>::new()),
                reads_registers: false,
                clock: crate::typed::ClockCell::now(),
            },
            members: patterns.len(),
            owner,
            entries,
            opens,
            always,
            by_kind,
            by_literal,
        }
    }

    /// How many members the union holds.
    pub(crate) fn len(&self) -> usize {
        self.members
    }

    /// The entries that may open at token `k`, into `out`: a superset of
    /// those whose opening atoms accept it, which [`Self::admits`] decides.
    fn candidates<S: SigStream + ?Sized>(&self, input: &[u8], stream: &S, k: usize, out: &mut Vec<u32>) {
        out.clear();
        out.extend_from_slice(&self.always);
        if let Some(entries) = self.by_kind.get(&stream.kind(k)) {
            out.extend_from_slice(entries);
        }
        if !self.by_literal.is_empty()
            && let Some(entries) = self.by_literal.get(text_of(input, stream, k))
        {
            out.extend_from_slice(entries);
        }
    }

    /// Whether one of entry `e`'s opening atoms accepts the token at `k`.
    fn admits<S: SigStream + ?Sized>(&self, e: usize, input: &[u8], stream: &S, k: usize) -> bool {
        self.opens[e].iter().any(|&pc| match &self.prog.insts[pc] {
            Inst::Atom(a) => atom_matches(a, &self.prog, input, stream, k, &[]),
            _ => false,
        })
    }
}

/// Emit `node` and everything under it: its instruction, then the branches
/// that follow it, one child subtree or one member's rest laid out
/// straight after, and more than one behind a chain of splits.
fn emit_trie(node: &Trie, insts: &mut Vec<Inst>, owner: &mut Vec<Option<u32>>, rests: &[(usize, Vec<Inst>)]) {
    insts.push(node.inst.clone());
    owner.push(None);
    let branches = node.children.len() + node.ends.len();
    if branches == 1 {
        match node.children.first() {
            Some(child) => emit_trie(child, insts, owner, rests),
            None => emit_rest(node.ends[0], insts, owner, rests),
        }
        return;
    }
    if branches == 0 {
        return;
    }
    let first_split = insts.len();
    for _ in 0..branches - 1 {
        insts.push(Inst::Split(0, 0));
        owner.push(None);
    }
    let mut starts = Vec::with_capacity(branches);
    for child in &node.children {
        starts.push(insts.len());
        emit_trie(child, insts, owner, rests);
    }
    for &m in &node.ends {
        starts.push(insts.len());
        emit_rest(m, insts, owner, rests);
    }
    for i in 0..branches - 1 {
        let next = if i + 1 < branches - 1 { first_split + i + 1 } else { starts[branches - 1] };
        insts[first_split + i] = Inst::Split(starts[i], next);
    }
}

/// Emit member `m`'s instructions past its leading run, rebasing the jumps
/// among them, every counter the member's own.
fn emit_rest(m: u32, insts: &mut Vec<Inst>, owner: &mut Vec<Option<u32>>, rests: &[(usize, Vec<Inst>)]) {
    let (split, rest) = &rests[m as usize];
    let base = insts.len();
    let rebase = |t: usize| base + t.checked_sub(*split).expect("a jump never lands in the shared run");
    for inst in rest {
        insts.push(match inst {
            Inst::Split(a, b) => Inst::Split(rebase(*a), rebase(*b)),
            Inst::Jmp(t) => Inst::Jmp(rebase(*t)),
            other => other.clone(),
        });
        owner.push(Some(m));
    }
}

/// Each member's first match at or after token `from` of `toks`, walking
/// every member `active` holds at once; `None` for a member with no match
/// or one not asked. Member by member, the span is the one the member's own
/// walk reports first.
///
/// A member's first match settles the way a lone walk's does. A thread
/// reaching the member's `Match` records the match and cuts the member's
/// lower-priority threads that step; a thread of the member that began
/// later than the recorded match is dropped, since only an earlier start
/// outranks it; and the match is final once no running thread began at or
/// before its start, so nothing left could outrank it. Seeds go on for
/// every entry, because an entry's thread serves every member under it.
/// The literals in `absent` are the ones a prefilter proved absent for every
/// member's guards.
pub(crate) fn first_spans_union(
    u: &Union,
    active: &[bool],
    input: &[u8],
    toks: &[Token],
    from: usize,
    absent: &HashSet<Vec<u8>>,
) -> Vec<Option<Span>> {
    u.prog.clock.refresh();
    let sig: Vec<usize> = (from.min(toks.len())..toks.len())
        .filter(|&i| toks[i].kind != TokenKind::Whitespace)
        .collect();
    let stream = Stitched { toks, sig: &sig };
    let m = stream.len();
    let n = u.len();
    let mut first: Vec<Option<Span>> = vec![None; n];
    let mut pending: Vec<Option<Saves>> = vec![None; n];
    let mut done: Vec<bool> = active.iter().map(|a| !*a).collect();
    let mut open = done.iter().filter(|d| !**d).count();
    // The step a member's match cut its lower threads on; a stamp rather
    // than a flag so nothing is cleared per step.
    let mut cut: Vec<usize> = vec![0; n];
    let np = u.prog.insts.len();
    let mut clist = ThreadList::new(np, false);
    let mut nlist = ThreadList::new(np, false);
    let mut seeds: Vec<u32> = Vec::new();
    let mut k = 0usize;
    let mut step = 0usize;
    while open > 0 {
        step += 1;
        if k < m {
            u.candidates(input, &stream, k, &mut seeds);
            for &e in &seeds {
                let e = e as usize;
                if u.admits(e, input, &stream, k) {
                    clist.add(&u.prog, u.entries[e], &Saves::unset(u.prog.nslots), k, input, &stream, absent);
                }
            }
        }
        nlist.clear();
        for &t in &clist.dense {
            let start = t.saves.get(0);
            if let Some(j) = u.owner[t.pc] {
                let j = j as usize;
                if done[j] || cut[j] == step {
                    continue;
                }
                if let Some(p) = pending[j]
                    && start > p.get(0)
                {
                    continue;
                }
            }
            match &u.prog.insts[t.pc] {
                Inst::Match => {
                    if let Some(j) = u.owner[t.pc] {
                        let j = j as usize;
                        pending[j] = Some(t.saves);
                        cut[j] = step;
                    }
                }
                Inst::Atom(a) if k < m && atom_matches(a, &u.prog, input, &stream, k, t.saves.as_slice()) => {
                    nlist.add(&u.prog, t.pc + 1, &t.saves, k + 1, input, &stream, absent);
                }
                _ => {}
            }
        }
        let oldest = nlist.dense.iter().map(|t| t.saves.get(0)).min();
        for j in 0..n {
            if done[j] {
                continue;
            }
            if let Some(p) = pending[j]
                && oldest.is_none_or(|o| o > p.get(0))
            {
                let (ks, ke) = (p.get(0), p.get(1));
                if ke > ks {
                    first[j] = Some(span_between(&stream, ks, ke));
                }
                done[j] = true;
                open -= 1;
            }
        }
        std::mem::swap(&mut clist, &mut nlist);
        if k >= m {
            break;
        }
        k += 1;
    }
    first
}

/// Whether any opening atom accepts the token at `k`.
///
/// The saves are empty because [`opening_atoms`] refuses a program that reads a
/// register, which is the only thing `atom_matches` consults them for.
#[inline]
fn opening_admits<S: SigStream + ?Sized>(
    prog: &Program,
    opens: &[&Atom],
    input: &[u8],
    stream: &S,
    k: usize,
) -> bool {
    k < stream.len() && opens.iter().any(|a| atom_matches(a, prog, input, stream, k, &[]))
}

/// Find a match's save slots starting the search at significant position
/// `from`, injecting new start threads only while the position is below
/// `inject_until`. With `inject_until = sig.len() + 1` this is the
/// leftmost-longest match at or after `from` over the whole stream; with
/// `inject_until = from + 1` it is the longest match anchored exactly at
/// `from`, the per-anchor unit the parallel scan computes for every
/// position. A match found may still extend past `inject_until`, since
/// once started a thread runs to its longest end.
#[allow(clippy::too_many_arguments)]
fn find_leftmost<'l, S: SigStream + ?Sized>(
    prog: &Program,
    input: &[u8],
    stream: &S,
    from: usize,
    absent: &HashSet<Vec<u8>>,
    inject_until: usize,
    clist: &'l mut ThreadList,
    nlist: &'l mut ThreadList,
) -> Option<Saves> {
    find_leftmost_ending(prog, input, stream, from, absent, inject_until, clist, nlist, false)
}

/// [`find_leftmost`], returning at the first token any thread accepts on when
/// `earliest_end` is set rather than letting the pattern's own preference
/// settle which match is reported.
#[allow(clippy::too_many_arguments)]
fn find_leftmost_ending<'l, S: SigStream + ?Sized>(
    prog: &Program,
    input: &[u8],
    stream: &S,
    from: usize,
    absent: &HashSet<Vec<u8>>,
    inject_until: usize,
    mut clist: &'l mut ThreadList,
    mut nlist: &'l mut ThreadList,
    earliest_end: bool,
) -> Option<Saves> {
    let m = stream.len();
    // The two thread lists are owned by the caller and reused across
    // calls, so a per-anchor scan allocates them once per leaf rather than
    // once per anchor. Reset them to empty before this attempt.
    clist.clear();
    nlist.clear();
    let mut matched: Option<Saves> = None;
    let mut k = from;

    loop {
        // Inject a start thread at the lowest priority while still
        // searching and while the start position is below the injection
        // bound. Threads already in `clist` started earlier and keep
        // priority.
        let may_start = k < inject_until;
        if matched.is_none() && may_start {
            clist.add(prog, 0, &Saves::unset(prog.nslots), k, input, stream, absent);
        }
        // An empty list ends the search only once no further start can be
        // injected. A zero-width assertion that fails here kills the thread
        // injected at this position while leaving a later one viable, so
        // stopping on emptiness alone would abandon the rest of the stream:
        // `$ \W` fails at every token but the last, and would find nothing.
        if clist.dense.is_empty() && (matched.is_some() || !may_start) {
            break;
        }

        nlist.clear();
        let mut ti = 0;
        while ti < clist.dense.len() {
            let pc = clist.dense[ti].pc;
            match &prog.insts[pc] {
                Inst::Match => {
                    matched = Some(clist.dense[ti].saves);
                    if earliest_end {
                        // The step a thread first reaches `Match` on is the
                        // step at which the soonest-ending match ends: every
                        // thread still running ends at this token or a later
                        // one. A caller asking how far a match reaches wants
                        // that step, where a caller asking what the match is
                        // wants the pattern's own preference and must let the
                        // higher-priority threads finish.
                        return matched;
                    }
                    // A match cuts every lower-priority thread this step.
                    break;
                }
                Inst::Atom(a)
                    if k < m
                        && atom_matches(a, prog, input, stream, k, clist.dense[ti].saves.as_slice()) =>
                {
                    nlist.add(prog, pc + 1, &clist.dense[ti].saves, k + 1, input, stream, absent);
                }
                _ => {}
            }
            ti += 1;
        }

        // The lists trade roles by swapping the two references, not the two
        // lists: the structs would be copied three times per step.
        std::mem::swap(&mut clist, &mut nlist);
        if k >= m {
            break;
        }
        k += 1;
    }

    matched
}

/// Each register the program names, with the byte span it bound.
///
/// The save slots hold significant-token indices, so a span is two lookups in
/// the stream. A register that bound nothing gets an empty span at zero, which
/// is what a reader distinguishes by its emptiness rather than by a sentinel.
///
/// Built as the match will hold them rather than as a vector the match then
/// copies: a program naming no more registers than ride inline allocates
/// nothing here, which is every pattern that binds one or two.
fn resolve_captures<S: SigStream + ?Sized>(
    prog: &Program,
    saves: &[usize],
    stream: &S,
) -> crate::engine::Regs {
    let bound = |base: usize| {
        let (start, end) = (saves[base], saves[base + 1]);
        if start != usize::MAX && end != usize::MAX && end > start {
            span_between(stream, start, end)
        } else {
            Span { start: 0, end: 0 }
        }
    };
    let n = prog.slots.len();
    if n <= crate::engine::INLINE_REGS {
        let mut held = [Span { start: 0, end: 0 }; crate::engine::INLINE_REGS];
        for (slot, &base) in held.iter_mut().zip(prog.slots.values()) {
            *slot = bound(base);
        }
        let count = u8::try_from(n).expect("at most INLINE_REGS");
        return crate::engine::Regs::Inline(count, held);
    }
    let caps: Vec<Span> = prog.slots.values().map(|&base| bound(base)).collect();
    crate::engine::Regs::Shared(caps.into())
}

/// The register names a program binds, in the order [`resolve_captures`] writes
/// their spans, held once for the program rather than copied into each match.
fn register_names(prog: &Program) -> std::sync::Arc<[String]> {
    prog.names.clone()
}

/// Each register the program names, written into `spans` and `extents` at the
/// index `order` gives its name.
///
/// The save slots hold significant-token indices, so the extent is that pair
/// and the byte span is two lookups in the stream. A register that bound
/// nothing leaves both of its slots `None`, which is what distinguishes it from
/// one that bound an empty span.
///
/// A name the program binds and `order` does not hold is skipped rather than
/// written to an arbitrary slot.
fn resolve_captures_into<S: SigStream + ?Sized>(
    prog: &Program,
    saves: &[usize],
    stream: &S,
    order: &[String],
    spans: &mut [Option<Span>],
    extents: &mut [Option<(usize, usize)>],
) {
    for s in spans.iter_mut() {
        *s = None;
    }
    for e in extents.iter_mut() {
        *e = None;
    }
    for (name, &base) in &prog.slots {
        let Some(i) = order.iter().position(|n| n == name) else { continue };
        let (start, end) = (saves[base], saves[base + 1]);
        if start != usize::MAX && end != usize::MAX && end > start {
            spans[i] = Some(span_between(stream, start, end));
            extents[i] = Some((start, end));
        }
    }
}

/// Longest number of tokens a single anchored attempt can consume, or
/// `None` when the pattern can match an unbounded run (an open `Star` /
/// `Plus` / `Repeat`). A bounded length means every anchored attempt
/// stops after at most that many tokens, so computing one attempt per
/// anchor is linear work overall; an unbounded pattern would let a single
/// attempt run to the end of the stream, which the serial scan handles in
/// one pass instead.
pub(crate) fn bounded_max_len(pat: &Pattern) -> Option<usize> {
    match pat {
        // Zero-width: it consumes no token, whatever its sub-pattern does.
        Pattern::Assert(..) => Some(0),
        Pattern::Empty | Pattern::Guard(..) => Some(0),
        Pattern::Atom(_) => Some(1),
        // Atomic changes which length is kept, never how long one can be.
        Pattern::Bind(_, _, p) | Pattern::Opt(p, _) | Pattern::Atomic(p) => bounded_max_len(p),
        Pattern::Concat(v) => {
            let mut total = 0usize;
            for p in v {
                total = total.checked_add(bounded_max_len(p)?)?;
            }
            Some(total)
        }
        Pattern::Alt(v, _) => {
            let mut mx = 0usize;
            for p in v {
                mx = mx.max(bounded_max_len(p)?);
            }
            Some(mx)
        }
        Pattern::Repeat(p, _, Some(hi), _) => bounded_max_len(p)?.checked_mul(*hi),
        Pattern::Star(..) | Pattern::Plus(..) | Pattern::Repeat(_, _, None, _) => None,
        Pattern::Balanced(..) | Pattern::Field(..) => None,
        // The longest run within `k` of `m` atoms is `m + k` tokens.
        Pattern::Within(v, k) => v.len().checked_add(usize::from(*k)),
        // An anchor is zero-width; it consumes no token.
        Pattern::Anchor(_) => Some(0),
    }
}

/// At or above this many significant tokens a bounded pattern's per-anchor
/// attempts are dispatched across cores; below it the serial scan beats
/// the dispatch cost. Set above the measured crossover: with the inline
/// save slots the per-anchor work is cheap, so the dispatch only pays off
/// once the stream is large. `benches/vs_regex` times both routes over
/// tokens just lexed and tokens long held; re-measure there when the
/// engine's per-anchor cost moves.
const PARALLEL_SCAN_MIN_ANCHORS: usize = 8192;

/// A bounded pattern is dispatched in parallel only when its longest match
/// is at most this many tokens. A longer bound would let each anchored
/// attempt run far down the stream, so the per-anchor work would stop
/// being linear and the serial single-pass scan is the better tool.
const PARALLEL_SCAN_MAX_MATCH_LEN: usize = 64;

/// Scan `input` for `pattern` with the single-pass engine, returning
/// the leftmost, non-overlapping matches, or `None` when the pattern
/// needs the set-reachability engine.
///
/// A short-bounded pattern over a large token stream runs the per-anchor
/// attempts across cores through the work-stealing pool and then selects
/// the leftmost, non-overlapping matches on one thread; everything else
/// runs the serial scan, where a dispatch would not pay for itself.
#[must_use]
pub fn scan_nfa(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
    let prog = compile(pattern)?;
    // The single-pass engine carries capture slots inline; a pattern with
    // more slots than fit (more than three named captures) routes to the
    // set-reachability engine, which holds its registers in a map.
    if prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    // Lex into this thread's held significant-stream parts: the lexer fans
    // the tokenization across cores for large inputs, falls back to one part
    // under its own threshold, writes into pages already mapped on every
    // scan after the thread's first, and stitches nothing. The lex is the
    // whole of what happens before the walk, so its phase ends as the walk
    // begins.
    let lexing = crate::trace::phase("the single pass: its lex, in parts");
    crate::parallel_lex::lex_significant_parts_held(input, |parts| {
        drop(lexing);
        scan_nfa_parts(pattern, input, parts)
    })
}

/// Scan a pre-lexed token stream with the single-pass engine, or `None` when
/// the pattern needs the set-reachability engine.
///
/// A caller whose tokens came from a lexer configured differently (a
/// [`crate::custom::ShapeSet`] in force, say) keeps the linear engine this
/// way; lexing again here would discard the boundaries it chose.
#[must_use]
pub fn scan_nfa_over(pattern: &Pattern, input: &[u8], toks: &[Token]) -> Option<Vec<Span>> {
    let prog = compile(pattern)?;
    if prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    let sig: Vec<usize> =
        (0..toks.len()).filter(|&i| toks[i].kind != TokenKind::Whitespace).collect();
    let stream = Stitched { toks, sig: &sig };
    Some(scan_stream(&prog, pattern, input, &stream, &stream))
}

/// The earliest byte an attempt still running at the end of `toks` began
/// at, for a caller deciding how much of a buffer it may drop when more
/// input is still to come.
///
/// The outer `None` says the question cannot be answered here - the pattern
/// needs the set-reachability engine, which keeps no thread list a caller
/// can read - and such a caller must keep everything. The inner `None` says
/// no attempt survives to the end, so nothing before it can reach past it
/// and the whole buffer is settled.
///
/// Every position injects a start and no match stops the walk, because the
/// question is not which match the pattern would report but which attempts
/// a later token could still carry forward. A thread sitting on an atom at
/// the end needs another token; one sitting on `Match` has a match that a
/// greedy repetition could still extend. Both are unsettled, and the
/// earliest start among them is the first byte a stream must keep.
#[must_use]
pub fn earliest_unsettled(pattern: &Pattern, input: &[u8], toks: &[Token]) -> Option<Option<usize>> {
    let prog = compile(pattern)?;
    if prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    let sig: Vec<usize> =
        (0..toks.len()).filter(|&i| toks[i].kind != TokenKind::Whitespace).collect();
    let stream = Stitched { toks, sig: &sig };
    let absent = crate::prefilter::absent_guard_literals(pattern, input);
    let m = stream.len();
    let np = prog.insts.len();
    let keyed = prog.reads_registers;
    let mut clist = ThreadList::new(np, keyed);
    let mut nlist = ThreadList::new(np, keyed);
    let mut k = 0;
    loop {
        clist.add(&prog, 0, &Saves::unset(prog.nslots), k, input, &stream, &absent);
        if k >= m {
            break;
        }
        nlist.clear();
        let mut ti = 0;
        while ti < clist.dense.len() {
            let pc = clist.dense[ti].pc;
            if let Inst::Atom(a) = &prog.insts[pc]
                && atom_matches(a, &prog, input, &stream, k, clist.dense[ti].saves.as_slice())
            {
                nlist.add(&prog, pc + 1, &clist.dense[ti].saves, k + 1, input, &stream, &absent);
            }
            ti += 1;
        }
        std::mem::swap(&mut clist, &mut nlist);
        k += 1;
    }
    let earliest = clist.dense.iter().map(|t| t.saves.get(0)).min();
    // A start slot never set belongs to a thread that has consumed nothing,
    // which can begin no earlier than the end itself.
    Some(earliest.map(|start| if start == usize::MAX { m } else { start }).map(|start| {
        if start < m { stream.span(start).0 } else { input.len() }
    }))
}

/// [`scan_nfa`] over the significant stream in the lexer's parts, read where
/// the chunks were lexed with no stream stitched and no index of the
/// significant tokens built, or `None` when the pattern needs the
/// set-reachability engine.
#[must_use]
pub fn scan_nfa_parts(pattern: &Pattern, input: &[u8], parts: &[Significant]) -> Option<Vec<Span>> {
    let prog = compile(pattern)?;
    if prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    let bases = Parts::bases_of(parts);
    let stream = Parts::new(parts, &bases);
    Some(scan_stream(&prog, pattern, input, &stream, &stream.cursor()))
}

/// Whether `pattern` matches anywhere in `input`, or `None` when the pattern
/// needs the set-reachability engine.
///
/// Equal to `scan_nfa(..).map(|m| !m.is_empty())`, and the walk stops at the
/// first match rather than resuming past it, so a pattern that matches early
/// is answered without the rest of the stream being walked at all. The lex is
/// still the whole input's: this engine reads tokens, and they have to exist
/// before it can read them.
///
/// The trade is the other way where nothing matches: this walks serially to
/// the end, where [`scan_nfa`] may put the per-anchor attempts across cores.
/// Whether [`crate::engine::is_match`] should route here is read by building
/// it both ways and timing the two commits against each other, not by a
/// caller outside the crate choosing between them.
#[doc(hidden)]
#[must_use]
pub fn any_nfa(pattern: &Pattern, input: &[u8]) -> Option<bool> {
    let prog = compile(pattern)?;
    if prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    crate::parallel_lex::lex_significant_parts_held(input, |parts| {
        let bases = Parts::bases_of(parts);
        let stream = Parts::new(parts, &bases);
        let absent = crate::prefilter::absent_guard_literals(pattern, input);
        Some(any_serial(&prog, input, &stream.cursor(), &absent, pattern.starts_with_resume()))
    })
}

/// A pattern compiled for this engine, for a caller that runs the engine
/// over a stream of its own and must not pay the compile on every call.
/// Opaque: the instruction form behind it is the engine's own.
pub struct Compiled(Program);

/// Compile `pattern` for this engine, or `None` when it needs the
/// set-reachability engine or carries more save slots than fit inline.
#[must_use]
pub fn compile_pattern(pattern: &Pattern) -> Option<Compiled> {
    let prog = compile(pattern)?;
    (prog.nslots <= MAX_SAVE_SLOTS).then_some(Compiled(prog))
}

/// [`scan_nfa_over`] with the program already compiled, for a caller running
/// one pattern over many token slices.
#[must_use]
pub fn scan_nfa_over_compiled(
    compiled: &Compiled,
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
) -> Vec<Span> {
    compiled.0.clock.refresh();
    let sig: Vec<usize> =
        (0..toks.len()).filter(|&i| toks[i].kind != TokenKind::Whitespace).collect();
    let stream = Stitched { toks, sig: &sig };
    scan_stream(&compiled.0, pattern, input, &stream, &stream)
}

/// The significant stream a device split carries: kind codes and absolute
/// byte spans, one entry a token. The atoms a device-eligible pattern is
/// built from read the kind alone, so a stream of this shape answers them
/// with no input bytes behind it.
pub struct KindSpans<'a> {
    kinds: &'a [u32],
    spans: &'a [(u32, u32)],
}

impl<'a> KindSpans<'a> {
    /// # Panics
    /// When the two are not one entry a token.
    #[must_use]
    pub fn new(kinds: &'a [u32], spans: &'a [(u32, u32)]) -> Self {
        assert_eq!(kinds.len(), spans.len(), "one kind and one span a token");
        KindSpans { kinds, spans }
    }
}

impl SigStream for KindSpans<'_> {
    #[inline]
    fn len(&self) -> usize {
        self.kinds.len()
    }
    #[inline]
    fn kind(&self, k: usize) -> TokenKind {
        TokenKind::from_code(self.kinds[k])
    }
    #[inline]
    fn span(&self, k: usize) -> (usize, usize) {
        let (a, b) = self.spans[k];
        (a as usize, b as usize)
    }
    fn longest_run(&self, kind: TokenKind, ceiling: usize) -> Option<usize> {
        let (mut run, mut best) = (0usize, 0usize);
        longest_run_in(self.kinds, kind.code(), ceiling, &mut run, &mut best).then_some(best)
    }
    fn first_at_or_after(&self, at: usize) -> usize {
        self.spans.partition_point(|&(s, _)| (s as usize) < at)
    }
}

/// The longest match end of every anchor in `lo..lo + out.len()` of `stream`,
/// as the device kernel's table holds them: `out[i]` is the end significant
/// index of the longest non-empty match anchored at `lo + i`, or 0 where none
/// begins there. Every slot is written, so a caller may select over the whole
/// table.
///
/// `input` is read only by an atom that asks for a token's bytes; a pattern
/// built from kind atoms alone asks for none, and an empty slice serves it.
pub fn anchor_ends_into(
    prog: &Compiled,
    input: &[u8],
    stream: &(impl SigStream + ?Sized),
    lo: usize,
    out: &mut [i32],
) {
    let prog = &prog.0;
    let np = prog.insts.len();
    let keyed = prog.reads_registers;
    let mut clist = ThreadList::new(np, keyed);
    let mut nlist = ThreadList::new(np, keyed);
    let absent = HashSet::new();
    for (i, slot) in out.iter_mut().enumerate() {
        let a = lo + i;
        *slot = match find_leftmost(prog, input, stream, a, &absent, a + 1, &mut clist, &mut nlist) {
            Some(s) if s.get(1) > s.get(0) => s.get(1) as u32 as i32,
            _ => 0,
        };
    }
}

/// The leftmost, non-overlapping matches of `prog` whose start lies below
/// `until`, written into `out` at their start anchors and nothing else
/// written, with the significant index the walk left off at.
///
/// One forward pass carrying its thread set, so an anchor a match covers is
/// stepped over rather than attempted, and no end is computed for an anchor
/// the walk does not land on. That is sound for a table a leftmost,
/// non-overlapping selection reads and for nothing else: the selection visits
/// anchors in increasing order and jumps past each match it takes, so it
/// lands on exactly the anchors this walk landed on. A caller that needs an
/// end at every anchor wants [`anchor_ends_into`].
///
/// `out` covers anchors `0..out.len()` and is left as the caller filled it
/// where no match begins. `input` is read as [`anchor_ends_into`] reads it.
pub fn walk_prefix_into(
    prog: &Compiled,
    input: &[u8],
    stream: &(impl SigStream + ?Sized),
    until: usize,
    out: &mut [i32],
) -> usize {
    let prog = &prog.0;
    let np = prog.insts.len();
    let keyed = prog.reads_registers;
    let mut clist = ThreadList::new(np, keyed);
    let mut nlist = ThreadList::new(np, keyed);
    let absent = HashSet::new();
    let mut from = 0usize;
    while from < until {
        let Some(saves) = find_leftmost(prog, input, stream, from, &absent, until, &mut clist, &mut nlist)
        else {
            return from;
        };
        let (ks, ke) = (saves.get(0), saves.get(1));
        if ke <= ks {
            // An empty match at `ks`; step past it to make progress, as the
            // serial walk does.
            from = ks + 1;
            continue;
        }
        out[ks] = ke as u32 as i32;
        from = ke;
    }
    from
}

/// The kind a pattern's one unbounded repeat consumes, and how many tokens the
/// rest of the pattern can span, when the pattern is bounded apart from it.
///
/// `"let" \W* "="` carries no bound of its own, because a star may run to the
/// end of the stream. But that star consumes one kind, so what it can actually
/// take is a run of that kind - and the rest of the pattern is bounded. Two
/// unbounded repeats, or one over anything but a single kind atom, are refused:
/// the first has no single run to measure and the second no kind to measure it
/// over.
fn open_repeat_kind(pat: &Pattern) -> Option<(TokenKind, usize)> {
    let over_one_kind = |body: &Pattern| match body {
        Pattern::Atom(Atom::Kind(k)) => Some((*k, 0usize)),
        _ => None,
    };
    match pat {
        Pattern::Star(body, _) | Pattern::Plus(body, _) | Pattern::Repeat(body, _, None, _) => {
            over_one_kind(body)
        }
        Pattern::Bind(_, _, p) | Pattern::Atomic(p) => open_repeat_kind(p),
        Pattern::Concat(v) => {
            let (mut open, mut fixed) = (None, 0usize);
            for part in v {
                if let Some(n) = bounded_max_len(part) {
                    fixed += n;
                    continue;
                }
                if open.is_some() {
                    return None;
                }
                open = Some(open_repeat_kind(part)?);
            }
            open.map(|(k, inner)| (k, fixed + inner))
        }
        _ => None,
    }
}

/// The longest match `pat` can have over `stream`, where only a repeat over one
/// kind unbounds it, or `None` where the pattern has no such shape or the bound
/// would exceed `ceiling`.
///
/// The run scan stops as soon as the bound passes the ceiling, because a caller
/// asking this is deciding whether a bound small enough to dispatch on exists,
/// and once it does not the rest of the stream cannot change that.
fn stream_bounded_max_len<S: SigStream + ?Sized>(
    pat: &Pattern,
    stream: &S,
    ceiling: usize,
) -> Option<usize> {
    let (kind, fixed) = open_repeat_kind(pat)?;
    if fixed > ceiling {
        return None;
    }
    let best = stream.longest_run(kind, ceiling - fixed)?;
    Some(fixed + best)
}

/// The buffers one anchored attempt needs, held by a caller making many.
///
/// A loop over tens of thousands of windows pays these per attempt otherwise:
/// two thread lists, each a vector the width of the program, and the index of
/// the significant tokens. Measured at 1.74 microseconds an attempt when they
/// were allocated per call, against about 150 nanoseconds of actual work.
pub struct AttemptScratch {
    clist: ThreadList,
    nlist: ThreadList,
    sig: Vec<usize>,
}

impl AttemptScratch {
    /// Scratch sized for `compiled`, reusable for every attempt against it.
    #[must_use]
    pub fn for_program(compiled: &Compiled) -> Self {
        let np = compiled.0.insts.len();
        let keyed = compiled.0.reads_registers;
        AttemptScratch {
            clist: ThreadList::new(np, keyed),
            nlist: ThreadList::new(np, keyed),
            sig: Vec::new(),
        }
    }
}

/// A program that is a flat run of atoms: the atom consuming each token, and
/// the save-slot base of the register wrapping it where there is one.
///
/// A match of such a program is its first [`Self::len`] significant tokens when
/// every atom accepts its own, so the attempt is that many tests and the
/// register spans are the tokens their bindings wrap. Held as the program's own
/// shape rather than the pattern's, so a change in what the compiler emits
/// fails the shape test and the simulation answers, instead of two readings of
/// one pattern disagreeing.
pub struct FlatShape {
    atoms: Vec<Atom>,
    slot: Vec<Option<usize>>,
    /// Where each atom's register belongs in a match, which is its index among
    /// the program's names.
    ///
    /// Held rather than looked up: the program's slots are a map keyed by name,
    /// and finding an atom's place in it is a walk of that map - which, done per
    /// match, is a tree traversal at every one of tens of thousands.
    reg_index: Vec<Option<usize>>,
    nslots: usize,
}

impl FlatShape {
    /// How many tokens a match spans.
    #[must_use]
    pub fn len(&self) -> usize {
        self.atoms.len()
    }

    /// Whether it spans none, which [`flat_shape`] never returns.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    /// Whether [`Self::regs_from_bytes`] can walk this shape at all: every atom
    /// is a plain literal or a word token, the two the bytes read directly.
    ///
    /// A shape test and not an answer. Whether a particular match walks is
    /// settled against its own bytes, and refused there.
    #[must_use]
    pub fn walks_from_bytes(&self) -> bool {
        self.atoms.iter().all(|a| {
            matches!(
                a,
                Atom::Literal(_, crate::orbit::OrbitGroup::Identity)
                    | Atom::Kind(TokenKind::Word)
            )
        })
    }
}

/// The flat shape of `compiled`, or `None` for every other program.
///
/// The instructions must be `Save(0)`, then one atom a token with its register
/// saves around it, then `Save(1)` and `Match`. A split, a jump, a guard or an
/// anchor is any other shape and is refused, as is a program reading a register
/// back, whose atoms answer from bindings an attempt has yet to make.
#[must_use]
pub fn flat_shape(compiled: &Compiled) -> Option<FlatShape> {
    let prog = &compiled.0;
    if prog.reads_registers || prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    let insts = &prog.insts;
    if !matches!(insts.first(), Some(Inst::Save(0))) {
        return None;
    }
    let (mut atoms, mut slot) = (Vec::new(), Vec::new());
    let mut i = 1usize;
    loop {
        match insts.get(i)? {
            Inst::Save(1) => break,
            Inst::Save(base) => {
                let base = *base;
                let Some(Inst::Atom(a)) = insts.get(i + 1) else { return None };
                if !matches!(insts.get(i + 2), Some(Inst::Save(c)) if *c == base + 1) {
                    return None;
                }
                atoms.push(a.clone());
                slot.push(Some(base));
                i += 3;
            }
            Inst::Atom(a) => {
                atoms.push(a.clone());
                slot.push(None);
                i += 1;
            }
            _ => return None,
        }
    }
    if !matches!(insts.get(i + 1), Some(Inst::Match)) {
        return None;
    }
    if atoms.is_empty() {
        return None;
    }
    // The slot bases in the order the program's names hold them, which is the
    // order a match carries its register spans.
    let order: Vec<usize> = prog.slots.values().copied().collect();
    let reg_index: Vec<Option<usize>> =
        slot.iter().map(|s| s.and_then(|b| order.iter().position(|&o| o == b))).collect();
    Some(FlatShape { atoms, slot, reg_index, nslots: prog.nslots })
}

/// Whether the first `shape.len()` significant tokens of `toks` each match
/// their atom, with `sig` left holding the significant index.
fn flat_accepts(
    compiled: &Compiled,
    shape: &FlatShape,
    input: &[u8],
    toks: &[Token],
    sig: &mut Vec<usize>,
) -> bool {
    sig.clear();
    sig.extend((0..toks.len()).filter(|&i| toks[i].kind != TokenKind::Whitespace));
    if sig.len() < shape.atoms.len() {
        return false;
    }
    let stream = Stitched { toks, sig };
    // No atom here reads a save slot: `flat_shape` refuses a program that reads
    // a register back, and that is the only atom that does.
    shape
        .atoms
        .iter()
        .enumerate()
        .all(|(k, a)| atom_matches(a, &compiled.0, input, &stream, k, &[]))
}

/// [`match_at_first_token`] for a program [`flat_shape`] accepted, one test a
/// token instead of the simulation.
#[must_use]
pub fn flat_match_at_first_token(
    compiled: &Compiled,
    shape: &FlatShape,
    input: &[u8],
    toks: &[Token],
    scratch: &mut AttemptScratch,
) -> Option<Span> {
    let AttemptScratch { sig, .. } = scratch;
    if !flat_accepts(compiled, shape, input, toks, sig) {
        return None;
    }
    Some(span_between(&Stitched { toks, sig }, 0, shape.atoms.len()))
}

/// The register names `compiled` binds, in the order a match holds their spans
/// and a [`FlatMatch`] holds them inline.
#[must_use]
pub fn names_of(compiled: &Compiled) -> std::sync::Arc<[String]> {
    compiled.0.names.clone()
}

/// The most registers a flat program's match carries. The save slots hold the
/// whole match and a pair a register, and there are [`MAX_SAVE_SLOTS`].
pub const MAX_FLAT_REGISTERS: usize = (MAX_SAVE_SLOTS - 2) / 2;

/// A match and the registers it bound, held inline in the order the program's
/// names hold them.
///
/// What a window reports to a caller that copies the registers out rather than
/// keeping them. A [`Match`] owns a vector a match, and a cursor building one
/// per match before it yields any holds tens of thousands of live allocations
/// where one would do; the whole of that measured 1.6309 ms over 50000 matches,
/// against 3.0759 for the resolve it is part of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlatMatch {
    /// The match's own byte span.
    pub span: Span,
    /// The register spans, the first `names().len()` of them meaningful.
    pub regs: [Span; MAX_FLAT_REGISTERS],
}

impl FlatShape {
    /// The registers of the match at `span`, read off the bytes without a lex,
    /// or `None` where the atoms do not reconstruct the match exactly.
    ///
    /// A match of a flat shape is its atoms' tokens in order, so walking them
    /// forward from the span's start - a literal consuming its own bytes, a word
    /// atom the run of word bytes, whitespace skipped between - lands on each
    /// register's token. The walk must end exactly at the span's end: a pattern
    /// whose tokens the bytes read differently reconstructs some other match, or
    /// none, and is refused rather than reported.
    ///
    /// What it is for: a route that found a match without lexing should not have
    /// to lex a window to say what the match bound. `\W:name "="` scans from the
    /// bytes in 2.9445 ms and resolved in 3.5156 through a window a span, where
    /// the regex crate answers both in 3.0283.
    #[must_use]
    pub fn regs_from_bytes(&self, input: &[u8], span: Span) -> Option<FlatMatch> {
        let (lo, hi) = (span.start(), span.end());
        if hi > input.len() {
            return None;
        }
        let mut out =
            FlatMatch { span, regs: [Span { start: 0, end: 0 }; MAX_FLAT_REGISTERS] };
        let mut p = lo;
        let skip = |p: &mut usize| {
            while *p < hi && input[*p].is_ascii_whitespace() {
                *p += 1;
            }
        };
        for (k, atom) in self.atoms.iter().enumerate() {
            skip(&mut p);
            let start = p;
            match atom {
                Atom::Literal(lit, crate::orbit::OrbitGroup::Identity) => {
                    let lit = lit.as_bytes();
                    if !input[p..hi].starts_with(lit) {
                        return None;
                    }
                    p += lit.len();
                }
                Atom::Kind(TokenKind::Word) => {
                    while p < hi && (input[p].is_ascii_alphanumeric() || input[p] == b'_') {
                        p += 1;
                    }
                    if p == start {
                        return None;
                    }
                }
                _ => return None,
            }
            if let Some(i) = self.reg_index[k] {
                out.regs[i] = Span { start: start as u32, end: p as u32 };
            }
        }
        skip(&mut p);
        if p != hi {
            return None;
        }
        Some(out)
    }
}

/// [`flat_captures_at_first_token`] reporting the registers inline, for a
/// caller that reads them out rather than owning them.
#[must_use]
pub fn flat_regs_at_first_token(
    compiled: &Compiled,
    shape: &FlatShape,
    input: &[u8],
    toks: &[Token],
    scratch: &mut AttemptScratch,
) -> Option<FlatMatch> {
    let AttemptScratch { sig, .. } = scratch;
    if !flat_accepts(compiled, shape, input, toks, sig) {
        return None;
    }
    let n = shape.atoms.len();
    let stream = Stitched { toks, sig };
    let mut out =
        FlatMatch { span: span_between(&stream, 0, n), regs: [Span { start: 0, end: 0 }; MAX_FLAT_REGISTERS] };
    for (k, idx) in shape.reg_index.iter().enumerate() {
        if let Some(i) = *idx {
            out.regs[i] = span_between(&stream, k, k + 1);
        }
    }
    Some(out)
}

/// [`captures_at_first_token`] for a program [`flat_shape`] accepted.
///
/// The save slots are written from the shape rather than run for: the match
/// opens at token zero and closes past the last atom's, and a register's pair
/// is the token its binding wraps. Resolving them then goes through the same
/// function the simulation's slots go through, so the names and their order are
/// one answer and not two.
#[must_use]
pub fn flat_captures_at_first_token(
    compiled: &Compiled,
    shape: &FlatShape,
    input: &[u8],
    toks: &[Token],
    scratch: &mut AttemptScratch,
) -> Option<Match> {
    let AttemptScratch { sig, .. } = scratch;
    if !flat_accepts(compiled, shape, input, toks, sig) {
        return None;
    }
    let n = shape.atoms.len();
    let mut saves = [usize::MAX; MAX_SAVE_SLOTS];
    saves[0] = 0;
    saves[1] = n;
    for (k, base) in shape.slot.iter().enumerate() {
        if let Some(base) = *base {
            saves[base] = k;
            saves[base + 1] = k + 1;
        }
    }
    let stream = Stitched { toks, sig };
    let span = span_between(&stream, 0, n);
    Some(Match::bound(
        span.start(),
        span.end(),
        resolve_captures(&compiled.0, &saves[..shape.nslots], &stream),
        register_names(&compiled.0),
    ))
}

#[cfg(test)]
mod flat_shape_tests {
    use super::{
        AttemptScratch, Stitched, captures_at_first_token, compile_pattern, flat_captures_at_first_token,
        flat_match_at_first_token, flat_shape, match_at_first_token,
    };

    /// The atom-a-token attempt answers what the simulation answers, anchored
    /// at every token of a corpus, for both the span and the registers.
    #[test]
    fn the_flat_attempt_answers_what_the_simulation_answers() {
        let mut text = String::new();
        for i in 0..120u32 {
            text.push_str(&format!("let value_{i} = {} ; call_{i}(alpha, beta) ;\n", i * 37));
        }
        let input = text.as_bytes();
        let toks = crate::lexer::lex(input);
        let absent = std::collections::HashSet::new();
        // The shapes the window routes take, each of which the program must be
        // flat for, and three the simulation must keep.
        for (src, flat) in [
            ("\"let\" \\W \"=\"", true),
            ("\"let\" \\W:v \"=\"", true),
            ("\\W:name \"=\"", true),
            ("\"alpha\"", true),
            ("\\W \\W", true),
            ("(\"let\" | \"call\")", false),
            ("\"let\" \\W*", false),
            ("\\W:v \"=\" =v", false),
        ] {
            let p = crate::parser::parse(src).expect("parses");
            let compiled = compile_pattern(&p).expect("compiles");
            let shape = flat_shape(&compiled);
            assert_eq!(shape.is_some(), flat, "{src}");
            let Some(shape) = shape else { continue };
            let mut sim = AttemptScratch::for_program(&compiled);
            let mut fast = AttemptScratch::for_program(&compiled);
            let mut answered = 0usize;
            for i in 0..toks.len() {
                let cut = &toks[i..];
                let want = match_at_first_token(&compiled, input, cut, &mut sim, &absent);
                let got = flat_match_at_first_token(&compiled, &shape, input, cut, &mut fast);
                assert_eq!(got, want, "{src} anchored at token {i}");
                let want = captures_at_first_token(&compiled, input, cut, &mut sim, &absent);
                let got = flat_captures_at_first_token(&compiled, &shape, input, cut, &mut fast);
                let seen = |m: &Option<super::Match>| {
                    m.as_ref().map(|m| (m.start, m.end, m.captures().to_vec(), m.names().to_vec()))
                };
                assert_eq!(seen(&got), seen(&want), "{src} anchored at token {i}");
                answered += usize::from(want.is_some());
            }
            assert!(answered > 50, "{src} matched at {answered} of {} anchors", toks.len());
        }
    }

    /// A pattern bounded only by the stream is dispatched, and reports what the
    /// serial sweep reports.
    ///
    /// The bound comes from the input, so the same pattern is dispatched over
    /// one corpus and swept over another; both must answer the same, and the
    /// long-run corpus is where the bound is refused and the sweep takes over.
    #[test]
    fn a_pattern_bounded_by_the_stream_answers_what_the_sweep_answers() {
        use super::{bounded_max_len, open_repeat_kind, scan_nfa, scan_nfa_over_serial};
        let mut short = String::new();
        for i in 0..2_000u32 {
            short.push_str(&format!("let value_{i} = {i} ; call_{i}(alpha beta) ;\n"));
        }
        // One statement whose word run is longer than any dispatch would take,
        // so the bound is refused and the sweep answers instead.
        let mut long = short.clone();
        long.push_str("let ");
        for i in 0..4_000u32 {
            long.push_str(&format!("w{i} "));
        }
        long.push_str("= 1 ;\n");
        for src in ["\"let\" \\W* \"=\"", "\"let\" \\W+ \"=\"", "\\W* \"=\"", "\"let\" \\W{1,}"] {
            let p = crate::parser::parse(src).expect("parses");
            assert!(bounded_max_len(&p).is_none(), "{src} carries no bound of its own");
            assert!(open_repeat_kind(&p).is_some(), "{src} is unbounded only by a repeat of one kind");
            for text in [&short, &long] {
                let input = text.as_bytes();
                let toks = crate::lexer::lex(input);
                assert_eq!(
                    scan_nfa(&p, input).expect("the single-pass engine takes this"),
                    scan_nfa_over_serial(&p, input, &toks).expect("and so does the sweep"),
                    "{src} over {} bytes",
                    input.len()
                );
            }
        }
        // Shapes with no single run to measure: two open repeats, and one over
        // something that is not a single kind atom.
        for src in ["\\W* \"=\" \\N*", "(\\W | \\N)* \"=\"", "\"let\" .* \"=\""] {
            let p = crate::parser::parse(src).expect("parses");
            assert!(open_repeat_kind(&p).is_none(), "{src} has no one kind to bound");
        }
    }

    /// A window holding fewer tokens than the shape spans has no match in it,
    /// and the two attempts agree on that rather than reading past the cut.
    #[test]
    fn a_window_too_short_for_the_shape_matches_nothing() {
        let input = b"let a = 1 ;";
        let toks = crate::lexer::lex(input);
        let absent = std::collections::HashSet::new();
        let p = crate::parser::parse("\"let\" \\W:v \"=\"").expect("parses");
        let compiled = compile_pattern(&p).expect("compiles");
        let shape = flat_shape(&compiled).expect("the program is flat");
        let sig = Stitched::significant_of(&toks);
        let cut = &toks[..sig[2]];
        let mut sim = AttemptScratch::for_program(&compiled);
        let mut fast = AttemptScratch::for_program(&compiled);
        assert_eq!(match_at_first_token(&compiled, input, cut, &mut sim, &absent), None);
        assert_eq!(flat_match_at_first_token(&compiled, &shape, input, cut, &mut fast), None);
    }
}

/// The match anchored at the first significant token of `toks`, or `None` where
/// none begins there.
///
/// The per-anchor unit, over a token slice the caller has already cut, with the
/// attempt's buffers passed in rather than made. `absent` is the guard-literal
/// set; an empty one is correct for a pattern with no guard.
#[must_use]
pub fn match_at_first_token(
    compiled: &Compiled,
    input: &[u8],
    toks: &[Token],
    scratch: &mut AttemptScratch,
    absent: &HashSet<Vec<u8>>,
) -> Option<Span> {
    let AttemptScratch { clist, nlist, sig } = scratch;
    let saves = attempt_at_first_token(compiled, input, toks, clist, nlist, sig, absent)?;
    let stream = Stitched { toks, sig };
    Some(span_between(&stream, saves.get(0), saves.get(1)))
}

/// [`match_at_first_token`] with the registers the attempt bound.
///
/// Two entry points rather than one returning both: a plain-match path takes
/// the eight-byte span and never builds a capture list, which is the split the
/// match type carries.
#[must_use]
pub fn captures_at_first_token(
    compiled: &Compiled,
    input: &[u8],
    toks: &[Token],
    scratch: &mut AttemptScratch,
    absent: &HashSet<Vec<u8>>,
) -> Option<Match> {
    let AttemptScratch { clist, nlist, sig } = scratch;
    let saves = attempt_at_first_token(compiled, input, toks, clist, nlist, sig, absent)?;
    let stream = Stitched { toks, sig };
    let span = span_between(&stream, saves.get(0), saves.get(1));
    Some(Match::bound(
        span.start(),
        span.end(),
        resolve_captures(&compiled.0, saves.as_slice(), &stream),
        register_names(&compiled.0),
    ))
}

/// The slots bound by the attempt anchored at the first significant token of
/// `toks`, with `sig` left holding the significant index it ran over so the
/// caller can read the stream it was found on.
fn attempt_at_first_token(
    compiled: &Compiled,
    input: &[u8],
    toks: &[Token],
    clist: &mut ThreadList,
    nlist: &mut ThreadList,
    sig: &mut Vec<usize>,
    absent: &HashSet<Vec<u8>>,
) -> Option<Saves> {
    sig.clear();
    sig.extend((0..toks.len()).filter(|&i| toks[i].kind != TokenKind::Whitespace));
    if sig.is_empty() {
        return None;
    }
    let stream = Stitched { toks, sig };
    let saves = find_leftmost(&compiled.0, input, &stream, 0, absent, 1, clist, nlist)?;
    (saves.get(1) > saves.get(0)).then_some(saves)
}

/// The leftmost, non-overlapping matches of `prog` over a stream, through
/// the per-anchor dispatch where it pays and the serial walk otherwise.
/// `shared` is the stream's form a leaf of the dispatch reads, `walked` the
/// form the serial walk reads; for the stitched stream they are one.
fn scan_stream<P: LeafRead + ?Sized, W: SigStream + ?Sized>(
    prog: &Program,
    pattern: &Pattern,
    input: &[u8],
    shared: &P,
    walked: &W,
) -> Vec<Span> {
    let m = shared.len();

    // Guard literals a prefilter proves are nowhere in the input. The
    // build is skipped (the set is empty) when the pattern has no guard.
    let guarding = crate::trace::phase("the single pass: the absent guards");
    let absent = crate::prefilter::absent_guard_literals(pattern, input);
    drop(guarding);

    // A `\G` run is sequential by definition - whether a match may be kept
    // depends on where the previous one ended - so it cannot use the
    // per-anchor decomposition, which computes every start independently.
    // A `\K` match starts after its anchor, which the per-anchor table of
    // ends does not carry, so that pattern runs serially too.
    let contiguous = pattern.starts_with_resume();
    // The longest a match can be: from the pattern's own shape where it has
    // one, and from the stream where the only thing unbounding it is a repeat
    // over a single kind, which can run no further than that kind's longest run
    // here. Both are real bounds, and the dispatch's linear-work guarantee
    // rests on the bound rather than on where it came from.
    let bounding = crate::trace::phase("the single pass: the bound on a match");
    let bound = bounded_max_len(pattern)
        .or_else(|| stream_bounded_max_len(pattern, walked, PARALLEL_SCAN_MAX_MATCH_LEN));
    drop(bounding);
    let parallel = !contiguous
        && !pattern.mentions_reset_start()
        && m >= PARALLEL_SCAN_MIN_ANCHORS
        && matches!(bound, Some(1..=PARALLEL_SCAN_MAX_MATCH_LEN));

    // Named apart in the trace: the per-anchor dispatch and the serial sweep
    // are different mechanisms, and a reading of one is not a reading of the
    // other. A pattern whose longest match is unbounded - a star over anything
    // but a single kind, a plus, an open repeat - never reaches the dispatch at
    // all, so work aimed at it must be sized against the patterns that do.
    if parallel {
        crate::trace::rung("scan", "an attempt an anchor, across the cores", input.len());
        scan_nfa_parallel(prog, input, shared, &absent)
    } else {
        crate::trace::rung("scan", "one serial sweep of the stream", input.len());
        let _sweeping = crate::trace::phase("the single pass: one serial sweep");
        scan_nfa_serial(prog, input, walked, &absent, contiguous)
    }
}

/// [`scan_nfa_over`] on one thread whatever the stream's size.
///
/// The per-anchor dispatch pays when the cores are otherwise idle. A caller
/// already running one scan per core - a batch of inputs, a chunked stream -
/// has no idle cores to give it, and a nested dispatch would only contend
/// with its own; this is the route for that caller. It is also what a
/// measurement compares the dispatched route against.
#[must_use]
pub fn scan_nfa_over_serial(pattern: &Pattern, input: &[u8], toks: &[Token]) -> Option<Vec<Span>> {
    scan_nfa_over_serial_from(pattern, input, toks, 0)
}

/// How far the soonest-ending match of `pattern` in `input` reaches, as a
/// byte offset, or `None` where nothing matches or this engine does not take
/// the pattern.
///
/// The counterpart of the regex crate's `shortest_match`, and the same
/// relationship to [`crate::find`]: the end reported here can be earlier,
/// because the question is where a match is first known to have occurred and
/// not which match the pattern prefers. A pattern with one way to match
/// reports the same end either way.
#[must_use]
pub fn shortest_end(pattern: &Pattern, input: &[u8], toks: &[Token], from: usize) -> Option<usize> {
    let prog = compile(pattern)?;
    if prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    let sig: Vec<usize> = (from.min(toks.len())..toks.len())
        .filter(|&i| toks[i].kind != TokenKind::Whitespace)
        .collect();
    let stream = Stitched { toks, sig: &sig };
    shortest_end_over(&prog, pattern, input, &stream, 0)
}

/// [`shortest_end`] over the significant stream in the lexer's parts, from the
/// first token starting at or after byte `at`.
///
/// The stitched form joins the chunks into one vector and then builds an index
/// of the significant tokens over all of them, both before the walk starts.
/// This reads the parts where they were lexed and addresses the stream by
/// index, so the only whole-input pass is the lex.
#[must_use]
pub fn shortest_end_from_byte(pattern: &Pattern, input: &[u8], at: usize) -> Option<usize> {
    let prog = compile(pattern)?;
    if prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    let stream = OwnedStream::over(input);
    let k = first_at_or_after(&stream, at);
    shortest_end_over(&prog, pattern, input, &stream, k)
}

/// [`shortest_end`] with the program already compiled, for a caller asking it
/// of one prefix after another.
#[must_use]
pub fn shortest_end_over_compiled(
    compiled: &Compiled,
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    from: usize,
) -> Option<usize> {
    let sig: Vec<usize> = (from.min(toks.len())..toks.len())
        .filter(|&i| toks[i].kind != TokenKind::Whitespace)
        .collect();
    let stream = Stitched { toks, sig: &sig };
    shortest_end_over(&compiled.0, pattern, input, &stream, 0)
}

/// How far the soonest-ending match beginning at or after significant index `k`
/// reaches, as a byte offset.
fn shortest_end_over<S: SigStream + ?Sized>(
    prog: &Program,
    pattern: &Pattern,
    input: &[u8],
    stream: &S,
    k: usize,
) -> Option<usize> {
    let absent = crate::prefilter::absent_guard_literals(pattern, input);
    let m = stream.len();
    let keyed = prog.reads_registers;
    let mut clist = ThreadList::new(prog.insts.len(), keyed);
    let mut nlist = ThreadList::new(prog.insts.len(), keyed);
    let saves = find_leftmost_ending(
        prog, input, stream, k, &absent, m + 1, &mut clist, &mut nlist, true,
    )?;
    let (ks, ke) = (saves.get(0), saves.get(1));
    if ke > ks && ke <= m {
        return Some(stream.span(ke - 1).1);
    }
    // A thread that accepted having consumed nothing ends where it began, and
    // where there is no token at all it began at the input's start.
    Some(if ks < m { stream.span(ks).0 } else { input.len() })
}

/// The first significant token of `stream` starting at or after byte `at`, or
/// the stream's length where none does: [`SigStream::first_at_or_after`], for
/// a caller holding the stream by reference.
fn first_at_or_after<S: SigStream + ?Sized>(stream: &S, at: usize) -> usize {
    stream.first_at_or_after(at)
}

/// [`scan_nfa_over_serial`] resuming at token index `from`, for a caller that
/// has already committed the matches before it.
///
/// The counterpart of [`crate::engine::scan_tokens_from`] on this engine, so a
/// caller that resumes a leftmost scan chunk by chunk can route the way
/// [`crate::engine::scan`] does rather than reaching for the set-reachability
/// engine because only that one could resume. `from` indexes `toks`, not the
/// significant subsequence.
#[must_use]
pub fn scan_nfa_over_serial_from(
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    from: usize,
) -> Option<Vec<Span>> {
    // The four costs of this entry, divided: a caller resuming chunk by chunk
    // reaches it once per push and pays all four again each time. The program is
    // compiled here rather than handed in, the significant indices are gathered
    // over every token from `from` on, and the absent guard literals are read
    // off `input` itself - only the last of the four is the walk proper.
    let compiling = crate::trace::phase("the resumed walk: compiling the program");
    let prog = compile(pattern)?;
    drop(compiling);
    if prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    let gathering = crate::trace::phase("the resumed walk: its significant indices");
    let sig: Vec<usize> = (from.min(toks.len())..toks.len())
        .filter(|&i| toks[i].kind != TokenKind::Whitespace)
        .collect();
    drop(gathering);
    let guarding = crate::trace::phase("the resumed walk: its absent guards");
    let absent = crate::prefilter::absent_guard_literals(pattern, input);
    drop(guarding);
    let simulating = crate::trace::phase("the resumed walk: the simulation");
    let stream = Stitched { toks, sig: &sig };
    let found = scan_nfa_serial(&prog, input, &stream, &absent, pattern.starts_with_resume());
    drop(simulating);
    Some(found)
}

/// [`scan_nfa_over_serial_from`] with the program already compiled, for a
/// caller resuming from the same token index over one prefix after another.
#[must_use]
pub fn scan_nfa_over_serial_from_compiled(
    compiled: &Compiled,
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    from: usize,
) -> Vec<Span> {
    compiled.0.clock.refresh();
    let sig: Vec<usize> = (from.min(toks.len())..toks.len())
        .filter(|&i| toks[i].kind != TokenKind::Whitespace)
        .collect();
    let absent = crate::prefilter::absent_guard_literals(pattern, input);
    let stream = Stitched { toks, sig: &sig };
    scan_nfa_serial(&compiled.0, input, &stream, &absent, pattern.starts_with_resume())
}

/// Serial leftmost, non-overlapping walk: find the leftmost match at or after
/// `from`, hand `emit` its start and end significant-token indices with its
/// save slots, resume past its end.
///
/// `emit` returning `false` ends the walk, for a caller that has its answer
/// from the matches so far and wants none of the stream past them read.
fn walk_serial<S: SigStream + ?Sized>(
    prog: &Program,
    input: &[u8],
    stream: &S,
    absent: &HashSet<Vec<u8>>,
    contiguous: bool,
    mut emit: impl FnMut(usize, usize, Saves) -> bool,
) {
    let np = prog.insts.len();
    let keyed = prog.reads_registers;
    let mut clist = ThreadList::new(np, keyed);
    let mut nlist = ThreadList::new(np, keyed);
    let mut from = 0;
    while let Some((ks, ke, saves)) =
        step_serial(prog, input, stream, absent, contiguous, &mut from, &mut clist, &mut nlist)
    {
        if !emit(ks, ke, saves) {
            break;
        }
    }
}

/// One match of the serial walk: the leftmost at or after `from`, with `from`
/// advanced past it, or `None` where the walk has nothing further.
///
/// The stepping lives here rather than inside [`walk_serial`]'s loop so that a
/// caller taking matches one at a time and a caller taking them all run the
/// same code. Two implementations of leftmost-and-resume would be two chances
/// to disagree about where a match ends, and the whole contract between the
/// eager scan and the cursor is that they do not.
///
/// The thread lists are the caller's, so a walk held between matches keeps its
/// allocations rather than rebuilding them per match.
///
/// Every argument is a distinct part of one walk's state - the program, the
/// input, the stream over it, the literals a guard has already ruled out, the
/// anchor's contiguity, the resume point and the two thread lists - so there
/// is no grouping of them that is not just a struct named after this call.
#[allow(clippy::too_many_arguments)]
fn step_serial<S: SigStream + ?Sized>(
    prog: &Program,
    input: &[u8],
    stream: &S,
    absent: &HashSet<Vec<u8>>,
    contiguous: bool,
    from: &mut usize,
    clist: &mut ThreadList,
    nlist: &mut ThreadList,
) -> Option<(usize, usize, Saves)> {
    let m = stream.len();
    while *from <= m {
        let Some(saves) = find_leftmost(prog, input, stream, *from, absent, m + 1, clist, nlist)
        else {
            // No match at or after the resume point is a verdict over the rest
            // of the stream, so the walk moves past its end. Leaving the resume
            // point where it was would make a caller that asks again search
            // from there to the end once more, and a cursor is an iterator that
            // callers do ask again.
            *from = m + 1;
            return None;
        };
        let ks = saves.get(0);
        // `\G` requires the match to begin exactly where the scan resumed.
        // `find_leftmost` is free to skip ahead to the next position that
        // matches, and that skip is what the anchor forbids, so a match found
        // further on ends the run.
        if contiguous && ks != *from {
            return None;
        }
        let ke = saves.get(1);
        if ke > ks {
            *from = ke;
            return Some((ks, ke, saves));
        }
        // An empty match at `ks`; step past it to make progress.
        *from = ks + 1;
    }
    None
}

/// A serial walk held between matches, so a caller can take them one at a time.
///
/// It holds its token stream, which is what distinguishes it from the walks
/// above: those run inside the closure that holds the lexer's chunk parts and
/// end when it does, and a walk outlives any such closure. The stream is a
/// parameter because the two callers want opposite things from it: a cursor
/// outlives the lex and takes an [`OwnedStream`], and a pattern set asks many
/// patterns about one lex and lends each of them a [`Stitched`] over it.
pub(crate) struct SerialWalk<S: Shardable> {
    prog: Program,
    stream: S,
    absent: HashSet<Vec<u8>>,
    contiguous: bool,
    from: usize,
    clist: ThreadList,
    nlist: ThreadList,
    /// Whether the pattern is one the per-anchor decomposition takes.
    sharded: bool,
    /// How many anchors the next window covers.
    window: usize,
    /// The last window's matches as significant-token bounds, in order.
    batch: Vec<(usize, usize)>,
    /// How many of `batch` have been handed out.
    taken: usize,
    /// How many matches this walk has handed out since it was made or sought.
    handed: usize,
    /// The window's per-anchor ends, kept so a walk allocates the table once.
    ends: Vec<u32>,
    /// The save slots of every match in `batch`, in the same order. Empty until
    /// a caller asks for the first of them, and as long as `batch` once it has.
    saves: Vec<Saves>,
}

impl SerialWalk<OwnedStream> {
    /// A walk owning its stream, for a caller outliving any scope the lexer
    /// could lend one for.
    ///
    /// The lex runs only once the pattern has compiled, so a pattern this
    /// engine declines costs the compile and not the input.
    pub(crate) fn over(pattern: &Pattern, input: &[u8]) -> Option<Self> {
        let prog = compile(pattern)?;
        if prog.nslots > MAX_SAVE_SLOTS {
            return None;
        }
        Some(Self::with_stream(prog, pattern, input, OwnedStream::over(input)))
    }

    /// Whether `pattern` matches anywhere in `input`, or `None` for a pattern
    /// this engine does not take.
    ///
    /// [`Self::next_span`] steps to its first match serially, because a caller
    /// taking one and stopping is what a held walk exists for. A yes-or-no
    /// caller is not that caller: it prefers no match over another, and the
    /// ladder in front of it has already tried a prefix, so what reaches here
    /// matches late or not at all. Both of those want the windows from the
    /// first ask, which is the one thing this changes.
    pub(crate) fn any_match(pattern: &Pattern, input: &[u8]) -> Option<bool> {
        let mut walk = Self::over(pattern, input)?;
        walk.handed = 1;
        Some(walk.next_span(input).is_some())
    }
}

impl<S: Shardable> SerialWalk<S> {
    /// A walk over a stream the caller holds, for one asking several patterns
    /// about one lex: a pattern set pays the lex once and no member copies it.
    pub(crate) fn over_stream(pattern: &Pattern, input: &[u8], stream: S) -> Option<Self> {
        let prog = compile(pattern)?;
        if prog.nslots > MAX_SAVE_SLOTS {
            return None;
        }
        Some(Self::with_stream(prog, pattern, input, stream))
    }

    fn with_stream(prog: Program, pattern: &Pattern, input: &[u8], stream: S) -> Self {
        let np = prog.insts.len();
        let keyed = prog.reads_registers;
        SerialWalk {
            absent: crate::prefilter::absent_guard_literals(pattern, input),
            contiguous: pattern.starts_with_resume(),
            // The three conditions [`scan_stream`] puts on the pattern before
            // it dispatches, which hold for a window of anchors as they hold
            // for all of them. Its fourth is on the stream's size and is asked
            // of what remains, in [`Self::fill_window`].
            sharded: !pattern.starts_with_resume()
                && !pattern.mentions_reset_start()
                && matches!(bounded_max_len(pattern), Some(1..=PARALLEL_SCAN_MAX_MATCH_LEN)),
            prog,
            stream,
            from: 0,
            clist: ThreadList::new(np, keyed),
            nlist: ThreadList::new(np, keyed),
            window: PARALLEL_SCAN_MIN_ANCHORS,
            batch: Vec::new(),
            taken: 0,
            handed: 0,
            ends: Vec::new(),
            saves: Vec::new(),
        }
    }

    /// The next match's significant-token bounds, from the window already
    /// resolved or by resolving windows until one holds a match.
    ///
    /// `None` where this walk shards nothing, where the pattern is not one the
    /// per-anchor decomposition takes, where what remains of the stream is
    /// below the floor at which dispatching pays, or where no match has been
    /// handed out yet. The walk's resume point is left past every window read,
    /// so the serial step continues from there.
    ///
    /// The first match after a construction or a [`Self::seek`] is stepped to
    /// serially. A caller taking one match and stopping is what this walk
    /// exists for, and the leftmost search reaches an early match in a few
    /// attempts where a window resolves every anchor in it.
    fn next_batched(&mut self, input: &[u8]) -> Option<(usize, usize)> {
        if self.handed == 0 {
            return None;
        }
        while self.taken == self.batch.len() {
            if !self.fill_window(input) {
                return None;
            }
        }
        let out = self.batch[self.taken];
        self.taken += 1;
        self.handed += 1;
        Some(out)
    }

    /// Resolve one window of anchors into `batch`, and report whether one was
    /// resolved. The window may hold no match, which is not the same as there
    /// being none left.
    ///
    /// A window bounds where a match may BEGIN and not what an attempt may
    /// read, so a match anchored inside it runs to its own end and no boundary
    /// is repaired. The selection over the window is the fold [`select_ends`]
    /// runs over the whole table, whose only state is the end of the last
    /// match taken, so the next window starts at that end or past the window,
    /// whichever is further.
    fn fill_window(&mut self, input: &[u8]) -> bool {
        if !self.sharded {
            return false;
        }
        let Self { prog, stream, absent, from, window, batch, taken, ends, saves, .. } = self;
        let Some(shard) = stream.shard() else {
            return false;
        };
        let m = shard.len();
        if *from >= m || m - *from < PARALLEL_SCAN_MIN_ANCHORS {
            return false;
        }
        let win = (*window).min(m - *from);
        ends.clear();
        ends.resize(win, 0);
        anchor_ends_across(&*prog, input, &shard, &*absent, *from, ends);
        batch.clear();
        // The slots belong to the batch being replaced, so they go with it and
        // are resolved again only if this batch's are asked for.
        saves.clear();
        *taken = 0;
        let mut at = *from;
        while at < *from + win {
            let ke = ends[at - *from] as usize;
            if ke > at {
                batch.push((at, ke));
                at = ke;
            } else {
                at += 1;
            }
        }
        *from = at;
        // A caller taking one match pays the smallest window that dispatches
        // at a profit; one taking every match reaches the whole stream in a
        // logarithmic number of them.
        *window = window.saturating_mul(2);
        true
    }

    /// The next match's significant-token bounds.
    fn next_bounds(&mut self, input: &[u8]) -> Option<(usize, usize)> {
        if let Some(bounds) = self.next_batched(input) {
            return Some(bounds);
        }
        let Self { prog, stream, absent, contiguous, from, clist, nlist, handed, .. } = self;
        let (ks, ke, _) =
            step_serial(&*prog, input, &*stream, &*absent, *contiguous, from, clist, nlist)?;
        *handed += 1;
        Some((ks, ke))
    }

    /// The save slots of every match in the window, resolved unless they are
    /// already.
    ///
    /// A window carries one end per anchor and no slots, so a match taken from
    /// one is re-run as the attempt anchored at its start - the same call the
    /// window's table was filled by, and cheaper than the leftmost search it
    /// replaces, because it injects at one position rather than every one.
    /// Those attempts do not depend on one another, so they are dispatched the
    /// way the table itself was rather than run one at a time on whichever
    /// thread is taking the matches.
    ///
    /// The whole window is resolved on the first ask rather than one match at a
    /// time, because reaching any of these matches already cost an attempt at
    /// every anchor in the window, and a window holds no more matches than
    /// anchors. A caller wanting spans and not registers never reaches here.
    ///
    /// # Panics
    ///
    /// When the stream that a window was filled from no longer shards, or when
    /// the attempt at an anchor the window reported a match at finds none.
    fn fill_batch_saves(&mut self, input: &[u8]) {
        if self.saves.len() == self.batch.len() {
            return;
        }
        let Self { prog, stream, absent, batch, saves, .. } = self;
        let shard = stream.shard().expect("a window is filled only from a stream that shards");
        saves.resize(batch.len(), Saves::unset(prog.nslots));
        anchor_saves_across(
            &*prog,
            input,
            &shard,
            &*absent,
            batch.as_slice(),
            saves.as_mut_slice(),
        );
    }

    /// The next match's bounds with the save slots it bound.
    ///
    /// # Panics
    ///
    /// When the attempt at an anchor the window reported a match at finds none.
    fn next_saves(&mut self, input: &[u8]) -> Option<(usize, usize, Saves)> {
        if let Some((ks, ke)) = self.next_batched(input) {
            self.fill_batch_saves(input);
            return Some((ks, ke, self.saves[self.taken - 1]));
        }
        let Self { prog, stream, absent, contiguous, from, clist, nlist, handed, .. } = self;
        let found =
            step_serial(&*prog, input, &*stream, &*absent, *contiguous, from, clist, nlist)?;
        *handed += 1;
        Some(found)
    }

    /// The next match, or `None` where there is none left.
    pub(crate) fn next_span(&mut self, input: &[u8]) -> Option<Span> {
        let (ks, ke) = self.next_bounds(input)?;
        Some(span_between(&self.stream, ks, ke))
    }

    /// The next match with the registers it bound, or `None` where there is
    /// none left.
    ///
    /// A serial step carries the save slots already, so resolving them here
    /// costs the resolution and not a second pass. A match taken from a window
    /// pays the anchored attempt [`Self::next_saves`] describes, which is what
    /// [`captures_over`] pays for every span it is given.
    pub(crate) fn next_match(&mut self, input: &[u8]) -> Option<Match> {
        let (ks, ke, saves) = self.next_saves(input)?;
        let span = span_between(&self.stream, ks, ke);
        Some(Match::bound(
            span.start(),
            span.end(),
            resolve_captures(&self.prog, saves.as_slice(), &self.stream),
            register_names(&self.prog),
        ))
    }

    /// The next match written into buffers the caller owns, or `None` where
    /// there is none left.
    ///
    /// Nothing is allocated here. [`Self::next_match`] builds a vector and
    /// clones a register name into it for every match; this writes each
    /// register's position into `spans` and `extents` at the index `order`
    /// gives its name, so a loop over many matches allocates once.
    ///
    /// Returns the match's byte span and the significant-token indices it ran
    /// between, which the walk already holds.
    pub(crate) fn next_into(
        &mut self,
        input: &[u8],
        order: &[String],
        spans: &mut [Option<Span>],
        extents: &mut [Option<(usize, usize)>],
    ) -> Option<(Span, usize, usize)> {
        let (ks, ke, saves) = self.next_saves(input)?;
        resolve_captures_into(&self.prog, saves.as_slice(), &self.stream, order, spans, extents);
        Some((span_between(&self.stream, ks, ke), ks, ke))
    }

    /// The next match and how many significant tokens it spans.
    ///
    /// The walk names a match by the significant-token indices it runs
    /// between, and everything above converts that pair to bytes and drops
    /// it. The token count is the difference, so it costs a subtraction.
    pub(crate) fn next_span_and_extent(&mut self, input: &[u8]) -> Option<(Span, usize)> {
        let (ks, ke) = self.next_bounds(input)?;
        Some((span_between(&self.stream, ks, ke), ke - ks))
    }

    /// Move the walk to the first significant token starting at or after byte
    /// `at`, so the next match found is the leftmost one from there.
    ///
    /// Any window already resolved is discarded: it covers anchors the walk is
    /// no longer at, and the caller is asking a first-match question again.
    pub(crate) fn seek(&mut self, at: usize) {
        self.from = first_at_or_after(&self.stream, at);
        self.batch.clear();
        self.saves.clear();
        self.taken = 0;
        self.handed = 0;
        self.window = PARALLEL_SCAN_MIN_ANCHORS;
    }
}

/// Serial leftmost, non-overlapping scan, as byte spans.
fn scan_nfa_serial<S: SigStream + ?Sized>(
    prog: &Program,
    input: &[u8],
    stream: &S,
    absent: &HashSet<Vec<u8>>,
    contiguous: bool,
) -> Vec<Span> {
    let mut matches = Vec::new();
    walk_serial(prog, input, stream, absent, contiguous, |ks, ke, _| {
        matches.push(span_between(stream, ks, ke));
        true
    });
    matches
}

/// Whether the walk finds any match at all, reading none of the stream past
/// the first. The answer is `!scan_nfa_serial(..).is_empty()`, reached without
/// the walk resuming past that match.
fn any_serial<S: SigStream + ?Sized>(
    prog: &Program,
    input: &[u8],
    stream: &S,
    absent: &HashSet<Vec<u8>>,
    contiguous: bool,
) -> bool {
    let mut found = false;
    walk_serial(prog, input, stream, absent, contiguous, |_, _, _| {
        found = true;
        false
    });
    found
}

/// Parallel leftmost, non-overlapping scan. The longest match anchored at
/// each position is an independent attempt, so they are computed across
/// cores through the work-stealing pool; the cheap leftmost,
/// non-overlapping selection then runs serially over the per-anchor
/// results and is byte-identical to [`scan_nfa_serial`]. This is the same
/// decomposition the GPU kernel uses (one anchor per thread), applied to
/// the CPU cores.
fn scan_nfa_parallel<S: LeafRead + ?Sized>(
    prog: &Program,
    input: &[u8],
    stream: &S,
    absent: &HashSet<Vec<u8>>,
) -> Vec<Span> {
    // The table is this thread's, held between scans so its pages stay
    // mapped: a table the length of the stream is written once a scan, and a
    // fresh one faults every page on its first touch, which the walk over
    // every anchor pays across the leaves and the literal-hit path pays on one
    // thread as it writes the ends in.
    let mut ends = HELD_ENDS.take();
    ends.clear();
    ends.resize(stream.len(), 0);
    // Each phase is entered once a scan and the attempts run inside the first:
    // a clock an anchor would cost more than most anchors do, so what the
    // attempts did is counted in the thread lists and handed over as they go.
    let attempting = crate::trace::phase("the single pass: an attempt an anchor");
    anchor_ends_across(prog, input, stream, absent, 0, &mut ends);
    drop(attempting);
    // The selection steps forward over the whole table, so it reads through a
    // reader of its own for the same reason a leaf does.
    let selecting = crate::trace::phase("the single pass: the leftmost selection");
    let matches = select_ends(&stream.leaf(), &ends);
    drop(selecting);
    HELD_ENDS.set(ends);
    matches
}

thread_local! {
    /// This thread's table of anchor ends, held between scans; empty while a
    /// scan on this thread has it out.
    static HELD_ENDS: std::cell::Cell<Vec<u32>> = const { std::cell::Cell::new(Vec::new()) };
}

/// The longest match end of every anchor in `base..base + ends.len()`, filled
/// across cores: `ends[i]` is the end significant-token index of the longest
/// non-empty match anchored at `base + i`, and 0 where none begins there.
///
/// [`anchor_ends_into`] is the same table on one thread with no guard set, for
/// a device split to fill part of. A slot is written wherever an anchor is
/// attempted; where the opening is plain literals, only the anchors their
/// bytes fall on are, and every other slot stays as the caller filled it -
/// which both callers fill with zero, so a caller may select over the whole
/// table either way.
fn anchor_ends_across<S: LeafRead + ?Sized>(
    prog: &Program,
    input: &[u8],
    stream: &S,
    absent: &HashSet<Vec<u8>>,
    base: usize,
    ends: &mut [u32],
) {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    // The atoms a match may open with, computed once for the whole table. An
    // anchor none of them accepts cannot begin a match, and rejecting it costs
    // one token read where the attempt costs the thread lists, a save array and
    // an epsilon closure before it reads the same token.
    let opens = opening_atoms(prog);
    let opens = opens.as_deref();
    // An opening of plain literals names the only bytes a match can begin
    // with, so the anchors come from a search of the table's bytes for them
    // rather than from a test at every anchor.
    if let (Some(opens), Some(lits)) = (opens, opens.and_then(opening_literals)) {
        anchor_ends_at_literal_hits(prog, input, stream, absent, base, ends, opens, &lits);
        return;
    }
    // Each slot is independent, so the closure writes only its own range. The
    // leaf is a contiguous run of anchors, sized so the pool gets a few
    // balanced chunks per core: enough to steal against a skewed match
    // distribution, few enough that the split stays shallow.
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let min_leaf = ends.len().div_ceil(cores * 4).max(64);
    // PortCompute marks this as compute work and the per-item estimate
    // makes the pool's serial-vs-parallel choice depend on total work, not
    // on K_outer (which a token scan does not have).
    let plan = JobPlan::new(0, ends.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::PortCompute);
    for_each_chunk_indexed_min_leaf(&plan, ends, min_leaf, |start, slots| {
        // One pair of thread lists per leaf, reused across the leaf's
        // anchors, so the allocation cost is per leaf, not per anchor.
        let np = prog.insts.len();
        let keyed = prog.reads_registers;
        let mut clist = ThreadList::new(np, keyed);
        let mut nlist = ThreadList::new(np, keyed);
        // One reader per leaf, holding where the leaf last read. The leaf's
        // anchors ascend and each attempt steps forward from its own, so the
        // reader crosses into the next part about as often as the leaf spans
        // one - against a read that locates a token from scratch every time.
        let read = stream.leaf();
        for (i, slot) in slots.iter_mut().enumerate() {
            let a = base + start + i;
            if opens.is_some_and(|o| !opening_admits(prog, o, input, &read, a)) {
                *slot = 0;
                continue;
            }
            *slot = match find_leftmost(prog, input, &read, a, absent, a + 1, &mut clist, &mut nlist) {
                // A real match ends past its start; an empty match
                // (end == start) is not emitted, so it stores 0.
                Some(s) if s.get(1) > s.get(0) => s.get(1) as u32,
                _ => 0,
            };
        }
    });
}

/// [`anchor_ends_across`] where every opening atom is a plain literal.
///
/// The bytes the table's anchors span are searched for each literal, and the
/// attempts run only at the anchors those hits fall on: a hit no token starts
/// at, or one inside a longer token, is refused by the same opening check the
/// walk over every anchor made, so the table holds exactly what that walk
/// would have left in it. The search is over the table's own bytes rather
/// than the input's, so a walk that takes the stream in windows searches each
/// window once.
///
/// One slot a hit is filled across cores and written into the table after,
/// because the hits land on anchors a leaf of the table cannot claim as its
/// own range.
#[allow(clippy::too_many_arguments)]
fn anchor_ends_at_literal_hits<S: LeafRead + ?Sized>(
    prog: &Program,
    input: &[u8],
    stream: &S,
    absent: &HashSet<Vec<u8>>,
    base: usize,
    ends: &mut [u32],
    opens: &[&Atom],
    lits: &[&[u8]],
) {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    let last = (base + ends.len()).min(stream.len());
    if base >= last {
        return;
    }
    // Two phases a call, dividing it where the work does: the search over the
    // bytes, then the attempts at what it found.
    let searching = crate::trace::phase("the single pass: the search for the opening literals");
    let read = stream.leaf();
    let lo = read.span(base).0;
    let hi = read.span(last - 1).1;
    let mut hits: Vec<usize> = Vec::new();
    for lit in lits {
        hits.extend(crate::byte_simd::find_all(&input[lo..hi], lit).into_iter().map(|h| h + lo));
    }
    hits.sort_unstable();
    hits.dedup();
    drop(searching);
    crate::trace::counted("the single pass: hits of the opening literals", hits.len() as u64);

    let attempting = crate::trace::phase("the single pass: the attempts at the hits");
    // `None` where the hit begins no attempt, else the anchor and what the
    // attempt there found, 0 where it found nothing.
    let mut found: Vec<Option<(u32, u32)>> = vec![None; hits.len()];
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let min_leaf = found.len().div_ceil(cores * 4).max(64);
    let plan = JobPlan::new(0, found.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::PortCompute);
    for_each_chunk_indexed_min_leaf(&plan, &mut found, min_leaf, |start, slots| {
        let np = prog.insts.len();
        let keyed = prog.reads_registers;
        let mut clist = ThreadList::new(np, keyed);
        let mut nlist = ThreadList::new(np, keyed);
        // The hits a leaf takes ascend, so one reader serves the leaf.
        let read = stream.leaf();
        for (i, slot) in slots.iter_mut().enumerate() {
            let h = hits[start + i];
            let a = first_at_or_after(&read, h);
            if a < base || a >= last || read.span(a).0 != h || !opening_admits(prog, opens, input, &read, a) {
                continue;
            }
            let end = match find_leftmost(prog, input, &read, a, absent, a + 1, &mut clist, &mut nlist) {
                Some(s) if s.get(1) > s.get(0) => s.get(1) as u32,
                _ => 0,
            };
            *slot = Some((a as u32, end));
        }
    });
    drop(attempting);
    let mut admitted = 0u64;
    for slot in found.iter().flatten() {
        let (a, end) = *slot;
        admitted += 1;
        if end != 0 {
            ends[a as usize - base] = end;
        }
    }
    crate::trace::counted("the single pass: anchors the hits admit", admitted);
}

/// The save slots bound by the match anchored at the start of each of
/// `matches`, filled across cores: `saves[i]` is what the attempt anchored at
/// `matches[i].0` bound.
///
/// This is the per-anchor unit [`anchor_ends_across`] fills its table with,
/// kept whole rather than reduced to an end, and run only at the anchors the
/// selection over that table kept. Keeping every anchor's slots in the table
/// instead would cost the window's whole size in them, of which the selection
/// keeps a few.
///
/// # Panics
///
/// When the attempt at an anchor the window reported a match at finds none.
fn anchor_saves_across<S: LeafRead + ?Sized>(
    prog: &Program,
    input: &[u8],
    stream: &S,
    absent: &HashSet<Vec<u8>>,
    matches: &[(usize, usize)],
    saves: &mut [Saves],
) {
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let min_leaf = saves.len().div_ceil(cores * 4).max(64);
    // An attempt anchored where a match begins runs to that match's end, where
    // most anchors in the table fail at once, so the per-anchor estimate is a
    // floor here. A floor only makes the pool likelier to run this serially,
    // which is the safe direction for the estimate to be wrong in.
    let plan = JobPlan::new(0, saves.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::PortCompute);
    for_each_chunk_indexed_min_leaf(&plan, saves, min_leaf, |start, slots| {
        let np = prog.insts.len();
        let keyed = prog.reads_registers;
        let mut clist = ThreadList::new(np, keyed);
        let mut nlist = ThreadList::new(np, keyed);
        // The anchors a leaf resolves ascend, being the starts of leftmost
        // non-overlapping matches in order, so one reader serves the leaf.
        let read = stream.leaf();
        for (i, slot) in slots.iter_mut().enumerate() {
            let a = matches[start + i].0;
            *slot = find_leftmost(prog, input, &read, a, absent, a + 1, &mut clist, &mut nlist)
                .expect("the window's table was filled by this attempt at this anchor");
        }
    });
}

/// The leftmost, non-overlapping matches over per-anchor longest ends,
/// identical to the serial scan. `ends[a]` is the end significant-token index
/// of the longest non-empty match anchored at `a`, or 0 when none begins
/// there.
fn select_ends<S: SigStream + ?Sized>(stream: &S, ends: &[u32]) -> Vec<Span> {
    let m = stream.len();
    // The selection walk run once for the count, so the matches are written
    // into a vector of their final size rather than grown through copies.
    let mut count = 0usize;
    selected_anchors(ends, m, |_, _| count += 1);
    let mut matches = Vec::with_capacity(count);
    selected_anchors(ends, m, |a, ke| matches.push(span_between(stream, a, ke)));
    matches
}

/// The anchors a leftmost, non-overlapping selection over `ends` keeps, in
/// order, each with the significant-token index its match ends at.
///
/// `ends[a]` is zero where no match begins at `a`, and a non-empty match ends
/// past its own start, so an entry that is zero and one that does not reach
/// past its index are the same entry. That is what lets a run of anchors
/// beginning nothing be skipped rather than stepped: the fold's only state is
/// where the last match ended, and a zero carries none of it. The table is
/// mostly zeros - a scan of the comparison's corpus finds fifty thousand
/// matches over one and a third million anchors - so the skip is the walk and
/// the steps are the exception.
fn selected_anchors(ends: &[u32], m: usize, mut each: impl FnMut(usize, usize)) {
    let m = m.min(ends.len());
    let mut from = 0usize;
    while from < m {
        let Some(step) = ends[from..m].iter().position(|&e| e != 0) else {
            return;
        };
        let at = from + step;
        let ke = ends[at] as usize;
        debug_assert!(ke > at, "a non-empty match at {at} ends past it, not at {ke}");
        each(at, ke);
        from = ke;
    }
}

/// The captures of `spans`, matches of `pattern` over `toks`, each resolved
/// by the bounded attempt anchored at its start, the attempt the parallel
/// scan's selection is built from. A `\K` match starts after its anchor, so
/// that pattern's spans are resolved by the serial walk instead. `None` when
/// the pattern does not compile for this engine with its registers inline.
///
/// # Panics
///
/// When a span is not a match of `pattern` over `toks`.
pub(crate) fn captures_over(
    pattern: &Pattern,
    input: &[u8],
    toks: &[Token],
    spans: &[Span],
) -> Option<Vec<Match>> {
    let prog = compile(pattern)?;
    if prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    let sig: Vec<usize> =
        (0..toks.len()).filter(|&i| toks[i].kind != TokenKind::Whitespace).collect();
    let stream = Stitched { toks, sig: &sig };
    Some(captures_over_stream(&prog, pattern, input, &stream, &stream, spans))
}

/// [`captures_over`] over the significant stream in the lexer's parts, lexed
/// here rather than taken from a caller.
///
/// The stitched form joins the chunks into one vector and then indexes the
/// significant tokens over all of them, both before the first attempt runs.
/// This reads the parts where they were lexed, and hands the dispatch the
/// borrowed view of them that a leaf can share.
pub(crate) fn captures_over_parts(
    pattern: &Pattern,
    input: &[u8],
    spans: &[Span],
) -> Option<Vec<Match>> {
    let prog = compile(pattern)?;
    if prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    // The lex runs only once the pattern has compiled, so a pattern this
    // engine declines costs the compile and not the input.
    let owned = OwnedStream::over(input);
    Some(captures_over_stream(&prog, pattern, input, &owned.parts_view(), &owned, spans))
}

/// The registers each of `spans` bound, resolved over a stream the caller
/// holds. `shared` is the form a leaf of the dispatch reads and `walked` the
/// form a serial walk reads, as [`scan_stream`] takes them.
///
/// # Panics
///
/// When a span is not a match of `pattern` over the stream.
fn captures_over_stream<P: LeafRead + ?Sized, W: SigStream + ?Sized>(
    prog: &Program,
    pattern: &Pattern,
    input: &[u8],
    shared: &P,
    walked: &W,
    spans: &[Span],
) -> Vec<Match> {
    let absent = crate::prefilter::absent_guard_literals(pattern, input);
    if pattern.mentions_reset_start() {
        let mut out = Vec::with_capacity(spans.len());
        walk_serial(prog, input, walked, &absent, pattern.starts_with_resume(), |ks, ke, saves| {
            let span = span_between(walked, ks, ke);
            if spans.get(out.len()) == Some(&span) {
                out.push(Match::bound(
                    span.start(),
                    span.end(),
                    resolve_captures(prog, saves.as_slice(), walked),
                    register_names(prog),
                ));
            }
            true
        });
        assert_eq!(out.len(), spans.len(), "every span is a match of this pattern over this input");
        return out;
    }
    // Each span's registers come from its own bounded attempt, which reads
    // nothing the other spans write, so the resolutions are independent and go
    // across the cores the same way the scan's per-anchor attempts do. The
    // spans and their ends are already known, so only the captures are filled
    // here and the order is the caller's.
    use flynnel::JobPlan;
    use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

    let mut out: Vec<Match> = spans
        .iter()
        .map(|s| Match::plain(s.start(), s.end()))
        .collect();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let min_leaf = out.len().div_ceil(cores * 4).max(16);
    let plan = JobPlan::new(0, out.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::PortCompute);
    for_each_chunk_indexed_min_leaf(&plan, &mut out, min_leaf, |start, slots| {
        // One pair of thread lists per leaf, as the scan does: the pair is the
        // reason this loop was serial, being the one thing the attempts shared.
        let np = prog.insts.len();
        let keyed = prog.reads_registers;
        let mut clist = ThreadList::new(np, keyed);
        let mut nlist = ThreadList::new(np, keyed);
        // The spans a leaf resolves ascend, so one reader holding where the
        // leaf last read serves the whole leaf.
        let read = shared.leaf();
        for (i, slot) in slots.iter_mut().enumerate() {
            let span = spans[start + i];
            let a = first_at_or_after(&read, span.start());
            let saves = (a < read.len())
                .then(|| find_leftmost(prog, input, &read, a, &absent, a + 1, &mut clist, &mut nlist))
                .flatten()
                .filter(|s| s.get(1) > a && read.span(s.get(1) - 1).1 == span.end())
                .expect("a span a scan of this pattern returned is reproduced by the attempt at its start");
            *slot = Match::bound(
                slot.start,
                slot.end,
                resolve_captures(prog, saves.as_slice(), &read),
                register_names(prog),
            );
        }
    });
    out
}

/// Bit-parallel NFA tables for the GPU kernel. The active set of states
/// is a 64-bit mask of program counters; each atom state carries the
/// kind it requires and the epsilon-closure of its successor, so one
/// device thread can simulate the NFA from an anchor with no host help.
#[cfg(feature = "gpu")]
pub(crate) struct GpuNfa {
    /// Required kind code per state (`Atom::Any` is `u32::MAX`); read only
    /// at the atom states.
    pub atom_kind: Vec<u32>,
    /// Epsilon-closure of `pc + 1` per atom state.
    pub next_closure: Vec<u64>,
    /// `1` where the state consumes a token, else `0`.
    pub is_atom: Vec<u32>,
    /// Least token magnitude x100 per state; negative infinity where the
    /// state tests no magnitude.
    pub mag_lo: Vec<f32>,
    /// Greatest token magnitude x100 per state; infinity where the state
    /// tests no magnitude.
    pub mag_hi: Vec<f32>,
    /// Whether any state tests a magnitude, so the scan needs one per token.
    pub reads_magnitude: bool,
    /// Tokens back from each register reference to the token it compares
    /// against; 0 at every other state.
    pub ref_back: Vec<i32>,
    /// The orbit group every register reference and literal compares under,
    /// or `None` when the pattern has neither.
    pub class_group: Option<crate::orbit::OrbitGroup>,
    /// Whether the pattern binds a register, so its matches carry captures
    /// the host resolves.
    pub binds_registers: bool,
    /// Byte classes every byte of the token must hold, as
    /// [`byte_class_bit`] bits, per state; 0 where the state tests none.
    pub need_mask: Vec<u32>,
    /// Whether any state tests a byte class, so the scan needs a mask per
    /// token.
    pub reads_bytes: bool,
    /// The literal each state compares against, whose orbit class the host
    /// resolves per scan; `None` where the state is not a literal.
    pub literals: Vec<Option<Vec<u8>>>,
    /// Least pooled entropy x100 per state; negative infinity where the state
    /// tests none.
    pub ent_lo: Vec<f32>,
    /// Greatest pooled entropy x100 per state; infinity where the state tests
    /// none.
    pub ent_hi: Vec<f32>,
    /// The period test per state: 0 none, 1 any detected period, 2 the period
    /// in `period_val`.
    pub period_need: Vec<u32>,
    /// The period a state's equality test needs; 0 where it has none.
    pub period_val: Vec<u32>,
    /// The texture class id a state needs, or `u32::MAX` where it needs none.
    pub texture_need: Vec<u32>,
    /// 1 where the state needs a change-point inside the token, else 0.
    pub onset_need: Vec<u32>,
    /// Whether any state reads the spectral field, so the scan needs a
    /// reading per token.
    pub reads_spectral: bool,
    /// Epsilon-closure of the program entry.
    pub start_closure: u64,
    /// Bits of the accept states.
    pub match_mask: u64,
    /// Number of states (program counters); at most 64.
    pub nstates: usize,
}

/// The id the device compares a texture class by. Every class the field can
/// report has one, so a predicate for a class a token is not in fails on the
/// device as it does on the CPU.
pub(crate) fn texture_id(t: crate::spectral::Texture) -> u32 {
    use crate::spectral::Texture;
    match t {
        Texture::Prose => 0,
        Texture::Code => 1,
        Texture::Math => 2,
        Texture::Data => 3,
        Texture::Mixed => 4,
    }
}

/// [`texture_id`] of the class a spectral predicate names.
#[cfg(feature = "gpu")]
fn spec_texture_id(t: crate::ast::SpecTexture) -> u32 {
    use crate::ast::SpecTexture;
    use crate::spectral::Texture;
    texture_id(match t {
        SpecTexture::Prose => Texture::Prose,
        SpecTexture::Code => Texture::Code,
        SpecTexture::Math => Texture::Math,
        SpecTexture::Data => Texture::Data,
    })
}

/// The magnitude x100 bounds an absolute predicate puts on a state, compared
/// as [`crate::ast::MagPred::matches`] compares them; `None` for a relative
/// predicate, whose threshold is read from the stream before the token.
#[cfg(feature = "gpu")]
fn magnitude_bounds(pred: &crate::ast::MagPred) -> Option<(f32, f32)> {
    use crate::ast::MagPred;
    match pred {
        MagPred::Ge(centi) => Some((*centi as f32, f32::INFINITY)),
        MagPred::Le(centi) => Some((f32::NEG_INFINITY, *centi as f32)),
        MagPred::Above(..) | MagPred::Below(..) => None,
    }
}

/// The bit a byte class holds in a token's byte-class mask, or `None` for
/// whitespace, which the significant-token stream never carries.
pub(crate) fn byte_class_bit(bc: crate::ast::ByteClass) -> Option<u32> {
    use crate::ast::ByteClass;
    match bc {
        ByteClass::Digit => Some(1),
        ByteClass::Word => Some(2),
        ByteClass::Hex => Some(4),
        ByteClass::Alpha => Some(8),
        ByteClass::Upper => Some(16),
        ByteClass::Lower => Some(32),
        ByteClass::Space => None,
    }
}

/// Per program counter, how many tokens back from a register reference its
/// bound token sits, for a device that reads the bound token at that fixed
/// offset; with the orbit group every reference and literal compares under.
/// Offsets are 0 at every state that is not a reference.
///
/// `None` when some reference cannot be read that way: its register has no
/// slot, the bind it reads is not exactly one token wide on every path that
/// reaches it, the distance back to that bind differs between paths or passes
/// 64 tokens, or two references or literals compare under different groups.
fn fixed_bind_offsets(prog: &Program) -> Option<(Vec<i32>, Option<crate::orbit::OrbitGroup>)> {
    use std::collections::BTreeSet;
    // Tokens since the last `Save(base)`, and tokens from that save to the
    // `Save(base + 1)` closing it; `FAR` marks never saved, not yet closed, and
    // out of reach alike.
    const FAR: u8 = 65;
    let n = prog.insts.len();
    let mut offsets = vec![0i32; n];
    let mut group = None;
    let mut refs: Vec<(usize, usize)> = Vec::new();
    for (pc, inst) in prog.insts.iter().enumerate() {
        let g = match inst {
            Inst::Atom(Atom::RegisterEq(name, g)) => {
                refs.push((pc, *prog.slots.get(name)?));
                g
            }
            Inst::Atom(Atom::Literal(_, g)) => g,
            _ => continue,
        };
        match group {
            None => group = Some(*g),
            Some(seen) if seen == *g => {}
            Some(_) => return None,
        }
    }
    let bases: BTreeSet<usize> = refs.iter().map(|&(_, base)| base).collect();
    for base in bases {
        let mut reached: Vec<BTreeSet<(u8, u8)>> = vec![BTreeSet::new(); n];
        let mut work = vec![(0usize, (FAR, FAR))];
        while let Some((pc, (since, width))) = work.pop() {
            if pc >= n || !reached[pc].insert((since, width)) {
                continue;
            }
            match &prog.insts[pc] {
                Inst::Save(s) if *s == base => work.push((pc + 1, (0, FAR))),
                Inst::Save(s) if *s == base + 1 => work.push((pc + 1, (since, since))),
                Inst::Save(_) | Inst::Guard(..) | Inst::Anchor(_) => work.push((pc + 1, (since, width))),
                Inst::Jmp(t) => work.push((*t, (since, width))),
                Inst::Split(a, b) => {
                    work.push((*a, (since, width)));
                    work.push((*b, (since, width)));
                }
                Inst::Atom(_) => work.push((pc + 1, (since.saturating_add(1).min(FAR), width))),
                Inst::Match => {}
            }
        }
        for &(pc, _) in refs.iter().filter(|&&(_, b)| b == base) {
            let mut states = reached[pc].iter();
            let &(since, width) = states.next()?;
            if width != 1 || since == 0 || since >= FAR || states.any(|&s| s != (since, width)) {
                return None;
            }
            offsets[pc] = i32::from(since);
        }
    }
    Some((offsets, group))
}

/// Whether the device can scan `pattern`'s registers and literals: it compiles
/// for the linear engine with its registers inline, every reference reads a
/// one-token bind at a fixed distance, and every reference and literal
/// compares under one orbit group.
pub(crate) fn device_binds_fit(pattern: &Pattern) -> bool {
    compile(pattern).is_some_and(|prog| prog.nslots <= MAX_SAVE_SLOTS && fixed_bind_offsets(&prog).is_some())
}

/// Build the GPU bit-NFA for `pattern`, or `None` when it falls outside
/// the device subset: a program over 64 states (wider than the mask), a
/// balanced or field pattern (no compiled program), more registers than the
/// linear engine carries inline, a register reference that cannot be read at
/// a fixed offset ([`fixed_bind_offsets`]), or an instruction the kernel does
/// not implement (a guard or a byte-level atom). A spectral atom compiles
/// here, since the kernel carries a pooled spectral reading per token. The
/// caller routes the rest to the CPU engine.
#[cfg(feature = "gpu")]
pub(crate) fn compile_for_gpu(pattern: &Pattern) -> Option<GpuNfa> {
    let prog = compile_unless(pattern, true)?;
    let n = prog.insts.len();
    if n > 64 || prog.nslots > MAX_SAVE_SLOTS {
        return None;
    }
    let (ref_back, class_group) = fixed_bind_offsets(&prog)?;
    let mut atom_kind = vec![0u32; n];
    let mut next_closure = vec![0u64; n];
    let mut is_atom = vec![0u32; n];
    let mut mag_lo = vec![f32::NEG_INFINITY; n];
    let mut mag_hi = vec![f32::INFINITY; n];
    let mut reads_magnitude = false;
    let mut need_mask = vec![0u32; n];
    let mut reads_bytes = false;
    let mut literals: Vec<Option<Vec<u8>>> = vec![None; n];
    let mut ent_lo = vec![f32::NEG_INFINITY; n];
    let mut ent_hi = vec![f32::INFINITY; n];
    let mut period_need = vec![0u32; n];
    let mut period_val = vec![0u32; n];
    let mut texture_need = vec![u32::MAX; n];
    let mut onset_need = vec![0u32; n];
    let mut reads_spectral = false;
    let mut match_mask = 0u64;
    for pc in 0..n {
        match &prog.insts[pc] {
            Inst::Atom(a) => {
                is_atom[pc] = 1;
                atom_kind[pc] = match a {
                    Atom::Kind(k) => k.code(),
                    Atom::Any => u32::MAX,
                    Atom::Magnitude(pred) => {
                        (mag_lo[pc], mag_hi[pc]) = magnitude_bounds(pred)?;
                        reads_magnitude = true;
                        u32::MAX
                    }
                    Atom::KindMag(k, pred) => {
                        (mag_lo[pc], mag_hi[pc]) = magnitude_bounds(pred)?;
                        reads_magnitude = true;
                        k.code()
                    }
                    // Any kind; the kernel compares the token's orbit class
                    // with the class `ref_back[pc]` tokens earlier.
                    Atom::RegisterEq(..) => u32::MAX,
                    // Any kind; the kernel compares the token's orbit class
                    // with the literal's, which the host resolves per scan.
                    Atom::Literal(text, _) => {
                        literals[pc] = Some(text.as_bytes().to_vec());
                        u32::MAX
                    }
                    // Any kind; every byte of the token must hold the class.
                    Atom::Byte(bc) => {
                        need_mask[pc] = byte_class_bit(*bc)?;
                        reads_bytes = true;
                        u32::MAX
                    }
                    // Any kind; the kernel tests the token's pooled spectral
                    // reading, computed on the host as the set engine reads it.
                    Atom::Spectral(pred) => {
                        use crate::ast::SpectralPred;
                        match pred {
                            SpectralPred::EntropyGe(p) => ent_lo[pc] = f32::from(*p),
                            SpectralPred::EntropyLe(p) => ent_hi[pc] = f32::from(*p),
                            SpectralPred::PeriodAny => period_need[pc] = 1,
                            SpectralPred::PeriodEq(v) => {
                                period_need[pc] = 2;
                                period_val[pc] = u32::from(*v);
                            }
                            SpectralPred::Texture(t) => texture_need[pc] = spec_texture_id(*t),
                            SpectralPred::Onset => onset_need[pc] = 1,
                        }
                        reads_spectral = true;
                        u32::MAX
                    }
                    // The kernel compares a single kind code, so a set
                    // expression has no representation there.
                    Atom::Class(_) => return None,
                    // A byte pattern or a spectral atom has no representation
                    // in the kernel.
                    _ => return None,
                };
                next_closure[pc] = epsilon_closure(&prog.insts, pc + 1);
            }
            Inst::Match => match_mask |= 1u64 << pc,
            // The kernel simulates over the token-kind stream alone and has
            // neither the bytes nor the position tests these need.
            Inst::Guard(..) | Inst::Anchor(_) => return None,
            Inst::Split(..) | Inst::Jmp(_) | Inst::Save(_) => {}
        }
    }
    let start_closure = epsilon_closure(&prog.insts, 0);
    Some(GpuNfa {
        atom_kind,
        next_closure,
        is_atom,
        mag_lo,
        mag_hi,
        reads_magnitude,
        ref_back,
        class_group,
        binds_registers: !prog.slots.is_empty(),
        need_mask,
        reads_bytes,
        literals,
        ent_lo,
        ent_hi,
        period_need,
        period_val,
        texture_need,
        onset_need,
        reads_spectral,
        start_closure,
        match_mask,
        nstates: n,
    })
}

/// The set of atom and accept states reachable from `start` by epsilon
/// transitions (split, jump, save), as a bitmask of program counters.
#[cfg(feature = "gpu")]
fn epsilon_closure(insts: &[Inst], start: usize) -> u64 {
    let mut seen = 0u64;
    let mut result = 0u64;
    let mut stack = vec![start];
    while let Some(pc) = stack.pop() {
        if pc >= insts.len() || (seen >> pc) & 1 == 1 {
            continue;
        }
        seen |= 1u64 << pc;
        match &insts[pc] {
            Inst::Atom(_) | Inst::Match => result |= 1u64 << pc,
            Inst::Jmp(t) => stack.push(*t),
            Inst::Split(a, b) => {
                stack.push(*a);
                stack.push(*b);
            }
            Inst::Save(_) => stack.push(pc + 1),
            Inst::Guard(..) | Inst::Anchor(_) => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn bind_offsets(src: &str) -> Option<(Vec<i32>, Option<crate::orbit::OrbitGroup>)> {
        fixed_bind_offsets(&compile(&parse(src).unwrap()).expect("compiles for the linear engine"))
    }

    fn reference_offsets(src: &str) -> Option<Vec<i32>> {
        bind_offsets(src).map(|(o, _)| o.into_iter().filter(|&d| d > 0).collect())
    }

    #[test]
    fn a_one_token_bind_at_a_fixed_distance_gives_its_offset() {
        assert_eq!(reference_offsets("\\W:x =x"), Some(vec![1]));
        assert_eq!(reference_offsets("\\W:x \\N =x"), Some(vec![2]));
        assert_eq!(reference_offsets("(\\W:x =x)+"), Some(vec![1]));
        assert_eq!(reference_offsets("\\W:x \\N{2} =x \\W =x"), Some(vec![3, 5]));
        assert_eq!(reference_offsets("\\W:t \\N"), Some(vec![]));
        assert_eq!(
            bind_offsets("\\W:x =case x").and_then(|(_, g)| g),
            Some(crate::orbit::OrbitGroup::Case)
        );
    }

    #[test]
    fn a_reference_with_no_single_offset_is_refused() {
        for src in ["\\W+:x =x", "\\W:x \\N* =x", "\\W:x \\N? =x", "\\W:x =x \\W:y =case y", "(\\W:x)? =x", "\\W:x =case x \"the\""] {
            assert_eq!(bind_offsets(src), None, "{src} has no single fixed offset");
        }
    }

    /// A scan on this engine with its captures resolved, so a test reads a
    /// match the way a caller wanting the registers does.
    fn run(pattern: &str, input: &str) -> Vec<Match> {
        let p = parse(pattern).unwrap();
        let spans = scan_nfa(&p, input.as_bytes()).expect("nfa handles this pattern");
        crate::engine::captures(&p, input.as_bytes(), &spans)
    }

    #[test]
    fn number_then_word() {
        // A unit symbol one space after a number is that quantity's, so the
        // word this reads is one no unit table holds.
        let m = run("\\N \\W", "weight 12 items here");
        assert_eq!(m.len(), 1);
        assert_eq!(&"weight 12 items here"[m[0].start..m[0].end], "12 items");
        assert!(run("\\N \\W", "weight 12 kg here").is_empty(), "`12 kg` is one quantity token");
    }

    #[test]
    fn matched_tag_binds_and_rejects() {
        let hay = "<div>hi</div>";
        let ok = run("<\\W:t>.*</=t>", hay);
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].group("t", hay.as_bytes()), Some(b"div" as &[u8]));
        let bad = run("<\\W:t>.*</=t>", "<div>hi</span>");
        assert!(bad.is_empty());
    }

    #[test]
    fn repeated_token() {
        let ok = run("\\W:x =x", "the the cat");
        assert_eq!(ok.len(), 1);
        assert_eq!(&"the the cat"[ok[0].start..ok[0].end], "the the");
        assert!(run("\\W:x =x", "the cat sat").is_empty());
    }

    #[test]
    fn leftmost_first_prefers_the_earlier_branch() {
        let m = run("\\W | \\W \\W", "a b");
        assert_eq!(m.len(), 2);
        assert_eq!(&"a b"[m[0].start..m[0].end], "a");
    }

    #[test]
    fn leftmost_first_falls_back_when_the_continuation_fails() {
        // The short branch matches, then `"c"` contradicts it, so the longer
        // branch is taken. Committed choice would return nothing here, which
        // is what separates the two semantics.
        let m = run("(\"a\" | \"a\" \"b\") \"c\"", "a b c");
        assert_eq!(m.len(), 1);
        assert_eq!(&"a b c"[m[0].start..m[0].end], "a b c");
    }

    #[test]
    fn an_iteration_that_matched_empty_ranks_as_a_regular_expression_ranks_it() {
        // The shape the conformance sweep finds at depth four. After the
        // first alternative matches empty in a later iteration, the loop's
        // remaining alternatives outrank its exit, as they do in the `regex`
        // crate, so one match runs to the end of the input rather than two
        // splitting it.
        let input = "foo 969 8 foo foo qux";
        let m = run("((.*? | \\N | .))+ (\\W{1,} | . | \\N \\N) \\W", input);
        assert_eq!(m.len(), 1, "{m:?}");
        assert_eq!(&input[m[0].start..m[0].end], input);

        // A star over a body that can match empty leaves alone the token its
        // lazy body preferred not to take.
        let m = run("\\W (\\N*?)*", "qux 769");
        assert_eq!(m.len(), 1, "{m:?}");
        assert_eq!(&"qux 769"[m[0].start..m[0].end], "qux");
    }

    #[test]
    fn an_open_repeat_is_laid_out_as_the_plus_it_means() {
        // `P{1,}` and `P+` compile to the same instructions, so they rank
        // the same derivations the same way; `P{2,}` is `P P+`.
        let pairs = [
            ("(\\N*? | \\W){1,} \\N", "(\\N*? | \\W)+ \\N"),
            ("(. | \\N \\N){2,} \\W", "(. | \\N \\N) (. | \\N \\N)+ \\W"),
            ("(\\W*?){0,} \\N", "(\\W*?)* \\N"),
        ];
        for (a, b) in pairs {
            for input in ["1 2 a 3", "a 1 2 3 b", "1 1 1 1 a", "a b c 1"] {
                assert_eq!(run(a, input), run(b, input), "{a} against {b} over {input:?}");
            }
        }
    }

    #[test]
    fn balanced_routes_to_other_engine() {
        // A pattern with a balanced group is not handled here.
        let p = parse("\\W\\B(.*)").unwrap();
        assert!(scan_nfa(&p, b"f(x)").is_none());
    }

    #[test]
    fn differential_against_set_reachability() {
        // The single-pass engine must agree with the proven
        // set-reachability engine on every non-balanced pattern over a
        // corpus of inputs.
        let patterns = [
            "\\N",
            "\\W",
            "\\Q",
            ".",
            "\\N \\W",
            "\\W \\W",
            "<\\W:t>.*</=t>",
            "\\W:x =x",
            "\\N | \\W",
            ". ~\"END\"",
            "\\W*",
            "\\W+",
            "\\W?",
            "\\N{1,2}",
            "\"foo\"",
            "(\\N | \\W)+",
            "\\W:a \\W:b =a",
            "\\N \\N",
            ".*",
            "\\W \\N \\W",
            "`[A-Z][a-z]*`",
            "`\\d+`",
            "`[a-z]+`:t =t",
            "`[A-Z]+` \\N",
            "<`[a-z]+`:t>.*</=t>",
            "\\M{>1}",
            "\\M{<3}",
            "\\N \\M{>1}",
            "\\N{>1}",
            "\\N{<3}",
            "\\W:x =shape x",
            "\\W:x =case x",
            "\\V",
            "\\G",
            "\\{mac}",
            // Positional anchors now compile here, so the differential can
            // finally carry them.
            "^ \\W",
            "$ \\W",
            "\\A \\W",
            "\\z \\W",
            "^ $ \\W",
            "^ \\W \\N",
            "(^ | \"|\") \\W",
            // Lazy quantifiers, across both rank encodings: a choiceless body
            // records one iteration count, a body that branches records one
            // entry per iteration.
            "\\W*? \\N",
            "\\W+? \\N",
            "\\W?? \\N",
            "\\W{1,3}? \\N",
            ".*? \"q\"",
            "(\\N | \\W)*? \\N",
            "(\\W | \\W \\W)+? \\N",
            "\\W*? \\N \\W*?",
            "(\\W*?)* \\N",
            "\\H",
            "\\C",
            "\\%",
            "\\Z",
            "\\$",
            "\\D",
            "\\R",
            "\\L",
            "\\W \\V",
            ". !~\"END\"",
            "\\W !~\"cat\"",
            "\\N !~\"the\"",
            "#\"W(W,W)\"",
            "#\"N.N\"",
            // Alternation whose first branch matches and whose continuation
            // then fails. Nothing above discriminates leftmost-first from
            // committed choice, because every earlier alternation has
            // single-token branches with nothing after them to contradict.
            "(\"a\" | \"a\" \"b\") \"c\"",
            "(\"a\" \"b\" | \"a\") \"c\"",
            "(\\W | \\W \\W) \\W",
            "(\\N | \\N \\N) \\W",
            "(\\W \\W | \\W) =x",
            "(\\W:x | \\W \\W) =x",
            "(\\W \\W | \\W:x) =x",
            "(\\W:x \\W | \\W \\W:x) =x",
            "[\\N \\W]",
            "[^\\N]",
            "[^\\W]",
            "[\\W && \\h]",
            "[\\W -- \\u]",
            "[\\N \\W] [\\N \\W]",
            "[\"cat\" \"dog\"]",
            "[`[a-z]+` \\N]",
            "[^\\N]:x =x",
        ];
        let inputs = [
            "",
            "a",
            "12",
            "a b c",
            "the the cat dog dog",
            "<div>hi</div>",
            "<a>x</a> y <b>z</b>",
            "12 kg 30 m s",
            "foo foo bar foo",
            "a a a a a",
            "x",
            "END here END now",
            "begin END end",
            "12 34 56",
            "word",
            "1 a 2 b 3 c",
            "the the the",
            "  spaced  out  ",
            "a1 b2 c3",
            "\"q\" \"q\"",
            "Hello World foo Bar",
            "abc DEF 99 ghi",
            "deadbeef cafe deadbeef",
            "<div>hi</div> <Span>x</Span>",
            "rel v1.2.3 id 550e8400-e29b-41d4-a716-446655440000 nic 01:23:45:67:89:ab",
            "bg #ff8800 net 192.168.0.0/24 host 10.0.0.1 tag 2.0.0",
            "f(a,b) g(x,y) h(1) k(p,q,r) 3.4",
            "up 42% cache 512KB cost $1,234.56 bare 10M plain 50",
            "took 1500ms wait 3h20m open /usr/bin/x rel ./a/b drive C:\\d\\e a/b",
        ];
        for pat in patterns {
            let p = parse(pat).expect("pattern parses");
            for inp in inputs {
                let nfa = scan_nfa(&p, inp.as_bytes()).expect("nfa handles pattern");
                let set = crate::engine::scan_set_reachability(&p, inp.as_bytes());
                assert_eq!(nfa, set, "disagreement on pattern {pat:?} input {inp:?}");
            }
        }
    }

    /// The serial scan computed directly, the reference the parallel path
    /// must reproduce byte for byte.
    fn scan_serial_ref(pattern: &Pattern, input: &[u8]) -> Vec<Span> {
        let prog = compile(pattern).expect("compiles");
        let toks = crate::lexer::lex(input);
        let sig: Vec<usize> =
            (0..toks.len()).filter(|&i| toks[i].kind != TokenKind::Whitespace).collect();
        let absent = crate::prefilter::absent_guard_literals(pattern, input);
        let stream = Stitched { toks: &toks, sig: &sig };
        scan_nfa_serial(&prog, input, &stream, &absent, pattern.starts_with_resume())
    }

    /// The significant stream cut into parts at the given token counts, as
    /// the lexer's chunks would leave it, with an empty part where a count
    /// is zero.
    fn parts_of(input: &[u8], counts: &[usize]) -> Vec<Significant> {
        let toks = crate::lexer::lex(input);
        let sig: Vec<&Token> = toks.iter().filter(|t| t.kind != TokenKind::Whitespace).collect();
        let mut parts = Vec::new();
        let mut at = 0;
        for &n in counts {
            let mut part = Significant::with_base(0, n);
            for t in &sig[at..at + n] {
                part.kinds.push(t.kind.code());
                part.spans.push((t.start, t.end));
            }
            parts.push(part);
            at += n;
        }
        assert_eq!(at, sig.len(), "the counts cover the stream");
        parts
    }

    /// The bound the dispatch reads off the parts is the bound read off the
    /// stitched stream, with runs crossing the cuts between parts and an empty
    /// part among them, and the ceiling refuses the same runs whichever form
    /// holds the stream: the shared parts, a leaf's cursor, or a split's kind
    /// codes.
    #[test]
    fn the_longest_run_is_the_same_whichever_form_holds_the_stream() {
        // Word runs of 3, 4, 2 and 6 tokens; number runs of 1, 1, 3 and 1.
        let input = b"a b c 1 d e f g 2 h i 3 4 5 j k l m n o 6";
        let toks = crate::lexer::lex(input);
        let sig = Stitched::significant_of(&toks);
        assert_eq!(sig.len(), 21, "one token a letter or digit");
        let stitched = Stitched { toks: &toks, sig: &sig };
        let kinds: Vec<u32> = sig.iter().map(|&i| toks[i].kind.code()).collect();
        let spans: Vec<(u32, u32)> = sig.iter().map(|&i| (toks[i].start, toks[i].end)).collect();
        let split = KindSpans::new(&kinds, &spans);
        // Cuts inside the run of six words, inside the run of three numbers,
        // two cuts inside one run, and an empty part.
        for cuts in [
            vec![21],
            vec![17, 4],
            vec![12, 9],
            vec![15, 3, 3],
            vec![17, 0, 4],
            vec![1, 1, 1, 18],
        ] {
            let parts = parts_of(input, &cuts);
            let bases = Parts::bases_of(&parts);
            let shared = Parts::new(&parts, &bases);
            for (kind, longest) in [(TokenKind::Word, 6usize), (TokenKind::Number, 3usize)] {
                for ceiling in 0..8usize {
                    let want = (longest <= ceiling).then_some(longest);
                    assert_eq!(stitched.longest_run(kind, ceiling), want, "stitched, {kind:?} under {ceiling}");
                    assert_eq!(split.longest_run(kind, ceiling), want, "split, {kind:?} under {ceiling}");
                    assert_eq!(
                        shared.longest_run(kind, ceiling),
                        want,
                        "parts cut {cuts:?}, {kind:?} under {ceiling}"
                    );
                    assert_eq!(
                        shared.cursor().longest_run(kind, ceiling),
                        want,
                        "a cursor over parts cut {cuts:?}, {kind:?} under {ceiling}"
                    );
                }
            }
            // The first token at or after every byte, the stitched form
            // being the reference the parts and the split must reproduce,
            // through the bytes inside tokens, between them, at the input's
            // end and past it.
            for at in 0..input.len() + 3 {
                let want = stitched.first_at_or_after(at);
                assert_eq!(shared.first_at_or_after(at), want, "parts cut {cuts:?}, at byte {at}");
                assert_eq!(
                    shared.cursor().first_at_or_after(at),
                    want,
                    "a cursor over parts cut {cuts:?}, at byte {at}"
                );
                assert_eq!(split.first_at_or_after(at), want, "split, at byte {at}");
            }
        }
    }

    #[test]
    fn every_kind_survives_its_code() {
        use crate::token::BracketKind::{Brace, Paren, Square};
        let kinds = [
            TokenKind::Number, TokenKind::Word, TokenKind::Quoted, TokenKind::Ip, TokenKind::Url,
            TokenKind::Email, TokenKind::Timestamp, TokenKind::Whitespace, TokenKind::Punct,
            TokenKind::Other, TokenKind::Open(Paren), TokenKind::Open(Square), TokenKind::Open(Brace),
            TokenKind::Close(Paren), TokenKind::Close(Square), TokenKind::Close(Brace),
            TokenKind::Version, TokenKind::Uuid, TokenKind::Mac, TokenKind::HexColor, TokenKind::Cidr,
            TokenKind::Percent, TokenKind::ByteSize, TokenKind::Money, TokenKind::HashDigest,
            TokenKind::Duration, TokenKind::Path, TokenKind::Jwt, TokenKind::CreditCard,
            TokenKind::Base64, TokenKind::Geo, TokenKind::Phone, TokenKind::Quantity,
            TokenKind::Custom(0), TokenKind::Custom(7), TokenKind::Custom(255),
        ];
        for k in kinds {
            assert_eq!(TokenKind::from_code(k.code()), k, "{k:?}");
        }
    }

    #[test]
    fn the_parts_stream_reads_as_the_stitched_one() {
        // Every token by index through both forms of the parts stream, over
        // parts of uneven size with an empty one among them, and through a
        // cursor asked out of order.
        let input = b"alpha = 1; beta = 2\nif (x) { y_2 = \"q\" } 3.5 z";
        let toks = crate::lexer::lex(input);
        let sig: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].kind != TokenKind::Whitespace).collect();
        let stitched = Stitched { toks: &toks, sig: &sig };
        let m = sig.len();
        assert!(m >= 12, "the input must span several parts ({m} tokens)");
        let parts = parts_of(input, &[3, 0, 5, m - 8]);
        // The owning form reads the same stream through its own cached locate,
        // over a lex of its own rather than the parts built here, so it is held
        // to the stitched answer alongside them.
        let owned = OwnedStream::over(input);
        assert_eq!(owned.len(), sig.len(), "the owned stream spans the same tokens");
        for k in (0..sig.len()).chain((0..sig.len()).rev()) {
            assert_eq!(owned.kind(k), stitched.kind(k), "owned kind at {k}");
            assert_eq!(owned.span(k), stitched.span(k), "owned span at {k}");
        }
        let bases = Parts::bases_of(&parts);
        let shared = Parts::new(&parts, &bases);
        let cursor = shared.cursor();
        assert_eq!(shared.len(), m);
        assert_eq!(cursor.len(), m);
        for k in (0..m).chain((0..m).rev()).chain([m - 1, 0, 3, 2, 8, 7]) {
            assert_eq!(shared.kind(k), stitched.kind(k), "kind at {k}");
            assert_eq!(shared.span(k), stitched.span(k), "span at {k}");
            assert_eq!(cursor.kind(k), stitched.kind(k), "cursor kind at {k}");
            assert_eq!(cursor.span(k), stitched.span(k), "cursor span at {k}");
        }
    }

    #[test]
    fn a_prefix_walk_and_a_per_anchor_fill_select_the_same_spans() {
        // The two ways a split's host share can fill its part of the ends
        // table: an end at every anchor, and a forward walk writing only the
        // matches it takes. The selection over either, joined with a
        // per-anchor fill of the rest, must be the engine's spans, at every
        // share and on patterns that match densely, sparsely and not at all.
        let mut text = String::new();
        for i in 0..4000u32 {
            text.push_str(&format!("tag {} word {} {}\n", i % 7, i % 13, i));
        }
        let input = text.as_bytes();
        let (kinds, spans) = crate::parallel_lex::lex_significant_parallel(input);
        let stream = KindSpans::new(&kinds, &spans);
        let n = kinds.len();
        for src in ["\\W \\N", "\\N \\W", "\\W \\W", "\\N{1,2}", "\\W \\N \\W", "\\Q \\N"] {
            let p = parse(src).expect("parses");
            let prog = compile_pattern(&p).expect("compiles");
            let want = crate::engine::scan(&p, input);
            for per_mille in [0u32, 1, 250, 700, 999, 1000] {
                let mid = (n * per_mille as usize / 1000).min(n);
                let mut filled = vec![0i32; n];
                anchor_ends_into(&prog, &[], &stream, 0, &mut filled);
                let mut walked = vec![0i32; n];
                walk_prefix_into(&prog, &[], &stream, mid, &mut walked[..]);
                // The rest of each table by the per-anchor fill, as the
                // device's half of a split gives it.
                anchor_ends_into(&prog, &[], &stream, mid, &mut walked[mid..]);
                let as_spans = |ends: &[i32]| {
                    let mut out = Vec::new();
                    let mut a = 0usize;
                    while a < n {
                        let e = ends[a] as usize;
                        if e > a {
                            out.push(crate::engine::Span { start: spans[a].0, end: spans[e - 1].1 });
                            a = e;
                        } else {
                            a += 1;
                        }
                    }
                    out
                };
                assert_eq!(as_spans(&filled), want, "{src} filled");
                assert_eq!(as_spans(&walked), want, "{src} walked at {per_mille} per mille");
            }
        }
    }

    #[test]
    fn the_scan_over_parts_is_the_scan_over_the_stitched_stream() {
        // Anchors of every kind, a register bind and its reference, a guard, a
        // kind class and a bounded repeat, over a stream cut into parts at a
        // match boundary and inside a match, serially and through the
        // per-anchor dispatch.
        let mut text = String::new();
        for i in 0..2500u32 {
            text.push_str(&format!("name{i}: alice{i} = alice{i} ; ^tag {i} end\n"));
        }
        let input = text.as_bytes();
        let toks = crate::lexer::lex(input);
        let m = toks.iter().filter(|t| t.kind != TokenKind::Whitespace).count();
        assert!(m >= PARALLEL_SCAN_MIN_ANCHORS, "the dispatch must be exercised ({m} tokens)");
        let cuts = [m / 3, 0, 1, m / 2 - m / 3 - 1, m - m / 2];
        let parts = parts_of(input, &cuts);
        for src in [
            "\\W \":\" \\W", "\\W:x \"=\" =x", "^ \\W", "\\W $", "\\A \\W", "\\W \\z", "\\W ~\"end\"",
            "\\N{1,2}", "\\W | \\N", "\"alice7\" \"=\"", "\\G \\W", "\\W \\K \":\"",
        ] {
            let p = parse(src).expect("parses");
            let over = scan_nfa_over(&p, input, &toks).expect("the engine takes the pattern");
            let parts_scan = scan_nfa_parts(&p, input, &parts).expect("the engine takes the pattern");
            assert_eq!(parts_scan, over, "{src}");
        }
    }

    /// An opening of plain literals sends the dispatch to the anchors a byte
    /// search finds rather than to every anchor, and the table has to read as
    /// the walk over every anchor left it: with the literal inside a longer
    /// token, glued to punctuation, one branch of an alternation, behind an
    /// anchor, at the end of a line, and nowhere at all.
    #[test]
    fn the_anchors_a_literal_opening_finds_are_the_anchors_the_walk_admits() {
        let mut text = String::new();
        for i in 0..2500u32 {
            text.push_str(&format!(
                "let x{i} = {i} ; letter outlet let({i}) k{i}={i} var y{i} = {i} ;\nlet\n"
            ));
        }
        let input = text.as_bytes();
        let toks = crate::lexer::lex(input);
        let m = toks.iter().filter(|t| t.kind != TokenKind::Whitespace).count();
        assert!(m >= PARALLEL_SCAN_MIN_ANCHORS, "the dispatch must be exercised ({m} tokens)");
        for src in [
            "\"let\" \\W \"=\"",
            "(\"let\" | \"var\") \\W \"=\"",
            "^ \"let\" \\W",
            "\"let\" \"(\" \\N",
            "\"=\" \\N",
            "\"zzzqqq\" \\W",
            "\"letter\" \\W",
            "\"let\" $",
        ] {
            let p = parse(src).expect("parses");
            let prog = compile(&p).expect("the engine takes it");
            let opens = opening_atoms(&prog).expect("an opening to test");
            assert!(opening_literals(&opens).is_some(), "{src} opens with plain literals");
            assert_eq!(
                scan_nfa(&p, input).expect("the engine takes the pattern"),
                scan_serial_ref(&p, input),
                "{src}"
            );
        }
        // An opening that is not plain literals alone takes the walk over
        // every anchor.
        for src in ["\\W \"=\"", "(\"let\" | \\W) \"=\"", "(?orbit:case \"let\") \\W"] {
            let p = parse(src).expect("parses");
            let prog = compile(&p).expect("the engine takes it");
            let opens = opening_atoms(&prog).expect("an opening to test");
            assert!(opening_literals(&opens).is_none(), "{src} does not open with plain literals alone");
        }
    }

    #[test]
    fn the_windowed_walk_hands_out_the_scan_s_matches() {
        // A stream past the dispatch floor, so the held walk resolves windows
        // of anchors rather than stepping. Every match it hands out, by span
        // and through the capture path, must be the scan's, on patterns that
        // match densely, sparsely and not at all, and on the two shapes the
        // decomposition declines.
        let mut text = String::new();
        for i in 0..3000u32 {
            text.push_str(&format!("tag {} word_{} = {} ;\n", i % 7, i % 13, i));
        }
        let input = text.as_bytes();
        let toks = crate::lexer::lex(input);
        let m = toks.iter().filter(|t| t.kind != TokenKind::Whitespace).count();
        assert!(m >= PARALLEL_SCAN_MIN_ANCHORS, "the windows must be exercised ({m} tokens)");
        for src in [
            "\\W \"=\"", "\\N", "\\W:x \"=\" =x", "\"tag\" \\N", "\"zzzqqq\"", "\\N{1,2}",
            "\\G \\W", "\\W \\K \"=\"",
        ] {
            let p = parse(src).expect("parses");
            let want = scan_nfa(&p, input).expect("the engine takes the pattern");
            let mut w = SerialWalk::over(&p, input).expect("the engine takes the pattern");
            let mut spans = Vec::new();
            while let Some(s) = w.next_span(input) {
                spans.push(s);
            }
            assert_eq!(spans, want, "{src} by span");
            // An exhausted walk stays exhausted rather than searching the rest
            // of the stream again for each further ask.
            assert_eq!(w.next_span(input), None, "{src} past the end");
            assert_eq!(w.next_span(input), None, "{src} past the end twice");
            let mut w = SerialWalk::over(&p, input).expect("the engine takes the pattern");
            let mut matched = Vec::new();
            while let Some(found) = w.next_match(input) {
                matched.push(found);
            }
            let spans: Vec<Span> = matched
                .iter()
                .map(|f| Span { start: f.start as u32, end: f.end as u32 })
                .collect();
            assert_eq!(spans, want, "{src} by match");
            // A match taken from a window carries no slots of its own, so what
            // it binds is resolved by a second attempt at its anchor. Those
            // registers must be the ones an attempt over the whole stream
            // binds, which the spans above cannot show.
            let bound = captures_over(&p, input, &toks, &want).expect("the engine takes it");
            for (got, expect) in matched.iter().zip(&bound) {
                assert_eq!(got.captures(), expect.captures(), "{src}: registers at {}", got.start);
            }
            // The yes-or-no form takes the windows from the first ask rather
            // than the second, so it reaches the same verdict down a path none
            // of the above covers.
            assert_eq!(
                SerialWalk::any_match(&p, input),
                Some(!want.is_empty()),
                "{src} by any_match"
            );
            // A seek drops the window it resolved, so the matches from a byte
            // offset are the scan's from that offset and not the ones the
            // window before it held. `seek` places the walk by token start, so
            // a `\K` match, whose span begins after the token it is anchored
            // at, is not addressed by the offset its span reports.
            if p.mentions_reset_start() {
                continue;
            }
            let at = want.get(want.len() / 2).map_or(0, |s| s.start());
            let mut w = SerialWalk::over(&p, input).expect("the engine takes the pattern");
            assert!(w.next_span(input).is_some() || want.is_empty(), "{src} has a first match");
            w.seek(at);
            let mut sought = Vec::new();
            while let Some(s) = w.next_span(input) {
                sought.push(s);
            }
            let from_at: Vec<Span> = want.iter().copied().filter(|s| s.start() >= at).collect();
            assert_eq!(sought, from_at, "{src} after a seek to {at}");
        }
    }

    #[test]
    fn resolving_over_the_parts_binds_what_resolving_over_the_stitched_stream_binds() {
        // The parts form reads the chunks where they were lexed and hands the
        // dispatch a borrowed view of them; the stitched form joins them and
        // indexes the significant ones first. Both must bind the same
        // registers at the same spans, including for `\K`, whose match does not
        // begin at its anchor and which takes the serial arm.
        let mut text = String::new();
        for i in 0..3000u32 {
            text.push_str(&format!("name{}: alice{} = alice{} ; tag {}\n", i % 11, i % 7, i % 7, i));
        }
        let input = text.as_bytes();
        let toks = crate::lexer::lex(input);
        let m = toks.iter().filter(|t| t.kind != TokenKind::Whitespace).count();
        assert!(m >= PARALLEL_SCAN_MIN_ANCHORS, "the dispatch must be exercised ({m} tokens)");
        for src in [
            "\\W:x \"=\" =x", "\\W:name \":\"", "\\N:n", "\\W:a \\W:b", "\\W \\K \":\"",
            "\\W:v \"=\" \\W",
        ] {
            let p = parse(src).expect("parses");
            let spans = scan_nfa(&p, input).expect("the engine takes the pattern");
            let stitched = captures_over(&p, input, &toks, &spans).expect("the engine takes it");
            let parts = captures_over_parts(&p, input, &spans).expect("the engine takes it");
            assert_eq!(parts.len(), stitched.len(), "{src}: a different number of matches");
            for (a, b) in parts.iter().zip(&stitched) {
                assert_eq!((a.start, a.end), (b.start, b.end), "{src}: a different span");
                assert_eq!(a.captures(), b.captures(), "{src}: different registers at {}", a.start);
            }
        }
    }

    #[test]
    fn the_opening_atoms_refuse_what_cannot_be_tested_before_an_attempt() {
        // Two shapes cannot be rejected on the anchor's own token: one that
        // accepts without consuming, because an empty match does not depend on
        // that token at all, and one that reads a register back, because the
        // atom's answer depends on bindings no thread has made yet.
        for src in ["\\W{0,1}", "\\W:x \"=\" =x"] {
            let p = parse(src).expect("parses");
            let prog = compile(&p).expect("the engine takes it");
            assert!(opening_atoms(&prog).is_none(), "{src} must attempt every anchor");
        }
        for src in ["\"let\" \\W \"=\"", "(\\W | \\N)", "^ \"let\"", "\\W{2}"] {
            let p = parse(src).expect("parses");
            let prog = compile(&p).expect("the engine takes it");
            assert!(opening_atoms(&prog).is_some(), "{src} opens with an atom to test");
        }
    }

    #[test]
    fn an_opening_atom_accepts_every_anchor_a_match_begins_at() {
        // The soundness of skipping an anchor rests on this and nothing else:
        // the test must never reject an anchor the engine would have found a
        // match at. Checked against the scan's own answers rather than against
        // a second reading of the reasoning that produced the test.
        let mut text = String::new();
        for i in 0..3000u32 {
            text.push_str(&format!("let value_{} = {} ; tag {}\n", i % 11, i * 7, i));
        }
        let input = text.as_bytes();
        let stream = OwnedStream::over(input);
        for src in ["\"let\" \\W \"=\"", "\\W \"=\"", "\\N", "(\\W | \\N)", "^ \"let\"", "\\W{2}"] {
            let p = parse(src).expect("parses");
            let prog = compile(&p).expect("the engine takes it");
            let Some(opens) = opening_atoms(&prog) else { continue };
            let want = scan_nfa(&p, input).expect("the engine takes the pattern");
            assert!(!want.is_empty(), "{src} must match this corpus for the check to mean anything");
            for s in &want {
                let a = first_at_or_after(&stream, s.start());
                assert!(
                    opening_admits(&prog, &opens, input, &stream, a),
                    "{src}: the anchor of the match at {} was rejected",
                    s.start()
                );
            }
        }
    }

    #[test]
    fn parallel_scan_matches_serial_on_a_large_stream() {
        // A stream past the parallel-dispatch floor, so `scan_nfa` takes the
        // parallel per-anchor path. Its output must be byte-identical to the
        // serial scan and to the set-reachability oracle, on bounded
        // patterns including register equality.
        let mut input = String::new();
        for i in 0..1500u32 {
            // "tag N the the N end": the repeated word feeds =x, the rest
            // is a mix of matching and non-matching positions.
            input.push_str(&format!("tag {} the the {} end ", i % 7, i % 13));
        }
        let bytes = input.as_bytes();
        let sig = crate::lexer::lex(bytes).iter().filter(|t| t.kind != TokenKind::Whitespace).count();
        assert!(
            sig >= PARALLEL_SCAN_MIN_ANCHORS,
            "test must exercise the parallel path ({sig} tokens)"
        );

        for pat in ["\\W \\N", "\\N \\W", "\\W:x =x", "\\N{1,2}", "\\W | \\N", "\\W \\N \\W"] {
            let p = parse(pat).expect("parses");
            let par = scan_nfa(&p, bytes).expect("nfa handles pattern");
            let ser = scan_serial_ref(&p, bytes);
            assert_eq!(par, ser, "parallel != serial on {pat:?}");
        }

        // Cross-check the two most semantically loaded patterns against
        // the independent set-reachability oracle as well.
        for pat in ["\\W \\N", "\\W:x =x"] {
            let p = parse(pat).expect("parses");
            let par = scan_nfa(&p, bytes).expect("nfa handles pattern");
            let oracle = crate::engine::scan_set_reachability(&p, bytes);
            assert_eq!(par, oracle, "parallel != set-reachability on {pat:?}");
        }

        // The captures of the parallel path's spans, resolved by the bounded
        // anchored attempt: each is the repeated word the register bound.
        let p = parse("\\W:x =x").expect("parses");
        let spans = scan_nfa(&p, bytes).expect("nfa handles pattern");
        let resolved = crate::engine::captures(&p, bytes, &spans);
        assert_eq!(resolved.len(), spans.len());
        for m in &resolved {
            let text = &input[m.start..m.end];
            let word = text.split(' ').next().expect("a match holds a word");
            assert_eq!(m.group("x", bytes), Some(word.as_bytes()), "{text:?}");
        }
    }

    #[test]
    fn reset_start_on_a_large_stream_reports_what_follows_it() {
        // Past the parallel-dispatch floor, where the per-anchor table keeps
        // only each match's end: a `\K` pattern must still report the start
        // it moved to, as the serial scan does.
        let mut input = String::new();
        for i in 0..3000u32 {
            input.push_str(&format!("name{i}: alice{i} "));
        }
        let bytes = input.as_bytes();
        let sig = crate::lexer::lex(bytes).iter().filter(|t| t.kind != TokenKind::Whitespace).count();
        assert!(sig >= PARALLEL_SCAN_MIN_ANCHORS, "test must reach the parallel floor ({sig} tokens)");
        let p = parse("\\W \":\" \\K \\W").expect("parses");
        let got = scan_nfa(&p, bytes).expect("nfa handles pattern");
        assert_eq!(got, scan_serial_ref(&p, bytes));
        assert_eq!(got.len(), 3000);
        assert_eq!(&input[got[0].range()], "alice0");
        let resolved = crate::engine::captures(&p, bytes, &got);
        assert_eq!(resolved.len(), 3000);
        assert_eq!(&input[resolved[7].start..resolved[7].end], "alice7");
    }

    #[test]
    fn high_capture_pattern_routes_to_set_engine() {
        // Four named captures exceed the inline save-slot cap, so the
        // single-pass engine declines and the set-reachability engine
        // handles it. The public scan still returns the correct match and
        // every capture.
        let p = parse("\\W:a \\W:b \\W:c \\W:d").unwrap();
        assert!(scan_nfa(&p, b"one two three four").is_none());
        let spans = crate::engine::scan(&p, b"one two three four");
        let got = crate::engine::captures(&p, b"one two three four", &spans);
        assert_eq!(got.len(), 1);
        assert_eq!(&"one two three four"[got[0].start..got[0].end], "one two three four");
        let hay = b"one two three four" as &[u8];
        assert_eq!(got[0].group("a", hay), Some(b"one" as &[u8]));
        assert_eq!(got[0].group("d", hay), Some(b"four" as &[u8]));
    }

}
