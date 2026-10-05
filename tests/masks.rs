//! `redact --mask shape` and `--mask pseudonym`: the two masks that read the
//! run they cover rather than writing over it blindly.
//!
//! Each is asserted by the property it exists for, not only by its output. A
//! shape mask is asserted by re-scanning the redacted copy with the same
//! pattern, and a pseudonym by an echoed scan finding the recurrence the
//! original carried.

use std::path::PathBuf;
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

/// One document holding a value that recurs, under a directory of this test's
/// own: these run in parallel, so a shared one is removed under the rest.
fn doc(named: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("trex/masks-{}-{named}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the directory");
    let path = dir.join("a.log");
    std::fs::write(
        &path,
        "from 10.4.5.6 user bob@x.com id 550e8400-e29b-41d4-a716-446655440000\n\
         from 10.9.9.9 user amy@y.org id 6ba7b810-9dad-11d1-80b4-00c04fd430c8\n\
         from 10.4.5.6 user bob@x.com again\n",
    )
    .expect("write the document");
    path
}

/// The redacted copy of `path` under `mask`.
fn redacted(pattern: &str, mask: &str, path: &str) -> String {
    let out = trex(&["redact", pattern, "--mask", mask, path]);
    assert!(out.status.success(), "{pattern} under {mask}: {}", stderr(&out));
    stdout(&out)
}

#[test]
fn a_shape_mask_keeps_every_byte_that_is_not_a_letter_or_a_digit() {
    let path = doc("shape");
    let name = path.to_string_lossy().into_owned();
    // The separators stay where they were, so the run keeps the shape its
    // kind is recognized by.
    let masked = redacted("\\I", "shape", &name);
    assert!(masked.starts_with("from 00.0.0.0 user bob@x.com"), "{masked:?}");
    assert!(masked.contains("from 00.0.0.0 user amy@y.org"), "{masked:?}");
    // A letter masks to `a`, which is a letter and a hex digit at once, so
    // one rule covers the alphabetic kinds and the hex-shaped kinds together.
    // Masking to `x` would keep this email and break the uuid below.
    let masked = redacted("\\E", "shape", &name);
    assert!(masked.contains("user aaa@a.aaa id"), "{masked:?}");
    let masked = redacted("\\{uuid}", "shape", &name);
    assert!(masked.contains("id 000a0000-a00a-00a0-a000-000000000000"), "{masked:?}");
    std::fs::remove_dir_all(path.parent().expect("a parent")).expect("remove the directory");
}

#[test]
fn the_shaped_copy_still_lexes_as_the_kind_it_replaced() {
    // The property the mask exists for, asserted by re-scanning the redacted
    // copy with the same pattern: a mask that kept the shape but not the kind
    // would pass the assertions above and fail here.
    let path = doc("relex");
    let dir = path.parent().expect("a parent").to_path_buf();
    for (pattern, count) in [("\\I", 3), ("\\E", 3), ("\\{uuid}", 2)] {
        let masked = redacted(pattern, "shape", &path.to_string_lossy());
        let copy = dir.join("masked.log");
        std::fs::write(&copy, &masked).expect("write the masked copy");
        let out = trex(&["scan", pattern, &copy.to_string_lossy(), "--color", "never"]);
        assert!(out.status.success(), "{pattern}: {}", stderr(&out));
        let found = stdout(&out).lines().count();
        assert_eq!(found, count, "{pattern} reads {found} of its own masked tokens: {}", stdout(&out));
    }
    std::fs::remove_dir_all(&dir).expect("remove the directory");
}

#[test]
fn a_pseudonym_gives_one_value_one_name_everywhere() {
    let path = doc("pseudonym");
    let name = path.to_string_lossy().into_owned();
    // Numbered in order of first sight, per kind, and the same value gives
    // the same name on every line it is on.
    let masked = redacted("\\I", "pseudonym", &name);
    assert_eq!(
        masked,
        "from IP_1 user bob@x.com id 550e8400-e29b-41d4-a716-446655440000\n\
         from IP_2 user amy@y.org id 6ba7b810-9dad-11d1-80b4-00c04fd430c8\n\
         from IP_1 user bob@x.com again\n"
    );
    // The kind names the pseudonym, so an email's name opens with `EMAIL`
    // rather than `IP`, and the numbering starts again for each kind.
    let masked = redacted("\\E", "pseudonym", &name);
    assert!(masked.contains("user EMAIL_1 id"), "{masked:?}");
    assert!(masked.contains("user EMAIL_2 id"), "{masked:?}");
    assert!(masked.ends_with("user EMAIL_1 again\n"), "{masked:?}");
    std::fs::remove_dir_all(path.parent().expect("a parent")).expect("remove the directory");
}

#[test]
fn a_pseudonym_names_a_declared_kind_and_keeps_a_value_s_name_across_a_tree() {
    // A declared kind names its pseudonym as its declaration does, and a tree
    // is redacted one file after another in walk order, the book carried from
    // each file into the next, so a value keeps its name in every file.
    let dir = std::env::temp_dir().join(format!("trex/masks-{}-declared", std::process::id()));
    let tree = dir.join("logs");
    std::fs::create_dir_all(&tree).expect("create the tree");
    let lib = dir.join("ops.trex");
    std::fs::write(&lib, "shape customer = `C\\d{5}`\n").expect("write the declaration");
    std::fs::write(tree.join("a.log"), "opened for C00042\nopened for C00077\n").expect("write a.log");
    std::fs::write(tree.join("b.log"), "closed for C00077\nclosed for C00042\n").expect("write b.log");
    let (lib, root) = (lib.to_string_lossy().into_owned(), tree.to_string_lossy().into_owned());
    let out = trex(&["redact", "\\{customer}", "--mask", "pseudonym", "--lib", &lib, &root, "--in-place"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let read = |name: &str| std::fs::read_to_string(tree.join(name)).expect("read back").replace("\r\n", "\n");
    assert_eq!(read("a.log"), "opened for CUSTOMER_1\nopened for CUSTOMER_2\n");
    assert_eq!(read("b.log"), "closed for CUSTOMER_2\nclosed for CUSTOMER_1\n");
    std::fs::remove_dir_all(&dir).expect("remove the directory");
}

#[test]
fn the_recurrence_a_value_carried_survives_its_pseudonym() {
    // The property the mask exists for. `10.4.5.6` appears twice, so an echoed
    // scan finds it twice; the pseudonym must appear twice in the same places,
    // which it can only do by lexing as a single token - a name that became
    // several would scatter the recurrence across them.
    let path = doc("echo");
    let dir = path.parent().expect("a parent").to_path_buf();
    let name = path.to_string_lossy().into_owned();
    let before = trex(&["scan", "@echo>1 \\I", &name, "--color", "never"]);
    assert!(before.status.success(), "{}", stderr(&before));
    assert_eq!(stdout(&before).lines().count(), 2, "{}", stdout(&before));

    let masked = redacted("\\I", "pseudonym", &name);
    let copy = dir.join("named.log");
    std::fs::write(&copy, &masked).expect("write the masked copy");
    let after = trex(&["scan", "@echo>1 \\W", &copy.to_string_lossy(), "--color", "never"]);
    assert!(after.status.success(), "{}", stderr(&after));
    let text = stdout(&after);
    let named = text.lines().filter(|l| l.contains("IP_1")).count();
    assert_eq!(named, 2, "the pseudonym did not recur: {text}");
    std::fs::remove_dir_all(&dir).expect("remove the directory");
}

#[test]
fn the_masks_that_were_there_before_are_unchanged() {
    // `shape` and `pseudonym` are words a token mask could have been, so the
    // two readings have to be told apart: any other multi-character mask is
    // still the token each run becomes, and a single character still masks
    // per character.
    let path = doc("old");
    let name = path.to_string_lossy().into_owned();
    let masked = redacted("\\I", "*", &name);
    assert!(masked.starts_with("from ******** user"), "{masked:?}");
    let masked = redacted("\\I", "REDACTED", &name);
    assert!(masked.starts_with("from REDACTED user"), "{masked:?}");
    std::fs::remove_dir_all(path.parent().expect("a parent")).expect("remove the directory");
}
