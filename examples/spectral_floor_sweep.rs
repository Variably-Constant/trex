//! What a floor under the change-point threshold does to the boundaries.
//!
//! A change-point fires where `dist` clears `mean + k*mad`. The `mad` term is
//! the reader's own estimate of how far `dist` moves, so a stretch where
//! `dist` stops moving drives the spread to zero and carries the threshold
//! down onto the signal; from there a step of one ULP is a boundary.
//! `cp_floor` holds the threshold a share of the mean above it, which no
//! single-ULP step can reach. The share has to be large enough to cover that
//! and small enough to leave a real boundary alone, and the value that does
//! both is a question about text rather than about the arithmetic.
//!
//! Five 4000-byte regimes - 'a', '7', '#', 'a', '7' - are the anchor, because
//! their answer is known: the reader puts a boundary at 2218 inside the first
//! uniform run and then holds closed through the switch at 4000, reading
//! `[2218, 8000, 12000, 16000]`. A floor is doing its job there when the
//! boundaries are the four switches and nothing else.
//!
//! Every other corpus is read from a path given on the command line. Each is
//! read at every floor and reported as a set against the floor-free reading,
//! because a boundary that moves leaves the count where it was: a column of
//! counts agreeing across a change that moved every boundary is the reading
//! this is built to avoid.
//!
//! The reading asked for is the one `\F{onset}` takes: the class mix, the
//! entropy, and the change-point over them. `dist` is the distance between
//! the mix's two clocks plus the entropy's, and reads neither the period nor
//! the k-gram surprise, so a field built without those two carries the same
//! boundaries as one built with them. Nothing is left out to save time.
//!
//!   cargo run --profile release-test --example spectral_floor_sweep -- <path>...

use std::collections::BTreeSet;

use trex::spectral::{Needs, SpectralConfig, analyze_needing};

/// The floors swept. The first is the reader with no floor, so every other row
/// is read against a baseline this run took rather than one quoted into it.
const FLOORS: [f32; 9] = [0.0, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08, 0.09, 0.10];

/// How far from a switch a boundary still counts as that switch, which is the
/// tolerance `a_regime_switch_is_found_past_the_settling_of_the_bias_corrections`
/// holds the reader to.
const NEAR: usize = 64;

/// Where the anchor's regimes change.
const SWITCHES: [usize; 4] = [4000, 8000, 12000, 16000];

/// How many positions a difference lists before it is summarized.
const SHOWN: usize = 8;

/// Five 4000-byte runs of one byte each.
fn five_regimes() -> Vec<u8> {
    let mut input = Vec::new();
    for b in *b"a7#a7" {
        input.extend(std::iter::repeat_n(b, 4000));
    }
    input
}

/// The boundaries `input` reads with the threshold floored at `floor`.
fn boundaries_at(input: &[u8], floor: f32) -> BTreeSet<usize> {
    let cfg = SpectralConfig { cp_floor: floor, ..SpectralConfig::default() };
    let needs = Needs { entropy: true, bands: true, onset: true, ..Needs::none() };
    analyze_needing(input, &cfg, needs).boundaries.into_iter().collect()
}

/// `positions` as a list, cut to [`SHOWN`] with the remainder counted.
fn listed(positions: &BTreeSet<usize>) -> String {
    let head: Vec<String> = positions.iter().take(SHOWN).map(usize::to_string).collect();
    if positions.len() > SHOWN {
        format!("{} and {} more", head.join(", "), positions.len() - SHOWN)
    } else {
        head.join(", ")
    }
}

/// The anchor at every floor, with what each reading gets right.
///
/// Reported as the switches found and the boundaries that are not switches,
/// because the defect has both halves: a fire inside the uniform run, and the
/// switch after it lost to the hysteresis that fire closed.
fn read_the_anchor() {
    let input = five_regimes();
    println!("five 4000-byte regimes a7#a7, switches at 4000, 8000, 12000, 16000");
    for floor in FLOORS {
        let got = boundaries_at(&input, floor);
        let near = |s: usize| got.iter().any(|&b| b.abs_diff(s) <= NEAR);
        let found = SWITCHES.iter().copied().filter(|&s| near(s)).count();
        let spurious: BTreeSet<usize> = got
            .iter()
            .copied()
            .filter(|&b| !SWITCHES.iter().any(|&s| b.abs_diff(s) <= NEAR))
            .collect();
        let verdict = if found == SWITCHES.len() && spurious.is_empty() {
            "every switch and nothing else".to_string()
        } else {
            format!("{found} of {} switches, {} elsewhere", SWITCHES.len(), spurious.len())
        };
        println!("  floor {floor:>5.2}  {verdict:<40}  [{}]", listed(&got));
    }
}

/// One corpus at every floor, each reading against the floor-free one.
fn read_a_corpus(what: &str, input: &[u8]) {
    let base = boundaries_at(input, FLOORS[0]);
    let (n, at) = (input.len(), FLOORS[0]);
    println!("\n{what}, {n} bytes, {} boundaries at floor {at:.2}", base.len());
    for floor in FLOORS.iter().skip(1).copied() {
        let got = boundaries_at(input, floor);
        let lost: BTreeSet<usize> = base.difference(&got).copied().collect();
        let gained: BTreeSet<usize> = got.difference(&base).copied().collect();
        println!(
            "  floor {floor:>5.2}  {:>7} boundaries, {:>7} kept, {:>7} lost, {:>7} gained",
            got.len(),
            base.intersection(&got).count(),
            lost.len(),
            gained.len()
        );
        if !lost.is_empty() {
            println!("            lost   [{}]", listed(&lost));
        }
        if !gained.is_empty() {
            println!("            gained [{}]", listed(&gained));
        }
    }
}

fn main() {
    read_the_anchor();
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        println!(
            "\nno corpus given. The anchor above is synthetic and says only that the floor \
             reaches the defect; what a floor costs is a question about real text, and this \
             takes its paths on the command line."
        );
        return;
    }
    for path in paths {
        match std::fs::read(&path) {
            Ok(bytes) => read_a_corpus(&path, &bytes),
            Err(e) => {
                eprintln!("{path}: {e}");
                std::process::exit(2);
            }
        }
    }
}
