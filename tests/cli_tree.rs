//! The scan and rewrite commands over files and trees, driven through the
//! compiled binary: a directory walked under ignore rules, matches reported
//! as `path:line:col:` with context lines, counts and file lists, the
//! standard input, and an in-place rewrite with its dry-run diff.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::io::Write;

/// A fresh directory under the target directory's temp space, removed when
/// dropped.
struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new(name: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("trex/cli-tree-{name}-{}", std::process::id()));
        match std::fs::remove_dir_all(&root) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("cannot clear {}: {e}", root.display()),
        }
        std::fs::create_dir_all(&root).expect("create tree root");
        Tree { root }
    }

    fn write(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, bytes).expect("write file");
        path
    }

    fn read(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.root.join(rel)).expect("read file")
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.root) {
            Ok(()) => {}
            Err(e) => eprintln!("cannot remove {}: {e}", self.root.display()),
        }
    }
}

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn trex_with_stdin(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_trex"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn trex");
    child.stdin.take().expect("stdin handle").write_all(stdin).expect("write stdin");
    child.wait_with_output().expect("wait for trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

/// What trex itself wrote to stderr, with a dependency's own diagnostics
/// dropped.
///
/// A library on this stderr is not trex's output, and an assertion on the
/// exact text is about what trex reports. Flynnel announces on startup when
/// the host's stored dispatch calibration disagrees with what this process
/// measures - "flynnel: this host's stored calibration dispatches in 400 ns
/// and this one in 1300" - which is a correct use of stderr and arrives
/// whenever that record is stale, so it reaches a run that asks for none of
/// it. Named rather than filtered by shape, so a line trex itself grows is
/// still compared.
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr)
        .replace("\r\n", "\n")
        .lines()
        .filter(|l| !l.starts_with("flynnel:"))
        .map(|l| format!("{l}\n"))
        .collect()
}

/// Every line of a report, with the tree root replaced by `ROOT` and its
/// separators by `/`, in plain and in JSON-escaped form, so the expected text
/// does not depend on where the tree was made.
fn relative(text: &str, root: &Path) -> String {
    let root_text = root.to_string_lossy().into_owned();
    let escaped = root_text.replace('\\', "\\\\");
    text.replace(&escaped, "ROOT")
        .replace(&root_text, "ROOT")
        .replace("\\\\", "/")
        .replace('\\', "/")
}

fn sample_tree(name: &str) -> Tree {
    let t = Tree::new(name);
    t.write("a.log", b"alpha 10\nbeta 20\ngamma 300\ndelta 4000\nepsilon 5\n");
    t.write("sub/b.txt", b"one 1\ntwo 2\n");
    t.write("ignored.log", b"nine 9000\n");
    t.write(".hidden.log", b"eight 8000\n");
    t.write("blob.bin", b"seven 7000\0");
    t.write(".ignore", b"ignored.log\n");
    t
}

#[test]
fn a_tree_is_walked_under_ignore_rules_and_reported_by_path_line_and_column() {
    let t = sample_tree("walk");
    let out = trex(&["scan", "\\N{>=20}", &t.path().to_string_lossy()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let got = relative(&stdout(&out), t.path());
    assert_eq!(
        got,
        "ROOT/a.log:2:6: \"20\"\nROOT/a.log:3:7: \"300\"\nROOT/a.log:4:7: \"4000\"\n",
        "hidden, ignored and binary files are skipped and the rest report in path order"
    );

    let out = trex(&["scan", "\\N{>=20}", &t.path().to_string_lossy(), "--hidden", "--no-ignore", "--binary"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let got = relative(&stdout(&out), t.path());
    assert!(got.contains("ROOT/.hidden.log:1:7: \"8000\"\n"), "{got}");
    assert!(got.contains("ROOT/ignored.log:1:6: \"9000\"\n"), "{got}");
    assert!(got.contains("ROOT/blob.bin:1:7: \"7000\"\n"), "{got}");
}

#[test]
fn context_lines_surround_each_match_once() {
    let t = sample_tree("context");
    let a = t.path().join("a.log");
    let out = trex(&["scan", "\\N{>=300}", &a.to_string_lossy(), "-C", "1", "-H"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let got = relative(&stdout(&out), t.path());
    assert_eq!(
        got,
        "ROOT/a.log-2-beta 20\nROOT/a.log:3:7: \"300\"\nROOT/a.log:4:7: \"4000\"\nROOT/a.log-5-epsilon 5\n",
        "adjacent matches share their context and no line prints twice"
    );

    let out = trex(&["scan", "\"alpha\"|\"epsilon\"", &a.to_string_lossy(), "-A", "1"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "1:1: \"alpha\"\n2-beta 20\n--\n5:1: \"epsilon\"\n",
        "one input with context prints line:col and a -- between groups that do not touch"
    );
}

#[test]
fn counts_file_lists_and_json_carry_the_path() {
    let t = sample_tree("count");
    let root = t.path().to_string_lossy().into_owned();
    let out = trex(&["scan", "\\N", &root, "--count"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(relative(&stdout(&out), t.path()), "ROOT/a.log:5\nROOT/sub/b.txt:2\n");

    let out = trex(&["scan", "\\N{>=1000}", &root, "-l"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(relative(&stdout(&out), t.path()), "ROOT/a.log\n");

    let out = trex(&["scan", "\\N{>=1000}", &root, "--json"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let got = relative(&stdout(&out), t.path());
    assert_eq!(
        got,
        "[{\"path\":\"ROOT/a.log\",\"line\":4,\"col\":7,\"start\":33,\"end\":37,\"text\":\"4000\",\"captures\":{}}]\n"
    );

    let out = trex(&["scan", "\\N{>=1000000}", &root, "--require-match"]);
    assert!(!out.status.success(), "--require-match fails when no file matches");
}

#[test]
fn stdin_is_one_unnamed_input() {
    let out = trex_with_stdin(&["scan", "\\N"], b"x 42 y\n");
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "[2..4] \"42\"\n");

    let out = trex_with_stdin(&["scan", "\\N", "-", "-H"], b"x 42 y\n");
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "-:1:3: \"42\"\n");
}

#[test]
fn a_binary_file_named_outright_is_refused_without_the_flag() {
    let t = sample_tree("binary");
    let blob = t.path().join("blob.bin");
    let out = trex(&["scan", "\\N", &blob.to_string_lossy()]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("binary"), "{}", stderr(&out));

    let out = trex(&["scan", "\\N", &blob.to_string_lossy(), "--binary"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "[6..10] \"7000\"\n");
}

#[test]
fn an_edit_of_one_named_binary_file_is_refused_and_a_walk_passes_over_one() {
    let t = sample_tree("binary-edit");
    let blob = t.path().join("blob.bin").to_string_lossy().into_owned();
    for (args, verb) in [
        (vec!["rewrite", "\\N", "[${0}]", blob.as_str(), "--dry-run"], "rewrites"),
        (vec!["rewrite", "\\N", "[${0}]", blob.as_str(), "--in-place"], "rewrites"),
        (vec!["redact", "\\N", blob.as_str(), "--dry-run"], "redacts"),
        (vec!["rewrite", "\\N", "[${0}]", blob.as_str(), "--interactive"], "rewrites"),
    ] {
        let out = trex_with_stdin(&args, b"");
        assert!(!out.status.success(), "{args:?}");
        let notice = format!("blob.bin holds a NUL byte and is binary; --binary {verb} it");
        assert!(stderr(&out).contains(&notice), "{args:?}: {}", stderr(&out));
        assert_eq!(stdout(&out), "", "{args:?}");
    }
    assert_eq!(t.read("blob.bin"), b"seven 7000\0", "a refused file is left alone");

    let out = trex(&["rewrite", "\\N", "[${0}]", &blob, "--dry-run", "--binary"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("+seven [7000]"), "{}", stdout(&out));

    let root = t.path().to_string_lossy().into_owned();
    let out = trex(&["rewrite", "\\N{>=1000000}", "[${0}]", &root, "--dry-run"]);
    assert!(out.status.success(), "a walk passes over a binary file: {}", stderr(&out));
    assert!(!stderr(&out).contains("binary"), "{}", stderr(&out));
}

#[test]
fn a_dry_run_prints_the_diff_and_an_in_place_run_writes_every_file() {
    let t = sample_tree("rewrite");
    let root = t.path().to_string_lossy().into_owned();
    let out = trex(&["rewrite", "\\N{>=1000}", "[${0}]", &root, "--dry-run", "-C", "1"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let got = relative(&stdout(&out), t.path());
    assert_eq!(
        got,
        "--- ROOT/a.log\n+++ ROOT/a.log\n@@ -3,3 +3,3 @@\n gamma 300\n-delta 4000\n+delta [4000]\n epsilon 5\n",
        "a unified diff per file that would change, with the asked context"
    );
    assert_eq!(t.read("a.log"), b"alpha 10\nbeta 20\ngamma 300\ndelta 4000\nepsilon 5\n", "a dry run writes nothing");

    let out = trex(&["rewrite", "\\N{>=2}", "<${0}>", &root]);
    assert!(!out.status.success(), "a tree needs --in-place or --dry-run");

    let out = trex(&["rewrite", "\\N{>=2}", "<${0}>", &root, "--in-place"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let said = relative(&stderr(&out), t.path());
    assert_eq!(said, "ROOT/a.log: 5 replacements\nROOT/sub/b.txt: 1 replacement\n");
    assert_eq!(t.read("a.log"), b"alpha <10>\nbeta <20>\ngamma <300>\ndelta <4000>\nepsilon <5>\n");
    assert_eq!(t.read("sub/b.txt"), b"one 1\ntwo <2>\n");
    assert_eq!(t.read("ignored.log"), b"nine 9000\n", "ignored files are left alone");
    assert_eq!(t.read("blob.bin"), b"seven 7000\0", "binary files are left alone");
}

#[test]
fn a_pattern_file_declares_names_and_the_library_answers_without_one() {
    let t = Tree::new("lib");
    let lib = t.write("defs.trex", b"# assignments\nlet rhs = \\N\nkind assign = \\W \"=\" \\{rhs}\n");
    let src = t.write("prog.txt", b"let x = 1; let y = 22;\n");
    let out = trex(&["scan", "\\{assign}", &src.to_string_lossy(), "--lib", &lib.to_string_lossy()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "[4..9] \"x = 1\"\n[15..21] \"y = 22\"\n");

    let out = trex(&["scan", "\\{iban}", "--text", "pay GB82WEST12345698765432 now, not GB82WEST12345698765433"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "[4..26] \"GB82WEST12345698765432\"\n");

    let out = trex(&["scan", "\\{log_level} \\W", "--text", "warn disk full; info ok"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "[0..9] \"warn disk\"\n[16..23] \"info ok\"\n");

    let out = trex(&["lib"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let listing = stdout(&out);
    assert!(listing.contains("iban") && listing.contains("checked"), "{listing}");
    let out = trex(&["lib", "--json"]);
    assert!(stdout(&out).contains("\"name\":\"cve\""));

    let bad = t.write("bad.trex", b"let ok = \\N\nshrug x = `a`\n");
    let out = trex(&["scan", "\\N", &src.to_string_lossy(), "--lib", &bad.to_string_lossy()]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("line 2"), "{}", stderr(&out));
}

#[test]
fn a_rewrite_lexes_under_a_declared_shape_as_every_other_command_does() {
    let t = Tree::new("rewritelib");
    // A kind fuses the tokens its pattern covers into one, so a rewrite
    // under it replaces the whole assignment rather than the number inside
    // it, and the boundaries the scan found the match on are the ones the
    // registers are read on.
    let lib = t.write("defs.trex", b"let rhs = \\N\nkind assign = \\W \"=\" \\{rhs}\n");
    let src = t.write("prog.txt", b"let x = 1; let y = 22;\n");
    let lib = lib.to_string_lossy().into_owned();
    let src = src.to_string_lossy().into_owned();
    let out = trex(&["rewrite", "\\{assign}", "<${0}>", &src, "--lib", &lib]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "let <x = 1>; let <y = 22>;\n");

    // A shape the lexer runs before its own recognizers, and a template
    // reading a register bound over it.
    let shapes = t.write("shapes.trex", b"shape ticket = `[A-Z]{2,4}-\\d{1,4}`\n");
    let notes = t.write("notes.txt", b"see AB-12 and XYZ-9 now\n");
    let shapes = shapes.to_string_lossy().into_owned();
    let notes = notes.to_string_lossy().into_owned();
    let out = trex(&["rewrite", "\\{ticket}:t", "[${t}]", &notes, "--lib", &shapes]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "see [AB-12] and [XYZ-9] now\n");

    // And in place, through the same edits a dry run diffs.
    let out = trex(&["rewrite", "\\{ticket}", "T", &notes, "--lib", &shapes, "--dry-run"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("-see AB-12 and XYZ-9 now"), "{}", stdout(&out));
    let out = trex(&["rewrite", "\\{ticket}", "T", &notes, "--lib", &shapes, "--in-place"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(t.read("notes.txt"), b"see T and T now\n");
}

#[test]
fn one_file_rewrites_to_the_standard_output_as_before() {
    let t = sample_tree("stdout");
    let b = t.path().join("sub/b.txt");
    let out = trex(&["rewrite", "\\N", "(${0})", &b.to_string_lossy()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "one (1)\ntwo (2)\n");
    assert_eq!(t.read("sub/b.txt"), b"one 1\ntwo 2\n");
}
