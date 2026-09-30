---
title: Rewriting
linkTitle: Rewriting
weight: 20
---

# Rewriting

`Edit-TrexText` is the module's `trex rewrite` and `Protect-TrexText` its `trex redact`. Text
piped in comes back rewritten; files are written back only with `-InPlace`, or shown as the
diff they would take with `-Diff`. The examples read these files:

```powershell
PS> Get-Content ./src/notes.md
The answer is 42, and legacy_call() is the old name.
See legacy_call in the guide.
PS> Get-Content ./src/main.rs
fn main() {
    legacy_call(42);
}
PS> Get-Content ./logs/app.log
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```

## Templates

A template renders `${name}` for a register, `${0}` for the whole match, `${name:upper}` for a
transform and `${ip:octet1-2}` for a typed slice, as the
[rewrite accessors](../../pattern-syntax/#rewrite-accessors) define them:

```powershell
PS> 'mail bob@x.com now' | Edit-TrexText '\E:e' '[${e:domain}]'
mail [x.com] now

PS> 'from 10.1.2.3' | Edit-TrexText '\I:ip' '${ip:octet1-2}.0.0'
from 10.1.0.0

PS> 'level warn' | Edit-TrexText '"level" \W:l' 'level ${l:upper}'
level WARN

PS> '1 2 3 4' | Edit-TrexText '\N' 'n' -MaxCount 2
n n 3 4
```

## Script blocks

With `-ScriptBlock` the block runs once per match with the [`Trex.Match`](../types/#trexmatch)
as `$_`, and its last output replaces the match, so a replacement can be computed from a
register's typed value:

```powershell
PS> 'a 1 b 22' | Edit-TrexText '\N' -ScriptBlock { [int]$_.Text * 2 }
a 2 b 44

PS> 'took 1500ms, then 2s' | Edit-TrexText '\R:t' -ScriptBlock { '{0}ms' -f $_.Groups[0].Value.TotalMilliseconds }
took 1500ms, then 2000ms
```

## Files

`-Path` writes each file's rewritten text, `-Diff` the unified diff each file would take, and
`-InPlace` writes the files that hold a match back where they are:

```powershell
PS> Edit-TrexText '"legacy_call"' 'current_call' -Path ./src -Diff
--- C:\Temp\demo\src\main.rs
+++ C:\Temp\demo\src\main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    legacy_call(42);
+    current_call(42);
 }

--- C:\Temp\demo\src\notes.md
+++ C:\Temp\demo\src\notes.md
@@ -1,2 +1,2 @@
-The answer is 42, and legacy_call() is the old name.
-See legacy_call in the guide.
+The answer is 42, and current_call() is the old name.
+See current_call in the guide.

PS> Edit-TrexText '"legacy_call"' 'current_call' -Path ./src -InPlace
PS> Get-Content -Path ./src/notes.md
The answer is 42, and current_call() is the old name.
See current_call in the guide.
```

`-InPlace` asks `ShouldProcess` for each file, so `-WhatIf` says which files it would write and
writes none, and `-Confirm` asks for each. A file is written back in the encoding it was read
in, behind its own byte order mark, and every byte outside a match stays as it was, a byte the
string could not hold among them.

## Parts of a file

`-Head`, `-Tail` and `-Lines` rewrite or mask only part of each input, counted in lines or in the
records `-Unit` names: the text written is that part alone, so nothing outside it comes back
unmasked, and a file written back or shown as its diff changes only there. `-Follow` writes a
file's rewritten text as it grows, a line at a time, until the pipeline is stopped:

```powershell
PS> Edit-TrexText '\R:t' '<${t}>' -Path ./logs/app.log -Tail 1
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took <95ms>

PS> Protect-TrexText '\E' -Path ./logs/app.log -Head 1
2026-09-27T09:00:04Z ERROR payment for ********* failed: card 4111 1111 1111 1111 declined
```

## Review

`-Interactive` puts each change to the person at the host's prompt before anything is written,
as its unified diff with the template of the tokens it replaces and how many later changes share
that template, and writes the files once the last change has been answered:

| Answer | Does |
|---|---|
| `Y` Yes | applies this change |
| `N` No | skips this change; the default |
| `E` Edit | reads another replacement for this change |
| `T` Template | applies this change and every later one of the same template |
| `S` Skip template | skips this change and every later one of the same template |
| `A` Yes to All | applies this change and every one after it |
| `Q` Quit | stops, leaving every file as it was |

How many changes a template answer skipped is said as a warning, and `-ShowSkipped` adds where
each one stands. A host that cannot prompt, such as `pwsh -NonInteractive`, stops with an error
at the first question and writes nothing.

## Masking

`Protect-TrexText` masks each match, keeping the fields `-Keep` names. `-Mask` sets what stands
in for what is masked: one character for each character, a longer token for each masked run,
`shape`, which masks letters and digits and keeps every other character so the text still lexes
as it did, or `pseudonym`, which gives each distinct value a stable name per kind:

```powershell
PS> 'mail bob@x.com now' | Protect-TrexText '\E:e' -Keep e:domain
mail ****x.com now

PS> 'card 4111 1111 1111 1111' | Protect-TrexText '\{card}:c' -Keep c:last4
card ***************1111

PS> 'mail bob@x.com now' | Protect-TrexText '\E' -Mask '[email]'
mail [email] now

PS> 'from 10.1.2.3 at 09:15' | Protect-TrexText '\I' -Mask shape
from 00.0.0.0 at 09:15

PS> 'bob@x.com, ann@y.org, bob@x.com' | Protect-TrexText '\E' -Mask pseudonym
EMAIL_1, EMAIL_2, EMAIL_1

PS> Protect-TrexText '\{card}:c' -Keep c:last4 -Path ./logs -Diff
--- C:\Temp\demo\logs\app.log
+++ C:\Temp\demo\logs\app.log
@@ -1,2 +1,2 @@
-2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
+2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card ***************1111 declined
 2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```

## Cmdlets

### Edit-TrexText

Replaces each match of a trex pattern with a rendered template or with what a script block returns for it.

With -Template, `${name}` renders a register, `${0}` the whole match, `${name:upper}` a transform and `${ip:octet1-2}` a typed slice. With -ScriptBlock, the block runs once per match with the Trex.Match as `$_`, and its last output replaces the match. Text piped in comes back rewritten; -Path writes each file's rewritten text, -InPlace writes the files that have a match back where they are, and -Diff writes the unified diff each would take. -MaxCount replaces only the first matches of each input.

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

Every character of a match becomes the mask, `*` when -Mask is not given, except the fields -Keep names, which stay where they stand: `card:last4` keeps a card's last four digits, `ip:octet1-2` an address's first two octets, `e:domain` a mail domain. -Mask takes one character, a token that stands in for each masked run, `shape`, which masks letters and digits and keeps every other character so the text still lexes as it did, or `pseudonym`, which gives each distinct value a stable name per kind. Text piped in comes back masked; -Path writes each file's masked text, -InPlace writes the files back, and -Diff writes the unified diff.

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
