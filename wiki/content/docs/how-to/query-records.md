---
title: Query the records of a log
linkTitle: Query records
weight: 6
---

Keep the entries of a log that hold, or lack, the patterns you name, where an entry runs over
several lines. The units and quantifiers are on [records](../../reference/records/).

```console
$ cat events.log
2026-09-15T10:00:00Z login ok
  user bob@x.com from 10.0.0.1
2026-09-15T10:01:00Z login failed
  user amy@y.org from 10.0.0.2
2026-09-15T10:02:00Z logout
  user bob@x.com
```

## Entries that lack a pattern

A timestamp at the start of a line begins an entry, so `^ \T` makes each entry one record. Keep
the entries holding no address:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --none -e '\I' --record-start '^ \T' events.log
2026-09-15T10:02:00Z logout
  user bob@x.com
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::{Quantifier, Query, RecordUnit};

let events = b"2026-09-15T10:00:00Z login ok\n  user bob@x.com from 10.0.0.1\n2026-09-15T10:01:00Z login failed\n  user amy@y.org from 10.0.0.2\n2026-09-15T10:02:00Z logout\n  user bob@x.com\n";
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
>>> import trex
>>> events = open("events.log").read()
>>> starts = [m.start for m in trex.Pattern(r"^ \T").scan(events)] + [len(events)]
>>> entries = list(zip(starts, starts[1:]))
>>> found = trex.Pattern(r"\I").scan(events)
>>> [events[s:e] for s, e in entries if not any(m.start < e and m.end > s for m in found)]
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

## Entries that hold every pattern

`--all` keeps an entry holding each pattern; a pattern may carry a typed predicate, here an
address inside one network:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --all -e '\E' -e '\I{in:10.0.0.0/24}' --record-start '^ \T' events.log
2026-09-15T10:00:00Z login ok
  user bob@x.com from 10.0.0.1
2026-09-15T10:01:00Z login failed
  user amy@y.org from 10.0.0.2
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::{Quantifier, Query, RecordUnit};

let events = b"2026-09-15T10:00:00Z login ok\n  user bob@x.com from 10.0.0.1\n2026-09-15T10:01:00Z login failed\n  user amy@y.org from 10.0.0.2\n2026-09-15T10:02:00Z logout\n  user bob@x.com\n";
let query = Query {
    quantifier: Quantifier::All,
    positives: vec![trex::parse(r"\E").expect("valid"), trex::parse(r"\I{in:10.0.0.0/24}").expect("valid")],
    set: None,
    negatives: Vec::new(),
    unit: RecordUnit::Start(trex::parse(r"^ \T").expect("valid")),
};
assert_eq!(query.hits(events, &trex::ShapeSet::new(), false).len(), 2);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> mail = trex.Pattern(r"\E").scan(events)
>>> local = trex.Pattern(r"\I{in:10.0.0.0/24}").scan(events)
>>> holds = lambda found, s, e: any(m.start < e and m.end > s for m in found)
>>> [events[s:e].split("\n")[0] for s, e in entries if holds(mail, s, e) and holds(local, s, e)]
['2026-09-15T10:00:00Z login ok', '2026-09-15T10:01:00Z login failed']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Find-TrexRecord '\E', '\I{in:10.0.0.0/24}' -Path ./events.log -RecordStart '^ \T' -All | Select-Object LineNumber

LineNumber
----------
         1
         3
```
{{< /tab >}}
{{< /tabs >}}

`--any` keeps an entry holding one of the patterns, `--at-least N` one holding N of them, and
`--not PATTERN` drops an entry holding that pattern under any of them. `--count` counts the
entries kept, and `--record paragraph`, `--record unit:assign` and the other units cut the
records another way.
