---
title: PowerShell
linkTitle: PowerShell
weight: 60
sidebar:
  open: true
---

# PowerShell

The engine is a PowerShell module, `Trex`, built from the repository's `powershell/` directory
with [PWRS](https://github.com/Variably-Constant/PWRS). Its 49 cmdlets cover what the command
line does. Each takes text from the pipeline or files by path, writes objects a pipeline can
sort, group and select, and answers to a second name with the `Tx` prefix: `Select-TxMatch` is
`Select-TrexMatch`. It runs in PowerShell 7 on Windows x64, Linux x64, FreeBSD x64 and macOS
arm64, and in Windows PowerShell 5.1, from one module; its Pester suites pass on each.

`Trex` installs from the PowerShell Gallery:

```powershell
Install-Module Trex
```

A checkout builds the module with PWRS 0.3.1 from crates.io and `cargo pwrs` of the same
release:

```text
cargo install cargo-pwrs --version 0.3.1
cd powershell
cargo pwrs build --release
Import-Module ./target/pwrs/Trex
```

Built from `powershell/`, the module's library targets the baseline CPU of its architecture,
x86-64 (apple-a14 on macOS), whatever `target-cpu` the host's cargo config names; trex picks its
AVX2 and AVX-512 kernels at run time. `cargo pwrs test --release` runs the module's Rust tests and
then its Pester suites in PowerShell 7 and in Windows PowerShell 5.1. The report suites compare the module's output with the command's,
so `TREX_CLI` names a trex binary built from the same tree.

## A first match

```powershell
PS> 'ping bob@x.com and ann@y.org' | Select-TrexMatch '\E:e' | Select-Object Start, Length, Text

Start Length Text
----- ------ ----
    5      9 bob@x.com
   19      9 ann@y.org

PS> Select-TrexMatch '\E:e' -InputObject 'ping bob@x.com' | ForEach-Object { $_.Captures.e }
bob@x.com

PS> 'mail bob@x.com now' | Edit-TrexText '\E:e' '[${e:domain}]'
mail [x.com] now
```

A pattern is written as the [pattern syntax](../pattern-syntax/) page writes one, inside single
quotes so PowerShell leaves `$` and backslashes alone.

## How the cmdlets read and write

| Concern | Behavior |
|---|---|
| Text | `-InputObject`, or strings piped in, each read on its own; `Group-TrexMatch`, `Get-TrexRecordShape`, `New-TrexBpe`, `New-TrexPrefilter` and `Test-TrexPrefilter` read every string piped in as one input |
| Files | `-Path` expands wildcards; `-LiteralPath` takes a path as written and binds the `Path` of what `Get-ChildItem` pipes. A directory is walked under `.gitignore` and `.ignore` rules with hidden files and files holding a NUL byte skipped, which `-Hidden`, `-NoIgnore` and `-Binary` widen. A file in UTF-16 or UTF-32 with a byte-order mark reads as its text |
| Offsets | `Start` and `Length` count UTF-16 code units, so `$text.Substring($m.Start, $m.Length)` is the match; `LineNumber` and `Column` count from 1 |
| Values | a register's `Value` is the .NET type of its kind: a number a `long` or a `decimal`, a duration a `TimeSpan`, a timestamp a `DateTimeOffset`, an address or a version its canonical text |
| Patterns | trex source text, or a `Trex.Pattern` from `New-TrexPattern`, compiled against the atoms in force; several patterns scan as one set |
| Atoms | `Register-TrexAtom` and `Import-TrexAtom` declare atoms for the session, held in `$TrexSession`; `-Library` hands a cmdlet a `Trex.Library` in their place |
| Changes | every cmdlet that writes a file takes `-WhatIf` and `-Confirm`, as do `Set-TrexClock` and `Unregister-TrexAtom` |
| Refusals | parameters that contradict each other stop the cmdlet with an error that names them |

The pages that follow run their examples in a folder shown as `C:\Temp\demo`, which holds the
files each page shows with `Get-Content` before its first example reads them.

{{< cards >}}
  {{< card link="matching/" title="Matching" subtitle="Select-TrexMatch, Test-TrexMatch, compiled patterns, file walks and indexes." >}}
  {{< card link="rewriting/" title="Rewriting" subtitle="Templates, script blocks, masking, diffs and in-place review." >}}
  {{< card link="grouping/" title="Grouping" subtitle="Counts by a rendered key, with sums, extremes and percentiles in .NET types." >}}
  {{< card link="records/" title="Records and tokens" subtitle="Tokens, records, record shapes, record queries and inferred patterns." >}}
  {{< card link="rules/" title="Rules" subtitle="Rule files, findings, fixes, SARIF and GitHub annotations." >}}
  {{< card link="atoms/" title="Atoms and settings" subtitle="Custom atoms for the session or a library, their tests, and the clock." >}}
  {{< card link="axes/" title="Property axes" subtitle="The ten Measure-Trex cmdlets and their reports." >}}
  {{< card link="streams/" title="Streams and grammars" subtitle="Streaming scans, token grammars, byte-pair encoders and presence filters." >}}
  {{< card link="types/" title="Types" subtitle="Every class and enumeration the cmdlets write and take." >}}
{{< /cards >}}

## Every cmdlet

| Cmdlet | Does |
|---|---|
| [Select-TrexMatch](matching/#select-trexmatch) | Finds every match of a trex pattern in text, files or directories and writes each as a Trex.Match. |
| [Test-TrexMatch](matching/#test-trexmatch) | Tells whether a trex pattern matches text or the files named. |
| [Get-TrexLine](matching/#get-trexline) | Writes an input's first lines, its last, or a range of them, as the trex command's head, tail and lines print them. |
| [New-TrexPattern](matching/#new-trexpattern) | Compiles a trex pattern against the atoms in force, for reuse across commands and as an object with IsMatch, Find, FindAll, Replace and Split methods. |
| [Get-TrexFile](matching/#get-trexfile) | Lists the files a scan of the paths would read, without scanning them. |
| [Get-TrexFileType](matching/#get-trexfiletype) | Lists the file types -FileType and -ExcludeFileType take, each with the globs a file of it matches: ripgrep's types, under ripgrep's names. |
| [New-TrexIndex](matching/#new-trexindex) | Writes the index of each tree named at its root, so every later scan of the tree opens only the files that can match. |
| [Get-TrexIndex](matching/#get-trexindex) | Writes what the index at each tree's root holds. |
| [Edit-TrexText](rewriting/#edit-trextext) | Replaces each match of a trex pattern with a rendered template or with what a script block returns for it. |
| [Protect-TrexText](rewriting/#protect-trextext) | Masks each match of a trex pattern, keeping the fields named. |
| [Group-TrexMatch](grouping/#group-trexmatch) | Groups the matches of a trex pattern by a rendered key, with the count of each and aggregates over a register's typed values. |
| [Get-TrexToken](records/#get-trextoken) | Lists the tokens trex reads an input as, with each token's kind and its value parsed as that kind. |
| [Get-TrexRecord](records/#get-trexrecord) | Splits an input into records: lines, paragraphs, blocks, or the other units trex reads a record as. |
| [Get-TrexRecordShape](records/#get-trexrecordshape) | Lists the record shapes an input repeats, most frequent first: each distinct sequence of token kinds once, with how many records have it. |
| [Find-TrexRecord](records/#find-trexrecord) | Finds the records of an input that hold all, any, none or at least some number of the patterns given, and none of the patterns -Not gives. |
| [ConvertTo-TrexPattern](records/#convertto-trexpattern) | Infers the most specific trex pattern every example matches, verified against each. |
| [ConvertFrom-TrexText](records/#convertfrom-trextext) | Converts lines of text to objects, one per line a pattern reads, with a property per field, as ConvertFrom-String does from a template. |
| [ConvertTo-TrexLiteral](records/#convertto-trexliteral) | Escapes a text so a trex pattern matches it literally. |
| [Get-TrexRule](rules/#get-trexrule) | Lists the rules declared for the session, in a library, or in the rule files named, in declaration order. |
| [Invoke-TrexRule](rules/#invoke-trexrule) | Scans the rules declared in pattern files over text, files or directories, and writes each finding as a Trex.Finding. |
| [ConvertTo-TrexSarif](rules/#convertto-trexsarif) | Renders findings as one SARIF 2.1.0 document, for a CI system or an editor that reads one. |
| [Register-TrexAtom](atoms/#register-trexatom) | Declares an atom a pattern reads by name as `\{name}`: a byte shape, a kind fused from a pattern, or a named sub-pattern. |
| [Import-TrexAtom](atoms/#import-trexatom) | Declares every atom the pattern files say: their `let`, `kind`, `shape`, `shape-after`, `rule` and `test` lines, with a relative `@file` set read from beside each file. |
| [Get-TrexAtom](atoms/#get-trexatom) | Lists the atoms declared for this session, in a library, or shipped with trex, in declaration order. |
| [Test-TrexAtom](atoms/#test-trexatom) | Runs the `test` lines declared for the session, a library, or the atoms trex ships, and writes one result per line. |
| [Unregister-TrexAtom](atoms/#unregister-trexatom) | Removes atoms from the session or a library: by name, by the pattern file that declared them, or all of them. |
| [New-TrexLibrary](atoms/#new-trexlibrary) | Makes a library: a set of atoms held in an object, passed to a cmdlet with -Library, which reads it in place of the session's atoms. |
| [Get-TrexClock](atoms/#get-trexclock) | Writes the clock typed predicates read: the instant `now` is, whether it is fixed, the offset a timestamp with no zone is read at, and the order of a slash date's fields. |
| [Set-TrexClock](atoms/#set-trexclock) | Sets the clock typed predicates read, for every later scan in this PowerShell process. |
| [Get-TrexInfo](atoms/#get-trexinfo) | Writes the trex engine's version and whether a CUDA device is present. |
| [Measure-TrexMagnitude](axes/#measure-trexmagnitude) | Reads the magnitude axis: each token's order of magnitude, its change from the token before, and the energy of the window behind it. |
| [Measure-TrexStress](axes/#measure-trexstress) | Reads the stress axis: how deeply the paired brackets nest at each token, how long the innermost has been held open, and how long all of them together. |
| [Measure-TrexFlow](axes/#measure-trexflow) | Reads the flow axis: the trend of a per-token signal, the magnitude by default, how long it has run one way, and where it reverses. |
| [Measure-TrexObservation](axes/#measure-trexobservation) | Reads the observation axis: the entropy of the byte classes behind each byte, ahead of it and around it, and the points where behind and ahead differ most. |
| [Measure-TrexEcho](axes/#measure-trexecho) | Reads the echo axis: which words, numbers and literals recur, how many times, and whether they recur at a regular distance. |
| [Measure-TrexOrbit](axes/#measure-trexorbit) | Reads the orbit axis: each token as the representative of its orbit under a symmetry, and how far the symmetry folds the input's vocabulary. |
| [Measure-TrexShape](axes/#measure-trexshape) | Reads the shape axis: each token's shape, the period at which the shapes repeat, and the regions that repeat like a table or a list of records. |
| [Measure-TrexRelation](axes/#measure-trexrelation) | Reads the relation axis: the graph an input's brackets, binding punctuation, adjacency and repeated content make, and what that graph's tree and its loops measure. |
| [Measure-TrexSpectral](axes/#measure-trexspectral) | Reads the spectral axis: the entropy, byte period, novelty and class mix of an input's bytes along its length, where they change, and the texture of each stretch between, prose, code, mathematics, data or mixed. |
| [Measure-TrexSeam](axes/#measure-trexseam) | Reads the seam axis: where an input divides into units, found where the text before a point stops predicting the text after it, read in both directions, with no dictionary. |
| [New-TrexStreamScanner](streams/#new-trexstreamscanner) | Begins a scan of input that arrives in pieces: a Trex.StreamScanner whose Push method takes each chunk and gives back the matches no later chunk can change, and whose Finish method gives back the rest. |
| [New-TrexGrammar](streams/#new-trexgrammar) | Compiles a token grammar for Invoke-TrexGrammar and as an object with Parse, Test, CountParses, BestProbability, TotalProbability, CountSegmentations and BestSegmentationProbability methods. |
| [Invoke-TrexGrammar](streams/#invoke-trexgrammar) | Parses text against a token grammar and writes the parse tree, or with -Count, -Best or -Probability a value over every derivation. |
| [New-TrexBpe](streams/#new-trexbpe) | Learns a byte-pair encoder from a corpus. |
| [Import-TrexBpe](streams/#import-trexbpe) | Reads a byte-pair encoder from a model file, one Export-TrexBpe or the trex command's `bpe train` wrote. |
| [Export-TrexBpe](streams/#export-trexbpe) | Writes a byte-pair encoder's model to a file, one merge a line with its two symbols separated by a tab, which Import-TrexBpe and the trex command's `bpe encode --model` read. |
| [ConvertTo-TrexBpe](streams/#convertto-trexbpe) | Splits text into a byte-pair encoder's subwords, writing each subword. |
| [New-TrexPrefilter](streams/#new-trexprefilter) | Builds a presence filter over a corpus, as an object whose MightContain method answers whether a literal might occur in it. |
| [Test-TrexPrefilter](streams/#test-trexprefilter) | Tests literals against a presence filter built over a corpus, or with -Verify every filter's contract over it. |
