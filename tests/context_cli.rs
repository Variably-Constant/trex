//! `trex context`: each token read against the fold a relative predicate
//! names, the record period and the grains' agreement, and a window that
//! folds nothing refused.

use std::process::{Command, Output};

const LINE: &str = "latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;";

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(args: &[&str]) -> String {
    let out = trex(args);
    assert!(out.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

#[test]
fn the_summary_names_the_fold_the_period_and_the_alignment() {
    let out = stdout(&["context", "--text", LINE]);
    assert!(out.starts_with("trex context (window fold): 81 bytes, 20 tokens, 5 supertokens\n"), "{out}");
    assert!(out.contains("  record period: 4 tokens (live periods 4, 8)\n"), "{out}");
    assert!(out.contains("  alignment: regime "), "{out}");
}

#[test]
fn the_key_fold_holds_the_values_a_key_predicate_compares_with() {
    let out = stdout(&["context", "--fold", "key", "--field", "--text", LINE]);
    assert!(out.contains("  per-token context, the values each one's key was bound to before:\n"), "{out}");
    assert!(out.contains("    @    64  latency        over 3\n        magnitude: mean 2.01, spread 0.05, max 2.08\n"), "{out}");
    assert!(out.contains("    @     0  latency        over 0\n    @     8  ="), "a fold holding nothing reads no axis: {out}");
}

#[test]
fn the_period_and_agreement_listings_read_each_unit() {
    let out = stdout(&["context", "--period", "--agreement", "--limit", "2", "--text", LINE]);
    assert!(out.contains("  per-token column of the record period:\n    @     0  latency          0\n    @     8  =                1\n"), "{out}");
    assert!(out.contains("    ... (+18 more tokens; raise --limit)\n"), "{out}");
    assert!(out.contains("    @     0  latency = 100  assign "), "{out}");
}

#[test]
fn an_unknown_fold_or_an_empty_window_is_refused() {
    let out = trex(&["context", "--fold", "windows", "--text", LINE]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown fold 'windows'"));
    let out = trex(&["context", "--token-window", "0", "--text", LINE]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("a window folds at least one unit"));
}
