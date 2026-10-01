---
title: Property axes
linkTitle: Property axes
weight: 70
---

# Property axes

Each `Measure-Trex` cmdlet reads one [property axis](../../axes/) over text or files and writes
a report per input: counts and summary readings as properties, and the frames that stand out as
arrays; `-Detail` adds a frame at every position. The examples are on each axis's page.

### Measure-TrexMagnitude

Reads the magnitude axis: each token's order of magnitude, its change from the token before, and the energy of the window behind it.

A number's magnitude is log10 of its value and any other token's log2 of its length. Jumps are the tokens where it changes by more than three orders from the token before; outliers are the tokens more than two and a half standard deviations from the input's mean.

Alias: `Measure-TxMagnitude`

```powershell
Measure-TrexMagnitude [-InputObject] <string> [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexMagnitude -Path <string[]> [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexMagnitude -LiteralPath <string[]> [-Detail] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every token's frame to the report. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.MagnitudeReport](../types/#trexmagnitudereport).

### Measure-TrexStress

Reads the stress axis: how deeply the paired brackets nest at each token, how long the innermost has been held open, and how long all of them together.

Peaks are the local maxima of depth, and fractures the tokens where the depth starts to fall from a nested level. An unpaired bracket is a character of the text and opens no level.

Alias: `Measure-TxStress`

```powershell
Measure-TrexStress [-InputObject] <string> [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexStress -Path <string[]> [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexStress -LiteralPath <string[]> [-Detail] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every token's frame to the report. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.StressReport](../types/#trexstressreport).

### Measure-TrexFlow

Reads the flow axis: the trend of a per-token signal, the magnitude by default, how long it has run one way, and where it reverses.

-Signal Stress reads the trend of the nesting depth instead, and -Signal Length that of the token lengths.

Alias: `Measure-TxFlow`

```powershell
Measure-TrexFlow [-InputObject] <string> [-Signal <FlowSignal>] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexFlow -Path <string[]> [-Signal <FlowSignal>] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexFlow -LiteralPath <string[]> [-Signal <FlowSignal>] [-Detail] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every token's frame to the report. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-Signal` | [Trex.FlowSignal](../types/#trexflowsignal) |  | The signal whose trend is read: Magnitude when absent. |

Writes [Trex.FlowReport](../types/#trexflowreport).

### Measure-TrexObservation

Reads the observation axis: the entropy of the byte classes behind each byte, ahead of it and around it, and the points where behind and ahead differ most.

The entropies are of five byte classes, digits, letters, whitespace, other ASCII and bytes above ASCII, so they read the texture of the text and not its words. The axis reads bytes, so it takes no atoms.

Alias: `Measure-TxObservation`

```powershell
Measure-TrexObservation [-InputObject] <string> [-Detail] [<CommonParameters>]
Measure-TrexObservation -Path <string[]> [-Detail] [<CommonParameters>]
Measure-TrexObservation -LiteralPath <string[]> [-Detail] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every byte's frame to the report. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.ObservationReport](../types/#trexobservationreport).

### Measure-TrexEcho

Reads the echo axis: which words, numbers and literals recur, how many times, and whether they recur at a regular distance.

-Group reads the keys under a symmetry, so under `case` `Error` and `ERROR` are one key, and under `subnet/24` every address of a network is. -Structure adds the structures that recur among runs of tokens, so `alpha: one` and `bravo: two` are one structure, a key beside a value.

Alias: `Measure-TxEcho`

```powershell
Measure-TrexEcho [-InputObject] <string> [-Group <string>] [-Structure] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexEcho -Path <string[]> [-Group <string>] [-Structure] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexEcho -LiteralPath <string[]> [-Group <string>] [-Structure] [-Detail] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every keyed token's frame to the report. |
| `-Group` | string |  | The symmetry the keys are read under: `identity`, exact text, when absent. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-Structure` | switch |  | Adds the structures that recur among runs of tokens. |

Writes [Trex.EchoReport](../types/#trexechoreport).

### Measure-TrexOrbit

Reads the orbit axis: each token as the representative of its orbit under a symmetry, and how far the symmetry folds the input's vocabulary.

Under `shape`, the default, `cat`, `dog` and `bat` are one orbit, a consonant, a vowel and a consonant; under `case`, `Error` and `ERROR` are; under `numeric`, `1,000` and `1e3` are. -SameAs lists the tokens in the same orbit as a text: every token that equals it up to the group.

Alias: `Measure-TxOrbit`

```powershell
Measure-TrexOrbit [-InputObject] <string> [-Group <string>] [-SameAs <string>] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexOrbit -Path <string[]> [-Group <string>] [-SameAs <string>] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexOrbit -LiteralPath <string[]> [-Group <string>] [-SameAs <string>] [-Detail] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every token with its orbit and the runs of one kind of character. |
| `-Group` | string |  | The symmetry the tokens are read under: `shape` when absent. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-SameAs` | string |  | A text whose orbit the report lists the tokens of: every token equal to it up to the group. |

Writes [Trex.OrbitReport](../types/#trexorbitreport).

### Measure-TrexShape

Reads the shape axis: each token's shape, the period at which the shapes repeat, and the regions that repeat like a table or a list of records.

A table reads as a region whether or not its columns line up, since the period is of token shapes and not of bytes. -Group reads each token's shape under a symmetry.

Alias: `Measure-TxShape`

```powershell
Measure-TrexShape [-InputObject] <string> [-Group <string>] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexShape -Path <string[]> [-Group <string>] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexShape -LiteralPath <string[]> [-Group <string>] [-Detail] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every token's frame to the report. |
| `-Group` | string |  | The symmetry each token's shape is read under; the lexer's own shape when absent. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.ShapeReport](../types/#trexshapereport).

### Measure-TrexRelation

Reads the relation axis: the graph an input's brackets, binding punctuation, adjacency and repeated content make, and what that graph's tree and its loops measure.

A reuse joins a later occurrence of repeated content to the earlier, and its depth difference is its holonomy: a tree has none, and a name bound outside and used deeper has some. The curvature, topology, geodesic and entanglement readings are of the same graph. -Canonical adds the input's alpha-equivalence form, which reads alike for two texts that differ only in the names their brackets bind.

Alias: `Measure-TxRelation`

```powershell
Measure-TrexRelation [-InputObject] <string> [-Canonical] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexRelation -Path <string[]> [-Canonical] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexRelation -LiteralPath <string[]> [-Canonical] [-Detail] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Canonical` | switch |  | Adds the input's alpha-equivalence form. |
| `-Detail` | switch |  | Adds every edge, every reuse and every nested token's frame. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.RelationReport](../types/#trexrelationreport).

### Measure-TrexSpectral

Reads the spectral axis: the entropy, byte period, novelty and class mix of an input's bytes along its length, where they change, and the texture of each stretch between, prose, code, mathematics, data or mixed.

-Classify adds the regions read by texture and by token shape together, so a table whose columns do not line up still reads as a table. The axis reads bytes, so it takes no atoms.

Alias: `Measure-TxSpectral`

```powershell
Measure-TrexSpectral [-InputObject] <string> [-Classify] [-Detail] [<CommonParameters>]
Measure-TrexSpectral -Path <string[]> [-Classify] [-Detail] [<CommonParameters>]
Measure-TrexSpectral -LiteralPath <string[]> [-Classify] [-Detail] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Classify` | switch |  | Adds the regions read by texture and token shape together. |
| `-Detail` | switch |  | Adds every frame to the report. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.SpectralReport](../types/#trexspectralreport).

### Measure-TrexSeam

Reads the seam axis: where an input divides into units, found where the text before a point stops predicting the text after it, read in both directions, with no dictionary.

-English reads with a model of English letter sequences, so even a short string divides where English does not join its letters, and a unit may be cut inside a token the lexer read. The axis reads bytes, so it takes no atoms.

Alias: `Measure-TxSeam`

```powershell
Measure-TrexSeam [-InputObject] <string> [-Order <uint>] [-Passes <uint>] [-English] [-Detail] [<CommonParameters>]
Measure-TrexSeam -Path <string[]> [-Order <uint>] [-Passes <uint>] [-English] [-Detail] [<CommonParameters>]
Measure-TrexSeam -LiteralPath <string[]> [-Order <uint>] [-Passes <uint>] [-English] [-Detail] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every byte's frame to the report. |
| `-English` | switch |  | Reads with a model of English letter sequences. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Order` | uint |  | The longest context the entropy is read over, in bytes: 3 when absent. |
| `-Passes` | uint |  | How many confidence passes to run: one when absent. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.SeamReport](../types/#trexseamreport).
