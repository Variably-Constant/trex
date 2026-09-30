//! Where a set-engine scan spends its time, by phase, over the corpus
//! `benches/engine_surface` reads.
//!
//! That bench times a whole row and names the mechanism that answered it. It
//! cannot say which part of the row a change reached, so a share of a row read
//! off it is an argument and not a measurement - and two such arguments were
//! wrong before this existed: an allocation predicted at two to three times its
//! row turned out to be a tenth of it, and a regression was attributed to a
//! change that proved not to have caused it.
//!
//! The phases here are the scan's own, named at the points that divide it: the
//! lex with its stitch, the axis fields built for the pattern, and the sweep
//! over the anchors. A phase inside another counts in both, so the readings are
//! shares of a tree and their sum can exceed the whole.
//!
//! One call a shape. A phase is recorded per call, so a benchmark's thousands
//! would report the run rather than the call, and the clock a phase starts is
//! itself the cost a benchmark is trying to avoid.
//!
//! Each shape is scanned twice over: once unwatched, which is the figure a
//! caller has and a change to the engine moves, and once watched, which is
//! what the shares are shares of. A phase starts a clock and takes a lock on
//! the way out, so the watched scan is the longer of the two, and by how much
//! is printed beside it - a shape whose phases are entered thousands of times
//! a scan is priced differently by the watch than one entered once, and a
//! share read without knowing which is an argument again.
//!
//! Every shape is read once a round over several rounds, in an order that
//! rotates, and each reading - the scan and every phase inside it - is reduced
//! by its median rather than its mean. Then the first shape is read once more
//! as a control. That last figure is the same work as the row it repeats, so
//! the distance between them is the box moving under the run and not the
//! shape, and it bounds the smallest effect any row here can carry. This is
//! what lets a reading be taken on a machine that has other work on it: the
//! rotation spreads another process's noise across the shapes instead of
//! landing it all on whichever ones a fixed order was passing, the median
//! drops it, and the control says whether that worked.
//!
//! Run: `cargo run --release --example where_the_time_goes`.

/// Statements of four shapes, `benches/engine_surface`'s corpus exactly, so a
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

/// The whole a share is read against.
const WHOLE: &str = "the set engine";

/// Every shape `benches/engine_surface` times, by the name and the source it
/// reports them under, so a row there and a row here name the same work.
///
/// The engine shapes are that file's `ENGINE_PATTERNS` and the route shapes
/// precede them. A shape held here and not there would be measured against
/// nothing; one held there and not here would have a row and no phases, which
/// is the state that let a route read three orders of magnitude over its
/// neighbours without anything saying so.
const SHAPES: &[(&str, &str)] = &[
    // The shapes a route answers. They name no engine phase and are here for
    // the whole-scan figure, because the engine-surface bench reports a
    // route-answered shape and skips timing it - it exists to size the engine.
    // That left the routes unmeasured, and a literal then a kind was reading
    // 10134.600 ms over 7.34 MB when it was finally timed: the confirmation of
    // a matched window doubled its lex out to the input's end.
    ("a literal alone", "\"let\""),
    ("a literal then a kind", "\"let\" \\W"),
    ("a kind then a literal", "\\W \"=\""),
    ("a kind sequence", "\\W \\N"),
    // Structure no finite automaton expresses.
    ("a balanced group", "\\B"),
    ("a word then a balanced group", "\\W \\B"),
    ("a balanced group with an interior", "\\B(\\W)"),
    ("a balanced group of one bracket kind", "\\B(\\W \"=\")"),
    // Repetition and choice, the shapes the dispatch was written for.
    ("a star", "\"let\" \\W* \"=\""),
    ("a plus", "\"let\" \\W+ \"=\""),
    ("an unequal bounded repeat", "\\W{2,4}"),
    ("an open repeat", "\\W{2,}"),
    ("an alternation of kinds", "(\\N | \\W) \"=\""),
    ("an alternation, first-match", "(\\N |> \\W) \"=\""),
    ("an atomic group", "(?>\\W*) \"=\""),
    ("a lazy star", "\"let\" \\W*? \"=\""),
    // A register read back, which no automaton carries.
    ("a back-reference", "\\W:a \"=\" =a"),
    ("a back-reference up to case", "\\W:a \"=\" =case a"),
    ("a back-reference within one edit", "\\W:a \"=\" =edit1 a"),
    // An assertion, which runs a sub-pattern as a filter.
    ("a sub-pattern assertion", "\\W ~(\"=\" \\N)"),
    ("a negative assertion", "\\W !~(\"=\" \\Q)"),
    // The axes, each a field built once per scan and read per token.
    ("a magnitude atom", "\\M{>3} \"=\""),
    // One atom an axis, because the spectral field is a single pass a byte
    // with a gate an axis inside it and so divides into no phases of its own.
    // Which axes a pattern names is what its `Needs` carries, so the field's
    // one phase read under each atom in turn is the same division read from
    // outside.
    ("a spectral atom", "\\F{entropy>0.7}"),
    ("a spectral atom on period", "\\F{period}"),
    ("a spectral atom on texture", "\\F{texture:code}"),
    ("a spectral atom on onset", "\\F{onset}"),
    // One anchor a grain. The segmentation counts a context per symbol at
    // every order in both directions, so what it costs follows the length of
    // the sequence it reads, and the three sequences differ by orders: the
    // bytes, the significant tokens cut from them, and the supertokens over
    // those. A default picked from the shape of that sentence rather than from
    // three readings is a guess.
    ("a seam anchor", "@seam \\W"),
    ("a seam anchor on bytes", "@seam:byte \\W"),
    ("a seam anchor on tokens", "@seam:token \\W"),
    ("a seam anchor on supertokens", "@seam:super \\W"),
    ("an echo anchor", "@echoed \\W"),
    ("a nesting anchor", "@nested>0 \\W"),
    ("a construct anchor", "@super:assign \\W"),
    // The token-grain edit distance, which offers a run of several lengths.
    ("an edit-distance group", "(\"let\" \\W \"=\")~1"),
    // Two fields that are built by a scan and were reached by no shape the
    // surface times, so what they cost was never a figure at all. Neither
    // answers a match over this corpus, which is the point: a field is built
    // because the pattern names it and not because it matches.
    ("a rarity anchor", "@shape:rare \\W"),
    ("a timestamp order anchor", "@order:desc \\N"),
    // A literal leading a pattern the single-pass engine declines, so the set
    // engine's sweep answers it and reads an opening-kind route.
    //
    // Every other literal-led shape above - the star, the plus, the lazy star -
    // is taken by the single-pass engine, which reads no route, so none of them
    // can show what a route for a literal is worth. Measured over the crate's
    // own source these three refuse 474,690 anchors of 875,524 where the rest
    // refuse none, and a caller writing `"ERROR"` in front of an assertion has
    // exactly this shape.
    ("a literal then an atomic group", "\"let\" (?>\\W*) \"=\""),
    ("a literal then a negative assertion", "\"let\" \\W !~(\"=\" \\Q)"),
    ("a literal then a spectral atom", "\"let\" \\F{entropy>0.7}"),
];

/// How many times each shape is timed, each in a different position.
const ROUNDS: usize = 5;

/// The middle reading of `v`, which is what a round's worth of samples is
/// reduced to. A mean carries an outlier into the answer whole; a median
/// discards it, and something else waking on the box is an outlier rather
/// than a cost of the shape.
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// What one timing of one shape answered: the scan unwatched, the same scan
/// watched, and what each phase took inside the watched one.
struct Reading {
    found: usize,
    unwatched: f64,
    watched: f64,
    phases: Vec<(&'static str, u64, f64)>,
    /// What a named count reached, for a quantity inside a loop that a phase
    /// cannot report without costing more than the loop.
    counts: Vec<(&'static str, u64)>,
}

/// Time `p` once each way, in milliseconds a scan.
fn read(p: &trex::ast::Pattern, input: &[u8], repeats: u32) -> Reading {
    let per_scan = f64::from(repeats);
    // The scan as a caller has it, with no clock started inside it and no lock
    // taken on the way out of one.
    let from = std::time::Instant::now();
    let mut found = 0;
    for _ in 0..repeats {
        found = trex::scan(p, input).len();
    }
    let unwatched = from.elapsed().as_secs_f64() * 1e3 / per_scan;

    // The same scan, watched. Whatever an earlier shape left is dropped first,
    // so the phases are this one's.
    trex::trace::record();
    drop(trex::trace::take_phases());
    let from = std::time::Instant::now();
    for _ in 0..repeats {
        drop(trex::scan(p, input));
    }
    let watched = from.elapsed().as_secs_f64() * 1e3 / per_scan;
    let taken = trex::trace::take_phases();
    let counts = trex::trace::take_counts();
    trex::trace::stop();
    let phases = taken
        .into_iter()
        .map(|(name, calls, spent)| (name, calls, spent.as_secs_f64() * 1e3 / per_scan))
        .collect();
    Reading { found, unwatched, watched, phases, counts }
}

fn main() {
    // Scans a shape, past one that is not counted. The uncounted scan maps the
    // pages this thread's held lex buffers live in and fills them; a share read
    // off that call is a share of mapping them, which is paid once rather than
    // per scan. More than one counted scan also gives a sampling profiler
    // enough of the run to attribute inside a phase, where a clock per anchor
    // would cost more than the anchor does.
    let repeats: u32 = match std::env::args().nth(1) {
        None => 4,
        Some(given) => match given.parse() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("the first argument is how many scans a shape: {given:?} is not one ({e})");
                std::process::exit(2);
            }
        },
    };
    // A real file where one is named, and the generated corpus otherwise. The
    // generated one is what every figure recorded from this harness was read
    // over, so it stays the default and the header says which was used - a
    // table that does not name its corpus is a table that gets compared with
    // one read over different bytes. Generated statements of four shapes are an
    // easy input: their tokens are the ones the generator chose, in the mix it
    // chose, so a filter's refusal count over them is a fact about the
    // generator as much as about the engine. Name a file to find out what a
    // reading is worth on bytes nobody wrote for it.
    let (input, whose) = match std::env::args().nth(2) {
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
    println!(
        "corpus {whose}, {} bytes, {repeats} scans a reading, {ROUNDS} rounds a shape past one warm\n",
        input.len()
    );

    // Parsed once, so the rounds below time scanning and not parsing.
    let mut shapes: Vec<(&str, trex::ast::Pattern)> = Vec::new();
    for (label, src) in SHAPES {
        match trex::parse(src) {
            Ok(p) => shapes.push((label, p)),
            Err(e) => println!("{label:>34}  does not parse: {e:?}"),
        }
    }
    // The buffers the counted rounds read, mapped and filled before any of them
    // is timed.
    for (_, p) in &shapes {
        drop(trex::scan(p, &input));
    }

    // Every shape read once a round, in an order that rotates. A shape always
    // read first would take whatever the run's first position is worth as its
    // own cost, and a box that wakes up partway through a fixed order lands all
    // of its noise on whichever shapes were passing at the time. Rotating
    // spreads that over the shapes and the median then drops it, so a reading
    // here does not need a quiet box - it needs the control below to say the
    // box held still, which is a thing it can check rather than assume.
    let n = shapes.len();
    let mut rounds: Vec<Vec<Reading>> = (0..n).map(|_| Vec::new()).collect();
    for round in 0..ROUNDS {
        for step in 0..n {
            let i = (round + step) % n;
            let reading = read(&shapes[i].1, &input, repeats);
            rounds[i].push(reading);
        }
    }
    // One shape read again, after every other has been. Its distance from that
    // shape's median is the box moving under the run, and it is the same work
    // either way, so anything the two do not share is not the shape.
    //
    // The shape is the one of middling cost rather than the first, because the
    // distance is read as a ratio and a ratio over a sub-millisecond row is the
    // clock's own grain rather than the box's drift: the cheapest shape here
    // read 0.516 ms against a median of 0.811 and reported a 36% move that a
    // row costing hundreds of milliseconds did not have.
    let mut ranked: Vec<(usize, f64)> = (0..n)
        .map(|i| (i, median(rounds[i].iter().map(|r| r.unwatched).collect())))
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (middling, middling_median) = ranked[n / 2];
    let control = read(&shapes[middling].1, &input, repeats);

    let mut totals: Vec<(&str, f64)> = Vec::new();
    for (i, (label, _)) in shapes.iter().enumerate() {
        let taken = &rounds[i];
        let found = taken[0].found;
        let unwatched = median(taken.iter().map(|r| r.unwatched).collect());
        let watched = median(taken.iter().map(|r| r.watched).collect());

        // Every shape is timed whole, so a shape a route answers still reports
        // what it costs; only one that reached the set engine divides further.
        println!("{label}  -  {found} matches, {unwatched:.3} ms a scan");
        if taken[0].phases.is_empty() {
            println!("{:>36}  names no phase\n", "");
            continue;
        }
        // What the watching costs this shape, which is what says how far a
        // share below can be carried back to the scan above it.
        println!("{:>36} {watched:>9.3} ms {:>6.2}x   watched", "", watched / unwatched);
        // A share is read against the watched scan, the one the phases were
        // taken from, so a shape a route answers divides the same way one the
        // set engine answers does. Each phase is reduced over the rounds the
        // same way the scan around it is.
        let mut named: Vec<(&'static str, u64, f64)> = Vec::new();
        for (name, calls, _) in &taken[0].phases {
            if *name == WHOLE {
                continue;
            }
            let each: Vec<f64> = taken
                .iter()
                .filter_map(|r| r.phases.iter().find(|(held, _, _)| held == name))
                .map(|(_, _, ms)| *ms)
                .collect();
            if each.is_empty() {
                continue;
            }
            named.push((name, *calls, median(each)));
        }
        for (name, calls, ms) in &named {
            let share = 100.0 * ms / watched;
            #[allow(clippy::cast_precision_loss)]
            let each = *calls as f64 / f64::from(repeats);
            println!("{name:>36} {ms:>9.3} ms {share:>7.1}%   {each:>5.1} call(s) a scan");
        }
        for (name, _, ms) in &named {
            match totals.iter_mut().find(|(held, _)| held == name) {
                Some((_, held)) => *held += ms,
                None => totals.push((name, *ms)),
            }
        }
        // A count is a quantity rather than a duration, so it is reported per
        // scan and carries no share.
        for (name, seen) in &taken[0].counts {
            #[allow(clippy::cast_precision_loss)]
            let each = *seen as f64 / f64::from(repeats);
            println!("{name:>36} {each:>9.0}    a scan");
        }
        println!();
    }

    // The control against the shape it repeats. The two are the same work, so
    // the distance between them is the smallest effect any row above can carry.
    println!(
        "{:>36} {:>9.3} ms {:>6.3}x   the control, against {} at {:.3} ms\n",
        "",
        control.unwatched,
        control.unwatched / middling_median,
        shapes[middling].0,
        middling_median
    );

    println!("{:>36}", "summed over every shape above, watched");
    let sum: f64 = totals.iter().map(|(_, ms)| ms).sum();
    for (name, ms) in &totals {
        let share = if sum > 0.0 { 100.0 * ms / sum } else { 0.0 };
        println!("{name:>36} {ms:>9.3} ms {share:>7.1}%");
    }
}
