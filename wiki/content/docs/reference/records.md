---
title: Records
linkTitle: Records
weight: 70
---

# Records

trex reads an input as tokens, and cuts it into records: lines, paragraphs, blocks, the runs
between two matches of a pattern, or units an axis finds with no delimiter at all. A record
query asks of each record which of several patterns it holds. Templates and patterns built
from records are on the [building patterns](../building-patterns/) page. The flags and
parameters are on the [CLI](../cli/#scan) and [PowerShell](../powershell/records/) pages.

## Tokens

Each token has a kind and a span; a typed token's text parses as its kind. Whitespace tokens
are kept in the stream, and a pattern never matches one.

{{< tabs >}}
{{< tab name="Rust" >}}
```rust
let text = b"retry 3 times in 1500ms from 10.0.0.1";
let kinds: Vec<(&str, &[u8])> = trex::lexer::lex(text)
    .iter()
    .filter(|t| t.is_significant())
    .map(|t| (t.kind.name(), &text[t.span()]))
    .collect();
assert_eq!(kinds[4], ("duration", &b"1500ms"[..]));
assert_eq!(kinds[6], ("ip", &b"10.0.0.1"[..]));
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexToken 'retry 3 times in 1500ms from 10.0.0.1'

Kind     Text     Start Length Value
----     ----     ----- ------ -----
word     retry    0     5
number   3        6     1      3
word     times    8     5
word     in       14    2
duration 1500ms   17    6      00:00:01.5000000
word     from     24    4
ip       10.0.0.1 29    8      10.0.0.1

PS> Get-TrexToken 'f(a, [b])' | Select-Object Kind, Text

Kind  Text
----  ----
word  f
open  (
word  a
punct ,
open  [
word  b
close ]
close )
```
{{< /tab >}}
{{< /tabs >}}

`Get-TrexToken` leaves whitespace out unless `-IncludeWhitespace` asks for it. The kinds are
the [token atoms](../pattern-syntax/#token-atoms) a pattern names.

## Record units

| Unit | A record is |
|---|---|
| `line` | a line, without its newline; the default |
| `paragraph` | a run of lines holding something other than whitespace |
| `file` | the whole input |
| `block` | a balanced bracket group, from its opening bracket to its closing one; the one unit whose records nest |
| `unit`, `unit:ROLE` | a supertoken, the statement, clause or argument list, of the role `call`, `assign`, `kv`, `list`, `numeric` or `plain` where one is named |
| `period` | the stream's own record period in supertokens; the whole input where it has none |
| `seam` | the [seam](../axes/seam/) axis's segments, cut where the past stops predicting the future |
| `bind`, `bind:Q` | the bytes cut where they hold together least under the input's pair field, as many cuts as `seam` makes, or a cut at every bond in the weakest `Q` per cent |
| `auto` | `seam` or `bind`, whichever puts more of its cuts at the input's line starts; `seam` where they tie |
| `texture` | the [spectral](../axes/spectral/) texture regions |
| `shape` | the [shape](../axes/shape/) axis's regions between its silhouette change-points |

`--record-start PATTERN` makes a record of each run from one match of the pattern to the next,
or to the end of the input, the bytes before the first match in no record; `--record-span
PATTERN` makes each match a record.

{{< tabs >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;

let text = b"a 1\nb 2\n\nc 3";
assert_eq!(RecordUnit::Paragraph.records(text), [(0, 7), (9, 12)]);
assert_eq!(RecordUnit::parse("line").expect("a unit").records(text), [(0, 3), (4, 7), (8, 8), (9, 12)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> trex.records("a 1\nb 2\n\nc 3", "paragraph")
[(0, 7), (9, 12)]
>>> trex.records("a 1\nb 2\n\nc 3", "line")
[(0, 3), (4, 7), (8, 8), (9, 12)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexRecord "a 1`nb 2`n`nc 3" -Unit paragraph | Format-List

Text   : a 1
         b 2
Start  : 0
Length : 7

Text   : c 3
Start  : 9
Length : 3

PS> Get-TrexRecord "a 1`nb 2`n`nc 3" | Select-Object Start, Length, Text

Start Length Text
----- ------ ----
    0      3 a 1
    4      3 b 2
    8      0
    9      3 c 3
```
{{< /tab >}}
{{< /tabs >}}

The same units cut a [window](../windows/), the [context](../matching/#context) printed around a
match, and the records a count or a table counts.

## Record queries

A query keeps the records holding every pattern (`--all`), at least one (`--any`), none
(`--none`) or at least N (`--at-least N`), and drops a record holding a pattern `--not` names.
A record definition or `--not` with no quantifier is `--any`. Every pattern is scanned once
over the whole input, and a record holds a pattern where one of its matches overlaps the
record, so `^` and `$` read lines and `\A` and `\z` the input whatever the record is.

```console
$ cat notes.txt
from 10.0.0.1
to bob@x.com

from 10.0.0.2
nothing

mail amy@y.org
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --all -e '\I' -e '\E' --record paragraph notes.txt
from 10.0.0.1
to bob@x.com

$ trex scan --any -e '\E' notes.txt
to bob@x.com
--
mail amy@y.org

$ trex scan --any -e '\I' -e '\E' --record paragraph --json notes.txt
[{"line":1,"start":0,"end":26,"text":"from 10.0.0.1\nto bob@x.com","patterns":[0,1]},{"line":4,"start":28,"end":49,"text":"from 10.0.0.2\nnothing","patterns":[0]},{"line":7,"start":51,"end":65,"text":"mail amy@y.org","patterns":[1]}]
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::{Quantifier, Query, RecordUnit};

let notes = b"from 10.0.0.1\nto bob@x.com\n\nfrom 10.0.0.2\nnothing\n\nmail amy@y.org\n";
let query = Query {
    quantifier: Quantifier::Any,
    positives: vec![trex::parse(r"\I").expect("valid"), trex::parse(r"\E").expect("valid")],
    set: None,
    negatives: Vec::new(),
    unit: RecordUnit::Paragraph,
};
let hits: Vec<(usize, usize, Vec<usize>)> = query
    .hits(notes, &trex::ShapeSet::new(), false)
    .into_iter()
    .map(|h| (h.start, h.end, h.present))
    .collect();
assert_eq!(hits, [(0, 26, vec![0, 1]), (28, 49, vec![0]), (51, 65, vec![1])]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> notes = open("notes.txt").read()
>>> found = trex.PatternSet([r"\I", r"\E"]).scan(notes)
>>> [(s, e, sorted({i for i, m in found if m.start < e and m.end > s})) for s, e in trex.records(notes, "paragraph")]
[(0, 26, [0, 1]), (28, 49, [0]), (51, 65, [1])]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Find-TrexRecord '\I', '\E' -Path ./notes.txt -Unit paragraph -All | Select-Object LineNumber, Text

LineNumber Text
---------- ----
         1 from 10.0.0.1…

PS> Find-TrexRecord '\I', '\E' -Path ./notes.txt -Unit paragraph | Select-Object LineNumber, Patterns

LineNumber Patterns
---------- --------
         1 {\I, \E}
         4 {\I}
         7 {\E}
```
{{< /tab >}}
{{< /tabs >}}

A record a query keeps prints as the lines it covers, each `path:line:text` over named inputs
and bare over one, with `--` between records that do not touch; `--count` counts the records,
`-l` and `-L` name the files, `-m N` keeps the first N records, and `--json` gives one object a
record with its first line, byte span, text and the patterns present, by index in the order
given or by name under `--patterns`. A query takes no `--format`, `--explain`, context lines,
`-v`, `--passthru`, `-x` or `--single-match`.

## Entries of several lines

A timestamp at the head of a line starts a log entry, so `--record-start '^ \T'` makes each
entry, however many lines it runs, one record:

```console
$ cat events.log
2026-09-15T10:00:00Z login ok
  user bob@x.com from 10.0.0.1
2026-09-15T10:01:00Z login failed
  user amy@y.org from 10.0.0.2
2026-09-15T10:02:00Z logout
  user bob@x.com
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --all -e '\E' -e '\I{in:10.0.0.0/24}' --record-start '^ \T' events.log
2026-09-15T10:00:00Z login ok
  user bob@x.com from 10.0.0.1
2026-09-15T10:01:00Z login failed
  user amy@y.org from 10.0.0.2

$ trex scan --none -e '\I' --record-start '^ \T' events.log
2026-09-15T10:02:00Z logout
  user bob@x.com

$ trex scan --any -e '\E' --record-start '^ \T' --count events.log
3
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::{Quantifier, Query, RecordUnit};

let events = b"2026-09-15T10:00:00Z login ok
  user bob@x.com from 10.0.0.1
2026-09-15T10:01:00Z login failed
  user amy@y.org from 10.0.0.2
2026-09-15T10:02:00Z logout
  user bob@x.com
";
let query = Query {
    quantifier: Quantifier::None,
    positives: vec![trex::parse(r"\I").expect("valid")],
    set: None,
    negatives: Vec::new(),
    unit: RecordUnit::Start(trex::parse(r"^ \T").expect("valid")),
};
let kept: Vec<&[u8]> = query.hits(events, &trex::ShapeSet::new(), false).iter().map(|h| &events[h.start..h.end]).collect();
assert_eq!(kept, [&b"2026-09-15T10:02:00Z logout\n  user bob@x.com\n"[..]]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> events = open("events.log").read()
>>> starts = [m.start for m in trex.Pattern(r"^ \T").scan(events)] + [len(events)]
>>> found = trex.Pattern(r"\I").scan(events)
>>> [events[s:e] for s, e in zip(starts, starts[1:]) if not any(m.start < e and m.end > s for m in found)]
['2026-09-15T10:02:00Z logout\n  user bob@x.com\n']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Find-TrexRecord '\I' -Path ./events.log -RecordStart '^ \T' -None | Select-Object LineNumber, Text

LineNumber Text
---------- ----
         5 2026-09-15T10:02:00Z logout…
```
{{< /tab >}}
{{< /tabs >}}

A supertoken is a record with no delimiter: `unit:assign` makes every binding one, whatever the
language.

```console
$ cat code.txt
x = 1
foo(3)
y = 3
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --any -e '\N{>=3}' --record unit:assign code.txt
y = 3
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Find-TrexRecord '\N{>=3}' -Path ./code.txt -Unit unit:assign | Select-Object LineNumber, Text

LineNumber Text
---------- ----
         3 y = 3
```
{{< /tab >}}
{{< /tabs >}}
