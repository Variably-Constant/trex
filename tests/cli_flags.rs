//! Every command answers `-h` and `--help` with its own usage on the standard
//! output and succeeds, refuses a flag it does not take by name, and reads
//! what follows `--` as a path or a value whatever it begins with; every axis
//! listing `--limit` cuts says how much it cut, and one printed in full
//! claims no cut; and every reading with a `--group` takes every orbit group,
//! says so in its help, and refuses the flag with no group after it.

use std::process::{Command, Output};

/// Every command the binary dispatches.
const COMMANDS: [&str; 31] = [
    "scan", "head", "tail", "lines", "rewrite", "redact", "templates", "infer", "count-by", "top", "uniq", "index",
    "tokens", "escape", "lib", "grammar", "bpe", "prefilter", "spectral", "shape", "magnitude", "stress", "echo",
    "relation", "flow", "observe", "gravity", "context", "seam", "orbit", "compress",
];

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

#[test]
fn every_command_answers_help_with_its_own_usage() {
    for command in COMMANDS {
        for flag in ["-h", "--help"] {
            let out = trex(&[command, flag]);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success(), "trex {command} {flag} failed: {}", String::from_utf8_lossy(&out.stderr));
            assert!(
                stdout.starts_with(&format!("usage: trex {command}")),
                "trex {command} {flag} printed no usage of its own: {stdout}"
            );
        }
    }
}

#[test]
fn a_flag_no_command_takes_is_refused_by_name() {
    // Each command is given the arguments it needs, so it refuses the flag
    // and not a missing argument.
    let runs: [&[&str]; 24] = [
        &["scan", "\\W", "--bogus", "--text", "a"],
        &["head", "1", "--bogus"],
        &["tail", "1", "--bogus"],
        &["lines", "1..2", "--bogus"],
        &["templates", "--bogus", "--text", "a"],
        &["infer", "a1", "b2", "--bogus"],
        &["count-by", "\\W", "${0}", "--bogus", "--text", "a"],
        &["index", "--bogus"],
        &["tokens", "--bogus", "--text", "a"],
        &["escape", "--bogus"],
        &["lib", "--bogus"],
        &["prefilter", "--bogus", "--text", "a"],
        &["spectral", "--bogus", "--text", "a"],
        &["shape", "--bogus", "--text", "a"],
        &["magnitude", "--bogus", "--text", "a"],
        &["stress", "--bogus", "--text", "a"],
        &["echo", "--bogus", "--text", "a"],
        &["relation", "--bogus", "--text", "a"],
        &["flow", "--bogus", "--text", "a"],
        &["observe", "--bogus", "--text", "a"],
        &["gravity", "--bogus", "--text", "a"],
        &["context", "--bogus", "--text", "a"],
        &["seam", "--bogus", "--text", "a"],
        &["orbit", "--bogus", "--text", "a"],
    ];
    for args in runs {
        let out = trex(args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?} was accepted");
        assert!(
            stderr.contains(&format!("trex {}: unknown flag --bogus", args[0])),
            "{args:?} did not name the flag it refused: {stderr}"
        );
    }
    // A command whose PATTERN comes first refuses a flag in its place.
    for args in [["redact", "--bogus", "a.txt"], ["rewrite", "--bogus", "x"]] {
        let stderr = String::from_utf8_lossy(&trex(&args).stderr).into_owned();
        assert!(stderr.contains("a PATTERN comes first, not --bogus"), "{args:?}: {stderr}");
    }
}

#[test]
fn what_follows_two_dashes_is_read_whatever_it_begins_with() {
    let dir = std::env::temp_dir().join(format!("trex-dashes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory");
    std::fs::write(dir.join("-notes.txt"), "alpha -beta 12\n").expect("written");
    let run = |args: &[&str]| -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_trex")).args(args).current_dir(&dir).output().expect("run trex");
        assert!(out.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
    };
    assert_eq!(run(&["escape", "--", "-x"]), "\"-\" \"x\"\n");
    assert_eq!(
        run(&["tokens", "--", "-notes.txt"]),
        "[0..5] word \"alpha\"\n[6..7] punct \"-\"\n[7..11] word \"beta\"\n[12..14] number \"12\"\n"
    );
    assert_eq!(run(&["head", "1", "--", "-notes.txt"]), "alpha -beta 12\n");
    assert_eq!(run(&["rewrite", "--", "\\N", "<n>", "-notes.txt"]), "alpha -beta <n>\n");
    assert!(run(&["magnitude", "--", "-notes.txt"]).starts_with("trex magnitude: 15 bytes, 4 tokens"));
    // The clock flags are taken anywhere in the arguments, but not after `--`.
    let out = Command::new(env!("CARGO_BIN_EXE_trex"))
        .args(["scan", "\\W", "--", "--now"])
        .current_dir(&dir)
        .output()
        .expect("run trex");
    assert!(String::from_utf8_lossy(&out.stderr).contains("--now"), "--now after -- is a path");
    std::fs::remove_dir_all(&dir).expect("the temporary directory is removed");
}

#[test]
fn every_listing_a_limit_cuts_says_how_much_it_cut() {
    // More than one of everything a listing holds: tokens, keyed tokens,
    // edges of each kind, reuse chords, enclosed tokens, orbits, segments,
    // spectral frames and supertokens.
    let text = "total = f(a, b) + g(c, d) ; x = 12 ; y = 3400 ;\n".repeat(8);
    let runs: [&[&str]; 17] = [
        &["magnitude", "--field"],
        &["stress", "--field"],
        &["flow", "--field"],
        &["flow", "--analytic"],
        &["echo", "--field"],
        &["relation", "--edges"],
        &["relation", "--field"],
        &["shape", "--classes"],
        &["spectral", "--bands"],
        &["seam", "--field"],
        &["orbit"],
        &["orbit", "--collapse"],
        &["orbit", "--boundary"],
        &["orbit", "--same-as", "x"],
        &["gravity", "--field"],
        &["context", "--field"],
        &["context", "--agreement"],
    ];
    for run in runs {
        let mut args: Vec<&str> = run.to_vec();
        args.extend(["--limit", "1", "--text", &text]);
        let out = trex(&args);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{run:?} failed: {}", String::from_utf8_lossy(&out.stderr));
        assert!(
            stdout.contains(" more ") && stdout.contains("; raise --limit)"),
            "{run:?} cut its listing and said nothing: {stdout}"
        );
    }
}

#[test]
fn a_listing_printed_in_full_claims_no_cut() {
    // 4,600 bytes are 288 spectral frames of 16 bytes, more than 256.
    let text = "alpha beta gamma delta ".repeat(200);
    let out = trex(&["spectral", "--bands", "--text", &text]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout.lines().filter(|l| l.trim_start().starts_with('@')).count(), 288, "{stdout}");
    assert!(!stdout.contains(" more "), "every frame printed, and a cut claimed: {stdout}");
}

#[test]
fn every_group_flag_takes_every_group_and_needs_one() {
    let groups = [
        "identity", "case", "notation", "shape", "e8", "ip", "url", "time", "path", "fold", "numeric", "subnet/24",
        "domain", "day",
    ];
    for command in ["shape", "echo", "orbit"] {
        for group in groups {
            let out = trex(&[command, "--group", group, "--text", "aei bcd 123 10.0.0.1 10.0.0.2"]);
            assert!(
                out.status.success(),
                "trex {command} --group {group} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        // The usage wraps its lines, so its words are read with the wrapping undone.
        let help = String::from_utf8_lossy(&trex(&[command, "--help"]).stdout).into_owned();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            help.contains("e8,") && help.contains("typed relation such as subnet/24"),
            "trex {command} --help: {help}"
        );
        let out = trex(&[command, "--text", "a b a", "--group"]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "trex {command} took --group with no group after it");
        assert!(stderr.contains("--group needs a"), "trex {command} did not name what it lacked: {stderr}");
    }
}

#[test]
fn a_count_written_as_coreutils_writes_it_is_still_a_count() {
    let out = trex(&["head", "-1", "--", "-"]);
    // `-` is the standard input, which holds nothing here.
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}
