//! A pattern opening with a literal takes the window route, and a window's
//! edge must not change what the match reads.
//!
//! The route lexes a window of bytes around each occurrence of the opening
//! literal rather than the whole input. Where that window's far edge falls
//! inside a token, the lex of the window can differ from the lex of the whole
//! input for bytes strictly inside it: a greedy recognizer stops at the edge,
//! and the bytes it gave up lex as tokens of their own. A match read from
//! such a window ends inside a token, which is a wrong span rather than a
//! missing one, so these assert spans and not merely that something matched.
//!
//! These run through the binary, so the pattern reaches the parser as written
//! rather than through a shell.

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

/// What `pattern` reports over `text`, as the one-input report prints it.
fn scan(pattern: &str, text: &str) -> String {
    let out = trex(&["scan", pattern, "--text", text, "--color", "never"]);
    assert!(out.status.success(), "{pattern}: {}", stderr(&out));
    stdout(&out)
}

#[test]
fn a_literal_before_a_token_a_window_would_cut_reports_the_whole_token() {
    // The window for this shape opens at twenty bytes and the input is
    // twenty-three, so the edge falls inside the timestamp. The cut piece,
    // `2026-09-16T12:04`, is itself a valid timestamp: a window that trusted
    // its own lex would report a match four bytes short and call it a match.
    let text = "at 2026-09-16T12:04:00Z";
    assert_eq!(scan("\"at\" \\T", text), "[0..23] \"at 2026-09-16T12:04:00Z\"\n");
    // The second atom does not decide it. A wildcard reads the same truncated
    // token, because the edge does its damage during the lex, before the
    // pattern asks anything of what it produced.
    assert_eq!(scan("\"at\" .", text), "[0..23] \"at 2026-09-16T12:04:00Z\"\n");
}

#[test]
fn every_route_to_the_same_match_reports_the_same_span() {
    // A kind atom, a class and a guarded atom reach this match without the
    // window route, or with a window the guard widens. The literal reaches it
    // through the window route alone, and the four must agree: which route
    // answered is an implementation detail and may not be visible in a span.
    let text = "at 2026-09-16T12:04:00Z";
    let whole = "[0..23] \"at 2026-09-16T12:04:00Z\"\n";
    assert_eq!(scan("\\W \\T", text), whole, "a kind atom");
    assert_eq!(scan("[\\W \\N] \\T", text), whole, "a class");
    assert_eq!(scan("\"at\" \\T{year=2026}", text), whole, "a guarded atom");
    assert_eq!(scan("\"at\" \\T", text), whole, "a literal");
}

#[test]
fn a_token_the_window_does_not_cut_is_read_from_the_window_it_has() {
    // The shorter forms fit inside the opening width, so the window is
    // already the lex the whole input gives and nothing widens. The window
    // route exists to avoid lexing whole inputs, so widening every window
    // would answer these correctly and cost what the route was built to save.
    assert_eq!(scan("\"at\" \\T", "at 2026-09-16T12:04"), "[0..19] \"at 2026-09-16T12:04\"\n");
    assert_eq!(scan("\"at\" \\T", "at 2026-09-16"), "[0..13] \"at 2026-09-16\"\n");
    assert_eq!(scan("\"code\" \\N", "code 200"), "[0..8] \"code 200\"\n");
}

#[test]
fn a_literal_before_a_long_token_holds_over_several_occurrences() {
    // The width a window settles at is carried to the next anchor in the same
    // leaf, so the second and third occurrences read a window that opened
    // wider than the first one did. They must report what the first reports.
    let text = "at 2026-09-16T12:04:00Z and at 2026-09-17T09:30:00Z and at 2026-09-18T22:15:00Z";
    assert_eq!(
        scan("\"at\" \\T", text),
        "[0..23] \"at 2026-09-16T12:04:00Z\"\n\
         [28..51] \"at 2026-09-17T09:30:00Z\"\n\
         [56..79] \"at 2026-09-18T22:15:00Z\"\n"
    );
}

#[test]
fn inference_can_verify_the_pattern_it_builds_from_two_timestamps() {
    // Inference checks its own output against every example, whole, before
    // printing it, and the pattern it builds from these is a literal followed
    // by a timestamp. So a span this route reports short is a pattern
    // inference cannot verify and must refuse: this asserts it need not.
    let out = trex(&["infer", "at 2026-09-16T12:04:00Z", "at 2026-09-16T12:59:00Z"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim_end(), "\"at\" \\T");
}
