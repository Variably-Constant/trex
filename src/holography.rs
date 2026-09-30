//! The holography reading - the boundary determines the bulk.
//!
//! A bracketed token stream is topological: what carries structure is the
//! connectivity (what encloses what), not the metric position of anything. In a
//! topological theory the bulk is fixed by boundary data plus the holonomies of
//! non-contractible loops - which is exactly the shape of a bracket language.
//!
//! - **Boundary** - the Dyck profile: the sequence of open (+1) / close (-1)
//!   bracket events. A one-dimensional surface.
//! - **Bulk** - the nesting tree the brackets induce, a two-dimensional interior.
//!
//! For balanced brackets the boundary determines the bulk exactly: matching each
//! close to the nearest unmatched open reconstructs the whole pairing from the
//! profile alone. That reconstruction identity is the holographic statement, and
//! it is provable, not argued - [`Holography::reconstructs_bulk`] checks the
//! boundary-rebuilt pairing against the lexer's own.
//!
//! Reuse of content is the one thing the boundary cannot see. Two leaves holding
//! the same token are identified - a handle, a non-contractible loop - and the
//! bracket profile says nothing about it. So the **holographic defect** (bulk
//! information absent from the boundary) equals the reuse-chord content, which is
//! exactly what carries [holonomy](crate::relation::RelationField::holonomy).
//! Boundary plus holonomy reconstructs the full bulk: the two axes are one
//! structure read two ways.

use crate::lexer::lex;
use crate::relation;
use crate::token::{BracketKind, Token, TokenKind};

/// The holographic reading of a token stream.
#[derive(Clone, Debug, Default)]
pub struct Holography {
    /// The Dyck boundary: `+1` per open bracket, `-1` per close, in stream order.
    pub boundary: Vec<i8>,
    /// The bracket-pairing reconstructed from the boundary alone: for each
    /// bracket token index, the index of its partner. This is the bulk tree
    /// recovered from the surface.
    pub reconstructed_mate: Vec<(usize, usize)>,
    /// Bulk nodes: the number of balanced bracket groups (interior structure).
    pub bulk_nodes: usize,
    /// The holographic defect: bulk information not present on the boundary - the
    /// reuse chords. Equals the holonomy chord count. Zero for a pure tree.
    pub holographic_defect: usize,
    /// Whether the boundary is closed (every bracket balanced). When true the
    /// boundary determines the bulk tree with no missing information.
    pub boundary_closed: bool,
}

impl Holography {
    /// Whether the bulk tree reconstructed from the boundary matches the pairing
    /// the lexer resolved directly - the holographic completeness check.
    #[must_use]
    pub fn reconstructs_bulk(&self, toks: &[Token]) -> bool {
        let mut direct: Vec<(usize, usize)> = toks
            .iter()
            .enumerate()
            .filter_map(|(i, t)| match (t.kind, t.mate()) {
                (TokenKind::Open(_), Some(m)) if i < m => Some((i, m)),
                _ => None,
            })
            .collect();
        let mut recon = self.reconstructed_mate.clone();
        direct.sort_unstable();
        recon.sort_unstable();
        direct == recon
    }

    /// Reconstruct the full bulk - every enclosure edge - from the boundary
    /// alone: the bracket tree this reading rebuilt from the Dyck profile, plus
    /// the surface heads. For each boundary-reconstructed bracket `(o, c)` whose
    /// head is the word just before `o`, every content token strictly inside
    /// receives an edge from that head. Nothing here reads the lexer's stored
    /// pairing or the relation field - the interior is traced out from the edge,
    /// the discrete form of building the bulk from boundary data.
    #[must_use]
    pub fn reconstruct_bulk(&self, toks: &[Token]) -> Vec<(usize, usize)> {
        let mut edges = Vec::new();
        for &(o, c) in &self.reconstructed_mate {
            // Head: the nearest significant token before the open, when a word.
            let head = (0..o)
                .rev()
                .find(|&j| toks[j].is_significant())
                .filter(|&j| matches!(toks[j].kind, TokenKind::Word));
            let Some(head) = head else { continue };
            // The enclosure relation covers every significant non-bracket token
            // inside (punctuation included), matching the relation tier's edges.
            for (i, t) in toks.iter().enumerate().take(c).skip(o + 1) {
                if t.is_significant() && !matches!(t.kind, TokenKind::Open(_) | TokenKind::Close(_)) {
                    edges.push((head, i));
                }
            }
        }
        edges
    }
}

fn is_open(k: TokenKind) -> bool {
    matches!(k, TokenKind::Open(_))
}

fn is_close(k: TokenKind) -> bool {
    matches!(k, TokenKind::Close(_))
}

fn bracket_kind(k: TokenKind) -> Option<BracketKind> {
    match k {
        TokenKind::Open(b) | TokenKind::Close(b) => Some(b),
        _ => None,
    }
}

/// Read the holography of an already-lexed stream.
#[must_use]
pub fn analyze(toks: &[Token], bytes: &[u8]) -> Holography {
    // The boundary is the Dyck event sequence.
    let boundary: Vec<i8> = toks
        .iter()
        .filter_map(|t| {
            if is_open(t.kind) {
                Some(1)
            } else if is_close(t.kind) {
                Some(-1)
            } else {
                None
            }
        })
        .collect();

    // Reconstruct the pairing from the boundary alone: a close matches the
    // nearest unmatched open of the same kind. This uses only open/close events
    // and their kind - the surface - never the lexer's stored mate.
    let mut stack: Vec<usize> = Vec::new();
    let mut reconstructed_mate: Vec<(usize, usize)> = Vec::new();
    let mut balanced = true;
    for (i, t) in toks.iter().enumerate() {
        if is_open(t.kind) {
            stack.push(i);
        } else if is_close(t.kind) {
            match stack.pop() {
                Some(open) if bracket_kind(toks[open].kind) == bracket_kind(t.kind) => {
                    reconstructed_mate.push((open, i));
                }
                _ => balanced = false,
            }
        }
    }
    let boundary_closed = balanced && stack.is_empty();
    let bulk_nodes = reconstructed_mate.len();

    // The holographic defect is the reuse-chord content - the bulk the boundary
    // cannot carry. It is exactly the relation tier's holonomy chords.
    let holographic_defect = relation::analyze(toks, bytes).chords.len();

    Holography { boundary, reconstructed_mate, bulk_nodes, holographic_defect, boundary_closed }
}

/// Read the holography at a chosen granularity, the reading its siblings on
/// this tier already take.
///
/// The boundary does not move: brackets are brackets whatever the nodes are,
/// so the Dyck profile and the pairing it reconstructs are unchanged. What
/// contraction changes is the defect. A reuse with both ends inside one node
/// is internal detail of that node, and a boundary describing nodes is not
/// obliged to carry it; a reuse whose ends land in different nodes is bulk the
/// boundary still cannot reach. So the defect counts the chords that survive
/// contraction, and it falls as the granularity coarsens.
///
/// What the difference measures is worth naming: the defect absorbed between
/// two granularities is the reuse the grouping made internal - structure that
/// stops being holographic because the boundary was redrawn around it.
#[must_use]
pub fn analyze_over(toks: &[Token], bytes: &[u8], map: &relation::NodeMap) -> Holography {
    let mut h = analyze(toks, bytes);
    h.holographic_defect = relation::analyze(toks, bytes)
        .chords
        .iter()
        .filter(|c| match (map.node_of(c.from), map.node_of(c.to)) {
            // Both ends in one node: the grouping absorbed it.
            (Some(a), Some(b)) => a != b,
            // An end in no node cannot be said to cross one, so it is not
            // defect at this granularity either.
            _ => false,
        })
        .count();
    h
}

#[cfg(test)]
mod grain_tests {
    use super::*;

    #[test]
    fn coarsening_absorbs_the_reuse_that_falls_inside_one_unit() {
        // A name bound and used inside one construct is a reuse the whole
        // document sees as a chord. Redraw the boundary around that construct
        // and the reuse is internal, so it stops being defect.
        let src = b"fn f(x) { x = x + 1; }\nfn g(y) { y = y + 2; }\n";
        let toks = crate::lexer::lex(src);
        let fine = analyze(&toks, src);
        let units = crate::supertoken::supertokens_from(&toks, src);
        let coarse = analyze_over(&toks, src, &relation::NodeMap::per_supertoken(&units, &toks));

        assert!(fine.holographic_defect > 0, "there is reuse to absorb");
        assert!(
            coarse.holographic_defect <= fine.holographic_defect,
            "coarsening never invents defect: {} then {}",
            fine.holographic_defect,
            coarse.holographic_defect
        );
        // The boundary is unchanged: brackets are brackets whatever the nodes.
        assert_eq!(coarse.boundary, fine.boundary);
        assert_eq!(coarse.bulk_nodes, fine.bulk_nodes);
    }

    #[test]
    fn the_identity_contraction_changes_nothing_it_should_not() {
        let src = b"a = 1; b = a + 1;";
        let toks = crate::lexer::lex(src);
        let per_token = analyze_over(&toks, src, &relation::NodeMap::per_token(toks.len()));
        let plain = analyze(&toks, src);
        assert_eq!(per_token.boundary, plain.boundary);
        assert_eq!(per_token.bulk_nodes, plain.bulk_nodes);
    }
}

/// Read the holography directly from bytes.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> Holography {
    analyze(&lex(bytes), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holo(s: &str) -> (Holography, Vec<Token>) {
        let toks = lex(s.as_bytes());
        (analyze(&toks, s.as_bytes()), toks)
    }

    #[test]
    fn flat_text_has_empty_boundary_and_no_bulk() {
        // No brackets and no repeated token: empty boundary, no bulk, no defect.
        let (h, _) = holo("quick brown fox jumps over lazy dog");
        assert!(h.boundary.is_empty());
        assert_eq!(h.bulk_nodes, 0);
        assert_eq!(h.holographic_defect, 0);
        assert!(h.boundary_closed);
    }

    #[test]
    fn a_same_level_reuse_is_defect_without_holonomy() {
        // "the ... the" at depth 0 twice: the boundary (empty) cannot encode the
        // identification, so it is a holographic defect - but the reuse crosses
        // no scope, so it carries zero holonomy. Defect counts all reuse chords;
        // holonomy counts only the scope-crossing ones.
        let (h, toks) = holo("the cat sat on the mat");
        assert_eq!(h.holographic_defect, 1);
        assert_eq!(relation::analyze(&toks, b"the cat sat on the mat").holonomy(), 0);
    }

    #[test]
    fn boundary_reconstructs_the_bulk_tree() {
        // The Dyck profile alone rebuilds the exact pairing the lexer resolved -
        // the holographic completeness of a balanced bracket structure.
        for s in ["f(g(x))", "a[b]{c}", "f(g(x), h(y))", "((()))", "a(b[c]{d})"] {
            let (h, toks) = holo(s);
            assert!(h.boundary_closed, "{s} is balanced");
            assert!(h.reconstructs_bulk(&toks), "{s}: boundary rebuilds the bulk");
        }
    }

    #[test]
    fn unbalanced_boundary_is_not_closed() {
        let (h, _) = holo("f(g(x)");
        assert!(!h.boundary_closed);
    }

    #[test]
    fn the_bulk_is_rebuilt_from_boundary_plus_holonomy() {
        // The Loop-Quantum-Gravity move, discretized: from the boundary tree and
        // the holonomy chords alone, rebuild the entire relation structure, and
        // check it equals the directly-computed bulk. Enclosure edges come from
        // the boundary; reuse identifications come from the holonomy loops;
        // together they are the whole thing.
        for s in ["f(g(x))", "a(b(a))", "f(g(x), h(y))", "a(a(a))"] {
            let (h, toks) = holo(s);
            let direct = relation::analyze(&toks, s.as_bytes());

            let mut rebuilt_encl = h.reconstruct_bulk(&toks);
            let mut direct_encl: Vec<(usize, usize)> = direct
                .edges_of(relation::RelationKind::Encloses)
                .map(|e| (e.from, e.to))
                .collect();
            rebuilt_encl.sort_unstable();
            direct_encl.sort_unstable();
            assert_eq!(rebuilt_encl, direct_encl, "{s}: boundary rebuilds the enclosure bulk");

            // The holonomy chords are the rest of the bulk - the identifications
            // the boundary cannot carry. Boundary edges + chords = full structure.
            assert_eq!(h.holographic_defect, direct.chords.len(), "{s}: chords are the defect");
        }
    }

    #[test]
    fn the_bridge_defect_equals_holonomy_chords() {
        // A pure tree: boundary-complete, zero defect, zero holonomy.
        let (tree, toks) = holo("f(g(x))");
        let rel_tree = relation::analyze(&toks, b"f(g(x))");
        assert_eq!(tree.holographic_defect, 0);
        assert_eq!(rel_tree.chords.len(), 0);

        // Reuse: the boundary still rebuilds the tree, but the defect is the
        // chord the boundary cannot see - and it equals the holonomy content.
        let (reuse, toks) = holo("a(b(a))");
        let rel_reuse = relation::analyze(&toks, b"a(b(a))");
        assert!(reuse.reconstructs_bulk(&toks), "boundary still gives the tree");
        assert_eq!(reuse.holographic_defect, rel_reuse.chords.len());
        assert!(reuse.holographic_defect > 0, "reuse is bulk off the boundary");
        assert!(rel_reuse.holonomy() > 0, "the same chord carries holonomy");
    }
}
