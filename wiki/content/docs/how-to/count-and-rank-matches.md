---
title: Count and rank matches
linkTitle: Count and rank matches
weight: 7
---

Group the matches of a pattern by a key rendered at each, rank the keys, and total a typed
value per key. The keys and aggregates are on [aggregates](../../reference/aggregates/).

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

## Rank the most frequent

The key is a template: here the verb each request bound. `top` puts the most frequent first,
`count-by` orders by key:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex top '\I:ip \W:verb' '${verb}' access.log
GET   3
POST  1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::aggregate::{Order, Table};

let log = b"10.0.0.5 GET /index.html 200 5120\n10.0.0.7 GET /login 302 0\n10.0.1.9 GET /missing 404 312\n10.0.0.5 POST /login 200 88\n";
let pat = trex::parse(r"\I:ip \W:verb").expect("valid pattern");
let key = trex::Template::parse_report("${verb}", &pat.capture_names()).expect("valid key");
let at = trex::ReportAt { path: "access.log", line: 0, col: 0, offsets: None, pattern: None, rule: None };
let mut table = Table::new(Vec::new());
for m in trex::captures(&pat, log, &trex::scan(&pat, log)) {
    table.add(key.render_report(&m, log, &at), &m, log);
}
let rows: Vec<(&str, u64)> = table.rows(Order::Count).iter().map(|r| (r.key, r.count)).collect();
assert_eq!(rows, [("GET", 3), ("POST", 1)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> log = open("access.log").read()
>>> trex.Pattern(r"\I:ip \W:verb").count_by("${verb}", log, order="count")
[('GET', 3), ('POST', 1)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Group-TrexMatch '\I:ip \W:verb' -Key '${verb}' -Path ./access.log

Key  Count
---  -----
GET      3
POST     1
```
{{< /tab >}}
{{< /tabs >}}

## Count by part of a value

A typed slice in the key groups by part of what a register bound, here the first three octets
of each address:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex count-by '\I:ip' '${ip:octet1-3}' access.log
10.0.0  3
10.0.1  1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::aggregate::{Order, Table};

let log = b"10.0.0.5 GET /index.html 200 5120\n10.0.0.7 GET /login 302 0\n10.0.1.9 GET /missing 404 312\n10.0.0.5 POST /login 200 88\n";
let pat = trex::parse(r"\I:ip").expect("valid pattern");
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
>>> [(g.key, g.count) for g in trex.Pattern(r"\I:ip").group_by("${ip:octet1-3}", path="access.log")]
[('10.0.0', 3), ('10.0.1', 1)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Group-TrexMatch '\I:ip' -Key '${ip:octet1-3}' -Path ./access.log -SortBy Key

Key    Count
---    -----
10.0.0     3
10.0.1     1
```
{{< /tab >}}
{{< /tabs >}}

## Total a value per key

`--sum` adds a column over one register's typed value, in its base unit: bytes for a size.
`--avg`, `--min`, `--max` and `--pN`, the Nth percentile, work the same way.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex count-by '\W:h \Z:s' '${h}' sizes.txt --sum '${s}' --min '${s}' --max '${s}'
       count  --sum s  --min s  --max s
alpha      3     7000     1000     4000
beta       1     8000     8000     8000
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::aggregate::{Order, Table};
use trex::typed::{Agg, Percentile, QuotientForm, ValueStyle};

let sizes = b"alpha 1000B 80ms\nalpha 2000B 80ms\nalpha 4000B 80ms\nbeta 8000B 300ms\n";
let pat = trex::parse(r"\W:h \Z:s").expect("valid pattern");
let asked = [(Agg::Sum, "s".to_string()), (Agg::Min, "s".to_string()), (Agg::Max, "s".to_string())];
let columns =
    trex::aggregate::columns(&asked, &pat.capture_kinds(), Percentile::Nearest).expect("a size sums and orders");
let key = trex::Template::parse_report("${h}", &pat.capture_names()).expect("valid key");
let at = trex::ReportAt { path: "sizes.txt", line: 0, col: 0, offsets: None, pattern: None, rule: None };
let mut table = Table::new(columns);
for m in trex::captures(&pat, sizes, &trex::scan(&pat, sizes)) {
    table.add(key.render_report(&m, sizes, &at), &m, sizes);
}
let rows = table.rows(Order::Key);
let alpha: Vec<Option<String>> = table
    .columns()
    .iter()
    .zip(&rows[0].values)
    .map(|(col, held)| {
        held.and_then(|v| {
            v.report(col.agg, col.kind, ValueStyle::new(), QuotientForm::Repetend, Percentile::Nearest, trex::Clock::current())
        })
    })
    .collect();
assert_eq!(rows[0].key, "alpha");
assert_eq!(alpha, [Some("7000".to_string()), Some("1000".to_string()), Some("4000".to_string())]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> groups = trex.Pattern(r"\W:h \Z:s").group_by("${h}", path="sizes.txt", sum="s", min="s", max="s")
>>> [(g.key, g.count, g.sum["s"], g.min["s"], g.max["s"]) for g in groups]
[('alpha', 3, 7000, 1000, 4000), ('beta', 1, 8000, 8000, 8000)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Group-TrexMatch '\W:h \Z:s' -Key '${h}' -Sum s -Minimum s -Maximum s -Path ./sizes.txt -SortBy Key | Format-Table Key, Count, Sum, Minimum, Maximum

Key   Count  Sum Minimum Maximum
---   -----  --- ------- -------
alpha     3 7000    1000    4000
beta      1 8000    8000    8000
```
{{< /tab >}}
{{< /tabs >}}

Every key is printed; `-n N` keeps the first N rows and says how many there were.
