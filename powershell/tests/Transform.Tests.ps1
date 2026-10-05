BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Edit-TrexText' {
    It 'renders a template with a typed slice in place of each match' {
        'mail bob@x.com now' | Edit-TrexText '\E:e' '[${e:domain}]' | Should -BeExactly 'mail [x.com] now'
    }

    It 'replaces each match with what a script block returns for $_' {
        'a 1 b 22' | Edit-TrexText '\N' -ScriptBlock { [int]$_.Text * 2 } | Should -BeExactly 'a 2 b 44'
    }

    It 'gives the script block the match as its first argument too' {
        'a 1 b 22' | Edit-TrexText '\N:n' -ScriptBlock { param($m) "<$($m.Captures.n)>" } | Should -BeExactly 'a <1> b <22>'
    }

    It 'replaces only the first matches with -MaxCount' {
        'a 1 b 22' | Edit-TrexText '\N' 'X' -MaxCount 1 | Should -BeExactly 'a X b 22'
    }

    It 'rewrites a file in place, and leaves it under -WhatIf' {
        $file = Join-Path $TestDrive 'mail.txt'
        Write-TrexTestFile -Path $file -Text "mail bob@x.com now`n"
        Edit-TrexText '\E' '<m>' -Path $file -InPlace -WhatIf
        [System.IO.File]::ReadAllText($file) | Should -BeExactly "mail bob@x.com now`n"
        Edit-TrexText '\E' '<m>' -Path $file -InPlace -Confirm:$false
        [System.IO.File]::ReadAllText($file) | Should -BeExactly "mail <m> now`n"
    }

    It 'writes the unified diff a file would take with -Diff' {
        $file = Join-Path $TestDrive 'diff.txt'
        Write-TrexTestFile -Path $file -Text "a 1`nb 2`n"
        $diff = Edit-TrexText '\N' 'X' -Path $file -Diff
        $diff | Should -Match '@@'
        $diff | Should -Match '\+a X'
        [System.IO.File]::ReadAllText($file) | Should -BeExactly "a 1`nb 2`n"
    }

    It 'puts each change to the person with -Interactive, writing nothing where the host cannot ask' {
        $file = Join-Path $TestDrive 'review.txt'
        Write-TrexTestFile -Path $file -Text "an old word`n"
        { Edit-TrexText '"old"' 'new' -Path $file -Interactive -ErrorAction Stop } | Should -Throw '*NonInteractive*'
        [System.IO.File]::ReadAllText($file) | Should -BeExactly "an old word`n"
    }

    It 'warns of a binary file named outright and passes over one a walk finds' {
        $dir = Join-Path $TestDrive 'bins'
        New-Item -ItemType Directory -Path $dir | Out-Null
        $bin = Join-Path $dir 'b.bin'
        [System.IO.File]::WriteAllBytes($bin, [byte[]](0x61, 0x62, 0x00, 0x63))
        foreach ($mode in @{ Diff = $true }, @{ InPlace = $true; Confirm = $false }, @{}) {
            Edit-TrexText '"ab"' 'XY' -Path $bin @mode -WarningVariable said -WarningAction SilentlyContinue | Should -BeNullOrEmpty
            "$said" | Should -BeLike '*b.bin holds a NUL byte and is binary; -Binary reads it'
        }
        [System.IO.File]::ReadAllBytes($bin) | Should -Be ([byte[]](0x61, 0x62, 0x00, 0x63))
        Edit-TrexText '"ab"' 'XY' -Path $dir -Diff -WarningVariable walked -WarningAction SilentlyContinue | Should -BeNullOrEmpty
        $walked | Should -BeNullOrEmpty
    }

    It 'refuses two of -InPlace, -Diff and -Interactive, and -ShowSkipped or -Explain without a review' {
        $file = Join-Path $TestDrive 'both.txt'
        Write-TrexTestFile -Path $file -Text "a 1`n"
        { Edit-TrexText '\N' 'X' -Path $file -InPlace -Diff -ErrorAction Stop } | Should -Throw '*give one*'
        { Edit-TrexText '\N' 'X' -Path $file -Diff -ShowSkipped -ErrorAction Stop } | Should -Throw '*only -Interactive*'
        { Edit-TrexText '\N' 'X' -Path $file -InPlace -Explain -ErrorAction Stop } | Should -Throw '*give -Interactive*'
    }

    It 'finds the matches with the engine -Backend names, the device saying when it cannot take them' {
        $file = Join-Path $TestDrive 'engine.txt'
        Write-TrexTestFile -Path $file -Text "a 1`nb 22`n"
        Edit-TrexText '\N' 'X' -Path $file -Diff -Backend Cpu | Should -Match '\+b X'
        $diff = Edit-TrexText '\N' 'X' -Path $file -Diff -Backend Gpu -Verbose 4>&1
        ($diff | Where-Object { $_ -is [string] }) | Should -Match '\+b X'
        @($diff | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] }).Count | Should -Be 1
        'a 1 b 22' | Edit-TrexText '\N' 'X' -Backend Gpu -MaxCount 1 | Should -BeExactly 'a X b 22'
    }
}

Describe 'Protect-TrexText' {
    It 'masks each match and keeps the fields named' {
        'mail bob@x.com now' | Protect-TrexText '\E:e' -Keep e:domain | Should -BeExactly 'mail ****x.com now'
    }

    It 'masks with the character given' {
        'mail bob@x.com now' | Protect-TrexText '\E' -Mask '#' | Should -BeExactly 'mail ######### now'
    }

    It 'keeps every character but letters and digits with -Mask shape' {
        $masked = 'mail bob@x.com now' | Protect-TrexText '\E' -Mask shape
        $masked.Length | Should -Be 'mail bob@x.com now'.Length
        $masked.Substring(8, 1) | Should -BeExactly '@'
        $masked | Should -Not -Match 'bob'
    }

    It 'refuses a -Keep naming a register the pattern does not bind' {
        { 'x' | Protect-TrexText '\E:e' -Keep 'z:domain' -ErrorAction Stop } | Should -Throw '*-Keep error*'
    }

    It 'masks a file in place, and leaves it under -WhatIf' {
        $file = Join-Path $TestDrive 'mask.txt'
        Write-TrexTestFile -Path $file -Text "mail bob@x.com now`n"
        Protect-TrexText '\E' -Path $file -InPlace -WhatIf
        [System.IO.File]::ReadAllText($file) | Should -BeExactly "mail bob@x.com now`n"
        Protect-TrexText '\E' -Path $file -InPlace -Confirm:$false
        [System.IO.File]::ReadAllText($file) | Should -BeExactly "mail ********* now`n"
    }

    It 'puts each masking to the person with -Interactive, writing nothing where the host cannot ask' {
        $file = Join-Path $TestDrive 'review-mask.txt'
        Write-TrexTestFile -Path $file -Text "mail bob@x.com now`n"
        { Protect-TrexText '\E' -Path $file -Interactive -Explain -ErrorAction Stop } | Should -Throw '*NonInteractive*'
        [System.IO.File]::ReadAllText($file) | Should -BeExactly "mail bob@x.com now`n"
        { Protect-TrexText '\E' -Path $file -InPlace -Interactive -ErrorAction Stop } | Should -Throw '*give one*'
        { Protect-TrexText '\E' -Path $file -Diff -ShowSkipped -ErrorAction Stop } | Should -Throw '*only -Interactive*'
        { Protect-TrexText '\E' -Path $file -Diff -Explain -ErrorAction Stop } | Should -Throw '*give -Interactive*'
    }
}
