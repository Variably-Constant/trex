---
title: Command reference
linkTitle: CLI
weight: 10
---

# Command reference

`trex` is a single binary with two command families: pattern-language tools (`scan`,
`rewrite`, `lib`, `grammar`, `bpe`, `prefilter`, `compress`) and property-axis analyzers (`spectral`,
`seam`, `shape`, `orbit`, `magnitude`, `stress`, `flow`, `observe`, `echo`, `relation`). Every
example below is verbatim output from the built binary.

## Invocation

```console
$ trex --version
trex 0.1.0
```

Most commands accept a positional `FILE` or an inline `--text STRING`, and print a usage line
and exit non-zero with neither; `scan` and `rewrite` take any number of files, directories and
`-` for the standard input, and read the standard input when given nothing.

| Concern | Behavior |
|---|---|
| Input | positional `FILE`, or `--text STRING`; for `scan` and `rewrite` any number of files, directories walked under `.gitignore` and `.ignore` rules, and `-` for the standard input. A file in any UTF encoding works: a BOM selects UTF-16 LE/BE, UTF-32 LE/BE, or UTF-8 (stripped); BOM-less UTF-16 is detected only on strict evidence (not valid UTF-8, full valid decode, control-clean text), so valid UTF-8, ASCII, and binary always pass through unchanged |
| Exit code | `0` on success; non-zero on missing input and where a command documents its own failure (`scan --require-match`, `grammar --count/--best/--prob` returning zero, `prefilter --verify` finding a false negative) |
| Backend | `scan` and `rewrite` auto-route a large enough eligible input to the GPU when a device is present, else the CPU; `--cpu` forces the CPU |
| Output | every listing prints in full; `--limit N` bounds any per-token / segment / boundary listing, and anything cut is marked (`... (+N more)`). Ranked top-K lists (`--top`) state their bound and the total |

## Pattern-language tools

### scan

```
scan PATTERN [FILE|DIR|-]... [--text STRING] [flags]
```

Tokenize, compile the pattern, sweep the significant-token subsequence once, and print each
leftmost, non-overlapping match span with its captured registers.

```console
$ trex scan '<\W:t>.*</=t>' --text '<div>hi</div>'
[0..13] "<div>hi</div>"  captures: t="div"

$ trex scan '\E:e' --text 'ping bob@x.com' --json
[{"start":5,"end":14,"text":"bob@x.com","captures":{"e":"bob@x.com"}}]
```

Any number of inputs may follow the pattern: a file, `-` for the standard input, or a
directory, walked with `.gitignore` and `.ignore` rules applied and hidden entries skipped, as
ripgrep walks one; no input at all reads the standard input. A file whose bytes hold a NUL
outside a UTF-16 or UTF-32 byte order mark is binary: a walk skips it and naming it outright
is refused, unless `--binary` is given, which decodes it (BOM-less UTF-16 on strict evidence)
and scans it; `--hidden` and `--no-ignore` widen the walk. The inputs are read and scanned
across the cores and reported in path order. One input reports as above. A directory, several inputs, or `-H`
prefix each match with its path, one-based line and character column, and `-A`, `-B` and `-C`
add the lines around the match's first line, with `--` between groups that do not touch. With
`logs/a.log` holding the five lines `alpha 10`, `beta 20`, `gamma 300`, `delta 4000` and
`epsilon 5`:

```console
$ trex scan '\N{>=1000}' logs/ -C 1
logs/a.log-3-gamma 300
logs/a.log:4:7: "4000"
logs/a.log-5-epsilon 5

$ trex scan '\N' logs/ --count
logs/a.log:5
```

| Flag | Effect |
|---|---|
| `--shape 'name = `pat`'` | declare a token shape: a bounded byte-pattern the lexer tries before its built-in recognizers, matched as `\{name}`; repeatable |
| `--shape-after 'name = `pat`'` | the same, tried only where no built-in recognizer matched |
| `--fields` | print the records the pattern's fields read, a row a record with its lines, as `infer` reports them: the fields a `fields` line gives `\{name}` in a pattern file, each typed, read through its accessor and beginning records as its mark says, else one field per register; with `--json`, the records as an array |
| `--json` | emit matches as a JSON array; over named inputs each object also carries `path`, `line` and `col`; under `captures` a register nested inside a bound pattern (`pair.k`) is a match-shaped object under its parent, its text and its children under `captures`, and a register bound under a repetition an array of its bindings, each turn of a bound group an object holding its own children |
| `--require-match` | exit non-zero when nothing matches |
| `-A N`, `-B N`, `-C N` | print N lines after, before, or around each match's first line. Each also takes a record unit in place of the number - `block`, `unit`, `paragraph`, `record` or any other `--record` names - and prints the whole construct the match sits in, whatever its line count; `-C` both sides of it, `-B` up to the match's line and `-A` from it |
| `--count` | print how many lines of each input hold a match, or records where `--record` names a unit; under `-v`, how many hold none; `-m N` stops the count at N |
| `--count-matches` | print how many matches each input holds |
| `-l`, `--files-with-matches` | print each input with a match, once |
| `-H`, `--with-filename` | prefix every match with `path:line:col:`, even for one input; `--no-filename` never does |
| `--hidden` | walk hidden files and directories |
| `--no-ignore` | walk what `.gitignore` and `.ignore` rules would skip |
| `--binary` | scan files that hold a NUL byte, decoded as the other inputs are |
| `--lib FILE` | declarations for `\{name}`, one a line: `let NAME = PATTERN`, `kind NAME = PATTERN`, `shape NAME = `BYTES``, `shape-after NAME = `BYTES``, and `test NAME accepts "text"... rejects "text"...` lines that `lib --test` runs; repeatable |
| `--chunk-size N` | feed the input in N-byte chunks (streaming scan) |
| `--head N`, `--tail N`, `--lines A..B` | scan only the first N records of each input, its last N, or records A through B, `A..` running to its end, `..B` from its start and `A` alone naming one; lines unless `--record` names another unit. A match counts only where it lies wholly inside, and stands at the input's own offset, line and column; see [head, tail, lines](#head-tail-lines) |
| `--follow` | after each file's `--tail`, its `--lines A..` or the whole of it, scan what the file gains as it grows, through truncation and rotation, printing each match once nothing arriving later can change it and `--json` as one object a line; ends once `-m` has printed all it will from every file, and otherwise runs until interrupted. It takes no report printed once the input ends or read around a match: `--count`, `--count-matches`, `-l`, `-L`, `--stats`, `-v`, `--passthru`, context lines, a record query or `--explain`; under `--rules`, no `--fix` or `--sarif`, and no rule that fires on records |
| `--dual-grain` | run the byte grain and token grain as a pipeline |
| `--gpu` | force the SIMT device backend (warns and falls back to CPU when unavailable) |
| `--cpu`, `--nogpu` | force the CPU engine; never probe the device |

The grep flags, as ripgrep and ugrep spell them:

| Flag | Effect |
|---|---|
| `-v`, `--invert-match` | print the lines no match touches, each as `path:line:text` over named inputs and bare over one; with `--count`, `-l` or `-L`, those lines are what is counted or looked for |
| `-x`, `--line-regexp` | keep only a match that covers its line's significant extent, from the line's first non-whitespace byte to its last |
| `-o`, `--only-matching` | the matched text alone, which every report prints already |
| `-m N`, `--max-count N` | at most N matches per input, or N lines under `-v` |
| `-e PATTERN`, `--regexp`, `--pattern` | a pattern, repeatable; several are joined as an alternation, `(P1) \| (P2)`, and no positional pattern is read |
| `-f FILE`, `--file FILE` | patterns from a file, one a line, blank lines skipped; joined as `-e` joins them |
| `--patterns FILE` | the members of a set, from a pattern file: a `let NAME = PATTERN` line is a member under its name, a bare pattern line a member under its line number, and the file's `kind`, `shape` and `test` lines serve every member, as `--lib` does; every match is reported under its member, `pattern: NAME` after its captures, `"pattern"` in `--json`, `${pattern}` in `--format`, and a record query's `patterns` by name; takes no `-e` or `-f` |
| `--single-match` | each member's first match per input and no more, as Hyperscan's `SINGLEMATCH` flag reports; takes `--patterns` |
| `--rules FILE\|DIR` | scan the rules of a pattern file, or of every `.trex` file under a directory, and report each finding under its rule as `path:line:col: severity: message [rule]`, the fix under it; repeatable; takes no pattern, `-e`, `-f` or `--patterns`, and a finding of an `error` rule fails the run |
| `--sarif`, `--github` | the findings as one SARIF 2.1.0 document, or as GitHub workflow annotations; each a report of its own beside `--json`, `--format`, `--count`, `-l` and `-L` |
| `--fix`, `--dry-run`, `-i`, `--interactive`, `-U` | apply the rules' fixes in place; `--dry-run` prints the unified diffs and writes nothing, `--interactive` reviews each fix as `rewrite` does and `-U` applies every fix without a question; `-C N` sets the diff's context lines |
| `--index` | build each walked tree's index as this scan reads its files and write it at the end, after which every scan of that tree prunes with it; see [index](#index) |
| `--no-index` | read no index, whatever a walked tree holds |
| `-g GLOB`, `--glob GLOB` | keep a walked file by a glob, `!GLOB` drops it, read against the path under the directory walked; repeatable; a file named on the command line is read whatever it says |
| `-t TYPE`, `--type TYPE`; `-T TYPE`, `--type-not TYPE` | keep, or drop, walked files of a type under ripgrep's names; `--type-list` prints every type with its globs |
| `-L`, `--files-without-match` | print each input with no match, once |
| `--files` | print the files the walk would read under the paths, or under the current directory, and scan nothing; no pattern is read. With a bare `--texture`, each file is printed with what it reads as |
| `--texture KIND` | keep a file the walk found whose dominant region kind is `table`, `blob`, `prose`, `numeric`, `code` or `mixed`; `!KIND` drops it instead; repeatable. A bare `--texture` names each file's kind rather than filtering. A file named on the command line is read whatever it reads as |
| `--sort KEY`, `--sortr KEY` | order every input by `path`, `modified`, `accessed` or `created`, `--sortr` largest first; path order within each directory otherwise |
| `--color WHEN` | `auto` (the default) paints a terminal at the depth it renders, `never` nothing, `always` what the environment says; `16`, `256` or `truecolor` force a depth |
| `--colors SPEC` | a role's paint, as ripgrep spells it: `match:fg:red`, `path:bg:#202020`, `line:style:bold`, `separator:none`, for the roles `match`, `path`, `line`, `column` and `separator`; also `kind:NAME:...` for a token kind, `kind:*:none\|values\|all` for how much of a line the kinds paint, and `capture:NAME:...` for a register the pattern binds; a color is a base name, `#rrggbb`, `0xRR,0xGG,0xBB` or a number 0-255; repeatable |
| `--stats`, `--stats=line` | after the report, ripgrep's eight lines (matches, matched lines, files with matches, files searched, bytes printed, bytes searched, seconds spent searching, seconds), then the tokens lexed, how the searching time split between lexing and matching, the bytes any device scanned, and how many inputs each scan rung answered; or one line |
| `--passthru` | print every line of each input, a matched line as `path:line:text` with its matches painted, the others as `path-line-text` |

The console's depth is read from the environment: nothing under `NO_COLOR`, off a terminal or
with `TERM=dumb`; 24-bit under `COLORTERM=truecolor` or `24bit`, Windows Terminal, VS Code,
kitty or alacritty; 256 where `TERM` says `256color`; a Windows console renders 24-bit and
has its escape-code processing switched on; any other terminal sixteen. The palette is the
console's own base colors at every depth, so its theme decides the hue: the match bold red,
the path magenta, the line and column green, the `:` and `-` separators cyan. A `--colors`
value the depth lacks goes to the nearest the depth has, a 24-bit value to the nearest entry
of the 256-color cube and grays, then to the nearest of the sixteen.

**The token kinds are painted too, and by default.** Every recognized kind in a printed line
carries a color, which is what makes the highlighting language-agnostic: trex knows a
timestamp, an address and a byte size in a log, a config file or a language it has never
seen, because it reads tokens rather than a grammar.

The scheme is written once as one 24-bit color per kind, and the depth decides how much of it
survives. Each value sits on an entry of the 256-color cube, so a 256-color console renders
exactly what a 24-bit one does and no two kinds collide on the way down; the only lossy step
is the one to sixteen, and that is the step the families are for. Kinds that answer the same
question sit in one arc of the hue wheel, so they stay apart where the console can tell them
apart and merge into their family where it cannot:

| family | the question | kinds | at sixteen colors |
| --- | --- | --- | --- |
| where | an address, a route, a place to reach | `ip` `cidr` `mac` `url` `email` `phone` | blue |
| | | `path` `geo` | cyan |
| how much | a quantity, in whatever unit | `number` `percent` `bytesize` `money` `duration` `quantity` | yellow |
| when | an instant | `timestamp` | green |
| which | an opaque identifier | `uuid` `version` `hexcolor` `quoted` | magenta |
| alarm | a value that is a finding when it appears | `creditcard` `jwt` `base64` `hash` | red, underlined |

The alarm kinds carry an underline beside their hue. An underline is an attribute rather than
a color, so it survives every depth, including the one where the hue has merged into its
family, and it reads to someone who cannot tell red from its neighbors.

Words, punctuation, brackets and whitespace are left plain. That is the load-bearing part: a
line where every token carries a color is a line where the color says nothing, so the quiet
tokens are what make the four or five lit ones readable at a glance. `kind:*` sets how much
of a line is painted - `none` for no kind, `values` for the recognized kinds and quoted
strings, which is the default, and `all` for words and punctuation too, so a line reads as an
editor paints source. `TREX_KIND_COLOR` says the same thing once for every command, and a
`--colors` on the command line wins over it. `--color never` and `NO_COLOR` turn off all
color, this included.

A spec naming one kind wins over both the scheme and the level, so `kind:*:none` followed by
`kind:ip:fg:blue` paints addresses and nothing else. `capture:NAME` paints the tokens a
register binds, over the match's own paint, since a register the reader named is the narrower
thing they asked for.

With `hosts.txt` holding the three lines `10.0.0.1`, `  10.0.0.2  ` and `from 10.0.0.3`
beside the `logs/` above:

```console
$ trex scan -v '\N{>=100}' logs/a.log
alpha 10
beta 20
epsilon 5

$ trex scan -x '\I' hosts.txt
[0..8] "10.0.0.1"
[9..17] "10.0.0.2"

$ trex scan -e '\N{>=1000}' -e '\N{<=10}' logs/a.log
[6..8] "10"
[33..37] "4000"
[46..47] "5"

$ trex scan --passthru -H '\N{>=1000}' logs/a.log
logs/a.log-1-alpha 10
logs/a.log-2-beta 20
logs/a.log-3-gamma 300
logs/a.log:4:delta 4000
logs/a.log-5-epsilon 5

$ trex scan -L '\I' logs/a.log hosts.txt
logs/a.log

$ trex scan --files logs/ -t log
logs/a.log

$ trex scan '\N{>=100}' logs/a.log --stats=line
[23..26] "300"
[33..37] "4000"
2 matches in 1 of 1 files, 48 bytes searched, 6.967 ms
```

The last figure of `--stats=line`, and the two times `--stats` prints, are the run's own.

Under those eight lines `--stats` prints why the scan cost what it did. A scan is a ladder
of routes over an engine, and which rung answered decides the cost far more than the input's
size does, so the block names the rung each input left by, in the words `TREX_TRACE` prints
for the same scan; both read one record, so a rung renamed in the engine moves both. Beside
them it prints the tokens lexed and the searching time split into the part spent reading
bytes into tokens and the part spent matching over them - the two sum to the `seconds spent
searching` above rather than adding to it - and, where a device scanned, the bytes it was
given.

```console
$ trex scan '@echo>0 \W' logs/ --stats
... the eight lines ...
20 tokens lexed
0.000071 seconds spent lexing
0.006768 seconds spent matching
1 files answered by the set engine over a whole lex
```

Three readings that come out of this and are worth knowing. A scan reporting `0 tokens lexed`
never lexed at all: a byte route answered it from the bytes, which is why it was fast. Two
patterns over one corpus can report different token counts, because the paths that read only
the significant tokens drop whitespace and so produce fewer. And the lexing figure is summed
over the lexes, which a large input runs one per chunk at once, so it is the time the cores
spent between them rather than the time on the wall; the matching figure is what is left of
the wall-clock searching time once it is taken off, which is why only one of the two is
measured directly.

The rungs and the lex timings are kept only while `--stats` asks for them, so a scan that
prints no statistics pays nothing: the counting sits behind one atomic read in the lexer's
own shared core, which is the one place every lex in the crate passes through.

A pattern file scanned as a set reports every match under the member that made it. A `let`
line is a member under its name, a bare pattern line one under its line number, and the
file's `kind` and `shape` lines serve every member, so a member may name a shape the file
declares. The members a byte route answers never reach the lexer, and the rest share one
lex, each walked over it as itself, so a set of twenty costs one lex and twenty walks where
twenty scans cost twenty of each. With `rules.trex` and `notes.txt` as shown, and
`other.txt` holding the line `mail amy@y.org`:

```console
$ cat rules.trex
# what a line may hold
let host = \I:addr
let mail = \E:e
\N{>=100}
$ cat notes.txt
from 10.0.0.1 at 500 to bob@x.com
x = 7 and 10.0.0.2
$ trex scan --patterns rules.trex notes.txt
[5..13] "10.0.0.1"  captures: addr="10.0.0.1"  pattern: host
[17..20] "500"  pattern: 4
[24..33] "bob@x.com"  captures: e="bob@x.com"  pattern: mail
[44..52] "10.0.0.2"  captures: addr="10.0.0.2"  pattern: host
$ trex scan --patterns rules.trex --json notes.txt
[{"start":5,"end":13,"text":"10.0.0.1","captures":{"addr":{"text":"10.0.0.1","value":"167772161"}},"pattern":"host"},{"start":17,"end":20,"text":"500","captures":{},"pattern":"4"},{"start":24,"end":33,"text":"bob@x.com","captures":{"e":"bob@x.com"},"pattern":"mail"},{"start":44,"end":52,"text":"10.0.0.2","captures":{"addr":{"text":"10.0.0.2","value":"167772162"}},"pattern":"host"}]
$ trex scan --patterns rules.trex --format '${pattern}:${line}:${col} ${0}' notes.txt other.txt
host:1:6 10.0.0.1
4:1:18 500
mail:1:25 bob@x.com
host:2:11 10.0.0.2
mail:1:6 amy@y.org
$ trex scan --patterns rules.trex --single-match notes.txt
[5..13] "10.0.0.1"  captures: addr="10.0.0.1"  pattern: host
[17..20] "500"  pattern: 4
[24..33] "bob@x.com"  captures: e="bob@x.com"  pattern: mail
$ trex scan --all --patterns rules.trex --json notes.txt
[{"line":1,"start":0,"end":33,"text":"from 10.0.0.1 at 500 to bob@x.com","patterns":["host","mail","4"]}]
```

A `--format` template may name a register of any member, and a match of another member
renders it empty; `${1}` counts through the set's registers in member order, as it counts
through one pattern's. A set scans on the CPU engines and takes no `--gpu`, `--dual-grain`
or `--chunk-size`.

A record query asks of each record whether several patterns are present in it:

| Flag | Effect |
|---|---|
| `--all`, `--any`, `--none`, `--at-least N` | the rule a record is held to over the patterns given positionally, with `-e` or `-f`, or as the members of `--patterns FILE`: every one present, at least one, none, or at least N; a record definition or `--not` with no rule is `--any` |
| `--not PATTERN` | a pattern the record must not hold, under any rule; repeatable |
| `--record UNIT` | what a record is: `line` (the default), `paragraph` (the lines between blank lines), `file`, `period` (the stream's own record period in significant tokens, the reading `@phase:k` uses; the whole input where it has none), `seam` (the seam axis's segments, where the past stops predicting the future), `bind` (the bytes cut where they hold together least under the input's pair field, as many cuts as `seam` makes), `bind:Q` (a cut at every bond in the weakest `Q` per cent), `auto` (`seam` or `bind`, whichever puts more of its cuts at the input's line starts, a boundary neither is built from), `texture` (the spectral texture regions), `shape` (the shape axis's regions between its silhouette change-points), `unit` (a supertoken: a statement, a clause, an argument list) or `unit:ROLE` with the role `call`, `assign`, `kv`, `list`, `numeric` or `plain` |
| `--record-start PATTERN` | a record runs from one match of the pattern to the next, or to the end of the input; the bytes before the first match are in no record |
| `--record-span PATTERN` | each match of the pattern is a record |

Every pattern is scanned once over the whole input, through the routes a scan takes, and a
record holds a pattern where one of its matches overlaps the record, so `^` and `$` read lines
and `\A` and `\z` the input whatever the record is, and nothing is lexed a second time per
record. A qualifying record prints as the lines it covers, each `path:line:text` over named
inputs and bare over one, the matches of the present patterns painted when color is on, and
`--` between records that do not touch; `--count` counts the records, `-l` and `-L` name the
files, `-m N` keeps the first N records, and `--json` gives one object per record with its
first line, byte span, text and the patterns present, by index in the order given or by name
under `--patterns`. A query takes no `--format`, `--explain`, context lines, `-v`,
`--passthru`, `-x` or `--single-match`.

```console
$ cat notes.txt
from 10.0.0.1
to bob@x.com

from 10.0.0.2
nothing

mail amy@y.org
$ trex scan --all -e '\I' -e '\E' --record paragraph notes.txt
from 10.0.0.1
to bob@x.com
$ trex scan --any -e '\E' notes.txt
to bob@x.com
--
mail amy@y.org
$ trex scan --any -e '\I' -e '\E' --record paragraph --json notes.txt
[{"line":1,"start":0,"end":26,"text":"from 10.0.0.1\nto bob@x.com","patterns":[0,1]},{"line":4,"start":28,"end":49,"text":"from 10.0.0.2\nnothing","patterns":[0]},{"line":7,"start":51,"end":65,"text":"mail amy@y.org","patterns":[1]}]
```

A timestamp at the head of a line starts a log entry, so `--record-start '^ \T'` makes each
entry, however many lines it runs, one record:

```console
$ cat events.log
2026-09-15T10:00:00Z login ok
  user bob@x.com from 10.0.0.1
2026-09-15T10:01:00Z login failed
  user amy@y.org from 10.0.0.2
2026-09-15T10:02:00Z logout
  user bob@x.com
$ trex scan --all -e '\E' -e '\I{in:10.0.0.0/24}' --record-start '^ \T' events.log
2026-09-15T10:00:00Z login ok
  user bob@x.com from 10.0.0.1
2026-09-15T10:01:00Z login failed
  user amy@y.org from 10.0.0.2
$ trex scan --none -e '\I' --record-start '^ \T' events.log
2026-09-15T10:02:00Z logout
  user bob@x.com
$ trex scan --any -e '\E' --record-start '^ \T' --count events.log
3
```

A supertoken is a record with no delimiter at all: `unit:assign` makes every binding one,
whatever the language.

```console
$ cat code.txt
x = 1
foo(3)
y = 3
$ trex scan --any -e '\N{>=3}' --record unit:assign code.txt
y = 3
```

The pattern grammar is on the [pattern syntax](../pattern-syntax/) page.

A pattern file may hold rules: a named pattern with what a finding of it says, how serious it
is, the fix that replaces the match, the inputs it reads and the metadata a pipeline filters
on. A rule is a block, `rule NAME` over indented `field = value` lines, or one line, `rule
NAME [error|warning|note] "message" = PATTERN`, its other fields as `fix NAME = TEMPLATE`,
`meta NAME KEY = VALUE`, `files NAME = GLOBS`, `unless NAME = PATTERN` and `record NAME = UNIT`
lines below it; the [pattern syntax](../pattern-syntax/) page gives the fields. `--rules FILE`
scans every rule of the file as one set, `--rules DIR` every `.trex` file under the directory,
and each finding prints as `path:line:col: severity: message [rule]`, the message a report
template rendered from the match - a register's typed slice, `${path:name}`, `${rule}` - with
the rendered fix under it. A rule is also a sub-pattern under its name, so a `test` line
checks it and `lib --test` runs it. With `rules.trex` and `app.conf` as shown:

```console
$ cat rules.trex
# what a config file may not hold
rule private_ip
  pattern = \{ip_private}:addr
  message = private address ${addr} in ${path:name}
  severity = warning
  fix = ${addr:octet1-2}.x.x
  meta.cwe = CWE-200
  meta.tags = network, config

rule cardnum error "card number ending ${card:last4}" = \{card}:card
fix cardnum = ****
meta cardnum tags = pii
rule todo note "a TODO left in ${path:name}" = "TODO"
test private_ip accepts "10.0.0.5" rejects "8.8.8.8"
$ cat app.conf
host = 10.0.0.5
pay 4111 1111 1111 1111 now
# TODO rotate
$ trex scan --rules rules.trex app.conf
app.conf:1:8: warning: private address 10.0.0.5 in app.conf [private_ip]
  fix: "10.0.x.x"
app.conf:2:5: error: card number ending 1111 [cardnum]
  fix: "****"
app.conf:3:3: note: a TODO left in app.conf [todo]
$ trex scan --rules rules.trex --github app.conf
::warning file=app.conf,line=1,col=8,endLine=1,endColumn=16,title=private_ip::private address 10.0.0.5 in app.conf
::error file=app.conf,line=2,col=5,endLine=2,endColumn=24,title=cardnum::card number ending 1111
::notice file=app.conf,line=3,col=3,endLine=3,endColumn=7,title=todo::a TODO left in app.conf
$ trex scan --rules rules.trex --format '${severity} ${rule} ${line}:${col} ${message}' app.conf
warning private_ip 1:8 private address 10.0.0.5 in app.conf
error cardnum 2:5 card number ending 1111
note todo 3:3 a TODO left in app.conf
$ trex scan --rules rules.trex --fix --dry-run app.conf
--- app.conf
+++ app.conf
@@ -1,3 +1,3 @@
-host = 10.0.0.5
-pay 4111 1111 1111 1111 now
+host = 10.0.x.x
+pay **** now
 # TODO rotate
$ trex lib --test rules.trex
rules.trex: 1 test passed
```

`--json` prints one object per finding: `rule`, `severity`, `message`, `path`, `line`, `col`,
`end_line`, `end_col`, `start`, `end`, `text`, `captures`, and `fix` and `meta` where the rule
has them. `--sarif` prints one SARIF 2.1.0 document: the rules under `tool.driver.rules` with
the message template, the pattern, the default level and the metadata (`meta.tags` as the
tags), and each finding a result with its `ruleId`, `level`, message, the file, the region's
start and end line and column with its snippet, and the fix as an artifact change. `--format`
writes `${rule}`, `${severity}`, `${message}` and `${fix}` beside the other report fields. A
`--count`, `-l` or `-L` counts or names inputs by their findings, `-m N` keeps the first N
findings per input, and `--require-match` fails a run with none.

A rule that names `unless` or `record` fires on a record that holds its pattern and none of
the `unless` patterns, a line where it names no record, and the finding covers the record
while the message and the fix read the rule's first match in it. `files = *.py, !test_*`
keeps a rule to the inputs its globs keep, as `-g` reads them, and a rule with globs reads no
unnamed input. The rules that fire on each match scan as one set per list of files; a rule on
records scans on its own.

```console
$ cat vault.trex
rule unguarded
  pattern = "password"
  unless = "vault"
  record = paragraph
  message = a password outside the vault
  severity = error
$ cat notes.txt
password = x
vault: ok

password = y
plain
$ trex scan --rules vault.trex notes.txt
notes.txt:4:1: error: a password outside the vault [unguarded]
```

`--explain` puts under each match the kind and text of every token it spans, what each
guarded kind passed to be that kind, the value of every axis the pattern reads at those
tokens - the number the predicate compared - and the rung of the scan ladder that answered,
which is the route `TREX_TRACE` prints; `--json` carries the same under `explain`.

```console
$ trex scan '\{card}' --explain --text 'pay 4111 1111 1111 1111 now'
[4..23] "4111 1111 1111 1111"
  tokens: creditcard "4111 1111 1111 1111"
  guard: creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check
  route: a route, not the engine

$ trex scan '\N{>+1}' --explain --text 'sizes 12 15 9 4000'
[14..18] "4000"
  tokens: number "4000"
  magnitude: 3.60 "4000"
  baseline: window: mean 1.38, spread 0.55, over 4 "4000"
  route: the set engine over a whole lex
```

`--format TEMPLATE` prints one line per match from a template in the rewrite language:
`${0}`, `${name}` and `${name:acc}` as a rewrite writes them, where the match stands as
`${path}`, `${line}`, `${col}`, `${start}` and `${end}`, and the set member that made it as
`${pattern}` under `--patterns`, each taking accessors (`${path:name}`); `\t` and `\n` are a
tab and a newline. It takes no `--json`, `--count`, `-l` or context lines, and it streams as
the plain report does.

```console
$ trex scan '\I:ip' --format '${line}:${col} ${ip:octet1-2} ${start}..${end}' --text 'from 10.1.2.3 to 192.168.0.1'
1:6 10.1 5..13
1:18 192.168 17..28
```

A template may also name a reading `--explain` computes for the match, written with an `@`,
one per property axis: the kinds it spans as `${@kind}`, the guard each guarded kind passed
as `${@guard}`, the rung that answered as `${@route}`, and every axis the pattern read -
`${@magnitude}`, `${@baseline}`, `${@spectral}`, `${@echo}`, `${@order}`, `${@template}`,
`${@nesting}`, `${@seam}`, `${@ambiguous}`, `${@gravity}`, `${@construct}`, `${@phase}`,
`${@field}` and `${@join}`. The `@` is what keeps these apart from the registers: a pattern binding `:kind`
keeps `${kind}` as its register under every pattern.

A bare axis renders the sentence `--explain` prints for it. A dot names one value of that
sentence instead - `${@spectral.entropy}`, `${@echo.count}`, `${@template.rarity}`,
`${@construct.role}` - which is also how the two depths are told apart:
`${@nesting.depth}` is the bracket nesting a token sits at and `${@construct.depth}` the
enclosing unit's. An axis reads at every token the match spans, so the field renders those
readings joined and an index picks one.

```console
$ trex scan '\W \N' --format '${0} kinds=${@kind}' --text 'code 200'
code 200 kinds=word, number
$ trex scan '\M{>0} \M{>0}' --format 'all=${@magnitude} first=${@magnitude[0]}' --text '12 3400'
all=1.08, 3.53 first=1.08
$ trex scan '@super:call \W' --format '${@construct.role} at depth ${@construct.depth}' --text 'foo(a) bar(b)'
call at depth 0
call at depth 0
```

A name that reads no axis is a parse error, and so is a value the named axis does not
state. An axis the pattern never read is not: it renders empty, since the axes are computed
for the ones the pattern names and no other.

```console
$ trex scan '\W' --format 'mag=[${@magnitude}]' --text 'alpha'
mag=[]
$ trex scan '\W' --format '${@entrpoy}' --text 'alpha'
trex: --format error at byte 0: ${@entrpoy} reads no axis; the axes are kind, guard, route, magnitude, baseline, spectral, echo, order, template, nesting, seam, ambiguous, gravity, construct, phase, join, field
```

Only a report reads an axis. A `rewrite` template and a `count-by` key each refuse one: the
bytes a rewrite splices in have no explanation to read, and a key that rendered every axis
empty would group every match together. A template naming an axis also leaves the streaming
path, as `--explain` does, because an axis is read over the whole input.

`--texture KIND` keeps a file the walk found by what the spectral and shape axes read it as,
and `!KIND` drops one. The kinds are `table`, `blob`, `prose`, `numeric`, `code` and
`mixed`, which are the shape axis's region kinds: a region carrying a strong shape period is
a table whatever its byte texture, and the rest take their spectral texture. A file's kind is
**the one covering the most bytes** - bytes rather than regions, because a source file
holding one long base64 line and forty short code regions is code by count and a blob by
bytes, and the bytes are what a reader means when they call a file one thing.

A bare `--texture` names each file's kind instead of filtering, which is the reading to take
before choosing a filter. A table reports the period of the widest table in it, since the
period belongs to the region rather than to the file.

```console
$ trex scan --files --texture sample/
sample/app.log: mixed
sample/code.rs: code
sample/payload.b64: blob
sample/prose.md: table, period 3
sample/rows.csv: table, period 7

$ trex scan --files --texture blob sample/
sample/payload.b64

$ trex scan --files --texture !blob sample/
sample/app.log
sample/code.rs
sample/prose.md
sample/rows.csv
```

It narrows a scan and not only a listing, and it filters what the walk turned up: a file
named on the command line is read whatever it reads as, since the reader asked for that file
and a texture filter is not an argument about what they asked for. The pass runs once per
walked file. A word that names no kind is refused rather than taken for a path, so a
misspelled kind says so instead of being reported later as a file that could not be read.

`-A`, `-B` and `-C` take a record unit in place of a number, and then print the whole
construct the match sits in rather than a count of lines: `block` is the balanced bracket
group, `unit` the supertoken, `paragraph` the run of non-blank lines, `record` whatever
`--record`, `--record-start` or `--record-span` defined, and any other unit those name works
too. `-C` prints both sides of the construct, `-B` from its start to the match's line, and
`-A` from that line to its end; the match's own line is printed either way, which is what
makes `-A` and `-B` narrower than `-C` rather than empty.

This is the case a count of lines cannot express. A construct's length is a property of the
input, so any `N` is right for some matches and wrong for the rest:

```console
$ cat body.txt
header line
fn outer(a) {
  let v = inner(a, 42);
  return 99;
}
trailer line

$ trex scan '\N{99..99}' body.txt -C block
2-fn outer(a) {
3-  let v = inner(a, 42);
4:10: "99"
5-}

$ trex scan '\N{99..99}' body.txt -B block
2-fn outer(a) {
3-  let v = inner(a, 42);
4:10: "99"
```

The construct is the innermost one holding the match, because the question is which one the
match is inside and the tightest answer is the true one. `42` sits in `inner(a, 42)`, so its
block is that call and not the body around it:

```console
$ trex scan '\N{42..42}' body.txt -C block
3:20: "42"
```

A match inside no construct of the unit reports its own line, and a word that names no unit
is refused, as is naming two:

```console
$ trex scan '\W{in:header}' body.txt -C block
1:1: "header"

$ trex scan '\N' body.txt -C nonsense
trex: the context flags take a number or a record unit: "nonsense" is not a record unit; write line, paragraph, file, period, seam, bind, bind:Q, auto, texture, shape, block, unit or unit:ROLE
```

Naming `record` in a context flag is what tells `--record` it is saying what a record is
rather than asking for a record query, so the report stays one line per match with the
record printed around it. A query asked for outright, with `--rule` or `--not`, still takes
no context lines: those are two reports and only one can be printed.

With the standard input alone and the plain report - no count, file list, context lines,
prefix or explanation - `scan` reads the stream as it arrives and prints each match the
moment the stream scanner commits it, flushed, so
`tail -f app.log | trex scan '\T:t \W \T{>+1h:t}'` reports a gap as soon as the line that
closes it ends; a chunk ending inside a token holds its match until the token ends, and the
bytes behind the scanner's retained window are dropped. `--json` on a stream prints one
object per line rather than an array. A pattern that cannot commit a match before the end of
the stream is scanned when the stream ends and says on the standard error how many bytes it
retained: one with a content guard, a lookahead, a lookbehind that can reach before its match,
a field anchor or a whole-stream axis, and one whose match has no bounded length and which the
set engine walks, such as a balanced group, since only the single-pass engine's own thread
list can say how far back a match completed by a later chunk may begin. An unbounded pattern
the single-pass engine walks does commit early, keeping only the bytes some attempt still
running can reach back over. The stream cuts only just after a newline, so `^` and `$`, and a
lookbehind that reads only tokens of its own match such as `~<($ .)`, commit as their line
ends: a whole-line pattern `trex infer` builds streams a line at a time. A set under
`--patterns` streams through one window, its most conservative member deciding what commits:
a match commits once the window holds the longest span any member can make, and nothing
commits before the end where any member depends on the whole input.

### head, tail, lines

```
head N [FILE|DIR|-]... [-n] [--record UNIT|--record-start PATTERN|--record-span PATTERN] [-H|--no-filename] [--binary]
tail N [FILE|DIR|-]... [-f] [-n] [...]
lines A..B [FILE|DIR|-]... [-f] [-n] [...]
```

Print each input's first N lines, its last N, or lines A through B, as the coreutils `head`
and `tail` print them. The lines are found with the vector newline search and read from the
end they sit at: a head stops reading at its last newline, a range at the end of its last
line, and a tail reads a file backward from its end, so a line near either end of a large
file is printed without the rest being read.

Measured on pc2 at c1c0d5a by `benches/headtail_timing.ps1`: the median of 15 runs of each
command through `cmd /c`, which alone takes 17 to 20 ms, while other work held 9.5 to 11 of the
machine's 24 cores. The UTF-8 rows span five files of 12 to 131 MB, two of them behind a byte
order mark; the UTF-16 rows are one file of 126 MB and 37 lines, whose last ten hold 51 MB of
text. Every command printed the same lines before any was timed.

| Selection | trex | GNU tools | ripgrep | ugrep | Pipes |
|---|---|---|---|---|---|
| `head 10` | 25.0-29.1 ms | `head -n 10` 30.7-34.4 | `rg -m 10 ''` 28.1-33.4 | `ugrep -K 1,10 ''` 29.6-32.8 | |
| `tail 10` | 27.3-27.9 | `tail -n 10` 29.8-35.0 | `rg '' \| tail` 93.5-392.5 | | |
| `tail 10 -n` | 31.4-64.3 | | `rg -n '' \| tail` 110.7-460.2 | | `cat -n \| tail` 89.1-494.5 |
| ten lines from the middle | 30.4-47.9 | `sed -n` 49.5-133.0 | | `ugrep -K` 32.7-43.4 | `head \| tail` 87.3-327.1 |
| UTF-16 `head 10` | 28.4 | | `rg -m 10 ''` 28.9 | `ugrep -K 1,10 ''` 46.0 | |
| UTF-16 `tail 10` | 169.5 | | `rg '' \| tail` 235.6 | | |
| UTF-16 `tail 10 -n` | 240.2 | | `rg -n '' \| tail` 238.2 | | |
| UTF-16 `lines 18..27` | 59.9 | | | `ugrep -K 18,27 ''` 442.6 | |

```console
$ trex head 2 logs/a.log
alpha 10
beta 20

$ trex tail 2 -n logs/a.log
4:delta 4000
5:epsilon 5

$ trex lines 2..3 logs/a.log
beta 20
gamma 300

$ trex head --lines ..2 logs/a.log
alpha 10
beta 20
```

`trex lines` takes `A..B`, `A..` for A to the end, `..B` from the start, or `A` alone, and
`--lines` gives `head` and `tail` the same range in place of N. `-n` prints each line after
its number in the input, `N:text`. A tail's lines are numbered by counting the newlines ahead
of it, in blocks across the cores, which a tail printing no numbers never reads. `--record
UNIT` counts paragraphs, blocks or any other unit `--record` names instead of lines, and
`tail -N` is `tail N`. Several inputs are each headed `==> name <==`, a blank line between
two, and the standard input is read when no input is named or `-` is. The text printed is the
input's own, without its byte order mark and decoded where the mark declares UTF-16 or
UTF-32. A marked file is read as any other is, only the part selected: a head or a range of
UTF-16 or UTF-32 is decoded as it is read and stops at its end, and a tail is read backward
in whole code units. Offsets count the text, so a marked file's are the unmarked text's, and
the text ahead of a UTF-16 or UTF-32 tail is counted only where a report prints its offsets
or line numbers or a follow continues them. A file holding a NUL byte and no UTF-16 or
UTF-32 mark is binary and printed only under `--binary`.

`-f` (`--follow`) prints, after a tail or a range with no last line, what each file gains as
it grows, until interrupted. A file cut shorter is printed again from its start and one
replaced under its name, as a rotated log is, is read from its first byte, each said so on
the standard error; under `-n` a line is printed once its end arrives, so its number heads
the whole of it.

The same three selections restrict the other commands as `--head N`, `--tail N` and `--lines
A..B`, counted in the unit `--record` names: `scan` reads only that part of each input, a
match counting only where it lies wholly inside and standing at the input's own offset, line
and column; `rewrite` and `redact` print the window alone, rewritten, and in place change only
the window, the file keeping every other byte; the tables group its matches and `templates`
mines its records. With `--follow`, `scan`, `rewrite` and `redact` go on reading each file as
it grows.

```console
$ trex scan '\N' logs/a.log --tail 2
[33..37] "4000"
[46..47] "5"

$ trex scan '\N' logs/a.log --lines 2..3 -H
logs/a.log:2:6: "20"
logs/a.log:3:7: "300"

$ trex rewrite '\N:n' '<${n}>' logs/a.log --tail 1
epsilon <5>
```

### rewrite

```
rewrite PATTERN TEMPLATE [FILE|DIR|-]... [--text STRING] [--in-place] [--dry-run] [--interactive] [-U] [-C N]
        [--explain] [--show-skipped] [--gpu|--cpu] [--head N|--tail N|--lines A..B] [--record UNIT] [--follow]
```

Replace each match with a rendered template.

```console
$ trex rewrite '\E:e' '[redacted]' --text 'mail bob@x.com now'
mail [redacted] now
```

Template syntax: `${name}` renders a named capture, `${0}` the whole match, and `${name:acc}`
transforms or slices it, chaining with `|`. `upper` and `lower` transform; `trim`, `firstN` and
`lastN` slice any capture; the rest slice a captured atom by its typed structure:

| Captured atom | Accessors |
|---|---|
| any | `firstN`, `lastN`: the first or last N characters |
| `\I` (IPv4) | `octetN`, `octetN-M` |
| `\I` (IPv6) | `groupN`, `groupN-M` |
| `\U` | `scheme`, `host`, `port`, `path`, `query` |
| `\E` | `user`, `domain` |
| `\V` | `major`, `minor`, `patch` |
| `\T` | `year`, `month`, `day`, `hour`, `minute`, `second` |
| `\L` | `dir`, `name`, `ext` |

`$$` is a literal dollar sign, and `\n`, `\t` and `\\` are a newline, a tab and a backslash; a
backslash before anything else is an error. One input is rewritten to the standard output. `--in-place`
writes each file with a match back in place, in the encoding it was read in and behind its
own byte order mark, every byte outside a match as it was, and `--dry-run` prints instead a unified diff
per file that would change, with three lines of context, or `-C N`. A directory or several
inputs need one of the two, and take `--hidden`, `--no-ignore` and `--binary` as `scan` does.
`--lib FILE` supplies the pattern's declarations, `let` sub-patterns and `kind` and `shape`
lines alike, as it does for `scan`, `redact` and the tables: a rewrite under a declared shape
lexes on the boundaries that shape makes and replaces what a scan under it reports. Such a
rewrite stays on the CPU engines, since the device kernel lexes for itself and knows nothing
of a shape a pattern file declared. `--head N`, `--tail N` and `--lines A..B` rewrite only
that part of each input, as [head, tail, lines](#head-tail-lines) says, and `--follow`
rewrites what one file gains as it grows, printing each byte once nothing arriving later can
change it.

```console
$ trex rewrite '\N{>=1000}' '[${0}]' logs/ --dry-run
--- logs/a.log
+++ logs/a.log
@@ -1,5 +1,5 @@
 alpha 10
 beta 20
 gamma 300
-delta 4000
+delta [4000]
 epsilon 5
```

`--interactive` (`-i`) shows each change as its unified diff and asks: `y` accepts it, `n`
skips it, `e` takes another replacement, in the editor `VISUAL` or `EDITOR` names, opened on
a file holding the proposed one and read back with its final newline dropped, or typed as one
line at the prompt where neither is set; `a` accepts it and every change after; `q`, or the
end of the answers, ends the session. The accepted changes are written once the session has
seen the last one, so a session that ends early leaves every file as it was, and what was
written is reported as `--in-place` reports it. `-U` (`--update-all`) applies every change
without asking. The answers are read from the standard input and the diffs and prompts go to
the standard output, so a review runs from a pipe as well as a hand; an answer arriving from
a pipe is not echoed, so the next diff follows the prompt on its line. With answers `n` and
`y`:

```console
$ printf 'n\ny\n' | trex rewrite '\N{>=100}' '[${0}]' logs/a.log --interactive
--- logs/a.log
+++ logs/a.log
@@ -1,5 +1,5 @@
 alpha 10
 beta 20
-gamma 300
+gamma [300]
 delta 4000
 epsilon 5
 template: number (1 later change shares it)
[1/2] accept, skip or edit this change, all of this template, none of it, accept all, or quit [y/n/e/t/T/a/q]? --- logs/a.log
+++ logs/a.log
@@ -1,5 +1,5 @@
 alpha 10
 beta 20
 gamma 300
-delta 4000
+delta [4000]
 epsilon 5
 template: number (0 later changes share it)
[2/2] accept, skip or edit this change, all of this template, none of it, accept all, or quit [y/n/e/t/T/a/q]? logs/a.log: 1 replacement
```

Under each change is the template of its match: the kinds of the tokens the match spans, in
order, which is the templates axis's reading of that span. Two changes share a template when
that reading is equal, so `user bob logged in` and `user amy logged in` are one template and
the text between them is not what decides it.

`t` accepts this change and every later one of its template, and `T` skips them, so a
thousand changes of five shapes take five answers rather than a thousand. The prompt says
how many later changes an answer would carry before it is given, so neither is a guess:

```console
$ printf 't\n' | trex rewrite '\N{>=100}' '[${0}]' logs/a.log --interactive
--- logs/a.log
+++ logs/a.log
@@ -1,5 +1,5 @@
 alpha 10
 beta 20
-gamma 300
+gamma [300]
 delta 4000
 epsilon 5
 template: number (1 later change shares it)
[1/2] accept, skip or edit this change, all of this template, none of it, accept all, or quit [y/n/e/t/T/a/q]? logs/a.log: 2 replacements
```

The two are told apart by their case, and the answer is read as typed rather than folded: a
reviewer typing `T` means the opposite of `t`, and folding the case would silently accept
what they meant to skip. How many changes a `T` passed over is always reported;
`--show-skipped` names the places as well, which is a flag rather than a default because a
list at the end of a long review is where a reviewer is least likely to read one. Nothing is
destroyed either way - a skipped change is simply not applied, and the same command offers
it again.

`--explain` puts under each diff what `scan --explain` puts under each match: the kinds the
match spans, the guard each guarded kind passed, the value of every axis the pattern read
there, and the route that answered. A reviewer answering for a whole template can then see
what the pattern actually read, rather than only what the text looks like.

```console
$ trex rewrite '\I:ip' '${ip:octet1-2}.0.0/16' --text 'conn from 192.168.5.9'
conn from 192.168.0.0/16
```

### redact

```
redact PATTERN [FILE|DIR|-]... [--text STRING] [--keep 'name:acc, ...'] [--mask C] [--in-place] [--dry-run] [-C N]
```

Mask every match, leaving the fields `--keep` names where they stand. A kept field is written
as a reference is written in a template - `card:last4`, `ip:octet1-2`, `email:domain`, or
`0:last4` for the whole match - and must slice rather than transform, so `upper` is refused.
The pattern's guarded kinds do the false-positive work: a run of digits is a card only under
the Luhn check, so a number that merely looks like one is left alone.

```console
$ trex redact '(\{card}:card | \I:ip | \E:email)' --keep 'card:last4, ip:octet1-2, email:domain' --text 'card 4111 1111 1111 1111 from 10.1.2.3 by bob@corp.example, ref 1234 5678 1234 5678'
card ***************1111 from 10.1**** by ****corp.example, ref 1234 5678 1234 5678
```

Each masked character becomes one `*`, so every offset and column after a match survives and
a redacted log still lines up with the one it came from. `--mask C` masks with another
character; a `--mask` longer than one character is a token each masked run becomes:

```console
$ trex redact '\{card}:c' --keep c:last4 --mask '[card]' --text 'paid with 4111 1111 1111 1111 today'
paid with [card]1111 today
```

Two masks read the run they cover rather than writing over it blindly.

`--mask shape` masks every letter and digit and keeps every other byte, so the run keeps the
shape its kind is recognized by and the redacted copy still lexes as the original did. A
digit becomes `0` and a letter becomes `a` - `a` because it is a letter and a hex digit at
once, so one rule covers the alphabetic kinds and the hex-shaped ones together. Masking
letters to `x` would keep an email reading as an email and turn `#A3F2B1` into a punctuation
mark and a word, and scatter a uuid and a mac into a dozen tokens each.

```console
$ trex redact '\I' --mask shape --text 'from 10.4.5.6 and 10.9.9.9'
from 00.0.0.0 and 00.0.0.0

$ trex redact '\E' --mask shape --text 'user bob@x.com wrote'
user aaa@a.aaa wrote
```

`--mask pseudonym` replaces each distinct value with a stable name for its kind, numbered in
order of first sight, so the same value gives the same name everywhere in a document and
across the inputs of one run. A declared shape or kind, and a library kind the pattern names,
is named as its declaration names it: a `customer` shape's values become `CUSTOMER_1`,
`CUSTOMER_2`. The recurrence a value carried survives with it, which is what lets `@echo`,
the joins and `trex templates` read the redacted copy as they read the original.

```console
$ trex redact '\I' --mask pseudonym --text 'from 10.4.5.6 to 10.9.9.9 and back to 10.4.5.6'
from IP_1 to IP_2 and back to IP_1
```

The name is one token to the lexer, which is the whole of why it is spelled this way: a name
that lexed as several tokens would scatter the recurrence across them and lose exactly what
the mask exists to keep. Because the numbering follows the order values are first seen, a
run under this mask reads its inputs in the order it walked them rather than the order the
cores finish, so two runs of one command over one tree agree.

`shape` and `pseudonym` are the only two words a `--mask` reads as a name rather than as the
token to write; any other multi-character mask is still that token.

One input is redacted to the standard output. `--in-place`, `--dry-run`, `-C N`, `--lib`,
`--shape`, `--hidden`, `--no-ignore`, `--binary`, the windows and `--follow` work as they do
for `rewrite`; printed, a window is redacted and printed alone, so nothing outside it reaches
the output unredacted.

### count-by, top, uniq

```
count-by PATTERN KEY [FILE|DIR|-]... [--text STRING] [--lib FILE] [--shape DECL] [--json] [-n N]
         [--sum ${c}] [--avg ${c}] [--min ${c}] [--max ${c}] [--p50 ${c}] [--p95 ${c}]
         [--percentile nearest|linear|lower|hybrid] [--avg-form repetend|rational]
         [--values exact|natural|tagged] [--duration-unit ns|ms|s]
         [--head N|--tail N|--lines A..B] [--record UNIT]
count-by --patterns FILE KEY [FILE|DIR|-]...
top      PATTERN KEY ...
uniq     PATTERN KEY ...
```

Scan for `PATTERN`, or for the members of the set `--patterns FILE` declares as `scan` reads
it, and group the matches by `KEY`, a report template as `scan --format` takes one, rendered
once per match, so a capture's typed slice is what the rows count: `${u:host}`,
`${ip:octet1-2}`, `${e:domain}`, or a composite like `${m:upper}/${u:host}`; `${path}` counts
the matches by input, `${line}` by line, and `${pattern}` by the set member that made them.
`count-by` orders by key, `top` by count with the key breaking ties, and `uniq` prints the
keys alone.

`--sum`, `--avg`, `--min`, `--max`, `--p50` and `--p95` each take one capture and add a
column, computed in the value's base unit through the same parse a `:value` clause compares
on, so an aggregate and a reported value agree. `--sum` and `--avg` take the numeric kinds:
number, byte size, duration, money and percent. `--min`, `--max` and the percentiles also
take timestamp, version and address, because ordering those is defined where adding them is
not. An aggregate a kind cannot carry is refused before anything is scanned, naming the
register, the kind it binds and the kinds the aggregate takes.

An aggregate names one plain capture: `${u:host}` is a slice of text and `${a}/${b}` names
two values, so both are refused. A key whose matches bound no value prints `-` rather than
`0`, and `null` in `--json`.

`--percentile` decides a percentile falling between two observed values and defaults to
`nearest`, the value at `ceil(p x n)` in sorted order. `linear` interpolates and takes
numeric kinds only, refusing by name on the rest rather than falling back. `lower` names the
observed value at or below. `hybrid` interpolates where the kind allows it and names an
observed value where it does not.

`--avg` is exact. Division is not closed over terminating decimals, but every rational has a
finite way to write it: `--avg-form repetend` brackets the repeating digits and `rational`
writes the fraction in lowest terms. Nothing rounds.

```console
$ trex count-by '\W:h \Z:s' '${h}' sizes.txt --sum '${s}' --min '${s}' --max '${s}'
       count  --sum s  --min s  --max s
alpha      3     7000     1000     4000
beta       1     8000     8000     8000

$ trex count-by '\W:h \Z:s' '${h}' sizes.txt --avg '${s}'
       count   --avg s
alpha      3  2333.(3)
beta       1      8000

$ trex count-by '\W:h \Z \R:d' '${h}' sizes.txt --sum '${d}' --duration-unit ms
       count  --sum d
alpha      3      240
beta       1      300
```

Every key is printed. `-n N` cuts the table to your own number and says how many keys and
matches there were, so a short table never reads as a complete one. A key an accessor leaves
empty prints as `-` and is counted, so the rows always sum to the matches.

```console
$ trex top '\U:u' '${u:host}' --text 'GET http://a.example/x 200, GET http://b.test/y 404, POST http://a.example/z 200, GET http://a.example/w 500'
a.example  3
b.test     1

$ trex top '\I:ip' '${ip:octet1-2}' --text 'from 10.1.2.3 and 10.1.9.9 and 192.168.0.1'
10.1     2
192.168  1
```

With the three files [scan](#scan) uses, shown again because the `rules.trex`
and `notes.txt` above them belong to [rules](#rules-and-linting) and a page is
read downwards:

```console
$ cat rules.trex
# what a line may hold
let host = \I:addr
let mail = \E:e
\N{>=100}
$ cat notes.txt
from 10.0.0.1 at 500 to bob@x.com
x = 7 and 10.0.0.2
$ cat other.txt
mail amy@y.org
$ trex count-by --patterns rules.trex '${pattern}' notes.txt other.txt
4     1
host  2
mail  2
$ trex count-by '\E' '${path}' notes.txt other.txt
notes.txt  1
other.txt  1
```

### templates

```
templates [FILE|DIR|-]... [--text STRING] [--record UNIT | --record-start PATTERN | --record-span PATTERN]
          [--against FILE|DIR] [--novel] [--pattern] [--rare] [--cut N|P%] [--json] [--hidden] [--no-ignore] [--binary]
          [--head N|--tail N|--lines A..B]
```

Group the records of the inputs by their token-kind silhouette and print each template once
with the records it covers, most frequent first. A record is a line until `--record` names
another unit. Within a group a position whose text is the same in every record is a literal,
and one whose text varies is a slot named by its kind, so a log of requests reads as its
handful of shapes rather than its thousands of lines. Several inputs are one stream.
`--head N`, `--tail N` and `--lines A..B` mine only that part of each input, counted in the
records `--record` names, and `--against` reads its inputs whole.

```console
$ cat access.log
10.0.0.1 GET /index.html status 200 12ms
10.0.0.2 GET /about.html status 200 8ms
10.0.0.1 POST /login status 302 40ms
10.0.0.3 GET /index.html status 200 11ms
10.0.0.9 GET /admin status 403 3ms
kernel: disk failure on /dev/sda

$ trex templates access.log
5  <ip> <word> <path> status <number> <duration>
1  kernel: disk failure on /dev/sda
```

`--pattern` prints each template as a pattern `scan` accepts as written, literals quoted and
slots as atoms, and each finds exactly its template's lines:

```console
$ trex templates access.log --pattern
5  \I \W \L "status" \N \R
1  "kernel" ":" "disk" "failure" "on" "/dev/sda"
```

A template is rare when it covers fewer lines than the mean template does - the lines with a
template divided by the distinct templates - so the cut moves with the input and names no
number; `--cut 5` makes it fewer than five lines and `--cut 1%` less than one percent of the
lines. `--rare` prints the rare templates alone, and `--json` carries every template's count,
record numbers, both spellings, whether it is rare under the cut, and the `shared` or `novel`
mark where `--against` gave one. The same reading is the `@shape:rare` anchor in a pattern,
which holds at every token of a rare line.

```console
$ trex templates access.log --rare
1  kernel: disk failure on /dev/sda

$ trex scan '@shape:rare \W' access.log
[194..200] "kernel"
[202..206] "disk"
[207..214] "failure"
[215..217] "on"
```

`--record UNIT` groups records rather than lines, taking the units a record query takes -
`line`, `paragraph`, `file`, `period`, `seam`, `bind`, `bind:Q`, `auto`, `texture`, `shape`,
`block`, `unit` or `unit:ROLE` - and `--record-start PATTERN` / `--record-span PATTERN` say where a record
begins or what one is. A record of several lines has one template spanning all of them:

```console
$ cat stanzas.log
job alpha
status ok
took 12s

job bravo
status ok
took 30s

job delta
status failed
took 4s

$ trex templates stanzas.log
6  <word> <word>
3  took <duration>

$ trex templates stanzas.log --record paragraph
3  job <word>
status <word>
took <duration>
```

`--against FILE` mines a second input the same way and marks each template of the first
`shared` or `novel`: shared when the other input holds a template that would accept these
records, which is the same silhouette with a literal matching that literal and a slot
admitting anything of its kind. The reading is asymmetric on purpose - a log that has seen
one name at a position accepts only that name, and one that has seen several accepts them
all - because the question is whether the other log would have found these records ordinary.
`--novel` prints only the novel ones.

```console
$ cat app.log
user bob logged in
user amy logged in
disk sda ok
disk sdb ok
quota 90 exceeded

$ cat other.log
user carl logged in
user dana logged in
user eve logged in
disk sda ok

$ trex templates app.log --against other.log
novel   2  disk <word> ok
shared  2  user <word> logged in
novel   1  quota 90 exceeded

$ trex templates app.log --against other.log --novel
novel   2  disk <word> ok
novel   1  quota 90 exceeded
```

`disk <word> ok` is novel because the other log has seen only `sda` at that position, so its
template would not accept `sdb`; `user <word> logged in` is shared because the other log has
seen several names there and its slot admits any of them.

### infer

```
infer EXAMPLE EXAMPLE... [--not EXAMPLE]... [-f FILE] [--anchored]
```

The most specific pattern every example matches, read off an alignment of the examples'
tokens: a position where every example has the same text is that literal, one where the
kinds agree and the texts differ is the kind's atom, one where the kinds differ is the class
of the kinds seen, a run of one kind whose length differs is that kind repeated with the
bounds seen, and a position some examples lack is optional. The pattern is verified against
every example, whole, before it is printed. `-f FILE` reads examples one per line, and with
no example and no `-f` the lines of the standard input are the examples.

```console
$ trex infer '10.0.0.1 GET /index.html status 200 12ms' '10.0.0.2 POST /login status 302 40ms'
\I \W \L "status" \N \R

$ trex infer 'id 200 ok' 'id abc ok'
"id" [\N \W] "ok"

$ trex infer 'disk failure on sda' 'disk failed sda'
"disk" \W{1,2} "sda"
```

A position whose texts differ but fold to one text under an orbit rung prints that text
under the rung, which is narrower than the kind and still names what every example had. The
rungs it folds to are the ones that fold two spellings of one thing: `case`, `notation`,
`numeric`, `ip`, `url`, `time`, `path` and `fold`. `shape` and `e8` are not among them -
they fold spans that merely share a structure, so `cat` and `dog` are one span under
`shape`, and a pattern folded there would match words no example resembled.

```console
$ trex infer 'GET /a' 'get /a'
(?orbit:case "get") "/" "a"

$ trex infer 'user cat' 'user dog'
"user" \W
```

`--not EXAMPLE` gives an example the pattern must miss, and it is what decides whether a
position reports a value range or its bare kind. With no counter-example every position
reports its kind, however well the values agree: two numbers are a sample of two, and with
nothing to tell them from there is no evidence a range around them means anything. Where
counter-examples are given, the ranges that exclude them are added one at a time, most
excluded first, so the pattern carries the ranges it needed and no others.

A range rounds outward to a boundary the kind's own units have rather than stopping at the
span the examples showed: a number to the leading digit place of its larger bound, an
address to the block its bits share, a timestamp to the calendar unit holding every one, a
version to the line it sits on. `200..204` is what three status codes happened to show;
`200..299` is the class they came from, and it is the one that still holds tomorrow's 206.

```console
$ trex infer 'code 200' 'code 204'
"code" \N

$ trex infer 'code 200' 'code 204' --not 'code 500'
"code" \N{200..299}

$ trex infer 'port 8080' 'port 8443' --not 'port 22'
"port" \N{8000..8999}

$ trex infer 'from 10.0.1.4' 'from 10.0.9.7' --not 'from 192.168.0.1'
"from" \I{in:10.0.0.0/20}

$ trex infer 'v 1.2.0' 'v 1.9.3' --not 'v 2.0.0'
"v" \V{major=1}
```

A counter-example nothing separates is refused rather than answered, since a pattern that
matches what it was told to miss is a wrong answer and not a near one:

```console
$ trex infer 'code 200' 'code 204' --not 'code 201'
trex infer: the inferred pattern `"code" \N` still matches counter-example 1; nothing the examples have in common tells them apart
```

`--anchored` adds `^` and `$`, so the pattern matches whole lines only:

```console
$ trex infer 'user bob logged in' 'user amy logged in' --anchored
^ "user" \W "logged" $ "in"
```

With a field marked, `infer` builds the pattern that extracts it from every shape of the
lines, and reports what it reads from each:

```
infer --mark LINE... [--marks FILE] [--field NAME=VALUE]... [LINE... | -f FILE] [--marked]
      [--not LINE]... [--unanchored] [--no-mint | --mint-shapes] [--lib FILE]... [--shape DECL]...
      [--pattern | --json | --lib-file]
```

`--mark LINE` gives a line with each value to extract written `{name:text}`, the markup of
PowerShell's `ConvertFrom-String` templates, so a saved template reads unchanged; `--marks
FILE` reads such lines one per line. `--field NAME=VALUE` names a field by one value it
takes, found wherever it stands in the lines; repeat it for more values or more fields. The
other lines are read as they are, and `--marked` reads marks in them too.

Lines group by the kinds of their tokens. A group holding a field is a shape, a group whose
tokens align with a shape joins it, and a group that aligns with none is a shape of its own,
its fields found where the literals standing around them in a marked shape stand. A marked line that marks fewer fields than another marked shape holds takes the rest the same way, its marks
always winning. A literal
stands there only as the same occurrence of its text, so a line cut short after its third comma
reads the fields between its commas and not the one after the fourth; and a marked shape gives
fields only to a line holding one of its words, since a comma or the line's end says nothing of
which record a line is. Each shape is one branch of the pattern, matching whole lines unless
`--unanchored` is given.
A field is never spelled as its text, since it is what varies: it is its kind, a group of
kinds with constant punctuation kept, the class of its kinds, its runs aligned as above, or
free text to the line's end when it ends the line, and optional where a line of its shape
lacks it. The pattern is verified before it is printed: every line matched whole, every
field read back as it was placed, and no `--not` line matched.

The report gives the pattern, the `--format` template writing every field, each shape with
its lines and how it reaches each field, and what the pattern reads from every line:

```console
$ trex infer --mark '{month:2023-10} Cumulative Update for Windows {os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})' '2023-09 Cumulative Update Preview for Windows 11 Version 22H2 for x64-based Systems (KB5030310)' '2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)' 'Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)'
pattern  ^ (\N "-" \N)?:month "Cumulative" "Update" "Preview"? "for" "Windows" (\N):os "Version" (\N \W?):version "for" "x64" "-" "based" "Systems" "(" (`KB[0-9]{7}`):kb ")" ~<($ .) | ^ (\N "-" \N):month "Security" "Monthly" "Quality" "Rollup" "for" "Windows" (\N):os "for" "x64" "-" "based" "Systems" "(" (`KB[0-9]{7}`):kb ")" ~<($ .)
format   ${month}\t${os}\t${version}\t${kb}

shape  lines  reads
1      1-2 4  month os version kb marked
2      3      month os kb by the literals of shape 1; no version

line  shape  month    os  version  kb         text
1     1      2023-10  11  22H2     KB5031354  2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB5031354)
2     1      2023-09  11  22H2     KB5030310  2023-09 Cumulative Update Preview for Windows 11 Version 22H2 for x64-based Systems (KB5030310)
3     2      2020-01  7   -        KB4534310  2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)
4     1      -        10  1607     KB4103720  Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)
```

Two shapes whose words differ only by words one of them lacks are one branch, those words
optional, as the Preview title is here; a merge is kept only where every line still reads its
fields back and no `--not` line matches.

A field that is one word token, whose values split into the same runs of letters, digits and
punctuation with a run of digits among them and a run of letters the same in every value, is
spelled as that byte shape rather than as `\W`: every `kb` above is `KB` then seven digits, so
`kb` is `` `KB[0-9]{7}` ``, an inline byte atom matched whole against the token, and a title
ending in `(KB50313541)` or `(XB5031354)` is refused where `\W` would take it. The shared letters
are kept and every other run is written as its class with the lengths the values show; digit
counts alone mint nothing, so `month` and `os` stay `\N`, as a value range stays unprinted until
a `--not` line needs one. Every branch holding the field reads the same shape, taken over all
of its values; a word inside a field of several tokens is minted from the values its own branch
holds. `--no-mint` spells the field as its kind.

A field the lexer reads as several tokens, whose every value a shape of the shipped library
reads as one, is that shape: `CVE-2023-1234` and `CVE-2024-56789` make the field `\{cve}`, and a
spaced IBAN `\{iban}`. The shape must carry evidence of its own, a letter or digit it holds as
written (`CVE-`, `AKIA`) or a check that refuses a value with one character changed (an IBAN's
mod 97, an ISBN's check digit), so a shape of bare character classes such as `k8s_name` never
replaces `(\N "-" \N)` for `2023-10`. It must also change no other token of the lines it is
tried on, since a pattern naming it lexes the whole line with it. A value the lexer already
reads as one number or word keeps its kind, whatever check its digits happen to pass.

`--mint-shapes` declares each field's byte shape as a named shape instead, `shape kb =
`KB[0-9]{7}``, and spells the field `\{kb}`, under a name that shadows no built-in, library or
declared one (the field's own, else with `_shape` after it). The pattern then reads only beside
its declarations: the report lists each as a `declare` line, `--json` holds them as
`declarations`, `--lib-file` writes them above the `let` lines, and `--pattern` names them on
standard error as the `--shape` options a scan needs. A declared shape is tried at every
token's start, before the built-in recognizers, so it lexes the whole of each line it is
scanned over, where an inline atom is matched against one token alone; a word inside a field of
several tokens stays an inline atom.

```console
$ trex infer --mark '{month:2023-10} Cumulative Update for Windows {os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})' '2023-09 Cumulative Update Preview for Windows 11 Version 22H2 for x64-based Systems (KB5030310)' '2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)' 'Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)' --mint-shapes --lib-file
# Built by trex infer from 4 lines in 2 shapes; `extract` names every shape in order.
shape kb = `KB[0-9]{7}`
let extract_1 = ^ (\N "-" \N)?:month "Cumulative" "Update" "Preview"? "for" "Windows" (\N):os "Version" (\N \W?):version "for" "x64" "-" "based" "Systems" "(" (\{kb}):kb ")" ~<($ .)
let extract_2 = ^ (\N "-" \N):month "Security" "Monthly" "Quality" "Rollup" "for" "Windows" (\N):os "for" "x64" "-" "based" "Systems" "(" (\{kb}):kb ")" ~<($ .)
let extract = \{extract_1} | \{extract_2}
fields extract {month} {os} {version} {kb}
test extract accepts "2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB5031354)" "2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)"
```

A field marked more than once in one line is a list. The words between its first two values
are the separator every later pair shares, the field repeats as often as a line holds values,
and a line with one value holds a list of one. Its template is `${ip[*]}`, every value joined
with a comma, and `--json` writes each value as an array:

```console
$ trex infer --mark 'from {ip:10.0.0.1} -> {ip:10.0.0.2} ok' 'from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok' 'from 10.0.0.3 ok'
pattern  ^ "from" (\I):ip ("-" ">" (\I):ip)* "ok" ~<($ .)
format   ${ip[*]}

shape  lines  reads
1      1-3    ip marked

line  shape  ip                          text
1     1      10.0.0.1,10.0.0.2           from 10.0.0.1 -> 10.0.0.2 ok
2     1      10.0.0.7,10.0.0.8,10.0.0.9  from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok
3     1      10.0.0.3                    from 10.0.0.3 ok
```

A value given by `--field` that stands twice in one line is not a list: it is refused unless a
marked line of its shape says which of the two is the field.

A mark inside a mark is a field inside a field, bound as trex binds a capture inside a capture:
the inner field is named after the outer one, a dot, and its own name, `Line.n`, and a template
reads it as `${Line.n}`. `--json` gives the outer field as an object of its `text` and the
fields inside it by their own names, `{"Line":{"text":"5 of 9","n":"5","m":"9"}}`, and each
field names its `parent`. The mark holding marks takes whole tokens, once in a line.

```console
$ trex infer --mark '{Line:{[int]n:1} of {[int]m:3}}' '5 of 9' '6 of 9'
pattern  ^ ((\N):n "of" (\N):m):Line ~<($ .)
format   ${Line}\t${Line.n}\t${Line.m}

shape  lines  reads
1      1-3    Line Line.n Line.m marked

line  shape  Line    Line.n  Line.m  text
1     1      1 of 3  1       3       1 of 3
2     1      5 of 9  5       9       5 of 9
3     1      6 of 9  6       9       6 of 9
```

A `--mark` or `--marks` template may run over several lines, each a marked line. A field
written `{name*:text}` begins a record, as a `ConvertFrom-String` template writes it: a line
holding it starts one, the lines after it join it with the first value of each field kept, and
a line before the first record starts none. The report then ends with the records:

```console
$ trex infer --mark 'Name: {Name*:Phoebe Cat}
Phone: {phone:425-123-6789}' 'Name: Elephant Wise' 'Phone: 425-888-7766' 'Name: Wise Owl'
pattern  ^ "Name" ":" (\W \W):Name ~<($ .) | ^ "Phone" ":" (.*? $ .):phone ~<($ .)
format   ${Name}\t${phone}

shape  lines  reads
1      1 3 5  Name marked; no phone
2      2 4    phone marked; no Name

line  shape  Name           phone         text
1     1      Phoebe Cat     -             Name: Phoebe Cat
2     2      -              425-123-6789  Phone: 425-123-6789
3     1      Elephant Wise  -             Name: Elephant Wise
4     2      -              425-888-7766  Phone: 425-888-7766
5     1      Wise Owl       -             Name: Wise Owl

record  lines  Name           phone
1       1-2    Phoebe Cat     425-123-6789
2       3-4    Elephant Wise  425-888-7766
3       5      Wise Owl       -
```

A mark may span the lines of a template, as a `ConvertFrom-String` template writes one record
of several lines: `{Person*:...}` around a name line and a phone line makes the fields inside
it `Person.Name` and `Person.Phone`. On each line it covers, the mark holds that line's part and
the marks inside it, so a starred one begins its record at the first mark inside it, and the
record's `Person` is the text of each of its lines joined with a newline, which the report
writes as `\n`. A starred mark spanning lines with no mark on its first line is refused, since
nothing then says which line of the input begins its record:

```console
$ trex infer --mark '{Person*:Name: {Name:Phoebe Cat}
Phone: {Phone:425-123-6789}}' 'Name: Wise Owl' 'Phone: 425-888-7766'
pattern  ^ ("Name" ":" (\W \W):Name):Person ~<($ .) | ^ ("Phone" ":" (.*? $ .):Phone):Person ~<($ .)
format   ${Person}\t${Person.Name}\t${Person.Phone}

shape  lines  reads
1      1 3    Person Person.Name marked; no Person.Phone
2      2 4    Person Person.Phone marked; no Person.Name

line  shape  Person               Person.Name  Person.Phone  text
1     1      Name: Phoebe Cat     Phoebe Cat   -             Name: Phoebe Cat
2     2      Phone: 425-123-6789  -            425-123-6789  Phone: 425-123-6789
3     1      Name: Wise Owl       Wise Owl     -             Name: Wise Owl
4     2      Phone: 425-888-7766  -            425-888-7766  Phone: 425-888-7766

record  lines  Person                                 Person.Name  Person.Phone
1       1-2    Name: Phoebe Cat\nPhone: 425-123-6789  Phoebe Cat   425-123-6789
2       3-4    Name: Wise Owl\nPhone: 425-888-7766    Wise Owl     425-888-7766
```

`--lib`, `--shape` and `--shape-after` lex the lines under their declarations, as they do for
`scan`, so a declared shape is one token the pattern names. A pattern built under them reads
only beside them, and its `--lib-file` is read with the file that declares them:

```console
$ trex infer --mark 'see {t:AB-12} now' 'see XYZ-9 now' --pattern
^ "see" (\W "-" \N):t "now" ~<($ .)

$ trex infer --mark 'see {t:AB-12} now' 'see XYZ-9 now' --shape 'ticket = `[A-Z]{2,4}-\d{1,4}`' --pattern
^ "see" (\{ticket}):t "now" ~<($ .)
```

A template line that marks a starred field more than once repeats its record, as a
`ConvertFrom-String` template writes two records on one line. The line is cut at each starred
mark into records that mark the same fields in the same order, and the words between each
record and the next are the separator; the branch reads the record, then the separator and the
record again as often as a line holds them. Each field of the record reads every value in the
line's row, written by `${Name[*]}`, and one value in each record, and a field outside the
records describes the line, so every record of the line carries it. `--json` marks each field
of the record `"repeats"`.

```console
$ trex infer --mark 'day {day:Mon}: {Name*:Phoebe Cat} ({[int]age:6}); {Name*:Lucky Shot} ({[int]age:12}) end' 'day Tue: Wise Owl (87) end' 'day Wed: Elmo Red (3); Oscar Grouch (9); Big Bird (5) end'
pattern  ^ "day" (\W):day ":" (\W \W):Name "(" (\N):age ")" (";" (\W \W):Name "(" (\N):age ")")* "end" ~<($ .)
format   ${day}\t${Name[*]}\t${age[*]}
suggest  day as \{weekday}

shape  lines  reads
1      1-3    day Name age marked

line  shape  day  Name                            age    text
1     1      Mon  Phoebe Cat,Lucky Shot           6,12   day Mon: Phoebe Cat (6); Lucky Shot (12) end
2     1      Tue  Wise Owl                        87     day Tue: Wise Owl (87) end
3     1      Wed  Elmo Red,Oscar Grouch,Big Bird  3,9,5  day Wed: Elmo Red (3); Oscar Grouch (9); Big Bird (5) end

record  lines  day  Name          age
1       1      Mon  Phoebe Cat    6
2       1      Mon  Lucky Shot    12
3       2      Tue  Wise Owl      87
4       3      Wed  Elmo Red      3
5       3      Wed  Oscar Grouch  9
6       3      Wed  Big Bird      5
```

Records that mark different fields, or that stand with no word between them, are refused with
the line, since nothing then says where one record ends and the next begins.

An unmarked column that holds a field's value in every line, while the field takes at least two
values, is written as a back-reference to it, `=tag`, so a line where the two differ is
refused; a column that matched a field only while the field never changed stays its kind:

```console
$ trex infer --mark '<{tag:b}>bold</b>' '<i>it</i>' '<em>x</em>' --pattern
^ "<" (\W):tag ">" \W "<" "/" =tag ">" ~<($ .)
```

A value class of the shipped library, one of its word lists or named sub-patterns
(`\{log_level}`, `\{http_2xx}`, `\{month}`, `\{country}`), is printed in a field's place only
when a `--not` line needs it, as a value range is; otherwise the report names each class every
value of a field belongs to as a `suggest` line, and `--json` as `suggestions`:

```console
$ trex infer --mark '{level:ERROR} disk {n:5}' 'WARN disk 7' --pattern
^ (\W):level "disk" (\N):n ~<($ .)

$ trex infer --mark '{level:ERROR} disk {n:5}' 'WARN disk 7' --not 'HELLO disk 9' --pattern
^ (\{log_level}):level "disk" (\N):n ~<($ .)
```

`--pattern` prints the pattern alone, `--json` the report as one JSON object, and
`--lib-file` the pattern as a file `--lib` reads, its test line accepting a line of each
shape and rejecting each `--not` line. A `--not` line adds the value ranges that exclude it,
as it does for plain `infer`:

```console
$ trex infer --field status=200 'GET /index.html 200 12ms' 'POST /login 302 40ms' 'GET /missing 404 3ms' --pattern
^ \W \L (\N):status \R ~<($ .)

$ trex infer --mark 'GET /a {status:200}' 'GET /b 204' --not 'GET /c 500' --lib-file
# Built by trex infer from 2 lines in 1 shape; `extract` names every shape in order.
let extract_1 = ^ "GET" "/" \W (\N{200..299}):status ~<($ .)
let extract = \{extract_1}
fields extract {status}
test extract accepts "GET /a 200" rejects "GET /c 500"
```

A built pattern is a pattern like any other, so it is built once and applied from then on
without `infer`. The file `--lib-file` writes holds, beside the pattern, a `fields` line keeping
what the marks said beyond it: each field in order, written as its mark with the example text
left out, with its `[type]`, the `*` of a field beginning a record, and the accessor of a field
read from part of a token, as `{host:host}` reads the host of a URL. Saved, the file is read
with `--lib`, and `scan '\{extract}' --fields` prints the records the fields read, as `infer`
reports them; `--json` writes them as an array:

```console
$ trex infer --mark 'GET https://{host:example.com}/a {[int]code:200}' 'GET https://trex.dev/b 404' --lib-file
# Built by trex infer from 2 lines in 1 shape; `extract` names every shape in order.
let extract_1 = ^ "GET" (\U):host (\N):code ~<($ .)
let extract = \{extract_1}
fields extract {host:host} {[int]code}
test extract accepts "GET https://example.com/a 200"

$ cat hosts.trex
let extract_1 = ^ "GET" (\U):host (\N):code ~<($ .)
let extract = \{extract_1}
fields extract {host:host} {[int]code}

$ trex scan '\{extract}' --lib hosts.trex --text 'GET https://example.org/x 500' --fields
record  lines  host         code
1       1      example.org  500

$ trex scan '\{extract}' --lib hosts.trex --text 'GET https://example.org/x 500' --fields --json
[{"lines":[1],"values":{"host":"example.org","code":"500"}}]
```

A field beginning a record makes a record of the lines that follow it, as it does for `infer`;
`--format` with the template the report prints writes the fields one line a match instead.

### index

```
index DIR... [--list] [-g GLOB]... [--hidden] [--no-ignore]
```

Write each tree's index, so a later scan of it opens only the files that can match. The index
is one file, `.trex-index`, at the tree's root, holding a summary of each file: which token
kinds its lex made, a filter over the words it holds, the range its numbers span and the range
its timestamps span. A scan of that tree skips any file every one of its patterns is refused
by, which skips the read, the lex and the walk together.

This is where a tree scan's time actually goes. A pattern led by a kind with no literal to
search for has no byte route: `trex scan '\I' src/` must read and lex every file to learn that
almost none hold an address. Measured over trex's own source in `benches/index_pruning.rs`
(70 files, 3.2 MB): 50x for `\E`, 23x for `\T`, 12x for `\I`, 9x for `\N{>=100000}`, 2x for a
rare word, and 0.99 to 1.02x for a pattern present everywhere, which the index cannot prune and
so costs nothing. The index is 320 bytes a file, 0.0071x the tree it covers.

A summary may say "cannot match" only where that is certain, and "might match" everywhere
else: a wrong "might" costs one wasted read, a wrong "cannot" would be a missed match. So the
kind mask and the ranges are exact, the word filter's only error is a false present, and what
a pattern is taken to require is read conservatively - an alternation requires nothing, since
a match may take the branch that needs least, and a `P*` requires nothing, since it may match
none. An entry records its file's length and modified time, and a file that differs, or that
the index has never seen, is scanned normally. **An index can change how long a scan takes and
never what it reports**, which is what lets it be read automatically.

A scan of a tree holding an index uses it without being asked; `--no-index` ignores it, and
`scan --index` builds one as the scan reads the files, so the tree is indexed by a scan you
were running anyway. A file named on the command line is scanned whatever the index says, as
naming a file is asking for it, and `-v`, `-L` and `--passthru` read no index, since those
report on the files that do not match.

```console
$ trex index tree/
tree/: 5 files indexed
$ trex index tree/ --list
tree/.trex-index: 5 files indexed
$ trex scan '\I' tree/
tree/logs\a.log:1:6: "10.0.0.1"
$ trex scan '\E' tree/
tree/notes\c.txt:1:6: "bob@x.com"
$ trex scan '\I' tree/ --no-index
tree/logs\a.log:1:6: "10.0.0.1"
```

The index is written by `trex index`, and equally by a scan asked to build one:

```console
$ trex scan '\W' tree/ --index --count
tree/: indexed 5 files; later scans of this tree prune with it
tree/counts.txt:2
tree/logs\a.log:2
tree/logs\b.log:2
tree/notes\c.txt:1
tree/notes\d.txt:2
```

A file that has changed since the index was built is scanned, so the report follows the tree
rather than the index. Rewrite one of them and ask again:

```console
$ cat tree/logs/b.log
queue drained at 10.0.0.9
nothing to report
$ trex scan '\I' tree/
tree/logs\a.log:1:6: "10.0.0.1"
tree/logs\b.log:1:18: "10.0.0.9"
```

### lib

```
lib [--json]
lib --test FILE...
```

List the shipped library of named patterns: each entry's name, whether it is a kind the lexer
produces or a sub-pattern the parser inlines, whether a check guards it beyond its shape, and
what it matches. Every entry is reached as `\{name}` with no declaration; the
[pattern syntax](../pattern-syntax/) page describes them and the pattern file that adds your
own.

```console
$ trex lib | head -4
name           form     guard    what
iban           kind     checked  an IBAN, compact or in groups of four, with its country's length and the mod 97-10 check
isbn           kind     checked  an ISBN-10 (mod 11, X as ten) or ISBN-13 (978 or 979, EAN check), hyphens or spaces allowed
vin            kind     checked  a vehicle identification number: seventeen characters without I, O or Q and the check digit ninth
```

`--test FILE` reads a pattern file as `--lib` does and runs its `test` lines: `test NAME
accepts "text"... rejects "text"...` beside the declarations, where the name accepts a text when
its match in the text is the whole of it, from the first significant token to the last, and
rejects a text when it matches nowhere in it. A text is double-quoted and reads `\"`, `\\`,
`\n` and `\t`; the name is any declared shape, kind or sub-pattern or a library entry, tested as
the file finally declares it. Each expectation not met prints as a `FILE:LINE:` line, one line
per file counts its tests, and the exit status is 1 when any test fails. Repeated, the files
declare into one set in order, so a later file's tests may name an earlier file's declarations.

```console
$ cat defs.trex
let rhs = \N | \Q
kind assign = \W "=" \{rhs}
test assign accepts "x = 1" "name = \"bob\"" rejects "x == 1"
test rhs accepts "42" rejects "forty-two"
test iban accepts "GB82 WEST 1234 5698 7654 32" rejects "GB82WEST12345698765433"
$ trex lib --test defs.trex
defs.trex: 3 tests passed
$ cat wrong.trex
let rhs = \N
test rhs accepts "42" "\"bob\"" rejects "4 2"
$ trex lib --test wrong.trex
wrong.trex:2: rhs accepts "\"bob\"": no match
  tokens: quoted "\"bob\""
wrong.trex:2: rhs rejects "4 2": matched "4" at 0..1
wrong.trex: 1 of 1 test failed
```

### grammar

```
grammar (GRAMMAR_FILE | --grammar-text SRC) (FILE | --text STRING) [flags]
```

Parse the input against a token grammar: named rules over the universal token stream, with no
per-language parser. Left-recursive rules encode operator precedence and associativity.

```console
$ trex grammar --grammar-text 'expr := number "+" number' --text '2 + 3'
(expr 2 + 3)
```

A rule is `name := alt | alt`. Symbols within an alternative: `<name>` references another rule;
`"lit"` matches a token by its text; a terminal keyword is a token kind (`number`, `ident`,
`string`, `ip`, `url`, `email`, `time`, `punct`); any symbol takes an EBNF quantifier (`*`,
`+`, `?`); `( a b | c )` is a group; `@p` after an alternative weights it.

| Flag | Effect |
|---|---|
| `--start RULE` | use `RULE` as the start symbol (default: the first rule) |
| `--count` | print the number of derivations |
| `--best` | print the most probable derivation's probability (`@p` weights) |
| `--prob` | print the total probability over all derivations |
| `--segment TEXT --dict FILE` | parse over a segmentation lattice of run-together text |

### bpe

```
bpe train CORPUS [--merges K] [--max-bytes N]
bpe encode (FILE | --text S) --model M
```

Learn a byte-pair-encoding subword tokenizer from a corpus, then segment text with it. `train`
prints the learned merge pairs (tab-separated).

```console
$ printf 'the cat sat on the mat that the hat' > corpus.txt
$ trex bpe train corpus.txt --merges 5
trex bpe: learned 5 merges from 34 bytes in 0.0s
a	t
at	</w>
e	</w>
h	e</w>
t	he</w>
```

### prefilter

```
prefilter (FILE | --text STRING) [--literal L]... [--filter bloom|cuckoo|xor] [--verify]
```

Report which literals might occur, using an approximate-membership filter over the corpus
n-grams. An absent literal is rejected with no corpus scan; a possible hit is confirmed with an
exact search.

```console
$ trex prefilter --text 'the quick brown fox ERROR here' --literal ERROR --literal MISSING
bloom: "ERROR" -> might occur; present (confirmed)
bloom: "MISSING" -> ABSENT (rejected with no corpus scan)
```

`--filter` selects the filter (default `bloom`); `--verify` runs a zero-false-negative check
and exits non-zero if the contract is violated.

### compress

```
compress (FILE | --text STRING) [--compare] [--no-baked] [--prior-cache DIR] [--chunks N] [--gpu] [--hybrid]
```

`compress` comes with the `compress` feature, the one trex feature outside the default build.
A default binary leaves out the coder and the 21.6 MB prior it reads, and answers the command
with the build that has it:

```text
cargo build --release --features compress
```

Report the achieved compression of the default context-mixing coder (logistic mix + orbit,
auto-using the baked prior).

```console
$ trex compress --text 'the quick brown fox jumps over the lazy dog the quick brown fox jumps'
trex compress: 69 bytes -> 15 bytes  (1.626 bits/byte, 21.7% of original)
  coder: logistic mix + orbit + baked prior   0.00 MB/s   (--compare for the full table)
```

`--compare` prints the full coder comparison table - every coder's bits/byte and encode speed,
including the BPE baseline; `--no-baked` runs corpus-free. The coder is a strong PAQ-class
context mixer: it beats BPE outright and sits with bzip2 / brotli / xz on ratio, at a much
slower speed.

Decoding the prior costs every run about 1.9 s and a gigabyte of memory before the first byte
is coded. `--prior-cache DIR` writes it once into DIR as the coder's own tables (1.4 GB for the
shipped prior, 2.4 s on PC2) and maps them on every later run in under a millisecond, coding to
the same bits. Each read of a mapped table costs 1.4-1.7x a decoded one, so the cache pays on
inputs under roughly 7-10 MB and the default decode pays above; the choice is per run.

Three parallel backends trade ratio for speed. `--chunks N` slices the input across N cores
(0 = every logical core), each chunk seeded from the baked prior so the ratio cost stays a few
percent. `--gpu` runs the device coder: one CUDA thread per chunk, thousands in flight, with
overlapping context windows and the baked prior projected into the device's tables, which a
context model reads when its slot misses; `--no-baked` runs it with no prior. `TREX_GPU_CHUNK`
and `TREX_GPU_OVERLAP` tune its chunk size and warmup (bigger chunks, better ratio; more
chunks, more parallelism).
`--hybrid` runs CPU and GPU concurrently on a head/tail split. Sequential CPU remains the best
ratio, which is why none of these is the default - see
[choose a backend](../../how-to/choose-a-backend/#compressing).

## Property-axis analysers

Each reads one axis and prints a summary, plus the per-token field on request. Full detail is
on each axis's [reference page](../axes/).

### spectral

Local entropy, dominant byte-period, texture, and change-points. See
[spectral](../axes/spectral/).

```console
$ trex spectral --text 'the quick brown fox jumps over the lazy dog'
trex spectral: 43 bytes, 3 frames (hop 16), 0 change-points
  entropy   min 0.62  mean 0.68  max 0.73  (normalised bits/byte)
  period    none detected (no strong byte-periodicity)
  texture timeline (merged regions, cut at change-points):
    [       0..43      ] prose  H=0.68 per=0 nov=1.00
```

Flags: `--segment`, `--bands`, `--classify`, `--code-classify`, `--json`.

### seam

Bidirectional predictive segmentation. See [seam](../axes/seam/).

```console
$ trex seam --text 'the cat sat'
trex seam: 11 bytes, order 3, 5 segments (bidirectional branching entropy)
  segments:
    [     0..3     ] "the"
    [     3..4     ] " "
    [     4..7     ] "cat"
    [     7..8     ] " "
    [     8..11    ] "sat"
```

Flags: `--order K`, `--field`, `--segment`, `--recover [--compare-bpe]`, `--english`, `--json`,
plus the `--compress` family (see [seam](../axes/seam/)).

### shape

Silhouette / template structure. See [shape](../axes/shape/).

```console
$ trex shape --text 'foo(a, b) bar(c, d) baz(e, f)'
trex shape: 29 bytes, 18 tokens, 1 template region(s), 2 change-point(s)
  dominant shape-period: 6 tokens  (strength 1.00)
```

Flags: `--classes`, `--period`, `--segment`, `--orbit G`, `--json`.

### orbit

Symmetry: each token mapped to its canonical representative under a group. See
[orbit](../axes/orbit/).

```console
$ trex orbit --collapse --group shape --text 'cat dog bat sat mat the fox'
trex orbit --collapse (group shape): 7 raw forms -> 2 orbits (3.5x reduction)
    "CCV"
    "CVC"  <-  ["bat", "cat", "dog", "fox", "mat", "sat"]
```

Flags: `--group identity|case|notation|shape|e8|ip|url|time|path|fold|numeric`, or a typed
relation such as `subnet/24`, `domain` or `day` (default `shape`), `--collapse`,
`--boundary`, `--match QUERY`.

### magnitude

Value scale, energy, gradient, scale outliers. See [magnitude](../axes/magnitude/).

```console
$ trex magnitude --text 'retries = 3 ; max_bytes = 5000000000'
trex magnitude: 36 bytes, 7 tokens, total energy 112.2, 3 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
```

Flags: `--field`, `--jumps`, `--energy`, `--outliers`.

### stress

Bracket-nesting load, peaks, fractures. See [stress](../axes/stress/).

```console
$ trex stress --text 'f(g(h(x)))'
trex stress: 10 bytes, 10 tokens, max depth 3, 2 peak(s), 1 fracture(s)
  peak load: 10 (depth 2, strain 4)
```

Flags: `--field`, `--peaks`, `--fractures`.

### flow

Slope, direction, momentum of a signal. See [flow](../axes/flow/).

```console
$ trex flow --text '1 10 100 1000 50 5'
trex flow (over magnitude): 18 bytes, 6 tokens, 1 reversal(s)
  peak momentum: 4 (rising)
```

Flags: `--over magnitude|stress|length` (default `magnitude`), `--field`, `--reversals`.

### observe

How vantage-dependent each position's reading is. See [observation](../axes/observation/).

```console
$ trex observe --text 'the old man the boats'
trex observe: 21 bytes, 2 contested point(s)
  peak observer-dependence: 0.35 at byte 15
```

Flags: `--field`, `--contested`.

### echo

The recurrence field: per-token echo count, lags, period, and the document's novelty and
echo rates. See [echo](../axes/echo/).

```console
$ trex echo --text 'the whale swam and the whale sang of the whale'
trex echo: 46 bytes, 19 tokens (10 keyed, 6 distinct), novelty 60.0%, echo rate 60.0%
  strongest echoes (top 2 of 2; first occurrence, count, period):
    the                  x3     period ~18 B
    whale                x3     period ~18 B
```

Flags: `--field` per-token detail; `--orbit case|shape|notation` recurrence up to a symmetry;
`--super` recurring supertoken structures (structural rhyme); `--top K` bounds the ranked
lists.

### relation

The two-point tier: the enclosure, operator, adjacency and reuse relations between tokens,
and the readings over that graph - holonomy, holography, curvature, topology, geodesic
distance, and entanglement across a cut.

```console
$ trex relation --text 'f(g(x)) x'
trex relation: 9 bytes, 9 tokens, max depth 2, nesting load 4
  edges: 3 encloses, 0 operator, 7 adjacent
  holonomy: 2 (1 reuse chord(s), 1 scope-crossing), net -2 (reuse flows outward)
  holography: 4 boundary events, 2 bulk node(s), holographic defect 1 (= reuse chords)
  bulk = 3 enclosure edge(s) rebuilt from the boundary + 1 holonomy chord(s)
  curvature: min -3 (sharpest bottleneck), mean 0.09, 3 bridge edge(s)
  topology: 8 nodes, 11 edges, b0 1 component(s), b1 4 independent loop(s), euler -3
  geodesic: longest reuse shortcut collapses a 4-token span to one hop
  entanglement: peak 2, minimal cut 1 crossing(s) before token 5
```

Flags: `--edges` lists every directed relation and reuse chord; `--field` the per-token
enclosure path and depth; `--gauge` the gauge-fixed (alpha-equivalent) form; `--limit N`
bounds the listings.
