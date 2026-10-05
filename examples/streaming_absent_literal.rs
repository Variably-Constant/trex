//! What a streaming scan costs against a whole-input scan of the same bytes,
//! and where a push spends it.
//!
//! Both arms answer the same question over the same input, so what separates
//! them is work one does and the other does not. The arms differ only in
//! whether the pattern's required literal occurs: absent, the whole-input scan
//! refuses without lexing a byte, so the comparison is against a floor; present,
//! it scans and reports fifty thousand matches, so the comparison is against a
//! real scan. A change that moves the second is a change to scanning rather
//! than to refusing.
//!
//! The ratio each arm prints - the chunked time over the whole-input time of
//! the same run - is what carries a reading, because this box is shared and its
//! absolute times move by more than half between runs.
//!
//! Every phase and every counter the chunked arm records is printed rather than
//! the ones looked up by name. A push's scan is a whole scan entry point, so
//! whatever the engine times inside itself is timed here too, and naming only
//! the streaming phases hid exactly the ones that said where the cost went.
//!
//! Run: `cargo run --release --example streaming_absent_literal [corpus]`,
//! where a corpus names a file read instead of the generated statements. The
//! chunk-size rows want one: a corpus of four statement shapes in rotation
//! has a period, and a chunk size that lands on a multiple of it holds
//! different bytes at its edges than one that does not, so a cost belonging
//! to the generator can read as a cost belonging to the chunk size.

use std::time::Instant;

/// Statements of four shapes, the corpus the engine-surface bench reads, so a
/// reading here and a reading there are over the same bytes.
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

/// One arm's reading: the median of its rounds in milliseconds, the fastest and
/// slowest of them, and what it answered.
struct Arm {
    median: f64,
    low: f64,
    high: f64,
    found: usize,
}

impl Arm {
    /// How far this arm moved across its own rounds, as the slowest over the
    /// fastest. A reading whose two arms are both near 1.00 was taken on a box
    /// that held still; one that does not says so rather than hiding it in a
    /// median.
    fn spread(&self) -> f64 {
        self.high / self.low
    }
}

/// The median of `rounds` timings of `f`, in milliseconds, past one that is not
/// counted so the buffers it maps are already mapped, with the spread of those
/// rounds beside it: a row whose spread is wide is the box moving, and without
/// it an outlier reads exactly like a result.
fn median_ms(rounds: usize, f: &mut dyn FnMut() -> usize) -> Arm {
    let found = f();
    let mut times: Vec<f64> = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let began = Instant::now();
        let n = f();
        times.push(began.elapsed().as_secs_f64() * 1e3);
        assert_eq!(n, found, "every round answers the same");
    }
    times.sort_by(f64::total_cmp);
    Arm { median: times[times.len() / 2], low: times[0], high: times[times.len() - 1], found }
}

/// Both arms over `rounds` rounds, timed in turn rather than in two blocks.
///
/// Two blocks measure two different boxes whenever the load drifts through a
/// run, and the ratio of their medians then reports that drift rather than the
/// arms. Alternating puts both under whatever the box is doing at the same
/// moment, so the drift lands on both and divides out. This machine is shared
/// with other work, so that is the difference between a ratio and a number.
///
/// Each arm's spread comes back with it, because alternating makes the ratio
/// survive drift but does not make a wildly drifting run worth quoting.
fn rotated_ms(
    rounds: usize,
    a: &mut dyn FnMut() -> usize,
    b: &mut dyn FnMut() -> usize,
) -> (Arm, Arm) {
    // One call of each, timed: it maps their buffers, and it sizes the repeats
    // below.
    let began = Instant::now();
    let found_a = a();
    let first_a = began.elapsed().as_secs_f64();
    let began = Instant::now();
    let found_b = b();
    let first_b = began.elapsed().as_secs_f64();

    // The cheaper arm is repeated inside its round until the round costs about
    // what the other's does. A reading of a fifth of a millisecond beside one
    // of sixty is not a smaller number of the same kind - it is mostly the
    // scheduler, and a ratio over it reports the scheduler. Measured here at
    // 1.26x spread on the cheap arm against 1.02x on the dear one. The counts
    // come from the two calls above rather than from a constant, so a corpus
    // that moves the balance moves them with it.
    let reps_a = repeats_for(first_b, first_a);
    let reps_b = repeats_for(first_a, first_b);

    let mut ta: Vec<f64> = Vec::with_capacity(rounds);
    let mut tb: Vec<f64> = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let began = Instant::now();
        for _ in 0..reps_a {
            assert_eq!(a(), found_a, "every round of the first arm answers the same");
        }
        ta.push(began.elapsed().as_secs_f64() * 1e3 / reps_a as f64);
        let began = Instant::now();
        for _ in 0..reps_b {
            assert_eq!(b(), found_b, "every round of the second arm answers the same");
        }
        tb.push(began.elapsed().as_secs_f64() * 1e3 / reps_b as f64);
    }
    ta.sort_by(f64::total_cmp);
    tb.sort_by(f64::total_cmp);
    (
        Arm { median: ta[ta.len() / 2], low: ta[0], high: ta[ta.len() - 1], found: found_a },
        Arm { median: tb[tb.len() / 2], low: tb[0], high: tb[tb.len() - 1], found: found_b },
    )
}

/// Every match a set of `members` makes over `chunks` fed as a stream, as
/// [`trex::scan_chunked`] does for one pattern. A stream takes ownership of
/// its set and a set does not clone, so the set is built per call from the
/// members; the whole-input arm builds one per call the same way, so the
/// construction is inside both arms of the comparison rather than one.
fn fed_to_a_set(members: &[trex::ast::Pattern], chunks: &[&[u8]]) -> usize {
    let mut s = trex::StreamScanner::over_set(trex::PatternSet::new(members.to_vec()));
    for c in chunks.iter().copied() {
        s.push(c);
    }
    s.finish_with_members().len()
}

/// How many calls of something taking `own` seconds make a round costing about
/// `other` seconds, and never fewer than one.
fn repeats_for(other: f64, own: f64) -> usize {
    if own <= 0.0 {
        return 1;
    }
    let n = (other / own).round();
    if n < 1.0 { 1 } else { n as usize }
}

fn main() {
    // A real file where one is named, the generated corpus otherwise, and the
    // header says which: a table that does not name its corpus is one that
    // gets compared against a table read over different bytes. The chunk-size
    // rows are why naming a file matters here. Four statement shapes in
    // rotation give the generated corpus a period, so a chunk size landing on
    // a multiple of that period is not the same input as one landing between
    // two, and a cost that shows at one size alone can be a fact about the
    // generator rather than about the stream.
    let (input, whose) = match std::env::args().nth(1) {
        None => (corpus(200_000).into_bytes(), "generated statements".to_string()),
        Some(path) => match std::fs::read(&path) {
            Ok(bytes) => (bytes, path),
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        },
    };
    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("corpus {} bytes, {whose}\n", input.len());

    // Above the parallel lexer's threshold, so each chunk reaches the pool and
    // the row measures the streaming and not the chunk size.
    let chunk = trex::parallel_lex::active_parallel_threshold().saturating_mul(2).max(64 << 10);
    let chunks: Vec<&[u8]> = input.chunks(chunk).collect();
    println!("{} chunks of {} kB\n", chunks.len(), chunk >> 10);

    for (label, src) in [
        ("the literal is absent", "\"quetzalcoatl\" \\W"),
        ("the literal is present  (control)", "\"let\" \\W"),
    ] {
        let p = trex::parse(src).expect("pattern parses");
        let (whole, fed) = rotated_ms(
            5,
            &mut || trex::scan(&p, &input).len(),
            &mut || trex::scan_chunked(&p, chunks.iter().copied()).len(),
        );
        assert_eq!(whole.found, fed.found, "{label}: both arms answer the same");
        println!("{label}  -  {} matches", whole.found);
        println!(
            "{:>34} {:>9.3} ms   spread {:>5.2}x",
            "over the whole input",
            whole.median,
            whole.spread()
        );
        println!(
            "{:>34} {:>9.3} ms   spread {:>5.2}x",
            "fed in chunks",
            fed.median,
            fed.spread()
        );
        // The load-normalized figure, and the only one comparable across runs:
        // the arms alternate, so whatever the box was doing it was doing to
        // both. The milliseconds above are comparable only within one run.
        println!("{:>34} {:>9.2}x\n", "the streaming costs", fed.median / whole.median);

        // What the chunked arm spends itself on, from one feed after the timing
        // rather than during it, so the phases do not charge the rows above.
        // A push lexes the buffer it retains, gathers the significant tokens of
        // the prefix it can decide, and scans from the last committed match;
        // both commit paths carry the same three names, so the reading is one
        // quantity whichever path a pattern takes.
        trex::trace::record();
        drop(trex::trace::take_phases());
        drop(trex::trace::take_counts());
        drop(trex::scan_chunked(&p, chunks.iter().copied()));
        let phases = trex::trace::take_phases();
        let counts = trex::trace::take_counts();
        trex::trace::stop();
        // Every phase the chunked arm records, not a chosen few: the scan it
        // makes per push is a whole scan entry point, so whatever the engine
        // times inside itself is timed here too, and naming only the streaming
        // phases would hide exactly the ones that say where the per-push cost
        // goes.
        assert!(
            phases.iter().any(|(had, _, _)| *had == "streaming: scanning the retained buffer"),
            "the chunked arm reaches the streaming commit"
        );
        for (name, calls, spent) in &phases {
            println!("{name:>52} {:>9.3} ms over {calls} calls", spent.as_secs_f64() * 1e3);
        }
        // Every counter the arm records, against the input's own size, for the
        // reason the phase dump gives: a tally fetched by name answers the
        // question it was named for and hides the one beside it. A tally that is
        // a multiple of the input is bytes read more than once.
        assert!(
            counts.iter().any(|(had, _)| *had == "streaming: bytes the scans cover"),
            "the chunked arm scans its buffer"
        );
        for (name, tally) in &counts {
            println!("{name:>52} {tally} bytes, {:.2}x the input", *tally as f64 / input.len() as f64);
        }
        println!();
    }

    // What a chunk size costs. It is the one thing a caller of a stream picks,
    // and it sets how much a push lexes at once - which is what decides whether
    // that lex is worth dispatching, since the pool's own threshold was
    // calibrated on a whole input lexed once rather than on a buffer lexed per
    // push with serial work between. The control's pattern throughout, so every
    // row scans rather than refusing.
    println!("\n{:>52}", "the streaming cost, by chunk size");
    println!(
        "{:>12} {:>8} {:>13} {:>7} {:>12}",
        "chunk", "pushes", "fed in chunks", "spread", "a push"
    );
    let p = trex::parse("\"let\" \\W").expect("pattern parses");
    for shift in [14u32, 16, 18, 20, 22] {
        let size = 1usize << shift;
        let parts: Vec<&[u8]> = input.chunks(size).collect();
        let pushes = parts.len();
        let fed = median_ms(3, &mut || trex::scan_chunked(&p, parts.iter().copied()).len());
        // Feeds after the timing rather than during it, so the phases do not
        // charge the row beside it, and as many of them as the row beside it
        // takes. One traced feed is one sample: read that way, the lex column
        // reported 25.9 ms at 16 kB in one run and 14.0 in the next, against a
        // `fed` column that agreed to within a few percent across the same two.
        let mut runs: Vec<Vec<(String, f64)>> = Vec::with_capacity(3);
        let mut tallies: Vec<(String, u64)> = Vec::new();
        for _ in 0..3 {
            trex::trace::record();
            drop(trex::trace::take_phases());
            drop(trex::trace::take_counts());
            drop(trex::scan_chunked(&p, parts.iter().copied()));
            let phases = trex::trace::take_phases();
            let counts = trex::trace::take_counts();
            trex::trace::stop();
            assert!(
                counts.iter().any(|(had, _)| *had == "streaming: bytes the drain moves"),
                "a push drops the bytes it has settled"
            );
            // A counter is a sum over the same input every feed, so the last
            // feed's is every feed's; only the times need a median.
            tallies = counts.iter().map(|(name, v)| (name.to_string(), *v)).collect();
            runs.push(
                phases
                    .iter()
                    .map(|(name, _, spent)| (name.to_string(), spent.as_secs_f64() * 1e3))
                    .collect(),
            );
        }
        println!(
            "{:>9} kB {pushes:>8} {:>10.3} ms {:>6.2}x {:>9.3} ms a push",
            size >> 10,
            fed.median,
            fed.spread(),
            fed.median / pushes as f64
        );
        // Every counter, against the pushes that raised it: what one push
        // walked, which is the quantity a chunk size actually sets.
        for (name, tally) in &tallies {
            println!("{name:>56} {:>12.0} bytes a push", *tally as f64 / pushes as f64);
        }
        // Every phase this size records, medianed over the same three feeds, for
        // the reason the arms above print every phase rather than a chosen one:
        // the question here is which act of a push a chunk size moves, and a
        // column picked by name cannot answer it.
        for (name, _) in &runs[0] {
            let mut times: Vec<f64> = runs
                .iter()
                .filter_map(|r| r.iter().find(|(had, _)| had == name).map(|(_, ms)| *ms))
                .collect();
            times.sort_by(f64::total_cmp);
            println!(
                "{name:>56} {:>9.3} ms {:>6.2}x",
                times[times.len() / 2],
                times[times.len() - 1] / times[0]
            );
        }
    }

    // The whole-input arm above reads far slower than this corpus takes
    // anywhere else, so the question is whether it grows with the input or
    // with the square of it. Doubling the statements doubles both the bytes
    // and the matches, so a linear scan doubles and a quadratic one quadruples.
    println!("{:>34}", "the whole-input scan, by size");
    let mut last: Option<(usize, f64)> = None;
    for statements in [25_000usize, 50_000, 100_000, 200_000] {
        let text = corpus(statements).into_bytes();
        let p = trex::parse("\"let\" \\W").expect("pattern parses");

        // Which mechanism answers it, and what the mechanism spends itself on,
        // read from one call before any is timed: a size that changes the rung
        // is a different answer to a different question, and the growth below
        // would be reporting the change of mechanism as a cost of the input.
        trex::trace::record();
        drop(trex::trace::take_recorded());
        drop(trex::trace::take_phases());
        drop(trex::scan(&p, &text));
        let rungs = trex::trace::take_recorded();
        let phases = trex::trace::take_phases();
        trex::trace::stop();
        let rung = rungs
            .iter()
            .find(|r| r.ladder == "scan")
            .map_or_else(|| String::from("no rung"), |r| r.rung.clone());
        println!("{statements:>10} statements  by {rung}");
        for (name, _, spent) in &phases {
            println!("{name:>36} {:>10.3} ms", spent.as_secs_f64() * 1e3);
        }

        let arm = median_ms(3, &mut || trex::scan(&p, &text).len());
        let grew = last.map_or(String::from("-"), |(_, prev)| format!("{:.2}x", arm.median / prev));
        println!(
            "{statements:>10} statements {:>10} bytes {:>10.3} ms {:>5.2}x {:>8} matches   {grew:>7}",
            text.len(),
            arm.median,
            arm.spread(),
            arm.found
        );
        last = Some((text.len(), arm.median));
    }

    // The same bytes every time, with only the literal's occurrences varied:
    // every occurrence past the first of each group is blunted in place, so the
    // length, the token count and every other byte are identical and the only
    // thing moving is how many windows the route opens. A cost that tracks the
    // occurrences is a per-window cost; a flat cost is not.
    println!("\n{:>34}", "one corpus, the literal thinned");
    let full = corpus(200_000).into_bytes();
    let p = trex::parse("\"let\" \\W").expect("pattern parses");
    for keep in [1usize, 10, 100, 1000] {
        let mut text = full.clone();
        let mut seen = 0usize;
        let mut i = 0;
        while i + 3 <= text.len() {
            if &text[i..i + 3] == b"let" {
                seen += 1;
                if !seen.is_multiple_of(keep) {
                    text[i + 1] = b'x';
                }
                i += 3;
            } else {
                i += 1;
            }
        }
        let arm = median_ms(3, &mut || trex::scan(&p, &text).len());
        let each =
            if arm.found > 0 { arm.median * 1e3 / arm.found as f64 } else { 0.0 };
        println!(
            "{:>10} bytes {:>8} occurrences {:>11.3} ms {:>5.2}x {each:>9.2} us each",
            text.len(),
            arm.found,
            arm.median,
            arm.spread()
        );
    }

    // A set streamed, against the same set over the whole input.
    //
    // One pattern's push lexes the retained buffer once and hands those tokens
    // to the scan. A set is handed nothing: its members can need shapes that
    // differ from one another, so the stream lexes under no shapes at all and
    // the set's own scan lexes the buffer a second time. These four members
    // draw no library kind, so that second lex is the parallel lex of bytes the
    // push has already lexed serially, and the counter of bytes the scans cover
    // says how many times a push reads what it holds.
    //
    // The rows are the arms of the same question the single pattern answers
    // above, so the two are read the same way: the ratio is the load-normalized
    // figure and the milliseconds are comparable only inside one run.
    println!("\n{:>34}", "a set of four members");
    let sources = ["\"let\" \\W", "\\W \"=\"", "\\W{2,4}", "(\\N | \\W) \"=\""];
    let members: Vec<trex::ast::Pattern> =
        sources.iter().map(|s| trex::parse(s).expect("a member parses")).collect();
    // A stream takes ownership of its set and a set does not clone, so each
    // call builds one. Both arms build it the same way, so the construction is
    // inside both measurements rather than only one, and the ratio the row
    // prints is of two figures that each carry it.
    let (whole, fed) = rotated_ms(
        5,
        &mut || trex::PatternSet::new(members.clone()).scan(&input).len(),
        &mut || fed_to_a_set(&members, &chunks),
    );
    assert_eq!(whole.found, fed.found, "the set's two arms answer the same");
    println!("{} members  -  {} matches", sources.len(), whole.found);
    println!(
        "{:>34} {:>9.3} ms   spread {:>5.2}x",
        "over the whole input",
        whole.median,
        whole.spread()
    );
    println!("{:>34} {:>9.3} ms   spread {:>5.2}x", "fed in chunks", fed.median, fed.spread());
    println!("{:>34} {:>9.2}x\n", "the streaming costs", fed.median / whole.median);

    // Both feeds traced, the whole-input one first, so the lex the set's own
    // scan takes can be told from the lex a push takes: the whole-input arm
    // records only the scan's, and the streamed arm records the push's beside
    // it. Every phase and every counter, for the reason the arms above print
    // all of them.
    for (label, run) in [
        (
            "over the whole input",
            &mut (|| trex::PatternSet::new(members.clone()).scan(&input).len())
                as &mut dyn FnMut() -> usize,
        ),
        (
            "fed in chunks",
            &mut (|| fed_to_a_set(&members, &chunks)) as &mut dyn FnMut() -> usize,
        ),
    ] {
        trex::trace::record();
        let _cleared_phases = trex::trace::take_phases();
        let _cleared_counts = trex::trace::take_counts();
        let found = run();
        let phases = trex::trace::take_phases();
        let counts = trex::trace::take_counts();
        trex::trace::stop();
        assert_eq!(found, whole.found, "{label}: the traced feed answers what the timed one did");
        println!("{label}");
        for (name, calls, spent) in &phases {
            println!("{name:>52} {:>9.3} ms over {calls} calls", spent.as_secs_f64() * 1e3);
        }
        for (name, tally) in &counts {
            println!(
                "{name:>52} {tally} bytes, {:.2}x the input",
                *tally as f64 / input.len() as f64
            );
        }
        println!();
    }
}
