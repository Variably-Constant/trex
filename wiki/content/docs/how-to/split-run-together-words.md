---
title: Split run-together words with a dictionary
linkTitle: Split run-together words
weight: 20
---

# Split run-together words with a dictionary

Read a string with no spaces over the words of a dictionary, every way it tiles into them, and let
a grammar choose which tilings to keep ([segmentation](../../reference/tools/#segmentation)).
Without a dictionary, [segment by the text's own statistics](../segment-without-delimiters/).

```console
$ cat words.txt
the
cat
sat
thecat
on
mat
```

## Count the readings

`thecatsat` tiles two ways, `the cat sat` and `thecat sat`; a grammar asking for three words
keeps one:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex grammar --grammar-text 's := <s> ident | ident' --segment thecatsat --dict words.txt --count
2

$ trex grammar --grammar-text 's := ident ident ident' --segment thecatsat --dict words.txt --count
1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let words: Vec<String> = ["the", "cat", "sat", "thecat", "on", "mat"].map(String::from).to_vec();
let any = trex::Grammar::parse("s := <s> ident | ident").expect("a grammar");
assert_eq!(any.count_segmentations("thecatsat", &words), 2);
let three = trex::Grammar::parse("s := ident ident ident").expect("a grammar");
assert_eq!(three.count_segmentations("thecatsat", &words), 1);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Invoke-TrexGrammar 's := <s> ident | ident' -Segment 'thecatsat' -Dictionary (Get-Content ./words.txt) -Count
2

PS> Invoke-TrexGrammar 's := ident ident ident' -Segment 'thecatsat' -Dictionary (Get-Content ./words.txt) -Count
1
```
{{< /tab >}}
{{< /tabs >}}

With `@p` weights on the rules, `--best` gives the probability of the most probable tiling.
