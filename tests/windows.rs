//! A head, a tail and a range of an input: `trex head`, `trex tail` and
//! `trex lines` listing them, `--head`, `--tail` and `--lines` restricting a
//! scan, a rewrite, a redaction, the rules and a table to them with the
//! input's own offsets and line numbers, and `-f` / `--follow` reading on as
//! a file grows, through truncation and rotation.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

/// How long a test waits for a line that must come, or for quiet where none
/// may; a follower wakes on the operating system's notification or within a
/// second without one.
const WAIT: Duration = Duration::from_secs(30);
const QUIET: Duration = Duration::from_secs(3);

/// A directory of inputs under the temp directory, removed on drop, that
/// commands run inside so their reports name files by their bare names.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/windows-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Dir(dir)
    }

    fn write(&self, name: &str, text: impl AsRef<[u8]>) -> &Dir {
        std::fs::write(self.0.join(name), text).expect("write input");
        self
    }

    fn append(&self, name: &str, text: &str) {
        let mut f = std::fs::OpenOptions::new().append(true).open(self.0.join(name)).expect("open to append");
        f.write_all(text.as_bytes()).expect("append");
    }

    fn read(&self, name: &str) -> String {
        String::from_utf8_lossy(&self.bytes(name)).into_owned()
    }

    fn bytes(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.0.join(name)).expect("read the file")
    }

    fn trex(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .current_dir(&self.0)
            .env_remove("NO_COLOR")
            .output()
            .expect("run trex")
    }

    /// Run with `input` written to the standard input.
    fn trex_stdin(&self, args: &[&str], input: impl AsRef<[u8]>) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .current_dir(&self.0)
            .env_remove("NO_COLOR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn trex");
        let mut stdin = child.stdin.take().expect("a piped stdin");
        stdin.write_all(input.as_ref()).expect("write the stream");
        drop(stdin);
        child.wait_with_output().expect("wait for trex")
    }

    /// A command that follows files, its output lines delivered as printed.
    fn follow(&self, args: &[&str]) -> Following {
        let mut child = Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .current_dir(&self.0)
            .env_remove("NO_COLOR")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn trex");
        let out = lines_of(child.stdout.take().expect("a piped stdout"));
        let err = lines_of(child.stderr.take().expect("a piped stderr"));
        Following { child, out, err }
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove temp dir");
    }
}

/// The lines of a child's output, delivered as it prints them.
fn lines_of(stream: impl std::io::Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
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
    rx
}

/// A running follow, killed when the test is done with it.
struct Following {
    child: Child,
    out: Receiver<String>,
    err: Receiver<String>,
}

impl Following {
    fn line(&self) -> String {
        self.out.recv_timeout(WAIT).expect("a line the follow must print")
    }

    fn note(&self) -> String {
        self.err.recv_timeout(WAIT).expect("a note the follow must write")
    }

    fn quiet(&self) {
        match self.out.recv_timeout(QUIET) {
            Ok(line) => panic!("printed {line:?} before a line of it had ended"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("the follow's output ended"),
        }
    }
}

impl Drop for Following {
    fn drop(&mut self) {
        if let Err(e) = self.child.kill() {
            eprintln!("the follow had already ended: {e}");
        }
        if let Err(e) = self.child.wait() {
            eprintln!("cannot wait for the follow: {e}");
        }
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

/// Five lines, each a word and a number; the line starts are 0, 6, 12, 20
/// and 27, and the input is 34 bytes long.
const FIVE: &str = "one 1\ntwo 2\nthree 3\nfour 4\nfive 5\n";

#[test]
fn head_tail_and_lines_print_what_they_select() {
    let dir = Dir::new("list");
    dir.write("f.txt", FIVE);
    assert_eq!(stdout(&dir.trex(&["head", "2", "f.txt"])), "one 1\ntwo 2\n");
    assert_eq!(stdout(&dir.trex(&["tail", "2", "f.txt"])), "four 4\nfive 5\n");
    assert_eq!(stdout(&dir.trex(&["tail", "-2", "f.txt"])), "four 4\nfive 5\n");
    assert_eq!(stdout(&dir.trex(&["lines", "2..3", "f.txt"])), "two 2\nthree 3\n");
    assert_eq!(stdout(&dir.trex(&["lines", "4..", "f.txt"])), "four 4\nfive 5\n");
    assert_eq!(stdout(&dir.trex(&["head", "--lines", "..2", "f.txt"])), "one 1\ntwo 2\n");
    assert_eq!(stdout(&dir.trex(&["tail", "9", "f.txt"])), FIVE);
    // Numbered, a tail's lines carry the file's own numbers.
    let out = dir.trex(&["tail", "2", "-n", "f.txt", "--color", "never"]);
    assert_eq!(stdout(&out), "4:four 4\n5:five 5\n");
    let out = dir.trex(&["lines", "2..3", "-n", "f.txt", "--color", "never"]);
    assert_eq!(stdout(&out), "2:two 2\n3:three 3\n");
    // A last line with no newline is printed as the file holds it.
    dir.write("g.txt", "x\ny");
    assert_eq!(stdout(&dir.trex(&["tail", "1", "g.txt"])), "y");
}

#[test]
fn several_inputs_are_headed_and_the_standard_input_is_read() {
    let dir = Dir::new("heads");
    dir.write("a.txt", "a1\na2\n").write("b.txt", "b1\nb2\n");
    let out = dir.trex(&["head", "1", "a.txt", "b.txt", "--color", "never"]);
    assert_eq!(stdout(&out), "==> a.txt <==\na1\n\n==> b.txt <==\nb1\n");
    let out = dir.trex_stdin(&["tail", "2"], "s1\ns2\ns3\ns4\n");
    assert_eq!(stdout(&out), "s3\ns4\n");
    let out = dir.trex_stdin(&["head", "1", "-n", "--color", "never"], "s1\ns2\n");
    assert_eq!(stdout(&out), "1:s1\n");
}

#[test]
fn a_record_unit_selects_whole_records() {
    let dir = Dir::new("records");
    dir.write("p.txt", "a one\na two\n\nb one\n\nc one\nc two\n");
    assert_eq!(stdout(&dir.trex(&["head", "1", "p.txt", "--record", "paragraph"])), "a one\na two\n");
    assert_eq!(stdout(&dir.trex(&["tail", "1", "p.txt", "--record", "paragraph"])), "c one\nc two\n");
    let out = dir.trex(&["tail", "1", "-n", "p.txt", "--record", "paragraph", "--color", "never"]);
    assert_eq!(stdout(&out), "6:c one\n7:c two\n");
}

#[test]
fn a_restricted_scan_reports_the_input_s_own_offsets_and_lines() {
    let dir = Dir::new("scan");
    dir.write("f.txt", FIVE);
    let out = dir.trex(&["scan", "\\N", "f.txt", "--tail", "2", "--color", "never"]);
    assert_eq!(stdout(&out), "[25..26] \"4\"\n[32..33] \"5\"\n");
    let out = dir.trex(&["scan", "\\N", "f.txt", "--lines", "2..3", "-H", "--color", "never"]);
    assert_eq!(stdout(&out), "f.txt:2:5: \"2\"\nf.txt:3:7: \"3\"\n");
    let out = dir.trex(&["scan", "\\N", "f.txt", "--tail", "1", "-H", "--color", "never"]);
    assert_eq!(stdout(&out), "f.txt:5:6: \"5\"\n");
    let out = dir.trex(&["scan", "\\N", "f.txt", "--head", "3", "--count"]);
    assert_eq!(stdout(&out), "3\n");
    let out = dir.trex(&["scan", "\\N", "f.txt", "--tail", "1", "-H", "--json"]);
    let json = stdout(&out);
    assert!(json.contains("\"line\":5") && json.contains("\"start\":32"), "{json}");
    let out = dir.trex(&["scan", "\\N", "f.txt", "--tail", "1", "--format", "${line}:${start} ${0}"]);
    assert_eq!(stdout(&out), "5:32 5\n");
    let out = dir.trex(&["scan", "\\N", "--text", "a 1\nb 2\nc 3\n", "--tail", "1", "--color", "never"]);
    assert_eq!(stdout(&out), "[10..11] \"3\"\n");
    let out = dir.trex_stdin(&["scan", "\\N", "--head", "2", "--color", "never"], "a 1\nb 2\nc 3\n");
    assert_eq!(stdout(&out), "[2..3] \"1\"\n[6..7] \"2\"\n");
}

#[test]
fn a_match_counts_only_when_it_is_wholly_inside_the_window() {
    let dir = Dir::new("edges");
    dir.write("c.txt", "call(a,\nb)\nnext(c)\n");
    let whole = stdout(&dir.trex(&["scan", "\\W\\B(.*)", "c.txt", "--count-matches"]));
    assert_eq!(whole, "2\n");
    let out = dir.trex(&["scan", "\\W\\B(.*)", "c.txt", "--lines", "2..", "--color", "never"]);
    assert_eq!(stdout(&out), "[11..18] \"next(c)\"\n");
}

#[test]
fn a_restricted_rewrite_prints_its_window_and_edits_only_it_in_place() {
    let dir = Dir::new("rewrite");
    dir.write("f.txt", FIVE).write("g.txt", FIVE);
    let out = dir.trex(&["rewrite", "\\N:n", "<${n}>", "f.txt", "--tail", "1"]);
    assert_eq!(stdout(&out), "five <5>\n");
    let out = dir.trex(&["rewrite", "\\N:n", "<${n}>", "g.txt", "--in-place", "--lines", "2..2"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(dir.read("g.txt"), "one 1\ntwo <2>\nthree 3\nfour 4\nfive 5\n");
    let out = dir.trex(&["rewrite", "\\N:n", "<${n}>", "f.txt", "--dry-run", "--head", "1"]);
    let diff = stdout(&out);
    assert!(diff.contains("-one 1") && diff.contains("+one <1>"), "{diff}");
    assert!(!diff.contains("<2>") && !diff.contains("<5>"), "{diff}");
    // Printed, a redaction prints its window alone, so nothing outside it
    // reaches the output unredacted.
    let out = dir.trex(&["redact", "\\N", "f.txt", "--head", "1"]);
    assert_eq!(stdout(&out), "one *\n");
    let out = dir.trex(&["redact", "\\N", "g.txt", "--in-place", "--tail", "1"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(dir.read("g.txt"), "one 1\ntwo <2>\nthree 3\nfour 4\nfive *\n");
}

/// `text` behind a byte order mark: `mark`, then `text` in the encoding the
/// mark declares.
fn marked(mark: &[u8], text: &str) -> Vec<u8> {
    let body: Vec<u8> = match mark {
        [0xEF, 0xBB, 0xBF] => text.as_bytes().to_vec(),
        [0xFF, 0xFE] => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        [0xFE, 0xFF] => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        [0xFF, 0xFE, 0, 0] => text.chars().flat_map(|c| u32::from(c).to_le_bytes()).collect(),
        other => panic!("{other:02X?} is not a mark these tests write"),
    };
    [mark, body.as_slice()].concat()
}

/// The marks a marked input is written under: UTF-8, UTF-16 either way
/// round, and UTF-32.
const MARKS: [&[u8]; 4] = [&[0xEF, 0xBB, 0xBF], &[0xFF, 0xFE], &[0xFE, 0xFF], &[0xFF, 0xFE, 0, 0]];

#[test]
fn a_marked_file_is_read_at_the_places_its_text_holds_unmarked() {
    let dir = Dir::new("marked");
    dir.write("f.txt", FIVE);
    let names = ["u8.txt", "le16.txt", "be16.txt", "le32.txt"];
    for (name, mark) in names.into_iter().zip(MARKS) {
        dir.write(name, marked(mark, FIVE));
    }
    // `F` stands for the file each command reads.
    let commands: [&[&str]; 12] = [
        &["head", "2", "F"],
        &["tail", "2", "F"],
        &["lines", "2..3", "F"],
        &["tail", "2", "-n", "F", "--color", "never"],
        &["lines", "4..", "-n", "F", "--color", "never"],
        &["scan", "\\N", "F", "--tail", "2", "--color", "never"],
        &["scan", "\\N", "F", "--lines", "2..3", "--format", "${line}:${start}..${end} ${0}"],
        &["scan", "\\N", "F", "--tail", "1", "--format", "${line}:${start}..${end} ${0}"],
        &["scan", "\\N", "F", "--head", "3", "--count"],
        &["rewrite", "\\N:n", "<${n}>", "F", "--tail", "1"],
        &["rewrite", "\\N:n", "<${n}>", "F", "--dry-run", "--lines", "2..2"],
        &["redact", "\\N", "F", "--head", "1"],
    ];
    for command in commands {
        let run = |file: &str| {
            let args: Vec<&str> = command.iter().map(|a| if *a == "F" { file } else { *a }).collect();
            let out = dir.trex(&args);
            assert!(out.status.success(), "{args:?}: {}", stderr(&out));
            stdout(&out)
        };
        let want = run("f.txt");
        assert!(!want.is_empty(), "{command:?} prints nothing");
        for name in names {
            // A diff names its file, which is the one difference allowed.
            assert_eq!(run(name).replace(name, "f.txt"), want, "{command:?} on {name}");
        }
    }
    // A window edited in place is written back in the file's own encoding,
    // behind its own mark, every byte outside the window as it was.
    for (name, mark) in names.into_iter().zip(MARKS) {
        let out = dir.trex(&["rewrite", "\\N:n", "<${n}>", name, "--in-place", "--tail", "1"]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(dir.bytes(name), marked(mark, "one 1\ntwo 2\nthree 3\nfour 4\nfive <5>\n"), "{name}");
        let out = dir.trex(&["redact", "\\N", name, "--in-place", "--head", "1"]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(dir.bytes(name), marked(mark, "one *\ntwo 2\nthree 3\nfour 4\nfive <5>\n"), "{name}");
    }
    // Read from the standard input, a marked stream holds the same lines.
    for mark in MARKS {
        let stream = marked(mark, FIVE);
        assert_eq!(stdout(&dir.trex_stdin(&["head", "1"], &stream)), "one 1\n", "{mark:02X?}");
        let out = dir.trex_stdin(&["tail", "2", "-n", "--color", "never"], &stream);
        assert_eq!(stdout(&out), "4:four 4\n5:five 5\n", "{mark:02X?}");
        let out = dir.trex_stdin(&["scan", "\\N", "--tail", "1", "--color", "never"], &stream);
        assert_eq!(stdout(&out), "[32..33] \"5\"\n", "{mark:02X?}");
    }
}

/// A marked stream scanned with no window, which streams the text after a
/// UTF-8 mark and reads the stream whole behind a wider one, reports the
/// places the same text holds in a file, and a mark alone holds no token.
#[test]
fn a_marked_stream_is_scanned_at_the_places_its_text_holds() {
    let dir = Dir::new("marked-stream");
    dir.write("f.txt", FIVE);
    let format = "${line}:${col}:${start}..${end} ${0}";
    let want = stdout(&dir.trex(&["scan", "\\N", "f.txt", "--format", format]));
    assert_eq!(want.lines().count(), 5, "{want}");
    for mark in MARKS {
        let out = dir.trex_stdin(&["scan", "\\N", "--format", format], marked(mark, FIVE));
        assert!(out.status.success(), "{mark:02X?}: {}", stderr(&out));
        assert_eq!(stdout(&out), want, "{mark:02X?}");
        let out = dir.trex_stdin(&["scan", ".", "--format", "${start}..${end}"], mark);
        assert_eq!(stdout(&out), "", "{mark:02X?} alone");
    }
}

/// An empty file has no lines, so no form lists one and a count is 0, while
/// a file holding one newline has one blank line in every form.
#[test]
fn an_empty_file_has_no_lines_and_a_newline_has_one() {
    let dir = Dir::new("empty");
    dir.write("e.txt", "").write("n.txt", "\n");
    for (name, listed, count, numbered) in [("e.txt", "", "0\n", ""), ("n.txt", "\n", "1\n", "1:\n")] {
        let run = |args: &[&str]| stdout(&dir.trex(args));
        assert_eq!(run(&["scan", "\"x\"", name, "-v", "--color", "never"]), listed, "{name}");
        assert_eq!(run(&["scan", "\"x\"", name, "-v", "--count"]), count, "{name}");
        assert_eq!(run(&["scan", "\"x\"", name, "--passthru", "--color", "never"]), listed, "{name}");
        assert_eq!(run(&["head", "3", name]), listed, "{name}");
        assert_eq!(run(&["tail", "3", "-n", name, "--color", "never"]), numbered, "{name}");
    }
}

#[test]
fn rules_and_tables_read_the_window_at_the_input_s_own_lines() {
    let dir = Dir::new("rules");
    dir.write("rules.trex", "rule todo note \"a TODO left\" = \"TODO\"\n");
    dir.write("app.conf", "TODO one\nfine\nTODO two\n");
    let out = dir.trex(&["scan", "--rules", "rules.trex", "app.conf", "--tail", "1", "--color", "never"]);
    assert_eq!(stdout(&out), "app.conf:3:1: note: a TODO left [todo]\n");
    dir.write("f.txt", FIVE);
    let out = dir.trex(&["count-by", "\\W:w \\N", "${line}", "f.txt", "--lines", "2..3", "--json"]);
    assert_eq!(stdout(&out), "[{\"key\":\"2\",\"count\":1},{\"key\":\"3\",\"count\":1}]\n");
    let out = dir.trex(&["templates", "f.txt", "--head", "2", "--json"]);
    assert!(stdout(&out).contains("\"count\":2"), "{}", stdout(&out));
}

#[test]
fn what_cannot_be_followed_is_refused_with_the_reason() {
    let dir = Dir::new("refused");
    dir.write("f.txt", FIVE);
    let out = dir.trex(&["head", "3", "-f", "f.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("ends before the file does"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "\\N", "f.txt", "--head", "3", "--follow"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--follow reads on as a file grows"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "\\N", "f.txt", "--follow", "--count"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("a followed file never ends"), "{}", stderr(&out));
    let out = dir.trex_stdin(&["tail", "3", "-f"], "a\n");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("follows a file by name"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "\\N", ".", "--index", "--tail", "2"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--index summarizes whole files"), "{}", stderr(&out));
}

#[test]
fn tail_follows_appends_truncation_and_rotation() {
    let dir = Dir::new("tail-f");
    dir.write("app.log", "a 1\nb 2\n");
    let f = dir.follow(&["tail", "1", "-f", "app.log"]);
    assert_eq!(f.line(), "b 2");
    dir.append("app.log", "c 3\n");
    assert_eq!(f.line(), "c 3");
    dir.write("app.log", "d 4\n");
    assert!(f.note().contains("app.log: truncated"));
    assert_eq!(f.line(), "d 4");
    std::fs::rename(dir.0.join("app.log"), dir.0.join("app.log.1")).expect("rotate the log");
    dir.write("app.log", "e 5\n");
    let note = f.note();
    assert!(note.contains("app.log: replaced") || note.contains("app.log: removed"), "{note}");
    assert_eq!(f.line(), "e 5");
}

#[test]
fn a_numbered_tail_numbers_each_line_once_it_ends() {
    let dir = Dir::new("tail-n");
    dir.write("app.log", "x\ny\n");
    let f = dir.follow(&["tail", "1", "-n", "-f", "app.log", "--color", "never"]);
    assert_eq!(f.line(), "2:y");
    dir.append("app.log", "z\n");
    assert_eq!(f.line(), "3:z");
    dir.append("app.log", "w");
    f.quiet();
    dir.append("app.log", "\n");
    assert_eq!(f.line(), "4:w");
}

#[test]
fn a_followed_scan_prints_each_match_at_the_file_s_own_place() {
    let dir = Dir::new("scan-f");
    dir.write("app.log", "a 1\n");
    let f = dir.follow(&["scan", "\\N", "app.log", "--tail", "1", "--follow", "-H", "--color", "never"]);
    assert_eq!(f.line(), "app.log:1:3: \"1\"");
    dir.append("app.log", "b 22\n");
    assert_eq!(f.line(), "app.log:2:3: \"22\"");
    let g = dir.follow(&["scan", "\\N", "app.log", "--follow", "--json"]);
    assert_eq!(g.line(), "{\"start\":2,\"end\":3,\"text\":\"1\",\"captures\":{}}");
    assert_eq!(g.line(), "{\"start\":6,\"end\":8,\"text\":\"22\",\"captures\":{}}");
}

#[test]
fn a_truncated_file_starts_its_count_again_unless_keep_count_keeps_it() {
    let dir = Dir::new("keep-count");
    dir.write("a.log", "x 11\n").write("b.log", "none\n").write("r.trex", "rule num note \"n\" = \\N\n");
    // b.log, with no match yet, keeps each follow going once a.log has
    // given its one match.
    let f = dir.follow(&["scan", "\\N", "a.log", "b.log", "--follow", "-m", "1", "--color", "never"]);
    assert_eq!(f.line(), "a.log:1:3: \"11\"");
    dir.write("a.log", "y 2\n");
    assert!(f.note().contains("a.log: truncated"));
    // The file under the name is a new input, and may give its own match.
    assert_eq!(f.line(), "a.log:1:3: \"2\"");
    let r = dir.follow(&["scan", "--rules", "r.trex", "a.log", "b.log", "--follow", "-m", "1", "--color", "never"]);
    assert_eq!(r.line(), "a.log:1:3: note: n [num]");
    dir.write("a.log", "z\n");
    assert!(r.note().contains("a.log: truncated"));
    dir.append("a.log", "w 3\n");
    assert_eq!(r.line(), "a.log:2:3: note: n [num]");

    dir.write("a.log", "x 11\n");
    let g = dir.follow(&["scan", "\\N", "a.log", "b.log", "--follow", "-m", "1", "--keep-count", "--color", "never"]);
    assert_eq!(g.line(), "a.log:1:3: \"11\"");
    dir.write("a.log", "y 2\n");
    assert!(g.note().contains("a.log: truncated"));
    g.quiet();
    // b.log's one match is all the follow has left to give.
    dir.append("b.log", "z 3\n");
    assert_eq!(g.line(), "b.log:2:3: \"3\"");
    dir.write("b.log", "none\n");
    let k = dir.follow(&[
        "scan", "--rules", "r.trex", "a.log", "b.log", "--follow", "-m", "1", "--keep-count", "--color", "never",
    ]);
    assert_eq!(k.line(), "a.log:1:3: note: n [num]");
    dir.write("a.log", "z\n");
    assert!(k.note().contains("a.log: truncated"));
    dir.append("a.log", "w 4\n");
    k.quiet();

    let out = dir.trex(&["scan", "\\N", "a.log", "-m", "1", "--keep-count"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("takes --follow"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "\\N", "a.log", "--follow", "--keep-count"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no -m was given"), "{}", stderr(&out));
}

#[test]
fn a_followed_rewrite_and_redaction_write_what_the_file_gains() {
    let dir = Dir::new("edit-f");
    dir.write("app.log", "a 1\n");
    let f = dir.follow(&["rewrite", "\\N:n", "<${n}>", "app.log", "--follow"]);
    assert_eq!(f.line(), "a <1>");
    dir.append("app.log", "b 2\n");
    assert_eq!(f.line(), "b <2>");
    let g = dir.follow(&["redact", "\\N", "app.log", "--tail", "1", "--follow"]);
    assert_eq!(g.line(), "b *");
    dir.append("app.log", "c 33\n");
    assert_eq!(g.line(), "c **");
}

#[test]
fn a_followed_rewrite_starts_its_count_again_on_a_new_file_unless_kept() {
    let dir = Dir::new("edit-count-f");
    dir.write("app.log", "a 1 2\n");
    let f = dir.follow(&["rewrite", "\\N", "#", "app.log", "--follow", "-m", "1"]);
    assert_eq!(f.line(), "a # 2");
    dir.append("app.log", "b 3\n");
    assert_eq!(f.line(), "b 3");
    dir.write("app.log", "c 4 5\n");
    assert!(f.note().contains("app.log: truncated"));
    // The file under the name is a new input, and gets its own first match.
    assert_eq!(f.line(), "c # 5");

    let g = dir.follow(&["rewrite", "\\N", "#", "app.log", "--follow", "-m", "1", "--keep-count"]);
    assert_eq!(g.line(), "c # 5");
    dir.write("app.log", "d 6\n");
    assert!(g.note().contains("app.log: truncated"));
    assert_eq!(g.line(), "d 6");

    let out = dir.trex(&["rewrite", "\\N", "#", "app.log", "-m", "1", "--keep-count"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("takes --follow"), "{}", stderr(&out));
    let out = dir.trex(&["rewrite", "\\N", "#", "app.log", "--follow", "--keep-count"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no -m was given"), "{}", stderr(&out));
}
