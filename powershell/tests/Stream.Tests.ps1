BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Stream scanners' {
    It 'gives back every match over the chunks, at its offset into the whole stream' {
        $s = [Trex.StreamScanner]::new('\E')
        $found = @($s.Push('mail bob@x.com and ')) + @($s.Push('ann@y.org now')) + @($s.Finish())
        $found | Should -BeOfType ([Trex.StreamMatch])
        $found.Text | Should -Be @('bob@x.com', 'ann@y.org')
        $found.Start | Should -Be @(5, 19)
        $found | ForEach-Object Length | Should -Be @(9, 9)
    }

    It 'finds over small chunks what one scan of the whole finds' {
        $text = 'from 10.0.0.1 to bob@x.com and 10.0.0.22 end, é 10.9.8.7'
        $whole = Select-TrexMatch '\I' -InputObject $text
        $s = New-TrexStreamScanner '\I'
        $found = @()
        for ($at = 0; $at -lt $text.Length; $at += 5) {
            $found += @($s.Push($text.Substring($at, [Math]::Min(5, $text.Length - $at))))
        }
        $found += @($s.Finish())
        $found.Text | Should -Be $whole.Text
        $found.Start | Should -Be $whole.Start
    }

    It 'names the pattern of a set that made each match' {
        $s = New-TrexStreamScanner '\E', '\I'
        $found = @($s.Push('bob@x.com from 10.0.0.1')) + @($s.Finish())
        $found.Pattern | Should -Be @('\E', '\I')
    }

    It 'refuses a chunk after the stream is finished' {
        $s = [Trex.StreamScanner]::new('\N')
        $s.Finish() | Out-Null
        { $s.Push('1') } | Should -Throw '*finished*'
    }
}
