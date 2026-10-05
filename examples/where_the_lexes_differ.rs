//! Where the pool's lexer and the serial one differ, on a file where they do.
//!
//! `lex_parallel` is held to be token for token the serial `lex`: the chunks are
//! cut at `safe_boundaries`, where no string crosses and a whitespace run ends,
//! and the brackets that cross a cut are paired afterward. A file where the two
//! disagree has a cut in the wrong place or a token whose reading depends on
//! bytes on the far side of a cut. This says which, in order:
//!
//!   1. the quote scan the cuts are placed by, whole against in pieces - the
//!      same spans or not, and the first span apart;
//!   2. the first token where the two lexes differ, with the cut nearest it and
//!      the bytes around both.
//!
//! Every input is a real file somebody produced for another purpose.
//!
//! Run: `cargo run --release --example where_the_lexes_differ -- <file>...`

use std::path::Path;

use trex::token::Token;

fn short(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Up to `each` bytes either side of `at`, escaped so a newline or a quote
/// reads as itself.
fn around(input: &[u8], at: usize, each: usize) -> String {
    let lo = at.saturating_sub(each);
    let hi = (at + each).min(input.len());
    format!(
        "{:?} | {:?}",
        String::from_utf8_lossy(&input[lo..at.min(hi)]),
        String::from_utf8_lossy(&input[at.min(hi)..hi])
    )
}

fn shown(input: &[u8], t: Option<&Token>) -> String {
    match t {
        None => "none".to_string(),
        Some(t) => format!(
            "{}..{} {:?} {:?}",
            t.start(),
            t.end(),
            t.kind,
            String::from_utf8_lossy(&input[t.start()..t.end().min(t.start() + 60)])
        ),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: where_the_lexes_differ <file>...");
        std::process::exit(2);
    }
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    // An input that cannot be read is a row missing from the report, which
    // reads as a file that agreed. So a run with one exits failing.
    let mut unreadable = 0usize;
    for arg in &args {
        let path = Path::new(arg);
        let name = short(path);
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{name}: unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };
        println!("=== {name}, {} bytes", bytes.len());

        let whole = trex::parallel_lex::quoted_spans(&bytes);
        let pieced = trex::parallel_lex::quoted_spans_across(&bytes, trex::parallel_lex::QUOTE_SCAN_MIN_LEAF);
        // The longest string, since a cut is refused anywhere inside one: a
        // string spanning much of the input leaves the divisions it covers to
        // search their whole windows and find nothing.
        let longest = whole.iter().max_by_key(|&&(open, close)| close - open);
        if let Some(&(open, close)) = longest {
            println!(
                "  the longest string: {open}..{close}, {} bytes, {:.1}% of the input",
                close - open + 1,
                100.0 * (close - open + 1) as f64 / bytes.len().max(1) as f64
            );
        }
        match whole.iter().zip(&pieced).position(|(a, b)| a != b) {
            None if whole.len() == pieced.len() => {
                println!("  quote spans: {} whole, the same in pieces", whole.len());
            }
            apart => {
                let k = apart.unwrap_or(whole.len().min(pieced.len()));
                println!(
                    "  quote spans APART at span {k} of {} whole / {} in pieces: whole {:?}, in pieces {:?}",
                    whole.len(),
                    pieced.len(),
                    whole.get(k),
                    pieced.get(k)
                );
                if let Some(&(open, _)) = whole.get(k).or(pieced.get(k)) {
                    println!("    at {open}: {}", around(&bytes, open, 60));
                }
            }
        }

        // How much of the input the strings hold, by their length: a cut is
        // refused anywhere inside one, so this is where no division can cut.
        let mut by_length = [0usize; 4];
        let mut held_bytes = 0usize;
        for &(open, close) in &whole {
            let len = close - open + 1;
            held_bytes += len;
            by_length[match len {
                0..=1_023 => 0,
                1_024..=10_239 => 1,
                10_240..=102_399 => 2,
                _ => 3,
            }] += 1;
        }
        println!(
            "  strings under 1 kB {}, 1-10 kB {}, 10-100 kB {}, over 100 kB {}; {:.1}% of the input inside one",
            by_length[0],
            by_length[1],
            by_length[2],
            by_length[3],
            100.0 * held_bytes as f64 / bytes.len().max(1) as f64
        );
        // The first strings that run past the end of the line they open on,
        // with the bytes where each opens. A string in most source closes on
        // its line, so the first of these is where the reading of the quotes
        // may have fallen out of step with what the file's author wrote.
        let over_a_line: Vec<&(usize, usize)> =
            whole.iter().filter(|&&(open, close)| bytes[open..close].contains(&b'\n')).take(5).collect();
        let crossing = whole.iter().filter(|&&(open, close)| bytes[open..close].contains(&b'\n')).count();
        println!("  strings running past their line: {crossing}");
        for &&(open, close) in &over_a_line {
            println!("    {open}..{close}: {}", around(&bytes, open, 50));
        }
        let asked = trex::parallel_lex::chunks_for(bytes.len(), cores);
        trex::trace::record();
        drop(trex::trace::take_counts());
        let t_bounds = std::time::Instant::now();
        let bounds = trex::parallel_lex::safe_boundaries(&bytes, asked);
        let bounds_ms = t_bounds.elapsed().as_secs_f64() * 1e3;
        let counts = trex::trace::take_counts();
        trex::trace::stop();
        for (name, n) in counts.iter().filter(|(name, _)| name.starts_with("the boundaries")) {
            println!("  {name}: {n}");
        }
        let t_quotes = std::time::Instant::now();
        let quotes_again = trex::parallel_lex::quoted_spans(&bytes).len();
        let quotes_ms = t_quotes.elapsed().as_secs_f64() * 1e3;
        println!(
            "  cuts: {} of {} asked for, the boundaries in {bounds_ms:.3} ms of which the quote scan \
             read alone takes {quotes_ms:.3} ms ({quotes_again} spans)",
            bounds.len().saturating_sub(2),
            asked - 1
        );
        let serial = trex::lexer::lex(&bytes);
        let pooled = trex::parallel_lex::lex_parallel(&bytes);
        let same = |a: &Token, b: &Token| a.start == b.start && a.end == b.end && a.kind == b.kind;
        let apart = serial
            .iter()
            .zip(&pooled)
            .position(|(a, b)| !same(a, b))
            .or_else(|| (serial.len() != pooled.len()).then(|| serial.len().min(pooled.len())));
        match apart {
            None => println!(
                "  tokens: {} serial, the same pooled, over {} cuts",
                serial.len(),
                bounds.len().saturating_sub(2)
            ),
            Some(k) => {
                let at = serial.get(k).or(pooled.get(k)).map_or(bytes.len(), Token::start);
                let cut = bounds.partition_point(|&b| b <= at);
                let (before, after) = (bounds[cut.saturating_sub(1)], bounds.get(cut).copied().unwrap_or(bytes.len()));
                println!("  tokens APART at token {k} of {} serial / {} pooled", serial.len(), pooled.len());
                println!("    serial {}", shown(&bytes, serial.get(k)));
                println!("    pooled {}", shown(&bytes, pooled.get(k)));
                println!("    the chunk holding it runs {before}..{after}");
                println!("    at the token: {}", around(&bytes, at, 60));
                println!("    at the cut before it, {before}: {}", around(&bytes, before, 60));
            }
        }
    }
    if unreadable > 0 {
        eprintln!("{unreadable} of {} inputs could not be read", args.len());
        std::process::exit(1);
    }
}
