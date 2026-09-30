---
title: Dual-grain scanning
linkTitle: Dual-grain
weight: 30
---

# Dual-grain scanning

A token is a span of bytes, so the token stream is a coarsening of the byte stream. trex runs
two co-operating grains over the same input at the same time.

- The **token grain** owns structure: balance, binding, valency, and the derivative fold over
  typed atoms.
- The **byte grain** owns literal speed and sub-token detail: SIMD literal scanning,
  character-class membership inside a token, and the presence prefilters that answer the
  content guard.

## Complementary, not redundant

The two grains answer different questions about the same bytes, and their results join. A
pattern is split at compile time: byte-level constraints route to the byte grain, structural
constraints route to the token grain, and the join reconciles them on overlapping spans. The
pattern `` <`[a-z]+`:t>.*</=t> `` carries both at once - the `` `[a-z]+` `` byte-pattern
constrains the tag name's bytes while `=t` enforces the structural open/close match.

## Producer and consumer

The byte grain also produces the token stream the token grain consumes. Rather than tokenize
fully and then scan, the byte grain tokenizes *ahead* while the token grain consumes the
emerging stream, so the two overlap in time as a producer and a consumer. `scan_dual_grain`
returns the matches (identical to a plain `scan`) plus a `GrainTiming` whose `overlap()` is a
lower bound on the time the two grains ran at once.

## Why not race

A racing variant - both grains attempting the whole match while a referee picks a winner - is
deliberately out of the architecture. Two engines doing the same job on the same bytes contend
for the same work, and the redundant grain wastes the cycles it spends. The co-operating split
keeps each grain on the work it is best at.
