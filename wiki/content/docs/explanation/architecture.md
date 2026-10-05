---
title: Architecture
linkTitle: Architecture
weight: 40
---

## Module map

Every public module of the `trex` crate.

| Module | Responsibility |
|---|---|
| `action` | the action reading: the magnitude energy `sum(m^2)` and the stress load `sum(depth)` combined over a span |
| `ast` | the pattern AST the parser emits and the engines consume, with quantifier normalization and the predicates the streaming, pipeline and device paths route on |
| `bpe` | learned subword tokenization (byte-pair encoding) |
| `builder` | a pattern built with its readings named, rather than parsed |
| `byte_dfa` | the determinizer over a byte-grain automaton, built as it is walked |
| `byte_lex` | the lexer's recognizers as byte automata |
| `byte_nfa` | a byte-grain nondeterministic automaton and the simulation that reads it |
| `byte_simd` | SIMD substring search (AVX2, SSE2 or scalar, chosen at run time, each returning the scalar path's position) under the content guard |
| `bytepat` | the byte grain inside a token: a byte-pattern matched against one whole token |
| `canon` | the canonical representative of a symmetry orbit |
| `captures` | a capture buffer the caller owns and refills, and the pattern properties fixed before any input is seen |
| `context` | the [rolling context](../../reference/axes/context/): every axis folded over a sliding window at the token and unit rungs, and the agreement of the grains' boundaries |
| `cursor` | matches taken one at a time, and the operations that stop before the end |
| `curvature` | for each edge of the relation graph, how many neighbors its two ends share: positive in a dense region, negative at a bridge |
| `custom` | the declarations a lex and a parse share: token shapes, token kinds declared from patterns over the stream, and the named sub-patterns `\{name}` inlines |
| `decoded` | the decoded content of an encoded token: a base64 blob's bytes, a JSON Web Token's header and payload |
| `dual_grain` | lexing and matching on two threads as a producer and a consumer ([dual-grain scanning](../dual-grain/)) |
| `e8` | the E8 root lattice and its Weyl group, the symmetry under the deepest `orbit` rung |
| `echo` | the [echo axis](../../reference/axes/echo/): whether each token's content occurs elsewhere, how often, how far away and how regularly |
| `edit` | edit distance over whole tokens: whether one text is within `k` character insertions, deletions or substitutions of another |
| `encoding` | input transcoding: UTF-32, UTF-16 or UTF-8 selected by a byte-order mark, and BOM-less UTF-16 detected strictly enough never to misread binary |
| `ends_simd` | the leftmost, non-overlapping selection: the first anchor in a range whose longest match ends past it, found a vector at a time |
| `engine` | the set-reachability engine with its register environment, for the patterns the single-pass engine routes away |
| `entanglement` | the count of enclosure, operator and reuse edges crossing each cut of the token stream; a cut with none splits two independent parts |
| `explain` | what a match is made of, for `--explain`: the kinds of its tokens, the guard each passed, every axis the pattern read there, and the rung that answered |
| `files` | the inputs a command reads: a tree walk under ignore rules, the binary check, the line and column of a byte offset, and a unified diff of a rewrite's edits |
| `flow` | the [flow axis](../../reference/axes/flow/): the windowed slope, direction, momentum and reversals of any per-token signal |
| `follow` | files followed as they grow: the bytes appended to each, and a note where one was truncated, replaced or removed |
| `gauge` | a term with each bound name replaced by its de Bruijn index, so terms that differ only by renaming compare equal |
| `geodesic` | the shortest-path distance between two tokens through the relation graph, read against their distance in the stream |
| `gpu` | the CUDA SIMT scan backend (the default `gpu` feature): detects the device at run time and falls back to the CPU |
| `grammar` | a parser generator over the typed tokens: named rules, left recursion as precedence, EBNF quantifiers and groups, and a semiring chart |
| `gravity` | the pull the input shows one unit type to have on another at a gap, and the readings taken from it |
| `holography` | whether the sequence of open and close bracket events alone reconstructs the lexer's bracket pairing |
| `index` | an index over a tree that says which files a pattern cannot match, so a scan never opens them |
| `infer` | pattern inference: the most specific pattern every example matches, read off an alignment of their token sequences |
| `isa` | which instruction-set rung this CPU can run, resolved once |
| `kind_route` | a pattern that is a fixed sequence of token kinds, such as `\W \N`, answered over the lexer's chunk parts in place |
| `lexer` | bytes to typed tokens with bracket pairing; a multi-byte letter joins its word and a CJK run is one token |
| `library` | the shipped library: named token kinds with a shape and, where a standard defines one, a checksum guard, and named sub-patterns |
| `magnitude` | the [magnitude axis](../../reference/axes/magnitude/): each token's order of magnitude, with its energy and gradient |
| `nfa` | the single-pass engine, a Pike-style virtual machine over the token stream, and the device's bit-NFA tables |
| `observation` | the [observation axis](../../reference/axes/observation/): byte-class entropy read from past-only, future-only and centered windows, and where they disagree |
| `orbit` | the [orbit axis](../../reference/axes/orbit/): a token's class under a symmetry such as case, shape or notation |
| `paint` | color for reports: the depth a console renders, the roles a report paints, and the overrides `--colors` spells |
| `parallel_lex` | tokenization across cores, byte-identical to the serial lexer |
| `parser` | a pattern string to a pattern AST |
| `pattern_set` | many patterns matched against one input from one lex |
| `prefilter` | approximate-membership filters over a corpus's n-grams (Bloom, Cuckoo, Xor) answering whether a literal might occur, with no false negatives |
| `prior_cache` | the baked prior laid out on disk as the coder's tables and mapped rather than decoded (the `compress` feature) |
| `profile` | the per-axis monoid that lifts a token reading to any unit above it |
| `quantity` | physical quantities: a number with a unit symbol, compared within the unit's family after normalizing to its base unit |
| `records` | the units a record query (`--all`, `--any`, `--none`, `--at-least`) is asked of: lines, paragraphs, runs between matches, or regions an axis finds |
| `relation` | the [relation axis](../../reference/axes/relation/): enclosure, operator, adjacency and reuse edges between tokens |
| `report` | the text of a scan's report for every surface: a match, its registers and an explanation as JSON, and lines with their matches painted |
| `resonator` | a bank of complex-pole resonators over any symbol stream: period and phase at byte, token and unit grain |
| `rewrite` | template substitution over matches: match, render, splice |
| `rule_scan` | the rules of pattern files scanned over an input, each finding reported under its rule with its message, severity and fix |
| `seam` | the [seam axis](../../reference/axes/seam/): segmentation where the stream stops predicting itself, read in both directions |
| `shape` | the [shape axis](../../reference/axes/shape/): each token's shape, the period at which shapes repeat, and the regions that repeat like a table |
| `spectral` | the [spectral axis](../../reference/axes/spectral/): the texture, entropy and period of the byte stream at each position |
| `streaming` | chunk-fed scanning that recovers exactly the matches a whole-input scan produces |
| `stress` | the [stress axis](../../reference/axes/stress/): nesting depth, how long open spans are held, and where a deep structure closes |
| `supertoken` | the unit above the token: a run of tokens collapsed to a role-tagged unit, with no grammar |
| `tandem` | CPU and GPU batch dispatch through the scheduler's hybrid join (the default `tandem` feature, which implies `gpu`) |
| `templates` | log-template mining: lines grouped by token-kind silhouette, and the rarity of each line's template |
| `token` | the typed-token contract the lexer and the engines share: kinds, spans, bracket mates, kind codes |
| `tokutil` | token utilities the axes share: significant-token lexing, a hasher for pre-hashed keys, and the identifier-shape classifier |
| `topology` | the relation graph's components, independent cycles `b1 = E - V + b0`, and Euler characteristic |
| `trace` | which rung of a ladder answered a call |
| `typed` | typed value predicates: a kind atom's `{...}` body compared in the type's own units |
| `window` | the part of an input a head, a tail or a line range selects, and readers that fetch only that much |

## Supertokens

A supertoken is a run of tokens read as one unit with a structural role. A run ends at a `;`, a
newline or a paired bracket, and is at the depth of the brackets around it, so the arguments
of a call are a supertoken one level deeper than the call's name. Its role is read from its
punctuation and its brackets, never from a language's keywords, as the first of these it fits:

| Role | A run that |
|---|---|
| `call` | ends in a word an opening bracket follows |
| `assign` | holds a lone `=` |
| `kv` | opens with a word and a `:` |
| `list` | holds two or more commas |
| `numeric` | holds a number, and at least as many numbers as words |
| `plain` | is none of these |

It is the chunk of shallow parsing, a run read as one phrase with no grammar, except that the
brackets nest it. `x = 1` and `let x = 1` are both assignments and `foo(a, b)` is a call whatever
the language, so one reading holds for code, configuration, logs and prose:

```console
$ trex scan '@super .' --format '${0} | ${@super.role} | depth ${@super.depth}' --text $'x = 1\nlet x = 1\nx: 1\nfoo(a, b)\nred, green, blue\n12 34 5\nhello world'
x | assign | depth 0
let | assign | depth 0
x | kv | depth 0
foo | call | depth 0
a | plain | depth 1
red | list | depth 0
12 | numeric | depth 0
hello | plain | depth 0
```

`@super` and `@super:ROLE` anchor a pattern on supertokens
([pattern syntax](../../reference/pattern-syntax/#axis-predicates-and-anchors)), `--record unit`
and `unit:ROLE` make each one a record ([record units](../../reference/records/#record-units)),
and `${@super}` reads the supertoken a match is in. Seam, observation, flow, gravity and context read
supertokens as a grain of their own, and echo's `--structure` keys them by role.

## Execution surfaces

The same pattern and the same matches are reachable through several backends, chosen by the
caller. Each returns exactly what a plain whole-input scan returns.

| Surface | Entry | What it adds |
|---|---|---|
| Whole-input scan | `scan` | the default: a route where one answers, then the single-pass engine, then the set-reachability engine ([the engine](../the-engine/)) |
| Streaming | `scan_chunked` / `StreamScanner` | feed the input in chunks of any size and recover the whole-input match set; a match commits only once no later byte can change it, which for an unbounded pattern means once no attempt still running reaches back over the cut |
| Parallel tokenization | `parallel_lex::lex_parallel` | lex a large input across cores, stitched byte-identical to the serial lexer |
| Dual-grain pipeline | `scan_dual_grain` | run lexing and matching on two threads as a producer and a consumer |
| Device backend | `scan_gpu`, `GpuTokens` | map the all-starts scan onto a GPU for the alternation-free subset - typed atoms, magnitude tests, literals, byte classes, spectral tests against a per-token reading of the field the host builds, and one-token back-references with host-resolved captures (a bind and a spectral test do not share a device pattern); `GpuTokens` holds the tokens and their properties on the device across scans, `upload_with_spectral` the spectral reading too; falls back to the engine otherwise and when no device is present |
| Rewrite | `rewrite` / `rewrite_with_backend` | replace each match with a rendered template |

The device backend compiles its kernel to PTX at build time and loads it through the driver at
runtime, so a deployed binary needs only the graphics driver, not a full toolkit. Its
eligibility gate keeps the device and the engine from ever disagreeing: a pattern the device
cannot represent runs on the engine instead. The automatic router places a pattern that reads
only each token's kind by what it has measured at the input's size: the engine alone,
or the input's anchors split between the cores and the device at once, the share set by each
side's measured cost. A pattern that reads a property stays on the cores; `--gpu` forces a
device attempt and `--cpu` forces the engine.

## Regex parity

On the subset trex shares with a regular expression - literals, `\N`, `\W`, `.`, sequence,
`|`, and the greedy and lazy quantifiers - the single-pass engine reports the spans the `regex`
crate reports. The measurement is `tests/conformance.rs`: it generates a pattern as one abstract
tree, renders it to both syntaxes so the two are equivalent by construction, and compares spans
over generated corpora against the crate's PikeVM, its reference machine. The test runs 20,000
cases at depth 6 by default, and `TREX_CONFORMANCE_CASES` and `TREX_CONFORMANCE_DEPTH` widen it:

| Cases | Depth | Comparisons | Single-pass engine disagrees | Set-reachability engine disagrees |
|---|---|---|---|---|
| 20,000 | 6 | 118,520 | 0 | 73 |
| 50,000 | 6 | 296,592 | 0 | 172 |
| 6,000 | 10 | 35,992 | 0 | 54 |

The engine lays a loop out as the crate's compiler does, so the two rank the same derivations
the same way: a loop head is a plain split, a thread that returns to a head it already reached
at this position is dropped, and a star over a body that can match empty is laid out as
`(P+)?`. Where Perl and the crate differ - a loop iteration that matched empty ends the loop in
Perl and is dropped as a duplicate in the crate, which then tries the body's other alternatives
before the exit - trex follows the crate.

The set-reachability engine, which runs only the constructs the single-pass engine routes away,
is compared on the same cases. Its disagreements are nested quantifiers with a lazy inner, where
it returns several shorter matches in place of one: `(((\N \W . | .) (\N*){2,3}? . | \N*? | \W+?))+ .`
over `792 foo 313 baz` is the one match `[0..15]` on the single-pass engine and the PikeVM, and
`[0..11]`, `[12..15]` on the set-reachability engine. The test prints every such case on each
run.
