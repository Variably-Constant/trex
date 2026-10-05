//! The byte routes cut across the cores, against the serial form that ships.
//!
//! On the comparison's corpus three of the rows trex loses to the regex crate
//! take a byte route with no lex at all: a literal word, a byte pattern, a
//! line-anchored literal. At 400000 statements the literal reads 3.78 ms
//! against 3.04 for about 800,000 matches, which is under a nanosecond a
//! match - not per-occurrence work but a fixed pass, and the route has one the
//! regex crate does not: the reader's quote scan over the whole input.
//!
//! The route is serial. The regex crate is serial. This machine is not. The
//! arms here cut the input at [`trex::parallel_lex::safe_boundaries`], the
//! points the parallel lexer proves are token boundaries - a newline before,
//! a significant byte at, no string across - and run the shipped route over
//! each chunk on its own core. At such a boundary no run, no string and no
//! literal crosses, so a chunk's reader answers from the chunk's bytes alone
//! and the spans it yields, rebased, are the serial route's for that stretch.
//! A chunk the bytes refuse refuses the whole, as the serial route does.
//!
//! What the arms separate:
//!
//!   serial          the route as shipped, trex::scan
//!   chunked xN      the same route over N-way cut input, on the cores
//!   boundaries      safe_boundaries alone: the serial quote pass the chunked
//!                   arms run and the serial arm does not
//!   regex           the crate, the thing to beat
//!
//! Chunked and serial are asserted equal, span for span, before anything is
//! timed - on the corpus, and on an input built so that a string spans every
//! would-be boundary and a quote the bytes cannot settle is at one.
//!
//! Run: `cargo run --release --example chunked_route_ab [statements]`.

use std::hint::black_box;
use std::time::Instant;

use regex::Regex;
use trex::engine::Span;

/// Statements of four shapes, the corpus `benches/vs_regex_full` builds.
fn corpus(statements: usize) -> String {
    let mut s = String::new();
    for i in 0..statements {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
    }
    s
}

const ROUNDS: usize = 7;
const BUDGET_MS: f64 = 25.0;

/// Time `f` until `BUDGET_MS` has passed and at least five calls have run,
/// reporting the mean call and its answer - the comparison's own harness.
fn time_budget(f: &mut dyn FnMut() -> usize) -> (f64, usize) {
    let n = f();
    let t0 = Instant::now();
    let mut iters = 0u32;
    loop {
        black_box(f());
        iters += 1;
        if iters >= 5 && t0.elapsed().as_secs_f64() * 1e3 >= BUDGET_MS {
            break;
        }
    }
    (t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters), n)
}

fn median_and_spread(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("a measured time is never NaN"));
    (v[v.len() / 2], v[v.len() - 1] / v[0])
}

/// Every arm timed once a round in an order that rotates, then the first one
/// again as a control. Returns each arm's median in the order given.
fn sweep(arms: &mut [(&str, &mut dyn FnMut() -> usize)]) -> Vec<f64> {
    let n = arms.len();
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); n];
    let mut answers: Vec<usize> = vec![0; n];
    for round in 0..ROUNDS {
        for step in 0..n {
            let i = (round + step) % n;
            let (t, got) = time_budget(&mut *arms[i].1);
            times[i].push(t);
            answers[i] = got;
        }
    }
    let (control, _) = time_budget(&mut *arms[0].1);

    println!("{:>22} {:>11} {:>9} {:>12}", "arm", "ms", "spread", "answer");
    let mut medians = Vec::with_capacity(n);
    for (i, (name, _)) in arms.iter().enumerate() {
        let (ms, spread) = median_and_spread(times[i].clone());
        medians.push(ms);
        println!("{name:>22} {ms:>11.4} {spread:>8.2}x {:>12}", answers[i]);
    }
    println!(
        "{:>22} {control:>11.4} {:>9} drift {:.3}x against the same work at the start",
        "CONTROL", "", control / medians[0]
    );
    medians
}

/// `route` over each chunk of `input` cut at safe boundaries, on the cores,
/// the spans rebased and joined in chunk order - which is start order, since
/// a chunk owns exactly the occurrences that start inside it.
///
/// `None` from any chunk is `None` for the whole: the serial route hands the
/// whole input to the lexer at the first construct the bytes refuse, and a
/// parallel form that answered around it would answer a different question.
fn chunked<F>(input: &[u8], chunks: usize, route: F) -> Option<Vec<Span>>
where
    F: Fn(&[u8]) -> Option<Vec<Span>> + Sync,
{
    // Line boundaries only, as the shipped cut keeps them: a safe boundary at
    // a whitespace-run end is a lexer's boundary, not one a route that reads
    // across whitespace may take.
    let n = input.len();
    let mut bounds = trex::parallel_lex::safe_boundaries(input, chunks);
    bounds.retain(|&b| b == 0 || b == n || input[b - 1] == b'\n');
    let leaves = bounds.len() - 1;
    let mut found: Vec<Option<Vec<Span>>> = vec![None; leaves];
    let plan = flynnel::JobPlan::set_profile(
        0,
        u32::try_from(leaves).unwrap_or(u32::MAX),
        flynnel::DispatchProfile::Streaming,
    );
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut found, 1, |base, slots| {
        for (k, slot) in slots.iter_mut().enumerate() {
            let lo = bounds[base + k];
            let hi = bounds[base + k + 1];
            let base_u32 = u32::try_from(lo).expect("an input this route takes fits a u32 span");
            *slot = route(&input[lo..hi]).map(|spans| {
                spans
                    .into_iter()
                    .map(|s| Span { start: s.start + base_u32, end: s.end + base_u32 })
                    .collect()
            });
        }
    });
    let mut out: Vec<Span> = Vec::new();
    for leaf in found {
        out.extend(leaf?);
    }
    Some(out)
}

/// [`chunked`] for a route whose match may run past the chunk that owns its
/// start: `route` gets the input from the chunk's start to the end and how
/// many bytes it owns, exactly as the shipped cut hands them over.
fn chunked_reading_past<F>(input: &[u8], chunks: usize, route: F) -> Option<Vec<Span>>
where
    F: Fn(&[u8], usize) -> Option<Vec<Span>> + Sync,
{
    let n = input.len();
    let mut bounds = trex::parallel_lex::safe_boundaries(input, chunks);
    bounds.retain(|&b| b == 0 || b == n || input[b - 1] == b'\n');
    let leaves = bounds.len() - 1;
    let mut found: Vec<Option<Vec<Span>>> = vec![None; leaves];
    let plan = flynnel::JobPlan::set_profile(
        0,
        u32::try_from(leaves).unwrap_or(u32::MAX),
        flynnel::DispatchProfile::Streaming,
    );
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut found, 1, |base, slots| {
        for (k, slot) in slots.iter_mut().enumerate() {
            let lo = bounds[base + k];
            let hi = bounds[base + k + 1];
            let base_u32 = u32::try_from(lo).expect("fits a u32 span");
            *slot = route(&input[lo..], hi - lo).map(|spans| {
                spans
                    .into_iter()
                    .map(|s| Span { start: s.start + base_u32, end: s.end + base_u32 })
                    .collect()
            });
        }
    });
    let mut out: Vec<Span> = Vec::new();
    for leaf in found {
        out.extend(leaf?);
    }
    Some(out)
}

/// Whether the token beginning at `at` is the first significant one on its
/// line: every byte back to the previous newline is whitespace. The engine's
/// own test, restated here because it is crate-private.
fn leads_its_line(input: &[u8], at: usize) -> bool {
    input[..at].iter().rev().take_while(|&&c| c != b'\n').all(u8::is_ascii_whitespace)
}

/// `chunked` and the serial route must agree span for span, at every cut the
/// boundary finder can make: `chunks = input.len()` asks for a boundary at
/// every position, so every safe one is taken.
fn assert_agree(name: &str, input: &[u8], route: &(dyn Fn(&[u8]) -> Option<Vec<Span>> + Sync)) {
    let serial = route(input);
    for chunks in [1usize, 2, 7, 64, input.len().max(1)] {
        let par = chunked(input, chunks, route);
        assert_eq!(
            par, serial,
            "{name}: chunked at {chunks} disagrees with the serial route"
        );
    }
}

/// Inputs an ordinary corpus never produces: a string across every candidate
/// boundary, a char literal holding a quote, a quote at a line's head, and a
/// URL carrying a quote the bytes cannot settle - the case that must make
/// both forms answer `None`.
fn adversarial_inputs() -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = Vec::new();
    // Strings spanning lines, so a division at a newline lands inside one.
    let mut s = String::new();
    for i in 0..400 {
        s.push_str(&format!("alpha \"line {i}\nstill in the string alpha\n\" alpha cond_{i}\n"));
    }
    v.push(s.into_bytes());
    // A char literal holding a quote, and an escaped quote in a string.
    let mut s = String::new();
    for i in 0..400 {
        s.push_str(&format!("let c = '\"' ; alpha \"esc \\\" {i} alpha\" cond_{i}\n"));
    }
    v.push(s.into_bytes());
    // Every line opens with a quote that closes two lines on.
    let mut s = String::new();
    for i in 0..400 {
        s.push_str(&format!("\"open {i}\nalpha inside\n close\" alpha cond_{i}\n"));
    }
    v.push(s.into_bytes());
    // A URL with a double quote in it, which the reader settles: a double
    // quote inside a blob run opens no string.
    let mut s = String::new();
    for i in 0..400 {
        s.push_str(&format!("alpha http://h/p?q=\"{i} alpha cond_{i}\n"));
    }
    v.push(s.into_bytes());
    // A char literal `'"'` after `://` in one run, which the reader cannot
    // settle: a URL runs over a single quote and up to a double one, so the
    // scan cannot say whether the char literal opens. Both forms must refuse.
    // This is the input that exercises `None` from a chunk refusing the
    // whole. A bare single quote that never closes on its line is nothing to
    // the scan and does not reach the rule.
    let mut s = String::new();
    for i in 0..400 {
        s.push_str(&format!("alpha see http://x/'\"' alpha cond_{i}\n"));
    }
    v.push(s.into_bytes());
    // Matches spanning a line: the literal at a line's end, its word and
    // punctuation on the next. A cut at the line boundary must not split one.
    let mut s = String::new();
    for i in 0..400 {
        s.push_str(&format!("let\nvalue_{i} = {i} ;\nalpha cond_{i}\nlet\n  x_{i}\n  = 2 ;\n"));
    }
    v.push(s.into_bytes());
    // Two literals on one line: only the first leads it, and the second is
    // the first's word for the three-token route.
    let mut s = String::new();
    for i in 0..400 {
        s.push_str(&format!("let let = {i} ;\n  let a_{i} = 1 ; let b = 2 ;\nalpha cond_{i}\n"));
    }
    v.push(s.into_bytes());
    // A stretch with no newline at all, so a division's search finds none and
    // the boundary finder falls back to whitespace-run ends, which the cut
    // must then decline: the whole stretch is one chunk.
    let mut s = String::new();
    for i in 0..1200 {
        s.push_str(&format!("let v_{i} = {i} ; alpha cond_{i} "));
    }
    s.push('\n');
    for i in 0..400 {
        s.push_str(&format!("let w_{i} = {i} ;\n"));
    }
    v.push(s.into_bytes());
    // Digit groups the phone recognizer reads across whitespace, ending at
    // the literal, so the run's lex reaches past its own end and the reader
    // reports Unsettled. Four shapes, because which one the recognizer takes
    // is its rule to decide and not this test's to guess.
    for shape in [
        "alpha (555) 123-4567 alpha cond_{i}\n",
        "alpha 555-123-4567 alpha cond_{i}\n",
        "alpha +1 555 123 4567 alpha cond_{i}\n",
        "alpha 555 1234 alpha 4567 cond_{i}\n",
    ] {
        let mut s = String::new();
        for i in 0..400 {
            s.push_str(&shape.replace("{i}", &i.to_string()));
        }
        v.push(s.into_bytes());
    }
    v
}

fn main() {
    let statements: usize = std::env::args()
        .nth(1)
        .map(|a| a.parse().expect("the statement count is a number"))
        .unwrap_or(200_000);
    let text = corpus(statements);
    let input = text.as_bytes();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);

    let p_word = trex::parse("\"alpha\"").expect("parses");
    let p_bp = trex::parse("`cond_[0-9]+`").expect("parses");
    let p_anch = trex::parse("^ \"let\"").expect("parses");
    let re_word = Regex::new(r"\balpha\b").expect("compiles");
    let re_bp = Regex::new(r"\bcond_[0-9]+\b").expect("compiles");
    let re_anch = Regex::new(r"(?m)^let\b").expect("compiles");

    let lits = trex::prefilter::byte_routable_literals(&p_word).expect("a word literal is routable");
    let anchored =
        trex::prefilter::byte_routable_line_anchored_literal(&p_anch).expect("a line-anchored literal is routable");
    let (bp, prefix) = trex::prefilter::byte_routable_byte_pattern(&p_bp).expect("a byte pattern is routable");

    println!("trex {}", trex::version());
    println!("corpus: {} bytes, {statements} statements, {cores} cores", input.len());

    // The three byte routes for every match in their one-pass form, each as a
    // closure over a slice so one `chunked` serves all three. One pass by name:
    // the shipped route cuts across the cores itself above its threshold, and
    // cutting that again would time the cut twice. The line-anchored one is the
    // word-literal route with the engine's own leads-its-line filter after it.
    let word = |s: &[u8]| trex::prefilter::byte_route_word_literals_one_pass(&lits, s);
    let anch = |s: &[u8]| {
        trex::prefilter::byte_route_word_literals_one_pass(&[anchored], s).map(|spans| {
            spans.into_iter().filter(|x| leads_its_line(s, x.start as usize)).collect()
        })
    };
    let bpat = |s: &[u8]| trex::prefilter::byte_route_byte_pattern_one_pass(bp, &prefix, s);
    // The shipped routes, which choose for themselves: what a caller gets.
    let shipped_word = |s: &[u8]| trex::prefilter::byte_route_word_literals(&lits, s);
    let shipped_bpat = |s: &[u8]| trex::prefilter::byte_route_byte_pattern(bp, &prefix, s);

    // Agreement first, on the corpus and on the inputs built to break a cut.
    assert_agree("word literal", input, &word);
    assert_agree("line-anchored", input, &anch);
    assert_agree("byte pattern", input, &bpat);
    for (i, adv) in adversarial_inputs().iter().enumerate() {
        assert_agree(&format!("word literal, adversarial {i}"), adv, &word);
        assert_agree(&format!("line-anchored, adversarial {i}"), adv, &anch);
        assert_agree(&format!("byte pattern, adversarial {i}"), adv, &bpat);
        let refused = word(adv).is_none();
        println!("  adversarial {i}: {} bytes, route {}", adv.len(), if refused { "REFUSED by both, as required" } else { "answered by both, equal" });
    }
    println!("chunked == serial at every cut, on the corpus and on every adversarial input");
    // The shipped routes must hand back what the one-pass form does, whichever
    // side of their threshold this input falls.
    assert_eq!(shipped_word(input), word(input), "the shipped word-literal route disagrees with one pass");
    assert_eq!(shipped_bpat(input), bpat(input), "the shipped byte-pattern route disagrees with one pass");
    println!("shipped == one pass on the corpus\n");

    // The three-token route, `"let" \W "="`, held to two oracles that never
    // touch it: the windows it replaces, and the single-pass engine.
    let p_lwp = trex::parse("\"let\" \\W \"=\"").expect("parses");
    let re_lwp = Regex::new(r"\blet\b [A-Za-z_][A-Za-z_0-9]* =").expect("compiles");
    let (lwp_lit, lwp_punct) =
        trex::prefilter::byte_routable_literal_word_punct(&p_lwp).expect("a literal, word, punctuation is routable");
    let lwp = |s: &[u8]| trex::prefilter::byte_route_literal_word_punct_one_pass(lwp_lit, lwp_punct, s);
    // The read-past form the shipped cut runs: a chunk owns the anchors that
    // start inside it and reads past its end for the word and punctuation.
    let lwp_owning =
        |tail: &[u8], owns: usize| trex::prefilter::byte_route_literal_word_punct_owning(lwp_lit, lwp_punct, tail, owns);
    let agree_past = |name: &str, s: &[u8]| {
        let serial = lwp(s);
        for chunks in [1usize, 2, 7, 64, s.len().max(1)] {
            let par = chunked_reading_past(s, chunks, lwp_owning);
            assert_eq!(par, serial, "{name}: read-past cut at {chunks} disagrees with one pass");
        }
    };
    agree_past("literal, word, punctuation", input);
    for (i, adv) in adversarial_inputs().iter().enumerate() {
        agree_past(&format!("literal, word, punctuation, adversarial {i}"), adv);
    }
    let lwp_spans = lwp(input).expect("the bytes settle the corpus");
    let windows = trex::prefilter::scan_by_literal_windows(&p_lwp, input).expect("the windows take this shape");
    assert_eq!(lwp_spans, windows, "the literal, word, punctuation route disagrees with the windows");
    let engine = trex::nfa::scan_nfa(&p_lwp, input).expect("the engine takes this shape");
    assert_eq!(lwp_spans, engine, "the literal, word, punctuation route disagrees with the engine");
    println!("literal, word, punctuation == the windows == the engine on the corpus\n");

    // A route under test: bytes in, the spans it found out, and callable from
    // several threads at once.
    type RouteFn<'a> = dyn Fn(&[u8]) -> Option<Vec<Span>> + Sync + 'a;

    for (name, tp, rp) in [
        ("a literal word", "\"alpha\"", r"\balpha\b"),
        ("a byte-pattern inside a token", "`cond_[0-9]+`", r"\bcond_[0-9]+\b"),
        ("a line-anchored literal", "^ \"let\"", r"(?m)^let\b"),
        ("a literal, a word and punctuation", "\"let\" \\W \"=\"", r"\blet\b [A-Za-z_][A-Za-z_0-9]* ="),
    ] {
        println!("=== {name}   trex `{tp}`   regex `{rp}` ===");
        let (p, re, route): (_, &Regex, &RouteFn<'_>) = match name {
            "a literal word" => (&p_word, &re_word, &word),
            "a byte-pattern inside a token" => (&p_bp, &re_bp, &bpat),
            "a line-anchored literal" => (&p_anch, &re_anch, &anch),
            _ => (&p_lwp, &re_lwp, &lwp),
        };
        // The three-token route's match may run past its chunk, so its ladder
        // cuts the way the shipped form does; the whole-token routes cut at
        // the chunk's own bytes.
        let past = name == "a literal, a word and punctuation";
        let cut = |chunks: usize| -> usize {
            if past {
                chunked_reading_past(input, chunks, lwp_owning).map_or(0, |v| v.len())
            } else {
                chunked(input, chunks, route).map_or(0, |v| v.len())
            }
        };
        let mut one_pass = || route(input).map_or(0, |v| v.len());
        let mut shipped = || trex::scan(p, input).len();
        let mut c1 = || cut(cores);
        let mut c4 = || cut(cores * 4);
        let mut c16 = || cut(cores * 16);
        let mut bounds4 = || trex::parallel_lex::safe_boundaries(input, cores * 4).len();
        let mut regex = || re.find_iter(&text).count();
        // The windows this route replaces, where it replaces any: what the
        // three-token row cost before, on the same sweep and control.
        let mut windows = || trex::prefilter::scan_by_literal_windows(p, input).map_or(0, |v| v.len());
        let mut arms: Vec<(&str, &mut dyn FnMut() -> usize)> = vec![
            ("one pass", &mut one_pass),
            ("shipped, trex::scan", &mut shipped),
            ("chunked x1 cores", &mut c1),
            ("chunked x4 cores", &mut c4),
            ("chunked x16 cores", &mut c16),
            ("boundaries x4 alone", &mut bounds4),
            ("regex", &mut regex),
        ];
        if name == "a literal, a word and punctuation" {
            arms.push(("the windows it replaces", &mut windows));
        }
        let m = sweep(&mut arms);
        let best = m[2..5].iter().cloned().fold(f64::INFINITY, f64::min);
        println!(
            "  one pass over regex {:.3}x   shipped over regex {:.3}x   best chunked over regex {:.3}x   chunked net of boundaries {:.4} ms\n",
            m[0] / m[6],
            m[1] / m[6],
            best / m[6],
            best - m[5]
        );
    }
}
