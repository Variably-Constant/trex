---
title: Echo
linkTitle: Echo
weight: 90
---

# The echo axis

For each token, whether its content occurs elsewhere in the input, how often, how far away, and
how regularly, at any range.

Source: [`src/echo.rs`](https://github.com/Variably-Constant/trex/blob/main/src/echo.rs).

## The readings

Each keyed token - words, numbers, quoted strings and the typed literals, not punctuation or
brackets, which recur by grammar - carries four readings:

| Field | Meaning |
|---|---|
| `count` | occurrences of this token's key in the input, 1 for a key that occurs once |
| `back_lag` / `fwd_lag` | the byte distance to the previous and the next occurrence; no previous one is novel |
| `period` | the mean lag, where the key recurs three or more times with regular spacing |
| `strength()` | the recurrence mass, `count - 1` |

The key is the token's text under an [orbit group](../orbit/): under `case`, `Whale` and `whale`
are one echo; under `shape`, `1,22,3` and `4,55,6` are. Prose echoes without a period, since
function words recur often but irregularly; structured data echoes with one, a template
repeating every N bytes. On Moby Dick (1.23 MB) the field reads in a quarter of a second:

```console
$ trex echo moby.txt
trex echo: 1234609 bytes, 477505 tokens (219476 keyed, 19083 distinct), novelty 8.7%, echo rate 96.1%
  strongest echoes (top 8 of 10435; first occurrence, count, period):
    the                  x13813
    of                   x6592
... and the six under them ...
```

## Reading the axis

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex echo --text 'the whale swam and the whale sang of the whale'
trex echo: 46 bytes, 19 tokens (10 keyed, 6 distinct), novelty 60.0%, echo rate 60.0%
  strongest echoes (top 2 of 2; first occurrence, count, period):
    the                  x3     period ~18 B
    whale                x3     period ~18 B
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::echo::analyze_bytes(b"the whale swam and the whale sang of the whale");
assert_eq!((field.keyed, field.distinct), (10, 6));
assert!((field.novelty() - 0.6).abs() < 1e-6);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexEcho 'the whale swam and the whale sang of the whale' | Select-Object Keyed, Distinct, Novelty, EchoRate | Format-List

Keyed    : 10
Distinct : 6
Novelty  : 0.6
EchoRate : 0.6

PS> (Measure-TrexEcho 'the whale swam and the whale sang of the whale').Echoes | Format-Table

Key   Count Period Offset
---   ----- ------ ------
the   3     18.5   0
whale 3     18.5   4
```
{{< /tab >}}
{{< /tabs >}}

`--field` prints the per-token readings, `--orbit` names the group the key is read under,
`--super` reports the recurring supertoken structures and `--top` sizes the two ranked lists,
whose headers state the bound and the total. In PowerShell `-Group` names the group and
`-Detail` adds a frame at every token.

### Structural rhyme

`--super` keys each [supertoken](../../../explanation/architecture/) by its role and its token-kind
silhouette, so two units differing only in their identifiers and values are one structure. Six
distinct words, every one novel, and one recurring structure:

```console
$ trex echo --super --text 'alpha: one
bravo: two
delta: six'
trex echo: 32 bytes, 14 tokens (6 keyed, 6 distinct), novelty 100.0%, echo rate 0.0%
  structural rhyme (top 1 of 1 recurring supertoken structures):
    kv:W:W                       x3    period ~11 B
```

## In a pattern

`@novel` holds on the first occurrence of a token's content and `@echoed` on content that recurs
elsewhere, before or after; `@echo>k`, `@echo:nth=k` and `@echo:period` read the count, the
occurrence and the period, and `@echoed:@other.log` reads a second input. The
[pattern syntax](../../pattern-syntax/#axis-predicates-and-anchors) page has every form.
The field is built once a scan, only where a pattern names an echo anchor.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@novel \W' --text 'cat dog cat bird dog cat'
[0..3] "cat"
[4..7] "dog"
[12..16] "bird"

$ trex scan '@echoed \W' --text 'cat dog cat bird'
[0..3] "cat"
[8..11] "cat"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"@novel \W", "cat dog cat bird dog cat"), ["cat", "dog", "bird"]);
assert_eq!(found(r"@echoed \W", "cat dog cat bird"), ["cat", "cat"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"@novel \W").scan("cat dog cat bird dog cat")]
['cat', 'dog', 'bird']
>>> [m.text for m in trex.Pattern(r"@echoed \W").scan("cat dog cat bird")]
['cat', 'cat']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@novel \W' -InputObject 'cat dog cat bird dog cat' -Raw
cat
dog
bird

PS> Select-TrexMatch '@echoed \W' -InputObject 'cat dog cat bird' -Raw
cat
cat
```
{{< /tab >}}
{{< /tabs >}}

## Data model

`EchoField` holds one `EchoFrame` per token, index-aligned with the lexed stream, and the input's
summary: the `keyed`, `distinct`, `novel` and `echoed` counts and the `novelty()` and
`echo_rate()` fractions. `EchoConfig` sets the orbit group, the minimum keyed length and the
lag-regularity bound for a period (`max_period_cv`, 0.3 by default). `analyze_super` returns the
recurring supertoken structures (`SuperEcho`: key, count, period, first offset), most frequent
first.
