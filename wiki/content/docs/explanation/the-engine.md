---
title: The engine
linkTitle: The engine
weight: 20
---

# The engine

`scan` answers a pattern with the first rung of a ladder that can: a route that needs no engine,
then the single-pass engine, then the set-reachability engine. A pattern that needs a literal the
input does not contain is one a route answers before either engine runs. With `TREX_TRACE` set,
every call prints the rung that answered it to standard error.

## The single-pass engine

The single-pass engine compiles the pattern to an instruction program, a Pike-style virtual
machine, and sweeps the significant-token subsequence once, carrying a priority-ordered set of
threads deduplicated at each position. For a pattern that reads no register, a thread is keyed on
its program counter, so each program-counter and position pair is reached at most once and the
scan is linear in the token count.

A pattern that reads a register keys each thread on its registers as well, since two threads at
one program counter with different bindings are not interchangeable. A bind whose width the lexer
fixes keeps one binding alive per thread; a bind of free width keeps one per span its quantifier
could have covered. Measured by
[`backref_scaling`](https://github.com/Variably-Constant/trex/blob/main/examples/backref_scaling.rs),
as time growth over input growth, where 1.00 is linear and 2.00 quadratic:

| Pattern | Tokens | Time growth over input growth |
|---|---|---|
| `\W:x =x` | 25,000 to 6,400,000 | 0.89 to 1.03 |
| `\W+:x =x` | 500 to 8,000 | 2.05 to 2.26 |

The parser collapses nested quantifiers such as `(r*)*`, `(r+)*` and `(r?)+` to a single
quantifier when the inner expression carries no capture.

## The set-reachability engine

A pattern the single-pass engine cannot compile goes to a set-reachability fold, the operational
form of the Antimirov partial derivative. It advances a deduplicated set of reachable states, each
a token position with a register environment, and never backtracks. It takes a pattern holding
any of:

- a balanced group `\B(...)`, a field anchor, or a token edit-distance group `(A B C)~k`
- an assertion `~(P)`, `!~(P)` or `~<(P)`, an atomic group `(?>P)`, or a possessive quantifier
- the choices `||` and `|>`
- a property anchor such as `@seam`, `@phase` or `@super`
- an atom that reads a field built once per scan: a spectral test `\F{...}`, a relative
  magnitude such as `\N{>+1}`, a timestamp against a register such as `\T{>+1h:t}`, or `=kin`
- an explicit whitespace atom, `\S` or `\s`

It recomputes a nested quantifier's closure once per start position, so its worst case is
polynomial rather than linear. The attempt at each start position is independent, so on a large
stream the attempts run across cores and only the leftmost, non-overlapping selection runs on one
thread.

## Beyond regular matching

- Registers. `:name` writes the matched token's value; `=name` consumes a token only when it
  equals the bound value. The engines carry the registers alongside their states, which makes
  each a register-set automaton.
- Balanced groups. `\B(...)` consumes a token whose mate marks the matching close, and the
  interior pattern runs over the enclosed span. The lexer has already paired the brackets, so any
  depth of nesting needs no recursion in the pattern.
- The content guard. `~"lit"` is a zero-width assertion answered by a presence check over the
  forward window, not a positional scan: it asks whether the window ahead contains the literal,
  not whether the next tokens are it.

## No exponential blowup

A backtracking matcher explores every way to divide the input among nested quantifiers, so
`(a+)+b` over a run of `a` with no `b` costs time exponential in the run length. trex does not
backtrack.
[`linear_immunity`](https://github.com/Variably-Constant/trex/blob/main/examples/linear_immunity.rs)
runs both and checks from the recorded trace that the single-pass engine answered the trex scans.
Measured: the textbook backtracker grows about 4x per two added characters, from 394 µs at 14 to
4.26 s at 28, while trex's `(.*)* \N` over words with no number stays between 32.7 and 38.5 ns per
token from 50,000 to 800,000 tokens.

| Property | trex | backtracking regex |
|---|---|---|
| number atom | `\N` | `[-+]?\d+(?:\.\d+)?` |
| balanced, nested group | `\B(...)` | not expressible |
| matched open/close pair | `<\W:t>.*</=t>` | backreference, exponential risk |
| repeated token | `\W:x =x` | backreference, exponential risk |
| worst-case match time | linear with no register or a fixed-width bind; quadratic with a free-width bind; polynomial on the set-reachability engine | exponential on adversarial input |
| whitespace handling | implicit | explicit `\s*` |

## Precedence

Tightest to loosest: atoms, classes, groups and balanced groups, with assertions and guards among
them; quantifiers; the binding suffix `:name`; concatenation; choice (`|`, `||`, `|>`). The
[pattern syntax](../../reference/pattern-syntax/#precedence) states it in full.
