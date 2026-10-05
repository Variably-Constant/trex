---
title: The echo axis
linkTitle: Echo
weight: 90
---

The echo axis checks whether each token's content occurs elsewhere in the input: how many
times, how far back it last appeared, and whether its occurrences come at a regular spacing. A
token whose content appears once is novel; one whose content recurs is an echo. Punctuation and
brackets recur by grammar rather than by content, so the axis leaves them unkeyed.

Source: [`src/echo.rs`](https://github.com/Variably-Constant/trex/blob/main/src/echo.rs).

## What it reads that nothing else does

```text
ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net
```

Every word here is short and lowercase or uppercase, so magnitude and shape read them alike.
Echo reads their content: `disk` occurs three times at a near-regular spacing, `ERROR`, `full`,
`slow` and `net` twice, and `WARN`, `down` and the rest once. With case folded, `ERROR` and
`error` are one piece of content occurring three times. The same reading finds a repeated
message in a log, a name a file reuses, or a value that should have been unique.

Every example on this page reads that line.

## The readings

| Reading | Meaning |
|---|---|
| count | how many times the token's content occurs in the input |
| back-lag | the bytes back to its previous occurrence |
| novel | the token is the first occurrence of its content |
| period | the mean spacing of a content's occurrences, where the spacings vary by at most `max_period_cv` of their mean, 0.3 by default |
| novelty | the share of keyed tokens that are novel |
| echo rate | the share of keyed tokens whose content occurs more than once |

The key is the token's text, read exactly by default, which is the `identity` group. Another
group reads it up to a symmetry or a representation: `case` folds case, `shape` reads its pattern
of letters and digits, `e8` its profile of character kinds, `numeric` its value, and `ip`, `url`,
`time`, `path`, `fold` and `notation` fold the different ways of writing an address, a URL, a
time, a path, accented or wide text and a symbol. A typed relation keys a token by the part of it
the relation reads, so under `subnet/24` every address of a network is one key. The
[orbit](../orbit/#the-groups) page shows what each group folds.

## Reading the axis

The summary counts the tokens, the keyed ones and the distinct contents, gives the novelty and
the echo rate, and lists the strongest echoes:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex echo --text 'ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net'
trex echo: 79 bytes, 33 tokens (15 keyed, 9 distinct), novelty 60.0%, echo rate 73.3%
  strongest echoes (top 5 of 5; first occurrence, count, period):
    disk                 x3     period ~16 B
    ERROR                x2    
    full                 x2    
    slow                 x2    
    net                  x2    
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::echo::analyze_bytes(b"ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net");
assert_eq!((field.frames.len(), field.keyed, field.distinct), (33, 15, 9));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.echo("ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net")
>>> r.tokens, r.keyed, r.distinct, round(r.novelty, 3), round(r.echo_rate, 3)
(33, 15, 9, 0.6, 0.733)
>>> [(e.key, e.count) for e in r.echoes]
[('disk', 3), ('ERROR', 2), ('full', 2), ('slow', 2), ('net', 2)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $line = 'ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net'
PS> (Measure-TrexEcho $line).Echoes | Format-Table Key, Count, Period, Offset

Key   Count Period Offset
---   ----- ------ ------
disk      3  16.50      6
ERROR     2             0
full      2            11
slow      2            22
net       2            56
```
{{< /tab >}}
{{< /tabs >}}

`--top K` sizes the ranked lists, eight by default; the reports carry every echo.

### Every token's reading

`--field` lists each keyed token's count, its back-lag and whether it is novel, and `--limit N`
stops the list after `N` tokens and counts the rest. `-Detail` in PowerShell and `detail=True` in
Python add a frame at every token:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex echo --field --limit 4 --text 'ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net'
trex echo: 79 bytes, 33 tokens (15 keyed, 9 distinct), novelty 60.0%, echo rate 73.3%
  strongest echoes (top 5 of 5; first occurrence, count, period):
    disk                 x3     period ~16 B
    ERROR                x2    
    full                 x2    
    slow                 x2    
    net                  x2    
  per-token (token -> count | back-lag | novel):
    ERROR                x2    back=      - novel
    disk                 x3    back=      - novel
    full                 x2    back=      - novel
    WARN                 x1    back=      - novel
    ... (+11 more tokens; raise --limit)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net";
let field = trex::echo::analyze_bytes(text);
let second_disk = field.frames.iter().find(|f| f.start == 27).expect("the second disk");
assert_eq!((second_disk.count, second_disk.back_lag.map(|lag| lag.get())), (3, Some(21)));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> d = trex.axes.echo("ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net", detail=True)
>>> [(f.text, f.offset, f.count, f.nth, f.previous) for f in d.frames if f.text == "disk"]
[('disk', 6, 3, 1, None), ('disk', 27, 3, 2, 6), ('disk', 39, 3, 3, 27)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexEcho $line -Detail).Frames | Where-Object Text -eq 'disk' | Format-Table Offset, Count, Nth, Previous

Offset Count Nth Previous
------ ----- --- --------
     6     3   1
    27     3   2 6
    39     3   3 27
```
{{< /tab >}}
{{< /tabs >}}

### Reading up to a group

`--group case` keys each token with its case folded, so `ERROR` and `error`, and `WARN` and
`warn`, are each one content. The distinct count falls and the echo rate rises:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex echo --group case --top 3 --text 'ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net'
trex echo: 79 bytes, 33 tokens (15 keyed, 7 distinct), novelty 46.7%, echo rate 93.3%
  strongest echoes (top 3 of 6; first occurrence, count, period):
    ERROR                x3    
    disk                 x3     period ~16 B
    full                 x2    
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let cfg = trex::echo::EchoConfig { orbit: trex::OrbitGroup::Case, ..trex::echo::EchoConfig::default() };
let text = b"ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net";
let field = trex::echo::analyze_with(&trex::tokutil::lex_sig(text), text, &cfg);
assert_eq!(field.distinct, 7);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> c = trex.axes.echo("ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net", group="case")
>>> c.group, c.distinct, [(e.key, e.count) for e in c.echoes][:2]
('case', 7, [('ERROR', 3), ('disk', 3)])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexEcho $line -Group case | Select-Object Group, Distinct | Format-List

Group    : case
Distinct : 7
```
{{< /tab >}}
{{< /tabs >}}

### Structural rhyme

`--structure` keys each [supertoken](../../../explanation/architecture/#supertokens) by its role and its
token-kind silhouette, so two units that differ only in their names and values are one structure.
Six distinct words, every one novel, and one recurring structure:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex echo --structure --text 'alpha: one
bravo: two
delta: six'
trex echo: 32 bytes, 14 tokens (6 keyed, 6 distinct), novelty 100.0%, echo rate 0.0%
  structural rhyme (top 1 of 1 recurring supertoken structures):
    kv:W:W                       x3    period ~11 B
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"alpha: one\nbravo: two\ndelta: six";
let structures = trex::echo::analyze_super_with(text, &trex::echo::EchoConfig::default());
assert_eq!(structures.len(), 1);
assert_eq!(structures[0].count, 3);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> s = trex.axes.echo("alpha: one\nbravo: two\ndelta: six", structure=True)
>>> [(e.key, e.count) for e in s.structures]
[('kv:W:W', 3)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexEcho "alpha: one`nbravo: two`ndelta: six" -Structure).Structures | Format-Table Key, Count

Key    Count
---    -----
kv:W:W     3
```
{{< /tab >}}
{{< /tabs >}}

## Choosing a threshold

**The period.** A content is periodic when its spacings are steady enough: their standard
deviation at most `max_period_cv` of their mean. `disk` recurs after 21 bytes and then after 12,
a mean of 16.5 with a deviation of 4.5, which is 0.27 of the mean, inside the default 0.3. Lower
the cut to demand a steadier rhythm, raise it to admit a looser one; at 0 only exactly regular
spacings count, and `disk` loses its period:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex echo --max-period-cv 0 --top 1 --text 'ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net'
trex echo: 79 bytes, 33 tokens (15 keyed, 9 distinct), novelty 60.0%, echo rate 73.3%
  strongest echoes (top 1 of 5; first occurrence, count, period):
    disk                 x3    
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let cfg = trex::echo::EchoConfig { max_period_cv: 0.0, ..trex::echo::EchoConfig::default() };
let text = b"ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net";
let field = trex::echo::analyze_with(&trex::tokutil::lex_sig(text), text, &cfg);
assert!(field.frames.iter().all(|f| f.period == 0.0), "no content keeps a period");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(e.key, e.period) for e in trex.axes.echo("ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net", max_period_cv=0).echoes][:1]
[('disk', None)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> ((Measure-TrexEcho $line -MaxPeriodCv 0).Echoes | Select-Object -First 1).Period -eq $null
True
```
{{< /tab >}}
{{< /tabs >}}

The comparison is inclusive: spacings that vary by exactly the cut are periodic.

**A count.** `@echo>k` takes the contents that occur more than `k` times, so read the counts the
summary lists and put `k` below the count of the repetition you are after: here `@echo>2` keeps
`disk` alone, and under `(?orbit:case ...)` `ERROR` and `error` join it.

## Declarations

A declared shape or kind changes what a token is, and so changes the content echo keys: a message
declared as one token recurs as a whole rather than word by word. Every axis that reads tokens
takes the declarations a scan takes: `--lib`, `--shape`, `--shape-after`, `--kind`, `--let` and
`--declare` on the command line, `-Library` in PowerShell and `lib=` in Python
([pattern files](../../pattern-files/)); the [magnitude](../magnitude/#declarations) page shows
one at work.

## In a pattern

`@novel` holds on the first occurrence of a token's content and `@echoed` on content that recurs
elsewhere, before or after. `@echo>k`, `@echo=k` and `@echo<k` read the count; `@echo:nth=k` tests
which occurrence this one is, counting from one, and `nth=-1` counts back from the last;
`@echo:period` holds on a periodic content. A scope `(?orbit:G ...)` around an anchor counts up
to that group, and `@echoed:@other.log` reads a second input. The
[pattern syntax](../../pattern-syntax/#axis-predicates-and-anchors) page has every form.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@echo>2 \W' --text 'ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net'
[6..10] "disk"
[27..31] "disk"
[39..43] "disk"

$ trex scan '(?orbit:case @echo>2 \W)' --text 'ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net'
[0..5] "ERROR"
[6..10] "disk"
[27..31] "disk"
[33..38] "error"
[39..43] "disk"
[50..55] "ERROR"

$ trex scan '@echo:nth=2 \W' --text 'ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net'
[27..31] "disk"
[44..48] "full"
[50..55] "ERROR"
[71..75] "slow"
[76..79] "net"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = "ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net";
let found = |pat: &str| -> Vec<&str> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| &text[s.range()]).collect()
};
assert_eq!(found(r"@echo>2 \W"), ["disk", "disk", "disk"]);
assert_eq!(found(r"(?orbit:case @echo>2 \W)"), ["ERROR", "disk", "disk", "error", "disk", "ERROR"]);
assert_eq!(found(r"@echo:nth=2 \W"), ["disk", "full", "ERROR", "slow", "net"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> text = "ERROR disk full; WARN slow disk; error disk full; ERROR net down; warn slow net"
>>> [m.text for m in trex.Pattern(r"@echo>2 \W").scan(text)]
['disk', 'disk', 'disk']
>>> [m.text for m in trex.Pattern(r"@echo:nth=2 \W").scan(text)]
['disk', 'full', 'ERROR', 'slow', 'net']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@echo>2 \W' -InputObject $line -Raw
disk
disk
disk
```
{{< /tab >}}
{{< /tabs >}}

## Data model

`EchoFrame`, one token's reading: its span `start..end`, whether it is `keyed`, its content's
`count`, `back_lag` and `fwd_lag` (the bytes to the previous and the next occurrence, none at
either end), `nth` (which occurrence it is) and `period` (the content's mean spacing where it is
periodic, 0 otherwise). `EchoField` holds a frame at every token with `keyed`, `distinct`,
`novel` and `echoed` counts, and `novelty()` and `echo_rate()`; `EchoConfig` holds `orbit` (the
group, `Identity` by default), `min_len` (1) and `max_period_cv` (0.3). `analyze_with(tokens, bytes, &cfg)` reads under it, and
`analyze_super_with(bytes, &cfg)` returns the recurring supertoken structures.

## Algorithm

One pass keys each token's content, under the group's canonical form where one is named, in a
map from key to its occurrences; a second pass reads each token's count, back-lag and novelty
from the map. A content's period is the mean of its successive spacings where their coefficient
of variation is at most `max_period_cv`.
