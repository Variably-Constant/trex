---
title: Segment text with no delimiters
linkTitle: Segment without delimiters
weight: 40
---

# Segment text with no delimiters

Cut text that has no separators into units, where the text before a point stops predicting the
text after it ([seam](../../reference/axes/seam/)).

## English with the spaces gone

`--english` scores each cut against letter counts from a paragraph of English, so even a short
string divides:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex seam --english --text 'thebirdsflyoverthehouses'
trex seam: 24 bytes, order 3, 6 segments (bidirectional branching entropy)
  segments:
    [     0..3     ] "the"
    [     3..8     ] "birds"
    [     8..11    ] "fly"
    [    11..15    ] "over"
    [    15..18    ] "the"
    [    18..24    ] "houses"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::seam::{SeamConfig, analyze_with_model, english_model};

let text = "thebirdsflyoverthehouses";
let field = analyze_with_model(text.as_bytes(), &english_model(), &SeamConfig::default());
let units: Vec<&str> = field.segments().iter().map(|&(s, e)| &text[s..e]).collect();
assert_eq!(units, ["the", "birds", "fly", "over", "the", "houses"]);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexSeam 'thebirdsflyoverthehouses' -English).Segments.Text -join ' '
the birds fly over the houses
```
{{< /tab >}}
{{< /tabs >}}

A word whose letter clusters the paragraph does not hold can be cut inside. To split against a
list of words you supply, [parse with a dictionary](../split-run-together-words/).

## Any other stream

Without `--english` the reader takes its counts from the input itself, so it needs enough text
for its units to repeat. Score it against known boundaries with `--recover`, which removes a
file's spaces, segments what is left and compares the cuts with where the spaces were:

```console
$ trex seam --recover --compare-bpe --max-bytes 16384 moby.txt
trex seam --recover: 16383 bytes, 3169 true boundaries, order 3, passes 1
  seam (no dictionary):   P=0.543 R=0.755 F1=0.632  (4405 cuts, 2393 hit)
  count-BPE (200 merges): P=0.365 R=0.939 F1=0.525  (8165 piece boundaries)
  -> seam recovers word boundaries +0.107 F1 over count-BPE: the bidirectional
     predictive signal (past<->future branching entropy) BPE has no access to.
```

On the first 131,072 bytes of the same text seam reaches F1 0.723. `--order K` sets the longest
context it reads; the default is 3.

## Cut records at the seams

`--record seam` makes each seam segment a record, so a query or a count reads the text in the
units the reader found ([record units](../../reference/records/#record-units)).
