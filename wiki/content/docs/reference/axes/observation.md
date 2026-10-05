---
title: The observation axis
linkTitle: Observation
weight: 80
---

The observation axis asks, at every point of the input, whether the text behind it and the text
ahead of it read alike. It reads the entropy of the byte classes in three windows: one behind the
point, one ahead of it, and one around it. Where the windows behind and ahead give different
readings, the point is contested: it is where one kind of text gives way to another, found at
the change itself rather than a window after it.

Source: [`src/observation.rs`](https://github.com/Variably-Constant/trex/blob/main/src/observation.rs).

## What it reads that nothing else does

```text
let x = 5; let y = 6; let z = 7; let w = 8; let v = 9; let u = 1; let t = 2; let s = 3;
U2VuZCB0aGUgcmVwb3J0IHRvIGJvYkB4LmNvbSBieSBGcmlkYXksIGFuZCBjb3B5IHRoZSB0ZWFtIG9uIHRoZSBtYWlsIHRocmVhZC4=
```

Within the code and within the blob the past and the future agree. At the line break they
diverge: the past-only reading still sees code and the future-only reading already sees the
blob, so observation contests byte 87, the line break itself, and peaks at 0.61 eight bytes
later. [Spectral](../spectral/), which reads only the bytes already behind each point, cuts its texture at
byte 96, on its 16-byte frame grid. The same reading finds where an embedded payload, a pasted
table or a change of language begins.

The rest of this page reads a short table that runs into a sentence:

```text
id,host,bytes
1,alpha,1024
2,beta,2048
The export finished without errors.
```

## The readings

| Vantage | Window | Reads |
|---|---|---|
| causal | `[t-w, t]` | the past only |
| anti-causal | `(t, t+w]` | the future only |
| centered | `[t-w/2, t+w/2]` | both |

| Reading | Meaning |
|---|---|
| disagreement | `\|causal - anticausal\|` at the point, from 0 to 1 |
| contested point | a local maximum of disagreement that reaches the threshold, 0.2 by default |

The windows are 32 bytes. Of two contested points closer than the minimum gap, 8 bytes by
default, the stronger is kept, a tie keeping the earlier, so the strongest point of a change is
always the one reported. Near the input's ends a window runs past the data, so where the input
is long enough the points within half a window of either end are not contested.

At the byte grain, the default, the point is a byte, and the bytes are read as UTF-8: a letter
outside ASCII is a letter on each of its bytes, any other character outside ASCII counts once by
its class, and a byte that is not part of a well-formed character is a class of its own, so the accents inside
a word are not a change of text. `--grain token` reads the same three windows over the
sequence of token kinds, and `--grain super` over the sequence of
[supertoken](../../../explanation/architecture/#supertokens) roles, so the point is a token or a supertoken.

## Reading the axis

The summary counts the contested points and names the strongest:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex observe --text 'id,host,bytes
1,alpha,1024
2,beta,2048
The export finished without errors.'
trex observe: 74 bytes, 3 contested point(s)
  peak observer-dependence: 0.53 at byte 38
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::observation::analyze(b"id,host,bytes\n1,alpha,1024\n2,beta,2048\nThe export finished without errors.");
assert_eq!(field.contested, [20, 38, 49]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> text = "id,host,bytes\n1,alpha,1024\n2,beta,2048\nThe export finished without errors."
>>> r = trex.axes.observation(text)
>>> r.length, r.grain, [c.offset for c in r.contested], r.peak.offset, round(r.peak.disagreement, 2)
(74, 'byte', [20, 38, 49], 38, 0.53)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $text = "id,host,bytes`n1,alpha,1024`n2,beta,2048`nThe export finished without errors."
PS> Measure-TrexObservation $text | Select-Object Length, Grain, Peak | Format-List

Length : 74
Grain  : Byte
Peak   : at 38, disagreement 0.5264722
```
{{< /tab >}}
{{< /tabs >}}

The peak is byte 38, the line break that ends the table: the window behind it holds rows of
digits and commas, the window ahead holds words.

### Every point's reading

`--field` samples the three readings and their disagreement across the input; in PowerShell
`-Detail` and in Python `detail=True` add a frame at every point:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex observe --field --text 'id,host,bytes
1,alpha,1024
2,beta,2048
The export finished without errors.' | head -6
trex observe: 74 bytes, 3 contested point(s)
  peak observer-dependence: 0.53 at byte 38
  vantage (causal | anticausal | centered | disagree), every 3 bytes:
    @     0  c=0.00 a=0.68 m=0.55 |d|=0.68
    @     3  c=0.35 a=0.71 m=0.50 |d|=0.36
    @     6  c=0.25 a=0.77 m=0.56 |d|=0.51
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::observation::analyze(b"id,host,bytes\n1,alpha,1024\n2,beta,2048\nThe export finished without errors.");
let peak = &field.frames[38];
assert!(peak.causal > peak.anticausal, "behind the break the classes are more varied");
assert!((peak.disagreement - 0.53).abs() < 0.01);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> d = trex.axes.observation(text, detail=True)
>>> f = d.frames[38]
>>> repr(f.text), round(f.causal, 2), round(f.anticausal, 2), round(f.disagreement, 2)
("'\\n'", 0.76, 0.23, 0.53)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexObservation $text -Detail).Frames[38] | Format-List Offset, Causal, Anticausal, Disagreement

Offset       : 38
Causal       : 0.7605727
Anticausal   : 0.2341005
Disagreement : 0.5264722
```
{{< /tab >}}
{{< /tabs >}}

### Contested points

`--contested` lists each contested point with the text around it:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex observe --contested --text 'id,host,bytes
1,alpha,1024
2,beta,2048
The export finished without errors.'
trex observe: 74 bytes, 3 contested point(s)
  peak observer-dependence: 0.53 at byte 38
  contested points (3):
    @    20  ...tes 1,alpha,1024 2,beta,...
    @    38  ...,beta,2048 The export fi...
    @    49  ...The export finished with...
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::observation::analyze(b"id,host,bytes\n1,alpha,1024\n2,beta,2048\nThe export finished without errors.");
let strongest = field.contested.iter().copied().max_by(|&a, &b| field.frames[a].disagreement.total_cmp(&field.frames[b].disagreement));
assert_eq!(strongest, Some(38));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(c.offset, round(c.disagreement, 2)) for c in r.contested]
[(20, 0.25), (38, 0.53), (49, 0.44)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexObservation $text).Contested | Format-Table Offset, Disagreement

Offset Disagreement
------ ------------
    20         0.25
    38         0.53
    49         0.44
```
{{< /tab >}}
{{< /tabs >}}

Byte 20 is inside the table, where `alpha` breaks the run of digits; byte 49 inside the
sentence, where the first words give way to longer ones. The table's end is the strongest point.

### Code in the middle of a sentence

The windows behind and ahead differ on both sides of a change, so a span that begins and ends
inside one line is contested at both edges. This is the passage the
[spectral](../spectral/#change-points) page reads, a line of code from byte 134 to 184 in the
middle of a sentence:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex observe --contested --text 'The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.'
trex observe: 324 bytes, 5 contested point(s)
  peak observer-dependence: 0.30 at byte 191
  contested points (5):
    @   107  ...or each one. For every r...
    @   124  ...every reply it runs a[i]...
    @   132  ...ply it runs a[i]=(b[j]*c...
    @   191  ...i;} on the payload, and ...
    @   207  ...ad, and it sends the tot...
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::observation::analyze(b"The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.");
assert_eq!(field.contested, [107, 124, 132, 191, 207]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> mixed = "The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers."
>>> [c.offset for c in trex.axes.observation(mixed).contested]
[107, 124, 132, 191, 207]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $mixed = 'The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.'
PS> (Measure-TrexObservation $mixed).Contested.Offset
107
124
132
191
207
```
{{< /tab >}}
{{< /tabs >}}

Byte 132 is two bytes before the code begins and byte 191 seven after it ends, the strongest
point of the passage. Spectral, which reads only the bytes already behind each point, places its
change points at 138 and 203, a few bytes later each. Observation also contests three points in the
sentences around the code, 107, 124 and 207.

### The token and supertoken grains

At the token grain each point is a token and the windows hold token kinds. The table's tokens
alternate numbers, words and commas and the sentence's are words, so the one contested token is
`2048`, the table's last value:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex observe --grain token --contested --text 'id,host,bytes
1,alpha,1024
2,beta,2048
The export finished without errors.'
trex observe (token grain): 74 bytes, 21 tokens, 1 contested point(s)
  peak observer-dependence: 0.29 at byte 34
  contested points (1):
    @    34  ...24 2,beta,2048 The expor...
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"id,host,bytes\n1,alpha,1024\n2,beta,2048\nThe export finished without errors.";
let toks: Vec<trex::token::Token> = trex::lexer::lex(text).into_iter().filter(|t| t.is_significant()).collect();
let cfg = trex::observation::ObservationConfig::default();
assert_eq!(trex::observation::contested_tokens(&toks, &cfg), [34]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> t = trex.axes.observation(text, grain="token")
>>> t.grain, t.units, [(c.text, c.offset) for c in t.contested]
('token', 21, [('2048', 34)])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexObservation $text -Grain Token).Contested | Format-Table Offset, Text

Offset Text
------ ----
    34 2048
```
{{< /tab >}}
{{< /tabs >}}

`--grain super` reads the supertokens the same way; its summary counts them, `4 supertokens`
over this input.

## Choosing a threshold

**The threshold.** Disagreement runs from 0 to 1, and `--field` shows the values around the
changes you care about. Set the threshold between the disagreement of the changes you want and
the background inside each kind of text. Here the table's own variation peaks at 0.25 and the
changes into the sentence read 0.53 and 0.44, so 0.4 keeps only the change:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex observe --contested --contested-threshold 0.4 --text 'id,host,bytes
1,alpha,1024
2,beta,2048
The export finished without errors.'
trex observe: 74 bytes, 2 contested point(s)
  peak observer-dependence: 0.53 at byte 38
  contested points (2):
    @    38  ...,beta,2048 The export fi...
    @    49  ...The export finished with...
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::observation::{ObservationConfig, analyze_with};

let cfg = ObservationConfig { contested_threshold: 0.4, ..ObservationConfig::default() };
let field = analyze_with(b"id,host,bytes\n1,alpha,1024\n2,beta,2048\nThe export finished without errors.", &cfg);
assert_eq!(field.contested, [38, 49]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [c.offset for c in trex.axes.observation(text, contested_threshold=0.4).contested]
[38, 49]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexObservation $text -ContestedThreshold 0.4).Contested.Offset
38
49
```
{{< /tab >}}
{{< /tabs >}}

The comparison is inclusive: a point whose disagreement equals the threshold is contested. A
higher threshold only removes points, since the strongest point of each change is always kept.

**The minimum gap.** At the byte grain, one change often raises several nearby maxima. The gap
says how close two contested points may be; of two closer than it, the weaker is dropped. A gap of
16 bytes treats the end of the table and the first words of the sentence as one change and keeps
the stronger, byte 38:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex observe --contested --contested-min-gap 16 --text 'id,host,bytes
1,alpha,1024
2,beta,2048
The export finished without errors.'
trex observe: 74 bytes, 2 contested point(s)
  peak observer-dependence: 0.53 at byte 38
  contested points (2):
    @    20  ...tes 1,alpha,1024 2,beta,...
    @    38  ...,beta,2048 The export fi...
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::observation::{ObservationConfig, analyze_with};

let cfg = ObservationConfig { contested_min_gap: 16, ..ObservationConfig::default() };
let field = analyze_with(b"id,host,bytes\n1,alpha,1024\n2,beta,2048\nThe export finished without errors.", &cfg);
assert_eq!(field.contested, [20, 38]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [c.offset for c in trex.axes.observation(text, contested_min_gap=16).contested]
[20, 38]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexObservation $text -ContestedMinGap 16).Contested.Offset
20
38
```
{{< /tab >}}
{{< /tabs >}}

The gap applies at the byte grain; a token or a supertoken is already a unit of its own.

## Declarations

At the token and supertoken grains a declared shape or kind changes the tokens the windows hold.
The axis takes the declarations a scan takes there: `--lib`, `--shape`, `--shape-after`,
`--kind`, `--let` and `--declare` on the command line, `-Library` in PowerShell and `lib=` in
Python ([pattern files](../../pattern-files/)). The byte grain reads no tokens, so it refuses
them:

```console
$ trex observe --shape 'id = `[a-z]{2}-[0-9]{2}`' --text 'ticket ab-12 and cd-34 closed'
trex observe: declarations decide how tokens are read, and the byte grain reads bytes; --grain token or super reads them
```

## In a pattern

`@ambiguous` is a zero-width anchor at a contested point, at the byte grain unless a grain is
named: `@ambiguous:token` and `@ambiguous:super` read the other two. A written cut replaces the
threshold: `@ambiguous>k` keeps the points whose disagreement passes `k`, and `@ambiguous>=k`
those that reach it, `k` running from 0 to 1; a bare anchor is `@ambiguous>=0.2`. The anchor
holds at the token that covers the point, so a point inside whitespace holds at no token:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@ambiguous:token .' --text 'id,host,bytes
1,alpha,1024
2,beta,2048
The export finished without errors.'
[34..38] "2048"

$ trex scan '@ambiguous:token>0.3 .' --text 'id,host,bytes
1,alpha,1024
2,beta,2048
The export finished without errors.'
no match
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = "id,host,bytes\n1,alpha,1024\n2,beta,2048\nThe export finished without errors.";
let found = |pat: &str| -> Vec<&str> {
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| &text[s.range()]).collect()
};
assert_eq!(found(r"@ambiguous:token ."), ["2048"]);
assert!(found(r"@ambiguous:token>0.3 .").is_empty());
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"@ambiguous:token .").scan(text)]
['2048']
>>> [m.text for m in trex.Pattern(r"@ambiguous:token>0.3 .").scan(text)]
[]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@ambiguous:token .' -InputObject $text -Raw
2048
```
{{< /tab >}}
{{< /tabs >}}

The contested token reads 0.29, so a cut of `>0.3` removes it. An
[explanation](../../matching/#explanations) repeats a written cut, `contested >0.3 at the token
grain`, and holds it in `${@ambiguous.cut}`.

## Data model

`ObservationFrame`, one point's reading:

| Field | Type | Meaning |
|---|---|---|
| `causal` | `f32` | entropy of the window behind |
| `anticausal` | `f32` | entropy of the window ahead |
| `centered` | `f32` | entropy of the window around |
| `disagreement` | `f32` | `\|causal - anticausal\|` |

`ObservationField` holds a frame at every byte and `contested`, the contested byte offsets.
`ObservationConfig` holds `window` (32), `contested_threshold` (0.2) and `contested_min_gap`
(8); `analyze_with(bytes, &cfg)` reads under it, and `contested_tokens(tokens, &cfg)` and
`contested_supertokens(units, &cfg)` read the other grains.

## Algorithm

At each byte the three windows count the byte classes they hold and take each count's
normalized entropy; the disagreement is the difference between the causal and anti-causal
readings. Contested points are the local maxima of disagreement that reach the threshold,
spaced by the minimum gap, the stronger of two close points kept. The windows slide by one
byte, so the pass is O(1) per byte after the first window.
