---
title: Beyond regex
linkTitle: Beyond regex
weight: 50
---

The rest of the regex surface, spelled over tokens, then the constructs with no regex
counterpart. The [pattern syntax](../../reference/pattern-syntax/) is the complete table.

## Position anchors

`^` and `$` hold at the first and last significant token of a line. `\A` and `\z` are the same
for the whole input. `\W \z` matches the word before the last token: `\z` holds at `gamma`, and
the word atom matches the token before it.

`\G` makes each match begin exactly where the previous one ended, so the run stops at the first
gap instead of skipping it. `\K` requires everything to its left and reports only what follows,
a lookbehind of any width:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\A \W' --text 'alpha beta gamma'
[0..5] "alpha"

$ trex scan '\W \z' --text 'alpha beta gamma'
[6..10] "beta"

$ trex scan '\G \N' --text '1 2 x 3 4'
[0..1] "1"
[2..3] "2"

$ trex scan '\N' --text '1 2 x 3 4'
[0..1] "1"
[2..3] "2"
[6..7] "3"
[8..9] "4"

$ trex scan '"key" ":" \W' --text 'key: value key: other'
[0..10] "key: value"
[11..21] "key: other"

$ trex scan '"key" ":" \K \W' --text 'key: value key: other'
[5..10] "value"
[16..21] "other"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"\A \W", "alpha beta gamma"), ["alpha"]);
assert_eq!(found(r"\W \z", "alpha beta gamma"), ["beta"]);
assert_eq!(found(r"\G \N", "1 2 x 3 4"), ["1", "2"]);
assert_eq!(found(r"\N", "1 2 x 3 4"), ["1", "2", "3", "4"]);
assert_eq!(found(r#""key" ":" \W"#, "key: value key: other"), ["key: value", "key: other"]);
assert_eq!(found(r#""key" ":" \K \W"#, "key: value key: other"), ["value", "other"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> found = lambda p, t: [m.text for m in trex.Pattern(p).scan(t)]
>>> found(r"\A \W", "alpha beta gamma"), found(r"\W \z", "alpha beta gamma")
(['alpha'], ['beta'])
>>> found(r"\G \N", "1 2 x 3 4"), found(r"\N", "1 2 x 3 4")
(['1', '2'], ['1', '2', '3', '4'])
>>> found(r'"key" ":" \W', "key: value key: other"), found(r'"key" ":" \K \W', "key: value key: other")
(['key: value', 'key: other'], ['value', 'other'])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\A \W' -InputObject 'alpha beta gamma' -Raw
alpha

PS> Select-TrexMatch '\W \z' -InputObject 'alpha beta gamma' -Raw
beta

PS> Select-TrexMatch '\G \N' -InputObject '1 2 x 3 4' -Raw
1
2

PS> Select-TrexMatch '\N' -InputObject '1 2 x 3 4' -Raw
1
2
3
4

PS> Select-TrexMatch '"key" ":" \W' -InputObject 'key: value key: other' -Raw
key: value
key: other

PS> Select-TrexMatch '"key" ":" \K \W' -InputObject 'key: value key: other' -Raw
value
other
```
{{< /tab >}}
{{< /tabs >}}

## Looking around

`~(P)` asserts that `P` matches at the current position without consuming it; `!~(P)` asserts
it does not. `~<(P)` and `!~<(P)` look backward, and need a `P` of bounded length:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\W ~(\N)' --text 'x 1 y z 2'
[0..1] "x"
[6..7] "z"

$ trex scan '\W !~(\N)' --text 'x 1 y z 2'
[4..5] "y"

$ trex scan '~<(\N) \W' --text 'x 1 y z 2'
[4..5] "y"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"\W ~(\N)", "x 1 y z 2"), ["x", "z"]);
assert_eq!(found(r"\W !~(\N)", "x 1 y z 2"), ["y"]);
assert_eq!(found(r"~<(\N) \W", "x 1 y z 2"), ["y"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found(r"\W ~(\N)", "x 1 y z 2"), found(r"\W !~(\N)", "x 1 y z 2"), found(r"~<(\N) \W", "x 1 y z 2")
(['x', 'z'], ['y'], ['y'])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\W ~(\N)' -InputObject 'x 1 y z 2' -Raw
x
z

PS> Select-TrexMatch '\W !~(\N)' -InputObject 'x 1 y z 2' -Raw
y

PS> Select-TrexMatch '~<(\N) \W' -InputObject 'x 1 y z 2' -Raw
y
```
{{< /tab >}}
{{< /tabs >}}

The literal guard `~"lit"` from [binding and balance](../binding-and-balance/#look-ahead-for-a-literal)
is the cheap case: a presence question over the forward window that a prefilter answers without
a positional scan.

## Three kinds of choice

`|` is a regex's alternation: the first branch that can match wins. `||` prefers the longest
overall match. `|>` commits to the first branch that matches and never reconsiders it, which is
a PEG's ordered choice:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '("a" | "a" "b") "c"' --text 'a b c'
[0..5] "a b c"

$ trex scan '("a" |> "a" "b") "c"' --text 'a b c'
no match

$ trex scan '\W | \W \W' --text 'a b'
[0..1] "a"
[2..3] "b"

$ trex scan '\W || \W \W' --text 'a b'
[0..3] "a b"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r#"("a" | "a" "b") "c""#, "a b c"), ["a b c"]);
assert!(found(r#"("a" |> "a" "b") "c""#, "a b c").is_empty());
assert_eq!(found(r"\W | \W \W", "a b"), ["a", "b"]);
assert_eq!(found(r"\W || \W \W", "a b"), ["a b"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found('("a" | "a" "b") "c"', "a b c"), found('("a" |> "a" "b") "c"', "a b c")
(['a b c'], [])
>>> found(r"\W | \W \W", "a b"), found(r"\W || \W \W", "a b")
(['a', 'b'], ['a b'])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '("a" | "a" "b") "c"' -InputObject 'a b c' -Raw
a b c

PS> @(Select-TrexMatch '("a" |> "a" "b") "c"' -InputObject 'a b c').Count
0

PS> Select-TrexMatch '\W | \W \W' -InputObject 'a b' -Raw
a
b

PS> Select-TrexMatch '\W || \W \W' -InputObject 'a b' -Raw
a b
```
{{< /tab >}}
{{< /tabs >}}

## Atomic groups and possessive quantifiers

`(?>P)` keeps only the match `P` prefers; nothing after it can make `P` give a token back.
`P*+`, `P++` and `P?+` are the same thing spelled on a quantifier:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N+ \N' --text '1 2 3'
[0..5] "1 2 3"

$ trex scan '(?>\N+) \N' --text '1 2 3'
no match
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"\N+ \N", "1 2 3"), ["1 2 3"]);
assert!(found(r"(?>\N+) \N", "1 2 3").is_empty());
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found(r"\N+ \N", "1 2 3"), found(r"(?>\N+) \N", "1 2 3")
(['1 2 3'], [])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N+ \N' -InputObject '1 2 3' -Raw
1 2 3

PS> @(Select-TrexMatch '(?>\N+) \N' -InputObject '1 2 3').Count
0
```
{{< /tab >}}
{{< /tabs >}}

## Matching up to a symmetry

A scope applies a symmetry to every literal inside it: `(?orbit:case ...)` is a regex's `(?i)`,
and `(?orbit:shape ...)` and `(?orbit:notation ...)` fold word shape and mathematical notation
the same way:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '"cat"' --text 'Cat CAT dog cat'
[12..15] "cat"

$ trex scan '(?orbit:case "cat")' --text 'Cat CAT dog cat'
[0..3] "Cat"
[4..7] "CAT"
[12..15] "cat"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r#""cat""#, "Cat CAT dog cat"), ["cat"]);
assert_eq!(found(r#"(?orbit:case "cat")"#, "Cat CAT dog cat"), ["Cat", "CAT", "cat"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found('"cat"', "Cat CAT dog cat"), found('(?orbit:case "cat")', "Cat CAT dog cat")
(['cat'], ['Cat', 'CAT', 'cat'])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '"cat"' -InputObject 'Cat CAT dog cat' -Raw
cat

PS> Select-TrexMatch '(?orbit:case "cat")' -InputObject 'Cat CAT dog cat' -Raw
Cat
CAT
cat
```
{{< /tab >}}
{{< /tabs >}}

## Token classes

A class is a set of single-token atoms: union `[\N \W]`, complement `[^\N]`, intersection
`[\W && \h]`, subtraction `[\W -- "if"]`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '[\W && \h]' --text 'deadbeef xyz cafe'
[0..8] "deadbeef"
[13..17] "cafe"

$ trex scan '[\W -- "if"]' --text 'if x if y'
[3..4] "x"
[8..9] "y"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"[\W && \h]", "deadbeef xyz cafe"), ["deadbeef", "cafe"]);
assert_eq!(found(r#"[\W -- "if"]"#, "if x if y"), ["x", "y"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found(r"[\W && \h]", "deadbeef xyz cafe"), found(r'[\W -- "if"]', "if x if y")
(['deadbeef', 'cafe'], ['x', 'y'])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '[\W && \h]' -InputObject 'deadbeef xyz cafe' -Raw
deadbeef
cafe

PS> Select-TrexMatch '[\W -- "if"]' -InputObject 'if x if y' -Raw
x
y
```
{{< /tab >}}
{{< /tabs >}}

## Inside a token

A byte-pattern between backticks is a regex over the characters of one token. `\p{L}` matches
any letter, in any script:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '`\p{L}+`' --text 'héllo wörld 123'
[0..6] "héllo"
[7..13] "wörld"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"`\p{L}+`").expect("valid pattern");
let text = "héllo wörld 123";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["héllo", "wörld"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found(r"`\p{L}+`", "héllo wörld 123")
['héllo', 'wörld']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '`\p{L}+`' -InputObject 'héllo wörld 123' -Raw
héllo
wörld
```
{{< /tab >}}
{{< /tabs >}}

## Shapes you declare

When your data has a kind the lexer does not know, a ticket id or a part number, declare it: a
name and a bounded byte-pattern, tried before the built-in recognizers and matched by name:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{ticket}' --shape 'ticket = `[A-Z]{2,4}-\d{1,4}`' --text 'see AB-12 and XYZ-9 now'
[4..9] "AB-12"
[14..19] "XYZ-9"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut shapes = trex::ShapeSet::new();
shapes.declare_text("shape ticket = `[A-Z]{2,4}-\\d{1,4}`").expect("a declaration that parses");
let pat = trex::parser::parse_with_shapes(r"\{ticket}", &shapes).expect("valid pattern");
let text = "see AB-12 and XYZ-9 now";
let found: Vec<&str> = trex::scan_with_shapes(&pat, text.as_bytes(), &shapes).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["AB-12", "XYZ-9"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> _ = open("ticket.trex", "w").write("shape ticket = `[A-Z]{2,4}-\\d{1,4}`\n")
>>> [m.text for m in trex.Pattern(r"\{ticket}", lib="ticket.trex").scan("see AB-12 and XYZ-9 now")]
['AB-12', 'XYZ-9']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Register-TrexAtom ticket '[A-Z]{2,4}-\d{1,4}'
PS> Select-TrexMatch '\{ticket}' -InputObject 'see AB-12 and XYZ-9 now' -Raw
AB-12
XYZ-9
```
{{< /tab >}}
{{< /tabs >}}

## Anchors on supertokens

Above tokens are [supertokens](../../explanation/architecture/#supertokens), such as a call, a binding, a
key/value entry or a list, read from punctuation and bracket shape and never from a language's
keywords. `@super` holds at the first token of a supertoken, and `@super:role` where the
supertoken containing the token has that role. A number inside a binding is a value and a number
inside a call is an argument:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@super:call \W' --text 'x = 1 ; foo(a, b) ; y : 2'
[8..11] "foo"

$ trex scan '@super:assign \N' --text 'x = 1 ; foo(a, b) ; y : 2'
[4..5] "1"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"@super:call \W", "x = 1 ; foo(a, b) ; y : 2"), ["foo"]);
assert_eq!(found(r"@super:assign \N", "x = 1 ; foo(a, b) ; y : 2"), ["1"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found(r"@super:call \W", "x = 1 ; foo(a, b) ; y : 2"), found(r"@super:assign \N", "x = 1 ; foo(a, b) ; y : 2")
(['foo'], ['1'])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@super:call \W' -InputObject 'x = 1 ; foo(a, b) ; y : 2' -Raw
foo

PS> Select-TrexMatch '@super:assign \N' -InputObject 'x = 1 ; foo(a, b) ; y : 2' -Raw
1
```
{{< /tab >}}
{{< /tabs >}}

The predictive anchors take a grain too. `@seam`, which is `@seam:token`, cuts where the
sequence of token kinds stops predicting itself, a break in the shape of a statement rather than
in a word; `@seam:byte` cuts where the bytes do, and `@seam:super` where the supertoken roles do:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@seam:token \W' --text 'let x = 1 ; let y = 2 ; print x ; print y ;'
[4..5] "x"
[34..39] "print"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"@seam:token \W").expect("valid pattern");
let text = "let x = 1 ; let y = 2 ; print x ; print y ;";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["x", "print"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found(r"@seam:token \W", "let x = 1 ; let y = 2 ; print x ; print y ;")
['x', 'print']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@seam:token \W' -InputObject 'let x = 1 ; let y = 2 ; print x ; print y ;' -Raw
x
print
```
{{< /tab >}}
{{< /tabs >}}

## Thresholds read from the stream

A threshold can be read from the stream at every token instead of written in the pattern as
`\N{>6}` is: from the window of tokens before the token, its column in a periodic record, or the
earlier values bound to a key. `\N{>+1}` is a number an order of magnitude above the mean of the
window before it:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N{>+1}' --text 'v 1 2 3 1 2 3 5000 2 3'
[14..18] "5000"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\N{>+1}").expect("valid pattern");
let text = "v 1 2 3 1 2 3 5000 2 3";
assert_eq!(&text[trex::scan(&pat, text.as_bytes())[0].range()], "5000");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found(r"\N{>+1}", "v 1 2 3 1 2 3 5000 2 3")
['5000']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N{>+1}' -InputObject 'v 1 2 3 1 2 3 5000 2 3' -Raw
5000
```
{{< /tab >}}
{{< /tabs >}}

`@phase:k` is column `k` of a periodic record, with no delimiter named: the period is read from
the record itself. `\N{>+2:phase}` is a value two orders above its own column:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@phase:2 \N' --text 'a , 1 , x ; b , 2 , y ; c , 3 , z ; d , 4 , w ;'
[4..5] "1"
[16..17] "2"
[28..29] "3"
[40..41] "4"

$ trex scan '\N{>+2:phase}' --text 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;'
[30..34] "9000"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"@phase:2 \N", "a , 1 , x ; b , 2 , y ; c , 3 , z ; d , 4 , w ;"), ["1", "2", "3", "4"]);
assert_eq!(found(r"\N{>+2:phase}", "a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;"), ["9000"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> found(r"@phase:2 \N", "a , 1 , x ; b , 2 , y ; c , 3 , z ; d , 4 , w ;")
['1', '2', '3', '4']
>>> found(r"\N{>+2:phase}", "a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;")
['9000']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@phase:2 \N' -InputObject 'a , 1 , x ; b , 2 , y ; c , 3 , z ; d , 4 , w ;' -Raw
1
2
3
4

PS> Select-TrexMatch '\N{>+2:phase}' -InputObject 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;' -Raw
9000
```
{{< /tab >}}
{{< /tabs >}}

With a key bound to a register, `\N{>+1:k}` is a value an order above every value bound to that
key before. The sizes below are large but bound to another key, so only the latency that breaks
its own history matches:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '"latency":k "=" \N{>+1:k}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[64..79] "latency = 12000"  captures: k="latency"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r#""latency":k "=" \N{>+1:k}"#).expect("valid pattern");
let text = b"latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;";
let m = &trex::captures(&pat, text, &trex::scan(&pat, text))[0];
assert_eq!((&text[m.start..m.end], m.group("k", text)), (&b"latency = 12000"[..], Some(&b"latency"[..])));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> m = trex.Pattern(r'"latency":k "=" \N{>+1:k}').find("latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;")
>>> m.text, m["k"]
('latency = 12000', 'latency')
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '"latency":k "=" \N{>+1:k}' -InputObject 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;' | ForEach-Object { $_.Text + ' / ' + $_.Captures.k }
latency = 12000 / latency
```
{{< /tab >}}
{{< /tabs >}}

The [how-to on outliers](../../how-to/find-outliers/) works through these on a log, and the
[context reference](../../reference/axes/context/) says what each context holds.

## You are done

The [pattern syntax reference](../../reference/pattern-syntax/) is the complete table, and the
[explanation](../../explanation/) says why none of it backtracks.
