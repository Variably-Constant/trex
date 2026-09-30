BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-TrexModule
}

Describe 'Get-TrexFile' {
    BeforeAll {
        $tree = Join-Path $TestDrive 'walked'
        New-Item -ItemType Directory -Path $tree | Out-Null
        Write-TrexTestFile -Path (Join-Path $tree 'main.rs') -Text "fn main() { let x = 42; }`n"
        Write-TrexTestFile -Path (Join-Path $tree 'notes.md') -Text "the answer is 42`n"
        $bytes = [byte[]]::new(9000)
        [System.Random]::new(7).NextBytes($bytes)
        Write-TrexTestFile -Path (Join-Path $tree 'payload.b64') -Text ([Convert]::ToBase64String($bytes))
        $cache = New-Item -ItemType Directory -Path (Join-Path $tree '.cache')
        Write-TrexTestFile -Path (Join-Path $cache.FullName 'kept.txt') -Text "x`n"
        function Get-Leaf { process { Split-Path $_.Path -Leaf } }
    }

    It 'lists the files a walk finds, skipping hidden ones, with no texture read' {
        $files = Get-TrexFile $tree
        $files | Should -BeOfType ([Trex.File])
        $files | Get-Leaf | Sort-Object | Should -Be @('main.rs', 'notes.md', 'payload.b64')
        $files | Where-Object { $null -ne $_.Texture } | Should -BeNullOrEmpty
    }

    It 'lists hidden files with -Hidden' {
        Get-TrexFile $tree -Hidden | Get-Leaf | Should -Contain 'kept.txt'
    }

    It 'keeps the walked files of a type or a glob, and a file named outright whatever they say' {
        Get-TrexFile $tree -FileType rust | Get-Leaf | Should -Be 'main.rs'
        Get-TrexFile $tree -Include '*.md' | Get-Leaf | Should -Be 'notes.md'
        Get-TrexFile (Join-Path $tree 'notes.md') -FileType rust | Get-Leaf | Should -Be 'notes.md'
    }

    It 'reads each file''s texture with -Classify, and -Texture keeps the files read as one' {
        $read = Get-TrexFile $tree -Classify
        ($read | Where-Object Path -Like '*payload.b64').Texture | Should -Be ([Trex.RegionKind]::Blob)
        $blobs = $read | Where-Object Texture -EQ ([Trex.RegionKind]::Blob) | ForEach-Object Path
        (Get-TrexFile $tree -Texture Blob).Path | Should -Be $blobs
        Get-TrexFile $tree -ExcludeTexture Blob | Where-Object Path -Like '*payload.b64' | Should -BeNullOrEmpty
    }

    It 'orders the files with -Sort, and -Descending reverses them' {
        Get-TrexFile $tree -Sort Path | Get-Leaf | Should -Be @('main.rs', 'notes.md', 'payload.b64')
        Get-TrexFile $tree -Sort Path -Descending | Get-Leaf | Should -Be @('payload.b64', 'notes.md', 'main.rs')
        { Get-TrexFile $tree -Descending -ErrorAction Stop } | Should -Throw '*give -Sort*'
    }

    It 'pipes into the file cmdlets by its Path' {
        Get-TrexFile $tree -FileType rust | Get-Content | Should -Be 'fn main() { let x = 42; }'
    }
}

Describe 'Get-TrexFileType' {
    It 'lists ripgrep''s types with their globs' {
        $rust = Get-TrexFileType rust
        $rust | Should -BeOfType ([Trex.FileType])
        $rust.Globs | Should -Contain '*.rs'
        (Get-TrexFileType py*).Name | Should -Contain 'py'
    }
}
