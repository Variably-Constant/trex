---
title: Axes and tools
linkTitle: Axes and tools
weight: 40
---

# Axes and tools

Read a property of every token, its scale here, and query it from a pattern; then rewrite what a
pattern finds.

## Read an axis

A token has a scale, a nesting depth, a texture and more; each is an [axis](../../reference/axes/),
read on its own or from a pattern. Magnitude is the order of magnitude of a token's value:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --text 'retries = 3 ; max_bytes = 5000000000'
trex magnitude: 36 bytes, 7 tokens, total energy 112.2, 3 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::magnitude::analyze_bytes(b"retries = 3 ; max_bytes = 5000000000");
let peak = field.frames.iter().map(|f| f.magnitude).fold(0.0f32, f32::max);
assert!((peak - 9.70).abs() < 0.01);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexMagnitude 'retries = 3 ; max_bytes = 5000000000').Peak

Offset Text       Magnitude Gradient Energy
------ ----       --------- -------- ------
26     5000000000 9.69897   9.69897  112.2273
```
{{< /tab >}}
{{< /tabs >}}

## Query it from a pattern

`\M{>6}` is a token more than six orders of magnitude big:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\M{>6}' --text 'retries = 3 ; max_bytes = 5000000000'
[26..36] "5000000000"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\M{>6}").expect("valid pattern");
let text = "retries = 3 ; max_bytes = 5000000000";
assert_eq!(&text[trex::scan(&pat, text.as_bytes())[0].range()], "5000000000");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"\M{>6}").scan("retries = 3 ; max_bytes = 5000000000")]
['5000000000']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\M{>6}' -InputObject 'retries = 3 ; max_bytes = 5000000000' -Raw
5000000000
```
{{< /tab >}}
{{< /tabs >}}

## Rewrite what a pattern finds

A template replaces each match: `${name}` renders a register and `${name:acc}` a slice of it:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex rewrite '\E:e' '[${e:domain}]' --text 'mail bob@x.com now'
mail [x.com] now
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\E:e").expect("valid pattern");
let tmpl = trex::Template::parse("[${e:domain}]", &pat.capture_names()).expect("valid template");
assert_eq!(trex::rewrite(&pat, &tmpl, b"mail bob@x.com now"), b"mail [x.com] now");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\E:e").rewrite("[${e:domain}]", "mail bob@x.com now")
'mail [x.com] now'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'mail bob@x.com now' | Edit-TrexText '\E:e' '[${e:domain}]'
mail [x.com] now
```
{{< /tab >}}
{{< /tabs >}}

## Where next

- [Beyond regex](../beyond-regex/) covers the rest of the pattern language.
- The [how-to guides](../../how-to/) solve one task each: grammars, prefilters, records, rules.
- The [reference](../../reference/) is the complete specification.
- The [explanation](../../explanation/) covers how the engine reads tokens in one pass.
