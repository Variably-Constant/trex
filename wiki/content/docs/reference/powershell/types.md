---
title: Types
linkTitle: Types
weight: 90
---

Every class and enumeration the cmdlets write and take. A property's type is the .NET type the module declares; where that is `object`, what it holds is said beside it. A proxy class keeps its value in Rust behind the object: each property read is one call into it, `Dispose()` frees it, and `IsDisposed` says whether it has been.

## Classes

### Trex.AnalyticFrame

One unit's analytic reading: the signal's position in its swing.

Inside another object, and as a string, it shows as `{Text} at {Offset}, amplitude {Amplitude}`.

Held in [Trex.FlowReport](#trexflowreport).Analytic.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the unit starts, in UTF-16 code units. |
| `Text` | string | The unit's text. |
| `Amplitude` | float | The envelope of the signal's swing around its running mean. |
| `Phase` | float | The unit's position in the swing, in radians from -pi to pi. |
| `Frequency` | float | How fast the swing runs, in radians a unit: the phase's advance from the unit before. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}, amplitude {Amplitude}`, each braced name read from that property. |

### Trex.Atom

One declared atom, as `Get-TrexAtom` lists it.

Written by [Get-TrexAtom](../atoms/#get-trexatom), [Import-TrexAtom](../atoms/#import-trexatom), [Register-TrexAtom](../atoms/#register-trexatom).

| Property | Type | Holds |
|---|---|---|
| `Name` | string | The name a pattern reads the atom by, as `\{name}`. |
| `Form` | [Trex.AtomForm](#trexatomform) | What the declaration declares. |
| `Definition` | string | The byte-pattern or pattern the declaration gives. |
| `Source` | string | Where it was declared: `session`, `library`, the pattern file's path, or `shipped` for the library TREX carries. |
| `Description` | string | For a shipped atom, what it matches. |

### Trex.AtomTest

The outcome of one `test` line: an atom's expectations, run against what the set finally declares.

Written by [Test-TrexAtom](../atoms/#test-trexatom).

| Property | Type | Holds |
|---|---|---|
| `Name` | string | The atom the line tests. |
| `Passed` | bool | Whether every expectation on the line held. |
| `Accepts` | string[] | Texts the atom must match whole. |
| `Rejects` | string[] | Texts the atom must match nowhere. |
| `Failures` | string[] | What TREX reported for each expectation that failed. |
| `File` | string | The pattern file the line is in, or empty for one declared here. |

### Trex.Bpe

A byte-pair encoder: the merges training learned, in order. A proxy class.

Written by [Import-TrexBpe](../streams/#import-trexbpe), [New-TrexBpe](../streams/#new-trexbpe).

| Property | Type | Holds |
|---|---|---|
| `Merges` | long | How many merges the model holds. |
| `Model` | string | The model as text, one merge a line with its two symbols separated by a tab, as Export-TrexBpe writes it. |
| `IsDisposed` | bool | Whether `Dispose()` has freed the value behind the object; a property read after that is `$null`. |

| Method | Returns | Does |
|---|---|---|
| `[Trex.Bpe]::new(string model)` | [Trex.Bpe](#trexbpe) | Reads a model from its text, one merge a line with its two symbols separated by a tab. |
| `Encode(string text)` | string[] | `text` split into the model's subwords, word by word. |

### Trex.BuiltField

One field of a built pattern.

Inside another object, and as a string, it shows as `{Name}`.

Held in [Trex.BuiltPattern](#trexbuiltpattern).Fields.

| Property | Type | Holds |
|---|---|---|
| `Name` | string | The capture's name. |
| `Accessor` | object | The accessor a template writes after the capture where the field is part of one token, `host` of a URL; `$null` otherwise. |
| `TypeName` | object | The type a mark gave the field as the mark writes it, `int` for `{[int]age:6}`, which its values are cast to; `$null` where none. |
| `StartsRecord` | bool | Whether a mark writes the field `{name*:text}`, so a line holding it begins a record. |
| `List` | bool | Whether a line marks the field more than once, so its value is every text it holds, in order, as an array. |
| `Repeats` | bool | Whether the field is inside records a line repeats, so a row holds all its values as an array and each record holds one. |
| `Parent` | object | The name of the field whose mark holds this one's, whose value then holds this field's under its last name part; `$null` at the top. |
| `Template` | string | How a report template writes the field. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Name}`, each braced name read from that property. |

### Trex.BuiltPattern

A pattern built for named fields, with the report on it. Its string form is the pattern.

Inside another object, and as a string, it shows as `{Pattern}`.

Written by [ConvertTo-TrexPattern](../records/#convertto-trexpattern).

| Property | Type | Holds |
|---|---|---|
| `Pattern` | string | The pattern, one branch per shape. |
| `Format` | string | A report template writing every field, tab-separated. |
| `File` | string | The pattern as a file -PatternFile and `trex lib` read, its `fields` line keeping each field's type, record start, accessor and order, so `\{extract}` read under it after Import-TrexAtom writes the objects this pattern writes. |
| `Declarations` | string[] | The shapes -MintShapes declared, each a line of File, `shape kb = `KB[0-9]{7}``; the pattern reads only under them, as ConvertFrom-TrexText reads it. |
| `Suggestions` | string[] | Each field every value of which a value class of the library holds, where no counter-example called for one, as `field: \{class} ...`; a -NotExample one of them refuses prints it in the pattern. |
| `Fields` | [Trex.BuiltField](#trexbuiltfield)[] | The fields, in the order first marked or named. |
| `Shapes` | [Trex.BuiltShape](#trexbuiltshape)[] | The shapes, in the order of their branches in the pattern. |
| `Rows` | object[] | Each line: its Line index, the index of its Shape (`$null` for a line with no token), its Text, and Values, each field's value by name (an array of strings for a list field, a hashtable of Text and the fields inside for a field holding others), `$null` where the line does not hold it. |
| `Records` | object[] | Each record: the indexes in Rows of its Lines, and Values, each field's first value among them by name, and for a field holding the one that begins the record, as a mark spanning lines does, its text on each line joined with a newline. A line holding a field that begins a record begins one and the lines after it join it; with no such field, each line is one. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Pattern}`, each braced name read from that property. |

### Trex.BuiltShape

One shape of the lines a built pattern reads: a branch of the pattern.

Inside another object, and as a string, it shows as `{Pattern}`.

Held in [Trex.BuiltPattern](#trexbuiltpattern).Shapes.

| Property | Type | Holds |
|---|---|---|
| `Pattern` | string | The branch. |
| `Lines` | long[] | The indexes in Rows of the lines of this shape. |
| `Reach` | object | How the shape reaches each field, by name: `marked`, `missing`, or the index in Shapes of the shape whose literals place it here. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Pattern}`, each braced name read from that property. |

### Trex.Capture

What one register bound.

Inside another object, and as a string, it shows as `{Name}={Text}`.

Held in [Trex.Match](#trexmatch).Groups.

| Property | Type | Holds |
|---|---|---|
| `Name` | string | The register's name. |
| `Text` | string | The text it bound; for a register bound under a repetition, the last binding. |
| `Start` | long | Where the text starts in the input, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units the text spans. |
| `Kind` | string | The token kind the register binds, such as `ip` or `timestamp`, a declared shape or kind by the name its declaration gave it, or empty where it binds more than one kind. |
| `Value` | object | The text parsed as its kind: a number as `long` or `decimal`, a duration as `TimeSpan`, an instant as `DateTimeOffset`, an address or a version as its canonical text; `$null` where it has none. |
| `Items` | string[] | Every binding of a register bound under a repetition, oldest first. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Name}={Text}`, each braced name read from that property. |

### Trex.ClassifiedRegion

A region of an input classified by its texture and its token shapes.

Inside another object, and as a string, it shows as `{Kind} at {Offset}, {Length} long`.

Held in [Trex.SpectralReport](#trexspectralreport).Regions.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the region starts, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units it spans. |
| `Kind` | [Trex.RegionKind](#trexregionkind) | What the region is. |
| `Period` | int | A table's period in tokens; 0 for every other kind. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Kind} at {Offset}, {Length} long`, each braced name read from that property. |

### Trex.Clock

The clock typed predicates read.

Written by [Get-TrexClock](../atoms/#get-trexclock), [Set-TrexClock](../atoms/#set-trexclock).

| Property | Type | Holds |
|---|---|---|
| `Now` | DateTimeOffset | The instant `now` reads as. |
| `Fixed` | bool | Whether `now` is fixed by Set-TrexClock rather than read from the system clock at each scan. |
| `TimeZoneOffset` | TimeSpan | The offset a timestamp written with no zone is read at. |
| `DateOrder` | [Trex.DateOrder](#trexdateorder) | Which field of an all-numeric slash date is the day. |

### Trex.ContextAgreement

One supertoken's role and how far each lower grain's nearest boundary is from its start, in significant tokens: negative before it.

Inside another object, and as a string, it shows as `{Role} at {Offset}`.

Held in [Trex.ContextReport](#trexcontextreport).Agreement.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the supertoken starts, in UTF-16 code units. |
| `Text` | string | Its text. |
| `Role` | string | Its role: call, assign, kv, list, numeric or plain. |
| `Regime` | int | The nearest spectral change point; `$null` where there is none. |
| `Shape` | int | The nearest shape change point; `$null` where there is none. |
| `Seam` | int | The nearest seam cut; `$null` where there is none. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Role} at {Offset}`, each braced name read from that property. |

### Trex.ContextAlignment

How consistently each lower grain's boundaries are at one offset from the starts of supertokens of one role: the share at their role's most common offset, from 0 to 1.

Inside another object, and as a string, it shows as `regime {Regime}, shape {Shape}, seam {Seam}`.

| Property | Type | Holds |
|---|---|---|
| `Regime` | float | The spectral change points. |
| `Shape` | float | The shape change points. |
| `Seam` | float | The seam cuts. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `regime {Regime}, shape {Shape}, seam {Seam}`, each braced name read from that property. |

### Trex.ContextEcho

A context's recurrence: how many of the tokens it folds are first sightings and how many recur.

Inside another object, and as a string, it shows as `novel {Novel}, echoed {Echoed}`.

| Property | Type | Holds |
|---|---|---|
| `Novel` | long | Tokens whose content occurs here for the first time. |
| `Echoed` | long | Tokens whose content recurs elsewhere in the input. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `novel {Novel}, echoed {Echoed}`, each braced name read from that property. |

### Trex.ContextFlow

A context's dynamics: the trend of magnitude over the tokens it folds.

Inside another object, and as a string, it shows as `slope {Slope}, reversals {Reversals}`.

| Property | Type | Holds |
|---|---|---|
| `Slope` | float | The mean windowed slope. |
| `Rising` | long | Tokens whose trend rises. |
| `Falling` | long | Tokens whose trend falls. |
| `Steady` | long | Tokens whose trend is steady. |
| `Momentum` | int | The longest run any token's trend has kept its direction. |
| `Reversals` | long | Changes of direction between adjacent tokens. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `slope {Slope}, reversals {Reversals}`, each braced name read from that property. |

### Trex.ContextFrame

One token's context: how many tokens it folds and each axis's reading over them.

Inside another object, and as a string, it shows as `{Text} at {Offset}, over {Over}`.

Held in [Trex.ContextReport](#trexcontextreport).Frames.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the token starts, in UTF-16 code units. |
| `Text` | string | The token's text. |
| `Phase` | int | The token's column of the record period; `$null` in a stream with no period. |
| `Over` | long | How many tokens the context folds; 0 where it holds none, and then every axis below is `$null`. |
| `Magnitude` | object | Their scale, a Trex.ContextMagnitude. |
| `Stress` | object | Their bracket depth, a Trex.ContextStress. |
| `Spectral` | object | Their texture, a Trex.ContextSpectral. |
| `Echo` | object | Their recurrence, a Trex.ContextEcho. |
| `Observation` | object | Their vantage, a Trex.ContextObservation. |
| `Seam` | object | Their segmentation, a Trex.ContextSeam. |
| `Flow` | object | Their dynamics, a Trex.ContextFlow. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}, over {Over}`, each braced name read from that property. |

### Trex.ContextMagnitude

A context's scale: the orders of magnitude of the tokens it folds.

Inside another object, and as a string, it shows as `mean {Mean}, spread {Spread}, max {Max}`.

| Property | Type | Holds |
|---|---|---|
| `Mean` | float | Their mean. |
| `Spread` | float | Their standard deviation, the spread an `s` predicate counts in. |
| `Max` | float | The largest. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `mean {Mean}, spread {Spread}, max {Max}`, each braced name read from that property. |

### Trex.ContextObservation

A context's vantage: what the bytes before, after and around the tokens it folds read, and how far before and after disagree.

Inside another object, and as a string, it shows as `disagreement {Disagreement}, contested {Contested}`.

| Property | Type | Holds |
|---|---|---|
| `Causal` | float | The mean reading of the window before each token, from 0 to 1. |
| `Anticausal` | float | The mean reading of the window after each token, from 0 to 1. |
| `Centered` | float | The mean reading of the window centered on each token, from 0 to 1. |
| `Disagreement` | float | The mean disagreement between before and after. |
| `DisagreementMax` | float | The largest disagreement. |
| `Contested` | long | Tokens spanning a contested point. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `disagreement {Disagreement}, contested {Contested}`, each braced name read from that property. |

### Trex.ContextReport

The context axis over one input: what each token is read against.

Written by [Measure-TrexContext](../axes/#measure-trexcontext).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Fold` | [Trex.ContextFold](#trexcontextfold) | The context read. |
| `Tokens` | long | How many tokens the axis read, whitespace left out. |
| `Units` | long | How many supertokens they form. |
| `Period` | int | The record period in tokens, the lag the stream's token shapes repeat at most; `$null` where none clears chance. |
| `LivePeriods` | int[] | Every lag that clears chance as the period does, strongest first. |
| `Alignment` | object | How consistently the lower grains' boundaries line up with the supertokens, a Trex.ContextAlignment. |
| `Agreement` | [Trex.ContextAgreement](#trexcontextagreement)[] | Every supertoken's role and nearest boundaries, with -Detail. |
| `Frames` | [Trex.ContextFrame](#trexcontextframe)[] | Every token's context, with -Detail. |

### Trex.ContextSeam

A context's segmentation: how strongly the tokens it folds align with seams, read in both directions.

Inside another object, and as a string, it shows as `strength {Strength}, cuts {Cuts}`.

| Property | Type | Holds |
|---|---|---|
| `Forward` | float | The mean forward branching entropy. |
| `Backward` | float | The mean backward branching entropy. |
| `Strength` | float | The mean seam strength. |
| `StrengthMax` | float | The strongest seam. |
| `Cuts` | long | Tokens starting a segment. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `strength {Strength}, cuts {Cuts}`, each braced name read from that property. |

### Trex.ContextSpectral

A context's texture: the spectral reading of the tokens it folds.

Inside another object, and as a string, it shows as `entropy {Entropy}, period {Period}`.

| Property | Type | Holds |
|---|---|---|
| `Entropy` | float | The mean pooled byte-class entropy, from 0 to 1. |
| `Period` | int | The byte period of the token whose period repeats most strongly; 0 for none. |
| `Strength` | float | How strongly that period repeats, from 0 to 1. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `entropy {Entropy}, period {Period}`, each braced name read from that property. |

### Trex.ContextStress

A context's load: the bracket depth of the tokens it folds.

Inside another object, and as a string, it shows as `mean {Mean}, max {Max}`.

| Property | Type | Holds |
|---|---|---|
| `Mean` | float | The mean depth. |
| `Max` | int | The deepest. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `mean {Mean}, max {Max}`, each braced name read from that property. |

### Trex.Echo

A key that recurs: a token's text, or the structure of a run of tokens.

Inside another object, and as a string, it shows as `{Key} ({Count})`.

Held in [Trex.EchoReport](#trexechoreport).Echoes, [Trex.EchoReport](#trexechoreport).Structures.

| Property | Type | Holds |
|---|---|---|
| `Key` | string | The key: the text of its first occurrence, or a structure's role and the kinds of its tokens. |
| `Count` | long | How many times it occurs. |
| `Period` | object | The mean distance between occurrences in bytes, where the distances are regular; `$null` where they are not. |
| `Offset` | long | Where its first occurrence starts, in UTF-16 code units. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Key} ({Count})`, each braced name read from that property. |

### Trex.EchoFrame

One token's echo reading.

Inside another object, and as a string, it shows as `{Text} at {Offset}`.

Held in [Trex.EchoReport](#trexechoreport).Frames.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the token starts, in UTF-16 code units. |
| `Text` | string | The token's text. |
| `Count` | long | How many times the token's key occurs in the input. |
| `Nth` | long | Which occurrence of its key this is, counting from 1. |
| `Previous` | object | Where the key's previous occurrence starts, in UTF-16 code units; `$null` at its first. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}`, each braced name read from that property. |

### Trex.EchoReport

The echo axis over one input: which tokens recur, how often, and how regularly.

Written by [Measure-TrexEcho](../axes/#measure-trexecho).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Group` | string | The group the keys were read under. |
| `Tokens` | long | How many tokens the lexer read. |
| `Keyed` | long | The tokens that take part: words, numbers, quoted runs and typed literals. |
| `Distinct` | long | How many distinct keys they hold. |
| `Novel` | long | The keyed tokens that are the first occurrence of their key. |
| `Echoed` | long | The keyed tokens whose key occurs more than once. |
| `Novelty` | float | The share of keyed tokens that are first occurrences, from 0 to 1. |
| `EchoRate` | float | The share of keyed tokens whose key recurs, from 0 to 1. |
| `Echoes` | [Trex.Echo](#trexecho)[] | Every key that recurs, most occurrences first. |
| `Structures` | [Trex.Echo](#trexecho)[] | The structures that recur among runs of tokens, most occurrences first, with -Structure. |
| `Frames` | [Trex.EchoFrame](#trexechoframe)[] | Every keyed token's frame, with -Detail. |

### Trex.ExplainedToken

One token a match spans, as an explanation reads it.

Inside another object, and as a string, it shows as `{Kind} {Text}`.

Held in [Trex.Explanation](#trexexplanation).Tokens.

| Property | Type | Holds |
|---|---|---|
| `Kind` | string | The token's kind. |
| `Text` | string | The token's text. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Kind} {Text}`, each braced name read from that property. |

### Trex.Explanation

What a match is made of: the kinds of the tokens it spans, the check each guarded kind passed, every axis the pattern read at them, and the route the scan took.

| Property | Type | Holds |
|---|---|---|
| `Tokens` | [Trex.ExplainedToken](#trexexplainedtoken)[] | The significant tokens the match spans, with their kinds. |
| `Guards` | string[] | The check each guarded token passed to be its kind, such as a card number's Luhn check. |
| `Readings` | [Trex.Reading](#trexreading)[] | Each axis the pattern reads, at each token of the match. |
| `Route` | string | The rung of the scan ladder that answered the scan of the match's input, as the `trex` command's `--explain` names it. |

### Trex.File

One file a walk found.

Written by [Get-TrexFile](../matching/#get-trexfile).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file's path. |
| `Texture` | object | What the file's text reads as mostly, where -Classify or a texture filter read it; `$null` where neither did, or where the text holds no region. |
| `Period` | int | A table's period in tokens; 0 for every other texture. |

### Trex.FileType

One file type the type filters name.

Written by [Get-TrexFileType](../matching/#get-trexfiletype).

| Property | Type | Holds |
|---|---|---|
| `Name` | string | The type's name, as -FileType takes it. |
| `Globs` | string[] | The globs a file of the type matches. |

### Trex.FilterCheck

How one filter kept its contract over a corpus.

Written by [Test-TrexPrefilter](../streams/#test-trexprefilter).

| Property | Type | Holds |
|---|---|---|
| `Filter` | [Trex.FilterKind](#trexfilterkind) | The filter probed. |
| `PresentProbes` | long | The substrings of the corpus probed, each present by construction. |
| `FalseNegatives` | long | The present probes the filter reported absent, which its contract forbids. |
| `AbsentProbes` | long | The byte strings probed that an exact search finds nowhere in the corpus. |
| `AbsentRejected` | long | The absent probes the filter rejected, each a scan it saves. |

### Trex.Finding

One finding of a rule.

Written by [Invoke-TrexRule](../rules/#invoke-trexrule).

| Property | Type | Holds |
|---|---|---|
| `Rule` | string | The rule's name. |
| `Severity` | [Trex.Severity](#trexseverity) | How serious it is. |
| `Message` | string | What it says, rendered from the match. |
| `Path` | string | The file it is in, empty for a string. |
| `Region` | [Trex.Region](#trexregion) | Where it starts and ends, by line and column. |
| `Start` | long | Where it starts in the input, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units it spans. |
| `Text` | string | Its text: the match, or the record for a rule that fires on records. |
| `Fix` | object | What a fix puts in the match's place; `$null` where the rule has no fix. |
| `FixRegion` | object | The span a fix replaces, the match, a Trex.Region; `$null` where the rule has no fix. |
| `Captures` | object | Each register the rule's pattern bound, as a hashtable of its name to its text. |
| `Definition` | object | The rule itself, a Trex.Rule. |

### Trex.FlowFrame

One token's flow reading.

Inside another object, and as a string, it shows as `{Text} at {Offset}, {Direction}`.

Held in [Trex.FlowReport](#trexflowreport).Frames, [Trex.FlowReport](#trexflowreport).Reversals.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the token starts, in UTF-16 code units. |
| `Text` | string | The token's text. |
| `Slope` | float | The signal's change per unit, averaged over the -Window units ending at this one, four by default. |
| `Direction` | [Trex.FlowDirection](#trexflowdirection) | Which way the trend runs: Steady where the slope is within -SteadyBand of zero, 0.05 by default. |
| `Momentum` | int | How many consecutive tokens have held the same direction, rising or falling. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}, {Direction}`, each braced name read from that property. |

### Trex.FlowReport

The flow axis over one input: the trend of a per-unit signal, how long each run of it lasts, and where it turns.

Written by [Measure-TrexFlow](../axes/#measure-trexflow).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Tokens` | long | How many tokens the input holds, whitespace left out. |
| `Grain` | [Trex.Grain](#trexgrain) | The stream the trend was read over: Token, or Super for the supertokens. |
| `Units` | long | How many units the axis read at that grain. |
| `Signal` | [Trex.FlowSignal](#trexflowsignal) | The signal whose trend was read. |
| `PeakMomentum` | object | The unit holding the longest run, a Trex.FlowFrame; `$null` for an input with no units. |
| `Reversals` | [Trex.FlowFrame](#trexflowframe)[] | The units where the trend turns between rising and falling. |
| `Frames` | [Trex.FlowFrame](#trexflowframe)[] | Every unit's frame, with -Detail. |
| `Latency` | long | How many units past a unit its analytic reading uses, with -Analytic; `$null` without it. The last Latency units have no reading. |
| `Analytic` | [Trex.AnalyticFrame](#trexanalyticframe)[] | Every unit's analytic reading that has one, with -Analytic. |

### Trex.Grammar

A token grammar, compiled. A proxy class.

Written by [New-TrexGrammar](../streams/#new-trexgrammar).

| Property | Type | Holds |
|---|---|---|
| `Start` | string | The rule a parse starts from. |
| `Rules` | string[] | Every rule's name, in the order the grammar declares them. |
| `IsDisposed` | bool | Whether `Dispose()` has freed the value behind the object; a property read after that is `$null`. |

| Method | Returns | Does |
|---|---|---|
| `[Trex.Grammar]::new(string source)` | [Trex.Grammar](#trexgrammar) | Compiles grammar source, one `name := alternatives` rule a line, starting from its first rule; New-TrexGrammar -Start names another. |
| `Parse(string text)` | [Trex.ParseNode](#trexparsenode) | The parse of `text` from the start rule, or `$null` where it does not parse in full. |
| `Test(string text)` | bool | Whether `text` parses in full under any derivation. |
| `CountParses(string text)` | ulong | How many derivations of `text` the grammar has: 0 where it does not parse, more than 1 where the grammar is ambiguous for it. |
| `BestProbability(string text)` | double | The probability of `text`'s most probable derivation under the grammar's `@p` weights; 0 where it does not parse. |
| `TotalProbability(string text)` | double | The probability of all of `text`'s derivations together. |
| `CountSegmentations(string text, string[] dictionary)` | ulong | How many ways the run-together `text` tiles into the words of `dictionary` with the grammar accepting the words. |
| `BestSegmentationProbability(string text, string[] dictionary)` | double | The probability of the most probable tiling of the run-together `text` into the words of `dictionary` that the grammar accepts. |
| `Segment(string text, string[] dictionary)` | [Trex.Tiling](#trextiling)[] | Every tiling of the run-together `text` into the words of `dictionary` that the grammar accepts, each with its most probable parse's probability and its parse count, most probable first. |

### Trex.GravityClass

One gravity class: types the pair field places together, kin to one another under @kin.

Inside another object, and as a string, it shows as `class {Class}, {Units} units`.

Held in [Trex.GravityReport](#trexgravityreport).Classes.

| Property | Type | Holds |
|---|---|---|
| `Class` | int | The class's number, from 0 to 15. |
| `Types` | string[] | Its types, most frequent first: at the byte grain each byte, as itself where it is printable ASCII, ' ' for a space and an escape otherwise; a token's kind with a punctuation mark's text; a supertoken's role and kinds. |
| `Units` | long | How many of the grain's units are of its types. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `class {Class}, {Units} units`, each braced name read from that property. |

### Trex.GravityFrame

One unit's gravity reading: how its past pushes it away, how strongly the input holds together across the cut before it, and its type's class.

Inside another object, and as a string, it shows as `{Text} at {Offset}`.

Held in [Trex.GravityReport](#trexgravityreport).Frames, [Trex.GravityReport](#trexgravityreport).Strained, [Trex.GravityReport](#trexgravityreport).Weakest.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | The unit's offset, in UTF-16 code units; each byte of a character written in several has a frame at the character's offset. |
| `Text` | string | The unit's text: at the byte grain the character the byte begins, empty for a byte inside a character written in several; the token or the supertoken otherwise. |
| `Strain` | float | The unit's mean potential against each unit before it within the field's reach, in bits: high where what came before pushes it away. `$null` at the first unit, which has nothing before it. |
| `StrainPercentile` | double | The share of the input's strain readings below this one, in percent. |
| `Bound` | float | The attraction across the cut before the unit, the mean log2 g over the pairs straddling it within 8 units, in bits per pair: low where the two sides hold together least. `$null` at the first unit, which has no cut before it. |
| `BoundPercentile` | double | The share of the input's bound readings below this one, in percent. |
| `Class` | int | The gravity class of the unit's type; `$null` where no significant cell places it, so it is kin to nothing but itself. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}`, each braced name read from that property. |

### Trex.GravityReport

The pair field over one input: where each unit's past pushes it away, where the input holds together least, and which types it treats alike.

Written by [Measure-TrexGravity](../axes/#measure-trexgravity).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Grain` | [Trex.Grain](#trexgrain) | The stream read: Token, Byte or Super for the supertokens. |
| `Units` | long | How many units the field read at that grain. |
| `Types` | long | How many distinct types those units are of. |
| `Strained` | [Trex.GravityFrame](#trexgravityframe)[] | The units under the most strain, most first, -Top of them. |
| `Weakest` | [Trex.GravityFrame](#trexgravityframe)[] | The units before the cuts held together least, least first, -Top of them. |
| `Classes` | [Trex.GravityClass](#trexgravityclass)[] | The classes the field places types in, ascending. |
| `Frames` | [Trex.GravityFrame](#trexgravityframe)[] | Every unit's frame, with -Detail. |

### Trex.IndexInfo

What a tree's index holds.

Written by [Get-TrexIndex](../matching/#get-trexindex), [New-TrexIndex](../matching/#new-trexindex).

| Property | Type | Holds |
|---|---|---|
| `Root` | string | The tree the index covers. |
| `Path` | string | The index file, at the tree's root. |
| `Files` | long | How many files it covers. |

### Trex.Info

What this build of TREX is.

Written by [Get-TrexInfo](../atoms/#get-trexinfo).

| Property | Type | Holds |
|---|---|---|
| `Version` | string | The TREX engine's version. |
| `DeviceAvailable` | bool | Whether a CUDA device is present for the scans that can use one. |

### Trex.Library

A set of atoms held in an object rather than the session, passed to a cmdlet with `-Library`. A proxy class.

Written by [New-TrexLibrary](../atoms/#new-trexlibrary).

| Property | Type | Holds |
|---|---|---|
| `Names` | string[] | The names declared in this library, in declaration order. |
| `Files` | string[] | The pattern files imported into it. |
| `IsDisposed` | bool | Whether `Dispose()` has freed the value behind the object; a property read after that is `$null`. |

| Method | Returns | Does |
|---|---|---|
| `Declare(string line)` | void | Declares one line as a pattern file writes it: `shape name = \`bytes\``, `kind name = pattern`, `let name = pattern`, or a `test` line. A relative `@file` in it is read from the location the library was made at. |

### Trex.Line

One line of an input.

Inside another object, and as a string, it shows as `{LineNumber}:{Text}`.

Written by [Get-TrexLine](../matching/#get-trexline).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file the line is in, or empty for text. |
| `LineNumber` | long | The line's number in the input, counting from 1. |
| `Text` | string | The line's text, without its line ending. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{LineNumber}:{Text}`, each braced name read from that property. |

### Trex.LiteralTest

What a filter answers for one literal, beside what an exact search of the corpus finds.

Written by [Test-TrexPrefilter](../streams/#test-trexprefilter).

| Property | Type | Holds |
|---|---|---|
| `Literal` | string | The literal asked about. |
| `Filter` | [Trex.FilterKind](#trexfilterkind) | The filter that answered. |
| `MightOccur` | bool | Whether the filter says the literal might occur; false is exact. |
| `Occurs` | bool | Whether an exact search finds the literal in the corpus. A literal the filter says might occur that does not is a false positive. |

### Trex.MagnitudeFrame

One token's magnitude reading.

Inside another object, and as a string, it shows as `{Text} at {Offset}`.

Held in [Trex.MagnitudeReport](#trexmagnitudereport).Frames, [Trex.MagnitudeReport](#trexmagnitudereport).Jumps, [Trex.MagnitudeReport](#trexmagnitudereport).Outliers.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the token starts, in UTF-16 code units. |
| `Text` | string | The token's text. |
| `Magnitude` | float | The token's order of magnitude: log10 of a number's absolute value, log2 of any other token's length in bytes. |
| `Gradient` | float | The change in magnitude from the token before. |
| `Energy` | float | The sum of squared magnitudes over the 16 tokens ending at this one. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}`, each braced name read from that property. |

### Trex.MagnitudeReport

The magnitude axis over one input: the order of magnitude each token carries, where it jumps, and the tokens far from the input's mean.

Written by [Measure-TrexMagnitude](../axes/#measure-trexmagnitude).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Tokens` | long | How many tokens the axis read, whitespace left out. |
| `TotalEnergy` | float | The sum of squared magnitudes over the whole input. |
| `Peak` | object | The token of greatest magnitude, a Trex.MagnitudeFrame; `$null` for an input with no tokens. |
| `Jumps` | [Trex.MagnitudeFrame](#trexmagnitudeframe)[] | The tokens whose magnitude differs from the one before by at least -JumpThreshold orders, three by default. |
| `Outliers` | [Trex.MagnitudeFrame](#trexmagnitudeframe)[] | The tokens whose magnitude is at least -OutlierSigma standard deviations from the input's mean, two and a half by default. |
| `Frames` | [Trex.MagnitudeFrame](#trexmagnitudeframe)[] | Every token's frame, with -Detail. |

### Trex.Match

One match.

Written by [Select-TrexMatch](../matching/#select-trexmatch).

Returned by [Trex.Pattern](#trexpattern).Find, [Trex.Pattern](#trexpattern).FindAll.

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file the match is in, or empty for text. |
| `LineNumber` | long | The line the match starts on, counting from 1. |
| `Column` | long | The character the match starts at within its line, counting from 1. |
| `Start` | long | Where the match starts in the input, in UTF-16 code units, so `$input.Substring($m.Start, $m.Length)` is the match. |
| `Length` | long | How many UTF-16 code units the match spans. |
| `Text` | string | The matched text. |
| `Captures` | object | A hashtable of each register's name to the text it bound, so `$m.Captures.name` reads one. |
| `Groups` | [Trex.Capture](#trexcapture)[] | Each register the pattern bound, in the order the pattern writes them, with its span, its kind and its parsed value. |
| `Pattern` | string | The pattern's name within a pattern set, or empty. |
| `PreContext` | string[] | The lines before the match's line, with -Context. |
| `PostContext` | string[] | The lines after the match's line, with -Context. |
| `Explanation` | object | What the match is made of, a Trex.Explanation, with -Explain; `$null` otherwise. |

### Trex.MatchCount

How many records of an input hold a match, or how many matches it holds.

Written by [Invoke-TrexRule](../rules/#invoke-trexrule), [Select-TrexMatch](../matching/#select-trexmatch).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file counted, empty for text. |
| `Count` | long | The records holding a match, or under -NotMatch holding none; under -CountMatches, the matches. |

### Trex.ObservationFrame

One unit's observation reading: the entropy of the classes around it, seen from behind, from ahead and from both sides.

Inside another object, and as a string, it shows as `at {Offset}, disagreement {Disagreement}`.

Held in [Trex.ObservationReport](#trexobservationreport).Contested, [Trex.ObservationReport](#trexobservationreport).Frames.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | The unit's offset, in UTF-16 code units; each byte of a character written in several has a frame at the character's offset. |
| `Text` | string | The unit's text: at the byte grain the character the byte begins, empty for a byte inside a character written in several; the token or the supertoken otherwise. |
| `Causal` | float | The class entropy of the window before the unit, from 0 to 1. |
| `Anticausal` | float | The class entropy of the window after the unit, from 0 to 1. |
| `Centered` | float | The class entropy of the window centered on the unit, from 0 to 1. |
| `Disagreement` | float | How far the two sides differ: Causal less Anticausal, unsigned. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `at {Offset}, disagreement {Disagreement}`, each braced name read from that property. |

### Trex.ObservationReport

The observation axis over one input: where the text behind a point and the text ahead of it read differently.

Written by [Measure-TrexObservation](../axes/#measure-trexobservation).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Grain` | [Trex.Grain](#trexgrain) | The stream read: Byte, Token or Super for the supertokens. |
| `Units` | long | How many units the axis read at that grain. |
| `Peak` | object | The contested unit where the two sides differ most, a Trex.ObservationFrame; `$null` where none is contested. |
| `Contested` | [Trex.ObservationFrame](#trexobservationframe)[] | The units where the two sides differ most: local maxima of Disagreement at least -ContestedThreshold, 0.2 by default; at the byte grain at least -ContestedMinGap bytes apart, 8 by default, the stronger of two closer maxima kept. |
| `Frames` | [Trex.ObservationFrame](#trexobservationframe)[] | Every unit's frame, with -Detail. |

### Trex.OrbitClass

One orbit and the distinct texts that fold onto it.

Inside another object, and as a string, it shows as `{Orbit}`.

Held in [Trex.OrbitReport](#trexorbitreport).Classes.

| Property | Type | Holds |
|---|---|---|
| `Orbit` | string | The orbit's representative. |
| `Forms` | string[] | The distinct texts in the input that fold onto it, sorted. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Orbit}`, each braced name read from that property. |

### Trex.OrbitReport

The orbit axis over one input: its tokens read under a symmetry, and the distinct texts each orbit folds together.

Written by [Measure-TrexOrbit](../axes/#measure-trexorbit).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Group` | string | The group the tokens were read under. |
| `Tokens` | long | How many tokens were read, whitespace left out. |
| `Forms` | long | How many distinct texts the tokens hold. |
| `Orbits` | long | How many distinct orbits they fold onto. |
| `Classes` | [Trex.OrbitClass](#trexorbitclass)[] | Every orbit with the texts that fold onto it, by representative. |
| `Matches` | [Trex.OrbitToken](#trexorbittoken)[] | The tokens in the same orbit as -SameAs, with -SameAs. |
| `Segments` | [Trex.Segment](#trexsegment)[] | The runs of one kind of character, letters, digits, whitespace or the rest, with -Detail. |
| `Frames` | [Trex.OrbitToken](#trexorbittoken)[] | Every token with its orbit, with -Detail. |

### Trex.OrbitToken

One token under a symmetry: its text and the orbit it belongs to.

Inside another object, and as a string, it shows as `{Text} at {Offset}`.

Held in [Trex.OrbitReport](#trexorbitreport).Frames, [Trex.OrbitReport](#trexorbitreport).Matches.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the token starts, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units it spans. |
| `Text` | string | The token's text. |
| `Orbit` | string | The orbit's representative: the form every token of the orbit takes under the group. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}`, each braced name read from that property. |

### Trex.ParseNode

One node of a parse tree: a rule with the nodes it matched, or a token.

Inside another object, and as a string, it shows as `{Expression}`.

Written by [Invoke-TrexGrammar](../streams/#invoke-trexgrammar).

Returned by [Trex.Grammar](#trexgrammar).Parse.

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file the text came from, on the root of a tree parsed from one; empty for a string and for every node under the root. |
| `Rule` | string | The rule the node matched, empty for a token. |
| `Text` | string | The token's text, or for a rule the texts of the tokens under it joined by single spaces. |
| `Children` | object[] | The nodes the rule matched, in order, each a Trex.ParseNode; none for a token. |
| `Expression` | string | The tree under this node as an S-expression: a token as its text, a rule as `(rule child ...)`, and a rule of one child as that child. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Expression}`, each braced name read from that property. |

### Trex.Pattern

A compiled TREX pattern and the atoms it was compiled against. A proxy class.

Written by [New-TrexPattern](../matching/#new-trexpattern).

| Property | Type | Holds |
|---|---|---|
| `Source` | string | The pattern's source text. |
| `CaptureNames` | string[] | The registers the pattern binds, in the order it writes them. |
| `IsDisposed` | bool | Whether `Dispose()` has freed the value behind the object; a property read after that is `$null`. |

| Method | Returns | Does |
|---|---|---|
| `[Trex.Pattern]::new(string source)` | [Trex.Pattern](#trexpattern) | Compiles `source` with the atoms TREX ships and none declared; New-TrexPattern compiles against the session's or a library's. |
| `IsMatch(string input)` | bool | Whether the pattern matches anywhere in `input`. |
| `Find(string input)` | [Trex.Match](#trexmatch) | The first match in `input`, or `$null`. |
| `FindAll(string input)` | [Trex.Match](#trexmatch)[] | Every match in `input`. |
| `Replace(string input, string template)` | string | `input` with each match replaced by `template` rendered at it: `${name}` a register, `${0}` the match, `${ip:octet1-2}` a typed slice. |
| `Split(string input)` | string[] | The text between the matches in `input`. |

### Trex.Prefilter

A presence filter built over a corpus. A proxy class.

Written by [New-TrexPrefilter](../streams/#new-trexprefilter).

| Property | Type | Holds |
|---|---|---|
| `Filter` | [Trex.FilterKind](#trexfilterkind) | Which filter it is. |
| `Bytes` | long | The size of the corpus it was built over, in bytes as UTF-8. |
| `IsDisposed` | bool | Whether `Dispose()` has freed the value behind the object; a property read after that is `$null`. |

| Method | Returns | Does |
|---|---|---|
| `[Trex.Prefilter]::new(string corpus)` | [Trex.Prefilter](#trexprefilter) | Builds a Bloom filter over `corpus`; New-TrexPrefilter builds the others and reads files. |
| `MightContain(string literal)` | bool | Whether `literal` might occur in the corpus: false is exact, true can be a false positive. |

### Trex.Reading

One reading of an axis the pattern reads, at a token of the match.

Inside another object, and as a string, it shows as `{Axis} {Text}: {Value}`.

Held in [Trex.Explanation](#trexexplanation).Readings.

| Property | Type | Holds |
|---|---|---|
| `Axis` | string | The axis read. |
| `Text` | string | The token's text. |
| `Value` | string | The value the pattern compared, as the axis gives it. |
| `Parts` | object | The values the reading gives, each under the name a report template uses after the dot in `${@axis.piece}`; empty for an axis that gives one value. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Axis} {Text}: {Value}`, each braced name read from that property. |

### Trex.Record

One record of an input.

Written by [Get-TrexRecord](../records/#get-trexrecord).

| Property | Type | Holds |
|---|---|---|
| `Text` | string | The record's text. |
| `Start` | long | Where it starts in the input, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units it spans. |

### Trex.RecordHit

One record a query keeps.

Written by [Find-TrexRecord](../records/#find-trexrecord).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file the record is in, empty for a string. |
| `LineNumber` | long | The line the record starts on, counting from 1. |
| `Start` | long | Where the record starts in the input, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units it spans. |
| `Text` | string | The record's text. |
| `Patterns` | string[] | The patterns it holds, as they were given. |

### Trex.RecordShape

One record shape: the records that share a sequence of token kinds, with the positions that vary written as their kind.

Written by [Get-TrexRecordShape](../records/#get-trexrecordshape).

| Property | Type | Holds |
|---|---|---|
| `Count` | long | How many records have this shape. |
| `Readable` | string | The shape with its varying positions written as `<kind>`. |
| `Pattern` | string | The shape as a TREX pattern that matches its records. |
| `Rare` | bool | Whether the shape covers fewer records than the cut. |
| `Novel` | object | With -Against, whether no template of the other input would accept these records; `$null` where nothing was compared. |
| `Records` | long[] | The records of this shape, counted from 0. |
| `Covered` | long | How many records of the input had a shape at all, the total a share such as `-Cut 1%` is taken of. |

### Trex.Region

A span's position: its first line and column and the line and column just past its end, each from 1.

Inside another object, and as a string, it shows as `{Line}:{Column}`.

Held in [Trex.Finding](#trexfinding).Region.

| Property | Type | Holds |
|---|---|---|
| `Line` | long | The line it starts on. |
| `Column` | long | The character it starts at within its line. |
| `EndLine` | long | The line just past its end is on. |
| `EndColumn` | long | The character just past its end. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Line}:{Column}`, each braced name read from that property. |

### Trex.RelationChord

One reuse: a later occurrence of repeated content joined to the earlier.

Inside another object, and as a string, it shows as `{From} to {To}`.

Held in [Trex.RelationReport](#trexrelationreport).Chords.

| Property | Type | Holds |
|---|---|---|
| `From` | string | The earlier occurrence's text. |
| `To` | string | The later occurrence's text. |
| `Residual` | int | The later occurrence's depth less the earlier's: positive where the reuse is deeper. |
| `FromOffset` | long | Where the earlier occurrence starts, in UTF-16 code units. |
| `ToOffset` | long | Where the later occurrence starts, in UTF-16 code units. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{From} to {To}`, each braced name read from that property. |

### Trex.RelationEdge

One directed relation between two tokens.

Inside another object, and as a string, it shows as `{From} {Kind} {To}`.

Held in [Trex.RelationReport](#trexrelationreport).Edges.

| Property | Type | Holds |
|---|---|---|
| `Kind` | [Trex.RelationKind](#trexrelationkind) | The relation the edge carries. |
| `From` | string | The text of the token it runs from. |
| `To` | string | The text of the token it runs to. |
| `FromOffset` | long | Where the token it runs from starts, in UTF-16 code units. |
| `ToOffset` | long | Where the token it runs to starts, in UTF-16 code units. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{From} {Kind} {To}`, each braced name read from that property. |

### Trex.RelationFrame

One token's place in the enclosure tree.

Inside another object, and as a string, it shows as `{Text} at {Offset}, depth {Depth}`.

Held in [Trex.RelationReport](#trexrelationreport).Frames.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the token starts, in UTF-16 code units. |
| `Text` | string | The token's text. |
| `Depth` | int | How many brackets enclose it. |
| `Enclosure` | string[] | The heads of the bracket groups enclosing it, outermost first. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}, depth {Depth}`, each braced name read from that property. |

### Trex.RelationReport

The relation axis over one input: the graph its brackets, operators, adjacency and reuse make, read as a tree with loops.

Written by [Measure-TrexRelation](../axes/#measure-trexrelation).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Tokens` | long | How many tokens the lexer read, whitespace included. |
| `MaxDepth` | int | The deepest nesting of brackets. |
| `NestingLoad` | long | How many tokens are inside at least one bracket group. |
| `Encloses` | long | The edges from a bracket group's head to the tokens inside it. |
| `Operators` | long | The edges between the operands of a binding punctuation. |
| `Adjacencies` | long | The edges from each significant token to the next. |
| `ReuseChords` | long | The reuses: later occurrences of repeated content joined to the earlier. |
| `ScopeCrossingChords` | long | The reuses whose two occurrences are at different depths. |
| `Holonomy` | long | The reuses' depth differences summed without sign: 0 for a tree, and positive once a reuse crosses a scope. |
| `NetHolonomy` | long | The same sum with its sign: positive where reuse flows inward, negative outward, 0 where the two cancel. |
| `BoundaryEvents` | long | The brackets the input holds, opens and closes, as the boundary reading counts them. |
| `BulkNodes` | long | The balanced bracket groups. |
| `HolographicDefect` | long | What the brackets alone do not determine: the reuses. |
| `BoundaryClosed` | bool | Whether every bracket is balanced, so that the brackets alone determine the nesting. |
| `RebuiltEdges` | long | The enclosure edges rebuilt from the brackets alone. |
| `MinRicci` | int | The most negative edge curvature: the sharpest bottleneck, 0 for a graph with no edges. |
| `MeanRicci` | float | The mean edge curvature. |
| `Bridges` | long | The edges of negative curvature: the bottlenecks. |
| `Nodes` | long | The tokens that take part in at least one relation. |
| `GraphEdges` | long | The relation graph's undirected edges. |
| `Components` | long | Its connected components. |
| `CycleRank` | long | Its independent loops: edges less nodes plus components. |
| `Euler` | long | Its Euler characteristic: nodes less edges. |
| `MaxShortcut` | long | The longest run of tokens a single reuse joins in one step; 0 where nothing is reused. |
| `PeakEntanglement` | long | The most relation edges crossing any boundary between two tokens. |
| `MinimalCut` | object | The fewest relation edges crossing a boundary between two tokens; `$null` where the input has no boundary inside it. |
| `MinimalCutOffset` | object | That boundary's offset, in UTF-16 code units: the start of the token after it; `$null` with MinimalCut. |
| `Canonical` | string | The input's alpha-equivalence form, with -Canonical: the first word inside each `[` is a binder written `#`, a later use of it `^k` for the binder k scopes out, a name bound nowhere is kept, and the tokens are joined by spaces. |
| `Edges` | [Trex.RelationEdge](#trexrelationedge)[] | Every relation edge, with -Detail. |
| `Chords` | [Trex.RelationChord](#trexrelationchord)[] | Every reuse, with -Detail. |
| `Frames` | [Trex.RelationFrame](#trexrelationframe)[] | The frame of every token inside a bracket group, with -Detail. |

### Trex.Rule

One rule as its pattern file declares it.

Inside another object, and as a string, it shows as `{Name}`.

Written by [Get-TrexRule](../rules/#get-trexrule).

| Property | Type | Holds |
|---|---|---|
| `Name` | string | The rule's name, the id a finding is reported under. |
| `Severity` | [Trex.Severity](#trexseverity) | How serious a finding is. |
| `Message` | string | What a finding says: a report template rendered at each match. |
| `Pattern` | string | What the rule matches, as written. |
| `Fix` | string | The template a fix renders in the match's place, empty where the rule has none. |
| `Files` | string[] | The globs the inputs the rule reads are kept or dropped by; every input where there are none. |
| `Meta` | object | The rule's `meta.KEY = VALUE` lines, as a hashtable. |
| `Tags` | string[] | The values `meta.tags` lists. |
| `File` | string | The pattern file the rule is in, empty for one declared here. |
| `Line` | long | The line the rule opens on, counting from 1. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Name}`, each braced name read from that property. |

### Trex.ScanRoute

How many inputs one rung of the scan ladder answered, which -Stats reads from the trace TREX keeps while the cmdlet runs.

Inside another object, and as a string, it shows as `{Rung}: {Inputs}`.

Held in [Trex.ScanStats](#trexscanstats).Routes.

| Property | Type | Holds |
|---|---|---|
| `Rung` | string | The rung, as the `trex` command's `--stats` names it. |
| `Inputs` | long | How many inputs it answered. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Rung}: {Inputs}`, each braced name read from that property. |

### Trex.ScanStats

What a scan read and found, which -Stats writes after the report.

Written by [Select-TrexMatch](../matching/#select-trexmatch).

| Property | Type | Holds |
|---|---|---|
| `Matches` | long | The matches the report read. |
| `MatchedLines` | long | The lines those matches touch. |
| `FilesWithMatches` | long | The inputs holding a match. |
| `FilesSearched` | long | The inputs scanned; each string piped in is one. |
| `BytesSearched` | long | The bytes scanned, counted as UTF-8. |
| `Searching` | TimeSpan | The time the scans took. |
| `Elapsed` | TimeSpan | The time from the cmdlet's start to its end. |
| `TokensLexed` | long | The tokens the scans' lexes produced. |
| `Lexing` | TimeSpan | The time those lexes took, summed over the cores that ran them, which comes out of Searching rather than adding to it. |
| `Matching` | TimeSpan | Searching less Lexing: the time left for matching. |
| `DeviceBytes` | long | The bytes handed to the device. |
| `Routes` | [Trex.ScanRoute](#trexscanroute)[] | How many inputs each rung of the scan ladder answered, in the order first seen. |

### Trex.SeamFrame

One byte's seam reading: how uncertain the text is on either side of it.

Inside another object, and as a string, it shows as `{Text} at {Offset}`.

Held in [Trex.SeamReport](#trexseamreport).Frames.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | The byte's offset, in UTF-16 code units. |
| `Text` | string | The character the byte begins, empty for a byte inside a character written in several. |
| `Forward` | float | The forward branching entropy at the byte: how uncertain what follows is, given what precedes. |
| `Backward` | float | The backward branching entropy at the byte: how uncertain what precedes is, given what follows. |
| `Boundary` | float | The seam strength of a cut before the byte, from both readings together. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}`, each braced name read from that property. |

### Trex.SeamReport

The seam axis over one input: the units it divides into where the text before stops predicting the text after.

Written by [Measure-TrexSeam](../axes/#measure-trexseam).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Order` | int | The longest context the entropy was read over, in bytes. |
| `Segments` | [Trex.Segment](#trexsegment)[] | The units, in order. |
| `Frames` | [Trex.SeamFrame](#trexseamframe)[] | Every byte's frame, with -Detail. |

### Trex.Segment

A run of an input: where it starts, how long it is, and its text.

Inside another object, and as a string, it shows as `{Text}`.

Held in [Trex.OrbitReport](#trexorbitreport).Segments, [Trex.SeamReport](#trexseamreport).Segments.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the run starts, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units it spans. |
| `Text` | string | The run's text. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text}`, each braced name read from that property. |

### Trex.ShapeFrame

One token's shape reading.

Inside another object, and as a string, it shows as `{Text} at {Offset}`.

Held in [Trex.ShapeReport](#trexshapereport).Frames.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the token starts, in UTF-16 code units. |
| `Text` | string | The token's text. |
| `Class` | uint | The token's shape class: its kind and silhouette as one number, equal for tokens of one shape. |
| `Period` | int | The period, in tokens, at which the shapes around the token repeat; 0 where they do not. |
| `PeriodStrength` | float | How strongly the shapes repeat at that period, from 0 to 1: the share of the tokens in the window behind the token whose shape equals the shape one period before. A run whose strength reaches -TemplateStrength is a region. |
| `Novelty` | float | How new the run of shapes ending here is, from 0 to 1, 1 for its first sighting. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}`, each braced name read from that property. |

### Trex.ShapeRegion

A run of tokens whose shapes repeat at one period: a table, a list of records, a template filled in again and again.

Inside another object, and as a string, it shows as `at {Offset}, {Length} long, period {Period}`.

Held in [Trex.ShapeReport](#trexshapereport).Regions.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the run starts, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units it spans. |
| `Period` | int | The period its shapes repeat at, in tokens. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `at {Offset}, {Length} long, period {Period}`, each braced name read from that property. |

### Trex.ShapeReport

The shape axis over one input: the template its tokens repeat, where that repetition holds, and where it breaks.

Written by [Measure-TrexShape](../axes/#measure-trexshape).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Group` | string | The group the token shapes were read under, empty for the lexer's own. |
| `Tokens` | long | How many tokens the axis read, whitespace left out. |
| `DominantPeriod` | int | The period of the strongest repetition, in tokens; 0 where nothing repeats. |
| `DominantStrength` | float | How strongly the dominant period repeats, from 0 to 1. |
| `Regions` | [Trex.ShapeRegion](#trexshaperegion)[] | The runs whose shapes repeat with a strength of at least -TemplateStrength, 0.6 by default, each ending at the last token that repeats the kind one period before it. |
| `ChangePoints` | long[] | Where the silhouette breaks, in UTF-16 code units. |
| `Frames` | [Trex.ShapeFrame](#trexshapeframe)[] | Every token's frame, with -Detail. |

### Trex.SpectralFrame

One frame of the spectral reading: the byte statistics of a window.

Inside another object, and as a string, it shows as `{Texture} at {Offset}`.

Held in [Trex.SpectralReport](#trexspectralreport).Frames.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | The last byte the frame reads, in UTF-16 code units. |
| `Texture` | [Trex.Texture](#trextexture) | The window's texture. |
| `Entropy` | float | The window's byte entropy, from 0 to 1. |
| `Period` | int | The period its bytes repeat at, in bytes; 0 where none. |
| `Novelty` | float | How new its runs of bytes are, from 0 to 1, 1 for a first sighting. |
| `Mix` | float[] | The shares of digits, letters, whitespace, punctuation, and control bytes or bytes not part of a well-formed UTF-8 character, among its recent bytes, in that order. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Texture} at {Offset}`, each braced name read from that property. |

### Trex.SpectralPeriod

A byte period the spectral reading found, and how strongly it holds.

Inside another object, and as a string, it shows as `{Period} ({Strength})`.

Held in [Trex.SpectralReport](#trexspectralreport).Periods.

| Property | Type | Holds |
|---|---|---|
| `Period` | int | The period, in bytes. |
| `Strength` | float | Its strongest showing, from 0 to 1. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Period} ({Strength})`, each braced name read from that property. |

### Trex.SpectralReport

The spectral axis over one input: its byte statistics along its length, where they change, and the texture of each stretch between.

Written by [Measure-TrexSpectral](../axes/#measure-trexspectral).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Hop` | long | How many bytes each frame advances. |
| `ChangePoints` | long[] | Where the byte statistics change, in UTF-16 code units. |
| `EntropyMinimum` | object | The least byte entropy of any frame, from 0 to 1; `$null` for an input with no frames. |
| `EntropyMean` | object | The mean byte entropy over the frames; `$null` for an input with no frames. |
| `EntropyMaximum` | object | The greatest byte entropy of any frame; `$null` for an input with no frames. |
| `Periods` | [Trex.SpectralPeriod](#trexspectralperiod)[] | The distinct byte periods the frames found, strongest first. |
| `Timeline` | [Trex.TextureRegion](#trextextureregion)[] | The input cut at its change points, each stretch with its texture. |
| `Regions` | [Trex.ClassifiedRegion](#trexclassifiedregion)[] | The input's regions classified by texture and token shape, with -Classify. |
| `Frames` | [Trex.SpectralFrame](#trexspectralframe)[] | Every frame, with -Detail. |

### Trex.StreamMatch

One match a stream scanner committed.

Returned by [Trex.StreamScanner](#trexstreamscanner).Finish, [Trex.StreamScanner](#trexstreamscanner).Push.

| Property | Type | Holds |
|---|---|---|
| `Start` | long | Where the match starts in the whole stream, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units it spans. |
| `Text` | string | The matched text. |
| `Pattern` | string | The pattern of a set that made the match, as it was given; empty for a scanner of one pattern. |

### Trex.StreamScanner

A scan fed a stream a chunk at a time. A proxy class.

Written by [New-TrexStreamScanner](../streams/#new-trexstreamscanner).

| Property | Type | Holds |
|---|---|---|
| `Source` | string | The patterns scanned, as given, one a line. |
| `IsDisposed` | bool | Whether `Dispose()` has freed the value behind the object; a property read after that is `$null`. |

| Method | Returns | Does |
|---|---|---|
| `[Trex.StreamScanner]::new(string pattern)` | [Trex.StreamScanner](#trexstreamscanner) | A scanner of one pattern, compiled with the atoms TREX ships; New-TrexStreamScanner takes several as a set. |
| `Push(string chunk)` | [Trex.StreamMatch](#trexstreammatch)[] | Feeds the next chunk of the stream and gives back the matches no later chunk can change. |
| `Finish()` | [Trex.StreamMatch](#trexstreammatch)[] | Ends the stream and gives back every match not given yet. |

### Trex.StressFrame

One token's stress reading.

Inside another object, and as a string, it shows as `{Text} at {Offset}, depth {Depth}`.

Held in [Trex.StressReport](#trexstressreport).Fractures, [Trex.StressReport](#trexstressreport).Frames, [Trex.StressReport](#trexstressreport).Peaks.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the token starts, in UTF-16 code units. |
| `Text` | string | The token's text. |
| `Depth` | int | How many paired brackets enclose the token. |
| `Strain` | float | How many tokens the innermost open bracket has been held open. |
| `Load` | float | How many tokens every open bracket has been held open, summed. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Text} at {Offset}, depth {Depth}`, each braced name read from that property. |

### Trex.StressReport

The stress axis over one input: how deeply the brackets nest at each token, the peaks of that nesting, and where it releases at once.

Written by [Measure-TrexStress](../axes/#measure-trexstress).

| Property | Type | Holds |
|---|---|---|
| `Path` | string | The file read, empty for a string. |
| `Length` | long | The input's length in UTF-16 code units. |
| `Tokens` | long | How many tokens the axis read, whitespace left out. |
| `MaxDepth` | int | The deepest nesting reached. |
| `PeakLoad` | object | The token carrying the greatest load, a Trex.StressFrame; `$null` for an input with no tokens. |
| `Peaks` | [Trex.StressFrame](#trexstressframe)[] | The tokens at a local maximum of depth, at least -PeakMinDepth deep, two by default. |
| `Fractures` | [Trex.StressFrame](#trexstressframe)[] | The tokens where the depth starts to fall from a level at least -FractureMinDepth deep, two by default: where a nested structure begins to close. |
| `Frames` | [Trex.StressFrame](#trexstressframe)[] | Every token's frame, with -Detail. |

### Trex.TextureRegion

A stretch of an input between two change points, with its texture.

Inside another object, and as a string, it shows as `{Texture} at {Offset}, {Length} long`.

Held in [Trex.SpectralReport](#trexspectralreport).Timeline.

| Property | Type | Holds |
|---|---|---|
| `Offset` | long | Where the stretch starts, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units it spans. |
| `Texture` | [Trex.Texture](#trextexture) | Its texture. |
| `Entropy` | float | Its byte entropy, from 0 to 1. |
| `Period` | int | The period its bytes repeat at, in bytes; 0 where none. |
| `Novelty` | float | How new its runs of bytes are, from 0 to 1. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Texture} at {Offset}, {Length} long`, each braced name read from that property. |

### Trex.Tiling

One way a run-together string tiles into dictionary words that the grammar accepts.

Inside another object, and as a string, it shows as `{Words}`.

Written by [Invoke-TrexGrammar](../streams/#invoke-trexgrammar).

Returned by [Trex.Grammar](#trexgrammar).Segment.

| Property | Type | Holds |
|---|---|---|
| `Words` | string[] | The words, in order; joined, they are the string. |
| `Probability` | double | The probability of the words' most probable parse under the grammar's `@p` weights. |
| `Parses` | ulong | How many parses of the words the grammar has. |

| Method | Returns | Does |
|---|---|---|
| `ToString()` | string | The object as text, `{Words}`, each braced name read from that property. |

### Trex.Token

One token as the lexer read it.

Written by [Get-TrexToken](../records/#get-trextoken).

| Property | Type | Holds |
|---|---|---|
| `Kind` | string | The token's kind: `number`, `word`, `quoted`, `ip`, `email`, `timestamp` and the rest, `open` and `close` for a bracket, or the name a declared shape or kind gave it. |
| `Text` | string | The token's text. |
| `Start` | long | Where the token starts in the input, in UTF-16 code units. |
| `Length` | long | How many UTF-16 code units the token spans. |
| `Value` | object | The text parsed as its kind, as a capture's Value reads it; `$null` where the kind carries none. |

### Trex.Group

One group of the matches `Group-TrexMatch` read: a PSObject with this type name, whose aggregate properties are the ones asked for, in this order.

Written by [Group-TrexMatch](../grouping/#group-trexmatch).

| Property | Type | Holds |
|---|---|---|
| `Key` | string | The key the group's matches rendered. |
| `Count` | long | How many matches rendered it. |
| `Sum` | the kind's type | With -Sum, the sum of the register's values: a `long` or `decimal` for numbers, a `TimeSpan` for durations. |
| `Average` | the kind's type | With -Average, their mean, computed exactly: a `decimal` for numbers, a `TimeSpan` for durations. |
| `Minimum` | the kind's type | With -Minimum, the least value, in the .NET type of its kind. |
| `Maximum` | the kind's type | With -Maximum, the greatest value, in the .NET type of its kind. |
| `P50, P95, ...` | the kind's type | With -Percentile, one property per percentile asked for, named `P` and the percentile. |
| `Sum_t, P95_t, ...` | the kind's type | Where an aggregate names several registers, one property for each in place of the one above, named after the aggregate and the register: `-Sum t, size` writes `Sum_t` and `Sum_size`. |

## Enumerations

### Trex.AtomForm

What a declaration declares.

| Value | Means |
|---|---|
| `Shape` | A bounded byte-pattern the lexer tries before its built-in recognizers, so where it matches it wins. |
| `ShapeAfter` | A bounded byte-pattern the lexer tries only where no built-in recognizer matched. |
| `Kind` | A pattern whose every match, after the lex, becomes one token of this kind. |
| `Pattern` | A pattern read in place wherever the name appears. |
| `Rule` | A named pattern with a message and a severity, reported by a rule scan and read as a pattern elsewhere. |

### Trex.Backend

The engine a scan runs on.

| Value | Means |
|---|---|
| `Auto` | Chosen per input: the CPU engine, or for a pattern that reads only token kinds where a device is present, the input split between the cores and the device, by what this process has measured at the input's size. |
| `Cpu` | The CPU engine, never probing the device. |
| `Gpu` | The device, falling back to the CPU where the pattern is outside what it runs, no device is present, or the build carries none. |

### Trex.ColorDepth

The depth -Color and -Passthru paint at, as the `trex` command's `--color` names it.

| Value | Means |
|---|---|
| `None` | No color: the report's text alone, as `--color never` prints it. |
| `Ansi16` | The sixteen base colors, as `--color 16` or `--color ansi` paints. |
| `Ansi256` | The 256-color table, as `--color 256` or `--color ansi256` paints. |
| `TrueColor` | Any 24-bit value, as `--color truecolor` paints. |

### Trex.ContextFold

The context a token is read against, as `\N{>+1:F}` names it, and the unit rung's two.

| Value | Means |
|---|---|
| `Window` | The window of tokens before the token. |
| `Unit` | The supertoken holding the token. |
| `Units` | The window of supertokens ending at the one holding the token. |
| `Enclosing` | The heads of the brackets enclosing the token. |
| `Echo` | The earlier occurrences of the token's key. |
| `Regime` | The tokens since the last spectral change point. |
| `Phase` | The earlier tokens at the token's column of the record period. |
| `Key` | The values the token's key was bound to before. |

### Trex.DateOrder

Which field of an all-numeric slash date such as `05/09/2026` is the day.

| Value | Means |
|---|---|
| `DayFirst` | `05/09/2026` is the 5th of September. |
| `MonthFirst` | `05/09/2026` is the 9th of May. |

### Trex.DurationUnit

The unit -Json writes a duration's value in, as the `trex` command's `--duration-unit` names it.

| Value | Means |
|---|---|
| `Nanoseconds` | The nanoseconds a duration is held and compared in. |
| `Milliseconds` | Milliseconds, by an exact decimal shift. |
| `Seconds` | Seconds, by an exact decimal shift: `1500ms` is `1.5`. |

### Trex.FilterKind

Which filter a prefilter is.

| Value | Means |
|---|---|
| `Bloom` | A Bloom filter: each n-gram sets bits chosen by several hashes, and an unset bit proves absence. |
| `Cuckoo` | A cuckoo filter: each n-gram a one-byte fingerprint in one of two buckets. |
| `Xor` | An xor filter: each n-gram's fingerprint the xor of three slots, filled by peeling. |

### Trex.FlowDirection

Which way a flow's trend runs at a token.

| Value | Means |
|---|---|
| `Steady` | Neither rising nor falling. |
| `Rising` | Rising. |
| `Falling` | Falling. |

### Trex.FlowSignal

The per-token signal the flow axis reads the trend of.

| Value | Means |
|---|---|
| `Magnitude` | Each token's order of magnitude, as the magnitude axis reads it. |
| `Stress` | Each token's nesting depth, as the stress axis reads it. |
| `Length` | Each token's length in bytes. |

### Trex.Grain

The stream an axis reads: the bytes, the significant tokens, or the supertokens they group into, named as a pattern's grain suffix names them.

| Value | Means |
|---|---|
| `Byte` | The bytes. |
| `Token` | The significant tokens, whitespace left out. |
| `Super` | The supertokens: runs of tokens read as one unit with a structural role. |

### Trex.GroupOrder

The order groups are written in.

| Value | Means |
|---|---|
| `Count` | Most matches first, and by key among equal counts. |
| `Key` | By key. |

### Trex.PercentileMethod

How a percentile is decided when its rank is between two observed values.

| Value | Means |
|---|---|
| `Nearest` | The value at rank ceil(p * n), always one that occurred. |
| `Linear` | Interpolated between the two neighbors; numeric kinds only. |
| `Lower` | The value at or below the rank. |
| `Hybrid` | Interpolated where the kind allows it, the nearest value otherwise. |

### Trex.RegionKind

What a region of an input is, by its texture and by whether its token shapes repeat.

| Value | Means |
|---|---|
| `Table` | Token shapes that repeat at a strong period: a table or a list of records, whatever its bytes look like. |
| `Blob` | A high-entropy run: packed, encoded or encrypted bytes. |
| `Prose` | Prose. |
| `Numeric` | Numbers. |
| `Code` | Code whose shapes do not repeat strongly. |
| `Mixed` | No texture dominates. |

### Trex.RelationKind

The relation an edge between two tokens carries.

| Value | Means |
|---|---|
| `Encloses` | From the word heading a bracket group to a token inside it. |
| `Operator` | From one operand of a binding punctuation to the other, in order. |
| `Adjacent` | From a significant token to the next. |

### Trex.Severity

How serious a rule's finding is.

| Value | Means |
|---|---|
| `Warning` | Worth a look; what a rule is where it names no severity. |
| `Error` | A fault; a scan with one fails in the `trex` command. |
| `Note` | For information. |

### Trex.SortKey

What -Sort orders the files read by.

| Value | Means |
|---|---|
| `Path` | The path, as text. |
| `Modified` | The time the file was last written. |
| `Accessed` | The time the file was last read. |
| `Created` | The time the file was created. |

### Trex.Texture

The texture a stretch of bytes has, by the classes of its bytes.

| Value | Means |
|---|---|
| `Prose` | Letters dominate and punctuation is low: prose. |
| `Code` | Punctuation and symbols high beside identifiers: code. |
| `Math` | Single symbols and digits mixed: mathematics. |
| `Data` | High entropy, allowing for the multi-byte characters it holds, or many bytes not part of a well-formed UTF-8 character: compressed, packed or binary data. |
| `Mixed` | No class dominates. |

### Trex.ValueSpelling

How -Json spells a typed register's value, as the `trex` command's `--values` names it.

| Value | Means |
|---|---|
| `Exact` | A JSON number only where a double holds the value exactly; a string or a small object everywhere else. |
| `Natural` | A JSON number throughout, rounding where a double must. |
| `Tagged` | `{"kind": ..., "exact": ...}` for every value. |
