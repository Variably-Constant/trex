//! Joins across inputs: `@echoed:@other` and `@novel:@other` keep the
//! tokens whose content occurs, or does not occur, in a second input, keyed
//! exactly or up to an orbit rung, the second input read once per pattern.

use std::process::{Command, Output};

use trex::{ShapeSet, parse, parse_with_inputs, scan};

/// Four requests, two of them sharing an id with the billing log, one of
/// those in another case.
const REQUESTS: &str = "req fa3b start\nreq c91d start\nreq fa3b end\nreq E5E5 end\n";
const BILLING: &str = "billing c91d ok\nbilling e5e5 ok\n";

fn found(pattern: &str, input: &str, others: &[(&str, &[u8])]) -> Vec<String> {
    let pat = parse_with_inputs(pattern, &ShapeSet::new(), others)
        .unwrap_or_else(|e| panic!("{pattern}: {e:?}"));
    scan(&pat, input.as_bytes()).iter().map(|s| input[s.range()].to_string()).collect()
}

#[test]
fn echoed_in_another_input_keeps_the_content_both_share() {
    let others: &[(&str, &[u8])] = &[("billing.log", BILLING.as_bytes())];
    assert_eq!(found("@echoed:@billing.log \\W", REQUESTS, others), vec!["c91d"]);
    // Keyed exactly, `E5E5` and `e5e5` are two contents; an orbit scope
    // folds the case and they are one.
    assert_eq!(
        found("(?orbit:case @echoed:@billing.log \\W)", REQUESTS, others),
        vec!["c91d", "E5E5"]
    );
    // `@novel` against the other input is the complement over keyed tokens.
    let novel = found("@novel:@billing.log \\W", REQUESTS, others);
    assert_eq!(novel.len(), 11, "{novel:?}");
    assert!(!novel.iter().any(|w| w == "c91d"));
    // Two anchors naming one input compose, and the in-stream readings are
    // unchanged beside them.
    assert_eq!(
        found("@echoed:@billing.log \\W @novel:@billing.log \\W", REQUESTS, others),
        vec!["c91d start"]
    );
    assert_eq!(found("@echoed \\W", REQUESTS, &[]).len(), 10);
    // In-stream `@novel` is the first sighting of each content, which is a
    // different reading from content the other input lacks.
    assert_eq!(
        found("@novel \\W", REQUESTS, &[]),
        vec!["req", "fa3b", "start", "c91d", "end", "E5E5"]
    );
}

#[test]
fn a_quoted_name_may_hold_a_space_and_a_bare_one_ends_at_a_bracket() {
    let others: &[(&str, &[u8])] = &[("my billing.log", BILLING.as_bytes())];
    assert_eq!(
        found("(?orbit:case @echoed:@\"my billing.log\" \\W)", REQUESTS, others),
        vec!["c91d", "E5E5"]
    );
    let others: &[(&str, &[u8])] = &[("billing.log", BILLING.as_bytes())];
    assert_eq!(found("(@echoed:@billing.log) \\W", REQUESTS, others), vec!["c91d"]);
}

#[test]
fn a_name_is_a_file_beside_the_working_directory_and_a_missing_one_is_named() {
    let dir = std::env::temp_dir().join(format!("trex/joins-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("requests.log"), REQUESTS).expect("write requests.log");
    std::fs::write(dir.join("billing.log"), BILLING).expect("write billing.log");
    let out: Output = Command::new(env!("CARGO_BIN_EXE_trex"))
        .args(["scan", "@echoed:@billing.log \\W", "requests.log"])
        .current_dir(&dir)
        .output()
        .expect("run trex");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    assert_eq!(text, "[19..23] \"c91d\"\n");
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
    let err = parse("@echoed:@no-such-file.log \\W").expect_err("a missing input");
    assert!(err.msg.contains("no-such-file.log"), "{}", err.msg);
    let err = parse("@echoed:billing.log \\W").expect_err("a name needs its @");
    assert!(err.msg.contains("@echoed:@other.log"), "{}", err.msg);
}
