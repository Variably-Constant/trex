---
title: Architecture
linkTitle: Architecture
weight: 40
---

# Architecture

## Module map

| Module | Responsibility |
|---|---|
| `token` | typed-token data contract (kinds, spans, bracket mates, kind codes) |
| `lexer` | bytes to typed tokens, with bracket pairing; unicode-aware, so a multi-byte letter joins its word and a CJK run is one token, never byte shards |
| `encoding` | input transcoding: BOM-selected UTF-32/16/8, plus strict BOM-less UTF-16 detection that cannot misread binary |
| `ast` | the pattern AST, quantifier normalization, and the dependence predicates the streaming / pipeline / device paths route on |
| `parser` | surface syntax to a pattern AST |
| `bytepat` | the byte grain inside a token: a byte-pattern matcher whole-anchored to one token |
| `byte_simd` | SIMD substring search backing the content guard |
| `prefilter` | approximate-membership filters over corpus n-grams (Bloom, Cuckoo, Xor) |
| `nfa` | the single-pass engine (worst-case linear) and the device bit-NFA tables |
| `engine` | the set-reachability fold plus register environment, for balanced and field patterns |
| `parallel_lex` | multi-core tokenization, byte-identical to the serial lexer |
| `streaming` | chunk-fed scanning with whole-input equivalence |
| `dual_grain` | the byte-grain / token-grain producer-consumer pipeline |
| `gpu` | the CUDA SIMT scan backend (on by default; auto-detects the device, falls back to the CPU) |
| `rewrite` | template substitution over matches: match, render, splice |
| `grammar` | a parser generator over the universal tokens: named rules, left-recursion as precedence, EBNF quantifiers and groups, and a semiring chart |
| `bpe` | learned subword tokenization (byte-pair encoding) |
| `tokutil` / `canon` | shared token utilities and orbit canonicalization |
| `magnitude` / `spectral` / `shape` / `orbit` / `seam` / `stress` / `flow` / `observation` / `echo` | the [property axes](../../reference/axes/) |
| `resonator` | a bank of complex-pole resonators over any symbol stream: period and phase at byte, token, and unit grain |
| `relation` / `curvature` / `geodesic` / `holography` / `entanglement` / `topology` | the two-point tier: enclosure, operator, adjacency and reuse edges between tokens, and the readings over that graph |
| `profile` | the per-axis monoid that lifts a token reading to a unit, a block, or a document with no level-specific code |
| `context` | the [rolling context](../../reference/axes/context/): every axis folded over a sliding window at the token and unit rungs, the relation-admitted folds at every token, and the agreement of the grains' boundaries |
| `supertoken` | the grammar-free unit above the token: a run of tokens collapsed to a role-tagged unit |
| `custom` | user-declared token shapes: a name and a bounded byte-pattern the lexer runs alongside its built-in recognizers |
| `tandem` | CPU+GPU batch dispatch through the scheduler's hybrid join (optional `tandem` feature) |

## Execution surfaces

The same pattern and the same matches are reachable through several backends, chosen by the
caller. Each returns exactly what a plain whole-input scan returns.

| Surface | Entry | What it adds |
|---|---|---|
| Whole-input scan | `scan` | the default: single-pass engine, set-reachability for balanced or field patterns |
| Streaming | `scan_chunked` / `StreamScanner` | feed the input in chunks of any size and recover the whole-input match set; a match commits only once no later byte can change it, which for an unbounded pattern means once no attempt still running reaches back over the cut |
| Parallel tokenization | `parallel_lex::lex_parallel` | lex a large input across cores, stitched byte-identical to the serial lexer |
| Dual-grain pipeline | `scan_dual_grain` | run the byte grain and the token grain on two threads as a producer and consumer |
| Device backend | `scan_gpu`, `GpuTokens` | map the all-starts scan onto a GPU for the alternation-free subset - typed atoms, magnitude tests, literals, byte classes, spectral tests against a per-token reading of the field the host builds, and one-token back-references with host-resolved captures (a bind and a spectral test do not share a device pattern); `GpuTokens` holds the tokens and their properties on the device across scans, `upload_with_spectral` the spectral reading too; falls back to the engine otherwise and when no device is present |
| Rewrite | `rewrite` / `rewrite_with_backend` | replace each match with a rendered template |

The device backend compiles its kernel to PTX at build time and loads it through the driver at
runtime, so a deployed binary needs only the graphics driver, not a full toolkit. Its
eligibility gate keeps the device and the engine from ever disagreeing: a pattern the device
cannot represent runs on the engine instead. The automatic router places a pattern that reads
nothing of a token but its kind by what it has measured at the input's size: the engine alone,
or the input's anchors split between the cores and the device at once, the share set by each
side's measured cost. A pattern that reads a property stays on the cores; `--gpu` forces a
device attempt and `--cpu` forces the engine.

## Regex parity

On the subset trex shares with a regular expression - literals, `\N`, `\W`, `.`, sequence,
`|`, and the greedy and lazy quantifiers - the single-pass engine reports the spans the
`regex` crate reports. The measurement is `tests/conformance.rs`: it generates a pattern as one
abstract tree, renders it to both syntaxes so the two are equivalent by construction, and
compares spans over generated corpora against the crate's PikeVM, its reference machine. The
sweep ships at 20,000 cases and depth 6, and the engine agrees on every comparison at every
size run, up to 50,000 cases at depth 6 and 6,000 at depth 10.

The engine lays a loop out as the crate's compiler does, so the two rank the same derivations
the same way: a loop head is a plain split, a thread that returns to a head it already reached
at this position is dropped, and a star over a body that can match empty is laid out as
`(P+)?`. Where Perl and the crate differ - a loop iteration that matched empty ends the loop in
Perl and is dropped as a duplicate in the crate, which then tries the body's other alternatives
before the exit - trex follows the crate.

The set-reachability engine, which runs only the constructs a regular expression lacks
(balanced groups, fields, the axis predicates, `||`, `|>`, atomic groups, assertions), is
compared on the same cases and differs on 0.8 to 1.6 percent of them, on nested quantifiers
with a lazy inner: `(.+?)+` over `956 116 bar` is one match on the single-pass engine and three
there. The sweep counts those on every run.
