//! The single-quoted strings the lexer makes tokens of in a file: how many,
//! how many tokens they hold in place of, and each of the first ones with
//! the bytes around it, so a reading of a corpus's stream can be made
//! string by string.
//!
//! Usage: `single_quotes <file> [shown]`.

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: single_quotes <file> [shown]");
        std::process::exit(2);
    };
    let shown: usize = args.next().map_or(40, |s| s.parse().expect("a count of strings to show"));
    let input = std::fs::read(&path).expect("the file is readable");
    let toks = trex::lexer::lex(&input);
    let quoted: Vec<&trex::token::Token> =
        toks.iter().filter(|t| t.kind == trex::token::TokenKind::Quoted && input[t.start()] == b'\'').collect();
    // Every single-quoted token by length, the char literals among them
    // being the ones of three and four bytes.
    let mut by_len = [0usize; 10];
    for t in &quoted {
        by_len[(t.end() - t.start()).min(9)] += 1;
    }
    let strings: Vec<&trex::token::Token> = quoted.iter().copied().filter(|t| t.end() - t.start() > 4).collect();
    let inner_tokens: usize = strings.iter().map(|t| trex::lexer::lex(&input[t.start() + 1..t.end() - 1]).len()).sum();
    println!(
        "{path}: {} bytes, {} tokens, {} single-quoted strings over four bytes holding {} bytes that lex alone as {} tokens",
        input.len(),
        toks.len(),
        strings.len(),
        strings.iter().map(|t| t.end() - t.start()).sum::<usize>(),
        inner_tokens
    );
    println!(
        "  single-quoted tokens by length: 2:{} 3:{} 4:{} 5:{} 6:{} 7:{} 8:{} 9+:{}",
        by_len[2], by_len[3], by_len[4], by_len[5], by_len[6], by_len[7], by_len[8], by_len[9]
    );
    for t in strings.iter().take(shown) {
        let a = t.start().saturating_sub(24);
        let b = (t.end() + 24).min(input.len());
        let before = String::from_utf8_lossy(&input[a..t.start()]).replace('\n', "\\n");
        let string = String::from_utf8_lossy(&input[t.start()..t.end()]).replace('\n', "\\n");
        let after = String::from_utf8_lossy(&input[t.end()..b]).replace('\n', "\\n");
        println!("  {:>9}  {before}[{string}]{after}", t.start());
    }
}
