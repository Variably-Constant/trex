---
title: Find values that break their history
linkTitle: Find outliers
weight: 45
---

# Find values that break their history

You have a log, a table, or a stream of key/value lines, and you want the values that are out
of scale - not above some number you would have to know in advance, but above what the
stream itself has been doing. A regex cannot say this: it has no arithmetic and no memory of
values. trex reads the threshold from the stream at every token.

## Against the neighbourhood

`\N{>+1}` matches a number an order of magnitude above the mean of the window of tokens
before it; `\N{>+2s}` two of that window's standard deviations above it.

```console
$ trex scan '\N{>+1}' --text 'v 1 2 3 1 2 3 5000 2 3'
[14..18] "5000"
```

The window knows neighbourhoods, not keys. In a log where sizes run near a million and
latencies near a hundred, every size is large against a mixed neighbourhood:

```console
$ trex scan '\N{>+1}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[39..46] "5000000"
[74..79] "12000"
```

## Against a key's own history

Bind the key to a register and compare against the values bound to it before. `\N{>+1:k}`
reads the values that followed `=` or `:` after every earlier occurrence of what `k` holds,
so the sizes are no longer in the way:

```console
$ trex scan '"latency":k "=" \N{>+1:k}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[64..79] "latency = 12000"  captures: k="latency"
```

The first occurrence of a key has no history, so it never matches: the pattern learns the
baseline from the stream and flags departures from it.

## Against a column

In a periodic record - a CSV row, a fixed-shape line - `@phase:k` is column `k`, counted in
significant tokens, and `\N{>+2:phase}` is a value two orders above the earlier values in its
own column. No delimiter is named; the period is read from the record's own shape.

```console
$ trex scan '@phase:2 \N' --text 'a , 1 , x ; b , 2 , y ; c , 3 , z ; d , 4 , w ;'
[4..5] "1"
[16..17] "2"
[28..29] "3"
[40..41] "4"

$ trex scan '\N{>+2:phase}' --text 'a , 10 , x ; b , 12 , y ; c , 9000 , z ; d , 11 , w ;'
[30..34] "9000"
```

## Choosing the delta

The delta is in orders of magnitude, so `+1` is ten times the baseline's typical value and
`+2` a hundred times; a sigma form (`+2s`) is in the context's own spread and needs a context
with more than one value in it. A context with nothing in it - the first token, the first
occurrence of a key, a stream with no period - is no baseline, and matches nothing.

The full set of contexts, including the tokens since the last regime change and the heads of
the brackets enclosing a token, is in the [context reference](../../reference/axes/context/).
