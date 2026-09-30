//! What walking a pattern set as one program costs against walking its
//! members apart, by set size and input size.
//!
//! The members the single-pass engine takes are either walked as one union
//! program over the shared lex, or each as itself over that lex. Both
//! answer the same; this reads how long each takes for sets of 10, 100 and
//! 1000 patterns over the corpus `benches/vs_regex_full.rs` builds, cut to
//! 64 KiB, 512 KiB and 2 MiB, for the whole-set verdict and for the first
//! spans. A third
//! arm repeats the per-member walk under another name, so the spread between
//! two readings of one thing is on the table beside the spread between the
//! two things.
//!
//! Run: `cargo bench --bench pattern_set_union`.

use std::hint::black_box;
use std::time::Instant;

/// The corpus `benches/vs_regex_full.rs` builds, cut to `bytes`.
fn corpus(bytes: usize) -> Vec<u8> {
    let mut s = String::new();
    let mut i = 0usize;
    while s.len() < bytes {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
        i += 1;
    }
    let mut v = s.into_bytes();
    v.truncate(bytes);
    v
}

/// `n` patterns spread over every route the set takes: literal-led
/// sequences the single-pass engine walks, kind-led ones with a typed
/// predicate, literal runs a byte route settles, balanced groups the set
/// engine owns, and a guard that keeps a member whole.
fn generated(n: usize) -> Vec<trex::ast::Pattern> {
    (0..n)
        .map(|i| {
            let src = match i % 8 {
                0 => format!("\"value_{i}\" \"=\" \\N"),
                1 => format!("\"call_{i}\" \\B(\\W \",\" \\W \",\" \\N)"),
                2 => format!("\"key_{i}\" \":\" \\W"),
                3 => format!("\"cond_{i}\" \")\" \"{{\""),
                4 => format!("\\W \"=\" \\N{{={}}}", i * 37),
                5 => format!("\"item_{i}\" ~\"alpha\""),
                6 => format!("\\N{{>={i}}} \";\""),
                _ => format!("\"do_{i}\" \\B(\\W)"),
            };
            trex::parse(&src).expect("pattern parses")
        })
        .collect()
}

const ROUNDS: usize = 5;
const BUDGET_MS: f64 = 100.0;

/// The mean of as many asks as fill the budget, and one ask where one
/// overruns it: a per-member walk of a thousand patterns over megabytes
/// takes seconds, and a floor of several would turn a cell into minutes.
fn time_budget(f: &mut dyn FnMut() -> usize) -> f64 {
    black_box(f());
    let t0 = Instant::now();
    let mut iters = 0u32;
    loop {
        black_box(f());
        iters += 1;
        if t0.elapsed().as_secs_f64() * 1e3 >= BUDGET_MS {
            break;
        }
    }
    t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// One set size and one input: the per-member walk, the union walk and the
/// per-member walk again as the control, in an order that rotates each round
/// so no arm reads its position as its own cost.
fn cell(input: &[u8], n: usize, spans: bool) -> (f64, f64, f64, usize) {
    let apart_set = trex::PatternSet::new(generated(n)).walked_as_one(false);
    let one_set = trex::PatternSet::new(generated(n));
    let control_set = trex::PatternSet::new(generated(n)).walked_as_one(false);
    let ask = |set: &trex::PatternSet| {
        if spans { set.matches_with_spans(input).len() } else { set.matches(input).len() }
    };
    let mut apart = || ask(&apart_set);
    let mut one = || ask(&one_set);
    let mut control = || ask(&control_set);
    let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
    for r in 0..ROUNDS {
        match r % 3 {
            0 => {
                a.push(time_budget(&mut apart));
                b.push(time_budget(&mut one));
                c.push(time_budget(&mut control));
            }
            1 => {
                b.push(time_budget(&mut one));
                c.push(time_budget(&mut control));
                a.push(time_budget(&mut apart));
            }
            _ => {
                c.push(time_budget(&mut control));
                a.push(time_budget(&mut apart));
                b.push(time_budget(&mut one));
            }
        }
    }
    assert_eq!(apart_set.matches(input), one_set.matches(input), "the two forms disagree");
    assert_eq!(
        apart_set.matches_with_spans(input),
        one_set.matches_with_spans(input),
        "the two forms disagree on first spans"
    );
    (median(a), median(b), median(c), ask(&one_set))
}

/// `n` patterns of one family, so a table can say which family carries a
/// set's cost: literal-led sequences the input holds, literal-led sequences
/// it does not, and kind-led sequences that differ only in their last atom.
fn family(name: &str, n: usize) -> Vec<trex::ast::Pattern> {
    (0..n)
        .map(|i| {
            let src = match name {
                "present" => format!("\"value_{}\" \"=\" \\N", i * 4),
                "absent" => format!("\"nowhere_{i}\" \"=\" \\N"),
                "kind" => format!("\\W \"=\" \\N{{={}}}", i * 37),
                other => panic!("no family {other}"),
            };
            trex::parse(&src).expect("pattern parses")
        })
        .collect()
}

/// The verdict over one family: walked apart, walked as one, walked as one
/// with the literals searched for per member rather than probed once, and
/// the control, in an order that rotates each round.
fn family_cell(input: &[u8], name: &str, n: usize) -> (f64, f64, f64, f64, usize) {
    let apart_set = trex::PatternSet::new(family(name, n)).walked_as_one(false);
    let one_set = trex::PatternSet::new(family(name, n));
    let probed_set = trex::PatternSet::new(family(name, n)).probed(true);
    let control_set = trex::PatternSet::new(family(name, n)).walked_as_one(false);
    let mut arms: [(&mut dyn FnMut() -> usize, Vec<f64>); 4] = [
        (&mut || apart_set.matches(input).len(), Vec::new()),
        (&mut || one_set.matches(input).len(), Vec::new()),
        (&mut || probed_set.matches(input).len(), Vec::new()),
        (&mut || control_set.matches(input).len(), Vec::new()),
    ];
    for r in 0..ROUNDS {
        for k in 0..4 {
            let (f, readings) = &mut arms[(k + r) % 4];
            readings.push(time_budget(&mut **f));
        }
    }
    assert_eq!(apart_set.matches(input), one_set.matches(input), "the two forms disagree");
    assert_eq!(probed_set.matches(input), one_set.matches(input), "the probe changes an answer");
    let [(_, a), (_, b), (_, p), (_, c)] = arms;
    (median(a), median(b), median(p), median(c), one_set.matches(input).len())
}

fn main() {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    println!("trex {}, {cores} cores", trex::version());
    let only_families = std::env::args().any(|a| a == "families");
    println!("\nmatches by family, 1000 patterns: milliseconds per ask, medians of {ROUNDS} rounds");
    println!(
        "{:>10} {:>9} {:>11} {:>11} {:>11} {:>11} {:>9} {:>9} {:>9} {:>8}",
        "bytes", "family", "apart", "as one", "probed", "control", "apart/one", "probe/one", "ctl/apart", "matched"
    );
    for bytes in [64 * 1024, 512 * 1024, 2 * 1024 * 1024] {
        let input = corpus(bytes);
        for name in ["present", "absent", "kind"] {
            let (apart, one, probed, control, matched) = family_cell(&input, name, 1000);
            println!(
                "{bytes:>10} {name:>9} {apart:>11.4} {one:>11.4} {probed:>11.4} {control:>11.4} {:>8.2}x {:>8.2}x {:>8.2}x {matched:>8}",
                apart / one,
                probed / one,
                control / apart
            );
        }
    }
    if only_families {
        return;
    }
    for (question, spans) in [("matches", false), ("matches_with_spans", true)] {
        println!("\n{question}: milliseconds per ask, medians of {ROUNDS} rounds");
        println!(
            "{:>10} {:>9} {:>11} {:>11} {:>11} {:>9} {:>9} {:>8}",
            "bytes", "patterns", "apart", "as one", "control", "apart/one", "ctl/apart", "matched"
        );
        for bytes in [64 * 1024, 512 * 1024, 2 * 1024 * 1024] {
            let input = corpus(bytes);
            for n in [10usize, 100, 1000] {
                let (apart, one, control, matched) = cell(&input, n, spans);
                println!(
                    "{bytes:>10} {n:>9} {apart:>11.4} {one:>11.4} {control:>11.4} {:>8.2}x {:>8.2}x {matched:>8}",
                    apart / one,
                    control / apart
                );
            }
        }
    }
    println!("\napart/one above 1.00 is the union walking faster; ctl/apart is two readings of one thing");
}
