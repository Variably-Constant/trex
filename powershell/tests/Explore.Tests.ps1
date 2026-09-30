BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Get-TrexToken' {
    It 'writes each significant token with its kind and value' {
        $toks = Get-TrexToken 'retry 3 times in 1500ms'
        $toks.Kind | Should -Be @('word', 'number', 'word', 'word', 'duration')
        $toks[1].Value | Should -Be 3
        $toks[4].Value | Should -BeOfType ([TimeSpan])
        $toks[4].Value.TotalMilliseconds | Should -Be 1500
    }

    It 'names a declared shape by its name' {
        Unregister-TrexAtom -All
        Register-TrexAtom ticket -Shape '[A-Z]{2,4}-\d{1,4}'
        (Get-TrexToken 'see AB-12').Kind | Should -Be @('word', 'ticket')
        Unregister-TrexAtom -All
    }

    It 'writes the whitespace with -IncludeWhitespace' {
        (Get-TrexToken 'a b' -IncludeWhitespace).Count | Should -Be 3
    }
}

Describe 'Get-TrexRecord' {
    It 'splits text into the records a unit names' {
        $recs = Get-TrexRecord "a 1`nb 2`n`nc 3`n" -Unit paragraph
        @($recs).Count | Should -Be 2
    }

    It 'refuses a unit it does not know' {
        { Get-TrexRecord 'x' -Unit nothing -ErrorAction Stop } | Should -Throw
    }
}

Describe 'Get-TrexRecordShape' {
    It 'writes each record shape once with its count, most frequent first' {
        $shapes = Get-TrexRecordShape "a 1`na 2`na 3`nb x y`n"
        $shapes[0].Count | Should -Be 3
        $shapes[0].Pattern | Should -Not -BeNullOrEmpty
        $shapes[0].Novel | Should -BeNullOrEmpty
    }

    It 'reads every string piped in as one stream' {
        $shapes = 'a 1', 'a 2', 'a 3', 'b x y' | Get-TrexRecordShape
        $shapes[0].Count | Should -Be 3
        $shapes[0].Records | Should -Be @(0, 1, 2)
    }

    It 'reads the records -Unit names in place of lines' {
        $paragraphs = Get-TrexRecordShape "a 1`na 2`n`nb x`n" -Unit paragraph
        @($paragraphs).Count | Should -Be 2
        $paragraphs | ForEach-Object Count | Should -Be @(1, 1)
    }

    It 'marks a shape novel where the other input holds none that accepts its records' {
        $old = Join-Path $TestDrive 'old.log'
        Write-TrexTestFile -Path $old -Text "GET /a 200`nGET /b 404`n"
        $new = Join-Path $TestDrive 'new.log'
        Write-TrexTestFile -Path $new -Text "GET /c 200`npanic at 0x1f`n"
        $shapes = Get-TrexRecordShape -Path $new -Against $old
        @($shapes | Where-Object Novel -EQ $true).Count | Should -Be 1
        @($shapes | Where-Object Novel -EQ $false).Count | Should -Be 1
        (Get-TrexRecordShape -Path $new -Against $old -Novel).Readable | Should -BeLike '*panic*'
        { Get-TrexRecordShape -Path $new -Novel -ErrorAction Stop } | Should -Throw '*give -Against*'
    }
}

Describe 'ConvertTo-TrexPattern' {
    BeforeAll {
        $script:Titles = @(
            '2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB5031354)'
            '2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)'
            'Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)'
        )
        $script:MarkedTitle = '{month:2023-10} Cumulative Update for Windows {[int]os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})'
    }

    It 'infers a pattern every example matches' {
        $pattern = 'GET /a 200', 'POST /b 404' | ConvertTo-TrexPattern
        $pattern | Should -BeOfType ([string])
        Test-TrexMatch $pattern 'GET /a 200' | Should -BeTrue
        Test-TrexMatch $pattern 'POST /b 404' | Should -BeTrue
    }

    It 'builds a pattern for the fields a marked line names, over every shape' {
        $built = $Titles | ConvertTo-TrexPattern -Marked $MarkedTitle
        $built.PSObject.TypeNames[0] | Should -Be 'Trex.BuiltPattern'
        "$built" | Should -BeExactly $built.Pattern
        $built.Fields.Name | Should -Be @('month', 'os', 'version', 'kb')
        $built.Fields[1].TypeName | Should -Be 'int'
        $built.Format | Should -BeExactly '${month}\t${os}\t${version}\t${kb}'
        $built.Rows[2].Values.month | Should -BeNullOrEmpty
        $built.Rows[2].Values.version | Should -Be '1607'
        $built.Rows[1].Values.version | Should -BeNullOrEmpty
        @($built.Shapes).Count | Should -Be 2
        @(Select-TrexMatch $built.Pattern -InputObject ($Titles -join "`n") | ForEach-Object { $_.Captures.kb }) |
            Should -Be @('KB5031354', 'KB4534310', 'KB4103720')
    }

    It 'names a field by value, an ordered dictionary keeping its order' {
        ($Titles | ConvertTo-TrexPattern -Field @{ kb = 'KB5031354' }).Rows.Values.kb |
            Should -Be @('KB5031354', 'KB4534310', 'KB4103720')
        ($Titles | ConvertTo-TrexPattern -Field ([ordered]@{ kb = 'KB5031354'; os = '11' })).Fields.Name |
            Should -Be @('kb', 'os')
        { 'a 1 b 1' | ConvertTo-TrexPattern -Field @{ n = '1' } -ErrorAction Stop } | Should -Throw '*more than once*'
    }

    It 'spells a word field whose values share a constant part as its byte shape, unless -NoMint' {
        $minted = $Titles | ConvertTo-TrexPattern -Marked $MarkedTitle
        $plain = $Titles | ConvertTo-TrexPattern -Marked $MarkedTitle -NoMint
        $minted.Pattern.Contains('(`KB[0-9]{7}`):kb') | Should -BeTrue
        $plain.Pattern.Contains('(\W):kb') | Should -BeTrue
        $near = '2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB50313541)'
        @($near | ConvertFrom-TrexText $minted).Count | Should -Be 0
        @($near | ConvertFrom-TrexText $plain).kb | Should -Be 'KB50313541'
        @($Titles | ConvertFrom-TrexText -Marked $MarkedTitle -NoMint).kb | Should -Be @('KB5031354', 'KB4534310', 'KB4103720')
        $shaped = $Titles | ConvertTo-TrexPattern -Marked $MarkedTitle -MintShapes
        $shaped.Declarations | Should -Be @('shape kb = `KB[0-9]{7}`')
        $shaped.Pattern.Contains('(\{kb}):kb') | Should -BeTrue
        @($Titles | ConvertFrom-TrexText $shaped).kb | Should -Be @('KB5031354', 'KB4534310', 'KB4103720')
        @($Titles | ConvertFrom-TrexText -Marked $MarkedTitle -MintShapes).kb | Should -Be @('KB5031354', 'KB4534310', 'KB4103720')
        { $Titles | ConvertTo-TrexPattern -Marked $MarkedTitle -NoMint -MintShapes -ErrorAction Stop } | Should -Throw '*opposite spellings*'
    }

    It 'suggests a library value class until a -NotExample needs it' {
        ('WARN disk 7' | ConvertTo-TrexPattern -Marked '{level:ERROR} disk {n:5}').Suggestions | Should -Be @('level: \{log_level}')
        $refused = 'WARN disk 7' | ConvertTo-TrexPattern -Marked '{level:ERROR} disk {n:5}' -NotExample 'HELLO disk 9'
        $refused.Pattern.Contains('(\{log_level}):level') | Should -BeTrue
        @($refused.Suggestions).Count | Should -Be 0
    }

    It 'reads marks inside the examples with -MarksInLines' {
        $built = 'GET /a {status:200}', 'GET /b 204' | ConvertTo-TrexPattern -MarksInLines -NotExample 'GET /c 500'
        $built.Pattern | Should -BeLike '*\N{200..299}*'
        $built.Rows.Values.status | Should -Be @('200', '204')
    }
}

Describe 'ConvertFrom-TrexText' {
    It 'writes an object per line a built pattern reads, typed as its marks say' {
        $titles = @(
            '2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB5031354)'
            'Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)'
        )
        $built = $titles | ConvertTo-TrexPattern -Marked '{month:2023-10} Cumulative Update for Windows {[int]os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})'
        $objects = @('2023-11 Cumulative Update for Windows 11 Version 23H2 for x64-based Systems (KB5032190)', 'no title here' |
                ConvertFrom-TrexText -Pattern $built)
        $objects.Count | Should -Be 1
        $objects[0].PSObject.Properties.Name | Should -Be @('month', 'os', 'version', 'kb')
        $objects[0].os | Should -BeOfType ([int])
        $objects[0].os | Should -Be 11
        $objects[0].version | Should -Be '23H2'
        $objects[0].kb | Should -Be 'KB5032190'
    }

    It 'builds from marked lines and converts every line piped in, as ConvertFrom-String -TemplateContent does' {
        $stock = 'apples 42', 'pears 7', 'plums 1300' | ConvertFrom-TrexText -Marked '{fruit:apples} {[int]qty:42}'
        $stock.fruit | Should -Be @('apples', 'pears', 'plums')
        $stock.qty | Should -Be @(42, 7, 1300)
    }

    It 'writes an object per record a starred field begins, the lines after it joining it across strings' {
        $template = "Name: {Name*:Phoebe Cat}`r`nPhone: {phone:425-123-6789}`r`n`r`nName: {Name*:Lucky Shot}`r`nPhone: {phone:206-987-4321}"
        $text = "Phone: 111-222-3333`nName: Phoebe Cat`nPhone: 425-123-6789`nPhone: 425-000-1111`n`nName: Lucky Shot`n`nName: Elephant Wise`nPhone: 425-888-7766"
        foreach ($given in @(, $text), ($text -split "`n")) {
            $pets = @($given | ConvertFrom-TrexText -Marked $template)
            $pets.Name | Should -Be @('Phoebe Cat', 'Lucky Shot', 'Elephant Wise')
            $pets[0].phone | Should -Be '425-123-6789'
            $pets[1].PSObject.Properties.Name | Should -Contain 'phone'
            $pets[1].phone | Should -BeNullOrEmpty
        }
        $built = $text -split "`n" | ConvertTo-TrexPattern -Marked $template
        $built.Fields.StartsRecord | Should -Be @($true, $false)
        @($built.Records).Count | Should -Be 5
        @($text -split "`n" | ConvertFrom-TrexText $built).Count | Should -Be 3
    }

    It 'casts and assembles records as ConvertFrom-String does' -Skip:($PSVersionTable.PSEdition -ne 'Desktop') {
        $template = "{[int]i*:6} {[long]l:70000000000} {[decimal]d:1.5} {[double]f:2.25} {[bool]b:true} {[datetime]t:2023-10-01} {[string]s:abc}`n" +
            '{[int]i*:12} {[long]l:8} {[decimal]d:3.25} {[double]f:4.5} {[bool]b:false} {[datetime]t:2024-01-15} {[string]s:xyz}'
        $text = "6 70000000000 1.5 2.25 true 2023-10-01 abc`n12 8 3.25 4.5 false 2024-01-15 xyz`n40 9 7.75 1.125 false 2025-02-03 qq"
        $cfs = @($text | ConvertFrom-String -TemplateContent $template)
        $trex = @($text | ConvertFrom-TrexText -Marked $template)
        $trex.Count | Should -Be $cfs.Count
        for ($i = 0; $i -lt $cfs.Count; $i++) {
            foreach ($p in $cfs[$i].PSObject.Properties) {
                $mine = $trex[$i].($p.Name)
                $mine | Should -Be $p.Value -Because "$($p.Name) of record $i"
                $mine.GetType() | Should -Be $p.Value.GetType() -Because "$($p.Name) of record $i"
            }
        }
    }

    It 'writes a field marked twice in a line as an array of every value it holds, each cast' {
        $built = 'from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok', 'from 10.0.0.3 ok' |
            ConvertTo-TrexPattern -Marked 'from {ip:10.0.0.1} -> {ip:10.0.0.2} ok'
        $built.Fields.List | Should -Be @($true)
        $built.Format | Should -BeExactly '${ip[*]}'
        $row = $built.Rows | Where-Object Text -EQ 'from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok'
        $row.Values.ip | Should -Be @('10.0.0.7', '10.0.0.8', '10.0.0.9')
        $hops = @('from 1.1.1.1 -> 2.2.2.2 ok', 'from 3.3.3.3 ok' | ConvertFrom-TrexText $built)
        $hops[0].ip | Should -Be @('1.1.1.1', '2.2.2.2')
        , $hops[1].ip | Should -BeOfType ([object[]])
        $hops[1].ip | Should -Be @('3.3.3.3')
        $ports = @('ports 22 open', 'ports 8080, 8443, 9000 open' |
                ConvertFrom-TrexText -Marked 'ports {[int]port:80}, {[int]port:443} open')
        $ports[1].port | Should -Be @(8080, 8443, 9000)
        $ports[1].port[0] | Should -BeOfType ([int])
        $ports[0].port | Should -Be @(22)
        (ConvertFrom-TrexText '(\W:k "=" \N:v ";")+' 'a = 1; b = 2;').k | Should -Be @('a', 'b')
    }

    It 'writes an object per record a line repeats, each carrying the line''s other fields, as ConvertFrom-String does' {
        $template = 'day {day:Mon}: {Name*:Phoebe Cat} ({[int]age:6}); {Name*:Lucky Shot} ({[int]age:12}) end'
        $people = @('day Tue: Wise Owl (87) end', 'day Wed: Elmo Red (3); Oscar Grouch (9) end' | ConvertFrom-TrexText -Marked $template)
        $people.Name | Should -Be @('Wise Owl', 'Elmo Red', 'Oscar Grouch')
        $people.day | Should -Be @('Tue', 'Wed', 'Wed')
        $people[2].age | Should -BeOfType ([int])
        $people[2].age | Should -Be 9
        $built = 'Wise Owl, 87; Big Bird, 5' | ConvertTo-TrexPattern -Marked '{Name*:Phoebe Cat}, {[int]age:6}; {Name*:Lucky Shot}, {[int]age:12}'
        $built.Fields.Repeats | Should -Be @($true, $true)
        @('Elmo Red, 3; Oscar Grouch, 9' | ConvertFrom-TrexText $built).Name | Should -Be @('Elmo Red', 'Oscar Grouch')
    }

    It 'writes a mark inside a mark as a property of an object holding the outer text, each cast' {
        $pages = @('5 of 9', '6 of 9' | ConvertFrom-TrexText -Marked '{Line:{[int]n:1} of {[int]m:3}}')
        $pages.Count | Should -Be 2
        $pages[0].PSObject.Properties.Name | Should -Be @('Line')
        $pages[0].Line.Text | Should -Be '5 of 9'
        $pages[0].Line.n | Should -BeOfType ([int])
        $pages[0].Line.m | Should -Be 9
        $built = '5 of 9' | ConvertTo-TrexPattern -Marked '{Line:{[int]n:1} of {[int]m:3}}'
        $built.Fields.Parent | Should -Be @($null, 'Line', 'Line')
        $built.Rows[1].Values.Line.Text | Should -Be '5 of 9'
        $built.Rows[1].Values.Line.n | Should -Be '5'
        $pair = ConvertFrom-TrexText '(\W:k "=" \N:v):pair' 'x = 1'
        $pair.pair.Text | Should -Be 'x = 1'
        $pair.pair.v | Should -Be '1'
    }

    It 'writes a starred mark spanning lines as an object per record of the lines it spans' {
        $template = "{Person*:Name: {Name:Phoebe Cat}`r`nPhone: {Phone:425-123-6789}}"
        $people = @("Name: Wise Owl`nPhone: 425-888-7766`nName: Big Bird`nPhone: 206-555-0100" | ConvertFrom-TrexText -Marked $template)
        $people.Count | Should -Be 2
        $people[0].PSObject.Properties.Name | Should -Be @('Person')
        $people.Person.Name | Should -Be @('Wise Owl', 'Big Bird')
        $people.Person.Phone | Should -Be @('425-888-7766', '206-555-0100')
        $people[1].Person.Text | Should -BeExactly "Name: Big Bird`nPhone: 206-555-0100"
        $built = 'Name: Wise Owl', 'Phone: 425-888-7766' | ConvertTo-TrexPattern -Marked $template
        $built.Fields.StartsRecord | Should -Be @($false, $true, $false)
        $built.Fields.Parent | Should -Be @($null, 'Person', 'Person')
    }

    It 'reads a pattern''s registers as the fields and a part of a token through its accessor' {
        (ConvertFrom-TrexText '\W:verb \N:code' 'GET 200').code | Should -Be '200'
        $hosts = 'GET https://example.com/a 200', 'GET https://trex.dev/b 404' |
            ConvertFrom-TrexText -Marked 'GET https://{host:example.com}/a 200'
        $hosts.host | Should -Be @('example.com', 'trex.dev')
    }
}

Describe 'ConvertTo-TrexLiteral' {
    It 'escapes a text so a pattern matches it literally' {
        $literal = ConvertTo-TrexLiteral 'x (y)'
        (Select-TrexMatch $literal -InputObject 'a x (y) b').Text | Should -BeExactly 'x (y)'
    }
}
