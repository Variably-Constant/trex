//! Many patterns scanned as one set, each match under the member that made
//! it: a `PatternSet` built from a pattern file, `scan --patterns FILE` in
//! every report, `--single-match`, a record query over the members, the
//! aggregates keyed by `${pattern}`, and a stream over a set.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// A directory of inputs under the temp directory, removed on drop, that
/// commands run inside so their reports name files by their bare names.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/sets-{tag}-{}", std::process::id()));
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

    /// Run with `input` written to the standard input, and no path, so the
    /// scan reads the stream as it arrives.
    fn trex_stdin(&self, args: &[&str], input: &str) -> Output {
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
        stdin.write_all(input.as_bytes()).expect("write the stream");
        drop(stdin);
        child.wait_with_output().expect("wait for trex")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove temp dir");
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Three members: two lets under their names and a bare line under its
/// line number, behind a comment.
const RULES: &str = "# the members\nlet host = \\I:addr\nlet mail = \\E:e\n\\N{>=100}\n";

/// Two lines: the first holds every member, the second only an address.
const NOTES: &str = "from 10.0.0.1 at 500 to bob@x.com\nx = 7 and 10.0.0.2\n";

/// The report the set gives over `NOTES`, each match under its member.
const REPORT: &str = "[5..13] \"10.0.0.1\"  captures: addr=\"10.0.0.1\"  pattern: host\n\
[17..20] \"500\"  pattern: 4\n\
[24..33] \"bob@x.com\"  captures: e=\"bob@x.com\"  pattern: mail\n\
[44..52] \"10.0.0.2\"  captures: addr=\"10.0.0.2\"  pattern: host\n";

fn span_of(m: &trex::Match) -> trex::Span {
    let at = |offset: usize| u32::try_from(offset).expect("an offset fits a span");
    trex::Span { start: at(m.start), end: at(m.end) }
}

/// Every member's matches asked one at a time, with their registers, in
/// position order: what the set must report.
fn alone(set: &trex::PatternSet, input: &[u8], shapes: &trex::ShapeSet) -> Vec<(usize, trex::Match)> {
    let mut want: Vec<(usize, trex::Match)> = Vec::new();
    for (i, p) in set.patterns().iter().enumerate() {
        let spans = trex::scan_with_shapes(p, input, shapes);
        want.extend(trex::captures_with_shapes(p, input, shapes, &spans).into_iter().map(|m| (i, m)));
    }
    want.sort_by_key(|(i, m)| (m.start, m.end, *i));
    want
}

/// The first match of each member of `all`, in member order.
fn firsts(all: &[(usize, trex::Match)]) -> Vec<(usize, trex::Match)> {
    let mut out: Vec<(usize, trex::Match)> = Vec::new();
    for (i, m) in all {
        if !out.iter().any(|(j, _)| j == i) {
            out.push((*i, m.clone()));
        }
    }
    out.sort_by_key(|(i, _)| *i);
    out
}

#[test]
fn a_set_from_a_file_reports_what_its_members_report_alone() {
    let mut shapes = trex::ShapeSet::new();
    let set = trex::PatternSet::from_text(RULES, &mut shapes).expect("the file parses");
    assert_eq!(set.names(), ["host", "mail", "4"]);
    assert_eq!(set.name(2), "4");
    let input = NOTES.as_bytes();
    let want = alone(&set, input, &shapes);
    assert_eq!(want.len(), 4);
    assert_eq!(set.scan_matches(input, false), want);
    let spans: Vec<(usize, trex::Span)> = want.iter().map(|(i, m)| (*i, span_of(m))).collect();
    assert_eq!(set.scan(input), spans);
    let mut first = firsts(&want);
    first.sort_by_key(|(i, m)| (m.start, m.end, *i));
    assert_eq!(set.first_matches(input, false), first);
    assert_eq!(set.matches(input), [0, 1, 2]);
    assert!(set.is_match(input));
    let mut first_spans: Vec<(usize, trex::Span)> = firsts(&want).iter().map(|(i, m)| (*i, span_of(m))).collect();
    first_spans.sort_by_key(|(i, _)| *i);
    assert_eq!(set.matches_with_spans(input), first_spans);
    // A set built from patterns alone goes by index.
    let bare = trex::PatternSet::new(set.patterns().to_vec());
    assert!(bare.names().is_empty());
    assert_eq!(bare.name(1), "1");
    assert_eq!(bare.scan(input), spans);
}

#[test]
fn a_shaped_file_lexes_every_member_under_its_shapes_once() {
    // A shape fuses `v12` into one token, which a byte route would never see,
    // so every member is lexed under it, and the answers are the members'
    // own under the same shapes.
    let text = "shape vtag = `v[0-9]{1,3}`\nlet release = \\{vtag}:v\n\\W \"=\"\n";
    let mut shapes = trex::ShapeSet::new();
    let set = trex::PatternSet::from_text(text, &mut shapes).expect("the file parses");
    assert_eq!(set.names(), ["release", "3"]);
    assert!(!set.shapes().is_empty());
    let input = b"app = v12 and lib = v7\n";
    let want = alone(&set, input, &shapes);
    assert_eq!(want.len(), 4, "{want:?}");
    assert_eq!(set.scan_matches(input, false), want);
    assert_eq!(set.matches(input), [0, 1]);
    assert!(set.is_match(input));
    assert!(!set.is_match(b"nothing at all"));
    let first: Vec<(usize, trex::Span)> = firsts(&want).iter().map(|(i, m)| (*i, span_of(m))).collect();
    assert_eq!(set.matches_with_spans(input), first);
    assert_eq!(set.matches_at(input, 10).iter().collect::<Vec<_>>(), [0, 1]);
    assert_eq!(set.matches_at(input, 20).iter().collect::<Vec<_>>(), [0]);
}

#[test]
fn a_line_that_is_neither_a_declaration_nor_a_pattern_names_its_line() {
    let mut shapes = trex::ShapeSet::new();
    let err = trex::PatternSet::from_text("let host = \\I\n\\N{>x}\n", &mut shapes).expect_err("a bad line");
    assert!(err.msg.starts_with("line 2: pattern error"), "{}", err.msg);
    let err = trex::PatternSet::from_text("let = \\I\n", &mut shapes).expect_err("a bad let");
    assert!(err.msg.starts_with("line 1:"), "{}", err.msg);
}

#[test]
fn a_stream_over_a_set_commits_what_the_whole_scan_reports() {
    let mut shapes = trex::ShapeSet::new();
    let set = trex::PatternSet::from_text(RULES, &mut shapes).expect("the file parses");
    let input = NOTES.as_bytes();
    let whole = set.scan(input);
    for chunk in [1usize, 3, 7, 100] {
        let mut scanner = trex::StreamScanner::over_set(set.clone());
        let mut got = Vec::new();
        for piece in input.chunks(chunk) {
            scanner.push(piece);
            got.extend(scanner.drain_committed_with_members());
        }
        got.extend(scanner.finish_with_members());
        assert_eq!(got, whole, "chunk={chunk}");
    }
}

#[test]
fn the_scan_report_names_each_matchs_member_after_its_captures() {
    let dir = Dir::new("report");
    dir.write("rules.trex", RULES).write("notes.txt", NOTES);
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "notes.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), REPORT);
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "--text", NOTES]);
    assert_eq!(stdout(&out), REPORT);
    // A register binding one typed kind carries its parsed value beside its
    // text: `addr` binds an address and reports the integer it orders by,
    // quoted because a v6 address is 128 bits and a JSON number is a double.
    // `e` binds an email, which carries no single value, so it is a bare
    // string, as every untyped register is.
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "--json", "notes.txt"]);
    assert_eq!(
        stdout(&out),
        "[{\"start\":5,\"end\":13,\"text\":\"10.0.0.1\",\"captures\":{\"addr\":{\"text\":\"10.0.0.1\",\"value\":\"167772161\"}},\"pattern\":\"host\"},\
{\"start\":17,\"end\":20,\"text\":\"500\",\"captures\":{},\"pattern\":\"4\"},\
{\"start\":24,\"end\":33,\"text\":\"bob@x.com\",\"captures\":{\"e\":\"bob@x.com\"},\"pattern\":\"mail\"},\
{\"start\":44,\"end\":52,\"text\":\"10.0.0.2\",\"captures\":{\"addr\":{\"text\":\"10.0.0.2\",\"value\":\"167772162\"}},\"pattern\":\"host\"}]\n"
    );
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "--format", "${pattern}:${line}:${0}", "notes.txt"]);
    assert_eq!(stdout(&out), "host:1:10.0.0.1\n4:1:500\nmail:1:bob@x.com\nhost:2:10.0.0.2\n");
    // A template names a register of any member; a match of another member
    // renders it empty.
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "--format", "${pattern} ${addr} ${e}", "notes.txt"]);
    assert_eq!(stdout(&out), "host 10.0.0.1 \n4  \nmail  bob@x.com\nhost 10.0.0.2 \n");
    // The four matches are on two lines, which is the only shape that tells
    // the two counts apart: `--count` answers records and the record here is
    // the line, `--count-matches` answers matches.
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "--count", "notes.txt"]);
    assert_eq!(stdout(&out), "2\n");
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "--count-matches", "notes.txt"]);
    assert_eq!(stdout(&out), "4\n");
}

#[test]
fn named_inputs_and_context_carry_the_member_too() {
    let dir = Dir::new("prefixed");
    dir.write("rules.trex", RULES).write("notes.txt", NOTES).write("other.txt", "mail amy@y.org\n");
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "notes.txt", "other.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "notes.txt:1:6: \"10.0.0.1\"  captures: addr=\"10.0.0.1\"  pattern: host\n\
notes.txt:1:18: \"500\"  pattern: 4\n\
notes.txt:1:25: \"bob@x.com\"  captures: e=\"bob@x.com\"  pattern: mail\n\
notes.txt:2:11: \"10.0.0.2\"  captures: addr=\"10.0.0.2\"  pattern: host\n\
other.txt:1:6: \"amy@y.org\"  captures: e=\"amy@y.org\"  pattern: mail\n"
    );
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "--json", "other.txt", "-H"]);
    assert_eq!(
        stdout(&out),
        "[{\"path\":\"other.txt\",\"line\":1,\"col\":6,\"start\":5,\"end\":14,\"text\":\"amy@y.org\",\"captures\":{\"e\":\"amy@y.org\"},\"pattern\":\"mail\"}]\n"
    );
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "-A", "1", "other.txt"]);
    assert_eq!(stdout(&out), "1:6: \"amy@y.org\"  captures: e=\"amy@y.org\"  pattern: mail\n");
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "--explain", "--text", "at 500"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.starts_with("[3..6] \"500\"  pattern: 4\n  tokens: number \"500\"\n"), "{text}");
    assert!(text.contains("  route: "), "{text}");
}

#[test]
fn single_match_keeps_each_members_first_match() {
    let dir = Dir::new("single");
    dir.write("rules.trex", RULES).write("notes.txt", NOTES);
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "--single-match", "notes.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "[5..13] \"10.0.0.1\"  captures: addr=\"10.0.0.1\"  pattern: host\n\
[17..20] \"500\"  pattern: 4\n\
[24..33] \"bob@x.com\"  captures: e=\"bob@x.com\"  pattern: mail\n"
    );
    let out = dir.trex(&["scan", "--single-match", "\\I", "notes.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--single-match keeps each member's first match and takes --patterns"), "{}", stderr(&out));
}

#[test]
fn a_record_query_over_the_members_names_them() {
    let dir = Dir::new("query");
    dir.write("rules.trex", RULES).write("notes.txt", NOTES);
    let out = dir.trex(&["scan", "--all", "--patterns", "rules.trex", "notes.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "from 10.0.0.1 at 500 to bob@x.com\n");
    let out = dir.trex(&["scan", "--all", "--patterns", "rules.trex", "--json", "notes.txt"]);
    assert_eq!(
        stdout(&out),
        "[{\"line\":1,\"start\":0,\"end\":33,\"text\":\"from 10.0.0.1 at 500 to bob@x.com\",\"patterns\":[\"host\",\"mail\",\"4\"]}]\n"
    );
    let out = dir.trex(&["scan", "--at-least", "1", "--patterns", "rules.trex", "--not", "\\E", "--json", "notes.txt"]);
    assert_eq!(
        stdout(&out),
        "[{\"line\":2,\"start\":34,\"end\":52,\"text\":\"x = 7 and 10.0.0.2\",\"patterns\":[\"host\"]}]\n"
    );
    // The patterns given one by one still go by index.
    let out = dir.trex(&["scan", "--any", "-e", "\\E", "-e", "\\N{>=100}", "--json", "notes.txt"]);
    assert_eq!(
        stdout(&out),
        "[{\"line\":1,\"start\":0,\"end\":33,\"text\":\"from 10.0.0.1 at 500 to bob@x.com\",\"patterns\":[0,1]}]\n"
    );
}

#[test]
fn the_aggregates_count_by_member_and_by_input() {
    let dir = Dir::new("aggregate");
    dir.write("rules.trex", RULES).write("notes.txt", NOTES).write("other.txt", "mail amy@y.org\n");
    let out = dir.trex(&["count-by", "--patterns", "rules.trex", "${pattern}", "notes.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "4     1\nhost  2\nmail  1\n");
    let out = dir.trex(&["top", "--patterns", "rules.trex", "${pattern}", "notes.txt", "other.txt"]);
    assert_eq!(stdout(&out), "host  2\nmail  2\n4     1\n");
    let out = dir.trex(&["count-by", "\\E", "${path}", "notes.txt", "other.txt"]);
    assert_eq!(stdout(&out), "notes.txt  1\nother.txt  1\n");
    let out = dir.trex(&["uniq", "--patterns", "rules.trex", "${pattern}:${line}", "notes.txt"]);
    assert_eq!(stdout(&out), "4:1\nhost:1\nhost:2\nmail:1\n");
}

#[test]
fn a_stream_prints_each_match_under_its_member() {
    let dir = Dir::new("stream");
    dir.write("rules.trex", RULES);
    let out = dir.trex_stdin(&["scan", "--patterns", "rules.trex"], NOTES);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), REPORT);
    let out = dir.trex_stdin(&["scan", "--patterns", "rules.trex", "--json"], "at 500\n");
    assert_eq!(stdout(&out), "{\"start\":3,\"end\":6,\"text\":\"500\",\"captures\":{},\"pattern\":\"4\"}\n");
    let out = dir.trex_stdin(&["scan", "--patterns", "rules.trex", "--format", "${pattern}@${line}:${col}"], NOTES);
    assert_eq!(stdout(&out), "host@1:6\n4@1:18\nmail@1:25\nhost@2:11\n");
}

#[test]
fn the_flags_a_set_takes_no_part_in_are_refused() {
    let dir = Dir::new("refused");
    dir.write("rules.trex", RULES).write("bad.trex", "let host = \\I\n\\N{>x}\n").write("notes.txt", NOTES);
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "-e", "\\N", "notes.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--patterns names the members and takes no -e or -f"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--patterns", "bad.trex", "notes.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("bad.trex: line 2: pattern error"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--patterns", "rules.trex", "--gpu", "notes.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("take no --gpu, --dual-grain or --chunk-size"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--all", "--patterns", "rules.trex", "--single-match", "notes.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--single-match"), "{}", stderr(&out));
    let out = dir.trex(&["count-by", "--patterns", "rules.trex"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("a KEY template is needed"), "{}", stderr(&out));
}
