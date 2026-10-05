//! How many of a corpus's anchors a pattern actually matches.
//!
//! The device path's leftmost non-overlapping selection walks the per-anchor
//! ends on one host thread, skipping past each match and stepping over each
//! miss, so what it costs depends on how many anchors it visits and how many
//! spans it writes. Compacting the matching anchors on the device shortens
//! that walk in proportion to how sparse the matches are, and this reports
//! that proportion rather than assuming it.
//!
//! For each pattern: the significant tokens (the anchors), the matches, the
//! matches per anchor, and the bytes the selection reads as ends against the
//! bytes it writes as spans.
//!
//! Run: `cargo run --release --example match_density -- [corpus]`. Without a
//! corpus it uses the tag-number lines the device benches build, where every
//! other anchor matches `\W \N` by construction, so a real corpus is the
//! reading that matters.

use trex::{parse, scan};

/// The kind-only patterns the device accepts, from one the tag-line corpus
/// matches densely to shapes that need a longer run to hit.
const PATTERNS: [&str; 6] =
    ["\\W \\N", "\\N \\N", "\\W \\W", "\\W \\N \\W", "\\W \\W \\W \\W", "\\N \\W \\N \\W"];

/// `pairs` lines of a word and a number, the corpus the device benches build.
fn tag_pairs(pairs: usize) -> Vec<u8> {
    let mut input = String::new();
    for i in 0..pairs {
        input.push_str("tag ");
        input.push_str(&(i % 1000).to_string());
        input.push('\n');
    }
    input.into_bytes()
}

/// The significant tokens of `bytes`: one anchor per token, which is what the
/// selection walks.
fn anchors(bytes: &[u8]) -> usize {
    use trex::token::TokenKind;
    trex::lexer::lex(bytes).iter().filter(|t| t.kind != TokenKind::Whitespace).count()
}

fn main() {
    let inputs: Vec<(String, Vec<u8>)> = match std::env::args().nth(1) {
        None => vec![("2097152 tag lines".to_string(), tag_pairs(2_097_152))],
        Some(path) => match std::fs::read(&path) {
            Ok(b) => vec![(path, b)],
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        },
    };

    for (name, input) in &inputs {
        let n = anchors(input);
        println!("{name}, {} bytes, {n} anchors", input.len());
        println!(
            "{:>16} {:>12} {:>16} {:>12} {:>12}",
            "pattern", "matches", "per anchor", "ends KB", "spans KB"
        );
        for src in PATTERNS {
            // Every pattern here is a constant of this file, so one that does
            // not parse is a defect in it rather than a corpus a run can skip.
            let pat = match parse(src) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!(
                        "the pattern {src:?} in PATTERNS does not parse at byte {}: {}",
                        e.pos, e.msg
                    );
                    std::process::exit(2);
                }
            };
            let matches = scan(&pat, input).len();
            let per = if n == 0 { 0.0 } else { matches as f64 / n as f64 };
            println!(
                "{src:>16} {matches:>12} {per:>15.4} {:>12} {:>12}",
                n * 4 / 1024,
                matches * 8 / 1024
            );
        }
        println!();
    }
    println!(
        "per anchor near 0.5 is one match every other anchor, where compacting the matching anchors saves the selection little; near zero is a sparse pattern, where the walk steps over almost every slot and compaction removes most of it. ends KB is what the selection reads, spans KB what it writes."
    );
}
