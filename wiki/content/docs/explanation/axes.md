---
title: The axes
linkTitle: The axes
weight: 15
---

A pattern asks what each token is: a number, a word, a bracket group. An axis reads what the
input does at each position: how large a value is against the values beside it, how deep the
brackets around it run, whether its content has been seen before. trex has twelve, each a
reading taken at every position and kept by byte offset, so every axis answers for the same
place and a pattern can ask any of them there.

## What an axis is

An axis reads one grain of the input: its bytes, its tokens, or the
[supertokens](../architecture/#supertokens) its tokens group into.
It takes its reading at every position in one pass, and marks the points where the reading
turns: a jump, a peak, a fracture, a reversal, a contested point, a change point, a cut. The
[reference pages](../../reference/axes/) give each axis's readings, events and cost.

Each is read on its own by a command, a cmdlet, a function in Python's `trex.axes` and a Rust
module, and most inside a pattern as well.

## The twelve

| Axis | Grain | Reads at each position | What it alone reads |
|---|---|---|---|
| [spectral](../../reference/axes/spectral/) | byte | the byte-class mix at five timescales, entropy, byte period, novelty | a fixed-width file's record width, 19 bytes, where shape reads one silhouette |
| [observation](../../reference/axes/observation/) | byte | the entropy behind, ahead and around, and how far they disagree | the line break where code turns into a base64 blob, nine bytes before spectral's cut |
| [seam](../../reference/axes/seam/) | byte, token, supertoken | where the input before a point stops predicting the input after it | the six words in `thebirdsflyoverthehouses`, one token to the lexer |
| [shape](../../reference/axes/shape/) | token | each token's silhouette, the period it repeats at, the template regions | rows of different widths repeating `N , N , N` |
| [orbit](../../reference/axes/orbit/) | token | each token's representative under case, notation or word shape | `ERROR`, `Error` and `error` as one token |
| [magnitude](../../reference/axes/magnitude/) | token | the order of magnitude, its change, the energy of the window behind | `max_bytes` six orders above the values beside it |
| [stress](../../reference/axes/stress/) | token | bracket depth, how long the brackets have stayed open, where they release | an argument list held open 17 tokens |
| [flow](../../reference/axes/flow/) | token | the slope, direction and momentum of a per-token signal | a climb with no single jump in it, and where it turns |
| [echo](../../reference/axes/echo/) | token | whether a token's content returns, how often, how far, how regularly | the session opened and never closed |
| [relation](../../reference/axes/relation/) | token pairs | enclosure, binding, adjacency and reuse between tokens, and the loops they make | two scopes equal up to renaming |
| [gravity](../../reference/axes/gravity/) | byte, token, supertoken | how much more or less often each type follows another at each gap than chance, and how hard a unit's past pushes it away | `sda` and `sdb` in a log that puts a number after every other `=` |
| [context](../../reference/axes/context/) | token | the other readings folded over what a token is read against | `latency = 12000` against the values `latency` held before |

Each reference page opens with the worked case in the last column, measured against the axes
that read the same input alike.

## How they differ

Three distinctions separate axes that look alike.

Grain. Spectral, observation and seam read bytes, so they read what the lexer cannot split, a
blob or a binary file. The token axes read a field as one unit whatever its width. Spectral's
period is of byte values and shape's of token silhouettes, so a fixed-width file has a period for
the first and rows of varying width have one for the second. Gravity reads whichever grain it is
asked for, and learns a field of its own at each.

Direction. Spectral reads each byte from the bytes before it, so a change shows in its reading a
few bytes late. Observation compares the past with the future at each byte, and seam reads both
directions at each cut. Gravity's strain reads a unit against the units before it, and its bound
weighs the pairs on both sides of a cut.

Reach. Magnitude, flow, spectral and observation read a window around or behind the position.
Echo and relation read the whole input, so a value's return and a name's scope are found at any
distance. Gravity learns from the whole input which types pull one another, and reads a unit
against those within 64 bytes, 32 tokens or 16 supertokens of it. Stress carries every bracket
still open, and context folds a window and five histories a token is related to.

## How they combine

With a pattern. Most axes have an anchor or a predicate a pattern names beside its atoms:
`\M{>k}` for magnitude, `@nested>k` for stress, `@ambiguous` for observation, `@seam` for seam,
`@novel` and `@echoed` for echo, `\F{...}` for spectral, `(?orbit:G ...)` for orbit, `@strain`,
`@bound` and `@kin` for gravity, and the relative forms `\N{>+1:...}` for context. They compose
like any atom; a word two brackets deep whose content recurs elsewhere:

```console
$ trex scan '@nested>=2 @echoed \W' --text 'x = 1; f(x, g(x, y))'
[14..15] "x"
```

With each other. Flow reads the trend of another axis's reading, magnitude by default and stress
or token length on request. Echo and shape read their keys under an orbit group. Context folds
magnitude, stress, spectral, echo, observation, seam and flow. Seam can take spectral's change
points at its cuts, and the region classification reads spectral texture, the lexer's encoded
tokens and shape period together.

```console
$ trex flow --signal stress --text 'f(g(h(x))) k(y)'
trex flow (signal stress): 15 bytes, 14 tokens, 1 reversal(s)
  peak momentum: 6 (rising)

$ trex echo --group case --text 'ERROR disk full; Error disk full; error disk full'
trex echo: 49 bytes, 19 tokens (9 keyed, 3 distinct), novelty 33.3%, echo rate 100.0%
  strongest echoes (top 3 of 3; first occurrence, count, period):
    ERROR                x3     period ~17 B
    disk                 x3     period ~17 B
    full                 x3     period ~17 B
```

The [axes reference](../../reference/axes/#where-each-axis-is-read) lists where each axis is read:
its command, cmdlet, Python function, Rust module and pattern form.
