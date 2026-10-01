---
title: Rewriting
linkTitle: Rewriting
weight: 40
---

# Rewriting

A rewrite replaces each match with a template rendered at it, or with what a function returns
for it, and splices the result into the input. The template language is the
[rewrite accessors](../pattern-syntax/#rewrite-accessors) section's; masking is on the
[redaction](../redaction/) page. The flags and parameters are on the
[CLI](../cli/#rewrite) and [PowerShell](../powershell/rewriting/) pages.

## Templates

`${name}` renders a register, `${0}` the whole match, `${1}`, `${2}` a register by position,
and `${name:accessor}` transforms or slices one: `upper`, `lower` and `trim` on any register,
the typed slices on a captured atom (`${ip:octet1-2}`, `${url:host}`, `${e:domain}`,
`${ts:year}`), chained with `|`. `$$` is a dollar sign, and `\n`, `\t` and `\\` are a newline,
a tab and a backslash.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex rewrite '\E:e' '[${e:domain}]' --text 'mail bob@x.com and amy@y.org now'
mail [x.com] and [y.org] now

$ trex rewrite '\I:ip' '${ip:octet1-2}.0.0/16' --text 'conn from 192.168.5.9'
conn from 192.168.0.0/16

$ trex rewrite '\W:a "=" \N:b' '${2} := ${1}' --text 'x = 1'
1 := x
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::{Template, parse, rewrite};

let pat = parse(r"\E:e").expect("valid pattern");
let tmpl = Template::parse("[${e:domain}]", &pat.capture_names()).expect("valid template");
assert_eq!(rewrite(&pat, &tmpl, b"mail bob@x.com and amy@y.org now"), b"mail [x.com] and [y.org] now");

let pat = parse(r#"\W:a "=" \N:b"#).expect("valid pattern");
let tmpl = Template::parse("${2} := ${1}", &pat.capture_names()).expect("valid template");
assert_eq!(rewrite(&pat, &tmpl, b"x = 1"), b"1 := x");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> trex.Pattern(r"\E:e").rewrite("[${e:domain}]", "mail bob@x.com and amy@y.org now")
'mail [x.com] and [y.org] now'
>>> trex.Pattern(r"\I:ip").rewrite("${ip:octet1-2}.0.0/16", "conn from 192.168.5.9")
'conn from 192.168.0.0/16'
>>> trex.Pattern(r'\W:a "=" \N:b').rewrite("${2} := ${1}", "x = 1")
'1 := x'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'mail bob@x.com and amy@y.org now' | Edit-TrexText '\E:e' '[${e:domain}]'
mail [x.com] and [y.org] now

PS> 'conn from 192.168.5.9' | Edit-TrexText '\I:ip' '${ip:octet1-2}.0.0/16'
conn from 192.168.0.0/16

PS> 'x = 1' | Edit-TrexText '\W:a "=" \N:b' '${2} := ${1}'
1 := x
```
{{< /tab >}}
{{< /tabs >}}

A template naming a register the pattern does not bind, or an accessor the register's kind
does not have, is refused before anything is rewritten.

## The first matches

{{< tabs >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\N").expect("valid pattern");
let out = trex::rewrite_n_with(&pat, b"1 2 3 4", 2, |_| "n");
assert_eq!(out, b"n n 3 4");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\N").rewrite_n("n", "1 2 3 4", 2)
'n n 3 4'
>>> trex.Pattern(r"\N").rewrite_first("n", "1 2 3 4")
'n 2 3 4'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> '1 2 3 4' | Edit-TrexText '\N' 'n' -MaxCount 2
n n 3 4
```
{{< /tab >}}
{{< /tabs >}}

## Computed replacements

A function in place of the template is handed each match and returns its replacement, so a
replacement can be computed from a register's typed value. It reads a register through the
same references a template writes.

{{< tabs >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\E:e").expect("valid pattern");
let mut n = 0;
let out = trex::rewrite_with(&pat, b"bob@x.com, amy@y.org", |m| {
    n += 1;
    format!("{n}:{}", m.get("e:domain").expect("a slice a template could render"))
});
assert_eq!(out, b"1:x.com, 2:y.org");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Pattern(r"\E:e").rewrite(lambda m: m["e:user"].upper(), "mail bob@x.com and amy@y.org now")
'mail BOB and AMY now'
>>> trex.Pattern(r"\R:t").rewrite(lambda m: f'{m.value("t", unit="ms")}ms', "took 1500ms, then 2s")
'took 1500ms, then 2000ms'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> 'mail bob@x.com and amy@y.org now' | Edit-TrexText '\E:e' -ScriptBlock { $_.Captures.e.Split('@')[0].ToUpper() }
mail BOB and AMY now

PS> 'took 1500ms, then 2s' | Edit-TrexText '\R:t' -ScriptBlock { '{0}ms' -f $_.Groups[0].Value.TotalMilliseconds }
took 1500ms, then 2000ms
```
{{< /tab >}}
{{< /tabs >}}

In Rust the closure receives a `Matched`, whose `get` reads a reference such as `e:domain`,
`0:last4` or `1`, and whose `value` reads a typed register where it was built with the
pattern's `capture_kinds`. In Python the callable receives the `Match` and returns `str` for a
`str` input and `bytes` for `bytes`. In PowerShell the script block runs with the
`Trex.Match` as `$_`, and its last output replaces the match.

## Files

One input is rewritten to the standard output. A directory or several inputs take
`--in-place`, which writes each file holding a match back in the encoding it was read in,
behind its own byte order mark, every byte outside a match as it was, or `--dry-run`, which
prints the unified diff each file would take, with three lines of context or `-C N`. A
directory is walked as [scan](../matching/#files-and-directory-trees) walks one, and a binary
file named alone is refused, by name, unless `--binary` asks for it.

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
for (path, text) in [
    ("src/notes.md", "The answer is 42, and legacy_call() is the old name.\nSee legacy_call in the guide.\n"),
    ("src/main.rs", "fn main() {\n    legacy_call(42);\n}\n"),
] {
    std::fs::create_dir_all(dir.join(path).parent().expect("a parent")).expect("a directory");
    std::fs::write(dir.join(path), text).expect("a file written");
}
std::env::set_current_dir(&dir).expect("the temporary directory");

let pat = trex::parse(r#""legacy_call""#).expect("valid pattern");
let tmpl = trex::Template::parse("current_call", &pat.capture_names()).expect("valid template");
let (sources, _) = trex::files::collect(&["src/".to_string()], &trex::files::WalkOptions::default());
let mut made = Vec::new();
for source in &sources {
    let raw = trex::files::read_source(source).expect("a readable file");
    let text = trex::encoding::text_of(&raw).into_owned();
    let edits = trex::rewrite::edits(&pat, &tmpl, &text);
    if source.name() == "src/main.rs" {
        let diff = trex::files::unified_diff(&source.name(), &text, &edits, 3);
        assert_eq!(diff, "--- src/main.rs\n+++ src/main.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    legacy_call(42);\n+    current_call(42);\n }\n");
    }
    let written = trex::files::written(&raw, &edits, trex::files::ReadAs::Decoded);
    std::fs::write(source.name(), written).expect("the file written");
    made.push((source.name(), edits.len()));
}
assert_eq!(made, [("src/main.rs".to_string(), 1), ("src/notes.md".to_string(), 2)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> pat = trex.Pattern('"legacy_call"')
>>> for path in trex.files("src/"):
...     print(pat.diff("current_call", path), end="")
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
>>> {path: pat.rewrite_file("current_call", path) for path in trex.files("src/")}
{'src/main.rs': 1, 'src/notes.md': 2}
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Edit-TrexText '"legacy_call"' 'current_call' -Path ./src -Diff
--- C:\Temp\demo\src\main.rs
+++ C:\Temp\demo\src\main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    legacy_call(42);
+    current_call(42);
 }

--- C:\Temp\demo\src\notes.md
+++ C:\Temp\demo\src\notes.md
@@ -1,2 +1,2 @@
-The answer is 42, and legacy_call() is the old name.
-See legacy_call in the guide.
+The answer is 42, and current_call() is the old name.
+See current_call in the guide.

PS> Edit-TrexText '"legacy_call"' 'current_call' -Path ./src -InPlace
PS> Get-Content -Path ./src/notes.md
The answer is 42, and current_call() is the old name.
See current_call in the guide.
```
{{< /tab >}}
{{< /tabs >}}

In PowerShell `-InPlace` asks `ShouldProcess` for each file, so `-WhatIf` names the files it
would write and writes none, and `-Confirm` asks for each.

## Review

`--interactive` (`-i`) shows each change as its unified diff, with the template of its match -
the kinds of the tokens the match spans - and how many later changes share that template,
and asks:

| Answer | Does |
|---|---|
| `y` | applies this change |
| `n` | skips it |
| `e` | takes another replacement, in the editor `VISUAL` or `EDITOR` names, or typed at the prompt where neither is set |
| `t` | applies this change and every later one of its template |
| `T` | skips this change and every later one of its template |
| `a` | applies this change and every one after it |
| `q` | stops, leaving every file as it was |

The accepted changes are written once the last one has been answered, so a session that ends
early leaves every file as it was. `-U` (`--update-all`) applies every change without asking.
The answers are read from the standard input, so a review runs from a pipe as well as by hand.
How many changes a `T` passed over is reported; `--show-skipped` names where each stands.
`--explain` puts under each diff what `scan --explain` puts under each match.

```console
$ cat levels.log
alpha 10
beta 20
gamma 300
delta 4000
epsilon 5
$ printf 'n\ny\n' | trex rewrite '\N{>=100}' '[${0}]' levels.log --interactive
--- levels.log
+++ levels.log
@@ -1,5 +1,5 @@
 alpha 10
 beta 20
-gamma 300
+gamma [300]
 delta 4000
 epsilon 5
  template: number (1 later change shares it)
[1/2] accept, skip or edit this change, all of this template, none of it, accept all, or quit [y/n/e/t/T/a/q]? --- levels.log
+++ levels.log
@@ -1,5 +1,5 @@
 alpha 10
 beta 20
 gamma 300
-delta 4000
+delta [4000]
 epsilon 5
  template: number (0 later changes share it)
[2/2] accept, skip or edit this change, all of this template, none of it, accept all, or quit [y/n/e/t/T/a/q]? levels.log: 1 replacement
```

`t` takes the rest of a template with one answer:

```console
$ cat levels.log
alpha 10
beta 20
gamma 300
delta 4000
epsilon 5
$ printf 't\n' | trex rewrite '\N{>=100}' '[${0}]' levels.log --interactive
--- levels.log
+++ levels.log
@@ -1,5 +1,5 @@
 alpha 10
 beta 20
-gamma 300
+gamma [300]
 delta 4000
 epsilon 5
  template: number (1 later change shares it)
[1/2] accept, skip or edit this change, all of this template, none of it, accept all, or quit [y/n/e/t/T/a/q]? levels.log: 2 replacements
```

In PowerShell `-Interactive` puts each change to the host's prompt with the same choices,
`Y`, `N` (the default), `E`, `T`, `S` for skipping a template, `A` and `Q`; a host that cannot
prompt, such as `pwsh -NonInteractive`, stops at the first question and writes nothing.

## Declared atoms

A pattern naming `\{name}` reads its declaration from a pattern file, `--lib FILE` on the
command line, `lib=` in Python, `Import-TrexAtom` or `-Library` in PowerShell and a
`ShapeSet` in Rust; a declared shape decides the token boundaries the rewrite lexes on, so it
replaces what a scan under the same file reports. A rewrite under a declared shape runs on the
CPU engines. The [pattern files](../pattern-files/) page has the declarations.
