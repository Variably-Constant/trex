//! What the anchor scan under the leftmost non-overlapping selection costs,
//! the dispatched path against the scalar baseline, and what the probe in
//! front of it is worth at several lengths.
//!
//! The selection over a resident split costs 0.736 ns an anchor walked plus
//! 2.50 ns a match written, and on a sparse pattern the anchor term is 96% of
//! it. That term is one loop over an array of per-anchor match ends, so what
//! it costs depends on the array's length and how far apart the matches the
//! walk takes are, not on which pattern produced it.
//!
//! The rows are built to train_corpus2's 5,162,460 anchors and to the matches
//! per anchor its two patterns select over the same bytes: 0.0118 for `\W \N`
//! and 0.3688 for `\W \W`. Those figures count the matches the walk takes
//! rather than the anchors that carry an end, and the walk jumps past each one
//! it takes, so the share of anchors carrying an end has to be higher than the
//! figure it produces. The table prints anchors per match so a row can be
//! lined up against the corpus directly.
//!
//! The vector scan pays its setup once a call rather than once a window, so
//! the p columns read a few anchors one at a time before reaching for it: p0
//! is no probe at all, and the simd column is the length the library ships.
//!
//! Every arm rotates its position by round, and the two being compared are
//! each also run against a second copy of themselves. Two controls rather
//! than one, because the arms do not share a bottleneck: the scalar walk is
//! issue-bound and the vector scan reads 20.6 MB and is bandwidth-bound, so a
//! neighbour moves them by different amounts and a control on one of them
//! alone cannot see it. Each reported ratio is the median of the per-round
//! paired ratios, never the ratio of two medians.

use std::time::Instant;

use trex::ends_simd::{
    next_match_from, next_match_from_scalar, next_match_from_unprobed, takes_vector,
};

/// Per-anchor match ends shaped as the device kernel writes them: `-1` where
/// an anchor matched nothing, and the end token where it did. An anchor
/// carries an end with probability `per_mille`, and a match runs one to four
/// tokens, so the walk skips the way it does over real text.
fn sample_ends(n: usize, per_mille: u32) -> Vec<i32> {
    (0..n)
        .map(|a| {
            let r = (a as u32).wrapping_mul(2_654_435_761) >> 11;
            if per_mille > 0 && r % 1000 < per_mille {
                a as i32 + 1 + (r % 4) as i32
            } else {
                -1
            }
        })
        .collect()
}

/// The selection's walk with the dispatched scan: step to the next anchor
/// whose end lies past it, jump to that end, repeat. It counts the matches
/// rather than building spans, so what is timed is the scan and the walk and
/// not an allocation.
fn walk_dispatched(ends: &[i32]) -> usize {
    let n = ends.len();
    let mut a = 0usize;
    let mut matches = 0usize;
    while let Some(m) = next_match_from(ends, a, n) {
        a = ends[m] as usize;
        matches += 1;
    }
    matches
}

/// The same walk with the scalar baseline.
fn walk_scalar(ends: &[i32]) -> usize {
    let n = ends.len();
    let mut a = 0usize;
    let mut matches = 0usize;
    while let Some(m) = next_match_from_scalar(ends, a, n) {
        a = ends[m] as usize;
        matches += 1;
    }
    matches
}

/// The same walk reading `P` anchors one at a time before reaching for the
/// vector scan. `P` of zero is the scan with no probe in front of it.
fn walk_probe<const P: usize>(ends: &[i32]) -> usize {
    let n = ends.len();
    let mut a = 0usize;
    let mut matches = 0usize;
    loop {
        let stop = (a + P).min(n);
        let hit = if P == 0 { None } else { next_match_from_scalar(ends, a, stop) };
        let m = match hit {
            Some(m) => m,
            None => match next_match_from_unprobed(ends, stop, n) {
                Some(m) => m,
                None => break,
            },
        };
        a = ends[m] as usize;
        matches += 1;
    }
    matches
}

/// The same walk letting the library choose its scan from how far the recent
/// calls reached, which is the path the two selections take.
fn walk_adaptive(ends: &[i32]) -> usize {
    // The selections choose once and then run a loop that calls only the scan
    // they chose, so the arm has to do the same or it measures a per-call test
    // the device path does not pay.
    if takes_vector(ends) { walk_dispatched(ends) } else { walk_scalar(ends) }
}

/// The rung this CPU resolved, named without depending on how `Tier` prints.
fn tier_name() -> &'static str {
    match trex::isa::tier() {
        trex::isa::Tier::Avx512 => "avx512",
        trex::isa::Tier::Avx2 => "avx2",
        trex::isa::Tier::Sse2 => "sse2",
        trex::isa::Tier::Scalar => "scalar",
    }
}

/// The median of `v`, which is sorted in place.
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).expect("no timing is NaN"));
    v[v.len() / 2]
}

/// One timed walk, in nanoseconds. The match count is returned into a black
/// box so the walk cannot be optimized away.
fn time(ends: &[i32], walk: fn(&[i32]) -> usize) -> f64 {
    let t = Instant::now();
    let matches = walk(ends);
    let ns = t.elapsed().as_nanos() as f64;
    std::hint::black_box(matches);
    ns
}

fn main() {
    const ROUNDS: usize = 21;
    const ANCHORS: usize = 5_162_460;
    // The share of anchors carrying an end. A walk that jumps past each match
    // it takes selects fewer matches than that share: 826 per mille is what
    // lands near the corpus's dense 0.3688 matches an anchor, and 12 near its
    // sparse 0.0118. The row with nothing to find is the scan's ceiling, where
    // the walk never jumps.
    const DENSITIES: [(u32, &str); 5] =
        [(0, "none"), (12, "sparse"), (60, "mid"), (369, "dense"), (826, "corpus")];

    type Walk = fn(&[i32]) -> usize;
    // Slot 0 is the baseline every ratio is taken against and slot 1 the path
    // the library ships. Slots 5 and 6 are a second copy of each of those two,
    // so each has a control of its own kind.
    const ARMS: [(&str, Walk); 8] = [
        ("scalar", walk_scalar),
        ("simd", walk_dispatched),
        ("adaptive", walk_adaptive),
        ("p0", walk_probe::<0>),
        ("p2", walk_probe::<2>),
        ("p8", walk_probe::<8>),
        ("scalar control", walk_scalar),
        ("simd control", walk_dispatched),
    ];

    println!("\n=== the anchor scan under the selection, {ROUNDS} rounds, tier {} ===", tier_name());
    println!(
        "{:>8} {:>10} {:>11} {:>13} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>13}",
        "density",
        "matches",
        "anchors/m",
        "scalar ns/a",
        "simd",
        "adaptive",
        "p0",
        "p2",
        "p8",
        "control",
        "simd control"
    );

    for (per_mille, label) in DENSITIES {
        let ends = sample_ends(ANCHORS, per_mille);
        let matches = walk_scalar(&ends);
        for (name, walk) in ARMS {
            assert_eq!(matches, walk(&ends), "arm {name} selected a different set of matches");
        }
        // How far the walk travels between the matches it takes, which is what
        // decides whether the probe answers a call before the vector setup.
        let per_match =
            if matches == 0 { f64::INFINITY } else { ANCHORS as f64 / matches as f64 };

        let mut ns: Vec<Vec<f64>> = vec![Vec::with_capacity(ROUNDS); ARMS.len()];
        for round in 0..ROUNDS {
            // Each arm takes a different position in the order every round, so
            // a drift across the run lands on all of them rather than on
            // whichever went first every time.
            for k in 0..ARMS.len() {
                let arm = (k + round) % ARMS.len();
                let t = time(&ends, ARMS[arm].1);
                ns[arm].push(t);
            }
        }

        let paired = |base: usize, arm: usize| {
            let mut r: Vec<f64> = (0..ROUNDS).map(|i| ns[base][i] / ns[arm][i]).collect();
            median(&mut r)
        };
        let mut base: Vec<f64> = ns[0].iter().map(|t| t / ANCHORS as f64).collect();
        println!(
            "{label:>8} {matches:>10} {per_match:>11.2} {:>13.4} {:>8.3}x {:>8.3}x {:>8.3}x {:>8.3}x {:>8.3}x {:>8.3}x {:>12.3}x",
            median(&mut base),
            paired(0, 1),
            paired(0, 2),
            paired(0, 3),
            paired(0, 4),
            paired(0, 5),
            paired(0, 6),
            paired(1, 7),
        );
    }
    println!(
        "\nanchors/m is how far the walk travels between the matches it takes; the corpus row sits \
         near train_corpus2's dense pattern at 2.71 and the sparse row near its 84.6. scalar ns/a \
         is the baseline's own time over every anchor in the array, the same denominator the \
         0.736 ns figure was fitted with, and every ratio column is the median of the per-round \
         ratios of that baseline to the arm, above 1 meaning faster. simd takes the vector scan on \
         every call behind the probe length the library ships; adaptive is what the selections run, \
         choosing per call from how far the recent ones reached; p0 is the vector scan with no \
         probe at all, and p2 and p8 are the shipped form at other probe lengths. The last two \
         columns run the baseline and the always-vector path each against a second copy of itself, \
         and no ratio is worth reading by more than how far it clears both: the arms do not share \
         a bottleneck, so a box that moves one need not move the other."
    );
}
