//! What the regex crate does on this corpus that trex does not.
//!
//! On `"let" \W "="` over 7.34 MB the regex crate takes every match in 2.152 ms
//! and trex in 10.660, of which 2.878 is the lex. The question this answers is
//! which part of that gap is the matcher and which part is everything around
//! it, because the two have opposite consequences: a slower matcher is work on
//! the engine, and a missing prefilter is work on what the engine is asked.
//!
//! The regex crate's own source says what it does. `regex-automata`'s
//! `meta::reverse_inner` plucks literals out of a pattern under the invariant
//! that "the literals we return reflect a set where at least one of them must
//! match in order for the overall regex to match", searches the raw bytes for
//! those with SIMD, and runs the automaton only where one is found. That is why
//! `(?P<name>[A-Za-z_][A-Za-z_0-9]*) =`, which has no prefix literal at all, is
//! still fast: the literal it prefilters on is the ` =` in the middle.
//!
//! Both crates are in this one process, so every arm is the same corpus, the
//! same machine and the same moment.
//!
//! The arms, and what each one isolates:
//!
//!   trex scan          the whole operation, lex and engine
//!   trex lex           the floor under it
//!   trex kind scan     every token's kind read once, in order, off the parts -
//!                      the cheapest possible traversal of what the engine
//!                      traverses, so the engine's cost above it is overhead
//!                      rather than the work itself
//!   trex find-all      every occurrence of a literal by trex's own SIMD
//!                      search: what a byte prefilter would cost trex
//!   regex meta         the crate's front door, prefilters and lazy DFA on
//!   regex PikeVM       the same crate's NFA simulation with no prefilter and
//!                      no DFA, which is the algorithm class trex's engine is
//!                      in - this is the honest matcher-against-matcher arm
//!   regex literal      the crate finding a bare literal, as a floor on what
//!                      its prefilter can cost
//!
//! Run: `cargo run --release --example why_regex_wins`.

use std::hint::black_box;
use std::time::Instant;

use regex::Regex;
use regex_automata::nfa::thompson::pikevm::PikeVM;
use trex::parallel_lex::lex_significant_parts_owned;

/// Statements of four shapes, the corpus `benches/vs_regex_full` builds.
fn corpus(statements: usize) -> String {
    let mut s = String::new();
    for i in 0..statements {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
    }
    s
}

const ROUNDS: usize = 5;
const BUDGET_MS: f64 = 25.0;

/// Time `f` until `BUDGET_MS` has passed and at least five calls have run,
/// reporting the mean call and its answer - the comparison's own harness, so
/// these milliseconds are comparable with the rows they explain.
fn time_budget(f: &mut dyn FnMut() -> usize) -> (f64, usize) {
    let n = f();
    let t0 = Instant::now();
    let mut iters = 0u32;
    loop {
        black_box(f());
        iters += 1;
        if iters >= 5 && t0.elapsed().as_secs_f64() * 1e3 >= BUDGET_MS {
            break;
        }
    }
    (t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters), n)
}

/// The middle of `v` and its spread, the largest reading over the smallest.
fn median_and_spread(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("a measured time is never NaN"));
    (v[v.len() / 2], v[v.len() - 1] / v[0])
}

/// Every arm timed once a round in an order that rotates, then the first one
/// again as a control. Returns each arm's median in the order given.
fn sweep(arms: &mut [(&str, &mut dyn FnMut() -> usize)]) -> Vec<f64> {
    let n = arms.len();
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); n];
    let mut answers: Vec<usize> = vec![0; n];
    for round in 0..ROUNDS {
        for step in 0..n {
            let i = (round + step) % n;
            let (t, got) = time_budget(&mut *arms[i].1);
            times[i].push(t);
            answers[i] = got;
        }
    }
    let (control, _) = time_budget(&mut *arms[0].1);

    println!("{:>22} {:>11} {:>9} {:>12}", "arm", "ms", "spread", "answer");
    let mut medians = Vec::with_capacity(n);
    for (i, (name, _)) in arms.iter().enumerate() {
        let (ms, spread) = median_and_spread(times[i].clone());
        medians.push(ms);
        println!("{name:>22} {ms:>11.4} {spread:>8.2}x {:>12}", answers[i]);
    }
    println!(
        "{:>22} {control:>11.4} {:>9} drift {:.3}x against the same work at the start",
        "CONTROL", "", control / medians[0]
    );
    medians
}

/// Every occurrence of `needle` in `haystack` by trex's own SIMD search, which
/// reports the leftmost and is asked again past each hit.
fn find_all(haystack: &[u8], needle: &[u8]) -> usize {
    let (mut at, mut n) = (0usize, 0usize);
    while let Some(i) = trex::byte_simd::find(&haystack[at..], needle) {
        n += 1;
        at += i + 1;
    }
    n
}

/// The whole of what a byte prefilter would cost: every occurrence of `needle`,
/// then the significant-token index of each token that IS that occurrence,
/// written into `out`.
///
/// A literal atom matches a whole token, so a hit counts only where a token
/// starts at it and ends at its end. Hits ascend and spans ascend, so one walk
/// consumes both - no search per hit, and the spans are read in order.
fn candidate_anchors(
    haystack: &[u8],
    needle: &[u8],
    spans: &[(u32, u32)],
    out: &mut Vec<u32>,
) -> usize {
    out.clear();
    let (mut at, mut k) = (0usize, 0usize);
    while let Some(i) = trex::byte_simd::find(&haystack[at..], needle) {
        let hit = (at + i) as u32;
        let end = hit + needle.len() as u32;
        while k < spans.len() && spans[k].0 < hit {
            k += 1;
        }
        if k < spans.len() && spans[k].0 == hit && spans[k].1 == end {
            out.push(k as u32);
        }
        at = hit as usize + 1;
    }
    out.len()
}

fn main() {
    let text = corpus(200_000);
    let input = text.as_bytes();
    // A copy of the kinds rather than the lexer's own parts. An OwnedParts held
    // across the run keeps this thread's lex workspace out, and every other
    // trex arm here would then lex into fresh buffers - measuring the holding
    // rather than the arm.
    // One copy per lexer part: that part's token kinds, and that part's token
    // spans.
    type PartsCopy = (Vec<Vec<u32>>, Vec<Vec<(u32, u32)>>);
    let (kinds, spans): PartsCopy = {
        let owned = lex_significant_parts_owned(input);
        (
            owned.parts().iter().map(|p| p.kinds.clone()).collect(),
            owned.parts().iter().map(|p| p.spans.clone()).collect(),
        )
    };
    let flat_spans: Vec<(u32, u32)> = spans.concat();
    let tokens: usize = kinds.iter().map(Vec::len).sum();
    // A kind read from the stream rather than named, so the count cannot fold
    // away and the arm measures the traversal it is there to measure.
    let probe = kinds[0][0];

    let tp = "\"let\" \\W \"=\"";
    let rp = r"\blet\b [A-Za-z_][A-Za-z_0-9]* =";
    let tp2 = "\\W:name \"=\"";
    let rp2 = r"(?P<name>[A-Za-z_][A-Za-z_0-9]*) =";

    let p = trex::parse(tp).expect("the comparison's own pattern parses");
    let p2 = trex::parse(tp2).expect("the comparison's own pattern parses");
    let re = Regex::new(rp).expect("the comparison's own pattern compiles");
    let re2 = Regex::new(rp2).expect("the comparison's own pattern compiles");
    let lit = Regex::new("let").expect("a bare literal compiles");
    let vm = PikeVM::new(rp).expect("the NFA simulation takes it");
    let vm2 = PikeVM::new(rp2).expect("the NFA simulation takes it");
    let mut cache = vm.create_cache();
    let mut cache2 = vm2.create_cache();

    println!("trex {}", trex::version());
    println!(
        "corpus: {} bytes, {tokens} significant tokens, {} cores",
        input.len(),
        std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
    );
    println!("trex  {tp}\nregex {rp}\n");

    let mut trex_scan = || trex::scan(&p, input).len();
    let mut trex_lex = || lex_significant_parts_owned(input).parts().len();
    let mut kind_scan = || kinds.iter().map(|k| k.iter().filter(|&&x| x == probe).count()).sum();
    let mut find_let = || find_all(input, b"let");
    let mut find_eq = || find_all(input, b" =");
    let mut regex_meta = || re.find_iter(&text).count();
    let mut regex_vm = || vm.find_iter(&mut cache, input).count();
    let mut regex_lit = || lit.find_iter(&text).count();
    let mut cands = Vec::new();
    let n_cands = candidate_anchors(input, b"let", &flat_spans, &mut cands);
    // Built before the arm that refills `cands` borrows it.
    let mut ends_table: Vec<u32> = vec![0u32; tokens];
    for &a in &cands {
        ends_table[a as usize] = a + 3;
    }
    let mut candidates = || candidate_anchors(input, b"let", &flat_spans, &mut cands);

    // The same pattern shape over a literal that is one token in the corpus
    // rather than fifty thousand. If the scan's cost is its attempts, this is
    // the lex and the search and nothing else.
    let rare = trex::parse("\"value_4\" \\W \"=\"").expect("parses");
    let mut trex_rare = || trex::scan(&rare, input).len();

    // The two things the per-anchor scan does once per token however few
    // anchors it attempts: a table of ends the width of the stream, and the
    // selection walk over it. The selection's next index comes out of the table
    // it is reading, so the loads are a dependent chain and no prefetch covers
    // it.
    // The same matches by the serial single-pass walk, which injects a start at
    // every position in one sweep rather than attempting each independently
    // across cores. The tokens are lexed once outside the arm, so this times the
    // walk and not a second lex, and the dispatched scan's own lex is its
    // separate arm above.
    let toks = trex::lexer::lex(input);
    let mut serial_walk =
        || trex::nfa::scan_nfa_over_serial(&p, input, &toks).map_or(0, |v| v.len());

    // The same matches found by lexing only around the literal's occurrences
    // instead of lexing the input. It lexes serially where the whole-input arm
    // lexes across cores, so this arm carries that difference as well as the
    // policy - which is the point of measuring it before wiring it in.
    let mut windows =
        || trex::prefilter::scan_by_literal_windows(&p, input).map_or(usize::MAX, |v| v.len());

    let mut alloc_ends = || vec![0u32; tokens].len();
    let mut select_shape = || {
        let (mut from, mut n) = (0usize, 0usize);
        while from < ends_table.len() {
            let ke = ends_table[from] as usize;
            if ke > from {
                n += 1;
                from = ke;
            } else {
                from += 1;
            }
        }
        n
    };

    println!("=== the pattern with a prefix literal ===");
    let mut arms: [(&str, &mut dyn FnMut() -> usize); 14] = [
        ("trex scan", &mut trex_scan),
        ("trex lex", &mut trex_lex),
        ("trex kind scan", &mut kind_scan),
        ("trex find-all let", &mut find_let),
        ("trex find-all ' ='", &mut find_eq),
        ("trex candidates", &mut candidates),
        ("trex scan, 1 anchor", &mut trex_rare),
        ("trex serial walk", &mut serial_walk),
        ("trex literal windows", &mut windows),
        ("ends table alloc", &mut alloc_ends),
        ("selection walk", &mut select_shape),
        ("regex meta", &mut regex_meta),
        ("regex PikeVM", &mut regex_vm),
        ("regex literal let", &mut regex_lit),
    ];
    let m = sweep(&mut arms);
    let (scan, lex, traversal, flet, feq, cand, rare_ms, serial, wins, alloc, select, meta, pike, rlit) =
        (m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10], m[11], m[12], m[13]);

    println!("\n=== what the numbers say ===");
    println!("{:>34} {:>10.4} ms", "trex engine, scan less lex", scan - lex);
    println!(
        "{:>34} {:>10.4} ms  one kind read an anchor, in order, off the parts",
        "the same anchors, read only",
        traversal
    );
    println!(
        "{:>34} {:>10.1}x  what the engine costs over merely reading every anchor",
        "engine over traversal",
        (scan - lex) / traversal
    );
    println!(
        "{:>34} {:>10.2}x  the crate's front door against its own NFA simulation",
        "regex meta over PikeVM",
        pike / meta
    );
    println!(
        "{:>34} {:>10.2}x  trex's whole operation against the same simulation",
        "trex scan over PikeVM",
        pike / scan
    );
    println!(
        "{:>34} {:>10.4} ms  a byte prefilter's candidates, by trex's own search",
        "find-all the prefix literal",
        flet
    );
    println!(
        "{:>34} {:>10.4} ms  and the inner literal the other pattern would use",
        "find-all ' ='",
        feq
    );
    println!(
        "{:>34} {:>10.4} ms  the crate finding that literal alone",
        "regex literal floor",
        rlit
    );
    println!(
        "{:>34} {:>10.4} ms  find-all AND the token index of every hit",
        "candidate anchors, whole",
        cand
    );
    // What the scan costs above its lex, against the same scan over a literal
    // that is one token rather than fifty thousand. The difference between the
    // two is what the attempts cost; what they share is what the scan pays per
    // token of the stream however few anchors it attempts.
    println!(
        "{:>34} {:>10.4} ms  the same shape over a literal at 1 anchor, not {n_cands}",
        "scan, one candidate",
        rare_ms
    );
    println!(
        "{:>34} {:>10.4} ms  so this is what {n_cands} attempts cost",
        "the attempts",
        scan - rare_ms
    );
    println!(
        "{:>34} {:>10.4} ms  and this is what the scan pays whatever it attempts",
        "the scan's fixed cost",
        rare_ms - lex
    );
    println!(
        "{:>34} {:>10.4} ms  a table of ends the width of the stream",
        "of which, the ends table",
        alloc
    );
    println!(
        "{:>34} {:>10.4} ms  the selection over it, a dependent chain of {tokens} loads",
        "and the selection walk",
        select
    );
    println!(
        "{:>34} {:>10.4} ms  one sweep on one thread, its lex already paid",
        "the serial walk instead",
        serial
    );
    println!(
        "{:>34} {:>10.2}x  what dispatching every anchor across {} cores is worth against it",
        "dispatch over serial",
        serial / (scan - lex),
        std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
    );
    println!(
        "{:>34} {:>10.4} ms  lexing only around the literal, against the whole scan's {scan:.3}",
        "the literal windows",
        wins
    );
    println!(
        "{:>34} {:>10.2}x  and against regex's {meta:.3}",
        "windows over regex",
        wins / meta
    );

    // The capture path, split into the two halves it is made of. It is the
    // largest block left and three mechanisms read off the source have been
    // wrong about it, so this measures where the time is rather than proposing
    // where it might be: the scan that finds the spans, the resolve that says
    // what each bound, and the whole call a caller makes.
    println!("\n=== the capture path on a binding pattern ===");
    let bound = trex::parse("\"let\" \\W:v \"=\"").expect("the comparison's own pattern parses");
    let bound_re = Regex::new(r"\blet\b (?P<v>[A-Za-z_][A-Za-z_0-9]*) =")
        .expect("the comparison's own pattern compiles");
    let bound_spans = trex::scan(&bound, input);
    let n_bound = bound_spans.len();
    let mut b_scan = || trex::scan(&bound, input).len();
    let mut b_resolve = || trex::captures(&bound, input, &bound_spans).len();
    let mut b_iter = || trex::captures_iter(&bound, input).count();
    let mut b_regex = || bound_re.captures_iter(&text).count();
    // What building the capture lists costs on its own, with no matching in it
    // at all: one shared name and the registers a match, which is what
    // `resolve_captures` does for every match it reports. The scan and the
    // resolve run the same windows over the same anchors and differ only in
    // this, so it is the term that sizes the match type's storage.
    //
    // Built the way a match holds them, so this reads what the type costs now
    // rather than what some other shape would: a register count at or under
    // `trex::INLINE_REGS` rides in the match and allocates nothing.
    let names: std::sync::Arc<[String]> = std::sync::Arc::from(vec!["v".to_string()]);
    let a_span = bound_spans[0];
    let mut b_build = || {
        let mut out: Vec<trex::Match> = Vec::with_capacity(n_bound);
        for _ in 0..n_bound {
            out.push(trex::Match::bound(
                a_span.start(),
                a_span.end(),
                trex::Regs::from_slice(&[a_span]),
                names.clone(),
            ));
        }
        out.len()
    };
    // The same loop over a match that binds nothing, which holds neither the
    // shared name nor the registers. What separates the two arms is those two
    // together; what is left in this one is the vector that takes fifty thousand
    // fifty-six-byte matches and frees them, which both pay and neither is about.
    //
    // The name cannot be priced apart from the registers through this door: an
    // empty name list still costs the clone and the drop that a full one does,
    // so an arm passing one would differ from the arm above in where the drop
    // lands and not in whether there is one.
    let mut b_span = || {
        let mut out: Vec<trex::Match> = Vec::with_capacity(n_bound);
        for _ in 0..n_bound {
            out.push(trex::Match::plain(a_span.start(), a_span.end()));
        }
        out.len()
    };
    // The same walk over the same matches, handing each one back borrowed. It
    // is the counterpart of `trex captures_iter` and differs from it in what a
    // match costs to hold, so the two together price the owning.
    let mut b_ref = || {
        let mut c = trex::captures_iter(&bound, input);
        let mut n = 0usize;
        while c.next_ref().is_some() {
            n += 1;
        }
        n
    };
    // The reusing form on this same pattern, which takes its own copy of the
    // ladder and so answers whether a rung change reaches it too. The section
    // below has this arm for a pattern the byte route answers; this is the one
    // for a pattern the windows answer.
    let mut slots_b = trex::CaptureSlots::of(&bound);
    let mut b_reuse = || {
        let mut n = 0usize;
        if let Some(mut c) = trex::captures_read_iter(&bound, input) {
            while c.next_into(&mut slots_b).is_some() {
                n += 1;
            }
        }
        n
    };
    let mut arms3: [(&str, &mut dyn FnMut() -> usize); 8] = [
        ("trex scan", &mut b_scan),
        ("trex resolve", &mut b_resolve),
        ("trex captures_iter", &mut b_iter),
        ("the capture lists alone", &mut b_build),
        ("the same, binding nothing", &mut b_span),
        ("trex captures_iter, borrowed", &mut b_ref),
        ("trex into a buffer", &mut b_reuse),
        ("regex captures_iter", &mut b_regex),
    ];
    let m3 = sweep(&mut arms3);
    println!(
        "\n{:>34} {:>10.4} ms  what captures_iter costs above scan plus resolve, over {n_bound} matches",
        "unaccounted",
        m3[2] - m3[0] - m3[1]
    );
    println!(
        "{:>34} {:>10.1} ns  a match, for the resolve alone",
        "resolve a match",
        m3[1] * 1e6 / n_bound as f64
    );
    println!(
        "{:>34} {:>10.4} ms  {:.1} ns a match, and no matching in it at all",
        "the capture lists alone",
        m3[3],
        m3[3] * 1e6 / n_bound as f64
    );
    println!(
        "{:>34} {:>10.1}%  of the resolve, spent owning a name and a vector a match",
        "which is",
        100.0 * m3[3] / m3[1]
    );
    println!(
        "{:>34} {:>10.1} ns  a match, for the shared name and the registers together",
        "the binding a match holds",
        (m3[3] - m3[4]) * 1e6 / n_bound as f64
    );
    println!(
        "{:>34} {:>10.1} ns  a match, for the vector alone, which binds nothing",
        "the vector under them",
        m3[4] * 1e6 / n_bound as f64
    );
    println!(
        "{:>34} {:>10.4} ms  {:.1} ns a match, for owning each match rather than borrowing it",
        "what the owning costs",
        m3[2] - m3[5],
        (m3[2] - m3[5]) * 1e6 / n_bound as f64
    );

    // What a cursor costs a match on a pattern that binds nothing, where there
    // is no register to resolve and the whole difference from the scan is the
    // handing over. `walk's own slots` costs about a millisecond more than
    // `every match` on each such pattern, which is twenty nanoseconds a match
    // to index a vector - so these arms say which layer spends it: the spans
    // alone, or the match built around each.
    println!("\n=== what handing a match over costs, with nothing bound ===");
    let n_plain = trex::scan(&p, input).len();
    let mut h_scan = || trex::scan(&p, input).len();
    let mut h_spans = || trex::find_iter(&p, input).count();
    let mut h_iter = || trex::captures_iter(&p, input).count();
    let mut arms8: [(&str, &mut dyn FnMut() -> usize); 3] = [
        ("trex scan", &mut h_scan),
        ("trex find_iter", &mut h_spans),
        ("trex captures_iter", &mut h_iter),
    ];
    let m8 = sweep(&mut arms8);
    println!(
        "\n{:>34} {:>10.1} ns  a match, for taking the spans one at a time",
        "the span cursor",
        (m8[1] - m8[0]) * 1e6 / n_plain as f64
    );
    println!(
        "{:>34} {:>10.1} ns  a match, for the match built around each span",
        "the match built on top",
        (m8[2] - m8[1]) * 1e6 / n_plain as f64
    );

    // The two whole-input passes a window scan makes before it opens a single
    // window - the blob table and the anchor search - beside the whole call.
    // The route reads a few tokens an anchor instead of every token in the
    // input, so a whole-input pass inside it is the cost it exists to remove.
    println!("\n=== what a window scan pays before the first window ===");
    let mut w_blobs = || trex::lexer::blob_runs(input).len() + 1;
    let mut w_blobs_par = || trex::lexer::blob_runs_parallel(input).len() + 1;
    let mut w_anchors =
        || trex::prefilter::byte_route_word_literals(&["let"], input).map_or(0, |v| v.len());
    let mut w_whole =
        || trex::prefilter::scan_captures_by_literal_windows(&bound, input).map_or(0, |v| v.len());
    // The same anchors and the same dispatch, with an attempt that reads each
    // window and reports nothing. What separates it from the whole scan is the
    // engine over each window's tokens, which is the split that says whether
    // lexing fifty thousand small windows or attempting a match in each is the
    // term worth attacking.
    let mut w_lexed = || trex::prefilter::anchor_windows_unscanned(&bound, input).unwrap_or(0);
    // The anchor term's own halves, over the literal the anchors are found by.
    // The literal sweep further down decomposes `alpha`, which sits in a
    // fourteen-byte run; `let` opens its own run and is three, so what the
    // probe costs over one says nothing about the other.
    let mut w_edges = || trex::prefilter::byte_route_word_literal_unprobed("let", input).unwrap_or(0);
    let mut w_quote =
        || trex::prefilter::byte_route_word_literal_quote_only("let", input).unwrap_or(0);
    let mut arms4: [(&str, &mut dyn FnMut() -> usize); 7] = [
        ("the blob table", &mut w_blobs),
        ("the blob table, cores", &mut w_blobs_par),
        ("the anchors", &mut w_anchors),
        ("the anchors, no probe", &mut w_edges),
        ("the anchors, quote only", &mut w_quote),
        ("the windows, lexed not scanned", &mut w_lexed),
        ("the whole window scan", &mut w_whole),
    ];
    let m4 = sweep(&mut arms4);
    println!(
        "\n{:>34} {:>10.4} ms  the whole scan less the blob table and the anchors",
        "the windows themselves",
        m4[6] - m4[0] - m4[2]
    );
    println!(
        "{:>34} {:>10.1}%  of the window scan, spent on a whole-input pass it opens no window for",
        "the blob table is",
        100.0 * m4[0] / m4[6]
    );
    println!(
        "{:>34} {:>10.4} ms  the anchors, the dispatch and the lexing, with no attempt in them",
        "the windows, lexed",
        m4[5]
    );
    println!(
        "{:>34} {:>10.4} ms  the engine attempting a match in each window",
        "the attempts",
        m4[6] - m4[5]
    );
    // The anchor term split the way the literal route is split, over its own
    // literal: the two byte tests, then the quoting, then the run.
    println!(
        "{:>34} {:>10.4} ms  the two byte tests at every occurrence of the anchor literal",
        "the anchors' edges",
        m4[3]
    );
    println!(
        "{:>34} {:>10.4} ms  asking whether a string holds the position",
        "the anchors' quote question",
        m4[4] - m4[3]
    );
    println!(
        "{:>34} {:>10.4} ms  finding the word run and settling it",
        "the anchors' run question",
        m4[2] - m4[4]
    );

    // A pattern that is one literal atom, over the three ways a caller asks for
    // it. The comparison's reusing loop asks through the capture cursor, whose
    // ladder is not the scan's, so a route the scan takes says nothing about
    // what that loop pays: these two arms are the same question of both.
    println!("\n=== a pattern that is one literal atom ===");
    let lone = trex::parse("\"alpha\"").expect("a bare literal parses");
    let lone_re = Regex::new(r"\balpha\b").expect("its regex compiles");
    let mut l_scan = || trex::scan(&lone, input).len();
    let mut l_slots = trex::CaptureSlots::of(&lone);
    let mut l_reuse = || {
        let mut n = 0usize;
        if let Some(mut c) = trex::captures_read_iter(&lone, input) {
            while c.next_into(&mut l_slots).is_some() {
                n += 1;
            }
        }
        n
    };
    let mut l_regex = || lone_re.find_iter(&text).count();
    let mut arms5: [(&str, &mut dyn FnMut() -> usize); 3] = [
        ("trex scan", &mut l_scan),
        ("trex into a buffer", &mut l_reuse),
        ("regex find_iter", &mut l_regex),
    ];
    let m5 = sweep(&mut arms5);
    println!(
        "\n{:>34} {:>10.2}x  the reusing loop over the scan, on a pattern that binds nothing",
        "the cursor over the scan",
        m5[1] / m5[0]
    );

    // The byte-pattern route, split into the three things it does at each
    // occurrence: find the prefix, read the token the prefix opens, and match
    // the pattern against that token's bytes. It loses on every operation it
    // has, and only one of the three is a candidate.
    println!("\n=== the byte-pattern route, at every occurrence ===");
    let bpat = trex::parse("`cond_[0-9]+`").expect("a byte pattern parses");
    let bpat_re = Regex::new(r"\bcond_[0-9]+\b").expect("its regex compiles");
    let bp = trex::bytepat::parse(b"cond_[0-9]+").expect("the byte pattern itself parses");
    let bp_spans: Vec<trex::Span> = trex::scan(&bpat, input);
    let n_bp = bp_spans.len();
    let mut p_find = || find_all(input, b"cond_");
    let mut p_whole =
        || bp_spans.iter().filter(|s| bp.matches_whole(&input[s.range()])).count();
    let mut p_scan = || trex::scan(&bpat, input).len();
    let mut p_regex = || bpat_re.find_iter(&text).count();
    let mut arms6: [(&str, &mut dyn FnMut() -> usize); 4] = [
        ("find-all cond_", &mut p_find),
        ("matches_whole alone", &mut p_whole),
        ("trex scan", &mut p_scan),
        ("regex find_iter", &mut p_regex),
    ];
    let m6 = sweep(&mut arms6);
    println!(
        "\n{:>34} {:>10.1} ns  a token, for the byte-pattern match alone over {n_bp} of them",
        "matches_whole a token",
        m6[1] * 1e6 / n_bp as f64
    );
    println!(
        "{:>34} {:>10.4} ms  the scan less the search and the byte-pattern match",
        "the token reads",
        m6[2] - m6[0] - m6[1]
    );

    println!("\n=== the pattern with no prefix literal, only an inner one ===");
    println!("trex  {tp2}\nregex {rp2}");
    let mut trex_scan2 = || trex::scan(&p2, input).len();
    let mut regex_meta2 = || re2.find_iter(&text).count();
    let mut regex_vm2 = || vm2.find_iter(&mut cache2, input).count();
    let mut arms2: [(&str, &mut dyn FnMut() -> usize); 3] = [
        ("trex scan", &mut trex_scan2),
        ("regex meta", &mut regex_meta2),
        ("regex PikeVM", &mut regex_vm2),
    ];
    let m2 = sweep(&mut arms2);
    println!(
        "\n{:>34} {:>10.2}x  with no prefix literal, the crate still prefilters on ' ='",
        "regex meta over PikeVM",
        m2[2] / m2[1]
    );

    // The same pattern's capture path, which is the comparison's worst row. The
    // cursor and the eager resolve reach the same windows by different doors,
    // so timing them in one sweep against one control is the only way to say
    // which door costs what: a cursor above the scan and the resolve together
    // is a cursor paying for something they do not.
    println!("\n=== the capture path on the pattern with only an inner literal ===");
    let spans2 = trex::scan(&p2, input);
    let n2 = spans2.len();
    let mut c_scan = || trex::scan(&p2, input).len();
    let mut c_eager = || trex::captures(&p2, input, &spans2).len();
    let mut c_iter = || trex::captures_iter(&p2, input).count();
    let mut slots2 = trex::CaptureSlots::of(&p2);
    let mut c_reuse = || {
        let mut n = 0usize;
        if let Some(mut c) = trex::captures_read_iter(&p2, input) {
            while c.next_into(&mut slots2).is_some() {
                n += 1;
            }
        }
        n
    };
    let mut c_regex = || re2.captures_iter(&text).count();
    // The same walk handing each match back borrowed. This pattern leaves by a
    // byte route with its registers inline, which is the source the borrowed
    // view is for: no match is built and the names stay with the cursor.
    let mut c_ref = || {
        let mut c = trex::captures_iter(&p2, input);
        let mut n = 0usize;
        while c.next_ref().is_some() {
            n += 1;
        }
        n
    };
    let mut arms7: [(&str, &mut dyn FnMut() -> usize); 6] = [
        ("trex scan", &mut c_scan),
        ("trex captures", &mut c_eager),
        ("trex captures_iter", &mut c_iter),
        ("trex captures_iter, borrowed", &mut c_ref),
        ("trex into a buffer", &mut c_reuse),
        ("regex captures_iter", &mut c_regex),
    ];
    let m7 = sweep(&mut arms7);
    println!(
        "\n{:>34} {:>10.4} ms  what the cursor costs above the scan and the resolve, over {n2} matches",
        "the cursor's own cost",
        m7[2] - m7[0] - m7[1]
    );
    println!(
        "{:>34} {:>10.4} ms  and what the reusing loop costs above them, building no match at all",
        "the reusing loop's own cost",
        m7[4] - m7[0] - m7[1]
    );
    println!(
        "{:>34} {:>10.4} ms  {:.1} ns a match, for owning each match rather than borrowing it",
        "what the owning costs",
        m7[2] - m7[3],
        (m7[2] - m7[3]) * 1e6 / n2 as f64
    );

    // The balanced route, split into the lex it takes and the pass it makes.
    //
    // It is the one route that reads a stitched token stream, because it reads
    // the mate the lexer recorded and a significant part carries kinds and
    // spans and no pairing. The parts lex is the same tokenization without the
    // join copy, so the two lexes beside each other are what pairing brackets
    // in the parts would be worth - and whether it is worth doing at all, which
    // is the open half of the lexer item and is not answerable from the source.
    println!("\n=== the balanced route, and the lex it takes ===");
    let balanced = trex::parse("\\B").expect("a bare balanced group parses");
    let n_bal = trex::scan(&balanced, input).len();
    let mut bal_route = || trex::scan(&balanced, input).len();
    let mut bal_stitched = || trex::parallel_lex::lex_parallel_held(input, <[trex::token::Token]>::len);
    let mut bal_parts = || {
        trex::parallel_lex::lex_significant_parts_held(input, |parts| {
            parts.iter().map(|p| p.kinds.len()).sum::<usize>()
        })
    };
    let mut bal_pass = || {
        trex::parallel_lex::lex_parallel_held(input, |toks| {
            trex::kind_route::scan_balanced(None, toks).len()
        })
    };
    let mut arms8: [(&str, &mut dyn FnMut() -> usize); 4] = [
        ("the whole route", &mut bal_route),
        ("the stitched lex alone", &mut bal_stitched),
        ("the parts lex alone", &mut bal_parts),
        ("that lex and the pass", &mut bal_pass),
    ];
    let m8 = sweep(&mut arms8);
    println!(
        "\n{:>34} {:>10.4} ms  the pass over the mates, over {n_bal} matches",
        "the pass itself",
        m8[3] - m8[1]
    );
    println!(
        "{:>34} {:>10.4} ms  the stitch over the parts lex, which is what pairing in place could save",
        "the join copy",
        m8[1] - m8[2]
    );
    println!(
        "{:>34} {:>9.1}%  of the whole route",
        "which is",
        100.0 * (m8[1] - m8[2]) / m8[0]
    );

    // The literal route, decomposed into the search and everything the route
    // does around it.
    //
    // The two search forms answer identically and differ in cost, and
    // `benches/find_all_crossover` is where that difference is read. What it
    // cannot report is what a caller pays to use the faster one: the split gives
    // each leaf a vector of its own and joins them with a copy, so a route
    // taking it pays allocations and a memcpy in exchange for the scanning.
    // These four arms sit in one sweep so the search and the route consuming it
    // are read against one control, which is what separates a faster search from
    // a faster route.
    println!("\n=== the literal route, and the search inside it ===");
    let lit = b"alpha";
    let mut r_one = || trex::byte_simd::find_all_one_pass(input, lit).len();
    let mut r_across = || trex::byte_simd::find_all_across(input, lit).len();
    let mut r_walk = || trex::byte_simd::occurrences(input, lit).count();
    let mut r_route = || trex::scan(&lone, input).len();
    let mut r_unsplit =
        || trex::prefilter::byte_route_word_literal_unsplit("alpha", input).map_or(0, |v| v.len());
    let mut r_unprobed =
        || trex::prefilter::byte_route_word_literal_unprobed("alpha", input).unwrap_or(0);
    let mut r_quote =
        || trex::prefilter::byte_route_word_literal_quote_only("alpha", input).unwrap_or(0);
    let mut r_run =
        || trex::prefilter::byte_route_word_literal_run_only("alpha", input).unwrap_or(0);
    // The pair that differs in one thing. Both settle every occurrence; one
    // reads the flags the bound scan carried and the other makes the pass those
    // flags replace. Their difference is the only reading of that pass, because
    // the route between two processes has moved 18% doing identical work.
    let mut r_carried =
        || trex::prefilter::byte_route_word_literal_carried("alpha", input).unwrap_or(0);
    let mut r_rescan =
        || trex::prefilter::byte_route_word_literal_rescanned("alpha", input).unwrap_or(0);
    let mut arms9: [(&str, &mut dyn FnMut() -> usize); 10] = [
        ("the search, one pass", &mut r_one),
        ("the search, split", &mut r_across),
        ("the walk the routes take", &mut r_walk),
        ("the whole route", &mut r_route),
        ("the whole route, unsplit", &mut r_unsplit),
        ("the same, with no probe", &mut r_unprobed),
        ("the same, quote check only", &mut r_quote),
        ("the same, run found not settled", &mut r_run),
        ("settling, flags carried", &mut r_carried),
        ("settling, run rescanned", &mut r_rescan),
    ];
    let m9 = sweep(&mut arms9);
    println!(
        "\n{:>34} {:>10.4} ms  what splitting the search saves, on its own",
        "the split's saving",
        m9[0] - m9[1]
    );
    println!(
        "{:>34} {:>10.4} ms  what the walk costs above the split it is built on",
        "the walk's own cost",
        m9[2] - m9[1]
    );
    println!(
        "{:>34} {:>10.4} ms  what the route costs above the walk, deciding each occurrence",
        "the route above the walk",
        m9[3] - m9[2]
    );
    println!(
        "{:>34} {:>9.1}%  of the saving that the walk gives back",
        "which leaves",
        if m9[0] - m9[1] > 0.0 { 100.0 * (m9[2] - m9[1]) / (m9[0] - m9[1]) } else { 0.0 }
    );
    // The two routes differ in one thing: where the occurrences come from. Every
    // decision about each one is the same function, so this difference is the
    // split and nothing else - which is what a reading taken across two
    // processes, minutes apart, cannot say.
    println!(
        "{:>34} {:>10.4} ms  the split route against the unsplit one, one process, one rotation",
        "what the split is worth",
        m9[4] - m9[3]
    );
    // The route's own per-occurrence work, split into the two byte tests and the
    // reader's probe. The unprobed arm answers a different question by
    // construction and is a cost only; what it is for is the subtraction.
    println!(
        "{:>34} {:>10.4} ms  the byte tests at every occurrence, the probe left out",
        "the edges alone",
        m9[5] - m9[1]
    );
    println!(
        "{:>34} {:>10.4} ms  what asking the reader whether a token ends there costs",
        "the probe",
        m9[4] - m9[5]
    );
    println!(
        "{:>34} {:>9.1}%  of the route, spent in the probe",
        "which is",
        100.0 * (m9[4] - m9[5]) / m9[4]
    );
    // The probe asks two questions and they have different fixes: whether a
    // string holds the position, read off a scan of the input's quoting, and
    // whether a word run settles, read off the run's own bytes.
    println!(
        "{:>34} {:>10.4} ms  asking whether a string holds the position",
        "the quote question",
        m9[6] - m9[5]
    );
    println!(
        "{:>34} {:>10.4} ms  finding the word run and settling it",
        "the run question",
        m9[4] - m9[6]
    );
    // And the run question's own two halves, which have nothing in common.
    // Finding the run is four scans over the bytes around the occurrence, past
    // one-entry caches that miss because consecutive occurrences of a literal
    // sit in different runs; asking once per run instead of once per occurrence
    // is what would move it. Settling the run is one table pass that returns on
    // its first test for a run holding no dot, slash or colon; only a cheaper
    // question moves that.
    println!(
        "{:>34} {:>10.4} ms  the four scans that find the run around the occurrence",
        "finding the run",
        m9[7] - m9[6]
    );
    println!(
        "{:>34} {:>10.4} ms  the table pass over the run that settles it",
        "settling the run",
        m9[4] - m9[7]
    );
    // The one subtraction here between two arms that differ in a single call
    // argument, rather than between a route and a route minus a step.
    //
    // Gross and not net: both arms accumulate the flags while scanning the
    // run's bounds, so this prices the settle pass alone. Carrying the flags is
    // worth this less what the accumulation costs, and only an A/B against a
    // build whose bound scan does not accumulate reads that.
    println!(
        "{:>34} {:>10.4} ms  the settle pass the carried flags replace, gross of carrying them",
        "the settle pass",
        m9[9] - m9[8]
    );
    println!(
        "{:>34} {:>9.1}%  of the route, spent settling runs",
        "which is",
        if m9[4] > 0.0 { 100.0 * (m9[9] - m9[8]) / m9[4] } else { 0.0 }
    );
}
