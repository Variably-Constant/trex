---
title: Rename or reshape matches
linkTitle: Rename matches
weight: 20
---

Replace each match with a template: `${name}` renders a register, `${0}` the whole match, and
`${name:acc}` a transform or a typed slice of one, chained with `|`. The
[rewrite accessors](../../reference/pattern-syntax/#rewrite-accessors) lists them.

## Keep part of each match

Bind the part to keep and render it alone. The closing tag must equal the opening one, `=t`,
so only well-formed pairs are touched:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex rewrite '<\W:t>.*</=t>' '[${t}]' --text '<b>hi</b> <i>yo</i>'
[b] [i]
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"<\W:t>.*</=t>").expect("valid pattern");
let tmpl = trex::Template::parse("[${t}]", &pat.capture_names()).expect("valid template");
assert_eq!(trex::rewrite(&pat, &tmpl, b"<b>hi</b> <i>yo</i>"), b"[b] [i]");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> trex.Pattern(r"<\W:t>.*</=t>").rewrite("[${t}]", "<b>hi</b> <i>yo</i>")
'[b] [i]'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> '<b>hi</b> <i>yo</i>' | Edit-TrexText '<\W:t>.*</=t>' '[${t}]'
[b] [i]
```
{{< /tab >}}
{{< /tabs >}}

## Reshape a typed value

A typed register slices by its structure: an IPv4 address by `octetN-M`, a URL by `host`, an
email by `domain`, a version by `major`, a timestamp by `year`.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex rewrite '\I:ip' '${ip:octet1-2}.0.0/16' --text 'conn from 192.168.5.9'
conn from 192.168.0.0/16
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\I:ip").expect("valid pattern");
let tmpl = trex::Template::parse("${ip:octet1-2}.0.0/16", &pat.capture_names()).expect("valid template");
assert_eq!(trex::rewrite(&pat, &tmpl, b"conn from 192.168.5.9"), b"conn from 192.168.0.0/16");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\I:ip").rewrite("${ip:octet1-2}.0.0/16", "conn from 192.168.5.9")
'conn from 192.168.0.0/16'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'conn from 192.168.5.9' | Edit-TrexText '\I:ip' '${ip:octet1-2}.0.0/16'
conn from 192.168.0.0/16
```
{{< /tab >}}
{{< /tabs >}}

## Compute the replacement

Where a template cannot say it, hand each match to a function. Here durations are written in
milliseconds whatever unit they were written in:

{{< tabs >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\R:t").expect("valid pattern");
let out = trex::rewrite_with(&pat, b"took 1500ms, then 2s", |m| match m.value("t") {
    Some(trex::typed::TypedValue::Num(ns)) => format!("{}ms", ns.as_f64().expect("a number") / 1e6),
    _ => m.text().to_string(),
});
assert_eq!(out, b"took 1500ms, then 2000ms");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\R:t").rewrite(lambda m: f'{m.value("t", unit="ms")}ms', "took 1500ms, then 2s")
'took 1500ms, then 2000ms'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'took 1500ms, then 2s' | Edit-TrexText '\R:t' -ScriptBlock { '{0}ms' -f $_.Groups[0].Value.TotalMilliseconds }
took 1500ms, then 2000ms
```
{{< /tab >}}
{{< /tabs >}}

## Rename across files

Look at the diff first, then write it:

```console
$ cat src/notes.md
The answer is 42, and legacy_call() is the old name.
See legacy_call in the guide.
$ cat src/main.rs
fn main() {
    legacy_call(42);
}
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex rewrite '"legacy_call"' 'current_call' src/ --dry-run
--- src/main.rs
+++ src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    legacy_call(42);
+    current_call(42);
 }
--- src/notes.md
+++ src/notes.md
@@ -1,2 +1,2 @@
-The answer is 42, and legacy_call() is the old name.
-See legacy_call in the guide.
+The answer is 42, and current_call() is the old name.
+See current_call in the guide.

$ trex rewrite '"legacy_call"' 'current_call' src/ --in-place
src/main.rs: 1 replacement
src/notes.md: 2 replacements
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
std::fs::create_dir_all(dir.join("src")).expect("a directory");
std::fs::write(dir.join("src/notes.md"), "The answer is 42, and legacy_call() is the old name.\nSee legacy_call in the guide.\n").expect("written");
std::fs::write(dir.join("src/main.rs"), "fn main() {\n    legacy_call(42);\n}\n").expect("written");
std::env::set_current_dir(&dir).expect("the temporary directory");

let pat = trex::parse(r#""legacy_call""#).expect("valid pattern");
let tmpl = trex::Template::parse("current_call", &pat.capture_names()).expect("valid template");
let (sources, errors) = trex::files::collect(&["src/".to_string()], &trex::files::WalkOptions::default());
assert!(errors.is_empty());
for source in &sources {
    let bytes = trex::files::read_source(source).expect("a readable file");
    let out = trex::rewrite(&pat, &tmpl, &bytes);
    if out != bytes {
        std::fs::write(source.name(), out).expect("written back");
    }
}
assert_eq!(
    std::fs::read_to_string("src/notes.md").expect("readable"),
    "The answer is 42, and current_call() is the old name.\nSee current_call in the guide.\n"
);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import pathlib
>>> rename = trex.Pattern('"legacy_call"')
>>> for path in sorted(pathlib.Path("src").iterdir()):
...     text = path.read_text()
...     if rename.is_match(text):
...         _ = path.write_text(rename.rewrite("current_call", text))
>>> print(pathlib.Path("src/notes.md").read_text(), end="")
The answer is 42, and current_call() is the old name.
See current_call in the guide.
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Edit-TrexText '"legacy_call"' 'current_call' -Path ./src -InPlace
PS> Get-Content -Path ./src/notes.md
The answer is 42, and current_call() is the old name.
See current_call in the guide.
```
{{< /tab >}}
{{< /tabs >}}

`--interactive`, and `-Interactive` in PowerShell, put each change to you instead, with one
answer covering every change of the same [template](../../reference/rewriting/#review).
