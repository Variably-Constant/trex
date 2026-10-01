---
title: Index a tree for faster scans
linkTitle: Index a tree
weight: 10
---

# Index a tree for faster scans

Write an index at a tree's root so later scans skip the files a pattern cannot match without
reading them. An index changes how long a scan takes and never what it reports; the measured
gains are on [indexes](../../reference/tools/#indexes).

```console
$ cat notes/c.txt
mail bob@x.com about the rollout
$ cat notes/d.txt
rollout notes for the second wave
check the queue before starting
$ cat notes/plan.md
the rollout starts monday
```

## Write the index

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex index notes/
notes/: 3 files indexed
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let dir = std::env::temp_dir().join(format!("trex-doc-{}", std::process::id()));
std::fs::create_dir_all(dir.join("notes")).expect("a directory");
std::fs::write(dir.join("notes/c.txt"), "mail bob@x.com about the rollout\n").expect("written");
std::fs::write(dir.join("notes/d.txt"), "rollout notes for the second wave\ncheck the queue before starting\n").expect("written");
std::fs::write(dir.join("notes/plan.md"), "the rollout starts monday\n").expect("written");
let root = dir.join("notes");

let paths: Vec<std::path::PathBuf> = std::fs::read_dir(&root).expect("a directory").map(|e| e.expect("an entry").path()).collect();
let index = trex::index::Index::build(&root, &paths);
assert_eq!(index.len(), 3);
index.save(&root).expect("written");

let read = trex::index::Index::load(&root).expect("an index");
let mail = trex::parse(r"\E").expect("valid pattern");
assert!(read.refuses(&mail, &root.join("d.txt")));
assert!(!read.refuses(&mail, &root.join("c.txt")));
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (New-TrexIndex ./notes).Files
3
```
{{< /tab >}}
{{< /tabs >}}

## Scan as usual

A scan of an indexed tree reads the index without being asked and opens only the files that can
match. A file changed since the index was written is read again, so the report follows the tree:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\E' notes/
notes/c.txt:1:6: "bob@x.com"
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\E' -Path ./notes -Raw
bob@x.com
```
{{< /tab >}}
{{< /tabs >}}

The Rust tab above asks the index the same question with `refuses`, which is what lets a scan
skip `d.txt`. `--no-index` reads no index; `scan --index`, and `-Index` in PowerShell, write one
while the scan reads the files, so a tree is indexed by a scan you were running anyway.
