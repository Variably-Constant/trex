---
title: Rules
linkTitle: Rules
weight: 50
---

`Invoke-TrexRule` is the module's `trex scan --rules`, and `ConvertTo-TrexSarif` its `--sarif`.
The examples are on [pattern files](../../pattern-files/#rules).

### Get-TrexRule

Lists the rules declared for the session, in a library, or in the rule files named, in declaration order.

Alias: `Get-TxRule`

```powershell
Get-TrexRule [[-Name] <string>] [-RuleFile <string[]>] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | Lists a library's rules in place of the session's. |
| `-Name` | string |  | Only rules whose name matches; wildcards apply. |
| `-RuleFile` | string[] |  | Pattern files, or directories of `.trex` files, imported for this call over the atoms in force; a file already imported is read in place of that import. |

Writes [Trex.Rule](../types/#trexrule).

### Invoke-TrexRule

Scans the rules declared in pattern files over text, files or directories, and writes each finding as a Trex.Finding.

A rule fires on each match of its pattern, or on each record holding it where the rule names a record or a pattern the record must not hold; its message is rendered from the match, and its fix, where it has one, renders what replaces the match. -Fix writes the fixes into the files, asking first under -Confirm and saying what it would do under -WhatIf, and with -Interactive puts each fix to the person first, as Edit-TrexText -Interactive puts a change; -Diff writes the unified diff the fixes would make instead.

-Json writes every finding as the trex command's `scan --rules --json` writes them, one array after the last input; -Sarif one SARIF 2.1.0 document for every finding, -GitHub one GitHub workflow annotation each, and -Format a report template rendered at each, which reads `${rule}`, `${severity}`, `${message}` and `${fix}` beside the registers. -Count writes how many findings each file holds, as the trex command's `--count` does, -FilesWithMatches and -FilesWithoutMatch the path of each file holding a finding or none, and -RequireMatch an error when no input held a finding.

-Head, -Tail and -Lines scan only the first lines of each input, its last, or a range of them, counted in records of -Unit where it names one: a finding counts only if it is wholly inside, and reports the input's own line and offset; -Fix and -Diff change only that part. -First and -Last are -Head and -Tail. -Follow scans what each file gains as it grows, after its tail, its open range or the whole of it, writing each finding once nothing that arrives later can change it, -Json as one object a string, until the pipeline is stopped or -MaxCount has written all it will from each file.

Alias: `Invoke-TxRule`

```powershell
Invoke-TrexRule [-InputObject] <string> [-RuleFile <string[]>] [-Library <Library>] [-MaxCount <uint>] [-Json] [-Sarif] [-GitHub] [-Format <string>] [-Count] [-RequireMatch] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-WhatIf] [-Confirm] [<CommonParameters>]
Invoke-TrexRule -Path <string[]> [-RuleFile <string[]>] [-Library <Library>] [-MaxCount <uint>] [-Fix] [-Diff] [-Context <uint>] [-Interactive] [-ShowSkipped] [-Json] [-Sarif] [-GitHub] [-Format <string>] [-Count] [-FilesWithMatches] [-FilesWithoutMatch] [-RequireMatch] [-Hidden] [-NoIgnore] [-Binary] [-Include <string[]>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-KeepCount] [-WhatIf] [-Confirm] [<CommonParameters>]
Invoke-TrexRule -LiteralPath <string[]> [-RuleFile <string[]>] [-Library <Library>] [-MaxCount <uint>] [-Fix] [-Diff] [-Context <uint>] [-Interactive] [-ShowSkipped] [-Json] [-Sarif] [-GitHub] [-Format <string>] [-Count] [-FilesWithMatches] [-FilesWithoutMatch] [-RequireMatch] [-Hidden] [-NoIgnore] [-Binary] [-Include <string[]>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-KeepCount] [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Binary` | switch |  | Reads files that hold a NUL byte, which a walk treats as binary. |
| `-Context` | uint |  | Lines of context around each change in a diff; 3 when absent. |
| `-Count` | switch |  | Writes how many findings each file holds, as a Trex.MatchCount for every file read, none included; over text, one count of every string, after the last. |
| `-Diff` | switch |  | Writes the unified diff the fixes would make, and nothing to the files. |
| `-FilesWithMatches` | switch |  | Writes the path of each file holding a finding. |
| `-FilesWithoutMatch` | switch |  | Writes the path of each file holding no finding. |
| `-Fix` | switch |  | Writes the fixes into the files. |
| `-Follow` | switch |  | After each file's -Tail, its open -Lines range or the whole of it, scans what it gains as it grows, writing each finding once nothing that arrives later can change it, until the pipeline is stopped. A file truncated, replaced or removed is a new input, its -MaxCount count started again. |
| `-Format` | string |  | A report template to write at each finding in place of the finding. |
| `-GitHub` | switch |  | Writes each finding as a GitHub workflow annotation. |
| `-Head` | uint |  | Scans the first this many lines of each input, or records of -Unit, reading a file no further. |
| `-Hidden` | switch |  | Reads hidden files and directories a walk would skip. |
| `-Include` | string[] |  | Keeps a walked file only when a glob matches it, or drops it for a glob that starts with `!`. |
| `-InputObject` | string | by value | The text to scan; each string piped in is scanned on its own. |
| `-Interactive` | switch |  | With -Fix, puts each fix to the person before it is written, as Edit-TrexText -Interactive puts a change. |
| `-Json` | switch |  | Writes every finding as the trex command's `scan --rules --json` writes them: one array after the last input, each object naming the rule, its severity and message, the finding's position, its text, registers and fix, the rule's metadata, and the path of a file. |
| `-KeepCount` | switch |  | With -Follow and -MaxCount, keeps a file's count when it is truncated, replaced or removed, in place of starting it again. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms, rules among them, to read in place of the session's. |
| `-Lines` | object |  | Scans a range of lines, or records of -Unit, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. |
| `-LiteralPath` | string[] | by name | Files or directories to scan, read as written, as Get-ChildItem pipes them. |
| `-MaxCount` | uint |  | Writes at most this many findings of each input. |
| `-NoIgnore` | switch |  | Reads files an ignore rule excludes. |
| `-Path` | string[] |  | Files or directories to scan; wildcards expand. |
| `-RequireMatch` | switch |  | Writes an error after the last input when no input held a finding, for a script that stops on one. |
| `-RuleFile` | string[] |  | Pattern files, or directories of `.trex` files, imported for this call over the atoms in force; a file already imported is read in place of that import. |
| `-Sarif` | switch |  | Writes every finding as one SARIF 2.1.0 document. |
| `-ShowSkipped` | switch |  | Lists the position of each fix a template answer skipped, beside the count -Interactive always reports. |
| `-Tail` | uint |  | Scans the last this many lines of each input, or records of -Unit, reading a file backward from its end. |
| `-Unit` | string |  | What -Head, -Tail and -Lines count: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph` or `block`. |

Writes [Trex.Finding](../types/#trexfinding), `string`, [Trex.MatchCount](../types/#trexmatchcount).

### ConvertTo-TrexSarif

Renders findings as one SARIF 2.1.0 document, for a CI system or an editor that reads one.

The rules the document lists are the findings' own, in the order they first appear. A finding with no file carries no location URI and no fix, since a fix in SARIF names the file it changes.

Alias: `ConvertTo-TxSarif`

```powershell
ConvertTo-TrexSarif [-Finding] <Finding[]> [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Finding` | [Trex.Finding](../types/#trexfinding)[] | by value | The findings to render. |

Writes `string`.
