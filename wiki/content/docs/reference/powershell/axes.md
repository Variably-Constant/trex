---
title: Property axes
linkTitle: Property axes
weight: 70
---

Each `Measure-Trex` cmdlet reads one [property axis](../../axes/) over text or files and writes
a report per input: counts and summary readings as properties, and the frames that stand out as
arrays; `-Detail` adds a frame at every position. The examples are on each axis's page.

### Measure-TrexMagnitude

Reads the magnitude axis: each token's order of magnitude, its change from the token before, and the energy of the window behind it.

A number's magnitude is log10 of its value and any other token's log2 of its length. Jumps are the tokens where it changes by at least -JumpThreshold orders from the token before, three by default; outliers are the tokens at least -OutlierSigma standard deviations from the input's mean, two and a half by default.

Alias: `Measure-TxMagnitude`

```powershell
Measure-TrexMagnitude [-InputObject] <string> [-Detail] [-JumpThreshold <float>] [-OutlierSigma <float>] [-Library <Library>] [<CommonParameters>]
Measure-TrexMagnitude -Path <string[]> [-Detail] [-JumpThreshold <float>] [-OutlierSigma <float>] [-Library <Library>] [<CommonParameters>]
Measure-TrexMagnitude -LiteralPath <string[]> [-Detail] [-JumpThreshold <float>] [-OutlierSigma <float>] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every token's frame to the report. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-JumpThreshold` | float |  | The change in orders of magnitude from the token before that makes a jump: three when absent. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-OutlierSigma` | float |  | The standard deviations from the input's mean that make an outlier: two and a half when absent. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.MagnitudeReport](../types/#trexmagnitudereport).

### Measure-TrexStress

Reads the stress axis: how deeply the paired brackets nest at each token, how long the innermost has been held open, and how long all of them together.

Peaks are the local maxima of depth at least -PeakMinDepth deep, and fractures the tokens where the depth starts to fall from a level at least -FractureMinDepth deep, both two by default. An unpaired bracket is a character of the text and opens no level.

Alias: `Measure-TxStress`

```powershell
Measure-TrexStress [-InputObject] <string> [-Detail] [-PeakMinDepth <uint>] [-FractureMinDepth <uint>] [-Library <Library>] [<CommonParameters>]
Measure-TrexStress -Path <string[]> [-Detail] [-PeakMinDepth <uint>] [-FractureMinDepth <uint>] [-Library <Library>] [<CommonParameters>]
Measure-TrexStress -LiteralPath <string[]> [-Detail] [-PeakMinDepth <uint>] [-FractureMinDepth <uint>] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every token's frame to the report. |
| `-FractureMinDepth` | uint |  | The depth a fall must start from to be a fracture: two when absent. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-PeakMinDepth` | uint |  | The depth a local maximum must reach to be a peak: two when absent. |

Writes [Trex.StressReport](../types/#trexstressreport).

### Measure-TrexFlow

Reads the flow axis: the trend of a per-token signal, the magnitude by default, how long it has run one way, and where it reverses.

-Signal Stress reads the trend of the nesting depth instead, and -Signal Length that of the token lengths. -Grain Super reads it from supertoken to supertoken. -Analytic adds the signal's position in its swing: its envelope, its phase and how fast the swing runs.

Alias: `Measure-TxFlow`

```powershell
Measure-TrexFlow [-InputObject] <string> [-Signal <FlowSignal>] [-Grain <Grain>] [-Detail] [-Analytic] [-Window <uint>] [-SteadyBand <float>] [-Library <Library>] [<CommonParameters>]
Measure-TrexFlow -Path <string[]> [-Signal <FlowSignal>] [-Grain <Grain>] [-Detail] [-Analytic] [-Window <uint>] [-SteadyBand <float>] [-Library <Library>] [<CommonParameters>]
Measure-TrexFlow -LiteralPath <string[]> [-Signal <FlowSignal>] [-Grain <Grain>] [-Detail] [-Analytic] [-Window <uint>] [-SteadyBand <float>] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Analytic` | switch |  | Adds every unit's analytic reading and the latency it carries. |
| `-Detail` | switch |  | Adds every unit's frame to the report. |
| `-Grain` | [Trex.Grain](../types/#trexgrain) |  | The stream the trend is read over: Token when absent, or Super. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-Signal` | [Trex.FlowSignal](../types/#trexflowsignal) |  | The signal whose trend is read: Magnitude when absent. |
| `-SteadyBand` | float |  | How far from zero a slope reads as steady: 0.05 when absent. |
| `-Window` | uint |  | How many units the slope is averaged over: four when absent. |

Writes [Trex.FlowReport](../types/#trexflowreport).

### Measure-TrexObservation

Reads the observation axis: the entropy of the classes behind each unit, ahead of it and around it, and the points where behind and ahead differ most.

At the byte grain the entropies are of five byte classes, digits, letters, whitespace, other characters and bytes that are not part of a well-formed UTF-8 character, so they read the texture of the text and not its words; a letter outside ASCII is a letter, and any other character outside ASCII counts once. -Grain Token reads the sequence of token kinds instead, and -Grain Super the sequence of supertoken roles, each lexed with the atoms in force, the session's or -Library's; the byte reading takes no atoms.

Alias: `Measure-TxObservation`

```powershell
Measure-TrexObservation [-InputObject] <string> [-Grain <Grain>] [-Detail] [-ContestedThreshold <float>] [-ContestedMinGap <uint>] [-Library <Library>] [<CommonParameters>]
Measure-TrexObservation -Path <string[]> [-Grain <Grain>] [-Detail] [-ContestedThreshold <float>] [-ContestedMinGap <uint>] [-Library <Library>] [<CommonParameters>]
Measure-TrexObservation -LiteralPath <string[]> [-Grain <Grain>] [-Detail] [-ContestedThreshold <float>] [-ContestedMinGap <uint>] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-ContestedMinGap` | uint |  | The bytes between two contested points at the byte grain: 8 when absent. |
| `-ContestedThreshold` | float |  | The disagreement a contested point must reach, from 0 to 1: 0.2 when absent. |
| `-Detail` | switch |  | Adds every unit's frame to the report. |
| `-Grain` | [Trex.Grain](../types/#trexgrain) |  | The stream read: Byte when absent, Token or Super. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex at the Token and Super grains, in place of the session's; the byte grain reads bytes and takes none. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.ObservationReport](../types/#trexobservationreport).

### Measure-TrexEcho

Reads the echo axis: which words, numbers and literals recur, how many times, and whether they recur at a regular distance.

-Group reads the keys under a symmetry, so under `case` `Error` and `ERROR` are one key, and under `subnet/24` so are all the addresses of one network. -Structure adds the structures that recur among supertokens, so `alpha: one` and `bravo: two` are one structure, a key beside a value. A key recurring three or more times has a period where the spread of its distances is at most -MaxPeriodCv of their mean, 0.3 by default.

Alias: `Measure-TxEcho`

```powershell
Measure-TrexEcho [-InputObject] <string> [-Group <string>] [-Structure] [-Detail] [-MaxPeriodCv <float>] [-Library <Library>] [<CommonParameters>]
Measure-TrexEcho -Path <string[]> [-Group <string>] [-Structure] [-Detail] [-MaxPeriodCv <float>] [-Library <Library>] [<CommonParameters>]
Measure-TrexEcho -LiteralPath <string[]> [-Group <string>] [-Structure] [-Detail] [-MaxPeriodCv <float>] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every keyed token's frame to the report. |
| `-Group` | string |  | The symmetry the keys are read under: `identity`, exact text, when absent. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-MaxPeriodCv` | float |  | How widely a key's distances may spread, as a fraction of their mean, for it to have a period: 0.3 when absent. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-Structure` | switch |  | Adds the structures that recur among supertokens. |

Writes [Trex.EchoReport](../types/#trexechoreport).

### Measure-TrexOrbit

Reads the orbit axis: each token as the representative of its orbit under a symmetry, and how far the symmetry folds the input's vocabulary.

Under `shape`, the default, `cat`, `dog` and `bat` are one orbit, a consonant, a vowel and a consonant; under `case`, so are `Error` and `ERROR`, and under `numeric`, `1e3`, `0x3e8` and `1_000`. -SameAs lists the tokens in the same orbit as a text: every token that equals it up to the group.

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
| `-SameAs` | string |  | A text: the report lists every token in its orbit, each equal to it up to the group. |

Writes [Trex.OrbitReport](../types/#trexorbitreport).

### Measure-TrexShape

Reads the shape axis: each token's shape, the period at which the shapes repeat, and the regions that repeat like a table or a list of records.

A table reads as a region whether or not its columns line up, since the period is of token shapes and not of bytes. -Group reads each token's shape under a symmetry. A region is a run whose period repeats with a strength of at least -TemplateStrength, 0.6 by default.

Alias: `Measure-TxShape`

```powershell
Measure-TrexShape [-InputObject] <string> [-Group <string>] [-Detail] [-TemplateStrength <float>] [-Library <Library>] [<CommonParameters>]
Measure-TrexShape -Path <string[]> [-Group <string>] [-Detail] [-TemplateStrength <float>] [-Library <Library>] [<CommonParameters>]
Measure-TrexShape -LiteralPath <string[]> [-Group <string>] [-Detail] [-TemplateStrength <float>] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every token's frame to the report. |
| `-Group` | string |  | The symmetry each token's shape is read under; the lexer's own shape when absent. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-TemplateStrength` | float |  | How strongly a run's period must repeat for it to be a region, from 0 to 1: 0.6 when absent. |

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

-Classify adds the regions read by texture and by token shape together, so a table whose columns do not line up still reads as a table. A change point is where the byte statistics move past their running mean by more than -CpThreshold mean deviations, 4 by default, or -CpFloor of the mean, 0.05 by default, at least -CpMinGap bytes after the last, 4 by default. The axis reads bytes, so it takes no atoms.

Alias: `Measure-TxSpectral`

```powershell
Measure-TrexSpectral [-InputObject] <string> [-Classify] [-Detail] [-CpThreshold <float>] [-CpFloor <float>] [-CpMinGap <uint>] [<CommonParameters>]
Measure-TrexSpectral -Path <string[]> [-Classify] [-Detail] [-CpThreshold <float>] [-CpFloor <float>] [-CpMinGap <uint>] [<CommonParameters>]
Measure-TrexSpectral -LiteralPath <string[]> [-Classify] [-Detail] [-CpThreshold <float>] [-CpFloor <float>] [-CpMinGap <uint>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Classify` | switch |  | Adds the regions read by texture and token shape together. |
| `-CpFloor` | float |  | The least move, as a fraction of the running mean, that makes a change point: 0.05 when absent. |
| `-CpMinGap` | uint |  | The bytes between two change points: 4 when absent. |
| `-CpThreshold` | float |  | How many mean deviations past the running mean make a change point: 4 when absent. |
| `-Detail` | switch |  | Adds every frame to the report. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.SpectralReport](../types/#trexspectralreport).

### Measure-TrexSeam

Reads the seam axis: where an input divides into units, found where the text before a point stops predicting the text after it, read in both directions, with no dictionary.

-English reads with a model of English letter sequences, so even a short string divides where English does not join its letters, and a unit may be cut inside a token the lexer read. A cut needs a seam strength at least -CutThreshold standard deviations above the input's mean, 0.6 by default. The axis reads bytes, so it takes no atoms.

Alias: `Measure-TxSeam`

```powershell
Measure-TrexSeam [-InputObject] <string> [-Order <uint>] [-Passes <uint>] [-CutThreshold <float>] [-English] [-Detail] [<CommonParameters>]
Measure-TrexSeam -Path <string[]> [-Order <uint>] [-Passes <uint>] [-CutThreshold <float>] [-English] [-Detail] [<CommonParameters>]
Measure-TrexSeam -LiteralPath <string[]> [-Order <uint>] [-Passes <uint>] [-CutThreshold <float>] [-English] [-Detail] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-CutThreshold` | float |  | How many standard deviations above the input's mean seam strength a cut needs: 0.6 when absent. |
| `-Detail` | switch |  | Adds every byte's frame to the report. |
| `-English` | switch |  | Reads with a model of English letter sequences. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Order` | uint |  | The longest context the entropy is read over, in bytes: 3 when absent. |
| `-Passes` | uint |  | How many confidence passes to run: one when absent. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |

Writes [Trex.SeamReport](../types/#trexseamreport).

### Measure-TrexGravity

Reads the pair field: how much more or less often each type of unit follows another at each gap than chance alone would, and three readings taken from it.

A unit's Strain is its mean potential against the units before it, high where its past pushes it away; the Bound of the cut before it is the attraction across that cut, low where the input holds together least; and its Class groups the types the field treats alike. -Grain Token reads the significant tokens, Byte the bytes and Super the supertokens, the token and super grains lexed with the atoms in force, the session's or -Library's; the byte grain takes no atoms. -Top sizes the lists of the most strained units and the weakest cuts.

Alias: `Measure-TxGravity`

```powershell
Measure-TrexGravity [-InputObject] <string> [-Grain <Grain>] [-Top <uint>] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexGravity -Path <string[]> [-Grain <Grain>] [-Top <uint>] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexGravity -LiteralPath <string[]> [-Grain <Grain>] [-Top <uint>] [-Detail] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every unit's frame to the report. |
| `-Grain` | [Trex.Grain](../types/#trexgrain) |  | The stream read: Token when absent, Byte or Super. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex at the Token and Super grains, in place of the session's; the byte grain reads bytes and takes none. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-Top` | uint |  | How many of the most strained units and the weakest cuts the report lists: 8 when absent. |

Writes [Trex.GravityReport](../types/#trexgravityreport).

### Measure-TrexContext

Reads the context axis: each token read against what surrounds it.

-Fold names the context as `\N{>+1:F}` names it: Window, the tokens before it; Unit, the supertoken holding it; Units, the window of supertokens ending there; Enclosing, the heads of the brackets around it; Echo, its key's earlier occurrences; Regime, the tokens since the texture last changed; Phase, the earlier tokens at its column of the record period; Key, the values its key was bound to before. Every frame carries each axis's reading over the tokens its context folds. -TokenWindow and -UnitWindow set the windows, 32 tokens and 8 supertokens when absent, and the lex reads the atoms in force, the session's or -Library's.

Alias: `Measure-TxContext`

```powershell
Measure-TrexContext [-InputObject] <string> [-Fold <ContextFold>] [-TokenWindow <uint>] [-UnitWindow <uint>] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexContext -Path <string[]> [-Fold <ContextFold>] [-TokenWindow <uint>] [-UnitWindow <uint>] [-Detail] [-Library <Library>] [<CommonParameters>]
Measure-TrexContext -LiteralPath <string[]> [-Fold <ContextFold>] [-TokenWindow <uint>] [-UnitWindow <uint>] [-Detail] [-Library <Library>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Detail` | switch |  | Adds every token's context and every supertoken's agreement to the report. |
| `-Fold` | [Trex.ContextFold](../types/#trexcontextfold) |  | The context read: Window when absent. |
| `-InputObject` | string | by value | The text to read; each string piped in is read on its own. |
| `-Library` | [Trex.Library](../types/#trexlibrary) |  | The atoms that decide the lex, in place of the session's. |
| `-LiteralPath` | string[] | by name | Files or directories to read, as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files or directories to read; wildcards expand. |
| `-TokenWindow` | uint |  | The significant tokens a window folds: 32 when absent. The window before a token is the one ending at the token before it. |
| `-UnitWindow` | uint |  | The supertokens the unit window folds: 8 when absent. |

Writes [Trex.ContextReport](../types/#trexcontextreport).
