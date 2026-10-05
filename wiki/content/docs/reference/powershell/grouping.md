---
title: Grouping
linkTitle: Grouping
weight: 30
---

`Group-TrexMatch` is the module's `trex count-by`, `top` and `uniq`. The examples are on
[aggregates](../../aggregates/).

### Group-TrexMatch

Groups the matches of a TREX pattern by a rendered key, with the count of each and aggregates over a register's typed values.

-Key is a report template: `${ip:octet1-2}`, `${u:host}`, `${e:domain}`, `${path}` for the file a match is in. Matches from every string piped in and every file named are grouped together, and the groups are written once, after the last input. -Sum, -Average, -Minimum and -Maximum each read the registers they name, whose kind orders or adds, and -Percentile reads each register -PercentileOf names. An aggregate over one register is written under its own name, `Sum` or `P95`, and over several under its name and each register's, `Sum_t` and `Sum_size`. -Unique writes the keys alone. A file named outright that holds a NUL byte is left unread and named in a warning, unless -Binary asks for it; a walk passes over one.

Several patterns, or the members of a -PatternFile, group as one set, and a key reads the member that made each match as `${pattern}`.

-Head, -Tail and -Lines group only the first lines of each input, its last, or a range of them, counted in records of -Unit where it names one, a key reading each match at its input's own line; -First and -Last are -Head and -Tail. -MaxCount writes only the first groups.

Alias: `Group-TxMatch`

```powershell
Group-TrexMatch [[-Pattern] <Object[]>] [-Key] <string> -InputObject <string> [-PatternFile <string>] [-SortBy <GroupOrder>] [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Unique] [-Sum <string[]>] [-Average <string[]>] [-Minimum <string[]>] [-Maximum <string[]>] [-Percentile <uint[]>] [-PercentileOf <string[]>] [-PercentileMethod <PercentileMethod>] [-Library <Library>] [<CommonParameters>]
Group-TrexMatch [[-Pattern] <Object[]>] [-Key] <string> -Path <string[]> [-PatternFile <string>] [-Hidden] [-NoIgnore] [-Binary] [-SortBy <GroupOrder>] [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Unique] [-Sum <string[]>] [-Average <string[]>] [-Minimum <string[]>] [-Maximum <string[]>] [-Percentile <uint[]>] [-PercentileOf <string[]>] [-PercentileMethod <PercentileMethod>] [-Library <Library>] [<CommonParameters>]
Group-TrexMatch [[-Pattern] <Object[]>] [-Key] <string> -LiteralPath <string[]> [-PatternFile <string>] [-Hidden] [-NoIgnore] [-Binary] [-SortBy <GroupOrder>] [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Unique] [-Sum <string[]>] [-Average <string[]>] [-Minimum <string[]>] [-Maximum <string[]>] [-Percentile <uint[]>] [-PercentileOf <string[]>] [-PercentileMethod <PercentileMethod>] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Average` | string[] |  | The registers whose values each group averages: one is written as `Average`, several as `Average_<register>` each. |
| `-Binary` | switch |  | Reads files that hold a NUL byte, which a walk treats as binary. |
| `-Head` | uint |  | Groups the matches of the first this many lines of each input, or records of -Unit, reading a file no further. |
| `-Hidden` | switch |  | Reads hidden files and directories a walk would skip. |
| `-InputObject` | string | by value | The text to scan; every string piped in is grouped with the rest. |
| `-Key` | string |  | The key each match is grouped by, a report template. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms a source-text pattern is compiled against, in place of the session's. |
| `-Lines` | object |  | Groups the matches of a range of lines, or records of -Unit, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. A match counts only if it is wholly inside. |
| `-LiteralPath` | string[] | by name | Files or directories to scan, read as written, as Get-ChildItem pipes them. |
| `-MaxCount` | uint |  | Writes only the first this many groups. |
| `-Maximum` | string[] |  | The registers whose greatest value each group reports: one is written as `Maximum`, several as `Maximum_<register>` each. |
| `-Minimum` | string[] |  | The registers whose least value each group reports: one is written as `Minimum`, several as `Minimum_<register>` each. |
| `-NoIgnore` | switch |  | Reads files an ignore rule excludes. |
| `-Path` | string[] |  | Files or directories to scan; wildcards expand. |
| `-Pattern` | object[] |  | The patterns: TREX source text or Trex.Pattern objects; several group as one set. |
| `-PatternFile` | string |  | A pattern file whose members group as one set: each `let name = pattern` line under its name and each bare pattern line under its line number, with the file's declarations in force. -Key is named beside it, since a first argument by position is read as -Pattern. |
| `-Percentile` | uint[] |  | Percentiles to report, such as 50 and 95, of each register -PercentileOf names. |
| `-PercentileMethod` | [Trex.PercentileMethod](../types/#trexpercentilemethod) |  | How a percentile between two observed values is decided: Nearest when absent. |
| `-PercentileOf` | string[] |  | The registers the percentiles read: one writes `P50`, `P95` and so on, several write `P50_<register>` for each. |
| `-SortBy` | [Trex.GroupOrder](../types/#trexgrouporder) |  | The order the groups are written in: Count, most first, when absent. |
| `-Sum` | string[] |  | The registers whose values each group sums: one is written as `Sum`, several as `Sum_<register>` each. |
| `-Tail` | uint |  | Groups the matches of the last this many lines of each input, or records of -Unit, reading a file backward from its end. |
| `-Unique` | switch |  | Writes the distinct keys alone. |
| `-Unit` | string |  | What -Head, -Tail and -Lines count: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph` or `block`. |

Writes [Trex.Group](../types/#trexgroup), `string`.
