---
title: Scan input that arrives in pieces
linkTitle: Scan input in pieces
weight: 18
---

Scan a stream chunk by chunk and take each match as soon as no later chunk can change it, ending
with the matches one scan over the whole stream would give. A chunk that ends inside a token holds
that match until the token ends ([streams](../../reference/tools/#streams)).

## Feed the chunks

The address split across the first two chunks, `ann@y.` and `org`, is reported once the second
chunk completes it:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ printf 'mail bob@x.com\nand ann@y.org now\nthen ted@z.net\n' | trex scan '\E'
[5..14] "bob@x.com"
[19..28] "ann@y.org"
[38..47] "ted@z.net"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut scan = trex::StreamScanner::new(trex::parse(r"\E").expect("valid pattern"));
scan.push(b"mail bob@x.com\nand ann@y.");
let first = scan.drain_committed();
scan.push(b"org now\nthen ted@z.net");
let second = scan.drain_committed();
let rest = scan.finish();
let at = |s: &[trex::Span]| s.iter().map(|s| s.start()).collect::<Vec<_>>();
assert_eq!((at(&first), at(&second), at(&rest)), (vec![5], vec![19], vec![38]));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> st = trex.StreamScanner(trex.Pattern(r"\E"))
>>> st.push(b"mail bob@x.com\nand ann@y."), st.push(b"org now\nthen ted@z.net"), st.finish()
([(5, 14)], [(19, 28)], [(38, 47)])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $scan = New-TrexStreamScanner '\E'
PS> ($scan.Push("mail bob@x.com`nand ann@y.")).Text
bob@x.com
PS> ($scan.Push("org now`nthen ted@z.net")).Text
ann@y.org
PS> ($scan.Finish()).Text
ted@z.net
```
{{< /tab >}}
{{< /tabs >}}

The command line reads its standard input this way, printing each match the moment it commits,
so `tail -f app.log | trex scan '\E'` reports an address as soon as its line arrives.

## Check against one scan of the whole

The matches over the stream are those of one scan over all of it, whatever the chunk size:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\E' --text $'mail bob@x.com\nand ann@y.org now\nthen ted@z.net\n' --chunk-size 3
[5..14] "bob@x.com"
[19..28] "ann@y.org"
[38..47] "ted@z.net"

$ trex scan '\E' --text $'mail bob@x.com\nand ann@y.org now\nthen ted@z.net\n' --chunk-size 1
[5..14] "bob@x.com"
[19..28] "ann@y.org"
[38..47] "ted@z.net"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"mail bob@x.com\nand ann@y.org now\nthen ted@z.net\n";
let pat = trex::parse(r"\E").expect("valid pattern");
for size in [1, 3, 7, 64] {
    assert_eq!(trex::scan_chunked(&pat, text.chunks(size)), trex::scan(&pat, text));
}
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> text = b"mail bob@x.com\nand ann@y.org now\nthen ted@z.net\n"
>>> whole = [(m.start, m.end) for m in trex.Pattern(r"\E").scan(text)]
>>> st = trex.StreamScanner(trex.Pattern(r"\E"))
>>> sum((st.push(text[i:i + 3]) for i in range(0, len(text), 3)), []) + st.finish() == whole
True
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $text = "mail bob@x.com`nand ann@y.org now`nthen ted@z.net`n"
PS> foreach ($size in 1, 3, 7, 64) { (Select-TrexMatch '\E' -InputObject $text -ChunkSize $size).Start -join ',' }
5,19,38
5,19,38
5,19,38
5,19,38
PS> (Select-TrexMatch '\E' -InputObject $text).Start -join ','
5,19,38
```
{{< /tab >}}
{{< /tabs >}}

A pattern that cannot settle before the stream ends, such as one with a lookahead or a
whole-input axis, is scanned when the stream ends. A pattern set streams through one scanner:
`--patterns FILE` on the command line, `over_set` in Rust, `StreamScanner(set)` in Python and
`New-TrexStreamScanner` given several patterns in PowerShell.
