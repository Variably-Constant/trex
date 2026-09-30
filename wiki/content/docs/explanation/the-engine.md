---
title: The engine
linkTitle: The engine
weight: 20
---

# The engine

trex matches with two engines behind one entry point.

## Two engines, one entry

The **primary engine** compiles the pattern to an instruction program and sweeps the
significant-token subsequence once, carrying a priority-ordered set of threads deduplicated by
program counter at each position. Each program-counter / position pair is reached at most once,
so matching is linear in the token count for a fixed pattern, with no backtracking and no
per-position recompute.

Patterns with a **balanced-bracket group, a field anchor, or an axis predicate** route to the
**second engine**, a set-reachability fold - the operational form of the Antimirov partial
derivative. It advances a deduplicated set of reachable states with no backtracking, which
handles the variable-length advance a balanced group needs. Adversarial inputs run in
polynomial time on this path rather than the exponential blowup of a backtracking matcher, and
the per-start attempts fan out across cores.

The parser normalizes nested quantifiers before matching, collapsing `(r*)*`, `(r+)*`, `(r?)+`
and related Kleene-identity forms to a single quantifier when the inner expression carries no
capture, which keeps both engines off the redundant nested form.

## Beyond regular matching

- **Register environment.** Partial-derivative states carry a small register map. `:name`
  writes the just-matched token's value; `=name` consumes a token only when it equals the
  bound value. This is a register-set automaton with no catastrophic backtracking on a fixed
  alphabet, unlike a backreference.
- **Balanced-group primitive.** `\B(...)` consumes a token whose `mate` index marks the
  matching close; the interior pattern runs over the enclosed span. Arbitrary nesting is
  handled by the lexer's pairing, so the pattern needs no recursion rule.
- **Content guard.** `~"lit"` is a zero-width assertion answered by a presence prefilter over
  the forward window, not a positional scan. It asks whether the field ahead *contains* the
  literal, not whether the next tokens *are* it.

## No ReDoS, by construction

A backtracking matcher explores every way to divide the input among nested quantifiers, so a
pattern like `(a+)+b` over a run with no `b` costs time exponential in the run length. trex has
no backtracking: the single-pass engine visits each program-counter / position pair at most
once.

| Property | trex | backtracking regex |
|---|---|---|
| number atom | `\N` | `[-+]?\d+(?:\.\d+)?` |
| balanced, nested group | `\B(...)` | not expressible |
| matched open/close pair | `<\W:t>.*</=t>` | backreference, exponential risk |
| repeated token | `\W:x =x` | backreference, exponential risk |
| worst-case match time | linear (single-pass), polynomial fallback | exponential on adversarial input |
| whitespace handling | implicit | explicit `\s*` |

The [`linear_immunity`](https://github.com/Variably-Constant/trex/blob/main/examples/linear_immunity.rs)
example runs both side by side: the backtracker's time multiplies with each added character
while trex's stays flat per token.

## Precedence

Tightest to loosest: atoms / classes / groups / balanced groups; quantifiers (postfix);
binding suffix `:name`; concatenation; content guard `~"lit"`; ordered choice `|`.
