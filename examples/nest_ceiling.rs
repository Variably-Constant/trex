//! Where the pattern parser runs out of stack.
//!
//! The parser descends recursively, so nesting depth is stack depth, and
//! [`trex::parser::NEST_LIMIT`] is the depth past which a pattern is refused
//! rather than allowed to overflow. This reports the depth at which an
//! unguarded parse actually dies, so that limit can be set from this parser's
//! frames rather than from another crate's default.
//!
//! Each depth is parsed by a fresh copy of this program and the parent reads
//! its exit status. That is the only way to ask: a stack overflow aborts the
//! process rather than unwinding, so it is not a panic a thread can fail to
//! join on and not something `catch_unwind` sees. A child that exits cleanly
//! survived the depth, whether the parse succeeded or was refused; a child
//! that dies did not.
//!
//! With one argument, parse at that depth and exit. With none, drive the
//! search.

use std::env;
use std::hint::black_box;
use std::process::Command;

/// The deepest nesting to try before giving up on finding an edge at all.
const DEEPEST: u32 = 1 << 22;

/// Parse `(((..."a"...)))` at `depth`, with the guard raised past anything
/// under test so the parser recurses rather than refusing: this measures the
/// stack, not the guard.
fn parse_at(depth: u32) {
    let src = format!("{}\"a\"{}", "(".repeat(depth as usize), ")".repeat(depth as usize));
    let shapes = trex::custom::ShapeSet::new();
    let parsed = trex::parser::parse_with_shapes_to_depth(&src, &shapes, u32::MAX);
    // A refusal is a return, and returning at all is the answer being sought.
    black_box(parsed.is_ok());
}

/// The stack a spawned thread is given when nothing asks for one, which is
/// what a test harness thread runs on. `RUST_MIN_STACK` overrides it, and the
/// search reports the size it actually used so a reading taken under a
/// different one is not mistaken for this.
fn spawned_stack() -> usize {
    env::var("RUST_MIN_STACK")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2 * 1024 * 1024)
}

/// [`parse_at`] on a spawned thread rather than the process's first one.
///
/// The two differ and the difference is the whole point of asking: on Windows
/// the main thread's stack is fixed by the linker at a megabyte, while a
/// spawned thread takes [`spawned_stack`]. A limit read off the main thread
/// alone describes neither the threads a test harness runs on nor the ones a
/// caller spawns.
fn parse_at_on_thread(depth: u32) {
    let handle = std::thread::Builder::new()
        .stack_size(spawned_stack())
        .spawn(move || parse_at(depth))
        .expect("spawn a thread to parse on");
    // An overflow aborts the process, so this join only returns when the depth
    // was survivable.
    handle.join().expect("the parsing thread to finish");
}

/// Whether a fresh copy of this program survives parsing at `depth`, on the
/// main thread or on a spawned one.
fn survives(exe: &str, depth: u32, on_thread: bool) -> bool {
    let mut cmd = Command::new(exe);
    cmd.arg(depth.to_string());
    if on_thread {
        cmd.arg("thread");
    }
    match cmd.status() {
        Ok(status) => status.success(),
        Err(e) => {
            println!("  could not run {exe} at depth {depth}: {e}");
            false
        }
    }
}

/// The deepest depth that lived and the shallowest that died, by doubling to
/// find an edge and bisecting to place it.
fn ceiling(exe: &str, on_thread: bool) -> Option<(u32, u32)> {
    let mut ok = 1u32;
    let mut dead = 0u32;
    let mut probe = 16u32;
    while probe < DEEPEST {
        if survives(exe, probe, on_thread) {
            ok = probe;
            probe *= 2;
        } else {
            dead = probe;
            break;
        }
    }
    if dead == 0 {
        println!("  no depth up to {DEEPEST} killed a child; deepest tried {ok}");
        return None;
    }
    let (mut lo, mut hi) = (ok, dead);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if survives(exe, mid, on_thread) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some((lo, hi))
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if let Some(depth) = args.get(1) {
        let on_thread = args.get(2).is_some_and(|a| a == "thread");
        match depth.parse::<u32>() {
            Ok(d) if on_thread => parse_at_on_thread(d),
            Ok(d) => parse_at(d),
            Err(e) => {
                println!("depth `{depth}` is not a number: {e}");
                std::process::exit(2);
            }
        }
        return;
    }

    let exe = args.first().map_or_else(
        || {
            println!("this program was started with no name to re-run itself by");
            std::process::exit(2);
        },
        String::clone,
    );
    println!("trex {}", trex::version());
    println!("the crate refuses past {} groups", trex::parser::NEST_LIMIT);
    println!("a child parses at each depth; the parent reads whether it lived");
    println!(
        "profile: {}",
        if cfg!(debug_assertions) { "debug, larger frames" } else { "release" }
    );

    println!("\non the process's first thread:");
    if let Some((lo, hi)) = ceiling(&exe, false) {
        println!("  deepest that lived: {lo}");
        println!("  first that died:    {hi}");
    }

    println!("\non a spawned thread of {} bytes, which is what a", spawned_stack());
    println!("test harness thread and a caller's own thread run on:");
    if let Some((lo, hi)) = ceiling(&exe, true) {
        println!("  deepest that lived: {lo}");
        println!("  first that died:    {hi}");
        let limit = trex::parser::NEST_LIMIT;
        if limit >= lo {
            println!(
                "\n  NEST_LIMIT is {limit}, at or past the {lo} this stack holds, so the\n  \
                 guard is never reached and a deep pattern takes the process down"
            );
        }
    }
    println!("\nDONE");
}
