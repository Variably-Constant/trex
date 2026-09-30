---
title: trex
toc: false
---

<p align="center">
  <em>Token-Regular EXpression. A regex you can read, over typed tokens instead of raw bytes, with named long-distance binding and no catastrophic backtracking.</em>
</p>

---

**trex** matches patterns over **typed tokens** - a number, a word, a quoted string, a balanced
bracket group - instead of raw bytes. A structure-aware lexer does the recognizing up front, so
by the time a pattern runs, `\N` is already a whole number and `\B(...)` is already a matched
bracket group.

That shift buys back what a regex gives up. Brackets pair, so a nestable group is one glyph.
Bindings have names: `:name` writes a token into a register, `=name` later demands one equal to
it - a backreference without the backtracking. And `~"lit"` asks whether a literal lies
somewhere ahead, not whether the next token happens to be it. All of it runs in linear time,
with no ReDoS cliff.

## What it looks like

```console
$ trex scan '<\W:t>.*</=t>' --text '<div>hi</div>'
[0..13] "<div>hi</div>"  captures: t="div"
```

`<\W:t>` binds the open-tag word to the register `t`; `.*` matches any run of tokens; `=t`
requires the close tag to equal the bound value. `<div>...</span>` does not match, and the
whole thing is a balanced, back-referenced match evaluated with no backtracking.

```console
$ trex scan '\E:e' --text 'ping bob@x.com' --json
[{"start":5,"end":14,"text":"bob@x.com","captures":{"e":"bob@x.com"}}]
```

## Two command families

trex is one binary with two families. The **pattern-language tools** match, transform, and
parse a pattern over the token stream:

- [`scan`](docs/reference/cli/#scan) prints each match and its captured registers,
- [`rewrite`](docs/reference/cli/#rewrite) replaces each match with a rendered template,
- [`grammar`](docs/reference/cli/#grammar) parses against a token grammar,
- [`bpe`](docs/reference/cli/#bpe) learns a subword tokenizer,
- [`prefilter`](docs/reference/cli/#prefilter) is an approximate-membership check.

The **property-axis analysers** read the numeric fields the token stream carries beyond its
identity - scale, temporal texture, silhouette, symmetry, predictive segmentation, structural
load, dynamics, vantage, and recurrence. Each is a standalone command and a pattern predicate:

```console
$ trex magnitude --text 'retries = 3 ; max_bytes = 5000000000'
trex magnitude: 36 bytes, 7 tokens, total energy 112.2, 3 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
```

The huge value is invisible to a byte regex, to shape, and to texture; the
[magnitude axis](docs/reference/axes/magnitude/) is the only reading that sees it.

## No ReDoS, by construction

A backtracking regex explores every way to divide a run among nested quantifiers, so
`(a+)+b` over a run of `a`s with no `b` costs time exponential in the run length. trex has no
backtracking: the single-pass engine visits each program-counter / position pair at most
once, so the analogous token pattern stays linear. The
[`linear_immunity`](https://github.com/Variably-Constant/trex/blob/main/examples/linear_immunity.rs)
example runs both side by side.

## Three doors in

If you came to **write patterns**, start with the
[getting-started tutorial](docs/tutorial/getting-started/): from zero to a working scan, then
binding, balance, and the axes.

If you came to **look something up**, the [reference](docs/reference/) has the command
surface, the full pattern grammar, and one page per axis.

If you came to **understand the design**, the [explanation](docs/explanation/) covers the two
engines, the dual-grain architecture, and why the axes are orthogonal.

## Reading order

The guide follows [Diátaxis](https://diataxis.fr/): Tutorial, How-To, Reference, Explanation.
They are independent; read in any order.

- **[Tutorial](docs/tutorial/)** is a teacher. It assumes nothing and walks you from your
  first scan to advanced patterns.
- **[How-To](docs/how-to/)** is a recipe book. Each page solves one task.
- **[Reference](docs/reference/)** is the operating manual. Every command, every atom, every
  axis.
- **[Explanation](docs/explanation/)** is the design rationale.

## License

trex is licensed under MIT.
