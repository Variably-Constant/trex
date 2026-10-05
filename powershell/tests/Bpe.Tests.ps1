BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Byte-pair encoders' {
    BeforeAll {
        # Ten rounds over this corpus, each merging the most frequent pair
        # and the smaller pair on a tie: l+o, lo+w, e+s, es+t, est+</w>,
        # low+</w>, e+w, ew+est</w>, n+ewest</w>, e+r.
        $corpus = 'low low low low lower lower newest newest newest widest'
        $bpe = $corpus | New-TrexBpe -Merges 10
    }

    It 'learns merges from a corpus and splits words into its subwords' {
        $bpe | Should -BeOfType ([Trex.Bpe])
        $bpe.Merges | Should -Be 10
        ConvertTo-TrexBpe $bpe 'lowest' | Should -Be @('low', 'est</w>')
        $bpe.Encode('low newest') | Should -Be @('low</w>', 'newest</w>')
    }

    It 'writes a model that reads back as the same encoder' {
        $file = Join-Path $TestDrive 'corpus.bpe'
        $bpe | Export-TrexBpe $file
        $read = Import-TrexBpe $file
        $read.Merges | Should -Be 10
        $read.Model | Should -BeExactly $bpe.Model
        $read.Encode('lower') | Should -Be $bpe.Encode('lower')
    }

    It 'refuses a model line with no tab, naming its number' {
        $bad = Join-Path $TestDrive 'bad.bpe'
        Write-TrexTestFile -Path $bad -Text "l`to`nnot a merge`n"
        { Import-TrexBpe $bad -ErrorAction Stop } | Should -Throw '*line 2*'
        { [Trex.Bpe]::new("l`to`nnot a merge") } | Should -Throw '*line 2*'
    }

    It 'constructs from a model''s text as [Trex.Bpe]::new' {
        $one = [Trex.Bpe]::new("l`to`n")
        $one.Merges | Should -Be 1
        $one.Encode('lo') | Should -Be @('lo', '</w>')
    }
}
