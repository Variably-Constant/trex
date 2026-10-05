//! What `infer` prints beyond a bare kind: the literal two texts share under
//! an orbit rung, and the value range a counter-example rules in.
//!
//! Every expected pattern here is what the built binary printed.

use std::process::{Command, Output};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

/// The pattern inferred from `args`, which must succeed.
fn inferred(args: &[&str]) -> String {
    let mut argv = vec!["infer"];
    argv.extend_from_slice(args);
    let out = trex(&argv);
    assert!(out.status.success(), "{args:?}: {}", stderr(&out));
    stdout(&out).trim_end().to_string()
}

/// The message a refused inference printed.
fn refused(args: &[&str]) -> String {
    let mut argv = vec!["infer"];
    argv.extend_from_slice(args);
    let out = trex(&argv);
    assert!(!out.status.success(), "{args:?} was accepted: {}", stdout(&out));
    stderr(&out)
}

#[test]
fn without_a_counter_example_a_position_reports_its_bare_kind() {
    // However well the values agree. Two numbers are a sample of two, and
    // with nothing to tell them from there is no evidence a range around
    // them means anything.
    assert_eq!(inferred(&["code 200", "code 204"]), "\"code\" \\N");
    assert_eq!(inferred(&["from 10.0.1.4", "from 10.0.9.7"]), "\"from\" \\I");
    assert_eq!(inferred(&["v 1.2.0", "v 1.9.3"]), "\"v\" \\V");
}

#[test]
fn a_counter_example_rules_a_range_in_and_the_range_rounds_outward() {
    // 200, 201 and 204 give the hundred they are in, not 200..204, which is
    // what the examples happened to show and what tomorrow's 206 would miss.
    assert_eq!(inferred(&["code 200", "code 204", "--not", "code 500"]), "\"code\" \\N{200..299}");
    // The place rounded to is the leading digit of the larger bound, so four
    // digits round to thousands.
    assert_eq!(inferred(&["port 8080", "port 8443", "--not", "port 22"]), "\"port\" \\N{8000..8999}");
    // An address rounds to the block its bits share, which is a boundary the
    // kind already has rather than one chosen for it.
    assert_eq!(
        inferred(&["from 10.0.1.4", "from 10.0.9.7", "--not", "from 192.168.0.1"]),
        "\"from\" \\I{in:10.0.0.0/20}"
    );
    // A version rounds to the line it is on.
    assert_eq!(inferred(&["v 1.2.0", "v 1.9.3", "--not", "v 2.0.0"]), "\"v\" \\V{major=1}");
}

#[test]
fn a_timestamp_rounds_to_the_calendar_unit_holding_every_example() {
    assert_eq!(
        inferred(&[
            "at 2026-09-16T12:04:00Z",
            "at 2026-09-16T12:59:00Z",
            "--not",
            "at 2026-09-16T18:00:00Z",
        ]),
        "\"at\" \\T{year=2026,month=9,day=16,hour=12}"
    );
    // Examples spanning two hours of one day round to the day, which is the
    // next unit out.
    assert_eq!(
        inferred(&[
            "at 2026-09-16T04:00:00Z",
            "at 2026-09-16T18:00:00Z",
            "--not",
            "at 2026-09-17T04:00:00Z",
        ]),
        "\"at\" \\T{year=2026,month=9,day=16}"
    );
}

#[test]
fn an_orbit_rung_folds_two_spellings_of_one_text_and_needs_no_counter_example() {
    // A rung covers exactly the texts the examples showed, under a folding,
    // where a range covers values none of them did. It is the extrapolation
    // that wants the evidence, so folding is not gated.
    assert_eq!(inferred(&["GET /a", "get /a"]), "(?orbit:case \"get\") \"/\" \"a\"");
    // `shape` would fold `cat` and `dog` to one consonant-vowel-consonant
    // span, which would match words no example resembled, so it is not a
    // rung inference folds to.
    assert_eq!(inferred(&["user cat", "user dog"]), "\"user\" \\W");
}

#[test]
fn a_counter_example_nothing_separates_is_refused_rather_than_answered() {
    // 201 is inside every band 200 and 204 share, so no range the examples
    // support excludes it. Printing a pattern that matches the thing the user
    // said it must miss would be the wrong answer, not a near miss.
    let e = refused(&["code 200", "code 204", "--not", "code 201"]);
    assert!(e.contains("still matches counter-example 1"), "{e}");
    assert!(e.contains("nothing the examples have in common tells them apart"), "{e}");
}

#[test]
fn the_flag_needs_an_example_and_the_anchors_still_apply() {
    let out = trex(&["infer", "code 200", "code 204", "--not"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--not needs an example the pattern must miss"), "{}", stderr(&out));
    assert_eq!(
        inferred(&["code 200", "code 204", "--not", "code 500", "--anchored"]),
        "^ \"code\" $ \\N{200..299}"
    );
}
