//! The count-by, top and uniq commands, driven through the compiled binary:
//! matches grouped by a key rendered from the rewrite template language, so
//! a capture's typed slice is what the rows count.

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
        let dir = std::env::temp_dir().join(format!("trex/aggregate-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Dir(dir)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> &Dir {
        std::fs::write(self.0.join(name), bytes).expect("write input");
        self
    }

    fn trex(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_trex")).args(args).current_dir(&self.0).output().expect("run trex")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove temp dir");
    }
}

/// Four requests over three hosts, one of them twice.
const LOG: &str = concat!(
    "GET http://a.example/x 200, GET http://b.test/y 404, ",
    "POST http://a.example/z 200, GET http://a.example/w 500"
);

#[test]
fn the_rows_count_a_captures_typed_slice() {
    let out = trex(&["top", "\\U:u", "${u:host}", "--text", LOG]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "a.example  3\nb.test     1\n");
    // count-by orders by key instead, over the same counts.
    let out = trex(&["count-by", "\\U:u", "${u:host}", "--text", LOG]);
    assert_eq!(stdout(&out), "a.example  3\nb.test     1\n");
    // uniq is the keys alone.
    let out = trex(&["uniq", "\\U:u", "${u:host}", "--text", LOG]);
    assert_eq!(stdout(&out), "a.example\nb.test\n");
}

#[test]
fn an_accessor_chain_groups_by_the_slice_it_names() {
    let text = "from 10.1.2.3 and 10.1.9.9 and 192.168.0.1";
    let out = trex(&["top", "\\I:ip", "${ip:octet1-2}", "--text", text]);
    assert_eq!(stdout(&out), "10.1     2\n192.168  1\n");
    // A composite key is a template like any other.
    let out = trex(&["count-by", "\\W:m \\S \\U:u", "${m:upper}/${u:host}", "--text", LOG]);
    assert_eq!(stdout(&out), "GET/a.example   2\nGET/b.test      1\nPOST/a.example  1\n");
}

#[test]
fn every_key_is_printed_and_a_cut_is_the_callers_own() {
    let whole = trex(&["top", "\\U:u", "${u:host}", "--text", LOG]);
    assert_eq!(stdout(&whole).lines().count(), 2, "nothing cuts the table by itself");
    let cut = trex(&["top", "\\U:u", "${u:host}", "--text", LOG, "-n", "1"]);
    // The cut says what it left out, so a short table never reads as a
    // complete one.
    assert_eq!(stdout(&cut), "a.example  3\n2 keys over 4 matches; 1 shown\n");
}

#[test]
fn a_key_the_accessor_leaves_empty_is_a_row_of_its_own() {
    // No URL here carries a port, so every match counts under the empty key
    // and the total still adds up to the matches.
    let out = trex(&["count-by", "\\U:u", "${u:port}", "--text", LOG]);
    assert_eq!(stdout(&out), "-    4\n");
}

#[test]
fn json_carries_the_same_rows() {
    let out = trex(&["top", "\\U:u", "${u:host}", "--text", LOG, "--json"]);
    assert_eq!(
        stdout(&out).trim(),
        r#"[{"key":"a.example","count":3},{"key":"b.test","count":1}]"#
    );
    let out = trex(&["uniq", "\\U:u", "${u:host}", "--text", LOG, "--json"]);
    assert_eq!(stdout(&out).trim(), r#"[{"key":"a.example"},{"key":"b.test"}]"#);
}

#[test]
fn a_binary_file_is_passed_over_as_a_scan_passes_it_unless_binary_asks() {
    let dir = Dir::new("binary");
    dir.write("a.log", b"x 1\nx 2\n").write("d.bin", b"x 3\0\nx 4\n");
    // Named alone, it is refused aloud with the notice a scan gives.
    let out = dir.trex(&["count-by", "\\W:k \\N", "${k}", "d.bin"]);
    assert!(!out.status.success(), "{}", stdout(&out));
    assert!(stderr(&out).contains("d.bin holds a NUL byte and is binary; --binary scans it"), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    // Among several, or found by a walk, it is passed over in silence.
    for args in [&["a.log", "d.bin"][..], &["."][..]] {
        let mut argv = vec!["count-by", "\\W:k \\N", "${k}"];
        argv.extend_from_slice(args);
        let out = dir.trex(&argv);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        assert_eq!(stdout(&out), "x    2\n", "{args:?}");
        assert_eq!(stderr(&out), "", "{args:?}");
    }
    // --binary counts its matches with the rest.
    let out = dir.trex(&["top", "\\W:k \\N", "${k}", "a.log", "d.bin", "--binary"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "x    4\n");
}

#[test]
fn a_malformed_pattern_or_key_is_refused_with_its_position() {
    let out = trex(&["top", "\\U:u", "${nosuch}", "--text", LOG]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("key error"), "{err}");
    let out = trex(&["top", "\\U:(", "${u:host}", "--text", LOG]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("pattern error"));
    // The usage line names both positional arguments.
    let out = trex(&["top"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("PATTERN KEY"));
}
