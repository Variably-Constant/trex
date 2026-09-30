---
title: Observation
linkTitle: Observation
weight: 80
---

# The observation axis

The vantage substrate. Every reader has a vantage in time: [spectral](../spectral/) reads the
stream as a single causal forward pass (the state at `t` is the decayed past),
[seam](../seam/) reads bidirectionally. Neither makes the vantage itself a parameter, nor
measures how much the reading depends on it. The observation axis does both: it reads the
same temporal signal (local byte-class entropy) from three vantages and measures the
disagreement between them.

Source: [`src/observation.rs`](https://github.com/Variably-Constant/trex/blob/main/src/observation.rs) · CLI: [`trex observe`](../cli/#observe).

## Contents

- [Thesis](#thesis)
- [Signal no other axis reaches](#signal-no-other-axis-reaches)
- [The three vantages and the event](#the-three-vantages-and-the-event)
- [Data model](#data-model)
- [Algorithm](#algorithm)
- [CLI](#cli)
- [Physics anchor and taxonomy](#physics-anchor-and-taxonomy)
- [Design decisions](#design-decisions)

## Thesis

The observation axis reads local byte-class entropy from three vantages - causal (past-only
window), anti-causal (future-only window), and centered (symmetric window) - and the
disagreement between the causal and anti-causal readings is the observer-dependence: how much
the future disambiguates the past at this position.

## Signal no other axis reaches

The disagreement is the bleed / transition / garden-path signal. A causal reader is committed
to the past and cannot yet see a change a centered reader would, so it smears a boundary
forward. Where past and future disagree most is exactly where observation matters.

```text
let x = 5; let y = 6; let z = 7; <base64 blob>
```

Every byte is read three ways. In the homogeneous code and the homogeneous blob, past and
future agree (low disagreement). At the code -> blob boundary they diverge - the causal
vantage still sees code, the anti-causal already sees the blob - so the boundary is the
contested point. The byte-period and the causal texture both smear it; observation pins it.

## The three vantages and the event

| Vantage | Window | Reads |
|---|---|---|
| **causal** | `[t-w, t]` | the past only (what an online reader knows) |
| **anti-causal** | `(t, t+w]` | the future only (what a reverse reader knows) |
| **centered** | `[t-w/2, t+w/2]` | both (the offline, lag-free reading) |

| Reading | Meaning |
|---|---|
| **disagreement** | `\|causal - anticausal\|`: the observer-dependence at the byte |

| Event | Meaning |
|---|---|
| **contested** | a local disagreement maximum past the threshold - where observation matters most |

## Data model

### `ObservationFrame` - one byte's reading

| Field | Type | Meaning |
|---|---|---|
| `causal` | `f32` | normalised byte-class entropy of the past window |
| `anticausal` | `f32` | ... of the future window |
| `centered` | `f32` | ... of the symmetric window |
| `disagreement` | `f32` | `\|causal - anticausal\|` |

### `ObservationField` - the side table (keyed by byte offset)

| Field | Type | Meaning |
|---|---|---|
| `len` | `usize` | input length in bytes |
| `frames` | `Vec<ObservationFrame>` | one per byte |
| `contested` | `Vec<usize>` | contested byte offsets (high observer-dependence) |

Query API: `causal_at(byte)`, `disagreement_at(byte)`, `is_contested(byte)`.

## Algorithm

Per byte: normalised Shannon entropy of the byte-class distribution (digit / alpha / space /
punct / high) over the causal, anti-causal, and centered windows;
`disagreement = |causal - anticausal|`. Contested points are a second pass for local
disagreement maxima above the threshold, spaced apart, with the incomplete-window edges
skipped (a causal observer genuinely has no past at byte 0, so that edge disagreement is an
artifact, not a transition). One entropy pass plus one contested scan.

## CLI

```console
$ trex observe --text 'the old man the boats'
trex observe: 21 bytes, 2 contested point(s)
  peak observer-dependence: 0.35 at byte 15
```

| Flag | Effect |
|---|---|
| `--field` | sampled causal / anticausal / centered / disagreement |
| `--contested` | the contested points (where past and future disagree most) |

## Physics anchor and taxonomy

Observation is the measurement-vantage parameter: when and from which temporal direction the
system is read. The causal vantage is the online observer (zero latency, smears boundaries);
the centered vantage is the offline observer (perfect, needs the whole signal); the
disagreement is the cost of observing online - the information the future holds that the
present cannot yet see.

| Family | Question | Axes |
|---|---|---|
| boundary | where to cut? | BPE, seam |
| character | what is here? | spectral (temporal), shape (structural) |
| identity | what is the same? | orbit (symmetry) |
| scale | how much? | magnitude / energy / gradient |
| load | how strained? | stress |
| dynamics | which way, how fast? | flow |
| **vantage** | **how observer-dependent?** | **observation (causal / anti-causal / centered / contested)** |

It does not read a new property of the stream but the dependence of any temporal reading on
the observer's position in time. The contested points are where a causal reader needs a
centered or marker-based override.

## Design decisions

| ID | Decision | Choice | Why |
|---|---|---|---|
| O1 | what observation is | the vantage (causal / anti-causal / centered) a signal is read from | the temporal parameter spectral and seam each fix; here it is explicit |
| O2 | the signal | local byte-class entropy | the spectral substrate's own quantity, read three ways |
| O3 | observer-dependence | `\|causal - anticausal\|` | how much the future disambiguates the past |
| O4 | contested | disagreement maxima, edges skipped | the incomplete-window edges disagree artifactually, not as transitions |
| O5 | home | `src/observation.rs` + `trex observe`, byte-offset-keyed | the temporal family, composes at the same offsets |
