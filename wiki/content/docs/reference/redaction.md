---
title: Redaction
linkTitle: Redaction
weight: 50
---

A redaction masks every match, leaving the fields it is told to keep unmasked. A kept
field is written as a reference is written in a template - `card:last4`, `ip:octet1-2`,
`email:domain`, or `0:last4` for the whole match - and must slice rather than transform, so
`upper` is refused. The pattern's guarded kinds decide what is masked: a run of digits is a
card only under the Luhn check, so a number that only looks like one is left alone. The flags
and parameters are on the [CLI](../cli/#redact) and
[PowerShell](../powershell/rewriting/#protect-trextext) pages.

## Mask and keep

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex redact '(\{card}:card | \I:ip | \E:email)' --keep 'card:last4, ip:octet1-2, email:domain' --text 'card 4111 1111 1111 1111 from 10.1.2.3 by bob@corp.example, ref 1234 5678 1234 5678'
card ***************1111 from 10.1**** by ****corp.example, ref 1234 5678 1234 5678
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::{Keep, Mask, redactions};

let text = b"card 4111 1111 1111 1111 from 10.1.2.3 by bob@corp.example, ref 1234 5678 1234 5678";
let pat = trex::parse(r"(\{card}:card | \I:ip | \E:email)").expect("valid pattern");
let keeps = Keep::parse_list("card:last4, ip:octet1-2, email:domain", &pat.capture_names()).expect("valid fields");
let mut mask = Mask::parse("*").expect("a mask");
let matches = trex::captures(&pat, text, &trex::scan(&pat, text));
let mut out = Vec::new();
let mut at = 0;
for edit in redactions(text, &matches, &keeps, &mut mask) {
    out.extend_from_slice(&text[at..edit.start]);
    out.extend_from_slice(&edit.replacement);
    at = edit.end;
}
out.extend_from_slice(&text[at..]);
assert_eq!(out, b"card ***************1111 from 10.1**** by ****corp.example, ref 1234 5678 1234 5678");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> text = "card 4111 1111 1111 1111 from 10.1.2.3 by bob@corp.example, ref 1234 5678 1234 5678"
>>> trex.Pattern(r"(\{card}:card | \I:ip | \E:email)").redact(text, keep="card:last4, ip:octet1-2, email:domain")
'card ***************1111 from 10.1**** by ****corp.example, ref 1234 5678 1234 5678'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'card 4111 1111 1111 1111 from 10.1.2.3 by bob@corp.example, ref 1234 5678 1234 5678' | Protect-TrexText '(\{card}:card | \I:ip | \E:email)' -Keep card:last4, ip:octet1-2, email:domain
card ***************1111 from 10.1**** by ****corp.example, ref 1234 5678 1234 5678
```
{{< /tab >}}
{{< /tabs >}}

Each masked character becomes one `*`, so every offset and column after a match survives and
a redacted log lines up with the one it came from.

## Other masks

A mask of one character masks each character with it; a longer mask is a token each masked run
becomes. Two words are masks that read the run: `shape` masks every letter as `a` and every
digit as `0` and keeps every other byte, so the run keeps the shape its kind is recognized by
and the redacted copy lexes as the original did; `pseudonym` replaces each distinct value with
a stable name for its kind, numbered in order of first sight across the inputs of one run, so a
value that recurred still recurs and `@echo`, the joins and `templates` read the redacted copy
as they read the original. A declared shape or kind, and a library kind the pattern names, is
named as its declaration names it: a `customer` shape's values become `CUSTOMER_1`,
`CUSTOMER_2`.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex redact '\{card}:c' --keep c:last4 --mask '[card]' --text 'paid with 4111 1111 1111 1111 today'
paid with [card]1111 today

$ trex redact '\I' --mask shape --text 'from 10.4.5.6 and 10.9.9.9'
from 00.0.0.0 and 00.0.0.0

$ trex redact '\E' --mask shape --text 'user bob@x.com wrote'
user aaa@a.aaa wrote

$ trex redact '\I' --mask pseudonym --text 'from 10.4.5.6 to 10.9.9.9 and back to 10.4.5.6'
from IP_1 to IP_2 and back to IP_1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::{Mask, redactions};

let text = b"from 10.4.5.6 to 10.9.9.9 and back to 10.4.5.6";
let pat = trex::parse(r"\I").expect("valid pattern");
let mut mask = Mask::parse("pseudonym").expect("a mask");
let matches = trex::captures(&pat, text, &trex::scan(&pat, text));
let names: Vec<Vec<u8>> = redactions(text, &matches, &[], &mut mask).into_iter().map(|e| e.replacement).collect();
assert_eq!(names, [b"IP_1".to_vec(), b"IP_2".to_vec(), b"IP_1".to_vec()]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\{card}:c").redact("paid with 4111 1111 1111 1111 today", keep="c:last4", mask="[card]")
'paid with [card]1111 today'
>>> trex.Pattern(r"\I").redact("from 10.4.5.6 and 10.9.9.9", mask="shape")
'from 00.0.0.0 and 00.0.0.0'
>>> trex.Pattern(r"\E").redact("user bob@x.com wrote", mask="shape")
'user aaa@a.aaa wrote'
>>> trex.Pattern(r"\I").redact("from 10.4.5.6 to 10.9.9.9 and back to 10.4.5.6", mask="pseudonym")
'from IP_1 to IP_2 and back to IP_1'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'paid with 4111 1111 1111 1111 today' | Protect-TrexText '\{card}:c' -Keep c:last4 -Mask '[card]'
paid with [card]1111 today

PS> 'from 10.4.5.6 and 10.9.9.9' | Protect-TrexText '\I' -Mask shape
from 00.0.0.0 and 00.0.0.0

PS> 'user bob@x.com wrote' | Protect-TrexText '\E' -Mask shape
user aaa@a.aaa wrote

PS> 'from 10.4.5.6 to 10.9.9.9 and back to 10.4.5.6' | Protect-TrexText '\I' -Mask pseudonym
from IP_1 to IP_2 and back to IP_1
```
{{< /tab >}}
{{< /tabs >}}

A pseudonym is one token to the lexer. Because the numbering follows the order values are first
seen, a run under this mask reads its inputs in the order it walked them, so two runs of one
command over one tree agree.

## Files

One input is redacted to the standard output. `--in-place`, `--dry-run`, `--interactive`,
`-U`, `--explain`, `--show-skipped`, `-C N`, `--lib`, `--shape`, `--shape-after`, `--kind`,
`--let`, `--declare`, `--hidden`, `--no-ignore` and `--binary` work as they do for
[rewrite](../rewriting/#files); a review puts each masking to you before any is written.

```console
$ cat app.log
2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex redact '\{card}:c' --keep c:last4 app.log --dry-run
--- app.log
+++ app.log
@@ -1,2 +1,2 @@
-2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
+2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card ***************1111 declined
 2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::{Keep, Mask, redactions};

let log = b"2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
";
let pat = trex::parse(r"\{card}:c").expect("valid pattern");
let keeps = Keep::parse_list("c:last4", &pat.capture_names()).expect("valid fields");
let mut mask = Mask::parse("*").expect("a mask");
let matches = trex::captures(&pat, log, &trex::scan(&pat, log));
let edits = redactions(log, &matches, &keeps, &mut mask);
assert_eq!(
    trex::files::unified_diff("app.log", log, &edits, 3),
    "--- app.log\n+++ app.log\n@@ -1,2 +1,2 @@\n\
     -2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined\n\
     +2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card ***************1111 declined\n \
     2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms\n"
);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> print(trex.Pattern(r"\{card}:c").redact_diff("app.log", keep="c:last4"), end="")
--- app.log
+++ app.log
@@ -1,2 +1,2 @@
-2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
+2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card ***************1111 declined
 2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```

`redact_file` writes the files back, taking `review=`, `explain=` and `show_skipped=` as
`rewrite_file` does.
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Protect-TrexText '\{card}:c' -Keep c:last4 -Path ./app.log -Diff
--- C:\Temp\demo\app.log
+++ C:\Temp\demo\app.log
@@ -1,2 +1,2 @@
-2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
+2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card ***************1111 declined
 2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
```
{{< /tab >}}
{{< /tabs >}}

## Part of a file

`--head N`, `--tail N` and `--lines A..B` redact one [window](../windows/) of each input; the
window is printed alone, so nothing outside it reaches the output unmasked, and in place only
the window changes. `--follow` redacts what a file gains as it grows.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex redact '\E' app.log --head 1
2026-09-27T09:00:04Z ERROR payment for ********* failed: card 4111 1111 1111 1111 declined
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::records::RecordUnit;
use trex::window::{Select, window_of};
use trex::{Mask, redactions};

let log = b"2026-09-27T09:00:04Z ERROR payment for bob@x.com failed: card 4111 1111 1111 1111 declined
2026-09-27T09:00:05Z INFO GET /v2/users from 10.0.0.5 took 95ms
";
let head = window_of(log, Select::Head(1), &RecordUnit::Line).bytes;
let pat = trex::parse(r"\E").expect("valid pattern");
let mut mask = Mask::parse("*").expect("a mask");
let matches = trex::captures(&pat, &head, &trex::scan(&pat, &head));
let mut out = Vec::new();
let mut at = 0;
for edit in redactions(&head, &matches, &[], &mut mask) {
    out.extend_from_slice(&head[at..edit.start]);
    out.extend_from_slice(&edit.replacement);
    at = edit.end;
}
out.extend_from_slice(&head[at..]);
assert_eq!(out, b"2026-09-27T09:00:04Z ERROR payment for ********* failed: card 4111 1111 1111 1111 declined\n");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\E").redact(open("app.log").read(), head=1)
'2026-09-27T09:00:04Z ERROR payment for ********* failed: card 4111 1111 1111 1111 declined\n'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Protect-TrexText '\E' -Path ./app.log -Head 1
2026-09-27T09:00:04Z ERROR payment for ********* failed: card 4111 1111 1111 1111 declined
```
{{< /tab >}}
{{< /tabs >}}
