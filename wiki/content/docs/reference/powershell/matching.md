---
title: Matching
linkTitle: Matching
weight: 10
---

# Matching

`Select-TrexMatch` is the module's `trex scan`: it finds every match of a pattern in text, files
or directories and writes each as a [`Trex.Match`](../types/#trexmatch). The examples read these
files:

```powershell
PS> Get-Content ./logs/app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
PS> Get-Content ./logs/access.log
10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
PS> Get-Content ./src/main.rs
fn main() {
    let answer = 42;
    println!("{}", answer);
}
PS> Get-Content ./people.csv
name,age,score
alice,30,95
bob,25,88
carol,41,73
dan,38,91
PS> Get-Content ./secrets.trex
let mail = \E
let addr = \I
```

## Matches

A match carries where it stands, the text it covers, and each register the pattern binds, by
name in `Captures` and in full in `Groups`, each with its kind and its value:

```powershell
PS> Select-TrexMatch '\E:e' -InputObject 'ping bob@x.com and ann@y.org'

Path        :
LineNumber  : 1
Column      : 6
Start       : 5
Length      : 9
Text        : bob@x.com
Captures    : {[e, bob@x.com]}
Groups      : {e=bob@x.com}
Pattern     :
PreContext  : {}
PostContext : {}
Explanation :

Path        :
LineNumber  : 1
Column      : 20
Start       : 19
Length      : 9
Text        : ann@y.org
Captures    : {[e, ann@y.org]}
Groups      : {e=ann@y.org}
Pattern     :
PreContext  : {}
PostContext : {}
Explanation :

PS> Select-TrexMatch '\R:t' -InputObject 'retried after 1500ms' | Select-Object -ExpandProperty Groups

Name   : t
Text   : 1500ms
Start  : 14
Length : 6
Kind   : duration
Value  : 00:00:01.5000000
Items  : {}
```

A register's `Value` is its text read as its kind, in the .NET type that holds it, so a duration
sorts, sums and compares as a `TimeSpan`:

```powershell
PS> Select-TrexMatch '\R:took' -Path ./logs/app.log | Select-Object LineNumber, Text, @{ n = 'Value'; e = { $_.Groups[0].Value } }

LineNumber Text   Value
---------- ----   -----
         1 120ms  00:00:00.1200000
         3 1450ms 00:00:01.4500000
         5 95ms   00:00:00.0950000

PS> Select-TrexMatch '\R:took' -Path ./logs/app.log | Measure-Object -Property { $_.Groups[0].Value.TotalMilliseconds } -Sum | Select-Object Count, Sum

Count     Sum
-----     ---
    3 1665.00
```

A value predicate reads a token's value in its own units, and `-Context` adds the lines around
each match:

```powershell
PS> Select-TrexMatch '\N{>=1000}' -Path ./logs -Context 1 | Select-Object LineNumber, Text, PreContext, PostContext

LineNumber Text PreContext PostContext
---------- ---- ---------- -----------
         1 5120 {}         {10.0.0.7 GET /login 302 0}
```

`-Raw` writes the matched text alone, `-List` each input's first match, `-MaxCount` at most so
many of each input, and `-WholeLine` keeps a match only where it covers its line:

```powershell
PS> Select-TrexMatch '\I' -Path ./logs/access.log -Raw
10.0.0.5
10.0.0.7
10.0.1.9
10.0.0.5

PS> Select-TrexMatch '\I' -Path ./logs -List | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, LineNumber, Text

File       LineNumber Text
----       ---------- ----
access.log          1 10.0.0.5
app.log             1 10.0.0.5

PS> Select-TrexMatch '\I' -Path ./logs/access.log -MaxCount 2 -Raw
10.0.0.5
10.0.0.7

PS> Select-TrexMatch '\W' -InputObject "alpha`nbeta gamma" -WholeLine -Raw
alpha
```

`-NotMatch` writes the lines no match touches, each as a match of its whole line:

```powershell
PS> Select-TrexMatch '"INFO"' -Path ./logs/app.log -NotMatch | Select-Object LineNumber, Text

LineNumber Text
---------- ----
         2 2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
         4 2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declin…
```

## Strings piped in

Strings piped in are the lines of one input, as `Get-Content` writes a file's, each ending a line
unless it ends with a newline of its own. `LineNumber` counts across them as `Select-String`
numbers them, `Start` counts from the first string, a match may run from one string into the
next, and `-Context` reads the strings around a match. Where the report writes the matches
alone, each is written once the text holding it has arrived, so `Get-Content -Wait` output is
matched as it lands; a report read over the whole input, such as `-Context`, `-NotMatch` or
`-Count`, is written after the last string. `-PerString` scans each string as an input of its
own:

```powershell
PS> 'ping bob@x.com', 'nothing here', 'and ann@y.org' | Select-TrexMatch '\E' | Select-Object LineNumber, Start, Text

LineNumber Start Text
---------- ----- ----
         1     5 bob@x.com
         3    32 ann@y.org

PS> 'ping bob@x.com', 'nothing here', 'and ann@y.org' | Select-TrexMatch '\E' -PerString | Select-Object LineNumber, Start, Text

LineNumber Start Text
---------- ----- ----
         1     5 bob@x.com
         1     4 ann@y.org
```

`-Format` writes a report template rendered at each match in place of the match. It reads the
registers, `${line}`, `${col}` and `${path}`:

```powershell
PS> Select-TrexMatch '\I:ip \W:verb' -Path ./logs/access.log -Format '${line}: ${verb} from ${ip:octet1-3}'
1: GET from 10.0.0
2: GET from 10.0.0
3: GET from 10.0.1
4: POST from 10.0.0
```

## Parts of a file

`Get-TrexLine` is the module's `trex head`, `trex tail` and `trex lines`: `-Head` writes a file's
first lines, `-Tail` its last, read backward from its end, and `-Lines` a range, `"2..3"`,
`"2.."`, `"..3"` or PowerShell's own `2..3`; `-First` and `-Last` are `-Head` and `-Tail`. Each
line is a [`Trex.Line`](../types/#trexline) carrying its number in the file, and `-Passthru`
writes the lines as strings:

```powershell
PS> Get-TrexLine -Path ./logs/app.log -Tail 2 | Select-Object LineNumber, Text

LineNumber Text
---------- ----
         4 2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declin…
         5 2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms

PS> Get-TrexLine -Path ./people.csv -Lines 2..3 -Passthru
alice,30,95
bob,25,88
```

The same three parameters read part of each input for `Select-TrexMatch`, `Test-TrexMatch`,
`Edit-TrexText`, `Protect-TrexText`, `Group-TrexMatch`, `Invoke-TrexRule` and
`Get-TrexRecordShape`, counted in lines or in the records `-Unit` names. A match counts only
where it lies wholly inside, and stands at the file's own line and offset:

```powershell
PS> Select-TrexMatch '\R:took' -Path ./logs/app.log -Tail 2 | Select-Object LineNumber, Start, Text

LineNumber Start Text
---------- ----- ----
         5   346 95ms

PS> Test-TrexMatch '"ERROR"' -Path ./logs/app.log -Head 3
False
```

`-Follow` goes on reading each file as it grows, after its tail, its open range or the whole of
it, and writes each new line or match once nothing arriving later can change it, until the
pipeline is stopped. A file cut shorter or replaced under its name is read again from its
start, which is said as a warning. `Get-TrexLine -Path ./logs/app.log -Tail 0 -Follow` writes
each line the log gains.

## Counts, files and statistics

`-Count` writes how many lines of each file hold a match and `-CountMatches` how many matches it
holds, each as a [`Trex.MatchCount`](../types/#trexmatchcount); over text the count covers every
string piped in:

```powershell
PS> Select-TrexMatch '\N' -Path ./logs -Count | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, Count

File       Count
----       -----
access.log     4
app.log        1

PS> Select-TrexMatch '\N' -Path ./logs -CountMatches | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, Count

File       Count
----       -----
access.log     8
app.log        1

PS> ('a 1', 'b', 'c 3 4' | Select-TrexMatch '\N' -Count).Count
2
```

`-FilesWithMatches` and `-FilesWithoutMatch` write the path of each file holding a match or none,
and `-Sort` orders the files read, by path or by the time each was written, read or created:

```powershell
PS> Select-TrexMatch '\E' -Path . -FilesWithMatches | Split-Path -Leaf
app.log

PS> Select-TrexMatch '\E' -Path . -FilesWithoutMatch -Sort Path -Descending | Split-Path -Leaf
main.rs
secrets.trex
people.csv
access.log
```

`-Quiet` writes one boolean after the last input, as `Test-TrexMatch` does, and `-Stats` adds a
[`Trex.ScanStats`](../types/#trexscanstats) after the report:

```powershell
PS> Select-TrexMatch '\E' -Path ./logs -Quiet
True

PS> Test-TrexMatch '\{uuid}' -Path ./logs/app.log
False

PS> Select-TrexMatch '\I' -Path ./logs -Stats | Select-Object -Last 1 | Select-Object Matches, MatchedLines, FilesWithMatches, FilesSearched, BytesSearched

Matches          : 7
MatchedLines     : 7
FilesWithMatches : 2
FilesSearched    : 2
BytesSearched    : 469
```

`-Stats` also reads the trace trex keeps while the cmdlet runs: the tokens the lexes produced,
the bytes the device scanned, and how many inputs each rung of the scan ladder answered. Its four
times, `Searching`, `Elapsed`, `Lexing` and `Matching`, are the run's own:

```powershell
PS> Select-TrexMatch '\I' -Path ./logs -Stats | Select-Object -Last 1 | Format-List TokensLexed, DeviceBytes, Routes

TokensLexed : 62
DeviceBytes : 0
Routes      : {a route, not the engine: 2}
```

## From `trex scan`

Each output flag of `trex scan` has a switch of the same meaning, and most have a way of saying
the same thing in PowerShell's own terms, over the objects the cmdlets write. `P` is a pattern and
`X` a path:

| `trex scan` | Switch | In PowerShell's own terms |
|---|---|---|
| `--count` | `-Count` | `Select-TrexMatch P -Path X \| Select-Object Path, LineNumber -Unique \| Group-Object Path -NoElement` |
| `--count-matches` | `-CountMatches` | `Select-TrexMatch P -Path X \| Group-Object Path -NoElement` |
| `-l` | `-FilesWithMatches` | `Select-TrexMatch P -Path X -List \| ForEach-Object Path` |
| `-L` | `-FilesWithoutMatch` | `Get-TrexFile X \| Where-Object { -not (Test-TrexMatch P -Path $_.Path) } \| ForEach-Object Path` |
| `-q` | `-Quiet` | `Test-TrexMatch P -Path X` |
| `--sort modified` | `-Sort Modified` | `Get-TrexFile X \| Get-Item \| Sort-Object LastWriteTime \| Select-TrexMatch P` |
| `--json` | `-Json` | `Select-TrexMatch P -Path X \| ConvertTo-Json`, which writes each `Trex.Match` in its own shape |
| `--color` | `-Color` | the `Trex.Match` objects, as the host formats them |
| `--passthru` | `-Passthru` | none: the objects are the matches, not the lines around them |
| `--stats` | `-Stats` | `Select-TrexMatch P -Path X \| Measure-Object`, which counts the matches alone |

## The command's own text

`-Json`, `-Color` and `-Passthru` write what `trex scan` prints for the same scan, a string a
line, so a script written against the command's output reads the module's unchanged. `-Json` is
`--json`: an array for each input; over strings piped in, one object a match as each settles, as
the command writes a stream on its standard input; and over a directory, several files or paths
piped in, one array whose objects name the path, line and column. `-ValueSpelling` and
`-DurationUnit` spell a typed register's value as `--values` and `--duration-unit` do:

```powershell
PS> Select-TrexMatch '\E' -InputObject 'ping bob@x.com and ann@y.org' -Json
[{"start":5,"end":14,"text":"bob@x.com","captures":{}},{"start":19,"end":28,"text":"ann@y.org","captures":{}}]

PS> Select-TrexMatch '\E' -Path ./logs -Json
[{"path":"C:\\Temp\\demo\\logs\\app.log","line":4,"col":40,"start":235,"end":244,"text":"bob@x.com","captures":{}}]

PS> Select-TrexMatch '\R:t' -InputObject 'retried after 1500ms' -Json -DurationUnit Milliseconds
[{"start":14,"end":20,"text":"1500ms","captures":{"t":{"text":"1500ms","value":1500}}}]
```

`-Color` writes the text report painted as `--color` paints it, at the depth `-ColorDepth` names
and with each role as `-Colors` sets it; `-Context`, `-Explain` and `-NotMatch` add their lines
as the command's flags do. `-ColorDepth None` writes the report unpainted, as here:

```powershell
PS> Select-TrexMatch '\E' -InputObject 'ping bob@x.com and ann@y.org' -Color -ColorDepth None
[5..14] "bob@x.com"
[19..28] "ann@y.org"

PS> Select-TrexMatch '\N{>=1000}' -Path ./logs -Context 1 -Color -ColorDepth None
C:\Temp\demo\logs\access.log:1:30: "5120"
C:\Temp\demo\logs\access.log-2-10.0.0.7 GET /login 302 0

PS> Select-TrexMatch '"INFO"' -Path ./logs/app.log -NotMatch -Color -ColorDepth None
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
```

`-Passthru` writes every line with the matches painted, as `--passthru` prints to a console;
where the report names its inputs, a line holding a match is joined to its path by `:` and the
rest by `-`:

```powershell
PS> Select-TrexMatch '"GET"' -Path ./logs/access.log, ./src/main.rs -Passthru -ColorDepth None
C:\Temp\demo\logs\access.log:1:10.0.0.5 GET /index.html 200 5120
C:\Temp\demo\logs\access.log:2:10.0.0.7 GET /login 302 0
C:\Temp\demo\logs\access.log:3:10.0.1.9 GET /missing 404 312
C:\Temp\demo\logs\access.log-4-10.0.0.5 POST /login 200 88
C:\Temp\demo\src\main.rs-1-fn main() {
C:\Temp\demo\src\main.rs-2-    let answer = 42;
C:\Temp\demo\src\main.rs-3-    println!("{}", answer);
C:\Temp\demo\src\main.rs-4-}
```

## Records

A count, and `-Context record`, read a record as a line unless `-Unit`, `-RecordStart` or
`-RecordSpan` says otherwise. Here the records are paragraphs:

```powershell
PS> ("a 1`nb 2`n`nc 3`n`nd" | Select-TrexMatch '\N' -Count -Unit paragraph).Count
2

PS> "a 1`nb 2`nc 3`n`nd 4" | Select-TrexMatch '"b"' -Context paragraph | Select-Object Text, PreContext, PostContext

Text PreContext PostContext
---- ---------- -----------
b    {a 1}      {c 3}
```

## Pattern sets

Several patterns scan as one set over one lex, and each match names the pattern that made it in
its `Pattern` property. The members of a pattern file are named by the file, a `let` line by its
name and a bare line by its number:

```powershell
PS> Select-TrexMatch '\E', '\I' -Path ./logs/app.log | Select-Object Pattern, Text

Pattern Text
------- ----
\I      10.0.0.5
\I      10.0.0.7
\E      bob@x.com
\I      10.0.0.5

PS> Select-TrexMatch -PatternFile ./secrets.trex -Path ./logs/app.log | Select-Object Pattern, Text

Pattern Text
------- ----
addr    10.0.0.5
addr    10.0.0.7
mail    bob@x.com
addr    10.0.0.5

PS> Select-TrexMatch -PatternFile ./secrets.trex -Path ./logs/app.log -SingleMatch | Select-Object Pattern, LineNumber, Text

Pattern LineNumber Text
------- ---------- ----
addr             1 10.0.0.5
mail             4 bob@x.com
```

A first argument by position is read as `-Pattern`, so beside `-PatternFile` the text to scan is
piped in or named with `-InputObject`.

## Explanations

`-Explain` fills each match's `Explanation` with the tokens it covers, the checks its guarded
kinds passed, every axis the pattern read at it, and the rung of the scan ladder that answered:

```powershell
PS> (Select-TrexMatch '\T \W' -InputObject '2026-09-27T09:00:04Z ERROR' -Explain).Explanation

Tokens                                       Guards Readings Route
------                                       ------ -------- -----
{timestamp 2026-09-27T09:00:04Z, word ERROR} {}     {}       a route, not the engine

PS> (Select-TrexMatch '\T \W' -InputObject '2026-09-27T09:00:04Z ERROR' -Explain).Explanation.Tokens

Kind      Text
----      ----
timestamp 2026-09-27T09:00:04Z
word      ERROR

PS> (Select-TrexMatch '\{card}' -InputObject 'card 4111 1111 1111 1111' -Explain).Explanation.Guards
creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check

PS> (Select-TrexMatch '\M{>5}' -InputObject 'sizes 12 and 5000000' -Explain).Explanation.Readings | Select-Object Axis, Text, Value

Axis      Text    Value
----      ----    -----
magnitude 5000000 6.70
```

## Compiled patterns

`New-TrexPattern` compiles a pattern once, against the atoms in force when it runs. The
[`Trex.Pattern`](../types/#trexpattern) it writes goes wherever a pattern is taken, and has
methods of its own:

```powershell
PS> $email = New-TrexPattern '\E:e'
PS> $email.IsMatch('mail bob@x.com')
True

PS> $email.Find('ping bob@x.com').Captures.e
bob@x.com

PS> $email.FindAll('bob@x.com, ann@y.org') | Select-Object Start, Text

Start Text
----- ----
    0 bob@x.com
   11 ann@y.org

PS> $email.Replace('mail bob@x.com now', '[${e:domain}]')
mail [x.com] now

PS> $email.Split('a bob@x.com b ann@y.org c')
a
 b
 c

PS> Select-TrexMatch $email -Path ./logs | Select-Object LineNumber, Text

LineNumber Text
---------- ----
         4 bob@x.com
```

## Engines

`-Backend` picks where one pattern's scan runs: `Auto` chooses per input, `Cpu` never probes the
device, and `Gpu` falls back to the CPU where the device cannot take the scan. `-DualGrain` runs
the byte and token grains as a pipeline, and `-ChunkSize` feeds each input in chunks of that many
bytes. Every engine finds the same matches, and how the scan ran is written as verbose output:

```powershell
PS> Select-TrexMatch '\I' -Path ./logs/access.log -Backend Cpu -Raw
10.0.0.5
10.0.0.7
10.0.1.9
10.0.0.5

PS> Select-TrexMatch '\I' -Path ./logs/access.log -ChunkSize 7 -Raw
10.0.0.5
10.0.0.7
10.0.1.9
10.0.0.5
```

## Files and indexes

`Get-TrexFile` lists the files a scan of the paths would read, and scans nothing. `-FileType`
and `-ExcludeFileType` take ripgrep's type names, which `Get-TrexFileType` lists; `-Texture`
and `-ExcludeTexture` keep or drop a file by what its text reads as mostly, which `-Classify`
reports:

```powershell
PS> Get-TrexFile . | Split-Path -Leaf
access.log
app.log
people.csv
secrets.trex
main.rs

PS> Get-TrexFile . -FileType rust, csv | Split-Path -Leaf
people.csv
main.rs

PS> Get-TrexFile . -Classify | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, Texture, Period

File         Texture Period
----         ------- ------
access.log     Table      5
app.log        Table     16
people.csv     Table     10
secrets.trex   Table      5
main.rs         Code      0

PS> Get-TrexFileType rust

Name Globs
---- -----
rust {*.rs}

PS> (Get-TrexFileType).Count
224
```

`New-TrexIndex` writes an index at a tree's root, after which every scan of the tree opens only
the files that can match; `-NoIndex` reads none, and a scan with `-Index` writes one from the
files it reads:

```powershell
PS> New-TrexIndex ./logs | Select-Object Files

Files
-----
    2

PS> Get-TrexIndex ./logs | Select-Object Files

Files
-----
    2

PS> Select-TrexMatch '\E' -Path ./logs -FilesWithMatches | Split-Path -Leaf
app.log
```

## Cmdlets

### Select-TrexMatch

Finds every match of a trex pattern in text, files or directories and writes each as a Trex.Match.

A pattern is trex source text or a Trex.Pattern from New-TrexPattern; a source text is compiled against the session's atoms, or -Library's. Strings piped in are the lines of one input, as Get-Content writes a file's, each ending a line unless it ends with a newline of its own: a match may run from one string into the next, LineNumber counts across them, and -Context reads the strings around a match. Where the report writes the matches alone, each is written once the text holding it has arrived; a report read over the whole input, such as -Context, -NotMatch or -Count, is written after the last string. -PerString scans each string as an input of its own. -Path reads each file whole and scans it in one call, so a large input crosses into trex once per file; a directory is walked with .gitignore and .ignore rules, skipping hidden and binary files, as the trex command walks one.

-Context adds the lines around each match, or the rest of the paragraph, block or other record a unit names; -NotMatch writes the lines no match touches instead, and -WholeLine keeps a match only where it covers its line. -FileType and -Texture keep the walked files of a type or a texture, and -Sort orders the files read.

In place of the matches, -Raw writes their text and -Format a report template rendered at each, which reads `${path}`, `${line}` and `${col}` beside the registers, and `${@axis}` for what -Explain reads. -Count writes how many lines of each file hold a match and -CountMatches how many matches it holds, each as a Trex.MatchCount for a file with any, and over text as one count of every string, after the last. -FilesWithMatches and -FilesWithoutMatch write the path of each file holding a match or none, and -Quiet one boolean. -Stats adds a Trex.ScanStats after the report.

-Json, -Color and -Passthru write what the trex command prints for the same scan, a string a line: -Json its `--json` report, -Color its text report painted as `--color` paints it, and -Passthru every line with the matches painted, as `--passthru` prints it to a console. A report names its inputs as the command's does, over a directory, several paths or paths piped in.

Several patterns scan as one set, each match naming the one that made it in its Pattern property: patterns given one by one are named by their text, and the members of a -PatternFile by the names the file gives, `let name = pattern`, or by their line numbers. -SingleMatch keeps each member's first match. -RequireMatch writes an error when no input held a match, for a script that stops on one.

One pattern's scan runs where -Backend says, the CPU engine or the device, or with -DualGrain as the byte and token grains in a pipeline, or with -ChunkSize over each input fed in chunks; every choice finds the same matches, and how the scan ran is written as verbose output.

Alias: `Select-TxMatch`

```powershell
Select-TrexMatch [[-Pattern] <Object[]>] [-InputObject] <string> [-PatternFile <string>] [-PerString] [-Library <Library>] [-List] [-Raw] [-Quiet] [-Context <string[]>] [-NotMatch] [-WholeLine] [-MaxCount <uint>] [-Format <string>] [-Json] [-ValueSpelling <ValueSpelling>] [-DurationUnit <DurationUnit>] [-Color] [-ColorDepth <ColorDepth>] [-Colors <string[]>] [-Passthru] [-Count] [-CountMatches] [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-Stats] [-SingleMatch] [-RequireMatch] [-Backend <Backend>] [-DualGrain] [-ChunkSize <uint>] [-Explain] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-WhatIf] [-Confirm] [<CommonParameters>]
Select-TrexMatch [[-Pattern] <Object[]>] -Path <string[]> [-PatternFile <string>] [-Library <Library>] [-List] [-Raw] [-Quiet] [-Context <string[]>] [-NotMatch] [-WholeLine] [-MaxCount <uint>] [-Format <string>] [-Json] [-ValueSpelling <ValueSpelling>] [-DurationUnit <DurationUnit>] [-Color] [-ColorDepth <ColorDepth>] [-Colors <string[]>] [-Passthru] [-Count] [-CountMatches] [-FilesWithMatches] [-FilesWithoutMatch] [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-Stats] [-SingleMatch] [-RequireMatch] [-Backend <Backend>] [-DualGrain] [-ChunkSize <uint>] [-Explain] [-Hidden] [-NoIgnore] [-Binary] [-Include <string[]>] [-FileType <string[]>] [-ExcludeFileType <string[]>] [-Sort <SortKey>] [-Descending] [-Texture <RegionKind[]>] [-ExcludeTexture <RegionKind[]>] [-Index] [-NoIndex] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Follow] [-WhatIf] [-Confirm] [<CommonParameters>]
Select-TrexMatch [[-Pattern] <Object[]>] -LiteralPath <string[]> [-PatternFile <string>] [-Library <Library>] [-List] [-Raw] [-Quiet] [-Context <string[]>] [-NotMatch] [-WholeLine] [-MaxCount <uint>] [-Format <string>] [-Json] [-ValueSpelling <ValueSpelling>] [-DurationUnit <DurationUnit>] [-Color] [-ColorDepth <ColorDepth>] [-Colors <string[]>] [-Passthru] [-Count] [-CountMatches] [-FilesWithMatches] [-FilesWithoutMatch] [-Unit <string>] [-RecordStart <Object>] [-RecordSpan <Object>] [-Stats] [-SingleMatch] [-RequireMatch] [-Backend <Backend>] [-DualGrain] [-ChunkSize <uint>] [-Explain] [-Hidden] [-NoIgnore] [-Binary] [-Include <string[]>] [-FileType <string[]>] [-ExcludeFileType <string[]>] [-Sort <SortKey>] [-Descending] [-Texture <RegionKind[]>] [-ExcludeTexture <RegionKind[]>] [-Index] [-NoIndex] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Follow] [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Backend` | [Trex.Backend](../types/#trexbackend) |  | The engine one pattern's scan runs on: Auto when absent, Cpu, or Gpu, which falls back to the CPU with a note where the device cannot take the scan. |
| `-Binary` | switch |  | Reads files that hold a NUL byte, which a walk treats as binary. |
| `-ChunkSize` | uint |  | Feeds each input to one pattern's scan in chunks of this many bytes, as a stream arrives. |
| `-Color` | switch |  | Writes the report as the trex command prints it, a line a string, painted: each match's span and text, or with its path, line and column over a directory, several paths or paths piped in; -Context, -Explain and -NotMatch add their lines as the command's flags do. |
| `-ColorDepth` | [Trex.ColorDepth](../types/#trexcolordepth) |  | The depth -Color and -Passthru paint at, as the trex command's `--color` names it: None, which paints nothing, as `--color never`; Ansi16, Ansi256 or TrueColor. What the environment says when absent, as `--color always` reads it. |
| `-Colors` | string[] |  | A role's paint for -Color and -Passthru, as the trex command's `--colors` takes it: `match:fg:red`, `path:bg:#202020`, `line:style:bold`, `kind:NAME:...` for a token kind, `capture:NAME:...` for a register. |
| `-Context` | string[] |  | The lines to add around each match: one count for both sides, or two, the lines before and the lines after. A record unit in place of a count, such as `paragraph` or `block`, adds the rest of the record holding the match on that side, and `record` names the one -Unit, -RecordStart or -RecordSpan defines. |
| `-Count` | switch |  | Writes how many records of each file hold a match, or under -NotMatch hold none, as a Trex.MatchCount for each file with any; over text, one count of every string, after the last. A record is a line unless -Unit, -RecordStart or -RecordSpan says otherwise. |
| `-CountMatches` | switch |  | Writes how many matches each file holds, as -Count writes the records holding one; under -NotMatch it counts the records none touches. |
| `-Descending` | switch |  | Reverses the order -Sort names. |
| `-DualGrain` | switch |  | Runs one pattern's scan as the byte grain and the token grain in a pipeline, writing their timing as verbose output. |
| `-DurationUnit` | [Trex.DurationUnit](../types/#trexdurationunit) |  | The unit -Json writes a duration in, as the trex command's `--duration-unit` takes it: Nanoseconds when absent, Milliseconds or Seconds. |
| `-ExcludeFileType` | string[] |  | Drops a walked file of one of these types. |
| `-ExcludeTexture` | [Trex.RegionKind](../types/#trexregionkind)[] |  | Drops a walked file whose text reads mostly as one of these. |
| `-Explain` | switch |  | Adds what each match is made of: its tokens' kinds, the checks its guarded kinds passed, and every axis the pattern read at it. |
| `-FilesWithMatches` | switch |  | Writes the path of each file holding a match, or under -NotMatch a record no match touches. |
| `-FilesWithoutMatch` | switch |  | Writes the path of each file holding no match, or under -NotMatch each file whose every record holds one. |
| `-FileType` | string[] |  | Keeps a walked file only when it is of one of these types, under ripgrep's names: `rust`, `py`, `js`, `log` and the rest. |
| `-Follow` | switch |  | After each file's -Tail, its open -Lines range or the whole of it, scans what it gains as it grows, writing each match once nothing that arrives later can change it, until the pipeline is stopped. |
| `-Format` | string |  | A report template to write at each match in place of the match: `${name}` a register, `${path}`, `${line}` and `${col}` where it stands, `${@axis}` an axis the pattern read. |
| `-Head` | uint |  | Scans the first this many lines of each input, or records of -Unit, reading a file no further. |
| `-Hidden` | switch |  | Reads hidden files and directories a walk would skip. |
| `-Include` | string[] |  | Keeps a walked file only when a glob matches it (`*.log`), or drops it for a glob that starts with `!`. |
| `-Index` | switch |  | Writes each directory's index from the files this scan reads, so every later scan of the tree opens only the files that can match. |
| `-InputObject` | string | by value | The text to scan. Strings piped in are the lines of one input, as Get-Content writes a file's, unless -PerString is given. |
| `-Json` | switch |  | Writes the matches as the trex command's `--json` writes them: an array for the text -InputObject gives or a string under -PerString; over strings piped in, each match's object as it settles, as the command writes a stream on its standard input, or one array after the last string where a flag reads the whole input; over a directory, several paths or paths piped in, one array of every match after the last input, each object naming its path, line and column. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms a source-text pattern is compiled against, in place of the session's. |
| `-Lines` | object |  | Scans a range of lines of each input, or records of -Unit, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. A match counts only where it lies wholly inside the part read, and every match stands at the input's own line and offset. |
| `-List` | switch |  | Writes only the first match of each input. |
| `-LiteralPath` | string[] | by name | Files or directories to scan, read as written, as Get-ChildItem pipes them. |
| `-MaxCount` | uint |  | Writes or counts at most this many matches, or lines under -NotMatch, of each input. |
| `-NoIgnore` | switch |  | Reads files an ignore rule excludes. |
| `-NoIndex` | switch |  | Reads no index, whatever the trees hold. |
| `-NotMatch` | switch |  | Writes the lines no match touches, each as a Trex.Match of the whole line with no registers. |
| `-Passthru` | switch |  | Writes every line of each input as the trex command's `--passthru` prints it, a line a string, the matches painted: where the report names its inputs, with its path and line number ahead of it, joined by `:` for a line holding a match and by `-` for the rest. -ColorDepth None writes the lines unpainted. |
| `-Path` | string[] |  | Files or directories to scan; wildcards expand. |
| `-Pattern` | object[] |  | The patterns: trex source text or Trex.Pattern objects; several scan as one set. |
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
| `-ValueSpelling` | [Trex.ValueSpelling](../types/#trexvaluespelling) |  | How -Json spells a typed register's value, as the trex command's `--values` takes it: Exact when absent, a JSON number only where a double holds the value; Natural, a number throughout; or Tagged, its kind and its exact text. |
| `-WholeLine` | switch |  | Keeps a match only where it covers its line, from the first character that is not whitespace to the last. |

Writes [Trex.Match](../types/#trexmatch), `string`, `bool`, [Trex.MatchCount](../types/#trexmatchcount), [Trex.ScanStats](../types/#trexscanstats).

### Test-TrexMatch

Tells whether a trex pattern matches text or the files named.

With several inputs the answer is whether any of them matched, and with several patterns whether any of them did. -Head, -Tail and -Lines test only the first lines of each input, its last, or a range of them, counted in records of -Unit where it names one; a match counts only where it lies wholly inside. -First and -Last are -Head and -Tail.

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
| `-Pattern` | object[] |  | The patterns: trex source text or Trex.Pattern objects. |
| `-PatternFile` | string |  | A pattern file whose members are tested as one set. Text to test beside it is piped in or named with -InputObject, since a first argument by position is read as -Pattern. |
| `-Tail` | uint |  | Tests the last this many lines of each input, or records of -Unit, reading a file backward from its end. |
| `-Unit` | string |  | What -Head, -Tail and -Lines count: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph` or `block`. |

Writes `bool`.

### Get-TrexLine

Writes an input's first lines, its last, or a range of them, as the trex command's head, tail and lines print them.

-Head N writes the first N lines, read no further than the Nth newline; -Tail N the last N, read backward from the file's end; -Lines a range, `"100..200"`, `"100.."`, `"..200"`, or PowerShell's own `100..200`. -Unit counts paragraphs, blocks or another record unit instead of lines. -First and -Last are -Head and -Tail. Each line is a Trex.Line with its number in the input; -Passthru writes the lines as strings, a `==> path <==` header ahead of each file's where several are read, as the trex command prints them.

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
| `-Passthru` | switch |  | Writes the lines as strings, as the trex command prints them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-Tail` | uint |  | Writes the last this many lines, or records of -Unit. |
| `-Unit` | string |  | What -Head, -Tail and -Lines count: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph` or `block`. |

Writes [Trex.Line](../types/#trexline), `string`.

### New-TrexPattern

Compiles a trex pattern against the atoms in force, for reuse across commands and as an object with IsMatch, Find, FindAll, Replace and Split methods.

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

The walk is Select-TrexMatch's: .gitignore and .ignore rules applied, hidden files skipped unless -Hidden asks for them, and -Include and -FileType keeping what it finds, while a file named outright is listed whatever they say. -Texture and -ExcludeTexture read every file listed, named or found, as the trex command's `--files` does, and -Classify reads each file's texture into its Texture without filtering.

Alias: `Get-TxFile`

```powershell
Get-TrexFile [[-Path] <string[]>] [-Hidden] [-NoIgnore] [-Include <string[]>] [-FileType <string[]>] [-ExcludeFileType <string[]>] [-Texture <RegionKind[]>] [-ExcludeTexture <RegionKind[]>] [-Sort <SortKey>] [-Descending] [-Classify] [<CommonParameters>]
Get-TrexFile -LiteralPath <string[]> [-Hidden] [-NoIgnore] [-Include <string[]>] [-FileType <string[]>] [-ExcludeFileType <string[]>] [-Texture <RegionKind[]>] [-ExcludeTexture <RegionKind[]>] [-Sort <SortKey>] [-Descending] [-Classify] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
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
