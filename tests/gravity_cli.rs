//! `trex gravity`: the pair field's readings at each grain, the first unit
//! reading neither strain nor bound, the ranked listings in order, a byte or
//! a type quoted where a sentence or a class list names it, and the
//! declarations refused at the byte grain.

use std::process::{Command, Output};

const LINE: &str = "let x = f(a) ; let y = g(b) ; print x";

fn trex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trex")).args(args).output().expect("run trex")
}

fn stdout(args: &[&str]) -> String {
    let out = trex(args);
    assert!(out.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

#[test]
fn the_summary_names_the_grain_its_units_and_the_extremes() {
    let out = stdout(&["gravity", "--text", LINE]);
    assert!(out.starts_with("trex gravity (token grain): 37 bytes, 18 tokens, 5 types, 0 gravity class(es)\n"), "{out}");
    assert!(out.contains("  most strain: 0.53 bits at 'x' (byte 4, percentile 94)\n"), "{out}");
    assert!(out.contains("  weakest cut: 0.26 bits per pair before 'x' (byte 36, percentile 0)\n"), "{out}");
}

#[test]
fn the_first_unit_reads_neither_strain_nor_bound() {
    let out = stdout(&["gravity", "--field", "--text", LINE]);
    assert!(out.contains("    @     0  let                  -     -        -     -   -\n"), "{out}");
    assert!(out.contains("    @     4  x                 0.53    94     0.27     6   -\n"), "{out}");
    assert_eq!(out.lines().filter(|l| l.starts_with("    @")).count(), 18, "{out}");
}

#[test]
fn the_listings_rank_the_readings_and_take_top() {
    let out = stdout(&["gravity", "--strain", "--bound", "--top", "2", "--text", LINE]);
    assert!(out.contains("  most strained (2 of 17 tokens with a reading):\n    @     4  x"), "{out}");
    assert!(out.contains("  weakest cuts (2 of 17), each before the token named:\n    @    36  x"), "{out}");
    let limited = stdout(&["gravity", "--field", "--limit", "3", "--text", LINE]);
    assert!(limited.contains("    ... (+15 more tokens; raise --limit)\n"), "{limited}");
}

#[test]
fn a_byte_or_a_type_is_quoted_in_a_sentence_and_in_a_class_list() {
    // More distinct bytes than the geometry has classes, so it places some,
    // the comma, the double quote, the newline and the space among them.
    let text = "fn main() {\n    let x = compute(1, 2);\n    println!(\"{x}\");\n}\n".repeat(200);
    let out = stdout(&["gravity", "--grain", "byte", "--classes", "--text", &text]);
    assert!(out.contains("  most strain: -0.74 bits at ' ' (byte 23, percentile 100)\n"), "{out}");
    assert!(out.contains("  weakest cut: 1.38 bits per pair before 'l' (byte 16, percentile 0)\n"), "{out}");
    let listed: Vec<&str> = out.lines().filter_map(|l| l.split_once(" bytes of ").map(|(_, types)| types)).collect();
    assert_eq!(listed.len(), 16, "{out}");
    let types: Vec<&str> = listed.iter().flat_map(|l| l.split(", ")).collect();
    assert!(types.iter().all(|t| t.len() >= 3 && t.starts_with('\'') && t.ends_with('\'')), "{types:?}");
    for t in ["','", "'\"'", "'\\n'", "' '"] {
        assert!(types.contains(&t), "{t} among {types:?}");
    }
}

#[test]
fn every_grain_reads_and_the_byte_grain_takes_no_declarations() {
    assert!(stdout(&["gravity", "--grain", "byte", "--text", LINE]).starts_with("trex gravity (byte grain): 37 bytes,"));
    assert!(stdout(&["gravity", "--grain", "super", "--text", LINE]).starts_with("trex gravity (super grain): 37 bytes,"));
    let out = trex(&["gravity", "--grain", "byte", "--shape", "pair = `[a-z] = [a-z]`", "--text", LINE]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("the byte grain reads bytes"));
}
