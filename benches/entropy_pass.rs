//! The entropy pre-pass on its own, against the lexer that pays for it.
//!
//! The lexer's sampling profile put `spectral::high_entropy_runs` at 40% of
//! a serial lex of real text. This times the pass alone and the whole lex on
//! the same input, so a change to the pass is read as its share of the lex
//! and not only as its own speed. The input is a file when one is named,
//! otherwise the generated code corpus the other lexer benches use.
//!
//! The pass must give the same runs twice, which the bench asserts before
//! timing; that it flags each byte as the entropy expression does is
//! asserted by `spectral`'s tests. Each figure is the median of seven rounds,
//! taken in an order that alternates so neither arm always follows the other,
//! and the pass is read once more at the end as a control: that row is the
//! same work as the one above it, so the distance between them is the box
//! moving rather than the pass, and it bounds the smallest share reportable.
//!
//!   cargo bench --bench entropy_pass -- [file]

use std::hint::black_box;
use std::time::Instant;

use trex::lexer::{blob_runs, lex};

fn corpus(bytes_wanted: usize) -> Vec<u8> {
    let mut s = String::new();
    let mut i = 0usize;
    while s.len() < bytes_wanted {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
        i += 1;
    }
    s.truncate(bytes_wanted);
    s.into_bytes()
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let named = std::env::args().skip(1).find(|a| !a.starts_with("--"));
    let bytes = match named {
        None => corpus(4 * 1024 * 1024),
        Some(path) => match std::fs::read(&path) {
            Ok(mut b) => {
                b.truncate(16 * 1024 * 1024);
                b
            }
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        },
    };
    let runs = blob_runs(&bytes);
    let again = blob_runs(&bytes);
    assert_eq!(runs, again, "the pass is not deterministic");
    println!("{} bytes, {} blob runs", bytes.len(), runs.len());

    // Which of the two runs first alternates, so neither holds a position the
    // other never sees - the second arm in a round reads a cache the first one
    // left, and a fixed order gives that to the same arm every time.
    let pass = || {
        let t = Instant::now();
        black_box(blob_runs(&bytes));
        t.elapsed().as_secs_f64() * 1e3
    };
    let whole = || {
        let t = Instant::now();
        black_box(lex(&bytes));
        t.elapsed().as_secs_f64() * 1e3
    };
    let (mut pass_ms, mut lex_ms) = (Vec::new(), Vec::new());
    for round in 0..7 {
        if round % 2 == 0 {
            pass_ms.push(pass());
            lex_ms.push(whole());
        } else {
            lex_ms.push(whole());
            pass_ms.push(pass());
        }
    }
    // The pass read once more, after every round. It is the same work as the
    // row above it, so what separates them is the box rather than the pass,
    // and that distance is the smallest share this bench can report.
    let control = pass();
    let (p, l) = (median(&mut pass_ms), median(&mut lex_ms));
    println!(
        "the control reads {control:.2} ms against the pass at {p:.2}: {:.3}x",
        control / p
    );
    println!(
        "entropy pass {p:.2} ms ({:.2} ns/byte), whole lex {l:.2} ms ({:.2} ns/byte), pass is {:.1}% of the lex",
        p * 1e6 / bytes.len() as f64,
        l * 1e6 / bytes.len() as f64,
        100.0 * p / l
    );
}
