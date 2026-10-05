//! `trex lib --test FILE`: the `test` lines of a pattern file run against
//! what the file declares, each failure naming its line; with no file, the
//! shipped library's own lines run the same way.

use std::path::PathBuf;
use std::process::{Command, Output};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A directory of pattern files under the temp directory, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/lib-test-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Dir(dir)
    }

    fn write(&self, name: &str, text: &str) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("write pattern file");
        path.to_string_lossy().into_owned()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove temp dir");
    }
}

#[test]
fn a_file_whose_tests_all_pass_counts_them_and_succeeds() {
    let dir = Dir::new("pass");
    let defs = dir.write(
        "defs.trex",
        "let rhs = \\N | \\Q\nkind assign = \\W \"=\" \\{rhs}\ntest assign accepts \"x = 1\" \"name = \\\"bob\\\"\" rejects \"x == 1\"\ntest rhs accepts \"42\" rejects \"forty-two\"\ntest iban accepts \"GB82 WEST 1234 5698 7654 32\" rejects \"GB82WEST12345698765433\"\n",
    );
    let out = trex(&["lib", "--test", &defs]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), format!("{defs}: 3 tests passed\n"));
    assert_eq!(stderr(&out), "");
}

#[test]
fn each_failure_names_its_line_and_the_status_is_failure() {
    let dir = Dir::new("fail");
    let wrong = dir.write(
        "wrong.trex",
        "let rhs = \\N\ntest rhs accepts \"42\" \"\\\"bob\\\"\" rejects \"4 2\"\ntest nosuch accepts \"x\"\ntest rhs accepts \"7\"\n",
    );
    let out = trex(&["lib", "--test", &wrong]);
    assert!(!out.status.success());
    assert_eq!(
        stdout(&out),
        format!(
            "{wrong}:2: rhs accepts \"\\\"bob\\\"\": no match\n\
             \x20 tokens: quoted \"\\\"bob\\\"\"\n\
             {wrong}:2: rhs rejects \"4 2\": matched \"4\" at 0..1\n\
             {wrong}:3: nosuch cannot be tested: unknown named atom \\{{nosuch}}; it is neither a built-in, a declared shape, kind or sub-pattern, nor a library entry\n\
             {wrong}: 2 of 3 tests failed\n"
        )
    );
    // A text the name matches, but not as a whole, says what it did match.
    let partial = dir.write("partial.trex", "test ip accepts \"from 10.0.0.1 now\"\n");
    let out = trex(&["lib", "--test", &partial]);
    assert!(!out.status.success());
    assert_eq!(
        stdout(&out),
        format!(
            "{partial}:1: ip accepts \"from 10.0.0.1 now\": matched only \"10.0.0.1\" at 5..13\n\
             \x20 tokens: word \"from\", ip \"10.0.0.1\", word \"now\"\n\
             {partial}: 1 of 1 test failed\n"
        )
    );
}

#[test]
fn a_failing_accepts_carries_the_lex_of_its_text() {
    let dir = Dir::new("lexed");
    let bad = dir.write("bad.trex", "test mac accepts \"00:1A:2B:3C:4D\"\n");
    let out = trex(&["lib", "--test", &bad]);
    assert!(!out.status.success());
    // Five groups rather than six, and the kinds say why the whole never
    // lexed as one address: `1A` reads as a quantity and `2B` as a byte size.
    let want = format!("{bad}:1: mac accepts \"00:1A:2B:3C:4D\": no match\n")
        + "  tokens: number \"00\", punct \":\", quantity \"1A\", punct \":\", bytesize \"2B\", "
        + "punct \":\", number \"3\", word \"C\", punct \":\", number \"4\", word \"D\"\n"
        + &format!("{bad}: 1 of 1 test failed\n");
    assert_eq!(stdout(&out), want);
}

#[test]
fn reads_takes_a_span_out_of_a_larger_text() {
    let dir = Dir::new("reads");
    let good = dir.write("reads.trex", "test kelvin reads \"4.2K\" in \"cooled to 4.2K overnight\"\n");
    let out = trex(&["lib", "--test", &good]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    assert_eq!(stdout(&out), format!("{good}: 1 test passed\n"));
    // A kind read from the words around it reads no value where the window
    // holds no cue, reads the wrong span where the span is not what it takes,
    // and a span the text does not hold is a fault in the line itself.
    let bad = dir.write(
        "bad.trex",
        "test kelvin reads \"300 K\" in \"see item 300 K for details\"\n\
         test kelvin reads \"9Q\" in \"cooled to 4.2K overnight\"\n\
         test kelvin reads \"4.2\" in \"cooled to 4.2K overnight\"\n",
    );
    let out = trex(&["lib", "--test", &bad]);
    assert!(!out.status.success());
    assert_eq!(
        stdout(&out),
        format!(
            "{bad}:1: kelvin reads \"300 K\" in \"see item 300 K for details\": no match\n\
             {bad}:2: kelvin reads \"9Q\": \"cooled to 4.2K overnight\" does not hold it\n\
             {bad}:3: kelvin reads \"4.2\" in \"cooled to 4.2K overnight\": read \"4.2K\" at 10..14\n\
             {bad}: 3 of 3 tests failed\n"
        )
    );
}

#[test]
fn a_test_may_be_anywhere_in_its_file_and_reads_the_escapes() {
    let dir = Dir::new("order");
    let defs = dir.write(
        "defs.trex",
        "test pair accepts \"1\\n2\" \"1\\t2\" rejects \"1\\t\"\nlet pair = \\N \\N\ntest quote accepts \"say \\\"hi\\\"\" \"a \\\\ b\"\nlet quote = \\W \\Q | \\W \"\\\\\" \\W\n",
    );
    let out = trex(&["lib", "--test", &defs]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    assert_eq!(stdout(&out), format!("{defs}: 2 tests passed\n"));
}

#[test]
fn a_malformed_test_line_is_refused_with_its_line() {
    let dir = Dir::new("malformed");
    let bad = dir.write("bad.trex", "let ok = \\N\ntest ok \"1\"\n");
    let out = trex(&["lib", "--test", &bad]);
    assert!(!out.status.success());
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("line 2: a text follows accepts, reads or rejects"), "{}", stderr(&out));
    let bad = dir.write("escape.trex", "test ok accepts \"\\q\"\n");
    let out = trex(&["lib", "--test", &bad]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("line 1: unknown escape \\q"), "{}", stderr(&out));
}

#[test]
fn later_files_see_earlier_declarations_and_each_file_is_counted() {
    let dir = Dir::new("two");
    let base = dir.write("base.trex", "let rhs = \\N\n");
    let more = dir.write("more.trex", "kind assign = \\W \"=\" \\{rhs}\ntest assign accepts \"x = 1\"\ntest rhs accepts \"5\"\n");
    let out = trex(&["lib", "--test", &base, "--test", &more]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), format!("{base}: no test lines\n{more}: 2 tests passed\n"));
}

#[test]
fn a_directory_tests_every_pattern_file_under_it_in_path_order() {
    let dir = Dir::new("dir");
    let nested = dir.0.join("more");
    std::fs::create_dir_all(&nested).expect("create the nested directory");
    let base = dir.write("a.trex", "let rhs = \\N\n");
    let more = nested.join("b.trex");
    std::fs::write(&more, "kind assign = \\W \"=\" \\{rhs}\ntest assign accepts \"x = 1\"\n").expect("write b.trex");
    let more = more.to_string_lossy().into_owned();
    dir.write("notes.txt", "test rhs accepts \"never read\"\n");
    let top = dir.0.to_string_lossy().into_owned();
    let out = trex(&["lib", "--test", &top]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), format!("{base}: no test lines\n{more}: 1 test passed\n"));

    let empty = dir.0.join("empty");
    std::fs::create_dir_all(&empty).expect("create the empty directory");
    let out = trex(&["lib", "--test", &empty.to_string_lossy()]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no .trex file under it"), "{}", stderr(&out));
}

#[test]
fn lib_reads_a_directory_as_its_pattern_files() {
    let dir = Dir::new("libdir");
    dir.write("ticket.trex", "shape ticket = `[A-Z]{2,4}-\\d{1,4}`\n");
    let top = dir.0.to_string_lossy().into_owned();
    let out = trex(&["scan", "\\{ticket}", "--text", "see AB-12 now", "--lib", &top]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("AB-12"), "{}", stdout(&out));
    let out = trex(&["tokens", "--text", "see AB-12 now", "--lib", &top]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("[4..9] ticket \"AB-12\""), "{}", stdout(&out));
}

#[test]
fn each_inline_flag_declares_what_its_pattern_file_line_declares() {
    let tokens = |decl: &[&str]| {
        let out = trex(&[&["tokens", "--text", "took 15ms"], decl].concat());
        assert!(out.status.success(), "{decl:?}: {}", stderr(&out));
        stdout(&out)
    };
    assert_eq!(tokens(&["--shape", "code = `[0-9]{2}ms`"]), "[0..4] word \"took\"\n[5..9] code \"15ms\"\n");
    assert_eq!(tokens(&["--shape-after", "code = `[0-9]{2}ms`"]), "[0..4] word \"took\"\n[5..9] duration \"15ms\"\n");
    assert_eq!(tokens(&["--kind", "slow = \"took\" \\R"]), "[0..9] slow \"took 15ms\"\n");
    assert_eq!(tokens(&["--declare", "kind slow = \"took\" \\R"]), "[0..9] slow \"took 15ms\"\n");
    let out = trex(&["scan", "\\{pair}", "--text", "a=1 b=2", "--let", "pair = \\W \"=\" \\N"]);
    assert_eq!(stdout(&out), "[0..3] \"a=1\"\n[4..7] \"b=2\"\n", "{}", stderr(&out));
    let out = trex(&["scan", "\\{pair}", "--text", "a=1", "--declare", "let pair = \\W \"=\" \\N"]);
    assert_eq!(stdout(&out), "[0..3] \"a=1\"\n", "{}", stderr(&out));
    let out = trex(&[
        "scan",
        "\\{pair}",
        "--text",
        "a=1",
        "--let",
        "pair = \\W \"=\" \\N",
        "--declare",
        "test pair accepts \"a=1\" rejects \"a\"",
    ]);
    assert_eq!(stdout(&out), "[0..3] \"a=1\"\n", "a test line is declared: {}", stderr(&out));
}

#[test]
fn every_command_that_takes_lib_takes_the_inline_flags() {
    let kind = ["--kind", "ticket = \\W \"-\" \\N"];
    let text = "see AB-12 and CD-34, AB-12 again";
    let run = |args: &[&str]| {
        let out = trex(&[args, &kind[..]].concat());
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        stdout(&out)
    };
    assert_eq!(run(&["scan", "\\{ticket}", "--text", text]), "[4..9] \"AB-12\"\n[14..19] \"CD-34\"\n[21..26] \"AB-12\"\n");
    assert_eq!(run(&["count-by", "\\{ticket}", "${0}", "--text", text]), "AB-12  2\nCD-34  1\n");
    assert_eq!(run(&["redact", "\\{ticket}", "--text", text]), "see ***** and *****, ***** again");
    assert_eq!(run(&["rewrite", "\\{ticket}", "<${0}>", "--text", text]), "see <AB-12> and <CD-34>, <AB-12> again");
    assert!(run(&["tokens", "--text", text]).contains("[4..9] ticket \"AB-12\"\n"));
    assert_eq!(
        run(&["infer", "--mark", "see {t:AB-12} now", "see XYZ-9 now", "--pattern"]),
        "^ \"see\" (\\{ticket}):t \"now\" ~<($ .)\n"
    );
}

#[test]
fn inline_declarations_read_in_the_order_given_with_lib() {
    let dir = Dir::new("inline-order");
    let lib = dir.write("ticket.trex", "shape ticket = `[A-Z]{2,4}-\\d{1,4}`\n");
    let kind = "ref = \"ref\" \\{ticket}";
    let out = trex(&["scan", "\\{ref}", "--text", "ref AB-12 here", "--lib", &lib, "--kind", kind]);
    assert_eq!(stdout(&out), "[0..9] \"ref AB-12\"\n", "{}", stderr(&out));
    let out = trex(&["scan", "\\{ref}", "--text", "ref AB-12 here", "--kind", kind, "--lib", &lib]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown named atom \\{ticket}"), "{}", stderr(&out));
}

#[test]
fn an_inline_declaration_is_refused_in_the_words_a_library_refuses_its_line() {
    let refused = |decl: &[&str]| {
        let out = trex(&[&["scan", "x", "--text", "x"], decl].concat());
        assert!(!out.status.success(), "{decl:?}");
        stderr(&out)
    };
    assert_eq!(refused(&["--kind", "x = ("]), "trex: line 1: pattern error at byte 1: expected ')'\n");
    assert_eq!(
        refused(&["--shape", "x = `a*`"]),
        "trex: line 1: the pattern must be bounded; `*`, `+` and `{m,}` have no fixed length\n"
    );
    assert_eq!(
        refused(&["--shape", "limb = `\\x00{4}`"]),
        "trex: line 1: \\x is not a byte-pattern escape; the escapes are \\d \\D \\w \\W \\s \\S \\p{..} \\P{..}, \
         and a backslash before punctuation is that punctuation\n"
    );
    assert_eq!(
        refused(&["--shape", "limb = `(?s:.){4}`"]),
        "trex: line 1: `(?` is not byte-pattern syntax: a group takes no flags or extensions\n"
    );
    assert_eq!(
        refused(&["--shape", "limb = `[\\x00-\\xff]{4}`"]),
        "trex: line 1: a byte-pattern class takes no backslash; write each character itself\n"
    );
    let opens = "opens no declaration; a line opens with let, kind, shape, shape-after, test, fields or rule";
    assert!(refused(&["--declare", "foo x = y"]).starts_with(&format!("trex: \"foo\" {opens}")));
    assert!(refused(&["--declare", ""]).starts_with(&format!("trex: \"\" {opens}")));
    assert!(refused(&["--declare", "# a note"]).starts_with(&format!("trex: \"#\" {opens}")));
    assert_eq!(
        refused(&["--let", "a = \\N\nlet b = \\W"]),
        "trex: a declaration is one line; this one holds a line break\n"
    );
    assert_eq!(
        refused(&["--kind", "k = \\N", "--kind", "k = \\W"]),
        "trex: line 1: a shape or kind with that name exists\n"
    );
    assert_eq!(refused(&["--kind"]), "trex: --kind needs a `name = pattern` declaration\n");
    assert_eq!(refused(&["--shape"]), "trex: --shape needs a `name = `pattern`` declaration\n");
    assert_eq!(refused(&["--declare"]), "trex: --declare needs a line a pattern file holds\n");
}

#[test]
fn a_pattern_file_named_twice_is_read_once() {
    let dir = Dir::new("twice");
    let lib = dir.write("ticket.trex", "kind ticket = \\W \"-\" \\N\ntest ticket accepts \"AB-12\"\n");
    let out = trex(&["scan", "\\{ticket}", "--text", "see AB-12", "--lib", &lib, "--lib", &lib]);
    assert_eq!(stdout(&out), "[4..9] \"AB-12\"\n", "{}", stderr(&out));
    let out = trex(&["lib", "--test", &lib, &lib]);
    assert_eq!(stdout(&out), format!("{lib}: 1 test passed\n"), "{}", stderr(&out));
}

#[test]
fn with_no_file_the_shipped_library_runs_its_own_test_lines() {
    let out = trex(&["lib", "--test"]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    assert_eq!(stdout(&out), format!("the library: {} tests passed\n", trex::library::TESTS.len()));
    assert_eq!(stderr(&out), "");
}

#[test]
fn a_file_without_the_flag_is_refused_and_the_flag_takes_no_json() {
    let dir = Dir::new("noflag");
    let defs = dir.write("defs.trex", "test ip accepts \"10.0.0.1\"\n");
    let out = trex(&["lib", &defs]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--test runs its test lines"), "{}", stderr(&out));
    let dir = Dir::new("json");
    let defs = dir.write("defs.trex", "test ip accepts \"10.0.0.1\"\n");
    let out = trex(&["lib", "--json", "--test", &defs]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("takes no --json"), "{}", stderr(&out));
    let out = trex(&["lib", "--test", "no-such-file.trex"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("cannot read the pattern file"), "{}", stderr(&out));
}
