---
title: Property axes
linkTitle: Property axes
weight: 70
---

# Property axes

Each `Measure-Trex` cmdlet reads one [property axis](../../axes/) over text or files and writes
a report per input: counts and summary readings as properties, and the frames that stand out as
arrays. A frame shows as its text and where it stands; select an array to read every field of
its frames, and add `-Detail` for a frame at every token. The examples read this file:

```powershell
PS> Get-Content ./people.csv
name,age,score
alice,30,95
bob,25,88
carol,41,73
dan,38,91
erin,29,84
frank,52,67
grace,33,99
heidi,47,78
ivan,26,90
judy,35,82
```

## Magnitude

The order of magnitude of each token, its change from the token before, and the energy of the
window behind it; see [magnitude](../../axes/magnitude/):

```powershell
PS> Measure-TrexMagnitude '1 3 2 1000000 2 3'

Path        :
Length      : 17
Tokens      : 6
TotalEnergy : 36.63653
Peak        : 1000000 at 6
Jumps       : {1000000 at 6, 2 at 14}
Outliers    : {}
Frames      : {}

PS> (Measure-TrexMagnitude '1 3 2 1000000 2 3').Jumps | Select-Object Offset, Text, Magnitude, Gradient

Offset Text    Magnitude Gradient
------ ----    --------- --------
     6 1000000      6.00     5.70
    14 2            0.30    -5.70
```

## Stress

How deeply the paired brackets nest at each token, and how long they have been held open; see
[stress](../../axes/stress/):

```powershell
PS> Measure-TrexStress '{"a": [1, {"b": [2, 3]}]}'

Path      :
Length    : 25
Tokens    : 17
MaxDepth  : 4
PeakLoad  : ] at 21, depth 3
Peaks     : {1 at 7, depth 2, "b" at 11, depth 3, 2 at 17, depth 4, 3 at 20, depth 4}
Fractures : {] at 21, depth 3}
Frames    : {}

PS> (Measure-TrexStress '{"a": [1, {"b": [2, 3]}]}').Peaks | Select-Object Offset, Text, Depth, Strain, Load

Offset : 7
Text   : 1
Depth  : 2
Strain : 1
Load   : 5

Offset : 11
Text   : "b"
Depth  : 3
Strain : 1
Load   : 12

Offset : 17
Text   : 2
Depth  : 4
Strain : 1
Load   : 22

Offset : 20
Text   : 3
Depth  : 4
Strain : 3
Load   : 30
```

## Flow

The trend of a per-token signal, the magnitude unless `-Signal` names `Stress` or `Length`, how
long it has run one way, and where it reverses; see [flow](../../axes/flow/):

```powershell
PS> Measure-TrexFlow '1 10 100 1000 10 1'

Path         :
Length       : 18
Tokens       : 6
Signal       : Magnitude
PeakMomentum : 10 at 14, Rising
Reversals    : {1 at 17, Falling}
Frames       : {}
```

## Observation

The entropy of the byte classes behind each byte, ahead of it and around it, and where behind and
ahead disagree most; see [observation](../../axes/observation/):

```powershell
PS> Measure-TrexObservation 'alpha beta gamma' | Select-Object Length, Peak, Contested

Length Peak                          Contested
------ ----                          ---------
    16 at 4, disagreement 0.29459932 {at 4, disagreement 0.29459932}
```

## Echo

Which words, numbers and literals recur, how often, and whether at a regular distance; see
[echo](../../axes/echo/):

```powershell
PS> Measure-TrexEcho 'alpha beta alpha gamma alpha'

Path       :
Length     : 28
Group      : identity
Tokens     : 9
Keyed      : 5
Distinct   : 3
Novel      : 3
Echoed     : 3
Novelty    : 0.6
EchoRate   : 0.6
Echoes     : {alpha (3)}
Structures : {}
Frames     : {}

PS> (Measure-TrexEcho 'alpha beta alpha gamma alpha').Echoes

Key   Count Period Offset
---   ----- ------ ------
alpha 3     11.5   0
```

## Orbit

Each token as the representative of its orbit under a symmetry, and how far the symmetry folds
the vocabulary; `-SameAs` lists the tokens in one text's orbit. See [orbit](../../axes/orbit/)
for the symmetries `-Group` names:

```powershell
PS> Measure-TrexOrbit 'Cat cat CAT dog' -Group case -SameAs cat

Path     :
Length   : 15
Group    : case
Tokens   : 4
Forms    : 4
Orbits   : 2
Classes  : {cat, dog}
Matches  : {Cat at 0, cat at 4, CAT at 8}
Segments : {}
Frames   : {}

PS> (Measure-TrexOrbit 'Cat cat CAT dog' -Group case).Classes

Orbit Forms
----- -----
cat   {CAT, Cat, cat}
dog   {dog}
```

## Shape

Each token's shape, the period at which the shapes repeat, and the regions that repeat like a
table; see [shape](../../axes/shape/):

```powershell
PS> Measure-TrexShape -Path ./people.csv | Select-Object Tokens, DominantPeriod, DominantStrength, Regions, ChangePoints

Tokens           : 55
DominantPeriod   : 25
DominantStrength : 0.9
Regions          : {at 56, 71 long, period 10}
ChangePoints     : {4, 15, 24, 37}
```

## Relation

The graph an input's brackets, binding punctuation, adjacency and repeated content make, and
what its tree and its loops measure; `-Detail` adds every edge:

```powershell
PS> Measure-TrexRelation 'f(a, [b])' | Select-Object Tokens, MaxDepth, Encloses, Adjacencies, Components, CycleRank

Tokens      : 9
MaxDepth    : 2
Encloses    : 3
Adjacencies : 7
Components  : 1
CycleRank   : 3

PS> (Measure-TrexRelation 'f(a, [b])' -Detail).Edges | Select-Object -First 4

Kind     From To FromOffset ToOffset
----     ---- -- ---------- --------
Encloses f    a  0          2
Encloses f    ,  0          3
Encloses f    b  0          6
Adjacent f    (  0          1
```

## Spectral

The entropy, byte period, novelty and class mix of an input's bytes along its length, where they
change, and the texture of each stretch; `-Classify` adds the regions read by texture and token
shape together. See [spectral](../../axes/spectral/):

```powershell
PS> Measure-TrexSpectral -Path ./people.csv -Classify | Select-Object Length, Hop, EntropyMean, Timeline, Regions

Length      : 128
Hop         : 16
EntropyMean : 0.6798461
Timeline    : {Math at 0, 128 long}
Regions     : {Table at 0, 128 long}
```

## Seam

Where an input divides into units, found where the text before a point stops predicting the text
after it; `-English` reads with a model of English letter sequences. See
[seam](../../axes/seam/). The command line's research instruments, `seam --recover`,
`--compare-bpe` and the `--compress` coder options, are the command line's alone:

```powershell
PS> (Measure-TrexSeam 'thequickbrownfoxjumpsoverthelazydog' -English).Segments -join ' '
the quick brown fox jumps over the lazy dog
```

## Cmdlets

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
