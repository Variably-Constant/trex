//! `(A B C)~k`: a run of tokens within `k` token edits of the group's atoms -
//! a token the run lacks, a token it has extra, or a token that matches no
//! atom in its place. The token-grain counterpart of `"lit"~k`, which counts
//! characters inside one token.

use std::process::{Command, Output};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

/// Scan `text` for `pattern` and return what the report printed.
fn scan(pattern: &str, text: &str) -> String {
    let out = trex(&["scan", pattern, "--text", text]);
    assert!(out.status.success(), "{pattern} over {text}: {}", stderr(&out));
    stdout(&out)
}

const PHRASE: &str = r#"("user" "bob" "logged" "in")~1"#;

#[test]
fn one_edit_is_a_token_missing_a_token_extra_or_a_token_that_does_not_match() {
    assert_eq!(scan(PHRASE, "user bob logged in"), "[0..18] \"user bob logged in\"\n");
    assert_eq!(scan(PHRASE, "user bob in"), "[0..11] \"user bob in\"\n", "a token the run lacks");
    assert_eq!(
        scan(PHRASE, "user bob has logged in"),
        "[0..22] \"user bob has logged in\"\n",
        "a token the run has extra"
    );
    assert_eq!(
        scan(PHRASE, "user amy logged in"),
        "[0..18] \"user amy logged in\"\n",
        "a token that matches no atom in its place"
    );
    // Two edits are past the count, and a transposition is two, as it is in
    // the character form.
    assert_eq!(scan(PHRASE, "user amy has logged in"), "no match\n");
    assert_eq!(scan(PHRASE, "user logged bob in"), "no match\n", "a transposition costs two");
    assert_eq!(
        scan(r#"("user" "bob" "logged" "in")~2"#, "user logged bob in"),
        "[0..18] \"user logged bob in\"\n",
        "and two is what it takes"
    );
}

#[test]
fn a_count_of_zero_is_the_exact_sequence() {
    let exact = r#"("user" "bob" "logged" "in")~0"#;
    assert_eq!(scan(exact, "user bob logged in"), "[0..18] \"user bob logged in\"\n");
    assert_eq!(scan(exact, "user bob in"), "no match\n");
}

#[test]
fn every_atom_matches_its_token_as_it_would_alone() {
    // Kinds, classes and predicates, not only literals.
    assert_eq!(scan(r"(\W \N \W)~1", "alpha 42 beta"), "[0..13] \"alpha 42 beta\"\n");
    assert_eq!(scan(r"(\W \N \W)~1", "alpha beta"), "[0..10] \"alpha beta\"\n");
    assert_eq!(scan(r#"([\N \Q] "=" \W)~1"#, "42 = alpha"), "[0..10] \"42 = alpha\"\n");
    assert_eq!(scan(r"(\W \N{>100} \W)~0", "alpha 4200 beta"), "[0..15] \"alpha 4200 beta\"\n");
    assert_eq!(scan(r"(\W \N{>100} \W)~0", "alpha 42 beta"), "no match\n");
}

#[test]
fn a_binding_takes_the_token_its_atom_aligned_with() {
    let out = trex(&["scan", r#"("user" \W:who "logged" "in")~1"#, "--text", "user bob logged in"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "[0..18] \"user bob logged in\"  captures: who=\"bob\"\n"
    );
    // The register is readable in a template, as any other is.
    let out = trex(&[
        "rewrite",
        r#"("user" \W:who "logged" "in")~1"#,
        "${who} signed in",
        "--text",
        "user bob logged in",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "bob signed in");
}

#[test]
fn every_run_within_the_count_is_offered_and_what_follows_chooses() {
    // Three tokens at no edits and four at one are both within the count, so
    // the atom after the group decides which alignment survives.
    assert_eq!(scan(r#"("a" "b" "c")~1 "d""#, "a b c d"), "[0..7] \"a b c d\"\n");
    assert_eq!(scan(r#"("a" "b" "c")~1 "e""#, "a b c d e"), "[0..9] \"a b c d e\"\n");
    // With nothing after it the cheaper alignment wins, which is the exact
    // three-token run rather than the four-token one that spent an edit.
    assert_eq!(scan(r#"("a" "b" "c")~1"#, "a b c d"), "[0..5] \"a b c\"\n");
}

#[test]
fn the_group_holds_single_token_atoms_and_a_count_under_their_number() {
    for (pattern, said) in [
        (r#"(\W "b"* \W)~1"#, "holds single-token atoms"),
        (r#"(\W (\N|\Q) \W)~1"#, "holds single-token atoms"),
        (r"(\W \B(\N) \W)~1", "holds single-token atoms"),
        (r"(\W @seam \W)~1", "holds single-token atoms"),
        (r"(\W \N \W)~3", "the count stands under the number of atoms"),
        (r"(\W)~1", "the count stands under the number of atoms"),
    ] {
        let out = trex(&["scan", pattern, "--text", "x"]);
        assert!(!out.status.success(), "{pattern}");
        assert!(stderr(&out).contains(said), "{pattern}: {}", stderr(&out));
    }
    // A `~` before anything but a digit still opens the assertion that
    // follows the group, as it does after a literal.
    assert_eq!(scan(r#"(\W \W) ~"end""#, "alpha beta end"), "[0..10] \"alpha beta\"\n");
}

#[test]
fn the_group_never_matches_an_empty_run() {
    // Even where the count would let every atom be deleted, the run holds a
    // token: a zero-width match at every position is no pattern at all. The
    // count is refused before that, and this is the case just under it.
    assert_eq!(scan(r"(\N \N \N)~2", "alpha"), "no match\n");
    assert_eq!(scan(r"(\N \N \N)~2", "42"), "[0..2] \"42\"\n");
}
