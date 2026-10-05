//! The patterns that still reach the per-anchor engine, and what each operation
//! costs over them.
//!
//! `vs_regex_full` does not touch this path at all. The rung trace reads the
//! scan of every pattern it pairs leaving by a route: eleven by a byte route,
//! four with the bindings off, three by the literal windows, and none by the
//! engine. A mechanism sized against that comparison is sized against a path it
//! does not exercise.
//!
//! What still reaches the engine is the surface a finite automaton cannot
//! express, plus the shapes no route is written for: a balanced group, a
//! back-reference, a field anchor, a star, an unequal bounded repeat, an
//! alternation of kinds. Those have no regex counterpart, so this bench has no
//! second column; it is a profile, read against its own control arm and its own
//! earlier runs.
//!
//! Every pattern is checked to reach the engine before it is timed, through
//! `routed_spans_public`, which exists for exactly that. A pattern a route
//! answers is reported and skipped rather than timed, because a route's reading
//! recorded here would be attributed to the engine, and it is engine work this
//! bench is meant to size.
//!
//! Three mechanisms answer a scan, and which one a shape reaches decides what
//! can move it. The single-pass engine has two: `scan_stream` dispatches an
//! attempt per anchor across the cores for a pattern with a bounded longest
//! match, and takes one serial sweep otherwise. The surface a single-pass
//! engine cannot express at all - an atomic group, a first-match alternation,
//! the Perl empty-loop reading - goes to the set engine, which explores
//! positions as a set on one core.
//!
//! Only the single-pass engine names its mechanism on the rung ladder, so the
//! set engine is read as the absence of both names. That reading is sound only
//! because a route-answered pattern is reported and skipped before it is timed,
//! which leaves the absence meaning the set engine and nothing else.
//!
//! A balanced group can match to any depth and has no single kind to measure a
//! run over, so `\B`, `\W \B` and `\B(\W)` never reach the dispatch, and the
//! star and the alternation are the only shapes here that do.
//!
//! `every match, one core` is the same matches through the serial walk, so
//! `every match` divided by it is what the dispatch is worth. It is printed
//! only on a dispatched row: a sweep shape is already on one core under `every
//! match`, and the serial walk declines a balanced group outright, so an arm
//! there would time an empty call and read as an unbounded speed-up.
//!
//! Read that ratio only under a pinned Flynnel profile. trex dispatches through
//! Flynnel, which on a shared host reads a stored calibration that may have
//! been drawn under contention, and the collapse threshold in it decides
//! whether a leaf runs on a worker or inline on the calling thread. Point
//! `FLYNNEL_CALIBRATION_DIR` at a directory of the run's own, and say which
//! profile a published figure was taken under.
//!
//! That makes the two balanced-group rows the control for any change to the
//! dispatch: the change cannot reach them, so whatever moves them is the box.
//! A run where they move as far as the star and the alternation is a run that
//! says nothing about either. One reading had them at -2.8% and +19% while the
//! dispatched shapes read -8.9% and +10.4%, which is how it was caught. Read the
//! control rows first, and only then the shapes under test.
//!
//! Run: `cargo run --release --bench engine_surface`.

use std::hint::black_box;
use std::time::Instant;

/// Statements of four shapes, the corpus `benches/vs_regex_full.rs` builds.
///
/// The formula is that file's exactly, so a reading here and a reading there
/// are over the same bytes.
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

const ROUNDS: usize = 5;
const BUDGET_MS: f64 = 25.0;

/// Time `f` until `BUDGET_MS` has passed and at least five calls have run.
///
/// The comparison's own harness shape, so a millisecond here reads against a
/// millisecond there.
fn time_budget(f: &mut dyn FnMut() -> usize) -> (f64, usize) {
    let n = f();
    let t0 = Instant::now();
    let mut iters = 0u32;
    loop {
        black_box(f());
        iters += 1;
        if iters >= 5 && t0.elapsed().as_secs_f64() * 1e3 >= BUDGET_MS {
            break;
        }
    }
    (t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters), n)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Every arm timed once a round in an order that rotates, then the first arm
/// over again as a control: position inside a run is worth a few percent, and
/// an arm always timed first would read that as its own cost. The control is
/// the same work as the row it is printed under, so what separates the two is the
/// box and nothing else, and that distance is the smallest effect a row here
/// can carry.
///
/// Answers the first arm's median, which is always `every match`, so a caller
/// can total the surface by mechanism without timing it a second time.
fn sweep(label: &str, arms: &mut [(&str, &mut dyn FnMut() -> usize)]) -> f64 {
    let n = arms.len();
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); n];
    let mut answers: Vec<usize> = vec![0; n];
    for round in 0..ROUNDS {
        for step in 0..n {
            let i = (round + step) % n;
            let (t, got) = time_budget(&mut *arms[i].1);
            times[i].push(t);
            answers[i] = got;
        }
    }
    // The first arm timed over again, as many times as it was timed above and
    // reduced the same way, so the figure beside it holds a median against a
    // median. One sample against a median of five is the noisier of the two
    // compared to the quieter, and reads as a spread the reported rows do not
    // have.
    let control = median((0..ROUNDS).map(|_| time_budget(&mut *arms[0].1).0).collect());
    let first = median(times[0].clone());
    for (i, (name, _)) in arms.iter().enumerate() {
        println!(
            "{label:>28}  {name:>22} {:>10.4} ms {:>10}",
            median(times[i].clone()),
            answers[i]
        );
    }
    println!("{label:>28}  {:>22} {control:>10.4} ms  drift {:.3}x", "CONTROL", control / first);
    first
}

/// Which of the engine's two mechanisms a shape reaches, which is what decides
/// whether a change to one of them could have moved its row.
///
/// `scan_stream` holds the rule: the dispatch takes a pattern whose longest
/// match is bounded, by its own shape or by the longest run in the stream of the
/// one kind an open repeat consumes, and every other pattern takes the sweep.
///
/// This is read from the trace rather than declared beside the pattern, so a
/// row cannot be mislabelled - which the shapes already proved was a live
/// risk: two of the six this bench once carried are answered by a route today
/// and were still written down as reaching the engine.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reaches {
    /// An attempt an anchor, across the cores.
    Dispatch,
    /// One serial sweep of the stream, on one core.
    Sweep,
    /// The set engine, which explores positions as a set instead of trying
    /// paths one at a time.
    ///
    /// It is read as the absence of the other two rather than by a rung of its
    /// own, because both mechanism rungs are named in the single-pass engine
    /// and the set engine names neither. That holds only for a pattern already
    /// checked to reach an engine at all, which is what `routed_spans_public`
    /// settles before a row is timed; a route-answered pattern is reported and
    /// skipped, so it never arrives here.
    Set,
}

impl Reaches {
    /// The mechanism `scan` takes for this pattern and how many matches it
    /// found, from the rung the scan names.
    ///
    /// The count comes back so the caller can hold the timed arms to the same
    /// answer this saw. The trace is turned off again before anything is
    /// timed: keeping a rung takes a lock per call, and a measurement taken
    /// with it on prices the trace as well as the work.
    fn of(p: &trex::ast::Pattern, input: &[u8]) -> (Reaches, usize) {
        trex::trace::record();
        // Whatever an earlier pattern left, so the rung below is this one's.
        drop(trex::trace::take_recorded());
        let found = trex::scan(p, input).len();
        let kept = trex::trace::take_recorded();
        trex::trace::stop();
        // By the rung's own name rather than by the last one on the `scan`
        // ladder: the engine names a further rung after the mechanism does -
        // "the single-pass engine over a whole lex" - so the last one is
        // always the engine's and never says which mechanism carried it.
        let named = |want: &str| kept.iter().any(|r| r.ladder == "scan" && r.rung == want);
        let reaches = if named("an attempt an anchor, across the cores") {
            Reaches::Dispatch
        } else if named("one serial sweep of the stream") {
            Reaches::Sweep
        } else {
            Reaches::Set
        };
        (reaches, found)
    }

    fn label(self) -> &'static str {
        match self {
            Reaches::Dispatch => "dispatch",
            Reaches::Sweep => "sweep - a control for any dispatch change",
            Reaches::Set => "the set engine, on one core",
        }
    }
}

/// The shapes no route answers, each named by what makes it reach the engine
/// and by which mechanism it reaches.
const ENGINE_PATTERNS: [(&str, &str); 26] = [
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
    ("a spectral atom", "\\F{entropy>0.7}"),
    ("a seam anchor", "@seam \\W"),
    ("an echo anchor", "@echoed \\W"),
    ("a nesting anchor", "@nested>0 \\W"),
    ("a construct anchor", "@super:assign \\W"),
    // The token-grain edit distance, which offers a run of several lengths.
    ("an edit-distance group", "(\"let\" \\W \"=\")~1"),
    // Two more fields a scan builds. Neither matches over this corpus, which
    // is why neither was here: a field is built because the pattern names it,
    // and a shape is timed for what the scan does rather than for what it
    // answers.
    ("a rarity anchor", "@shape:rare \\W"),
    ("a timestamp order anchor", "@order:desc \\N"),
];

fn main() {
    let text = corpus(200_000);
    let input = text.as_bytes();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    println!("trex {}", trex::version());
    println!("corpus {} bytes, {cores} cores\n", input.len());
    println!("{:>28}  {:>22} {:>13} {:>10}", "pattern", "operation", "ms", "answer");

    // Summed per mechanism, because the proportion is what says where work is
    // worth aiming: a mechanism's share of the surface bounds what any change
    // to it can return, however good the change is.
    let mut by_mechanism: [(usize, f64); 2] = [(0, 0.0), (0, 0.0)];

    for (label, src) in ENGINE_PATTERNS {
        let p = match trex::parse(src) {
            Ok(p) => p,
            Err(e) => {
                println!("{label:>28}  {src} does not parse: {e:?}");
                continue;
            }
        };
        // The guard this bench exists to keep: a pattern a route answers is not
        // an engine reading, and timing it here would attribute a route's cost
        // to the engine.
        if trex::engine::routed_spans_public(&p, input).is_some() {
            println!("{label:>28}  a route answers {src}, so it is not timed here");
            continue;
        }
        let (reaches, traced) = Reaches::of(&p, input);
        println!("{label:>28}  reaches the {}", reaches.label());
        let spans = trex::scan(&p, input);
        if spans.len() != traced {
            println!("{label:>28}  two scans of {src} answered {traced} and {}", spans.len());
            continue;
        }
        // A shape that matches nothing is timed like any other. Finding no
        // match is not doing no work: the scan still lexes, still builds every
        // axis field the pattern names, and still sweeps every anchor, and the
        // dearest shape on this surface is one of them - `@seam \W` answers
        // nothing over the corpus and spends 1090.222 ms doing it, of which
        // 889.826 is counting byte contexts. Its capture arm reads near zero
        // because there is nothing to capture, and that is the arm being
        // truthful rather than the shape being free.
        if spans.is_empty() {
            println!("{label:>28}  {src} matches nothing, so only its capture arm is idle");
        }
        let mut scan = || trex::scan(&p, input).len();
        let mut matches = || usize::from(trex::is_match(&p, input));
        let mut first = || trex::find(&p, input).map_or(0, |s| s.start() + 1);
        let mut caps = || trex::captures(&p, input, &spans).len();
        // The same matches on one core, which is what says what the per-anchor
        // dispatch is worth: `every match` divided by this is the speed-up.
        //
        // Only for a shape that reaches the dispatch. A sweep shape already
        // runs on one core under `every match`, and the serial walk declines
        // it outright - a balanced group routes to the set engine, so this
        // returns None and would time an empty call, which reads as an
        // unbounded speed-up rather than as the arm not applying.
        //
        // The tokens are lexed once outside the arm, because the dispatched
        // arm lexes across the cores and a lex inside here would price that
        // difference as the dispatch's.
        let toks = trex::lexer::lex(input);
        let mut one_core =
            || trex::nfa::scan_nfa_over_serial(&p, input, &toks).map_or(usize::MAX, |v| v.len());
        if reaches == Reaches::Dispatch && one_core() != spans.len() {
            println!(
                "{label:>28}  the serial walk answers {} where the scan answers {}, so the two arms are not one question",
                one_core(),
                spans.len()
            );
            continue;
        }
        // The window route with its gate off. A gate refused this pattern's
        // windows on coverage, which is a judgment about whether they are
        // worth taking - and the ungated form exists to price that judgment
        // rather than argue with it. An arm here faster than `every match` is
        // a gate leaving time on the table for this shape; one slower is the
        // gate right.
        let mut ungated =
            || trex::prefilter::scan_required_windows_ungated(&p, input).map_or(0, |v| v.len());
        let mut arms: Vec<(&str, &mut dyn FnMut() -> usize)> = vec![("every match", &mut scan)];
        if reaches == Reaches::Dispatch {
            arms.push(("every match, one core", &mut one_core));
        }
        arms.push(("does it match at all", &mut matches));
        arms.push(("where it first matches", &mut first));
        arms.push(("captures at every match", &mut caps));
        arms.push(("windows, gate off", &mut ungated));
        let every = sweep(label, &mut arms);
        if reaches == Reaches::Dispatch {
            by_mechanism[0].0 += 1;
            by_mechanism[0].1 += every;
        } else {
            by_mechanism[1].0 += 1;
            by_mechanism[1].1 += every;
        }
        println!();
    }

    let (dispatched, swept) = (by_mechanism[0], by_mechanism[1]);
    let total = dispatched.1 + swept.1;
    println!("{:>28}  every match, summed", "the engine surface");
    for (name, (count, ms)) in [("dispatched", dispatched), ("the set engine, one core", swept)] {
        let share = if total > 0.0 { 100.0 * ms / total } else { 0.0 };
        println!("{name:>28}  {count:>2} shapes {ms:>10.4} ms {share:>7.1}%");
    }
    println!("{:>28}  {:>2} shapes {total:>10.4} ms", "all of it", dispatched.0 + swept.0);
}
