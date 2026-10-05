---
title: The gravity axis
linkTitle: Gravity
weight: 97
---

The gravity axis reads the pair field: for every type of unit and every other, how much more or
less often the second follows the first at each gap than chance alone would. A pair the input
repeats pulls and a pair it avoids pushes. Three readings are taken from the field at every unit:
its strain, how hard the units before it push it away; the bound of the cut before it, how
strongly the input holds together across that cut; and its type's class, the types the field
treats alike.

The field is learned from the input it reads, at one of three grains: the bytes, the significant
tokens, or the [supertokens](../../../explanation/architecture/#supertokens). Nothing is trained
ahead of time, so a high strain means unusual for
this input, whether it is a log, a program or a table.

Source: [`src/gravity.rs`](https://github.com/Variably-Constant/trex/blob/main/src/gravity.rs).

## What it reads that nothing else does

```console
$ cat app.log
12:00:01 INFO start job=17
12:00:02 INFO read rows=1200
12:00:03 INFO read rows=1180
12:00:04 WARN slow disk=sda
12:00:05 INFO read rows=1210
12:00:06 INFO write rows=3590
12:00:07 ERROR lost disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
```

Two lines of this log break its pattern, the warning and the error, and the pair field finds both
without being told what a log looks like. At the token grain the field reads each token's kind,
and the log puts a number after every `=` but two: `disk=sda` and `disk=sdb` put a word there.
Those two words are the only tokens whose past pushes them away, so they are the units under the
most strain, and the cuts before each `disk` and each disk name are the ones the log holds
together least. Echo's `@novel` marks the first occurrence of each word, number and timestamp, 29
of the log's 54 tokens with the two disk names among them; strain marks the two alone, because it
reads how a token fits what comes before it rather than whether its text is new.

Every example on this page reads that file, except the gravity classes and the kin examples,
which need a longer input: Moby-Dick, which the TREX repository holds as
`tests/documented/moby.txt`.

## The readings

| Reading | Definition | Reads |
|---|---|---|
| strain | the mean of `-log2 g` between the unit and each unit before it within the reach: 64 bytes, 32 tokens or 16 supertokens | how hard the unit's past pushes it away, in bits |
| bound | the mean of `log2 g` over the pairs of units that straddle the cut before the unit, each within eight units of it | how strongly the input holds together across the cut, in bits per pair |
| percentile | the share of the input's own readings below the unit's, in percent | how a reading ranks in its input |
| class | the type's gravity class, from 0 to 15, or none where no significant pair places the type | which types the field treats alike |

`g` for a type `a` and a type `b` at gap `d` is the count of `a` followed `d` units later by `b`
against the count chance would give, each with one pair added, so a pair never seen reads as a
push in proportion to how often chance would show it. `-log2 g`, the pair's potential, is
negative where the pair pulls and positive where it pushes. The first unit has nothing before it
and no cut before it, so it reads neither strain nor bound, and no anchor holds there.

## Reading the axis

The summary counts the units and their types, and names the unit under the most strain and the
cut held together least:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex gravity app.log
trex gravity (token grain): 256 bytes, 54 tokens, 4 types, 0 gravity class(es)
  most strain: 0.68 bits at 'sda' (byte 109, percentile 98)
  weakest cut: 0.53 bits per pair before 'disk' (byte 104, percentile 0)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"12:00:01 INFO start job=17
12:00:02 INFO read rows=1200
12:00:03 INFO read rows=1180
12:00:04 WARN slow disk=sda
12:00:05 INFO read rows=1210
12:00:06 INFO write rows=3590
12:00:07 ERROR lost disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
";
let r = trex::gravity::Readings::read(trex::ast::Grain::Token, log, &trex::lexer::lex(log));
assert_eq!((r.len(), r.types(), r.classes().len()), (54, 4, 0));
let (s, e) = r.span_of(r.most_strained(1)[0]);
assert_eq!(&log[s..e], b"sda");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> g = trex.axes.gravity(path="app.log")
>>> g.grain, g.units, g.types, len(g.classes)
('token', 54, 4, 0)
>>> g.strained[0].text, round(g.strained[0].strain, 2)
('sda', 0.68)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexGravity -Path ./app.log | Select-Object Grain, Units, Types | Format-List

Grain : Token
Units : 54
Types : 4
```
{{< /tab >}}
{{< /tabs >}}

The log has no gravity class: the field places types in classes only when at least 16 of them
have a significant pair, and the log has four types. [Gravity classes](#gravity-classes) reads
them on a longer input.

### Every unit's reading

`--field` lists every unit with its strain and that strain's percentile, the bound of the cut
before it and the bound's percentile, and its type's class, a dash where there is none. In
PowerShell `-Detail` and in Python `detail=True` add the same frames to the report:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex gravity --field --limit 6 app.log
trex gravity (token grain): 256 bytes, 54 tokens, 4 types, 0 gravity class(es)
  most strain: 0.68 bits at 'sda' (byte 109, percentile 98)
  weakest cut: 0.53 bits per pair before 'disk' (byte 104, percentile 0)
  per-token (offset, token, strain in bits and its percentile, bound in bits per pair and its percentile, class):
    @     0  12:00:01             -     -        -     -   -
    @     9  INFO             -0.76    45     1.19    96   -
    @    14  start            -0.47    58     0.77    42   -
    @    20  job              -0.24    94     0.71    38   -
    @    23  =                -1.04    38     0.71    40   -
    @    24  17               -1.17    19     0.87    79   -
    ... (+48 more tokens; raise --limit)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"12:00:01 INFO start job=17
12:00:02 INFO read rows=1200
12:00:03 INFO read rows=1180
12:00:04 WARN slow disk=sda
12:00:05 INFO read rows=1210
12:00:06 INFO write rows=3590
12:00:07 ERROR lost disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
";
let r = trex::gravity::Readings::read(trex::ast::Grain::Token, log, &trex::lexer::lex(log));
let first = r.reading_of(0);
assert_eq!((first.strain, first.bound), (None, None));
let round = |v: Option<f32>| (v.expect("a reading") * 100.0).round() / 100.0;
let info = r.reading_of(1);
assert_eq!((round(info.strain), round(info.bound)), (-0.76, 1.19));
assert_eq!(r.class_of(r.type_of(1)), None);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> d = trex.axes.gravity(path="app.log", detail=True)
>>> d.frames[0].text, d.frames[0].strain, d.frames[0].bound
('12:00:01', None, None)
>>> [(f.text, round(f.strain, 2), round(f.bound, 2)) for f in d.frames[1:4]]
[('INFO', -0.76, 1.19), ('start', -0.47, 0.77), ('job', -0.24, 0.71)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $frames = (Measure-TrexGravity -Path ./app.log -Detail).Frames
PS> $null -eq $frames[0].Strain
True
PS> $frames | Select-Object -Skip 1 -First 3 | Format-Table Offset, Text, Strain, StrainPercentile, Bound, BoundPercentile

Offset Text  Strain StrainPercentile Bound BoundPercentile
------ ----  ------ ---------------- ----- ---------------
     9 INFO   -0.76            45.28  1.19           96.23
    14 start  -0.47            58.49  0.77           41.51
    20 job    -0.24            94.34  0.71           37.74
```
{{< /tab >}}
{{< /tabs >}}

A percentile places a reading among the input's own. `job` at byte 20 reads a strain of -0.24
bits, pulled rather than pushed, yet ranks above 94 percent of the log's strains, because nearly
every token in the log is pulled harder.

### The most strained units

`--strain` lists the units under the most strain, most first, `--top K` of them, eight by
default. The report's `Strained` in PowerShell and `strained` in Python hold the same, `-Top` and
`top=` of them:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex gravity --strain --top 3 app.log
trex gravity (token grain): 256 bytes, 54 tokens, 4 types, 0 gravity class(es)
  most strain: 0.68 bits at 'sda' (byte 109, percentile 98)
  weakest cut: 0.53 bits per pair before 'disk' (byte 104, percentile 0)
  most strained (3 of 53 tokens with a reading):
    @   109  sda               0.68 bits  percentile  98
    @   197  sdb               0.62 bits  percentile  96
    @    20  job              -0.24 bits  percentile  94
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"12:00:01 INFO start job=17
12:00:02 INFO read rows=1200
12:00:03 INFO read rows=1180
12:00:04 WARN slow disk=sda
12:00:05 INFO read rows=1210
12:00:06 INFO write rows=3590
12:00:07 ERROR lost disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
";
let r = trex::gravity::Readings::read(trex::ast::Grain::Token, log, &trex::lexer::lex(log));
let named: Vec<&[u8]> = r
    .most_strained(3)
    .into_iter()
    .map(|u| {
        let (s, e) = r.span_of(u);
        &log[s..e]
    })
    .collect();
assert_eq!(named, [&b"sda"[..], &b"sdb"[..], &b"job"[..]]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, f.offset, round(f.strain, 2)) for f in trex.axes.gravity(path="app.log", top=3).strained]
[('sda', 109, 0.68), ('sdb', 197, 0.62), ('job', 20, -0.24)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexGravity -Path ./app.log -Top 3).Strained | Format-Table Offset, Text, Strain, StrainPercentile

Offset Text Strain StrainPercentile
------ ---- ------ ----------------
   109 sda    0.68            98.11
   197 sdb    0.62            96.23
    20 job   -0.24            94.34
```
{{< /tab >}}
{{< /tabs >}}

A tie goes to the earlier unit, and the first unit, which reads no strain, is never among them.

### The weakest cuts

`--bound` lists the cuts held together least, least first, each named by the unit after it.
`Weakest` and `weakest` hold the same:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex gravity --bound --top 4 app.log
trex gravity (token grain): 256 bytes, 54 tokens, 4 types, 0 gravity class(es)
  most strain: 0.68 bits at 'sda' (byte 109, percentile 98)
  weakest cut: 0.53 bits per pair before 'disk' (byte 104, percentile 0)
  weakest cuts (4 of 53), each before the token named:
    @   104  disk              0.53 bits per pair  percentile   0
    @   192  disk              0.53 bits per pair  percentile   0
    @   109  sda               0.53 bits per pair  percentile   4
    @   197  sdb               0.53 bits per pair  percentile   4
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"12:00:01 INFO start job=17
12:00:02 INFO read rows=1200
12:00:03 INFO read rows=1180
12:00:04 WARN slow disk=sda
12:00:05 INFO read rows=1210
12:00:06 INFO write rows=3590
12:00:07 ERROR lost disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
";
let r = trex::gravity::Readings::read(trex::ast::Grain::Token, log, &trex::lexer::lex(log));
let starts: Vec<usize> = r.weakest_cuts(4).into_iter().map(|u| r.start_of(u)).collect();
assert_eq!(starts, [104, 192, 109, 197]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, f.offset, round(f.bound, 2)) for f in trex.axes.gravity(path="app.log", top=4).weakest]
[('disk', 104, 0.53), ('disk', 192, 0.53), ('sda', 109, 0.53), ('sdb', 197, 0.53)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexGravity -Path ./app.log -Top 4).Weakest | Format-Table Offset, Text, Bound, BoundPercentile

Offset Text Bound BoundPercentile
------ ---- ----- ---------------
   104 disk  0.53            0.00
   192 disk  0.53            0.00
   109 sda   0.53            3.77
   197 sdb   0.53            3.77
```
{{< /tab >}}
{{< /tabs >}}

The four cuts are before `disk` and before the disk's name, in the warning and in the error.

### Gravity classes

A pair is significant where chance would give it at least five times and its count is more than
four standard deviations from that. The types such pairs join are placed, and when at least
16 are placed the field groups them into 16 classes of types it treats alike. `--classes` lists
each class with how many units are of it and its types. Over Moby-Dick at the byte grain the
vowels `a`, `e` and `o` share a class with `s`, the comma shares one with the semicolon, and the
exclamation and question marks share one with the last bytes of the em dash and of the curly
apostrophe, which the text writes in UTF-8:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex gravity --classes --grain byte moby.txt
trex gravity (byte grain): 1234609 bytes, 108 types, 16 gravity class(es)
  most strain: 0.58 bits at ' ' (byte 9, percentile 100)
  weakest cut: -0.95 bits per pair before '\xe2' (byte 1108995, percentile 0)
  gravity classes (16):
    class  0: 194869 bytes of ' '
    class  1: 21936 bytes of '\n'
    class  2: 15518 bytes of '\x80', '\xe2'
    class  3: 7973 bytes of '.'
    class  4: 122148 bytes of 'c', 'f', 'g', 'm', 'p', 'w'
    class  5: 86112 bytes of 't'
    class  6: 324114 bytes of 'a', 'e', 'o', 's'
    class  7: 23415 bytes of ',', ';'
    class  8: 3093 bytes of '\x9c', '\x9d'
    class  9: 37717 bytes of 'd'
    class 10: 158995 bytes of 'l', 'n', 'r'
    class 11: 61987 bytes of 'h'
    class 12: 8898 bytes of 'A', 'C', 'H', 'P', 'T', '_'
    class 13: 97957 bytes of '(', ')', '*', '-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ':', 'B', 'D', 'E', 'F', 'G', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'Q', 'R', 'S', 'U', 'V', 'W', 'Y', 'b', 'j', 'k', 'q', 'u', 'v', 'x', 'y', 'z', '\x98'
    class 14: 7292 bytes of '!', '?', '\x94', '\x99'
    class 15: 62446 bytes of 'i'
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = std::fs::read("tests/documented/moby.txt").expect("Moby-Dick, which tests/documented holds");
let r = trex::gravity::Readings::read(trex::ast::Grain::Byte, &text, &[]);
let classes = r.classes();
assert_eq!(classes.len(), 16);
let labels = |b: u8| -> Vec<String> {
    let class = classes.iter().find(|g| g.types.contains(&u32::from(b))).expect("a placed byte");
    class.types.iter().map(|&t| r.type_label(t)).collect()
};
assert_eq!(labels(b'a'), ["a", "e", "o", "s"]);
assert_eq!(labels(b','), [",", ";"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> b = trex.axes.gravity(path="moby.txt", grain="byte")
>>> b.units, b.types, len(b.classes)
(1234609, 108, 16)
>>> [(c.class_, c.types) for c in b.classes if "a" in c.types or "," in c.types or "!" in c.types]
[(6, ['a', 'e', 'o', 's']), (7, [',', ';']), (14, ['!', '?', '\\x94', '\\x99'])]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexGravity -Path ./moby.txt -Grain Byte).Classes | Where-Object { $_.Types -ccontains 'a' -or $_.Types -ccontains ',' -or $_.Types -ccontains '!' } | Format-Table Class, Units, @{ Name = 'Types'; Expression = { $_.Types -join ' ' } }

Class  Units Types
-----  ----- -----
    6 324114 a e o s
    7  23415 , ;
   14   7292 ! ? \x94 \x99
```
{{< /tab >}}
{{< /tabs >}}

The command quotes each type it names, so a comma reads `','` and a space `' '`. Python and
PowerShell hold each type's label as a string: a byte as itself where it is printable ASCII,
`' '` for a space and an escape such as `\n` or `\x94` otherwise; a token type as its kind with a
punctuation mark's text, `Punct ,`; and a supertoken type as its role and the kinds of its first
six tokens. `-ccontains` compares case-sensitively, so `'a'` does not also find the class of `A`.

### Grains

The field reads the tokens by default; `--grain byte` reads the bytes and `--grain super` the
supertokens, `-Grain` and `grain=` in PowerShell and Python. Each grain learns its own field, so the same log reads differently at each.
At the byte grain its 256 bytes are 36 types, the `1` that ends the first timestamp has the most
strain, and the weakest cut is before the log's one `w`. At the supertoken grain each line
is one unit, and the field reads two types: the seven lines ending in a number and the two ending
in a word.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex gravity --grain byte app.log
trex gravity (byte grain): 256 bytes, 36 types, 0 gravity class(es)
  most strain: 0.30 bits at '1' (byte 7, percentile 100)
  weakest cut: 0.66 bits per pair before 'w' (byte 156, percentile 0)

$ trex gravity --grain super app.log
trex gravity (super grain): 256 bytes, 9 supertokens, 2 types, 0 gravity class(es)
  most strain: 0.17 bits at '12:00:03 INFO read rows=1180' (byte 56, percentile 88)
  weakest cut: -0.12 bits per pair before '12:00:02 INFO read rows=1200' (byte 27, percentile 0)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::ast::Grain;
use trex::gravity::Readings;

let log = b"12:00:01 INFO start job=17
12:00:02 INFO read rows=1200
12:00:03 INFO read rows=1180
12:00:04 WARN slow disk=sda
12:00:05 INFO read rows=1210
12:00:06 INFO write rows=3590
12:00:07 ERROR lost disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
";
let bytes = Readings::read(Grain::Byte, log, &[]);
assert_eq!((bytes.len(), bytes.types()), (256, 36));
let lines = Readings::read(Grain::Super, log, &trex::lexer::lex(log));
assert_eq!((lines.len(), lines.types()), (9, 2));
let ending_in_a_word: Vec<usize> = (0..9).filter(|&u| lines.type_of(u) != lines.type_of(0)).collect();
assert_eq!(ending_in_a_word, [3, 6]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(x.grain, x.units, x.types) for x in (trex.axes.gravity(path="app.log", grain=n) for n in ("byte", "super"))]
[('byte', 256, 36), ('super', 9, 2)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'Byte', 'Super' | ForEach-Object { Measure-TrexGravity -Path ./app.log -Grain $_ } | Format-Table Grain, Units, Types

Grain Units Types
----- ----- -----
 Byte   256    36
Super     9     2
```
{{< /tab >}}
{{< /tabs >}}

A byte's percentile of 100 is a rounding of the share below it: 254 of the 255 bytes with a
reading are below the `1`. The supertoken percentile of 88 is the most the nine lines allow,
seven of the eight readings below the highest.

## Choosing a threshold

Every gravity anchor compares a reading with a threshold written one of two ways. A percentile,
the default, names a share of the input's own readings: `@strain>90` holds where a unit's strain
is above the reading at the 90th percentile of the input's strains, taken by the nearest rank over
the units that have one. A value in bits, written with `b`, means the same in every input:
`@strain>0b` holds wherever a unit's strain is above zero bits, where its past pushes it away
rather than pulls it. A percentile adapts to each input and takes about the same share of it; a
value in bits holds a threshold steady while inputs change.

Read the readings before choosing. `--strain --top 53` lists all of the log's strains: they run
from 0.68 bits down to -1.35, and only the two disk names are above zero. The bounds are closer
together: the four weakest cuts read 0.53 bits per pair and the next 0.54.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@strain>90 .' app.log
[20..23] "job"
[109..112] "sda"
[197..200] "sdb"
[220..224] "rows"
[249..252] "job"

$ trex scan '@strain>0b .' app.log
[109..112] "sda"
[197..200] "sdb"

$ trex scan '@bound<10 .' app.log
[104..108] "disk"
[109..112] "sda"
[192..196] "disk"
[197..200] "sdb"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = "12:00:01 INFO start job=17
12:00:02 INFO read rows=1200
12:00:03 INFO read rows=1180
12:00:04 WARN slow disk=sda
12:00:05 INFO read rows=1210
12:00:06 INFO write rows=3590
12:00:07 ERROR lost disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
";
let found = |pat: &str| -> Vec<&str> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, log.as_bytes()).iter().map(|s| &log[s.range()]).collect()
};
assert_eq!(found("@strain>90 ."), ["job", "sda", "sdb", "rows", "job"]);
assert_eq!(found("@strain>0b ."), ["sda", "sdb"]);
assert_eq!(found("@bound<10 ."), ["disk", "sda", "disk", "sdb"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> log = open("app.log").read()
>>> [m.text for m in trex.Pattern(r"@strain>90 .").scan(log)]
['job', 'sda', 'sdb', 'rows', 'job']
>>> [m.text for m in trex.Pattern(r"@strain>0b .").scan(log)]
['sda', 'sdb']
>>> [m.text for m in trex.Pattern(r"@bound<10 .").scan(log)]
['disk', 'sda', 'disk', 'sdb']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@strain>90 .' -Path ./app.log -Raw
job
sda
sdb
rows
job

PS> Select-TrexMatch '@strain>0b .' -Path ./app.log -Raw
sda
sdb

PS> Select-TrexMatch '@bound<10 .' -Path ./app.log -Raw
disk
sda
disk
sdb
```
{{< /tab >}}
{{< /tabs >}}

**Strain.** The percentile takes its share whether or not that share is unusual: `@strain>90`
takes five of the log's 53 strains, the two disk names and three tokens the log pulls, `job` at
either end and the last `rows`. Zero bits is the cut these readings ask for, since every unit but
the two disk names is pulled and the readings are far from zero on both sides, 0.62 bits above
it and -0.24 below.

**Bound.** No value in bits separates the four weakest cuts from the next with any room, so a
percentile is the steadier cut here: `@bound<10` takes the cuts below the reading at the 10th
percentile, the four around the disk names.

The comparisons are strict as written: `>` and `<` leave out a reading equal to the threshold,
and `>=` and `<=` take it.

## Declarations

A declared shape or kind changes what a token is, and so changes the types the field counts. The
three levels declared as one shape become a type of their own, five types where there were four. The
disk names stay under the most strain, and the weakest cut moves to the one before `sda`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex gravity --shape 'level = `INFO|WARN|ERROR`' app.log
trex gravity (token grain): 256 bytes, 54 tokens, 5 types, 0 gravity class(es)
  most strain: 0.54 bits at 'sda' (byte 109, percentile 98)
  weakest cut: 0.94 bits per pair before 'sda' (byte 109, percentile 0)

$ trex gravity --grain byte --shape 'level = `INFO|WARN|ERROR`' app.log
trex gravity: declarations decide how tokens are read, and the byte grain reads bytes; --grain token or super reads them
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"12:00:01 INFO start job=17
12:00:02 INFO read rows=1200
12:00:03 INFO read rows=1180
12:00:04 WARN slow disk=sda
12:00:05 INFO read rows=1210
12:00:06 INFO write rows=3590
12:00:07 ERROR lost disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
";
let mut shapes = trex::ShapeSet::new();
shapes.declare("level = `INFO|WARN|ERROR`", trex::Precedence::Before).expect("a bounded shape");
let toks = trex::lexer::lex_with_shapes(log, &trex::lexer::blob_runs(log), &shapes, 0);
let r = trex::gravity::Readings::read(trex::ast::Grain::Token, log, &toks);
assert_eq!(r.types(), 5);
let (s, e) = r.span_of(r.weakest_cuts(1)[0]);
assert_eq!(&log[s..e], b"sda");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> lib = trex.Library()
>>> lib.shape("level", r"INFO|WARN|ERROR")
>>> s = trex.axes.gravity(path="app.log", lib=lib)
>>> s.types, s.strained[0].text, s.weakest[0].text
(5, 'sda', 'sda')
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $lib = New-TrexLibrary
PS> Register-TrexAtom level -Shape 'INFO|WARN|ERROR' -Library $lib
PS> $g = Measure-TrexGravity -Path ./app.log -Library $lib
PS> $g.Types, $g.Strained[0].Text, $g.Weakest[0].Text
5
sda
sda
```
{{< /tab >}}
{{< /tabs >}}

The token and supertoken grains take the declarations a scan takes: `--lib`, `--shape`,
`--shape-after`, `--kind`, `--let` and `--declare` on the command line, `-Library` in PowerShell
and `lib=` in Python ([pattern files](../../pattern-files/)). The byte grain reads bytes, which no
declaration changes, and refuses them.

## In a pattern

Three anchors read the field at the token the pattern is matching. Each reads the token grain unless
a grain follows its name: `:byte` reads the token's first byte and `:super` the supertoken holding
the token, each over the field of its own grain.

| Anchor | Holds where |
|---|---|
| `@strain>k`, `@strain:byte>k`, `@strain:super>=k` | the unit's strain is above `k`, a percentile or a value in bits |
| `@bound<k`, `@bound:byte<k`, `@bound:super<=k` | the bound of the cut before the unit is below `k`; at `:super` only at a token that opens its supertoken |
| `@kin("x")`, `@kin:byte("e")`, `@kin:super("f(x)")` | the unit is of `x`'s type or of a type the field places in one class with it; `x` is one byte at `:byte`, and otherwise the first token or supertoken it forms |
| `=kin name`, `=kin:byte name`, `=kin:super name` | a later token whose unit is kin, in the same sense, to the unit the value bound to `name` starts in |

An example the input never holds names no type and matches nothing, and where the field places no
class, kin is the type alone. Over Moby-Dick, `@kin:byte("e")` takes the words whose first byte
the field places with `e`, one of `a`, `e`, `o` or `s`, and `\W:w =kin:byte w` takes a word with
the token after it when the field places their first bytes together: `S` and `O`, `T` and `P`,
`G` and `E`.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@kin:byte("e") \W' moby.txt | head -3
[66..68] "or"
[148..149] "a"
[829..832] "and"

$ trex scan '\W:w =kin:byte w' moby.txt | head -3
[4..12] "START OF"  captures: w="START"
[13..24] "THE PROJECT"  captures: w="THE"
[25..40] "GUTENBERG EBOOK"  captures: w="GUTENBERG"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = std::fs::read("tests/documented/moby.txt").expect("Moby-Dick, which tests/documented holds");
let first_three = |pat: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, &text).iter().take(3).map(|s| String::from_utf8_lossy(&text[s.range()]).into_owned()).collect()
};
assert_eq!(first_three(r#"@kin:byte("e") \W"#), ["or", "a", "and"]);
assert_eq!(first_three(r"\W:w =kin:byte w"), ["START OF", "THE PROJECT", "GUTENBERG EBOOK"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> moby = open("moby.txt", encoding="utf-8").read()
>>> [m.text for m in trex.Pattern(r'@kin:byte("e") \W').scan(moby)][:3]
['or', 'a', 'and']
>>> [m.text for m in trex.Pattern(r"\W:w =kin:byte w").scan(moby)][:3]
['START OF', 'THE PROJECT', 'GUTENBERG EBOOK']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@kin:byte("e") \W' -Path ./moby.txt -Raw | Select-Object -First 3
or
a
and

PS> Select-TrexMatch '\W:w =kin:byte w' -Path ./moby.txt -Raw | Select-Object -First 3
START OF
THE PROJECT
GUTENBERG EBOOK
```
{{< /tab >}}
{{< /tabs >}}

### The readings at a match

A scan's `--format`, PowerShell's `-Format` and Python's `Match.format` render the field's readings
at a match: `${@gravity}` as the sentence `--explain` prints, or one piece of it as
`${@gravity.strain}`, `${@gravity.bound}`, `${@gravity.class}` or `${@gravity.grain}`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@strain>0b .' --format '${0}: ${@gravity}' app.log
sda: strain 0.68 bits (percentile 98), bound 0.53 bits per pair (percentile 4), class unplaced at the token grain
sdb: strain 0.62 bits (percentile 96), bound 0.53 bits per pair (percentile 4), class unplaced at the token grain

$ trex scan '@strain>0b .' --format '${0} strain=${@gravity.strain} bound=${@gravity.bound} class=${@gravity.class} grain=${@gravity.grain}' app.log
sda strain=0.68 bound=0.53 class=unplaced grain=token
sdb strain=0.62 bound=0.53 class=unplaced grain=token
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"12:00:01 INFO start job=17
12:00:02 INFO read rows=1200
12:00:03 INFO read rows=1180
12:00:04 WARN slow disk=sda
12:00:05 INFO read rows=1210
12:00:06 INFO write rows=3590
12:00:07 ERROR lost disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
";
let pat = trex::parse("@strain>0b .").expect("valid pattern");
let tmpl = trex::Template::parse_report("${0}: ${@gravity}", &pat.capture_names()).expect("valid template");
let explainer = trex::explain::Explainer::new(&pat, log, &trex::ShapeSet::new());
let lines: Vec<String> = trex::captures(&pat, log, &trex::scan(&pat, log))
    .iter()
    .map(|m| tmpl.render_explained(m, log, None, &explainer.explain(m, &String::new())))
    .collect();
assert_eq!(
    lines[0],
    "sda: strain 0.68 bits (percentile 98), bound 0.53 bits per pair (percentile 4), class unplaced at the token grain"
);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.format("${0}: ${@gravity}", log) for m in trex.Pattern(r"@strain>0b .").scan(log)][0]
'sda: strain 0.68 bits (percentile 98), bound 0.53 bits per pair (percentile 4), class unplaced at the token grain'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@strain>0b .' -Path ./app.log -Format '${0}: ${@gravity}'
sda: strain 0.68 bits (percentile 98), bound 0.53 bits per pair (percentile 4), class unplaced at the token grain
sdb: strain 0.62 bits (percentile 96), bound 0.53 bits per pair (percentile 4), class unplaced at the token grain
```
{{< /tab >}}
{{< /tabs >}}

A pattern with no gravity anchor reads no field, and renders `${@gravity}` empty, as it does any
axis it never read.

## Records

`--record bind` cuts an input into records where its bytes hold together least: as many cuts as
the [seam](../seam/) makes, at the weakest cuts of the byte field. `bind:Q` cuts at every byte
whose cut is in the weakest `Q` percent, and `auto` takes whichever of `seam` and `bind` puts more
of its cuts at the starts of lines ([record units](../../records/#record-units)). On the log,
`bind:1` makes five records, cut before each ` disk=` and on either side of the log's one `w`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex lines 5..5 --record bind:1 app.log
 disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;

let log = b"12:00:01 INFO start job=17
12:00:02 INFO read rows=1200
12:00:03 INFO read rows=1180
12:00:04 WARN slow disk=sda
12:00:05 INFO read rows=1210
12:00:06 INFO write rows=3590
12:00:07 ERROR lost disk=sdb
12:00:08 INFO read rows=1190
12:00:09 INFO done job=17
";
let bind = RecordUnit::parse("bind:1").expect("a unit");
assert_eq!(bind.records(log), [(0, 103), (103, 156), (156, 157), (157, 191), (191, 256)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.records(log, "bind:1")
[(0, 103), (103, 156), (156, 157), (157, 191), (191, 256)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexRecord (Get-Content ./app.log -Raw) -Unit bind:1 | Select-Object Start, Length

Start Length
----- ------
    0    103
  103     53
  156      1
  157     34
  191     65
```
{{< /tab >}}
{{< /tabs >}}

On a log this short the default `bind` cuts inside words: it makes as many cuts as the seam, and
the seam cuts the log at every token, 90 records. `auto` takes the seam's cuts here, nine of whose
90 records start a line, against four of `bind`'s.

## Data model

`Readings`, one grain of one input:

| Field | Type | Meaning |
|---|---|---|
| `strain` | `Vec<Option<f32>>` | each unit's strain in bits; `None` at the first unit |
| `binding` | `Vec<Option<f32>>` | the bound of the cut before each unit in bits per pair; `None` at the first unit |
| `class` | `Vec<u16>` | each type's class; `UNPLACED` (16) where no significant pair places it |

| Method | Returns |
|---|---|
| `read(grain, bytes, toks)` | the readings of `bytes` at `grain`, over the lex `toks` |
| `grain()`, `len()`, `is_empty()`, `types()` | the grain, how many units it holds, and how many distinct types they are of |
| `span_of(unit)`, `start_of(unit)` | the bytes a unit spans, and the offset it starts at, which is the offset of the cut before it |
| `unit_at(byte)`, `unit_starting_at(byte)` | the unit holding a byte, and the unit starting at it |
| `type_of(unit)`, `type_named(key)`, `type_label(ty)` | a unit's type, the type a key names, and a type's label |
| `class_of(ty)`, `kin(a, b)` | a type's class, and whether two types are the same or share a class |
| `reading_of(unit)` | a `UnitReading`: `strain` and `bound`, each with its percentile |
| `most_strained(k)`, `weakest_cuts(k)` | the units under the most strain, and the units after the weakest cuts |
| `classes()` | each `GravityClass`: `class`, its `types` most frequent first, and the `units` of them |
| `strain_percentile(p)`, `binding_percentile(p)` | the reading at percentile `p`, by the nearest rank |
| `strain_rank(value)`, `binding_rank(value)` | the share of readings below `value`, in percent |

`Field::learn(seq, types, reach)` learns the field over any stream of type numbers, and
`strain`, `binding` and `geometry` read it, for a caller with a stream of its own.

| Constant | Value | Meaning |
|---|---|---|
| `BYTE_REACH`, `TOKEN_REACH`, `UNIT_REACH` | 64, 32, 16 | the gaps each grain's field reads to |
| `BIND_REACH` | 8 | how far either side of a cut its bound reaches |
| `TOP_TYPES` | 255 | the types a token or supertoken grain keeps by frequency; every other type is one more |
| `Z_CELL` | 4 | Poisson standard deviations past which a pair is significant |
| `MIN_EXPECTED` | 5 | the pairs chance must give before a pair's significance is read |
| `GEOMETRY_DIMS` | 8 | the dimensions the placed types are arranged in |
| `GEOMETRY_CLASSES` | 16 | the classes; fewer placed types than this places none |

## Algorithm

One pass learns the field and one reads it:

1. types: each unit's type is its byte at the byte grain; at the token grain the token's kind,
   a punctuation mark told apart by up to three characters of its text; at the supertoken grain
   the unit's role and the kinds of up to its first six tokens. The 255 most frequent types are
   kept and the rest are one type.
2. the field: for each gap `d` up to the reach, the count of each pair of types `d` apart, and the
   count chance gives it, how often the first type opens a pair at that gap times how often the
   second closes one, over the pairs the gap fits. `g = (count + 1) / (expected + 1)`, and
   `-log2 g` is the pair's potential.
3. strain: each unit's mean potential against the units before it within the reach.
4. bound: each cut's mean `log2 g` over the pairs straddling it within eight units. A cut at least
   eight units from both ends has 36 such pairs and one near an end fewer; the mean reads both on
   one scale.
5. geometry: the pairs whose count is more than four Poisson standard deviations from what
   chance gives, where chance gives at least five, place their two types. With at least 16 types
   placed, each pair of placed types takes the mean of `ln g` over every gap and both orders, a
   pair that is not significant counting zero; the eight leading eigenvectors of that matrix,
   each scaled by the root of its eigenvalue's size, place the types in eight dimensions, and
   k-means from a fixed start, each type weighted by its frequency, groups them into 16 classes.

## Cost

| Kernel | Per-unit cost | Bounded by |
|---|---|---|
| field | O(reach) | the reach: 64, 32 or 16 gaps |
| strain | O(reach) | the reach |
| bound | 36 pairs | `BIND_REACH` |
| geometry | once an input, over every pair of types at every gap, then the eigenvectors and k-means over the placed types | at most 256 types |

Measured against the parallel lex of the same input, on ten 16 MB inputs of different kinds, the
byte field costs 129 to 188 times the lex, the token field 16 to 62 times and the supertoken field
6 to 24 times. `--record bind` and `auto` read the byte field and the seam both, 244 to 417 times
the lex, five to seven seconds on 16 MB. A pattern learns the field once a scan, at each grain its
gravity anchors name.
