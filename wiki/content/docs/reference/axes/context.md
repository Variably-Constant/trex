---
title: Context
linkTitle: Context
weight: 100
---

# Context

At every token, the other axes' readings folded over what the token is read against: the window
before it, its column, the tokens since the last regime change, the heads of the brackets
enclosing it, its earlier occurrences, and the values bound to its key. A relative predicate in a
pattern (`\N{>+1}`, `\N{>+1:k}`) compares a token's reading with one of these.

Source: [`src/context.rs`](https://github.com/Variably-Constant/trex/blob/main/src/context.rs).

## The rolling window

A reading has an identity and an associative combine, so a span's reading is the fold of its
tokens' readings. The window slides one token at a time and keeps two stacks, suffix folds of
its older half and a running fold of its newer half, so each step is one combine whatever the
window's length. It folds at the token rung and, one rung up, over the units (supertokens), each
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

Four more folds admit tokens by relation rather than by distance. Each is a running fold, exact
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
grain's nearest boundary sits relative to the unit's start, in significant tokens, and
`alignment()` how consistently that offset repeats for units of one role. On the code corpus
[`benches/context_window.rs`](https://github.com/Variably-Constant/trex/blob/main/benches/context_window.rs)
reads, the token-grain seam cuts one token before a call, two before a key, one after a list or a
binding: 0.875 of units at their role's offset, against 0.372 on the same tokens shuffled.
