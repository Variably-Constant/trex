//! trex against the regex crate across both crates' whole matching surface.
//!
//! `benches/vs_regex` compares one operation, finding every match, over sizes
//! and corpora. This one holds the corpus still and opens the other axis: the
//! operations each crate offers, the pattern classes each can express, and the
//! places where one has a function the other has none for.
//!
//! Three things are reported that a single ratio hides.
//!
//! Operations are not one cost. Asking whether a pattern matches, finding
//! where it first matches, counting every match, resolving captures, and
//! rewriting are five different amounts of work, and the two crates do not
//! divide them the same way: trex lexes once and matches over tokens, so its
//! per-operation costs share a fixed lex that a regular expression never pays
//! and never amortizes.
//!
//! Some rows have no opposite number. trex takes a back-reference, a
//! lookaround assertion and a scan fed in chunks, which the regex crate
//! cannot express or does not offer. Those rows are timed on the side that
//! has them and marked on the side that does not, rather than left out.
//!
//! Every cell is five rounds of rotating arms reporting the middle reading and
//! each arm's own spread, since the host is shared: a spread far from one says
//! the cell moved while it was timed and is not a reading.

use std::hint::black_box;
use std::time::Instant;

use regex::Regex;

/// Statements of four shapes, the corpus `benches/vs_regex` sweeps.
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

/// What one timed iteration should cost, so the harness's own per-iteration
/// cost is under a percent of it.
///
/// The loop reads the clock once an iteration, and that read is divided
/// across the calls the iteration made rather than added to each. Sizing an
/// iteration this way is what keeps the clock out of the reading: an
/// iteration of one call carries a whole clock read, some twenty-five
/// nanoseconds, in every figure it reports.
///
/// At ten microseconds an iteration the floor is 0.6715 ns a call, spread
/// 1.58x, against 0.5765 ns at spread 1.51x for the same loop with a barrier
/// around every call instead. So where the barrier sits does not separate at
/// this scale, and a baseline of a few tens of nanoseconds carries the
/// harness at a few per cent rather than at half. [`timing_floor`] measures
/// it on the host and the build in hand and prints it with the table,
/// because it is a property of both.
const INNER_TARGET_MS: f64 = 0.01;

/// How many times `f` must run inside one timed iteration for that iteration to
/// reach [`INNER_TARGET_MS`].
///
/// One for anything already that slow, which is every row answering in
/// milliseconds, so their readings are what they always were.
fn inner_reps(mut f: impl FnMut() -> usize) -> u32 {
    let t0 = Instant::now();
    let mut probe = 0u32;
    loop {
        black_box(f());
        probe += 1;
        if probe >= 3 || t0.elapsed().as_secs_f64() * 1e3 >= INNER_TARGET_MS {
            break;
        }
    }
    let each = t0.elapsed().as_secs_f64() * 1e3 / f64::from(probe);
    if each <= 0.0 {
        return 1024;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let want = (INNER_TARGET_MS / each).ceil() as u64;
    want.clamp(1, 1 << 20) as u32
}

/// Time `f` until `budget_ms` has passed and at least five iterations have run,
/// reporting the mean call and its answer.
///
/// Each iteration runs `f` as many times as [`inner_reps`] says, so the mean is
/// over calls and not over iterations, and the harness's own cost is divided by
/// that count rather than added to every call.
///
/// Every answer is summed into `acc` and the sum alone is passed to
/// `black_box`, once an iteration. Both halves of that matter. The barrier is
/// what stops the optimizer removing work nothing reads, and one an iteration
/// costs a call `1/reps` of one where a barrier per call charges each call a
/// whole one - which on a call of a few tens of nanoseconds is a large share of
/// the reading. The sum is what still makes every answer read: `acc` depends on
/// all of them, so no call can be dropped, and the alternative of simply not
/// looking at the answers would let a pure `f` be computed once and multiplied.
/// The chain costs one add a call.
///
/// A row that reads far below [`timing_floor`] is the signal that this went
/// wrong - it means the calls were hoisted rather than run, and the reading is
/// of an empty loop.
fn time_budget(budget_ms: f64, mut f: impl FnMut() -> usize) -> (f64, usize) {
    let n = f();
    let reps = inner_reps(&mut f);
    let t0 = Instant::now();
    let mut iters = 0u32;
    loop {
        for _ in 0..reps {
            black_box(f());
        }
        iters += 1;
        if iters >= 5 && t0.elapsed().as_secs_f64() * 1e3 >= budget_ms {
            break;
        }
    }
    let calls = f64::from(iters) * f64::from(reps);
    (t0.elapsed().as_secs_f64() * 1e3 / calls, n)
}

/// The middle of `v` and its spread, the largest reading over the smallest.
fn median_and_spread(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("a measured time is never NaN"));
    (v[v.len() / 2], v[v.len() - 1] / v[0])
}

const ROUNDS: usize = 5;

/// Both arms of a cell, timed in an order that rotates every round.
fn duel(
    mut left: impl FnMut() -> usize,
    mut right: impl FnMut() -> usize,
) -> (f64, f64, f64, f64, usize, usize) {
    let (mut l, mut r) = (Vec::new(), Vec::new());
    let (mut nl, mut nr) = (0usize, 0usize);
    for round in 0..ROUNDS {
        for arm in 0..2 {
            if (round + arm) % 2 == 0 {
                let (t, n) = time_budget(25.0, &mut left);
                l.push(t);
                nl = n;
            } else {
                let (t, n) = time_budget(25.0, &mut right);
                r.push(t);
                nr = n;
            }
        }
    }
    let (tl, sl) = median_and_spread(l);
    let (tr, sr) = median_and_spread(r);
    (tl, tr, sl, sr, nl, nr)
}

/// One arm alone, for an operation only one crate offers.
fn solo(mut f: impl FnMut() -> usize) -> (f64, f64, usize) {
    let mut v = Vec::new();
    let mut n = 0usize;
    for _ in 0..ROUNDS {
        let (t, got) = time_budget(25.0, &mut f);
        v.push(t);
        n = got;
    }
    let (t, s) = median_and_spread(v);
    (t, s, n)
}

fn head(title: &str) {
    println!("\n=== {title} ===");
    println!(
        "  {:<34} {:>9} {:>9} {:>7} {:>6} {:>6} {:>9} {:>9}",
        "operation", "trex ms", "regex ms", "ratio", "s.trex", "s.re", "trex n", "regex n"
    );
}

/// The smallest time the millisecond columns distinguish from zero. A baseline
/// under this prints as zero, so a ratio taken against it is a division by the
/// format's rounding floor: the quotient moves with the precision and with
/// neither crate.
const PRINTED_MS: f64 = 0.000_001;

/// A time in milliseconds, in nine columns, with the precision the value needs.
///
/// The operations the regex crate short-circuits on answer in hundreds of
/// nanoseconds, which three decimals report as zero. Fixed precision here meant
/// a row could be a measurement on one side and a rounding floor on the other,
/// and the ratio between them read as neither.
fn ms(t: f64) -> String {
    if t >= 1.0 {
        format!("{t:>9.3}")
    } else if t >= 0.001 {
        format!("{t:>9.5}")
    } else {
        format!("{t:>9.6}")
    }
}

/// What [`time_budget`] reports for a call that does nothing.
///
/// The clock read is paid once an iteration and divided across the calls
/// [`inner_reps`] fits into one, so what this measures is what a call still
/// carries on its own: entering the closure and passing a `black_box`.
///
/// Printed once per run as the resolution the table is read at: a reading this
/// close to it is a row the harness can no longer separate from nothing.
fn timing_floor() -> f64 {
    let (t, _) = time_budget(25.0, || black_box(0usize));
    t
}

/// [`timing_floor`], measured once and read by every row.
static FLOOR: std::sync::OnceLock<f64> = std::sync::OnceLock::new();

/// How far above the floor a reading must sit before its ratio is a magnitude
/// rather than a lower bound.
///
/// A reading at the floor is one the harness cannot separate from doing
/// nothing, and a ratio divided by it says more about the instrument than the
/// crate. Four is where what the floor contributes falls under a third, and a
/// row below it is marked rather than dropped: the loss is real and only its
/// size is in question.
const FLOOR_MULTIPLE: f64 = 4.0;

fn row(label: &str, tl: f64, tr: f64, sl: f64, sr: f64, nl: usize, nr: usize) {
    let floor = FLOOR.get().copied().unwrap_or(0.0);
    let note = match (nl == nr, tr > 0.0 && tr < FLOOR_MULTIPLE * floor) {
        (false, true) => "  counts differ, and the baseline is near the harness floor",
        (false, false) => "  counts differ",
        (true, true) => "  baseline near the harness floor, so the ratio is a lower bound",
        (true, false) => "",
    };
    // Both fields are seven columns wide, so the table keeps its shape whether
    // or not the ratio is one the baseline can carry.
    let ratio = if tr >= PRINTED_MS {
        format!("{:>6.2}x", tl / tr)
    } else {
        format!("{:>7}", "-")
    };
    println!(
        "  {label:<34} {} {} {ratio} {sl:>5.2}x {sr:>5.2}x {nl:>9} {nr:>9}{note}",
        ms(tl),
        ms(tr)
    );
}

/// A row for an operation only one crate offers: its time, its spread and its
/// count sit in that crate's own columns, and the other crate's read `none`.
fn only(label: &str, side: &str, t: f64, s: f64, n: usize) {
    let time = ms(t);
    let spread = format!("{s:>5.2}x");
    let count = format!("{n:>9}");
    let none9 = format!("{:>9}", "none");
    let none6 = format!("{:>6}", "none");
    let (lt, rt, ls, rs, ln, rn) = if side == "trex" {
        (&time, &none9, &spread, &none6, &count, &none9)
    } else {
        (&none9, &time, &none6, &spread, &none9, &count)
    };
    println!("  {label:<34} {lt} {rt} {:>7} {ls} {rs} {ln} {rn}", "-");
}

/// The pattern pairs, in both spellings. A trex atom is a whole token, so a
/// literal is `\bfoo\b` on the regex side and a word token is a maximal
/// identifier run.
/// A pattern and its binding twin sit next to each other - `"let" \W "="` and
/// `"let" \W:v "="` - so the difference between their rows is the binding and
/// nothing else: same literal, same tokens, same route, same corpus. Every
/// other reading of the binding tax compares patterns that also differ in what
/// they match.
const PAIRS: [(&str, &str, &str); 14] = [
    ("a literal word", "\"alpha\"", r"\balpha\b"),
    ("a literal word that is absent", "\"zzzqqq\"", r"\bzzzqqq\b"),
    ("any word token", "\\W", r"\b[A-Za-z_][A-Za-z_0-9]*\b"),
    ("any number token", "\\N", r"\b[0-9]+\b"),
    // The two above with the token bound, so a capture change is read on a
    // route that takes rather than only on one that declined. The other two
    // binding rows sit on a route that declined and on a widening prefix, and
    // a change to how captures resolve cannot be attributed while every
    // binding cell pays a different cost before the resolution. Their unbound
    // twins are the rows directly above, so each pair differs in the binding
    // and in nothing else.
    ("any word token, bound", "\\W:w", r"(?P<w>\b[A-Za-z_][A-Za-z_0-9]*\b)"),
    ("any number token, bound", "\\N:n", r"(?P<n>\b[0-9]+\b)"),
    ("either of two literals", "(\"alpha\" | \"beta\")", r"\b(?:alpha|beta)\b"),
    ("a word then punctuation", "\\W \"=\"", r"\b[A-Za-z_][A-Za-z_0-9]*\b ="),
    ("a byte-pattern inside a token", "`cond_[0-9]+`", r"\bcond_[0-9]+\b"),
    ("a literal, a word and punctuation", "\"let\" \\W \"=\"", r"\blet\b [A-Za-z_][A-Za-z_0-9]* ="),
    (
        "the same, with the word bound",
        "\"let\" \\W:v \"=\"",
        r"\blet\b (?P<v>[A-Za-z_][A-Za-z_0-9]*) =",
    ),
    (
        "a bounded repeat of a token",
        "\\W{2}",
        r"\b[A-Za-z_][A-Za-z_0-9]*\b [A-Za-z_][A-Za-z_0-9]*\b",
    ),
    ("a line-anchored literal", "^ \"let\"", r"(?m)^let\b"),
    ("a named capture then punctuation", "\\W:name \"=\"", r"(?P<name>[A-Za-z_][A-Za-z_0-9]*) ="),
];

/// Patterns the regex crate cannot express at all. A back-reference and a
/// lookaround are outside a finite automaton's language class, and the crate
/// rejects both by design rather than by omission - it guarantees linear time
/// and neither construct admits one.
const TREX_ONLY: [(&str, &str); 8] = [
    ("a back-reference to a bound token", "\\W:x \"=\" =x"),
    ("a lookahead assertion", "\\W \"=\" ~(\\N)"),
    ("a token that must be absent after", "\\W \"=\" !~\"zzzqqq\""),
    // Distance in tokens, and a count of occurrences at that distance.
    // Neither is a regular property: a token is not bounded in bytes, so a
    // byte-distance is a different question, and counting is not one a finite
    // automaton can answer at all.
    ("a token within five tokens of another", "\\W ~>5(\"alpha\")"),
    ("two numbers within eight tokens", "\\W ~>8{2,}(\\N)"),
    ("at most one number within eight", "\\W ~>8{0,1}(\\N)"),
    // The region is the group opening at the position rather than a distance
    // the pattern names, and its extent is found by counting brackets. The
    // corpus calls are `call_i(alpha, beta, N)`, so one number sits in each
    // call's parens and no group holds three.
    ("a number inside the group opening here", "\\W ~#{1,}(\\N)"),
    ("three numbers inside that group", "\\W ~#{3,}(\\N)"),
];

fn operations(text: &str) {
    let bytes = text.as_bytes();
    for (label, tp, rp) in PAIRS {
        // Said rather than asserted: a spelling one side does not take should
        // not discard a sweep that has been running for an hour.
        let p = match trex::parse(tp) {
            Ok(p) => p,
            Err(e) => {
                println!("\n  {label}: trex does not take `{tp}`: {e:?}");
                continue;
            }
        };
        let re = match Regex::new(rp) {
            Ok(re) => re,
            Err(e) => {
                println!("\n  {label}: the regex crate does not take `{rp}`: {e:?}");
                continue;
            }
        };
        let spans = trex::scan(&p, bytes);
        let names: Vec<String> = Vec::new();
        let tmpl = match trex::Template::parse("X", &names) {
            Ok(t) => t,
            Err(e) => {
                println!("\n  {label}: the replacement template does not parse: {e:?}");
                continue;
            }
        };
        head(&format!("{label}   trex `{tp}`   regex `{rp}`"));

        let (tl, tr, sl, sr, nl, nr) = duel(
            || usize::from(trex::parse(tp).is_ok()),
            || usize::from(Regex::new(rp).is_ok()),
        );
        row("compile the pattern", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) = duel(
            || usize::from(trex::is_match(&p, bytes)),
            || usize::from(re.is_match(text)),
        );
        row("does it match at all", tl, tr, sl, sr, nl, nr);

        // Two trex arms against one regex arm, because the question is both
        // how the crates compare and what the entry point bought. The scan
        // arm is what this row measured before `find` existed: it finds every
        // match in the input and takes the first.
        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::find(&p, bytes).map_or(0, |s| s.start() + 1),
            || re.find(text).map_or(0, |m| m.start() + 1),
        );
        row("where it first matches", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::cursor::find_by_full_lex(&p, bytes).map_or(0, |s| s.start() + 1),
            || re.find(text).map_or(0, |m| m.start() + 1),
        );
        row("  the same, lexing the whole input", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::scan(&p, bytes).first().map_or(0, |s| s.start() + 1),
            || re.find(text).map_or(0, |m| m.start() + 1),
        );
        row("  the same, via the whole scan", tl, tr, sl, sr, nl, nr);

        // Anchored at the end of the first match, so the arm is answering
        // where the SECOND match is rather than repeating the first.
        let from = trex::find(&p, bytes).map_or(0, |s| s.end());
        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::find_at(&p, bytes, from).map_or(0, |s| s.start() + 1),
            || re.find_at(text, from).map_or(0, |m| m.start() + 1),
        );
        row("where it next matches, from an offset", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) =
            duel(|| trex::scan(&p, bytes).len(), || re.find_iter(text).count());
        row("every match", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::captures(&p, bytes, &spans).len(),
            || re.captures_iter(text).count(),
        );
        row("captures at every match", tl, tr, sl, sr, nl, nr);

        // The eager form is handed spans from elsewhere and re-runs an
        // attempt at each one to recover what it bound; the cursor resolves
        // the save slots its own walk already carried. The two are timed
        // together because on a pattern that binds nothing they do the same
        // trivial work, and the difference only appears where one binds.
        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::captures_iter(&p, bytes).count(),
            || re.captures_iter(text).count(),
        );
        row("  the same, from the walk's own slots", tl, tr, sl, sr, nl, nr);

        // The fastest capture path each crate has, against each other. Both
        // rows above allocate a capture list per match; these two refill one
        // buffer the caller owns. Running only the convenient API would show
        // neither crate at its best, and regex's reusing form is the one its
        // own documentation points a reader chasing speed at.
        //
        // The two sides reach it differently, and the difference is the point.
        // regex's anchored form RESUMES, so its loop is one pass over the
        // input. trex's RESTARTS - a fresh scan for a byte route, a fresh lex
        // for the walk - so the same loop written that way is one pass per
        // match. The cursor holds the walk between matches, which is what makes
        // the trex side one pass as well.
        //
        // The fallback keeps the counts comparable: a pattern the walk declines
        // is taken by the allocating iterator rather than reported as zero.
        let mut locs = re.capture_locations();
        let mut slots = trex::CaptureSlots::of(&p);
        let (tl, tr, sl, sr, nl, nr) = duel(
            || {
                let mut n = 0usize;
                if let Some(mut c) = trex::captures_read_iter(&p, bytes) {
                    while c.next_into(&mut slots).is_some() {
                        n += 1;
                    }
                } else {
                    n = trex::captures_iter(&p, bytes).count();
                }
                n
            },
            || {
                let (mut at, mut n) = (0usize, 0usize);
                while let Some(m) = re.captures_read_at(&mut locs, text, at) {
                    n += 1;
                    if m.end() <= at {
                        break;
                    }
                    at = m.end();
                }
                n
            },
        );
        row("  the same, into a reused buffer", tl, tr, sl, sr, nl, nr);

        // Both sides own their output. `replace_all` hands back a `Cow` that
        // borrows the input when nothing matched, and reading its length
        // takes that borrow: against a trex rewrite that always returns an
        // owned buffer, the row would be timing a copy on one side and none
        // on the other. `into_owned` asks the two for the same thing.
        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::rewrite(&p, &tmpl, bytes).len(),
            || re.replace_all(text, "X").into_owned().len(),
        );
        row("replace every match", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::rewrite_first(&p, &tmpl, bytes).len(),
            || re.replace(text, "X").into_owned().len(),
        );
        row("replace the first match only", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) =
            duel(|| trex::split(&p, bytes).count(), || re.split(text).count());
        row("split on every match", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) =
            duel(|| trex::splitn(&p, bytes, 4).count(), || re.splitn(text, 4).count());
        row("split into at most four pieces", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::shortest_match(&p, bytes).unwrap_or(0),
            || re.shortest_match(text).unwrap_or(0),
        );
        row("how far the first match reaches", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::shortest_match_at(&p, bytes, from).unwrap_or(0),
            || re.shortest_match_at(text, from).unwrap_or(0),
        );
        row("  the same, from an offset", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::captures_first(&p, bytes).map_or(0, |m| m.captures().len() + 1),
            || re.captures(text).map_or(0, |c| c.len()),
        );
        row("captures of the first match", tl, tr, sl, sr, nl, nr);

        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::captures_at(&p, bytes, from).map_or(0, |m| m.captures().len() + 1),
            || re.captures_at(text, from).map_or(0, |c| c.len()),
        );
        row("  the same, from an offset", tl, tr, sl, sr, nl, nr);

        // Two chunk sizes, because one of them measures the chunk size rather
        // than the streaming. A chunk under the parallel lexer's threshold
        // lexes on one core, so a single row there reads a serial lex against
        // the whole-input arm's parallel one and reports the difference as
        // streaming's cost. The second row is sized off the threshold this
        // process actually resolved, so each chunk reaches the pool.
        let small = 64 << 10;
        let large = trex::parallel_lex::active_parallel_threshold().saturating_mul(2).max(small);

        let chunks: Vec<&[u8]> = bytes.chunks(small).collect();
        let (t, s, n) = solo(|| trex::scan_chunked(&p, chunks.iter().copied()).len());
        only("every match, fed in 64 kB chunks (a serial lex a chunk)", "trex", t, s, n);

        let chunks: Vec<&[u8]> = bytes.chunks(large).collect();
        let (t, s, n) = solo(|| trex::scan_chunked(&p, chunks.iter().copied()).len());
        only(
            &format!("  the same, in {} kB chunks (the pool a chunk)", large >> 10),
            "trex",
            t,
            s,
            n,
        );

        // The control: the section's first timed row, run again as its last.
        // It is the same work both times, so its drift from the first reading
        // is attributable to position in the run and to load, and to nothing
        // else. The spread column cannot say that on its own - a spread is
        // taken across the two arms, so it carries any real difference between
        // trex and regex as well as any disturbance.
        //
        // A section whose control drifts is a section whose cells moved,
        // whatever their individual spreads looked like.
        let (tl, tr, sl, sr, nl, nr) = duel(
            || usize::from(trex::is_match(&p, bytes)),
            || usize::from(re.is_match(text)),
        );
        row("CONTROL does it match at all, again", tl, tr, sl, sr, nl, nr);
    }
}

/// Separators that lex as punctuation and nothing else, so a corpus of them
/// holds no word and no number token until whatever follows.
fn barren(bytes: usize) -> String {
    let mut s = String::with_capacity(bytes + 32);
    while s.len() < bytes {
        s.push_str(" ; , ; , ; , ;\n");
    }
    s
}

/// First-match timings on an input whose first match is at the very end.
///
/// The routes that answer a first match win two different ways, and the corpus
/// above cannot tell them apart because its first match is in the first line.
/// A literal's bytes are rare, so its route SKIPS: the byte search passes over
/// most of the input without a probe. A byte that could open a word token is
/// everywhere, so the kind route cannot skip and wins only by STOPPING.
///
/// Here there is nothing to stop at until the end, so a route that only stops
/// pays its candidate scan in full, and this says what that costs. A route that
/// skips should barely notice the difference.
fn first_match_when_it_is_late() {
    let head = barren(2 << 20);
    println!("\n=== the first match is at the very end   {} bytes ===", head.len());
    println!(
        "  {:<34} {:>9} {:>9} {:>7} {:>6} {:>6} {:>9} {:>9}",
        "operation", "trex ms", "regex ms", "ratio", "s.trex", "s.re", "trex n", "regex n"
    );
    for (label, tp, rp, tail) in [
        ("a word token, found last", "\\W", r"\b[A-Za-z_][A-Za-z_0-9]*\b", "alpha\n"),
        ("a number token, found last", "\\N", r"\b[0-9]+\b", "4242\n"),
        ("a literal word, found last", "\"alpha\"", r"\balpha\b", "alpha\n"),
    ] {
        let mut text = head.clone();
        text.push_str(tail);
        let bytes = text.as_bytes();
        // Said rather than dropped: a spelling one side does not take must not
        // read as a row that was simply not run.
        let p = match trex::parse(tp) {
            Ok(p) => p,
            Err(e) => {
                println!("\n  {label}: trex does not take `{tp}`: {e:?}");
                continue;
            }
        };
        let re = match Regex::new(rp) {
            Ok(re) => re,
            Err(e) => {
                println!("\n  {label}: the regex crate does not take `{rp}`: {e:?}");
                continue;
            }
        };
        let (tl, tr, sl, sr, nl, nr) = duel(
            || trex::find(&p, bytes).map_or(0, |s| s.start() + 1),
            || re.find(&text).map_or(0, |m| m.start() + 1),
        );
        row(label, tl, tr, sl, sr, nl, nr);
    }
}

fn expressible_only_by_trex(text: &str) {
    let bytes = text.as_bytes();
    println!("\n=== patterns the regex crate cannot express ===");
    println!(
        "  {:<34} {:>9} {:>9} {:>7} {:>6} {:>6} {:>9} {:>9}",
        "operation", "trex ms", "regex ms", "ratio", "s.trex", "s.re", "trex n", "regex n"
    );
    for (label, tp) in TREX_ONLY {
        // Said rather than asserted: one spelling this build does not take
        // should not discard a sweep that has been running for an hour.
        match trex::parse(tp) {
            Ok(p) => {
                let (t, s, n) = solo(|| trex::scan(&p, bytes).len());
                only(&format!("{label}  `{tp}`"), "trex", t, s, n);
            }
            Err(e) => println!("  {label:<34}  `{tp}` does not parse: {e:?}"),
        }
    }
    // Splitting on the separators at one bracket depth. The regex crate has
    // `split` and cannot have this one: telling a nested comma from a
    // top-level one means counting brackets, and a regular language cannot
    // count. Timed beside the flat split so the cost of knowing the depth is
    // visible rather than implied.
    match trex::parse("\",\"") {
        Ok(p) => {
            let (t, s, n) = solo(|| trex::split(&p, bytes).count());
            only("split on every comma", "trex", t, s, n);
            let (t, s, n) = solo(|| trex::split_at_depth(&p, bytes, 1).count());
            only("split on the top-level commas only", "trex", t, s, n);
        }
        Err(e) => println!("  a comma does not parse: {e:?}"),
    }
}

/// Many patterns asked of one input, against the regex crate's `RegexSet`.
///
/// The two crates save different things here. A `RegexSet` compiles its
/// patterns into one automaton and reads the input once, so it saves passes.
/// A trex set saves the lex, which is the pattern-independent cost, and runs
/// each pattern's walk over one token stream - and saves even that for the
/// members a byte route answers, which never reach the lexer at all.
///
/// Both sides are asked the same two questions: whether anything matched, and
/// which members did.
fn many_patterns_at_once(text: &str) {
    let bytes = text.as_bytes();
    let pairs: &[(&str, &str)] = &[
        ("\"alpha\"", r"\balpha\b"),
        ("\"zzzqqq\"", r"\bzzzqqq\b"),
        ("\\W \"=\"", r"\b[A-Za-z_][A-Za-z_0-9]*\b ="),
        ("`cond_[0-9]+`", r"\bcond_[0-9]+\b"),
        ("\"let\" \\W \"=\"", r"\blet\b [A-Za-z_][A-Za-z_0-9]* ="),
        ("\"nowhere_at_all\"", r"\bnowhere_at_all\b"),
        ("\\N", r"\b[0-9]+\b"),
        ("\"beta\"", r"\bbeta\b"),
    ];
    // Said rather than asserted, and said for the member that failed: a
    // spelling one side does not take must not quietly shorten the set, since
    // a set of seven timed against a set of eight is not a comparison.
    let mut tp = Vec::new();
    let mut res = Vec::new();
    for (t, r) in pairs {
        match trex::parse(t) {
            Ok(p) => tp.push(p),
            Err(e) => {
                println!("\n  set member `{t}`: trex does not take it: {e:?}");
                return;
            }
        }
        match regex::Regex::new(r) {
            Ok(re) => res.push(re),
            Err(e) => {
                println!("\n  set member `{r}`: the regex crate does not take it: {e:?}");
                return;
            }
        }
    }
    let rp: Vec<&str> = pairs.iter().map(|(_, r)| *r).collect();
    let reset = match regex::RegexSet::new(&rp) {
        Ok(s) => s,
        Err(e) => {
            println!("\n  the regex crate does not take the set: {e:?}");
            return;
        }
    };
    let set = trex::pattern_set::PatternSet::new(tp);
    head(&format!("{} patterns asked of one input", pairs.len()));

    let (tl, tr, sl, sr, nl, nr) =
        duel(|| usize::from(set.is_match(bytes)), || usize::from(reset.is_match(text)));
    row("did any of them match", tl, tr, sl, sr, nl, nr);

    let (tl, tr, sl, sr, nl, nr) =
        duel(|| set.matches(bytes).len(), || reset.matches(text).iter().count());
    row("which of them matched", tl, tr, sl, sr, nl, nr);

    // The same question asked one pattern at a time, which is what the set
    // has to beat to be worth having: it pays one lex where this pays one per
    // member that needs one.
    let ones = set.patterns();
    let (tl, tr, sl, sr, nl, nr) = duel(
        || ones.iter().filter(|p| trex::is_match(p, bytes)).count(),
        || res.iter().filter(|r| r.is_match(text)).count(),
    );
    row("  the same, one pattern at a time", tl, tr, sl, sr, nl, nr);

    // `RegexSet` states that it reports which patterns match and not where:
    // one automaton carrying every pattern loses which of them accepted. A
    // token set's saving is the shared lex rather than a shared automaton, so
    // each member is still walked as itself and keeps its span. The row is
    // one-sided because there is nothing on the other side to time.
    let (t, s, n) = solo(|| set.matches_with_spans(bytes).len());
    only("which of them matched, and where", "trex", t, s, n);
}

/// One untimed pass over the corpus on each side, so the first timed section
/// is not the one paying for a cold start.
///
/// The control cell caught this. Across eleven sections ten drift between -7.4
/// and +6.3 percent between their first row and its repeat, and "any word
/// token" read 8.941 ms then 0.928 ms for identical work - 9.6x, with its
/// spread column independently at 2.87 to 4.83 where every other section sits
/// at 1.01 to 1.05. Foreign load was 4 percent before the run and 2 after,
/// which does not explain 9.6x. That section is the first to force a full lex,
/// so the first reading was buying the lexer's warm-up and charging it to the
/// pattern.
///
/// A scan and a find on each side, over the same corpus every section uses, so
/// the pools, the allocator and the caches are in the state the second section
/// would have found them in.
fn warm(text: &str) {
    let bytes = text.as_bytes();
    let p = match trex::parse("\\W") {
        Ok(p) => p,
        Err(e) => {
            println!("\n  warm-up: trex does not take `\\W`: {e:?}");
            return;
        }
    };
    black_box(trex::scan(&p, bytes).len());
    black_box(trex::find(&p, bytes).is_some());
    let re = match Regex::new(r"\b[A-Za-z_][A-Za-z_0-9]*\b") {
        Ok(re) => re,
        Err(e) => {
            println!("\n  warm-up: the regex crate does not take the word pattern: {e:?}");
            return;
        }
    };
    black_box(re.find_iter(text).count());
    black_box(re.find(text).is_some());
}

fn main() {
    // Statements to build the corpus from, 200_000 unless the environment names
    // another. A ladder of sizes is run as one invocation each rather than as a
    // sweep inside one, so every size gets its own warm cache and its own
    // timing floor, and a size can be retaken without retaking the rest.
    //
    // Absent and unreadable are not the same: a value nobody wrote means take
    // the default, and a value somebody wrote and this cannot read means the
    // run would report a size the caller did not ask for.
    let statements = match std::env::var("TREX_BENCH_STATEMENTS") {
        Err(std::env::VarError::NotPresent) => 200_000,
        Err(e) => panic!("TREX_BENCH_STATEMENTS is set and unreadable: {e}"),
        Ok(v) => v
            .parse::<usize>()
            .unwrap_or_else(|e| panic!("TREX_BENCH_STATEMENTS is {v:?}, not a count: {e}")),
    };
    let text = corpus(statements);
    println!("corpus: {} bytes, {} lines", text.len(), text.lines().count());
    println!("trex {}", trex::version());
    // What this binary is, stated by the binary. A reading is only comparable
    // with another taken under the same three, and none of them is visible in
    // the numbers: the `gpu` feature is compiled in by default and a device is
    // found at run time, so the same source gives four configurations and a row
    // from one of them looks exactly like a row from another.
    println!(
        "build: gpu feature {}, device {}, tandem feature {}",
        if cfg!(feature = "gpu") { "on" } else { "off" },
        if trex::device_available() { "PRESENT" } else { "absent" },
        if cfg!(feature = "tandem") { "on" } else { "off" },
    );
    // Read three times and reduced by the middle one, for the reason every
    // cell below is: this box is shared, and a single reading of a
    // sub-nanosecond quantity says as much about what else was running as
    // about the loop.
    let floors = (0..3).map(|_| timing_floor()).collect();
    let (floor, floor_spread) = median_and_spread(floors);
    FLOOR.set(floor).expect("the floor is set once, here, before any row is printed");
    // In nanoseconds, and not through `ms`. This is the only figure in the run
    // that is a fraction of a nanosecond, and the column format stops at six
    // decimals of a millisecond - which is half a nanosecond, so `ms` prints
    // the floor as zero and the number the table is read against cannot be
    // read at all.
    println!(
        "timing floor: {:.4} ns a call, what the harness reports for doing nothing, spread {:.2}x. \
         Each timed iteration runs its arm enough times to reach {} ms, so the clock read and the \
         barrier are divided across those calls rather than charged to each; a baseline under {}x \
         the floor is still marked, and its ratio is a lower bound",
        floor * 1e6,
        floor_spread,
        INNER_TARGET_MS,
        FLOOR_MULTIPLE
    );
    warm(&text);
    operations(&text);
    first_match_when_it_is_late();
    many_patterns_at_once(&text);
    expressible_only_by_trex(&text);
    println!("\nDONE");
}
