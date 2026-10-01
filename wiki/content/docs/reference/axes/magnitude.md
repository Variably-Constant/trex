---
title: Magnitude
linkTitle: Magnitude
weight: 50
---

# The magnitude axis

The order of magnitude of each token: a number's value, any other token's byte length, on a
log scale, with the change from the token before and the energy of the window behind it.

Source: [`src/magnitude.rs`](https://github.com/Variably-Constant/trex/blob/main/src/magnitude.rs).

## What it reads that nothing else does

```text
timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080
```

Every value here is a number. Shape reads one silhouette for all four, spectral one digit
texture, and orbit folds none together. Magnitude reads them as `m = 1.48, 0.70, 9.70, 3.91`:
`max_bytes` is six orders above its neighbours. The same reading finds an address among indices,
a magic constant among small literals, a size field among flags, a price among quantities.

## The readings

| Quantity | Definition | Reads |
|---|---|---|
| magnitude `m(t)` | a number's `log10(\|value\|)`, any other token's `log2(byte_len)` | the scale of each token |
| gradient | `m[i] - m[i-1]` | the direction and rate of scale change |
| energy | `sum(m^2)` over a trailing window; `total_energy` over the whole stream | where scale concentrates |

A jump is a token whose gradient reaches the jump threshold, three orders by default; an outlier
is a token more than `outlier_sigma` standard deviations from the stream's mean magnitude.

## Reading the axis

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --text 'retries = 3 ; max_bytes = 5000000000'
trex magnitude: 36 bytes, 7 tokens, total energy 112.2, 3 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::magnitude::analyze_bytes(b"retries = 3 ; max_bytes = 5000000000");
let peak = field.frames.iter().map(|f| f.magnitude).fold(0.0f32, f32::max);
assert!((peak - 9.70).abs() < 0.01);
assert_eq!((field.n_tokens, field.jumps.len()), (7, 3));
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexMagnitude 'retries = 3 ; max_bytes = 5000000000' | Select-Object Tokens, TotalEnergy, Peak, Jumps | Format-List

Tokens      : 7
TotalEnergy : 112.2273
Peak        : 5000000000 at 26
Jumps       : {max_bytes at 14, = at 24, 5000000000 at 26}
```
{{< /tab >}}
{{< /tabs >}}

`--field` prints the per-token magnitude, gradient and energy, `--jumps` the scale
discontinuities, `--energy` the heaviest points by local energy, and `--outliers` the tokens
whose magnitude is a stream outlier. In PowerShell each frame carries `Offset`, `Text`,
`Magnitude`, `Gradient` and `Energy`, and `-Detail` adds a frame at every token.

## In a pattern

`\M{>k}` and `\M{<k}` hold where a token's magnitude crosses `k`; `\N{mag>k}` is a number atom
with a magnitude predicate, and the relative forms `\N{>+1}` read it against a
[context](../context/).

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\M{>5}' --text 'retries = 3 ; max_bytes = 5000000000'
[26..36] "5000000000"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\M{>5}").expect("valid pattern");
let text = b"retries = 3 ; max_bytes = 5000000000";
assert_eq!(trex::scan(&pat, text).iter().map(|s| &text[s.range()]).collect::<Vec<_>>(), [&b"5000000000"[..]]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"\M{>5}").scan("retries = 3 ; max_bytes = 5000000000")]
['5000000000']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\M{>5}' -InputObject 'retries = 3 ; max_bytes = 5000000000' -Raw
5000000000
```
{{< /tab >}}
{{< /tabs >}}

## Data model

`MagnitudeFrame`, one token's reading:

| Field | Type | Meaning |
|---|---|---|
| `magnitude` | `f32` | a number's `log10(\|value\|)`, any other token's `log2(byte_len)` |
| `gradient` | `f32` | the change from the previous token |
| `energy` | `f32` | local `sum(m^2)` over the trailing `energy_window` |

`MagnitudeField`, keyed by byte offset:

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | token count |
| `spans` | `Vec<(usize, usize)>` | byte span per token |
| `frames` | `Vec<MagnitudeFrame>` | one per token |
| `jumps` | `Vec<usize>` | byte offsets where `\|gradient\|` reaches the threshold |
| `total_energy` | `f32` | `sum(m^2)` over the whole stream |

Query API: `magnitude_at(byte)`, `gradient_at(byte)`, `energy_of(start, end)`, `outliers()`.

## Algorithm

One pass over the token stream:

1. magnitude: a number's value parsed (decimal, hex, scientific, underscores stripped) to
   `log10(|value|)`, else `log2(byte_len)`;
2. statistics: the stream's mean and standard deviation of `m`, and `total_energy`, in the same
   scan;
3. gradient: `m[i] - m[i-1]`, the first token `0`;
4. jump: `|gradient| >= jump_threshold` records a discontinuity;
5. energy: local `sum(m^2)` over the trailing `energy_window`.

## Cost

| Kernel | Per-token cost | Bounded by |
|---|---|---|
| magnitude | O(1) | numeric parse of the token text |
| statistics | O(1) | one accumulation pass |
| gradient | O(1) | fixed |
| energy | O(1) amortized | `energy_window` |
