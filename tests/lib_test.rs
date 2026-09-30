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
    // A kind read from the words around it reads nothing where the window
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
fn a_test_stands_anywhere_in_its_file_and_reads_the_escapes() {
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
