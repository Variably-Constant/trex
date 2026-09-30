//! trex against a regular-expression engine on the work they both do.
//!
//! The comparison is only honest if it says what each side is paying for.
//! trex lexes the input into typed tokens and then matches over those; a
//! regular expression matches bytes directly and never lexes. So the lex is
//! timed on its own, and the match is timed twice - once from bytes, which is
//! what a caller actually pays, and once over a token slice already lexed,
//! which is what the matching engine costs by itself.
//!
//! Patterns are paired so the two sides mean the same thing. trex's atoms are
//! whole tokens, so a literal is `\bfoo\b` on the regex side and a word token
//! is a maximal identifier run.
//!
//! One size cannot answer whether the gap closes. trex pays a lex before it
//! matches anything, so a fixed cost shows as a large ratio on a small input
//! and fades on a large one, while a per-byte difference holds at every size.
//! The sweep runs the same pairs from six kilobytes to sixteen megabytes and
//! reports throughput beside the times, so the two can be told apart. Above a
//! few megabytes `trex::scan` may also place a pattern's anchors across the
//! host and the device, which the no-default-features build exists to separate.
//!
//! The sweep runs over three corpora, because a literal word is answered
//! from the bytes by three different routes. In plain statements every
//! occurrence has punctuation or whitespace either side and the bytes settle
//! it. In typed statements the literal is joined by a dot, a slash or a
//! colon, with quoted strings and URLs around it, and the route lexes the
//! occurrence's own run. The third corpus ends with one long unbroken run
//! holding the literal, which the route refuses, so every earlier occurrence
//! was walked for nothing and the whole input is then lexed.
//!
//! A fourth section holds the density axis instead of the size axis. A
//! pattern the earlier routes do not answer reaches the window route, which
//! lexes only the regions a literal of the pattern can put a match in. How
//! much of the input that is depends on how often the literal occurs, so the
//! corpus there carries a marker word at a chosen count and the section runs
//! the same pattern from one occurrence to one every other line, against the
//! same scan reaching the engine directly.

use std::hint::black_box;
use std::time::Instant;

use regex::Regex;

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

/// The plain statements with occurrences the bytes alone cannot settle:
/// joined by a dot, a slash or a colon, beside quoted strings and URLs that
/// hold no literal. Both sides count the same: a string or a URL is one
/// token to trex, so the literal is kept out of them.
fn corpus_typed(statements: usize) -> String {
    let mut s = String::new();
    for i in 0..statements {
        match i % 8 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}.alpha, item_{}.beta ;\n", i + 1)),
            3 => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
            4 => s.push_str(&format!("open(dir_{i}/alpha, \"gamma delta {i}\") ;\n")),
            5 => s.push_str(&format!("bind(alpha:{}, https://example.com/v{i}) ;\n", 8000 + i % 1000)),
            6 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("mail_{i} = user_{i}@example.org ;\n")),
        }
    }
    s
}

/// [`corpus_typed`] with the literal inside a run of sixty-three bytes with
/// no whitespace in its last statement. A run that long may be a blob, which
/// the route refuses to decide, so the whole input is lexed after every
/// earlier occurrence was walked. The run is one letter repeated, so the
/// lexer makes ordinary tokens of it and both sides count the occurrence.
fn corpus_long_run_last(statements: usize) -> String {
    let mut s = corpus_typed(statements);
    s.push_str("token = aaaaaaaaaaaaaaaaaaaaaaaa.alpha.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ;\n");
    s
}

/// Statements whose values are long unbroken runs, the shape that makes a
/// window's own lex expensive: a token here is hundreds of bytes where a
/// statement corpus averages two to six.
fn corpus_long_tokens(statements: usize) -> String {
    let mut s = String::new();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for i in 0..statements {
        let run: String = (0..200usize)
            .map(|j| alphabet[(i * 31 + j * 17) % alphabet.len()] as char)
            .collect();
        s.push_str(&format!("blob_{i} = {run} ;\n"));
    }
    s
}

/// [`corpus_long_tokens`] with no whitespace at all past the first line: one
/// token for the whole input, which is the worst a window's lex can meet.
fn corpus_one_run(statements: usize) -> String {
    let mut s = corpus_long_tokens(statements);
    s.retain(|c| c != '\n' && c != ' ');
    s.insert(0, '\n');
    s
}

/// A statement whose first word occurs nowhere else in [`corpus`] or
/// [`corpus_long_tokens`].
const MARKER: &str = "rare_value = 4242 ;\n";

/// [`corpus_long_tokens`] with `markers` marker lines spread evenly through
/// it, the long-token counterpart of [`corpus_marked`].
///
/// A window is a fixed width in bytes, so a corpus whose tokens are long puts
/// fewer tokens inside one and the route's cost a window is not the same
/// thing here as it is among statements. Whether the crossing moves with the
/// shape is the question this shape exists to answer.
fn corpus_marked_long(statements: usize, markers: usize) -> String {
    let base = corpus_long_tokens(statements);
    let mut s = String::with_capacity(base.len() + markers * MARKER.len() + 1);
    let step = (statements / markers.max(1)).max(1);
    for (i, line) in base.split_inclusive('\n').enumerate() {
        if i % step == step / 2 && i / step < markers {
            s.push_str(MARKER);
        }
        s.push_str(line);
    }
    s
}

/// The plain statements with `markers` marker lines spread evenly through
/// them. The marker word occurs nowhere else, so a pattern holding it has
/// exactly `markers` byte occurrences, and that count is the axis the window
/// route's coverage gate decides on: few enough and the route lexes only the
/// regions those occurrences reach, many enough and it hands the whole input
/// to the lex as before.
fn corpus_marked(statements: usize, markers: usize) -> String {
    let base = corpus(statements);
    let mut s = String::with_capacity(base.len() + markers * MARKER.len() + 1);
    let step = (statements / markers.max(1)).max(1);
    for (i, line) in base.split_inclusive('\n').enumerate() {
        if i % step == step / 2 && i / step < markers {
            s.push_str(MARKER);
        }
        s.push_str(line);
    }
    s
}

fn time(iters: u32, mut f: impl FnMut() -> usize) -> (f64, usize) {
    let n = f();
    let t0 = Instant::now();
    for _ in 0..iters {
        black_box(f());
    }
    (t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters), n)
}

/// Time `f` until `budget_ms` has passed and at least five calls have run,
/// reporting the mean call, its answer, and the call count that produced it.
///
/// A fixed iteration count cannot span the sweep: two hundred calls is a tenth
/// of a second at six kilobytes and minutes at sixteen megabytes. The floor of
/// five keeps a large size from being judged on one call.
fn time_budget(budget_ms: f64, mut f: impl FnMut() -> usize) -> (f64, usize, u32) {
    let n = f();
    let t0 = Instant::now();
    let mut iters = 0u32;
    loop {
        black_box(f());
        iters += 1;
        if iters >= 5 && t0.elapsed().as_secs_f64() * 1e3 >= budget_ms {
            break;
        }
    }
    (t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters), n, iters)
}

/// A trex word token is an identifier run, digits included after the first
/// character, so the regex side has to say the same or the two are not
/// counting the same thing.
const PAIRS: [(&str, &str, &str); 7] = [
    ("a literal word", "\"alpha\"", r"\balpha\b"),
    // Nowhere in the corpus, so both engines answer nothing. regex reaches a
    // prefilter and returns without a pass; trex can answer from the bytes
    // too, because a literal atom under the identity orbit is a byte string
    // every match must contain. This row is what a scan costs when it can be
    // refused before the lex.
    ("a literal word that is absent", "\"zzzqqq\"", r"\bzzzqqq\b"),
    ("any word token", "\\W", r"\b[A-Za-z_][A-Za-z_0-9]*\b"),
    ("any number token", "\\N", r"\b[0-9]+\b"),
    ("either of two literals", "(\"alpha\" | \"beta\")", r"\b(?:alpha|beta)\b"),
    ("word then punctuation", "\\W \"=\"", r"\b[A-Za-z_][A-Za-z_0-9]*\b ="),
    ("a byte-pattern inside a token", "`cond_[0-9]+`", r"\bcond_[0-9]+\b"),
];

/// The same pairs at every size, with throughput beside the times. A ratio
/// that falls as the input grows is a fixed cost being amortized; one that
/// holds is a per-byte difference.
fn sweep(label: &str, corpus: fn(usize) -> String) {
    println!("\n=== trex against the regex crate across corpus sizes: {label} ===");
    for statements in [200usize, 2_000, 20_000, 200_000, 520_000] {
        let text = corpus(statements);
        let bytes = text.as_bytes();
        let toks = trex::lexer::lex(bytes);
        let (lex_ms, ntok, _) = time_budget(50.0, || trex::lexer::lex(bytes).len());
        // Both lexers, because the trex column below is a full scan and a scan
        // takes the parallel one. Subtracting the serial figure from it says
        // nothing: at the top size the serial lex alone is most of a scan whose
        // pre-lexed match is already most of it.
        let (plex_ms, _, _) = time_budget(50.0, || trex::parallel_lex::lex_parallel(bytes).len());
        let mb = bytes.len() as f64 / (1024.0 * 1024.0);
        println!(
            "\n{} bytes, {} tokens, lex serial {:.3} ms ({:.0} MB/s), lex as a scan takes it {:.3} ms ({:.0} MB/s)",
            bytes.len(),
            ntok,
            lex_ms,
            mb / (lex_ms / 1e3),
            plex_ms,
            mb / (plex_ms / 1e3)
        );
        println!(
            "  {:<30} {:>9} {:>10} {:>9} {:>7} {:>10} {:>10} {:>6} {:>6} {:>6}",
            "pattern", "trex ms", "lexed ms", "regex ms", "ratio", "trex MB/s", "re MB/s", "calls", "c.trex", "c.re"
        );
        for (label, tp, rp) in PAIRS {
            let p = trex::parse(tp).expect("trex pattern parses");
            let re = Regex::new(rp).expect("oracle regex compiles");
            // Each side is timed twice with the sides alternating, and a
            // side's second timing over its first is that side's control: the
            // two engines are not bottlenecked alike, so a neighbour moves
            // them by different amounts and a repeat of one cannot speak for
            // the other.
            let (t_full, n_full, calls) = time_budget(50.0, || trex::scan(&p, bytes).len());
            let (t_re, n_re, _) = time_budget(50.0, || re.find_iter(&text).count());
            let (t_full_again, _, _) = time_budget(50.0, || trex::scan(&p, bytes).len());
            let (t_re_again, _, _) = time_budget(50.0, || re.find_iter(&text).count());
            // The pre-lexed column has to take the same engine the full scan
            // took, or it measures a different matcher rather than the same
            // one without its lex.
            let (t_lexed, _, _) = time_budget(50.0, || {
                trex::nfa::scan_nfa_over(&p, bytes, &toks).map_or(0, |m| m.len())
            });
            // A count that differs makes the row's ratio meaningless, so it is
            // said rather than asserted: one bad row should not discard the
            // rest of a sweep that takes minutes.
            let counts =
                if n_full == n_re { String::new() } else { format!("  counts {n_full} vs {n_re}") };
            println!(
                "  {label:<30} {t_full:>9.3} {t_lexed:>10.3} {t_re:>9.3} {:>6.1}x {:>10.1} {:>10.1} {calls:>6} {:>5.2}x {:>5.2}x{counts}",
                t_full / t_re,
                mb / (t_full / 1e3),
                mb / (t_re / 1e3),
                t_full_again / t_full,
                t_re_again / t_re
            );
        }
    }
}

/// Patterns no earlier route answers. A sequence whose first element is a
/// literal is not one literal token, not a word then punctuation, not all
/// kinds and not a byte pattern, so `scan` reaches the window route with
/// them. The first holds the marker word, whose count the corpus sets; the
/// second holds a word on every fourth line, which is the dense end of the
/// same axis on a literal the corpus already has.
const WINDOW_PAIRS: [(&str, &str, &str); 2] = [
    ("marker literal then a number", "\"rare_value\" \"=\" \\N", r"\brare_value\b = [0-9]+"),
    ("dense literal then a word", "\"alpha\" \",\" \\W", r"\balpha\b, [A-Za-z_][A-Za-z_0-9]*"),
];

/// Rounds of the three arms behind each row of the window sweep. Five, and
/// the middle one reported, because a box shared with other work moves a
/// single pair of readings by more than the route's own effect at some
/// densities.
const WINDOW_ROUNDS: usize = 5;

/// The most marker occurrences the ungated arm is run at.
///
/// With its gate open the route lexes one window an occurrence, so a density
/// the gate exists to refuse costs minutes a cell to price. The crossing this
/// arm is here to find sits near a thousand windows, three orders of
/// magnitude below the dense end of the ladder.
const UNGATED_CEILING: usize = 8_000;

/// The middle of `v`, and its spread as the largest reading over the
/// smallest. A spread far from one says the arm moved under the box while the
/// row was timed, so its middle is not a reading.
fn median_and_spread(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("a measured time is never NaN"));
    let mid = v[v.len() / 2];
    let spread = if v[0] > 0.0 { v[v.len() - 1] / v[0] } else { f64::INFINITY };
    (mid, spread)
}

/// What the window route costs and saves against the same scan reaching the
/// engine directly, over the density of the literal it windows on.
///
/// The middle column is `scan_nfa`, which lexes the whole input and matches
/// over it - what the scan cost before the route existed - so the two trex
/// columns are one matcher with and without the window, and their ratio is
/// the route's whole effect. regex is the outside reference. The `take`
/// column says whether the route answered or handed the input back, so a
/// ratio near one is readable as the route declining rather than as the
/// route running and gaining nothing. The last three columns are each arm's
/// own spread over the rounds; a row whose spread is far from one says the
/// box moved while it was timed and its ratios are not readable.
fn window_sweep(statements: usize) {
    window_ladder("statements", statements, corpus_marked);
    // A window is a fixed width in bytes, so a corpus whose tokens are long
    // puts fewer of them inside one. The ladder runs again over that shape,
    // because a cut point read on one shape is not a cut point.
    window_ladder("two-hundred-byte tokens", statements / 8, corpus_marked_long);
}

fn window_ladder(shape: &str, statements: usize, build: fn(usize, usize) -> String) {
    println!("\n=== the window route over literal density: {shape} ===");
    // The route's reach is 768 bytes each side of a hit, so a marker count
    // near 6_300 puts the windows at the coverage limit on the statement
    // corpus. The ladder crosses it from both sides.
    for markers in [260_000usize, 65_000, 8_000, 6_300, 5_000, 3_500, 2_000, 1_000, 128, 1] {
        let text = build(statements, markers);
        let bytes = text.as_bytes();
        println!("\n{} bytes, {markers} marker occurrences", bytes.len());
        println!(
            "  {:<30} {:>4} {:>9} {:>9} {:>9} {:>8} {:>7} {:>9} {:>9} {:>6} {:>6} {:>6} {:>6}",
            "pattern",
            "take",
            "route ms",
            "no route",
            "regex ms",
            "vs none",
            "vs re",
            "ungated",
            "un/none",
            "s.rt",
            "s.no",
            "s.re",
            "s.un"
        );
        for (label, tp, rp) in WINDOW_PAIRS {
            let p = trex::parse(tp).expect("trex pattern parses");
            let re = Regex::new(rp).expect("oracle regex compiles");
            let take = if trex::prefilter::scan_required_windows(&p, bytes).is_some() {
                "win"
            } else {
                "lex"
            };
            // The route with its gate open, so the row prices it at densities
            // the gate refuses. The gate is a cut point on this curve, and a
            // reading that only ever sees the accepted side cannot say where
            // the curve crosses or how far the cut sits from the crossing.
            // Not past UNGATED_CEILING windows: with the gate open the route
            // lexes one window an occurrence, so a density the gate exists to
            // refuse would spend minutes a cell proving what the gate already
            // knows. The crossing is three orders of magnitude below that.
            let (t_un, s_un) = if markers <= UNGATED_CEILING {
                let mut v: Vec<f64> = Vec::with_capacity(WINDOW_ROUNDS);
                for _ in 0..WINDOW_ROUNDS {
                    let (t, _, _) = time_budget(30.0, || {
                        trex::prefilter::scan_required_windows_ungated(&p, bytes)
                            .map_or(0, |s| s.len())
                    });
                    v.push(t);
                }
                median_and_spread(v)
            } else {
                (f64::NAN, f64::NAN)
            };
            // What each gate decides here, in the order they were proposed.
            // A gate is right when it takes every density the ungated column
            // shows a win at and refuses every one it shows a loss at, so the
            // decisions belong beside that column rather than in a table of
            // their own.
            let decisions: String = [
                ("byte", trex::prefilter::WindowGate::ByteCount),
                ("cover", trex::prefilter::WindowGate::CoverageOnly),
                ("priced", trex::prefilter::WindowGate::PricedWindow),
                ("dens", trex::prefilter::WindowGate::PredictedDensity),
            ]
            .iter()
            .map(|&(name, g)| {
                let took = trex::prefilter::scan_required_windows_gated(&p, bytes, g).is_some();
                format!(" {name}={}", if took { "win" } else { "lex" })
            })
            .collect();
        // Which stage turned the route away, since a corpus the route refuses
        // reads exactly like a corpus its gate refuses, and only one of those
        // is a gate's fault.
        let why = trex::prefilter::required_window_reason(&p, bytes);
            let mut rt: Vec<f64> = Vec::with_capacity(WINDOW_ROUNDS);
            let mut no: Vec<f64> = Vec::with_capacity(WINDOW_ROUNDS);
            let mut rx: Vec<f64> = Vec::with_capacity(WINDOW_ROUNDS);
            let (mut n_rt, mut n_no, mut n_re) = (0usize, 0usize, 0usize);
            for round in 0..WINDOW_ROUNDS {
                // The three arms run in an order that rotates every round, so
                // no arm always follows the same neighbour, and a box that
                // drifts across the sweep shows in each arm's own spread
                // rather than as a difference between the arms. The arms are
                // not bottlenecked alike - the route is serial where the scan
                // it replaces lexes across cores - so one arm's repeat cannot
                // speak for another's.
                for arm in 0..3 {
                    match (round + arm) % 3 {
                        0 => {
                            let (t, n, _) = time_budget(30.0, || trex::scan(&p, bytes).len());
                            rt.push(t);
                            n_rt = n;
                        }
                        1 => {
                            let (t, n, _) = time_budget(30.0, || {
                                trex::nfa::scan_nfa(&p, bytes).map_or(0, |m| m.len())
                            });
                            no.push(t);
                            n_no = n;
                        }
                        _ => {
                            let (t, n, _) = time_budget(30.0, || re.find_iter(&text).count());
                            rx.push(t);
                            n_re = n;
                        }
                    }
                }
            }
            let (t_rt, s_rt) = median_and_spread(rt);
            let (t_no, s_no) = median_and_spread(no);
            let (t_re, s_re) = median_and_spread(rx);
            // A count that differs makes the row's ratios meaningless, so it
            // is said rather than asserted: one bad row should not discard a
            // sweep that takes minutes.
            let counts = if n_rt == n_no && n_rt == n_re {
                String::new()
            } else {
                format!("  counts {n_rt} route, {n_no} none, {n_re} re")
            };
            println!(
                "  {label:<30} {take:>4} {t_rt:>9.3} {t_no:>9.3} {t_re:>9.3} {:>7.2}x {:>6.2}x {t_un:>9.3} {:>8.2}x {s_rt:>5.2}x {s_no:>5.2}x {s_re:>5.2}x {s_un:>5.2}x{decisions}  why={why}{counts}",
                t_rt / t_no,
                t_rt / t_re,
                t_un / t_no,
            );
        }
    }
}

/// What a byte costs in a window's own lex against the same byte in the lex
/// it replaces, per corpus shape.
///
/// A window is lexed serially where the whole-input scan lexes across cores,
/// so this ratio is the coverage the window route may take before its own
/// lexing costs more than the lexing it saves: it is
/// `prefilter::WINDOW_LEX_SHARE`, in per mille, and the smallest reading
/// across shapes is the one that constant has to hold.
/// `build` at a statement count that brings the corpus to about `target`
/// bytes.
///
/// The shapes below differ in statement length by six times, so building them
/// all at one statement count compares them at six different sizes - and the
/// ratio this sweep reads moves with size, because the across-cores lex has
/// more to amortize on a larger input. One size is the axis worth holding
/// still.
fn corpus_of_about(build: fn(usize) -> String, target: usize) -> String {
    let probe = build(2_000);
    let per = (probe.len() / 2_000).max(1);
    build((target / per).max(1))
}

fn lex_ratio_sweep() {
    println!("\n=== a window's own lex against the lex it replaces ===");
    println!(
        "  {:<34} {:>10} {:>9} {:>9} {:>8} {:>6} {:>6}",
        "corpus", "bytes", "serial", "across", "per mille", "s.ser", "s.acr"
    );
    for (label, build) in [
        ("plain statements", corpus as fn(usize) -> String),
        ("typed statements", corpus_typed),
        ("typed, a long run last", corpus_long_run_last),
        ("two-hundred-byte tokens", corpus_long_tokens),
        ("one run, no whitespace", corpus_one_run),
    ] {
        let text = corpus_of_about(build, 16 << 20);
        let bytes = text.as_bytes();
        let mut ser: Vec<f64> = Vec::new();
        let mut par: Vec<f64> = Vec::new();
        for round in 0..WINDOW_ROUNDS {
            for arm in 0..2 {
                if (round + arm) % 2 == 0 {
                    let (t, _, _) = time_budget(30.0, || trex::lexer::lex(bytes).len());
                    ser.push(t);
                } else {
                    let (t, _, _) =
                        time_budget(30.0, || trex::parallel_lex::lex_parallel(bytes).len());
                    par.push(t);
                }
            }
        }
        let (t_ser, s_ser) = median_and_spread(ser);
        let (t_par, s_par) = median_and_spread(par);
        println!(
            "  {label:<34} {:>10} {t_ser:>9.3} {t_par:>9.3} {:>8.0} {s_ser:>5.2}x {s_par:>5.2}x",
            bytes.len(),
            t_par / t_ser * 1000.0
        );
    }
}

/// What asking only whether a match exists costs, against finding every match
/// and against the engine's own early-stopping walk.
///
/// Three arms, rotating, each with its own spread. `is_match` short-circuits
/// on the byte side - the absent-literal refusal, and the window route
/// stopping at its first window - and then falls back to the scan, so what
/// this reads is how much the byte side alone saves, on a pattern the route
/// answers and on one it hands back. The engine's own early-stopping walk is
/// crate-private: whether the fallback should route through it is read by
/// building the crate both ways and timing the two commits.
fn existence_sweep(statements: usize) {
    println!("\n=== asking only whether a match exists ===");
    for (label, markers, tp) in [
        ("a rare literal, matching early", 128usize, "\"rare_value\" \"=\" \\N"),
        ("a rare literal, no match at all", 128, "\"rare_value\" \";\" \\N"),
        ("a dense literal, matching early", 128, "\"alpha\" \",\" \\W"),
        ("a dense literal, no match at all", 128, "\"alpha\" \";\" \\W"),
    ] {
        let text = corpus_marked(statements, markers);
        let bytes = text.as_bytes();
        let p = trex::parse(tp).expect("trex pattern parses");
        let found = !trex::scan(&p, bytes).is_empty();
        let mut ex: Vec<f64> = Vec::new();
        let mut al: Vec<f64> = Vec::new();
        let mut en: Vec<f64> = Vec::new();
        for round in 0..WINDOW_ROUNDS {
            for arm in 0..3 {
                match (round + arm) % 3 {
                    0 => {
                        let (t, _, _) =
                            time_budget(30.0, || usize::from(trex::is_match(&p, bytes)));
                        ex.push(t);
                    }
                    1 => {
                        let (t, _, _) = time_budget(30.0, || trex::scan(&p, bytes).len());
                        al.push(t);
                    }
                    _ => {
                        // The third arm was the engine's own early-stopping
                        // walk. It is crate-private, so whether is_match
                        // should route through it is read by building the
                        // crate both ways and timing the two commits, not
                        // from here.
                        let (t, _, _) = time_budget(30.0, || {
                            usize::from(!trex::scan(&p, bytes).is_empty())
                        });
                        en.push(t);
                    }
                }
            }
        }
        let (t_ex, s_ex) = median_and_spread(ex);
        let (t_al, s_al) = median_and_spread(al);
        let (t_en, s_en) = median_and_spread(en);
        println!(
            "  {label:<34} {:>5} {t_ex:>9.3} {t_al:>9.3} {t_en:>9.3} {:>7.2}x {:>7.2}x {s_ex:>5.2}x {s_al:>5.2}x {s_en:>5.2}x",
            if found { "match" } else { "none" },
            t_ex / t_al,
            t_en / t_al,
        );
    }
}

/// The sections named on the command line, or all of them when none is.
///
/// The whole run is the best part of an hour, and the channel it streams over
/// gives up at one: a caller after one section should not have to buy the
/// rest, and a section that has already answered should not be re-run to
/// reach the one after it. Names: `parts`, `plain`, `typed`, `longrun`,
/// `window`, `lexratio`, `existence`.
/// Each sweep the caller asked for, in the order they read best.
fn run_sweeps(run: &[String], on: &impl Fn(&str) -> bool) {
    if on("plain") {
        sweep("plain statements", corpus);
    }
    if on("typed") {
        sweep("typed statements", corpus_typed);
    }
    if on("longrun") {
        sweep("typed statements, a long run last", corpus_long_run_last);
    }
    if on("window") {
        window_sweep(520_000);
    }
    if on("lexratio") {
        lex_ratio_sweep();
    }
    if on("existence") {
        existence_sweep(520_000);
    }
    let known = ["parts", "plain", "typed", "longrun", "window", "lexratio", "existence"];
    for name in run {
        if !known.contains(&name.as_str()) {
            println!("  no section named `{name}`; the sections are {}", known.join(" "));
        }
    }
    println!("\nDONE");
}

fn wanted() -> Vec<String> {
    // Split on commas as well as taking them one an argument: a PowerShell
    // `-File` invocation hands `a,b` through as one string rather than two.
    let named: Vec<String> = std::env::args()
        .skip(1)
        .flat_map(|a| a.split(',').map(str::to_string).collect::<Vec<_>>())
        .filter(|a| !a.is_empty())
        .collect();
    if named.is_empty() {
        ["parts", "plain", "typed", "longrun", "window", "lexratio", "existence"]
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    } else {
        named
    }
}

fn main() {
    let run = wanted();
    let on = |name: &str| run.iter().any(|s| s == name);
    println!("sections: {}", run.join(" "));
    if !on("parts") {
        run_sweeps(&run, &on);
        return;
    }
    let text = corpus(2000);
    let bytes = text.as_bytes();
    // Enough repetitions that a tenth of a millisecond is a difference and
    // not the spread: at twenty, the lex alone moved by that much between
    // runs.
    let iters = 200;
    println!("corpus: {} bytes", bytes.len());

    let (lex_ms, ntok) = time(iters, || trex::lexer::lex(bytes).len());
    println!("\nwhat only trex pays");
    println!("  lex into typed tokens                    {lex_ms:>8.3} ms  ({ntok} tokens)");
    // The full scan's time past its match is the lex route `scan` takes and
    // the guard prefilter it builds; each is timed alone so a gap between the
    // full column and lex plus match below has a name.
    let (plex_ms, _) = time(iters, || trex::parallel_lex::lex_parallel(bytes).len());
    println!("  the lex route a scan takes               {plex_ms:>8.3} ms");
    // The match is timed over tokens just written and over tokens that
    // have sat in cache since the last iteration, on each engine route,
    // because the dispatched route reads the tokens from every core and a
    // buffer the lexer just filled is dirty in one core's cache.
    let word = trex::parse("\\W").expect("pattern parses");
    let warm = trex::lexer::lex(bytes);
    let (fresh_par, _) = time(iters, || {
        let t = trex::lexer::lex(bytes);
        trex::nfa::scan_nfa_over(&word, bytes, &t).map_or(0, |m| m.len())
    });
    let (fresh_ser, _) = time(iters, || {
        let t = trex::lexer::lex(bytes);
        trex::nfa::scan_nfa_over_serial(&word, bytes, &t).map_or(0, |m| m.len())
    });
    let (warm_par, _) =
        time(iters, || trex::nfa::scan_nfa_over(&word, bytes, &warm).map_or(0, |m| m.len()));
    let (warm_ser, _) = time(iters, || {
        trex::nfa::scan_nfa_over_serial(&word, bytes, &warm).map_or(0, |m| m.len())
    });
    println!("  match any word, tokens just lexed        {fresh_par:>8.3} ms dispatched  {fresh_ser:>8.3} ms serial  (lex included)");
    println!("  match any word, tokens long held         {warm_par:>8.3} ms dispatched  {warm_ser:>8.3} ms serial");
    // The same call split by a clock between its phases, so a gap between
    // the combined time and the two parts timed alone is charged to the
    // phase that grew rather than guessed at. Once with the token buffer
    // held across iterations and once fresh per call, in that order and
    // then the reverse, so an effect of which loop ran first shows as a
    // disagreement between the two orders.
    let per = 1e3 / f64::from(iters);
    let phases_fresh = || {
        let (mut lex_in, mut match_in, mut drop_in) = (0.0f64, 0.0f64, 0.0f64);
        for _ in 0..iters {
            let t0 = Instant::now();
            let t = trex::lexer::lex(bytes);
            let t1 = Instant::now();
            black_box(trex::nfa::scan_nfa_over(&word, bytes, &t).map_or(0, |m| m.len()));
            let t2 = Instant::now();
            drop(t);
            let t3 = Instant::now();
            lex_in += (t1 - t0).as_secs_f64();
            match_in += (t2 - t1).as_secs_f64();
            drop_in += (t3 - t2).as_secs_f64();
        }
        (lex_in * per, match_in * per, drop_in * per)
    };
    let phases_held = || {
        let mut held: Vec<trex::token::Token> = Vec::new();
        let (mut lex_in, mut match_in) = (0.0f64, 0.0f64);
        for _ in 0..iters {
            let t0 = Instant::now();
            trex::lexer::lex_into(bytes, &mut held);
            let t1 = Instant::now();
            black_box(trex::nfa::scan_nfa_over(&word, bytes, &held).map_or(0, |m| m.len()));
            let t2 = Instant::now();
            lex_in += (t1 - t0).as_secs_f64();
            match_in += (t2 - t1).as_secs_f64();
        }
        (lex_in * per, match_in * per)
    };
    let (h_lex, h_match) = phases_held();
    let (f_lex, f_match, f_drop) = phases_fresh();
    println!("  held buffer first:  lex {h_lex:.3} ms, match {h_match:.3} ms");
    println!("  then fresh buffer:  lex {f_lex:.3} ms, match {f_match:.3} ms, drop {f_drop:.3} ms");
    let (f_lex, f_match, f_drop) = phases_fresh();
    let (h_lex, h_match) = phases_held();
    println!("  fresh buffer first: lex {f_lex:.3} ms, match {f_match:.3} ms, drop {f_drop:.3} ms");
    println!("  then held buffer:   lex {h_lex:.3} ms, match {h_match:.3} ms");
    let guarded = trex::parse("\\W ~\"zzz\"").expect("pattern parses");
    let (pre_ms, _) =
        time(iters, || trex::prefilter::absent_guard_literals(&guarded, bytes).len());
    println!("  guard prefilter, one literal             {pre_ms:>8.3} ms");
    let (find_ms, _) = time(iters, || usize::from(trex::byte_simd::contains(bytes, b"zzz")));
    println!("  one absent literal searched directly     {find_ms:>8.3} ms");

    let toks = trex::lexer::lex(bytes);

    println!(
        "\n{:<34} {:>9} {:>9} {:>9} {:>7}",
        "pattern", "trex", "trex-lexed", "regex", "ratio"
    );
    for (label, tp, rp) in PAIRS {
        let p = trex::parse(tp).expect("trex pattern parses");
        let re = Regex::new(rp).expect("oracle regex compiles");
        let (t_full, n_full) = time(iters, || trex::scan(&p, bytes).len());
        let (t_lexed, _) = time(iters, || {
            trex::nfa::scan_nfa_over(&p, bytes, &toks).map_or(0, |m| m.len())
        });
        let (t_re, n_re) = time(iters, || re.find_iter(&text).count());
        println!(
            "  {label:<32} {t_full:>8.3} {t_lexed:>9.3} {t_re:>9.3} {:>6.1}x   {n_full} vs {n_re}",
            t_full / t_re
        );
    }

    println!("\nand what a regular expression cannot express at all");
    for (label, tp) in [
        ("balanced group", "\\B( \\W )"),
        ("bound then referenced", "\\W:n \"=\" =n"),
        ("token inside an assignment", "@super:assign \\N"),
    ] {
        let p = trex::parse(tp).expect("trex pattern parses");
        let (ms, n) = time(iters, || trex::scan(&p, bytes).len());
        println!("  {label:<32} {ms:>8.3} ms   {n} matches");
    }

    run_sweeps(&run, &on);
}
