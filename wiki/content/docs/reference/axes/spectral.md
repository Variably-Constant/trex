---
title: Spectral
linkTitle: Spectral
weight: 10
---

# The spectral axis

A reading of the input at every byte position: the byte-class mix at five timescales, the
entropy, the byte period and the novelty of the neighbourhood, and the offsets where that
reading changes.

Source: [`src/spectral.rs`](https://github.com/Variably-Constant/trex/blob/main/src/spectral.rs).

The reader is one causal forward pass: the reading at byte `t` depends only on the bytes up to
`t`, so a change in the input shows in the reading a few bytes after the bytes that cause it.

## Reading the axis

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex spectral --text 'the quick brown fox jumps over the lazy dog'
trex spectral: 43 bytes, 3 frames (hop 16), 0 change-points
  entropy   min 0.62  mean 0.68  max 0.73  (normalized bits/byte)
  period    none detected (no strong byte-periodicity)
  texture timeline (merged regions, cut at change-points):
    [       0..43      ] prose  H=0.68 per=0 nov=1.00
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::spectral::{Texture, analyze, regions};

let field = analyze(b"the quick brown fox jumps over the lazy dog");
assert_eq!((field.len, field.hop, field.frames.len()), (43, 16, 3));
assert!(field.boundaries.is_empty());
assert_eq!(regions(&field), [(0, 43, Texture::Prose)]);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexSpectral 'the quick brown fox jumps over the lazy dog' | Select-Object Length, Hop, ChangePoints, Timeline | Format-List

Length       : 43
Hop          : 16
ChangePoints : {}
Timeline     : {Prose at 0, 43 long}
```
{{< /tab >}}
{{< /tabs >}}

| Flag | Effect |
|---|---|
| `--segment` | the change-point byte offsets |
| `--bands` | one row a frame: the medium-clock class mix, entropy, period, novelty and texture |
| `--classify` | the texture timeline alongside `--segment` or `--bands`; it is printed by default otherwise |
| `--code-classify` | the regions read by texture and token [shape](../shape/#region-fusion) together: `table`, `code`, `prose`, `blob`, `numeric` or `mixed` |
| `--json` | the field's frames and boundaries as JSON |
| `--limit N` | at most `N` rows of `--segment` and `--bands` |

In PowerShell `-Classify` adds the regions `--code-classify` prints, and `-Detail` adds every
frame.

## In a pattern

`\F{pred}` is a token atom: a token matches when the predicate holds of the pooled reading over
its span.

| Atom | Holds when |
|---|---|
| `\F{entropy>0.8}` / `\F{entropy<0.3}` | the pooled entropy is above or below the threshold, read to the hundredth |
| `\F{period=4}` | the dominant byte period is 4 |
| `\F{period}` / `\F{period:any}` / `\F{period:line}` | any strong period is detected; the three are one predicate |
| `\F{texture:prose}` / `:code` / `:math` / `:data` | the pooled texture is that class; `texture=` is the same |
| `\F{onset}` | a change-point lies inside the token's span |

A pattern naming the axis runs on the set engine, which builds the field once a scan with only
the readings its atoms take: `entropy` the entropy, `period` the period, `texture` the class mix
and the entropy, and `onset` those two and the change-points.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\F{texture:code}+' --text 'the quick brown fox jumps; fn add(a,b){let c=a+b;return c*2;}'
[33..61] "(a,b){let c=a+b;return c*2;}"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\F{texture:code}+").expect("valid pattern");
let text = "the quick brown fox jumps; fn add(a,b){let c=a+b;return c*2;}";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["(a,b){let c=a+b;return c*2;}"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"\F{texture:code}+").scan("the quick brown fox jumps; fn add(a,b){let c=a+b;return c*2;}")]
['(a,b){let c=a+b;return c*2;}']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\F{texture:code}+' -InputObject 'the quick brown fox jumps; fn add(a,b){let c=a+b;return c*2;}' -Raw
(a,b){let c=a+b;return c*2;}
```
{{< /tab >}}
{{< /tabs >}}

`fn add` is read as prose: the medium clock still holds the sentence before it.

## In the lexer

The lexer collapses each whitespace-free run whose rolling entropy holds at or above 0.85 for at
least 48 bytes (`BLOB_ENTROPY_PCT`, `BLOB_MIN_LEN`) into one opaque token, so base64, hex and
packed data are not split into word and number tokens:

```console
$ trex scan . --text 'key = aGVsbG8gd29ybGQgdGhpcyBpcyBiYXNlNjQgZGF0YSB0aGF0IGtlZXBzIGdvaW5nIGZvciBhIHdoaWxl end'
[0..3] "key"
[4..5] "="
[6..86] "aGVsbG8gd29ybGQgdGhpcyBpcyBiYXNlNjQgZGF0YSB0aGF0IGtlZXBzIGdvaW5nIGZvciBhIHdoaWxl"
[87..90] "end"
```

Prose and code stay below the gate. `spectral::high_entropy_runs` returns the runs.

## Data model

`SpectralFrame`, one position's reading:

| Field | Type | Meaning |
|---|---|---|
| `bands` | `[f32; 25]` | the filterbank: `bands[d*5 + c]` is decay `d` by byte class `c`, each in `[0,1]` |
| `entropy` | `f32` | rolling Shannon entropy of the local window, normalized to `[0,1]` |
| `period` | `u16` | the dominant byte period of the neighbourhood, `0` for none |
| `period_strength` | `f32` | the normalized autocorrelation peak, `[0,1]` |
| `novelty` | `f32` | k-gram surprise, `[0,1]`, `1` for a first sighting |

`SpectralField`, keyed by byte offset:

| Field | Type | Meaning |
|---|---|---|
| `len` | `usize` | input length in bytes |
| `hop` | `usize` | sampling stride: `frames[j]` reads the bytes up to `(j+1)*hop - 1` |
| `frames` | `Vec<SpectralFrame>` | the sampled frames |
| `boundaries` | `Vec<usize>` | change-point byte offsets, ascending |
| `needs` | `Needs` | the readings the field carries; one it does not carry reads as zero |

| Method | Returns |
|---|---|
| `frame_at(byte)` | the sampled frame covering `byte` |
| `signature(start, end)` | the mean of the frames covering `[start, end)`, with the period of the strongest one |
| `boundary_near(byte, tol)` | whether a change-point lies within `tol` bytes |
| `boundary_in(start, end)` | whether a change-point lies inside the span |

`analyze(input)` computes every reading with the default configuration, `analyze_with(input,
cfg)` with a `SpectralConfig`, and `analyze_needing(input, cfg, needs)` only the readings
`needs` names. `texture_of(&frame)` classifies a frame as `Prose`, `Code`, `Math`, `Data` or
`Mixed`, `regions(&field)` cuts the input at its change-points and merges adjacent spans of one
texture, and `code_regions(input)` labels each span `Code`, `Blob`, `Prose`, `Numeric` or `Mixed`
from a fresh pass over that span's bytes.

| Index | Byte class | Bytes |
|---|---|---|
| 0 | digit | `0-9` |
| 1 | alpha | `_`, `A-Z`, `a-z` |
| 2 | space | ASCII whitespace |
| 3 | punct | other printable ASCII |
| 4 | high | `>= 0x80` and non-space control bytes |

| `SpectralConfig` field | Default | Meaning |
|---|---|---|
| `hop` | 16 | frame sampling stride in bytes |
| `entropy_window` | 64 | rolling entropy window in bytes |
| `period_window` | 512 | autocorrelation window in bytes |
| `max_lag` | 128 | the largest period searched |
| `period_hop` | 64 | baseline stride between autocorrelation evaluations |
| `ngram` | 4 | k-gram size for novelty |
| `novelty_window` | 4096 | novelty window in k-grams |
| `cp_threshold` | 4.0 | change-point sensitivity `k` |
| `cp_floor` | 0.05 | change-point floor `f`, a fraction of the mean |
| `cp_min_gap` | 4 | minimum bytes between two change-points |

## Algorithms

### Filterbank

Five leaky integrators per byte class, `x[t] = a*x[t-1] + (1-a)*u[t]`, with `u[t]` one for the
byte's class and zero for the others. The decays `a_d = exp(-1/tau_d)` follow a power-of-two
ladder of time constants:

| `d` | `tau_d` (bytes) | `a_d` | reach |
|---|---|---|---|
| 0 | 2 | 0.6065 | byte-local |
| 1 | 8 | 0.8825 | sub-word |
| 2 | 32 | 0.9692 | word and line |
| 3 | 128 | 0.9922 | line and record |
| 4 | 512 | 0.9980 | block and section |

Each band is a leaky average in `[0,1]`. The texture is read from the medium clock, `d = 2`.

### Entropy, period and novelty

- Entropy: a 256-bin histogram over the sliding window, keeping `s = sum c_i*log2(c_i)`
  incrementally, so each byte is O(1); `H = log2(n) - s/n`, normalized by `log2(min(W, 256))`.
- Period: centered autocorrelation over the trailing window at lags `2..max_lag`, the dominant
  period the arg-max with prominence, recomputed every `period_hop` bytes; the hop rises with the
  input so the total work stays under a fixed operation budget.
- Novelty: a rolling hash of the last `k` bytes with a count map over the window and a ring
  buffer for eviction; `novelty = 1/(1 + count_before_increment)`.

### Change-points

Per byte, `d[t]` is the distance between the two fastest clocks' class mix and entropy at `t`
and at `t-1`. An EWMA mean and mean absolute deviation of `d` give the threshold: a boundary is
recorded where `d[t] > mean + max(k*mad, f*mean)` and at least `cp_min_gap` bytes have passed
since the last.

The floor `f` acts where `d` is constant rather than noisy: a run of one byte class drives `mad`
to zero and puts the threshold on the signal. Over six files of prose and source in both
line-ending conventions (69,118 boundaries as CRLF, 74,841 as LF) and three padded binaries with
constant runs of 5,140, 4,169 and 9,812 bytes, a floor anywhere from 0.03 to 0.10 moves no
boundary. In those three binaries the run lies 26 kB, 49 kB and 380 kB in, and the reader marks
its two ends and nothing between.

The floor acts where the input opens with the constant run. `mean` then converges onto the run's
own residual divergence, the spread collapses, and without the floor one ULP clears the
threshold at byte 2218, for letters and zeros alike and at run lengths from 3,735 to 14,864; the
hysteresis then holds through the run's real end, so that boundary is lost too. In a corpus of
14,017 binaries three files open this way, all sparse instruction-set decode tables:

| File | Bytes | Leading run | Floor 0.00 | Floor 0.04 and up |
|---|---|---|---|---|
| `arm32.idx_t32_sub.bin` | 16,384 | 14,864 | `[2218]` | `[14864]` |
| `ppc.form_idx.bin` | 6,654 | 6,654 (all) | `[2218]` | none |
| `ppc.prefix_bits.bin` | 13,312 | 3,735 | `[2218, ...]` | `[3735, ...]` |

Each boundary the floor gains is the leading run's length, where the zeros stop. The knee is 0.04
on all three files and on a synthetic constant run; the default is 0.05.

## Cost

| Kernel | Per-byte cost | Bounded by |
|---|---|---|
| filterbank | 25 multiply-adds | fixed |
| entropy | O(1) incremental | fixed |
| novelty | O(1) amortized | `novelty_window` |
| period | O(1) amortized | the adaptive `period_hop` under a fixed operation budget |
| change-point | O(1) | fixed |
