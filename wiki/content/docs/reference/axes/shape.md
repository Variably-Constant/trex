---
title: The shape axis
linkTitle: Shape
weight: 30
---

The shape axis reads the form of the token stream apart from its text. Each token has a
silhouette, its shape class: the token's kind, with a word's shape or a punctuation mark's
glyph. `foo(a, b)` and `bar(x, y)` read alike, `W ( W , W )`, and so do the rows `ann,12,3` and
`bo,1200,1`. Over the silhouettes the axis reads the period they repeat at and how strongly, how
new each run of three silhouettes is, where the stream's form changes, and the regions that
repeat like a table.

Source: [`src/shape.rs`](https://github.com/Variably-Constant/trex/blob/main/src/shape.rs).

## What it reads that nothing else does

```console
$ cat scores.csv
name,score,rank
ann,12,3
bo,1200,1
cyrus,7,22
dee,450,4
eve,3,9
```

The rows are of different widths, so no byte repeats at a fixed distance and spectral reads no
period in them. A field is one token whatever its width, so every row reads one silhouette, a
word, a comma, a number, a comma and a number, and shape reads it every five tokens:

```console
$ trex spectral scores.csv | head -3
trex spectral: 64 bytes, 4 frames (hop 16), 0 change-points
  entropy   min 0.56  mean 0.64  max 0.69  (normalized bits/byte)
  period    none detected (no strong byte-periodicity)

$ trex shape scores.csv
trex shape: 64 bytes, 30 tokens, 1 template region(s), 4 change-point(s)
  dominant shape-period: 5 tokens  (strength 0.84)
```

The same reading finds a table whose columns do not line up and a list of records written one
after another. Every example on this page reads that file, except where a reading needs an
input of its own.

## The readings

| Reading | Definition | Reads |
|---|---|---|
| silhouette | the token's kind, with a word's shape (Pascal, snake, camel, SCREAM, short for four characters or fewer, or word) or a punctuation mark's glyph | what form of token is here |
| period | the lag, in tokens, at which the silhouettes of the window behind the token repeat most, where at least half of them do; 0 otherwise | the length of a repeating record |
| period strength | the share of the window's silhouettes equal to the one a period before | how regularly it repeats |
| novelty | `1 / (1 + n)`, `n` the times the run of three silhouettes ending at the token was seen before | how new the local form is |
| change point | a token whose run of three silhouettes is new, at least four tokens after the last change point | where the form changes |
| template region | a run of tokens whose period strength reaches the template strength, 0.6 by default | where the input repeats like a table |

The window is a token and the 256 before it, and the period is read again every eight tokens, so
a token's period is the one its own stretch of the input repeats at.

## Reading the axis

The summary counts the tokens, the template regions and the change points, and gives the
strongest period:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex shape scores.csv
trex shape: 64 bytes, 30 tokens, 1 template region(s), 4 change-point(s)
  dominant shape-period: 5 tokens  (strength 0.84)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let csv = b"name,score,rank\nann,12,3\nbo,1200,1\ncyrus,7,22\ndee,450,4\neve,3,9\n";
let field = trex::shape::analyze_bytes(csv);
assert_eq!((field.n_tokens, field.shape_regions().len(), field.boundaries.len()), (30, 1, 4));
let strongest = field
    .frames
    .iter()
    .filter(|f| f.period > 0)
    .max_by(|a, b| a.period_strength.total_cmp(&b.period_strength))
    .expect("a period");
assert_eq!(strongest.period, 5);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.shape(path="scores.csv")
>>> r.tokens, r.dominant_period, round(r.dominant_strength, 2)
(30, 5, 0.84)
>>> [(g.offset, g.length, g.period) for g in r.regions], r.change_points
([(40, 23, 5)], [4, 16, 23, 35])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexShape -Path ./scores.csv | Select-Object Tokens, DominantPeriod, DominantStrength, Regions, ChangePoints | Format-List

Tokens           : 30
DominantPeriod   : 5
DominantStrength : 0.84
Regions          : {at 40, 23 long, period 5}
ChangePoints     : {4, 16, 23, 35}
```
{{< /tab >}}
{{< /tabs >}}

### Every token's reading

`--classes` lists every token with its silhouette, its period, the period's strength and its
novelty. In PowerShell `-Detail` and in Python `detail=True` add the same frames to the report,
the silhouette under `class_` in Python, since `class` is a Python keyword:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex shape --classes --limit 18 scores.csv
trex shape: 64 bytes, 30 tokens, 1 template region(s), 4 change-point(s)
  dominant shape-period: 5 tokens  (strength 0.84)
  per-token (token -> class | period | strength | novelty):
    name           cls=00010005 per=0   str=0.00 nov=1.00
    ,              cls=0008002c per=0   str=0.00 nov=1.00
    score          cls=00010000 per=0   str=0.00 nov=1.00
    ,              cls=0008002c per=0   str=0.00 nov=1.00
    rank           cls=00010005 per=0   str=0.00 nov=1.00
    ann            cls=00010005 per=0   str=0.00 nov=1.00
    ,              cls=0008002c per=0   str=0.00 nov=1.00
    12             cls=00000000 per=0   str=0.00 nov=1.00
    ,              cls=0008002c per=0   str=0.29 nov=1.00
    3              cls=00000000 per=0   str=0.29 nov=1.00
    bo             cls=00010005 per=0   str=0.29 nov=1.00
    ,              cls=0008002c per=0   str=0.29 nov=1.00
    1200           cls=00000000 per=0   str=0.29 nov=0.50
    ,              cls=0008002c per=0   str=0.29 nov=0.50
    1              cls=00000000 per=0   str=0.29 nov=0.50
    cyrus          cls=00010000 per=0   str=0.29 nov=1.00
    ,              cls=0008002c per=5   str=0.75 nov=1.00
    7              cls=00000000 per=5   str=0.75 nov=1.00
    ... (+12 more tokens; raise --limit)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let csv = b"name,score,rank\nann,12,3\nbo,1200,1\ncyrus,7,22\ndee,450,4\neve,3,9\n";
let field = trex::shape::analyze_bytes(csv);
let (ann, cyrus, comma) = (&field.frames[5], &field.frames[15], &field.frames[16]);
assert_eq!((ann.class, cyrus.class, comma.class), (0x0001_0005, 0x0001_0000, 0x0008_002c));
assert_eq!((cyrus.period, comma.period), (0, 5));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> d = trex.axes.shape(path="scores.csv", detail=True)
>>> [(f.text, hex(f.class_), f.period, round(f.period_strength, 2), round(f.novelty, 2)) for f in d.frames[14:18]]
[('1', '0x0', 0, 0.29, 0.5), ('cyrus', '0x10000', 0, 0.29, 1.0), (',', '0x8002c', 5, 0.75, 1.0), ('7', '0x0', 5, 0.75, 1.0)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexShape -Path ./scores.csv -Detail).Frames | Select-Object -Skip 14 -First 4 | Format-Table Offset, Text, Class, Period, PeriodStrength, Novelty

Offset Text   Class Period PeriodStrength Novelty
------ ----   ----- ------ -------------- -------
    33 1          0      0           0.29    0.50
    35 cyrus  65536      0           0.29    1.00
    40 ,     524332      5           0.75    1.00
    41 7          0      5           0.75    1.00
```
{{< /tab >}}
{{< /tabs >}}

The silhouette prints in hexadecimal, the token kind's code in the high four digits and in the
low four a word's shape or a punctuation mark's code point. `name`, `rank` and `ann` read
`00010005`, a word of four characters or fewer, and `score` and `cyrus` `00010000`, a longer
word; the comma reads `0008002c`, punctuation with the code point of `,`, and every number
`00000000`, whatever its value. The period is read first at the seventeenth token, where the window behind holds enough rows to
repeat, and its strength climbs to 0.84 by the last row. `--json` writes the same frames, with the
dominant period, the change points and the regions, as one JSON object:

```console
$ trex shape --json --text 'a = 1 ; b = 2'
{"n_tokens":7,"dominant_period":0,"dominant_strength":0.0000,"boundaries":[2,10],"regions":[],"frames":[{"start":0,"end":1,"class":65541,"period":0,"period_strength":0.0000,"novelty":1.0000},{"start":2,"end":3,"class":524349,"period":0,"period_strength":0.0000,"novelty":1.0000},{"start":4,"end":5,"class":0,"period":0,"period_strength":0.0000,"novelty":1.0000},{"start":6,"end":7,"class":524347,"period":0,"period_strength":0.0000,"novelty":1.0000},{"start":8,"end":9,"class":65541,"period":0,"period_strength":0.0000,"novelty":1.0000},{"start":10,"end":11,"class":524349,"period":0,"period_strength":0.0000,"novelty":1.0000},{"start":12,"end":13,"class":0,"period":0,"period_strength":0.0000,"novelty":0.5000}]}
```

### Template regions

`--period` lists the template regions, each as its byte span and the period it repeats at:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex shape --period scores.csv
trex shape: 64 bytes, 30 tokens, 1 template region(s), 4 change-point(s)
  dominant shape-period: 5 tokens  (strength 0.84)
  template regions (byte span -> shape-period):
    [      40..63      ] period 5    ,7,22 dee,450,4 eve,3,9
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::shape::analyze_bytes(b"name,score,rank\nann,12,3\nbo,1200,1\ncyrus,7,22\ndee,450,4\neve,3,9\n");
assert_eq!(field.shape_regions(), [(40, 63, 5)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(g.offset, g.length, g.period) for g in trex.axes.shape(path="scores.csv").regions]
[(40, 23, 5)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexShape -Path ./scores.csv).Regions | Format-Table Offset, Length, Period

Offset Length Period
------ ------ ------
    40     23      5
```
{{< /tab >}}
{{< /tabs >}}

The region opens at byte 40, inside the fourth line, at the first token whose window has read
the period, and ends with the last token that repeats the token a period before it. A region
keeps the period it opened at. The [region classification](#region-fusion) extends each run
back over the rows that already repeat it, and reads the whole file as one table.

### Change points

`--segment` lists the change points as byte offsets, each the start of a token whose run of
three silhouettes the input has not shown before:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex shape --segment scores.csv
trex shape: 64 bytes, 30 tokens, 1 template region(s), 4 change-point(s)
  dominant shape-period: 5 tokens  (strength 0.84)
  shape change-points (4):  4 16 23 35
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::shape::analyze_bytes(b"name,score,rank\nann,12,3\nbo,1200,1\ncyrus,7,22\ndee,450,4\neve,3,9\n");
assert_eq!(field.boundaries, [4, 16, 23, 35]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.axes.shape(path="scores.csv").change_points
[4, 16, 23, 35]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexShape -Path ./scores.csv).ChangePoints
4
16
23
35
```
{{< /tab >}}
{{< /tabs >}}

Every run is new at the start of an input, so the first change point is at its second token
and the next ones at least four tokens apart: at `ann`, the first word after the header; at `3`,
the first number after a comma after a number; and at `cyrus`, the one name longer than four
letters, which makes a run of three silhouettes that no earlier row had.

### Over an orbit quotient

`--group G` reads each token's silhouette as the token's representative under an
[orbit](../orbit/) group, so the period, the novelty and the change points are of the folded
stream: `identity` the literal token, `case` case folded, `notation` notation folded, `shape`
the consonant, vowel and digit pattern, and each other group in the
[orbit table](../orbit/#the-groups), `e8` and the typed relations among them, folded as that table
describes. `-Group` and `group=` name it in PowerShell and Python.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex shape --group identity --period --text 'the cat sat THE CAT SAT the cat sat THE CAT SAT'
trex shape: 47 bytes, 12 tokens, 1 template region(s), 2 change-point(s)
  dominant shape-period: 6 tokens  (strength 1.00)
  template regions (byte span -> shape-period):
    [      44..47      ] period 6    SAT

$ trex shape --group case --period --text 'the cat sat THE CAT SAT the cat sat THE CAT SAT'
trex shape: 47 bytes, 12 tokens, 1 template region(s), 1 change-point(s)
  dominant shape-period: 3 tokens  (strength 1.00)
  template regions (byte span -> shape-period):
    [      32..47      ] period 3    sat THE CAT SAT
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::orbit::OrbitGroup;

let text = b"the cat sat THE CAT SAT the cat sat THE CAT SAT";
let dominant = |group| {
    let field = trex::shape::analyze_bytes_over(text, group);
    let strongest = field
        .frames
        .iter()
        .filter(|f| f.period > 0)
        .max_by(|a, b| a.period_strength.total_cmp(&b.period_strength))
        .expect("a period");
    strongest.period
};
assert_eq!((dominant(OrbitGroup::Identity), dominant(OrbitGroup::Case)), (6, 3));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> text = "the cat sat THE CAT SAT the cat sat THE CAT SAT"
>>> [(g, trex.axes.shape(text, group=g).dominant_period) for g in ("identity", "case")]
[('identity', 6), ('case', 3)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $text = 'the cat sat THE CAT SAT the cat sat THE CAT SAT'
PS> 'identity', 'case' | ForEach-Object { Measure-TrexShape $text -Group $_ } | Format-Table Group, DominantPeriod

Group    DominantPeriod
-----    --------------
identity              6
case                  3
```
{{< /tab >}}
{{< /tabs >}}

The identity group reads period 6, the literal tokens repeating with the case cycle; the case
group reads period 3, the phrase with its case folded away.

## Choosing a threshold

A template region holds the tokens whose period strength reaches the template strength,
`--template-strength`, 0.6 by default. Read `--classes` for the strengths before choosing: in
the table they climb from 0.75 to 0.84 once the period is read, and the tokens before it read
0.29 at most, with no period. The default is between them; a threshold above every reading forms
no region:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex shape --period --template-strength 0.9 scores.csv
trex shape: 64 bytes, 30 tokens, 0 template region(s), 4 change-point(s)
  dominant shape-period: 5 tokens  (strength 0.84)
  template regions (byte span -> shape-period):
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::shape::{ShapeConfig, analyze_with};

let csv = b"name,score,rank\nann,12,3\nbo,1200,1\ncyrus,7,22\ndee,450,4\neve,3,9\n";
let cfg = ShapeConfig { template_strength: 0.9, ..ShapeConfig::default() };
assert!(analyze_with(&trex::tokutil::lex_sig(csv), csv, &cfg).shape_regions().is_empty());
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.axes.shape(path="scores.csv", template_strength=0.9).regions
[]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexShape -Path ./scores.csv -TemplateStrength 0.9).Regions.Count
0
```
{{< /tab >}}
{{< /tabs >}}

The comparison is inclusive: a token whose strength equals the threshold is in the region, and a
lower threshold admits runs that repeat less regularly.

## Declarations

A declared shape or kind changes what a token is, and so changes the silhouettes. A name and its
score declared as one token make every row three tokens, and the period reads 3:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex shape --shape 'entry = `[a-z]{1,5},[0-9]{1,4}`' scores.csv
trex shape: 64 bytes, 20 tokens, 1 template region(s), 3 change-point(s)
  dominant shape-period: 3 tokens  (strength 0.76)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let csv = b"name,score,rank\nann,12,3\nbo,1200,1\ncyrus,7,22\ndee,450,4\neve,3,9\n";
let mut shapes = trex::ShapeSet::new();
shapes.declare("entry = `[a-z]{1,5},[0-9]{1,4}`", trex::Precedence::Before).expect("a bounded shape");
let toks: Vec<trex::token::Token> = trex::lexer::lex_with_shapes(csv, &trex::lexer::blob_runs(csv), &shapes, 0)
    .into_iter()
    .filter(|t| t.is_significant())
    .collect();
let field = trex::shape::analyze(&toks, csv);
assert_eq!(field.n_tokens, 20);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> lib = trex.Library()
>>> lib.shape("entry", r"[a-z]{1,5},[0-9]{1,4}")
>>> s = trex.axes.shape(path="scores.csv", lib=lib)
>>> s.tokens, s.dominant_period, round(s.dominant_strength, 2)
(20, 3, 0.76)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $lib = New-TrexLibrary
PS> Register-TrexAtom entry -Shape '[a-z]{1,5},[0-9]{1,4}' -Library $lib
PS> Measure-TrexShape -Path ./scores.csv -Library $lib | Select-Object Tokens, DominantPeriod | Format-List

Tokens         : 20
DominantPeriod : 3
```
{{< /tab >}}
{{< /tabs >}}

Every axis that reads tokens takes the declarations a scan takes: `--lib`, `--shape`,
`--shape-after`, `--kind`, `--let` and `--declare` on the command line, `-Library` in PowerShell
and `lib=` in Python ([pattern files](../../pattern-files/)).

## In a pattern and in records

A pattern spells a silhouette with `#"..."`: `W` a word, `N` a number, `.` any token and any
other character itself, so `#"W,N,N"` finds each row of the table by its form alone.
`--record shape` cuts an input into records at the shape change points
([record units](../../records/#record-units)); the last record of the table holds the three rows
from `cyrus` on:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '#"W,N,N"' scores.csv
[16..24] "ann,12,3"
[25..34] "bo,1200,1"
[35..45] "cyrus,7,22"
[46..55] "dee,450,4"
[56..63] "eve,3,9"

$ trex lines 5..5 --record shape scores.csv
cyrus,7,22
dee,450,4
eve,3,9
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;

let csv = "name,score,rank\nann,12,3\nbo,1200,1\ncyrus,7,22\ndee,450,4\neve,3,9\n";
let pat = trex::parse(r#"#"W,N,N""#).expect("valid pattern");
let rows: Vec<&str> = trex::scan(&pat, csv.as_bytes()).iter().map(|s| &csv[s.range()]).collect();
assert_eq!(rows, ["ann,12,3", "bo,1200,1", "cyrus,7,22", "dee,450,4", "eve,3,9"]);
assert_eq!(RecordUnit::Shape.records(csv.as_bytes()), [(0, 4), (4, 16), (16, 23), (23, 35), (35, 64)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> csv = open("scores.csv").read()
>>> [m.text for m in trex.Pattern(r'#"W,N,N"').scan(csv)]
['ann,12,3', 'bo,1200,1', 'cyrus,7,22', 'dee,450,4', 'eve,3,9']
>>> trex.records(csv, "shape")
[(0, 4), (4, 16), (16, 23), (23, 35), (35, 64)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '#"W,N,N"' -Path ./scores.csv -Raw
ann,12,3
bo,1200,1
cyrus,7,22
dee,450,4
eve,3,9

PS> Get-TrexRecord (Get-Content ./scores.csv -Raw) -Unit shape | Select-Object Start, Length

Start Length
----- ------
    0      4
    4     12
   16      7
   23     12
   35     29
```
{{< /tab >}}
{{< /tabs >}}

The silhouette reads token kinds alone, so `#"W,N,N"` takes a row whatever its name's length,
where the axis's own silhouette tells a short word from a long one.

Lines are grouped into templates by their silhouette, as `trex templates` groups them, and
`@shape:rare` holds at every token of a line whose template covers fewer lines than the mean
template does. The table has two templates, the header's on one line and the rows' on five, so
the mean template covers three lines and the header is the rare one:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@shape:rare \W' scores.csv
[0..4] "name"
[5..10] "score"
[11..15] "rank"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let csv = "name,score,rank\nann,12,3\nbo,1200,1\ncyrus,7,22\ndee,450,4\neve,3,9\n";
let pat = trex::parse(r"@shape:rare \W").expect("valid pattern");
let rare: Vec<&str> = trex::scan(&pat, csv.as_bytes()).iter().map(|s| &csv[s.range()]).collect();
assert_eq!(rare, ["name", "score", "rank"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"@shape:rare \W").scan(csv)]
['name', 'score', 'rank']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@shape:rare \W' -Path ./scores.csv -Raw
name
score
rank
```
{{< /tab >}}
{{< /tabs >}}

The cut can be written instead of the mean: `@shape:rare<5` holds on a line whose template covers
fewer than five lines, and `@shape:rare<1%` on one whose template covers less than one percent of
the lines. Read it off the counts `trex templates` prints beside each template, here 5 for the
rows and 1 for the header: a cut from `<2` to `<5`, or from `<17%` to `<83%` of the six lines,
keeps the header alone, `<6` takes the rows too, and `<16%` keeps nothing. `trex templates --rare
--cut` lists the rare templates themselves
([summarize a log by templates](../../../how-to/summarize-a-log-by-templates/)).

## Region fusion

`shape::classified_regions(input)` fuses the spectral region texture with the lexer's tokens and
the shape period, applying three tests to each `spectral::code_regions` span in turn.

First, encoded text: a span more than half of whose bytes the lexer reads as base64, hash or
hex tokens, or as runs its blob gate collapses, is `RegionKind::Blob`. That holds however the
encoding is laid out, so base64 wrapped at 76 columns, a list of SHA-256 digests one to a line
and a PEM certificate bundle are blobs, though each repeats one token kind line after line.

Second, tables. Each template region is extended back token by token over the tokens that
already repeat it: a token is taken while its token kind equals the kind one period later, or
one period earlier past the input's end, and the period of tokens starting at it holds a kind
other than a word (`ShapeField::template_spans`). A run counts toward a table when its tokens
hold a kind other than a word, and, where its period is one or two tokens, when its lines hold
about as many tokens as each other: the standard deviation of the tokens a line holds is at
most half their mean, a line ending at `\n` or `\r` (`ShapeField::table_spans`). A period that
short holds no columns, only one kind over and over or a value and a separator in turn, so the
rows have to be lines: a column of numbers one to a line is a table, and a paragraph of
clauses and commas, whose lines are paragraphs of any length, is not. A span more than half of
whose bytes those runs cover becomes `RegionKind::Table(period)`, with the period of the run
covering most of it.

Third, the rest keep their spectral texture. A five-row file of `alpha 10` lines is a table
though the reader finds its period only at the last row; a paragraph of prose holding a short
repeating phrase is prose, in Japanese or Korean as in English.

```rust
pub enum RegionKind { Table(u16), Blob, Prose, Numeric, Code, Mixed }
```

`trex spectral --classify` prints the regions, `-Classify` and `classify=True` add them to the
spectral report, and the `--texture` filter on a file walk keeps the files whose dominant region
is of a kind. The table reads as one region, its period the row's:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex spectral --classify scores.csv
trex spectral (region classification: shape x spectral): 64 bytes, 1 regions
    [       0..64      ] table(p5)  name,score,rank ann,12,3 bo,
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::shape::{RegionKind, classified_regions};

let csv = b"name,score,rank\nann,12,3\nbo,1200,1\ncyrus,7,22\ndee,450,4\neve,3,9\n";
assert_eq!(classified_regions(csv), [(0, 64, RegionKind::Table(5))]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(g.kind, g.offset, g.length) for g in trex.axes.spectral(path="scores.csv", classify=True).regions]
[('table', 0, 64)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexSpectral -Path ./scores.csv -Classify).Regions | Format-Table Offset, Length, Kind

Offset Length  Kind
------ ------  ----
     0     64 Table
```
{{< /tab >}}
{{< /tabs >}}

## Data model

`ShapeClass` is a `u32` packing a token's structural identity: the `TokenKind` code in the high
bits, for a word the [`tokutil::shape`](https://github.com/Variably-Constant/trex/blob/main/src/tokutil.rs)
class (Pascal 1, snake 2, camel 3, SCREAM 4, short 5, word 0), and for punctuation the glyph,
the low sixteen bits of its first character's code point. Two tokens with the same class have
the same shape. The word class reads case and length from characters, so `Επειδή` is Pascal as
`Whereas` is, a word of a script with no case is short or word, and short is four characters or
fewer whatever their bytes; `、` and `。` are two glyphs as `,` and `.` are.

`ShapeFrame`, one token's reading:

| Field | Type | Meaning |
|---|---|---|
| `class` | `u32` | the token's `ShapeClass` |
| `period` | `u16` | the dominant shape period of the window behind the token, `0` for none |
| `period_strength` | `f32` | the share of the window's silhouettes equal to the one a period before, `[0,1]` |
| `novelty` | `f32` | `1 / (1 + n)` for the run of three silhouettes ending at the token, `1` for a first sighting |

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
| `in_template(byte)` | whether `byte` is inside a template region |
| `shape_regions()` | the template regions as byte spans with their period |
| `template_spans()` | the template regions reached back over the periods before them that already repeat |
| `table_spans(input)` | the template spans a table is made of |

`analyze(tokens, bytes)` and `analyze_bytes(bytes)` read with the default `ShapeConfig`, and
`analyze_with(tokens, bytes, &cfg)` under one; `analyze_over`, `analyze_over_with` and
`analyze_bytes_over` read over an orbit group.

| `ShapeConfig` field | Default | Meaning |
|---|---|---|
| `max_lag` | 64 | the longest lag the period search tries |
| `period_window` | 256 | the tokens behind a token the period is read over |
| `period_hop` | 8 | the period is read again every this many tokens and held between |
| `novelty_k` | 3 | the silhouettes in the run novelty counts |
| `novelty_window` | 4096 | the runs the novelty counts span |
| `template_strength` | 0.6 | the period strength a template region needs |
| `cp_min_gap` | 4 | the tokens between two change points at least |

## Algorithms

One pass over the token stream (`analyze`):

- silhouette: `(TokenKind, word shape or punct glyph)` folded into a `u32`, O(1) a token;
- shape periodicity: every `period_hop` tokens, over the `period_window` silhouettes behind the
  token, the share of positions whose silhouette equals the one a lag back, for each lag up to
  `max_lag` and half the window; the period is the lag with the largest share where the share
  reaches 0.5, and every token until the next reading takes it;
- shape novelty: a count of each run of `novelty_k` silhouettes over the last `novelty_window`
  runs, `novelty = 1 / (1 + count before this one)`, `1` for the first `novelty_k - 1` tokens;
- shape change point: a token whose novelty is above 0.5, a run never seen in the window, at
  least `cp_min_gap` tokens after the last change point and never at the first token.

## Cost

| Kernel | Per-token cost | Bounded by |
|---|---|---|
| silhouette | O(1) | fixed |
| novelty | O(1) amortized | `novelty_window` |
| periodicity | O(1) amortized | `period_window` and `max_lag`, read every `period_hop` tokens |
| change-point | O(1) | fixed |
