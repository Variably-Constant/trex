//! End-to-end capability tests: drive the real `trex` binary and
//! assert it matches the constructs no regular expression can,
//! plus their negative controls.
//!
//! These run the compiled binary (via `CARGO_BIN_EXE_trex`), so a
//! pass means the user-facing action produced the expected effect,
//! not merely that a library function returned a value.

use std::process::Command;

/// Run `trex scan PATTERN --text TEXT` and return `(stdout,
/// matched)` where `matched` is true when the binary reported at
/// least one match.
fn scan(pattern: &str, text: &str) -> (String, bool) {
    let exe = env!("CARGO_BIN_EXE_trex");
    let out = Command::new(exe)
        .args(["scan", pattern, "--text", text])
        .output()
        .expect("failed to run trex binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let matched = !stdout.contains("no match");
    (stdout, matched)
}

#[test]
fn matched_tag_pair_binds_and_rejects_mismatch() {
    let (ok, matched) = scan("<\\W:t>.*</=t>", "<div>hi</div>");
    assert!(matched, "balanced tag should match, got: {ok}");
    assert!(ok.contains("<div>hi</div>"));
    assert!(ok.contains("t=\"div\""));

    let (_, matched) = scan("<\\W:t>.*</=t>", "<div>hi</span>");
    assert!(!matched, "mismatched tag must not match");
}

#[test]
fn repeated_token_matches_only_a_repeat() {
    let (ok, matched) = scan("\\W:x =x", "the the cat");
    assert!(matched);
    assert!(ok.contains("the the"));

    let (_, matched) = scan("\\W:x =x", "the cat sat");
    assert!(!matched, "non-repeat must not match");
}

#[test]
fn balanced_paren_group_handles_nesting() {
    let (ok, matched) = scan("\\W\\B(.*)", "call f(g(x)) end");
    assert!(matched);
    assert!(ok.contains("f(g(x))"), "expected the full nested call, got: {ok}");

    let (_, bare) = scan("\\W\\B(.*)", "just a word");
    assert!(!bare, "a word with no following group must not match");

    // A word followed by an unclosed bracket has no paired group
    // (the open's mate is None), so it must not match. Note that
    // "f(g(x)" would match the embedded valid group "g(x)", which
    // is correct scanner behavior; the control here is an input
    // with no closable group at all.
    let (_, unbalanced) = scan("\\W\\B(.*)", "f(");
    assert!(!unbalanced, "a word followed by an unclosed group must not match");
}

#[test]
fn balanced_brace_group_handles_nesting() {
    let (ok, matched) = scan("\\B{.*}", "cfg {a {b} c} done");
    assert!(matched);
    assert!(ok.contains("{a {b} c}"), "expected the full nested brace span, got: {ok}");
}

#[test]
fn token_mode_skips_whitespace() {
    // A unit symbol one space after a number belongs to that quantity, so the
    // word this reads across the whitespace is in no unit table.
    let (ok, matched) = scan("\\N \\W", "shipping weight 12 crates total");
    assert!(matched);
    assert!(ok.contains("12 crates"));
}

#[test]
fn byte_grain_and_token_grain_compose() {
    // One pattern, both grains: the tag name is constrained at the
    // byte grain (lowercase letters), the open/close match at the
    // token grain. This does what a regex does and what it cannot.
    let pat = "<`[a-z]+`:t>.*</=t>";
    let (ok, matched) = scan(pat, "<div>hi</div>");
    assert!(matched, "lowercase matched tag should match, got: {ok}");
    assert!(ok.contains("t=\"div\""));

    // An uppercase tag name fails the byte grain.
    let (_, upper) = scan(pat, "<Div>hi</Div>");
    assert!(!upper, "uppercase tag name must fail the byte grain");

    // A mismatched close fails the token grain.
    let (_, mismatch) = scan(pat, "<div>hi</span>");
    assert!(!mismatch, "mismatched close must fail the token grain");

    // The byte grain alone selects TitleCase words.
    let (tc, _) = scan("`[A-Z][a-z]+`", "Hello world Bar");
    assert!(tc.contains("Hello") && tc.contains("Bar"));
    assert!(!tc.contains("\"world\""), "lowercase word must not match TitleCase");
}
