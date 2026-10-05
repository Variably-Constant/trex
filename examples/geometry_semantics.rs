//! Geometry -> semantics, the bottom rung made empirical.
//!
//! The claim: a configuration is parts + relations, and the semantics lives in
//! the relations, not the parts. The test mirrors the object-configuration
//! setup used on images - hold the parts constant, vary the arrangement, and
//! ask which readings can recover the arrangement.
//!
//! Three families of same-parts/different-arrangement text pairs:
//!   - nesting     `a(b(z))` vs `b(a(z))`   (which word encloses which)
//!   - operator    `a = b`   vs `b = a`     (binding direction)
//!   - adjacency   `a b`     vs `b a`       (linear order)
//!
//! Every pair holds an identical token multiset; only the arrangement flips,
//! and the binary label IS the arrangement (is the lower-id vocab word on the
//! outer / left side?). One logistic regression is trained under five feature
//! conditions, identical hyper-parameters and split, only the features differ:
//!
//!   1. parts            - bag-of-vocab + token-kind histogram (intrinsic)
//!   2. stress-depth     - the shipped structural axis as a one-point histogram
//!   3. undirected edges - the relation graph with direction erased
//!   4. directed edges   - the two-point relation field (the claim)
//!   5. parts + directed - both together
//!
//! Conditions 1-3 are at the 50% floor by construction (proven: their feature
//! vectors are byte-identical within every matched pair). Condition 4 is the
//! two-point field. The gap between 3 and 4 is the direction alone.
//!
//! Run: `cargo run --release --example geometry_semantics`

use trex::lexer::lex;
use trex::relation::{self, RelationKind};
use trex::token::TokenKind;

const VOCAB: [&str; 6] = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"];

fn vocab_id(word: &[u8]) -> Option<usize> {
    VOCAB.iter().position(|v| v.as_bytes() == word)
}

/// One generated example: the text, its arrangement label, and which family it
/// came from (for the per-family breakdown).
struct Example {
    text: String,
    label: f64,
    family: &'static str,
}

/// Emit the two arrangements of one matched pair, consecutively, with opposite
/// labels. `lo` < `hi` are vocab ids; `label` is 1.0 when the lower id is on
/// the outer/left side.
fn pair(out: &mut Vec<Example>, family: &'static str, a_first: String, b_first: String) {
    out.push(Example { text: a_first, label: 1.0, family });
    out.push(Example { text: b_first, label: 0.0, family });
}

fn generate() -> Vec<Example> {
    let mut ex = Vec::new();
    for (lo, &a) in VOCAB.iter().enumerate() {
        for &b in &VOCAB[lo + 1..] {
            // Nesting: cycle an inner filler that is neither member of the pair,
            // so parts still tie within each arrangement pair.
            for filler in VOCAB.iter().copied().filter(|&w| w != a && w != b) {
                pair(
                    &mut ex,
                    "nesting",
                    format!("{a}({b}({filler}))"),
                    format!("{b}({a}({filler}))"),
                );
            }

            // Operator: two binding glyphs, two trailing contexts.
            for op in ["=", ":"] {
                for tail in ["", " end"] {
                    pair(
                        &mut ex,
                        "operator",
                        format!("{a} {op} {b}{tail}"),
                        format!("{b} {op} {a}{tail}"),
                    );
                }
            }

            // Adjacency: frames that keep the two words next to each other.
            for (pre, post) in [("", ""), ("( ", " )"), ("start ", ""), ("", " .")] {
                pair(
                    &mut ex,
                    "adjacency",
                    format!("{pre}{a} {b}{post}"),
                    format!("{pre}{b} {a}{post}"),
                );
            }
        }
    }
    ex
}

// ---- feature extractors -------------------------------------------------

const KINDS: usize = 5; // Word, Open, Close, Punct, Other

fn kind_bin(k: TokenKind) -> usize {
    match k {
        TokenKind::Word => 0,
        TokenKind::Open(_) => 1,
        TokenKind::Close(_) => 2,
        TokenKind::Punct => 3,
        _ => 4,
    }
}

/// Intrinsic parts: bag over the vocab plus a token-kind histogram. A pure
/// function of the token multiset, so it is identical for any arrangement.
fn parts(bytes: &[u8]) -> Vec<f64> {
    let toks = lex(bytes);
    let mut v = vec![0.0; VOCAB.len() + KINDS];
    for t in &toks {
        if !t.is_significant() {
            continue;
        }
        if let Some(id) = vocab_id(&bytes[t.span()]) {
            v[id] += 1.0;
        }
        v[VOCAB.len() + kind_bin(t.kind)] += 1.0;
    }
    v
}

/// The shipped stress axis, read one-point: a histogram of nesting depth over
/// the significant tokens. Depth is structural, but as a bag it still ties.
fn stress_depth(bytes: &[u8]) -> Vec<f64> {
    let f = trex::stress::analyze_bytes(bytes);
    let mut v = vec![0.0; 6];
    for fr in &f.frames {
        let b = (fr.depth as usize).min(5);
        v[b] += 1.0;
    }
    v
}

const REL_KINDS: usize = 3;
const VV: usize = VOCAB.len() * VOCAB.len();

fn rel_bin(kind: RelationKind) -> usize {
    match kind {
        RelationKind::Encloses => 0,
        RelationKind::Operator => 1,
        RelationKind::Adjacent => 2,
    }
}

/// The two-point relation field, indexed by (kind, from-vocab, to-vocab).
/// `directed` keeps the arrangement; erasing direction (min/max) is the control.
fn edges(bytes: &[u8], directed: bool) -> Vec<f64> {
    let toks = lex(bytes);
    let field = relation::analyze(&toks, bytes);
    let mut v = vec![0.0; REL_KINDS * VV];
    for e in &field.edges {
        let (Some(fa), Some(fb)) = (
            vocab_id(&bytes[toks[e.from].span()]),
            vocab_id(&bytes[toks[e.to].span()]),
        ) else {
            continue;
        };
        let (i, j) = if directed { (fa, fb) } else { (fa.min(fb), fa.max(fb)) };
        v[rel_bin(e.kind) * VV + i * VOCAB.len() + j] += 1.0;
    }
    // Holonomy: the ordered head->head pairs of each token's enclosure stack
    // (outer before inner). Flat enclosure edges lose the nesting ORDER; the
    // stack keeps it. Folded into the Encloses block, so conditions 3 and 4
    // stay identical in dimension and differ only in whether direction is kept.
    for fr in &field.frames {
        let heads: Vec<usize> = fr
            .enclosure
            .iter()
            .filter_map(|&h| vocab_id(&bytes[toks[h].span()]))
            .collect();
        for a in 0..heads.len() {
            for b in (a + 1)..heads.len() {
                let (i, j) = if directed {
                    (heads[a], heads[b])
                } else {
                    (heads[a].min(heads[b]), heads[a].max(heads[b]))
                };
                v[rel_bin(RelationKind::Encloses) * VV + i * VOCAB.len() + j] += 1.0;
            }
        }
    }
    v
}

fn concat(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter().chain(b).copied().collect()
}

// ---- logistic regression (deterministic) --------------------------------

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

/// Column-standardize with stats computed on the training rows only.
fn standardize(rows: &mut [Vec<f64>], n_train: usize) {
    if rows.is_empty() {
        return;
    }
    let d = rows[0].len();
    for j in 0..d {
        let mean = (0..n_train).map(|i| rows[i][j]).sum::<f64>() / n_train.max(1) as f64;
        let var = (0..n_train).map(|i| (rows[i][j] - mean).powi(2)).sum::<f64>()
            / n_train.max(1) as f64;
        let std = var.sqrt().max(1e-6);
        for row in rows.iter_mut() {
            row[j] = (row[j] - mean) / std;
        }
    }
}

fn train(x: &[Vec<f64>], y: &[f64], iters: usize, lr: f64, l2: f64) -> (Vec<f64>, f64) {
    let d = x.first().map_or(0, Vec::len);
    let n = x.len().max(1) as f64;
    let mut w = vec![0.0; d];
    let mut b = 0.0;
    for _ in 0..iters {
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
            w[j] -= lr * (gw[j] / n + l2 * w[j]);
        }
        b -= lr * gb / n;
    }
    (w, b)
}

fn accuracy(x: &[Vec<f64>], y: &[f64], w: &[f64], b: f64) -> f64 {
    let correct = x
        .iter()
        .zip(y)
        .filter(|(xi, yi)| {
            let z = xi.iter().zip(w).map(|(a, b)| a * b).sum::<f64>() + b;
            (sigmoid(z) >= 0.5) == (**yi >= 0.5)
        })
        .count();
    correct as f64 / x.len().max(1) as f64
}

/// A fixed-seed LCG permutation, so the split is reproducible run to run.
fn shuffle(n: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..n).collect();
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    for i in (1..n).rev() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = (state >> 33) as usize % (i + 1);
        idx.swap(i, j);
    }
    idx
}

/// Train under one feature set and report train / test accuracy.
fn evaluate(name: &str, feats: &[Vec<f64>], labels: &[f64], order: &[usize], n_train: usize) {
    let mut x: Vec<Vec<f64>> = order.iter().map(|&i| feats[i].clone()).collect();
    let y: Vec<f64> = order.iter().map(|&i| labels[i]).collect();
    standardize(&mut x, n_train);
    let (xtr, xte) = x.split_at(n_train);
    let (ytr, yte) = y.split_at(n_train);
    let (w, b) = train(xtr, ytr, 500, 0.5, 1e-4);
    let tr = accuracy(xtr, ytr, &w, b);
    let te = accuracy(xte, yte, &w, b);
    println!("  {name:<22} dim {:>3}   train {:>5.1}%   test {:>5.1}%", feats[0].len(), tr * 100.0, te * 100.0);
}

fn main() {
    let ex = generate();
    let n = ex.len();
    let labels: Vec<f64> = ex.iter().map(|e| e.label).collect();

    // Precompute every feature set for every example.
    let f_parts: Vec<Vec<f64>> = ex.iter().map(|e| parts(e.text.as_bytes())).collect();
    let f_depth: Vec<Vec<f64>> = ex.iter().map(|e| stress_depth(e.text.as_bytes())).collect();
    let f_undir: Vec<Vec<f64>> = ex.iter().map(|e| edges(e.text.as_bytes(), false)).collect();
    let f_dir: Vec<Vec<f64>> = ex.iter().map(|e| edges(e.text.as_bytes(), true)).collect();
    let f_both: Vec<Vec<f64>> =
        (0..n).map(|i| concat(&f_parts[i], &f_dir[i])).collect();

    println!("geometry -> semantics: same parts, different arrangement");
    println!("{n} examples across nesting / operator / adjacency, balanced 50/50\n");

    // Proof: within every matched pair (consecutive rows), the three
    // arrangement-blind feature sets are byte-identical, so any classifier on
    // them is at the 50% floor by construction.
    let mut ident = [0usize; 3];
    for k in 0..n / 2 {
        let (a, b) = (2 * k, 2 * k + 1);
        ident[0] += usize::from(f_parts[a] == f_parts[b]);
        ident[1] += usize::from(f_depth[a] == f_depth[b]);
        ident[2] += usize::from(f_undir[a] == f_undir[b]);
    }
    let pairs = n / 2;
    println!("matched-pair feature identity (arrangement-blind sets must tie):");
    println!("  parts            identical in {}/{pairs} pairs", ident[0]);
    println!("  stress-depth     identical in {}/{pairs} pairs", ident[1]);
    println!("  undirected edges identical in {}/{pairs} pairs", ident[2]);
    let directed_differ = (0..n / 2).filter(|&k| f_dir[2 * k] != f_dir[2 * k + 1]).count();
    println!("  directed edges   DIFFER    in {directed_differ}/{pairs} pairs\n");

    // Nesting load is the enclosure tree's occupancy; holonomy is the cycle
    // residual from scope-crossing reuse. A tree, however deep, has zero
    // holonomy - only reuse that jumps a scope carries it.
    let flat = relation::analyze_bytes(b"alpha beta gamma");
    let tree = relation::analyze_bytes(b"alpha(beta(gamma))");
    let twisted = relation::analyze_bytes(b"alpha(beta(alpha))");
    println!(
        "nesting load / holonomy:  flat = {}/{},  tree 'a(b(c))' = {}/{},  reuse 'a(b(a))' = {}/{}\n",
        flat.nesting_load,
        flat.holonomy(),
        tree.nesting_load,
        tree.holonomy(),
        twisted.nesting_load,
        twisted.holonomy(),
    );

    let order = shuffle(n);
    let n_train = n * 7 / 10;
    println!("logistic regression, identical hyper-parameters, {n_train} train / {} test:", n - n_train);
    evaluate("1. parts", &f_parts, &labels, &order, n_train);
    evaluate("2. stress-depth", &f_depth, &labels, &order, n_train);
    evaluate("3. undirected edges", &f_undir, &labels, &order, n_train);
    evaluate("4. directed edges", &f_dir, &labels, &order, n_train);
    evaluate("5. parts + directed", &f_both, &labels, &order, n_train);

    // Per-family test accuracy on the directed field, so no family carries the
    // result alone.
    println!("\ndirected-edge test accuracy by family:");
    for fam in ["nesting", "operator", "adjacency"] {
        let idx: Vec<usize> = order.iter().copied().filter(|&i| ex[i].family == fam).collect();
        let n_ftr = idx.len() * 7 / 10;
        let mut x: Vec<Vec<f64>> = idx.iter().map(|&i| f_dir[i].clone()).collect();
        let y: Vec<f64> = idx.iter().map(|&i| labels[i]).collect();
        standardize(&mut x, n_ftr);
        let (xtr, xte) = x.split_at(n_ftr);
        let (ytr, yte) = y.split_at(n_ftr);
        let (w, b) = train(xtr, ytr, 500, 0.5, 1e-4);
        println!("  {fam:<12} test {:>5.1}%  ({} examples)", accuracy(xte, yte, &w, b) * 100.0, idx.len());
    }
}
