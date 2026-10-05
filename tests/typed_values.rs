//! `--values` and `--duration-unit`: a register binding a single typed kind
//! carries its parsed value beside its text in `--json`, in each of the three
//! spellings, with a duration in the unit asked for.
//!
//! These run the binary, because the flags and the JSON shape are what a
//! caller sees and neither is reachable from the library alone.

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

/// The value of the one register in a one-match scan.
fn value_of(args: &[&str]) -> String {
    let out = trex(args);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let at = text.find("\"value\":").unwrap_or_else(|| panic!("a value member in {text}"));
    let rest = &text[at + "\"value\":".len()..];
    // The member runs to the close of the capture object it is in, which is
    // the first brace or bracket that closes with nothing of its own open.
    let mut depth = 0i32;
    for (i, c) in rest.char_indices() {
        match c {
            '{' | '[' => depth += 1,
            '}' | ']' if depth == 0 => return rest[..i].trim().to_string(),
            '}' | ']' => depth -= 1,
            ',' if depth == 0 => return rest[..i].trim().to_string(),
            _ => {}
        }
    }
    panic!("an unterminated value in {text}")
}

#[test]
fn each_kind_reports_its_value_beside_its_text() {
    assert_eq!(value_of(&["scan", "\\Z:s", "--text", "size 4KB", "--json"]), "4000");
    assert_eq!(value_of(&["scan", "\\Z:s", "--text", "size 4KiB", "--json"]), "4096");
    assert_eq!(value_of(&["scan", "\\N:n", "--text", "n 1234", "--json"]), "1234");
    assert_eq!(
        value_of(&["scan", "\\T:t", "--text", "at 2026-09-15T00:00:00Z", "--json"]),
        "1789430400"
    );
    // An address is the integer it orders by, quoted because a v6 address is
    // 128 bits and a JSON number is a double.
    assert_eq!(
        value_of(&["scan", "\\I:h", "--text", "from 192.168.1.7", "--json"]),
        "\"3232235783\""
    );
    // A version is the two lists semver orders on; build metadata takes no
    // part in the order and is absent.
    assert_eq!(
        value_of(&["scan", "\\V:v", "--text", "v1.2.3-beta.2+build.7", "--json"]),
        "{\"parts\": [1, 2, 3], \"pre\": [\"beta\", \"2\"]}"
    );
    // Money keeps the digits past the point that a double would lose.
    assert_eq!(value_of(&["scan", "\\$:p", "--text", "paid $12.50", "--json"]), "\"12.5\"");
}

#[test]
fn the_three_spellings_differ_only_in_what_a_double_would_lose() {
    let money = ["scan", "\\$:p", "--text", "paid $12.50", "--json"];
    let with = |how: &str| {
        let mut args = money.to_vec();
        args.extend_from_slice(&["--values", how]);
        value_of(&args)
    };
    // Absent is exact, so the two agree.
    assert_eq!(value_of(&money), "\"12.5\"");
    assert_eq!(with("exact"), "\"12.5\"");
    assert_eq!(with("natural"), "12.5");
    assert_eq!(with("tagged"), "{\"kind\": \"money\", \"exact\": \"12.5\"}");
}

#[test]
fn a_duration_reports_nanoseconds_unless_another_unit_is_asked_for() {
    let took = ["scan", "\\R:d", "--text", "took 90s", "--json"];
    let in_unit = |unit: &str| {
        let mut args = took.to_vec();
        args.extend_from_slice(&["--duration-unit", unit]);
        value_of(&args)
    };
    assert_eq!(value_of(&took), "90000000000");
    assert_eq!(in_unit("ns"), "90000000000");
    assert_eq!(in_unit("ms"), "90000");
    assert_eq!(in_unit("s"), "90");
    // The shift is exact, so a sub-second duration keeps its fraction rather
    // than rounding through a double.
    let half = ["scan", "\\R:d", "--text", "took 1500ms", "--json", "--duration-unit", "s"];
    assert_eq!(value_of(&half), "\"1.5\"");
}

#[test]
fn a_register_with_no_one_kind_carries_no_value() {
    // A word has no value to parse, so the register stays the bare text a
    // report has always printed for it.
    let out = trex(&["scan", "\\W:w", "--text", "hello there", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("\"w\":\"hello\""), "{text}");
    assert!(!text.contains("\"value\""), "{text}");
}

#[test]
fn a_bad_word_names_itself_rather_than_only_failing() {
    for (flag, bad) in [("--values", "roughly"), ("--duration-unit", "fortnight")] {
        let out = trex(&["scan", "\\N:n", "--text", "n 1", "--json", flag, bad]);
        assert!(!out.status.success(), "{flag} {bad} should fail");
        let said = stderr(&out);
        assert!(said.contains(bad), "{flag}: {said}");
        assert!(said.contains(flag), "{flag}: {said}");
    }
}
