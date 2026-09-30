BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Token grammars' {
    BeforeAll {
        $arithSource = @(
            'expr   := <expr> "+" <term> | <expr> "-" <term> | <term>'
            'term   := <term> "*" <factor> | <term> "/" <factor> | <factor>'
            'factor := number | ident | "(" <expr> ")"'
        ) -join "`n"
        $arith = New-TrexGrammar $arithSource
        $ambiguous = New-TrexGrammar 'expr := <expr> "+" <expr> | number'
        $weighted = New-TrexGrammar 'expr := <expr> "+" <expr> @0.5 | number @0.5'
        $words = New-TrexGrammar 's := <s> ident | ident'
    }

    It 'compiles a grammar and names its rules and the rule it starts from' {
        $arith | Should -BeOfType ([Trex.Grammar])
        $arith.Start | Should -BeExactly 'expr'
        $arith.Rules | Should -Be @('expr', 'term', 'factor')
        (New-TrexGrammar $arithSource -Start term).Start | Should -BeExactly 'term'
    }

    It 'parses with the precedence the rules layer' {
        $tree = Invoke-TrexGrammar $arith '2 + 3 * 4'
        $tree | Should -BeOfType ([Trex.ParseNode])
        $tree.Expression | Should -BeExactly '(expr 2 + (term 3 * 4))'
        $tree.Rule | Should -BeExactly 'expr'
        $tree.Text | Should -BeExactly '2 + 3 * 4'
        $tree.Children.Count | Should -Be 3
        $tree.Children[1].Text | Should -BeExactly '+'
        $arith.Parse('2 - 3 - 4').Expression | Should -BeExactly '(expr (expr 2 - 3) - 4)'
    }

    It 'reports text that does not parse in full as an error' {
        { Invoke-TrexGrammar $arith '2 +' -ErrorAction Stop } | Should -Throw '*does not parse in full*'
        $arith.Parse('2 +') | Should -BeNullOrEmpty
        $arith.Test('2 +') | Should -BeFalse
        $arith.Test('2 + 3') | Should -BeTrue
    }

    It 'counts the derivations of an ambiguous grammar, the Catalan numbers' {
        '1', '1 + 2 + 3', '1 + 2 + 3 + 4', '1 + 2 + 3 + 4 + 5' | Invoke-TrexGrammar $ambiguous -Count |
            Should -Be @(1, 2, 5, 14)
        $ambiguous.CountParses('1 +') | Should -Be 0
    }

    It 'scores the best derivation and all of them under the weights' {
        Invoke-TrexGrammar $weighted '1 + 2 + 3' -Best | Should -Be 0.03125
        Invoke-TrexGrammar $weighted '1 + 2 + 3' -Probability | Should -Be 0.0625
        $weighted.TotalProbability('1 + 2') | Should -Be 0.125
        $weighted.BestProbability('1') | Should -Be 0.5
    }

    It 'counts the tilings of a run-together string the grammar accepts' {
        Invoke-TrexGrammar $words -Segment 'abab' -Dictionary a, b, ab -Count | Should -Be 4
        $words.CountSegmentations('abc', @('a', 'b', 'ab')) | Should -Be 0
        $onlyAb = New-TrexGrammar 's := "ab" <s> | "ab"'
        Invoke-TrexGrammar $onlyAb -Segment 'abab' -Dictionary a, b, ab -Count | Should -Be 1
        Invoke-TrexGrammar $onlyAb -Segment 'abab' -Dictionary a, b, ab -Best | Should -BeGreaterThan 0
    }

    It 'takes grammar source text in place of a Trex.Grammar' {
        (Invoke-TrexGrammar 'sum := <sum> "+" number | number' '1 + 2 + 3').Expression |
            Should -BeExactly '(sum (sum 1 + 2) + 3)'
    }

    It 'refuses a grammar that does not parse, naming the line, and a start it does not declare' {
        { New-TrexGrammar "a := number`nnot a rule" -ErrorAction Stop } | Should -Throw '*line 2*'
        { New-TrexGrammar 'a := number' -Start b -ErrorAction Stop } | Should -Throw '*declares no rule*'
        { Invoke-TrexGrammar $arith '1' -Count -Best -ErrorAction Stop } | Should -Throw '*give one*'
    }

    It 'constructs from script as [Trex.Grammar]::new' {
        [Trex.Grammar]::new('n := number').Test('7') | Should -BeTrue
    }
}
