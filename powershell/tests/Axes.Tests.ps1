BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Measure-TrexMagnitude' {
    It 'marks the jumps of more than three orders and the peak' {
        # log10: 0, 6, 0.3, so both steps move more than three orders.
        $r = Measure-TrexMagnitude '1 1000000 2'
        $r.Tokens | Should -Be 3
        $r.Peak.Text | Should -BeExactly '1000000'
        $r.Peak.Offset | Should -Be 2
        $r.Jumps.Text | Should -Be @('1000000', '2')
        $r.Frames.Count | Should -Be 0
    }

    It 'shows a frame inside its report as its text and where it stands' {
        $r = Measure-TrexMagnitude '1 1000000 2'
        "$($r.Peak)" | Should -BeExactly '1000000 at 2'
        ($r | Format-List Peak | Out-String) | Should -Match 'Peak : 1000000 at 2'
        "$((Measure-TrexStress '[1, [2]]').Peaks[0])" | Should -Match '^\S+ at \d+, depth \d+$'
    }

    It 'adds every token frame with -Detail' {
        (Measure-TrexMagnitude '1 1000000 2' -Detail).Frames.Text | Should -Be @('1', '1000000', '2')
    }

    It 'lexes with the atoms in force' {
        Unregister-TrexAtom -All
        Register-TrexAtom ticket -Shape '[A-Z]{2,4}-\d{1,4}'
        $r = Measure-TrexMagnitude 'see AB-12' -Detail
        $r.Frames.Text | Should -Be @('see', 'AB-12')
        Unregister-TrexAtom -All
    }
}

Describe 'Measure-TrexStress' {
    It 'reads the depth, the load and where a nested structure begins to close' {
        $r = Measure-TrexStress '{"a": [1, {"b": [2, 3]}]}'
        $r.MaxDepth | Should -Be 4
        # Open spans at the innermost 3: 12 + 9 + 6 + 3 tokens.
        $r.PeakLoad.Load | Should -Be 30
        $r.Fractures.Offset | Should -Be @(21)
        $r.Fractures[0].Text | Should -BeExactly ']'
        $r.Peaks.Text | Should -Contain '2'
    }

    It 'reads a file named with -Path and names it in the report' {
        $file = Join-Path $TestDrive 'nested.json'
        Write-TrexTestFile -Path $file -Text '[[[1]]]'
        $r = Measure-TrexStress -Path $file
        $r.Path | Should -BeExactly $file
        $r.MaxDepth | Should -Be 3
    }

    It 'reads files piped from Get-ChildItem' {
        Write-TrexTestFile -Path (Join-Path $TestDrive 'a.txt') -Text '(x)'
        Write-TrexTestFile -Path (Join-Path $TestDrive 'b.txt') -Text '((x))'
        $reports = Get-ChildItem $TestDrive -Filter '?.txt' | Measure-TrexStress
        ($reports | Sort-Object Path).MaxDepth | Should -Be @(1, 2)
    }
}

Describe 'Measure-TrexFlow' {
    It 'reads the trend of the magnitude and where it turns' {
        $r = Measure-TrexFlow '1 10 100 1000 10 1'
        $r.Signal | Should -Be 'Magnitude'
        $r.PeakMomentum.Direction | Should -Be 'Rising'
        $r.Reversals.Count | Should -BeGreaterThan 0
        $r.Reversals[0].Offset | Should -BeGreaterThan 9
    }

    It 'reads the signal -Signal names' {
        (Measure-TrexFlow 'a bb ccc dddd' -Signal Length).Signal | Should -Be 'Length'
    }
}

Describe 'Measure-TrexObservation' {
    It 'contests the border between a uniform run and a mixed one' {
        $text = ('a' * 40) + 'a1 b2 c3 d4 e5 f6 g7 h8 i9 j0 k1 l2 m3'
        $r = Measure-TrexObservation $text
        $r.Contested.Count | Should -BeGreaterThan 0
        $r.Peak.Disagreement | Should -BeGreaterOrEqual 0.2
        $r.Length | Should -Be $text.Length
    }

    It 'adds a frame a byte with -Detail' {
        (Measure-TrexObservation 'abc 123' -Detail).Frames.Count | Should -Be 7
    }
}

Describe 'Measure-TrexEcho' {
    It 'counts what recurs, and the period of a regular recurrence' {
        # alpha at 0, 11 and 23: lags of 11 and 12, a period of 11.5 bytes.
        $r = Measure-TrexEcho 'alpha beta alpha gamma alpha'
        $r.Keyed | Should -Be 5
        $r.Distinct | Should -Be 3
        $r.Echoed | Should -Be 3
        $r.Echoes[0].Key | Should -BeExactly 'alpha'
        $r.Echoes[0].Count | Should -Be 3
        $r.Echoes[0].Offset | Should -Be 0
        $r.Echoes[0].Period | Should -Be 11.5
    }

    It 'points each occurrence at the one before with -Detail' {
        $frames = (Measure-TrexEcho 'alpha beta alpha gamma alpha' -Detail).Frames
        $last = $frames | Where-Object Offset -eq 23
        $last.Nth | Should -Be 3
        $last.Previous | Should -Be 11
        ($frames | Where-Object Offset -eq 0).Previous | Should -BeNullOrEmpty
    }

    It 'reads keys under the group -Group names' {
        $r = Measure-TrexEcho 'Error ERROR error' -Group case
        $r.Distinct | Should -Be 1
        $r.Echoes[0].Count | Should -Be 3
    }

    It 'adds the structures that recur with -Structure' {
        $text = "alpha: one`nbravo: two`ncharlie: three`ndelta: four`n"
        (Measure-TrexEcho $text).Structures.Count | Should -Be 0
        (Measure-TrexEcho $text -Structure).Structures.Count | Should -BeGreaterThan 0
    }

    It 'refuses a group it does not know' {
        { Measure-TrexEcho 'a a' -Group nothing -ErrorAction Stop } | Should -Throw '*names no group*'
    }
}

Describe 'Measure-TrexOrbit' {
    It 'folds the forms of one orbit under the group' {
        $r = Measure-TrexOrbit 'Cat cat CAT dog' -Group case
        $r.Tokens | Should -Be 4
        $r.Forms | Should -Be 4
        $r.Orbits | Should -Be 2
        ($r.Classes | Where-Object { $_.Forms.Count -eq 3 }).Forms | Should -Be @('CAT', 'Cat', 'cat')
    }

    It 'lists the tokens in the orbit of -SameAs' {
        (Measure-TrexOrbit 'Cat cat CAT dog' -Group case -SameAs cAt).Matches.Text | Should -Be @('Cat', 'cat', 'CAT')
    }

    It 'reads consonant and vowel shapes by default' {
        $r = Measure-TrexOrbit 'cat dog bat'
        $r.Group | Should -BeExactly 'shape'
        $r.Orbits | Should -Be 1
    }

    It 'covers the input with its runs of one kind with -Detail' {
        $r = Measure-TrexOrbit 'ab 12 cd' -Detail
        ($r.Segments.Text -join '') | Should -BeExactly 'ab 12 cd'
        $r.Frames.Count | Should -Be 3
    }
}

Describe 'Measure-TrexShape' {
    It 'finds the period of a repeated record' {
        $text = (1..40 | ForEach-Object { "alpha $_ beta" }) -join "`n"
        $r = Measure-TrexShape $text
        $r.DominantPeriod | Should -Be 3
        $r.Regions.Count | Should -BeGreaterThan 0
    }
}

Describe 'Measure-TrexRelation' {
    It 'writes the alpha-equivalence form with -Canonical' {
        $r = Measure-TrexRelation '[ x ( x ) ]' -Canonical
        $r.Canonical | Should -BeExactly '[ # ( ^0 ) ]'
        $r.MaxDepth | Should -Be 2
        $r.BoundaryClosed | Should -BeTrue
    }

    It 'reads an unbalanced bracket as an open boundary' {
        (Measure-TrexRelation '( x').BoundaryClosed | Should -BeFalse
    }

    It 'adds the edges with -Detail' {
        $r = Measure-TrexRelation 'f(a, b)' -Detail
        ($r.Edges | Where-Object Kind -eq 'Encloses').Count | Should -Be $r.Encloses
        ($r.Edges | Where-Object Kind -eq 'Adjacent').Count | Should -Be $r.Adjacencies
    }
}

Describe 'Measure-TrexSpectral' {
    It 'reads prose as prose and orders its entropy readings' {
        $text = 'the quick brown fox jumps over the lazy dog ' * 20
        $r = Measure-TrexSpectral $text
        $r.Timeline.Texture | Should -Contain 'Prose'
        $r.EntropyMinimum | Should -BeLessOrEqual $r.EntropyMean
        $r.EntropyMean | Should -BeLessOrEqual $r.EntropyMaximum
        ($r.Timeline | Measure-Object Length -Sum).Sum | Should -Be $text.Length
    }

    It 'classifies regions with -Classify' {
        $r = Measure-TrexSpectral ('the quick brown fox jumps over the lazy dog ' * 20) -Classify
        $r.Regions.Count | Should -BeGreaterThan 0
        $r.Regions[0].Kind | Should -BeOfType ([Trex.RegionKind])
    }
}

Describe 'Measure-TrexSeam' {
    It 'divides a spaceless text into segments that cover it' {
        $text = 'thequickbrownfoxjumpsoverthelazydog'
        $r = Measure-TrexSeam $text -English
        $r.Segments.Count | Should -BeGreaterThan 1
        ($r.Segments.Text -join '') | Should -BeExactly $text
    }

    It 'adds a frame a byte with -Detail' {
        (Measure-TrexSeam 'abc def' -Detail).Frames.Count | Should -Be 7
    }

    It 'refuses an order of 0' {
        { Measure-TrexSeam 'abc' -Order 0 -ErrorAction Stop } | Should -Throw '*1 or more*'
    }
}
