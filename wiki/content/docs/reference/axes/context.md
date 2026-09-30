---
title: Context
linkTitle: Context
weight: 100
---

# Context

Every other axis reads one token, or one window around it, and reports a number. The context
module keeps, at every token, what that token is read *against*: the fold of every axis over
the window before it, over its column, over the tokens since the last regime change, over the
heads of the brackets enclosing it, over its earlier occurrences, and over the values bound to
a key. A relative predicate in a pattern (`\N{>+1}`, `\N{>+1:k}`) compares a token's reading
to one of these instead of to a number written into the pattern.

## The rolling window

Each axis reading is a monoid - it has an identity and an associative combine - so a span's
reading is the fold of its tokens' readings. A window that slides one token at a time is the
same fold with the oldest value leaving; a monoid has no inverse, so the window keeps two
stacks, suffix folds of the older half and a running fold of the newer, and its fold is one
combine whatever its extent. The window folds at the token rung and, one rung up, over the
units (supertokens), each unit's own reading being the fold of its tokens.

| Reading | Folds |
|---|---|
| `ContextField::at_token[i]` | the window of significant tokens ending at token `i` |
| `ContextField::of_unit[u]` | unit `u`'s own tokens |
| `ContextField::at_unit[u]` | the window of units ending at unit `u` |

The default window is 32 significant tokens (the observation reader's window) and 8 units.

## The related context

A window admits by distance. Four more folds admit by relation, and because a reading is a
monoid none of them keeps members or evicts any: each is a running fold, exact and unbounded
in range, one combine when a token joins it.

| Fold | Over |
|---|---|
| `enclosing` | the heads of the brackets enclosing the token, outermost first |
| `echoing` | every earlier occurrence of the token's key |
| `regime` | the significant tokens since the byte grain's last spectral change-point |
| `phase` | every earlier significant token at the token's phase of the record period |
| `value_history` | the values bound by `=` or `:` to earlier occurrences of the token's key |

Each leaves the token itself out, so it is the context *before* the token.

The record period is the smallest lag at which the silhouettes of the significant tokens best
repeat - the share of positions whose silhouette equals the one a lag later, at or above the
same floor a byte period must clear - searched up to 32 tokens. Silhouettes rather than kinds,
because in kinds a record `word , number , word ;` is an alternation of content and
punctuation and would read as two.

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
suffix. An empty context is no baseline and matches nothing; a sigma form also needs a
context of more than one value with a spread.

```console
$ trex scan '"latency":k "=" \N{>+1:k}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[64..79] "latency = 12000"  captures: k="latency"

$ trex scan '\N{>+2:phase}' --text 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;'
[30..34] "9000"
```

## Where the grains agree

The byte grain places spectral change-points, the token grain shape change-points and seam
cuts, the unit grain the unit starts. `ContextField::agreement` records, per unit, where each
lower grain's nearest boundary sits relative to the unit's start, in significant tokens, and
`alignment()` how consistently that offset repeats for units of one role. On the code corpus
`benches/context_window` reads, the token-grain seam cuts one token before a call, two before
a key, one after a list or a binding: 0.875 of units at their role's offset, against 0.372 on
the same tokens shuffled.

## Library

```rust
let field = trex::context::analyze(b"latency = 100 ; latency = 12000 ;");
let related = trex::context::relate_bytes(b"latency = 100 ; latency = 12000 ;");
let toks = trex::lexer::lex(b"a , 1 , x ; b , 2 , y ; c , 3 , z ;");
assert_eq!(trex::context::record_period(&toks, b"a , 1 , x ; b , 2 , y ; c , 3 , z ;"), Some(6));
```

| Item | Signature |
|---|---|
| `analyze` | `fn analyze(bytes: &[u8]) -> ContextField` |
| `fold_windows` / `fold_windows_parallel` | `fn(toks: &[Token], ctx: &AxisCtx, supers: &SuperContext, cfg: &ContextConfig) -> ContextField` |
| `relate` / `relate_bytes` | `fn relate(toks, ctx, regime_cuts: &[usize], echo: &EchoField, period: Option<u16>) -> RelationContext` |
| `record_period` | `fn record_period(toks: &[Token], bytes: &[u8]) -> Option<u16>` |
| `agreement` | `fn agreement(units, toks, regime_cuts, shape_cuts, seam_cuts) -> Vec<Agreement>` |
| `Window<P>` | `new(capacity)`, `push(value)`, `fold()` over any `AxisProfile` |
