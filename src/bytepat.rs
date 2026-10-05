//! Byte-level sub-patterns: the low grain that runs inside a single
//! token.
//!
//! A byte-pattern matches a token by its raw bytes, whole-anchored, so
//! a token-level structural pattern can also constrain a token's byte
//! shape. This is where trex does what a regular expression does
//! (character classes, quantifiers over bytes) at the same time as the
//! surrounding pattern does what a regular expression cannot
//! (structure, binding, balance). The two grains compose in one
//! pattern.
//!
//! Matching is set-of-positions reachability, the same non-backtracking
//! shape the token engine uses, so a byte-pattern never backtracks.
//! Tokens are short, so the bounded cost is irrelevant.
//!
//! ## Bytes or characters, and which construct is which
//!
//! Some constructs here read bytes and some read characters. The split is
//! deliberate, because the two answer different questions: the lexer is UTF-8
//! aware at token grain, so a Greek or Cyrillic word is one word token, and
//! what a byte-pattern adds is a claim about the characters inside a token.
//!
//! - `.`, `[...]` and `\p{...}` match one character, and advance by however
//!   many bytes it occupies. A class range is over codepoints, so `[a-z]` and
//!   an alpha-to-omega range are the same construct.
//! - `\d`, `\w`, `\s` and their negations stay ASCII, and match one byte.
//!   This is the one place the reading is narrower than a regular expression
//!   would give: Rust's `regex` makes `\w` Unicode-aware by default. It is
//!   kept ASCII because the lexer's own recognizers are written against these
//!   and widening `\w` would silently change what lexes as what - a change to
//!   tokenization, not to matching. `\p{L}` is the Unicode-aware spelling.
//! - A literal byte is still a byte, and `\` before punctuation is that
//!   punctuation. `\` before any letter other than `d D w W s S p P`, `(?` at
//!   the start of a group and `\` inside a class are parse errors, so no
//!   pattern parses as something other than what it spells.
//!
//! Malformed bytes match nothing rather than panicking or matching a lone
//! byte, so a pattern can never half-match into the middle of a character.
//!
//! [`BytePat::max_len`] widens accordingly: a class or `.` reports four
//! rather than one, because that is what a character can cost. A caller
//! sizing a window from it reads slightly more than it needs, which is the
//! safe direction.
//!
//! The categories `\p{...}` accepts are those the standard library can decide
//! exactly. An unknown name is a parse error rather than a class that matches
//! nothing, because a silent empty class reads as "no such character in the
//! input" and would be indistinguishable from a working pattern.

/// A byte-level pattern node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BytePat {
    /// The empty pattern.
    Empty,
    /// A literal byte.
    Byte(u8),
    /// Any single character (`.`).
    Any,
    /// A character class `[...]` or negated `[^...]`: a list of inclusive
    /// codepoint ranges plus a negation flag.
    ///
    /// Ranges are over characters rather than bytes, so `[a-z]` and
    /// `[alpha-omega]` are the same construct. A byte range could not express
    /// the second: above `0x7f` the individual bytes of a character carry no
    /// order a range could read.
    Class(Vec<(char, char)>, bool),
    /// `\p{...}` and its negation `\P{...}`: a Unicode general category.
    Property(UnicodeClass, bool),
    /// `\d`, `\w`, `\s` and their negations `\D`, `\W`, `\S`.
    Builtin(ByteClassKind),
    /// A sequence.
    Concat(Vec<BytePat>),
    /// Unordered alternation (standard regular-expression `|`).
    Alt(Vec<BytePat>),
    /// `P*`.
    Star(Box<BytePat>),
    /// `P+`.
    Plus(Box<BytePat>),
    /// `P?`.
    Opt(Box<BytePat>),
    /// `P{m,n}`; `n` is `None` for `P{m,}`.
    Repeat(Box<BytePat>, usize, Option<usize>),
}

/// A Unicode general category a `\p{...}` class can name.
///
/// The set is what the standard library answers exactly, and no more. A
/// category needs a table to decide, `char` already carries the tables for
/// these, and offering one it cannot decide would mean shipping a partial
/// answer that looks total - an unknown name is refused at parse time
/// instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnicodeClass {
    /// `\p{L}`, any letter.
    Letter,
    /// `\p{N}`, any numeric character.
    Number,
    /// `\p{Alnum}`, letter or number.
    Alphanumeric,
    /// `\p{Lu}`, an uppercase letter.
    Uppercase,
    /// `\p{Ll}`, a lowercase letter.
    Lowercase,
    /// `\p{White_Space}`.
    Whitespace,
    /// `\p{C}`, a control character.
    Control,
}

impl UnicodeClass {
    /// The category named, or `None` when it is not one this can decide.
    #[must_use]
    pub fn parse(name: &str) -> Option<UnicodeClass> {
        Some(match name {
            "L" | "Letter" | "Alphabetic" => UnicodeClass::Letter,
            "N" | "Number" | "Numeric" => UnicodeClass::Number,
            "Alnum" | "Alphanumeric" => UnicodeClass::Alphanumeric,
            "Lu" | "Uppercase" => UnicodeClass::Uppercase,
            "Ll" | "Lowercase" => UnicodeClass::Lowercase,
            "White_Space" | "Whitespace" | "Z" => UnicodeClass::Whitespace,
            "C" | "Control" => UnicodeClass::Control,
            _ => return None,
        })
    }

    /// The names accepted, for a diagnostic.
    pub(crate) const NAMES: &'static str =
        "L, N, Alnum, Lu, Ll, White_Space, C (with long aliases)";

    #[must_use]
    fn holds(self, c: char) -> bool {
        match self {
            UnicodeClass::Letter => c.is_alphabetic(),
            UnicodeClass::Number => c.is_numeric(),
            UnicodeClass::Alphanumeric => c.is_alphanumeric(),
            UnicodeClass::Uppercase => c.is_uppercase(),
            UnicodeClass::Lowercase => c.is_lowercase(),
            UnicodeClass::Whitespace => c.is_whitespace(),
            UnicodeClass::Control => c.is_control(),
        }
    }
}

/// The built-in byte classes and their negations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteClassKind {
    /// `\d`.
    Digit,
    /// `\D`.
    NotDigit,
    /// `\w`.
    Word,
    /// `\W`.
    NotWord,
    /// `\s`.
    Space,
    /// `\S`.
    NotSpace,
}

impl BytePat {
    /// Whether this pattern matches the entire byte slice.
    #[must_use]
    pub fn matches_whole(&self, text: &[u8]) -> bool {
        if text.len() <= BITSET_TEXT_MAX {
            return (reach_bits(self, text, 1) & (1 << text.len())) != 0;
        }
        reach(self, text, vec![0]).contains(&text.len())
    }

    /// The bytes every match of this pattern opens with: its leading literal
    /// bytes, and none when it opens with anything else.
    #[must_use]
    pub fn literal_prefix(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.push_literal_prefix(&mut out);
        out
    }

    /// Appends the leading literal bytes, returning whether the whole of
    /// `self` is literal, so that a sibling after it may go on.
    fn push_literal_prefix(&self, out: &mut Vec<u8>) -> bool {
        match self {
            BytePat::Empty => true,
            BytePat::Byte(b) => {
                out.push(*b);
                true
            }
            BytePat::Concat(v) => v.iter().all(|p| p.push_literal_prefix(out)),
            _ => false,
        }
    }

    /// The longest byte length this pattern can ever match, or `None` when it
    /// is unbounded (`*`, `+`, or an open `{m,}`).
    ///
    /// A bounded length is what lets a recognizer read a fixed window from a
    /// position instead of the whole remaining input, so a lexer using one
    /// stays linear. An unbounded pattern is refused where that matters rather
    /// than silently scanning to the end.
    #[must_use]
    pub fn max_len(&self) -> Option<usize> {
        match self {
            BytePat::Empty => Some(0),
            // A literal byte and the ASCII built-ins are one byte; `.`, a
            // class and a property each match one character, which is up to
            // four. The bound stays honest by widening rather than by
            // treating a character as a byte, so a caller sizing a window
            // from this reads a little more than it needs instead of
            // truncating one.
            BytePat::Byte(_) | BytePat::Builtin(_) => Some(1),
            BytePat::Any | BytePat::Class(..) | BytePat::Property(..) => Some(MAX_CHAR_BYTES),
            BytePat::Concat(parts) => {
                let mut total = 0usize;
                for p in parts {
                    total = total.checked_add(p.max_len()?)?;
                }
                Some(total)
            }
            BytePat::Alt(parts) => {
                let mut mx = 0usize;
                for p in parts {
                    mx = mx.max(p.max_len()?);
                }
                Some(mx)
            }
            BytePat::Opt(inner) => inner.max_len(),
            BytePat::Star(_) | BytePat::Plus(_) | BytePat::Repeat(_, _, None) => None,
            BytePat::Repeat(inner, _, Some(hi)) => inner.max_len()?.checked_mul(*hi),
        }
    }

    /// The longest prefix of `text` this pattern matches, or `None` when it
    /// matches no prefix at all. A zero-length match is reported as `Some(0)`.
    #[must_use]
    pub fn longest_prefix(&self, text: &[u8]) -> Option<usize> {
        if text.len() <= BITSET_TEXT_MAX {
            let ends = reach_bits(self, text, 1);
            return (ends != 0).then(|| (Bits::BITS - 1 - ends.leading_zeros()) as usize);
        }
        reach(self, text, vec![0]).into_iter().max()
    }

    /// Every prefix length of `text` this pattern matches, ascending; empty
    /// when it matches no prefix.
    #[must_use]
    pub fn prefix_ends(&self, text: &[u8]) -> Vec<usize> {
        if text.len() <= BITSET_TEXT_MAX {
            let ends = reach_bits(self, text, 1);
            return (0..=text.len()).filter(|&i| (ends & (1 << i)) != 0).collect();
        }
        let mut ends = reach(self, text, vec![0]);
        ends.sort_unstable();
        ends.dedup();
        ends
    }
}

/// The positions of a byte-pattern whose every leaf tests one ASCII byte, as a
/// machine that reads a token a byte at a time.
///
/// [`BytePat::matches_whole`] walks the pattern's tree for every token it is
/// asked about, which on `cond_[0-9]+` over a ten-byte token measured 47
/// nanoseconds and is 2.3623 ms of the 4.9927 the byte-pattern route spends on
/// a 7.34 MB input. This reads one table entry and one word operation a byte.
///
/// A position is a leaf, and the state is the set of positions that could
/// consume the next byte, held in one word. Two tables decide a step: `accept`
/// says which positions take a given byte, `follow` which positions can come
/// after one. There is no state to construct and none to cap, so no pattern
/// this accepts can explode it.
///
/// [`BytePat::automaton`] refuses everything else, and the caller falls back to
/// the tree walk, which answers every pattern.
pub struct ByteAutomaton {
    /// Which positions accept each byte.
    accept: Box<[u64; 256]>,
    /// Which positions can follow each position.
    follow: Vec<u64>,
    /// The positions a match can open at.
    first: u64,
    /// The positions a match can close at.
    last: u64,
    /// Whether the pattern matches the empty token.
    nullable: bool,
}

/// The most positions one word of state holds. A pattern with more falls back
/// to the tree walk, which has no such limit.
const MAX_POSITIONS: usize = 64;

/// What one position tests.
enum ByteTest {
    /// Exactly this byte.
    Byte(u8),
    /// One of `\d`, `\w`, `\s` or a negation, which are ASCII and one byte.
    Builtin(ByteClassKind),
    /// An inclusive ASCII range of a positive class.
    Ranges(Vec<(u8, u8)>),
}

/// The positions built so far and what can follow each.
struct Positions {
    tests: Vec<ByteTest>,
    follow: Vec<u64>,
}

impl Positions {
    /// Record a leaf, reporting it as a sub-pattern: never nullable, opening
    /// and closing at itself.
    fn leaf(&mut self, test: ByteTest) -> Option<(bool, u64, u64)> {
        if self.tests.len() == MAX_POSITIONS {
            return None;
        }
        let bit = 1u64 << self.tests.len();
        self.tests.push(test);
        self.follow.push(0);
        Some((false, bit, bit))
    }
}

/// The positions of `p`, with `follow` filled in: whether it matches empty,
/// where it can open, and where it can close.
fn positions_of(p: &BytePat, b: &mut Positions) -> Option<(bool, u64, u64)> {
    match p {
        BytePat::Empty => Some((true, 0, 0)),
        // A byte outside ASCII is left to the tree walk. A class and `.` read a
        // character there, and a pattern mixing the two readings is one this
        // has no reason to be trusted about.
        BytePat::Byte(x) if *x < 0x80 => b.leaf(ByteTest::Byte(*x)),
        BytePat::Builtin(k) => b.leaf(ByteTest::Builtin(*k)),
        BytePat::Class(ranges, false) => {
            let mut rs = Vec::with_capacity(ranges.len());
            for &(lo, hi) in ranges {
                if lo > '\u{7f}' || hi > '\u{7f}' {
                    return None;
                }
                rs.push((lo as u8, hi as u8));
            }
            b.leaf(ByteTest::Ranges(rs))
        }
        BytePat::Concat(v) => {
            let (mut nullable, mut first, mut last) = (true, 0u64, 0u64);
            for part in v {
                let (n, f, l) = positions_of(part, b)?;
                for i in set_bits(last) {
                    b.follow[i] |= f;
                }
                if nullable {
                    first |= f;
                }
                last = if n { last | l } else { l };
                nullable &= n;
            }
            Some((nullable, first, last))
        }
        BytePat::Alt(v) => {
            let (mut nullable, mut first, mut last) = (false, 0u64, 0u64);
            for part in v {
                let (n, f, l) = positions_of(part, b)?;
                nullable |= n;
                first |= f;
                last |= l;
            }
            Some((nullable, first, last))
        }
        BytePat::Star(p) => {
            let (_, f, l) = positions_of(p, b)?;
            for i in set_bits(l) {
                b.follow[i] |= f;
            }
            Some((true, f, l))
        }
        BytePat::Plus(p) => {
            let (n, f, l) = positions_of(p, b)?;
            for i in set_bits(l) {
                b.follow[i] |= f;
            }
            Some((n, f, l))
        }
        BytePat::Opt(p) => {
            let (_, f, l) = positions_of(p, b)?;
            Some((true, f, l))
        }
        // A bounded repeat is its copies written out, which is what gives each
        // one its own positions; an open one closes with a star.
        BytePat::Repeat(p, lo, hi) => {
            let mut parts: Vec<BytePat> = Vec::new();
            for _ in 0..*lo {
                parts.push((**p).clone());
            }
            match hi {
                Some(hi) => {
                    for _ in *lo..*hi {
                        parts.push(BytePat::Opt(p.clone()));
                    }
                }
                None => parts.push(BytePat::Star(p.clone())),
            }
            positions_of(&BytePat::Concat(parts), b)
        }
        BytePat::Byte(_) | BytePat::Any | BytePat::Property(..) | BytePat::Class(_, true) => None,
    }
}

/// The indices of the set bits of `mask`, low to high.
fn set_bits(mask: u64) -> impl Iterator<Item = usize> {
    std::iter::successors((mask != 0).then_some(mask), |m| {
        let next = *m & (*m - 1);
        (next != 0).then_some(next)
    })
    .map(|m| m.trailing_zeros() as usize)
}

impl ByteAutomaton {
    /// Whether the pattern matches the whole of `text`.
    ///
    /// Answers what [`BytePat::matches_whole`] answers for every pattern
    /// [`BytePat::automaton`] accepts.
    #[must_use]
    pub fn matches_whole(&self, text: &[u8]) -> bool {
        let Some((&last_byte, head)) = text.split_last() else {
            return self.nullable;
        };
        let mut cur = self.first;
        for &byte in head {
            cur &= self.accept[byte as usize];
            if cur == 0 {
                return false;
            }
            cur = self.step(cur);
        }
        cur & self.accept[last_byte as usize] & self.last != 0
    }

    /// The positions reachable after the ones in `cur` have each consumed a
    /// byte.
    #[inline]
    fn step(&self, cur: u64) -> u64 {
        let (mut m, mut out) = (cur, 0u64);
        while m != 0 {
            out |= self.follow[m.trailing_zeros() as usize];
            m &= m - 1;
        }
        out
    }
}

impl BytePat {
    /// A byte machine for this pattern, or `None` where a leaf reads a
    /// character rather than an ASCII byte, or there are more positions than
    /// one word of state holds.
    ///
    /// `.`, `\p{...}`, a negated class, a class reaching past ASCII and a
    /// literal byte past ASCII are each refused: those read a character, and a
    /// machine stepping bytes would answer a different question from the tree
    /// walk about a token carrying one.
    #[must_use]
    pub fn automaton(&self) -> Option<ByteAutomaton> {
        let mut b = Positions { tests: Vec::new(), follow: Vec::new() };
        let (nullable, first, last) = positions_of(self, &mut b)?;
        let mut accept = Box::new([0u64; 256]);
        for (i, test) in b.tests.iter().enumerate() {
            let bit = 1u64 << i;
            for (byte, slot) in accept.iter_mut().enumerate() {
                let byte = byte as u8;
                let takes = match test {
                    ByteTest::Byte(x) => byte == *x,
                    ByteTest::Builtin(k) => builtin_match(*k, byte),
                    ByteTest::Ranges(rs) => {
                        byte < 0x80 && rs.iter().any(|&(lo, hi)| byte >= lo && byte <= hi)
                    }
                };
                if takes {
                    *slot |= bit;
                }
            }
        }
        Some(ByteAutomaton { accept, follow: b.follow, first, last, nullable })
    }
}

fn builtin_match(kind: ByteClassKind, b: u8) -> bool {
    match kind {
        ByteClassKind::Digit => b.is_ascii_digit(),
        ByteClassKind::NotDigit => !b.is_ascii_digit(),
        ByteClassKind::Word => b == b'_' || b.is_ascii_alphanumeric(),
        ByteClassKind::NotWord => !(b == b'_' || b.is_ascii_alphanumeric()),
        ByteClassKind::Space => b.is_ascii_whitespace(),
        ByteClassKind::NotSpace => !b.is_ascii_whitespace(),
    }
}

fn class_match(c: char, ranges: &[(char, char)], negated: bool) -> bool {
    let inside = ranges.iter().any(|&(lo, hi)| c >= lo && c <= hi);
    inside != negated
}

/// Decode the character starting at `at`, with how many bytes it took.
///
/// `None` where no character starts there: a continuation byte, a lead byte
/// no character uses, a truncated sequence, or a value that is not a
/// character. That is the reading rather than a dropped error - a token
/// carrying malformed bytes fails a character class instead of panicking or
/// matching a lone byte, so a pattern can never half-match its way into the
/// middle of a character.
fn next_char(text: &[u8], at: usize) -> Option<(char, usize)> {
    let lead = *text.get(at)?;
    let (width, mut code) = match lead {
        0x00..=0x7f => return Some((char::from(lead), 1)),
        0xc2..=0xdf => (2usize, u32::from(lead & 0x1f)),
        0xe0..=0xef => (3, u32::from(lead & 0x0f)),
        0xf0..=0xf4 => (4, u32::from(lead & 0x07)),
        _ => return None,
    };
    for k in 1..width {
        let b = *text.get(at + k)?;
        if b & 0xc0 != 0x80 {
            return None;
        }
        code = (code << 6) | u32::from(b & 0x3f);
    }
    char::from_u32(code).map(|c| (c, width))
}

/// The longest a single character can be, in bytes. What a class or `.` costs
/// a caller sizing a window from [`BytePat::max_len`].
const MAX_CHAR_BYTES: usize = 4;

fn dedup(mut positions: Vec<usize>) -> Vec<usize> {
    positions.sort_unstable();
    positions.dedup();
    positions
}

/// The set of end positions reachable by matching `p` from each start
/// position in `starts` over `text`.
fn reach(p: &BytePat, text: &[u8], starts: Vec<usize>) -> Vec<usize> {
    match p {
        BytePat::Empty => dedup(starts),
        BytePat::Byte(b) => one_byte(starts, text, |c| c == *b),
        // `.` and the classes step a character, so they never land inside
        // one; the built-ins stay ASCII and step a byte. See the header.
        BytePat::Any => one_char(starts, text, |_| true),
        BytePat::Class(ranges, neg) => one_char(starts, text, |c| class_match(c, ranges, *neg)),
        BytePat::Property(class, neg) => {
            one_char(starts, text, |c| class.holds(c) != *neg)
        }
        BytePat::Builtin(kind) => one_byte(starts, text, |c| builtin_match(*kind, c)),
        BytePat::Concat(parts) => {
            parts.iter().fold(dedup(starts), |acc, part| reach(part, text, acc))
        }
        BytePat::Alt(parts) => {
            let mut out = Vec::new();
            for part in parts {
                out.extend(reach(part, text, starts.clone()));
            }
            dedup(out)
        }
        BytePat::Opt(inner) => {
            let mut out = starts.clone();
            out.extend(reach(inner, text, starts));
            dedup(out)
        }
        BytePat::Star(inner) => closure(inner, text, starts),
        BytePat::Plus(inner) => {
            let once = reach(inner, text, starts);
            closure(inner, text, once)
        }
        BytePat::Repeat(inner, m, n) => repeat(inner, *m, *n, text, starts),
    }
}

fn one_byte(starts: Vec<usize>, text: &[u8], pred: impl Fn(u8) -> bool) -> Vec<usize> {
    let mut out = Vec::new();
    for s in starts {
        if s < text.len() && pred(text[s]) {
            out.push(s + 1);
        }
    }
    dedup(out)
}

/// Step one character, advancing by however many bytes it occupies.
fn one_char(starts: Vec<usize>, text: &[u8], pred: impl Fn(char) -> bool) -> Vec<usize> {
    let mut out = Vec::new();
    for s in starts {
        if let Some((c, width)) = next_char(text, s)
            && pred(c)
        {
            out.push(s + width);
        }
    }
    dedup(out)
}

fn closure(inner: &BytePat, text: &[u8], starts: Vec<usize>) -> Vec<usize> {
    let mut seen = dedup(starts);
    let mut frontier = seen.clone();
    loop {
        let next = reach(inner, text, frontier);
        let fresh: Vec<usize> = next.into_iter().filter(|q| !seen.contains(q)).collect();
        if fresh.is_empty() {
            break;
        }
        for q in &fresh {
            seen.push(*q);
        }
        frontier = fresh;
    }
    dedup(seen)
}

fn repeat(
    inner: &BytePat,
    m: usize,
    n: Option<usize>,
    text: &[u8],
    starts: Vec<usize>,
) -> Vec<usize> {
    let mut cur = dedup(starts);
    for _ in 0..m {
        cur = reach(inner, text, cur);
        if cur.is_empty() {
            return cur;
        }
    }
    match n {
        None => closure(inner, text, cur),
        Some(nn) => {
            let mut acc = cur.clone();
            let mut frontier = cur;
            for _ in m..nn {
                let next = reach(inner, text, frontier);
                let fresh: Vec<usize> = next.into_iter().filter(|q| !acc.contains(q)).collect();
                if fresh.is_empty() {
                    break;
                }
                for q in &fresh {
                    acc.push(*q);
                }
                frontier = fresh;
            }
            dedup(acc)
        }
    }
}

/// A set of positions in a text of at most [`BITSET_TEXT_MAX`] bytes: bit
/// `i` set means position `i`, from the start to one past the last byte.
type Bits = u128;

/// The longest text whose positions fit [`Bits`]. A token is far shorter
/// but for a blob, which takes the vector form.
const BITSET_TEXT_MAX: usize = (Bits::BITS - 1) as usize;

/// [`reach`] over [`Bits`]: the same end positions, with no vector built
/// at any node. The matcher runs once per token the engine offers a
/// byte-pattern atom, and the vector form's allocation and sort at every
/// node cost more than the match.
fn reach_bits(p: &BytePat, text: &[u8], starts: Bits) -> Bits {
    if starts == 0 {
        return 0;
    }
    match p {
        BytePat::Empty => starts,
        BytePat::Byte(b) => step_byte(starts, text, |c| c == *b),
        BytePat::Any => step_char(starts, text, |_| true),
        BytePat::Class(ranges, neg) => step_char(starts, text, |c| class_match(c, ranges, *neg)),
        BytePat::Property(class, neg) => step_char(starts, text, |c| class.holds(c) != *neg),
        BytePat::Builtin(kind) => step_byte(starts, text, |c| builtin_match(*kind, c)),
        BytePat::Concat(parts) => parts.iter().fold(starts, |acc, part| reach_bits(part, text, acc)),
        BytePat::Alt(parts) => parts.iter().fold(0, |acc, part| acc | reach_bits(part, text, starts)),
        BytePat::Opt(inner) => starts | reach_bits(inner, text, starts),
        BytePat::Star(inner) => closure_bits(inner, text, starts),
        BytePat::Plus(inner) => {
            let once = reach_bits(inner, text, starts);
            closure_bits(inner, text, once)
        }
        BytePat::Repeat(inner, m, n) => {
            let mut cur = starts;
            for _ in 0..*m {
                cur = reach_bits(inner, text, cur);
                if cur == 0 {
                    return 0;
                }
            }
            match n {
                None => closure_bits(inner, text, cur),
                Some(nn) => {
                    let mut acc = cur;
                    let mut frontier = cur;
                    for _ in *m..*nn {
                        let fresh = reach_bits(inner, text, frontier) & !acc;
                        if fresh == 0 {
                            break;
                        }
                        acc |= fresh;
                        frontier = fresh;
                    }
                    acc
                }
            }
        }
    }
}

fn step_byte(starts: Bits, text: &[u8], pred: impl Fn(u8) -> bool) -> Bits {
    let mut out = 0;
    let mut rest = starts;
    while rest != 0 {
        let s = rest.trailing_zeros() as usize;
        rest &= rest - 1;
        if s < text.len() && pred(text[s]) {
            out |= 1 << (s + 1);
        }
    }
    out
}

/// Step one character, advancing by however many bytes it occupies.
fn step_char(starts: Bits, text: &[u8], pred: impl Fn(char) -> bool) -> Bits {
    let mut out = 0;
    let mut rest = starts;
    while rest != 0 {
        let s = rest.trailing_zeros() as usize;
        rest &= rest - 1;
        if let Some((c, width)) = next_char(text, s)
            && pred(c)
        {
            out |= 1 << (s + width);
        }
    }
    out
}

fn closure_bits(inner: &BytePat, text: &[u8], starts: Bits) -> Bits {
    let mut seen = starts;
    let mut frontier = starts;
    loop {
        let fresh = reach_bits(inner, text, frontier) & !seen;
        if fresh == 0 {
            return seen;
        }
        seen |= fresh;
        frontier = fresh;
    }
}

/// Parse a byte-pattern from its source bytes.
///
/// # Errors
///
/// Returns a message describing the first malformed construct.
pub fn parse(src: &[u8]) -> Result<BytePat, String> {
    let mut p = BParser { s: src, i: 0 };
    let pat = p.alt()?;
    if p.i != p.s.len() {
        return Err(format!("unexpected byte at {} in byte-pattern", p.i));
    }
    Ok(pat)
}

struct BParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl BParser<'_> {
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

    fn alt(&mut self) -> Result<BytePat, String> {
        let mut alts = vec![self.concat()?];
        while self.peek() == Some(b'|') {
            self.bump();
            alts.push(self.concat()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap() } else { BytePat::Alt(alts) })
    }

    fn concat(&mut self) -> Result<BytePat, String> {
        let mut items = Vec::new();
        while !matches!(self.peek(), None | Some(b'|') | Some(b')')) {
            items.push(self.postfix()?);
        }
        Ok(match items.len() {
            0 => BytePat::Empty,
            1 => items.pop().unwrap(),
            _ => BytePat::Concat(items),
        })
    }

    fn postfix(&mut self) -> Result<BytePat, String> {
        let atom = self.atom()?;
        match self.peek() {
            Some(b'*') => {
                self.bump();
                Ok(BytePat::Star(Box::new(atom)))
            }
            Some(b'+') => {
                self.bump();
                Ok(BytePat::Plus(Box::new(atom)))
            }
            Some(b'?') => {
                self.bump();
                Ok(BytePat::Opt(Box::new(atom)))
            }
            Some(b'{') => self.repeat(atom),
            _ => Ok(atom),
        }
    }

    fn repeat(&mut self, inner: BytePat) -> Result<BytePat, String> {
        self.bump(); // '{'
        let m = self.number()?;
        let n = if self.peek() == Some(b',') {
            self.bump();
            if self.peek() == Some(b'}') { None } else { Some(self.number()?) }
        } else {
            Some(m)
        };
        if self.bump() != Some(b'}') {
            return Err("expected '}' in byte-pattern quantifier".to_string());
        }
        Ok(BytePat::Repeat(Box::new(inner), m, n))
    }

    fn number(&mut self) -> Result<usize, String> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.bump();
        }
        if self.i == start {
            return Err("expected a number in byte-pattern".to_string());
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| "invalid number in byte-pattern".to_string())
    }

    fn atom(&mut self) -> Result<BytePat, String> {
        match self.peek() {
            Some(b'\\') => {
                self.bump();
                let c = self.bump().ok_or("dangling backslash in byte-pattern")?;
                Ok(match c {
                    b'd' => BytePat::Builtin(ByteClassKind::Digit),
                    b'D' => BytePat::Builtin(ByteClassKind::NotDigit),
                    b'w' => BytePat::Builtin(ByteClassKind::Word),
                    b'W' => BytePat::Builtin(ByteClassKind::NotWord),
                    b's' => BytePat::Builtin(ByteClassKind::Space),
                    b'S' => BytePat::Builtin(ByteClassKind::NotSpace),
                    b'p' | b'P' => return self.property(c == b'P'),
                    other if other.is_ascii_alphabetic() => {
                        return Err(format!(
                            "\\{} is not a byte-pattern escape; the escapes are \\d \\D \\w \\W \\s \\S \\p{{..}} \\P{{..}}, and a backslash before punctuation is that punctuation",
                            other as char
                        ));
                    }
                    other => BytePat::Byte(other),
                })
            }
            Some(b'.') => {
                self.bump();
                Ok(BytePat::Any)
            }
            Some(b'[') => self.class(),
            Some(b'(') => {
                self.bump();
                if self.peek() == Some(b'?') {
                    return Err("`(?` is not byte-pattern syntax: a group takes no flags or extensions".to_string());
                }
                let inner = self.alt()?;
                if self.bump() != Some(b')') {
                    return Err("expected ')' in byte-pattern".to_string());
                }
                Ok(inner)
            }
            Some(c) => {
                self.bump();
                Ok(BytePat::Byte(c))
            }
            None => Err("expected a byte-pattern atom".to_string()),
        }
    }

    /// Parse `{Name}` after `\p` or `\P`.
    fn property(&mut self, negated: bool) -> Result<BytePat, String> {
        if self.bump() != Some(b'{') {
            return Err("expected '{' after \\p in byte-pattern".to_string());
        }
        let start = self.i;
        while matches!(self.peek(), Some(c) if c != b'}') {
            self.bump();
        }
        let raw = &self.s[start..self.i];
        if self.bump() != Some(b'}') {
            return Err("unterminated \\p{...} in byte-pattern".to_string());
        }
        let name = match std::str::from_utf8(raw) {
            Ok(n) => n,
            Err(e) => return Err(format!("\\p{{...}} name is not UTF-8: {e}")),
        };
        match UnicodeClass::parse(name) {
            Some(class) => Ok(BytePat::Property(class, negated)),
            // Refused rather than approximated: a category this cannot decide
            // would otherwise match nothing and read as "no such character".
            None => Err(format!(
                "unknown Unicode class {name:?} in byte-pattern (known: {})",
                UnicodeClass::NAMES
            )),
        }
    }

    /// Take one character of class body, so a class member can be any
    /// character rather than one byte of one.
    fn class_char(&mut self) -> Option<char> {
        let (c, width) = next_char(self.s, self.i)?;
        self.i += width;
        Some(c)
    }

    fn class(&mut self) -> Result<BytePat, String> {
        self.bump(); // '['
        let negated = if self.peek() == Some(b'^') {
            self.bump();
            true
        } else {
            false
        };
        let mut ranges = Vec::new();
        while let Some(c) = self.peek() {
            if c == b']' {
                self.bump();
                return Ok(BytePat::Class(ranges, negated));
            }
            if c == b'\\' {
                return Err("a byte-pattern class takes no backslash; write each character itself".to_string());
            }
            let Some(lo) = self.class_char() else {
                return Err("malformed character in byte-pattern class".to_string());
            };
            // A range `a-z`, unless the '-' is the last character before ']'.
            if self.peek() == Some(b'-') && self.s.get(self.i + 1).is_some_and(|&n| n != b']') {
                self.bump(); // '-'
                let Some(hi) = self.class_char() else {
                    return Err("malformed character in byte-pattern class".to_string());
                };
                ranges.push((lo.min(hi), lo.max(hi)));
            } else {
                ranges.push((lo, lo));
            }
        }
        Err("unterminated character class in byte-pattern".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pat: &str, text: &str) -> bool {
        parse(pat.as_bytes()).unwrap().matches_whole(text.as_bytes())
    }

    #[test]
    fn the_bitset_reach_is_the_vector_reach() {
        // Every construct, over texts that exercise empty matches, the closure's
        // fixpoint, a bounded repeat, multi-byte characters, a malformed byte
        // and the position one past the last byte: the two forms must name the
        // same end positions from every start.
        let pats = [
            "cond_[0-9]+", "a*b", "(ab|a)(c|bcd)", "x{2,3}", "x{2,}", "x{0,2}y", ".+", "\\w+\\d",
            "[a-c\u{e9}]+", "a?b*", "(a|ab)(bc|c)?", "", "\\s*\\S+", "[^x]{1,3}",
        ];
        let texts: &[&[u8]] = &[
            b"", b"a", b"ab", b"abc", b"abcd", b"cond_42", b"cond_", b"xx", b"xxx", b"xxxxy",
            b"aaab", b"ba", "\u{e9}a\u{e9}".as_bytes(), b"a\xffb", b"snake_case2", b"  two words ",
        ];
        for pat in pats {
            let p = parse(pat.as_bytes()).unwrap();
            for text in texts {
                let vector = reach(&p, text, vec![0]);
                let bits = reach_bits(&p, text, 1);
                let from_bits: Vec<usize> = (0..=text.len()).filter(|&i| (bits & (1 << i)) != 0).collect();
                assert_eq!(from_bits, vector, "{pat} over {text:?}");
                assert_eq!(p.longest_prefix(text), vector.iter().copied().max(), "{pat} over {text:?}");
            }
        }
    }

    #[test]
    fn classes_and_quantifiers() {
        assert!(m("[A-Z][a-z]+", "Title"));
        assert!(!m("[A-Z][a-z]+", "title"));
        assert!(!m("[A-Z][a-z]+", "TITLE"));
        assert!(m("\\d{4}", "2026"));
        assert!(!m("\\d{4}", "202"));
        assert!(m("\\w+", "snake_case2"));
        assert!(m("a|bc|d", "bc"));
        assert!(m("(ab)+", "ababab"));
        assert!(!m("(ab)+", "aba"));
        assert!(m("[^0-9]+", "letters"));
        assert!(!m("[^0-9]+", "ha2"));
    }

    #[test]
    fn a_class_range_spans_characters_not_bytes() {
        // A byte range cannot express these: above 0x7f the individual bytes
        // of a character carry no order a range could read.
        assert!(m("[\u{3b1}-\u{3c9}]+", "\u{3b1}\u{3b2}\u{3b3}"), "a greek range");
        assert!(!m("[\u{3b1}-\u{3c9}]+", "abc"), "and it excludes latin");
        assert!(m("[\u{430}-\u{44f}]+", "\u{434}\u{430}"), "a cyrillic range");
        // ASCII is the same construct, so the existing spellings keep working.
        assert!(m("[a-z]+", "abc"));
    }

    #[test]
    fn a_property_class_reads_unicode_where_the_ascii_builtins_do_not() {
        assert!(m("\\p{L}+", "\u{3b1}\u{3b2}"), "greek letters are letters");
        assert!(m("\\p{L}+", "abc"));
        assert!(m("\\p{N}+", "123"));
        assert!(m("\\p{Lu}\\p{Ll}+", "Title"));
        assert!(m("\\P{L}+", "123"), "the negation excludes letters");
        assert!(!m("\\P{L}+", "abc"));

        // The stated narrowing: the ASCII built-ins stay ASCII, because the
        // lexer's recognizers are written against them and widening \w would
        // change what lexes as what rather than only what matches.
        assert!(!m("\\w+", "\u{3b1}\u{3b2}"), "\\w is ASCII by decision");
        assert!(m("\\p{L}+", "\u{3b1}\u{3b2}"), "and \\p{{L}} is the wide spelling");
    }

    #[test]
    fn an_unknown_property_is_refused_rather_than_matching_nothing() {
        // A class that silently matched nothing would read as "no such
        // character in the input", which is indistinguishable from a pattern
        // that works.
        let err = parse(b"\\p{Nonesuch}").expect_err("unknown category is refused");
        assert!(err.contains("Nonesuch"), "the diagnostic names it: {err}");
        assert!(err.contains('L'), "and lists what is known: {err}");
        assert!(parse(b"\\p{L").is_err(), "unterminated");
        assert!(parse(b"\\pL").is_err(), "missing brace");
    }

    #[test]
    fn an_escape_flag_or_class_backslash_it_does_not_define_is_refused() {
        for pat in ["\\x00", "\\n", "\\t", "a\\bc", "\\u0041", "\\Z"] {
            let err = parse(pat.as_bytes()).expect_err(pat);
            assert!(err.contains("is not a byte-pattern escape"), "{pat}: {err}");
        }
        let err = parse(b"(?s:.){4}").expect_err("an inline flag group");
        assert!(err.contains("`(?`"), "{err}");
        let err = parse(b"[\\x00-\\xff]{4}").expect_err("a backslash in a class");
        assert!(err.contains("class takes no backslash"), "{err}");
        for ok in ["\\d", "\\D", "\\w", "\\W", "\\s", "\\S", "\\p{L}", "\\P{L}"] {
            assert!(parse(ok.as_bytes()).is_ok(), "{ok}");
        }
        assert!(m("a\\.b", "a.b"), "punctuation stays literal");
        assert!(!m("a\\.b", "axb"));
        assert!(m("\\(\\d+\\)", "(12)"));
    }

    #[test]
    fn dot_steps_a_character_not_a_byte() {
        // Three characters, six bytes. Stepping bytes would need six dots and
        // would land inside a character on the way.
        assert!(m("...", "\u{3b1}\u{3b2}\u{3b3}"));
        assert!(!m("......", "\u{3b1}\u{3b2}\u{3b3}"));
        assert_eq!(parse(b".").unwrap().max_len(), Some(MAX_CHAR_BYTES));
        assert_eq!(parse(b"[a-z]").unwrap().max_len(), Some(MAX_CHAR_BYTES));
        // A literal byte and the ASCII built-ins are still one byte.
        assert_eq!(parse(b"a").unwrap().max_len(), Some(1));
        assert_eq!(parse(b"\\d").unwrap().max_len(), Some(1));
    }

    #[test]
    fn malformed_bytes_match_nothing_rather_than_panicking() {
        let pat = parse(b".").expect("parses");
        // A lone continuation byte, a truncated two-byte sequence, and a lead
        // byte no character uses. None starts a character, so none matches,
        // and a pattern cannot land in the middle of one.
        for bad in [&[0x80u8][..], &[0xc3][..], &[0xff][..], &[0xe2, 0x28][..]] {
            assert!(!pat.matches_whole(bad), "no match on {bad:?}");
        }
        let cls = parse("[\u{3b1}-\u{3c9}]".as_bytes()).expect("parses");
        assert!(!cls.matches_whole(&[0xce]), "a truncated greek lead byte");
    }

    /// The byte machine answers what the tree walk answers, for every pattern
    /// it accepts, over texts holding an empty match, a bounded repeat, an open
    /// repeat, a multi-byte character and a malformed byte. A pattern it
    /// refuses is not a failure; a different answer is.
    #[test]
    fn the_byte_machine_answers_what_the_tree_walk_answers() {
        let pats = [
            "cond_[0-9]+", "a*b", "(ab|a)(c|bcd)", "x{2,3}", "x{2,}", "x{0,2}y", "\\w+\\d",
            "a?b*", "(a|ab)(bc|c)?", "", "\\s*\\S+", "[0-9a-f]{2,}", "v[0-9]+\\.[0-9]+",
            ".+", "[a-c\u{e9}]+", "\\p{L}+", "[^x]{1,3}",
        ];
        let texts: &[&[u8]] = &[
            b"", b"a", b"ab", b"abc", b"abcd", b"cond_42", b"cond_", b"xx", b"xxx", b"xxxxy",
            b"aaab", b"ba", "\u{e9}a\u{e9}".as_bytes(), b"a\xffb", b"snake_case2",
            b"  two words ", b"deadbeef", b"v1.20", b"cond_", b"_0",
        ];
        let mut taken = 0usize;
        for src in pats {
            let p = parse(src.as_bytes()).expect("parses");
            let Some(machine) = p.automaton() else { continue };
            taken += 1;
            for text in texts {
                assert_eq!(
                    machine.matches_whole(text),
                    p.matches_whole(text),
                    "{src:?} over {text:?}"
                );
            }
        }
        assert!(taken >= 12, "the machine took {taken} of the patterns");
        // The gate itself: a leaf reading a character, or a negation over one,
        // is refused rather than stepped a byte at a time.
        for src in [".+", "[a-c\u{e9}]+", "\\p{L}+", "[^x]{1,3}", "[\u{3b1}-\u{3c9}]"] {
            let p = parse(src.as_bytes()).expect("parses");
            assert!(p.automaton().is_none(), "{src:?} reads characters and must be refused");
        }
    }

    #[test]
    fn whole_anchored() {
        // matches_whole requires the whole token, not a substring.
        assert!(!m("\\d+", "12a"));
        assert!(m(".*\\d.*", "12a"));
    }
}
