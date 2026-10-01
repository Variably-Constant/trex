---
title: Find where text changes kind
linkTitle: Find where text changes kind
weight: 25
---

# Find where text changes kind

Find the stretch of an input that is code inside prose, or data inside code, from the bytes
alone, with no delimiter or fence named ([spectral](../../reference/axes/spectral/)).

## The tokens of one texture

`\F{texture:code}+` is a run of tokens whose surrounding bytes read as code:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\F{texture:code}+' --text 'the quick brown fox jumps; fn add(a,b){let c=a+b;return c*2;}'
[33..61] "(a,b){let c=a+b;return c*2;}"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\F{texture:code}+").expect("valid pattern");
let text = "the quick brown fox jumps; fn add(a,b){let c=a+b;return c*2;}";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["(a,b){let c=a+b;return c*2;}"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"\F{texture:code}+").scan("the quick brown fox jumps; fn add(a,b){let c=a+b;return c*2;}")]
['(a,b){let c=a+b;return c*2;}']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\F{texture:code}+' -InputObject 'the quick brown fox jumps; fn add(a,b){let c=a+b;return c*2;}' -Raw
(a,b){let c=a+b;return c*2;}
```
{{< /tab >}}
{{< /tabs >}}

The reader reads forward, so `fn add` still reads as the prose before it.
`\F{texture:prose}`, `:math` and `:data` name the other textures, and `\F{onset}` holds on a token
where the reading changes.

## The regions of a file

`trex spectral --code-classify`, and `Measure-TrexSpectral -Classify`, cut a whole input into its
regions and name each: `table`, `code`, `prose`, `blob`, `numeric` or `mixed`. The
[find tables](../find-tables/) how-to walks a tree by the same reading.
