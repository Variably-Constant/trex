---
title: Declare an atom for one command
linkTitle: Declare an atom for one command
weight: 71
---

Declare a shape, a kind or a sub-pattern for the one command that reads it, with no pattern
file. Each takes what a [pattern file](../../reference/pattern-files/)'s line says after its
keyword, and each reads the declarations given before it.

```console
$ cat ops.log
2026-09-27T09:00:01Z OPS-1234 opened for C00042 by ann
2026-09-27T09:05:12Z DB-77 linked to OPS-1234
2026-09-27T09:07:40Z OPS-1250 opened for C00077 by bob
2026-09-27T09:12:03Z OPS-1234 closed for C00042
```

## A shape and a sub-pattern

`--shape` declares a token by its bytes, and `--let` a sub-pattern read in place of `\{name}`,
its captures with it:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{ticket}:t "opened" "for" \W \{by}' --shape 'ticket = `[A-Z]{2,5}-\d{1,5}`' --let 'by = "by" \W:who' ops.log --format '${t} ${who}'
OPS-1234 ann
OPS-1250 bob
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut lib = trex::ShapeSet::new();
lib.declare("ticket = `[A-Z]{2,5}-\\d{1,5}`", trex::Precedence::Before).expect("a bounded shape");
lib.declare_let(r#"by = "by" \W:who"#).expect("a sub-pattern that parses");
let pat = trex::parser::parse_with_shapes(r#"\{ticket}:t "opened" "for" \W \{by}"#, &lib).expect("valid pattern");
let log = b"2026-09-27T09:00:01Z OPS-1234 opened for C00042 by ann\n2026-09-27T09:05:12Z DB-77 linked to OPS-1234\n2026-09-27T09:07:40Z OPS-1250 opened for C00077 by bob\n2026-09-27T09:12:03Z OPS-1234 closed for C00042\n";
let found: Vec<(&[u8], &[u8])> = trex::captures_with_shapes(&pat, log, &lib, &trex::scan_with_shapes(&pat, log, &lib))
    .iter()
    .map(|m| (m.group("t", log).expect("bound"), m.group("who", log).expect("bound")))
    .collect();
assert_eq!(found, [(&b"OPS-1234"[..], &b"ann"[..]), (&b"OPS-1250"[..], &b"bob"[..])]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> lib = trex.Library()
>>> lib.shape("ticket", r"[A-Z]{2,5}-\d{1,5}")
>>> lib.let("by", r'"by" \W:who')
>>> p = trex.Pattern(r'\{ticket}:t "opened" "for" \W \{by}', lib=lib)
>>> [(m["t"], m["who"]) for m in p.scan(trex.read("ops.log"))]
[('OPS-1234', 'ann'), ('OPS-1250', 'bob')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $lib = New-TrexLibrary
PS> Register-TrexAtom ticket -Shape '[A-Z]{2,5}-\d{1,5}' -Library $lib
PS> Register-TrexAtom by -Pattern '"by" \W:who' -Library $lib
PS> Select-TrexMatch '\{ticket}:t "opened" "for" \W \{by}' -Path ./ops.log -Library $lib -Format '${t} ${who}'
OPS-1234 ann
OPS-1250 bob
```
{{< /tab >}}
{{< /tabs >}}

`--shape-after` declares a shape tried only where no built-in recognizer matched.

## A kind

`--kind` declares a token made of the tokens a pattern matches: after the lex, each match fuses
into one token of the kind, so a table counts it whole:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex tokens --text 'DB-77 linked to OPS-1234' --kind 'ticket = \W "-" \N'
[0..5] ticket "DB-77"
[6..12] word "linked"
[13..15] word "to"
[16..24] ticket "OPS-1234"

$ trex top '\{ticket}' '${0}' --kind 'ticket = \W "-" \N' ops.log
OPS-1234  3
DB-77     1
OPS-1250  1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::aggregate::{Order, Table};

let mut lib = trex::ShapeSet::new();
lib.declare_kind(r#"ticket = \W "-" \N"#).expect("a kind that parses");
let text = b"DB-77 linked to OPS-1234";
let tokens: Vec<(String, &[u8])> = trex::lexer::lex_with_shapes(text, &trex::lexer::blob_runs(text), &lib, 0)
    .iter()
    .filter(|t| t.is_significant())
    .map(|t| (lib.kind_name(t.kind), &text[t.start()..t.end()]))
    .collect();
assert_eq!(
    tokens,
    [
        ("ticket".to_string(), &b"DB-77"[..]),
        ("word".to_string(), &b"linked"[..]),
        ("word".to_string(), &b"to"[..]),
        ("ticket".to_string(), &b"OPS-1234"[..]),
    ]
);

let log = b"2026-09-27T09:00:01Z OPS-1234 opened for C00042 by ann\n2026-09-27T09:05:12Z DB-77 linked to OPS-1234\n2026-09-27T09:07:40Z OPS-1250 opened for C00077 by bob\n2026-09-27T09:12:03Z OPS-1234 closed for C00042\n";
let pat = trex::parser::parse_with_shapes(r"\{ticket}", &lib).expect("valid pattern");
let key = trex::Template::parse_report("${0}", &pat.capture_names()).expect("valid key");
let at = trex::ReportAt { path: "ops.log", line: 0, col: 0, offsets: None, pattern: None, rule: None };
let mut table = Table::new(Vec::new());
for m in trex::captures_with_shapes(&pat, log, &lib, &trex::scan_with_shapes(&pat, log, &lib)) {
    table.add(key.render_report(&m, log, &at), &m, log);
}
let rows: Vec<(&str, u64)> = table.rows(Order::Count).iter().map(|r| (r.key, r.count)).collect();
assert_eq!(rows, [("OPS-1234", 3), ("DB-77", 1), ("OPS-1250", 1)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> lib = trex.Library()
>>> lib.kind("ticket", r'\W "-" \N')
>>> [(t.kind, t.text) for t in trex.tokens("DB-77 linked to OPS-1234", lib=lib)]
[('ticket', 'DB-77'), ('word', 'linked'), ('word', 'to'), ('ticket', 'OPS-1234')]
>>> trex.Pattern(r"\{ticket}", lib=lib).count_by("${0}", trex.read("ops.log"), order="count")
[('OPS-1234', 3), ('DB-77', 1), ('OPS-1250', 1)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $lib = New-TrexLibrary
PS> Register-TrexAtom ticket -Kind '\W "-" \N' -Library $lib
PS> Get-TrexToken 'DB-77 linked to OPS-1234' -Library $lib | Select-Object Kind, Text

Kind   Text
----   ----
ticket DB-77
word   linked
word   to
ticket OPS-1234

PS> Group-TrexMatch '\{ticket}' -Key '${0}' -Path ./ops.log -Library $lib

Key      Count
---      -----
OPS-1234     3
DB-77        1
OPS-1250     1
```
{{< /tab >}}
{{< /tabs >}}

## A line from a pattern file

`--declare` takes a line as a pattern file writes it, a `shape-after`, `test`, `fields` or
`rule` line among them, so a line copied from a file works unchanged. It reads what a
`--lib` file before it declared:

```console
$ cat tickets.trex
shape ticket = `[A-Z]{2,5}-\d{1,5}`
shape customer = `C\d{5}`
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{opened}' --lib tickets.trex --declare 'kind opened = \{ticket} "opened" "for" \{customer}' ops.log
[21..47] "OPS-1234 opened for C00042"
[122..148] "OPS-1250 opened for C00077"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut lib = trex::Declarations::new();
lib.declare("shape ticket = `[A-Z]{2,5}-\\d{1,5}`", None).expect("a bounded shape");
lib.declare("shape customer = `C\\d{5}`", None).expect("a bounded shape");
lib.declare(r#"kind opened = \{ticket} "opened" "for" \{customer}"#, None).expect("a kind over the shapes");
let pat = trex::parser::parse_with_shapes(r"\{opened}", lib.set()).expect("valid pattern");
let log = b"2026-09-27T09:00:01Z OPS-1234 opened for C00042 by ann\n2026-09-27T09:05:12Z DB-77 linked to OPS-1234\n2026-09-27T09:07:40Z OPS-1250 opened for C00077 by bob\n2026-09-27T09:12:03Z OPS-1234 closed for C00042\n";
let found: Vec<&[u8]> = trex::scan_with_shapes(&pat, log, lib.set()).iter().map(|s| &log[s.range()]).collect();
assert_eq!(found, [&b"OPS-1234 opened for C00042"[..], b"OPS-1250 opened for C00077"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> lib = trex.Library.load("tickets.trex")
>>> lib.declare(r'kind opened = \{ticket} "opened" "for" \{customer}')
>>> [m.text for m in trex.Pattern(r"\{opened}", lib=lib).scan(trex.read("ops.log"))]
['OPS-1234 opened for C00042', 'OPS-1250 opened for C00077']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $lib = New-TrexLibrary -Path ./tickets.trex -Declaration 'kind opened = \{ticket} "opened" "for" \{customer}'
PS> Select-TrexMatch '\{opened}' -Path ./ops.log -Library $lib -Raw
OPS-1234 opened for C00042
OPS-1250 opened for C00077
```
{{< /tab >}}
{{< /tabs >}}

A refused declaration reads as the line refused in a pattern file, and as Python's
`lib.declare` and PowerShell's `-Declaration` refuse it:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{x}' --text 'a b' --kind 'x = ('
trex: line 1: pattern error at byte 1: expected ')'
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut lib = trex::Declarations::new();
let refused = lib.declare("kind x = (", None).expect_err("an unclosed group");
assert_eq!(refused.to_string(), "line 1: pattern error at byte 1: expected ')'");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Library().declare("kind x = (")
Traceback (most recent call last):
    ...
ValueError: line 1: pattern error at byte 1: expected ')'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> try { New-TrexLibrary -Declaration 'kind x = (' -ErrorAction Stop } catch { $_.Exception.Message }
line 1: pattern error at byte 1: expected ')'
```
{{< /tab >}}
{{< /tabs >}}
