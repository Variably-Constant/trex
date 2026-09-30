//! The explanation's readings as `--format` fields: `${@axis}` renders the
//! sentence `--explain` prints for that axis, `${@axis.piece}` one value of
//! it, and an index picks one of the tokens the match spans.
//!
//! Every expected string here is what the built binary printed, not what the
//! format strings in the source suggest it would.

use std::process::{Command, Output};

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

/// The rendered lines of a scan that must succeed.
fn rendered(args: &[&str]) -> String {
    let out = trex(args);
    assert!(out.status.success(), "{args:?}: {}", stderr(&out));
    stdout(&out)
}

/// The message a refused run printed, with the run's failure asserted.
fn refused(args: &[&str]) -> String {
    let out = trex(args);
    assert!(!out.status.success(), "{args:?} was accepted: {}", stdout(&out));
    stderr(&out)
}

#[test]
fn the_three_whole_match_names_read_the_explanation() {
    // The kinds the match spans, in order, joined.
    assert_eq!(
        rendered(&["scan", "\\W \\N", "--format", "${0} kinds=${@kind}", "--text", "code 200"]),
        "code 200 kinds=word, number\n"
    );
    // The guard a guarded kind passed to be that kind.
    assert_eq!(
        rendered(&[
            "scan",
            "\\{card}",
            "--format",
            "${@guard}",
            "--text",
            "pay 4111 1111 1111 1111 now",
        ]),
        "creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check\n"
    );
    // The route is the rung that answered, and it is the same rung
    // `--explain` names, because both read the one trace the scan recorded.
    for pattern in ["\\W", "\\M{>6}", "\\W \\N"] {
        let input = "a 5000000000 code 200";
        let explained = rendered(&["scan", pattern, "--explain", "--text", input]);
        let named = explained
            .lines()
            .find_map(|l| l.strip_prefix("  route: "))
            .unwrap_or_else(|| panic!("{pattern}: a route line in {explained}"));
        let formatted = rendered(&["scan", pattern, "--format", "${@route}", "--text", input]);
        assert_eq!(formatted.lines().next(), Some(named), "{pattern}");
    }
}

#[test]
fn an_axis_renders_the_sentence_explain_prints_and_a_dot_names_one_value() {
    // A reading of a single value needs no piece: the value is the reading.
    assert_eq!(
        rendered(&["scan", "\\M{>6}", "--format", "mag=${@magnitude}", "--text", "a 5000000000"]),
        "mag=9.70\n"
    );
    // The composite readings state several values, and each is named.
    assert_eq!(
        rendered(&[
            "scan",
            "\\N{>+1}",
            "--format",
            "mean=${@baseline.mean} spread=${@baseline.spread} n=${@baseline.count}",
            "--text",
            "1 2 3 4000",
        ]),
        "mean=0.26 spread=0.20 n=3\n"
    );
    assert_eq!(
        rendered(&[
            "scan",
            "@echo>1 \\W",
            "--format",
            "c=${@echo.count} nth=${@echo.occurrence} rung=${@echo.rung} whole=[${@echo}]",
            "--text",
            "alpha beta alpha",
        ]),
        "c=2 nth=1 rung=identity whole=[count 2, occurrence 1 of 2 at the identity rung]\n\
         c=2 nth=2 rung=identity whole=[count 2, occurrence 2 of 2 at the identity rung]\n"
    );
    assert_eq!(
        rendered(&[
            "scan",
            "@shape:rare \\W",
            "--format",
            "r=${@template.rarity} t=${@template.template}",
            "--text",
            "a 1\nb 2\nkernel: fail\n",
        ]),
        "r=rare t=kernel: fail\nr=rare t=kernel: fail\n"
    );
    // The two depths the axes tell apart, which one field name could not:
    // the bracket nesting a token sits at, and the enclosing unit's.
    assert_eq!(
        rendered(&[
            "scan",
            "@nested>1 \\W",
            "--format",
            "d=${@nesting.depth} whole=[${@nesting}]",
            "--text",
            "f(g(x))",
        ]),
        "d=2 whole=[depth 2]\n"
    );
    assert_eq!(
        rendered(&[
            "scan",
            "@super:call \\W",
            "--format",
            "role=${@construct.role} depth=${@construct.depth} begins=${@construct.begins}",
            "--text",
            "foo(a) bar(b)",
        ]),
        "role=call depth=0 begins=true\nrole=call depth=0 begins=true\n"
    );
    // The field index is the `@k` anchor's own count of the columns.
    assert_eq!(
        rendered(&["scan", "@3 \\W", "--format", "f=${@field.index}", "--text", "a,b,c,d"]),
        "f=3\n"
    );
    // An accessor applies to an axis as it does to a register.
    assert_eq!(
        rendered(&["scan", "\\W", "--format", "${@kind:upper}", "--text", "alpha"]),
        "WORD\n"
    );
}

#[test]
fn an_axis_reads_at_every_token_and_an_index_picks_one() {
    // Two tokens, both read for magnitude: the bare field joins the readings
    // and an index takes one. Past the last reading renders nothing, as a
    // register's index past its last binding does.
    assert_eq!(
        rendered(&[
            "scan",
            "\\M{>0} \\M{>0}",
            "--format",
            "all=[${@magnitude}] a=[${@magnitude[0]}] b=[${@magnitude[1]}] past=[${@magnitude[9]}]",
            "--text",
            "12 3400",
        ]),
        "all=[1.08, 3.53] a=[1.08] b=[3.53] past=[]\n"
    );
}

#[test]
fn an_axis_the_pattern_never_read_renders_empty() {
    // The axes are computed for the axes the pattern names and no other, so
    // a field naming one the pattern never read has nothing to report. It is
    // not an error: the axis exists, this pattern had no use for it.
    assert_eq!(
        rendered(&["scan", "\\W", "--format", "mag=[${@magnitude}]", "--text", "alpha"]),
        "mag=[]\n"
    );
}

#[test]
fn the_at_keeps_an_axis_out_of_the_registers_namespace() {
    // A pattern may bind `:kind`, and it does not shadow the axis or lose to
    // it: `${kind}` is the register under every pattern and `${@kind}` the
    // axis. Without the `@` one template text would mean two things.
    assert_eq!(
        rendered(&["scan", "\\W:kind", "--format", "reg=${kind} axis=${@kind}", "--text", "alpha"]),
        "reg=alpha axis=word\n"
    );
}

#[test]
fn a_name_that_reads_no_axis_is_refused_when_the_template_is_parsed() {
    let e = refused(&["scan", "\\W", "--format", "${@entrpoy}", "--text", "alpha"]);
    assert!(e.contains("${@entrpoy} reads no axis; the axes are kind, guard, route"), "{e}");
    assert!(e.contains("construct, phase, join, field"), "{e}");
    let e = refused(&["scan", "\\W", "--format", "${@magnitude.nope}", "--text", "alpha"]);
    assert!(e.contains("${@magnitude} states one value and takes no .nope after it"), "{e}");
    let e = refused(&["scan", "\\W", "--format", "${@spectral.nope}", "--text", "alpha"]);
    assert!(
        e.contains(
            "${@spectral.nope} reads nothing; spectral states entropy, period, strength, novelty, texture"
        ),
        "{e}"
    );
}

#[test]
fn an_axis_is_refused_where_nothing_can_compute_one() {
    // The bytes a rewrite splices in stand in the input it rewrote; there is
    // no explanation to read, so the axis is refused rather than rendered
    // empty into the file.
    let e = refused(&["rewrite", "\\W", "${@kind}", "--text", "alpha"]);
    assert!(e.contains("${@kind} reads an axis, which a scan's --format renders"), "{e}");
    assert!(e.contains("have no explanation to read"), "{e}");
    // A key groups matches. Rendering every axis empty would land every match
    // under one key, which is an answer rather than a failure, so it stops
    // before the scan.
    let e = refused(&["count-by", "\\W", "${@kind}", "--text", "alpha beta"]);
    assert!(e.contains("a KEY groups matches and reads no axis"), "{e}");
}

#[test]
fn explain_prints_what_it_always_did_beside_a_template_that_names_an_axis() {
    // The readings carry their pieces for a template to name, and the
    // sentence `--explain` prints is the same sentence it printed before,
    // in the text report and in `--json`.
    let text = rendered(&["scan", "\\M{>6}", "--explain", "--text", "a 5000000000"]);
    assert_eq!(
        text,
        "[2..12] \"5000000000\"\n  tokens: number \"5000000000\"\n  magnitude: 9.70 \"5000000000\"\n  route: the single-pass engine over a whole lex\n"
    );
    let json = rendered(&["scan", "\\M{>6}", "--explain", "--json", "--text", "a 5000000000"]);
    assert_eq!(
        json,
        "[{\"start\":2,\"end\":12,\"text\":\"5000000000\",\"captures\":{},\"explain\":{\"tokens\":[{\"kind\":\"number\",\"text\":\"5000000000\"}],\"guards\":[],\"readings\":[{\"axis\":\"magnitude\",\"text\":\"5000000000\",\"value\":\"9.70\"}],\"route\":\"the single-pass engine over a whole lex\"}}]\n"
    );
    // Both together: the template's line, then the explanation under it, off
    // the one explanation the match was given.
    let both =
        rendered(&["scan", "\\M{>6}", "--format", "mag=${@magnitude}", "--explain", "--text", "a 5000000000"]);
    assert_eq!(
        both,
        "mag=9.70\n  tokens: number \"5000000000\"\n  magnitude: 9.70 \"5000000000\"\n  route: the single-pass engine over a whole lex\n"
    );
}
