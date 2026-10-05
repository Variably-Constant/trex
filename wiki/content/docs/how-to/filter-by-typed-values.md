---
title: Filter by typed values
linkTitle: Filter by typed values
weight: 13
---

Match a duration, a size, an address or a status by its value rather than its text: `1450ms`
is above a second, `2MB` above a megabyte, whatever unit they were written in. The predicates are
on [typed value predicates](../../reference/pattern-syntax/#typed-value-predicates).

```console
$ cat req.log
10.0.0.5 GET /a 200 5120B 12ms
10.0.1.9 GET /b 404 312B 1450ms
192.168.0.7 POST /c 500 2MB 95ms
```

## Compare a value

`\R{>1s}` is a duration above a second, `\Z{>=1MB}` a size of a megabyte or more, `\N{500..599}`
a number in that range, and `\I{in:10.0.0.0/16}` an address inside that network:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\R{>1s}' req.log
[56..62] "1450ms"

$ trex scan '\Z{>=1MB}' req.log
[87..90] "2MB"

$ trex scan '\I{in:10.0.0.0/16}' req.log
[0..8] "10.0.0.5"
[31..39] "10.0.1.9"

$ trex scan '\N{500..599}' req.log
[83..86] "500"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = "10.0.0.5 GET /a 200 5120B 12ms\n10.0.1.9 GET /b 404 312B 1450ms\n192.168.0.7 POST /c 500 2MB 95ms\n";
let found = |pat: &str| -> Vec<&str> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, log.as_bytes()).iter().map(|s| &log[s.range()]).collect()
};
assert_eq!(found(r"\R{>1s}"), ["1450ms"]);
assert_eq!(found(r"\Z{>=1MB}"), ["2MB"]);
assert_eq!(found(r"\I{in:10.0.0.0/16}"), ["10.0.0.5", "10.0.1.9"]);
assert_eq!(found(r"\N{500..599}"), ["500"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> log = open("req.log").read()
>>> [[m.text for m in trex.Pattern(p).scan(log)] for p in [r"\R{>1s}", r"\Z{>=1MB}", r"\I{in:10.0.0.0/16}", r"\N{500..599}"]]
[['1450ms'], ['2MB'], ['10.0.0.5', '10.0.1.9'], ['500']]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> '\R{>1s}', '\Z{>=1MB}', '\I{in:10.0.0.0/16}', '\N{500..599}' | ForEach-Object { (Select-TrexMatch $_ -Path ./req.log -Raw) -join ', ' }
1450ms
2MB
10.0.0.5, 10.0.1.9
500
```
{{< /tab >}}
{{< /tabs >}}

## Keep the lines that hold one

A typed predicate is a pattern like any other, so it selects lines, counts and records:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\R{>1s} | \N{500..599}' req.log --count
2
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"10.0.0.5 GET /a 200 5120B 12ms\n10.0.1.9 GET /b 404 312B 1450ms\n192.168.0.7 POST /c 500 2MB 95ms\n";
let pat = trex::parse(r"\R{>1s} | \N{500..599}").expect("valid pattern");
let lines = trex::files::LineIndex::new(log);
let mut held: Vec<usize> = trex::scan(&pat, log).iter().map(|s| lines.line_of(s.start())).collect();
held.dedup();
assert_eq!(held.len(), 2);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> len({log.count("\n", 0, m.start) for m in trex.Pattern(r"\R{>1s} | \N{500..599}").scan(log)})
2
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Select-TrexMatch '\R{>1s} | \N{500..599}' -Path ./req.log -Count).Count
2
```
{{< /tab >}}
{{< /tabs >}}

A timestamp compares with the clock: `\T{age<24h}` is a timestamp within the last day, read
against `--now`, `trex.set_now` or `Set-TrexClock -Now` where one is fixed.
