---
title: Atoms and settings
linkTitle: Atoms and settings
weight: 60
---

An atom is a name a pattern reads as `\{name}`: a byte shape, a kind fused from a pattern, or a
named sub-pattern. The session's atoms live in `$TrexSession`; a
[`Trex.Library`](../types/#trexlibrary) holds a set of its own, which a cmdlet reads in their
place when `-Library` passes it. The examples are on [pattern files](../../pattern-files/) and
[typed value predicates](../../pattern-syntax/#typed-value-predicates).

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

Declares every atom the pattern files say: their `let`, `kind`, `shape`, `shape-after`, `rule` and `test` lines, with a relative `@file` set read from beside each file. A directory reads as every `.trex` file under it, in path order.

A file saved as UTF-8, UTF-16 or UTF-32 with a byte-order mark reads as its text. Importing a file again reads its current text in place of the earlier import.

Alias: `Import-TxAtom`

```powershell
Import-TrexAtom [-Path] <string[]> [-Library <Library>] [-PassThru] [<CommonParameters>]
Import-TrexAtom -LiteralPath <string[]> [-Library <Library>] [-PassThru] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The library to declare into, in place of the session. |
| `-LiteralPath` | string[] | by name | Pattern files to import, read as written, as Get-ChildItem pipes them; a directory as every `.trex` file under it. |
| `-PassThru` | switch |  | Writes each atom the files declare. |
| `-Path` | string[] |  | Pattern files to import, a directory as every `.trex` file under it; wildcards expand. |

Writes [Trex.Atom](../types/#trexatom).

### Get-TrexAtom

Lists the atoms declared for this session, in a library, or shipped with TREX, in declaration order.

Alias: `Get-TxAtom`

```powershell
Get-TrexAtom [[-Name] <string>] [-Library <Library>] [-Shipped] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | Lists a library's atoms in place of the session's. |
| `-Name` | string |  | Only atoms whose name matches; wildcards apply. |
| `-Shipped` | switch |  | Lists the atoms TREX ships, which every pattern reads with no declaration. |

Writes [Trex.Atom](../types/#trexatom).

### Test-TrexAtom

Runs the `test` lines declared for the session, a library, or the atoms TREX ships, and writes one result per line.

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
| `-Shipped` | switch |  | Tests the atoms TREX ships. |

Writes [Trex.AtomTest](../types/#trexatomtest), `bool`.

### Unregister-TrexAtom

Removes atoms from the session or a library: by name, by the pattern file that declared them, by a directory files were imported from under, or all of them.

Every name and path given, and every one piped in, is removed as one change after the last, so atoms that read one another go together in any order, each with the lines declared that add to it: its `test` lines, and a rule's `fix` and `meta`. An atom another one still reads stays, with an error naming it.

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
| `-Path` | string[] |  | Imported pattern files to remove, with every atom they declared; a directory removes every file imported from under it. |

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
| `-Path` | string[] |  | Pattern files to declare, a directory as every `.trex` file under it; wildcards expand. |

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
| `-PassThru` | switch |  | Writes the clock's settings after the change. |
| `-SystemClock` | switch |  | Returns `now` to the system clock. |
| `-TimeZoneOffset` | TimeSpan |  | The offset a timestamp written with no zone is read at. |

Writes [Trex.Clock](../types/#trexclock).

### Get-TrexInfo

Writes the TREX engine's version and whether a CUDA device is present.

Alias: `Get-TxInfo`

```powershell
Get-TrexInfo [<CommonParameters>]
```

Writes [Trex.Info](../types/#trexinfo).
