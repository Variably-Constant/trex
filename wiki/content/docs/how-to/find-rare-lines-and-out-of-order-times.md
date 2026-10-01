---
title: Find rare lines and out-of-order times
linkTitle: Find rare lines and out-of-order times
weight: 24
---

# Find rare lines and out-of-order times

Find the lines whose shape a log seldom repeats, and the timestamps that run backward
([axis predicates and anchors](../../reference/pattern-syntax/#axis-predicates-and-anchors)).

```console
$ cat times.log
2026-09-15T10:00:00Z GET /a 200
2026-09-15T10:01:00Z GET /b 200
2026-09-15T09:58:00Z GET /c 200
2026-09-15T10:02:00Z GET /d 200
kernel: disk failure on /dev/sda
```

## A timestamp earlier than the one before it

`@order:desc` holds on a timestamp before the timestamp token ahead of it in the stream:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@order:desc \T' times.log
[64..84] "2026-09-15T09:58:00Z"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"@order:desc \T").expect("valid pattern");
let log = "2026-09-15T10:00:00Z GET /a 200\n2026-09-15T10:01:00Z GET /b 200\n2026-09-15T09:58:00Z GET /c 200\n2026-09-15T10:02:00Z GET /d 200\nkernel: disk failure on /dev/sda\n";
let found: Vec<&str> = trex::scan(&pat, log.as_bytes()).iter().map(|s| &log[s.range()]).collect();
assert_eq!(found, ["2026-09-15T09:58:00Z"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> log = open("times.log").read()
>>> [m.text for m in trex.Pattern(r"@order:desc \T").scan(log)]
['2026-09-15T09:58:00Z']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@order:desc \T' -Path ./times.log -Raw
2026-09-15T09:58:00Z
```
{{< /tab >}}
{{< /tabs >}}

## A line of a rare shape

`@shape:rare` holds at every token of a line whose template covers fewer lines than the mean
template does; `@shape:rare<5` fewer than five, `@shape:rare<1%` less than one percent of the
lines:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@shape:rare \W' times.log
[128..134] "kernel"
[136..140] "disk"
[141..148] "failure"
[149..151] "on"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"@shape:rare \W").expect("valid pattern");
let log = "2026-09-15T10:00:00Z GET /a 200\n2026-09-15T10:01:00Z GET /b 200\n2026-09-15T09:58:00Z GET /c 200\n2026-09-15T10:02:00Z GET /d 200\nkernel: disk failure on /dev/sda\n";
let found: Vec<&str> = trex::scan(&pat, log.as_bytes()).iter().map(|s| &log[s.range()]).collect();
assert_eq!(found, ["kernel", "disk", "failure", "on"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"@shape:rare \W").scan(log)]
['kernel', 'disk', 'failure', 'on']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@shape:rare \W' -Path ./times.log -Raw
kernel
disk
failure
on
```
{{< /tab >}}
{{< /tabs >}}

`trex templates --rare` lists the rare templates themselves ([summarize a log by
templates](../summarize-a-log-by-templates/)).
