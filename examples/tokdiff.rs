//! Where two token dumps of one corpus differ, region by region.
//!
//! `tokdiff OLD NEW CORPUS [REGIONS]`: each dump is `start end kind` lines,
//! as `lex_scaling --dump` writes them. Tokens are matched on all three
//! fields; the tokens each side has that the other lacks are grouped into
//! regions by the bytes they cover, and each region is printed with the
//! corpus bytes around it and the tokens each side reads there. A summary
//! first counts the regions by the shape of the change, old kinds to new
//! kinds, so a change meant to touch one construct shows as one shape.

use std::collections::{BTreeMap, HashMap, HashSet};

/// A token as a dump line reads: its span and its kind's index in `kinds`.
type Tok = (u32, u32, u16);

fn offset(field: Option<&str>, what: &str, line: &str) -> u32 {
    let field = field.unwrap_or_else(|| panic!("no {what} in the dump line {line:?}"));
    field.parse().unwrap_or_else(|e| panic!("the {what} {field:?} in the dump line {line:?} is not an offset: {e}"))
}

fn load(path: &str, kinds: &mut Vec<String>, index: &mut HashMap<String, u16>) -> Vec<Tok> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    text.lines()
        .map(|line| {
            let mut fields = line.splitn(3, ' ');
            let s = offset(fields.next(), "start", line);
            let e = offset(fields.next(), "end", line);
            let k = fields.next().unwrap_or_else(|| panic!("no kind in the dump line {line:?}"));
            let id = *index.entry(k.to_string()).or_insert_with(|| {
                kinds.push(k.to_string());
                u16::try_from(kinds.len() - 1).expect("fewer kinds than u16 holds")
            });
            (s, e, id)
        })
        .collect()
}

struct Region {
    start: u32,
    end: u32,
    old: Vec<Tok>,
    new: Vec<Tok>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, old_path, new_path, corpus_path, rest @ ..] = args.as_slice() else {
        eprintln!("usage: tokdiff OLD NEW CORPUS [REGIONS]");
        std::process::exit(2);
    };
    let limit: usize = match rest.first() {
        None => 40,
        Some(x) => x.parse().unwrap_or_else(|e| panic!("REGIONS {x:?} is not a count: {e}")),
    };
    let mut kinds: Vec<String> = Vec::new();
    let mut index: HashMap<String, u16> = HashMap::new();
    let old = load(old_path, &mut kinds, &mut index);
    let new = load(new_path, &mut kinds, &mut index);
    let corpus = std::fs::read(corpus_path).unwrap_or_else(|e| panic!("cannot read {corpus_path}: {e}"));

    let in_old: HashSet<Tok> = old.iter().copied().collect();
    let in_new: HashSet<Tok> = new.iter().copied().collect();
    // Every token one side has and the other lacks, in span order, tagged
    // with the side that has it.
    let mut events: Vec<(Tok, bool)> = Vec::new();
    events.extend(old.iter().filter(|t| !in_new.contains(t)).map(|&t| (t, false)));
    events.extend(new.iter().filter(|t| !in_old.contains(t)).map(|&t| (t, true)));
    events.sort_unstable();
    let only_old = events.iter().filter(|(_, is_new)| !is_new).count();
    let only_new = events.len() - only_old;

    let mut regions: Vec<Region> = Vec::new();
    for (tok, is_new) in events {
        let (s, e, _) = tok;
        match regions.last_mut() {
            Some(r) if s <= r.end => {
                r.end = r.end.max(e);
                if is_new { r.new.push(tok) } else { r.old.push(tok) }
            }
            _ => {
                let mut r = Region { start: s, end: e, old: Vec::new(), new: Vec::new() };
                if is_new { r.new.push(tok) } else { r.old.push(tok) }
                regions.push(r);
            }
        }
    }
    println!(
        "tokens old {} new {}; only in old {only_old}, only in new {only_new}; regions {}",
        old.len(),
        new.len(),
        regions.len()
    );

    let names = |toks: &[Tok]| -> String {
        let v: Vec<&str> = toks.iter().map(|&(_, _, k)| kinds[usize::from(k)].as_str()).collect();
        if v.is_empty() { "-".to_string() } else { v.join(" ") }
    };
    let mut shapes: BTreeMap<(String, String), usize> = BTreeMap::new();
    for r in &regions {
        *shapes.entry((names(&r.old), names(&r.new))).or_insert(0) += 1;
    }
    let mut shapes: Vec<((String, String), usize)> = shapes.into_iter().collect();
    shapes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    println!("shapes (old kinds -> new kinds): regions");
    for ((o, n), c) in &shapes {
        println!("  {c:>8}  {o}  ->  {n}");
    }

    let corpus_len = u32::try_from(corpus.len()).expect("a corpus the dump offsets address");
    let text = |s: u32, e: u32| -> String {
        String::from_utf8_lossy(&corpus[s as usize..e as usize]).replace('\n', "\\n").replace('\r', "\\r")
    };
    let show = |toks: &[Tok]| -> String {
        toks.iter()
            .map(|&(s, e, k)| format!("{}[{}]", kinds[usize::from(k)], text(s, e)))
            .collect::<Vec<String>>()
            .join(" ")
    };
    for r in regions.iter().take(limit) {
        let a = r.start.saturating_sub(12);
        let b = (r.end + 12).min(corpus_len);
        println!("@{}..{}  {:?}", r.start, r.end, text(a, b));
        println!("   old: {}", show(&r.old));
        println!("   new: {}", show(&r.new));
    }
}
