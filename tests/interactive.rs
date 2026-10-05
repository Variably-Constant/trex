//! `rewrite --interactive`: each edit shown as its diff and accepted,
//! skipped or edited at the prompt, `a` accepting the rest, `q` or the end
//! of the answers leaving every file as it was, and `-U` applying all.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// A directory of inputs under the temp directory, removed on drop, that
/// commands run inside so their reports name files by their bare names.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/interactive-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Dir(dir)
    }

    fn write(&self, name: &str, text: &str) -> &Dir {
        std::fs::write(self.0.join(name), text).expect("write input");
        self
    }

    fn read(&self, name: &str) -> String {
        String::from_utf8(std::fs::read(self.0.join(name)).expect("read input")).expect("utf-8")
    }

    /// Run trex with `answers` on its standard input.
    fn trex(&self, args: &[&str], answers: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .current_dir(&self.0)
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run trex");
        child.stdin.take().expect("a stdin").write_all(answers.as_bytes()).expect("write the answers");
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
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

fn inputs(tag: &str) -> Dir {
    let dir = Dir::new(tag);
    dir.write("a.txt", "one 1\ntwo 2\n").write("b.txt", "three 3\n");
    dir
}

#[test]
fn each_change_is_shown_as_its_diff_and_answered() {
    let dir = inputs("answers");
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "b.txt", "--interactive"], "y\nn\ny\n");
    assert!(out.status.success(), "{}", stderr(&out));
    let shown = stdout(&out);
    // The template of the change is between its diff and its prompt, so
    // an answer for the whole template is offered with the count it carries.
    assert!(shown.starts_with("--- a.txt\n+++ a.txt\n@@ -1,2 +1,2 @@\n-one 1\n+one <1>\n two 2\n  template: number (2 later changes share it)\n[1/3] accept, skip or edit this change, all of this template, none of it, accept all, or quit [y/n/e/t/T/a/q]? "), "{shown}");
    assert!(shown.contains("-two 2\n+two <2>\n  template: number (1 later change shares it)\n[2/3] accept"), "{shown}");
    assert!(shown.contains("--- b.txt\n+++ b.txt\n@@ -1,1 +1,1 @@\n-three 3\n+three <3>\n  template: number (0 later changes share it)\n[3/3] accept"), "{shown}");
    assert_eq!(dir.read("a.txt"), "one <1>\ntwo 2\n");
    assert_eq!(dir.read("b.txt"), "three <3>\n");
    assert_eq!(stderr(&out), "a.txt: 1 replacement\nb.txt: 1 replacement\n");
}

#[test]
fn quitting_or_running_out_of_answers_leaves_every_file_as_it_was() {
    let dir = inputs("quit");
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "b.txt", "-i"], "y\nq\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("quit; every file is left as it was"), "{}", stderr(&out));
    assert_eq!(dir.read("a.txt"), "one 1\ntwo 2\n");
    assert_eq!(dir.read("b.txt"), "three 3\n");
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "b.txt", "-i"], "y\ny\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("the answers ended; every file is left as it was"), "{}", stderr(&out));
    assert_eq!(dir.read("a.txt"), "one 1\ntwo 2\n");
}

#[test]
fn accepting_all_and_update_all_apply_the_rest_without_asking() {
    let dir = inputs("all");
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "b.txt", "--interactive"], "n\na\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).matches("accept, skip or edit").count(), 2, "{}", stdout(&out));
    assert_eq!(dir.read("a.txt"), "one 1\ntwo <2>\n");
    assert_eq!(dir.read("b.txt"), "three <3>\n");
    let dir = inputs("update-all");
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "b.txt", "--interactive", "-U"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(dir.read("a.txt"), "one <1>\ntwo <2>\n");
    assert_eq!(dir.read("b.txt"), "three <3>\n");
    assert_eq!(stderr(&out), "a.txt: 2 replacements\nb.txt: 1 replacement\n");
}

#[test]
fn editing_takes_a_replacement_at_the_prompt_and_a_wrong_answer_is_asked_again() {
    let dir = inputs("edit");
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "--interactive"], "e\n[one]\nx\ny\n");
    assert!(out.status.success(), "{}", stderr(&out));
    let shown = stdout(&out);
    assert!(shown.contains("[y/n/e/t/T/a/q]? replacement: "), "{shown}");
    assert!(shown.contains("\"x\" is not an answer: y accepts, n skips, e edits, t takes every change of this template, T skips them, a accepts all, q quits\n"), "{shown}");
    assert_eq!(dir.read("a.txt"), "one [one]\ntwo <2>\n");
    assert_eq!(stderr(&out), "a.txt: 2 replacements\n");
}

#[test]
fn a_redaction_is_reviewed_as_a_rewrite_is() {
    let dir = inputs("redact");
    let out = dir.trex(&["redact", "\\N", "a.txt", "b.txt", "--interactive"], "y\nn\ny\n");
    assert!(out.status.success(), "{}", stderr(&out));
    let shown = stdout(&out);
    assert!(shown.starts_with("--- a.txt\n+++ a.txt\n@@ -1,2 +1,2 @@\n-one 1\n+one *\n two 2\n  template: number (2 later changes share it)\n[1/3] accept, skip or edit this change"), "{shown}");
    assert_eq!(dir.read("a.txt"), "one *\ntwo 2\n");
    assert_eq!(dir.read("b.txt"), "three *\n");
    assert_eq!(stderr(&out), "a.txt: 1 replacement\nb.txt: 1 replacement\n");
    let out = dir.trex(&["redact", "\\N", "a.txt", "-i"], "q\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("trex redact: quit; every file is left as it was"), "{}", stderr(&out));
    assert_eq!(dir.read("a.txt"), "one *\ntwo 2\n");
    // Under --explain each change carries what a scan's explanation says of
    // its match.
    let out = dir.trex(&["redact", "\\N", "a.txt", "-i", "--explain"], "n\n");
    assert!(stdout(&out).contains("  tokens: "), "{}", stdout(&out));
    assert!(stdout(&out).contains("  route: "), "{}", stdout(&out));
    let dir = inputs("redact-all");
    let out = dir.trex(&["redact", "\\N", "a.txt", "b.txt", "-i", "-U"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(dir.read("a.txt"), "one *\ntwo *\n");
    assert_eq!(dir.read("b.txt"), "three *\n");
}

#[test]
fn a_flag_only_a_review_reads_is_refused_without_one() {
    let dir = inputs("lone");
    for (args, said) in [
        (&["rewrite", "\\N", "x", "a.txt", "--show-skipped"][..], "which only --interactive gives"),
        (&["rewrite", "\\N", "x", "a.txt", "--in-place", "--explain"][..], "give --interactive"),
        (&["redact", "\\N", "a.txt", "--show-skipped"][..], "which only --interactive gives"),
        (&["redact", "\\N", "a.txt", "--dry-run", "--explain"][..], "give --interactive"),
    ] {
        let out = dir.trex(args, "");
        assert!(!out.status.success(), "{args:?}");
        assert!(stderr(&out).contains(said), "{args:?}: {}", stderr(&out));
    }
    assert_eq!(dir.read("a.txt"), "one 1\ntwo 2\n");
}

#[test]
fn a_count_rewrites_the_first_matches_of_each_input_however_they_are_written() {
    let dir = inputs("count");
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "-m", "1"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "one <1>\ntwo 2\n");
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "b.txt", "--dry-run", "-m", "1"], "");
    assert!(stdout(&out).contains("-one 1\n+one <1>\n two 2\n"), "{}", stdout(&out));
    assert!(stdout(&out).contains("-three 3\n+three <3>\n"), "{}", stdout(&out));
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "b.txt", "-i", "-m", "1"], "y\ny\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).matches("accept, skip or edit").count(), 2, "{}", stdout(&out));
    assert_eq!(dir.read("a.txt"), "one <1>\ntwo 2\n");
    let out = dir.trex(&["rewrite", "\\N", "#", "a.txt", "--in-place", "--max-count", "2"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(dir.read("a.txt"), "one <#>\ntwo #\n");
    let out = dir.trex(&["rewrite", "\\N", "#", "a.txt", "-m", "x"], "");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("-m needs a number"), "{}", stderr(&out));
}

#[test]
fn a_forced_backend_finds_the_matches_of_a_file_edit_and_says_what_ran() {
    let dir = inputs("backend");
    // Whether a device is present or not, the edits are the same, and what
    // ran is said once for the run.
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "b.txt", "--in-place", "--gpu"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stderr(&out).matches("gpu: ").count(), 1, "{}", stderr(&out));
    assert_eq!(dir.read("a.txt"), "one <1>\ntwo <2>\n");
    assert_eq!(dir.read("b.txt"), "three <3>\n");
    let out = dir.trex(&["rewrite", "\\N", "#", "a.txt", "--dry-run", "--cpu"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!stderr(&out).contains("gpu: "), "{}", stderr(&out));
    assert!(stdout(&out).contains("+one <#>"), "{}", stdout(&out));
}

#[test]
fn a_review_needs_files_and_no_other_way_of_writing() {
    let dir = inputs("refused");
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "a.txt", "--interactive", "--dry-run"], "");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--interactive reviews each change"), "{}", stderr(&out));
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "--interactive", "--text", "1"], "");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("need a FILE"), "{}", stderr(&out));
    let out = dir.trex(&["rewrite", "\\N", "<${0}>", "-", "--interactive"], "1\n");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("cannot write the standard input"), "{}", stderr(&out));
    let out = dir.trex(&["rewrite", "\\E", "<${0}>", "a.txt", "--interactive"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "no match\n");
}
