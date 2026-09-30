//! `rewrite --interactive` with template answers: `t` accepts this change and
//! every later one sharing its silhouette, `T` skips them, and `--explain`
//! puts under each diff what a scan's `--explain` puts under each match.
//!
//! The answers are piped, so a review of several changes is driven from one
//! string and what each answer carried is read off what the file became.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// Run a review with `answers` on the standard input.
fn review(args: &[&str], answers: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_trex"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run trex");
    child
        .stdin
        .as_mut()
        .expect("a standard input")
        .write_all(answers.as_bytes())
        .expect("write the answers");
    child.wait_with_output().expect("wait for trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

/// A file of two silhouettes interleaved: `word punct number` on the odd
/// lines and `word punct word` on the even ones, so a template answer has
/// later changes to carry and the other shape is still asked about.
fn conf(named: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("trex/review-{}-{named}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the directory");
    let mut text = String::new();
    for i in 1..=4 {
        text.push_str(&format!("alpha_{i} = {i}\n"));
        text.push_str(&format!("beta_{i} = gamma_{i}\n"));
    }
    let path = dir.join("conf.txt");
    std::fs::write(&path, text).expect("write the file");
    path
}

/// The pattern reaches `name = number` without a quoted literal, because `=`
/// lexes as punctuation and the three tokens are what the silhouette reads.
const PATTERN: &str = "\\W:k \\P \\N:v";
const TEMPLATE: &str = "${k} := ${v}";

#[test]
fn the_prompt_names_the_template_and_how_many_later_changes_share_it() {
    let path = conf("prompt");
    let name = path.to_string_lossy().into_owned();
    let out = review(&["rewrite", PATTERN, TEMPLATE, &name, "-i"], "n\nn\nn\nn\n");
    let text = stdout(&out);
    // The silhouette is the kinds of the matched span's tokens, in order, and
    // the count is of changes still to come rather than of all of them.
    assert!(text.contains("template: word punct number (3 later changes share it)"), "{text}");
    assert!(text.contains("template: word punct number (2 later changes share it)"), "{text}");
    // One left says so in the singular, which is the case a plural rule gets
    // wrong and a reviewer reads on every long review.
    assert!(text.contains("template: word punct number (1 later change shares it)"), "{text}");
    assert!(text.contains("[y/n/e/t/T/a/q]"), "the answers are not offered: {text}");
    std::fs::remove_dir_all(path.parent().expect("a parent")).expect("remove the directory");
}

#[test]
fn one_t_accepts_every_change_of_the_template() {
    let path = conf("accept");
    let name = path.to_string_lossy().into_owned();
    // One answer, four changes: the three later ones are never offered.
    let out = review(&["rewrite", PATTERN, TEMPLATE, &name, "-i"], "t\n");
    assert!(stderr(&out).contains("4 replacements"), "{}", stderr(&out));
    let after = std::fs::read_to_string(&path).expect("read the file");
    assert!(after.contains("alpha_1 := 1"), "{after}");
    assert!(after.contains("alpha_4 := 4"), "{after}");
    // The other silhouette is untouched: an answer is for one template and
    // not for the rest of the review.
    assert!(after.contains("beta_1 = gamma_1"), "{after}");
    std::fs::remove_dir_all(path.parent().expect("a parent")).expect("remove the directory");
}

#[test]
fn one_capital_t_skips_every_change_of_the_template_and_says_how_many() {
    let path = conf("skip");
    let name = path.to_string_lossy().into_owned();
    let before = std::fs::read_to_string(&path).expect("read the file");
    let out = review(&["rewrite", PATTERN, TEMPLATE, &name, "-i"], "T\n");
    // The count is always reported, whether or not the places were asked for:
    // a reviewer who answered for a template should know what it carried.
    assert!(stderr(&out).contains("4 changes skipped by a template answer"), "{}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&path).expect("read the file"), before, "the file changed");
    std::fs::remove_dir_all(path.parent().expect("a parent")).expect("remove the directory");
}

#[test]
fn the_places_a_skip_passed_over_are_named_only_where_they_were_asked_for() {
    let path = conf("places");
    let name = path.to_string_lossy().into_owned();
    let quiet = review(&["rewrite", PATTERN, TEMPLATE, &name, "-i"], "T\n");
    assert!(!stderr(&quiet).contains("conf.txt ["), "unasked-for places: {}", stderr(&quiet));
    let asked = review(&["rewrite", PATTERN, TEMPLATE, &name, "-i", "--show-skipped"], "T\n");
    let text = stderr(&asked);
    assert!(text.contains("4 changes skipped by a template answer"), "{text}");
    assert_eq!(text.matches("conf.txt [").count(), 4, "{text}");
    std::fs::remove_dir_all(path.parent().expect("a parent")).expect("remove the directory");
}

#[test]
fn t_and_capital_t_are_told_apart_by_their_case() {
    // The two answers mean opposite things, so the case is read from the
    // answer as typed. Folding it would silently accept what a reviewer meant
    // to skip, which is the one mistake this prompt must not make.
    let accepted = conf("case-lower");
    let out = review(&["rewrite", PATTERN, TEMPLATE, &accepted.to_string_lossy(), "-i"], "t\n");
    assert!(stderr(&out).contains("4 replacements"), "{}", stderr(&out));

    let skipped = conf("case-upper");
    let out = review(&["rewrite", PATTERN, TEMPLATE, &skipped.to_string_lossy(), "-i"], "T\n");
    assert!(stderr(&out).contains("skipped by a template answer"), "{}", stderr(&out));
    assert!(!stderr(&out).contains("replacements"), "a skip wrote: {}", stderr(&out));

    for path in [accepted, skipped] {
        std::fs::remove_dir_all(path.parent().expect("a parent")).expect("remove the directory");
    }
}

#[test]
fn explain_puts_the_readings_under_each_diff() {
    let path = conf("explain");
    let name = path.to_string_lossy().into_owned();
    // A pattern that reads an axis, so there is a reading to print beside the
    // kinds and the route.
    let out = review(&["rewrite", "\\M{>0}:v", "N", &name, "-i", "--explain"], "n\n");
    let text = stdout(&out);
    assert!(text.contains("  tokens: "), "{text}");
    assert!(text.contains("  magnitude: "), "{text}");
    assert!(text.contains("  route: "), "{text}");
    // Without it, the diff and the template line stand alone.
    let out = review(&["rewrite", "\\M{>0}:v", "N", &name, "-i"], "n\n");
    assert!(!stdout(&out).contains("  magnitude: "), "{}", stdout(&out));
    std::fs::remove_dir_all(path.parent().expect("a parent")).expect("remove the directory");
}
