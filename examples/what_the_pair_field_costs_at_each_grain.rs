//! What the pair field costs to learn and read at each grain, against the
//! parallel lex of the same bytes, and what `--record seam`, `bind` and `auto`
//! cost to cut, in one process with the arms rotated.
//!
//! `Readings::read` learns the field from the input at the grain asked and
//! reads strain, binding and geometry off it, over gaps to the grain's reach:
//! O(units x reach) counts, then the readings. The lex it reads over is taken
//! once outside the clock, so what each grain's arm times is the field alone.
//! `--record bind` learns the token field and the seam both, and `auto` cuts
//! both ways and judges, so each is timed whole beside `seam`.
//!
//! Four rounds, the first discarded as the warm-up, the arms rotated so no arm
//! holds a fixed position in a round; the median of the three kept is printed
//! with its ratio to the parallel lex's median. A ratio above 1 is an arm
//! dearer than the lex of the same bytes.
//!
//! Each file is read as `trex scan` reads it (`trex::encoding::decode`).
//!
//! Run: `cargo run --release --example what_the_pair_field_costs_at_each_grain -- <file>...`

use std::time::Instant;

use trex::ast::Grain;
use trex::gravity::Readings;
use trex::records::RecordUnit;
use trex::token::Token;

/// Rounds timed, the first of them discarded.
const ROUNDS: usize = 4;

/// One timed arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arm {
    LexPar,
    ByteField,
    TokenField,
    UnitField,
    RecordSeam,
    RecordBind,
    RecordAuto,
}

impl Arm {
    const ALL: [Arm; 7] = [
        Arm::LexPar,
        Arm::ByteField,
        Arm::TokenField,
        Arm::UnitField,
        Arm::RecordSeam,
        Arm::RecordBind,
        Arm::RecordAuto,
    ];

    fn name(self) -> &'static str {
        match self {
            Arm::LexPar => "lex par",
            Arm::ByteField => "byte field",
            Arm::TokenField => "token field",
            Arm::UnitField => "unit field",
            Arm::RecordSeam => "record seam",
            Arm::RecordBind => "record bind",
            Arm::RecordAuto => "record auto",
        }
    }

    /// Runs the arm once over `bytes`, the field arms reading over `toks`, and
    /// returns how many things it produced with the milliseconds it took.
    fn run(self, bytes: &[u8], toks: &[Token]) -> (usize, f64) {
        match self {
            Arm::LexPar => {
                let (t, ms) = timed(|| trex::parallel_lex::lex_parallel(bytes));
                (t.len(), ms)
            }
            Arm::ByteField => {
                let (r, ms) = timed(|| Readings::read(Grain::Byte, bytes, toks));
                (r.len(), ms)
            }
            Arm::TokenField => {
                let (r, ms) = timed(|| Readings::read(Grain::Token, bytes, toks));
                (r.len(), ms)
            }
            Arm::UnitField => {
                let (r, ms) = timed(|| Readings::read(Grain::Super, bytes, toks));
                (r.len(), ms)
            }
            Arm::RecordSeam => {
                let (r, ms) = timed(|| RecordUnit::Seam.records(bytes));
                (r.len(), ms)
            }
            Arm::RecordBind => {
                let (r, ms) = timed(|| RecordUnit::Bind(None).records(bytes));
                (r.len(), ms)
            }
            Arm::RecordAuto => {
                let (r, ms) = timed(|| RecordUnit::Auto.records(bytes));
                (r.len(), ms)
            }
        }
    }
}

/// Milliseconds one call takes, with what it returned.
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let started = Instant::now();
    let out = f();
    (out, started.elapsed().as_secs_f64() * 1000.0)
}

/// The median of the readings kept, the first round discarded.
fn median_kept(readings: &[f64]) -> f64 {
    let mut kept: Vec<f64> = readings[1..].to_vec();
    kept.sort_by(f64::total_cmp);
    kept[kept.len() / 2]
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: what_the_pair_field_costs_at_each_grain <file>...");
        std::process::exit(2);
    }
    println!("trex {}, {} rounds, the first discarded, arms rotated a round", env!("CARGO_PKG_VERSION"), ROUNDS);
    let mut unreadable = 0usize;
    for path in &paths {
        let raw = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("\n{path}: unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };
        let bytes = trex::encoding::decode(raw);
        let toks = trex::parallel_lex::lex_parallel(&bytes);
        let mut ms: Vec<Vec<f64>> = vec![Vec::with_capacity(ROUNDS); Arm::ALL.len()];
        let mut sizes = [0usize; 7];
        for round in 0..ROUNDS {
            for k in 0..Arm::ALL.len() {
                let i = (k + round) % Arm::ALL.len();
                let (size, took) = Arm::ALL[i].run(&bytes, &toks);
                sizes[i] = size;
                ms[i].push(took);
            }
        }
        let lex = median_kept(&ms[0]);
        println!("\ncorpus {path}, {} bytes, {} tokens", bytes.len(), toks.len());
        println!("{:<13} {:>10} {:>12} {:>10}", "arm", "produced", "median ms", "vs lex par");
        for (i, arm) in Arm::ALL.iter().enumerate() {
            let m = median_kept(&ms[i]);
            println!("{:<13} {:>10} {:>12.2} {:>9.2}x", arm.name(), sizes[i], m, m / lex);
        }
    }
    if unreadable > 0 {
        std::process::exit(1);
    }
}
