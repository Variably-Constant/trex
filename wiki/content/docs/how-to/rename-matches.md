---
weight: 20
---

# Rename or reshape matches

`rewrite` replaces each match with a rendered template. `${name}` renders a captured register,
`${0}` the whole match, and `${name:acc}` transforms or slices a capture - `upper`, `lower`,
`trim`, or a typed sub-field accessor, chaining with `|`.

## Reduce a matched tag to its name

Bind the tag word, then render only it:

```console
$ trex rewrite '<\W:t>.*</=t>' '[${t}]' --text '<b>hi</b> <i>yo</i>'
[b] [i]
```

Because the match side understands balance and binding, the close tag was required to equal the
open (`=t`), so only well-formed pairs are touched.

## Redact a typed value

```console
$ trex rewrite '\E:e' '[redacted]' --text 'mail bob@x.com now'
mail [redacted] now
```

## Slice a captured atom

A captured typed atom decomposes by its structure - no second pattern. Keep the first two
octets of an IPv4:

```console
$ trex rewrite '\I:ip' '${ip:octet1-2}.0.0/16' --text 'conn from 192.168.5.9'
conn from 192.168.0.0/16
```

The same `\I` matched as IPv6 uses `group1-3` instead of `octet1-2`. Other atoms carry their
own fields: `${url:host}`, `${email:domain}`, `${ver:major}`, `${ts:year}`, `${path:ext}`.
Accessors chain, so `${email:domain|upper}` renders the domain upper-cased.

## Transform in place

`--in-place` writes the result back to the `FILE` instead of stdout. `$$` renders a literal
dollar sign. The rewrite inherits the engine's no-backtracking guarantee, so it cannot stall on
adversarial input the way a backtracking substitution can.
