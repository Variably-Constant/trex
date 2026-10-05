---
title: Windows
linkTitle: Windows
weight: 30
---

A window is part of an input: its first N lines, its last N, or a range of them, counted in
lines or in the records a unit names. A window is read from the end nearest it, so a line near
either end of a large file is reached without the rest being read, and every scan, rewrite,
redaction, table and template mining can be restricted to one. The flags and parameters are
on the [CLI](../cli/#head-tail-lines) and [PowerShell](../powershell/matching/#get-trexline)
pages.

The examples read this file:

```console
$ cat app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```

## First, last and a range

A range is `A..B`, `A..` to the end, `..B` from the start, or `A` alone, counted from one with
both ends included.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex head 2 app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures

$ trex tail 2 -n app.log
4:2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
5:2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms

$ trex lines 2..3 app.log
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;
use trex::window::{Select, window_of};

let log = b"2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
";
let tail = window_of(log, Select::Tail(2), &RecordUnit::Line);
assert_eq!(tail.line_base, Some(3));
assert!(tail.bytes.starts_with(b"2026-09-27T09:00:04Z ERROR"));
let range = window_of(log, Select::parse_range("2..3").expect("a range"), &RecordUnit::Line);
assert_eq!(range.bytes.split(|&b| b == b'\n').filter(|l| !l.is_empty()).count(), 2);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> print(trex.head(2, path="app.log"), end="")
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
>>> print(trex.lines("2..3", open("app.log").read()), end="")
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexLine -Path ./app.log -Head 2 -Passthru
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures

PS> Get-TrexLine -Path ./app.log -Tail 2 | Select-Object LineNumber, Text

LineNumber Text
---------- ----
         4 2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declin…
         5 2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms

PS> Get-TrexLine -Path ./app.log -Lines 2..3 -Passthru
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
```
{{< /tab >}}
{{< /tabs >}}

`-n` prints each line after its number in the input, `N:text`; a tail's lines are numbered by
counting the newlines ahead of it, which a tail printing no numbers never reads. `tail -N` is
`tail N`, and `--lines` gives `head` and `tail` a range in place of N. In PowerShell each line
is a `Trex.Line` carrying its number, `-Passthru` writes the text, and `-First` and `-Last` are
`-Head` and `-Tail`; Python reads a file with `path=` from the end nearest its part and
returns a `str`, or cuts a `str` or `bytes` it is given.

## Records

`--record UNIT` counts paragraphs, blocks or any other unit `--record` names instead of lines.

```console
$ cat notes.txt
from 10.0.0.1
to bob@x.com

from 10.0.0.2
nothing

mail amy@y.org
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex tail 2 --record paragraph notes.txt
from 10.0.0.2
nothing

mail amy@y.org
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;
use trex::window::{Select, window_of};

let notes = b"from 10.0.0.1\nto bob@x.com\n\nfrom 10.0.0.2\nnothing\n\nmail amy@y.org\n";
let tail = window_of(notes, Select::Tail(2), &RecordUnit::Paragraph);
assert_eq!(tail.bytes, b"from 10.0.0.2\nnothing\n\nmail amy@y.org\n");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.tail(2, path="notes.txt", unit="paragraph")
'from 10.0.0.2\nnothing\n\nmail amy@y.org\n'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexLine -Path ./notes.txt -Tail 2 -Unit paragraph -Passthru
from 10.0.0.2
nothing

mail amy@y.org
```
{{< /tab >}}
{{< /tabs >}}

## Several inputs and encodings

Several inputs are each headed `==> name <==`, a blank line between two, and the standard
input is read when no input is named or `-` is. The text printed is the input's own, without
its byte order mark and decoded where the mark declares UTF-16 or UTF-32: a head or a range of
UTF-16 or UTF-32 is decoded as it is read and stops at its end, and a tail is read backward in
whole code units. Offsets count the decoded text. A file holding a NUL byte and no UTF-16 or
UTF-32 mark is binary and printed only under `--binary`.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex head 1 app.log notes.txt
==> app.log <==
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms

==> notes.txt <==
from 10.0.0.1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;
use trex::window::{Select, window_of};

let inputs: [(&str, &[u8]); 2] = [
    ("app.log", b"2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
"),
    ("notes.txt", b"from 10.0.0.1\nto bob@x.com\n\nfrom 10.0.0.2\nnothing\n"),
];
let heads: Vec<(&str, Vec<u8>)> = inputs
    .iter()
    .map(|&(name, text)| (name, window_of(text, Select::Head(1), &RecordUnit::Line).bytes))
    .collect();
assert_eq!(heads, [
    ("app.log", b"2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms\n".to_vec()),
    ("notes.txt", b"from 10.0.0.1\n".to_vec()),
]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [trex.head(1, path=name) for name in ("app.log", "notes.txt")]
['2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms\n', 'from 10.0.0.1\n']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexLine -Path ./app.log, ./notes.txt -Head 1 | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, Text

File      Text
----      ----
app.log   2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
notes.txt from 10.0.0.1
```
{{< /tab >}}
{{< /tabs >}}

## A window of a scan

The same three selections restrict `scan`, `rewrite`, `redact`, the tables and `templates`:
`--head N`, `--tail N` and `--lines A..B`, counted in the unit `--record` names. A scan reads
only the window, a match counts only if it is wholly inside, and it reports the input's
own offset, line and column; a rewrite or a redaction prints the window alone, rewritten, and
in place changes only the window, the file keeping every other byte.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\R:took' app.log --tail 2
[346..350] "95ms"  captures: took="95ms"

$ trex scan '\I' app.log --lines 1..3 -H
app.log:1:46: "10.0.0.5"
app.log:3:47: "10.0.0.7"

$ trex rewrite '\R:took' '<${took}>' app.log --tail 1
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took <95ms>
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;
use trex::window::{Select, window_of};

let log = b"2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
";
let window = window_of(log, Select::Tail(2), &RecordUnit::Line);
let base = window.byte_base.expect("a cut knows its offset in the file");
let pat = trex::parse(r"\R:took").expect("valid pattern");
let at: Vec<usize> = trex::scan(&pat, &window.bytes).iter().map(|s| base + s.start()).collect();
assert_eq!(at, [346]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> log = open("app.log").read()
>>> [(m.text, m.start) for m in trex.Pattern(r"\R:took").scan(log, tail=2)]
[('95ms', 346)]
>>> trex.Pattern(r"\R:took").rewrite("<${took}>", log, tail=1)
'2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took <95ms>\n'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\R:took' -Path ./app.log -Tail 2 | Select-Object LineNumber, Start, Text

LineNumber Start Text
---------- ----- ----
         5   346 95ms

PS> Edit-TrexText '\R:took' '<${took}>' -Path ./app.log -Tail 1
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took <95ms>
```
{{< /tab >}}
{{< /tabs >}}

## Following a file

`trex tail 0 -f app.log` prints what the file gains as it grows, until interrupted, and
`--follow` does the same for a tail or a range with no last line on `scan`, `rewrite` and
`redact`, printing each match once nothing arriving later can change it and `--json` as one
object a line. A file cut shorter is read again from its start, and one replaced under its
name, as a rotated log is, from its first byte, each said on the standard error; under `-n` a
line is printed once its end arrives. A file read again is a new input, so `-m` counts its
matches afresh, on `scan` and `rewrite` alike; `--keep-count` keeps the count it had. A
followed edit runs on the stream scanner on the CPU and takes no `--gpu`. `scan --follow` takes no report printed
once the input ends or read around a match: `--count`, `--count-matches`, `-l`, `-L`,
`--stats`, `-v`, `--passthru`, context lines, a record query or `--explain`.

In PowerShell `-Follow` does the same on `Get-TrexLine`, `Select-TrexMatch`, `Invoke-TrexRule`,
`Edit-TrexText` and `Protect-TrexText`, until the pipeline is stopped, a file cut shorter or
replaced said as a warning and `-MaxCount` counting it afresh unless `-KeepCount` is given:
`Get-TrexLine -Path ./app.log -Tail 0 -Follow` writes each line the log gains. In Python
`trex.follow(pattern, *paths)` yields each match of a pattern as the files grow, from each
file's end or with `from_end=False` from its start, its file in `path`, and `Library.follow`
yields each finding of a library's rules; `max_count=` and `keep_count=` read as `-m` and
`--keep-count` do. `trex.follow_lines(*paths, tail=0)` yields each `Line` the files gain, and
`trex.follow_rewrite(pattern, repl, path)` and `trex.follow_redact(pattern, path)` each line of
a file rewritten or redacted, without its ending, as the command line writes them. In Rust
`trex::follow::Follower` watches a set of files from given offsets, and `wait` or `poll` hands
back what each gains; `trex::EditStream` edits what a followed file gains as `rewrite` and
`redact` do.

Each tab below starts from the log's last line. The command line and PowerShell keep running
until interrupted, so `head -1` and `Select-Object -First 1` stop them after that line; the Rust
and Python tabs append a line to the log and read it back as it arrives.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex tail 1 -f app.log | head -1
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
use std::io::Write;
use trex::follow::{Followed, Follower};

let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
std::fs::create_dir_all(&dir).expect("a directory");
let log = dir.join("app.log");
std::fs::write(&log, "2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms\n").expect("written");
let read_to = std::fs::metadata(&log).expect("a file").len() as usize;
let mut follower = Follower::new(&[(log.clone(), read_to)]).expect("a followed file");

let gained = b"2026-09-27T09:00:06Z INFO GET /v2/orders from 10.0.0.7 took 30ms\n";
std::fs::OpenOptions::new().append(true).open(&log).expect("appendable").write_all(gained).expect("appended");
let (file, change) = follower.wait().expect("a change");
assert_eq!(file, 0);
assert_eq!(change, Followed::Appended { bytes: gained.to_vec(), at: read_to });
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> follow = trex.follow_lines("app.log", tail=1)
>>> next(follow).text
'2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms'
>>> with open("app.log", "a") as log:
...     _ = log.write("2026-09-27T09:00:06Z INFO GET /v2/orders from 10.0.0.7 took 30ms\n")
>>> next(follow).text
'2026-09-27T09:00:06Z INFO GET /v2/orders from 10.0.0.7 took 30ms'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Get-TrexLine -Path ./app.log -Tail 1 -Follow | Select-Object -First 1).Text
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```
{{< /tab >}}
{{< /tabs >}}

## Cost

Measured on an AMD Ryzen 9 7900X under Windows 11 by `benches/headtail_timing.ps1`: the median of 15 runs of each
command through `cmd /c`, which alone takes 17 to 20 ms, while other work held 9.5 to 11 of
the machine's 24 cores. The UTF-8 rows span five files of 12 to 131 MB, two of them behind a
byte order mark; the UTF-16 rows are one file of 126 MB and 37 lines, whose last ten hold 51 MB
of text. Every command printed the same lines before any was timed.

| Selection | trex | GNU tools | ripgrep | ugrep | Pipes |
|---|---|---|---|---|---|
| `head 10` | 25.0-29.1 ms | `head -n 10` 30.7-34.4 | `rg -m 10 ''` 28.1-33.4 | `ugrep -K 1,10 ''` 29.6-32.8 | |
| `tail 10` | 27.3-27.9 | `tail -n 10` 29.8-35.0 | `rg '' \| tail` 93.5-392.5 | | |
| `tail 10 -n` | 31.4-64.3 | | `rg -n '' \| tail` 110.7-460.2 | | `cat -n \| tail` 89.1-494.5 |
| ten lines from the middle | 30.4-47.9 | `sed -n` 49.5-133.0 | | `ugrep -K` 32.7-43.4 | `head \| tail` 87.3-327.1 |
| UTF-16 `head 10` | 28.4 | | `rg -m 10 ''` 28.9 | `ugrep -K 1,10 ''` 46.0 | |
| UTF-16 `tail 10` | 169.5 | | `rg '' \| tail` 235.6 | | |
| UTF-16 `tail 10 -n` | 240.2 | | `rg -n '' \| tail` 238.2 | | |
| UTF-16 `lines 18..27` | 59.9 | | | `ugrep -K 18,27 ''` 442.6 | |
