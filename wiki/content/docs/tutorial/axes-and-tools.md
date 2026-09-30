---
weight: 40
---

# Axes and tools

You can match, bind, and balance. This chapter covers the two remaining halves of trex: the
**property axes** that read the numeric fields a token stream carries, and the **tools**
beyond `scan` - rewrite, grammar, and prefilter.

## The property axes

A token has more than an identity. It has a scale, sits at a nesting depth, carries a temporal
texture, belongs to a symmetry class. Each is an **axis**: a field over the stream, available
both as a standalone command and as a pattern predicate.

The **magnitude** axis reads the order of magnitude of each token's value - the one thing kind,
shape, and texture all discard:

```console
$ trex magnitude --text 'retries = 3 ; max_bytes = 5000000000'
trex magnitude: 36 bytes, 7 tokens, total energy 112.2, 3 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
```

Query it from inside a pattern with `\M{...}`. "A token at least six orders of magnitude big":

```console
$ trex scan '\M{>6}' --text 'retries = 3 ; max_bytes = 5000000000'
[26..36] "5000000000"
```

The **orbit** axis folds symmetry-equivalent tokens to one representative. Under the word-shape
group, `cat`, `dog`, and `bat` are all `CVC`:

```console
$ trex orbit --collapse --group shape --text 'cat dog bat sat mat the fox'
trex orbit --collapse (group shape): 7 raw forms -> 2 orbits (3.5x reduction)
    "CCV"
    "CVC"  <-  ["bat", "cat", "dog", "fox", "mat", "sat"]
```

The **seam** axis segments a stream where the past stops predicting the future, with no
delimiter knowledge:

```console
$ trex seam --text 'the cat sat'
trex seam: 11 bytes, order 3, 5 segments (bidirectional branching entropy)
  segments:
    [     0..3     ] "the"
    [     3..4     ] " "
    [     4..7     ] "cat"
    [     7..8     ] " "
    [     8..11    ] "sat"
```

There are nine axes in all - magnitude, spectral, shape, orbit, seam, stress, flow,
observation, and echo, which reads whether a token's content returns elsewhere in the
document. Each has a [reference page](../../reference/axes/) with its full field, flags, and
the physics it mirrors.

## Rewrite

`rewrite` replaces each match with a rendered template. `${name}` renders a capture, and
`${name:acc}` transforms or slices it - `upper`, `lower`, `trim`, or a typed sub-field like
`${ip:octet1-2}`:

```console
$ trex rewrite '\E:e' '[redacted]' --text 'mail bob@x.com now'
mail [redacted] now
```

## Grammar

`grammar` parses the token stream against named rules. Left-recursive rules encode operator
precedence with no annotations:

```console
$ trex grammar --grammar-text 'expr := number "+" number' --text '2 + 3'
(expr 2 + 3)
```

## Prefilter

`prefilter` answers "might this literal occur?" with an approximate-membership filter, so an
absent literal is rejected with no corpus scan:

```console
$ trex prefilter --text 'the quick brown fox ERROR here' --literal ERROR --literal MISSING
bloom: "ERROR" -> might occur; present (confirmed)
bloom: "MISSING" -> ABSENT (rejected with no corpus scan)
```

## You are done

You can now read and write trex patterns, query every axis, and reach for the right command.
From here:

- the [How-To guides](../../how-to/) solve specific tasks;
- the [Reference](../../reference/) is the complete spec;
- the [Explanation](../../explanation/) covers why the engine never backtracks and how the
  dual-grain architecture works.
