---
title: Scan many patterns at once
linkTitle: Scan many patterns at once
weight: 14
---

Ask several patterns together over one read of the input, each match reported under the pattern
that made it. A set of twenty costs one lex rather than twenty ([pattern
sets](../../reference/matching/#pattern-sets)).

In a pattern file a `let NAME = PATTERN` line is a member under its name and a bare pattern line
a member under its line number:

```console
$ cat rules.trex
# what a line may hold
let host = \I:addr
let mail = \E:e
\N{>=100}
$ cat notes.txt
from 10.0.0.1 at 500 to bob@x.com
x = 7 and 10.0.0.2
```

## Every match, under its member

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --patterns rules.trex notes.txt
[5..13] "10.0.0.1"  captures: addr="10.0.0.1"  pattern: host
[17..20] "500"  pattern: 4
[24..33] "bob@x.com"  captures: e="bob@x.com"  pattern: mail
[44..52] "10.0.0.2"  captures: addr="10.0.0.2"  pattern: host
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let rules = "# what a line may hold\nlet host = \\I:addr\nlet mail = \\E:e\n\\N{>=100}\n";
let notes = b"from 10.0.0.1 at 500 to bob@x.com\nx = 7 and 10.0.0.2\n";
let mut shapes = trex::ShapeSet::new();
let set = trex::PatternSet::from_text(rules, &mut shapes).expect("a valid pattern file");
let found: Vec<(String, &[u8])> = set.scan(notes).iter().map(|(member, span)| (set.name(*member), &notes[span.range()])).collect();
assert_eq!(found, [
    ("host".to_string(), &b"10.0.0.1"[..]),
    ("4".to_string(), &b"500"[..]),
    ("mail".to_string(), &b"bob@x.com"[..]),
    ("host".to_string(), &b"10.0.0.2"[..]),
]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> rules = trex.PatternSet.from_file("rules.trex")
>>> [(rules.names[i], m.text) for i, m in rules.scan(open("notes.txt").read())]
[('host', '10.0.0.1'), ('4', '500'), ('mail', 'bob@x.com'), ('host', '10.0.0.2')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch -PatternFile ./rules.trex -Path ./notes.txt | Select-Object Pattern, Text

Pattern Text
------- ----
host    10.0.0.1
4       500
mail    bob@x.com
host    10.0.0.2
```
{{< /tab >}}
{{< /tabs >}}

## Which patterns match a line

A set also answers which of its members match at all, without listing the matches. On the
command line `--single-match` gives that answer as each member's first match, one line for each
member that matches, numbered by its line in the file:

```console
$ cat kinds.trex
\E
\I
\U
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --patterns kinds.trex --single-match --text 'from 10.0.0.1 to bob@x.com'
[5..13] "10.0.0.1"  pattern: 2
[17..26] "bob@x.com"  pattern: 1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let set = trex::PatternSet::new(vec![
    trex::parse(r"\E").expect("valid"),
    trex::parse(r"\I").expect("valid"),
    trex::parse(r"\U").expect("valid"),
]);
assert_eq!(set.matches(b"from 10.0.0.1 to bob@x.com"), [0, 1]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.PatternSet([r"\E", r"\I", r"\U"]).matches("from 10.0.0.1 to bob@x.com")
[0, 1]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\E', '\I', '\U' -InputObject 'from 10.0.0.1 to bob@x.com' | Select-Object -ExpandProperty Pattern -Unique
\I
\E
```
{{< /tab >}}
{{< /tabs >}}

`--single-match` reports each member's first match and no more, and `--all`, `--any` and
`--none` keep the [records](../query-records/) holding a combination of members.
