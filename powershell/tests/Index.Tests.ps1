BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Indexes' {
    BeforeEach {
        $tree = Join-Path $TestDrive ([guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $tree | Out-Null
        Write-TrexTestFile -Path (Join-Path $tree 'words.txt') -Text "alpha beta gamma`n"
        Write-TrexTestFile -Path (Join-Path $tree 'mail.txt') -Text "write to bob@x.com`n"
    }

    It 'writes a tree''s index and reads it back' {
        $info = New-TrexIndex $tree
        $info.Files | Should -Be 2
        Test-Path -LiteralPath $info.Path | Should -BeTrue
        (Get-TrexIndex $tree).Files | Should -Be 2
    }

    It 'finds the same matches with an index as without one' {
        $without = Select-TrexMatch '\E' -Path $tree -NoIndex
        New-TrexIndex $tree | Out-Null
        $with = Select-TrexMatch '\E' -Path $tree
        $with.Text | Should -Be $without.Text
        $with.Path | Should -BeLike '*mail.txt'
    }

    It 'lists a file the index would skip under -FilesWithoutMatch' {
        New-TrexIndex $tree | Out-Null
        Select-TrexMatch '\E' -Path $tree -FilesWithoutMatch | Should -BeLike '*words.txt'
    }

    It 'writes the index while it scans with -Index' {
        Select-TrexMatch '\E' -Path $tree -Index | Out-Null
        (Get-TrexIndex $tree).Files | Should -Be 2
    }

    It 'scans and writes no index under -Index -WhatIf' {
        @(Select-TrexMatch '\E' -Path $tree -Index -WhatIf).Text | Should -Be @('bob@x.com')
        { Get-TrexIndex $tree -ErrorAction Stop } | Should -Throw '*holds no index*'
    }

    It 'reports a tree with no index as an error' {
        { Get-TrexIndex $tree -ErrorAction Stop } | Should -Throw '*holds no index*'
    }

    It 'refuses a path that is not a directory' {
        { New-TrexIndex (Join-Path $tree 'words.txt') -ErrorAction Stop } | Should -Throw '*not a directory*'
    }
}
