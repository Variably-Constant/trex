//! Which rung of a ladder answered a call.
//!
//! Every public operation is a ladder of routes over an engine, and which rung
//! answers decides what the call costs. Reading that off the source is what the
//! trace replaces: a row measured at sixteen milliseconds and a row measured at
//! three can enter the same function and leave by different rungs.
//!
//! Off unless `TREX_TRACE` is set in the environment, read once per process.
//!
//! One line is one call into one ladder, never one token: a scan over a million
//! tokens prints a single line. That also makes it wrong for a benchmark, which
//! calls an operation thousands of times - trace a single call and read the
//! line it prints.
//!
//! What is kept for a caller to read back is kept without a lock: every thread
//! pushes what it saw onto a lock-free queue, and a take drains the queue and
//! adds up what it drained. Nothing is added up where it is pushed, so a push
//! costs the same whether it is the first of its name or the millionth.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use crossbeam_queue::SegQueue;

/// Whether `TREX_TRACE` has been read: 0 before, 1 when it was absent, 2 when
/// it was set. Two threads that both read it first read the same value.
static ON: AtomicU8 = AtomicU8::new(0);

/// Whether the trace is on, read once from `TREX_TRACE`.
#[must_use]
pub fn on() -> bool {
    match ON.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var_os("TREX_TRACE").is_some();
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Whether rungs are being kept for a caller to read back, as [`record`] and
/// [`stop`] set it.
static RECORDING: AtomicBool = AtomicBool::new(false);

/// How many [`Recording`]s are held, each keeping the rungs whatever
/// [`RECORDING`] says.
static HELD: AtomicUsize = AtomicUsize::new(0);

/// Keeps the rungs for as long as it is held, for a caller in a process that
/// outlives the call, where another caller may be keeping them at the same
/// time: the rungs are kept while any `Recording` is held or [`record`] is in
/// force, so one holder's drop never stops another's.
pub struct Recording {
    _private: (),
}

impl Recording {
    /// Keep the rungs until this is dropped.
    #[must_use]
    pub fn start() -> Self {
        HELD.fetch_add(1, Ordering::SeqCst);
        Recording { _private: () }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        HELD.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One rung that answered: the ladder, the rung, and the bytes it was given.
///
/// The bytes are the input's length rather than what the rung read, which is
/// what makes a rung's share of a scan readable: a route that reads a
/// thousandth of what it is handed and one that reads all of it are told
/// apart by the time, not by this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rung {
    /// The ladder the call entered.
    pub ladder: String,
    /// The rung that answered it.
    pub rung: String,
    /// The input's length.
    pub bytes: usize,
}

/// The rungs kept since recording began, oldest first.
static RECORDED: SegQueue<Rung> = SegQueue::new();

/// Each rung kept, again, for the totals.
///
/// A queue of its own because the two are read by callers that want different
/// things and would otherwise consume each other: a report naming the route it
/// took TAKES [`RECORDED`], so that the rungs it reads are this input's and not
/// the whole run's, and a report counting the routes would then find it empty.
/// Each reader drains only its own queue, so both readings survive the other
/// being asked for.
static ANSWERED: SegQueue<Rung> = SegQueue::new();

/// How many calls one rung of one ladder answered, and the bytes they were
/// given between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RungTotal {
    /// The ladder the calls entered.
    pub ladder: String,
    /// The rung that answered them.
    pub rung: String,
    /// How many calls it answered.
    pub calls: u64,
    /// The bytes those calls were given.
    pub bytes: u64,
}

/// The tokens lexed since recording began.
///
/// An atomic rather than a queue, because a lex is not always a whole input. A
/// route that lexes a token's worth of bytes at every occurrence of a literal
/// reaches the count once an occurrence, and fifty thousand occurrences over a
/// corpus of a few megabytes is fifty thousand pushes where one addition does.
static LEXED_TOKENS: AtomicU64 = AtomicU64::new(0);

/// The nanoseconds spent lexing since recording began, beside
/// [`LEXED_TOKENS`] for the same reason.
static LEXED_NANOS: AtomicU64 = AtomicU64::new(0);

/// How many calls each rung answered and the bytes each was given, every
/// ladder's in the order its first call was kept, since recording began or
/// since the last take.
#[must_use]
pub fn take_totals() -> Vec<RungTotal> {
    let mut out: Vec<RungTotal> = Vec::new();
    while let Some(kept) = ANSWERED.pop() {
        let bytes = kept.bytes as u64;
        match out.iter_mut().find(|t| t.ladder == kept.ladder && t.rung == kept.rung) {
            Some(t) => {
                t.calls += 1;
                t.bytes += bytes;
            }
            None => out.push(RungTotal { ladder: kept.ladder, rung: kept.rung, calls: 1, bytes }),
        }
    }
    out
}

/// Count one lex, where the rungs are being kept.
///
/// The caller times the lex only when [`keeping`] answers true, so a scan
/// that reports no statistics reads one atomic and does no clock work at
/// all: a lex is the hot path, and a timer on it would cost every caller to
/// serve the few that ask.
pub fn lexed(tokens: usize, elapsed: Duration) {
    if !keeping() {
        return;
    }
    let tokens = u64::try_from(tokens).expect("a token count within the counter's width");
    let nanos = u64::try_from(elapsed.as_nanos())
        .expect("a lex shorter than the five hundred years the counter holds");
    LEXED_TOKENS.fetch_add(tokens, Ordering::Relaxed);
    LEXED_NANOS.fetch_add(nanos, Ordering::Relaxed);
}

/// Whether the rungs are being kept, for a caller deciding whether to time
/// itself.
#[must_use]
pub fn keeping() -> bool {
    RECORDING.load(Ordering::Relaxed) || HELD.load(Ordering::Relaxed) > 0
}

/// One phase's name, the number of calls that entered it, and what they took
/// between them.
pub type Tally = (&'static str, u64, Duration);

/// Each phase as it ended: its name and what it took.
static PHASES: SegQueue<(&'static str, Duration)> = SegQueue::new();

/// A phase being timed. Its drop records what the scope holding it took.
pub struct Phase {
    name: &'static str,
    began: std::time::Instant,
}

impl Drop for Phase {
    fn drop(&mut self) {
        PHASES.push((self.name, self.began.elapsed()));
    }
}

/// Time the scope this is held for under `name`, where rungs are being kept.
///
/// `None` where they are not, so a scan reporting nothing reads one atomic and
/// starts no clock. A phase held inside another counts in both, so the readings
/// are shares of a tree and their sum can exceed the call.
#[must_use]
pub fn phase(name: &'static str) -> Option<Phase> {
    keeping().then(|| Phase { name, began: std::time::Instant::now() })
}

/// Each count as it was handed over: its name and what it added.
static COUNTS: SegQueue<(&'static str, u64)> = SegQueue::new();

/// Add `n` to the count under `name`, where rungs are being kept.
///
/// For a quantity a phase cannot report: how many times the inside of a loop
/// took one branch rather than another. A phase there would push once per
/// iteration and cost more than the iteration, where the caller can carry a
/// local counter for nothing and hand it over once.
pub fn counted(name: &'static str, n: u64) {
    if keeping() {
        COUNTS.push((name, n));
    }
}

/// What each named count reached since recording began or since the last take,
/// in the order first handed over.
#[must_use]
pub fn take_counts() -> Vec<(&'static str, u64)> {
    let mut out: Vec<(&'static str, u64)> = Vec::new();
    while let Some((name, n)) = COUNTS.pop() {
        match out.iter_mut().find(|(held, _)| *held == name) {
            Some((_, seen)) => *seen += n,
            None => out.push((name, n)),
        }
    }
    out
}

/// What each phase took and how many times it ran, since recording began or
/// since the last take, in the order each first ended.
#[must_use]
pub fn take_phases() -> Vec<Tally> {
    let mut out: Vec<Tally> = Vec::new();
    while let Some((name, took)) = PHASES.pop() {
        match out.iter_mut().find(|(held, _, _)| *held == name) {
            Some((_, calls, spent)) => {
                *calls += 1;
                *spent += took;
            }
            None => out.push((name, 1, took)),
        }
    }
    out
}

/// The tokens lexed and the seconds spent lexing since recording began or
/// since the last take.
#[must_use]
pub fn take_lexing() -> (u64, Duration) {
    let tokens = LEXED_TOKENS.swap(0, Ordering::SeqCst);
    let nanos = LEXED_NANOS.swap(0, Ordering::SeqCst);
    (tokens, Duration::from_nanos(nanos))
}

/// Forget everything kept so far: the rungs, the totals, the phases, the counts
/// and the lexing, for a caller about to read back only what its own call
/// keeps.
pub fn clear() {
    LEXED_TOKENS.store(0, Ordering::SeqCst);
    LEXED_NANOS.store(0, Ordering::SeqCst);
    while RECORDED.pop().is_some() {}
    while ANSWERED.pop().is_some() {}
    while PHASES.pop().is_some() {}
    while COUNTS.pop().is_some() {}
}

/// Keep every rung named from now on, for a caller that reports which route
/// answered rather than reading the standard error; independent of the
/// printing `TREX_TRACE` turns on.
pub fn record() {
    RECORDING.store(true, Ordering::SeqCst);
}

/// Stop keeping them.
///
/// The counterpart of [`record`], for a caller that reads which rung answered
/// and then measures something: keeping a rung pushes twice per call, so a
/// measurement taken with the trace still on prices the trace as well as the
/// work. What has already been kept stays kept until it is taken.
pub fn stop() {
    RECORDING.store(false, Ordering::SeqCst);
}

/// The rungs kept since recording began or since the last take, oldest
/// first.
#[must_use]
pub fn take_recorded() -> Vec<Rung> {
    let mut out = Vec::new();
    while let Some(kept) = RECORDED.pop() {
        out.push(kept);
    }
    out
}

/// Name the rung of `ladder` that answered, where the trace is on or the
/// rungs are being kept.
///
/// `bytes` is the input's length, because the rungs differ in how much of it
/// they read and the line is worth nothing without it.
pub fn rung(ladder: &str, rung: &str, bytes: usize) {
    if on() {
        eprintln!("trex route: {ladder} over {bytes} bytes -> {rung}");
    }
    if keeping() {
        let named = Rung { ladder: ladder.to_string(), rung: rung.to_string(), bytes };
        ANSWERED.push(named.clone());
        RECORDED.push(named);
    }
}

#[cfg(test)]
mod tests {
    use super::{Recording, counted, keeping, on, phase, rung, take_counts, take_phases, take_recorded, take_totals};

    #[test]
    fn the_trace_is_off_unless_the_environment_asks_for_it() {
        // The default matters more than the printing: a rung call is on
        // every ladder in the crate, so the off path has to be a read of the
        // flag and nothing else. The suite runs without TREX_TRACE set.
        assert!(!on(), "TREX_TRACE is set, so this process cannot check the default");
        rung("a ladder", "a rung", 0);
    }

    #[test]
    fn what_many_threads_keep_at_once_is_all_taken_and_added_up() {
        // Every name here is this test's own, so a reading another test keeps
        // at the same time is left out of what is checked rather than counted.
        // The one test in the crate that takes from the queues, so no other
        // test drains what this one kept before it reads it back.
        const THREADS: usize = 8;
        const EACH: usize = 1000;
        let outer = Recording::start();
        let inner = Recording::start();
        drop(inner);
        assert!(keeping(), "one holder's drop must not stop another's keeping");
        std::thread::scope(|s| {
            for _ in 0..THREADS {
                s.spawn(|| {
                    for k in 0..EACH {
                        rung("the trace test's ladder", if k % 2 == 0 { "even" } else { "odd" }, 3);
                        counted("the trace test's count", 2);
                        let timed = phase("the trace test's phase");
                        drop(timed);
                    }
                });
            }
        });
        drop(outer);
        let totals: Vec<_> = take_totals().into_iter().filter(|t| t.ladder == "the trace test's ladder").collect();
        let calls: u64 = totals.iter().map(|t| t.calls).sum();
        assert_eq!(calls, (THREADS * EACH) as u64, "{totals:?}");
        assert_eq!(totals.len(), 2, "{totals:?}");
        assert!(totals.iter().all(|t| t.calls == (THREADS * EACH / 2) as u64 && t.bytes == t.calls * 3), "{totals:?}");
        let counted_up: Vec<_> = take_counts().into_iter().filter(|(n, _)| *n == "the trace test's count").collect();
        assert_eq!(counted_up, vec![("the trace test's count", (THREADS * EACH * 2) as u64)]);
        let phases: Vec<_> = take_phases().into_iter().filter(|(n, _, _)| *n == "the trace test's phase").collect();
        assert_eq!(phases.len(), 1, "{phases:?}");
        assert_eq!(phases[0].1, (THREADS * EACH) as u64);
        let kept = take_recorded().into_iter().filter(|r| r.ladder == "the trace test's ladder").count();
        assert_eq!(kept, THREADS * EACH);
        assert!(take_totals().iter().all(|t| t.ladder != "the trace test's ladder"), "a take leaves nothing to take twice");
    }
}
