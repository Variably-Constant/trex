//! The echo axis read inside a pattern: how often a token's content recurs,
//! which occurrence this one is, and whether the recurrence is regular. Every
//! case runs through the front door and, forced by a committed choice,
//! through the set-reachability engine, and the two must agree.

use trex::{parse, scan};

fn texts(pattern: &trex::ast::Pattern, bytes: &[u8]) -> Vec<String> {
    scan(pattern, bytes)
        .into_iter()
        .map(|s| String::from_utf8_lossy(&bytes[s.range()]).into_owned())
        .collect()
}

/// The matched texts of `pattern` over `input`, through both engines.
fn found(pattern: &str, input: &str) -> Vec<String> {
    let bytes = input.as_bytes();
    let pat = parse(pattern).unwrap_or_else(|e| panic!("{pattern}: {e:?}"));
    let direct = texts(&pat, bytes);
    let forced = parse(&format!("({pattern} |> [\\N && \\W])"))
        .unwrap_or_else(|e| panic!("forced {pattern}: {e:?}"));
    assert_eq!(direct, texts(&forced, bytes), "{pattern}: the two engines disagree");
    direct
}

/// Three alphas, two betas, one gamma.
const REPEATS: &str = "alpha beta alpha gamma alpha beta";

#[test]
fn the_count_reads_how_often_the_content_recurs() {
    assert_eq!(found("@echo>2 \\W", REPEATS), vec!["alpha", "alpha", "alpha"]);
    assert_eq!(found("@echo>=3 \\W", REPEATS), vec!["alpha", "alpha", "alpha"]);
    assert_eq!(found("@echo=1 \\W", REPEATS), vec!["gamma"]);
    assert_eq!(found("@echo=2 \\W", REPEATS), vec!["beta", "beta"]);
    assert_eq!(found("@echo<2 \\W", REPEATS), vec!["gamma"]);
    assert_eq!(found("@echo!=1 \\W", REPEATS).len(), 5);
    // A bare `@echo` is content that recurs at all, which is `@echoed`.
    assert_eq!(found("@echo \\W", REPEATS), found("@echoed \\W", REPEATS));
    assert_eq!(found("@echo \\W", REPEATS).len(), 5);
}

#[test]
fn the_index_counts_from_the_first_and_from_the_last() {
    // Each word's first occurrence, which is what `@novel` says.
    assert_eq!(found("@echo:nth=1 \\W", REPEATS), found("@novel \\W", REPEATS));
    assert_eq!(found("@echo:nth=1 \\W", REPEATS), vec!["alpha", "beta", "gamma"]);
    assert_eq!(found("@echo:nth=2 \\W", REPEATS), vec!["alpha", "beta"]);
    assert_eq!(found("@echo:nth=3 \\W", REPEATS), vec!["alpha"]);
    assert_eq!(found("@echo:nth>1 \\W", REPEATS).len(), 3);
    // Counting back from the last: every word's last occurrence, then the
    // one before it.
    assert_eq!(found("@echo:nth=-1 \\W", REPEATS), vec!["gamma", "alpha", "beta"]);
    assert_eq!(found("@echo:nth=-2 \\W", REPEATS), vec!["beta", "alpha"]);
    // A token occurring once is both the first and the last.
    assert_eq!(found("@echo:nth=1 \\W", "gamma"), vec!["gamma"]);
    assert_eq!(found("@echo:nth=-1 \\W", "gamma"), vec!["gamma"]);
}

#[test]
fn the_period_reads_a_regular_recurrence() {
    // `req` is at bytes 0, 7 and 14: a lag of seven, twice.
    let regular = "req a1 req a2 req a3";
    assert_eq!(found("@echo:period \\W", regular), vec!["req", "req", "req"]);
    assert_eq!(found("@echo:period=7 \\W", regular), vec!["req", "req", "req"]);
    assert!(found("@echo:period=9 \\W", regular).is_empty());
    assert_eq!(found("@echo:period>5 \\W", regular).len(), 3);
    // Two occurrences give one lag, which is not yet a spacing.
    assert!(found("@echo:period \\W", "req a1 req").is_empty());
}

#[test]
fn an_orbit_scope_counts_recurrence_at_its_rung() {
    let cased = "Alpha alpha beta";
    // Exactly, the two spellings are different content.
    assert!(found("@echo>1 \\W", cased).is_empty());
    // Up to case, they are one.
    assert_eq!(found("(?orbit:case @echo>1 \\W)", cased), vec!["Alpha", "alpha"]);
    assert_eq!(found("(?orbit:case @echo:nth=2 \\W)", cased), vec!["alpha"]);
    // The scope reaches only what it encloses, so a rung named around one
    // anchor leaves another outside it counting exactly.
    assert_eq!(found("(?orbit:case @echo>1) \\W", cased), vec!["Alpha", "alpha"]);
}

#[test]
fn an_unkeyed_token_carries_no_recurrence() {
    // Punctuation and brackets recur by grammar rather than by content, so
    // the axis does not key them and no reading of it holds.
    assert!(found("@echo>1 \\P", "a , b , c ,").is_empty());
    assert!(found("@echo:nth=1 \\P", "a , b , c ,").is_empty());
}

#[test]
fn a_malformed_reading_is_a_parse_error_that_names_the_forms() {
    let err = parse("@echo:nth=0").expect_err("there is no zeroth occurrence");
    assert!(err.msg.contains("counts from one"), "{}", err.msg);
    let err = parse("@echo:bogus").expect_err("an unknown reading");
    assert!(err.msg.contains("@echo:nth"), "{}", err.msg);
    let err = parse("@echo:nth").expect_err("an index needs an operator");
    assert!(err.msg.contains("operator"), "{}", err.msg);
}
