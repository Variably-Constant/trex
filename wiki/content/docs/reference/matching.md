---
title: Matching
linkTitle: Matching
weight: 20
---

# Matching

Finding the matches of a pattern in text, in files and in directory trees, and the reports a
scan writes about them. The pattern language is the [pattern syntax](../pattern-syntax/)
page's; the flags and parameters are on the [CLI](../cli/#scan) and
[PowerShell](../powershell/matching/) pages.

The examples read these files:

```console
$ cat app.log
2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
$ cat access.log
10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
```

## Matches

A match is the leftmost, non-overlapping span of the input the pattern covers, with the text
each register bound in it.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\I:ip \W:verb' access.log
[0..12] "10.0.0.5 GET"  captures: ip="10.0.0.5", verb="GET"
[34..46] "10.0.0.7 GET"  captures: ip="10.0.0.7", verb="GET"
[60..72] "10.0.1.9 GET"  captures: ip="10.0.1.9", verb="GET"
[90..103] "10.0.0.5 POST"  captures: ip="10.0.0.5", verb="POST"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
";
let pat = trex::parse(r"\I:ip \W:verb").expect("valid pattern");
let spans = trex::scan(&pat, log);
let found: Vec<(usize, usize, &[u8])> = trex::captures(&pat, log, &spans)
    .iter()
    .map(|m| (m.start, m.end, m.group("verb", log).expect("verb is bound")))
    .collect();
assert_eq!(found[3], (90, 103, &b"POST"[..]));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> log = open("access.log").read()
>>> [(m.start, m.end, m["ip"], m["verb"]) for m in trex.Pattern(r"\I:ip \W:verb").scan(log)]
[(0, 12, '10.0.0.5', 'GET'), (34, 46, '10.0.0.7', 'GET'), (60, 72, '10.0.1.9', 'GET'), (90, 103, '10.0.0.5', 'POST')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\I:ip \W:verb' -Path ./access.log | Select-Object Start, Length, Text, @{ n = 'verb'; e = { $_.Captures.verb } }

Start Length Text          verb
----- ------ ----          ----
    0     12 10.0.0.5 GET  GET
   34     12 10.0.0.7 GET  GET
   60     12 10.0.1.9 GET  GET
   90     13 10.0.0.5 POST POST
```
{{< /tab >}}
{{< /tabs >}}

Offsets are bytes on the command line and in Rust. Python reports a `str` input in
characters, so `s[m.start:m.end] == m.text`, with `byte_start` and `byte_end` beside them;
PowerShell counts UTF-16 code units, so `$text.Substring($m.Start, $m.Length)` is the match.

## Typed values

A register binding one typed kind has a value: a number, a byte size, a duration, a
timestamp, an address, a version, money or a percentage, read in its base unit by the parse
a value predicate compares with.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\R:took' app.log --json --duration-unit ms
[{"start":59,"end":64,"text":"120ms","captures":{"took":{"text":"120ms","value":120}}},{"start":189,"end":195,"text":"1450ms","captures":{"took":{"text":"1450ms","value":1450}}},{"start":346,"end":350,"text":"95ms","captures":{"took":{"text":"95ms","value":95}}}]
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"2026-09-27T09:00:01Z INFO GET /v2/users from 10.0.0.5 took 120ms
2026-09-27T09:00:03Z INFO GET /v2/orders from 10.0.0.7 took 1450ms
";
let pat = trex::parse(r"\R:took").expect("valid pattern");
let names = pat.capture_names();
let kinds = pat.capture_kinds();
let style = trex::typed::ValueStyle { duration: trex::typed::DurationUnit::Milliseconds, ..Default::default() };
let took: Vec<String> = trex::captures(&pat, log, &trex::scan(&pat, log))
    .iter()
    .map(|m| {
        let v = trex::Matched::with_kinds(m, log, &names, &kinds).value("took").expect("a duration");
        v.json(trex::token::TokenKind::Duration, style, trex::typed::Clock::current())
    })
    .collect();
assert_eq!(took, ["120", "1450"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.value("took", unit="ms") for m in trex.Pattern(r"\R:took").scan(open("app.log").read())]
[120, 1450, 95]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\R:took' -Path ./app.log | ForEach-Object { $_.Groups[0].Value.TotalMilliseconds }
120
1450
95
```
{{< /tab >}}
{{< /tabs >}}

Each surface spells a value in its own type: a duration is nanoseconds in Rust and Python
(`unit="ms"` or `unit="s"` shifts it exactly) and a `TimeSpan` in PowerShell, a timestamp a
`datetime` or a `DateTimeOffset`, a number a `Decimal` or a `long`, an address an `int` of its
bits or its canonical text. On the command line `--values exact|natural|tagged` and
`--duration-unit ns|ms|s` choose how `--json` spells one.

## Files and directory trees

Any number of files and directories may follow the pattern. A directory is walked as
ripgrep walks one: `.gitignore`, `.ignore` and `.rgignore` rules apply, hidden entries are
skipped, and a file holding a NUL byte outside a UTF-16 or UTF-32 byte order mark is binary
and skipped. `--hidden`, `--no-ignore` and `--binary` widen the walk; a file named outright is
read whatever the ignore rules and filters say. A binary file named alone is refused, by name,
unless `--binary` asks for it. Over several inputs each match is prefixed with its path, line
and column, counted from one.

```console
$ cat tree/counts.txt
alpha 3
beta 11
$ cat tree/logs/a.log
from 10.0.0.1 at 09:14
retry once
$ cat tree/logs/b.log
queue drained
nothing to report
$ cat tree/notes/c.txt
mail bob@x.com about the rollout
$ cat tree/notes/d.txt
rollout notes for the second wave
check the queue before starting
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '"queue" | "rollout"' tree/logs/ tree/notes/
tree/logs/b.log:1:1: "queue"
tree/notes/c.txt:1:26: "rollout"
tree/notes/d.txt:1:1: "rollout"
tree/notes/d.txt:2:11: "queue"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
for (path, text) in [
    ("tree/logs/a.log", "from 10.0.0.1 at 09:14\nretry once\n"),
    ("tree/logs/b.log", "queue drained\nnothing to report\n"),
    ("tree/notes/c.txt", "mail bob@x.com about the rollout\n"),
    ("tree/notes/d.txt", "rollout notes for the second wave\ncheck the queue before starting\n"),
] {
    std::fs::create_dir_all(dir.join(path).parent().expect("a parent")).expect("a directory");
    std::fs::write(dir.join(path), text).expect("a file written");
}
std::env::set_current_dir(&dir).expect("the temporary directory");

let pat = trex::parse(r#""queue" | "rollout""#).expect("valid pattern");
let dirs = ["tree/logs/".to_string(), "tree/notes/".to_string()];
let (sources, errors) = trex::files::collect(&dirs, &trex::files::WalkOptions::default());
assert!(errors.is_empty());
let mut found = Vec::new();
for source in &sources {
    let bytes = trex::files::read_source(source).expect("a readable file");
    for span in trex::scan(&pat, &bytes) {
        found.push((source.name(), String::from_utf8_lossy(&bytes[span.range()]).into_owned()));
    }
}
assert_eq!(found[0], ("tree/logs/b.log".to_string(), "queue".to_string()));
assert_eq!(found.len(), 4);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> pat = trex.Pattern('"queue" | "rollout"')
>>> [(path, m.text) for path in trex.files(["tree/logs/", "tree/notes/"]) for m in pat.scan(trex.read(path))]
[('tree/logs/b.log', 'queue'), ('tree/notes/c.txt', 'rollout'), ('tree/notes/d.txt', 'rollout'), ('tree/notes/d.txt', 'queue')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '"queue" | "rollout"' -Path ./tree/logs, ./tree/notes | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, LineNumber, Column, Text

File  LineNumber Column Text
----  ---------- ------ ----
b.log          1      1 queue
c.txt          1     26 rollout
d.txt          1      1 rollout
d.txt          2     11 queue
```
{{< /tab >}}
{{< /tabs >}}

`trex::files::collect` returns the files in path order, with each error the walk met. Python's
`trex.files` lists the same files and raises `OSError` naming every error, and `trex.read`
gives a file's text decoded as a scan decodes it.

Globs and ripgrep's file types narrow a walk: `-g GLOB` keeps a file by a glob read against
its path under the directory walked and `!GLOB` drops one, `-t TYPE` keeps a type and
`-T TYPE` drops one, and `--files` lists the files a scan would read, without scanning them.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '"queue"' tree/logs/ tree/notes/ -g '*.txt'
tree/notes/d.txt:2:11: "queue"

$ trex scan --files tree/logs/ tree/notes/ -t log
tree/logs/a.log
tree/logs/b.log
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
for (path, text) in [
    ("tree/logs/a.log", "from 10.0.0.1 at 09:14\nretry once\n"),
    ("tree/logs/b.log", "queue drained\nnothing to report\n"),
    ("tree/notes/c.txt", "mail bob@x.com about the rollout\n"),
    ("tree/notes/d.txt", "rollout notes for the second wave\ncheck the queue before starting\n"),
] {
    std::fs::create_dir_all(dir.join(path).parent().expect("a parent")).expect("a directory");
    std::fs::write(dir.join(path), text).expect("a file written");
}
std::env::set_current_dir(&dir).expect("the temporary directory");

let walk = trex::files::WalkOptions { types: vec!["log".to_string()], ..Default::default() };
let (sources, _) = trex::files::collect(&["tree/logs/".to_string(), "tree/notes/".to_string()], &walk);
let names: Vec<String> = sources.iter().map(trex::files::Source::name).collect();
assert_eq!(names, ["tree/logs/a.log", "tree/logs/b.log"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> queue = trex.Pattern('"queue"')
>>> [(path, m.text) for path in trex.files(["tree/logs/", "tree/notes/"], globs=["*.txt"]) for m in queue.scan(trex.read(path))]
[('tree/notes/d.txt', 'queue')]
>>> trex.files(["tree/logs/", "tree/notes/"], types=["log"])
['tree/logs/a.log', 'tree/logs/b.log']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '"queue"' -Path ./tree/logs, ./tree/notes -Include '*.txt' | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, LineNumber, Text

File  LineNumber Text
----  ---------- ----
d.txt          2 queue

PS> Get-TrexFile ./tree/logs, ./tree/notes -FileType log | Split-Path -Leaf
a.log
b.log
```
{{< /tab >}}
{{< /tabs >}}

`-l` names each input holding a match and `-L` each holding none; `--sort` orders the inputs
by `path`, `modified`, `accessed` or `created`, `--sortr` largest first.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan -l '\E' tree/logs/ tree/notes/
tree/notes/c.txt

$ trex scan -L '\E' tree/logs/ tree/notes/ --sortr path
tree/notes/d.txt
tree/logs/b.log
tree/logs/a.log
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
for (path, text) in [
    ("tree/logs/a.log", "from 10.0.0.1 at 09:14\nretry once\n"),
    ("tree/logs/b.log", "queue drained\nnothing to report\n"),
    ("tree/notes/c.txt", "mail bob@x.com about the rollout\n"),
    ("tree/notes/d.txt", "rollout notes for the second wave\ncheck the queue before starting\n"),
] {
    std::fs::create_dir_all(dir.join(path).parent().expect("a parent")).expect("a directory");
    std::fs::write(dir.join(path), text).expect("a file written");
}
std::env::set_current_dir(&dir).expect("the temporary directory");

let email = trex::parse(r"\E").expect("valid pattern");
let sort = trex::files::Sort { key: trex::files::SortKey::Path, reverse: true };
let walk = trex::files::WalkOptions { sort: Some(sort), ..Default::default() };
let (sources, _) = trex::files::collect(&["tree/logs/".to_string(), "tree/notes/".to_string()], &walk);
let holds = |s: &trex::files::Source| trex::is_match(&email, &trex::files::read_source(s).expect("a readable file"));
let with: Vec<String> = sources.iter().filter(|s| holds(s)).map(trex::files::Source::name).collect();
let without: Vec<String> = sources.iter().filter(|s| !holds(s)).map(trex::files::Source::name).collect();
assert_eq!(with, ["tree/notes/c.txt"]);
assert_eq!(without, ["tree/notes/d.txt", "tree/logs/b.log", "tree/logs/a.log"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> email = trex.Pattern(r"\E")
>>> [path for path in trex.files(["tree/logs/", "tree/notes/"]) if email.is_match(trex.read(path))]
['tree/notes/c.txt']
>>> [path for path in trex.files(["tree/logs/", "tree/notes/"], sort="path", reverse=True) if not email.is_match(trex.read(path))]
['tree/notes/d.txt', 'tree/logs/b.log', 'tree/logs/a.log']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\E' -Path ./tree/logs, ./tree/notes -FilesWithMatches | Split-Path -Leaf
c.txt

PS> Select-TrexMatch '\E' -Path ./tree/logs, ./tree/notes -FilesWithoutMatch -Sort Path -Descending | Split-Path -Leaf
d.txt
b.log
a.log
```
{{< /tab >}}
{{< /tabs >}}

## Context

`-A N`, `-B N` and `-C N` print N lines after, before or around each match's first line,
with `--` between groups that do not touch. In PowerShell `-Context` fills each match's
`PreContext` and `PostContext`.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N{>=1000}' access.log -C 1
1:30: "5120"
2-10.0.0.7 GET /login 302 0
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N{>=1000}' -Path ./access.log -Context 1 | Select-Object LineNumber, Text, PreContext, PostContext

LineNumber Text PreContext PostContext
---------- ---- ---------- -----------
         1 5120 {}         {10.0.0.7 GET /login 302 0}
```
{{< /tab >}}
{{< /tabs >}}

A record unit in place of the number prints the whole construct the match sits in, whatever
its line count: `block` the innermost balanced bracket group, `unit` the supertoken,
`paragraph` the run of non-blank lines, and `record` whatever `--record`, `--record-start` or
`--record-span` defines. `-C` prints both sides of the construct, `-B` from its start to the
match's line and `-A` from that line to its end.

```console
$ cat body.txt
header line
fn outer(a) {
  let v = inner(a, 42);
  return 99;
}
trailer line
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N{99..99}' body.txt -C block
2-fn outer(a) {
3-  let v = inner(a, 42);
4:10: "99"
5-}

$ trex scan '\N{99..99}' body.txt -B block
2-fn outer(a) {
3-  let v = inner(a, 42);
4:10: "99"

$ trex scan '\N{42..42}' body.txt -C block
3:20: "42"
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N{99..99}' -Path ./body.txt -Context block | Select-Object LineNumber, Text, PreContext, PostContext

LineNumber Text PreContext                               PostContext
---------- ---- ----------                               -----------
         4 99   {fn outer(a) {,   let v = inner(a, 42);} {}}
```
{{< /tab >}}
{{< /tabs >}}

`42` sits in `inner(a, 42)`, so its block is that call and not the body around it. A match
inside no construct of the unit reports its own line, and a word that names no unit is
refused:

```console
$ trex scan '\N' body.txt -C nonsense
trex: the context flags take a number or a record unit: "nonsense" is not a record unit; write line, paragraph, file, period, seam, bind, bind:Q, auto, texture, shape, block, unit or unit:ROLE
```

## Selecting lines

`-v` prints the lines no match touches, `-x` keeps only a match that covers its line from the
first non-whitespace byte to the last, `-m N` stops after N matches of each input, and
`--passthru` prints every line with the matched ones marked.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan -v '"INFO"' app.log
2026-09-27T09:00:02Z WARN retrying the primary after 3 failures
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined

$ trex scan -x '\W' --text 'alpha
beta gamma'
[0..5] "alpha"

$ trex scan -m 2 '\I' access.log
[0..8] "10.0.0.5"
[34..42] "10.0.0.7"

$ trex scan --passthru -H '"POST"' access.log
access.log-1-10.0.0.5 GET /index.html 200 5120
access.log-2-10.0.0.7 GET /login 302 0
access.log-3-10.0.1.9 GET /missing 404 312
access.log:4:10.0.0.5 POST /login 200 88
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '"INFO"' -Path ./app.log -NotMatch | Select-Object LineNumber

LineNumber
----------
         2
         4

PS> Select-TrexMatch '\W' -InputObject "alpha`nbeta gamma" -WholeLine -Raw
alpha

PS> Select-TrexMatch '\I' -Path ./access.log -MaxCount 2 -Raw
10.0.0.5
10.0.0.7

PS> Select-TrexMatch '"POST"' -Path ./access.log -Passthru -ColorDepth None
10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
```
{{< /tab >}}
{{< /tabs >}}

`-e PATTERN`, repeatable, and `-f FILE`, one pattern a line, join their patterns as an
alternation, `(P1) | (P2)`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan -e '\E' -e '\{card}' app.log
[235..244] "bob@x.com"
[258..277] "4111 1111 1111 1111"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined\n";
let pat = trex::parse(r"(\E) | (\{card})").expect("valid pattern");
let found: Vec<&[u8]> = trex::scan(&pat, log).iter().map(|s| &log[s.range()]).collect();
assert_eq!(found, [&b"bob@x.com"[..], b"4111 1111 1111 1111"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"(\E) | (\{card})").scan(open("app.log").read())]
['bob@x.com', '4111 1111 1111 1111']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '(\E) | (\{card})' -Path ./app.log -Raw
bob@x.com
4111 1111 1111 1111
```
{{< /tab >}}
{{< /tabs >}}

## Counts

`--count` prints how many lines of each input hold a match, `--count-matches` how many
matches it holds.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N' access.log --count
4

$ trex scan '\N' access.log --count-matches
8

$ trex scan '\I' tree/logs/ tree/notes/ --count
tree/logs/a.log:1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
";
let pat = trex::parse(r"\N").expect("valid pattern");
let spans = trex::scan(&pat, log);
let line_of = |at: usize| log[..at].iter().filter(|&&b| b == b'\n').count();
let mut lines: Vec<usize> = spans.iter().map(|s| line_of(s.start())).collect();
lines.dedup();
assert_eq!((lines.len(), spans.len()), (4, 8));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> log = open("access.log").read()
>>> found = trex.Pattern(r"\N").scan(log)
>>> len({log.count("\n", 0, m.start) for m in found}), len(found)
(4, 8)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Select-TrexMatch '\N' -Path ./access.log -Count).Count
4

PS> (Select-TrexMatch '\N' -Path ./access.log -CountMatches).Count
8
```
{{< /tab >}}
{{< /tabs >}}

Under `--record`, `--record-start` or `--record-span` the count is of records rather than
lines, and under `-v` of the lines that hold none; `-m N` stops a count at N.

## Report templates

`--format TEMPLATE` prints one line a match, rendered in the rewrite language: `${0}`,
`${name}` and `${name:accessor}` as a [rewrite](../rewriting/) writes them, and where the match
stands as `${path}`, `${line}`, `${col}`, `${start}` and `${end}`, each taking accessors
(`${path:name}`). `\t` and `\n` are a tab and a newline.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\I:ip \W:verb' access.log --format '${line}: ${verb} from ${ip:octet1-3}'
1: GET from 10.0.0
2: GET from 10.0.0
3: GET from 10.0.1
4: POST from 10.0.0
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
";
let pat = trex::parse(r"\I:ip \W:verb").expect("valid pattern");
let names = pat.capture_names();
let rows: Vec<String> = trex::captures(&pat, log, &trex::scan(&pat, log))
    .iter()
    .map(|m| {
        let m = trex::Matched::new(m, log, &names);
        let line = log[..m.start()].iter().filter(|&&b| b == b'\n').count() + 1;
        format!("{line}: {} from {}", m.get("verb").expect("bound"), m.get("ip:octet1-3").expect("an address"))
    })
    .collect();
assert_eq!(rows[3], "4: POST from 10.0.0");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> log = open("access.log").read()
>>> [f'{log.count(chr(10), 0, m.start) + 1}: {m["verb"]} from {m["ip:octet1-3"]}' for m in trex.Pattern(r"\I:ip \W:verb").scan(log)]
['1: GET from 10.0.0', '2: GET from 10.0.0', '3: GET from 10.0.1', '4: POST from 10.0.0']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\I:ip \W:verb' -Path ./access.log -Format '${line}: ${verb} from ${ip:octet1-3}'
1: GET from 10.0.0
2: GET from 10.0.0
3: GET from 10.0.1
4: POST from 10.0.0
```
{{< /tab >}}
{{< /tabs >}}

A template may also name a reading `--explain` computes for the match, written with an `@`:
the kinds it spans as `${@kind}`, the guard each guarded kind passed as `${@guard}`, the rung
that answered as `${@route}`, and every axis the pattern reads - `${@magnitude}`,
`${@baseline}`, `${@spectral}`, `${@echo}`, `${@order}`, `${@template}`, `${@nesting}`,
`${@seam}`, `${@ambiguous}`, `${@gravity}`, `${@construct}`, `${@phase}`, `${@field}` and
`${@join}`. A bare axis renders the sentence `--explain` prints for it; a dot names one value
of it (`${@spectral.entropy}`, `${@echo.count}`, `${@construct.role}`), and an index picks the
reading at one token of the match. An axis the pattern never read renders empty, and a name
that reads no axis is refused.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\W \N' --format '${0} kinds=${@kind}' --text 'code 200'
code 200 kinds=word, number

$ trex scan '\M{>0} \M{>0}' --format 'all=${@magnitude} first=${@magnitude[0]}' --text '12 3400'
all=1.08, 3.53 first=1.08

$ trex scan '@super:call \W' --format '${@construct.role} at depth ${@construct.depth}' --text 'foo(a) bar(b)'
call at depth 0
call at depth 0

$ trex scan '\W' --format '${@entrpoy}' --text 'alpha'
trex: --format error at byte 0: ${@entrpoy} reads no axis; the axes are kind, guard, route, magnitude, baseline, spectral, echo, order, template, nesting, seam, ambiguous, gravity, construct, phase, join, field
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\W \N' -InputObject 'code 200' -Format '${0} kinds=${@kind}'
code 200 kinds=word, number
```
{{< /tab >}}
{{< /tabs >}}

## Explanations

An explanation says why a match matched: the kind and text of every token it spans, what each
guarded kind passed to be that kind, the value of every axis the pattern reads at those
tokens, and the rung of the scan ladder that answered.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{card}' --explain --text 'pay 4111 1111 1111 1111 now'
[4..23] "4111 1111 1111 1111"
  tokens: creditcard "4111 1111 1111 1111"
  guard: creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check
  route: a route, not the engine

$ trex scan '\N{>+1}' --explain --text 'sizes 12 15 9 4000'
[14..18] "4000"
  tokens: number "4000"
  magnitude: 3.60 "4000"
  baseline: window: mean 1.38, spread 0.55, over 4 "4000"
  route: the set engine over a whole lex
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let pat = trex::parse(r"\{card}").expect("valid pattern");
let text = b"pay 4111 1111 1111 1111 now";
trex::trace::clear();
let route = {
    let _recording = trex::trace::Recording::start();
    let _ = trex::scan_with_backend(&pat, text, trex::Backend::Auto);
    trex::explain::route_of(&trex::trace::take_recorded())
};
let m = &trex::captures(&pat, text, &trex::scan(&pat, text))[0];
let e = trex::explain::Explainer::new(&pat, text, &trex::ShapeSet::new()).explain(m, &route);
assert_eq!(e.tokens, [("creditcard".to_string(), "4111 1111 1111 1111".to_string())]);
assert_eq!(e.guards, ["creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check"]);
assert_eq!(e.route, "a route, not the engine");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> text = "pay 4111 1111 1111 1111 now"
>>> e = trex.Pattern(r"\{card}").find(text).explain(text)
>>> e["tokens"], e["guards"], e["route"]
([('creditcard', '4111 1111 1111 1111')], ["creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check"], 'a route, not the engine')
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $e = (Select-TrexMatch '\{card}' -InputObject 'pay 4111 1111 1111 1111 now' -Explain).Explanation
PS> $e.Tokens

Kind       Text
----       ----
creditcard 4111 1111 1111 1111

PS> $e.Guards
creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check
PS> $e.Route
a route, not the engine
```
{{< /tab >}}
{{< /tabs >}}

`--json` carries the same under `explain`. An explanation is computed over the whole input,
so a scan that asks for one does not stream.

## Pattern sets

A set is several patterns asked together over one lex, each match reported under the member
that made it. In a pattern file a `let NAME = PATTERN` line is a member under its name and a
bare pattern line a member under its line number; the file's `kind` and `shape` lines serve
every member.

```console
$ cat rules.trex
# what a line may hold
let host = \I:addr
let mail = \E:e
\N{>=100}
$ cat notes.txt
from 10.0.0.1 at 500 to bob@x.com
x = 7 and 10.0.0.2
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --patterns rules.trex notes.txt
[5..13] "10.0.0.1"  captures: addr="10.0.0.1"  pattern: host
[17..20] "500"  pattern: 4
[24..33] "bob@x.com"  captures: e="bob@x.com"  pattern: mail
[44..52] "10.0.0.2"  captures: addr="10.0.0.2"  pattern: host

$ trex scan --patterns rules.trex --single-match notes.txt
[5..13] "10.0.0.1"  captures: addr="10.0.0.1"  pattern: host
[17..20] "500"  pattern: 4
[24..33] "bob@x.com"  captures: e="bob@x.com"  pattern: mail
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let rules = r"# what a line may hold
let host = \I:addr
let mail = \E:e
\N{>=100}
";
let notes = b"from 10.0.0.1 at 500 to bob@x.com\nx = 7 and 10.0.0.2\n";
let mut shapes = trex::ShapeSet::new();
let set = trex::PatternSet::from_text(rules, &mut shapes).expect("a valid pattern file");
let found: Vec<(String, &[u8])> = set
    .scan(notes)
    .iter()
    .map(|(member, span)| (set.name(*member), &notes[span.range()]))
    .collect();
assert_eq!(found[1], ("4".to_string(), &b"500"[..]));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> rules = trex.PatternSet.from_file("rules.trex")
>>> [(rules.names[i], m.text) for i, m in rules.scan(open("notes.txt").read())]
[('host', '10.0.0.1'), ('4', '500'), ('mail', 'bob@x.com'), ('host', '10.0.0.2')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch -PatternFile ./rules.trex -Path ./notes.txt | Select-Object Pattern, Text

Pattern Text
------- ----
host    10.0.0.1
4       500
mail    bob@x.com
host    10.0.0.2

PS> Select-TrexMatch -PatternFile ./rules.trex -Path ./notes.txt -SingleMatch | Select-Object Pattern, Text

Pattern Text
------- ----
host    10.0.0.1
4       500
mail    bob@x.com
```
{{< /tab >}}
{{< /tabs >}}

Patterns given one by one form a set too, each named by its text in PowerShell
(`Select-TrexMatch '\E', '\I'`) and by its index in Rust and Python (`PatternSet([...])`). In
`--format` a member is `${pattern}`; a register of another member renders empty. A set scans
on the CPU engines.

## Exit status

`--require-match` exits non-zero when nothing matches, `-RequireMatch` writes an error, and
`Test-TrexMatch` answers whether anything matches.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{uuid}' app.log --require-match
no match
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\{uuid}").expect("valid pattern");
assert!(trex::scan(&pat, b"no identifiers here").is_empty());
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\{uuid}").is_match(open("app.log").read())
False
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Test-TrexMatch '\{uuid}' -Path ./app.log
False
```
{{< /tab >}}
{{< /tabs >}}

## Color

A report printed to a terminal is painted: the match bold red, the path magenta, the line and
column green, the `:` and `-` separators cyan, each in the console's own base colors, so its
theme decides the hue. `--color auto` (the default) paints at the depth the terminal renders,
`never` paints nothing, `always` what the environment says, and `16`, `256` or `truecolor`
force a depth. The depth is read from the environment: nothing under `NO_COLOR`, off a
terminal or with `TERM=dumb`; 24-bit under `COLORTERM=truecolor` or `24bit`, Windows
Terminal, VS Code, kitty or alacritty; 256 where `TERM` says `256color`; sixteen otherwise.

The token kinds are painted too. Each kind has one 24-bit color on an entry of the 256-color
cube, so a 256-color console renders what a 24-bit one does; at sixteen colors each kind
takes its family's color:

| Family | Kinds | At sixteen colors |
| --- | --- | --- |
| where | `ip` `cidr` `mac` `url` `email` `phone` | blue |
| | `path` `geo` | cyan |
| how much | `number` `percent` `bytesize` `money` `duration` `quantity` | yellow |
| when | `timestamp` | green |
| which | `uuid` `version` `hexcolor` `quoted` | magenta |
| alarm | `creditcard` `jwt` `base64` `hash` | red, underlined |

Words, punctuation, brackets and whitespace are left plain. `--colors SPEC`, repeatable,
paints a role as ripgrep spells it - `match:fg:red`, `path:bg:#202020`, `line:style:bold`,
`separator:none` for the roles `match`, `path`, `line`, `column` and `separator` - and
`kind:NAME:...` a kind, `kind:*:none|values|all` how much of a line the kinds paint (`values`
by default), and `capture:NAME:...` a register. A spec naming one kind wins over the scheme
and the level; `TREX_KIND_COLOR` sets kind specs for every command, and `--colors` wins over
it. In PowerShell `-Color` writes the painted report, `-ColorDepth` names the depth and
`-Colors` takes the same specs.

## Statistics

`--stats` prints, after the report, ripgrep's eight lines - matches, matched lines, files
with matches, files searched, bytes printed, bytes searched, seconds spent searching and
seconds - then the tokens lexed, how the searching time split between lexing and matching,
the bytes a device scanned, and how many inputs each rung of the scan ladder answered;
`--stats=line` prints one line. The times are the run's own.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N{>=1000}' access.log --stats=line
[29..33] "5120"
1 matches in 1 of 1 files, 118 bytes searched, 6.967 ms
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\I' -Path ./access.log -Stats | Select-Object -Last 1 | Select-Object Matches, MatchedLines, FilesSearched, BytesSearched

Matches MatchedLines FilesSearched BytesSearched
------- ------------ ------------- -------------
      4            4             1           118
```
{{< /tab >}}
{{< /tabs >}}

A scan reporting `0 tokens lexed` was answered by a byte route from the bytes alone. The
lexing time is summed over the lexes, which a large input runs one per chunk at once; the
matching time is the wall-clock searching time less it. The counting runs only while
`--stats` asks for it.

## Texture

`--texture KIND` keeps a walked file whose dominant region kind is `table`, `blob`, `prose`,
`numeric`, `code` or `mixed`, and `!KIND` drops one. A file's kind is the region kind covering
most of its bytes, as the [shape](../axes/shape/#region-fusion) and [spectral](../axes/spectral/)
axes read it: a region more than half of whose bytes repeat a strong shape period is a table,
and the rest take their spectral texture. A bare `--texture` with `--files` names each file's
kind. A file named outright is read whatever it reads as.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --files --texture sample/
sample/app.log: mixed
sample/code.rs: code
sample/payload.b64: blob
sample/prose.md: prose
sample/rows.csv: table, period 7

$ trex scan --files --texture '!blob' sample/
sample/app.log
sample/code.rs
sample/prose.md
sample/rows.csv
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let rows = b"id,host,bytes,ms\n1,alpha,1024,12\n2,beta,2048,19\n3,gamma,4096,31\n4,delta,8192,44\n5,epsilon,16384,57\n6,zeta,32768,73\n";
let kind = trex::shape::dominant_kind(rows);
assert_eq!(kind, Some(trex::shape::RegionKind::Table(7)));
assert!(trex::shape::keeps_texture(&[("blob".to_string(), false)], kind));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(path, trex.texture(path=path)) for path in trex.files("sample/")]
[('sample/app.log', ('mixed', 0)), ('sample/code.rs', ('code', 0)), ('sample/payload.b64', ('blob', 0)), ('sample/prose.md', ('prose', 0)), ('sample/rows.csv', ('table', 7))]
>>> trex.files("sample/", texture_not=["blob"])
['sample/app.log', 'sample/code.rs', 'sample/prose.md', 'sample/rows.csv']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexFile ./sample -Classify | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, Texture, Period

File        Texture Period
----        ------- ------
app.log       Mixed      0
code.rs        Code      0
payload.b64    Blob      0
prose.md      Prose      0
rows.csv      Table      7

PS> Get-TrexFile ./sample -ExcludeTexture Blob | Split-Path -Leaf
app.log
code.rs
prose.md
rows.csv
```
{{< /tab >}}
{{< /tabs >}}

## Engines

Every engine finds the same matches. `--cpu` never probes a device, `--gpu` forces the device
backend and falls back to the CPU with a warning where it cannot take the scan,
`--dual-grain` runs the byte and token grains as a pipeline, and `--chunk-size N` feeds the
input in N-byte chunks through the [stream scanner](../tools/#streams). Without a flag a large
enough eligible input goes to the device where one is present.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\I' access.log --chunk-size 7
[0..8] "10.0.0.5"
[34..42] "10.0.0.7"
[60..68] "10.0.1.9"
[90..98] "10.0.0.5"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
";
let pat = trex::parse(r"\I").expect("valid pattern");
let whole = trex::scan(&pat, log);
assert_eq!(trex::scan_chunked(&pat, log.chunks(7)), whole);
assert_eq!(trex::scan_dual_grain(&pat, log).0, whole);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\I' -Path ./access.log -ChunkSize 7 -Raw
10.0.0.5
10.0.0.7
10.0.1.9
10.0.0.5
```
{{< /tab >}}
{{< /tabs >}}
