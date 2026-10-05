//! The redact command, driven through the compiled binary: every match of a
//! typed pattern masked, the fields `--keep` names left unmasked,
//! and the guarded kinds deciding what is a card at all.

use std::path::PathBuf;
use std::process::{Command, Output};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A card that passes the Luhn check, an address, an email, and a run of
/// digits shaped like a card that fails the check.
const DOC: &str =
    "card 4111 1111 1111 1111 from 10.1.2.3 by bob@corp.example, ref 1234 5678 1234 5678";

const PATTERN: &str = "(\\{card}:card | \\I:ip | \\E:email)";

#[test]
fn kept_fields_stay_where_they_were_and_everything_else_is_masked() {
    let out = trex(&[
        "redact",
        PATTERN,
        "--keep",
        "card:last4, ip:octet1-2, email:domain",
        "--text",
        DOC,
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    // Byte for byte: one `*` per masked character, so the line is as long
    // as it was, and the digits that fail the Luhn check are not a card.
    assert_eq!(
        stdout(&out),
        "card ***************1111 from 10.1**** by ****corp.example, ref 1234 5678 1234 5678"
    );
    assert_eq!(stdout(&out).len(), DOC.len());
}

#[test]
fn without_keep_the_whole_match_is_masked() {
    let out = trex(&["redact", "\\E", "--text", "mail bob@x.com now"]);
    assert_eq!(stdout(&out), "mail ********* now");
    // A field the accessor finds nothing for keeps nothing: no URL here
    // carries a port.
    let out = trex(&["redact", "\\U:u", "--keep", "u:port", "--text", "see http://x.example/p"]);
    assert_eq!(stdout(&out), "see ******************");
}

#[test]
fn the_mask_is_a_character_per_character_or_a_token_per_run() {
    let text = "paid with 4111 1111 1111 1111 today";
    let out = trex(&["redact", "\\{card}:c", "--keep", "c:last4", "--mask", "#", "--text", text]);
    assert_eq!(stdout(&out), "paid with ###############1111 today");
    let out = trex(&["redact", "\\{card}:c", "--keep", "c:last4", "--mask", "[card]", "--text", text]);
    assert_eq!(stdout(&out), "paid with [card]1111 today");
    // A kept field in the middle of the span leaves a masked run on each
    // side, and a token stands in for each run.
    let out = trex(&["redact", "\\I:ip", "--keep", "ip:octet2-3", "--mask", "..", "--text", "at 10.1.2.3 x"]);
    assert_eq!(stdout(&out), "at ..1.2.. x");
    // A multi-byte mask character and multi-byte text: one character per
    // character, counted in characters rather than bytes.
    let out = trex(&["redact", "\\E", "--mask", "\u{2588}", "--text", "mail bob@x.com now"]);
    assert_eq!(stdout(&out), "mail \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} now");
    let out = trex(&["redact", "\\W", "--text", "caf\u{e9} au lait"]);
    assert_eq!(stdout(&out), "**** ** ****");
}

#[test]
fn first_and_last_slice_any_capture_in_a_template_too() {
    let out = trex(&["rewrite", "\\{card}:c", "${c:last4}", "--text", "4111 1111 1111 1111"]);
    assert_eq!(stdout(&out), "1111");
    let out = trex(&["rewrite", "\\W:w", "<${w:first2}>", "--text", "hello"]);
    assert_eq!(stdout(&out), "<he>");
    // A count past the end is the whole value, and a chain slices the slice.
    let out = trex(&["rewrite", "\\W:w", "${w:last9}", "--text", "hello"]);
    assert_eq!(stdout(&out), "hello");
    let out = trex(&["rewrite", "\\U:u", "${u:host|first3}", "--text", "http://example.test/p"]);
    assert_eq!(stdout(&out), "exa");
}

#[test]
fn a_keep_that_names_nothing_to_keep_in_place_is_refused() {
    let out = trex(&["redact", "\\E:e", "--keep", "e:upper", "--text", "bob@x.com"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("transforms the text"), "{}", stderr(&out));
    let out = trex(&["redact", "\\E:e", "--keep", "nosuch:domain", "--text", "bob@x.com"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no such capture"), "{}", stderr(&out));
    let out = trex(&["redact", "\\E:e", "--keep", "e:last0", "--text", "bob@x.com"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("last0 names no characters"), "{}", stderr(&out));
    let out = trex(&["redact", "\\E:e", "--mask", "", "--text", "bob@x.com"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("mask is empty"), "{}", stderr(&out));
}

#[test]
fn files_are_redacted_in_place_or_shown_as_a_diff() {
    let dir = std::env::temp_dir().join(format!("trex/redact-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("orders.log");
    std::fs::write(&path, format!("{DOC}\nno secrets here\n")).expect("write file");
    let name = path.to_string_lossy().into_owned();
    let out = trex(&["redact", PATTERN, "--keep", "card:last4", &name, "--dry-run"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let diff = stdout(&out);
    assert!(diff.contains("-card 4111 1111 1111 1111 from 10.1.2.3 by bob@corp.example"), "{diff}");
    assert!(
        diff.contains("+card ***************1111 from ******** by ****************"),
        "{diff}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("read file"),
        format!("{DOC}\nno secrets here\n"),
        "a dry run writes nothing"
    );
    let out = trex(&["redact", PATTERN, "--keep", "card:last4", &name, "--in-place"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(&path).expect("read file"),
        "card ***************1111 from ******** by ****************, ref 1234 5678 1234 5678\nno secrets here\n"
    );
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}
