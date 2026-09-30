//! What the widening prefix costs against a whole lex, across where the first
//! match sits, which pattern is asked, and how large the input is.
//!
//! The prefix lexes a block, walks it, and widens until it settles. The lex is
//! appended at each cut so widening costs only the new bytes; the walk re-runs
//! over everything lexed so far, so its cost is the sum over rounds rather
//! than one pass. That sum is what a late match pays for and an early one
//! escapes, and the arms below are the policies that trade between them.
//!
//! `whole` lexes the input once and walks it once. `probe` lexes one prefix
//! and takes the whole input if that says nothing, so it pays one wasted block
//! at worst. The numbered arms widen without limit by that factor.
//!
//! Each cell is five rotating rounds reporting the middle reading and the
//! spread, largest over smallest: a spread far from one says the cell moved
//! while it was timed and is not a reading.

use std::hint::black_box;
use std::time::Instant;

/// Growth factors to read the curve at, widening without limit.
const GROWTH: &[usize] = &[2, 4, 16];

const ROUNDS: usize = 5;

/// Where the first match sits, in per mille of the way through. Past a
/// thousand means nowhere.
const PLACES: &[(&str, usize)] = &[
    ("at 1 per mille", 1),
    ("at 250", 250),
    ("at 500", 500),
    ("at 990", 990),
    ("nowhere", 2000),
];

/// Patterns no route answers, so both arms reach the engine. Each needs a
/// marker whose shape it matches, given beside it.
/// Reaching it means no byte-routable literal, no kind sequence and no
/// contained literal for a window, which rules out most shapes: a pattern
/// naming any literal at all is usually taken by the window route. What is
/// left is kinds repeated or alternated, and binding without a literal.
const PATTERNS: &[(&str, &str)] = &[
    ("two adjacent words", "\\W{2}"),
    ("three adjacent numbers", "\\N{3}"),
    ("two adjacent words or numbers", "(\\W | \\N){2}"),
    ("a word repeated", "\\W:x =x"),
    ("a word then a number asserted", "\\W ~(\\N)"),
];

/// The marker line each pattern needs, in the same order.
const MARKERS: &[&str] = &[
    "alpha beta ;\n",
    "1 2 3 ;\n",
    "alpha beta ;\n",
    "alpha alpha ;\n",
    "alpha 7 ;\n",
];

/// Statements holding nothing any of the patterns match, so a marker's
/// position is where the first match is.
fn filler(i: usize) -> String {
    match i % 3 {
        0 => format!("key_{i} : {} ;\n", i * 37),
        1 => format!("call_{i}({i}) ;\n"),
        _ => format!("item_{i} : {} ;\n", i + 1),
    }
}

/// `statements` lines of filler with `marker` at `at_share` per mille, or
/// nowhere when `at_share` is past a thousand.
fn marked(statements: usize, marker: &str, at_share: usize) -> Vec<u8> {
    let mut s = String::new();
    let at = statements * at_share / 1000;
    for i in 0..statements {
        if i == at && at_share <= 1000 {
            s.push_str(marker);
        }
        s.push_str(&filler(i));
    }
    s.into_bytes()
}

/// Time `f` until `budget_ms` has passed and at least five calls have run.
fn time_budget(budget_ms: f64, mut f: impl FnMut() -> usize) -> (f64, usize) {
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
    (t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters), n)
}

/// The middle of `v` and its spread, the largest reading over the smallest.
fn median_and_spread(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("a measured time is never NaN"));
    (v[v.len() / 2], v[v.len() - 1] / v[0])
}

/// Five rounds of `f`, reporting the middle, the spread and what it answered.
fn cell(mut f: impl FnMut() -> usize) -> (f64, f64, usize) {
    let mut v = Vec::with_capacity(ROUNDS);
    let mut got = 0usize;
    for _ in 0..ROUNDS {
        let (t, n) = time_budget(25.0, &mut f);
        v.push(t);
        got = n;
    }
    let (ms, spread) = median_and_spread(v);
    (ms, spread, got)
}

fn main() {
    println!("trex {}", trex::version());
    for size in [50_000usize, 200_000] {
        for (pi, (label, src)) in PATTERNS.iter().enumerate() {
            let p = match trex::parse(src) {
                Ok(p) => p,
                Err(e) => {
                    println!("\n`{src}` does not parse: {e:?}");
                    continue;
                }
            };
            for (where_label, share) in PLACES {
                let input = marked(size, MARKERS[pi], *share);
                // Said rather than assumed: a route answering this pattern
                // would make every number below a reading of that route.
                if trex::engine::routed_spans_public(&p, &input).is_some() {
                    println!("\n  a route answers `{src}`, so nothing below times the prefix");
                    break;
                }
                let want = trex::scan(&p, &input).first().copied();
                println!(
                    "\n=== {label} `{src}`   first match {where_label}   {} bytes ===",
                    input.len()
                );
                println!("  {:>9} {:>10} {:>8} {:>10}", "arm", "ms", "spread", "vs whole");

                // Two whole-input arms, because they differ in the thing that
                // otherwise confounds every ratio below. `par` reaches the
                // multi-core lexer; `ser` lexes through the same serial path
                // the prefix arms use, so prefix against `ser` is a policy
                // reading and `ser` against `par` is what the lexer is worth.
                let (par_ms, par_spread, _) = cell(|| {
                    trex::cursor::find_by_full_lex(&p, &input).map_or(0, |s| s.start() + 1)
                });
                println!("  {:>9} {par_ms:>10.3} {par_spread:>7.2}x {:>10}", "whole par", "-");

                // A cap of no prefixes takes the whole input on the first
                // round, which is one serial lex and one walk.
                let (whole_ms, whole_spread, _) = cell(|| {
                    trex::prefilter::find_by_growing_prefix_at(&p, &input, 2, 0)
                        .flatten()
                        .map_or(0, |s| s.start() + 1)
                });
                println!(
                    "  {:>9} {whole_ms:>10.3} {whole_spread:>7.2}x {:>9.2}x",
                    "whole ser",
                    whole_ms / par_ms
                );

                // The same whole-input round through the same walk, lexed by
                // the multi-core lexer. Against `whole ser` it varies only the
                // lexer, and against `whole par` only the walk, so the two
                // things `whole ser` over `whole par` had summed are separate.
                let (sp_ms, sp_spread, _) = cell(|| {
                    trex::prefilter::find_by_growing_prefix_lexed(&p, &input, 2, 0, true)
                        .flatten()
                        .map_or(0, |s| s.start() + 1)
                });
                println!(
                    "  {:>9} {sp_ms:>10.3} {sp_spread:>7.2}x {:>9.2}x   lexer alone {:.2}x",
                    "whole sp",
                    sp_ms / par_ms,
                    sp_ms / whole_ms
                );

                // Each policy twice, once per lexer, because the lexer is
                // most of what separates a prefix from a whole-input lex on
                // this many cores and the two questions must not be summed.
                let mut arms: Vec<(String, usize, usize, bool)> =
                    vec![("probe".into(), 4, 1, false), ("probe par".into(), 4, 1, true)];
                for &g in GROWTH {
                    arms.push((g.to_string(), g, usize::MAX, false));
                    arms.push((format!("{g} par"), g, usize::MAX, true));
                }
                for (name, growth, cap, parallel) in arms {
                    // A refusal and a wrong answer must not read alike. The
                    // outer None is the prefix declining the pattern, which
                    // is correct behavior; the inner is a verdict of no
                    // match, which is an answer and can be wrong. Collapsing
                    // them would make a real false negative look exactly like
                    // a correct refusal, and that is what this row is for.
                    const DECLINED: usize = usize::MAX;
                    let (ms, spread, got) = cell(|| {
                        match trex::prefilter::find_by_growing_prefix_lexed(
                            &p, &input, growth, cap, parallel,
                        ) {
                            None => DECLINED,
                            Some(answer) => answer.map_or(0, |s| s.start() + 1),
                        }
                    });
                    let note = if got == DECLINED {
                        "  declined, not timed"
                    } else if got == want.map_or(0, |s| s.start() + 1) {
                        ""
                    } else {
                        "  disagrees with the scan"
                    };
                    println!(
                        "  {name:>9} {ms:>10.3} {spread:>7.2}x {:>9.2}x{note}",
                        ms / whole_ms
                    );
                }
            }
        }
    }
    println!("\nDONE");
}
