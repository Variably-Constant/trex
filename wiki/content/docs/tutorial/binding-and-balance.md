---
title: Binding and balance
linkTitle: Binding and balance
weight: 30
---

Bind a token to a register and require a later token to equal it, and match balanced brackets as
one atom. Both run in one pass with no backtracking.

## Bind and compare

`A:name` binds what `A` matched to the register `name`; `=name` later matches a token equal to
it. A word repeated:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\W:x =x' --text 'the the cat'
[0..7] "the the"  captures: x="the"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\W:x =x").expect("valid pattern");
let text = b"the the cat";
let m = &trex::captures(&pat, text, &trex::scan(&pat, text))[0];
assert_eq!((&text[m.start..m.end], m.group("x", text)), (&b"the the"[..], Some(&b"the"[..])));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> m = trex.Pattern(r"\W:x =x").find("the the cat")
>>> m.text, m["x"]
('the the', 'the')
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\W:x =x' -InputObject 'the the cat' | ForEach-Object { $_.Text + ' / ' + $_.Captures.x }
the the / the
```
{{< /tab >}}
{{< /tabs >}}

`=shape name`, `=case name` and the other groups compare up to a symmetry: `\W:w =shape w` pairs
`cat` with `bat` ([match up to a symmetry](../../how-to/match-up-to-a-symmetry/)).

## Balanced brackets

The lexer pairs `()`, `[]` and `{}`, so a balanced group at any depth is the one atom `\B`, and
`\B(P)`, `\B[P]`, `\B{P}` a group of one kind whose inside matches `P`. With a binding, a tag
pair whose closing name must equal its opening one:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\B' --text 'f(a, b) g(c)'
[1..7] "(a, b)"
[9..12] "(c)"

$ trex scan '<\W:t>.*</=t>' --text '<div>hi</div> <b>x</span>'
[0..13] "<div>hi</div>"  captures: t="div"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"\B", "f(a, b) g(c)"), ["(a, b)", "(c)"]);
assert_eq!(found(r"<\W:t>.*</=t>", "<div>hi</div> <b>x</span>"), ["<div>hi</div>"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"\B").scan("f(a, b) g(c)")]
['(a, b)', '(c)']
>>> [m.text for m in trex.Pattern(r"<\W:t>.*</=t>").scan("<div>hi</div> <b>x</span>")]
['<div>hi</div>']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\B' -InputObject 'f(a, b) g(c)' -Raw
(a, b)
(c)

PS> Select-TrexMatch '<\W:t>.*</=t>' -InputObject '<div>hi</div> <b>x</span>' -Raw
<div>hi</div>
```
{{< /tab >}}
{{< /tabs >}}

`<b>x</span>` does not match, since `span` is not `b`.

## Look ahead for a literal

`~"lit"` holds where the literal occurs somewhere ahead, and `!~"lit"` where it does not:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\W ~"END"' --text 'begin middle END'
[0..5] "begin"
[6..12] "middle"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r#"\W ~"END""#).expect("valid pattern");
let text = "begin middle END";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["begin", "middle"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern('\\W ~"END"').scan("begin middle END")]
['begin', 'middle']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\W ~"END"' -InputObject 'begin middle END' -Raw
begin
middle
```
{{< /tab >}}
{{< /tabs >}}

## Where next

[Axes and tools](../axes-and-tools/) reads scale, recurrence and structure, and rewrites what a
pattern finds.
