//! The grep flags `scan` takes as ripgrep and ugrep spell them: `-v`, `-x`,
//! `-o`, `-m`, `-e` and `-f`, `-g` and `-t`, `-L`, `--files`, `--sort`,
//! `--stats`, `--passthru` and `--color`.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A directory of inputs under the temp directory, removed on drop, that
/// commands run inside so their reports name files by their bare names.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/grep-{tag}-{}", std::process::id()));
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

/// The three inputs most tests read.
fn inputs(tag: &str) -> Dir {
    let dir = Dir::new(tag);
    dir.write("a.log", "from 10.0.0.1\nno ip here\n  10.0.0.2  \n")
        .write("b.txt", "alpha 1\nbeta\n")
        .write("c.rs", "fn main() {}\n");
    dir
}

#[test]
fn invert_prints_the_lines_no_match_touches() {
    let out = trex(&["scan", "-v", "\\N", "--text", "a 1\nb\nc 2\n"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "b\n");
    let out = trex(&["scan", "-v", "--count", "\\N", "--text", "a 1\nb\nc\n"]);
    assert_eq!(stdout(&out), "2\n");
    let out = trex(&["scan", "-v", "-m", "1", "\\N", "--text", "a 1\nb\nc\n"]);
    assert_eq!(stdout(&out), "b\n");
    let out = trex(&["scan", "-v", "-m", "1", "--count", "\\N", "--text", "a 1\nb\nc\n"]);
    assert_eq!(stdout(&out), "1\n");
    let dir = inputs("invert");
    let out = dir.trex(&["scan", "-v", "\\I", "a.log", "b.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "a.log:2:no ip here\nb.txt:1:alpha 1\nb.txt:2:beta\n");
    // Over several inputs as over one, `-vc` counts the lines no match
    // touches and `-vl` names the inputs holding one.
    let out = dir.trex(&["scan", "-v", "--count", "\\I", "a.log", "b.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "a.log:1\nb.txt:2\n");
    let out = dir.trex(&["scan", "-v", "--count-matches", "\\I", "a.log", "b.txt"]);
    assert_eq!(stdout(&out), "a.log:1\nb.txt:2\n");
    let out = dir.trex(&["scan", "-v", "-l", "\\I", "a.log", "b.txt"]);
    assert_eq!(stdout(&out), "a.log\nb.txt\n");
    let out = trex(&["scan", "-v", "--json", "\\N", "--text", "1"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("-v prints the lines no match touches"), "{}", stderr(&out));
}

#[test]
fn whole_line_keeps_a_match_covering_the_significant_extent() {
    let out = trex(&["scan", "-x", "\\I", "--text", "10.0.0.1\n  10.0.0.2  \nfrom 10.0.0.3\n"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "[0..8] \"10.0.0.1\"\n[11..19] \"10.0.0.2\"\n");
    let out = trex(&["scan", "-x", "-v", "\\I", "--text", "10.0.0.1\nfrom 10.0.0.3\n"]);
    assert_eq!(stdout(&out), "from 10.0.0.3\n");
}

#[test]
fn max_count_and_only_matching() {
    let out = trex(&["scan", "-m", "2", "\\N", "--text", "1 2 3 4"]);
    assert_eq!(stdout(&out), "[0..1] \"1\"\n[2..3] \"2\"\n");
    let out = trex(&["scan", "--max-count=1", "\\N", "--text", "1 2 3 4"]);
    assert_eq!(stdout(&out), "[0..1] \"1\"\n");
    let out = trex(&["scan", "-o", "\\N", "--text", "1"]);
    assert_eq!(stdout(&out), "[0..1] \"1\"\n");
    let dir = inputs("max");
    let out = dir.trex(&["scan", "-m", "1", "--count", "\\I", "a.log", "b.txt"]);
    assert_eq!(stdout(&out), "a.log:1\n");
}

#[test]
fn several_patterns_join_as_an_alternation() {
    let out = trex(&["scan", "-e", "\\I", "-e", "\\E", "--text", "x 10.0.0.1 bob@x.com"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "[2..10] \"10.0.0.1\"\n[11..20] \"bob@x.com\"\n");
    let dir = Dir::new("patterns");
    dir.write("pats.txt", "\\I\n\n\\E\n").write("empty.txt", "\n");
    let out = dir.trex(&["scan", "-f", "pats.txt", "--text", "x 10.0.0.1 bob@x.com"]);
    assert_eq!(stdout(&out), "[2..10] \"10.0.0.1\"\n[11..20] \"bob@x.com\"\n");
    let out = dir.trex(&["scan", "-f", "empty.txt", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("hold no pattern"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "-f", "missing.txt", "--text", "x"]);
    assert!(stderr(&out).contains("cannot read the pattern file"), "{}", stderr(&out));
    let out = trex(&["scan", "--text", "x"]);
    assert!(stderr(&out).contains("a pattern is needed"), "{}", stderr(&out));
}

#[test]
fn files_with_and_without_a_match_and_the_files_alone() {
    let dir = inputs("files");
    let out = dir.trex(&["scan", "-L", "\\I", "a.log", "b.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "b.txt\n");
    let out = dir.trex(&["scan", "-l", "\\I", "a.log", "b.txt"]);
    assert_eq!(stdout(&out), "a.log\n");
    let out = dir.trex(&["scan", "-L", "-l", "\\I", "a.log"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("-L names the inputs with no match"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "-L", "--count-matches", "\\I", "a.log", "b.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("-L names the inputs with no match"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--files"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let listed = stdout(&out);
    let names: Vec<&str> = listed.lines().map(|l| l.rsplit(['/', '\\']).next().unwrap_or(l)).collect();
    assert_eq!(names, ["a.log", "b.txt", "c.rs"]);
    let out = dir.trex(&["scan", "--files", "-g", "*.rs", "."]);
    assert_eq!(stdout(&out).lines().count(), 1);
    assert!(stdout(&out).contains("c.rs"));
    let out = dir.trex(&["scan", "--files", "--glob=!*.rs", "."]);
    assert!(stdout(&out).contains("a.log") && stdout(&out).contains("b.txt") && !stdout(&out).contains("c.rs"));
    let out = dir.trex(&["scan", "--files", "-t", "rust", "."]);
    assert_eq!(stdout(&out).lines().count(), 1);
    let out = dir.trex(&["scan", "--files", "-T", "rust", "."]);
    assert_eq!(stdout(&out).lines().count(), 2);
    let out = dir.trex(&["scan", "--files", "-t", "bogus", "."]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unrecognized file type"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--files", "-g", "[", "."]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("glob"), "{}", stderr(&out));
    let out = trex(&["scan", "--type-list"]);
    assert!(out.status.success());
    assert!(stdout(&out).lines().any(|l| l.starts_with("rust: ") && l.contains("*.rs")), "{}", stdout(&out));
}

#[test]
fn sort_orders_the_files_and_sortr_reverses() {
    let dir = inputs("sort");
    let out = dir.trex(&["scan", "--files", "--sort", "path", "."]);
    let names: Vec<String> =
        stdout(&out).lines().map(|l| l.rsplit(['/', '\\']).next().unwrap_or(l).to_string()).collect();
    assert_eq!(names, ["a.log", "b.txt", "c.rs"]);
    let out = dir.trex(&["scan", "--files", "--sortr", "path", "."]);
    let names: Vec<String> =
        stdout(&out).lines().map(|l| l.rsplit(['/', '\\']).next().unwrap_or(l).to_string()).collect();
    assert_eq!(names, ["c.rs", "b.txt", "a.log"]);
    let out = dir.trex(&["scan", "--files", "--sort", "modified", "."]);
    assert_eq!(stdout(&out).lines().count(), 3);
    let out = dir.trex(&["scan", "--files", "--sort", "size", "."]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("takes path, modified, accessed or created"), "{}", stderr(&out));
}

#[test]
fn files_lists_what_a_scan_reads_leaving_out_binary_files() {
    let dir = inputs("binary");
    std::fs::write(dir.0.join("d.bin"), b"ab\0cd\n").expect("write input");
    let names = |out: &Output| -> Vec<String> {
        stdout(out).lines().map(|l| l.rsplit(['/', '\\']).next().unwrap_or(l).to_string()).collect()
    };
    let out = dir.trex(&["scan", "--files", "."]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(names(&out), ["a.log", "b.txt", "c.rs"]);
    let out = dir.trex(&["scan", "--files", "--binary", "."]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(names(&out), ["a.log", "b.txt", "c.rs", "d.bin"]);
    // One file named alone is refused aloud, as a scan of it is; among
    // several it is left out without a word, as a scan leaves it.
    let out = dir.trex(&["scan", "--files", "d.bin"]);
    assert!(!out.status.success());
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("d.bin holds a NUL byte and is binary; --binary scans it"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--files", "d.bin", "a.log"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(names(&out), ["a.log"]);
}

#[test]
fn stats_open_with_ripgrep_s_eight_lines_or_come_as_one() {
    let out = trex(&["scan", "--stats", "\\N", "--text", "1 2"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(&lines[..3], ["[0..1] \"1\"", "[2..3] \"2\"", ""]);
    assert_eq!(
        &lines[3..9],
        [
            "2 matches",
            "1 matched lines",
            "1 files contained matches",
            "1 files searched",
            "22 bytes printed",
            "3 bytes searched"
        ]
    );
    assert!(lines[9].ends_with(" seconds spent searching"), "{}", lines[9]);
    // Those eight open the block and keep their order, which is what a reader
    // coming from ripgrep and anything parsing the output both depend on. What
    // follows them says why the scan cost what it did and is asserted in
    // tests/route_stats.rs; this is only that the eight still come first.
    assert!(lines[10].ends_with(" seconds"), "{text}");
    assert!(lines.len() > 11, "the block stops at the eight lines: {text}");
    let out = trex(&["scan", "--stats=line", "\\N", "--text", "1 2"]);
    let text = stdout(&out);
    let last = text.lines().last().expect("a stats line");
    assert!(last.starts_with("2 matches in 1 of 1 files, 3 bytes searched, "), "{last}");
    assert!(last.ends_with(" ms"), "{last}");
    let dir = inputs("stats");
    let out = dir.trex(&["scan", "--stats=line", "\\I", "a.log", "b.txt"]);
    assert!(stdout(&out).contains("2 matches in 1 of 2 files, 51 bytes searched, "), "{}", stdout(&out));
    let out = trex(&["scan", "--stats", "--json", "\\N", "--text", "1"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--stats prints after the report"), "{}", stderr(&out));
    let out = trex(&["scan", "--stats=all", "\\N", "--text", "1"]);
    assert!(stderr(&out).contains("--stats takes no value but line"), "{}", stderr(&out));
}

#[test]
fn passthru_prints_every_line_with_the_matches_painted() {
    let out = trex(&["scan", "--passthru", "--color", "never", "\\N", "--text", "a 1\nb\nc 2\n"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "a 1\nb\nc 2\n");
    let out = trex(&["scan", "--passthru", "--color", "16", "\\N", "--text", "a 1\nb\nc 2\n"]);
    assert_eq!(stdout(&out), "a \x1b[1;31m1\x1b[0m\nb\nc \x1b[1;31m2\x1b[0m\n");
    let dir = inputs("passthru");
    let out = dir.trex(&["scan", "--passthru", "--color", "never", "-H", "\\I", "a.log"]);
    assert_eq!(stdout(&out), "a.log:1:from 10.0.0.1\na.log-2-no ip here\na.log:3:  10.0.0.2  \n");
    let out = dir.trex(&["scan", "--passthru", "--count", "\\I", "a.log"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--passthru prints every line"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--passthru", "--count-matches", "\\I", "a.log", "b.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--passthru prints every line"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--format", "${ip}", "--count-matches", "\\I:ip", "a.log", "b.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--format prints one line per match"), "{}", stderr(&out));
}

#[test]
fn color_paints_the_roles_at_the_depth_asked_for() {
    let dir = inputs("color");
    let out = dir.trex(&["scan", "--color", "16", "\\I", "a.log", "b.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "\x1b[35ma.log\x1b[0m\x1b[36m:\x1b[0m\x1b[32m1\x1b[0m\x1b[36m:\x1b[0m\x1b[32m6\x1b[0m\x1b[36m:\x1b[0m \x1b[1;31m\"10.0.0.1\"\x1b[0m\n\
         \x1b[35ma.log\x1b[0m\x1b[36m:\x1b[0m\x1b[32m3\x1b[0m\x1b[36m:\x1b[0m\x1b[32m3\x1b[0m\x1b[36m:\x1b[0m \x1b[1;31m\"10.0.0.2\"\x1b[0m\n"
    );
    // Nothing is painted where the output is not a terminal, or under never.
    let out = dir.trex(&["scan", "\\I", "a.log", "b.txt"]);
    assert_eq!(stdout(&out), "a.log:1:6: \"10.0.0.1\"\na.log:3:3: \"10.0.0.2\"\n");
    let out = dir.trex(&["scan", "--color", "never", "\\I", "a.log", "b.txt"]);
    assert_eq!(stdout(&out), "a.log:1:6: \"10.0.0.1\"\na.log:3:3: \"10.0.0.2\"\n");
    // An override in ripgrep's spelling, sent at the depth asked for.
    let out = trex(&[
        "scan",
        "--color",
        "truecolor",
        "--colors",
        "match:fg:#ff8700",
        "--colors",
        "match:style:nobold",
        "\\I",
        "--text",
        "from 10.0.0.1",
    ]);
    assert_eq!(stdout(&out), "[5..13] \x1b[38;2;255;135;0m\"10.0.0.1\"\x1b[0m\n");
    let out = trex(&["scan", "--color=256", "--colors=match:fg:#ff8700", "\\I", "--text", "from 10.0.0.1"]);
    assert_eq!(stdout(&out), "[5..13] \x1b[1;38;5;208m\"10.0.0.1\"\x1b[0m\n");
    let out = trex(&["scan", "--color", "sometimes", "\\I", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--color takes always, never, auto"), "{}", stderr(&out));
    let out = trex(&["scan", "--colors", "match:weight:bold", "\\I", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--colors:"), "{}", stderr(&out));
    // The count, the file list and the context lines paint the path and the numbers too.
    let out = dir.trex(&["scan", "--color", "16", "--count", "\\I", "a.log", "b.txt"]);
    assert_eq!(stdout(&out), "\x1b[35ma.log\x1b[0m\x1b[36m:\x1b[0m2\n");
    let out = dir.trex(&["scan", "--color", "16", "-l", "\\I", "a.log", "b.txt"]);
    assert_eq!(stdout(&out), "\x1b[35ma.log\x1b[0m\n");
    let out = dir.trex(&["scan", "--color", "16", "-A", "1", "\\N", "b.txt"]);
    assert_eq!(
        stdout(&out),
        "\x1b[32m1\x1b[0m\x1b[36m:\x1b[0m\x1b[32m7\x1b[0m\x1b[36m:\x1b[0m \x1b[1;31m\"1\"\x1b[0m\n\x1b[32m2\x1b[0m\x1b[36m-\x1b[0mbeta\n"
    );
}
