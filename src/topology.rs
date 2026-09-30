//! The topology reading - the global loop structure of the relation graph.
//!
//! Holonomy is the twist around one loop; curvature is that twist's local
//! density on one edge. Topology is the global count that ties them: how many
//! independent loops the structure has at all. For a graph with `V` nodes, `E`
//! edges, and `b0` connected components, the first Betti number (the cycle rank)
//! is `b1 = E - V + b0`. It is the number of independent cycles: the dimension of
//! the space of loops, each of which carries its own holonomy. A tree (a pure
//! nesting with no reuse and no cross-links) has `b1 = 0`; every independent
//! long-range dependency adds one.
//!
//! `b0` (components) counts the separate pieces - passages that share no token
//! and no bracket. `b1` (cycle rank) counts the loops. The Euler characteristic
//! `chi = V - E = b0 - b1` is their signed combination, the coarsest topological
//! invariant of the structure.


use crate::lexer::lex;
use crate::relation;
use crate::token::Token;

/// The topological invariants of the relation graph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Topology {
    /// Nodes: tokens that take part in at least one relation.
    pub nodes: usize,
    /// Undirected edges of the relation graph.
    pub edges: usize,
    /// `b0`: connected components.
    pub components: usize,
    /// `b1 = E - V + b0`: the cycle rank, the number of independent loops.
    pub cycle_rank: usize,
}

impl Topology {
    /// The Euler characteristic `chi = V - E`, equal to `b0 - b1`.
    #[must_use]
    pub fn euler(&self) -> i64 {
        self.nodes as i64 - self.edges as i64
    }
}

/// Union-find over a small dense index space.
struct Uf {
    parent: Vec<usize>,
}

impl Uf {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect() }
    }
    fn find(&mut self, x: usize) -> usize {
        let mut r = x;
        while self.parent[r] != r {
            r = self.parent[r];
        }
        // Path compression.
        let mut c = x;
        while self.parent[c] != r {
            let next = self.parent[c];
            self.parent[c] = r;
            c = next;
        }
        r
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

/// Read the topology of an already-lexed stream.
#[must_use]
pub fn analyze(toks: &[Token], bytes: &[u8]) -> Topology {
    let field = relation::analyze(toks, bytes);

    // Undirected simple edges over the relation graph (enclosure, operator,
    // adjacency, reuse), then Betti numbers by union-find. Node ids are token
    // indices (dense), so the dedup is sort+dedup on a flat Vec and the
    // compaction a direct-indexed sentinel table - no tree maps on the hot
    // per-turn extraction path.
    let mut undirected: Vec<(usize, usize)> = Vec::new();
    let mut push = |a: usize, b: usize| {
        if a != b {
            undirected.push((a.min(b), a.max(b)));
        }
    };
    for e in &field.edges {
        push(e.from, e.to);
    }
    for c in &field.chords {
        push(c.from, c.to);
    }
    undirected.sort_unstable();
    undirected.dedup();
    of_edges(&undirected)
}

/// The topology of a graph given as undirected simple edges, sorted and
/// deduplicated, with node ids of any density.
///
/// The node ids carry no meaning here, so this serves a token-node graph and a
/// contracted one alike; [`analyze_over`] is the contracted entry.
#[must_use]
pub fn of_edges(undirected: &[(usize, usize)]) -> Topology {
    // Compact the participating token indices to a dense range (first-seen
    // order over the sorted edge list - identical numbering to the tree-map).
    let max_tok = undirected.iter().map(|&(u, v)| u.max(v)).max().unwrap_or(0);
    let mut ids: Vec<usize> = vec![usize::MAX; max_tok + 1];
    let mut nodes = 0usize;
    for &(u, v) in undirected {
        if ids[u] == usize::MAX {
            ids[u] = nodes;
            nodes += 1;
        }
        if ids[v] == usize::MAX {
            ids[v] = nodes;
            nodes += 1;
        }
    }
    let edges = undirected.len();
    let mut uf = Uf::new(nodes.max(1));
    for &(u, v) in undirected {
        uf.union(ids[u], ids[v]);
    }
    let components = if nodes == 0 {
        0
    } else {
        let mut root_seen = vec![false; nodes];
        let mut n = 0usize;
        for i in 0..nodes {
            let r = uf.find(i);
            if !root_seen[r] {
                root_seen[r] = true;
                n += 1;
            }
        }
        n
    };
    // b1 = E - V + b0 (never negative for a real graph).
    let cycle_rank = (edges + components).saturating_sub(nodes);
    Topology { nodes, edges, components, cycle_rank }
}

/// Read the topology of the relation graph contracted onto `map`'s nodes.
///
/// The relations are found once over tokens and then contracted, so a coarser
/// reading is the same structure at a larger scale.
#[must_use]
pub fn analyze_over(toks: &[Token], bytes: &[u8], map: &relation::NodeMap) -> Topology {
    of_edges(&relation::analyze(toks, bytes).contracted_edges(map))
}

/// Read the topology directly from bytes.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> Topology {
    analyze(&lex(bytes), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cycle rank at both granularities, for the same input.
    fn both(src: &str) -> (usize, usize) {
        let bytes = src.as_bytes();
        let toks = lex(bytes);
        let units = crate::supertoken::supertokens_from(&toks, bytes);
        let field = relation::analyze(&toks, bytes);
        let per_token = relation::NodeMap::per_token(toks.len());
        let per_unit = relation::NodeMap::per_supertoken(&units, &toks);
        (
            of_edges(&field.contracted_edges(&per_token)).cycle_rank,
            of_edges(&field.contracted_edges(&per_unit)).cycle_rank,
        )
    }

    #[test]
    fn supertoken_nodes_separate_structure_from_repetition() {
        // Prose and flat log lines carry no construct depending on another.
        // A token graph still reports cycles for both, because its adjacency
        // backbone plus repeated words closes them; the count is reading
        // vocabulary repetition. Contracted onto constructs, both go to zero.
        let (prose_tok, prose_unit) =
            both("the quick brown fox jumps over the lazy dog\nand then the dog looks up at the fox\n");
        let (log_tok, log_unit) = both("INFO start id=1\nINFO stop id=1\nWARN retry id=2\n");
        assert!(prose_tok > 0, "the token graph finds cycles in prose: {prose_tok}");
        assert!(log_tok > 0, "the token graph finds cycles in flat logs: {log_tok}");
        assert_eq!(prose_unit, 0, "prose has no construct depending on another");
        assert_eq!(log_unit, 0, "flat log lines have no construct depending on another");

        // Code that rebinds a name, and config repeating keys across blocks,
        // both keep a non-zero rank at construct scale.
        let (_, code_unit) =
            both("let total = sum(price, tax);\nlet net = round(total);\nprint(net, total);\n");
        let (_, cfg_unit) = both(
            "server {\n  host: example.com\n  port: 8080\n}\nclient {\n  host: example.com\n  port: 9090\n}\n",
        );
        assert!(code_unit > 0, "a rebound name is an independent dependency: {code_unit}");
        assert!(cfg_unit > 0, "keys repeated across blocks close a cycle: {cfg_unit}");
    }

    #[test]
    fn the_identity_contraction_reproduces_the_token_reading() {
        let src = "f(g(x), y)\nh(x)\n";
        let bytes = src.as_bytes();
        let toks = lex(bytes);
        let direct = analyze(&toks, bytes);
        let via_map =
            analyze_over(&toks, bytes, &relation::NodeMap::per_token(toks.len()));
        assert_eq!(direct.nodes, via_map.nodes);
        assert_eq!(direct.edges, via_map.edges);
        assert_eq!(direct.components, via_map.components);
        assert_eq!(direct.cycle_rank, via_map.cycle_rank);
    }

    fn topo(s: &str) -> Topology {
        analyze_bytes(s.as_bytes())
    }

    #[test]
    fn a_path_is_acyclic() {
        // A repeat-free flat chain is a path graph: connected, no loops.
        let t = topo("quick brown fox jumps");
        assert_eq!(t.components, 1);
        assert_eq!(t.cycle_rank, 0);
        // Euler characteristic of a tree is +1 (V - E = 1).
        assert_eq!(t.euler(), 1);
    }

    #[test]
    fn nesting_and_reuse_add_independent_loops() {
        // Enclosure edges over the linear backbone already close loops; reuse
        // adds more. Cycle rank rises with structure.
        let flat = topo("a b c").cycle_rank;
        let nested = topo("f(g(x))").cycle_rank;
        let reuse = topo("f(g(x)) x").cycle_rank;
        assert_eq!(flat, 0, "a flat chain has no loops");
        assert!(nested > flat, "nesting closes loops");
        assert!(reuse > nested, "a reuse adds an independent loop");
    }

    #[test]
    fn separate_passages_are_separate_components() {
        // Two runs that share no token and no bracket are two components. A
        // dot is a Punct token between them but does not join the two words.
        let t = topo("alpha beta");
        assert_eq!(t.components, 1, "adjacency joins a single run");
        // Betti-0 counts pieces; a single run is one piece.
        assert!(t.cycle_rank == 0);
    }
}
