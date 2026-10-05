//! `trex tokens` and `trex escape` through the compiled binary: the tokens an
//! input is read as, and a text written as the pattern that matches it.

use std::path::PathBuf;
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

/// A directory of the test's own under the system's temporary one, removed
/// when the test ends.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/tokens-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Dir(dir)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }

    fn write(&self, name: &str, bytes: &[u8]) -> String {
        std::fs::write(self.0.join(name), bytes).expect("write input");
        self.path(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove temp dir");
    }
}

#[test]
fn each_token_prints_with_its_byte_span_and_kind() {
    let out = trex(&["tokens", "--text", "retry 3 times in 1500ms"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "[0..5] word \"retry\"\n[6..7] number \"3\"\n[8..13] word \"times\"\n[14..16] word \"in\"\n[17..23] duration \"1500ms\"\n"
    );
    // Whitespace is left out unless asked for.
    let out = trex(&["tokens", "--text", "a 1", "--whitespace"]);
    assert_eq!(stdout(&out), "[0..1] word \"a\"\n[1..2] whitespace \" \"\n[2..3] number \"1\"\n");
}

#[test]
fn json_carries_each_tokens_value_as_scan_json_writes_values() {
    let out = trex(&["tokens", "--text", "3 in 1500ms from 10.0.0.1", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out).trim(),
        concat!(
            r#"[{"kind":"number","start":0,"end":1,"text":"3","value":3},"#,
            r#"{"kind":"word","start":2,"end":4,"text":"in","value":null},"#,
            r#"{"kind":"duration","start":5,"end":11,"text":"1500ms","value":1500000000},"#,
            r#"{"kind":"word","start":12,"end":16,"text":"from","value":null},"#,
            r#"{"kind":"ip","start":17,"end":25,"text":"10.0.0.1","value":"167772161"}]"#
        )
    );
}

#[test]
fn a_declared_shape_is_one_token_of_its_own_name() {
    let dir = Dir::new("lib");
    let lib = dir.write("ticket.trex", b"shape ticket = `[A-Z]{2,4}-\\d{1,4}`\n");
    let out = trex(&["tokens", "--text", "see XYZ-9 now", "--lib", &lib]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "[0..3] word \"see\"\n[4..9] ticket \"XYZ-9\"\n[10..13] word \"now\"\n");
}

#[test]
fn a_binary_file_is_refused_unless_binary_asks_for_it() {
    let dir = Dir::new("binary");
    let bin = dir.write("d.bin", b"x 3\0\n");
    let out = trex(&["tokens", &bin]);
    assert!(!out.status.success(), "{}", stdout(&out));
    assert!(stderr(&out).contains("d.bin holds a NUL byte and is binary; --binary lexes it"), "{}", stderr(&out));
    let out = trex(&["tokens", &bin, "--binary"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).starts_with("[0..1] word \"x\"\n[2..3] number \"3\"\n"), "{}", stdout(&out));
    let text = dir.write("a.log", b"x 3\n");
    assert_eq!(stdout(&trex(&["tokens", &text])), "[0..1] word \"x\"\n[2..3] number \"3\"\n");
}

#[test]
fn escape_writes_the_pattern_that_matches_the_text_literally() {
    let out = trex(&["escape", "x.y"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let pattern = stdout(&out).trim().to_string();
    assert_eq!(pattern, "\"x\" \".\" \"y\"");
    let found = trex(&["scan", &pattern, "--text", "say x.y now xay"]);
    assert_eq!(stdout(&found), "[4..7] \"x.y\"\n");
    // One text, given whole.
    assert!(!trex(&["escape"]).status.success());
    let two = trex(&["escape", "a", "b"]);
    assert!(!two.status.success());
    assert!(stderr(&two).contains("quote a text holding spaces"), "{}", stderr(&two));
}
