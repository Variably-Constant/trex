//! The typed relations as orbit rungs: `(?orbit:subnet/24 ...)` and the rest
//! key both sides through the relation's projection, so a join across two
//! inputs holds where the two texts are not the same bytes, and a plain
//! back-reference inside the scope compares there too. With them, the slash
//! dates the rungs read a calendar out of.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A directory of inputs under the temp directory, removed on drop, that
/// commands run inside so their reports name files by their bare names.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("trex/typed-orbits-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Dir(dir)
    }

    fn write(&self, name: &str, text: &str) -> &Dir {
        std::fs::write(self.0.join(name), text).expect("write input");
        self
    }

    fn trex(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .expect("run trex")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove temp dir");
    }
}

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

/// Two logs sharing a network, a mail domain and a calendar day, but sharing
/// only one address exactly.
fn two_logs(tag: &str) -> Dir {
    let dir = Dir::new(tag);
    dir.write(
        "a.log",
        "login from 10.0.0.7 by bob@corp.example\n\
         login from 192.168.4.9 by amy@other.example\n\
         login from 172.16.9.1 by carl@corp.example\n",
    )
    .write(
        "b.log",
        "alert 10.0.0.201 scanned the subnet\n\
         alert 172.16.9.1 reached a closed port\n\
         notice mail to dana@corp.example bounced\n",
    );
    dir
}

#[test]
fn a_join_at_the_subnet_rung_holds_where_the_exact_join_does_not() {
    let dir = two_logs("subnet");
    // Exactly one address is written in both logs.
    let out = dir.trex(&["scan", "@echoed:@b.log \\I", "a.log"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "[95..105] \"172.16.9.1\"\n");
    // Under the subnet rung 10.0.0.7 joins as well, because 10.0.0.201 is in
    // its /24 - which is the whole point of keying both sides through the
    // projection rather than through the bytes.
    let out = dir.trex(&["scan", "(?orbit:subnet/24 @echoed:@b.log \\I)", "a.log"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "[11..19] \"10.0.0.7\"\n[95..105] \"172.16.9.1\"\n");
    // The address in neither of the other log's networks is the novel one.
    let out = dir.trex(&["scan", "(?orbit:subnet/24 @novel:@b.log \\I)", "a.log"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "[51..62] \"192.168.4.9\"\n");
}

#[test]
fn a_join_at_the_domain_rung_holds_at_a_shared_mail_domain() {
    let dir = two_logs("domain");
    let out = dir.trex(&["scan", "(?orbit:domain @echoed:@b.log \\E)", "a.log"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "[23..39] \"bob@corp.example\"\n[109..126] \"carl@corp.example\"\n"
    );
    // No address is written in both logs, so the exact join reports none.
    let out = dir.trex(&["scan", "@echoed:@b.log \\E", "a.log"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "no match\n");
}

#[test]
fn the_explained_join_names_the_key_the_rung_made() {
    let dir = two_logs("explain");
    let out = dir.trex(&["scan", "(?orbit:subnet/24 @echoed:@b.log \\I)", "a.log", "--explain"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let shown = stdout(&out);
    assert!(
        shown.contains("join: occurs in b.log at the subnet rung, keyed subnet:10.0.0.0/24"),
        "{shown}"
    );
    // The identity rung's key is the token's own text, which the reading
    // already shows, so it is not repeated.
    let out = dir.trex(&["scan", "@echoed:@b.log \\I", "a.log", "--explain"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("join: occurs in b.log at the identity rung \"172.16.9.1\""), "{}", stdout(&out));
}

#[test]
fn a_plain_back_reference_inside_a_scope_compares_at_its_rung() {
    let text = "from bob@corp.example to amy@corp.example";
    let out = trex(&["scan", "(?orbit:domain \\E:a . =a)", "--text", text]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "[5..41] \"bob@corp.example to amy@corp.example\"  captures: a=\"bob@corp.example\"\n"
    );
    // A reference whose own spelling names a comparison keeps it, and one
    // outside a scope is exact.
    for pattern in ["(?orbit:domain \\E:a . =case a)", "\\E:a . =a"] {
        let out = trex(&["scan", pattern, "--text", text]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(stdout(&out), "no match\n", "{pattern}");
    }
}

#[test]
fn an_orbit_scope_is_refused_inside_another_and_a_prefix_only_on_subnet() {
    for (pattern, said) in [
        ("(?orbit:case (?orbit:numeric \"1\"))", "cannot be nested inside another"),
        ("(?orbit:domain/24 \\E)", "only the subnet rung"),
        ("(?orbit:nosuch \\W)", "unknown orbit group"),
    ] {
        let out = trex(&["scan", pattern, "--text", "x"]);
        assert!(!out.status.success(), "{pattern}");
        assert!(stderr(&out).contains(said), "{pattern}: {}", stderr(&out));
    }
}

#[test]
fn a_slash_date_is_one_timestamp_and_its_fields_read_where_the_form_puts_them() {
    let text = "2026/09/15 and 09/15/2026 and 15/09/2026 and 1/2/3 and 13/13/2026";
    let out = trex(&["scan", "\\T", "--text", text]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "[0..10] \"2026/09/15\"\n[15..25] \"09/15/2026\"\n[30..40] \"15/09/2026\"\n"
    );
    let fields = "${0:year}-${0:month}-${0:day}";
    for (written, read) in [
        ("2026/09/15", "2026-09-15"),
        ("09/15/2026", "2026-09-15"),
        ("15/09/2026", "2026-09-15"),
        ("2026-09-15", "2026-09-15"),
    ] {
        let out = trex(&["scan", "\\T", "--text", written, "--format", fields]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(stdout(&out), format!("{read}\n"), "{written}");
    }
}

#[test]
fn the_date_order_flag_decides_the_slash_date_both_readings_hold() {
    let fields = "${0:year}-${0:month}-${0:day}";
    // Day first until told otherwise, which is the order of the one slash
    // date trex read before these: Apache's.
    let out = trex(&["scan", "\\T", "--text", "03/04/2026", "--format", fields]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "2026-04-03\n");
    let out = trex(&["--date-order", "mdy", "scan", "\\T", "--text", "03/04/2026", "--format", fields]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "2026-03-04\n");
    // A date only one reading holds is read that way whatever the flag says.
    for order in ["dmy", "mdy"] {
        let out = trex(&["--date-order", order, "scan", "\\T", "--text", "15/09/2026", "--format", fields]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(stdout(&out), "2026-09-15\n", "{order}");
    }
    let out = trex(&["--date-order", "ymd", "scan", "\\T", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("is neither dmy nor mdy"), "{}", stderr(&out));
}

#[test]
fn a_typed_rung_folds_a_collapse_table_and_leaves_what_it_cannot_read_alone() {
    let out = trex(&[
        "orbit",
        "--group",
        "domain",
        "--text",
        "bob@corp.example amy@corp.example carl@other.example plainword",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let shown = stdout(&out);
    assert!(shown.contains("\"bob@corp.example\" -> \"domain:corp.example\""), "{shown}");
    assert!(shown.contains("\"amy@corp.example\" -> \"domain:corp.example\""), "{shown}");
    assert!(shown.contains("\"carl@other.example\" -> \"domain:other.example\""), "{shown}");
    // A text the projection cannot read stands for itself, behind a mark no
    // projected key carries, so a bare word never folds onto a domain.
    assert!(shown.contains("\"plainword\" -> \"?plainword\""), "{shown}");
    // The width is written after a slash here as it is in a scope.
    let out = trex(&["orbit", "--group", "subnet/16", "--text", "10.0.0.7 10.0.9.9 10.1.0.1"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let shown = stdout(&out);
    assert!(shown.contains("\"10.0.0.7\" -> \"subnet:10.0.0.0/16\""), "{shown}");
    assert!(shown.contains("\"10.0.9.9\" -> \"subnet:10.0.0.0/16\""), "{shown}");
    assert!(shown.contains("\"10.1.0.1\" -> \"subnet:10.1.0.0/16\""), "{shown}");
    let out = trex(&["orbit", "--group", "domain/16", "--text", "x"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown group"), "{}", stderr(&out));
}
