---
title: Why tokens
linkTitle: Why tokens
weight: 10
---

# Why tokens

A regular expression's alphabet is bytes. A number is `[-+]?\d+(?:\.\d+)?`, whitespace is spelled
`\s*` wherever two things sit next to each other, and capture groups are numbered by counting open
parentheses, so inserting one renumbers every reference after it.

trex's alphabet is typed tokens. A span the lexer classes as a number or a quoted string is one
atom, `\N` or `\Q`. Whitespace between atoms is insignificant, so a pattern never spells it.
Captures carry names, so adding one renumbers nothing.

## What the lexer makes expressible

Bracket pairing. `(a(b)c)` needs the last `)` matched to the first `(`, which is counting, and a
regular language cannot count. The lexer pairs `()`, `[]` and `{}` as it runs, so a balanced
group is the one atom `\B(...)` with the nesting already resolved.

Long-distance equality. `:name` writes the matched token into a register and `=name` later
requires a token equal to it. A backtracking regex spells this with a backreference, the
construct behind catastrophic backtracking; trex answers it with a register lookup and never
backtracks.

Whole characters. The lexer decodes characters rather than bytes, so `中文分词` is one word
token that `\W` matches whole, and an em-dash or a curly quote is one punctuation token:

```console
$ trex scan '\W' --text '中文分词 a—b “q”'
[0..12] "中文分词"
[13..14] "a"
[17..18] "b"
[22..23] "q"

$ trex scan '\P' --text '中文分词 a—b “q”'
[14..17] "—"
[19..22] "“"
[23..26] "”"
```

Input in UTF-32, UTF-16 or UTF-8 with a byte-order mark, or in UTF-16 without one, is transcoded
to UTF-8 before it is lexed.

## Token-mode habits

Whitespace between atoms is never written. The byte classes `\d`, `\w` and a
`` `byte-pattern` `` read inside one token. A literal is a quoted whole token, so `"("`, `")"`
and `"."` match themselves with no escaping, while an unquoted `.` is any one token. A reference
is by name, so an edit above it leaves it pointing at the same register.

## What it costs

trex lexes before it matches, so on a plain literal search the regex crate is faster. What the
lexing buys is what a byte regex cannot state: binding, balance, guards and the
[property axes](../../reference/axes/). The [engine](../the-engine/) says how a match runs with no
backtracking.
