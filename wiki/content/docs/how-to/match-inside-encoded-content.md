---
title: Match inside encoded content
linkTitle: Match inside encoded content
weight: 16
---

Find tokens by what they encode: a JWT by its decoded header and claims, a base64 blob by the
text or the entropy of what it decodes to. Nothing is verified; a JWT's signature is not checked.
Every form is on [decoded content](../../reference/pattern-syntax/#decoded-content).

## A JWT that turns signing off

`\{jwt}{alg:none}` is a JWT whose decoded header names no algorithm:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{jwt}{alg:none}' --text 'auth eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig ok'
[5..69] "eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\{jwt}{alg:none}").expect("valid pattern");
let text = "auth eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig ok";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"\{jwt}{alg:none}").scan("auth eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig ok")]
['eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\{jwt}{alg:none}' -InputObject 'auth eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig ok' -Raw
eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig
```
{{< /tab >}}
{{< /tabs >}}

A claim is named the same way, `\{jwt}{role:admin}`, and an expiry compares with the clock,
`\{jwt}{exp<now}`.

## A key hidden in base64

`\{base64}{text:*BEGIN*}` is a blob whose decoded text holds `BEGIN`, and `\{base64}{bits>4.5}` one
whose decoded bytes carry more than 4.5 bits of entropy a byte:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{base64}{text:*BEGIN*}' --text 'key LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ== blob AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd end'
[4..48] "LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ=="

$ trex scan '\{base64}{bits>4.5}' --text 'key LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ== blob AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd end'
[54..94] "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let blobs = "key LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ== blob AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd end";
let found = |pat: &str| -> Vec<&str> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, blobs.as_bytes()).iter().map(|s| &blobs[s.range()]).collect()
};
assert_eq!(found(r"\{base64}{text:*BEGIN*}"), ["LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ=="]);
assert_eq!(found(r"\{base64}{bits>4.5}"), ["AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> blobs = "key LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ== blob AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd end"
>>> [m.text for m in trex.Pattern(r"\{base64}{text:*BEGIN*}").scan(blobs)]
['LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ==']
>>> [m.text for m in trex.Pattern(r"\{base64}{bits>4.5}").scan(blobs)]
['AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $blobs = 'key LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ== blob AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd end'
PS> Select-TrexMatch '\{base64}{text:*BEGIN*}' -InputObject $blobs -Raw
LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ==

PS> Select-TrexMatch '\{base64}{bits>4.5}' -InputObject $blobs -Raw
AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd
```
{{< /tab >}}
{{< /tabs >}}

`\{base64}{match:name}` runs a sub-pattern you declared over the decoded bytes.
