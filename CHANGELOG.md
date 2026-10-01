# Changelog

All notable changes to trex are recorded here. The Rust crate (`trex-re` on
crates.io), the Python package (`trex-re` on PyPI) and the PowerShell module
(`Trex` on the PowerShell Gallery) share one version number and release
together. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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

## [0.1.0] - 2026-09-29

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
