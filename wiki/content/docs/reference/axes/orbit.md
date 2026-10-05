---
title: The orbit axis
linkTitle: Orbit
weight: 40
---

The orbit axis reads each token as the canonical representative of its orbit under a symmetry
group: two spans that differ only by a symmetry of the group are one token.

Source: [`src/orbit.rs`](https://github.com/Variably-Constant/trex/blob/main/src/orbit.rs),
[`src/canon.rs`](https://github.com/Variably-Constant/trex/blob/main/src/canon.rs).

## What it reads that nothing else does

```text
ERROR disk full; Error disk full; error disk full
```

As written, the three spellings of the level are three different words, each seen once: echo finds
`disk` and `full` recurring three times and the level not at all. Orbit reads each token under a
symmetry, here case, and folds `ERROR`, `Error` and `error` into one orbit, six forms into four;
echo keyed under the same group then finds the level three times too. Under `notation`, `θ` and
`theta` are one symbol, and under `shape` an unseen word joins the words spelled like it.

## The groups

| Group | One orbit | Where |
|---|---|---|
| identity | the literal bytes | `OrbitGroup::Identity` |
| case | `Cat` = `cat` = `CAT` | `OrbitGroup::Case` |
| notation | `theta` = `\theta` = the glyph, case folded too | `OrbitGroup::Notation` (`canon::canon_symbol`) |
| shape | `cat` = `dog` = `bat` (`CVC`) | `OrbitGroup::Shape` |
| e8 | `aei` = `bcd` = `123`, one profile of character kinds | `OrbitGroup::E8` (`orbit::embed_e8`) |
| ip | `2001:DB8::1` = `2001:db8:0:0:0:0:0:1` | `OrbitGroup::Ip` |
| url | `HTTP://Example.Com:80` = `http://example.com/` | `OrbitGroup::Url` |
| time | `2024-01-02T03:04:05Z` = `2024-01-02T04:04:05+01:00`, one instant | `OrbitGroup::Time` |
| path | `C:\a\b` = `C:/a/b` | `OrbitGroup::Path` |
| fold | `Café` = `cafe` = the fullwidth `ｃａｆｅ` | `OrbitGroup::Fold` |
| numeric | `1e3` = `0x3e8` = `1_000` = `1000.0` | `OrbitGroup::Numeric` |
| a typed relation | `10.0.0.1` = `10.0.0.200` under `subnet/24`, `amy@corp.example` = `bob@corp.example` under `domain` | `OrbitGroup::Typed` |
| register renaming | code blocks up to a relabeling of registers | `canon::rename_invariant_sig` |

A word's shape is its consonant, vowel and digit pattern, invariant to which symbol fills each
slot, so an unseen word maps to a known shape orbit. `e8` reads a span as eight counts: its
vowels, consonants, digits, spaces, punctuation, capitals and other bytes, and a band for its
length. Two spans are one orbit when their counts are related by a symmetry of the E8 lattice,
and swapping the counts of two kinds is such a symmetry, so `aei`, `bcd` and `123` fold though
they share no character, while `cat` and `the`, two consonants and a vowel in either order, are a second
orbit. Under `shape` the five stay apart:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex orbit --group e8 --collapse --text 'aei bcd 123 cat the'
trex orbit --collapse (group e8): 5 raw forms -> 2 orbits (2.5x reduction)
    "E8[0,0,0,0,0,0,4,8]"  <-  ["123", "aei", "bcd"]
    "E8[0,0,0,0,0,2,4,6]"  <-  ["cat", "the"]

$ trex orbit --group shape --collapse --text 'aei bcd 123 cat the'
trex orbit --collapse (group shape): 5 raw forms -> 5 orbits (1x reduction)
    "CCC"
    "CCV"
    "CVC"
    "DDD"
    "VVV"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::{OrbitGroup, canonical};

assert_eq!(canonical(b"aei", OrbitGroup::E8), canonical(b"123", OrbitGroup::E8));
assert_eq!(canonical(b"cat", OrbitGroup::E8), canonical(b"the", OrbitGroup::E8));
assert_ne!(canonical(b"aei", OrbitGroup::E8), canonical(b"cat", OrbitGroup::E8));
assert_ne!(canonical(b"aei", OrbitGroup::Shape), canonical(b"123", OrbitGroup::Shape));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.orbit("aei bcd 123 cat the", group="e8")
>>> r.forms, r.orbits
(5, 2)
>>> [(c.orbit, c.forms) for c in r.classes]
[('E8[0,0,0,0,0,0,4,8]', ['123', 'aei', 'bcd']), ('E8[0,0,0,0,0,2,4,6]', ['cat', 'the'])]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexOrbit 'aei bcd 123 cat the' -Group e8).Classes | Format-Table

Orbit               Forms
-----               -----
E8[0,0,0,0,0,0,4,8] {123, aei, bcd}
E8[0,0,0,0,0,2,4,6] {cat, the}
```
{{< /tab >}}
{{< /tabs >}}

Every group in the table but register renaming is a value of `--group`, `-Group` and `group=`
on the orbit, echo and shape readings, and a pattern's `(?orbit:G ...)` takes each of them, as
the [pattern syntax](../../pattern-syntax/#binding-back-reference-and-symmetry) page lists. A
typed relation is any relation that page names, `day` and `hour` among them, with its width where
it takes one, as in `subnet/24`.

Every use calls `orbit::canonical(span, group)`: `collapse` folds symmetry-equivalent spans to
one token and reports how much that shrinks the vocabulary, `shape_boundaries` cuts where the
symbol kind changes, `matches` returns every span in a query's orbit, and `same_orbit` says
whether two spans are in one orbit.

## Reading the axis

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex orbit --same-as cat --group case --text 'the Cat and the CAT and a cat'
trex orbit --same-as "cat" (group case): 3 spans in the same orbit
    [     4..7     ] "Cat"
    [    16..19    ] "CAT"
    [    26..29    ] "cat"

$ trex orbit --collapse --group shape --text 'cat dog bat sat mat the fox'
trex orbit --collapse (group shape): 7 raw forms -> 2 orbits (3.5x reduction)
    "CCV"
    "CVC"  <-  ["bat", "cat", "dog", "fox", "mat", "sat"]

$ trex orbit --boundary --text 'cat123dog!!'
trex orbit --boundary: 3 symbol-kind transitions
    [     0..3     ] "cat"
    [     3..6     ] "123"
    [     6..9     ] "dog"
    [     9..11    ] "!!"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::{OrbitGroup, canonical};

assert_eq!(canonical(b"Cat", OrbitGroup::Case), canonical(b"CAT", OrbitGroup::Case));
assert_eq!(canonical(b"dog", OrbitGroup::Shape), "CVC");
assert_eq!(canonical(b"the", OrbitGroup::Shape), "CCV");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.orbit("the Cat and the CAT and a cat", group="case", same_as="cat")
>>> r.tokens, r.forms, r.orbits
(8, 6, 4)
>>> [(t.text, t.offset) for t in r.matches]
[('Cat', 4), ('CAT', 16), ('cat', 26)]
>>> [(c.orbit, c.forms) for c in trex.axes.orbit("cat dog bat sat mat the fox").classes]
[('CCV', ['the']), ('CVC', ['bat', 'cat', 'dog', 'fox', 'mat', 'sat'])]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexOrbit 'the Cat and the CAT and a cat' -Group case -SameAs cat | Select-Object Tokens, Forms, Orbits, Matches | Format-List

Tokens  : 8
Forms   : 6
Orbits  : 4
Matches : {Cat at 4, CAT at 16, cat at 26}

PS> (Measure-TrexOrbit 'cat dog bat sat mat the fox' -Group shape).Classes | Format-Table

Orbit Forms
----- -----
CCV   {the}
CVC   {bat, cat, dog, fox…}
```
{{< /tab >}}
{{< /tabs >}}

`--group G` names the group, any one [above](#the-groups) and `shape` by default. `--collapse`
prints the orbit table and the vocabulary reduction, `--boundary` the symbol-kind boundaries and
`--same-as QUERY` every span in the query's orbit; with no mode flag each token is listed with its
representative, and `--limit N` stops any of these lists after `N` rows and counts the rest. In
PowerShell `-Group` names the group and `-SameAs` lists the tokens in one
text's orbit; in Python `group=` and `same_as=` do, and `detail=True` adds every token with its
orbit and the runs of one kind of character.

## In a pattern

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '(?orbit:case "cat")' --text 'the Cat and the CAT and a cat'
[4..7] "Cat"
[16..19] "CAT"
[26..29] "cat"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r#"(?orbit:case "cat")"#).expect("valid pattern");
let text = "the Cat and the CAT and a cat";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["Cat", "CAT", "cat"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern('(?orbit:case "cat")').scan("the Cat and the CAT and a cat")]
['Cat', 'CAT', 'cat']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '(?orbit:case "cat")' -InputObject 'the Cat and the CAT and a cat' -Raw
Cat
CAT
cat
```
{{< /tab >}}
{{< /tabs >}}

## Choosing a threshold

Orbit has no numeric threshold: its one setting is the group, and the group decides what counts
as the same token. Choose the narrowest group that folds the variation you mean to ignore and no
more. `case` folds `Cat`, `CAT` and `cat` and keeps every other word its own; `shape` folds them
too, but would also fold `dog`, `bat` and any other consonant-vowel-consonant word into the same
orbit. `--collapse` shows what a group folds, so compare two before choosing:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex orbit --collapse --group case --text 'the Cat and the CAT and a cat'
trex orbit --collapse (group case): 6 raw forms -> 4 orbits (1.5x reduction)
    "a"
    "and"
    "cat"  <-  ["CAT", "Cat", "cat"]
    "the"

$ trex orbit --collapse --group shape --text 'the Cat and the CAT and a cat'
trex orbit --collapse (group shape): 6 raw forms -> 4 orbits (1.5x reduction)
    "CCV"
    "CVC"  <-  ["CAT", "Cat", "cat"]
    "V"
    "VCC"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::{OrbitGroup, canonical};

assert_eq!(canonical(b"CAT", OrbitGroup::Case), "cat");
assert_eq!(canonical(b"CAT", OrbitGroup::Shape), canonical(b"dog", OrbitGroup::Shape));
assert_ne!(canonical(b"CAT", OrbitGroup::Case), canonical(b"dog", OrbitGroup::Case));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(c.orbit, c.forms) for c in trex.axes.orbit("the Cat and the CAT and a cat", group="case").classes if len(c.forms) > 1]
[('cat', ['CAT', 'Cat', 'cat'])]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexOrbit 'the Cat and the CAT and a cat' -Group case).Classes | Where-Object { $_.Forms.Count -gt 1 } | Format-Table Orbit, Forms

Orbit Forms
----- -----
cat   {CAT, Cat, cat}
```
{{< /tab >}}
{{< /tabs >}}

Over this line the two groups fold the same three spellings, so either reads them as one; over a
line that also held `dog`, `shape` would fold it in with them and `case` would not.

## Declarations

A declared shape or kind changes what a token is, and so changes the span a group canonicalizes. Every
axis that reads tokens takes the declarations a scan takes: `--lib`, `--shape`, `--shape-after`,
`--kind`, `--let` and `--declare` on the command line, `-Library` in PowerShell and `lib=` in
Python ([pattern files](../../pattern-files/)); the [magnitude](../magnitude/#declarations) page
shows one at work.

## Grain separation

The register-renaming orbit separates a coarse grain (functions) from a fine grain (basic
blocks) in a token stream with no markers given: a recurring prologue at each function head is
one rename-invariant orbit whatever registers it uses. `trex seam --grain-separation` scores four
strategies against known grain on a synthetic stream:

```console
$ trex seam --grain-separation --words 60
trex seam --grain-separation: 1539 instructions, 60 functions, 210 blocks (synthetic)
  flat single-threshold  (function): P=0.198 R=0.900 F1=0.324  <- the conflation
  multi-scale coarse tier(function): P=0.417 R=0.250 F1=0.312
  multi-scale fine tier  (block)   : P=0.430 R=0.367 F1=0.396
  prologue-orbit         (function): P=1.000 R=1.000 F1=1.000
  UNIFIED  discovered coarse marker [8, 0, 1] (the recurring inter-function transition):
    function (texture discovers, marker completes, +/-1): P=1.000 R=0.983 F1=0.992
    block    (function anchors + texture)  : P=0.472 R=0.433 F1=0.452
```

The prologue orbit recovers function grain at F1 1.000 where a flat predictability threshold
reaches 0.324.
