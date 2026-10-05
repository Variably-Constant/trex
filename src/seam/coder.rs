//! The compressor behind `trex compress`: the byte coders its `--compare`
//! table ranks, the logistic mixer that codes by default, the byte-ngram prior
//! baked into the binary, and the rhythm selector and second mixer that read
//! the stream's period.
//!
//! Compiled with the `compress` feature, which is off by default. `seam`
//! re-exports every public item here, so each keeps its `trex::seam::` path.
//! The corpus-free measures the coders build on, `adaptive_byte_bits`,
//! `ppm_byte_bits` and `ctw_bits`, stay in `seam` in every build.

use std::collections::HashMap;

use super::{ByteModels, context_key};

/// Identity-hashed `u64` map. The count-table keys are already avalanched
/// (`base.wrapping_mul(MIX).wrapping_add(node)`), so the default cryptographic
/// SipHash is wasted work; the crate's [`crate::tokutil::IdHash`] passes the key
/// straight through. The flamegraph put SipHash at 5.4% of the coder.
///
/// This is one of two maps in `seam` and they take opposite keys. Passing a
/// key through is right only where the caller avalanched it first, which every
/// caller of this one does. A context model's key is a packed context carrying
/// no mixing of its own, so it takes [`OrderModel`](super::OrderModel) and
/// [`ContextHash`](super::ContextHash) instead - reading this justification
/// onto that map reads it onto keys it was never about.
type U64Map<V> = HashMap<u64, V, std::hash::BuildHasherDefault<crate::tokutil::IdHash>>;

/// Per-order context models for the id coder (the BPE token stream).
type IdModels = Vec<HashMap<Vec<usize>, (HashMap<usize, u32>, u32)>>;

/// Adaptive multi-order coder over an arbitrary symbol-id stream (used for the
/// BPE token stream). Same mixing scheme as the byte coder.
fn adaptive_id_bits(seq: &[usize], alphabet: usize, max_order: usize) -> f64 {
    let mut models: IdModels = (0..=max_order).map(|_| HashMap::new()).collect();
    let alpha = alphabet.max(1) as f64;
    let mut bits = 0.0f64;
    for t in 0..seq.len() {
        let actual = seq[t];
        let mut mass = 0.0f64;
        let mut wsum = 0.0f64;
        let hi = max_order.min(t);
        for k in 0..=hi {
            let ctx = seq[t - k..t].to_vec();
            if let Some((counts, total)) = models[k].get(&ctx)
                && *total > 0
            {
                let w = f64::from(*total) * (k as f64 + 1.0);
                mass += w * (f64::from(*counts.get(&actual).unwrap_or(&0)) / f64::from(*total));
                wsum += w;
            }
        }
        mass += 1.0 / alpha;
        wsum += 1.0;
        let p = (mass / wsum).max(1e-12);
        bits += -p.log2();
        for k in 0..=hi {
            let ctx = seq[t - k..t].to_vec();
            let e = models[k].entry(ctx).or_insert_with(|| (HashMap::new(), 0));
            *e.0.entry(actual).or_insert(0) += 1;
            e.1 += 1;
        }
    }
    bits
}

/// Code `input` the BPE way and return `(stream_bits, codebook_bits)`. BPE is
/// trained on the input itself (no train/test gap, the generous case), then a
/// lossless symbol stream is built - BPE pieces for word runs, each whitespace
/// run as one literal symbol - and coded with an adaptive order-1 token model.
/// The codebook (the distinct token strings the decoder needs to map ids back
/// to bytes) is charged honestly at one byte per character.
#[must_use]
pub fn bpe_total_bits(input: &[u8], num_merges: usize) -> (f64, f64) {
    let bpe = crate::bpe::Bpe::train(input, num_merges);
    let mut symbols: Vec<String> = Vec::new();
    let mut i = 0;
    while i < input.len() {
        let ws = input[i].is_ascii_whitespace();
        let start = i;
        while i < input.len() && input[i].is_ascii_whitespace() == ws {
            i += 1;
        }
        let run = &input[start..i];
        if ws {
            symbols.push(String::from_utf8_lossy(run).into_owned());
        } else {
            symbols.extend(bpe.encode(run));
        }
    }
    let mut ids: HashMap<&str, usize> = HashMap::new();
    let seq: Vec<usize> = symbols
        .iter()
        .map(|s| {
            let n = ids.len();
            *ids.entry(s.as_str()).or_insert(n)
        })
        .collect();
    let stream = adaptive_id_bits(&seq, ids.len(), 1);
    let codebook = ids.keys().map(|s| s.len() as f64 * 8.0).sum();
    (stream, codebook)
}

/// [`ppm_byte_bits`](super::ppm_byte_bits) with a side symbol folded into the
/// contexts of its lowest orders.
///
/// A byte's context is the bytes before it. A side symbol is some other
/// reading of the stream held beside them - the rhythm a resonator bank reads,
/// say - and folding it into the key splits each context by that reading, so
/// a context that predicts one way in one state of the stream and another way
/// in another can say so. Whether that is worth its cost is the question: every
/// split divides a context's counts, so a side symbol carrying nothing about
/// the next byte costs escapes and gains nothing.
///
/// `side[t]` is read with the context that codes byte `t`, so it must be a
/// symbol a decoder can compute from bytes before `t` alone; a reading taken
/// after byte `t` is folded in would code the byte from itself. Orders below
/// `side_orders` key on the byte string and the side symbol together; orders
/// at or above it key on the bytes alone, as `ppm_byte_bits` does. With
/// `side_orders` at zero no context carries it and the bits are exactly
/// `ppm_byte_bits`'s, which is the control a test holds it to.
///
/// # Panics
///
/// When `side` is not one symbol a byte.
#[must_use]
pub fn ppm_byte_bits_with_side(
    input: &[u8],
    max_order: usize,
    side: &[u8],
    side_orders: usize,
) -> f64 {
    assert_eq!(side.len(), input.len(), "one side symbol a byte");
    let key = |t: usize, k: usize| -> Vec<u8> {
        let mut ctx = input[t - k..t].to_vec();
        if k < side_orders {
            ctx.push(side[t]);
        }
        ctx
    };
    let mut models: ByteModels = (0..=max_order).map(|_| HashMap::new()).collect();
    let mut bits = 0.0f64;
    for (t, &actual) in input.iter().enumerate() {
        let hi = max_order.min(t);
        let mut excluded = [false; 256];
        let mut n_excluded = 0usize;
        let mut log_p = 0.0f64;
        let mut coded = false;
        for k in (0..=hi).rev() {
            if let Some((counts, _)) = models[k].get(&key(t, k)) {
                let mut eff_total = 0u32;
                let mut eff_distinct = 0u32;
                for (&s, &c) in counts {
                    if !excluded[s as usize] {
                        eff_total += c;
                        eff_distinct += 1;
                    }
                }
                if eff_total > 0 {
                    let denom = f64::from(eff_total + eff_distinct);
                    let sym_c =
                        if excluded[actual as usize] { 0 } else { *counts.get(&actual).unwrap_or(&0) };
                    if sym_c > 0 {
                        log_p += (f64::from(sym_c) / denom).log2();
                        coded = true;
                        break;
                    }
                    log_p += (f64::from(eff_distinct) / denom).log2();
                    for &s in counts.keys() {
                        if !excluded[s as usize] {
                            excluded[s as usize] = true;
                            n_excluded += 1;
                        }
                    }
                }
            }
        }
        if !coded {
            let avail = (256 - n_excluded).max(1);
            log_p += (1.0 / avail as f64).log2();
        }
        bits += -log_p;
        for (k, model) in models.iter_mut().enumerate().take(hi + 1) {
            let e = model.entry(key(t, k)).or_insert_with(|| (HashMap::new(), 0));
            *e.0.entry(actual).or_insert(0) += 1;
            e.1 += 1;
        }
    }
    bits
}

/// Quantize a preloaded prior's counts to the classes its rows resonate in.
///
/// This is where merging can help and an online model cannot. A coder builds
/// its own model from bytes both sides have, so that model costs nothing to
/// carry and sharing can only lose accuracy. A baked prior is stored and
/// shipped once per binary, so what it costs is bytes on disk
/// and the question becomes what a cheaper prior gives up in prediction.
///
/// Every key is kept. What changes is the values: a row is a pair of
/// bit-tree counts, its shape is the split between them, and rows whose
/// splits agree to within `rho` take their class's average. So the saving
/// is entropy in the value stream rather than rows removed - a blob with
/// few distinct values compresses where one with many does not - and the
/// key list, which dominates the stored size, is untouched. Dropping keys
/// is a different question and this does not answer it.
///
/// Reports rows in and classes out, summed over the orders.
#[must_use]
pub fn cluster_preload(tables: &[PreloadTable], rho: f32) -> (Vec<PreloadTable>, usize, usize) {
    let mut out = Vec::with_capacity(tables.len());
    let (mut before, mut after) = (0usize, 0usize);
    for (order, map) in tables {
        before += map.len();
        // Prototypes as the zero-share of the split, kept sorted so a
        // placement finds its candidates by binary search rather than by
        // walking every class: at these sizes a linear scan per row is
        // quadratic in the rows and the pass does not finish.
        let mut protos: Vec<(f32, (u64, u64), u32)> = Vec::new();
        let mut merged: U64Map<(u32, u32)> = U64Map::default();
        let mut keys: Vec<u64> = map.keys().copied().collect();
        // Sorted so the merge is reproducible: a map's iteration order is not
        // stable across runs, and a prior that differs run to run is not a
        // prior anyone can ship.
        keys.sort_unstable();
        for k in keys {
            let (n0, n1) = map[&k];
            if n0 + n1 == 0 {
                // A row with no observations has no split to compare, so it
                // is carried through untouched. Skipping it here would drop
                // it from the prior, which is a different change wearing
                // this one's clothes.
                merged.insert(k, (n0, n1));
                continue;
            }
            let share0 = n0 as f32 / (n0 + n1) as f32;
            // Overlap of two two-symbol distributions is 1 minus half the
            // absolute difference of their shares, so the vigilance test is
            // that difference against twice the slack.
            let slack = 2.0 * (1.0 - rho);
            // The search locates this share; the vigilance test
            // decides whether it may join, and applies to an exact hit as
            // much as to a neighbor. An exact hit is an overlap of one, and
            // a vigilance above one admits nothing - so a `rho` past one has
            // to reject even a row identical to a class, or the arm that is
            // supposed to merge nothing quietly merges the duplicates.
            let at = match protos.binary_search_by(|p| p.0.partial_cmp(&share0).expect("no NaN shares")) {
                Ok(i) | Err(i) => i,
            };
            let hit = [at.checked_sub(1), Some(at)]
                .into_iter()
                .flatten()
                .filter(|&j| j < protos.len())
                .find(|&j| (protos[j].0 - share0).abs() <= slack);
            match hit {
                Some(j) => {
                    protos[j].1.0 += u64::from(n0);
                    protos[j].1.1 += u64::from(n1);
                    protos[j].2 += 1;
                    let (a, b) = protos[j].1;
                    let cnt = protos[j].2;
                    // The class keeps the average of its members rather than
                    // their sum, so a merged row predicts like the rows it
                    // stands for instead of like all of them at once.
                    merged.insert(
                        k,
                        (
                            u32::try_from(a / u64::from(cnt)).unwrap_or(u32::MAX),
                            u32::try_from(b / u64::from(cnt)).unwrap_or(u32::MAX),
                        ),
                    );
                }
                None => {
                    protos.insert(at, (share0, (u64::from(n0), u64::from(n1)), 1));
                    merged.insert(k, (n0, n1));
                }
            }
        }
        after += protos.len();
        out.push((*order, merged));
    }
    (out, before, after)
}

/// Bits per byte under PPM when the context alphabet is a learned orbit.
///
/// The orbit `ppm_byte_bits` takes is fixed: it lowercases, so `A` and `a`
/// are one context symbol and nothing else is. A learned orbit asks the
/// stream instead - two bytes belong together when what follows them
/// agrees - and folds the context through the classes that fall out.
///
/// This changes which key a context is written under rather than what a row
/// holds, so it is a different question from merging rows: contexts stay
/// distinct and keep their own futures, and what shrinks is the alphabet
/// they are spelled in.
///
/// Causal, so a decoder can follow: a byte's class comes from successor
/// counts over already-coded bytes only, and is recomputed when its count
/// reaches a power of two. Recomputing on a schedule the decoder can also
/// compute is what keeps the two alphabets in step; recomputing every step
/// would give the same alphabet for far more work.
///
/// `rho` above one cannot be reached by an overlap, so every byte keeps its
/// own class and the coder's cost must equal its cost with no orbit at all.
/// That is the control.
#[must_use]
pub fn ppm_byte_bits_learned_orbit(input: &[u8], max_order: usize, rho: f32) -> (f64, usize) {
    let mut succ: Vec<HashMap<u8, u32>> = vec![HashMap::new(); 256];
    let mut seen = [0u32; 256];
    // Every byte starts in a class of its own, so the alphabet only ever
    // narrows from the identity and a byte never lands in a class before it
    // has been observed.
    let mut class: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut protos: Vec<Vec<(u8, f32)>> = Vec::new();
    let mut proto_of: Vec<u8> = Vec::new();

    let fold = |ctx: &[u8], class: &[u8; 256]| -> Vec<u8> {
        ctx.iter().map(|&b| class[b as usize]).collect()
    };

    let mut models: ByteModels = (0..=max_order).map(|_| HashMap::new()).collect();
    let mut bits = 0.0f64;
    for t in 0..input.len() {
        let actual = input[t];
        let hi = max_order.min(t);
        let mut excluded = [false; 256];
        let mut n_excluded = 0usize;
        let mut log_p = 0.0f64;
        let mut coded = false;
        for k in (0..=hi).rev() {
            let ctx = fold(&input[t - k..t], &class);
            let Some((counts, _)) = models[k].get(&ctx) else { continue };
            let mut eff_total = 0u32;
            let mut eff_distinct = 0u32;
            for (&s, &c) in counts {
                if !excluded[s as usize] {
                    eff_total += c;
                    eff_distinct += 1;
                }
            }
            if eff_total == 0 {
                continue;
            }
            let denom = f64::from(eff_total + eff_distinct);
            let sym_c = if excluded[actual as usize] { 0 } else { *counts.get(&actual).unwrap_or(&0) };
            if sym_c > 0 {
                log_p += (f64::from(sym_c) / denom).log2();
                coded = true;
                break;
            }
            log_p += (f64::from(eff_distinct) / denom).log2();
            for &s in counts.keys() {
                if !excluded[s as usize] {
                    excluded[s as usize] = true;
                    n_excluded += 1;
                }
            }
        }
        if !coded {
            let avail = (256 - n_excluded).max(1);
            log_p += (1.0 / avail as f64).log2();
        }
        bits += -log_p;

        for k in 0..=hi {
            let ctx = fold(&input[t - k..t], &class);
            let e = models[k].entry(ctx).or_insert_with(|| (HashMap::new(), 0));
            *e.0.entry(actual).or_insert(0) += 1;
            e.1 += 1;
        }

        // The byte just coded is the successor of the one before it, so the
        // orbit learns from a pair both sides have.
        if t > 0 {
            let prev = input[t - 1];
            *succ[prev as usize].entry(actual).or_insert(0) += 1;
            seen[prev as usize] += 1;
            let n = seen[prev as usize];
            if n >= 8 && n.is_power_of_two() {
                let x = as_shares(&succ[prev as usize], n);
                let mut placed = None;
                for (i, w) in protos.iter().enumerate() {
                    if fuzzy_overlap_bytes(&x, w) >= rho {
                        placed = Some(i);
                        break;
                    }
                }
                match placed {
                    Some(i) => class[prev as usize] = proto_of[i],
                    None => {
                        protos.push(x);
                        // A new class is named for the byte that founded it,
                        // so class ids stay inside the byte alphabet the
                        // context keys are spelled in.
                        proto_of.push(prev);
                        class[prev as usize] = prev;
                    }
                }
            }
        }
    }
    let distinct = {
        let mut s: Vec<u8> = class.to_vec();
        s.sort_unstable();
        s.dedup();
        s.len()
    };
    (bits, distinct)
}

/// Bits per byte under PPM when contexts that predict alike share one row.
///
/// The merging is causal, which is what makes it a codec rather than an
/// analysis: a context is placed into a class using only counts taken from
/// bytes already coded, so a decoder holding the same prefix reaches the
/// same class at the same step and reads the same distribution. Nothing
/// here consults a byte that has not been coded yet.
///
/// A context stays private until it has been seen [`PLACE_AFTER`] times.
/// Placing on first sight would match on a single observation, which every
/// context agrees with, and would collapse the model to one class; waiting
/// gives the context enough shape to be placed on what it predicts.
///
/// `rho` at or above one admits only exactly-agreeing distributions, so it
/// is the control: its bits must equal [`ppm_byte_bits`](super::ppm_byte_bits)'s.
#[must_use]
pub fn ppm_byte_bits_clustered(
    input: &[u8],
    max_order: usize,
    orbit_context: bool,
    rho: f32,
) -> (f64, usize, usize) {
    /// Sightings before a context is offered to the classes.
    const PLACE_AFTER: u32 = 4;

    // Per order: the contexts, each either private or pointing at a class,
    // and the classes themselves.
    let mut assign: Vec<HashMap<Vec<u8>, usize>> = (0..=max_order).map(|_| HashMap::new()).collect();
    /// A context's next-byte counts and their total.
    type Followers = (HashMap<u8, u32>, u32);
    let mut private: Vec<HashMap<Vec<u8>, Followers>> = (0..=max_order).map(|_| HashMap::new()).collect();
    let mut classes: Vec<Vec<Followers>> = (0..=max_order).map(|_| Vec::new()).collect();
    // Classes indexed by their most probable symbol, so a placement tests
    // only the classes that could resonate rather than every class at the
    // order. Two distributions whose top symbols differ cannot overlap by
    // more than the smaller one's remaining mass, so above a half they
    // cannot match at all - and below a half the bucket is a search order
    // rather than a filter, which is what the choice function is for.
    // Without it a placement scans every class and the pass is quadratic in
    // the contexts: on a quarter-megabyte corpus that is fifty-six thousand
    // classes against fifty-six thousand placements.
    let mut by_top: Vec<[Vec<usize>; 256]> =
        (0..=max_order).map(|_| std::array::from_fn(|_| Vec::new())).collect();

    let mut bits = 0.0f64;
    let mut placed_total = 0usize;
    for t in 0..input.len() {
        let actual = input[t];
        let hi = max_order.min(t);
        let mut excluded = [false; 256];
        let mut n_excluded = 0usize;
        let mut log_p = 0.0f64;
        let mut coded = false;
        for k in (0..=hi).rev() {
            let ctx = context_key(&input[t - k..t], orbit_context);
            let found = match assign[k].get(&ctx) {
                Some(&ci) => Some(&classes[k][ci]),
                None => private[k].get(&ctx),
            };
            let Some((counts, _)) = found else { continue };
            let mut eff_total = 0u32;
            let mut eff_distinct = 0u32;
            for (&s, &c) in counts {
                if !excluded[s as usize] {
                    eff_total += c;
                    eff_distinct += 1;
                }
            }
            if eff_total == 0 {
                continue;
            }
            let denom = f64::from(eff_total + eff_distinct);
            let sym_c = if excluded[actual as usize] { 0 } else { *counts.get(&actual).unwrap_or(&0) };
            if sym_c > 0 {
                log_p += (f64::from(sym_c) / denom).log2();
                coded = true;
                break;
            }
            log_p += (f64::from(eff_distinct) / denom).log2();
            for &s in counts.keys() {
                if !excluded[s as usize] {
                    excluded[s as usize] = true;
                    n_excluded += 1;
                }
            }
        }
        if !coded {
            let avail = (256 - n_excluded).max(1);
            log_p += (1.0 / avail as f64).log2();
        }
        bits += -log_p;

        // Update, then consider placement - both from bytes now coded.
        for k in 0..=hi {
            let ctx = context_key(&input[t - k..t], orbit_context);
            if let Some(&ci) = assign[k].get(&ctx) {
                let e = &mut classes[k][ci];
                *e.0.entry(actual).or_insert(0) += 1;
                e.1 += 1;
                continue;
            }
            let e = private[k].entry(ctx.clone()).or_insert_with(|| (HashMap::new(), 0));
            *e.0.entry(actual).or_insert(0) += 1;
            e.1 += 1;
            if e.1 < PLACE_AFTER {
                continue;
            }
            let (counts, total) = private[k].remove(&ctx).expect("just inserted");
            let x = as_shares(&counts, total);
            let top = top_symbol(&x);
            let mut placed = None;
            for &i in &by_top[k][top as usize] {
                let (cc, ct) = &classes[k][i];
                if fuzzy_overlap_bytes(&x, &as_shares(cc, *ct)) >= rho {
                    placed = Some(i);
                    break;
                }
            }
            let ci = match placed {
                Some(i) => {
                    let e = &mut classes[k][i];
                    for (s, c) in counts {
                        *e.0.entry(s).or_insert(0) += c;
                    }
                    e.1 += total;
                    i
                }
                None => {
                    classes[k].push((counts, total));
                    let i = classes[k].len() - 1;
                    by_top[k][top as usize].push(i);
                    i
                }
            };
            assign[k].insert(ctx, ci);
            placed_total += 1;
        }
    }
    let class_count: usize = classes.iter().map(Vec::len).sum();
    (bits, placed_total, class_count)
}

/// The symbol carrying the most of a distribution's mass, ties to the lowest
/// symbol so the bucket a distribution lands in does not depend on the order
/// its counts happened to be built in.
fn top_symbol(x: &[(u8, f32)]) -> u8 {
    let mut best = (0u8, -1.0f32);
    for &(s, p) in x {
        if p > best.1 {
            best = (s, p);
        }
    }
    best.0
}

/// A count table as sorted `(symbol, share)` pairs.
fn as_shares(counts: &HashMap<u8, u32>, total: u32) -> Vec<(u8, f32)> {
    let mut v: Vec<(u8, f32)> =
        counts.iter().map(|(&s, &c)| (s, c as f32 / total as f32)).collect();
    v.sort_unstable_by_key(|&(s, _)| s);
    v
}

/// The mass two byte distributions agree on, as [`fuzzy_overlap`](super::fuzzy_overlap).
fn fuzzy_overlap_bytes(x: &[(u8, f32)], w: &[(u8, f32)]) -> f32 {
    let (mut i, mut j, mut acc) = (0usize, 0usize, 0.0f32);
    while i < x.len() && j < w.len() {
        match x[i].0.cmp(&w[j].0) {
            std::cmp::Ordering::Equal => {
                acc += x[i].1.min(w[j].1);
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    acc
}

// --- Logistic context mixing (PAQ-style), with orbit models ---
//
// PPM is winner-take-all (one escape path). A logistic mixer blends every
// model's bit prediction in the stretch domain with online-adapted weights, so
// a weak model is down-weighted rather than corrupting the estimate. That is
// what lets the orbit context models (case fold, structural shape) ride as
// extra predictors: the literal context is never discarded, and the mixer
// keeps the quotient only where it helps.

/// Logistic stretch: probability to log-odds.
fn stretch(p: f64) -> f64 {
    (p / (1.0 - p)).ln()
}

/// Logistic squash: log-odds back to probability.
fn squash(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

// --- PAQ-style fixed-point stretch / squash (the integer mixer substrate) ---
//
// Probabilities are 12-bit (0..4095); the stretched log-odds domain is the
// symmetric [-2047, 2047] (scale 256, so +-2047 ~ +-8 natural log-odds, like
// lpaq). Integer arithmetic is associative, so the mixer dot-product over these
// is bit-identical under any SIMD lane order, which a floating-point
// dot-product is not.
fn stretch_squash_tables() -> &'static ([i16; 4096], [i16; 4096]) {
    static T: std::sync::OnceLock<([i16; 4096], [i16; 4096])> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut st = [0i16; 4096];
        let mut sq = [0i16; 4096];
        for (p, s) in st.iter_mut().enumerate() {
            let pf = p.clamp(1, 4095) as f64;
            *s = (256.0 * (pf / (4096.0 - pf)).ln()).round().clamp(-2047.0, 2047.0) as i16;
        }
        for (xi, q) in sq.iter_mut().enumerate() {
            let x = xi as f64 - 2048.0;
            *q = (4096.0 / (1.0 + (-x / 256.0).exp())).round().clamp(0.0, 4095.0) as i16;
        }
        (st, sq)
    })
}


/// FNV-1a over `bytes`, seeded by a per-model `tag` so different models do not
/// collide in a shared table layout.
fn ctx_hash(tag: u64, bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ tag.wrapping_mul(0x100_0000_01b3);
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// An adaptive probability map (APM / SSE): a secondary stage that calibrates a
/// probability against its own recent accuracy. Indexed by a small context and
/// the stretched input probability over 33 interpolation points spanning the
/// log-odds domain; each point adapts toward the observed bit.
struct Apm {
    points: usize,
    table: Vec<f64>,
}

impl Apm {
    fn new(n_ctx: usize) -> Apm {
        let points = 33;
        let mut table = vec![0.0f64; n_ctx * points];
        for c in 0..n_ctx {
            for i in 0..points {
                table[c * points + i] = squash(-8.0 + i as f64 * 0.5);
            }
        }
        Apm { points, table }
    }

    /// Calibrate `p` under context `ctx`; returns the refined probability and
    /// the table index to nudge once the bit is known.
    fn refine(&self, p: f64, ctx: usize) -> (f64, usize) {
        let s = stretch(p).clamp(-7.999, 7.999);
        let pos = (s + 8.0) * 2.0;
        let i = pos.floor() as usize;
        let w = pos - i as f64;
        let base = ctx * self.points + i;
        let refined = self.table[base] * (1.0 - w) + self.table[base + 1] * w;
        let idx = if w < 0.5 { base } else { base + 1 };
        (refined.clamp(1e-6, 1.0 - 1e-6), idx)
    }

    fn update(&mut self, idx: usize, bit: u8, rate: f64) {
        self.table[idx] += (f64::from(bit) - self.table[idx]) * rate;
    }
}

/// Bit-level logistic-mixing coder with context-selected mixer weights and an
/// APM/SSE calibration stage. Several context models (literal byte orders plus,
/// when `orbit_models`, the case-fold, structural-shape, and word-stem orbits
/// as extra predictors) each estimate `P(next bit = 1)`; their stretched
/// predictions are mixed with a weight set selected by the previous byte (so
/// the mixer specializes per regime), the mix is calibrated by an APM keyed on
/// the previous byte, and the actual bit is coded. Corpus-free and
/// codebook-free. Returns the total code length in bits.
/// A small baked warm-start corpus: the most valuable English words and common
/// code tokens, with punctuation and structure. Run through the same online
/// embedding/neural algorithm at startup (with zero bits charged), it pre-organizes
/// the learned embedding and the match model so they begin warm instead of
/// random - a shared prior baked once, never transmitted per document. Not a
/// frequency dictionary: the online adaptation keeps refining from there.
pub const WARM_SEED: &[u8] = b"the of and a to in is was that it he for as his on be at by had not are but from or have an they which one you were her all she there would their we him been has when who will more no if out so said what up its about into than them can only other new some could time these two may then do first any my now such like our over man me even most made after also did many before must through back years where much your way well down should because each just those people how too little state good very make world still see men work long get here between both life being under never day same another know while last might us great old year off come since against go came right used take three states home small however found system program data file value type result function return error message status request response server client name path key index size count length offset buffer header content format number string object number array list map set node edge graph token model context score weight order block range public struct impl trait fn let mut self use crate mod enum match where async await unsafe const static for while loop break continue return if else true false None Some Ok Err Vec String usize u8 u32 u64 f64 bool 0 1 2 3 4 5 6 7 8 9 ; , . ( ) { } [ ] : :: -> => & * + - = == != < > <= >= | || && // /* */ \" \n";


// --- Task B: the instant-load byte-level integer n-gram model ---
//
// The corpus warm reconstructs the count tables from text every run, which is
// slow on a large prior. The instant model trains those statistics once (a fast
// byte count pass), serializes them as integers, and loads them directly - no
// re-warm. It is a byte-level n-gram (per literal order, context -> next-byte
// counts), stored as raw context bytes plus sparse counts (which compress, unlike
// high-entropy hash keys) and expanded into the bit-tree count tables at load.

/// Literal orders covered by the baked byte-level model. Orders 1..4 are the
/// workhorses; orders 6 and 8 add longer-range context (order 0 is trivial).
/// All are literal orders present in the mixer's specs.
pub const MODEL_ORDERS: [usize; 6] = [1, 2, 3, 4, 6, 8];

/// The bit-tree node multiplier. It must equal the `MIX` constant inside
/// `logistic_mix_bits`, so a preloaded table's keys match the runtime's.
const NODE_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

thread_local! {
    /// How many times this thread has read the baked prior.
    static PRIOR_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Reads of the baked prior on this thread since [`reset_prior_reads`], or
/// since the thread started if it was never reset.
///
/// The prior is reached only when a context's slot in the live adaptive table
/// holds a different context, so this counts slot misses rather than the bit
/// loop's six orders by eight bits per input byte. Divide by that product for
/// the share of lookups a change to the prior's storage would touch.
///
/// Per thread, so a chunked compression reports per chunk.
#[must_use]
pub fn prior_reads() -> u64 {
    PRIOR_READS.with(std::cell::Cell::get)
}

/// Set this thread's prior-read count back to zero.
pub fn reset_prior_reads() {
    PRIOR_READS.with(|c| c.set(0));
}

thread_local! {
    /// The literal order and key of each baked-prior read on this thread, in
    /// read order; `None` when the thread is not recording.
    static PRIOR_KEYS: std::cell::RefCell<Option<Vec<(u8, u64)>>> = const { std::cell::RefCell::new(None) };
}

/// Record this thread's baked-prior reads into an empty record.
pub fn record_prior_keys() {
    PRIOR_KEYS.with(|r| *r.borrow_mut() = Some(Vec::new()));
}

/// This thread's recorded baked-prior reads, which ends the recording.
///
/// # Panics
///
/// When this thread has no recording.
#[must_use]
pub fn take_prior_keys() -> Vec<(u8, u64)> {
    PRIOR_KEYS.with(|r| r.borrow_mut().take().expect("this thread is recording its prior reads"))
}

/// Push one baked-prior read onto this thread's record, if it has one.
#[inline]
fn note_prior_key(order: usize, key: u64) {
    PRIOR_KEYS.with(|r| {
        if let Some(keys) = r.borrow_mut().as_mut() {
            keys.push((order as u8, key));
        }
    });
}

/// Per-order training counts: context bytes -> next-byte -> count.
type NgramCounts = std::collections::BTreeMap<Vec<u8>, std::collections::BTreeMap<u8, u32>>;

/// Train a byte-level n-gram model in one fast count pass (no prediction or
/// mixing), far cheaper than a full warm. Run once offline to build the prior.
#[must_use]
pub fn byte_ngram_train(corpus: &[u8]) -> Vec<NgramCounts> {
    let mut model: Vec<NgramCounts> = MODEL_ORDERS.iter().map(|_| NgramCounts::new()).collect();
    for t in 0..corpus.len() {
        let b = corpus[t];
        for (oi, &k) in MODEL_ORDERS.iter().enumerate() {
            if t < k {
                continue;
            }
            *model[oi].entry(corpus[t - k..t].to_vec()).or_default().entry(b).or_insert(0) += 1;
        }
    }
    model
}

/// Marks a stream whose context keys are front-coded. No order in
/// [`MODEL_ORDERS`] can take this value, so a reader tells the two apart by the
/// first byte and a blob serialized before the encoding existed still loads.
pub(crate) const FRONT_CODED: u8 = 0xFF;

/// Serialize the trained model: per order, the top-`top_k` contexts by total
/// count, each as its context bytes plus sparse next-byte counts (u16). The
/// decoder recomputes the context hash, so the keys cost only compressible
/// bytes. The whole blob is brotli-compressed at quality 11.
///
/// Contexts are written in byte order and each stores how many leading bytes it
/// shares with the one before it, then only the bytes that differ. Sorted keys
/// share long prefixes - an order-8 context usually shares seven bytes with its
/// neighbor - and brotli reaches only part of that: measured over a 96.5 MB key
/// stream, coding the prefixes takes the compressed keys from 19,699,730 bytes
/// to 15,120,478.
///
/// Nothing is dropped and no count changes, so the decoded model is the one that
/// went in.
#[must_use]
pub fn serialize_byte_ngram(model: &[NgramCounts], top_k: usize) -> Vec<u8> {
    let mut raw = vec![FRONT_CODED];
    for (oi, &k) in MODEL_ORDERS.iter().enumerate() {
        let mut ctxs: Vec<(&Vec<u8>, u32)> =
            model[oi].iter().map(|(c, m)| (c, m.values().sum::<u32>())).collect();
        ctxs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        ctxs.truncate(top_k);
        ctxs.sort_by(|a, b| a.0.cmp(b.0)); // by bytes: shared prefixes, and brotli locality
        raw.push(k as u8);
        raw.extend_from_slice(&(ctxs.len() as u32).to_le_bytes());
        let mut prev: &[u8] = &[];
        for (c, _) in ctxs {
            // The shared run cannot reach the key's own length, since the keys
            // are distinct and all of one order's keys are k bytes wide.
            let shared = c.iter().zip(prev).take_while(|(a, b)| a == b).count();
            raw.push(shared as u8);
            raw.extend_from_slice(&c[shared..]);
            prev = c;
            let m = &model[oi][c];
            let pairs: Vec<(&u8, &u32)> = m.iter().collect();
            raw.push(pairs.len().min(255) as u8);
            for (b, n) in pairs.into_iter().take(255) {
                raw.push(*b);
                raw.extend_from_slice(&(u16::try_from((*n).min(65535)).unwrap_or(65535)).to_le_bytes());
            }
        }
    }
    let mut out = Vec::new();
    {
        let mut w = brotli::CompressorWriter::new(&mut out, 4096, 11, 24);
        std::io::Write::write_all(&mut w, &raw).expect("brotli encode model");
    }
    out
}

/// Drop rows of a serialized prior that predict what their backoff already
/// predicts, and re-serialize.
///
/// A stored prior costs keys as much as counts - the shipped
/// blob carries every context's literal bytes - so removing a row removes
/// bytes, where collapsing its counts only makes the value stream cheaper to
/// compress. A row earns its place by disagreeing with the shorter context
/// the coder would otherwise fall back to; one that agrees is answering a
/// question already answered.
///
/// The backoff is the next order down in [`MODEL_ORDERS`], which is not
/// always one byte shorter - the orders run 1, 2, 3, 4, 6, 8, so an order-6
/// context falls back two bytes, not one. The parent is the suffix of that
/// length, because a context is the bytes immediately before a position and
/// a shorter one drops the oldest.
///
/// Order 1 has nothing to fall back to and is kept whole.
///
/// `rho` above one cannot be reached by an overlap, so nothing is dropped
/// and the blob round-trips: that is the control, and it also proves the
/// decode and re-encode are lossless on their own.
#[must_use]
pub fn prune_baked(blob: &[u8], rho: f32) -> (Vec<u8>, usize, usize) {
    let mut per_order = decode_baked(blob);
    let (before, after) = prune_counts(&mut per_order, rho);
    // top_k is the surviving row count, so the re-serialization keeps what
    // survived rather than re-truncating it.
    let widest = per_order.iter().map(std::collections::BTreeMap::len).max().unwrap_or(0);
    (serialize_byte_ngram(&per_order, widest), before, after)
}

/// Drop the rows of a trained model that predict what their backoff parent
/// already predicts, in place. Returns the row count before and after.
///
/// A row resonates with its parent when their next-byte distributions overlap
/// by at least `rho`; a resonating row carries nothing the parent does not.
///
/// Callers wanting this to bound what ships must run it before
/// [`serialize_byte_ngram`], whose `top_k` ranks contexts by total count.
///
/// The backoff is the next order down in [`MODEL_ORDERS`], which is not always
/// one byte shorter - the orders run 1, 2, 3, 4, 6, 8, so an order-6 context
/// falls back two bytes. The parent is the suffix of that length, because a
/// context is the bytes immediately before a position and a shorter one drops
/// the oldest. Order 1 has nothing to fall back to and is kept whole.
///
/// `rho` above one cannot be reached by an overlap, so nothing is dropped.
/// Decode a serialized blob back into per-order contexts and their next-byte
/// counts, the form the model has before serialization.
///
/// Orders arrive in [`MODEL_ORDERS`] positions; a segment whose order is not in
/// that list ends the decode, because the rest of the stream cannot be placed.
#[must_use]
pub fn decode_baked(blob: &[u8]) -> Vec<NgramCounts> {
    let mut raw = Vec::new();
    {
        let mut d = brotli::Decompressor::new(blob, 4096);
        std::io::Read::read_to_end(&mut d, &mut raw).expect("decode baked model");
    }
    let mut per_order: Vec<NgramCounts> = MODEL_ORDERS.iter().map(|_| NgramCounts::new()).collect();
    let mut i = 0usize;
    let coded = raw.first() == Some(&FRONT_CODED);
    if coded {
        i += 1;
    }
    while i + 5 <= raw.len() {
        let k = raw[i] as usize;
        i += 1;
        let nctx = u32::from_le_bytes(raw[i..i + 4].try_into().expect("four bytes")) as usize;
        i += 4;
        let Some(oi) = MODEL_ORDERS.iter().position(|&o| o == k) else {
            break;
        };
        let mut prev: Vec<u8> = Vec::new();
        for _ in 0..nctx {
            let ctx = if coded {
                if i >= raw.len() {
                    break;
                }
                let shared = raw[i] as usize;
                i += 1;
                if shared > k || i + (k - shared) > raw.len() {
                    break;
                }
                let mut c = Vec::with_capacity(k);
                c.extend_from_slice(&prev[..shared.min(prev.len())]);
                c.extend_from_slice(&raw[i..i + (k - shared)]);
                i += k - shared;
                c
            } else {
                if i + k > raw.len() {
                    break;
                }
                let c = raw[i..i + k].to_vec();
                i += k;
                c
            };
            prev.clear();
            prev.extend_from_slice(&ctx);
            if i >= raw.len() {
                break;
            }
            let npairs = raw[i] as usize;
            i += 1;
            let mut m = std::collections::BTreeMap::new();
            for _ in 0..npairs {
                if i + 3 > raw.len() {
                    break;
                }
                let b = raw[i];
                let n = u16::from_le_bytes(raw[i + 1..i + 3].try_into().expect("two bytes"));
                i += 3;
                m.insert(b, u32::from(n));
            }
            per_order[oi].insert(ctx, m);
        }
    }
    per_order
}

/// Merge `b` into `a`, scaling `b`'s counts by `weight`, and re-serialize.
///
/// Counts are observation tallies, so adding them is what training on both
/// corpora at once would have produced: a context both priors saw gets the sum
/// of what each saw, and a context only one saw arrives with its own counts
/// intact. That is what lets one prior carry another's languages without
/// shipping two.
///
/// `weight` scales `b` before the sum, so the blend can favor either side. At
/// 1.0 the two corpora count equally regardless of how much text each came
/// from, which is rarely what is wanted when one is far larger.
///
/// Counts saturate at the u16 the format stores. That ceiling is reported
/// rather than absorbed: saturation affects the contexts seen most often,
/// which are where a prior is worth its bytes, and a merge that flattens
/// them has changed what the prior predicts rather than added to it.
///
/// Returns the blob, `a`'s row count, `b`'s, the merged total, and how many
/// symbol counts hit the ceiling.
#[must_use]
pub fn merge_baked(a: &[u8], b: &[u8], weight: f32) -> (Vec<u8>, usize, usize, usize, usize) {
    let mut left = decode_baked(a);
    let right = decode_baked(b);
    let rows_a: usize = left.iter().map(std::collections::BTreeMap::len).sum();
    let rows_b: usize = right.iter().map(std::collections::BTreeMap::len).sum();

    let ceiling = u32::from(u16::MAX);
    let mut saturated = 0usize;
    for (oi, rows) in right.into_iter().enumerate() {
        if oi >= left.len() {
            break;
        }
        for (ctx, counts) in rows {
            // Scale first, and mint nothing for a row that contributes nothing.
            // Creating the entry before the counts are known adds an empty row
            // for every context in `b`: at weight 0 that is the whole of `b`
            // arriving as rows carrying no observation, which is neither the
            // merge asked for nor a no-op.
            let scaled: Vec<(u8, u32)> = counts
                .into_iter()
                .map(|(sym, n)| (sym, (n as f32 * weight).round().max(0.0) as u32))
                .filter(|&(_, s)| s > 0)
                .collect();
            if scaled.is_empty() {
                continue;
            }
            let slot = left[oi].entry(ctx).or_default();
            for (sym, s) in scaled {
                let e = slot.entry(sym).or_insert(0);
                let sum = e.saturating_add(s);
                if sum > ceiling {
                    saturated += 1;
                }
                *e = sum.min(ceiling);
            }
        }
    }

    let rows_out: usize = left.iter().map(std::collections::BTreeMap::len).sum();
    let widest = left.iter().map(std::collections::BTreeMap::len).max().unwrap_or(0);
    (serialize_byte_ngram(&left, widest), rows_a, rows_b, rows_out, saturated)
}

pub fn prune_counts(per_order: &mut [NgramCounts], rho: f32) -> (usize, usize) {
    let before: usize = per_order.iter().map(std::collections::BTreeMap::len).sum();
    // Pruned in place. A second copy of the model is a second copy of every
    // context's bytes, and at twenty-one million rows that is gigabytes for
    // nothing. Removing from an order cannot disturb a comparison still to
    // come, because the parent an order is judged against always belongs to
    // another order than the one being pruned.
    //
    // Highest order first, so a row is judged against a parent that is still
    // whole. Pruning a parent first would let a child survive for
    // disagreeing with something itself about to go.
    for oi in (1..MODEL_ORDERS.len().min(per_order.len())).rev() {
        let drop_at = MODEL_ORDERS[oi] - MODEL_ORDERS[oi - 1];
        let doomed: Vec<Vec<u8>> = {
            let (lower, upper) = per_order.split_at(oi);
            upper[0]
                .iter()
                .filter(|(ctx, counts)| {
                    lower[oi - 1]
                        .get(&ctx[drop_at..])
                        .is_some_and(|pc| ngram_overlap(counts, pc) >= rho)
                })
                .map(|(ctx, _)| ctx.clone())
                .collect()
        };
        for c in doomed {
            per_order[oi].remove(&c);
        }
    }
    let after: usize = per_order.iter().map(std::collections::BTreeMap::len).sum();
    (before, after)
}

/// The mass two next-byte count tables agree on, as distributions.
fn ngram_overlap(
    a: &std::collections::BTreeMap<u8, u32>,
    b: &std::collections::BTreeMap<u8, u32>,
) -> f32 {
    let ta: u32 = a.values().sum();
    let tb: u32 = b.values().sum();
    if ta == 0 || tb == 0 {
        return 0.0;
    }
    let mut acc = 0.0f32;
    for (s, &ca) in a {
        if let Some(&cb) = b.get(s) {
            acc += (ca as f32 / ta as f32).min(cb as f32 / tb as f32);
        }
    }
    acc
}

/// One preloaded order: its literal order `k` and the bit-tree count table.
pub type PreloadTable = (usize, U64Map<(u32, u32)>);

/// A byte-ngram prior as the coder reads it: per literal order, the `(n0, n1)`
/// counts at a bit-tree key, consulted when a context's slot holds a
/// different context.
///
/// Two forms implement it - the decoded hash tables `load_byte_ngram` builds,
/// and `crate::prior_cache::MappedPrior`, the same rows laid out on disk and
/// mapped - and the coder codes to the same bits against either.
pub trait PriorLookup {
    /// Whether the prior holds a table for literal order `order`.
    fn has_order(&self, order: usize) -> bool;
    /// The counts at `key` in the table for `order`, or `None` when that table
    /// has no such row or the prior has no such order.
    fn get(&self, order: usize, key: u64) -> Option<(u32, u32)>;
}

impl PriorLookup for [PreloadTable] {
    fn has_order(&self, order: usize) -> bool {
        self.iter().any(|(o, _)| *o == order)
    }

    fn get(&self, order: usize, key: u64) -> Option<(u32, u32)> {
        self.iter().find(|(o, _)| *o == order).and_then(|(_, table)| table.get(&key).copied())
    }
}

/// Decode a serialized byte-n-gram blob and expand it into bit-tree count
/// tables, one per order, keyed exactly as `logistic_mix_bits` keys its tables
/// (so they can be preloaded). Counts are capped proportionally at 1024 to match
/// the runtime's saturation, keeping the prior adaptive rather than overconfident.
#[must_use]
pub fn load_byte_ngram(blob: &[u8]) -> Vec<PreloadTable> {
    let mut raw = Vec::new();
    {
        let mut d = brotli::Decompressor::new(blob, 4096);
        std::io::Read::read_to_end(&mut d, &mut raw).expect("decode baked model");
    }
    // `segs`: each order's segment boundary (k, data start, context count),
    // from a cheap serial pass that expands nothing.
    //
    // A front-coded key is a shared-prefix length and the bytes that differ, so
    // the pass reads its width rather than assuming k. Reconstruction needs the
    // previous key, but only within an order - and the expand below runs one
    // worker per order, so each carries its own and nothing is serialized that
    // was parallel before.
    let coded = raw.first() == Some(&FRONT_CODED);
    let mut segs: Vec<(usize, usize, usize)> = Vec::new();
    let mut i = usize::from(coded);
    while i + 5 <= raw.len() {
        let k = raw[i] as usize;
        i += 1;
        let nctx = u32::from_le_bytes(raw[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        let data_start = i;
        for _ in 0..nctx {
            let key_width = if coded {
                if i >= raw.len() {
                    break;
                }
                let shared = raw[i] as usize;
                if shared > k {
                    break;
                }
                1 + (k - shared)
            } else {
                k
            };
            if i + key_width + 1 > raw.len() {
                break;
            }
            i += key_width;
            let npairs = raw[i] as usize;
            i += 1 + npairs * 3;
        }
        i = i.min(raw.len());
        segs.push((k, data_start, nctx));
    }
    // The per-order bit-tree tables, expanded in parallel across cores
    // (Flynnel, SMT-on). The expand was the serial prefix that capped
    // chunk-parallel scaling; spreading it over cores shrinks it,
    // bit-identically to a serial expand. Each table is pre-sized to cut
    // rehash churn.
    let mut out: Vec<PreloadTable> = segs.iter().map(|&(k, _, _)| (k, U64Map::default())).collect();
    let plan = flynnel::JobPlan::new(0, out.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::Gather);
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut out, 1, |start, slice| {
        for (j, (_k, table)) in slice.iter_mut().enumerate() {
            let (k, data_start, nctx) = segs[start + j];
            table.reserve(nctx * 12);
            let mut p = data_start;
            // This worker's own previous key, so reconstruction stays inside
            // the order it owns.
            let mut prev: Vec<u8> = Vec::with_capacity(k);
            let mut key: Vec<u8> = Vec::with_capacity(k);
            for _ in 0..nctx {
                let base = if coded {
                    if p >= raw.len() {
                        break;
                    }
                    let shared = raw[p] as usize;
                    p += 1;
                    if shared > k || p + (k - shared) > raw.len() {
                        break;
                    }
                    key.clear();
                    key.extend_from_slice(&prev[..shared.min(prev.len())]);
                    key.extend_from_slice(&raw[p..p + (k - shared)]);
                    p += k - shared;
                    prev.clear();
                    prev.extend_from_slice(&key);
                    ctx_hash(k as u64, &key)
                } else {
                    if p + k > raw.len() {
                        break;
                    }
                    let b = ctx_hash(k as u64, &raw[p..p + k]);
                    p += k;
                    b
                };
                if p >= raw.len() {
                    break;
                }
                let npairs = raw[p] as usize;
                p += 1;
                for _ in 0..npairs {
                    if p + 3 > raw.len() {
                        break;
                    }
                    let b = raw[p];
                    let n = u32::from(u16::from_le_bytes(raw[p + 1..p + 3].try_into().unwrap()));
                    p += 3;
                    let mut node = 1u64;
                    for bit_idx in (0..8).rev() {
                        let actual = (b >> bit_idx) & 1;
                        let key = base.wrapping_mul(NODE_MIX).wrapping_add(node);
                        let e = table.entry(key).or_insert((0, 0));
                        if actual == 1 {
                            e.1 += n;
                        } else {
                            e.0 += n;
                        }
                        node = (node << 1) | u64::from(actual);
                    }
                }
            }
            for v in table.values_mut() {
                let tot = v.0 + v.1;
                if tot > 1024 {
                    v.0 = v.0 * 1024 / tot;
                    v.1 = v.1 * 1024 / tot;
                }
            }
        }
    });
    out
}

/// The baked byte-level integer model, brotli-compressed and compiled in,
/// decoded and expanded once on first use: 21,590,905 bytes, expanding to
/// 58,109,862 rows in about two seconds. It comes with the `compress` feature,
/// so a build without that feature carries none of it.
const BAKED_MODEL_BR: &[u8] = include_bytes!("../../_corpus/baked_model.br");
static BAKED_MODEL: std::sync::OnceLock<Vec<PreloadTable>> = std::sync::OnceLock::new();

/// The baked, expanded byte-level model, cached after its first decode.
#[must_use]
pub fn baked_model() -> &'static [PreloadTable] {
    BAKED_MODEL.get_or_init(|| load_byte_ngram(BAKED_MODEL_BR))
}

/// The compiled-in byte-ngram prior as it was serialized, before any decode.
#[must_use]
pub fn baked_model_blob() -> &'static [u8] {
    BAKED_MODEL_BR
}

/// Data-parallel compression: slice the input into `n_chunks` contiguous spans,
/// code each independently on its own core (each initialized from the shared
/// baked prior, so a chunk does not start cold), and sum the per-chunk bit
/// costs. The per-bit model dependency is serial within a slice but the slices
/// are independent across cores - the signal in the text is parallelizable, and
/// the baked prior keeps the only loss (the cross-slice adaptation at the seams)
/// small. Returns the total code length in bits.
#[must_use]
pub fn compress_chunks(input: &[u8], n_chunks: usize, baked: Option<&[PreloadTable]>) -> f64 {
    compress_chunks_with(input, n_chunks, baked)
}

/// [`compress_chunks`] against any [`PriorLookup`], read from every chunk's
/// core at once.
#[must_use]
pub fn compress_chunks_with<P: PriorLookup + Sync + ?Sized>(input: &[u8], n_chunks: usize, baked: Option<&P>) -> f64 {
    code_in_chunks(input, n_chunks, &|chunk| logistic_mix_bits_with(chunk, true, &[], baked))
}

/// [`compress_chunks_with`] with the second mixer beside the shipped one in
/// every chunk, each chunk reading its own rhythm and carrying its own side
/// information, since each is decoded on its own.
#[must_use]
pub fn compress_chunks_second_mixer_with<P: PriorLookup + Sync + ?Sized>(
    input: &[u8],
    n_chunks: usize,
    baked: Option<&P>,
) -> f64 {
    code_in_chunks(input, n_chunks, &|chunk| second_mixer_bits_with(chunk, baked))
}

/// The bits of `input` coded as `n_chunks` independent chunks by `code`, the
/// chunks across the cores.
fn code_in_chunks(input: &[u8], n_chunks: usize, code: &(dyn Fn(&[u8]) -> f64 + Sync)) -> f64 {
    let n = input.len();
    if n == 0 {
        return 0.0;
    }
    // n_chunks == 0 means "all logical cores" (the production default) - SMT
    // siblings included, since the coder is memory-latency-bound and siblings
    // help there.
    let auto = std::thread::available_parallelism().map_or(1, |p| p.get());
    let nc = (if n_chunks == 0 { auto } else { n_chunks }).clamp(1, n);
    let chunk = n.div_ceil(nc);
    let bounds: Vec<(usize, usize)> =
        (0..nc).map(|i| (i * chunk, ((i + 1) * chunk).min(n))).filter(|(a, b)| a < b).collect();
    let mut out = vec![0.0f64; bounds.len()];
    // Dispatched through Flynnel (the project scheduler) with an SMT-on plan so
    // its arena routes the latency-bound chunks across siblings; one chunk per
    // leaf (min_leaf = 1) since each is a full, heavy coder run.
    // `Gather` is pointer-chase work (the coder's hashmap lookups), so the
    // bisect splits one chunk per leaf and activates SMT siblings to
    // interleave the cache-miss loads, which measures 6% faster than
    // primaries alone.
    let plan = flynnel::JobPlan::new(0, out.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::Gather);
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut out, 1, |start, slice| {
        for (j, slot) in slice.iter_mut().enumerate() {
            let (a, b) = bounds[start + j];
            *slot = code(&input[a..b]);
        }
    });
    out.iter().sum()
}

/// The pole radius the second mixer's rhythm is read at: a memory of about two
/// tokens. Over seven real files, 0.7 codes shorter with the second mixer than
/// 0.6, 0.8, 0.85, 0.9, 0.95 or 0.98 does
/// (`examples/does_the_group_delay_select_the_weights --grid`).
pub const RHYTHM_R: f32 = 0.7;

/// The longest period the rhythm's bank holds, in significant tokens.
const RHYTHM_MAX_PERIOD: u16 = 32;

/// The shape of the second mixer's weight-set selector: how many of the byte
/// before's top bits it reads, and how many bins it cuts the rhythm's power and
/// the loudest band's group delay into. A value is those bits, crossed with the
/// power's bin, crossed with the group delay's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RhythmShape {
    /// The byte before's top bits read, from none to eight.
    pub byte_bits: u8,
    /// Bins the power is cut into at the stream's own quantiles, at least one.
    pub power_bins: u16,
    /// Fixed bins of the bank's memory the group delay is cut into, at least one.
    pub age_bins: u16,
}

impl RhythmShape {
    /// The shape `trex compress --second-mixer` codes with: the rhythm alone,
    /// sixteen power bins by sixteen group-delay bins. Over seven real files it
    /// is the best of the shapes that code no file longer than the default
    /// coder does; a shape reading more of the byte before codes the log
    /// shorter and Moby-Dick longer.
    pub const SHIPPED: RhythmShape = RhythmShape { byte_bits: 0, power_bins: 16, age_bins: 16 };

    /// The weight sets the shape's values choose among.
    #[must_use]
    pub fn sets(self) -> usize {
        (1usize << self.byte_bits) * usize::from(self.power_bins) * usize::from(self.age_bins)
    }

    /// Whether the shape reads the rhythm at all, and so needs the bank.
    fn reads_rhythm(self) -> bool {
        self.power_bins > 1 || self.age_bins > 1
    }
}

/// The second mixer's weight-set selector over one stretch the coder codes on
/// its own, and what a decoder must be sent before it can read the same one.
pub struct RhythmSelector {
    /// One value a byte, below [`Self::sets`]: the byte before's top bits,
    /// crossed with the bin of the rhythm's power and the bin of the loudest
    /// band's group delay, both read from the bank as it was at the last line
    /// a decoder holding the bytes before has whole.
    pub values: Vec<u16>,
    /// The weight sets the values choose among.
    pub sets: usize,
    /// The bits a decoder is sent to read the same values: the stream's set of
    /// token kinds, which sizes the bank, and the power's bin edges; none when
    /// the shape reads no rhythm.
    pub side_bits: u64,
}

/// The rhythm read once over one stretch the coder codes on its own, from which
/// a selector of any [`RhythmShape`] is cut: the stretch's significant tokens,
/// how many token kinds it holds, and after every token the bank's power summed
/// over the periods, as a log, and the loudest band's group delay as a share of
/// the bank's memory.
pub struct RhythmReadings {
    toks: Vec<crate::token::Token>,
    kinds: usize,
    powers: Vec<f32>,
    ages: Vec<f32>,
}

/// Which group delay a rhythm reading takes from the bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RhythmAge {
    /// The loudest band's own group delay.
    Loudest,
    /// Every band's group delay weighted by its power.
    PowerWeighted,
}

impl RhythmReadings {
    /// A bank at [`RHYTHM_R`] over `input`'s significant tokens' kinds, each
    /// kind numbered by its rank in the stream's kind set, read after every
    /// token for the loudest band's group delay: the readings the shipped
    /// selector is cut from.
    #[must_use]
    pub fn read(input: &[u8]) -> RhythmReadings {
        Self::read_with(input, RHYTHM_R, RhythmAge::Loudest)
    }

    /// [`Self::read`] with the bank at pole radius `r` and the group delay read
    /// as `age` says.
    ///
    /// # Panics
    ///
    /// When `r` is not strictly between zero and one.
    #[must_use]
    pub fn read_with(input: &[u8], r: f32, age: RhythmAge) -> RhythmReadings {
        assert!(r > 0.0 && r < 1.0, "a pole radius is strictly between 0 and 1, got {r}");
        let toks = rhythm_tokens(input);
        let mut kinds: Vec<u32> = toks.iter().map(|t| t.kind.code()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        let (powers, ages) = rhythm_readings(&toks, &kinds, r, age);
        RhythmReadings { toks, kinds: kinds.len(), powers, ages }
    }

    /// The selector of `shape` over `input`, the stretch these readings were
    /// read from. A byte reads the bank at the last line before it that a
    /// decoder has whole ([`rhythm_values`]), so the kind set and the power's
    /// bin edges, sent ahead, are all a decoder needs beyond the bytes it has
    /// decoded.
    ///
    /// # Panics
    ///
    /// When `shape` reads more than eight bits of the byte before, cuts a
    /// reading into no bins, or has more sets than a `u16` value can name.
    #[must_use]
    pub fn selector(&self, input: &[u8], shape: RhythmShape) -> RhythmSelector {
        assert!(shape.byte_bits <= 8, "a byte has eight bits, the shape reads {}", shape.byte_bits);
        assert!(shape.power_bins >= 1 && shape.age_bins >= 1, "every reading is cut into a bin or more: {shape:?}");
        assert!(shape.sets() <= usize::from(u16::MAX) + 1, "{shape:?} has more sets than a u16 names");
        let edges = power_edges(&self.powers, shape.power_bins);
        let ages = age_bins(&self.ages, shape.age_bins);
        let values = rhythm_values(input, &self.toks, &power_bins(&self.powers, &edges), &ages, shape);
        let side_bits = if shape.reads_rhythm() {
            32 * (u64::from(shape.power_bins) - 1) + 8 + 32 * self.kinds as u64
        } else {
            0
        };
        RhythmSelector { values, sets: shape.sets(), side_bits }
    }
}

/// The rhythm [`logistic_mix_bits_with_second_selector`] chooses the second
/// mixer's weights by, over `input`: the [`RhythmShape::SHIPPED`] selector,
/// the power cut into sixteen bins at the stream's own quantiles crossed with
/// the loudest band's group delay cut into sixteen fixed bins of the bank's
/// memory.
#[must_use]
pub fn rhythm_selector(input: &[u8]) -> RhythmSelector {
    RhythmReadings::read(input).selector(input, RhythmShape::SHIPPED)
}

/// The significant tokens the rhythm reads.
fn rhythm_tokens(input: &[u8]) -> Vec<crate::token::Token> {
    crate::lexer::lex(input).into_iter().filter(crate::token::Token::is_significant).collect()
}

/// Each token's rhythm, the bank at pole radius `r` read after the token goes
/// in: the power summed over the periods, as a log, and the group delay `which`
/// names as a share of the bank's memory. A token's kind is its rank in
/// `kinds`, which holds every kind among `toks`.
fn rhythm_readings(toks: &[crate::token::Token], kinds: &[u32], r: f32, which: RhythmAge) -> (Vec<f32>, Vec<f32>) {
    let periods = crate::resonator::default_periods(RHYTHM_MAX_PERIOD);
    let memory = r / (1.0 - r);
    let mut bank = crate::resonator::Bank::new(kinds.len(), &periods, r);
    let (mut gd, mut pw) = (vec![0.0f32; periods.len()], vec![0.0f32; periods.len()]);
    let mut powers = Vec::with_capacity(toks.len());
    let mut ages = Vec::with_capacity(toks.len());
    for tok in toks {
        let code = tok.kind.code();
        let rank = u32::try_from(kinds.partition_point(|&k| k < code)).expect("a kind's rank fits the bank's symbol");
        bank.push(rank);
        bank.read_into(&mut gd, &mut pw);
        let total: f32 = pw.iter().sum();
        let age = match which {
            RhythmAge::Loudest => pw.iter().zip(&gd).max_by(|a, b| a.0.total_cmp(b.0)).map_or(0.0, |(_, &g)| g),
            RhythmAge::PowerWeighted if total > f32::EPSILON => {
                gd.iter().zip(&pw).map(|(g, p)| g * p).sum::<f32>() / total
            }
            RhythmAge::PowerWeighted => 0.0,
        };
        ages.push(age / memory);
        powers.push((total + f32::EPSILON).ln());
    }
    (powers, ages)
}

/// The power's `bins - 1` bin edges: the stream's own quantiles, sent ahead of
/// it.
fn power_edges(powers: &[f32], bins: u16) -> Vec<f32> {
    let bins = usize::from(bins);
    let mut sorted = powers.to_vec();
    sorted.sort_unstable_by(f32::total_cmp);
    if sorted.is_empty() { Vec::new() } else { (1..bins).map(|k| sorted[k * sorted.len() / bins]).collect() }
}

/// Each power's bin: how many of `edges` it reaches.
fn power_bins(powers: &[f32], edges: &[f32]) -> Vec<u16> {
    powers
        .iter()
        .map(|&p| u16::try_from(edges.iter().filter(|&&e| p >= e).count()).expect("a bin count fits a u16"))
        .collect()
}

/// Each group delay's bin among `bins` fixed bins of the bank's memory, the
/// last taking every age past it.
fn age_bins(ages: &[f32], bins: u16) -> Vec<u16> {
    ages.iter().map(|&a| ((a * f32::from(bins)) as u16).min(bins - 1)).collect()
}

/// The selector's value at every byte of `input`: the byte before's top
/// `shape.byte_bits` bits, crossed with the bins `power_bins` and `ages` give
/// the last token read, one each a token of `toks`.
///
/// A byte reads the bank as it was after the last significant token ending
/// before the last newline below it that has no quote directly before it and
/// no backslash before it, directly or before its CR. No token spans such
/// a newline - a string closes on its own line, and only a backslash or a char
/// literal carries one past it - so the tokens before it are the tokens the
/// whole input's lex gives them, whatever follows: the cut the parallel and
/// streaming lexers make. A decoder holding the bytes before a byte therefore
/// reads the same value. A token read as soon as it ended could not promise
/// that, since the lexer can settle a token from bytes past its end
/// (examples/does_the_lexer_read_ahead).
fn rhythm_values(
    input: &[u8],
    toks: &[crate::token::Token],
    power_bins: &[u16],
    ages: &[u16],
    shape: RhythmShape,
) -> Vec<u16> {
    let mut values = vec![0u16; input.len()];
    let mut done = 0usize;
    let mut reading = (0u16, 0u16);
    for (t, slot) in values.iter_mut().enumerate() {
        if t > 0
            && input[t - 1] == b'\n'
            && !crate::lexer::backslash_before_newline(input, t - 1)
            && (t < 2 || input[t - 2] != b'\'')
        {
            while done < toks.len() && toks[done].end() < t {
                done += 1;
            }
            if done > 0 {
                reading = (power_bins[done - 1], ages[done - 1]);
            }
        }
        let top = if t == 0 { 0 } else { u16::from(input[t - 1]) >> (8 - shape.byte_bits) };
        *slot = (top * shape.power_bins + reading.0) * shape.age_bins + reading.1;
    }
    values
}

/// [`logistic_mix_bits_with`] with the second mixer beside the shipped one,
/// its weight sets chosen by [`rhythm_selector`], and the selector's side
/// information counted in the length.
#[must_use]
pub fn second_mixer_bits_with<P: PriorLookup + ?Sized>(input: &[u8], preload: Option<&P>) -> f64 {
    let selector = rhythm_selector(input);
    logistic_mix_bits_with_second_selector(input, true, &[], preload, &selector.values, selector.sets)
        + selector.side_bits as f64
}

#[must_use]
/// A slot in a direct-mapped context table: a checksum of the key's low bits
/// plus the two bit-counts. Replacing the per-bit `HashMap` lookup with a
/// shift-and-mask index into a flat array removes the hashing, the key
/// comparison, and the growth-rehash that the profile showed dominating the
/// coder, and touches one cache line per lookup instead of several. Counts are
/// bounded by the `> 1024` halving, so `u16` holds them.
#[derive(Clone, Copy, Default)]
struct CtxSlot {
    check: u16,
    n0: u16,
    n1: u16,
}

/// Cap on the direct-mapped table size (2^22 slots x 6 bytes = 24 MB per model).
/// The actual size is chosen per call from the input length (see `ctx_bits`).
const CTX_MAX_BITS: usize = 22;

pub fn logistic_mix_bits(
    input: &[u8],
    orbit_models: bool,
    warm_seed: &[u8],
    preload: Option<&[PreloadTable]>,
) -> f64 {
    logistic_mix_bits_with(input, orbit_models, warm_seed, preload)
}

/// [`logistic_mix_bits`] against any [`PriorLookup`]; the decoded tables and
/// the mapped prior code an input to the same bits.
pub fn logistic_mix_bits_with<P: PriorLookup + ?Sized>(
    input: &[u8],
    orbit_models: bool,
    warm_seed: &[u8],
    preload: Option<&P>,
) -> f64 {
    logistic_mix_core(input, orbit_models, warm_seed, preload, None, Selection::ByteBefore)
}

/// [`logistic_mix_bits_with`] with one more auxiliary predictor, keyed on a
/// context the caller supplies for every byte.
///
/// It enters as every auxiliary predictor does: a count table read per bit-tree
/// node, stretched, and mixed with a weight that starts at zero, so the mixer
/// raises it only where it predicts better than the models already there. That
/// makes this the additive test of a reading, where folding the reading into
/// the context models' keys would be the multiplicative one - a split divides
/// every context it touches, while a mixer input can only be down-weighted.
///
/// `aux[t]` is read with the context that codes byte `t`, so it must be
/// computable from the bytes before `t` alone, and there is one entry a byte
/// of the input the models see - the warm seed prepended to `input` when one
/// is given.
///
/// # Panics
///
/// When `aux` is not one context a byte of that input.
pub fn logistic_mix_bits_with_aux<P: PriorLookup + ?Sized>(
    input: &[u8],
    orbit_models: bool,
    warm_seed: &[u8],
    preload: Option<&P>,
    aux: &[u64],
) -> f64 {
    assert_eq!(aux.len(), warm_seed.len() + input.len(), "one auxiliary context a byte");
    logistic_mix_core(input, orbit_models, warm_seed, preload, Some((aux, false)), Selection::ByteBefore)
}

/// [`logistic_mix_bits_with_aux`] with the auxiliary predictor's mixer weight
/// shared across every weight set rather than learned once per set.
///
/// Every other input has a weight per selector value, which lets the mixer
/// trust a model more in one regime than another. The price for an input that
/// carries little is that 256 separate weights each have to learn it from the
/// fraction of bytes their selector sees, and while they learn they add noise:
/// an auxiliary input costs this mixer about 0.002 bits per byte even when it
/// carries nothing. One shared weight learns from every byte. What it gives up
/// is trusting the input differently by regime, so this asks which of the two
/// a weak reading needs more.
///
/// # Panics
///
/// As [`logistic_mix_bits_with_aux`].
pub fn logistic_mix_bits_with_shared_aux<P: PriorLookup + ?Sized>(
    input: &[u8],
    orbit_models: bool,
    warm_seed: &[u8],
    preload: Option<&P>,
    aux: &[u64],
) -> f64 {
    assert_eq!(aux.len(), warm_seed.len() + input.len(), "one auxiliary context a byte");
    logistic_mix_core(input, orbit_models, warm_seed, preload, Some((aux, true)), Selection::ByteBefore)
}

/// [`logistic_mix_bits_with`] with the mixer's weight set chosen by a selector
/// the caller supplies, rather than by the previous byte.
///
/// The mixer keeps one weight set per selector value so it can weight its
/// models differently in different regimes, and the shipped selector is the
/// byte before, a regime read at one byte's range. A reading of the stream's
/// state over a longer range can stand in for it or refine it; this is how
/// such a reading is tested where a regime reading belongs, which is the
/// choice of weights rather than a prediction of its own.
///
/// `selector[t]` must be computable from the bytes before `t`, below `sets`,
/// with one entry a byte of the input the models see - the warm seed
/// prepended to `input` when one is given. The calibration stage is keyed on
/// the same selector, as it is on the byte before in the shipped coder.
///
/// # Panics
///
/// When `selector` is not one value a byte of that input, or a value is not
/// below `sets`.
pub fn logistic_mix_bits_with_selector<P: PriorLookup + ?Sized>(
    input: &[u8],
    orbit_models: bool,
    warm_seed: &[u8],
    preload: Option<&P>,
    selector: &[u16],
    sets: usize,
) -> f64 {
    assert_eq!(selector.len(), warm_seed.len() + input.len(), "one selector value a byte");
    assert!(
        selector.iter().all(|&s| usize::from(s) < sets),
        "every selector value below the {sets} weight sets"
    );
    logistic_mix_core(input, orbit_models, warm_seed, preload, None, Selection::Instead(selector, sets))
}

/// [`logistic_mix_bits_with`] with a second mixer beside the shipped one, its
/// weight set chosen by a selector the caller supplies, and a final mixer of
/// two weights over the two mixers' outputs.
///
/// A regime reading can choose the weights better than the byte before on one
/// kind of stream and worse on another, so selecting on it alone is a bet on
/// which stream is being coded. Here both mixers see the same inputs and differ
/// only in how their weights are chosen, and the final mixer learns how far to
/// trust each, which is the arrangement PAQ8's mixer uses: several weight sets
/// chosen by different contexts, and a mixer over their outputs. The
/// calibration stage stays keyed on the byte before.
///
/// `selector[t]` must be computable from the bytes before `t`, below `sets`,
/// with one value a byte of the input the models see - the warm seed prepended
/// to `input` when one is given.
///
/// # Panics
///
/// As [`logistic_mix_bits_with_selector`].
pub fn logistic_mix_bits_with_second_selector<P: PriorLookup + ?Sized>(
    input: &[u8],
    orbit_models: bool,
    warm_seed: &[u8],
    preload: Option<&P>,
    selector: &[u16],
    sets: usize,
) -> f64 {
    logistic_mix_bits_with_mixers_beside(input, orbit_models, warm_seed, preload, &[(selector, sets)])
}

/// [`logistic_mix_bits_with_second_selector`] with any number of mixers beside
/// the shipped one, each `(selector, sets)` choosing one mixer's weight set,
/// and a final mixer over the shipped mixer's output and theirs, its weights
/// starting at an equal share each. With none it is the shipped coder.
///
/// # Panics
///
/// As [`logistic_mix_bits_with_selector`], for every selector.
pub fn logistic_mix_bits_with_mixers_beside<P: PriorLookup + ?Sized>(
    input: &[u8],
    orbit_models: bool,
    warm_seed: &[u8],
    preload: Option<&P>,
    beside: &[(&[u16], usize)],
) -> f64 {
    for &(selector, sets) in beside {
        assert_eq!(selector.len(), warm_seed.len() + input.len(), "one selector value a byte");
        assert!(
            selector.iter().all(|&s| usize::from(s) < sets),
            "every selector value below the {sets} weight sets"
        );
    }
    let selection = if beside.is_empty() { Selection::ByteBefore } else { Selection::Beside(beside) };
    logistic_mix_core(input, orbit_models, warm_seed, preload, None, selection)
}

/// How the mixer's weight set is chosen for each bit.
#[derive(Clone, Copy)]
enum Selection<'a> {
    /// By the byte before, over 256 sets: the shipped coder.
    ByteBefore,
    /// By the caller's selector in place of the byte before, over its sets.
    Instead(&'a [u16], usize),
    /// By the byte before, with more mixers beside it, each one's set chosen by
    /// its own selector over its own count of sets, and a final mixer over them
    /// all.
    Beside(&'a [(&'a [u16], usize)]),
}

fn logistic_mix_core<P: PriorLookup + ?Sized>(
    input: &[u8],
    orbit_models: bool,
    warm_seed: &[u8],
    preload: Option<&P>,
    aux: Option<(&[u64], bool)>,
    selection: Selection<'_>,
) -> f64 {
    // When `warm_seed` is non-empty, prepend it; the models train on it but no
    // bits are charged for it (the decoder runs the identical warm-up), so the
    // per-document size excludes the seed. The seed is a shared prior (a rich
    // disjoint corpus warmed through the same online algorithm), not a merge
    // dictionary - it must be disjoint from the input to avoid train-on-test.
    let combined: Vec<u8>;
    let (input, warm_len): (&[u8], usize) = if warm_seed.is_empty() {
        (input, 0)
    } else {
        combined = [warm_seed, input].concat();
        (&combined[..], warm_seed.len())
    };
    // (kind, order): 0 = literal, 1 = case fold, 2 = shape, 3 = word stem.
    let mut specs: Vec<(u8, usize)> = vec![(0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 6), (0, 8)];
    if orbit_models {
        specs.extend_from_slice(&[(1, 4), (2, 3), (2, 6), (3, 0)]);
    }
    let m = specs.len();
    // Mixer inputs: the m context models, one match (LZP) model at index m, and
    // one neural trace predictor at index m+1.
    // Learned embeddings at two context orders. Each (hashed, order-o)
    // context owns an EMB_DIM vector trained online by the coder's error, so
    // predictive-similar contexts migrate together; the higher orders are
    // sparse, which is where the count models are empty. The vectors are
    // plain gradient-trained embeddings with no lattice structure imposed on
    // them; `crate::e8` is used by the orbit rung, not here.
    const EMB_ORDERS: [usize; 2] = [5, 8];
    const N_EMB: usize = 2;
    const EMB_BITS: usize = 18;
    const EMB_LR: f64 = 0.03;
    // Sixteen dimensions per embedding.
    const EMB_DIM: usize = 16;
    let neural_idx = m + 1;
    let learn0 = m + 2;
    let shape_idx = m + 2 + N_EMB;
    // A caller's auxiliary context takes the slot after the shape input. With
    // none, the mixer holds exactly the inputs it always has, so a call without
    // one codes to the same bits it always did.
    let aux_idx = m + 3 + N_EMB;
    let n_in = m + 3 + N_EMB + usize::from(aux.is_some());
    let emb_mask = (1usize << EMB_BITS) - 1;
    // The embedding is stored in 24-bit fixed-point (value = int / 2^24) so the
    // bilinear predict-dot is an integer reduction that auto-vectorizes (the f64
    // dot could not, without breaking reassociation). The gradient is computed in
    // f64 and applied to the fixed-point store, so small learning updates do not
    // underflow the quantization.
    const EMB_FRAC: f64 = (1u64 << 24) as f64;
    // Pulling each embedding toward its nearest E8 lattice point measures at
    // best 23 bytes in 731 437 on a 3 MB enwik8 slice, so the embeddings are
    // left to the gradient alone.
    let mut emb: Vec<Vec<[i32; EMB_DIM]>> = (0..N_EMB)
        .map(|oi| {
            let mut s = 0x1234_5678_9abc_def0u64 ^ (oi as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            (0..=emb_mask)
                .map(|_| {
                    std::array::from_fn(|_| {
                        s = s
                            .wrapping_mul(6_364_136_223_846_793_005)
                            .wrapping_add(1_442_695_040_888_963_407);
                        (((s >> 40) as f64 / f64::from(1u32 << 24) - 0.5) * 0.2 * EMB_FRAC) as i32
                    })
                })
                .collect()
        })
        .collect();
    let mut wnode: Vec<Vec<[i32; EMB_DIM]>> =
        (0..N_EMB).map(|_| vec![[0i32; EMB_DIM]; 256]).collect();
    // Per-call count tables are a small copy-on-write delta over the shared
    // read-only baked prior (`prior_order` names each model's table) rather than a clone of it. N
    // parallel chunks share one prior in memory instead of cloning it N times,
    // so scaling is per-core with no per-chunk clone cost. Lookups check the delta
    // then fall back to the baked map; the first write copies the baked count
    // into the delta and increments, so the result is bit-identical to a clone.
    // Size the direct-mapped tables to the input: ~16x the byte count keeps the
    // load factor low (few collisions) while a small input gets a small table
    // that stays in cache. Chunked compression sizes each chunk's tables this
    // way, so N chunks do not each allocate a full-size table.
    let ctx_bits = (input.len().max(1).ilog2() as usize + 4).clamp(16, CTX_MAX_BITS);
    let ctx_size = 1usize << ctx_bits;
    let ctx_mask = ctx_size - 1;
    let mut tables: Vec<Vec<CtxSlot>> = (0..m).map(|_| vec![CtxSlot::default(); ctx_size]).collect();
    // The literal order each model reads from the prior, or `None` for an
    // orbit model or an order the prior does not hold.
    let prior_order: Vec<Option<usize>> = specs
        .iter()
        .map(|&(kind, k)| match preload {
            Some(pre) if kind == 0 && pre.has_order(k) => Some(k),
            _ => None,
        })
        .collect();
    // Shape axis (consuming the tokenizer's silhouette `crate::shape::shape_class`):
    // a leak-free structural context, the silhouette of the last completed
    // tokens before t (never the covering token, which would peek at future
    // bytes). In a periodic template region (a CSV row, a struct block) the
    // silhouette predicts the next byte's structure where byte n-grams are
    // weak. It enters as a zero-initialized auxiliary input, so the mixer
    // raises its weight only where it helps. Off unless `TREX_SHAPE=1`: on
    // prose, code and CSV it measures neutral to slightly negative, the byte
    // models already carrying the structure it sees.
    let shape_on = match std::env::var("TREX_SHAPE") {
        Ok(v) => v == "1",
        Err(std::env::VarError::NotPresent) => false,
        Err(e) => panic!("TREX_SHAPE cannot be read: {e}"),
    };
    let shape_ctx: Vec<u64> = if shape_on {
        // The silhouette can be taken over an orbit quotient
        // (TREX_SHAPE_ORBIT=identity|case|notation|shape); default Case folds
        // case before keying, the new orbit-over-shape composition axis. A value
        // that cannot be read or names no group stops the run rather than
        // coding under a group nobody asked for.
        let group = match std::env::var("TREX_SHAPE_ORBIT") {
            Err(std::env::VarError::NotPresent) => crate::orbit::OrbitGroup::Case,
            Err(e) => panic!("TREX_SHAPE_ORBIT cannot be read: {e}"),
            Ok(s) => crate::orbit::OrbitGroup::parse(&s)
                .unwrap_or_else(|| panic!("TREX_SHAPE_ORBIT={s:?} names no orbit group")),
        };
        let toks = crate::lexer::lex(input);
        let cls: Vec<u32> = toks
            .iter()
            .map(|tk| crate::shape::shape_class_over(tk.kind, &input[tk.span()], group))
            .collect();
        let mut ctx = vec![0u64; input.len()];
        let mut completed = 0usize;
        for (t, slot) in ctx.iter_mut().enumerate() {
            // Strictly `< t`: a token ending exactly at t had its boundary decided
            // by byte t (the byte being predicted), which the decoder cannot know
            // yet - using it would leak. Only tokens fully resolved before t count.
            while completed < toks.len() && toks[completed].end() < t {
                completed += 1;
            }
            let c0 = if completed >= 1 { u64::from(cls[completed - 1]) } else { 0 };
            let c1 = if completed >= 2 { u64::from(cls[completed - 2]) } else { 0 };
            *slot = ((c0 << 32) | c1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
        ctx
    } else {
        Vec::new()
    };
    let mut shape_table: U64Map<(u32, u32)> = U64Map::default();
    let mut aux_table: U64Map<(u32, u32)> = U64Map::default();
    // When the auxiliary predictor's weight is shared, it lives here and the
    // per-set weight for it stays at zero, so the input enters the mix once.
    let aux_shared = aux.is_some_and(|(_, shared)| shared);
    let mut aux_w: i32 = 0;
    // Neural trace predictor (ported online architecture): per bit-tree node, a
    // fast / medium / slow exponential trace of the bit seen there, combined by
    // gradient-adapted weights - a long-range multi-timescale signal the
    // fixed-order contexts do not carry. Corpus-free (no shipped weights); the
    // 240-unit forward model maps onto the 256 bit-tree nodes.
    const NT_DECAY: [f64; 3] = [0.7, 0.9, 0.97];
    const NT_LR: f64 = 0.01;
    let mut nt_trace = vec![[0.0f64; 3]; 256];
    let mut nt_w = vec![[0.34f64, 0.33, 0.33]; 256];
    // One mixer weight set per selector (the previous byte), so the mixer
    // specializes by local regime. Context models and match start at the global
    // default; the auxiliary predictors (neural, CTW) start at zero so they are
    // ignored until the mixer finds them useful - they can help, never dilute.
    // Fixed-point mixer weights (16 fractional bits): the context models start
    // at 0.3 (= 0.3 * 65536), the auxiliary predictors at 0 so they can only
    // help. Integer weights make the dot-product associative and SIMD-bit-exact.
    const W_ONE: i32 = 1 << 16;
    // A caller's selector in place of the byte before brings its own count of
    // weight sets; the byte before brings 256.
    let n_sel = match selection {
        Selection::Instead(_, sets) => sets,
        Selection::ByteBefore | Selection::Beside(..) => 256,
    };
    let fresh = {
        let mut w = vec![(0.3 * f64::from(W_ONE)) as i32; n_in];
        w[neural_idx] = 0;
        for oi in 0..N_EMB {
            w[learn0 + oi] = 0;
        }
        w[shape_idx] = 0;
        if aux.is_some() {
            w[aux_idx] = 0;
        }
        w
    };
    let mut weights: Vec<Vec<i32>> = vec![fresh.clone(); n_sel];
    // The mixers beside the shipped one, each with its weight sets starting
    // where the shipped mixer's do, and the final mixer's weights over every
    // mixer's stretched output, the shipped one's first, starting at an equal
    // share each so the final mix begins as their mean.
    let beside: &[(&[u16], usize)] = match selection {
        Selection::Beside(list) => list,
        Selection::ByteBefore | Selection::Instead(..) => &[],
    };
    let mut beside_w: Vec<Vec<Vec<i32>>> = beside.iter().map(|&(_, sets)| vec![fresh.clone(); sets]).collect();
    let mixers = 1 + beside.len();
    let mut final_w = vec![W_ONE / i32::try_from(mixers).expect("a handful of mixers"); mixers];
    // Every mixer's output this bit, the shipped one's first, and the set each
    // mixer beside it chose: written a bit at a time, allocated once.
    let mut outs = vec![0i32; mixers];
    let mut chosen = vec![0usize; beside.len()];
    let mut apm = Apm::new(n_sel);
    let mut bases = vec![0u64; m];
    let mut st = vec![0i32; n_in];
    let mut emb_logit = [0.0f64; N_EMB];
    // Match model (LZP): a hash of the last MATCH_H bytes maps to the most
    // recent position that context was followed; while a match holds, the
    // predicted byte advances in lockstep and confidence grows with its length.
    const MATCH_H: usize = 8;
    const MATCH_TAG: u64 = 0x6D61_7463;
    let mut htable: U64Map<usize> = U64Map::default();
    let mut mp = 0usize;
    let mut ml = 0usize;
    const APM_RATE: f64 = 0.04;
    const MIX: u64 = 0x9E37_79B9_7F4A_7C15;
    // Hoist the fixed-point tables once per call (not per lookup) - the OnceLock
    // fetch on every one of ~57M stretches was the scalar-integer overhead.
    let (stab, sqtab) = stretch_squash_tables();
    let stretch_i = |p12: i32| i32::from(stab[p12.clamp(0, 4095) as usize]);
    let squash_i = |x: i32| i32::from(sqtab[(x.clamp(-2048, 2047) + 2048) as usize]).clamp(1, 4095);
    let mut bits = 0.0f64;
    for t in 0..input.len() {
        for (mi, &(kind, k)) in specs.iter().enumerate() {
            let tag = (u64::from(kind) << 8) | k as u64;
            bases[mi] = match kind {
                1 => {
                    let kk = k.min(t);
                    let f: Vec<u8> = input[t - kk..t].iter().map(u8::to_ascii_lowercase).collect();
                    ctx_hash(tag, &f)
                }
                2 => {
                    let kk = k.min(t);
                    let s: Vec<u8> =
                        input[t - kk..t].iter().map(|&b| crate::orbit::shape_char(b) as u8).collect();
                    ctx_hash(tag, &s)
                }
                3 => {
                    // The current word prefix (lowercased alnum run ending at t):
                    // the morphological orbit, so inflections share a stem prefix.
                    let mut s = t;
                    while s > 0 && input[s - 1].is_ascii_alphanumeric() && t - s < 32 {
                        s -= 1;
                    }
                    let w: Vec<u8> = input[s..t].iter().map(u8::to_ascii_lowercase).collect();
                    ctx_hash(tag, &w)
                }
                _ => {
                    let kk = k.min(t);
                    ctx_hash(tag, &input[t - kk..t])
                }
            };
        }
        let sel = match selection {
            Selection::Instead(values, _) => usize::from(values[t]),
            Selection::ByteBefore | Selection::Beside(..) if t > 0 => input[t - 1] as usize,
            Selection::ByteBefore | Selection::Beside(..) => 0,
        };
        // Hashed context index per order for the learned embeddings.
        let mut ec = [0usize; N_EMB];
        for (oi, &o) in EMB_ORDERS.iter().enumerate() {
            ec[oi] = ctx_hash(0x1d ^ o as u64, &input[t - o.min(t)..t]) as usize & emb_mask;
        }
        // Match model: hash the current order-MATCH_H context once; acquire a
        // match pointer if none is active, then read the predicted byte.
        let mh = if t >= MATCH_H { Some(ctx_hash(MATCH_TAG, &input[t - MATCH_H..t])) } else { None };
        if ml == 0
            && let Some(h) = mh
            && let Some(&p) = htable.get(&h)
            && p < t
        {
            mp = p;
            ml = MATCH_H;
        }
        let predicted = if ml > 0 && mp < t { Some(input[mp]) } else { None };
        let mut matched = predicted.is_some();
        let byte = input[t];
        let mut node = 1u64;
        for bit_idx in (0..8).rev() {
            let actual = (byte >> bit_idx) & 1;
            // The stretched mixer inputs are fixed-point, mixed in integer
            // arithmetic (integer addition is associative, so the dot is
            // SIMD-bit-exact).
            for mi in 0..m {
                let key = bases[mi].wrapping_mul(MIX).wrapping_add(node);
                let idx = (key as usize) & ctx_mask;
                let chk = (key >> ctx_bits) as u16;
                let slot = tables[mi][idx];
                let (n0, n1) = if slot.check == chk {
                    (u32::from(slot.n0), u32::from(slot.n1))
                } else {
                    // A key new to this slot: seed the prediction from the baked
                    // prior when present, else the uniform (0, 0).
                    //
                    // Counted here because this branch is how often the prior is
                    // actually read. The bit loop runs six orders by eight bits
                    // per byte, but those are slot reads; the prior is reached
                    // only when a slot holds a different context. The increment
                    // is next to a hash lookup and is absent from the hit path.
                    PRIOR_READS.with(|c| c.set(c.get() + 1));
                    match (prior_order[mi], preload) {
                        (Some(k), Some(pre)) => {
                            note_prior_key(k, key);
                            pre.get(k, key).unwrap_or((0, 0))
                        }
                        _ => (0, 0),
                    }
                };
                let p12 = (u64::from(2 * n1 + 1) * 4096 / u64::from(2 * (n0 + n1) + 2)) as i32;
                st[mi] = stretch_i(p12);
            }
            // Match model input: while still consistent with the predicted byte,
            // the next bit is that byte's bit, confidence rising with ml.
            st[m] = if matched {
                let pb = predicted.unwrap_or(0);
                let s = (ml.min(28) as i32 * 82).min(2047);
                if (pb >> bit_idx) & 1 == 1 { s } else { -s }
            } else {
                0
            };
            // Neural trace predictor input (keyed by the bit-tree node).
            let ni = (node as usize) & 0xFF;
            let p_nt = (nt_w[ni][0] * nt_trace[ni][0]
                + nt_w[ni][1] * nt_trace[ni][1]
                + nt_w[ni][2] * nt_trace[ni][2])
                .clamp(1e-3, 1.0 - 1e-3);
            st[neural_idx] = stretch_i((p_nt * 4096.0) as i32);
            // Learned E8 embedding inputs (bilinear logit e_ctx . w_node) per order.
            // The embedding keeps learning in f64; only its mixer input quantizes.
            let nn = (node as usize) & 0xFF;
            for oi in 0..N_EMB {
                // Integer bilinear dot (i32 * i32 -> i64, associative -> auto-SIMD).
                let e = &emb[oi][ec[oi]];
                let w = &wnode[oi][nn];
                let dot_i: i64 = (0..EMB_DIM).map(|k| i64::from(e[k]) * i64::from(w[k])).sum();
                let logit_l = (dot_i as f64 / (EMB_FRAC * EMB_FRAC)).clamp(-12.0, 12.0);
                emb_logit[oi] = logit_l;
                st[learn0 + oi] = ((logit_l * 256.0) as i32).clamp(-2047, 2047);
            }
            // Shape silhouette context input (zero-init aux).
            st[shape_idx] = if shape_on {
                let key = shape_ctx[t].wrapping_mul(MIX).wrapping_add(node);
                let (n0, n1) = *shape_table.get(&key).unwrap_or(&(0, 0));
                let p12 = (u64::from(2 * n1 + 1) * 4096 / u64::from(2 * (n0 + n1) + 2)) as i32;
                stretch_i(p12)
            } else {
                0
            };
            // The caller's auxiliary context, read the way the shape input is.
            if let Some((a, _)) = aux {
                let key = a[t].wrapping_mul(MIX).wrapping_add(node);
                let (n0, n1) = *aux_table.get(&key).unwrap_or(&(0, 0));
                let p12 = (u64::from(2 * n1 + 1) * 4096 / u64::from(2 * (n0 + n1) + 2)) as i32;
                st[aux_idx] = stretch_i(p12);
            }
            let mut dot: i64 = 0;
            for i in 0..n_in {
                dot += i64::from(weights[sel][i]) * i64::from(st[i]);
            }
            if aux_shared {
                dot += i64::from(aux_w) * i64::from(st[aux_idx]);
            }
            let x = (dot >> 16).clamp(-2047, 2047) as i32;
            let p12_mix = squash_i(x);
            // Where there are mixers beside the shipped one, each reads the same
            // inputs under its own weight set, and the final mixer takes every
            // mixer's stretched output; the clamped dot is a mixer's output
            // before it is squashed.
            let p12_out = if beside.is_empty() {
                p12_mix
            } else {
                outs[0] = x;
                for (k, &(values, _)) in beside.iter().enumerate() {
                    let s = usize::from(values[t]);
                    chosen[k] = s;
                    let mut dot_k: i64 = 0;
                    for i in 0..n_in {
                        dot_k += i64::from(beside_w[k][s][i]) * i64::from(st[i]);
                    }
                    outs[k + 1] = (dot_k >> 16).clamp(-2047, 2047) as i32;
                }
                let mut dot_final: i64 = 0;
                for (&w, &o) in final_w.iter().zip(&outs) {
                    dot_final += i64::from(w) * i64::from(o);
                }
                squash_i((dot_final >> 16).clamp(-2047, 2047) as i32)
            };
            let pm = f64::from(p12_out) / 4096.0;
            let (pa, apm_idx) = apm.refine(pm, sel);
            let p = ((pm + 3.0 * pa) / 4.0).clamp(1e-6, 1.0 - 1e-6);
            let pbit = if actual == 1 { p } else { 1.0 - p };
            if t >= warm_len {
                bits += -pbit.log2();
            }
            let err12 = (i32::from(actual) << 12) - p12_mix;
            // Integer mixer-weight update over all inputs (LR_eff = 0.02 via the
            // *82 >> 16 scale): one contiguous loop - the SIMD-bit-exact update.
            for i in 0..n_in {
                weights[sel][i] += ((i64::from(err12) * i64::from(st[i]) * 82) >> 16) as i32;
            }
            if aux_shared {
                // Held at zero after the update the loop gave it, so the input
                // is weighted by the shared weight alone; the loop stays one
                // contiguous pass.
                weights[sel][aux_idx] = 0;
                aux_w += ((i64::from(err12) * i64::from(st[aux_idx]) * 82) >> 16) as i32;
            }
            // Each mixer beside learns from its own output's error, and the
            // final mixer from the output it gave, at the same rate.
            if !beside.is_empty() {
                for (k, sets_k) in beside_w.iter_mut().enumerate() {
                    let err_k = (i32::from(actual) << 12) - squash_i(outs[k + 1]);
                    let w = &mut sets_k[chosen[k]];
                    for i in 0..n_in {
                        w[i] += ((i64::from(err_k) * i64::from(st[i]) * 82) >> 16) as i32;
                    }
                }
                let err_final = (i32::from(actual) << 12) - p12_out;
                for (w, &o) in final_w.iter_mut().zip(&outs) {
                    *w += ((i64::from(err_final) * i64::from(o) * 82) >> 16) as i32;
                }
            }
            // Context-model count tables: bump the slot, seeding a slot new to
            // this key from the shared baked prior (checksum-based eviction).
            for mi in 0..m {
                let key = bases[mi].wrapping_mul(MIX).wrapping_add(node);
                let idx = (key as usize) & ctx_mask;
                let chk = (key >> ctx_bits) as u16;
                let slot = &mut tables[mi][idx];
                if slot.check != chk {
                    let (b0, b1) = match (prior_order[mi], preload) {
                        (Some(k), Some(pre)) => {
                            note_prior_key(k, key);
                            pre.get(k, key).unwrap_or((0, 0))
                        }
                        _ => (0, 0),
                    };
                    slot.check = chk;
                    slot.n0 = b0.min(1024) as u16;
                    slot.n1 = b1.min(1024) as u16;
                }
                if actual == 1 {
                    slot.n1 += 1;
                } else {
                    slot.n0 += 1;
                }
                if slot.n0 + slot.n1 > 1024 {
                    slot.n0 = slot.n0.div_ceil(2);
                    slot.n1 = slot.n1.div_ceil(2);
                }
            }
            // Shape silhouette count table.
            if shape_on {
                let key = shape_ctx[t].wrapping_mul(MIX).wrapping_add(node);
                let e = shape_table.entry(key).or_insert((0, 0));
                if actual == 1 {
                    e.1 += 1;
                } else {
                    e.0 += 1;
                }
                if e.0 + e.1 > 1024 {
                    e.0 = e.0.div_ceil(2);
                    e.1 = e.1.div_ceil(2);
                }
            }
            // The auxiliary context's count table, bumped the same way.
            if let Some((a, _)) = aux {
                let key = a[t].wrapping_mul(MIX).wrapping_add(node);
                let e = aux_table.entry(key).or_insert((0, 0));
                if actual == 1 {
                    e.1 += 1;
                } else {
                    e.0 += 1;
                }
                if e.0 + e.1 > 1024 {
                    e.0 = e.0.div_ceil(2);
                    e.1 = e.1.div_ceil(2);
                }
            }
            // Online gradient on each order's embedding (f64).
            for oi in 0..N_EMB {
                let el = f64::from(actual) - squash(emb_logit[oi]);
                let ci = ec[oi];
                for k in 0..EMB_DIM {
                    let ev = emb[oi][ci][k];
                    let wv = wnode[oi][nn][k];
                    emb[oi][ci][k] = ev + (EMB_LR * el * f64::from(wv)) as i32;
                    wnode[oi][nn][k] = wv + (EMB_LR * el * f64::from(ev)) as i32;
                }
            }
            // Neural predictor self-update: a gradient step on its own error
            // (normalized, non-negative weights), then observe the bit into the
            // multi-timescale traces.
            let nt_err = f64::from(actual) - p_nt;
            let mut wsum = 0.0;
            for d in 0..3 {
                nt_w[ni][d] = (nt_w[ni][d] + NT_LR * nt_err * nt_trace[ni][d]).max(0.01);
                wsum += nt_w[ni][d];
            }
            for d in 0..3 {
                nt_w[ni][d] /= wsum;
                nt_trace[ni][d] =
                    NT_DECAY[d] * nt_trace[ni][d] + (1.0 - NT_DECAY[d]) * f64::from(actual);
            }
            apm.update(apm_idx, actual, APM_RATE);
            if matched && ((predicted.unwrap_or(0) >> bit_idx) & 1) != actual {
                matched = false;
            }
            node = (node << 1) | u64::from(actual);
        }
        // Match update: extend on a full-byte match, else drop; record the
        // current context as followed by this position (lookup happened first).
        if let Some(pb) = predicted {
            if pb == byte {
                ml = (ml + 1).min(65535);
                mp += 1;
            } else {
                ml = 0;
            }
        } else {
            ml = 0;
        }
        if let Some(h) = mh {
            htable.insert(h, t);
        }
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seam::{adaptive_byte_bits, ppm_byte_bits};

    /// Front-coding a key stream must return the model that went in.
    ///
    /// The encoding writes how many leading bytes a context shares with the one
    /// before it and then only the rest, so a reader that reconstructs wrongly
    /// produces contexts that are the right length and the wrong bytes - which
    /// hashes to a valid key and answers every lookup with a plausible count.
    /// Nothing downstream would notice, so the round trip is checked here on
    /// contexts built to share long prefixes, short ones, and none.
    #[test]
    fn front_coded_keys_round_trip_to_the_same_model() {
        let mut model: Vec<NgramCounts> = MODEL_ORDERS.iter().map(|_| NgramCounts::new()).collect();
        for (oi, &k) in MODEL_ORDERS.iter().enumerate() {
            for a in 0u8..40 {
                // Contexts sharing every byte but the last, so the coded suffix
                // is one byte and the shared run is k-1.
                let mut ctx = vec![0xAAu8; k];
                ctx[k - 1] = a;
                let mut m = std::collections::BTreeMap::new();
                m.insert(a, u32::from(a) + 1);
                m.insert(a ^ 0x5A, 7);
                model[oi].insert(ctx, m);
                // And contexts sharing nothing with their neighbor.
                let mut lone = vec![a; k];
                lone[0] = a ^ 0xF0;
                let mut m2 = std::collections::BTreeMap::new();
                m2.insert(a, 3);
                model[oi].insert(lone, m2);
            }
        }
        let widest = model.iter().map(std::collections::BTreeMap::len).max().unwrap_or(0);
        let blob = serialize_byte_ngram(&model, widest);
        let back = decode_baked(&blob);
        assert_eq!(back.len(), model.len(), "order count changed");
        for (oi, (want, got)) in model.iter().zip(&back).enumerate() {
            assert_eq!(want, got, "order {} did not survive the round trip", MODEL_ORDERS[oi]);
        }
    }

    /// The runtime reader and the offline reader must agree on a coded stream.
    ///
    /// They parse the same bytes through different code: `decode_baked` rebuilds
    /// contexts and counts, `load_byte_ngram` expands straight into bit-tree
    /// tables across one worker per order. A reconstruction bug in one and not
    /// the other would leave the shipped prior disagreeing with every offline
    /// tool that inspects it.
    #[test]
    fn both_readers_agree_on_a_front_coded_stream() {
        let mut model: Vec<NgramCounts> = MODEL_ORDERS.iter().map(|_| NgramCounts::new()).collect();
        for (oi, &k) in MODEL_ORDERS.iter().enumerate() {
            for a in 0u8..24 {
                let mut ctx = vec![0x11u8; k];
                ctx[k - 1] = a;
                let mut m = std::collections::BTreeMap::new();
                m.insert(b'x', u32::from(a) + 2);
                model[oi].insert(ctx, m);
            }
        }
        let widest = model.iter().map(std::collections::BTreeMap::len).max().unwrap_or(0);
        let blob = serialize_byte_ngram(&model, widest);
        let tables = load_byte_ngram(&blob);
        let decoded = decode_baked(&blob);
        for (oi, (k, table)) in tables.iter().enumerate() {
            // Every context the offline reader found must key a live entry in
            // the runtime reader's table, at the root of its bit tree.
            for ctx in decoded[oi].keys() {
                let base = ctx_hash(*k as u64, ctx);
                let root = base.wrapping_mul(NODE_MIX).wrapping_add(1);
                assert!(
                    table.contains_key(&root),
                    "order {} context {ctx:?} is in the decoded model and not in the loaded table",
                    MODEL_ORDERS[oi]
                );
            }
        }
    }

    /// An auxiliary context reaches the mixer, and its absence leaves the
    /// mixer exactly as it was.
    ///
    /// The second half is what lets a difference be read as the context's: a
    /// call without one takes the same path the shipped entry point does. The
    /// first half is what keeps a measurement honest - an input the mixer never
    /// read would score "no effect" on every corpus and look like a finding.
    #[test]
    fn an_auxiliary_context_is_read_and_its_absence_changes_nothing() {
        let input: Vec<u8> = b"alpha beta gamma delta alpha beta gamma delta epsilon "
            .iter()
            .copied()
            .cycle()
            .take(4000)
            .collect();
        let plain = logistic_mix_bits(&input, false, &[], None);
        let without = logistic_mix_bits_with(&input, false, &[], None::<&[PreloadTable]>);
        assert!(
            (plain - without).abs() < 1e-9,
            "no auxiliary context must code as the shipped entry point: {plain} against {without}"
        );
        // A context that tells the position within the 55-byte cycle, which a
        // decoder knows before each byte. The mixer must read it for the bits
        // to move at all.
        let phase: Vec<u64> = (0..input.len()).map(|t| (t % 55) as u64 + 1).collect();
        let with = logistic_mix_bits_with_aux(&input, false, &[], None::<&[PreloadTable]>, &phase);
        assert!(with.is_finite() && with > 0.0, "a finite code length, got {with}");
        assert!((with - plain).abs() > 1e-6, "the auxiliary context must be read: {with} against {plain}");
        // Through one shared weight the same context codes differently again,
        // which is what says the shared path is the one taken.
        let shared =
            logistic_mix_bits_with_shared_aux(&input, false, &[], None::<&[PreloadTable]>, &phase);
        assert!(shared.is_finite() && shared > 0.0, "a finite code length, got {shared}");
        assert!(
            (shared - with).abs() > 1e-6,
            "a shared weight must be a different mix from one a set: {shared} against {with}"
        );
    }

    /// The byte before, handed in as a selector over 256 weight sets, is the
    /// shipped coder exactly - so a selector that codes differently does so by
    /// what it selects on and not by the path it took.
    #[test]
    fn the_byte_before_as_a_selector_codes_as_the_shipped_mixer() {
        let input: Vec<u8> = b"alpha beta gamma delta alpha beta gamma delta epsilon "
            .iter()
            .copied()
            .cycle()
            .take(4000)
            .collect();
        let shipped = logistic_mix_bits(&input, false, &[], None);
        let before: Vec<u16> =
            (0..input.len()).map(|t| if t == 0 { 0 } else { u16::from(input[t - 1]) }).collect();
        let selected =
            logistic_mix_bits_with_selector(&input, false, &[], None::<&[PreloadTable]>, &before, 256);
        assert!(
            (shipped - selected).abs() < 1e-9,
            "the byte before as a selector must be the shipped mixer: {shipped} against {selected}"
        );
    }

    /// A second mixer beside the shipped one is read: chosen by the position
    /// within the 55-byte cycle, it codes differently from the shipped mixer,
    /// to a finite length under the input's raw eight bits a byte. A second
    /// mixer the final one never took from would code as the shipped mixer does
    /// and read as no effect on every corpus.
    #[test]
    fn a_second_mixer_beside_the_shipped_one_is_read() {
        let input: Vec<u8> = b"alpha beta gamma delta alpha beta gamma delta epsilon "
            .iter()
            .copied()
            .cycle()
            .take(4000)
            .collect();
        let shipped = logistic_mix_bits(&input, false, &[], None);
        let phase: Vec<u16> = (0..input.len()).map(|t| (t % 55) as u16).collect();
        let layered =
            logistic_mix_bits_with_second_selector(&input, false, &[], None::<&[PreloadTable]>, &phase, 55);
        assert!(layered.is_finite() && layered > 0.0, "a finite code length, got {layered}");
        assert!(layered < 8.0 * input.len() as f64, "under the raw eight bits a byte, got {layered}");
        assert!(
            (layered - shipped).abs() > 1e-6,
            "the second mixer must be read: {layered} against {shipped}"
        );
    }

    /// Mixers beside the shipped one, any number of them: none is the shipped
    /// coder to the bit, one is the second-selector coder to the bit, and a
    /// third is read, coding differently from two.
    #[test]
    fn mixers_beside_the_shipped_one_are_each_read() {
        let input: Vec<u8> = b"alpha beta gamma delta alpha beta gamma delta epsilon "
            .iter()
            .copied()
            .cycle()
            .take(4000)
            .collect();
        let none = None::<&[PreloadTable]>;
        let shipped = logistic_mix_bits_with(&input, false, &[], none);
        let alone = logistic_mix_bits_with_mixers_beside(&input, false, &[], none, &[]);
        assert_eq!(alone.to_bits(), shipped.to_bits(), "no mixer beside is the shipped coder");
        let phase: Vec<u16> = (0..input.len()).map(|t| (t % 55) as u16).collect();
        let nothing = vec![0u16; input.len()];
        let second = logistic_mix_bits_with_second_selector(&input, false, &[], none, &phase, 55);
        let two = logistic_mix_bits_with_mixers_beside(&input, false, &[], none, &[(phase.as_slice(), 55)]);
        assert_eq!(two.to_bits(), second.to_bits(), "one mixer beside is the second-selector coder");
        let three = logistic_mix_bits_with_mixers_beside(
            &input,
            false,
            &[],
            none,
            &[(nothing.as_slice(), 1), (phase.as_slice(), 55)],
        );
        assert!(three.is_finite() && three < 8.0 * input.len() as f64, "a finite length under raw, got {three}");
        assert!((three - two).abs() > 1e-6, "the third mixer must be read: {three} against {two}");
    }

    /// The rhythm selector gives every byte a value below its sets, reads a
    /// periodic stream's rhythm into more than one weight set, and charges what
    /// a decoder must be sent: the power's fifteen bin edges and the stream's
    /// kind set.
    #[test]
    fn the_rhythm_selector_reads_a_rhythm_and_charges_what_a_decoder_needs() {
        let input: Vec<u8> = b"alpha 12, beta 34;\ngamma: 56 delta: 78 epsilon: 90;\nx = 1\n"
            .iter()
            .copied()
            .cycle()
            .take(8000)
            .collect();
        let selector = rhythm_selector(&input);
        assert_eq!(selector.values.len(), input.len(), "one value a byte");
        assert!(selector.values.iter().all(|&v| usize::from(v) < selector.sets), "every value below the sets");
        let chosen: std::collections::BTreeSet<u16> = selector.values.iter().copied().collect();
        assert!(chosen.len() > 1, "the rhythm chooses among sets, got {} values", chosen.len());
        assert!(selector.side_bits > 32 * 15, "the edges and the kinds are charged, got {}", selector.side_bits);
        let shipped = logistic_mix_bits_with(&input, true, &[], None::<&[PreloadTable]>);
        let second = second_mixer_bits_with(&input, None::<&[PreloadTable]>);
        assert!(second.is_finite() && second < 8.0 * input.len() as f64, "a finite length under raw, got {second}");
        assert!((second - shipped).abs() > 1e-6, "the second mixer must be read: {second} against {shipped}");
    }

    /// The rhythm selector reads a byte from nothing a decoder holding the bytes
    /// before it lacks: every prefix read with the whole input's side
    /// information gives the whole input's value at every byte it holds. The
    /// input opens with the constructs that carry a token over a newline - a
    /// string carried by a backslash before an LF and one before a CRLF, a char
    /// literal holding the newline, a quote before a line's end, a backslash
    /// ending a line outside a string - cut after every one of their bytes, and
    /// goes on into real source, cut at intervals and right after each of its
    /// quotes and backslashes.
    #[test]
    fn the_rhythm_selector_reads_only_what_a_decoder_holds() {
        let mut input: Vec<u8> = Vec::new();
        for i in 0..24 {
            input.extend_from_slice(
                format!("let s{i} = \"first\\\nsecond\" ;\nlet c = '\n' ;\nsay 'it' done '\nmacro \\\nnext {i} ;\n")
                    .as_bytes(),
            );
            input.extend_from_slice(format!("let t{i} = \"first\\\r\nsecond\" ;\r\n").as_bytes());
        }
        let held = input.len();
        let source: &[u8] = include_bytes!("../lexer.rs");
        input.extend_from_slice(&source[..source.len().min(60_000)]);
        let toks = rhythm_tokens(&input);
        let mut kinds: Vec<u32> = toks.iter().map(|t| t.kind.code()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        let (powers, ages) = rhythm_readings(&toks, &kinds, RHYTHM_R, RhythmAge::Loudest);
        let readings = RhythmReadings::read(&input);
        let shapes = [
            RhythmShape::SHIPPED,
            RhythmShape { byte_bits: 0, power_bins: 16, age_bins: 16 },
            RhythmShape { byte_bits: 8, power_bins: 2, age_bins: 1 },
        ];
        let mut wholes = Vec::new();
        for shape in shapes {
            let edges = power_edges(&powers, shape.power_bins);
            let parts = rhythm_values(&input, &toks, &power_bins(&powers, &edges), &age_bins(&ages, shape.age_bins), shape);
            let whole = readings.selector(&input, shape);
            assert_eq!(parts, whole.values, "the {shape:?} selector is its parts");
            assert!(whole.values.iter().all(|&v| usize::from(v) < whole.sets), "every {shape:?} value below its sets");
            wholes.push((shape, edges, whole.values));
        }
        assert_eq!(wholes[0].2, rhythm_selector(&input).values, "the shipped selector is the shipped shape");
        let read_at_lines = wholes[0].2.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(read_at_lines > 100, "the rhythm is read at the lines, not held at its start: {read_at_lines}");
        let mut cuts: Vec<usize> = (1..=held).collect();
        cuts.extend((held..input.len()).step_by(997));
        cuts.extend((held..input.len()).filter(|&i| matches!(input[i], b'"' | b'\'' | b'\\')).map(|i| i + 1));
        assert!(cuts.len() > held + 300, "cuts at every construct that could part the two: {}", cuts.len());
        for cut in cuts {
            let prefix = &input[..cut];
            let p_toks = rhythm_tokens(prefix);
            let (p_powers, p_ages) = rhythm_readings(&p_toks, &kinds, RHYTHM_R, RhythmAge::Loudest);
            for (shape, edges, whole) in &wholes {
                let values = rhythm_values(
                    prefix,
                    &p_toks,
                    &power_bins(&p_powers, edges),
                    &age_bins(&p_ages, shape.age_bins),
                    *shape,
                );
                assert_eq!(values[..], whole[..cut], "a {shape:?} value moved with the bytes after it, the prefix cut at {cut}");
            }
        }
    }

    /// A shape reading no rhythm charges nothing and chooses by the byte before
    /// alone; one reading the rhythm charges the kind set and every power edge.
    #[test]
    fn a_rhythm_shape_charges_only_what_it_reads() {
        let input: Vec<u8> = b"alpha 12, beta 34;\ngamma: 56 delta: 78 epsilon: 90;\nx = 1\n"
            .iter()
            .copied()
            .cycle()
            .take(8000)
            .collect();
        let readings = RhythmReadings::read(&input);
        let plain = readings.selector(&input, RhythmShape { byte_bits: 3, power_bins: 1, age_bins: 1 });
        assert_eq!(plain.side_bits, 0, "no rhythm read, nothing sent");
        assert_eq!(plain.sets, 8);
        let expected: Vec<u16> =
            (0..input.len()).map(|t| if t == 0 { 0 } else { u16::from(input[t - 1] >> 5) }).collect();
        assert_eq!(plain.values, expected, "the byte before's top three bits and nothing else");
        let ages_only = readings.selector(&input, RhythmShape { byte_bits: 0, power_bins: 1, age_bins: 4 });
        let powers = readings.selector(&input, RhythmShape { byte_bits: 0, power_bins: 16, age_bins: 1 });
        assert_eq!(powers.side_bits - ages_only.side_bits, 32 * 15, "fifteen power edges sent");
        assert!(ages_only.side_bits > 0, "a rhythm read sends the kind set");
    }

    /// With no order taking the side symbol, the side variant is the shipped
    /// coder: same contexts, same counts, same bits to the last one. That is
    /// what lets a gain it reports be read as the side symbol's and nothing
    /// else's.
    #[test]
    fn a_side_symbol_no_order_takes_codes_exactly_as_the_plain_coder() {
        let input = b"the cat sat on the mat and the cat ran to the cat and the mat sat on the cat";
        let side: Vec<u8> = (0..input.len()).map(|i| (i % 3) as u8).collect();
        for order in [0usize, 2, 5] {
            let plain = ppm_byte_bits(input, order, false);
            let with = ppm_byte_bits_with_side(input, order, &side, 0);
            assert!(
                (plain - with).abs() < 1e-9,
                "order {order}: plain {plain} against a side no order takes {with}"
            );
        }
        // And a side every order takes does change the coding, so the control
        // above is not passing for want of the symbol being read at all.
        let split = ppm_byte_bits_with_side(input, 2, &side, 3);
        assert!((split - ppm_byte_bits(input, 2, false)).abs() > 1e-6);
    }

    #[test]
    fn logistic_mix_codes_under_order0() {
        let input = b"the cat sat on the mat and the cat ran to the cat and the mat sat on the cat";
        let lm = logistic_mix_bits(input, true, &[], None);
        let o0 = adaptive_byte_bits(input, 0, false);
        assert!(lm < input.len() as f64 * 8.0, "logistic mix under raw: {lm:.1}");
        assert!(lm < o0, "logistic mix {lm:.1} should beat order-0 {o0:.1}");
    }
}
