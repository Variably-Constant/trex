---
title: Grouping
linkTitle: Grouping
weight: 30
---

# Grouping

`Group-TrexMatch` is the module's `trex count-by`, `top` and `uniq`: it groups the matches of a
pattern by a key rendered at each, and writes one `Trex.Group` a key, most matches first, with
the aggregates asked for over a register's typed values. Matches from every string piped in and
every file named are grouped together, and the groups are written once, after the last input.
The examples read these files:

```powershell
PS> Get-Content ./logs/access.log
10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
PS> Get-Content ./logs/app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
PS> Get-Content ./secrets.trex
let mail = \E
let addr = \I
```

## Keys

`-Key` is a report template, so a key can be a typed slice of a register:

```powershell
PS> Group-TrexMatch '\I:ip' -Key '${ip:octet1-3}' -Path ./logs/access.log

Key    Count
---    -----
10.0.0     3
10.0.1     1

PS> Group-TrexMatch '\I:ip \W:verb' -Key '${verb}' -Path ./logs/access.log -SortBy Key

Key  Count
---  -----
GET      3
POST     1

PS> 'GET /a', 'POST /b', 'GET /c' | Group-TrexMatch '\W:verb' -Key '${verb}' -MaxCount 1

Key Count
--- -----
GET     2

PS> Group-TrexMatch '\I:ip' -Key '${ip}' -Path ./logs -Unique
10.0.0.5
10.0.0.7
10.0.1.9
```

`-Head`, `-Tail` and `-Lines` group the matches of only part of each input, a key reading each
match at the file's own line:

```powershell
PS> Group-TrexMatch '\I:ip' -Key '${ip}' -Path ./logs/access.log -Tail 2

Key      Count
---      -----
10.0.0.5     1
10.0.1.9     1
```

## Aggregates

`-Sum`, `-Average`, `-Minimum` and `-Maximum` each read one register whose kind adds or orders,
and `-Percentile` reads the register `-PercentileOf` names. Each aggregate is a property of the
group, named for it, in the .NET type that holds its kind: numbers as `long` or `decimal`,
durations as `TimeSpan`, timestamps as `DateTimeOffset`:

```powershell
PS> Group-TrexMatch '\I:ip . . \N \N:bytes' -Key '${ip}' -Sum bytes -Maximum bytes -Path ./logs/access.log

Key      Count  Sum Maximum
---      -----  --- -------
10.0.0.5     2 5208    5120
10.0.0.7     1    0       0
10.0.1.9     1  312     312

PS> Group-TrexMatch '\R:t' -Key all -Average t -Minimum t -Maximum t -Path ./logs/app.log

Key     : all
Count   : 3
Average : 00:00:00.5550000
Minimum : 00:00:00.0950000
Maximum : 00:00:01.4500000

PS> Group-TrexMatch '\R:t' -Key all -Percentile 50, 95 -PercentileOf t -Path ./logs/app.log

Key Count P50              P95
--- ----- ---              ---
all     3 00:00:00.1200000 00:00:01.4500000

PS> '1 2 4' | Group-TrexMatch '\N:n' -Key all -Average n | ForEach-Object { $_.Average.GetType().Name + ' ' + $_.Average }
Decimal 2.3333333333333333333333333333
```

An average is computed exactly and written as the `decimal` that holds it. A percentile is the
observed value at rank ceil(p * n) unless `-PercentileMethod` says otherwise: `Linear`
interpolates between the two neighbors and takes numeric kinds only, `Lower` takes the value at
or below the rank, and `Hybrid` interpolates where the kind allows it and takes the nearest value
otherwise:

```powershell
PS> '10 20 30 40' | Group-TrexMatch '\N:n' -Key all -Percentile 50 -PercentileOf n

Key Count P50
--- ----- ---
all     4  20

PS> '10 20 30 40' | Group-TrexMatch '\N:n' -Key all -Percentile 50 -PercentileOf n -PercentileMethod Linear

Key Count P50
--- ----- ---
all     4  25
```

An aggregate a kind does not have, such as the sum of an address, stops the cmdlet with an error
naming the register and its kind.

## Pattern sets

Several patterns, or the members of a pattern file, group as one set, and `${pattern}` in a key
reads the member that made each match:

```powershell
PS> Group-TrexMatch -PatternFile ./secrets.trex -Key '${pattern}' -Path ./logs

Key  Count
---  -----
addr     7
mail     1

PS> Group-TrexMatch '\E', '\I' -Key '${pattern}' -Path ./logs/app.log

Key Count
--- -----
\I      3
\E      1
```

## Cmdlets

### Group-TrexMatch

Groups the matches of a trex pattern by a rendered key, with the count of each and aggregates over a register's typed values.

-Key is a report template: `${ip:octet1-2}`, `${u:host}`, `${e:domain}`, `${path}` for the file a match is in. Matches from every string piped in and every file named are grouped together, and the groups are written once, after the last input. -Sum, -Average, -Minimum and -Maximum each read one register whose kind orders or adds, and -Percentile reads the register -PercentileOf names. -Unique writes the keys alone.

Several patterns, or the members of a -PatternFile, group as one set, and a key reads the member that made each match as `${pattern}`.

-Head, -Tail and -Lines group only the first lines of each input, its last, or a range of them, counted in records of -Unit where it names one, a key reading each match at its input's own line; -First and -Last are -Head and -Tail. -MaxCount writes only the first groups.

Alias: `Group-TxMatch`

```powershell
Group-TrexMatch [[-Pattern] <Object[]>] [-Key] <string> -InputObject <string> [-PatternFile <string>] [-SortBy <GroupOrder>] [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Unique] [-Sum <string>] [-Average <string>] [-Minimum <string>] [-Maximum <string>] [-Percentile <uint[]>] [-PercentileOf <string>] [-PercentileMethod <PercentileMethod>] [-Library <Library>] [<CommonParameters>]
Group-TrexMatch [[-Pattern] <Object[]>] [-Key] <string> -Path <string[]> [-PatternFile <string>] [-Hidden] [-NoIgnore] [-SortBy <GroupOrder>] [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Unique] [-Sum <string>] [-Average <string>] [-Minimum <string>] [-Maximum <string>] [-Percentile <uint[]>] [-PercentileOf <string>] [-PercentileMethod <PercentileMethod>] [-Library <Library>] [<CommonParameters>]
Group-TrexMatch [[-Pattern] <Object[]>] [-Key] <string> -LiteralPath <string[]> [-PatternFile <string>] [-Hidden] [-NoIgnore] [-SortBy <GroupOrder>] [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Unique] [-Sum <string>] [-Average <string>] [-Minimum <string>] [-Maximum <string>] [-Percentile <uint[]>] [-PercentileOf <string>] [-PercentileMethod <PercentileMethod>] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Average` | string |  | The register whose values each group averages. |
| `-Head` | uint |  | Groups the matches of the first this many lines of each input, or records of -Unit, reading a file no further. |
| `-Hidden` | switch |  | Reads hidden files and directories a walk would skip. |
| `-InputObject` | string | by value | The text to scan; every string piped in is grouped with the rest. |
| `-Key` | string |  | The key each match is grouped by, a report template. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms a source-text pattern is compiled against, in place of the session's. |
| `-Lines` | object |  | Groups the matches of a range of lines, or records of -Unit, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. A match counts only where it lies wholly inside. |
| `-LiteralPath` | string[] | by name | Files or directories to scan, read as written, as Get-ChildItem pipes them. |
| `-MaxCount` | uint |  | Writes only the first this many groups. |
| `-Maximum` | string |  | The register whose greatest value each group reports. |
| `-Minimum` | string |  | The register whose least value each group reports. |
| `-NoIgnore` | switch |  | Reads files an ignore rule excludes. |
| `-Path` | string[] |  | Files or directories to scan; wildcards expand. |
| `-Pattern` | object[] |  | The patterns: trex source text or Trex.Pattern objects; several group as one set. |
| `-PatternFile` | string |  | A pattern file whose members group as one set: each `let name = pattern` line under its name and each bare pattern line under its line number, with the file's declarations in force. -Key is named beside it, since a first argument by position is read as -Pattern. |
| `-Percentile` | uint[] |  | Percentiles to report, such as 50 and 95, of the register -PercentileOf names. |
| `-PercentileMethod` | [Trex.PercentileMethod](../types/#trexpercentilemethod) |  | How a percentile between two observed values is decided: Nearest when absent. |
| `-PercentileOf` | string |  | The register the percentiles read. |
| `-SortBy` | [Trex.GroupOrder](../types/#trexgrouporder) |  | The order the groups are written in: Count, most first, when absent. |
| `-Sum` | string |  | The register whose values each group sums. |
| `-Tail` | uint |  | Groups the matches of the last this many lines of each input, or records of -Unit, reading a file backward from its end. |
| `-Unique` | switch |  | Writes the distinct keys alone. |
| `-Unit` | string |  | What -Head, -Tail and -Lines count: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph` or `block`. |

Writes [Trex.Group](../types/#trexgroup), `string`.
