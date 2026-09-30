//! `--sum --avg --min --max --p50 --p95` over a typed capture on `count-by`
//! and `top`, the percentile methods, the exact average, and the refusals
//! that happen before anything is scanned.
//!
//! These run the binary: the flags, the refusal messages and the column
//! layout are what a caller sees and none is reachable from the library.

use std::process::{Command, Output};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn ok(args: &[&str]) -> String {
    let out = trex(args);
    assert!(out.status.success(), "{}", stderr(&out));
    stdout(&out)
}

/// Four sizes under two keys. The keys are single words on purpose: the
/// lexer splits `a.example` at the dot, so a `\W` register would bind only
/// `example` and the table would group by a key the test did not intend.
const SIZES: &str = "alpha 1KB\nbeta 8KB\nalpha 2KB\nalpha 4KB\n";

/// Four sizes under one key, so a percentile falls between two observed
/// values and the four methods can disagree. Three values put the position
/// on a value and every method agrees, which proves nothing about any of them.
const FOUR: &str = "alpha 10B\nalpha 20B\nalpha 30B\nalpha 40B\n";

#[test]
fn a_sum_and_an_average_read_in_the_kinds_base_unit() {
    // Bytes, on KB of 1000: 1000 + 2000 + 4000 is 7000 over three matches.
    let table = ok(&["count-by", "\\W:h \\Z:s", "${h}", "--text", SIZES, "--sum", "${s}"]);
    assert!(table.contains("7000"), "{table}");
    assert!(table.contains("8000"), "{table}");

    // The average of those three is exact and terminates.
    let avg = ok(&["count-by", "\\W:h \\Z:s", "${h}", "--text", SIZES, "--avg", "${s}"]);
    assert!(avg.contains("2333.(3)"), "{avg}");

    // The same average as a fraction in lowest terms.
    let rat = ok(&[
        "count-by", "\\W:h \\Z:s", "${h}", "--text", SIZES, "--avg", "${s}", "--avg-form",
        "rational",
    ]);
    assert!(rat.contains("7000/3"), "{rat}");
}

#[test]
fn min_and_max_read_the_ends_of_the_order() {
    let table = ok(&[
        "count-by", "\\W:h \\Z:s", "${h}", "--text", SIZES, "--min", "${s}", "--max", "${s}",
    ]);
    assert!(table.contains("1000"), "{table}");
    assert!(table.contains("4000"), "{table}");
}

#[test]
fn the_four_percentile_methods_differ_where_the_position_falls_between_values() {
    // Four values, 10 20 30 40 bytes, so p50 lands between the second and
    // third and the methods part. Three values would put it on one and every
    // method would agree, which would prove nothing.
    let run = |method: &str| {
        ok(&[
            "count-by", "\\W:h \\Z:s", "${h}", "--text", FOUR, "--p50", "${s}", "--percentile",
            method,
        ])
    };
    assert!(run("nearest").contains(" 20"), "{}", run("nearest"));
    assert!(run("lower").contains(" 20"), "{}", run("lower"));
    // Interpolated, and exact: halfway between 20 and 30.
    assert!(run("linear").contains(" 25"), "{}", run("linear"));
    // A byte size is numeric, so hybrid interpolates it as linear does.
    assert!(run("hybrid").contains(" 25"), "{}", run("hybrid"));

    // p95 over the same four is the last value under nearest.
    let p95 = ok(&[
        "count-by", "\\W:h \\Z:s", "${h}", "--text", FOUR, "--p95", "${s}", "--percentile",
        "nearest",
    ]);
    assert!(p95.contains(" 40"), "{p95}");
}

#[test]
fn several_aggregates_make_several_columns() {
    let table = ok(&[
        "count-by", "\\W:h \\Z:s", "${h}", "--text", SIZES, "--sum", "${s}", "--min", "${s}",
        "--max", "${s}",
    ]);
    // The heading names each column by its flag and its capture, so two
    // aggregates over one capture are told apart.
    assert!(table.contains("--sum s"), "{table}");
    assert!(table.contains("--min s"), "{table}");
    assert!(table.contains("--max s"), "{table}");
}

#[test]
fn an_aggregate_a_kind_cannot_carry_is_refused_before_the_scan() {
    // A word carries no value to sum, and the refusal names the register, the
    // kind and the kinds the aggregate takes.
    let out = trex(&["count-by", "\\W:w", "${w}", "--text", "hello there", "--sum", "${w}"]);
    assert!(!out.status.success());
    let said = stderr(&out);
    assert!(said.contains("--sum"), "{said}");
    assert!(said.contains("word"), "{said}");
    assert!(said.contains("number"), "{said}");

    // A version orders but does not add, so --sum is refused and --min is not.
    let bad = trex(&["count-by", "\\V:v", "${v}", "--text", "v1.2.3", "--sum", "${v}"]);
    assert!(!bad.status.success(), "{}", stdout(&bad));
    assert!(stderr(&bad).contains("version"), "{}", stderr(&bad));
    let good = trex(&["count-by", "\\V:v", "${v}", "--text", "v1.2.3", "--min", "${v}"]);
    assert!(good.status.success(), "{}", stderr(&good));
}

#[test]
fn linear_is_refused_where_it_cannot_interpolate_and_hybrid_is_not() {
    let args = ["count-by", "\\V:v", "${v}", "--text", "v1.2.3 v1.10.0", "--p50", "${v}"];
    let mut linear = args.to_vec();
    linear.extend_from_slice(&["--percentile", "linear"]);
    let out = trex(&linear);
    assert!(!out.status.success(), "{}", stdout(&out));
    let said = stderr(&out);
    assert!(said.contains("linear"), "{said}");
    assert!(said.contains("version"), "{said}");
    // The message names the ways through rather than only refusing.
    assert!(said.contains("hybrid"), "{said}");

    let mut hybrid = args.to_vec();
    hybrid.extend_from_slice(&["--percentile", "hybrid"]);
    let out = trex(&hybrid);
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn an_aggregate_naming_no_single_capture_is_refused() {
    // A slice of text is not a value to sum, and a composite names two.
    for spec in ["${s:upper}", "${h}/${s}", "${nosuch}"] {
        let out = trex(&["count-by", "\\W:h \\Z:s", "${h}", "--text", SIZES, "--sum", spec]);
        assert!(!out.status.success(), "{spec} should be refused: {}", stdout(&out));
        assert!(stderr(&out).contains("--sum"), "{spec}: {}", stderr(&out));
    }
}

#[test]
fn a_bad_method_or_form_names_the_word_back() {
    for (flag, bad) in [("--percentile", "median"), ("--avg-form", "decimal")] {
        let out =
            trex(&["count-by", "\\Z:s", "${s}", "--text", SIZES, "--sum", "${s}", flag, bad]);
        assert!(!out.status.success(), "{flag} {bad} should fail");
        let said = stderr(&out);
        assert!(said.contains(bad), "{flag}: {said}");
        assert!(said.contains(flag), "{flag}: {said}");
    }
}

#[test]
fn the_json_report_carries_the_columns() {
    let out = ok(&[
        "count-by", "\\W:h \\Z:s", "${h}", "--text", SIZES, "--sum", "${s}", "--json",
    ]);
    assert!(out.contains("\"--sum s\""), "{out}");
    assert!(out.contains("7000"), "{out}");
}

#[test]
fn a_table_with_no_aggregate_is_unchanged() {
    // The rows a caller asking for no aggregate has always seen: no heading,
    // no columns, the count straight after the key.
    let plain = ok(&["count-by", "\\W:h \\Z:s", "${h}", "--text", SIZES]);
    assert_eq!(plain, "alpha  3\nbeta   1\n");
}
