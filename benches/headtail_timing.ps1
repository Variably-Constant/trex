# Timings of trex head, tail and lines against GNU coreutils head, tail, cat
# and sed, ugrep -K, and ripgrep piped into tail, over the files named:
#
#   powershell -NoProfile -File benches\headtail_timing.ps1 -Files a.txt,b.log
#       [-Tree DIR] [-GitBin DIR] [-Reps N]
#
# Every command runs through `cmd /s /c "... > NUL"` with its arguments given
# exactly, so each arm has the same launch cost, and a `cmd` running only `rem` is
# timed beside them as that floor. Each repetition runs every arm of a cell in
# an order rotated by one, so no arm keeps a position. Before anything is
# timed, every arm of every cell on every file runs once and must print the
# same ten lines on standard output as the cell's first arm, text only, since
# the numbered forms spell the number each their own way; a disagreement stops
# the run with exit 8, and any other failure prints RUN_FAILED and exits 9. On
# a UTF-16 file the coreutils arms, which cut bytes, are skipped by name.
# Whatever an arm writes to standard error is logged. trex writes no note of
# its scheduler to the console, so its calibration is read from the store
# itself: the numbered tail of the largest file runs until a start leaves the
# store as it found it, and every timed start of trex that writes the store is
# named. The timed trex is the release build of the tree, logged with its hash
# before the first cell and again after the last, and a changed hash voids the
# run. The foreign load, every other process's CPU time over the file's
# window, is printed beside each file, with the processes whose CPU time
# could not be read named once. A build running when it starts stops it with
# exit 3. The log is written under the tree's target directory.
param(
    [Parameter(Mandatory)][string[]] $Files,
    [string] $Tree,
    [string] $GitBin = 'C:\Program Files\Git\usr\bin',
    [int] $Reps = 15
)
# The tree the script is in, where no other is named; Windows PowerShell
# sets $PSScriptRoot only once the parameters are bound.
if (-not $Tree) { $Tree = Split-Path -Parent $PSScriptRoot }
# `powershell -File` hands a script each argument as the one string typed, so
# the paths come as one argument, separated by commas.
$Files = @($Files | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$ErrorActionPreference = 'Continue'
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$log = Join-Path $Tree "target\headtail_timing_$stamp.log"
function Say([string] $s) { Write-Output $s; Add-Content -Path $log -Value $s -Encoding utf8 }

function Snapshot {
    $m = @{}
    $unread = [Collections.Generic.List[string]]::new()
    Get-Process | Where-Object { $_.Id -ne 0 -and $_.Name -ne 'Idle' } | ForEach-Object {
        $cpu = try { $_.TotalProcessorTime.TotalSeconds } catch { $null }
        if ($null -ne $cpu) { $m[$_.Id] = $cpu } else { $unread.Add("$($_.Name)#$($_.Id)") }
    }
    @{ At = Get-Date; Cpu = $m; Unread = $unread }
}

# The cores every process present at the second snapshot held between the two,
# less this runner's own.
function ForeignCores($a, $b) {
    $held = 0.0
    foreach ($id in $b.Cpu.Keys) {
        if ($id -eq $PID) { continue }
        $then = if ($a.Cpu.ContainsKey($id)) { $a.Cpu[$id] } else { 0.0 }
        $d = $b.Cpu[$id] - $then
        if ($d -gt 0) { $held += $d }
    }
    $held / [math]::Max(($b.At - $a.At).TotalSeconds, 0.001)
}

# Where each arm's standard error goes, read after every run.
$errFile = Join-Path $Tree 'target\headtail_arm_stderr.txt'

# What the last arm run wrote to standard error, as one line.
function Get-ArmStderr {
    if (-not (Test-Path -LiteralPath $errFile)) { return '' }
    ([IO.File]::ReadAllText($errFile) -replace "`r?`n", ' | ').Trim(' ', '|')
}

# The calibration store's files, each with its size and last write, as one
# line: a start that measures the host writes its draw there, and a start that
# reads a settled record leaves the store as it was.
function Get-StoreMark {
    $files = @(Get-ChildItem -LiteralPath $env:FLYNNEL_CALIBRATION_DIR -File -Recurse -ErrorAction Stop)
    (@($files | Sort-Object FullName | ForEach-Object { '{0}:{1}:{2}' -f $_.Name, $_.Length, $_.LastWriteTimeUtc.Ticks }) -join ';')
}

# `line` run by cmd, and the milliseconds from start to exit, its standard
# error written to $errFile unless `bare` says the line is the floor. A
# nonzero exit throws, since that arm did not do the work the others did.
function Measure-Arm([string] $line, [switch] $bare) {
    $full = if ($bare) { $line } else { $line + ' 2> "' + $errFile + '"' }
    $psi = [Diagnostics.ProcessStartInfo]::new('cmd.exe', '/s /c "' + $full + '"')
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $p = [Diagnostics.Process]::Start($psi)
    $p.WaitForExit()
    $clock.Stop()
    if ($p.ExitCode -ne 0) { throw "exit $($p.ExitCode) from: $line; stderr: $(Get-ArmStderr)" }
    $clock.Elapsed.TotalMilliseconds
}

# What `line` prints on standard output, decoded as UTF-8 and split into
# lines, each line's leading number and its separator removed so the numbered
# forms compare by their text. Standard error goes to $errFile, as it does
# under Measure-Arm, and a nonzero exit throws with what was printed.
function Get-ArmText([string] $line) {
    $psi = [Diagnostics.ProcessStartInfo]::new('cmd.exe', '/s /c "' + $line + ' 2> "' + $errFile + '""')
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $p = [Diagnostics.Process]::Start($psi)
    $text = $p.StandardOutput.ReadToEnd()
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) { throw "exit $($p.ExitCode) from: $line`n$text`nstderr: $(Get-ArmStderr)" }
    $lines = @($text -split "`r?`n")
    if ($lines.Count -gt 0 -and $lines[-1] -eq '') { $lines = @($lines | Select-Object -First ($lines.Count - 1)) }
    @($lines | ForEach-Object { $_ -replace '^\s*\d+(:|\t)', '' })
}

# The byte order mark a file opens with, by the encoding it declares, or none.
function Get-Mark([string] $path) {
    $f = [IO.File]::OpenRead($path)
    try {
        $b = [byte[]]::new(4)
        $n = $f.Read($b, 0, 4)
        if ($n -ge 4 -and $b[0] -eq 0xFF -and $b[1] -eq 0xFE -and $b[2] -eq 0 -and $b[3] -eq 0) { return 'utf-32le' }
        if ($n -ge 4 -and $b[0] -eq 0 -and $b[1] -eq 0 -and $b[2] -eq 0xFE -and $b[3] -eq 0xFF) { return 'utf-32be' }
        if ($n -ge 2 -and $b[0] -eq 0xFF -and $b[1] -eq 0xFE) { return 'utf-16le' }
        if ($n -ge 2 -and $b[0] -eq 0xFE -and $b[1] -eq 0xFF) { return 'utf-16be' }
        if ($n -ge 3 -and $b[0] -eq 0xEF -and $b[1] -eq 0xBB -and $b[2] -eq 0xBF) { return 'utf-8' }
        'none'
    } finally { $f.Dispose() }
}

# The newline bytes in a file, counted apart from every tool under test.
Add-Type -TypeDefinition @'
public static class NewlineCount {
    public static long Of(string path) {
        var buffer = new byte[1 << 20];
        long n = 0;
        using (var f = System.IO.File.OpenRead(path)) {
            int r;
            while ((r = f.Read(buffer, 0, buffer.Length)) > 0)
                for (int i = 0; i < r; i++) if (buffer[i] == 10) n++;
        }
        return n;
    }
}
'@

function Median([double[]] $v) { $s = @($v | Sort-Object); $s[[int][math]::Floor($s.Count / 2)] }

$runFailed = $false
try {
    Set-Location -LiteralPath $Tree
    Say "TIMING_START $(Get-Date -Format o) reps=$Reps tree=$Tree head=$(git -C $Tree rev-parse --short HEAD) uncommitted=$(@(git -C $Tree status --short).Count)"
    $busy = @(Get-Process -Name cargo, rustc, cargo-pwrs, link, cl -ErrorAction SilentlyContinue | ForEach-Object { "$($_.Name)#$($_.Id)" })
    if ($busy.Count -gt 0) { Say ("BUSY a build is running, so nothing was timed: " + ($busy -join ' ')); exit 3 }
    if (Test-Path Env:\CARGO_TARGET_DIR) { Remove-Item Env:\CARGO_TARGET_DIR }
    $build = @(cargo build --release --bin trex 2>&1 | ForEach-Object { "$_" })
    if ($LASTEXITCODE -ne 0) { $build | ForEach-Object { Say "BUILD $_" }; Say 'BUILD_FAILED'; exit 5 }
    $trex = Join-Path $Tree 'target\release\trex.exe'
    $built = Get-Item -LiteralPath $trex -ErrorAction Stop
    $hash = (Get-FileHash -LiteralPath $trex -Algorithm SHA256).Hash
    Say ("BINARY {0} bytes={1} written={2} sha256={3}" -f $trex, $built.Length, $built.LastWriteTime.ToString('o'), $hash)
    $head = Join-Path $GitBin 'head.exe'
    $tail = Join-Path $GitBin 'tail.exe'
    $sed = Join-Path $GitBin 'sed.exe'
    $cat = Join-Path $GitBin 'cat.exe'
    $rg = (Get-Command rg).Source
    $ug = (Get-Command ugrep).Source
    Say ("VERSION trex: " + (& $trex --version))
    Say ("VERSION rg: " + (& $rg --version | Select-Object -First 1))
    Say ("VERSION ugrep: " + (& $ug --version | Select-Object -First 1))
    Say ("VERSION coreutils: " + (& $tail --version | Select-Object -First 1))
    $env:FLYNNEL_CALIBRATION_DIR = Join-Path $Tree "target\headtail_cal_$stamp"
    New-Item -ItemType Directory -Force -Path $env:FLYNNEL_CALIBRATION_DIR | Out-Null
    Say "PROFILE pinned to $($env:FLYNNEL_CALIBRATION_DIR)"
    $seen = Snapshot
    Say ("UNREADABLE {0}: {1}" -f $seen.Unread.Count, ($seen.Unread -join ' '))

    foreach ($f in $Files) {
        if (-not (Test-Path -LiteralPath $f)) { Say "MISSING $f"; exit 7 }
    }
    $floor = 'rem'
    $plan = [ordered]@{}
    foreach ($f in $Files) {
        $bytes = (Get-Item -LiteralPath $f).Length
        $lines = [NewlineCount]::Of($f)
        $mark = Get-Mark $f
        if ($lines -lt 20) { throw "$f has $lines lines, too few for ten from its middle" }
        $a = [math]::Floor($lines / 2)
        $b = $a + 9
        $q = "`"$f`""
        $cells = [ordered]@{
            'head 10' = [ordered]@{
                'trex head' = "`"$trex`" head 10 $q"
                'coreutils head' = "`"$head`" -n 10 $q"
                'ugrep -K' = "`"$ug`" -K 1,10 `"`" $q"
                'rg -m' = "`"$rg`" -m 10 `"`" $q"
            }
            'tail 10' = [ordered]@{
                'trex tail' = "`"$trex`" tail 10 $q"
                'coreutils tail' = "`"$tail`" -n 10 $q"
                'rg | tail' = "`"$rg`" `"`" $q | `"$tail`" -n 10"
            }
            'tail 10 numbered' = [ordered]@{
                'trex tail -n' = "`"$trex`" tail 10 -n $q"
                'rg -n | tail' = "`"$rg`" -n `"`" $q | `"$tail`" -n 10"
                'cat -n | tail' = "`"$cat`" -n $q | `"$tail`" -n 10"
            }
            "lines $a..$b" = [ordered]@{
                'trex lines' = "`"$trex`" lines $a..$b $q"
                'sed -n' = "`"$sed`" -n `"$a,$($b)p;$($b)q`" $q"
                'ugrep -K' = "`"$ug`" -K $a,$b `"`" $q"
                'head | tail' = "`"$head`" -n $b $q | `"$tail`" -n 10"
            }
        }
        # A UTF-16 file's lines are text only to the tools that decode it;
        # the coreutils arms cut its bytes, and tail starts mid code unit.
        if ($mark -like 'utf-16*') {
            foreach ($cell in @($cells.Keys)) {
                foreach ($arm in @($cells[$cell].Keys)) {
                    if ($arm -in @('coreutils head', 'coreutils tail', 'cat -n | tail', 'sed -n', 'head | tail')) {
                        $cells[$cell].Remove($arm)
                        Say ("SKIP {0} {1} {2}: the file is UTF-16 and this arm reads its bytes" -f $f, $cell, $arm)
                    }
                }
            }
        }
        $plan[$f] = @{ Bytes = $bytes; Lines = $lines; Mark = $mark; Cells = $cells }
    }

    # Every arm of every cell prints the same ten lines before anything is timed.
    $disagree = 0
    foreach ($f in $plan.Keys) {
        $cells = $plan[$f].Cells
        foreach ($cell in $cells.Keys) {
            $arms = @($cells[$cell].Keys)
            $want = @(Get-ArmText $cells[$cell][$arms[0]])
            foreach ($arm in $arms) {
                $got = @(Get-ArmText $cells[$cell][$arm])
                $err = Get-ArmStderr
                if ($err) { Say ("STDERR {0} {1} {2}: {3}" -f $f, $cell, $arm, $err) }
                if ($got.Count -ne 10 -or ($got -join "`n") -ne ($want -join "`n")) {
                    $disagree++
                    Say ("DISAGREE {0} {1} {2}: {3} lines, first {4}" -f $f, $cell, $arm, $got.Count, ($got | Select-Object -First 1))
                }
            }
            Say ("AGREE_CHECKED {0} {1}: {2} arms against {3}'s {4} lines" -f $f, $cell, $arms.Count, $arms[0], $want.Count)
        }
    }
    if ($disagree -gt 0) { Say "DISAGREEMENTS $disagree, so nothing was timed"; exit 8 }

    # The largest file's numbered tail runs until a start leaves the store as
    # it found it, so the timed starts read a settled record.
    $largest = @($plan.Keys | Sort-Object { $plan[$_].Bytes } -Descending)[0]
    $settled = $false
    for ($i = 1; $i -le 20; $i++) {
        $before = Get-StoreMark
        $null = Get-ArmText "`"$trex`" tail 10 -n `"$largest`""
        $after = Get-StoreMark
        if ($after -eq $before) { Say "CALIBRATION settled after $i start(s) of the numbered tail on $largest"; $settled = $true; break }
        Say ("CALIBRATION start {0} wrote the store: {1}" -f $i, $after)
    }
    if (-not $settled) { Say 'CALIBRATION_UNSETTLED after 20 starts: every timed start of trex may measure the host inside its clock' }

    foreach ($f in $plan.Keys) {
        $cells = $plan[$f].Cells
        Say ("FILE {0} bytes={1} lines={2} mark={3}" -f $f, $plan[$f].Bytes, $plan[$f].Lines, $plan[$f].Mark)
        $s0 = Snapshot
        foreach ($cell in $cells.Keys) {
            $arms = @($cells[$cell].Keys)
            $times = @{}
            $noisy = @{}
            $drew = @{}
            foreach ($arm in $arms) { $times[$arm] = [Collections.Generic.List[double]]::new(); $noisy[$arm] = 0; $drew[$arm] = 0 }
            $control = [Collections.Generic.List[double]]::new()
            for ($r = 0; $r -lt $Reps; $r++) {
                $control.Add((Measure-Arm $floor -bare))
                for ($k = 0; $k -lt $arms.Count; $k++) {
                    $arm = $arms[($k + $r) % $arms.Count]
                    $before = Get-StoreMark
                    $times[$arm].Add((Measure-Arm ($cells[$cell][$arm] + ' > NUL')))
                    if ((Get-StoreMark) -ne $before) {
                        $drew[$arm]++
                        Say ("  CALIBRATION_IN_CLOCK {0} {1} rep {2}" -f $cell, $arm, $r)
                    }
                    $err = Get-ArmStderr
                    if ($err) {
                        $noisy[$arm]++
                        Say ("  STDERR_IN_CLOCK {0} {1} rep {2}: {3}" -f $cell, $arm, $r, $err)
                    }
                }
            }
            foreach ($arm in $arms) {
                $v = $times[$arm].ToArray()
                $min = ($v | Measure-Object -Minimum).Minimum
                $max = ($v | Measure-Object -Maximum).Maximum
                Say ("  {0,-18} {1,-16} median {2,9:N2} ms  min {3,9:N2}  max {4,9:N2}  spread {5,5:N2}x  stderr {6}/{7}  drew {8}/{7}" -f `
                    $cell, $arm, (Median $v), $min, $max, ($max / [math]::Max($min, 0.001)), $noisy[$arm], $Reps, $drew[$arm])
            }
            Say ("  {0,-18} {1,-16} median {2,9:N2} ms" -f $cell, 'cmd rem (floor)', (Median $control.ToArray()))
        }
        $s1 = Snapshot
        Say ("  FOREIGN cores={0:N2} over {1:N1} s, {2} processes unreadable" -f (ForeignCores $s0 $s1), ($s1.At - $s0.At).TotalSeconds, $s1.Unread.Count)
    }

    $after = (Get-FileHash -LiteralPath $trex -Algorithm SHA256).Hash
    if ($after -ne $hash) { Say "BINARY_CHANGED sha256=$after during the run, so its timings are void"; exit 6 }
    Say "BINARY_UNCHANGED sha256=$after"
    $records = @(Get-ChildItem $env:FLYNNEL_CALIBRATION_DIR -File -Recurse -ErrorAction Stop)
    Say ("PROFILE records {0}" -f $records.Count)
    Say "TIMING_DONE $(Get-Date -Format o) log=$log"
} catch {
    Say ("RUN_FAILED " + $_.Exception.Message)
    Say ("  at " + $_.InvocationInfo.PositionMessage)
    $runFailed = $true
}
if ($runFailed) { exit 9 }
