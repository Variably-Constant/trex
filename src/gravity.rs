//! Gravity and repulsion between units: the pull the data shows one type to
//! have on another at a gap, and three readings taken from it.
//!
//! The field is `g_ab(d) = P(b at t+d | a at t) / P(b)` over gaps `1..=reach`,
//! above one a pull and below one a push. Each cell is smoothed by one pair on
//! either side, `(count + 1) / (expected + 1)`, so a pair never seen reads as a
//! push in proportion to how often chance would show it, and `-log2 g` is the
//! cell's potential in bits.
//!
//! - Strain: a unit's mean potential against each unit before it within the
//!   reach, in bits. High where the units before it push it away.
//! - Binding: the attraction across the cut before a unit, the mean `log2 g`
//!   over the pairs that straddle the cut within [`BIND_REACH`], in bits per
//!   pair. Low where the two sides hold together least. A mean rather than a
//!   sum, so a cut near either end of the input, which fewer pairs straddle,
//!   reads on the scale every other cut reads on.
//! - Geometry: each type's class, from placing the types in a space built from
//!   the field's significant cells, so types the data treats alike share a
//!   class.
//!
//! The first unit has nothing before it and no cut before it, so it reads
//! neither strain nor binding.
//!
//! The field is learned from the input it is read on, at one of three grains:
//! the bytes, the significant tokens, or the supertokens. The reaches, the
//! binding window, the significance cut and the class count are the
//! configuration `examples/what_gravity_and_repulsion_carry` measured.

use std::collections::HashMap;

use crate::ast::Grain;
use crate::token::{Token, TokenKind};

/// The gaps the byte field reads to.
pub const BYTE_REACH: usize = 64;
/// The gaps the token field reads to.
pub const TOKEN_REACH: usize = 32;
/// The gaps the supertoken field reads to.
pub const UNIT_REACH: usize = 16;
/// How far either side of a cut its binding reaches.
pub const BIND_REACH: usize = 8;
/// The types a grain with an open alphabet keeps by frequency; every other
/// type is one type more.
pub const TOP_TYPES: usize = 255;
/// Poisson standard deviations past which a cell reads as attracting or
/// repelling.
pub const Z_CELL: f64 = 4.0;
/// Pairs chance must give a cell before its z is read.
pub const MIN_EXPECTED: f64 = 5.0;
/// The dimensions of the space the geometry clusters in.
pub const GEOMETRY_DIMS: usize = 8;
/// The classes the geometry clusters the placed types into.
pub const GEOMETRY_CLASSES: usize = 16;
/// The class of a type no significant cell places: kin to nothing but itself.
pub const UNPLACED: u16 = GEOMETRY_CLASSES as u16;

/// Pair counts over gaps `1..=reach`, and the potential each cell gives.
pub struct Field {
    types: usize,
    reach: usize,
    /// `pair[(d - 1) * types * types + a * types + b]`: positions `i` holding
    /// `a` with `b` at `i + d`.
    pair: Vec<u32>,
    /// `from[(d - 1) * types + a]`: positions holding `a` with a unit `d` on.
    from: Vec<u32>,
    /// `to[(d - 1) * types + b]`: positions holding `b` with a unit `d` back.
    to: Vec<u32>,
    /// `pairs[d - 1]`: the pairs a gap of `d` fits.
    pairs: Vec<u64>,
    /// `-log2 g` a cell.
    potential: Vec<f32>,
}

impl Field {
    /// The field of the pairs in `seq`, whose values are below `types`, over
    /// gaps `1..=reach`.
    ///
    /// # Panics
    ///
    /// When a value of `seq` is not below `types`.
    #[must_use]
    pub fn learn(seq: &[u32], types: usize, reach: usize) -> Field {
        let tt = types * types;
        let mut pair = vec![0u32; reach * tt];
        let mut from = vec![0u32; reach * types];
        let mut to = vec![0u32; reach * types];
        let mut pairs = vec![0u64; reach];
        for d in 1..=reach.min(seq.len().saturating_sub(1)) {
            let cells = &mut pair[(d - 1) * tt..d * tt];
            let froms = &mut from[(d - 1) * types..d * types];
            let tos = &mut to[(d - 1) * types..d * types];
            for (&a, &b) in seq.iter().zip(&seq[d..]) {
                let (a, b) = (a as usize, b as usize);
                assert!(a < types && b < types, "a unit's type is below the type count");
                cells[a * types + b] += 1;
                froms[a] += 1;
                tos[b] += 1;
            }
            pairs[d - 1] = (seq.len() - d) as u64;
        }
        let mut field = Field { types, reach, pair, from, to, pairs, potential: Vec::new() };
        field.potential = (0..reach * tt)
            .map(|k| {
                let (d, a, b) = (k / tt + 1, k % tt / types, k % types);
                -field.lift(a, b, d).log2() as f32
            })
            .collect();
        field
    }

    /// How many types the field is over.
    #[must_use]
    pub fn types(&self) -> usize {
        self.types
    }

    /// The largest gap the field reads.
    #[must_use]
    pub fn reach(&self) -> usize {
        self.reach
    }

    /// The pairs of `a` then `b` at gap `d` chance alone would give.
    #[must_use]
    pub fn expected(&self, a: usize, b: usize, d: usize) -> f64 {
        let m = self.pairs[d - 1] as f64;
        if m <= 0.0 {
            return 0.0;
        }
        f64::from(self.from[(d - 1) * self.types + a]) * f64::from(self.to[(d - 1) * self.types + b]) / m
    }

    /// The pairs of `a` then `b` at gap `d` the input holds.
    #[must_use]
    pub fn count(&self, a: usize, b: usize, d: usize) -> f64 {
        f64::from(self.pair[(d - 1) * self.types * self.types + a * self.types + b])
    }

    /// `g`, smoothed by one pair on each side.
    #[must_use]
    pub fn lift(&self, a: usize, b: usize, d: usize) -> f64 {
        (self.count(a, b, d) + 1.0) / (self.expected(a, b, d) + 1.0)
    }

    /// The count's distance from chance in Poisson standard deviations.
    #[must_use]
    pub fn z(&self, a: usize, b: usize, d: usize) -> f64 {
        let e = self.expected(a, b, d);
        if e <= 0.0 { 0.0 } else { (self.count(a, b, d) - e) / e.sqrt() }
    }

    /// `-log2 g` of `a` then `b` at gap `d`, in bits.
    #[must_use]
    pub fn potential(&self, a: u32, b: u32, d: usize) -> f32 {
        self.potential[(d - 1) * self.types * self.types + a as usize * self.types + b as usize]
    }

    /// Whether the cell of `a` then `b` at gap `d` clears chance: expected at
    /// least [`MIN_EXPECTED`] and past [`Z_CELL`] either way.
    #[must_use]
    pub fn significant(&self, a: usize, b: usize, d: usize) -> bool {
        self.expected(a, b, d) >= MIN_EXPECTED && self.z(a, b, d).abs() > Z_CELL
    }

    /// How often type `a` opens a pair at the shortest gap.
    fn frequency(&self, a: usize) -> f64 {
        f64::from(self.from[a])
    }
}

/// Each unit's strain: the mean potential between it and each unit before it
/// within the field's reach, in bits. The first unit has nothing before it
/// and reads `None`.
#[must_use]
pub fn strain(seq: &[u32], f: &Field) -> Vec<Option<f32>> {
    (0..seq.len())
        .map(|t| {
            let span = t.min(f.reach);
            (span > 0).then(|| (1..=span).map(|d| f.potential(seq[t - d], seq[t], d)).sum::<f32>() / span as f32)
        })
        .collect()
}

/// The binding of the cut before each unit: the mean `log2 g` over the pairs
/// of units that straddle it within [`BIND_REACH`], in bits per pair. The
/// first unit has no cut before it and reads `None`.
#[must_use]
pub fn binding(seq: &[u32], f: &Field) -> Vec<Option<f32>> {
    let n = seq.len();
    let reach = BIND_REACH.min(f.reach);
    (0..n)
        .map(|t| {
            (t > 0).then(|| {
                let mut s = 0.0f32;
                let mut pairs = 0u32;
                for d in 1..=reach {
                    for i in t.saturating_sub(d)..t {
                        if i + d < n {
                            s -= f.potential(seq[i], seq[i + d], d);
                            pairs += 1;
                        }
                    }
                }
                // Every cut after the first unit has the pair of its two
                // neighbors straddling it.
                s / pairs as f32
            })
        })
        .collect()
}

/// Each type's class in the space where attraction pulls types together and
/// repulsion pushes them apart.
///
/// The space is built only from significant cells, so a pair seen once by
/// accident places nothing; a type with no such cell reads [`UNPLACED`]. The
/// rest take the leading eigenvectors of the mean `ln g` over the gaps,
/// symmetrized, each scaled by the root of its eigenvalue's size, clustered by
/// frequency-weighted k-means. Fewer placed types than classes places none.
#[must_use]
pub fn geometry(f: &Field) -> Vec<u16> {
    let t = f.types;
    let mut out = vec![UNPLACED; t];
    let mut affinity = vec![0.0f64; t * t];
    let mut evident = vec![false; t];
    for d in 1..=f.reach {
        for a in 0..t {
            for b in 0..t {
                if f.significant(a, b, d) {
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

/// The `k` eigenvectors of the symmetric `m` by `m` matrix `s` whose
/// eigenvalues are largest in size, by orthogonal iteration from a fixed start,
/// with their eigenvalues.
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
///
/// # Panics
///
/// With no points or no classes; [`geometry`] calls it with at least
/// [`GEOMETRY_CLASSES`] points.
fn kmeans(points: &[Vec<f64>], weights: &[f64], k: usize) -> Vec<u16> {
    let dist = squared_distance;
    let first = (0..points.len())
        .max_by(|&a, &b| weights[a].total_cmp(&weights[b]))
        .expect("points to cluster");
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
            let best = (0..k)
                .min_by(|&x, &y| dist(p, &centers[x]).total_cmp(&dist(p, &centers[y])))
                .expect("classes to assign to");
            let best = u16::try_from(best).expect("a class count that fits a u16");
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

/// A token's type at the token grain: its kind, with punctuation and the
/// other one-off bytes told apart by up to three characters of their text, as
/// a pattern tells them apart.
#[must_use]
pub fn token_key(kind: TokenKind, text: &[u8]) -> String {
    match kind {
        TokenKind::Punct | TokenKind::Other => {
            let text: String = String::from_utf8_lossy(text).chars().take(3).collect();
            format!("{kind:?} {text}")
        }
        kind => format!("{kind:?}"),
    }
}

/// A supertoken's type: its role and the kinds of up to its first six
/// significant tokens.
#[must_use]
pub fn unit_key(role: crate::supertoken::Role, kinds: &[TokenKind]) -> String {
    let shown: Vec<String> = kinds.iter().take(6).map(|k| format!("{k:?}")).collect();
    format!("{}:{}", role.label(), shown.join(","))
}

/// The key a byte's type is named by.
#[must_use]
pub fn byte_key(b: u8) -> String {
    format!("{b:02x}")
}

/// The type key `example` names at `grain`, read as a pattern's `@kin`
/// example: the one byte it is, the first significant token it lexes to, or
/// the first supertoken it forms. `None` where it names none: not exactly one
/// byte at the byte grain, or no token or unit at the others.
#[must_use]
pub fn example_key(grain: Grain, example: &[u8]) -> Option<String> {
    match grain {
        Grain::Byte => match example {
            [b] => Some(byte_key(*b)),
            _ => None,
        },
        Grain::Token => {
            let toks = crate::lexer::lex(example);
            let t = toks.iter().find(|t| t.is_significant())?;
            Some(token_key(t.kind, &example[t.span()]))
        }
        Grain::Super => {
            let toks = crate::lexer::lex(example);
            let unit = crate::supertoken::supertokens_from(&toks, example).into_iter().next()?;
            let kinds: Vec<TokenKind> = toks
                .iter()
                .filter(|t| t.is_significant() && t.start() >= unit.start && t.start() < unit.end)
                .map(|t| t.kind)
                .collect();
            Some(unit_key(unit.role, &kinds))
        }
    }
}

/// Keys numbered densely by frequency, most frequent first and ties by key.
/// The first [`TOP_TYPES`] are kept, and every other key is the one type after
/// them.
fn dense_types(keys: &[String]) -> (Vec<u32>, usize, HashMap<String, u32>) {
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for key in keys {
        *counts.entry(key.as_str()).or_insert(0) += 1;
    }
    let mut ranked: Vec<(&str, u32)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let kept = ranked.len().min(TOP_TYPES);
    let index: HashMap<String, u32> = ranked[..kept]
        .iter()
        .enumerate()
        .map(|(i, (k, _))| ((*k).to_string(), u32::try_from(i).expect("fewer than TOP_TYPES")))
        .collect();
    let rest = u32::try_from(kept).expect("fewer than TOP_TYPES");
    let seq = keys
        .iter()
        .map(|k| match index.get(k.as_str()) {
            Some(&t) => t,
            None => rest,
        })
        .collect();
    let types = if ranked.len() > kept { kept + 1 } else { kept.max(1) };
    (seq, types, index)
}

/// The index of the span in `spans`, ascending by start, that starts at
/// `offset`.
fn span_starting_at(spans: &[(usize, usize)], offset: usize) -> Option<usize> {
    let i = spans.partition_point(|&(s, _)| s < offset);
    spans.get(i).is_some_and(|&(s, _)| s == offset).then_some(i)
}

/// The three readings of one grain of one input, with each unit's position.
pub struct Readings {
    grain: Grain,
    /// Byte span of each unit, ascending; empty at the byte grain, where unit
    /// `i` is byte `i`.
    spans: Vec<(usize, usize)>,
    seq: Vec<u32>,
    /// Each unit's strain in bits; `None` at the first unit, which has nothing
    /// before it.
    pub strain: Vec<Option<f32>>,
    /// The binding of the cut before each unit, in bits; `None` at the first
    /// unit, which has no cut before it.
    pub binding: Vec<Option<f32>>,
    /// Each type's geometry class.
    pub class: Vec<u16>,
    /// The strain and binding readings taken, sorted, for percentiles.
    strain_sorted: Vec<f32>,
    binding_sorted: Vec<f32>,
    /// The type each kept key denotes, for naming a type by example.
    index: HashMap<String, u32>,
}

impl Readings {
    /// The readings of `bytes` at `grain`, over the lex `toks`.
    #[must_use]
    pub fn read(grain: Grain, bytes: &[u8], toks: &[Token]) -> Readings {
        let (spans, seq, types, index, reach) = match grain {
            Grain::Byte => {
                let seq: Vec<u32> = bytes.iter().map(|&b| u32::from(b)).collect();
                let index = (0..=255u8).map(|b| (byte_key(b), u32::from(b))).collect();
                (Vec::new(), seq, 256, index, BYTE_REACH)
            }
            Grain::Token => {
                let sig: Vec<&Token> = toks.iter().filter(|t| t.is_significant()).collect();
                let keys: Vec<String> = sig.iter().map(|t| token_key(t.kind, &bytes[t.span()])).collect();
                let (seq, types, index) = dense_types(&keys);
                let spans = sig.iter().map(|t| (t.start(), t.end())).collect();
                (spans, seq, types, index, TOKEN_REACH)
            }
            Grain::Super => {
                let units = crate::supertoken::supertokens_from(toks, bytes);
                let sig: Vec<&Token> = toks.iter().filter(|t| t.is_significant()).collect();
                let mut i = 0usize;
                let keys: Vec<String> = units
                    .iter()
                    .map(|u| {
                        while i < sig.len() && sig[i].start() < u.start {
                            i += 1;
                        }
                        let kinds: Vec<TokenKind> =
                            sig[i..].iter().take_while(|t| t.start() < u.end).map(|t| t.kind).collect();
                        unit_key(u.role, &kinds)
                    })
                    .collect();
                let (seq, types, index) = dense_types(&keys);
                let spans = units.iter().map(|u| (u.start, u.end)).collect();
                (spans, seq, types, index, UNIT_REACH)
            }
        };
        let field = Field::learn(&seq, types, reach);
        let strain = strain(&seq, &field);
        let binding = binding(&seq, &field);
        let class = geometry(&field);
        let sorted = |v: &[Option<f32>]| {
            let mut s: Vec<f32> = v.iter().flatten().copied().collect();
            s.sort_by(f32::total_cmp);
            s
        };
        Readings {
            grain,
            spans,
            strain_sorted: sorted(&strain),
            binding_sorted: sorted(&binding),
            seq,
            strain,
            binding,
            class,
            index,
        }
    }

    /// The grain these readings are of.
    #[must_use]
    pub fn grain(&self) -> Grain {
        self.grain
    }

    /// How many units the grain holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.seq.len()
    }

    /// Whether the grain holds no unit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.seq.is_empty()
    }

    /// The unit holding byte `offset`: the byte itself, the significant token
    /// starting there, or the supertoken spanning it.
    #[must_use]
    pub fn unit_at(&self, offset: usize) -> Option<usize> {
        match self.grain {
            Grain::Byte => (offset < self.seq.len()).then_some(offset),
            Grain::Token => span_starting_at(&self.spans, offset),
            Grain::Super => {
                let i = self.spans.partition_point(|&(s, _)| s <= offset).checked_sub(1)?;
                (offset < self.spans[i].1).then_some(i)
            }
        }
    }

    /// The unit that starts at byte `offset`, where one does: the cut its
    /// binding is read at is just before it.
    #[must_use]
    pub fn unit_starting_at(&self, offset: usize) -> Option<usize> {
        match self.grain {
            Grain::Byte => (offset < self.seq.len()).then_some(offset),
            Grain::Token | Grain::Super => span_starting_at(&self.spans, offset),
        }
    }

    /// The byte offset unit `unit` starts at, which is the offset of the cut
    /// before it.
    #[must_use]
    pub fn start_of(&self, unit: usize) -> usize {
        match self.grain {
            Grain::Byte => unit,
            Grain::Token | Grain::Super => self.spans[unit].0,
        }
    }

    /// The bytes unit `unit` spans: the byte itself at the byte grain, the
    /// significant token or the supertoken otherwise.
    #[must_use]
    pub fn span_of(&self, unit: usize) -> (usize, usize) {
        match self.grain {
            Grain::Byte => (unit, unit + 1),
            Grain::Token | Grain::Super => self.spans[unit],
        }
    }

    /// How many distinct types the input's units are of, every key past the
    /// kept ones counting as one type.
    #[must_use]
    pub fn types(&self) -> usize {
        let mut seen = vec![false; self.class.len()];
        for &ty in &self.seq {
            seen[ty as usize] = true;
        }
        seen.into_iter().filter(|&s| s).count()
    }

    /// Type `ty` as a reader would name it: at the byte grain the byte, as
    /// itself where it is printable ASCII, `' '` for a space and an escape
    /// otherwise; at the other grains its key, a token's kind with a
    /// punctuation mark's text or a supertoken's role and kinds, and `other`
    /// for the type every key past the kept ones shares.
    ///
    /// # Panics
    ///
    /// At the byte grain, when `ty` is not a byte: every type there is one.
    #[must_use]
    pub fn type_label(&self, ty: u32) -> String {
        match self.grain {
            Grain::Byte => match u8::try_from(ty).expect("a byte-grain type is a byte") {
                b' ' => "' '".to_string(),
                b'\n' => "\\n".to_string(),
                b'\r' => "\\r".to_string(),
                b'\t' => "\\t".to_string(),
                b if b.is_ascii_graphic() => char::from(b).to_string(),
                b => format!("\\x{b:02x}"),
            },
            Grain::Token | Grain::Super => match self.index.iter().find(|&(_, &t)| t == ty) {
                Some((key, _)) => key.clone(),
                None => "other".to_string(),
            },
        }
    }

    /// The gravity class of type `ty`; `None` where no significant cell
    /// places it, so it is kin to nothing but itself.
    ///
    /// # Panics
    ///
    /// When `ty` is not one of this grain's types.
    #[must_use]
    pub fn class_of(&self, ty: u32) -> Option<u16> {
        let c = self.class[ty as usize];
        (c != UNPLACED).then_some(c)
    }

    /// The `k` units under the most strain, most first, a tie going to the
    /// earlier unit. The first unit reads none and is never among them.
    #[must_use]
    pub fn most_strained(&self, k: usize) -> Vec<usize> {
        let mut order: Vec<(usize, f32)> =
            self.strain.iter().enumerate().filter_map(|(u, s)| s.map(|s| (u, s))).collect();
        order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        order.into_iter().take(k).map(|(u, _)| u).collect()
    }

    /// The units just after the `k` cuts that hold together least, least
    /// first, a tie going to the earlier cut. The first unit has no cut
    /// before it and is never among them.
    #[must_use]
    pub fn weakest_cuts(&self, k: usize) -> Vec<usize> {
        let mut order: Vec<(usize, f32)> =
            self.binding.iter().enumerate().filter_map(|(u, b)| b.map(|b| (u, b))).collect();
        order.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        order.into_iter().take(k).map(|(u, _)| u).collect()
    }

    /// The classes the field places types in, ascending: each with its types,
    /// most frequent first, and how many of the grain's units are of them.
    #[must_use]
    pub fn classes(&self) -> Vec<GravityClass> {
        let mut units = vec![0usize; self.class.len()];
        for &ty in &self.seq {
            units[ty as usize] += 1;
        }
        let mut out: Vec<GravityClass> = Vec::new();
        for (ty, &c) in self.class.iter().enumerate() {
            if c == UNPLACED {
                continue;
            }
            let ty = u32::try_from(ty).expect("a grain's types fit a u32");
            match out.iter_mut().find(|g| g.class == c) {
                Some(g) => {
                    g.types.push(ty);
                    g.units += units[ty as usize];
                }
                None => out.push(GravityClass { class: c, types: vec![ty], units: units[ty as usize] }),
            }
        }
        out.sort_by_key(|g| g.class);
        out
    }
}

/// One gravity class of a grain: the types the pair field places together,
/// and how many units are of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GravityClass {
    /// The class's number, below [`GEOMETRY_CLASSES`].
    pub class: u16,
    /// Its types, most frequent first, as [`Readings::type_label`] names them
    /// by number.
    pub types: Vec<u32>,
    /// How many of the grain's units are of its types.
    pub units: usize,
}

/// One unit's two readings, each with its share of the input's readings
/// below it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitReading {
    /// The unit's strain in bits; `None` at the first unit.
    pub strain: Option<f32>,
    /// The share of the input's strain readings below it, in percent.
    pub strain_percentile: Option<f64>,
    /// The binding of the cut before the unit in bits; `None` at the first
    /// unit.
    pub bound: Option<f32>,
    /// The share of the input's binding readings below it, in percent.
    pub bound_percentile: Option<f64>,
}

impl Readings {
    /// Unit `unit`'s strain and the binding of the cut before it, each with
    /// its percentile among this input's readings.
    #[must_use]
    pub fn reading_of(&self, unit: usize) -> UnitReading {
        let strain = self.strain[unit];
        let bound = self.binding[unit];
        UnitReading {
            strain,
            strain_percentile: strain.map(|s| self.strain_rank(s)),
            bound,
            bound_percentile: bound.map(|b| self.binding_rank(b)),
        }
    }

    /// The type of unit `unit`.
    #[must_use]
    pub fn type_of(&self, unit: usize) -> u32 {
        self.seq[unit]
    }

    /// The type `key` denotes at this grain, where the input holds it among
    /// its kept types.
    #[must_use]
    pub fn type_named(&self, key: &str) -> Option<u32> {
        self.index.get(key).copied()
    }

    /// Whether types `a` and `b` are kin: the same type, or two placed types
    /// the geometry puts in one class.
    #[must_use]
    pub fn kin(&self, a: u32, b: u32) -> bool {
        if a == b {
            return true;
        }
        match (self.class.get(a as usize), self.class.get(b as usize)) {
            (Some(&x), Some(&y)) => x == y && x != UNPLACED,
            (None, _) | (_, None) => false,
        }
    }

    /// The strain at percentile `p` of this input's strain readings.
    #[must_use]
    pub fn strain_percentile(&self, p: f64) -> Option<f32> {
        percentile(&self.strain_sorted, p)
    }

    /// The binding at percentile `p` of this input's binding readings.
    #[must_use]
    pub fn binding_percentile(&self, p: f64) -> Option<f32> {
        percentile(&self.binding_sorted, p)
    }

    /// The share of this input's strain readings below `value`, in percent.
    #[must_use]
    pub fn strain_rank(&self, value: f32) -> f64 {
        rank(&self.strain_sorted, value)
    }

    /// The share of this input's binding readings below `value`, in percent.
    #[must_use]
    pub fn binding_rank(&self, value: f32) -> f64 {
        rank(&self.binding_sorted, value)
    }
}

/// The share of `sorted` below `value`, in percent; zero for no readings.
fn rank(sorted: &[f32], value: f32) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    100.0 * sorted.partition_point(|&x| x < value) as f64 / sorted.len() as f64
}

/// The value at percentile `p`, `0..=100`, of `sorted`, by the nearest rank.
fn percentile(sorted: &[f32], p: f64) -> Option<f32> {
    let last = sorted.len().checked_sub(1)?;
    let rank = (p.clamp(0.0, 100.0) / 100.0 * last as f64).round() as usize;
    sorted.get(rank.min(last)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream of a six-unit cycle: `0` is followed by `1` two units on every
    /// time, and never by `3`.
    fn periodic() -> Vec<u32> {
        (0..6000u32).map(|i| [0, 2, 1, 2, 3, 2][i as usize % 6]).collect()
    }

    #[test]
    fn a_pair_the_data_repeats_attracts_and_one_it_avoids_repels() {
        let seq = periodic();
        let f = Field::learn(&seq, 4, 8);
        assert!(f.lift(0, 1, 2) > 1.0, "a pair the data repeats reads as a pull");
        assert!(f.potential(0, 1, 2) < 0.0, "and its potential is negative");
        assert!(f.lift(0, 3, 2) < 1.0, "a pair the data avoids reads as a push");
        assert!(f.significant(0, 1, 2) && f.significant(0, 3, 2));
    }

    #[test]
    fn strain_is_higher_where_a_unit_breaks_the_pattern() {
        let mut seq = periodic();
        // A 3 where the cycle puts a 1.
        seq[3002] = 3;
        let f = Field::learn(&seq, 4, 8);
        let s = strain(&seq, &f);
        let at = |t: usize| s[t].expect("a unit with units before it");
        assert!(at(3002) > at(3008), "the unit out of place is strained against its past");
    }

    #[test]
    fn binding_is_weakest_at_a_cut_between_unrelated_runs() {
        // Two alphabets that meet at one seam.
        let mut seq: Vec<u32> = (0..3000u32).map(|i| i % 2).collect();
        seq.extend((0..3000u32).map(|i| 2 + i % 2));
        let f = Field::learn(&seq, 4, 16);
        let b = binding(&seq, &f);
        let at = |t: usize| b[t].expect("a cut with a unit before it");
        let seam = at(3000);
        assert!(at(1500) > seam && at(4500) > seam, "the seam between runs binds least");
    }

    #[test]
    fn the_rankings_read_the_units_that_have_a_reading_in_order() {
        let text = b"alpha beta; gamma(delta)\n".repeat(50);
        let toks = crate::lexer::lex(&text);
        let r = Readings::read(Grain::Token, &text, &toks);
        let strained = r.most_strained(5);
        assert_eq!(strained.len(), 5);
        assert!(!strained.contains(&0), "the first unit reads no strain");
        let s: Vec<f32> = strained.iter().map(|&u| r.strain[u].expect("a reading")).collect();
        assert!(s.windows(2).all(|w| w[0] >= w[1]), "most strained first: {s:?}");
        let weakest = r.weakest_cuts(5);
        assert!(!weakest.contains(&0), "the first unit has no cut before it");
        let b: Vec<f32> = weakest.iter().map(|&u| r.binding[u].expect("a reading")).collect();
        assert!(b.windows(2).all(|w| w[0] <= w[1]), "weakest first: {b:?}");
        let u = strained[0];
        let reading = r.reading_of(u);
        assert_eq!((reading.strain, reading.strain_percentile), (r.strain[u], r.strain[u].map(|s| r.strain_rank(s))));
        assert_eq!(r.reading_of(0).strain_percentile, None);
    }

    #[test]
    fn a_class_holds_its_types_and_counts_their_units() {
        // More distinct bytes than the geometry has classes, so it places some.
        let text = b"fn main() {\n    let x = compute(1, 2);\n    println!(\"{x}\");\n}\n".repeat(200);
        let toks = crate::lexer::lex(&text);
        let r = Readings::read(Grain::Byte, &text, &toks);
        let classes = r.classes();
        assert!(!classes.is_empty(), "the bytes place some types");
        for g in &classes {
            assert!(g.types.iter().all(|&t| r.class_of(t) == Some(g.class)), "{g:?}");
            let units = (0..r.len()).filter(|&u| g.types.contains(&r.type_of(u))).count();
            assert_eq!(g.units, units, "{g:?}");
        }
        assert_eq!(r.type_label(u32::from(b'x')), "x");
        assert_eq!(r.type_label(u32::from(b' ')), "' '");
        assert_eq!(r.type_label(u32::from(b'\n')), "\\n");
        assert_eq!(r.span_of(3), (3, 4));
        let distinct: std::collections::HashSet<u8> = text.iter().copied().collect();
        assert_eq!(r.types(), distinct.len(), "the types the input holds, not every byte value");
    }

    #[test]
    fn the_first_unit_reads_no_strain_and_no_bound() {
        let text = b"let x = f(a) ; let y = g(b) ; print x".to_vec();
        let toks = crate::lexer::lex(&text);
        for grain in [Grain::Byte, Grain::Token, Grain::Super] {
            let r = Readings::read(grain, &text, &toks);
            assert_eq!((r.strain[0], r.binding[0]), (None, None), "{grain:?}: nothing before the first unit");
            assert!(r.strain[1..].iter().all(Option::is_some), "{grain:?}: every later unit has a strain");
            assert!(r.binding[1..].iter().all(Option::is_some), "{grain:?}: and a cut before it");
            // The percentiles are of the readings taken.
            let lowest = r.binding[1..].iter().flatten().copied().fold(f32::INFINITY, f32::min);
            assert_eq!(r.binding_percentile(0.0), Some(lowest), "{grain:?}");
        }
    }

    #[test]
    fn geometry_never_joins_types_the_data_treats_differently() {
        // Four groups, each a marker and four interchangeable members that
        // follow that marker and no other. Four groups fit the space's
        // dimensions, and with more classes than groups a group may be split,
        // but members of two groups are never treated alike.
        let mut seq = Vec::new();
        let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..40_000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let group = (s % 4) as u32;
            let member = ((s >> 8) % 4) as u32;
            seq.push(16 + group);
            seq.push(group * 4 + member);
        }
        let f = Field::learn(&seq, 20, 2);
        let class = geometry(&f);
        assert!(class.iter().all(|&c| c != UNPLACED), "every type is placed: {class:?}");
        let (mut within, mut across) = (0, 0);
        for a in 0..16usize {
            for b in a + 1..16 {
                if class[a] == class[b] {
                    if a / 4 == b / 4 { within += 1 } else { across += 1 }
                }
            }
        }
        assert_eq!(across, 0, "members of two groups share a class: {class:?}");
        assert!(within > 0, "no two members of one group share a class: {class:?}");
    }

    #[test]
    fn the_readings_are_one_a_unit_at_every_grain_and_the_same_twice() {
        let text = b"fn main() {\n    let x = compute(1, 2);\n    println!(\"{x}\");\n}\n".repeat(200);
        let toks = crate::lexer::lex(&text);
        for grain in [Grain::Byte, Grain::Token, Grain::Super] {
            let r = Readings::read(grain, &text, &toks);
            assert!(!r.is_empty(), "{grain:?} holds units");
            assert_eq!(r.strain.len(), r.len());
            assert_eq!(r.binding.len(), r.len());
            let again = Readings::read(grain, &text, &toks);
            assert_eq!(r.strain, again.strain, "{grain:?} strain is deterministic");
            assert_eq!(r.class, again.class, "{grain:?} geometry is deterministic");
            let lo = r.strain_percentile(0.0).expect("a reading");
            let hi = r.strain_percentile(100.0).expect("a reading");
            assert!(lo <= hi);
        }
    }

    #[test]
    fn a_unit_is_found_by_the_bytes_it_holds() {
        let text = b"alpha beta; gamma(delta)\n".repeat(50);
        let toks = crate::lexer::lex(&text);
        let r = Readings::read(Grain::Token, &text, &toks);
        let sig: Vec<&Token> = toks.iter().filter(|t| t.is_significant()).collect();
        assert_eq!(r.unit_at(sig[1].start()), Some(1), "a token is found at its start");
        assert_eq!(r.unit_at(sig[1].start() + 1), None, "and nowhere inside it");
        let s = Readings::read(Grain::Super, &text, &toks);
        let first = s.unit_at(0).expect("the first unit holds the first byte");
        assert_eq!(s.unit_at(1), Some(first), "a supertoken is found anywhere inside it");
    }

    #[test]
    fn kin_is_the_same_type_or_one_placed_class() {
        let text = b"alpha beta; gamma(delta)\n".repeat(50);
        let toks = crate::lexer::lex(&text);
        let r = Readings::read(Grain::Byte, &text, &toks);
        let a = r.type_named(&byte_key(b'a')).expect("every byte has a type");
        let absent = r.type_named(&byte_key(0xfe)).expect("every byte has a type");
        assert!(r.kin(a, a), "a type is kin to itself");
        assert_eq!(r.class[absent as usize], UNPLACED, "a byte the input never holds is unplaced");
        assert!(!r.kin(a, absent), "and kin to nothing but itself");
    }
}
