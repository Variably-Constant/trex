---
title: Shape
linkTitle: Shape
weight: 30
---

# The shape axis

The structural form of the stream apart from its bytes: each token's silhouette - its token
class and word shape - the period at which the silhouettes repeat, and the regions that repeat
like a table. `foo(a, b)` and `bar(x, y)` are both `W ( W , W )`; `1,22,3` and `444,5,66` are
both `N , N , N`.

Source: [`src/shape.rs`](https://github.com/Variably-Constant/trex/blob/main/src/shape.rs).

## A width-free period

A field is one token whatever its width, so rows whose fields differ in width still repeat one
silhouette:

```console
$ trex shape --period --text '1,22,3
444,5,66
7,888,9'
trex shape: 23 bytes, 15 tokens, 1 template region(s), 2 change-point(s)
  dominant shape-period: 5 tokens  (strength 1.00)
  template regions (byte span -> shape-period):
    [      12..23      ] period 2    ,66 7,888,9
```

## Silhouette ladder

| Grain | Silhouette unit |
|---|---|
| byte | character class: digit, alpha, space, punct, bracket |
| token | token class and word shape (`W:snake`, `N`, `P:,`, `(`, `)`) |
| region | the token-class sequence, the structural template |

The reader works at the token grain and keys every frame by the token's byte span, so the byte,
spectral and pattern layers query it by byte offset.

## Reading the axis

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex shape --text 'foo(a, b) bar(c, d) baz(e, f)'
trex shape: 29 bytes, 18 tokens, 1 template region(s), 2 change-point(s)
  dominant shape-period: 6 tokens  (strength 1.00)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::shape::analyze_bytes(b"foo(a, b) bar(c, d) baz(e, f)");
assert_eq!(field.n_tokens, 18);
assert_eq!(field.shape_regions().len(), 1);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexShape 'foo(a, b) bar(c, d) baz(e, f)' | Select-Object Tokens, DominantPeriod, DominantStrength, Regions, ChangePoints | Format-List

Tokens           : 18
DominantPeriod   : 6
DominantStrength : 1
Regions          : {at 14, 15 long, period 4}
ChangePoints     : {3, 8}
```
{{< /tab >}}
{{< /tabs >}}

| Flag | Effect |
|---|---|
| `--classes` | per-token shape class, period and novelty |
| `--period` | the dominant shape period and the template regions |
| `--segment` | shape change-point boundaries |
| `--orbit G` | measure over an orbit quotient: `identity`, `case`, `notation` or `shape` |
| `--json` | the field as JSON |

In PowerShell `-Detail` adds a frame at every token. The `--texture` filter on a file walk and
the `shape` [record unit](../../records/#record-units) read the same regions.

## Data model

`ShapeClass` is a `u32` packing a token's structural identity: the `TokenKind` code in the high
bits, for a word the [`tokutil::shape`](https://github.com/Variably-Constant/trex/blob/main/src/tokutil.rs)
class (Pascal, snake, camel, SCREAM, short, word), and for punctuation the glyph byte. Two
tokens with the same class have the same shape.

`ShapeFrame`, one token's reading:

| Field | Type | Meaning |
|---|---|---|
| `class` | `u32` | the token's `ShapeClass` |
| `period` | `u16` | the dominant shape period of the neighbourhood in tokens, `0` for none |
| `period_strength` | `f32` | the normalized silhouette-autocorrelation peak, `[0,1]` |
| `novelty` | `f32` | shape n-gram surprise, `[0,1]`, `1` for a first sighting |

`ShapeField`, keyed by byte offset:

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | token count |
| `spans` | `Vec<(usize, usize)>` | byte span per token |
| `frames` | `Vec<ShapeFrame>` | one per token |
| `boundaries` | `Vec<usize>` | shape change-point byte offsets |

| Method | Returns |
|---|---|
| `class_at(byte)` | the shape class of the token covering `byte` |
| `period_at(byte)` | the dominant shape period and strength at `byte` |
| `in_template(byte)` | whether `byte` is inside a strong shape-period run |
| `shape_regions()` | the periodic byte spans and their period |

## Algorithms

One pass over the token stream (`analyze`):

- silhouette: `(TokenKind, word shape or punct glyph)` folded into a `u32`, O(1) a token;
- shape periodicity: centered autocorrelation of the shape-class sequence at token lags
  `1..max_lag`, the dominant period the arg-max with prominence, recomputed every `period_hop`
  tokens under a fixed operation budget;
- shape novelty: a rolling hash of the last `k` shape classes with a count map, `novelty = 1/(1
  + count_before_increment)`;
- shape change-point: a boundary where the local shape n-gram stops matching the running
  template, a novelty spike past an adaptive threshold, at least `cp_min_gap` tokens after the
  last cut.

## Region fusion

`shape::classified_regions(input)` fuses the spectral region texture with the shape period. Each
template region is first extended back token by token over the tokens that already repeat it:
a token is taken while its token kind equals the kind one period later, or one period earlier
past the input's end, and the period of tokens starting at it holds a kind other than a word
(`ShapeField::template_spans`). A `spectral::code_regions` span more than half of whose bytes
those spans cover becomes `RegionKind::Table(period)`, with the period of the span covering most
of it, and the rest keep their spectral texture. A five-row file of `alpha 10` lines is a
table though the reader finds its period only at the last row; a paragraph of prose holding a
short repeating phrase is prose.

```rust
pub enum RegionKind { Table(u16), Blob, Prose, Numeric, Code, Mixed }
```

`trex spectral --code-classify` renders it, so a tabular block reads as one `table` region.

## Over an orbit quotient

`shape::analyze_over(tokens, bytes, group)`, with `analyze_bytes_over` and `analyze_over_with`,
keys the silhouette on `orbit::canonical(.., group)`, so the period, novelty and change-points
are measured over a chosen [orbit](../orbit/) group: `identity` the literal token, `case` case
folded, `notation` notation folded, `shape` the consonant, vowel and digit pattern.

```console
$ trex shape --orbit identity --period --text 'the cat sat THE CAT SAT the cat sat THE CAT SAT'
trex shape: 47 bytes, 12 tokens, 1 template region(s), 2 change-point(s)
  dominant shape-period: 6 tokens  (strength 1.00)
  template regions (byte span -> shape-period):
    [      44..47      ] period 6    SAT

$ trex shape --orbit case --period --text 'the cat sat THE CAT SAT the cat sat THE CAT SAT'
trex shape: 47 bytes, 12 tokens, 1 template region(s), 1 change-point(s)
  dominant shape-period: 3 tokens  (strength 1.00)
  template regions (byte span -> shape-period):
    [      32..47      ] period 3    sat THE CAT SAT
```

The identity orbit reads period 6, the literal tokens repeating with the case cycle; the case
orbit reads period 3, the phrase with case folded away.

## Cost

| Kernel | Per-token cost | Bounded by |
|---|---|---|
| silhouette | O(1) | fixed |
| novelty | O(1) amortized | `novelty_window` |
| periodicity | O(1) amortized | adaptive `period_hop` under a fixed operation budget |
| change-point | O(1) | fixed |
