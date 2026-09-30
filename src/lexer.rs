//! The typed-atom lexer: input bytes to a `Vec<Token>` with
//! bracket pairing.
//!
//! The lexer classifies each span into a [`TokenKind`] and records
//! bracket partners on the [`Token::mate`] field so the engine can
//! match a balanced group by a constant-time jump rather than a
//! match-time recursion.
//!
//! Each span is classified in one pass: whitespace runs, numbers,
//! words, quoted strings (with backslash escapes), the three bracket
//! pairs, single-character punctuation, and the richer self-delimiting
//! typed classes - IP, URL, email, timestamp, and the other
//! [`TokenKind`] variants - each recognized by a `try_*` function in
//! precedence order.

use crate::token::{BracketKind, Token, TokenKind};

/// The byte slice a token covers in the input.
#[must_use]
pub fn text<'a>(input: &'a [u8], t: &Token) -> &'a [u8] {
    &input[t.span()]
}

/// Tokenize `input` into a flat token stream with bracket pairing.
#[must_use]
pub fn lex(input: &[u8]) -> Vec<Token> {
    // The same estimate the parallel lexer gives each of its chunks. Starting
    // empty made this path grow by doubling all the way up - about twenty
    // reallocations for a megabyte of input, each copying every token written
    // so far - which is the cost the note on `lex_with_capacity` describes and
    // only the parallel path was avoiding.
    lex_with_capacity(input, input.len() / TOKEN_BYTES_ESTIMATE)
}

/// Input bytes per token, for sizing the token vector up front; the parallel
/// lexer sizes each chunk by the same divisor.
///
/// Measured across the lexer's own scaling bench: 2.28 bytes per token at
/// 16kB rising to 2.62 at 1MB, the ratio drifting only with how much of the
/// corpus is punctuation. Two reserves at most a third more than that
/// corpus fills and never grows. A divisor above the measured ratio
/// under-reserves, and the one growth that costs is not cheap: it copies
/// every token written so far, and in the lexer's profile that copy and its
/// reallocation were an eighth of the whole lex.
pub(crate) const TOKEN_BYTES_ESTIMATE: usize = 2;

/// Tokenize `input` into a token stream, pre-reserving room for `cap`
/// tokens. The parallel lexer passes a per-chunk estimate so each chunk
/// fills without the repeated reallocation an empty `Vec` would pay.
#[must_use]
pub fn lex_with_capacity(input: &[u8], cap: usize) -> Vec<Token> {
    lex_with_blobs(input, &blob_runs(input), cap)
}

/// The high-entropy blob spans of `input`, in `input`-relative offsets: the
/// spectral gate's own input, exposed so a chunked caller computes it once
/// over the whole stream instead of once per chunk.
///
/// The rolling entropy window trails the cursor, so a chunk-local call reads a
/// truncated window for its first 64 bytes and can classify them differently
/// from a whole-input call. A caller that splits the input must compute this
/// here and pass the slices down through [`lex_with_blobs`].
#[must_use]
pub fn blob_runs(input: &[u8]) -> Vec<(usize, usize)> {
    crate::spectral::high_entropy_runs(
        input,
        crate::spectral::BLOB_ENTROPY_PCT,
        crate::spectral::BLOB_MIN_LEN,
    )
}

/// [`blob_runs`] computed across cores, for the parallel lexer: the same
/// table, so the chunks it hands out lex as the serial lexer would.
#[must_use]
pub fn blob_runs_parallel(input: &[u8]) -> Vec<(usize, usize)> {
    crate::spectral::high_entropy_runs_parallel(
        input,
        crate::spectral::BLOB_ENTROPY_PCT,
        crate::spectral::BLOB_MIN_LEN,
    )
}

/// [`blob_runs`] inside one whitespace-free span `a..b` of `input`: what the
/// whole-input table holds there, from the span's own flag pass.
pub(crate) fn blob_runs_in_span(input: &[u8], a: usize, b: usize) -> Vec<(usize, usize)> {
    crate::spectral::high_entropy_runs_in_span(
        input,
        a,
        b,
        crate::spectral::BLOB_ENTROPY_PCT,
        crate::spectral::BLOB_MIN_LEN,
    )
}

/// [`blob_runs`] inside `input[lo..hi]`, a stretch of lines rather than one
/// whitespace-free span: what the whole-input table holds there, each
/// whitespace-free span inside it judged on its own bytes.
pub(crate) fn blob_runs_within(input: &[u8], lo: usize, hi: usize) -> Vec<(usize, usize)> {
    crate::spectral::high_entropy_runs_within(
        input,
        lo,
        hi,
        crate::spectral::BLOB_ENTROPY_PCT,
        crate::spectral::BLOB_MIN_LEN,
    )
}

/// Tokenize `input` using a caller-supplied blob table, in `input`-relative
/// offsets, ascending and non-overlapping.
///
/// A sustained high-entropy run becomes one opaque token rather than a shredded
/// sequence of garbage word and number tokens. Every span in `blobs` is
/// whitespace-free, so no blob crosses a line boundary.
#[must_use]
pub fn lex_with_blobs(input: &[u8], blobs: &[(usize, usize)], cap: usize) -> Vec<Token> {
    lex_with_shapes(input, blobs, &crate::custom::ShapeSet::new(), cap)
}

/// What a chunk's lex leaves for the bracket pairing across chunks: the
/// opening brackets no close in the chunk paired, in stack order, and the
/// closing brackets that found the open stack empty, each an index into the
/// chunk's tokens. A close that found a mismatched open on the stack pairs
/// with nothing serially either, so it is not here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seams {
    /// Indices of the opens left unclosed, ascending.
    pub open: Vec<usize>,
    /// Indices of the closes met over an empty open stack, ascending.
    pub close: Vec<usize>,
}

/// Where the lexer writes what it classifies: the token vector the engines
/// read, or the device path's significant kind codes and byte spans.
pub(crate) trait TokenSink {
    /// Record the token `kind` over `start..end`, returning its index in this
    /// sink.
    fn emit(&mut self, kind: TokenKind, start: usize, end: usize) -> usize;
    /// Pair the bracket tokens at `open` and `close`.
    fn mate(&mut self, open: usize, close: usize);
    /// How many tokens the sink holds.
    ///
    /// Read before and after a lex to count what that lex produced, which is
    /// what keeps the counting out of the token loop: a counter incremented
    /// per token would be a branch on the hot path paid by every caller, to
    /// serve the one that reports statistics.
    fn held(&self) -> usize;
}

impl TokenSink for Vec<Token> {
    #[inline]
    fn emit(&mut self, kind: TokenKind, start: usize, end: usize) -> usize {
        let idx = self.len();
        self.push(Token::new(kind, start, end));
        idx
    }

    #[inline]
    fn mate(&mut self, open: usize, close: usize) {
        self[open].set_mate(Some(close));
        self[close].set_mate(Some(open));
    }

    #[inline]
    fn held(&self) -> usize {
        self.len()
    }
}

/// The significant-token stream as the device path reads it: kind codes and
/// byte spans, whitespace dropped and no bracket paired, with `base` added to
/// every span so a chunk's spans are absolute in the whole input.
pub struct Significant {
    base: usize,
    /// Kind codes, one per significant token.
    pub kinds: Vec<u32>,
    /// Byte spans, one per significant token.
    pub spans: Vec<(u32, u32)>,
}

impl Significant {
    /// An empty stream whose spans are offset by `base`, with room for `cap`
    /// tokens.
    #[must_use]
    pub fn with_base(base: usize, cap: usize) -> Self {
        Significant { base, kinds: Vec::with_capacity(cap), spans: Vec::with_capacity(cap) }
    }

    /// Empty the stream, keeping its capacity, and offset its spans by `base`.
    pub fn reset(&mut self, base: usize) {
        self.base = base;
        self.kinds.clear();
        self.spans.clear();
    }

    /// Room for `cap` tokens past those held.
    pub fn reserve(&mut self, cap: usize) {
        self.kinds.reserve(cap);
        self.spans.reserve(cap);
    }
}

impl TokenSink for Significant {
    #[inline]
    fn emit(&mut self, kind: TokenKind, start: usize, end: usize) -> usize {
        if kind == TokenKind::Whitespace {
            return self.kinds.len();
        }
        let base = self.base;
        let at = |offset: usize| {
            u32::try_from(offset + base).expect("a byte offset within the stored width")
        };
        self.kinds.push(kind.code());
        self.spans.push((at(start), at(end)));
        self.kinds.len() - 1
    }

    #[inline]
    fn mate(&mut self, _open: usize, _close: usize) {}

    #[inline]
    fn held(&self) -> usize {
        self.kinds.len()
    }
}

/// The mate of a token that has none: an unclosed open, a stray close, or any
/// token that is not a bracket.
pub const NO_MATE: u32 = u32::MAX;

/// The significant-token stream with its brackets paired: [`Significant`] and
/// the mate of every token in it.
///
/// A second sink type rather than a flag on the first, because `lex_inner` is
/// generic over its sink and monomorphizes: the device path, which never reads
/// a mate, keeps the sink it had and pays neither a branch a token nor the
/// mates it would not read - four bytes a token over millions of them.
///
/// Mates are indices into this chunk's own stream. A bracket whose pair is in
/// another chunk is left unmated here and paired by the seam walk, which is the
/// first point that knows where each chunk begins.
pub struct PairedSignificant {
    /// Kinds and spans, exactly as the device path's sink holds them.
    pub parts: Significant,
    /// The mate of each token, `NO_MATE` where it has none.
    pub mates: Vec<u32>,
}

impl PairedSignificant {
    /// An empty stream whose spans are offset by `base`, with room for `cap`
    /// tokens.
    #[must_use]
    pub fn with_base(base: usize, cap: usize) -> Self {
        PairedSignificant {
            parts: Significant::with_base(base, cap),
            mates: Vec::with_capacity(cap),
        }
    }

    /// Empty the stream, keeping its capacity, and offset its spans by `base`.
    pub fn reset(&mut self, base: usize) {
        self.parts.reset(base);
        self.mates.clear();
    }

    /// Room for `cap` tokens past those held.
    pub fn reserve(&mut self, cap: usize) {
        self.parts.reserve(cap);
        self.mates.reserve(cap);
    }
}

impl TokenSink for PairedSignificant {
    #[inline]
    fn emit(&mut self, kind: TokenKind, start: usize, end: usize) -> usize {
        // Which tokens the stream keeps is the inner sink's rule, and asking it
        // whether this one landed keeps that rule in one place: whitespace is
        // dropped there, and a mate pushed for it here would put every later
        // index out by one.
        let before = self.parts.kinds.len();
        let idx = self.parts.emit(kind, start, end);
        if self.parts.kinds.len() != before {
            self.mates.push(NO_MATE);
        }
        idx
    }

    #[inline]
    fn mate(&mut self, open: usize, close: usize) {
        self.mates[open] = u32::try_from(close).expect("a token index within the stored width");
        self.mates[close] = u32::try_from(open).expect("a token index within the stored width");
    }

    #[inline]
    fn held(&self) -> usize {
        self.parts.held()
    }
}

/// [`lex_with_blobs`] for one chunk of a larger input, with what its bracket
/// pairing leaves for the pairing across chunks.
#[must_use]
pub fn lex_chunk(input: &[u8], blobs: &[(usize, usize)], cap: usize) -> (Vec<Token>, Seams) {
    let mut toks = Vec::with_capacity(cap);
    let mut seams = Seams::default();
    lex_inner(input, blobs, &crate::custom::ShapeSet::new(), &mut toks, &mut seams);
    (toks, seams)
}

/// [`lex_chunk`] appending to `toks`, whose room the caller set, with the
/// chunk's seams written to `seams`.
pub fn lex_chunk_into(
    input: &[u8],
    blobs: &[(usize, usize)],
    toks: &mut Vec<Token>,
    seams: &mut Seams,
) {
    lex_inner(input, blobs, &crate::custom::ShapeSet::new(), toks, seams);
}

/// [`lex_chunk`]'s significant stream alone, spans offset by `base`: what the
/// device path reads, written without a token vector. Brackets pair with
/// nothing here; the device never reads a mate.
#[must_use]
pub fn lex_chunk_significant(
    input: &[u8],
    blobs: &[(usize, usize)],
    base: usize,
    cap: usize,
) -> Significant {
    let mut out = Significant::with_base(base, cap);
    lex_chunk_significant_into(input, blobs, &mut out);
    out
}

/// [`lex_chunk_significant`] appending to `out`, whose base and room the
/// caller set.
pub fn lex_chunk_significant_into(input: &[u8], blobs: &[(usize, usize)], out: &mut Significant) {
    lex_inner(input, blobs, &crate::custom::ShapeSet::new(), out, &mut Seams::default());
}

/// [`lex_chunk_significant_into`] pairing the brackets that close inside the
/// chunk, with what the pairing leaves for the walk across chunks written to
/// `seams`.
///
/// The seams are the caller's here where the unpaired form discards them: a
/// stream that carries mates is the one that has a use for the brackets this
/// chunk could not pair.
pub fn lex_chunk_paired_into(
    input: &[u8],
    blobs: &[(usize, usize)],
    out: &mut PairedSignificant,
    seams: &mut Seams,
) {
    lex_inner(input, blobs, &crate::custom::ShapeSet::new(), out, seams);
}

/// Tokenize `input` with a caller-supplied blob table and a set of
/// user-declared shapes.
///
/// A [`crate::custom::Precedence::Before`] shape is tried ahead of the
/// built-in recognizers and wins an overlap; a
/// [`crate::custom::Precedence::After`] shape is tried where the built-ins and
/// the default classifiers all decline, so it fuses spans they would leave as
/// separate tokens.
#[must_use]
pub fn lex_with_shapes(
    input: &[u8],
    blobs: &[(usize, usize)],
    shapes: &crate::custom::ShapeSet,
    cap: usize,
) -> Vec<Token> {
    // Sized from the input itself where `cap` is the smaller, for the reason
    // the note on `TOKEN_BYTES_ESTIMATE` gives: a token vector that starts
    // under-reserved grows by doubling the whole way up, and every growth
    // copies each token written so far. `input` is the whole of what this call
    // lexes, so its own length is an estimate no caller has to supply.
    let mut toks = Vec::with_capacity(cap.max(input.len() / TOKEN_BYTES_ESTIMATE));
    lex_inner(input, blobs, shapes, &mut toks, &mut Seams::default());
    if !shapes.pattern_kinds().is_empty() {
        fuse_pattern_kinds(input, &mut toks, shapes);
    }
    toks
}

/// Fuse the tokens each declared kind's pattern matches into one token of
/// that kind, and pair the brackets again over what remains. The library's
/// unit kinds read from the context fuse together in one pass, their
/// patterns joined as one union whose spans the guards sort by symbol;
/// every other kind runs its own pass, in declaration order.
fn fuse_pattern_kinds(input: &[u8], toks: &mut Vec<Token>, shapes: &crate::custom::ShapeSet) {
    let (context, plain): (Vec<&crate::custom::PatternKind>, Vec<&crate::custom::PatternKind>) =
        shapes.pattern_kinds().iter().partition(|k| crate::library::is_context_kind(k.id));
    if !context.is_empty() {
        let union = crate::library::context_union(&context);
        let spans = kind_spans(&union, input, toks)
            .into_iter()
            .filter_map(|s| {
                let text = &input[s.start()..s.end()];
                context
                    .iter()
                    .find(|k| k.guard.is_some_and(|g| g.accepts(text)))
                    .map(|k| (s, k.id))
            })
            .collect::<Vec<_>>();
        fuse_spans(toks, &spans);
    }
    for kind in plain {
        let spans = kind_spans(&kind.pattern, input, toks)
            .into_iter()
            .filter(|s| kind.guard.is_none_or(|g| g.accepts(&input[s.start()..s.end()])))
            .map(|s| (s, kind.id))
            .collect::<Vec<_>>();
        fuse_spans(toks, &spans);
    }
}

/// The non-empty matches of a kind's pattern over the stream, routed as the
/// scan routes them.
fn kind_spans(pattern: &crate::ast::Pattern, input: &[u8], toks: &[Token]) -> Vec<crate::engine::Span> {
    let spans = match crate::nfa::scan_nfa_over(pattern, input, toks) {
        Some(spans) => spans,
        None => crate::engine::scan_tokens_from(pattern, input, toks, 0),
    };
    spans.into_iter().filter(|s| s.end() > s.start()).collect()
}

/// Replace the tokens each span covers with one token of the kind beside
/// it, and pair the brackets again. The spans are ascending and disjoint,
/// as a scan's matches are.
fn fuse_spans(toks: &mut Vec<Token>, spans: &[(crate::engine::Span, u8)]) {
    if spans.is_empty() {
        return;
    }
    let mut fused = Vec::with_capacity(toks.len());
    let mut next = spans.iter().peekable();
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        match next.peek() {
            Some((s, id)) if t.start() >= s.start() && t.end() <= s.end() => {
                fused.push(Token::new(TokenKind::Custom(*id), s.start(), s.end()));
                while i < toks.len() && toks[i].end() <= s.end() {
                    i += 1;
                }
                next.next();
            }
            _ => {
                fused.push(t);
                i += 1;
            }
        }
    }
    *toks = fused;
    pair_brackets(toks);
}

/// Pair each close bracket with the nearest open of its kind before it, as
/// the lex does, and leave every other bracket unpaired.
fn pair_brackets(toks: &mut [Token]) {
    let mut open: Vec<(BracketKind, usize)> = Vec::new();
    for i in 0..toks.len() {
        match toks[i].kind {
            TokenKind::Open(bk) => {
                toks[i].set_mate(None);
                open.push((bk, i));
            }
            TokenKind::Close(bk) => {
                toks[i].set_mate(None);
                if let Some(&(open_bk, open_at)) = open.last()
                    && open_bk == bk
                {
                    open.pop();
                    toks[open_at].set_mate(Some(i));
                    toks[i].set_mate(Some(open_at));
                }
            }
            _ => {}
        }
    }
}

/// Tokenize `input` into `out`, cleared first, so a caller scanning many
/// inputs keeps one token buffer instead of taking a fresh one each time.
///
/// A token vector is many times the input's size, and a fresh one is a
/// fresh block from the allocator whose pages fault in as the lexer writes
/// them; a buffer held across calls has its pages already, and lexes a
/// four-megabyte input in three quarters of the time. Hold one across
/// serial work only: after a dispatched match has read the tokens on every
/// core, writing the held buffer again means invalidating its lines in every
/// core's cache, and `benches/vs_regex` measures that at more than the fresh
/// pages cost. The output is the same as [`lex`]'s.
pub fn lex_into(input: &[u8], out: &mut Vec<Token>) {
    out.clear();
    out.reserve(input.len() / TOKEN_BYTES_ESTIMATE);
    lex_inner(input, &blob_runs(input), &crate::custom::ShapeSet::new(), out, &mut Seams::default());
}

/// Tokenize a whitespace-free run that holds no blob into `out`, cleared
/// first: the tokens [`lex`] makes of the same bytes, with no entropy pass,
/// since the caller has read the run for blobs through [`blob_runs_in_span`]
/// and found none.
pub(crate) fn lex_run_into(input: &[u8], out: &mut Vec<Token>) {
    out.clear();
    lex_inner(input, &[], &crate::custom::ShapeSet::new(), out, &mut Seams::default());
}

fn lex_inner<S: TokenSink>(
    input: &[u8],
    blobs: &[(usize, usize)],
    shapes: &crate::custom::ShapeSet,
    toks: &mut S,
    seams: &mut Seams,
) {
    // Every lex in the crate reaches here, whichever entry point was called
    // and whether it runs over a whole input or one chunk of one, so this is
    // the one place that can count them all. The flag is read once, the clock
    // only where it is set, and the tokens are counted from the sink's own
    // length rather than in the loop below.
    if crate::trace::keeping() {
        let held = toks.held();
        let began = std::time::Instant::now();
        lex_counted(input, blobs, shapes, toks, seams);
        crate::trace::lexed(toks.held().saturating_sub(held), began.elapsed());
        return;
    }
    lex_counted(input, blobs, shapes, toks, seams);
}

/// [`lex_inner`] with the counting taken off it, so the counted and uncounted
/// paths run exactly the same code.
fn lex_counted<S: TokenSink>(
    input: &[u8],
    blobs: &[(usize, usize)],
    shapes: &crate::custom::ShapeSet,
    toks: &mut S,
    seams: &mut Seams,
) {
    let n = input.len();
    // Stack of indices (into `toks`) of opening brackets not yet closed;
    // what it holds at the end is the input's unclosed opens. The kind of
    // each is kept beside it, so a close is matched without reading the sink.
    let Seams { open: open_stack, close: bare_closes } = seams;
    let mut open_kinds: Vec<BracketKind> = Vec::new();
    let mut i = 0;

    let mut bi = 0usize;

    while i < n {
        // Collapse a high-entropy blob covering the cursor into one token.
        while bi < blobs.len() && blobs[bi].1 <= i {
            bi += 1;
        }
        if bi < blobs.len() && i >= blobs[bi].0 {
            let end = blobs[bi].1;
            // A high-entropy blob that is exactly base64-shaped is a Base64
            // token; every other blob stays Other.
            let kind = if try_base64(input, i) == Some(end) {
                TokenKind::Base64
            } else {
                TokenKind::Other
            };
            toks.emit(kind, i, end);
            i = end;
            bi += 1;
            continue;
        }
        // A `Before` shape outranks every built-in, so it is tried first.
        if !shapes.is_empty()
            && let Some((id, end)) =
                shapes.longest_at(input, i, crate::custom::Precedence::Before)
        {
            toks.emit(TokenKind::Custom(id), i, end);
            i = end;
            continue;
        }
        let b = input[i];
        if b.is_ascii_whitespace() {
            let start = i;
            // A lone whitespace byte, the usual case between two words, is
            // its own run; the SIMD run search is for the longer ones.
            i += if input.get(i + 1).is_some_and(u8::is_ascii_whitespace) {
                crate::byte_simd::space_run(&input[i..])
            } else {
                1
            };
            toks.emit(TokenKind::Whitespace, start, i);
        } else if b == b'"'
            && let Some(end) = double_quoted_end(input, i)
        {
            toks.emit(TokenKind::Quoted, i, end);
            i = end;
        } else if b == b'\'' && char_literal_end(input, i).is_some() {
            // A bounded single-quoted char literal (`'a'`, `'\n'`, `'"'`). The
            // bound is what makes this safe across languages: a Rust lifetime
            // (`'static`) and a multi-char single-quoted string (`'hello'`) have
            // no closing quote within the char-literal window, so they fall
            // through to the punctuation path unchanged. Scanning it as one
            // Quoted token stops an inner `"` (as in `'"'`) from opening a
            // string that swallows the rest of the file.
            let start = i;
            i = char_literal_end(input, i).unwrap_or(i + 1);
            toks.emit(TokenKind::Quoted, start, i);
        } else if b == b'\''
            && let Some(end) = single_quoted_end(input, i)
        {
            toks.emit(TokenKind::Quoted, i, end);
            i = end;
        } else {
            // The typed recognizers and the word branch both begin by walking
            // the word run at `i`; it is walked once here and both read it.
            let (typed, word_end) = if typed_token_possible(b) {
                try_typed_token(input, i)
            } else {
                (None, i)
            };
            if let Some((kind, end)) = typed {
                toks.emit(kind, i, end);
                i = end;
            } else if !shapes.is_empty()
                && let Some((id, end)) =
                    shapes.longest_at(input, i, crate::custom::Precedence::After)
            {
                // No built-in recognized this span, so an `After` shape may
                // fuse what the default classifiers would split.
                toks.emit(TokenKind::Custom(id), i, end);
                i = end;
            } else if b.is_ascii_digit() {
                let start = i;
                i = scan_number(input, i);
                toks.emit(TokenKind::Number, start, i);
            } else if b == b'_' || b.is_ascii_alphabetic() || utf8_letter_at(input, i).is_some() {
                let start = i;
                // SIMD-classify the identifier run: 32 bytes per step,
                // stopping at the first non-word byte. The SIMD path is ASCII
                // (a byte >= 0x80 stops it); the loop then decodes the UTF-8
                // char there and continues while it is alphabetic, so a
                // multi-byte letter joins the word instead of shattering into
                // per-byte tokens. An ASCII word start already has its run.
                i = if word_end > i { word_end } else { i + crate::byte_simd::word_run(&input[i..]) };
                while let Some(len) = utf8_letter_at(input, i) {
                    i += len;
                    i += crate::byte_simd::word_run(&input[i..]);
                }
                toks.emit(TokenKind::Word, start, i);
            } else if let Some(bk) = open_bracket(b) {
                let idx = toks.emit(TokenKind::Open(bk), i, i + 1);
                open_stack.push(idx);
                open_kinds.push(bk);
                i += 1;
            } else if let Some(bk) = close_bracket(b) {
                let idx = toks.emit(TokenKind::Close(bk), i, i + 1);
                // Pair with the nearest open only when the kind matches; a
                // mismatched close is left unpaired (mate stays None). A
                // close over an empty stack is kept for the pairing across
                // chunks, where an earlier chunk's open may be its mate.
                match open_kinds.last() {
                    Some(open_bk) if *open_bk == bk => {
                        open_kinds.pop();
                        let open_idx = open_stack.pop().expect("one index per open kind");
                        toks.mate(open_idx, idx);
                    }
                    Some(_) => {}
                    None => bare_closes.push(idx),
                }
                i += 1;
            } else if b == 0xE2
                && input.get(i + 1..i + 3) == Some(&[0x88, 0x92])
                && let Some(end) = crate::quantity::recognize(input, i)
            {
                // The minus sign U+2212 opens a signed quantity as `-` does.
                toks.emit(TokenKind::Quantity, i, end);
                i = end;
            } else {
                // A non-ASCII, non-letter char is one whole token, never
                // split into its UTF-8 bytes: unicode whitespace lexes as
                // Whitespace, everything else (em-dash, curly quote, emoji)
                // as Punct.
                let (c, len) = utf8_char_at(input, i);
                let kind = if len > 1 && c.is_whitespace() {
                    TokenKind::Whitespace
                } else {
                    TokenKind::Punct
                };
                toks.emit(kind, i, i + len);
                i += len;
            }
        }
    }
}

/// Decode the UTF-8 char starting at byte `i`, returning `(char, byte_len)`.
/// An invalid or truncated sequence decodes as one replacement char of length
/// 1, so the lexer always advances and never splits a valid sequence. The one
/// decoder the crate reads bytes as characters with.
pub(crate) fn utf8_char_at(input: &[u8], i: usize) -> (char, usize) {
    let b0 = input[i];
    // An ASCII byte is its own char; every punctuation token comes through
    // here, so it is not decoded as a one-byte string.
    if b0 < 0x80 {
        return (char::from(b0), 1);
    }
    let len = match b0 {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return ('\u{FFFD}', 1),
    };
    if i + len <= input.len()
        && let Ok(s) = std::str::from_utf8(&input[i..i + len])
        && let Some(c) = s.chars().next()
    {
        return (c, len);
    }
    ('\u{FFFD}', 1)
}

/// When a multi-byte alphabetic char starts at byte `i`, its byte length;
/// `None` for ASCII, non-letters, and invalid sequences. The word path uses
/// this to extend an identifier across unicode letters, and the quantity
/// recognizer to end a unit symbol before one.
pub(crate) fn utf8_letter_at(input: &[u8], i: usize) -> Option<usize> {
    if i >= input.len() || input[i] < 0x80 {
        return None;
    }
    let (c, len) = utf8_char_at(input, i);
    (len > 1 && c.is_alphabetic()).then_some(len)
}

/// If a bounded single-quoted char literal starts at byte `i` (`'a'`, `'\n'`,
/// `'"'`, `'\''`), return the byte just past its closing quote. The window is
/// deliberately tight - one char, or a backslash escape and one char - so a
/// Rust lifetime (`'static`) and a multi-char single-quoted string (`'hello'`),
/// which have no closing quote in that window, return `None` and are lexed by
/// the ordinary paths. This is what stops an inner quote (`'"'`) desyncing the
/// scan without disturbing lifetimes or single-quoted strings.
pub(crate) fn char_literal_end(input: &[u8], i: usize) -> Option<usize> {
    if input.get(i) != Some(&b'\'') {
        return None;
    }
    // '\X' - opening quote, backslash, one char, closing quote.
    if input.get(i + 1) == Some(&b'\\') {
        return (input.get(i + 3) == Some(&b'\'')).then_some(i + 4);
    }
    // 'X' - opening quote, one char that is neither a quote nor a backslash,
    // closing quote.
    if input.get(i + 1).is_some_and(|&c| c != b'\'' && c != b'\\')
        && input.get(i + 2) == Some(&b'\'')
    {
        return Some(i + 3);
    }
    None
}

/// An identifier byte.
const fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// A byte that continues a word across a quote: an identifier byte, or any
/// byte of a letter beyond ASCII.
const fn joins_word(c: u8) -> bool {
    is_word_byte(c) || c >= 0x80
}

/// How many bytes a backslash at `j` in a quoted string takes with what it
/// escapes: the byte after it, or a CRLF, which ends a line as an LF does.
fn escape_width(input: &[u8], j: usize) -> usize {
    if input.get(j + 1) == Some(&b'\r') && input.get(j + 2) == Some(&b'\n') { 3 } else { 2 }
}

/// Whether a backslash stands directly before the line ending at the newline
/// `nl`, before the LF itself or before the CR of a CRLF: the only line ending
/// a quoted string can continue past.
pub(crate) fn backslash_before_newline(input: &[u8], nl: usize) -> bool {
    let end = if nl > 0 && input[nl - 1] == b'\r' { nl - 1 } else { nl };
    end > 0 && input[end - 1] == b'\\'
}

/// If a double-quoted string starts at byte `i`, return the byte just past its
/// closing quote: the first unescaped quote after it on the same line. Inside
/// the string a backslash escapes the byte after it, so an escaped quote does
/// not close it and an escaped line ending, LF or CRLF, continues it onto the
/// next line, as a C string continues. A newline reached first, or the input's
/// end, means no string opened, and the quote is a plain byte.
///
/// Held to its line so an odd quote cannot invert the reading of every line
/// after it: a stray quote in a comment, a regular expression, a raw string of
/// another syntax or a sentence costs the rest of its own line and no more. A
/// multi-line string, a Python docstring among them, lexes as the tokens it
/// holds, as a single-quoted string that runs past its line does.
pub(crate) fn double_quoted_end(input: &[u8], i: usize) -> Option<usize> {
    if input.get(i) != Some(&b'"') {
        return None;
    }
    let mut j = i + 1;
    while j < input.len() {
        match input[j] {
            b'\\' => j += escape_width(input, j),
            b'"' => return Some(j + 1),
            b'\n' => return None,
            _ => j += 1,
        }
    }
    None
}

/// [`double_quoted_end`] read off `closes`, the double quotes and newlines of
/// `input`, rather than walked to: the first of them after the opening quote
/// that no backslash escapes decides, a quote closing the string and a newline
/// leaving none, as the input's end does. A byte is escaped where an odd run of
/// backslashes stands directly before it, or before the CR of the CRLF whose LF
/// it is, counted back no further than the opening quote, which is the walk's
/// pairing read from the other end.
///
/// The quote scans the parallel lexer and the byte routes run visit only a
/// file's quote bytes, and a walk would cost them every byte of every string -
/// on JSON, half the file. Read off the positions, a string costs the quotes
/// and newlines inside it, its bytes classified a block at a time. Two
/// searches a string instead, one for the close and one for a newline before
/// it, measured 1.6 to 1.9 times this on 4 and 16 MB of configuration and data
/// files, whose strings are short enough that a search's setup costs more than
/// its speed saves. The lexer, which reads every byte of a string anyway,
/// walks. A test holds the two to one answer at every double quote of real
/// source.
pub(crate) fn double_quoted_end_at(
    input: &[u8],
    i: usize,
    closes: &mut crate::byte_simd::CloseOrNewlinePositions<'_>,
) -> Option<usize> {
    if input.get(i) != Some(&b'"') {
        return None;
    }
    let mut from = i + 1;
    loop {
        let q = closes.next_at_or_after(from)?;
        let at = if input[q] == b'\n' && q > i + 1 && input[q - 1] == b'\r' { q - 1 } else { q };
        let mut run = 0;
        while at - run > i + 1 && input[at - run - 1] == b'\\' {
            run += 1;
        }
        if run % 2 == 0 {
            return (input[q] == b'"').then_some(q + 1);
        }
        from = q + 1;
    }
}

/// If a single-quoted string starts at byte `i`, return the byte just past
/// its closing quote. The quote opens one only where no word byte precedes
/// it, so the apostrophe of `don't`, the elision of `Γι' αυτό` and the foot
/// mark of `5'10` open nothing; nor does a quote followed by one lowercase
/// letter and whitespace, the article and elisions Afrikaans and Dutch write
/// as `'n`, `'t`, `'s` and `'k`, while `'I was there,'` opens; nor does a
/// quote beside another, the `''` of wiki markup and of an empty string. The
/// first unescaped quote after it on the same line decides: it closes the
/// string where no word byte follows it, and where one does nothing opened,
/// since no language that writes such strings holds an unescaped quote
/// inside one - so the second quote of `&'a str) -> &'a str` and of `'90s
/// and '80s` leaves those without a string, while `'n 'massiewe aanval'`
/// holds the one string it reads as. A backslash escapes the byte after it, or
/// the CRLF after it. An opener with no quote after it on its line -
/// `'static` - is left to the ordinary paths.
pub(crate) fn single_quoted_end(input: &[u8], i: usize) -> Option<usize> {
    if input.get(i) != Some(&b'\'') || (i > 0 && joins_word(input[i - 1])) {
        return None;
    }
    if (i > 0 && input[i - 1] == b'\'') || input.get(i + 1) == Some(&b'\'') {
        return None;
    }
    if input.get(i + 1).is_some_and(u8::is_ascii_lowercase)
        && input.get(i + 2).is_none_or(|&c| c.is_ascii_whitespace())
    {
        return None;
    }
    let mut j = i + 1;
    while j < input.len() {
        match input[j] {
            b'\\' => j += escape_width(input, j),
            b'\n' => return None,
            b'\'' => return (!input.get(j + 1).is_some_and(|&c| joins_word(c))).then_some(j + 1),
            _ => j += 1,
        }
    }
    None
}

/// Consume a numeric run: digits, with an optional single decimal
/// point that must sit between digits.
fn scan_number(input: &[u8], mut i: usize) -> usize {
    let n = input.len();
    i += crate::byte_simd::digit_run(&input[i..]);
    if i + 1 < n && input[i] == b'.' && input[i + 1].is_ascii_digit() {
        i += 1;
        i += crate::byte_simd::digit_run(&input[i..]);
    }
    i
}

fn open_bracket(b: u8) -> Option<BracketKind> {
    match b {
        b'(' => Some(BracketKind::Paren),
        b'[' => Some(BracketKind::Square),
        b'{' => Some(BracketKind::Brace),
        _ => None,
    }
}

fn close_bracket(b: u8) -> Option<BracketKind> {
    match b {
        b')' => Some(BracketKind::Paren),
        b']' => Some(BracketKind::Square),
        b'}' => Some(BracketKind::Brace),
        _ => None,
    }
}

/// The kind of the one-byte token a punctuation byte lexes as on its own: an
/// opening or closing bracket, else `Punct`.
pub(crate) fn punct_kind(b: u8) -> TokenKind {
    if let Some(bk) = open_bracket(b) {
        TokenKind::Open(bk)
    } else if let Some(bk) = close_bracket(b) {
        TokenKind::Close(bk)
    } else {
        TokenKind::Punct
    }
}

/// Try to recognize a richer typed token (URL, email, IP, timestamp)
/// at `i`, returning its kind and end offset. The recognizers are
/// ordered most-distinctive first; each self-gates on its start byte
/// and returns `None` cheaply when the span is not its shape, so an
/// ordinary number or word falls through to the default classifier.
/// Whether any typed recognizer can begin with this byte.
///
/// The recognizers each self-gate, but reaching that gate costs a call, and
/// the chain is run at every token start - so an ordinary `=` or `;` paid for
/// eighteen rejections before falling through to its own one-line path. This
/// is the same gate hoisted to a table read, which the common punctuation
/// token now answers with instead.
///
/// The set is deliberately generous. Everything alphanumeric is in it, since
/// an email or a path or a hex-lettered digest may start there, and so are the
/// punctuation marks that anchor a recognizer: `+` a phone, `$` money, `#` a
/// hex color, `/` a path, `.` a relative path, `-` a signed coordinate, `:`
/// an IPv6 run, `_` an identifier-ish local part. A byte outside it starts no
/// typed token, so skipping the chain there changes nothing.
const fn typed_token_possible(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(b, b'+' | b'$' | b'#' | b'/' | b'.' | b'-' | b':' | b'_')
}

/// The shortest word run a typed token can lie within: the fourteen-byte
/// body of the shortest base64 blob, whose two bytes of `=` padding make the
/// sixteen it needs. A shorter run that opens with a letter or underscore and
/// that no byte after it joins to a typed token is a Word token whatever its
/// bytes; the byte route in `prefilter` answers such runs on that.
pub(crate) const TYPED_WORD_RUN: usize = 14;

/// What every recognizer begins by walking, walked once per token start.
///
/// Each recognizer scans the run of word bytes at the start and then asks
/// what follows it: a `:` after a scheme, an `@` after a mail local part,
/// a `.` after an octet. Over a plain identifier that is the same run walked
/// by every recognizer whose start byte allows it, and every one of them
/// then refuses. The run's end and the digit run's end, read once with the
/// SIMD classifier, let a recognizer be skipped when the byte after the run
/// is one it could never accept.
struct RunFacts {
    /// End of the `[A-Za-z0-9_]` run starting at the token start.
    word_end: usize,
    /// End of the ASCII digit run starting at the token start.
    digit_end: usize,
}

impl RunFacts {
    fn at(input: &[u8], i: usize) -> Self {
        let word_end = i + crate::byte_simd::word_run(&input[i..]);
        let mut digit_end = i;
        while digit_end < word_end && input[digit_end].is_ascii_digit() {
            digit_end += 1;
        }
        Self { word_end, digit_end }
    }
}

/// Recognize a typed token at `i`, also returning the end of the word run
/// there so the word branch that follows a refusal does not walk it again.
///
/// Most token starts are a plain word or a plain number, where every gate
/// of the chain is false: a word run under fourteen bytes that no joining
/// byte follows and that does not open a JWT header, or a bare digit run
/// under thirteen digits that no joining byte follows and that is not a
/// four-digit group before a space. Such a start is refused here, inside the
/// lexer loop this is inlined into, without entering the chain's frame.
#[inline(always)]
fn try_typed_token(input: &[u8], i: usize) -> (Option<(TokenKind, usize)>, usize) {
    let facts = RunFacts::at(input, i);
    let b = input[i];
    let run = facts.word_end - i;
    let after = input.get(facts.word_end).copied();
    let joined = matches!(after, Some(b':' | b'+' | b'.' | b'-' | b'@' | b'%' | b'/'));
    let plain_word = b.is_ascii_alphabetic()
        && !joined
        && run < TYPED_WORD_RUN
        && !(b == b'e' && input[i..].starts_with(b"eyJ"))
        && !(run == 3 && after == Some(b' ') && syslog_head(input, i));
    // A digit run a space then a unit symbol could follow, or a byte beyond
    // ASCII (a degree sign, a micro sign, a no-break space), may open a
    // quantity, so those enter the chain.
    let plain_number = b.is_ascii_digit()
        && !joined
        && facts.digit_end == facts.word_end
        && run < 13
        && !(run == 4 && after == Some(b' '))
        && !after.is_some_and(|c| c >= 0x80)
        && !(after == Some(b' ')
            && crate::quantity::opens_a_symbol(input.get(facts.word_end + 1).copied()));
    if plain_word || plain_number {
        return (None, facts.word_end);
    }
    (try_typed_token_chain(input, i, Some(&facts)), facts.word_end)
}

/// The recognizer chain. With `facts`, a recognizer is called only where
/// the bytes after the runs leave it a way to succeed; without, every
/// recognizer its start byte allows is called. The two must agree
/// everywhere, which is what the gate test checks. Kept out of line: its
/// frame holds every recognizer's locals, and the plain starts
/// [`try_typed_token`] refuses never enter it.
#[inline(never)]
fn try_typed_token_chain(
    input: &[u8],
    i: usize,
    facts: Option<&RunFacts>,
) -> Option<(TokenKind, usize)> {
    // Each recognizer already refuses a start byte it cannot begin with, but
    // reaching that refusal costs a call, and the chain runs at every token
    // start. These are the same refusals hoisted to three flags computed once,
    // so a call is made only where it could succeed. The order is untouched,
    // because it is precedence - a MAC must be tried before an IPv6 address,
    // a CIDR block before a bare IPv4 - and a guard only ever skips a call
    // that would have returned nothing.
    //
    // The run facts sharpen each guard from the start byte to the byte after
    // the run. Every gate is a condition the recognizer's own success
    // implies, so a recognizer that would have matched is always called;
    // over a plain identifier none of them is.
    let b = input[i];
    let digit = b.is_ascii_digit();
    let alpha = b.is_ascii_alphabetic();
    let hex = b.is_ascii_hexdigit();
    let gate = |ok: &dyn Fn(&RunFacts) -> bool| facts.is_none_or(ok);
    let after_word = |f: &RunFacts| input.get(f.word_end).copied();
    let after_digits = |f: &RunFacts| input.get(f.digit_end).copied();

    // A scheme is letters, digits and `+.-`, then `://`, so the word run ends
    // at the colon or at one of those three.
    if alpha
        && gate(&|f| matches!(after_word(f), Some(b':' | b'+' | b'.' | b'-')))
        && let Some(end) = try_url(input, i)
    {
        return Some((TokenKind::Url, end));
    }
    // A JWT is anchored by its `eyJ` header prefix, so it never steals a plain
    // dotted identifier.
    if b == b'e' && let Some(end) = try_jwt(input, i) {
        return Some((TokenKind::Jwt, end));
    }
    // Anchored by a leading `+`, so it never steals a bare number.
    if b == b'+' && let Some(end) = try_phone(input, i) {
        return Some((TokenKind::Phone, end));
    }
    // The same number written nationally: an area code, or a `1`, then a
    // hyphen.
    if digit
        && gate(&|f| {
            (f.word_end - i == 3 || (f.word_end - i == 1 && b == b'1'))
                && after_word(f) == Some(b'-')
        })
        && let Some(end) = try_nanp(input, i)
    {
        return Some((TokenKind::Phone, end));
    }
    // UUID and MAC before IPv6, CIDR before IPv4: the more specific shape
    // wins, so a hex-lettered MAC is not swallowed as an IPv6 address and a
    // `/prefix` block stays one token. A UUID's first group is eight hex
    // digits then a dash; a MAC's is two then a separator.
    if hex
        && gate(&|f| f.word_end - i == 8 && after_word(f) == Some(b'-'))
        && let Some(end) = try_uuid(input, i)
    {
        return Some((TokenKind::Uuid, end));
    }
    if hex
        && gate(&|f| f.word_end - i == 2 && matches!(after_word(f), Some(b':' | b'-')))
        && let Some(end) = try_mac(input, i)
    {
        return Some((TokenKind::Mac, end));
    }
    // A hash digest is a dashless fixed-length hex run, so it never collides
    // with the dash-bearing UUID/MAC above; the run is the whole word.
    if hex && gate(&|f| matches!(f.word_end - i, 32 | 40 | 64)) && let Some(end) = try_hash(input, i) {
        return Some((TokenKind::HashDigest, end));
    }
    // An address opens with an octet of up to three digits then a dot.
    let octet_then_dot = |f: &RunFacts| f.word_end - i <= 3 && after_word(f) == Some(b'.');
    if digit && gate(&octet_then_dot) && let Some(end) = try_cidr(input, i) {
        return Some((TokenKind::Cidr, end));
    }
    // A local part runs over the word bytes and `.%+-` to the `@`.
    if (alpha || digit)
        && gate(&|f| matches!(after_word(f), Some(b'@' | b'.' | b'%' | b'+' | b'-')))
        && let Some(end) = try_email(input, i)
    {
        return Some((TokenKind::Email, end));
    }
    // Hex groups and colons: from a hex start the word run ends at a colon.
    if (hex || b == b':')
        && gate(&|f| b == b':' || after_word(f) == Some(b':'))
        && let Some(end) = try_ipv6(input, i)
    {
        return Some((TokenKind::Ip, end));
    }
    if digit && gate(&octet_then_dot) && let Some(end) = try_ipv4(input, i) {
        return Some((TokenKind::Ip, end));
    }
    // 13-19 digits, contiguous or in an issuer's grouping, that pass Luhn: a
    // card, not a number. Contiguous, the run is at least thirteen long;
    // grouped, the first group is four digits and a space or a hyphen
    // follows it.
    if digit
        && gate(&|f| {
            f.word_end - i >= 13
                || (f.word_end - i == 4 && matches!(after_word(f), Some(b' ' | b'-')))
        })
        && let Some(end) = try_creditcard(input, i)
    {
        return Some((TokenKind::CreditCard, end));
    }
    // A lat,long pair with in-range decimals; the comma and ranges keep an
    // integer pair from being a coordinate. The latitude needs a fractional
    // part, so from a digit the digits end at a dot.
    if (digit || b == b'-')
        && gate(&|f| b == b'-' || after_digits(f) == Some(b'.'))
        && let Some(end) = try_geo(input, i)
    {
        return Some((TokenKind::Geo, end));
    }
    if b == b'$' && let Some(end) = try_money(input, i) {
        return Some((TokenKind::Money, end));
    }
    // Semver reads exactly three dotted numeric parts, so it never steals a
    // four-part IPv4 address (which the IPv4 recognizer above already took).
    // The first part, `v`-prefixed or not, is the word run and ends at a dot.
    if (digit || b == b'v' || b == b'V')
        && gate(&|f| after_word(f) == Some(b'.'))
        && let Some(end) = try_semver(input, i)
    {
        return Some((TokenKind::Version, end));
    }
    // Byte-size (ends in a B unit) and percent (ends in %) before the plain
    // number branch fuses them, and after semver/IP so a dotted address wins.
    // A unit is letters, so the digits end inside the word run or at the
    // decimal point; a percent sign or the decimal point ends the digits.
    let digits_then_unit = |f: &RunFacts| f.digit_end < f.word_end || after_digits(f) == Some(b'.');
    if digit && gate(&digits_then_unit) && let Some(end) = try_bytesize(input, i) {
        return Some((TokenKind::ByteSize, end));
    }
    if digit
        && gate(&|f| matches!(after_digits(f), Some(b'.' | b'%')))
        && let Some(end) = try_percent(input, i)
    {
        return Some((TokenKind::Percent, end));
    }
    if digit && gate(&digits_then_unit) && let Some(end) = try_duration(input, i) {
        return Some((TokenKind::Duration, end));
    }
    // A quantity: digits, or a sign then digits, then a unit symbol attached
    // or one separator away. After the digits stands a letter, a byte of a
    // non-ASCII symbol, `%`, a decimal point or a space; from a sign the
    // digits are past the start and the run facts say nothing.
    if (digit || b == b'-' || b == b'+')
        && gate(&|f| {
            !digit
                || matches!(after_digits(f), Some(c) if c.is_ascii_alphabetic() || c >= 0x80 || matches!(c, b'%' | b'.' | b' '))
        })
        && let Some(end) = crate::quantity::recognize(input, i)
    {
        return Some((TokenKind::Quantity, end));
    }
    if b == b'#' && let Some(end) = try_hexcolor(input, i) {
        return Some((TokenKind::HexColor, end));
    }
    // From a letter the only path form is a drive letter then a colon.
    if (alpha || b == b'/' || b == b'.' || b == b'~')
        && gate(&|f| !alpha || (f.word_end - i == 1 && after_word(f) == Some(b':')))
        && let Some(end) = try_path(input, i)
    {
        return Some((TokenKind::Path, end));
    }
    // Base64 is greedy over alnum, so it runs late - the specific recognizers
    // above win first, and it catches the leftover base64-shaped runs that the
    // entropy blob gate did not already collapse. Its body is at least
    // fourteen bytes when it lies within the word run, and continues past
    // the run only through `+` or `/`.
    if gate(&|f| f.word_end - i >= TYPED_WORD_RUN || matches!(after_word(f), Some(b'+' | b'/')))
        && let Some(end) = try_base64(input, i)
    {
        return Some((TokenKind::Base64, end));
    }
    // A date's year ends at a dash and a time's hour at a colon; an Apache
    // day ends at a slash; a syslog month is three letters then a space.
    if gate(&|f| {
        (digit && matches!(after_word(f), Some(b'-' | b':' | b'/')))
            || (alpha && f.word_end - i == 3 && after_word(f) == Some(b' '))
    }) && let Some(end) = try_timestamp(input, i)
    {
        return Some((TokenKind::Timestamp, end));
    }
    None
}

/// A JSON Web Token: three `.`-separated base64url segments. Anchored by the
/// `eyJ` header prefix (the base64url of a JSON object's `{"`), which every JWT
/// carries and which makes a false positive essentially impossible.
fn try_jwt(input: &[u8], i: usize) -> Option<usize> {
    if !input[i..].starts_with(b"eyJ") {
        return None;
    }
    let is_b64url = |b: u8| b.is_ascii_alphanumeric() || b == b'-' || b == b'_';
    let seg = |mut j: usize| -> Option<usize> {
        let start = j;
        while j < input.len() && is_b64url(input[j]) {
            j += 1;
        }
        (j > start).then_some(j)
    };
    let a = seg(i)?;
    if input.get(a) != Some(&b'.') {
        return None;
    }
    let b = seg(a + 1)?;
    if input.get(b) != Some(&b'.') {
        return None;
    }
    seg(b + 1)
}

/// The group lengths a grouped card number is printed in, longest first so
/// a nineteen-digit card is not cut at sixteen: `4-4-4-4-3` (nineteen
/// digits), `4-4-4-4` (sixteen), `4-6-5` (fifteen) and `4-6-4` (fourteen).
const CARD_LAYOUTS: [&[usize]; 4] = [&[4, 4, 4, 4, 3], &[4, 4, 4, 4], &[4, 6, 5], &[4, 6, 4]];

/// The most digits a fixed layout reads: a nineteen-digit card.
const DIGITS_MAX: usize = 19;

/// Digit values in a fixed buffer of [`DIGITS_MAX`].
struct Digits {
    buf: [u8; DIGITS_MAX],
    len: usize,
}

impl Digits {
    fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// The digits of exactly the group lengths in `widths`, joined by `sep`,
/// read from `i`, with the end offset. `None` when the bytes there are not
/// that shape: a group of a different length, or a missing separator, ends
/// the read. What follows the last group is the caller's to check. The
/// widths sum to at most [`DIGITS_MAX`].
fn fixed_groups(input: &[u8], i: usize, widths: &[usize], sep: u8) -> Option<(Digits, usize)> {
    assert!(
        widths.iter().sum::<usize>() <= DIGITS_MAX,
        "a fixed layout holds at most {} digits",
        DIGITS_MAX
    );
    let mut digits = Digits { buf: [0; DIGITS_MAX], len: 0 };
    let mut j = i;
    for (k, &len) in widths.iter().enumerate() {
        if k > 0 {
            if input.get(j) != Some(&sep) {
                return None;
            }
            j += 1;
        }
        let start = j;
        while j - start < len && input.get(j).is_some_and(u8::is_ascii_digit) {
            digits.buf[digits.len] = input[j] - b'0';
            digits.len += 1;
            j += 1;
        }
        if j - start != len || input.get(j).is_some_and(u8::is_ascii_digit) {
            return None;
        }
    }
    Some((digits, j))
}

/// A run of digit groups joined by single separators, read from `i`.
struct DigitRun {
    /// Every digit's value, groups concatenated.
    digits: Vec<u8>,
    /// The length of each group.
    groups: Vec<usize>,
    /// The separator before each group after the first.
    seps: Vec<u8>,
    /// The offset just past the run.
    end: usize,
}

/// The run of digit groups at `i`, joined by single spaces, hyphens or
/// dots; `None` when there is no digit there. A separator not followed by a
/// digit ends the run.
fn digit_run(input: &[u8], i: usize) -> Option<DigitRun> {
    if !input.get(i).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let mut run = DigitRun { digits: Vec::new(), groups: Vec::new(), seps: Vec::new(), end: i };
    loop {
        let start = run.end;
        while run.end < input.len() && input[run.end].is_ascii_digit() {
            run.digits.push(input[run.end] - b'0');
            run.end += 1;
        }
        run.groups.push(run.end - start);
        let Some(&c) = input.get(run.end) else { break };
        if !matches!(c, b' ' | b'-' | b'.')
            || !input.get(run.end + 1).is_some_and(u8::is_ascii_digit)
        {
            break;
        }
        run.seps.push(c);
        run.end += 1;
    }
    Some(run)
}

/// The number of leading digits forming an assigned ITU-T E.164 country
/// calling code, or `None` when the digits open with none.
///
/// The assigned codes are a prefix code, so the length is unambiguous:
/// zones 1 and 7 are one digit, the codes listed here are two, and every
/// other assigned code is three, which is taken on length rather than from
/// a list so a code assigned later still reads as one. Zero opens no code,
/// being the trunk prefix a national number drops.
pub(crate) fn country_code_len(digits: &[u8]) -> Option<usize> {
    const TWO_DIGIT: [u8; 44] = [
        20, 27, 30, 31, 32, 33, 34, 36, 39, 40, 41, 43, 44, 45, 46, 47, 48, 49, 51, 52, 53, 54,
        55, 56, 57, 58, 60, 61, 62, 63, 64, 65, 66, 81, 82, 84, 86, 90, 91, 92, 93, 94, 95, 98,
    ];
    match digits {
        [0, ..] => None,
        [1, ..] | [7, ..] => Some(1),
        [a, b, ..] if TWO_DIGIT.contains(&(*a * 10 + *b)) => Some(2),
        [_, _, _, ..] => Some(3),
        _ => None,
    }
}

/// A payment-card number: 13-19 digits, contiguous or grouped in one of
/// [`CARD_LAYOUTS`] with one separator throughout, a space or a hyphen, that
/// pass the Luhn checksum.
///
/// The layout is a gate alongside Luhn, which about a tenth of arbitrary
/// digit runs pass: separate numbers in any other layout stay the numbers
/// they look like, and a card followed by a number is the card and then the
/// number. Groups laid out as a card and passing Luhn are read as one
/// whatever they meant, there being nothing in the bytes to separate them.
fn try_creditcard(input: &[u8], i: usize) -> Option<usize> {
    let mut first = i;
    while first < input.len() && input[first].is_ascii_digit() {
        first += 1;
    }
    // A run continuing into a letter is part of a longer token.
    let ends_cleanly = |end: usize| !input.get(end).is_some_and(|b| b.is_ascii_alphabetic());
    // Contiguous: thirteen to nineteen digits.
    if (13..=19).contains(&(first - i)) {
        let mut digits = Digits { buf: [0; DIGITS_MAX], len: first - i };
        for (d, &b) in digits.buf.iter_mut().zip(&input[i..first]) {
            *d = b - b'0';
        }
        return (ends_cleanly(first) && luhn_ok(digits.as_slice())).then_some(first);
    }
    // Grouped: one of the layouts, with one separator throughout.
    for layout in CARD_LAYOUTS {
        for &sep in b" -" {
            if let Some((digits, end)) = fixed_groups(input, i, layout, sep)
                && ends_cleanly(end)
                && luhn_ok(digits.as_slice())
            {
                return Some(end);
            }
        }
    }
    None
}

/// The Luhn (mod-10) checksum over decimal digits, most-significant first.
fn luhn_ok(digits: &[u8]) -> bool {
    crate::checksum::luhn_values(digits)
}

/// A base64 / base64url blob: a `[A-Za-z0-9+/]` run whose length (with any `=`
/// padding) is a multiple of four and at least 16, and which carries a
/// base64-only char (`+` `/` `=`) or a mix of a digit with both cases - so a
/// plain word or a camelCase identifier is not read as base64.
fn try_base64(input: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    while j < input.len() && (input[j].is_ascii_alphanumeric() || matches!(input[j], b'+' | b'/')) {
        j += 1;
    }
    let body = j;
    let mut pad = 0;
    while j < input.len() && input[j] == b'=' && pad < 2 {
        j += 1;
        pad += 1;
    }
    let len = j - i;
    if len < 16 || !len.is_multiple_of(4) {
        return None;
    }
    // Do not stop mid-token: the next byte must be a boundary.
    if input.get(j).is_some_and(|&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')) {
        return None;
    }
    let s = &input[i..body];
    let special = pad > 0 || s.iter().any(|&b| matches!(b, b'+' | b'/'));
    let diverse = s.iter().any(u8::is_ascii_digit)
        && s.iter().any(u8::is_ascii_uppercase)
        && s.iter().any(u8::is_ascii_lowercase);
    (special || diverse).then_some(j)
}

/// The fractional digits a decimal degree needs before it names a point
/// rather than a region. A ten-thousandth of a degree is about eleven
/// metres, the width of a building; above that a pair reads as well as two
/// measurements.
const GEO_MIN_FRACTION: usize = 4;

/// A decimal-degree coordinate `lat,long`: two signed decimals separated by
/// a comma and an optional space, with lat in -90..90 and long in
/// -180..180.
///
/// Two further gates keep a row of figures from reading as a pair. The
/// comma-separated run is exactly two members, so a comma directly before
/// the first, or a comma and a third number after the second, makes it a
/// row of columns. And each part carries [`GEO_MIN_FRACTION`] fractional
/// digits.
fn try_geo(input: &[u8], i: usize) -> Option<usize> {
    // A comma just before, over at most one literal space, puts this inside
    // a longer run. Only a space is stepped over, never a newline, so a
    // chunk-local lex reads this the same as a whole-input one: every chunk
    // boundary directly follows a newline.
    let before = input[..i].strip_suffix(b" ").unwrap_or(&input[..i]);
    if before.ends_with(b",") {
        return None;
    }
    let (lat, lat_frac, a) = signed_decimal(input, i)?;
    let mut j = a;
    if input.get(j) != Some(&b',') {
        return None;
    }
    j += 1;
    if input.get(j) == Some(&b' ') {
        j += 1;
    }
    let (long, long_frac, b) = signed_decimal(input, j)?;
    if input.get(b).is_some_and(|&c| c.is_ascii_alphanumeric()) {
        return None;
    }
    // A third column after the pair.
    let mut k = b;
    if input.get(k) == Some(&b',') {
        k += 1;
        if input.get(k) == Some(&b' ') {
            k += 1;
        }
        if matches!(input.get(k), Some(b'-' | b'+')) || input.get(k).is_some_and(u8::is_ascii_digit)
        {
            return None;
        }
    }
    if lat_frac < GEO_MIN_FRACTION || long_frac < GEO_MIN_FRACTION {
        return None;
    }
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&long)).then_some(b)
}

/// A signed decimal that requires a fractional part (a `.`), returning the
/// value, how many fractional digits it carries, and the end offset. Used by
/// the geo recognizer to reject bare integers and coarse pairs.
///
/// The digits are accumulated rather than parsed, so the read is total: the
/// value serves a range comparison, for which the last place of a long
/// fraction does not signify.
fn signed_decimal(input: &[u8], i: usize) -> Option<(f64, usize, usize)> {
    let mut j = i;
    let negative = input.get(j) == Some(&b'-');
    if matches!(input.get(j), Some(b'-' | b'+')) {
        j += 1;
    }
    let int_start = j;
    let mut value = 0.0f64;
    while j < input.len() && input[j].is_ascii_digit() {
        value = value * 10.0 + f64::from(input[j] - b'0');
        j += 1;
    }
    if j == int_start || input.get(j) != Some(&b'.') {
        return None;
    }
    j += 1;
    let frac_start = j;
    let mut place = 1.0f64;
    while j < input.len() && input[j].is_ascii_digit() {
        place /= 10.0;
        value += f64::from(input[j] - b'0') * place;
        j += 1;
    }
    if j == frac_start {
        return None;
    }
    Some((if negative { -value } else { value }, j - frac_start, j))
}

/// A telephone number in international form: `+`, an ITU-T E.164 country
/// calling code, then the national number, 7 to 15 digits in all, which is
/// E.164's maximum, written in one group or in groups joined by single
/// spaces, hyphens or dots.
///
/// The `+` alone is not marker enough - any signed number followed by
/// figures would fuse - so three properties of the standard carry it. The
/// digits open with an assigned country code, and the first group holds the
/// whole of it, since a code is not written across a separator. A dot
/// separates only where every separator is a dot, a lone dot between digits
/// being a decimal point. And a number under code 1 is a North American
/// plan number, which is fixed at ten national digits.
fn try_phone(input: &[u8], i: usize) -> Option<usize> {
    if input.get(i) != Some(&b'+') {
        return None;
    }
    let run = digit_run(input, i + 1)?;
    if !(7..=15).contains(&run.digits.len()) {
        return None;
    }
    if run.seps.contains(&b'.') && !run.seps.iter().all(|s| *s == b'.') {
        return None;
    }
    let code = country_code_len(&run.digits)?;
    if run.groups[0] < code || (code == 1 && run.digits.len() != 11) {
        return None;
    }
    Some(run.end)
}

/// A North American Numbering Plan number written nationally:
/// `NPA-NXX-XXXX`, optionally prefixed `1-`, joined by hyphens.
///
/// The area code and the central-office code each open with a digit in
/// 2..=9 and neither is an N11 service code; that structure is the marker,
/// a national number carrying no `+`. A space-separated or unbroken
/// ten-digit run carries no marker at all and stays the figures it looks
/// like.
fn try_nanp(input: &[u8], i: usize) -> Option<usize> {
    let (digits, end) = fixed_groups(input, i, &[3, 3, 4], b'-')
        .or_else(|| fixed_groups(input, i, &[1, 3, 3, 4], b'-').filter(|(d, _)| d.as_slice()[0] == 1))?;
    let plan = &digits.as_slice()[digits.len - 10..];
    let assignable = |g: &[u8]| (2..=9).contains(&g[0]) && !(g[1] == 1 && g[2] == 1);
    if !assignable(&plan[..3]) || !assignable(&plan[3..6]) {
        return None;
    }
    if input.get(end).is_some_and(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some(end)
}

/// `scheme://rest`, where `rest` runs to the next whitespace, quote, or
/// angle bracket.
fn try_url(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    if j >= n || !input[j].is_ascii_alphabetic() {
        return None;
    }
    j += 1;
    while j < n
        && (input[j].is_ascii_alphanumeric() || matches!(input[j], b'+' | b'.' | b'-'))
    {
        j += 1;
    }
    if j + 3 > n || &input[j..j + 3] != b"://" {
        return None;
    }
    j += 3;
    let body = j;
    while j < n && !input[j].is_ascii_whitespace() && !matches!(input[j], b'"' | b'<' | b'>') {
        j += 1;
    }
    if j == body { None } else { Some(j) }
}

/// `local@domain.tld`, with a 2+ letter top-level domain.
fn try_email(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    let local = j;
    while j < n
        && (input[j].is_ascii_alphanumeric() || matches!(input[j], b'.' | b'_' | b'%' | b'+' | b'-'))
    {
        j += 1;
    }
    if j == local || j >= n || input[j] != b'@' {
        return None;
    }
    j += 1;
    let domain = j;
    while j < n && (input[j].is_ascii_alphanumeric() || matches!(input[j], b'.' | b'-')) {
        j += 1;
    }
    let dom = &input[domain..j];
    let last_dot = dom.iter().rposition(|&c| c == b'.')?;
    let tld = &dom[last_dot + 1..];
    if tld.len() < 2 || !tld.iter().all(u8::is_ascii_alphabetic) {
        return None;
    }
    Some(j)
}

/// Dotted-quad IPv4 with each octet in `0..=255`.
fn try_ipv4(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    for group in 0..4 {
        let start = j;
        let mut count = 0;
        while j < n && input[j].is_ascii_digit() && count < 3 {
            j += 1;
            count += 1;
        }
        if j == start {
            return None;
        }
        let val = input[start..j].iter().fold(0u32, |a, &c| a * 10 + u32::from(c - b'0'));
        if val > 255 {
            return None;
        }
        if group < 3 {
            if j >= n || input[j] != b'.' {
                return None;
            }
            j += 1;
        }
    }
    if j < n && (input[j].is_ascii_alphanumeric() || input[j] == b'.') {
        return None;
    }
    Some(j)
}

/// An IPv6 address: groups of one to four hex digits joined by single
/// colons, eight of them, or up to seven with one `::` standing for the
/// rest, at least one group written, and no word byte after the last. A
/// clock time is told apart by the address needing a hex letter or a `::`,
/// and a path separator between identifiers by the boundary: `std::alpha`
/// carries a word byte after `::a`, and `Vec::new` writes no group.
fn try_ipv6(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    let mut groups = 0usize;
    let mut colons = 0usize;
    let mut hex_letter = false;
    let mut double_colon = false;
    loop {
        let start = j;
        while j < n && input[j].is_ascii_hexdigit() {
            hex_letter |= input[j].is_ascii_alphabetic();
            j += 1;
        }
        if j - start > 4 {
            return None;
        }
        if j > start {
            groups += 1;
        }
        if j + 1 < n && input[j] == b':' && input[j + 1] == b':' {
            if double_colon {
                return None;
            }
            double_colon = true;
            colons += 2;
            j += 2;
            continue;
        }
        if j > start && j + 1 < n && input[j] == b':' && input[j + 1].is_ascii_hexdigit() {
            colons += 1;
            j += 1;
            continue;
        }
        break;
    }
    let complete = if double_colon { (1..=7).contains(&groups) } else { groups == 8 };
    if !complete || colons < 2 || (!hex_letter && !double_colon) {
        return None;
    }
    if input.get(j).is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'_') {
        return None;
    }
    Some(j)
}

/// A timestamp in any of the forms logs write: ISO `YYYY-MM-DD`, the slash
/// dates `YYYY/MM/DD`, `D/M/YYYY` and `M/D/YYYY`, a clock time, a date and a
/// clock joined by `T` or by a single space, RFC 3339's fractional seconds
/// and zone on the time, syslog's `Mon DD HH:MM:SS`, and Apache's
/// `DD/Mon/YYYY:HH:MM:SS` with an optional ` +hhmm`.
///
/// Each form that spans a space carries a marker the standard writes: the
/// whole date shape directly before the whole clock shape, a month name
/// before a day and a clock, a month name between slashes before a
/// colon-joined clock.
fn try_timestamp(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    if let Some(end) = try_syslog(input, i) {
        return Some(end);
    }
    if let Some(end) = try_apache(input, i) {
        return Some(end);
    }
    if let Some(mut j) = try_date(input, i) {
        if j < n && matches!(input[j], b'T' | b't' | b' ')
            && let Some(jt) = try_time(input, j + 1, false)
        {
            j = jt;
        }
        return Some(j);
    }
    try_time(input, i, false)
}

/// Whether a month abbreviation at `i` opens a syslog timestamp: three
/// letters, a space, and a day that may be padded by a second space.
fn syslog_head(input: &[u8], i: usize) -> bool {
    crate::typed::month_abbrev(&input[i..]).is_some()
        && input.get(i + 3) == Some(&b' ')
        && (input.get(i + 4).is_some_and(u8::is_ascii_digit)
            || (input.get(i + 4) == Some(&b' ') && input.get(i + 5).is_some_and(u8::is_ascii_digit)))
}

/// syslog: a month abbreviation, one or two spaces, a day of one or two
/// digits in `1..=31`, a space, and a clock time carrying seconds (`Sep 14
/// 18:50:53`, `Sep  5 03:04:05`). A month and a day with no clock after them
/// stay a word and a number.
fn try_syslog(input: &[u8], i: usize) -> Option<usize> {
    if !syslog_head(input, i) {
        return None;
    }
    let mut j = i + 4;
    if input.get(j) == Some(&b' ') {
        j += 1;
    }
    let day_start = j;
    let mut day = 0u32;
    while j < input.len() && input[j].is_ascii_digit() && j - day_start < 2 {
        day = day * 10 + u32::from(input[j] - b'0');
        j += 1;
    }
    if j == day_start || day == 0 || day > 31 || input.get(j) != Some(&b' ') {
        return None;
    }
    let end = try_time(input, j + 1, false)?;
    (end - (j + 1) >= 8).then_some(end)
}

/// Apache: `DD/Mon/YYYY:HH:MM:SS`, then an optional space and a signed
/// four-digit zone. The month name between the slashes and the colon joining
/// the year to the clock are the markers.
fn try_apache(input: &[u8], i: usize) -> Option<usize> {
    let digit_at = |k: usize| input.get(k).is_some_and(u8::is_ascii_digit);
    if !(digit_at(i) && digit_at(i + 1) && input.get(i + 2) == Some(&b'/')) {
        return None;
    }
    crate::typed::month_abbrev(input.get(i + 3..)?)?;
    if input.get(i + 6) != Some(&b'/') || !(i + 7..i + 11).all(digit_at) {
        return None;
    }
    if input.get(i + 11) != Some(&b':') {
        return None;
    }
    let end = try_time(input, i + 12, true)?;
    (end - (i + 12) >= 8).then_some(end)
}

/// A date with no clock: ISO `YYYY-MM-DD`, or one of the slash forms
/// [`crate::typed::slash_date`] reads. The slash forms are read there so the
/// shape the lexer spans and the fields a predicate reads out of it are one
/// piece of code; the extent it answers is the same under either order.
fn try_date(input: &[u8], i: usize) -> Option<usize> {
    try_iso_date(input, i).or_else(|| crate::typed::slash_date(input, i).map(|d| d.end))
}

/// ISO `YYYY-MM-DD`, not run into a further digit.
fn try_iso_date(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    let digits = |count: usize, j: &mut usize| -> bool {
        for _ in 0..count {
            if *j < n && input[*j].is_ascii_digit() {
                *j += 1;
            } else {
                return false;
            }
        }
        true
    };
    if !digits(4, &mut j) {
        return None;
    }
    if j >= n || input[j] != b'-' {
        return None;
    }
    j += 1;
    if !digits(2, &mut j) {
        return None;
    }
    if j >= n || input[j] != b'-' {
        return None;
    }
    j += 1;
    if !digits(2, &mut j) {
        return None;
    }
    if j < n && input[j].is_ascii_digit() {
        return None;
    }
    Some(j)
}

/// A UUID / GUID: `8-4-4-4-12` hex digits joined by dashes, not run into an
/// adjacent hex or word byte.
fn try_uuid(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut j = i;
    for (g, &len) in GROUPS.iter().enumerate() {
        for _ in 0..len {
            if j < n && input[j].is_ascii_hexdigit() {
                j += 1;
            } else {
                return None;
            }
        }
        if g < GROUPS.len() - 1 {
            if j >= n || input[j] != b'-' {
                return None;
            }
            j += 1;
        }
    }
    if j < n && (input[j].is_ascii_alphanumeric() || input[j] == b'-') {
        return None;
    }
    Some(j)
}

/// A MAC / EUI-48 address: six two-hex-digit groups joined by a single
/// separator, either `:` or `-`. The separator is fixed by the byte after
/// the first pair so a mixed-separator run is rejected.
fn try_mac(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let sep = *input.get(i + 2)?;
    if sep != b':' && sep != b'-' {
        return None;
    }
    let mut j = i;
    for group in 0..6 {
        if j + 2 > n || !input[j].is_ascii_hexdigit() || !input[j + 1].is_ascii_hexdigit() {
            return None;
        }
        j += 2;
        if group < 5 {
            if j >= n || input[j] != sep {
                return None;
            }
            j += 1;
        }
    }
    if j < n && (input[j].is_ascii_alphanumeric() || input[j] == b':' || input[j] == b'-') {
        return None;
    }
    Some(j)
}

/// A CIDR block: an IPv4 address then `/` and a 0-32 prefix length. Reuses
/// the IPv4 recognizer, whose trailing guard already accepts the `/`.
fn try_cidr(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let after_ip = try_ipv4(input, i)?;
    if after_ip >= n || input[after_ip] != b'/' {
        return None;
    }
    let start = after_ip + 1;
    let mut j = start;
    while j < n && input[j].is_ascii_digit() && j - start < 2 {
        j += 1;
    }
    if j == start {
        return None;
    }
    let prefix = input[start..j].iter().fold(0u32, |a, &c| a * 10 + u32::from(c - b'0'));
    if prefix > 32 {
        return None;
    }
    if j < n && (input[j].is_ascii_alphanumeric() || input[j] == b'.') {
        return None;
    }
    Some(j)
}

/// A semantic version: three dot-separated numeric parts, with an optional
/// `-prerelease` and/or `+build` suffix. Rejected when a fourth dotted
/// number follows (that is an IPv4 address, not a version).
fn try_semver(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    // An optional `v` / `V` prefix (the git-tag style `v1.2.3`), taken only
    // when a digit follows, so the word `version` is not mistaken for one.
    if matches!(input.get(j), Some(b'v' | b'V')) && input.get(j + 1).is_some_and(u8::is_ascii_digit) {
        j += 1;
    }
    for part in 0..3 {
        let start = j;
        while j < n && input[j].is_ascii_digit() {
            j += 1;
        }
        if j == start {
            return None;
        }
        if part < 2 {
            if j >= n || input[j] != b'.' {
                return None;
            }
            j += 1;
        }
    }
    if j < n && input[j] == b'.' && input.get(j + 1).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    // Optional `-prerelease` and `+build`: dot- or dash-joined alphanumerics,
    // each taken only when an alphanumeric follows the introducer.
    let ident = |j: &mut usize| {
        while *j < n && (input[*j].is_ascii_alphanumeric() || matches!(input[*j], b'.' | b'-')) {
            *j += 1;
        }
    };
    if j + 1 < n && input[j] == b'-' && input[j + 1].is_ascii_alphanumeric() {
        j += 1;
        ident(&mut j);
    }
    if j + 1 < n && input[j] == b'+' && input[j + 1].is_ascii_alphanumeric() {
        j += 1;
        ident(&mut j);
    }
    if j < n && (input[j].is_ascii_alphanumeric() || input[j] == b'_') {
        return None;
    }
    Some(j)
}

/// A hash digest: a run of exactly 32, 40, or 64 hex characters with at least
/// one hex letter (md5 / sha1 / sha256). The letter requirement keeps a long
/// all-decimal number from reading as a hash; the exact-length requirement and
/// the trailing-word guard keep a longer hex/word run from matching.
fn try_hash(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    let mut has_letter = false;
    while j < n && input[j].is_ascii_hexdigit() {
        has_letter |= input[j].is_ascii_alphabetic();
        j += 1;
    }
    let len = j - i;
    if !matches!(len, 32 | 40 | 64) || !has_letter {
        return None;
    }
    if j < n && (input[j].is_ascii_alphanumeric() || input[j] == b'_') {
        return None;
    }
    Some(j)
}

/// A money amount: `$` then a digit, optional thousands commas, and an
/// optional decimal part. `$5`, `$1,000`, `$1,234.56`, `$3.99`.
fn try_money(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    if input.get(i) != Some(&b'$') || !input.get(i + 1).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let mut j = i + 1;
    while j < n && (input[j].is_ascii_digit() || input[j] == b',') {
        j += 1;
    }
    // A trailing comma is not part of the amount.
    if input[j - 1] == b',' {
        j -= 1;
    }
    if j + 1 < n && input[j] == b'.' && input[j + 1].is_ascii_digit() {
        j += 1;
        while j < n && input[j].is_ascii_digit() {
            j += 1;
        }
    }
    if j < n && input[j].is_ascii_alphanumeric() {
        return None;
    }
    Some(j)
}

/// A percentage: a number (optional decimal) immediately followed by `%`.
fn try_percent(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    let start = j;
    while j < n && input[j].is_ascii_digit() {
        j += 1;
    }
    if j == start {
        return None;
    }
    if j + 1 < n && input[j] == b'.' && input[j + 1].is_ascii_digit() {
        j += 1;
        while j < n && input[j].is_ascii_digit() {
            j += 1;
        }
    }
    (j < n && input[j] == b'%').then_some(j + 1)
}

/// A byte size: a number (optional decimal) then a unit ending in `B` (with an
/// optional `K`/`M`/`G`/`T`/`P`/`E` magnitude and optional binary `i`):
/// `10B`, `512KB`, `10MB`, `1.5GiB`, `2TB`. The trailing `B` is required, so a
/// bare `10M` stays a number plus a word rather than being read as bytes.
fn try_bytesize(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    let start = j;
    while j < n && input[j].is_ascii_digit() {
        j += 1;
    }
    if j == start {
        return None;
    }
    if j + 1 < n && input[j] == b'.' && input[j + 1].is_ascii_digit() {
        j += 1;
        while j < n && input[j].is_ascii_digit() {
            j += 1;
        }
    }
    let mag = j;
    if j < n && matches!(input[j] | 0x20, b'k' | b'm' | b'g' | b't' | b'p' | b'e') {
        j += 1;
    }
    if j > mag && j < n && input[j] == b'i' {
        j += 1;
    }
    if j >= n || (input[j] | 0x20) != b'b' {
        return None;
    }
    j += 1;
    if j < n && input[j].is_ascii_alphanumeric() {
        return None;
    }
    Some(j)
}

/// A duration: one or more `number + time-unit` segments (`1500ms`, `2.5s`,
/// `3h20m`, `90s`). Units are the two-letter `ns` / `us` / `ms` and the
/// single-letter `s` / `m` / `h` / `d` / `w` / `y` (lowercase, the
/// conventional form). A unit run into more letters is a word, not a unit, so
/// `5string` stays a number plus a word; a bare non-time letter (`5x`) is not a
/// duration. The single-letter units are accepted at a boundary, so `5m` reads
/// as five minutes - the common log/config reading (documented trade-off).
fn try_duration(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    let mut segments = 0usize;
    loop {
        let num_start = j;
        while j < n && input[j].is_ascii_digit() {
            j += 1;
        }
        if j == num_start {
            break;
        }
        if j + 1 < n && input[j] == b'.' && input[j + 1].is_ascii_digit() {
            j += 1;
            while j < n && input[j].is_ascii_digit() {
                j += 1;
            }
        }
        let two = input.get(j..j + 2).unwrap_or(&[]);
        if matches!(two, b"ns" | b"us" | b"ms") {
            j += 2;
        } else if j < n && matches!(input[j], b's' | b'm' | b'h' | b'd' | b'w' | b'y') {
            j += 1;
        } else {
            // A number with no time unit ends the duration; rewind it.
            j = num_start;
            break;
        }
        segments += 1;
        // A unit run into another letter is a word (e.g. `5string`), not a unit.
        if j < n && input[j].is_ascii_alphabetic() {
            return None;
        }
        // Continue only into another `number + unit` segment (`3h20m`).
        if !(j < n && input[j].is_ascii_digit()) {
            break;
        }
    }
    if segments == 0 || (j < n && input[j].is_ascii_alphanumeric()) {
        return None;
    }
    Some(j)
}

/// The path-body scan shared by every path form: consume path characters from
/// `j` and accept only when the whole span (from `start`) is at least three
/// bytes, so a lone `/x` or `./` does not read as a path.
fn consume_path(input: &[u8], mut j: usize, start: usize) -> Option<usize> {
    let n = input.len();
    while j < n
        && (input[j].is_ascii_alphanumeric()
            || matches!(input[j], b'/' | b'\\' | b'.' | b'_' | b'-' | b'~'))
    {
        j += 1;
    }
    (j - start >= 3).then_some(j)
}

/// A filesystem path. Recognized forms: a Unix absolute `/a/b` (only at a clean
/// boundary, so the `/` in `a/b` division is not a path), the relative
/// introducers `./` `../` `~/`, a Windows drive path `X:\...` or `X:/...`, and
/// a UNC `\\server\share`.
fn try_path(input: &[u8], i: usize) -> Option<usize> {
    let b = input[i];
    // Windows drive path: X:\ or X:/
    if b.is_ascii_alphabetic()
        && input.get(i + 1) == Some(&b':')
        && matches!(input.get(i + 2), Some(b'\\' | b'/'))
    {
        return consume_path(input, i + 3, i);
    }
    // UNC: \\server\share
    if b == b'\\' && input.get(i + 1) == Some(&b'\\') {
        return consume_path(input, i + 2, i);
    }
    // Relative introducers: ./  ../  ~/
    let rel = (b == b'.' && input.get(i + 1) == Some(&b'/'))
        || (b == b'.' && input.get(i + 1) == Some(&b'.') && input.get(i + 2) == Some(&b'/'))
        || (b == b'~' && input.get(i + 1) == Some(&b'/'));
    if rel {
        return consume_path(input, i, i);
    }
    // Unix absolute: /a/b, but only after a clean left boundary (start of
    // input, whitespace, a quote, or an opening bracket). This keeps `a/b`
    // division and an HTML close tag `</div>` from reading as a path, while a
    // real path is preceded by a space, a quote, or a bracket.
    if b == b'/'
        && (i == 0 || matches!(input[i - 1], b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\'' | b'(' | b'[' | b'{'))
        && input.get(i + 1).is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    {
        return consume_path(input, i, i);
    }
    None
}

/// A hex color literal: `#` then exactly three or six hex digits, not run
/// into an adjacent hex or word byte.
fn try_hexcolor(input: &[u8], i: usize) -> Option<usize> {
    let n = input.len();
    if input.get(i) != Some(&b'#') {
        return None;
    }
    let mut j = i + 1;
    while j < n && input[j].is_ascii_hexdigit() {
        j += 1;
    }
    let len = j - (i + 1);
    if len != 3 && len != 6 {
        return None;
    }
    if j < n && (input[j].is_ascii_alphanumeric() || input[j] == b'_') {
        return None;
    }
    Some(j)
}

/// `HH:MM`, `HH:MM:SS`, `HH:MM:SS.frac`, then, after seconds, an optional
/// zone: `Z`, `z`, `+HH:MM`, `-HHMM` or `+HH`. With `space_zone`, a single
/// space may stand before a signed four-digit zone, which only the Apache
/// form writes; elsewhere a time followed by a space and a signed number is
/// a time and a number.
fn try_time(input: &[u8], i: usize, space_zone: bool) -> Option<usize> {
    let n = input.len();
    let mut j = i;
    if j + 2 > n || !input[j].is_ascii_digit() || !input[j + 1].is_ascii_digit() {
        return None;
    }
    j += 2;
    if j >= n || input[j] != b':' {
        return None;
    }
    j += 1;
    if j + 2 > n || !input[j].is_ascii_digit() || !input[j + 1].is_ascii_digit() {
        return None;
    }
    j += 2;
    let mut seconds = false;
    if j + 2 < n && input[j] == b':' && input[j + 1].is_ascii_digit() && input[j + 2].is_ascii_digit()
    {
        j += 3;
        seconds = true;
        if j + 1 < n && input[j] == b'.' && input[j + 1].is_ascii_digit() {
            j += 1;
            while j < n && input[j].is_ascii_digit() {
                j += 1;
            }
        }
    }
    if j < n && input[j].is_ascii_digit() {
        return None;
    }
    if !seconds {
        return Some(j);
    }
    match zone_end(input, j, space_zone) {
        Some(end) => Some(end),
        None => Some(j),
    }
}

/// The end of a zone at `j`, or `None` where none is written: `Z` or `z`; a
/// sign, two hour digits, then `:MM`, `MM` or nothing; with `space_zone`, a
/// space then a sign and four digits. A zone that runs into a letter, a digit,
/// a colon or a dot is not one, so `12:30:45-13:00:00` stays two times.
fn zone_end(input: &[u8], j: usize, space_zone: bool) -> Option<usize> {
    let digit_at = |k: usize| input.get(k).is_some_and(u8::is_ascii_digit);
    let ends_cleanly =
        |e: usize| !input.get(e).is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.'));
    match input.get(j) {
        Some(b'Z' | b'z') => ends_cleanly(j + 1).then_some(j + 1),
        Some(b'+' | b'-') => {
            if !(digit_at(j + 1) && digit_at(j + 2)) {
                return None;
            }
            let mut e = j + 3;
            if input.get(e) == Some(&b':') && digit_at(e + 1) && digit_at(e + 2) {
                e += 3;
            } else if digit_at(e) && digit_at(e + 1) {
                e += 2;
            }
            ends_cleanly(e).then_some(e)
        }
        Some(b' ') if space_zone && matches!(input.get(j + 1), Some(b'+' | b'-')) => {
            let e = j + 6;
            ((j + 2..e).all(digit_at) && ends_cleanly(e)).then_some(e)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_from_a_pattern_fuses_its_match_into_one_token_and_pairs_the_brackets_again() {
        let mut set = crate::custom::ShapeSet::new();
        set.declare_text("kind assign = \\W \"=\" \\N").expect("declares");
        let input = b"f(x = 1) y";
        let toks = lex_with_shapes(input, &[], &set, 0);
        let kinds: Vec<TokenKind> =
            toks.iter().filter(|t| t.is_significant()).map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Word,
                TokenKind::Open(BracketKind::Paren),
                TokenKind::Custom(0),
                TokenKind::Close(BracketKind::Paren),
                TokenKind::Word,
            ]
        );
        let open = toks
            .iter()
            .position(|t| t.kind == TokenKind::Open(BracketKind::Paren))
            .expect("an open");
        let close = toks
            .iter()
            .position(|t| t.kind == TokenKind::Close(BracketKind::Paren))
            .expect("a close");
        assert_eq!(toks[open].mate(), Some(close));
        assert_eq!(toks[close].mate(), Some(open));
        let pat = crate::parser::parse_with_shapes("\\{assign}", &set).expect("parses");
        let spans = crate::engine::scan_with_shapes(&pat, input, &set);
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start(), spans[0].end()), (2, 7));
    }

    fn kinds(input: &str) -> Vec<TokenKind> {
        lex(input.as_bytes()).iter().map(|t| t.kind).collect()
    }

    #[test]
    fn a_path_separator_is_not_an_address() {
        // `::` between identifiers is two punctuation tokens: an address
        // writes at least one group of at most four hex digits and ends
        // before a word byte, and without a `::` it writes eight of them.
        let sep = vec![TokenKind::Word, TokenKind::Punct, TokenKind::Punct, TokenKind::Word];
        assert_eq!(kinds("std::alpha"), sep);
        assert_eq!(kinds("Vec::new"), sep);
        assert_eq!(kinds("::"), vec![TokenKind::Punct, TokenKind::Punct]);
        assert_eq!(kinds("deadbeef::1"), vec![TokenKind::Word, TokenKind::Ip]);
        assert_eq!(
            kinds("a:b:c"),
            vec![TokenKind::Word, TokenKind::Punct, TokenKind::Word, TokenKind::Punct, TokenKind::Word]
        );
        for addr in [
            "fe80::1",
            "::1",
            "2001:db8::ff00:42:8329",
            "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
            "a::b",
            "fe80::",
        ] {
            assert_eq!(kinds(addr), vec![TokenKind::Ip], "{addr}");
        }
        assert_eq!(kinds("12:30:45"), vec![TokenKind::Timestamp]);
    }

    #[test]
    fn recognizer_gates_skip_only_refusals() {
        // Every typed shape the chain recognizes, each also in a form one
        // byte short of it, plus the plain identifiers and numbers the gates
        // exist to spare; the gated chain must agree with the ungated one at
        // every position a typed token could start.
        let corpus = concat!(
            "https://example.com/a?b=1 svn+ssh://host/x ftp.example.com eyJhbGci.eyJzdWI.SflKx eyJ\n",
            "+1 555-123-4567 550e8400-e29b-41d4-a716-446655440000 550e8400-e29b 00:1a:2b:3c:4d:5e 00-1a-2b\n",
            "d41d8cd98f00b204e9800998ecf8427e da39a3ee5e6b4b0d3255bfef95601890afd80709 d41d8cd98f00b204e9800998ecf8427eX\n",
            "192.168.1.0/24 192.168.1.1 10.0.0.256 1234 fe80::1 2001:db8::ff00:42:8329 12:30 12:30:45 ::1\n",
            "user@example.com first.last+tag@mail.co a.b-c%d@x.y user@ user_name@host.org\n",
            "4111111111111111 4111 1111 1111 1111 4111-1111-1111-1111 1234567890123 12345678901234567\n",
            "698 27 54 164 196 330 3782 822463 10005 4111 1111 1111 1111 003 4111 1111-1111 1111 10000 00000 00009\n",
            "212-555-1234 1-800-555-0199 123-456-7890 555-123-4567 2024-01-02 12-34-56 +1 555-123-4567\n",
            "+44 20 7123 4567 +33 1 42 68 53 00 +5 10 20 30 40 +3.5 7 8 9 10 11 +1.555.123.4567 +55 10 20 30 40\n",
            "37.7749,-122.4194 55.7558, 37.6173 12.5,34.7 1.25,2.50,3.75 40.7,-74.0 x,37.7749,-122.4194\n",
            "40.7128,-74.0060 -33.8688, 151.2093 12,34 $1,234.56 $5 $ v1.2.3 1.2.3 1.2 version\n",
            "10MB 1.5GiB 512KB 10M 50% 12.5% 1500ms 2.5s 3h20m 5string 5x 90s\n",
            "#fff #a1b2c3 #ab C:\\Users\\x D:/data ./rel/path ../up ~/home /usr/bin a/b </div>\n",
            "SGVsbG8gV29ybGQhIQ== YWJjZGVmZ2hpamtsbW5vcA abcdefghijklmnop_x abc+def/ghi+jkl/mnop==\n",
            "2024-01-02 2024-01-02T12:30:00 2024-1-2 let value_123 = 4551 ; call_7(alpha, beta, 7) ;\n",
            "key_9: item_9, item_10 ; if (cond_3) { do_3(x) ; } snake_case_name CamelCase x_ _y 0x1F 007\n",
            "e ey eyJ eyJa.b.c beyJa.b.c plain, word; text) tail. 42, 1234 56789 1234-5678 4111 1111 9 0 007. 1e5 12x 3rd\n",
            "abcdefghijklm abcdefghijklmn abcdefghijklmn+ word/ word: word- word@ word% 123456789012 1234567890123 12345:\n",
            "Sep 14 18:50:53 Sep  5 03:04:05 Dec 31 23:59:59 Sep 14 apples Sep 99 10:00:00 dec 1 10:00\n",
            "10/Oct/2000:13:55:36 -0700 10/Oct/2000:13:55:36 10/Xyz/2000:13:55:36 2026-09-01T12:00:00Z\n",
            "2026-09-01T12:00:00.123456+02:00 2026-09-01 12:00:00 12:30:45-13:00:00 12:30-13:00 12:30:45 -0700\n",
        );
        let input = corpus.as_bytes();
        for i in 0..input.len() {
            if !typed_token_possible(input[i]) {
                continue;
            }
            let facts = RunFacts::at(input, i);
            let gated = try_typed_token_chain(input, i, Some(&facts));
            let ungated = try_typed_token_chain(input, i, None);
            assert_eq!(
                gated,
                ungated,
                "at {i} ({:?})",
                String::from_utf8_lossy(&input[i..(i + 24).min(input.len())])
            );
        }
    }

    #[test]
    fn jwt_and_credit_card() {
        // JWT: three base64url segments, anchored by the eyJ header prefix.
        assert_eq!(
            kinds("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.SflKxwRJSMeKKF2QT4fwpMeJf"),
            vec![TokenKind::Jwt]
        );
        // A plain dotted identifier / semver is not a JWT.
        assert!(!kinds("1.2.3").contains(&TokenKind::Jwt));
        assert!(!kinds("a.b.c").contains(&TokenKind::Jwt));
        // Credit card: Luhn-valid, contiguous or in an issuer's grouping.
        assert_eq!(kinds("4111 1111 1111 1111"), vec![TokenKind::CreditCard]);
        assert_eq!(kinds("4111-1111-1111-1111"), vec![TokenKind::CreditCard]);
        assert_eq!(kinds("4111111111111111"), vec![TokenKind::CreditCard]);
        assert_eq!(kinds("3782 822463 10005"), vec![TokenKind::CreditCard], "fifteen digits, 4-6-5");
        assert_eq!(kinds("3056 930902 5904"), vec![TokenKind::CreditCard], "fourteen digits, 4-6-4");
        assert_eq!(
            kinds("4111 1111 1111 1111 003"),
            vec![TokenKind::CreditCard],
            "nineteen digits, 4-4-4-4-3"
        );
        // A 16-digit run that fails Luhn is a plain number, not a card.
        assert!(!kinds("1234567812345678").contains(&TokenKind::CreditCard));
    }

    #[test]
    fn digit_groups_in_no_issuers_layout_are_the_numbers_they_look_like() {
        // Luhn-valid as a whole, laid out as no card is: six numbers.
        let numbers = |s: &str| kinds(s).into_iter().filter(|k| *k == TokenKind::Number).count();
        assert!(!kinds("698 27 54 164 196 330").contains(&TokenKind::CreditCard));
        assert_eq!(numbers("698 27 54 164 196 330"), 6);
        assert!(!kinds("10000 00000 00009").contains(&TokenKind::CreditCard), "5-5-5, Luhn-valid");
        assert!(!kinds("4111 1111-1111 1111").contains(&TokenKind::CreditCard), "two separators");
        assert!(!kinds("4111 1111 1111 11111").contains(&TokenKind::CreditCard), "a five-digit group");
        // A card followed by a number is the card, then the number.
        let then_number = kinds("4111 1111 1111 1111 42");
        assert_eq!(then_number.first(), Some(&TokenKind::CreditCard));
        assert_eq!(then_number.last(), Some(&TokenKind::Number));
        assert_eq!(numbers("4111 1111 1111 1111 42"), 1);
    }

    #[test]
    fn runs_of_separate_numbers_never_lex_as_a_card() {
        // Lines of one to ten digit groups of one to four digits each, the
        // shape of a table row or a measurement stream. Luhn passes about a
        // tenth of them as a whole. A line holding no run of groups laid out
        // as a card must not lex as one; a line that does hold such a run is
        // counted instead, since those groups are what a card looks like and
        // nothing in the bytes separates the two.
        let mut state = 0x9e37_79b9u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let card_shaped = |widths: &[usize]| {
            (0..widths.len())
                .any(|i| CARD_LAYOUTS.iter().any(|l| widths[i..].starts_with(l)))
        };
        let mut lines = 0usize;
        let mut fused = 0usize;
        let mut shaped = 0usize;
        for _ in 0..10_000 {
            let n = 1 + next() % 10;
            let mut widths: Vec<usize> = Vec::new();
            let mut line = String::new();
            for _ in 0..n {
                let width = (1 + next() % 4) as usize;
                widths.push(width);
                let value = next() % 10u32.pow(width as u32);
                line.push_str(&format!("{value:0width$} "));
            }
            if card_shaped(&widths) {
                shaped += 1;
                continue;
            }
            lines += 1;
            if lex(line.as_bytes()).iter().any(|t| t.kind == TokenKind::CreditCard) {
                fused += 1;
            }
        }
        assert!(lines > 9_000, "{lines} lines generated, {shaped} card-shaped");
        assert_eq!(fused, 0, "{fused} of {lines} lines of separate numbers lexed as a card");
    }

    #[test]
    fn base64_geo_phone() {
        // Base64: mult-of-4, >= 16, with padding or charset diversity.
        assert!(kinds("aGVsbG8gd29ybGQhIQ==").contains(&TokenKind::Base64));
        assert!(kinds("YWJjMTIzWFlaZGVmR0hJ").contains(&TokenKind::Base64));
        // A long lowercase word is not base64 (no diversity, no padding).
        assert!(!kinds("supercalifragilistic").contains(&TokenKind::Base64));
        // Geo: in-range decimals separated by a comma.
        assert!(kinds("37.7749,-122.4194").contains(&TokenKind::Geo));
        assert!(!kinds("12,34").contains(&TokenKind::Geo)); // no decimals
        assert!(!kinds("91.0000,10.0000").contains(&TokenKind::Geo)); // lat out of range
        // Phone: + prefix then 7-15 digits.
        assert_eq!(kinds("+15551234567"), vec![TokenKind::Phone]);
        assert!(!kinds("12345").contains(&TokenKind::Phone)); // no + prefix
    }

    #[test]
    fn a_coordinate_is_a_pair_at_a_points_precision() {
        // Every continent, each written as a mapping service writes it.
        for coord in [
            "37.7749,-122.4194",
            "40.7128,-74.0060",
            "-33.8688, 151.2093",
            "51.5074,-0.1278",
            "35.6762,139.6503",
            "-22.9068,-43.1729",
            "1.3521,103.8198",
            "55.7558, 37.6173",
        ] {
            assert_eq!(kinds(coord), vec![TokenKind::Geo], "{coord}");
        }
        // A row of columns is not a pair, wherever the reading starts.
        for row in ["1.2500,2.5000,3.7500", "1.2500, 2.5000, 3.7500", "0.1111,0.2222,0.3333,0.4444"] {
            assert!(!kinds(row).contains(&TokenKind::Geo), "{row}");
        }
        // A pair coarser than a ten-thousandth of a degree reads as two
        // numbers: at that width it names a region, not a point.
        for coarse in ["12.5,34.7", "40.7,-74.0", "1.25,2.50"] {
            assert!(!kinds(coarse).contains(&TokenKind::Geo), "{coarse}");
        }
    }

    #[test]
    fn a_phone_number_opens_with_an_assigned_country_code() {
        // Written as each country writes it.
        for phone in [
            "+15551234567",
            "+1 555 123 4567",
            "+1-555-123-4567",
            "+1 555-123-4567",
            "+1.555.123.4567",
            "+44 20 7123 4567",
            "+49 30 12345678",
            "+81 3 1234 5678",
            "+33 1 42 68 53 00",
            "+86 10 8888 8888",
            "+351 21 123 4567",
        ] {
            assert_eq!(kinds(phone), vec![TokenKind::Phone], "{phone}");
        }
        // A signed number followed by figures opens with no country code,
        // since zone 5 has no one-digit code and a code is not written
        // across a separator.
        for run in ["+5 10 20 30 40", "+1 2 3 4 5 6 7", "+3 51 234 5678"] {
            assert!(!kinds(run).contains(&TokenKind::Phone), "{run}");
        }
        // A decimal is not a group separator unless every separator is a dot.
        for run in ["+44.5 20 30 40 50", "+3.5 7 8 9 10 11"] {
            assert!(!kinds(run).contains(&TokenKind::Phone), "{run}");
        }
        // Under code 1 the plan is fixed at ten national digits.
        assert!(!kinds("+1 234 5678").contains(&TokenKind::Phone));
        // A leading zero is a trunk prefix, not the start of a code.
        assert!(!kinds("+07 123 456 78").contains(&TokenKind::Phone));
    }

    #[test]
    fn a_north_american_number_is_read_without_its_country_code() {
        for phone in ["212-555-1234", "1-800-555-0199", "800-555-0199", "1-212-555-1234"] {
            assert_eq!(kinds(phone), vec![TokenKind::Phone], "{phone}");
        }
        // An area code or an exchange opening with 0 or 1, or an N11 service
        // code in either place, is not assignable.
        for not_a_number in ["123-456-7890", "555-123-4567", "211-555-1234", "212-411-1234"] {
            assert!(!kinds(not_a_number).contains(&TokenKind::Phone), "{not_a_number}");
        }
        // Space-separated and unbroken ten-digit runs carry no marker.
        assert!(!kinds("212 555 1234").contains(&TokenKind::Phone));
        assert!(!kinds("2125551234").contains(&TokenKind::Phone));
        // A date keeps its own reading.
        assert_eq!(kinds("2024-01-02"), vec![TokenKind::Timestamp]);
    }

    #[test]
    fn rows_of_decimals_never_lex_as_a_coordinate() {
        // Comma-separated decimal rows, the shape of a measurement CSV: two
        // to eight columns, one to six fractional digits, values in 0..100.
        // A two-column row at four or more fractional digits is a coordinate
        // pair in every observable respect and is counted, not asserted.
        let mut state = 0x1357_9bdfu32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let mut rows = 0usize;
        let mut fused = 0usize;
        for _ in 0..10_000 {
            let columns = 2 + next() % 7;
            let frac = 1 + next() % 6;
            if columns == 2 && frac >= GEO_MIN_FRACTION as u32 {
                continue;
            }
            let sep = if next() % 2 == 0 { "," } else { ", " };
            let mut row = String::new();
            for k in 0..columns {
                if k > 0 {
                    row.push_str(sep);
                }
                let whole = next() % 100;
                let f = next() % 10u32.pow(frac);
                row.push_str(&format!("{whole}.{f:0width$}", width = frac as usize));
            }
            rows += 1;
            if lex(row.as_bytes()).iter().any(|t| t.kind == TokenKind::Geo) {
                fused += 1;
            }
        }
        assert!(rows > 8_000, "{rows} rows generated");
        assert_eq!(fused, 0, "{fused} of {rows} decimal rows lexed as a coordinate");
    }

    #[test]
    fn unicode_letters_join_words() {
        // A multi-byte letter is part of a Word token, never per-byte shards.
        assert_eq!(kinds("\u{03B8}"), vec![TokenKind::Word]); // Greek theta
        assert_eq!(kinds("na\u{EF}ve caf\u{E9}"), vec![
            TokenKind::Word,
            TokenKind::Whitespace,
            TokenKind::Word
        ]);
        // Mixed ASCII + unicode + ASCII stays one word.
        let toks = lex("Gr\u{FC}\u{DF}e42".as_bytes());
        assert_eq!(toks.len(), 1, "one token: {toks:?}");
        assert_eq!(toks[0].kind, TokenKind::Word);
        assert_eq!(toks[0].len(), "Gr\u{FC}\u{DF}e42".len());
    }

    #[test]
    fn cjk_runs_are_single_word_tokens() {
        // The byte-tokenizer failure mode: a CJK char shattering into three
        // byte tokens. Here a CJK run is one Word token, boundaries only at
        // real separators.
        let toks = lex("\u{4E2D}\u{6587}\u{5206}\u{8BCD}".as_bytes()); // 中文分词
        assert_eq!(toks.len(), 1, "one Word token: {toks:?}");
        assert_eq!(toks[0].kind, TokenKind::Word);
        assert_eq!(toks[0].end - toks[0].start, 12, "4 chars x 3 bytes, unsplit");
        // Mixed script: ASCII and CJK words separated by real whitespace.
        assert_eq!(kinds("hello \u{4E16}\u{754C} ok"), vec![
            TokenKind::Word,
            TokenKind::Whitespace,
            TokenKind::Word,
            TokenKind::Whitespace,
            TokenKind::Word
        ]);
        // Japanese mixed kana/kanji joins too (all alphabetic).
        assert_eq!(kinds("\u{65E5}\u{672C}\u{3054}\u{3068}"), vec![TokenKind::Word]);
    }

    #[test]
    fn unicode_non_letters_are_whole_tokens() {
        // An em-dash is one 3-byte Punct token; NBSP is Whitespace.
        let toks = lex("a\u{2014}b".as_bytes());
        let ks: Vec<TokenKind> = toks.iter().map(|t| t.kind).collect();
        assert_eq!(ks, vec![TokenKind::Word, TokenKind::Punct, TokenKind::Word]);
        assert_eq!(toks[1].end - toks[1].start, 3, "em-dash is one whole token");
        assert_eq!(kinds("a\u{A0}b"), vec![
            TokenKind::Word,
            TokenKind::Whitespace,
            TokenKind::Word
        ]);
        // An invalid sequence (a lone continuation byte) still advances.
        assert_eq!(lex(&[0x80, b'a']).len(), 2);
    }

    #[test]
    fn classifies_core_kinds() {
        let k = kinds("ab 12 3.5 \"x\" ,");
        assert_eq!(
            k,
            vec![
                TokenKind::Word,
                TokenKind::Whitespace,
                TokenKind::Number,
                TokenKind::Whitespace,
                TokenKind::Number,
                TokenKind::Whitespace,
                TokenKind::Quoted,
                TokenKind::Whitespace,
                TokenKind::Punct,
            ]
        );
    }

    #[test]
    fn pairs_nested_brackets() {
        // f(g(x))
        let toks = lex(b"f(g(x))");
        // indices: 0 f, 1 (, 2 g, 3 (, 4 x, 5 ), 6 )
        assert_eq!(toks[1].kind, TokenKind::Open(BracketKind::Paren));
        assert_eq!(toks[1].mate(), Some(6));
        assert_eq!(toks[3].mate(), Some(5));
        assert_eq!(toks[5].mate(), Some(3));
        assert_eq!(toks[6].mate(), Some(1));
    }

    #[test]
    fn mismatched_close_is_unpaired() {
        // a) has a close with no open.
        let toks = lex(b"a)");
        assert_eq!(toks[1].kind, TokenKind::Close(BracketKind::Paren));
        assert_eq!(toks[1].mate(), None);
    }

    #[test]
    fn char_literal_with_inner_quote_does_not_swallow() {
        // `'"'` is one char literal; the inner `"` must not open a string that
        // eats the following `x`. Tokens: '"' , x.
        let toks: Vec<_> = lex(b"'\"' x").into_iter().filter(Token::is_significant).collect();
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].kind, TokenKind::Quoted);
        assert_eq!(toks[1].kind, TokenKind::Word);
    }

    #[test]
    fn rust_lifetime_is_not_a_char_literal() {
        // `'static` has no closing quote in the window, so the apostrophe stays
        // punctuation and `static` is its own word - lifetimes are preserved.
        let toks: Vec<_> = lex(b"&'static T").into_iter().filter(Token::is_significant).collect();
        assert!(toks.iter().any(|t| t.kind == TokenKind::Word && text(b"&'static T", t) == b"static"));
        assert!(toks.iter().all(|t| t.kind != TokenKind::Quoted));
    }

    #[test]
    fn a_single_quoted_string_is_one_token_where_its_quotes_open_and_close() {
        // The quote opens a string where no word byte precedes it and a
        // quote on the same line that no word byte follows closes it: what
        // a Dart, JavaScript, Python or shell string looks like, and what a
        // Rust lifetime, a contraction, a decade and a foot mark do not.
        // The Quoted tokens opening with a single quote; a double-quoted
        // string holding single quotes, or opened by an unbalanced double
        // quote, is not one of them.
        let quoted = |input: &str| {
            lex(input.as_bytes())
                .iter()
                .filter(|t| t.kind == TokenKind::Quoted && input.as_bytes()[t.start()] == b'\'')
                .count()
        };
        assert_eq!(quoted("'hello world'"), 1);
        assert_eq!(quoted("x = 'ab \"b\" c' ;"), 1);
        assert_eq!(quoted("\"a 'b' c\""), 0);
        assert_eq!(quoted("'it\\'s'"), 1);
        // A quote beside another opens nothing: SQL's doubled quote leaves
        // one string, wiki markup and an empty string leave none.
        assert_eq!(quoted("'foo''bar'"), 1);
        assert_eq!(quoted("''αρχηγός αποθήκης'', δείχνει ''μέχρι το τέλος''."), 0);
        assert_eq!(quoted("x = '' ;"), 0);
        assert_eq!(quoted("'\\u{10000}\\u{10001}\\u{10002}'"), 1);
        assert_eq!(kinds("'\\u{10000}\\u{10001}'"), vec![TokenKind::Quoted]);
        assert_eq!(quoted("rock 'n' roll"), 1);
        // Afrikaans writes its article as 'n, which opens nothing: the first
        // quote after it is followed by a word byte, or there is none.
        assert_eq!(quoted("sê hy 'ja' en 'n ander"), 1);
        assert_eq!(quoted("'n 'massiewe aanval' teen"), 1);
        assert_eq!(lex(b"'n 'massiewe aanval' teen").iter().filter(|t| t.kind == TokenKind::Quoted).map(|t| t.start()).collect::<Vec<_>>(), vec![3]);
        for open in [
            "&'a str) -> &'a str", "impl<'a, 'b>", "where 'a: 'b", "&'static T", "don't stop, it's late",
            "the '90s and '80s", "5'10 tall", "'abc\ndef'", "r'raw'", "'n dag 'n beer met 'n karakter",
            "'Hello, world,' he said, 'it's late'", "'n harde \"doo-doo-du-du''.", "'s morgens en 't is 'k",
            "Γι' αυτό θα παραμείνει θρύλος στ' όνομά του", "'I was there,' she said",
        ] {
            let strings = usize::from(open.starts_with("'Hello") || open.starts_with("'I "));
            assert_eq!(quoted(open), strings, "{open}");
        }
        // The word after a closed string is its own token.
        let toks: Vec<_> = lex(b"'ab c' d").into_iter().filter(Token::is_significant).collect();
        assert_eq!(toks.iter().map(|t| t.kind).collect::<Vec<_>>(), vec![TokenKind::Quoted, TokenKind::Word]);
    }

    #[test]
    fn escaped_char_literal_is_one_token() {
        // `'\n'` and `'\''` are single Quoted tokens.
        let toks: Vec<_> = lex(b"'\\n'").into_iter().filter(Token::is_significant).collect();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Quoted);
    }

    #[test]
    fn quoted_handles_escaped_quote() {
        let toks = lex(b"\"a\\\"b\"");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Quoted);
        assert_eq!(toks[0].end, 6);
    }

    #[test]
    fn a_double_quoted_string_closes_on_its_own_line_or_is_no_string() {
        let quoted = |input: &str| -> Vec<String> {
            lex(input.as_bytes())
                .iter()
                .filter(|t| t.kind == TokenKind::Quoted)
                .map(|t| input[t.start()..t.end()].to_string())
                .collect()
        };
        // Closed on its line, an escaped quote inside it.
        assert_eq!(quoted("x = \"a \\\" b\" ;"), vec!["\"a \\\" b\""]);
        // A quote its line does not close opens nothing, and the quotes of the
        // lines after it are read afresh rather than as its close.
        assert_eq!(quoted("it's 5\" long\nthe \"real\" one\n"), vec!["\"real\""]);
        assert_eq!(quoted("say \"no close\nx = \"y\" ;"), vec!["\"y\""]);
        // A backslash before the newline carries the string over it.
        assert_eq!(quoted("\"a\\\nb\" c"), vec!["\"a\\\nb\""]);
        // An escaped quote leaves the string waiting on a close its line does
        // not hold, and the input's end closes nothing either.
        assert!(quoted("\"a\\\"\nb").is_empty());
        assert!(quoted("tail \"never closed").is_empty());
        // The words a quote left open would have taken into a string are words.
        let input = "say \"no close\nnext";
        let words: Vec<&str> =
            lex(input.as_bytes()).iter().filter(|t| t.kind == TokenKind::Word).map(|t| &input[t.start()..t.end()]).collect();
        assert_eq!(words, vec!["say", "no", "close", "next"]);
    }

    /// The double-quote rule read off the positions gives the walk's answer at
    /// every double quote of two real source files and of the constructs that
    /// decide it: an escaped quote, an escaped backslash before a quote, runs of
    /// backslashes of both parities, a newline before the close with a
    /// backslash and without, a backslash before a carriage return, a quote
    /// right after the opening one, and no close at all. The positions are
    /// asked in the order a scan asks them, one reader an input, then from the
    /// last quote back to the first, so every ask behind the block last read
    /// is made too.
    #[test]
    fn the_double_quote_rule_read_off_the_positions_is_the_rule_walked() {
        let lexer_src: &[u8] = include_bytes!("lexer.rs");
        let scan_src: &[u8] = include_bytes!("parallel_lex.rs");
        let edges: [&[u8]; 11] = [
            b"\"a\\\"b\" x",
            b"\"a\\\\\" x",
            b"\"a\nb\"",
            b"\"a\\\nb\" c",
            b"\"a\\\r\nb\"",
            b"\"\\\\\\\"\\\\\"",
            b"\"\"",
            b"\"",
            b"\"never closed",
            b"\"a\\",
            b"x \" y \" z \"\n\"",
        ];
        let mut read = 0usize;
        for input in [lexer_src, scan_src].into_iter().chain(edges) {
            let quotes: Vec<usize> = (0..input.len()).filter(|&i| input[i] == b'"').collect();
            let mut ascending = crate::byte_simd::CloseOrNewlinePositions::new(input);
            let mut descending = crate::byte_simd::CloseOrNewlinePositions::new(input);
            for (&up, &down) in quotes.iter().zip(quotes.iter().rev()) {
                for (i, closes) in [(up, &mut ascending), (down, &mut descending)] {
                    let context = String::from_utf8_lossy(&input[i.saturating_sub(24)..(i + 40).min(input.len())]);
                    assert_eq!(double_quoted_end_at(input, i, closes), double_quoted_end(input, i), "at {i}: {context:?}");
                }
                read += 1;
            }
        }
        assert!(read > 1000, "the two sources hold the quotes this is about: {read}");
    }

    /// A backslash ending a line inside a quoted string continues the string
    /// onto the next line whether the line ends in LF or CRLF, so the numbers
    /// written inside it are the string's either way and only those outside
    /// are numbers.
    #[test]
    fn a_string_continued_across_a_crlf_reads_as_across_an_lf() {
        let lf = "let a = 1;\nlet s = \"see \\\n 10/3 here\";\nlet t = 'ab \\\n 7';\nlet b = 2;\n";
        let crlf = lf.replace('\n', "\r\n");
        let pattern = crate::parse("\\N").expect("the pattern parses");
        for text in [lf, crlf.as_str()] {
            let found: Vec<&str> =
                crate::scan(&pattern, text.as_bytes()).iter().map(|s| &text[s.start as usize..s.end as usize]).collect();
            assert_eq!(found, ["1", "2"], "{text:?}");
        }
    }

    #[test]
    fn timestamps_in_every_form_are_one_token() {
        for ts in [
            "2026-09-01T12:00:00Z",
            "2026-09-01T12:00:00.123456+02:00",
            "2026-09-01T12:00:00-0700",
            "2026-09-01t12:00:00+05",
            "2026-09-01 12:00:00",
            "2026-09-01 12:00",
            "Sep 14 18:50:53",
            "Sep  5 03:04:05",
            "dec 31 23:59:59.5",
            "10/Oct/2000:13:55:36 -0700",
            "10/Oct/2000:13:55:36",
            "12:30:45.5",
            "12:30:45Z",
        ] {
            assert_eq!(kinds(ts), vec![TokenKind::Timestamp], "{ts}");
        }
        // A range of times is two times: a zone runs into nothing.
        let two_times = vec![TokenKind::Timestamp, TokenKind::Punct, TokenKind::Timestamp];
        assert_eq!(kinds("12:30:45-13:00:00"), two_times);
        assert_eq!(kinds("12:30-13:00"), two_times);
        // A zone that runs into a word is not a zone.
        assert_eq!(kinds("2026-09-01T12:00:00Zulu"), vec![TokenKind::Timestamp, TokenKind::Word]);
        // The space-joined zone is Apache's alone; after a bare time a signed
        // number is a number.
        assert_eq!(
            kinds("12:30:45 -0700"),
            vec![TokenKind::Timestamp, TokenKind::Whitespace, TokenKind::Punct, TokenKind::Number]
        );
        // A month and a day with no clock after them, or a day no month
        // has, stay a word and a number.
        assert_eq!(
            kinds("Sep 14 apples"),
            vec![
                TokenKind::Word,
                TokenKind::Whitespace,
                TokenKind::Number,
                TokenKind::Whitespace,
                TokenKind::Word
            ]
        );
        assert_eq!(kinds("Sep 99 10:00:00")[0], TokenKind::Word);
        assert_eq!(kinds("10/Xyz/2000:13:55:36")[0], TokenKind::Number);
        // A date and a number after it stay apart; only a clock joins.
        assert_eq!(
            kinds("2026-09-01 12345"),
            vec![TokenKind::Timestamp, TokenKind::Whitespace, TokenKind::Number]
        );
    }

    #[test]
    fn a_slash_date_is_one_timestamp_in_every_order_the_calendar_holds() {
        for ts in [
            "2026/09/15",
            "2026/09/15 10:11:12",
            "2026/09/15T10:11:12Z",
            "09/15/2026",
            "15/09/2026",
            "3/4/2026",
            "15/09/2026 10:11",
        ] {
            assert_eq!(kinds(ts), vec![TokenKind::Timestamp], "{ts}");
        }
        // The year's four digits are what tell a date from a run of figures,
        // and no reading of the leading fields is a date the calendar holds.
        let figures = vec![
            TokenKind::Number,
            TokenKind::Punct,
            TokenKind::Number,
            TokenKind::Punct,
            TokenKind::Number,
        ];
        assert_eq!(kinds("1/2/3"), figures);
        assert_eq!(kinds("13/13/2026"), figures);
        assert_eq!(kinds("2026/13/01"), figures);
        // Which of the two leading fields is the day never moves the extent,
        // so the span is the same under either order.
        assert_eq!(kinds("03/04/2026"), vec![TokenKind::Timestamp]);
    }

    #[test]
    fn classifies_typed_spans() {
        assert_eq!(kinds("1.2.3.4"), vec![TokenKind::Ip]);
        assert_eq!(kinds("fe80::1"), vec![TokenKind::Ip]);
        assert_eq!(kinds("a@b.com"), vec![TokenKind::Email]);
        assert_eq!(kinds("http://x.com/p"), vec![TokenKind::Url]);
        assert_eq!(kinds("2026-06-16"), vec![TokenKind::Timestamp]);
        assert_eq!(kinds("12:30:45"), vec![TokenKind::Timestamp]);
        assert_eq!(kinds("2026-06-16T12:30:45"), vec![TokenKind::Timestamp]);
    }

    #[test]
    fn typed_recognizers_leave_plain_numbers_alone() {
        assert_eq!(kinds("12.34"), vec![TokenKind::Number]);
        assert_eq!(kinds("2026"), vec![TokenKind::Number]);
        // An over-range octet is not an IP.
        assert!(!kinds("999.1.1.1").contains(&TokenKind::Ip));
    }

    #[test]
    fn classifies_new_typed_atoms() {
        assert_eq!(kinds("550e8400-e29b-41d4-a716-446655440000"), vec![TokenKind::Uuid]);
        assert_eq!(kinds("1.2.3"), vec![TokenKind::Version]);
        assert_eq!(kinds("v1.2.3"), vec![TokenKind::Version]);
        assert_eq!(kinds("1.0.0-alpha.1"), vec![TokenKind::Version]);
        assert_eq!(kinds("01:23:45:67:89:ab"), vec![TokenKind::Mac]);
        assert_eq!(kinds("aa-bb-cc-dd-ee-ff"), vec![TokenKind::Mac]);
        assert_eq!(kinds("#ff8800"), vec![TokenKind::HexColor]);
        assert_eq!(kinds("#f80"), vec![TokenKind::HexColor]);
        assert_eq!(kinds("192.168.0.0/24"), vec![TokenKind::Cidr]);
    }

    #[test]
    fn classifies_unit_typed_atoms() {
        assert_eq!(kinds("42%"), vec![TokenKind::Percent]);
        assert_eq!(kinds("3.14%"), vec![TokenKind::Percent]);
        assert_eq!(kinds("512KB"), vec![TokenKind::ByteSize]);
        assert_eq!(kinds("1.5GiB"), vec![TokenKind::ByteSize]);
        assert_eq!(kinds("2TB"), vec![TokenKind::ByteSize]);
        assert_eq!(kinds("10B"), vec![TokenKind::ByteSize]);
        assert_eq!(kinds("$5"), vec![TokenKind::Money]);
        assert_eq!(kinds("$1,234.56"), vec![TokenKind::Money]);
    }

    #[test]
    fn unit_typed_recognizers_reject_near_misses() {
        // A bare magnitude with no `B` is a number plus a word, not a size.
        assert_eq!(kinds("10M"), vec![TokenKind::Number, TokenKind::Word]);
        // A plain number is neither a percent nor a size.
        assert_eq!(kinds("50"), vec![TokenKind::Number]);
        // `$` before a non-digit is punctuation, not money.
        assert!(!kinds("$word").contains(&TokenKind::Money));
    }

    #[test]
    fn classifies_durations() {
        assert_eq!(kinds("1500ms"), vec![TokenKind::Duration]);
        assert_eq!(kinds("2.5s"), vec![TokenKind::Duration]);
        assert_eq!(kinds("3h20m"), vec![TokenKind::Duration]);
        assert_eq!(kinds("90s"), vec![TokenKind::Duration]);
        // A non-time unit letter is a number plus a word, not a duration.
        assert_eq!(kinds("5x"), vec![TokenKind::Number, TokenKind::Word]);
        assert_eq!(kinds("50"), vec![TokenKind::Number]);
    }

    #[test]
    fn classifies_quantities() {
        assert_eq!(kinds("5kg"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("5 kg"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("20\u{b0}C"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("-40\u{b0}C"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("3.2 GHz"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("40 %"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("5\u{a0}kg"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("\u{2212}5 dB"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("5in"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("5A"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("5 m/s"), vec![TokenKind::Quantity]);
        assert_eq!(kinds("5 m\u{b2}"), vec![TokenKind::Quantity]);
        // The attached forms `\Z`, `\R` and `\%` read keep their kinds.
        assert_eq!(kinds("10MB"), vec![TokenKind::ByteSize]);
        assert_eq!(kinds("5m"), vec![TokenKind::Duration]);
        assert_eq!(kinds("40%"), vec![TokenKind::Percent]);
        // A single letter, an English word and the kelvin after a space are
        // left to the context, and a symbol run into more letters is a word.
        let apart = vec![TokenKind::Number, TokenKind::Whitespace, TokenKind::Word];
        assert_eq!(kinds("5 m"), apart);
        assert_eq!(kinds("3 in"), apart);
        assert_eq!(kinds("5 items"), apart);
        assert_eq!(kinds("5  kg"), apart);
        assert_eq!(kinds("5K"), vec![TokenKind::Number, TokenKind::Word]);
        assert_eq!(kinds("5kgs"), vec![TokenKind::Number, TokenKind::Word]);
        // A dash after a digit is a range dash and a sign after a letter is
        // no sign; after a bracket it is the quantity's own.
        assert_eq!(kinds("10-20kg"), vec![TokenKind::Number, TokenKind::Punct, TokenKind::Quantity]);
        assert_eq!(kinds("x-5kg"), vec![TokenKind::Word, TokenKind::Punct, TokenKind::Quantity]);
        assert_eq!(
            kinds("(-5kg)"),
            vec![
                TokenKind::Open(BracketKind::Paren),
                TokenKind::Quantity,
                TokenKind::Close(BracketKind::Paren)
            ]
        );
    }

    #[test]
    fn classifies_paths() {
        assert_eq!(kinds("/usr/bin/x"), vec![TokenKind::Path]);
        assert_eq!(kinds("./rel/f.txt"), vec![TokenKind::Path]);
        assert_eq!(kinds("../up/one"), vec![TokenKind::Path]);
        assert_eq!(kinds("C:\\dir\\file"), vec![TokenKind::Path]);
        // `a/b` is division / a bare pair, not a path (the `/` is glued to a
        // word), so it does not read as a single Path token.
        assert!(!kinds("a/b").contains(&TokenKind::Path));
    }

    #[test]
    fn classifies_hash_digests() {
        assert_eq!(kinds("d41d8cd98f00b204e9800998ecf8427e"), vec![TokenKind::HashDigest]); // md5
        assert_eq!(
            kinds("da39a3ee5e6b4b0d3255bfef95601890afd80709"),
            vec![TokenKind::HashDigest]
        ); // sha1
        // Too short is a word; an all-decimal 32-run has no hex letter so it
        // stays a number, not a hash.
        assert!(!kinds("deadbeef").contains(&TokenKind::HashDigest));
        assert!(!kinds("12345678901234567890123456789012").contains(&TokenKind::HashDigest));
    }

    #[test]
    fn new_typed_recognizers_reject_near_misses() {
        // Eight hex with no dashes is a word, not a UUID.
        assert!(!kinds("deadbeef").contains(&TokenKind::Uuid));
        // A fourth dotted number is an IPv4 address, not a version.
        assert_eq!(kinds("1.2.3.4"), vec![TokenKind::Ip]);
        assert!(!kinds("1.2.3.4").contains(&TokenKind::Version));
        // The word `version` is not a version string.
        assert!(!kinds("version").contains(&TokenKind::Version));
        // An IPv6 address is not a MAC.
        assert_eq!(kinds("fe80::1"), vec![TokenKind::Ip]);
        assert!(!kinds("fe80::1").contains(&TokenKind::Mac));
        // Wrong hex-digit count is not a color.
        assert!(!kinds("#12345").contains(&TokenKind::HexColor));
        // A plain IP is not a CIDR, and an over-32 prefix is not a CIDR.
        assert!(!kinds("10.0.0.1").contains(&TokenKind::Cidr));
        assert!(!kinds("1.2.3.4/99").contains(&TokenKind::Cidr));
        // A clock time keeps its timestamp class (not stolen by MAC).
        assert_eq!(kinds("12:30:45"), vec![TokenKind::Timestamp]);
    }

    /// The significant stream pairs a bracket exactly where the token stream
    /// does, read over one chunk so every pair that exists is one the chunk
    /// closed itself.
    ///
    /// The two streams are indexed differently - one holds whitespace and the
    /// other drops it - so the token stream's mates are carried across that
    /// difference before they are compared, which is also what catches a mate
    /// slot pushed for a token the stream dropped.
    #[test]
    fn the_paired_sink_pairs_what_the_token_stream_pairs() {
        for src in [
            &b""[..],
            b"()",
            b"(a)",
            b"a (b) c",
            b"f(x) g(yy) h(zzz)",
            b"{ [ ( ) ] }",
            b"( ( ) ( ) )",
            b"a ( b [ c ] d ) e",
            // Unpaired on both sides, and a mismatched close, none of which
            // the token stream pairs either.
            b"(",
            b")",
            b"( ]",
            b"a ( b",
            b"a ) b",
            b"if (cond) { do(x) ; }",
        ] {
            let toks = lex(src);
            let mut paired = PairedSignificant::with_base(0, 0);
            lex_chunk_paired_into(src, &blob_runs(src), &mut paired, &mut Seams::default());

            // Where each token sits in the significant stream, and which token
            // each significant slot came from.
            let mut sig_of = vec![usize::MAX; toks.len()];
            let mut tok_of: Vec<usize> = Vec::new();
            for (i, t) in toks.iter().enumerate() {
                if t.kind != TokenKind::Whitespace {
                    sig_of[i] = tok_of.len();
                    tok_of.push(i);
                }
            }

            assert_eq!(
                paired.mates.len(),
                tok_of.len(),
                "one mate slot per significant token in {:?}",
                String::from_utf8_lossy(src)
            );
            assert_eq!(
                paired.parts.kinds.len(),
                tok_of.len(),
                "the mates keep step with the kinds in {:?}",
                String::from_utf8_lossy(src)
            );
            for (s, &t) in tok_of.iter().enumerate() {
                let want = toks[t].mate().map(|m| sig_of[m]);
                let got =
                    (paired.mates[s] != NO_MATE).then(|| paired.mates[s] as usize);
                assert_eq!(
                    got,
                    want,
                    "token {t} (significant {s}) in {:?}",
                    String::from_utf8_lossy(src)
                );
            }
        }
    }
}
