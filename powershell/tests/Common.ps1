# Shared by every suite: imports the module cargo pwrs built, which the runner
# names in PWRS_MODULE.
function Import-TrexModule {
    if (-not $env:PWRS_MODULE) {
        throw 'PWRS_MODULE is not set; run the suites through cargo pwrs test'
    }
    Import-Module (Join-Path $env:PWRS_MODULE 'Trex.psd1') -Force
}

# The trex command the report suites compare the module with, which the runner
# names in TREX_CLI. A run without it cannot say whether the module writes what
# the command writes, so it fails rather than passing on nothing.
function Get-TrexCli {
    if (-not $env:TREX_CLI) {
        throw 'TREX_CLI is not set; it names the trex command the report suites compare with: build trex from this tree (cargo build --release at the repository root) and set TREX_CLI to target/release/trex, or trex.exe on Windows'
    }
    if (-not (Test-Path -LiteralPath $env:TREX_CLI -PathType Leaf)) {
        throw "TREX_CLI names $env:TREX_CLI, which is not a file"
    }
    $env:TREX_CLI
}

# The lines the trex command prints given $Arguments, one string of them as
# cmd.exe would take it, and $Text on its standard input as UTF-8 with the
# newlines it holds, which piping a string to a native command would not
# keep. Windows PowerShell writes the preamble of the console's input encoding
# to a child's standard input as it starts the child, a byte-order mark on a
# UTF-8 console, so the start is made under a UTF-8 encoding that has none and
# the child reads the bytes of $Text alone.
function Invoke-TrexCliInput {
    param(
        [Parameter(Mandatory)][string] $Arguments,
        [Parameter(Mandatory)][AllowEmptyString()][string] $Text
    )
    $psi = [System.Diagnostics.ProcessStartInfo]::new((Get-TrexCli), $Arguments)
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.StandardOutputEncoding = [System.Text.UTF8Encoding]::new($false)
    $console = [Console]::InputEncoding
    $marked = $console.GetPreamble().Length -gt 0
    if ($marked) { [Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false) }
    try {
        $p = [System.Diagnostics.Process]::Start($psi)
    } finally {
        if ($marked) { [Console]::InputEncoding = $console }
    }
    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($Text)
    $p.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length)
    $p.StandardInput.Close()
    $out = $p.StandardOutput.ReadToEnd()
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) { throw "trex $Arguments exited $($p.ExitCode)" }
    @($out -split "`r?`n" | Where-Object { $_ -ne '' })
}

# A file holding $Text in the encoding named: UTF-8 with or without a
# byte-order mark, or UTF-16 LE with one, which is what Windows PowerShell's
# Out-File writes.
function Write-TrexTestFile {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $Text,
        [ValidateSet('Utf8', 'Utf8Bom', 'Utf16')][string] $Encoding = 'Utf8'
    )
    $enc = switch ($Encoding) {
        'Utf8' { [System.Text.UTF8Encoding]::new($false) }
        'Utf8Bom' { [System.Text.UTF8Encoding]::new($true) }
        'Utf16' { [System.Text.UnicodeEncoding]::new($false, $true) }
    }
    [System.IO.File]::WriteAllText($Path, $Text, $enc)
}
