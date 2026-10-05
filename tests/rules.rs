//! Rules in a pattern file and the findings report: the block and one-line
//! rule forms, `scan --rules` over files and directories of rules, the
//! compiler-style, JSON, SARIF, GitHub and `--format` reports, rules on
//! records and on globs, and the fixes applied through the rewrite review.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// A directory of inputs under the temp directory, removed on drop, that
/// commands run inside so their reports name files by their bare names.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/rules-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Dir(dir)
    }

    fn write(&self, name: &str, text: &str) -> &Dir {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the file's directory");
        }
        std::fs::write(path, text).expect("write input");
        self
    }

    fn read(&self, name: &str) -> String {
        String::from_utf8_lossy(&std::fs::read(self.0.join(name)).expect("read the file")).into_owned()
    }

    fn trex(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .current_dir(&self.0)
            .env_remove("NO_COLOR")
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .output()
            .expect("run trex")
    }

    /// Run with `input` written to the standard input.
    fn trex_stdin(&self, args: &[&str], input: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .current_dir(&self.0)
            .env_remove("NO_COLOR")
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn trex");
        let mut stdin = child.stdin.take().expect("a piped stdin");
        stdin.write_all(input.as_bytes()).expect("write the stream");
        drop(stdin);
        child.wait_with_output().expect("wait for trex")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove temp dir");
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

/// Three rules: a block with a fix and metadata, a one-line error with its
/// fix and metadata on lines below, and a one-line note.
const RULES: &str = concat!(
    "# what a config file may not hold\n",
    "rule private_ip\n",
    "  pattern = \\{ip_private}:addr\n",
    "  message = private address ${addr} in ${path:name}\n",
    "  severity = warning\n",
    "  fix = ${addr:octet1-2}.x.x\n",
    "  meta.cwe = CWE-200\n",
    "  meta.tags = network, config\n",
    "\n",
    "rule cardnum error \"card number ending ${card:last4}\" = \\{card}:card\n",
    "fix cardnum = ****\n",
    "meta cardnum tags = pii\n",
    "rule todo note \"a TODO left in ${path:name}\" = \"TODO\"\n",
    "test private_ip accepts \"10.0.0.5\" rejects \"8.8.8.8\"\n",
);

const CONF: &str = "host = 10.0.0.5\npay 4111 1111 1111 1111 now\n# TODO rotate\n";

/// The report the rules give over `CONF`.
const REPORT: &str = concat!(
    "app.conf:1:8: warning: private address 10.0.0.5 in app.conf [private_ip]\n",
    "  fix: \"10.0.x.x\"\n",
    "app.conf:2:5: error: card number ending 1111 [cardnum]\n",
    "  fix: \"****\"\n",
    "app.conf:3:3: note: a TODO left in app.conf [todo]\n",
);

#[test]
fn a_pattern_file_declares_rules_in_both_forms() {
    let mut shapes = trex::ShapeSet::new();
    shapes.declare_text(RULES).expect("the rules parse");
    let rules = shapes.rules();
    assert_eq!(rules.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["private_ip", "cardnum", "todo"]);
    let ip = &rules[0];
    assert_eq!((ip.line, ip.severity, ip.source.as_str()), (2, trex::Severity::Warning, "\\{ip_private}:addr"));
    assert_eq!(ip.message, "private address ${addr} in ${path:name}");
    assert_eq!(ip.fix.as_deref(), Some("${addr:octet1-2}.x.x"));
    assert_eq!(ip.meta, [("cwe".to_string(), "CWE-200".to_string()), ("tags".to_string(), "network, config".to_string())]);
    assert_eq!(ip.tags(), ["network", "config"]);
    assert!(!ip.on_records());
    let card = &rules[1];
    assert_eq!((card.line, card.severity, card.message.as_str()), (10, trex::Severity::Error, "card number ending ${card:last4}"));
    assert_eq!(card.fix.as_deref(), Some("****"));
    assert_eq!(card.tags(), ["pii"]);
    assert_eq!((rules[2].severity, rules[2].source.as_str()), (trex::Severity::Note, "\"TODO\""));
    // A rule is a sub-pattern under its name, so a test line checks it and
    // a later pattern reads it.
    assert!(shapes.run_tests().is_empty());
    assert!(shapes.let_of("cardnum").is_some());
    let set = trex::PatternSet::from_text(RULES, &mut trex::ShapeSet::new()).expect("a set from the rules");
    assert_eq!(set.names(), ["private_ip", "cardnum", "todo"]);
    assert_eq!(set.scan(CONF.as_bytes()).len(), 3);
}

#[test]
fn a_rule_that_is_not_one_names_its_line() {
    let refused = |text: &str| trex::ShapeSet::new().declare_text(text).expect_err("refused").msg;
    assert!(refused("rule x\n  message = m\n").starts_with("line 1: rule x has no `pattern = PATTERN` line"));
    assert!(refused("rule x\n  pattern = \\N\n").starts_with("line 1: rule x has no `message = TEXT` line"));
    let e = refused("rule x\n  pattern = \\N\n  message = ${nope}\n");
    assert!(e.starts_with("line 1: message error at byte 0"), "{e}");
    let e = refused("rule x\n  pattern = \\N\n  message = m\n  severity = loud\n");
    assert!(e.starts_with("line 4: \"loud\" is not a severity"), "{e}");
    let e = refused("rule x\n  pattern = \\N\n  message = m\n  color = red\n");
    assert!(e.starts_with("line 4: \"color\" is not a rule field"), "{e}");
    let e = refused("rule x warning \"m\" = \\N\nrule x \"m\" = \\W\n");
    assert!(e.starts_with("line 2: rule x is declared twice"), "{e}");
    let e = refused("fix x = y\n");
    assert!(e.starts_with("line 1: no rule named x is declared above this line"), "{e}");
    let e = refused("rule x \"m\" = \\N\nfix x = ${nope}\n");
    assert!(e.starts_with("line 2: fix error at byte 0"), "{e}");
    let e = refused("rule ip \"m\" = \\I\n");
    assert!(e.contains("built-in atom"), "{e}");
    let e = refused("rule x \"m\"\n");
    assert!(e.contains("expected `rule NAME"), "{e}");
}

#[test]
fn findings_print_as_compiler_lines_and_an_error_fails_the_run() {
    let dir = Dir::new("report");
    dir.write("rules.trex", RULES).write("app.conf", CONF);
    let out = dir.trex(&["scan", "--rules", "rules.trex", "app.conf"]);
    assert_eq!(stdout(&out), REPORT);
    assert!(!out.status.success(), "an error finding fails the run");
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--text", "see 10.0.0.9 here"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "1:5: warning: private address 10.0.0.9 in  [private_ip]\n  fix: \"10.0.x.x\"\n");
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--text", "nothing"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "no finding\n");
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--require-match", "--text", "nothing"]);
    assert!(!out.status.success());
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--count", "app.conf"]);
    assert_eq!(stdout(&out), "3\n");
    let out = dir.trex(&["scan", "--rules", "rules.trex", "-m", "1", "app.conf"]);
    assert_eq!(stdout(&out), "app.conf:1:8: warning: private address 10.0.0.5 in app.conf [private_ip]\n  fix: \"10.0.x.x\"\n");
    let out = dir.trex_stdin(&["scan", "--rules", "rules.trex"], "# TODO later\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "-:1:3: note: a TODO left in - [todo]\n");
}

#[test]
fn a_count_and_a_file_list_over_a_tail_place_only_what_a_message_writes() {
    let dir = Dir::new("tailcount");
    dir.write("plain.trex", "rule todo note \"a TODO\" = \"TODO\"\n")
        .write("placed.trex", "rule todo note \"a TODO at ${line}:${col}, ${start}..${end}\" = \"TODO\"\n")
        .write("app.conf", "one\ntwo TODO\nthree TODO\n");
    for rules in ["plain.trex", "placed.trex"] {
        let out = dir.trex(&["scan", "--rules", rules, "--count", "--tail", "1", "app.conf"]);
        assert!(out.status.success(), "{rules}: {}", stderr(&out));
        assert_eq!(stdout(&out), "1\n", "{rules}");
        let out = dir.trex(&["scan", "--rules", rules, "-l", "--tail", "1", "app.conf"]);
        assert!(out.status.success(), "{rules}: {}", stderr(&out));
        assert_eq!(stdout(&out), "app.conf\n", "{rules}");
    }
    // A message writing its match's position places it in the file it was
    // read from, as the report does.
    let out = dir.trex(&["scan", "--rules", "placed.trex", "--tail", "1", "app.conf"]);
    assert_eq!(stdout(&out), "app.conf:3:7: note: a TODO at 3:7, 19..23 [todo]\n");
}

#[test]
fn the_pipeline_reports_carry_the_rule_its_place_and_its_fix() {
    let dir = Dir::new("pipeline");
    dir.write("rules.trex", RULES).write("app.conf", CONF);
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--json", "app.conf"]);
    assert_eq!(
        stdout(&out),
        "[{\"rule\":\"private_ip\",\"severity\":\"warning\",\"message\":\"private address 10.0.0.5 in app.conf\",\"path\":\"app.conf\",\"line\":1,\"col\":8,\"end_line\":1,\"end_col\":16,\"start\":7,\"end\":15,\"text\":\"10.0.0.5\",\"captures\":{\"addr\":\"10.0.0.5\"},\"fix\":\"10.0.x.x\",\"meta\":{\"cwe\":\"CWE-200\",\"tags\":\"network, config\"}},\
{\"rule\":\"cardnum\",\"severity\":\"error\",\"message\":\"card number ending 1111\",\"path\":\"app.conf\",\"line\":2,\"col\":5,\"end_line\":2,\"end_col\":24,\"start\":20,\"end\":39,\"text\":\"4111 1111 1111 1111\",\"captures\":{\"card\":\"4111 1111 1111 1111\"},\"fix\":\"****\",\"meta\":{\"tags\":\"pii\"}},\
{\"rule\":\"todo\",\"severity\":\"note\",\"message\":\"a TODO left in app.conf\",\"path\":\"app.conf\",\"line\":3,\"col\":3,\"end_line\":3,\"end_col\":7,\"start\":46,\"end\":50,\"text\":\"TODO\",\"captures\":{}}]\n"
    );
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--github", "app.conf"]);
    assert_eq!(
        stdout(&out),
        "::warning file=app.conf,line=1,col=8,endLine=1,endColumn=16,title=private_ip::private address 10.0.0.5 in app.conf\n\
::error file=app.conf,line=2,col=5,endLine=2,endColumn=24,title=cardnum::card number ending 1111\n\
::notice file=app.conf,line=3,col=3,endLine=3,endColumn=7,title=todo::a TODO left in app.conf\n"
    );
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--sarif", "app.conf"]);
    let sarif = stdout(&out);
    assert!(sarif.starts_with("{\"$schema\":\"https://json.schemastore.org/sarif-2.1.0.json\",\"version\":\"2.1.0\",\"runs\":[{\"tool\":{\"driver\":{\"name\":\"trex\",\"version\":\""), "{sarif}");
    assert!(sarif.contains("\"rules\":[{\"id\":\"private_ip\",\"shortDescription\":{\"text\":\"private address ${addr} in ${path:name}\"},\"fullDescription\":{\"text\":\"\\\\{ip_private}:addr\"},\"defaultConfiguration\":{\"level\":\"warning\"},\"properties\":{\"tags\":[\"network\",\"config\"],\"cwe\":\"CWE-200\"}},{\"id\":\"cardnum\","), "{sarif}");
    assert!(sarif.contains("\"results\":[{\"ruleId\":\"private_ip\",\"ruleIndex\":0,\"level\":\"warning\",\"message\":{\"text\":\"private address 10.0.0.5 in app.conf\"},\"locations\":[{\"physicalLocation\":{\"artifactLocation\":{\"uri\":\"app.conf\"},\"region\":{\"startLine\":1,\"startColumn\":8,\"endLine\":1,\"endColumn\":16,\"snippet\":{\"text\":\"10.0.0.5\"}}}}],\"fixes\":[{\"description\":{\"text\":\"private_ip\"},\"artifactChanges\":[{\"artifactLocation\":{\"uri\":\"app.conf\"},\"replacements\":[{\"deletedRegion\":{\"startLine\":1,\"startColumn\":8,\"endLine\":1,\"endColumn\":16},\"insertedContent\":{\"text\":\"10.0.x.x\"}}]}]}]},{\"ruleId\":\"cardnum\",\"ruleIndex\":1,\"level\":\"error\","), "{sarif}");
    assert!(sarif.ends_with("{\"ruleId\":\"todo\",\"ruleIndex\":2,\"level\":\"note\",\"message\":{\"text\":\"a TODO left in app.conf\"},\"locations\":[{\"physicalLocation\":{\"artifactLocation\":{\"uri\":\"app.conf\"},\"region\":{\"startLine\":3,\"startColumn\":3,\"endLine\":3,\"endColumn\":7,\"snippet\":{\"text\":\"TODO\"}}}}]}]}]}\n"), "{sarif}");
    // A run holding results says what its columns count, as SARIF 2.1.0
    // section 3.14.27 requires.
    assert!(sarif.contains("]}},\"columnKind\":\"unicodeCodePoints\",\"results\":[{"), "{sarif}");
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--format", "${severity}|${rule}|${line}|${message}|${fix}|${0}", "app.conf"]);
    assert_eq!(
        stdout(&out),
        "warning|private_ip|1|private address 10.0.0.5 in app.conf|10.0.x.x|10.0.0.5\nerror|cardnum|2|card number ending 1111|****|4111 1111 1111 1111\nnote|todo|3|a TODO left in app.conf||TODO\n"
    );
}

#[test]
fn fixes_are_applied_through_the_rewrite_review() {
    let dir = Dir::new("fix");
    dir.write("rules.trex", RULES).write("app.conf", CONF);
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--fix", "--dry-run", "app.conf"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "--- app.conf\n+++ app.conf\n@@ -1,3 +1,3 @@\n-host = 10.0.0.5\n-pay 4111 1111 1111 1111 now\n+host = 10.0.x.x\n+pay **** now\n # TODO rotate\n"
    );
    assert_eq!(dir.read("app.conf"), CONF, "a dry run writes nothing");
    let out = dir.trex_stdin(&["scan", "--rules", "rules.trex", "--fix", "--interactive", "app.conf"], "y\nn\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(dir.read("app.conf"), "host = 10.0.x.x\npay 4111 1111 1111 1111 now\n# TODO rotate\n");
    assert!(stderr(&out).contains("app.conf: 1 replacement"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--fix", "app.conf"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(dir.read("app.conf"), "host = 10.0.x.x\npay **** now\n# TODO rotate\n");
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--fix", "--json", "app.conf"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--fix applies the fixes and prints no report"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--dry-run", "app.conf"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--dry-run and --interactive review fixes and take --fix"), "{}", stderr(&out));
}

#[test]
fn a_rule_on_records_fires_where_the_record_lacks_the_unless_pattern() {
    let dir = Dir::new("records");
    dir.write(
        "vault.trex",
        "rule unguarded\n  pattern = \"password\"\n  unless = \"vault\"\n  record = paragraph\n  message = a password outside the vault\n  severity = error\n",
    )
    .write("notes.txt", "password = x\nvault: ok\n\npassword = y\nplain\n");
    let out = dir.trex(&["scan", "--rules", "vault.trex", "notes.txt"]);
    assert_eq!(stdout(&out), "notes.txt:4:1: error: a password outside the vault [unguarded]\n");
    assert!(!out.status.success());
    let out = dir.trex(&["scan", "--rules", "vault.trex", "--json", "notes.txt"]);
    assert_eq!(
        stdout(&out),
        "[{\"rule\":\"unguarded\",\"severity\":\"error\",\"message\":\"a password outside the vault\",\"path\":\"notes.txt\",\"line\":4,\"col\":1,\"end_line\":5,\"end_col\":6,\"start\":24,\"end\":42,\"text\":\"password = y\\nplain\",\"captures\":{}}]\n"
    );
    // A rule naming only what a record must not hold reads lines.
    dir.write("line.trex", "rule bare warning \"a password without a vault\" = \"password\"\nunless bare = \"vault\"\n");
    let out = dir.trex(&["scan", "--rules", "line.trex", "notes.txt"]);
    assert_eq!(stdout(&out), "notes.txt:1:1: warning: a password without a vault [bare]\nnotes.txt:4:1: warning: a password without a vault [bare]\n");
}

#[test]
fn a_rule_reads_only_the_files_its_globs_keep() {
    let dir = Dir::new("globs");
    dir.write(
        "rules.trex",
        "rule py_todo note \"a TODO in Python\" = \"TODO\"\nfiles py_todo = *.py\nrule any_todo note \"a TODO\" = \"TODO\"\nfiles any_todo = *.py, *.rs, !test_*\n",
    )
    .write("a.py", "# TODO one\n")
    .write("b.rs", "// TODO two\n")
    .write("test_c.py", "# TODO three\n")
    .write("d.txt", "TODO four\n");
    let out = dir.trex(&["scan", "--rules", "rules.trex", "a.py", "b.rs", "test_c.py", "d.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "a.py:1:3: note: a TODO in Python [py_todo]\na.py:1:3: note: a TODO [any_todo]\nb.rs:1:4: note: a TODO [any_todo]\ntest_c.py:1:3: note: a TODO in Python [py_todo]\n"
    );
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--text", "TODO"]);
    assert_eq!(stdout(&out), "no finding\n", "an unnamed input is read by no rule with globs");
}

#[test]
fn a_directory_of_rules_is_every_trex_file_under_it() {
    let dir = Dir::new("dir");
    dir.write("rules/a.trex", "rule one note \"one\" = \"alpha\"\n")
        .write("rules/sub/b.trex", "rule two note \"two\" = \"beta\"\n")
        .write("rules/README.md", "not a rule file\n")
        .write("in.txt", "alpha beta\n");
    let out = dir.trex(&["scan", "--rules", "rules", "in.txt"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "in.txt:1:1: note: one [one]\nin.txt:1:7: note: two [two]\n");
    let out = dir.trex(&["scan", "--rules", "rules", "-l", "in.txt"]);
    assert_eq!(stdout(&out), "in.txt\n");
    dir.write("empty/none.txt", "");
    let out = dir.trex(&["scan", "--rules", "empty", "in.txt"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no .trex file under it"), "{}", stderr(&out));
    let out = dir.trex(&["lib", "--test", "rules/a.trex"]);
    assert_eq!(stdout(&out), "rules/a.trex: no test lines\n");
}

#[test]
fn the_flags_rules_take_no_part_in_are_refused() {
    let dir = Dir::new("refused");
    dir.write("rules.trex", RULES).write("app.conf", CONF);
    let out = dir.trex(&["scan", "--rules", "rules.trex", "-e", "\\N", "app.conf"]);
    assert!(stderr(&out).contains("--rules names the rules and takes no -e, -f or --patterns"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--single-match", "app.conf"]);
    assert!(stderr(&out).contains("takes no record query, -v, -x, --passthru, --single-match"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--json", "--sarif", "app.conf"]);
    assert!(stderr(&out).contains("each a report of their own"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--rules", "rules.trex", "-C", "2", "app.conf"]);
    assert!(stderr(&out).contains("takes no context lines"), "{}", stderr(&out));
    let out = dir.trex(&["scan", "--rules", "rules.trex", "--fix", "--text", "10.0.0.1"]);
    assert!(stderr(&out).contains("--fix writes files and takes no --text"), "{}", stderr(&out));
    dir.write("bad.trex", "rule x \"m\" = \\N\nrule y\n  pattern = \\W\n");
    let out = dir.trex(&["scan", "--rules", "bad.trex", "app.conf"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("bad.trex: line 2: rule y has no `message = TEXT` line"), "{}", stderr(&out));
}
