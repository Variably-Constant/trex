BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Rules' {
    BeforeAll {
        $ruleFile = Join-Path $TestDrive 'app.trex'
        $lines = @(
            'rule cardnum error "card number ending ${card:last4}" = \{card}:card'
            'fix cardnum = ****'
            'meta cardnum tags = pii'
            'rule legacy note "a legacy call" = "legacy"'
        )
        Write-TrexTestFile -Path $ruleFile -Text (($lines -join "`n") + "`n")
        $conf = Join-Path $TestDrive 'app.conf'
        $confText = "host = 10.0.0.5`npay 4111 1111 1111 1111 now`n# legacy rotate`n"
    }

    BeforeEach {
        Unregister-TrexAtom -All
        Write-TrexTestFile -Path $conf -Text $confText
    }

    It 'lists the rules a file declares' {
        $rules = Get-TrexRule -RuleFile $ruleFile
        $rules.Name | Should -Be @('cardnum', 'legacy')
        $rules[0].Severity | Should -Be 'Error'
        $rules[0].Fix | Should -BeExactly '****'
        $rules[0].Tags | Should -Be @('pii')
    }

    It 'writes each finding with its rule, message, place and fix' {
        $found = Invoke-TrexRule -Path $conf -RuleFile $ruleFile
        @($found).Count | Should -Be 2
        $found[0].Rule | Should -BeExactly 'cardnum'
        $found[0].Severity | Should -Be 'Error'
        $found[0].Message | Should -BeExactly 'card number ending 1111'
        $found[0].Region.Line | Should -Be 2
        $found[0].Region.Column | Should -Be 5
        $found[0].Fix | Should -BeExactly '****'
        $found[1].Rule | Should -BeExactly 'legacy'
        $found[1].Region.Line | Should -Be 3
        $found[1].Fix | Should -BeNullOrEmpty
    }

    It 'counts findings, names the files with and without one, and requires one, as the command line does' {
        $clean = Join-Path $TestDrive 'clean.conf'
        Write-TrexTestFile -Path $clean -Text "nothing here`n"
        Invoke-TrexRule -Path $conf, $clean -RuleFile $ruleFile -Count |
            ForEach-Object { '{0}={1}' -f (Split-Path -Leaf $_.Path), $_.Count } |
            Should -Be @('app.conf=2', 'clean.conf=0')
        Invoke-TrexRule -Path $conf, $clean -RuleFile $ruleFile -FilesWithMatches | Split-Path -Leaf | Should -Be @('app.conf')
        Invoke-TrexRule -Path $conf, $clean -RuleFile $ruleFile -FilesWithoutMatch | Split-Path -Leaf | Should -Be @('clean.conf')
        ('pay 4111 1111 1111 1111', 'nothing' | Invoke-TrexRule -RuleFile $ruleFile -Count).Count | Should -Be 1
        { 'nothing' | Invoke-TrexRule -RuleFile $ruleFile -RequireMatch -ErrorAction Stop } | Should -Throw '*no input holds a finding*'
        { Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Count -Sarif -ErrorAction Stop } | Should -Throw '*give one*'
        { Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Fix -RequireMatch -ErrorAction Stop } | Should -Throw '*takes no -Fix or -Diff*'
    }

    It 'reads a rule file the session already imported in place of that import' {
        Import-TrexAtom $ruleFile
        @(Get-TrexRule -RuleFile $ruleFile).Name | Should -Be @('cardnum', 'legacy')
        @(Invoke-TrexRule -Path $conf -RuleFile $ruleFile).Rule | Should -Be @('cardnum', 'legacy')
        (Get-TrexAtom).Name | Should -Be @('cardnum', 'legacy')
        $session = New-TrexLibrary -FromSession
        @(Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Library $session).Count | Should -Be 2
    }

    It 'scans a string piped in' {
        $found = 'pay 4111 1111 1111 1111' | Invoke-TrexRule -RuleFile $ruleFile
        @($found).Rule | Should -Be @('cardnum')
        $found.Path | Should -BeNullOrEmpty
    }

    It 'reads rules declared for the session' {
        Import-TrexAtom $ruleFile
        @(Invoke-TrexRule -Path $conf).Count | Should -Be 2
    }

    It 'writes the diff the fixes would make with -Diff, and changes nothing' {
        $diff = Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Diff
        $diff | Should -Match '\+pay \*\*\*\* now'
        Get-Content -Raw $conf | Should -BeExactly $confText
    }

    It 'writes the fixes into the file with -Fix, and not under -WhatIf' {
        Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Fix -WhatIf
        Get-Content -Raw $conf | Should -BeExactly $confText
        Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Fix
        Get-Content -Raw $conf | Should -Match 'pay \*\*\*\* now'
    }

    It 'puts each fix to the person with -Fix -Interactive, writing nothing where the host cannot ask' {
        { Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Fix -Interactive -ErrorAction Stop } | Should -Throw '*NonInteractive*'
        Get-Content -Raw $conf | Should -BeExactly $confText
        { Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Interactive -ErrorAction Stop } | Should -Throw '*takes -Fix*'
    }

    It 'writes one SARIF document with -Sarif' {
        $sarif = Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Sarif | ConvertFrom-Json
        $sarif.version | Should -Be '2.1.0'
        $sarif.runs[0].tool.driver.rules.id | Should -Be @('cardnum', 'legacy')
        $sarif.runs[0].results.ruleId | Should -Be @('cardnum', 'legacy')
        $sarif.runs[0].results[0].level | Should -Be 'error'
    }

    It 'renders findings piped to ConvertTo-TrexSarif' {
        $sarif = Invoke-TrexRule -Path $conf -RuleFile $ruleFile | ConvertTo-TrexSarif | ConvertFrom-Json
        $sarif.runs[0].results.ruleId | Should -Be @('cardnum', 'legacy')
        $sarif.runs[0].results[0].fixes[0].artifactChanges[0].replacements[0].insertedContent.text | Should -BeExactly '****'
        $sarif.runs[0].tool.driver.rules[0].properties.tags | Should -Be @('pii')
    }

    It 'writes GitHub annotations with -GitHub' {
        $lines = Invoke-TrexRule -Path $conf -RuleFile $ruleFile -GitHub
        $lines[0] | Should -BeLike '::error file=*,line=2,col=5,*title=cardnum::card number ending 1111'
        $lines[1] | Should -BeLike '::notice *'
    }

    It 'renders a report template at each finding with -Format' {
        Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Format '${rule}:${severity}:${line}' |
            Should -Be @('cardnum:error:2', 'legacy:note:3')
    }

    It 'refuses a call with no rule declared' {
        { Invoke-TrexRule -Path $conf -ErrorAction Stop } | Should -Throw '*no rule is declared*'
    }
}
