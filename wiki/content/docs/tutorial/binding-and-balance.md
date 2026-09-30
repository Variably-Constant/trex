---
weight: 30
---

# Binding and balance

This is where trex leaves a regex behind. A regex fakes a back-reference and pays for it with
backtracking; it cannot match balanced brackets at all. trex does both, in one pass. The two
sections below take them in turn.

## Binding a register

`A:name` binds the token `A` matched to a register called `name`. `=name` later matches a
token only when it equals the bound value. So "a word repeated" is:

```console
$ trex scan '\W:x =x' --text 'the the cat'
[0..7] "the the"  captures: x="the"
```

`\W:x` bound `x = "the"`; `=x` then required the next word to equal `the`, which it did. The
`captures:` field on the match line shows the binding. In a regex this is a backreference
(`\b(\w+)\s+\1\b`), and on adversarial input it can trigger catastrophic backtracking. trex
evaluates it as a register check with no backtracking at all.

## Fuzzy back-reference: equal up to a symmetry

`=shape name` (and `=case name`, `=notation name`) match a token in the same **orbit** as the
bound value rather than byte-for-byte. `cat` and `bat` share the word-shape `CVC`, so:

```console
$ trex scan '\W:w =shape w' --text 'cat bat xyz'
[0..7] "cat bat"  captures: w="cat"
```

`cat` bound `w`; `bat` matched because it has the same shape; `xyz` (shape `CCC`) would not.
The [orbit axis](../../reference/axes/orbit/) defines the symmetry groups.

## Balanced bracket groups

The lexer pairs `()`, `[]`, and `{}`, so a balanced, arbitrarily-nested group is a single
primitive `\B` - not the impossible recursion it is for a regular expression:

```console
$ trex scan '\B' --text 'f(a, b) g(c)'
[1..7] "(a, b)"
[9..12] "(c)"
```

`\B(P)`, `\B[P]`, `\B{P}` match a specific bracket kind whose interior matches `P`. Combined
with binding, a matched open/close tag is one line:

```console
$ trex scan '<\W:t>.*</=t>' --text '<div>hi</div>'
[0..13] "<div>hi</div>"  captures: t="div"
```

The close tag must equal the open tag's bound word. Change the close tag and it stops
matching:

```console
$ trex scan '<\W:t>.*</=t>' --text '<div>hi</span>'
no match
```

## Content-addressed lookahead

`~"lit"` is a zero-width assertion: it holds when the literal occurs somewhere in the forward
window, answered by a presence prefilter rather than a positional scan. `!~"lit"` is its
negation. Words that have `END` somewhere ahead:

```console
$ trex scan '\W ~"END"' --text 'begin middle END'
[0..5] "begin"
[6..12] "middle"
```

`begin` and `middle` match because `END` lies ahead of each; `END` itself has nothing after
it, so it does not.

## Where next

You can now express the patterns that motivated trex. The last chapter,
[axes and tools](../axes-and-tools/), covers the property axes (reading scale, symmetry, and
predictive segmentation) and the rewrite, grammar, and prefilter tools.
