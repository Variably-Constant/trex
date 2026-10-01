---
title: Match up to a symmetry
linkTitle: Match up to a symmetry
weight: 50
---

# Match up to a symmetry

Match text that is the same up to case, notation, word shape or a typed relation rather than
byte for byte. The groups are on the [orbit](../../reference/axes/orbit/) page.

## Every spelling of a word

`(?orbit:case ...)` compares every literal inside it with case folded:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '(?orbit:case "error")' --text 'Error: disk; ERROR: net; error: cpu'
[0..5] "Error"
[13..18] "ERROR"
[25..30] "error"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r#"(?orbit:case "error")"#).expect("valid pattern");
let text = "Error: disk; ERROR: net; error: cpu";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["Error", "ERROR", "error"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern('(?orbit:case "error")').scan("Error: disk; ERROR: net; error: cpu")]
['Error', 'ERROR', 'error']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '(?orbit:case "error")' -InputObject 'Error: disk; ERROR: net; error: cpu' -Raw
Error
ERROR
error
```
{{< /tab >}}
{{< /tabs >}}

## A later token equal up to a symmetry

`=G name` matches a token equal to the one `name` bound under group `G`: `=shape` the same
consonant and vowel pattern, `=case` the same letters, `=subnet` an address in the same
network.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\W:w =shape w' --text 'cat bat xyz'
[0..7] "cat bat"  captures: w="cat"

$ trex scan '\I:a .*? =subnet a' --text 'from 10.0.0.1 to 10.0.0.77 and 192.168.1.1'
[5..26] "10.0.0.1 to 10.0.0.77"  captures: a="10.0.0.1"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"\W:w =shape w", "cat bat xyz"), ["cat bat"]);
assert_eq!(found(r"\I:a .*? =subnet a", "from 10.0.0.1 to 10.0.0.77 and 192.168.1.1"), ["10.0.0.1 to 10.0.0.77"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"\W:w =shape w").scan("cat bat xyz")]
['cat bat']
>>> [m.text for m in trex.Pattern(r"\I:a .*? =subnet a").scan("from 10.0.0.1 to 10.0.0.77 and 192.168.1.1")]
['10.0.0.1 to 10.0.0.77']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\W:w =shape w' -InputObject 'cat bat xyz' -Raw
cat bat

PS> Select-TrexMatch '\I:a .*? =subnet a' -InputObject 'from 10.0.0.1 to 10.0.0.77 and 192.168.1.1' -Raw
10.0.0.1 to 10.0.0.77
```
{{< /tab >}}
{{< /tabs >}}

`=subnet` reads a `/24` network for IPv4 and `/64` for IPv6, and `=subnet/16 a` names the width.
Every typed relation is in the [pattern syntax](../../reference/pattern-syntax/#binding-back-reference-and-symmetry).

## Count the forms of a vocabulary

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex orbit --collapse --group case --text 'Error ERROR error warn WARN'
trex orbit --collapse (group case): 5 raw forms -> 2 orbits (2.5x reduction)
    "error"  <-  ["ERROR", "Error", "error"]
    "warn"  <-  ["WARN", "warn"]
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::{OrbitGroup, canonical};

let mut orbits: Vec<String> = "Error ERROR error warn WARN".split(' ').map(|w| canonical(w.as_bytes(), OrbitGroup::Case)).collect();
orbits.sort();
orbits.dedup();
assert_eq!(orbits, ["error", "warn"]);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexOrbit 'Error ERROR error warn WARN' -Group case).Classes

Orbit Forms
----- -----
error {ERROR, Error, error}
warn  {WARN, warn}
```
{{< /tab >}}
{{< /tabs >}}
