---
title: Aggregates
linkTitle: Aggregates
weight: 60
---

# Aggregates

A table groups the matches of a pattern by a key rendered at each match, counts each group, and
can aggregate a register's typed values in it. `count-by` orders the rows by key, `top` by
count with the key breaking ties, and `uniq` prints the keys alone. The flags and parameters
are on the [CLI](../cli/#count-by-top-uniq) and
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
use std::collections::BTreeMap;

let log = b"10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
";
let pat = trex::parse(r"\I:ip \W:verb").expect("valid pattern");
let names = pat.capture_names();
let mut counts: BTreeMap<String, usize> = BTreeMap::new();
for m in trex::captures(&pat, log, &trex::scan(&pat, log)) {
    let key = trex::Matched::new(&m, log, &names).get("ip:octet1-3").expect("an address");
    *counts.entry(key).or_insert(0) += 1;
}
assert_eq!(counts.into_iter().collect::<Vec<_>>(), [("10.0.0".to_string(), 3), ("10.0.1".to_string(), 1)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> log = open("access.log").read()
>>> trex.Pattern(r"\I:ip \W:verb").count_by("${verb}", log, order="count")
[('GET', 3), ('POST', 1)]
>>> trex.Pattern(r"\I:ip").count_by("${ip:octet1-3}", log)
[('10.0.0', 3), ('10.0.1', 1)]
>>> [key for key, _ in trex.Pattern(r"\I:ip").count_by("${ip}", log)]
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
were, so a short table never reads as a complete one; Python returns every row and
`-MaxCount` writes the first groups. A key an accessor leaves empty prints as `-` and is
counted, so the rows sum to the matches.

## Aggregates

`--sum`, `--avg`, `--min`, `--max`, `--p50` and `--p95` each take one register and add a
column, computed in the value's base unit through the parse a value predicate compares with.
`--sum` and `--avg` take the numeric kinds: number, byte size, duration, money and percent.
`--min`, `--max` and the percentiles also take timestamp, version and address. An aggregate a
kind cannot carry is refused before anything is scanned, naming the register, its kind and the
kinds the aggregate takes; a key whose matches bound no value prints `-`.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex count-by '\W:h \Z:s' '${h}' sizes.txt --sum '${s}' --min '${s}' --max '${s}'
       count  --sum s  --min s  --max s
alpha      3     7000     1000     4000
beta       1     8000     8000     8000

$ trex count-by '\W:h \Z \R:d' '${h}' sizes.txt --sum '${d}' --duration-unit ms
       count  --sum d
alpha      3      240
beta       1      300
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::typed::TypedValue;

let sizes = b"alpha 1000B 80ms\nalpha 2000B 80ms\nalpha 4000B 80ms\nbeta 8000B 300ms\n";
let pat = trex::parse(r"\W:h \Z:s").expect("valid pattern");
let (names, kinds) = (pat.capture_names(), pat.capture_kinds());
let alpha = trex::captures(&pat, sizes, &trex::scan(&pat, sizes))
    .iter()
    .map(|m| trex::Matched::with_kinds(m, sizes, &names, &kinds))
    .filter(|m| m.group("h") == Some(&b"alpha"[..]))
    .filter_map(|m| match m.value("s") {
        Some(TypedValue::Num(bytes)) => Some(bytes),
        _ => None,
    })
    .reduce(|a, b| a.add(&b))
    .expect("alpha has sizes");
assert_eq!(alpha.to_text(), "7000");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found = trex.Pattern(r"\W:h \Z:s").scan(open("sizes.txt").read())
>>> alpha = [m.value("s") for m in found if m["h"] == "alpha"]
>>> sum(alpha), min(alpha), max(alpha)
(7000, 1000, 4000)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Group-TrexMatch '\W:h \Z:s' -Key '${h}' -Sum s -Minimum s -Maximum s -Path ./sizes.txt -SortBy Key | Format-Table Key, Count, Sum, Minimum, Maximum

Key   Count  Sum Minimum Maximum
---   -----  --- ------- -------
alpha     3 7000    1000    4000
beta      1 8000    8000    8000

PS> Group-TrexMatch '\W:h \Z \R:d' -Key '${h}' -Sum d -Path ./sizes.txt -SortBy Key | Select-Object Key, @{ n = 'Sum'; e = { $_.Sum.TotalMilliseconds } }

Key      Sum
---      ---
alpha 240.00
beta  300.00
```
{{< /tab >}}
{{< /tabs >}}

An aggregate names one plain register: `${u:host}` is a slice of text and `${a}/${b}` names two
values, so both are refused. In PowerShell each aggregate is a property of the group in the
.NET type that holds its kind: a number a `long` or a `decimal`, a duration a `TimeSpan`, a
timestamp a `DateTimeOffset`.

## Averages and percentiles

`--avg` is exact: `--avg-form repetend` (the default) brackets the repeating digits and
`rational` writes the fraction in lowest terms. A percentile falling between two observed
values is decided by `--percentile`: `nearest` (the default) takes the value at
`ceil(p x n)` in sorted order, `linear` interpolates and takes numeric kinds only, `lower`
takes the observed value at or below, and `hybrid` interpolates where the kind allows it and
takes an observed value where it does not.

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
use std::collections::BTreeMap;

let rules = r"# what a line may hold
let host = \I:addr
let mail = \E:e
\N{>=100}
";
let inputs: [&[u8]; 2] = [b"from 10.0.0.1 at 500 to bob@x.com\nx = 7 and 10.0.0.2\n", b"mail amy@y.org\n"];
let mut shapes = trex::ShapeSet::new();
let set = trex::PatternSet::from_text(rules, &mut shapes).expect("a valid pattern file");
let mut counts: BTreeMap<String, usize> = BTreeMap::new();
for input in inputs {
    for (member, _) in set.scan(input) {
        *counts.entry(set.name(member)).or_insert(0) += 1;
    }
}
assert_eq!(counts.into_iter().collect::<Vec<_>>(), [("4".to_string(), 1), ("host".to_string(), 2), ("mail".to_string(), 2)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> from collections import Counter
>>> rules = trex.PatternSet.from_file("rules.trex")
>>> found = [rules.names[i] for name in ("notes.txt", "other.txt") for i, _ in rules.scan(open(name).read())]
>>> sorted(Counter(found).items())
[('4', 1), ('host', 2), ('mail', 2)]
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

## JSON and windows

`--json` prints the rows as an array of objects, each aggregate under its flag's name and a
value with no aggregate as `null`; `--values` and `--duration-unit` spell the values as `scan
--json` spells them. `--head N`, `--tail N` and `--lines A..B` group the matches of one
[window](../windows/) of each input, and `--record UNIT` counts that window in records.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex top '\I:ip' '${ip}' access.log --tail 2
10.0.0.5  1
10.0.1.9  1
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\I:ip").count_by("${ip}", open("access.log").read(), order="count", tail=2)
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
