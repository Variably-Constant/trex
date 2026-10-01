---
title: Records and tokens
linkTitle: Records and tokens
weight: 40
---

# Records and tokens

`Get-TrexToken` reads an input as tokens, `Get-TrexRecord` as records, `Get-TrexRecordShape` as
the record shapes it repeats (the module's `trex templates`), and `Find-TrexRecord` as the
records holding a combination of patterns. `ConvertTo-TrexPattern` is the module's `trex infer`,
and `ConvertFrom-TrexText` turns lines into objects with a pattern it built. The examples are on
[records](../../records/) and [building patterns](../../building-patterns/).

### Get-TrexToken

Lists the tokens trex reads an input as, with each token's kind and its value parsed as that kind.

The atoms in force decide the lex: a declared shape or kind is one token of its own name. Whitespace is left out unless -IncludeWhitespace asks for it, since a pattern never matches it.

Alias: `Get-TxToken`

```powershell
Get-TrexToken [-InputObject] <string> [-IncludeWhitespace] [-Library <Library>] [<CommonParameters>]
Get-TrexToken -Path <string[]> [-IncludeWhitespace] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-IncludeWhitespace` | switch |  | Writes the whitespace between tokens too. |
| `-InputObject` | string | by value | The text to lex; each string piped in is lexed on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-Path` | string[] |  | Files to lex; wildcards expand. |

Writes [Trex.Token](../types/#trextoken).

### Get-TrexRecord

Splits an input into records: lines, paragraphs, blocks, or the other units trex reads a record as.

-Unit takes `line`, `paragraph`, `file`, `period`, `seam`, `bind`, `auto`, `texture`, `shape`, `block` or `unit`, as the trex command's --record does.

Alias: `Get-TxRecord`

```powershell
Get-TrexRecord [-InputObject] <string> [[-Unit] <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-InputObject` | string | by value | The text to split. |
| `-Unit` | string |  | What a record is; `line` when absent. |

Writes [Trex.Record](../types/#trexrecord).

### Get-TrexRecordShape

Lists the record shapes an input repeats, most frequent first: each distinct sequence of token kinds once, with how many records have it.

A position whose text varies across the records is written as its kind. A record is a line unless -Unit, -RecordStart or -RecordSpan says what it is, and every string piped in, or every file named, is read as one stream. -Rare keeps only the shapes that cover fewer records than the cut: the mean when -Cut is absent, a count such as `5`, or a share such as `1%`. -Against reads other inputs the same way and marks each shape novel where no shape of theirs would accept its records, and -Novel keeps only those.

-Head, -Tail and -Lines read only the first records of each input, its last, or a range of them, counted in the unit a record is; -First and -Last are -Head and -Tail. -Against reads its inputs whole.

Alias: `Get-TxRecordShape`

```powershell
Get-TrexRecordShape [-InputObject] <string> [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-Against <string[]>] [-Novel] [-Rare] [-Cut <string>] [-Hidden] [-NoIgnore] [-Binary] [-Library <Library>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [<CommonParameters>]
Get-TrexRecordShape -Path <string[]> [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-Against <string[]>] [-Novel] [-Rare] [-Cut <string>] [-Hidden] [-NoIgnore] [-Binary] [-Library <Library>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [<CommonParameters>]
Get-TrexRecordShape -LiteralPath <string[]> [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-Against <string[]>] [-Novel] [-Rare] [-Cut <string>] [-Hidden] [-NoIgnore] [-Binary] [-Library <Library>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Against` | string[] |  | Files or directories whose records each shape is compared with, read as one stream as the input is. |
| `-Binary` | switch |  | Reads files that hold a NUL byte, which a walk treats as binary. |
| `-Cut` | string |  | The cut a rare shape falls under: a count, a share such as `1%`, or `mean`. |
| `-Head` | uint |  | Reads the first this many records of each input, reading a file no further. |
| `-Hidden` | switch |  | Reads hidden files and directories a walk would skip. |
| `-InputObject` | string | by value | The text to read; every string piped in is read into one stream. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms a record pattern is compiled against, in place of the session's. |
| `-Lines` | object |  | Reads a range of records of each input, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. |
| `-LiteralPath` | string[] | by name | Files or directories to read as one stream, read as written, as Get-ChildItem pipes them. |
| `-NoIgnore` | switch |  | Reads files an ignore rule excludes. |
| `-Novel` | switch |  | Writes only the shapes no shape of -Against would accept. |
| `-Path` | string[] |  | Files or directories to read as one stream; wildcards expand. |
| `-Rare` | switch |  | Writes only the rare shapes. |
| `-RecordSpan` | object |  | A pattern whose every match is a record. |
| `-RecordStart` | object |  | A pattern whose every match starts a record that runs to the next. |
| `-Tail` | uint |  | Reads the last this many records of each input, reading a file backward from its end. |
| `-Unit` | string |  | What a record is: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph`, `file` or `block`. |

Writes [Trex.RecordShape](../types/#trexrecordshape).

### Find-TrexRecord

Finds the records of an input that hold all, any, none or at least some number of the patterns given, and none of the patterns -Not gives.

A record is a line unless -Unit names another unit: `paragraph`, `file`, `period`, `seam`, `bind`, `auto`, `texture`, `shape`, `block` or `unit`, as Get-TrexRecord reads them. -RecordStart makes a record the run from one match of a pattern to the next, and -RecordSpan each match of one. -Any is the rule when none is named. Every pattern is scanned once over the whole input under the atoms in force.

Alias: `Find-TxRecord`

```powershell
Find-TrexRecord [-Pattern] <Object[]> [-InputObject] <string> [-All] [-Any] [-None] [-AtLeast <uint>] [-Not <Object[]>] [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-MaxCount <uint>] [-Library <Library>] [<CommonParameters>]
Find-TrexRecord [-Pattern] <Object[]> -Path <string[]> [-All] [-Any] [-None] [-AtLeast <uint>] [-Not <Object[]>] [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-MaxCount <uint>] [-Library <Library>] [<CommonParameters>]
Find-TrexRecord [-Pattern] <Object[]> -LiteralPath <string[]> [-All] [-Any] [-None] [-AtLeast <uint>] [-Not <Object[]>] [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-MaxCount <uint>] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-All` | switch |  | Keeps the records holding every pattern. |
| `-Any` | switch |  | Keeps the records holding at least one pattern. |
| `-AtLeast` | uint |  | Keeps the records holding at least this many of the patterns. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms the patterns are compiled against and scanned under, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-MaxCount` | uint |  | Writes at most this many records of each input. |
| `-None` | switch |  | Keeps the records holding none of the patterns. |
| `-Not` | object[] |  | Patterns a record must not hold. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-Pattern` | object[] |  | The patterns a record is held to: trex source text or Trex.Pattern objects. |
| `-RecordSpan` | object |  | A pattern whose every match is a record. |
| `-RecordStart` | object |  | A pattern whose every match starts a record that runs to the next. |
| `-Unit` | string |  | What a record is: `line` when absent. |

Writes [Trex.RecordHit](../types/#trexrecordhit).

### ConvertTo-TrexPattern

Infers the most specific trex pattern every example matches, verified against each.

Examples piped in are gathered and the pattern is written once, after the last. -NotExample gives texts the pattern must not match, which is what lets a position report a range of values rather than its kind.

With -Marked, -Field or -MarksInLines it builds the pattern that extracts named fields from every shape of the examples instead, and writes a Trex.BuiltPattern: -Marked gives lines with each value to extract written `{name:text}`, ConvertFrom-String's markup; -Field names a field by a value it takes, or several, found wherever they stand; and -MarksInLines reads marks in the examples themselves. The pattern matches whole lines unless -Unanchored, and is verified against every example. A word field whose values share a constant part is spelled as its byte shape, `` `KB[0-9]{7}` `` for KB5031354, unless -NoMint; -MintShapes declares it as a named shape instead, held in Declarations.

Alias: `ConvertTo-TxPattern`

```powershell
ConvertTo-TrexPattern [[-Example] <string[]>] [-NotExample <string[]>] [-Anchored] [-Marked <string[]>] [-Field <Object>] [-MarksInLines] [-Unanchored] [-NoMint] [-MintShapes] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Anchored` | switch |  | Wraps the pattern so it must match a whole input. |
| `-Example` | string[] | by value | The texts the pattern must match; a blank line among them holds no token and is left out. |
| `-Field` | object |  | Fields named by value: a dictionary of each field's name and a value it takes, or several, found wherever they stand in the examples. |
| `-Marked` | string[] |  | Lines with each value to extract written `{name:text}`; given, the pattern extracts those fields from every shape of the examples. |
| `-MarksInLines` | switch |  | Reads marks inside the examples themselves. |
| `-MintShapes` | switch |  | Declares each word field's byte shape as a named shape the pattern reads, in place of an inline byte atom. |
| `-NoMint` | switch |  | Spells every word field as its kind, with no byte shape. |
| `-NotExample` | string[] |  | Texts the pattern must not match. |
| `-Unanchored` | switch |  | Lets a built pattern's match start and end inside a longer line. |

Writes `string`, or [Trex.BuiltPattern](../types/#trexbuiltpattern) where a field is marked or named.

### ConvertFrom-TrexText

Converts lines of text to objects, one per line a pattern reads, with a property per field, as ConvertFrom-String does from a template.

-Pattern is a pattern ConvertTo-TrexPattern built, whose fields are read as it placed them and cast to their marks' [type] as PowerShell casts, or a Trex.Pattern or source text, whose registers are the fields. -Marked builds the pattern from the marked lines, a template of several lines included, and every line piped in, as ConvertFrom-String -TemplateContent learns from its template, then converts every line piped in. A string of several lines gives an object per line the pattern reads, and a line it does not read gives none. Where a mark writes a field `{name*:text}`, a line holding it begins a record, the lines after it join the record across the strings piped in, and one object is written per record, as ConvertFrom-String reads one; a field a record lacks is `$null`. A field a line marks more than once, and a register bound under a repetition, is an array of every value it holds, each cast. A field marked inside another, and a register bound inside another, is a property of an object holding the outer one's Text. Where a template line marks a starred field more than once, each record a line repeats is an object, carrying the line's other fields.

Alias: `ConvertFrom-TxText`

```powershell
ConvertFrom-TrexText [-Pattern] <Object> [-InputObject] <string> [-Library <Library>] [<CommonParameters>]
ConvertFrom-TrexText [-InputObject] <string> -Marked <string[]> [-Field <Object>] [-NotExample <string[]>] [-NoMint] [-MintShapes] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Field` | object |  | Fields named by value, as ConvertTo-TrexPattern -Field takes them. |
| `-InputObject` | string | by value | The text to convert; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms a pattern is compiled against, in place of the session's. |
| `-Marked` | string[] |  | Lines with each value to extract written `{name:text}`; the pattern is built from them and every line piped in. |
| `-MintShapes` | switch |  | Declares each word field's byte shape as a named shape the built pattern reads, in place of an inline byte atom. |
| `-NoMint` | switch |  | Spells every word field of the built pattern as its kind, with no byte shape. |
| `-NotExample` | string[] |  | Texts the built pattern must not match. |
| `-Pattern` | object |  | The pattern: one ConvertTo-TrexPattern built, a Trex.Pattern, or source text. |

Writes an object per line read, with a property per field.

### ConvertTo-TrexLiteral

Escapes a text so a trex pattern matches it literally.

Alias: `ConvertTo-TxLiteral`

```powershell
ConvertTo-TrexLiteral [-InputObject] <string> [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-InputObject` | string | by value | The text to escape. |

Writes `string`.
