---
title: Orbit
linkTitle: Orbit
weight: 40
---

# The orbit axis: symmetry and grain separation

The symmetry axis. Where BPE asks *where to merge* and [seam](../seam/) asks *where to cut*,
the orbit axis asks *what is the same?* A token, on this axis, is the canonical
representative of its orbit under a symmetry group `G`: two spans that differ only by a
symmetry of `G` are one token.

Source: [`src/orbit.rs`](https://github.com/Variably-Constant/trex/blob/main/src/orbit.rs), [`src/canon.rs`](https://github.com/Variably-Constant/trex/blob/main/src/canon.rs) · CLI:
[`trex orbit`](../cli/#orbit).

## Contents

- [Three axes of tokenization](#three-axes-of-tokenization)
- [The principle in canon.rs](#the-principle-in-canonrs)
- [The orbit ladder](#the-orbit-ladder)
- [Three uses, one function](#three-uses-one-function)
- [CLI](#cli)
- [Hierarchical grain separation](#hierarchical-grain-separation)
- [Design decisions](#design-decisions)
- [Cross-system applicability](#cross-system-applicability)

## Three axes of tokenization

A tokenizer answers one of three independent questions about a stream:

| Axis | Question | Method | Signal |
|---|---|---|---|
| Frequency | where to **merge**? | BPE ([`src/bpe.rs`](https://github.com/Variably-Constant/trex/blob/main/src/bpe.rs)) | co-occurrence count |
| Predictability | where to **cut**? | seam ([`src/seam.rs`](https://github.com/Variably-Constant/trex/blob/main/src/seam.rs)) | branching entropy |
| **Symmetry** | what is the **same**? | orbit ([`src/orbit.rs`](https://github.com/Variably-Constant/trex/blob/main/src/orbit.rs)) | group invariance |

The first two partition the stream into spans. The symmetry axis is orthogonal to both: it
does not decide *where* the spans fall but *what identity* each span carries. You can cut
identically and identify differently (fold case or not), or identify the same content under
different cuts. Standard tokenizers do a sliver of this ad-hoc (Unicode normalisation,
lowercasing); the symmetry axis makes it a principled group quotient
`bytes -> bytes / G`.

## The principle in canon.rs

[`src/canon.rs`](https://github.com/Variably-Constant/trex/blob/main/src/canon.rs) computes orbit canonical representatives under one stated
law: *a checkable zero is an invariant under a symmetry group, and the zero is reached at
the canonical orbit representative.*

- **Notation orbit** (`canon_symbol`): a Greek glyph, its TeX command, and the bare name
  collapse to one key (`theta` = `\theta` = the glyph); case folds too. The orbit under the
  choice of surface notation.
- **Register-renaming orbit** (`rename_invariant_sig`): two machine-code blocks that differ
  only in which registers they use share a signature, by relabelling registers in
  first-occurrence order. The orbit under register permutation - a working orbit-based
  tokenizer for code.

[`src/orbit.rs`](https://github.com/Variably-Constant/trex/blob/main/src/orbit.rs) lifts this from special cases into one tokenization axis
with a ladder of groups.

## The orbit ladder

| Group `G` | one token (orbit) | where |
|---|---|---|
| identity | literal bytes | `OrbitGroup::Identity` (the trivial quotient) |
| case | `Cat` = `cat` = `CAT` | `OrbitGroup::Case` |
| notation | `theta` = `\theta` = glyph | `OrbitGroup::Notation` (reuses `canon`) |
| within-class shape | `cat` = `dog` = `bat` (`CVC`) | `OrbitGroup::Shape` |
| register renaming | code blocks up to register relabel | `canon::rename_invariant_sig` |

The within-class **shape** orbit is the formal version of word structure: a word's shape is
its consonant / vowel / digit pattern, invariant to which specific symbol fills each slot.
`cat`, `dog`, and `bat` are one orbit (`CVC`) under substituting one symbol for another of
the same class. An unseen word maps to a *known* shape orbit, so the axis generalises rather
than memorises.

## Three uses, one function

Every use derives from `orbit::canonical(span, group)`.

- **Collapse** (identity): symmetry-equivalent spans become one token. `collapse` and
  `collapse_stats` report the vocabulary the quotient yields and which raw forms fold onto
  each orbit.
- **Boundary** (orbit-change): `shape_boundaries` cuts where the symbol *kind* changes - the
  stream's stratification into maximal same-class runs, a boundary signal orthogonal to
  frequency and predictability.
- **Match** (equivariance): `matches` returns every span in a query's orbit, so a query
  matches *up to the group* rather than byte-for-byte (`same_orbit` is the membership test).

## CLI

```console
$ trex orbit --collapse --group shape --text 'cat dog bat sat mat the fox'
trex orbit --collapse (group shape): 7 raw forms -> 2 orbits (3.5x reduction)
    "CCV"
    "CVC"  <-  ["bat", "cat", "dog", "fox", "mat", "sat"]

$ trex orbit --boundary --text 'cat123dog!!'
trex orbit --boundary: 3 symbol-kind transitions
    [     0..3     ] "cat"
    [     3..6     ] "123"
    [     6..9     ] "dog"
    [     9..11    ] "!!"

$ trex orbit --match cat --group case --text 'the Cat and the CAT and a cat'
trex orbit --match "cat" (group case): 3 spans in the same orbit
    [     4..7     ] "Cat"
    [    16..19    ] "CAT"
    [    26..29    ] "cat"
```

| Flag | Effect |
|---|---|
| `--group identity\|case\|notation\|shape` | the symmetry group (default `shape`) |
| `--collapse` | orbit-collapse table and vocabulary reduction |
| `--boundary` | orbit-change (symbol-kind) boundaries |
| `--match QUERY` | equivariant match: every span in the query's orbit |

With no mode flag, `trex orbit` lists each token with its orbit representative.

## Hierarchical grain separation

The register-renaming orbit is the key to separating a coarse grain (functions) from a fine
grain (basic blocks) in a token stream with no markers supplied in advance. A recurring
inter-function prologue is a rename-invariant orbit: the same instruction silhouette repeats
at each function head whatever registers it uses. Texture *discovers* that a marker exists
at coarse boundaries; symmetry *completes* the grain by matching every occurrence of the
marker's orbit. `trex seam --grain` scores the four strategies against known grain:

```console
$ trex seam --grain --words 60
trex seam --grain: 1539 instructions, 60 functions, 210 blocks (synthetic)
  flat single-threshold  (function): P=0.198 R=0.900 F1=0.324  <- the conflation
  multi-scale coarse tier(function): P=0.417 R=0.250 F1=0.312
  multi-scale fine tier  (block)   : P=0.430 R=0.367 F1=0.396
  prologue-orbit         (function): P=1.000 R=1.000 F1=1.000
  UNIFIED  discovered coarse marker [8, 0, 1] (the recurring inter-function transition):
    function (texture discovers, marker completes, +/-1): P=1.000 R=0.983 F1=0.992
    block    (function anchors + texture)  : P=0.472 R=0.433 F1=0.452
```

The prologue orbit recovers function grain exactly where a flat predictability threshold
conflates the two grains into one. The predictive-compression counterpart lives in
`trex seam --compress` (see [SEAM.md](../seam/) and the [CLI reference](../cli/#seam)).

## Design decisions

| ID | Decision | Choice | Why |
|---|---|---|---|
| O1 | what a token is | the canonical representative of its orbit under `G` | the symmetry axis, orthogonal to frequency and predictability |
| O2 | the group ladder | identity / case / notation / shape | grounded rungs that reuse `canon` |
| O3 | shape orbit | within-class (C / V / D) substitution | the formal version of word shape; generalises to unseen forms |
| O4 | function grain | the prologue orbit (rename-invariant) | a recurring symmetric marker, complete where texture is not |
| O5 | block grain | multi-scale persistence, fine tier | texture is the only signal an inner grain has |
| O6 | marker discovery | mode class-pattern at coarse texture boundaries | unsupervised: texture finds what symmetry then completes |
| O7 | scale matching | time constants set near the grain sizes | a coarse scale far above grain size localises nothing |

## Cross-system applicability

The symmetry axis is portable wherever a stream carries equivalence structure a frequency or
predictability tokenizer ignores:

- **text**: case, notation, and morphology orbits collapse surface variants to one identity
  and generalise to unseen forms;
- **code**: the register-renaming orbit recovers function grain and folds renamed blocks to
  one token;
- **any nested-grain stream**: the unified bootstrap (texture discovers a marker, symmetry
  completes the grain) separates a coarse grain from a fine one with no marker supplied in
  advance.
