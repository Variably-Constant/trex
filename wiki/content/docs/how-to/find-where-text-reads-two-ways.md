---
title: Find where text reads two ways
linkTitle: Find where text reads two ways
weight: 47
---

Find the points where the text behind and the text ahead read differently: where one kind of
text gives way to another, placed at the change rather than a window after it
([observation](../../reference/axes/observation/)).

```console
$ cat export.txt
id,host,bytes
1,alpha,1024
2,beta,2048
3,gamma,4096
The export finished without errors and wrote three rows.
```

## Where the two sides differ most

Each byte has three readings of the byte classes around it: from behind, from ahead and from both
sides. A contested point is where behind and ahead differ most, and the peak is the contested
point where they differ more than anywhere else:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex observe export.txt --contested
trex observe: 109 bytes, 6 contested point(s)
  peak observer-dependence: 0.54 at byte 55
  contested points (6):
    @    20  ...tes 1,alpha,1024 2,beta,...
    @    40  ...eta,2048 3,gamma,4096 Th...
    @    55  ...a,4096 The export finish...
    @    63  ...he export finished witho...
    @    71  ...t finished without error...
    @    89  ... errors and wrote three ...
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"id,host,bytes\n1,alpha,1024\n2,beta,2048\n3,gamma,4096\nThe export finished without errors and wrote three rows.\n";
let field = trex::observation::analyze(text);
assert_eq!(field.contested, [20, 40, 55, 63, 71, 89]);
let peak = field.contested.iter().copied().max_by(|&a, &b| field.frames[a].disagreement.total_cmp(&field.frames[b].disagreement));
assert_eq!(peak, Some(55));
assert!(trex::spectral::analyze(text).boundaries.is_empty());
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> import trex.axes
>>> r = trex.axes.observation(path="export.txt")
>>> r.peak.offset, round(r.peak.disagreement, 2)
(55, 0.54)
>>> [c.offset for c in r.contested]
[20, 40, 55, 63, 71, 89]
>>> trex.axes.spectral(path="export.txt").change_points
[]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $observed = Measure-TrexObservation -Path ./export.txt
PS> $observed.Peak

Offset       : 55
Text         :
Causal       : 0.7744313
Anticausal   : 0.2341005
Centered     : 0.6100973
Disagreement : 0.5403309

PS> $observed.Contested | Select-Object Offset, Disagreement | Format-Table

Offset Disagreement
------ ------------
    20         0.29
    40         0.23
    55         0.54
    63         0.49
    71         0.34
    89         0.21
```
{{< /tab >}}
{{< /tabs >}}

The disagreement rises at byte 51, the newline that ends the last row, and holds near its height
across the first word of the prose, peaking at byte 55, the space after `The`: the reading from
behind still sees a table there, and the reading from ahead already sees words. Of two contested
points closer together than eight bytes, the stronger is the one kept, so the point reported in
that stretch is its peak. A reading of the past alone places a change a window late, and over an
input this short [spectral](../../reference/axes/spectral/), which reads that way, finds no
change point at all. The smaller contested points are local disagreements inside each kind of
text.

## Match at the point

`@ambiguous` anchors a pattern at a contested point. At the token grain it reads the sequence of
token kinds, so it holds where the kinds behind and ahead differ most, on the table's last token:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@ambiguous:token .' export.txt
[47..51] "4096"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"@ambiguous:token .").expect("valid pattern");
let text = "id,host,bytes\n1,alpha,1024\n2,beta,2048\n3,gamma,4096\nThe export finished without errors and wrote three rows.\n";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["4096"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"@ambiguous:token .").scan(trex.read("export.txt"))]
['4096']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@ambiguous:token .' -Path ./export.txt -Raw
4096
```
{{< /tab >}}
{{< /tabs >}}

`@ambiguous:byte` reads the bytes and `@ambiguous:super` the sequence of [supertoken](../../explanation/architecture/#supertokens) roles; a bare
`@ambiguous` reads the bytes.
