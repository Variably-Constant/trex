---
title: Magnitude
linkTitle: Magnitude
weight: 50
---

# The magnitude axis

The scale substrate: *how much / how big / how intense*. trex's other axes answer where to
cut ([BPE](../cli/#bpe), [seam](../seam/)), what is here ([spectral](../spectral/),
[shape](../shape/)), and what is the same ([orbit](../orbit/)). None read the scale of a value.
A `Number` token is just a span - [`src/token.rs`](https://github.com/Variably-Constant/trex/blob/main/src/token.rs) carries no parsed value -
so `5` and `5000000000` are identical to the lexer, to shape, to spectral, and to orbit. The
magnitude axis reads the one thing they all drop.

Source: [`src/magnitude.rs`](https://github.com/Variably-Constant/trex/blob/main/src/magnitude.rs) · CLI: [`trex magnitude`](../cli/#magnitude).

## Contents

- [Thesis](#thesis)
- [Signal no other axis reaches](#signal-no-other-axis-reaches)
- [The calculus chain](#the-calculus-chain)
- [Data model](#data-model)
- [Algorithm](#algorithm)
- [CLI](#cli)
- [Physics anchors](#physics-anchors)
- [Taxonomy](#taxonomy)
- [Design decisions](#design-decisions)

## Thesis

Magnitude is an intensive scalar field `m(t)` over the token stream: the order of magnitude
of each token's natural scalar (a Number's numeric value, any other token's byte size). Two
physical quantities derive from it by calculus, exactly as energy and momentum derive from a
field: **energy** `E = sum(m^2)` over a window (the extensive accumulated intensity, locating
heavy regions) and **gradient** `dm/dt = m[i] - m[i-1]` (the directional flow).

## Signal no other axis reaches

```text
timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080
```

Every value here is a `Number`. Shape reads one silhouette for all four; spectral one digit
texture; orbit folds none together. The huge one is invisible to all three. Magnitude reads
the four values as `m = 1.48, 0.70, 9.70, 3.91`: `max_bytes` is a scale outlier, six orders
above its neighbours. The same signal finds an address among indices, a magic constant among
small literals, a size field among flags, a price among quantities - any value whose *scale*
marks it.

## The calculus chain

| Quantity | Definition | Physics | Reads |
|---|---|---|---|
| **magnitude** `m(t)` | order of magnitude of the token scalar | intensive scalar field | the scale of each token |
| **gradient** `dm/dt` | `m[i] - m[i-1]` | derivative (flow) | direction and rate of scale change |
| **energy** `E` | `sum(m^2)` over a window; `total_energy` over all | extensive integral | where scale concentrates |

Build the field once and energy and gradient are reads of it: the same
position -> velocity -> acceleration relationship, for "how much" instead of "where".

## Data model

### `MagnitudeFrame` - one token's reading

| Field | Type | Meaning |
|---|---|---|
| `magnitude` | `f32` | `m(t)`: a Number's `log10(\|value\|)`, any other token's `log2(byte_len)` |
| `gradient` | `f32` | `dm/dt`: change from the previous token |
| `energy` | `f32` | local `sum(m^2)` over the trailing `energy_window` |

### `MagnitudeField` - the side table (keyed by byte offset)

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | token count |
| `spans` | `Vec<(usize, usize)>` | byte span per token |
| `frames` | `Vec<MagnitudeFrame>` | one per token |
| `jumps` | `Vec<usize>` | scale-discontinuity byte offsets (`\|gradient\|` past threshold) |
| `total_energy` | `f32` | `sum(m^2)` over the whole stream |

Query API: `magnitude_at(byte)`, `gradient_at(byte)`, `energy_of(start, end)`, `outliers()`
(token indices more than `outlier_sigma` standard deviations from the stream mean).

## Algorithm

One pass over the token stream (`analyze`), all linear-with-constant:

1. **magnitude** - a Number's value parsed (decimal / hex / scientific, underscores stripped)
   to `log10(|value|)`, else `log2(byte_len)`.
2. **statistics** - stream mean / std of `m` and `total_energy = sum(m^2)` in the same scan.
3. **gradient** - `m[i] - m[i-1]`, first token `0`.
4. **jump** - `|gradient| >= jump_threshold` (default 3 orders) records a discontinuity.
5. **energy** - local `sum(m^2)` over the trailing `energy_window`.

## CLI

```console
$ trex magnitude --text 'retries = 3 ; max_bytes = 5000000000'
trex magnitude: 36 bytes, 7 tokens, total energy 112.2, 3 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
```

| Flag | Effect |
|---|---|
| `--field` | per-token magnitude / gradient / energy table |
| `--jumps` | scale discontinuities (`\|gradient\|` past threshold) |
| `--energy` | the heaviest points by local energy |
| `--outliers` | tokens whose magnitude is a stream outlier |

## Physics anchors

- **Intensive vs extensive** (thermodynamics). Magnitude is intensive (per-token, like
  temperature); energy is extensive (additive, scales with span size). Spectral entropy and
  shape period are intensive readings; `total_energy` is the first extensive one.
- **Noether duality**. Every continuous symmetry has a conserved quantity
  (time-symmetry <-> energy). The orbit axis is the symmetry axis; an energy is its dual
  conserved quantity.

## Taxonomy

| Family | Question | Axes |
|---|---|---|
| boundary | where to cut? | BPE (frequency), seam (predictability) |
| character | what is here? | spectral (temporal), shape (structural) |
| identity | what is the same? | orbit (symmetry) |
| **scale** | **how much?** | **magnitude (intensive), energy (extensive), gradient (flow)** |

Magnitude composes: an outlier inside a shape `Table` region is a record's anomalous cell;
the heavy regions are where a spectral data run carries large values.

## Cost model

| Kernel | Per-token cost | Bounded by |
|---|---|---|
| magnitude | O(1) | numeric parse of the token text |
| statistics | O(1) | one accumulation pass |
| gradient | O(1) | fixed |
| energy | O(1) amortised | `energy_window` |

## Design decisions

| ID | Decision | Choice | Why |
|---|---|---|---|
| M1 | what magnitude is | order of magnitude of the token scalar (value for Numbers, byte size else) | the intensive scale every other axis discards; log scale keeps values comparable |
| M2 | the stack | energy = `sum(m^2)`, gradient = `dm/dt` derive from `m` | one field, three physical quantities by calculus |
| M3 | jumps | `\|gradient\|` past a threshold | a scale discontinuity no kind / shape / texture axis can see |
| M4 | outliers | standard-deviation-from-mean of the magnitude field | scale anomalies surfaced statistically |
| M5 | home | `src/magnitude.rs` + `trex magnitude`, byte-span-keyed | composes with the other axes at the same byte offsets |
