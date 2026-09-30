---
weight: 50
---

# Beyond regex

The earlier chapters covered what a regex does, spelled over tokens. This one covers the rest
of the regex surface - the constructs a regex user reaches for once the basics run out - and
then the constructs that have no regex counterpart at all. Every example is a real command
with its real output.

## Where the match stands

`^` and `$` hold at the first and last significant token of a line. `\A` and `\z` are the
same for the whole input:

```console
$ trex scan '\A \W' --text 'alpha beta gamma'
[0..5] "alpha"

$ trex scan '\W \z' --text 'alpha beta gamma'
[6..10] "beta"
```

(`\W \z` matches the word *before* the last token: `\z` holds at `gamma`, the word atom has to
sit in front of it.)

`\G` turns a scan into a tokenizer: each match must begin exactly where the previous one
ended, so the run stops at the first gap instead of skipping it.

```console
$ trex scan '\G \N' --text '1 2 x 3 4'
[0..1] "1"
[2..3] "2"

$ trex scan '\N' --text '1 2 x 3 4'
[0..1] "1"
[2..3] "2"
[6..7] "3"
[8..9] "4"
```

`\K` requires everything to its left and reports only what follows - a lookbehind that needs
no fixed width:

```console
$ trex scan '"key" ":" \W' --text 'key: value key: other'
[0..10] "key: value"
[11..21] "key: other"

$ trex scan '"key" ":" \K \W' --text 'key: value key: other'
[5..10] "value"
[16..21] "other"
```

## Looking around

`~(P)` asserts that `P` matches at the current position without consuming it; `!~(P)` asserts
it does not. `~<(P)` and `!~<(P)` look backward, and need a `P` of bounded length.

```console
$ trex scan '\W ~(\N)' --text 'x 1 y z 2'
[0..1] "x"
[6..7] "z"

$ trex scan '\W !~(\N)' --text 'x 1 y z 2'
[4..5] "y"

$ trex scan '~<(\N) \W' --text 'x 1 y z 2'
[4..5] "y"
```

The literal guard `~"lit"` you met earlier is the cheap case of this: a presence question over
the forward window that a prefilter answers without a positional scan.

## Three kinds of choice

`|` is what a regex means: the first branch that can match wins. `||` prefers the longest
overall match. `|>` commits to the first branch that matches and never reconsiders, which is
what a PEG means by ordered choice.

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

## Atomic groups and possessive quantifiers

`(?>P)` keeps only the match `P` prefers; nothing after it can make `P` give a token back.
`P*+`, `P++` and `P?+` are the same thing spelled on a quantifier.

```console
$ trex scan '\N+ \N' --text '1 2 3'
[0..5] "1 2 3"

$ trex scan '(?>\N+) \N' --text '1 2 3'
no match
```

## Matching up to a symmetry

A regex has one case-folding flag. trex has a ladder of symmetries, and a scope that applies
one to every literal inside it: `(?orbit:case ...)` is `(?i)`, `(?orbit:shape ...)` and
`(?orbit:notation ...)` fold word shape and mathematical notation the same way.

```console
$ trex scan '"cat"' --text 'Cat CAT dog cat'
[12..15] "cat"

$ trex scan '(?orbit:case "cat")' --text 'Cat CAT dog cat'
[0..3] "Cat"
[4..7] "CAT"
[12..15] "cat"
```

## Token classes

A class is a set of single-token atoms: union `[\N \W]`, complement `[^\N]`, intersection
`[\W && \h]`, subtraction `[\W -- "if"]`.

```console
$ trex scan '[\W && \h]' --text 'deadbeef xyz cafe'
[0..8] "deadbeef"
[13..17] "cafe"

$ trex scan '[\W -- "if"]' --text 'if x if y'
[3..4] "x"
[8..9] "y"
```

## Inside a token

A byte-pattern between backticks is a regex over the characters of one token. `\p{L}` matches
any letter, in any script:

```console
$ trex scan '`\p{L}+`' --text 'héllo wörld 123'
[0..6] "héllo"
[7..13] "wörld"
```

## Shapes you declare

The lexer knows thirty typed kinds. When your data has one it does not - a ticket id, a part
number - declare it: a name and a bounded byte-pattern, tried before the built-in recognizers,
and matched by name.

```console
$ trex scan '\{ticket}' --shape 'ticket = `[A-Z]{2,4}-\d{1,4}`' --text 'see AB-12 and XYZ-9 now'
[4..9] "AB-12"
[14..19] "XYZ-9"
```

## Anchors on constructs

Above tokens sit constructs - a call, a binding, a key/value entry, a list - read from
punctuation and bracket shape, never from a language's keywords. `@super` holds at the first
token of a construct and `@super:role` where the construct containing the token has that role:

```console
$ trex scan '@super:call \W' --text 'x = 1 ; foo(a, b) ; y : 2'
[8..11] "foo"

$ trex scan '@super:assign \N' --text 'x = 1 ; foo(a, b) ; y : 2'
[4..5] "1"
```

A number is a number wherever it sits, but a number inside a binding is a value and a number
inside a call is an argument; this is what tells them apart with no grammar for the language.

The predictive anchors take a grain too. `@seam` cuts where the bytes stop predicting
themselves; `@seam:token` where the sequence of token kinds does, which is a break in the
shape of a statement rather than in a word:

```console
$ trex scan '@seam:token \W' --text 'let x = 1 ; let y = 2 ; print x ; print y ;'
[0..3] "let"
[4..5] "x"
[34..39] "print"
```

## Thresholds read from the stream

Every threshold so far was a number in the pattern: `\N{>6}`. A regex has nothing else. trex
can read the threshold from the stream, at every token, from a context it keeps as it goes:
the window of tokens before the token, its column in a periodic record, or the earlier values
bound to a key.

`\N{>+1}` is a number an order of magnitude above the mean of the window before it:

```console
$ trex scan '\N{>+1}' --text 'v 1 2 3 1 2 3 5000 2 3'
[14..18] "5000"
```

`@phase:k` is column `k` of a periodic record, with no delimiter named - the period is read
from the record itself - and `\N{>+2:phase}` is a value two orders above its own column:

```console
$ trex scan '@phase:2 \N' --text 'a , 1 , x ; b , 2 , y ; c , 3 , z ; d , 4 , w ;'
[4..5] "1"
[16..17] "2"
[28..29] "3"
[40..41] "4"

$ trex scan '\N{>+2:phase}' --text 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;'
[30..34] "9000"
```

And with a key bound to a register, `\N{>+1:k}` is a value an order above every value bound
to that key before. The sizes below are large, but they are bound to another key, so only the
latency that breaks its own history matches:

```console
$ trex scan '"latency":k "=" \N{>+1:k}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[64..79] "latency = 12000"  captures: k="latency"
```

The [how-to on outliers](../../how-to/find-outliers/) works through these on a log, and the
[context reference](../../reference/axes/context/) says exactly what each context holds.

## You are done

You have now seen the whole pattern language. The [pattern syntax reference](../../reference/pattern-syntax/)
is the complete table, and the [explanation](../../explanation/) says why none of it ever
backtracks.
