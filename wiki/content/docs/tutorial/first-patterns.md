---
weight: 20
---

# Your first patterns

You can already match single atoms and sequences. This chapter covers the rest of ordinary
pattern matching: the full atom set, alternation, and quantifiers. None of this needs the
advanced engine yet - it is the part that overlaps with a regex, only terser.

## The token atoms

Each atom matches exactly one token of a given kind. The common ones:

| Atom | Matches | Atom | Matches |
|---|---|---|---|
| `\N` | a number | `\U` | a URL |
| `\W` | a word / identifier | `\T` | a timestamp or date |
| `\Q` | a quoted string | `\P` | a punctuation token |
| `\I` | an IP address | `.` | any one token |
| `\E` | an email | `"lit"` | a literal token equal to `lit` |

There are more typed atoms (versions, UUIDs, money, durations, ...); the
[pattern reference](../../reference/pattern-syntax/) has the full table. The lexer recognises
each kind, so `\E` matches a whole email as one atom:

```console
$ trex scan '\E' --text 'ping bob@x.com please'
[5..14] "bob@x.com"
```

## Alternation: this or that

`A|B` matches whichever alternative comes first. Numbers or emails:

```console
$ trex scan '\N|\E' --text 'id 7 mail a@b.com'
[3..4] "7"
[10..17] "a@b.com"
```

## Quantifiers: how many

Suffix an atom with `*` (zero or more), `+` (one or more), `?` (optional), or `{m,n}` (between
m and n). A run of numbers:

```console
$ trex scan '\N+' --text 'coords 1 2 3 stop'
[7..12] "1 2 3"
```

The `+` is greedy: it took all three numbers as one match. A counted range splits a longer run
into the largest allowed chunks:

```console
$ trex scan '\N{2,3}' --text '1 2 3 4 5'
[0..5] "1 2 3"
[6..9] "4 5"
```

## Lenses: named structural shapes

A **lens** is a shorthand for a convergent structural shape that holds across languages.
`@call` is an identifier followed by a balanced parenthesis group - a function call:

```console
$ trex scan '@call' --text 'foo(1) bar(2, 3)'
[0..6] "foo(1)"
[7..16] "bar(2, 3)"
```

Other lenses name blocks (`@block`), key/value heads (`@kv`), command-line flags (`@flag`),
comma lists (`@list`), and numeric ranges (`@range`). They all expand to ordinary token
patterns; the [reference](../../reference/pattern-syntax/#lenses) lists them.

## Where next

Everything so far a regex could also do (more verbosely). Next,
[binding and balance](../binding-and-balance/) covers the two capabilities that make trex more
than a terser regex: named registers with back-reference, and true balanced-bracket matching.
