//! Record-level boolean queries: `--all`, `--any`, `--none`, `--at-least N`
//! and `--not` over several patterns, asked of each record an input is cut
//! into, a line by default or what `--record`, `--record-start` and
//! `--record-span` define.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A directory of inputs under the temp directory, removed on drop, that
/// commands run inside so their reports name files by their bare names.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/records-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Dir(dir)
    }

    fn write(&self, name: &str, text: &str) -> &Dir {
        std::fs::write(self.0.join(name), text).expect("write input");
        self
    }

    fn trex(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .current_dir(&self.0)
            .env_remove("NO_COLOR")
            .output()
            .expect("run trex")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove temp dir");
    }
}

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).env_remove("NO_COLOR").output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const LINES: &str = "from 10.0.0.1 bob@x.com\nfrom 10.0.0.2\nmail amy@y.org\nnothing here\n";

#[test]
fn the_rules_over_lines() {
    let out = trex(&["scan", "--all", "-e", "\\I", "-e", "\\E", "--text", LINES]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "from 10.0.0.1 bob@x.com\n");
    let out = trex(&["scan", "--any", "-e", "\\I", "-e", "\\E", "--text", LINES]);
    assert_eq!(stdout(&out), "from 10.0.0.1 bob@x.com\nfrom 10.0.0.2\nmail amy@y.org\n");
    let out = trex(&["scan", "--none", "-e", "\\I", "-e", "\\E", "--text", LINES]);
    assert_eq!(stdout(&out), "nothing here\n");
    let out = trex(&["scan", "--at-least", "2", "-e", "\\I", "-e", "\\E", "-e", "\"from\"", "--text", LINES]);
    assert_eq!(stdout(&out), "from 10.0.0.1 bob@x.com\nfrom 10.0.0.2\n");
    let out = trex(&["scan", "--any", "-e", "\\I", "--not", "\\E", "--text", LINES]);
    assert_eq!(stdout(&out), "from 10.0.0.2\n");
    // A positional pattern with a record definition and no rule is `--any`,
    // and records that do not touch are set apart by `--`.
    let out = trex(&["scan", "\\E", "--record", "line", "--text", LINES]);
    assert_eq!(stdout(&out), "from 10.0.0.1 bob@x.com\n--\nmail amy@y.org\n");
    let out = trex(&["scan", "--all", "-e", "\\I", "-e", "\\E", "--count", "--text", LINES]);
    assert_eq!(stdout(&out), "1\n");
    let out = trex(&["scan", "--all", "-e", "\\I", "-e", "\\E", "-m", "1", "--text", "a 1.1.1.1 a@a.io\nb 2.2.2.2 b@b.io\n"]);
    assert_eq!(stdout(&out), "a 1.1.1.1 a@a.io\n");
    let out = trex(&["scan", "--all", "-e", "\\U", "--text", LINES]);
    assert_eq!(stdout(&out), "no match\n");
}

#[test]
fn a_paragraph_a_file_and_a_pattern_define_the_record() {
    let text = "from 10.0.0.1\nto bob@x.com\n\nfrom 10.0.0.2\nnothing\n\nmail amy@y.org\n";
    let out = trex(&["scan", "--all", "-e", "\\I", "-e", "\\E", "--record", "paragraph", "--text", text]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "from 10.0.0.1\nto bob@x.com\n");
    let out = trex(&["scan", "--any", "-e", "\\E", "--record", "paragraph", "--text", text]);
    assert_eq!(stdout(&out), "from 10.0.0.1\nto bob@x.com\n--\nmail amy@y.org\n");
    let dir = Dir::new("units");
    dir.write("a.log", "2026-09-15T10:00:00Z start\n  detail bob@x.com\n2026-09-15T10:01:00Z other\n  detail 10.0.0.1\n")
        .write("b.log", "only 10.0.0.9\n");
    let out = dir.trex(&["scan", "--all", "-e", "\\E", "--record-start", "^ \\T", "a.log"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "2026-09-15T10:00:00Z start\n  detail bob@x.com\n");
    let out = dir.trex(&["scan", "--all", "-e", "\\I", "-e", "\\E", "--record", "file", "-l", "a.log", "b.log"]);
    assert_eq!(stdout(&out), "a.log\n");
    let out = dir.trex(&["scan", "--all", "-e", "\\I", "-e", "\\E", "--record", "file", "-L", "a.log", "b.log"]);
    assert_eq!(stdout(&out), "b.log\n");
    let out = trex(&["scan", "--any", "-e", "\\E", "--record-span", "\\B(.*)", "--text", "a (x 1) b (y bob@x.com) c\n"]);
    assert_eq!(stdout(&out), "a (x 1) b (y bob@x.com) c\n");
    let out = trex(&["scan", "--any", "-e", "\\N{>=3}", "--record", "unit:assign", "--text", "x = 1\nfoo(3)\ny = 3\n"]);
    assert_eq!(stdout(&out), "y = 3\n");
    let out = trex(&["scan", "--any", "-e", "\\N{>=3}", "--record", "period", "--text", "a 1\nb 2\nc 3\nd 4\n"]);
    assert_eq!(stdout(&out), "c 3\nd 4\n");
    for unit in ["seam", "texture", "shape", "unit"] {
        let out = trex(&["scan", "--any", "-e", "\\N", "--record", unit, "--count", "--text", "alpha 1 beta\n2 3 4\n"]);
        assert!(out.status.success(), "{unit}: {}", stderr(&out));
        assert!(stdout(&out).trim().parse::<usize>().is_ok(), "{unit}: {}", stdout(&out));
    }
}

#[test]
fn a_tree_report_names_the_path_and_json_names_the_patterns() {
    let dir = Dir::new("tree");
    dir.write("a.txt", "x 10.0.0.1 bob@x.com\nno\n").write("b.txt", "amy@y.org and 10.0.0.2\nplain\n");
    let out = dir.trex(&["scan", "--all", "-e", "\\I", "-e", "\\E", "a.txt", "b.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "a.txt:1:x 10.0.0.1 bob@x.com\nb.txt:1:amy@y.org and 10.0.0.2\n");
    let out = dir.trex(&["scan", "--all", "-e", "\\I", "-e", "\\E", "--count", "a.txt", "b.txt"]);
    assert_eq!(stdout(&out), "a.txt:1\nb.txt:1\n");
    let out = dir.trex(&["scan", "--any", "-e", "\\I", "-e", "\\E", "--json", "a.txt"]);
    assert_eq!(
        stdout(&out),
        "[{\"line\":1,\"start\":0,\"end\":20,\"text\":\"x 10.0.0.1 bob@x.com\",\"patterns\":[0,1]}]\n"
    );
    let out = dir.trex(&["scan", "--any", "-e", "\\E", "--json", "a.txt", "b.txt"]);
    assert_eq!(
        stdout(&out),
        "[{\"path\":\"a.txt\",\"line\":1,\"start\":0,\"end\":20,\"text\":\"x 10.0.0.1 bob@x.com\",\"patterns\":[0]},{\"path\":\"b.txt\",\"line\":1,\"start\":0,\"end\":22,\"text\":\"amy@y.org and 10.0.0.2\",\"patterns\":[0]}]\n"
    );
    let out = dir.trex(&["scan", "--all", "-e", "\\I", "-e", "\\E", "--color", "16", "a.txt", "b.txt"]);
    assert_eq!(
        stdout(&out),
        "\x1b[35ma.txt\x1b[0m\x1b[36m:\x1b[0m\x1b[32m1\x1b[0m\x1b[36m:\x1b[0mx \x1b[1;31m10.0.0.1\x1b[0m \x1b[1;31mbob@x.com\x1b[0m\n\
         \x1b[35mb.txt\x1b[0m\x1b[36m:\x1b[0m\x1b[32m1\x1b[0m\x1b[36m:\x1b[0m\x1b[1;31mamy@y.org\x1b[0m and \x1b[1;31m10.0.0.2\x1b[0m\n"
    );
}

#[test]
fn a_query_refuses_what_it_cannot_combine_with() {
    let out = trex(&["scan", "--all", "--any", "-e", "\\I", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--all and --any are two rules"), "{}", stderr(&out));
    let out = trex(&["scan", "--all", "-e", "\\I", "--record", "sentence", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("is not a record unit"), "{}", stderr(&out));
    let out = trex(&["scan", "--all", "-e", "\\I", "--record", "line", "--record-span", "\\N", "--text", "x"]);
    assert!(stderr(&out).contains("a second record definition"), "{}", stderr(&out));
    let out = trex(&["scan", "--all", "-e", "\\I", "--format", "${0}", "--text", "x"]);
    assert!(stderr(&out).contains("a record query prints the records that qualify"), "{}", stderr(&out));
    let out = trex(&["scan", "--at-least", "-e", "\\I", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--at-least needs a number"), "{}", stderr(&out));
    let out = trex(&["scan", "--all", "-e", "\\I", "--not", "\\Q{", "--text", "x"]);
    assert!(stderr(&out).contains("--not: pattern error"), "{}", stderr(&out));
}
