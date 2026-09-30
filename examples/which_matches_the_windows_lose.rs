//! Which matches the window route loses or adds against the scan, for one
//! pattern over each file named: every span the scan found that the route did
//! not, and the reverse, each with the bytes around it, so a lost match is read
//! at its place in the input rather than counted. With `TREX_TRACE` set the
//! route prints each span it settled to beside them, in order.
//!
//! The route is asked with its gate open, so a corpus the gate would decline
//! is still read, and a decline for any other reason is printed as the
//! route's own reason.
//!
//! With a span `lo..hi` between the pattern and the files, that span of each
//! file is lexed as the route lexes a settled span and printed token by token
//! beside the whole input's tokens over the same bytes, so a span that lexes
//! apart from the whole is read at the token where the two part.
//!
//! Run: `cargo run --release --example which_matches_the_windows_lose -- <pattern> [lo..hi] <file>...`

use std::collections::BTreeSet;

use trex::token::Token;

/// The token as one line: kind, span, and the first bytes it holds.
fn show(bytes: &[u8], t: &Token) -> String {
    let (s, e) = (t.start(), t.end());
    let head = &bytes[s..e.min(s + 40)];
    format!("{:?} {s}..{e} {:?}", t.kind, String::from_utf8_lossy(head))
}

/// The token at `k` as one line, or a dash past the end.
fn show_at(bytes: &[u8], toks: &[Token], k: usize) -> String {
    match toks.get(k) {
        Some(t) => show(bytes, t),
        None => "-".to_string(),
    }
}

/// The route's lex of `lo..hi` against the whole lex's tokens inside it, up
/// to the first token where they part and a few after it.
fn compare_span(bytes: &[u8], lo: usize, hi: usize) {
    let whole: Vec<Token> = trex::parallel_lex::lex_parallel(bytes)
        .into_iter()
        .filter(|t| t.start() >= lo && t.end() <= hi)
        .collect();
    let mine = trex::prefilter::lex_span_as_a_window(bytes, lo, hi);
    println!(
        "  span {lo}..{hi}: the whole lex holds {} tokens inside it, the span's own lex {}",
        whole.len(),
        mine.len()
    );
    let part = whole
        .iter()
        .zip(&mine)
        .position(|(a, b)| (a.kind, a.start(), a.end()) != (b.kind, b.start(), b.end()));
    match part {
        Some(i) => {
            println!("  the two part at token {i}:");
            for k in i.saturating_sub(2)..(i + 4) {
                println!("    {k:>6}  whole {}", show_at(bytes, &whole, k));
                println!("    {k:>6}  span  {}", show_at(bytes, &mine, k));
            }
        }
        None => println!("  the two agree over the shorter of them"),
    }
    if mine.len() <= 24 {
        println!("  every token of the span's own lex:");
        for t in &mine {
            println!("    {}", show(bytes, t));
        }
    }
}

/// A number from one half of a `lo..hi` argument, or the program stops
/// saying which half did not read.
fn bound(text: &str, which: &str) -> usize {
    match text.parse::<usize>() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("the span's {which} `{text}` does not read as a number: {e}");
            std::process::exit(2);
        }
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: which_matches_the_windows_lose <pattern> [lo..hi] <file>...");
        std::process::exit(2);
    }
    let pattern = match trex::parse(&args[0]) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}: {e:?}", args[0]);
            std::process::exit(2);
        }
    };
    // A second argument shaped `lo..hi` is the span to compare.
    let mut span: Option<(usize, usize)> = None;
    if let Some((a, b)) = args[1].split_once("..") {
        span = Some((bound(a, "start"), bound(b, "end")));
        args.remove(1);
    }
    let mut unreadable = 0usize;
    let mut differing = 0usize;
    for path in &args[1..] {
        let raw = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{path}: unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };
        let bytes = trex::encoding::decode(raw);
        if let Some((lo, hi)) = span {
            if hi <= bytes.len() && lo < hi {
                compare_span(&bytes, lo, hi);
            } else {
                println!("{path}: the span {lo}..{hi} lies outside its {} bytes", bytes.len());
            }
        }
        let scanned: BTreeSet<(usize, usize)> =
            trex::scan(&pattern, &bytes).iter().map(|s| (s.start(), s.end())).collect();
        let Some(routed) = trex::prefilter::scan_required_windows_ungated(&pattern, &bytes) else {
            println!(
                "{path}: the route declined with its gate open: {}",
                trex::prefilter::required_window_reason(&pattern, &bytes)
            );
            continue;
        };
        let routed: BTreeSet<(usize, usize)> = routed.iter().map(|s| (s.start(), s.end())).collect();
        println!(
            "{path}: {} bytes, the scan found {}, the windows found {}",
            bytes.len(),
            scanned.len(),
            routed.len()
        );
        let around = |s: usize, e: usize| {
            let lo = s.saturating_sub(48);
            let hi = (e + 48).min(bytes.len());
            String::from_utf8_lossy(&bytes[lo..hi]).replace('\n', "\u{23ce}").replace('\r', "")
        };
        for &(s, e) in scanned.difference(&routed) {
            differing += 1;
            println!("  lost  {s}..{e}  {:?}  |{}|", String::from_utf8_lossy(&bytes[s..e]), around(s, e));
        }
        for &(s, e) in routed.difference(&scanned) {
            differing += 1;
            println!("  added {s}..{e}  {:?}  |{}|", String::from_utf8_lossy(&bytes[s..e]), around(s, e));
        }
    }
    if unreadable > 0 {
        std::process::exit(1);
    }
    if differing > 0 {
        std::process::exit(3);
    }
}
