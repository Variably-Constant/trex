//! Time read against the stream rather than against the clock: an instant
//! measured from the one a register holds, and how a timestamp compares with
//! the timestamp before it. Every case runs through the front
//! door and, forced by a committed choice, through the set engine.

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

/// Four records: half a minute apart, then a two-hour gap, then a stamp an
/// hour behind the one before it.
const LOG: &str = concat!(
    "a 2026-09-15T10:00:00Z b 2026-09-15T10:00:30Z ",
    "c 2026-09-15T12:00:00Z d 2026-09-15T11:00:00Z e"
);

#[test]
fn an_instant_reads_against_the_one_a_register_holds() {
    // A gap: the next stamp is more than an hour after the bound one.
    assert_eq!(
        found("\\T:t \\W \\T{>+1h:t}", LOG),
        vec!["2026-09-15T10:00:30Z c 2026-09-15T12:00:00Z"]
    );
    // No stamp is more than three hours after the bound one.
    assert!(found("\\T:t \\W \\T{>+3h:t}", LOG).is_empty());
    // A stamp more than half an hour before the bound one: the skew.
    assert_eq!(
        found("\\T:t \\W \\T{<-30m:t}", LOG),
        vec!["2026-09-15T12:00:00Z d 2026-09-15T11:00:00Z"]
    );
    // `<` alone admits a stamp before the bound one as well as one close
    // after it, so a burst that must run forward names the direction too.
    assert_eq!(found("\\T:t \\W \\T{<+1m:t}", LOG).len(), 2);
    assert_eq!(
        found("\\T:t \\W (@order:asc \\T{<+1m:t})", LOG),
        vec!["2026-09-15T10:00:00Z b 2026-09-15T10:00:30Z"]
    );
    // The bound register must hold an instant for the reading to mean
    // anything, and a word does not.
    assert!(found("\\W:t \\T{>+1h:t}", LOG).is_empty());
}

#[test]
fn the_order_anchor_reads_the_direction_of_the_stream() {
    assert_eq!(found("@order:desc \\T", LOG), vec!["2026-09-15T11:00:00Z"]);
    assert_eq!(
        found("@order:asc \\T", LOG),
        vec!["2026-09-15T10:00:30Z", "2026-09-15T12:00:00Z"]
    );
    // The first stamp of an input has nothing to compare with, so neither
    // direction holds of it, and a token that is no stamp holds neither.
    assert!(found("@order:asc \\T", "a 2026-09-15T10:00:00Z b").is_empty());
    assert!(found("@order:desc \\T", "a 2026-09-15T10:00:00Z b").is_empty());
    assert!(found("@order:asc \\W", LOG).is_empty());
    // Records running backward throughout are all descending after the
    // first.
    let backward = "2026-09-15T12:00:00Z x 2026-09-15T11:00:00Z y 2026-09-15T10:00:00Z";
    assert_eq!(found("@order:desc \\T", backward).len(), 2);
    assert!(found("@order:asc \\T", backward).is_empty());
}

#[test]
fn a_malformed_reading_is_a_parse_error_that_names_the_form() {
    let err = parse("\\T{>+5:t}").expect_err("a duration needs a unit");
    assert!(err.msg.contains("not a duration"), "{}", err.msg);
    let err = parse("@order").expect_err("a direction is required");
    assert!(err.msg.contains("direction"), "{}", err.msg);
    let err = parse("@order:sideways").expect_err("an unknown direction");
    assert!(err.msg.contains("asc, desc"), "{}", err.msg);
    // An absolute instant keeps its own reading, colons and all.
    assert!(parse("\\T{>2026-09-01T12:00:00Z}").is_ok());
    assert!(parse("\\T{age<24h}").is_ok());
}
