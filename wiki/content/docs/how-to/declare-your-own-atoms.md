---
title: Declare your own atoms
linkTitle: Declare your own atoms
weight: 70
---

Declare the identifiers only your data has, ticket ids and customer numbers here, so each lexes
as one token a pattern names as `\{name}`; test them, then find, count and mask them. The forms
of a declaration are on [pattern files](../../reference/pattern-files/).

```console
$ cat ops.trex
shape ticket = `[A-Z]{2,5}-\d{1,5}`
shape customer = `C\d{5}`
test ticket accepts "OPS-1234" "DB-77" rejects "ops-1234" "OPS1234"
test customer accepts "C00042" rejects "C42"
$ cat ops.log
2026-09-27T09:00:01Z OPS-1234 opened for C00042 by ann
2026-09-27T09:05:12Z DB-77 linked to OPS-1234
2026-09-27T09:07:40Z OPS-1250 opened for C00077 by bob
2026-09-27T09:12:03Z OPS-1234 closed for C00042
```

## Test the declarations

Each `test` line passes when its `accepts` texts match whole and its `rejects` texts match
nowhere:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex lib --test ops.trex
ops.trex: 2 tests passed
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut ops = trex::ShapeSet::new();
ops.declare_text(r#"shape ticket = `[A-Z]{2,5}-\d{1,5}`
shape customer = `C\d{5}`
test ticket accepts "OPS-1234" "DB-77" rejects "ops-1234" "OPS1234"
test customer accepts "C00042" rejects "C42""#).expect("declarations that parse");
assert!(ops.run_tests().is_empty());
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> ops = trex.Library.load("ops.trex")
>>> [(t.name, t.passed) for t in ops.test()]
[('ticket', True), ('customer', True)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $ops = New-TrexLibrary -Path ./ops.trex
PS> Test-TrexAtom -Library $ops | Select-Object Name, Passed

Name     Passed
----     ------
ticket     True
customer   True
```
{{< /tab >}}
{{< /tabs >}}

## Find them

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{ticket}:t "opened" "for" \{customer}:c' --lib ops.trex ops.log --format '${line}: ${t} for ${c}'
1: OPS-1234 for C00042
3: OPS-1250 for C00077
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut ops = trex::ShapeSet::new();
ops.declare_text("shape ticket = `[A-Z]{2,5}-\\d{1,5}`\nshape customer = `C\\d{5}`").expect("declarations that parse");
let pat = trex::parser::parse_with_shapes(r#"\{ticket}:t "opened" "for" \{customer}:c"#, &ops).expect("valid pattern");
let log = b"2026-09-27T09:00:01Z OPS-1234 opened for C00042 by ann\n2026-09-27T09:05:12Z DB-77 linked to OPS-1234\n2026-09-27T09:07:40Z OPS-1250 opened for C00077 by bob\n2026-09-27T09:12:03Z OPS-1234 closed for C00042\n";
let found: Vec<(&[u8], &[u8])> = trex::captures_with_shapes(&pat, log, &ops, &trex::scan_with_shapes(&pat, log, &ops))
    .iter()
    .map(|m| (m.group("t", log).expect("bound"), m.group("c", log).expect("bound")))
    .collect();
assert_eq!(found, [(&b"OPS-1234"[..], &b"C00042"[..]), (&b"OPS-1250"[..], &b"C00077"[..])]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> log = open("ops.log").read()
>>> p = trex.Pattern(r'\{ticket}:t "opened" "for" \{customer}:c', lib="ops.trex")
>>> [(m["t"], m["c"]) for m in p.scan(log)]
[('OPS-1234', 'C00042'), ('OPS-1250', 'C00077')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Import-TrexAtom ./ops.trex
PS> Select-TrexMatch '\{ticket}:t "opened" "for" \{customer}:c' -Path ./ops.log -Format '${line}: ${t} for ${c}'
1: OPS-1234 for C00042
3: OPS-1250 for C00077
```
{{< /tab >}}
{{< /tabs >}}

`Import-TrexAtom` declares the file for the rest of the session; `-Library $ops` hands one
cmdlet the atoms instead, so a script reads the same atoms in any session.

## Count them

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex top '\{ticket}:t' '${t}' --lib ops.trex ops.log
OPS-1234  3
DB-77     1
OPS-1250  1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut ops = trex::ShapeSet::new();
ops.declare_text("shape ticket = `[A-Z]{2,5}-\\d{1,5}`").expect("a declaration that parses");
let pat = trex::parser::parse_with_shapes(r"\{ticket}", &ops).expect("valid pattern");
let log = b"OPS-1234 opened\nDB-77 linked to OPS-1234\nOPS-1250 opened\nOPS-1234 closed\n";
let mut counts: Vec<(&[u8], usize)> = Vec::new();
for span in trex::scan_with_shapes(&pat, log, &ops) {
    let key = &log[span.range()];
    match counts.iter_mut().find(|(k, _)| *k == key) {
        Some((_, n)) => *n += 1,
        None => counts.push((key, 1)),
    }
}
counts.sort_by(|a, b| b.1.cmp(&a.1));
assert_eq!(counts, [(&b"OPS-1234"[..], 3), (&b"DB-77"[..], 1), (&b"OPS-1250"[..], 1)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\{ticket}:t", lib="ops.trex").count_by("${t}", log, order="count")
[('OPS-1234', 3), ('DB-77', 1), ('OPS-1250', 1)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Group-TrexMatch '\{ticket}:t' -Key '${t}' -Path ./ops.log

Key      Count
---      -----
OPS-1234     3
DB-77        1
OPS-1250     1
```
{{< /tab >}}
{{< /tabs >}}

## Mask them

A pseudonym gives each distinct value the same stand-in wherever it occurs, named after the
atom, so the masked log still shows which lines are about one customer:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex redact '\{customer}' --mask pseudonym --lib ops.trex ops.log
2026-09-27T09:00:01Z OPS-1234 opened for CUSTOMER_1 by ann
2026-09-27T09:05:12Z DB-77 linked to OPS-1234
2026-09-27T09:07:40Z OPS-1250 opened for CUSTOMER_2 by bob
2026-09-27T09:12:03Z OPS-1234 closed for CUSTOMER_1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut ops = trex::ShapeSet::new();
ops.declare_text("shape customer = `C\\d{5}`").expect("a declaration that parses");
let pat = trex::parser::parse_with_shapes(r"\{customer}", &ops).expect("valid pattern");
let log = b"for C00042 then C00077 and C00042";
let matches = trex::captures_with_shapes(&pat, log, &ops, &trex::scan_with_shapes(&pat, log, &ops));
let mut mask = trex::Mask::parse("pseudonym").expect("a mask");
let names: Vec<Vec<u8>> = trex::redactions_with_shapes(log, &matches, &[], &mut mask, &pat, &ops)
    .into_iter()
    .map(|e| e.replacement)
    .collect();
assert_eq!(names, [b"CUSTOMER_1".to_vec(), b"CUSTOMER_2".to_vec(), b"CUSTOMER_1".to_vec()]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> print(trex.Pattern(r"\{customer}", lib="ops.trex").redact(log, mask="pseudonym"), end="")
2026-09-27T09:00:01Z OPS-1234 opened for CUSTOMER_1 by ann
2026-09-27T09:05:12Z DB-77 linked to OPS-1234
2026-09-27T09:07:40Z OPS-1250 opened for CUSTOMER_2 by bob
2026-09-27T09:12:03Z OPS-1234 closed for CUSTOMER_1
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Protect-TrexText '\{customer}' -Mask pseudonym -Path ./ops.log
2026-09-27T09:00:01Z OPS-1234 opened for CUSTOMER_1 by ann
2026-09-27T09:05:12Z DB-77 linked to OPS-1234
2026-09-27T09:07:40Z OPS-1250 opened for CUSTOMER_2 by bob
2026-09-27T09:12:03Z OPS-1234 closed for CUSTOMER_1
```
{{< /tab >}}
{{< /tabs >}}
