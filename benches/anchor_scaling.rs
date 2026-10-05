//! Does an anchored pattern scale linearly now that it compiles to the
//! single-pass engine?
//!
//! `^`, `$`, `\A` and `\z` used to force the set-reachability engine, which
//! recomputes a nested quantifier's closure once per starting position. They
//! now compile to a zero-width instruction on the linear engine. The reported
//! figure is nanoseconds per line: flat across the sweep is linear, growing
//! with the line count is not.
//!
//! Three arms run over the same corpus so a lexer or engine change moves all
//! of them and cannot be mistaken for a change in the anchor. The unanchored
//! control is the floor; the property-anchor arm still routes to the set
//! engine and is what the positional anchors used to cost.

use std::time::Instant;

/// How many times each arm is timed at each size, each in a different position.
const ROUNDS: usize = 5;

/// The middle reading, which is what a round's worth of samples reduces to.
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// A log-shaped corpus: `lines` lines, each leading with a word and carrying
/// one more occurrence of it further along, so an anchor has something to
/// discriminate rather than matching everything.
fn corpus(lines: usize) -> Vec<u8> {
    let mut out = String::with_capacity(lines * 48);
    for i in 0..lines {
        out.push_str("cat handler ");
        out.push_str(&i.to_string());
        out.push_str(" saw cat again here\n");
    }
    out.into_bytes()
}

/// Wall time of one scan in nanoseconds, with the match count so a run that
/// silently stopped matching cannot read as a speed-up.
#[allow(clippy::cast_precision_loss)]
fn once(pat: &trex::ast::Pattern, input: &[u8]) -> (f64, usize) {
    let t = Instant::now();
    let matches = trex::scan(pat, input);
    (t.elapsed().as_nanos() as f64, matches.len())
}

fn main() {
    // State which engine each arm reaches, so the table is read against the
    // routing rather than against an assumption about it.
    for pat in ["^ \"cat\"", "\"cat\"", "@seam \"cat\""] {
        let p = trex::parse(pat).expect("pattern parses");
        let engine = if trex::nfa::scan_nfa(&p, b"cat x").is_some() {
            "single-pass"
        } else {
            "set-reachability"
        };
        println!("{pat:<16} -> {engine}");
    }
    println!();
    println!("lines      bytes   ^\"cat\" ns/line   \"cat\" ns/line   @seam ns/line   ^ hits");
    println!("---------------------------------------------------------------------------");

    // Parsed once, so the rounds below time scanning and not parsing. The
    // anchored arm is first and the unanchored control second, which is the
    // order the assertions below read them in.
    let arms: Vec<(&str, trex::ast::Pattern)> = ["^ \"cat\"", "\"cat\"", "@seam \"cat\""]
        .iter()
        .map(|src| (*src, trex::parse(src).expect("pattern parses")))
        .collect();

    let mut first: Option<f64> = None;
    let mut last: Option<f64> = None;
    for lines in [250usize, 500, 1_000, 2_000, 4_000] {
        let input = corpus(lines);
        let (_, warm_hits) = once(&arms[0].1, &input);

        // Each arm read once a round in an order that rotates, so none of the
        // three holds a position the others never see, and the middle reading
        // is what each reduces to - which leaves a spike somewhere on the box
        // out of the answer rather than in it.
        let mut taken: Vec<Vec<f64>> = vec![Vec::new(); arms.len()];
        let mut answered: Vec<usize> = vec![0; arms.len()];
        for round in 0..ROUNDS {
            for step in 0..arms.len() {
                let i = (round + step) % arms.len();
                let (ns, h) = once(&arms[i].1, &input);
                taken[i].push(ns);
                answered[i] = h;
            }
        }
        let (hits, plain_hits) = (answered[0], answered[1]);
        assert_eq!(warm_hits, hits, "the warm-up and timed scans must agree");
        assert_eq!(hits, lines, "one line-leading cat per line");
        assert_eq!(plain_hits, lines * 2, "two cats per line without the anchor");

        #[allow(clippy::cast_precision_loss)]
        let lines_f = lines as f64;
        let per = median(taken[0].clone()) / lines_f;
        let plain_per = median(taken[1].clone()) / lines_f;
        let seam_per = median(taken[2].clone()) / lines_f;
        println!(
            "{lines:<10} {:<7} {per:>15.1} {plain_per:>15.1} {seam_per:>15.1}   {hits}",
            input.len()
        );
        if first.is_none() {
            first = Some(per);
        }
        last = Some(per);
    }
    if let (Some(lo), Some(hi)) = (first, last) {
        println!();
        println!("^\"cat\" ns/line {lo:.1} at 250 lines, {hi:.1} at 4000: a ratio of {:.2}x.", hi / lo);
        println!("Flat is linear. Compare the @seam column, which still takes the set engine.");
    }
}
