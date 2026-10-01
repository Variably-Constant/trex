---
title: Dual-grain scanning
linkTitle: Dual-grain
weight: 30
---

# Dual-grain scanning

A token is a span of bytes, so a scan has two stages over the same input: the byte grain lexes
the bytes into typed tokens, and the token grain matches the pattern over those tokens.

## Inside a match

The byte grain also answers what a token's bytes hold while the token grain matches. A
byte-pattern between backticks is matched against one whole token's bytes, and the content guard
`~"lit"` searches the forward window's bytes with a SIMD substring search. In
`` <`[a-z]+`:t>.*</=t> `` the byte-pattern constrains the tag name's bytes while `=t` requires the
closing name to equal the opening one:

```console
$ trex scan '<`[a-z]+`:t>.*</=t>' --text '<div>hi</div> <H1>x</H1> <b>y</span>'
[0..13] "<div>hi</div>"  captures: t="div"
```

`H1` fails the byte-pattern and `span` is not `b`.

## Producer and consumer

`scan_dual_grain` runs the two stages on two threads. The byte grain lexes the input in chunks
of about 32 KiB, at most 64 of them, and hands each chunk's tokens on; the token grain matches
the tokens produced so far while the byte grain lexes ahead.

The matches are the ones `scan` returns. The token grain commits a match only once no later byte
can change it: it resumes the leftmost scan from the last committed token and commits up to the
largest chunk boundary no match straddles. A pattern whose reach extends outside one match span,
such as a content guard's forward window or a field's comma count from the input start, commits
once at the end, and the byte grain still lexes ahead of it.

## Observing the overlap

Each grain counts its own compute time and not the time it waits on the other. `scan_dual_grain`
returns the matches with a `GrainTiming`: where the two compute totals sum to more than the
wall-clock span, the grains ran at the same time for at least the difference, and `overlap()`
reports that lower bound.
