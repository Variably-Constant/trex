---
title: Stress
linkTitle: Stress
weight: 60
---

# The stress axis

The structural-load substrate. Material science reads a body by the internal forces it holds
under load: stress (force concentration), strain (deformation), fracture (sudden release). A
token stream holds an analogous load - the unresolved context at each point: how deeply
nested it is, how long its open structures have been held, how much total tension is
outstanding. The open-bracket stack at each token *is* the tension held there.

Source: [`src/stress.rs`](https://github.com/Variably-Constant/trex/blob/main/src/stress.rs) · CLI: [`trex stress`](../cli/#stress).

## Contents

- [The three readings and two events](#the-three-readings-and-two-events)
- [Data model](#data-model)
- [Algorithm](#algorithm)
- [CLI](#cli)
- [Physics anchor](#physics-anchor)
- [Taxonomy](#taxonomy)
- [Design decisions](#design-decisions)

## The three readings and two events

| Reading | Material analogue | Definition |
|---|---|---|
| **depth** | stress (force concentration) | number of brackets enclosing the token |
| **strain** | deformation | tokens the innermost open bracket has been held |
| **load** | total tension | sum of every open bracket's stretch (`sum(now - opened)`) |

| Event | Material analogue | Definition |
|---|---|---|
| **peak** | stress concentration | a local depth maximum at or above `peak_min_depth` |
| **fracture** | sudden release | the start of a release cascade from a level `>= fracture_min_depth` |

A flat stream carries zero stress; a deeply nested one builds load to a peak then releases it
at a fracture.

## Data model

### `StressFrame` - one token's reading

| Field | Type | Meaning |
|---|---|---|
| `depth` | `u16` | enclosing-bracket nesting depth |
| `strain` | `f32` | stretch of the innermost open bracket |
| `load` | `f32` | total outstanding tension (sum of open-bracket stretches) |

### `StressField` - the side table (keyed by byte offset)

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | token count |
| `spans` | `Vec<(usize, usize)>` | byte span per token |
| `frames` | `Vec<StressFrame>` | one per token |
| `peaks` | `Vec<usize>` | stress-peak byte offsets (most-loaded points) |
| `fractures` | `Vec<usize>` | fracture byte offsets (deep releases) |
| `max_depth` | `u16` | deepest nesting reached |

Query API: `depth_at(byte)`, `load_at(byte)`.

## Algorithm

The open-bracket stack (token indices) is maintained directly from `TokenKind::Open` /
`Close`. At each token a close pops first (it reads the outer depth and is the release
point); `depth = stack.len()`; `strain = now - innermost_open`; `load = sum(now - open)` over
the stack. A fracture is the first decreasing-depth token of a cascade whose prior depth
cleared `fracture_min_depth`. Peaks are a second linear pass over the depth sequence (local
maxima at or above `peak_min_depth`). One pass plus one peak scan; O(depth) per token for
load.

## CLI

```console
$ trex stress --text 'f(g(h(x)))'
trex stress: 10 bytes, 10 tokens, max depth 3, 2 peak(s), 1 fracture(s)
  peak load: 10 (depth 2, strain 4)
```

| Flag | Effect |
|---|---|
| `--field` | per-token depth / strain / load table |
| `--peaks` | stress peaks (most-loaded points) |
| `--fractures` | fracture points (deep releases) |

## Physics anchor

Stress / strain / fracture is the material-mechanics triple. The stream mirror is exact at
the structural level: nesting concentrates load (stress), a long-held span deforms (strain),
and a deep structure closing all at once releases it (fracture). It is the extensive
structural counterpart to [magnitude](../magnitude/)'s scalar load - where magnitude weighs a
value, stress weighs the context held around it.

## Taxonomy

| Family | Question | Axes |
|---|---|---|
| boundary | where to cut? | BPE, seam |
| character | what is here? | spectral (temporal), shape (structural) |
| identity | what is the same? | orbit (symmetry) |
| scale | how much? | magnitude / energy / gradient |
| **load** | **how strained?** | **stress (depth / strain / load / fracture)** |

Orthogonal to all: a `5000000000` magnitude outlier may sit at depth 0 (no stress) or deep
inside a call (high stress); the two readings are independent. The most-loaded points are
where a shape template is most deeply embedded.

## Design decisions

| ID | Decision | Choice | Why |
|---|---|---|---|
| S1 | what stress is | the open-bracket stack at each token | the structural load the parser already carries; no new state |
| S2 | strain | innermost-bracket stretch (tokens held) | a long-held span is more deformed than a short one |
| S3 | load | sum of every open bracket's stretch | total outstanding tension, extensive over depth |
| S4 | fracture | first decreasing token of a deep cascade | one fracture per deep structure, not one per close level |
| S5 | home | `src/stress.rs` + `trex stress`, byte-span-keyed | composes with the other axes at the same byte offsets |
