---
title: Rules
linkTitle: Rules
weight: 50
---

# Rules

A rule is a named pattern with what a finding of it says, declared in a pattern file as the
[pattern syntax](../../pattern-syntax/#declared-names-and-the-library) page describes.
`Invoke-TrexRule` is the module's `trex scan --rules`: it scans the rules over text, files or
directories and writes each finding as a [`Trex.Finding`](../types/#trexfinding). The examples
read these files:

```powershell
PS> Get-Content ./rules.trex
rule cardnum error "card number ending ${card:last4}" = \{card}:card
fix cardnum = ****
rule private_ip
  pattern = \{ip_private}:addr
  message = private address ${addr}
  severity = note
  fix = ${addr:octet1-2}.x.x
  meta.cwe = CWE-200
rule legacy warning "a legacy call" = "legacy_call"
files legacy = *.py
PS> Get-Content ./logs/app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:04Z ERROR payment failed: card 4111 1111 1111 1111 declined
PS> Get-Content ./src/job.py
result = legacy_call(42)
PS> Get-Content ./src/notes.md
legacy_call is the old name.
```

## Rules

`Get-TrexRule` lists the rules declared for the session, in a library, or in the files
`-RuleFile` names:

```powershell
PS> Get-TrexRule -RuleFile ./rules.trex | Select-Object Name, Severity, Message, Fix

Name       Severity Message                          Fix
----       -------- -------                          ---
cardnum       Error card number ending ${card:last4} ****
private_ip     Note private address ${addr}          ${addr:octet1-2}.x.x
legacy      Warning a legacy call

PS> Get-TrexRule private_ip -RuleFile ./rules.trex | Select-Object Name, Pattern, Meta, @{ n = 'File'; e = { Split-Path -Leaf $_.File } }, Line

Name    : private_ip
Pattern : \{ip_private}:addr
Meta    : {[cwe, CWE-200]}
File    : rules.trex
Line    : 3
```

## Findings

A finding carries its rule, its severity, its message rendered at the match, where it stands,
and the fix rendered in its place. `files legacy = *.py` keeps the `legacy` rule to Python
files, so `notes.md` holds no finding:

```powershell
PS> Invoke-TrexRule -Path ./logs, ./src -RuleFile ./rules.trex | Select-Object Rule, Severity, Message, Text, Fix

Rule     : private_ip
Severity : Note
Message  : private address 10.0.0.5
Text     : 10.0.0.5
Fix      : 10.0.x.x

Rule     : cardnum
Severity : Error
Message  : card number ending 1111
Text     : 4111 1111 1111 1111
Fix      : ****

Rule     : legacy
Severity : Warning
Message  : a legacy call
Text     : legacy_call
Fix      :

PS> Invoke-TrexRule -Path ./logs -RuleFile ./rules.trex | Select-Object Rule, Region, Text

Rule       Region Text
----       ------ ----
private_ip 1:46   10.0.0.5
cardnum    2:49   4111 1111 1111 1111

PS> Invoke-TrexRule -Path ./logs -RuleFile ./rules.trex -Format '${line}:${col} ${rule} ${severity}'
1:46 private_ip note
2:49 cardnum error
```

`-Head`, `-Tail` and `-Lines` scan only part of each input, each finding standing at the file's
own line, and `-Fix` and `-Diff` then change only that part. `-Follow` writes each finding a file
gains as it grows, for the rules that fire on matches, until the pipeline is stopped:

```powershell
PS> Invoke-TrexRule -Path ./logs/app.log -RuleFile ./rules.trex -Tail 1 | Select-Object Rule, Region, Text

Rule    Region Text
----    ------ ----
cardnum 2:49   4111 1111 1111 1111
```

`-Count` writes how many findings each file holds, every file read included, as the command
line's `--count` does; `-FilesWithMatches` and `-FilesWithoutMatch` write the path of each file
holding a finding or none, and `-RequireMatch` writes an error when no input held one:

```powershell
PS> Invoke-TrexRule -Path ./logs, ./src -RuleFile ./rules.trex -Count | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, Count

File     Count
----     -----
app.log      2
job.py       1
notes.md     0

PS> Invoke-TrexRule -Path ./logs, ./src -RuleFile ./rules.trex -FilesWithoutMatch | Split-Path -Leaf
notes.md
```

Rules declared for the session are read with no `-RuleFile`:

```powershell
PS> Import-TrexAtom ./rules.trex
PS> 'db at 192.168.1.20' | Invoke-TrexRule | Select-Object Rule, Message, Fix

Rule       Message                      Fix
----       -------                      ---
private_ip private address 192.168.1.20 192.168.x.x
```

## Reports for other tools

`-Json` writes every finding as `trex scan --rules --json` does, one array after the last input.
`-GitHub` writes each finding as a GitHub workflow annotation, and `-Sarif` writes every finding
as one SARIF 2.1.0 document, as `ConvertTo-TrexSarif` renders findings already written:

```powershell
PS> Invoke-TrexRule -Path ./logs -RuleFile ./rules.trex -Json
[{"rule":"private_ip","severity":"note","message":"private address 10.0.0.5","path":"C:\\Temp\\demo\\logs\\app.log","line":1,"col":46,"end_line":1,"end_col":54,"start":45,"end":53,"text":"10.0.0.5","captures":{"addr":"10.0.0.5"},"fix":"10.0.x.x","meta":{"cwe":"CWE-200"}},{"rule":"cardnum","severity":"error","message":"card number ending 1111","path":"C:\\Temp\\demo\\logs\\app.log","line":2,"col":49,"end_line":2,"end_col":68,"start":113,"end":132,"text":"4111 1111 1111 1111","captures":{"card":"4111 1111 1111 1111"},"fix":"****"}]

PS> Invoke-TrexRule -Path ./logs -RuleFile ./rules.trex -GitHub
::notice file=C%3A/Temp/demo/logs/app.log,line=1,col=46,endLine=1,endColumn=54,title=private_ip::private address 10.0.0.5
::error file=C%3A/Temp/demo/logs/app.log,line=2,col=49,endLine=2,endColumn=68,title=cardnum::card number ending 1111

PS> $sarif = Invoke-TrexRule -Path ./logs -RuleFile ./rules.trex -Sarif | ConvertFrom-Json
PS> $sarif.version
2.1.0

PS> $sarif.runs[0].results | Select-Object ruleId, level, @{ n = 'message'; e = { $_.message.text } }

ruleId     level message
------     ----- -------
private_ip note  private address 10.0.0.5
cardnum    error card number ending 1111

PS> (Invoke-TrexRule -Path ./logs -RuleFile ./rules.trex | ConvertTo-TrexSarif | ConvertFrom-Json).runs[0].results.Count
2
```

## Fixes

`-Diff` writes the unified diff the fixes would make and writes nothing to the files; `-Fix`
writes them, asking `ShouldProcess` for each file, and `-Interactive` puts each fix to the person
first, as `Edit-TrexText -Interactive` puts a change:

```powershell
PS> Invoke-TrexRule -Path ./logs -RuleFile ./rules.trex -Diff
--- C:\Temp\demo\logs\app.log
+++ C:\Temp\demo\logs\app.log
@@ -1,2 +1,2 @@
-2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
-2026-09-27T09:00:04Z ERROR payment failed: card 4111 1111 1111 1111 declined
+2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.x.x took 120ms
+2026-09-27T09:00:04Z ERROR payment failed: card **** declined

PS> Invoke-TrexRule -Path ./logs -RuleFile ./rules.trex -Fix | Out-Null
PS> Get-Content -Path ./logs/app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.x.x took 120ms
2026-09-27T09:00:04Z ERROR payment failed: card **** declined
```

## Cmdlets

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

-Head, -Tail and -Lines scan only the first lines of each input, its last, or a range of them, counted in records of -Unit where it names one: a finding counts only where it lies wholly inside, and stands at the input's own line and offset; -Fix and -Diff change only that part. -First and -Last are -Head and -Tail. -Follow scans what each file gains as it grows, after its tail, its open range or the whole of it, writing each finding once nothing that arrives later can change it, -Json as one object a string, until the pipeline is stopped or -MaxCount has written all it will from each file.

Alias: `Invoke-TxRule`

```powershell
Invoke-TrexRule [-InputObject] <string> [-RuleFile <string[]>] [-Library <Library>] [-MaxCount <uint>] [-Json] [-Sarif] [-GitHub] [-Format <string>] [-Count] [-RequireMatch] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-WhatIf] [-Confirm] [<CommonParameters>]
Invoke-TrexRule -Path <string[]> [-RuleFile <string[]>] [-Library <Library>] [-MaxCount <uint>] [-Fix] [-Diff] [-Context <uint>] [-Interactive] [-ShowSkipped] [-Json] [-Sarif] [-GitHub] [-Format <string>] [-Count] [-FilesWithMatches] [-FilesWithoutMatch] [-RequireMatch] [-Hidden] [-NoIgnore] [-Binary] [-Include <string[]>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-WhatIf] [-Confirm] [<CommonParameters>]
Invoke-TrexRule -LiteralPath <string[]> [-RuleFile <string[]>] [-Library <Library>] [-MaxCount <uint>] [-Fix] [-Diff] [-Context <uint>] [-Interactive] [-ShowSkipped] [-Json] [-Sarif] [-GitHub] [-Format <string>] [-Count] [-FilesWithMatches] [-FilesWithoutMatch] [-RequireMatch] [-Hidden] [-NoIgnore] [-Binary] [-Include <string[]>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-WhatIf] [-Confirm] [<CommonParameters>]
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
| `-Follow` | switch |  | After each file's -Tail, its open -Lines range or the whole of it, scans what it gains as it grows, writing each finding once nothing that arrives later can change it, until the pipeline is stopped. |
| `-Format` | string |  | A report template to write at each finding in place of the finding. |
| `-GitHub` | switch |  | Writes each finding as a GitHub workflow annotation. |
| `-Head` | uint |  | Scans the first this many lines of each input, or records of -Unit, reading a file no further. |
| `-Hidden` | switch |  | Reads hidden files and directories a walk would skip. |
| `-Include` | string[] |  | Keeps a walked file only when a glob matches it, or drops it for a glob that starts with `!`. |
| `-InputObject` | string | by value | The text to scan; each string piped in is scanned on its own. |
| `-Interactive` | switch |  | With -Fix, puts each fix to the person before it is written, as Edit-TrexText -Interactive puts a change. |
| `-Json` | switch |  | Writes every finding as the trex command's `scan --rules --json` writes them: one array after the last input, each object naming the rule, its severity and message, where the finding stands, its text, registers and fix, the rule's metadata, and the path of a file. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms, rules among them, to read in place of the session's. |
| `-Lines` | object |  | Scans a range of lines, or records of -Unit, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. |
| `-LiteralPath` | string[] | by name | Files or directories to scan, read as written, as Get-ChildItem pipes them. |
| `-MaxCount` | uint |  | Writes at most this many findings of each input. |
| `-NoIgnore` | switch |  | Reads files an ignore rule excludes. |
| `-Path` | string[] |  | Files or directories to scan; wildcards expand. |
| `-RequireMatch` | switch |  | Writes an error after the last input when no input held a finding, for a script that stops on one. |
| `-RuleFile` | string[] |  | Pattern files, or directories of `.trex` files, imported for this call over the atoms in force; a file already imported is read in place of that import. |
| `-Sarif` | switch |  | Writes every finding as one SARIF 2.1.0 document. |
| `-ShowSkipped` | switch |  | Lists where each fix a template answer skipped stands, beside the count -Interactive always says. |
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
