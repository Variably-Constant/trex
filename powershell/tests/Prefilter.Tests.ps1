BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Presence prefilters' {
    BeforeAll {
        $corpus = "GET /index.html 200`nPOST /login 302`nGET /missing 404`n"
    }

    It 'says a literal that occurs might occur, and settles each answer with an exact search' {
        $tests = Test-TrexPrefilter -InputObject $corpus -Literal 'login', 'teapot'
        $tests | Should -BeOfType ([Trex.LiteralTest])
        $tests[0].MightOccur | Should -BeTrue
        $tests[0].Occurs | Should -BeTrue
        $tests[1].Occurs | Should -BeFalse
    }

    It 'never rejects a literal shorter than one four-byte n-gram' {
        $short = Test-TrexPrefilter -InputObject $corpus -Literal 'zz'
        $short.MightOccur | Should -BeTrue
        $short.Occurs | Should -BeFalse
    }

    It 'finds no false negative in any filter with -Verify' {
        $checks = Test-TrexPrefilter -InputObject ($corpus * 20) -Verify
        $checks | Should -BeOfType ([Trex.FilterCheck])
        $checks.Filter | Should -Be @([Trex.FilterKind]::Bloom, [Trex.FilterKind]::Cuckoo, [Trex.FilterKind]::Xor)
        $checks | ForEach-Object FalseNegatives | Should -Be @(0, 0, 0)
        $checks | ForEach-Object AbsentProbes | Should -Be @(500, 500, 500)
    }

    It 'builds a filter whose MightContain answers without the corpus' {
        $seen = New-TrexPrefilter -InputObject $corpus -Filter Xor
        $seen | Should -BeOfType ([Trex.Prefilter])
        $seen.Filter | Should -Be ([Trex.FilterKind]::Xor)
        $seen.Bytes | Should -Be ([Text.Encoding]::UTF8.GetByteCount($corpus))
        $seen.MightContain('login') | Should -BeTrue
        [Trex.Prefilter]::new('abc def').MightContain('abc ') | Should -BeTrue
    }

    It 'refuses a test that names neither -Literal nor -Verify' {
        { Test-TrexPrefilter -InputObject $corpus -ErrorAction Stop } | Should -Throw '*give either*'
    }
}
