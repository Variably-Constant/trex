//! `--format`: one line per match from a report template, the match's
//! position beside its captures and typed slices, in the inline, file, tree
//! and stream reports; and the escapes every template reads.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_line_per_match_with_the_place_beside_the_captures() {
    let out = trex(&[
        "scan",
        "\\I:ip",
        "--format",
        "${line}:${col} ${ip:octet1-2}\\t${start}..${end}",
        "--text",
        "from 10.1.2.3\nto 192.168.0.1\n",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "1:6 10.1\t5..13\n2:4 192.168\t17..28\n");
    // A place field takes accessors like any other, and `${0}` is the match.
    let out = trex(&["scan", "\\N", "--format", "${0}", "--text", "a 1 b 22"]);
    assert_eq!(stdout(&out), "1\n22\n");
}

#[test]
fn a_tree_report_names_the_path() {
    let dir = std::env::temp_dir().join(format!("trex/format-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("a.log"), "from 10.0.0.1\n").expect("write a.log");
    std::fs::write(dir.join("b.log"), "to 192.168.1.9\n").expect("write b.log");
    let name = dir.to_string_lossy().into_owned();
    let out = trex(&["scan", "\\I", &name, "--format", "${path:name}:${line}:${col} ${0}"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "a.log:1:6 10.0.0.1\nb.log:1:4 192.168.1.9\n");
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn escapes_read_in_rewrite_templates_too() {
    let out = trex(&["rewrite", "\\W", "a\\tb", "--text", "x y"]);
    assert_eq!(stdout(&out), "a\tb a\tb");
    let out = trex(&["rewrite", "\\W", "$$\\\\", "--text", "x"]);
    assert_eq!(stdout(&out), "$\\");
    let out = trex(&["rewrite", "\\W", "\\q", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown escape \\q"), "{}", stderr(&out));
}

#[test]
fn a_format_takes_no_other_report() {
    let out = trex(&["scan", "\\I", "--format", "${0}", "--json", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--format prints one line per match"), "{}", stderr(&out));
    let out = trex(&["scan", "\\I", "--format", "${nosuch}", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--format error"), "{}", stderr(&out));
}

fn start(args: &[&str]) -> (Child, ChildStdin, Receiver<String>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_trex"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn trex");
    let stdin = child.stdin.take().expect("a piped stdin");
    let out = child.stdout.take().expect("a piped stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines() {
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

#[test]
fn a_stream_formats_each_match_as_it_commits() {
    let (mut child, mut stdin, rx) =
        start(&["scan", "\\W:w \"=\" \\N", "--format", "${line}:${col} ${w}=${0:last1} at ${start}"]);
    stdin.write_all(b"a = 1\n").expect("write");
    stdin.flush().expect("flush");
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)).expect("the first line"), "1:1 a=1 at 0");
    // A blank line between makes the next match the third line, counted
    // across the bytes the stream dropped.
    stdin.write_all(b"\n  b = 2\n").expect("write");
    stdin.flush().expect("flush");
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)).expect("the second line"), "3:3 b=2 at 9");
    drop(stdin);
    assert!(child.wait().expect("wait for the scan").success());
}
