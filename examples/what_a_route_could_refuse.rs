//! What an opening-kind route can refuse, read off a real corpus.
//!
//! `kind_route::first_kinds` names the token kinds a match can begin with, and
//! the sweep skips an anchor whose kind is not among them. Whether widening
//! that route is worth anything turns on two counts the source does not carry:
//! the anchors it refuses today, and the anchors it would refuse if it named
//! exactly the kinds that do open a match here.
//!
//! The second is measured rather than derived. Every match the scan reports is
//! resolved to the token its first byte lands on, and those kinds are the
//! shape's openings over these bytes. That count is a ceiling: no opening-kind
//! route can refuse more and still answer the same spans.
//!
//! A route that fails to admit one of those kinds is a route that loses a
//! match, so the `unsound` column is a check rather than a reading. It is empty
//! for a sound route and it is printed for every shape, so that a change to the
//! route is tested over real input before it is believed.
//!
//! A ceiling is only a saving where the route is read at all, so every shape
//! says which engine answers it. The sweep reads the route; a shape a byte
//! route answers, or one the single-pass engine takes, never reaches
//! `per_start_matches`, and its refusal count is an arithmetic rather than
//! time any scan would have spent. The distinction is not cosmetic: every
//! literal-led shape on `benches/engine_surface` is taken by the single-pass
//! engine, so the surface cannot show what a route for a literal is worth and a
//! table without this column reads as though it could.
//!
//! A ceiling read here is a fact about the bytes it was read over, so the
//! header names them and the harness takes several files rather than one.
//!
//! Run: `cargo run --release --example what_a_route_could_refuse -- <file>...`

use std::collections::BTreeMap;
use trex::ast::Pattern;
use trex::kind_route::{OpeningKinds, first_kinds};
use trex::token::TokenKind;

/// `benches/engine_surface`'s shapes exactly, so a count here and a duration
/// there describe the same pattern.
const SHAPES: [(&str, &str); 26] = [
    ("a balanced group", "\\B"),
    ("a word then a balanced group", "\\W \\B"),
    ("a balanced group with an interior", "\\B(\\W)"),
    ("a balanced group of one bracket kind", "\\B(\\W \"=\")"),
    ("a star", "\"let\" \\W* \"=\""),
    ("a plus", "\"let\" \\W+ \"=\""),
    ("an unequal bounded repeat", "\\W{2,4}"),
    ("an open repeat", "\\W{2,}"),
    ("an alternation of kinds", "(\\N | \\W) \"=\""),
    ("an alternation, first-match", "(\\N |> \\W) \"=\""),
    ("an atomic group", "(?>\\W*) \"=\""),
    ("a lazy star", "\"let\" \\W*? \"=\""),
    ("a back-reference", "\\W:a \"=\" =a"),
    ("a back-reference up to case", "\\W:a \"=\" =case a"),
    ("a back-reference within one edit", "\\W:a \"=\" =edit1 a"),
    ("a sub-pattern assertion", "\\W ~(\"=\" \\N)"),
    ("a negative assertion", "\\W !~(\"=\" \\Q)"),
    ("a magnitude atom", "\\M{>3} \"=\""),
    ("a spectral atom", "\\F{entropy>0.7}"),
    ("a seam anchor", "@seam \\W"),
    ("an echo anchor", "@echoed \\W"),
    ("a nesting anchor", "@nested>0 \\W"),
    ("a construct anchor", "@super:assign \\W"),
    ("an edit-distance group", "(\"let\" \\W \"=\")~1"),
    ("a rarity anchor", "@shape:rare \\W"),
    ("a timestamp order anchor", "@order:desc \\N"),
];

/// Literal-led shapes the single-pass engine declines, so the sweep answers
/// them and an opening-kind route is read at all.
///
/// The surface above has none. Every literal-led shape on it - the star, the
/// plus, the lazy star - is taken by the single-pass engine, which reads no
/// route, so a route for a literal cannot move one of them however many anchors
/// it could name. These pair a leading literal with a construct that engine
/// does not take, which is what a caller writing `"ERROR" ...` over a log gets
/// as soon as the rest of the pattern needs the set engine.
const LITERAL_LED: [(&str, &str); 3] = [
    ("a literal then an atomic group", "\"let\" (?>\\W*) \"=\""),
    ("a literal then a negative assertion", "\"let\" \\W !~(\"=\" \\Q)"),
    ("a literal then a spectral atom", "\"let\" \\F{entropy>0.7}"),
];

/// The built-in kind codes, which are contiguous from zero and end below the
/// first custom one. A code past this range belongs to a declared shape, whose
/// name is a property of the shape set rather than of the kind.
const BUILTIN_CODES: std::ops::RangeInclusive<u32> = 0..=32;

/// Which engine answers `pattern` over `input`.
///
/// An opening-kind route is read by the sweep and by nothing else, so this is
/// what decides whether a ceiling below is a saving or an arithmetic. A shape a
/// route answers, or one the single-pass engine takes, never reaches
/// `per_start_matches`, and the anchors a route "would" refuse there are
/// anchors nothing was going to spend.
fn reaches(pattern: &Pattern, input: &[u8]) -> &'static str {
    if trex::engine::routed_spans_public(pattern, input).is_some() {
        "route"
    } else if trex::nfa::scan_nfa(pattern, input).is_some() {
        "one-pass"
    } else {
        "sweep"
    }
}

/// The kinds `set` admits, named.
fn kinds_named(set: OpeningKinds) -> String {
    let mut out: Vec<String> = Vec::new();
    for code in BUILTIN_CODES {
        let kind = TokenKind::from_code(code);
        if set.admits(kind) {
            out.push(kind.name().to_string());
        }
    }
    out.join(" ")
}

/// One corpus against every shape.
fn report(path: &str, input: &[u8], shapes: &[(&str, &str, Pattern)]) {
    let toks = trex::lexer::lex(input);

    // The anchors a sweep offers are every token but the whitespace it skips,
    // counted per kind: a refusal count is then a sum over kinds rather than a
    // walk over tokens, and the two agree because the route reads only the kind.
    let mut per_kind: BTreeMap<u32, (TokenKind, usize)> = BTreeMap::new();
    let mut starts: Vec<(usize, TokenKind)> = Vec::with_capacity(toks.len());
    for t in &toks {
        starts.push((t.start(), t.kind));
        if t.kind != TokenKind::Whitespace {
            per_kind.entry(t.kind.code()).or_insert((t.kind, 0)).1 += 1;
        }
    }
    let attempts: usize = per_kind.values().map(|(_, n)| n).sum();

    println!(
        "\ncorpus {path}, {} bytes, {} tokens, {attempts} anchors",
        input.len(),
        toks.len()
    );
    println!(
        "{:>36} {:>8} {:>8} {:>9} {:>6} {:>9} {:>6}  route today / opens on",
        "shape", "answers", "matches", "refused", "", "ceiling", ""
    );

    let mut refused_all = 0usize;
    let mut ceiling_all = 0usize;
    let mut refused_swept = 0usize;
    let mut ceiling_swept = 0usize;
    let mut swept = 0usize;
    for (label, src, pat) in shapes {
        let spans = trex::scan(pat, input);
        let today = first_kinds(pat);
        let answers = reaches(pat, input);

        // The kinds that in fact open a match here, taken from the token each
        // match's first byte lands on. Matches and tokens both ascend, so one
        // walk resolves every match; a match that lands on no token start is
        // counted and reported, since it would mean the scan and the lex
        // disagree about where a token begins.
        let mut opens: BTreeMap<u32, TokenKind> = BTreeMap::new();
        let mut unresolved = 0usize;
        let mut at = 0usize;
        for s in &spans {
            while at < starts.len() && starts[at].0 < s.start() {
                at += 1;
            }
            if at < starts.len() && starts[at].0 == s.start() {
                opens.insert(starts[at].1.code(), starts[at].1);
            } else {
                unresolved += 1;
            }
        }

        let refused: usize = per_kind
            .values()
            .filter(|(k, _)| today.is_some_and(|set| !set.admits(*k)))
            .map(|(_, n)| n)
            .sum();
        let ceiling: usize = per_kind
            .values()
            .filter(|(k, _)| !opens.contains_key(&k.code()))
            .map(|(_, n)| n)
            .sum();
        let unsound: Vec<String> = opens
            .values()
            .filter(|k| today.is_some_and(|set| !set.admits(**k)))
            .map(|k| k.name().to_string())
            .collect();
        refused_all += refused;
        ceiling_all += ceiling;
        if answers == "sweep" {
            refused_swept += refused;
            ceiling_swept += ceiling;
            swept += 1;
        }

        let share = |n: usize| {
            if attempts == 0 { 0.0 } else { n as f64 * 100.0 / attempts as f64 }
        };
        let route = today.map_or_else(|| "-".to_string(), kinds_named);
        let opens_named: Vec<String> = opens.values().map(|k| k.name().to_string()).collect();
        // Whether a literal-position filter could reach this shape at all. A
        // required literal is one the input must hold for a match to exist, so
        // its occurrences bound where a match can begin - and a shape that
        // requires none cannot be filtered that way whatever the search costs.
        // `collect_required_literals` already declines the cases that would
        // make it wrong: an orbit-scoped literal, an assertion satisfied by
        // absence, a counted repetition.
        let requires = trex::prefilter::required_literal_count(pat);
        println!(
            "{label:>36} {answers:>8} {:>8} {refused:>9} {:>5.1}% {ceiling:>9} {:>5.1}% {requires:>4} lit  [{route}] opens [{}]",
            spans.len(),
            share(refused),
            share(ceiling),
            opens_named.join(" ")
        );
        if unresolved > 0 {
            println!("{:>36}  {unresolved} matches began at no token start", "");
        }
        if !unsound.is_empty() {
            println!(
                "{:>36}  {src} opens on {} and the route refuses it, so the route loses a match",
                "",
                unsound.join(" ")
            );
        }
    }

    let offered = attempts * shapes.len();
    let share = |n: usize, of: usize| {
        if of == 0 { 0.0 } else { n as f64 * 100.0 / of as f64 }
    };
    println!(
        "{:>36} {:>8} {:>8} {refused_all:>9} {:>5.1}% {ceiling_all:>9} {:>5.1}%  of {offered} offered",
        "all of it",
        "",
        "",
        share(refused_all, offered),
        share(ceiling_all, offered)
    );
    // The line that says what a route is worth. Only a shape the sweep answers
    // reads an opening-kind route at all, so a refusal counted against one a
    // route or the single-pass engine answers is an anchor nothing would have
    // tried.
    let swept_offered = attempts * swept;
    println!(
        "{:>36} {:>8} {:>8} {refused_swept:>9} {:>5.1}% {ceiling_swept:>9} {:>5.1}%  of {swept_offered} the sweep is asked",
        "of that, what the sweep reads",
        "",
        "",
        share(refused_swept, swept_offered),
        share(ceiling_swept, swept_offered)
    );
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!(
            "name one or more corpus files: a ceiling is a fact about the bytes it was read over"
        );
        std::process::exit(2);
    }

    // Parsed once, before any corpus is read, so a typo in a shape is reported
    // without waiting for a scan.
    let mut shapes: Vec<(&str, &str, Pattern)> = Vec::new();
    for (label, src) in SHAPES.iter().chain(LITERAL_LED.iter()).copied() {
        match trex::parse(src) {
            Ok(p) => shapes.push((label, src, p)),
            Err(e) => {
                eprintln!("{label}: {src} does not parse: {e:?}");
                std::process::exit(2);
            }
        }
    }

    println!("trex {}", env!("CARGO_PKG_VERSION"));
    for path in &paths {
        match std::fs::read(path) {
            Ok(input) => report(path, &input, &shapes),
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        }
    }
}
