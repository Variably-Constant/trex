BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Text piped in' {
    It '<Command> reads a string piped in as the text it is given by position' -TestCases @(
        @{ Command = 'Get-TrexToken'; Text = 'retry 3 times in 1500ms' }
        @{ Command = 'Get-TrexRecordShape'; Text = "a 1`na 2`na 3`nb x y`n" }
        @{ Command = 'ConvertTo-TrexLiteral'; Text = 'x (y)' }
        @{ Command = 'Measure-TrexMagnitude'; Text = '1 1000000 2' }
        @{ Command = 'Measure-TrexStress'; Text = '{"a": [1, {"b": [2, 3]}]}' }
        @{ Command = 'Measure-TrexFlow'; Text = '1 10 100 1000 10 1' }
        @{ Command = 'Measure-TrexEcho'; Text = 'alpha beta alpha gamma alpha' }
        @{ Command = 'Measure-TrexOrbit'; Text = 'Cat cat CAT dog' }
        @{ Command = 'Measure-TrexShape'; Text = "k=1 v=2`nk=3 v=4`nk=5 v=6`n" }
        @{ Command = 'Measure-TrexRelation'; Text = 'f(a, [b])' }
        @{ Command = 'Measure-TrexGravity'; Text = 'let x = f(a) ; let y = g(b) ; print x' }
        @{ Command = 'Measure-TrexContext'; Text = 'latency = 100 ; latency = 120 ; latency = 90 ;' }
    ) {
        param($Command, $Text)
        $piped = $Text | & $Command
        $given = & $Command $Text
        @($piped).Count | Should -BeGreaterThan 0
        ($piped | ConvertTo-Json -Depth 6 -Compress) | Should -BeExactly ($given | ConvertTo-Json -Depth 6 -Compress)
    }

    It 'Find-TrexRecord reads a string piped in as -InputObject' {
        $text = "error disk full`nwarn slow`ninfo ok"
        ($text | Find-TrexRecord '"error"', '"warn"').LineNumber | Should -Be @(1, 2)
    }

    It 'ConvertTo-TrexBpe splits each string piped in on its own' {
        $bpe = 'low low low lower' | New-TrexBpe -Merges 3
        $piped = 'low', 'lower' | ConvertTo-TrexBpe $bpe
        $piped | Should -Be (@(ConvertTo-TrexBpe $bpe 'low') + @(ConvertTo-TrexBpe $bpe 'lower'))
    }

    It 'New-TrexPrefilter and Test-TrexPrefilter read the strings piped in as one corpus' {
        $seen = 'GET /login', 'POST /logout' | New-TrexPrefilter
        $seen.Bytes | Should -Be ([Text.Encoding]::UTF8.GetByteCount("GET /login`nPOST /logout"))
        $seen.MightContain('logout') | Should -BeTrue
        ('GET /login', 'POST /logout' | Test-TrexPrefilter -Literal 'logout').Occurs | Should -BeTrue
    }
}

Describe 'Blank lines piped in' {
    BeforeAll {
        # A log as Get-Content yields it: three of its seven lines are blank.
        $script:LogLines = @('ERROR disk full', '', 'warn slow', '', '', 'ERROR again ERROR', 'info ok')
        $script:Bpe = 'low low low lower' | New-TrexBpe -Merges 3
        $script:Words = New-TrexGrammar 's := <s> ident | ident'
        $script:RuleFile = Join-Path $TestDrive 'blank.trex'
        Write-TrexTestFile -Path $script:RuleFile -Text "rule todo warning `"a todo left`" = `"TODO`"`n"
    }

    It 'declares every string -InputObject able to take an empty string' {
        $checked = @(Get-Command -Module Trex -CommandType Cmdlet | Where-Object {
                $_.Parameters.ContainsKey('InputObject') -and $_.Parameters['InputObject'].ParameterType -eq [string]
            })
        $refusing = @($checked | Where-Object {
                -not ($_.Parameters['InputObject'].Attributes | Where-Object { $_ -is [System.Management.Automation.AllowEmptyStringAttribute] })
            } | ForEach-Object Name)
        $checked.Count | Should -BeGreaterOrEqual 27
        $refusing | Should -BeNullOrEmpty
    }

    It '<Command> reads a blank line piped between two others' -TestCases @(
        @{ Command = 'ConvertFrom-TrexText'; Run = { $args[0] | ConvertFrom-TrexText '\W:w \N:n' } }
        @{ Command = 'ConvertTo-TrexPattern'; Run = { $args[0] | ConvertTo-TrexPattern -Marked '{w:x} {n:1}' } }
        @{ Command = 'ConvertTo-TrexBpe'; Run = { $args[0] | ConvertTo-TrexBpe $script:Bpe } }
        @{ Command = 'ConvertTo-TrexLiteral'; Run = { $args[0] | ConvertTo-TrexLiteral } }
        @{ Command = 'Edit-TrexText'; Run = { $args[0] | Edit-TrexText '\N' '<n>' } }
        @{ Command = 'Find-TrexRecord'; Run = { $args[0] | Find-TrexRecord '"x"' } }
        @{ Command = 'Get-TrexLine'; Run = { $args[0] | Get-TrexLine -Head 1 } }
        @{ Command = 'Get-TrexRecord'; Run = { $args[0] | Get-TrexRecord } }
        @{ Command = 'Get-TrexRecordShape'; Run = { $args[0] | Get-TrexRecordShape } }
        @{ Command = 'Get-TrexToken'; Run = { $args[0] | Get-TrexToken } }
        @{ Command = 'Group-TrexMatch'; Run = { $args[0] | Group-TrexMatch '\W:w' -Key '${w}' } }
        @{ Command = 'Invoke-TrexRule'; Run = { $args[0] | Invoke-TrexRule -RuleFile $script:RuleFile } }
        @{ Command = 'Measure-TrexContext'; Run = { $args[0] | Measure-TrexContext } }
        @{ Command = 'Measure-TrexEcho'; Run = { $args[0] | Measure-TrexEcho } }
        @{ Command = 'Measure-TrexFlow'; Run = { $args[0] | Measure-TrexFlow } }
        @{ Command = 'Measure-TrexGravity'; Run = { $args[0] | Measure-TrexGravity } }
        @{ Command = 'Measure-TrexMagnitude'; Run = { $args[0] | Measure-TrexMagnitude } }
        @{ Command = 'Measure-TrexObservation'; Run = { $args[0] | Measure-TrexObservation } }
        @{ Command = 'Measure-TrexOrbit'; Run = { $args[0] | Measure-TrexOrbit } }
        @{ Command = 'Measure-TrexRelation'; Run = { $args[0] | Measure-TrexRelation } }
        @{ Command = 'Measure-TrexSeam'; Run = { $args[0] | Measure-TrexSeam } }
        @{ Command = 'Measure-TrexShape'; Run = { $args[0] | Measure-TrexShape } }
        @{ Command = 'Measure-TrexSpectral'; Run = { $args[0] | Measure-TrexSpectral } }
        @{ Command = 'Measure-TrexStress'; Run = { $args[0] | Measure-TrexStress } }
        @{ Command = 'New-TrexBpe'; Run = { $args[0] | New-TrexBpe -Merges 2 } }
        @{ Command = 'New-TrexPrefilter'; Run = { $args[0] | New-TrexPrefilter } }
        @{ Command = 'Protect-TrexText'; Run = { $args[0] | Protect-TrexText '\N' } }
        @{ Command = 'Select-TrexMatch'; Run = { $args[0] | Select-TrexMatch '\N' } }
        @{ Command = 'Test-TrexMatch'; Run = { $args[0] | Test-TrexMatch '\N' } }
        @{ Command = 'Test-TrexPrefilter'; Run = { $args[0] | Test-TrexPrefilter -Literal 'x' } }
    ) {
        param($Command, $Run)
        { $ErrorActionPreference = 'Stop'; & $Run @('x 1', '', 'y 2') } | Should -Not -Throw
    }

    It 'Invoke-TrexGrammar parses a blank line, which the grammar refuses rather than the binder' {
        { '' | Invoke-TrexGrammar $script:Words -ErrorAction Stop } | Should -Throw '*does not parse in full*'
    }

    It 'Select-TrexMatch counts the matches of a log with blank lines as Select-String does' {
        # -SimpleMatch leaves a MatchInfo's Matches empty, so the reference is
        # the same literal as a pattern.
        $trex = ($LogLines | Select-TrexMatch '"ERROR"' -CountMatches).Count
        $trex | Should -Be (@($LogLines | Select-String 'ERROR' -CaseSensitive -AllMatches).Matches.Count)
        $trex | Should -Be 3
    }

    It 'Select-TrexMatch writes the blank lines no match touches with -NotMatch, as Select-String does' {
        $trex = @($LogLines | Select-TrexMatch '"ERROR"' -NotMatch)
        $string = @($LogLines | Select-String 'ERROR' -CaseSensitive -NotMatch)
        $trex.Text | Should -Be $string.Line
        ($LogLines | Select-TrexMatch '"ERROR"' -Count -NotMatch).Count | Should -Be 5
        ($LogLines | Select-TrexMatch '"ERROR"' -Count -NotMatch).Count | Should -Be $string.Count
        ('' | Select-TrexMatch '"ERROR"' -Count).Count | Should -Be 0
    }

    It 'reads an empty file as no lines and an empty string as one blank line' {
        $empty = Join-Path $TestDrive 'empty.txt'
        [System.IO.File]::WriteAllBytes($empty, [byte[]]@())
        @(Select-TrexMatch '"x"' -Path $empty -NotMatch).Count | Should -Be 0
        @(Select-TrexMatch '"x"' -Path $empty -NotMatch -Count).Count | Should -Be 0
        @(Get-TrexLine -Path $empty -Head 3).Count | Should -Be 0
        $blank = @(Select-TrexMatch '"x"' -InputObject '' -NotMatch)
        $blank.Count | Should -Be 1
        $blank[0].Text | Should -BeExactly ''
        @('' | Select-TrexMatch '"x"' -NotMatch).Count | Should -Be 1
        @('' | Get-TrexLine -Head 3).Count | Should -Be 1
        (Select-TrexMatch '"x"' -InputObject '' -NotMatch -Count).Count | Should -Be 1
    }

    It 'Edit-TrexText and Protect-TrexText write a blank line back as the blank line it was' {
        $edited = @($LogLines | Edit-TrexText '"ERROR"' 'E')
        $edited | Should -Be @('E disk full', '', 'warn slow', '', '', 'E again E', 'info ok')
        @($LogLines | Protect-TrexText '"ERROR"' -Mask '#') | Should -Be @('##### disk full', '', 'warn slow', '', '', '##### again #####', 'info ok')
    }
}

Describe 'Paths piped in' {
    It 'Get-TrexFile walks a directory piped in' {
        $tree = Join-Path $TestDrive 'piped'
        New-Item -ItemType Directory -Path $tree | Out-Null
        Write-TrexTestFile -Path (Join-Path $tree 'a.txt') -Text "x`n"
        ($tree | Get-TrexFile).Path | Should -BeLike '*a.txt'
    }
}

Describe 'Parameter names' {
    # A command's parameters are a dictionary, and a parameter named Keys or
    # Values would hide the dictionary's own Keys or Values from code that
    # lists them, as $PSBoundParameters.Values in a wrapper does.
    It 'names no parameter Keys or Values' {
        $hiding = foreach ($command in Get-Command -Module Trex -CommandType Cmdlet) {
            foreach ($name in $command.Parameters.get_Keys()) {
                if ($name -in 'Keys', 'Values') { '{0} -{1}' -f $command.Name, $name }
            }
        }
        @($hiding) | Should -BeNullOrEmpty
        $scan = Get-Command Select-TrexMatch
        @($scan.Parameters.Values).Count | Should -Be $scan.Parameters.get_Count()
    }
}
