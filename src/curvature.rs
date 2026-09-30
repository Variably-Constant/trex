//! The curvature reading - the local twist density of the relation graph.
//!
//! Holonomy is the twist accumulated around a whole loop; curvature is its local
//! density - how a single edge sits in its neighborhood. Two nodes joined by an
//! edge either share many common neighbors (the edge is embedded in a dense,
//! triangle-rich region: positive curvature) or bridge two otherwise-separate
//! regions (a bottleneck, few or no shared neighbors: negative curvature). This
//! is the graph-Ricci curvature that explains bottlenecks in message-passing,
//! read here over trex's own relation graph.
//!
//! It sees what a two-point reading cannot. A reuse chord that binds - a binder
//! and the use it encloses - is triangle-supported: both touch the enclosing
//! structure, so the edge sits in a dense neighborhood, positive curvature. A
//! reuse that only looks like binding - a name recurring into a closed sibling
//! scope - is a bridge between two regions that share nothing, negative
//! curvature. The sign of the curvature is the distinction the chord's residual
//! alone misses.
//!
//! The measure is Forman-Ricci curvature, augmented with triangles: for an edge
//! `(u, v)` in the undirected relation graph,
//!   `Ric(u, v) = 4 - deg(u) - deg(v) + 3 * triangles(u, v)`
//! combinatorial and cheap, no optimal transport.


use crate::lexer::lex;
use crate::relation::{self, RelationField};
use crate::token::Token;

/// One edge's Forman-Ricci curvature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeCurvature {
    /// One endpoint token index (the smaller).
    pub u: usize,
    /// The other endpoint token index (the larger).
    pub v: usize,
    /// The Forman-Ricci curvature of the edge. Negative marks a bridge /
    /// bottleneck, positive a triangle-rich neighborhood.
    pub ricci: i32,
}

/// The curvature reading of the relation graph.
#[derive(Clone, Debug, Default)]
pub struct Curvature {
    /// Per-edge curvature, one entry per undirected relation edge.
    pub edges: Vec<EdgeCurvature>,
}

impl Curvature {
    /// The most negative edge curvature - the sharpest bottleneck. `0` when the
    /// graph has no edges.
    #[must_use]
    pub fn min_ricci(&self) -> i32 {
        self.edges.iter().map(|e| e.ricci).min().unwrap_or(0)
    }

    /// The mean edge curvature.
    #[must_use]
    pub fn mean_ricci(&self) -> f32 {
        if self.edges.is_empty() {
            return 0.0;
        }
        self.edges.iter().map(|e| e.ricci).sum::<i32>() as f32 / self.edges.len() as f32
    }

    /// The number of bridge edges (strictly negative curvature).
    #[must_use]
    pub fn bridges(&self) -> usize {
        self.edges.iter().filter(|e| e.ricci < 0).count()
    }

    /// The curvature of the edge between `a` and `b`, in either direction.
    #[must_use]
    pub fn edge(&self, a: usize, b: usize) -> Option<i32> {
        let (u, v) = (a.min(b), a.max(b));
        self.edges.iter().find(|e| e.u == u && e.v == v).map(|e| e.ricci)
    }
}

/// The undirected adjacency of the relation graph: every enclosure, operator,
/// adjacency edge, and reuse chord, as an undirected simple graph. Node ids are
/// token indices (dense, bounded by the token count), so the adjacency is a
/// direct-indexed `Vec` of sorted, deduplicated neighbour lists: O(1) node
/// access and cache-linear neighbour scans.
fn adjacency(field: &RelationField) -> Vec<Vec<usize>> {
    let mut max = 0usize;
    for e in &field.edges {
        max = max.max(e.from).max(e.to);
    }
    for c in &field.chords {
        max = max.max(c.from).max(c.to);
    }
    // A degree-count pass sizes each node's list exactly, so no list grows
    // push by push.
    let mut deg = vec![0u32; max + 1];
    let mut count = |a: usize, b: usize| {
        if a != b {
            deg[a] += 1;
            deg[b] += 1;
        }
    };
    for e in &field.edges {
        count(e.from, e.to);
    }
    for c in &field.chords {
        count(c.from, c.to);
    }
    let mut adj: Vec<Vec<usize>> = deg.iter().map(|&d| Vec::with_capacity(d as usize)).collect();
    let mut link = |a: usize, b: usize| {
        if a != b {
            adj[a].push(b);
            adj[b].push(a);
        }
    };
    for e in &field.edges {
        link(e.from, e.to);
    }
    for c in &field.chords {
        link(c.from, c.to);
    }
    for nbrs in &mut adj {
        nbrs.sort_unstable();
        nbrs.dedup();
    }
    adj
}

/// Read the curvature of an already-lexed stream.
#[must_use]
pub fn analyze(toks: &[Token], bytes: &[u8]) -> Curvature {
    let field = relation::analyze(toks, bytes);
    curvature_of(&adjacency(&field))
}

/// Read the curvature of the relation graph contracted onto `map`'s nodes.
///
/// Contraction changes degree, and Forman-Ricci is `4 - deg(u) - deg(v) + 3t`,
/// so a coarser reading is not a rescaling of the finer one: a node absorbs
/// its whole construct's edges and the degree term grows. Whether that reads
/// better is a question to measure per input, not to assume.
#[must_use]
pub fn analyze_over(toks: &[Token], bytes: &[u8], map: &relation::NodeMap) -> Curvature {
    let field = relation::analyze(toks, bytes);
    let pairs = field.contracted_edges(map);
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); map.n_nodes()];
    for &(u, v) in &pairs {
        adj[u].push(v);
        adj[v].push(u);
    }
    for nbrs in &mut adj {
        nbrs.sort_unstable();
    }
    curvature_of(&adj)
}

/// Forman-Ricci over an adjacency list whose neighbour lists are sorted and
/// deduplicated.
///
/// The triangle count on an edge is the size of its endpoints' common
/// neighbourhood, and what it costs is decided by which list is walked. Merging
/// both costs `deg(u) + deg(v)` per edge, so the total is the sum of `deg^2`
/// over nodes - unbounded on a graph with hubs, and a relation graph over
/// repetitive data is all hubs. Walking one list and probing the other costs
/// the length of the one walked, so the shorter is taken.
fn curvature_of(adj: &[Vec<usize>]) -> Curvature {
    let mut edges = Vec::new();
    // The neighbours of the node being emitted from, so the probe is an
    // indexed read. Rebuilt once per node, which totals the edge count.
    let mut mark = vec![false; adj.len()];
    // Ascending node order with sorted neighbour lists reproduces the exact edge
    // sequence the tree-map iteration emitted; each undirected edge appears once
    // at its smaller endpoint (v > u replaces the dedup set).
    for (u, nbrs) in adj.iter().enumerate() {
        if !nbrs.last().is_some_and(|&v| v > u) {
            continue;
        }
        for &w in nbrs {
            mark[w] = true;
        }
        for &v in nbrs {
            if v <= u {
                continue;
            }
            let du = adj[u].len() as i32;
            let dv = adj[v].len() as i32;
            // Triangles on the edge: neighbours common to both endpoints. Only
            // `u` is marked, so a shorter `adj[v]` is probed against the marks
            // and a shorter `adj[u]` searches the sorted `adj[v]` instead.
            let triangles = if adj[v].len() <= nbrs.len() {
                adj[v].iter().filter(|&&w| mark[w]).count()
            } else {
                nbrs.iter().filter(|&&w| adj[v].binary_search(&w).is_ok()).count()
            } as i32;
            let ricci = 4 - du - dv + 3 * triangles;
            edges.push(EdgeCurvature { u, v, ricci });
        }
        for &w in nbrs {
            mark[w] = false;
        }
    }
    Curvature { edges }
}

/// Read the curvature directly from bytes.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> Curvature {
    analyze(&lex(bytes), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Count of values common to two sorted, deduped slices, by two-pointer
    /// merge. The reading the marked probe has to reproduce.
    fn sorted_common(a: &[usize], b: &[usize]) -> usize {
        let (mut i, mut j, mut n) = (0usize, 0usize, 0usize);
        while i < a.len() && j < b.len() {
            match a[i].cmp(&b[j]) {
                core::cmp::Ordering::Less => i += 1,
                core::cmp::Ordering::Greater => j += 1,
                core::cmp::Ordering::Equal => {
                    n += 1;
                    i += 1;
                    j += 1;
                }
            }
        }
        n
    }

    /// Forman-Ricci by merging both neighbour lists at every edge.
    fn curvature_by_merge(adj: &[Vec<usize>]) -> Vec<EdgeCurvature> {
        let mut edges = Vec::new();
        for (u, nbrs) in adj.iter().enumerate() {
            for &v in nbrs {
                if v <= u {
                    continue;
                }
                let du = adj[u].len() as i32;
                let dv = adj[v].len() as i32;
                let triangles = sorted_common(&adj[u], &adj[v]) as i32;
                edges.push(EdgeCurvature { u, v, ricci: 4 - du - dv + 3 * triangles });
            }
        }
        edges
    }

    fn graph(n: usize, pairs: &[(usize, usize)]) -> Vec<Vec<usize>> {
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &(a, b) in pairs {
            if a != b {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
        for nbrs in &mut adj {
            nbrs.sort_unstable();
            nbrs.dedup();
        }
        adj
    }

    #[test]
    fn the_marked_probe_reads_what_the_merge_reads() {
        // Both branches of the probe have to be exercised, so the shapes
        // include a hub whose degree dwarfs its neighbours' (the shorter list
        // is the far endpoint's) and a clique where the degrees are equal.
        let star: Vec<(usize, usize)> = (1..40).map(|i| (0, i)).collect();
        let mut hub_with_rim = star.clone();
        for i in 1..39 {
            hub_with_rim.push((i, i + 1));
        }
        let clique: Vec<(usize, usize)> =
            (0..12).flat_map(|a| (a + 1..12).map(move |b| (a, b))).collect();
        let path: Vec<(usize, usize)> = (0..30).map(|i| (i, i + 1)).collect();
        let inverted: Vec<(usize, usize)> = (0..39).map(|i| (i, 39)).collect();

        for (name, n, pairs) in [
            ("star", 40, star),
            ("hub with rim", 40, hub_with_rim),
            ("clique", 12, clique),
            ("path", 31, path),
            ("late hub", 40, inverted),
        ] {
            let adj = graph(n, &pairs);
            let got = curvature_of(&adj).edges;
            let want = curvature_by_merge(&adj);
            assert_eq!(got.len(), want.len(), "{name}: edge count");
            for (g, w) in got.iter().zip(&want) {
                assert_eq!((g.u, g.v, g.ricci), (w.u, w.v, w.ricci), "{name}: edge order and value");
            }
        }
    }

    /// Curvature at both granularities for the same input.
    fn both(src: &str) -> (Curvature, Curvature) {
        let bytes = src.as_bytes();
        let toks = lex(bytes);
        let units = crate::supertoken::supertokens_from(&toks, bytes);
        let map = relation::NodeMap::per_supertoken(&units, &toks);
        (analyze(&toks, bytes), analyze_over(&toks, bytes, &map))
    }

    #[test]
    fn supertoken_nodes_separate_structure_from_repetition() {
        // The same result topology showed: at token granularity the reading is
        // dominated by the adjacency backbone and repeated words, so prose
        // looks as bottlenecked as code. Over constructs, prose and flat logs
        // carry no bridge at all.
        let (prose_t, prose_u) =
            both("the quick brown fox jumps over the lazy dog\nand then the dog looks up at the fox\n");
        let (log_t, log_u) = both("INFO start id=1\nINFO stop id=1\nWARN retry id=2\n");
        let (code_t, code_u) =
            both("let total = sum(price, tax);\nlet net = round(total);\nprint(net, total);\n");

        assert!(prose_t.bridges() > 0, "the token graph finds bridges in prose");
        assert!(log_t.bridges() > 0, "the token graph finds bridges in flat logs");
        assert!(code_t.bridges() > 0);
        assert_eq!(prose_u.bridges(), 0, "prose has no construct bridging two regions");
        assert_eq!(log_u.bridges(), 0, "flat log lines have none either");
        assert!(prose_u.min_ricci() > 0, "and its edges sit in dense neighbourhoods");
        assert!(code_u.min_ricci() <= 0, "code keeps at least one non-positive edge");
    }

    #[test]
    fn the_identity_contraction_reproduces_the_token_reading() {
        let src = "f(g(x), y)\nh(x)\n";
        let bytes = src.as_bytes();
        let toks = lex(bytes);
        let direct = analyze(&toks, bytes);
        let via_map = analyze_over(&toks, bytes, &relation::NodeMap::per_token(toks.len()));
        assert_eq!(direct.min_ricci(), via_map.min_ricci());
        assert_eq!(direct.bridges(), via_map.bridges());
        assert_eq!(direct.edges.len(), via_map.edges.len());
    }

    fn curv(s: &str) -> Curvature {
        analyze_bytes(s.as_bytes())
    }

    #[test]
    fn a_plain_chain_is_flat() {
        // A repeat-free sequence is a path graph: interior edges join degree-2
        // nodes with no shared neighbours, so Forman curvature is 0 (or +1 at
        // the ends). No triangles, no bridges - the graph is flat.
        let c = curv("quick brown fox jumps over");
        assert!(!c.edges.is_empty());
        assert!(c.edges.iter().all(|e| (0..=1).contains(&e.ricci)), "a path is flat");
        assert_eq!(c.bridges(), 0);
    }

    #[test]
    fn enclosure_triangles_raise_curvature() {
        // f(g(x)): f encloses g and x, g encloses x, and adjacency links them -
        // the (f, x) and (g, x) region carries triangles, so some edge curves
        // upward relative to a bare bridge.
        let nested = curv("f(g(x))");
        let flat = curv("f g x");
        assert!(
            nested.edges.iter().map(|e| e.ricci).max().unwrap_or(i32::MIN)
                > flat.edges.iter().map(|e| e.ricci).max().unwrap_or(i32::MIN),
            "nesting builds triangles a flat chain lacks"
        );
    }

    #[test]
    fn a_reused_token_that_binds_curves_above_a_sibling_bridge() {
        // Binding reuse - the binder encloses the use - sits in the enclosure
        // neighborhood (triangle support). A sibling-scope reuse bridges two
        // regions that share nothing. The binding chord curves above the bridge.
        let bound = curv("[ q ( q ) ]");
        let sibling = curv("[ q ( b ) ] ( ( q ) )");
        // Locate each graph's reuse chord (the q-q edge) and compare curvature.
        let field_b = relation::analyze_bytes(b"[ q ( q ) ]");
        let cb = field_b.chords[0];
        let field_s = relation::analyze_bytes(b"[ q ( b ) ] ( ( q ) )");
        let cs = field_s.chords[0];
        let kb = bound.edge(cb.from, cb.to).expect("bound chord curvature");
        let ks = sibling.edge(cs.from, cs.to).expect("sibling chord curvature");
        assert!(kb > ks, "binding chord ({kb}) curves above the sibling bridge ({ks})");
    }
}
