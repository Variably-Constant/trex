---
title: Count and rank matches
linkTitle: Count and rank matches
weight: 7
---

# Count and rank matches

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
let log = b"10.0.0.5 GET /index.html 200 5120\n10.0.0.7 GET /login 302 0\n10.0.1.9 GET /missing 404 312\n10.0.0.5 POST /login 200 88\n";
let pat = trex::parse(r"\I:ip \W:verb").expect("valid pattern");
let mut counts: Vec<(Vec<u8>, usize)> = Vec::new();
for m in trex::captures(&pat, log, &trex::scan(&pat, log)) {
    let verb = m.group("verb", log).expect("bound").to_vec();
    match counts.iter_mut().find(|(k, _)| *k == verb) {
        Some((_, n)) => *n += 1,
        None => counts.push((verb, 1)),
    }
}
counts.sort_by(|a, b| b.1.cmp(&a.1));
assert_eq!(counts, [(b"GET".to_vec(), 3), (b"POST".to_vec(), 1)]);
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
use std::collections::BTreeMap;

let log = b"10.0.0.5 GET /index.html 200 5120\n10.0.0.7 GET /login 302 0\n10.0.1.9 GET /missing 404 312\n10.0.0.5 POST /login 200 88\n";
let pat = trex::parse(r"\I:ip").expect("valid pattern");
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
>>> trex.Pattern(r"\I:ip").count_by("${ip:octet1-3}", log)
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
`--avg`, `--min`, `--max`, `--p50` and `--p95` work the same way.

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
```
{{< /tab >}}
{{< /tabs >}}

Every key is printed; `-n N` keeps the first N rows and says how many there were.
