---
title: Flow
linkTitle: Flow
weight: 70
---

# The flow axis

The trend of a per-token signal: its windowed slope, its direction, how long it has held that
direction, and where it reverses. The signal is the magnitude by default, the stress depth or the
token length on request, or any per-token value a caller computes.

Source: [`src/flow.rs`](https://github.com/Variably-Constant/trex/blob/main/src/flow.rs).

## The readings

| Reading | Meaning |
|---|---|
| slope | the average per-step change over the trailing window |
| direction | `+1` rising, `0` steady, `-1` falling, with a steady band around zero |
| momentum | consecutive tokens holding the same non-steady direction |

| Event | Meaning |
|---|---|
| reversal | the trend flips between rising and falling; steady does not count |

| Signal | `--over` | Source |
|---|---|---|
| `Magnitude` | `magnitude` | the [magnitude](../magnitude/) of each token |
| `StressDepth` | `stress` | the [stress](../stress/) depth of each token |
| `Length` | `length` | each token's byte length |

`analyze_signal(tokens, signal, cfg)` takes any precomputed per-token `f32`.

## Reading the axis

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow --text '1 10 100 1000 50 5'
trex flow (over magnitude): 18 bytes, 6 tokens, 1 reversal(s)
  peak momentum: 4 (rising)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::flow::analyze_bytes(b"1 10 100 1000 50 5", trex::flow::Signal::Magnitude);
assert_eq!((field.n_tokens, field.reversals.len()), (6, 1));
assert_eq!(field.frames.iter().map(|f| f.momentum).max(), Some(4));
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexFlow '1 10 100 1000 50 5' | Select-Object Tokens, Signal, PeakMomentum, Reversals | Format-List

Tokens       : 6
Signal       : Magnitude
PeakMomentum : 50 at 14, Rising
Reversals    : {5 at 17, Falling}
```
{{< /tab >}}
{{< /tabs >}}

`--over magnitude|stress|length` picks the signal, `--field` prints the per-token slope,
direction and momentum, and `--reversals` the turning points. In PowerShell `-Signal` picks the
signal and `-Detail` adds a frame at every token.

## Data model

`FlowFrame`, one token's reading:

| Field | Type | Meaning |
|---|---|---|
| `slope` | `f32` | windowed rate of change |
| `direction` | `i8` | `-1`, `0` or `+1` |
| `momentum` | `u16` | consecutive same-direction tokens |

`FlowField`, keyed by byte offset:

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | token count |
| `spans` | `Vec<(usize, usize)>` | byte span per token |
| `frames` | `Vec<FlowFrame>` | one per token |
| `reversals` | `Vec<usize>` | turning-point byte offsets |

Query API: `slope_at(byte)`, `direction_at(byte)`.

## Algorithm

Per token: `slope = (signal[i] - signal[i - window]) / window`, the average trend over the
trailing window; `direction` thresholds the slope against a steady band; `momentum` is the run
length of the same non-steady direction; a reversal is recorded where the direction flips
between `+1` and `-1`. One pass, O(1) per token; the signal costs one pass of the axis it comes
from.
