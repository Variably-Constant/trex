//! `--explain`: under each match, the kinds of the tokens it spans, the
//! guard each guarded kind passed, the value of every axis the pattern read
//! there, and the route that answered, which is the rung the trace records.

use std::process::{Command, Output};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn trex_traced(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex"))
        .args(args)
        .env("TREX_TRACE", "1")
        .output()
        .expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

#[test]
fn the_kinds_and_the_guard_are_named_under_the_match() {
    let out = trex(&["scan", "\\{card}", "--explain", "--text", "pay 4111 1111 1111 1111 now"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.starts_with(
            "[4..23] \"4111 1111 1111 1111\"\n  tokens: creditcard \"4111 1111 1111 1111\"\n  guard: creditcard: 13 to 19 digits"
        ),
        "{text}"
    );
    assert!(text.contains("passing the Luhn check"), "{text}");
    assert!(text.lines().any(|l| l.starts_with("  route: ")), "{text}");
    // A match of several tokens lists each, and an unguarded kind adds no
    // guard line.
    let out = trex(&["scan", "\\W \\N", "--explain", "--text", "code 200"]);
    let text = stdout(&out);
    assert!(text.contains("  tokens: word \"code\", number \"200\"\n  route: "), "{text}");
}

#[test]
fn the_route_is_the_rung_the_trace_records() {
    for pattern in ["\\W:w \"=\" \\N", "\"login\"", "\\I ~\"down\""] {
        let out = trex_traced(&["scan", pattern, "--explain", "--text", "x = 1 login 10.0.0.1 down"]);
        assert!(out.status.success(), "{pattern}: {}", stderr(&out));
        let traced = stderr(&out)
            .lines()
            .filter_map(|l| l.strip_prefix("trex route: scan over ").map(str::to_string))
            .next_back()
            .unwrap_or_else(|| panic!("{pattern}: a scan rung in the trace"));
        let rung = traced.split_once(" -> ").expect("an arrow after the size").1.to_string();
        let text = stdout(&out);
        let route = text
            .lines()
            .find_map(|l| l.strip_prefix("  route: "))
            .unwrap_or_else(|| panic!("{pattern}: a route line in {text}"));
        assert_eq!(route, rung, "{pattern}");
    }
}

#[test]
fn axis_readings_carry_the_value_the_predicate_compared() {
    let out = trex(&["scan", "@echo>1 \\W", "--explain", "--text", "alpha beta alpha"]);
    assert!(
        stdout(&out).contains("  echo: count 2, occurrence 1 of 2 at the identity rung \"alpha\""),
        "{}",
        stdout(&out)
    );
    let out = trex(&["scan", "\\M{>6}", "--explain", "--text", "a 5000000000"]);
    assert!(stdout(&out).contains("  magnitude: 9."), "{}", stdout(&out));
    let out = trex(&["scan", "@shape:rare \\W", "--explain", "--text", "a 1\nb 2\nkernel: fail\n"]);
    assert!(
        stdout(&out).contains("  template: rare under @shape:rare: `kernel: fail` on 1 of 3 lines \"kernel\""),
        "{}",
        stdout(&out)
    );
    let out = trex(&[
        "scan",
        "@order:desc \\T",
        "--explain",
        "--text",
        "at 2026-09-15T12:00:00Z then 2026-09-15T11:00:00Z",
    ]);
    assert!(
        stdout(&out).contains("  order: before the timestamp before it \"2026-09-15T11:00:00Z\""),
        "{}",
        stdout(&out)
    );
    let out = trex(&["scan", "\\N{>+1}", "--explain", "--text", "1 2 3 4000"]);
    let text = stdout(&out);
    assert!(text.contains("  magnitude: ") && text.contains("  baseline: window: mean "), "{text}");
}

#[test]
fn json_carries_the_explanation() {
    let out = trex(&["scan", "\\{card}", "--explain", "--json", "--text", "pay 4111 1111 1111 1111 now"]);
    let text = stdout(&out);
    assert!(
        text.contains(
            "\"captures\":{},\"explain\":{\"tokens\":[{\"kind\":\"creditcard\",\"text\":\"4111 1111 1111 1111\"}],\"guards\":[\"creditcard: "
        ),
        "{text}"
    );
    assert!(text.contains("\"readings\":[],\"route\":\""), "{text}");
}

#[test]
fn a_tree_report_explains_under_each_prefixed_line() {
    let dir = std::env::temp_dir().join(format!("trex/explain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("a.log"), "from 10.0.0.1\n").expect("write a.log");
    std::fs::write(dir.join("b.log"), "to 192.168.1.9\n").expect("write b.log");
    let name = dir.to_string_lossy().into_owned();
    let out = trex(&["scan", "\\I", &name, "--explain"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("a.log:1:6: \"10.0.0.1\"\n  tokens: ip \"10.0.0.1\"\n  route: "), "{text}");
    assert!(text.contains("b.log:1:4: \"192.168.1.9\"\n  tokens: ip \"192.168.1.9\"\n  route: "), "{text}");
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}
