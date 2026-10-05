//! A pattern that is a fixed sequence of token kinds - `\W`, `\N`, `\W \N` -
//! answered over the lexer's chunk parts in place.
//!
//! Such a pattern matches wherever that many consecutive significant tokens
//! carry those kinds, leftmost and non-overlapping, which is a window over
//! the kind-code stream and needs no engine. The parts the parallel lexer
//! writes for the device path already hold that stream - kind codes and
//! absolute byte spans, whitespace dropped - one part per chunk, so the scan
//! reads them where they were lexed: no token stream is stitched and no
//! index of the significant tokens is built.

use crate::ast::{Atom, Pattern};
use crate::engine::Span;
use crate::lexer::Significant;
use crate::token::{BracketKind, TokenKind};

/// The kind codes of `pattern` when it is one token kind or a concatenation
/// of them, none whitespace, which the significant stream drops, and none a
/// custom shape, which the default lex does not make.
#[must_use]
pub fn kind_sequence(pattern: &Pattern) -> Option<Vec<u32>> {
    let atoms: &[Pattern] = match pattern {
        Pattern::Concat(v) => v,
        one => std::slice::from_ref(one),
    };
    let mut codes = Vec::with_capacity(atoms.len());
    for atom in atoms {
        // `\W{2}` is the window `\W \W` written shorter: a repeat whose bounds
        // meet takes a fixed count of one kind and offers no other length, so it
        // expands here rather than reaching the engine. An open or unequal bound
        // does not, having a choice of lengths that a fixed window cannot carry.
        let (kind, times) = match atom {
            Pattern::Atom(Atom::Kind(kind)) => (kind, 1usize),
            Pattern::Repeat(inner, lo, Some(hi), _) if lo == hi && *lo > 0 => {
                let Pattern::Atom(Atom::Kind(kind)) = inner.as_ref() else {
                    return None;
                };
                (kind, *lo)
            }
            _ => return None,
        };
        if matches!(kind, TokenKind::Whitespace | TokenKind::Custom(_)) {
            return None;
        }
        for _ in 0..times {
            codes.push(kind.code());
        }
    }
    (!codes.is_empty()).then_some(codes)
}

/// The token kinds a match can open with, one bit a kind code.
///
/// A set rather than one kind because a balanced group opens on any of three
/// brackets where its own kind is not fixed, and reading only one of them would
/// refuse the other two.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OpeningKinds(u64);

/// The bit standing for "a token of any declared kind opens here", rather than
/// for one kind's code.
///
/// A built-in kind codes below 33 and a declared one from 33 up, so the top bit
/// carries no built-in kind and every declared kind shares this one. It is
/// reachable only through [`OpeningKinds::with_custom`]: [`OpeningKinds::just`]
/// refuses a custom kind outright and [`OpeningKinds::admits`] answers for one
/// before it takes a code, so no declared kind can set or read this bit as
/// though it were its own.
const CUSTOM: u64 = 1 << 63;

impl OpeningKinds {
    /// The set holding `kind` alone, or `None` for a kind no bit can carry.
    ///
    /// A declared kind has no bit of its own, and a set that admits one says so
    /// through [`Self::with_custom`].
    #[must_use]
    pub fn just(kind: TokenKind) -> Option<OpeningKinds> {
        if matches!(kind, TokenKind::Custom(_)) {
            return None;
        }
        (kind.code() < 64).then(|| OpeningKinds(1 << kind.code()))
    }

    /// Both sets.
    #[must_use]
    pub fn with(self, other: OpeningKinds) -> OpeningKinds {
        OpeningKinds(self.0 | other.0)
    }

    /// This set, and a token of any declared kind besides.
    ///
    /// For an atom that tests a token's text rather than its kind. A declared
    /// shape decides its own token boundaries, so a token of a custom kind can
    /// carry the same bytes as the atom wants and does match it; refusing that
    /// anchor would lose the match.
    #[must_use]
    pub fn with_custom(self) -> OpeningKinds {
        OpeningKinds(self.0 | CUSTOM)
    }

    /// Whether a token of `kind` could open a match.
    #[must_use]
    pub fn admits(self, kind: TokenKind) -> bool {
        // Answered before a code is taken, because a declared kind's code is no
        // bit position here: the set carries one bit for all of them.
        if matches!(kind, TokenKind::Custom(_)) {
            return self.0 & CUSTOM != 0;
        }
        let code = kind.code();
        code < 64 && self.0 & (1u64 << code) != 0
    }
}

/// The token kinds a match of `pattern` can begin with, where the pattern forces
/// them, or `None` where it does not.
///
/// For a caller that holds an anchor's token and wants to know whether a match
/// could begin there at all - the sweep tries an attempt at every significant
/// token, and one whose kind the pattern cannot open with dies inside `advance`
/// after the work of entering it.
///
/// Conservative by construction, and it has to be: an anchor refused here is an
/// anchor never tried, so a wrong answer is a missing match rather than a slow
/// scan. Every shape that can begin on a token other than the one it appears to
/// open with answers `None`, and `None` admits every anchor.
///
/// What it reads through, and why each is safe:
///
///   a concatenation   its elements in order, past any number of leading nodes
///                     that consume none, which are never the token an anchor
///                     is on. An element that must take a token is where
///                     the match opens and ends the walk. One that may take
///                     nothing leaves the opening to whatever follows, so its
///                     kinds join the next element's and the walk goes on -
///                     `(?>\W*) "="` opens on a word or on the `=`, and over
///                     the crate's own source that refuses 173,285 anchors of
///                     875,524 where declining the whole sequence refused none.
///   a binding         the pattern it wraps, which it does not change.
///   a repeat          its inner pattern. Taking a token at all means taking
///                     the inner pattern at least once, so the bounds do not
///                     change what it opens with; whether the repeat may take
///                     nothing is a separate question, asked by the
///                     concatenation above and by `takes_no_tokens` for the
///                     pattern as a whole. `P*`, `P?` and `P+` read the same way.
///   an atomic group   the group it wraps, whose first token is its own: the
///                     group discards the lengths its body did not prefer and
///                     none of that reaches the token it began at.
///   an alternation    every branch, unioned, and only where every one of them
///                     answers. A branch that declines is a branch that could
///                     open on anything, so one is enough to decline the whole -
///                     and the mode does not matter, because first-match picks
///                     among the same branches an ordinary alternation offers.
///   a balanced group  the open bracket it must start with - one kind where the
///                     group names one, all three where it does not.
///   a literal         the kind its own bytes lex to, where they lex to exactly
///                     one token, under the identity orbit alone. The atom
///                     compares a token's text and reads no kind, so the set
///                     also admits every declared kind: a shape chooses its own
///                     boundaries and can carry those same bytes.
///
/// What it does not read through, and why:
///
///   an edit group     `(A B C)~k` aligns its atoms against the tokens by a
///                     Levenshtein walk whose diagonal step is a match or a
///                     substitution, so at `k >= 1` the first atom can be
///                     substituted and any token at all opens the run. Measured
///                     over the crate's own source, `("let" \W "=")~1` opens on
///                     ten kinds - number, word, quoted, punct and all six
///                     brackets - and an opening-kind route could refuse 398 of
///                     873,589 anchors, 0.0%. A set built from the leading
///                     atoms would have answered `word` and lost the rest.
#[must_use]
pub fn first_kinds(pattern: &Pattern) -> Option<OpeningKinds> {
    // A pattern that can match no tokens matches at every anchor, so it forces
    // no opening kind and every anchor has to be tried. Asked once here rather
    // than at each node, because it is a question about the whole pattern and
    // the walk below asks a different one.
    if pattern.takes_no_tokens() {
        return None;
    }
    kinds_when_consuming(pattern)
}

/// The kinds `pattern` can begin with on a match that takes at least one token,
/// or `None` where it does not force them.
///
/// Distinct from [`first_kinds`], which answers for the pattern as a whole. A
/// star forces nothing on its own, since it matches nothing anywhere, but a star
/// that does take a token takes its inner pattern's first - and that is what a
/// concatenation needs to know about an element it may have to look past.
fn kinds_when_consuming(pattern: &Pattern) -> Option<OpeningKinds> {
    match pattern {
        Pattern::Concat(v) => {
            let mut set: Option<OpeningKinds> = None;
            for p in v {
                // A node that consumes nothing under any match is never the
                // token an anchor is on, so it is stepped over.
                if p.max_tokens() == Some(0) {
                    continue;
                }
                let here = kinds_when_consuming(p)?;
                set = Some(set.map_or(here, |s| s.with(here)));
                // An element that must take a token is where the match opens,
                // and nothing after it can carry the anchor. One that may take
                // nothing leaves the opening to whatever follows, so its kinds
                // join theirs and the walk goes on.
                if !p.takes_no_tokens() {
                    return set;
                }
            }
            // Every element could take nothing, so the run can match nothing and
            // opens anywhere. `first_kinds` refuses that case before it reaches
            // here; a nested one reaches it and declines.
            None
        }
        Pattern::Bind(_, _, inner) | Pattern::Atomic(inner) => kinds_when_consuming(inner),
        // Taking a token at all means taking the inner pattern at least once,
        // whatever the bounds allow, so the bounds do not matter here.
        Pattern::Repeat(inner, _, _, _)
        | Pattern::Plus(inner, _)
        | Pattern::Star(inner, _)
        | Pattern::Opt(inner, _) => kinds_when_consuming(inner),
        Pattern::Alt(branches, _) => {
            let mut rest = branches.iter();
            let first = kinds_when_consuming(rest.next()?)?;
            rest.try_fold(first, |set, branch| Some(set.with(kinds_when_consuming(branch)?)))
        }
        Pattern::Balanced(which, _) => match which {
            Some(bracket) => OpeningKinds::just(TokenKind::Open(*bracket)),
            None => [BracketKind::Paren, BracketKind::Square, BracketKind::Brace]
                .into_iter()
                .try_fold(OpeningKinds(0), |set, bracket| {
                    Some(set.with(OpeningKinds::just(TokenKind::Open(bracket))?))
                }),
        },
        // Whitespace is not offered an attempt and a custom shape is not made by
        // the default lex, so neither names an anchor this could refuse.
        Pattern::Atom(Atom::Kind(kind))
            if !matches!(kind, TokenKind::Whitespace | TokenKind::Custom(_)) =>
        {
            OpeningKinds::just(*kind)
        }
        // A literal under any other orbit matches a token whose canonical form
        // agrees with its own, which is not required to carry the same kind -
        // the notation orbit reads a glyph and its name as one - so only the
        // identity orbit, a byte compare, fixes the kind.
        Pattern::Atom(Atom::Literal(lit, crate::orbit::OrbitGroup::Identity)) => {
            literal_kind(lit).and_then(OpeningKinds::just).map(OpeningKinds::with_custom)
        }
        _ => None,
    }
}

/// The kind a token whose text is exactly `lit` carries under the default lex,
/// or `None` where those bytes are not one token.
///
/// Bytes that lex to several tokens name no single kind, and a literal that
/// covers only part of what it lexes says nothing about a token equal to the
/// whole of it.
fn literal_kind(lit: &str) -> Option<TokenKind> {
    let toks = crate::lexer::lex(lit.as_bytes());
    let mut significant = toks.iter().filter(|t| t.is_significant());
    let only = significant.next()?;
    if significant.next().is_some() || only.start() != 0 || only.end() != lit.len() {
        return None;
    }
    Some(only.kind)
}

/// The leftmost, non-overlapping matches of the kind sequence `codes` over
/// `parts`, the significant stream in input order, as byte spans.
#[must_use]
pub fn scan_kind_sequence(codes: &[u32], parts: &[Significant]) -> Vec<Span> {
    let mut out = Vec::new();
    if codes.is_empty() {
        return out;
    }
    // `(p, i)` is the next position to try. A window is read forward from it
    // across part boundaries; the scan resumes past a match, or one token on.
    let (mut p, mut i) = (0usize, 0usize);
    loop {
        while p < parts.len() && i >= parts[p].kinds.len() {
            p += 1;
            i = 0;
        }
        if p >= parts.len() {
            break;
        }
        let (mut q, mut j) = (p, i);
        let mut matched = true;
        for &code in codes {
            while q < parts.len() && j >= parts[q].kinds.len() {
                q += 1;
                j = 0;
            }
            if q >= parts.len() || parts[q].kinds[j] != code {
                matched = false;
                break;
            }
            j += 1;
        }
        if matched {
            // The window's last token is the one before `(q, j)`, in part `q`.
            out.push(Span { start: parts[p].spans[i].0, end: parts[q].spans[j - 1].1 });
            p = q;
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// The kind code and bounds of a pattern that is one bounded repeat of a token
/// kind whose bounds differ - `\W{2,4}` - with how many the repeat takes.
///
/// A repeat whose bounds meet is a fixed window and [`kind_sequence`] expands
/// it. One whose bounds differ is a run: it takes as many consecutive tokens of
/// that kind as it can up to the upper bound, or as few as the lower bound
/// where it is lazy, and a fixed window cannot carry that choice. An open upper
/// bound is refused, since a run to the end of the stream is what the engine's
/// own sweep is for.
#[must_use]
pub fn kind_run(pattern: &Pattern) -> Option<(u32, usize, usize)> {
    let Pattern::Repeat(inner, lo, Some(hi), greed) = pattern else {
        return None;
    };
    let (lo, hi) = (*lo, *hi);
    if lo == 0 || lo >= hi {
        return None;
    }
    let Pattern::Atom(Atom::Kind(kind)) = inner.as_ref() else {
        return None;
    };
    if matches!(kind, TokenKind::Whitespace | TokenKind::Custom(_)) {
        return None;
    }
    // A lazy repeat takes the fewest it may, which is its lower bound and so a
    // fixed window after all; it is left to the sequence route above.
    matches!(greed, crate::ast::Greed::Greedy).then(|| (kind.code(), lo, hi))
}

/// The leftmost, non-overlapping matches of a run of between `lo` and `hi`
/// consecutive tokens of kind `code` over `parts`, as byte spans.
///
/// Greedy: a run takes as many as it can up to `hi`, and the scan resumes past
/// what it took. A run shorter than `lo` is no match and the scan steps one
/// token on, which is what the engine does with it.
#[must_use]
pub fn scan_kind_run(code: u32, lo: usize, hi: usize, parts: &[Significant]) -> Vec<Span> {
    let mut out = Vec::new();
    let (mut p, mut i) = (0usize, 0usize);
    loop {
        while p < parts.len() && i >= parts[p].kinds.len() {
            p += 1;
            i = 0;
        }
        if p >= parts.len() {
            break;
        }
        // How many of this kind follow, counted across part boundaries and
        // never past the upper bound.
        let (mut q, mut j, mut took) = (p, i, 0usize);
        while took < hi {
            while q < parts.len() && j >= parts[q].kinds.len() {
                q += 1;
                j = 0;
            }
            if q >= parts.len() || parts[q].kinds[j] != code {
                break;
            }
            j += 1;
            took += 1;
        }
        if took >= lo {
            // The run's last token is the one before `(q, j)`, in part `q`.
            let (mut e, mut k) = (q, j);
            while k == 0 {
                e -= 1;
                k = parts[e].kinds.len();
            }
            out.push(Span { start: parts[p].spans[i].0, end: parts[e].spans[k - 1].1 });
            p = q;
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// The bracket kind a balanced group accepts, or `None` for any kind, when its
/// interior is unconstrained.
///
/// Bare `\B` parses to a balanced group over `.*`: any bracket kind, any
/// interior. Such a group asks only where the brackets pair, and the lexer has
/// already decided that. One whose interior constrains anything needs an engine
/// that can test it, and this reports nothing for it.
#[must_use]
pub fn bare_balanced(pattern: &Pattern) -> Option<Option<crate::token::BracketKind>> {
    let Pattern::Balanced(kind, inner) = pattern else {
        return None;
    };
    let Pattern::Star(body, _) = inner.as_ref() else {
        return None;
    };
    matches!(body.as_ref(), Pattern::Atom(Atom::Any)).then_some(*kind)
}

/// Every balanced pair of `kind` in `toks`, leftmost and non-overlapping, from
/// the mates the lexer recorded.
///
/// The lexer pairs each bracket as it reads and writes the partner on both
/// tokens, so the answer exists the moment the lex ends and this is one pass
/// over it: an open bracket beginning at or after the last match's end opens
/// the next match, and one inside that match is the nesting the selection
/// skips. The set-reachability engine, which is what this shape reaches
/// otherwise, read 44.6203 ms over 7.34 MB where the lex alone is 2.9.
///
/// An unpaired bracket has no mate and is no match: the lexer leaves an
/// unclosed open and a stray close unpaired, which is what a scan reports of
/// them too.
#[must_use]
pub fn scan_balanced(
    kind: Option<crate::token::BracketKind>,
    toks: &[crate::token::Token],
) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut from = 0usize;
    for t in toks {
        let TokenKind::Open(k) = t.kind else { continue };
        if kind.is_some_and(|want| want != k) {
            continue;
        }
        let Some(m) = t.mate() else { continue };
        let start = t.start();
        if start < from {
            continue;
        }
        let end = toks[m].end();
        out.push(Span { start: start as u32, end: end as u32 });
        from = end;
    }
    out
}

/// [`scan_balanced`] over the lexer's parts, which carry mates of their own, so
/// the route answers where the chunks were lexed instead of over a token stream
/// stitched into one array first.
///
/// It reads the four things the token form reads - an open's kind, its mate,
/// its start, and the mate's end - and nothing else of a token, which is what
/// lets the parts stand in for one.
#[must_use]
pub fn scan_balanced_parts(
    kind: Option<crate::token::BracketKind>,
    parts: &[(crate::lexer::PairedSignificant, crate::lexer::Seams)],
) -> Vec<Span> {
    use crate::token::BracketKind;
    // The codes an open of the wanted bracket carries, read once. Asking which
    // bracket a code stands for walks the bracket block, and a scan asking it
    // per token would walk it over the whole stream to answer what one integer
    // compare answers here.
    let mut codes = [0u32; 3];
    let wanted = match kind {
        Some(k) => {
            codes = [TokenKind::Open(k).code(); 3];
            &codes[..1]
        }
        None => {
            for (slot, k) in codes
                .iter_mut()
                .zip([BracketKind::Paren, BracketKind::Square, BracketKind::Brace])
            {
                *slot = TokenKind::Open(k).code();
            }
            &codes[..]
        }
    };
    // The lowest and highest of them, so one comparison rejects every token
    // that is no open bracket at all - which is nearly every token - and the
    // exact test runs only for the few it lets through. Read off the codes
    // rather than assumed contiguous, so it stays a necessary condition
    // whatever the encoding does.
    let (lo, hi) = wanted.iter().fold((u32::MAX, 0), |(lo, hi), &c| (lo.min(c), hi.max(c)));

    // Where each part's run begins in the whole stream, with the total past the
    // end, so a mate resolves to the part holding it.
    let mut bases: Vec<u32> = Vec::with_capacity(parts.len() + 1);
    let mut acc = 0u32;
    for (part, _) in parts {
        bases.push(acc);
        acc += u32::try_from(part.mates.len()).expect("a token index within the stored width");
    }
    bases.push(acc);
    let end_of = |at: u32| -> u32 {
        let ci = bases.partition_point(|&b| b <= at) - 1;
        parts[ci].0.parts.spans[(at - bases[ci]) as usize].1
    };

    let _walking = crate::trace::phase("the balanced pass over the parts");
    let mut out: Vec<Span> = Vec::new();
    let mut from = 0u32;
    for (ci, (part, _)) in parts.iter().enumerate() {
        let base = bases[ci];
        let past = bases[ci + 1];
        for (i, &code) in part.parts.kinds.iter().enumerate() {
            if code < lo || code > hi || !wanted.contains(&code) {
                continue;
            }
            // An unpaired bracket has no mate and is no match, exactly as the
            // token form reports it.
            let mate = part.mates[i];
            if mate == crate::lexer::NO_MATE {
                continue;
            }
            let start = part.parts.spans[i].0;
            if start < from {
                continue;
            }
            // A bracket closes in the chunk that opened it unless it is one of
            // the few the seam walk paired, so the mate is looked for here
            // before it is looked for anywhere.
            let end = if mate >= base && mate < past {
                part.parts.spans[(mate - base) as usize].1
            } else {
                end_of(mate)
            };
            out.push(Span { start, end });
            from = end;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(src: &str, input: &[u8]) -> Vec<Span> {
        let p = crate::parse(src).expect("pattern parses");
        crate::nfa::scan_nfa(&p, input).expect("the single-pass engine takes these patterns")
    }

    fn routed(src: &str, input: &[u8]) -> Vec<Span> {
        let p = crate::parse(src).expect("pattern parses");
        let codes = kind_sequence(&p).expect("a kind sequence");
        crate::parallel_lex::lex_significant_parts_held(input, |parts| scan_kind_sequence(&codes, parts))
    }

    fn opens(src: &str) -> Option<OpeningKinds> {
        first_kinds(&crate::parse(src).expect("pattern parses"))
    }

    fn just(kind: TokenKind) -> Option<OpeningKinds> {
        OpeningKinds::just(kind)
    }

    /// The opening kind is read only where the pattern forces it, and declining
    /// is the answer everywhere else.
    ///
    /// An anchor this refuses is an anchor the sweep never tries, so the cost of
    /// reading a kind out of a pattern that does not force one is a match that
    /// is never found. A pattern that can match no tokens at all is the case
    /// that must decline: it matches at every anchor, so no kind is forced.
    #[test]
    fn an_opening_kind_is_read_only_where_the_pattern_forces_one() {
        assert_eq!(opens("\\W \\B"), just(TokenKind::Word), "a kind leads it");
        assert_eq!(opens("\\N \"=\""), just(TokenKind::Number), "and so here");
        assert_eq!(opens("\\W"), just(TokenKind::Word), "one atom is its own first");
        assert_eq!(opens("\\W:a \\B"), just(TokenKind::Word), "a binding is transparent");
        assert_eq!(opens("\\W{2} \\B"), just(TokenKind::Word), "a repeat that must take one");
        assert_eq!(opens("\\W+ \\B"), just(TokenKind::Word), "a plus always takes one");
        assert_eq!(opens("\\W*"), None, "a star alone matches nothing anywhere");
        assert_eq!(opens("\\W{0,3}"), None, "and so does a repeat from zero");
        assert_eq!(opens("(?>\\W*)"), None, "a group around one is the same case");
    }

    /// A literal opens on the kind its own bytes lex to, and on a declared kind
    /// besides.
    ///
    /// The atom compares a token's text and reads no kind at all, so the kinds
    /// it can match are the kinds a token carrying those bytes can have. Under
    /// the default lex that is one kind. Under a shape set it is also whatever
    /// shape claimed those bytes, and refusing that anchor would lose a match
    /// `scan_with_shapes` does find - which a pattern naming a library kind
    /// reaches without declaring anything, since those join the set themselves.
    ///
    /// A kind atom is the contrasting case and is asserted here beside it: that
    /// one tests the kind, so a declared kind is rightly refused.
    #[test]
    fn a_literal_opens_on_the_kind_its_bytes_lex_to() {
        let word = opens("\"let\" \\W \"=\"").expect("a literal leads it");
        assert!(word.admits(TokenKind::Word), "`let` lexes to a word");
        assert!(!word.admits(TokenKind::Number), "and to no other built-in kind");
        assert!(word.admits(TokenKind::Custom(0)), "a declared shape can carry those bytes");
        assert!(word.admits(TokenKind::Custom(200)), "whatever id it was given");

        let number = opens("\"200\" \\W").expect("a number literal leads it");
        assert!(number.admits(TokenKind::Number), "`200` lexes to a number");
        assert!(!number.admits(TokenKind::Word), "and not to a word");

        assert_eq!(opens("\"x=\" \\W"), None, "bytes that lex to two tokens name no kind");
        assert_eq!(opens("(?orbit:case \"Cat\") \\W"), None, "another orbit does not fix a kind");

        let kind_led = opens("\\W \\B").expect("a kind leads it");
        assert!(!kind_led.admits(TokenKind::Custom(0)), "a kind atom tests the kind itself");
    }

    /// The route admits the kind of every token that actually opens a match.
    ///
    /// The route reads the kind a literal's bytes lex to on their own, and the
    /// lexer's classification can turn on what surrounds a span: the blob gate
    /// reads entropy over the whole input, so a span read one way alone can be
    /// read another way in company. A literal whose kind moved between the two
    /// would refuse a real anchor, so the check is made against the matches
    /// rather than against the reading that produced them.
    ///
    /// The literals are the ones whose classification has the most room to
    /// move - a base64 run, a hex digest - beside a plain word and a number.
    #[test]
    fn the_route_admits_every_kind_that_opens_a_match() {
        let inputs: &[&[u8]] = &[
            b"let x = 1",
            b"a let = 2 let = 3",
            b"200 = x",
            b"aGVsbG8gd29ybGQhIQ== = x",
            b"deadbeefdeadbeefdeadbeefdeadbeef = x",
            b"tail aGVsbG8gd29ybGQhIQ== = x head",
        ];
        let sources = [
            "\"let\" \\W \"=\"",
            "\"200\" \"=\"",
            "\"aGVsbG8gd29ybGQhIQ==\" \"=\"",
            "\"deadbeefdeadbeefdeadbeefdeadbeef\" \"=\"",
        ];
        for src in sources {
            let pattern = crate::parse(src).expect("pattern parses");
            match first_kinds(&pattern) {
                // A route that declines admits every anchor, so it refuses none.
                None => {}
                Some(set) => {
                    for input in inputs {
                        let toks = crate::lexer::lex(input);
                        for span in crate::scan(&pattern, input) {
                            let opener = toks
                                .iter()
                                .find(|t| t.start() == span.start())
                                .expect("a match begins at a token");
                            assert!(
                                set.admits(opener.kind),
                                "{src} opens on {} over {:?}, and the route refuses it",
                                opener.kind.name(),
                                String::from_utf8_lossy(input)
                            );
                        }
                    }
                }
            }
        }
    }

    /// A declared shape that claims a literal's bytes still opens a match there.
    ///
    /// The route reads the kind those bytes lex to under the default lex, and a
    /// shape set can give the same bytes a kind of its own. The anchor has to
    /// survive that: `atom_test` compares a literal against the token's text and
    /// never against its kind, so the match is real and refusing its anchor
    /// would lose it. A pattern naming a library kind reaches this without
    /// declaring anything, since `scan_with_shapes` adds those to the set.
    ///
    /// This is the case the corpora cannot check - they declare no shapes - so
    /// it is asserted here rather than read off a table.
    #[test]
    fn a_declared_shape_carrying_a_literals_bytes_still_matches() {
        use crate::custom::{Precedence, ShapeSet};
        let pattern = crate::parse("\"let\" \\W \"=\"").expect("pattern parses");
        let input = b"let x = 1";

        let plain = crate::engine::scan_with_shapes(&pattern, input, &ShapeSet::new());
        assert_eq!(plain.len(), 1, "the default lex reads `let` as a word and matches");

        let mut shapes = ShapeSet::new();
        shapes.declare("keyword = `let`", Precedence::Before).expect("declares");
        let claimed = crate::engine::scan_with_shapes(&pattern, input, &shapes);
        assert_eq!(claimed, plain, "a shape claiming the bytes does not lose the match");
    }

    /// An alternation opens on any of its branches, and declines where one of
    /// them could open on anything.
    ///
    /// The union is the point: reading one branch would refuse every anchor the
    /// others begin at. The declining case is the guard on it - a branch that
    /// answers `None` is a branch whose first token is not forced, so no union
    /// over the rest describes what the pattern can start with.
    #[test]
    fn an_alternation_opens_on_every_branch_or_on_none() {
        for src in ["(\\N | \\W) \"=\"", "(\\N |> \\W) \"=\""] {
            let set = opens(src).unwrap_or_else(|| panic!("both branches force a kind: {src}"));
            assert!(set.admits(TokenKind::Number), "the number branch: {src}");
            assert!(set.admits(TokenKind::Word), "the word branch: {src}");
            assert!(!set.admits(TokenKind::Punct), "and nothing else: {src}");
        }
        // A branch that may take nothing does not decline the alternation: the
        // match opens on that branch or, where it takes nothing, on what
        // follows the alternation, so all three kinds join.
        let optional = opens("(\\N | \\W*) \"=\"").expect("the run must take a token");
        assert!(optional.admits(TokenKind::Number), "the number branch");
        assert!(optional.admits(TokenKind::Word), "the star branch when it takes one");
        assert!(optional.admits(TokenKind::Punct), "and the `=` when it takes none");
        let mixed = opens("(\\N | \"let\") \"=\"").expect("a literal branch forces a kind too");
        assert!(mixed.admits(TokenKind::Number), "the kind branch");
        assert!(mixed.admits(TokenKind::Word), "the literal branch, which lexes to a word");
        assert!(mixed.admits(TokenKind::Custom(0)), "and the literal's declared kinds");
        assert!(!mixed.admits(TokenKind::Punct), "and nothing else");
    }

    /// A leading node that takes no token is read through, and the kind is the
    /// first element's that can take one.
    ///
    /// A node matching no token is between tokens, so it is never the token
    /// an anchor is on. The first token a match takes is the first
    /// consuming element's, and that is the kind the anchor has to carry.
    /// Reading through one is what lets an anchored shape name a kind at all,
    /// because the anchor is written first and consumes nothing.
    #[test]
    fn a_leading_node_that_takes_no_token_is_read_through() {
        assert_eq!(opens("~\"lit\" \\W"), just(TokenKind::Word), "a guard is zero width");
        assert_eq!(opens("~(\\W) \\N"), just(TokenKind::Number), "so is an assertion");
        assert_eq!(opens("!~(\\W) \\N"), just(TokenKind::Number), "and its negation");
        assert_eq!(opens("@seam \\W"), just(TokenKind::Word), "and an axis anchor");
        assert_eq!(opens("@nested>0 \\N"), just(TokenKind::Number), "and a stress anchor");
    }

    /// A leading node that may take nothing joins its kinds to what follows.
    ///
    /// This declined until the union was measured, on the reading that no
    /// single set of kinds describes a node that may take nothing. A union
    /// does describe it, and exactly: a concatenation's match begins either
    /// with that element or, where it takes nothing, with whatever follows,
    /// and there is no third case. So the two sets together are complete
    /// rather than approximate, and refuse only anchors that are neither.
    ///
    /// The shape that made it worth doing is `(?>\W*) "="`, which over the
    /// crate's own source refuses 173,285 anchors of 875,524 under a route
    /// naming `word` and `punct`, and refused none while this declined. The
    /// kinds asserted here are the ones measured to open a match there.
    #[test]
    fn a_leading_node_that_may_take_nothing_joins_what_follows() {
        for (src, second) in [
            ("\\W{0,3} \"=\"", TokenKind::Punct),
            ("(?>\\W*) \"=\"", TokenKind::Punct),
            ("\\W* \\N", TokenKind::Number),
            ("\\W? \\N", TokenKind::Number),
        ] {
            let set = opens(src).unwrap_or_else(|| panic!("the run must take a token: {src}"));
            assert!(set.admits(TokenKind::Word), "the node itself can open it: {src}");
            assert!(set.admits(second), "and so can what follows, when it takes none: {src}");
            assert!(!set.admits(TokenKind::Quoted), "and nothing else does: {src}");
        }

        // A run whose every element may take nothing can match nothing, so it
        // opens at every anchor and forces no kind at all.
        assert_eq!(opens("\\W* \\N*"), None, "every element may take nothing");
        assert_eq!(opens("\\W*"), None, "and a star alone is that case");
    }

    /// A balanced group opens on a bracket, and on all three where it names no
    /// kind of its own.
    ///
    /// This is the shape the set exists for: reading one bracket where three
    /// are possible would refuse the other two, and every match beginning on
    /// them would be lost.
    #[test]
    fn a_balanced_group_opens_on_the_brackets_it_admits() {
        let set = opens("\\B").expect("a bare balanced group forces a bracket");
        for bracket in [BracketKind::Paren, BracketKind::Square, BracketKind::Brace] {
            assert!(set.admits(TokenKind::Open(bracket)), "{bracket:?} opens a bare group");
        }
        assert!(!set.admits(TokenKind::Word), "a word does not");
        assert!(!set.admits(TokenKind::Close(BracketKind::Paren)), "nor a close");
    }

    #[test]
    fn a_sequence_of_kinds_is_routable_and_nothing_else_is() {
        for src in ["\\W", "\\N", "\\W \\N", "\\N \\N \\W", "\\Q"] {
            let p = crate::parse(src).expect("pattern parses");
            assert!(kind_sequence(&p).is_some(), "{src} is a kind sequence");
        }
        // A repeat whose bounds meet is a fixed count of one kind and expands
        // to the window it is short for.
        for (src, want) in [("\\W{2}", 2usize), ("\\N{3}", 3), ("\\W{2} \\N", 3), ("\\W{1}", 1)] {
            let p = crate::parse(src).expect("pattern parses");
            let codes = kind_sequence(&p).unwrap_or_else(|| panic!("{src} is a kind sequence"));
            assert_eq!(codes.len(), want, "{src} spans {want} tokens");
        }
        // A literal, an alternation and a bind are not a fixed sequence of
        // kinds, and neither is a repeat that has a choice of lengths: an open
        // upper bound, unequal bounds, or a count of none.
        for src in
            ["\"alpha\"", "\\W \"=\"", "\\W*", "(\\W | \\N)", "\\W:n", "\\W{2,}", "\\W{2,4}"]
        {
            let p = crate::parse(src).expect("pattern parses");
            assert_eq!(kind_sequence(&p), None, "{src} must not route");
        }
    }

    #[test]
    fn the_kind_route_answers_what_the_engine_answers() {
        let inputs: &[&[u8]] = &[
            b"tag 1 tag 2 tag 3",
            b"1 2 3 tag tag 4 5 tag",
            b"alpha (beta) 42 \"q\" 7 8 9 x",
            b"",
            b"   ",
            b"tag\n1\ntag\n2",
            b"x",
        ];
        // The written-out windows, then the repeats they are short for, which
        // must give the same spans as the engine and as each other.
        for src in [
            "\\W", "\\N", "\\W \\N", "\\N \\W", "\\N \\N", "\\W \\W \\N", "\\Q", "\\W{2}",
            "\\N{2}", "\\W{3}", "\\W{2} \\N",
        ] {
            for input in inputs {
                assert_eq!(routed(src, input), engine(src, input), "{src} on {input:?}");
            }
        }
        for (long, short) in [("\\N \\N", "\\N{2}"), ("\\W \\W \\N", "\\W{2} \\N")] {
            for input in inputs {
                assert_eq!(routed(long, input), routed(short, input), "{long} against {short}");
            }
        }
    }

    /// A run of a kind reports what the engine reports, at every pair of bounds
    /// and over runs shorter than the lower bound, exactly as long as it, and
    /// longer than the upper bound - which is where a greedy run and a fixed
    /// window differ.
    #[test]
    fn a_kind_run_reports_what_the_engine_reports() {
        let mut corpus = String::new();
        for i in 0..300u32 {
            // Runs of one, two, three, four and six words, and numbers between
            // them so a run ends where the kind changes rather than at a line.
            corpus.push_str(&format!("a{i} {i}\n"));
            corpus.push_str(&format!("b{i} c{i} {i}\n"));
            corpus.push_str(&format!("d{i} e{i} f{i} {i}\n"));
            corpus.push_str(&format!("g{i} h{i} i{i} j{i} {i}\n"));
            corpus.push_str(&format!("k{i} l{i} m{i} n{i} o{i} p{i} {i}\n"));
        }
        let input = corpus.as_bytes();
        for src in ["\\W{2,4}", "\\W{1,2}", "\\W{3,6}", "\\W{2,3}", "\\N{1,2}"] {
            let p = crate::parse(src).expect("pattern parses");
            let (code, lo, hi) = kind_run(&p).expect("this shape is a kind run");
            let got = crate::parallel_lex::lex_significant_parts_held(input, |parts| {
                scan_kind_run(code, lo, hi, parts)
            });
            assert_eq!(got, engine(src, input), "{src}");
        }
        // Shapes this route is not for: bounds that meet are a fixed window the
        // sequence route expands, an open bound is a sweep, and a lazy repeat
        // takes its lower bound and so is a window too.
        for src in ["\\W{2}", "\\W{2,}", "\\W{2,4}?"] {
            let p = crate::parse(src).expect("pattern parses");
            assert!(kind_run(&p).is_none(), "{src} is not a greedy bounded run");
        }
    }

    /// The balanced route reports what the set engine reports, over the cases
    /// that separate reading the lexer's mates from matching brackets naively:
    /// nesting, adjacency, an unclosed open, a stray close, a mismatched pair,
    /// all three kinds, and brackets inside a string, which the lexer does not
    /// pair because they are not tokens.
    #[test]
    fn the_balanced_route_reports_what_the_set_engine_reports() {
        let mut corpus = String::new();
        for i in 0..300u32 {
            corpus.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n"));
            corpus.push_str(&format!("call_{i}(alpha, [beta, {i}], gamma) ;\n"));
            corpus.push_str(&format!("bare_{i} = \"a ( b ) c\" ;\n"));
        }
        corpus.push_str("unclosed ( a b\nstray ) c\nmixed ( a ] b\n");
        let input = corpus.as_bytes();
        let toks = crate::lexer::lex(input);
        for src in ["\\B", "\\B(.*)", "\\B[.*]", "\\B{.*}"] {
            let p = crate::parse(src).expect("pattern parses");
            let kind = bare_balanced(&p).expect("this shape is a bare balanced group");
            let got = scan_balanced(kind, &toks);
            let want = crate::engine::scan_set_reachability(&p, input);
            assert_eq!(got, want, "{src}");
        }
        // A constrained interior is not this route's shape and must be refused,
        // or a group the engine has to test would be answered without testing.
        for src in ["\\B(\\W)", "\\B(\\N .*)", "\\B[\\W]"] {
            let p = crate::parse(src).expect("pattern parses");
            assert!(bare_balanced(&p).is_none(), "{src} constrains its interior");
        }
    }

    #[test]
    fn a_window_crossing_a_chunk_boundary_is_read_across_it() {
        // Above the parallel threshold the lex splits at newlines, so a
        // number ending one chunk and the word opening the next are one
        // window of `\N \W`.
        let mut input = String::new();
        for i in 0..200_000u32 {
            input.push_str("tag ");
            input.push_str(&(i % 1000).to_string());
            input.push('\n');
        }
        let bytes = input.as_bytes();
        assert!(bytes.len() > crate::parallel_lex::active_parallel_threshold());
        for src in ["\\N \\W", "\\W \\N", "\\N", "\\W \\N \\W"] {
            assert_eq!(routed(src, bytes), engine(src, bytes), "{src}");
        }
    }

    /// The balanced route over the parts answers what it answers over the
    /// stitched tokens, at every bracket kind and over an input long enough to
    /// be split - so the groups that span a chunk boundary are in it.
    #[test]
    fn the_balanced_route_over_parts_answers_what_it_answers_over_tokens() {
        use crate::token::BracketKind;
        let mut src = String::from("( {\n");
        for i in 0..20_000 {
            src.push_str(&format!("f_{i} [ a_{i} ] b_{i} ( c_{i} )\n"));
        }
        src.push_str("} )\n");
        let input = src.as_bytes();
        let toks = crate::parallel_lex::lex_parallel(input);
        for kind in [
            None,
            Some(BracketKind::Paren),
            Some(BracketKind::Square),
            Some(BracketKind::Brace),
        ] {
            let want = scan_balanced(kind, &toks);
            let got = crate::parallel_lex::lex_paired_parts_held(input, |parts| {
                scan_balanced_parts(kind, parts)
            });
            assert_eq!(got, want, "bracket kind {kind:?}");
        }
        // A short input takes the one-part path, which pairs everything inside
        // that part and joins nothing.
        for short in [&b""[..], b"()", b"a (b [c] d) e", b"( ]", b"a ) b ( c"] {
            let toks = crate::parallel_lex::lex_parallel(short);
            let want = scan_balanced(None, &toks);
            let got = crate::parallel_lex::lex_paired_parts_held(short, |parts| {
                scan_balanced_parts(None, parts)
            });
            assert_eq!(got, want, "{:?}", String::from_utf8_lossy(short));
        }
    }
}
