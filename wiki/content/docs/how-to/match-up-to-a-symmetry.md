---
weight: 50
---

# Match up to a symmetry

Sometimes "the same" means the same up to case, notation, or shape rather than byte-for-byte.
The [orbit axis](../../reference/axes/orbit/) folds each token to its canonical representative
under a symmetry group.

## Find every span in a query's orbit

`orbit --match` returns every span equivalent to the query under the group. Under the case
group, `cat` matches `Cat`, `CAT`, and `cat`:

```console
$ trex orbit --match cat --group case --text 'the Cat and the CAT and a cat'
trex orbit --match "cat" (group case): 3 spans in the same orbit
    [     4..7     ] "Cat"
    [    16..19    ] "CAT"
    [    26..29    ] "cat"
```

## Back-reference up to a symmetry inside a pattern

Inside a `scan` pattern, `=shape name` matches a later token in the bound token's shape orbit.
`cat` and `bat` share the shape `CVC`:

```console
$ trex scan '\W:w =shape w' --text 'cat bat xyz'
[0..7] "cat bat"  captures: w="cat"
```

Use `=case name` or `=notation name` for the case and notation orbits. To collapse a whole
stream to its orbit vocabulary, use `orbit --collapse --group GROUP`.
