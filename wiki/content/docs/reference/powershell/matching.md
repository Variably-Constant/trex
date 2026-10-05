---
title: Matching
linkTitle: Matching
weight: 10
---

`Select-TrexMatch` is the module's `trex scan`, `Get-TrexLine` its `head`, `tail` and `lines`,
and `New-TrexIndex` its `index`. The examples are on [matching](../../matching/),
[windows](../../windows/) and [indexes](../../tools/#indexes).

### Select-TrexMatch

Finds every match of a TREX pattern in text, files or directories and writes each as a Trex.Match.

A pattern is TREX source text or a Trex.Pattern from New-TrexPattern; a source text is compiled against the session's atoms, or -Library's. Strings piped in are the lines of one input, as Get-Content writes a file's, each ending a line unless it ends with a newline of its own: a match may run from one string into the next, LineNumber counts across them, and -Context reads the strings around a match. Where the report writes the matches alone, each is written once the text holding it has arrived; a report read over the whole input, such as -Context, -NotMatch or -Count, is written after the last string. -PerString scans each string as an input of its own. -Path reads each file whole and scans it in one call, so a large input crosses into TREX once per file; a directory is walked with .gitignore and .ignore rules, skipping hidden and binary files, as the `trex` command walks one. A file named outright that holds a NUL byte is left unread and named in a warning, unless -Binary asks for it.

-Context adds the lines around each match, or the rest of the paragraph, block or other record a unit names; -NotMatch writes the lines no match touches instead, and -WholeLine keeps a match only where it covers its line. -FileType and -Texture keep the walked files of a type or a texture, and -Sort orders the files read.

In place of the matches, -Raw writes their text and -Format a report template rendered at each, which reads `${path}`, `${line}` and `${col}` beside the registers, and `${@axis}` for what -Explain reads. -Count writes how many lines of each file hold a match and -CountMatches how many matches it holds, each as a Trex.MatchCount for a file with any, and over text as one count of every string, after the last. -FilesWithMatches and -FilesWithoutMatch write the path of each file holding a match or none, and -Quiet one boolean. -Stats adds a Trex.ScanStats after the report.

-Json, -Color and -Passthru write what the `trex` command prints for the same scan, a string a line: -Json its `--json` report, -Color its text report painted as `--color` paints it, and -Passthru every line with the matches painted, as `--passthru` prints it to a console. A report names its inputs as the command's does, over a directory, several paths or paths piped in.

Several patterns scan as one set, each match naming the one that made it in its Pattern property: patterns given one by one are named by their text, and the members of a -PatternFile by the names the file gives, `let name = pattern`, or by their line numbers. -SingleMatch keeps each member's first match. -RequireMatch writes an error when no input held a match, for a script that stops on one.

One pattern's scan runs where -Backend says, the CPU engine or the device, or with -DualGrain as the byte and token grains in a pipeline, or with -ChunkSize over each input fed in chunks; every choice finds the same matches, and how the scan ran is written as verbose output.

Alias: `Select-TxMatch`

```powershell
Select-TrexMatch [[-Pattern] <Object[]>] [-InputObject] <string> [-PatternFile <string>] [-PerString] [-Library <Library>] [-List] [-Raw] [-Quiet] [-Context <string[]>] [-NotMatch] [-WholeLine] [-MaxCount <uint>] [-Format <string>] [-Json] [-ValueSpelling <ValueSpelling>] [-DurationUnit <DurationUnit>] [-Color] [-ColorDepth <ColorDepth>] [-Colors <string[]>] [-Passthru] [-Count] [-CountMatches] [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-Stats] [-SingleMatch] [-RequireMatch] [-Backend <Backend>] [-DualGrain] [-ChunkSize <uint>] [-Explain] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-WhatIf] [-Confirm] [<CommonParameters>]
Select-TrexMatch [[-Pattern] <Object[]>] -Path <string[]> [-PatternFile <string>] [-Library <Library>] [-List] [-Raw] [-Quiet] [-Context <string[]>] [-NotMatch] [-WholeLine] [-MaxCount <uint>] [-Format <string>] [-Json] [-ValueSpelling <ValueSpelling>] [-DurationUnit <DurationUnit>] [-Color] [-ColorDepth <ColorDepth>] [-Colors <string[]>] [-Passthru] [-Count] [-CountMatches] [-FilesWithMatches] [-FilesWithoutMatch] [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-Stats] [-SingleMatch] [-RequireMatch] [-Backend <Backend>] [-DualGrain] [-ChunkSize <uint>] [-Explain] [-Hidden] [-NoIgnore] [-Binary] [-Include <string[]>] [-FileType <string[]>] [-ExcludeFileType <string[]>] [-Sort <SortKey>] [-Descending] [-Texture <RegionKind[]>] [-ExcludeTexture <RegionKind[]>] [-Index] [-NoIndex] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Follow] [-KeepCount] [-WhatIf] [-Confirm] [<CommonParameters>]
Select-TrexMatch [[-Pattern] <Object[]>] -LiteralPath <string[]> [-PatternFile <string>] [-Library <Library>] [-List] [-Raw] [-Quiet] [-Context <string[]>] [-NotMatch] [-WholeLine] [-MaxCount <uint>] [-Format <string>] [-Json] [-ValueSpelling <ValueSpelling>] [-DurationUnit <DurationUnit>] [-Color] [-ColorDepth <ColorDepth>] [-Colors <string[]>] [-Passthru] [-Count] [-CountMatches] [-FilesWithMatches] [-FilesWithoutMatch] [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-Stats] [-SingleMatch] [-RequireMatch] [-Backend <Backend>] [-DualGrain] [-ChunkSize <uint>] [-Explain] [-Hidden] [-NoIgnore] [-Binary] [-Include <string[]>] [-FileType <string[]>] [-ExcludeFileType <string[]>] [-Sort <SortKey>] [-Descending] [-Texture <RegionKind[]>] [-ExcludeTexture <RegionKind[]>] [-Index] [-NoIndex] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Follow] [-KeepCount] [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Backend` | [Trex.Backend](../types/#trexbackend) |  | The engine one pattern's scan runs on: Auto when absent, Cpu, or Gpu, which falls back to the CPU with a note where the device cannot take the scan. |
| `-Binary` | switch |  | Reads files that hold a NUL byte, which a walk treats as binary. |
| `-ChunkSize` | uint |  | Feeds each input to one pattern's scan in chunks of this many bytes, as a stream arrives. |
| `-Color` | switch |  | Writes the report as the `trex` command prints it, a line a string, painted: each match's span and text, or with its path, line and column over a directory, several paths or paths piped in; -Context, -Explain and -NotMatch add their lines as the command's flags do. |
| `-ColorDepth` | [Trex.ColorDepth](../types/#trexcolordepth) |  | The depth -Color and -Passthru paint at, as the `trex` command's `--color` names it: None, which paints nothing, as `--color never`; Ansi16, Ansi256 or TrueColor. What the environment says when absent, as `--color always` reads it. |
| `-Colors` | string[] |  | A role's paint for -Color and -Passthru, as the `trex` command's `--colors` takes it: `match:fg:red`, `path:bg:#202020`, `line:style:bold`, `kind:NAME:...` for a token kind, `capture:NAME:...` for a register. |
| `-Context` | string[] |  | The lines to add around each match: one count for both sides, or two, the lines before and the lines after. A record unit in place of a count, such as `paragraph` or `block`, adds the rest of the record holding the match on that side, and `record` names the one -Unit, -RecordStart or -RecordSpan defines. |
| `-Count` | switch |  | Writes how many records of each file hold a match, or under -NotMatch hold none, as a Trex.MatchCount for each file with any; over text, one count of every string, after the last. A record is a line unless -Unit, -RecordStart or -RecordSpan says otherwise. |
| `-CountMatches` | switch |  | Writes how many matches each file holds, as -Count writes the records holding one; under -NotMatch it counts the records none touches. |
| `-Descending` | switch |  | Reverses the order -Sort names. |
| `-DualGrain` | switch |  | Runs one pattern's scan as the byte grain and the token grain in a pipeline, writing their timing as verbose output. |
| `-DurationUnit` | [Trex.DurationUnit](../types/#trexdurationunit) |  | The unit -Json writes a duration in, as the `trex` command's `--duration-unit` takes it: Nanoseconds when absent, Milliseconds or Seconds. |
| `-ExcludeFileType` | string[] |  | Drops a walked file of one of these types. |
| `-ExcludeTexture` | [Trex.RegionKind](../types/#trexregionkind)[] |  | Drops a walked file whose text reads mostly as one of these. |
| `-Explain` | switch |  | Adds what each match is made of: its tokens' kinds, the checks its guarded kinds passed, and every axis the pattern read at it. |
| `-FilesWithMatches` | switch |  | Writes the path of each file holding a match, or under -NotMatch a record no match touches. |
| `-FilesWithoutMatch` | switch |  | Writes the path of each file holding no match, or under -NotMatch each file whose every record holds one. |
| `-FileType` | string[] |  | Keeps a walked file only when it is of one of these types, under ripgrep's names: `rust`, `py`, `js`, `log` and the rest. |
| `-Follow` | switch |  | After each file's -Tail, its open -Lines range or the whole of it, scans what it gains as it grows, writing each match once nothing that arrives later can change it, until the pipeline is stopped. A file truncated, replaced or removed is a new input, its -MaxCount count started again. |
| `-Format` | string |  | A report template to write at each match in place of the match: `${name}` a register, `${path}`, `${line}` and `${col}` its file, line and column, `${start}` and `${end}` its offsets in UTF-16 code units, as Start counts, `${@axis}` an axis the pattern read. |
| `-Head` | uint |  | Scans the first this many lines of each input, or records of -Unit, reading a file no further. |
| `-Hidden` | switch |  | Reads hidden files and directories a walk would skip. |
| `-Include` | string[] |  | Keeps a walked file only when a glob matches it (`*.log`), or drops it for a glob that starts with `!`. |
| `-Index` | switch |  | Writes each directory's index from the files this scan reads, so every later scan of the tree opens only the files that can match. |
| `-InputObject` | string | by value | The text to scan. Strings piped in are the lines of one input, as Get-Content writes a file's, unless -PerString is given. |
| `-Json` | switch |  | Writes the matches as the `trex` command's `--json` writes them: an array for the text -InputObject gives or a string under -PerString; over strings piped in, each match's object as it settles, as the command writes a stream on its standard input, or one array after the last string where a flag reads the whole input; over a directory, several paths or paths piped in, one array of every match after the last input, each object naming its path, line and column. |
| `-KeepCount` | switch |  | With -Follow and -MaxCount, keeps a file's count when it is truncated, replaced or removed, in place of starting it again. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms a source-text pattern is compiled against, in place of the session's. |
| `-Lines` | object |  | Scans a range of lines of each input, or records of -Unit, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. A match counts only if it is wholly inside the part read, and every match reports the input's own line and offset. |
| `-List` | switch |  | Writes only the first match of each input. |
| `-LiteralPath` | string[] | by name | Files or directories to scan, read as written, as Get-ChildItem pipes them. |
| `-MaxCount` | uint |  | Writes or counts at most this many matches, or lines under -NotMatch, of each input. |
| `-NoIgnore` | switch |  | Reads files an ignore rule excludes. |
| `-NoIndex` | switch |  | Reads no index, whatever the trees hold. |
| `-NotMatch` | switch |  | Writes the lines no match touches, each as a Trex.Match of the whole line with no registers. |
| `-Passthru` | switch |  | Writes every line of each input as the `trex` command's `--passthru` prints it, a line a string, the matches painted: where the report names its inputs, with its path and line number ahead of it, joined by `:` for a line holding a match and by `-` for the rest. -ColorDepth None writes the lines unpainted. |
| `-Path` | string[] |  | Files or directories to scan; wildcards expand. |
| `-Pattern` | object[] |  | The patterns: TREX source text or Trex.Pattern objects; several scan as one set. |
| `-PatternFile` | string |  | A pattern file whose members scan as one set: each `let name = pattern` line under its name and each bare pattern line under its line number, with the file's declarations in force. Text to scan beside it is piped in or named with -InputObject, since a first argument by position is read as -Pattern. |
| `-PerString` | switch |  | Scans each string piped in as an input of its own, rather than as the next line of one input. |
| `-Quiet` | switch |  | Writes only whether any input matched, once, after every input. |
| `-Raw` | switch |  | Writes the matched text rather than match objects. |
| `-RecordSpan` | object |  | A pattern whose every match is a record, for -Count and -Context record. |
| `-RecordStart` | object |  | A pattern whose every match starts a record that runs to the next, for -Count and -Context record. |
| `-RequireMatch` | switch |  | Writes an error after the last input when none held a match, or under -NotMatch when every line of every input held one. |
| `-SingleMatch` | switch |  | Keeps the first match of each pattern of a set in each input, where several patterns or -PatternFile give one. |
| `-Sort` | [Trex.SortKey](../types/#trexsortkey) |  | Orders the files read: by path, or by the time each was last written, read or created, oldest first. |
| `-Stats` | switch |  | Writes a Trex.ScanStats after the report: the matches, the lines they touch, the inputs scanned and those holding a match, the bytes scanned and the time taken. |
| `-Tail` | uint |  | Scans the last this many lines of each input, or records of -Unit, reading a file backward from its end. |
| `-Texture` | [Trex.RegionKind](../types/#trexregionkind)[] |  | Keeps a walked file only when its text reads mostly as one of these. |
| `-Unit` | string |  | What a record is for -Count and -Context record, and what -Head, -Tail and -Lines count: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph`, `file` or `block`. |
| `-ValueSpelling` | [Trex.ValueSpelling](../types/#trexvaluespelling) |  | How -Json spells a typed register's value, as the `trex` command's `--values` takes it: Exact when absent, a JSON number only where a double holds the value; Natural, a number throughout; or Tagged, its kind and its exact text. |
| `-WholeLine` | switch |  | Keeps a match only where it covers its line, from the first character that is not whitespace to the last. |

Writes [Trex.Match](../types/#trexmatch), `string`, `bool`, [Trex.MatchCount](../types/#trexmatchcount), [Trex.ScanStats](../types/#trexscanstats).

### Test-TrexMatch

Tells whether a TREX pattern matches text or the files named.

With several inputs the answer is whether any of them matched, and with several patterns whether any of them did. -Head, -Tail and -Lines test only the first lines of each input, its last, or a range of them, counted in records of -Unit where it names one; a match counts only if it is wholly inside. -First and -Last are -Head and -Tail.

Alias: `Test-TxMatch`

```powershell
Test-TrexMatch [[-Pattern] <Object[]>] [-InputObject] <string> [-PatternFile <string>] [-Library <Library>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [<CommonParameters>]
Test-TrexMatch [[-Pattern] <Object[]>] -Path <string[]> [-PatternFile <string>] [-Library <Library>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Head` | uint |  | Tests the first this many lines of each input, or records of -Unit, reading a file no further. |
| `-InputObject` | string | by value | The text to test; each string piped in is tested on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms a source-text pattern is compiled against, in place of the session's. |
| `-Lines` | object |  | Tests a range of lines, or records of -Unit, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. |
| `-Path` | string[] |  | Files to test; wildcards expand. |
| `-Pattern` | object[] |  | The patterns: TREX source text or Trex.Pattern objects. |
| `-PatternFile` | string |  | A pattern file whose members are tested as one set. Text to test beside it is piped in or named with -InputObject, since a first argument by position is read as -Pattern. |
| `-Tail` | uint |  | Tests the last this many lines of each input, or records of -Unit, reading a file backward from its end. |
| `-Unit` | string |  | What -Head, -Tail and -Lines count: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph` or `block`. |

Writes `bool`.

### Get-TrexLine

Writes an input's first lines, its last, or a range of them, as the `trex` command's head, tail and lines print them.

-Head N writes the first N lines, read no further than the Nth newline; -Tail N the last N, read backward from the file's end; -Lines a range, `"100..200"`, `"100.."`, `"..200"`, or PowerShell's own `100..200`. -Unit counts paragraphs, blocks or another record unit instead of lines. -First and -Last are -Head and -Tail. Each line is a Trex.Line with its number in the input; -Passthru writes the lines as strings, a `==> path <==` header ahead of each file's where several are read, as the `trex` command prints them.

-Follow writes the lines each file gains as it grows, after its tail or its open range, through truncation and rotation, which it writes as a warning, until the pipeline is stopped.

Alias: `Get-TxLine`

```powershell
Get-TrexLine [-Path] <string[]> [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-Passthru] [-Binary] [-Hidden] [-NoIgnore] [<CommonParameters>]
Get-TrexLine -LiteralPath <string[]> [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-Passthru] [-Binary] [-Hidden] [-NoIgnore] [<CommonParameters>]
Get-TrexLine -InputObject <string> [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Passthru] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Binary` | switch |  | Reads files that hold a NUL byte, which a walk treats as binary. |
| `-Follow` | switch |  | Writes the lines each file gains as it grows, after its tail or its open range, until the pipeline is stopped. |
| `-Head` | uint |  | Writes the first this many lines, or records of -Unit. |
| `-Hidden` | switch |  | Reads hidden files and directories a walk would skip. |
| `-InputObject` | string | by value | Text to read; each string piped in is read on its own. |
| `-Lines` | object |  | Writes a range of lines, or records of -Unit, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. |
| `-LiteralPath` | string[] | by name | Files or directories to read, read as written, as Get-ChildItem pipes them. |
| `-NoIgnore` | switch |  | Reads files an ignore rule excludes. |
| `-Passthru` | switch |  | Writes the lines as strings, as the `trex` command prints them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-Tail` | uint |  | Writes the last this many lines, or records of -Unit. |
| `-Unit` | string |  | What -Head, -Tail and -Lines count: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph` or `block`. |

Writes [Trex.Line](../types/#trexline), `string`.

### New-TrexPattern

Compiles a TREX pattern against the atoms in force, for reuse across commands and as an object with IsMatch, Find, FindAll, Replace and Split methods.

The pattern keeps the atoms it was compiled against, so it matches the same way wherever it is passed later.

Alias: `New-TxPattern`

```powershell
New-TrexPattern [-Pattern] <string> [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms to compile against, in place of the session's. |
| `-Pattern` | string | by value | The pattern's source text. |

Writes [Trex.Pattern](../types/#trexpattern).

### Get-TrexFile

Lists the files a scan of the paths would read, without scanning them.

The walk is Select-TrexMatch's: .gitignore and .ignore rules applied, hidden files skipped unless -Hidden asks for them, and -Include and -FileType keeping what it finds, while a file named outright is listed whatever they say. A file holding a NUL byte is binary and is left out, named or found, as Select-TrexMatch leaves it unread, unless -Binary asks for it. -Texture and -ExcludeTexture read every file listed, named or found, as the `trex` command's `--files` does, and -Classify reads each file's texture into its Texture without filtering.

Alias: `Get-TxFile`

```powershell
Get-TrexFile [[-Path] <string[]>] [-Hidden] [-NoIgnore] [-Binary] [-Include <string[]>] [-FileType <string[]>] [-ExcludeFileType <string[]>] [-Texture <RegionKind[]>] [-ExcludeTexture <RegionKind[]>] [-Sort <SortKey>] [-Descending] [-Classify] [<CommonParameters>]
Get-TrexFile -LiteralPath <string[]> [-Hidden] [-NoIgnore] [-Binary] [-Include <string[]>] [-FileType <string[]>] [-ExcludeFileType <string[]>] [-Texture <RegionKind[]>] [-ExcludeTexture <RegionKind[]>] [-Sort <SortKey>] [-Descending] [-Classify] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Binary` | switch |  | Lists files that hold a NUL byte, which a scan treats as binary and leaves unread. |
| `-Classify` | switch |  | Reads what each file's text reads as mostly into its Texture, and a table's period into its Period. |
| `-Descending` | switch |  | Reverses the order -Sort names. |
| `-ExcludeFileType` | string[] |  | Drops a walked file of one of these types. |
| `-ExcludeTexture` | [Trex.RegionKind](../types/#trexregionkind)[] |  | Drops a file whose text reads mostly as one of these. |
| `-FileType` | string[] |  | Keeps a walked file only when it is of one of these types, under ripgrep's names, which Get-TrexFileType lists. |
| `-Hidden` | switch |  | Lists hidden files and directories a walk would skip. |
| `-Include` | string[] |  | Keeps a walked file only when a glob matches it (`*.log`), or drops it for a glob that starts with `!`. |
| `-LiteralPath` | string[] | by name | Files or directories to walk, read as written, as Get-ChildItem pipes them. |
| `-NoIgnore` | switch |  | Lists files an ignore rule excludes. |
| `-Path` | string[] | by value | Files or directories to walk; wildcards expand. The current directory where none is named. |
| `-Sort` | [Trex.SortKey](../types/#trexsortkey) |  | Orders the files: by path, or by the time each was last written, read or created, oldest first. |
| `-Texture` | [Trex.RegionKind](../types/#trexregionkind)[] |  | Keeps a file only when its text reads mostly as one of these. |

Writes [Trex.File](../types/#trexfile).

### Get-TrexFileType

Lists the file types -FileType and -ExcludeFileType take, each with the globs a file of it matches: ripgrep's types, under ripgrep's names.

Alias: `Get-TxFileType`

```powershell
Get-TrexFileType [[-Name] <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Name` | string |  | Only the types whose name matches; wildcards apply. |

Writes [Trex.FileType](../types/#trexfiletype).

### New-TrexIndex

Writes the index of each tree named at its root, so every later scan of the tree opens only the files that can match.

The tree is walked as a scan walks it, with .gitignore and .ignore rules applied and hidden files skipped unless asked for. Select-TrexMatch reads the index of each directory it scans without being asked, and -Index writes one while it scans.

Alias: `New-TxIndex`

```powershell
New-TrexIndex [-Path] <string[]> [-Hidden] [-NoIgnore] [-Include <string[]>] [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Hidden` | switch |  | Indexes hidden files and directories a walk would skip. |
| `-Include` | string[] |  | Indexes a walked file only when a glob matches it, or drops it for a glob that starts with `!`. |
| `-NoIgnore` | switch |  | Indexes files an ignore rule excludes. |
| `-Path` | string[] | by value | The trees to index; wildcards expand. |

Writes [Trex.IndexInfo](../types/#trexindexinfo).

### Get-TrexIndex

Writes what the index at each tree's root holds.

Alias: `Get-TxIndex`

```powershell
Get-TrexIndex [-Path] <string[]> [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Path` | string[] | by value | The trees whose index to read; wildcards expand. |

Writes [Trex.IndexInfo](../types/#trexindexinfo).
