//! Where splitting a byte count across the cores starts paying, by input size.
//!
//! The lines ahead of a window are counted with `count_byte`, one pass of the
//! vector count over the text before the window: a numbered tail counts every
//! newline ahead of its last lines, which for a large file is nearly all of
//! it. The count is embarrassingly parallel - a leaf's count depends on its
//! own bytes and nothing else - and `count_byte_across` splits it, but a split
//! has a cost of its own, and below some size it is the slower form.
//!
//! This times the two forms over sizes from 4 KiB to 256 MiB, of the corpus
//! the other crossover benches build and, where a path is given, of the
//! opening bytes of a real file at each size it holds. Two bytes are counted:
//! the newline, which a line holds one of, and NUL, which text never holds, so
//! a crossover that moves with how often the byte occurs shows as two.
//!
//! Run: `cargo bench --bench count_byte_crossover [-- FILE...]`.

use std::hint::black_box;
use std::time::Instant;

/// The corpus `benches/vs_regex_full.rs` builds, cut to `bytes`.
fn corpus(bytes: usize) -> Vec<u8> {
    let mut s = String::new();
    let mut i = 0usize;
    while s.len() < bytes {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
        i += 1;
    }
    let mut v = s.into_bytes();
    v.truncate(bytes);
    v
}

const ROUNDS: usize = 7;
const BUDGET_MS: f64 = 20.0;
const SIZES: [usize; 9] = [
    4 * 1024,
    16 * 1024,
    64 * 1024,
    256 * 1024,
    1024 * 1024,
    4 * 1024 * 1024,
    16 * 1024 * 1024,
    64 * 1024 * 1024,
    256 * 1024 * 1024,
];

fn time_budget(f: &mut dyn FnMut() -> usize) -> f64 {
    black_box(f());
    let t0 = Instant::now();
    let mut iters = 0u32;
    loop {
        black_box(f());
        iters += 1;
        if iters >= 5 && t0.elapsed().as_secs_f64() * 1e3 >= BUDGET_MS {
            break;
        }
    }
    t0.elapsed().as_secs_f64() * 1e3 / f64::from(iters)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// One input, the two forms timed in an order that rotates so neither reads
/// its position as its own cost, and the first read once more after both as
/// the control: the distance between the two readings of one form is the box
/// moving under the cell.
fn cell(input: &[u8], byte: u8) -> (f64, f64, usize, f64) {
    let mut one = || trex::byte_simd::count_byte(input, byte);
    let mut across = || trex::byte_simd::count_byte_across(input, byte);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for r in 0..ROUNDS {
        if r % 2 == 0 {
            a.push(time_budget(&mut one));
            b.push(time_budget(&mut across));
        } else {
            b.push(time_budget(&mut across));
            a.push(time_budget(&mut one));
        }
    }
    let control = time_budget(&mut one);
    let n = trex::byte_simd::count_byte(input, byte);
    assert_eq!(trex::byte_simd::count_byte_across(input, byte), n, "the two forms disagree");
    assert_eq!(trex::byte_simd::count_byte_scalar(input, byte), n, "the scalar baseline disagrees with both");
    let first = median(a);
    (first, median(b), n, control / first)
}

fn row(source: &str, input: &[u8]) {
    for (name, byte) in [("newline", b'\n'), ("nul", 0u8)] {
        let (one, across, found, drift) = cell(input, byte);
        let gbps = |ms: f64| input.len() as f64 / (ms * 1e6);
        println!(
            "{source:>24} {name:>8} {:>11} {one:>11.4} {across:>11.4} {:>8.2}x {:>8.2} {:>8.2} {found:>10} {drift:>8.3}x",
            input.len(),
            one / across,
            gbps(one),
            gbps(across)
        );
    }
}

fn main() {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    println!("trex {}, {cores} cores", trex::version());
    println!(
        "{:>24} {:>8} {:>11} {:>11} {:>11} {:>9} {:>8} {:>8} {:>10} {:>9}",
        "source", "byte", "bytes", "one ms", "across ms", "ratio", "one GB/s", "acr GB/s", "found", "control"
    );
    for bytes in SIZES {
        row("corpus", &corpus(bytes));
    }
    for path in std::env::args().skip(1).filter(|a| !a.starts_with('-')) {
        let whole = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("count_byte_crossover: cannot read {path}: {e}");
                std::process::exit(2);
            }
        };
        let name = std::path::Path::new(&path)
            .file_name()
            .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        for bytes in SIZES.iter().copied().filter(|&b| b <= whole.len()) {
            row(&name, &whole[..bytes]);
        }
        row(&name, &whole);
    }
    println!("\nthe crossover is the smallest size whose ratio exceeds 1.00 in every source and for both bytes");
}
