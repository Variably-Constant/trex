//! Nested and repeated captures: a register bound inside a bound pattern is
//! named through it and every reference inside follows; a register bound
//! under a repetition keeps every binding; templates, `--json` and the
//! library read both.

use std::process::{Command, Output};

use trex::{Template, captures, captures_with_lists, parse, rewrite, scan};

fn matches(pattern: &str, input: &str) -> Vec<trex::Match> {
    let p = parse(pattern).expect("valid pattern");
    let spans = scan(&p, input.as_bytes());
    captures_with_lists(&p, input.as_bytes(), &spans)
}

fn group<'a>(m: &trex::Match, name: &str, input: &'a str) -> &'a str {
    std::str::from_utf8(m.group(name, input.as_bytes()).expect("a register")).expect("utf-8")
}

fn list<'a>(m: &trex::Match, name: &str, input: &'a str) -> Vec<&'a str> {
    m.list(name).expect("a list register").iter().map(|s| &input[s.range()]).collect()
}

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_binding_inside_a_bound_pattern_nests_under_its_name() {
    let p = parse("(\\W:k \"=\" \\N:v):pair").expect("valid pattern");
    assert_eq!(p.capture_names(), ["pair", "pair.k", "pair.v"]);
    let input = "x = 1; y = 22";
    let ms = matches("(\\W:k \"=\" \\N:v):pair", input);
    assert_eq!(ms.len(), 2);
    assert_eq!(ms[0].names(), ["pair", "pair.k", "pair.v"]);
    assert_eq!(group(&ms[0], "pair", input), "x = 1");
    assert_eq!(group(&ms[0], "pair.k", input), "x");
    assert_eq!(group(&ms[1], "pair.v", input), "22");
    assert_eq!(ms[0].list("pair"), None, "bound under no repetition");
    // A pattern binding inside no binding is flat, as it always was.
    let flat = parse("<\\W:t>(.*):b</=t>").expect("valid pattern");
    assert_eq!(flat.capture_names(), ["t", "b"]);
    // A reference to a register bound outside the bound pattern is left as it is.
    let outer = "a a 1";
    let ms = matches("\\W:out (=out \\N:n):inner", outer);
    assert_eq!(ms.len(), 1);
    assert_eq!(group(&ms[0], "inner.n", outer), "1");
    assert_eq!(group(&ms[0], "out", outer), "a");
}

#[test]
fn a_let_used_under_two_names_nests_twice_and_its_back_reference_follows() {
    let mut shapes = trex::ShapeSet::new();
    shapes.declare_text("let twice = \\W:w =w\n").expect("declares");
    let p = trex::parser::parse_with_shapes("\\{twice}:a \"and\" \\{twice}:b", &shapes).expect("valid pattern");
    assert_eq!(p.capture_names(), ["a", "a.w", "b", "b.w"]);
    let input = "go go and run run";
    let spans = scan(&p, input.as_bytes());
    let ms = captures(&p, input.as_bytes(), &spans);
    assert_eq!(ms.len(), 1, "{spans:?}");
    // The cheaper resolution reads the last binding alone, and a register
    // bound under no repetition has no list under either.
    assert_eq!(captures_with_lists(&p, input.as_bytes(), &spans)[0].list("a.w"), None);
    assert_eq!(group(&ms[0], "a", input), "go go");
    assert_eq!(group(&ms[0], "a.w", input), "go");
    assert_eq!(group(&ms[0], "b.w", input), "run");
    // A template reaches a nested register as `${a.w}`, with accessors.
    let t = Template::parse("${a.w:upper}/${b.w}", &p.capture_names()).expect("a template");
    assert_eq!(rewrite(&p, &t, input.as_bytes()), b"GO/run");
    assert_eq!(ms[0].group("w", input.as_bytes()), None, "the flat name is gone");
}

#[test]
fn a_register_under_a_repetition_keeps_every_binding() {
    let input = "a = 1; b = 2; c = 3;";
    let ms = matches("(\\W:k \"=\" \\N:v \";\")+", input);
    assert_eq!(ms.len(), 1);
    let m = &ms[0];
    assert_eq!(list(m, "k", input), ["a", "b", "c"]);
    assert_eq!(list(m, "v", input), ["1", "2", "3"]);
    assert_eq!(group(m, "k", input), "c", "the flat reading is the last binding");
    // Resolved without the lists, the same match holds its last bindings and no list.
    let p = parse("(\\W:k \"=\" \\N:v \";\")+").expect("valid pattern");
    let cheap = captures(&p, input.as_bytes(), &scan(&p, input.as_bytes()));
    assert_eq!(group(&cheap[0], "k", input), "c");
    assert_eq!(cheap[0].list("k"), None);
    let listed: Vec<&str> = m.lists().map(|(name, _)| name).collect();
    assert_eq!(listed, ["k", "v"]);
    let p = parse("(\\W:k \"=\" \\N:v \";\")+").expect("valid pattern");
    assert_eq!(p.list_registers(), ["k", "v"]);
    let t = Template::parse("${k[0]}-${v[2]}-${k}", &p.capture_names()).expect("a template");
    assert_eq!(rewrite(&p, &t, input.as_bytes()), b"a-3-c");
    // Past the last binding a reference renders nothing.
    let t = Template::parse("<${k[5]}>", &p.capture_names()).expect("a template");
    assert_eq!(rewrite(&p, &t, input.as_bytes()), b"<>");
    // A register bound once is no list, and a bounded repeat of one is none.
    assert!(parse("\\W:k \\N{1,1}:v").expect("valid").list_registers().is_empty());
    assert_eq!(parse("(\\W:k){2,4}").expect("valid").list_registers(), ["k"]);
}

#[test]
fn a_repeated_bound_group_nests_and_lists_together() {
    let input = "a = 1; b = 2; c = 3;";
    let ms = matches("((\\W:k \"=\" \\N:v):pair \";\")+", input);
    let m = &ms[0];
    assert_eq!(m.names(), ["pair", "pair.k", "pair.v"]);
    assert_eq!(list(m, "pair", input), ["a = 1", "b = 2", "c = 3"]);
    assert_eq!(list(m, "pair.k", input), ["a", "b", "c"]);
    let p = parse("((\\W:k \"=\" \\N:v):pair \";\")+").expect("valid pattern");
    let t = Template::parse("${pair[1].k}=${pair.v[2]};${pair[0]}", &p.capture_names()).expect("a template");
    assert_eq!(rewrite(&p, &t, input.as_bytes()), b"b=3;a = 1");
    let e = Template::parse("${pair[0].k[1]}", &p.capture_names()).expect_err("two indexes");
    assert!(e.msg.contains("carries two indexes"), "{}", e.msg);
    let e = Template::parse("${pair[x]}", &p.capture_names()).expect_err("no number");
    assert!(e.msg.contains("an index is a number"), "{}", e.msg);
    let e = Template::parse("${0[1]}", &p.capture_names()).expect_err("the whole");
    assert!(e.msg.contains("takes no index"), "{}", e.msg);
}

#[test]
fn a_star_index_writes_every_binding_joined_with_a_comma() {
    let input = "a = 1; b = 2; c = 3;";
    let p = parse("(\\W:k \"=\" \\N:v \";\")+").expect("valid pattern");
    let t = Template::parse("${k[*]}|${v[*]:upper}|${k}", &p.capture_names()).expect("a template");
    assert_eq!(rewrite(&p, &t, input.as_bytes()), b"a,b,c|1,2,3|c");
    // A register bound once is its one binding.
    let once = parse("\\W:k \"=\" \\N:v").expect("valid pattern");
    let t = Template::parse("[${v[*]}]", &once.capture_names()).expect("a template");
    assert_eq!(rewrite(&once, &t, b"x = 7"), b"[7]");
    // A list field as the pattern builder spells one.
    let arrow = parse("(\\I):ip (\"-\" \">\" (\\I):ip)*").expect("valid pattern");
    let t = Template::parse("${ip[*]}", &arrow.capture_names()).expect("a template");
    assert_eq!(rewrite(&arrow, &t, b"10.0.0.1 -> 10.0.0.2 -> 10.0.0.3"), b"10.0.0.1,10.0.0.2,10.0.0.3");
    let e = Template::parse_report("${@magnitude[*]}", &p.capture_names()).expect_err("an axis");
    assert!(e.msg.contains("already joins every reading"), "{}", e.msg);
}

#[test]
fn json_reports_the_tree_and_the_lists_and_a_flat_pattern_as_before() {
    // The tree and the lists are as they were. What changed is that a
    // register binding one typed kind carries its parsed value beside its
    // text: `v` binds a number and reports one, while `k` binds a word,
    // which has no single value, and stays the bare string it always was.
    let out = trex(&["scan", "\\W:k \"=\" \\N:v", "--json", "--text", "x = 1"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "[{\"start\":0,\"end\":5,\"text\":\"x = 1\",\"captures\":{\"k\":\"x\",\"v\":{\"text\":\"1\",\"value\":1}}}]\n"
    );
    let out = trex(&["scan", "(\\W:k \"=\" \\N:v):pair", "--json", "--text", "x = 1"]);
    assert_eq!(
        stdout(&out),
        "[{\"start\":0,\"end\":5,\"text\":\"x = 1\",\"captures\":{\"pair\":{\"text\":\"x = 1\",\"captures\":{\"k\":\"x\",\"v\":{\"text\":\"1\",\"value\":1}}}}}]\n"
    );
    let out = trex(&["scan", "((\\W:k \"=\" \\N:v):pair \";\")+", "--json", "--text", "a = 1; b = 2;"]);
    assert_eq!(
        stdout(&out),
        "[{\"start\":0,\"end\":13,\"text\":\"a = 1; b = 2;\",\"captures\":{\"pair\":[{\"text\":\"a = 1\",\"captures\":{\"k\":\"a\",\"v\":{\"text\":\"1\",\"value\":1}}},{\"text\":\"b = 2\",\"captures\":{\"k\":\"b\",\"v\":{\"text\":\"2\",\"value\":2}}}]}}]\n"
    );
    let out = trex(&["scan", "(\\W:k \"=\" \\N:v \";\")+", "--json", "--text", "a = 1; b = 2;"]);
    assert_eq!(
        stdout(&out),
        "[{\"start\":0,\"end\":13,\"text\":\"a = 1; b = 2;\",\"captures\":{\"k\":[\"a\",\"b\"],\"v\":[{\"text\":\"1\",\"value\":1},{\"text\":\"2\",\"value\":2}]}}]\n"
    );
    let out = trex(&["scan", "((\\W:k \"=\" \\N:v):pair \";\")+", "--format", "${pair[0].k}..${pair[1].v}", "--text", "a = 1; b = 2;"]);
    assert_eq!(stdout(&out), "a..2\n");
    let out = trex(&["count-by", "((\\W:k \"=\" \\N:v):pair \";\")+", "${pair[0].k}", "--text", "a = 1; b = 2;\na = 3;\n"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("a"), "{}", stdout(&out));
}
