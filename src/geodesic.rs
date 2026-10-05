//! The geodesic reading - distance along the relation graph, not along the line.
//!
//! Two tokens can be far apart in the byte stream yet one step apart in the
//! structure: a reuse chord identifies them, an enclosure edge binds a head to a
//! deep leaf. The geodesic distance is the shortest path through the relation
//! graph, and it is the structural distance the linear offset misuses. Where the
//! geodesic is far below the linear gap, a long-range dependency has collapsed
//! the distance - a shortcut through the structure. Where it is far above (two
//! adjacent tokens separated by a bracket wall), the surface neighbors are
//! structurally remote.

use std::collections::{BTreeMap, VecDeque};

use crate::lexer::lex;
use crate::relation;
use crate::token::Token;

/// The geodesic reading: the relation graph as an adjacency structure, with
/// shortest-path queries over it.
#[derive(Clone, Debug, Default)]
pub struct Geodesic {
    /// Token index -> compact node id (only tokens that take part in a relation).
    id: BTreeMap<usize, usize>,
    /// Adjacency by compact node id.
    adj: Vec<Vec<usize>>,
    /// The reuse chords, kept for the shortcut summary: (from, to).
    chords: Vec<(usize, usize)>,
}

impl Geodesic {
    /// The geodesic distance in graph hops between two token indices, or `None`
    /// when they are in different components (or absent from the graph).
    #[must_use]
    pub fn distance(&self, a: usize, b: usize) -> Option<usize> {
        let (&sa, &sb) = (self.id.get(&a)?, self.id.get(&b)?);
        if sa == sb {
            return Some(0);
        }
        let mut dist = vec![usize::MAX; self.adj.len()];
        let mut q = VecDeque::new();
        dist[sa] = 0;
        q.push_back(sa);
        while let Some(u) = q.pop_front() {
            if u == sb {
                return Some(dist[u]);
            }
            for &w in &self.adj[u] {
                if dist[w] == usize::MAX {
                    dist[w] = dist[u] + 1;
                    q.push_back(w);
                }
            }
        }
        None
    }

    /// The graph diameter: the largest finite geodesic between any two nodes.
    /// All-pairs BFS - for the whole-structure view, not a hot path.
    #[must_use]
    pub fn diameter(&self) -> usize {
        let n = self.adj.len();
        let mut best = 0;
        for s in 0..n {
            let mut dist = vec![usize::MAX; n];
            let mut q = VecDeque::new();
            dist[s] = 0;
            q.push_back(s);
            while let Some(u) = q.pop_front() {
                for &w in &self.adj[u] {
                    if dist[w] == usize::MAX {
                        dist[w] = dist[u] + 1;
                        best = best.max(dist[w]);
                        q.push_back(w);
                    }
                }
            }
        }
        best
    }

    /// The largest linear token span a single reuse chord collapses to one hop -
    /// the longest-range dependency the structure carries. `0` when there is no
    /// reuse.
    #[must_use]
    pub fn max_shortcut(&self) -> usize {
        self.chords.iter().map(|&(f, t)| t.abs_diff(f)).max().unwrap_or(0)
    }
}

/// Read the geodesic structure of an already-lexed stream.
#[must_use]
pub fn analyze(toks: &[Token], bytes: &[u8]) -> Geodesic {
    let field = relation::analyze(toks, bytes);
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for e in &field.edges {
        if e.from != e.to {
            edges.push((e.from, e.to));
        }
    }
    let chords: Vec<(usize, usize)> = field.chords.iter().map(|c| (c.from, c.to)).collect();
    for &(f, t) in &chords {
        if f != t {
            edges.push((f, t));
        }
    }

    let mut id: BTreeMap<usize, usize> = BTreeMap::new();
    for &(u, v) in &edges {
        let n = id.len();
        id.entry(u).or_insert(n);
        let n = id.len();
        id.entry(v).or_insert(n);
    }
    let mut adj = vec![Vec::new(); id.len()];
    for &(u, v) in &edges {
        let (iu, iv) = (id[&u], id[&v]);
        adj[iu].push(iv);
        adj[iv].push(iu);
    }
    Geodesic { id, adj, chords }
}

/// Read the geodesic structure of the relation graph contracted onto `map`'s
/// nodes.
///
/// The chords carry over as node pairs, so `max_shortcut` reports the largest
/// span a reuse collapses measured in nodes rather than in tokens.
#[must_use]
pub fn analyze_over(toks: &[Token], bytes: &[u8], map: &relation::NodeMap) -> Geodesic {
    let field = relation::analyze(toks, bytes);
    let edges = field.contracted_edges(map);
    let chords: Vec<(usize, usize)> = field
        .chords
        .iter()
        .filter_map(|c| match (map.node_of(c.from), map.node_of(c.to)) {
            (Some(u), Some(v)) if u != v => Some((u, v)),
            _ => None,
        })
        .collect();

    let mut id: BTreeMap<usize, usize> = BTreeMap::new();
    for &(u, v) in &edges {
        let n = id.len();
        id.entry(u).or_insert(n);
        let n = id.len();
        id.entry(v).or_insert(n);
    }
    let mut adj = vec![Vec::new(); id.len()];
    for &(u, v) in &edges {
        let (iu, iv) = (id[&u], id[&v]);
        adj[iu].push(iv);
        adj[iv].push(iu);
    }
    Geodesic { id, adj, chords }
}

/// Read the geodesic structure directly from bytes.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> Geodesic {
    analyze(&lex(bytes), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word_index(bytes: &[u8], nth_word: &str, occurrence: usize) -> usize {
        let toks = lex(bytes);
        toks.iter()
            .enumerate()
            .filter(|(_, t)| &bytes[t.span()] == nth_word.as_bytes())
            .nth(occurrence)
            .map(|(i, _)| i)
            .expect("occurrence present")
    }

    #[test]
    fn a_reuse_collapses_linear_distance_to_one_hop() {
        // "the" recurs far apart linearly, but the reuse chord makes them
        // geodesically adjacent.
        let s = b"the cat sat on a warm mat near the door";
        let g = analyze(&lex(s), s);
        let first = word_index(s, "the", 0);
        let second = word_index(s, "the", 1);
        assert!(second - first > 5, "linearly far apart");
        assert_eq!(g.distance(first, second), Some(1), "one hop through the reuse chord");
        assert!(g.max_shortcut() >= second - first);
    }

    #[test]
    fn a_path_distance_equals_the_hop_count() {
        // A flat chain: geodesic between the ends equals the number of steps.
        let s = b"alpha beta gamma delta";
        let g = analyze(&lex(s), s);
        let a = word_index(s, "alpha", 0);
        let d = word_index(s, "delta", 0);
        assert_eq!(g.distance(a, d), Some(3), "three adjacency hops end to end");
        assert_eq!(g.max_shortcut(), 0, "no reuse, no shortcut");
    }

    #[test]
    fn nesting_keeps_the_diameter_small() {
        // Enclosure edges from the head reach every leaf directly, so a nested
        // group has a small diameter despite its depth.
        let s = b"f(g(h(x)))";
        let g = analyze(&lex(s), s);
        assert!(g.diameter() >= 1);
    }
}
