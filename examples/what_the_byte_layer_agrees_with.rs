//! Whether a byte recognizer answers the same token the lexer does, over real
//! corpora, and what a pass over the bytes costs beside the lex it would stand
//! in front of.
//!
//! `byte_lex` builds a recognizer per kind and each header says it has the shape
//! of a `lexer::try_*` function. The two have never been asked the same question
//! over the same bytes. A route that returns a span the lexer would not have
//! made answers wrongly, and one that misses a span the lexer makes loses a
//! match, so which recognizers can carry a route is a reading rather than a
//! property of how they were written.
//!
//! Two questions, because a route can be wired two ways and the two are sound
//! under different conditions.
//!
//! A route that answers must agree exactly: every token the lexer makes of that
//! kind is one the recognizer claims at the same start and the same end, and the
//! recognizer claims nothing else. The `missed` and `extra` columns are that
//! test together.
//!
//! A route that refuses need only never miss. Where the recognizer finds nothing
//! anywhere in the input, no token of that kind is there and the scan answers
//! without a lex. `missed` alone is that test, and an `extra` costs a lex that
//! was going to happen.
//!
//! The cost column is the whole sweep, which is what a refusal pays in full: a
//! recognizer that matches nothing still reads every byte. It sits beside the
//! lex over the same bytes, because a sweep is worth taking only where it is
//! faster than the lex it stands in for.
//!
//! Every recognizer `byte_lex` builds is read, including the two the engine
//! already routes by hand. Those two are the scale the other ten are read
//! against, and dropping them would leave the table with nothing to be read
//! against.
//!
//! A reading here is a fact about the bytes it was read over, so the harness
//! takes corpus files and generates none.
//!
//! Run: `cargo run --release --example what_the_byte_layer_agrees_with -- <file>...`

use std::collections::BTreeMap;
use std::time::Instant;

use trex::byte_dfa::ByteDfa;
use trex::byte_lex;
use trex::byte_nfa::ByteNfa;
use trex::token::TokenKind;

/// One recognizer: its name, the lexer kind whose shape it claims, how it is
/// built, and the id it accepts under.
type Recognizer = (&'static str, TokenKind, fn() -> ByteNfa, u32);

/// Every recognizer `byte_lex` builds.
const RECOGNIZERS: [Recognizer; 12] = [
    ("number", TokenKind::Number, byte_lex::number, byte_lex::id::NUMBER),
    ("word", TokenKind::Word, byte_lex::word, byte_lex::id::WORD),
    ("url", TokenKind::Url, byte_lex::url, byte_lex::id::URL),
    ("jwt", TokenKind::Jwt, byte_lex::jwt, byte_lex::id::JWT),
    ("uuid", TokenKind::Uuid, byte_lex::uuid, byte_lex::id::UUID),
    ("mac", TokenKind::Mac, byte_lex::mac, byte_lex::id::MAC),
    ("hexcolor", TokenKind::HexColor, byte_lex::hexcolor, byte_lex::id::HEX_COLOR),
    ("percent", TokenKind::Percent, byte_lex::percent, byte_lex::id::PERCENT),
    ("base64", TokenKind::Base64, byte_lex::base64, byte_lex::id::BASE64),
    ("hash_digest", TokenKind::HashDigest, byte_lex::hash_digest, byte_lex::id::HASH_DIGEST),
    ("bytesize", TokenKind::ByteSize, byte_lex::bytesize, byte_lex::id::BYTE_SIZE),
    ("duration", TokenKind::Duration, byte_lex::duration, byte_lex::id::DURATION),
];

/// The two kinds the engine already reaches at the byte layer, through
/// `prefilter::byte_route_kind_reading` and its hand-written reader rather than
/// through any of this.
const ALREADY_ROUTED: [&str; 2] = ["number", "word"];

/// What the recognizer claims over the whole input, as the division a sweep
/// makes: every position it accepts at, with how far it reached.
///
/// A position it claims nothing at advances by one; a position it claims a token
/// at advances past that token.
fn sweep(dfa: &mut ByteDfa, input: &[u8]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut at = 0usize;
    while at < input.len() {
        match dfa.recognize(&input[at..]) {
            Some((len, _)) if len > 0 => {
                found.push((at, at + len));
                at += len;
            }
            _ => at += 1,
        }
    }
    found
}

/// How many of the lexer's tokens of this kind the recognizer does not answer at
/// the token's own start, which is how many matches a route would lose.
///
/// Asked per token rather than read out of the sweep. The sweep advances past
/// what it claims, so a token beginning inside a run it already took is one the
/// sweep never asks about, while a route asked at a caller's position does ask.
fn missed(dfa: &mut ByteDfa, input: &[u8], toks: &[(usize, usize)], id: u32) -> usize {
    let mut n = 0usize;
    for &(start, end) in toks {
        let answered = match dfa.recognize(&input[start..]) {
            Some((len, who)) => who == id && start + len == end,
            None => false,
        };
        if !answered {
            n += 1;
        }
    }
    n
}

/// The median of `times`, in milliseconds.
fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// One corpus against every recognizer.
fn report(path: &str, input: &[u8]) {
    let bytes = input.len() as f64;

    // The lex, timed the way the sweep is: warmed once, then the median of
    // several, so the two columns are read the same way.
    let toks = trex::lexer::lex(input);
    let mut lex_times = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        let again = trex::lexer::lex(input);
        lex_times.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(again.len(), toks.len(), "a repeat lex read the corpus differently");
    }
    let lex_ms = median(lex_times);

    let mut per_kind: BTreeMap<u32, Vec<(usize, usize)>> = BTreeMap::new();
    for t in &toks {
        per_kind.entry(t.kind.code()).or_default().push((t.start(), t.end()));
    }

    println!(
        "\ncorpus {path}, {} bytes, {} tokens, lex {lex_ms:.2} ms {:.2} ns/B",
        input.len(),
        toks.len(),
        lex_ms * 1e6 / bytes
    );
    println!(
        "{:>12} {:>9} {:>9} {:>8} {:>8} {:>10} {:>8} {:>7}  verdict",
        "recognizer", "lexer", "claimed", "missed", "extra", "sweep ms", "ns/B", "vs lex"
    );

    for (label, kind, build, id) in RECOGNIZERS {
        let want: &[(usize, usize)] =
            per_kind.get(&kind.code()).map_or(&[][..], |v| v.as_slice());

        let nfa = build();
        let mut dfa = ByteDfa::new(&nfa);

        // Warmed before timing, because the table is built as it is walked and
        // the first pass pays for states every later pass reuses. A route meets
        // both, and the cold cost is what `byte_grain_rate` reads.
        let claimed = sweep(&mut dfa, input);
        let mut times = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            let again = sweep(&mut dfa, input);
            times.push(started.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(again.len(), claimed.len(), "a warm sweep read the corpus differently");
        }
        let sweep_ms = median(times);

        let lost = missed(&mut dfa, input, want, id);

        // A claim the lexer did not make as a token of this kind. Harmless to a
        // route that only refuses, fatal to one that answers.
        let held: std::collections::HashSet<(usize, usize)> = want.iter().copied().collect();
        let extra = claimed.iter().filter(|s| !held.contains(s)).count();

        // What the table is read for. A recognizer that loses a match cannot
        // carry either wiring; one that agrees exactly can answer; one that only
        // never misses can refuse, which is the case that kills a scan over a
        // corpus holding none of its kind.
        let verdict = if lost > 0 {
            "loses matches"
        } else if extra == 0 && !want.is_empty() {
            "can answer"
        } else if extra == 0 && want.is_empty() {
            "nothing here to answer"
        } else {
            "can refuse"
        };
        let routed = if ALREADY_ROUTED.contains(&label) { " (routed today)" } else { "" };

        println!(
            "{label:>12} {:>9} {:>9} {lost:>8} {extra:>8} {sweep_ms:>10.2} {:>8.2} {:>6.2}x  {verdict}{routed}",
            want.len(),
            claimed.len(),
            sweep_ms * 1e6 / bytes,
            lex_ms / sweep_ms
        );
    }
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!(
            "name one or more corpus files: an agreement is a fact about the bytes it was read over"
        );
        std::process::exit(2);
    }

    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("{} corpora, {} recognizers each", paths.len(), RECOGNIZERS.len());

    for (i, path) in paths.iter().enumerate() {
        eprintln!("[{}/{}] reading {path}", i + 1, paths.len());
        match std::fs::read(path) {
            Ok(input) => report(path, &input),
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        }
    }

    println!(
        "\nA recognizer that loses a match cannot carry a route at all. One that agrees\n\
         exactly can answer a span without a lex. One that only never misses can still\n\
         refuse an input outright, which is the whole of the 100x-and-up losses the\n\
         campaign recorded: rows where trex lexes a corpus to agree it holds no match."
    );
}
