BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule

    # Five lines, each a word and a number; the line starts are 0, 6, 12, 20
    # and 27, and the text is 34 bytes long.
    $script:Five = "one 1`ntwo 2`nthree 3`nfour 4`nfive 5`n"

    # A file under the test drive holding $Text as UTF-8.
    function New-TestFile([string] $Name, [string] $Text) {
        $path = Join-Path $TestDrive $Name
        Write-TrexTestFile -Path $path -Text $Text
        $path
    }

    function Add-TestText([string] $Path, [string] $Text) {
        [System.IO.File]::AppendAllText($Path, $Text, [System.Text.UTF8Encoding]::new($false))
    }

    # $Script run with the module imported in a runspace of its own, where a
    # follow blocks until it is stopped while the test feeds its file.
    function Start-Follow([string] $Script) {
        $shell = [PowerShell]::Create()
        $module = Join-Path $env:PWRS_MODULE 'Trex.psd1'
        $null = $shell.AddScript("Import-Module '$module'; $Script")
        $output = [System.Management.Automation.PSDataCollection[psobject]]::new()
        $none = [System.Management.Automation.PSDataCollection[psobject]]::new()
        $none.Complete()
        $handle = $shell.BeginInvoke($none, $output)
        [pscustomobject]@{ Shell = $shell; Output = $output; Handle = $handle }
    }

    # The items a follow's output, warning or error collection holds now, read
    # by index: enumerating a collection its pipeline still writes to waits for
    # the pipeline to end.
    function Get-Items($Collection) {
        @(for ($i = 0; $i -lt $Collection.Count; $i++) { $Collection[$i] })
    }

    # Waits until the follow has written $Count objects, or warnings with
    # -Warnings, and fails with what it wrote and why where it stops first or
    # half a minute passes.
    function Wait-Follow($Follow, [int] $Count, [switch] $Warnings) {
        # Assigned, not written from an if: output unrolls a collection.
        $items = $Follow.Output
        if ($Warnings) {
            $items = $Follow.Shell.Streams.Warning
        }
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        while ($items.Count -lt $Count) {
            $state = $Follow.Shell.InvocationStateInfo
            $errors = (Get-Items $Follow.Shell.Streams.Error) -join '; '
            if ($state.State -ne 'Running') {
                throw "the follow ended ($($state.State)) with $($items.Count) of $Count written: $($state.Reason) $errors"
            }
            if ([DateTime]::UtcNow -gt $deadline) {
                throw "waited 30 s for $Count; the follow wrote $($items.Count): $((Get-Items $items) -join ' | '); errors: $errors"
            }
            Start-Sleep -Milliseconds 100
        }
    }

    function Stop-Follow($Follow) {
        $Follow.Shell.Stop()
        $Follow.Shell.Dispose()
    }
}

Describe 'Get-TrexLine' {
    It 'writes the first lines, the last, or a range, each at its line number' {
        $f = New-TestFile 'five.txt' $Five
        $head = Get-TrexLine -Path $f -Head 2
        $head | Should -BeOfType ([Trex.Line])
        $head.Text | Should -Be @('one 1', 'two 2')
        $head.LineNumber | Should -Be @(1, 2)
        $tail = Get-TrexLine -Path $f -Tail 2
        $tail.Text | Should -Be @('four 4', 'five 5')
        $tail.LineNumber | Should -Be @(4, 5)
        (Get-TrexLine -Path $f -Lines '2..3').LineNumber | Should -Be @(2, 3)
        (Get-TrexLine -Path $f -Lines '4..').Text | Should -Be @('four 4', 'five 5')
        (Get-TrexLine -Path $f -Lines (2..3)).Text | Should -Be @('two 2', 'three 3')
        (Get-TrexLine -Path $f -First 1).Text | Should -Be 'one 1'
        (Get-TrexLine -Path $f -Last 1).LineNumber | Should -Be 5
        (Get-TrexLine -Path $f -Tail 9).Count | Should -Be 5
    }

    It 'reads text piped in, each string on its own' {
        $line = "s1`ns2`ns3`n" | Get-TrexLine -Last 1
        $line.Text | Should -Be 's3'
        $line.LineNumber | Should -Be 3
        $line.Path | Should -Be ''
    }

    It 'counts paragraphs with -Unit' {
        $f = New-TestFile 'para.txt' "a one`na two`n`nb one`n`nc one`nc two`n"
        (Get-TrexLine -Path $f -First 1 -Unit paragraph).Text | Should -Be @('a one', 'a two')
        (Get-TrexLine -Path $f -Last 1 -Unit paragraph).LineNumber | Should -Be @(6, 7)
    }

    It 'writes strings with -Passthru, a header ahead of each file where several are read' {
        $a = New-TestFile 'a.txt' "a1`na2`n"
        $b = New-TestFile 'b.txt' "b1`nb2`n"
        Get-TrexLine -Path $a -Head 1 -Passthru | Should -BeExactly 'a1'
        $both = Get-TrexLine -Path $a, $b -Head 1 -Passthru
        $both | Should -Be @("==> $a <==", 'a1', '', "==> $b <==", 'b1')
    }

    It 'prints what the trex command prints' {
        $f = New-TestFile 'cli.txt' $Five
        $cli = Get-TrexCli
        $expected = @(& $cli tail 2 $f)
        Get-TrexLine -Path $f -Tail 2 -Passthru | Should -Be $expected
        $expected = @(& $cli lines '2..3' $f)
        Get-TrexLine -Path $f -Lines '2..3' -Passthru | Should -Be $expected
    }

    It 'refuses no window, two windows, and a follow of a window that ends' {
        $f = New-TestFile 'refuse.txt' $Five
        { Get-TrexLine -Path $f -ErrorAction Stop } | Should -Throw '*give -Head, -Tail or -Lines*'
        { Get-TrexLine -Path $f -Head 1 -Tail 1 -ErrorAction Stop } | Should -Throw '*each name the part read*'
        { Get-TrexLine -Path $f -Head 3 -Follow -ErrorAction Stop } | Should -Throw '*ends before the file does*'
        { Get-TrexLine -Path $f -Lines '0..2' -ErrorAction Stop } | Should -Throw '*count from 1*'
    }
}

Describe 'A window on a scan' {
    It 'places each match at the input''s own line, column and UTF-16 offset' {
        $f = New-TestFile 'scan.txt' $Five
        $m = Select-TrexMatch '\N' -Path $f -Tail 2
        $m.Text | Should -Be @('4', '5')
        $m.Start | Should -Be @(25, 32)
        $m.LineNumber | Should -Be @(4, 5)
        $m.Column | Should -Be @(6, 6)
        (Select-TrexMatch '\N' -Path $f -Lines '2..3').LineNumber | Should -Be @(2, 3)
        (Select-TrexMatch '\N' -Path $f -Last 1).Start | Should -Be 32
    }

    It 'counts the UTF-16 code units ahead of a tail, not its bytes' {
        # The code point, since Windows PowerShell reads this file in the ANSI code page.
        $f = New-TestFile 'wide.txt' "$([char]0x00E9) 1`nb 2`n"
        $m = Select-TrexMatch '\N' -Path $f -Tail 1
        $m.Text | Should -Be '2'
        $m.Start | Should -Be 6
        $m.LineNumber | Should -Be 2
    }

    It 'reads a file marked UTF-8 or UTF-16 at the places the same text holds unmarked' {
        $text = "$([char]0x00E9) 1`nb 2`nc $([char]0x4E2D) 3`nd 4`n"
        $plain = New-TestFile 'plain.txt' $text
        $want = Select-TrexMatch '\N' -Path $plain -Tail 2
        foreach ($encoding in 'Utf8Bom', 'Utf16') {
            $f = Join-Path $TestDrive "marked-$encoding.txt"
            Write-TrexTestFile -Path $f -Text $text -Encoding $encoding
            $m = Select-TrexMatch '\N' -Path $f -Tail 2
            $m.Text | Should -Be $want.Text
            $m.Start | Should -Be $want.Start
            $m.LineNumber | Should -Be $want.LineNumber
            (Get-TrexLine -Path $f -Tail 2).Text | Should -Be (Get-TrexLine -Path $plain -Tail 2).Text
            (Get-TrexLine -Path $f -Head 1).Text | Should -Be (Get-TrexLine -Path $plain -Head 1).Text
            (Get-TrexLine -Path $f -Lines '2..3').LineNumber | Should -Be @(2, 3)
        }
    }

    It 'reads a window of text piped in' {
        $m = "a 1`nb 2`nc 3`n" | Select-TrexMatch '\N' -Tail 1
        $m.Start | Should -Be 10
        $m.LineNumber | Should -Be 3
    }

    It 'counts a match only where it lies wholly inside the window' {
        $f = New-TestFile 'edge.txt' "call(a,`nb)`nnext(c)`n"
        (Select-TrexMatch '\W\B(.*)' -Path $f -CountMatches).Count | Should -Be 2
        $m = Select-TrexMatch '\W\B(.*)' -Path $f -Lines '2..'
        $m.Text | Should -Be 'next(c)'
        $m.Start | Should -Be 11
    }

    It 'reports a format and JSON at the input''s own place, as the trex command does' {
        $f = New-TestFile 'report.txt' $Five
        Select-TrexMatch '\N' -Path $f -Tail 1 -Format '${line}:${start} ${0}' | Should -BeExactly '5:32 5'
        (Select-TrexMatch '\N' -Path $f -Head 3 -CountMatches).Count | Should -Be 3
        $cli = Get-TrexCli
        $expected = (& $cli scan '\N' $f --tail 2 --json) -join "`n"
        (Select-TrexMatch '\N' -Path $f -Tail 2 -Json) -join "`n" | Should -BeExactly $expected
    }

    It 'answers Test-TrexMatch over the window alone' {
        $f = New-TestFile 'test.txt' "TODO one`nfine`nTODO two`n"
        Test-TrexMatch '"TODO"' -Path $f -Lines '2..2' | Should -BeFalse
        Test-TrexMatch '"TODO"' -Path $f -First 1 | Should -BeTrue
        "a`nTODO`n" | Test-TrexMatch '"TODO"' -Head 1 | Should -BeFalse
    }

    It 'groups the window''s matches under keys that read its own lines' {
        $f = New-TestFile 'group.txt' $Five
        $groups = Group-TrexMatch '\W:w \N' -Key '${line}' -Path $f -Lines '2..3' -SortBy Key
        $groups.Key | Should -Be @('2', '3')
        $groups | ForEach-Object Count | Should -Be @(1, 1)
        @(Group-TrexMatch '\W:w \N' -Key '${w}' -Path $f -MaxCount 2).Count | Should -Be 2
    }

    It 'mines the shapes of the records a window holds' {
        $f = New-TestFile 'shape.txt' $Five
        $shapes = Get-TrexRecordShape -Path $f -Head 2
        $shapes.Count | Should -Be 2
    }

    It 'places a rule''s findings at the input''s own line and offset' {
        $rules = New-TestFile 'rules.trex' "rule todo note `"a TODO left`" = `"TODO`"`n"
        $f = New-TestFile 'app.conf' "TODO one`nfine`nTODO two`n"
        $found = Invoke-TrexRule -Path $f -RuleFile $rules -Tail 1
        $found.Rule | Should -Be 'todo'
        $found.Region.Line | Should -Be 3
        $found.Start | Should -Be 14
        Invoke-TrexRule -Path $f -RuleFile $rules -Tail 1 -Format '${line}:${col} ${rule}' | Should -BeExactly '3:1 todo'
    }
}

Describe 'A window on a rewrite' {
    It 'writes the window alone, rewritten or masked' {
        $f = New-TestFile 'write.txt' $Five
        Edit-TrexText '\N:n' '<${n}>' -Path $f -Tail 1 | Should -BeExactly "five <5>`n"
        Protect-TrexText '\N' -Path $f -Head 1 | Should -BeExactly "one *`n"
        "a 1`nb 2`n" | Edit-TrexText '\N:n' '<${n}>' -Last 1 | Should -BeExactly "b <2>`n"
        Edit-TrexText '\N' -ScriptBlock { "$($_.Start)" } -Path $f -Tail 1 | Should -BeExactly "five 32`n"
    }

    It 'changes a file in place only inside the window' {
        $f = New-TestFile 'inplace.txt' $Five
        Edit-TrexText '\N:n' '<${n}>' -Path $f -Lines '2..2' -InPlace -Confirm:$false
        [System.IO.File]::ReadAllText($f) | Should -BeExactly "one 1`ntwo <2>`nthree 3`nfour 4`nfive 5`n"
        Protect-TrexText '\N' -Path $f -Tail 1 -InPlace -Confirm:$false
        [System.IO.File]::ReadAllText($f) | Should -BeExactly "one 1`ntwo <2>`nthree 3`nfour 4`nfive *`n"
    }

    It 'writes a window back into a file in the file''s own encoding' {
        $hex = { param($bytes) [System.BitConverter]::ToString([byte[]]$bytes) }
        foreach ($encoding in 'Utf8Bom', 'Utf16') {
            $enc = if ($encoding -eq 'Utf16') { [System.Text.UnicodeEncoding]::new($false, $true) } else { [System.Text.UTF8Encoding]::new($true) }
            $f = Join-Path $TestDrive "marked-inplace-$encoding.txt"
            Write-TrexTestFile -Path $f -Text $Five -Encoding $encoding
            Edit-TrexText '\N:n' '<${n}>' -Path $f -Tail 1 -InPlace -Confirm:$false
            Protect-TrexText '\N' -Path $f -Head 1 -InPlace -Confirm:$false
            $want = $enc.GetPreamble() + $enc.GetBytes("one *`ntwo 2`nthree 3`nfour 4`nfive <5>`n")
            & $hex ([System.IO.File]::ReadAllBytes($f)) | Should -BeExactly (& $hex $want)
            $rules = New-TestFile "fix-$encoding.trex" "rule todo note `"a TODO left`" = `"TODO`"`nfix todo = DONE`n"
            $conf = Join-Path $TestDrive "fix-$encoding.conf"
            Write-TrexTestFile -Path $conf -Text "TODO one`nfine`nTODO two`n" -Encoding $encoding
            Invoke-TrexRule -Path $conf -RuleFile $rules -Tail 1 -Fix -Confirm:$false
            $want = $enc.GetPreamble() + $enc.GetBytes("TODO one`nfine`nDONE two`n")
            & $hex ([System.IO.File]::ReadAllBytes($conf)) | Should -BeExactly (& $hex $want)
        }
        # A byte that is not UTF-8 is a U+FFFD in the string the module edits,
        # and stays the byte it is in the file.
        $latin = Join-Path $TestDrive 'latin.txt'
        [System.IO.File]::WriteAllBytes($latin, [byte[]](0x63, 0x61, 0x66, 0xE9, 0x20, 0x31, 0x0A, 0x62, 0x20, 0x32, 0x0A))
        Edit-TrexText '\N:n' '<${n}>' -Path $latin -Tail 1 -InPlace -Confirm:$false
        $want = [byte[]](0x63, 0x61, 0x66, 0xE9, 0x20, 0x31, 0x0A, 0x62, 0x20, 0x3C, 0x32, 0x3E, 0x0A)
        & $hex ([System.IO.File]::ReadAllBytes($latin)) | Should -BeExactly (& $hex $want)
    }

    It 'describes only the window''s changes in a diff' {
        $f = New-TestFile 'diff.txt' $Five
        $diff = Edit-TrexText '\N:n' '<${n}>' -Path $f -Head 1 -Diff
        $diff | Should -Match '-one 1'
        $diff | Should -Match '\+one <1>'
        $diff | Should -Not -Match '<2>'
    }

    It 'fixes only inside the window' {
        $rules = New-TestFile 'fix.trex' "rule todo note `"a TODO left`" = `"TODO`"`nfix todo = DONE`n"
        $f = New-TestFile 'fix.conf' "TODO one`nfine`nTODO two`n"
        Invoke-TrexRule -Path $f -RuleFile $rules -Tail 1 -Fix -Confirm:$false
        [System.IO.File]::ReadAllText($f) | Should -BeExactly "TODO one`nfine`nDONE two`n"
    }

    It 'refuses -Unit without a window, and a followed file written back' {
        $f = New-TestFile 'unit.txt' $Five
        { Edit-TrexText '\N' 'X' -Path $f -Unit paragraph -ErrorAction Stop } | Should -Throw '*-Unit says what*'
        { Edit-TrexText '\N' 'X' -Path $f -Follow -InPlace -ErrorAction Stop } | Should -Throw '*takes no -InPlace*'
        { Protect-TrexText '\N' -Path $f -Head 2 -Follow -ErrorAction Stop } | Should -Throw '*ends before the file does*'
    }
}

Describe 'The window parameters' {
    It 'names -First and -Last for -Head and -Tail on every cmdlet that reads a window' {
        foreach ($name in 'Get-TrexLine', 'Select-TrexMatch', 'Test-TrexMatch', 'Edit-TrexText', 'Protect-TrexText',
            'Group-TrexMatch', 'Invoke-TrexRule', 'Get-TrexRecordShape') {
            $parameters = (Get-Command $name).Parameters
            $parameters['Head'].Aliases | Should -Contain 'First' -Because $name
            $parameters['Tail'].Aliases | Should -Contain 'Last' -Because $name
            $parameters.ContainsKey('Lines') | Should -BeTrue -Because $name
        }
    }

    It 'caps with -MaxCount where -First once did' {
        (Get-Command Edit-TrexText).Parameters.ContainsKey('MaxCount') | Should -BeTrue
        (Get-Command Group-TrexMatch).Parameters.ContainsKey('MaxCount') | Should -BeTrue
    }
}

Describe 'Following a file' {
    It 'writes the lines a tail gains, and reads a truncated file from its start' {
        $f = New-TestFile 'tail.log' "a 1`nb 2`n"
        $follow = Start-Follow "Get-TrexLine -Path '$f' -Tail 1 -Follow"
        try {
            Wait-Follow $follow 1
            $follow.Output[0].Text | Should -Be 'b 2'
            $follow.Output[0].LineNumber | Should -Be 2
            Add-TestText $f "c 3`n"
            Wait-Follow $follow 2
            $follow.Output[1].Text | Should -Be 'c 3'
            $follow.Output[1].LineNumber | Should -Be 3
            Write-TrexTestFile -Path $f -Text "d 4`n"
            Wait-Follow $follow 1 -Warnings
            $follow.Shell.Streams.Warning[0].Message | Should -Match 'truncated'
            Wait-Follow $follow 3
            $follow.Output[2].Text | Should -Be 'd 4'
            $follow.Output[2].LineNumber | Should -Be 1
        } finally {
            Stop-Follow $follow
        }
    }

    It 'holds a line until it ends' {
        $f = New-TestFile 'part.log' "x`n"
        $follow = Start-Follow "Get-TrexLine -Path '$f' -Tail 1 -Follow"
        try {
            Wait-Follow $follow 1
            Add-TestText $f 'w'
            Start-Sleep -Seconds 3
            $follow.Output.Count | Should -Be 1
            Add-TestText $f "`n"
            Wait-Follow $follow 2
            $follow.Output[1].Text | Should -Be 'w'
            $follow.Output[1].LineNumber | Should -Be 2
        } finally {
            Stop-Follow $follow
        }
    }

    It 'writes each match a followed file gains at its own place' {
        $f = New-TestFile 'scan.log' "a 1`n"
        $follow = Start-Follow "Select-TrexMatch '\N' -Path '$f' -Tail 1 -Follow"
        try {
            Wait-Follow $follow 1
            $follow.Output[0].Text | Should -Be '1'
            $follow.Output[0].LineNumber | Should -Be 1
            Add-TestText $f "b 22`n"
            Wait-Follow $follow 2
            $follow.Output[1].Text | Should -Be '22'
            $follow.Output[1].LineNumber | Should -Be 2
            $follow.Output[1].Start | Should -Be 6
        } finally {
            Stop-Follow $follow
        }
    }

    It 'writes a followed file''s JSON one object a string, as the trex command prints them' {
        $f = New-TestFile 'json.log' "a 1`n"
        $follow = Start-Follow "Select-TrexMatch '\N' -Path '$f' -Follow -Json"
        try {
            Wait-Follow $follow 1
            Add-TestText $f "b 22`n"
            Wait-Follow $follow 2
            $follow.Output[0] | Should -BeExactly '{"start":2,"end":3,"text":"1","captures":{}}'
            $follow.Output[1] | Should -BeExactly '{"start":6,"end":8,"text":"22","captures":{}}'
        } finally {
            Stop-Follow $follow
        }
    }

    It 'ends a follow once -MaxCount has written all it will' {
        $f = New-TestFile 'cap.log' "a 1`n"
        $follow = Start-Follow "Select-TrexMatch '\N' -Path '$f' -Follow -MaxCount 2"
        try {
            Wait-Follow $follow 1
            Add-TestText $f "b 2`nc 3`n"
            $follow.Handle.AsyncWaitHandle.WaitOne(30000) | Should -BeTrue
            $follow.Shell.InvocationStateInfo.State | Should -Be 'Completed'
            Get-Items $follow.Output | ForEach-Object Text | Should -Be @('1', '2')
        } finally {
            $follow.Shell.Dispose()
        }
    }

    It 'writes a followed file''s rewritten and masked lines as it grows' {
        $f = New-TestFile 'edit.log' "a 1`n"
        $edit = Start-Follow "Edit-TrexText '\N:n' '<`${n}>' -Path '$f' -Follow"
        $mask = Start-Follow "Protect-TrexText '\N' -Path '$f' -Tail 1 -Follow"
        try {
            Wait-Follow $edit 1
            Wait-Follow $mask 1
            $edit.Output[0] | Should -BeExactly 'a <1>'
            $mask.Output[0] | Should -BeExactly 'a *'
            Add-TestText $f "b 22`n"
            Wait-Follow $edit 2
            Wait-Follow $mask 2
            $edit.Output[1] | Should -BeExactly 'b <22>'
            $mask.Output[1] | Should -BeExactly 'b **'
        } finally {
            Stop-Follow $edit
            Stop-Follow $mask
        }
    }

    It 'writes each finding a followed file gains' {
        $rules = New-TestFile 'follow.trex' "rule todo note `"a TODO left`" = `"TODO`"`n"
        $f = New-TestFile 'follow.conf' "TODO one`nfine`n"
        $follow = Start-Follow "Invoke-TrexRule -Path '$f' -RuleFile '$rules' -Follow"
        try {
            Wait-Follow $follow 1
            $follow.Output[0].Region.Line | Should -Be 1
            Add-TestText $f "TODO two`n"
            Wait-Follow $follow 2
            $follow.Output[1].Region.Line | Should -Be 3
            $follow.Output[1].Start | Should -Be 14
        } finally {
            Stop-Follow $follow
        }
    }

    It 'refuses what cannot be followed, with the reason' {
        $f = New-TestFile 'refused.log' $Five
        { Select-TrexMatch '\N' -Path $f -Head 3 -Follow -ErrorAction Stop } | Should -Throw '*ends before the file does*'
        { Select-TrexMatch '\N' -Path $f -Follow -Count -ErrorAction Stop } | Should -Throw '*a followed file never ends*'
        { Select-TrexMatch '\N' -Path $f -Follow -Index -ErrorAction Stop } | Should -Throw '*summarizes whole files*'
        $rules = New-TestFile 'refused.trex' "rule todo note `"a TODO left`" = `"TODO`"`n"
        { Invoke-TrexRule -Path $f -RuleFile $rules -Follow -Sarif -ErrorAction Stop } | Should -Throw '*a followed file never ends*'
        $a = New-TestFile 'one.log' "a`n"
        $b = New-TestFile 'two.log' "b`n"
        { Edit-TrexText '\W' 'X' -Path $a, $b -Follow -ErrorAction Stop } | Should -Throw '*one file*'
    }
}
