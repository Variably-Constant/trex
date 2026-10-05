---
title: The stress axis
linkTitle: Stress
weight: 60
---

The stress axis reads the brackets around every token. Its depth is how many brackets enclose
it, its strain how long the innermost of them has been held open, and its load how long all of
them together have been held open. A flat stream carries no stress; a deeply nested one builds
load as it goes in and releases it as the brackets close.

Source: [`src/stress.rs`](https://github.com/Variably-Constant/trex/blob/main/src/stress.rs).

## What it reads that nothing else does

```text
retry(fetch(url), 3)
retry(fetch(url, timeout=30, headers=h, auth=a, proxy=p), 3)
```

Both calls nest two deep, and relation counts their enclosures over each whole call, 5 against
37. Stress reads at each token how long every bracket around it has stayed open: the second
call's load peaks at `p`, 36, with `fetch` open 17 tokens and `retry` 19, and drops at the `)`
closing `fetch`; the first call's load never passes 6. The same reading finds a long argument
list, or a block held open across a file.

The rest of this page reads one line, which nests four deep and holds two kinds of collection:

```text
call(a, f(g(h(x))), [1, {k: v}]) ; done
```

## The readings

| Reading | Definition |
|---|---|
| depth | the number of brackets enclosing the token |
| strain | the tokens the innermost open bracket has been held open |
| load | the sum of every open bracket's stretch, `sum(now - opened)` |

| Event | Definition |
|---|---|
| peak | a token at least `peak_min_depth` deep, as deep as both neighbors and deeper than one of them |
| fracture | the first token of a release, where the depth falls from a level at least `fracture_min_depth` deep |

Both thresholds are 2 by default and both comparisons are inclusive. An opening bracket reads the
depth outside it and a closing bracket the depth it returns to, so a token's depth is the count
of brackets that hold it, not the brackets it opens or closes. Only brackets that pair count: an
unmatched one is a character in the text.

## Reading the axis

The summary gives the token count, the deepest nesting, the number of peaks and fractures, and
the token of greatest load:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex stress --text 'call(a, f(g(h(x))), [1, {k: v}]) ; done'
trex stress: 39 bytes, 27 tokens, max depth 4, 6 peak(s), 2 fracture(s)
  peak load: 29 (depth 3, strain 3)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::stress::analyze_bytes(b"call(a, f(g(h(x))), [1, {k: v}]) ; done");
assert_eq!((field.n_tokens, field.max_depth), (27, 4));
assert_eq!((field.peaks.len(), field.fractures.len()), (6, 2));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex.axes
>>> r = trex.axes.stress("call(a, f(g(h(x))), [1, {k: v}]) ; done")
>>> r.tokens, r.max_depth, r.peak_load.text, r.peak_load.offset, r.peak_load.load
(27, 4, 'v', 28, 29.0)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $line = 'call(a, f(g(h(x))), [1, {k: v}]) ; done'
PS> Measure-TrexStress $line | Select-Object Tokens, MaxDepth, PeakLoad | Format-List

Tokens   : 27
MaxDepth : 4
PeakLoad : v at 28, depth 3
```
{{< /tab >}}
{{< /tabs >}}

### Every token's reading

`--field` lists each token's depth `d`, strain and load, and `--limit N` stops the list after `N`
tokens and counts the rest. In PowerShell `-Detail` and in Python `detail=True` add the same
frames:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex stress --field --text 'call(a, f(g(h(x))), [1, {k: v}]) ; done'
trex stress: 39 bytes, 27 tokens, max depth 4, 6 peak(s), 2 fracture(s)
  peak load: 29 (depth 3, strain 3)
  per-token (token -> depth | strain | load):
    call           d=  0 strain=    0 load=     0
    (              d=  0 strain=    0 load=     0
    a              d=  1 strain=    1 load=     1
    ,              d=  1 strain=    2 load=     2
    f              d=  1 strain=    3 load=     3
    (              d=  1 strain=    4 load=     4
    g              d=  2 strain=    1 load=     6
    (              d=  2 strain=    2 load=     8
    h              d=  3 strain=    1 load=    11
    (              d=  3 strain=    2 load=    14
    x              d=  4 strain=    1 load=    18
    )              d=  3 strain=    4 load=    20
    )              d=  2 strain=    7 load=    18
    )              d=  1 strain=   12 load=    12
    ,              d=  1 strain=   13 load=    13
    [              d=  1 strain=   14 load=    14
    1              d=  2 strain=    1 load=    16
    ,              d=  2 strain=    2 load=    18
    {              d=  2 strain=    3 load=    20
    k              d=  3 strain=    1 load=    23
    :              d=  3 strain=    2 load=    26
    v              d=  3 strain=    3 load=    29
    }              d=  2 strain=    7 load=    28
    ]              d=  1 strain=   22 load=    22
    )              d=  0 strain=    0 load=     0
    ;              d=  0 strain=    0 load=     0
    done           d=  0 strain=    0 load=     0
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"call(a, f(g(h(x))), [1, {k: v}]) ; done";
let field = trex::stress::analyze_bytes(text);
let deepest = field.frames.iter().position(|f| f.depth == 4).expect("a token four deep");
let (s, e) = field.spans[deepest];
assert_eq!(&text[s..e], b"x");
assert_eq!((field.frames[deepest].strain, field.frames[deepest].load), (1.0, 18.0));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> r = trex.axes.stress("call(a, f(g(h(x))), [1, {k: v}]) ; done", detail=True)
>>> [(f.text, f.depth, f.strain, f.load) for f in r.frames if f.depth == 4]
[('x', 4, 1.0, 18.0)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexStress $line -Detail).Frames | Where-Object Depth -eq 4 | Format-Table Offset, Text, Depth, Strain, Load

Offset Text Depth Strain  Load
------ ---- ----- ------  ----
    14 x        4   1.00 18.00
```
{{< /tab >}}
{{< /tabs >}}

Strain and load differ where a collection holds several items. Inside `[1, {k: v}]` the innermost
bracket changes from `[` to `{`, so strain restarts at `k`, while load keeps counting every
bracket still open: the closing `]` reads strain 22, the length of time `[` was held.

### Peaks

A peak is the top of a rise: a token at least as deep as both its neighbors and deeper than one
of them. Every step of a staircase of brackets rises to a new level, so `g`, `h` and `x` are
each a peak as `f(g(h(x)))` goes in:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex stress --peaks --text 'call(a, f(g(h(x))), [1, {k: v}]) ; done'
trex stress: 39 bytes, 27 tokens, max depth 4, 6 peak(s), 2 fracture(s)
  peak load: 29 (depth 3, strain 3)
  stress peaks (6):
    @    10  g(h(x))), [1, {k: v}]) ;
    @    12  h(x))), [1, {k: v}]) ; d
    @    14  x))), [1, {k: v}]) ; don
    @    21  1, {k: v}]) ; done
    @    25  k: v}]) ; done
    @    28  v}]) ; done
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::stress::analyze_bytes(b"call(a, f(g(h(x))), [1, {k: v}]) ; done");
assert_eq!(field.peaks, [10, 12, 14, 21, 25, 28]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, f.offset, f.depth) for f in r.peaks]
[('g', 10, 2), ('h', 12, 3), ('x', 14, 4), ('1', 21, 2), ('k', 25, 3), ('v', 28, 3)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexStress $line).Peaks | Format-Table Offset, Text, Depth

Offset Text Depth
------ ---- -----
    10 g        2
    12 h        3
    14 x        4
    21 1        2
    25 k        3
    28 v        3
```
{{< /tab >}}
{{< /tabs >}}

### Fractures

A fracture is where a run of closing brackets starts: the first token whose depth falls, from a
level at least the fracture depth. One fracture marks one release, however many brackets close
in it, so `)))` after `x` is a single fracture at its first `)`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex stress --fractures --text 'call(a, f(g(h(x))), [1, {k: v}]) ; done'
trex stress: 39 bytes, 27 tokens, max depth 4, 6 peak(s), 2 fracture(s)
  peak load: 29 (depth 3, strain 3)
  fractures (2):
    @    15  ))), [1, {k: v}]) ; done
    @    29  }]) ; done
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let field = trex::stress::analyze_bytes(b"call(a, f(g(h(x))), [1, {k: v}]) ; done");
assert_eq!(field.fractures, [15, 29]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(f.text, f.offset) for f in r.fractures]
[(')', 15), ('}', 29)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexStress $line).Fractures | Format-Table Offset, Text, Depth

Offset Text Depth
------ ---- -----
    15 )        3
    29 }        2
```
{{< /tab >}}
{{< /tabs >}}

## Choosing a threshold

Both cuts on this axis are depths, so read `--field` and choose the depth that separates the
nesting you care about from the nesting every line has.

**The peak depth.** At the default of 2 every collection inside a call peaks, `1` in `[1, ...]`
among them. To keep only the deep structure, raise it to the depth the ordinary nesting never
reaches. Here 3 drops the peaks at depth 2 and keeps the rise into `h(x)` and into `{k: v}`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex stress --peaks --peak-min-depth 3 --text 'call(a, f(g(h(x))), [1, {k: v}]) ; done'
trex stress: 39 bytes, 27 tokens, max depth 4, 4 peak(s), 2 fracture(s)
  peak load: 29 (depth 3, strain 3)
  stress peaks (4):
    @    12  h(x))), [1, {k: v}]) ; d
    @    14  x))), [1, {k: v}]) ; don
    @    25  k: v}]) ; done
    @    28  v}]) ; done
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::stress::{StressConfig, analyze_with};

let text = b"call(a, f(g(h(x))), [1, {k: v}]) ; done";
let cfg = StressConfig { peak_min_depth: 3, ..StressConfig::default() };
assert_eq!(analyze_with(&trex::tokutil::lex_sig(text), text, &cfg).peaks, [12, 14, 25, 28]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [f.text for f in trex.axes.stress("call(a, f(g(h(x))), [1, {k: v}]) ; done", peak_min_depth=3).peaks]
['h', 'x', 'k', 'v']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexStress $line -PeakMinDepth 3).Peaks.Text
h
x
k
v
```
{{< /tab >}}
{{< /tabs >}}

**The fracture depth.** A fracture's depth is the level the release starts from, the depth before
its first closing bracket. Raising it to 4 keeps only the release from inside `h(x)`, and drops
the closing of `{k: v}`, which starts from depth 3:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex stress --fractures --fracture-min-depth 4 --text 'call(a, f(g(h(x))), [1, {k: v}]) ; done'
trex stress: 39 bytes, 27 tokens, max depth 4, 6 peak(s), 1 fracture(s)
  peak load: 29 (depth 3, strain 3)
  fractures (1):
    @    15  ))), [1, {k: v}]) ; done
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::stress::{StressConfig, analyze_with};

let text = b"call(a, f(g(h(x))), [1, {k: v}]) ; done";
let cfg = StressConfig { fracture_min_depth: 4, ..StressConfig::default() };
assert_eq!(analyze_with(&trex::tokutil::lex_sig(text), text, &cfg).fractures, [15]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [f.offset for f in trex.axes.stress("call(a, f(g(h(x))), [1, {k: v}]) ; done", fracture_min_depth=4).fractures]
[15]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Measure-TrexStress $line -FractureMinDepth 4).Fractures.Offset
15
```
{{< /tab >}}
{{< /tabs >}}

**A pattern's depth.** `@nested>k` reads the same depth, so choose `k` from the field as you
would the peak depth: the depth your ordinary lines reach is the one to stay above.

## Declarations

Declarations decide which tokens a bracket holds, so a declared shape that takes in a bracket
removes it from the depth count. An index such as `[1]` declared as one token is no longer a
bracket around `1`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex stress --text 'x = a[1] + b[2]'
trex stress: 15 bytes, 11 tokens, max depth 1, 0 peak(s), 0 fracture(s)
  peak load: 1 (depth 1, strain 1)

$ trex stress --shape 'index = `\[[0-9]\]`' --text 'x = a[1] + b[2]'
trex stress: 15 bytes, 7 tokens, max depth 0, 0 peak(s), 0 fracture(s)
  peak load: 0 (depth 0, strain 0)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let text = b"x = a[1] + b[2]";
let mut shapes = trex::ShapeSet::new();
shapes.declare("index = `\\[[0-9]\\]`", trex::Precedence::Before).expect("a bounded shape");
let toks: Vec<trex::token::Token> = trex::lexer::lex_with_shapes(text, &trex::lexer::blob_runs(text), &shapes, 0)
    .into_iter()
    .filter(|t| t.is_significant())
    .collect();
assert_eq!(trex::stress::analyze(&toks, text).max_depth, 0);
assert_eq!(trex::stress::analyze_bytes(text).max_depth, 1);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> lib = trex.Library()
>>> lib.shape("index", r"\[[0-9]\]")
>>> trex.axes.stress("x = a[1] + b[2]").max_depth, trex.axes.stress("x = a[1] + b[2]", lib=lib).max_depth
(1, 0)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $lib = New-TrexLibrary
PS> Register-TrexAtom index -Shape '\[[0-9]\]' -Library $lib
PS> (Measure-TrexStress 'x = a[1] + b[2]' -Library $lib).MaxDepth
0
```
{{< /tab >}}
{{< /tabs >}}

Every axis that reads tokens takes the declarations a scan takes: `--lib`, `--shape`,
`--shape-after`, `--kind`, `--let` and `--declare` on the command line, `-Library` in PowerShell
and `lib=` in Python ([pattern files](../../pattern-files/)).

## In a pattern

`@nested>k` and `@nested>=k` are zero-width anchors on tokens more than, or at least, `k`
brackets deep. With `\W` after it the anchor takes the deep names; with `.` after it, every deep
token, brackets included:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@nested>2 \W' --text 'call(a, f(g(h(x))), [1, {k: v}]) ; done'
[12..13] "h"
[14..15] "x"
[25..26] "k"
[28..29] "v"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"@nested>2 \W").expect("valid pattern");
let text = "call(a, f(g(h(x))), [1, {k: v}]) ; done";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["h", "x", "k", "v"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"@nested>2 \W").scan("call(a, f(g(h(x))), [1, {k: v}]) ; done")]
['h', 'x', 'k', 'v']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@nested>2 \W' -InputObject $line -Raw
h
x
k
v
```
{{< /tab >}}
{{< /tabs >}}

## Data model

`StressFrame`, one token's reading:

| Field | Type | Meaning |
|---|---|---|
| `depth` | `u16` | enclosing-bracket nesting depth |
| `strain` | `f32` | stretch of the innermost open bracket |
| `load` | `f32` | total outstanding tension, the sum of the open-bracket stretches |

`StressField`, keyed by byte offset:

| Field | Type | Meaning |
|---|---|---|
| `n_tokens` | `usize` | token count |
| `spans` | `Vec<(usize, usize)>` | byte span per token |
| `frames` | `Vec<StressFrame>` | one per token |
| `peaks` | `Vec<usize>` | stress-peak byte offsets |
| `fractures` | `Vec<usize>` | fracture byte offsets |
| `max_depth` | `u16` | deepest nesting reached |

Query API: `depth_at(byte)`, `load_at(byte)`. `StressConfig` holds `peak_min_depth` and
`fracture_min_depth`, both 2; `analyze_with(tokens, bytes, &cfg)` reads under it.

## Algorithm

The open-bracket stack of token indices is kept from `TokenKind::Open` and `Close`. At each
token a close pops first, so it reads the outer depth and is the release point; `depth =
stack.len()`; `strain = now - innermost_open`; `load = sum(now - open)` over the stack. A
fracture is the first decreasing-depth token of a cascade whose prior depth reached
`fracture_min_depth`. Peaks are a second linear pass over the depth sequence: a token at least
`peak_min_depth` deep, at least as deep as both neighbors and deeper than one. One pass and one
peak scan; O(depth) per token for load.
