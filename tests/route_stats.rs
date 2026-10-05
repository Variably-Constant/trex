//! `--stats` beyond ripgrep's eight lines: the tokens lexed, how the
//! searching time split between lexing and matching, and which rung answered
//! for how many inputs.
//!
//! The route lines are asserted against what `TREX_TRACE` prints for the same
//! scan, so the two cannot drift apart: both read the same recorded rungs.

use std::path::PathBuf;
use std::process::{Command, Output};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn traced(args: &[&str]) -> Output {
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

/// Two small inputs under a directory of this test's own.
///
/// `named` is the caller's own name, because these run in parallel in one
/// process: a directory keyed only on the process id is one directory shared
/// by every test here, and the first to finish removes the corpus the rest
/// are still reading.
fn corpus(named: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("trex/routestats-{}-{named}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the corpus directory");
    std::fs::write(dir.join("a.log"), "code 200 from 10.0.0.1\ncode 404 from 10.0.0.2\n")
        .expect("write a.log");
    std::fs::write(dir.join("b.log"), "code 500 from 10.0.0.3\nnothing here\n").expect("write b.log");
    dir
}

/// The statistics block a scan prints, which is everything after the blank
/// line the report ends with.
fn stats(pattern: &str, dir: &str) -> String {
    let out = trex(&["scan", pattern, dir, "--stats", "--color", "never"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let at = text.find("\n\n").expect("a blank line before the statistics");
    text[at + 2..].to_string()
}

/// The value of the statistics line ending in `name`.
fn figure<'a>(block: &'a str, name: &str) -> &'a str {
    block
        .lines()
        .find(|l| l.ends_with(name))
        .map(|l| l[..l.len() - name.len()].trim())
        .unwrap_or_else(|| panic!("no {name:?} line in {block:?}"))
}

#[test]
fn the_eight_lines_ripgrep_prints_are_still_the_first_eight() {
    // A reader's habits and anything parsing the block both depend on these
    // eight opening it, in this order.
    let dir = corpus("eight");
    let block = stats("\\N", &dir.to_string_lossy());
    let heads: Vec<&str> = block.lines().take(8).collect();
    assert!(heads[0].ends_with(" matches"), "{heads:?}");
    assert!(heads[1].ends_with(" matched lines"), "{heads:?}");
    assert!(heads[2].ends_with(" files contained matches"), "{heads:?}");
    assert!(heads[3].ends_with(" files searched"), "{heads:?}");
    assert!(heads[4].ends_with(" bytes printed"), "{heads:?}");
    assert!(heads[5].ends_with(" bytes searched"), "{heads:?}");
    assert!(heads[6].ends_with(" seconds spent searching"), "{heads:?}");
    assert!(heads[7].ends_with(" seconds"), "{heads:?}");
    std::fs::remove_dir_all(&dir).expect("remove the corpus directory");
}

#[test]
fn the_tokens_and_the_split_of_the_searching_time_are_reported() {
    let dir = corpus("tokens");
    let dir_name = dir.to_string_lossy().into_owned();
    // A pattern the set engine walks over a whole lex, so there is a lex to
    // count and the count cannot be zero by construction.
    let block = stats("@echo>0 \\W", &dir_name);
    let tokens: u64 = figure(&block, " tokens lexed").parse().expect("a count of tokens");
    assert!(tokens > 0, "a scan over a whole lex lexed nothing: {block:?}");
    // The lexing time comes out of the searching time rather than adding to
    // it, so the two below sum to what the seventh line reports.
    let searching: f64 = figure(&block, " seconds spent searching").parse().expect("seconds");
    let lexing: f64 = figure(&block, " seconds spent lexing").parse().expect("seconds");
    let matching: f64 = figure(&block, " seconds spent matching").parse().expect("seconds");
    assert!(lexing <= searching, "lexing {lexing} over searching {searching}");
    // Each figure is printed to six decimals, so each carries up to half of
    // 1e-6 of rounding and three of them can differ by more than one. The
    // tolerance is that rounding and not a claim about the clock.
    assert!(
        (lexing + matching - searching).abs() <= 2e-6,
        "lexing {lexing} and matching {matching} do not sum to searching {searching}"
    );
    std::fs::remove_dir_all(&dir).expect("remove the corpus directory");
}

#[test]
fn the_route_lines_name_the_rungs_the_trace_prints() {
    let dir = corpus("routes");
    let dir_name = dir.to_string_lossy().into_owned();
    for pattern in ["\\N", "@echo>0 \\W", "\\M{>0}"] {
        // What the trace says answered, for the scan ladder only.
        let out = traced(&["scan", pattern, &dir_name, "--color", "never"]);
        assert!(out.status.success(), "{pattern}: {}", stderr(&out));
        let mut from_trace: Vec<String> = stderr(&out)
            .lines()
            .filter_map(|l| l.strip_prefix("trex route: scan over "))
            .filter_map(|l| l.split_once(" -> ").map(|(_, rung)| rung.to_string()))
            .collect();
        from_trace.sort();
        from_trace.dedup();
        assert!(!from_trace.is_empty(), "{pattern}: the trace named no scan rung");
        // What the statistics say answered, which must be the same names: the
        // two read one record, so a rung renamed in the engine moves both.
        let block = stats(pattern, &dir_name);
        let mut from_stats: Vec<String> = block
            .lines()
            .filter_map(|l| l.split_once(" files answered by ").map(|(_, rung)| rung.to_string()))
            .collect();
        from_stats.sort();
        from_stats.dedup();
        assert_eq!(from_stats, from_trace, "{pattern}");
    }
    std::fs::remove_dir_all(&dir).expect("remove the corpus directory");
}

#[test]
fn every_input_is_counted_against_exactly_one_rung() {
    let dir = corpus("accounted");
    let block = stats("@echo>0 \\W", &dir.to_string_lossy());
    let searched: u64 = figure(&block, " files searched").parse().expect("a count of files");
    let answered: u64 = block
        .lines()
        .filter_map(|l| l.split_once(" files answered by "))
        .map(|(count, _)| count.parse::<u64>().expect("a count of files"))
        .sum();
    assert_eq!(answered, searched, "the rungs do not account for every input: {block:?}");
    std::fs::remove_dir_all(&dir).expect("remove the corpus directory");
}

#[test]
fn explaining_and_counting_the_routes_do_not_consume_each_other() {
    // `--explain` takes the recorded rungs, so that the route it names is the
    // input it is explaining and not the whole run. Reading the same queue for
    // the statistics would report no routes whenever both flags are given,
    // and whichever ran first would decide which reading survived. The
    // statistics read running totals instead, which nothing takes.
    let dir = corpus("both");
    let dir_name = dir.to_string_lossy().into_owned();
    let out = trex(&["scan", "@echo>0 \\W", &dir_name, "--stats", "--explain", "--color", "never"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains(" files answered by "), "no route line beside --explain: {text:?}");
    assert!(text.contains("  route: "), "no explanation beside --stats: {text:?}");
    // And the route the explanation names is one of the rungs the statistics
    // counted, since both describe the same scan.
    let named: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("  route: ")).collect();
    let counted: Vec<&str> =
        text.lines().filter_map(|l| l.split_once(" files answered by ").map(|(_, r)| r)).collect();
    assert!(!named.is_empty() && !counted.is_empty(), "{text:?}");
    for route in &named {
        assert!(counted.contains(route), "{route:?} is explained but not counted: {counted:?}");
    }
    std::fs::remove_dir_all(&dir).expect("remove the corpus directory");
}

#[test]
fn the_one_line_form_stays_one_line() {
    let dir = corpus("oneline");
    let out = trex(&["scan", "\\N", &dir.to_string_lossy(), "--stats=line", "--color", "never"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let last = text.lines().next_back().expect("a line");
    assert!(last.contains("matches in"), "{last:?}");
    assert!(!text.contains("tokens lexed"), "the one-line form grew a block: {text:?}");
    std::fs::remove_dir_all(&dir).expect("remove the corpus directory");
}

#[test]
fn a_scan_that_asks_for_no_statistics_prints_none_of_this() {
    let dir = corpus("silent");
    let out = trex(&["scan", "\\N", &dir.to_string_lossy(), "--color", "never"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    for line in ["tokens lexed", "seconds spent lexing", "files answered by"] {
        assert!(!text.contains(line), "{line:?} printed without --stats: {text:?}");
    }
    std::fs::remove_dir_all(&dir).expect("remove the corpus directory");
}
