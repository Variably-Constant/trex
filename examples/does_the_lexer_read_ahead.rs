//! Whether a token the lexer has finished can still change with the bytes after
//! it, read on real files: the question every reading taken "after the last
//! token that ended before byte t" rests on.
//!
//! A coder that keys byte `t` on the last significant token ending strictly
//! before `t` is leak-free only if that token, and every one before it, is
//! decided by the bytes before `t`. The token covering `t` is excluded by
//! construction; a finished token whose extent or kind the lexer settled by
//! reading past its end is not. `does_the_group_delay_help_the_coder` and
//! `does_the_group_delay_select_the_weights` both take their readings under
//! that rule, so their figures are clean only as far as this finds nothing.
//!
//! This lexes the whole input and the prefix `input[..t]` at positions spread
//! through it, and compares the significant tokens ending before `t`. A prefix
//! holds every byte those tokens were read from, so a token of the whole lex
//! that is missing or different in the prefix is the lexer having read at or
//! past `t` to decide it - a read-ahead, and how far past the token's end is
//! printed beside it. The prefix may also hold a token of its own after the
//! last one they share, made from bytes the cut left unfinished (a multi-byte
//! char split, a quote whose close is past `t`); that is the prefix's tail,
//! not a decision about the whole lex's tokens, and is counted apart.
//!
//! Every input is a real file somebody produced for another purpose.
//!
//! Run: `cargo run --release --example does_the_lexer_read_ahead -- <file>...`

use std::path::Path;

use trex::token::Token;

/// Positions read a file, spread evenly through it.
const SAMPLES: usize = 256;

fn short(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn significant(input: &[u8]) -> Vec<Token> {
    trex::lexer::lex(input).into_iter().filter(Token::is_significant).collect()
}

fn same(a: &Token, b: &Token) -> bool {
    a.start() == b.start() && a.end() == b.end() && a.kind == b.kind
}

/// A token as a read-ahead is reported: its span, its kind and its bytes.
fn shown(input: &[u8], t: Option<&&Token>) -> String {
    match t {
        None => "none".to_string(),
        Some(t) => {
            let text = String::from_utf8_lossy(&input[t.start()..t.end().min(t.start() + 40)]);
            format!("{}..{} {:?} {text:?}", t.start(), t.end(), t.kind)
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: does_the_lexer_read_ahead <file>...");
        std::process::exit(2);
    }
    println!("significant tokens ending before t, lexed whole and lexed from input[..t] alone");
    println!("read ahead: a finished token of the whole lex missing or changed in the prefix");
    println!("tail only: the prefix's own token after every finished one matched");
    println!(
        "{:<22} {:>10} {:>8} {:>12} {:>10} {:>9} {:>12}",
        "input", "bytes", "samples", "tokens read", "read ahead", "tail only", "farthest"
    );
    // An input that cannot be read is a row missing from the table, and a
    // table missing a row reads as complete. So a run with one exits failing.
    let mut unreadable = 0usize;
    let mut misses: Vec<String> = Vec::new();
    for arg in &args {
        let path = Path::new(arg);
        let name = short(path);
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{name:<22} unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };
        let n = bytes.len();
        let whole = significant(&bytes);
        let (mut samples, mut read, mut ahead, mut tail) = (0usize, 0usize, 0usize, 0usize);
        // How far past a token's end the lexer read to decide it, at the most
        // this file showed: `t` less the end of the token that changed.
        let mut farthest = 0usize;
        for j in 1..=SAMPLES {
            let t = j * n / (SAMPLES + 1);
            if t == 0 {
                continue;
            }
            samples += 1;
            let prefix = significant(&bytes[..t]);
            let done_whole: Vec<&Token> = whole.iter().take_while(|x| x.end() < t).collect();
            let done_prefix: Vec<&Token> = prefix.iter().take_while(|x| x.end() < t).collect();
            read += done_whole.len();
            let changed = done_whole
                .iter()
                .enumerate()
                .find(|(k, a)| done_prefix.get(*k).is_none_or(|b| !same(a, b)))
                .map(|(k, _)| k);
            match changed {
                Some(k) => {
                    ahead += 1;
                    let reach = t - done_whole[k].end();
                    farthest = farthest.max(reach);
                    misses.push(format!(
                        "{name} at t {t}, {reach} bytes past the token's end: token {k} whole {} / prefix {}",
                        shown(&bytes, done_whole.get(k)),
                        shown(&bytes, done_prefix.get(k))
                    ));
                }
                None if done_prefix.len() > done_whole.len() => tail += 1,
                None => {}
            }
        }
        println!("{name:<22} {n:>10} {samples:>8} {read:>12} {ahead:>10} {tail:>9} {farthest:>12}");
    }
    if misses.is_empty() {
        println!("\nno finished token of the whole lex changed with the bytes after it at any position read");
    } else {
        println!("\nevery read-ahead found:");
        for line in &misses {
            println!("  {line}");
        }
    }
    if unreadable > 0 {
        eprintln!("{unreadable} of {} inputs could not be read", args.len());
        std::process::exit(1);
    }
}
