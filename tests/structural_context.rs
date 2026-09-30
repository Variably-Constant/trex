//! `-A`, `-B` and `-C` given a record unit instead of a number: the whole
//! construct the match sits in, whatever its line count.
//!
//! Every expected block here is what the built binary printed.

use std::path::PathBuf;
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

/// A file of `contents` under a directory of this test's own, returned as the
/// path to pass to the binary.
fn fixture(name: &str, contents: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("trex/context-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the fixture directory");
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write the fixture");
    path
}

/// A function body holding a call, so the block a match sits in depends on
/// which match it is.
const CODE: &str = "header line\nfn outer(a) {\n  let v = inner(a, 42);\n  return 99;\n}\ntrailer line\n";

/// Log entries of several lines each, separated by blank lines.
const LOG: &str = "first entry\n  detail alpha\n  code 500\n\nsecond entry\n  detail beta\n  code 200\n";

/// What a scan that must succeed printed.
fn report(args: &[&str]) -> String {
    let out = trex(args);
    assert!(out.status.success(), "{args:?}: {}", stderr(&out));
    stdout(&out)
}

/// The message a refused scan printed.
fn refused(args: &[&str]) -> String {
    let out = trex(args);
    assert!(!out.status.success(), "{args:?} was accepted: {}", stdout(&out));
    stderr(&out)
}

#[test]
fn a_block_context_prints_the_balanced_group_the_match_sits_in() {
    let path = fixture("body.txt", CODE);
    let file = path.to_string_lossy().into_owned();
    // 99 sits in the brace group and in no group inside it, so the construct
    // is the whole body - four lines, which no count of context lines would
    // have known to ask for.
    assert_eq!(
        report(&["scan", "\\N{99..99}", &file, "-C", "block"]),
        "2-fn outer(a) {\n3-  let v = inner(a, 42);\n4:10: \"99\"\n5-}\n"
    );
    // 42 sits inside `inner(a, 42)`, which is the innermost group holding it
    // and lies on one line. The construct is that line, not the body it is
    // nested in: the question is which group this is inside, and the tightest
    // answer is the true one.
    assert_eq!(report(&["scan", "\\N{42..42}", &file, "-C", "block"]), "3:20: \"42\"\n");
}

#[test]
fn the_before_and_after_flags_clip_the_construct_to_one_side() {
    let path = fixture("body.txt", CODE);
    let file = path.to_string_lossy().into_owned();
    // The match's own line is printed either way, which is what makes these
    // narrower than `-C` rather than empty.
    assert_eq!(
        report(&["scan", "\\N{99..99}", &file, "-B", "block"]),
        "2-fn outer(a) {\n3-  let v = inner(a, 42);\n4:10: \"99\"\n"
    );
    assert_eq!(report(&["scan", "\\N{99..99}", &file, "-A", "block"]), "4:10: \"99\"\n5-}\n");
}

#[test]
fn a_paragraph_context_prints_the_whole_multi_line_entry() {
    let path = fixture("log.txt", LOG);
    let file = path.to_string_lossy().into_owned();
    assert_eq!(
        report(&["scan", "\\N{500..500}", &file, "-C", "paragraph"]),
        "1-first entry\n2-  detail alpha\n3:8: \"500\"\n"
    );
    // A count of lines cannot ask for the entry: one line short leaves the
    // entry's first line out, and one line long reaches into the next.
    assert_eq!(
        report(&["scan", "\\N{500..500}", &file, "-C", "1"]),
        "2-  detail alpha\n3:8: \"500\"\n4-\n"
    );
}

#[test]
fn record_names_whatever_the_record_flags_defined() {
    let path = fixture("log.txt", LOG);
    let file = path.to_string_lossy().into_owned();
    // `--record` says what a record is here rather than asking for a record
    // query: the report stays one line per match with the record around it.
    assert_eq!(
        report(&["scan", "\\N{500..500}", &file, "-C", "record", "--record", "paragraph"]),
        "1-first entry\n2-  detail alpha\n3:8: \"500\"\n"
    );
    // With nothing defining a record there is nothing to print, and saying so
    // beats printing the line alone as though the flag had been honored.
    let e = refused(&["scan", "\\N{500..500}", &file, "-C", "record"]);
    assert!(e.contains("read `record` from --record, --record-start or --record-span"), "{e}");
}

#[test]
fn a_unit_context_prints_the_statement_and_a_match_in_no_construct_stands_alone() {
    let path = fixture("body.txt", CODE);
    let file = path.to_string_lossy().into_owned();
    // The supertoken holding 99 is the statement it sits in, one line here.
    assert_eq!(report(&["scan", "\\N{99..99}", &file, "-C", "unit"]), "4:10: \"99\"\n");
    // `header` is inside no bracket group at all, so there is no construct to
    // print and the match reports its own line.
    assert_eq!(report(&["scan", "\\W{in:header}", &file, "-C", "block"]), "1:1: \"header\"\n");
}

#[test]
fn a_word_that_names_no_unit_and_two_different_units_are_both_refused() {
    let path = fixture("body.txt", CODE);
    let file = path.to_string_lossy().into_owned();
    let e = refused(&["scan", "\\N", &file, "-C", "nonsense"]);
    assert!(e.contains("the context flags take a number or a record unit"), "{e}");
    assert!(
        e.contains("write line, paragraph, file, period, seam, bind, bind:Q, auto, texture, shape, block, unit"),
        "{e}"
    );
    // One construct is printed around a match, so two names is a question
    // with no answer rather than a merge of them.
    let e = refused(&["scan", "\\N", &file, "-B", "block", "-A", "paragraph"]);
    assert!(e.contains("the context flags name one unit"), "{e}");
    // A number still means a count of lines, unchanged.
    assert_eq!(
        report(&["scan", "\\N{42..42}", &file, "-C", "1"]),
        "2-fn outer(a) {\n3:20: \"42\"\n4-  return 99;\n"
    );
}
