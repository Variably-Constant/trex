---
title: Search a tree of files
linkTitle: Search a tree of files
weight: 9
---

Search every file under a directory, keep or drop files by name, and list the files that match.
The walk applies `.gitignore` and `.ignore` rules and skips hidden and binary files, as
[files and trees](../../reference/matching/#files-and-directory-trees) describes.

```console
$ cat notes/c.txt
mail bob@x.com about the rollout
$ cat notes/d.txt
rollout notes for the second wave
check the queue before starting
$ cat notes/plan.md
the rollout starts monday
```

## Search a directory

Each match is prefixed with its file, line and column:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '"queue" | "rollout"' notes/
notes/c.txt:1:26: "rollout"
notes/d.txt:1:1: "rollout"
notes/d.txt:2:11: "queue"
notes/plan.md:1:5: "rollout"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
std::fs::create_dir_all(dir.join("notes")).expect("a directory");
std::fs::write(dir.join("notes/c.txt"), "mail bob@x.com about the rollout\n").expect("written");
std::fs::write(dir.join("notes/d.txt"), "rollout notes for the second wave\ncheck the queue before starting\n").expect("written");
std::fs::write(dir.join("notes/plan.md"), "the rollout starts monday\n").expect("written");
std::env::set_current_dir(&dir).expect("the temporary directory");

let pat = trex::parse(r#""queue" | "rollout""#).expect("valid pattern");
let (sources, errors) = trex::files::collect(&["notes/".to_string()], &trex::files::WalkOptions::default());
assert!(errors.is_empty());
let mut found = Vec::new();
for source in &sources {
    let bytes = trex::files::read_source(source).expect("readable");
    let index = trex::files::LineIndex::new(&bytes);
    for span in trex::scan(&pat, &bytes) {
        let line = index.line_of(span.start());
        let column = span.start() - index.line_span(line).0 + 1;
        found.push((source.name(), line + 1, column, String::from_utf8_lossy(&bytes[span.range()]).into_owned()));
    }
}
assert_eq!(found, [
    ("notes/c.txt".to_string(), 1, 26, "rollout".to_string()),
    ("notes/d.txt".to_string(), 1, 1, "rollout".to_string()),
    ("notes/d.txt".to_string(), 2, 11, "queue".to_string()),
    ("notes/plan.md".to_string(), 1, 5, "rollout".to_string()),
]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> found = trex.Pattern('"queue" | "rollout"').grep(path="notes/")
>>> [(l.path, l.number, m.column, m.text) for l in found for m in l.matches]
[('notes/c.txt', 1, 26, 'rollout'), ('notes/d.txt', 1, 1, 'rollout'), ('notes/d.txt', 2, 11, 'queue'), ('notes/plan.md', 1, 5, 'rollout')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '"queue" | "rollout"' -Path ./notes | Select-Object @{ n = 'File'; e = { Split-Path -Leaf $_.Path } }, LineNumber, Column, Text

File    LineNumber Column Text
----    ---------- ------ ----
c.txt            1     26 rollout
d.txt            1      1 rollout
d.txt            2     11 queue
plan.md          1      5 rollout
```
{{< /tab >}}
{{< /tabs >}}

## Keep some files and list the matches

`-g GLOB` keeps the files a glob names, `!GLOB` drops them, and `-t TYPE` keeps a file type
under ripgrep's names; `-l` lists each file with a match once:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '"queue" | "rollout"' notes/ -g '*.txt' -l
notes/c.txt
notes/d.txt
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
std::fs::create_dir_all(dir.join("notes")).expect("a directory");
std::fs::write(dir.join("notes/c.txt"), "mail bob@x.com about the rollout\n").expect("written");
std::fs::write(dir.join("notes/d.txt"), "rollout notes for the second wave\ncheck the queue before starting\n").expect("written");
std::fs::write(dir.join("notes/plan.md"), "the rollout starts monday\n").expect("written");
std::env::set_current_dir(&dir).expect("the temporary directory");

let pat = trex::parse(r#""queue" | "rollout""#).expect("valid pattern");
let walk = trex::files::WalkOptions { globs: vec!["*.txt".to_string()], ..Default::default() };
let (sources, errors) = trex::files::collect(&["notes/".to_string()], &walk);
assert!(errors.is_empty());
let listed: Vec<String> = sources
    .iter()
    .filter(|s| !trex::scan(&pat, &trex::files::read_source(s).expect("readable")).is_empty())
    .map(trex::files::Source::name)
    .collect();
assert_eq!(listed, ["notes/c.txt", "notes/d.txt"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> list(dict.fromkeys(l.path for l in trex.Pattern('"queue" | "rollout"').grep(path="notes/", globs=["*.txt"])))
['notes/c.txt', 'notes/d.txt']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '"queue" | "rollout"' -Path ./notes -Include '*.txt' -FilesWithMatches | Split-Path -Leaf
c.txt
d.txt
```
{{< /tab >}}
{{< /tabs >}}

In Rust the walk takes globs as `WalkOptions { globs, .. }`, and Python's `grep` takes
`globs=`. `--hidden`, `--no-ignore` and `--binary` widen the walk, as `hidden=`, `no_ignore=` and
`binary=` do, and a file named outright is read whatever the rules say.
