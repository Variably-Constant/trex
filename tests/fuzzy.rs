//! Edit-distance matching over whole tokens: a literal within `k` edits and
//! a back-reference within `k` edits, through the front door and, forced by
//! a committed choice, the set-reachability engine, which must agree.

use trex::{parse, scan};

fn texts(pattern: &trex::ast::Pattern, bytes: &[u8]) -> Vec<String> {
    scan(pattern, bytes)
        .into_iter()
        .map(|s| String::from_utf8_lossy(&bytes[s.range()]).into_owned())
        .collect()
}

/// The matched texts of `pattern` over `input`, through the front door and
/// through the set engine, which must agree.
fn found(pattern: &str, input: &str) -> Vec<String> {
    let bytes = input.as_bytes();
    let pat = parse(pattern).unwrap_or_else(|e| panic!("{pattern}: {e:?}"));
    let direct = texts(&pat, bytes);
    let forced = parse(&format!("({pattern} |> [\\N && \\W])"))
        .unwrap_or_else(|e| panic!("forced {pattern}: {e:?}"));
    let through_set = texts(&forced, bytes);
    assert_eq!(direct, through_set, "{pattern}: the two engines disagree");
    direct
}

#[test]
fn a_literal_within_k_edits_takes_insertions_deletions_and_substitutions() {
    let words = "color colour colar colours collar";
    assert_eq!(found("\"color\"~1", words), vec!["color", "colour", "colar"]);
    assert_eq!(found("\"color\"~2", words), vec!["color", "colour", "colar", "colours", "collar"]);
    assert_eq!(found("\"color\"~0", words), vec!["color"]);
    // A transposition is two edits: one deletion and one insertion.
    assert_eq!(found("\"the\"~1", "the teh tho"), vec!["the", "tho"]);
    assert_eq!(found("\"the\"~2", "the teh tho"), vec!["the", "teh", "tho"]);
}

#[test]
fn a_back_reference_within_k_edits_reads_the_bound_token() {
    assert_eq!(found("\\W:w \"and\" =edit1 w", "color and colour"), vec!["color and colour"]);
    assert_eq!(found("\\W:w \"and\" =edit1 w", "color and colours"), Vec::<String>::new());
    assert_eq!(found("\\W:w \"and\" =edit2 w", "color and colours"), vec!["color and colours"]);
    assert_eq!(found("\\W:w \"and\" =edit1 w", "color and colour, tint and tone"), vec!["color and colour"]);
}

#[test]
fn edits_compose_with_an_orbit_group() {
    assert_eq!(
        found("(?orbit:case \"color\"~1)", "COLOUR Color colar tint"),
        vec!["COLOUR", "Color", "colar"]
    );
    assert_eq!(
        found("(?orbit:case \\W:w \"and\" =edit1 w)", "Color and COLOUR"),
        vec!["Color and COLOUR"]
    );
}

#[test]
fn the_edit_count_is_always_written() {
    assert!(parse("\"lit\"~").is_err(), "a bare tilde after a literal is not a count");
    assert!(parse("\"lit\"~300").is_err(), "the count fits one byte");
    // A tilde before a quote is still the content guard that follows the
    // literal, and a register named `edit` is still a register.
    let guard = parse("\"a\"~\"b\"").expect("a literal then a guard");
    assert_eq!(texts(&guard, b"a x b"), vec!["a"]);
    assert!(parse("\\W:edit =edit").is_ok());
}
