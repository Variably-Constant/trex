//! trex next to a regular-expression engine: pattern density, the
//! patterns a regular-expression engine cannot compile, and trex's
//! linear scaling on an adversarial input.
//!
//! This is a self-checking demonstration, not a throughput contest:
//! trex tokenizes first and is not claimed to out-throughput a
//! finely-tuned byte regex on simple patterns. The point is what trex
//! expresses (binding, balance, guards) that a regular-expression
//! engine rejects or cannot state, the shorter patterns, and the
//! absence of a catastrophic-backtracking cliff.
//!
//! Run: `cargo run --release --example vs_regex`

use std::time::Instant;

use regex::Regex;
use trex::{parse, scan};

struct Row {
    task: &'static str,
    trex: &'static str,
    regex: Option<&'static str>,
}

fn main() {
    let rows = [
        Row { task: "number + unit", trex: r"\N \W", regex: Some(r"[-+]?\d+(?:\.\d+)?\s+[A-Za-z_]\w*") },
        Row { task: "key = quoted", trex: r#"\W:k = \Q:v"#, regex: Some(r#"(\w+)\s*=\s*"([^"]*)""#) },
        Row { task: "matched tag", trex: r"<\W:t>.*</=t>", regex: Some(r"<(\w+)[^>]*>.*?</\1>") },
        Row { task: "repeated token", trex: r"\W:x =x", regex: Some(r"\b(\w+)\s+\1\b") },
        Row { task: "IP has ERROR", trex: r#"\I ~"ERROR""#, regex: Some(r"(?=.*ERROR).*\b\d{1,3}(\.\d{1,3}){3}\b") },
        Row { task: "balanced call", trex: r"\W\B(.*)", regex: None },
    ];

    println!("== pattern density and regular-expression-engine support ==");
    // Column headers are intentionally literals aligned to the rows.
    #[allow(clippy::print_literal)]
    {
        println!("{:<16} {:>5} {:>6}  {}", "task", "trex", "regex", "regex engine");
    }
    for r in &rows {
        let (rlen, support) = match r.regex {
            None => ("  -".to_string(), "not expressible".to_string()),
            Some(re) => {
                let s = if Regex::new(re).is_ok() { "compiles" } else { "REJECTED (backref/lookaround)" };
                (re.len().to_string(), s.to_string())
            }
        };
        println!("{:<16} {:>5} {:>6}  {}", r.task, r.trex.len(), rlen, support);
    }

    println!("\n== capability: trex matches these; the regex engine rejects or cannot state them ==");
    check(r"\W:x =x", "the the cat", "the the");
    check(r"<\W:t>.*</=t>", "<div>hi</div>", "<div>hi</div>");
    check(r"\W\B(.*)", "call f(g(x)) end", "f(g(x))");
    // Deliberately an invalid regex: the point is that the engine
    // rejects the backreference, which trex matches in linear time.
    #[allow(clippy::invalid_regex)]
    {
        assert!(
            Regex::new(r"(\w+)\s+\1").is_err(),
            "the regex engine should reject a backreference"
        );
    }
    println!("  regex engine rejects the backreference (\\w+)\\s+\\1 : confirmed");

    println!("\n== linear scaling on a nested-quantifier adversarial that ReDoSes a backtracker ==");
    let pat = parse(r#"(.*)* "ZZZ""#).expect("pattern parses");
    for n in [50_000usize, 100_000, 200_000, 400_000] {
        let input = "a ".repeat(n);
        let start = Instant::now();
        let matches = scan(&pat, input.as_bytes());
        let ms = start.elapsed().as_millis();
        assert!(matches.is_empty(), "the adversarial pattern must not match");
        println!("  N={n:>7} tokens : {ms} ms");
    }

    println!("\nall self-checks passed.");
}

fn check(pattern: &str, input: &str, expected: &str) {
    let p = parse(pattern).expect("pattern parses");
    let matches = scan(&p, input.as_bytes());
    assert!(!matches.is_empty(), "pattern {pattern} should match {input:?}");
    let got = &input[matches[0].range()];
    assert_eq!(got, expected, "pattern {pattern} on {input:?}");
    println!("  {pattern:<16} on {input:?} -> {got:?}");
}

#[cfg(test)]
mod tests {
    /// Every claim here is an assertion inside `main`, so running it is the
    /// test. The manifest marks this example `test = true`, so `cargo test`
    /// runs it rather than only compiling it.
    #[test]
    fn every_comparison_holds() {
        super::main();
    }
}
