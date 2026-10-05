//! The entanglement reading - shared structure across a cut.
//!
//! The Ryu-Takayanagi picture: the entanglement entropy of a region is set by
//! the AREA of its boundary, not its volume. Here the boundary of a cut in the
//! token stream is the set of structural edges that straddle it - an enclosure
//! reaching across, a reuse chord binding one side to the other. The count of
//! those crossing edges is the entanglement across the cut: how much the two
//! sides share.
//!
//! A cut with zero crossings is a clean separation - the two sides are a product
//! state, structurally independent, the place to split with no dependency lost.
//! The minimal cut is exactly the Ryu-Takayanagi minimal surface, and it is a
//! grammar-free segmentation boundary: cut where the structure is thinnest. Only
//! the long-range structural edges count - enclosure, operator, reuse - not the
//! linear adjacency backbone, which crosses every cut by one and carries no
//! information about where the seams are.

use crate::lexer::lex;
use crate::relation::{self, RelationKind};
use crate::token::Token;

/// The entanglement reading of a token stream.
#[derive(Clone, Debug, Default)]
pub struct Entanglement {
    /// One entry per interior cut position `k` (the boundary before token `k`):
    /// the number of structural edges that straddle it.
    pub profile: Vec<usize>,
}

impl Entanglement {
    /// The minimal cut: the interior boundary crossed by the fewest structural
    /// edges, as `(token index of the cut, crossing count)`. The Ryu-Takayanagi
    /// minimal surface, and the cleanest structural split. `None` when there is
    /// no interior boundary.
    #[must_use]
    pub fn min_cut(&self) -> Option<(usize, usize)> {
        let n = self.profile.len();
        self.profile
            .iter()
            .enumerate()
            .filter(|(k, _)| *k > 0 && *k < n)
            .min_by_key(|(_, c)| **c)
            .map(|(k, c)| (k, *c))
    }

    /// The most entangled interior boundary: the peak crossing count.
    #[must_use]
    pub fn max_entanglement(&self) -> usize {
        self.profile.iter().copied().max().unwrap_or(0)
    }
}

/// Read the entanglement of an already-lexed stream.
#[must_use]
pub fn analyze(toks: &[Token], bytes: &[u8]) -> Entanglement {
    let field = relation::analyze(toks, bytes);
    let n = toks.len();

    // Structural edges as ordered spans (lo, hi); the linear adjacency backbone
    // is excluded so the profile reflects long-range sharing only.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for e in &field.edges {
        if e.kind != RelationKind::Adjacent && e.from != e.to {
            spans.push((e.from.min(e.to), e.from.max(e.to)));
        }
    }
    for c in &field.chords {
        if c.from != c.to {
            spans.push((c.from.min(c.to), c.from.max(c.to)));
        }
    }

    Entanglement { profile: crossing_profile(&spans, n) }
}

/// Read the entanglement counting only edges that cross a boundary between
/// `map`'s nodes.
///
/// The cut positions stay every token position. Contracting the graph and
/// cutting between nodes instead would shrink the candidate set, and a minimum
/// over a subset can only rise, so the coarse reading would be the minimum of
/// a different function over a different domain rather than a coarser view of
/// the same one. Keeping the domain and filtering the edges makes `map` a
/// resolution parameter, so the two readings compare cut for cut.
///
/// An edge counts unless both endpoints are in the same node. A token in no
/// node at all (a bracket, a separator) is its own singleton, so an edge
/// touching one still crosses.
#[must_use]
pub fn analyze_over(toks: &[Token], bytes: &[u8], map: &relation::NodeMap) -> Entanglement {
    let field = relation::analyze(toks, bytes);
    let n = toks.len();
    let same_node = |a: usize, b: usize| {
        matches!((map.node_of(a), map.node_of(b)), (Some(x), Some(y)) if x == y)
    };

    let mut spans: Vec<(usize, usize)> = Vec::new();
    for e in &field.edges {
        if e.kind != RelationKind::Adjacent && e.from != e.to && !same_node(e.from, e.to) {
            spans.push((e.from.min(e.to), e.from.max(e.to)));
        }
    }
    for c in &field.chords {
        if c.from != c.to && !same_node(c.from, c.to) {
            spans.push((c.from.min(c.to), c.from.max(c.to)));
        }
    }

    Entanglement { profile: crossing_profile(&spans, n) }
}

/// The per-position crossing count of a set of spans.
///
/// A cut before token `k` is crossed by span `(lo, hi)` when `lo < k <= hi`,
/// swept with a difference array.
fn crossing_profile(spans: &[(usize, usize)], n: usize) -> Vec<usize> {
    let mut delta = vec![0i32; n + 2];
    for &(lo, hi) in spans {
        delta[lo + 1] += 1;
        delta[hi + 1] -= 1;
    }
    let mut profile = vec![0usize; n];
    let mut running = 0i32;
    for (k, slot) in profile.iter_mut().enumerate() {
        running += delta[k];
        *slot = running.max(0) as usize;
    }
    profile
}

/// Read the entanglement directly from bytes.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> Entanglement {
    analyze(&lex(bytes), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_contraction_reproduces_the_token_reading() {
        // Every token is its own node, so no edge is dropped.
        let src = "f(g(x), y)\nh(x)\n";
        let bytes = src.as_bytes();
        let toks = lex(bytes);
        let direct = analyze(&toks, bytes);
        let via_map = analyze_over(&toks, bytes, &relation::NodeMap::per_token(toks.len()));
        assert_eq!(direct.profile, via_map.profile);
    }

    #[test]
    fn a_coarser_node_set_keeps_the_candidate_cuts() {
        // The domain is the same either way, which is what makes the two
        // readings comparable cut for cut. Contracting the graph instead
        // would have left one cut per unit.
        let src = "let total = sum(price, tax);\nlet net = round(total);\nprint(net, total);\n";
        let bytes = src.as_bytes();
        let toks = lex(bytes);
        let units = crate::supertoken::supertokens_from(&toks, bytes);
        let map = relation::NodeMap::per_supertoken(&units, &toks);
        let fine = analyze(&toks, bytes);
        let coarse = analyze_over(&toks, bytes, &map);
        assert_eq!(fine.profile.len(), coarse.profile.len(), "same candidate cuts");
        assert_eq!(coarse.profile.len(), toks.len());
        // Dropping the within-unit edges can only lower a crossing count.
        for (k, (f, c)) in fine.profile.iter().zip(coarse.profile.iter()).enumerate() {
            assert!(c <= f, "cut {k}: coarse {c} exceeded fine {f}");
        }
        // The filter does remove edges: the operator edge binding `total` to
        // `sum` is wholly inside one unit. It does not lower the maximum,
        // and should not - the peak is set by the long-range reuse of `total`
        // across three lines, which crosses units and is the structure worth
        // keeping. Local sharing goes, long-range sharing stays.
        assert_ne!(fine.profile, coarse.profile, "within-unit edges were dropped");
        assert_eq!(
            coarse.max_entanglement(),
            fine.max_entanglement(),
            "the peak is a cross-unit reuse, so it survives the filter"
        );
    }

    #[test]
    fn the_adjacency_backbone_stays_excluded_under_contraction() {
        // The backbone crosses every cut by one and says nothing about where
        // the seams are, so readmitting it would flatten the profile.
        let src = "alpha beta gamma delta epsilon\n";
        let bytes = src.as_bytes();
        let toks = lex(bytes);
        let units = crate::supertoken::supertokens_from(&toks, bytes);
        let map = relation::NodeMap::per_supertoken(&units, &toks);
        let coarse = analyze_over(&toks, bytes, &map);
        // Pure prose with no bracket and no repeat has no structural edge at
        // all; a readmitted backbone would show as a flat non-zero profile.
        assert_eq!(coarse.max_entanglement(), 0, "no structural edge to cross");
    }

    fn ent(s: &str) -> Entanglement {
        analyze_bytes(s.as_bytes())
    }

    #[test]
    fn flat_text_is_a_product_state() {
        // No structural edges - every cut has zero crossings, fully separable.
        let e = ent("quick brown fox jumps over");
        assert_eq!(e.max_entanglement(), 0);
    }

    #[test]
    fn nesting_entangles_the_interior() {
        // Enclosure edges span the bracketed interior, so cuts inside carry
        // entanglement a flat chain does not.
        let e = ent("f(g(x))");
        assert!(e.max_entanglement() > 0, "the nested interior is entangled");
    }

    #[test]
    fn two_independent_groups_split_at_a_zero_cut() {
        // Two separate bracket groups sharing no token: the boundary between them
        // is crossed by nothing - a minimal cut of zero, the clean seam.
        let e = ent("f(x) g(y)");
        let (_, crossings) = e.min_cut().expect("an interior boundary");
        assert_eq!(crossings, 0, "the seam between independent groups is clean");
    }

    #[test]
    fn a_reuse_entangles_across_its_span() {
        // A token reused across a gap binds the two sides: cuts between the two
        // occurrences carry the chord's crossing.
        let e = ent("a ( x ) b ( a )");
        assert!(e.max_entanglement() > 0, "the reuse binds across the gap");
    }
}
