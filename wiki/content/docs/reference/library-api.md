---
title: Library API
linkTitle: Library API
weight: 40
---

# Library API

Everything the `trex` binary does is a call into the `trex` crate. This page covers the
surface most callers need; the runnable version is
[`examples/scan_basics.rs`](https://github.com/Variably-Constant/trex/blob/main/examples/scan_basics.rs).

## Add the dependency

The package on crates.io is `trex-re`, and the crate it names in code is `trex`:

```toml
[dependencies]
trex-re = "0.1.0"
```

For a local checkout, use a path dependency: `trex = { package = "trex-re", path = "../trex" }`.
The published crate leaves out the prior the `compress` feature compiles in, so that feature
builds from a checkout.

## Parse and scan

`parse` compiles a pattern string once; `scan` matches it over any byte slice and returns the
leftmost, non-overlapping matches as byte spans. `captures` resolves the registers a binding
pattern bound over those spans.

```rust
use trex::{captures, parse, scan};

let pat = parse(r"<\W:t>.*</=t>").expect("valid pattern");
let input = b"<div>hi</div> <span>yo</span>";
let spans = scan(&pat, input);
for m in captures(&pat, input, &spans) {
    let text = std::str::from_utf8(&input[m.start..m.end]).unwrap();
    // A register is asked for by the name the pattern gave it. `captures()`
    // and `names()` are the same registers as parallel slices, sorted by name.
    let tag = m.group("t", input).map_or("", |b| std::str::from_utf8(b).unwrap());
    println!("[{}..{}] {text:?}  tag={tag:?}", m.start, m.end);
}
```

```text
[0..13] "<div>hi</div>"  tag="div"
[14..29] "<span>yo</span>"  tag="span"
```

| Item | Signature |
|---|---|
| `parse` | `fn parse(src: &str) -> Result<Pattern, ParseError>` |
| `scan` | `fn scan(pattern: &Pattern, input: &[u8]) -> Vec<Span>` |
| `Span` | `struct Span { start: u32, end: u32 }`; `start()`, `end()` and `range()` widen to `usize` |
| `captures` | `fn captures(pattern: &Pattern, input: &[u8], spans: &[Span]) -> Vec<Match>`; a pattern that binds nothing gets its spans back with empty captures. `captures_over`, `captures_with_shapes` and `captures_with_empty_loop` resolve the spans a pre-lexed, shaped or Perl-reading scan returned; `captures_with_lists`, `captures_over_with_lists` and `captures_with_shapes_and_lists` keep every binding a register made under a repetition |
| `Match` | `struct Match { start: usize, end: usize, .. }`. Registers are read through accessors, not as a field: `group(name, input) -> Option<&[u8]>`, `group_span(name) -> Option<Span>`, `group_at(i, input)`, `captures() -> &[Span]` and `names() -> &[String]` as parallel slices sorted by name, `span()`, and `extract::<N>(input)` for all of them at once. A register bound inside a bound pattern is named through it (`pair.k`); one bound under a repetition keeps every binding, `list(name) -> Option<&[Span]>` oldest first and `lists()` over all of them, while `group` holds its last |
| `Regs` | How a match holds its registers, and why they are behind an accessor: up to `INLINE_REGS` spans ride in the match, and beyond that they are shared by reference count. A heap allocation and its free costs 60.5 ns a match against a reference count's 10 |

## Rewrite

A `Template` is validated against the pattern's capture names, then `rewrite` renders each
match and splices the result:

```rust
use trex::{parse, rewrite, Template};

let pat = parse(r"\E:e").expect("valid pattern");
let tmpl = Template::parse("<${e:upper}>", &["e".to_string()]).expect("valid template");
let out = rewrite(&pat, &tmpl, b"reach me at bob@x.com today");
assert_eq!(out, b"reach me at <BOB@X.COM> today");
```

`rewrite_with` takes a closure in the template's place: each match arrives as a `Matched`,
whose `get` reads the reference a template writes inside `${...}` through the same
`Reference`, so a closure and a template read a match the same way, and the closure computes
what a template cannot say. `rewrite_n_with` stops after `n` matches.

```rust
use trex::{parse, rewrite_with};

let pat = parse(r"\E:e").expect("valid pattern");
let mut n = 0;
let out = rewrite_with(&pat, b"bob@x.com, amy@y.org", |m| {
    n += 1;
    format!("{n}:{}", m.get("e:domain").expect("a slice a template could render"))
});
assert_eq!(out, b"1:x.com, 2:y.org");
```

| Item | Signature |
|---|---|
| `rewrite_with` | `fn rewrite_with<F, R>(pattern: &Pattern, input: &[u8], replace: F) -> Vec<u8>` where `F: FnMut(&Matched<'_>) -> R`, `R: AsRef<[u8]>` |
| `rewrite_n_with` | the same with `n: usize` before the closure |
| `Matched` | `start()`, `end()`, `as_bytes()`, `text()`, `input()`, `inner() -> &Match`, `group(name) -> Option<&[u8]>`, `names()`, `value(name) -> Option<TypedValue>` for a register binding a single typed kind, and `get(reference) -> Result<String, TemplateError>` for `0`, a register, a position such as `1`, or a slice such as `e:domain`, `0:last4`, `ip:octet1-2`, `w:trim\|upper` |
| `TypedValue` | what `value` and `typed::value_of(kind, text)` answer, over the parsers a `:value` clause compares on, so a value and a predicate cannot disagree. `json(kind, ValueStyle, Clock) -> String` renders it; `ValueStyle` carries a `ValueSpelling` (`Exact`, `Natural`, `Tagged`) and a `DurationUnit` (`Nanoseconds`, `Milliseconds`, `Seconds`), both defaulting to the first |
| `Pattern::capture_kinds` | `Vec<(String, Option<TokenKind>)>` in `capture_names` order, `None` for a register binding a run, a repetition or an alternation of two or more, which have no one kind. `Matched::with_kinds` takes it; `Matched::new` supplies none and so reports no values |
| `Reference` | `parse(body, bound) -> Result<Reference, TemplateError>` over the pattern's `capture_names()`, `field() -> &Field` (`Whole` or `Register(name)`), `apply(text) -> String`, `read(&match, input) -> String` |

## Property axes

Each axis is a module with an `analyze*` function returning a field keyed by byte offset:

```rust
let field = trex::magnitude::analyze_bytes(b"retries = 3 ; max_bytes = 5000000000");
let peak = field.frames.iter().map(|f| f.magnitude).fold(0.0f32, f32::max);
assert!((peak - 9.70).abs() < 0.01);
```

| Module | Entry | Field |
|---|---|---|
| `magnitude` | `analyze_bytes(&[u8])` | `MagnitudeField` |
| `spectral` | `analyze(&[u8])` | `SpectralField` |
| `shape` | `analyze_bytes(&[u8])` | `ShapeField` |
| `stress` | `analyze_bytes(&[u8])` | `StressField` |
| `flow` | `analyze_bytes(&[u8], Signal)` | `FlowField` |
| `observation` | `analyze(&[u8])` | `ObservationField` |
| `seam` | `analyze(&[u8])` | `SeamField` |
| `orbit` | `canonical(&[u8], OrbitGroup)`, `tokenize`, `collapse`, `matches` | `String` / `Vec<OrbitToken>` |
| `echo` | `analyze_bytes(&[u8])`, `analyze_super(&[u8])` | `EchoField` / `Vec<SuperEcho>` |
| `context` | `analyze(&[u8])`, `relate_bytes(&[u8])`, `record_period(&[Token], &[u8])` | `ContextField` / `RelationContext` / `Option<u16>` |

The field types and their query methods are on each [axis reference page](../axes/).

The [context](../axes/context/) module also exposes the pieces the fields are built from:
`Window<P>` slides any `AxisProfile` monoid one value at a time at one combine a step,
`fold_windows` / `fold_windows_parallel` run it over a token stream at the token and unit
rungs, `relate` reads the relation-admitted folds, and `agreement` reads where each grain's
boundaries sit against the units.

## Custom token shapes

A `ShapeSet` holds token kinds you declare: a name and a bounded byte-pattern the lexer runs
alongside its built-in recognizers. The same set is passed to the parser, so `\{name}` in the
pattern resolves to the shape, and to the scan, so the lexer produces it:

```rust
use trex::{Precedence, ShapeSet, parser::parse_with_shapes, scan_with_shapes};

let mut shapes = ShapeSet::new();
shapes.declare("ticket = `[A-Z]{2,4}-\\d{1,4}`", Precedence::Before).expect("a bounded shape");
let pat = parse_with_shapes(r"\{ticket}", &shapes).expect("valid pattern");
let hits = scan_with_shapes(&pat, b"see AB-12 and XYZ-9 now", &shapes);
assert_eq!(hits.len(), 2);
```

`Precedence::Before` tries the shape ahead of the built-in recognizers and wins an overlap;
`Precedence::After` tries it only where no built-in matched.

The same set reads a pattern file: `declare_text` and `declare_file` take its `let`, `kind`,
`shape`, `shape-after`, `test` and `rule` lines, `declare_lines` also collects the members a
`PatternSet` built from the file holds, `run_tests` runs the `test` lines, and `rules()` and
`rule_of(name)` read the rules back as `Rule` values: the name, pattern and its source, the
message and fix templates as written, the `Severity` (`Error`, `Warning`, `Note`; `name()`
as SARIF's level, `github()` as an annotation's), the `files` globs, the `unless` patterns,
the record unit (`on_records()`, `record_unit()`) and the `meta` pairs (`tags()` splits
`meta.tags`). A rule is also a `let` under its name. A report template writes a finding's
`${rule}`, `${severity}`, `${message}` and `${fix}` from the `ReportRule` a `ReportAt`
carries.

Two substrate layers sit beside the axes. `supertokens(&[u8])` collapses token runs into
role-tagged units (`SuperToken`: span, bracket depth, grammar-free `Role`), the layer the
echo axis rhymes over. And `encoding::decode(Vec<u8>)` transcodes any BOM-declared UTF input
(or strictly-detected BOM-less UTF-16) to the UTF-8 the lexer reads - bytes it cannot vouch
for come back unchanged.

## Execution surfaces

The same matches are reachable through several backends; each returns what a plain `scan`
returns.

| Surface | Entry | Adds |
|---|---|---|
| Streaming | `scan_chunked`, `StreamScanner` | feed the input in chunks and recover the whole-input match set; an unbounded pattern commits only where no attempt still running reaches back over the cut, which `nfa::earliest_unsettled` reports from the walk's own thread list, and `commits_early` says where nothing can commit before the end; `StreamScanner::over_set` streams a `PatternSet` through one window, its most conservative member deciding what commits, and `drain_committed_with_members` / `finish_with_members` hand each span back with its member |
| Many patterns | `PatternSet` | patterns asked together over one lex: `matches`, `matched`, `matches_at` and `is_match` say which members match, `matches_with_spans` where each first does, `scan` every member's matches each with its member, `scan_matches` and `first_matches` the same resolved to `Match` values under the member's own registers (`lists` keeps every binding under a repetition); `from_text` / `from_file` build a set from a pattern file, a `let` a member under its name, a bare pattern line one under its line number, the file's `kind` and `shape` lines lexing every member, with `names`, `name(i)` and `shapes` reading them back; the members a byte route answers never reach the lexer, the rest share one lex |
| Dual-grain | `scan_dual_grain` | run the byte and token grains as a producer/consumer pipeline; returns `(Vec<Span>, GrainTiming)` |
| Device | `scan_gpu`, `scan_with_backend`, `GpuTokens` | map the all-starts scan onto a GPU for the eligible subset; `Backend` / `BackendUsed` report the choice (`BackendUsed::Split` for a scan whose anchors the cores and the device shared), `gpu_eligible` / `device_available` gate it, `gpu_auto_routes` is what the automatic router accepts, and `GpuTokens` holds a text's tokens on the device so many scans share one upload: `upload` holds the kinds, `upload_with_magnitudes` the magnitudes too, `upload_with_properties(input, group)` every property but the spectral reading, `upload_with_spectral(input, group)` that as well, and `from_tokens` holds kinds already lexed |
| Grammar | `Grammar`, `Node` | parse the token stream against named rules |

`version()` returns the crate version string.
