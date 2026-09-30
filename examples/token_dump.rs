//! Every token of a file, one a line as `start end kind`, so two builds of
//! the lexer can be compared token by token with a file comparison.
//!
//! Usage: `token_dump <file>`.

use std::io::Write;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: token_dump <file>");
        std::process::exit(2);
    };
    let input = std::fs::read(&path).expect("the file is readable");
    let toks = trex::lexer::lex(&input);
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for t in &toks {
        writeln!(out, "{} {} {:?}", t.start(), t.end(), t.kind).expect("stdout is writable");
    }
    out.flush().expect("stdout is writable");
}
