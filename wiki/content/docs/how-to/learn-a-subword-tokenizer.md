---
title: Learn a subword tokenizer
linkTitle: Learn a subword tokenizer
weight: 21
---

# Learn a subword tokenizer

Learn a byte-pair encoder from a corpus, save it, and split new words into its subwords
([byte-pair encoders](../../reference/tools/#byte-pair-encoders)).

```console
$ cat corpus.txt
low low low low lower lower newest newest newest widest
```

## Learn the merges

Each merge joins the most frequent adjacent pair; the merges print in the order learned:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex bpe train corpus.txt --merges 3
trex bpe: learned 3 merges from 56 bytes in 0.0s
l	o
lo	w
e	s
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let bpe = trex::bpe::Bpe::train(b"low low low low lower lower newest newest newest widest", 3);
assert!(bpe.to_lines().starts_with("l\to\nlo\tw\ne\ts"));
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $three = Get-Content ./corpus.txt | New-TrexBpe -Merges 3
PS> $three.Encode('lowest') -join ' '
low es t </w>
```
{{< /tab >}}
{{< /tabs >}}

## Split words with it

{{< tabs >}}
{{< tab name="Rust" >}}
```rust
let bpe = trex::bpe::Bpe::train(b"low low low low lower lower newest newest newest widest", 10);
assert_eq!(bpe.encode(b"lower").join(" "), "low er </w>");
let again = trex::bpe::Bpe::parse(&bpe.to_lines()).expect("a model");
assert_eq!(again.encode(b"lower"), bpe.encode(b"lower"));
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $bpe = Get-Content ./corpus.txt | New-TrexBpe -Merges 10
PS> $bpe.Encode('lower') -join ' '
low er </w>

PS> Export-TrexBpe ./words.bpe -Bpe $bpe
PS> (Import-TrexBpe ./words.bpe).Encode('lower') -join ' '
low er </w>
```
{{< /tab >}}
{{< /tabs >}}

`</w>` marks a word's end. The saved model is the merge list `train` prints, one pair a line,
and `trex bpe encode --model FILE` splits text with it.
