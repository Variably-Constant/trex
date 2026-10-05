//! What a scan costs per token either side of the threshold that sends it wide.
//!
//! The per-start sweep runs on one thread under a token count and across cores
//! over it, and which count applies depends on whether this process has already
//! dispatched: `PARALLEL_SCAN_THRESHOLD_COLD` while the pool's spawn is still
//! ahead, `PARALLEL_SCAN_THRESHOLD_WARM` once it has happened. Both bounds carry
//! the readings they were set from.
//!
//! This reads the cost per token across a ladder of sizes on a warm pool, which
//! is the case a long-lived process spends almost all of its time in. Its first
//! pass warms the pool, so the counted pass is warm at every size.
//!
//! The cold case needs one process per reading, since the first scan is what
//! makes a process warm: `--cold <file> <bytes>` scans once and exits, and the
//! reading is the median of nine such runs.
//!
//! Neither case can be read against a bound the engine is not using. Moving one
//! for a measurement means editing the constant, measuring, and putting it
//! back - which is how the readings in both bounds' docs were taken.
//!
//! Run: `cargo run --release --example where_the_parallel_scan_wins -- <file>`

/// Byte lengths chosen to bracket the threshold, since tokens are what the
/// threshold counts and bytes are what a file is cut at. The token count each
/// one yields is reported beside it rather than assumed.
const BYTES: [usize; 16] = [
    512, 1_024, 2_048, 3_072, 4_096, 8_192, 12_288, 16_384, 20_480, 24_576, 28_672, 32_768, 40_960,
    49_152, 65_536, 131_072,
];

/// Scans a size, past one that is not counted.
const REPEATS: u32 = 20;

/// Timings of each size, reduced to the middle one.
///
/// This box is shared and a clock on it says so - two identical configurations
/// of another scan here timed 161.338 ms and 83.775 ms in one run. A step at a
/// token count is a shape rather than a level and survives more drift than an
/// absolute would, but one reading of it is still one reading. The rounds are
/// what make the step readable without a sentence explaining it away.
const ROUNDS: usize = 5;

/// The middle of `v`, which is what a size's rounds reduce to.
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // One scan of one size in a process that has done nothing else, which is
    // the case the threshold exists for: the pool spawns on the first dispatch,
    // and a scan that only ever happens once spends that spawn out of its own
    // time. It cannot be read in the same process as a warm one, because the
    // first scan is what makes the process warm.
    let cold = args.first().is_some_and(|a| a == "--cold");
    if cold {
        args.remove(0);
    }
    let amortize = args.first().is_some_and(|a| a == "--amortize");
    if amortize {
        args.remove(0);
    }
    let path = args.first().cloned().unwrap_or_else(|| {
        eprintln!("usage: where_the_parallel_scan_wins [--cold] <file> [bytes]");
        eprintln!("a real file - a corpus written for this measures the writer");
        std::process::exit(2);
    });
    let whole = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(1);
    });
    // A word then a balanced group: a shape whose sweep is most of its scan, so
    // what the sweep's dispatch does shows through rather than being hidden by
    // a field's cost.
    let pattern = trex::parse("\\W \\B").expect("the shape parses");

    // Many scans of one small input in a single process. No one of them is
    // large enough to spawn the pool, so the spawn is earned by what they sweep
    // between them, and the step where that happens is what this prints.
    if amortize {
        let n = match args.get(1) {
            None => {
                eprintln!("--amortize takes a file, then a byte count, then a scan count");
                std::process::exit(2);
            }
            Some(s) => match s.parse::<usize>() {
                Ok(v) => v.min(whole.len()),
                Err(e) => {
                    eprintln!("the byte count {s:?} is not a number: {e}");
                    std::process::exit(2);
                }
            },
        };
        let scans = match args.get(2) {
            None => 400usize,
            Some(s) => match s.parse::<usize>() {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("the scan count {s:?} is not a number: {e}");
                    std::process::exit(2);
                }
            },
        };
        let input = &whole[..n];
        let tokens = trex::lexer::lex(input).len();
        println!("trex {}", env!("CARGO_PKG_VERSION"));
        println!("{path}: {n} bytes, {tokens} tokens per scan, {scans} scans in one process");
        println!("{:>7} {:>14} {:>12}", "scan", "tokens swept", "ms");
        for i in 0..scans {
            let from = std::time::Instant::now();
            drop(trex::scan(&pattern, input));
            let ms = from.elapsed().as_secs_f64() * 1e3;
            // A tenth of them printed: the step is a few scans wide, and a
            // summary either side would hide the scan where it happened.
            if i % 10 == 0 {
                println!("{i:>7} {:>14} {ms:>12.4}", (i + 1) * tokens);
            }
        }
        return;
    }

    if cold {
        // Refused rather than defaulted: a cold reading taken at a size the
        // caller did not ask for is labeled with the size they did.
        let n = match args.get(1) {
            None => {
                eprintln!("--cold takes a byte count after the file");
                std::process::exit(2);
            }
            Some(s) => match s.parse::<usize>() {
                Ok(v) => v.min(whole.len()),
                Err(e) => {
                    eprintln!("--cold's byte count {s:?} is not a number: {e}");
                    std::process::exit(2);
                }
            },
        };
        let input = &whole[..n];
        // Lexed before the clock starts: the threshold decides how the sweep is
        // dispatched, and a lex is the same work either side of it.
        let tokens = trex::lexer::lex(input).len();
        let from = std::time::Instant::now();
        let found = trex::scan(&pattern, input).len();
        let ms = from.elapsed().as_secs_f64() * 1e3;
        println!("cold {n} {tokens} {ms:.4} {found}");
        return;
    }

    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("{path}: {} bytes, the threshold is 4096 tokens\n", whole.len());
    // No column names the path a row took, because a caller cannot know it.
    // The sweep picks its width from whether this process has already
    // dispatched across cores, which is state inside the engine; the token
    // count alone does not determine it anywhere between the warm bound and
    // the cold one.
    println!(
        "{:>9} {:>9} {:>12} {:>10} {:>10} {:>10}",
        "bytes", "tokens", "ms a scan", "ns a token", "low", "high"
    );

    // The pool spawns once and stays up, so every reading below is a warm one.
    // The cold case is a different regime and one process can only have it once,
    // which is the whole reason the threshold is where it is.
    for &n in &BYTES {
        if n > whole.len() {
            continue;
        }
        drop(trex::scan(&pattern, &whole[..n]));
    }

    for &n in &BYTES {
        if n > whole.len() {
            println!("{n:>9}  past the end of this file");
            continue;
        }
        let input = &whole[..n];
        let tokens = trex::lexer::lex(input).len();
        drop(trex::scan(&pattern, input));
        let mut got = Vec::with_capacity(ROUNDS);
        for _ in 0..ROUNDS {
            let from = std::time::Instant::now();
            for _ in 0..REPEATS {
                drop(trex::scan(&pattern, input));
            }
            got.push(from.elapsed().as_secs_f64() * 1e3 / f64::from(REPEATS));
        }
        let lo = got.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = got.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let ms = median(got);
        println!(
            "{n:>9} {tokens:>9} {ms:>12.4} {:>10.1} {lo:>10.4} {hi:>10.4}",
            ms * 1e6 / tokens.max(1) as f64
        );
    }
}
