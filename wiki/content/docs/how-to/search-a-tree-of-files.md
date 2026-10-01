---
title: Search a tree of files
linkTitle: Search a tree of files
weight: 9
---

# Search a tree of files

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
let walk = trex::files::WalkOptions { globs: vec!["*.txt".to_string()], ..Default::default() };
for (opts, files) in [(trex::files::WalkOptions::default(), 3), (walk, 2)] {
    let (sources, errors) = trex::files::collect(&["notes/".to_string()], &opts);
    assert!(errors.is_empty());
    let matching = sources
        .iter()
        .filter(|s| !trex::scan(&pat, &trex::files::read_source(s).expect("readable")).is_empty())
        .count();
    assert_eq!(matching, files);
}
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
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '"queue" | "rollout"' -Path ./notes -Include '*.txt' -FilesWithMatches | Split-Path -Leaf
c.txt
d.txt
```
{{< /tab >}}
{{< /tabs >}}

The Rust tab above walks with `WalkOptions { globs, .. }` the same way. `--hidden`,
`--no-ignore` and `--binary` widen the walk, and a file named outright is read whatever the
rules say.
