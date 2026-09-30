---
weight: 10
---

# Getting started

This chapter installs the binary and runs your first scan. It assumes a working Rust toolchain
(1.96 or newer) and nothing else.

## Install

trex is a single binary with no required system dependencies. It installs from crates.io,
where the package is `trex-re`; the command it installs is `trex`:

```console
$ cargo install trex-re
$ trex --version
trex 0.1.0
```

A checkout builds the same binary with `cargo build --release`, as `./target/release/trex`.
The rest of this guide writes `trex` for whichever binary you have.

## Your first scan

A trex pattern matches over **tokens**, not bytes. The atom `\N` means "one number token":

```console
$ trex scan '\N' --text 'order 42 shipped'
[6..8] "42"
```

The output is one line per match: the **byte span** `[start..end)` and the matched text. Only
`42` matched, because it is the only number. The words `order` and `shipped` are word tokens
(`\W`), not numbers.

{{< callout type="info" >}}
Every command takes either an inline `--text STRING` or a positional `FILE`. The examples use
`--text` so you can paste them; swap in a filename to scan a file.
{{< /callout >}}

## Reading the whole stream

Ask for word tokens instead, and both words match while the number is skipped:

```console
$ trex scan '\W' --text 'the year 2026'
[0..3] "the"
[4..8] "year"
```

Whitespace between tokens is insignificant, so you never write `\s*`. Two atoms in sequence
match two tokens in a row - a number followed by a word:

```console
$ trex scan '\N \W' --text 'weight 12 crates'
[7..16] "12 crates"
```

The match span covers `12 crates`: the number `12`, the (insignificant) space, and the word
`crates`. A unit symbol reads differently - `12 kg` is one quantity token, not a number and a
word - and `\{qty}` is the atom for those.

## Machine-readable output

Pass `--json` for a JSON array, one object per match, with any captured registers:

```console
$ trex scan '\N' --text 'order 42 shipped' --json
[{"start":6,"end":8,"text":"42","captures":{}}]
```

`captures` is empty here because this pattern binds nothing. You will fill it in
[Binding and balance](../binding-and-balance/).

## Where next

You now have a working binary and can read a match line. Next,
[your first patterns](../first-patterns/) covers the full set of atoms plus sequence,
alternation, and quantifiers - everything you need for ordinary matching before the parts a
regex cannot express.
