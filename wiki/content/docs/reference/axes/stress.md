---
title: Stress
linkTitle: Stress
weight: 60
---

# The stress axis

The structural load at each token: how many brackets enclose it, how long the innermost has been
open, and how long all of them together.

Source: [`src/stress.rs`](https://github.com/Variably-Constant/trex/blob/main/src/stress.rs).

## The readings

| Reading | Definition |
|---|---|
| depth | the number of brackets enclosing the token |
| strain | the tokens the innermost open bracket has been held open |
| load | the sum of every open bracket's stretch, `sum(now - opened)` |

| Event | Definition |
|---|---|
| peak | a local depth maximum at or above `peak_min_depth` |
| fracture | the start of a release cascade from a level of at least `fracture_min_depth` |

A flat stream carries no stress; a deeply nested one builds load to a peak and releases it at a
fracture.

## Reading the axis

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex stress --text 'f(g(h(x)))'
trex stress: 10 bytes, 10 tokens, max depth 3, 2 peak(s), 1 fracture(s)
  peak load: 10 (depth 2, strain 4)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::stress::analyze_bytes(b"f(g(h(x)))");
assert_eq!((field.n_tokens, field.max_depth), (10, 3));
assert_eq!((field.peaks.len(), field.fractures.len()), (2, 1));
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexStress 'f(g(h(x)))' | Select-Object Tokens, MaxDepth, PeakLoad, Peaks, Fractures | Format-List

Tokens    : 10
MaxDepth  : 3
PeakLoad  : ) at 7, depth 2
Peaks     : {h at 4, depth 2, x at 6, depth 3}
Fractures : {) at 7, depth 2}
```
{{< /tab >}}
{{< /tabs >}}

`--field` prints the per-token depth, strain and load, `--peaks` the stress peaks and
`--fractures` the fracture points. In PowerShell each frame carries `Offset`, `Text`, `Depth`,
`Strain` and `Load`, and `-Detail` adds a frame at every token.

## In a pattern

`@nested>k` and `@nested>=k` are zero-width anchors on tokens at least that many brackets deep.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@nested>=3 \W' --text 'f(g(h(x)))'
[6..7] "x"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"@nested>=3 \W").expect("valid pattern");
let text = b"f(g(h(x)))";
assert_eq!(trex::scan(&pat, text).iter().map(|s| &text[s.range()]).collect::<Vec<_>>(), [&b"x"[..]]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"@nested>=3 \W").scan("f(g(h(x)))")]
['x']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@nested>=3 \W' -InputObject 'f(g(h(x)))' -Raw
x
```
{{< /tab >}}
{{< /tabs >}}

## Data model

`StressFrame`, one token's reading:

| Field | Type | Meaning |
|---|---|---|
| `depth` | `u16` | enclosing-bracket nesting depth |
| `strain` | `f32` | stretch of the innermost open bracket |
| `load` | `f32` | total outstanding tension, the sum of the open-bracket stretches |

`StressField`, keyed by byte offset:

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | token count |
| `spans` | `Vec<(usize, usize)>` | byte span per token |
| `frames` | `Vec<StressFrame>` | one per token |
| `peaks` | `Vec<usize>` | stress-peak byte offsets |
| `fractures` | `Vec<usize>` | fracture byte offsets |
| `max_depth` | `u16` | deepest nesting reached |

Query API: `depth_at(byte)`, `load_at(byte)`.

## Algorithm

The open-bracket stack of token indices is kept from `TokenKind::Open` and `Close`. At each
token a close pops first, so it reads the outer depth and is the release point; `depth =
stack.len()`; `strain = now - innermost_open`; `load = sum(now - open)` over the stack. A
fracture is the first decreasing-depth token of a cascade whose prior depth reached
`fracture_min_depth`. Peaks are a second linear pass over the depth sequence, local maxima at or
above `peak_min_depth`. One pass and one peak scan; O(depth) per token for load.
