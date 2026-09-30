//! A flag's value that does not read as what the flag takes fails the command,
//! naming the flag and what it was given, rather than running it with a default
//! the caller never typed: `--limit 1O` scanning with no limit at all is the
//! failure this holds the commands to.

use std::process::{Command, Output};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

#[test]
fn an_unreadable_flag_value_fails_the_command_and_names_the_flag() {
    let every_build: &[(&[&str], &str)] = &[
        (&["flow", "--text", "a b c", "--limit", "1O"], "--limit"),
        (&["echo", "--text", "a b c", "--top", "many"], "--top"),
        (&["magnitude", "--text", "a 1 b 2", "--top", "-3"], "--top"),
        (&["spectral", "--text", "a b c", "--limit"], "--limit"),
    ];
    // The coder's flags, in the build that has the coder.
    let coder: &[(&[&str], &str)] = if cfg!(feature = "compress") {
        &[
            (&["compress", "--text", "a b c", "--chunks", "x"], "--chunks"),
            (&["compress", "--text", "a b c", "--max-bytes", "1O"], "--max-bytes"),
        ]
    } else {
        &[]
    };
    for &(args, flag) in every_build.iter().chain(coder) {
        let out = trex(args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?} succeeded with an unreadable {flag}");
        assert!(stderr.contains(flag), "{args:?}: the error does not name {flag}: {stderr}");
    }
}

#[test]
fn a_readable_flag_value_still_runs() {
    let every_build: &[&[&str]] = &[&["flow", "--text", "a b c a b c", "--limit", "1"]];
    let coder: &[&[&str]] =
        if cfg!(feature = "compress") { &[&["compress", "--text", "a b c a b c", "--chunks", "1"]] } else { &[] };
    for &args in every_build.iter().chain(coder) {
        let out = trex(args);
        assert!(out.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
}

/// `--second-mixer` codes with the second mixer and names it in the coder it
/// reports, over one chunk and over several. Beside a device coder, which has
/// no second mixer, it fails the command naming itself, rather than coding
/// without the mixer the caller asked for.
#[cfg(feature = "compress")]
#[test]
fn the_second_mixer_is_the_coder_reported_or_the_command_fails() {
    let text = "let a = 1 ; let b = 2 ; call(a, b) ;\n".repeat(64);
    for extra in [&[][..], &["--chunks", "2"][..]] {
        let args: Vec<&str> =
            ["compress", "--text", text.as_str(), "--second-mixer"].into_iter().chain(extra.iter().copied()).collect();
        let out = trex(&args);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{extra:?}: {}", String::from_utf8_lossy(&out.stderr));
        assert!(stdout.contains("a second mixer by rhythm"), "{extra:?}: the coder reported is not the second mixer: {stdout}");
    }
    for device in ["--gpu", "--hybrid"] {
        let out = trex(&["compress", "--text", text.as_str(), "--second-mixer", device]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "--second-mixer {device} succeeded");
        assert!(stderr.contains("--second-mixer"), "{device}: the error does not name --second-mixer: {stderr}");
    }
}
