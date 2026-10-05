---
title: Axes and tools
linkTitle: Axes and tools
weight: 40
---

Read a property of every token, its scale here, and query it from a pattern; meet the other
eleven axes one example each; then rewrite what a pattern finds.

## Read an axis

A token has a scale, a nesting depth, a texture and more; each is an [axis](../../reference/axes/),
read on its own or from a pattern. Magnitude is the order of magnitude of a token's value:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --text 'retries = 3 ; max_bytes = 5000000000'
trex magnitude: 36 bytes, 7 tokens, total energy 112.2, 3 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::magnitude::analyze_bytes(b"retries = 3 ; max_bytes = 5000000000");
let peak = field.frames.iter().map(|f| f.magnitude).fold(0.0f32, f32::max);
assert!((peak - 9.70).abs() < 0.01);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> trex.axes.magnitude("retries = 3 ; max_bytes = 5000000000").peak
MagnitudeFrame(offset=26, text='5000000000', magnitude=9.698969841003418)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexMagnitude 'retries = 3 ; max_bytes = 5000000000').Peak

Offset Text       Magnitude Gradient Energy
------ ----       --------- -------- ------
26     5000000000 9.69897   9.69897  112.2273
```
{{< /tab >}}
{{< /tabs >}}

## Query it from a pattern

`\M{>6}` is a token of more than six orders of magnitude:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\M{>6}' --text 'retries = 3 ; max_bytes = 5000000000'
[26..36] "5000000000"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\M{>6}").expect("valid pattern");
let text = "retries = 3 ; max_bytes = 5000000000";
assert_eq!(&text[trex::scan(&pat, text.as_bytes())[0].range()], "5000000000");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"\M{>6}").scan("retries = 3 ; max_bytes = 5000000000")]
['5000000000']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\M{>6}' -InputObject 'retries = 3 ; max_bytes = 5000000000' -Raw
5000000000
```
{{< /tab >}}
{{< /tabs >}}

## The other axes

Each of the other axes reads one more property of the input. One example of each follows; the
[axis pages](../../reference/axes/) give every reading, how to choose its thresholds, and how it
is built.

### Stress

Stress reads how deep in brackets a token is. `@nested>=2` holds two brackets deep or more:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@nested>=2 \W' --text 'f(a, g(b, h(c)))'
[7..8] "b"
[10..11] "h"
[12..13] "c"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = "f(a, g(b, h(c)))";
let pat = trex::parse(r"@nested>=2 \W").expect("valid pattern");
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["b", "h", "c"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"@nested>=2 \W").scan("f(a, g(b, h(c)))")]
['b', 'h', 'c']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@nested>=2 \W' -InputObject 'f(a, g(b, h(c)))' -Raw
b
h
c
```
{{< /tab >}}
{{< /tabs >}}

### Flow

Flow reads the trend of a signal along the input, magnitude by default, and where it turns. The
slope averages over the four tokens behind, so the turn after `729` reads at `27`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow --reversals --text '3 9 27 81 243 729 243 81 27 9'
trex flow (signal magnitude): 29 bytes, 10 tokens, 1 reversal(s)
  peak momentum: 6 (rising)
  reversals (1):
    @    25  27 9
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::flow::analyze_bytes(b"3 9 27 81 243 729 243 81 27 9", trex::flow::Signal::Magnitude);
assert_eq!(field.reversals, [25]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, f.offset) for f in trex.axes.flow("3 9 27 81 243 729 243 81 27 9").reversals]
[('27', 25)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexFlow '3 9 27 81 243 729 243 81 27 9').Reversals | Format-Table Offset, Text, Direction

Offset Text Direction
------ ---- ---------
    25 27     Falling
```
{{< /tab >}}
{{< /tabs >}}

### Shape

Shape reads each token's silhouette, its kind and a word's shape, and the period the silhouettes
repeat at. Rows of different widths read one silhouette, so the period is the row's five tokens:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex shape --text 'ann,12,3 bo,1200,1 cy,7,22 dee,450,4'
trex shape: 36 bytes, 20 tokens, 1 template region(s), 2 change-point(s)
  dominant shape-period: 5 tokens  (strength 1.00)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::shape::analyze_bytes(b"ann,12,3 bo,1200,1 cy,7,22 dee,450,4");
assert_eq!((field.n_tokens, field.shape_regions().len()), (20, 1));
assert!(field.frames.iter().any(|f| f.period == 5 && f.period_strength == 1.0));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> r = trex.axes.shape("ann,12,3 bo,1200,1 cy,7,22 dee,450,4")
>>> r.dominant_period, r.dominant_strength
(5, 1.0)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexShape 'ann,12,3 bo,1200,1 cy,7,22 dee,450,4').DominantPeriod
5
```
{{< /tab >}}
{{< /tabs >}}

### Spectral

Spectral reads the bytes as a signal, their class mix, entropy and period, and where they change.
With the token shapes it classifies the regions of an input; the code in this paragraph reads as
code, with the cut a few bytes in, where the bytes before it change:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex spectral --classify --text 'The report lists every host that answered within the window. a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} The totals go back to the collector before the next window opens.'
trex spectral (region classification: shape x spectral): 177 bytes, 3 regions
    [       0..65      ] prose      The report lists every host 
    [      65..117     ] code       =(b[j]*c[k]+d[i-1])>>2;if(a[
    [     117..177     ] prose      otals go back to the collect
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::shape::{RegionKind, classified_regions};

let text = b"The report lists every host that answered within the window. a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} The totals go back to the collector before the next window opens.";
let kinds: Vec<RegionKind> = classified_regions(text).into_iter().map(|(_, _, k)| k).collect();
assert_eq!(kinds, [RegionKind::Prose, RegionKind::Code, RegionKind::Prose]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> mixed = "The report lists every host that answered within the window. a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} The totals go back to the collector before the next window opens."
>>> [(g.kind, g.offset, g.length) for g in trex.axes.spectral(mixed, classify=True).regions]
[('prose', 0, 65), ('code', 65, 52), ('prose', 117, 60)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $mixed = 'The report lists every host that answered within the window. a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} The totals go back to the collector before the next window opens.'
PS> (Measure-TrexSpectral $mixed -Classify).Regions | Format-Table Offset, Length, Kind

Offset Length  Kind
------ ------  ----
     0     65 Prose
    65     52  Code
   117     60 Prose
```
{{< /tab >}}
{{< /tabs >}}

### Seam

Seam cuts where the bytes before a point stop predicting the bytes after it. With the built-in
English letter model it finds the words of a string with no spaces:

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
let words: Vec<&str> = field.segments().iter().map(|&(s, e)| &text[s..e]).collect();
assert_eq!(words, ["the", "birds", "fly", "over", "the", "houses"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [s.text for s in trex.axes.seam("thebirdsflyoverthehouses", english=True).segments]
['the', 'birds', 'fly', 'over', 'the', 'houses']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexSeam 'thebirdsflyoverthehouses' -English).Segments.Text -join ' '
the birds fly over the houses
```
{{< /tab >}}
{{< /tabs >}}

### Observation

Observation reads the input from behind a point, from ahead of it and around it, and marks where
the readings disagree. `@ambiguous:token` holds at the last value of a table that prose follows,
where what came before and what comes after read the point differently:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@ambiguous:token .' --text 'id,host,bytes
1,alpha,1024
2,beta,2048
The export finished without errors.'
[34..38] "2048"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = "id,host,bytes\n1,alpha,1024\n2,beta,2048\nThe export finished without errors.";
let pat = trex::parse(r"@ambiguous:token .").expect("valid pattern");
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["2048"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> export = "id,host,bytes\n1,alpha,1024\n2,beta,2048\nThe export finished without errors."
>>> [m.text for m in trex.Pattern(r"@ambiguous:token .").scan(export)]
['2048']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $export = "id,host,bytes`n1,alpha,1024`n2,beta,2048`nThe export finished without errors."
PS> Select-TrexMatch '@ambiguous:token .' -InputObject $export -Raw
2048
```
{{< /tab >}}
{{< /tabs >}}

### Echo

Echo reads whether a token's content returns, how often and how far away. `@novel` holds at a
content's first occurrence:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@novel \W' --text 'cat dog cat bird dog cat'
[0..3] "cat"
[4..7] "dog"
[12..16] "bird"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = "cat dog cat bird dog cat";
let pat = trex::parse(r"@novel \W").expect("valid pattern");
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["cat", "dog", "bird"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"@novel \W").scan("cat dog cat bird dog cat")]
['cat', 'dog', 'bird']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@novel \W' -InputObject 'cat dog cat bird dog cat' -Raw
cat
dog
bird
```
{{< /tab >}}
{{< /tabs >}}

### Orbit

Orbit reads a token up to a symmetry, such as case, notation or word shape; the
[orbit page](../../reference/axes/orbit/#the-groups) lists every group. `(?orbit:case ...)`
matches a literal in any case:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '(?orbit:case "cat")' --text 'Cat CAT dog cat'
[0..3] "Cat"
[4..7] "CAT"
[12..15] "cat"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = "Cat CAT dog cat";
let pat = trex::parse(r#"(?orbit:case "cat")"#).expect("valid pattern");
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["Cat", "CAT", "cat"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern('(?orbit:case "cat")').scan("Cat CAT dog cat")]
['Cat', 'CAT', 'cat']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '(?orbit:case "cat")' -InputObject 'Cat CAT dog cat' -Raw
Cat
CAT
cat
```
{{< /tab >}}
{{< /tabs >}}

### Relation

Relation reads the graph the brackets and names make. Its canonical form writes a name a bracket
binds as `#` and each use of it as `^k`, so two texts that differ only in their names read alike:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex relation --canonical --text '[ x ( x ) ]'
... the readings ...
  canonical form (alpha-equivalence):
    [ # ( ^0 ) ]
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::gauge::{alpha_equivalent, canonicalize_bytes};

assert_eq!(canonicalize_bytes(b"[ x ( x ) ]"), "[ # ( ^0 ) ]");
assert!(alpha_equivalent(b"[ x ( x ) ]", b"[ y ( y ) ]"));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [trex.axes.relation(text, canonical=True).canonical for text in ("[ x ( x ) ]", "[ y ( y ) ]")]
['[ # ( ^0 ) ]', '[ # ( ^0 ) ]']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexRelation '[ y ( y ) ]' -Canonical).Canonical
[ # ( ^0 ) ]
```
{{< /tab >}}
{{< /tabs >}}

### Gravity

Gravity reads which types of token the input puts near each other more or less often than chance,
and how hard a token's past pushes it away. In a log that puts a number after every `=` but two,
`@strain>0b` takes the two words in a number's place:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@strain>0b .' --text 'job=17 rows=1200 rows=1180 disk=sda rows=1210 rows=3590 disk=sdb rows=1190 job=17'
[32..35] "sda"
[61..64] "sdb"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = "job=17 rows=1200 rows=1180 disk=sda rows=1210 rows=3590 disk=sdb rows=1190 job=17";
let pat = trex::parse("@strain>0b .").expect("valid pattern");
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["sda", "sdb"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"@strain>0b .").scan("job=17 rows=1200 rows=1180 disk=sda rows=1210 rows=3590 disk=sdb rows=1190 job=17")]
['sda', 'sdb']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@strain>0b .' -InputObject 'job=17 rows=1200 rows=1180 disk=sda rows=1210 rows=3590 disk=sdb rows=1190 job=17' -Raw
sda
sdb
```
{{< /tab >}}
{{< /tabs >}}

### Context

Context compares a token with what surrounds it: the window before it, its column of a record,
or the values its key held before. `\N{>+1:k}` takes a value more than an order of
magnitude above the values bound to the same key:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '"latency":k "=" \N{>+1:k}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[64..79] "latency = 12000"  captures: k="latency"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = "latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;";
let pat = trex::parse(r#""latency":k "=" \N{>+1:k}"#).expect("valid pattern");
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["latency = 12000"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern('"latency":k "=" \\N{>+1:k}').scan("latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;")]
['latency = 12000']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '"latency":k "=" \N{>+1:k}' -InputObject 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;' -Raw
latency = 12000
```
{{< /tab >}}
{{< /tabs >}}

## Rewrite what a pattern finds

A template replaces each match: `${name}` renders a register and `${name:acc}` a slice of it:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex rewrite '\E:e' '[${e:domain}]' --text 'mail bob@x.com now'
mail [x.com] now
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\E:e").expect("valid pattern");
let tmpl = trex::Template::parse("[${e:domain}]", &pat.capture_names()).expect("valid template");
assert_eq!(trex::rewrite(&pat, &tmpl, b"mail bob@x.com now"), b"mail [x.com] now");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\E:e").rewrite("[${e:domain}]", "mail bob@x.com now")
'mail [x.com] now'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'mail bob@x.com now' | Edit-TrexText '\E:e' '[${e:domain}]'
mail [x.com] now
```
{{< /tab >}}
{{< /tabs >}}

## Where next

- [Beyond regex](../beyond-regex/) covers the rest of the pattern language.
- The [how-to guides](../../how-to/) solve one task each: grammars, prefilters, records, rules.
- The [reference](../../reference/) is the complete specification.
- The [explanation](../../explanation/) covers how the engine reads tokens in one pass.
