BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Session atoms' {
    BeforeEach {
        Unregister-TrexAtom -All
    }

    It 'declares a shape that a pattern reads by name' {
        Register-TrexAtom ticket -Shape '[A-Z]{2,4}-\d{1,4}'
        $found = Select-TrexMatch '\{ticket}' -InputObject 'see AB-12 and XYZ-9 now'
        @($found).Text | Should -Be @('AB-12', 'XYZ-9')
        (Select-TrexMatch '\{ticket}:t' -InputObject 'see AB-12').Groups[0].Kind | Should -BeExactly 'ticket'
        (New-TrexPattern '\{ticket}:t').Find('see AB-12').Groups[0].Kind | Should -BeExactly 'ticket'
    }

    It 'declares a sub-pattern and a kind built from it' {
        Register-TrexAtom rhs -Pattern '\N | \Q'
        Register-TrexAtom assign -Kind '\W "=" \{rhs}'
        $found = Select-TrexMatch '\{assign}' -InputObject 'let x = 1; name = "bob"'
        @($found).Text | Should -Be @('x = 1', 'name = "bob"')
        (Select-TrexMatch '\{assign}:a' -InputObject 'let x = 1').Groups[0].Kind | Should -BeExactly 'assign'
        (Select-TrexMatch '\{assign}:a', '\E:e' -InputObject 'let x = 1').Groups[0].Kind | Should -BeExactly 'assign'
    }

    It 'removes atoms that read one another together, in any order' {
        Register-TrexAtom rhs -Pattern '\N | \Q'
        Register-TrexAtom assign -Kind '\W "=" \{rhs}'
        Register-TrexAtom other -Shape '[A-Z]{3}'
        Unregister-TrexAtom rhs, assign
        (Get-TrexAtom).Name | Should -Be @('other')
        Register-TrexAtom rhs -Pattern '\N | \Q'
        Register-TrexAtom assign -Kind '\W "=" \{rhs}'
        Get-TrexAtom | Where-Object Name -ne 'other' | Unregister-TrexAtom
        (Get-TrexAtom).Name | Should -Be @('other')
    }

    It 'keeps an atom another still reads, and says why' {
        Register-TrexAtom rhs -Pattern '\N | \Q'
        Register-TrexAtom assign -Kind '\W "=" \{rhs}'
        { Unregister-TrexAtom rhs -ErrorAction Stop } | Should -Throw '*\{rhs}*'
        (Get-TrexAtom).Name | Should -Be @('rhs', 'assign')
    }

    It 'refuses a second atom of one name unless -Force replaces it' {
        Register-TrexAtom code -Shape '[A-Z]{3}'
        { Register-TrexAtom code -Shape '[A-Z]{4}' -ErrorAction Stop } | Should -Throw '*already declared*'
        Register-TrexAtom code -Shape '[A-Z]{4}' -Force
        (Get-TrexAtom code).Definition | Should -BeExactly '[A-Z]{4}'
    }

    It 'refuses an unbounded shape and leaves the session as it was' {
        { Register-TrexAtom loose -Shape '[A-Z]+' -ErrorAction Stop } | Should -Throw '*bounded*'
        Get-TrexAtom | Should -BeNullOrEmpty
    }

    It 'refuses a byte-pattern escape, group flag or class backslash it does not define' {
        { Register-TrexAtom limb -Shape '\x00{4}' -ErrorAction Stop } | Should -Throw '*is not a byte-pattern escape*'
        { Register-TrexAtom limb -Shape '(?s:.){4}' -ErrorAction Stop } | Should -Throw '*is not byte-pattern syntax*'
        { Register-TrexAtom limb -Shape '[\x00-\xff]{4}' -ErrorAction Stop } | Should -Throw '*class takes no backslash*'
        Get-TrexAtom | Should -BeNullOrEmpty
    }

    It 'checks -Accepts and -Rejects with Test-TrexAtom' {
        Register-TrexAtom ticket -Shape '[A-Z]{2,4}-\d{1,4}' -Accepts 'AB-12' -Rejects 'A-1'
        $result = Test-TrexAtom ticket
        $result.Passed | Should -BeTrue
        $result.Accepts | Should -Be @('AB-12')
        Register-TrexAtom ticket -Shape '[A-Z]{2,4}-\d{1,4}' -Accepts 'A-1' -Force
        (Test-TrexAtom ticket).Passed | Should -BeFalse
        Test-TrexAtom -Quiet | Should -BeFalse
    }

    It 'lists what the session declares, in order, and removes by name' {
        Register-TrexAtom one -Shape '[a-z]{2}1'
        Register-TrexAtom two -Pattern '\N'
        (Get-TrexAtom).Name | Should -Be @('one', 'two')
        (Get-TrexAtom t*).Form | Should -Be ([Trex.AtomForm]::Pattern)
        Get-TrexAtom one | Unregister-TrexAtom
        (Get-TrexAtom).Name | Should -Be @('two')
    }

    It 'reports a name nothing declares as an error' {
        { Unregister-TrexAtom nothing -ErrorAction Stop } | Should -Throw '*no atom named nothing*'
    }

    It 'holds the session atoms in $TrexSession' {
        Register-TrexAtom ticket -Shape '[A-Z]{2,4}-\d{1,4}'
        $TrexSession | Should -BeOfType ([Trex.Library])
        $TrexSession.Names | Should -Be @('ticket')
        (Select-TrexMatch '\{ticket}' -InputObject 'see AB-12' -Library $TrexSession).Text | Should -BeExactly 'AB-12'
    }

    It 'compiles against them through New-TrexPattern, and not through the constructor' {
        Register-TrexAtom ticket -Shape '[A-Z]{2,4}-\d{1,4}'
        (New-TrexPattern '\{ticket}').IsMatch('see AB-12') | Should -BeTrue
        { [Trex.Pattern]::new('\{ticket}') } | Should -Throw '*unknown named atom*'
    }

    It 'starts a runspace of its own with none' {
        Register-TrexAtom ticket -Shape '[A-Z]{2,4}-\d{1,4}'
        $other = [powershell]::Create()
        try {
            $module = Join-Path $env:PWRS_MODULE 'Trex.psd1'
            $null = $other.AddScript("Import-Module '$module'; @(Get-TrexAtom).Count")
            $other.Invoke() | Should -Be 0
        }
        finally {
            $other.Dispose()
        }
    }
}

Describe 'Import-TrexAtom' {
    BeforeEach {
        Unregister-TrexAtom -All
    }

    It 'declares a pattern file in <Encoding>' -ForEach @(
        @{ Encoding = 'Utf8' }, @{ Encoding = 'Utf8Bom' }, @{ Encoding = 'Utf16' }
    ) {
        $defs = Join-Path $TestDrive "defs-$Encoding.trex"
        Write-TrexTestFile -Path $defs -Text "let rhs = \N | \Q`r`nkind assign = \W `"=`" \{rhs}`r`n" -Encoding $Encoding
        Import-TrexAtom $defs
        @(Select-TrexMatch '\{assign}' -InputObject 'let x = 1; name = "bob"').Count | Should -Be 2
        (Get-TrexAtom).Source | Select-Object -Unique | Should -BeLike "*defs-$Encoding.trex"
    }

    It 'removes a file with every atom it declared' {
        $defs = Join-Path $TestDrive 'removed.trex'
        Write-TrexTestFile -Path $defs -Text "shape code = ``[A-Z]{3}```n"
        Import-TrexAtom $defs
        (Get-TrexAtom).Name | Should -Be @('code')
        { Unregister-TrexAtom code -ErrorAction Stop } | Should -Throw '*declared by*'
        Unregister-TrexAtom -Path $defs
        Get-TrexAtom | Should -BeNullOrEmpty
    }

    It 'imports a directory as every .trex file under it, and removes it the same way' {
        $dir = Join-Path $TestDrive 'atoms'
        New-Item -ItemType Directory -Path (Join-Path $dir 'more') | Out-Null
        Write-TrexTestFile -Path (Join-Path $dir 'a.trex') -Text "let rhs = \N | \Q`n"
        Write-TrexTestFile -Path (Join-Path $dir 'more\b.trex') -Text "kind assign = \W `"=`" \{rhs}`n"
        Write-TrexTestFile -Path (Join-Path $dir 'notes.txt') -Text "let ignored = \N`n"
        (Import-TrexAtom $dir -PassThru).Name | Should -Be @('rhs', 'assign')
        (Get-TrexAtom).Name | Should -Be @('rhs', 'assign')
        @(Select-TrexMatch '\{assign}' -InputObject 'let x = 1; name = "bob"').Count | Should -Be 2
        Unregister-TrexAtom -Path (Join-Path $dir 'more')
        (Get-TrexAtom).Name | Should -Be @('rhs')
        Unregister-TrexAtom -Path $dir
        Get-TrexAtom | Should -BeNullOrEmpty
        { Unregister-TrexAtom -Path $dir -ErrorAction Stop } | Should -Throw '*nothing imported here is*'
        $empty = Join-Path $TestDrive 'empty'
        New-Item -ItemType Directory -Path $empty | Out-Null
        { Import-TrexAtom $empty -ErrorAction Stop } | Should -Throw '*no .trex file under it*'
    }

    It 'makes a library from a directory' {
        $dir = Join-Path $TestDrive 'libdir'
        New-Item -ItemType Directory -Path $dir | Out-Null
        Write-TrexTestFile -Path (Join-Path $dir 'ticket.trex') -Text "shape ticket = ``[A-Z]{2,4}-\d{1,4}```n"
        $lib = New-TrexLibrary $dir
        $lib.Names | Should -Be @('ticket')
        (Select-TrexMatch '\{ticket}' -InputObject 'see AB-12' -Library $lib).Text | Should -BeExactly 'AB-12'
    }
}

Describe 'Libraries' {
    BeforeEach {
        Unregister-TrexAtom -All
    }

    It 'holds atoms apart from the session' {
        $lib = New-TrexLibrary -Declaration 'shape ticket = `[A-Z]{2,4}-\d{1,4}`'
        $lib.Names | Should -Be @('ticket')
        (Select-TrexMatch '\{ticket}' -InputObject 'see AB-12' -Library $lib).Text | Should -BeExactly 'AB-12'
        { Select-TrexMatch '\{ticket}' -InputObject 'see AB-12' -ErrorAction Stop } | Should -Throw '*unknown named atom*'
    }

    It 'takes declarations through Register-TrexAtom -Library' {
        $lib = [Trex.Library]::new()
        Register-TrexAtom num -Pattern '\N' -Library $lib
        $lib.Names | Should -Be @('num')
        Get-TrexAtom | Should -BeNullOrEmpty
    }

    It 'starts from a copy of the session with -FromSession' {
        Register-TrexAtom two -Pattern '\N'
        $lib = New-TrexLibrary -FromSession
        Unregister-TrexAtom -All
        $lib.Names | Should -Be @('two')
    }
}

Describe 'Files a pattern names' {
    BeforeAll {
        $here = Join-Path $TestDrive 'here'
        $elsewhere = Join-Path $TestDrive 'elsewhere'
        New-Item -ItemType Directory -Path $here, $elsewhere | Out-Null
        Write-TrexTestFile -Path (Join-Path $here 'cidrs.txt') -Text "10.0.0.0/8`n" -Encoding Utf8
        Write-TrexTestFile -Path (Join-Path $here 'billing.log') -Text "billing c91d ok`n" -Encoding Utf8
    }

    BeforeEach {
        Unregister-TrexAtom -All
    }

    It 'reads a relative @file from the PowerShell location, not the process directory' {
        [System.Environment]::CurrentDirectory | Should -Not -Be $here
        Push-Location -LiteralPath $here
        try {
            Select-TrexMatch '\I{in:@cidrs.txt}' -InputObject 'from 10.4.5.6 and 8.8.8.8' -Raw | Should -BeExactly '10.4.5.6'
            Select-TrexMatch '@echoed:@billing.log \W' -InputObject 'req fa3b; req c91d' -Raw | Should -BeExactly 'c91d'
            (New-TrexPattern '\I{in:@cidrs.txt}').FindAll('8.8.8.8 10.1.1.1').Text | Should -BeExactly '10.1.1.1'
        } finally {
            Pop-Location
        }
    }

    It 'reads a declared line''s @file from where it was declared, after the session changes directory' {
        Push-Location -LiteralPath $here
        try {
            Register-TrexAtom lan -Pattern '\I{in:@cidrs.txt}'
            $lib = New-TrexLibrary -Declaration 'let lan = \I{in:@cidrs.txt}'
        } finally {
            Pop-Location
        }
        Push-Location -LiteralPath $elsewhere
        try {
            Register-TrexAtom other -Pattern '\N'
            Select-TrexMatch '\{lan}' -InputObject 'from 10.4.5.6 and 8.8.8.8' -Raw | Should -BeExactly '10.4.5.6'
            $lib.Declare('let lan2 = \I{in:@cidrs.txt}')
            Select-TrexMatch '\{lan2}' -InputObject 'from 10.4.5.6 and 8.8.8.8' -Library $lib -Raw | Should -BeExactly '10.4.5.6'
        } finally {
            Pop-Location
        }
    }
}

Describe 'Shipped atoms' {
    It 'lists the library trex ships' {
        @(Get-TrexAtom -Shipped).Count | Should -Be 75
        (Get-TrexAtom iban -Shipped).Form | Should -Be ([Trex.AtomForm]::Kind)
    }

    It 'passes its own test lines' {
        Test-TrexAtom -Shipped -Quiet | Should -BeTrue
    }

    It 'reads a shipped atom with no declaration' {
        (Select-TrexMatch '\{iban}' -InputObject 'pay GB82 WEST 1234 5698 7654 32 now').Text | Should -BeExactly 'GB82 WEST 1234 5698 7654 32'
    }
}
