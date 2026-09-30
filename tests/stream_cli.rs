//! The standard input scanned as it arrives: a match printed the moment the
//! stream scanner commits it, before the next chunk is written, and a
//! pattern that cannot commit early reporting what it retained.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

/// How long a test waits for a line that must come, or for one that must
/// not; a wrong implementation prints, or fails to, at once.
const WAIT: Duration = Duration::from_secs(30);
const QUIET: Duration = Duration::from_secs(2);

/// A running scan of the standard input, its output lines delivered as
/// they are printed.
fn start(args: &[&str]) -> (Child, ChildStdin, Receiver<String>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_trex"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn trex");
    let stdin = child.stdin.take().expect("a piped stdin");
    let stdout = child.stdout.take().expect("a piped stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(text) => {
                    if let Err(e) = tx.send(text) {
                        eprintln!("the test stopped listening: {e}");
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("the output ended: {e}");
                    break;
                }
            }
        }
    });
    (child, stdin, rx)
}

fn write(stdin: &mut ChildStdin, bytes: &[u8]) {
    stdin.write_all(bytes).expect("write to the scan");
    stdin.flush().expect("flush the scan's input");
}

fn next_line(rx: &Receiver<String>) -> String {
    rx.recv_timeout(WAIT).expect("a line the scan must print")
}

fn nothing_yet(rx: &Receiver<String>) {
    match rx.recv_timeout(QUIET) {
        Ok(line) => panic!("printed {line:?} before its match was final"),
        Err(RecvTimeoutError::Timeout) => {}
        Err(RecvTimeoutError::Disconnected) => panic!("the scan's output ended"),
    }
}

#[test]
fn a_match_is_printed_before_the_next_chunk_is_written() {
    let (mut child, mut stdin, rx) = start(&["scan", "\\W:w \"=\" \\N"]);
    write(&mut stdin, b"a = 1\n");
    assert_eq!(next_line(&rx), "[0..5] \"a = 1\"  captures: w=\"a\"");
    write(&mut stdin, b"b = 2\n");
    assert_eq!(next_line(&rx), "[6..11] \"b = 2\"  captures: w=\"b\"");
    drop(stdin);
    assert!(child.wait().expect("wait for the scan").success());
}

#[test]
fn a_chunk_ending_inside_a_token_is_held_until_the_token_ends() {
    let (mut child, mut stdin, rx) = start(&["scan", "\\W:w \"=\" \\N"]);
    // `1` may continue in the next chunk, so nothing is final yet.
    write(&mut stdin, b"a = 1");
    nothing_yet(&rx);
    write(&mut stdin, b"2\n");
    assert_eq!(next_line(&rx), "[0..6] \"a = 12\"  captures: w=\"a\"");
    drop(stdin);
    assert!(child.wait().expect("wait for the scan").success());
}

#[test]
fn json_on_a_stream_is_one_object_per_line() {
    let (mut child, mut stdin, rx) = start(&["scan", "\\W:w \"=\" \\N", "--json"]);
    write(&mut stdin, b"a = 1\n");
    assert_eq!(next_line(&rx), "{\"start\":0,\"end\":5,\"text\":\"a = 1\",\"captures\":{\"w\":\"a\"}}");
    write(&mut stdin, b"b = 2\n");
    assert_eq!(next_line(&rx), "{\"start\":6,\"end\":11,\"text\":\"b = 2\",\"captures\":{\"w\":\"b\"}}");
    drop(stdin);
    assert!(child.wait().expect("wait for the scan").success());
}

#[test]
fn a_pattern_that_cannot_commit_early_reports_what_it_retained() {
    let (child, mut stdin, rx) = start(&["scan", "\\W ~\"end\""]);
    write(&mut stdin, b"alpha end\n");
    // The guard reads forward without bound, so the match waits for the
    // end of the stream.
    nothing_yet(&rx);
    write(&mut stdin, b"beta\n");
    drop(stdin);
    assert_eq!(next_line(&rx), "[0..5] \"alpha\"");
    let out = child.wait_with_output().expect("wait for the scan");
    assert!(out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("15 bytes retained until the stream ended"), "{err}");
}

#[test]
fn a_stream_with_no_match_says_so_and_require_match_fails_it() {
    let (mut child, mut stdin, rx) = start(&["scan", "\\I", "--require-match"]);
    write(&mut stdin, b"nothing here\n");
    drop(stdin);
    assert_eq!(next_line(&rx), "no match");
    assert!(!child.wait().expect("wait for the scan").success());
}
