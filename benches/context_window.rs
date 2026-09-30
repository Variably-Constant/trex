//! What the rolling context costs, against refolding the window at every
//! position, and what the grains' boundaries agree on against a shuffled
//! stream.
//!
//! The window fold is one combine a step whatever its extent; the refold is
//! the extent's worth. The two are timed at several extents over the same
//! corpus so the crossover, if there is one, shows. The agreement rates are
//! printed for the corpus and for its significant tokens in a random order,
//! which is the chance level the real rates are read against.

use std::hint::black_box;
use std::time::Instant;

use trex::context::{ContextConfig, WindowProfile, fold_windows, fold_windows_parallel};
use trex::profile::{AxisCtx, AxisProfile};
use trex::supertoken::SuperContext;

fn corpus(statements: usize) -> Vec<u8> {
    let mut s = String::new();
    for i in 0..statements {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {i}) ;\n")),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
    }
    s.into_bytes()
}

/// How many times a reading is taken before its middle one is kept.
const ROUNDS: usize = 5;

/// One arm of the crossover: the extent it folds over, how it folds, and the
/// work it repeats.
struct Arm<'a> {
    window: usize,
    method: &'static str,
    run: Box<dyn Fn() -> usize + 'a>,
}

/// Time `f` once, in milliseconds a call.
fn once(f: &dyn Fn() -> usize, iters: u32) -> f64 {
    let t0 = Instant::now();
    for _ in 0..iters {
        black_box(f());
    }
    t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters)
}

/// The middle of several readings.
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Several readings of one arm, reduced by the middle one.
fn time(iters: u32, f: impl Fn() -> usize) -> f64 {
    black_box(f());
    median((0..ROUNDS).map(|_| once(&f, iters)).collect())
}

/// Every arm once a round, in an order that rotates, each reduced by its
/// median.
///
/// The rounds of one arm are not taken together. A crossover is read off two
/// rows dividing into each other, so a burst of load landing wherever a fixed
/// order happened to be passing carries straight into the verdict; rotating
/// spreads it over every row instead, and the median drops it.
fn sweep(arms: &[Arm], iters: u32) -> Vec<f64> {
    for arm in arms {
        black_box((arm.run)());
    }
    let mut taken: Vec<Vec<f64>> = vec![Vec::with_capacity(ROUNDS); arms.len()];
    for round in 0..ROUNDS {
        for step in 0..arms.len() {
            let i = (step + round) % arms.len();
            taken[i].push(once(&*arms[i].run, iters));
        }
    }
    taken.into_iter().map(median).collect()
}

fn main() {
    let bytes = corpus(2000);
    let toks = trex::lexer::lex(&bytes);
    let spectral = trex::spectral::analyze(&bytes);
    let stress = trex::stress::analyze(&toks, &bytes);
    let echo = trex::echo::analyze(&toks, &bytes);
    let observation = trex::observation::analyze(&bytes);
    let seam = trex::seam::analyze(&bytes);
    let ctx = AxisCtx {
        spectral: Some(&spectral),
        stress: Some(&stress),
        echo: Some(&echo),
        observation: Some(&observation),
        seam: Some(&seam),
        ..AxisCtx::new(&bytes)
    };
    let supers = SuperContext::build(&toks, &bytes);
    let sig: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].is_significant()).collect();
    println!(
        "corpus: {} bytes, {} tokens, {} significant, {} units",
        bytes.len(),
        toks.len(),
        sig.len(),
        supers.units.len()
    );
    // Each token's reading, made once, so the timings below are the fold and
    // not the axis lookups behind it.
    let readings: Vec<WindowProfile> =
        sig.iter().map(|&i| WindowProfile::of_token(i, &toks[i], &ctx)).collect();
    let iters = 20;

    println!(
        "\n{:>8} {:>12} {:>12} {:>12} {:>10}",
        "window", "rolling ms", "parallel ms", "refold ms", "ns/token"
    );
    // Declared before the arms, because an arm borrows them and a binding is
    // dropped in reverse order of declaration.
    let toks_at: &[trex::token::Token] = &toks;
    let ctx_at = &ctx;
    let supers_at = &supers;
    let readings_at: &[WindowProfile] = &readings;
    let mut arms: Vec<Arm> = Vec::new();
    for w in [4usize, 16, 32, 128, 512] {
        let cfg = ContextConfig { token_window: w, unit_window: (w / 4).max(1) };
        arms.push(Arm {
            window: w,
            method: "rolling",
            run: Box::new(move || fold_windows(toks_at, ctx_at, supers_at, &cfg).at_token.len()),
        });
        arms.push(Arm {
            window: w,
            method: "parallel",
            run: Box::new(move || {
                fold_windows_parallel(toks_at, ctx_at, supers_at, &cfg).at_token.len()
            }),
        });
        arms.push(Arm {
            window: w,
            method: "refold",
            run: Box::new(move || {
                let mut out = Vec::with_capacity(readings_at.len());
                for k in 0..readings_at.len() {
                    let lo = k + 1 - w.min(k + 1);
                    out.push(
                        readings_at[lo..=k]
                            .iter()
                            .fold(WindowProfile::identity(), |a, r| a.combine(r)),
                    );
                }
                out.len()
            }),
        });
    }
    // The three methods at one extent are pushed together, so a row reads three
    // consecutive arms and the first of each three is its rolling fold.
    let taken = sweep(&arms, iters);
    for (i, arm) in arms.iter().enumerate().filter(|(_, a)| a.method == "rolling") {
        println!(
            "{:>8} {:>12.3} {:>12.3} {:>12.3} {:>10.1}",
            arm.window,
            taken[i],
            taken[i + 1],
            taken[i + 2],
            taken[i] * 1e6 / sig.len() as f64
        );
    }
    // The same work as the first row, read again once the sweep is done. The
    // distance between the two is the box moving under the run rather than
    // anything an arm did, and it bounds the smallest difference this table can
    // carry.
    let control = median((0..ROUNDS).map(|_| once(&*arms[0].run, iters)).collect());
    println!(
        "  (the control: {control:.3} ms against {:.3} for the {} fold at window {}, {:.3}x)",
        taken[0],
        arms[0].method,
        arms[0].window,
        control / taken[0]
    );
    let per_token = time(iters, || {
        let all: Vec<WindowProfile> =
            sig.iter().map(|&i| WindowProfile::of_token(i, &toks[i], &ctx)).collect();
        all.len()
    });
    println!("  (the per-token readings alone: {per_token:.3} ms, {:.1} ns/token)", per_token * 1e6 / sig.len() as f64);

    println!("\nthe related context: one running fold per relation");
    let period = trex::context::record_period(&toks, &bytes);
    let related = time(iters, || {
        trex::context::relate(&toks, &ctx, &spectral.boundaries, &echo, period).at_token.len()
    });
    println!(
        "  relate (readings across cores, folds in one pass) {related:>8.3} ms  {:.1} ns/token",
        related * 1e6 / sig.len() as f64
    );

    println!("\nthe whole field with every axis built");
    let full = time(5, || trex::context::analyze(&bytes).at_token.len());
    println!("  context::analyze                        {full:>8.3} ms");
    let full_rel = time(5, || trex::context::relate_bytes(&bytes).at_token.len());
    println!("  context::relate_bytes                   {full_rel:>8.3} ms");

    // Patterns reading the context: a log with values bound to a few keys
    // and one latency in every two hundred and fifty planted four orders
    // above its kind. The window predicate knows neighbourhoods and so takes
    // the sizes too; the keyed one knows histories and takes the plants.
    let mut log = String::new();
    let mut planted = 0usize;
    for i in 0..6000 {
        match i % 3 {
            0 => {
                let plant = i % 750 == 300;
                planted += usize::from(plant);
                let v = if plant { 5_000_000 } else { 90 + (i * 37) % 21 };
                log.push_str(&format!("svc_{} latency = {v} ms ;\n", i % 7));
            }
            1 => log.push_str(&format!("svc_{} size = {} ;\n", i % 7, 1_000_000 + i)),
            _ => log.push_str(&format!("state: {} ;\n", if i % 2 == 0 { "ready" } else { "busy" })),
        }
    }
    println!("\npatterns over the context ({} bytes, {planted} planted latencies)", log.len());
    for (label, pat) in [
        ("above the window by two orders", "\\N{>+2}"),
        ("above the window by two sigmas", "\\N{>+2s}"),
        ("above the key's history by one order", "\"latency\":k \"=\" \\N{>+1:k}"),
        ("absolute, six orders", "\\N{mag>6}"),
    ] {
        let p = trex::parse(pat).expect("pattern parses");
        let n = trex::scan(&p, log.as_bytes()).len();
        let ms = time(5, || trex::scan(&p, log.as_bytes()).len());
        println!("  {label:<40} {ms:>8.3} ms  {n} matches");
    }
    // The column anchor needs a record that repeats: six significant tokens a
    // row, no header, and one value in every two hundred rows planted three
    // orders above its column.
    let mut rows = String::new();
    let mut planted_rows = 0usize;
    for i in 0..4000 {
        let plant = i % 200 == 70;
        planted_rows += usize::from(plant);
        let v = if plant { 3_000_000 } else { 1000 + (i * 37) % 900 };
        rows.push_str(&format!("r{} , {v} , x{} ;\n", i % 50, i % 4));
    }
    println!("  periodic rows ({} bytes, {planted_rows} planted values)", rows.len());
    for (label, pat) in [
        ("column two of the period", "@phase:2 \\N"),
        ("above the column by two orders", "\\N{>+2:phase}"),
    ] {
        let p = trex::parse(pat).expect("pattern parses");
        let n = trex::scan(&p, rows.as_bytes()).len();
        let ms = time(5, || trex::scan(&p, rows.as_bytes()).len());
        println!("  {label:<40} {ms:>8.3} ms  {n} matches");
    }

    println!("\nboundary agreement at unit starts, real against shuffled");
    let real = trex::context::analyze(&bytes);
    let mut words: Vec<&[u8]> =
        toks.iter().filter(|t| t.is_significant()).map(|t| &bytes[t.span()]).collect();
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    for i in (1..words.len()).rev() {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        words.swap(i, (x >> 33) as usize % (i + 1));
    }
    let shuffled: Vec<u8> = words.join(&b' ');
    let null = trex::context::analyze(&shuffled);
    let (rr, rs, rm) = real.alignment();
    let (nr, ns, nm) = null.alignment();
    for (name, input) in [("real", &bytes), ("shuffled", &shuffled)] {
        let t = trex::lexer::lex(input);
        let units = trex::supertoken::supertokens_from(&t, input);
        let regime = trex::spectral::analyze(input).boundaries;
        let shape = trex::shape::analyze(&t, input).boundaries;
        let seam = trex::seam::analyze_tokens(&t, &trex::seam::SeamConfig::default());
        println!(
            "  {name}: {} units, {} regime cuts, {} shape cuts, {} seam cuts",
            units.len(),
            regime.len(),
            shape.len(),
            seam.len(),
        );
    }
    println!("  share of units at their role's modal offset of the nearest boundary");
    println!("  {:<24} {:>8} {:>10}", "boundary", "real", "shuffled");
    println!("  {:<24} {rr:>8.3} {nr:>10.3}", "byte regime cut");
    println!("  {:<24} {rs:>8.3} {ns:>10.3}", "token shape cut");
    println!("  {:<24} {rm:>8.3} {nm:>10.3}", "token seam cut");
    // The seam offsets by role: (role, offset, units), the commonest first.
    let by_role = |field: &trex::context::ContextField| {
        let mut counts: std::collections::HashMap<(String, i32), u32> =
            std::collections::HashMap::new();
        for a in &field.agreement {
            if let Some(o) = a.seam {
                *counts.entry((format!("{:?}", a.role), o)).or_insert(0) += 1;
            }
        }
        let mut v: Vec<((String, i32), u32)> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(8);
        v
    };
    println!("  seam offsets by role, real: {:?}", by_role(&real));
    println!("  seam offsets by role, shuffled: {:?}", by_role(&null));
}
