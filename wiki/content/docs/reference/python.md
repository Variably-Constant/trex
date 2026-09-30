---
title: Python
linkTitle: Python
weight: 50
---

# Python

The engine is a Python module, `trex`, packaged as the `trex-re` wheel: one stable-ABI wheel
for Python 3.11 and later, built from the repository's `python/` directory with maturin and
the `gpu` feature, so a host with a CUDA device uses it the way the binary does and any other
host runs the CPU engine.

PyPI carries a wheel for Windows (`win_amd64`), Linux (`manylinux_2_28_x86_64`) and macOS
(`macosx_11_0_arm64`), each installed and its suite run when it is built. PyPI takes no
FreeBSD wheel, so pip builds one there from the source distribution; a wheel built on FreeBSD
15.0 passes the same suite. `trex-re` installs from PyPI:

```console
$ pip install trex-re
```

A checkout builds the wheel with `maturin build --release --out dist` in `python/`.

## Inputs and offsets

Every call takes a `str` or `bytes`. A `str` is scanned as its UTF-8 bytes and reported in
characters, so `s[m.start:m.end] == m.text`; `bytes` are scanned and reported as bytes. Every
match also carries `byte_start` and `byte_end`, its UTF-8 byte offsets, whatever the input
was, and results come back in the input's type. Scans release the interpreter lock while they
run.

```python
>>> import trex
>>> p = trex.Pattern(r"<\W:t>.*</=t>")
>>> m = p.find("say <div>hi</div> now")
>>> (m.start, m.end, m.text, m["t"], m.capture_span("t"))
(4, 17, '<div>hi</div>', 'div', (5, 8))
>>> [(m.start, m.end, m.byte_start, m.byte_end) for m in trex.Pattern(r"\N").scan("héllo 42")]
[(6, 8, 7, 9)]
```

## Pattern

| Call | Answer |
|---|---|
| `Pattern(source)`, `parse(source)` | the compiled pattern; a bad one raises `ValueError` naming the byte |
| `Pattern(source, lib=path)` | the pattern read under a pattern file's declarations, or a list of files, as `--lib` supplies them on the command line: `\{name}` resolves to its `let`, `kind` and `shape` lines, and a declared shape or kind decides the token boundaries every scan of the pattern is made on |
| `p.source`, `p.capture_names()` | the source text; the register names it binds |
| `p.is_match(input)` | whether it matches anywhere |
| `p.find(input)`, `p.captures(input)` | the first match, or `None` |
| `p.scan(input)` | every leftmost, non-overlapping match, as a list |
| `p.find_iter(input)`, `p.captures_iter(input)` | the same matches, one at a time |
| `p.rewrite(repl, input)` | every match replaced by `repl`: a template rendered at each, or a callable handed each match |
| `p.rewrite_n(repl, input, n)`, `p.rewrite_first(repl, input)` | the first `n`, or the first, replaced |
| `p.redact(input, keep=None, mask="*")` | every match masked as the `redact` command masks one: each character by `mask`, or a token per masked run where `mask` is longer, the fields `keep` names (`"card:last4, ip:octet1-2"`) left where they stand |
| `p.split(input)`, `p.splitn(input, limit)` | the pieces between matches; `splitn` yields at most `limit`, the last unsplit |
| `head=N`, `tail=N`, `lines="A..B"`, `unit="line"` | keywords of the scans, the rewrites and `redact`, reading only that part of the input; see [parts of an input](#parts-of-an-input) |

Templates are the `rewrite` command's: `${name}`, `${0}`, `${name:upper}`, `${ip:octet1-2}`,
`${url:host}`, `${ts:year}`, chained with `|`; a bad template raises `ValueError`. A callable
takes the `Match` and returns the replacement in the input's type, `str` for `str` and `bytes`
for `bytes`, so what a template cannot say is computed from the match; anything else returned,
or a `repl` that is neither, raises `TypeError`.

```python
>>> trex.Pattern(r"\E:e").rewrite("[${e:domain}]", "mail bob@x.com and amy@y.org now")
'mail [x.com] and [y.org] now'
>>> trex.Pattern(r"\E:e").rewrite(lambda m: m["e:user"].upper(), "mail bob@x.com and amy@y.org now")
'mail BOB and AMY now'
>>> trex.Pattern(r"\P").split("a, b; c")
['a', ' b', ' c']
```

With `defs.trex` holding the two lines `let rhs = \N | \Q` and
`kind assign = \W "=" \{rhs}`, a pattern read under it sees what it declares, and a kind from
a pattern fuses the tokens its pattern covers into one token the scan reports whole:

```python
>>> p = trex.Pattern(r"\{assign}", lib="defs.trex")
>>> [m.text for m in p.scan('let x = 1; name = "bob"')]
['x = 1', 'name = "bob"']
>>> p.rewrite("<${0}>", 'let x = 1; name = "bob"')
'let <x = 1>; <name = "bob">'
```

## Match

| Attribute | Meaning |
|---|---|
| `start`, `end`, `span()` | the span in the input's units |
| `byte_start`, `byte_end`, `byte_span()` | the span in UTF-8 bytes |
| `text` | the matched text, in the input's type |
| `captures`, `m[name]`, `group(name)` | what each register bound; an unknown name raises `KeyError`. A register nested inside a bound pattern is named through it (`m["pair.k"]`); one bound under a repetition holds every binding as a list in `captures`, `m[name]` its last, and `m["pair[0].k"]` or `m["k[2]"]` one by index, `IndexError` past the last |
| `m["e:domain"]`, `m["0:last4"]`, `m["1"]`, `group("ip:octet1-2")` | a template reference read as `${...}` renders it: `0` the whole match, `1` the first register the pattern binds, and any accessor chain after the colon, in the input's type; a bad accessor or position raises `ValueError` |
| `capture_span(name)`, `capture_byte_span(name)` | where it bound it |
| `value(name)`, `value(name, unit="s")` | what the register bound, parsed, in its base unit, or `None` where it binds no single typed kind or its text does not parse as one; an unknown name raises `KeyError`. A byte size in bytes, a duration in nanoseconds with `unit="ms"` or `unit="s"` shifting it exactly, a timestamp as a timezone-aware `datetime` in UTC, money and a percentage as `Decimal`, an address as an `int` of its 32 or 128 bits, a version as its parts and its pre-release identifiers. A register bound under a repetition answers a list, one value per binding |

```python
>>> m = trex.Pattern(r"\W:w \I:ip").find("from 10.1.2.3")
>>> (m["ip"], m["ip:octet1-2"], m["0:last1"], m["1"], m["2:octet4"])
('10.1.2.3', '10.1', '3', 'from', '3')
```

## PatternSet and StreamScanner

`PatternSet(patterns)` takes patterns or their sources and asks them together over one lex:
`is_match(input)`, `matches(input)` (the indices that match, in order), `matches_at(input, at)`
(at or after a position in the input's units), `matches_with_spans(input)` (each matching
index with its first match, its registers resolved) and `scan(input)` (every match of every
member, each with its member's index, in position order). `PatternSet.from_file(path)` builds
the set a pattern file declares: a `let NAME = PATTERN` line is a member under its name, a bare
pattern line a member under its line number, and the file's `kind` and `shape` lines serve
every member; `names` is each member's name, the index as text for a set built from patterns.

`StreamScanner(pattern)` scans input fed in chunks: `push(chunk)` returns the `(start, end)`
byte spans that can no longer change, and `finish()` the rest, so the union over a stream
equals `scan` over the whole input. `StreamScanner(set)` streams a set through one window,
its most conservative member deciding what commits, and returns `(member, start, end)`.

```python
>>> s = trex.PatternSet([r"\E", r"\I", r"\U"])
>>> s.matches("from 10.0.0.1 to bob@x.com")
[0, 1]
>>> [(i, m.text) for i, m in s.scan("10.0.0.1 and 10.0.0.2 mail bob@x.com")]
[(1, '10.0.0.1'), (1, '10.0.0.2'), (0, 'bob@x.com')]
>>> st = trex.StreamScanner(trex.Pattern(r"\N"))
>>> st.push(b"one 1 two 2\nthree 3 fo") + st.push(b"ur 4\n") + st.finish()
[(4, 5), (10, 11), (18, 19), (25, 26)]
```

With `rules.trex` holding the three lines `let host = \I:addr`, `let mail = \E:e` and
`\N{>=100}`:

```python
>>> f = trex.PatternSet.from_file("rules.trex")
>>> f.names
['host', 'mail', '3']
>>> [(f.names[i], m.text, m.captures) for i, m in f.scan("from 10.0.0.1 at 500 to bob@x.com")]
[('host', '10.0.0.1', {'addr': '10.0.0.1'}), ('3', '500', {}), ('mail', 'bob@x.com', {'e': 'bob@x.com'})]
>>> st = trex.StreamScanner(f)
>>> st.push(b"from 10.0.0.1\n") + st.push(b"to bob@x.com\n") + st.finish()
[(0, 5, 13), (1, 17, 26)]
```

## Module functions

| Call | Effect |
|---|---|
| `set_now(secs)` | the instant `now` reads in clock clauses such as `\T{age<24h}`, as seconds since the epoch; `None` reads the wall clock at each scan |
| `set_tz_offset(secs)` | the zone a timestamp written with no zone is read in, as seconds east of UTC |
| `escape(text)` | `text` as a pattern that matches it literally |
| `head(n, input=None, *, path=None, unit="line")`, `tail(n, ...)`, `lines(range, ...)` | the first `n` lines of `input`, a `str` or `bytes`, in its type, its last `n`, or lines `"A..B"`, `"A.."`, `"..B"` or `"A"`; or of the file at `path=`, read from the end the part sits at and no further, as a `str`. `unit` counts paragraphs or another record unit instead of lines |
| `templates(text, rare_under=None, *, head=None, tail=None, lines=None)` | the line templates, most lines first: each a dict of `count`, `lines`, `readable`, `pattern`, `rare` and `covered`. `rare_under` is the cut as `--cut` takes it (`"5"`, `"1%"`); absent, a template is rare when it covers fewer lines than the mean template, which is read from the input. `head`, `tail` or `lines` mines that part of the text alone |
| `infer(examples, anchored=False, against=[], marked=[], fields=None, marks_in_lines=False, unanchored=False)` | the pattern inferred from a list of `str` or `bytes` examples, as source; examples with nothing in common raise `ValueError`. `against` holds examples the pattern must miss, and is what decides whether a position reports a value range or its bare kind; a counter-example nothing separates raises `ValueError` naming it. With `marked`, `fields` or `marks_in_lines`, a `Built` instead: the pattern extracting those fields from every shape of the examples, with its report (see [Reading a log](#reading-a-log)) |
| `records(text, unit)` | the spans of a record unit (`"line"`, `"paragraph"`, `"block"`, or a pattern), as `(start, end)` in the input's units |
| `version()`, `__version__` | the engine's version |
| `device_available()` | whether a CUDA device is present |

## Parts of an input

`head`, `tail` and `lines` answer an input's first lines, its last, or a range, as the
command's `head`, `tail` and `lines` print them. The same selections are keywords of every
scan, rewrite and `redact`: `head=N`, `tail=N` or `lines="A..B"`, one of them, counted in lines
or in the records `unit=` names. A scan reads only that part, a match counting only where it
lies wholly inside, and each match stands at the input's own offsets. A rewrite or a redaction
gives back that part alone, rewritten, so nothing outside it comes back unchanged.

```python
>>> five = "one 1\ntwo 2\nthree 3\nfour 4\nfive 5\n"
>>> trex.tail(2, five)
'four 4\nfive 5\n'
>>> trex.lines("2..3", five)
'two 2\nthree 3\n'
>>> p = trex.Pattern(r"\N")
>>> [(m.text, m.start) for m in p.scan(five, tail=2)]
[('4', 25), ('5', 32)]
>>> p.rewrite("<${0}>", five, tail=1)
'five <5>\n'
>>> p.redact(five, head=1)
'one *\n'
```

With `path=`, `head`, `tail` and `lines` read a file from the end the part sits at: a head
stops at its last newline and a tail reads backward from the file's end, so `trex.tail(20,
path="app.log")` reads the last twenty lines of a log without reading the rest.

## Reading a log

The module answers the questions the command line answers, so a script need not shell out.

```python
>>> log = "GET /a 200 12ms\nGET /b 404 30ms\nPOST /c 200 9ms\n"
>>> [(t["count"], t["readable"], t["rare"]) for t in trex.templates(log)]
[(3, '<word> /<word> <number> <duration>', False)]
>>> trex.infer([b"GET /a 200", b"GET /b 404"])
'"GET" "/" \\W \\N'
>>> trex.records(log, "line")
[(0, 15), (16, 31), (32, 47)]
```

With a field given, `infer` builds the pattern that extracts it from every shape of the
examples, as `trex infer --mark` does, and returns a `Built`. `marked=` holds lines with each
value written `{name:text}`, `fields=` maps a field's name to a value it takes or a list of
them, and `marks_in_lines=True` reads marks in the examples themselves; `unanchored=True`
lets a match start and end inside a longer line, and `no_mint=True` spells a word field as `\W`
where it would be the byte shape its values share, `` `KB[0-9]{7}` `` for the KB numbers below.
`suggestions` holds each field every value of which a library value class holds, as
`(field, [class, ...])`, where no `against` line called for one; one that does prints the class
in the pattern. `mint_shapes=True` declares that shape as a named one instead: `declarations` holds each
`shape` line, `file` writes them above the pattern, and the pattern reads under `lib=` a file
holding them. `lib=` on `infer` lexes the examples under a file's shapes, as `Pattern`'s does,
so a declared shape is one token the built pattern names. A `Built`'s string form is its
pattern.
`fields`, `shapes`, `rows` and `records` are the report as lists of dicts, `format` the
`--format` template writing every field, and `file` the pattern as a file
`PatternSet.from_file` reads. A marked entry may be a template of several lines, and a field
written `{name*:text}` begins a record that the lines after it join, as `ConvertFrom-String`
reads one.

```python
>>> titles = [
...     "2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB5031354)",
...     "2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)",
...     "Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)",
... ]
>>> built = trex.infer(titles, marked=["{month:2023-10} Cumulative Update for Windows {os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})"])
>>> [r["values"] for r in built.rows]
[{'month': '2023-10', 'os': '11', 'version': '22H2', 'kb': 'KB5031354'}, {'month': '2020-01', 'os': '7', 'version': None, 'kb': 'KB4534310'}, {'month': None, 'os': '10', 'version': '1607', 'kb': 'KB4103720'}]
>>> built.format
'${month}\\t${os}\\t${version}\\t${kb}'
>>> [r["values"]["kb"] for r in trex.infer(titles, fields={"kb": "KB5031354"}).rows]
['KB5031354', 'KB4534310', 'KB4103720']
```

A field marked more than once in one line is a list: its field dict has `list` true, its
value in `rows` and `records` is a list of str, and its template, `${ip[*]}`, writes every
value joined with a comma. `Match` reads the same reference. A mark inside a mark is a field
named after its `parent`, `Line.n`, and a value in `rows` and `records` gives the outer field
as a dict of its `text` and the fields inside it by their own names:
`{"Line": {"text": "5 of 9", "n": "5", "m": "9"}}`. A template line marking a starred field
more than once repeats its record: each field of the record has `repeats` true, its value in a
row is a list of every record's, and `records` holds one dict per record, a field outside the
records carried by every record of its line. A starred mark spanning the lines of a template,
`{Person*:Name: {Name:Phoebe Cat}\nPhone: {Phone:425-123-6789}}`, is a record of the lines it
spans: its record's value is a dict of the fields inside it and a `text` of each line joined
with a newline.

```python
>>> hops = trex.infer(["from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok", "from 10.0.0.3 ok"],
...                   marked=["from {ip:10.0.0.1} -> {ip:10.0.0.2} ok"])
>>> [r["values"] for r in hops.rows]
[{'ip': ['10.0.0.1', '10.0.0.2']}, {'ip': ['10.0.0.7', '10.0.0.8', '10.0.0.9']}, {'ip': ['10.0.0.3']}]
>>> trex.Pattern(hops.pattern).find("from 1.1.1.1 -> 2.2.2.2 ok")["ip[*]"]
'1.1.1.1,2.2.2.2'
```

`Pattern.count_by(key, text, order="key")` returns the rows `count-by` prints, and
`order="count"` the rows `top` prints. Every row is returned: a truncated table and a
complete one read alike, so the cut is the caller's to make.

```python
>>> p = trex.Pattern(r"^ \W:verb \P \W \N:code")
>>> p.count_by("${verb}", log)
[('GET', 2), ('POST', 1)]
```

`Match.explain(input)` says why a match matched: `tokens` the significant tokens it spans,
`guards` what each guarded token passed, `readings` each axis the pattern reads at each
token, and `route` the rung of the scan ladder that answered. The input comes back because
holding it on every match would copy the whole of it per scan, for a report most callers
never ask for.

```python
>>> m = p.find(log)
>>> m.explain(log)["tokens"]
[('word', 'GET'), ('punct', '/'), ('word', 'a'), ('number', '200')]
>>> m.value("code")
200
```

The pattern language is the [pattern syntax](../pattern-syntax/) page's; the Rust surface the
module mirrors is the [library API](../library-api/) page's.
