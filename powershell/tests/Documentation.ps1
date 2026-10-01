# The documented examples: every `powershell` block in the README and the
# wiki whose first line begins `PS> `, read as a reader reads it and run
# against the module this build produced.
#
# A `PS> ` line begins a command, a `>> ` line continues it, and the lines
# under it up to the next command or the block's end are what it writes,
# formatted as the console formats it at a width of 100. A command that is
# only `Get-Content ./a/path` is a fixture rather than an example: the lines
# under it are that file's text, written where the page's examples run
# before any example below it, so each example reads the files a reader has
# been shown by the time they reach it. A `$ cat NAME` line in a `console`
# block is a fixture too, as tests/wiki_examples.rs reads one: the lines
# under it up to the next `$ ` line are the text of NAME. The files under
# tests/documented that a page's commands name are placed before any of
# them, as that test places them.
#
# The pages show the folder the examples run in as C:\Temp\demo. Wherever a
# run writes the folder it actually ran in, as a path, a GitHub annotation's
# file, a file URI or a JSON string, that folder is read as C:\Temp\demo
# before the output is compared.

$script:TrexDocumentedRoot = 'C:\Temp\demo'
$script:TrexDocumentedSeeds = [System.IO.Path]::GetFullPath((Join-Path (Join-Path (Join-Path $PSScriptRoot '..') '..') (Join-Path 'tests' 'documented')))

# The examples and fixtures of the page at $Path, in page order.
function Read-TrexDocumentedPage {
    param([Parameter(Mandatory)][string] $Path)
    $lines = [System.IO.File]::ReadAllLines($Path)
    $items = [System.Collections.Generic.List[object]]::new()
    $i = 0
    while ($i -lt $lines.Count) {
        if ($lines[$i] -ceq '```console') {
            $end = $i + 1
            while ($end -lt $lines.Count -and $lines[$end] -cne '```') { $end++ }
            $j = $i + 1
            while ($j -lt $end) {
                if (-not $lines[$j].StartsWith('$ cat ')) {
                    $j++
                    continue
                }
                $at = $j + 1
                $name = $lines[$j].Substring(6).Trim()
                if ([System.IO.Path]::IsPathRooted($name) -or @($name -split '[\\/]' | Where-Object { $_ -eq '..' -or $_ -eq '' }).Count -gt 0) {
                    throw "${Path}:${at}: `$ cat $name names a file outside the folder the examples run in"
                }
                $j++
                $shown = [System.Collections.Generic.List[string]]::new()
                while ($j -lt $end -and -not $lines[$j].StartsWith('$ ')) {
                    $shown.Add($lines[$j])
                    $j++
                }
                while ($shown.Count -gt 0 -and $shown[$shown.Count - 1].Trim() -eq '') { $shown.RemoveAt($shown.Count - 1) }
                $items.Add([pscustomobject]@{ Kind = 'Fixture'; Line = $at; File = "./$name"; Shown = [string[]]$shown })
            }
            $i = $end + 1
            continue
        }
        if ($lines[$i] -cne '```powershell') {
            $i++
            continue
        }
        $open = $i
        $end = $i + 1
        while ($end -lt $lines.Count -and $lines[$end] -cne '```') { $end++ }
        if ($end -ge $lines.Count) {
            throw "${Path}:$($open + 1): a powershell block with no closing fence"
        }
        $j = $open + 1
        while ($j -lt $end -and $lines[$j].Trim() -eq '') { $j++ }
        if ($j -lt $end -and $lines[$j].StartsWith('PS> ')) {
            while ($j -lt $end) {
                $at = $j + 1
                $command = $lines[$j].Substring(4)
                $j++
                while ($j -lt $end -and ($lines[$j] -ceq '>>' -or $lines[$j].StartsWith('>> '))) {
                    $continued = if ($lines[$j].Length -gt 3) { $lines[$j].Substring(3) } else { '' }
                    $command += "`n" + $continued
                    $j++
                }
                $shown = [System.Collections.Generic.List[string]]::new()
                while ($j -lt $end -and -not $lines[$j].StartsWith('PS> ')) {
                    $shown.Add($lines[$j])
                    $j++
                }
                if ($command -cmatch '^Get-Content (\./[^\s''"]+)$') {
                    $items.Add([pscustomobject]@{ Kind = 'Fixture'; Line = $at; File = $Matches[1]; Shown = (Get-TrexShownLines $shown) })
                } else {
                    $items.Add([pscustomobject]@{ Kind = 'Example'; Line = $at; Command = $command; Shown = (Get-TrexShownLines $shown) })
                }
            }
        }
        $i = $end + 1
    }
    $items
}

# The lines of an output with each one's trailing whitespace and the blank
# lines before the first and after the last taken off, which is how a page
# shows what a command wrote.
function Get-TrexShownLines {
    param([AllowNull()][AllowEmptyCollection()][string[]] $Lines)
    $kept = @(foreach ($line in $Lines) { $line.TrimEnd() })
    $first = 0
    while ($first -lt $kept.Count -and $kept[$first] -eq '') { $first++ }
    $last = $kept.Count - 1
    while ($last -ge $first -and $kept[$last] -eq '') { $last-- }
    if ($last -lt $first) {
        return , [string[]]@()
    }
    , [string[]]$kept[$first..$last]
}

# Each form the folder $Actual takes in output, paired with the same form of
# the folder the pages show.
function Get-TrexRootForms {
    param([Parameter(Mandatory)][string] $Actual)
    $shown = $script:TrexDocumentedRoot
    $slashed = $Actual.Replace('\', '/')
    $shownSlashed = $shown.Replace('\', '/')
    , @(
        , @($Actual, $shown)
        , @($Actual.Replace('\', '\\'), $shown.Replace('\', '\\'))
        , @((ConvertTo-TrexAnnotated $slashed), (ConvertTo-TrexAnnotated $shownSlashed))
        , @($slashed, $shownSlashed)
    )
}

# $Text as a GitHub workflow annotation writes a property's value.
function ConvertTo-TrexAnnotated {
    param([Parameter(Mandatory)][string] $Text)
    $Text.Replace('%', '%25').Replace(':', '%3A').Replace(',', '%2C')
}

# The lines of what a command wrote, $Written as formatted at a width of 100,
# with the folder it ran in, $Root, read as the one the pages show.
function ConvertTo-TrexShownOutput {
    param([AllowEmptyString()][string] $Written, [Parameter(Mandatory)][string] $Root)
    foreach ($form in (Get-TrexRootForms -Actual $Root)) {
        $Written = $Written.Replace($form[0], $form[1])
    }
    Get-TrexShownLines ($Written -split "`r?`n")
}

# Copies into $Root each file under tests/documented that a word of
# $Commands names, by its path under that folder or by its file name, or
# that lies under a folder a word names, as tests/wiki_examples.rs seeds a
# page. A word is read with its quotes, a leading `./` and a trailing `/`
# taken off.
function Copy-TrexDocumentedSeed {
    param([AllowEmptyCollection()][string[]] $Commands, [Parameter(Mandatory)][string] $Root)
    $words = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($command in $Commands) {
        foreach ($word in ($command -split '[\s,;()]+')) {
            $w = $word.Trim('''', '"')
            if ($w.StartsWith('./')) { $w = $w.Substring(2) }
            $w = $w.TrimEnd('/')
            if ($w -ne '') { [void]$words.Add($w) }
        }
    }
    $from = $script:TrexDocumentedSeeds
    foreach ($file in Get-ChildItem -LiteralPath $from -Recurse -File) {
        $named = $file.FullName.Substring($from.Length + 1).Replace('\', '/')
        $under = @($words | Where-Object { $named.StartsWith("$_/", [System.StringComparison]::Ordinal) }).Count -gt 0
        if (-not ($words.Contains($named) -or $words.Contains($file.Name) -or $under)) {
            continue
        }
        $to = Join-Path $Root $named
        $dir = Split-Path -Parent $to
        if (-not (Test-Path -LiteralPath $dir)) {
            New-Item -ItemType Directory -Path $dir | Out-Null
        }
        [System.IO.File]::Copy($file.FullName, $to)
    }
}

# Runs every example of the page at $Path in a new folder $Root, fixtures
# written as the page reaches them, and returns a description of each
# example whose output differs from what the page shows: its line, its
# command, the lines shown and the lines written. Every example runs in
# this function's scope, so a variable one sets is there for the next and
# gone when the page is done.
function Invoke-TrexDocumentedPage {
    param([Parameter(Mandatory)][string] $Path, [Parameter(Mandatory)][string] $Root)
    $trexDocItems = @(Read-TrexDocumentedPage -Path $Path)
    New-Item -ItemType Directory -Path $Root | Out-Null
    Copy-TrexDocumentedSeed -Commands @($trexDocItems | Where-Object Kind -eq 'Example' | ForEach-Object Command) -Root $Root
    $trexDocUtf8 = [System.Text.UTF8Encoding]::new($false)
    $trexDocRendering = $null
    if ($null -ne (Get-Variable -Name PSStyle -ErrorAction Ignore)) {
        $trexDocRendering = $PSStyle.OutputRendering
        $PSStyle.OutputRendering = 'PlainText'
    }
    Push-Location -LiteralPath $Root
    try {
        foreach ($trexDocItem in $trexDocItems) {
            if ($trexDocItem.Kind -eq 'Fixture') {
                $trexDocFile = Join-Path $Root $trexDocItem.File.Substring(2)
                $trexDocDir = Split-Path -Parent $trexDocFile
                if (-not (Test-Path -LiteralPath $trexDocDir)) {
                    New-Item -ItemType Directory -Path $trexDocDir | Out-Null
                }
                $trexDocText = ($trexDocItem.Shown -join "`n") + "`n"
                [System.IO.File]::WriteAllText($trexDocFile, $trexDocText, $trexDocUtf8)
                continue
            }
            $trexDocBlock = [scriptblock]::Create($trexDocItem.Command)
            $trexDocWritten = try {
                . $trexDocBlock 2>&1 | Out-String -Width 100
            } catch {
                'ERROR ' + $_.Exception.Message
            }
            $trexDocGot = ConvertTo-TrexShownOutput -Written $trexDocWritten -Root $Root
            if (($trexDocGot -join "`n") -cne ($trexDocItem.Shown -join "`n")) {
                "line $($trexDocItem.Line): PS> $($trexDocItem.Command)`n--- shown`n$($trexDocItem.Shown -join "`n")`n--- written`n$($trexDocGot -join "`n")"
            }
        }
    } finally {
        Pop-Location
        if ($null -ne $trexDocRendering) {
            $PSStyle.OutputRendering = $trexDocRendering
        }
    }
}
