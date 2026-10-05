//! Identity-free holonomy: semantics from geometry regardless of the tokens.
//!
//! The first experiment (`geometry_semantics`) recovered arrangement for a
//! KNOWN vocabulary - its label was anchored to token identity. This one drops
//! the anchor. The label is a purely structural property - does a reused token
//! CROSS a scope boundary (non-zero cycle holonomy) or stay within one scope
//! (zero holonomy) - and the test vocabulary is entirely disjoint from the
//! training vocabulary. If a holonomy feature transfers to words it has never
//! seen, the structure is being read from the geometry, not the tokens.
//!
//! Each matched pair shares a bracket skeleton and a token multiset; only the
//! arrangement of the reused token flips:
//!   twisted  `a(a)b`   the reused `a` is at depth 0 and depth 1 (crosses)
//!   flat     `a(b)a`   the reused `a` is at depth 0 twice (stays)
//! Same tokens, same per-depth histogram, same kinds - so parts, the depth
//! reading, and even the identity of the reused token are byte-identical within
//! the pair. Only the holonomy differs.
//!
//! Four feature conditions, one logistic regression, trained on the training
//! vocabulary and evaluated twice: on held-out arrangements of the same words,
//! and on a disjoint vocabulary. A feature that reads geometry shows no drop
//! across the vocabulary boundary.
//!
//! Run: `cargo run --release --example geometry_holonomy_transfer`

use trex::lexer::lex;
use trex::relation;
use trex::token::TokenKind;

const TRAIN_VOCAB: [&str; 8] =
    ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"];
const TEST_VOCAB: [&str; 8] =
    ["red", "green", "blue", "cyan", "black", "white", "amber", "coral"];

/// One example: the text, its structural label (1 = scope-crossing reuse), and
/// which vocabulary it was built from.
struct Example {
    text: String,
    label: f64,
    unseen: bool,
}

/// The three bracket skeletons, each as (twisted, flat) given the reused token
/// `a` and the singleton `b`. Both members share the token multiset {a, a, b}
/// and the per-position depth pattern; only which position `a` reuses differs.
fn skeletons(a: &str, b: &str) -> [(String, String); 3] {
    [
        (format!("{a}({a}){b}"), format!("{a}({b}){a}")), // depths 0,1,0
        (format!("{a}(({a})){b}"), format!("{a}(({b})){a}")), // 0,2,0
        (format!("({a}({a}){b})"), format!("({a}({b}){a})")), // 1,2,1
    ]
}

fn generate() -> Vec<Example> {
    let mut ex = Vec::new();
    for (vocab, unseen) in [(TRAIN_VOCAB, false), (TEST_VOCAB, true)] {
        for &a in &vocab {
            for &b in &vocab {
                if a == b {
                    continue;
                }
                for (twisted, flat) in skeletons(a, b) {
                    ex.push(Example { text: twisted, label: 1.0, unseen });
                    ex.push(Example { text: flat, label: 0.0, unseen });
                }
            }
        }
    }
    ex
}

// ---- feature extractors -------------------------------------------------

/// Parts: a token-kind histogram. Identical within a matched pair (same tokens).
fn parts(bytes: &[u8]) -> Vec<f64> {
    let mut v = vec![0.0; 5];
    for t in lex(bytes) {
        let b = match t.kind {
            TokenKind::Word => 0,
            TokenKind::Open(_) => 1,
            TokenKind::Close(_) => 2,
            TokenKind::Punct => 3,
            _ => 4,
        };
        v[b] += 1.0;
    }
    v
}

/// One-point depth: a histogram of per-token enclosure depth. Identical within
/// a matched pair (same skeleton), so it cannot see which reuse crosses a scope.
fn depth_hist(bytes: &[u8]) -> Vec<f64> {
    let f = relation::analyze_bytes(bytes);
    let mut v = vec![0.0; 6];
    for fr in &f.frames {
        v[(fr.depth as usize).min(5)] += 1.0;
    }
    v
}

/// Identity: a bag of the content tokens over the training vocabulary, with one
/// bucket for anything unseen. Identical within a matched pair, and all-unseen
/// on the disjoint vocabulary - so it carries neither the label nor transfer.
fn identity(bytes: &[u8]) -> Vec<f64> {
    let mut v = vec![0.0; TRAIN_VOCAB.len() + 1];
    for t in lex(bytes) {
        if t.kind != TokenKind::Word {
            continue;
        }
        let text = &bytes[t.span()];
        let id = TRAIN_VOCAB.iter().position(|w| w.as_bytes() == text).unwrap_or(TRAIN_VOCAB.len());
        v[id] += 1.0;
    }
    v
}

/// Holonomy: a histogram of reuse-chord residual magnitudes, plus the chord
/// count. Reads only |depth jump| of each reuse - no token identity - so it is
/// the identity-free structural reading.
fn holonomy(bytes: &[u8]) -> Vec<f64> {
    let f = relation::analyze_bytes(bytes);
    let mut v = vec![0.0; 6];
    for c in &f.chords {
        v[(c.residual.unsigned_abs() as usize).min(4)] += 1.0;
    }
    v[5] = f.chords.len() as f64;
    v
}

// ---- logistic regression (deterministic) --------------------------------

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

/// Standardize every column using statistics from the training rows only.
fn standardize(rows: &mut [Vec<f64>], stats_from: &[Vec<f64>]) {
    if rows.is_empty() || stats_from.is_empty() {
        return;
    }
    let n = stats_from.len() as f64;
    for j in 0..rows[0].len() {
        let mean = stats_from.iter().map(|r| r[j]).sum::<f64>() / n;
        let var = stats_from.iter().map(|r| (r[j] - mean).powi(2)).sum::<f64>() / n;
        let std = var.sqrt().max(1e-6);
        for row in rows.iter_mut() {
            row[j] = (row[j] - mean) / std;
        }
    }
}

fn train(x: &[Vec<f64>], y: &[f64]) -> (Vec<f64>, f64) {
    let d = x.first().map_or(0, Vec::len);
    let n = x.len().max(1) as f64;
    let (mut w, mut b) = (vec![0.0; d], 0.0);
    for _ in 0..500 {
        let mut gw = vec![0.0; d];
        let mut gb = 0.0;
        for (xi, &yi) in x.iter().zip(y) {
            let z = xi.iter().zip(&w).map(|(a, b)| a * b).sum::<f64>() + b;
            let err = sigmoid(z) - yi;
            for j in 0..d {
                gw[j] += err * xi[j];
            }
            gb += err;
        }
        for j in 0..d {
            w[j] -= 0.5 * (gw[j] / n + 1e-4 * w[j]);
        }
        b -= 0.5 * gb / n;
    }
    (w, b)
}

fn accuracy(x: &[Vec<f64>], y: &[f64], w: &[f64], b: f64) -> f64 {
    let ok = x
        .iter()
        .zip(y)
        .filter(|(xi, yi)| {
            let z = xi.iter().zip(w).map(|(a, b)| a * b).sum::<f64>() + b;
            (sigmoid(z) >= 0.5) == (**yi >= 0.5)
        })
        .count();
    ok as f64 / x.len().max(1) as f64
}

/// Deterministic LCG permutation, so the seen-vocab split is reproducible.
fn shuffle(n: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..n).collect();
    let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in (1..n).rev() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        idx.swap(i, (s >> 33) as usize % (i + 1));
    }
    idx
}

fn evaluate(name: &str, feat: fn(&[u8]) -> Vec<f64>, ex: &[Example]) {
    // Split the seen vocabulary into train / held-out; the unseen vocabulary is
    // the transfer set. Standardization stats come from the training rows only.
    let seen: Vec<usize> = (0..ex.len()).filter(|&i| !ex[i].unseen).collect();
    let unseen: Vec<usize> = (0..ex.len()).filter(|&i| ex[i].unseen).collect();
    let order = shuffle(seen.len());
    let n_tr = seen.len() * 7 / 10;

    let raw = |i: usize| feat(ex[i].text.as_bytes());
    let mut tr: Vec<Vec<f64>> = order[..n_tr].iter().map(|&k| raw(seen[k])).collect();
    let mut ho: Vec<Vec<f64>> = order[n_tr..].iter().map(|&k| raw(seen[k])).collect();
    let mut un: Vec<Vec<f64>> = unseen.iter().map(|&i| raw(i)).collect();
    let ytr: Vec<f64> = order[..n_tr].iter().map(|&k| ex[seen[k]].label).collect();
    let yho: Vec<f64> = order[n_tr..].iter().map(|&k| ex[seen[k]].label).collect();
    let yun: Vec<f64> = unseen.iter().map(|&i| ex[i].label).collect();

    let fit = tr.clone();
    standardize(&mut tr, &fit);
    standardize(&mut ho, &fit);
    standardize(&mut un, &fit);
    let (w, b) = train(&tr, &ytr);
    println!(
        "  {name:<20} dim {:>2}   seen-vocab test {:>5.1}%   UNSEEN-vocab {:>5.1}%",
        fit[0].len(),
        accuracy(&ho, &yho, &w, b) * 100.0,
        accuracy(&un, &yun, &w, b) * 100.0
    );
}

fn main() {
    let ex = generate();
    let n = ex.len();
    let (seen, unseen) = (ex.iter().filter(|e| !e.unseen).count(), ex.iter().filter(|e| e.unseen).count());
    println!("identity-free holonomy: structural label, disjoint train/test vocabularies");
    println!("{n} examples ({seen} seen-vocab, {unseen} unseen-vocab), balanced 50/50\n");

    // Proof: within every matched pair the three arrangement-blind sets are
    // byte-identical, including the identity of the reused token. Only holonomy
    // separates the pair.
    let mut ident = [0usize; 3];
    let mut holo_differ = 0usize;
    for k in 0..n / 2 {
        let (a, b) = (ex[2 * k].text.as_bytes(), ex[2 * k + 1].text.as_bytes());
        ident[0] += usize::from(parts(a) == parts(b));
        ident[1] += usize::from(depth_hist(a) == depth_hist(b));
        ident[2] += usize::from(identity(a) == identity(b));
        holo_differ += usize::from(holonomy(a) != holonomy(b));
    }
    let pairs = n / 2;
    println!("matched-pair feature identity (arrangement-blind sets must tie):");
    println!("  parts (kinds)     identical in {}/{pairs} pairs", ident[0]);
    println!("  one-point depth   identical in {}/{pairs} pairs", ident[1]);
    println!("  reused-token id   identical in {}/{pairs} pairs", ident[2]);
    println!("  holonomy          DIFFERS   in {holo_differ}/{pairs} pairs\n");

    println!("logistic regression, trained on the SEEN vocabulary only:");
    evaluate("parts (kinds)", parts, &ex);
    evaluate("one-point depth", depth_hist, &ex);
    evaluate("reused-token id", identity, &ex);
    evaluate("holonomy", holonomy, &ex);
    println!(
        "\nthe holonomy feature reads |scope jump|, never a token, so unseen vocabulary\n\
         is no harder than held-out arrangements of seen vocabulary."
    );
}
