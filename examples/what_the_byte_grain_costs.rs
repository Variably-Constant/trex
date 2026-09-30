//! What it costs to establish that a corpus holds no token of a given kind,
//! five ways, against what trex pays for that answer today.
//!
//! This is the question behind every loss the campaign recorded above 100x:
//! rows where trex lexes a whole corpus to agree it holds no match. The lex is
//! the cost, so the arms below are the ways of not paying it, and the thing
//! they are read against is [`trex::parallel_lex::lex_parallel`] - what a scan
//! actually pays - rather than the serial lexer, which no production path takes
//! on an input this size.
//!
//! ## Soundness first
//!
//! A refusal that is wrong is not a speed-up, so every recognizer is asked
//! whether it answers each token the lexer makes of its kind, at that token's
//! own start. `missed` is how many matches a route built on it would lose and
//! must be zero before any timing below means anything.
//!
//! ## The arms
//!
//! `lex` is the serial lexer and `lex par` the parallel one, both producing the
//! token stream a scan would then match over. The rest answer only "is there a
//! token of this kind anywhere", which is strictly less than the lex produces
//! and is all a refusal needs.
//!
//! `anchored` restarts the automaton at every byte, which is what
//! [`trex::byte_dfa::ByteDfa::recognize`] is for: it answers what the input
//! opens with. Sweeping with it costs a restart a byte.
//!
//! `one pass` puts `.*` in front of the recognizer so the start state never
//! dies, and reads the input once. The same question, one walk instead of n.
//!
//! `chunked` splits the input at [`trex::parallel_lex::safe_boundaries`] - the
//! boundaries the parallel lexer itself uses, each a guaranteed token boundary
//! with no string across it - and runs a whole one-pass walk per chunk through
//! Flynnel. Splitting at anything else would let a token straddle a cut and the
//! refusal would be a false negative; an even division is not available here
//! for that reason.
//!
//! `literal` is `byte_simd::contains` of the one byte string the kind's
//! recognizer refuses without, where it has one. It stops at the first
//! occurrence, so a common literal ends the search early and a corpus lacking
//! it is refused after one scan.
//!
//! Each leaf of `chunked` builds its own determinizer, because the table is
//! built as it is walked and no leaf can read another's. That is a real cost of
//! the parallel form and is left in rather than amortized away.
//!
//! Every recognizer `byte_lex` builds is read, including the two the engine
//! already routes by hand through its own byte reader. Those two are the scale
//! the other ten are read against.
//!
//! Timings here are dispatch timings, so the Flynnel calibration the process
//! reads changes them. The runner pins `FLYNNEL_CALIBRATION_DIR`; a reading
//! taken without it is a reading of whichever profile the box last wrote, and
//! the header prints which one this process got.
//!
//! Run: `cargo run --release --example what_the_byte_grain_costs -- <file>...`

use std::time::Instant;

use trex::byte_dfa::ByteDfa;
use trex::byte_lex;
use trex::byte_nfa::{Builder, ByteClass, ByteNfa};
use trex::token::TokenKind;

/// One recognizer: its name, the lexer kind whose shape it claims, how it is
/// built, the id it accepts under, and the byte string its lexer counterpart
/// refuses without where there is one.
type Recognizer = (&'static str, TokenKind, fn() -> ByteNfa, u32, Option<&'static [u8]>);

const RECOGNIZERS: [Recognizer; 12] = [
    ("number", TokenKind::Number, byte_lex::number, byte_lex::id::NUMBER, None),
    ("word", TokenKind::Word, byte_lex::word, byte_lex::id::WORD, None),
    ("url", TokenKind::Url, byte_lex::url, byte_lex::id::URL, Some(b"://")),
    ("jwt", TokenKind::Jwt, byte_lex::jwt, byte_lex::id::JWT, Some(b"eyJ")),
    ("uuid", TokenKind::Uuid, byte_lex::uuid, byte_lex::id::UUID, Some(b"-")),
    ("mac", TokenKind::Mac, byte_lex::mac, byte_lex::id::MAC, None),
    ("hexcolor", TokenKind::HexColor, byte_lex::hexcolor, byte_lex::id::HEX_COLOR, Some(b"#")),
    ("percent", TokenKind::Percent, byte_lex::percent, byte_lex::id::PERCENT, Some(b"%")),
    ("base64", TokenKind::Base64, byte_lex::base64, byte_lex::id::BASE64, None),
    ("hash_digest", TokenKind::HashDigest, byte_lex::hash_digest, byte_lex::id::HASH_DIGEST, None),
    ("bytesize", TokenKind::ByteSize, byte_lex::bytesize, byte_lex::id::BYTE_SIZE, None),
    ("duration", TokenKind::Duration, byte_lex::duration, byte_lex::id::DURATION, None),
];

/// The two kinds the engine reaches at the byte layer today, through
/// `prefilter::byte_route_kind_reading` and its hand-written reader.
const ALREADY_ROUTED: [&str; 2] = ["number", "word"];

/// How many cores this host reports, saying so where it cannot tell rather than
/// reading an unavailable count as one.
fn cores() -> usize {
    match std::thread::available_parallelism() {
        Ok(n) => n.get(),
        Err(e) => {
            eprintln!("available_parallelism failed, reading this host as one core: {e}");
            1
        }
    }
}

/// `inner` with `.*` in front, so its start state survives every byte and one
/// walk over the input answers whether a token of its kind is anywhere in it.
///
/// [`ByteDfa::recognize`] answers what the input opens with. Prefixing the
/// recognizer with a run of any byte turns that into "holds anywhere", which is
/// the refusal's question, and reaches it in one pass rather than in a restart
/// per position.
fn unanchored(inner: &ByteNfa) -> ByteNfa {
    let mut b = Builder::new();
    let any = b.class(ByteClass::any());
    let skip = b.star(any);
    let core = b.adopt(inner);
    let all = b.then(skip, core);
    b.build(all)
}

/// Whether the anchored recognizer claims a token anywhere, asked at every
/// position as a sweep over an anchored automaton must.
fn exists_anchored(dfa: &mut ByteDfa, input: &[u8]) -> bool {
    let mut at = 0usize;
    while at < input.len() {
        match dfa.recognize(&input[at..]) {
            Some((len, _)) if len > 0 => return true,
            _ => at += 1,
        }
    }
    false
}

/// Whether the unanchored recognizer claims a token anywhere, in one walk.
fn exists_one_pass(dfa: &mut ByteDfa, input: &[u8]) -> bool {
    dfa.recognize(input).is_some()
}

/// [`exists_one_pass`] with the input cut at safe boundaries and a whole walk
/// run per chunk across the cores.
///
/// The cut points come from the parallel lexer's own boundary finder, so no
/// token crosses one and a chunk's answer is the answer it would give inside a
/// whole-input walk. Where it finds no safe cut the input is walked once, which
/// is what the parallel lexer does with the same input.
fn exists_chunked(nfa: &ByteNfa, input: &[u8]) -> (bool, usize) {
    let bounds = trex::parallel_lex::safe_boundaries(input, cores() * 4);
    if bounds.len() < 3 {
        let mut dfa = ByteDfa::new(nfa);
        return (exists_one_pass(&mut dfa, input), 1);
    }
    let ranges: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
    let leaves = ranges.len();
    let mut found: Vec<bool> = vec![false; leaves];
    let plan = flynnel::JobPlan::set_profile(
        0,
        u32::try_from(leaves).unwrap_or(u32::MAX),
        flynnel::DispatchProfile::Streaming,
    );
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut found, 1, |base, slots| {
        for (k, slot) in slots.iter_mut().enumerate() {
            let (lo, hi) = ranges[base + k];
            // Its own determinizer: the table is built as it is walked and a
            // leaf cannot read one another leaf is writing.
            let mut dfa = ByteDfa::new(nfa);
            *slot = exists_one_pass(&mut dfa, &input[lo..hi]);
        }
    });
    (found.iter().any(|&b| b), leaves)
}

/// Which recognizer ids the combined gate claims anywhere in `input`, found in
/// one chunked pass.
///
/// The whole point of asking it this way. `byte_grain_rate` reads the gate of
/// every recognizer at the same nanoseconds a byte as the gate of two, because
/// the walk costs per byte and not per recognizer. If that holds unanchored,
/// a refusal is one scan an input rather than one scan a kind, and a pattern
/// over any mix of kinds reads its answer out of this set.
///
/// The walk cannot stop at the first acceptance the way a single-kind ask can:
/// it is collecting which ids occur, and a later byte may carry an id no
/// earlier byte did.
fn kinds_present(nfa: &ByteNfa, input: &[u8]) -> (Vec<u32>, usize) {
    let bounds = trex::parallel_lex::safe_boundaries(input, cores() * 4);
    let ranges: Vec<(usize, usize)> = if bounds.len() < 3 {
        vec![(0, input.len())]
    } else {
        bounds.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let leaves = ranges.len();
    let mut found: Vec<Vec<u32>> = vec![Vec::new(); leaves];
    let plan = flynnel::JobPlan::set_profile(
        0,
        u32::try_from(leaves).unwrap_or(u32::MAX),
        flynnel::DispatchProfile::Streaming,
    );
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut found, 1, |base, slots| {
        for (k, slot) in slots.iter_mut().enumerate() {
            let (lo, hi) = ranges[base + k];
            let mut dfa = ByteDfa::new(nfa);
            let chunk = &input[lo..hi];
            let mut seen: Vec<u32> = Vec::new();
            let mut at = 0usize;
            while at < chunk.len() {
                match dfa.recognize(&chunk[at..]) {
                    Some((len, id)) if len > 0 => {
                        if !seen.contains(&id) {
                            seen.push(id);
                        }
                        at += len;
                    }
                    _ => at += 1,
                }
            }
            *slot = seen;
        }
    });
    let mut all: Vec<u32> = Vec::new();
    for part in found {
        for id in part {
            if !all.contains(&id) {
                all.push(id);
            }
        }
    }
    all.sort_unstable();
    (all, leaves)
}

/// Milliseconds one call takes, with what it returned.
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let started = Instant::now();
    let out = f();
    (out, started.elapsed().as_secs_f64() * 1000.0)
}

/// One corpus against every recognizer.
fn report(path: &str, input: &[u8]) {
    let bytes = input.len() as f64;
    let ns_b = |ms: f64| ms * 1e6 / bytes;

    let (toks, lex_ms) = timed(|| trex::lexer::lex(input));
    let (par_toks, lex_par_ms) = timed(|| trex::parallel_lex::lex_parallel(input));
    assert_eq!(par_toks.len(), toks.len(), "the parallel lex read the corpus differently");

    // Token spans per kind, ascending, which is the order the lexer makes them
    // in and the order the miss check reads them in.
    let mut per_kind: Vec<(u32, Vec<(usize, usize)>)> = Vec::new();
    for t in &toks {
        let code = t.kind.code();
        match per_kind.iter_mut().find(|(c, _)| *c == code) {
            Some((_, v)) => v.push((t.start(), t.end())),
            None => per_kind.push((code, vec![(t.start(), t.end())])),
        }
    }

    println!("\ncorpus {path}, {} bytes, {} tokens", input.len(), toks.len());
    println!(
        "  lex {lex_ms:.1} ms {:.2} ns/B    lex par {lex_par_ms:.1} ms {:.2} ns/B  ({:.2}x)",
        ns_b(lex_ms),
        ns_b(lex_par_ms),
        lex_ms / lex_par_ms
    );
    println!(
        "{:>12} {:>9} {:>7} {:>11} {:>11} {:>11} {:>7} {:>11} {:>10}",
        "recognizer",
        "tokens",
        "missed",
        "anchored",
        "one pass",
        "chunked",
        "leaves",
        "literal",
        "vs lex par"
    );

    for (label, kind, build, id, lit) in RECOGNIZERS {
        let want: &[(usize, usize)] = per_kind
            .iter()
            .find(|(c, _)| *c == kind.code())
            .map_or(&[][..], |(_, v)| v.as_slice());

        let nfa = build();
        let open = unanchored(&nfa);

        // Soundness, asked at each token's own start, which is where a route
        // reaching a caller's position asks.
        let mut probe = ByteDfa::new(&nfa);
        let mut lost = 0usize;
        for &(start, end) in want {
            let answered = match probe.recognize(&input[start..]) {
                Some((len, who)) => who == id && start + len == end,
                None => false,
            };
            if !answered {
                lost += 1;
            }
        }

        // The determinized state counts beside the automaton sizes they came
        // from. A recognizer that is slow because its table grew is a
        // different fault from one that is slow per state, and the two are
        // told apart by nothing else here.
        let ((held_anchored, built_anchored), anchored_ms) = timed(|| {
            let mut dfa = ByteDfa::new(&nfa);
            let held = exists_anchored(&mut dfa, input);
            (held, dfa.built())
        });
        let ((held_one, built_one), one_ms) = timed(|| {
            let mut dfa = ByteDfa::new(&open);
            let held = exists_one_pass(&mut dfa, input);
            (held, dfa.built())
        });
        let ((held_chunk, leaves), chunk_ms) = timed(|| exists_chunked(&open, input));
        let (held_lit, lit_ms) = match lit {
            Some(l) => {
                let (found, ms) = timed(|| trex::byte_simd::contains(input, l));
                (Some(found), Some(ms))
            }
            None => (None, None),
        };

        // The arms answer one question, so they must agree. A disagreement is
        // the reading rather than a detail: it means one of them is unsound.
        assert_eq!(held_anchored, held_one, "{label}: anchored and one-pass disagree on {path}");
        assert_eq!(held_one, held_chunk, "{label}: chunking changed the answer on {path}");
        if let Some(found) = held_lit {
            assert!(
                found || !held_one,
                "{label}: its required literal is absent where a token of it is not"
            );
        }

        let lit_col = lit_ms.map_or_else(|| "-".to_string(), |ms| format!("{ms:.2} ms"));
        let best = lit_ms.map_or(chunk_ms, |ms| ms.min(chunk_ms));
        let routed = if ALREADY_ROUTED.contains(&label) { "  (routed today)" } else { "" };
        println!(
            "{label:>12} {:>9} {lost:>7} {anchored_ms:>8.1} ms {one_ms:>8.1} ms {chunk_ms:>8.1} ms \
             {leaves:>7} {lit_col:>11} {:>9.2}x{routed}",
            want.len(),
            lex_par_ms / best
        );
        // What the `.*` prefix cost this recognizer, which is the whole
        // question for the two built as an intersection: an automaton that
        // grows a state per live starting position determinizes to a set per
        // combination of them.
        println!(
            "{:>12}  nfa {} -> {} states, determinized {built_anchored} -> {built_one}",
            "",
            nfa.len(),
            open.len()
        );
        if lost > 0 {
            println!(
                "{:>12}  loses {lost} of {} matches: no route may be built on it",
                "",
                want.len()
            );
        }
    }

    // Every recognizer at once, which is the question the per-kind rows above
    // are each a twelfth of. A gate costing about what one kind costs turns a
    // refusal into one scan an input.
    let gate = byte_lex::tokens();
    let ((ids, leaves), gate_ms) = timed(|| kinds_present(&gate, input));
    let named: Vec<&str> = RECOGNIZERS
        .iter()
        .filter(|(_, _, _, id, _)| ids.contains(id))
        .map(|(label, _, _, _, _)| *label)
        .collect();
    println!(
        "    combined {:>9} kinds {gate_ms:>8.1} ms over {leaves} leaves, {:>9.2}x the parallel lex",
        ids.len(),
        lex_par_ms / gate_ms
    );
    println!("             {} states, present [{}]", gate.len(), named.join(" "));

    // The gate's own soundness, which does not follow from each recognizer
    // agreeing alone. In the gate the branches compete by precedence and the
    // sweep advances past whatever is claimed, so a token can be masked by a
    // higher-precedence branch owning its bytes - and a kind the lexer found
    // but the gate does not report is a kind this would refuse wrongly.
    let mut masked: Vec<&str> = Vec::new();
    for (label, kind, _, id, _) in RECOGNIZERS {
        let held = per_kind.iter().any(|(c, v)| *c == kind.code() && !v.is_empty());
        if held && !ids.contains(&id) {
            masked.push(label);
        }
    }
    if masked.is_empty() {
        println!("             every kind the lexer found is one the gate reports");
    } else {
        println!(
            "             the gate misses [{}] and would refuse an input holding them",
            masked.join(" ")
        );
    }
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("name one or more corpus files: a cost is a fact about the bytes it was read over");
        std::process::exit(2);
    }

    // Not-set and set-to-something-unreadable are different answers, and the
    // second must not read as the first: a run that believes it is pinned and
    // is not produces a dispatch timing nobody can reproduce.
    let calibration = match std::env::var("FLYNNEL_CALIBRATION_DIR") {
        Ok(dir) => dir,
        Err(std::env::VarError::NotPresent) => "unpinned".to_string(),
        Err(e) => {
            eprintln!("FLYNNEL_CALIBRATION_DIR is set but unreadable: {e}");
            std::process::exit(2);
        }
    };

    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("cores {}, flynnel calibration {calibration}", cores());

    for (i, path) in paths.iter().enumerate() {
        eprintln!("[{}/{}] {path}", i + 1, paths.len());
        let started = Instant::now();
        match std::fs::read(path) {
            Ok(input) => report(path, &input),
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        }
        eprintln!(
            "[{}/{}] {path} done in {:.1}s",
            i + 1,
            paths.len(),
            started.elapsed().as_secs_f64()
        );
    }

    println!(
        "\n`vs lex par` is the parallel lex over the cheapest sound refusal for that kind.\n\
         Above 1 means the kind can be refused for less than trex pays to lex the corpus\n\
         and agree it holds nothing.\n\n\
         `combined` is one pass carrying every recognizer at once, which answers the same\n\
         question for all of them together. Where it costs about what one kind costs, a\n\
         refusal is one scan an input rather than one scan a kind, and a pattern over any\n\
         mix of kinds reads the answer out of a set it did not pay for."
    );
}
