---
title: Python
linkTitle: Python
weight: 50
---

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

A checkout builds the wheel with `maturin build --release --out dist` in `python/`. The wheel
ships type stubs beside `py.typed`: `trex/trex.pyi` for the compiled module, `trex/__init__.pyi`
re-exporting it as the package does, and `trex/axes.pyi`, so an editor and a type checker see
every signature.

## Inputs and offsets

Every call takes a `str` or `bytes`. A `str` is scanned as its UTF-8 bytes and reported in
characters, so `s[m.start:m.end] == m.text`; `bytes` are scanned and reported as bytes. Every
match also carries `byte_start` and `byte_end`, its UTF-8 byte offsets, and results come back in
the input's type. A report template's `${start}` and `${end}` count the same units, in
`Match.format`, `Finding.format`, a table's key and a rule's message. Scans release the
interpreter lock while they run.

The scans, the rewrites, `redact` and the tables take the keywords `head=N`, `tail=N` or
`lines="A..B"`, one of them, counted in lines or in the records `unit=` names, and read only
that part of the input ([a window of a scan](../windows/#a-window-of-a-scan)).

## Pattern

| Call | Answer |
|---|---|
| `Pattern(source, lib=None)`, `parse(source, lib=None)` | the compiled pattern, read under `lib`: a `Library`, a pattern file's path or a directory's, read as its `.trex` files, or a list of them; a bad pattern raises `ValueError` naming the byte ([pattern files](../pattern-files/)) |
| `source`, `capture_names()` | the source text; the register names it binds |
| `is_match(input, ...)` | `bool`: whether it matches anywhere |
| `find(input, ...)`, `captures(input, ...)` | the first `Match`, or `None` ([matches](../matching/#matches)) |
| `scan(input, ...)` | `list[Match]`: every leftmost, non-overlapping match |
| `find_iter(input, ...)`, `captures_iter(input, ...)` | the same matches one at a time, an iterator with a length |
| `grep(input=None, *, path=None, before=0, after=0, context=None, invert=False, whole_line=False, max_count=None, ..., hidden=False, no_ignore=False, binary=False, globs=None, backend="auto")` | an iterator of `Line`: each line a match starts on, or with `invert=True` each line no match touches, and the context lines around them, a count or a record unit's name on each side; `max_count` takes the first N matches, or N lines under `invert`; `path` is walked as `trex.files` walks it, one file read at a time ([context](../matching/#context), [selecting lines](../matching/#selecting-lines)) |
| `rewrite(repl, input, ...)` | the input with every match replaced: `repl` a template, or a callable taking a `Match` and returning the replacement in the input's type ([templates](../rewriting/#templates), [computed replacements](../rewriting/#computed-replacements)) |
| `rewrite_n(repl, input, n, ...)`, `rewrite_first(repl, input, ...)` | the first `n`, or the first, replaced ([the first matches](../rewriting/#the-first-matches)) |
| `diff(repl, path, *, context=3, max_count=None, backend="auto", head=None, tail=None, lines=None, unit="line", hidden=False, no_ignore=False, binary=False)` | `str`: the unified diff a rewrite of the files `path` names would make, each file's in walk order, `""` where nothing matches; `path` a file, a directory or a list of them; a binary file named alone raises `ValueError` ([files](../rewriting/#files)) |
| `rewrite_file(repl, path, *, review=None, show_skipped=False, explain=False, ...)` | `int`: the replacements written into the files, each in the encoding its file is read in; `review` is handed each change as a `Change` first, as `lib.fix` hands a fix; the rest as `diff` takes them ([files](../rewriting/#files)) |
| `redact(input, *, keep=None, mask="*", ...)` | every match masked, the fields `keep` names left unmasked ([mask and keep](../redaction/#mask-and-keep)) |
| `redact_file(path, *, keep=None, mask="*", review=None, show_skipped=False, explain=False, ...)`, `redact_diff(path, *, keep=None, mask="*", context=3, ...)` | `int`, `str`: the files redacted in place, or the diff that would make, a pseudonym naming each value alike in every file; the window and walk keywords as `diff` takes them ([files](../redaction/#files)) |
| `split(input)`, `splitn(input, limit)` | `list`: the pieces between matches, at most `limit` with the last unsplit |
| `count_by(key, input, order="key", ...)` | `list[tuple[str, int]]`: the matches grouped by a report template, by key or, with `order="count"`, most frequent first ([count by a key](../aggregates/#count-by-a-key)) |
| `group_by(key, text=None, *, path=None, order="key", limit=None, sum=None, avg=None, min=None, max=None, percentiles=None, percentile_method="nearest", duration_unit="ns", binary=False, ...)` | `list[Group]`: the matches of `text`, or of the files `path` names, grouped by a report template, with the aggregates asked for ([aggregates](../aggregates/#aggregates)); `path` is a file, a directory walked as `count-by` walks one, or a list of them; a file holding a NUL byte is passed over unless `binary=True`, and one named alone raises `ValueError` |
| `distinct(key, text=None, *, path=None, order="key", limit=None, binary=False, ...)` | `list[str]`: the distinct keys, as `trex uniq` prints them |
| `fields` | `list[dict]`: the fields the pattern reads, from a pattern file's `fields` line or one per register |
| `records(input)` | `list[dict]`: each record those fields read, its `lines` and `values` ([saving a build](../building-patterns/#saving-and-reusing-a-build)) |

A bad template raises `ValueError`, and a callable returning anything but the input's type
raises `TypeError`.

## Match

| Attribute | Meaning |
|---|---|
| `start`, `end`, `span()` | the span in the input's units |
| `byte_start`, `byte_end`, `byte_span()` | the span in UTF-8 bytes |
| `line`, `column` | where it starts, from 1, the column in characters |
| `path` | the file it is in, `None` for text |
| `pattern` | the name of the `PatternSet` member that made it, `None` for one pattern |
| `text` | the matched text, in the input's type |
| `captures` | `dict`: each register's text; a register bound under a repetition holds a list of every binding |
| `m[ref]`, `group(ref)` | a register (`"e"`), a nested one (`"pair.k"`), one binding by index (`"k[2]"`), every binding joined (`"ip[*]"`), the whole match (`"0"`), a register by position (`"1"`), or any of them through accessors (`"e:domain"`, `"0:last4"`); an unknown name raises `KeyError`, a bad accessor `ValueError` |
| `capture_span(name)`, `capture_byte_span(name)` | where the register bound |
| `value(name, unit="ns")` | the register's value parsed in its base unit, or `None`: a byte size in bytes, a duration in nanoseconds or `unit="ms"` or `"s"`, a timestamp as a UTC `datetime`, money and a percentage as `Decimal`, an address as an `int`, a version as its parts ([typed values](../matching/#typed-values)) |
| `kind(name)` | the kind of token the register binds (`"ip"`, `"timestamp"`, a declared shape's name), `None` where it binds more than one |
| `explain(input)` | `dict` of `tokens`, `guards`, `readings` and `route`, the rung of the scan ladder that answered, for the input the match was found in ([explanations](../matching/#explanations)) |
| `format(template, input)` | `str`: a report template rendered at the match, as `scan --format` renders one, for the input it was found in: registers, `${line}`, `${col}`, `${path}`, `${pattern}`, `${start}` and `${end}`, and the readings `${@axis}` names ([report templates](../matching/#report-templates)) |

## Line

| Attribute | Meaning |
|---|---|
| `path`, `number` | the file the line is in, `None` for text, and its number from 1, as the input numbers it |
| `text` | the line, in the input's type, without its line ending |
| `is_match` | whether the grep selected the line, as ripgrep's JSON marks a line `match`; `False` for a context line |
| `matches` | the `Match` objects starting on the line, each with its `path`; empty for a context line and for a line selected because no match touches it |

## Record

| Attribute | Meaning |
|---|---|
| `path`, `line` | the file the record is in, `None` for text, and the line it starts on, from 1 |
| `start`, `end`, `byte_start`, `byte_end` | its span in the input's units and in UTF-8 bytes |
| `text` | the record, in the input's type |
| `patterns` | the patterns it holds, each as it was given |
| `to_dict()` | the record as a plain `dict` of these fields |

## Group

| Attribute | Meaning |
|---|---|
| `key`, `count` | the key the group's matches rendered, and how many there are |
| `sum`, `avg`, `min`, `max` | `dict`: each register the keyword named, to its aggregate; `None` where the group's matches bound no value. A sum is an `int` or a `Decimal`, an average an exact `fractions.Fraction`, a least or greatest value the type `Match.value` gives, a duration in `duration_unit` ([averages and percentiles](../aggregates/#averages-and-percentiles)) |
| `percentiles` | `dict`: each register `percentiles` named, to a `dict` of each percent asked for to its value |
| `to_dict()` | the group as a plain `dict` of these fields |

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
| `group_by(key, ...)`, `distinct(key, ...)` | `Pattern.group_by` and `distinct` over every member, `${pattern}` naming the member that made each match ([keys by place and by member](../aggregates/#keys-by-place-and-by-member)) |
| `StreamScanner(pattern)`, `StreamScanner(set)` | a scan of input fed in chunks ([streams](../tools/#streams)) |
| `push(chunk)`, `finish()` | the `(start, end)` byte spans, or `(member, start, end)` over a set, that can no longer change, and the rest at the end |

## Library

| Call | Answer |
|---|---|
| `Library()`, `Library.load(*paths)`, `Library.shipped()` | an empty set of declarations; one declaring what the pattern files say, a directory read as its `.trex` files in path order; the shipped library ([libraries](../pattern-files/#libraries)) |
| `shape(name, bytepat, *, after=False, accepts=(), rejects=(), replace=False)`, `kind(name, pattern, ...)`, `let(name, pattern, ...)` | declares a shape, a `shape-after` with `after=True`, a kind or a sub-pattern, `accepts` and `rejects` its `test` line; a name already declared raises `ValueError` unless `replace=True` ([shapes](../pattern-files/#shapes)) |
| `declare(line)` | declares one line as a pattern file writes it, a rule's lines among them |
| `include(*paths)`, `remove(*items)`, `clear()` | declares what more files or directories say; takes names, included files or directories back out as one change, refusing with `ValueError` what an atom left would still read; takes everything out |
| `names`, `files`, `len(lib)`, `name in lib`, `iter(lib)` | the names declared, in order; the files included; how many names; whether one is declared; each `Atom` |
| `test()` | `list[AtomTest]`: each `test` line's outcome, in order ([tests](../pattern-files/#tests)) |
| `rules` | `list[Rule]`: the rules declared, in order |
| `check(text=None, *, path=None, max_count=None, head=None, tail=None, lines=None, unit="line", hidden=False, no_ignore=False, globs=None, binary=False)` | `list[Finding]`: the rules' findings over a text, or the files `path` names, walked as a scan walks them ([rules](../pattern-files/#rules)) |
| `fix(path, *, dry_run=False, context=3, review=None, show_skipped=False, ...)` | `int`: the fixes written, or with `dry_run=True` the `str` diff they make; `review` is a callable handed each `Change`, answering `True`, `False`, a replacement `str`, `"template"`, `"skip template"`, `"all"` or `"quit"` ([lint with rules](../../how-to/lint-with-rules/)) |
| `follow(path, *, max_count=None, keep_count=False, tail=None, lines=None, unit="line", ...)` | `Follow`: an iterator yielding each finding once nothing arriving later can change it, as the files grow; a truncated or replaced file starts its `max_count` again unless `keep_count=True` ([following a file](../windows/#following-a-file)) |

| Class | Attributes |
|---|---|
| `Atom` | `name`, `form` (`"shape"`, `"shape-after"`, `"kind"`, `"let"` or `"rule"`), `definition`, `source` (`"library"`, the pattern file's path, or `"shipped"`), `description` |
| `AtomTest` | `name`, `passed`, `accepts`, `rejects`, `failures`, `file` |
| `Rule` | `name`, `severity`, `message`, `pattern`, `fix`, `files`, `meta`, `tags`, `file`, `line` |
| `Finding` | `rule`, `severity`, `message`, `path`, `line`, `column`, `end_line`, `end_column`, `start` and `end` in the input's units, `byte_start`, `byte_end`, `text`, `fix`, `fix_span`, `captures`, `definition` (its `Rule`); `github()` the GitHub annotation, `format(template)` a report template rendered at it |
| `Change` | `path`, `start`, `end`, `text`, `replacement`, `diff`, `template`, `later`, `index`, `total`, and `explanation`, `Match.explain`'s dict where `explain=True` asked for it and `None` otherwise: one fix, rewrite or masking put to a review |
| `Token` | `kind`, `start`, `end`, `text`, `value` ([tokens](../records/#tokens)) |

Each has `to_dict()`. A fix skipped for overlapping an earlier one, the changes a template answer
passed over, a review that quits and a followed file truncated, replaced or removed are reported
through `warnings` as `trex.TrexWarning`, a `UserWarning`.

## Grammars, encoders, indexes and filters

| Call | Answer |
|---|---|
| `Grammar(source, *, start=None)` | a token grammar; `start` and `rules` name its rules ([grammars](../tools/#grammars)) |
| `parse(text=None, *, path=None)`, `accepts(...)`, `count(...)`, `best(...)`, `probability(...)` | the `ParseNode` tree or `None`; whether it parses in full; its derivations; the most probable one's probability and all of them together |
| `segment(text, dictionary)`, `count_segmentations(...)`, `best_segmentation_probability(...)` | `list[Tiling]`: each tiling of a run-together string the grammar accepts, most probable first, with its `words`, `probability` and `parses`; every parse of every tiling counted; the first tiling's probability ([segmentation](../tools/#segmentation)) |
| `Bpe.train(text=None, *, path=None, merges=1000, max_bytes=None)`, `Bpe(model)`, `Bpe.load(path)` | an encoder learned from a corpus, read from model text, or read from a model file ([byte-pair encoders](../tools/#byte-pair-encoders)) |
| `encode(text=None, *, path=None)`, `save(path)`, `merges`, `model`, `len(bpe)` | `list[str]`: the subwords; the model written; how many merges and their text |
| `Index.build(path, *, hidden=False, no_ignore=False, globs=None)`, `Index.load(path)` | writes the tree's `.trex-index` and returns it; reads one, raising `FileNotFoundError` where none is ([indexes](../tools/#indexes)) |
| `candidates(pattern, ...)`, `root`, `path`, `files` | `list[str]`: the files a scan of `pattern` must read; the tree, the index file and how many files it covers |
| `Prefilter(text=None, *, path=None, kind="bloom")` | a presence filter over a corpus's n-grams, `"bloom"`, `"cuckoo"` or `"xor"` ([prefilters](../tools/#prefilters)) |
| `might_contain(literal)`, `test(*literals)`, `verify()` | `False` means absent; `list[LiteralTest]`, each answer beside an exact search; `list[FilterCheck]`, each filter's contract probed over the corpus |

## Axes

The submodule `trex.axes` reads the [axes](../axes/), one function an axis, each over the text
positionally or the file at `path=` and giving one report with the fields of PowerShell's
`Measure-Trex` report in snake case. Offsets are in the input's units, a point inside a
character reported at the character's offset, and a text taken from the input comes back in its
type. `detail=True` adds every frame; every report and frame has `to_dict()`.

| Function | Options | Report |
|---|---|---|
| `magnitude(...)` | `lib=`, `jump_threshold=`, `outlier_sigma=` | `MagnitudeReport`: `tokens`, `total_energy`, `peak`, `jumps`, `outliers`, `frames` |
| `stress(...)` | `lib=`, `peak_min_depth=`, `fracture_min_depth=` | `StressReport`: `tokens`, `max_depth`, `peak_load`, `peaks`, `fractures`, `frames` |
| `flow(...)` | `lib=`, `signal="magnitude"`, `"stress"` or `"length"`, `grain="token"` or `"super"`, `analytic=True`, `window=`, `steady_band=` | `FlowReport`: `tokens`, `grain`, `units`, `signal`, `peak_momentum`, `reversals`, `latency`, `analytic`, `frames` |
| `observation(...)` | `lib=` at the token and super grains, `grain="byte"`, `"token"` or `"super"`, `contested_threshold=`, `contested_min_gap=` | `ObservationReport`: `grain`, `units`, `peak`, `contested`, `frames` |
| `gravity(...)` | `lib=` at the token and super grains, `grain="token"`, `"byte"` or `"super"`, `top=8` | `GravityReport`: `grain`, `units`, `types`, `strained`, `weakest`, `classes`, `frames` |
| `context(...)` | `lib=`, `fold="window"`, `"unit"`, `"units"`, `"enclosing"`, `"echo"`, `"regime"`, `"phase"` or `"key"`, `token_window=`, `unit_window=` | `ContextReport`: `fold`, `tokens`, `units`, `period`, `live_periods`, `alignment`, `agreement`, `frames` |
| `relation(...)` | `lib=`, `canonical=True` | `RelationReport`: the graph's counts, holonomy, holography, curvature, topology, geodesic and entanglement readings, `canonical`, `edges`, `chords`, `frames` |
| `spectral(...)` | `classify=True`, `cp_threshold=`, `cp_floor=`, `cp_min_gap=` | `SpectralReport`: `hop`, `change_points`, `entropy_minimum`, `entropy_mean`, `entropy_maximum`, `periods`, `timeline`, `regions`, `frames` |
| `seam(...)` | `order=`, `passes=`, `english=True`, `cut_threshold=` | `SeamReport`: `order`, `segments`, `frames` |
| `echo(...)` | `lib=`, `group="identity"`, `structure=True`, `max_period_cv=` | `EchoReport`: `group`, `tokens`, `keyed`, `distinct`, `novel`, `echoed`, `novelty`, `echo_rate`, `echoes`, `structures`, `frames` |
| `orbit(...)` | `lib=`, `group="shape"`, `same_as=` | `OrbitReport`: `group`, `tokens`, `forms`, `orbits`, `classes`, `matches`, `segments`, `frames` |
| `shape(...)` | `lib=`, `group=`, `template_strength=` | `ShapeReport`: `group`, `tokens`, `dominant_period`, `dominant_strength`, `regions`, `change_points`, `frames` |

Every report also carries `path` and `length`. Spectral and seam read bytes and take no `lib=`,
and observation and gravity take none at the byte grain. `detail=True` adds the frames, and for
context each [supertoken](../../explanation/architecture/#supertokens)'s `agreement` too. A context frame holds an object an axis, `magnitude`
through `flow`, each `None` where the context folds no token. Two fields are renamed from
PowerShell's because their names are Python keywords: the `class` of a shape frame, a gravity
frame and a gravity class is `class_`, and a relation edge's or chord's first token `from_`.

## Functions

| Call | Answer |
|---|---|
| `head(n, input=None, *, path=None, unit="line")`, `tail(n, ...)`, `lines(range, ...)` | the first `n`, last `n`, or range `"A..B"` of an input or of the file at `path`, read from the end nearest the part ([first, last and a range](../windows/#first-last-and-a-range)) |
| `follow(pattern, *paths, from_end=True, max_count=None, keep_count=False, lib=None)` | an iterator of `Match`: each match of `pattern`, a `Pattern` or its source read under `lib`, as the files grow, from each file's end or with `from_end=False` its start, its file in `path`; a file truncated, replaced or removed is a `TrexWarning` and is read again from its start ([following a file](../windows/#following-a-file)) |
| `follow_lines(*paths, tail=None, lines=None, unit="line")` | an iterator of `Line`: each line the files gain, after each file's last `tail` lines or its open `lines` range, one of them given, as `trex tail -f` prints them ([following a file](../windows/#following-a-file)) |
| `follow_rewrite(pattern, repl, path, *, tail=None, lines=None, unit="line", max_count=None, keep_count=False, lib=None)`, `follow_redact(pattern, path, *, keep=None, mask="*", tail=None, lines=None, unit="line", lib=None)` | an iterator of `str`: each line of the file rewritten or redacted, without its ending, the window or the whole file first, then what it gains, as `rewrite --follow` and `redact --follow` write it ([following a file](../windows/#following-a-file)) |
| `scan_stats()` | `ScanStats`: what the last scan or grep on this thread read and found, every field `--stats` prints, or `None` before any ([statistics](../matching/#statistics)) |
| `query(patterns, input=None, *, path=None, require="any", at_least=None, exclude=(), unit="line", record_start=None, record_span=None, max_count=None, lib=None)` | `list[Record]`: the records holding any, all or none of the patterns, or `at_least` of them, and none of `exclude`, a record a line, another unit, or made by `record_start` or `record_span` ([record queries](../records/#record-queries)) |
| `set_date_order(order)` | `"dmy"` or `"mdy"`: how a slash date both readings hold is read, as `--date-order` sets it |
| `files(paths='.', *, hidden=False, no_ignore=False, binary=False, globs=None, types=None, types_not=None, texture=None, texture_not=None, sort=None, reverse=False)` | `list[str]`: the files a scan of `paths` reads, as `trex scan --files` lists them; an error the walk or a read meets raises `OSError` ([files and directory trees](../matching/#files-and-directory-trees)) |
| `read(path)` | `str`: the file's text decoded as a scan decodes it ([files and directory trees](../matching/#files-and-directory-trees)) |
| `texture(input=None, *, path=None)` | `tuple[str, int]`: what the text reads as mostly and, for a table, its period; `None` for text holding no region ([texture](../matching/#texture)) |
| `records(text, unit)` | `list[tuple[int, int]]`: the records of a unit `--record` names ([record units](../records/#record-units)) |
| `templates(text=None, cut=None, *, path=None, against=None, novel=False, rare=False, unit="line", record_start=None, record_span=None, head=None, tail=None, lines=None, hidden=False, no_ignore=False, binary=False, lib=None)` | `list[dict]` of `count`, `records`, `readable`, `pattern`, `rare`, `novel` (`None` without `against`) and `covered`, most records first, of a text or the files `path` names read as one stream; `against` is the other input's text, or a path or a list of them; `cut` is `--cut` ([templates](../building-patterns/#templates), [against another input](../building-patterns/#against-another-input)) |
| `tokens(text=None, *, path=None, whitespace=False, lib=None, binary=False)` | `list[Token]`: the tokens the lexer reads, whitespace left out unless asked for ([tokens](../records/#tokens)) |
| `sarif(findings, *, lib=None)`, `findings_json(findings)` | `str`: one SARIF 2.1.0 document, listing `lib`'s rules; one JSON array, as `scan --rules --json` writes ([lint with rules](../../how-to/lint-with-rules/)) |
| `infer(examples, anchored=False, against=[], ...)` | `str`: the pattern every example matches, missing each of `against` ([a pattern from examples](../building-patterns/#a-pattern-from-examples)) |
| `infer(examples, marked=[...] \| fields={...} \| marks_in_lines=True, unanchored=False, no_mint=False, mint_shapes=False, lib=None)` | `Built`: the pattern extracting the fields ([a pattern that extracts fields](../building-patterns/#a-pattern-that-extracts-fields)) |
| `escape(text)` | `text` as a pattern matching it literally |
| `set_now(secs)`, `set_tz_offset(secs)` | the instant `now` reads, `None` for the wall clock; the zone offset, in seconds east of UTC, given to a timestamp with no zone ([typed value predicates](../pattern-syntax/#typed-value-predicates)) |
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
