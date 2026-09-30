---
title: Seam
linkTitle: Seam
weight: 20
---

# The seam axis: bidirectional predictive segmentation

Cut a stream where the past stops predicting the future, with no dictionary and no trained
model. The companion of the [spectral axis](../spectral/): the spectral reader is the causal
(past-to-present) temporal substrate; seam is its bidirectional mirror, segmenting where
prediction breaks rather than where frequency is high.

Source: [`src/seam.rs`](https://github.com/Variably-Constant/trex/blob/main/src/seam.rs) · CLI: [`trex seam`](../cli/#seam).

## Contents

- [Thesis](#thesis)
- [What this reaches that BPE cannot](#what-this-reaches-that-bpe-cannot)
- [The bidirectional reader](#the-bidirectional-reader)
- [Algorithm](#algorithm)
- [The proof: recover word boundaries, beat BPE](#the-proof-recover-word-boundaries-beat-bpe)
- [CLI](#cli)
- [Cost](#cost)
- [Design decisions](#design-decisions)

## Thesis

Byte-pair encoding builds a tokenizer from one statistic: the global frequency of adjacent
symbol pairs. It merges the most frequent pair repeatedly into a trained dictionary, then
applies it greedily everywhere. It is a compression method repurposed as a segmenter.

A token boundary is something else: a point where the past stops predicting the future.
Inside a token the parts cohere; at a boundary the past and future decouple. A boundary is
a local minimum of the mutual information `I(past; future)` across the seam between two
units. Seam cuts at those minima, reading the predictor from the stream's own running
statistics in both directions - no trained dictionary, no learned model.

## What this reaches that BPE cannot

BPE's per-pair frequency is a flattened projection of `I(past; future)`. Seam recovers the
field along four axes BPE discards:

| Axis | BPE | Seam |
|---|---|---|
| Direction | symmetric pair count | forward **and** backward branching entropy |
| Locality | global merge table | position-specific, context-conditioned |
| Scale | char to subword | byte to phrase, multi-scale (spectral fusion) |
| Statistic | raw count of `(a,b)` | branching entropy `H(next \| context)` |
| Output | bare spans | spans plus their fwd/bwd surprise |

The forward-and-backward pairing is the load-bearing novelty. The prior art that uses
prediction for segmentation is forward-only: Harris successor variety / branching entropy
(Harris 1955; Jin and Tanaka-Ishii) cuts on `H(future | past)` alone, and the Byte Latent
Transformer (arXiv:2412.09871) patches on a trained byte-LM's next-byte entropy. Seam adds
the backward term (`H(past | future)`), reads both from the stream itself, and fuses the
multi-scale spectral prior.

## The bidirectional reader

The spectral reader is causal: its state at `t` is the decayed past. Reading one unit ahead
turns a lagged estimator into a centered one: at the seam between `t` and `t+1` the reader
holds the summary of the past and the actual next unit, so it scores the seam from both
sides with no lag. Seam realises this as two passes: forward branching entropy at each
position (looking right) and backward branching entropy (looking left), combined at every
seam. The one-unit look-ahead lets the cut at `t` be decided already knowing `t+1`.

## Algorithm

For context order `k` (default 3):

- **forward** maps each `k`-byte context to the distribution of the byte that follows it;
  `fwd_entropy[t]` is the Shannon entropy of that distribution for the context ending at
  `t`. High = many continuations = a boundary lies just after `t`.
- **backward** maps each `k`-byte context to the distribution of the byte that precedes it;
  `bwd_entropy[t]` is the entropy of the predecessor distribution for the context starting
  at `t`. High = a boundary lies just before `t`.

For a cut between byte `t-1` and `t`:

```text
boundary[t] = fwd_entropy[t-1] + bwd_entropy[t]       (both sides branch)
            + spectral bonus     when a spectral change-point is near t
            + lexer-cut union    (token starts are always boundaries)
```

The spectral bonus folds the coarse, multi-scale regime signal into the fine byte-level
one. A cut is placed at `t` when `boundary[t]` is a local maximum above an adaptive
threshold (`mean + cut_threshold * std`) and at least `min_seg` bytes past the previous
cut. Offset `0` is always a cut. The result is
`SeamField { len, fwd_entropy[], bwd_entropy[], boundary[], cuts[] }`, plus
`segments() -> [(start, end)]` and `internal_cuts()`.

> [!NOTE]
> Branching entropy needs statistical mass. On a single short one-shot string with no
> repetition, seam under-segments, exactly as the theory predicts; on a repeated corpus it
> is near-exact. The signal strengthens with stream length as `H(next | context)` converges.

## The proof: recover word boundaries, beat BPE

The measurable end-to-end test: generate text by concatenating words from a fixed
vocabulary in pseudo-random order with no spaces, recording the true boundary offsets, then
score boundary recovery for seam (no dictionary) against count-BPE trained on the same text.

```console
$ trex seam --recover --compare-bpe
trex seam --recover: 2113 bytes (400 words), 399 true boundaries, order 3, passes 1
  seam (no dictionary):   P=0.998 R=1.000 F1=0.999  (400 cuts, 399 hit)
  count-BPE (200 merges): P=1.000 R=0.411 F1=0.583  (164 piece boundaries)
  -> seam recovers word boundaries +0.416 F1 over count-BPE: the bidirectional
     predictive signal (past<->future branching entropy) BPE has no access to.
```

The end of a word is followed by the start of any word (high branching entropy);
word-internal transitions are near-deterministic (low entropy). BPE merges frequent
char-pairs across word boundaries and recovers them poorly. The F1 gap is the signal BPE
cannot reach, made into a number.

## CLI

```console
$ trex seam --text 'the cat sat'
trex seam: 11 bytes, order 3, 5 segments (bidirectional branching entropy)
  segments:
    [     0..3     ] "the"
    [     3..4     ] " "
    [     4..7     ] "cat"
    [     7..8     ] " "
    [     8..11    ] "sat"
```

| Flag | Effect |
|---|---|
| `--order K` | context order (default 3) |
| `--field` | per-byte forward / backward branching-entropy table |
| `--segment` | the cut points and recovered segments |
| `--recover [--compare-bpe]` | the spaceless-text boundary-recovery benchmark, optionally vs count-BPE |
| `--english` | seed with the built-in English model |
| `--json` | machine-readable field and cuts |

The `--compress` mode and its model-building flags (a dictionary-free predictive coder) are
in the [CLI reference](../cli/#seam).

## Cost

Two `HashMap`-backed passes (forward, backward): O(n) build plus O(n x avg-branch) entropy.
The spectral prior is one `spectral::analyze` pass. Context maps are bounded by the distinct
`k`-grams; large inputs cap `k` and the map.

## Design decisions

| ID | Decision | Choice | Why |
|---|---|---|---|
| S1 | boundary signal | bidirectional branching entropy (fwd + bwd) | the full `I(past;future)` dip, not the forward-only shadow |
| S2 | model | the input's own running k-gram statistics, two passes | dictionary-free and model-free; no pre-training |
| S3 | multi-scale | fuse `spectral` change-points as a coarse prior | one segmenter spanning byte to regime scale |
| S4 | proof | spaceless word-boundary recovery F1 vs BPE | the signal gap made measurable and fair |
