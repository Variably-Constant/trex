---
title: Find deep nesting
linkTitle: Find deep nesting
weight: 22
---

# Find deep nesting

Find the tokens sitting deepest inside paired brackets, in code or in data, and read how deep an
input nests ([stress](../../reference/axes/stress/)).

```console
$ cat deep.json
{"a": {"b": {"c": {"d": 1}}}, "e": 2}
```

## The tokens past a depth

`@nested>=4` holds on a token inside at least four paired brackets:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@nested>=4 .' deep.json
[19..22] "\"d\""
[22..23] ":"
[24..25] "1"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"@nested>=4 .").expect("valid pattern");
let json = r#"{"a": {"b": {"c": {"d": 1}}}, "e": 2}"#;
let found: Vec<&str> = trex::scan(&pat, json.as_bytes()).iter().map(|s| &json[s.range()]).collect();
assert_eq!(found, [r#""d""#, ":", "1"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"@nested>=4 .").scan(open("deep.json").read())]
['"d"', ':', '1']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@nested>=4 .' -Path ./deep.json -Raw
"d"
:
1
```
{{< /tab >}}
{{< /tabs >}}

An unpaired bracket is a character of the text and opens no level.

## How deep an input goes

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex stress --text 'f(g(h(x))) and y(z)'
trex stress: 19 bytes, 15 tokens, max depth 3, 2 peak(s), 1 fracture(s)
  peak load: 10 (depth 2, strain 4)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::stress::analyze_bytes(b"f(g(h(x))) and y(z)");
assert_eq!(field.max_depth, 3);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexStress 'f(g(h(x))) and y(z)').MaxDepth
3
```
{{< /tab >}}
{{< /tabs >}}

`--peaks` lists where the depth peaks and `--fractures` where it starts to fall.
