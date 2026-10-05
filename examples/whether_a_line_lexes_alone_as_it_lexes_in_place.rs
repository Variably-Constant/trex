//! Whether each line of a real file lexes alone into the tokens the whole-file
//! lex gives it, and where it does not, what the two readings disagree on.
//!
//! The blob gate reads a rolling entropy window that trails the cursor, so a
//! position is classified partly by the bytes before it. A whole-file lex
//! carries that window across a line end; a line lexed alone starts it empty.
//! The two agree wherever the window's history does not change a verdict, and
//! this counts the lines where it does.
//!
//! Each file is read as `trex scan` reads it (`trex::encoding::decode`).
//!
//! Run: `cargo run --release --example whether_a_line_lexes_alone_as_it_lexes_in_place -- <file>...`

use trex::token::{Token, TokenKind};

/// How many differing lines to show in full per file.
const SHOWN: usize = 3;

/// The significant tokens of `toks` as (kind, bytes) pairs, offsets shifted by
/// `base` into `bytes`.
fn named<'a>(toks: &[Token], bytes: &'a [u8], base: usize) -> Vec<(TokenKind, &'a [u8])> {
    toks.iter()
        .filter(|t| t.is_significant())
        .map(|t| (t.kind, &bytes[base + t.start as usize..base + t.end as usize]))
        .collect()
}

fn shown(tok: Option<&(TokenKind, &[u8])>) -> String {
    match tok {
        Some((kind, text)) => {
            let text = String::from_utf8_lossy(text);
            let head: String = text.chars().take(40).collect();
            let more = if text.chars().count() > 40 { "..." } else { "" };
            format!("{kind:?} {head:?}{more}")
        }
        None => "nothing".into(),
    }
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: whether_a_line_lexes_alone_as_it_lexes_in_place <file>...");
        std::process::exit(2);
    }
    let mut unreadable = 0usize;
    for path in &paths {
        let raw = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{path}: unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };
        let bytes = trex::encoding::decode(raw);
        let whole = trex::lexer::lex(&bytes);
        let sig: Vec<&Token> = whole.iter().filter(|t| t.is_significant()).collect();

        let mut lines = 0usize;
        let mut differ = 0usize;
        let mut alone_total = 0usize;
        let mut in_place_total = 0usize;
        let mut examples: Vec<String> = Vec::new();
        let mut at = 0usize;
        let mut k = 0usize;
        for line in bytes.split(|&b| b == b'\n') {
            let (lo, hi) = (at, at + line.len());
            at = hi + 1;
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            lines += 1;
            // The whole-file tokens that start on this line. A token that
            // starts on an earlier line and runs into this one belongs to that
            // line, and would show there as a difference.
            while k < sig.len() && (sig[k].start as usize) < lo {
                k += 1;
            }
            let from = k;
            while k < sig.len() && (sig[k].start as usize) < hi {
                k += 1;
            }
            let in_place: Vec<(TokenKind, &[u8])> =
                sig[from..k].iter().map(|t| (t.kind, &bytes[t.start as usize..t.end as usize])).collect();
            let alone = named(&trex::lexer::lex(line), &bytes, lo);
            alone_total += alone.len();
            in_place_total += in_place.len();
            if alone != in_place {
                differ += 1;
                if examples.len() < SHOWN {
                    let first = alone.iter().zip(&in_place).position(|(a, b)| a != b).unwrap_or(alone.len().min(in_place.len()));
                    examples.push(format!(
                        "     line at byte {lo}: {} tokens alone, {} in place; first difference at token {first}: alone {}, in place {}",
                        alone.len(),
                        in_place.len(),
                        shown(alone.get(first)),
                        shown(in_place.get(first))
                    ));
                }
            }
        }
        println!(
            "== {path}: {lines} lines, {differ} lex differently alone; {alone_total} significant tokens alone against {in_place_total} in place"
        );
        for e in &examples {
            println!("{e}");
        }
    }
    if unreadable > 0 {
        std::process::exit(1);
    }
}
