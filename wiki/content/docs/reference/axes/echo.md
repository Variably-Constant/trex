---
title: Echo
linkTitle: Echo
weight: 90
---

# The echo axis

The recurrence substrate. Every other axis is a one-point function: it reads a local property
of a window around one position - its scale ([magnitude](../magnitude/)), its load
([stress](../stress/)), its texture ([spectral](../spectral/)), its form ([shape](../shape/)),
its symmetry class ([orbit](../orbit/)), its boundary ([seam](../seam/)), its vantage
([observation](../observation/)). Echo is the two-point function: for each token, does this
content occur *elsewhere*, how often, how far away, and how regularly?

Source: [`src/echo.rs`](https://github.com/Variably-Constant/trex/blob/main/src/echo.rs) · CLI: [`trex echo`](../cli/#echo).

## Thesis

Recurrence is content-addressed and unbounded-range, which no windowed reading can see.
Spectral autocorrelation finds a rhythm inside a bounded window; echo says "this span and
that span, four hundred kilobytes apart, are the same event." An identifier bound forty times
across a file, a log template returning every two kilobytes, a phrase that comes back in the
final chapter - all echo, and nothing else.

Each keyed token (words, numbers, quoted strings, and the typed literals; not punctuation or
brackets, which recur by grammar rather than by content) carries four readings:

| Field | Meaning |
|---|---|
| `count` | occurrences of this token's key in the document (1 = unique) |
| `back_lag` / `fwd_lag` | byte distance to the previous / next occurrence; no previous = **novel** |
| `period` | when a key recurs three or more times with regular spacing, the mean lag |
| `strength()` | the recurrence mass, `count - 1` |

The key is the token's text quotiented by an [orbit group](../orbit/), so recurrence composes
with the symmetry axis: under `Case`, `Whale` and `whale` are one echo; under `Shape`,
`1,22,3` and `4,55,6` are.

## Signal no other axis reaches

On Moby Dick (1.23 MB), the field reads in a quarter of a second:

```console
$ trex echo moby.txt
trex echo: 1234609 bytes, 477505 tokens (219476 keyed, 19083 distinct), novelty 8.7%, echo rate 96.1%
  strongest echoes (top 8 of 10435; first occurrence, count, period):
    the                  x13813
    of                   x6592
... and the six under them ...
```

Prose echoes without period (function words recur constantly but irregularly). Structured
data echoes *with* period - a template repeating every N bytes reads as a document-scale
pitch:

```console
$ trex echo --text 'the whale swam and the whale sang of the whale'
trex echo: 46 bytes, 19 tokens (10 keyed, 6 distinct), novelty 60.0%, echo rate 60.0%
  strongest echoes (top 2 of 2; first occurrence, count, period):
    the                  x3     period ~18 B
    whale                x3     period ~18 B
```

## Structural rhyme

Echo lifts to the [supertoken](../../explanation/architecture/) tower: `--super` keys each
unit by its role plus its token-kind silhouette, so two units that differ only in their
identifiers and values are the same structure. Six distinct words, 100% novelty, and still
one recurring structure:

```console
$ trex echo --super --text 'alpha: one
bravo: two
delta: six'
trex echo: 32 bytes, 14 tokens (6 keyed, 6 distinct), novelty 100.0%, echo rate 0.0%
  structural rhyme (top 1 of 1 recurring supertoken structures):
    kv:W:W                       x3    period ~11 B
```

That is a repeated config block, a log template, a stanza - the document's large-scale form,
read grammar-free.

## Pattern anchors

Two zero-width anchors query the field from inside a pattern, like `@seam` and `@ambiguous`:

| Anchor | Holds when |
|---|---|
| `@novel` | the current token is the FIRST occurrence of its content |
| `@echoed` | the current token's content recurs elsewhere (before or after) |

```console
$ trex scan '@novel \W' --text 'cat dog cat bird dog cat'
[0..3] "cat"
[4..7] "dog"
[12..16] "bird"

$ trex scan '@echoed \W' --text 'cat dog cat bird'
[0..3] "cat"
[8..11] "cat"
```

`@novel` finds each word's debut (deduplication, first-mention extraction); `@echoed` keeps
only content that repeats (template mining, hot-identifier surveys). The field is built once
per scan, only when the pattern uses one of the anchors, and each anchor check is an O(1)
lookup.

## Data model

`EchoField` holds one `EchoFrame` per token, index-aligned with the lexed stream, plus the
document summary: `keyed`, `distinct`, `novel`, `echoed` counts and the derived `novelty()` /
`echo_rate()` fractions. `EchoConfig` sets the orbit group, the minimum keyed length, and the
lag-regularity bound for a period (`max_period_cv`, default 0.3). `analyze_super` returns the
recurring supertoken structures (`SuperEcho`: key, count, period, first offset), most
frequent first.

## CLI

```
trex echo (FILE | --text STRING) [--field] [--orbit case|shape|notation] [--super] [--top K] [--limit N]
```

`--field` prints the per-token readings, all of them; `--limit N` bounds the dump when you
want less. `--orbit` selects the symmetry the key is quotiented by, `--super` reports the
recurring supertoken structures, and `--top` sizes the two ranked lists, whose headers always
state the bound and the total.

## Physics anchor and taxonomy

In the bundle picture the eight one-point axes are fibers over the byte/token base; echo is
the connection - the structure relating the fiber at one base point to the fiber at a distant
one. In field-theory terms the other axes are one-point functions and echo is the two-point
correlation. The engine already consumed recurrence ad hoc (`=name` back-references, `~"lit"`
content guards, the prefilter); echo is that signal promoted to a measured field.
