//! Where the seam's time actually goes, at both grains.
//!
//! The segmentation counts a context per symbol at every order in both
//! directions, so what it costs follows the length of the sequence it reads and
//! the number of orders it reads at. `@seam` reads the token sequence and
//! `@seam:byte` the bytes under it, and the two are far apart: over 7.34 MB of
//! source `examples/where_the_time_goes` puts the byte grain at 1095.373 ms and
//! the token grain at 337.231. Both are here, because the knobs are shared and
//! an optimization to the model reaches both.
//!
//! Three knobs could each explain a reading: the context order (how many n-gram
//! models are built), the pass count (the reading is iterated), and the fusions
//! the byte path runs. This separates them, so an optimization targets the one
//! that actually costs time rather than the one that looks slow.
//!
//! Every arm is timed once a round over several rounds in an order that
//! ROTATES, and each is reduced by its MEDIAN rather than its mean. Then one
//! arm is timed again as a CONTROL. A mean carries an outlier in whole; a fixed
//! order gives whichever arm runs first whatever that position is worth; and
//! with nothing re-timed there is no way to tell a box that moved from a number
//! that moved. The control is the same work as the arm it repeats, so the
//! distance between them bounds the smallest difference any row here can carry.

use std::hint::black_box;
use std::time::Instant;

/// How many times each arm is timed, each in a different position.
const ROUNDS: usize = 5;

/// How many calls one timing averages over, which amortizes the clock.
const ITERS: u32 = 3;

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

/// An input with no fixed period: line lengths, vocabulary and punctuation all
/// vary, and the structure does not repeat every fourth line.
///
/// A context model is asked how well a sequence predicts itself, so a corpus
/// built from four statement shapes in rotation is the most predictable text
/// there is, and a knob that looks inert on it can be load-bearing elsewhere.
/// Two inputs of different regularity are the least that can say which.
fn irregular(lines: usize) -> Vec<u8> {
    const WORDS: [&str; 12] = [
        "the", "quick", "brown", "resonance", "of", "a", "settled", "boundary", "walks",
        "between", "predictable", "thresholds",
    ];
    let mut s = String::new();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for i in 0..lines {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let len = 3 + (seed >> 33) as usize % 14;
        for j in 0..len {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            s.push_str(WORDS[(seed >> 33) as usize % WORDS.len()]);
            if j + 1 < len {
                s.push(' ');
            }
        }
        s.push_str(match i % 5 {
            0 => ".\n",
            1 => ", and\n",
            2 => "?\n",
            3 => ";\n",
            _ => "\n",
        });
    }
    s.into_bytes()
}

/// The middle reading, which is what a round's worth of samples reduces to.
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// One arm: which group it belongs to, what it is called, and the work.
///
/// The work answers how many cuts it found, so a row carries what it costs
/// beside what it says. A knob that halves a reading by finding half as much
/// is not an optimization, and a column of milliseconds alone cannot tell the
/// two apart.
struct Arm<'a> {
    group: &'static str,
    label: String,
    run: Box<dyn Fn() -> usize + 'a>,
}

/// Time `f` once, in milliseconds a call, with what it answered.
fn once(f: &dyn Fn() -> usize) -> (f64, usize) {
    let t0 = Instant::now();
    let mut found = 0;
    for _ in 0..ITERS {
        found = f();
    }
    (t0.elapsed().as_secs_f64() * 1e3 / f64::from(ITERS), found)
}

fn main() {
    let bytes = corpus(2000);
    println!("corpus: {} bytes", bytes.len());
    let base = trex::seam::SeamConfig::default();
    println!(
        "default: order={} passes={} spectral_fusion={} lexer_fusion={}",
        base.order, base.passes, base.spectral_fusion, base.lexer_fusion
    );
    println!("{ROUNDS} rounds an arm, {ITERS} calls a round, the order rotating\n");

    let toks = trex::lexer::lex(&bytes);
    let sbase = trex::spectral::SpectralConfig::default();
    // Every arm reads the same input and the same tokens. A slice is Copy, so
    // an arm that moves its own config in still shares these rather than
    // taking a copy of the corpus each.
    let b: &[u8] = &bytes;
    let t: &[trex::token::Token] = &toks;
    // Declared before the arms, because an arm holds a slice of one and a
    // binding is dropped in reverse order of declaration.
    // Up to the size `examples/where_the_time_goes` reads, because a rate that
    // holds over a megabyte can still bend by seven, and the per-axis figures
    // quoted for this field are taken at the larger one.
    let sizes: Vec<Vec<u8>> =
        [500usize, 2_000, 8_000, 32_000, 128_000, 200_000].iter().map(|n| corpus(*n)).collect();

    // The same symbols under the two containers a context model can be. It
    // holds a direct-indexed table where the packed key fits in its width and
    // a hash map above that, so widening the alphabet moves the same stream
    // from the one to the other. Shifting every symbol left widens it without
    // changing the stream's length, how many distinct symbols it holds, or how
    // many bumps a pass takes, which leaves the container as the only
    // difference between the two arms.
    let narrow: Vec<u32> =
        toks.iter().filter(|k| k.is_significant()).map(|k| k.kind.code()).collect();
    let wide: Vec<u32> = narrow.iter().map(|s| s << 14).collect();
    let at_one = trex::seam::SeamConfig { order: 1, ..base };
    let mut arms: Vec<Arm<'_>> = Vec::new();

    for passes in [1usize, 2, 3] {
        let cfg = trex::seam::SeamConfig { passes, ..base };
        arms.push(Arm {
            group: "pass count, everything else default",
            label: format!("passes={passes}"),
            run: Box::new(move || black_box(trex::seam::analyze_with(b, &cfg)).cuts.len()),
        });
    }

    for order in [1usize, 2, 3, 4] {
        let cfg = trex::seam::SeamConfig { order, passes: 1, ..base };
        arms.push(Arm {
            group: "context order over the bytes, one pass",
            label: format!("order={order}"),
            run: Box::new(move || black_box(trex::seam::analyze_with(b, &cfg)).cuts.len()),
        });
    }

    // The same knob over the token sequence, which an unqualified `@seam`
    // reads. The order is clamped by what packs into the key at the width of
    // the alphabet in use, and token kinds are wider than bytes, so a reading
    // here is not the byte reading scaled.
    for order in [1usize, 2, 3, 4] {
        let cfg = trex::seam::SeamConfig { order, ..base };
        arms.push(Arm {
            group: "context order over the token kinds",
            label: format!("order={order}"),
            run: Box::new(move || black_box(trex::seam::analyze_tokens(t, &cfg)).len()),
        });
    }

    let one = trex::seam::SeamConfig { passes: 1, ..base };
    for (label, cfg) in [
        ("both fusions".to_string(), one),
        ("no spectral fusion".to_string(), trex::seam::SeamConfig { spectral_fusion: false, ..one }),
        ("no lexer fusion".to_string(), trex::seam::SeamConfig { lexer_fusion: false, ..one }),
        (
            "neither fusion".to_string(),
            trex::seam::SeamConfig { spectral_fusion: false, lexer_fusion: false, ..one },
        ),
    ] {
        arms.push(Arm {
            group: "the fusions, one pass at the default order",
            label,
            run: Box::new(move || black_box(trex::seam::analyze_with(b, &cfg)).cuts.len()),
        });
    }

    arms.push(Arm {
        group: "inside the spectral pass the byte fusion can run",
        label: "spectral::analyze (default)".to_string(),
        run: Box::new(|| black_box(trex::spectral::analyze(b)).boundaries.len()),
    });
    // The autocorrelation is the one quadratic-shaped term: it walks
    // period_window bytes at every lag up to max_lag, once per period_hop.
    // Raising the hop past the input evaluates it once, which is as close to
    // off as the config gets.
    let hop = trex::spectral::SpectralConfig { period_hop: bytes.len(), ..sbase };
    arms.push(Arm {
        group: "inside the spectral pass the byte fusion can run",
        label: "period evaluated once (hop = len)".to_string(),
        run: Box::new(move || black_box(trex::spectral::analyze_with(b, &hop)).boundaries.len()),
    });
    let lag = trex::spectral::SpectralConfig { max_lag: 16, ..sbase };
    arms.push(Arm {
        group: "inside the spectral pass the byte fusion can run",
        label: "max_lag 16 rather than 128".to_string(),
        run: Box::new(move || black_box(trex::spectral::analyze_with(b, &lag)).boundaries.len()),
    });

    // One axis at a time, read from the field directly. A pattern cannot spell
    // every combination - `\F{texture:name}` takes the class mix and the
    // entropy together, and `\F{onset}` takes both and the change-point - so a
    // reading taken through an atom carries whatever that atom rests on. A
    // `Needs` built here separates them, which is what says whether the
    // change-point or the mix it measures over is the term that costs.
    let none = trex::spectral::Needs::none();
    for (label, needs) in [
        ("entropy alone", trex::spectral::Needs { entropy: true, ..none }),
        ("the class mix alone", trex::spectral::Needs { bands: true, ..none }),
        (
            "the change-point over the mix",
            trex::spectral::Needs { bands: true, onset: true, ..none },
        ),
        ("the period pass alone", trex::spectral::Needs { period: true, ..none }),
        ("the k-gram surprise alone", trex::spectral::Needs { novelty: true, ..none }),
        ("every reading", trex::spectral::Needs::all()),
    ] {
        arms.push(Arm {
            group: "the spectral field, an axis at a time",
            label: label.to_string(),
            run: Box::new(move || {
                black_box(trex::spectral::analyze_needing(b, &sbase, needs)).frames.len()
            }),
        });
    }

    // The change-point at several sizes, reported per byte. The autocorrelation
    // bounds its own total work through PERIOD_OP_BUDGET, deriving its hop from
    // the input length; the change-point has no such bound, so whether its cost
    // a byte holds as the input grows is a question about the term rather than
    // about its share. A row that rises across these sizes is the answer.
    // The entropy is in both, because the change-point measures how far the
    // class mix and the entropy have diverged: `Needs::of` gives an onset atom
    // all three, and a pair that leaves the entropy out isolates a reading the
    // engine never takes. With it out, the marginal cost here reads 52.1 ms
    // over 7.34 MB where the same difference between the atoms reads 144.1.
    let onset = trex::spectral::Needs { entropy: true, bands: true, onset: true, ..none };
    let mix = trex::spectral::Needs { entropy: true, bands: true, ..none };
    for text in &sizes {
        let at: &[u8] = text;
        arms.push(Arm {
            group: "the change-point as the input grows",
            label: format!("the mix alone over {} bytes", at.len()),
            run: Box::new(move || black_box(trex::spectral::analyze_needing(at, &sbase, mix)).frames.len()),
        });
        arms.push(Arm {
            group: "the change-point as the input grows",
            label: format!("the change-point over {} bytes", at.len()),
            // The boundaries summed rather than counted. Four of them over
            // seven megabytes is a thin answer: a boundary that moves without
            // another appearing leaves the count where it was, and the column
            // then reads as agreement across a change that moved the answer.
            run: Box::new(move || {
                black_box(trex::spectral::analyze_needing(at, &sbase, onset))
                    .boundaries
                    .iter()
                    .sum()
            }),
        });
    }

    for (label, symbols) in [("a direct table", &narrow), ("a hash map", &wide)] {
        let s: &[u32] = symbols;
        arms.push(Arm {
            group: "one order over one stream, by the container it lands in",
            label: label.to_string(),
            run: Box::new(move || black_box(trex::seam::analyze_symbols(s, &at_one)).cuts.len()),
        });
    }

    arms.push(Arm {
        group: "the same period question, asked by the resonator",
        label: "resonator over_bytes, one causal pass".to_string(),
        run: Box::new(|| black_box(trex::resonator::over_bytes(b, 32)).periods.len()),
    });

    // Once each, uncounted, so no arm's time includes the pages the first one
    // maps.
    for arm in &arms {
        (arm.run)();
    }

    let n = arms.len();
    let mut taken: Vec<Vec<f64>> = (0..n).map(|_| Vec::new()).collect();
    let mut answered: Vec<usize> = vec![0; n];
    for round in 0..ROUNDS {
        for step in 0..n {
            let i = (round + step) % n;
            let (ms, found) = once(&arms[i].run);
            taken[i].push(ms);
            answered[i] = found;
        }
    }
    // The arm of middling cost, because the distance is read as a ratio and a
    // ratio over the cheapest row reads the clock's own grain.
    let middling = {
        let mut ranked: Vec<(usize, f64)> =
            (0..n).map(|i| (i, median(taken[i].clone()))).collect();
        ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
        ranked[n / 2]
    };
    let (control, _) = once(&arms[middling.0].run);

    let mut group = "";
    for (i, arm) in arms.iter().enumerate() {
        if arm.group != group {
            group = arm.group;
            println!(" {group}");
        }
        println!(
            "  {:<44} {:>9.3} ms {:>9} found",
            arm.label,
            median(taken[i].clone()),
            answered[i]
        );
    }
    println!(
        "\n  {:<44} {control:>9.3} ms  {:>6.3}x against {} at {:.3} ms",
        "the control",
        control / middling.1,
        arms[middling.0].label,
        middling.1
    );

    // Whether a cheaper order answers the same question, which is what tells a
    // knob that is waste from one that is a trade. Two readings can hold the
    // same count of cuts and put them in different places, so the sets are
    // compared rather than their lengths.
    println!("\n the same question at another order, against the default of {}", base.order);
    let other = irregular(4000);
    // Two inputs nobody generated for this question: real prose with markup,
    // and real source. A corpus written alongside the measurement can be
    // regular in whatever way the measurement is blind to.
    let prose = include_str!("../wiki/content/docs/reference/pattern-syntax.md").as_bytes();
    let source = include_str!("../src/seam.rs").as_bytes();
    for (what, text) in [
        ("the rotating corpus", b),
        ("an irregular one", &other[..]),
        ("real prose", prose),
        ("real source", source),
    ] {
        let toks = trex::lexer::lex(text);
        // The byte path unions every token start in when `lexer_fusion` is on,
        // which is most of what it answers, so the order is asked with the
        // union and again without it. An order that only stops mattering with
        // the union on is an entropy model whose reading the union covers.
        for fused in [true, false] {
            let at = |order: usize| trex::seam::SeamConfig {
                order,
                passes: 1,
                lexer_fusion: fused,
                ..base
            };
            let want = trex::seam::analyze_with(text, &at(base.order)).cuts;
            for order in [1usize, 2, 4] {
                let got = trex::seam::analyze_with(text, &at(order)).cuts;
                let verdict = if got == want {
                    "the same cuts".to_string()
                } else {
                    format!("{} cuts where the default finds {}", got.len(), want.len())
                };
                let fusion = if fused { "fused" } else { "unfused" };
                println!("  {:<44} {verdict}", format!("{what}, bytes {fusion}, order={order}"));
            }
        }
        let want = trex::seam::analyze_tokens(&toks, &base);
        for order in [1usize, 2, 4] {
            let cfg = trex::seam::SeamConfig { order, ..base };
            let got = trex::seam::analyze_tokens(&toks, &cfg);
            let verdict = if got == want {
                "the same cuts".to_string()
            } else {
                format!("{} cuts where the default finds {}", got.len(), want.len())
            };
            println!("  {:<44} {verdict}", format!("{what}, token kinds, order={order}"));
        }
    }

    // Whether a cheaper period answers the same question. The two knobs the
    // arms above price - the hop deciding how often the autocorrelation runs,
    // and the largest lag it searches - change what the pass reads and not only
    // what it costs, so the time they save is a trade until a reading says
    // which one it is.
    //
    // The period is carried per frame rather than in the cut list, so the cuts
    // alone cannot report it: `period` and `period_strength` on a frame are
    // what a consumer reads, and a setting that leaves every boundary where it
    // was can still move the reading under all of them. Both are counted, and
    // the strength by its bits, because a peak height that shifts in its last
    // place is a different answer to the same question.
    println!("\n the same period question at a cheaper setting, against the default");
    for (what, text) in [
        ("the rotating corpus", b),
        ("an irregular one", &other[..]),
        ("real prose", prose),
        ("real source", source),
    ] {
        let want = trex::spectral::analyze_with(text, &sbase);
        for (label, cfg) in [
            ("hop = len", trex::spectral::SpectralConfig { period_hop: text.len(), ..sbase }),
            ("max_lag 16", trex::spectral::SpectralConfig { max_lag: 16, ..sbase }),
        ] {
            let got = trex::spectral::analyze_with(text, &cfg);
            let cuts = if got.boundaries == want.boundaries {
                "the same cuts".to_string()
            } else {
                format!(
                    "{} cuts where the default finds {}",
                    got.boundaries.len(),
                    want.boundaries.len()
                )
            };
            // A frame count that differs makes the pairing meaningless, so it
            // is reported instead of a difference taken over the shorter.
            let frames = if got.frames.len() == want.frames.len() {
                let pairs = got.frames.iter().zip(&want.frames);
                let moved = pairs.clone().filter(|(g, w)| g.period != w.period).count();
                let swung = pairs
                    .filter(|(g, w)| g.period_strength.to_bits() != w.period_strength.to_bits())
                    .count();
                format!(
                    "{moved} of {} frames read another period, {swung} another strength",
                    want.frames.len()
                )
            } else {
                format!("{} frames where the default has {}", got.frames.len(), want.frames.len())
            };
            println!("  {:<44} {cuts}, {frames}", format!("{what}, {label}"));
        }
    }

    // How often the backoff is actually taken, and what the counting pass built
    // so that it could be.
    //
    // The counting pass bumps a model at every order at every position; the
    // reading pass tries the longest order first and stops at the first that
    // clears `min_count`. So an input whose longest order has observations
    // everywhere spends time on the shorter models and never reads them. Over the
    // rotating corpus the reading backed off 44 times in 14,684,996 - but that
    // corpus is four statement shapes repeated, which is the most repetitive
    // text there is, and says nothing about text nobody generated for this.
    println!("\n how often the backoff is taken, against what was built for it");
    for (what, text) in [
        ("the rotating corpus", b),
        ("an irregular one", &other[..]),
        ("real prose", prose),
        ("real source", source),
    ] {
        trex::trace::record();
        drop(trex::trace::take_counts());
        black_box(trex::seam::analyze_with(text, &base).cuts.len());
        let counts = trex::trace::take_counts();
        trex::trace::stop();
        let at = |want: &str| {
            counts
                .iter()
                .find(|(n, _)| *n == want)
                .map_or_else(|| panic!("the seam did not report {want}"), |(_, v)| *v)
        };
        let looked = at("the seam field: contexts looked up for an entropy");
        let read = at("the seam field: entropies read");
        let table = at("the seam field: bumps into a direct table");
        let map = at("the seam field: bumps into a map");
        println!(
            "  {what:<22} backed off {:>9} of {read:>10} reads; built {table:>10} into tables, {map:>10} into the map",
            looked - read
        );
    }
}
