//! Whether gravity and repulsion between units - their pull to turn up closer
//! together, or push to stay further apart, than chance puts them - carry
//! structure trex's axes do not, read on real files at the byte, the token and
//! the supertoken grain.
//!
//! The field is the pair correlation `g_ab(d) = P(b at t + d | a at t) / P(b)`:
//! above one `a` pulls `b` to the gap `d`, below one it pushes `b` off it, at one
//! it is indifferent; `-ln g` is the potential of mean force. Echo reads a unit's
//! pull on its own repeats and the resonator bank a class's pull on itself, and
//! the relation tier reads the bonds a rule names: a bracket's head on what it
//! encloses, an operator's operands. No axis reads the pull the data shows one
//! type to have on another, or a push at all.
//!
//! Four readings come from the field, one a position:
//!
//!   strain      the position's energy under the field: the mean potential
//!               between the unit there and each unit before it within the
//!               grain's reach - low where the unit is where the field expects
//!               it, high where it is not
//!   bond reach  the distance to the strongest bond the unit completes: the gap
//!               at which some earlier unit's pull on it peaks - none, adjacent,
//!               two to four, five to sixteen, or further
//!   binding     the attraction across the cut before the position, summed over
//!               the pairs of units either side of it within eight: a unit holds
//!               together where it is high and splits where it is low
//!   geometry    the class of the unit's type in the space where attraction pulls
//!               types together and repulsion pushes them apart: the leading
//!               eigenvectors of the mean `ln g` over the gaps, read only from
//!               cells whose count clears chance, clustered
//!
//! and each is judged three ways:
//!
//!   real        the field holds cells past |z| 4, and mutual information at a
//!               gap, that a shuffled copy of the same units does not
//!   stable      the reading from a field learned on every other block of the
//!               first half agrees with the reading from a field learned on the
//!               blocks between, where two fields learned on the same blocks
//!               shuffled do not
//!   unique      the bits the reading still takes on held-out blocks once the
//!               readings trex already has at that position are known
//!
//! The field is learned on the first half. The second half is dealt into
//! blocks: every other block is scored and the models are fit on the rest, so
//! what is fit and what is scored cover the same stretch of the input. Two
//! models read a reading from trex's readings, and the fewer bits either leaves
//! is what trex's readings explain:
//!
//!   a chain     a context one more of trex's readings a link, each link's
//!               counts smoothed toward the prediction of the link before; a link
//!               holding exactly the positions of the one before is the same
//!               evidence and is passed over, and the chain stops where every
//!               context holds a single position. Its first links are chosen one
//!               at a time, each the reading that with the links before it reads
//!               held-out fit blocks best, for as long as one reads them better;
//!               the rest follow in the order each alone reads them best.
//!   a mixer     a logistic mixer a node of the reading's bit tree, over every
//!               reading's own prediction, the reading's prior and the chain's
//!               prediction, starting from the chain. It learns on held-out fit
//!               blocks from experts fit on the others, and reads the score
//!               blocks with experts refit on every fit block.
//!
//! Every setting either model has - how strongly a link leans on the one
//! before, how many links it keeps, the mixer's rate and passes - is the one
//! that reads the score blocks best, so trex's readings are given every benefit
//! and what they leave unexplained is the least it can be. Two controls run
//! beside every grain: a reading trex's readings fix, which the models have to
//! explain whole, and a reading drawn at random, whose explained share is what
//! choosing the settings on the score blocks gains by itself.
//!
//! The readings trex already has are
//!
//!   bytes        the byte and the two before it; the token over it - its kind,
//!                the byte's place in it, its echo, shape class, stress depth
//!                and the entanglement of the cut before it; the byte seam's
//!                cut, its strength, and the forward and backward branching
//!                entropy; spectral entropy, novelty and period strength, and
//!                the spectral bands - each byte class's share over the last 8,
//!                32 and 128 bytes; the observer dependence; and the token's
//!                windowed readings
//!   tokens       the token's class and the two before it; the record period's
//!                column; echo and the lag back to the last occurrence; shape
//!                class, novelty and period; stress depth, strain and load;
//!                magnitude; spectral entropy and the spectral bands; the token
//!                seam's cut and the byte seam's strength at the token; the
//!                supertoken's role and whether the token opens it; the rhythm's
//!                power and group delay; the entanglement of the cut before it;
//!                the observer dependence; the flow of magnitude; the action; how
//!                far back the head enclosing it is; and its windowed readings
//!   supertokens  the unit's structure and the role before it; depth; length;
//!                whether its structure recurs; the unit seam's cut; the kind it
//!                opens with; the entanglement between units at its start; and
//!                the unit rung's windowed readings
//!
//! A token's windowed readings are the rolling context's folds over the tokens
//! before it - scale, depth, entropy, novelty, observer dependence, seam cuts
//! and reversals - and the related context's: how often the token occurred
//! before, how long since the regime changed, the scale and novelty at its
//! phase of the period, the scale of the heads enclosing it, and the values its
//! key was bound to before. A unit's are the unit-rung window's folds and its
//! own. A token's class is its kind, with punctuation and the other one-off
//! bytes told apart by their text, as a pattern tells them apart. Curvature,
//! geodesic distance, topology and holography read edges, pairs or the whole
//! stream rather than a position, so no position carries them.
//!
//! The geometry is a function of the unit's type, so it is held against the
//! partitions trex already groups units by - the byte's class; the token's kind
//! and shape class; the unit's role - rather than against the context.
//!
//! If gravity builds the monoid, a grain's binding drops where the next grain's
//! units begin, so binding is ranked against them: the byte grain's against
//! token starts, the token grain's against supertoken starts, the supertoken
//! grain's against line starts. Beside it are trex's own signals for a cut at
//! that grain ranked the same way, its seam's cuts, and binding at as many cuts
//! as the seam makes.
//!
//! Every file is read as `trex scan` reads it (`trex::encoding::decode`).
//!
//! Run: `cargo run --release --example what_gravity_and_repulsion_carry -- [--blocks N] <file>...`

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use trex::token::{Token, TokenKind};

/// The gaps each grain's field reads to.
const BYTE_REACH: usize = 64;
const TOKEN_REACH: usize = 32;
const UNIT_REACH: usize = 16;
/// How far either side of a cut its binding reaches.
const BIND_REACH: usize = 8;
/// The types a grain with an open alphabet keeps; the rest are one type.
const TOP_TYPES: usize = 255;
/// Poisson standard deviations past which a cell reads as attracting or
/// repelling. The shuffled copy says how many cells clear it by chance.
const Z_CELL: f64 = 4.0;
/// Expected pairs a cell needs before its z is read.
const MIN_EXPECTED: f64 = 5.0;
/// Quantile bins a continuous reading is cut into.
const BINS: usize = 8;
/// The geometry's dimensions and classes. A type with no cell clearing chance
/// has no place in it and takes a class of its own.
const GEOMETRY_DIMS: usize = 8;
const GEOMETRY_CLASSES: usize = 16;
/// Bond reach's classes: none, adjacent, two to four, five to sixteen, further.
const REACH_CLASSES: usize = 5;
/// The blocks each half is dealt into, unless `--blocks` says otherwise.
const SPLIT_BLOCKS: usize = 64;
/// The weights, in positions, a link of the chain may give the prediction of
/// the link before it.
const BETAS: [f64; 7] = [0.25, 1.0, 4.0, 16.0, 64.0, 256.0, 1024.0];
/// The rates a mixer may learn at, and the most passes it may make over what it
/// learns from.
const RATES: [f64; 3] = [0.0005, 0.002, 0.008];
const MAX_PASSES: usize = 4;
/// The spectral bands read: each byte class's share over the last 8, 32 and 128
/// bytes, as the clock's index, the class's, and the reading's name.
const BAND_COUNT: usize = 15;
const BANDS: [(usize, usize, &str); BAND_COUNT] = [
    (1, 0, "digits over 8 bytes"),
    (1, 1, "letters over 8 bytes"),
    (1, 2, "space over 8 bytes"),
    (1, 3, "punctuation over 8 bytes"),
    (1, 4, "other bytes over 8 bytes"),
    (2, 0, "digits over 32 bytes"),
    (2, 1, "letters over 32 bytes"),
    (2, 2, "space over 32 bytes"),
    (2, 3, "punctuation over 32 bytes"),
    (2, 4, "other bytes over 32 bytes"),
    (3, 0, "digits over 128 bytes"),
    (3, 1, "letters over 128 bytes"),
    (3, 2, "space over 128 bytes"),
    (3, 3, "punctuation over 128 bytes"),
    (3, 4, "other bytes over 128 bytes"),
];
/// The names of a window's folds, at the token rung, at the unit rung, and of a
/// unit's own.
const WINDOW_NAMES: [&str; 7] = [
    "window scale",
    "window depth",
    "window entropy",
    "window novelty",
    "window observer dependence",
    "window seam cuts",
    "window reversals",
];
const UNIT_WINDOW_NAMES: [&str; 7] = [
    "unit window scale",
    "unit window depth",
    "unit window entropy",
    "unit window novelty",
    "unit window observer dependence",
    "unit window seam cuts",
    "unit window reversals",
];
const OWN_NAMES: [&str; 7] = [
    "the unit's scale",
    "the unit's depth",
    "the unit's entropy",
    "the unit's novelty",
    "the unit's observer dependence",
    "the unit's seam cuts",
    "the unit's reversals",
];
/// The seeds the shuffled copies are drawn from.
const SHUFFLE_SEED: u64 = 0x9e37_79b9_7f4a_7c15;
const RANDOM_READING_SEED: u64 = 0xd1b5_4a32_d192_ed03;
/// The key of the empty context every chain starts from.
const CHAIN_ROOT: u64 = 0x2545_f491_4f6c_dd1d;

/// Pair counts over gaps `1..=reach`, and the potential each cell gives.
struct Field {
    types: usize,
    reach: usize,
    /// `pair[(d - 1) * types * types + a * types + b]`: positions `i` holding `a`
    /// with `b` at `i + d`.
    pair: Vec<u32>,
    /// `from[(d - 1) * types + a]`: positions holding `a` with a unit `d` on.
    from: Vec<u32>,
    /// `to[(d - 1) * types + b]`: positions holding `b` with a unit `d` back.
    to: Vec<u32>,
    /// `pairs[d - 1]`: the pairs a gap of `d` fits.
    pairs: Vec<u32>,
    /// `-ln g` a cell, `g` smoothed by one pair on each side, so a pair never
    /// seen reads as a push in proportion to how often chance would show it.
    potential: Vec<f32>,
}

impl Field {
    /// The field of the pairs inside each of `blocks` of `seq`, over gaps
    /// `1..=reach`.
    fn learn(seq: &[u32], blocks: &[Range<usize>], types: usize, reach: usize) -> Field {
        let tt = types * types;
        let mut pair = vec![0u32; reach * tt];
        let mut from = vec![0u32; reach * types];
        let mut to = vec![0u32; reach * types];
        let mut pairs = vec![0u32; reach];
        for block in blocks {
            let s = &seq[block.clone()];
            for d in 1..=reach.min(s.len().saturating_sub(1)) {
                let cells = &mut pair[(d - 1) * tt..d * tt];
                let froms = &mut from[(d - 1) * types..d * types];
                let tos = &mut to[(d - 1) * types..d * types];
                for (&a, &b) in s.iter().zip(&s[d..]) {
                    cells[a as usize * types + b as usize] += 1;
                    froms[a as usize] += 1;
                    tos[b as usize] += 1;
                }
                pairs[d - 1] += u32::try_from(s.len() - d).expect("a block's length fits a count");
            }
        }
        let mut field = Field { types, reach, pair, from, to, pairs, potential: Vec::new() };
        let potential: Vec<f32> = (0..reach * tt)
            .map(|k| {
                let (d, a, b) = (k / tt + 1, k % tt / types, k % types);
                -field.lift(a, b, d).ln() as f32
            })
            .collect();
        field.potential = potential;
        field
    }

    /// The pairs of `a` then `b` at gap `d` chance alone would give.
    fn expected(&self, a: usize, b: usize, d: usize) -> f64 {
        let m = f64::from(self.pairs[d - 1]);
        if m <= 0.0 {
            return 0.0;
        }
        f64::from(self.from[(d - 1) * self.types + a]) * f64::from(self.to[(d - 1) * self.types + b]) / m
    }

    fn count(&self, a: usize, b: usize, d: usize) -> f64 {
        f64::from(self.pair[(d - 1) * self.types * self.types + a * self.types + b])
    }

    /// `g`, smoothed by one pair on each side.
    fn lift(&self, a: usize, b: usize, d: usize) -> f64 {
        (self.count(a, b, d) + 1.0) / (self.expected(a, b, d) + 1.0)
    }

    /// The count's distance from chance in Poisson standard deviations.
    fn z(&self, a: usize, b: usize, d: usize) -> f64 {
        let e = self.expected(a, b, d);
        if e <= 0.0 { 0.0 } else { (self.count(a, b, d) - e) / e.sqrt() }
    }

    fn potential_at(&self, a: u32, b: u32, d: usize) -> f64 {
        f64::from(self.potential[(d - 1) * self.types * self.types + a as usize * self.types + b as usize])
    }

    /// How often type `a` opens a pair at the shortest gap: its count in what
    /// the field was learned on, less the last unit of each block.
    fn frequency(&self, a: usize) -> f64 {
        f64::from(self.from[a])
    }

    /// Bits of mutual information between a unit and the unit `d` on.
    fn mutual_information(&self, d: usize) -> f64 {
        let m = f64::from(self.pairs[d - 1]);
        if m <= 0.0 {
            return 0.0;
        }
        let mut bits = 0.0;
        for a in 0..self.types {
            for b in 0..self.types {
                let c = self.count(a, b, d);
                if c > 0.0 {
                    bits += c / m * (c / self.expected(a, b, d)).log2();
                }
            }
        }
        bits
    }

    /// The cells past `Z_CELL`, attracting and repelling, among those chance
    /// fills to at least `MIN_EXPECTED`.
    fn significant_cells(&self) -> (usize, usize) {
        let (mut up, mut down) = (0, 0);
        for d in 1..=self.reach {
            for a in 0..self.types {
                for b in 0..self.types {
                    if self.expected(a, b, d) < MIN_EXPECTED {
                        continue;
                    }
                    let z = self.z(a, b, d);
                    if z > Z_CELL {
                        up += 1;
                    } else if z < -Z_CELL {
                        down += 1;
                    }
                }
            }
        }
        (up, down)
    }
}

/// Each ordered pair's bond: the gap at which the first unit's pull on the
/// second peaks, where the peak clears `Z_CELL`, with its z; gap zero where none
/// does.
struct Bonds {
    types: usize,
    gap: Vec<u8>,
    z: Vec<f32>,
}

fn bonds(f: &Field) -> Bonds {
    let tt = f.types * f.types;
    let (mut gap, mut z) = (vec![0u8; tt], vec![0f32; tt]);
    for a in 0..f.types {
        for b in 0..f.types {
            let mut best = (0usize, Z_CELL);
            for d in 1..=f.reach {
                if f.expected(a, b, d) < MIN_EXPECTED {
                    continue;
                }
                let zd = f.z(a, b, d);
                if zd > best.1 {
                    best = (d, zd);
                }
            }
            if best.0 > 0 {
                gap[a * f.types + b] = u8::try_from(best.0).expect("a reach under 256");
                z[a * f.types + b] = best.1 as f32;
            }
        }
    }
    Bonds { types: f.types, gap, z }
}

/// Each position's energy under the field: the mean potential between its unit
/// and each unit before it within the field's reach.
fn strain(seq: &[u32], f: &Field, at: Range<usize>) -> Vec<f64> {
    at.map(|t| {
        let span = t.min(f.reach);
        if span == 0 {
            return 0.0;
        }
        (1..=span).map(|d| f.potential_at(seq[t - d], seq[t], d)).sum::<f64>() / span as f64
    })
    .collect()
}

/// The class of the gap to the strongest bond each position completes.
fn bond_reach(seq: &[u32], b: &Bonds, reach: usize, at: Range<usize>) -> Vec<u16> {
    at.map(|t| {
        let mut best: Option<(usize, f32)> = None;
        for d in 1..=t.min(reach) {
            let k = seq[t - d] as usize * b.types + seq[t] as usize;
            if usize::from(b.gap[k]) == d && best.is_none_or(|(_, z)| b.z[k] > z) {
                best = Some((d, b.z[k]));
            }
        }
        match best {
            None => 0,
            Some((1, _)) => 1,
            Some((d, _)) if d <= 4 => 2,
            Some((d, _)) if d <= 16 => 3,
            Some(_) => 4,
        }
    })
    .collect()
}

/// The attraction across the cut before each position: `ln g` summed over the
/// pairs of units that straddle it within `BIND_REACH`.
fn binding(seq: &[u32], f: &Field, at: Range<usize>) -> Vec<f64> {
    let n = seq.len();
    at.map(|t| {
        let mut s = 0.0;
        for d in 1..=BIND_REACH.min(f.reach) {
            for i in t.saturating_sub(d)..t {
                if i + d < n {
                    s -= f.potential_at(seq[i], seq[i + d], d);
                }
            }
        }
        s
    })
    .collect()
}

/// Each type's class in the space where attraction pulls types together and
/// repulsion pushes them apart. The space is built only from cells whose count
/// clears chance, so a pair seen once by accident places nothing; a type with no
/// such cell has no place in it and takes class `GEOMETRY_CLASSES`. The rest
/// take the leading eigenvectors of the mean `ln g` over the gaps, symmetrized,
/// each scaled by the root of its eigenvalue's size, clustered.
fn geometry(f: &Field) -> Vec<u16> {
    let t = f.types;
    let unplaced = u16::try_from(GEOMETRY_CLASSES).expect("a handful of classes");
    let mut out = vec![unplaced; t];
    let mut affinity = vec![0.0f64; t * t];
    let mut evident = vec![false; t];
    for d in 1..=f.reach {
        for a in 0..t {
            for b in 0..t {
                if f.expected(a, b, d) >= MIN_EXPECTED && f.z(a, b, d).abs() > Z_CELL {
                    affinity[a * t + b] += f.lift(a, b, d).ln();
                    evident[a] = true;
                    evident[b] = true;
                }
            }
        }
    }
    let placed: Vec<usize> = (0..t).filter(|&a| evident[a]).collect();
    let m = placed.len();
    if m < GEOMETRY_CLASSES {
        return out;
    }
    let scale = (2 * f.reach) as f64;
    let mut s = vec![0.0f64; m * m];
    for (i, &a) in placed.iter().enumerate() {
        for (j, &b) in placed.iter().enumerate() {
            s[i * m + j] = (affinity[a * t + b] + affinity[b * t + a]) / scale;
        }
    }
    let (vecs, vals) = leading_eigenvectors(&s, m, GEOMETRY_DIMS.min(m));
    let points: Vec<Vec<f64>> =
        (0..m).map(|i| vecs.iter().zip(&vals).map(|(v, l)| v[i] * l.abs().sqrt()).collect()).collect();
    let weights: Vec<f64> = placed.iter().map(|&a| f.frequency(a)).collect();
    let classes = kmeans(&points, &weights, GEOMETRY_CLASSES);
    for (&a, &c) in placed.iter().zip(&classes) {
        out[a] = c;
    }
    out
}

fn dot(x: &[f64], y: &[f64]) -> f64 {
    x.iter().zip(y).map(|(a, b)| a * b).sum()
}

fn squared_distance(x: &[f64], y: &[f64]) -> f64 {
    x.iter().zip(y).map(|(a, b)| (a - b) * (a - b)).sum()
}

fn mat_vec(s: &[f64], m: usize, v: &[f64]) -> Vec<f64> {
    s.chunks(m).map(|row| dot(row, v)).collect()
}

fn orthonormalize(vs: &mut [Vec<f64>]) {
    for i in 0..vs.len() {
        let (done, rest) = vs.split_at_mut(i);
        let v = &mut rest[0];
        for u in done.iter() {
            let p = dot(v.as_slice(), u);
            for (x, y) in v.iter_mut().zip(u) {
                *x -= p * y;
            }
        }
        let norm = dot(v.as_slice(), v.as_slice()).sqrt();
        if norm > 0.0 {
            for x in v.iter_mut() {
                *x /= norm;
            }
        }
    }
}

/// The `k` eigenvectors of the symmetric `m` by `m` matrix `s` whose eigenvalues
/// are largest in size, by orthogonal iteration from a fixed start, with their
/// eigenvalues.
fn leading_eigenvectors(s: &[f64], m: usize, k: usize) -> (Vec<Vec<f64>>, Vec<f64>) {
    let mut q: Vec<Vec<f64>> = (0..k)
        .map(|j| (0..m).map(|i| ((i * 7919 + j * 104_729) % 1000) as f64 / 1000.0 - 0.5).collect())
        .collect();
    orthonormalize(&mut q);
    for _ in 0..300 {
        let mut z: Vec<Vec<f64>> = q.iter().map(|v| mat_vec(s, m, v)).collect();
        orthonormalize(&mut z);
        q = z;
    }
    let vals = q.iter().map(|v| dot(v, &mat_vec(s, m, v))).collect();
    (q, vals)
}

/// Weighted k-means from a fixed start: the heaviest point, then each next
/// center the point farthest from every center so far, its distance weighted.
fn kmeans(points: &[Vec<f64>], weights: &[f64], k: usize) -> Vec<u16> {
    let dist = squared_distance;
    let first = (0..points.len()).max_by(|&i, &j| weights[i].total_cmp(&weights[j])).expect("points to cluster");
    let mut centers: Vec<Vec<f64>> = vec![points[first].clone()];
    while centers.len() < k {
        let reach = |i: usize| {
            centers.iter().map(|c| dist(&points[i], c)).fold(f64::INFINITY, f64::min) * weights[i]
        };
        let far = (0..points.len()).max_by(|&i, &j| reach(i).total_cmp(&reach(j))).expect("points to cluster");
        centers.push(points[far].clone());
    }
    let mut assign = vec![0u16; points.len()];
    for _ in 0..100 {
        let mut moved = false;
        for (p, a) in points.iter().zip(assign.iter_mut()) {
            let best = (0..k).min_by(|&x, &y| dist(p, &centers[x]).total_cmp(&dist(p, &centers[y]))).expect("centers");
            let best = u16::try_from(best).expect("a handful of classes");
            if best != *a {
                *a = best;
                moved = true;
            }
        }
        for (c, center) in centers.iter_mut().enumerate() {
            let mut sum = vec![0.0; center.len()];
            let mut w = 0.0;
            for ((p, &a), &pw) in points.iter().zip(&assign).zip(weights) {
                if usize::from(a) == c {
                    for (s, x) in sum.iter_mut().zip(p) {
                        *s += pw * x;
                    }
                    w += pw;
                }
            }
            if w > 0.0 {
                *center = sum.into_iter().map(|s| s / w).collect();
            }
        }
        if !moved {
            break;
        }
    }
    assign
}

/// `seq` shuffled from a fixed seed: the same values with every gap between
/// them broken.
fn shuffled<T: Copy>(seq: &[T], seed: u64) -> Vec<T> {
    let mut out = seq.to_vec();
    let mut s = seed | 1;
    for i in (1..out.len()).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let j = usize::try_from(s % (i as u64 + 1)).expect("an index below the length");
        out.swap(i, j);
    }
    out
}

/// `range` cut into `count` consecutive blocks.
fn blocks(range: Range<usize>, count: usize) -> Vec<Range<usize>> {
    let len = range.len();
    (0..count).map(|b| range.start + b * len / count..range.start + (b + 1) * len / count).collect()
}

/// A stretch's positions dealt out by block. Every other block is scored; of
/// the blocks between, alternate ones take the first fit and the check a model
/// learns its choices on. `fit` is the first fit and the check together, so a
/// model that is scored is fit on every block that is not.
struct Split {
    inner: Vec<usize>,
    check: Vec<usize>,
    fit: Vec<usize>,
    score: Vec<usize>,
}

impl Split {
    fn deal(range: Range<usize>, count: usize) -> Split {
        let (mut inner, mut check, mut score) = (Vec::new(), Vec::new(), Vec::new());
        for (b, block) in blocks(range, count).into_iter().enumerate() {
            match b % 4 {
                0 => inner.extend(block),
                2 => check.extend(block),
                _ => score.extend(block),
            }
        }
        let mut fit: Vec<usize> = inner.iter().chain(&check).copied().collect();
        fit.sort_unstable();
        Split { inner, check, fit, score }
    }
}

/// Edges cutting `sample` into `BINS` bins at its quantiles.
fn quantile_edges(mut sample: Vec<f64>) -> Vec<f64> {
    sample.sort_by(f64::total_cmp);
    if sample.is_empty() {
        return Vec::new();
    }
    (1..BINS).map(|k| sample[k * sample.len() / BINS]).collect()
}

fn bin_of(v: f64, edges: &[f64]) -> u16 {
    u16::try_from(edges.iter().filter(|&&e| v >= e).count()).expect("seven edges at most")
}

/// A reading cut at the quantiles its values take at the positions `fit`,
/// `values[0]` being position `offset`.
fn binned(values: &[f64], offset: usize, fit: &[usize]) -> Vec<u16> {
    let edges = quantile_edges(fit.iter().map(|&t| values[t - offset]).collect());
    values.iter().map(|&v| bin_of(v, &edges)).collect()
}

/// A continuous reading of trex's, cut at its own quantiles.
fn quantiled(values: &[f64]) -> Vec<u32> {
    let edges = quantile_edges(values.to_vec());
    values.iter().map(|&v| u32::from(bin_of(v, &edges))).collect()
}

/// A count cut at powers of two: zero, one, two to three, four to seven, on up.
fn octave(count: u32) -> u32 {
    if count == 0 { 0 } else { 1 + count.ilog2() }
}

fn share(part: u32, count: u32) -> f64 {
    if count == 0 { 0.0 } else { f64::from(part) / f64::from(count) }
}

/// Passes a context key through: the keys are mixed already, so hashing them
/// again costs time and gains nothing.
#[derive(Default)]
struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(b);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

type Keyed<V> = HashMap<u64, V, BuildHasherDefault<KeyHasher>>;

/// Counts of a reading's values in each context a model has seen.
struct Counts {
    values: usize,
    slots: Keyed<u32>,
    arena: Vec<u32>,
}

impl Counts {
    fn new(values: usize) -> Counts {
        Counts { values, slots: Keyed::default(), arena: Vec::new() }
    }

    fn add(&mut self, key: u64, value: usize) {
        let next = u32::try_from(self.arena.len() / self.values).expect("contexts fit a count");
        let slot = *self.slots.entry(key).or_insert(next);
        if slot == next {
            self.arena.resize(self.arena.len() + self.values, 0);
        }
        self.arena[slot as usize * self.values + value] += 1;
    }

    fn get(&self, key: u64) -> Option<&[u32]> {
        self.slots.get(&key).map(|&s| &self.arena[s as usize * self.values..(s as usize + 1) * self.values])
    }

    fn contexts(&self) -> usize {
        self.slots.len()
    }
}

/// splitmix64's finalizer: every input bit reaches about half the output bits.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A context's key extended by one more feature's value.
fn extend(key: u64, value: u32) -> u64 {
    mix(key ^ u64::from(value).wrapping_mul(0xd6e8_feb8_6659_fd93))
}

/// The context key of position `t` at each depth of a chain up to `depth`,
/// each the key one link shorter extended by the next feature.
fn context_keys(features: &[&[u32]], t: usize, depth: usize, keys: &mut Vec<u64>) {
    keys.clear();
    let mut key = CHAIN_ROOT;
    keys.push(key);
    for f in &features[..depth] {
        key = extend(key, f[t]);
        keys.push(key);
    }
}

/// A reading's counts in the context each depth of a feature chain makes, to
/// the depth at which every context holds a single position: a deeper context
/// holds that same position or none, so it adds nothing.
struct Model {
    values: usize,
    tables: Vec<Counts>,
}

impl Model {
    fn fit(reading: &[u16], offset: usize, values: usize, features: &[&[u32]], at: &[usize]) -> Model {
        let mut tables: Vec<Counts> = Vec::with_capacity(features.len() + 1);
        let mut keys = vec![CHAIN_ROOT; at.len()];
        for depth in 0..=features.len() {
            if depth > 0 {
                let f = features[depth - 1];
                for (key, &t) in keys.iter_mut().zip(at) {
                    *key = extend(*key, f[t]);
                }
            }
            let mut table = Counts::new(values);
            for (&key, &t) in keys.iter().zip(at) {
                table.add(key, usize::from(reading[t - offset]));
            }
            let every_one_alone = table.contexts() == at.len();
            tables.push(table);
            if every_one_alone {
                break;
            }
        }
        Model { values, tables }
    }

    /// `p` leaned toward a context's counts `c` over `total` positions, the
    /// prediction before it weighing `beta` positions.
    fn lean(p: &mut [f64], c: &[u32], total: u64, beta: f64) {
        let n = total as f64;
        for (q, &x) in p.iter_mut().zip(c) {
            *q = (f64::from(x) + beta * *q) / (n + beta);
        }
    }

    /// The mean bits a position at `at` takes, for each weight in `BETAS` and
    /// each depth of the chain: `bits[beta][depth]`. A context seen nowhere in
    /// the fit ends the chain there, since every deeper one refines it; a
    /// context holding as many positions as the one before holds the same
    /// positions, so it is the same evidence again and is passed over.
    fn bits(&self, reading: &[u16], offset: usize, features: &[&[u32]], at: &[usize]) -> Vec<Vec<f64>> {
        let depths = self.tables.len();
        let mut bits = vec![vec![0.0f64; depths]; BETAS.len()];
        let mut keys = Vec::with_capacity(depths);
        let mut chain: Vec<(&[u32], u64)> = Vec::with_capacity(depths);
        let mut p = vec![0.0f64; self.values];
        let uniform = 1.0 / self.values as f64;
        for &t in at {
            context_keys(features, t, depths - 1, &mut keys);
            chain.clear();
            for (table, &key) in self.tables.iter().zip(&keys) {
                let Some(c) = table.get(key) else { break };
                chain.push((c, c.iter().map(|&x| u64::from(x)).sum()));
            }
            let value = usize::from(reading[t - offset]);
            for (row, &beta) in bits.iter_mut().zip(&BETAS) {
                p.fill(uniform);
                let mut applied: Option<u64> = None;
                for (depth, cell) in row.iter_mut().enumerate() {
                    if let Some(&(c, total)) = chain.get(depth)
                        && applied != Some(total)
                    {
                        Self::lean(&mut p, c, total, beta);
                        applied = Some(total);
                    }
                    *cell -= p[value].log2();
                }
            }
        }
        let n = at.len().max(1) as f64;
        for row in &mut bits {
            for cell in row.iter_mut() {
                *cell /= n;
            }
        }
        bits
    }

    /// The chain's prediction for position `t`, at the weight `BETAS[beta]`
    /// and to `depth` links, into `p`.
    fn predict(&self, features: &[&[u32]], t: usize, (beta, depth): (usize, usize), keys: &mut Vec<u64>, p: &mut [f64]) {
        let depth = depth.min(self.tables.len() - 1);
        context_keys(features, t, depth, keys);
        p.fill(1.0 / self.values as f64);
        let mut applied: Option<u64> = None;
        for (table, &key) in self.tables.iter().zip(keys.iter()) {
            let Some(c) = table.get(key) else { break };
            let total: u64 = c.iter().map(|&x| u64::from(x)).sum();
            if applied != Some(total) {
                Self::lean(p, c, total, BETAS[beta]);
                applied = Some(total);
            }
        }
    }
}

/// The fewest bits a grid holds, with the weight and the depth reading them.
fn fewest(bits: &[Vec<f64>]) -> (f64, usize, usize) {
    let mut best = (f64::INFINITY, 0, 0);
    for (b, row) in bits.iter().enumerate() {
        for (d, &x) in row.iter().enumerate() {
            if x < best.0 {
                best = (x, b, d);
            }
        }
    }
    best
}

/// `work` done for every job from `0..jobs` across the machine's cores, each
/// result in its job's place. The jobs are handed out one at a time from a
/// shared counter, so a slow job holds up no other.
fn in_parallel<T: Send, F: Fn(usize) -> T + Sync>(jobs: usize, work: F) -> Vec<T> {
    let cores = std::thread::available_parallelism().expect("the machine reports its cores").get();
    let next = AtomicUsize::new(0);
    let mut placed: Vec<Option<T>> = (0..jobs).map(|_| None).collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..cores.min(jobs).max(1))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let job = next.fetch_add(1, Ordering::Relaxed);
                        if job >= jobs {
                            break;
                        }
                        done.push((job, work(job)));
                    }
                    done
                })
            })
            .collect();
        for worker in workers {
            for (job, result) in worker.join().expect("a worker finishes its jobs") {
                placed[job] = Some(result);
            }
        }
    });
    placed.into_iter().map(|result| result.expect("every job ran")).collect()
}

/// A chain grown one link at a time, fit on the inner blocks and read on the
/// check: each inner and check position's key at the depth reached, and at
/// each check position its prediction under every weight, the positions of the
/// last context leaned on, and whether its chain has already ended. A link is
/// tried against it by building that link's table alone.
struct Growing {
    values: usize,
    inner_keys: Vec<u64>,
    check_keys: Vec<u64>,
    /// `p[j * BETAS.len() * values + b * values + v]`.
    p: Vec<f64>,
    applied: Vec<Option<u64>>,
    ended: Vec<bool>,
}

impl Growing {
    fn start(values: usize, split: &Split) -> Growing {
        let n = split.check.len();
        Growing {
            values,
            inner_keys: vec![CHAIN_ROOT; split.inner.len()],
            check_keys: vec![CHAIN_ROOT; n],
            p: vec![1.0 / values as f64; n * BETAS.len() * values],
            applied: vec![None; n],
            ended: vec![false; n],
        }
    }

    /// The counts the next link makes over the inner blocks: the chain's own
    /// contexts again when `link` is none, or each extended by `link`.
    fn table(&self, reading: &[u16], offset: usize, split: &Split, link: Option<&[u32]>) -> Counts {
        let mut table = Counts::new(self.values);
        for (&key, &t) in self.inner_keys.iter().zip(&split.inner) {
            let key = link.map_or(key, |f| extend(key, f[t]));
            table.add(key, usize::from(reading[t - offset]));
        }
        table
    }

    /// The context a check position reaches through `link`, and its counts
    /// where they are new evidence: none where the chain has ended there,
    /// where the context was never seen, or where it holds the positions of
    /// the context leaned on last.
    fn reach<'t>(&self, j: usize, t: usize, link: Option<&[u32]>, table: &'t Counts) -> (u64, Option<(&'t [u32], u64)>) {
        let key = link.map_or(self.check_keys[j], |f| extend(self.check_keys[j], f[t]));
        if self.ended[j] {
            return (key, None);
        }
        let found = table
            .get(key)
            .map(|c| (c, c.iter().map(|&x| u64::from(x)).sum::<u64>()))
            .filter(|&(_, total)| self.applied[j] != Some(total));
        (key, found)
    }

    /// The mean bits a check position takes at each weight with `link` added.
    fn bits_with(&self, reading: &[u16], offset: usize, split: &Split, link: Option<&[u32]>, table: &Counts) -> Vec<f64> {
        let per = BETAS.len() * self.values;
        let mut bits = vec![0.0f64; BETAS.len()];
        let mut q = vec![0.0f64; self.values];
        for (j, &t) in split.check.iter().enumerate() {
            let value = usize::from(reading[t - offset]);
            let (_, found) = self.reach(j, t, link, table);
            for ((b, &beta), cell) in BETAS.iter().enumerate().zip(bits.iter_mut()) {
                let p = &self.p[j * per + b * self.values..j * per + (b + 1) * self.values];
                match found {
                    Some((c, total)) => {
                        q.copy_from_slice(p);
                        Model::lean(&mut q, c, total, beta);
                        *cell -= q[value].log2();
                    }
                    None => *cell -= p[value].log2(),
                }
            }
        }
        let n = split.check.len().max(1) as f64;
        for cell in &mut bits {
            *cell /= n;
        }
        bits
    }

    /// The chain with `link` added: its keys extended and its predictions
    /// leaned on the new counts.
    fn grow(&mut self, split: &Split, link: Option<&[u32]>, table: &Counts) {
        let per = BETAS.len() * self.values;
        for (j, &t) in split.check.iter().enumerate() {
            let ended = self.ended[j];
            let (key, found) = self.reach(j, t, link, table);
            if ended {
                continue;
            }
            self.check_keys[j] = key;
            match found {
                Some((c, total)) => {
                    for (b, &beta) in BETAS.iter().enumerate() {
                        Model::lean(&mut self.p[j * per + b * self.values..j * per + (b + 1) * self.values], c, total, beta);
                    }
                    self.applied[j] = Some(total);
                }
                None if table.get(key).is_none() => self.ended[j] = true,
                None => {}
            }
        }
        if let Some(f) = link {
            for (key, &t) in self.inner_keys.iter_mut().zip(&split.inner) {
                *key = extend(*key, f[t]);
            }
        }
    }
}

/// The order the chain reads the axes in, and the axis that alone reads the
/// check blocks best. Each next link is the axis that, with the links before
/// it, reads the check best when fit on the inner blocks, for as long as one
/// reads it better than the chain without it; the rest follow in the order
/// each alone reads the check best.
fn chain_order(
    reading: &[u16],
    offset: usize,
    values: usize,
    axes: &[(&'static str, &DenseAxis)],
    split: &Split,
) -> (Vec<usize>, usize) {
    let ids = |i: usize| axes[i].1.ids.as_slice();
    let fewest_weight = |bits: Vec<f64>| bits.into_iter().fold(f64::INFINITY, f64::min);
    let mut growing = Growing::start(values, split);
    let root = growing.table(reading, offset, split, None);
    let mut current = fewest_weight(growing.bits_with(reading, offset, split, None, &root));
    growing.grow(split, None, &root);
    let mut chosen: Vec<usize> = Vec::new();
    let mut single: Vec<(f64, usize)> = Vec::new();
    loop {
        let candidates: Vec<usize> = (0..axes.len()).filter(|i| !chosen.contains(i)).collect();
        if candidates.is_empty() {
            break;
        }
        let tried: Vec<f64> = in_parallel(candidates.len(), |c| {
            let link = Some(ids(candidates[c]));
            let table = growing.table(reading, offset, split, link);
            fewest_weight(growing.bits_with(reading, offset, split, link, &table))
        });
        if chosen.is_empty() {
            single = tried.iter().zip(&candidates).map(|(&bits, &i)| (bits, i)).collect();
        }
        let (bits, i) = tried
            .iter()
            .zip(&candidates)
            .map(|(&bits, &i)| (bits, i))
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .expect("a candidate to try");
        if bits >= current {
            break;
        }
        chosen.push(i);
        current = bits;
        let link = Some(ids(i));
        let table = growing.table(reading, offset, split, link);
        growing.grow(split, link, &table);
    }
    single.sort_by(|x, y| x.0.total_cmp(&y.0));
    let alone = single[0].1;
    let rest: Vec<usize> = single.iter().map(|&(_, i)| i).filter(|i| !chosen.contains(i)).collect();
    chosen.extend(rest);
    (chosen, alone)
}

fn stretch(p: f64) -> f64 {
    let p = p.clamp(1e-4, 1.0 - 1e-4);
    (p / (1.0 - p)).ln()
}

fn squash(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

/// An axis's values renumbered densely from zero, and how many there are.
struct DenseAxis {
    ids: Vec<u32>,
    count: usize,
}

fn dense_axis(values: &[u32]) -> DenseAxis {
    let mut index: HashMap<u32, u32> = HashMap::new();
    let ids = values
        .iter()
        .map(|&v| {
            let next = u32::try_from(index.len()).expect("an axis's values fit a count");
            *index.entry(v).or_insert(next)
        })
        .collect();
    DenseAxis { ids, count: index.len() }
}

/// The bit tree over a reading's values: node 1 is the root, node `2k + b`
/// follows node `k` on bit `b`, and a value's path is its bits from the most
/// significant.
struct BitTree {
    bits: u32,
}

impl BitTree {
    fn for_values(values: usize) -> BitTree {
        BitTree { bits: usize::BITS - (values.max(2) - 1).leading_zeros() }
    }

    fn nodes(&self) -> usize {
        1 << self.bits
    }

    /// The node and bit at each step of a value's path.
    fn path(&self, value: usize) -> impl Iterator<Item = (usize, usize)> {
        let bits = self.bits;
        (0..bits).scan(1usize, move |node, j| {
            let b = (value >> (bits - 1 - j)) & 1;
            let here = *node;
            *node = 2 * here + b;
            Some((here, b))
        })
    }

    /// The values under `node`: where they start, where its one-branch starts,
    /// and where they end.
    fn span(&self, node: usize) -> (usize, usize, usize) {
        let level = usize::BITS - 1 - node.leading_zeros();
        let size = 1usize << (self.bits - level);
        let lo = (node - (1 << level)) * size;
        (lo, lo + size / 2, lo + size)
    }
}

/// What the mixer reads besides the chain: each axis's own prediction of each
/// node's bit, and the reading's prior, as stretched probabilities counted at
/// the blocks the experts were fit on, zero where an axis's value never reached
/// the node.
struct Experts<'a> {
    reading: &'a [u16],
    offset: usize,
    tree: BitTree,
    axes: &'a [&'a DenseAxis],
    tables: Vec<Vec<f32>>,
    prior: Vec<f32>,
}

impl<'a> Experts<'a> {
    fn new(reading: &'a [u16], offset: usize, values: usize, axes: &'a [&'a DenseAxis], at: &[usize]) -> Experts<'a> {
        let tree = BitTree::for_values(values);
        let nodes = tree.nodes();
        let table = |axis: Option<&DenseAxis>| -> Vec<f32> {
            let mut counts = vec![[0u32; 2]; axis.map_or(1, |a| a.count) * nodes];
            for &t in at {
                let base = axis.map_or(0, |a| a.ids[t] as usize) * nodes;
                for (node, b) in tree.path(usize::from(reading[t - offset])) {
                    counts[base + node][b] += 1;
                }
            }
            counts
                .iter()
                .map(|&[n0, n1]| {
                    if n0 + n1 == 0 { 0.0 } else { stretch((f64::from(n1) + 0.5) / (f64::from(n0 + n1) + 1.0)) as f32 }
                })
                .collect()
        };
        let tables: Vec<Vec<f32>> = axes.iter().map(|&a| table(Some(a))).collect();
        let prior = table(None);
        Experts { reading, offset, tree, axes, tables, prior }
    }
}

/// The experts and the chain a mixer reads at one stage, fit on the same blocks.
struct Stage<'a> {
    experts: Experts<'a>,
    chain: &'a Model,
}

/// One walk over `at` with the mixer: the mean bits a position takes, and when
/// a rate is given, the weights learned at it along the way. A node's inputs are
/// each axis's prediction, the prior's, the chain's at `setting`, and a bias.
fn mix_pass(
    stage: &Stage,
    features: &[&[u32]],
    setting: (usize, usize),
    weights: &mut [f64],
    rate: Option<f64>,
    at: &[usize],
) -> f64 {
    let experts = &stage.experts;
    let nodes = experts.tree.nodes();
    let k = experts.tables.len() + 3;
    let values = stage.chain.values;
    let mut keys = Vec::new();
    let mut p = vec![0.0f64; nodes];
    let mut cumulative = vec![0.0f64; nodes + 1];
    let mut x = vec![0.0f64; k];
    let mut bits = 0.0;
    for &t in at {
        stage.chain.predict(features, t, setting, &mut keys, &mut p[..values]);
        let mut running = 0.0;
        for (slot, &q) in cumulative[1..].iter_mut().zip(&p) {
            running += q;
            *slot = running;
        }
        for (node, b) in experts.tree.path(usize::from(experts.reading[t - experts.offset])) {
            for ((xi, table), axis) in x.iter_mut().zip(&experts.tables).zip(experts.axes) {
                *xi = f64::from(table[axis.ids[t] as usize * nodes + node]);
            }
            let (lo, mid, hi) = experts.tree.span(node);
            let under = cumulative[hi] - cumulative[lo];
            x[k - 3] = f64::from(experts.prior[node]);
            x[k - 2] = stretch(if under > 0.0 { (cumulative[hi] - cumulative[mid]) / under } else { 0.5 });
            x[k - 1] = 1.0;
            let w = &mut weights[node * k..(node + 1) * k];
            let z: f64 = w.iter().zip(&x).map(|(wi, xi)| wi * xi).sum();
            let one = squash(z).clamp(1e-6, 1.0 - 1e-6);
            bits -= if b == 1 { one.log2() } else { (1.0 - one).log2() };
            if let Some(rate) = rate {
                let err = b as f64 - one;
                for (wi, &xi) in w.iter_mut().zip(&x) {
                    *wi += rate * err * xi;
                }
            }
        }
    }
    bits / at.len().max(1) as f64
}

/// The weights a mixer starts from: the chain's prediction taken whole and every
/// other input at nothing, so a mixer that learns nothing reads as the chain.
fn start_weights(nodes: usize, k: usize) -> Vec<f64> {
    let mut w = vec![0.0; nodes * k];
    for node in w.chunks_mut(k) {
        node[k - 2] = 1.0;
    }
    w
}

/// The fewest bits the mixer leaves on the score blocks. It learns on the check
/// blocks from the stage fit on the inner ones, and after each pass at each rate
/// reads the score blocks from the stage refit on every fit block.
fn mixed_bits(learn: &Stage, read: &Stage, features: &[&[u32]], setting: (usize, usize), split: &Split) -> f64 {
    let nodes = learn.experts.tree.nodes();
    let k = learn.experts.tables.len() + 3;
    let mut best = f64::INFINITY;
    for &rate in &RATES {
        let mut w = start_weights(nodes, k);
        for _ in 0..MAX_PASSES {
            mix_pass(learn, features, setting, &mut w, Some(rate), &split.check);
            best = best.min(mix_pass(read, features, setting, &mut w, None, &split.score));
        }
    }
    best
}

/// How much of a reading the readings trex already has leave unexplained on
/// the score blocks: its bits alone; given the axis that alone explains most;
/// given every axis, through the chain, which keeps `kept` links of `chain`, and
/// through the mixer.
struct Uniqueness {
    alone: f64,
    given_best: f64,
    given_chain: f64,
    given_mixed: f64,
    chain: Vec<&'static str>,
    kept: usize,
}

impl Uniqueness {
    /// The fewer bits either model leaves: what trex's readings leave.
    fn given_all(&self) -> f64 {
        self.given_chain.min(self.given_mixed)
    }

    /// The share of the reading's bits the given bits leave explained.
    fn explained(&self, given: f64) -> f64 {
        if self.alone > 0.0 { 1.0 - given / self.alone } else { 0.0 }
    }
}

fn uniqueness(
    reading: &[u16],
    offset: usize,
    values: usize,
    axes: &[(&'static str, &DenseAxis)],
    split: &Split,
) -> Uniqueness {
    let scored = |features: &[&[u32]]| {
        fewest(&Model::fit(reading, offset, values, features, &split.fit).bits(reading, offset, features, &split.score))
    };
    let (alone, _, _) = scored(&[]);
    let (order, single) = chain_order(reading, offset, values, axes, split);
    let (given_best, _, _) = scored(&[axes[single].1.ids.as_slice()]);
    let features: Vec<&[u32]> = order.iter().map(|&i| axes[i].1.ids.as_slice()).collect();
    let full = Model::fit(reading, offset, values, &features, &split.fit);
    let (given_chain, _, kept) = fewest(&full.bits(reading, offset, &features, &split.score));
    let inner = Model::fit(reading, offset, values, &features, &split.inner);
    let (_, beta, depth) = fewest(&inner.bits(reading, offset, &features, &split.check));
    let dense: Vec<&DenseAxis> = axes.iter().map(|&(_, a)| a).collect();
    let learn = Stage { experts: Experts::new(reading, offset, values, &dense, &split.inner), chain: &inner };
    let read = Stage { experts: Experts::new(reading, offset, values, &dense, &split.fit), chain: &full };
    let given_mixed = mixed_bits(&learn, &read, &features, (beta, depth), split);
    Uniqueness {
        alone,
        given_best,
        given_chain,
        given_mixed,
        chain: order.iter().map(|&i| axes[i].0).collect(),
        kept,
    }
}

/// The hits the `k` positions a signal ranks first are expected to make, where
/// a signal ranks its smallest values first and ties at the cut-off are broken
/// at random.
fn expected_hits(ranked: &[(f64, bool)], k: usize) -> f64 {
    let k = k.min(ranked.len());
    if k == 0 {
        return 0.0;
    }
    let mut sorted = ranked.to_vec();
    sorted.sort_by(|x, y| x.0.total_cmp(&y.0));
    let edge = sorted[k - 1].0;
    let (mut below, mut below_hits, mut tied, mut tied_hits) = (0usize, 0usize, 0usize, 0usize);
    for &(v, hit) in &sorted {
        match v.total_cmp(&edge) {
            std::cmp::Ordering::Less => {
                below += 1;
                below_hits += usize::from(hit);
            }
            std::cmp::Ordering::Equal => {
                tied += 1;
                tied_hits += usize::from(hit);
            }
            std::cmp::Ordering::Greater => break,
        }
    }
    below_hits as f64 + (k - below) as f64 * tied_hits as f64 / tied as f64
}

/// Entropy in bits of the counts of `n` positions.
fn entropy_bits(counts: impl Iterator<Item = u32>, n: f64) -> f64 {
    counts.map(|c| f64::from(c) / n).map(|p| p * (1.0 / p).log2()).sum::<f64>()
}

/// Entropy in bits of a labeling, and of it given a second one.
fn entropies(x: &[u16], given: &[u32]) -> (f64, f64) {
    let n = x.len().max(1) as f64;
    let mut px: HashMap<u16, u32> = HashMap::new();
    let mut pg: HashMap<u32, u32> = HashMap::new();
    let mut joint: HashMap<(u16, u32), u32> = HashMap::new();
    for (&a, &g) in x.iter().zip(given) {
        *px.entry(a).or_insert(0) += 1;
        *pg.entry(g).or_insert(0) += 1;
        *joint.entry((a, g)).or_insert(0) += 1;
    }
    let hx = entropy_bits(px.values().copied(), n);
    let hg = entropy_bits(pg.values().copied(), n);
    let hj = entropy_bits(joint.values().copied(), n);
    (hx, hj - hg)
}

/// Normalized mutual information between two labelings of the same positions.
fn nmi(x: &[u16], y: &[u16]) -> f64 {
    let y32: Vec<u32> = y.iter().map(|&v| u32::from(v)).collect();
    let (hx, hx_given_y) = entropies(x, &y32);
    let mut py: HashMap<u16, u32> = HashMap::new();
    for &v in y {
        *py.entry(v).or_insert(0) += 1;
    }
    let hy = entropy_bits(py.values().copied(), y.len().max(1) as f64);
    // Mutual information is non-negative; the difference of two equal sums
    // can round a hair below zero.
    let shared = (hx - hx_given_y).max(0.0);
    if hx + hy <= 0.0 { 0.0 } else { 2.0 * shared / (hx + hy) }
}

fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().max(1) as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (&a, &b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
        syy += (b - my) * (b - my);
    }
    if sxx <= 0.0 || syy <= 0.0 { 0.0 } else { sxy / (sxx * syy).sqrt() }
}

/// Labels renumbered densely from zero, with how many there are.
fn dense(labels: &[u32]) -> (Vec<u16>, usize) {
    let mut index: HashMap<u32, u16> = HashMap::new();
    let out = labels
        .iter()
        .map(|&l| {
            let next = u16::try_from(index.len()).expect("a partition of few labels");
            *index.entry(l).or_insert(next)
        })
        .collect();
    (out, index.len())
}

/// trex's windowed readings, cut for the held-out models: at each token index
/// the rolling window's folds and the related context's, and at each unit the
/// unit-rung window's folds and the unit's own.
struct Context {
    at_token: Vec<(&'static str, Vec<u32>)>,
    at_unit: Vec<(&'static str, Vec<u32>)>,
}

/// A window's fold as the numbers the held-out models read: scale, depth,
/// entropy, novelty, observer dependence, seam cuts and reversals.
fn folds(w: &trex::context::WindowProfile) -> [f64; 7] {
    [
        f64::from(w.magnitude.mean()),
        f64::from(w.stress.mean_depth()),
        f64::from(w.spectral.mean_entropy()),
        share(w.echo.novel, w.echo.count),
        if w.observation.count == 0 {
            0.0
        } else {
            f64::from(w.observation.disagreement_sum) / f64::from(w.observation.count)
        },
        f64::from(w.seam.cuts),
        f64::from(w.flow.reversals),
    ]
}

/// Each fold of every window, cut at its own quantiles, under `names`.
fn fold_readings(windows: &[trex::context::WindowProfile], names: [&'static str; 7]) -> Vec<(&'static str, Vec<u32>)> {
    let all: Vec<[f64; 7]> = windows.iter().map(folds).collect();
    names
        .iter()
        .enumerate()
        .map(|(k, &name)| (name, quantiled(&all.iter().map(|f| f[k]).collect::<Vec<f64>>())))
        .collect()
}

fn context_of(bytes: &[u8], toks: &[Token]) -> Context {
    let window = trex::context::analyze(bytes);
    assert_eq!(window.at_token.len(), toks.len(), "the rolling window reads one fold a token");
    let spectral = trex::spectral::analyze(bytes);
    let stress = trex::stress::analyze(toks, bytes);
    let echo = trex::echo::analyze(toks, bytes);
    let observation = trex::observation::analyze(bytes);
    let seam = trex::seam::analyze(bytes);
    let flow = trex::flow::analyze(toks, bytes, trex::flow::Signal::Magnitude);
    let ctx = trex::profile::AxisCtx {
        spectral: Some(&spectral),
        stress: Some(&stress),
        echo: Some(&echo),
        observation: Some(&observation),
        seam: Some(&seam),
        flow: Some(&flow),
        ..trex::profile::AxisCtx::new(bytes)
    };
    let related =
        trex::context::relate(toks, &ctx, &spectral.boundaries, &echo, trex::context::record_period(toks, bytes));
    assert_eq!(related.at_token.len(), toks.len(), "the related context reads one fold a token");
    assert_eq!(related.value_history.len(), toks.len(), "the value history reads one fold a token");
    let mut at_token = fold_readings(&window.at_token, WINDOW_NAMES);
    let r = &related.at_token;
    let scale_of = |w: &trex::context::WindowProfile| f64::from(w.magnitude.mean());
    at_token.push(("earlier occurrences", r.iter().map(|x| octave(x.echoing.magnitude.count)).collect()));
    at_token.push(("tokens since the regime changed", r.iter().map(|x| octave(x.regime.magnitude.count)).collect()));
    at_token.push(("phase scale", quantiled(&r.iter().map(|x| scale_of(&x.phase)).collect::<Vec<f64>>())));
    at_token.push((
        "phase novelty",
        quantiled(&r.iter().map(|x| share(x.phase.echo.novel, x.phase.echo.count)).collect::<Vec<f64>>()),
    ));
    at_token.push(("enclosing heads' scale", quantiled(&r.iter().map(|x| scale_of(&x.enclosing)).collect::<Vec<f64>>())));
    at_token.push(("values bound before", related.value_history.iter().map(|v| octave(v.magnitude.count)).collect()));
    let mut at_unit = fold_readings(&window.at_unit, UNIT_WINDOW_NAMES);
    at_unit.extend(fold_readings(&window.of_unit, OWN_NAMES));
    Context { at_token, at_unit }
}

/// One grain of a file: its units as dense types, the readings trex already has
/// at each unit, and where the grain above's units begin.
struct Grain {
    name: &'static str,
    seq: Vec<u32>,
    types: usize,
    names: Vec<String>,
    /// The readings trex already has, one a unit; the first is the unit's type.
    axes: Vec<(&'static str, DenseAxis)>,
    /// The partitions trex already groups units by, one label a unit; the first
    /// is fixed by the unit's type.
    partitions: Vec<(&'static str, Vec<u32>)>,
    /// Whether each unit opens a unit of the grain above.
    above: Vec<bool>,
    above_name: &'static str,
    /// Where trex's own seam cuts this grain, and which seam that is.
    seam: Vec<bool>,
    seam_name: &'static str,
    /// trex's own signals for where a cut is, one value a unit, each with
    /// whether it ranks a cut by its low values first.
    cut_signals: Vec<(&'static str, Vec<f64>, bool)>,
    /// What the grain's construction found worth saying before it is read.
    notes: Vec<String>,
    reach: usize,
}

/// Each reading renumbered densely, so a reading is held once.
fn dense_axes(axes: Vec<(&'static str, Vec<u32>)>) -> Vec<(&'static str, DenseAxis)> {
    axes.into_iter().map(|(name, values)| (name, dense_axis(&values))).collect()
}

/// Each key's type: its rank by frequency among the `TOP_TYPES` most frequent,
/// the rest sharing the type after them; with each type's name.
fn dense_types(keys: &[String]) -> (Vec<u32>, usize, Vec<String>) {
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for key in keys {
        *counts.entry(key.as_str()).or_insert(0) += 1;
    }
    let mut ranked: Vec<(&str, u32)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let kept = ranked.len().min(TOP_TYPES);
    let index: HashMap<&str, u32> = ranked[..kept]
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (*k, u32::try_from(i).expect("under TOP_TYPES")))
        .collect();
    let other = u32::try_from(kept).expect("under TOP_TYPES");
    let seq = keys.iter().map(|k| index.get(k.as_str()).copied().unwrap_or(other)).collect();
    let mut names: Vec<String> = ranked[..kept].iter().map(|(k, _)| (*k).to_string()).collect();
    let types = if ranked.len() > kept {
        names.push("(the rest)".to_string());
        kept + 1
    } else {
        kept
    };
    (seq, types, names)
}

fn bin01(v: f32, bins: u32) -> u32 {
    ((v.clamp(0.0, 1.0) * bins as f32) as u32).min(bins - 1)
}

/// The class trex's byte readers group a byte by: letter, digit, whitespace,
/// punctuation, control, or beyond ASCII.
fn byte_class(b: u8) -> u32 {
    if b.is_ascii_alphabetic() {
        0
    } else if b.is_ascii_digit() {
        1
    } else if b.is_ascii_whitespace() {
        2
    } else if b.is_ascii_punctuation() {
        3
    } else if b.is_ascii() {
        4
    } else {
        5
    }
}

fn byte_name(b: u8) -> String {
    match b {
        b'\n' => "\\n".to_string(),
        b'\t' => "\\t".to_string(),
        b'\r' => "\\r".to_string(),
        b' ' => "' '".to_string(),
        b if b.is_ascii_graphic() => char::from(b).to_string(),
        _ => format!("\\x{b:02x}"),
    }
}

fn widened(values: &[f32]) -> Vec<f64> {
    values.iter().map(|&x| f64::from(x)).collect()
}

/// The spectral bands at a frame, each cut into four.
fn band_readings(frame: &trex::spectral::SpectralFrame) -> [u32; BAND_COUNT] {
    BANDS.map(|(d, c, _)| bin01(frame.bands[d * trex::spectral::N_CLASSES + c], 4))
}

fn byte_grain(bytes: &[u8], toks: &[Token], context: &Context) -> Grain {
    let n = bytes.len();
    let seq: Vec<u32> = bytes.iter().map(|&b| u32::from(b)).collect();
    // The token over each byte, its kind, and the byte's place in it.
    let mut over = vec![usize::MAX; n];
    let mut kind = vec![u32::MAX; n];
    let mut place = vec![0u32; n];
    let mut above = vec![false; n];
    for (j, t) in toks.iter().enumerate() {
        let s = t.start().min(n);
        let e = t.end().min(n).max(s);
        if s < n {
            above[s] = true;
        }
        let len = e - s;
        for (i, ((k, p), o)) in kind[s..e].iter_mut().zip(&mut place[s..e]).zip(&mut over[s..e]).enumerate() {
            *k = t.kind.code();
            *p = if len == 1 {
                3
            } else if i == 0 {
                0
            } else if i + 1 == len {
                2
            } else {
                1
            };
            *o = j;
        }
    }
    let mut notes = Vec::new();
    let uncovered = over.iter().filter(|&&j| j == usize::MAX).count();
    if uncovered > 0 {
        notes.push(format!("{uncovered} bytes are in no token and read the token readings as none"));
    }

    let echo = trex::echo::analyze(toks, bytes);
    let shape = trex::shape::analyze(toks, bytes);
    let stress = trex::stress::analyze(toks, bytes);
    let entangled = trex::entanglement::analyze(toks, bytes).profile;
    for (what, len) in [
        ("echo", echo.frames.len()),
        ("shape", shape.frames.len()),
        ("stress", stress.frames.len()),
        ("entanglement", entangled.len()),
    ] {
        assert_eq!(len, toks.len(), "{what} reads one frame a token");
    }
    let echoed: Vec<u32> =
        echo.frames.iter().map(|f| if f.novel() { 1 } else if f.echoed() { 2 } else { 0 }).collect();
    let shape_class: Vec<u32> = shape.frames.iter().map(|f| f.class).collect();
    let depth: Vec<u32> = stress.frames.iter().map(|f| u32::from(f.depth)).collect();
    let crossing = quantiled(&entangled.iter().map(|&c| c as f64).collect::<Vec<f64>>());
    let token_reading = |per_token: &[u32]| -> Vec<u32> {
        over.iter().map(|&j| if j == usize::MAX { u32::MAX } else { per_token[j] }).collect()
    };

    let seam_field = trex::seam::analyze(bytes);
    let pure = trex::seam::SeamConfig { lexer_fusion: false, ..trex::seam::SeamConfig::default() };
    let seam_alone = trex::seam::analyze_with(bytes, &pure);
    for (what, len) in [
        ("the seam's strength", seam_field.boundary.len()),
        ("the forward branching entropy", seam_field.fwd_entropy.len()),
        ("the backward branching entropy", seam_field.bwd_entropy.len()),
        ("the seam's strength without the lexer", seam_alone.boundary.len()),
    ] {
        assert_eq!(len, n, "{what} is read at every byte");
    }
    let cuts_at = |cuts: &[usize]| -> Vec<bool> {
        let mut at = vec![false; n];
        for &c in cuts {
            if c < n {
                at[c] = true;
            }
        }
        at
    };
    let spectral = trex::spectral::analyze(bytes);
    let (mut entropy, mut novelty, mut periodic) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    let mut bands: Vec<Vec<u32>> = (0..BAND_COUNT).map(|_| Vec::with_capacity(n)).collect();
    for i in 0..n {
        let frame = spectral.frame_at(i);
        entropy.push(bin01(frame.entropy, 8));
        novelty.push(bin01(frame.novelty, 4));
        periodic.push(bin01(frame.period_strength, 4));
        for (band, v) in bands.iter_mut().zip(band_readings(&frame)) {
            band.push(v);
        }
    }
    let observation = trex::observation::analyze(bytes);
    assert_eq!(observation.frames.len(), n, "the observer dependence is read at every byte");
    let dependence: Vec<f64> = observation.frames.iter().map(|f| f64::from(f.disagreement)).collect();
    let before = |k: usize| -> Vec<u32> { (0..n).map(|i| if i >= k { seq[i - k] } else { 256 }).collect() };
    let mut axes = vec![
        ("the byte", seq.clone()),
        ("the byte before", before(1)),
        ("two bytes before", before(2)),
        ("token kind", kind),
        ("place in token", place),
        ("seam cut", cuts_at(&seam_field.cuts).into_iter().map(u32::from).collect()),
        ("seam strength", quantiled(&widened(&seam_field.boundary))),
        ("forward branching", quantiled(&widened(&seam_field.fwd_entropy))),
        ("backward branching", quantiled(&widened(&seam_field.bwd_entropy))),
        ("spectral entropy", entropy),
        ("spectral novelty", novelty),
        ("spectral period", periodic),
        ("observer dependence", quantiled(&dependence)),
        ("echo", token_reading(&echoed)),
        ("shape class", token_reading(&shape_class)),
        ("stress depth", token_reading(&depth)),
        ("entanglement", token_reading(&crossing)),
    ];
    axes.extend(BANDS.iter().zip(bands).map(|(&(_, _, name), band)| (name, band)));
    axes.extend(context.at_token.iter().map(|(name, per_token)| (*name, token_reading(per_token))));
    Grain {
        name: "bytes",
        seq,
        types: 256,
        names: (0..=255u8).map(byte_name).collect(),
        axes: dense_axes(axes),
        partitions: vec![("byte class", bytes.iter().map(|&b| byte_class(b)).collect())],
        above,
        above_name: "token starts",
        seam: cuts_at(&seam_alone.cuts),
        seam_name: "trex's byte seam without the lexer's cuts",
        cut_signals: vec![
            ("the seam's strength without the lexer, highest first", widened(&seam_alone.boundary), false),
            ("the observer dependence, highest first", dependence, false),
        ],
        notes,
        reach: BYTE_REACH,
    }
}

/// The rhythm trex's second mixer reads, a token at a time: a bank over the
/// kinds at r 0.95, its power's octile bin and its loudest band's group-delay
/// bin.
fn rhythm(sig: &[Token]) -> (Vec<u32>, Vec<u32>) {
    let r = 0.95f32;
    let codes: Vec<u32> = sig.iter().map(|t| t.kind.code()).collect();
    let mut kinds = codes.clone();
    kinds.sort_unstable();
    kinds.dedup();
    let periods = trex::resonator::default_periods(32);
    let memory = r / (1.0 - r);
    let mut bank = trex::resonator::Bank::new(kinds.len(), &periods, r);
    let (mut gd, mut pw) = (vec![0.0f32; periods.len()], vec![0.0f32; periods.len()]);
    let mut powers = Vec::with_capacity(codes.len());
    let mut ages = Vec::with_capacity(codes.len());
    for &c in &codes {
        bank.push(u32::try_from(kinds.partition_point(|&k| k < c)).expect("a kind's rank fits the bank"));
        bank.read_into(&mut gd, &mut pw);
        let total: f32 = pw.iter().sum();
        let age = pw.iter().zip(&gd).max_by(|a, b| a.0.total_cmp(b.0)).map_or(0.0, |(_, &g)| g);
        ages.push(((age / memory * 8.0) as u32).min(7));
        powers.push(f64::from((total + f32::EPSILON).ln()));
    }
    (quantiled(&powers), ages)
}

fn token_grain(bytes: &[u8], toks: &[Token], context: &Context) -> Grain {
    let sig_at: Vec<usize> = (0..toks.len()).filter(|&j| toks[j].is_significant()).collect();
    let sig: Vec<Token> = sig_at.iter().map(|&j| toks[j]).collect();
    let n = sig.len();
    let keys: Vec<String> = sig
        .iter()
        .map(|t| match t.kind {
            TokenKind::Punct | TokenKind::Other => {
                let text: String = String::from_utf8_lossy(&bytes[t.span()]).chars().take(3).collect();
                format!("{:?} {text}", t.kind)
            }
            kind => format!("{kind:?}"),
        })
        .collect();
    let (seq, types, names) = dense_types(&keys);
    let before = |k: usize| -> Vec<u32> { (0..n).map(|i| if i >= k { seq[i - k] } else { u32::MAX }).collect() };
    let per_sig = |per_token: &[u32]| -> Vec<u32> { sig_at.iter().map(|&j| per_token[j]).collect() };
    let per_sig_wide = |per_token: &[f64]| -> Vec<f64> { sig_at.iter().map(|&j| per_token[j]).collect() };

    let period = trex::context::record_period(toks, bytes).map_or(0, usize::from);
    let column: Vec<u32> = (0..n)
        .map(|i| if period > 0 { u32::try_from(i % period).expect("a column under the period") } else { 0 })
        .collect();

    let echo = trex::echo::analyze(toks, bytes);
    let shape = trex::shape::analyze(toks, bytes);
    let stress = trex::stress::analyze(toks, bytes);
    let magnitude = trex::magnitude::analyze(toks, bytes);
    let relation = trex::relation::analyze(toks, bytes);
    let entangled = trex::entanglement::analyze(toks, bytes).profile;
    let flow = trex::flow::analyze(toks, bytes, trex::flow::Signal::Magnitude);
    let action = trex::action::analyze(toks, bytes).lagrangian;
    for (what, len) in [
        ("echo", echo.frames.len()),
        ("shape", shape.frames.len()),
        ("stress", stress.frames.len()),
        ("magnitude", magnitude.frames.len()),
        ("relation", relation.frames.len()),
        ("entanglement", entangled.len()),
        ("flow", flow.frames.len()),
        ("action", action.len()),
    ] {
        assert_eq!(len, toks.len(), "{what} reads one frame a token");
    }
    let echoed =
        per_sig(&echo.frames.iter().map(|f| if f.novel() { 1 } else if f.echoed() { 2 } else { 0 }).collect::<Vec<u32>>());
    let lag = per_sig(&echo.frames.iter().map(|f| f.back_lag.map_or(0, |l| 1 + l.get().ilog2())).collect::<Vec<u32>>());
    let shape_class = per_sig(&shape.frames.iter().map(|f| f.class).collect::<Vec<u32>>());
    let shape_novelty = per_sig(&shape.frames.iter().map(|f| bin01(f.novelty, 4)).collect::<Vec<u32>>());
    let shape_period = per_sig(&shape.frames.iter().map(|f| u32::from(f.period)).collect::<Vec<u32>>());
    let depth = per_sig(&stress.frames.iter().map(|f| u32::from(f.depth)).collect::<Vec<u32>>());
    let bracket_strain = quantiled(&per_sig_wide(&stress.frames.iter().map(|f| f64::from(f.strain)).collect::<Vec<f64>>()));
    let bracket_load = quantiled(&per_sig_wide(&stress.frames.iter().map(|f| f64::from(f.load)).collect::<Vec<f64>>()));
    let scale = quantiled(&per_sig_wide(&magnitude.frames.iter().map(|f| f64::from(f.magnitude)).collect::<Vec<f64>>()));
    let head = per_sig(
        &relation
            .frames
            .iter()
            .enumerate()
            .map(|(j, f)| f.enclosure.last().map_or(0, |&h| 1 + j.saturating_sub(h).max(1).ilog2()))
            .collect::<Vec<u32>>(),
    );
    let crossing = per_sig_wide(&entangled.iter().map(|&c| c as f64).collect::<Vec<f64>>());
    let direction = per_sig(
        &flow
            .frames
            .iter()
            .map(|f| u32::try_from(i32::from(f.direction) + 1).expect("a direction of -1, 0 or 1"))
            .collect::<Vec<u32>>(),
    );
    let lagrangian = quantiled(&per_sig_wide(&action.iter().map(|&x| f64::from(x)).collect::<Vec<f64>>()));

    let spectral = trex::spectral::analyze(bytes);
    let mut entropy: Vec<u32> = Vec::with_capacity(n);
    let mut bands: Vec<Vec<u32>> = (0..BAND_COUNT).map(|_| Vec::with_capacity(n)).collect();
    for t in &sig {
        let frame = spectral.frame_at(t.start());
        entropy.push(bin01(frame.entropy, 8));
        for (band, v) in bands.iter_mut().zip(band_readings(&frame)) {
            band.push(v);
        }
    }
    let observation = trex::observation::analyze(bytes);
    let dependence: Vec<f64> = sig.iter().map(|t| f64::from(observation.disagreement_at(t.start()))).collect();
    let seam_field = trex::seam::analyze(bytes);
    let seam_strength: Vec<f64> = sig.iter().map(|t| f64::from(seam_field.boundary[t.start()])).collect();
    let seam_cuts: HashSet<usize> =
        trex::seam::analyze_tokens(toks, &trex::seam::SeamConfig::default()).into_iter().collect();
    let seam: Vec<bool> = sig.iter().map(|t| seam_cuts.contains(&t.start())).collect();

    let units = trex::supertoken::supertokens_from(toks, bytes);
    let mut role = vec![u32::MAX; n];
    let mut opens = vec![false; n];
    let mut u = 0usize;
    for ((t, r), o) in sig.iter().zip(role.iter_mut()).zip(opens.iter_mut()) {
        while u < units.len() && units[u].end <= t.start() {
            u += 1;
        }
        if u < units.len() && units[u].start <= t.start() {
            *r = units[u].role.code();
            *o = units[u].start == t.start();
        }
    }
    let (power, age) = rhythm(&sig);
    let kinds: Vec<u32> = sig.iter().map(|t| t.kind.code()).collect();
    let mut axes = vec![
        ("the token's class", seq.clone()),
        ("the class before", before(1)),
        ("two classes before", before(2)),
        ("record column", column),
        ("echo", echoed),
        ("echo lag", lag),
        ("shape class", shape_class.clone()),
        ("shape novelty", shape_novelty),
        ("shape period", shape_period),
        ("stress depth", depth),
        ("bracket strain", bracket_strain),
        ("bracket load", bracket_load),
        ("magnitude", scale),
        ("spectral entropy", entropy),
        ("seam cut", seam.iter().map(|&c| u32::from(c)).collect()),
        ("seam strength", quantiled(&seam_strength)),
        ("unit role", role),
        ("opens a unit", opens.iter().map(|&o| u32::from(o)).collect()),
        ("rhythm power", power),
        ("rhythm group delay", age),
        ("entanglement", quantiled(&crossing)),
        ("observer dependence", quantiled(&dependence)),
        ("flow of magnitude", direction),
        ("action", lagrangian),
        ("enclosing head", head),
    ];
    axes.extend(BANDS.iter().zip(bands).map(|(&(_, _, name), band)| (name, band)));
    axes.extend(context.at_token.iter().map(|(name, per_token)| (*name, per_sig(per_token))));
    Grain {
        name: "tokens",
        seq,
        types,
        names,
        axes: dense_axes(axes),
        partitions: vec![("token kind", kinds), ("shape class", shape_class)],
        above: opens,
        above_name: "supertoken starts",
        seam,
        seam_name: "trex's token seam",
        cut_signals: vec![
            ("entanglement, lowest first", crossing, true),
            ("the byte seam's strength, highest first", seam_strength, false),
            ("the observer dependence, highest first", dependence, false),
        ],
        notes: Vec::new(),
        reach: TOKEN_REACH,
    }
}

fn unit_grain(bytes: &[u8], toks: &[Token], context: &Context) -> Grain {
    let units = trex::supertoken::supertokens_from(toks, bytes);
    let sig: Vec<Token> = toks.iter().filter(|t| t.is_significant()).copied().collect();
    let n = units.len();
    let mut keys: Vec<String> = Vec::with_capacity(n);
    let mut length: Vec<u32> = Vec::with_capacity(n);
    let mut first: Vec<u32> = Vec::with_capacity(n);
    let mut i = 0usize;
    for unit in &units {
        while i < sig.len() && sig[i].start() < unit.start {
            i += 1;
        }
        let kinds: Vec<TokenKind> =
            sig[i..].iter().take_while(|t| t.start() < unit.end).map(|t| t.kind).collect();
        let shown: Vec<String> = kinds.iter().take(6).map(|k| format!("{k:?}")).collect();
        keys.push(format!("{}:{}", unit.role.label(), shown.join(",")));
        length.push(match kinds.len() {
            0..=1 => 0,
            2 => 1,
            3..=4 => 2,
            5..=8 => 3,
            _ => 4,
        });
        first.push(kinds.first().map_or(u32::MAX, |k| k.code()));
    }
    let mut key_count: HashMap<&str, u32> = HashMap::new();
    for key in &keys {
        *key_count.entry(key.as_str()).or_insert(0) += 1;
    }
    let recurs: Vec<u32> = keys.iter().map(|key| u32::from(key_count[key.as_str()] >= 2)).collect();
    let (seq, types, names) = dense_types(&keys);
    let roles: Vec<u32> = units.iter().map(|u| u.role.code()).collect();
    let role_before: Vec<u32> = (0..n).map(|u| if u >= 1 { roles[u - 1] } else { u32::MAX }).collect();
    let depth: Vec<u32> = units.iter().map(|u| u.depth).collect();
    let seam_cuts: HashSet<usize> =
        trex::seam::analyze_supertokens(&units, &trex::seam::SeamConfig::default()).into_iter().collect();
    let seam: Vec<bool> = units.iter().map(|u| seam_cuts.contains(&u.start)).collect();
    let above: Vec<bool> = (0..n)
        .map(|u| u == 0 || bytes[units[u - 1].end.min(units[u].start)..units[u].start].contains(&b'\n'))
        .collect();

    // The entanglement between units: the relation graph's edges that cross
    // from one unit to another, read at the cut before each unit's first token.
    let map = trex::relation::NodeMap::per_supertoken(&units, toks);
    let entangled = trex::entanglement::analyze_over(toks, bytes, &map).profile;
    let first_token: Vec<usize> = units.iter().map(|u| toks.partition_point(|t| t.start() < u.start)).collect();
    let off_token = units
        .iter()
        .zip(&first_token)
        .filter(|&(u, &j)| toks.get(j).is_none_or(|t| t.start() != u.start))
        .count();
    let mut notes = Vec::new();
    if off_token > 0 {
        notes.push(format!("{off_token} units start at no token's start; each is read at the next token"));
    }
    let crossing: Vec<f64> = first_token.iter().map(|&j| entangled.get(j).map_or(0.0, |&c| c as f64)).collect();
    for (name, values) in &context.at_unit {
        assert_eq!(values.len(), n, "{name} reads one value a unit");
    }
    let mut axes = vec![
        ("the unit's structure", seq.clone()),
        ("the role before", role_before),
        ("depth", depth),
        ("length", length),
        ("structure recurs", recurs),
        ("seam cut", seam.iter().map(|&c| u32::from(c)).collect()),
        ("the kind it opens with", first),
        ("entanglement", quantiled(&crossing)),
    ];
    axes.extend(context.at_unit.iter().map(|(name, values)| (*name, values.clone())));
    Grain {
        name: "supertokens",
        seq,
        types,
        names,
        axes: dense_axes(axes),
        partitions: vec![("role", roles)],
        above,
        above_name: "line starts",
        seam,
        seam_name: "trex's supertoken seam",
        cut_signals: vec![("entanglement between units, lowest first", crossing, true)],
        notes,
        reach: UNIT_REACH,
    }
}

/// The strongest bonds and the strongest repulsions the field holds.
fn print_extremes(f: &Field, b: &Bonds, names: &[String]) {
    let name = |t: usize| names.get(t).map_or("?", String::as_str);
    let mut pulls: Vec<(usize, usize, usize, f32)> = (0..f.types)
        .flat_map(|a| (0..f.types).map(move |c| (a, c)))
        .filter_map(|(a, c)| {
            let k = a * f.types + c;
            (b.gap[k] > 0).then_some((a, c, usize::from(b.gap[k]), b.z[k]))
        })
        .collect();
    pulls.sort_by(|x, y| y.3.total_cmp(&x.3));
    let shown: Vec<String> = pulls
        .iter()
        .take(6)
        .map(|&(a, c, d, z)| format!("{} -> {} at {d} (x{:.2}, z {z:.0})", name(a), name(c), f.lift(a, c, d)))
        .collect();
    println!("   strongest bonds: {}", shown.join("; "));
    let mut pushes: Vec<(usize, usize, usize, f64)> = Vec::new();
    for a in 0..f.types {
        for c in 0..f.types {
            let worst = (1..=f.reach)
                .filter(|&d| f.expected(a, c, d) >= MIN_EXPECTED)
                .map(|d| (d, f.z(a, c, d)))
                .min_by(|x, y| x.1.total_cmp(&y.1));
            if let Some((d, z)) = worst
                && z < -Z_CELL
            {
                pushes.push((a, c, d, z));
            }
        }
    }
    pushes.sort_by(|x, y| x.3.total_cmp(&y.3));
    let shown: Vec<String> = pushes
        .iter()
        .take(6)
        .map(|&(a, c, d, z)| format!("{} -| {} at {d} (x{:.2}, z {z:.0})", name(a), name(c), f.lift(a, c, d)))
        .collect();
    println!("   strongest repulsions: {}", shown.join("; "));
}

fn report(g: &Grain, block_count: usize) {
    let clock = Instant::now();
    let n = g.seq.len();
    println!("-- {} ({} types, {n} units; the field over gaps 1..={})", g.name, g.types, g.reach);
    for note in &g.notes {
        println!("   {note}");
    }
    // A block no longer than the reach holds no pair at the longest gap.
    if n / 2 / block_count <= g.reach {
        println!("   too few units for {block_count} blocks a half each longer than the field's reach");
        return;
    }
    let half = n / 2;
    let first = blocks(0..half, block_count);
    let evens: Vec<Range<usize>> = first.iter().step_by(2).cloned().collect();
    let odds: Vec<Range<usize>> = first.iter().skip(1).step_by(2).cloned().collect();
    let first_half = 0..half;
    let whole = std::slice::from_ref(&first_half);
    let mixed = shuffled(&g.seq[..half], SHUFFLE_SEED);
    let learn = |seq: &[u32], on: &[Range<usize>]| Field::learn(seq, on, g.types, g.reach);
    // The field on the whole first half, on its even and odd blocks, and the
    // same three over the first half shuffled.
    let fields = [
        learn(&g.seq, whole),
        learn(&g.seq, &evens),
        learn(&g.seq, &odds),
        learn(&mixed, whole),
        learn(&mixed, &evens),
        learn(&mixed, &odds),
    ];
    let (field, mixed_field) = (&fields[0], &fields[3]);

    let (up, down) = field.significant_cells();
    let (up_s, down_s) = mixed_field.significant_cells();
    println!(
        "   real: {up} attracting and {down} repelling cells past |z| {Z_CELL} (a shuffled copy: {up_s} and {down_s})"
    );
    let gaps: Vec<String> = [1usize, 2, 4, 8, 16, 32, 64]
        .into_iter()
        .filter(|&d| d <= g.reach)
        .map(|d| format!("d{d} {:.4} ({:.4})", field.mutual_information(d), mixed_field.mutual_information(d)))
        .collect();
    println!("   mutual information in bits, real (shuffled): {}", gaps.join("  "));
    let bonds_of = fields.each_ref().map(bonds);
    print_extremes(field, &bonds_of[0], &g.names);

    // The readings over the second half, from each of the six fields.
    let at = half..n;
    let split = Split::deal(at.clone(), block_count);
    let strains = fields.each_ref().map(|f| strain(&g.seq, f, at.clone()));
    let bindings = fields.each_ref().map(|f| binding(&g.seq, f, at.clone()));
    let reaches = bonds_of.each_ref().map(|b| bond_reach(&g.seq, b, g.reach, at.clone()));
    let classes = fields.each_ref().map(geometry);
    let geometries =
        classes.each_ref().map(|c| g.seq[at.clone()].iter().map(|&x| c[x as usize]).collect::<Vec<u16>>());

    let axes: Vec<(&'static str, &DenseAxis)> = g.axes.iter().map(|(name, a)| (*name, a)).collect();
    let strain_bins = binned(&strains[0], half, &split.fit);
    let binding_bins = binned(&bindings[0], half, &split.fit);
    let rows: [(&str, &[u16], usize, f64, f64); 3] = [
        (
            "strain",
            strain_bins.as_slice(),
            BINS,
            pearson(&strains[1], &strains[2]),
            pearson(&strains[4], &strains[5]),
        ),
        (
            "bond reach",
            reaches[0].as_slice(),
            REACH_CLASSES,
            nmi(&reaches[1], &reaches[2]),
            nmi(&reaches[4], &reaches[5]),
        ),
        (
            "binding",
            binding_bins.as_slice(),
            BINS,
            pearson(&bindings[1], &bindings[2]),
            pearson(&bindings[4], &bindings[5]),
        ),
    ];
    println!("   reading      bits alone  given trex's  unique  stable (shuffled)  the one axis that explains most");
    let mut read: Vec<(&str, Uniqueness)> = Vec::new();
    for (name, reading, values, stable, stable_shuffled) in rows {
        let u = uniqueness(reading, half, values, &axes, &split);
        println!(
            "   {name:<11} {:>10.3}  {:>12.3}  {:>5.0}%  {stable:>6.3} ({stable_shuffled:>7.3})  {} {:.0}%",
            u.alone,
            u.given_all(),
            100.0 * (1.0 - u.explained(u.given_all())),
            u.chain[0],
            100.0 * u.explained(u.given_best)
        );
        read.push((name, u));
    }
    for (name, u) in &read {
        println!(
            "   {name}: the chain keeps {} of {} readings and leaves {:.3} bits, the mixer of all {} leaves {:.3}; the chain: {}",
            u.kept,
            u.chain.len(),
            u.given_chain,
            u.chain.len(),
            u.given_mixed,
            u.chain[..u.kept].join(", ")
        );
    }
    // The models' two controls: a reading trex's readings fix, and one drawn at
    // random, over the same positions and blocks.
    let (fixed, fixed_values) = dense(&g.partitions[0].1[half..]);
    let fixed_u = uniqueness(&fixed, half, fixed_values, &axes, &split);
    let random = shuffled(&strain_bins, RANDOM_READING_SEED);
    let random_u = uniqueness(&random, half, BINS, &axes, &split);
    let fixed_explained = fixed_u.explained(fixed_u.given_all());
    let random_explained = random_u.explained(random_u.given_all());
    println!(
        "   the models explain {:.1}% of the {}'s {:.3} bits, which trex's readings fix, and {:.1}% of the {:.3} bits of a reading drawn at random",
        100.0 * fixed_explained,
        g.partitions[0].0,
        fixed_u.alone,
        100.0 * random_explained,
        random_u.alone
    );
    // The controls check the estimator only while they come apart the right way
    // round. A grain whose fixed reading never varies, or whose random reading
    // is drawn from a constant, leaves both shares as ratios of nothing.
    if fixed_explained <= random_explained {
        println!(
            "   THE CONTROLS DO NOT SEPARATE at this grain: the reading trex fixes reads no more explained than the random one, so the unique column above is not checked here"
        );
    }

    // The geometry against the partitions trex already groups units by.
    let (h_classes, _) = entropies(&geometries[0], &g.partitions[0].1[at.clone()]);
    println!(
        "   geometry: {h_classes:.3} bits of classes over the second half, stable {:.3} ({:.3} between fields learned shuffled)",
        nmi(&geometries[1], &geometries[2]),
        nmi(&geometries[4], &geometries[5])
    );
    for (name, labels) in &g.partitions {
        let (h, given) = entropies(&geometries[0], &labels[at.clone()]);
        let new = if h > 0.0 { given / h } else { 0.0 };
        println!("     given {name}, the classes still hold {given:.3} of their {h:.3} bits ({:.0}% new)", 100.0 * new);
    }
    let mut members: Vec<Vec<(f64, usize)>> = vec![Vec::new(); GEOMETRY_CLASSES + 1];
    for (t, &c) in classes[0].iter().enumerate() {
        let freq = field.frequency(t);
        if freq > 0.0 {
            members[usize::from(c)].push((freq, t));
        }
    }
    for (c, m) in members.iter_mut().enumerate().take(GEOMETRY_CLASSES) {
        if m.is_empty() {
            continue;
        }
        m.sort_by(|x, y| y.0.total_cmp(&x.0));
        let shown: Vec<&str> = m.iter().take(8).map(|&(_, t)| g.names[t].as_str()).collect();
        println!("     class {c:>2} ({} types): {}", m.len(), shown.join("  "));
    }
    let unplaced = members[GEOMETRY_CLASSES].len();
    if unplaced > 0 {
        println!("     {unplaced} types seen with no cell clearing chance take a class of their own");
    }

    // Binding against where the grain above's units begin, beside trex's own
    // signals for a cut at this grain and its seam.
    let scored = &split.score;
    let truth: Vec<bool> = scored.iter().map(|&t| g.above[t]).collect();
    let wanted = truth.iter().filter(|&&x| x).count();
    if wanted == 0 {
        println!("   no {} are in the scored blocks", g.above_name);
    } else {
        let ranked = |values: &[f64], offset: usize, low_first: bool| -> Vec<(f64, bool)> {
            scored
                .iter()
                .zip(&truth)
                .map(|(&t, &hit)| {
                    let v = values[t - offset];
                    (if low_first { v } else { -v }, hit)
                })
                .collect()
        };
        println!(
            "   finding {} ({wanted} in the scored blocks, chance {:.3}):",
            g.above_name,
            wanted as f64 / scored.len() as f64
        );
        println!("     {:<56} {:>8} {:>9} {:>7}", "cut signal", "cuts", "precision", "recall");
        let row = |name: &str, cuts: usize, hits: f64| {
            println!(
                "     {name:<56} {cuts:>8} {:>9.3} {:>7.3}",
                hits / cuts.max(1) as f64,
                hits / wanted as f64
            );
        };
        let binding_ranked = ranked(&bindings[0], half, true);
        row("binding, lowest first", wanted, expected_hits(&binding_ranked, wanted));
        row(
            "binding from a shuffled field, lowest first",
            wanted,
            expected_hits(&ranked(&bindings[3], half, true), wanted),
        );
        for (name, values, low_first) in &g.cut_signals {
            row(name, wanted, expected_hits(&ranked(values, 0, *low_first), wanted));
        }
        let seam_cuts = scored.iter().filter(|&&t| g.seam[t]).count();
        let seam_hits = scored.iter().filter(|&&t| g.seam[t] && g.above[t]).count();
        row(g.seam_name, seam_cuts, seam_hits as f64);
        row("binding at as many cuts as the seam makes", seam_cuts, expected_hits(&binding_ranked, seam_cuts));
    }
    eprintln!("   {} read in {:.1} s", g.name, clock.elapsed().as_secs_f64());
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut block_count = SPLIT_BLOCKS;
    if args.first().is_some_and(|a| a == "--blocks") {
        match args.get(1).map(|v| v.parse::<usize>()) {
            Some(Ok(b)) if b >= 4 && b.is_multiple_of(4) => block_count = b,
            Some(Ok(b)) => {
                eprintln!("--blocks takes a multiple of four, at least four, not {b}");
                std::process::exit(2);
            }
            Some(Err(e)) => {
                eprintln!("--blocks takes a count: {e}");
                std::process::exit(2);
            }
            None => {
                eprintln!("--blocks takes a count");
                std::process::exit(2);
            }
        }
        args.drain(..2);
    }
    if args.is_empty() {
        eprintln!("usage: what_gravity_and_repulsion_carry [--blocks N] <file>...");
        std::process::exit(2);
    }
    println!("each half dealt into {block_count} blocks");
    // An input that cannot be read is a section missing from the report, and a
    // report missing a section reads as complete. So a run with one exits
    // failing.
    let mut unreadable = 0usize;
    for arg in &args {
        let path = Path::new(arg);
        let name =
            path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let raw = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{name}: unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };
        let read = raw.len();
        let bytes = trex::encoding::decode(raw);
        if bytes.len() == read {
            println!("== {name} ({read} bytes)");
        } else {
            println!("== {name} ({} bytes as trex scan decodes the {read} read)", bytes.len());
        }
        let toks = trex::lexer::lex(&bytes);
        let context = context_of(&bytes, &toks);
        report(&byte_grain(&bytes, &toks, &context), block_count);
        report(&token_grain(&bytes, &toks, &context), block_count);
        report(&unit_grain(&bytes, &toks, &context), block_count);
        println!();
    }
    if unreadable > 0 {
        eprintln!("{unreadable} of {} inputs could not be read", args.len());
        std::process::exit(1);
    }
}
