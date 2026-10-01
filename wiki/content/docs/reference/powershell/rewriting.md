---
title: Rewriting
linkTitle: Rewriting
weight: 20
---

# Rewriting

`Edit-TrexText` is the module's `trex rewrite` and `Protect-TrexText` its `trex redact`. The
examples are on [rewriting](../../rewriting/) and [redaction](../../redaction/).

### Edit-TrexText

Replaces each match of a trex pattern with a rendered template or with what a script block returns for it.

With -Template, `${name}` renders a register, `${0}` the whole match, `${name:upper}` a transform and `${ip:octet1-2}` a typed slice. With -ScriptBlock, the block runs once per match with the Trex.Match as `$_`, and its last output replaces the match. Text piped in comes back rewritten; -Path writes each file's rewritten text, -InPlace writes the files that have a match back where they are, and -Diff writes the unified diff each would take. -MaxCount replaces only the first matches of each input. A file named outright that holds a NUL byte is left as it is and named in a warning, unless -Binary asks for it; a walk passes over one.

-Head, -Tail and -Lines rewrite the first lines of each input, its last, or a range of them, counted in records of -Unit where it names one: the text written is that part alone, rewritten, and a file written back or described by its diff changes only there. -First and -Last are -Head and -Tail. -Follow writes a file's rewritten text as it grows, after its tail, its open range or the whole of it, a line at a time, until the pipeline is stopped.

-Interactive puts each change to the person first, as its diff and the template of the tokens it replaces: they apply it, skip it, type another replacement, answer for every later change of the same template, apply everything after it, or quit, which leaves every file as it was. The changes accepted are written once the last has been answered, and how many a template answer skipped is said as a warning, with where under -ShowSkipped.

Alias: `Edit-TxText`

```powershell
Edit-TrexText [-Pattern] <Object> [-Template] <string> -InputObject <string> [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
Edit-TrexText [-Pattern] <Object> [-Template] <string> -Path <string[]> [-InPlace] [-Diff] [-Interactive] [-ShowSkipped] [-Context <uint>] [-Hidden] [-NoIgnore] [-Binary] [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
Edit-TrexText [-Pattern] <Object> [-Template] <string> -LiteralPath <string[]> [-InPlace] [-Diff] [-Interactive] [-ShowSkipped] [-Context <uint>] [-Hidden] [-NoIgnore] [-Binary] [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
Edit-TrexText [-Pattern] <Object> -ScriptBlock <scriptblock> -InputObject <string> [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
Edit-TrexText [-Pattern] <Object> -ScriptBlock <scriptblock> -Path <string[]> [-InPlace] [-Diff] [-Interactive] [-ShowSkipped] [-Context <uint>] [-Hidden] [-NoIgnore] [-Binary] [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
Edit-TrexText [-Pattern] <Object> -ScriptBlock <scriptblock> -LiteralPath <string[]> [-InPlace] [-Diff] [-Interactive] [-ShowSkipped] [-Context <uint>] [-Hidden] [-NoIgnore] [-Binary] [-MaxCount <uint>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Binary` | switch |  | Reads files that hold a NUL byte, which a walk treats as binary. |
| `-Context` | uint |  | Lines of context around each change in a diff; 3 when absent. |
| `-Diff` | switch |  | Writes the unified diff each file would take instead of its text. |
| `-Follow` | switch |  | Writes a file's rewritten text as it grows, a line at a time, after its tail, its open range or the whole of it, until the pipeline is stopped. |
| `-Head` | uint |  | Rewrites the first this many lines of each input, or records of -Unit, reading a file no further. |
| `-Hidden` | switch |  | Reads hidden files and directories a walk would skip. |
| `-InPlace` | switch |  | Writes each file that has a match back where it is. |
| `-InputObject` | string | by value | The text to rewrite; each string piped in is rewritten on its own. |
| `-Interactive` | switch |  | Puts each change to the person before it is written. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms a source-text pattern is compiled against, in place of the session's. |
| `-Lines` | object |  | Rewrites a range of lines, or records of -Unit, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. |
| `-LiteralPath` | string[] | by name | Files or directories to rewrite, read as written, as Get-ChildItem pipes them. |
| `-MaxCount` | uint |  | Replaces only the first this many matches of each input. |
| `-NoIgnore` | switch |  | Reads files an ignore rule excludes. |
| `-Path` | string[] |  | Files or directories to rewrite; wildcards expand. |
| `-Pattern` | object |  | The pattern: trex source text, or a Trex.Pattern. |
| `-ScriptBlock` | scriptblock |  | A script block run once per match, with the match as `$_`; its last output replaces the match. |
| `-ShowSkipped` | switch |  | Lists where each change a template answer skipped stands, beside the count -Interactive always says. |
| `-Tail` | uint |  | Rewrites the last this many lines of each input, or records of -Unit, reading a file backward from its end. |
| `-Template` | string |  | The template rendered in place of each match. |
| `-Unit` | string |  | What -Head, -Tail and -Lines count: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph` or `block`. |

Writes `string`.

### Protect-TrexText

Masks each match of a trex pattern, keeping the fields named.

Every character of a match becomes the mask, `*` when -Mask is not given, except the fields -Keep names, which stay where they stand: `card:last4` keeps a card's last four digits, `ip:octet1-2` an address's first two octets, `e:domain` a mail domain. -Mask takes one character, a token that stands in for each masked run, `shape`, which masks letters and digits and keeps every other character so the text still lexes as it did, or `pseudonym`, which gives each distinct value a stable name per kind. Text piped in comes back masked; -Path writes each file's masked text, -InPlace writes the files back, and -Diff writes the unified diff. A file named outright that holds a NUL byte is left as it is and named in a warning, unless -Binary asks for it; a walk passes over one.

-Head, -Tail and -Lines mask the first lines of each input, its last, or a range of them, counted in records of -Unit where it names one: the text written is that part alone, masked, so nothing outside it is written unmasked, and a file written back or described by its diff changes only there. -First and -Last are -Head and -Tail. -Follow writes a file's masked text as it grows, after its tail, its open range or the whole of it, a line at a time, until the pipeline is stopped.

Alias: `Protect-TxText`

```powershell
Protect-TrexText [-Pattern] <Object> -InputObject <string> [-Keep <string[]>] [-Mask <string>] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
Protect-TrexText [-Pattern] <Object> -Path <string[]> [-Keep <string[]>] [-Mask <string>] [-InPlace] [-Diff] [-Context <uint>] [-Hidden] [-NoIgnore] [-Binary] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
Protect-TrexText [-Pattern] <Object> -LiteralPath <string[]> [-Keep <string[]>] [-Mask <string>] [-InPlace] [-Diff] [-Context <uint>] [-Hidden] [-NoIgnore] [-Binary] [-Head <uint>] [-Tail <uint>] [-Lines <Object>] [-Unit <string>] [-Follow] [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Binary` | switch |  | Reads files that hold a NUL byte, which a walk treats as binary. |
| `-Context` | uint |  | Lines of context around each change in a diff; 3 when absent. |
| `-Diff` | switch |  | Writes the unified diff each file would take instead of its text. |
| `-Follow` | switch |  | Writes a file's masked text as it grows, a line at a time, after its tail, its open range or the whole of it, until the pipeline is stopped. |
| `-Head` | uint |  | Masks the first this many lines of each input, or records of -Unit, reading a file no further. |
| `-Hidden` | switch |  | Reads hidden files and directories a walk would skip. |
| `-InPlace` | switch |  | Writes each file that has a match back where it is. |
| `-InputObject` | string | by value | The text to mask; each string piped in is masked on its own. |
| `-Keep` | string[] |  | The fields to leave in place, as a register and an accessor: `card:last4`, `ip:octet1-2`, `e:domain`. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms a source-text pattern is compiled against, in place of the session's. |
| `-Lines` | object |  | Masks a range of lines, or records of -Unit, counted from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`. |
| `-LiteralPath` | string[] | by name | Files or directories to mask, read as written, as Get-ChildItem pipes them. |
| `-Mask` | string |  | What stands in for a masked character: one character, a token for each run, `shape`, or `pseudonym`; `*` when absent. |
| `-NoIgnore` | switch |  | Reads files an ignore rule excludes. |
| `-Path` | string[] |  | Files or directories to mask; wildcards expand. |
| `-Pattern` | object |  | The pattern: trex source text, or a Trex.Pattern. |
| `-Tail` | uint |  | Masks the last this many lines of each input, or records of -Unit, reading a file backward from its end. |
| `-Unit` | string |  | What -Head, -Tail and -Lines count: a line when absent, or a unit Find-TrexRecord reads, such as `paragraph` or `block`. |

Writes `string`.
