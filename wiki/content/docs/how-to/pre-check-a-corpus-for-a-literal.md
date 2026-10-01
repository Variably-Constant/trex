---
title: Pre-check a corpus for a literal
linkTitle: Pre-check a corpus for a literal
weight: 26
---

# Pre-check a corpus for a literal

Build a small presence filter over a corpus once, then answer whether a literal might occur in it
without scanning the corpus again. A filter never says a present literal is absent; a "might
occur" is confirmed by an exact search ([prefilters](../../reference/tools/#prefilters)).

```console
$ cat corpus.txt
the quick brown fox ERROR here
warn: disk at 91% full
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex prefilter corpus.txt --literal ERROR --literal MISSING
bloom: "ERROR" -> might occur; present (confirmed)
bloom: "MISSING" -> ABSENT (rejected with no corpus scan)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::prefilter::{BloomFilter, Membership};

let filter = BloomFilter::build(b"the quick brown fox ERROR here\nwarn: disk at 91% full\n");
assert!(filter.might_contain(b"ERROR"));
assert!(!filter.might_contain(b"MISSING"));
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $filter = Get-Content ./corpus.txt | New-TrexPrefilter
PS> $filter.MightContain('ERROR'), $filter.MightContain('MISSING')
True
False
```
{{< /tab >}}
{{< /tabs >}}

`--filter cuckoo` and `--filter xor` build the other two filters, and `--verify` checks every
filter over the corpus: no literal present is ever reported absent.
