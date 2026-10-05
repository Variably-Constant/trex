---
title: Aggregates
linkTitle: Aggregates
weight: 60
---

A table groups the matches of a pattern by a key rendered at each match, counts each group, and
can aggregate a register's typed values in it. `count-by` orders the rows by key, `top` by
count with the key breaking ties, and `uniq` prints the keys alone; Python's `group_by` and
`distinct` and Rust's `trex::aggregate::Table` build the same table. The flags and parameters
are on the [CLI](../cli/#count-by-top-uniq), [Python](../python/) and
[PowerShell](../powershell/grouping/) pages.

The examples read these files:

```console
$ cat access.log
10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
$ cat sizes.txt
alpha 1000B 80ms
alpha 2000B 80ms
alpha 4000B 80ms
beta 8000B 300ms
```

## Count by a key

The key is a report template, as `scan --format` takes one, so a register's typed slice is
what the rows count: `${ip:octet1-3}`, `${u:host}`, `${e:domain}`, or a composite such as
`${m:upper}/${u:host}`.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex top '\I:ip \W:verb' '${verb}' access.log
GET   3
POST  1

$ trex count-by '\I:ip' '${ip:octet1-3}' access.log
10.0.0  3
10.0.1  1

$ trex uniq '\I:ip' '${ip}' access.log
10.0.0.5
10.0.0.7
10.0.1.9
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::aggregate::{Order, Table};

let log = b"10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
";
let pat = trex::parse(r"\I:ip \W:verb").expect("valid pattern");
let key = trex::Template::parse_report("${ip:octet1-3}", &pat.capture_names()).expect("valid key");
let at = trex::ReportAt { path: "access.log", line: 0, col: 0, offsets: None, pattern: None, rule: None };
let mut table = Table::new(Vec::new());
for m in trex::captures(&pat, log, &trex::scan(&pat, log)) {
    table.add(key.render_report(&m, log, &at), &m, log);
}
let rows: Vec<(&str, u64)> = table.rows(Order::Key).iter().map(|r| (r.key, r.count)).collect();
assert_eq!(rows, [("10.0.0", 3), ("10.0.1", 1)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> log = open("access.log").read()
>>> trex.Pattern(r"\I:ip \W:verb").count_by("${verb}", log, order="count")
[('GET', 3), ('POST', 1)]
>>> [(g.key, g.count) for g in trex.Pattern(r"\I:ip").group_by("${ip:octet1-3}", path="access.log")]
[('10.0.0', 3), ('10.0.1', 1)]
>>> trex.Pattern(r"\I:ip").distinct("${ip}", path="access.log")
['10.0.0.5', '10.0.0.7', '10.0.1.9']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Group-TrexMatch '\I:ip \W:verb' -Key '${verb}' -Path ./access.log

Key  Count
---  -----
GET      3
POST     1

PS> Group-TrexMatch '\I:ip' -Key '${ip:octet1-3}' -Path ./access.log -SortBy Key

Key    Count
---    -----
10.0.0     3
10.0.1     1

PS> Group-TrexMatch '\I:ip' -Key '${ip}' -Path ./access.log -Unique
10.0.0.5
10.0.0.7
10.0.1.9
```
{{< /tab >}}
{{< /tabs >}}

Every key is printed. `-n N` cuts the table to N rows and says how many keys and matches there
were, so a short table never reads as a complete one; `limit=` and `-MaxCount` keep the first
groups. A key an accessor leaves empty prints as `-` and is counted, so the rows sum to the
matches.

## Aggregates

`--sum`, `--avg`, `--min` and `--max` each take a register and add a column, and `--pN` adds the
Nth percentile of one, N from 0 to 100; each repeats for another register. A column is computed
in the value's base unit through the parse a value predicate compares with. `--sum` and `--avg`
take the numeric kinds: number, byte size, duration, money and percent. `--min`, `--max` and the
percentiles also take timestamp, version and address. An aggregate a kind cannot carry is
refused before anything is scanned, naming the register, its kind and the kinds the aggregate
takes; a key whose matches bound no value prints `-`.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex count-by '\W:h \Z:s' '${h}' sizes.txt --sum '${s}' --min '${s}' --max '${s}'
       count  --sum s  --min s  --max s
alpha      3     7000     1000     4000
beta       1     8000     8000     8000

$ trex count-by '\W:h \Z:s \R:d' '${h}' sizes.txt --sum '${s}' --sum '${d}' --duration-unit ms
       count  --sum s  --sum d
alpha      3     7000      240
beta       1     8000      300
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::aggregate::{Order, Table};
use trex::typed::{Agg, Percentile, QuotientForm, ValueStyle};

let sizes = b"alpha 1000B 80ms\nalpha 2000B 80ms\nalpha 4000B 80ms\nbeta 8000B 300ms\n";
let pat = trex::parse(r"\W:h \Z:s").expect("valid pattern");
let asked = [(Agg::Sum, "s".to_string()), (Agg::Max, "s".to_string())];
let columns =
    trex::aggregate::columns(&asked, &pat.capture_kinds(), Percentile::Nearest).expect("a size sums and orders");
let key = trex::Template::parse_report("${h}", &pat.capture_names()).expect("valid key");
let at = trex::ReportAt { path: "sizes.txt", line: 0, col: 0, offsets: None, pattern: None, rule: None };
let mut table = Table::new(columns);
for m in trex::captures(&pat, sizes, &trex::scan(&pat, sizes)) {
    table.add(key.render_report(&m, sizes, &at), &m, sizes);
}
let rows = table.rows(Order::Key);
let cell = |r: usize, c: usize| {
    let col = &table.columns()[c];
    let held = rows[r].values[c]?;
    held.report(col.agg, col.kind, ValueStyle::new(), QuotientForm::Repetend, Percentile::Nearest, trex::Clock::current())
};
assert_eq!((rows[0].key, rows[0].count), ("alpha", 3));
assert_eq!(cell(0, 0).as_deref(), Some("7000"));
assert_eq!(cell(0, 1).as_deref(), Some("4000"));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> groups = trex.Pattern(r"\W:h \Z:s").group_by("${h}", path="sizes.txt", sum="s", min="s", max="s")
>>> [(g.key, g.count, g.sum["s"], g.min["s"], g.max["s"]) for g in groups]
[('alpha', 3, 7000, 1000, 4000), ('beta', 1, 8000, 8000, 8000)]
>>> trex.Pattern(r"\W:h \Z:s \R:d").group_by("${h}", path="sizes.txt", sum=["s", "d"], duration_unit="ms")[0].sum
{'s': 7000, 'd': 240}
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Group-TrexMatch '\W:h \Z:s' -Key '${h}' -Sum s -Minimum s -Maximum s -Path ./sizes.txt -SortBy Key | Format-Table Key, Count, Sum, Minimum, Maximum

Key   Count  Sum Minimum Maximum
---   -----  --- ------- -------
alpha     3 7000    1000    4000
beta      1 8000    8000    8000

PS> Group-TrexMatch '\W:h \Z:s \R:d' -Key '${h}' -Sum s, d -Path ./sizes.txt -SortBy Key | Select-Object Key, Sum_s, @{ n = 'Sum_d'; e = { $_.Sum_d.TotalMilliseconds } }

Key   Sum_s  Sum_d
---   -----  -----
alpha  7000 240.00
beta   8000 300.00
```
{{< /tab >}}
{{< /tabs >}}

An aggregate names one plain register: `${u:host}` is a slice of text and `${a}/${b}` names two
values, so both are refused, as is a register one aggregate names twice. In PowerShell each
aggregate is a property of the group in the .NET type that holds its kind: a number a `long` or
a `decimal`, a duration a `TimeSpan`, a timestamp a `DateTimeOffset`; one over several registers
is a property for each, `Sum_s` and `Sum_d`. In Python each is a dict by register, a value of it
the type `Match.value` gives.

## Averages and percentiles

`--avg` is exact: `--avg-form repetend` (the default) brackets the repeating digits and
`rational` writes the fraction in lowest terms; Python's average is a `fractions.Fraction`. A
percentile whose rank is between two observed values is decided by `--percentile`: `nearest`
(the default) takes the value at `ceil(p x n)` in sorted order, `linear` interpolates and takes
numeric kinds only, `lower` takes the observed value at or below, and `hybrid` interpolates where
the kind allows it and takes an observed value where it does not.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex count-by '\W:h \Z:s' '${h}' sizes.txt --avg '${s}'
       count   --avg s
alpha      3  2333.(3)
beta       1      8000

$ trex count-by '\W:h \Z:s' '${h}' sizes.txt --avg '${s}' --avg-form rational
       count  --avg s
alpha      3   7000/3
beta       1     8000

$ trex count-by '\N:n' all --text '10 20 30 40' --p50 '${n}'
     count  --p50 n
all      4       20

$ trex count-by '\N:n' all --text '10 20 30 40' --p50 '${n}' --percentile linear
     count  --p50 n
all      4       25

$ trex count-by '\N:n' all --text '10 20 30 40' --p25 '${n}' --p99 '${n}'
     count  --p25 n  --p99 n
all      4       10       40
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::aggregate::{Order, Table};
use trex::typed::{Agg, Percentile, QuotientForm, ValueStyle};

let text = b"10 20 30 40";
let pat = trex::parse(r"\N:n").expect("valid pattern");
let p50 = |how: Percentile| {
    let asked = [(Agg::Pct(50), "n".to_string())];
    let columns = trex::aggregate::columns(&asked, &pat.capture_kinds(), how).expect("a number orders");
    let mut table = Table::new(columns);
    for m in trex::captures(&pat, text, &trex::scan(&pat, text)) {
        table.add("all".to_string(), &m, text);
    }
    let col = table.columns()[0].clone();
    let rows = table.rows(Order::Key);
    let held = rows[0].values[0]?;
    held.report(col.agg, col.kind, ValueStyle::new(), QuotientForm::Repetend, how, trex::Clock::current())
};
assert_eq!(p50(Percentile::Nearest).as_deref(), Some("20"));
assert_eq!(p50(Percentile::Linear).as_deref(), Some("25"));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> nums = trex.Pattern(r"\N:n")
>>> nums.group_by("all", "1 2 4", avg="n")[0].avg
{'n': Fraction(7, 3)}
>>> nums.group_by("all", "10 20 30 40", percentiles={"n": (25, 50, 99)})[0].percentiles
{'n': {25: 10, 50: 20, 99: 40}}
>>> nums.group_by("all", "10 20 30 40", percentiles={"n": 50}, percentile_method="linear")[0].percentiles
{'n': {50: 25}}
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> '1 2 4' | Group-TrexMatch '\N:n' -Key all -Average n | ForEach-Object { $_.Average.GetType().Name + ' ' + $_.Average }
Decimal 2.3333333333333333333333333333

PS> '10 20 30 40' | Group-TrexMatch '\N:n' -Key all -Percentile 50 -PercentileOf n

Key Count P50
--- ----- ---
all     4  20

PS> '10 20 30 40' | Group-TrexMatch '\N:n' -Key all -Percentile 50 -PercentileOf n -PercentileMethod Linear

Key Count P50
--- ----- ---
all     4  25
```
{{< /tab >}}
{{< /tabs >}}

## Keys by place and by member

`${path}` counts the matches by input and `${line}` by line. Over a set, from `--patterns FILE`
or several patterns, `${pattern}` counts them by the member that made them.

```console
$ cat rules.trex
# what a line may hold
let host = \I:addr
let mail = \E:e
\N{>=100}
$ cat notes.txt
from 10.0.0.1 at 500 to bob@x.com
x = 7 and 10.0.0.2
$ cat other.txt
mail amy@y.org
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex count-by --patterns rules.trex '${pattern}' notes.txt other.txt
4     1
host  2
mail  2

$ trex count-by '\E' '${path}' notes.txt other.txt
notes.txt  1
other.txt  1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::aggregate::{Order, Table};

let rules = r"# what a line may hold
let host = \I:addr
let mail = \E:e
\N{>=100}
";
let inputs: [&[u8]; 2] = [b"from 10.0.0.1 at 500 to bob@x.com\nx = 7 and 10.0.0.2\n", b"mail amy@y.org\n"];
let mut shapes = trex::ShapeSet::new();
let set = trex::PatternSet::from_text(rules, &mut shapes).expect("a valid pattern file");
let key = trex::Template::parse_report("${pattern}", &set.capture_names()).expect("valid key");
let mut table = Table::new(Vec::new());
for input in inputs {
    for (member, m) in set.scan_matches(input, false) {
        let name = set.name(member);
        let at = trex::ReportAt { path: "", line: 0, col: 0, offsets: None, pattern: Some(&name), rule: None };
        table.add(key.render_report(&m, input, &at), &m, input);
    }
}
let rows: Vec<(&str, u64)> = table.rows(Order::Key).iter().map(|r| (r.key, r.count)).collect();
assert_eq!(rows, [("4", 1), ("host", 2), ("mail", 2)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> rules = trex.PatternSet.from_file("rules.trex")
>>> [(g.key, g.count) for g in rules.group_by("${pattern}", path=["notes.txt", "other.txt"])]
[('4', 1), ('host', 2), ('mail', 2)]
>>> [(g.key, g.count) for g in trex.Pattern(r"\E").group_by("${path}", path=["notes.txt", "other.txt"])]
[('notes.txt', 1), ('other.txt', 1)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Group-TrexMatch -PatternFile ./rules.trex -Key '${pattern}' -Path ./notes.txt, ./other.txt -SortBy Key

Key  Count
---  -----
4        1
host     2
mail     2

PS> Group-TrexMatch '\E' -Key '${path:name}' -Path ./notes.txt, ./other.txt -SortBy Key

Key       Count
---       -----
notes.txt     1
other.txt     1
```
{{< /tab >}}
{{< /tabs >}}

## JSON, windows and binary files

`--json` prints the rows as an array of objects, each aggregate under its flag's name and a
value with no aggregate as `null`; `--values` and `--duration-unit` spell the values as `scan
--json` spells them. `--head N`, `--tail N` and `--lines A..B` group the matches of one
[window](../windows/) of each input, and `--record UNIT` counts that window in records. A file
holding a NUL byte is passed over as a scan passes it: one named alone is refused aloud, and
`--binary`, `-Binary` or `binary=True` reads it.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex top '\I:ip' '${ip}' access.log --tail 2
10.0.0.5  1
10.0.1.9  1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::aggregate::{Order, Table};
use trex::records::RecordUnit;
use trex::window::{Select, window_of};

let log = b"10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
";
let tail = window_of(log, Select::Tail(2), &RecordUnit::Line).bytes;
let pat = trex::parse(r"\I:ip").expect("valid pattern");
let key = trex::Template::parse_report("${ip}", &pat.capture_names()).expect("valid key");
let at = trex::ReportAt { path: "access.log", line: 0, col: 0, offsets: None, pattern: None, rule: None };
let mut table = Table::new(Vec::new());
for m in trex::captures(&pat, &tail, &trex::scan(&pat, &tail)) {
    table.add(key.render_report(&m, &tail, &at), &m, &tail);
}
let rows: Vec<(&str, u64)> = table.rows(Order::Count).iter().map(|r| (r.key, r.count)).collect();
assert_eq!(rows, [("10.0.0.5", 1), ("10.0.1.9", 1)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\I:ip").count_by("${ip}", open("access.log").read(), order="count", tail=2)
[('10.0.0.5', 1), ('10.0.1.9', 1)]
>>> [(g.key, g.count) for g in trex.Pattern(r"\I:ip").group_by("${ip}", path="access.log", order="count", tail=2)]
[('10.0.0.5', 1), ('10.0.1.9', 1)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Group-TrexMatch '\I:ip' -Key '${ip}' -Path ./access.log -Tail 2

Key      Count
---      -----
10.0.0.5     1
10.0.1.9     1
```
{{< /tab >}}
{{< /tabs >}}
