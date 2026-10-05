---
title: The magnitude axis
linkTitle: Magnitude
weight: 50
---

The magnitude axis gives every token a size on a log scale. A number's size is its order of
magnitude, `log10` of its value, so `5000000000` reads 9.70 and `30` reads 1.48. Any other token's
size is `log2` of its length in bytes, so an eight-letter word reads 3 and a one-byte punctuation
mark reads 0. Beside each size the axis keeps the change from the token before and the energy of
the window behind it.

Source: [`src/magnitude.rs`](https://github.com/Variably-Constant/trex/blob/main/src/magnitude.rs).

## What it reads that nothing else does

```text
timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080
```

Every value here is a number. Shape reads one silhouette for all four, spectral one digit
texture, and orbit folds none together. Magnitude reads them as `m = 1.48, 0.70, 9.70, 3.91`:
the value of `max_bytes` is six orders above the next largest. The same reading finds an address among indices,
a magic constant among small literals, a size field among flags, a price among quantities.

Every example on this page reads that line, except where a reading needs an input of its own.

## The readings

| Quantity | Definition | Reads |
|---|---|---|
| magnitude `m(t)` | a number's `log10(\|value\|)`, any other token's `log2(byte_len)` | the scale of each token |
| gradient | `m[i] - m[i-1]`, the first token `0` | the direction and rate of scale change |
| energy | `sum(m^2)` over the 16 tokens up to and including this one | where scale concentrates |
| jump | a token whose `\|gradient\|` is at least the jump threshold, 3 by default | a break in scale |
| outlier | a token at least `outlier_sigma` standard deviations from the mean magnitude, 2.5 by default | a value out of place in its stream |

A number keeps its own scale whatever its spelling: `0x1F`, `1_000_000` and `2.5e9` are parsed
for their value. A word, a path or a quoted string reads its length, which is why every
punctuation mark reads 0 and why a sentence's magnitude rises and falls with its word lengths.
`6.02e23` and the same number written out read one magnitude:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --field --text 'mole = 6.02e23 ; same = 602000000000000000000000'
trex magnitude: 48 bytes, 7 tokens, total energy 1138.9, 3 scale-jump(s)
  peak magnitude: 23.78 at '602000000000000000000000'
  per-token (token -> magnitude | gradient | energy):
    mole           m=  2.00 dm= +0.00 E=    4.0
    =              m=  0.00 dm= -2.00 E=    4.0
    6.02e23        m= 23.78 dm=+23.78 E=  569.5
    ;              m=  0.00 dm=-23.78 E=  569.5
    same           m=  2.00 dm= +2.00 E=  573.5
    =              m=  0.00 dm= -2.00 E=  573.5
    60200000000000 m= 23.78 dm=+23.78 E= 1138.9
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::magnitude::analyze_bytes(b"mole = 6.02e23 ; same = 602000000000000000000000");
assert_eq!(field.n_tokens, 7);
let (written, spelled) = (field.frames[2].magnitude, field.frames[6].magnitude);
assert!((written - 23.78).abs() < 0.01 && (written - spelled).abs() < 1e-4);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.magnitude("mole = 6.02e23 ; same = 602000000000000000000000", detail=True)
>>> [(f.text, round(f.magnitude, 2)) for f in r.frames if f.text[0].isdigit()]
[('6.02e23', 23.78), ('602000000000000000000000', 23.78)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexMagnitude 'mole = 6.02e23 ; same = 602000000000000000000000' -Detail).Frames | Where-Object Text -Match '^\d' | Format-Table Text, Magnitude

Text                     Magnitude
----                     ---------
6.02e23                      23.78
602000000000000000000000     23.78
```
{{< /tab >}}
{{< /tabs >}}

## Reading the axis

The summary line counts the tokens, totals the energy, counts the jumps and names the token of
greatest magnitude:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --text 'timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080'
trex magnitude: 65 bytes, 15 tokens, total energy 141.8, 5 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::magnitude::analyze_bytes(b"timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080");
let peak = field.frames.iter().map(|f| f.magnitude).fold(0.0f32, f32::max);
assert!((peak - 9.70).abs() < 0.01);
assert_eq!((field.n_tokens, field.jumps.len()), (15, 5));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.magnitude("timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080")
>>> r.tokens, round(r.total_energy, 1), len(r.jumps)
(15, 141.8, 5)
>>> r.peak.text, r.peak.offset, round(r.peak.magnitude, 2)
('5000000000', 41, 9.7)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $line = 'timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080'
PS> Measure-TrexMagnitude $line | Select-Object Tokens, Peak | Format-List

Tokens : 15
Peak   : 5000000000 at 41
```
{{< /tab >}}
{{< /tabs >}}

### Every token's reading

`--field` lists each token with its magnitude `m`, its gradient `dm` and its energy `E`, and
`--limit N` stops the list after `N` tokens and counts the rest. In PowerShell `-Detail` and in
Python `detail=True` add the same frames to the report:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --field --text 'timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080'
trex magnitude: 65 bytes, 15 tokens, total energy 141.8, 5 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
  per-token (token -> magnitude | gradient | energy):
    timeout        m=  2.81 dm= +0.00 E=    7.9
    =              m=  0.00 dm= -2.81 E=    7.9
    30             m=  1.48 dm= +1.48 E=   10.1
    ;              m=  0.00 dm= -1.48 E=   10.1
    retries        m=  2.81 dm= +2.81 E=   17.9
    =              m=  0.00 dm= -2.81 E=   17.9
    5              m=  0.70 dm= +0.70 E=   18.4
    ;              m=  0.00 dm= -0.70 E=   18.4
    max_bytes      m=  3.17 dm= +3.17 E=   28.5
    =              m=  0.00 dm= -3.17 E=   28.5
    5000000000     m=  9.70 dm= +9.70 E=  122.6
    ;              m=  0.00 dm= -9.70 E=  122.6
    port           m=  2.00 dm= +2.00 E=  126.6
    =              m=  0.00 dm= -2.00 E=  126.6
    8080           m=  3.91 dm= +3.91 E=  141.8
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080";
let field = trex::magnitude::analyze_bytes(text);
let first: Vec<(&[u8], f32)> = field
    .spans
    .iter()
    .zip(&field.frames)
    .take(3)
    .map(|(&(s, e), f)| (&text[s..e], (f.magnitude * 100.0).round() / 100.0))
    .collect();
assert_eq!(first, [(&b"timeout"[..], 2.81), (&b"="[..], 0.0), (&b"30"[..], 1.48)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> r = trex.axes.magnitude("timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080", detail=True)
>>> [(f.text, round(f.magnitude, 2), round(f.gradient, 2)) for f in r.frames[:3]]
[('timeout', 2.81, 0.0), ('=', 0.0, -2.81), ('30', 1.48, 1.48)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexMagnitude $line -Detail).Frames | Select-Object -First 3 | Format-Table Offset, Text, Magnitude, Gradient

Offset Text    Magnitude Gradient
------ ----    --------- --------
     0 timeout      2.81     0.00
     8 =            0.00    -2.81
    10 30           1.48     1.48
```
{{< /tab >}}
{{< /tabs >}}

Over fewer than 16 tokens the energy window holds every token so far, so `E` here only grows;
over a longer input it rises and falls as large tokens enter and leave the window.

### Scale jumps

A jump is a token whose magnitude differs from the token before by at least the jump threshold.
`--jumps` lists them with the text that follows each; the report's jumps are frames:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --jumps --text 'timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080'
trex magnitude: 65 bytes, 15 tokens, total energy 141.8, 5 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
  scale discontinuities (5 jump(s)):
    @    29  max_bytes = 5000000000 ;
    @    39  = 5000000000 ; port = 80
    @    41  5000000000 ; port = 8080
    @    52  ; port = 8080
    @    61  8080
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::magnitude::analyze_bytes(b"timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080");
assert_eq!(field.jumps, [29, 39, 41, 52, 61]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> r = trex.axes.magnitude("timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080")
>>> [(f.text, f.offset) for f in r.jumps]
[('max_bytes', 29), ('=', 39), ('5000000000', 41), (';', 52), ('8080', 61)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexMagnitude $line).Jumps | Format-Table Offset, Text, Gradient

Offset Text       Gradient
------ ----       --------
    29 max_bytes      3.17
    39 =             -3.17
    41 5000000000     9.70
    52 ;             -9.70
    61 8080           3.91
```
{{< /tab >}}
{{< /tabs >}}

Each large value jumps twice, once on the way in and once on the way out, and so does a long
name beside the one-byte `=`: `max_bytes` reads 3.17 and `=` reads 0, a step past three orders.

### Outliers

An outlier is a token whose magnitude is at least `outlier_sigma` standard deviations from the
stream's mean. A jump is local, a change from one token to the next; an outlier is global, a
token out of place among all of them:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --outliers --text 'timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080'
trex magnitude: 65 bytes, 15 tokens, total energy 141.8, 5 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
  scale outliers (1):
    '5000000000'  m=9.70
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080";
let field = trex::magnitude::analyze_bytes(text);
let outliers: Vec<&[u8]> = field.outliers().into_iter().map(|i| &text[field.spans[i].0..field.spans[i].1]).collect();
assert_eq!(outliers, [&b"5000000000"[..]]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, round(f.magnitude, 2)) for f in r.outliers]
[('5000000000', 9.7)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexMagnitude $line).Outliers | Format-Table Offset, Text, Magnitude

Offset Text       Magnitude
------ ----       ---------
    41 5000000000      9.70
```
{{< /tab >}}
{{< /tabs >}}

### Energy

Energy is the sum of squared magnitudes over the window up to a token, so it is high where large
tokens crowd together. `--energy` lists the tokens of greatest energy, `--top K` of them, three by
default; the report carries every frame's energy for the other surfaces to rank:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --energy --top 2 --text 'timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080'
trex magnitude: 65 bytes, 15 tokens, total energy 141.8, 5 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
  heaviest points (top 2 of 15 by local energy):
    '8080'  E=141.8
    'port'  E=126.6
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080";
let field = trex::magnitude::analyze_bytes(text);
let mut order: Vec<usize> = (0..field.frames.len()).collect();
order.sort_by(|&a, &b| field.frames[b].energy.total_cmp(&field.frames[a].energy));
let heaviest: Vec<&[u8]> = order.iter().take(2).map(|&i| &text[field.spans[i].0..field.spans[i].1]).collect();
assert_eq!(heaviest, [&b"8080"[..], &b"port"[..]]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> frames = trex.axes.magnitude("timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080", detail=True).frames
>>> [(f.text, round(f.energy, 1)) for f in sorted(frames, key=lambda f: f.energy, reverse=True)[:2]]
[('8080', 141.8), ('port', 126.6)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexMagnitude $line -Detail).Frames | Sort-Object Energy -Descending | Select-Object -First 2 | Format-Table Text, Energy

Text Energy
---- ------
8080 141.82
port 126.55
```
{{< /tab >}}
{{< /tabs >}}

## Choosing a threshold

Each cut on this axis is a number of orders of magnitude, and the readings themselves show where a
cut would be. Read `--field` first, then choose the threshold that separates what you want from
what you do not.

**A predicate's `k`.** In the field above, the values read 1.48, 0.70, 9.70 and 3.91, and every
name reads between 2 and 3.17. `\M{>5}` cuts in the wide gap between 3.91 and 9.70, so it takes
the one large value and nothing else. `\M{>3}` would also take `8080` and the name `max_bytes`.
Choose `k` in the widest gap of the values you care about, so a small change in the input does
not move a token across it.

**The jump threshold.** A jump compares neighbors, so it reads the spread between adjacent
tokens rather than the values. Punctuation reads 0, so a word or number beside an `=` jumps by
its own magnitude: at the default of 3, `max_bytes` (3.17) and `8080` (3.91) jump. To keep only
the breaks a large value makes, raise the threshold above the largest neighbor step you want to
ignore:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --jumps --jump-threshold 5 --text 'timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080'
trex magnitude: 65 bytes, 15 tokens, total energy 141.8, 2 scale-jump(s)
  peak magnitude: 9.70 at '5000000000'
  scale discontinuities (2 jump(s)):
    @    41  5000000000 ; port = 8080
    @    52  ; port = 8080
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::magnitude::{MagnitudeConfig, analyze_with};

let text = b"timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080";
let cfg = MagnitudeConfig { jump_threshold: 5.0, ..MagnitudeConfig::default() };
let field = analyze_with(&trex::tokutil::lex_sig(text), text, &cfg);
assert_eq!(field.jumps, [41, 52]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, f.offset) for f in trex.axes.magnitude("timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080", jump_threshold=5).jumps]
[('5000000000', 41), (';', 52)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexMagnitude $line -JumpThreshold 5).Jumps | Format-Table Offset, Text

Offset Text
------ ----
    41 5000000000
    52 ;
```
{{< /tab >}}
{{< /tabs >}}

The comparison is inclusive: a step of exactly the threshold is a jump.

**The outlier sigma.** An outlier is measured against the whole stream's spread, so the cut is a
count of standard deviations. Lower it to admit milder outliers, raise it to keep only the
extreme: here `5000000000` is far enough out that it is the one outlier at 1.5 and at the
default 2.5 alike. On the command line the setting is `--outlier-sigma`, in PowerShell
`-OutlierSigma` and in Python `outlier_sigma=`.

## Declarations

A declared shape or kind changes what a token is, and so changes its magnitude. `port = 8080`
declared as one token is no longer a name, a sign and a value of 3.91: it is one token of 11
bytes, which reads `log2(11) = 3.46`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex magnitude --text 'retries = 5 ; port = 8080'
trex magnitude: 25 bytes, 7 tokens, total energy 27.6, 1 scale-jump(s)
  peak magnitude: 3.91 at '8080'

$ trex magnitude --shape 'setting = `port = [0-9]{1,5}`' --text 'retries = 5 ; port = 8080'
trex magnitude: 25 bytes, 5 tokens, total energy 20.3, 1 scale-jump(s)
  peak magnitude: 3.46 at 'port = 8080'
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"retries = 5 ; port = 8080";
let mut shapes = trex::ShapeSet::new();
shapes.declare("setting = `port = [0-9]{1,5}`", trex::Precedence::Before).expect("a bounded shape");
let toks: Vec<trex::token::Token> = trex::lexer::lex_with_shapes(text, &trex::lexer::blob_runs(text), &shapes, 0)
    .into_iter()
    .filter(|t| t.is_significant())
    .collect();
assert_eq!(trex::magnitude::analyze(&toks, text).n_tokens, 5);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> lib = trex.Library()
>>> lib.shape("setting", r"port = [0-9]{1,5}")
>>> r = trex.axes.magnitude("retries = 5 ; port = 8080", lib=lib)
>>> r.tokens, r.peak.text, round(r.peak.magnitude, 2)
(5, 'port = 8080', 3.46)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $lib = New-TrexLibrary
PS> Register-TrexAtom setting -Shape 'port = [0-9]{1,5}' -Library $lib
PS> (Measure-TrexMagnitude 'retries = 5 ; port = 8080' -Library $lib).Tokens
5
```
{{< /tab >}}
{{< /tabs >}}

Every axis that reads tokens takes the declarations a scan takes: `--lib`, `--shape`,
`--shape-after`, `--kind`, `--let` and `--declare` on the command line, `-Library` in PowerShell
and `lib=` in Python ([pattern files](../../pattern-files/)).

## In a pattern

`\M{>k}` and `\M{<k}` hold where a token's magnitude crosses `k`, whatever the token's kind;
`\N{mag>k}` is a number atom with a magnitude predicate, so it skips the names and punctuation
`\M` would also weigh. The relative forms `\N{>+1}` read a value against a
[context](../context/) instead of a fixed `k`.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\M{>5}' --text 'timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080'
[41..51] "5000000000"

$ trex scan '\N{mag>3}' --text 'timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080'
[41..51] "5000000000"
[61..65] "8080"

$ trex scan '\N{mag<1}' --text 'timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080'
[25..26] "5"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let found = |pat: &str| -> Vec<String> {
    let text = "timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080";
    let p = trex::parse(pat).expect("valid pattern");
    trex::scan(&p, text.as_bytes()).iter().map(|s| text[s.range()].to_string()).collect()
};
assert_eq!(found(r"\M{>5}"), ["5000000000"]);
assert_eq!(found(r"\N{mag>3}"), ["5000000000", "8080"]);
assert_eq!(found(r"\N{mag<1}"), ["5"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> text = "timeout = 30 ; retries = 5 ; max_bytes = 5000000000 ; port = 8080"
>>> [m.text for m in trex.Pattern(r"\M{>5}").scan(text)]
['5000000000']
>>> [m.text for m in trex.Pattern(r"\N{mag>3}").scan(text)]
['5000000000', '8080']
>>> [m.text for m in trex.Pattern(r"\N{mag<1}").scan(text)]
['5']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\M{>5}' -InputObject $line -Raw
5000000000

PS> Select-TrexMatch '\N{mag>3}' -InputObject $line -Raw
5000000000
8080

PS> Select-TrexMatch '\N{mag<1}' -InputObject $line -Raw
5
```
{{< /tab >}}
{{< /tabs >}}

`\M{<1}` over the same line would take every punctuation mark as well as `5`, since each reads 0.

## Data model

`MagnitudeFrame`, one token's reading:

| Field | Type | Meaning |
|---|---|---|
| `magnitude` | `f32` | a number's `log10(\|value\|)`, any other token's `log2(byte_len)` |
| `gradient` | `f32` | the change from the previous token |
| `energy` | `f32` | local `sum(m^2)` over the trailing `energy_window` |

`MagnitudeField`, keyed by byte offset:

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | token count |
| `spans` | `Vec<(usize, usize)>` | byte span per token |
| `frames` | `Vec<MagnitudeFrame>` | one per token |
| `jumps` | `Vec<usize>` | byte offsets where `\|gradient\|` reaches the threshold |
| `total_energy` | `f32` | `sum(m^2)` over the whole stream |

Query API: `magnitude_at(byte)`, `gradient_at(byte)`, `energy_of(start, end)`, `outliers()`.
`MagnitudeConfig` holds `energy_window` (16), `jump_threshold` (3) and `outlier_sigma` (2.5);
`analyze_with(tokens, bytes, &cfg)` reads under it.

## Algorithm

One pass over the token stream:

1. magnitude: a number's value parsed (decimal digits, a `0x`, `0b` or `0o` prefix, an exponent,
   underscores between digits) to
   `log10(|value|)`, else `log2(byte_len)`;
2. statistics: the stream's mean and standard deviation of `m`, and `total_energy`, in the same
   scan;
3. gradient: `m[i] - m[i-1]`, the first token `0`;
4. jump: `|gradient| >= jump_threshold` records a discontinuity;
5. energy: local `sum(m^2)` over the trailing `energy_window`.

## Cost

| Kernel | Per-token cost | Bounded by |
|---|---|---|
| magnitude | O(1) | numeric parse of the token text |
| statistics | O(1) | one accumulation pass |
| gradient | O(1) | fixed |
| energy | O(1) amortized | `energy_window` |
