---
title: Atoms and settings
linkTitle: Atoms and settings
weight: 60
---

# Atoms and settings

An atom is a name a pattern reads as `\{name}`: a byte shape the lexer tries before its
built-in recognizers, a kind fused from a pattern into one token, or a named sub-pattern.
`Register-TrexAtom` declares one for the session and `Import-TrexAtom` declares what a pattern
file says, and from then on every cmdlet that takes a pattern reads them. The session's atoms
live in `$TrexSession`; a [`Trex.Library`](../types/#trexlibrary) holds a set of its own, which
a cmdlet reads in their place when `-Library` passes it. The examples read this file:

```powershell
PS> Get-Content ./defs.trex
shape ticket = `[A-Z]{2,4}-\d{1,4}`
let level = "INFO" | "WARN" | "ERROR"
test ticket accepts "AB-12" "OPS-9" rejects "ab-12" "A-1"
test level accepts "WARN" rejects "DEBUG"
```

## Shapes

A shape is a bounded byte-pattern; an unbounded `*`, `+` or `{m,}` is refused. `-Accepts` and
`-Rejects` write the atom's test line, which `Test-TrexAtom` runs:

```powershell
PS> Register-TrexAtom sku -Shape '[A-Z]{3}-\d{3}' -Accepts 'ABC-123' -Rejects 'AB-123' -PassThru

Name Form  Definition     Source  Description
---- ----  ----------     ------  -----------
sku  Shape [A-Z]{3}-\d{3} session

PS> Get-TrexToken 'order ABC-123 shipped'

Kind Text    Start Length Value
---- ----    ----- ------ -----
word order   0     5
sku  ABC-123 6     7
word shipped 14    7

PS> Select-TrexMatch '\{sku}:s' -InputObject 'ABC-123 and XYZ-789' | Select-Object Text, @{ n = 'Kind'; e = { $_.Groups[0].Kind } }

Text    Kind
----    ----
ABC-123 sku
XYZ-789 sku

PS> Test-TrexAtom sku

Name     : sku
Passed   : True
Accepts  : {ABC-123}
Rejects  : {AB-123}
Failures : {}
File     :
```

`-After` tries the shape only where no built-in recognizer matched, so a shape that overlaps a
built-in kind leaves that kind alone.

## Kinds and sub-patterns

`-Pattern` names a sub-pattern, read in place wherever `\{name}` appears. `-Kind` fuses every
match of a pattern, after the lex, into one token of its own kind, which a scan then reports
whole:

```powershell
PS> Register-TrexAtom rhs -Pattern '\N | \Q'
PS> Register-TrexAtom assign -Kind '\W "=" \{rhs}'
PS> Get-TrexToken 'let x = 1; name = "bob"' | Select-Object Kind, Text

Kind   Text
----   ----
word   let
assign x = 1
punct  ;
assign name = "bob"

PS> Select-TrexMatch '\{assign}' -InputObject 'let x = 1; name = "bob"' -Raw
x = 1
name = "bob"
```

## Pattern files

`Import-TrexAtom` declares every `let`, `kind`, `shape`, `shape-after`, `rule` and `test` line of
the files named. Importing a file again reads its current text in place of the earlier import:

```powershell
PS> Import-TrexAtom ./defs.trex -PassThru | Select-Object Name, Form, Definition

Name      Form Definition
----      ---- ----------
ticket   Shape `[A-Z]{2,4}-\d{1,4}`
level  Pattern "INFO" | "WARN" | "ERROR"

PS> Get-TrexAtom | Select-Object Name, Form, @{ n = 'Source'; e = { Split-Path -Leaf $_.Source } }

Name      Form Source
----      ---- ------
sku      Shape session
rhs    Pattern session
assign    Kind session
ticket   Shape defs.trex
level  Pattern defs.trex

PS> Test-TrexAtom | Select-Object Name, Passed, Accepts, Rejects

Name   Passed Accepts        Rejects
----   ------ -------        -------
sku      True {ABC-123}      {AB-123}
ticket   True {AB-12, OPS-9} {ab-12, A-1}
level    True {WARN}         {DEBUG}

PS> Test-TrexAtom -Quiet
True
```

`Unregister-TrexAtom` removes atoms by name, by the file that declared them, or all of them:

```powershell
PS> Unregister-TrexAtom sku, rhs, assign
PS> Unregister-TrexAtom -Path ./defs.trex
PS> @(Get-TrexAtom).Count
0
```

## Libraries

`New-TrexLibrary` makes a library from pattern files, declaration lines, or a copy of the
session's atoms, and its `Declare` method adds a line. A script that passes its own library
reads the same atoms whatever the session has declared:

```powershell
PS> $lib = New-TrexLibrary -Path ./defs.trex -Declaration 'let big = \N{>1000}'
PS> $lib.Names
ticket
level
big

PS> Get-TrexToken 'OPS-9 opened' -Library $lib | Select-Object Kind, Text

Kind   Text
----   ----
ticket OPS-9
word   opened

PS> Select-TrexMatch '\{big}' -InputObject 'sizes 5 and 5000' -Library $lib -Raw
5000

PS> $lib.Declare('let small = \N{<10}')
PS> Get-TrexAtom -Library $lib | Select-Object Name, Form

Name      Form
----      ----
ticket   Shape
level  Pattern
big    Pattern
small  Pattern

PS> Register-TrexAtom sku -Shape '[A-Z]{3}-\d{3}' -Library $lib
PS> @(Get-TrexAtom sku).Count
0
```

## Shipped atoms

trex ships a library that every pattern reads with no declaration, and `-Shipped` lists and
tests it:

```powershell
PS> @(Get-TrexAtom -Shipped).Count
75

PS> Get-TrexAtom iban -Shipped | Select-Object Name, Form, Description

Name Form Description
---- ---- -----------
iban Kind an IBAN, compact or in groups of four, with its country's length and the mod 97-10 check

PS> Test-TrexAtom -Shipped -Quiet
True
```

## The clock

The clock is what typed predicates read: `\T{age<24h}` compares a timestamp with its `now`, a
timestamp written with no zone is read at its offset, and an all-numeric slash date is read day
first or month first by its order. `Set-TrexClock` sets it for every later scan in the process:

```powershell
PS> (Set-TrexClock -Now '2026-09-27T12:00:00Z' -PassThru).Now.ToString('o')
2026-09-27T12:00:00.0000000+00:00

PS> 'seen 2026-09-27T09:00:00Z and 2026-09-20T09:00:00Z' | Select-TrexMatch '\T{age<24h}' -Raw
2026-09-27T09:00:00Z

PS> (Get-TrexToken '05/09/2026').Value.ToString('yyyy-MM-dd')
2026-09-05

PS> Set-TrexClock -DateOrder MonthFirst
PS> (Get-TrexToken '05/09/2026').Value.ToString('yyyy-MM-dd')
2026-05-09

PS> Set-TrexClock -SystemClock -DateOrder DayFirst
PS> Get-TrexClock | Select-Object Fixed, TimeZoneOffset, DateOrder

Fixed TimeZoneOffset DateOrder
----- -------------- ---------
False 00:00:00        DayFirst
```

`Get-TrexInfo` writes the engine's version and whether a CUDA device is present:

```powershell
PS> (Get-TrexInfo).Version
0.1.0
```

## Cmdlets

### Register-TrexAtom

Declares an atom a pattern reads by name as `\{name}`: a byte shape, a kind fused from a pattern, or a named sub-pattern.

With no -Library the atom is declared for this PowerShell session, and every later cmdlet that takes a pattern reads it. -Accepts and -Rejects add expectations that Test-TrexAtom checks.

Alias: `Register-TxAtom`

```powershell
Register-TrexAtom [-Name] <string> [-Shape] <string> [-After] [-Accepts <string[]>] [-Rejects <string[]>] [-Library <Library>] [-Force] [-PassThru] [<CommonParameters>]
Register-TrexAtom [-Name] <string> -Kind <string> [-Accepts <string[]>] [-Rejects <string[]>] [-Library <Library>] [-Force] [-PassThru] [<CommonParameters>]
Register-TrexAtom [-Name] <string> -Pattern <string> [-Accepts <string[]>] [-Rejects <string[]>] [-Library <Library>] [-Force] [-PassThru] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Accepts` | string[] |  | Texts the atom must match whole, from the first token to the last. |
| `-After` | switch |  | Tries the shape only where no built-in recognizer matched. |
| `-Force` | switch |  | Replaces an atom of the same name declared here. |
| `-Kind` | string |  | A pattern whose every match, after the lex, becomes one token of this kind. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The library to declare into, in place of the session. |
| `-Name` | string |  | The name a pattern reads the atom by, as `\{name}`: letters, digits and underscores. |
| `-PassThru` | switch |  | Writes the declared atom. |
| `-Pattern` | string |  | A pattern read in place wherever `\{name}` appears. |
| `-Rejects` | string[] |  | Texts the atom must match nowhere. |
| `-Shape` | string |  | A bounded byte-pattern the lexer tries at each position before its built-in recognizers, such as `[A-Z]{2,4}-\d{1,4}`; an unbounded `*`, `+` or `{m,}` is refused. |

Writes [Trex.Atom](../types/#trexatom).

### Import-TrexAtom

Declares every atom the pattern files say: their `let`, `kind`, `shape`, `shape-after`, `rule` and `test` lines, with a relative `@file` set read from beside each file.

A file saved as UTF-8, UTF-16 or UTF-32 with a byte-order mark reads as its text. Importing a file again reads its current text in place of the earlier import.

Alias: `Import-TxAtom`

```powershell
Import-TrexAtom [-Path] <string[]> [-Library <Library>] [-PassThru] [<CommonParameters>]
Import-TrexAtom -LiteralPath <string[]> [-Library <Library>] [-PassThru] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The library to declare into, in place of the session. |
| `-LiteralPath` | string[] | by name | Pattern files to import, read as written, as Get-ChildItem pipes them. |
| `-PassThru` | switch |  | Writes each atom the files declare. |
| `-Path` | string[] |  | Pattern files to import; wildcards expand. |

Writes [Trex.Atom](../types/#trexatom).

### Get-TrexAtom

Lists the atoms declared for this session, in a library, or shipped with trex, in declaration order.

Alias: `Get-TxAtom`

```powershell
Get-TrexAtom [[-Name] <string>] [-Library <Library>] [-Shipped] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | Lists a library's atoms in place of the session's. |
| `-Name` | string |  | Only atoms whose name matches; wildcards apply. |
| `-Shipped` | switch |  | Lists the atoms trex ships, which every pattern reads with no declaration. |

Writes [Trex.Atom](../types/#trexatom).

### Test-TrexAtom

Runs the `test` lines declared for the session, a library, or the atoms trex ships, and writes one result per line.

A line's `accepts` texts must each be matched whole by the atom, and its `rejects` texts matched nowhere. Register-TrexAtom writes a test line from -Accepts and -Rejects; a pattern file writes its own.

Alias: `Test-TxAtom`

```powershell
Test-TrexAtom [[-Name] <string>] [-Library <Library>] [-Shipped] [-Quiet] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | Tests a library's atoms in place of the session's. |
| `-Name` | string |  | Only the test lines for atoms whose name matches; wildcards apply. |
| `-Quiet` | switch |  | Writes only whether every test line passed. |
| `-Shipped` | switch |  | Tests the atoms trex ships. |

Writes [Trex.AtomTest](../types/#trexatomtest), `bool`.

### Unregister-TrexAtom

Removes atoms from the session or a library: by name, by the pattern file that declared them, or all of them.

Every name and file given, and every one piped in, is removed as one change after the last, so atoms that read one another go together in any order. An atom another one still reads stays, with an error naming it.

Alias: `Unregister-TxAtom`

```powershell
Unregister-TrexAtom [-Name] <string[]> [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
Unregister-TrexAtom -Path <string[]> [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
Unregister-TrexAtom -All [-Library <Library>] [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-All` | switch |  | Removes every atom and every imported file. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The library to remove from, in place of the session. |
| `-Name` | string[] | by name | The names to remove. |
| `-Path` | string[] |  | Imported pattern files to remove, with every atom they declared. |

### New-TrexLibrary

Makes a library: a set of atoms held in an object, passed to a cmdlet with -Library, which reads it in place of the session's atoms.

Alias: `New-TxLibrary`

```powershell
New-TrexLibrary [[-Path] <string[]>] [-Declaration <string[]>] [-FromSession] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Declaration` | string[] |  | Declaration lines, each as a pattern file writes one. |
| `-FromSession` | switch |  | Starts from a copy of the session's atoms. |
| `-Path` | string[] |  | Pattern files to declare; wildcards expand. |

Writes [Trex.Library](../types/#trexlibrary).

### Get-TrexClock

Writes the clock typed predicates read: the instant `now` is, whether it is fixed, the offset a timestamp with no zone is read at, and the order of a slash date's fields.

Alias: `Get-TxClock`

```powershell
Get-TrexClock [<CommonParameters>]
```

Writes [Trex.Clock](../types/#trexclock).

### Set-TrexClock

Sets the clock typed predicates read, for every later scan in this PowerShell process.

-Now fixes the instant `\T{age<24h}` and `\T{<now}` compare against, and -SystemClock returns to reading the system clock. -TimeZoneOffset is the offset a timestamp written with no zone is read at, UTC until set. -DateOrder says which field of an all-numeric slash date is the day.

Alias: `Set-TxClock`

```powershell
Set-TrexClock [-Now <DateTimeOffset>] [-SystemClock] [-TimeZoneOffset <timespan>] [-DateOrder <DateOrder>] [-PassThru] [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-DateOrder` | [Trex.DateOrder](../types/#trexdateorder) |  | Which field of an all-numeric slash date is the day. |
| `-Now` | DateTimeOffset |  | The instant `now` reads as from here on. |
| `-PassThru` | switch |  | Writes the clock as it stands after the change. |
| `-SystemClock` | switch |  | Returns `now` to the system clock. |
| `-TimeZoneOffset` | TimeSpan |  | The offset a timestamp written with no zone is read at. |

Writes [Trex.Clock](../types/#trexclock).

### Get-TrexInfo

Writes the trex engine's version and whether a CUDA device is present.

Alias: `Get-TxInfo`

```powershell
Get-TrexInfo [<CommonParameters>]
```

Writes [Trex.Info](../types/#trexinfo).
