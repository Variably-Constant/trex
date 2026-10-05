---
title: Your first patterns
linkTitle: Your first patterns
weight: 20
---

The atoms, alternatives, repetition and the named structural shapes called lenses: the part of
trex that overlaps with a regex.

## The token atoms

Each atom matches one token of a kind:

| Atom | Matches | Atom | Matches |
|---|---|---|---|
| `\N` | a number | `\U` | a URL |
| `\W` | a word or identifier | `\T` | a timestamp or date |
| `\Q` | a quoted string | `\P` | a punctuation token |
| `\I` | an IP address | `.` | any one token |
| `\E` | an email address | `"lit"` | a token equal to `lit` |

The [pattern syntax](../../reference/pattern-syntax/#token-atoms) lists versions, UUIDs, money,
durations and the rest. An email is one token, so `\E` matches it whole.

## Alternatives and repetition

`A | B` matches either; `+` one or more, `*` zero or more, `?` one or none, and `{m,n}` between m
and n:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N | \E' --text 'id 7 mail a@b.com'
[3..4] "7"
[10..17] "a@b.com"

$ trex scan '\N+' --text 'coords 1 2 3 stop'
[7..12] "1 2 3"

$ trex scan '\N{2,3}' --text '1 2 3 4 5'
[0..5] "1 2 3"
[6..9] "4 5"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"\N | \E", "id 7 mail a@b.com"), ["7", "a@b.com"]);
assert_eq!(found(r"\N+", "coords 1 2 3 stop"), ["1 2 3"]);
assert_eq!(found(r"\N{2,3}", "1 2 3 4 5"), ["1 2 3", "4 5"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"\N | \E").scan("id 7 mail a@b.com")]
['7', 'a@b.com']
>>> [m.text for m in trex.Pattern(r"\N+").scan("coords 1 2 3 stop")]
['1 2 3']
>>> [m.text for m in trex.Pattern(r"\N{2,3}").scan("1 2 3 4 5")]
['1 2 3', '4 5']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N | \E' -InputObject 'id 7 mail a@b.com' -Raw
7
a@b.com

PS> Select-TrexMatch '\N+' -InputObject 'coords 1 2 3 stop' -Raw
1 2 3

PS> Select-TrexMatch '\N{2,3}' -InputObject '1 2 3 4 5' -Raw
1 2 3
4 5
```
{{< /tab >}}
{{< /tabs >}}

`+` takes as many as it can, so `\N+` is one match of three numbers, and `{2,3}` cuts a longer run
into the largest pieces it allows.

## Lenses

A lens names a structural shape found across languages. `@call` is an identifier followed by a
balanced parenthesis group:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@call' --text 'foo(1) bar(2, 3)'
[0..6] "foo(1)"
[7..16] "bar(2, 3)"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse("@call").expect("valid pattern");
let text = "foo(1) bar(2, 3)";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["foo(1)", "bar(2, 3)"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern("@call").scan("foo(1) bar(2, 3)")]
['foo(1)', 'bar(2, 3)']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@call' -InputObject 'foo(1) bar(2, 3)' -Raw
foo(1)
bar(2, 3)
```
{{< /tab >}}
{{< /tabs >}}

`@block`, `@kv`, `@flag`, `@list` and `@range` name the others ([lenses](../../reference/pattern-syntax/#lenses)).

## Where next

[Binding and balance](../binding-and-balance/) covers named registers with back-references and
balanced brackets.
