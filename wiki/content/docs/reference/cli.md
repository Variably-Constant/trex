---
title: Command reference
linkTitle: CLI
weight: 10
---

Each `trex` command's synopsis and flags. The examples are on the pages the rows link.

## Invocation

```console
$ trex --version
trex 0.3.0
```

| Concern | Behavior |
|---|---|
| Help | `trex --help` prints every command's usage, and every command prints its own with `-h` or `--help`, exiting `0` |
| Flags | a flag a command does not take is refused by name; `--` ends the flags, so a path, pattern, template or text that begins with a dash goes after it (`trex escape -- -x`, `trex tokens -- -notes.txt`); a lone `-` is the standard input |
| Input | a positional `FILE` or `--text STRING`; `scan`, `rewrite`, `redact`, `templates`, the tables, `head`, `tail` and `lines` take any number of files, directories and `-` for the standard input, and read the standard input when given none ([files and trees](../matching/#files-and-directory-trees)) |
| Encodings | a byte order mark selects UTF-8, UTF-16 or UTF-32, little or big endian; BOM-less UTF-16 is read as such only when the bytes are not valid UTF-8, decode in full and hold no control characters ([several inputs](../windows/#several-inputs-and-encodings)) |
| Clock | `--now TIMESTAMP`, `--tz OFFSET` and `--date-order dmy\|mdy`, given anywhere on any command, set what a timestamp is read against ([typed value predicates](../pattern-syntax/#typed-value-predicates)) |
| Exit status | `0` on success, non-zero on a missing input and where a command states its own failure ([exit status](../matching/#exit-status)) |
| Output | every listing prints in full; `--limit N` bounds a per-token or per-boundary listing and marks what it cut, and a ranked list states its bound and its total |

## scan

```text
scan PATTERN [FILE|DIR|-]... [--text STRING] [flags]
scan -e PATTERN... | -f FILE | --patterns FILE | --rules FILE|DIR  [FILE|DIR|-]... [flags]
scan --files [DIR]... [--texture [KIND]]
```

| Flag | Effect |
|---|---|
| `--text STRING` | scan a string |
| `-e PATTERN`, `--regexp`, `--pattern` | a pattern, repeatable; several are one alternation ([selecting lines](../matching/#selecting-lines)) |
| `-f FILE`, `--file FILE` | patterns from a file, one a line |
| `--patterns FILE` | scan a pattern file's members as one set and name the member of each match ([pattern sets](../matching/#pattern-sets)) |
| `--single-match` | each member's first match per input |
| `--lib FILE\|DIR` | declarations for `\{name}`, a directory as its `.trex` files; repeatable, a file named twice read once ([pattern files](../pattern-files/)) |
| `--shape 'NAME = `BYTES`'`, `--shape-after ...` | declare a token shape tried before, or after, the built-in recognizers ([shapes](../pattern-files/#shapes)) |
| `--kind 'NAME = PATTERN'`, `--let 'NAME = PATTERN'` | declare a kind, each match one token, or a sub-pattern read in place of `\{NAME}` ([kinds and sub-patterns](../pattern-files/#kinds-and-sub-patterns)) |
| `--declare LINE` | declare any line a pattern file holds; each declaration reads the ones given before it, `--lib` among them |
| `--json` | the matches as JSON ([matches](../matching/#matches)) |
| `--format TEMPLATE` | one line a match from a report template ([report templates](../matching/#report-templates)) |
| `--explain` | under each match its kinds, guards, axis readings and route ([explanations](../matching/#explanations)) |
| `--fields` | the records a pattern's `fields` line reads ([saving a build](../building-patterns/#saving-and-reusing-a-build)) |
| `--values exact\|natural\|tagged`, `--duration-unit ns\|ms\|s` | how `--json` spells a typed value ([typed values](../matching/#typed-values)) |
| `-A N`, `-B N`, `-C N` (`--after-context`, `--before-context`, `--context`) | lines, or a record unit, after, before or around a match ([context](../matching/#context)) |
| `--count`, `--count-matches` | lines holding a match, or matches, per input ([counts](../matching/#counts)) |
| `-l`, `--files-with-matches`; `-L`, `--files-without-match` | the inputs with a match, or without one |
| `-v`, `--invert-match` | the lines no match touches, with the lines `-A`, `-B` and `-C` ask for around them ([selecting lines](../matching/#selecting-lines)) |
| `-x`, `--line-regexp` | a match covering its line's significant extent |
| `-o`, `--only-matching` | the matched text alone |
| `-m N`, `--max-count N` | at most N matches an input |
| `--passthru` | every line, the matched ones painted |
| `-H`, `--with-filename`; `--no-filename` | prefix `path:line:col:`, or never |
| `--require-match` | exit non-zero when nothing matches ([exit status](../matching/#exit-status)) |
| `--all`, `--any`, `--none`, `--at-least N` | hold each record to the patterns ([record queries](../records/#record-queries)) |
| `--not PATTERN` | a pattern the record must not hold, repeatable |
| `--record UNIT`, `--record-start PATTERN`, `--record-span PATTERN` | what a record is ([record units](../records/#record-units)) |
| `--rules FILE\|DIR` | report each finding of a file's rules ([rules](../pattern-files/#rules)) |
| `--sarif`, `--github` | the findings as SARIF 2.1.0, or as GitHub annotations |
| `--fix`, `--dry-run`, `-i`, `--interactive`, `-U`, `--update-all` | apply the rules' fixes, print their diffs, or review each |
| `--head N`, `--tail N`, `--lines A..B` | scan only part of each input ([a window of a scan](../windows/#a-window-of-a-scan)) |
| `--follow` | then scan what each file gains ([following a file](../windows/#following-a-file)) |
| `--keep-count` | with `--follow` and `-m`, keep a file's count when it is truncated, replaced or removed |
| `--hidden`, `--no-ignore`, `--binary` | read what the walk skips ([files and trees](../matching/#files-and-directory-trees)) |
| `-g GLOB`, `--glob GLOB` | keep a walked file by a glob, `!GLOB` drops it |
| `-t TYPE`, `--type`; `-T TYPE`, `--type-not`; `--type-list` | keep or drop walked files by type, or list the types |
| `--sort KEY`, `--sortr KEY` | order inputs by `path`, `modified`, `accessed` or `created` |
| `--files` | print the files a scan would read, without scanning them |
| `--texture [KIND]` | keep walked files by their dominant region kind, or name each file's ([texture](../matching/#texture)) |
| `--index`, `--no-index` | build a tree's index during the scan, or read none ([indexes](../tools/#indexes)) |
| `--color WHEN`, `--colors SPEC` | when and how to paint ([color](../matching/#color)) |
| `--stats`, `--stats=line` | the statistics after the report ([statistics](../matching/#statistics)) |
| `--chunk-size N` | feed the input in N-byte chunks ([engines](../matching/#engines)) |
| `--dual-grain` | run the byte and token grains as a pipeline |
| `--gpu`; `--cpu`, `--nogpu` | force the device backend, or the CPU |

## head, tail, lines

```text
head N [FILE|DIR|-]... [flags]
tail N [FILE|DIR|-]... [flags]
lines A..B [FILE|DIR|-]... [flags]
```

| Flag | Effect |
|---|---|
| `--lines A..B` | give `head` or `tail` a range in place of N ([first, last and a range](../windows/#first-last-and-a-range)) |
| `-n`, `--line-number` | each line after its number |
| `-f`, `--follow` | then print what each file gains ([following a file](../windows/#following-a-file)) |
| `--record UNIT`, `--record-start PATTERN`, `--record-span PATTERN` | count records instead of lines ([records](../windows/#records)) |
| `-H`, `--with-filename`; `--no-filename` | head every input with its name, or none |
| `--hidden`, `--no-ignore`, `--binary` | read what the walk skips |
| `--color WHEN`, `--colors SPEC` | when and how to paint |

## rewrite

```text
rewrite PATTERN TEMPLATE (FILE | --text STRING)
rewrite PATTERN TEMPLATE [FILE|DIR|-]... (--in-place | --dry-run | --interactive) [flags]
```

| Flag | Effect |
|---|---|
| `--text STRING` | rewrite a string ([templates](../rewriting/#templates)) |
| `--in-place` | write each file with a match back ([files](../rewriting/#files)) |
| `--dry-run` | print the unified diff instead |
| `-C N`, `--context N` | context lines in the diff |
| `-i`, `--interactive` | review each change ([review](../rewriting/#review)) |
| `-U`, `--update-all` | apply every change without asking |
| `--show-skipped` | under a review, name the places a template-wide skip passed over |
| `--explain` | under a review, put under each diff what `scan --explain` prints |
| `-m N`, `--max-count N` | rewrite the first N matches of each input alone |
| `--lib FILE\|DIR`, `--shape ...`, `--shape-after ...`, `--kind ...`, `--let ...`, `--declare ...` | declarations for `\{name}` ([declared atoms](../rewriting/#declared-atoms)) |
| `--head N`, `--tail N`, `--lines A..B`, `--follow` | rewrite part of each input, or what it gains ([a window of a scan](../windows/#a-window-of-a-scan)) |
| `--keep-count` | keep a followed file's `-m` count when it is truncated, replaced or removed |
| `--record UNIT`, `--record-start PATTERN`, `--record-span PATTERN` | the unit a window counts |
| `--hidden`, `--no-ignore`, `--binary` | read what the walk skips |
| `--gpu`; `--cpu`, `--nogpu` | force the engine that finds the matches, for a file written back, diffed or reviewed as for the standard output; `--follow` takes no `--gpu` |

## redact

```text
redact PATTERN [FILE|DIR|-]... [--text STRING] [flags]
```

| Flag | Effect |
|---|---|
| `--keep 'NAME:ACC, ...'` | leave the named fields unmasked ([mask and keep](../redaction/#mask-and-keep)) |
| `--mask C\|TOKEN\|shape\|pseudonym` | another mask character, a token per run, the run's shape, or a stable name ([other masks](../redaction/#other-masks)) |
| `--text STRING` | redact a string |
| `--in-place`, `--dry-run`, `-C N` | write back, or print the diff ([files](../redaction/#files)) |
| `-i`, `--interactive`, `-U`, `--update-all`, `--show-skipped`, `--explain` | review each masking, or apply every one, as `rewrite` does |
| `--lib FILE\|DIR`, `--shape ...`, `--shape-after ...`, `--kind ...`, `--let ...`, `--declare ...` | declarations |
| `--head N`, `--tail N`, `--lines A..B`, `--follow` | redact part of each input ([part of a file](../redaction/#part-of-a-file)) |
| `--record UNIT`, `--record-start PATTERN`, `--record-span PATTERN` | the unit a window counts |
| `--hidden`, `--no-ignore`, `--binary` | read what the walk skips |

## count-by, top, uniq

```text
count-by PATTERN KEY [FILE|DIR|-]... [--text STRING] [flags]
count-by --patterns FILE KEY [FILE|DIR|-]... [flags]
top PATTERN KEY ...
uniq PATTERN KEY ...
```

`count-by` orders by key, `top` by count, `uniq` prints the keys alone ([count by a key](../aggregates/#count-by-a-key)).

| Flag | Effect |
|---|---|
| `--patterns FILE` | count a pattern file's members ([keys by place and by member](../aggregates/#keys-by-place-and-by-member)) |
| `--sum ${c}`, `--avg ${c}`, `--min ${c}`, `--max ${c}` | a column over one capture, each repeatable ([aggregates](../aggregates/#aggregates)) |
| `--pN ${c}` | the Nth percentile of a capture, N from 0 to 100: `--p50`, `--p95`, `--p99` |
| `--percentile nearest\|linear\|lower\|hybrid` | how a percentile is decided when its rank is between two values ([averages and percentiles](../aggregates/#averages-and-percentiles)) |
| `--avg-form repetend\|rational` | how an exact average is written |
| `-n N` | the first N rows |
| `--json`, `--values ...`, `--duration-unit ...` | the table as JSON ([JSON, windows and binary files](../aggregates/#json-windows-and-binary-files)) |
| `--lib FILE\|DIR`, `--shape ...`, `--shape-after ...`, `--kind ...`, `--let ...`, `--declare ...` | declarations |
| `--head N`, `--tail N`, `--lines A..B` | count part of each input |
| `--record UNIT`, `--record-start PATTERN`, `--record-span PATTERN` | the unit a window counts |
| `--hidden`, `--no-ignore`, `--binary` | read what the walk skips, as a scan does ([binary files](../aggregates/#json-windows-and-binary-files)) |

## templates

```text
templates [FILE|DIR|-]... [--text STRING] [flags]
```

| Flag | Effect |
|---|---|
| `--pattern` | each template as a pattern ([templates](../building-patterns/#templates)) |
| `--rare`, `--cut N\|P%` | the rare templates, and where rare ends ([rare templates](../building-patterns/#rare-templates)) |
| `--record UNIT`, `--record-start PATTERN`, `--record-span PATTERN` | group records ([records of several lines](../building-patterns/#records-of-several-lines)) |
| `--lib FILE\|DIR`, `--shape ...`, `--shape-after ...`, `--kind ...`, `--let ...`, `--declare ...` | declarations a `--record-start` or `--record-span` pattern reads |
| `--against FILE\|DIR`, `--novel` | mark each template shared or novel, or print the novel ([against another input](../building-patterns/#against-another-input)) |
| `--json` | the templates as JSON: `count`, `records`, `readable`, `pattern`, `rare`, `novel` (`null` without `--against`) and `covered`, the records that had a template |
| `--head N`, `--tail N`, `--lines A..B` | mine part of each input, in records |
| `--hidden`, `--no-ignore`, `--binary` | read what the walk skips; a binary file named alone is refused, as a scan refuses one |

## infer

```text
infer EXAMPLE EXAMPLE... [--not EXAMPLE]... [-f FILE] [--anchored]
infer --mark LINE... [--marks FILE] [--field NAME=VALUE]... [LINE... | -f FILE] [flags]
```

| Flag | Effect |
|---|---|
| `-f FILE`, `--file FILE` | examples or lines from a file ([a pattern from examples](../building-patterns/#a-pattern-from-examples)) |
| `--not EXAMPLE` | an example the pattern must miss ([value ranges](../building-patterns/#value-ranges)) |
| `--anchored`; `--unanchored` | match whole lines, or not, where the form defaults otherwise |
| `--mark LINE`, `--marks FILE` | lines with each field marked `{name:text}` ([a pattern that extracts fields](../building-patterns/#a-pattern-that-extracts-fields)) |
| `--field NAME=VALUE` | a field named by one value it takes |
| `--marked` | read marks in the other lines too |
| `--no-mint`; `--mint-shapes` | spell a field as its kind, or declare its byte shape ([how a field is spelled](../building-patterns/#how-a-field-is-spelled)) |
| `--lib FILE\|DIR`, `--shape ...`, `--shape-after ...`, `--kind ...`, `--let ...`, `--declare ...` | lex the lines under declarations ([declared shapes](../building-patterns/#declared-shapes)) |
| `--pattern`, `--json`, `--lib-file` | the pattern alone, the report as JSON, or a pattern file ([saving a build](../building-patterns/#saving-and-reusing-a-build)) |

## index

```text
index DIR... [--list] [-g GLOB]... [--hidden] [--no-ignore]
```

Write each tree's `.trex-index`, which a later scan reads to skip files that cannot match
([indexes](../tools/#indexes)). `--list` reports an index, `-g`/`--glob` keeps files by a glob,
and `--hidden` and `--no-ignore` widen the walk.

## tokens, escape

```text
tokens (FILE | --text STRING) [--whitespace] [--lib FILE|DIR] [--shape|--shape-after|--kind|--let DECL] [--declare LINE] [--json] [--binary]
escape TEXT
```

`tokens` prints each token of the input as `[start..end] kind "text"`, in byte offsets
([tokens](../records/#tokens)); `--whitespace` keeps the whitespace tokens, `--lib` and the
declarations `scan` takes lex under what they declare, `--json` writes each token's value too, and
`--binary` reads a file holding a NUL byte. `escape` prints the pattern that matches TEXT literally
([a text taken literally](../pattern-syntax/#a-text-taken-literally)).

## lib

```text
lib [--json]
lib --test [FILE|DIR]...
```

The shipped library, or as JSON ([the shipped library](../pattern-files/#the-shipped-library));
`--test` runs the `test` lines of pattern files, a directory read as its `.trex` files, or the
library's own where none is named ([tests](../pattern-files/#tests)).

## grammar

```text
grammar (GRAMMAR_FILE | --grammar-text SRC) (FILE | --text STRING) [flags]
```

| Flag | Effect |
|---|---|
| `--start RULE` | the start symbol; the first rule otherwise ([grammars](../tools/#grammars)) |
| `--count` | the number of derivations |
| `--best` | the most probable derivation's probability |
| `--prob` | the total probability over every derivation |
| `--segment TEXT --dict FILE` | parse over the segmentations of run-together text ([segmentation](../tools/#segmentation)) |

## bpe

```text
bpe train CORPUS [--merges K] [--max-bytes N]
bpe encode (FILE | --text STRING) --model MODEL
```

`train` learns `K` merges from the first `N` bytes of a corpus and prints them; `encode`
segments text with a learned model ([byte-pair encoders](../tools/#byte-pair-encoders)).

## prefilter

```text
prefilter (FILE | --text STRING) [--literal L]... [--filter bloom|cuckoo|xor] [--verify]
```

Which literals might occur, from an approximate-membership filter over the input's n-grams;
`--verify` checks the filter reports every literal present ([prefilters](../tools/#prefilters)).

## compress

```text
compress (FILE | --text STRING) [flags]
```

Needs a build with the `compress` feature ([compression](../tools/#compression)).

| Flag | Effect |
|---|---|
| `--compare` | every coder's bits per byte and speed |
| `--no-baked` | code with no prior; `--baked`, the default, codes with it |
| `--prior-cache DIR` | map the prior from DIR, writing it there on the first run |
| `--max-bytes N` | code the first N bytes |
| `--chunks N` | code N slices across the cores, `0` for every core |
| `--gpu`, `--hybrid` | code on the device, or on the device and the CPU at once |
| `--second-mixer` | a second mixer weighted by the stream's rhythm, on the CPU coder |

## Axis commands

Each reads one [axis](../axes/) of a `FILE` or `--text STRING`, prints a summary, and adds the
listings its flags ask for.

| Command | Flags |
|---|---|
| [`spectral`](../axes/spectral/) | `--segment`, `--bands`, `--timeline`, `--classify`, `--cp-threshold K`, `--cp-floor F`, `--cp-min-gap N`, `--json`, `--limit N` |
| [`seam`](../axes/seam/) | `--english`, `--order K`, `--passes N`, `--segment`, `--field`, `--cut-threshold K`, `--json`, `--limit N`, `--recover [FILE]`, `--compare-bpe`, `--words N`, `--seed N`, `--max-bytes N`, `--grain-separation` |
| [`shape`](../axes/shape/) | `--classes`, `--period`, `--segment`, `--group G`, `--template-strength S`, `--json`, `--limit N` |
| [`orbit`](../axes/orbit/) | `--group G`, `--collapse`, `--boundary`, `--same-as TEXT`, `--limit N` |
| [`magnitude`](../axes/magnitude/) | `--field`, `--jumps`, `--energy`, `--top K` (the `--energy` list, 3 by default), `--outliers`, `--jump-threshold K`, `--outlier-sigma S`, `--limit N` |
| [`stress`](../axes/stress/) | `--field`, `--peaks`, `--fractures`, `--peak-min-depth D`, `--fracture-min-depth D`, `--limit N` |
| [`flow`](../axes/flow/) | `--signal magnitude\|stress\|length`, `--grain token\|super`, `--field`, `--reversals`, `--analytic`, `--window N`, `--steady-band B`, `--limit N` |
| [`observe`](../axes/observation/) | `--grain byte\|token\|super`, `--field`, `--contested`, `--contested-threshold T`, `--contested-min-gap N` |
| [`echo`](../axes/echo/) | `--field`, `--group G`, `--structure`, `--top K` (8 by default), `--max-period-cv C`, `--limit N` |
| [`relation`](../axes/relation/) | `--edges`, `--field`, `--canonical`, `--limit N` |
| [`gravity`](../axes/gravity/) | `--grain byte\|token\|super`, `--field`, `--strain`, `--bound`, `--classes`, `--top K` (8 by default), `--limit N` |
| [`context`](../axes/context/) | `--fold window\|unit\|units\|enclosing\|echo\|regime\|phase\|key`, `--field`, `--period`, `--agreement`, `--token-window N`, `--unit-window N`, `--limit N` |

The commands that read tokens - every one above but `spectral` and `seam` - also take the
declarations a scan takes, `--lib FILE|DIR`, `--shape ...`, `--shape-after ...`, `--kind ...`,
`--let ...` and `--declare ...`, and read the input as those declarations lex it. `observe` and
`gravity` take them at the token and [supertoken](../../explanation/architecture/#supertokens) grains; the byte grain reads no tokens and
refuses them.

`seam` takes, in a build with the `compress` feature, the coder's flags:

| Flag | Effect |
|---|---|
| `--compress` | the code length of the input under each coder |
| `--warm-file FILE`, `--warm-bytes N` | code after the first `N` bytes of `FILE` |
| `--bench-coder`, `--chunks N` | time the shipped coder, over N slices |
| `--archive-dir DIR` | the code length of a directory's files coded apart and coded as one stream |
| `--build-model CORPUS OUT` | train the byte model on a corpus and write it |
| `--model-vigilance RHO`, `--model-topk K` | drop the rows a trained model's backoff already predicts, then keep the K most counted (600,000 by default) |
| `--merge-model A B WEIGHT OUT` | add B's counts, scaled by WEIGHT, to A's |
| `--prune-model IN OUT RHO` | drop the rows of a written model that its backoff already predicts |
