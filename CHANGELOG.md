# Changelog

All notable changes to trex are recorded here. The Rust crate (`trex-re` on
crates.io), the Python package (`trex-re` on PyPI) and the PowerShell module
(`Trex` on the PowerShell Gallery) share one version number and release
together. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
