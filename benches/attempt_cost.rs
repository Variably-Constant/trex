//! What a match attempt costs when it fails, against when it succeeds.
//!
//! `its sweep: an attempt a start` is the largest single line on the measured
//! surface, and the counts say 1,500,001 of its 1,650,000 attempts fail, every
//! one of them coming back from `advance` with an empty state set. Whether a
//! cheaper per-anchor rejection is worth building turns on what those failures
//! cost, and "the set came back empty" is an outcome rather than a cost: a set
//! that dies on the first atom and one that expands over twenty tokens and then
//! dies both report empty.
//!
//! The per-shape figures on the surface cannot answer it, because those shapes
//! differ by pattern as well as by match rate - an atomic group and a lazy star
//! both find fifty thousand and read 114.880 ms against 15.465. So this runs one
//! pattern over two corpora built to differ only in whether it matches: the same
//! statements, one separator byte apart. The token count and the kinds are
//! asserted equal, so the attempt count is identical by construction and the
//! only difference left is success against failure.
//!
//! The phase is read rather than the whole scan, because the two corpora also
//! differ in what the leftmost selection does - fifty thousand matches to select
//! against none - and that is its own phase.
//!
//! Both arms run in one process in a rotating order, reduced by their medians,
//! with one re-read at the end as a control. A cross-run comparison on this box
//! is worth nothing at this size: untouched fields have moved 30 to 40% between
//! runs whose own controls were good.
//!
//! Run: `cargo bench --bench attempt_cost`.

/// How many times each arm is read before its middle reading is kept.
const ROUNDS: usize = 5;

/// The phase this exists to read.
const PHASE: &str = "its sweep: an attempt a start";

/// Statements whose assigned value decides whether the pattern can match: a
/// number in one corpus and, in the other, a word of the same length with each
/// digit carried to a letter. The byte count, the token count and every other
/// token are identical; one token a statement changes kind.
///
/// Both corpora hold the `=` the pattern requires, and that is load-bearing
/// rather than incidental. A corpus missing it is refused whole by
/// `requires_absent` before the set engine is reached, so the arm would
/// measure the prefilter's exit instead of a failing attempt - which is what a
/// first draft of this bench did, and what its own assertion caught.
fn corpus(numeric: bool, statements: usize) -> Vec<u8> {
    let mut s = String::new();
    for i in 0..statements {
        let digits = (i * 37).to_string();
        let value: String = if numeric {
            digits
        } else {
            digits.bytes().map(|b| char::from(b'a' + (b - b'0'))).collect()
        };
        s.push_str(&format!("let value_{i} = {value} ;\n"));
    }
    s.into_bytes()
}

/// The middle of several readings.
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let statements = 50_000;
    let matching = corpus(true, statements);
    let failing = corpus(false, statements);
    // The comparison rests on these two being the same work up to the match, so
    // it is asserted rather than assumed: the same bytes to read, the same
    // tokens to attempt at, and exactly one token a statement differing in
    // kind - the value, which is the whole reason one matches and one does not.
    assert_eq!(matching.len(), failing.len(), "the corpora differ in length");
    let a_toks = trex::lexer::lex(&matching);
    let b_toks = trex::lexer::lex(&failing);
    assert_eq!(a_toks.len(), b_toks.len(), "the corpora cut into different token counts");
    let differing = a_toks.iter().zip(&b_toks).filter(|(x, y)| x.kind != y.kind).count();
    assert_eq!(differing, statements, "the corpora differ in more than the assigned value");

    // Assertions, which run a sub-pattern as a filter and so cannot be answered
    // by a byte route: they reach the set engine's per-anchor sweep, which is
    // the phase this bench reads. A plainer shape like `\W "=" \N` is dispatched
    // instead and records no sweep at all.
    //
    // The two are the same shape at opposite polarity over the same two
    // corpora: the first matches where the value is a number and the second
    // where it is a word, so each corpus is the matching one for exactly one of
    // them. If a failing attempt only looked dearer because one corpus is
    // intrinsically dearer to walk, the two would disagree about which way.
    let numeric = trex::parse("\\W ~(\"=\" \\N)").expect("the pattern parses");
    let worded = trex::parse("\\W ~(\"=\" \\W)").expect("the pattern parses");
    println!(
        "corpus: {} bytes, {} tokens, one value a statement apart",
        matching.len(),
        a_toks.len()
    );
    println!("{ROUNDS} rounds an arm, the order rotating\n");

    // One reading of one arm: the phase in milliseconds, what the scan found,
    // and the two counts that say the arms are comparable.
    let read = |p: &_, input: &[u8]| -> (f64, usize, u64, u64) {
        trex::trace::record();
        drop(trex::trace::take_phases());
        drop(trex::trace::take_counts());
        let found = trex::scan(p, input).len();
        let phases = trex::trace::take_phases();
        let counts = trex::trace::take_counts();
        trex::trace::stop();
        let (_, _, spent) = phases
            .iter()
            .find(|(name, _, _)| *name == PHASE)
            .expect("the shape reaches the set engine's per-anchor sweep");
        let count = |want: &str| {
            counts
                .iter()
                .find(|(name, _)| *name == want)
                .map_or_else(|| panic!("the sweep did not report {want}"), |(_, n)| *n)
        };
        (
            spent.as_secs_f64() * 1e3,
            found,
            count("the sweep: attempts made"),
            count("the sweep: attempts that match"),
        )
    };

    // Every pattern against every corpus, so each shape is read on the corpus
    // it matches and on the one it does not.
    let arms: [(&str, &_, &[u8]); 4] = [
        ("number, on the numbers", &numeric, &matching),
        ("number, on the words", &numeric, &failing),
        ("word, on the numbers", &worded, &matching),
        ("word, on the words", &worded, &failing),
    ];
    let mut taken: Vec<Vec<f64>> = vec![Vec::new(); arms.len()];
    let mut answered = [(0usize, 0u64, 0u64); 4];
    for round in 0..ROUNDS {
        for step in 0..arms.len() {
            let i = (step + round) % arms.len();
            let (ms, found, made, matched) = read(arms[i].1, arms[i].2);
            taken[i].push(ms);
            answered[i] = (found, made, matched);
        }
    }

    let mut ms = [0.0f64; 4];
    println!(
        "{:<24} {:>10} {:>12} {:>10} {:>14}",
        "", "phase ms", "attempts", "matching", "ns an attempt"
    );
    for (i, (label, _, _)) in arms.iter().enumerate() {
        ms[i] = median(taken[i].clone());
        let (found, made, matched) = answered[i];
        println!(
            "{label:<24} {:>10.3} {made:>12} {matched:>10} {:>14.1}   {found} spans",
            ms[i],
            ms[i] * 1e6 / made as f64
        );
    }

    // The first arm read again once the sweep is done. The distance between this
    // and its row is the box moving under the run rather than any arm, and it
    // bounds the smallest difference between them that means anything.
    let control = median((0..ROUNDS).map(|_| read(arms[0].1, arms[0].2).0).collect());
    println!(
        "\n  the control: {control:.3} ms against {:.3} for {}, {:.3}x",
        ms[0],
        arms[0].0,
        control / ms[0]
    );
    // One ratio a pattern, each dividing its own failing arm by its own matching
    // one. The two patterns fail on opposite corpora, so a ratio that is really
    // one corpus being dearer to walk would come out above one for a pattern and
    // below one for the other.
    println!(
        "  a failing attempt costs {:.3}x a succeeding one for the number shape",
        ms[1] / ms[0]
    );
    println!(
        "  a failing attempt costs {:.3}x a succeeding one for the word shape",
        ms[2] / ms[3]
    );
}
