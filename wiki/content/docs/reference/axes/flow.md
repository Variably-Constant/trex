---
title: Flow
linkTitle: Flow
weight: 70
---

# The flow axis

The dynamics substrate: which way the stream is heading and how fast. Every other axis reads
a *state* at each token - its scale ([magnitude](../magnitude/)), its structural load
([stress](../stress/)), its texture ([spectral](../spectral/)), its form ([shape](../shape/)).
Flow reads the *dynamics*: the windowed slope, direction, momentum, and reversal structure of
any scalar signal.

That makes flow a **meta axis**. `analyze_signal` takes a precomputed per-token scalar, so the
same dynamics machinery composes over the magnitude field, the stress depth, or the raw token
length - the flow of any axis.

Source: [`src/flow.rs`](https://github.com/Variably-Constant/trex/blob/main/src/flow.rs) · CLI: [`trex flow`](../cli/#flow).

## Contents

- [The readings and the event](#the-readings-and-the-event)
- [The composable signal](#the-composable-signal)
- [Data model](#data-model)
- [Algorithm](#algorithm)
- [CLI](#cli)
- [Physics anchor and taxonomy](#physics-anchor-and-taxonomy)
- [Design decisions](#design-decisions)

## The readings and the event

| Reading | Meaning |
|---|---|
| **slope** | windowed rate of change: average per-step change over the trailing window |
| **direction** | `+1` rising / `0` steady / `-1` falling (steady band around zero) |
| **momentum** | trend persistence: consecutive tokens holding the same non-steady direction |

| Event | Meaning |
|---|---|
| **reversal** | the trend flips between rising and falling (a turning point; steady does not count) |

## The composable signal

`Signal` selects which per-token scalar the flow reads:

| `Signal` | `--over` | Source | Reads the dynamics of |
|---|---|---|---|
| `Magnitude` | `magnitude` | `magnitude::analyze` | value scale (a ramp of growing numbers) |
| `StressDepth` | `stress` | `stress::analyze` | nesting depth (a structure deepening / releasing) |
| `Length` | `length` | token byte-length | token size (self-contained) |

`analyze_signal(tokens, signal, cfg)` is the open door: any axis that emits a per-token `f32`
composes, so flow is not bound to these three.

## Data model

### `FlowFrame` - one token's reading

| Field | Type | Meaning |
|---|---|---|
| `slope` | `f32` | windowed rate of change |
| `direction` | `i8` | `-1` / `0` / `+1` |
| `momentum` | `u16` | consecutive same-direction tokens |

### `FlowField` - the side table (keyed by byte offset)

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | token count |
| `spans` | `Vec<(usize, usize)>` | byte span per token |
| `frames` | `Vec<FlowFrame>` | one per token |
| `reversals` | `Vec<usize>` | turning-point byte offsets |

Query API: `slope_at(byte)`, `direction_at(byte)`.

## Algorithm

Per token: `slope = (signal[i] - signal[i - window]) / window` (the average trend over the
trailing window); `direction` thresholds the slope against a steady band; `momentum` is the
run length of the same non-steady direction; a `reversal` is recorded where direction flips
between `+1` and `-1`. One pass, O(1) per token; the signal itself costs one pass of whichever
axis it comes from.

## CLI

```console
$ trex flow --text '1 10 100 1000 50 5'
trex flow (over magnitude): 18 bytes, 6 tokens, 1 reversal(s)
  peak momentum: 4 (rising)
```

| Flag | Effect |
|---|---|
| `--over magnitude\|stress\|length` | the signal to flow over (default `magnitude`) |
| `--field` | per-token slope / direction / momentum |
| `--reversals` | turning points (trend flips) |

## Physics anchor and taxonomy

Flow is momentum / gradient: the directional movement of a quantity. It completes the
calculus picture - magnitude is the field, stress an extensive load over it, and flow the
directional derivative of any of them.

| Family | Question | Axes |
|---|---|---|
| boundary | where to cut? | BPE, seam |
| character | what is here? | spectral (temporal), shape (structural) |
| identity | what is the same? | orbit (symmetry) |
| scale | how much? | magnitude / energy / gradient |
| load | how strained? | stress |
| **dynamics** | **which way, how fast?** | **flow (slope / direction / momentum / reversal)** |

Flow is the only axis that reads the others: a steady magnitude with a rising stress is a
flat value sequence sinking into deeper nesting, two independent flows the single-axis
readings cannot express together.

## Design decisions

| ID | Decision | Choice | Why |
|---|---|---|---|
| F1 | what flow is | windowed slope / direction / momentum of a scalar signal | the dynamics generalisation of one signal's derivative |
| F2 | composability | `analyze_signal` over an arbitrary per-token `f32` | a meta-axis: it reads the flow of magnitude, stress, or any axis that emits a signal |
| F3 | momentum | run length of the same non-steady direction | distinguishes a sustained trend from noise |
| F4 | reversal | a flip between rising and falling | the turning point change-points cannot name (they find shifts, not direction) |
| F5 | home | `src/flow.rs` + `trex flow --over`, byte-span-keyed | composes with every axis at the same byte offsets |
