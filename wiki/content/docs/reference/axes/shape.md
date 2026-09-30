---
title: Shape
linkTitle: Shape
weight: 30
---

# The shape axis

The structural substrate. Where the [spectral axis](../spectral/) reads the *temporal*
character of the stream, the shape axis reads its *structural form*, abstracted from the
specific bytes. Two spans have the same shape when they share a silhouette - the same
sequence of token-classes and word-shapes - regardless of content. `foo(a, b)` and
`bar(x, y)` are both `W ( W , W )`; `1,22,3` and `444,5,66` are both `N , N , N`.

Source: [`src/shape.rs`](https://github.com/Variably-Constant/trex/blob/main/src/shape.rs) · CLI: [`trex shape`](../cli/#shape).

## Contents

- [Thesis](#thesis)
- [Signal no other axis reaches](#signal-no-other-axis-reaches)
- [Scale-free silhouette ladder](#scale-free-silhouette-ladder)
- [Data model](#data-model)
- [Algorithms](#algorithms)
- [Region fusion](#region-fusion)
- [CLI](#cli)
- [Cost model](#cost-model)
- [Duality with the spectral axis](#duality-with-the-spectral-axis)
- [Composing over an orbit quotient](#composing-over-an-orbit-quotient)
- [Design decisions](#design-decisions)

## Thesis

A silhouette *is* a grammar production, so a shape token is a grammar-free grammar: the unit
a grammar would name (a call, a row, a field block), recovered from the stream's own
repetition with no grammar and no dictionary. It is the structural dual of the spectral
token.

## Signal no other axis reaches

The spectral period autocorrelates *bytes*, so it finds fixed-width structure. A ragged
table defeats it:

```text
1,22,3
444,5,66
7,888,9
```

The field widths vary, so the comma byte-offsets shift every row and the byte
autocorrelation finds nothing. But the shape is rock-solid: every row is the silhouette
`N , N , N` (period 6 over tokens). Shape autocorrelates *silhouettes*, where width
variation has already collapsed (a field is one `Number` token whatever its width). The
same signal recovers struct field blocks, repeated code idioms, and key/value records by
their silhouette's repetition.

## Scale-free silhouette ladder

| Grain | Silhouette unit |
|---|---|
| byte | character-class (digit / alpha / space / punct / bracket) |
| token | token-class + word-shape (`W:snake`, `N`, `P:,`, `(`, `)`) |
| region | the token-class sequence - the structural template |

The shipped reader operates at the token grain and keys every frame by the token's byte
span, so the byte, spectral, and pattern layers all query the field by byte offset.

## Data model

### `ShapeClass` - one token's silhouette as a comparable code

A `u32` packing the token's structural identity: the `TokenKind` code in the high bits,
plus - for a `Word` - the [`tokutil::shape`](https://github.com/Variably-Constant/trex/blob/main/src/tokutil.rs) class (Pascal / snake /
camel / SCREAM / short / word), and - for a `Punct` - the glyph byte. Two tokens with the
same `ShapeClass` are the same shape; the packing is `Eq`, so the autocorrelation compares
classes by equality.

### `ShapeFrame` - one token's reading

| Field | Type | Meaning |
|---|---|---|
| `class` | `u32` | the token's `ShapeClass` code |
| `period` | `u16` | dominant shape-period of the neighbourhood in tokens (`0` = none) |
| `period_strength` | `f32` | normalised silhouette-autocorrelation peak `[0,1]` |
| `novelty` | `f32` | shape n-gram surprise `[0,1]` (`1` = first sighting of this template) |

### `ShapeField` - the side table (keyed by byte offset)

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | token count |
| `spans` | `Vec<(usize, usize)>` | byte span per token (the byte-offset key) |
| `frames` | `Vec<ShapeFrame>` | one per token |
| `boundaries` | `Vec<usize>` | shape-change-point byte offsets (silhouette-break cuts) |

### Query API

| Method | Returns |
|---|---|
| `class_at(byte)` | the shape-class of the token covering `byte` |
| `period_at(byte)` | the dominant shape-period and strength at `byte` |
| `in_template(byte)` | whether `byte` is inside a strong shape-period run |
| `shape_regions()` | the periodic / template byte spans and their period |

## Algorithms

One pass over the token stream (`analyze`).

- **Silhouette** - fold `(TokenKind, word-shape | punct-glyph)` into a `u32` `ShapeClass`.
  O(1)/token.
- **Shape periodicity** - centred autocorrelation of the shape-class sequence at token lags
  `1..max_lag`; the dominant period is the arg-max with prominence. The same
  Wiener-Khinchin kernel as the spectral period, but over shape-class codes rather than
  bytes, so width-varying rows still align. Recomputed every `period_hop` tokens under a
  fixed op budget.
- **Shape novelty** - a rolling hash of the last `k` shape-classes with a count map;
  `novelty = 1/(1 + count_before_increment)`. A first-sighting silhouette scores `1`, a
  repeated row near `0`.
- **Shape change-point** - a boundary where the local shape n-gram stops matching the
  running template (a novelty spike past an adaptive threshold), at least `cp_min_gap`
  tokens since the last cut.

## Region fusion

`shape::classified_regions(input)` fuses the spectral region texture with the shape period:
each `spectral::code_regions` span that overlaps a strong shape template becomes
`RegionKind::Table(period)`; the rest keep their spectral texture.

```rust
pub enum RegionKind { Table(u16), Blob, Prose, Numeric, Code, Mixed }
```

`shape` calls `spectral` here (token grain over byte grain, never the reverse). The
`trex spectral --code-classify` surface renders it, so a tabular block reads as one `table`
region rather than a texture-shredded `data` run.

## CLI

```console
$ trex shape --text 'foo(a, b) bar(c, d) baz(e, f)'
trex shape: 29 bytes, 18 tokens, 1 template region(s), 2 change-point(s)
  dominant shape-period: 6 tokens  (strength 1.00)
```

| Flag | Effect |
|---|---|
| `--classes` | per-token shape-class + period + novelty table |
| `--period` | the dominant shape-period and template regions |
| `--segment` | shape-change-point boundaries |
| `--orbit G` | measure over an orbit quotient (`identity` / `case` / `notation` / `shape`) |
| `--json` | machine-readable field |

## Cost model

| Kernel | Per-token cost | Bounded by |
|---|---|---|
| silhouette | O(1) | fixed |
| novelty | O(1) amortised | `novelty_window` |
| periodicity | amortised O(1) | adaptive `period_hop` under a fixed op budget |
| change-point | O(1) | fixed |

All linear-with-constant; periodicity is held linear by the adaptive hop and the `max_lag`
cap, mirroring the spectral cost model.

## Duality with the spectral axis

| | Spectral token | Shape token |
|---|---|---|
| Question | what temporal character? | what structural form? |
| Domain | continuous (band energy, entropy) | discrete (silhouette classes) |
| Dual | texture <-> time (a band is a clock) | silhouette <-> grammar (a silhouette is a production) |
| Period | byte autocorrelation (fixed-width) | shape autocorrelation (width-free) |
| Segments where | the temporal signature changes | the silhouette breaks |
| Grain | byte | token |

They compose: spectral texture + shape period = a named structure with neither a delimiter
nor a grammar. That fusion is what `classified_regions` produces.

## Composing over an orbit quotient

The silhouette is itself a quotient - "ignore the specific bytes, keep the token-kind +
case-shape." That is one orbit (the symmetry axis, [ORBIT.md](../orbit/)). Making the
quotient explicit lets the same period / novelty / change-point measurement run over any
rung of the orbit ladder instead of the hardcoded word-shape:

- `shape::analyze_over(tokens, bytes, group)` (and `analyze_bytes_over`, `analyze_over_with`)
  key the silhouette on `orbit::canonical(.., group)`, sharing the `build_field` core with
  `analyze` so the default reader is byte-identical.
- `Identity` = the literal token; `Case` = case folded before measuring; `Notation` =
  notation folded (`theta` = `\theta`); `Shape` = the consonant/vowel/digit pattern.

Orbit is the pre-transform; shape is the measurement that composes over it.

```console
$ trex shape --orbit identity --period --text 'the cat sat THE CAT SAT the cat sat THE CAT SAT'
trex shape: 47 bytes, 12 tokens, 1 template region(s), 2 change-point(s)
  dominant shape-period: 6 tokens  (strength 1.00)
  template regions (byte span -> shape-period):
    [      44..47      ] period 6    SAT

$ trex shape --orbit case --period --text 'the cat sat THE CAT SAT the cat sat THE CAT SAT'
trex shape: 47 bytes, 12 tokens, 1 template region(s), 1 change-point(s)
  dominant shape-period: 3 tokens  (strength 1.00)
  template regions (byte span -> shape-period):
    [      32..47      ] period 3    sat THE CAT SAT
```

The identity orbit reads period 6 (the literal tokens repeat every six, the case cycle);
the case orbit reads period 3 (the true phrase "the cat sat", case folded away) - the
period the byte-period and the default silhouette both miss.

## Design decisions

| ID | Decision | Choice | Why |
|---|---|---|---|
| SH1 | grain | token-primary, byte-span-keyed | structural period lives over tokens; byte keys compose with spectral |
| SH2 | silhouette | `TokenKind` + `tokutil::shape` packed to a `u32` `ShapeClass` | `Eq` codes drive the autocorrelation |
| SH3 | period | silhouette autocorrelation (reuse the spectral kernel) | a width-free structural period the byte period cannot reach |
| SH4 | cost | fuse silhouette + novelty + change-point into the token pass; bounded adaptive-hop autocorrelation | linear with a small constant |
| SH5 | region fusion | `classified_regions` = `spectral::code_regions` x `shape_regions`; `shape` calls `spectral` | a tabular block is named `Table` where the byte-period reads only "data"; layer direction is one-way |
| SH6 | orbit composition | the silhouette is keyed on `orbit::canonical(.., group)`; `analyze_over` shares `build_field` with `analyze` | shape measures over a chosen symmetry quotient, not only the built-in word-shape |
