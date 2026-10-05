//! What lexing only a literal's windows saves a shortest match, against
//! lexing the whole input two ways.
//!
//! Usage: `what_the_window_route_saves <corpus> <pattern>...`
//!
//! `shortest_match` stops at the least end rather than finding every match,
//! and an early-terminating operation has to stop the lexer rather than the
//! walk, because stopping the walk saves only the cheap half. The window
//! route is the form that stops the lexer: it lexes only the regions a
//! required literal can put a match in. Whether that saves time is what this
//! reads.
//!
//! Three arms, because two would not separate the question. The window route
//! lexes its regions serially while the whole-input arm lexes in parallel, so
//! a window win read against the parallel lex alone is a win over windowing
//! and over parallelism at once, with no way to tell which. The serial whole
//! lex is the third arm: windows against it isolates the windowing, and the
//! two whole-lex arms against each other isolate the parallelism.
//!
//! The route reports itself and needs no counter. `shortest_end_in_windows`
//! answers `Ok(Some(end))` where it ran and found an end, `Ok(None)` where it
//! ran and found nothing, and `Err(reason)` where it declined - so a declined
//! route is visible rather than inferred from a flat reading, which is how
//! six sections were once recorded as unmoved by a route they never entered.
//! The reason matters as much as the refusal: a pattern carrying no literal
//! can never take this route, while one whose literal is too dense is refused
//! by this input alone, and only the second is worth asking again elsewhere.
//!
//! A pattern only reaches the route if its literal is rare. The gate is
//! `input.len() / (WINDOW_COST_IN_INPUT_BYTES * WINDOW_MARGIN)`, one merged
//! window per 32 kB, so `"let"` on a quarter of the lines misses by orders of
//! magnitude while a literal occurring tens of times over a few megabytes
//! fits. Patterns are given on the command line for that reason: the right
//! one is a property of the corpus.
//!
//! The three arms must agree on the end. Where they do not the reading is
//! void and says so, because a faster answer to a different question is not a
//! saving.

use std::hint::black_box;
use std::time::Instant;

use trex::ast::Pattern;

/// How long one arm is timed for, in milliseconds.
///
/// A window route over a rare literal can finish in microseconds where the
/// whole lex takes tens of milliseconds, so one clock read over the fast arm
/// is mostly the clock. Both are read from as many passes as this needs.
const INNER_TARGET_MS: f64 = 50.0;

/// The width of the table's columns before the pattern, so a note under a row
/// lines up with the pattern it is about.
const INDENT: &str = "                                                  ";

/// Milliseconds one pass takes, read from as many passes as
/// [`INNER_TARGET_MS`] needs, with the first discarded.
fn timed_ms(run: &mut dyn FnMut() -> usize) -> f64 {
    black_box(run());
    let mut reps: u32 = 0;
    let t0 = Instant::now();
    let ms = loop {
        black_box(run());
        reps += 1;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if ms >= INNER_TARGET_MS {
            break ms;
        }
    };
    ms / f64::from(reps)
}

/// An end as a number the timing closures can return, since a missing end and
/// a found one have to be told apart in the agreement check but not in the
/// clock.
fn code(end: Option<usize>) -> usize {
    end.unwrap_or(usize::MAX)
}

/// The three arms and the three ends, or the reason the route declined.
fn read(
    pattern: &Pattern,
    input: &[u8],
) -> Result<(f64, f64, f64, usize, usize, usize), trex::prefilter::WindowRefusal> {
    // Asked before any clock starts: a declined route has no arm. The reason
    // comes back rather than a bare refusal, because "no literal anchors a
    // window" and "the windows cost more than the whole lex" are different
    // facts about the pattern and only the second is about this input.
    let routed = trex::prefilter::shortest_end_in_windows(pattern, input)?;

    let parallel_end =
        trex::nfa::shortest_end(pattern, input, &trex::parallel_lex::lex_parallel(input), 0);
    let serial_end = trex::nfa::shortest_end(pattern, input, &trex::lexer::lex(input), 0);

    // The route answered above or this function returned, and it is
    // deterministic over one input, so a refusal here is a broken invariant
    // rather than a case to handle quietly.
    let windows = timed_ms(&mut || {
        code(
            trex::prefilter::shortest_end_in_windows(pattern, input)
                .expect("the route answered once already for this pattern and input"),
        )
    });
    let parallel = timed_ms(&mut || {
        let toks = trex::parallel_lex::lex_parallel(input);
        code(trex::nfa::shortest_end(pattern, input, &toks, 0))
    });
    let serial = timed_ms(&mut || {
        let toks = trex::lexer::lex(input);
        code(trex::nfa::shortest_end(pattern, input, &toks, 0))
    });

    Ok((windows, parallel, serial, code(routed), code(parallel_end), code(serial_end)))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: what_the_window_route_saves <corpus> <pattern>...");
        std::process::exit(2);
    };
    let input = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("the corpus at {path} could not be read: {e}");
            std::process::exit(2);
        }
    };
    let sources: Vec<String> = args.collect();
    if sources.is_empty() {
        eprintln!("no pattern given; the right one is a property of the corpus");
        std::process::exit(2);
    }

    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("corpus {path}, {} bytes", input.len());
    println!(
        "one merged window per 32 kB is the gate, so this corpus allows {}\n",
        input.len() / 32_000
    );
    // The end is printed beside the times because it is what the whole-lex
    // arms stop at: `shortest_end` returns the least end, so their walk ends
    // where the first match is and a literal that occurs early is cheap to
    // find by any route. A table of times without it invites reading the
    // occurrence count as the axis when the first match's position is.
    println!(
        "{:>10} {:>10} {:>10} {:>8} {:>9} {:>10}  pattern",
        "windows", "parallel", "serial", "w/serial", "route", "end"
    );

    for src in &sources {
        let pattern = match trex::parse(src) {
            Ok(p) => p,
            Err(e) => {
                // The reason is printed rather than dropped: a pattern the
                // parser refuses and one the route declines read the same in
                // a table that says only which rows are missing.
                println!(
                    "{:>10} {:>10} {:>10} {:>8} {:>9} {:>10}  {src}",
                    "-", "-", "-", "-", "unparsed", ""
                );
                println!("{INDENT}{src}: {e:?}");
                continue;
            }
        };
        match read(&pattern, &input) {
            Err(why) => {
                // The reason is the row, not a footnote: a pattern with no
                // literal and a literal too dense to window both used to
                // print as "declined", and only the second is a fact about
                // this corpus.
                println!(
                    "{:>10} {:>10} {:>10} {:>8} {:>9} {:>10}  {src}",
                    "-", "-", "-", "-", "declined", ""
                );
                println!("{INDENT}{why}");
            }
            Ok((windows, parallel, serial, routed, par_end, ser_end)) => {
                let agree = routed == par_end && par_end == ser_end;
                let ratio = if serial > 0.0 { windows / serial } else { 0.0 };
                let end =
                    if routed == usize::MAX { "none".to_string() } else { routed.to_string() };
                println!(
                    "{windows:>10.3} {parallel:>10.3} {serial:>10.3} {ratio:>7.3}x {:>9} {end:>10}  {src}",
                    if agree { "taken" } else { "disagree" }
                );
                if !agree {
                    println!(
                        "{INDENT}the ends differ: windows {routed}, parallel {par_end}, serial {ser_end} - the reading is void"
                    );
                }
            }
        }
    }

    // What the parallel arm's idle path actually did, read after the run
    // because the answer can change during one.
    //
    // The parallel lex parks its idle workers through Flynnel, whose parker
    // takes an in-core monitor wait where the part supports it and falls back
    // to the kernel park where the wait does not hold. The fallback latches in
    // a process-global and is never cleared, so a run can begin on one idle
    // path and end on the other. The two read alike in the times and differ by
    // several microseconds a park, so the parallel column is only comparable
    // against another run that answered this the same way. The serial and
    // window columns never enter the scheduler and are unaffected.
    println!(
        "\nthe parallel arm's parker: monitor wait {}",
        if flynnel::sched::sleep::monitor_wait_held() {
            "held throughout"
        } else {
            "did not hold, so this run fell back to the kernel park"
        }
    );
}
