---
title: Context
linkTitle: Context
weight: 100
---

At every token, the other axes' readings folded over what the token is read against: the window
before it, its column, the tokens since the last regime change, the heads of the brackets
enclosing it, its earlier occurrences, and the values bound to its key. A relative predicate in a
pattern (`\N{>+1}`, `\N{>+1:k}`) compares a token's reading with one of these.

Source: [`src/context.rs`](https://github.com/Variably-Constant/trex/blob/main/src/context.rs).

## What it reads that nothing else does

```text
latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;
```

Against the whole line the value out of scale is `5000000`: magnitude reads it 6.70 orders up
and names it the stream's one outlier, and a fixed bound, `\M{>3}`, takes `5000000` and `12000`
alike. Context reads each value against the values its own key held before. `size` has none, so
`5000000` has nothing to be read against, while `12000` is two orders above `latency`'s 100, 120 and 90,
and `"latency":k "=" \N{>+1:k}` matches `latency = 12000` alone. The same folds read a value
against its column, the window before it, or the tokens since the input last changed texture.

## Reading the axis

`trex context` reads every fold of an input. Its summary counts the tokens and the supertokens,
gives the record period with every live period, and the share of supertokens at which each lower
grain's nearest boundary keeps its role's usual offset:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex context --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
trex context (window fold): 81 bytes, 20 tokens, 5 supertokens
  record period: 4 tokens (live periods 4, 8)
  alignment: regime 0.00, shape 0.60, seam 0.20 of supertokens at their role's usual offset
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::context::{ContextConfig, Contexts};

let log = b"latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;";
let c = Contexts::read(&trex::lexer::lex(log), log, &ContextConfig::default());
assert_eq!((c.supers.units.len(), c.live_periods.as_slice()), (5, &[4, 8][..]));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> log = "latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;"
>>> c = trex.axes.context(log)
>>> c.fold, c.tokens, c.units, c.period, c.live_periods
('window', 20, 5, 4, [4, 8])
>>> round(c.alignment.shape, 2), round(c.alignment.seam, 2)
(0.6, 0.2)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $log = 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
PS> Measure-TrexContext $log | Select-Object Fold, Tokens, Units, Period, LivePeriods | Format-List

Fold        : Window
Tokens      : 20
Units       : 5
Period      : 4
LivePeriods : {4, 8}
```
{{< /tab >}}
{{< /tabs >}}

### Every token's context

`--field` lists every token with how many tokens its fold holds and each axis's reading over
them, and `--fold F` names the fold: `window`, the default, `unit`, `units`, `enclosing`,
`echo`, `regime`, `phase` or `key`, as `\N{>+1:F}` names it. `-Fold` and `fold=` name it in
PowerShell and Python, and `-Detail` and `detail=True` add the frames. Under the key fold each
`latency` reads the values `latency` was bound to before it, three at the last, whose magnitude
averages 2.01 against the 4.08 of `12000`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex context --fold key --field --limit 5 --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
trex context (key fold): 81 bytes, 20 tokens, 5 supertokens
  record period: 4 tokens (live periods 4, 8)
  alignment: regime 0.00, shape 0.60, seam 0.20 of supertokens at their role's usual offset
  per-token context, the values each one's key was bound to before:
    @     0  latency        over 0
    @     8  =              over 0
    @    10  100            over 0
    @    14  ;              over 0
    @    16  latency        over 1
        magnitude: mean 2.00, spread 0.00, max 2.00
        stress: mean 0.00, max 0
        spectral: entropy 0.56, period 0, strength 0.00
        echo: novel 1, echoed 0
        observation: causal 0.64, anticausal 0.83, centered 0.75, disagreement 0.18, disagreement max 0.18, contested 0
        seam: forward 0.92, backward 0.00, strength 1.37, strength max 1.37, cuts 1
        flow: slope -0.20, rising 0, falling 1, steady 0, momentum 4, reversals 0
    ... (+15 more tokens; raise --limit)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::context::{ContextConfig, Contexts, Fold};

let log = b"latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;";
let toks = trex::lexer::lex(log);
let c = Contexts::read(&toks, log, &ContextConfig::default());
let last = toks.iter().rposition(|t| &log[t.span()] == b"latency").expect("a latency");
let held = c.at(Fold::Key, last);
assert_eq!(held.magnitude.count, 3);
assert!((held.magnitude.mean() - 2.01).abs() < 0.01);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, f.offset, f.over, round(f.magnitude.mean, 2)) for f in trex.axes.context(log, fold="key", detail=True).frames if f.over]
[('latency', 16, 1, 2.0), ('latency', 49, 2, 2.04), ('latency', 64, 3, 2.01)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexContext $log -Fold Key -Detail).Frames | Where-Object Over | Format-Table Offset, Text, Over, @{ Name = 'Mean'; Expression = { $_.Magnitude.Mean } }

Offset Text    Over Mean
------ ----    ---- ----
    16 latency    1 2.00
    49 latency    2 2.04
    64 latency    3 2.01
```
{{< /tab >}}
{{< /tabs >}}

A fold that holds no token reads `over 0`, and a relative predicate that reads it there matches
nothing.

### The record period

`--period` lists each token's column of the record period, and the report's frames carry it as
`phase`. Rows of six tokens repeat at 6, and at 12 as well:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex context --period --limit 8 --text 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;'
trex context (window fold): 53 bytes, 24 tokens, 4 supertokens
  record period: 6 tokens (live periods 6, 12)
  alignment: regime 0.00, shape 0.25, seam 1.00 of supertokens at their role's usual offset
  per-token column of the record period:
    @     0  a                0
    @     2  ,                1
    @     4  10               2
    @     7  ,                3
    @     9  x                4
    @    11  ;                5
    @    13  b                0
    @    15  ,                1
    ... (+16 more tokens; raise --limit)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::context::{ContextConfig, Contexts};

let rows = b"a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;";
let toks = trex::lexer::lex(rows);
let c = Contexts::read(&toks, rows, &ContextConfig::default());
let columns: Vec<u16> = (0..toks.len()).filter_map(|i| c.phase_of(i)).take(8).collect();
assert_eq!(columns, [0, 1, 2, 3, 4, 5, 0, 1]);
assert_eq!(c.live_periods, [6, 12]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> r = trex.axes.context("a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;", detail=True)
>>> r.period, r.live_periods, [f.phase for f in r.frames[:8]]
(6, [6, 12], [0, 1, 2, 3, 4, 5, 0, 1])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $rows = 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;'
PS> (Measure-TrexContext $rows -Detail).Frames | Select-Object -First 8 | ForEach-Object Phase
0
1
2
3
4
5
0
1
```
{{< /tab >}}
{{< /tabs >}}

## Choosing a threshold

A relative predicate's delta is in orders of magnitude, or in standard deviations of the
context with an `s`. Read the context before choosing: against the window before it `5000000`
and `12000` are both more than an order above, and only `5000000` more than two of the
window's standard deviations; in the rows `9000` reads 3.95 against its column's mean of 1.04,
2.91 orders above, so a delta of 3 takes nothing:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N{>+1}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[39..46] "5000000"
[74..79] "12000"

$ trex scan '\N{>+2s}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[39..46] "5000000"

$ trex scan '\N{>+3:phase}' --text 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;'
no match
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str, text: &str| -> Vec<String> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
let log = "latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;";
assert_eq!(found(r"\N{>+1}", log), ["5000000", "12000"]);
assert_eq!(found(r"\N{>+2s}", log), ["5000000"]);
assert!(found(r"\N{>+3:phase}", "a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;").is_empty());
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"\N{>+1}").scan(log)], [m.text for m in trex.Pattern(r"\N{>+2s}").scan(log)]
(['5000000', '12000'], ['5000000'])
>>> [m.text for m in trex.Pattern(r"\N{>+3:phase}").scan("a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;")]
[]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N{>+1}' -InputObject $log -Raw
5000000
12000

PS> Select-TrexMatch '\N{>+2s}' -InputObject $log -Raw
5000000
```
{{< /tab >}}
{{< /tabs >}}

`--token-window N` sets how many significant tokens the window folds, 32 by default, and
`--unit-window N` how many supertokens the units fold, 8; `-TokenWindow` and `-UnitWindow`, and
`token_window=` and `unit_window=`, set them in PowerShell and Python. A shorter window reads a
value against its nearer neighbors only.

## Declarations

A declared shape or kind changes the tokens the folds hold and the values a key is bound to.
`trex context` takes the declarations a scan takes, `--lib`, `--shape`, `--shape-after`,
`--kind`, `--let` and `--declare`, as `-Library` and `lib=` do in PowerShell and Python
([pattern files](../../pattern-files/)).

## The rolling window

A reading has an identity and an associative combine, so a span's reading is the fold of its
tokens' readings. The window slides one token at a time and keeps two stacks, suffix folds of
its older half and a running fold of its newer half, so each step is one combine whatever the
window's length. It folds at the token rung and, one rung up, over the [supertokens](../../../explanation/architecture/#supertokens), each
unit's reading being the fold of its tokens. The fold carries the magnitude, stress, spectral,
echo, observation, seam and flow readings.

| Reading | Folds |
|---|---|
| `ContextField::at_token[i]` | the window of significant tokens ending at token `i` |
| `ContextField::of_unit[u]` | unit `u`'s own tokens |
| `ContextField::at_unit[u]` | the window of units ending at unit `u` |

The window is 32 significant tokens, the [observation](../observation/) reader's window, and 8
units (`ContextConfig`).

## The related context

Five more folds admit tokens by relation rather than by distance. Each is a running fold, exact
and unbounded in range, one combine when a token joins it.

| Fold | Over |
|---|---|
| `enclosing` | the heads of the brackets enclosing the token, outermost first |
| `echoing` | every earlier occurrence of the token's key |
| `regime` | the significant tokens since the byte grain's last spectral change-point |
| `phase` | every earlier significant token at the token's phase of the record period |
| `value_history` | the values bound by `=` or `:` to earlier occurrences of the token's key |

Each leaves the token itself out, so it is the context before the token.

The record period is the smallest lag, up to 32 tokens, at which the silhouettes of the
significant tokens best repeat: the share of positions whose silhouette equals the one a lag
later, at or above the floor a byte period must clear. It reads silhouettes rather than kinds; in
kinds the record `word , number , word ;` alternates content and punctuation and repeats at 2.

```rust
let text = b"a , 1 , x ; b , 2 , y ; c , 3 , z ;";
let toks = trex::lexer::lex(text);
assert_eq!(trex::context::record_period(&toks, text), Some(6));

let field = trex::context::analyze(text);
assert_eq!(field.at_token.len(), toks.len());
let related = trex::context::relate_bytes(text);
assert_eq!(related.period, Some(6));
```

| Item | Signature |
|---|---|
| `analyze` / `analyze_with` | `fn(bytes: &[u8]) -> ContextField`, the second with a `&ContextConfig` |
| `fold_windows` / `fold_windows_parallel` | `fn(toks: &[Token], ctx: &AxisCtx, supers: &SuperContext, cfg: &ContextConfig) -> ContextField` |
| `relate` / `relate_bytes` | `fn relate(toks, ctx, regime_cuts: &[usize], echo: &EchoField, period: Option<u16>) -> RelationContext` |
| `record_period` | `fn record_period(toks: &[Token], bytes: &[u8]) -> Option<u16>` |
| `agreement` | `fn agreement(units, toks, regime_cuts, shape_cuts, seam_cuts) -> Vec<Agreement>` |
| `Window<P>` | `new(capacity)`, `push(value)`, `fold()` over any `AxisProfile` |

## In a pattern

| Form | Baseline |
|---|---|
| `\N{>+1}` `\N{>+2s}` `\N{<-1}` | the window before the token |
| `\N{>+1:phase}` | the token's column |
| `\N{>+1:regime}` | since the last regime change |
| `\N{>+1:echo}` | the token's earlier occurrences |
| `\N{>+1:enclosing}` | the enclosing heads |
| `"key":k "=" \N{>+1:k}` | the values bound to earlier occurrences of the key `k` holds |
| `@phase:k` | column `k` of the record period |
| `@phase:k/p` `@phase:k#n` | column `k` of the live `p`-token period, or of the `n`-th strongest live period |

The delta is in orders of magnitude, or in standard deviations of the context with an `s`
suffix. An empty context is no baseline and matches nothing; a sigma form also needs a context
of more than one value with a spread.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '"latency":k "=" \N{>+1:k}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[64..79] "latency = 12000"  captures: k="latency"

$ trex scan '\N{>+2:phase}' --text 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;'
[30..34] "9000"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;";
let pat = trex::parse(r#""latency":k "=" \N{>+1:k}"#).expect("valid pattern");
let found: Vec<(&[u8], &[u8])> = trex::captures(&pat, log, &trex::scan(&pat, log))
    .iter()
    .map(|m| (&log[m.start..m.end], m.group("k", log).expect("k is bound")))
    .collect();
assert_eq!(found, [(&b"latency = 12000"[..], &b"latency"[..])]);

let rows = "a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;";
let pat = trex::parse(r"\N{>+2:phase}").expect("valid pattern");
let found: Vec<&str> = trex::scan(&pat, rows.as_bytes()).iter().map(|s| &rows[s.range()]).collect();
assert_eq!(found, ["9000"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [(m.text, m.captures) for m in trex.Pattern('"latency":k "=" \\N{>+1:k}').scan("latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;")]
[('latency = 12000', {'k': 'latency'})]
>>> [m.text for m in trex.Pattern(r"\N{>+2:phase}").scan("a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;")]
['9000']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '"latency":k "=" \N{>+1:k}' -InputObject 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;' | Select-Object Text, Groups

Text            Groups
----            ------
latency = 12000 {k=latency}

PS> Select-TrexMatch '\N{>+2:phase}' -InputObject 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;' -Raw
9000
```
{{< /tab >}}
{{< /tabs >}}

## Where the grains agree

The byte grain places spectral change-points, the token grain shape change-points and seam cuts,
the unit grain the unit starts. `ContextField::agreement` records, per unit, where each lower
grain's nearest boundary is relative to the unit's start, in significant tokens, and
`alignment()` how consistently that offset repeats for units of one role. On the code corpus
[`benches/context_window.rs`](https://github.com/Variably-Constant/trex/blob/main/benches/context_window.rs)
reads, the token-grain seam cuts one token before a call, two before a key, one after a list or a
binding: 0.875 of units at their role's offset, against 0.372 on the same tokens shuffled.

`--agreement` lists every supertoken with its role and the three offsets, a dash where a grain
places no boundary near it, and `agreement` and `Agreement` hold the same in Python and
PowerShell. In the rows every seam cut is at a supertoken's start, so the seam's share is
1.00:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex context --agreement --text 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;'
trex context (window fold): 53 bytes, 24 tokens, 4 supertokens
  record period: 6 tokens (live periods 6, 12)
  alignment: regime 0.00, shape 0.25, seam 1.00 of supertokens at their role's usual offset
  per-supertoken (role, then the nearest spectral change point, shape change point and seam cut, in tokens from its start):
    @     0  a , 10 , x     list        -    0    0
    @    13  b , 12 , y     list        -   -2    0
    @    26  c , 9000 , z   list        -   -8    0
    @    41  d , 11 , w     list        -  -14    0
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::context::{ContextConfig, Contexts};

let rows = b"a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;";
let c = Contexts::read(&trex::lexer::lex(rows), rows, &ContextConfig::default());
assert_eq!(c.field.agreement.len(), 4);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(g.text, g.role, g.regime, g.shape, g.seam) for g in trex.axes.context("a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;", detail=True).agreement]
[('a , 10 , x', 'list', None, 0, 0), ('b , 12 , y', 'list', None, -2, 0), ('c , 9000 , z', 'list', None, -8, 0), ('d , 11 , w', 'list', None, -14, 0)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexContext $rows -Detail).Agreement | Format-Table Offset, Text, Role, Regime, Shape, Seam

Offset Text         Role Regime Shape Seam
------ ----         ---- ------ ----- ----
     0 a , 10 , x   list            0    0
    13 b , 12 , y   list           -2    0
    26 c , 9000 , z list           -8    0
    41 d , 11 , w   list          -14    0
```
{{< /tab >}}
{{< /tabs >}}
