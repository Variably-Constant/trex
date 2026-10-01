---
title: Test a pattern file
linkTitle: Test a pattern file
weight: 12
---

# Test a pattern file

Write the texts each declaration must accept and reject beside it, run them, and read what a
failure says. The `test` line is on [pattern files](../../reference/pattern-files/#tests).

A name accepts a text when its match is the whole of it, and rejects a text when it matches
nowhere in it:

```console
$ cat wrong.trex
let rhs = \N
test rhs accepts "42" "\"bob\"" rejects "4 2"
```

## Run the tests

Each expectation not met says what was read instead, and a failing run exits with status 1:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex lib --test wrong.trex
wrong.trex:2: rhs accepts "\"bob\"": no match
  tokens: quoted "\"bob\""
wrong.trex:2: rhs rejects "4 2": matched "4" at 0..1
wrong.trex: 1 of 1 test failed
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut defs = trex::ShapeSet::new();
defs.declare_text("let rhs = \\N\ntest rhs accepts \"42\" \"\\\"bob\\\"\" rejects \"4 2\"").expect("declarations that parse");
let failures = defs.run_tests();
assert_eq!(failures.len(), 2);
assert_eq!(failures[0].name, "rhs");
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Test-TrexAtom -Library (New-TrexLibrary -Path ./wrong.trex) | Select-Object Name, Passed

Name Passed
---- ------
rhs   False
```
{{< /tab >}}
{{< /tabs >}}

A quoted string is not a number, and `4 2` holds one, so the declaration is wrong twice.

## Fix it and run again

```console
$ cat fixed.trex
let rhs = \N | \Q
test rhs accepts "42" "\"bob\"" rejects "forty-two"
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex lib --test fixed.trex
fixed.trex: 1 test passed
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut defs = trex::ShapeSet::new();
defs.declare_text("let rhs = \\N | \\Q\ntest rhs accepts \"42\" \"\\\"bob\\\"\" rejects \"forty-two\"").expect("declarations that parse");
assert!(defs.run_tests().is_empty());
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Test-TrexAtom -Library (New-TrexLibrary -Path ./fixed.trex) | Select-Object Name, Passed

Name Passed
---- ------
rhs    True
```
{{< /tab >}}
{{< /tabs >}}

`trex lib --test` with no file runs the shipped library's own tests, and several files declare
into one set in order, so a later file's tests may name an earlier file's declarations.
