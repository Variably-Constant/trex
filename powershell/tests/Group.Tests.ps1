BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Group-TrexMatch' {
    It 'groups by a typed slice of a register, most frequent first' {
        $groups = 'a 10.0.0.1 b 10.0.0.2 c 10.0.1.9' | Group-TrexMatch '\I:ip' -Key '${ip:octet1-3}'
        $groups[0].Key | Should -BeExactly '10.0.0'
        $groups[0].Count | Should -Be 2
        $groups[1].Key | Should -BeExactly '10.0.1'
        $groups[1].Count | Should -Be 1
    }

    It 'groups every string piped in together' {
        $groups = 'x 1', 'x 2', 'y 3' | Group-TrexMatch '\W:w \N' -Key '${w}' -SortBy Key
        $groups.Key | Should -Be @('x', 'y')
        $groups | ForEach-Object Count | Should -Be @(2, 1)
    }

    It 'sums durations into a TimeSpan' {
        $groups = 'x 1500ms y 500ms x 2s' | Group-TrexMatch '\W:u \R:t' -Key '${u}' -Sum t
        $x = $groups | Where-Object Key -eq 'x'
        $x.Sum | Should -BeOfType ([TimeSpan])
        $x.Sum.TotalMilliseconds | Should -Be 3500
    }

    It 'averages exactly, as the decimal nearest a repeating quotient' {
        $g = 'a 1 a 2 a 4' | Group-TrexMatch '\W:u \N:n' -Key '${u}' -Average n
        $g.Average | Should -BeOfType ([decimal])
        $g.Average | Should -Be (7d / 3d)
        ('a 5 a 10 a 10' | Group-TrexMatch '\W:u \N:n' -Key '${u}' -Average n).Average | Should -Be (25d / 3d)
        ('a 2 a 0 a 0' | Group-TrexMatch '\W:u \N:n' -Key '${u}' -Average n).Average | Should -Be (2d / 3d)
    }

    It 'reports the least, the greatest and a percentile' {
        $g = 'a 5 a 1 a 9 a 3' | Group-TrexMatch '\W:u \N:n' -Key '${u}' -Minimum n -Maximum n -Percentile 50 -PercentileOf n
        $g.Minimum | Should -Be 1
        $g.Maximum | Should -Be 9
        $g.P50 | Should -Be 3
    }

    It 'decides a percentile between two values by the method named' {
        $text = 'a 10 a 20 a 30 a 40'
        $p60 = { param($method) ($text | Group-TrexMatch '\W:u \N:n' -Key '${u}' -Percentile 60 -PercentileOf n -PercentileMethod $method).P60 }
        & $p60 Nearest | Should -Be 30
        & $p60 Lower | Should -Be 20
        & $p60 Linear | Should -Be 28
        & $p60 Hybrid | Should -Be 28
    }

    It 'reports the earliest and latest instants whole, fraction and all' {
        $g = 'h 2026-09-27T10:00:00.250Z h 2026-09-27T09:00:00.750Z' |
            Group-TrexMatch '\W:h \T:t' -Key '${h}' -Minimum t -Maximum t
        $g.Minimum | Should -BeOfType ([DateTimeOffset])
        $g.Minimum | Should -Be ([DateTimeOffset]'2026-09-27T09:00:00.750Z')
        $g.Maximum | Should -Be ([DateTimeOffset]'2026-09-27T10:00:00.250Z')
    }

    It 'reports an address as its text and a version as the text semver orders it by' {
        ('x 10.0.0.10 x 10.0.0.9' | Group-TrexMatch '\W:u \I:ip' -Key '${u}' -Minimum ip).Minimum | Should -BeExactly '10.0.0.9'
        ('x v1.2.0 x v1.10.0 x v1.3.0' | Group-TrexMatch '\W:u \V:v' -Key '${u}' -Maximum v).Maximum | Should -BeExactly '1.10.0'
    }

    It 'groups the matches of several patterns by the member that made each, as ${pattern}' {
        $groups = 'bob@x.com 10.0.0.1 ann@y.org' | Group-TrexMatch '\E', '\I' -Key '${pattern}' -SortBy Key
        $groups.Key | Should -Be @('\E', '\I')
        $groups | ForEach-Object Count | Should -Be @(2, 1)
    }

    It 'groups the members of a pattern file under their names' {
        $set = Join-Path $TestDrive 'set.trex'
        Write-TrexTestFile -Path $set -Text "let mail = \E`nlet addr = \I`n"
        $groups = 'bob@x.com 10.0.0.1 ann@y.org' | Group-TrexMatch -PatternFile $set -Key '${pattern}'
        $groups[0].Key | Should -BeExactly 'mail'
        $groups[0].Count | Should -Be 2
        $groups[1].Key | Should -BeExactly 'addr'
    }

    It 'refuses a key that reads an axis, which no match is explained for' {
        { 'a 1' | Group-TrexMatch '\N' -Key '${@kind}' -ErrorAction Stop } | Should -Throw '*reads no axis*'
    }

    It 'writes the distinct keys alone with -Unique' {
        'x 1 y 2 x 3' | Group-TrexMatch '\W:w \N' -Key '${w}' -Unique -SortBy Key | Should -Be @('x', 'y')
    }

    It 'refuses a sum over a kind that does not add' {
        { 'a b' | Group-TrexMatch '\W:u \W:v' -Key '${u}' -Sum v -ErrorAction Stop } | Should -Throw '*not defined*'
    }

    It 'refuses a linear percentile of a kind that does not interpolate' {
        { 'x v1.2.0' | Group-TrexMatch '\W:u \V:v' -Key '${u}' -Percentile 50 -PercentileOf v -PercentileMethod Linear -ErrorAction Stop } |
            Should -Throw '*cannot interpolate*'
    }
}
