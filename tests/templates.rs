//! The templates command and the `@shape:rare` anchor: lines grouped by
//! their token-kind silhouette, printed once each with a count, as text or
//! as a pattern, and a scan that keeps only the tokens of rare lines.

use std::process::{Command, Output};

use trex::{parse, scan};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

/// Five request lines of one shape and one line of another.
const LOG: &str = "10.0.0.1 GET /index.html status 200 12ms\n\
                   10.0.0.2 GET /about.html status 200 8ms\n\
                   10.0.0.1 POST /login status 302 40ms\n\
                   10.0.0.3 GET /index.html status 200 11ms\n\
                   10.0.0.9 GET /admin status 403 3ms\n\
                   kernel: disk failure on /dev/sda\n";

#[test]
fn each_template_prints_once_with_its_count_most_frequent_first() {
    let out = trex(&["templates", "--text", LOG]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        stdout(&out),
        "5  <ip> <word> <path> status <number> <duration>\n1  kernel: disk failure on /dev/sda\n"
    );
}

#[test]
fn the_pattern_form_matches_exactly_the_lines_of_its_template() {
    let out = trex(&["templates", "--text", LOG, "--pattern"]);
    let text = stdout(&out);
    assert_eq!(
        text,
        "5  \\I \\W \\L \"status\" \\N \\R\n1  \"kernel\" \":\" \"disk\" \"failure\" \"on\" \"/dev/sda\"\n"
    );
    // Each printed pattern, scanned as written, finds its template's lines
    // and no others.
    for line in text.lines() {
        let (count, pattern) = line.split_once("  ").expect("a count, then the pattern");
        let pat = parse(pattern).unwrap_or_else(|e| panic!("{pattern}: {e:?}"));
        let expected: usize = count.trim().parse().expect("a count");
        assert_eq!(scan(&pat, LOG.as_bytes()).len(), expected, "{pattern}");
    }
}

#[test]
fn rare_lists_the_rare_templates_and_the_anchor_keeps_their_lines() {
    // Six lines over two templates: the mean is three, so the singleton is
    // rare and the template of five is not.
    let out = trex(&["templates", "--text", LOG, "--rare"]);
    assert_eq!(stdout(&out), "1  kernel: disk failure on /dev/sda\n");
    // An explicit cut: under 90% of the lines takes both, under 5 lines the
    // singleton alone, five not being fewer than five.
    let out = trex(&["templates", "--text", LOG, "--rare", "--cut", "90%"]);
    assert_eq!(stdout(&out).lines().count(), 2);
    let out = trex(&["templates", "--text", LOG, "--rare", "--cut", "5"]);
    assert_eq!(stdout(&out).lines().count(), 1);
    // The anchor holds at every token of a rare line, so the words of the
    // kernel line come back and no request's do.
    let pat = parse("@shape:rare \\W").expect("a rare anchor");
    let got: Vec<&str> = scan(&pat, LOG.as_bytes()).iter().map(|s| &LOG[s.range()]).collect();
    assert_eq!(got, vec!["kernel", "disk", "failure", "on"]);
    // A cut written into the pattern: under six lines every template is
    // rare and the five addresses come back; under 20% the request
    // template, at five of six lines, is not.
    let pat = parse("@shape:rare<6 \\I").expect("a counted cut");
    assert_eq!(scan(&pat, LOG.as_bytes()).len(), 5);
    let pat = parse("@shape:rare<20% \\I").expect("a share cut");
    assert!(scan(&pat, LOG.as_bytes()).is_empty());
}

#[test]
fn json_carries_the_records_and_the_rarity() {
    let out = trex(&["templates", "--text", LOG, "--json"]);
    let text = stdout(&out);
    assert!(
        text.contains(
            "{\"count\":5,\"records\":[0,1,2,3,4],\"template\":\"<ip> <word> <path> status <number> <duration>\",\"pattern\":\"\\\\I \\\\W \\\\L \\\"status\\\" \\\\N \\\\R\",\"rare\":false}"
        ),
        "{text}"
    );
    assert!(text.contains("{\"count\":1,\"records\":[5],\"template\":\"kernel: disk failure on /dev/sda\""), "{text}");
    assert!(text.trim_end().ends_with("\"rare\":true}]"), "{text}");
    // No second input, so no mark: shared and novel are a comparison and
    // there is nothing to compare against.
    assert!(!text.contains("\"mark\""), "{text}");
}

#[test]
fn a_malformed_reading_is_refused_naming_its_form() {
    let err = parse("@shape").expect_err("a reading is required");
    assert!(err.msg.contains("@shape:rare"), "{}", err.msg);
    let err = parse("@shape:common").expect_err("an unknown reading");
    assert!(err.msg.contains("use rare"), "{}", err.msg);
    let err = parse("@shape:rare<0").expect_err("a cut of zero");
    assert!(err.msg.contains("holds of nothing"), "{}", err.msg);
    let err = parse("@shape:rare<101%").expect_err("a share past 100%");
    assert!(err.msg.contains("at most 100%"), "{}", err.msg);
    let out = trex(&["templates", "--text", LOG, "--cut", "five"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--cut"));
}

/// The library's own reading of a log with a blank line in it.
const SHORT: &str = "10.0.0.1 GET /index.html status 200 12ms\n\
                     10.0.0.2 GET /about.html status 200 8ms\n\
                     10.0.0.1 POST /login status 302 40ms\n\
                     \n\
                     kernel: disk failure on /dev/sda\n";

#[test]
fn lines_group_by_silhouette_and_a_varying_position_is_a_slot() {
    use trex::templates::Mining;
    let m = Mining::mine(SHORT.as_bytes());
    assert_eq!(m.templates.len(), 2);
    assert_eq!(m.templates[0].count(), 3);
    assert_eq!(m.templates[0].records, vec![0, 1, 2]);
    // `status` is the same on every line and stays a literal; the rest
    // vary and become slots named by kind.
    assert_eq!(m.templates[0].readable(), "<ip> <word> <path> status <number> <duration>");
    assert_eq!(m.templates[0].pattern(), "\\I \\W \\L \"status\" \\N \\R");
    // A group of one line is all literals, spaced as the line was.
    assert_eq!(m.templates[1].readable(), "kernel: disk failure on /dev/sda");
    assert_eq!(m.templates[1].records, vec![4]);
    // The blank line has no template.
    assert_eq!(m.record_template, vec![Some(0), Some(0), Some(0), None, Some(1)]);
    assert_eq!(m.covered(), 4);
}

#[test]
fn rarity_is_read_against_the_mean_or_an_explicit_cut() {
    use trex::templates::{Mining, Rarity};
    let m = Mining::mine(SHORT.as_bytes());
    // Four lines over two templates: the mean is two, so the singleton is
    // rare and the template of three is not.
    assert!(!m.is_rare(0, Rarity::Mean));
    assert!(m.is_rare(1, Rarity::Mean));
    assert!(m.is_rare(0, Rarity::Fewer(4)));
    assert!(!m.is_rare(0, Rarity::Fewer(3)));
    // One line of four is 25%: rare under 30%, not under 25%.
    assert!(m.is_rare(1, Rarity::Share(3_000)));
    assert!(!m.is_rare(1, Rarity::Share(2_500)));
}

#[test]
fn a_cut_is_a_count_or_a_share_to_hundredths() {
    use trex::templates::Rarity;
    assert_eq!(Rarity::parse("5").expect("a count"), Rarity::Fewer(5));
    assert_eq!(Rarity::parse("1%").expect("a share"), Rarity::Share(100));
    assert_eq!(Rarity::parse("0.5%").expect("a share"), Rarity::Share(50));
    assert_eq!(Rarity::parse("12.25%").expect("a share"), Rarity::Share(1_225));
    assert_eq!(Rarity::parse("100%").expect("a share"), Rarity::Share(10_000));
    for (written, reason) in [
        ("0", "holds of nothing"),
        ("0%", "above 0%"),
        ("101%", "at most 100%"),
        ("1.234%", "to hundredths"),
        ("five", "not a count"),
    ] {
        let refusal = Rarity::parse(written).expect_err("a cut that must be refused");
        assert!(refusal.contains(reason), "{written:?}: {refusal}");
    }
    assert_eq!(Rarity::Share(50).label(), "<0.5%");
    assert_eq!(Rarity::Share(1_225).label(), "<12.25%");
    assert_eq!(Rarity::Fewer(5).label(), "<5");
    assert_eq!(Rarity::Mean.label(), "");
}

#[test]
fn files_pool_into_one_stream_of_lines() {
    let dir = std::env::temp_dir().join(format!("trex/templates-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let (head, tail) = LOG.split_at(LOG.find("10.0.0.3").expect("the fourth line"));
    std::fs::write(dir.join("a.log"), head).expect("write a.log");
    std::fs::write(dir.join("b.log"), tail).expect("write b.log");
    let name = dir.to_string_lossy().into_owned();
    let out = trex(&["templates", &name]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        stdout(&out),
        "5  <ip> <word> <path> status <number> <duration>\n1  kernel: disk failure on /dev/sda\n"
    );
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

/// A directory of the two logs `--against` compares, removed on drop.
struct Logs(std::path::PathBuf);

impl Logs {
    fn new(tag: &str) -> Logs {
        let dir = std::env::temp_dir().join(format!("trex/templates-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        // The first log has two shapes the second also has and one it does
        // not; the second has seen more names at the `disk` position, and
        // fewer at nothing.
        std::fs::write(
            dir.join("app.log"),
            "user bob logged in\n\
             user amy logged in\n\
             disk sda ok\n\
             disk sdb ok\n\
             quota 90 exceeded\n",
        )
        .expect("write app.log");
        std::fs::write(
            dir.join("other.log"),
            "user carl logged in\n\
             user dana logged in\n\
             user eve logged in\n\
             disk sda ok\n",
        )
        .expect("write other.log");
        Logs(dir)
    }

    fn trex(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .expect("run trex")
    }
}

impl Drop for Logs {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove temp dir");
    }
}

#[test]
fn against_marks_each_template_by_whether_the_other_log_would_accept_it() {
    let logs = Logs::new("against");
    let out = logs.trex(&["templates", "app.log", "--against", "other.log"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // `user <word> logged in` is shared: the other log's template has a slot
    // where this one does. `disk <word> ok` is novel: the other has seen only
    // `sda` there, so its template would not accept `sdb`. And nothing over
    // there resembles the quota line at all.
    assert_eq!(
        stdout(&out),
        "novel   2  disk <word> ok\n\
         shared  2  user <word> logged in\n\
         novel   1  quota 90 exceeded\n"
    );
    let out = logs.trex(&["templates", "app.log", "--against", "other.log", "--novel"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout(&out), "novel   2  disk <word> ok\nnovel   1  quota 90 exceeded\n");
    // The mark rides in the JSON too, and only where a second input was given.
    let out = logs.trex(&["templates", "app.log", "--against", "other.log", "--json"]);
    let text = stdout(&out);
    assert!(text.contains("\"template\":\"user <word> logged in\",\"pattern\":\"\\\"user\\\" \\\\W \\\"logged\\\" \\\"in\\\"\",\"rare\":false,\"mark\":\"shared\""), "{text}");
    assert!(text.contains("\"template\":\"disk <word> ok\",\"pattern\":\"\\\"disk\\\" \\\\W \\\"ok\\\"\",\"rare\":false,\"mark\":\"novel\""), "{text}");
    // Novel is a comparison, so it needs something to compare against.
    let out = logs.trex(&["templates", "app.log", "--novel"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--novel needs --against"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_record_unit_groups_records_rather_than_lines() {
    let dir = std::env::temp_dir().join(format!("trex/templates-records-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let stanzas = "job alpha\nstatus ok\ntook 12s\n\n\
                   job bravo\nstatus ok\ntook 30s\n\n\
                   job delta\nstatus failed\ntook 4s\n";
    std::fs::write(dir.join("stanzas.log"), stanzas).expect("write stanzas.log");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_trex"))
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run trex")
    };
    // By line the stanzas are three shapes; by paragraph they are one, whose
    // template spans the three lines of a record.
    let out = run(&["templates", "stanzas.log"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout(&out), "6  <word> <word>\n3  took <duration>\n");
    let out = run(&["templates", "stanzas.log", "--record", "paragraph"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout(&out), "3  job <word>\nstatus <word>\ntook <duration>\n");
    let out = run(&["templates", "stanzas.log", "--record", "paragraph", "--pattern"]);
    assert_eq!(stdout(&out), "3  \"job\" \\W \"status\" \\W \"took\" \\R\n");
    // A pattern may say where a record starts, as it may for a query.
    let out = run(&["templates", "stanzas.log", "--record-start", "\"job\""]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout(&out), "3  job <word>\nstatus <word>\ntook <duration>\n");
    // The record numbers in the JSON are records, not lines.
    let out = run(&["templates", "stanzas.log", "--record", "paragraph", "--json"]);
    assert!(stdout(&out).contains("\"count\":3,\"records\":[0,1,2]"), "{}", stdout(&out));
    let out = run(&["templates", "stanzas.log", "--record", "nonsense"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("is not a record unit"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}
