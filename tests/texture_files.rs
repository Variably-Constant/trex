//! `--texture`: the walk keeps a file by what the spectral and shape axes
//! read it as, and `--files --texture` names that reading.
//!
//! These assert the mechanism rather than the axes' verdicts. What a
//! particular file reads as is the shape axis's judgment and has its own
//! tests; pinning "a markdown file is prose" here would make this suite fail
//! whenever that judgment is tuned, which is the wrong thing to guard. So the
//! filter is checked against whatever the listing just reported, and only the
//! one verdict that cannot drift - a file of base64 is a blob - is named.

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

/// A tree holding a file of each rough sort, under a directory of this test's
/// own: these run in parallel in one process, so a shared directory is one
/// the first to finish removes while the rest are reading it.
fn tree(named: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("trex/texture-{}-{named}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the tree");

    // Irregular text: several message shapes interleaved, as a log is.
    let mut log = String::new();
    for i in 0..400 {
        let line = match i % 4 {
            0 => format!("2026-09-16T{:02}:{:02}:00Z GET /v2/users from 10.0.{}.{} took {}ms\n", i % 24, i % 60, i % 250, (i * 7) % 250, (i * 13) % 900),
            1 => format!("2026-09-16T{:02}:{:02}:00Z warning: retrying the primary after {} failures\n", i % 24, i % 60, i % 9),
            2 => format!("2026-09-16T{:02}:{:02}:00Z worker {} finished a batch of {} rows\n", i % 24, i % 60, i % 16, i * 31),
            _ => format!("2026-09-16T{:02}:{:02}:00Z ERROR expected a value for key field_{} and the record ended\n", i % 24, i % 60, i),
        };
        log.push_str(&line);
    }
    std::fs::write(dir.join("app.log"), log).expect("write app.log");

    // A blob: one long run of base64, which no texture but `blob` describes.
    let alphabet: Vec<u8> = (b'A'..=b'Z').chain(b'a'..=b'z').chain(b'0'..=b'9').collect();
    let mut blob = String::new();
    let mut state = 7u64;
    for _ in 0..12000 {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        blob.push(alphabet[(state >> 33) as usize % alphabet.len()] as char);
    }
    std::fs::write(dir.join("payload.b64"), blob).expect("write payload.b64");

    // Fixed-shape rows.
    let mut csv = String::from("id,host,port,bytes,ms\n");
    for i in 1..400 {
        csv.push_str(&format!("{i},host{},{},{},{}\n", i % 40, 8000 + (i % 90), i * 977, i % 400));
    }
    std::fs::write(dir.join("rows.csv"), csv).expect("write rows.csv");
    dir
}

/// The `path: kind` lines `--files --texture` prints, as pairs.
fn listed(dir: &str) -> Vec<(String, String)> {
    let out = trex(&["scan", "--files", "--texture", dir, "--color", "never"]);
    assert!(out.status.success(), "{}", stderr(&out));
    stdout(&out)
        .lines()
        .filter_map(|l| l.rsplit_once(": "))
        .map(|(path, kind)| (path.to_string(), kind.to_string()))
        .collect()
}

/// The files `--files` lists under the given texture flags.
fn kept(dir: &str, flags: &[&str]) -> Vec<String> {
    let mut args = vec!["scan", "--files"];
    args.extend_from_slice(flags);
    args.extend_from_slice(&[dir, "--color", "never"]);
    let out = trex(&args);
    assert!(out.status.success(), "{args:?}: {}", stderr(&out));
    let mut names: Vec<String> = stdout(&out).lines().map(str::to_string).collect();
    names.sort();
    names
}

#[test]
fn the_listing_names_every_file_and_names_it_one_of_the_kinds() {
    let dir = tree("listing");
    let rows = listed(&dir.to_string_lossy());
    assert_eq!(rows.len(), 3, "{rows:?}");
    for (path, kind) in &rows {
        // A table reports the period of the table it found beside its name,
        // since the period belongs to the region rather than to the kind.
        let head = kind.split(',').next().expect("a kind");
        assert!(
            ["table", "blob", "prose", "numeric", "code", "mixed"].contains(&head),
            "{path} reads as {kind:?}, which is no kind"
        );
        if head == "table" {
            assert!(kind.contains(", period "), "a table with no period: {kind:?}");
        }
    }
    std::fs::remove_dir_all(&dir).expect("remove the tree");
}

#[test]
fn a_file_of_base64_reads_as_a_blob() {
    // The one verdict named here. Every other kind is a judgment the shape
    // axis can tune; a long run of base64 has no other texture to be.
    let dir = tree("blob");
    let rows = listed(&dir.to_string_lossy());
    let blob = rows.iter().find(|(p, _)| p.ends_with("payload.b64")).expect("the blob listed");
    assert_eq!(blob.1, "blob", "{rows:?}");
    std::fs::remove_dir_all(&dir).expect("remove the tree");
}

#[test]
fn keeping_a_kind_keeps_the_files_the_listing_named_that_kind() {
    let dir = tree("keep");
    let name = dir.to_string_lossy().into_owned();
    let rows = listed(&name);
    // Asked of whatever the listing just said, so this holds however the
    // axes read these files.
    let blob_kind = &rows.iter().find(|(p, _)| p.ends_with("payload.b64")).expect("the blob").1;
    let head = blob_kind.split(',').next().expect("a kind").to_string();
    let expected: Vec<String> = {
        let mut names: Vec<String> = rows
            .iter()
            .filter(|(_, k)| k.split(',').next() == Some(head.as_str()))
            .map(|(p, _)| p.clone())
            .collect();
        names.sort();
        names
    };
    assert_eq!(kept(&name, &["--texture", &head]), expected);
    std::fs::remove_dir_all(&dir).expect("remove the tree");
}

#[test]
fn dropping_a_kind_leaves_every_other_file() {
    let dir = tree("drop");
    let name = dir.to_string_lossy().into_owned();
    let rows = listed(&name);
    let mut all: Vec<String> = rows.iter().map(|(p, _)| p.clone()).collect();
    all.sort();
    let dropped = format!("!{}", "blob");
    let left = kept(&name, &["--texture", &dropped]);
    assert!(!left.iter().any(|p| p.ends_with("payload.b64")), "the blob survived: {left:?}");
    assert_eq!(left.len(), all.len() - 1, "{left:?} against {all:?}");
    std::fs::remove_dir_all(&dir).expect("remove the tree");
}

#[test]
fn a_file_named_on_the_command_line_is_read_whatever_it_reads_as() {
    // The filter is over what the walk turned up. A file the reader named is
    // a file they asked for, and a texture filter is not an argument about
    // that: `.` matches any token, so a file that is read at all reports.
    let dir = tree("named");
    let blob = dir.join("payload.b64");
    let out = trex(&[
        "scan",
        ".",
        &blob.to_string_lossy(),
        "-l",
        "--texture",
        "code",
        "--color",
        "never",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("payload.b64"), "{}", stdout(&out));
    std::fs::remove_dir_all(&dir).expect("remove the tree");
}

#[test]
fn the_filter_narrows_a_scan_and_not_only_a_listing() {
    let dir = tree("scan");
    let name = dir.to_string_lossy().into_owned();
    let all = trex(&["scan", ".", &name, "-l", "--color", "never"]);
    let some = trex(&["scan", ".", &name, "-l", "--texture", "blob", "--color", "never"]);
    assert!(all.status.success() && some.status.success());
    assert_eq!(stdout(&all).lines().count(), 3, "{}", stdout(&all));
    assert_eq!(stdout(&some).lines().count(), 1, "{}", stdout(&some));
    assert!(stdout(&some).contains("payload.b64"), "{}", stdout(&some));
    std::fs::remove_dir_all(&dir).expect("remove the tree");
}

#[test]
fn a_word_that_names_no_kind_is_refused_rather_than_taken_for_a_path() {
    let dir = tree("refused");
    let out = trex(&["scan", "--files", "--texture", "nonsense", &dir.to_string_lossy()]);
    assert!(!out.status.success(), "{}", stdout(&out));
    let e = stderr(&out);
    assert!(e.contains("--texture takes table, blob, prose, numeric, code, mixed"), "{e}");
    assert!(e.contains("not \"nonsense\""), "{e}");
    std::fs::remove_dir_all(&dir).expect("remove the tree");
}
