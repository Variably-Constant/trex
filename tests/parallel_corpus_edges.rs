//! Spectral change points at a line of code spliced into a translation of
//! the parallel corpus. `_corpus/parallel/SOURCES.md` names the texts; the
//! corpus is in the repository and not the published crate, as this test is.

use std::fs;

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/_corpus/parallel");

/// `text` with `span` put in at the first boundary at or past byte `at` with
/// a letter on each side, a space before and after it, and 400 bytes of the
/// text kept past it; and where the span starts and ends.
fn spliced(text: &str, span: &str, at: usize) -> (String, usize, usize) {
    let mut i = at;
    while !(text.is_char_boundary(i)
        && text[..i].chars().next_back().is_some_and(char::is_alphabetic)
        && text[i..].chars().next().is_some_and(char::is_alphabetic))
    {
        i += 1;
    }
    let mut end = (i + 400).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let pre = format!("{} ", &text[..i]);
    let entry = pre.len();
    (format!("{pre}{span} {}", &text[i..end]), entry, entry + span.len())
}

#[test]
fn a_line_of_code_inside_japanese_prose_gets_an_entry_and_an_exit() {
    // The threshold's running mean and deviation take each byte's departure
    // only up to three deviations, so a divergence that climbs over many
    // bytes, as a line of code does inside Japanese, still passes it.
    let path = format!("{CORPUS}/prose/jpn.txt");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let (sample, entry, exit) = spliced(&text, "let mut total = scores.iter().sum::<u32>() / 3;", 1600);
    let cuts = trex::spectral::analyze(sample.as_bytes()).boundaries;
    assert!(cuts.iter().any(|&c| c + 4 >= entry && c <= entry + 32), "no entry near {entry}: {cuts:?}");
    assert!(cuts.iter().any(|&c| c + 4 >= exit && c <= exit + 32), "no exit near {exit}: {cuts:?}");
}
