//! What one bump of the seam field's context counting costs, split into the
//! part that is the follower walk and the part that is not.
//!
//! `the seam field: counting the contexts` is 305.566 ms a scan over
//! `benches/engine_surface`, the largest leaf in the engine's phase table that
//! is neither an attempt nor the spectral field. A bump walks 3.49 follower
//! pairs there, and whether holding followers some other way would pay turns
//! entirely on how much of a bump is the walk: the pack, the index and the
//! memory touch are paid whatever the followers sit in, and only the walk is
//! what a different layout could shorten.
//!
//! A clock inside `bump` would cost more than the bump - it runs forty-four
//! million times - so the reading is taken from outside. The context order is
//! the lever: at order one a context is a single byte and holds many followers
//! to walk, and at higher orders contexts are sparse and hold few.
//!
//! A line through the orders cannot separate the two costs, which is what this
//! was written to do and does not. Fitting `ms = a * bumps + b * pairs` over
//! the orders gives `b` at -3.28 ns, and no comparison saves time. The two
//! columns are the trouble: raising the order multiplies bumps and pairs
//! together, four times over from order one to four, while the pairs a bump
//! move only from 4.22 to 3.33. Two nearly parallel columns carry no plane
//! between them, and the normal equations answer anyway, with a determinant
//! small rather than zero and an error that swamps the figure. A guard against
//! an exactly singular fit does not catch it.
//!
//! What the orders do answer is the sign, and the sign is what the question
//! turns on. Each order's own work is the difference from the order below,
//! because a run at `k` builds every order up to `k`. Read that way, the cost a
//! bump rises while the pairs a bump falls - so a bump does not get dearer as
//! it walks further, and the walk is not what a bump costs.
//!
//! It still cannot tell a cause from a companion. Raising the order changes the
//! key width, the number of models and whether the contexts sit in a direct
//! table or a map, and all of that moves together. What it rules out is the one
//! thing it was asked about: a layout that shortens the walk answers a question
//! these readings say is not the one being asked.
//!
//! Every figure above is over the generated statements, four shapes in
//! rotation. How far a bump walks is a property of the contexts a corpus holds,
//! and that corpus holds the fewest distinct ones of any text available here,
//! so the sign it gives is the sign for it. The first argument is a corpus
//! path, and a real one is where a figure quoted elsewhere has to come from.

/// Statements of four shapes, `benches/engine_surface`'s corpus exactly, so a
/// reading here and a reading there are over the same bytes.
fn code(statements: usize) -> String {
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

/// What one order's counting pass reported.
struct Reading {
    order: usize,
    bumps: f64,
    pairs: f64,
    ms: f64,
}

/// The named count, or zero where the run did not report it.
fn count_of(counts: &[(&'static str, u64)], name: &str) -> f64 {
    counts.iter().find(|(n, _)| *n == name).map_or(0.0, |(_, v)| *v as f64)
}

/// Segment `input` at `order` once, watched, and read the counting phase and
/// its counters back.
fn read(input: &[u8], order: usize) -> Reading {
    let cfg = trex::seam::SeamConfig { order, ..trex::seam::SeamConfig::default() };
    // An unwatched pass first: the models allocate, and a share of the first
    // call is the allocator rather than the counting.
    drop(trex::seam::analyze_with(input, &cfg));

    trex::trace::record();
    drop(trex::trace::take_phases());
    drop(trex::trace::take_counts());
    drop(trex::seam::analyze_with(input, &cfg));
    let phases = trex::trace::take_phases();
    let counts = trex::trace::take_counts();
    trex::trace::stop();

    let ms = phases
        .iter()
        .find(|(n, _, _)| *n == "the seam field: counting the contexts")
        .map_or(0.0, |(_, _, d)| d.as_secs_f64() * 1e3);
    let bumps = count_of(&counts, "the seam field: bumps into a direct table")
        + count_of(&counts, "the seam field: bumps into a map");
    let pairs = count_of(&counts, "the seam field: follower pairs a bump walks");
    Reading { order, bumps, pairs, ms }
}

/// What one more order added over the order below it.
///
/// The totals cannot be read directly, because a run at order `k` builds every
/// order from one to `k`: the row for four holds the work of one, two and three
/// as well. A difference between neighbouring rows is one order's own work, and
/// it is the difference rather than the total that says what a bump costs where
/// the followers are thin.
struct Marginal {
    order: usize,
    ms: f64,
    bumps: f64,
    pairs: f64,
}

fn marginals(rows: &[Reading]) -> Vec<Marginal> {
    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let (ms, bumps, pairs) = match i {
            0 => (r.ms, r.bumps, r.pairs),
            _ => (r.ms - rows[i - 1].ms, r.bumps - rows[i - 1].bumps, r.pairs - rows[i - 1].pairs),
        };
        out.push(Marginal { order: r.order, ms, bumps, pairs });
    }
    out
}

/// Whether what a bump costs moves with how far it walks, over the marginals.
///
/// This is the whole question, and it is a sign rather than a slope: if a bump
/// gets dearer as it walks further, the walk is worth shortening, and if it
/// gets dearer as it walks LESS, then whatever else changed between the orders
/// is the cost and the walk is not.
fn walk_and_cost_agree(m: &[Marginal]) -> bool {
    let ns = |x: &Marginal| x.ms * 1e6 / x.bumps;
    let per = |x: &Marginal| x.pairs / x.bumps;
    m.windows(2).all(|w| (ns(&w[1]) - ns(&w[0])) * (per(&w[1]) - per(&w[0])) >= 0.0)
}

fn main() {
    // A corpus by path, because how far a bump walks turns on how diverse the
    // contexts are, and the generated statements are four shapes in rotation.
    // With no path the generated input still runs and says so, so a reading
    // over it is never taken for one over real text.
    let (input, whose) = match std::env::args().nth(1) {
        Some(path) => match std::fs::read(&path) {
            Ok(bytes) => (bytes, path),
            Err(e) => {
                eprintln!("the corpus at {path} could not be read: {e}");
                std::process::exit(2);
            }
        },
        None => (
            code(200_000).into_bytes(),
            "generated statements, four shapes in rotation".to_string(),
        ),
    };
    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("corpus {whose}, {} bytes, the seam field's byte grain\n", input.len());

    println!(
        "{:>6} {:>13} {:>14} {:>11} {:>11} {:>11}",
        "order", "bumps", "pairs walked", "a bump", "ms", "ns a bump"
    );
    let rows: Vec<Reading> = (1..=4).map(|o| read(&input, o)).collect();
    for r in &rows {
        let per = if r.bumps > 0.0 { r.pairs / r.bumps } else { 0.0 };
        let ns = if r.bumps > 0.0 { r.ms * 1e6 / r.bumps } else { 0.0 };
        println!(
            "{:>6} {:>13.0} {:>14.0} {:>11.2} {:>11.3} {:>11.2}",
            r.order, r.bumps, r.pairs, per, r.ms, ns
        );
    }

    let m = marginals(&rows);
    println!("\nwhat each order added over the one below it");
    println!(
        "{:>6} {:>13} {:>14} {:>11} {:>11} {:>11}",
        "order", "bumps", "pairs walked", "a bump", "ms", "ns a bump"
    );
    for x in &m {
        println!(
            "{:>6} {:>13.0} {:>14.0} {:>11.2} {:>11.3} {:>11.2}",
            x.order,
            x.bumps,
            x.pairs,
            x.pairs / x.bumps,
            x.ms,
            x.ms * 1e6 / x.bumps
        );
    }

    println!();
    if walk_and_cost_agree(&m) {
        println!("a bump gets dearer as it walks further, so the walk is worth shortening");
    } else {
        println!("a bump does NOT get dearer as it walks further: over these orders the");
        println!("cost a bump rises while the pairs a bump falls, so what a bump costs is");
        println!("not its follower walk, and holding followers some other way answers a");
        println!("question the readings say is not the one being asked.");
    }
}
