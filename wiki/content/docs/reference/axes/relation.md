---
title: Relation
linkTitle: Relation
weight: 95
---

# The relation axis

The directed graph between token positions: which bracket head encloses which token, which
operands a binding punctuation joins, which tokens are adjacent, and where content repeats; and
what that graph's tree and its loops measure.

Source: [`src/relation.rs`](https://github.com/Variably-Constant/trex/blob/main/src/relation.rs),
with the graph readings in `holography.rs`, `curvature.rs`, `topology.rs`, `geodesic.rs`,
`entanglement.rs` and `gauge.rs`.

## The relations

| Edge | From | To |
|---|---|---|
| `Encloses` | the word heading a paired bracket | each content token inside it |
| `Operator` | the left operand of `=` or `:` | the right operand |
| `Adjacent` | a significant token | the next significant token |

An unpaired bracket encloses nothing. Each content token's frame records its depth, the number of
paired brackets around it, and the heads enclosing it, outermost first.

A repeated word, number or literal adds a reuse chord from its previous occurrence. The chord's
residual is the depth of the later occurrence less the depth of the earlier: zero for a reuse
within one scope, positive for a name used deeper than it was first seen, negative for one used
shallower. The holonomy is the summed absolute residual, the net holonomy the summed signed
residual, and the nesting load the number of tokens inside at least one bracket.

## Reading the axis

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex relation --text 'x = 1; f(x, g(x))'
trex relation: 17 bytes, 17 tokens, max depth 2, nesting load 7
  edges: 5 encloses, 1 operator, 12 adjacent
  holonomy: 2 (2 reuse chord(s), 2 scope-crossing), net +2 (reuse flows inward)
  holography: 4 boundary events, 2 bulk node(s), holographic defect 2 (= reuse chords)
  bulk = 5 enclosure edge(s) rebuilt from the boundary + 2 holonomy chord(s)
  curvature: min -4 (sharpest bottleneck), mean -0.35, 8 bridge edge(s)
  topology: 13 nodes, 20 edges, b0 1 component(s), b1 8 independent loop(s), euler -7
  geodesic: longest reuse shortcut collapses a 9-token span to one hop
  entanglement: peak 5, minimal cut 0 crossing(s) before token 15
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::relation::{RelationKind, analyze_bytes};

let text = b"x = 1; f(x, g(x))";
let field = analyze_bytes(text);
assert_eq!(field.max_depth(), 2);
assert_eq!(field.edges_of(RelationKind::Encloses).count(), 5);
assert_eq!(field.edges_of(RelationKind::Operator).count(), 1);
assert_eq!((field.chords.len(), field.holonomy(), field.net_holonomy()), (2, 2, 2));
assert_eq!(trex::topology::analyze_bytes(text).cycle_rank, 8);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexRelation 'x = 1; f(x, g(x))' | Select-Object MaxDepth, Encloses, Operators, ReuseChords, Holonomy, NetHolonomy, CycleRank | Format-List

MaxDepth    : 2
Encloses    : 5
Operators   : 1
ReuseChords : 2
Holonomy    : 2
NetHolonomy : 2
CycleRank   : 8
```
{{< /tab >}}
{{< /tabs >}}

`x` is bound at depth 0 and used at depths 1 and 2, so both chords cross a scope and the net
holonomy is +2.

| Flag | Effect |
|---|---|
| `--edges` | every directed edge, then every reuse chord with its residual |
| `--field` | each enclosed token's depth and enclosing heads |
| `--gauge` | the [alpha-equivalence form](#alpha-equivalence) |
| `--limit N` | at most `N` rows of `--edges` and `--field` |

In PowerShell `-Detail` adds the `Edges`, `Chords` and `Frames` arrays and `-Canonical` the
alpha-equivalence form:

```powershell
PS> (Measure-TrexRelation 'x = 1; f(x, g(x))' -Detail).Chords | Format-Table

From To Residual FromOffset ToOffset
---- -- -------- ---------- --------
x    x  1        0          9
x    x  1        9          14
```

## Graph readings

The readings below are of the same edges and chords, taken undirected.

| Reading | What it counts |
|---|---|
| holography | the bracket open and close events (boundary events), the bracket pairs (bulk nodes), the enclosure edges rebuilt from the open and close sequence alone, and the reuse chords that sequence cannot carry (the holographic defect) |
| curvature | per edge, `4 - deg(u) - deg(v) + 3 * triangles(u, v)`: the minimum, the mean, and the bridges, the edges below zero |
| topology | nodes `V`, edges `E`, components `b0`, independent loops `b1 = E - V + b0`, and `V - E` |
| geodesic | the most tokens a single reuse chord spans |
| entanglement | per cut between two tokens, the enclosure, operator and reuse edges that cross it; the peak, and the first interior cut with the fewest crossings |

The CLI names the minimal cut by token index; PowerShell's `MinimalCutOffset` is the byte offset
of that token.

## Alpha-equivalence

The first word inside a `[ ]` binds its scope. The alpha-equivalence form writes each binder as
`#`, each use of a bound name as `^k`, where `k` counts the `[ ]` scopes between the use and its
binder, and keeps a free name literal, so two inputs that differ only in their bound names read
alike:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex relation --gauge --text '[ x ( x ) ]'
... the readings ...
  gauge-fixed (alpha-equivalence canonical form):
    [ # ( ^0 ) ]

$ trex relation --gauge --text '[ y ( y ) ]'
... the readings ...
  gauge-fixed (alpha-equivalence canonical form):
    [ # ( ^0 ) ]

$ trex relation --gauge --text '[ x ( y ) ]'
... the readings ...
  gauge-fixed (alpha-equivalence canonical form):
    [ # ( y ) ]
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::gauge::{alpha_equivalent, canonicalize_bytes};

assert_eq!(canonicalize_bytes(b"[ x ( x ) ]"), "[ # ( ^0 ) ]");
assert!(alpha_equivalent(b"[ x ( x ) ]", b"[ y ( y ) ]"));
assert_eq!(canonicalize_bytes(b"[ x ( y ) ]"), "[ # ( y ) ]");
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexRelation '[ x ( x ) ]' -Canonical).Canonical
[ # ( ^0 ) ]

PS> (Measure-TrexRelation '[ y ( y ) ]' -Canonical).Canonical
[ # ( ^0 ) ]

PS> (Measure-TrexRelation '[ x ( y ) ]' -Canonical).Canonical
[ # ( y ) ]
```
{{< /tab >}}
{{< /tabs >}}

## Data model

`RelationField`:

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | tokens in the lexed stream, whitespace included |
| `spans` | `Vec<(usize, usize)>` | byte span per token |
| `frames` | `Vec<RelationFrame>` | per token: `depth` and `enclosure`, the enclosing heads outermost first |
| `edges` | `Vec<Edge>` | `from`, `to` and `kind` of every edge |
| `chords` | `Vec<Chord>` | `from`, `to` and `residual` of every reuse chord |
| `nesting_load` | `usize` | tokens inside at least one paired bracket |

| Method | Returns |
|---|---|
| `edges_of(kind)` | the edges of one kind |
| `max_depth()` | the deepest enclosure |
| `holonomy()` / `net_holonomy()` | the summed absolute and signed residuals |
| `twisted_chords()` | the chords with a non-zero residual |
| `contracted_edges(&map)` | the undirected edges between the nodes of a `NodeMap`: one per token, or one per supertoken |

`relation::analyze(toks, bytes)` reads an already-lexed stream and `analyze_bytes(bytes)` lexes
first. `holography`, `curvature`, `topology`, `geodesic` and `entanglement` each have
`analyze(toks, bytes)`, `analyze_bytes(bytes)` and `analyze_over(toks, bytes, &map)`, the last
reading the graph contracted onto a `NodeMap`.

## Cost

The enclosure, operator and adjacency edges are one pass over the tokens with a stack of open
brackets; the reuse chords one pass with a map from content to its last position. An enclosed
token takes one `Encloses` edge from each head above it, so the edges grow with the tokens times
the depth.
