---
title: Use your own atoms from PowerShell
linkTitle: Your own atoms in PowerShell
weight: 70
---

# Use your own atoms from PowerShell

A log names things only your team knows the shape of: ticket ids, customer numbers. Declared
once as atoms, each is one token that every cmdlet reads by name, so a pattern says
`\{ticket}` rather than spelling out its bytes. This guide declares two in a pattern file,
tests them, and then finds, counts and masks them in a log.

The pattern file and the log:

```powershell
PS> Get-Content ./ops.trex
shape ticket = `[A-Z]{2,5}-\d{1,5}`
shape customer = `C\d{5}`
test ticket accepts "OPS-1234" "DB-77" rejects "ops-1234" "OPS1234"
test customer accepts "C00042" rejects "C42"
PS> Get-Content ./logs/ops.log
2026-09-27T09:00:01Z OPS-1234 opened for C00042 by ann
2026-09-27T09:05:12Z DB-77 linked to OPS-1234
2026-09-27T09:07:40Z OPS-1250 opened for C00077 by bob
2026-09-27T09:12:03Z OPS-1234 closed for C00042
```

## Test the declarations first

A library holds atoms apart from the session, so the file can be checked before anything uses
it. Each `test` line must pass: its `accepts` texts matched whole, its `rejects` texts matched
nowhere.

```powershell
PS> $ops = New-TrexLibrary -Path ./ops.trex
PS> Test-TrexAtom -Library $ops | Select-Object Name, Passed

Name     Passed
----     ------
ticket     True
customer   True
```

A shape that is too narrow fails its own test, and the failure says which text it could not
match:

```powershell
PS> $narrow = New-TrexLibrary -Declaration 'shape ticket = `[A-Z]{2,5}-\d{1,2}`', 'test ticket accepts "OPS-1234"'
PS> Test-TrexAtom -Library $narrow | Select-Object Name, Passed, Failures

Name   Passed Failures
----   ------ --------
ticket  False {accepts "OPS-1234": matched only "OPS-12" at 0..6…
```

## Declare them for the session

`Import-TrexAtom` declares the file for the rest of the session, and every cmdlet then lexes a
ticket and a customer number as one token each:

```powershell
PS> Import-TrexAtom ./ops.trex
PS> Get-TrexToken 'OPS-1234 opened for C00042' | Select-Object Kind, Text

Kind     Text
----     ----
ticket   OPS-1234
word     opened
word     for
customer C00042
```

## Find and count them

```powershell
PS> Select-TrexMatch '\{ticket}:t "opened" "for" \{customer}:c' -Path ./logs -Format '${line}: ${t} for ${c}'
1: OPS-1234 for C00042
3: OPS-1250 for C00077

PS> Group-TrexMatch '\{ticket}:t' -Key '${t}' -Path ./logs

Key      Count
---      -----
OPS-1234     3
DB-77        1
OPS-1250     1
```

## Mask them

`-Mask pseudonym` gives each distinct customer number the same stand-in wherever it occurs,
named after the atom, so the masked log still shows which lines are about one customer:

```powershell
PS> Protect-TrexText '\{customer}' -Mask pseudonym -Path ./logs
2026-09-27T09:00:01Z OPS-1234 opened for CUSTOMER_1 by ann
2026-09-27T09:05:12Z DB-77 linked to OPS-1234
2026-09-27T09:07:40Z OPS-1250 opened for CUSTOMER_2 by bob
2026-09-27T09:12:03Z OPS-1234 closed for CUSTOMER_1
```

## Use them from a script

A script that passes `-Library` reads the atoms it was given whatever the session has
declared, so it runs the same in a fresh session:

```powershell
PS> Unregister-TrexAtom -All
PS> Select-TrexMatch '\{customer}:c' -Path ./logs -Library $ops | Group-Object { $_.Captures.c } | Select-Object Name, Count

Name   Count
----   -----
C00042     2
C00077     1
```

The [atoms and settings](../../reference/powershell/atoms/) reference has every form of
declaration and every cmdlet that reads them.
