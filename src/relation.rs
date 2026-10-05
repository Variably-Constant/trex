//! The relation tier - trex's two-point substrate.
//!
//! Every other axis except [`echo`](crate::echo) is a one-point function: it
//! reads a local property of a window around a single position (its scale,
//! load, texture, form, symmetry, boundary, vantage). A one-point reading,
//! aggregated over a stream, is a bag of parts: it is invariant to how the
//! tokens are arranged. That invariance is exactly what makes it blind to
//! configuration - `f(g(x))` and `g(f(x))` hold the same parts and differ only
//! in which encloses which.
//!
//! The relation tier reads the two-point structure the parts drop: for a pair
//! of positions, how do they relate to one another? Three relations cover the
//! ways one token can bear on another across distance:
//!
//! - **Encloses** - a bracket headed by word `h` contains token `t` (`h` -> `t`
//!   for every `t` inside `h(...)`). This is the connection on the bracket
//!   bundle: the ordered stack of heads enclosing a token is its holonomy, the
//!   twist that a flat stream does not have.
//! - **Operator** - a binding punctuation (`=`, `:`) with an operand on each
//!   side carries a directed edge left -> right.
//! - **Adjacent** - consecutive significant tokens, the linear-order relation.
//!
//! [`echo`](crate::echo) is the fourth two-point relation - identity of content
//! across distance - and was the tier's first member. Reuse of content is more
//! than a one-point recurrence, though: it is a chord over the enclosure tree.
//! The enclosure relation alone is a tree, and a tree always closes - its
//! holonomy is trivial however deep it nests. A reuse chord adds a non-tree
//! edge, and the cycle it closes can carry holonomy: the net enclosure depth
//! the reuse jumps. A token bound at one level and used inside a deeper
//! construct closes a twisted cycle; a token repeated at the same level does
//! not. That residual, not nesting depth, is the gauge-honest holonomy, and it
//! is identity-free - it depends on that a token recurs across a scope, never on
//! which token. The separate `nesting_load` is the tree's occupancy, named for
//! what it is.
//!
//! The relations are directed, and the direction is the arrangement. An
//! undirected reading of the same edges collapses back to a bag of parts.
//!
//! [`RelationField::net_holonomy`] is the reading that uses that direction:
//! reuse flowing inward and reuse flowing outward are opposite arrangements
//! and it reports opposite signs, where the magnitude-summing
//! [`RelationField::holonomy`] reports the same number for both. The other
//! readings on this tier (topology, curvature, entanglement) are undirected by
//! definition - Betti numbers, Forman-Ricci and cut-crossing counts are all
//! defined on undirected graphs - so they symmetrise the edges deliberately
//! rather than by oversight.
//!
//! One limit on the claim, stated because the example above invites it:
//! `f(g(x))` and `g(f(x))` are isomorphic as unlabelled directed graphs, both
//! being the transitive tournament on three nodes. No label-free invariant,
//! directed or not, separates them; what differs is which identity is at
//! which position. Direction separates arrangements that differ in the shape
//! of the ordering, not ones that differ only in labelling.

use std::collections::HashMap;

use crate::lexer::lex;
use crate::token::{Token, TokenKind};

/// The kind of a directed relation between two token positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RelationKind {
    /// `from` heads a bracket group whose interior contains `to`.
    Encloses,
    /// `from` and `to` are the operands of a binding punctuation, in that order.
    Operator,
    /// `from` immediately precedes `to` in the significant-token stream.
    Adjacent,
}

/// One directed relation between two token positions (indices into the lexed
/// stream).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    /// Source token index.
    pub from: usize,
    /// Target token index.
    pub to: usize,
    /// Which relation this edge carries.
    pub kind: RelationKind,
}

/// One token's reading on the relation tier.
#[derive(Clone, Debug, Default)]
pub struct RelationFrame {
    /// Enclosure depth: the number of brackets enclosing this token.
    pub depth: u16,
    /// The heads of the enclosing brackets, outermost first - the transport
    /// path up the enclosure tree. Empty for a token at top level.
    pub enclosure: Vec<usize>,
}

/// A reuse chord: a non-tree edge closing a cycle over the enclosure tree.
///
/// The same content occurring at two positions identifies them, which adds an
/// edge the tree does not have. The `residual` is the holonomy of the cycle
/// that chord closes: the net enclosure depth the reuse jumps
/// (`depth(to) - depth(from)`). Zero when both occurrences are at the same
/// depth (the reuse stays within one scope); non-zero when the reuse crosses a
/// scope boundary (bound outside, used inside a construct) - the signature no
/// tree, and so no pure nesting, ever produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    /// The earlier occurrence's token index.
    pub from: usize,
    /// The later occurrence's token index.
    pub to: usize,
    /// The cycle holonomy: `depth(to) - depth(from)`, signed.
    pub residual: i32,
}

/// The relation side table, keyed by byte offset like the one-point fields.
#[derive(Clone, Debug, Default)]
pub struct RelationField {
    /// Token count (the lexed stream length, whitespace included).
    pub n_tokens: usize,
    /// Byte span per token - the byte-offset key shared with the other axes.
    pub spans: Vec<(usize, usize)>,
    /// One frame per token.
    pub frames: Vec<RelationFrame>,
    /// Every directed relation found, in discovery order.
    pub edges: Vec<Edge>,
    /// Reuse chords: the non-tree edges from repeated content, each carrying
    /// its cycle holonomy.
    pub chords: Vec<Chord>,
    /// Nesting load: positions inside at least one construct. A load /
    /// occupancy measure of the enclosure tree - zero for a flat stream, growing
    /// with nesting. This is not holonomy: a tree has trivial holonomy however
    /// deep it is. See [`RelationField::holonomy`].
    pub nesting_load: usize,
}

impl RelationField {
    /// Edges of one kind.
    pub fn edges_of(&self, kind: RelationKind) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter(move |e| e.kind == kind)
    }

    /// The deepest enclosure reached anywhere in the stream.
    #[must_use]
    pub fn max_depth(&self) -> u16 {
        self.frames.iter().map(|f| f.depth).max().unwrap_or(0)
    }

    /// Total cycle holonomy: the summed magnitude of every reuse chord's
    /// residual. Zero for a pure tree (no reuse, or reuse within one scope);
    /// positive when a reuse crosses a scope boundary. This is the gauge-honest
    /// holonomy - it lives in the non-tree chords, not in nesting depth.
    #[must_use]
    pub fn holonomy(&self) -> u32 {
        self.chords.iter().map(|c| c.residual.unsigned_abs()).sum()
    }

    /// Net holonomy: the summed residual with its sign kept.
    ///
    /// The one reading on this tier that changes when the arrangement is
    /// reversed. [`Self::holonomy`] sums magnitudes, so it reports the same
    /// number for a name bound outside and used deeper as for one bound deep
    /// and used outside; those are opposite arrangements and this separates
    /// them. Positive is reuse flowing inward, negative outward, zero either a
    /// flat stream or an inward and an outward reuse cancelling.
    #[must_use]
    pub fn net_holonomy(&self) -> i32 {
        self.chords.iter().map(|c| c.residual).sum()
    }

    /// The reuse chords whose residual is non-zero - the scope-crossing reuses,
    /// the twisted (holonomy-obstructed) structure.
    #[must_use]
    pub fn twisted_chords(&self) -> usize {
        self.chords.iter().filter(|c| c.residual != 0).count()
    }
}

/// The punctuation glyphs that carry a directed operator relation.
fn is_operator(text: &[u8]) -> bool {
    matches!(text, b"=" | b":")
}

/// The index of the nearest significant token strictly before `i`, if any.
fn prev_significant(toks: &[Token], i: usize) -> Option<usize> {
    (0..i).rev().find(|&j| toks[j].is_significant())
}

/// The index of the nearest significant token strictly after `i`, if any.
fn next_significant(toks: &[Token], i: usize) -> Option<usize> {
    (i + 1..toks.len()).find(|&j| toks[j].is_significant())
}

/// Which node each token belongs to, so a reading over the relation graph can
/// choose its granularity.
///
/// The relations are found once over tokens; a coarser node set is a
/// contraction of that graph rather than a second analysis, so the two
/// readings are the same relations at two scales and are directly comparable.
#[derive(Clone, Debug)]
pub struct NodeMap {
    /// Node id per token index, or `None` for a token belonging to no node.
    of_token: Vec<Option<usize>>,
    /// Number of distinct nodes.
    n_nodes: usize,
}

impl NodeMap {
    /// One node per token: the identity contraction.
    #[must_use]
    pub fn per_token(n_tokens: usize) -> Self {
        NodeMap { of_token: (0..n_tokens).map(Some).collect(), n_nodes: n_tokens }
    }

    /// One node per supertoken unit. A token outside every unit (a bracket, a
    /// separator) belongs to no node, so an edge touching one is dropped by
    /// the contraction.
    #[must_use]
    pub fn per_supertoken(units: &[crate::supertoken::SuperToken], toks: &[Token]) -> Self {
        let mut of_token = vec![None; toks.len()];
        // Units and tokens are both in stream order, so one cursor walks both.
        let mut u = 0usize;
        for (i, t) in toks.iter().enumerate() {
            while u < units.len() && units[u].end <= t.start() {
                u += 1;
            }
            if u < units.len() && t.start() >= units[u].start && t.end() <= units[u].end {
                of_token[i] = Some(u);
            }
        }
        NodeMap { of_token, n_nodes: units.len() }
    }

    /// Number of distinct nodes.
    #[must_use]
    pub fn n_nodes(&self) -> usize {
        self.n_nodes
    }

    /// The node holding token `i`, if any.
    #[must_use]
    pub fn node_of(&self, i: usize) -> Option<usize> {
        self.of_token.get(i).copied().flatten()
    }
}

impl RelationField {
    /// The undirected simple edges of this field's graph, contracted onto
    /// `map`'s nodes: deduplicated, self-edges dropped.
    ///
    /// A self-edge is a relation wholly inside one node, which says nothing
    /// about how nodes relate to each other, and an edge touching a token in no
    /// node is dropped for the same reason.
    #[must_use]
    pub fn contracted_edges(&self, map: &NodeMap) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = Vec::new();
        let mut push = |a: usize, b: usize| {
            if a != b {
                out.push((a.min(b), a.max(b)));
            }
        };
        for e in &self.edges {
            if let (Some(u), Some(v)) = (map.node_of(e.from), map.node_of(e.to)) {
                push(u, v);
            }
        }
        for c in &self.chords {
            if let (Some(u), Some(v)) = (map.node_of(c.from), map.node_of(c.to)) {
                push(u, v);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Read the relation field of an already-lexed stream.
#[must_use]
pub fn analyze(toks: &[Token], bytes: &[u8]) -> RelationField {
    let n = toks.len();
    let mut frames: Vec<RelationFrame> = Vec::with_capacity(n);
    let mut edges: Vec<Edge> = Vec::new();
    // The open-bracket stack carries (open token index, head word index).
    let mut stack: Vec<(usize, Option<usize>)> = Vec::new();

    for i in 0..n {
        let kind = toks[i].kind;
        // A close pops before we read this token's depth, so the close is at
        // the depth of the structure it releases into, not the one it ends.
        //
        // Only a bracket the lexer paired counts. An unpaired one is a
        // character in the text - free-text data and binary read as text carry
        // them constantly - and treating it as a construct would enclose the
        // whole remainder of the stream in it.
        if matches!(kind, TokenKind::Close(_)) && toks[i].mate().is_some() {
            stack.pop();
        }
        let enclosure: Vec<usize> = stack.iter().filter_map(|&(_, head)| head).collect();
        let depth = stack.len() as u16;
        // Every enclosing head bears on this token, outermost to innermost.
        // Emit the enclosure edges for a content token (not the brackets).
        if toks[i].is_significant() && !matches!(kind, TokenKind::Open(_) | TokenKind::Close(_)) {
            for &head in &enclosure {
                edges.push(Edge { from: head, to: i, kind: RelationKind::Encloses });
            }
        }
        frames.push(RelationFrame { depth, enclosure });
        // An open pushes after its own depth is recorded; its head is the
        // significant word immediately before it (a function-call head).
        if matches!(kind, TokenKind::Open(_)) && toks[i].mate().is_some() {
            let head = prev_significant(toks, i)
                .filter(|&j| matches!(toks[j].kind, TokenKind::Word));
            stack.push((i, head));
        }
    }

    // Operator edges: a binding punctuation with an operand on each side.
    // `prev_significant` / `next_significant` already skip whitespace, so the
    // neighbors they return are significant by construction.
    for i in 0..n {
        if toks[i].kind == TokenKind::Punct
            && is_operator(&bytes[toks[i].span()])
            && let (Some(l), Some(r)) = (prev_significant(toks, i), next_significant(toks, i))
        {
            edges.push(Edge { from: l, to: r, kind: RelationKind::Operator });
        }
    }

    // Adjacency edges over the significant-token subsequence.
    let sig: Vec<usize> = (0..n).filter(|&i| toks[i].is_significant()).collect();
    for w in sig.windows(2) {
        edges.push(Edge { from: w[0], to: w[1], kind: RelationKind::Adjacent });
    }

    // Reuse chords: the same content occurring twice identifies the two
    // positions, adding a non-tree edge over the enclosure tree. Chain
    // consecutive occurrences of each content token; the residual is the
    // enclosure-depth the reuse jumps - the holonomy of the cycle it closes.
    let mut last_seen: HashMap<&[u8], usize> = HashMap::new();
    let mut chords: Vec<Chord> = Vec::new();
    for i in 0..n {
        let content = toks[i].is_significant()
            && !matches!(toks[i].kind, TokenKind::Open(_) | TokenKind::Close(_) | TokenKind::Punct);
        if !content {
            continue;
        }
        let text = &bytes[toks[i].span()];
        if let Some(&prev) = last_seen.get(text) {
            let residual = i32::from(frames[i].depth) - i32::from(frames[prev].depth);
            chords.push(Chord { from: prev, to: i, residual });
        }
        last_seen.insert(text, i);
    }

    let nesting_load = frames.iter().filter(|f| f.depth > 0).count();
    let spans = toks.iter().map(|t| (t.start(), t.end())).collect();
    RelationField { n_tokens: n, spans, frames, edges, chords, nesting_load }
}

/// Read the relation field directly from bytes.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> RelationField {
    analyze(&lex(bytes), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unpaired_bracket_encloses_nothing() {
        // Free-text data carries brackets that never close - a truncated field,
        // a smiley, a byte in a binary blob read as text. Under a rule that
        // pushes every opener, one of them puts the whole remainder of the
        // stream inside it and its head takes an edge to every token after.
        let open = analyze_bytes(b"alpha [BETA;GAMMA, delta epsilon zeta eta theta");
        assert!(
            open.frames.iter().all(|f| f.depth == 0),
            "an unpaired opener is text, so nothing after it is nested"
        );
        assert!(
            open.frames.iter().all(|f| f.enclosure.is_empty()),
            "and nothing after it is enclosed by it"
        );
        assert_eq!(
            open.edges.iter().filter(|e| e.kind == RelationKind::Encloses).count(),
            0,
            "so it emits no enclosure edges"
        );

        // The same holds for a closer with nothing to close.
        let close = analyze_bytes(b"alpha BETA] gamma delta");
        assert!(close.frames.iter().all(|f| f.depth == 0), "an unpaired closer is text too");
    }

    #[test]
    fn a_paired_bracket_still_encloses_what_it_holds() {
        // The correction must not cost the reading it is there to make: a
        // bracket that does close still nests its contents and its head still
        // bears on them.
        let f = analyze_bytes(b"alpha (beta gamma) delta");
        assert!(f.frames.iter().any(|fr| fr.depth == 1), "the pair nests its contents");
        let encloses = f.edges.iter().filter(|e| e.kind == RelationKind::Encloses).count();
        assert!(encloses > 0, "and the head bears on them");

        // An unpaired opener beside a paired one leaves the paired one alone.
        let mixed = analyze_bytes(b"alpha (beta gamma) delta [EPSILON;ZETA, eta");
        assert_eq!(
            mixed.edges.iter().filter(|e| e.kind == RelationKind::Encloses).count(),
            encloses,
            "the unpaired opener adds no enclosure of its own"
        );
    }

    #[test]
    fn depth_returns_to_zero_when_every_bracket_is_unpaired() {
        // The shape that made a 250 KB parser-torture fixture cost seconds: a
        // long run of openers and nothing that closes them. Under the old rule
        // the depth climbed once per token and never came down.
        let src = b"[".repeat(200);
        let f = analyze_bytes(&src);
        assert!(f.frames.iter().all(|fr| fr.depth == 0), "none of them pair, so none of them nest");
    }

    #[test]
    fn net_holonomy_separates_arrangements_the_magnitude_collapses() {
        // `x (x)` binds outside and reuses deeper; `(x) x` is the reverse.
        // The two are opposite arrangements of the same parts.
        let inward = field("x (x)");
        let outward = field("(x) x");
        assert_eq!(inward.chords.len(), 1);
        assert_eq!(outward.chords.len(), 1);

        // The magnitude reading cannot tell them apart.
        assert_eq!(inward.holonomy(), outward.holonomy());
        // The directed one reports opposite signs.
        assert_eq!(inward.net_holonomy(), 1);
        assert_eq!(outward.net_holonomy(), -1);
        assert_eq!(inward.net_holonomy(), -outward.net_holonomy());
    }

    #[test]
    fn net_holonomy_is_zero_where_there_is_no_asymmetry() {
        // A flat reuse crosses no scope, so it has no direction to report.
        assert_eq!(field("x y x").net_holonomy(), 0);
        // An inward and an outward reuse cancel, which the magnitude sum does
        // not: it counts both.
        let mixed = field("a (a) (b) b");
        assert_eq!(mixed.net_holonomy(), 0);
        assert_eq!(mixed.holonomy(), 2, "the magnitude counts both crossings");
    }

    fn field(s: &str) -> RelationField {
        analyze_bytes(s.as_bytes())
    }

    /// The head word index of the nth significant word, for readable asserts.
    fn word_at(f: &RelationField, bytes: &[u8], want: &str) -> usize {
        (0..f.n_tokens)
            .find(|&i| &bytes[f.spans[i].0..f.spans[i].1] == want.as_bytes())
            .expect("word present")
    }

    #[test]
    fn flat_text_has_no_load_and_no_holonomy() {
        let f = field("the cat sat on the mat");
        assert_eq!(f.nesting_load, 0);
        assert_eq!(f.holonomy(), 0);
        assert_eq!(f.max_depth(), 0);
        assert!(f.edges_of(RelationKind::Encloses).next().is_none());
    }

    #[test]
    fn a_pure_tree_has_zero_holonomy_however_deep() {
        // No reuse -> no chords -> no cycle -> zero holonomy, at any depth. This
        // is the gauge-honest statement the old `holonomy_defect` name got wrong:
        // nesting is load, not holonomy.
        for s in ["f(x)", "f(g(x))", "a(b(c(d(e))))"] {
            let f = field(s);
            assert!(f.nesting_load > 0, "{s} carries nesting load");
            assert_eq!(f.holonomy(), 0, "{s} is a tree: zero holonomy");
            assert!(f.chords.is_empty(), "{s} has no reuse chords");
        }
    }

    #[test]
    fn scope_crossing_reuse_carries_holonomy() {
        // a(a): the head `a` at depth 0 is reused inside its own bracket at
        // depth 1 - a chord that jumps one scope level. Non-zero holonomy.
        let twisted = field("a(a)");
        assert_eq!(twisted.chords.len(), 1);
        assert_eq!(twisted.chords[0].residual, 1);
        assert_eq!(twisted.holonomy(), 1);
        assert_eq!(twisted.twisted_chords(), 1);

        // a(b)a: `a` reused, but both occurrences are at depth 0 - the reuse
        // stays in one scope, so the cycle closes: zero holonomy. Same token
        // multiset as a(a) plus a `b`, but the arrangement differs.
        let flat = field("a(b)a");
        assert_eq!(flat.chords.len(), 1);
        assert_eq!(flat.chords[0].residual, 0);
        assert_eq!(flat.holonomy(), 0);
        assert_eq!(flat.twisted_chords(), 0);

        // Deeper crossing carries more: a((a)) jumps two levels.
        assert_eq!(field("a((a))").holonomy(), 2);
    }

    #[test]
    fn nesting_direction_is_a_directed_edge() {
        // f(g(x)): f encloses g and x; g encloses x. The direction is the
        // arrangement - g(f(x)) reverses the f/g edge.
        let a = field("f(g(x))");
        let b = field("g(f(x))");
        let has = |f: &RelationField, bytes: &[u8], from: &str, to: &str| {
            let fi = word_at(f, bytes, from);
            let ti = word_at(f, bytes, to);
            f.edges_of(RelationKind::Encloses).any(|e| e.from == fi && e.to == ti)
        };
        // f(g(x)): f is outer, so f encloses g; not the reverse.
        assert!(has(&a, b"f(g(x))", "f", "g"));
        assert!(!has(&a, b"f(g(x))", "g", "f"));
        // g(f(x)): the edge flips.
        assert!(has(&b, b"g(f(x))", "g", "f"));
        assert!(!has(&b, b"g(f(x))", "f", "g"));
        // Same parts: identical enclosure depth multiset over the letter words
        // (both {0, 1, 2}) - the depths tie, only their binding to a word flips.
        let letter_depths = |f: &RelationField, bytes: &[u8]| {
            let mut d: Vec<u16> = (0..f.n_tokens)
                .filter(|&i| bytes[f.spans[i].0].is_ascii_alphabetic())
                .map(|i| f.frames[i].depth)
                .collect();
            d.sort_unstable();
            d
        };
        assert_eq!(letter_depths(&a, b"f(g(x))"), letter_depths(&b, b"g(f(x))"));
    }

    #[test]
    fn operator_direction_is_a_directed_edge() {
        let a = field("a = b");
        let b = field("b = a");
        let ai = word_at(&a, b"a = b", "a");
        let bi = word_at(&a, b"a = b", "b");
        assert!(a.edges_of(RelationKind::Operator).any(|e| e.from == ai && e.to == bi));
        // b = a flips it.
        let bi2 = word_at(&b, b"b = a", "b");
        let ai2 = word_at(&b, b"b = a", "a");
        assert!(b.edges_of(RelationKind::Operator).any(|e| e.from == bi2 && e.to == ai2));
    }

    #[test]
    fn deeper_nesting_raises_the_load() {
        assert_eq!(field("a b c").nesting_load, 0);
        assert!(field("f(x)").nesting_load > 0);
        assert!(field("f(g(x))").nesting_load > field("f(x)").nesting_load);
    }
}
