//! Where two token dumps of one file differ: every region where the tokens
//! of one build are not the tokens of the other, with the bytes around it,
//! and the regions by size, so a change to the lexer is read region by
//! region and a long swallow cannot hide behind a count.
//!
//! Usage: `token_diff <file> <old dump> <new dump> [shown]`, the dumps as
//! `token_dump` writes them.

/// A token as the dump names it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tok {
    start: usize,
    end: usize,
    kind: String,
}

fn read_dump(path: &str) -> Vec<Tok> {
    let text = std::fs::read_to_string(path).expect("the dump is readable");
    text.lines()
        .map(|line| {
            let mut parts = line.splitn(3, ' ');
            let start = parts.next().expect("a start offset").parse().expect("a numeric start offset");
            let end = parts.next().expect("an end offset").parse().expect("a numeric end offset");
            let kind = parts.next().expect("a kind").to_string();
            Tok { start, end, kind }
        })
        .collect()
}

/// The tokens of one dump that the other does not hold, in order.
fn only_in(a: &[Tok], b: &[Tok]) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut j = 0;
    for t in a {
        while j < b.len() && (b[j].start, b[j].end) < (t.start, t.end) {
            j += 1;
        }
        if !(j < b.len() && b[j] == *t) {
            out.push(t.clone());
        }
    }
    out
}

/// The tokens as `Kind[text]`, a long text cut to its head and its length.
fn show(input: &[u8], toks: &[Tok]) -> String {
    toks.iter()
        .map(|t| {
            let text = String::from_utf8_lossy(&input[t.start..t.end]).replace('\n', "\\n");
            let text = if text.len() > 40 {
                let head: String = text.chars().take(40).collect();
                format!("{head}...({} bytes)", t.end - t.start)
            } else {
                text
            };
            format!(" {}[{text}]", t.kind)
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [file, old_path, new_path, rest @ ..] = args.as_slice() else {
        eprintln!("usage: token_diff <file> <old dump> <new dump> [shown]");
        std::process::exit(2);
    };
    let shown: usize = rest.first().map_or(30, |s| s.parse().expect("a count of regions to show"));
    let input = std::fs::read(file).expect("the file is readable");
    let old = read_dump(old_path);
    let new = read_dump(new_path);
    let old_only = only_in(&old, &new);
    let new_only = only_in(&new, &old);
    // Regions: the differing tokens of both sides, merged where they touch.
    let mut all: Vec<(usize, usize)> =
        old_only.iter().chain(new_only.iter()).map(|t| (t.start, t.end)).collect();
    all.sort_unstable();
    let mut regions: Vec<(usize, usize)> = Vec::new();
    for &(s, e) in &all {
        match regions.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => regions.push((s, e)),
        }
    }
    let mut sizes = [0usize; 5];
    for &(s, e) in &regions {
        let len = e - s;
        let bucket = if len <= 16 {
            0
        } else if len <= 64 {
            1
        } else if len <= 1024 {
            2
        } else if len <= 16384 {
            3
        } else {
            4
        };
        sizes[bucket] += 1;
    }
    println!(
        "{file}: {} old tokens, {} new tokens, {} only in old, {} only in new, {} differing regions",
        old.len(),
        new.len(),
        old_only.len(),
        new_only.len(),
        regions.len()
    );
    println!(
        "  regions by bytes: <=16:{} <=64:{} <=1k:{} <=16k:{} longer:{}",
        sizes[0], sizes[1], sizes[2], sizes[3], sizes[4]
    );
    let mut largest: Vec<(usize, usize)> = regions.clone();
    largest.sort_by_key(|&(s, e)| std::cmp::Reverse(e - s));
    let first: Vec<(usize, usize)> = regions.iter().take(shown).copied().collect();
    let largest: Vec<(usize, usize)> = largest.into_iter().take(10).collect();
    for (label, list) in [("first", first), ("largest", largest)] {
        println!("  --- {label} regions ---");
        for (s, e) in list {
            let a = s.saturating_sub(30);
            let b = (e + 30).min(input.len());
            let ctx = String::from_utf8_lossy(&input[a..b]).replace('\n', "\\n");
            let ctx: String = if ctx.chars().count() > 200 { ctx.chars().take(200).collect::<String>() + "..." } else { ctx };
            let olds: Vec<Tok> = old_only.iter().filter(|t| t.start >= s && t.end <= e).cloned().collect();
            let news: Vec<Tok> = new_only.iter().filter(|t| t.start >= s && t.end <= e).cloned().collect();
            println!("  {s}..{e} ({} bytes): {ctx}", e - s);
            println!("      old ({}):{}", olds.len(), show(&input, &olds[..olds.len().min(12)]));
            println!("      new ({}):{}", news.len(), show(&input, &news[..news.len().min(12)]));
        }
    }
}
