# Changelog

All notable changes to trex are recorded here. The Rust crate (`trex-re` on
crates.io), the Python package (`trex-re` on PyPI) and the PowerShell module
(`Trex` on the PowerShell Gallery) share one version number and release
together. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). A version heading
links to the GitHub release that published it, where there is one. The
repository ships as a single commit that is rewritten on every release, so
this file, not the commit log, is the record of what came before.

Each version after 0.2.0 groups its changes by where they show. Every
surface holds a change in the shared core that the command line, Rust,
Python and PowerShell all see. Command line, Rust, Python and PowerShell
each hold what changed for that surface, even when the cause is in the
core, so a reader of one surface finds everything that touches it in one
place. Documentation holds changes to the documentation alone. Within
each, changes are listed as Breaking, Added, Changed, Removed and Fixed.
0.2.0 and earlier list their changes by kind alone.

## [0.3.0] - 2026-10-05

### Every surface

#### Breaking

- The unit above the token is called a supertoken everywhere: a template's
  `${@construct}` is `${@super}`, an explanation reads `call supertoken at
  depth 0`, and the axes a refused template lists name `super`.
- Bare `@seam` and `@seam:super` do not hold at the input's first unit, as
  `@seam:byte` and every written cut do not; they held there before.
- The pair field's bound of a cut is the mean `log2 g` over the pairs of
  units straddling it, in bits per pair, where it was their sum. A cut near
  either end of the input, which fewer pairs straddle, reads on the scale
  every other cut reads on, so `@bound` and `--record bind:Q` no longer
  favor the input's ends; on a short log the weakest cuts are its odd lines
  where they were its first and last tokens. A cut more than eight units
  from both ends keeps its order, and a bound written in bits
  (`@bound<=-1b`) reads per pair.
- A run of more than 32 hex digits at a length no digest has lexes as one
  `hex` token; it lexed as a word, a number and a word, or base64. `hex` is
  a built-in atom name, so a declaration named `hex` is refused. A declared
  kind's code is 34 and up, and an index an earlier trex wrote reads as
  absent until `trex index` writes it again.

#### Added

- A `hex` token kind, `\{hex}`: a run of more than 32 hex digits holding a
  hex letter at a length no digest has, as a key, a dump or a digest of
  another size is written; 32, 40 and 64 digits stay `\{hash}`. It is
  painted red and underlined with the other alarm kinds, a scan for
  `\{hex}` over an input with no such run answers without lexing, and the
  GPU matches it as the CPU does.
- A written cut on `@seam` and `@ambiguous`: `@seam>k`, `@seam:byte>=k` and
  `@seam:super>k` keep the cuts whose strength is more than `k`
  standard deviations above the input's mean, or at least `k`;
  `@ambiguous>k` and `@ambiguous:token>=k` the points whose disagreement
  passes `k`, from 0 to 1. An explanation repeats the cut as written (`on
  at a token cut >0.5`) and holds it in a `cut` piece, `${@seam.cut}` and
  `${@ambiguous.cut}`.
- `TREX_REQUIRE_CUDA=1` fails a build for Windows or Linux whose CUDA kernel
  did not compile, naming why; without it such a build warns and runs on the
  CPU. trex's own release builds set it.

#### Changed

- A number written with an exponent (`6.02e23`, `2.5e-4`), a `0x`, `0b`
  or `0o` prefix (`0x3e8`), or `_` before each further group of three
  digits (`1_000_000`, `0xFFFF_FFFF`) lexes as one number token on every
  surface and the device; it lexed as a number and the word after it.
  Magnitude reads it at its value's scale, so `6.02e23` reads 23.78 where
  it read 0.78, and `\N` predicates, typed values, aggregates, the
  `numeric` orbit group and a tree index read its value. A comma still
  separates, so `1,000` is three tokens, and a form with a letter, digit
  or underscore right after it (`1e3a`, `2024_01_02`) lexes as before.
  Readings of any input holding these forms change.
- A number's exact value is held as its digits and a power of ten, so a
  huge one costs a comparison, a sort or a key what its written digits
  cost, and prints in that form where writing it out would put more than
  21 zeros past its digits: `1e999999` prints as `1e999999` in a `--json`
  value, an aggregate and an orbit or echo key, as does a number written
  out with 22 or more trailing zeros.
- A declaration takes every line a pattern file holds, `fields` and a
  rule's `fix`, `meta`, `files`, `unless`, `record`, `record-start` and
  `record-span` among them, and removing an atom takes the lines declared
  that add to it. A `test` line declared on its own is no longer listed as
  an atom.
- SARIF output declares its columns as `"columnKind": "unicodeCodePoints"`,
  which SARIF 2.1.0 requires of a run holding results.
- Of two contested points closer than the observation's minimum gap, the
  stronger is kept, a tie keeping the earlier; the first was kept before.
  The strongest point is always kept, and a higher threshold removes only
  points below it.
- A shape template region ends at its last token whose kind matches the
  kind one period before.
- Region classification (`trex spectral --classify`, `scan --files
  --texture` and the `--texture` filter, `Measure-TrexSpectral -Classify`
  and `trex.axes.spectral(classify=True)`): a region is a blob where base64,
  hash and hex tokens and high-entropy runs cover more than half of it, and
  otherwise a table only where template runs holding a kind other than a
  word cover more than half of it. A run repeating every one or two tokens
  counts toward a table only where its lines hold about as many tokens as
  each other, so a column of numbers one to a line is a table and a
  paragraph of clauses and commas is not. A paragraph whose words repeat
  reads as prose, and base64 of English text as a blob, on one line or
  wrapped.
- The shape axis reads a word's case and length from its characters, so
  `Επειδή` is Pascal as `Whereas` is and a Korean word of two syllables is
  short, and a punctuation token's glyph is its whole character, so `、` and
  `。` are two classes and echo's structural-rhyme keys print them; it read
  ASCII case, length in bytes and the glyph's first byte. Shape classes,
  periods, templates and change points, the context axis and echo's keys
  change on text outside ASCII; ASCII text reads as it did.
- A spectral change point also marks where a span's texture gives way to
  the texture before it, so a line of code in the middle of a sentence has
  one at each edge (`trex spectral --segment`, `\F{onset}`, the regions of
  `--classify`, `Measure-TrexSpectral` and `trex.axes.spectral`). The exit
  is at the first of twelve bytes in a row on which the recent reading
  stays close to the texture the span interrupted; before, a span shorter
  than the reader's slow clock had an entry and no exit, and the regions
  after it read as one with the span.
- The spectral change-point threshold's running mean and deviation take
  each byte's departure only up to three deviations, so a divergence that
  rises over many bytes, as code inside Chinese, Japanese or Korean prose
  does, no longer lifts the threshold ahead of itself; such a span was
  often given no change point at all. Change points move on most inputs,
  and plain text holds a few more of them.
- The CUDA kernel compiles with an `nvcc` of CUDA 12.0 or later; with an
  older one the build warns, as it does with none, and runs on the CPU.
  trex's releases are built with CUDA 12.0, and their kernel loads on an
  NVIDIA driver of the 525 series or later.
- Release builds of the `trex` binary and the Python extension use no
  link-time optimization, and the PowerShell module uses thin link-time
  optimization. Measured against fat link-time optimization under Windows
  on an AMD Ryzen 9 7900X, the binary is 3% smaller and scans in the same
  time, and a scan takes 11% less time in the extension and 6% less in the
  module.
- Error messages, help text and `--explain` readings use plain wording.
  Messages a script may match read: a `~k` group's count "must be less than
  the number of atoms"; "an orbit scope cannot be nested inside another"; in
  a `test` line, "`in` goes between the span ... and the text it is read
  from"; an empty mark "holds no text, so nothing places the field"; a
  template field "is one value and takes no .PIECE after it" or "is not a
  reading AXIS gives; it gives ..."; and `@order` on the first timestamp
  "the first timestamp, with nothing to compare with".

#### Fixed

- The pair field's first unit, with nothing before it, reads no strain and
  no bound: `@strain` and `@bound` hold nowhere there at any grain,
  `--explain` says there is nothing before it to strain against, a
  percentile is taken over the units that have a reading, and `--record
  bind:Q` cuts at the weakest Q% of those. Both read zero there before, so
  `@bound<10` held at the first token of most code files.
- A byte-pattern refuses a backslash before a letter other than `d`, `D`,
  `w`, `W`, `s`, `S`, `p` and `P`, `(?` at the start of a group and a
  backslash inside a class, each a parse error naming it, in a shape and
  between backticks. They parsed as other characters before: `\x00` was the
  characters `x00`, `(?s:.)` the characters `?s:` then one more, and
  `[\x00-\xff]` a class of punctuation, digits and capitals.
- Prose in a script written outside ASCII reads as prose: the spectral
  texture readings (`--classify`, the `--texture` filters, `-Classify`,
  `classify=` and `\F{texture:...}`), observation's byte grain and
  `resonator::over_bytes` read their bytes as UTF-8. A letter outside ASCII
  is a letter on each of its bytes, any other character outside ASCII
  counts once by its class, so `’` weighs what `'` does, and only a byte
  that is not part of a well-formed character is read apart. They read
  Cyrillic, Greek, Chinese and Japanese prose as a blob before, and the
  accents inside Latin words as foreign bytes. The texture readings'
  entropy gates rise with the share of bytes inside multi-byte characters,
  whose bytes carry more entropy than the text they spell. Readings of any
  text outside ASCII change; ASCII text reads as it did.
- The lexer's blob gate takes the same share into account, so a Chinese or
  Japanese sentence lexes as words and punctuation; a sentence of a real
  Chinese document collapsed into one opaque `other` token, and no scan
  could find a word inside it.
- The region classification reads Arabic, Chinese, Greek, Japanese, Korean
  and Thai prose as prose; it read the Universal Declaration of Human Rights
  in each as a table, since their words were in one or two shape classes
  that repeat at any period. Base64 wrapped at 76 columns, digests one to a
  line and PEM certificate bundles read as a blob; they read as a table, one
  token kind repeating line after line.
- The Windows and Linux Python wheels, the Linux binary and the PowerShell
  module's Linux native carry the CUDA kernel, as the Windows binary and the
  module's Windows native did; they ran every scan on the CPU. The macOS and
  FreeBSD ones run on the CPU, since NVIDIA ships no CUDA toolkit for either.

### Command line

#### Breaking

- Flags take the name their other surfaces already use: `trex echo --super`
  is `--structure`, `trex echo` and `trex shape --orbit G` are `--group G`,
  `trex orbit --match` is `--same-as`, `trex relation --gauge` is
  `--canonical` and prints its section as `canonical form
  (alpha-equivalence)`, `trex spectral --code-classify` is `--classify` and
  the old `--classify`, the texture timeline, is `--timeline`, and `trex
  seam --grain` is `--grain-separation`.
- `trex flow` takes `--signal magnitude|stress|length` in place of `--over`,
  and its header reads `trex flow (signal magnitude)`, as Python's `signal=`,
  PowerShell's `-Signal` and the report's `signal` field name it.
- A flag a command does not take is refused by name; the commands read it
  as a file name, a pattern or a text before. A path that begins with a
  dash goes after `--`.
- `trex templates --json` writes `readable` in place of `template`, and
  `novel` as `true`, `false` or `null` without `--against` in place of
  `"mark": "shared"` or `"novel"`.
- `--show-skipped` and `--explain` without `--interactive` are refused on
  `trex rewrite`, as `-ShowSkipped` without `-Interactive` is; they did
  nothing before.

#### Added

- `trex tokens` lists each token of a file or `--text` string as
  `[start..end] kind "text"`, or as JSON with each token's value, and
  `trex escape TEXT` prints the pattern that matches TEXT literally.
- Every command answers `-h` and `--help` with its own usage on the
  standard output and exits 0, and `--` ends the flags, so a path, pattern,
  template or text that begins with a dash goes after it.
- `--kind 'NAME = PATTERN'`, `--let 'NAME = PATTERN'` and `--declare LINE`
  beside `--shape` and `--shape-after` on every command that takes
  `--lib`: `scan`, `count-by`, `top`, `uniq`, `redact`, `infer`, `rewrite`
  and `tokens`, the last two gaining `--shape` and `--shape-after` too.
  Each declares the pattern file line it stands for, read in order with
  `--lib`.
- `--lib DIR` and `trex lib --test DIR` read a directory as every `.trex`
  file under it, in path order.
- `--pN` on `count-by`, `top` and `uniq` asks for any percentile, `--p0`
  through `--p100`.
- `--keep-count` on `scan --follow -m`: a followed file truncated, replaced
  or removed keeps its count.
- `-v` takes `-A`, `-B` and `-C`, printing the lines around each line it
  selects as `grep -v` does.
- `trex grammar --segment TEXT --dict FILE --list` prints each tiling the
  grammar accepts with its words, its best parse's probability and its
  parse count, most probable first.
- `trex templates` takes `--lib` and the declaration flags for its
  `--record-start` and `--record-span` patterns, and its `--json` carries
  `covered`, the records that had a template.
- A redaction is reviewed as a rewrite is: `trex redact --interactive`,
  `-U`, `--show-skipped` and `--explain`.
- `trex rewrite -m N` rewrites the first N matches of each input, with
  `--keep-count` under `--follow`.
- Every axis's own threshold is a flag named as its configuration field:
  `magnitude --jump-threshold` and `--outlier-sigma`, `stress
  --peak-min-depth` and `--fracture-min-depth`, `flow --window` and
  `--steady-band`, `observe --contested-threshold` and
  `--contested-min-gap`, `spectral --cp-threshold`, `--cp-floor` and
  `--cp-min-gap`, `shape --template-strength`, `echo --max-period-cv` and
  `seam --cut-threshold`.
- `trex flow --grain token|super` and `--analytic`, every unit's amplitude,
  phase and frequency with the filter's latency, and `trex observe --grain
  byte|token|super`.
- The axis commands that read tokens take declarations: `--lib`,
  `--shape`, `--shape-after`, `--kind`, `--let` and `--declare` on
  `magnitude`, `stress`, `flow`, `observe`, `relation`, `echo`, `orbit` and
  `shape`. `observe` reads them at the token and supertoken grains and
  refuses them at the byte grain.
- `trex gravity` reads the pair field at the token, byte or supertoken
  grain: every unit's strain and the bound of the cut before it in bits,
  each with its percentile among the input's readings, and its type's
  gravity class; the units under the most strain and the cuts held together
  least (`--top K`); and each gravity class with its types. It quotes a
  byte or a type where a summary line or a class list names it (`'a'`,
  `','`, `'\n'`, `'Punct ,'`), and `--field` gives every unit.
- `trex context` reads each token against a context, the fold `\N{>+1:F}`
  names (`--fold window|unit|units|enclosing|echo|regime|phase|key`): how
  many tokens it folds and each axis's reading over them, with the record
  period and its live periods, and each supertoken's role and how far the
  lower grains' nearest boundaries are from its start (`--period`,
  `--agreement`); `--token-window` and `--unit-window` set the windows.
- `trex seam --field` prints the strength of a cut before each byte, `str=`,
  and `--json` carries it as `boundary`, as `Measure-TrexSeam -Detail` and
  `trex.axes.seam(detail=True)` give it.
- `trex shape --classes` prints each token's period strength, which
  `--template-strength` is set against, as `str=` beside the period, and
  `trex shape --json` carries every token's frame (its span, class, period,
  period strength and novelty) with the dominant period and its strength.

#### Changed

- `trex scan`, `head`, `tail`, `lines`, `templates` and `infer` print their
  `--help` on the standard output and exit 0.
- The command line declares `--lib` and the declaration flags through
  `trex::Declarations`, as Python's `trex.Library` and PowerShell's
  `Trex.Library` do: a refused declaration reads as it does there
  (`line 1: ...` in place of `token shape "...": ...`), and a pattern file
  named twice to `--lib` or `trex lib --test` is read once; it was refused
  before.
- `count-by`, `top` and `uniq` pass over a file holding a NUL byte as a
  scan does, refusing one named alone, unless `--binary` asks for it; they
  counted such a file's matches before.
- `trex templates` refuses a file holding a NUL byte named alone, as a scan
  does, failing with its name; it mined nothing from it before.
- A review's notices name the command reviewing: `trex scan --fix -i` and
  `trex redact -i` say `trex scan:` and `trex redact:`, where every review
  said `trex rewrite:`.

#### Fixed

- The Windows binary is built for any x86-64 CPU and picks its wide kernels
  at run time, as the other binaries, the wheels and the module do; the
  0.2.0 one was built for the AVX-512 CPU of the machine that built it.
- `trex rewrite --gpu` with `--in-place`, `--dry-run` or `--interactive`
  found its matches on the CPU and said nothing; it finds them on the
  device, and says what ran once a run, as a rewrite to the standard output
  does.
- `scan --rules` with `--count` or `-l` over `--tail` stopped with a panic
  at the first finding; a rule's message is placed only when it writes its
  match's position, and the lines and offsets before the tail are counted
  only then.
- A context side given a count beside a side naming a record unit keeps
  its count: `-B block -A 1` prints one line after the match, and
  `-B 1 -A block` one line before it; the command line printed the unit's
  side alone, as PowerShell's `-Context block, 1` did not.
- An axis listing `--limit` cuts says how much it cut, `... (+N more
  tokens; raise --limit)`, as the usage and the command reference say:
  `magnitude`, `stress` and `flow --field`, `flow --analytic`, `echo
  --field`, `relation --edges` and `--field`, `shape --classes`,
  `spectral --bands` and every `orbit` listing cut in silence. `trex
  spectral --bands` printed `... (+N more frames)` after every frame of an
  input of more than 256, and `relation --field --limit N` counted against
  N the tokens outside every bracket it does not print.
- `trex echo` refuses `--group` with no group after it, as `trex shape` and
  `trex orbit` do; it read the keys exactly and said nothing.
- The usage of `trex shape`, `trex echo` and `trex orbit` names every group
  `--group` takes, `e8` and the typed relations such as `subnet/24` among
  them; each command took them all while its usage named four, ten or
  eleven.
- A value past a double's range prints exactly in natural-spelling JSON
  (`--values natural`), `1e400` for a 1 and 400 zeros; it printed `inf`,
  which is not JSON.
- `trex spectral --classify` starts a region's preview at the start of the
  character holding the region's first byte and ends it before a character
  the 28-byte cut would split; it printed a split character's bytes as
  U+FFFD.

### Rust

#### Breaking

- `gravity::strain`, `gravity::binding` and `Readings::strain` and
  `binding` give `Option<f32>`, `None` at the first unit, which reads no
  strain and no bound.
- `ReportAt` carries the match's `offsets` in the surface's unit in place of
  a byte `base`, and `RuleScan::stream` and `SharedRuleStream::new` take the
  unit a surface counts in and how many of it come before the stream.

#### Added

- `trex::aggregate`, the table `count-by`, `Group-TrexMatch` and Python's
  `group_by` build: `Table`, `columns`, `Order`, `Row`; and
  `PatternSet::capture_names` and `capture_kinds` across a set's members.
- `trex::declarations` (`Declarations`, `pattern_files`), the declarations
  PowerShell's `Trex.Library` and Python's `trex.Library` hold;
  `trex::review`, the review every `--interactive`, `-Interactive` and
  `review=` runs; `ShapeSet::kind_name`;
  `trex::window::read_file_with_chars`; and
  `trex::rule_scan::SharedRuleStream`.
- `trex::Engine`, `trex::Ran` and `trex::scan_engine`, the one place the
  command line, PowerShell and Python choose how a scan runs: a forced
  device, the dual-grain pipeline, chunks, or the routed backend.
- `trex::stats` (`ScanStats`, `Route`, `Counting`), one count of what scans
  read and found, which the command's `--stats`, PowerShell's `-Stats` and
  Python's `trex.scan_stats()` all read.
- `trex::EditStream`, the edit of a followed file every surface runs, and
  `trex::rewrite::edits_by`, the edits every surface rewrites a file with.
- `trex::encoding::OffsetUnit`, `Offsets` and `char_start`, a UTF-8 text's
  offsets counted in bytes, code points or UTF-16 code units;
  `trex::files::LineCursor`; `LineIndex::counting` and `offsets`; and
  `RuleScan::messages_read_place` and `messages_read_offsets`.
- `trex::encoding::complete_prefix`, how much of a piece of UTF-8 ends on a
  whole character.
- `trex::report::grep_lines` and `GrepLine`, the lines a grep selects and
  carries as context; `Context::lines` and `any`; `untouched_lines`; and
  `covers_its_line`, what `-x` keeps.
- `Grammar::segment` returns each tiling a grammar accepts as a `Tiling`.
- `gravity::Readings::span_of`, `types`, `type_label`, `class_of`,
  `most_strained`, `weakest_cuts`, `classes` and `reading_of`, with
  `GravityClass` and `UnitReading`.
- `context::Contexts` and `context::Fold`, and `SeamProfile::mean_strength`.

#### Changed

- `resonator::analyze_symbols` normalizes its power by the symbols that
  advanced the bank, so a code past the alphabet changes nothing; it
  counted in the stream's length before.

#### Fixed

- `context::relate_bytes` builds every axis field its folds read, so its
  observation, seam and flow readings are no longer empty.

### Python

#### Breaking

- `trex.templates()` takes `cut=` in place of `rare_under=`, takes its text
  as `text=` or `path=`, and gives `novel` in each dict.

#### Added

- `trex.Library` (`shape`, `kind`, `let`, `declare`, `include`, `remove`,
  `clear`, `test`, `rules`, `Library.shipped()`) and `trex.tokens()`; rules
  with `lib.check()`, `lib.fix()` with `review=`, `lib.follow()`,
  `trex.sarif()`, `trex.findings_json()`, `Finding.github()` and
  `Finding.format()`, and `trex.TrexWarning` for the notices beside a
  result.
- `Library.load`, `include` and `lib=` take a directory, read as every
  `.trex` file under it in path order, and `remove(dir)` takes out every
  file imported from under it.
- `trex.axes`, a function an axis - `magnitude`, `stress`, `flow`,
  `observation`, `relation`, `spectral`, `seam`, `echo`, `orbit` and
  `shape` - each giving a report with every field of PowerShell's
  `Measure-Trex` report, its texts in the input's type and its offsets in
  the input's units.
- `trex.axes.gravity()` reads the pair field at the token, byte or
  supertoken grain: every unit's strain and the bound of the cut before it
  in bits, each with its percentile among the input's readings, and its
  type's gravity class; the units under the most strain and the cuts held
  together least (`top=`); and each gravity class with its types.
  `detail=True` gives every unit.
- `trex.axes.context()` reads each token against a context, the fold
  `\N{>+1:F}` names (`fold=`): how many tokens it folds and each axis's
  reading over them, with the record period and its live periods, and each
  supertoken's role and how far the lower grains' nearest boundaries are
  from its start.
- Every axis's own threshold is a keyword named as its configuration field
  in snake case (`jump_threshold=`), as the command line's flags are.
- `grain=` and `analytic=` (`AnalyticFrame`) on the flow reading, and
  `grain=` on the observation reading: a flow report carries `grain`,
  `units`, `latency` and `analytic`, an observation report `grain` and
  `units`, and an observation frame the `text` of its unit.
- `trex.axes.observation(lib=)` joins the other axes' `lib=`; the
  observation reads declarations at the token and supertoken grains and
  refuses them at the byte grain.
- `ShapeFrame.period_strength`, each token's shape period strength, which
  `template_strength=` is set against.
- `Pattern.group_by()` and `distinct()`, and the same on a `PatternSet`,
  over a text or the files `path=` names, with sums, exact averages as a
  `Fraction`, least and greatest values and percentiles by register.
- `Pattern.grep()`, an iterator of `Line` (`path`, `number`, `text`,
  `is_match`, `matches`) over a text or the files `path=` names: the lines
  a match starts on, or with `invert=True` the lines none touches, with
  `before=`, `after=` and `context=` lines around them, a count or a record
  unit's name, `whole_line=`, `max_count=`, windows, the walk's switches and
  `backend=`.
- `trex.follow(pattern, *paths, from_end=True, max_count=None,
  keep_count=False, lib=None)`, each match of a pattern as the files grow,
  its file in `Match.path`, as `scan --follow` prints them.
- `keep_count=` where a follow takes `max_count=`: a followed file
  truncated, replaced or removed keeps its count.
- `trex.query()`, the record query of `scan --all`, `--any`, `--none`,
  `--at-least` and `--not` and `Find-TrexRecord`, over a text or files, with
  `unit=`, `record_start=` and `record_span=`, giving a `Record` (`path`,
  `line`, `start`, `end`, `byte_start`, `byte_end`, `text`, `patterns`) for
  each record kept.
- `trex.templates()` mines the files `path=` names as one stream, marks
  each template against another input's with `against=`, a text or a path,
  keeps the novel ones with `novel=True` and the rare ones with
  `rare=True`, groups records with `unit=`, `record_start=` and
  `record_span=` read under `lib=`, and takes `hidden=`, `no_ignore=` and
  `binary=`.
- `Pattern.redact_file()` and `redact_diff()`, with `review=` on
  `redact_file()`, so a redaction is reviewed as a rewrite is;
  `Pattern.diff()` and `rewrite_file()` take files and directories walked as
  `trex.files` walks them, `max_count=`, `backend=`, `head=`, `tail=`,
  `lines=`, `unit=`, `hidden=`, `no_ignore=` and `binary=`, and
  `rewrite_file()` takes `review=`, `show_skipped=` and `explain=`;
  `trex.Change.explanation`; `trex.follow_lines()`, `trex.follow_rewrite()`
  and `trex.follow_redact()`, what `trex tail -f`, `rewrite --follow` and
  `redact --follow` print.
- `Match.format(template, input)`, a report template rendered at a match as
  `scan --format` renders one, `${@axis}` among its fields; and
  `Match.path`, `line`, `column`, `pattern` and `kind(name)`, the fields
  PowerShell's `Trex.Match` carries.
- `trex.set_date_order("dmy" | "mdy")`; `backend=`, `dual_grain=` and
  `chunk_size=` on `Pattern.scan`, `find_iter` and `rewrite`; and
  `trex.scan_stats()`, a `ScanStats` of the last scan on the calling thread
  with every field PowerShell's `-Stats` writes.
- `trex.Grammar` with `ParseNode` and `Tiling`, `trex.Bpe`, `trex.Index`
  and `trex.Prefilter` with `LiteralTest` and `FilterCheck`.
  `Grammar.segment()` returns each tiling a grammar accepts as a `Tiling`.
- The wheel ships type stubs, `trex/__init__.pyi` and `trex/axes.pyi` beside
  `py.typed`. The extension is `trex.trex` inside the `trex` package, which
  re-exports it.
- The compiled module has its own stub, `trex/trex.pyi`, which
  `trex/__init__.pyi` re-exports as the package re-exports the module.

#### Changed

- `${start}` and `${end}` in a report template count what a Python string
  is indexed by, characters of a `str` and bytes of `bytes`, in
  `Finding.format`, a table's key and a rule's message; they wrote UTF-8
  bytes before.
- `count_by` refuses a key reading an axis, and renders `${line}` and
  `${col}` at each match's place.
- `Match.explain` and `Match.format` refuse a text too short to hold the
  match with `ValueError`.
- `trex.templates()` raises `ValueError` for a file holding a NUL byte
  named alone, as a scan refuses one; it mined nothing from it before.

### PowerShell

#### Added

- `Measure-TrexGravity` reads the pair field at the token, byte or
  supertoken grain: every unit's strain and the bound of the cut before it
  in bits, each with its percentile among the input's readings, and its
  type's gravity class; the units under the most strain and the cuts held
  together least (`-Top`); and each gravity class with its types.
  `-Detail` gives every unit.
- `Measure-TrexContext` reads each token against a context, the fold
  `\N{>+1:F}` names (`-Fold`): how many tokens it folds and each axis's
  reading over them, with the record period and its live periods, and each
  supertoken's role and how far the lower grains' nearest boundaries are
  from its start.
- Every axis's own threshold is a parameter named as its configuration
  field in Pascal case (`-JumpThreshold`), as the command line's flags are.
- `-Grain` (`Trex.Grain`) and `-Analytic` (`Trex.AnalyticFrame`) on the flow
  reading, and `-Grain` on the observation reading: a flow report carries
  `Grain`, `Units`, `Latency` and `Analytic`, an observation report `Grain`
  and `Units`, and an observation frame the `Text` of its unit.
- `Measure-TrexObservation -Library` joins the other axes' `-Library`; the
  observation reads declarations at the token and supertoken grains and
  refuses them at the byte grain.
- `Trex.ShapeFrame` gains `PeriodStrength`, each token's shape period
  strength, which `-TemplateStrength` is set against.
- `Group-TrexMatch` takes several registers for `-Sum`, `-Average`,
  `-Minimum`, `-Maximum` and `-PercentileOf`, writing `Sum_t`, `P95_t` and
  so on, and gains `-Binary`; `Get-TrexToken` gains `-Binary`.
- `Import-TrexAtom` and `New-TrexLibrary` take a directory, read as every
  `.trex` file under it in path order, and `Unregister-TrexAtom -Path DIR`
  takes out every file imported from under it.
- `-KeepCount` on `Select-TrexMatch` and `Invoke-TrexRule -Follow
  -MaxCount`: a followed file truncated, replaced or removed keeps its
  count.
- `Invoke-TrexGrammar -Segment` with neither `-Count` nor `-Best` writes a
  `Trex.Tiling` for each tiling the grammar accepts, as `Grammar.Segment()`
  returns them.
- `ConvertTo-TrexPattern -Library` lexes the lines of a pattern built for
  fields under a library's atoms in place of the session's.
- `-NotMatch -Color` prints the lines around each line it selects for
  `-Context` too, as `grep -v` does.
- `Trex.RecordShape` gains `Covered`, the records that had a template.
- A redaction is reviewed as a rewrite is: `Protect-TrexText -Interactive`,
  `-ShowSkipped` and `-Explain`. `Edit-TrexText` gains `-Explain`, putting a
  scan's explanation under each change it asks about, and `-Backend` and
  `-KeepCount`.

#### Changed

- `${start}` and `${end}` in a report template count UTF-16 code units, as
  a match's `Start` counts, in `-Format`, `Group-TrexMatch -Key` and a
  rule's message; they wrote UTF-8 bytes before.
- A followed file truncated, replaced or removed starts its `-MaxCount`
  count and its `-SingleMatch` members again in `Select-TrexMatch` and
  `Invoke-TrexRule`, as the command line does; they kept counting before.
- `Edit-TrexText -Follow -MaxCount` starts its count again for a file
  truncated, replaced or removed, as every other followed count does, unless
  `-KeepCount` carries it on; it kept counting before.
- `Group-TrexMatch` and `Get-TrexToken` warn of a file holding a NUL byte
  named with `-Path`.
- `Get-TrexRecordShape` warns of a file holding a NUL byte named with
  `-Path` or `-Against`, as a scan does; it mined nothing from it before.

#### Fixed

- The axis reports put a point inside a character written in several bytes
  at that character's offset - observation and seam byte frames, a spectral
  frame's last byte, contested and change points, and the ends of segments
  and regions - as `Trex.ObservationFrame` says; they gave the next
  character's offset.
- `Invoke-TrexRule -Count` over `-Tail` stopped with a panic at the first
  finding; a rule's message is placed only when it writes its match's
  position, and the lines and offsets before the tail are counted only then.

### Documentation

#### Added

- An explanation page,
  [The axes](https://variably-constant.github.io/trex/docs/explanation/axes/),
  on what an axis is, the grain each of the twelve reads, and what each
  reads at every position that nothing else does.
- A reference page for the gravity axis,
  [Gravity](https://variably-constant.github.io/trex/docs/reference/axes/gravity/).
- Six how-to guides, each tabbed for the command line, Rust, Python and
  PowerShell:
  [Find where a trend turns](https://variably-constant.github.io/trex/docs/how-to/find-where-a-trend-turns/),
  [Find where text reads two ways](https://variably-constant.github.io/trex/docs/how-to/find-where-text-reads-two-ways/),
  [Find a name reused across scopes](https://variably-constant.github.io/trex/docs/how-to/find-a-name-reused-across-scopes/),
  [Match balanced groups and what they hold](https://variably-constant.github.io/trex/docs/how-to/match-balanced-groups/),
  [Declare an atom for one command](https://variably-constant.github.io/trex/docs/how-to/declare-an-atom-for-one-command/)
  and
  [Scan, rewrite or mask part of a file](https://variably-constant.github.io/trex/docs/how-to/work-on-part-of-a-file/).
- Every axis reference page has a section on what that axis reads that
  nothing else does, and shows its readings, predicates and anchors worked
  on the command line, in Rust, in Python and in PowerShell, with how to
  choose a threshold from the readings.
- The tutorial's axes chapter meets each of the other eleven axes with one
  example, tabbed for the command line, Rust, Python and PowerShell.

#### Changed

- The documentation uses plain wording. The pattern syntax section on
  position anchors is titled "Position anchors", and a link to its old
  title opens the page at its top.
- A supertoken is defined once, on the architecture page, and the pages
  that use the word link there.

#### Fixed

- The README's table of commands names `gravity` and `context` beside the
  other ten axes.

## [0.2.0] - 2026-10-01

### Added

- The `trex` binary for Windows x64, Linux x64, FreeBSD x64 and macOS arm64
  on the GitHub release.
- A `fields` line in pattern files, which the file `trex infer` saves
  carries: a saved build read again keeps its casts, record starts, nested
  fields and accessors on every surface. `scan --fields` prints its records
  table, and Python's `Pattern.fields` and `Pattern.records(text)` read it.
- Python: `trex.files()` lists the files a scan of some paths reads,
  `trex.read()` gives a file's text decoded as a scan decodes it,
  `trex.texture()` names what a text reads as mostly, and `Pattern.diff()`
  and `Pattern.rewrite_file()` diff a rewrite of a file or write it in the
  file's own encoding.
- PowerShell: `Get-TrexFile -Binary`.

### Changed

- An explanation names the token of a declared shape or kind by its
  declaration (`ticket`), as a token listing does.
- The shape axis reads a region as a table only where template runs cover
  more than half of its bytes.
- `trex seam --recover FILE` leaves the word count out of its header.
- `Select-TrexMatch`, `Edit-TrexText` and `Protect-TrexText` write a warning
  naming a file given by `-Path` or `-LiteralPath` that holds a NUL byte.

### Fixed

- `trex scan --files` and `Get-TrexFile` listed files holding a NUL byte that
  a scan leaves unread; they leave them out unless `--binary` or `-Binary`
  asks for them.
- `rewrite`, `redact` and `scan --rules --fix` passed over one named file
  holding a NUL byte without a word under `--dry-run`, `--in-place` and
  `--interactive`; they refuse it and exit with failure, as they do when
  writing to the standard output.
- Python `Match.explain` reports the rung of the scan ladder that answered,
  as `scan --explain` does.
- The PowerShell module reads a pattern's relative `@file` paths against
  PowerShell's current location.

### Removed

- Python `Match.explain` takes no `route=`.

## 0.1.0 - 2026-09-29

The first release.

### Added

- A pattern language over typed tokens: the lexer reads the input once into
  25 kinds of token (numbers, words, quoted strings, IP addresses, URLs,
  emails, timestamps, versions, payment cards and the rest), and an atom such
  as `\N` or `\E` matches one whole token. Matching runs by derivatives and
  does not backtrack.
- Value predicates in a kind's own units (`\N{500..599}`, `\I{in:10.0.0.0/8}`,
  `\T{age<24h}`), balanced bracket groups, and named registers that a later
  `=name` must equal, exactly or up to case, shape, representation or a typed
  relation.
- Custom atoms declared with `let`, `kind` and `shape` lines checked by `test`
  lines, beside a shipped library of 75 named kinds and patterns.
- The `trex` command: `scan` over files, standard input and directory trees
  walked under `.gitignore` rules; `rewrite` and `redact`, in place behind a
  diff dry run; `head`, `tail`, `lines` and `--follow`; `top`, `count-by` and
  `uniq`; the ten property axes; and `infer`, which builds a pattern from
  examples and from `{name:text}` marks, reading `ConvertFrom-String`
  templates.
- A CUDA backend in the default build for patterns that read only token kinds;
  a machine without a device runs the same binary on the CPU.
- The Python package `trex-re`, imported as `trex`: one stable-ABI wheel per
  platform for Python 3.11 and later.
- The PowerShell module `Trex`: 49 cmdlets writing matches, findings and
  reports as objects, for PowerShell 7 and Windows PowerShell 5.1.
- The MIT license.

[0.3.0]: https://github.com/Variably-Constant/trex/releases/tag/v0.3.0
[0.2.0]: https://github.com/Variably-Constant/trex/releases/tag/v0.2.0
