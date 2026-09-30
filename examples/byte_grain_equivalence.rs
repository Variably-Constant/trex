//! For which atoms does a byte-regular translation give trex's own spans, and
//! on what input does it stop?
//!
//! A byte-grain matcher answers a token pattern without lexing, which is the
//! only path measured to reach the regex crate's rate: trex's lex alone is
//! 3.03 ms on the comparison corpus where regex answers the whole operation in
//! 2.16. The translation is only usable where it is PROVABLE, though, because a
//! byte answer that differs from the token answer is not a faster answer to the
//! same question - it is an answer to a different one.
//!
//! `\W` does not mean "an identifier run". It means "an identifier run as the
//! lexer cuts it", and the lexer also decides blobs by entropy, treats a
//! non-ASCII non-letter as one whole token however many bytes it spans, and
//! carries quoting rules with escapes and unclosed strings. Each of those is a
//! place the byte class and the token can part company.
//!
//! So this puts each pattern's byte translation beside trex over corpora built
//! to stress exactly those decisions, and prints where the spans differ rather
//! than whether they do. A divergence is the finding; agreement on a corpus
//! that cannot produce one is not.
//!
//! Run: `cargo run --release --example byte_grain_equivalence`.

use regex::Regex;

/// A corpus and what about the lexer it is built to exercise.
fn corpora() -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();

    let mut plain = String::new();
    for i in 0..400u32 {
        plain.push_str(&format!("let value_{i} = {} ; call_{i}(alpha, beta) ;\n", i * 37));
    }
    out.push(("plain code", plain));

    // A high-entropy run the blob gate takes as one token, where a byte class
    // sees a long identifier.
    let mut blob = String::from("let payload = ");
    for i in 0..900u32 {
        blob.push_str(&format!("{}{}", "aZ9kQ2mX7pL4vB8n", i % 7));
    }
    blob.push_str(" ;\nlet after = 1 ;\n");
    out.push(("a high-entropy blob", blob));

    // A non-ASCII non-letter is one whole token spanning its UTF-8 bytes.
    let mut uni = String::new();
    for i in 0..200u32 {
        uni.push_str(&format!("let x{i} = \u{201c}quoted\u{201d} \u{2014} caf\u{e9} \u{1f600} ;\n"));
    }
    out.push(("unicode punctuation and letters", uni));

    // A quote that never closes encloses the rest of the file.
    let mut unclosed = String::new();
    for i in 0..200u32 {
        unclosed.push_str(&format!("let a{i} = \"closed\" ; let b{i} = 2 ;\n"));
    }
    unclosed.push_str("let broken = \"never closed ; let after = 3 ;\n");
    for i in 0..200u32 {
        unclosed.push_str(&format!("let c{i} = 4 ;\n"));
    }
    out.push(("an unclosed quote", unclosed));

    // Letters and digits adjacent, where the two grains could cut differently.
    let mut glued = String::new();
    for i in 0..300u32 {
        glued.push_str(&format!("let a{i}b = 12ab34 ; let _{i} = 0x{i:x} ; let z = {i}e{i} ;\n"));
    }
    out.push(("digits glued to letters", glued));

    // Spans the typed recognizers take before the default classifiers ever see
    // them. `try_typed_token` runs ahead of both the digit branch and the word
    // branch, so a translation that writes out only a number and a word reads
    // these as several tokens where the lexer reads one.
    let mut typed = String::new();
    for i in 0..200u32 {
        typed.push_str(&format!(
            "get https://host{i}.example/p?q={i} from a{i}@mail.example at 10.0.{}.{} on 2026-09-17T12:{:02}:00Z ;\n",
            i % 250,
            (i * 7) % 250,
            i % 60
        ));
    }
    out.push(("spans the typed recognizers take", typed));

    out
}

/// The lexer's dispatch order, as one byte alternation with every branch named.
///
/// Reading down `lex_inner`: a quoted run comes before anything that could
/// start inside it, a number before a word because `is_ascii_digit` is tested
/// one branch earlier, and a single byte of punctuation last.
///
/// An atom does not translate to a byte class, and that is the whole point of
/// the shape here. A class on its own reads spans the lexer never cut, because
/// an earlier branch would have taken them - so a pattern translates to one
/// named branch of this, and a span counts for that pattern only where its
/// branch is the one that took the span.
///
/// Whitespace is deliberately not a branch, and `punct` takes any single
/// remaining character, so every non-space character belongs to some match and
/// two consecutive matches are two consecutive significant tokens. That is what
/// lets a two-atom pattern be read off the stream at all.
///
/// Three of the lexer's decisions are missing from this on purpose: the blob
/// gate, which decides by entropy and is not regular; the typed recognizers,
/// which run ahead of both the number and the word branch; and the char-literal
/// and single-quoted forms. The corpora hold an input for each, so the harness
/// prints where that parts company instead of passing for want of an input that
/// reaches it.
const TOKENIZER: &str = concat!(
    r#"(?P<quoted>"(?:[^"\\]|\\.)*(?:"|$))"#,
    r"|(?P<number>[0-9]+(?:\.[0-9]+)?)",
    r"|(?P<word>[_A-Za-z\p{Alphabetic}][A-Za-z0-9_]*(?:\p{Alphabetic}[A-Za-z0-9_]*)*)",
    r"|(?P<punct>[^\s])",
);

/// The branch names, in the alternation's own order.
const BRANCHES: [&str; 4] = ["quoted", "number", "word", "punct"];

/// How a pattern's spans are read out of the stream the alternation cuts.
enum Rule {
    /// Every span that branch took.
    Branch(&'static str),
    /// Every span that branch took whose whole text this matches.
    Like(&'static str, &'static str),
    /// Every adjacent pair whose first is that branch and whose second is
    /// exactly this text, spanning from one to the other.
    Then(&'static str, &'static str),
}

/// A trex pattern and the branch of the tokenizer it is claimed to equal.
const PAIRS: [(&str, &str, Rule); 6] = [
    ("a word token", "\\W", Rule::Branch("word")),
    ("a number token", "\\N", Rule::Branch("number")),
    ("a literal word", "\"alpha\"", Rule::Like("word", "^alpha$")),
    ("a word then punctuation", "\\W \"=\"", Rule::Then("word", "=")),
    ("a byte pattern in a token", "`value_[0-9]+`", Rule::Like("word", "^value_[0-9]+$")),
    ("a quoted token", "\\Q", Rule::Branch("quoted")),
];

/// The stream the alternation cuts: each span with the branch that took it.
fn tokenize(re: &Regex, text: &str) -> Vec<(usize, usize, &'static str)> {
    re.captures_iter(text)
        .map(|caps| {
            let whole = caps.get(0).expect("a match has a group zero");
            let name = BRANCHES
                .iter()
                .copied()
                .find(|n| caps.name(n).is_some())
                .expect("every branch of the alternation is named");
            (whole.start(), whole.end(), name)
        })
        .collect()
}

/// The spans of one branch whose text the expression matches whole.
fn like_spans(
    toks: &[(usize, usize, &'static str)],
    text: &str,
    branch: &str,
    like: &Regex,
) -> Vec<(usize, usize)> {
    toks.iter()
        .filter(|t| t.2 == branch && like.is_match(&text[t.0..t.1]))
        .map(|t| (t.0, t.1))
        .collect()
}

/// The spans a rule reads out of that stream.
fn spans_for(rule: &Rule, toks: &[(usize, usize, &'static str)], text: &str) -> Vec<(usize, usize)> {
    match rule {
        Rule::Branch(b) => toks.iter().filter(|t| t.2 == *b).map(|t| (t.0, t.1)).collect(),
        Rule::Like(b, pat) => {
            let like = Regex::new(pat).expect("the rule's own expression parses");
            like_spans(toks, text, b, &like)
        }
        Rule::Then(b, lit) => toks
            .windows(2)
            .filter(|w| w[0].2 == *b && &text[w[1].0..w[1].1] == *lit)
            .map(|w| (w[0].0, w[1].1))
            .collect(),
    }
}

fn main() {
    println!("trex {}", trex::version());
    println!(
        "Each cell is trex's spans against the byte translation's. `same` means a byte-grain\n\
         matcher could answer that pattern on that input without lexing; anything else is\n\
         where the translation stops being provable.\n"
    );
    println!("{:>28} {:>26} {:>9} {:>9}  verdict", "pattern", "corpus", "trex n", "regex n");

    let tokenizer = Regex::new(TOKENIZER).expect("the tokenizer alternation parses");
    // Cut once a corpus rather than once a pair: every pattern reads the same
    // stream, which is the claim being tested.
    // One tokenized corpus: its name, its text, and each token as a start, an
    // end and the branch of the tokenizer alternation that took it.
    type CutCorpus<'a> = (&'a str, String, Vec<(usize, usize, &'static str)>);
    let cut: Vec<CutCorpus<'_>> = corpora()
        .into_iter()
        .map(|(cname, text)| {
            let toks = tokenize(&tokenizer, &text);
            (cname, text, toks)
        })
        .collect();

    let mut provable = 0usize;
    let mut broken: Vec<(&str, &str, String)> = Vec::new();
    for (pname, tp, rule) in &PAIRS {
        let p = match trex::parse(tp) {
            Ok(p) => p,
            Err(e) => {
                println!("{pname:>28} {:>26} {e:?}", "-");
                continue;
            }
        };
        for (cname, text, toks) in &cut {
            let input = text.as_bytes();
            let t: Vec<(usize, usize)> =
                trex::scan(&p, input).iter().map(|s| (s.start(), s.end())).collect();
            let r: Vec<(usize, usize)> = spans_for(rule, toks, text);
            let verdict = if t == r {
                provable += 1;
                "same".to_string()
            } else {
                // The first place they part, which is what names the rule that
                // does not carry over. A count alone says they differ and not
                // where, and where is the whole question.
                let at = t.iter().zip(&r).position(|(a, b)| a != b);
                let detail = match at {
                    Some(i) => {
                        let (ts, te) = t[i];
                        let (rs, re_) = r[i];
                        format!(
                            "differ at {i}: trex {ts}..{te} {:?}, bytes {rs}..{re_} {:?}",
                            String::from_utf8_lossy(&input[ts..te.min(ts + 24)]),
                            String::from_utf8_lossy(&input[rs..re_.min(rs + 24)])
                        )
                    }
                    None => format!("agree for {} then one side stops", t.len().min(r.len())),
                };
                broken.push((*pname, *cname, detail.clone()));
                detail
            };
            println!("{pname:>28} {cname:>26} {:>9} {:>9}  {verdict}", t.len(), r.len());
        }
    }

    println!(
        "\n{provable} of {} cells agree. The ones that do not are the rule that does not carry:",
        PAIRS.len() * cut.len()
    );
    for (p, c, d) in &broken {
        println!("  {p} on {c}\n    {d}");
    }
    if broken.is_empty() {
        println!("  none - which means these corpora did not reach a lexer decision that parts them");
    }
}
