//! Whether a typed atom over a corpus holding none of its kind is now refused
//! from the bytes, and what a scan of it costs.
//!
//! The change this checks: `prefilter::kind_required_literal` names, for each
//! kind whose recognizer refuses without a fixed byte string, that string, so
//! `requires_absent` can refuse the whole input with one
//! `byte_simd::contains` and no lex. That refusal is read at the top of ten
//! call sites, so a pattern that is one typed atom reaches it from every
//! entry point.
//!
//! Measuring the parts said the refusal is cheap. It did not say the refusal
//! fires. A literal named in a table that no scan consults would measure
//! exactly as fast and answer exactly as slowly, so this asks the scan.
//!
//! ## What would make it wrong
//!
//! Two failures, and they are not symmetric.
//!
//! A refusal on an input that holds a token of the kind loses matches, and is
//! the one that must never happen. Every row asserts that a refused input is
//! one the full scan also finds nothing in - the scan is run either way, so
//! the assertion is over the answer rather than over the reasoning.
//!
//! A missed refusal on an input holding none of the kind costs a lex and
//! answers correctly. It is reported rather than asserted, because a kind
//! whose literal the corpus happens to hold for another reason - a `-` in
//! prose, a `.` in a version-free log - is not a fault, it is the filter
//! being a filter.
//!
//! ## What the timing says and does not say
//!
//! `scan ms` is the whole scan through `trex::scan`, and `lex par ms` is what
//! `parallel_lex::lex_parallel` costs over the same bytes - the floor a scan
//! that must lex cannot go below. A refused row should be far under that
//! floor, and a row that is not is a row where the refusal did not fire
//! whatever the table says.
//!
//! Run: `cargo run --release --example does_the_typed_atom_short_circuit -- <file>...`

use std::time::Instant;

use trex::token::TokenKind;

/// The typed atoms whose kind now names a required literal, as the surface
/// spells them, with the kind each one asks for.
const ATOMS: [(&str, TokenKind); 10] = [
    ("\\E", TokenKind::Email),
    ("\\U", TokenKind::Url),
    ("\\{jwt}", TokenKind::Jwt),
    ("\\%", TokenKind::Percent),
    ("\\H", TokenKind::HexColor),
    ("\\V", TokenKind::Version),
    ("\\C", TokenKind::Cidr),
    ("\\{uuid}", TokenKind::Uuid),
    ("\\$", TokenKind::Money),
    ("\\{geo}", TokenKind::Geo),
];

/// The atoms whose kind names a required run rather than a literal.
///
/// None names a byte string, so the literal table cannot reach them, and an
/// unanchored automaton for base64 or a digest is combinatorial in live
/// starting positions. What each does have is a run of one byte class every
/// token of it must hold, which `requires_absent_run` reads through the nibble
/// tables.
///
/// `\{mac}` is here rather than among the atoms below because a Mac is six
/// pairs of hex digits joined by five copies of one separator - always
/// seventeen bytes, every one a hex digit or a separator - so unlike the other
/// kinds with alternative forms it does name a run. Ip does not: an IPv6
/// address may be `::1`, three bytes of a class most words clear.
const RUN_ATOMS: [(&str, TokenKind); 3] = [
    ("\\{base64}", TokenKind::Base64),
    ("\\D", TokenKind::HashDigest),
    ("\\{mac}", TokenKind::Mac),
];

/// The atoms deliberately left out of both tables, which must still be
/// answered by a lex.
///
/// Read here rather than assumed: each has alternative forms and so requires
/// no single literal and no single run, and a row of one of these that
/// reported a refusal would mean a table had grown an entry that loses
/// matches. A kind is in exactly one of the three lists.
const NOT_IN_THE_TABLE: [(&str, TokenKind); 4] = [
    ("\\I", TokenKind::Ip),
    ("\\L", TokenKind::Path),
    ("\\T", TokenKind::Timestamp),
    ("\\{phone}", TokenKind::Phone),
];

/// The window route's gates, each timed where it admits the route: the shipped
/// one counts windows against the input's length, and the others are kept so
/// the gates can be measured against each other. `open` is the route with no
/// gate at all, so a row every gate declines still shows what the route would
/// have cost there.
const GATES: [(&str, trex::prefilter::WindowGate); 5] = [
    ("count", trex::prefilter::WindowGate::ByteCount),
    ("coverage", trex::prefilter::WindowGate::CoverageOnly),
    ("priced", trex::prefilter::WindowGate::PricedWindow),
    ("density", trex::prefilter::WindowGate::PredictedDensity),
    ("open", trex::prefilter::WindowGate::Open),
];

/// Milliseconds one call takes, with what it returned.
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let started = Instant::now();
    let out = f();
    (out, started.elapsed().as_secs_f64() * 1000.0)
}

/// One corpus against every atom.
fn report(path: &str, input: &[u8]) {
    let toks = trex::lexer::lex(input);
    let (_, lex_par_ms) = timed(|| trex::parallel_lex::lex_parallel(input));
    // The window route reads every quote in the input before its first
    // window, whatever the windows hold, so each taken row's time includes
    // this cost in full.
    let (quotes, quote_ms) = timed(|| trex::parallel_lex::quoted_spans(input));

    println!(
        "\ncorpus {path}, {} bytes, {} tokens, lex par {lex_par_ms:.2} ms, quote scan {quote_ms:.2} ms over {} strings",
        input.len(),
        toks.len(),
        quotes.len()
    );
    println!(
        "{:>10} {:>9} {:>9} {:>10} {:>10} {:>12}  verdict; windows under each gate, and against the scan",
        "atom", "in lexer", "refused", "matches", "scan ms", "vs lex par"
    );

    for (src, kind, refusable) in ATOMS
        .iter()
        .map(|(s, k)| (*s, *k, true))
        .chain(RUN_ATOMS.iter().map(|(s, k)| (*s, *k, true)))
        .chain(NOT_IN_THE_TABLE.iter().map(|(s, k)| (*s, *k, false)))
    {
        let pattern = match trex::parse(src) {
            Ok(p) => p,
            Err(e) => {
                println!("{src:>10}  does not parse: {e:?}");
                continue;
            }
        };

        let held = toks.iter().filter(|t| t.kind == kind).count();
        let refused = trex::prefilter::requires_absent(&pattern, input);
        // The scan is what ships and is timed; the answer it is held to comes
        // from the single-pass engine over a whole lex, a route the scan's
        // ladder never takes for these atoms, so the windows inside the scan
        // are checked against something they are not.
        let (spans, scan_ms) = timed(|| trex::scan(&pattern, input));
        let Some(engine) = trex::nfa::scan_nfa(&pattern, input) else {
            println!("{src:>10}  the single-pass engine does not take it, so this row has no oracle");
            continue;
        };
        assert!(
            spans == engine,
            "{src} on {path}: the scan found {} matches where the engine found {}",
            spans.len(),
            engine.len()
        );

        // The one that must never happen: a refusal on an input the scan finds
        // something in. Asserted over the answer rather than over the table,
        // because the table is what is being checked.
        assert!(
            !refused || spans.is_empty(),
            "{src} was refused from the bytes on {path} and yet the scan found {} matches",
            spans.len()
        );
        // A kind left out of both tables must not have grown an entry.
        assert!(
            refusable || !refused,
            "{src} names neither a required literal nor a required run, and yet was refused on {path}"
        );

        // The windows the kind's required literal opens, lexed in place of the
        // input under each gate, timed beside the scan and held to its answer:
        // a window that loses a match fails the run. A gate that declines
        // prints a dash and what deciding cost, since a rung that hands the
        // input back still charges the scan that follows for the decision;
        // the shipped gate's reason is printed beside it.
        let mut windows = String::new();
        for (name, gate) in GATES {
            let (windowed, window_ms) =
                timed(|| trex::prefilter::scan_required_windows_gated(&pattern, input, gate));
            match windowed {
                Some(w) => {
                    assert!(
                        w == spans,
                        "{src} on {path} under {name}: the windows found {} matches where the scan found {}",
                        w.len(),
                        spans.len()
                    );
                    windows.push_str(&format!(" {name} {window_ms:.2} ms {:.2}x", scan_ms / window_ms));
                }
                None => windows.push_str(&format!(" {name} - ({window_ms:.2} ms to decline)")),
            }
        }
        if trex::prefilter::scan_required_windows(&pattern, input).is_none() {
            windows.push_str(&format!(" (shipped declines: {})", trex::prefilter::required_window_reason(&pattern, input)));
        }

        let verdict = match (held > 0, refused) {
            (true, _) => "the kind is here, a lex is owed",
            (false, true) => "refused from the bytes, no lex",
            (false, false) => "absent but not refused: its literal is here for another reason",
        };
        println!(
            "{src:>10} {held:>9} {:>9} {:>10} {scan_ms:>7.2} ms {:>11.2}x  {verdict};{windows}",
            if refused { "yes" } else { "no" },
            spans.len(),
            lex_par_ms / scan_ms
        );
    }
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("name one or more corpus files: whether a refusal fires is a fact about bytes");
        std::process::exit(2);
    }

    // A kind in two lists asks one refusal two contradictory things.
    let listed: Vec<(&str, TokenKind)> = ATOMS.iter().chain(&RUN_ATOMS).chain(&NOT_IN_THE_TABLE).copied().collect();
    for (i, (a, kind)) in listed.iter().enumerate() {
        if let Some((b, _)) = listed[i + 1..].iter().find(|(_, k)| k == kind) {
            eprintln!("{a} and {b} name the kind {kind:?} in two lists");
            std::process::exit(2);
        }
    }

    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!(
        "{} atoms refused by a literal, {} by a run, {} by neither",
        ATOMS.len(),
        RUN_ATOMS.len(),
        NOT_IN_THE_TABLE.len()
    );

    for (i, path) in paths.iter().enumerate() {
        eprintln!("[{}/{}] {path}", i + 1, paths.len());
        match std::fs::read(path) {
            Ok(input) => report(path, &input),
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        }
    }

    println!(
        "\nA row reading `refused from the bytes, no lex` is the change working: the corpus\n\
         holds no token of that kind and no token was built to find that out. A row reading\n\
         `absent but not refused` is the filter admitting an input it cannot settle, which\n\
         costs a lex and answers correctly. A row where the kind is present owes a lex and\n\
         always did."
    );
}
