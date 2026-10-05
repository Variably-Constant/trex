//! A universal grammar engine: a parser generator over the typed-token
//! alphabet.
//!
//! trex patterns are already grammar productions over tokens. This
//! module makes that explicit: a grammar is a set of named rules over
//! the universal token stream, and the engine parses any text against it
//! with no per-language parser. Because the token alphabet is universal,
//! one grammar reads the same construct across languages.
//!
//! ## Grammar syntax
//!
//! One rule per line, `name := alt | alt | ...`. A `#` starts a comment.
//! Within an alternative, whitespace separates symbols:
//!
//! - `<name>` references another rule.
//! - `"lit"` is a literal token matched by its text (`"+"`, `"("`).
//! - a terminal keyword is a token kind: `number`, `ident`, `string`,
//!   `ip`, `url`, `email`, `time`, `punct`.
//! - any symbol takes an EBNF quantifier suffix: `<x>*` (zero or more),
//!   `<x>+` (one or more), `<x>?` (optional). A quantified symbol matches
//!   greedily, so `call := ident "(" number* ")"` reads a call with a run
//!   of arguments without a recursive list rule.
//! - `( a b | c )` is a group: its own ordered alternatives, matched as a
//!   committed choice with no rule name. A group composes with a quantifier
//!   to repeat a sequence or a choice, so `( ident number )+` and
//!   `( number | ident )*` both parse, and an internal `|` is local to the
//!   group rather than splitting the rule.
//! - `@p` after a top-level alternative weights it (default 1.0), making
//!   the grammar a PCFG the probability semirings score (see below).
//!
//! The first rule is the start symbol unless one is named explicitly.
//!
//! ## Left recursion and precedence
//!
//! A rule whose alternative begins with itself is left-recursive, the
//! form an operator grammar uses to encode precedence and
//! associativity:
//!
//! ```text
//! expr := <expr> "+" <term> | <expr> "-" <term> | <term>
//! term := <term> "*" <factor> | <term> "/" <factor> | <factor>
//! factor := number | ident | "(" <expr> ")"
//! ```
//!
//! The engine grows each left-recursive rule from a seed: it parses the
//! non-recursive base, then re-parses the rule with the recursive
//! references at that position returning the current result, growing it
//! left-associatively until it stops consuming tokens. The precedence falls
//! out of the rule layering (`expr` defers to `term` defers to `factor`),
//! so `2 + 3 * 4` parses as `2 + (3 * 4)` with no precedence annotations.
//! Nested groups ride the same recursion through the `"(" <expr> ")"`
//! alternative.
//!
//! Left recursion is detected through the full left-corner relation, so it
//! is handled the same whether a rule begins with itself (`<expr> "+" ...`)
//! or returns to itself through other rules (`expr := <sum>`, `sum :=
//! <expr> "+" ...`). Both terminate -- the seed bottoms out the recursion
//! -- and both build the left-associative tree.
//!
//! ## Semiring parsing
//!
//! `parse_input` returns one ordered-choice parse. The same grammar also
//! has a value over *every* derivation, computed by one inside chart
//! parameterized by a [`Semiring`] (Goodman 1999): swap the semiring to
//! swap the answer. [`Grammar::recognizes`] is the boolean instance
//! (recognition), [`Grammar::count_parses`] the counting instance (the
//! number of derivations, which quantifies ambiguity), and
//! [`Grammar::value`] the general entry for any semiring.
//!
//! A rule alternative may carry a weight (`expr := <a> "+" <b> @0.7 | ...`,
//! default 1.0), making the grammar a PCFG. [`Grammar::best_parse_prob`] is
//! the Viterbi instance (the most probable derivation's probability, which
//! *resolves* ambiguity) and [`Grammar::total_prob`] the inside instance
//! (the summed probability). The EBNF quantifiers and groups desugar to BNF
//! first; the inside recursion is cubic in the token count, so long input
//! is parsed a sentence at a time.
//!
//! ## Complexity of each entry point
//!
//! [`Grammar::parse_input`] is a packrat descent: a rule's result at a
//! position is memoized for the whole parse, except where the rule can reach a
//! left-recursive one, whose in-progress seed makes a cached result unsound.
//! The semiring chart is cubic in the token count. Neither carries the
//! pattern engine's linear bound, and a reader should not carry it across.
//!
//! The chart parses over a [`Lattice`] -- a DAG of candidate tokens -- not
//! just a linear token stream, so ambiguous tokenizations stay alive and
//! the grammar disambiguates them. [`Lattice::segment`] builds the lattice
//! of all ways a run-together string tiles into dictionary words,
//! [`Grammar::count_segmentations`] / [`Grammar::best_segmentation_prob`]
//! parse over it, and [`Grammar::segment`] lists each tiling the grammar
//! accepts. The linear token stream is the trivial lattice.

use std::collections::{HashMap, HashSet};

use crate::lexer::text;
use crate::token::TokenKind;

/// One symbol in an alternative.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Sym {
    /// `<name>`: a reference to another rule.
    Ref(String),
    /// `"lit"`: a literal token matched by its text.
    Lit(String),
    /// A terminal token kind keyword.
    Kind(TokenKind),
    /// A quantified symbol: `<x>*`, `<x>+`, or `<x>?`. The inner symbol is
    /// matched greedily the quantifier's number of times.
    Repeat(Box<Sym>, Quant),
    /// A parenthesized group `( a b | c )`: its own ordered alternatives,
    /// matched as a committed choice. Logical only -- no input bracket is
    /// consumed -- so it groups symbols for a quantifier or localizes an
    /// alternation without naming a rule.
    Group(Vec<Vec<Sym>>),
}

/// An EBNF repetition quantifier on a grammar symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Quant {
    /// `*`: zero or more.
    Star,
    /// `+`: one or more.
    Plus,
    /// `?`: zero or one.
    Opt,
}

impl Quant {
    /// The inclusive `(min, max)` repetition count.
    fn bounds(self) -> (usize, usize) {
        match self {
            Quant::Star => (0, usize::MAX),
            Quant::Plus => (1, usize::MAX),
            Quant::Opt => (0, 1),
        }
    }

    /// The surface glyph, for diagnostics.
    fn glyph(self) -> char {
        match self {
            Quant::Star => '*',
            Quant::Plus => '+',
            Quant::Opt => '?',
        }
    }
}

/// A rule: a name, its ordered alternatives, and a weight per alternative
/// (the probability semirings use the weights; default 1.0).
#[derive(Clone, Debug)]
struct Rule {
    name: String,
    alts: Vec<Vec<Sym>>,
    weights: Vec<f64>,
}

/// A parsed grammar.
#[derive(Clone, Debug)]
pub struct Grammar {
    rules: Vec<Rule>,
    start: String,
}

/// A grammar parse failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrammarError {
    /// 1-based line number in the grammar text.
    pub line: usize,
    /// Human-readable reason.
    pub msg: String,
}

/// A node in the parse tree of an input against a grammar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// A matched terminal: the token's text.
    Terminal(String),
    /// A rule match with its children, in order.
    Branch(String, Vec<Node>),
}

/// A rule's result at a position: the node and the position after it, or
/// `None` for no match.
type Memo = HashMap<(String, usize), Option<(Node, usize)>>;

/// The state one `parse_input` call threads through the descent.
///
/// The two memos answer different questions and must not be merged. `seed`
/// holds the in-progress result of a left-recursive rule so recursive
/// references at that position return the seed instead of recursing forever;
/// its entries are transient and removed when growth ends. `packrat` holds
/// completed results and is what makes the descent polynomial.
///
/// A rule is cached only when `dep` says its derivation cannot reach a
/// left-recursive rule. That condition is what makes caching sound: a
/// derivation that touches a rule currently being grown depends on the seed
/// installed at that moment, so its result is not a function of the grammar
/// and the token stream alone.
struct ParseCtx<'a> {
    lr: &'a HashSet<String>,
    dep: &'a HashSet<String>,
    seed: Memo,
    packrat: Memo,
}

impl Grammar {
    /// Parse grammar text into a [`Grammar`].
    ///
    /// # Errors
    ///
    /// Returns a [`GrammarError`] for a malformed rule, an unknown
    /// terminal keyword, or an empty grammar.
    pub fn parse(src: &str) -> Result<Grammar, GrammarError> {
        let mut rules: Vec<Rule> = Vec::new();
        for (idx, raw) in src.lines().enumerate() {
            let line_no = idx + 1;
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            let Some((name, rhs)) = line.split_once(":=") else {
                return Err(GrammarError { line: line_no, msg: "expected `name := ...`".into() });
            };
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err(GrammarError { line: line_no, msg: "empty rule name".into() });
            }
            let (alts, weights) = parse_rhs(rhs, line_no)?;
            rules.push(Rule { name, alts, weights });
        }
        if rules.is_empty() {
            return Err(GrammarError { line: 0, msg: "grammar has no rules".into() });
        }
        let start = rules[0].name.clone();
        Ok(Grammar { rules, start })
    }

    /// Set the start rule by name.
    #[must_use]
    pub fn with_start(mut self, start: &str) -> Self {
        self.start = start.to_string();
        self
    }

    /// The rule a parse starts from: the first one declared unless
    /// [`Self::with_start`] named another.
    #[must_use]
    pub fn start(&self) -> &str {
        &self.start
    }

    /// Every rule's name, in the order the grammar declares them.
    #[must_use]
    pub fn rule_names(&self) -> Vec<&str> {
        self.rules.iter().map(|r| r.name.as_str()).collect()
    }

    fn rule(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.name == name)
    }

    /// Parse `input` against the grammar's start rule, returning the
    /// parse tree, or `None` when the input does not parse in full.
    #[must_use]
    pub fn parse_input(&self, input: &[u8]) -> Option<Node> {
        let toks: Vec<Term> = crate::parallel_lex::lex_parallel(input)
            .iter()
            .filter(|t| t.kind != TokenKind::Whitespace)
            .map(|t| Term {
                kind: t.kind,
                text: String::from_utf8_lossy(text(input, t)).into_owned(),
            })
            .collect();
        let lr = self.left_recursive_rules();
        let dep = self.lr_dependent(&lr);
        let mut ctx =
            ParseCtx { lr: &lr, dep: &dep, seed: HashMap::new(), packrat: HashMap::new() };
        let (node, pos) = self.parse_rule(&self.start, &toks, 0, &mut ctx)?;
        // The whole input must be consumed for a successful parse.
        (pos == toks.len()).then_some(node)
    }

    /// Rule names whose derivation can reach a left-recursive rule, through any
    /// reference at any position. A rule outside this set derives independently
    /// of every in-progress seed, so its result at a position is a function of
    /// the grammar and the token stream alone and is safe to memoize for the
    /// whole parse.
    fn lr_dependent(&self, lr: &HashSet<String>) -> HashSet<String> {
        let mut out = lr.clone();
        loop {
            let mut changed = false;
            for rule in &self.rules {
                if out.contains(&rule.name) {
                    continue;
                }
                let mut refs: Vec<String> = Vec::new();
                for alt in &rule.alts {
                    for s in alt {
                        sym_all_refs(s, &mut refs);
                    }
                }
                if refs.iter().any(|r| out.contains(r.as_str())) {
                    out.insert(rule.name.clone());
                    changed = true;
                }
            }
            if !changed {
                return out;
            }
        }
    }

    /// The rule names that are left-recursive, directly (an alternative
    /// begins with the rule itself) or indirectly (the left-corner chain
    /// returns to the rule through other rules). Left-recursive rules take
    /// the seed-growing parse; the rest take a plain ordered choice.
    fn left_recursive_rules(&self) -> HashSet<String> {
        let nullable = self.nullable_rules();
        self.rules
            .iter()
            .filter(|r| self.reaches_self_leftmost(&r.name, &nullable))
            .map(|r| r.name.clone())
            .collect()
    }

    /// The rule names that can match the empty token sequence, by fixpoint:
    /// a rule is nullable if some alternative is all-nullable, which can
    /// newly make a referencing rule nullable, so iterate to convergence.
    fn nullable_rules(&self) -> HashSet<String> {
        let mut nullable: HashSet<String> = HashSet::new();
        loop {
            let mut changed = false;
            for rule in &self.rules {
                if !nullable.contains(&rule.name)
                    && rule.alts.iter().any(|alt| self.seq_nullable(alt, &nullable))
                {
                    nullable.insert(rule.name.clone());
                    changed = true;
                }
            }
            if !changed {
                return nullable;
            }
        }
    }

    /// Whether a sequence can match empty: every symbol in it is nullable.
    fn seq_nullable(&self, seq: &[Sym], nullable: &HashSet<String>) -> bool {
        seq.iter().all(|s| self.sym_nullable(s, nullable))
    }

    /// Whether one symbol can match empty.
    fn sym_nullable(&self, sym: &Sym, nullable: &HashSet<String>) -> bool {
        match sym {
            Sym::Ref(r) => nullable.contains(r.as_str()),
            Sym::Lit(_) | Sym::Kind(_) => false,
            Sym::Repeat(inner, q) => {
                matches!(q, Quant::Star | Quant::Opt) || self.sym_nullable(inner, nullable)
            }
            Sym::Group(alts) => alts.iter().any(|alt| self.seq_nullable(alt, nullable)),
        }
    }

    /// Whether `name` returns to itself through the left-corner relation:
    /// any rule it can begin with, transitively. A nullable leading symbol
    /// exposes the next as a left corner, so `<x>? <name>` and a nullable
    /// group prefix still count -- which routes a left recursion behind a
    /// nullable prefix onto the seed-growing path instead of recursing
    /// forever.
    fn reaches_self_leftmost(&self, name: &str, nullable: &HashSet<String>) -> bool {
        let mut stack: Vec<String> = vec![name.to_string()];
        let mut seen: HashSet<String> = HashSet::new();
        let mut refs: Vec<String> = Vec::new();
        while let Some(cur) = stack.pop() {
            let Some(rule) = self.rule(&cur) else { continue };
            refs.clear();
            for alt in &rule.alts {
                self.seq_leftmost_refs(alt, nullable, &mut refs);
            }
            for r in refs.drain(..) {
                if r.as_str() == name {
                    return true;
                }
                if seen.insert(r.clone()) {
                    stack.push(r);
                }
            }
        }
        false
    }

    /// Collect the rule names that can be a left corner of a sequence: the
    /// leftmost symbol's, plus -- through each nullable leading symbol --
    /// the next symbol's.
    fn seq_leftmost_refs(&self, seq: &[Sym], nullable: &HashSet<String>, out: &mut Vec<String>) {
        for s in seq {
            self.sym_leftmost_refs(s, nullable, out);
            if !self.sym_nullable(s, nullable) {
                break;
            }
        }
    }

    /// Collect the rule names a single symbol can begin with.
    fn sym_leftmost_refs(&self, sym: &Sym, nullable: &HashSet<String>, out: &mut Vec<String>) {
        match sym {
            Sym::Ref(r) => out.push(r.clone()),
            Sym::Lit(_) | Sym::Kind(_) => {}
            Sym::Repeat(inner, _) => self.sym_leftmost_refs(inner, nullable, out),
            Sym::Group(alts) => {
                for alt in alts {
                    self.seq_leftmost_refs(alt, nullable, out);
                }
            }
        }
    }

    /// Parse one rule at significant-token position `pos`, returning the
    /// node and the position after it. A left-recursive rule grows a seed:
    /// a base alternative seeds the parse, then the rule is re-parsed with
    /// the recursive references at this position returning the current seed,
    /// growing it until it stops consuming more tokens. That builds the
    /// left-associative structure and terminates both direct and indirect
    /// left recursion (the recursion bottoms out on the installed seed).
    fn parse_rule(
        &self,
        name: &str,
        toks: &[Term],
        pos: usize,
        ctx: &mut ParseCtx<'_>,
    ) -> Option<(Node, usize)> {
        let key = (name.to_string(), pos);
        if let Some(seed) = ctx.seed.get(&key) {
            return seed.clone();
        }
        if !ctx.lr.contains(name) {
            let cacheable = !ctx.dep.contains(name);
            if cacheable && let Some(hit) = ctx.packrat.get(&key) {
                return hit.clone();
            }
            let out = self.parse_alts(name, toks, pos, ctx);
            if cacheable {
                ctx.packrat.insert(key, out.clone());
            }
            return out;
        }
        // Install a failing seed so the recursive references at this
        // position fail until a base alternative seeds the growth.
        ctx.seed.insert(key.clone(), None);
        let mut seed = self.parse_alts(name, toks, pos, ctx);
        while let Some((_, end)) = seed.clone() {
            ctx.seed.insert(key.clone(), seed.clone());
            match self.parse_alts(name, toks, pos, ctx) {
                Some((node, np)) if np > end => seed = Some((node, np)),
                _ => break,
            }
        }
        ctx.seed.remove(&key);
        seed
    }

    /// Try each alternative of `name` in order at `pos`, returning the first
    /// that matches.
    fn parse_alts(
        &self,
        name: &str,
        toks: &[Term],
        pos: usize,
        ctx: &mut ParseCtx<'_>,
    ) -> Option<(Node, usize)> {
        let rule = self.rule(name)?;
        for alt in &rule.alts {
            if let Some((children, np)) = self.match_seq(alt, toks, pos, ctx) {
                return Some((Node::Branch(name.to_string(), children), np));
            }
        }
        None
    }

    /// Match a sequence of symbols starting at `pos`, returning the child
    /// nodes and the position after the sequence, or `None`.
    fn match_seq(
        &self,
        syms: &[Sym],
        toks: &[Term],
        pos: usize,
        ctx: &mut ParseCtx<'_>,
    ) -> Option<(Vec<Node>, usize)> {
        let mut children: Vec<Node> = Vec::new();
        let mut p = pos;
        for sym in syms {
            p = self.match_sym(sym, toks, p, ctx, &mut children)?;
        }
        Some((children, p))
    }

    /// Match one symbol at `pos`, pushing its node(s) into `children` and
    /// returning the position after it, or `None` if it does not match.
    fn match_sym(
        &self,
        sym: &Sym,
        toks: &[Term],
        pos: usize,
        ctx: &mut ParseCtx<'_>,
        children: &mut Vec<Node>,
    ) -> Option<usize> {
        match sym {
            Sym::Ref(name) => {
                let (node, np) = self.parse_rule(name, toks, pos, ctx)?;
                children.push(node);
                Some(np)
            }
            Sym::Lit(lit) => {
                let t = toks.get(pos)?;
                if &t.text != lit {
                    return None;
                }
                children.push(Node::Terminal(t.text.clone()));
                Some(pos + 1)
            }
            Sym::Kind(k) => {
                let t = toks.get(pos)?;
                if t.kind != *k {
                    return None;
                }
                children.push(Node::Terminal(t.text.clone()));
                Some(pos + 1)
            }
            Sym::Repeat(inner, quant) => {
                // Greedy, committed repetition (PEG semantics): match the
                // inner symbol as many times as it will, bounded by the
                // quantifier. Progress is required each round so a nullable
                // inner cannot loop forever. The matched nodes are collected
                // locally and only committed if the minimum count is met, so
                // a failed `+` leaves `children` untouched.
                let (min, max) = quant.bounds();
                let mut local: Vec<Node> = Vec::new();
                let mut p = pos;
                let mut count = 0;
                while count < max {
                    let mut round: Vec<Node> = Vec::new();
                    match self.match_sym(inner, toks, p, ctx, &mut round) {
                        Some(np) if np > p => {
                            local.append(&mut round);
                            p = np;
                            count += 1;
                        }
                        _ => break,
                    }
                }
                if count < min {
                    return None;
                }
                children.append(&mut local);
                Some(p)
            }
            Sym::Group(alts) => {
                // Ordered (committed) choice over the group's alternatives,
                // the same PEG semantics as a rule's alternatives: the first
                // alternative that matches at `pos` wins, and its nodes are
                // flattened into the parent, the group being logical only.
                for alt in alts {
                    if let Some((mut kids, np)) = self.match_seq(alt, toks, pos, ctx) {
                        children.append(&mut kids);
                        return Some(np);
                    }
                }
                None
            }
        }
    }
}

/// A significant token reduced to what the grammar engine needs.
struct Term {
    kind: TokenKind,
    text: String,
}

impl Node {
    /// Render the parse tree as a compact S-expression: a terminal is
    /// its text, a branch is `(rule child ...)`, with single-child
    /// branches collapsed so the operator structure is the visible
    /// shape.
    #[must_use]
    pub fn sexpr(&self) -> String {
        match self {
            Node::Terminal(t) => t.clone(),
            Node::Branch(_, kids) if kids.len() == 1 => kids[0].sexpr(),
            Node::Branch(name, kids) => {
                let parts: Vec<String> = kids.iter().map(Node::sexpr).collect();
                format!("({name} {})", parts.join(" "))
            }
        }
    }
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(before, _)| before)
}

/// Every rule name a symbol references, at any position and any depth.
/// Distinct from the left-corner walk, which stops at the first symbol that
/// cannot match empty.
fn sym_all_refs(sym: &Sym, out: &mut Vec<String>) {
    match sym {
        Sym::Ref(r) => out.push(r.clone()),
        Sym::Lit(_) | Sym::Kind(_) => {}
        Sym::Repeat(inner, _) => sym_all_refs(inner, out),
        Sym::Group(alts) => {
            for alt in alts {
                for s in alt {
                    sym_all_refs(s, out);
                }
            }
        }
    }
}


/// A lexical token of a rule body: the building blocks the recursive
/// descent in [`parse_rhs`] assembles into symbols and alternatives.
#[derive(Clone, Debug, PartialEq)]
enum RhsTok {
    /// `<name>`.
    Ref(String),
    /// `"lit"`.
    Lit(String),
    /// A bare word, resolved to a terminal kind.
    Word(String),
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `|`
    Pipe,
    /// `*` / `+` / `?`
    Quant(Quant),
    /// `@p`: the weight of the alternative just parsed.
    Weight(f64),
}

/// Tokenize a rule body. `<ref>` and `"lit"` are single tokens (their
/// delimiters keep their contents together through whitespace); `(`, `)`,
/// `|`, and the quantifiers are punctuation; everything else is a bare word.
fn tokenize_rhs(rhs: &str, line: usize) -> Result<Vec<RhsTok>, GrammarError> {
    let bytes = rhs.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'<' {
            let start = i + 1;
            let end = rhs[start..]
                .find('>')
                .map(|o| start + o)
                .ok_or_else(|| GrammarError { line, msg: "unterminated `<reference>`".into() })?;
            if start == end {
                return Err(GrammarError { line, msg: "empty `<reference>`".into() });
            }
            out.push(RhsTok::Ref(rhs[start..end].to_string()));
            i = end + 1;
        } else if c == b'"' {
            let start = i + 1;
            let end = rhs[start..]
                .find('"')
                .map(|o| start + o)
                .ok_or_else(|| GrammarError { line, msg: "unterminated `\"literal\"`".into() })?;
            out.push(RhsTok::Lit(rhs[start..end].to_string()));
            i = end + 1;
        } else if c == b'(' {
            out.push(RhsTok::LParen);
            i += 1;
        } else if c == b')' {
            out.push(RhsTok::RParen);
            i += 1;
        } else if c == b'|' {
            out.push(RhsTok::Pipe);
            i += 1;
        } else if let Some(q) = quant_of(c) {
            out.push(RhsTok::Quant(q));
            i += 1;
        } else if c == b'@' {
            // `@p` weights the alternative just parsed; read the number.
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            let num = &rhs[start..j];
            let w: f64 = num
                .parse()
                .map_err(|_| GrammarError { line, msg: format!("`@{num}` is not a weight") })?;
            out.push(RhsTok::Weight(w));
            i = j;
        } else {
            // A bare word runs until whitespace or a punctuation byte. Those
            // stops are all ASCII, so the slice ends on a char boundary even
            // when the word contains multibyte text.
            let start = i;
            while i < bytes.len() {
                let d = bytes[i];
                if d.is_ascii_whitespace()
                    || matches!(d, b'<' | b'"' | b'(' | b')' | b'|' | b'*' | b'+' | b'?' | b'@')
                {
                    break;
                }
                i += 1;
            }
            out.push(RhsTok::Word(rhs[start..i].to_string()));
        }
    }
    Ok(out)
}

/// The quantifier a punctuation byte denotes, if any.
fn quant_of(c: u8) -> Option<Quant> {
    match c {
        b'*' => Some(Quant::Star),
        b'+' => Some(Quant::Plus),
        b'?' => Some(Quant::Opt),
        _ => None,
    }
}

/// Parse a rule body into its ordered alternatives and their weights. The
/// grammar is a small EBNF: `alts := walt ('|' walt)*`, `walt := seq
/// ('@' weight)?`, `seq := item*`, `item := primary quant*`, `primary :=
/// <ref> | "lit" | kind | '(' alts ')'`. A top-level alternative may carry
/// a weight (`@p`, default 1.0); a group is its own nested alternatives,
/// unweighted, so `( <a> | <b> )*` and an internal `|` both parse.
fn parse_rhs(rhs: &str, line: usize) -> Result<(Vec<Vec<Sym>>, Vec<f64>), GrammarError> {
    let toks = tokenize_rhs(rhs, line)?;
    let mut pos = 0;
    let mut alts = Vec::new();
    let mut weights = Vec::new();
    loop {
        alts.push(parse_seq(&toks, &mut pos, line)?);
        let w = if let Some(RhsTok::Weight(w)) = toks.get(pos) {
            pos += 1;
            *w
        } else {
            1.0
        };
        weights.push(w);
        if matches!(toks.get(pos), Some(RhsTok::Pipe)) {
            pos += 1;
        } else {
            break;
        }
    }
    if pos != toks.len() {
        return Err(GrammarError { line, msg: "unexpected `)` in rule body".into() });
    }
    Ok((alts, weights))
}

fn parse_alts(toks: &[RhsTok], pos: &mut usize, line: usize) -> Result<Vec<Vec<Sym>>, GrammarError> {
    let mut alts = vec![parse_seq(toks, pos, line)?];
    while matches!(toks.get(*pos), Some(RhsTok::Pipe)) {
        *pos += 1;
        alts.push(parse_seq(toks, pos, line)?);
    }
    Ok(alts)
}

fn parse_seq(toks: &[RhsTok], pos: &mut usize, line: usize) -> Result<Vec<Sym>, GrammarError> {
    let mut seq = Vec::new();
    while let Some(t) = toks.get(*pos) {
        if matches!(t, RhsTok::Pipe | RhsTok::RParen | RhsTok::Weight(_)) {
            break;
        }
        seq.push(parse_item(toks, pos, line)?);
    }
    Ok(seq)
}

fn parse_item(toks: &[RhsTok], pos: &mut usize, line: usize) -> Result<Sym, GrammarError> {
    let mut sym = parse_primary(toks, pos, line)?;
    while let Some(RhsTok::Quant(q)) = toks.get(*pos) {
        sym = Sym::Repeat(Box::new(sym), *q);
        *pos += 1;
    }
    Ok(sym)
}

fn parse_primary(toks: &[RhsTok], pos: &mut usize, line: usize) -> Result<Sym, GrammarError> {
    let tok = toks
        .get(*pos)
        .ok_or_else(|| GrammarError { line, msg: "unexpected end of rule body".into() })?;
    match tok {
        RhsTok::Ref(r) => {
            *pos += 1;
            Ok(Sym::Ref(r.clone()))
        }
        RhsTok::Lit(l) => {
            *pos += 1;
            Ok(Sym::Lit(l.clone()))
        }
        RhsTok::Word(w) => {
            let sym = kind_from_word(w, line)?;
            *pos += 1;
            Ok(sym)
        }
        RhsTok::LParen => {
            *pos += 1;
            let alts = parse_alts(toks, pos, line)?;
            if !matches!(toks.get(*pos), Some(RhsTok::RParen)) {
                return Err(GrammarError { line, msg: "unterminated `(` group".into() });
            }
            *pos += 1;
            Ok(Sym::Group(alts))
        }
        RhsTok::Quant(q) => {
            Err(GrammarError { line, msg: format!("`{}` has no symbol to repeat", q.glyph()) })
        }
        RhsTok::RParen | RhsTok::Pipe | RhsTok::Weight(_) => {
            Err(GrammarError { line, msg: "expected a symbol".into() })
        }
    }
}

/// Resolve a bare word to its terminal token kind.
fn kind_from_word(word: &str, line: usize) -> Result<Sym, GrammarError> {
    let kind = match word {
        "number" => TokenKind::Number,
        "ident" => TokenKind::Word,
        "string" => TokenKind::Quoted,
        "ip" => TokenKind::Ip,
        "url" => TokenKind::Url,
        "email" => TokenKind::Email,
        "time" => TokenKind::Timestamp,
        "punct" => TokenKind::Punct,
        other => {
            return Err(GrammarError {
                line,
                msg: format!("unknown symbol `{other}` (use <ref>, \"lit\", a group `(...)`, or a terminal: number/ident/string/ip/url/email/time/punct)"),
            });
        }
    };
    Ok(Sym::Kind(kind))
}

// Semiring parsing (Goodman 1999): one Earley chart computes a different
// quantity over all derivations by swapping the semiring. `parse_input`
// returns one ordered-choice (PEG) parse; this returns a value over every
// derivation, which is what ambiguity in natural language needs.

/// A semiring `(A, add, mul, zero, one)`: `add` is associative and
/// commutative with identity `zero`; `mul` is associative, distributes over
/// `add`, has identity `one`, and annihilates (`x.mul(zero) == zero`).
/// Recognition, derivation counting, and best-derivation scoring are the
/// boolean, counting, and Viterbi instances of this one structure.
pub trait Semiring: Clone + PartialEq {
    /// The additive identity (no derivation).
    fn zero() -> Self;
    /// The multiplicative identity (the empty derivation).
    fn one() -> Self;
    /// Combine alternative derivations of the same item.
    fn add(&self, other: &Self) -> Self;
    /// Combine the parts of one derivation in sequence.
    fn mul(&self, other: &Self) -> Self;
}

/// The boolean semiring: whether any derivation exists (recognition).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recognize(pub bool);

impl Semiring for Recognize {
    fn zero() -> Self {
        Recognize(false)
    }
    fn one() -> Self {
        Recognize(true)
    }
    fn add(&self, other: &Self) -> Self {
        Recognize(self.0 || other.0)
    }
    fn mul(&self, other: &Self) -> Self {
        Recognize(self.0 && other.0)
    }
}

/// The counting semiring: the number of distinct derivations. Saturating,
/// so an unboundedly ambiguous input reports the ceiling rather than
/// overflowing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Count(pub u64);

impl Semiring for Count {
    fn zero() -> Self {
        Count(0)
    }
    fn one() -> Self {
        Count(1)
    }
    fn add(&self, other: &Self) -> Self {
        Count(self.0.saturating_add(other.0))
    }
    fn mul(&self, other: &Self) -> Self {
        Count(self.0.saturating_mul(other.0))
    }
}

/// The Viterbi semiring: the probability of the single most probable
/// derivation. `add` keeps the better derivation (max), `mul` is the
/// product of a derivation's rule probabilities. Resolves ambiguity by
/// scoring each parse under a weighted grammar and taking the best.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viterbi(pub f64);

impl Semiring for Viterbi {
    fn zero() -> Self {
        Viterbi(0.0)
    }
    fn one() -> Self {
        Viterbi(1.0)
    }
    fn add(&self, other: &Self) -> Self {
        Viterbi(self.0.max(other.0))
    }
    fn mul(&self, other: &Self) -> Self {
        Viterbi(self.0 * other.0)
    }
}

/// The inside (probability) semiring: the total probability of the input,
/// summed over every derivation. `add` sums, `mul` is the product of a
/// derivation's rule probabilities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prob(pub f64);

impl Semiring for Prob {
    fn zero() -> Self {
        Prob(0.0)
    }
    fn one() -> Self {
        Prob(1.0)
    }
    fn add(&self, other: &Self) -> Self {
        Prob(self.0 + other.0)
    }
    fn mul(&self, other: &Self) -> Self {
        Prob(self.0 * other.0)
    }
}

/// A symbol in the desugared BNF the Earley chart runs over: a literal or
/// kind terminal, or a nonterminal by index.
#[derive(Clone, Debug)]
enum BSym {
    Lit(String),
    Kind(TokenKind),
    Nt(usize),
}

/// The grammar as plain BNF: `alts[nt]` is nonterminal `nt`'s ordered
/// alternatives, each a sequence of [`BSym`]; `weights[nt][ai]` is that
/// alternative's rule weight (the probability semirings use it; recognition
/// and counting ignore it). EBNF quantifiers and groups are desugared into
/// synthetic nonterminals weighted 1.0, so the chart sees only plain rules.
struct Bnf {
    alts: Vec<Vec<Vec<BSym>>>,
    weights: Vec<Vec<f64>>,
    start: usize,
}

/// Lower one EBNF symbol to a BNF symbol, minting synthetic nonterminals
/// (weighted 1.0) for quantifiers and groups. Returns `None` for a
/// reference to an undefined rule.
fn desugar_sym(
    sym: &Sym,
    names: &HashMap<&str, usize>,
    weights: &mut Vec<Vec<f64>>,
    alts: &mut Vec<Vec<Vec<BSym>>>,
) -> Option<BSym> {
    match sym {
        Sym::Lit(l) => Some(BSym::Lit(l.clone())),
        Sym::Kind(k) => Some(BSym::Kind(*k)),
        Sym::Ref(r) => names.get(r.as_str()).map(|&i| BSym::Nt(i)),
        Sym::Repeat(inner, quant) => {
            let inner_sym = desugar_sym(inner, names, weights, alts)?;
            let nt = alts.len();
            alts.push(Vec::new());
            weights.push(Vec::new());
            // `N := inner N | <empty>` for `*`, `inner N | inner` for `+`,
            // `inner | <empty>` for `?` -- a right-recursive list whose empty
            // alternative is the epsilon Earley handles.
            alts[nt] = match quant {
                Quant::Star => vec![vec![inner_sym, BSym::Nt(nt)], vec![]],
                Quant::Plus => vec![vec![inner_sym.clone(), BSym::Nt(nt)], vec![inner_sym]],
                Quant::Opt => vec![vec![inner_sym], vec![]],
            };
            weights[nt] = vec![1.0; alts[nt].len()];
            Some(BSym::Nt(nt))
        }
        Sym::Group(galts) => {
            let nt = alts.len();
            alts.push(Vec::new());
            weights.push(Vec::new());
            let mut bnf_alts = Vec::with_capacity(galts.len());
            for ga in galts {
                let mut seq = Vec::with_capacity(ga.len());
                for s in ga {
                    seq.push(desugar_sym(s, names, weights, alts)?);
                }
                bnf_alts.push(seq);
            }
            let count = bnf_alts.len();
            alts[nt] = bnf_alts;
            weights[nt] = vec![1.0; count];
            Some(BSym::Nt(nt))
        }
    }
}

/// Whether a BNF terminal matches a token.
/// An edge in a [`Lattice`]: a candidate token spanning to a later
/// position, with its text and kind.
struct LatEdge {
    to: usize,
    text: String,
    kind: TokenKind,
}

/// A lattice over input positions: a directed acyclic graph whose edges are
/// candidate tokens, so several edges leaving one position are competing
/// segmentations of the same span. The linear token stream is the trivial
/// lattice (one edge `i -> i+1` per token); an ambiguous tokenization has
/// several, and the inside chart sums or maximizes over every path through
/// it (Mohri et al. 2002) -- the grammar disambiguates the segmentation
/// rather than the lexer committing to one.
pub struct Lattice {
    n: usize,
    edges: Vec<Vec<LatEdge>>,
}

impl Lattice {
    /// The trivial lattice of a token stream: one edge `i -> i+1` per token.
    fn linear(toks: &[Term]) -> Lattice {
        let mut edges: Vec<Vec<LatEdge>> = (0..toks.len()).map(|_| Vec::new()).collect();
        for (i, t) in toks.iter().enumerate() {
            edges[i].push(LatEdge { to: i + 1, text: t.text.clone(), kind: t.kind });
        }
        Lattice { n: toks.len(), edges }
    }

    /// The segmentation lattice of a run-together string under a dictionary:
    /// from each character position, an edge for every dictionary word that
    /// matches there, so every path from 0 to the end is one way to tile the
    /// input with dictionary words. Entries match by text and carry kind
    /// [`TokenKind::Word`], so a grammar `ident` or a `"word"` literal
    /// matches them.
    #[must_use]
    pub fn segment(input: &str, dict: &[String]) -> Lattice {
        let chars: Vec<char> = input.chars().collect();
        let n = chars.len();
        let mut edges: Vec<Vec<LatEdge>> = (0..n).map(|_| Vec::new()).collect();
        for i in 0..n {
            for w in dict {
                let wlen = w.chars().count();
                if wlen > 0 && i + wlen <= n && chars[i..i + wlen].iter().copied().eq(w.chars()) {
                    edges[i].push(LatEdge { to: i + wlen, text: w.clone(), kind: TokenKind::Word });
                }
            }
        }
        Lattice { n, edges }
    }

    /// Whether some edge `i -> m` matches the BNF terminal `term`.
    fn edge_matches(&self, i: usize, m: usize, term: &BSym) -> bool {
        self.edges.get(i).is_some_and(|es| {
            es.iter().any(|e| {
                e.to == m
                    && match term {
                        BSym::Lit(l) => e.text == *l,
                        BSym::Kind(k) => e.kind == *k,
                        BSym::Nt(_) => false,
                    }
            })
        })
    }
}

/// The inside computation (Goodman's recognition/inside recursion): the
/// semiring value of each nonterminal over each token span. A rule
/// alternative's value is the `mul` of its symbols, summed by `add` over
/// the ways to split the span; a nonterminal's value is the `add` over its
/// alternatives. Spans are memoized and reached innermost-first, so each
/// derivation contributes exactly once -- which is what a non-idempotent
/// semiring (counting, probability) needs and a flat chart fixpoint gets
/// wrong. Left recursion recurses on a strictly smaller span (the operator
/// and operand consume), so it terminates; a same-span re-entry is a
/// nullable cycle, returned as `zero` and absorbed by the annihilating
/// `mul`. Cost is cubic in the token count, so a long natural-language
/// input is parsed a sentence at a time.
struct Inside<'a, S: Semiring, W: Fn(f64) -> S> {
    bnf: &'a Bnf,
    lattice: &'a Lattice,
    /// Maps a rule alternative's weight to its value in the semiring.
    weight_of: &'a W,
    /// `(nt, i, j)` to value; `None` marks an in-progress span (a cycle).
    nt_memo: HashMap<(usize, usize, usize), Option<S>>,
    /// `(nt, alt, dot, i, j)` to the value of that alternative's tail.
    seq_memo: HashMap<(usize, usize, usize, usize, usize), S>,
}

impl<S: Semiring, W: Fn(f64) -> S> Inside<'_, S, W> {
    /// The value of nonterminal `nt` deriving tokens `[i, j)`.
    fn nt(&mut self, nt: usize, i: usize, j: usize) -> S {
        if let Some(entry) = self.nt_memo.get(&(nt, i, j)) {
            return entry.clone().unwrap_or_else(S::zero);
        }
        self.nt_memo.insert((nt, i, j), None);
        let mut val = S::zero();
        for ai in 0..self.bnf.alts[nt].len() {
            let tail = self.seq(nt, ai, 0, i, j);
            let weighted = (self.weight_of)(self.bnf.weights[nt][ai]).mul(&tail);
            val = val.add(&weighted);
        }
        self.nt_memo.insert((nt, i, j), Some(val.clone()));
        val
    }

    /// The value of alternative `ai` of `nt` from symbol `dot` onward
    /// deriving `[i, j)`, summed over the split points.
    fn seq(&mut self, nt: usize, ai: usize, dot: usize, i: usize, j: usize) -> S {
        if dot == self.bnf.alts[nt][ai].len() {
            return if i == j { S::one() } else { S::zero() };
        }
        if let Some(v) = self.seq_memo.get(&(nt, ai, dot, i, j)) {
            return v.clone();
        }
        let mut val = S::zero();
        for m in i..=j {
            let head = self.sym(nt, ai, dot, i, m);
            if head == S::zero() {
                continue;
            }
            let tail = self.seq(nt, ai, dot + 1, m, j);
            val = val.add(&head.mul(&tail));
        }
        self.seq_memo.insert((nt, ai, dot, i, j), val.clone());
        val
    }

    /// The value of the `dot`-th symbol of `nt`/`ai` deriving `[i, m)`.
    fn sym(&mut self, nt: usize, ai: usize, dot: usize, i: usize, m: usize) -> S {
        match self.bnf.alts[nt][ai][dot].clone() {
            BSym::Nt(b) => self.nt(b, i, m),
            ref term => {
                if self.lattice.edge_matches(i, m, term) {
                    S::one()
                } else {
                    S::zero()
                }
            }
        }
    }
}

/// The semiring value of `bnf`'s start rule over every path through
/// `lattice` and every derivation, `weight_of` mapping a rule alternative's
/// weight into the semiring.
fn inside_value<S, W>(bnf: &Bnf, lattice: &Lattice, weight_of: W) -> S
where
    S: Semiring,
    W: Fn(f64) -> S,
{
    let mut inside =
        Inside { bnf, lattice, weight_of: &weight_of, nt_memo: HashMap::new(), seq_memo: HashMap::new() };
    inside.nt(bnf.start, 0, lattice.n)
}

/// One way a run-together string tiles into dictionary words that a grammar
/// accepts: the words in order, the probability of their most probable parse
/// under the grammar's weights, and how many parses they have.
#[derive(Clone, Debug, PartialEq)]
pub struct Tiling {
    /// The words, in order; joined, they are the string.
    pub words: Vec<String>,
    /// The probability of the words' most probable parse.
    pub probability: f64,
    /// How many parses of the words the grammar has.
    pub parses: u64,
}

impl Grammar {
    /// Desugar the EBNF rules to plain BNF for the Earley chart. `None` when
    /// the start rule or a referenced rule is undefined.
    fn to_bnf(&self) -> Option<Bnf> {
        let mut names: HashMap<&str, usize> = HashMap::new();
        for (i, r) in self.rules.iter().enumerate() {
            names.entry(r.name.as_str()).or_insert(i);
        }
        let start = *names.get(self.start.as_str())?;
        let mut alts: Vec<Vec<Vec<BSym>>> = vec![Vec::new(); self.rules.len()];
        let mut weights: Vec<Vec<f64>> = vec![Vec::new(); self.rules.len()];
        for (i, rule) in self.rules.iter().enumerate() {
            let mut rule_alts = Vec::with_capacity(rule.alts.len());
            for alt in &rule.alts {
                let mut seq = Vec::with_capacity(alt.len());
                for sym in alt {
                    seq.push(desugar_sym(sym, &names, &mut weights, &mut alts)?);
                }
                rule_alts.push(seq);
            }
            alts[i] = rule_alts;
            weights[i] = rule.weights.clone();
        }
        Some(Bnf { alts, weights, start })
    }

    /// The semiring value of `input` against the start rule, over every
    /// derivation, computed by one inside chart. `weight_of` maps a rule
    /// alternative's weight to its value in the semiring -- ignoring it
    /// (returning `one`) recovers recognition or a derivation count, and
    /// passing the weight through gives a probability. `None` only when the
    /// grammar references an undefined rule.
    #[must_use]
    pub fn value<S, W>(&self, input: &[u8], weight_of: W) -> Option<S>
    where
        S: Semiring,
        W: Fn(f64) -> S,
    {
        let toks: Vec<Term> = crate::parallel_lex::lex_parallel(input)
            .iter()
            .filter(|t| t.kind != TokenKind::Whitespace)
            .map(|t| Term {
                kind: t.kind,
                text: String::from_utf8_lossy(text(input, t)).into_owned(),
            })
            .collect();
        self.value_over_lattice(&Lattice::linear(&toks), weight_of)
    }

    /// The semiring value of an input lattice against the start rule, over
    /// every path through the lattice and every derivation. A lexed token
    /// stream is the trivial linear lattice; a segmentation lattice
    /// ([`Lattice::segment`]) makes the grammar parse over all tokenizations
    /// at once -- the WFST composition of the grammar with the input.
    #[must_use]
    pub fn value_over_lattice<S, W>(&self, lattice: &Lattice, weight_of: W) -> Option<S>
    where
        S: Semiring,
        W: Fn(f64) -> S,
    {
        let bnf = self.to_bnf()?;
        Some(inside_value(&bnf, lattice, weight_of))
    }

    /// Whether `input` has any derivation against the start rule. Unlike
    /// `parse_input` (one ordered-choice parse), this is recognition over
    /// every derivation -- the boolean-semiring value.
    #[must_use]
    pub fn recognizes(&self, input: &[u8]) -> bool {
        self.value::<Recognize, _>(input, |w| Recognize(w > 0.0)).is_some_and(|r| r.0)
    }

    /// The number of distinct derivations of `input` against the start rule,
    /// saturating. Zero means it does not parse; greater than one means the
    /// grammar is ambiguous for this input.
    #[must_use]
    pub fn count_parses(&self, input: &[u8]) -> u64 {
        self.value::<Count, _>(input, |_| Count::one()).map_or(0, |c| c.0)
    }

    /// The probability of the most probable derivation of `input` under the
    /// grammar's rule weights (the Viterbi value). Zero when it does not
    /// parse. Resolves ambiguity: of the several parses an ambiguous
    /// sentence admits, this scores the best one.
    #[must_use]
    pub fn best_parse_prob(&self, input: &[u8]) -> f64 {
        self.value::<Viterbi, _>(input, Viterbi).map_or(0.0, |v| v.0)
    }

    /// The total probability of `input`: the sum over every derivation of
    /// the product of the rule weights along it (the inside value).
    #[must_use]
    pub fn total_prob(&self, input: &[u8]) -> f64 {
        self.value::<Prob, _>(input, Prob).map_or(0.0, |p| p.0)
    }

    /// The number of ways the run-together `input` segments into dictionary
    /// words such that the start rule parses the result -- the grammar
    /// parsing over the segmentation lattice, so tokenization ambiguity and
    /// parse ambiguity are counted together.
    #[must_use]
    pub fn count_segmentations(&self, input: &str, dict: &[String]) -> u64 {
        let lattice = Lattice::segment(input, dict);
        self.value_over_lattice::<Count, _>(&lattice, |_| Count::one()).map_or(0, |c| c.0)
    }

    /// The probability of the most probable (segmentation, parse) of the
    /// run-together `input` over the dictionary, under the grammar's rule
    /// weights -- the grammar disambiguating the tokenization.
    #[must_use]
    pub fn best_segmentation_prob(&self, input: &str, dict: &[String]) -> f64 {
        let lattice = Lattice::segment(input, dict);
        self.value_over_lattice::<Viterbi, _>(&lattice, Viterbi).map_or(0.0, |v| v.0)
    }

    /// Every tiling of the run-together `input` into the words of `dict`
    /// that the grammar accepts, each with its most probable parse's
    /// probability and its parse count: most probable first, ties in the
    /// order the tilings read from the left, the dictionary's order deciding
    /// between the words that open at one place. The parse counts sum to
    /// [`Self::count_segmentations`] and the first probability is
    /// [`Self::best_segmentation_prob`]. Every tiling is found before the
    /// list is ordered, and how many there are can grow exponentially with
    /// the string's length.
    #[must_use]
    pub fn segment(&self, input: &str, dict: &[String]) -> Vec<Tiling> {
        let Some(bnf) = self.to_bnf() else {
            return Vec::new();
        };
        let lattice = Lattice::segment(input, dict);
        let mut out = Vec::new();
        let mut path: Vec<&LatEdge> = Vec::new();
        // Each place a tiling has reached, with the next of its edges to try;
        // a place past the root was reached by the last edge on `path`.
        let mut places: Vec<(usize, usize)> = vec![(0, 0)];
        while let Some(place) = places.last_mut() {
            let (at, next) = *place;
            if at == lattice.n {
                let toks: Vec<Term> = path.iter().map(|e| Term { kind: e.kind, text: e.text.clone() }).collect();
                let linear = Lattice::linear(&toks);
                let parses = inside_value(&bnf, &linear, |_| Count::one()).0;
                if parses > 0 {
                    let probability = inside_value(&bnf, &linear, Viterbi).0;
                    out.push(Tiling { words: toks.into_iter().map(|t| t.text).collect(), probability, parses });
                }
                places.pop();
                path.pop();
                continue;
            }
            let edges = &lattice.edges[at];
            let Some(edge) = edges.get(next) else {
                places.pop();
                path.pop();
                continue;
            };
            place.1 += 1;
            // A word the dictionary lists twice is one edge, as the
            // segmentation count reads it.
            if edges[..next].iter().any(|e| e.to == edge.to && e.text == edge.text) {
                continue;
            }
            path.push(edge);
            places.push((edge.to, 0));
        }
        out.sort_by(|a, b| b.probability.total_cmp(&a.probability));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_non_left_recursive_grammar_parses_in_bounded_time() {
        // Each level is re-derived once per alternative on failure, so an
        // unmemoized descent costs 2^depth. None of these rules is
        // left-recursive, so every one is cacheable.
        let depth = 22;
        let mut src = String::new();
        for i in 0..depth {
            src.push_str(&format!("r{i} := <r{}> \"a\" | <r{}> \"b\"\n", i + 1, i + 1));
        }
        src.push_str(&format!("r{depth} := \"x\"\n"));
        let g = Grammar::parse(&src).expect("grammar parses");
        let start = std::time::Instant::now();
        assert!(g.parse_input(b"x").is_none(), "no alternative can complete");
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        assert!(ms < 1000.0, "depth-{depth} descent took {ms:.1} ms; memoization is not engaged");
    }

    #[test]
    fn left_recursion_still_grows_under_the_packrat_memo() {
        // The seed memo and the packrat memo answer different questions; a
        // left-recursive rule must keep growing left-associatively and must
        // not be served a stale cached result.
        let g = Grammar::parse(
            "expr := <expr> \"+\" <term> | <term>\nterm := number\n",
        )
        .expect("grammar parses");
        let node = g.parse_input(b"1 + 2 + 3").expect("parses");
        assert_eq!(node.sexpr(), "(expr (expr 1 + 2) + 3)");
    }

    const ARITH: &str = r#"
        expr   := <expr> "+" <term> | <expr> "-" <term> | <term>
        term   := <term> "*" <factor> | <term> "/" <factor> | <factor>
        factor := number | ident | "(" <expr> ")"
    "#;

    fn parse(grammar: &str, input: &str) -> Option<String> {
        Grammar::parse(grammar).unwrap().parse_input(input.as_bytes()).map(|n| n.sexpr())
    }

    #[test]
    fn precedence_is_respected() {
        // Multiplication binds tighter than addition.
        assert_eq!(parse(ARITH, "2 + 3 * 4").unwrap(), "(expr 2 + (term 3 * 4))");
    }

    #[test]
    fn left_associativity() {
        // `2 - 3 - 4` is `(2 - 3) - 4`, not `2 - (3 - 4)`.
        assert_eq!(parse(ARITH, "2 - 3 - 4").unwrap(), "(expr (expr 2 - 3) - 4)");
    }

    #[test]
    fn parenthesized_grouping_overrides_precedence() {
        // The grouping parens are kept as terminals in the factor, so the
        // sum nests under the product where the parentheses put it.
        assert_eq!(parse(ARITH, "(2 + 3) * 4").unwrap(), "(term (factor ( (expr 2 + 3) )) * 4)");
    }

    #[test]
    fn the_same_grammar_parses_identifiers_any_language() {
        // The token grammar does not care which language the identifiers
        // come from; `width * 2 + height` parses the same everywhere.
        assert_eq!(
            parse(ARITH, "width * 2 + height").unwrap(),
            "(expr (term width * 2) + height)"
        );
    }

    #[test]
    fn recursive_call_grammar() {
        let g = r#"
            call := ident "(" <args> ")"
            args := <arg> "," <args> | <arg>
            arg  := <call> | ident | number
        "#;
        // Nested calls parse recursively, language-agnostically.
        let tree = parse(g, "f(g(x), 3)").unwrap();
        assert!(tree.contains("call"), "expected a call node, got {tree}");
        assert!(tree.contains("g"), "expected the nested call, got {tree}");
    }

    #[test]
    fn indirect_left_recursion_left_associates() {
        // `expr` is left-recursive only through `sum` (expr -> sum -> expr),
        // the indirect form the direct-LR check misses. Seed-growing must
        // terminate without a stack overflow and build a left-associative
        // tree, the same shape direct left recursion produces.
        let g = r#"
            expr := <sum>
            sum  := <expr> "+" number | number
        "#;
        assert_eq!(parse(g, "1 + 2 + 3").unwrap(), "(sum (sum 1 + 2) + 3)");
        assert_eq!(parse(g, "7").unwrap(), "7");
        assert!(parse(g, "1 +").is_none());
    }

    #[test]
    fn indirect_left_recursion_through_a_three_rule_cycle() {
        // a -> b -> c -> a: the leftmost reference returns to `a` only after
        // two hops, so the cycle detection must close the full left-corner
        // relation, not just the immediate reference.
        let g = r#"
            a := <b>
            b := <c>
            c := <a> "-" number | number
        "#;
        assert_eq!(parse(g, "9 - 4 - 1").unwrap(), "(c (c 9 - 4) - 1)");
    }

    #[test]
    fn quantifiers_star_plus_opt() {
        // `*`: zero or more.
        assert_eq!(parse("items := number*", "1 2 3").unwrap(), "(items 1 2 3)");
        assert!(parse("items := number*", "x").is_none()); // leftover word unconsumed

        // `+`: one or more.
        assert_eq!(parse("nums := number+", "7 8").unwrap(), "(nums 7 8)");
        assert!(parse("nums := number+", "x").is_none()); // zero matches fails `+`

        // `?`: zero or one.
        assert_eq!(parse("signed := \"-\"? number", "-5").unwrap(), "(signed - 5)");
        assert_eq!(parse("signed := \"-\"? number", "5").unwrap(), "5");
        assert!(parse("signed := \"-\"? number", "5 6").is_none()); // trailing token unconsumed

        // A realistic universal-grammar rule: a call with a run of arguments,
        // expressed directly with `*` instead of a recursive list rule.
        let call = "call := ident \"(\" number* \")\"";
        assert_eq!(parse(call, "f(1 2 3)").unwrap(), "(call f ( 1 2 3 ))");
    }

    #[test]
    fn bare_quantifier_is_a_grammar_error() {
        let e = Grammar::parse("r := *").unwrap_err();
        assert!(e.msg.contains("no symbol to repeat"), "got {}", e.msg);
    }

    #[test]
    fn group_alternation_and_repetition() {
        // Group-internal alternation: `( "a" | "b" )`.
        assert_eq!(parse("x := ( \"a\" | \"b\" )", "a").unwrap(), "a");
        assert_eq!(parse("x := ( \"a\" | \"b\" )", "b").unwrap(), "b");
        assert!(parse("x := ( \"a\" | \"b\" )", "c").is_none());

        // Quantified group: a run of arguments that are numbers OR idents.
        let call = "call := ident \"(\" ( number | ident )* \")\"";
        assert_eq!(parse(call, "f(1 x 2)").unwrap(), "(call f ( 1 x 2 ))");
        assert_eq!(parse(call, "g()").unwrap(), "(call g ( ))");

        // A multi-symbol sequence inside a group, repeated with `+`.
        let pairs = "pairs := ( ident number )+";
        assert_eq!(parse(pairs, "a 1 b 2").unwrap(), "(pairs a 1 b 2)");
        assert!(parse(pairs, "a 1 b").is_none()); // dangling ident without its number
    }

    #[test]
    fn left_recursion_behind_a_nullable_prefix_terminates() {
        // `expr` is left-recursive only through a nullable prefix `<pre>`
        // (pre := "neg"? matches empty), which a first-symbol-only check
        // misses and recurses on forever. Nullability-aware detection routes
        // it to seed-growing, so it terminates and parses in full.
        let g = "expr := <pre> <expr> \"+\" number | number\npre := \"neg\"?";
        let tree = parse(g, "1 + 2 + 3").expect("must parse without stack overflow");
        assert!(tree.contains('+'), "got {tree}");
    }

    #[test]
    fn unterminated_or_stray_group_is_a_grammar_error() {
        assert!(Grammar::parse("r := ( <a>").is_err());
        assert!(Grammar::parse("r := <a> )").is_err());
    }

    #[test]
    fn input_that_does_not_parse_returns_none() {
        // A trailing operator leaves the input unconsumed.
        assert!(parse(ARITH, "2 +").is_none());
    }

    #[test]
    fn unknown_symbol_is_a_grammar_error() {
        let e = Grammar::parse("r := frobnicate").unwrap_err();
        assert!(e.msg.contains("unknown symbol"));
    }

    #[test]
    fn semiring_recognition_matches_the_parser() {
        // The boolean semiring (recognition over all derivations) agrees
        // with parse_input (one ordered-choice parse) on whether the input
        // parses, across the precedence-layered arithmetic grammar.
        let g = Grammar::parse(ARITH).unwrap();
        for input in ["2 + 3 * 4", "(2 + 3) * 4", "width * 2 + height"] {
            assert!(g.recognizes(input.as_bytes()), "{input} should recognize");
            assert!(g.parse_input(input.as_bytes()).is_some());
        }
        for input in ["2 +", "+ 3", ""] {
            assert!(!g.recognizes(input.as_bytes()), "{input} should not recognize");
        }
    }

    #[test]
    fn semiring_counts_ambiguous_derivations() {
        // The classic ambiguous grammar: `+` with no precedence layer. The
        // number of binary parse trees over k operands is the Catalan number
        // C(k-1) -- a quantity the single ordered-choice parse cannot see.
        let g = Grammar::parse("expr := <expr> \"+\" <expr> | number").unwrap();
        assert_eq!(g.count_parses(b"1"), 1);
        assert_eq!(g.count_parses(b"1 + 2"), 1);
        assert_eq!(g.count_parses(b"1 + 2 + 3"), 2); // (1+2)+3 and 1+(2+3)
        assert_eq!(g.count_parses(b"1 + 2 + 3 + 4"), 5); // Catalan(3)
        assert_eq!(g.count_parses(b"1 + 2 + 3 + 4 + 5"), 14); // Catalan(4)
        assert_eq!(g.count_parses(b"1 +"), 0); // does not parse
        // Recognition is exactly "count is nonzero".
        assert_eq!(g.recognizes(b"1 + 2 + 3"), g.count_parses(b"1 + 2 + 3") > 0);
        assert_eq!(g.recognizes(b"1 +"), g.count_parses(b"1 +") > 0);
    }

    #[test]
    fn semiring_counts_through_quantifiers_and_groups() {
        // A quantified group desugars to BNF; the chart counts its
        // derivations. `( ident )*` over k idents has exactly one derivation.
        let g = Grammar::parse("seq := ( ident )*").unwrap();
        assert_eq!(g.count_parses(b"a b c"), 1);
        assert_eq!(g.count_parses(b""), 1); // the empty derivation
        assert_eq!(g.count_parses(b"1"), 0); // a number is not an ident
    }

    #[test]
    fn weighted_grammar_scores_parses() {
        // A symmetric PCFG: the binary rule and the number leaf both weigh
        // 0.5. A derivation's probability is the product of its rule weights.
        let g = Grammar::parse("expr := <expr> \"+\" <expr> @0.5 | number @0.5").unwrap();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        // "1": one number leaf.
        assert!(close(g.total_prob(b"1"), 0.5));
        assert!(close(g.best_parse_prob(b"1"), 0.5));
        // "1 + 2": one parse, binary * number * number = 0.5^3.
        assert!(close(g.total_prob(b"1 + 2"), 0.125));
        // "1 + 2 + 3": two parses, each 0.5^5; total sums them, best is max.
        assert!(close(g.best_parse_prob(b"1 + 2 + 3"), 0.031_25));
        assert!(close(g.total_prob(b"1 + 2 + 3"), 0.062_5));
        // No parse: zero probability.
        assert!(close(g.total_prob(b"1 +"), 0.0));
    }

    #[test]
    fn weighted_grammar_picks_the_more_probable_parse() {
        // Two structurally distinct rules cover the same tokens; the heavier
        // top alternative wins under Viterbi. Total sums both, best takes the
        // larger, count sees two derivations -- one ambiguity, three answers.
        let g = Grammar::parse(
            "s := <add> @0.8 | <mul> @0.2\nadd := number \"x\" number\nmul := number \"x\" number",
        )
        .unwrap();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(g.total_prob(b"1 x 2"), 1.0));
        assert!(close(g.best_parse_prob(b"1 x 2"), 0.8));
        assert_eq!(g.count_parses(b"1 x 2"), 2);
    }

    #[test]
    fn bare_weight_with_no_number_is_a_grammar_error() {
        assert!(Grammar::parse("r := number @x").is_err());
    }

    #[test]
    fn grammar_parses_over_a_segmentation_lattice() {
        let dict: Vec<String> = ["a", "b", "ab"].iter().map(|s| (*s).to_string()).collect();
        // A run of words: every tiling of the run-together string into
        // dictionary words is a path the grammar accepts. "abab" tiles four
        // ways with {a, b, ab}: a-b-a-b, ab-a-b, a-b-ab, ab-ab.
        let words = Grammar::parse("s := <s> ident | ident").unwrap();
        assert_eq!(words.count_segmentations("abab", &dict), 4);
        assert_eq!(words.count_segmentations("aa", &dict), 1); // only a-a
        assert_eq!(words.count_segmentations("abc", &dict), 0); // "c" uncoverable

        // The grammar disambiguates the tokenization: one that accepts only
        // "ab" tokens picks the single all-"ab" tiling out of the four.
        let only_ab = Grammar::parse("s := \"ab\" <s> | \"ab\"").unwrap();
        assert_eq!(only_ab.count_segmentations("abab", &dict), 1);
        assert!(only_ab.best_segmentation_prob("abab", &dict) > 0.0);
    }

    #[test]
    fn the_tilings_a_grammar_accepts_are_listed_most_probable_first() {
        // "a" listed twice is one word; the weights are exact in binary, so
        // the tilings of equal probability tie exactly.
        let dict: Vec<String> = ["a", "b", "ab", "a"].iter().map(|s| (*s).to_string()).collect();
        // Words joined in any bracketing: n words have Catalan(n - 1) parses.
        let g = Grammar::parse("s := <s> <s> | <w>\nw := \"ab\" @0.5 | \"a\" @0.25 | \"b\" @0.25").unwrap();
        let tilings = g.segment("abab", &dict);
        let words: Vec<String> = tilings.iter().map(|t| t.words.join("-")).collect();
        assert_eq!(words, ["ab-ab", "a-b-ab", "ab-a-b", "a-b-a-b"]);
        assert_eq!(tilings.iter().map(|t| t.parses).collect::<Vec<_>>(), [1, 2, 2, 5]);
        assert_eq!(tilings.iter().map(|t| t.probability).collect::<Vec<_>>(), [0.25, 0.03125, 0.03125, 0.00390625]);
        // The listing agrees with the count and the best probability, which
        // read the same lattice without walking its paths.
        assert_eq!(tilings.iter().map(|t| t.parses).sum::<u64>(), g.count_segmentations("abab", &dict));
        assert_eq!(tilings[0].probability, g.best_segmentation_prob("abab", &dict));
        assert!(g.segment("abc", &dict).is_empty());
    }
}
