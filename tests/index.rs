//! The tree index: `trex index DIR` writing it, a scan pruning with it, and
//! the contract that binds both, which is that an index may change how long
//! a scan takes and never what it reports.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A tree under the temp directory, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/index-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Dir(dir)
    }

    fn write(&self, name: &str, text: &str) -> &Dir {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the file's directory");
        }
        std::fs::write(path, text).expect("write input");
        self
    }

    fn has(&self, name: &str) -> bool {
        self.0.join(name).exists()
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

/// The report, with line endings and path separators read the one way, so
/// an expectation is written once and holds on either platform.
fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n").replace('\\', "/")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

/// A tree where each pattern under test matches in exactly one file, so a
/// prune that loses a match loses a visible one.
fn tree(tag: &str) -> Dir {
    let dir = Dir::new(tag);
    dir.write("logs/a.log", "from 10.0.0.1 at 2026-09-15T10:00:00Z\n")
        .write("logs/b.log", "nothing of interest here\n")
        .write("notes/c.txt", "mail bob@x.com about it\n")
        .write("notes/d.txt", "a plain note with words\n")
        .write("counts.txt", "small 12 and 40 here\n");
    dir
}

#[test]
fn an_index_is_written_and_read_back_by_the_command() {
    let dir = tree("command");
    let out = dir.trex(&["index", "."]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), ".: 5 files indexed\n");
    assert!(dir.has(".trex-index"), "the index is written at the tree's root");
    let out = dir.trex(&["index", ".", "--list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("5 files indexed"), "{}", stdout(&out));
    // A file is not a tree, and a tree with no index has none to list.
    let out = dir.trex(&["index", "counts.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("is not a directory"), "{}", stderr(&out));
    let out = dir.trex(&["index", "logs", "--list"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("holds no index"), "{}", stderr(&out));
}

#[test]
fn a_scan_reports_the_same_matches_with_the_index_as_without() {
    let dir = tree("same");
    // Every pattern, over the same tree, before and after the index exists.
    let patterns = ["\\I", "\\T", "\\E", "\\N{>=100}", "\\N{>=10}", "\"mail\" \\E", "\\W", "\\W \\N"];
    let before: Vec<String> = patterns.iter().map(|p| stdout(&dir.trex(&["scan", p, "."]))).collect();
    let out = dir.trex(&["index", "."]);
    assert!(out.status.success(), "{}", stderr(&out));
    for (p, want) in patterns.iter().zip(&before) {
        let got = stdout(&dir.trex(&["scan", p, "."]));
        assert_eq!(&got, want, "{p}: the index changed what the scan reported");
        // And the same again with the index explicitly ignored.
        let bare = stdout(&dir.trex(&["scan", p, ".", "--no-index"]));
        assert_eq!(&bare, want, "{p}: --no-index changed what the scan reported");
    }
}

#[test]
fn a_changed_file_is_scanned_though_the_index_answered_for_it() {
    let dir = tree("stale");
    assert!(dir.trex(&["index", "."]).status.success());
    // b.log held no address when the index was built.
    let out = dir.trex(&["scan", "\\I", "logs"]);
    assert_eq!(stdout(&out), "logs/a.log:1:6: \"10.0.0.1\"\n");
    // It holds one now, and the index must not hide it: the entry is for a
    // file of another size and time, so the file is scanned. The rewrite
    // changes the file's LENGTH deliberately, so this holds whatever the
    // filesystem's time resolution is and the test cannot pass by the
    // accident of crossing a tick.
    dir.write("logs/b.log", "now from 10.0.0.9 over here as well\n");
    let out = dir.trex(&["scan", "\\I", "logs"]);
    assert_eq!(stdout(&out), "logs/a.log:1:6: \"10.0.0.1\"\nlogs/b.log:1:10: \"10.0.0.9\"\n");
    // The same holds for a file rewritten to exactly its old length, which
    // only the modified time can catch.
    dir.write("logs/b.log", "now from 10.0.0.8 over here as well\n");
    let out = dir.trex(&["scan", "\\I", "logs"]);
    assert!(stdout(&out).contains("10.0.0.8"), "a same-length rewrite was missed: {}", stdout(&out));
    // A file the index never saw is scanned too.
    dir.write("logs/e.log", "and 10.0.0.7 as well\n");
    let out = dir.trex(&["scan", "\\I", "logs", "--count"]);
    assert!(stdout(&out).contains("logs/e.log:1"), "{}", stdout(&out));
}

#[test]
fn an_index_built_as_a_scan_runs_serves_every_scan_after_it() {
    let dir = tree("onthefly");
    let out = dir.trex(&["scan", "\\W", ".", "--index", "--count"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("indexed 5 files"), "{}", stderr(&out));
    assert!(dir.has(".trex-index"));
    // The counts are the tree's own, index or no index.
    let counted = stdout(&out);
    assert_eq!(counted, stdout(&dir.trex(&["scan", "\\W", ".", "--count", "--no-index"])));
    // And the scan that built it now prunes with it.
    let out = dir.trex(&["scan", "\\E", "."]);
    assert_eq!(stdout(&out), "./notes/c.txt:1:6: \"bob@x.com\"\n");
}

#[test]
fn a_set_and_a_rule_file_skip_a_file_only_where_no_member_can_match() {
    let dir = tree("set");
    dir.write("rules.trex", "rule addr warning \"an address\" = \\I\nrule mail warning \"an email\" = \\E\n")
        .write("members.trex", "let addr = \\I\nlet mail = \\E\n");
    assert!(dir.trex(&["index", "."]).status.success());
    // The set's members between them need an address or an email, so only
    // the files with neither are skipped, and the report is unchanged.
    let with = stdout(&dir.trex(&["scan", "--patterns", "members.trex", "."]));
    let without = stdout(&dir.trex(&["scan", "--patterns", "members.trex", ".", "--no-index"]));
    assert_eq!(with, without);
    assert!(with.contains("10.0.0.1") && with.contains("bob@x.com"), "{with}");
    let with = stdout(&dir.trex(&["scan", "--rules", "rules.trex", "."]));
    let without = stdout(&dir.trex(&["scan", "--rules", "rules.trex", ".", "--no-index"]));
    assert_eq!(with, without);
    assert!(with.contains("[addr]") && with.contains("[mail]"), "{with}");
}

#[test]
fn the_reports_that_name_files_without_matches_read_no_index() {
    let dir = tree("misses");
    assert!(dir.trex(&["index", "."]).status.success());
    // -L names the files with no match, which a skipped file is: it must be
    // named, not silently passed over.
    let out = dir.trex(&["scan", "-L", "\\E", "."]);
    let named = stdout(&out);
    assert!(named.contains("logs/a.log") && named.contains("counts.txt"), "{named}");
    assert!(!named.contains("notes/c.txt"), "{named}");
    assert_eq!(named, stdout(&dir.trex(&["scan", "-L", "\\E", ".", "--no-index"])));
    // -v prints the lines no match touches, in every file.
    let out = dir.trex(&["scan", "-v", "\\E", "."]);
    assert_eq!(stdout(&out), stdout(&dir.trex(&["scan", "-v", "\\E", ".", "--no-index"])));
}

#[test]
fn a_named_file_is_scanned_whatever_the_index_says() {
    let dir = tree("named");
    assert!(dir.trex(&["index", "."]).status.success());
    // Naming a file is asking for it, as it is when a glob would drop it.
    let out = dir.trex(&["scan", "\\E", "logs/b.log"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "no match\n");
}
