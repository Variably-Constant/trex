---
title: The spectral axis
linkTitle: Spectral
weight: 10
---

A reading of the input at every byte position: the byte-class mix at five timescales, the
entropy, the byte period and the novelty of the neighborhood, and the offsets where that
reading changes.

Source: [`src/spectral.rs`](https://github.com/Variably-Constant/trex/blob/main/src/spectral.rs).

The reader is one causal forward pass: the reading at byte `t` depends only on the bytes up to
`t`, so a change in the input shows in the reading a few bytes after the bytes that cause it.

## What it reads that nothing else does

```text
ID0001ALICE     NY
ID0002BOB       CA
ID0003CAROL     TX
```

A fixed-width file places its fields by position, not by a separator: the lexer reads
`ID0001ALICE` as one word and the padding as space, so over sixty such rows shape reads every
token as one silhouette, a period of 1 token. Spectral reads the bytes at every position and
finds the record width, a byte period of 19 at strength 0.90. The same reading measures a binary
file, where there are no tokens to read at all.

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
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.spectral("the quick brown fox jumps over the lazy dog")
>>> r.length, r.hop, r.change_points
(43, 16, [])
>>> [(t.texture, t.offset, t.length) for t in r.timeline]
[('prose', 0, 43)]
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
| `--timeline` | the texture timeline alongside `--segment` or `--bands`; it is printed by default otherwise |
| `--classify` | the regions read by texture and token [shape](../shape/#region-fusion) together: `table`, `code`, `prose`, `blob`, `numeric` or `mixed` |
| `--json` | the field's frames and boundaries as JSON |
| `--limit N` | at most `N` rows of `--segment` and `--bands` |

In PowerShell `-Classify` adds the regions `--classify` prints, and `-Detail` adds every
frame; in Python `classify=True` and `detail=True` do the same.

### Change points

The reader compares each frame's reading with the frames before it. A change point is a byte
where the two differ by more than their usual spread, and, once a change point has opened a new
texture, the byte where the reading comes back to the texture it left. A span of code inside a
sentence gets both, one where the code begins and one where the prose resumes:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex spectral --text 'The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.'
trex spectral: 324 bytes, 21 frames (hop 16), 2 change-points
  entropy   min 0.60  mean 0.67  max 0.78  (normalized bits/byte)
  period    none detected (no strong byte-periodicity)
  texture timeline (merged regions, cut at change-points):
    [       0..138     ] prose  H=0.64 per=0 nov=0.89
    [     138..192     ] code   H=0.76 per=0 nov=0.83
    [     192..324     ] prose  H=0.67 per=0 nov=0.93
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::spectral::analyze(b"The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.");
assert_eq!(field.boundaries, [138, 192]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> mixed = "The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers."
>>> s = trex.axes.spectral(mixed)
>>> s.change_points, [(t.texture, t.offset, t.length) for t in s.timeline]
([138, 192], [('prose', 0, 138), ('code', 138, 54), ('prose', 192, 132)])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $mixed = 'The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.'
PS> (Measure-TrexSpectral $mixed).ChangePoints
138
192
```
{{< /tab >}}
{{< /tabs >}}

The code runs from byte 134 to 184. The entry is 4 bytes in, once enough of its brackets and
operators have arrived to move the reading. The exit is 8 bytes past the end: the reader waits
until the recent reading has stayed close to the prose the code interrupted for twelve bytes in a
row, so a run of letters inside the code, such as `if`, does not end it early, and then places
the exit on the first of those twelve bytes, where the prose came back. A short span is also hard
to see at all: a run of code that reads much like the words around it, with spaces between its
words, moves the reading too little for a change point at either edge.

### Regions

`--classify` reads the regions by texture and token [shape](../shape/#region-fusion) together,
naming each `table`, `code`, `prose`, `blob`, `numeric` or `mixed`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex spectral --classify --text 'The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.'
trex spectral (region classification: shape x spectral): 324 bytes, 3 regions
    [       0..138     ] prose      The nightly report lists eve
    [     138..192     ] code       =(b[j]*c[k]+d[i-1])>>2;if(a[
    [     192..324     ] prose      payload, and it sends the to
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::shape::{RegionKind, classified_regions};

let text = b"The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.";
assert_eq!(
    classified_regions(text),
    [(0, 138, RegionKind::Prose), (138, 192, RegionKind::Code), (192, 324, RegionKind::Prose)]
);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(r.kind, r.offset, r.length) for r in trex.axes.spectral(mixed, classify=True).regions]
[('prose', 0, 138), ('code', 138, 54), ('prose', 192, 132)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexSpectral $mixed -Classify).Regions | Format-Table Offset, Length, Kind

Offset Length  Kind
------ ------  ----
     0    138 Prose
   138     54  Code
   192    132 Prose
```
{{< /tab >}}
{{< /tabs >}}

The texture readings read the bytes as UTF-8, so prose in any script reads as prose: a letter
outside ASCII is a letter on every byte, and a typographic quote, dash or full stop counts once,
as its ASCII counterpart does. Bytes not part of a well-formed character read as data.

## Choosing a threshold

**The change-point threshold.** A change point needs the divergence between a frame and the
frames before it to pass its running mean by more than `K` times its running mean absolute
deviation, `K` being `--cp-threshold`, 4 by default; the comparison is strict. Each byte moves
that mean and deviation by at most three deviations, so a divergence that rises over many bytes,
as code inside Chinese or Japanese prose does, cannot carry the threshold up ahead of itself.
Lower `K` to catch milder changes, raise it to keep only the sharpest. At 2 the passage above
gains a change point at 109, in the prose before the code, where a milder shift passes the lower
bar, and the entry moves to 135, the first byte whose divergence passes it:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex spectral --segment --cp-threshold 2 --text 'The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.'
trex spectral: 324 bytes, 21 frames (hop 16), 3 change-points
  entropy   min 0.60  mean 0.67  max 0.78  (normalized bits/byte)
  period    none detected (no strong byte-periodicity)
  change-points (3):
    109 135 192
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::spectral::{SpectralConfig, analyze_with};

let cfg = SpectralConfig { cp_threshold: 2.0, ..SpectralConfig::default() };
let field = analyze_with(b"The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.", &cfg);
assert_eq!(field.boundaries, [109, 135, 192]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.axes.spectral(mixed, cp_threshold=2).change_points
[109, 135, 192]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexSpectral $mixed -CpThreshold 2).ChangePoints
109
135
192
```
{{< /tab >}}
{{< /tabs >}}

The exit is not a threshold crossing: it is where the reading has come back to the texture
the entry left, so it stays at 192 whatever `K` is. `K` decides which entries open a span, and
each span then ends where its texture gives way to the one before it, if it does before the
input ends.

**The floor.** Where the divergence is steady rather than noisy its spread falls toward zero,
which would put the threshold on the signal itself. `--cp-floor F` keeps the margin over the mean
at least `F` of the mean, 0.05 by default; at 0 the threshold is the mean plus `K` spreads
whatever the spread. **The gap.** `--cp-min-gap N` keeps two change points at least `N` bytes
apart, 4 by default. In PowerShell the settings are `-CpThreshold`, `-CpFloor` and `-CpMinGap`,
and in Python `cp_threshold=`, `cp_floor=` and `cp_min_gap=`.

**A predicate's value.** `\F{entropy>k}` reads the pooled entropy, from 0 to 1; `--bands` lists
each frame's entropy, so choose `k` between the frames of the texture you want and the rest.

## Declarations

Spectral reads bytes, not tokens, so it takes no declarations: no declared shape changes what it
reads. The region classification reads token shape too, and lexes with the built-in recognizers.

## In a pattern

`\F{pred}` is a token atom: a token matches when the predicate holds of the pooled reading over
its span.

| Atom | Holds when |
|---|---|
| `\F{entropy>0.8}` / `\F{entropy<0.3}` | the pooled entropy is above or below the threshold, read to the hundredth |
| `\F{period=4}` | the dominant byte period is 4 |
| `\F{period}` / `\F{period:any}` / `\F{period:line}` | any strong period is detected; the three are one predicate |
| `\F{texture:prose}` / `:code` / `:math` / `:data` | the pooled texture is that class; `texture=` is the same |
| `\F{onset}` | a change-point is inside the token's span |

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

Prose and code stay below the gate. A script written in multi-byte characters spreads each
character over bytes that carry more entropy than the text they spell, so the gate rises toward
one with the share of the window's bytes that are inside well-formed UTF-8 characters: a Chinese
sentence stays words and punctuation, while base64 and bytes not part of a well-formed character
collapse into one token. `spectral::high_entropy_runs` returns the runs.

## Data model

`SpectralFrame`, one position's reading:

| Field | Type | Meaning |
|---|---|---|
| `bands` | `[f32; 25]` | the filterbank: `bands[d*5 + c]` is decay `d` by byte class `c`, each in `[0,1]` |
| `entropy` | `f32` | rolling Shannon entropy of the local window, normalized to `[0,1]` |
| `period` | `u16` | the dominant byte period of the neighborhood, `0` for none |
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
| `boundary_near(byte, tol)` | whether a change-point is within `tol` bytes |
| `boundary_in(start, end)` | whether a change-point is inside the span |

`analyze(input)` computes every reading with the default configuration, `analyze_with(input,
cfg)` with a `SpectralConfig`, and `analyze_needing(input, cfg, needs)` only the readings
`needs` names. `texture_of(&frame)` classifies a frame as `Prose`, `Code`, `Math`, `Data` or
`Mixed`, `regions(&field)` cuts the input at its change-points and merges adjacent spans of one
texture, and `code_regions(input)` labels each span `Code`, `Blob`, `Prose`, `Numeric` or `Mixed`
from a fresh pass over that span's bytes.

| Index | Byte class | Bytes |
|---|---|---|
| 0 | digit | `0-9`, and every numeral outside ASCII |
| 1 | alpha | `_`, `A-Z`, `a-z`, and every letter or combining mark outside ASCII |
| 2 | space | ASCII whitespace, and every space outside ASCII |
| 3 | punct | other printable ASCII, and every other character outside ASCII |
| 4 | other | non-space control bytes, and every byte that is not part of a well-formed UTF-8 character |

The reader takes its input as UTF-8. A letter outside ASCII is alpha on each of its bytes, so a
word in Cyrillic or Chinese weighs by its bytes as an ASCII word does. Any other character
outside ASCII, a typographic quote, a dash, an ideographic full stop, counts once: its first byte
adds one unit of its class and its other bytes each add the current mix, which leaves the mix
where that unit put it, so `’` weighs what `'` does.

`texture_of` calls a span `Data`, and `code_texture` calls it `Blob`, when more than 30% of its
recent bytes are class 4, or when its entropy passes a gate, 0.85 and 0.78, that rises toward
one with the share of its recent bytes inside well-formed multi-byte characters.

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
byte's class and zero for the others. The bytes of a multi-byte character are held until the
character completes and then go in together, `count` bytes of one class whose newest arrived
`age` bytes ago adding the `a^age * (1 - a^count)` they would have added one at a time, so the
reading at a byte never depends on a byte after it. A character other than a letter adds its one
unit and then scales the rest of the band to the total its bytes bring, keeping the mix's
proportions. The decays `a_d = exp(-1/tau_d)` follow a power-of-two ladder of time constants:

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

Per byte, `d[t]` is the distance between the class mix of the 8-byte clock and the 128-byte
clock, each corrected for its warm-up, plus twice the gap between the windowed entropy and its
slow average. An EWMA mean and mean absolute deviation of `d` give the threshold: an entry is
recorded where `d[t] > mean + max(k*mad, f*mean)` and at least `cp_min_gap` bytes have passed
since the last change point, and the reader takes no further entry until `d` falls below half
the threshold or an exit closes the span. From byte 32, where entries may begin, each byte moves
the mean and the deviation by its departure from the mean clipped at three times the spread the
threshold is computed from, `max(mad, f*mean/k)` and never under 1e-6, the rounding noise of a reading
of order one: Huber's robust update. A span whose `d` climbs over many bytes otherwise lifts the
threshold faster than itself and is never entered, and after a long span the threshold stays
high over the prose that follows.

An entry also records the 128-byte clock's mix as the texture the input left. From there the
reader follows the distance of the 8-byte clock's mix from that texture and its running mean
since the entry; once the distance has stayed under half that mean for twelve bytes in a row, an
exit is recorded on the first of them, at least `cp_min_gap` bytes after the entry. The 128-byte
clock moves little over a span shorter than its time constant, so the divergence alone never
marks where such a span ends; the return to the texture before it does.

The floor `f` acts where `d` is constant rather than noisy: a run of one byte class drives `mad`
to zero and puts the threshold on the signal. Over six files of prose and source in both
line-ending conventions (69,118 boundaries as CRLF, 74,841 as LF) and three padded binaries with
constant runs of 5,140, 4,169 and 9,812 bytes, a floor anywhere from 0.03 to 0.10 moves no
boundary. In those three binaries the run is 26 kB, 49 kB and 380 kB in, and the reader marks
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
| filterbank | 25 multiply-adds, and one more pass over the 25 bands when a multi-byte character completes | fixed |
| entropy | O(1) incremental | fixed |
| novelty | O(1) amortized | `novelty_window` |
| period | O(1) amortized | the adaptive `period_hop` under a fixed operation budget |
| change-point | O(1) | fixed |
