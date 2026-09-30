//! The supertoken tower: a grammar-free unit one level above the token.
//!
//! A supertoken collapses a run of tokens - a clause, a statement, a line -
//! into a single unit tagged by a structural [`Role`]. The role is read from
//! punctuation, bracket shape, and token classes, never from a language's
//! keywords, so `x = 1`, `let x = 1`, and `x: 1` all read as bindings and
//! `foo(a)` reads as a call whatever the language. It is the layer above
//! [`crate::token`] and below a grammar, built from the same signals the
//! property axes use (the bracket stack for depth, the call / assignment /
//! key-value / list shapes for role), so one pass reads code, config, prose,
//! and logs with no per-format parser.
//!
//! `meaning carries, surface does not`: two lines that differ only in their
//! identifiers but share a role and a depth are the same supertoken.

use crate::lexer::lex;
use crate::token::{Token, TokenKind};

/// The grammar-free structural role of a supertoken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// A word run ending just before an opening bracket: a call or application.
    Call,
    /// Contains a lone `=` (not `==`, `<=`, `=>`): a binding.
    Assign,
    /// A word at the head followed by `:`: a key/value entry.
    KeyValue,
    /// Two or more commas: a comma-separated list.
    List,
    /// Dominated by number tokens.
    Numeric,
    /// Anything else - a plain word/text run, such as a prose clause.
    Plain,
}

impl Role {
    /// A small dense code, for hashing the role into a context key.
    #[must_use]
    pub fn code(self) -> u32 {
        match self {
            Role::Call => 1,
            Role::Assign => 2,
            Role::KeyValue => 3,
            Role::List => 4,
            Role::Numeric => 5,
            Role::Plain => 6,
        }
    }

    /// The role a name denotes, or `None` when it names none.
    ///
    /// The inverse of [`Role::label`], so a pattern spells a role the same way
    /// a readout prints it.
    #[must_use]
    pub fn parse(name: &str) -> Option<Role> {
        Some(match name {
            "call" => Role::Call,
            "assign" => Role::Assign,
            "kv" => Role::KeyValue,
            "list" => Role::List,
            "numeric" => Role::Numeric,
            "plain" => Role::Plain,
            _ => return None,
        })
    }

    /// The short name a readout prints for this role.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Role::Call => "call",
            Role::Assign => "assign",
            Role::KeyValue => "kv",
            Role::List => "list",
            Role::Numeric => "numeric",
            Role::Plain => "plain",
        }
    }
}

/// One supertoken: a run of tokens collapsed to a single unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuperToken {
    /// Byte offset where the unit's first token begins.
    pub start: usize,
    /// Byte offset just past the unit's last token.
    pub end: usize,
    /// Bracket-nesting depth the unit sits at.
    pub depth: u32,
    /// The unit's grammar-free structural role.
    pub role: Role,
}

/// The upper-grain window a scan reads while it walks tokens.
///
/// A scan moving over tokens can see the token behind and the token ahead
/// because both are in the slice it holds. What it cannot see is the construct
/// behind or ahead, because a construct is a run of tokens and nothing in the
/// token slice says where one begins. This is that view: indexed by token
/// position, it answers which unit a token is inside, where it sits within it,
/// and which units stand either side.
///
/// Built once per scan and only when a pattern asks for it, in the shape the
/// other axis fields already use.
#[derive(Clone, Debug, Default)]
pub struct SuperContext {
    /// The units themselves, in stream order.
    pub units: Vec<SuperToken>,
    /// Unit index holding each token, or `None` for a token in no unit - a
    /// bracket, a separator, whitespace.
    of_token: Vec<Option<usize>>,
}

impl SuperContext {
    /// Build the window over an already-lexed stream.
    #[must_use]
    pub fn build(toks: &[Token], bytes: &[u8]) -> Self {
        let units = supertokens_from(toks, bytes);
        let mut of_token = vec![None; toks.len()];
        // Units and tokens are both in stream order, so one cursor walks both.
        let mut u = 0usize;
        for (i, t) in toks.iter().enumerate() {
            while u < units.len() && units[u].end <= t.start() {
                u += 1;
            }
            if u < units.len() && t.start() >= units[u].start && t.end() <= units[u].end {
                of_token[i] = Some(u);
            }
        }
        SuperContext { units, of_token }
    }

    /// The unit holding token `i`, if it is in one.
    #[must_use]
    pub fn unit_of(&self, i: usize) -> Option<&SuperToken> {
        self.index_of(i).and_then(|u| self.units.get(u))
    }

    /// The index of the unit holding token `i`.
    #[must_use]
    pub fn index_of(&self, i: usize) -> Option<usize> {
        self.of_token.get(i).copied().flatten()
    }

    /// Whether token `i` is the first token of its unit - the unit boundary a
    /// pattern anchors to.
    #[must_use]
    pub fn starts_unit(&self, i: usize) -> bool {
        match (self.index_of(i), i.checked_sub(1)) {
            (Some(u), Some(prev)) => self.index_of(prev) != Some(u),
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// The unit before the one holding token `i`.
    #[must_use]
    pub fn unit_before(&self, i: usize) -> Option<&SuperToken> {
        self.index_of(i).and_then(|u| u.checked_sub(1)).and_then(|u| self.units.get(u))
    }

    /// The unit after the one holding token `i`.
    #[must_use]
    pub fn unit_after(&self, i: usize) -> Option<&SuperToken> {
        self.index_of(i).and_then(|u| self.units.get(u + 1))
    }
}

/// Build the supertoken tower over `bytes` (lexes internally).
#[must_use]
pub fn supertokens(bytes: &[u8]) -> Vec<SuperToken> {
    supertokens_from(&lex(bytes), bytes)
}

/// Build the supertoken tower over an already-lexed token slice.
///
/// A unit is a maximal run of non-whitespace, non-bracket tokens. It ends at a
/// statement terminator (`;`), a newline, or a bracket boundary; the bracket
/// stack gives each unit its depth. Empty runs (only punctuation or space)
/// emit nothing.
#[must_use]
pub fn supertokens_from(toks: &[Token], bytes: &[u8]) -> Vec<SuperToken> {
    let mut out = Vec::new();
    let mut depth: u32 = 0;
    // Indices into `toks` of the current unit's members (non-whitespace,
    // non-bracket tokens); the token that ends the unit is not a member.
    let mut unit: Vec<usize> = Vec::new();

    let text = |t: &Token| &bytes[t.span()];
    let is_newline = |t: &Token| bytes[t.span()].contains(&b'\n');

    // The end of the last token seen, so a newline can be found in the bytes
    // between two tokens as well as inside one. A stream filtered to
    // significant tokens carries no whitespace, and the newline that ends a
    // unit is then in the gap rather than in a token; over the full stream the
    // tokens are adjacent, the gap is empty, and the whitespace arm below is
    // what fires.
    let mut prev_end = 0usize;
    for (i, t) in toks.iter().enumerate() {
        if prev_end < t.start() && bytes[prev_end..t.start()].contains(&b'\n') {
            flush(&mut out, toks, &unit, depth, bytes, false);
            unit.clear();
        }
        prev_end = t.end();
        match &t.kind {
            TokenKind::Whitespace => {
                if is_newline(t) {
                    flush(&mut out, toks, &unit, depth, bytes, false);
                    unit.clear();
                }
            }
            TokenKind::Punct if text(t) == b";" => {
                flush(&mut out, toks, &unit, depth, bytes, false);
                unit.clear();
            }
            // Only a bracket the lexer paired divides anything. An unpaired one
            // is a character in the text - a truncated field, a byte in a
            // binary blob - and falls to the arm below as a member of whatever
            // unit it sits in.
            TokenKind::Open(_) if t.mate().is_some() => {
                flush(&mut out, toks, &unit, depth, bytes, true);
                unit.clear();
                depth += 1;
            }
            TokenKind::Close(_) if t.mate().is_some() => {
                flush(&mut out, toks, &unit, depth, bytes, false);
                unit.clear();
                depth = depth.saturating_sub(1);
            }
            _ => unit.push(i),
        }
    }
    flush(&mut out, toks, &unit, depth, bytes, false);
    out
}

/// Emit one supertoken for the accumulated `unit`, or nothing if it is empty.
/// `next_open` is true when the unit was ended by an opening bracket, which
/// turns a trailing word into a call.
fn flush(
    out: &mut Vec<SuperToken>,
    toks: &[Token],
    unit: &[usize],
    depth: u32,
    bytes: &[u8],
    next_open: bool,
) {
    let Some(&first) = unit.first() else {
        return;
    };
    let last = *unit.last().expect("non-empty");
    let role = classify(toks, unit, bytes, next_open);
    out.push(SuperToken { start: toks[first].start(), end: toks[last].end(), depth, role });
}

/// Read a unit's role from its shape, grammar-free.
fn classify(toks: &[Token], unit: &[usize], bytes: &[u8], next_open: bool) -> Role {
    let text = |i: usize| &bytes[toks[i].span()];
    let kind = |i: usize| &toks[i].kind;

    // A call: the run ends in a word and an opening bracket follows.
    if next_open && matches!(kind(*unit.last().expect("non-empty")), TokenKind::Word) {
        return Role::Call;
    }
    // An assignment: a lone `=` (its neighbour in the run is not another `=`,
    // and it is not `<=` / `>=` / `!=` / `=>` formed with an adjacent operator).
    let is_op = |b: &[u8]| matches!(b, b"=" | b"<" | b">" | b"!" | b"+" | b"-" | b"*" | b"/" | b"%" | b"&" | b"|" | b"^" | b":");
    let assign = unit.iter().enumerate().any(|(k, &i)| {
        matches!(kind(i), TokenKind::Punct)
            && text(i) == b"="
            && unit.get(k + 1).is_none_or(|&j| text(j) != b"=")
            && (k == 0 || !is_op(text(unit[k - 1])))
    });
    if assign {
        return Role::Assign;
    }
    // A key/value: a word at the head, then a `:`.
    if matches!(kind(unit[0]), TokenKind::Word)
        && unit.get(1).is_some_and(|&i| matches!(kind(i), TokenKind::Punct) && text(i) == b":")
    {
        return Role::KeyValue;
    }
    // A list: two or more commas.
    let commas = unit.iter().filter(|&&i| matches!(kind(i), TokenKind::Punct) && text(i) == b",").count();
    if commas >= 2 {
        return Role::List;
    }
    // Numeric-dominated.
    let nums = unit.iter().filter(|&&i| matches!(kind(i), TokenKind::Number)).count();
    let words = unit.iter().filter(|&&i| matches!(kind(i), TokenKind::Word)).count();
    if nums > 0 && nums >= words {
        return Role::Numeric;
    }
    Role::Plain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_division_does_not_depend_on_which_stream_it_is_given() {
        // A newline ends a unit. It arrives as a whitespace token on the full
        // stream and as a gap on the significant one, where whitespace is the
        // single kind filtered out. Comma-separated rows have no other
        // boundary, so a stream-dependent rule divides them into one unit.
        let src = b"alpha,1,x\nbeta,2,y\ngamma,3,z\ndelta,4,w\nepsilon,5,v\n";
        let full = supertokens_from(&crate::lexer::lex(src), src);
        let sig = supertokens_from(&crate::tokutil::lex_sig(src), src);
        assert!(full.len() >= 5, "a unit per row, got {}", full.len());
        let spans = |u: &[SuperToken]| u.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>();
        assert_eq!(spans(&sig), spans(&full), "both streams divide the file the same way");
    }

    #[test]
    fn an_unpaired_bracket_does_not_divide_a_unit() {
        // The shape a truncated data field takes: a name, an opening bracket,
        // and no close anywhere. Dividing there cuts a row in half at a
        // character, and the level it opens never closes again.
        let src = b"PROBIOTICS [BIFIDOBACTERIUM,LACTOBACILLUS\nnext row here\n";
        let u = supertokens_from(&crate::tokutil::lex_sig(src), src);
        assert_eq!(u.len(), 2, "a unit per row, got {}", u.len());
        assert!(u.iter().all(|s| s.depth == 0), "and no row sits inside the bracket");

        // A pair still divides, and still nests what it holds.
        let paired = b"call(alpha, beta)\nnext row here\n";
        let p = supertokens_from(&crate::tokutil::lex_sig(paired), paired);
        assert!(p.iter().any(|s| s.depth == 1), "the pair still nests its contents");
    }

    #[test]
    fn a_file_whose_only_boundary_is_the_newline_still_divides() {
        // The shape that read as one unit: a million tokens of comma-separated
        // data with no bracket and no semicolon anywhere.
        let mut src = Vec::new();
        for i in 0..400 {
            src.extend_from_slice(format!("row{i},{i},value{i}\n").as_bytes());
        }
        let sig = supertokens_from(&crate::tokutil::lex_sig(&src), &src);
        assert_eq!(sig.len(), 400, "one unit per row on the significant stream");
        assert!(
            sig.iter().all(|u| !src[u.start..u.end].contains(&b'\n')),
            "no unit may span a newline"
        );
    }

    fn roles(s: &str) -> Vec<Role> {
        supertokens(s.as_bytes()).iter().map(|s| s.role).collect()
    }

    #[test]
    fn code_assignment_and_call() {
        // `let x = 1` is a binding; `foo(2)` is a call.
        let st = supertokens(b"let x = 1;\nfoo(2)\n");
        let got: Vec<(Role, u32)> = st.iter().map(|s| (s.role, s.depth)).collect();
        assert!(got.contains(&(Role::Assign, 0)), "assign at depth 0: {got:?}");
        assert!(got.contains(&(Role::Call, 0)), "call at depth 0: {got:?}");
    }

    #[test]
    fn prose_is_plain() {
        // Two lines of prose, both plain word runs.
        let r = roles("the cat sat on the mat\nthe dog ran to the park\n");
        assert_eq!(r, vec![Role::Plain, Role::Plain]);
    }

    #[test]
    fn key_value_and_list() {
        assert_eq!(roles("name: bob\n"), vec![Role::KeyValue]);
        // A comma list inside brackets: the interior run has the commas.
        let r = roles("[1, 2, 3]\n");
        assert!(r.contains(&Role::List), "list: {r:?}");
    }

    #[test]
    fn depth_tracks_nesting() {
        // `f(g(x))`: the innermost `x` sits deeper than the outer `f`.
        let st = supertokens(b"f(g(x))\n");
        let max = st.iter().map(|s| s.depth).max().unwrap_or(0);
        assert!(max >= 2, "nesting reaches depth 2: {st:?}");
    }

    #[test]
    fn empty_is_safe() {
        assert!(supertokens(b"").is_empty());
        assert!(supertokens(b"   \n  \n").is_empty());
    }
}
