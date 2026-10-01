---
title: Observation
linkTitle: Observation
weight: 80
---

# The observation axis

The entropy of the byte classes around each byte read from three vantages - the past alone, the
future alone, and both - and how far the past and the future disagree.

Source: [`src/observation.rs`](https://github.com/Variably-Constant/trex/blob/main/src/observation.rs).

## What it reads

```text
let x = 5; let y = 6; let z = 7; <base64 blob>
```

In the code and in the blob the past and the future agree. At the boundary they diverge: the
past-only reading still sees code and the future-only reading already sees the blob, so the
boundary is the contested point. A causal reading, such as the [spectral](../spectral/) texture,
places the change a window late; the disagreement places it where it is.

| Vantage | Window | Reads |
|---|---|---|
| causal | `[t-w, t]` | the past only |
| anti-causal | `(t, t+w]` | the future only |
| centered | `[t-w/2, t+w/2]` | both |

| Reading | Meaning |
|---|---|
| disagreement | `\|causal - anticausal\|` at the byte |

| Event | Meaning |
|---|---|
| contested | a local disagreement maximum past the threshold |

## Reading the axis

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex observe --text 'the old man the boats'
trex observe: 21 bytes, 2 contested point(s)
  peak observer-dependence: 0.35 at byte 15
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::observation::analyze(b"the old man the boats");
assert_eq!(field.contested, [2, 15]);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexObservation 'the old man the boats' | Select-Object Length, Peak, Contested | Format-List

Length    : 21
Peak      : at 15, disagreement 0.34939846
Contested : {at 2, disagreement 0.32912496, at 15, disagreement 0.34939846}
```
{{< /tab >}}
{{< /tabs >}}

`--field` prints the sampled causal, anti-causal, centered and disagreement readings, and
`--contested` the contested points. In PowerShell `-Detail` adds a frame at every byte.

## In a pattern

`@ambiguous`, `@ambiguous:token`, `@ambiguous:byte` and `@ambiguous:super` are zero-width anchors
at a contested point at the named grain: the sequence of token kinds, the bytes, or the sequence
of construct roles.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@ambiguous:byte \W' --text 'the old man the boats'
[0..3] "the"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"@ambiguous:byte \W").expect("valid pattern");
let text = b"the old man the boats";
assert_eq!(trex::scan(&pat, text).iter().map(|s| &text[s.range()]).collect::<Vec<_>>(), [&b"the"[..]]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"@ambiguous:byte \W").scan("the old man the boats")]
['the']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@ambiguous:byte \W' -InputObject 'the old man the boats' -Raw
the
```
{{< /tab >}}
{{< /tabs >}}

## Data model

`ObservationFrame`, one byte's reading:

| Field | Type | Meaning |
|---|---|---|
| `causal` | `f32` | normalized byte-class entropy of the past window |
| `anticausal` | `f32` | the same of the future window |
| `centered` | `f32` | the same of the symmetric window |
| `disagreement` | `f32` | `\|causal - anticausal\|` |

`ObservationField`, keyed by byte offset:

| Field | Type | Meaning |
|---|---|---|
| `len` | `usize` | input length in bytes |
| `frames` | `Vec<ObservationFrame>` | one per byte |
| `contested` | `Vec<usize>` | contested byte offsets |

Query API: `causal_at(byte)`, `disagreement_at(byte)`, `is_contested(byte)`.

## Algorithm

Per byte: the normalized Shannon entropy of the byte-class distribution (digit, alpha, space,
punct, high) over the causal, anti-causal and centered windows; `disagreement = |causal -
anticausal|`. Contested points are a second pass for local disagreement maxima above the
threshold, spaced apart, with the incomplete-window edges skipped, since a causal window has no
past at byte 0. One entropy pass and one contested scan.
