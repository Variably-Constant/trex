---
title: Orbit
linkTitle: Orbit
weight: 40
---

# The orbit axis

Each token as the canonical representative of its orbit under a symmetry group: two spans that
differ only by a symmetry of the group are one token.

Source: [`src/orbit.rs`](https://github.com/Variably-Constant/trex/blob/main/src/orbit.rs),
[`src/canon.rs`](https://github.com/Variably-Constant/trex/blob/main/src/canon.rs).

## The groups

| Group | One orbit | Where |
|---|---|---|
| identity | the literal bytes | `OrbitGroup::Identity` |
| case | `Cat` = `cat` = `CAT` | `OrbitGroup::Case` |
| notation | `theta` = `\theta` = the glyph, case folded too | `OrbitGroup::Notation` (`canon::canon_symbol`) |
| shape | `cat` = `dog` = `bat` (`CVC`) | `OrbitGroup::Shape` |
| register renaming | code blocks up to a relabelling of registers | `canon::rename_invariant_sig` |

A word's shape is its consonant, vowel and digit pattern, invariant to which symbol fills each
slot, so an unseen word maps to a known shape orbit. In a pattern `(?orbit:G ...)` takes these
groups and every typed relation (`ip`, `url`, `time`, `path`, `fold`, `numeric`, `subnet/24`,
`domain` and the rest), as the [pattern syntax](../../pattern-syntax/#binding-back-reference-and-symmetry)
page lists them.

Every use reads `orbit::canonical(span, group)`: `collapse` folds symmetry-equivalent spans to
one token and reports the vocabulary the quotient leaves, `shape_boundaries` cuts where the
symbol kind changes, and `matches` returns every span in a query's orbit, `same_orbit` being
the membership test.

## Reading the axis

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex orbit --match cat --group case --text 'the Cat and the CAT and a cat'
trex orbit --match "cat" (group case): 3 spans in the same orbit
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

`--group identity|case|notation|shape` names the group (`shape` by default), `--collapse` prints
the orbit table and the vocabulary reduction, `--boundary` the symbol-kind boundaries and
`--match QUERY` every span in the query's orbit; with no mode flag each token is listed with its
representative. In PowerShell `-Group` names the group and `-SameAs` lists the tokens in one
text's orbit.

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

## Grain separation

The register-renaming orbit separates a coarse grain (functions) from a fine grain (basic
blocks) in a token stream with no markers given: a recurring prologue at each function head is
one rename-invariant orbit whatever registers it uses. `trex seam --grain` scores four strategies
against known grain on a synthetic stream:

```console
$ trex seam --grain --words 60
trex seam --grain: 1539 instructions, 60 functions, 210 blocks (synthetic)
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
