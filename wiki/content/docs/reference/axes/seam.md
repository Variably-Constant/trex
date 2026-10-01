---
title: Seam
linkTitle: Seam
weight: 20
---

# The seam axis

The points where an input divides into units: a cut falls where the bytes before a point stop
predicting the bytes after it, read in both directions, with no dictionary.

Source: [`src/seam.rs`](https://github.com/Variably-Constant/trex/blob/main/src/seam.rs).

## Reading the axis

With `--english` the reader scores each seam against order-3 letter counts taken from a
paragraph of English, so a short string with no spaces divides where English does not join its
letters:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex seam --english --text 'thebirdsflyoverthehouses'
trex seam: 24 bytes, order 3, 6 segments (bidirectional branching entropy)
  segments:
    [     0..3     ] "the"
    [     3..8     ] "birds"
    [     8..11    ] "fly"
    [    11..15    ] "over"
    [    15..18    ] "the"
    [    18..24    ] "houses"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::seam::{SeamConfig, analyze_with_model, english_model};

let text = "thebirdsflyoverthehouses";
let field = analyze_with_model(text.as_bytes(), &english_model(), &SeamConfig::default());
let units: Vec<&str> = field.segments().iter().map(|&(s, e)| &text[s..e]).collect();
assert_eq!(units, ["the", "birds", "fly", "over", "the", "houses"]);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexSeam 'thebirdsflyoverthehouses' -English).Segments | Format-Table

Offset Length Text
------ ------ ----
0      3      the
3      5      birds
8      3      fly
11     4      over
15     3      the
18     6      houses
```
{{< /tab >}}
{{< /tabs >}}

A cluster the paragraph does not hold can be cut inside a word:

```console
$ trex seam --english --text 'opentheshopdoorearly'
trex seam: 20 bytes, order 3, 7 segments (bidirectional branching entropy)
  segments:
    [     0..4     ] "open"
    [     4..7     ] "the"
    [     7..9     ] "sh"
    [     9..11    ] "op"
    [    11..13    ] "do"
    [    13..15    ] "or"
    [    15..20    ] "early"
```

Without a model the reader takes its statistics from the input itself. Every token start the
lexer finds is a cut, so spaced text divides at its tokens; a short string with no spaces has too
few repeats for its own counts to place the cuts:

```console
$ trex seam --text 'the cat sat'
trex seam: 11 bytes, order 3, 5 segments (bidirectional branching entropy)
  segments:
    [     0..3     ] "the"
    [     3..4     ] " "
    [     4..7     ] "cat"
    [     7..8     ] " "
    [     8..11    ] "sat"

$ trex seam --text 'thebirdsflyoverthehouses'
trex seam: 24 bytes, order 3, 5 segments (bidirectional branching entropy)
  segments:
    [     0..14    ] "thebirdsflyove"
    [    14..18    ] "rthe"
    [    18..19    ] "h"
    [    19..23    ] "ouse"
    [    23..24    ] "s"
```

| Flag | Effect |
|---|---|
| `--english` | score seams against the built-in English letter model |
| `--order K` | the longest context, in bytes; 3 by default |
| `--passes N` | confidence passes; one by default |
| `--segment` | the segments, which are printed by default |
| `--field` | the forward and backward branching entropy at every byte |
| `--limit N` | at most `N` rows of `--field` |
| `--json` | the cuts and both entropies as JSON |
| `--recover [FILE]` | score the cuts against known word boundaries; see [below](#boundary-recovery) |
| `--grain` | the [grain-separation](../orbit/#grain-separation) scores on a synthetic instruction stream |

`--compress` and the model-building flags need a build with the `compress` feature. In
PowerShell `-English`, `-Order` and `-Passes` match the flags, and `-Detail` adds every byte's
frame.

## In a pattern

`@seam:byte` is a zero-width anchor at a strong cut of the byte reading: a cut whose seam strength
is at least one standard deviation above the input's mean. `@seam`, which is `@seam:token`, runs
the same reader over the sequence of token kinds, and `@seam:super` over the sequence of construct
roles; the [pattern syntax](../../pattern-syntax/#axis-predicates-and-anchors) page has
each.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@seam:byte \W' --text 'aaaa bbbb aaaa bbbb cccc aaaa bbbb'
[10..14] "aaaa"
[20..24] "cccc"
[25..29] "aaaa"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"@seam:byte \W").expect("valid pattern");
let text = "aaaa bbbb aaaa bbbb cccc aaaa bbbb";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["aaaa", "cccc", "aaaa"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"@seam:byte \W").scan("aaaa bbbb aaaa bbbb cccc aaaa bbbb")]
['aaaa', 'cccc', 'aaaa']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@seam:byte \W' -InputObject 'aaaa bbbb aaaa bbbb cccc aaaa bbbb' -Raw
aaaa
cccc
aaaa
```
{{< /tab >}}
{{< /tabs >}}

## Boundary recovery

`--recover` removes the spaces from a text, segments what is left, and scores the cuts against
where the spaces were, a cut within one byte of a true boundary counting as a hit. `--compare-bpe`
scores byte-pair encoding with 200 merges trained on the same bytes. With no file the text is 400
words drawn from a vocabulary of 15 (`--words` and `--seed` set the draw), so each word recurs
about 27 times; with a file it is the file's own words, up to `--max-bytes` bytes (131,072 by
default).

```console
$ trex seam --recover --compare-bpe
trex seam --recover: 2113 bytes (400 words), 399 true boundaries, order 3, passes 1
  seam (no dictionary):   P=0.998 R=1.000 F1=0.999  (400 cuts, 399 hit)
  count-BPE (200 merges): P=1.000 R=0.411 F1=0.583  (164 piece boundaries)
  -> seam recovers word boundaries +0.416 F1 over count-BPE: the bidirectional
     predictive signal (past<->future branching entropy) BPE has no access to.

$ trex seam --recover --compare-bpe --max-bytes 16384 moby.txt
trex seam --recover: 16383 bytes, 3169 true boundaries, order 3, passes 1
  seam (no dictionary):   P=0.543 R=0.755 F1=0.632  (4405 cuts, 2393 hit)
  count-BPE (200 merges): P=0.365 R=0.939 F1=0.525  (8165 piece boundaries)
  -> seam recovers word boundaries +0.107 F1 over count-BPE: the bidirectional
     predictive signal (past<->future branching entropy) BPE has no access to.
```

On the first 131,072 bytes of Moby Dick seam reads P 0.644, R 0.825, F1 0.723 and count-BPE P
0.397, R 0.981, F1 0.565. Seam takes 0.58 seconds there; with `--compare-bpe` the command takes
7.6 minutes, against 7 seconds at 16 kB, the count-BPE baseline's time growing with the square
of the input.

## Data model

`SeamField`, keyed by byte offset:

| Field | Type | Meaning |
|---|---|---|
| `len` | `usize` | input length in bytes |
| `fwd_entropy` | `Vec<f32>` | forward branching entropy at each byte |
| `bwd_entropy` | `Vec<f32>` | backward branching entropy at each byte |
| `boundary` | `Vec<f32>` | the seam strength of a cut before each byte |
| `cuts` | `Vec<usize>` | the byte offsets that start a segment, ascending, from `0` |

| Method | Returns |
|---|---|
| `fwd_at(byte)` / `bwd_at(byte)` / `strength_at(byte)` | one reading, `0.0` past the input |
| `starts_segment(byte)` | whether a segment starts at `byte` |
| `segments()` | the segments as `[start, end)` spans |
| `internal_cuts()` | the cuts other than `0` |
| `strong_cuts()` | the cuts whose strength is at least one standard deviation above the mean |

`analyze(input)` reads with the default configuration and `analyze_with(input, cfg)` with a
`SeamConfig`. `analyze_with_model(input, &model, cfg)` scores against a `SeamModel`, which
`SeamModel::train(corpus, order)` builds and `english_model()` returns for English.
`analyze_tokens` and `analyze_supertokens` return the cuts over the token-kind and construct-role
sequences, and `analyze_symbols` reads any `u32` symbol stream.

| `SeamConfig` field | Default | Meaning |
|---|---|---|
| `order` | 3 | the longest context, in bytes, at most 8 |
| `cut_threshold` | 0.6 | a cut needs `boundary > mean + cut_threshold * std` |
| `min_seg` | 1 | minimum bytes between two cuts |
| `spectral_fusion` | `false` | add the [spectral](../spectral/) change-points as a bonus |
| `lexer_fusion` | `true` | make every lexer token start a cut |
| `min_count` | 2 | observations a context needs before it is used over a shorter one |
| `passes` | 0 | confidence passes; `0` runs one |
| `vigilance` | `None` | merge contexts whose follower distributions agree to at least this fraction |

## Algorithm

For each order from 1 to `order` the reader counts, over the whole input, the byte that follows
each context (forward) and the byte that precedes it (backward). At each byte it takes the longest
context seen at least `min_count` times and reads the Shannon entropy of its follower or
predecessor distribution. The seam strength of a cut before byte `t` is the forward entropy at
`t-1` plus the backward entropy at `t`.

A cut is placed at a local maximum of the strength above `mean + cut_threshold * std` and at least
`min_seg` bytes after the last cut. With `lexer_fusion` every token start is a cut and no entropy
cut falls inside a token of 12 bytes or fewer. With `spectral_fusion` each spectral change-point
adds 1.5 to the strength of the seams within a byte of it. A second pass and later ones add 2.0 to
both ends of every segment whose bytes recur elsewhere, from the first pass's strengths, and cut
again.

With a model the forward and backward readings are the surprisal, in bits, of the byte under the
model's counts, backing off to shorter contexts and to 8 bits for a context the model never saw.
Token starts are still cuts, and cuts may fall inside short tokens.

## Cost

Two counting passes, forward and backward, of `order` context updates a byte, then one backoff
lookup and one entropy read a byte in each direction. The contexts held are bounded by the
distinct `k`-grams of the input.
