---
title: Find values that break their history
linkTitle: Find outliers
weight: 45
---

# Find values that break their history

Find the values out of scale with what came before them, with the threshold read from the
input at each token rather than written into the pattern. The baselines are on the
[context](../../reference/axes/context/) page.

## Against the tokens before it

`\N{>+1}` matches a number an order of magnitude above the window of tokens before it, and
`\N{>+2s}` one two of that window's standard deviations above it:

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
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["5000"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"\N{>+1}").scan("v 1 2 3 1 2 3 5000 2 3")]
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

## Against a key's own history

The window mixes keys. Bind the key and compare against the values bound to it before:
`\N{>+1:k}` reads the values that followed `=` or `:` after each earlier occurrence of what `k`
holds, so a large size does not hide a slow latency:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N{>+1}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[39..46] "5000000"
[74..79] "12000"

$ trex scan '"latency":k "=" \N{>+1:k}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[64..79] "latency = 12000"  captures: k="latency"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = "latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;";
let found = |pat: &str| -> Vec<&str> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| &text[s.range()]).collect()
};
assert_eq!(found(r"\N{>+1}"), ["5000000", "12000"]);
assert_eq!(found(r#""latency":k "=" \N{>+1:k}"#), ["latency = 12000"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> log = "latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;"
>>> [m.text for m in trex.Pattern(r"\N{>+1}").scan(log)]
['5000000', '12000']
>>> [m.text for m in trex.Pattern('"latency":k "=" \\N{>+1:k}').scan(log)]
['latency = 12000']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $log = 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
PS> Select-TrexMatch '\N{>+1}' -InputObject $log -Raw
5000000
12000

PS> Select-TrexMatch '"latency":k "=" \N{>+1:k}' -InputObject $log -Raw
latency = 12000
```
{{< /tab >}}
{{< /tabs >}}

The first occurrence of a key has no history and never matches.

## Against its column

In a record that repeats, `\N{>+2:phase}` compares a value with the earlier values in its own
column, with no delimiter named:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N{>+2:phase}' --text 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;'
[30..34] "9000"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\N{>+2:phase}").expect("valid pattern");
let rows = "a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;";
let found: Vec<&str> = trex::scan(&pat, rows.as_bytes()).iter().map(|s| &rows[s.range()]).collect();
assert_eq!(found, ["9000"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"\N{>+2:phase}").scan("a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;")]
['9000']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N{>+2:phase}' -InputObject 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;' -Raw
9000
```
{{< /tab >}}
{{< /tabs >}}

## Choose the delta

The delta is in orders of magnitude: `+1` is ten times the baseline, `+2` a hundred. With an `s`
it is in the baseline's standard deviations, which needs a baseline of more than one value. An
empty baseline, such as the first token or a key's first occurrence, matches nothing.
