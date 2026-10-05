//! The infer command: the most specific pattern every example matches,
//! read off an alignment of the examples' tokens and verified against each
//! before it is printed.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use trex::{parse, scan};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn trex_with_stdin(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_trex"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn trex");
    child.stdin.take().expect("a piped stdin").write_all(stdin).expect("write stdin");
    child.wait_with_output().expect("wait for trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The inferred pattern for `examples`, checked to match each of them from
/// its first token to its last.
fn inferred(examples: &[&str]) -> String {
    let mut args = vec!["infer"];
    args.extend_from_slice(examples);
    let out = trex(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    let pattern = stdout(&out).trim_end().to_string();
    let pat = parse(&pattern).unwrap_or_else(|e| panic!("{pattern}: {e:?}"));
    for example in examples {
        let spans = scan(&pat, example.as_bytes());
        let whole = example.trim();
        let start = example.len() - example.trim_start().len();
        assert!(
            spans.iter().any(|s| s.range() == (start..start + whole.len())),
            "{pattern} does not match {example:?} whole: {spans:?}"
        );
    }
    pattern
}

#[test]
fn agreeing_kinds_are_atoms_and_agreeing_texts_are_literals() {
    assert_eq!(
        inferred(&["10.0.0.1 GET /index.html status 200 12ms", "10.0.0.2 POST /login status 302 40ms"]),
        "\\I \\W \\L \"status\" \\N \\R"
    );
    // Three examples narrow nothing that two agreed on and widen what the
    // third differs in.
    assert_eq!(
        inferred(&[
            "GET /index.html status 200",
            "GET /about.html status 200",
            "PUT /admin/users status 500"
        ]),
        "\\W \\L \"status\" \\N"
    );
}

#[test]
fn differing_kinds_become_a_class_of_the_kinds_seen() {
    assert_eq!(inferred(&["id 200 ok", "id abc ok"]), "\"id\" [\\N \\W] \"ok\"");
}

#[test]
fn a_run_whose_length_differs_takes_the_bounds_seen() {
    // The words both examples share stay literals; the words between them
    // are one run of one or two.
    assert_eq!(inferred(&["disk failure on sda", "disk failed sda"]), "\"disk\" \\W{1,2} \"sda\"");
    // The shared `b` pairs with itself and the three extra words are a run
    // the first example lacks.
    assert_eq!(inferred(&["a b c d", "a b b b b c d"]), "\"a\" \\W{0,3} \"b\" \"c\" \"d\"");
}

#[test]
fn a_position_some_examples_lack_is_optional_and_never_a_literal() {
    // `/a` is a slash and a word to the lexer; `cached` is a word only one
    // example has, so it is an optional word rather than a literal.
    assert_eq!(inferred(&["GET /a 200", "GET /a 200 cached"]), "\"GET\" \"/\" \"a\" \"200\" \\W?");
    assert_eq!(inferred(&["x = 1", "x = 1 ; y = 2"]), "\"x\" \"=\" \"1\" \\P? \\W? \\P? \\N?");
}

#[test]
fn anchored_adds_the_line_anchors_and_matches_whole_lines_only() {
    let out = trex(&["infer", "user bob logged in", "user amy logged in", "--anchored"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let pattern = stdout(&out).trim_end().to_string();
    assert_eq!(pattern, "^ \"user\" \\W \"logged\" $ \"in\"");
    let pat = parse(&pattern).expect("an anchored pattern");
    let log = "user bob logged in\nnote: user amy logged in today\n";
    let got: Vec<&str> = scan(&pat, log.as_bytes()).iter().map(|s| &log[s.range()]).collect();
    assert_eq!(got, vec!["user bob logged in"]);
}

#[test]
fn examples_come_from_a_file_or_the_standard_input() {
    let dir = std::env::temp_dir().join(format!("trex/infer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("examples.txt");
    std::fs::write(&path, "10.0.0.1 GET /index.html\r\n\r\n10.0.0.2 POST /login\r\n").expect("write");
    let name = path.to_string_lossy().into_owned();
    let out = trex(&["infer", "-f", &name]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "\\I \\W \\L\n");
    let out = trex_with_stdin(&["infer"], b"10.0.0.1 GET /index.html\n10.0.0.2 POST /login\n");
    assert_eq!(stdout(&out), "\\I \\W \\L\n");
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

/// Cumulative update titles as Windows 7 through 11 and Server 2022 write
/// them.
const TITLES: [&str; 7] = [
    "2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB5031354)",
    "2023-10 Cumulative Update for Windows 10 Version 22H2 for x64-based Systems (KB5031356)",
    "2023-09 Cumulative Update Preview for Windows 11 Version 22H2 for x64-based Systems (KB5030310)",
    "2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)",
    "2023-01 Security Monthly Quality Rollup for Windows 8.1 for x64-based Systems (KB5022352)",
    "Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)",
    "2023-10 Cumulative Update for Microsoft server operating system version 21H2 for x64-based Systems (KB5031364)",
];

const MARKED_TITLE: &str =
    "{month:2023-10} Cumulative Update for Windows {os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})";

#[test]
fn a_marked_line_builds_a_pattern_that_reads_every_field_of_every_shape() {
    let mut args = vec!["infer", "--mark", MARKED_TITLE, "--pattern"];
    args.extend_from_slice(&TITLES);
    let out = trex(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    let pattern = stdout(&out).trim_end().to_string();
    let text = TITLES.join("\n");
    let out = trex(&["scan", &pattern, "--text", &text, "--format", "${month}|${os}|${version}|${kb}"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "2023-10|11|22H2|KB5031354\n2023-10|10|22H2|KB5031356\n2023-09|11|22H2|KB5030310\n\
         2020-01|7||KB4534310\n2023-01|8.1||KB5022352\n|10|1607|KB4103720\n2023-10||21H2|KB5031364\n"
    );
}

#[test]
fn the_report_names_each_shape_and_what_every_line_reads() {
    let mut args = vec!["infer", "--mark", MARKED_TITLE];
    args.extend_from_slice(&TITLES);
    let out = trex(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    let report = stdout(&out);
    assert!(report.contains("format   ${month}\\t${os}\\t${version}\\t${kb}\n"), "{report}");
    assert!(report.contains("month os version kb marked"), "{report}");
    assert!(report.contains("month os kb by the literals of shape"), "{report}");
    let row = |line: &str| -> Vec<String> {
        let found = report.lines().find(|l| l.starts_with(&format!("{line} "))).expect("the line's row");
        found.split_whitespace().take(6).map(str::to_string).collect()
    };
    assert_eq!(row("6")[2..], ["-", "10", "1607", "KB4103720"]);
    assert_eq!(row("7")[2..], ["2023-10", "-", "21H2", "KB5031364"]);
}

#[test]
fn a_field_named_by_one_value_reaches_every_line_and_reports_as_json() {
    let mut args = vec!["infer", "--field", "kb=KB5031354", "--json"];
    args.extend_from_slice(&TITLES);
    let out = trex(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    let json = stdout(&out);
    for kb in ["KB5031354", "KB5030310", "KB4534310", "KB4103720", "KB5031364"] {
        assert!(json.contains(&format!("\"values\":{{\"kb\":\"{kb}\"}}")), "{kb}: {json}");
    }
    assert!(json.contains("\"format\":\"${kb}\""), "{json}");
}

#[test]
fn the_lib_file_passes_its_own_test_and_the_marked_lines_carry_their_marks() {
    let dir = std::env::temp_dir().join(format!("trex/infer-build-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let lines = dir.join("lines.txt");
    std::fs::write(&lines, "GET /a {status:200}\nGET /b 204\n").expect("write");
    let lines = lines.to_string_lossy().into_owned();
    let out = trex(&["infer", "--marked", "-f", &lines, "--not", "GET /c 500", "--lib-file"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let file = stdout(&out);
    assert!(file.contains("\\N{200..299}"), "{file}");
    let lib = dir.join("status.trex");
    std::fs::write(&lib, &file).expect("write");
    let lib = lib.to_string_lossy().into_owned();
    let out = trex(&["lib", "--test", &lib]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    let out = trex(&["scan", "\\{extract}", "--lib", &lib, "--text", "GET /x 201\nGET /y 503", "--format", "${status}"]);
    assert_eq!(stdout(&out), "201\n");
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_word_field_whose_values_share_a_constant_part_is_spelled_as_its_byte_shape() {
    let mut args = vec!["infer", "--mark", MARKED_TITLE, "--pattern"];
    args.extend_from_slice(&TITLES);
    let out = trex(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    let minted = stdout(&out).trim_end().to_string();
    assert!(minted.contains("(`KB[0-9]{7}`):kb"), "{minted}");
    args.push("--no-mint");
    let plain = stdout(&trex(&args)).trim_end().to_string();
    assert!(plain.contains("(\\W):kb"), "{plain}");
    let near = "2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB50313541)";
    let whole = "2023-11 Cumulative Update for Windows 11 Version 23H2 for x64-based Systems (KB5032190)";
    let text = format!("{near}\n{whole}");
    let out = trex(&["scan", &minted, "--text", &text, "--format", "${kb}"]);
    assert_eq!(stdout(&out), "KB5032190\n", "the byte shape refuses a KB of eight digits");
    let out = trex(&["scan", &plain, "--text", &text, "--format", "${kb}"]);
    assert_eq!(stdout(&out), "KB50313541\nKB5032190\n");
}

#[test]
fn mint_shapes_declares_the_byte_shape_beside_the_pattern_on_every_output() {
    let mut args = vec!["infer", "--mark", MARKED_TITLE, "--mint-shapes"];
    args.extend_from_slice(&TITLES);
    let report = trex(&args);
    assert!(report.status.success(), "{}", stderr(&report));
    assert!(stdout(&report).contains("declare  shape kb = `KB[0-9]{7}`\n"), "{}", stdout(&report));
    let json = stdout(&trex(&[args.as_slice(), &["--json"]].concat()));
    assert!(json.contains("\"declarations\":[\"shape kb = `KB[0-9]{7}`\"]"), "{json}");
    let out = trex(&[args.as_slice(), &["--pattern"]].concat());
    let pattern = stdout(&out).trim_end().to_string();
    assert!(pattern.contains("(\\{kb}):kb"), "{pattern}");
    assert!(stderr(&out).contains("--shape 'kb = `KB[0-9]{7}`'"), "{}", stderr(&out));
    let text = "2023-11 Cumulative Update for Windows 11 Version 23H2 for x64-based Systems (KB5032190)";
    let out = trex(&["scan", &pattern, "--shape", "kb = `KB[0-9]{7}`", "--text", text, "--format", "${kb}"]);
    assert_eq!(stdout(&out), "KB5032190\n", "{}", stderr(&out));
    let file = stdout(&trex(&[args.as_slice(), &["--lib-file"]].concat()));
    assert!(file.lines().nth(1) == Some("shape kb = `KB[0-9]{7}`"), "{file}");
    let out = trex(&[args.as_slice(), &["--no-mint"]].concat());
    assert!(!out.status.success());
    assert!(stderr(&out).contains("opposite spellings"), "{}", stderr(&out));
}

#[test]
fn the_builder_lexes_its_lines_under_the_shapes_lib_and_shape_declare() {
    let lines = ["--mark", "see {t:AB-12} now", "see XYZ-9 now", "--pattern"];
    let plain = stdout(&trex(&[&["infer"], &lines[..]].concat()));
    assert_eq!(plain, "^ \"see\" (\\W \"-\" \\N):t \"now\" ~<($ .)\n");
    let shaped = trex(&[&["infer", "--shape", "ticket = `[A-Z]{2,4}-\\d{1,4}`"], &lines[..]].concat());
    assert_eq!(stdout(&shaped), "^ \"see\" (\\{ticket}):t \"now\" ~<($ .)\n", "{}", stderr(&shaped));
    let dir = std::env::temp_dir().join(format!("trex/infer-lib-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let lib = dir.join("ticket.trex");
    std::fs::write(&lib, "shape ticket = `[A-Z]{2,4}-\\d{1,4}`\n").expect("write");
    let lib = lib.to_string_lossy().into_owned();
    let from_lib = trex(&[&["infer", "--lib", &lib], &lines[..]].concat());
    assert_eq!(stdout(&from_lib), stdout(&shaped));
    let built = dir.join("built.trex");
    let file = trex(&["infer", "--lib", &lib, "--mark", "see {t:AB-12} now", "see XYZ-9 now", "--lib-file"]);
    std::fs::write(&built, stdout(&file)).expect("write");
    let out = trex(&["lib", "--test", &lib, &built.to_string_lossy()]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    let out = trex(&["infer", "a 1", "b 2", "--lib", &lib]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("mark a field or give --field"), "{}", stderr(&out));
    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_mark_inside_a_mark_is_a_nested_capture_and_a_nested_json_object() {
    let out = trex(&["infer", "--mark", "{Line:{[int]n:1} of {[int]m:3}}", "5 of 9", "6 of 9"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let report = stdout(&out);
    assert!(report.starts_with("pattern  ^ ((\\N):n \"of\" (\\N):m):Line ~<($ .)\n"), "{report}");
    assert!(report.contains("format   ${Line}\\t${Line.n}\\t${Line.m}\n"), "{report}");
    let json = stdout(&trex(&["infer", "--mark", "{Line:{[int]n:1} of {[int]m:3}}", "5 of 9", "--json"]));
    assert!(json.contains("\"values\":{\"Line\":{\"text\":\"5 of 9\",\"n\":\"5\",\"m\":\"9\"}}"), "{json}");
    assert!(json.contains("{\"name\":\"Line.n\",\"accessor\":null,\"type\":\"int\""), "{json}");
    assert!(json.contains("\"parent\":\"Line\""), "{json}");
    let out = trex(&["scan", "((\\N):n \"of\" (\\N):m):Line", "--text", "7 of 8", "--format", "${Line.m}/${Line}"]);
    assert_eq!(stdout(&out), "8/7 of 8\n");
}

#[test]
fn a_field_a_library_shape_reads_as_one_token_is_spelled_as_that_shape() {
    let out = trex(&["infer", "--mark", "patched {cve:CVE-2023-1234} in {pkg:openssl}", "patched CVE-2024-56789 in zlib", "--pattern"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let pattern = stdout(&out).trim_end().to_string();
    assert_eq!(pattern, "^ \"patched\" (\\{cve}):cve \"in\" (\\W):pkg ~<($ .)");
    let out = trex(&["scan", &pattern, "--text", "patched CVE-2025-0001 in curl\npatched CVE-25-1 in curl", "--format", "${cve}"]);
    assert_eq!(stdout(&out), "CVE-2025-0001\n");
}

#[test]
fn a_library_value_class_is_suggested_until_a_counter_example_prints_it() {
    let report = stdout(&trex(&["infer", "--mark", "{level:ERROR} disk {n:5}", "WARN disk 7"]));
    assert!(report.contains("suggest  level as \\{log_level}\n"), "{report}");
    let json = stdout(&trex(&["infer", "--mark", "{level:ERROR} disk {n:5}", "WARN disk 7", "--json"]));
    assert!(json.contains("\"suggestions\":[{\"field\":\"level\",\"classes\":[\"log_level\"]}]"), "{json}");
    let out = trex(&["infer", "--mark", "{level:ERROR} disk {n:5}", "WARN disk 7", "--not", "HELLO disk 9", "--pattern"]);
    assert_eq!(stdout(&out), "^ (\\{log_level}):level \"disk\" (\\N):n ~<($ .)\n", "{}", stderr(&out));
}

#[test]
fn a_line_repeating_a_starred_record_gives_a_record_per_repeat() {
    let template = "{Name*:Phoebe Cat}, {[int]age:6}; {Name*:Lucky Shot}, {[int]age:12}";
    let out = trex(&["infer", "--mark", template, "Wise Owl, 87; Big Bird, 5", "Elmo Red, 3; Oscar Grouch, 9"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let report = stdout(&out);
    assert!(
        report.starts_with("pattern  ^ (\\W \\W):Name \",\" (\\N):age (\";\" (\\W \\W):Name \",\" (\\N):age)* ~<($ .)\n"),
        "{report}"
    );
    assert!(report.contains("format   ${Name[*]}\\t${age[*]}\n"), "{report}");
    assert!(report.contains("4       2      Big Bird      5\n"), "{report}");
    assert!(report.contains("6       3      Oscar Grouch  9\n"), "{report}");
    let json = stdout(&trex(&["infer", "--mark", template, "Wise Owl, 87; Big Bird, 5", "--json"]));
    assert!(json.contains("\"repeats\":true"), "{json}");
    assert!(json.contains("{\"lines\":[2],\"values\":{\"Name\":\"Big Bird\",\"age\":\"5\"}}"), "{json}");
}

#[test]
fn a_field_marked_twice_in_a_line_is_a_list_of_every_value_it_holds() {
    let hops = ["from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok", "from 10.0.0.3 ok"];
    let mut args = vec!["infer", "--mark", "from {ip:10.0.0.1} -> {ip:10.0.0.2} ok", "--json"];
    args.extend_from_slice(&hops);
    let out = trex(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    let json = stdout(&out);
    assert!(json.contains("\"format\":\"${ip[*]}\""), "{json}");
    assert!(json.contains("\"list\":true"), "{json}");
    assert!(json.contains("\"values\":{\"ip\":[\"10.0.0.7\",\"10.0.0.8\",\"10.0.0.9\"]}"), "{json}");
    assert!(json.contains("\"values\":{\"ip\":[\"10.0.0.3\"]}"), "{json}");
    let mut args = vec!["infer", "--mark", "from {ip:10.0.0.1} -> {ip:10.0.0.2} ok", "--pattern"];
    args.extend_from_slice(&hops);
    let out = trex(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    let pattern = stdout(&out).trim_end().to_string();
    let out = trex(&["scan", &pattern, "--text", "from 1.1.1.1 -> 2.2.2.2 ok\nfrom 3.3.3.3 ok", "--format", "${ip[*]}"]);
    assert_eq!(stdout(&out), "1.1.1.1,2.2.2.2\n3.3.3.3\n");
    // The report's table writes a list joined with a comma.
    let mut args = vec!["infer", "--mark", "from {ip:10.0.0.1} -> {ip:10.0.0.2} ok"];
    args.extend_from_slice(&hops);
    let report = stdout(&trex(&args));
    assert!(report.contains("10.0.0.7,10.0.0.8,10.0.0.9"), "{report}");
}

#[test]
fn a_value_appearing_twice_in_a_line_is_refused_with_the_line() {
    let out = trex(&["infer", "--field", "os=10", "2023-10 Cumulative Update for Windows 10 (KB5031356)"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("more than once"), "{err}");
    assert!(err.contains("line 1: 2023-10 Cumulative Update for Windows 10 (KB5031356)"), "{err}");
}

#[test]
fn too_few_or_empty_examples_are_refused() {
    let out = trex(&["infer", "only one"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("at least two"), "{}", stderr(&out));
    let out = trex(&["infer", "one", "   "]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("example 2 has no token"), "{}", stderr(&out));
}
