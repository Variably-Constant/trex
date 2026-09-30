//! The single-pass engine over the significant stream in the lexer's parts,
//! read in place, against the same engine over the stitched token stream,
//! each with its lex: what the stitch and the index of significant tokens
//! cost every pattern the byte routes do not take.
//!
//! The patterns are the shapes the routes hand to the engine - a kind, a
//! literal and a kind in sequence, a register bound then referenced, a guard,
//! a bounded repeat - and a bare kind, which the engine scanned at 46 ms over
//! pre-lexed tokens at 19.5 MB before the kind route took it.
//!
//! Every arm rotates its position by round, each of the two is also run
//! against a second copy of itself, and every ratio is the median of the
//! per-round paired ratios. The two arms share a bottleneck - both lex the
//! same bytes across the cores and walk the same stream on one - so a
//! neighbour moves them alike and a control on each reads the disturbance.

use std::time::Instant;

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

/// The mean call over at least three calls and at least `budget_ms`, with
/// the answer of the first.
fn time(budget_ms: f64, mut f: impl FnMut() -> usize) -> (f64, usize) {
    let n = f();
    let t0 = Instant::now();
    let mut calls = 0u32;
    loop {
        std::hint::black_box(f());
        calls += 1;
        if calls >= 3 && t0.elapsed().as_secs_f64() * 1e3 >= budget_ms {
            break;
        }
    }
    (t0.elapsed().as_secs_f64() * 1e3 / f64::from(calls), n)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

const ROUNDS: usize = 7;

fn main() {
    let patterns = [
        ("word, punctuation, number", "\\W \"=\" \\N"),
        ("bound then referenced", "\\W:n \"=\" =n"),
        ("a word with no literal after", "\\W ~\"zzzqqq\""),
        ("one or two numbers", "\\N{1,2}"),
        ("any word", "\\W"),
    ];
    for statements in [20_000usize, 200_000, 520_000] {
        let text = corpus(statements);
        let bytes = text.as_bytes();
        let mb = bytes.len() as f64 / (1024.0 * 1024.0);
        println!("\n{} bytes, {statements} statements", bytes.len());
        println!(
            "  {:<32} {:>11} {:>9} {:>14} {:>11} {:>11}",
            "pattern", "stitched ms", "parts ms", "parts/stitched", "stitched/ctl", "parts/ctl"
        );
        for (label, src) in patterns {
            let p = trex::parse(src).expect("pattern parses");
            let stitched = || {
                trex::parallel_lex::lex_parallel_held(bytes, |toks| {
                    trex::nfa::scan_nfa_over(&p, bytes, toks).map_or(0, |m| m.len())
                })
            };
            let parts = || trex::nfa::scan_nfa(&p, bytes).map_or(0, |m| m.len());
            let n_stitched = stitched();
            let n_parts = parts();
            assert_eq!(n_parts, n_stitched, "{label}: the two forms must agree on the matches");
            // Four arms: each form and a second copy of it, rotated by round.
            let (mut s_ms, mut p_ms) = (Vec::new(), Vec::new());
            let (mut ps, mut s_ctl, mut p_ctl) = (Vec::new(), Vec::new(), Vec::new());
            for round in 0..ROUNDS {
                let mut got = [0.0f64; 4];
                for slot in 0..4 {
                    let arm = (slot + round) % 4;
                    let (ms, _) = match arm {
                        0 | 2 => time(30.0, stitched),
                        _ => time(30.0, parts),
                    };
                    got[arm] = ms;
                }
                s_ms.push(got[0]);
                p_ms.push(got[1]);
                ps.push(got[1] / got[0]);
                s_ctl.push(got[0] / got[2]);
                p_ctl.push(got[1] / got[3]);
            }
            let ratio = median(&mut ps);
            println!(
                "  {label:<32} {:>11.3} {:>9.3} {:>13.3}x {:>10.3}x {:>10.3}x   {n_parts} matches, {:.0} MB/s over parts",
                median(&mut s_ms),
                median(&mut p_ms),
                ratio,
                median(&mut s_ctl),
                median(&mut p_ctl),
                mb / (median(&mut p_ms) / 1e3)
            );
        }
    }
}
