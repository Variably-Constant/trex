---
title: Python
linkTitle: Python
weight: 50
---

# Python

The module `trex`'s classes and functions, each linking the page its examples are on.

## Installing

The module is packaged as the `trex-re` wheel: one stable-ABI wheel for Python 3.11 and later,
built from the repository's `python/` directory with maturin and the `gpu` feature, so a host
with a CUDA device uses it and any other host runs the CPU engine. PyPI carries a wheel for
Windows (`win_amd64`), Linux (`manylinux_2_28_x86_64`) and macOS (`macosx_11_0_arm64`); on
FreeBSD pip builds one from the source distribution.

```console
$ pip install trex-re
```

A checkout builds the wheel with `maturin build --release --out dist` in `python/`.

## Inputs and offsets

Every call takes a `str` or `bytes`. A `str` is scanned as its UTF-8 bytes and reported in
characters, so `s[m.start:m.end] == m.text`; `bytes` are scanned and reported as bytes. Every
match also carries `byte_start` and `byte_end`, its UTF-8 byte offsets, and results come back in
the input's type. Scans release the interpreter lock while they run.

The scans, the rewrites, `redact` and `count_by` take the keywords `head=N`, `tail=N` or
`lines="A..B"`, one of them, counted in lines or in the records `unit=` names, and read only
that part of the input ([a window of a scan](../windows/#a-window-of-a-scan)).

## Pattern

| Call | Answer |
|---|---|
| `Pattern(source, lib=None)`, `parse(source, lib=None)` | the compiled pattern, read under the pattern files `lib` names, one path or a list; a bad pattern raises `ValueError` naming the byte ([pattern files](../pattern-files/)) |
| `source`, `capture_names()` | the source text; the register names it binds |
| `is_match(input, ...)` | `bool`: whether it matches anywhere |
| `find(input, ...)`, `captures(input, ...)` | the first `Match`, or `None` ([matches](../matching/#matches)) |
| `scan(input, ...)` | `list[Match]`: every leftmost, non-overlapping match |
| `find_iter(input, ...)`, `captures_iter(input, ...)` | the same matches one at a time, an iterator with a length |
| `rewrite(repl, input, ...)` | the input with every match replaced: `repl` a template, or a callable taking a `Match` and returning the replacement in the input's type ([templates](../rewriting/#templates), [computed replacements](../rewriting/#computed-replacements)) |
| `rewrite_n(repl, input, n, ...)`, `rewrite_first(repl, input, ...)` | the first `n`, or the first, replaced ([the first matches](../rewriting/#the-first-matches)) |
| `diff(repl, path, *, context=3)` | `str`: the unified diff a rewrite of the file at `path` would make, `""` where nothing matches; a binary file raises `ValueError` ([files](../rewriting/#files)) |
| `rewrite_file(repl, path)` | `int`: the replacements written into the file at `path`, each in the encoding the file is read in; a file with none is not written ([files](../rewriting/#files)) |
| `redact(input, *, keep=None, mask="*", ...)` | every match masked, the fields `keep` names left standing ([mask and keep](../redaction/#mask-and-keep)) |
| `split(input)`, `splitn(input, limit)` | `list`: the pieces between matches, at most `limit` with the last unsplit |
| `count_by(key, input, order="key", ...)` | `list[tuple[str, int]]`: the matches grouped by a report template, by key or, with `order="count"`, most frequent first ([count by a key](../aggregates/#count-by-a-key)) |
| `fields` | `list[dict]`: the fields the pattern reads, from a pattern file's `fields` line or one per register |
| `records(input)` | `list[dict]`: each record those fields read, its `lines` and `values` ([saving a build](../building-patterns/#saving-and-reusing-a-build)) |

A bad template raises `ValueError`, and a callable returning anything but the input's type
raises `TypeError`.

## Match

| Attribute | Meaning |
|---|---|
| `start`, `end`, `span()` | the span in the input's units |
| `byte_start`, `byte_end`, `byte_span()` | the span in UTF-8 bytes |
| `text` | the matched text, in the input's type |
| `captures` | `dict`: each register's text; a register bound under a repetition holds a list of every binding |
| `m[ref]`, `group(ref)` | a register (`"e"`), a nested one (`"pair.k"`), one binding by index (`"k[2]"`), every binding joined (`"ip[*]"`), the whole match (`"0"`), a register by position (`"1"`), or any of them through accessors (`"e:domain"`, `"0:last4"`); an unknown name raises `KeyError`, a bad accessor `ValueError` |
| `capture_span(name)`, `capture_byte_span(name)` | where the register bound |
| `value(name, unit="ns")` | the register's value parsed in its base unit, or `None`: a byte size in bytes, a duration in nanoseconds or `unit="ms"` or `"s"`, a timestamp as a UTC `datetime`, money and a percentage as `Decimal`, an address as an `int`, a version as its parts ([typed values](../matching/#typed-values)) |
| `explain(input)` | `dict` of `tokens`, `guards`, `readings` and `route`, the rung of the scan ladder that answered, for the input the match was found in ([explanations](../matching/#explanations)) |

## PatternSet and StreamScanner

| Call | Answer |
|---|---|
| `PatternSet(patterns)` | a set over patterns or their sources ([pattern sets](../matching/#pattern-sets)) |
| `PatternSet.from_file(path)` | the set a pattern file declares |
| `names`, `len(s)` | each member's name, or its index as text; the member count |
| `is_match(input)` | whether any member matches |
| `matches(input)`, `matches_at(input, at)` | the indices of the members that match, or match at or after `at` |
| `matches_with_spans(input)` | each matching index with its first `Match` |
| `scan(input)` | `list[tuple[int, Match]]`: every match of every member, in position order |
| `StreamScanner(pattern)`, `StreamScanner(set)` | a scan of input fed in chunks ([streams](../tools/#streams)) |
| `push(chunk)`, `finish()` | the `(start, end)` byte spans, or `(member, start, end)` over a set, that can no longer change, and the rest at the end |

## Functions

| Call | Answer |
|---|---|
| `head(n, input=None, *, path=None, unit="line")`, `tail(n, ...)`, `lines(range, ...)` | the first `n`, last `n`, or range `"A..B"` of an input or of the file at `path`, read from the end the part sits at ([first, last and a range](../windows/#first-last-and-a-range)) |
| `files(paths='.', *, hidden=False, no_ignore=False, binary=False, globs=None, types=None, types_not=None, texture=None, texture_not=None, sort=None, reverse=False)` | `list[str]`: the files a scan of `paths` reads, as `trex scan --files` lists them; an error the walk or a read meets raises `OSError` ([files and directory trees](../matching/#files-and-directory-trees)) |
| `read(path)` | `str`: the file's text decoded as a scan decodes it ([files and directory trees](../matching/#files-and-directory-trees)) |
| `texture(input=None, *, path=None)` | `tuple[str, int]`: what the text reads as mostly and, for a table, its period; `None` for text holding no region ([texture](../matching/#texture)) |
| `records(text, unit)` | `list[tuple[int, int]]`: the records of a unit `--record` names ([record units](../records/#record-units)) |
| `templates(text, rare_under=None, *, head=None, tail=None, lines=None)` | `list[dict]` of `count`, `lines`, `readable`, `pattern`, `rare` and `covered`, most lines first ([templates](../building-patterns/#templates)) |
| `infer(examples, anchored=False, against=[], ...)` | `str`: the pattern every example matches, missing each of `against` ([a pattern from examples](../building-patterns/#a-pattern-from-examples)) |
| `infer(examples, marked=[...] \| fields={...} \| marks_in_lines=True, unanchored=False, no_mint=False, mint_shapes=False, lib=None)` | `Built`: the pattern extracting the fields ([a pattern that extracts fields](../building-patterns/#a-pattern-that-extracts-fields)) |
| `escape(text)` | `text` as a pattern matching it literally |
| `set_now(secs)`, `set_tz_offset(secs)` | the instant `now` reads, `None` for the wall clock; the zone, in seconds east of UTC, a timestamp with no zone is read in ([typed value predicates](../pattern-syntax/#typed-value-predicates)) |
| `version()`, `__version__` | the engine's version |
| `device_available()` | whether a CUDA device is present |

## Built

| Attribute | Meaning |
|---|---|
| `pattern`, `str(built)` | the pattern, one branch per shape |
| `format` | the `--format` template writing every field |
| `file` | the pattern as a file `lib=` and `PatternSet.from_file` read |
| `declarations` | the `shape` lines `mint_shapes` declared |
| `suggestions` | `list[tuple[str, list[str]]]`: the library value classes holding every value of a field |
| `fields`, `shapes`, `rows`, `records` | the report as lists of dicts |
