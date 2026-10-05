//! Linear-time immunity to adversarial input.
//!
//! A backtracking regular-expression engine explores every way to
//! divide a run of characters among nested repetitions, so the
//! pattern `(a+)+b` over a run of `a`s with no trailing `b` costs
//! time exponential in the run length. This is the classic
//! catastrophic-backtracking (ReDoS) shape that ships in many
//! widely-used regex engines.
//!
//! trex has no backtracking. The analogous nested-quantifier token
//! pattern `(.*)* \N` over a long stream of words with no number is
//! scanned in linear time by the single-pass engine. This example
//! runs both across a size sweep and self-checks the growth shapes:
//! the backtracker grows superlinearly (time roughly doubles per two
//! extra characters), trex stays flat per token. It also checks, from
//! the recorded trace, that the single-pass engine answered the trex
//! scan rather than a route that answers before the engine runs.
//!
//! Run: `cargo run --release --example linear_immunity`

use std::time::{Duration, Instant};

use trex::{parse, scan};

/// A node in a tiny regular expression, matched by the naive
/// backtracking algorithm below. This is deliberately the textbook
/// backtracker, not an optimized automaton: it is the reference whose
/// exponential blowup trex avoids.
enum Re {
    Char(u8),
    Seq(Vec<Re>),
    Group(Box<Re>),
    /// Greedy `*`: match the inner as many times as possible, giving
    /// back repetitions on failure. The inner of every `Star` used
    /// here consumes at least one byte, so the recursion terminates.
    Star(Box<Re>),
}

/// Match `re` starting at `pos`, calling `k` with each end position the
/// match can reach; returns true as soon as some path makes `k` true.
/// This continuation-passing shape is what makes the backtracking
/// explicit: `Star` tries the greedy branch first, then yields control
/// back to `k`, exactly the search a backtracking engine performs.
fn matches_at(re: &Re, s: &[u8], pos: usize, k: &dyn Fn(usize) -> bool) -> bool {
    match re {
        Re::Char(c) => pos < s.len() && s[pos] == *c && k(pos + 1),
        Re::Group(inner) => matches_at(inner, s, pos, k),
        Re::Seq(items) => match_seq(items, 0, s, pos, k),
        Re::Star(inner) => {
            // Greedy: consume one inner match and recurse on the star,
            // or stop here. The closure re-enters this same node.
            matches_at(inner, s, pos, &|p2| matches_at(re, s, p2, k)) || k(pos)
        }
    }
}

fn match_seq(items: &[Re], i: usize, s: &[u8], pos: usize, k: &dyn Fn(usize) -> bool) -> bool {
    if i == items.len() {
        return k(pos);
    }
    matches_at(&items[i], s, pos, &|p2| match_seq(items, i + 1, s, p2, k))
}

/// Build `(a+)+b`: an outer one-or-more over a group of an inner
/// one-or-more `a`, then a required `b`. Over a run of `a`s with no
/// `b`, this is the canonical exponential backtracking pattern. Each
/// `x+` is desugared to `x x*`.
fn catastrophic_pattern() -> Re {
    let inner_aplus = || Re::Seq(vec![Re::Char(b'a'), Re::Star(Box::new(Re::Char(b'a')))]);
    let group = || Re::Group(Box::new(inner_aplus()));
    let outer_plus = Re::Seq(vec![group(), Re::Star(Box::new(group()))]);
    Re::Seq(vec![outer_plus, Re::Char(b'b')])
}

/// Does the backtracker match? (It never will here: there is no `b`.)
/// The work is in the failing search, which is the point.
fn backtrack_matches(re: &Re, s: &[u8]) -> bool {
    matches_at(re, s, 0, &|end| end == s.len())
}

fn main() {
    println!("== backtracking reference: (a+)+b over a run of 'a' (no 'b') ==");
    println!("   a naive backtracking engine, the kind that ReDoSes\n");
    let re = catastrophic_pattern();
    let mut prev: Option<(usize, f64)> = None;
    let mut worst_ratio = 0.0f64;
    // Stop once a single run crosses this budget; exponential growth
    // makes the next step far slower, so the table always finishes.
    let budget = Duration::from_millis(1500);
    for n in (14..=40).step_by(2) {
        let input = vec![b'a'; n];
        let start = Instant::now();
        let matched = backtrack_matches(&re, &input);
        let secs = start.elapsed().as_secs_f64();
        assert!(!matched, "the pattern has no 'b'; it must not match");
        let us = secs * 1_000_000.0;
        if let Some((pn, pus)) = prev {
            let ratio = us / pus;
            worst_ratio = worst_ratio.max(ratio);
            println!("   n={n:>3} : {us:>12.1} us   ({ratio:.2}x over n={pn})");
        } else {
            println!("   n={n:>3} : {us:>12.1} us");
        }
        prev = Some((n, us));
        if start.elapsed() > budget {
            println!("   (stopping: a single run crossed the {} ms budget)", budget.as_millis());
            break;
        }
    }
    // Two extra characters roughly double the work: a linear engine
    // would be near 1.0x. A wide margin above that is the signature
    // of superlinear (here exponential) growth.
    assert!(
        worst_ratio > 1.6,
        "backtracker should grow superlinearly; worst step ratio was {worst_ratio:.2}x"
    );
    println!("   superlinear: worst two-character step was {worst_ratio:.2}x (a linear engine is ~1.0x)");

    println!("\n== trex: (.*)* \\N over a long stream of words with no number ==");
    println!("   the same nested-quantifier shape, scanned in one pass\n");
    let pat = parse(r"(.*)* \N").expect("pattern parses");
    let sizes = [50_000usize, 100_000, 200_000, 400_000, 800_000];

    // One untimed scan of every size with the rungs kept, so the timings
    // below are of the engine and do not price the trace.
    let recording = trex::trace::Recording::start();
    for &n in &sizes {
        assert!(scan(&pat, "a ".repeat(n).as_bytes()).is_empty(), "the adversarial pattern must not match");
        let rungs = trex::trace::take_recorded();
        assert!(
            rungs.iter().any(|r| r.ladder == "scan" && r.rung == "the single-pass engine over a whole lex"),
            "the single-pass engine must answer the scan of {n} tokens; the rungs were {rungs:?}"
        );
    }
    drop(recording);
    println!("   every size answered by: the single-pass engine over a whole lex\n");

    let mut samples: Vec<(usize, f64)> = Vec::new();
    for n in sizes {
        // n word tokens separated by spaces; no number appears.
        let input = "a ".repeat(n);
        let start = Instant::now();
        let matches = scan(&pat, input.as_bytes());
        let secs = start.elapsed().as_secs_f64();
        assert!(matches.is_empty(), "the adversarial pattern must not match");
        let ns_per_token = secs * 1e9 / n as f64;
        println!(
            "   N={n:>7} tokens : {ms:>7.1} ms   ({ns_per_token:>5.1} ns/token)",
            ms = secs * 1000.0
        );
        samples.push((n, ns_per_token));
    }
    // Linear means time per token is flat. Allow a generous band for
    // cache and warmup effects, but exponential or quadratic growth
    // would blow this by orders of magnitude.
    let first = samples.first().expect("at least one sample").1;
    let last = samples.last().expect("at least one sample").1;
    assert!(
        last < first * 3.0,
        "trex ns/token should stay flat (linear): {first:.1} -> {last:.1}"
    );
    println!("   linear: ns/token went {first:.1} -> {last:.1} across a 16x input growth");

    println!("\nThe backtracker's work explodes with input length; trex's does not.");
    println!("all self-checks passed.");
}

#[cfg(test)]
mod tests {
    /// Every claim here is an assertion inside `main`, so running it is the
    /// test. The manifest marks this example `test = true`, so `cargo test`
    /// runs it rather than only compiling it.
    #[test]
    fn every_self_check_holds() {
        super::main();
    }
}
