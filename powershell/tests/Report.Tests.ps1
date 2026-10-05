BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule

    # The logs every suite here reads: a.log with two addresses and an IP,
    # b.log with none, c.log with a duration.
    function New-TrexReportLogs([string] $Root) {
        New-Item -ItemType Directory -Path $Root | Out-Null
        Write-TrexTestFile -Path (Join-Path $Root 'a.log') -Text "ping bob@x.com at 10.0.0.5`nidle`nmail ann@y.org`n"
        Write-TrexTestFile -Path (Join-Path $Root 'b.log') -Text "no mail here`n"
        Write-TrexTestFile -Path (Join-Path $Root 'c.log') -Text "took 1500ms`n"
    }

    # A pattern file declaring two rules, the first with a fix and a tag.
    function New-TrexReportRules([string] $Path) {
        $lines = @(
            'rule cardnum error "card number ending ${card:last4}" = \{card}:card'
            'fix cardnum = ****'
            'meta cardnum tags = pii'
            'rule legacy note "a legacy call" = "legacy"'
        )
        Write-TrexTestFile -Path $Path -Text (($lines -join "`n") + "`n")
    }
}

Describe 'Reports written as the trex command writes them' {
    BeforeAll {
        Unregister-TrexAtom -All
        $root = Join-Path $TestDrive 'logs'
        New-TrexReportLogs $root
        $a = Join-Path $root 'a.log'
        $b = Join-Path $root 'b.log'
        $ja = $a.Replace('\', '\\')
        $esc = [string][char]27
    }

    It 'writes -Json for a string as the one-input array, strings piped in as the command writes its standard input, and each under -PerString as its array' {
        Select-TrexMatch '\E' -InputObject 'ping bob@x.com' -Json |
            Should -BeExactly '[{"start":5,"end":14,"text":"bob@x.com","captures":{}}]'
        $lines = 'ping bob@x.com', 'idle', 'mail ann@y.org'
        $piped = @($lines | Select-TrexMatch '\E' -Json)
        $piped.Count | Should -Be 2
        $piped -join "`n" | Should -BeExactly ((Invoke-TrexCliInput -Arguments 'scan \E --json' -Text (($lines -join "`n") + "`n")) -join "`n")
        @($lines | Select-TrexMatch '\E' -Json -PerString) -join "`n" |
            Should -BeExactly ('[{"start":5,"end":14,"text":"bob@x.com","captures":{}}]' + "`n" + '[]' + "`n" + '[{"start":5,"end":14,"text":"ann@y.org","captures":{}}]')
    }

    It 'writes -Json over several files as one array naming each path, line and column' {
        $first = '{"path":"' + $ja + '","line":1,"col":6,"start":5,"end":14,"text":"bob@x.com","captures":{}}'
        $second = '{"path":"' + $ja + '","line":3,"col":6,"start":37,"end":46,"text":"ann@y.org","captures":{}}'
        @(Select-TrexMatch '\E' -Path $a, $b -Json) -join "`n" | Should -BeExactly ('[' + $first + ',' + $second + ']')
    }

    It 'writes one -Json array for paths piped in one at a time' {
        $out = @(Get-ChildItem -LiteralPath $root -Filter '*.log' | Select-TrexMatch '\E' -Json)
        $out.Count | Should -Be 1
        $out[0].StartsWith('[{"path":"') | Should -BeTrue
        $out[0].EndsWith('}]') | Should -BeTrue
    }

    It 'writes the -Color report of one input as the span and quoted text of each match' {
        Select-TrexMatch '\E' -InputObject 'ping bob@x.com' -Color -ColorDepth None | Should -BeExactly '[5..14] "bob@x.com"'
        Select-TrexMatch '\E' -InputObject 'idle' -Color -ColorDepth None | Should -BeExactly 'no match'
    }

    It 'writes the -Color report over several files as path, line and column' {
        @(Select-TrexMatch '\E' -Path $a, $b -Color -ColorDepth None) -join "`n" |
            Should -BeExactly ("${a}:1:6: `"bob@x.com`"" + "`n" + "${a}:3:6: `"ann@y.org`"")
    }

    It 'paints the -Color report at the depth asked' {
        $painted = Select-TrexMatch '\E' -InputObject 'ping bob@x.com' -Color -ColorDepth Ansi16
        $painted.Contains($esc + '[') | Should -BeTrue
        $painted.Contains('bob@x.com') | Should -BeTrue
    }

    It 'writes every line with -Passthru, joined to its path by : where it matched and by - where not' {
        $expected = @(
            "${a}:1:ping bob@x.com at 10.0.0.5"
            "${a}-2-idle"
            "${a}:3:mail ann@y.org"
            "${b}-1-no mail here"
        )
        @(Select-TrexMatch '\E' -Path $a, $b -Passthru -ColorDepth None) -join "`n" | Should -BeExactly ($expected -join "`n")
        @(Select-TrexMatch '\E' -Path $a -Passthru -ColorDepth None) -join "`n" |
            Should -BeExactly ("ping bob@x.com at 10.0.0.5`nidle`nmail ann@y.org")
    }

    It 'paints the matches -Passthru writes unless -ColorDepth None' {
        (@(Select-TrexMatch '\E' -Path $a -Passthru) -join '').Contains($esc + '[') | Should -BeTrue
        (@(Select-TrexMatch '\E' -Path $a -Passthru -ColorDepth None) -join '').Contains($esc) | Should -BeFalse
    }

    It 'writes the lines no match touches with -NotMatch -Color' {
        Select-TrexMatch '\E' -Path $a -NotMatch -Color -ColorDepth None | Should -BeExactly 'idle'
    }

    It 'answers each idiom the reference names as the switch beside it' {
        $leaf = { '{0}={1}' -f (Split-Path -Leaf $_.Path), $_.Count }
        $switch = @(Select-TrexMatch '\E' -Path $root -Count | ForEach-Object $leaf) -join ','
        $idiom = @(Select-TrexMatch '\E' -Path $root | Select-Object Path, LineNumber -Unique | Group-Object Path -NoElement |
            ForEach-Object { '{0}={1}' -f (Split-Path -Leaf $_.Name), $_.Count }) -join ','
        $idiom | Should -BeExactly $switch
        $switch | Should -BeExactly 'a.log=2'

        $switch = @(Select-TrexMatch '\W' -Path $root -CountMatches | ForEach-Object $leaf) -join ','
        $idiom = @(Select-TrexMatch '\W' -Path $root | Group-Object Path -NoElement |
            ForEach-Object { '{0}={1}' -f (Split-Path -Leaf $_.Name), $_.Count }) -join ','
        $idiom | Should -BeExactly $switch

        $switch = @(Select-TrexMatch '\E' -Path $root -FilesWithMatches) -join ','
        $idiom = @(Select-TrexMatch '\E' -Path $root -List | ForEach-Object Path) -join ','
        $idiom | Should -BeExactly $switch

        $switch = @(Select-TrexMatch '\E' -Path $root -FilesWithoutMatch) -join ','
        $idiom = @(Get-TrexFile $root | Where-Object { -not (Test-TrexMatch '\E' -Path $_.Path) } | ForEach-Object Path) -join ','
        $idiom | Should -BeExactly $switch
        @($switch -split ',' | Split-Path -Leaf) | Should -Be @('b.log', 'c.log')

        Test-TrexMatch '\E' -Path $root | Should -Be (Select-TrexMatch '\E' -Path $root -Quiet)

        (Get-Item -LiteralPath (Join-Path $root 'c.log')).LastWriteTime = [datetime]::new(2026, 1, 1)
        (Get-Item -LiteralPath $a).LastWriteTime = [datetime]::new(2026, 2, 1)
        (Get-Item -LiteralPath $b).LastWriteTime = [datetime]::new(2026, 3, 1)
        $switch = @(Select-TrexMatch '\W' -Path $root -Sort Modified -FilesWithMatches | Split-Path -Leaf) -join ','
        $idiom = @(Get-TrexFile $root | Get-Item | Sort-Object LastWriteTime | Select-TrexMatch '\W' -FilesWithMatches |
            Split-Path -Leaf) -join ','
        $idiom | Should -BeExactly $switch
        $switch | Should -BeExactly 'c.log,a.log,b.log'
    }

    It 'refuses switches that say two things about what is written' {
        { Select-TrexMatch '\E' -InputObject 'x' -Json -Raw -ErrorAction Stop } | Should -Throw '*give one*'
        { Select-TrexMatch '\E' -InputObject 'x' -NotMatch -Json -ErrorAction Stop } | Should -Throw '*takes no -Json or -Passthru*'
        { Select-TrexMatch '\E' -InputObject 'x' -ColorDepth Ansi16 -ErrorAction Stop } |
            Should -Throw '*paint what -Color and -Passthru write*'
        { Select-TrexMatch '\E' -InputObject 'x' -ValueSpelling Natural -ErrorAction Stop } | Should -Throw '*give -Json*'
        { Select-TrexMatch '\E' -InputObject 'x' -Passthru -Explain -ErrorAction Stop } | Should -Throw '*-Explain says*'
        { Select-TrexMatch '\E' -InputObject 'x' -Json -Context 1 -ErrorAction Stop } | Should -Throw '*-Context adds lines*'
    }
}

Describe 'Rule findings written as the trex command writes them' {
    BeforeAll {
        Unregister-TrexAtom -All
        $ruleFile = Join-Path $TestDrive 'app.trex'
        New-TrexReportRules $ruleFile
        $conf = Join-Path $TestDrive 'app.conf'
        Write-TrexTestFile -Path $conf -Text "host = 10.0.0.5`npay 4111 1111 1111 1111 now`n# legacy rotate`n"
    }

    It 'writes -Json as one array of every finding' {
        $json = @(Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Json)
        $json.Count | Should -Be 1
        $parsed = ConvertFrom-Json -InputObject $json[0]
        $found = @($parsed)
        $found.rule | Should -Be @('cardnum', 'legacy')
        $found[0].severity | Should -BeExactly 'error'
        $found[0].message | Should -BeExactly 'card number ending 1111'
        $found[0].path | Should -BeExactly $conf
        $found[0].line | Should -Be 2
        $found[0].col | Should -Be 5
        $found[0].fix | Should -BeExactly '****'
        $found[0].meta.tags | Should -BeExactly 'pii'
    }

    It 'writes -Json for text with no path' {
        $json = 'pay 4111 1111 1111 1111' | Invoke-TrexRule -RuleFile $ruleFile -Json
        $json.StartsWith('[{"rule":"cardnum","severity":"error","message":"card number ending 1111","line":1,"col":5,') | Should -BeTrue
        $json.Contains('"path"') | Should -BeFalse
        'nothing' | Invoke-TrexRule -RuleFile $ruleFile -Json | Should -BeExactly '[]'
    }

    It 'refuses -Json beside another way of writing findings' {
        { Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Json -Sarif -ErrorAction Stop } | Should -Throw '*give one*'
        { Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Json -Fix -ErrorAction Stop } | Should -Throw '*take no -Json*'
    }
}

Describe 'The same bytes the trex command prints' {
    BeforeAll {
        Unregister-TrexAtom -All
        $cli = Get-TrexCli
        $root = Join-Path $TestDrive 'logs'
        New-TrexReportLogs $root
        $a = Join-Path $root 'a.log'
        $b = Join-Path $root 'b.log'
        $c = Join-Path $root 'c.log'
        $ruleFile = Join-Path $TestDrive 'app.trex'
        New-TrexReportRules $ruleFile
        $conf = Join-Path $TestDrive 'app.conf'
        Write-TrexTestFile -Path $conf -Text "host = 10.0.0.5`npay 4111 1111 1111 1111 now`n# legacy rotate`n"

        # The command run with -Arguments, and the module's lines from
        # -Module, read as one text each.
        function Assert-TrexSameAsCli([string[]] $Arguments, [scriptblock] $Module) {
            $expected = @(& $cli @Arguments)
            $expected.Count | Should -BeGreaterThan 0 -Because "trex $($Arguments -join ' ') printed nothing to compare with"
            $actual = @(& $Module)
            ($actual -join "`n") | Should -BeExactly ($expected -join "`n") -Because "trex $($Arguments -join ' ')"
        }
    }

    It 'writes -Json as --json over one file, two, and a directory' {
        Assert-TrexSameAsCli @('scan', '\E', $a, '--json') { Select-TrexMatch '\E' -Path $a -Json }
        Assert-TrexSameAsCli @('scan', '\E', $a, $b, '--json') { Select-TrexMatch '\E' -Path $a, $b -Json }
        Assert-TrexSameAsCli @('scan', '\E', $root, '--json') { Select-TrexMatch '\E' -Path $root -Json }
    }

    It 'writes typed values as --values and --duration-unit spell them' {
        Assert-TrexSameAsCli @('scan', '\R:d', $c, '--json', '--values', 'tagged', '--duration-unit', 's') {
            Select-TrexMatch '\R:d' -Path $c -Json -ValueSpelling Tagged -DurationUnit Seconds
        }
        Assert-TrexSameAsCli @('scan', '\R:d', $c, '--json', '--values', 'natural', '--duration-unit', 'ms') {
            Select-TrexMatch '\R:d' -Path $c -Json -ValueSpelling Natural -DurationUnit Milliseconds
        }
    }

    It 'writes the -Color report as --color paints it, at each depth' {
        Assert-TrexSameAsCli @('scan', '\E', $a, '--color', '16') { Select-TrexMatch '\E' -Path $a -Color -ColorDepth Ansi16 }
        Assert-TrexSameAsCli @('scan', '\E', $a, '--color', 'always') { Select-TrexMatch '\E' -Path $a -Color }
        Assert-TrexSameAsCli @('scan', '\E', $a, '--color', '16', '--colors', 'match:fg:red') {
            Select-TrexMatch '\E' -Path $a -Color -ColorDepth Ansi16 -Colors 'match:fg:red'
        }
        Assert-TrexSameAsCli @('scan', '\E', $root, '-C', '1', '--color', '256') {
            Select-TrexMatch '\E' -Path $root -Context 1 -Color -ColorDepth Ansi256
        }
    }

    It 'writes -Passthru and -NotMatch -Color as --passthru and -v print them' {
        Assert-TrexSameAsCli @('scan', '\E', $root, '--passthru', '--color', 'truecolor') {
            Select-TrexMatch '\E' -Path $root -Passthru -Color -ColorDepth TrueColor
        }
        Assert-TrexSameAsCli @('scan', '\E', $a, '--passthru', '--color', 'always') {
            Select-TrexMatch '\E' -Path $a -Passthru
        }
        Assert-TrexSameAsCli @('scan', '\E', $a, '--passthru', '-m', '1', '--color', 'never') {
            Select-TrexMatch '\E' -Path $a -Passthru -MaxCount 1 -ColorDepth None
        }
        Assert-TrexSameAsCli @('scan', '\E', $a, '-v', '--color', '16') {
            Select-TrexMatch '\E' -Path $a -NotMatch -Color -ColorDepth Ansi16
        }
    }

    It 'writes -Explain under each match as --explain does' {
        Assert-TrexSameAsCli @('scan', '\E', $a, '--explain', '--color', 'never') {
            Select-TrexMatch '\E' -Path $a -Explain -Color -ColorDepth None
        }
        Assert-TrexSameAsCli @('scan', '\E', $a, '--explain', '--json') { Select-TrexMatch '\E' -Path $a -Explain -Json }
    }

    It 'writes Invoke-TrexRule -Json as scan --rules --json' {
        Assert-TrexSameAsCli @('scan', '--rules', $ruleFile, '--json', $conf) {
            Invoke-TrexRule -Path $conf -RuleFile $ruleFile -Json
        }
    }
}
