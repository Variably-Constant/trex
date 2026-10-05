//! What the parallel lex loses to a busy neighbor, split by what the
//! neighbor is doing.
//!
//! The lex-kernel pipeline's lex stage reads 18.5 ms at 16.5 MB over eight
//! ranges against 8.7 ms for the same lex run alone, and the thread beside it
//! is doing two separable things: holding a core, and walking memory for the
//! selection at about eleven gigabytes a second. This times the lex against a
//! neighbor that does one without the other.
//!
//! Two tables, each of three arms rotating every round and each arm run a
//! second time in the round, the rerun's ratio against its arm being that
//! arm's control, which reads about 1.00x once the rounds have absorbed the
//! box's noise. The arms contend for different things by design, so one
//! arm's repeat cannot speak for another.
//!
//! The first is the neighbor: the lex alone; beside a thread spinning on
//! registers and touching no memory; beside a thread streaming a buffer far
//! past the last level of cache.
//!
//! The second is the buffers: the whole lex; the same input lexed as ranges
//! with one workspace kept across them; lexed as ranges with a workspace taken
//! fresh for each, the cost `scan_pipelined`'s pool of three exists to avoid;
//! and lexed as ranges from a pool of three with every finished range handed
//! to a thread that reads it whole, which is the pipeline's own shape with the
//! device taken out. That last arm is there because the pipeline's lex reads
//! 2.13x its solo time and neither neighbor above accounts for it: those
//! touch their own buffers, while the reader pulls lines the lexing cores have
//! just written out of their caches.
//!
//! Nothing here uses the device, so the run needs no CUDA host. It needs a
//! quiet one: a foreign tenant is a neighbor the arms cannot tell from their
//! own.
//!
//! Run: `cargo run --release --example lex_contention -- [corpus ...]`.

use std::hint::black_box;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use trex::lexer::blob_runs_parallel;
use trex::parallel_lex::{
    SignificantWorkspace, lex_significant_parallel_into, lex_significant_range_into, safe_boundaries,
};

const ROUNDS: usize = 21;
/// Ranges the second table lexes in, the count the pipeline's 2.13x was
/// measured at.
const RANGES: usize = 8;
const TAG_PAIRS: [usize; 2] = [262_144, 2_097_152];
/// The streaming neighbor's buffer, sized past any last-level cache so every
/// pass of it reaches memory rather than answering from cache.
const STREAM_BYTES: usize = 64 << 20;
/// Bytes the streaming neighbor touches between checks of the stop flag,
/// which bounds how long it outlives the lex it was there to disturb.
const STREAM_STEP: usize = 4096;

/// `pairs` lines of a word and a number: the corpus the device benches build.
fn tag_pairs(pairs: usize) -> Vec<u8> {
    let mut input = String::new();
    for i in 0..pairs {
        input.push_str("tag ");
        input.push_str(&(i % 1000).to_string());
        input.push('\n');
    }
    input.into_bytes()
}

/// The middle sample, sorting `v`.
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// What a neighboring thread does while the lex runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Beside {
    /// No neighbor at all.
    Nothing,
    /// One core held, no memory touched.
    Spinning,
    /// One core held, streaming a buffer past the last level of cache.
    Streaming,
}

/// Milliseconds to lex `input` into `ws` once, with a neighbor of `beside`
/// running for exactly that call.
fn lex_beside(input: &[u8], ws: &mut SignificantWorkspace, beside: Beside) -> f64 {
    if beside == Beside::Nothing {
        let t = Instant::now();
        lex_significant_parallel_into(input, ws);
        black_box(ws.kinds.len());
        return t.elapsed().as_secs_f64() * 1e3;
    }
    // Both buffers are allocated and first-touched here, before the clock
    // starts: filling them takes longer than the smaller cells' whole lex, so
    // a thread that allocates inside the timed window would time the
    // allocation rather than the traffic it is there to make.
    let (src, mut dst) = if beside == Beside::Streaming {
        (vec![1u8; STREAM_BYTES], vec![2u8; STREAM_BYTES])
    } else {
        (Vec::new(), Vec::new())
    };
    let stop = AtomicBool::new(false);
    std::thread::scope(|sc| {
        sc.spawn(|| match beside {
            Beside::Streaming => {
                // Copies between the two buffers, far past the last level of
                // cache, so the arm moves bytes both ways rather than walking
                // pages: every line of each chunk is read and written, which
                // is the traffic a device upload of the kinds puts on the bus.
                while !stop.load(Ordering::Relaxed) {
                    for (s, d) in src.chunks(STREAM_STEP).zip(dst.chunks_mut(STREAM_STEP)) {
                        d.copy_from_slice(s);
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                }
                black_box(dst[0]);
            }
            Beside::Spinning | Beside::Nothing => {
                let mut x = 1u64;
                while !stop.load(Ordering::Relaxed) {
                    x = black_box(x).wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                }
                black_box(x);
            }
        });
        let t = Instant::now();
        lex_significant_parallel_into(input, ws);
        black_box(ws.kinds.len());
        let ms = t.elapsed().as_secs_f64() * 1e3;
        stop.store(true, Ordering::Relaxed);
        ms
    })
}

/// Milliseconds to lex `input` as the ranges `bounds` names, in order and
/// against the whole input's blob table, into `held` reused across them when
/// `recycle` is set or into a workspace taken fresh for each range when it is
/// not. The blob table is built outside the clock, as the pipeline times it
/// separately.
fn lex_ranges(
    input: &[u8],
    bounds: &[usize],
    held: &mut SignificantWorkspace,
    recycle: bool,
) -> f64 {
    let blobs = blob_runs_parallel(input);
    let t = Instant::now();
    for w in bounds.windows(2) {
        if recycle {
            lex_significant_range_into(input, w[0], w[1], &blobs, held);
            black_box(held.kinds.len());
        } else {
            let mut fresh = SignificantWorkspace::default();
            lex_significant_range_into(input, w[0], w[1], &blobs, &mut fresh);
            black_box(fresh.kinds.len());
        }
    }
    t.elapsed().as_secs_f64() * 1e3
}

/// Milliseconds to lex `input` as the ranges `bounds` names, each into a
/// workspace from a pool of `pool`, handing every finished one to a second
/// thread that reads it whole and hands it back: the pipeline's shape with the
/// device taken out of it.
///
/// The clock covers the lex calls and nothing else, so what it shows is the
/// lex slowed by the reader rather than the lex waiting for it. That is the
/// one thing neither neighbor arm reproduces: those touch their own buffers,
/// while this reader pulls lines the lexing cores have just written out of
/// their caches, which is what the device thread's copy of each range into its
/// window does.
///
/// Both sides run to the end of `bounds` and then stop, so neither channel
/// closes while the other still wants a workspace. A closed one is a workspace
/// lost, and this says so rather than lexing on with a shorter pool.
fn lex_handed_off(input: &[u8], bounds: &[usize], pool: usize) -> f64 {
    use std::sync::mpsc::{RecvError, TryRecvError, channel, sync_channel};

    let blobs = blob_runs_parallel(input);
    let (to_reader, from_lexer) = sync_channel::<SignificantWorkspace>(1);
    let (to_lexer, from_reader) = channel::<SignificantWorkspace>();
    std::thread::scope(|sc| {
        sc.spawn(move || {
            // Every kind and every span read, so the whole of what the lex
            // wrote is pulled across, as the device thread's window copy pulls
            // it.
            // `while let Ok(ws)` is the shorter spelling and discards the
            // error that ends the loop. The receive has exactly one failure
            // and it is the stop condition, so it is named in an arm of its
            // own rather than dropped.
            #[allow(clippy::while_let_loop)]
            loop {
                match from_lexer.recv() {
                    Ok(ws) => {
                        let mut sum = 0u64;
                        for &k in &ws.kinds {
                            sum += u64::from(k);
                        }
                        for &(a, b) in &ws.spans {
                            sum += u64::from(b - a);
                        }
                        black_box(sum);
                        to_lexer
                            .send(ws)
                            .expect("the lexer holds its receiver until the ranges are done");
                    }
                    // The lexer dropped its sender at the end of the ranges,
                    // which is this thread's stop.
                    Err(RecvError) => break,
                }
            }
        });
        let mut spare: Vec<SignificantWorkspace> =
            (0..pool).map(|_| SignificantWorkspace::default()).collect();
        let mut ms = 0.0;
        for w in bounds.windows(2) {
            let mut ws = match from_reader.try_recv() {
                Ok(ws) => ws,
                Err(TryRecvError::Empty) => match spare.pop() {
                    Some(ws) => ws,
                    None => from_reader.recv().expect("the reader returns every workspace it takes"),
                },
                Err(TryRecvError::Disconnected) => {
                    panic!("the reader stopped before the ranges were done")
                }
            };
            let t = Instant::now();
            lex_significant_range_into(input, w[0], w[1], &blobs, &mut ws);
            black_box(ws.kinds.len());
            ms += t.elapsed().as_secs_f64() * 1e3;
            to_reader.send(ws).expect("the reader holds its receiver until the ranges are done");
        }
        // The reader stops when this sender is gone, and the scope joins it
        // before dropping what the closure captured, so holding it here waits
        // for a thread that is waiting for it.
        drop(to_reader);
        ms
    })
}

fn main() {
    let mut cells: Vec<(String, Vec<u8>)> =
        TAG_PAIRS.iter().map(|&pairs| (format!("tag-lines-{pairs}"), tag_pairs(pairs))).collect();
    for path in std::env::args().skip(1) {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        };
        let stem = match Path::new(&path).file_stem() {
            Some(s) => s.to_string_lossy().into_owned(),
            None => {
                eprintln!("{path} names no file");
                std::process::exit(1);
            }
        };
        cells.push((stem, bytes));
    }

    // Arms 3 to 5 rerun arms 0 to 2: a rerun's ratio against its arm is the
    // reading whose answer is known.
    const ARMS: [Beside; 6] = [
        Beside::Nothing,
        Beside::Spinning,
        Beside::Streaming,
        Beside::Nothing,
        Beside::Spinning,
        Beside::Streaming,
    ];
    println!("=== the parallel lex beside a busy neighbor, median of {ROUNDS} rounds ===");
    println!(
        "{:>22} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10}",
        "cell", "bytes", "alone ms", "spinning ms", "streaming ms", "spin/alone", "stream/alone"
    );
    let mut ws = SignificantWorkspace::default();
    for (cell, input) in &cells {
        // A warm call first, so no cell touches its buffers for the first
        // time inside a timed round.
        lex_beside(input, &mut ws, Beside::Nothing);
        let mut alone = Vec::with_capacity(ROUNDS);
        let mut spinning = Vec::with_capacity(ROUNDS);
        let mut streaming = Vec::with_capacity(ROUNDS);
        let mut spin_ratio = Vec::with_capacity(ROUNDS);
        let mut stream_ratio = Vec::with_capacity(ROUNDS);
        let mut alone_control = Vec::with_capacity(ROUNDS);
        let mut spin_control = Vec::with_capacity(ROUNDS);
        let mut stream_control = Vec::with_capacity(ROUNDS);
        for round in 0..ROUNDS {
            let mut ms = [0.0f64; 6];
            for i in 0..ARMS.len() {
                let arm = (i + round) % ARMS.len();
                ms[arm] = lex_beside(input, &mut ws, ARMS[arm]);
            }
            alone.push(ms[0]);
            spinning.push(ms[1]);
            streaming.push(ms[2]);
            spin_ratio.push(ms[1] / ms[0]);
            stream_ratio.push(ms[2] / ms[0]);
            alone_control.push(ms[3] / ms[0]);
            spin_control.push(ms[4] / ms[1]);
            stream_control.push(ms[5] / ms[2]);
        }
        println!(
            "{cell:>22} {:>10} {:>10.3} {:>12.3} {:>12.3} {:>11.3}x {:>9.3}x   control alone {:.3}x spin {:.3}x stream {:.3}x",
            input.len(),
            median(&mut alone),
            median(&mut spinning),
            median(&mut streaming),
            median(&mut spin_ratio),
            median(&mut stream_ratio),
            median(&mut alone_control),
            median(&mut spin_control),
            median(&mut stream_control)
        );
    }
    println!(
        "\nspin/alone is what one held core costs the lex; stream/alone is what one core walking memory costs it. The pipeline's own lex reads 2.13x its solo time at 16.5 MB, so whatever these two do not account for is the device path's own."
    );

    println!(
        "\n=== the lex as {RANGES} ranges, one workspace kept, one taken per range, and one handed to a reader, median of {ROUNDS} rounds ==="
    );
    println!(
        "{:>22} {:>10} {:>10} {:>12} {:>10} {:>9} {:>15} {:>12} {:>13}",
        "cell", "bytes", "whole ms", "recycled ms", "fresh ms", "handed ms", "recycled/whole",
        "fresh/whole", "handed/whole"
    );
    for (cell, input) in &cells {
        let bounds = safe_boundaries(input, RANGES);
        if bounds.len() <= 2 {
            println!("{cell:>22} {:>10} splits into no ranges", input.len());
            continue;
        }
        lex_beside(input, &mut ws, Beside::Nothing);
        let mut whole = Vec::with_capacity(ROUNDS);
        let mut recycled = Vec::with_capacity(ROUNDS);
        let mut fresh = Vec::with_capacity(ROUNDS);
        let mut handed = Vec::with_capacity(ROUNDS);
        let mut recycled_ratio = Vec::with_capacity(ROUNDS);
        let mut fresh_ratio = Vec::with_capacity(ROUNDS);
        let mut handed_ratio = Vec::with_capacity(ROUNDS);
        let mut whole_control = Vec::with_capacity(ROUNDS);
        let mut recycled_control = Vec::with_capacity(ROUNDS);
        let mut fresh_control = Vec::with_capacity(ROUNDS);
        let mut handed_control = Vec::with_capacity(ROUNDS);
        // Arms 4 to 7 rerun arms 0 to 3. The handed arm takes the pipeline's
        // own pool of three.
        for round in 0..ROUNDS {
            let mut ms = [0.0f64; 8];
            for i in 0..8 {
                let arm = (i + round) % 8;
                ms[arm] = match arm % 4 {
                    1 => lex_ranges(input, &bounds, &mut ws, true),
                    2 => lex_ranges(input, &bounds, &mut ws, false),
                    3 => lex_handed_off(input, &bounds, 3),
                    _ => lex_beside(input, &mut ws, Beside::Nothing),
                };
            }
            whole.push(ms[0]);
            recycled.push(ms[1]);
            fresh.push(ms[2]);
            handed.push(ms[3]);
            recycled_ratio.push(ms[1] / ms[0]);
            fresh_ratio.push(ms[2] / ms[0]);
            handed_ratio.push(ms[3] / ms[0]);
            whole_control.push(ms[4] / ms[0]);
            recycled_control.push(ms[5] / ms[1]);
            fresh_control.push(ms[6] / ms[2]);
            handed_control.push(ms[7] / ms[3]);
        }
        println!(
            "{cell:>22} {:>10} {:>10.3} {:>12.3} {:>10.3} {:>9.3} {:>14.3}x {:>11.3}x {:>12.3}x   control whole {:.3}x recycled {:.3}x fresh {:.3}x handed {:.3}x",
            input.len(),
            median(&mut whole),
            median(&mut recycled),
            median(&mut fresh),
            median(&mut handed),
            median(&mut recycled_ratio),
            median(&mut fresh_ratio),
            median(&mut handed_ratio),
            median(&mut whole_control),
            median(&mut recycled_control),
            median(&mut fresh_control),
            median(&mut handed_control)
        );
    }
    println!(
        "\nrecycled/whole is what lexing in ranges costs with one workspace kept across them; fresh/whole is the same with a workspace taken per range, which is the cost the pipeline's pool of three exists to avoid; handed/whole is the pipeline's own shape, a pool of three with every finished range read by a second thread, and the reader is the one thing the neighbor arms above do not reproduce."
    );
}
