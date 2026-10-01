---
title: Measure how far text compresses
linkTitle: Compress text
weight: 27
---

# Measure how far text compresses

Read the code length trex's context-mixing model gives a text: the size a coder driven by that
model would write, in bits per byte. The model reads tokens, orbits and a prior learned from a
corpus. No compressed stream is written ([compression](../../reference/tools/#compression)).

`compress` needs a build with the `compress` feature:

```text
cargo build --release --features compress
```

## Read the code length

```console
$ trex compress --text 'the quick brown fox jumps over the lazy dog the quick brown fox jumps'
trex compress: 69 bytes -> 15 bytes  (1.626 bits/byte, 21.7% of original)
  coder: logistic mix + orbit + baked prior   0.00 MB/s   (--compare for the full table)
```

The size is the code length in bits rounded up to bytes.

## Compare the models

`--compare` prints every model's bits per byte and speed beside a byte-pair-encoding baseline, and
`--no-baked` reads with no prior, which shows what the prior is worth on your text. `--chunks N`,
`--gpu` and `--hybrid` split the work for speed at some cost in length, as [choose a
backend](../choose-a-backend/#compressing) measures.
