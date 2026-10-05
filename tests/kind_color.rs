//! Color by token kind: every token of a painted kind in a printed line
//! carries its kind's style, at the depth the console renders.
//!
//! The depth is forced with `--color`, so nothing here depends on the
//! terminal the suite happens to run under. Every expected escape is what the
//! built binary emitted.

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

/// A log holding one token of several kinds, written where this test can
/// reach it.
fn fixture() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("trex/kindcolor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the fixture directory");
    let path = dir.join("app.log");
    std::fs::write(
        &path,
        "2026-09-16T12:04:00Z GET https://api.internal/v2 from 10.0.1.4 took 4.2ms 1.5KiB\n\
         card 4111 1111 1111 1111 seen in \"user record\"\n\
         plain words with no values at all\n",
    )
    .expect("write the fixture");
    path
}

/// Every line a passthru prints at `depth`, with the given extra flags. The
/// pattern matches nothing, so what comes back is the kinds alone.
fn painted(depth: &str, extra: &[&str]) -> String {
    let path = fixture();
    let file = path.to_string_lossy().into_owned();
    let mut args = vec!["scan", "\\N{9999..9999}", &file, "--passthru", "--color", depth];
    args.extend_from_slice(extra);
    let out = trex(&args);
    assert!(out.status.success(), "{args:?}: {}", stderr(&out));
    stdout(&out)
}

#[test]
fn a_kind_carries_its_own_color_where_the_console_renders_them_all() {
    let text = painted("truecolor", &[]);
    // Each is the 24-bit value the scheme writes, sent as itself because the
    // depth has it.
    assert!(text.contains("\x1b[38;2;0;215;95m2026-09-16T12:04:00Z\x1b[0m"), "timestamp: {text:?}");
    assert!(text.contains("\x1b[38;2;0;95;255m10.0.1.4\x1b[0m"), "ip: {text:?}");
    assert!(text.contains("\x1b[38;2;0;95;215mhttps://api.internal/v2\x1b[0m"), "url: {text:?}");
    assert!(text.contains("\x1b[38;2;215;135;0m4.2ms\x1b[0m"), "duration: {text:?}");
    assert!(text.contains("\x1b[38;2;215;175;0m1.5KiB\x1b[0m"), "bytesize: {text:?}");
    assert!(text.contains("\x1b[38;2;215;0;215m\"user record\"\x1b[0m"), "quoted: {text:?}");
    // A card is a finding, so it carries the underline as well as the hue.
    assert!(text.contains("\x1b[4;38;2;255;0;0m4111 1111 1111 1111\x1b[0m"), "card: {text:?}");
    // `GET`, `from` and `took` are words, and the last line is all words, so
    // neither carries a code: the quiet part of a line is what makes the rest
    // readable.
    assert!(text.contains("plain words with no values at all\n"), "words: {text:?}");
}

#[test]
fn the_families_take_over_where_the_console_has_only_sixteen() {
    let text = painted("16", &[]);
    // The same tokens, each now its family's base code: green for when,
    // blue for where, yellow for how much, magenta for which, red for the
    // alarm - and the underline survives, which is why it is an attribute
    // rather than a color.
    assert!(text.contains("\x1b[32m2026-09-16T12:04:00Z\x1b[0m"), "timestamp: {text:?}");
    assert!(text.contains("\x1b[94m10.0.1.4\x1b[0m"), "ip: {text:?}");
    assert!(text.contains("\x1b[34mhttps://api.internal/v2\x1b[0m"), "url: {text:?}");
    assert!(text.contains("\x1b[33m4.2ms\x1b[0m"), "duration: {text:?}");
    assert!(text.contains("\x1b[33m1.5KiB\x1b[0m"), "bytesize: {text:?}");
    assert!(text.contains("\x1b[35m\"user record\"\x1b[0m"), "quoted: {text:?}");
    assert!(text.contains("\x1b[4;91m4111 1111 1111 1111\x1b[0m"), "card: {text:?}");
}

#[test]
fn the_level_decides_how_much_of_a_line_is_painted() {
    // Nothing, through the spec grammar the kinds already speak.
    let off = painted("truecolor", &["--colors", "kind:*:none"]);
    assert!(!off.contains('\x1b'), "an escape survived kind:*:none: {off:?}");
    // Everything, where a line should read as an editor paints source.
    let all = painted("truecolor", &["--colors", "kind:*:all"]);
    assert!(all.contains("\x1b[38;2;175;175;175mGET\x1b[0m"), "word: {all:?}");
    assert!(all.contains("\x1b[38;2;175;175;175mplain\x1b[0m"), "word: {all:?}");
    // The default is between them: the values, and the words plain.
    let values = painted("truecolor", &[]);
    assert!(!values.contains("\x1b[38;2;175;175;175mGET\x1b[0m"), "word painted: {values:?}");
    assert!(values.contains("\x1b[38;2;0;95;255m10.0.1.4\x1b[0m"), "ip: {values:?}");
}

#[test]
fn all_color_off_still_turns_this_off_too() {
    let never = painted("never", &[]);
    assert!(!never.contains('\x1b'), "an escape survived --color never: {never:?}");
}

#[test]
fn one_kind_can_be_named_over_the_scheme_and_over_the_level() {
    // Every kind off, then one back on: a spec naming a kind is asking for
    // that kind, whatever the level would have done with it.
    let text = painted("truecolor", &["--colors", "kind:*:none", "--colors", "kind:ip:fg:blue"]);
    assert!(text.contains("\x1b[34m10.0.1.4\x1b[0m"), "ip: {text:?}");
    assert!(!text.contains("2026-09-16T12:04:00Z\x1b[0m"), "timestamp still painted: {text:?}");
}

#[test]
fn a_register_named_by_a_spec_paints_over_the_match() {
    let dir = std::env::temp_dir().join(format!("trex/capcolor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the fixture directory");
    let path = dir.join("kv.log");
    std::fs::write(&path, "host=alpha port=8080\n").expect("write the fixture");
    let file = path.to_string_lossy().into_owned();
    let run = |extra: &[&str]| {
        let mut args = vec![
            "scan",
            "\\W:key \\P \\W:val",
            &file,
            "--passthru",
            "--color",
            "truecolor",
            "--colors",
            "kind:*:none",
        ];
        args.extend_from_slice(extra);
        let out = trex(&args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        stdout(&out)
    };
    // With no spec the whole match is the match's own color.
    assert!(run(&[]).starts_with("\x1b[1;31mhost=alpha\x1b[0m port=8080"), "{:?}", run(&[]));
    // A register the reader named is the narrower thing they asked for, so it
    // paints over the match, and the rest of the match keeps its own color.
    let one = run(&["--colors", "capture:key:fg:green"]);
    assert!(one.starts_with("\x1b[32mhost\x1b[0m\x1b[1;31m=alpha\x1b[0m"), "{one:?}");
    // Two registers leave only the punctuation between them as the match.
    let two = run(&["--colors", "capture:key:fg:green", "--colors", "capture:val:fg:blue"]);
    assert!(
        two.starts_with("\x1b[32mhost\x1b[0m\x1b[1;31m=\x1b[0m\x1b[34malpha\x1b[0m"),
        "{two:?}"
    );
    // A register at the very start of its match leaves no text before it, and
    // an empty run is not painted: an escape pair with nothing between it is
    // bytes that render as nothing and read as a mistake to anything counting
    // them.
    assert!(!one.contains("\x1b[1;31m\x1b[0m"), "an empty match run was painted: {one:?}");
    assert!(!two.contains("\x1b[1;31m\x1b[0m"), "an empty match run was painted: {two:?}");
    // A second spec for one register builds on the first rather than
    // replacing it.
    let bold = run(&["--colors", "capture:key:fg:green", "--colors", "capture:key:style:bold"]);
    assert!(bold.starts_with("\x1b[1;32mhost\x1b[0m"), "{bold:?}");
    // A name the pattern binds no register under paints nothing, and is not
    // an error: the kinds still apply.
    let none = run(&["--colors", "capture:nosuch:fg:green"]);
    assert!(none.starts_with("\x1b[1;31mhost=alpha\x1b[0m"), "{none:?}");
    std::fs::remove_dir_all(&dir).expect("remove the fixture directory");
}

#[test]
fn a_kind_a_level_or_a_register_that_is_not_one_is_refused() {
    let path = fixture();
    let file = path.to_string_lossy().into_owned();
    let refused = |spec: &str| {
        let out = trex(&["scan", "\\N", &file, "--colors", spec]);
        assert!(!out.status.success(), "{spec:?} was accepted: {}", stdout(&out));
        stderr(&out)
    };
    assert!(refused("kind:nonsense:fg:blue").contains("is no token kind"), "kind");
    assert!(refused("kind:*:sometimes").contains("kind:* takes none, values or all"), "level");
    assert!(refused("capture:").contains("names a register and nothing to paint it"), "register");
}
