---
title: Records and tokens
linkTitle: Records and tokens
weight: 40
---

# Records and tokens

These cmdlets read an input as trex reads it: `Get-TrexToken` as tokens, `Get-TrexRecord` as
records, `Get-TrexRecordShape` as the record shapes it repeats (the module's `trex templates`),
and `Find-TrexRecord` as the records holding a combination of patterns. `ConvertTo-TrexPattern`
is the module's `trex infer`, and `ConvertFrom-TrexText` turns lines into objects with a
pattern it built. The examples read these files:

```powershell
PS> Get-Content ./logs/access.log
10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
PS> Get-Content ./logs/today.log
10.0.0.9 GET /index.html 200 4096
10.0.0.9 GET /admin 403 0
worker 3 restarted after 2 failures
```

## Tokens

Each token carries its kind and its text parsed as that kind, as a register's value is:

```powershell
PS> Get-TrexToken 'retry 3 times in 1500ms from 10.0.0.1'

Kind     Text     Start Length Value
----     ----     ----- ------ -----
word     retry    0     5
number   3        6     1      3
word     times    8     5
word     in       14    2
duration 1500ms   17    6      00:00:01.5000000
word     from     24    4
ip       10.0.0.1 29    8      10.0.0.1

PS> Get-TrexToken 'f(a, [b])' | Select-Object Kind, Text

Kind  Text
----  ----
word  f
open  (
word  a
punct ,
open  [
word  b
close ]
close )
```

Whitespace is left out, since a pattern never matches it, unless `-IncludeWhitespace` asks for
it.

## Records

`-Unit` takes `line`, `paragraph`, `file`, `period`, `seam`, `bind`, `auto`, `texture`, `shape`,
`block` or `unit`, as the command line's `--record` does:

```powershell
PS> Get-TrexRecord "a 1`nb 2`n`nc 3" -Unit paragraph | Format-List

Text   : a 1
         b 2
Start  : 0
Length : 7

Text   : c 3
Start  : 9
Length : 3

PS> Get-TrexRecord "a 1`nb 2`n`nc 3" | Select-Object Start, Length, Text

Start Length Text
----- ------ ----
    0      3 a 1
    4      3 b 2
    8      0
    9      3 c 3
```

## Record shapes

A record shape is the sequence of token kinds a set of records shares, with each position whose
text varies written as its kind. Every string piped in, or every file named, is read as one
stream, and the shapes come most frequent first:

```powershell
PS> Get-TrexRecordShape -Path ./logs/access.log

Count    : 4
Readable : <ip> <word> <path> <number> <number>
Pattern  : \I \W \L \N \N
Rare     : False
Novel    :
Records  : {0, 1, 2, 3}

PS> Get-TrexRecordShape -Path ./logs/access.log, ./logs/today.log | Select-Object Count, Readable, Rare

Count Readable                              Rare
----- --------                              ----
    6 <ip> <word> <path> <number> <number> False
    1 worker 3 restarted after 2 failures   True
```

`-Head`, `-Tail` and `-Lines` read only part of each input, counted in the records the shapes
are made of:

```powershell
PS> Get-TrexRecordShape -Path ./logs/access.log -Head 2 | Select-Object Count, Readable

Count Readable
----- --------
    2 <ip> GET <path> <number> <number>
```

`-Rare` keeps the shapes that cover fewer records than the cut: the mean when `-Cut` is absent,
a count such as `2`, or a share such as `10%`. `-Against` reads other inputs the same way and
marks each shape `Novel` where no shape of theirs would accept its records, and `-Novel` keeps
only those, which is how a new kind of line is found in today's log:

```powershell
PS> Get-TrexRecordShape -Path ./logs/today.log -Against ./logs/access.log | Select-Object Count, Readable, Novel

Count Readable                              Novel
----- --------                              -----
    2 10.0.0.9 GET <path> <number> <number> False
    1 worker 3 restarted after 2 failures    True

PS> Get-TrexRecordShape -Path ./logs/today.log -Against ./logs/access.log -Novel | Select-Object Readable, Records

Readable                            Records
--------                            -------
worker 3 restarted after 2 failures {2}
```

## Record queries

`Find-TrexRecord` keeps the records holding all of the patterns with `-All`, any with `-Any`, the
default, none with `-None`, or at least so many with `-AtLeast`, and drops a record holding one
of `-Not`:

```powershell
PS> Find-TrexRecord '"GET"', '\N{400..599}' -Path ./logs/access.log, ./logs/today.log -All | Select-Object LineNumber, Text

LineNumber Text
---------- ----
         3 10.0.1.9 GET /missing 404 312
         2 10.0.0.9 GET /admin 403 0

PS> Find-TrexRecord '"GET"' -Not '"200"' -Path ./logs/access.log | Select-Object LineNumber, Text, Patterns

LineNumber Text                          Patterns
---------- ----                          --------
         2 10.0.0.7 GET /login 302 0     {"GET"}
         3 10.0.1.9 GET /missing 404 312 {"GET"}

PS> Find-TrexRecord '\I', '\N{400..599}', '"POST"' -Path ./logs/access.log -AtLeast 2 | Select-Object LineNumber, Patterns

LineNumber Patterns
---------- --------
         3 {\I, \N{400..599}}
         4 {\I, "POST"}
```

## Inferred patterns

`ConvertTo-TrexPattern` writes the most specific pattern every example matches, verified against
each. `-NotExample` gives texts it must not match, which is what lets a position report a range
of values rather than its bare kind, and `-Anchored` makes it match a whole input:

```powershell
PS> ConvertTo-TrexPattern 'GET /a 200', 'POST /b 404'
\W "/" \W \N

PS> ConvertTo-TrexPattern 'GET /a 200', 'GET /b 204' -NotExample 'GET /c 500'
"GET" "/" \W \N{200..299}

PS> 'x (y)' | ConvertTo-TrexLiteral
"x" "(" "y" ")"
```

With `-Marked`, `-Field` or `-MarksInLines`, `ConvertTo-TrexPattern` builds the pattern that
extracts named fields from every shape of the examples, as `trex infer --mark` does, and writes
a [Trex.BuiltPattern](../types/#trexbuiltpattern) whose string form is the pattern. `-Marked`
gives lines with each value to extract written `{name:text}`, the markup of
`ConvertFrom-String` templates, a `[type]` included; `-Field` names a field by a value it takes,
or several, in a dictionary, an `[ordered]` one keeping its order; and `-MarksInLines` reads
marks in the examples themselves. The pattern matches whole lines unless `-Unanchored`. A
word field whose values share a constant part is spelled as that byte shape, as `trex infer`
spells it, unless `-NoMint`: `kb` below is `` `KB[0-9]{7}` ``, so a title ending in
`(KB50313541)` gives no object. `-MintShapes` declares the shape by name instead; the built
pattern's `Declarations` holds each `shape` line, and `ConvertFrom-TrexText` reads the pattern
under them.
`ConvertFrom-TrexText` then writes an object per line it reads, each field typed as its mark
says; with `-Marked` it builds and converts in one step, as `ConvertFrom-String
-TemplateContent` does:

```powershell
PS> $titles = '2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB5031354)', '2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)', 'Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)'
PS> $built = $titles | ConvertTo-TrexPattern -Marked '{month:2023-10} Cumulative Update for Windows {[int]os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})'
PS> $built.Fields | Format-Table

Name    Accessor TypeName StartsRecord  List Repeats Parent Template
----    -------- -------- ------------  ---- ------- ------ --------
month                            False False   False        ${month}
os               int             False False   False        ${os}
version                          False False   False        ${version}
kb                               False False   False        ${kb}

PS> $titles | ConvertFrom-TrexText $built

month   os version kb
-----   -- ------- --
2023-10 11 22H2    KB5031354
2020-01  7         KB4534310
        10 1607    KB4103720

PS> 'apples 42', 'pears 7' | ConvertFrom-TrexText -Marked '{fruit:apples} {[int]qty:42}'

fruit  qty
-----  ---
apples  42
pears    7
```

A built pattern is built once and applied from then on. `$built.Pattern`, which is also its
string form, goes wherever a pattern is taken, and `$built.File` is a pattern file whose `fields`
line keeps what the marks said beyond the pattern: each field's `[type]`, record start, accessor
and order. Saved and imported, `ConvertFrom-TrexText '\{extract}'` writes the objects the built
pattern writes, with no template and no build:

```powershell
PS> $built.File | Set-Content ./updates.trex
PS> Import-TrexAtom ./updates.trex
PS> $titles | ConvertFrom-TrexText '\{extract}'

month   os version kb
-----   -- ------- --
2023-10 11 22H2    KB5031354
2020-01  7         KB4534310
        10 1607    KB4103720
```

A `[type]` casts the value as PowerShell casts it, the cast `ConvertFrom-String` makes, so a
saved template gives the same values and .NET types: `[int]` an `Int32`, `[decimal]` a `Decimal`,
and `[bool]` true for any text but the empty, `false` included. A template may be one text of
several lines, as `ConvertFrom-String -TemplateContent` takes it. A field written `{name*:text}`
begins a record: a line holding it starts one, the lines after it join it across the strings
piped in, and one object is written per record, the first value of each field kept and a line
before the first record left out, as `ConvertFrom-String` reads records. A field a record lacks
is `$null`, where `ConvertFrom-String` leaves the property off:

```powershell
PS> $template = "Name: {Name*:Phoebe Cat}`nPhone: {phone:425-123-6789}`n`nName: {Name*:Lucky Shot}`nPhone: {phone:206-987-4321}"
PS> "Name: Elephant Wise`nPhone: 425-888-7766`nName: Wise Owl" | ConvertFrom-TrexText -Marked $template

Name          phone
----          -----
Elephant Wise 425-888-7766
Wise Owl
```

A field marked more than once in one line is a list: its `List` is true, and its property is
an array of every value the line holds, each cast as its mark says, a line with one value
giving an array of one. A pattern's register bound under a repetition reads the same way.

```powershell
PS> 'ports 22 open', 'ports 8080, 8443, 9000 open' | ConvertFrom-TrexText -Marked 'ports {[int]port:80}, {[int]port:443} open'

port
----
{22}
{8080, 8443, 9000}
```

A mark inside a mark is a property of an object: the outer field's property holds its Text and
a property per field inside it, each cast as its mark says, and the built pattern's fields name
their `Parent`. ConvertFrom-String reads such a template only with the outer mark starred, and
then wraps the object in a one-item list; trex reads it with or without the star, as the object
itself. A pattern's register bound inside another reads the same way.

```powershell
PS> $pages = '5 of 9', '6 of 9' | ConvertFrom-TrexText -Marked '{Line:{[int]n:1} of {[int]m:3}}'
PS> $pages[0].Line

Text   n m
----   - -
5 of 9 5 9
```

A mark may span the lines of a template, as a `ConvertFrom-String` template writes one record of
several lines. Starred, it writes an object per record whose property holds the fields inside it
and a Text of the record's lines joined with a newline, where `ConvertFrom-String` wraps the
fields in a one-item list:

```powershell
PS> $template = "{Person*:Name: {Name:Phoebe Cat}`nPhone: {Phone:425-123-6789}}"
PS> $people = "Name: Wise Owl`nPhone: 425-888-7766`nName: Big Bird`nPhone: 206-555-0100" | ConvertFrom-TrexText -Marked $template
PS> $people.Person | Select-Object Name, Phone

Name     Phone
----     -----
Wise Owl 425-888-7766
Big Bird 206-555-0100

PS> $people[1].Person.Text
Name: Big Bird
Phone: 206-555-0100
```

A template line that marks a starred field more than once repeats its record, and each record a
line holds is an object, as `ConvertFrom-String` reads two records on one line. A field outside
the records describes the line, so each record of the line carries it, and the fields of the
record have `Repeats` true:

```powershell
PS> 'day Tue: Wise Owl (87) end', 'day Wed: Elmo Red (3); Oscar Grouch (9) end' | ConvertFrom-TrexText -Marked 'day {day:Mon}: {Name*:Phoebe Cat} ({[int]age:6}); {Name*:Lucky Shot} ({[int]age:12}) end'

day Name         age
--- ----         ---
Tue Wise Owl      87
Wed Elmo Red       3
Wed Oscar Grouch   9
```

## Cmdlets

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
