BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Select-TrexMatch over text' {
    It 'writes a match with its span, text and registers' {
        $m = Select-TrexMatch '\E:e' -InputObject 'ping bob@x.com'
        $m | Should -BeOfType ([Trex.Match])
        $m.Start | Should -Be 5
        $m.Length | Should -Be 9
        $m.Text | Should -BeExactly 'bob@x.com'
        $m.Captures.e | Should -BeExactly 'bob@x.com'
        $m.Groups[0].Name | Should -BeExactly 'e'
        $m.Groups[0].Kind | Should -BeExactly 'email'
    }

    It 'reads the strings piped in as the lines of one input, and each on its own under -PerString' {
        $lines = 'ping bob@x.com', 'nothing here', 'and ann@y.org'
        $found = $lines | Select-TrexMatch '\E'
        @($found).Count | Should -Be 2
        $found[1].Text | Should -BeExactly 'ann@y.org'
        $found.LineNumber | Should -Be @(1, 3)
        $found.Start | Should -Be @(5, 32)
        $whole = Select-TrexMatch '\E' -InputObject (($lines -join "`n") + "`n")
        $found.Start | Should -Be $whole.Start
        $found.LineNumber | Should -Be $whole.LineNumber
        $each = $lines | Select-TrexMatch '\E' -PerString
        $each.Text | Should -Be $found.Text
        $each.LineNumber | Should -Be @(1, 1)
        $each.Start | Should -Be @(5, 4)
    }

    It 'numbers the lines of strings piped in as Select-String numbers them' {
        $lines = 'a 1', 'no digits', 'b 2', '', 'c 3'
        ($lines | Select-TrexMatch '\N').LineNumber | Should -Be @(($lines | Select-String '\d').LineNumber)
    }

    It 'finds a match that runs from one string piped in into the next' {
        $found = 'call(a,', 'b)', 'next(c)' | Select-TrexMatch '\W\B(.*)'
        $found.Text | Should -Be @("call(a,`nb)", 'next(c)')
        $found.LineNumber | Should -Be @(1, 3)
        ('call(a,', 'b)', 'next(c)' | Select-TrexMatch '\W\B(.*)' -PerString).Text | Should -Be @('next(c)')
    }

    It 'treats the strings piped in as lines for -Context and -NotMatch' {
        $m = 'x', 'y 1', 'z' | Select-TrexMatch '\N' -Context 1
        $m.PreContext | Should -Be @('x')
        $m.PostContext | Should -Be @('z')
        ('x', 'y 1', 'z' | Select-TrexMatch '\N' -NotMatch).LineNumber | Should -Be @(1, 3)
    }

    It 'reads strings piped in as -Path reads a file holding the same lines' {
        $lines = 'alpha 10', 'beta 2.5', 'gamma', 'delta 4000'
        $file = Join-Path $TestDrive 'lines.txt'
        Write-TrexTestFile -Path $file -Text (($lines -join "`n") + "`n")
        foreach ($asked in @{}, @{ MaxCount = 2 }, @{ Tail = 2 }) {
            $piped = @($lines | Select-TrexMatch '\N' @asked)
            $read = @(Select-TrexMatch '\N' -Path $file @asked)
            $piped.Text | Should -Be $read.Text
            $piped.Start | Should -Be $read.Start
            $piped.LineNumber | Should -Be $read.LineNumber
        }
    }

    It 'requires a closing tag equal to the opening one' {
        $m = Select-TrexMatch '<\W:t>.*</=t>' -InputObject '<div>hi</div> <p>x</q>'
        @($m).Count | Should -Be 1
        $m.Captures.t | Should -BeExactly 'div'
    }

    It 'reports offsets in UTF-16 code units, so Substring is the match' {
        $text = 'é 𝄞 at bob@x.com'
        $m = Select-TrexMatch '\E' -InputObject $text
        $text.Substring($m.Start, $m.Length) | Should -BeExactly 'bob@x.com'
    }

    It 'writes the matched text with -Raw' {
        Select-TrexMatch '\N' -InputObject 'a 1 b 22' -Raw | Should -Be @('1', '22')
    }

    It 'writes the first match of each input with -List' {
        @(Select-TrexMatch '\N' -InputObject 'a 1 b 22' -List).Count | Should -Be 1
    }

    It 'writes one boolean with -Quiet' {
        'x', 'y 3' | Select-TrexMatch '\N' -Quiet | Should -BeTrue
        'x', 'y' | Select-TrexMatch '\N' -Quiet | Should -BeFalse
    }

    It 'refuses a pattern that does not parse, naming the byte' {
        { Select-TrexMatch '\N{' -InputObject 'x' -ErrorAction Stop } | Should -Throw '*pattern error at byte*'
    }
}

Describe 'Typed values' {
    It 'reads a whole number as a long and a fraction as a decimal' {
        $m = Select-TrexMatch '\N:a \N:b' -InputObject '42 2.50'
        ($m.Groups | Where-Object Name -eq 'a').Value | Should -BeOfType ([long])
        ($m.Groups | Where-Object Name -eq 'a').Value | Should -Be 42
        ($m.Groups | Where-Object Name -eq 'b').Value | Should -BeOfType ([decimal])
        ($m.Groups | Where-Object Name -eq 'b').Value | Should -Be 2.50
    }

    It 'reads a duration as a TimeSpan' {
        $m = Select-TrexMatch '\R:d' -InputObject 'took 1500ms'
        $m.Groups[0].Value | Should -BeOfType ([TimeSpan])
        $m.Groups[0].Value.TotalMilliseconds | Should -Be 1500
    }

    It 'reads an instant as a UTC DateTimeOffset' {
        $m = Select-TrexMatch '\T:t' -InputObject 'at 2026-09-27T01:02:03Z ok'
        $m.Groups[0].Value | Should -BeOfType ([DateTimeOffset])
        $m.Groups[0].Value.UtcDateTime | Should -Be ([datetime]::new(2026, 9, 27, 1, 2, 3, [DateTimeKind]::Utc))
    }

    It 'reads an address as its canonical text' {
        (Select-TrexMatch '\I:ip' -InputObject 'from 10.0.0.1 in').Groups[0].Value | Should -BeExactly '10.0.0.1'
    }

    It 'reads a number in any notation as one token, by its value' {
        $text = 'a 1e3 b 0x3e8 c 50 d 1_000 e 2.5e-4 f 6.02e23 g 0b1010'
        Select-TrexMatch '\N{>100}' -InputObject $text -Raw | Should -Be @('1e3', '0x3e8', '1_000', '6.02e23')
        $hex = (Select-TrexMatch '\N:n' -InputObject 'x 0x3e8').Groups[0].Value
        $hex | Should -BeOfType ([long])
        $hex | Should -Be 1000
        $big = (Select-TrexMatch '\N:n' -InputObject 'x 1e22').Groups[0].Value
        $big | Should -BeOfType ([decimal])
        $big | Should -Be ([decimal]::Parse('10000000000000000000000'))
        (Get-TrexToken '1,000 1e3a').Text | Should -Be @('1', ',', '000', '1', 'e3a')
    }
}

Describe 'Select-TrexMatch over files' {
    BeforeAll {
        $logs = Join-Path $TestDrive 'logs'
        New-Item -ItemType Directory -Path $logs | Out-Null
        Write-TrexTestFile -Path (Join-Path $logs 'a.log') -Text "alpha 10`nbeta 20`ngamma 300`ndelta 4000`nepsilon 5`n"
        Write-TrexTestFile -Path (Join-Path $logs 'b.log') -Text "size 12000`n" -Encoding Utf16
        [System.IO.File]::WriteAllBytes((Join-Path $logs 'c.bin'), [byte[]](0x31, 0x30, 0x30, 0x30, 0x30, 0x00))
    }

    It 'walks a directory and reports each match with its path, line and column' {
        $found = Select-TrexMatch '\N{>=1000}' -Path $logs | Sort-Object Path
        @($found).Count | Should -Be 2
        $found[0].Path | Should -BeLike '*a.log'
        $found[0].LineNumber | Should -Be 4
        $found[0].Column | Should -Be 7
        $found[0].Text | Should -BeExactly '4000'
    }

    It 'reads a UTF-16 file as its text' {
        $m = Select-TrexMatch '\N' -Path (Join-Path $logs 'b.log')
        $m.Text | Should -BeExactly '12000'
    }

    It 'skips a binary file unless -Binary is given, warning for a file named in -Path but not for one found in a directory' {
        $bin = Join-Path $logs 'c.bin'
        @(Select-TrexMatch '\N' -Path $bin -WarningVariable said -WarningAction SilentlyContinue).Count | Should -Be 0
        "$said" | Should -BeLike '*c.bin holds a NUL byte and is binary; -Binary reads it'
        @(Select-TrexMatch '\N' -Path $bin -Binary -WarningVariable said -WarningAction SilentlyContinue).Count | Should -Be 1
        $said | Should -BeNullOrEmpty
        Select-TrexMatch '\N' -Path $logs -WarningVariable walked -WarningAction SilentlyContinue | Out-Null
        $walked | Should -BeNullOrEmpty
    }

    It 'keeps only the walked files a glob matches' {
        @(Select-TrexMatch '\N' -Path $logs -Include '*.log' -Binary | Where-Object Path -like '*c.bin').Count | Should -Be 0
    }
}

Describe 'Select-TrexMatch reporting' {
    BeforeAll {
        [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseDeclaredVarsMoreThanAssignments', 'text', Justification = 'the It blocks below read it')]
        param()
        $text = "alpha 10`nbeta 20`ngamma 300`ndelta 4000`nepsilon 5"
    }

    It 'adds the lines around each match with -Context' {
        $m = Select-TrexMatch '\N{>=1000}' -InputObject $text -Context 1
        $m.PreContext | Should -Be @('gamma 300')
        $m.PostContext | Should -Be @('epsilon 5')
        $wide = Select-TrexMatch '\N{>=1000}' -InputObject $text -Context 2, 0
        $wide.PreContext | Should -Be @('beta 20', 'gamma 300')
        $wide.PostContext.Count | Should -Be 0
    }

    It 'adds the rest of the record a unit names, on a side or both, with -Context' {
        $paragraphs = "intro`n`nalpha`nbeta 1`ngamma`n`nend"
        $m = Select-TrexMatch '\N' -InputObject $paragraphs -Context paragraph
        $m.PreContext | Should -Be @('alpha')
        $m.PostContext | Should -Be @('gamma')
        $ahead = Select-TrexMatch '\N' -InputObject $paragraphs -Context paragraph, 0
        $ahead.PreContext | Should -Be @('alpha')
        $ahead.PostContext.Count | Should -Be 0
        $mixed = Select-TrexMatch '\N' -InputObject $paragraphs -Context 3, paragraph
        $mixed.PreContext | Should -Be @('intro', '', 'alpha')
        $mixed.PostContext | Should -Be @('gamma')
    }

    It 'adds the record -RecordStart defines with -Context record' {
        $log = "2026-01-01T00:00:00Z start`n  detail`n  code 42`n2026-01-01T00:00:01Z next`n  more"
        $m = Select-TrexMatch '"code" \N' -InputObject $log -Context record -RecordStart '\T'
        $m.PreContext | Should -Be @('2026-01-01T00:00:00Z start', '  detail')
        $m.PostContext.Count | Should -Be 0
        { Select-TrexMatch '\N' -InputObject $log -Context record -ErrorAction Stop } | Should -Throw '*-Context record names*'
        { Select-TrexMatch '\N' -InputObject $log -Context 1 -Unit paragraph -ErrorAction Stop } | Should -Throw '*-Unit, -RecordStart and -RecordSpan*'
    }

    It 'writes the lines no match touches with -NotMatch' {
        $lines = Select-TrexMatch '\N{>=100}' -InputObject $text -NotMatch
        $lines.Text | Should -Be @('alpha 10', 'beta 20', 'epsilon 5')
        $lines.LineNumber | Should -Be @(1, 2, 5)
    }

    It 'writes the context around each line -NotMatch selects, as the trex command''s -v does' {
        $numbered = "a 1`nb`nc 2`nd 3`ne`n"
        Select-TrexMatch '\N' -InputObject $numbered -NotMatch -Context 1, 0 -Color -ColorDepth None |
            Should -Be @('a 1', 'b', '--', 'd 3', 'e')
        $found = Select-TrexMatch '\N' -InputObject $numbered -NotMatch -Context 1, 0
        $found.LineNumber | Should -Be @(2, 5)
        $found[1].PreContext | Should -Be @('d 3')
        $cli = Get-TrexCli
        $file = Join-Path $TestDrive 'numbered.txt'
        Write-TrexTestFile -Path $file -Text $numbered
        Select-TrexMatch '\N' -Path $file -NotMatch -Context 1, 0 -Color -ColorDepth None |
            Should -Be (& $cli scan -v '\N' $file -B 1)
    }

    It 'keeps a match only where it covers its line with -WholeLine' {
        $found = Select-TrexMatch '\W \N' -InputObject "  alpha 10  `nsee beta 20" -WholeLine
        @($found).Text | Should -Be @('alpha 10')
    }

    It 'writes at most -MaxCount matches of each input' {
        @(Select-TrexMatch '\N' -InputObject $text -MaxCount 2).Text | Should -Be @('10', '20')
    }

    It 'writes a report template rendered at each match with -Format' {
        Select-TrexMatch '\W:w \N:n' -InputObject $text -Format '${line}:${w}=${n}' -MaxCount 2 |
            Should -Be @('1:alpha=10', '2:beta=20')
    }

    It 'writes a template''s offsets in UTF-16 code units, as a match''s Start counts' {
        $placed = "$([char]0xE9)$([char]::ConvertFromUtf32(0x1F600)) abc 12"
        $m = Select-TrexMatch '\W:w \N' -InputObject $placed
        ($m.Start, $m.Length, $m.Column) | Should -Be @(4, 6, 4)
        Select-TrexMatch '\W:w \N' -InputObject $placed -Format '${start}..${end} ${col}' | Should -BeExactly '4..10 4'
        $placed.Substring($m.Start, $m.Length) | Should -BeExactly 'abc 12'
        ($placed | Group-TrexMatch '\W:w \N' -Key '${start}').Key | Should -BeExactly '4'
        # The line no match touches starts after the first line's 10 units and its newline.
        Select-TrexMatch '\W' -InputObject "$placed`n42" -NotMatch -Format '${start}..${end}' | Should -BeExactly '11..13'
    }

    It 'says what a match is made of with -Explain' {
        $m = Select-TrexMatch '\W \N' -InputObject 'alpha 10' -Explain
        $m.Explanation | Should -BeOfType ([Trex.Explanation])
        $m.Explanation.Tokens.Kind | Should -Be @('word', 'number')
        (Select-TrexMatch '\W \N' -InputObject 'alpha 10').Explanation | Should -BeNullOrEmpty
    }

    It 'writes an error after the last input when none matched, with -RequireMatch' {
        { 'none here', 'nor here' | Select-TrexMatch '\E' -RequireMatch -ErrorAction Stop } | Should -Throw '*no input holds a match*'
        { 'none here', 'bob@x.com' | Select-TrexMatch '\E' -RequireMatch -Quiet -ErrorAction Stop } | Should -Not -Throw
    }

    It 'refuses a -Context of more than two counts' {
        { Select-TrexMatch '\N' -InputObject 'a 1' -Context 1, 2, 3 -ErrorAction Stop } | Should -Throw '*one count*'
    }
}

Describe 'Pattern sets' {
    BeforeAll {
        $setFile = Join-Path $TestDrive 'set.trex'
        Write-TrexTestFile -Path $setFile -Text "let mail = \E`nlet addr = \I`n"
    }

    It 'scans several patterns as one set, each match naming its pattern' {
        $found = Select-TrexMatch '\E', '\I' -InputObject 'bob@x.com from 10.0.0.1'
        $found.Text | Should -Be @('bob@x.com', '10.0.0.1')
        $found.Pattern | Should -Be @('\E', '\I')
    }

    It 'reads the members of a pattern file under their names' {
        $found = Select-TrexMatch -PatternFile $setFile -InputObject 'bob@x.com from 10.0.0.1'
        $found.Pattern | Should -Be @('mail', 'addr')
    }

    It 'writes a member''s name in a report template as ${pattern}' {
        Select-TrexMatch -PatternFile $setFile -InputObject 'bob@x.com from 10.0.0.1' -Format '${pattern}' |
            Should -Be @('mail', 'addr')
    }

    It 'keeps each member''s first match with -SingleMatch' {
        $found = Select-TrexMatch '\E', '\I' -InputObject 'bob@x.com 10.0.0.1 ann@y.org 10.0.0.2' -SingleMatch
        $found.Text | Should -Be @('bob@x.com', '10.0.0.1')
        { Select-TrexMatch '\E' -InputObject 'bob@x.com' -SingleMatch -ErrorAction Stop } | Should -Throw '*-SingleMatch keeps*'
    }

    It 'tests a pattern file as one set' {
        Test-TrexMatch -PatternFile $setFile -InputObject 'from 10.0.0.1' | Should -BeTrue
        'nothing here' | Test-TrexMatch -PatternFile $setFile | Should -BeFalse
    }

    It 'refuses -Pattern and -PatternFile together, and neither' {
        { Select-TrexMatch '\N' -PatternFile $setFile -InputObject 'a 1' -ErrorAction Stop } | Should -Throw '*give one*'
        { Select-TrexMatch -InputObject 'a 1' -ErrorAction Stop } | Should -Throw '*pattern is required*'
    }
}

Describe 'Select-TrexMatch walk filters' {
    BeforeAll {
        $tree = Join-Path $TestDrive 'tree'
        New-Item -ItemType Directory -Path $tree | Out-Null
        Write-TrexTestFile -Path (Join-Path $tree 'main.rs') -Text "fn main() { let x = 42; }`n"
        Write-TrexTestFile -Path (Join-Path $tree 'notes.md') -Text "the answer is 42`n"
    }

    It 'keeps the walked files of a type with -FileType' {
        (Select-TrexMatch '\N' -Path $tree -FileType rust).Path | Should -BeLike '*main.rs'
        (Select-TrexMatch '\N' -Path $tree -ExcludeFileType rust).Path | Should -BeLike '*notes.md'
    }

    It 'refuses a file type it does not know' {
        { Select-TrexMatch '\N' -Path $tree -FileType nosuchtype -ErrorAction Stop } | Should -Throw
    }
}

Describe 'Select-TrexMatch counts, file lists and statistics' {
    BeforeAll {
        $tree = Join-Path $TestDrive 'counted'
        New-Item -ItemType Directory -Path $tree | Out-Null
        Write-TrexTestFile -Path (Join-Path $tree 'a.log') -Text "from 10.0.0.1 to 10.0.0.2`nno ip here`n  10.0.0.3  `n"
        Write-TrexTestFile -Path (Join-Path $tree 'b.txt') -Text "alpha 1`nbeta`n"
    }

    It 'counts the lines of each file holding a match with -Count, and the matches with -CountMatches' {
        $lines = Select-TrexMatch '\I' -Path $tree -Count
        $lines | Should -BeOfType ([Trex.MatchCount])
        @($lines).Count | Should -Be 1
        $lines.Path | Should -BeLike '*a.log'
        $lines | ForEach-Object Count | Should -Be 2
        Select-TrexMatch '\I' -Path $tree -CountMatches | ForEach-Object Count | Should -Be 3
    }

    It 'counts the lines no match touches with -Count -NotMatch, to -MaxCount at most' {
        Select-TrexMatch '\I' -Path $tree -Count -NotMatch | Sort-Object Path | ForEach-Object Count | Should -Be @(1, 2)
        Select-TrexMatch '\N' -InputObject "a 1`nb`nc" -Count -NotMatch -MaxCount 1 | ForEach-Object Count | Should -Be 1
    }

    It 'counts every string piped in as one input, written after the last' {
        $counted = 'a 1', 'b', 'c 2 3' | Select-TrexMatch '\N' -Count
        @($counted).Count | Should -Be 1
        $counted.Path | Should -BeExactly ''
        $counted | ForEach-Object Count | Should -Be 2
        'a 1', 'b', 'c 2 3' | Select-TrexMatch '\N' -CountMatches | ForEach-Object Count | Should -Be 3
        'x' | Select-TrexMatch '\N' -Count | ForEach-Object Count | Should -Be 0
    }

    It 'counts the records -Unit names in place of lines' {
        $text = "a 1`nb 2`n`nc`n"
        Select-TrexMatch '\N' -InputObject $text -Count | ForEach-Object Count | Should -Be 2
        Select-TrexMatch '\N' -InputObject $text -Count -Unit paragraph | ForEach-Object Count | Should -Be 1
        Select-TrexMatch '\N' -InputObject $text -Count -NotMatch -Unit paragraph | ForEach-Object Count | Should -Be 1
    }

    It 'writes the paths of the files holding a match, or none' {
        Select-TrexMatch '\I' -Path $tree -FilesWithMatches | Should -BeLike '*a.log'
        Select-TrexMatch '\I' -Path $tree -FilesWithoutMatch | Should -BeLike '*b.txt'
        @(Select-TrexMatch '\I' -Path $tree -FilesWithMatches -NotMatch).Count | Should -Be 2
    }

    It 'orders the files read with -Sort, and -Descending reverses it' {
        Select-TrexMatch '\W' -Path $tree -FilesWithMatches -Sort Path | Split-Path -Leaf | Should -Be @('a.log', 'b.txt')
        Select-TrexMatch '\W' -Path $tree -FilesWithMatches -Sort Path -Descending | Split-Path -Leaf |
            Should -Be @('b.txt', 'a.log')
        { Select-TrexMatch '\W' -Path $tree -Descending -ErrorAction Stop } | Should -Throw '*give -Sort*'
    }

    It 'adds a Trex.ScanStats after the report with -Stats' {
        $out = Select-TrexMatch '\I' -Path $tree -Stats
        @($out | Where-Object { $_ -is [Trex.Match] }).Count | Should -Be 3
        $stats = $out[-1]
        $stats | Should -BeOfType ([Trex.ScanStats])
        $stats.Matches | Should -Be 3
        $stats.MatchedLines | Should -Be 2
        $stats.FilesWithMatches | Should -Be 1
        $stats.FilesSearched | Should -Be 2
        $bytes = (Get-ChildItem -LiteralPath $tree -File | Measure-Object Length -Sum).Sum
        $stats.BytesSearched | Should -Be $bytes
        $stats.Searching | Should -BeOfType ([TimeSpan])
        $stats.Elapsed | Should -BeGreaterOrEqual $stats.Searching
        $text = ('a 1', 'b 2 3' | Select-TrexMatch '\N' -Stats)[-1]
        $text.FilesSearched | Should -Be 1
        $text.Matches | Should -Be 3
        ('a 1', 'b 2 3' | Select-TrexMatch '\N' -Stats -PerString)[-1].FilesSearched | Should -Be 2
    }

    It 'reads the rungs that answered and the lexing from the trace under -Stats, and the route under -Explain' {
        $out = @(Select-TrexMatch '\I' -InputObject 'from 10.0.0.1 to 10.0.0.2' -Stats -Explain)
        $stats = $out[-1]
        $stats.TokensLexed | Should -BeOfType ([long])
        $stats.Lexing | Should -BeOfType ([TimeSpan])
        $stats.Matching | Should -BeLessOrEqual $stats.Searching
        @($stats.Routes).Count | Should -BeGreaterThan 0
        $stats.Routes[0] | Should -BeOfType ([Trex.ScanRoute])
        $stats.Routes.Inputs | ForEach-Object { $_ | Should -Be 1 }
        $out[0].Explanation.Route | Should -BeIn @($stats.Routes.Rung)
        # Each input may be answered on a different rung, so the two inputs
        # are counted across every rung.
        $again = @('a 1', 'b 2' | Select-TrexMatch '\N' -Stats -PerString)[-1]
        $read = ($again.Routes | ForEach-Object { '{0}={1}' -f $_.Rung, $_.Inputs }) -join '; '
        ($again.Routes | Measure-Object -Property Inputs -Sum).Sum | Should -Be 2 -Because "the routes read $read"
    }

    It 'shows a register inside its match as its name and its text' {
        $m = Select-TrexMatch '\E:e' -InputObject 'ping bob@x.com'
        "$($m.Groups[0])" | Should -BeExactly 'e=bob@x.com'
    }

    It 'refuses two switches that each choose the output, and a switch the chosen output cannot take' {
        { Select-TrexMatch '\N' -InputObject 'a 1' -Count -Raw -ErrorAction Stop } | Should -Throw '*give one*'
        { Select-TrexMatch '\N' -InputObject 'a 1' -Count -Context 1 -ErrorAction Stop } | Should -Throw '*-Context adds lines*'
        { Select-TrexMatch '\N' -InputObject 'a 1' -Raw -Explain -ErrorAction Stop } | Should -Throw '*-Explain says*'
        { Select-TrexMatch '\N' -InputObject 'a 1' -Unit paragraph -ErrorAction Stop } | Should -Throw '*-Unit, -RecordStart and -RecordSpan*'
    }
}

Describe 'Select-TrexMatch engines' {
    BeforeAll {
        [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseDeclaredVarsMoreThanAssignments', 'text', Justification = 'the It blocks below read it')]
        param()
        $text = (1..400 | ForEach-Object { "line $_ from 10.0.0.$($_ % 250) by bob$_@x.com" }) -join "`n"
    }

    It 'finds the same matches on every engine' {
        $plain = (Select-TrexMatch '\I' -InputObject $text).Text
        @($plain).Count | Should -Be 400
        (Select-TrexMatch '\I' -InputObject $text -Backend Cpu).Text | Should -Be $plain
        (Select-TrexMatch '\I' -InputObject $text -Backend Gpu).Text | Should -Be $plain
        (Select-TrexMatch '\I' -InputObject $text -DualGrain).Text | Should -Be $plain
        (Select-TrexMatch '\I' -InputObject $text -ChunkSize 64).Text | Should -Be $plain
    }

    It 'says how the scan ran as verbose output' {
        $notes = Select-TrexMatch '\I' -InputObject $text -DualGrain -Verbose 4>&1 |
            Where-Object { $_ -is [System.Management.Automation.VerboseRecord] }
        $notes.Message | Should -BeLike 'text: dual grain:*'
    }

    It 'refuses two engine choices, one for a set, and a chunk of no bytes' {
        { Select-TrexMatch '\I' -InputObject 'x' -DualGrain -ChunkSize 8 -ErrorAction Stop } | Should -Throw '*give one*'
        { Select-TrexMatch '\I', '\E' -InputObject 'x' -DualGrain -ErrorAction Stop } | Should -Throw '*take no -Backend Gpu*'
        { Select-TrexMatch '\I' -InputObject 'x' -ChunkSize 0 -ErrorAction Stop } | Should -Throw '*1 or more*'
    }
}

Describe 'Test-TrexMatch' {
    It 'answers whether the pattern matches' {
        Test-TrexMatch '\E' 'ping bob@x.com' | Should -BeTrue
        Test-TrexMatch '\E' 'no address' | Should -BeFalse
        'a', 'bob@x.com' | Test-TrexMatch '\E' | Should -BeTrue
    }
}

Describe 'New-TrexPattern' {
    BeforeAll {
        [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseDeclaredVarsMoreThanAssignments', 'p', Justification = 'the It blocks below read it')]
        param()
        $p = New-TrexPattern '\E:e'
    }

    It 'compiles once and matches through its methods' {
        $p | Should -BeOfType ([Trex.Pattern])
        $p.Source | Should -BeExactly '\E:e'
        $p.CaptureNames | Should -Be @('e')
        $p.IsMatch('ping bob@x.com') | Should -BeTrue
        $p.Find('ping bob@x.com').Captures.e | Should -BeExactly 'bob@x.com'
        $p.Find('none') | Should -BeNullOrEmpty
        @($p.FindAll('a@b.com c@d.org')).Count | Should -Be 2
    }

    It 'replaces each match with a rendered template' {
        $p.Replace('mail bob@x.com now', '[${e:domain}]') | Should -BeExactly 'mail [x.com] now'
    }

    It 'splits on the matches' {
        (New-TrexPattern '\P').Split('a, b; c') | Should -Be @('a', ' b', ' c')
    }

    It 'constructs from script as [Trex.Pattern]::new' {
        ([Trex.Pattern]::new('\N')).IsMatch('x 1') | Should -BeTrue
    }

    It 'is taken by Select-TrexMatch in place of source text' {
        (Select-TrexMatch $p -InputObject 'ping bob@x.com').Captures.e | Should -BeExactly 'bob@x.com'
    }
}
