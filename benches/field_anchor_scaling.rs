//! Does an `@k` field anchor scale linearly in the token count?
//!
//! `is_field_start` used to count the comma tokens in the whole prefix on
//! every call, once per reachable state per start position, which made an
//! `@k` pattern quadratic where every other construct is linear or near it.
//! The count is a function of the token stream alone, so it is now a per-token
//! index built once per scan and read in constant time.
//!
//! This measures the claim rather than asserting it. The reported figure is
//! nanoseconds per row: flat across the sweep is linear, growing with the row
//! count is quadratic. A control pattern with no field anchor runs on the same
//! input, so a change in the lexer or the engine shows up in both arms and
//! cannot be mistaken for a change in the anchor.

use std::time::Instant;

/// How many times each arm is timed at each size, each in a different order.
const ROUNDS: usize = 5;

/// The middle reading, which is what a round's worth of samples reduces to.
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// A CSV-shaped corpus: `rows` lines of four comma-separated fields.
fn csv(rows: usize) -> Vec<u8> {
    let mut out = String::with_capacity(rows * 24);
    for i in 0..rows {
        out.push_str("alpha,");
        out.push_str(&i.to_string());
        out.push_str(",gamma,delta\n");
    }
    out.into_bytes()
}

/// Wall time of one scan, in nanoseconds, with the match count so a run that
/// silently stopped matching cannot read as a speed-up.
#[allow(clippy::cast_precision_loss)]
fn once(pat: &trex::ast::Pattern, input: &[u8]) -> (f64, usize) {
    let t = Instant::now();
    let matches = trex::scan(pat, input);
    (t.elapsed().as_nanos() as f64, matches.len())
}

fn main() {
    println!("rows      bytes   @3 ns/row   plain ns/row   @3 matches");
    println!("------------------------------------------------------");
    let mut smallest: Option<f64> = None;
    let mut largest: Option<f64> = None;
    let anchored = trex::parse("@3 \\W").expect("pattern parses");
    let plain = trex::parse("\\W").expect("pattern parses");
    for rows in [500usize, 1_000, 2_000, 4_000, 8_000] {
        let input = csv(rows);
        // Warm the allocator and any lazy init so the first timed scan does
        // not include them. The hit count is checked against the timed runs
        // below, so the warm-up is evidence rather than a discarded call.
        let (_, warm_hits) = once(&anchored, &input);

        // Each arm read once a round, and which of the two goes first
        // alternates, so neither holds a position the other never sees. The
        // middle reading is what each reduces to, which leaves a spike
        // somewhere on the box out of the answer rather than in it.
        let (mut anchored_ns, mut plain_ns) = (Vec::new(), Vec::new());
        let (mut hits, mut plain_hits) = (0, 0);
        for round in 0..ROUNDS {
            if round % 2 == 0 {
                let (ns, h) = once(&anchored, &input);
                anchored_ns.push(ns);
                hits = h;
                let (ns, h) = once(&plain, &input);
                plain_ns.push(ns);
                plain_hits = h;
            } else {
                let (ns, h) = once(&plain, &input);
                plain_ns.push(ns);
                plain_hits = h;
                let (ns, h) = once(&anchored, &input);
                anchored_ns.push(ns);
                hits = h;
            }
        }
        assert_eq!(warm_hits, hits, "the warm-up and timed scans must agree");
        assert!(hits > 0, "the anchored pattern must actually match");
        assert!(plain_hits > hits, "the control matches more than the anchored pattern");

        #[allow(clippy::cast_precision_loss)]
        let rows_f = rows as f64;
        let per_row = median(anchored_ns) / rows_f;
        let plain_per_row = median(plain_ns) / rows_f;
        println!(
            "{rows:<9} {:<7} {per_row:>9.1}   {plain_per_row:>12.1}   {hits}",
            input.len()
        );
        if smallest.is_none() {
            smallest = Some(per_row);
        }
        largest = Some(per_row);
    }
    if let (Some(lo), Some(hi)) = (smallest, largest) {
        println!();
        println!("ns/row {lo:.1} at 500 rows, {hi:.1} at 8000: a ratio of {:.2}x.", hi / lo);
        println!(
            "Flat is linear. The quadratic prefix walk this replaced would show \
             roughly 16x across that 16x size range."
        );
    }
}
