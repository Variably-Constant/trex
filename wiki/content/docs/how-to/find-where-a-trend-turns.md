---
title: Find where a trend turns
linkTitle: Find where a trend turns
weight: 46
---

Read which way a series is heading, how long it has held that way, and where it turns, with no
threshold to choose ([flow](../../reference/axes/flow/)).

```console
$ cat latency.txt
12 15 40 180 950 3100 800 95 20 18
```

## Where the series turns

A reversal is a token where the trend flips between rising and falling; the peak momentum is the
token ending the longest run one way:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow latency.txt --reversals
trex flow (signal magnitude): 35 bytes, 10 tokens, 1 reversal(s)
  peak momentum: 6 (rising)
  reversals (1):
    @    26  95 20 18
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::flow::{Signal, analyze_bytes};

let field = analyze_bytes(b"12 15 40 180 950 3100 800 95 20 18\n", Signal::Magnitude);
assert_eq!(field.reversals, [26]);
assert_eq!(field.frames.iter().map(|f| f.momentum).max(), Some(6));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.flow(path="latency.txt")
>>> r.tokens, r.peak_momentum.text, r.peak_momentum.momentum
(10, '800', 6)
>>> [(f.text, f.offset, f.direction) for f in r.reversals]
[('95', 26, 'falling')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexFlow -Path ./latency.txt | Select-Object Tokens, PeakMomentum, Reversals | Format-List

Tokens       : 10
PeakMomentum : 800 at 22, Rising
Reversals    : {95 at 26, Falling}
```
{{< /tab >}}
{{< /tabs >}}

The slope at a token is the change per token averaged over the four tokens ending at it, so the
trend turns a token or two after the top: 3100 is the largest value, and the series reads as
falling from 95. The [magnitude](../../reference/axes/magnitude/) axis names the top itself.

## Every step of the trend

Each token's frame carries its slope, its direction and how many tokens in a row have held it:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow latency.txt --field
trex flow (signal magnitude): 35 bytes, 10 tokens, 1 reversal(s)
  peak momentum: 6 (rising)
  per-token (token -> slope | direction | momentum):
    12             slope=  +0.00 --   mom=0
    15             slope=  +0.10 up   mom=1
    40             slope=  +0.26 up   mom=2
    180            slope=  +0.39 up   mom=3
    950            slope=  +0.47 up   mom=4
    3100           slope=  +0.58 up   mom=5
    800            slope=  +0.33 up   mom=6
    95             slope=  -0.07 down mom=1
    20             slope=  -0.42 down mom=2
    18             slope=  -0.56 down mom=3
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::flow::analyze_bytes(b"12 15 40 180 950 3100 800 95 20 18\n", trex::flow::Signal::Magnitude);
let directions: Vec<i8> = field.frames.iter().map(|f| f.direction).collect();
assert_eq!(directions, [0, 1, 1, 1, 1, 1, 1, -1, -1, -1]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, f.direction, f.momentum) for f in trex.axes.flow(path="latency.txt", detail=True).frames]
[('12', 'steady', 0), ('15', 'rising', 1), ('40', 'rising', 2), ('180', 'rising', 3), ('950', 'rising', 4), ('3100', 'rising', 5), ('800', 'rising', 6), ('95', 'falling', 1), ('20', 'falling', 2), ('18', 'falling', 3)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexFlow -Path ./latency.txt -Detail).Frames | Select-Object Text, Slope, Direction, Momentum | Format-Table

Text Slope Direction Momentum
---- ----- --------- --------
12    0.00    Steady        0
15    0.10    Rising        1
40    0.26    Rising        2
180   0.39    Rising        3
950   0.47    Rising        4
3100  0.58    Rising        5
800   0.33    Rising        6
95   -0.07   Falling        1
20   -0.42   Falling        2
18   -0.56   Falling        3
```
{{< /tab >}}
{{< /tabs >}}

A slope within 0.05 of zero is steady, and a steady token ends no run and starts none.

## The trend of another signal

The trend is of each token's magnitude unless another signal is named: its nesting depth, as the
[stress](../../reference/axes/stress/) axis reads it, or its length in bytes. The depth of a
nested call deepens for eleven tokens and turns at the first closing bracket:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow --signal stress --text 'f(a, g(b, h(c, d)), e)' --reversals
trex flow (signal stress): 22 bytes, 18 tokens, 1 reversal(s)
  peak momentum: 11 (rising)
  reversals (1):
    @    17  ), e)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::flow::analyze_bytes(b"f(a, g(b, h(c, d)), e)", trex::flow::Signal::StressDepth);
assert_eq!(field.reversals, [17]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, f.offset) for f in trex.axes.flow("f(a, g(b, h(c, d)), e)", signal="stress").reversals]
[(')', 17)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexFlow 'f(a, g(b, h(c, d)), e)' -Signal Stress).Reversals

Offset Text Slope Direction Momentum
------ ---- ----- --------- --------
17     )    -0.5  Falling   1
```
{{< /tab >}}
{{< /tabs >}}

`--signal length`, `-Signal Length` and `signal="length"` read the trend of token lengths.
