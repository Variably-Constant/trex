---
title: Find tables with no delimiters
linkTitle: Find tables
weight: 30
---

Find the tables in text and in a tree of files, whatever their separators and column widths.
A table is found by its token shape repeating row after row: a field is one token whatever its
width, so a ragged table still repeats ([shape](../../reference/axes/shape/#region-fusion)).

```console
$ cat data/rows.csv
id,host,bytes,ms
1,alpha,1024,12
2,beta,2048,19
3,gamma,4096,31
4,delta,8192,44
5,epsilon,16384,57
6,zeta,32768,73
$ cat data/notes.md
# Reading a directory

The walk reports what it found rather than what it was asked for. A filter that
silently drops a file reads exactly the same as a directory that never held one, and
the reader cannot tell the two apart afterwards.
```

## Read a text's regions

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex spectral --classify data/rows.csv
trex spectral (region classification: shape x spectral): 115 bytes, 1 regions
    [       0..115     ] table(p7)  id,host,bytes,ms 1,alpha,102
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::shape::{RegionKind, classified_regions};

let rows = b"id,host,bytes,ms\n1,alpha,1024,12\n2,beta,2048,19\n3,gamma,4096,31\n4,delta,8192,44\n5,epsilon,16384,57\n6,zeta,32768,73\n";
assert_eq!(classified_regions(rows), [(0, 115, RegionKind::Table(7))]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> [(r.kind, r.offset, r.length, r.period) for r in trex.axes.spectral(path="data/rows.csv", classify=True).regions]
[('table', 0, 115, 7)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexSpectral -Path ./data/rows.csv -Classify).Regions

Offset Length Kind  Period
------ ------ ----  ------
0      115    Table 7
```
{{< /tab >}}
{{< /tabs >}}

The period is in tokens: seven, a row's four fields and three commas.

## Find the files that are tables

A file's kind is the region kind covering most of its bytes:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --files --texture data/
data/notes.md: prose
data/rows.csv: table, period 7

$ trex scan --files --texture table data/
data/rows.csv
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
std::fs::create_dir_all(dir.join("data")).expect("a directory");
std::fs::write(dir.join("data/rows.csv"), "id,host,bytes,ms\n1,alpha,1024,12\n2,beta,2048,19\n3,gamma,4096,31\n4,delta,8192,44\n5,epsilon,16384,57\n6,zeta,32768,73\n").expect("written");
std::fs::write(dir.join("data/notes.md"), "# Reading a directory\n\nThe walk reports what it found rather than what it was asked for. A filter that\nsilently drops a file reads exactly the same as a directory that never held one, and\nthe reader cannot tell the two apart afterwards.\n").expect("written");
std::env::set_current_dir(&dir).expect("the temporary directory");

let (sources, errors) = trex::files::collect(&["data/".to_string()], &trex::files::WalkOptions::default());
assert!(errors.is_empty());
let tables: Vec<String> = sources
    .iter()
    .filter(|s| {
        let bytes = trex::files::read_source(s).expect("a readable file");
        trex::shape::dominant_kind(&bytes).is_some_and(|k| k.named("table"))
    })
    .map(trex::files::Source::name)
    .collect();
assert_eq!(tables, ["data/rows.csv"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [(name, trex.texture(path=name)) for name in trex.files("data/")]
[('data/notes.md', ('prose', 0)), ('data/rows.csv', ('table', 7))]
>>> trex.files("data/", texture=["table"])
['data/rows.csv']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexFile ./data -Classify | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, Texture, Period

File     Texture Period
----     ------- ------
notes.md   Prose      0
rows.csv   Table      7

PS> Get-TrexFile ./data -Texture Table | Split-Path -Leaf
rows.csv
```
{{< /tab >}}
{{< /tabs >}}

## Scan only the tables

The same filter narrows a scan of a tree:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N{>10000}' --texture table data/
data/rows.csv:6:11: "16384"
data/rows.csv:7:8: "32768"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
std::fs::create_dir_all(dir.join("data")).expect("a directory");
std::fs::write(dir.join("data/rows.csv"), "id,host,bytes,ms\n1,alpha,1024,12\n2,beta,2048,19\n3,gamma,4096,31\n4,delta,8192,44\n5,epsilon,16384,57\n6,zeta,32768,73\n").expect("written");
std::fs::write(dir.join("data/notes.md"), "# Reading a directory\n\nThe walk reports what it found rather than what it was asked for. A filter that\nsilently drops a file reads exactly the same as a directory that never held one, and\nthe reader cannot tell the two apart afterwards.\n").expect("written");
std::env::set_current_dir(&dir).expect("the temporary directory");

let pat = trex::parse(r"\N{>10000}").expect("valid pattern");
let (sources, _) = trex::files::collect(&["data/".to_string()], &trex::files::WalkOptions::default());
let mut found = Vec::new();
for source in &sources {
    let bytes = trex::files::read_source(source).expect("a readable file");
    if trex::shape::dominant_kind(&bytes).is_some_and(|k| k.named("table")) {
        found.extend(trex::scan(&pat, &bytes).iter().map(|s| String::from_utf8_lossy(&bytes[s.range()]).into_owned()));
    }
}
assert_eq!(found, ["16384", "32768"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> big = trex.Pattern(r"\N{>10000}")
>>> [m.text for name in trex.files("data/", texture=["table"]) for m in big.scan(trex.read(name))]
['16384', '32768']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N{>10000}' -Path ./data -Texture Table | Select-Object LineNumber, Column, Text

LineNumber Column Text
---------- ------ ----
         6     11 16384
         7      8 32768
```
{{< /tab >}}
{{< /tabs >}}

`!table`, `-ExcludeTexture` in PowerShell and `texture_not=` in Python drop the tables instead.
