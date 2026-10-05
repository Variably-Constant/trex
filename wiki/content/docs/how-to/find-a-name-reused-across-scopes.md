---
title: Find a name reused across scopes
linkTitle: Find a name reused across scopes
weight: 48
---

Find where repeated content crosses from one bracket depth to another, such as a name bound
outside a block and used inside it, and compare terms up to the names they bind
([relation](../../reference/axes/relation/)).

```console
$ cat totals.js
let total = 0;
for (const item of items) {
  if (item.ok) { total = total + item.size; }
}
report(total);
```

## Where a reuse crosses a scope

A reuse joins a later occurrence of repeated content to the earlier one. Its residual is how many
brackets deeper the later occurrence is, and the holonomy sums the residuals without their
sign, 0 for text whose every reuse stays at one depth:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex relation totals.js --edges
trex relation: 106 bytes, 56 tokens, max depth 2, nesting load 34
... the readings and every edge ...
  reuse chords (from -> to, residual = scope jump):
          item -> item       residual +1
         total -> total      residual +2
         total -> total      residual +0
          item -> item       residual +0
         total -> total      residual -1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let code = b"let total = 0;\nfor (const item of items) {\n  if (item.ok) { total = total + item.size; }\n}\nreport(total);\n";
let field = trex::relation::analyze_bytes(code);
let residuals: Vec<i32> = field.chords.iter().map(|c| c.residual).collect();
assert_eq!(residuals, [1, 2, 0, 0, -1]);
assert_eq!((field.holonomy(), field.net_holonomy(), field.twisted_chords()), (4, 2, 3));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.relation(path="totals.js", detail=True)
>>> r.reuse_chords, r.scope_crossing_chords, r.holonomy, r.net_holonomy
(5, 3, 4, 2)
>>> [(c.from_, c.residual, c.from_offset, c.to_offset) for c in r.chords]
[('item', 1, 26, 49), ('total', 2, 4, 60), ('total', 0, 60, 68), ('item', 0, 49, 76), ('total', -1, 68, 98)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexRelation -Path ./totals.js | Select-Object ReuseChords, ScopeCrossingChords, Holonomy, NetHolonomy | Format-List

ReuseChords         : 5
ScopeCrossingChords : 3
Holonomy            : 4
NetHolonomy         : 2

PS> (Measure-TrexRelation -Path ./totals.js -Detail).Chords | Format-Table

From  To    Residual FromOffset ToOffset
----  --    -------- ---------- --------
item  item  1        26         49
total total 2        4          60
total total 0        60         68
item  item  0        49         76
total total -1       68         98
```
{{< /tab >}}
{{< /tabs >}}

`total` is declared at depth 0 and first used two brackets in, inside the `for` block and the
`if` block, the reuse of residual +2; the loop variable `item` reaches one bracket deeper. The net
holonomy keeps the signs, so +2 says the reuse flows inward on the whole: the final
`report(total)` comes back out by one.

## Terms equal up to the names they bind

The first word inside a `[` binds a name for the rest of the group. The canonical form writes
that binder as `#` and each later use of it as `^k`, the binder `k` groups out, so two terms that
differ only in the names they bind read alike, and a name used in its binder's place stands out:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex relation --canonical --text 'map [item (item.size)] items'
... the readings ...
  canonical form (alpha-equivalence):
    map [ # ( ^0 . size ) ] items

$ trex relation --canonical --text 'map [row (row.size)] items'
... the readings ...
  canonical form (alpha-equivalence):
    map [ # ( ^0 . size ) ] items

$ trex relation --canonical --text 'map [row (item.size)] items'
... the readings ...
  canonical form (alpha-equivalence):
    map [ # ( item . size ) ] items
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::gauge::{alpha_equivalent, canonicalize_bytes};

assert!(alpha_equivalent(b"map [item (item.size)] items", b"map [row (row.size)] items"));
assert_eq!(canonicalize_bytes(b"map [row (item.size)] items"), "map [ # ( item . size ) ] items");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> terms = ["map [item (item.size)] items", "map [row (row.size)] items", "map [row (item.size)] items"]
>>> [trex.axes.relation(term, canonical=True).canonical for term in terms]
['map [ # ( ^0 . size ) ] items', 'map [ # ( ^0 . size ) ] items', 'map [ # ( item . size ) ] items']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'map [item (item.size)] items', 'map [row (row.size)] items', 'map [row (item.size)] items' | Measure-TrexRelation -Canonical | Select-Object -ExpandProperty Canonical
map [ # ( ^0 . size ) ] items
map [ # ( ^0 . size ) ] items
map [ # ( item . size ) ] items
```
{{< /tab >}}
{{< /tabs >}}

The third term binds `row` and never uses it, reading the outer `item` instead: its canonical form
keeps `item` as a name bound nowhere, where the other two read the binder.
