---
title: Build a pattern once and reuse it
linkTitle: Build and reuse a pattern
weight: 5
---

Mark the fields in one line, let TREX build the pattern that extracts them from every line of
that shape, save it as a pattern file, and read new input with it from then on.

## Build it from a marked line

Write each value to extract as `{name:text}`, with `[int]` or another type before the name where
the value has one. The other lines are examples the pattern must also read:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer --mark 'GET https://{host:example.com}/a {[int]code:200}' 'GET https://trex.dev/b 404' --lib-file
# Built by trex infer from 2 lines in 1 shape; `extract` names every shape in order.
let extract_1 = ^ "GET" (\U):host (\N):code ~<($ .)
let extract = \{extract_1}
fields extract {host:host} {[int]code}
test extract accepts "GET https://example.com/a 200"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::infer::build::{Mint, Spec, build};

let marked = trex::infer::marks::parse_lines("GET https://{host:example.com}/a {[int]code:200}").expect("valid marks");
let lines = ["GET https://trex.dev/b 404".to_string()];
let shapes = trex::ShapeSet::new();
let spec = Spec { lines: &lines, marked: &marked, hints: &[], counters: &[], shapes: &shapes, unanchored: false, mint: Mint::Inline };
let file = build(&spec).expect("a pattern").file();
assert!(file.contains("fields extract {host:host} {[int]code}"));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> built = trex.infer(["GET https://trex.dev/b 404"], marked=["GET https://{host:example.com}/a {[int]code:200}"])
>>> print(built.file, end="")
# Built by trex infer from 2 lines in 1 shape; `extract` names every shape in order.
let extract_1 = ^ "GET" (\U):host (\N):code ~<($ .)
let extract = \{extract_1}
fields extract {host:host} {[int]code}
test extract accepts "GET https://example.com/a 200"
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $built = ConvertTo-TrexPattern -Example 'GET https://trex.dev/b 404' -Marked 'GET https://{host:example.com}/a {[int]code:200}'
PS> $built.File
# Built by trex infer from 2 lines in 1 shape; `extract` names every shape in order.
let extract_1 = ^ "GET" (\U):host (\N):code ~<($ .)
let extract = \{extract_1}
fields extract {host:host} {[int]code}
test extract accepts "GET https://example.com/a 200"
```
{{< /tab >}}
{{< /tabs >}}

`{host:host}` reads the host of the URL the field holds. The `fields` line keeps each field's
name, type and accessor beside the pattern; PowerShell casts a field to its `[type]`, and the
other surfaces give its text.

## Save it and read with it

Save that text as `hosts.trex`, then read new lines with `\{extract}`:

```console
$ cat hosts.trex
let extract_1 = ^ "GET" (\U):host (\N):code ~<($ .)
let extract = \{extract_1}
fields extract {host:host} {[int]code}
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{extract}' --lib hosts.trex --text 'GET https://example.org/x 500' --fields
record  lines  host         code
1       1      example.org  500
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::infer::build::{fields_for, read_records};

let mut shapes = trex::ShapeSet::new();
shapes.declare_text("let extract_1 = ^ \"GET\" (\\U):host (\\N):code ~<($ .)\nlet extract = \\{extract_1}\nfields extract {host:host} {[int]code}").expect("a pattern file");
let pattern = trex::parser::parse_with_shapes(r"\{extract}", &shapes).expect("valid pattern");
let fields = fields_for(r"\{extract}", &pattern, &shapes);
let records = read_records(&fields, &pattern, &shapes, "GET https://example.org/x 500").expect("records");
let host = fields.iter().position(|f| f.name == "host").expect("a host field");
assert_eq!(records[0].values[host].as_ref().expect("a host").joined(), "example.org");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\{extract}", lib="hosts.trex").records("GET https://example.org/x 500")
[{'lines': [0], 'values': {'host': 'example.org', 'code': '500'}}]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Import-TrexAtom ./hosts.trex
PS> 'GET https://example.org/x 500' | ConvertFrom-TrexText '\{extract}'

host        code
----        ----
example.org  500
```
{{< /tab >}}
{{< /tabs >}}

Lines of several layouts build one branch a layout; [a pattern that extracts
fields](../../reference/building-patterns/#a-pattern-that-extracts-fields) has the marks, lists,
records and declared shapes a build reads.
