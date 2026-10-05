---
title: The flow axis
linkTitle: Flow
weight: 70
---

The flow axis follows a signal along the input and reads its trend: how steeply it is rising or
falling, which way it is heading, how long it has kept heading that way, and where it turns. The
signal is each token's [magnitude](../magnitude/) by default, its [stress](../stress/) depth or
its length on request, and any value a Rust caller computes per token. With `--analytic` the
axis also reads how widely the signal swings and how fast.

Source: [`src/flow.rs`](https://github.com/Variably-Constant/trex/blob/main/src/flow.rs).

## What it reads that nothing else does

```text
3 9 27 81 243 729 243 81 27 9
```

Each value is three times the one before until the turn, half an order of magnitude a step, so
magnitude finds no jump and no outlier: no step is far from its neighbor and no value far from
the rest. Flow reads the run: a slope of +0.48 a token, a rising run six tokens long, and the
reversal at the second `27`, where the trend has turned to falling. The same reading finds a
queue filling, a latency creeping up, or a count that peaked and is draining.

Most examples on this page read that line.

## The readings

| Reading | Meaning |
|---|---|
| slope | the average change a step over the trailing window, 4 units by default |
| direction | rising, steady or falling: a slope within the steady band of zero, 0.05 by default, is steady |
| momentum | how many units in a row have held the same rising or falling direction |
| reversal | a unit where the direction flips between rising and falling; a steady stretch between them does not count as a flip |

| Signal | `--signal` | Source |
|---|---|---|
| `Magnitude` | `magnitude` | the [magnitude](../magnitude/) of each token |
| `StressDepth` | `stress` | the [stress](../stress/) depth of each token |
| `Length` | `length` | each token's byte length |

The unit is a token by default; `--grain super` reads one value a
[supertoken](../../../explanation/architecture/#supertokens) instead.

## Reading the axis

The summary names the signal, counts the units and the reversals, and gives the longest run:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow --text '3 9 27 81 243 729 243 81 27 9'
trex flow (signal magnitude): 29 bytes, 10 tokens, 1 reversal(s)
  peak momentum: 6 (rising)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::flow::analyze_bytes(b"3 9 27 81 243 729 243 81 27 9", trex::flow::Signal::Magnitude);
assert_eq!((field.n_tokens, field.reversals.len()), (10, 1));
assert_eq!(field.frames.iter().map(|f| f.momentum).max(), Some(6));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.flow("3 9 27 81 243 729 243 81 27 9")
>>> r.tokens, r.signal, r.grain, r.peak_momentum.text, r.peak_momentum.momentum
(10, 'magnitude', 'token', '243', 6)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $line = '3 9 27 81 243 729 243 81 27 9'
PS> Measure-TrexFlow $line | Select-Object Tokens, Signal, Grain, PeakMomentum | Format-List

Tokens       : 10
Signal       : Magnitude
Grain        : Token
PeakMomentum : 243 at 18, Rising
```
{{< /tab >}}
{{< /tabs >}}

### Every unit's reading

`--field` lists each unit's slope, direction and momentum, and `--limit N` stops this list and
the [analytic](#the-analytic-reading) one after `N` units and counts the rest. `-Detail` in
PowerShell and `detail=True` in Python add the same frames:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow --field --text '3 9 27 81 243 729 243 81 27 9'
trex flow (signal magnitude): 29 bytes, 10 tokens, 1 reversal(s)
  peak momentum: 6 (rising)
  per-token (token -> slope | direction | momentum):
    3              slope=  +0.00 --   mom=0
    9              slope=  +0.48 up   mom=1
    27             slope=  +0.48 up   mom=2
    81             slope=  +0.48 up   mom=3
    243            slope=  +0.48 up   mom=4
    729            slope=  +0.48 up   mom=5
    243            slope=  +0.24 up   mom=6
    81             slope=  +0.00 --   mom=0
    27             slope=  -0.24 down mom=1
    9              slope=  -0.48 down mom=2
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::flow::analyze_bytes(b"3 9 27 81 243 729 243 81 27 9", trex::flow::Signal::Magnitude);
let directions: Vec<i8> = field.frames.iter().map(|f| f.direction).collect();
assert_eq!(directions, [0, 1, 1, 1, 1, 1, 1, 0, -1, -1]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> r = trex.axes.flow("3 9 27 81 243 729 243 81 27 9", detail=True)
>>> [(f.text, round(f.slope, 2), f.direction) for f in r.frames[5:9]]
[('729', 0.48, 'rising'), ('243', 0.24, 'rising'), ('81', 0.0, 'steady'), ('27', -0.24, 'falling')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexFlow $line -Detail).Frames | Select-Object -Skip 5 -First 4 | Format-Table Offset, Text, Slope, Direction, Momentum

Offset Text Slope Direction Momentum
------ ---- ----- --------- --------
    14 729   0.48    Rising        5
    18 243   0.24    Rising        6
    22 81    0.00    Steady        0
    25 27   -0.24   Falling        1
```
{{< /tab >}}
{{< /tabs >}}

The slope averages over the window, so it lags the turn: at the second `243` the values are
already falling, but the window still reaches back to the rise and reads +0.24. The trend crosses
zero at the second `81` and reads falling from the second `27`.

### Reversals

A reversal is the unit where the direction flips between rising and falling. The steady `81`
between the two runs is passed over, so the reversal is at the first falling unit:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow --reversals --text '3 9 27 81 243 729 243 81 27 9'
trex flow (signal magnitude): 29 bytes, 10 tokens, 1 reversal(s)
  peak momentum: 6 (rising)
  reversals (1):
    @    25  27 9
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::flow::analyze_bytes(b"3 9 27 81 243 729 243 81 27 9", trex::flow::Signal::Magnitude);
assert_eq!(field.reversals, [25]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, f.offset, f.direction) for f in r.reversals]
[('27', 25, 'falling')]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexFlow $line).Reversals | Format-Table Offset, Text, Direction

Offset Text Direction
------ ---- ---------
    25 27     Falling
```
{{< /tab >}}
{{< /tabs >}}

### Another signal

`--signal stress` follows the bracket depth and `--signal length` the token length. Over nested
calls the depth rises into the innermost bracket and turns at the first close:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow --signal stress --reversals --text 'f(g(h(x))) k(y)'
trex flow (signal stress): 15 bytes, 14 tokens, 1 reversal(s)
  peak momentum: 6 (rising)
  reversals (1):
    @     8  )) k(y)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::flow::analyze_bytes(b"f(g(h(x))) k(y)", trex::flow::Signal::StressDepth);
assert_eq!(field.reversals, [8]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [f.offset for f in trex.axes.flow("f(g(h(x))) k(y)", signal="stress").reversals]
[8]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexFlow 'f(g(h(x))) k(y)' -Signal Stress).Reversals.Offset
8
```
{{< /tab >}}
{{< /tabs >}}

### The supertoken grain

`--grain super` reads one value a supertoken, the magnitude or length summed over its span, so
the trend is of statements rather than of tokens. The summary counts supertokens:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow --grain super --text 'x = 1 ; f(a, b) ; y = 22 ; g(c) ; z = 333'
trex flow (signal magnitude): 41 bytes, 7 supertokens, 0 reversal(s)
  peak momentum: 1 (rising)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"x = 1 ; f(a, b) ; y = 22 ; g(c) ; z = 333";
let field = trex::flow::analyze_supertokens_with(
    &trex::lexer::lex(text),
    text,
    trex::flow::Signal::Magnitude,
    &trex::flow::FlowConfig::default(),
);
assert_eq!(field.n_tokens, 7);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> s = trex.axes.flow("x = 1 ; f(a, b) ; y = 22 ; g(c) ; z = 333", grain="super")
>>> s.grain, s.units
('super', 7)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Measure-TrexFlow 'x = 1 ; f(a, b) ; y = 22 ; g(c) ; z = 333' -Grain Super | Select-Object Grain, Units | Format-List

Grain : Super
Units : 7
```
{{< /tab >}}
{{< /tabs >}}

### The analytic reading

A slope says which way a signal heads; it cannot say how widely the signal swings or how fast,
and a signal that oscillates keeps reversing however steady its swing. `--analytic` reads the
signal's analytic form at every unit: its amplitude, the envelope of the swing; its phase, the
unit's position in the swing, from -pi to pi; and its frequency, how far the phase advances a
unit, in radians.

The transform reads no further than the input goes: each unit uses the input up to 15 units past
it, so the last 15 units have no full reading, and the command prints a dash for
each. Over magnitudes that climb and fall every six tokens, `1 10 100 1000 100 10` repeated, the
phase turns once every six tokens:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow --analytic --text '1 10 100 1000 100 10 1 10 100 1000 100 10 1 10 100 1000 100 10 1 10 100 1000 100 10 1 10 100 1000 100 10 1 10 100 1000 100 10 1 10 100 1000 100 10 1 10 100 1000 100 10' | head -4
trex flow (signal magnitude): 167 bytes, 48 tokens, 14 reversal(s)
  peak momentum: 4 (rising)
  analytic (token -> amplitude | phase | frequency), latency 15 tokens:
    1              amp=  0.58 phase= -1.57 freq= +0.00
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = "1 10 100 1000 100 10 ".repeat(8);
let field = trex::flow::analytic(&trex::tokutil::lex_sig(text.as_bytes()), text.as_bytes(), trex::flow::Signal::Magnitude);
assert_eq!(field.latency, 15);
// The phase at each `1000` is the same: one full turn every six tokens.
let at = |i: usize| (field.phase[i] * 100.0).round() / 100.0;
assert_eq!(at(21), at(27));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> a = trex.axes.flow("1 10 100 1000 100 10 " * 8, analytic=True)
>>> a.latency, len(a.analytic)
(15, 33)
>>> [(f.text, round(f.phase, 2)) for f in a.analytic[21:28]]
[('1000', 0.1), ('100', 1.28), ('10', 2.09), ('1', -3.05), ('10', -1.87), ('100', -1.05), ('1000', 0.1)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $a = Measure-TrexFlow ('1 10 100 1000 100 10 ' * 8) -Analytic
PS> $a.Latency, $a.Analytic.Count
15
33
```
{{< /tab >}}
{{< /tabs >}}

The report lists the units with a full reading, 33 of the 48 here. The frequency alternates
around 2pi/6, about 1.05 radians a unit, the six-token period of the input.

## Choosing a threshold

Both settings shape how readily the trend moves.

**The window.** The slope averages over the trailing window, so a short window follows the
signal closely and turns as soon as it does, while a long one smooths a jagged signal and turns
later. With a window of 2 the second `243` already reads steady and the fall begins one unit
sooner:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex flow --reversals --window 2 --text '3 9 27 81 243 729 243 81 27 9'
trex flow (signal magnitude): 29 bytes, 10 tokens, 1 reversal(s)
  peak momentum: 5 (rising)
  reversals (1):
    @    22  81 27 9
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::flow::{FlowConfig, Signal, analyze_with};

let text = b"3 9 27 81 243 729 243 81 27 9";
let cfg = FlowConfig { window: 2, ..FlowConfig::default() };
assert_eq!(analyze_with(&trex::tokutil::lex_sig(text), text, Signal::Magnitude, &cfg).reversals, [22]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [f.offset for f in trex.axes.flow("3 9 27 81 243 729 243 81 27 9", window=2).reversals]
[22]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexFlow $line -Window 2).Reversals.Offset
22
```
{{< /tab >}}
{{< /tabs >}}

**The steady band.** A slope within the band reads steady, which neither extends a run nor ends
one in a reversal. Set it from the slopes `--field` reports: just above the size of the jitter
you want to ignore, and below the trend you want to see. Here every slope is at most 0.48, so a
band of 0.6 reads the whole line as steady and finds no reversal at all. On the command line the
setting is `--steady-band`, in PowerShell `-SteadyBand` and in Python `steady_band=`. The
comparison is inclusive: a slope of
exactly the band is steady.

## Declarations

A declared shape or kind changes what a token is, and so changes the value the signal reads at it.
Every axis that reads tokens takes the declarations a scan takes: `--lib`, `--shape`,
`--shape-after`, `--kind`, `--let` and `--declare` on the command line, `-Library` in PowerShell
and `lib=` in Python ([pattern files](../../pattern-files/)); the [magnitude](../magnitude/#declarations)
page shows one at work.

## Data model

`FlowFrame`, one unit's reading:

| Field | Type | Meaning |
|---|---|---|
| `slope` | `f32` | windowed rate of change |
| `direction` | `i8` | `-1`, `0` or `+1` |
| `momentum` | `u16` | consecutive same-direction units |

`FlowField`, keyed by byte offset:

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | unit count |
| `spans` | `Vec<(usize, usize)>` | byte span per unit |
| `frames` | `Vec<FlowFrame>` | one per unit |
| `reversals` | `Vec<usize>` | turning-point byte offsets |

`AnalyticField` holds `amplitude`, `phase` and `frequency`, one each a unit, and `latency`.
Query API: `slope_at(byte)`, `direction_at(byte)`; `analyze_signal(tokens, signal, cfg)` and
`analytic_signal(signal, cfg)` read any per-token `f32` a caller computes, and `FlowConfig`
holds `window` (4) and `steady_band` (0.05).

## Algorithm

Per unit: `slope = (signal[i] - signal[i - window]) / window`, the average trend over the
trailing window; `direction` thresholds the slope against the steady band; `momentum` is the run
length of the same non-steady direction; a reversal is recorded where the direction flips
between `+1` and `-1`. One pass, O(1) per unit; the signal costs one pass of the axis it comes
from.

The analytic reading detrends the signal by a trailing mean over 16 units, then applies a
31-tap Hamming-windowed Hilbert kernel shifted to be causal, which delays it by 15 units:
amplitude and phase are the modulus and angle of the result, and frequency the phase advance.
