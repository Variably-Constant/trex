---
weight: 10
---

# Extract fields

## Pull one typed value

Name the atom for the kind you want. Emails:

```console
$ trex scan '\E' --text 'ping bob@x.com please'
[5..14] "bob@x.com"
```

Swap `\E` for `\N` (number), `\I` (IP), `\U` (URL), `\T` (timestamp), `\V` (version), and so
on - the full set is in the [pattern syntax](../../reference/pattern-syntax/#token-atoms)
reference.

## Pull key/value pairs

Bind each side to a register. A literal `=` token is written `"="` (a bare `=` is the
back-reference operator):

```console
$ trex scan '\W:k "=" \Q:v' --text 'name = "bob" city = "nyc"'
[0..12] "name = \"bob\""  captures: k="name", v="\"bob\""
[13..25] "city = \"nyc\""  captures: k="city", v="\"nyc\""
```

Each match line carries the bound `k` and `v`. Add `--json` for a machine-readable array whose
objects include the `captures`.

## Anchor to a column

`@k P` matches `P` only in the k-th comma-delimited field, so you can extract the third column
of a CSV row without a parser. The `@list` lens matches a whole comma-separated run. See the
[lenses](../../reference/pattern-syntax/#lenses) reference.
