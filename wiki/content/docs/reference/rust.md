---
title: Rust API
linkTitle: Rust
weight: 40
---

The items of the `trex` crate, grouped by task, each linking the page its examples are on. The
[generated reference](https://docs.rs/trex-re) lists every item of the released crate with its
full signature.

## The dependency

The package on crates.io is `trex-re`, and the crate it names in code is `trex`:

```toml
[dependencies]
trex-re = "0.3.0"
```

From a checkout: `trex = { package = "trex-re", path = "../trex" }`. The published crate leaves
out the prior the `compress` feature compiles in, so that feature builds from a checkout.

## Patterns and matches

| Item | What it is |
|---|---|
| `parse(src) -> Result<Pattern, ParseError>` | compile a pattern ([pattern syntax](../pattern-syntax/)) |
| `parser::parse_with_shapes(src, &shapes)`, `parse_with_inputs(src, &shapes, &[(name, bytes)])`, `parse_with_empty_loop` | compile against declarations, named second inputs, or an empty-loop reading |
| `PatternBuilder` | `new(src)`, `case_insensitive`, `orbit(group)`, `swap_greed`, `nest_limit`, `empty_loop`, `build()` |
| `Pattern::capture_names()`, `capture_kinds()` | the registers a pattern binds, and the one kind each binds where it has one |
| `scan(&pattern, input) -> Vec<Span>` | every leftmost, non-overlapping match ([matches](../matching/#matches)) |
| `scan_with_shapes(&pattern, input, &shapes)`, `scan_with_backend(&pattern, input, Backend)` | the same under declarations, or on a chosen backend |
| `is_match(&pattern, input)` | whether the pattern matches |
| `Span` | `start`, `end` as `u32`; `start()`, `end()` and `range()` as `usize` |
| `captures(&pattern, input, &spans) -> Vec<Match>` | the registers bound over each span; `captures_over`, `captures_with_shapes`, `captures_with_empty_loop`, and the `_with_lists` forms that keep every binding under a repetition |
| `Match` | `start`, `end`; `group(name, input)`, `group_span(name)`, `group_at(i, input)`, `captures()` and `names()` sorted by name, `span()`, `extract::<N>(input)`, `list(name)` and `lists()` for the bindings under a repetition |
| `Regs`, `INLINE_REGS` | how a match holds its registers: up to `INLINE_REGS` inline, the rest shared |
| `find`, `find_at`, `find_iter`, `is_match_at` | the first match, the first at or after a byte, or each in turn as asked for |
| `captures_first`, `captures_at`, `captures_iter`, `Cursor`, `MatchCursor`, `MatchRef` | the same with their registers |
| `shortest_match`, `shortest_match_at` | how far the soonest-ending match reaches |
| `split`, `splitn`, `split_at_depth` | the input split on the matches, or on those at one bracket depth |
| `escape(text)` | a pattern matching exactly `text` |
| `capture_names(&pattern)`, `captures_len(&pattern)` | the names a pattern binds, in binding order |
| `CaptureSlots`, `captures_read`, `captures_read_at`, `captures_read_iter`, `SlotCursor` | matches written into one reused buffer |
| `static_captures_len`, `static_token_extent` | the registers, or the tokens, every match of a pattern has, where fixed |
| `stats::ScanStats`, `Route`, `Counting` | what scans read and found, as `--stats` reports it: `count` and `count_records` add an input, `read_trace` takes the lexes and the rungs TREX's trace kept, `matching()` is the time left for matching, and `Counting::start` and `finish` time scans and read the trace ([statistics](../matching/#statistics)) |

## Typed values and the clock

| Item | What it is |
|---|---|
| `typed::TypedValue` | a register's value through the parse a `:value` clause compares on; `json(kind, ValueStyle, Clock)` spells it ([typed values](../matching/#typed-values)) |
| `typed::value_of(kind, text)` | the value of a span of a kind |
| `typed::ValueStyle`, `ValueSpelling` (`Exact`, `Natural`, `Tagged`), `DurationUnit` (`Nanoseconds`, `Milliseconds`, `Seconds`) | how a value is spelled as JSON |
| `set_now(Option<i64>)`, `now_override()`, `typed::Clock` | the instant `now` reads ([typed value predicates](../pattern-syntax/#typed-value-predicates)) |
| `set_tz_offset(seconds)`, `set_date_order_day_first(bool)`, `date_order_day_first()` | the zone of an unzoned timestamp, and which field of an all-numeric date is the day |

## Pattern sets and streams

| Item | What it is |
|---|---|
| `PatternSet` | `new(patterns)`, `named`, `from_text(text, &mut shapes)`, `from_file`; `matches`, `matched`, `matches_at`, `is_match`, `matches_with_spans`, `scan`, `scan_matches`, `first_matches`; `names`, `name(i)`, `shapes`; `capture_names`, `capture_kinds` across the members ([pattern sets](../matching/#pattern-sets)) |
| `SetMatches` | which members matched, as a bitset: `matched(i)`, `matched_any`, `matched_all`, `iter` |
| `StreamScanner` | `new(pattern)`, `with_shapes`, `over_set(set)`; `push(chunk)`, `drain_committed`, `finish`, and the `_with_members` forms ([streams](../tools/#streams)) |
| `HeldStream` | a stream scanner over an input that starts mid-way: `push` and `finish` return the committed matches with their lines |
| `scan_chunked(&pattern, chunks)` | the whole-input matches of an input fed in chunks |

## Rewriting and redaction

| Item | What it is |
|---|---|
| `Template::parse(src, &capture_names)` | a rewrite template checked against the pattern ([templates](../rewriting/#templates)) |
| `rewrite`, `rewrite_first`, `rewrite_n` | replace every match, the first, or the first `n` ([the first matches](../rewriting/#the-first-matches)) |
| `rewrite_with`, `rewrite_n_with` | replace through a closure taking a `Matched` ([computed replacements](../rewriting/#computed-replacements)) |
| `rewrite_with_backend` | `rewrite` on a chosen backend |
| `Matched` | `start()`, `end()`, `text()`, `group(name)`, `names()`, `value(name) -> Option<TypedValue>`, `get(reference)` |
| `Reference`, `Field` | a template reference parsed against the pattern's names: `read(&match, input)`, `apply(text)` |
| `Keep::parse_list(src, &names)`, `Mask::parse(src)` | the fields a redaction keeps and how it masks ([mask and keep](../redaction/#mask-and-keep)) |
| `redactions(input, &matches, &keeps, &mut mask)`, `redactions_with_shapes` | the edits a redaction makes |
| `ReportAt`, `ReportField`, `ReportRule` | what a report template reads: the place of a match and the rule that made it |

## Tables

| Item | What it is |
|---|---|
| `aggregate::Table` | `new(columns)`, `add(key, &match, input)`, `rows(Order) -> Vec<Row>`, `columns()`, `matched()`, `keys()`: matches counted under a rendered key, with the values each column reads ([count by a key](../aggregates/#count-by-a-key)) |
| `aggregate::columns(&asked, &capture_kinds, Percentile)`, `Column`, `ColumnError`, `one_register(spec)` | the columns asked for, each an aggregate and a register, checked against the kind the pattern binds before anything is read ([aggregates](../aggregates/#aggregates)) |
| `aggregate::Order` (`Count`, `Key`), `Row` | how the rows are ordered; a row's `key`, `count` and each column's values |
| `typed::Agg` (`Sum`, `Avg`, `Min`, `Max`, `Pct(p)`), `typed::Percentile`, `typed::Collected` | an aggregate, how a percentile between two values is decided, and the values a column collected: `report(..)` renders the aggregate, `picked(..)` hands back the value it lands on ([averages and percentiles](../aggregates/#averages-and-percentiles)) |
| `Template::parse_report(src, &capture_names)`, `render_report(&match, input, &ReportAt)` | the key a table groups by |

## Records, windows and files

| Item | What it is |
|---|---|
| `records::RecordUnit` | `parse(name)`, `records(input) -> Vec<(usize, usize)>` ([record units](../records/#record-units)) |
| `records::Query`, `Quantifier` | a record query: `hits(input, &shapes, ..)` ([record queries](../records/#record-queries)) |
| `window::Select` (`Head`, `Tail`, `Range`, `parse_range`), `window::window_of(input, select, &unit)` | part of an input by records ([first, last and a range](../windows/#first-last-and-a-range)) |
| `window::read_file(path, select, &unit, Asked)`, `read_file_with_chars(..)` | a file's part read from the end nearest it and no further; the second also counts the characters ahead of the part, in the same pass |
| `follow::Follower` | `new(&[(path, offset)])`, `wait`, `poll`: what each file gains ([following a file](../windows/#following-a-file)) |
| `files::collect(&paths, &WalkOptions)`, `Source`, `read_source`, `LineIndex` | walk paths as a scan does and read what it finds ([files and trees](../matching/#files-and-directory-trees)) |
| `encoding::decode(bytes)` | a BOM-declared or strictly detected UTF-16 input as UTF-8 |

## Declarations and rules

| Item | What it is |
|---|---|
| `ShapeSet` | `new`, `declare`, `declare_kind`, `declare_let`, `declare_test`, `declare_text`, `declare_file`, `declare_lines`, `run_tests`; `shapes`, `lets`, `tests`, `rules`, `rule_of`, `name_of(id)`, `fields_of`; `base_dir`, `set_base_dir` ([pattern files](../pattern-files/)) |
| `Precedence` (`Before`, `After`), `TokenShape`, `ShapeError` | where a shape is tried, the shape, and a refused declaration ([shapes](../pattern-files/#shapes)) |
| `LibTest`, `TestFailure` | a `test` line and an expectation it did not meet ([tests](../pattern-files/#tests)) |
| `Rule`, `Severity` | a declared rule: its pattern, message, fix, files, `unless` patterns, record unit and metadata ([rules](../pattern-files/#rules)) |
| `rule_scan::RuleScan::new(shapes)` | every rule of a set scanned as one |
| `rule_scan::SharedRuleStream` | a stream of findings holding its scan in an `Arc`, for a caller that keeps it across calls |
| `declarations::Declarations` | `new`, `declare(line, base)`, `add(&AtomDecl, base, replace)`, `include(&paths)`, `remove(&names, &files)`, `clear`, `set()`, `declares(name)`, `atoms()`, `files()`, `with_files(&files)`: declarations kept line by line, so one can be taken back out with what adds to it ([libraries](../pattern-files/#libraries)) |
| `declarations::{Form, Atom, AtomDecl, DeclareError, Kept}`, `atoms_in_file(path)`, `pattern_files(&paths)` | what a line declares, one declared atom, an atom to declare, a refused declaration, why a removal kept something; a file's atoms; the `.trex` files paths name, a directory's in path order |
| `ShapeSet::kind_name(kind)` | the name a token kind reads by, a declared kind's among them |
| `review::review(queue, ask)`, `Queued`, `Change`, `Answer`, `Reviewed`, `Skipped` | fixes put to a reviewer one at a time, each accepted, skipped or replaced, a whole template at once, all the rest, or none ([lint with rules](../../how-to/lint-with-rules/)) |
| `library` | the shipped library of named patterns ([the shipped library](../pattern-files/#the-shipped-library)) |

## Building patterns

| Item | What it is |
|---|---|
| `templates::Mining` | `mine(input)`, `mine_tokens`, `mine_records(&tokens, input, &records)`: each record shape once with its count ([templates](../building-patterns/#templates)) |
| `infer::infer(&examples)`, `infer_against(&examples, &counters)` | the most specific pattern every example matches ([a pattern from examples](../building-patterns/#a-pattern-from-examples)) |
| `infer::marks::parse_lines(src)` | lines with fields marked `{name:text}` |
| `infer::build::{Spec, Mint, build, Built}` | the pattern extracting marked fields ([a pattern that extracts fields](../building-patterns/#a-pattern-that-extracts-fields)) |
| `infer::build::fields_for(source, &pattern, &shapes)`, `read_records` | the fields a saved build reads, and the records they read ([saving a build](../building-patterns/#saving-and-reusing-a-build)) |

## Explanations

| Item | What it is |
|---|---|
| `explain::Explainer::new(&pattern, input, &shapes)` | `explain(&match, &route) -> Explanation`, `tokens(range)` ([explanations](../matching/#explanations)) |
| `explain::Explanation` | `tokens`, `guards`, `readings`, `route`; `field(axis, piece, at)` |
| `trace::Recording::start()`, `trace::take_recorded()`, `trace::clear()`, `explain::route_of(&rungs)` | keep the rungs a scan takes, and name the route they make |

## Tools

| Item | What it is |
|---|---|
| `index::Index` | `build(root, &paths)`, `save(root)`, `load(root)`, `refuses(&pattern, path)`; `Summary::of(input)`, `Needs::of(&pattern)` ([indexes](../tools/#indexes)) |
| `Grammar`, `Node`, `GrammarError` | `Grammar::parse(src)`, `with_start`, `parse_input`, `count_parses`, `best_parse_prob`, `total_prob` ([grammars](../tools/#grammars)) |
| `Grammar::segment`, `count_segmentations`, `best_segmentation_prob`, `grammar::Tiling`, `Lattice::segment` | each tiling of a run-together string the grammar accepts, most probable first, as `words`, `probability` and `parses`; every parse of every tiling counted; the most probable's probability; the lattice of every tiling ([segmentation](../tools/#segmentation)) |
| `bpe::Bpe` | `train(corpus, merges)`, `encode(text)`, `to_lines()`, `parse(lines)` ([byte-pair encoders](../tools/#byte-pair-encoders)) |
| `prefilter::{BloomFilter, CuckooFilter, XorFilter}`, `Membership`, `verify(corpus)` | `build(corpus)`, `might_contain(literal)` ([prefilters](../tools/#prefilters)) |
| `seam` coder items | with the `compress` feature ([compression](../tools/#compression)) |

## Lexing

| Item | What it is |
|---|---|
| `lexer::lex(input) -> Vec<Token>` | the token stream a scan reads ([tokens](../records/#tokens)) |
| `token::Token`, `TokenKind` | a token's kind and span |
| `supertokens(input) -> Vec<SuperToken>`, `Role` | the [supertokens](../../explanation/architecture/#supertokens), each with its role: a call, an assignment, a key and value, a list, a run of numbers or plain text |

## Axes

Each axis module reads a field keyed by byte offset ([property axes](../axes/)).

| Module | Entry | Field |
|---|---|---|
| `spectral` | `analyze`, `analyze_with`, `analyze_needing`, `regions`, `code_regions`, `high_entropy_runs` | `SpectralField` |
| `seam` | `analyze`, `analyze_with`, `analyze_with_model`, `english_model`, `analyze_tokens`, `analyze_supertokens` | `SeamField` |
| `shape` | `analyze_bytes`, `analyze_over`, `classified_regions` | `ShapeField` |
| `orbit` | `canonical`, `same_orbit`, `tokenize`, `collapse`, `matches` | `String`, `Vec<OrbitToken>` |
| `magnitude` | `analyze`, `analyze_bytes`, `analyze_with` | `MagnitudeField` |
| `stress` | `analyze_bytes` | `StressField` |
| `flow` | `analyze`, `analyze_bytes(input, Signal)`, `analytic_signal` | `FlowField`, `AnalyticField` |
| `observation` | `analyze`, `analyze_with` | `ObservationField` |
| `echo` | `analyze`, `analyze_bytes`, `analyze_with`, `analyze_super` | `EchoField`, `Vec<SuperEcho>` |
| `relation` | `analyze`, `analyze_bytes` | `RelationField` |
| `holography`, `curvature`, `topology`, `geodesic`, `entanglement` | `analyze`, `analyze_bytes`, `analyze_over` | the [graph readings](../axes/relation/#graph-readings) |
| `gauge` | `canonicalize_bytes`, `alpha_equivalent` | `String`, `bool` |
| `context` | `analyze`, `analyze_with`, `relate`, `relate_bytes`, `record_period`, `fold_windows`, `agreement` | `ContextField`, `RelationContext` |
| `profile` | `fold_tokens`, `fold_profiles` | `Profile` |

## Backends

| Item | What it is |
|---|---|
| `Backend`, `BackendUsed` | which engine a scan runs on, and which answered ([engines](../matching/#engines)) |
| `scan_gpu`, `gpu_eligible`, `device_available` | the device scan and when it applies |
| `scan_dual_grain(&pattern, input) -> (Vec<Span>, GrainTiming)` | the byte and token grains as a pipeline |
| `notice::set_sink` | where the scheduler's notes go |
| `version()` | the crate version |
