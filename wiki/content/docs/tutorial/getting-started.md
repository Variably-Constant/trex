---
title: Getting started
linkTitle: Getting started
weight: 10
---

# Getting started

Install trex and run a first scan on the surface you use: the command line, Rust, Python or
PowerShell.

## Install

{{< tabs >}}
{{< tab name="CLI" >}}
The package on crates.io is `trex-re`; the command it installs is `trex`. It needs Rust 1.96 or
newer. The `trex` binary for Windows x64, Linux x64, FreeBSD x64 and macOS arm64 is also on the
[latest release](https://github.com/Variably-Constant/trex/releases/latest).

```console
$ cargo install trex-re
$ trex --version
trex 0.2.0
```
{{< /tab >}}
{{< tab name="Rust" >}}
```toml
[dependencies]
trex-re = "0.2.0"
```

The crate is named `trex` in code.
{{< /tab >}}
{{< tab name="Python" >}}
```console
$ pip install trex-re
```

The module is named `trex`; it needs Python 3.11 or newer.
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
Install-Module Trex
```
{{< /tab >}}
{{< /tabs >}}

## A first scan

A trex pattern matches **tokens**, not characters. `\N` is one number token:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N' --text 'order 42 shipped'
[6..8] "42"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\N").expect("valid pattern");
let text = "order 42 shipped";
let spans = trex::scan(&pat, text.as_bytes());
assert_eq!((spans[0].start(), spans[0].end(), &text[spans[0].range()]), (6, 8, "42"));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [(m.start, m.end, m.text) for m in trex.Pattern(r"\N").scan("order 42 shipped")]
[(6, 8, '42')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N' -InputObject 'order 42 shipped' | Select-Object Start, Length, Text

Start Length Text
----- ------ ----
    6      2 42
```
{{< /tab >}}
{{< /tabs >}}

Each match has its span, start inclusive and end exclusive, and its text. `order` and `shipped`
are word tokens, `\W`, so they do not match.

## Two tokens in a row

Whitespace between tokens is not written, so two atoms in sequence match two tokens in a row:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N \W' --text 'weight 12 crates'
[7..16] "12 crates"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\N \W").expect("valid pattern");
let text = "weight 12 crates";
assert_eq!(&text[trex::scan(&pat, text.as_bytes())[0].range()], "12 crates");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\N \W").find("weight 12 crates").text
'12 crates'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N \W' -InputObject 'weight 12 crates' -Raw
12 crates
```
{{< /tab >}}
{{< /tabs >}}

A number with a unit symbol reads differently: `12 kg` is one quantity token, matched by
`\{qty}`.

## Where next

[Your first patterns](../first-patterns/) covers the atoms, sequences, alternatives and
repetition, everything ordinary matching needs before the parts a regex cannot express.
