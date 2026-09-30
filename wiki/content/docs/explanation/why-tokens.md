---
title: Why tokens
linkTitle: Why tokens
weight: 10
---

# Why tokens

A regular expression works one byte at a time. That single fact is behind most of the
friction. A number is `[-+]?\d+(?:\.\d+)?`. Whitespace has to be spelled out with `\s*`
wherever two things sit next to each other. Capture groups are numbered by counting open
parentheses, so inserting one three lines up renumbers every reference below it.

Move the recognition into a lexer and the friction goes away. Once a span has been classed as
a whole number or a whole quoted string it is one atom, `\N` or `\Q`, not a sub-expression.
Whitespace between atoms means nothing, so it never appears. Captures carry names instead of
positions, so a new group disturbs nothing.

## What the lexer unlocks

Pairing brackets is something a regex cannot do at all. `(a(b)c)` needs to know that the last
`)` closes the first `(`, and that is counting; a regular language cannot count. The lexer
pairs `()`, `[]`, and `{}` as it runs, so a balanced group is a single glyph, `\B(...)`, with
the nesting already resolved.

Long-distance equality is the other one. `:name` writes the matched token into a register and
`=name` later demands a token equal to it. A regex spells this with a backreference, and the
backreference is what opens the door to catastrophic backtracking. Here it is a lookup in a
register set, which has nothing to backtrack.

Unicode gets the same treatment. Byte-level tokenizers are notorious for shattering a CJK
character into fragments, and a byte regex sees the fragments too. The lexer decodes whole
chars instead: `中文分词` is one Word token and `\W` matches it whole, an em-dash or a curly
quote is one Punct token, and a file in UTF-16 or UTF-32 transcodes on the way in, so a
Windows-written log needs no conversion step.

## Token-mode semantics

A few habits change once the alphabet is tokens. `\s*` litter never appears, because
whitespace between atoms is already insignificant; drop to the byte grain (`\d`, `\w`, a
`` `byte-regex` ``) when you need detail inside a token. Literals are whole tokens, so `.`,
`(`, and `)` match as themselves with no escaping. And a reference is by name, so it survives
any edit above it.

## What it costs

trex tokenizes before it matches, so on a plain literal search a finely-tuned byte regex will
out-throughput it. That is fine; throughput is not the trade. What the tokenizing pass buys is
the work a regex rejects or cannot state - binding, balance, guards, the property axes -
alongside shorter patterns and no backtracking cliff. The [engine](../the-engine/) is where
that last claim is made good.
