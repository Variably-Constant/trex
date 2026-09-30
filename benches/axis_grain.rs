//! What the axis readings cost at each grain.
//!
//! The grain-qualified anchors let a pattern name which stream an axis reads,
//! so the cost of naming one has to be known rather than assumed. Each grain
//! reads a different sequence, and those sequences are very different lengths
//! for the same input - a token stream is a fraction of the byte stream, and a
//! supertoken stream a fraction of that - so the interesting number is not
//! only the time but how it scales with the stream the grain actually walks.
//!
//! Every arm is timed once a round over several rounds, in an order that
//! rotates so no arm holds the first position, and each is reduced by its
//! middle reading, which leaves an outlier out of the answer. One arm is then
//! timed again as a control: it is the same work as the row it repeats, so the
//! distance between the two is the box moving under the run, and it bounds the
//! smallest difference any row here can carry. A reading taken this way holds
//! on a machine with other work on it.
//!
//! Each arm reports what it answered beside what it cost, because a column of
//! milliseconds alone cannot tell a cheaper grain from one that finds less.

use std::hint::black_box;
use std::time::Instant;

/// How many times each arm is timed, each in a different position.
const ROUNDS: usize = 5;

/// A corpus with real structure at every grain: statements, calls, nesting.
fn corpus(statements: usize) -> Vec<u8> {
    let mut s = String::new();
    for i in 0..statements {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
    }
    s.into_bytes()
}

/// The middle reading, which is what a round's worth of samples reduces to.
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// One arm: which group it belongs to, what it is called, and the work.
struct Arm<'a> {
    group: &'static str,
    label: String,
    run: Box<dyn Fn() -> usize + 'a>,
}

/// Time `f` once, in milliseconds a call, with what it answered.
fn once(f: &dyn Fn() -> usize, iters: u32) -> (f64, usize) {
    let t0 = Instant::now();
    let mut found = 0;
    for _ in 0..iters {
        found = black_box(f());
    }
    (t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters), found)
}

fn main() {
    for statements in [200usize, 2000] {
        let bytes = corpus(statements);
        let toks = trex::lexer::lex(&bytes);
        let units = trex::supertoken::supertokens_from(&toks, &bytes);
        let sig = toks.iter().filter(|t| t.is_significant()).count();
        println!(
            "\ncorpus {statements} statements: {} bytes, {sig} significant tokens, {} units",
            bytes.len(),
            units.len()
        );

        let seam_cfg = trex::seam::SeamConfig::default();
        let obs_cfg = trex::observation::ObservationConfig::default();
        let iters = if statements > 1000 { 5 } else { 30 };
        let b: &[u8] = &bytes;
        let t: &[trex::token::Token] = &toks;
        let u: &[trex::supertoken::SuperToken] = &units;

        // Parsed before the arms, because an arm borrows one and a binding is
        // dropped in reverse order of declaration.
        let sources = [
            ("plain \\W", "\\W"),
            ("@seam \\W", "@seam \\W"),
            ("@seam:byte \\W", "@seam:byte \\W"),
            ("@seam:token \\W", "@seam:token \\W"),
            ("@seam:super \\W", "@seam:super \\W"),
            ("@super:assign \\N", "@super:assign \\N"),
        ];
        let parsed: Vec<(&str, trex::ast::Pattern)> = sources
            .iter()
            .map(|(label, src)| (*label, trex::parse(src).expect("pattern parses")))
            .collect();

        let mut arms: Vec<Arm<'_>> = vec![
            // An unqualified `@seam` reads the token grain and `@seam:byte`
            // names the bytes, so both spellings sit beside the grain they
            // read.
            Arm {
                group: "seam, by grain",
                label: "byte".to_string(),
                run: Box::new(|| trex::seam::analyze(b).cuts.len()),
            },
            Arm {
                group: "seam, by grain",
                label: "token".to_string(),
                run: Box::new(move || trex::seam::analyze_tokens(t, &seam_cfg).len()),
            },
            Arm {
                group: "seam, by grain",
                label: "super".to_string(),
                run: Box::new(move || trex::seam::analyze_supertokens(u, &seam_cfg).len()),
            },
            Arm {
                group: "observation, by grain",
                label: "byte".to_string(),
                run: Box::new(|| trex::observation::analyze(b).contested.len()),
            },
            Arm {
                group: "observation, by grain",
                label: "token".to_string(),
                run: Box::new(move || trex::observation::contested_tokens(t, &obs_cfg).len()),
            },
            Arm {
                group: "observation, by grain",
                label: "super".to_string(),
                run: Box::new(move || trex::observation::contested_supertokens(u, &obs_cfg).len()),
            },
            Arm {
                group: "resonator, by alphabet",
                label: "byte class (5)".to_string(),
                run: Box::new(|| trex::resonator::over_bytes(b, 32).periods.len()),
            },
            Arm {
                group: "resonator, by alphabet",
                label: "byte value (256)".to_string(),
                run: Box::new(|| trex::resonator::over_byte_values(b, 32).periods.len()),
            },
            Arm {
                group: "resonator, by alphabet",
                label: "token kind".to_string(),
                run: Box::new(|| trex::resonator::over_tokens(t, 32).periods.len()),
            },
            Arm {
                group: "resonator, by alphabet",
                label: "supertoken role".to_string(),
                run: Box::new(|| trex::resonator::over_supertokens(u, 32).periods.len()),
            },
            Arm {
                group: "the upper-grain window",
                label: "SuperContext::build".to_string(),
                run: Box::new(|| trex::supertoken::SuperContext::build(t, b).units.len()),
            },
        ];

        for (label, p) in &parsed {
            arms.push(Arm {
                group: "a scan that names a grain",
                label: (*label).to_string(),
                run: Box::new(move || trex::scan(p, b).len()),
            });
        }

        // Once each, uncounted, so no arm pays for the pages the first maps or
        // for a table another arm builds lazily.
        for arm in &arms {
            black_box((arm.run)());
        }

        let n = arms.len();
        let mut taken: Vec<Vec<f64>> = (0..n).map(|_| Vec::new()).collect();
        let mut answered: Vec<usize> = vec![0; n];
        for round in 0..ROUNDS {
            for step in 0..n {
                let i = (round + step) % n;
                let (ms, found) = once(&arms[i].run, iters);
                taken[i].push(ms);
                answered[i] = found;
            }
        }
        // The arm of middling cost, because the distance is read as a ratio and
        // a ratio over the cheapest row reads the clock's own grain.
        let middling = {
            let mut ranked: Vec<(usize, f64)> =
                (0..n).map(|i| (i, median(taken[i].clone()))).collect();
            ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
            ranked[n / 2]
        };
        let (control, _) = once(&arms[middling.0].run, iters);

        let mut group = "";
        for (i, arm) in arms.iter().enumerate() {
            if arm.group != group {
                group = arm.group;
                println!(" {group}");
            }
            println!(
                "  {:<38} {:>9.3} ms {:>9} found",
                arm.label,
                median(taken[i].clone()),
                answered[i]
            );
        }
        println!(
            "  {:<38} {control:>9.3} ms  {:>6.3}x against {} at {:.3} ms",
            "the control",
            control / middling.1,
            arms[middling.0].label,
            middling.1
        );
    }
}
