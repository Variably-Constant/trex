---
title: Spectral
linkTitle: Spectral
weight: 10
---

# The spectral axis

The temporal substrate. Where the byte grain (the lexer) and the token grain (the
derivative engine) both ask *where are the boundaries?*, the spectral axis asks a
question segmentation cannot: *what is the temporal character of the stream at this
position, independent of any cut?* It is a continuous, position-indexed signal computed
from the bytes by a multi-timescale recurrence.

Source: [`src/spectral.rs`](https://github.com/Variably-Constant/trex/blob/main/src/spectral.rs) · CLI: [`trex spectral`](../cli/#spectral).

## Contents

- [Thesis](#thesis)
- [Four properties, one object](#four-properties-one-object)
- [Where the reader sits](#where-the-reader-sits)
- [Data model](#data-model)
- [Algorithms](#algorithms)
- [Pattern-language surface](#pattern-language-surface)
- [CLI](#cli)
- [Spectral-gated lexing](#spectral-gated-lexing)
- [Cost model](#cost-model)
- [Consumers](#consumers)
- [Design decisions](#design-decisions)

## Thesis

trex reads input at two segmentation scales: the byte grain
([`src/lexer.rs`](https://github.com/Variably-Constant/trex/blob/main/src/lexer.rs), [`src/bytepat.rs`](https://github.com/Variably-Constant/trex/blob/main/src/bytepat.rs),
[`src/byte_simd.rs`](https://github.com/Variably-Constant/trex/blob/main/src/byte_simd.rs)) and the token grain (the partial-derivative
engine over a `Vec<Token>`, [`src/engine.rs`](https://github.com/Variably-Constant/trex/blob/main/src/engine.rs)). Both answer "where are
the boundaries?" and differ only in granularity.

The spectral axis is a third, perpendicular scale. It rides alongside the byte and token
readers and makes explicit the signal whose change-points *are* the boundaries the lexer
draws. Tokenization becomes a reading of it rather than an independent rule set.

## Four properties, one object

A spectral frame is one vector per position, and four things that look separate are the
same numbers read four ways.

1. **Texture and time are duals.** A leaky integrator `x[t] = a*x[t-1] + (1-a)*u[t]` is
   both a one-pole low-pass filter (its pole `a` sets a cutoff, giving texture / band
   energy) and a decaying memory (its impulse response `a^k` sets a time constant
   `tau = -1/ln a`, giving recency). The band is a clock.
2. **Segmentation by change-point.** Byte-pair encoding merges the most *frequent*
   adjacent pair (timeless, dictionary-based). The spectral segmenter cuts where the
   temporal signature *changes* and merges where it is continuous: no dictionary, one
   pass, online, and scale-free across byte, subtoken, and token grains.
3. **Cross-grain connective tissue.** The spectral state is "the decayed past available
   at the present", so it binds prev to current within a grain and across grains (a
   byte-level spike predicting a token-level cut).
4. **Scale-free.** The same recurrence and change-point logic apply at every grain, which
   is what lets one axis sit beneath all of them.

## Where the reader sits

```text
            +---------------------------------------------+
  bytes --->| SPECTRAL READER  (causal, one pass)         |
            |  filterbank . entropy . period . novelty    |
            |  -> SpectralField { frames[], boundaries[] } |
            +---+-----------------------------+-----------+
                | (lexer gate)                | (\F{} pattern atom)
                v                             v
  bytes --> byte reader --> tokens --> token engine --> matches
            (lexer)         (Vec<Token>)  (derivatives)
```

The reader is causal (forward only), so it composes with streaming
([`src/streaming.rs`](https://github.com/Variably-Constant/trex/blob/main/src/streaming.rs)) and keeps the temporal arrow honest. A
bidirectional batch-refinement pass exists for whole-buffer modes but is never on the
streaming path.

## Data model

### `SpectralFrame` - one position's reading

| Field | Type | Meaning |
|---|---|---|
| `bands` | `[f32; 25]` | filterbank EMA, `bands[d*5 + c]` = decay `d` x byte-class `c`, each in `[0,1]` |
| `entropy` | `f32` | rolling Shannon entropy of the local window, normalised to `[0,1]` |
| `period` | `u16` | dominant byte-period of the neighbourhood (`0` = none) |
| `period_strength` | `f32` | normalised autocorrelation peak height `[0,1]` |
| `novelty` | `f32` | k-gram surprise `[0,1]` (`1` = first sighting, near `0` = frequent) |

### `SpectralField` - the side table

| Field | Type | Meaning |
|---|---|---|
| `len` | `usize` | input byte length |
| `hop` | `usize` | sampling stride: `frames[i]` is the frame at byte `i*hop` |
| `frames` | `Vec<SpectralFrame>` | downsampled frames (storage-bounded for large inputs) |
| `boundaries` | `Vec<usize>` | change-point byte offsets, sorted |

The field is keyed by byte offset, not by token, so bytes, subtokens, and tokens all query
it the same way and `Token` stays a small `Copy` struct.

### Byte classes (filterbank input)

| Index | Class | Bytes |
|---|---|---|
| 0 | Digit | `0-9` |
| 1 | Alpha | `_`, `A-Z`, `a-z` |
| 2 | Space | ASCII whitespace |
| 3 | Punct | other printable ASCII |
| 4 | High | `>= 0x80` or non-space control |

### Query API

| Method | Returns |
|---|---|
| `signature(start, end)` | mean-pool of frames covering `[start, end)` (the per-token / per-span reading) |
| `frame_at(byte)` | the nearest sampled frame |
| `boundary_near(byte, tol)` | whether a change-point lies within `tol` bytes |
| `boundary_in(start, end)` | whether a change-point lies inside the span (powers `\F{onset}`) |
| `texture_of(&frame)` | one of `Prose`, `Code`, `Math`, `Data`, `Mixed` |

## Algorithms

All in one causal pass (`analyze_with`).

### Filterbank - the clocks

Decays `a_d = exp(-1/tau_d)` over a pow2-spaced time-constant ladder:

| `d` | `tau_d` (bytes) | `a_d` | reach |
|---|---|---|---|
| 0 | 2 | 0.6065 | byte-local |
| 1 | 8 | 0.8825 | sub-word |
| 2 | 32 | 0.9692 | word / line |
| 3 | 128 | 0.9922 | line / record |
| 4 | 512 | 0.9980 | block / section |

Per byte at class `c`: for each `d`, decay all five band slots by `a_d`, then add `1-a_d`
to slot `c`. Each band is a leaky average bounded in `[0,1]`.

<details>
<summary>The other four kernels</summary>

- **Entropy** - a 256-bin histogram over a sliding window (default 64), maintaining
  `s = sum c_i*log2(c_i)` incrementally so each byte is O(1). `H = log2(n) - s/n`,
  normalised by `log2(min(W,256))`.
- **Periodicity** - centered autocorrelation over a trailing window (default 512) at lags
  `2..max_lag` (default 128); the dominant period is the arg-max with prominence. Recomputed
  every `period_hop` bytes, raised adaptively so total work stays under a fixed op budget
  regardless of input size. No FFT, no dependency.
- **Novelty** - a polynomial rolling hash of the last `k` bytes (default 4) with a count
  map over a sliding window (default 4096) and a ring buffer for eviction;
  `novelty = 1/(1 + count_before_increment)`.
- **Change-point** - per byte, `d[t] = norm(fast_bands[t] - fast_bands[t-1])` over the two
  fastest decays plus entropy; an EWMA mean and mean-abs-deviation of `d` gate a boundary
  when `d[t] > mean + max(k*mad, f*mean)` (`k` default 4.0, `f` default 0.05) and at least
  `cp_min_gap` bytes have passed. The floor `f` matters where `d` is constant rather than
  noisy - a run of one byte class, padding, a block of repeated punctuation - because `mad`
  then falls to nothing and the threshold lands on the signal. Measured over six files of
  prose and source in both line-ending conventions - 69,118 boundaries as CRLF and 74,841
  as LF, the two differing because a carriage return is a byte class arriving at the line
  rate - and over three padded binaries carrying runs of 5,140, 4,169 and 9,812 bytes, a
  floor anywhere from 0.03 to 0.10 moves no boundary in any of them.

  Whether the floor acts is decided by where the run sits rather than by how long it is.
  In all three of those files the run lies deep - 26 kB, 49 kB and 380 kB in - and the
  reader marks the run's two ends and nothing between, so there is nothing to correct.
  The precondition is that the input opens with the constant bytes: `mean` then converges
  onto the run's own residual divergence, the spread collapses, and one ULP clears the
  threshold at byte 2218. That byte is a property of `TAUS` and `CP_ALPHA` rather than
  of the data - it is where the fire lands on letters and on zeros alike, at run lengths
  from 3,735 to 14,864 - and the fire then holds the hysteresis closed through the run's
  real end, so the boundary that matters is lost as well.

  Three committed files in one corpus of 14,017 binaries have it, all sparse
  instruction-set decode tables whose low index range is unpopulated:

  | file | bytes | leading run | floor 0.00 | floor 0.04+ |
  |---|---|---|---|---|
  | `arm32.idx_t32_sub.bin` | 16,384 | 14,864 | `[2218]` | `[14864]` |
  | `ppc.form_idx.bin` | 6,654 | 6,654 (all) | `[2218]` | none |
  | `ppc.prefix_bits.bin` | 13,312 | 3,735 | `[2218, ...]` | `[3735, ...]` |

  Each gained boundary is the leading run's own length, which is where the zeros stop.
  The first file reports only a boundary the file does not have and misses the only one
  it does; the second is zero end to end and so has none, yet reports one. The knee is
  0.04 on all three and on the synthetic case, and the default of 0.05 is one step above
  it.
</details>

## Pattern-language surface

A token atom, parallel to `Atom::BytePattern`, evaluated read-only against the field:
`\F{ <pred> }`. A candidate token matches when the predicate holds of
`field.signature(token.start, token.end)`.

| Surface | Holds when |
|---|---|
| `\F{entropy>0.8}` / `\F{entropy<0.3}` | pooled entropy crosses the threshold (packed vs plain) |
| `\F{period=4}` | the dominant period equals 4 |
| `\F{period}` / `\F{period:any}` / `\F{period:line}` | any strong period is detected |
| `\F{texture:code}` / `:prose` / `:math` / `:data` | the pooled texture class matches |
| `\F{onset}` | a change-point lies within the token's span |

> [!NOTE]
> `\F{period:line}` and `\F{period:any}` are the same predicate: both match any strong
> detected period. Use `\F{period=N}` to require a specific period `N`.

These compose with every operator: `\F{texture:code} \W` (a word in a code-textured
region), `\F{entropy>0.85}+` (a run of high-entropy tokens). A pattern that queries the
axis routes to the set-reachability engine, which builds the field once and threads it
read-only.

## CLI

```console
$ trex spectral --text 'the quick brown fox jumps over the lazy dog'
trex spectral: 43 bytes, 3 frames (hop 16), 0 change-points
  entropy   min 0.62  mean 0.68  max 0.73  (normalised bits/byte)
  period    none detected (no strong byte-periodicity)
  texture timeline (merged regions, cut at change-points):
    [       0..43      ] prose  H=0.68 per=0 nov=1.00
```

| Flag | Effect |
|---|---|
| `--segment` | print the change-point byte offsets |
| `--bands` | per-hop band table (filterbank / entropy / period / novelty) |
| `--classify` | per-region texture timeline |
| `--code-classify` | fused shape x spectral region map (`table` / `code` / `prose` / `data`) |
| `--json` | machine-readable frames and boundaries |

## Spectral-gated lexing

The default `lex()` consults a spectral entropy gate (`spectral::high_entropy_runs`) and
collapses each sustained, whitespace-free high-entropy run (base64 / hex / packed data)
into a single opaque token, instead of shredding it into garbage word and number tokens.
Clean prose and code stay below the gate, so their tokenization is byte-identical to the
ungated lexer. The period-cut and change-point tie-break faces of the gate are described
above but are not on the default path, since they would alter normal tokenization.

## Cost model

| Kernel | Per-byte cost | Bounded by |
|---|---|---|
| filterbank | 25 mul-add | fixed |
| entropy | O(1) incremental | fixed |
| novelty | O(1) amortised | `novelty_window` |
| periodicity | amortised O(1) | adaptive `period_hop` under a fixed op budget |
| change-point | O(1) | fixed |

Filterbank, entropy, novelty, and change-point are genuinely linear and fuse into the byte
scan. Periodicity is the only super-linear kernel and is held linear-with-constant by the
adaptive hop and the `max_lag` cap.

## Consumers

- **The lexer** - the blob-collapse gate above, so every downstream reader sees packed
  data as one token with no per-reader change.
- **`spectral::regions`** - the intrinsic prose / code / math / data region split over
  change-point segments, the dictionary-free, fence-free alternative to marker-based
  region detection; `trex spectral --classify` reads it.
- **The `\F{...}` pattern atom** - the axis composed into the token grain.

## Design decisions

| ID | Decision | Choice | Why |
|---|---|---|---|
| D1 | causal vs bidirectional | causal-primary; bidirectional only as a batch refinement off the streaming path | honest temporal arrow, streaming-safe |
| D2 | segmenter | online change-point | dictionary-free, one-pass, scale-free |
| D3 | signature storage | side table keyed by byte offset, not fields on `Token` | preserves orthogonality; `Token` stays `Copy` |
| D4 | pattern surface | a `\F{...}` atom, four predicates | additive to set-reachability |
| D5 | clocks | pow2 time-constant ladder {2,8,32,128,512} x 5 byte-classes | natural multi-clock spanning byte to block |
| D6 | cost | fuse four kernels into the byte pass; bounded adaptive-hop autocorrelation; no FFT | linear with a small constant |
