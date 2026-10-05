---
title: Scan, rewrite or mask part of a file
linkTitle: Work on part of a file
weight: 8
---

Read the first lines of a file, its last, or a range of them, and scan, rewrite or mask that
part alone. A part at the end is read backward from the end, so how large the file is does not
matter, and each match keeps the file's own line and offset. The selections are on
[windows](../../reference/windows/).

```console
$ cat app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```

## Read the end of a file

`tail` reads backward from the end until it holds the lines asked for, and `-n` numbers them as
the file does:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex tail 3 -n app.log
3:2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
4:2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
5:2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;
use trex::window::{Asked, Select, read_file};

let path = std::env::temp_dir().join("trex-part-of-a-file.log");
std::fs::write(
    &path,
    "2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
",
)
.expect("write the log");
let tail = read_file(&path, Select::Tail(3), &RecordUnit::Line, Asked { numbers: true, ..Asked::default() })
    .expect("a readable file");
assert_eq!(tail.line_base, Some(2));
assert!(tail.bytes.starts_with(b"2026-09-27T09:00:03Z INFO GET /v2/orders"));
std::fs::remove_file(&path).expect("remove the log");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> print(trex.tail(3, path="app.log"), end="")
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexLine -Path ./app.log -Tail 3 | ForEach-Object { "$($_.LineNumber):$($_.Text)" }
3:2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
4:2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
5:2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```
{{< /tab >}}
{{< /tabs >}}

## Scan only the last lines

`--tail N` scans only the last N lines, read from the end, and a match reports its line and
offset in the whole file:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\R{>1s}:took' app.log --tail 3 -H
app.log:3:61: "1450ms"  captures: took="1450ms"
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
let window = window_of(log, Select::Tail(3), &RecordUnit::Line);
assert_eq!(window.line_base, Some(2));
let base = window.byte_base.expect("a cut knows its offset in the file");
let pat = trex::parse(r"\R{>1s}:took").expect("valid pattern");
let at: Vec<usize> = trex::scan(&pat, &window.bytes).iter().map(|s| base + s.start()).collect();
assert_eq!(at, [189]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> log = trex.read("app.log")
>>> [(m.text, m.start) for m in trex.Pattern(r"\R{>1s}:took").scan(log, tail=3)]
[('1450ms', 189)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\R{>1s}:took' -Path ./app.log -Tail 3 | Select-Object LineNumber, Start, Text

LineNumber Start Text
---------- ----- ----
         3   189 1450ms
```
{{< /tab >}}
{{< /tabs >}}

## Rewrite the first lines

`--head N` rewrites the first N lines and prints them alone:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex rewrite '\R{>1s}:took' '${took} (slow)' app.log --head 3
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms (slow)
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
let window = window_of(log, Select::Head(3), &RecordUnit::Line);
let pat = trex::parse(r"\R{>1s}:took").expect("valid pattern");
let tmpl = trex::Template::parse("${took} (slow)", &pat.capture_names()).expect("valid template");
let out = trex::rewrite(&pat, &tmpl, &window.bytes);
assert!(out.ends_with(b"took 1450ms (slow)\n"));
assert_eq!(out.split(|&b| b == b'\n').filter(|l| !l.is_empty()).count(), 3);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> print(trex.Pattern(r"\R{>1s}:took").rewrite("${took} (slow)", log, head=3), end="")
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms (slow)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Edit-TrexText '\R{>1s}:took' '${took} (slow)' -Path ./app.log -Head 3
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms (slow)
```
{{< /tab >}}
{{< /tabs >}}

## Mask a range

`--lines A..B` masks that range and prints it alone, so nothing outside it reaches the output:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex redact '(\E:email | \{card}:card)' --keep 'card:last4, email:domain' app.log --lines 4..5
2026-09-27T09:00:04Z ERROR payment for ****x.com failed: card ***************1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;
use trex::window::{Select, window_of};
use trex::{Keep, Mask, redactions};

let log = b"2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
";
let window = window_of(log, Select::parse_range("4..5").expect("a range"), &RecordUnit::Line);
let part: &[u8] = &window.bytes;
let pat = trex::parse(r"(\E:email | \{card}:card)").expect("valid pattern");
let keeps = Keep::parse_list("card:last4, email:domain", &pat.capture_names()).expect("valid fields");
let mut mask = Mask::parse("*").expect("a mask");
let matches = trex::captures(&pat, part, &trex::scan(&pat, part));
let mut out = Vec::new();
let mut at = 0;
for edit in redactions(part, &matches, &keeps, &mut mask) {
    out.extend_from_slice(&part[at..edit.start]);
    out.extend_from_slice(&edit.replacement);
    at = edit.end;
}
out.extend_from_slice(&part[at..]);
assert!(out.starts_with(b"2026-09-27T09:00:04Z ERROR payment for ****x.com failed: card ***************1111 declined\n"));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> secrets = trex.Pattern(r"(\E:email | \{card}:card)")
>>> print(secrets.redact(log, keep="card:last4, email:domain", lines="4..5"), end="")
2026-09-27T09:00:04Z ERROR payment for ****x.com failed: card ***************1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Protect-TrexText '(\E:email | \{card}:card)' -Keep card:last4, email:domain -Path ./app.log -Lines 4..5
2026-09-27T09:00:04Z ERROR payment for ****x.com failed: card ***************1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```
{{< /tab >}}
{{< /tabs >}}

`--record UNIT` counts paragraphs or another record unit in place of lines, on every command
above.
