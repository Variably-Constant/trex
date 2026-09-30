//! The action reading: the two conjugate energies trex already computes,
//! combined.
//!
//! [`crate::magnitude`] accumulates `sum(m^2)` over a span - the extensive
//! intensity of the values passing through it. [`crate::stress`] accumulates
//! the load held by spans left open - the work owed to close what is open.
//! One is energy of motion through scale, the other energy stored by position
//! in the bracket field, and they are the two halves of a Lagrangian that
//! nothing had put together.
//!
//! ```text
//! T = sum(m^2)     kinetic, from the magnitude profile
//! V = sum(depth)   potential, from the stress profile
//! T + V            total energy
//! T - V            the Lagrangian
//! ```
//!
//! ## Why the potential integrates rather than peaks
//!
//! [`crate::profile::StressProfile`] carries both a peak depth and a summed
//! one. `T` is extensive: it grows with the span. Pairing it with a peak would
//! subtract an intensive quantity from an extensive one, so the difference
//! would change meaning with span length rather than measuring anything. `V`
//! is the summed depth for that reason.
//!
//! ## What the zero crossing means
//!
//! [`crate::magnitude::token_magnitude`] reads a non-number as
//! `log2(byte length)`, so a single-character token has exactly zero kinetic
//! energy and its Lagrangian is the negative of its depth. A sign change is
//! therefore the point where a token stops being long enough to outweigh how
//! deep it sits. That is the reading at token level, and it is worth knowing
//! before reaching for it as a boundary detector: it responds to token width
//! against nesting, not to anything about content.
//!
//! ## Boundary detection: measured and rejected
//!
//! Sign changes in the Lagrangian were the obvious candidate for structure
//! boundaries, so they were built and scored against statement starts in
//! generated bracketed code, 2771 bytes with 70 true boundaries, tolerance 2:
//!
//! ```text
//! action turning   precision 0.095   recall 0.529   f1 0.161   391 cuts
//! seam cuts        precision 0.052   recall 1.000   f1 0.099  1341 cuts
//! shape changes    precision 0.250   recall 0.071   f1 0.111    20 cuts
//! ```
//!
//! The best f1 of the three, and not worth having: a precision of 0.095 means
//! nine of every ten predicted boundaries are wrong, and 391 cuts for 70
//! boundaries is over-prediction by five and a half times. It edges seam only
//! because seam buys recall 1.0 by cutting almost everywhere. Neither seam nor
//! shape is aimed at statement boundaries in bracketed code, so this is not a
//! win over them either.
//!
//! The turning-point surface is therefore not here. The energies below are,
//! because they read something the axes they come from do not.
//!
//! ## The total is not conserved
//!
//! Wrapping a construct in one more bracket leaves every token, and so every
//! magnitude, untouched while raising each interior token's depth by one. The
//! kinetic term does not move; the potential rises by the interior token
//! count. `T + V` therefore grows with nesting rather than staying invariant,
//! and the quantity is a reading of how deeply a span sits as much as of what
//! it contains. What breaks conservation is that the potential is extensive in
//! depth as well as in span, so an enclosing bracket adds energy without
//! adding content.
//!
//! ## Every level at once
//!
//! The reading is defined over a [`Profile`], which is a monoid, so it is not
//! computed per level: one leaf definition and the fold gives a token, a
//! supertoken, a block and a document together. Changing the input alphabet is
//! a different matter and not available by folding - a token-kind stream is
//! not a function of the byte-class stream, since the lexer's boundaries are
//! not in it.

use crate::profile::{AxisCtx, AxisProfile, MagnitudeProfile, Profile};
use crate::token::Token;

/// The energies of one span.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Action {
    /// Summed squared magnitude: the extensive scale intensity.
    pub kinetic: f32,
    /// Summed bracket depth: the load held across the span.
    pub potential: f32,
}

impl Action {
    /// Total energy, `T + V`.
    #[must_use]
    pub fn total(self) -> f32 {
        self.kinetic + self.potential
    }

    /// The Lagrangian, `T - V`. Positive where scale dominates, negative
    /// where structure does.
    #[must_use]
    pub fn lagrangian(self) -> f32 {
        self.kinetic - self.potential
    }

    /// The energies of a span, read from its pooled profile. Because the
    /// profile is a monoid, this reads at whatever level the profile was
    /// folded to.
    #[must_use]
    pub fn of_profile(p: &Profile) -> Self {
        Action { kinetic: p.magnitude.sumsq, potential: p.stress.sum_depth as f32 }
    }
}

/// The per-token Lagrangian over a stream.
#[derive(Clone, Debug, Default)]
pub struct ActionField {
    /// `T - V` at each token index.
    pub lagrangian: Vec<f32>,
}

impl ActionField {
    /// The action integrated over the half-open token range.
    #[must_use]
    pub fn action(&self, from: usize, to: usize) -> f32 {
        let hi = to.min(self.lagrangian.len());
        if from >= hi {
            return 0.0;
        }
        self.lagrangian[from..hi].iter().sum()
    }
}

/// Read the action field of an already-lexed stream.
#[must_use]
pub fn analyze(toks: &[Token], bytes: &[u8]) -> ActionField {
    let stress = crate::stress::analyze(toks, bytes);
    let ctx = AxisCtx { stress: Some(&stress), ..AxisCtx::new(bytes) };
    let lagrangian: Vec<f32> = toks
        .iter()
        .enumerate()
        .map(|(i, t)| Action::of_profile(&Profile::of_token(i, t, &ctx)).lagrangian())
        .collect();
    ActionField { lagrangian }
}

/// Read the action field directly from bytes.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> ActionField {
    analyze(&crate::lexer::lex(bytes), bytes)
}

/// The action of each supertoken: the level at which a held-open span and the
/// values inside it belong to one unit.
#[must_use]
pub fn per_supertoken(toks: &[Token], bytes: &[u8]) -> Vec<Action> {
    let stress = crate::stress::analyze(toks, bytes);
    let ctx = AxisCtx { stress: Some(&stress), ..AxisCtx::new(bytes) };
    let units = crate::supertoken::supertokens_from(toks, bytes);

    // Both energies are plain sums over tokens, so a unit's reading is the
    // difference of two prefixes rather than a fold. The two other things that
    // makes possible are worth stating: no `Profile` is built, so the six
    // members `Action` does not read are never computed and the free monoid in
    // `shape` never allocates; and the kinetic prefix carries f64 while the
    // potential carries u64, so the depth term is exact at any width and the
    // scale term keeps twenty-nine bits of headroom over the f32 it returns to.
    let n = toks.len();
    let mut kinetic: Vec<f64> = Vec::with_capacity(n + 1);
    let mut potential: Vec<u64> = Vec::with_capacity(n + 1);
    kinetic.push(0.0);
    potential.push(0);
    for (i, t) in toks.iter().enumerate() {
        let m = MagnitudeProfile::of_token(i, t, &ctx);
        // The field was built from these very tokens, so its frame `i` is the
        // frame the byte-offset lookup resolves to: token starts are strictly
        // increasing, so the search for `t.start()` lands on `i`. Reading it by
        // index is one load where the lookup is a binary search over every span
        // in the stream, and this runs once per token.
        let depth = stress.frames[i].depth;
        kinetic.push(kinetic[i] + f64::from(m.sumsq));
        potential.push(potential[i] + u64::from(depth));
    }

    let mut out = Vec::with_capacity(units.len());
    let mut cursor = 0usize;
    for u in &units {
        // Unit starts are non-decreasing, so one cursor walks the left ends.
        // The right end is found by search rather than by scanning from the
        // left one: a nested unit ends before its parent, so a span walk costs
        // the sum of every unit's span, which is the token count only while
        // the units stay disjoint.
        while cursor < n && toks[cursor].start() < u.start {
            cursor += 1;
        }
        let lo = cursor;
        let hi = lo + toks[lo..].partition_point(|t| t.end() <= u.end);
        out.push(Action {
            kinetic: (kinetic[hi] - kinetic[lo]) as f32,
            potential: (potential[hi] - potential[lo]) as f32,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::fold_tokens;

    #[test]
    fn per_supertoken_reads_what_the_folded_profile_reads() {
        // The prefix difference replaces a fold, so what it must equal is the
        // fold: `Action::of_profile` over each unit's tokens, which is the
        // definition the module documents. Nested and flat units both, since
        // the two take different paths through the right-end search.
        for src in [
            &b"{module, [{a, [1, 2, {b, [3, 4]}]}, {c, [5, {d, [6, 7]}]}]}. {x, [{y, 8}]}."[..],
            &b"alpha beta gamma delta"[..],
            &b"f(x) g(yy) h(zzz)"[..],
        ] {
            let toks = crate::lexer::lex(src);
            let stress = crate::stress::analyze(&toks, src);
            let ctx = AxisCtx { stress: Some(&stress), ..AxisCtx::new(src) };
            let units = crate::supertoken::supertokens_from(&toks, src);
            let got = per_supertoken(&toks, src);
            assert_eq!(got.len(), units.len(), "one reading per unit for {src:?}");

            let mut cursor = 0usize;
            for (k, u) in units.iter().enumerate() {
                while cursor < toks.len() && toks[cursor].start() < u.start {
                    cursor += 1;
                }
                let lo = cursor;
                let mut hi = cursor;
                while hi < toks.len() && toks[hi].end() <= u.end {
                    hi += 1;
                }
                let want = Action::of_profile(&fold_tokens::<Profile>(lo, &toks[lo..hi], &ctx));
                // The potential is a sum of integers on both paths, so it is
                // exact; the kinetic term is a f64 prefix difference against an
                // f32 running sum and agrees to the width f32 can carry.
                assert_eq!(
                    got[k].potential, want.potential,
                    "unit {k} potential, {src:?}"
                );
                assert!(
                    (got[k].kinetic - want.kinetic).abs() <= 1e-5 * want.kinetic.abs().max(1.0),
                    "unit {k} kinetic {} against the fold's {}, {src:?}",
                    got[k].kinetic,
                    want.kinetic
                );
            }
        }
    }

    #[test]
    fn nesting_raises_the_potential_and_flips_the_lagrangian() {
        // A flat run carries no load, so the Lagrangian stays non-negative.
        let flat = analyze_bytes(b"alpha beta gamma");
        assert!(flat.lagrangian.iter().all(|&l| l >= 0.0), "no brackets, no potential");

        // A long token at the surface outweighs its depth; a short one buried
        // three deep does not, so the two energies trade places between them.
        let nested = analyze_bytes(b"alphabet ((( x )))");
        assert!(nested.lagrangian.iter().any(|&l| l > 0.0), "the long token carries scale");
        assert!(nested.lagrangian.iter().any(|&l| l < 0.0), "the buried one is outweighed");
    }

    #[test]
    fn a_one_character_token_carries_no_kinetic_energy() {
        // magnitude::token_magnitude reads a non-number as log2(byte length),
        // so a single-character token sits at exactly zero and its Lagrangian
        // is the negative of its depth. The zero crossing is therefore "is
        // this token long enough to outweigh how deep it sits", which is what
        // the reading means at token level.
        let f = analyze_bytes(b"x");
        assert_eq!(f.lagrangian[0], 0.0, "log2(1) is zero, so no kinetic term");
        let buried = analyze_bytes(b"((x))");
        assert!(
            buried.lagrangian.iter().any(|&l| l < 0.0),
            "and depth alone drives it negative"
        );
    }

    #[test]
    fn a_large_value_raises_the_kinetic_term() {
        let small = analyze_bytes(b"(5)");
        let large = analyze_bytes(b"(5000000000)");
        let ts: f32 = small.lagrangian.iter().sum();
        let tl: f32 = large.lagrangian.iter().sum();
        assert!(tl > ts, "the larger value carries more kinetic energy at one depth");
    }

    #[test]
    fn action_integrates_over_a_range() {
        let f = analyze_bytes(b"a (b) c");
        let whole = f.action(0, f.lagrangian.len());
        let split = f.action(0, 3) + f.action(3, f.lagrangian.len());
        assert!((whole - split).abs() < 1e-3, "the integral is additive over a split");
        assert_eq!(f.action(2, 2), 0.0, "an empty range integrates to zero");
        assert_eq!(f.action(5, 1), 0.0, "an inverted range integrates to zero");
    }

    #[test]
    fn the_reading_lifts_to_supertokens() {
        let bytes = b"let x = 1;\nfoo(2, 3)\nname: bob\n";
        let toks = crate::lexer::lex(bytes);
        let per = per_supertoken(&toks, bytes);
        assert!(!per.is_empty(), "the input has constructs");
        assert!(per.iter().any(|a| a.potential > 0.0), "some construct holds load");
        assert!(per.iter().any(|a| a.kinetic > 0.0), "some construct carries scale");
        let summed: f32 = per.iter().map(|a| a.total()).sum();
        assert!(summed > 0.0);
    }

    #[test]
    fn total_energy_is_not_conserved_under_nesting() {
        // Wrapping a construct in one more bracket leaves every token and so
        // every magnitude untouched, and raises each interior token's depth by
        // one. The kinetic term is therefore unchanged and the potential rises
        // by the interior token count, so the total is not invariant.
        let total = |src: &[u8]| -> (f32, f32) {
            let toks = crate::lexer::lex(src);
            let stress = crate::stress::analyze(&toks, src);
            let ctx = AxisCtx { stress: Some(&stress), ..AxisCtx::new(src) };
            let p: Profile = fold_tokens(0, &toks, &ctx);
            let a = Action::of_profile(&p);
            (a.kinetic, a.potential)
        };

        let (t0, v0) = total(b"alpha beta");
        let (t1, v1) = total(b"( alpha beta )");
        let (t2, v2) = total(b"( ( alpha beta ) )");

        // The kinetic term is a function of the tokens alone, and the brackets
        // added are one-byte tokens of zero magnitude, so it does not move.
        assert!((t0 - t1).abs() < 1e-3, "kinetic unchanged by one wrap: {t0} vs {t1}");
        assert!((t1 - t2).abs() < 1e-3, "and by a second: {t1} vs {t2}");

        // The potential rises with each wrap, so the total rises with it.
        assert!(v1 > v0, "one wrap raises the potential: {v0} -> {v1}");
        assert!(v2 > v1, "two wraps raise it further: {v1} -> {v2}");
        assert!(t1 + v1 > t0 + v0, "so the total is not conserved");
        assert!(t2 + v2 > t1 + v1);
    }

    #[test]
    fn the_potential_is_extensive_so_it_grows_with_the_span() {
        // The dimensional argument for summed rather than peak depth:
        // lengthening a nested span raises the potential, matching how the
        // kinetic term grows.
        let one = analyze_bytes(b"(a b)");
        let two = analyze_bytes(b"(a b a b)");
        let v1: f32 = -one.lagrangian.iter().filter(|&&l| l < 0.0).sum::<f32>();
        let v2: f32 = -two.lagrangian.iter().filter(|&&l| l < 0.0).sum::<f32>();
        assert!(v2 > v1, "a longer held-open span stores more potential");
    }
}
