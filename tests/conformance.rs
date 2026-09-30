//! Randomized differential conformance: on the subset where trex and a
//! regular expression mean the same thing, they must return the same spans.
//!
//! `differential.rs` pairs patterns by hand, so it can only find gaps someone
//! already suspected. This generates a pattern once, as an abstract tree, and
//! renders it to both syntaxes, so the two sides are equivalent by
//! construction rather than by a human having matched them up.
//!
//! ## The grain mismatch, and how the rendering handles it
//!
//! trex concatenates over tokens with whitespace between them; a regular
//! expression concatenates over bytes. A separator cannot simply be placed
//! between rendered nodes, because a nullable quantifier changes how many
//! separators there are: in `X A* Y`, zero `A` leaves one space and one `A`
//! leaves two.
//!
//! So the corpus gives every token exactly one trailing space, including the
//! last, and each regex atom is rendered with that separator attached. `A*`
//! then renders as `(?:<A> )*` and stays correct however it nests. trex
//! reports spans without the trailing whitespace and the regex reports them
//! with it, so the regex span is trimmed before comparison.
//!
//! ## What is deliberately outside the subset
//!
//! Rust's `regex` is leftmost-first, which is trex's default `|`. The other
//! two alternation modes have no oracle here, so they are not generated,
//! rather than generated and compared loosely.
//!
//! Every generated pattern must parse on both sides. A parse failure is a
//! finding - either the generator left the shared subset or trex cannot read
//! something it should - so it fails the run with its seed instead of being
//! skipped.
//!
//! ## The oracle, and both engines
//!
//! The oracle is the regex crate's PikeVM, the reference machine its other
//! engines are checked against, driven directly. The crate's front door is
//! run as well and its disagreements with the PikeVM are counted and printed
//! rather than asserted: its literal prefilters can report a match that is
//! not the leftmost one, which the PikeVM never does.
//!
//! Every case runs through the single-pass engine, which is what `trex::scan`
//! uses for the whole shared subset, and through the set-reachability engine,
//! which handles the constructs beyond it. The single-pass engine's spans are
//! the assertion; the set engine's disagreements with the oracle are counted
//! and printed, so a run reports where the two engines part company on the
//! same patterns.

use regex::Regex;
use regex_automata::nfa::thompson::pikevm::{Cache, PikeVM};

/// A pattern in the shared subset, renderable to either syntax.
#[derive(Clone, Debug)]
enum Node {
    /// A whole-token literal word.
    Word(String),
    /// Any word token (`\W`).
    AnyWord,
    /// Any number token (`\N`).
    AnyNum,
    /// Any significant token (`.`).
    Any,
    Concat(Vec<Node>),
    /// Leftmost-first alternation only.
    Alt(Vec<Node>),
    Star(Box<Node>, bool),
    Plus(Box<Node>, bool),
    Opt(Box<Node>, bool),
    /// `{m,n}`; `None` is an open upper bound.
    Repeat(Box<Node>, usize, Option<usize>, bool),
}

impl Node {
    /// Whether this matches a single token, so a quantifier over it needs no
    /// grouping in trex's surface syntax.
    fn is_atom(&self) -> bool {
        matches!(self, Node::Word(_) | Node::AnyWord | Node::AnyNum | Node::Any)
    }

    /// Whether this can match no tokens at all. Such a pattern has no
    /// leftmost-first agreement to test: trex reports no zero-width matches
    /// and a regex reports one at every position.
    fn nullable(&self) -> bool {
        match self {
            Node::Word(_) | Node::AnyWord | Node::AnyNum | Node::Any => false,
            Node::Concat(v) => v.iter().all(Node::nullable),
            Node::Alt(v) => v.iter().any(Node::nullable),
            Node::Star(..) | Node::Opt(..) => true,
            Node::Plus(p, _) => p.nullable(),
            Node::Repeat(p, m, _, _) => *m == 0 || p.nullable(),
        }
    }

    fn trex(&self) -> String {
        let lazy = |l: bool| if l { "?" } else { "" };
        // A quantifier binds to one atom, so anything larger is parenthesised.
        let group =
            |n: &Node| if n.is_atom() { n.trex() } else { format!("({})", n.trex()) };
        match self {
            Node::Word(w) => format!("\"{w}\""),
            Node::AnyWord => "\\W".to_string(),
            Node::AnyNum => "\\N".to_string(),
            Node::Any => ".".to_string(),
            Node::Concat(v) => v.iter().map(Node::trex).collect::<Vec<_>>().join(" "),
            Node::Alt(v) => {
                format!("({})", v.iter().map(Node::trex).collect::<Vec<_>>().join(" | "))
            }
            Node::Star(p, l) => format!("{}*{}", group(p), lazy(*l)),
            Node::Plus(p, l) => format!("{}+{}", group(p), lazy(*l)),
            Node::Opt(p, l) => format!("{}?{}", group(p), lazy(*l)),
            Node::Repeat(p, m, n, l) => {
                let bound = match n {
                    Some(n) => format!("{m},{n}"),
                    None => format!("{m},"),
                };
                format!("{}{{{}}}{}", group(p), bound, lazy(*l))
            }
        }
    }

    /// The regex rendering. Every atom carries its own trailing separator, so
    /// a nullable quantifier does not leave a stray space behind.
    fn regex(&self) -> String {
        let lazy = |l: bool| if l { "?" } else { "" };
        match self {
            Node::Word(w) => format!("(?:{w} )"),
            Node::AnyWord => "(?:[A-Za-z_]+ )".to_string(),
            Node::AnyNum => "(?:[0-9]+ )".to_string(),
            Node::Any => "(?:[A-Za-z_0-9]+ )".to_string(),
            Node::Concat(v) => v.iter().map(Node::regex).collect::<String>(),
            Node::Alt(v) => {
                format!("(?:{})", v.iter().map(Node::regex).collect::<Vec<_>>().join("|"))
            }
            Node::Star(p, l) => format!("(?:{})*{}", p.regex(), lazy(*l)),
            Node::Plus(p, l) => format!("(?:{})+{}", p.regex(), lazy(*l)),
            Node::Opt(p, l) => format!("(?:{})?{}", p.regex(), lazy(*l)),
            Node::Repeat(p, m, n, l) => {
                let bound = match n {
                    Some(n) => format!("{m},{n}"),
                    None => format!("{m},"),
                };
                format!("(?:{}){{{}}}{}", p.regex(), bound, lazy(*l))
            }
        }
    }
}

/// xorshift32, so a failing case reproduces from its printed seed.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n.max(1)
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u32) as usize]
    }
}

/// The vocabulary both sides draw on. Kept small so patterns actually hit.
const WORDS: [&str; 4] = ["foo", "bar", "baz", "qux"];

fn gen_node(rng: &mut Rng, depth: u32) -> Node {
    let choice = if depth == 0 { rng.below(4) } else { rng.below(10) };
    match choice {
        0 => Node::Word((*rng.pick(&WORDS)).to_string()),
        1 => Node::AnyWord,
        2 => Node::AnyNum,
        3 => Node::Any,
        4 | 5 => {
            let n = 2 + rng.below(2) as usize;
            Node::Concat((0..n).map(|_| gen_node(rng, depth - 1)).collect())
        }
        6 => {
            let n = 2 + rng.below(2) as usize;
            Node::Alt((0..n).map(|_| gen_node(rng, depth - 1)).collect())
        }
        7 => Node::Star(Box::new(gen_node(rng, depth - 1)), rng.below(2) == 1),
        8 => Node::Plus(Box::new(gen_node(rng, depth - 1)), rng.below(2) == 1),
        _ => {
            let inner = Box::new(gen_node(rng, depth - 1));
            let l = rng.below(2) == 1;
            match rng.below(3) {
                0 => Node::Opt(inner, l),
                1 => {
                    let m = rng.below(3) as usize;
                    Node::Repeat(inner, m, Some(m + rng.below(3) as usize), l)
                }
                _ => Node::Repeat(inner, rng.below(2) as usize, None, l),
            }
        }
    }
}

/// A corpus of single-space-separated tokens, with a trailing space so every
/// token is followed by exactly one separator.
///
/// The rendering assumes one token per separated run, so a corpus the lexer
/// reads differently would compare two patterns that do not mean the same
/// thing. That assumption is checked rather than trusted: trex has composite
/// atoms that span separators (a card number in an issuer's grouping, a
/// `+`-prefixed phone), so a corpus that does not lex one-to-one is
/// regenerated.
fn gen_input(rng: &mut Rng) -> String {
    for _ in 0..64 {
        let n = 1 + rng.below(10);
        let mut s = String::new();
        for _ in 0..n {
            if rng.below(3) == 1 {
                s.push_str(&rng.below(1000).to_string());
            } else {
                s.push_str(rng.pick(&WORDS));
            }
            s.push(' ');
        }
        let toks = trex::lexer::lex(s.as_bytes());
        let sig: Vec<_> = toks.iter().filter(|t| t.is_significant()).collect();
        if sig.len() == n as usize && sig.iter().all(|t| !s[t.span()].contains(' ')) {
            return s;
        }
    }
    // Words alone are never fused, so this always lexes one-to-one.
    let mut s = String::new();
    for _ in 0..3 {
        s.push_str(rng.pick(&WORDS));
        s.push(' ');
    }
    s
}

/// A span with its trailing separator trimmed, so it is comparable to trex's,
/// which never includes it; `None` when nothing is left.
fn trimmed(input: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let end = start + input[start..end].trim_end().len();
    (end > start).then_some((start, end))
}

/// The front door's spans, trimmed.
fn regex_spans(re: &Regex, input: &str) -> Vec<(usize, usize)> {
    re.find_iter(input).filter_map(|m| trimmed(input, m.start(), m.end())).collect()
}

/// The PikeVM's spans, trimmed.
fn pikevm_spans(vm: &PikeVM, cache: &mut Cache, input: &str) -> Vec<(usize, usize)> {
    vm.find_iter(cache, input).filter_map(|m| trimmed(input, m.start(), m.end())).collect()
}

/// trex's matches as byte spans.
fn spans(matches: &[trex::Span]) -> Vec<(usize, usize)> {
    matches.iter().map(|m| (m.start(), m.end())).collect()
}

/// A sweep size from the environment, or its shipped default. The defaults
/// are what trex clears, and raising either from the command line is how a
/// wider search is run; a value that is present but not a count is a
/// mistake in the invocation and fails rather than being read as absent.
fn knob(name: &str, default: u32) -> u32 {
    match std::env::var(name) {
        Ok(v) => v.parse().unwrap_or_else(|e| panic!("{name}={v:?} is not a count: {e}")),
        Err(std::env::VarError::NotPresent) => default,
        Err(e) => panic!("{name}: {e}"),
    }
}

#[test]
fn trex_agrees_with_a_regular_expression_on_the_shared_subset() {
    // The count and depth record the frontier trex clears, not what the
    // generator can produce; `TREX_CONFORMANCE_CASES` and
    // `TREX_CONFORMANCE_DEPTH` raise them for a run, and a failure prints its
    // seed so the case reproduces. Lowering them to keep the gate green
    // would hide what they record.
    let cases = knob("TREX_CONFORMANCE_CASES", 20_000);
    let depth = knob("TREX_CONFORMANCE_DEPTH", 6);
    let mut rng = Rng(0x1234_5678);
    let mut compared = 0usize;
    let mut nullable = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut set_disagreements: Vec<String> = Vec::new();
    let mut front_door_disagreements: Vec<String> = Vec::new();

    for case in 0..cases {
        let seed = rng.0;
        let node = gen_node(&mut rng, depth);
        if node.nullable() {
            nullable += 1;
            continue;
        }
        let tp = node.trex();
        let rp = node.regex();

        let vm = match PikeVM::new(&rp) {
            Ok(vm) => vm,
            Err(e) => panic!("case {case} seed {seed:#x}: oracle rejected {rp:?}: {e}"),
        };
        let mut cache = vm.create_cache();
        let re = match Regex::new(&rp) {
            Ok(re) => re,
            Err(e) => panic!("case {case} seed {seed:#x}: the front door rejected {rp:?}: {e}"),
        };
        let parsed = match trex::parse(&tp) {
            Ok(p) => p,
            Err(e) => panic!("case {case} seed {seed:#x}: trex rejected {tp:?}: {e:?}"),
        };

        for _ in 0..8 {
            let input = gen_input(&mut rng);
            let bytes = input.as_bytes();
            let want = pikevm_spans(&vm, &mut cache, &input);
            let door = regex_spans(&re, &input);
            let toks = trex::lexer::lex(bytes);
            let set = spans(&trex::engine::scan_tokens_from(&parsed, bytes, &toks, 0));
            let got = trex::nfa::scan_nfa(&parsed, bytes).map(|m| spans(&m));
            let shown = format!(
                "case {case} seed {seed:#x}\n  trex   {tp}\n  regex  {rp}\n  input  {input:?}\n  oracle {want:?}\n  front door {door:?}\n  single-pass {got:?}\n  set    {set:?}"
            );
            // The single-pass engine handles the whole shared subset, so a
            // declined pattern is as much a finding as a wrong span.
            if got.as_ref() != Some(&want) {
                failures.push(shown.clone());
            }
            if set != want {
                set_disagreements.push(shown.clone());
            }
            if door != want {
                front_door_disagreements.push(shown);
            }
            compared += 1;
        }
    }

    println!(
        "{compared} comparisons over {cases} cases at depth {depth} ({nullable} nullable cases skipped): \
         single-pass engine disagrees with the oracle on {}, set engine on {}, the regex crate's front door on {}",
        failures.len(),
        set_disagreements.len(),
        front_door_disagreements.len()
    );
    for d in &set_disagreements {
        println!("set engine disagrees:\n{d}");
    }
    for d in &front_door_disagreements {
        println!("front door disagrees:\n{d}");
    }

    // A generator that produced nothing would pass every assertion above
    // without testing anything, which is the failure this harness exists to
    // rule out. About a quarter of the cases are nullable and skipped at
    // every depth run, and every kept case compares eight inputs.
    assert!(
        compared >= cases as usize * 2,
        "the sweep must actually compare cases: {compared} compared, {nullable} nullable"
    );
    assert!(
        failures.is_empty(),
        "{} of {compared} comparisons disagree with the oracle:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
