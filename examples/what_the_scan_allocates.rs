//! What a scan asks the allocator for, counted rather than read off the source.
//!
//! A sampled profile puts about a tenth of a scan inside `RtlAllocateHeap` -
//! more than any function of this crate, and more than `advance`'s own body -
//! but a profile names the allocator, not the caller. These counts say what a
//! scan requests from it: how many allocations a scan makes, how many of those
//! are a vector growing rather than a fresh request, how many bytes in total,
//! and the size each one asks for.
//!
//! The size distribution is the part that decides what to do next. Many small
//! allocations point at something built per attempt or per token, where an
//! arena or a reused buffer wins; a few large ones point at a vector whose
//! capacity is wrong, where a reservation wins. The two have different fixes
//! and the profile alone cannot tell them apart.
//!
//! Counts, not times: an allocation count is the same on a busy box as a quiet
//! one, so this needs no lease, no control arm and no alternating rounds.
//!
//! Run: `cargo run --release --example what_the_scan_allocates -- <file>`

// A global allocator has to be an `unsafe impl`, and forwarding each call to
// the system allocator is the only unsafe here: every counter is atomic and
// nothing else is touched.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Capture the caller of one allocation in this many, or never at zero.
///
/// A capture costs far more than the allocation it describes, so it is sampled.
/// Which allocations get sampled is decided by a plain counter rather than by
/// size or by shape: sampling on a property of the allocation would return the
/// sites that have that property, which is the question rather than the answer.
static SAMPLE_EVERY: AtomicU64 = AtomicU64::new(0);
/// How many distinct frames of this crate a captured site is named by, which
/// `--chain` sets and this is the default of.
///
/// One frame names the function an allocation is in. Three carries two callers
/// past it, which is what an inlined match arm needs to be told from its
/// siblings: every `vec![..]` inside `advance` reports as `advance`, and what
/// differs between them is who asked `advance` to run. Whether a longer chain
/// divides the rows further or only widens them is a question for the flag
/// rather than for this line.
const CHAIN_FRAMES: usize = 3;
static CHAIN: AtomicUsize = AtomicUsize::new(CHAIN_FRAMES);
/// Which shape is being scanned while a site is captured, as an index into
/// `SHAPES`, or `usize::MAX` outside any of them.
///
/// A site alone cannot say which pattern reached it. The arms of `advance` are
/// inlined into `advance` and every shape's outermost call carries the same
/// chain, so a row pools whatever each shape did there - and one shape can be
/// half of every allocation a run makes without ever appearing under its own
/// name. The shape is recorded beside the chain so a row names both what ran
/// and what asked for it.
static SHAPE: AtomicUsize = AtomicUsize::new(usize::MAX);
static SEEN: AtomicU64 = AtomicU64::new(0);
static SITES: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Times the tally's lock was found poisoned, which means a thread panicked
/// holding it and the sites below are missing whatever it had not yet pushed.
static POISONED: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Set while this thread is capturing a backtrace.
    ///
    /// Capturing allocates and formatting it allocates more. Without this the
    /// allocator would recurse into itself forever; with it, those allocations
    /// go to the system allocator uncounted, so a sampled allocation's own
    /// bookkeeping never appears in the tally it is building.
    static CAPTURING: Cell<bool> = const { Cell::new(false) };
}

/// Record where this allocation came from, if it is one of the sampled ones.
fn note_site(size: usize) {
    let every = SAMPLE_EVERY.load(Ordering::Relaxed);
    if every == 0 {
        return;
    }
    if !SEEN.fetch_add(1, Ordering::Relaxed).is_multiple_of(every) {
        return;
    }
    CAPTURING.with(|c| {
        if c.get() {
            return;
        }
        c.set(true);
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        // The innermost frames belonging to this crate, past the harness's own,
        // innermost first and separated by `<` so the row reads as a chain from
        // the allocation outward.
        //
        // One frame alone cannot separate the arms of a match that the compiler
        // inlined into the function holding it: every `vec![..]` inside
        // `advance` reports as `advance` whichever arm built it. The caller two
        // or three frames out is what differs between them, so a row carries
        // that far and a site is named by its callers rather than only by the
        // function it is in.
        let frames = trace
            .lines()
            .filter_map(|l| l.split_once("trex::"))
            .map(|(_, rest)| rest.split([' ', '<', '(']).next().unwrap_or(rest).to_string())
            .filter(|f| !f.starts_with("what_the_scan_allocates"));
        let mut chain: Vec<String> = Vec::new();
        for frame in frames {
            // A recursive function repeats for as long as the recursion is deep,
            // and a row of one name repeated says nothing the first name did
            // not. Consecutive repeats collapse so the depth a chain reaches is
            // measured in distinct callers.
            if chain.last() == Some(&frame) {
                continue;
            }
            chain.push(frame);
            if chain.len() == CHAIN.load(Ordering::Relaxed) {
                break;
            }
        }
        let named = if chain.is_empty() {
            "outside this crate".to_string()
        } else {
            chain.join(" < ")
        };
        let whose = match SHAPES.get(SHAPE.load(Ordering::Relaxed)) {
            Some((label, _)) => *label,
            None => "outside a shape",
        };
        let row = format!("{whose}  |  {named}  <{}B", (size + 1).next_power_of_two());
        match SITES.lock() {
            Ok(mut sites) => sites.push(row),
            Err(poisoned) => {
                // A thread panicked while holding the tally. The data behind it
                // is still intact, so it is taken and the event counted and
                // reported, rather than this sample vanishing silently.
                POISONED.fetch_add(1, Ordering::Relaxed);
                poisoned.into_inner().push(row);
            }
        }
        c.set(false);
    });
}

static FRESH: AtomicU64 = AtomicU64::new(0);
static GROWN: AtomicU64 = AtomicU64::new(0);
static FREED: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

/// Bytes asked for and not yet given back, and the most that has ever been
/// outstanding at once.
///
/// `BYTES` sums every request over a whole scan, which is throughput and not
/// footprint: a buffer reused a million times sums to a million requests and
/// occupies one. Only the high-water mark answers what a change costs in
/// memory, which is the question a reuse or a retained capacity raises and the
/// summed column cannot speak to.
///
/// Signed, because a free of something allocated before the count was zeroed
/// takes it below zero, and a saturating count would hide that rather than show
/// it. The peak is only ever raised, so a transient dip does not disturb it.
static LIVE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

/// Record `delta` bytes coming or going, and raise the high-water mark.
fn live_by(delta: i64) {
    let now = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
    if now > 0 {
        PEAK.fetch_max(now as u64, Ordering::Relaxed);
    }
}

/// Requests by size, one bucket a power of two: bucket `k` counts a request of
/// at least `2^(k-1)` and under `2^k`, and the last holds everything above.
const CLASSES: usize = 24;
static BY_SIZE: [AtomicU64; CLASSES] = [const { AtomicU64::new(0) }; CLASSES];

/// The system allocator with a tally in front of it.
///
/// `realloc` forwards rather than falling back to the default of allocate, copy
/// and free: that default would turn every vector growth into a fresh
/// allocation and make this harness disagree with the build it is measuring.
/// It is counted on its own line instead, because a growth and a first request
/// have different fixes.
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        FRESH.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        BY_SIZE[class_of(layout.size())].fetch_add(1, Ordering::Relaxed);
        live_by(layout.size() as i64);
        note_site(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        FRESH.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        BY_SIZE[class_of(layout.size())].fetch_add(1, Ordering::Relaxed);
        live_by(layout.size() as i64);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        GROWN.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size.saturating_sub(layout.size()) as u64, Ordering::Relaxed);
        BY_SIZE[class_of(new_size)].fetch_add(1, Ordering::Relaxed);
        live_by(new_size as i64 - layout.size() as i64);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        FREED.fetch_add(1, Ordering::Relaxed);
        live_by(-(layout.size() as i64));
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The power-of-two bucket a request of `n` bytes belongs to.
fn class_of(n: usize) -> usize {
    let bits = usize::BITS - n.leading_zeros();
    (bits as usize).min(CLASSES - 1)
}

/// The tally so far, and zero it for the next reading.
///
/// The peak restarts from what is outstanding now rather than from zero, so a
/// shape's figure is the most it held at once and not the most the process ever
/// held. `LIVE` itself is never reset: it is a running balance, and zeroing it
/// would make every later peak read as though the heap were empty.
fn take() -> (u64, u64, u64, u64, u64, [u64; CLASSES]) {
    let mut sizes = [0u64; CLASSES];
    for (k, slot) in BY_SIZE.iter().enumerate() {
        sizes[k] = slot.swap(0, Ordering::Relaxed);
    }
    let held = LIVE.load(Ordering::Relaxed).max(0) as u64;
    let peak = PEAK.swap(held, Ordering::Relaxed);
    (
        FRESH.swap(0, Ordering::Relaxed),
        GROWN.swap(0, Ordering::Relaxed),
        FREED.swap(0, Ordering::Relaxed),
        BYTES.swap(0, Ordering::Relaxed),
        peak,
        sizes,
    )
}

/// Shapes the set engine's sweep answers, which is where the allocation is, and
/// two a route answers as a floor for what a scan costs before the sweep runs.
const SHAPES: [(&str, &str); 8] = [
    ("a kind then a literal", "\\W \"=\""),
    ("a literal alone", "\"let\""),
    ("a word then a balanced group", "\\W \\B"),
    ("an atomic group", "(?>\\W*) \"=\""),
    ("a negative assertion", "\\W !~(\"=\" \\Q)"),
    ("a sub-pattern assertion", "\\W ~(\"=\" \\N)"),
    ("an edit-distance group", "(\"let\" \\W \"=\")~1"),
    ("a seam anchor", "@seam \\W"),
];

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `--sites N` captures the caller of one allocation in N and reports the
    // sites instead of the per-shape table. Stripped before the path is read,
    // because a flag left in place becomes the filename.
    let mut sample = 0u64;
    if let Some(at) = args.iter().position(|a| a == "--sites") {
        args.remove(at);
        let given = if at < args.len() { args.remove(at) } else { "20000".to_string() };
        match given.parse::<u64>() {
            Ok(n) if n > 0 => sample = n,
            Ok(_) => {
                eprintln!("--sites takes how many allocations to one capture, and zero is none");
                std::process::exit(2);
            }
            Err(e) => {
                eprintln!("--sites takes a count: {given:?} is not one ({e})");
                std::process::exit(2);
            }
        }
    }
    // `--chain N` names a captured site by N frames of this crate rather than
    // by the default. Stripped for the same reason `--sites` is.
    if let Some(at) = args.iter().position(|a| a == "--chain") {
        args.remove(at);
        let given =
            if at < args.len() { args.remove(at) } else { CHAIN_FRAMES.to_string() };
        match given.parse::<usize>() {
            Ok(n) if n > 0 => CHAIN.store(n, Ordering::Relaxed),
            Ok(_) => {
                eprintln!("--chain takes how many frames name a site, and zero names none");
                std::process::exit(2);
            }
            Err(e) => {
                eprintln!("--chain takes a count: {given:?} is not one ({e})");
                std::process::exit(2);
            }
        }
    }
    let path = match args.first() {
        Some(p) => p.clone(),
        None => {
            eprintln!("name a corpus file: an allocation count is a fact about the bytes scanned");
            std::process::exit(2);
        }
    };
    let input = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };

    let mut shapes = Vec::new();
    for (label, src) in SHAPES {
        match trex::parse(src) {
            Ok(p) => shapes.push((label, p)),
            Err(e) => {
                eprintln!("{label}: {src} does not parse: {e:?}");
                std::process::exit(2);
            }
        }
    }

    let tokens = trex::lexer::lex(&input).len();
    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("corpus {path}, {} bytes, {tokens} tokens\n", input.len());

    // One scan before any is counted, so a lazily built table or a pool spun up
    // on first use is not charged to the first shape that happened to run. What
    // that scan asked for is printed rather than dropped, because it is the
    // once-per-process part and a reader comparing two runs needs to know it
    // was taken out.
    drop(trex::scan(&shapes[0].1, &input));
    let (warm_fresh, warm_grown, _, warm_bytes, _, _) = take();
    println!(
        "the first scan, which is not counted below: {warm_fresh} fresh, {warm_grown} grown, {warm_bytes} bytes\n"
    );

    println!(
        "{:>30} {:>12} {:>10} {:>12} {:>9} {:>12}",
        "shape", "allocations", "of those", "bytes asked", "per token", "peak held"
    );
    println!("{:>30} {:>12} {:>10} {:>12} {:>9} {:>12}", "", "fresh", "grown", "summed", "", "at once");
    for (label, pat) in &shapes {
        drop(trex::scan(pat, &input));
        let (fresh, grown, _freed, bytes, peak, sizes) = take();
        let per_token = if tokens == 0 { 0.0 } else { (fresh + grown) as f64 / tokens as f64 };
        println!(
            "{label:>30} {fresh:>12} {grown:>10} {bytes:>12} {per_token:>9.3} {peak:>12}"
        );
        // The distribution, printed only where it is not one bucket, since the
        // question it answers is whether the requests are many and small or few
        // and large.
        let loud: Vec<String> = sizes
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(k, n)| format!("<{}B:{n}", 1usize << k))
            .collect();
        if !loud.is_empty() {
            println!("{:>30}   {}", "", loud.join(" "));
        }
    }

    if sample == 0 {
        return;
    }
    // The sites, each recorded against the shape being scanned when it was
    // captured. Dividing them this way makes each row thinner, and `--sites`
    // makes up for it: a denser sample restores the resolution the
    // division costs. Pooling them instead hides a shape that allocates more
    // than all the others together, because its arm is inlined into `advance`
    // and its chain is the chain every shape's outermost call has.
    SAMPLE_EVERY.store(sample, Ordering::Relaxed);
    for (i, (_, pat)) in shapes.iter().enumerate() {
        SHAPE.store(i, Ordering::Relaxed);
        drop(trex::scan(pat, &input));
    }
    SHAPE.store(usize::MAX, Ordering::Relaxed);
    SAMPLE_EVERY.store(0, Ordering::Relaxed);

    let taken = match SITES.lock() {
        Ok(sites) => sites.clone(),
        Err(poisoned) => {
            println!("the tally's lock was poisoned, so these sites are incomplete");
            poisoned.into_inner().clone()
        }
    };
    let lost = POISONED.load(Ordering::Relaxed);
    println!("\none allocation in {sample} captured, {} taken, {lost} lost to a poisoned lock", taken.len());
    let mut tally: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for row in taken {
        *tally.entry(row).or_insert(0) += 1;
    }
    let mut ranked: Vec<(String, u64)> = tally.into_iter().collect();
    ranked.sort_by_key(|row| std::cmp::Reverse(row.1));
    println!("{:>9}  site and the size it asked for", "captures");
    let shown = 24;
    for (site, n) in ranked.iter().take(shown) {
        println!("{n:>9}  {site}");
    }
    // What the printed rows leave out. Naming a site by a chain of callers
    // divides one row into several, so a list that covered every capture when a
    // site was one function need not cover them now; a tail reported as a
    // number cannot be mistaken for a tail that is not there.
    if ranked.len() > shown {
        let rest: u64 = ranked.iter().skip(shown).map(|(_, n)| n).sum();
        println!("{rest:>9}  in {} further sites, unprinted", ranked.len() - shown);
    }
}
