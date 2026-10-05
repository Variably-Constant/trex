# trex for Python

The [trex](https://github.com/Variably-Constant/trex) engine as a Python module: a
regex-shaped pattern language whose alphabet is typed tokens (numbers, words, quoted
strings, IPs, URLs, emails, timestamps, balanced bracket groups, and the rest), with
long-distance binding, typed value predicates, and no catastrophic backtracking.

```console
$ pip install trex-re
```

```python
>>> import trex
>>> p = trex.Pattern(r"<\W:t>.*</=t>")
>>> m = p.find("say <div>hi</div> now")
>>> (m.start, m.end, m.text, m["t"])
(4, 17, '<div>hi</div>', 'div')
>>> [m.text for m in trex.Pattern(r"\N{>=100}").find_iter("5 50 500 5000")]
['500', '5000']
>>> trex.Pattern(r"\E:e").rewrite("[${e:domain}]", "mail bob@x.com now")
'mail [x.com] now'
>>> trex.Pattern(r"\E:e").rewrite(lambda m: m["e:user"].upper(), "mail bob@x.com now")
'mail BOB now'
```

## Inputs and offsets

Every scan takes a `str` or `bytes`. A `str` is scanned as its UTF-8 bytes and
reported in characters, so `s[m.start:m.end] == m.text`; `bytes` are scanned and
reported as bytes. A match also carries `byte_start` and `byte_end`, its UTF-8 byte
offsets, whatever the input was. Results come back in the input's type: `str` for
`str`, `bytes` for `bytes`.

## API

| Name | What it does |
|---|---|
| `Pattern(source)`, `parse(source)` | compile a pattern; a bad one raises `ValueError` with the byte position |
| `Pattern.is_match(input)` | whether the pattern matches anywhere |
| `Pattern.find(input)`, `Pattern.captures(input)` | the first match, or `None` |
| `Pattern.scan(input)` | every leftmost, non-overlapping match, as a list |
| `Pattern.find_iter(input)`, `Pattern.captures_iter(input)` | the same matches, one at a time |
| `Pattern.rewrite(repl, input)`, `rewrite_n(repl, input, n)`, `rewrite_first(repl, input)` | every, the first `n`, or the first match replaced by `repl`: a template (`${name}`, `${0}`, `${name:upper}`, `${ip:octet1-2}`, ...) rendered at each match, or a callable handed each `Match` and returning its replacement in the input's type |
| `Pattern.split(input)`, `Pattern.splitn(input, limit)` | the pieces between matches |
| `Pattern.capture_names()` | the register names the pattern binds |
| `Match.start`, `Match.end`, `Match.span()` | the span in the input's units |
| `Match.byte_start`, `Match.byte_end`, `Match.byte_span()` | the span in UTF-8 bytes |
| `Match.text` | the matched text |
| `Match.captures`, `Match[name]`, `Match.group(name)` | what each register bound; `Match["e:domain"]`, `Match["0:last4"]`, `Match["1"]` read a template reference as `${...}` renders it; a register nested inside a bound pattern is `Match["pair.k"]`, one bound under a repetition a list in `captures` with `Match["pair[0].k"]` one by index |
| `Match.capture_span(name)`, `Match.capture_byte_span(name)` | where it bound it |
| `Pattern(source, lib=path)`, `trex.parse(source, lib=...)` | the pattern read under a pattern file's declarations, or a list of files: `\{name}` resolves to its `let`, `kind` and `shape` lines, and a declared shape or kind decides the token boundaries every scan of the pattern is made on |
| `PatternSet(patterns)`, `PatternSet.from_file(path)` | patterns or sources asked together over one lex; or the set a pattern file declares, a `let` a member under its name and a bare pattern line one under its line number, the file's `kind` and `shape` lines serving every member |
| `PatternSet.is_match(input)`, `.matches(input)`, `.matches_at(input, at)`, `.matches_with_spans(input)` | which patterns match, and each one's first match with its registers |
| `PatternSet.scan(input)`, `PatternSet.names` | every match of every member as `(index, Match)` in position order, the registers under the member's own names; and each member's name, the index as text for a set built from patterns |
| `StreamScanner(pattern)`, `StreamScanner(set)` | `push(chunk)` returns the `(start, end)` byte spans that can no longer change, `finish()` the rest; over a set, `(member, start, end)`, the set streaming through one window under its most conservative member |
| `set_now(secs)`, `set_tz_offset(secs)` | the instant `now` reads in clock clauses such as `\T{age<24h}`, and the zone offset given to a timestamp with no zone |
| `escape(text)` | `text` as a pattern that matches it literally |
| `templates(text=None, cut=None, *, path=None, against=None, novel=False, rare=False, unit="line", ...)` | the record templates of a text or of files read as one stream, most records first, each with its count, its records, both spellings, whether it is under the cut, whether `against`'s input holds no template that would accept its records, and how many records had a template; absent a cut, rare means covering fewer records than the mean template does |
| `infer(examples, anchored=False, against=[])` | the pattern inferred from `bytes` examples, as source; `against` holds examples it must miss, which is what decides whether a position reports a value range or its bare kind |
| `records(text, unit)` | the spans of a record unit (`"line"`, `"paragraph"`, `"block"`, or a pattern) |
| `Pattern.count_by(key, text, order="key")` | the rows `count-by` prints, or `top`'s with `order="count"`; every row, since a truncated table reads like a complete one |
| `Pattern.group_by(key, text=None, *, path=None, ...)`, `Pattern.distinct(key, ...)`, and the same on a `PatternSet` | the groups `count-by` makes over a text or the files `path=` names, each with its sums, exact averages, least and greatest values and percentiles by register; and the keys `uniq` prints |
| `Match.explain(input)` | why a match matched: its tokens, guards, axis readings and the route that answered. The input is passed back rather than held, because holding it would copy the whole input per scan |
| `version()`, `__version__`, `device_available()` | the engine version, and whether a CUDA device is present |

Scans release the interpreter lock while they run. The pattern language is documented
in the [pattern syntax reference](https://github.com/Variably-Constant/trex/blob/main/wiki/content/docs/reference/pattern-syntax.md).

## Building

The wheel is built with [maturin](https://www.maturin.rs) from this directory of the
trex repository, with the `gpu` feature and the stable ABI from Python 3.11:

```console
$ pip install maturin
$ maturin build --release -m python/Cargo.toml
```

PyPI carries a wheel for Windows (`win_amd64`), Linux (`manylinux_2_28_x86_64`) and macOS
(`macosx_11_0_arm64`), each installed and its suite run when it is built. PyPI takes no
FreeBSD wheel, so pip builds one there from the source distribution; a wheel built on FreeBSD
15.0 passes the same suite. A host without a CUDA
driver runs the CPU engine; the device is probed once and its absence is not an error.
