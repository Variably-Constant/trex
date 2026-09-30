BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Find-TrexRecord' {
    BeforeAll {
        $text = "error disk full`nwarn disk slow`nerror net down`ninfo ok"
    }

    It 'keeps the lines holding every pattern with -All' {
        $hits = Find-TrexRecord '"error"', '"disk"' -InputObject $text -All
        @($hits).Text | Should -Be @('error disk full')
        $hits.LineNumber | Should -Be 1
        $hits.Patterns | Should -Be @('"error"', '"disk"')
    }

    It 'keeps the lines holding any pattern when no rule is named' {
        (Find-TrexRecord '"error"', '"warn"' -InputObject $text).LineNumber | Should -Be @(1, 2, 3)
    }

    It 'keeps the lines holding none with -None' {
        @(Find-TrexRecord '"error"', '"warn"' -InputObject $text -None).Text | Should -Be @('info ok')
    }

    It 'keeps the lines holding at least a count with -AtLeast' {
        $hits = Find-TrexRecord '"disk"', '"full"', '"slow"' -InputObject $text -AtLeast 2
        $hits.Text | Should -Be @('error disk full', 'warn disk slow')
    }

    It 'drops a record holding a pattern -Not names' {
        @(Find-TrexRecord '"disk"' -InputObject $text -Not '"slow"').Text | Should -Be @('error disk full')
    }

    It 'reads records as the unit -Unit names' {
        $paragraphs = "error a`nwarn b`n`ninfo c`nerror d`n"
        @(Find-TrexRecord '"error"', '"warn"' -InputObject $paragraphs -All -Unit paragraph).Count | Should -Be 1
    }

    It 'makes a record of each run between matches with -RecordStart' {
        $log = "2026-09-27T01:00:00Z start`n  detail`n2026-09-27T01:00:05Z error`n  more"
        $hits = Find-TrexRecord '"error"' -InputObject $log -RecordStart '\T'
        @($hits).Count | Should -Be 1
        $hits.Text | Should -BeLike '*error*more'
    }

    It 'refuses two rules' {
        { Find-TrexRecord '"a"' -InputObject 'a' -All -None -ErrorAction Stop } | Should -Throw '*two rules*'
    }

    It 'refuses two definitions of a record' {
        { Find-TrexRecord '"a"' -InputObject 'a' -Unit line -RecordSpan '\N' -ErrorAction Stop } | Should -Throw '*give one*'
    }
}
