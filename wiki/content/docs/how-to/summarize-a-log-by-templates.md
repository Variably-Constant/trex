---
title: Summarize a log by its templates
linkTitle: Summarize a log by templates
weight: 17
---

# Summarize a log by its templates

Read a log as the handful of line shapes it repeats, with how many lines each covers, then scan
for the lines of one shape. A position whose text varies is a slot named by its kind
([templates](../../reference/building-patterns/#templates)).

```console
$ cat requests.log
10.0.0.1 GET /index.html status 200 12ms
10.0.0.2 GET /about.html status 200 8ms
10.0.0.1 POST /login status 302 40ms
10.0.0.3 GET /index.html status 200 11ms
10.0.0.9 GET /admin status 403 3ms
kernel: disk failure on /dev/sda
```

## The shapes and their counts

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex templates requests.log
5  <ip> <word> <path> status <number> <duration>
1  kernel: disk failure on /dev/sda
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"10.0.0.1 GET /index.html status 200 12ms\n10.0.0.2 GET /about.html status 200 8ms\n10.0.0.1 POST /login status 302 40ms\n10.0.0.3 GET /index.html status 200 11ms\n10.0.0.9 GET /admin status 403 3ms\nkernel: disk failure on /dev/sda\n";
let mining = trex::templates::Mining::mine(log);
let counts: Vec<usize> = mining.templates.iter().map(|t| t.count()).collect();
assert_eq!(counts, [5, 1]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> log = open("requests.log").read()
>>> [(t["count"], t["readable"]) for t in trex.templates(log)]
[(5, '<ip> <word> <path> status <number> <duration>'), (1, 'kernel: disk failure on /dev/sda')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexRecordShape -Path ./requests.log | Select-Object Count, Readable

Count Readable
----- --------
    5 <ip> <word> <path> status <number> <duration>
    1 kernel: disk failure on /dev/sda
```
{{< /tab >}}
{{< /tabs >}}

## Scan for the lines of one shape

Each template is also a pattern that finds exactly its lines:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex templates requests.log --pattern
5  \I \W \L "status" \N \R
1  "kernel" ":" "disk" "failure" "on" "/dev/sda"

$ trex scan '\I \W \L "status" \N{>=300} \R' requests.log
[81..117] "10.0.0.1 POST /login status 302 40ms"
[159..193] "10.0.0.9 GET /admin status 403 3ms"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"10.0.0.1 GET /index.html status 200 12ms\n10.0.0.2 GET /about.html status 200 8ms\n10.0.0.1 POST /login status 302 40ms\n10.0.0.3 GET /index.html status 200 11ms\n10.0.0.9 GET /admin status 403 3ms\nkernel: disk failure on /dev/sda\n";
let shape = trex::templates::Mining::mine(log).templates[0].pattern();
assert_eq!(shape, r#"\I \W \L "status" \N \R"#);
let failures = trex::parse(&shape.replace(r"\N", r"\N{>=300}")).expect("valid pattern");
assert_eq!(trex::scan(&failures, log).len(), 2);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> shape = trex.templates(log)[0]["pattern"]
>>> [m.text for m in trex.Pattern(shape.replace(r"\N", r"\N{>=300}")).scan(log)]
['10.0.0.1 POST /login status 302 40ms', '10.0.0.9 GET /admin status 403 3ms']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $shape = (Get-TrexRecordShape -Path ./requests.log)[0].Pattern
PS> Select-TrexMatch $shape.Replace('\N', '\N{>=300}') -Path ./requests.log -Raw
10.0.0.1 POST /login status 302 40ms
10.0.0.9 GET /admin status 403 3ms
```
{{< /tab >}}
{{< /tabs >}}

`--record paragraph` and the other units group records rather than lines, and `--against
FILE` marks each template shared with another log or new to this one. The lines whose shape is
rare are a [how-to of their own](../find-rare-lines-and-out-of-order-times/).
