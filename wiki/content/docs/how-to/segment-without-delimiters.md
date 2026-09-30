---
weight: 40
---

# Segment text with no delimiters

The [seam axis](../../reference/axes/seam/) cuts where the past stops predicting the future,
using the stream's own running statistics in both directions - no dictionary, no trained model.

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

## Recover word boundaries from run-together text

The decisive test: segment spaceless text and score the recovered boundaries against the known
ones, next to a count-BPE baseline.

```console
$ trex seam --recover --compare-bpe
trex seam --recover: 2113 bytes (400 words), 399 true boundaries, order 3, passes 1
  seam (no dictionary):   P=0.998 R=1.000 F1=0.999  (400 cuts, 399 hit)
  count-BPE (200 merges): P=1.000 R=0.411 F1=0.583  (164 piece boundaries)
  -> seam recovers word boundaries +0.416 F1 over count-BPE: the bidirectional
     predictive signal (past<->future branching entropy) BPE has no access to.
```

Tune the context order with `--order K`; add `--field` for the per-byte forward / backward
branching entropy.
