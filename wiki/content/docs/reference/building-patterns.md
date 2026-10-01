---
title: Building patterns
linkTitle: Building patterns
weight: 80
---

# Building patterns

trex writes patterns from text: the templates a log's lines repeat, the most specific pattern
a set of examples shares, and a pattern that extracts named fields from every shape of a set
of lines, built from lines with the values marked. A built pattern is saved as a pattern file
and applied from then on with no build. The flags and parameters are on the
[CLI](../cli/#templates) and [PowerShell](../powershell/records/) pages.

## Templates

Records group by their token-kind silhouette. Within a group a position whose text is the
same in every record is a literal, and one whose text varies is a slot named by its kind, so a
log of requests reads as its handful of shapes rather than its thousands of lines. The
templates print most frequent first; several inputs are one stream.

```console
$ cat requests.log
10.0.0.1 GET /index.html status 200 12ms
10.0.0.2 GET /about.html status 200 8ms
10.0.0.1 POST /login status 302 40ms
10.0.0.3 GET /index.html status 200 11ms
10.0.0.9 GET /admin status 403 3ms
kernel: disk failure on /dev/sda
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex templates requests.log
5  <ip> <word> <path> status <number> <duration>
1  kernel: disk failure on /dev/sda

$ trex templates requests.log --pattern
5  \I \W \L "status" \N \R
1  "kernel" ":" "disk" "failure" "on" "/dev/sda"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"10.0.0.1 GET /index.html status 200 12ms
10.0.0.2 GET /about.html status 200 8ms
10.0.0.1 POST /login status 302 40ms
10.0.0.3 GET /index.html status 200 11ms
10.0.0.9 GET /admin status 403 3ms
kernel: disk failure on /dev/sda
";
let mining = trex::templates::Mining::mine(log);
let found: Vec<(usize, String)> = mining.templates.iter().map(|t| (t.count(), t.pattern())).collect();
assert_eq!(found[0], (5, r#"\I \W \L "status" \N \R"#.to_string()));
assert_eq!(found[1].0, 1);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [(t["count"], t["readable"], t["pattern"]) for t in trex.templates(open("requests.log").read())]
[(5, '<ip> <word> <path> status <number> <duration>', '\\I \\W \\L "status" \\N \\R'), (1, 'kernel: disk failure on /dev/sda', '"kernel" ":" "disk" "failure" "on" "/dev/sda"')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexRecordShape -Path ./requests.log | Select-Object Count, Readable, Pattern

Count Readable                                      Pattern
----- --------                                      -------
    5 <ip> <word> <path> status <number> <duration> \I \W \L "status" \N \R
    1 kernel: disk failure on /dev/sda              "kernel" ":" "disk" "failure" "on" "/dev/sda"
```
{{< /tab >}}
{{< /tabs >}}

`--pattern` writes each template as a pattern `scan` accepts, literals quoted and slots as
atoms, and each finds exactly its template's lines.

### Rare templates

A template is rare when it covers fewer lines than the mean template does - the lines with a
template divided by the distinct templates - so the cut moves with the input. `--cut 5` makes
it fewer than five lines and `--cut 1%` less than one percent of the lines. `--rare` prints the
rare templates alone. The same reading is the `@shape:rare` anchor, which holds at every token
of a rare line.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex templates requests.log --rare
1  kernel: disk failure on /dev/sda

$ trex scan '@shape:rare \W' requests.log
[194..200] "kernel"
[202..206] "disk"
[207..214] "failure"
[215..217] "on"
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [t["readable"] for t in trex.templates(open("requests.log").read()) if t["rare"]]
['kernel: disk failure on /dev/sda']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexRecordShape -Path ./requests.log -Rare | Select-Object Count, Readable

Count Readable
----- --------
    1 kernel: disk failure on /dev/sda
```
{{< /tab >}}
{{< /tabs >}}

### Records of several lines

`--record UNIT` groups records rather than lines, taking the [record units](../records/#record-units),
and `--record-start` and `--record-span` say where a record begins or what one is. A record of
several lines has one template spanning all of them.

```console
$ cat stanzas.log
job alpha
status ok
took 12s

job bravo
status ok
took 30s

job delta
status failed
took 4s
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex templates stanzas.log
6  <word> <word>
3  took <duration>

$ trex templates stanzas.log --record paragraph
3  job <word>
status <word>
took <duration>
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;

let log = b"job alpha\nstatus ok\ntook 12s\n\njob bravo\nstatus ok\ntook 30s\n\njob delta\nstatus failed\ntook 4s\n";
let records = RecordUnit::Paragraph.records(log);
let mining = trex::templates::Mining::mine_records(&trex::lexer::lex(log), log, &records);
assert_eq!(mining.templates.len(), 1);
assert_eq!(mining.templates[0].count(), 3);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexRecordShape -Path ./stanzas.log -Unit paragraph | Select-Object Count, Readable

Count Readable
----- --------
    3 job <word>…
```
{{< /tab >}}
{{< /tabs >}}

### Against another input

`--against FILE` mines a second input the same way and marks each template of the first
`shared` or `novel`: shared when the other input holds a template that would accept these
records - the same silhouette, a literal matching that literal and a slot admitting anything of
its kind. The reading is asymmetric: a log that has seen one name at a position accepts only
that name, and one that has seen several accepts them all. `--novel` prints only the novel
ones.

```console
$ cat today.log
user bob logged in
user amy logged in
disk sda ok
disk sdb ok
quota 90 exceeded
$ cat yesterday.log
user carl logged in
user dana logged in
user eve logged in
disk sda ok
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex templates today.log --against yesterday.log
novel   2  disk <word> ok
shared  2  user <word> logged in
novel   1  quota 90 exceeded

$ trex templates today.log --against yesterday.log --novel
novel   2  disk <word> ok
novel   1  quota 90 exceeded
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let today = trex::templates::Mining::mine(b"user bob logged in\nuser amy logged in\ndisk sda ok\ndisk sdb ok\nquota 90 exceeded\n");
let yesterday = trex::templates::Mining::mine(b"user carl logged in\nuser dana logged in\nuser eve logged in\ndisk sda ok\n");
let shared = today.shared_with(&yesterday);
let novel: Vec<String> = today.templates.iter().zip(&shared).filter(|(_, s)| !**s).map(|(t, _)| t.readable()).collect();
assert_eq!(novel, ["disk <word> ok", "quota 90 exceeded"]);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Get-TrexRecordShape -Path ./today.log -Against ./yesterday.log | Select-Object Count, Readable, Novel

Count Readable              Novel
----- --------              -----
    2 disk <word> ok         True
    2 user <word> logged in False
    1 quota 90 exceeded      True
```
{{< /tab >}}
{{< /tabs >}}

`disk <word> ok` is novel because the other log has seen only `sda` at that position, so its
template would not accept `sdb`; `user <word> logged in` is shared because the other log has
seen several names there.

## A pattern from examples

`infer` writes the most specific pattern every example matches, read off an alignment of the
examples' tokens: a position where every example has the same text is that literal, one where
the kinds agree and the texts differ is the kind's atom, one where the kinds differ is the
class of the kinds seen, a run of one kind whose length differs is that kind repeated with the
bounds seen, and a position some examples lack is optional. The pattern is verified against
every example before it is returned.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer '10.0.0.1 GET /index.html status 200 12ms' '10.0.0.2 POST /login status 302 40ms'
\I \W \L "status" \N \R

$ trex infer 'id 200 ok' 'id abc ok'
"id" [\N \W] "ok"

$ trex infer 'disk failure on sda' 'disk failed sda'
"disk" \W{1,2} "sda"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let inferred = trex::infer::infer(&[b"id 200 ok", b"id abc ok"]).expect("a pattern");
assert_eq!(inferred.pattern(), r#""id" [\N \W] "ok""#);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.infer(["10.0.0.1 GET /index.html status 200 12ms", "10.0.0.2 POST /login status 302 40ms"])
'\\I \\W \\L "status" \\N \\R'
>>> trex.infer(["id 200 ok", "id abc ok"])
'"id" [\\N \\W] "ok"'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> ConvertTo-TrexPattern '10.0.0.1 GET /index.html status 200 12ms', '10.0.0.2 POST /login status 302 40ms'
\I \W \L "status" \N \R

PS> ConvertTo-TrexPattern 'id 200 ok', 'id abc ok'
"id" [\N \W] "ok"
```
{{< /tab >}}
{{< /tabs >}}

A position whose texts differ but fold to one text under an orbit rung prints that text under
the rung: `case`, `notation`, `numeric`, `ip`, `url`, `time`, `path` and `fold`. `shape` and
`e8` are not among them, since they fold spans that only share a structure.

```console
$ trex infer 'GET /a' 'get /a'
(?orbit:case "get") "/" "a"
```

### Value ranges

A counter-example, `--not`, is what decides whether a position reports a value range or its
bare kind. With none every position reports its kind; where counter-examples are given, the
ranges that exclude them are added one at a time, most excluded first. A range rounds outward
to a boundary of the kind's own units: a number to the leading digit place of its larger bound,
an address to the block its bits share, a timestamp to the calendar unit holding every one, a
version to the line it sits on. A counter-example nothing separates is refused.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer 'code 200' 'code 204' --not 'code 500'
"code" \N{200..299}

$ trex infer 'from 10.0.1.4' 'from 10.0.9.7' --not 'from 192.168.0.1'
"from" \I{in:10.0.0.0/20}

$ trex infer 'v 1.2.0' 'v 1.9.3' --not 'v 2.0.0'
"v" \V{major=1}

$ trex infer 'code 200' 'code 204' --not 'code 201'
trex infer: the inferred pattern `"code" \N` still matches counter-example 1; nothing the examples have in common tells them apart
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let inferred = trex::infer::infer_against(&[b"code 200", b"code 204"], &[b"code 500"]).expect("a pattern");
assert_eq!(inferred.pattern(), r#""code" \N{200..299}"#);
assert!(trex::infer::infer_against(&[b"code 200", b"code 204"], &[b"code 201"]).is_err());
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.infer(["code 200", "code 204"], against=["code 500"])
'"code" \\N{200..299}'
>>> trex.infer(["v 1.2.0", "v 1.9.3"], against=["v 2.0.0"])
'"v" \\V{major=1}'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> ConvertTo-TrexPattern 'code 200', 'code 204' -NotExample 'code 500'
"code" \N{200..299}

PS> ConvertTo-TrexPattern 'v 1.2.0', 'v 1.9.3' -NotExample 'v 2.0.0'
"v" \V{major=1}
```
{{< /tab >}}
{{< /tabs >}}

`--anchored` adds `^` and `$`, so the pattern matches whole lines:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer 'user bob logged in' 'user amy logged in' --anchored
^ "user" \W "logged" $ "in"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let inferred = trex::infer::infer(&[b"user bob logged in", b"user amy logged in"]).expect("a pattern");
assert_eq!(inferred.anchored(), r#"^ "user" \W "logged" $ "in""#);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.infer(["user bob logged in", "user amy logged in"], anchored=True)
'^ "user" \\W "logged" $ "in"'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> ConvertTo-TrexPattern 'user bob logged in', 'user amy logged in' -Anchored
^ "user" \W "logged" $ "in"
```
{{< /tab >}}
{{< /tabs >}}

## A pattern that extracts fields

With fields marked, `infer` builds the pattern that extracts them from every shape of the
lines. A mark writes a value as `{name:text}`, the markup of PowerShell's `ConvertFrom-String`
templates, and `{[int]name:text}` gives its type; a field is also named by a value it takes,
found wherever it stands. Lines group by the kinds of their tokens: a group holding a field is
a shape, a group whose tokens align with a shape joins it, and a group aligning with none is a
shape of its own, its fields found where the literals around them in a marked shape stand,
once the line holds one of that shape's words. Each shape is a branch of the pattern, which
matches whole lines unless unanchored, and the pattern is verified before it is returned:
every line matched, every field read back as it was placed, no counter-example matched.

```console
$ cat titles.txt
2023-09 Cumulative Update Preview for Windows 11 Version 22H2 for x64-based Systems (KB5030310)
2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)
Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer --mark '{month:2023-10} Cumulative Update for Windows {os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})' -f titles.txt
pattern  ^ (\N "-" \N)?:month "Cumulative" "Update" "Preview"? "for" "Windows" (\N):os "Version" (\N \W?):version "for" "x64" "-" "based" "Systems" "(" (`KB[0-9]{7}`):kb ")" ~<($ .) | ^ (\N "-" \N):month "Security" "Monthly" "Quality" "Rollup" "for" "Windows" (\N):os "for" "x64" "-" "based" "Systems" "(" (`KB[0-9]{7}`):kb ")" ~<($ .)
format   ${month}\t${os}\t${version}\t${kb}

shape  lines  reads
1      1-2 4  month os version kb marked
2      3      month os kb by the literals of shape 1; no version

line  shape  month    os  version  kb         text
1     1      2023-10  11  22H2     KB5031354  2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB5031354)
2     1      2023-09  11  22H2     KB5030310  2023-09 Cumulative Update Preview for Windows 11 Version 22H2 for x64-based Systems (KB5030310)
3     2      2020-01  7   -        KB4534310  2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)
4     1      -        10  1607     KB4103720  Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::infer::build::{Mint, Spec, build};

let marked = trex::infer::marks::parse_lines(
    "{month:2023-10} Cumulative Update for Windows {os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})",
)
.expect("valid marks");
let lines = [
    "2023-09 Cumulative Update Preview for Windows 11 Version 22H2 for x64-based Systems (KB5030310)".to_string(),
    "2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)".to_string(),
    "Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)".to_string(),
];
let shapes = trex::ShapeSet::new();
let spec = Spec { lines: &lines, marked: &marked, hints: &[], counters: &[], shapes: &shapes, unanchored: false, mint: Mint::Inline };
let built = build(&spec).expect("a pattern");
assert_eq!(built.format(), r"${month}\t${os}\t${version}\t${kb}");
let kb: Vec<String> = built.rows.iter().map(|r| r.values[3].as_ref().expect("every title has a kb").joined()).collect();
assert_eq!(kb, ["KB5031354", "KB5030310", "KB4534310", "KB4103720"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> titles = open("titles.txt").read().splitlines()
>>> built = trex.infer(titles, marked=["{month:2023-10} Cumulative Update for Windows {os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})"])
>>> built.format
'${month}\\t${os}\\t${version}\\t${kb}'
>>> [r["values"] for r in built.rows][2:]
[{'month': '2020-01', 'os': '7', 'version': None, 'kb': 'KB4534310'}, {'month': None, 'os': '10', 'version': '1607', 'kb': 'KB4103720'}]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $titles = Get-Content ./titles.txt
PS> $built = $titles | ConvertTo-TrexPattern -Marked '{month:2023-10} Cumulative Update for Windows {[int]os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})'
PS> $titles | ConvertFrom-TrexText $built

month   os version kb
-----   -- ------- --
2023-09 11 22H2    KB5030310
2020-01  7         KB4534310
        10 1607    KB4103720
```
{{< /tab >}}
{{< /tabs >}}

The report gives the pattern, the `--format` template writing every field, each shape with its
lines and how it reaches each field, and what the pattern reads from every line. Two shapes
whose words differ only by words one of them lacks are one branch, those words optional, as the
Preview title is here. `--field NAME=VALUE` names a field by a value it takes instead of a mark,
`--marks FILE` reads marked lines from a file, and `--marked` reads marks in the lines
themselves. In Python `fields=` maps a name to a value or a list of them and
`marks_in_lines=True` reads marks in the examples; in PowerShell `-Field` takes a dictionary
and `-MarksInLines` reads marks in the examples. `ConvertFrom-TrexText -Marked` builds and
converts in one step, as `ConvertFrom-String -TemplateContent` does.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer --field status=200 'GET /index.html 200 12ms' 'POST /login 302 40ms' 'GET /missing 404 3ms' --pattern
^ \W \L (\N):status \R ~<($ .)
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.infer(["GET /index.html 200 12ms", "POST /login 302 40ms", "GET /missing 404 3ms"], fields={"status": "200"}).pattern
'^ \\W \\L (\\N):status \\R ~<($ .)'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (ConvertTo-TrexPattern 'GET /index.html 200 12ms', 'POST /login 302 40ms', 'GET /missing 404 3ms' -Field @{ status = '200' }).Pattern
^ \W \L (\N):status \R ~<($ .)

PS> 'apples 42', 'pears 7' | ConvertFrom-TrexText -Marked '{fruit:apples} {[int]qty:42}'

fruit  qty
-----  ---
apples  42
pears    7
```
{{< /tab >}}
{{< /tabs >}}

A `[type]` casts the value as PowerShell casts it, the cast `ConvertFrom-String` makes: `[int]`
an `Int32`, `[decimal]` a `Decimal`, and `[bool]` true for any text but the empty, `false`
included. A field a line lacks is `None` in Python, `-` in the report and `$null` in
PowerShell, where `ConvertFrom-String` leaves the property off.

### How a field is spelled

A field is never spelled as its text, since it is what varies: it is its kind, a group of kinds
with constant punctuation kept, the class of its kinds, its runs aligned, or free text to the
line's end where it ends the line, and optional where a line of its shape lacks it.

A field that is one word token, whose values split into the same runs of letters, digits and
punctuation with a run of digits among them and a run of letters the same in every value, is
spelled as that byte shape: every `kb` above is `KB` then seven digits, so `kb` is
`` `KB[0-9]{7}` ``, an inline byte atom matched whole against the token, and a title ending in
`(KB50313541)` is refused where `\W` would take it. Digit counts alone mint nothing, so `month`
and `os` stay `\N`. `--no-mint` spells the field as its kind; `--mint-shapes` declares the byte
shape as a named one, `shape kb = `KB[0-9]{7}``, and spells the field `\{kb}`, the pattern then
reading only beside its declarations.

A field the lexer reads as several tokens, whose every value a shape of the shipped library
reads as one, is that shape: `CVE-2023-1234` and `CVE-2024-56789` make the field `\{cve}`. The
shape must carry evidence of its own - a letter or digit it holds as written (`CVE-`, `AKIA`)
or a check that refuses a value with one character changed - and change no other token of the
lines it is tried on.

An unmarked column that holds a field's value in every line, while the field takes at least two
values, is written as a back-reference to it, `=tag`, so a line where the two differ is refused.
A value class of the shipped library (`\{log_level}`, `\{http_2xx}`, `\{month}`) is printed in a
field's place only when a counter-example needs it; otherwise the report names each class every
value of a field belongs to as a `suggest` line.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer --mark '<{tag:b}>bold</b>' '<i>it</i>' '<em>x</em>' --pattern
^ "<" (\W):tag ">" \W "<" "/" =tag ">" ~<($ .)

$ trex infer --mark '{level:ERROR} disk {n:5}' 'WARN disk 7' --not 'HELLO disk 9' --pattern
^ (\{log_level}):level "disk" (\N):n ~<($ .)
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.infer(["<i>it</i>", "<em>x</em>"], marked=["<{tag:b}>bold</b>"]).pattern
'^ "<" (\\W):tag ">" \\W "<" "/" =tag ">" ~<($ .)'
>>> trex.infer(["WARN disk 7"], marked=["{level:ERROR} disk {n:5}"], against=["HELLO disk 9"]).pattern
'^ (\\{log_level}):level "disk" (\\N):n ~<($ .)'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> ('<i>it</i>', '<em>x</em>' | ConvertTo-TrexPattern -Marked '<{tag:b}>bold</b>').Pattern
^ "<" (\W):tag ">" \W "<" "/" =tag ">" ~<($ .)
```
{{< /tab >}}
{{< /tabs >}}

### Lists, fields inside fields, and records

A field marked more than once in one line is a list: the words between its first two values are
the separator every later pair shares, and a line with one value holds a list of one. Its
template is `${ip[*]}`, every value joined with a comma.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer --mark 'from {ip:10.0.0.1} -> {ip:10.0.0.2} ok' 'from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok' 'from 10.0.0.3 ok'
pattern  ^ "from" (\I):ip ("-" ">" (\I):ip)* "ok" ~<($ .)
format   ${ip[*]}

shape  lines  reads
1      1-3    ip marked

line  shape  ip                          text
1     1      10.0.0.1,10.0.0.2           from 10.0.0.1 -> 10.0.0.2 ok
2     1      10.0.0.7,10.0.0.8,10.0.0.9  from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok
3     1      10.0.0.3                    from 10.0.0.3 ok
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::infer::build::{Mint, Spec, Value, build};

let marked = trex::infer::marks::parse_lines("from {ip:10.0.0.1} -> {ip:10.0.0.2} ok").expect("valid marks");
let lines = ["from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok".to_string(), "from 10.0.0.3 ok".to_string()];
let shapes = trex::ShapeSet::new();
let spec = Spec { lines: &lines, marked: &marked, hints: &[], counters: &[], shapes: &shapes, unanchored: false, mint: Mint::Inline };
let built = build(&spec).expect("a pattern");
assert_eq!(built.pattern, r#"^ "from" (\I):ip ("-" ">" (\I):ip)* "ok" ~<($ .)"#);
assert_eq!(built.rows[1].values[0], Some(Value::Many(vec!["10.0.0.7".into(), "10.0.0.8".into(), "10.0.0.9".into()])));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> hops = trex.infer(["from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok", "from 10.0.0.3 ok"], marked=["from {ip:10.0.0.1} -> {ip:10.0.0.2} ok"])
>>> [r["values"] for r in hops.rows]
[{'ip': ['10.0.0.1', '10.0.0.2']}, {'ip': ['10.0.0.7', '10.0.0.8', '10.0.0.9']}, {'ip': ['10.0.0.3']}]
>>> trex.Pattern(hops.pattern).find("from 1.1.1.1 -> 2.2.2.2 ok")["ip[*]"]
'1.1.1.1,2.2.2.2'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'ports 22 open', 'ports 8080, 8443, 9000 open' | ConvertFrom-TrexText -Marked 'ports {[int]port:80}, {[int]port:443} open'

port
----
{22}
{8080, 8443, 9000}
```
{{< /tab >}}
{{< /tabs >}}

A mark inside a mark is a field inside a field, named after the outer one, a dot, and its own
name, `Line.n`, as trex names a capture inside a capture. A template reads it as `${Line.n}`;
`--json` and Python give the outer field as an object of its `text` and the fields inside it,
and PowerShell as an object with a `Text` property and a property per field inside it.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer --mark '{Line:{[int]n:1} of {[int]m:3}}' '5 of 9' '6 of 9'
pattern  ^ ((\N):n "of" (\N):m):Line ~<($ .)
format   ${Line}\t${Line.n}\t${Line.m}

shape  lines  reads
1      1-3    Line Line.n Line.m marked

line  shape  Line    Line.n  Line.m  text
1     1      1 of 3  1       3       1 of 3
2     1      5 of 9  5       9       5 of 9
3     1      6 of 9  6       9       6 of 9
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> pages = trex.infer(["5 of 9", "6 of 9"], marked=["{Line:{[int]n:1} of {[int]m:3}}"])
>>> pages.rows[1]["values"]
{'Line': {'text': '5 of 9', 'n': '5', 'm': '9'}}
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $pages = '5 of 9', '6 of 9' | ConvertFrom-TrexText -Marked '{Line:{[int]n:1} of {[int]m:3}}'
PS> $pages[0].Line

Text   n m
----   - -
5 of 9 5 9
```
{{< /tab >}}
{{< /tabs >}}

A field written `{name*:text}` begins a record, as a `ConvertFrom-String` template writes it: a
line holding it starts one, the lines after it join it with the first value of each field kept,
and a line before the first record starts none. A template may run over several lines, and a
starred mark may span them, `{Person*:...}` around a name line and a phone line, its record's
`Person` then the text of each line joined with a newline. A template line marking a starred
field more than once repeats its record: each record of the line is one, and a field outside
the records describes the line, so every record of the line carries it.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer --mark 'Name: {Name*:Phoebe Cat}
Phone: {phone:425-123-6789}' 'Name: Elephant Wise' 'Phone: 425-888-7766' 'Name: Wise Owl'
pattern  ^ "Name" ":" (\W \W):Name ~<($ .) | ^ "Phone" ":" (.*? $ .):phone ~<($ .)
format   ${Name}\t${phone}

shape  lines  reads
1      1 3 5  Name marked; no phone
2      2 4    phone marked; no Name

line  shape  Name           phone         text
1     1      Phoebe Cat     -             Name: Phoebe Cat
2     2      -              425-123-6789  Phone: 425-123-6789
3     1      Elephant Wise  -             Name: Elephant Wise
4     2      -              425-888-7766  Phone: 425-888-7766
5     1      Wise Owl       -             Name: Wise Owl

record  lines  Name           phone
1       1-2    Phoebe Cat     425-123-6789
2       3-4    Elephant Wise  425-888-7766
3       5      Wise Owl       -

$ trex infer --mark 'day {day:Mon}: {Name*:Phoebe Cat} ({[int]age:6}); {Name*:Lucky Shot} ({[int]age:12}) end' 'day Tue: Wise Owl (87) end' 'day Wed: Elmo Red (3); Oscar Grouch (9); Big Bird (5) end' --pattern
^ "day" (\W):day ":" (\W \W):Name "(" (\N):age ")" (";" (\W \W):Name "(" (\N):age ")")* "end" ~<($ .)
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> people = trex.infer(["Name: Elephant Wise", "Phone: 425-888-7766", "Name: Wise Owl"], marked=["Name: {Name*:Phoebe Cat}\nPhone: {phone:425-123-6789}"])
>>> [r["values"] for r in people.records]
[{'Name': 'Phoebe Cat', 'phone': '425-123-6789'}, {'Name': 'Elephant Wise', 'phone': '425-888-7766'}, {'Name': 'Wise Owl', 'phone': None}]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $template = "Name: {Name*:Phoebe Cat}`nPhone: {phone:425-123-6789}"
PS> "Name: Elephant Wise`nPhone: 425-888-7766`nName: Wise Owl" | ConvertFrom-TrexText -Marked $template

Name          phone
----          -----
Elephant Wise 425-888-7766
Wise Owl

PS> 'day Tue: Wise Owl (87) end', 'day Wed: Elmo Red (3); Oscar Grouch (9) end' | ConvertFrom-TrexText -Marked 'day {day:Mon}: {Name*:Phoebe Cat} ({[int]age:6}); {Name*:Lucky Shot} ({[int]age:12}) end'

day Name         age
--- ----         ---
Tue Wise Owl      87
Wed Elmo Red       3
Wed Oscar Grouch   9
```
{{< /tab >}}
{{< /tabs >}}

`ConvertFrom-TrexText` writes one object per record, where the report and Python also list the
template's own record; a starred mark spanning lines with no mark on its first line is refused,
since nothing then says which line begins its record, and so are records that mark different
fields or stand with no word between them.

### Declared shapes

A pattern file's `shape` lines, given with `--lib`, `--shape` or `--shape-after`, lex the lines
under their declarations, so a declared shape is one token the built pattern names:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer --mark 'see {t:AB-12} now' 'see XYZ-9 now' --pattern
^ "see" (\W "-" \N):t "now" ~<($ .)

$ trex infer --mark 'see {t:AB-12} now' 'see XYZ-9 now' --shape 'ticket = `[A-Z]{2,4}-\d{1,4}`' --pattern
^ "see" (\{ticket}):t "now" ~<($ .)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::infer::build::{Mint, Spec, build};

let marked = trex::infer::marks::parse_lines("see {t:AB-12} now").expect("valid marks");
let lines = ["see XYZ-9 now".to_string()];
let mut shapes = trex::ShapeSet::new();
shapes.declare("ticket = `[A-Z]{2,4}-\\d{1,4}`", trex::Precedence::Before).expect("a bounded shape");
let spec = Spec { lines: &lines, marked: &marked, hints: &[], counters: &[], shapes: &shapes, unanchored: false, mint: Mint::Inline };
assert_eq!(build(&spec).expect("a pattern").pattern, r#"^ "see" (\{ticket}):t "now" ~<($ .)"#);
```
{{< /tab >}}
{{< /tabs >}}

## Saving and reusing a build

A built pattern is a pattern like any other, built once and applied from then on. The pattern
file a build writes - `--lib-file` on the command line, `file` in Python, `File` in
PowerShell, `Built::file` in Rust - holds a `let` per shape, one named `extract` joining them, a
`fields` line keeping what the marks said beyond the pattern, and a `test` line. The `fields`
line writes each field in order as its mark with the example text left out: its `[type]`, the
`*` of a field beginning a record, and the accessor of a field read from part of a token, as
`{host:host}` reads the host of a URL. Read with `--lib`, `\{extract}` then reads the records the
build read.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex infer --mark 'GET https://{host:example.com}/a {[int]code:200}' 'GET https://trex.dev/b 404' --lib-file
# Built by trex infer from 2 lines in 1 shape; `extract` names every shape in order.
let extract_1 = ^ "GET" (\U):host (\N):code ~<($ .)
let extract = \{extract_1}
fields extract {host:host} {[int]code}
test extract accepts "GET https://example.com/a 200"

$ cat hosts.trex
let extract_1 = ^ "GET" (\U):host (\N):code ~<($ .)
let extract = \{extract_1}
fields extract {host:host} {[int]code}

$ trex scan '\{extract}' --lib hosts.trex --text 'GET https://example.org/x 500' --fields
record  lines  host         code
1       1      example.org  500

$ trex scan '\{extract}' --lib hosts.trex --text 'GET https://example.org/x 500' --fields --json
[{"lines":[1],"values":{"host":"example.org","code":"500"}}]
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::infer::build::{Mint, Spec, build, fields_for, read_records};

let marked = trex::infer::marks::parse_lines("GET https://{host:example.com}/a {[int]code:200}").expect("valid marks");
let lines = ["GET https://trex.dev/b 404".to_string()];
let mut shapes = trex::ShapeSet::new();
let spec = Spec { lines: &lines, marked: &marked, hints: &[], counters: &[], shapes: &shapes, unanchored: false, mint: Mint::Inline };
let file = build(&spec).expect("a pattern").file();

shapes.declare_text(&file).expect("a pattern file");
let pattern = trex::parser::parse_with_shapes(r"\{extract}", &shapes).expect("valid pattern");
let fields = fields_for(r"\{extract}", &pattern, &shapes);
let records = read_records(&fields, &pattern, &shapes, "GET https://example.org/x 500").expect("records");
let host = fields.iter().position(|f| f.name == "host").expect("a host field");
assert_eq!(records[0].values[host].as_ref().expect("a host").joined(), "example.org");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> pets = trex.infer(["Name: Wise Owl", "Phone: 425-888-7766"], marked=["Name: {Name*:Phoebe Cat}\nPhone: {phone:425-123-6789}"])
>>> with open("pets.trex", "w") as f:
...     _ = f.write(pets.file)
>>> trex.Pattern(r"\{extract}", lib="pets.trex").records("Name: Big Bird\nPhone: 206-555-0100\nName: Elmo Red\n")
[{'lines': [0, 1], 'values': {'Name': 'Big Bird', 'phone': '206-555-0100'}}, {'lines': [2], 'values': {'Name': 'Elmo Red', 'phone': None}}]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $titles = Get-Content ./titles.txt
PS> $built = $titles | ConvertTo-TrexPattern -Marked '{month:2023-10} Cumulative Update for Windows {[int]os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})'
PS> $built.File | Set-Content ./updates.trex
PS> Import-TrexAtom ./updates.trex
PS> $titles | ConvertFrom-TrexText '\{extract}'

month   os version kb
-----   -- ------- --
2023-09 11 22H2    KB5030310
2020-01  7         KB4534310
        10 1607    KB4103720
```
{{< /tab >}}
{{< /tabs >}}

A pattern with no `fields` line has one field per register. A field beginning a record makes a
record of the lines that follow it, as it does for the build; `scan --format` with the template
the report prints writes the fields one line a match instead. The `fields` line is described
on the [pattern files](../pattern-files/#fields) page.
