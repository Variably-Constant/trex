---
title: Split run-together words with a dictionary
linkTitle: Split run-together words
weight: 20
---

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
{{< tab name="Python" >}}
```python
>>> import trex
>>> words = trex.read("words.txt").split()
>>> trex.Grammar("s := <s> ident | ident").count_segmentations("thecatsat", words)
2
>>> trex.Grammar("s := ident ident ident").count_segmentations("thecatsat", words)
1
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

## List the readings

With `@p` weights on the rules, each tiling the grammar accepts has a probability, its most
probable parse's, and `--list` lists the tilings most probable first. A grammar that weighs each
word at a half prefers the reading with fewer words:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex grammar --grammar-text 's := <s> ident @0.5 | ident @0.5' --segment thecatsat --dict words.txt --list
thecat sat	0.25	1
the cat sat	0.125	1

$ trex grammar --grammar-text 's := <s> ident @0.5 | ident @0.5' --segment thecatsat --dict words.txt --best
0.25
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let words: Vec<String> = ["the", "cat", "sat", "thecat", "on", "mat"].map(String::from).to_vec();
let halves = trex::Grammar::parse("s := <s> ident @0.5 | ident @0.5").expect("a grammar");
let tilings = halves.segment("thecatsat", &words);
let listed: Vec<String> = tilings.iter().map(|t| t.words.join(" ")).collect();
assert_eq!(listed, ["thecat sat", "the cat sat"]);
assert_eq!(tilings[0].probability, halves.best_segmentation_prob("thecatsat", &words));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> halves = trex.Grammar("s := <s> ident @0.5 | ident @0.5")
>>> [(" ".join(t.words), t.probability, t.parses) for t in halves.segment("thecatsat", words)]
[('thecat sat', 0.25, 1), ('the cat sat', 0.125, 1)]
>>> halves.best_segmentation_probability("thecatsat", words)
0.25
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Invoke-TrexGrammar 's := <s> ident @0.5 | ident @0.5' -Segment 'thecatsat' -Dictionary (Get-Content ./words.txt)

Words           Probability Parses
-----           ----------- ------
{thecat, sat}   0.25        1
{the, cat, sat} 0.125       1

PS> Invoke-TrexGrammar 's := <s> ident @0.5 | ident @0.5' -Segment 'thecatsat' -Dictionary (Get-Content ./words.txt) -Best
0.25
```
{{< /tab >}}
{{< /tabs >}}

`--best` gives the first tiling's probability alone. Each line of `--list` is a tiling's words,
its probability and its parse count, separated by tabs.
