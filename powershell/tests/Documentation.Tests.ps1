# Every PowerShell example the README and the wiki show, run against the
# module this build produced and compared with the output the page shows,
# as tests/wiki_examples.rs holds the console examples to the trex binary.
#
# The pages show what pwsh 7 writes on Windows, so the examples run there;
# Windows PowerShell 5.1 formats tables and truncation differently.

BeforeDiscovery {
    $repo = [System.IO.Path]::GetFullPath((Join-Path (Join-Path $PSScriptRoot '..') '..'))
    $docs = Join-Path (Join-Path (Join-Path $repo 'wiki') 'content') 'docs'
    $candidates = @(Join-Path $repo 'README.md') + @(Get-ChildItem -LiteralPath $docs -Recurse -Filter '*.md' | ForEach-Object { $_.FullName })
    $pages = @(foreach ($candidate in $candidates) {
            if (Select-String -LiteralPath $candidate -Pattern '^PS> ' -CaseSensitive -Quiet) {
                @{ Page = $candidate.Substring($repo.Length + 1); Full = $candidate }
            }
        })
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    . (Join-Path $PSScriptRoot 'Documentation.ps1')
    Import-TrexModule

    # The session as a new PowerShell process has it: no atoms declared, the
    # system clock, UTC, and slash dates read day first.
    function Reset-TrexDocumentedSession {
        Unregister-TrexAtom -All -Confirm:$false
        Set-TrexClock -SystemClock -TimeZoneOffset 0 -DateOrder DayFirst -Confirm:$false
    }
}

Describe 'Documented examples' -Skip:(-not ($PSVersionTable.PSEdition -eq 'Core' -and $IsWindows)) {
    It 'finds a tabbed page among the pages it runs' {
        $repo = [System.IO.Path]::GetFullPath((Join-Path (Join-Path $PSScriptRoot '..') '..'))
        $tabbed = Join-Path $repo 'wiki/content/docs/reference/matching.md'
        @(Read-TrexDocumentedPage -Path $tabbed | Where-Object Kind -eq 'Example').Count | Should -BeGreaterThan 0
    }

    It 'reads every PS> line of <Page> as a command' -ForEach $pages {
        $read = @(Read-TrexDocumentedPage -Path $Full | ForEach-Object { $_.Line })
        $prompts = @(Select-String -LiteralPath $Full -Pattern '^PS> ' -CaseSensitive | ForEach-Object { $_.LineNumber })
        $unread = @($prompts | Where-Object { $read -notcontains $_ })
        $unread -join ', ' | Should -BeNullOrEmpty -Because 'a PS> line outside a block that begins with one is an example nothing runs'
    }

    It 'shows what <Page> runs' -ForEach $pages {
        Reset-TrexDocumentedSession
        try {
            $root = Join-Path $TestDrive ([System.Guid]::NewGuid().ToString('N'))
            $failures = @(Invoke-TrexDocumentedPage -Path $Full -Root $root)
            $failures -join "`n`n" | Should -BeNullOrEmpty
        } finally {
            Reset-TrexDocumentedSession
        }
    }
}
