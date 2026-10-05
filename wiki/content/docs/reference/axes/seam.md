---
title: The seam axis
linkTitle: Seam
weight: 20
---

The points where an input divides into units: a cut is where the bytes before a point stop
predicting the bytes after it, read in both directions, with no dictionary.

Source: [`src/seam.rs`](https://github.com/Variably-Constant/trex/blob/main/src/seam.rs).

## What it reads that nothing else does

```text
thebirdsflyoverthehouses
```

The lexer reads one word, 24 bytes long, and spectral and observation find no change and no
contested point: every byte is a lowercase letter. Seam reads where the letters before a point
stop predicting the letters after it, and against an English letter model cuts the six words,
`the birds fly over the houses`. With no model it reads the same signal from the input's own
repeats: on 400 words with their spaces removed it recovers all 399 boundaries with 400 cuts,
where byte-pair encoding trained on the same bytes places 164 ([boundary
recovery](#boundary-recovery)).

## The readings

| Reading | Definition | Reads |
|---|---|---|
| forward entropy | the entropy, in bits, of the byte that follows the longest context before the byte the input holds at least twice | how freely the input goes on from here |
| backward entropy | the same read from the right: of the byte that precedes the longest context after the byte | how freely the input could have led here |
| strength | for a cut before byte `t`, the forward entropy at `t-1` plus the backward entropy at `t` | how sharply the past stops predicting the future |
| cut | a local maximum of the strength above the input's mean by `--cut-threshold` standard deviations, 0.6 by default, and every token start | where one unit ends and the next begins |
| segment | the bytes between two cuts | one unit |

With a model the two readings are the surprisal of each byte under the model's counts, in bits,
and the strength of a cut before `t` is the surprisal of byte `t` after the bytes before it plus
that of byte `t-1` before the bytes after it.

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
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> [s.text for s in trex.axes.seam("thebirdsflyoverthehouses", english=True).segments]
['the', 'birds', 'fly', 'over', 'the', 'houses']
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
| `--cut-threshold K` | a cut's strength is at least `K` standard deviations above the input's mean strength; 0.6 by default |
| `--segment` | the segments, which are printed by default |
| `--field` | the forward and backward branching entropy at every byte, and the strength of a cut before it |
| `--limit N` | at most `N` rows of `--field` and of the segments |
| `--json` | the cuts, both entropies and the strengths as JSON |
| `--recover [FILE]` | score the cuts against known word boundaries; see [below](#boundary-recovery) |
| `--grain-separation` | the [grain-separation](../orbit/#grain-separation) scores on a synthetic instruction stream |

`--compress` and the model-building flags need a build with the `compress` feature. In
PowerShell `-English`, `-Order`, `-Passes` and `-CutThreshold` match the flags, and `-Detail` adds
every byte's frame; in Python `english=True`, `order=`, `passes=`, `cut_threshold=` and
`detail=True` do.

### Every byte's reading

`--field` lists every byte with its forward and backward reading and the strength of a cut
before it, `-Detail` and `detail=True` the same frames. Under the English model `the` runs on
into `b` with a surprise: `e` after `th` costs 2.47 bits and `b` after `the` 8.29, so the cut
before `b` is the strongest of the four:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex seam --english --field --limit 4 --text 'thebirdsflyoverthehouses'
trex seam: 24 bytes, order 3, 6 segments (bidirectional branching entropy)
  per-byte branching entropy, and the strength of a cut before the byte:
         0 t  fwd=8.00 bwd=8.10 str=0.00
         1 h  fwd=2.47 bwd=8.01 str=10.57
         2 e  fwd=2.47 bwd=8.01 str=10.48
         3 b  fwd=8.29 bwd=6.43 str=16.31
    ... (+20 more bytes; raise --limit)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::seam::{SeamConfig, analyze_with_model, english_model};

let field = analyze_with_model(b"thebirdsflyoverthehouses", &english_model(), &SeamConfig::default());
let near = |v: f32, want: f32| (v - want).abs() < 0.001;
assert!(near(field.fwd_at(3), 8.295) && near(field.bwd_at(3), 6.426) && near(field.strength_at(3), 16.306));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> f = trex.axes.seam("thebirdsflyoverthehouses", english=True, detail=True).frames
>>> [(x.text, round(x.forward, 2), round(x.backward, 2), round(x.boundary, 2)) for x in f[1:4]]
[('h', 2.47, 8.01, 10.57), ('e', 2.47, 8.01, 10.48), ('b', 8.29, 6.43, 16.31)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexSeam 'thebirdsflyoverthehouses' -English -Detail).Frames | Select-Object -Skip 1 -First 3 | Format-Table Offset, Text, Forward, Backward, Boundary

Offset Text Forward Backward Boundary
------ ---- ------- -------- --------
     1 h       2.47     8.01    10.57
     2 e       2.47     8.01    10.48
     3 b       8.29     6.43    16.31
```
{{< /tab >}}
{{< /tabs >}}

`--json` gives the cuts, both readings and the strengths of the whole input as arrays.

## Choosing a threshold

A cut is a local maximum of the strength at least `--cut-threshold` standard deviations above
the input's mean strength. Read `--field` for the strengths before choosing: at 0.6 the English
model cuts the six words; at 0.2 it also cuts inside `birds` and `over`, where the strength has a
weaker local maximum; at 1.5 no maximum clears the bar and the string stays whole:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex seam --english --cut-threshold 0.2 --text 'thebirdsflyoverthehouses'
trex seam: 24 bytes, order 3, 8 segments (bidirectional branching entropy)
  segments:
    [     0..3     ] "the"
    [     3..6     ] "bir"
    [     6..8     ] "ds"
    [     8..11    ] "fly"
    [    11..13    ] "ov"
    [    13..15    ] "er"
    [    15..18    ] "the"
    [    18..24    ] "houses"

$ trex seam --english --cut-threshold 1.5 --text 'thebirdsflyoverthehouses'
trex seam: 24 bytes, order 3, 1 segments (bidirectional branching entropy)
  segments:
    [     0..24    ] "thebirdsflyoverthehouses"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::seam::{SeamConfig, analyze_with_model, english_model};

let text = "thebirdsflyoverthehouses";
let cut = |k: f32| -> Vec<&str> {
    let cfg = SeamConfig { cut_threshold: k, ..SeamConfig::default() };
    analyze_with_model(text.as_bytes(), &english_model(), &cfg).segments().iter().map(|&(s, e)| &text[s..e]).collect()
};
assert_eq!(cut(0.2), ["the", "bir", "ds", "fly", "ov", "er", "the", "houses"]);
assert_eq!(cut(1.5), ["thebirdsflyoverthehouses"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [s.text for s in trex.axes.seam("thebirdsflyoverthehouses", english=True, cut_threshold=0.2).segments]
['the', 'bir', 'ds', 'fly', 'ov', 'er', 'the', 'houses']
>>> [s.text for s in trex.axes.seam("thebirdsflyoverthehouses", english=True, cut_threshold=1.5).segments]
['thebirdsflyoverthehouses']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexSeam 'thebirdsflyoverthehouses' -English -CutThreshold 0.2).Segments.Text -join ' '
the bir ds fly ov er the houses

PS> (Measure-TrexSeam 'thebirdsflyoverthehouses' -English -CutThreshold 1.5).Segments.Text -join ' '
thebirdsflyoverthehouses
```
{{< /tab >}}
{{< /tabs >}}

Without a model every token start is a cut whatever the threshold, and no entropy cut is
inside a token of 12 bytes or fewer, so the threshold decides only the cuts inside longer tokens.
The anchors take their own threshold, in the same standard deviations,
as [In a pattern](#in-a-pattern) shows.

## Declarations

The command reads bytes, which no declaration changes, and takes none. `@seam` and `@seam:super`
read the token kinds and the supertoken roles a scan's lex gives, so in a pattern they follow
the scan's declarations as every token reading does.

## In a pattern

`@seam:byte` is a zero-width anchor at a strong cut of the byte reading: a cut whose seam strength
is at least one standard deviation above the input's mean. `@seam`, which is `@seam:token`, runs
the same reader over the sequence of token kinds, and `@seam:super` over the sequence of
[supertoken](../../../explanation/architecture/#supertokens) roles; the
[pattern syntax](../../pattern-syntax/#axis-predicates-and-anchors) page has
each.

A threshold after the anchor keeps the cuts whose strength is more than that many standard
deviations above the mean, or at least that many with `>=`: `@seam:byte` alone is
`@seam:byte>=1`, and `@seam` and `@seam:super` alone take every cut their reading finds.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@seam:byte \W' --text 'aaaa bbbb aaaa bbbb cccc aaaa bbbb'
[10..14] "aaaa"
[20..24] "cccc"
[25..29] "aaaa"

$ trex scan '@seam:byte>1.5 \W' --text 'aaaa bbbb aaaa bbbb cccc aaaa bbbb'
[25..29] "aaaa"

$ trex scan '@seam \W' --text 'let x = 1 ; let y = 2 ; print x ; print y ;'
[4..5] "x"
[34..39] "print"

$ trex scan '@seam>1 \W' --text 'let x = 1 ; let y = 2 ; print x ; print y ;'
[4..5] "x"

$ trex scan '@seam:super .' --text 'x = 1 ; y = 2 ; f(a, b) ; g(c, d) ; z = 3 ;'
[16..17] "f"
[26..27] "g"
[36..37] "z"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
let (spaced, code) = ("aaaa bbbb aaaa bbbb cccc aaaa bbbb", "let x = 1 ; let y = 2 ; print x ; print y ;");
assert_eq!(found(r"@seam:byte \W", spaced), ["aaaa", "cccc", "aaaa"]);
assert_eq!(found(r"@seam:byte>1.5 \W", spaced), ["aaaa"]);
assert_eq!(found(r"@seam \W", code), ["x", "print"]);
assert_eq!(found(r"@seam>1 \W", code), ["x"]);
assert_eq!(found("@seam:super .", "x = 1 ; y = 2 ; f(a, b) ; g(c, d) ; z = 3 ;"), ["f", "g", "z"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> spaced, code = "aaaa bbbb aaaa bbbb cccc aaaa bbbb", "let x = 1 ; let y = 2 ; print x ; print y ;"
>>> [m.text for m in trex.Pattern(r"@seam:byte \W").scan(spaced)]
['aaaa', 'cccc', 'aaaa']
>>> [m.text for m in trex.Pattern(r"@seam:byte>1.5 \W").scan(spaced)]
['aaaa']
>>> [m.text for m in trex.Pattern(r"@seam \W").scan(code)], [m.text for m in trex.Pattern(r"@seam>1 \W").scan(code)]
(['x', 'print'], ['x'])
>>> [m.text for m in trex.Pattern(r"@seam:super .").scan("x = 1 ; y = 2 ; f(a, b) ; g(c, d) ; z = 3 ;")]
['f', 'g', 'z']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@seam:byte \W' -InputObject 'aaaa bbbb aaaa bbbb cccc aaaa bbbb' -Raw
aaaa
cccc
aaaa

PS> Select-TrexMatch '@seam:byte>1.5 \W' -InputObject 'aaaa bbbb aaaa bbbb cccc aaaa bbbb' -Raw
aaaa

PS> Select-TrexMatch '@seam \W' -InputObject 'let x = 1 ; let y = 2 ; print x ; print y ;' -Raw
x
print

PS> Select-TrexMatch '@seam:super .' -InputObject 'x = 1 ; y = 2 ; f(a, b) ; g(c, d) ; z = 3 ;' -Raw
f
g
z
```
{{< /tab >}}
{{< /tabs >}}

The byte reading holds at the second `aaaa`, at `cccc` and at the `aaaa` after it, and only the
last is more than 1.5 standard deviations above the mean. The token reading cuts where the
sequence of token kinds stops predicting itself, and the supertoken reading where the sequence of
roles does, here at both calls and at the binding after them.

## Records

`--record seam` cuts an input into records at the seam's cuts, the reader's own statistics with
no model ([record units](../../records/#record-units)). Every token start is a cut, so on spaced
text the records are the words and the spaces between them, and over real files the unit makes
from 1.02 to 1.23 records a significant token, measured on ten 16 MB inputs of different kinds.
`auto` takes it or
[`bind`](../gravity/#records), whichever puts more of its cuts at the starts of lines.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ printf 'aaaa bbbb aaaa bbbb cccc aaaa bbbb' | trex lines 1..3 --record seam
aaaa bbbb
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;

let records = RecordUnit::Seam.records(b"aaaa bbbb aaaa bbbb cccc aaaa bbbb");
assert_eq!(records[..5], [(0, 4), (4, 5), (5, 9), (9, 10), (10, 14)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.records("aaaa bbbb aaaa bbbb cccc aaaa bbbb", "seam")[:5]
[(0, 4), (4, 5), (5, 9), (9, 10), (10, 14)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexRecord 'aaaa bbbb aaaa bbbb cccc aaaa bbbb' -Unit seam | Select-Object -First 5 | Format-Table Start, Length

Start Length
----- ------
    0      4
    4      1
    5      4
    9      1
   10      4
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
`analyze_tokens` and `analyze_supertokens` return the cuts over the token-kind and supertoken-role
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
cut is inside a token of 12 bytes or fewer. With `spectral_fusion` each spectral change-point
adds 1.5 to the strength of the seams within a byte of it. A second pass and later ones add 2.0 to
both ends of every segment whose bytes recur elsewhere, from the first pass's strengths, and cut
again.

With a model the forward and backward readings are the surprisal, in bits, of the byte under the
model's counts, backing off to shorter contexts and to 8 bits for a context the model never saw,
and the strength of a cut before `t` is the forward surprisal of byte `t` plus the backward
surprisal of byte `t-1`. Token starts are still cuts, and cuts may be inside short tokens.

Over the token kinds and the supertoken roles (`analyze_tokens`, `analyze_supertokens`, and the
`@seam` and `@seam:super` grains) the reader counts symbols in place of bytes, and the strength of
a cut before `t` is the fall of the forward entropy from `t-1` to `t` plus the rise of the
backward entropy, each where it is positive; every local maximum above zero is a cut.

## Cost

Two counting passes, forward and backward, of `order` context updates a byte, then one backoff
lookup and one entropy read a byte in each direction. The contexts held are bounded by the
distinct `k`-grams of the input.
