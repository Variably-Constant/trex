---
title: Redact a log and follow it
linkTitle: Redact and follow logs
weight: 8
---

Mask card numbers, addresses and emails in a log, keep the parts that still identify a row, and
go on masking what the log gains as it grows. The masks are on
[redaction](../../reference/redaction/).

```console
$ cat app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.7 took 95ms
```

## Mask, keeping what identifies a row

Each masked character becomes one `*`, so every column after a match stays where it was. `--keep`
leaves the named slices unmasked: a card's last four digits, an address's first two octets, an
email's domain.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex redact '(\{card}:card | \I:ip | \E:email)' --keep 'card:last4, ip:octet1-2, email:domain' app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0**** took 120ms
2026-09-27T09:00:04Z ERROR payment for ****x.com failed: card ***************1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0**** took 95ms
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::{Keep, Mask, redactions};

let log = b"2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined\n";
let pat = trex::parse(r"(\{card}:card | \I:ip | \E:email)").expect("valid pattern");
let keeps = Keep::parse_list("card:last4, ip:octet1-2, email:domain", &pat.capture_names()).expect("valid fields");
let mut mask = Mask::parse("*").expect("a mask");
let matches = trex::captures(&pat, log, &trex::scan(&pat, log));
let mut out = Vec::new();
let mut at = 0;
for edit in redactions(log, &matches, &keeps, &mut mask) {
    out.extend_from_slice(&log[at..edit.start]);
    out.extend_from_slice(&edit.replacement);
    at = edit.end;
}
out.extend_from_slice(&log[at..]);
assert_eq!(out, b"2026-09-27T09:00:04Z ERROR payment for ****x.com failed: card ***************1111 declined\n");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> secrets = trex.Pattern(r"(\{card}:card | \I:ip | \E:email)")
>>> print(secrets.redact(open("app.log").read(), keep="card:last4, ip:octet1-2, email:domain"), end="")
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0**** took 120ms
2026-09-27T09:00:04Z ERROR payment for ****x.com failed: card ***************1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0**** took 95ms
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Protect-TrexText '(\{card}:card | \I:ip | \E:email)' -Keep card:last4, ip:octet1-2, email:domain -Path ./app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0**** took 120ms
2026-09-27T09:00:04Z ERROR payment for ****x.com failed: card ***************1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0**** took 95ms
```
{{< /tab >}}
{{< /tabs >}}

`--mask pseudonym` writes a name such as `IP_1` instead, the same name for the same value
wherever it occurs.

## Mask the end of a log

`--tail N` masks and prints the last N lines alone, so nothing outside them reaches the output
unmasked:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex redact '(\{card}:card | \I:ip | \E:email)' --keep 'card:last4, ip:octet1-2, email:domain' app.log --tail 1
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0**** took 95ms
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;
use trex::window::{Select, window_of};
use trex::{Keep, Mask, redactions};

let log = b"2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms\n2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined\n2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.7 took 95ms\n";
let window = window_of(log, Select::Tail(1), &RecordUnit::Line);
let last: &[u8] = &window.bytes;
let pat = trex::parse(r"(\{card}:card | \I:ip | \E:email)").expect("valid pattern");
let keeps = Keep::parse_list("card:last4, ip:octet1-2, email:domain", &pat.capture_names()).expect("valid fields");
let mut mask = Mask::parse("*").expect("a mask");
let matches = trex::captures(&pat, last, &trex::scan(&pat, last));
let mut out = Vec::new();
let mut at = 0;
for edit in redactions(last, &matches, &keeps, &mut mask) {
    out.extend_from_slice(&last[at..edit.start]);
    out.extend_from_slice(&edit.replacement);
    at = edit.end;
}
out.extend_from_slice(&last[at..]);
assert_eq!(out, b"2026-09-27T09:00:05Z INFO GET /v2/users from 10.0**** took 95ms\n");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> print(secrets.redact(open("app.log").read(), keep="card:last4, ip:octet1-2, email:domain", tail=1), end="")
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0**** took 95ms
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Protect-TrexText '(\{card}:card | \I:ip | \E:email)' -Keep card:last4, ip:octet1-2, email:domain -Path ./app.log -Tail 1
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0**** took 95ms
```
{{< /tab >}}
{{< /tabs >}}

## Follow it as it grows

`--follow` then masks what the log gains, line by line, until interrupted, and reads a log cut
shorter or rotated under its name from its start again. Each tab below starts from the log's last
line. The command line and PowerShell keep running, so `head -1` and `Select-Object -First 1`
stop them after that line; the Rust and Python tabs append a line holding a card and an address,
and read it back masked as it arrives. `--tail 0` starts with nothing and masks only what
arrives.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex redact '(\{card}:card | \I:ip | \E:email)' --keep 'card:last4, ip:octet1-2, email:domain' app.log --tail 1 --follow | head -1
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0**** took 95ms
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
use std::io::Write;
use trex::follow::{Followed, Follower};
use trex::{Keep, Mask, redactions};

let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
std::fs::create_dir_all(&dir).expect("a directory");
let log = dir.join("app.log");
std::fs::write(&log, "2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.7 took 95ms\n").expect("written");
let read_to = std::fs::metadata(&log).expect("a file").len() as usize;
let mut follower = Follower::new(&[(log.clone(), read_to)]).expect("a followed file");
std::fs::OpenOptions::new()
    .append(true)
    .open(&log)
    .expect("appendable")
    .write_all(b"2026-09-27T09:00:06Z WARN card 4111 1111 1111 1111 from 10.0.0.9\n")
    .expect("appended");
let Followed::Appended { bytes, .. } = follower.wait().expect("a change").1 else {
    panic!("the log grew and nothing else changed it");
};

let pat = trex::parse(r"(\{card}:card | \I:ip | \E:email)").expect("valid pattern");
let keeps = Keep::parse_list("card:last4, ip:octet1-2, email:domain", &pat.capture_names()).expect("valid fields");
let mut mask = Mask::parse("*").expect("a mask");
let matches = trex::captures(&pat, &bytes, &trex::scan(&pat, &bytes));
let mut out = Vec::new();
let mut at = 0;
for edit in redactions(&bytes, &matches, &keeps, &mut mask) {
    out.extend_from_slice(&bytes[at..edit.start]);
    out.extend_from_slice(&edit.replacement);
    at = edit.end;
}
out.extend_from_slice(&bytes[at..]);
assert_eq!(out, b"2026-09-27T09:00:06Z WARN card ***************1111 from 10.0****\n");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> masked = trex.follow_redact(secrets, "app.log", keep="card:last4, ip:octet1-2, email:domain", tail=1)
>>> next(masked)
'2026-09-27T09:00:05Z INFO GET /v2/users from 10.0**** took 95ms'
>>> with open("app.log", "a") as log:
...     _ = log.write("2026-09-27T09:00:06Z WARN card 4111 1111 1111 1111 from 10.0.0.9\n")
>>> next(masked)
'2026-09-27T09:00:06Z WARN card ***************1111 from 10.0****'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Protect-TrexText '(\{card}:card | \I:ip | \E:email)' -Keep card:last4, ip:octet1-2, email:domain -Path ./app.log -Tail 1 -Follow | Select-Object -First 1
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0**** took 95ms
```
{{< /tab >}}
{{< /tabs >}}

In Rust `trex::EditStream` runs the same masking over a followed file's stream for you, the
whole of a match held back until a later chunk can no longer change it
([following a file](../../reference/windows/#following-a-file)).
