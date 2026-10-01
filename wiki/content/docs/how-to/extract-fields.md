---
title: Extract fields
linkTitle: Extract fields
weight: 10
---

# Extract fields

Pull typed values, key and value pairs, and the columns of a table out of text.

## One typed value

Name the atom for the kind: `\E` an email address, `\N` a number, `\I` an IP address, `\U` a
URL, `\T` a timestamp, `\V` a version; the [token atoms](../../reference/pattern-syntax/#token-atoms)
lists them all.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\E' --text 'ping bob@x.com please'
[5..14] "bob@x.com"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\E").expect("valid pattern");
let text = "ping bob@x.com please";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["bob@x.com"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"\E").scan("ping bob@x.com please")]
['bob@x.com']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\E' -InputObject 'ping bob@x.com please' -Raw
bob@x.com
```
{{< /tab >}}
{{< /tabs >}}

## Key and value pairs

Bind each side to a register. A literal `=` is written `"="`, since a bare `=` is the
back-reference operator.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\W:k "=" \Q:v' --text 'name = "bob" city = "nyc"'
[0..12] "name = \"bob\""  captures: k="name", v="\"bob\""
[13..25] "city = \"nyc\""  captures: k="city", v="\"nyc\""
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r#"\W:k "=" \Q:v"#).expect("valid pattern");
let text = br#"name = "bob" city = "nyc""#;
let pairs: Vec<(&[u8], &[u8])> = trex::captures(&pat, text, &trex::scan(&pat, text))
    .iter()
    .map(|m| (m.group("k", text).expect("k is bound"), m.group("v", text).expect("v is bound")))
    .collect();
assert_eq!(pairs, [(&b"name"[..], &br#""bob""#[..]), (&b"city"[..], &br#""nyc""#[..])]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.captures for m in trex.Pattern(r'\W:k "=" \Q:v').scan('name = "bob" city = "nyc"')]
[{'k': 'name', 'v': '"bob"'}, {'k': 'city', 'v': '"nyc"'}]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\W:k "=" \Q:v' -InputObject 'name = "bob" city = "nyc"' | ForEach-Object { $_.Captures.k + ' ' + $_.Captures.v }
name "bob"
city "nyc"
```
{{< /tab >}}
{{< /tabs >}}

`--json` writes each match's registers under `captures`.

## The columns of a table

Write one row as a pattern, naming the columns to keep. `^` starts it at a line's first token,
and the header row does not match, since `age` is no number:

```console
$ cat people.csv
name,age,score
alice,30,95
bob,25,88
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '^ \W:name "," \N "," \N:score' people.csv --format '${name} ${score}'
alice 95
bob 88
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r#"^ \W:name "," \N "," \N:score"#).expect("valid pattern");
let csv = b"name,age,score\nalice,30,95\nbob,25,88\n";
let rows: Vec<(&[u8], &[u8])> = trex::captures(&pat, csv, &trex::scan(&pat, csv))
    .iter()
    .map(|m| (m.group("name", csv).expect("bound"), m.group("score", csv).expect("bound")))
    .collect();
assert_eq!(rows, [(&b"alice"[..], &b"95"[..]), (&b"bob"[..], &b"88"[..])]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> csv = open("people.csv").read()
>>> [(m["name"], m.value("score")) for m in trex.Pattern(r'^ \W:name "," \N "," \N:score').scan(csv)]
[('alice', 95), ('bob', 88)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '^ \W:name "," \N "," \N:score' -Path ./people.csv | ForEach-Object { $_.Captures.name + ' ' + $_.Captures.score }
alice 95
bob 88
```
{{< /tab >}}
{{< /tabs >}}

For lines whose layouts differ, [build the pattern from marked
examples](../build-and-reuse-a-pattern/) instead of writing it.
