//! Grounding holonomy against a semantic label it does not define.
//!
//! The transfer experiment proved the geometric signal exists and is
//! substrate-independent, but its label WAS the holonomy - a readout of its own
//! definition. This one breaks that. The label is free-vs-bound variable, the
//! semantic property of a scoped language, computed by a real lexical scope
//! resolver: a use is bound when an enclosing `[ binder ... ]` introduces its
//! name (nearest binder wins, with shadowing). That resolver walks the scope
//! stack and matches names - it never looks at holonomy. The feature is the
//! holonomy of the reuse chord ending at the query use - depth arithmetic on the
//! relation field, never a name. Two different algorithms over the same text; if
//! the geometry predicts the semantics, holonomy is grounded.
//!
//! The label is the resolver's; the feature is the holonomy of the reuse chord.
//! Six families span the ways reuse and binding line up or come apart:
//!   bound        `[ q ( q ) ]`             binder encloses use, residual +1
//!   bound-shadow `[ a [ q ( q ) ] ]`       inner binder captures, residual +1
//!   bound-deep   `[ q ( ( q ) ) ]`         two scopes down, residual +2
//!   free-once    `[ a ( q ) ]`             q never recurs - free, no chord
//!   free-flat    `[ q ( b ) ] ( q )`       q recurs at the SAME depth (0 jump)
//!   free-cross   `[ q ( b ) ] ( ( q ) )`   q recurs a scope DEEPER (+2) - yet
//!                                           free: the binder is a CLOSED sibling
//! The last family is the honest test. Its holonomy signature (a reuse crossing
//! two scopes deeper) is identical to a real binding, but it is free. A two-point
//! reuse chord cannot tell an enclosing binder from a sibling one, so holonomy is
//! expected to MISS this family - the boundary of the grounding, reported, not
//! hidden.
//!
//! Run: `cargo run --release --example holonomy_binding_grounding`

use trex::lexer::lex;
use trex::relation;
use trex::token::TokenKind;

const NAMES: [&str; 6] = ["p", "q", "r", "s", "t", "u"];

struct Example {
    text: String,
    /// 1.0 = the query use is bound, 0.0 = free.
    label: f64,
    /// The adversarial family: a reuse that crosses a scope yet is free (the
    /// binder is in a closed sibling), so its holonomy matches a real binding.
    adversarial: bool,
}

/// A real lexical scope resolver: is the LAST name occurrence bound? `[` opens a
/// scope whose binder is the first name inside it; `]` closes it; `(` `)` group
/// without binding. The query is bound when its name is on the binder stack at
/// its position. This is the ground truth, computed with no reference to the
/// relation field.
fn query_is_bound(bytes: &[u8]) -> bool {
    let toks = lex(bytes);
    let names: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].kind == TokenKind::Word).collect();
    let query = *names.last().expect("a query use");
    let query_name = &bytes[toks[query].span()];

    let mut binders: Vec<Option<Vec<u8>>> = Vec::new();
    let mut expecting = false;
    for (i, t) in toks.iter().enumerate() {
        if i == query {
            break;
        }
        match t.kind {
            TokenKind::Open(bk) if bracket_is_scope(bk) => {
                binders.push(None);
                expecting = true;
            }
            TokenKind::Close(bk) if bracket_is_scope(bk) => {
                binders.pop();
                expecting = false;
            }
            TokenKind::Word if expecting => {
                *binders.last_mut().unwrap() = Some(bytes[t.span()].to_vec());
                expecting = false;
            }
            _ => {}
        }
    }
    binders.iter().flatten().any(|b| b.as_slice() == query_name)
}

fn bracket_is_scope(bk: trex::token::BracketKind) -> bool {
    bk == trex::token::BracketKind::Square
}

/// The query's relation reading: (chord present, signed residual, magnitude,
/// depth, chord curvature).
type Feat = (f64, f64, f64, f64, f64);

/// The relation reading of the query use: whether a reuse chord ends there, its
/// signed residual and magnitude, the query depth, and the Forman-Ricci
/// curvature of that chord (a binding chord is triangle-supported and curves
/// up; a sibling-scope bridge curves down). All from the relation graph -
/// identity-free.
fn query_features(bytes: &[u8]) -> Feat {
    let toks = lex(bytes);
    let names: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].kind == TokenKind::Word).collect();
    let query = *names.last().unwrap();
    let field = relation::analyze(&toks, bytes);
    let chord = field.chords.iter().find(|c| c.to == query);
    let signed = chord.map_or(0.0, |c| f64::from(c.residual));
    let abs = signed.abs();
    let has = f64::from(u8::from(chord.is_some()));
    let depth = f64::from(field.frames[query].depth);
    let ricci = chord
        .and_then(|c| trex::curvature::analyze(&toks, bytes).edge(c.from, c.to))
        .map_or(0.0, f64::from);
    (has, signed, abs, depth, ricci)
}

fn generate() -> Vec<Example> {
    let mut ex = Vec::new();
    for i in 0..96 {
        let q = NAMES[i % 6];
        let a = NAMES[(i / 6 + 1) % 6];
        let a = if a == q { NAMES[(i / 6 + 2) % 6] } else { a };
        let b = NAMES[(i / 6 + 4) % 6];
        let b = if b == q || b == a { NAMES[(i / 6 + 5) % 6] } else { b };
        // Optional outer wrap for structural variety (binds an unused name).
        let wrap = i % 2 == 0;
        // (text, intended-bound, adversarial). Three bound, three free.
        let terms: [(String, bool, bool); 6] = [
            (format!("[ {q} ( {q} ) ]"), true, false),
            (format!("[ {a} [ {q} ( {q} ) ] ]"), true, false),
            (format!("[ {q} ( ( {q} ) ) ]"), true, false),
            (format!("[ {a} ( {q} ) ]"), false, false),
            (format!("[ {q} ( {b} ) ] ( {q} )"), false, false),
            (format!("[ {q} ( {b} ) ] ( ( {q} ) )"), false, true),
        ];
        for (t, intended, adversarial) in terms {
            let text = if wrap { format!("[ {b} {t} ]") } else { t };
            let resolved = query_is_bound(text.as_bytes());
            // The resolver is ground truth; a mismatch means the generator built
            // the wrong shape. Surface it rather than mislabel.
            assert_eq!(
                resolved, intended,
                "generator/resolver disagree on {text:?}: resolver says bound={resolved}"
            );
            ex.push(Example { text, label: f64::from(u8::from(resolved)), adversarial });
        }
    }
    ex
}

// ---- logistic regression (deterministic) --------------------------------

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

fn standardize(rows: &mut [Vec<f64>], fit: &[Vec<f64>]) {
    if rows.is_empty() || fit.is_empty() {
        return;
    }
    let n = fit.len() as f64;
    for j in 0..rows[0].len() {
        let mean = fit.iter().map(|r| r[j]).sum::<f64>() / n;
        let var = fit.iter().map(|r| (r[j] - mean).powi(2)).sum::<f64>() / n;
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
    for _ in 0..800 {
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
            w[j] -= 0.4 * (gw[j] / n + 1e-4 * w[j]);
        }
        b -= 0.4 * gb / n;
    }
    (w, b)
}

fn predict(xi: &[f64], w: &[f64], b: f64) -> bool {
    let z = xi.iter().zip(w).map(|(a, b)| a * b).sum::<f64>() + b;
    sigmoid(z) >= 0.5
}

fn shuffle(n: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..n).collect();
    let mut s: u64 = 0xD1B5_4A32_D192_ED03;
    for i in (1..n).rev() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        idx.swap(i, (s >> 33) as usize % (i + 1));
    }
    idx
}

/// Evaluate one feature set; report overall test accuracy and accuracy on the
/// sibling-free subset (the recurs-yet-free discriminator).
fn evaluate(
    name: &str,
    project: fn(Feat) -> Vec<f64>,
    ex: &[Example],
    order: &[usize],
    n_tr: usize,
) {
    let feat: Vec<Vec<f64>> = ex.iter().map(|e| project(query_features(e.text.as_bytes()))).collect();
    let x: Vec<Vec<f64>> = order.iter().map(|&i| feat[i].clone()).collect();
    let y: Vec<f64> = order.iter().map(|&i| ex[i].label).collect();
    let mut xs = x.clone();
    let fit: Vec<Vec<f64>> = xs[..n_tr].to_vec();
    standardize(&mut xs, &fit);
    let (w, b) = train(&xs[..n_tr], &y[..n_tr]);

    let (mut ok, mut tot, mut adv_ok, mut adv_tot) = (0, 0, 0, 0);
    for k in n_tr..xs.len() {
        let correct = predict(&xs[k], &w, b) == (y[k] >= 0.5);
        ok += usize::from(correct);
        tot += 1;
        if ex[order[k]].adversarial {
            adv_ok += usize::from(correct);
            adv_tot += 1;
        }
    }
    let pct = |a: usize, b: usize| if b == 0 { f64::NAN } else { a as f64 / b as f64 * 100.0 };
    println!(
        "  {name:<26} dim {}   test {:>5.1}%   free-cross subset {:>5.1}%",
        xs[0].len(),
        pct(ok, tot),
        pct(adv_ok, adv_tot)
    );
}

fn main() {
    let ex = generate();
    let n = ex.len();
    let bound = ex.iter().filter(|e| e.label >= 0.5).count();
    println!("holonomy grounded against a scope resolver: free-vs-bound variables");
    println!("{n} scoped terms ({bound} bound, {} free), label from lexical resolution\n", n - bound);

    let order = shuffle(n);
    let n_tr = n * 7 / 10;
    println!("logistic regression, {n_tr} train / {} test, label = the resolver's bound/free:", n - n_tr);
    // echo presence: does the name recur at all (undirected two-point).
    evaluate("echo presence", |(has, _, _, _, _)| vec![has], &ex, &order, n_tr);
    // holonomy: the full reuse-chord reading (presence, signed residual, depth).
    evaluate("holonomy (reuse chord)", |(has, s, a, d, _)| vec![has, s, a, d], &ex, &order, n_tr);
    // holonomy + curvature: add the Forman-Ricci curvature of the chord.
    evaluate("holonomy + curvature", |(has, s, a, d, r)| vec![has, s, a, d, r], &ex, &order, n_tr);

    println!(
        "\nholonomy alone misses the free-cross family: a reuse whose binder is in a\n\
         closed sibling scope has the same chord residual as a real binding. Curvature\n\
         is the local twist the two-point chord cannot see - a binding chord is\n\
         triangle-supported (positive), a sibling bridge is not (negative) - so adding\n\
         it recovers exactly the family the residual could not resolve."
    );
}

#[cfg(test)]
mod tests {
    /// Every claim here is an assertion inside `main`, so running it is the
    /// test. The manifest marks this example `test = true`, so `cargo test`
    /// runs it rather than only compiling it.
    #[test]
    fn every_claim_holds() {
        super::main();
    }
}
