---
title: Streams, grammars, subwords and prefilters
linkTitle: Streams and grammars
weight: 80
---

A scan of input that arrives in pieces, token grammars (`trex grammar`), byte-pair subword
encoders (`trex bpe`) and presence filters (`trex prefilter`), each an object with methods as
well as a set of cmdlets. The examples are on [tools](../../tools/).

### New-TrexStreamScanner

Begins a scan of input that arrives in pieces: a Trex.StreamScanner whose Push method takes each chunk and gives back the matches no later chunk can change, and whose Finish method gives back the rest.

The matches over the whole stream are the ones Select-TrexMatch finds in it at once, each at its offset into the whole stream in UTF-16 code units, and the scanner holds only the part of the stream a match may still reach. Several patterns scan as one set, each match naming its pattern. The patterns are compiled with the atoms trex ships, since the stream is lexed as it arrives, with no declarations in force.

Alias: `New-TxStreamScanner`

```powershell
New-TrexStreamScanner [-Pattern] <string[]> [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Pattern` | string[] |  | The patterns to scan for, as trex source text; several scan as one set. |

Writes [Trex.StreamScanner](../types/#trexstreamscanner).

### New-TrexGrammar

Compiles a token grammar for Invoke-TrexGrammar and as an object with Parse, Test, CountParses, BestProbability, TotalProbability, Segment, CountSegmentations and BestSegmentationProbability methods.

A grammar is one rule a line, `name := alternatives`, alternatives separated by `|`: `<rule>` another rule, `"lit"` a token by its text, `number`, `ident`, `string`, `ip`, `url`, `email`, `time` or `punct` a token by its kind, `*`, `+` and `?` after a symbol its repetition, `( a | b )` a group, and `@p` after an alternative its weight, 1 when absent. `#` starts a comment. A rule that begins with itself is left recursive, which is how an operator grammar writes precedence. The first rule is the start unless -Start names another.

Alias: `New-TxGrammar`

```powershell
New-TrexGrammar [-Source] <string> [-Start <string>] [<CommonParameters>]
New-TrexGrammar -Path <string> [-Start <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Path` | string |  | A file holding the grammar. |
| `-Source` | string | by value | The grammar's source text. |
| `-Start` | string |  | The rule a parse starts from, in place of the first one declared. |

Writes [Trex.Grammar](../types/#trexgrammar).

### Invoke-TrexGrammar

Parses text against a token grammar and writes the parse tree, or with -Count, -Best or -Probability a value over every derivation.

The parse is one ordered-choice parse from the start rule, written as a Trex.ParseNode; text that does not parse in full is an error record. -Count writes how many derivations the grammar has of the text, which says how ambiguous it is; -Best the probability of the most probable one under the grammar's `@p` weights, which resolves the ambiguity; and -Probability all of them together. -Segment reads a run-together string over the words of -Dictionary, every way it tiles into them, and writes each tiling the grammar accepts as a Trex.Tiling, most probable first; with -Count how many there are, or with -Best the probability of the most probable one.

Each input gets one answer, in the order read. The count and the probabilities come from a chart cubic in the token count, so a long text is parsed a sentence at a time.

Alias: `Invoke-TxGrammar`

```powershell
Invoke-TrexGrammar [-Grammar] <Object> [-InputObject] <string> [-Start <string>] [-Count] [-Best] [-Probability] [<CommonParameters>]
Invoke-TrexGrammar [-Grammar] <Object> -Path <string[]> [-Start <string>] [-Count] [-Best] [-Probability] [<CommonParameters>]
Invoke-TrexGrammar [-Grammar] <Object> -LiteralPath <string[]> [-Start <string>] [-Count] [-Best] [-Probability] [<CommonParameters>]
Invoke-TrexGrammar [-Grammar] <Object> -Segment <string> -Dictionary <string[]> [-Start <string>] [-Count] [-Best] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Best` | switch |  | Writes the probability of each input's most probable derivation, or with -Segment of its most probable tiling. |
| `-Count` | switch |  | Writes how many derivations the grammar has of each input, or with -Segment how many tilings it accepts. |
| `-Dictionary` | string[] |  | The words -Segment tiles into. |
| `-Grammar` | object |  | The grammar: a Trex.Grammar from New-TrexGrammar, or grammar source text. |
| `-InputObject` | string | by value | The text to parse; each string piped in is parsed on its own. |
| `-LiteralPath` | string[] | by name | Files to parse, read as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files to parse, each whole; wildcards expand. |
| `-Probability` | switch |  | Writes the probability of all of each input's derivations together. |
| `-Segment` | string |  | A run-together string to tile into the words of -Dictionary. |
| `-Start` | string |  | The rule a parse starts from, in place of the grammar's own. |

Writes [Trex.ParseNode](../types/#trexparsenode), [Trex.Tiling](../types/#trextiling), `ulong`, `double`.

### New-TrexBpe

Learns a byte-pair encoder from a corpus.

Each word of the corpus is its characters and an end-of-word mark, and every round merges the most frequent adjacent pair of symbols, ties broken by the pair's text, so the same corpus always learns the same model. The files named, or the strings piped in, are read as one corpus. -MaxBytes trains on the start of a large corpus, cut back to a whole word.

Alias: `New-TxBpe`

```powershell
New-TrexBpe [-Path] <string[]> [-Merges <uint>] [-MaxBytes <ulong>] [<CommonParameters>]
New-TrexBpe -LiteralPath <string[]> [-Merges <uint>] [-MaxBytes <ulong>] [<CommonParameters>]
New-TrexBpe -InputObject <string> [-Merges <uint>] [-MaxBytes <ulong>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-InputObject` | string | by value | The corpus as text; every string piped in is read into the one corpus. |
| `-LiteralPath` | string[] | by name | The corpus files, read as written, as Get-ChildItem pipes them. |
| `-MaxBytes` | ulong |  | Trains on at most this many bytes from the start of the corpus, cut back to the end of a word. |
| `-Merges` | uint |  | How many merges to learn: 1000 when absent. |
| `-Path` | string[] |  | The corpus files; wildcards expand, and every file is read into the one corpus. |

Writes [Trex.Bpe](../types/#trexbpe).

### Import-TrexBpe

Reads a byte-pair encoder from a model file, one Export-TrexBpe or the trex command's `bpe train` wrote.

A model line is two symbols separated by a tab, and a line that is not is an error naming its number rather than a merge left out.

Alias: `Import-TxBpe`

```powershell
Import-TrexBpe [-Path] <string[]> [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Path` | string[] | by value | The model files; wildcards expand. |

Writes [Trex.Bpe](../types/#trexbpe).

### Export-TrexBpe

Writes a byte-pair encoder's model to a file, one merge a line with its two symbols separated by a tab, which Import-TrexBpe and the trex command's `bpe encode --model` read.

Alias: `Export-TxBpe`

```powershell
Export-TrexBpe [-Path] <string> -Bpe <Bpe> [-WhatIf] [-Confirm] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Bpe` | [Trex.Bpe](../types/#trexbpe) | by value | The encoder whose model to write. |
| `-Path` | string |  | The file to write. |

### ConvertTo-TrexBpe

Splits text into a byte-pair encoder's subwords, writing each subword.

Each word is split on its own, the last subword of a word ending in `</w>`, so joining a word's subwords and dropping the mark gives the word back.

Alias: `ConvertTo-TxBpe`

```powershell
ConvertTo-TrexBpe [-Bpe] <Bpe> [-InputObject] <string> [<CommonParameters>]
ConvertTo-TrexBpe [-Bpe] <Bpe> -Path <string[]> [<CommonParameters>]
ConvertTo-TrexBpe [-Bpe] <Bpe> -LiteralPath <string[]> [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Bpe` | [Trex.Bpe](../types/#trexbpe) |  | The encoder: a Trex.Bpe from New-TrexBpe or Import-TrexBpe. |
| `-InputObject` | string | by value | The text to split; each string piped in is split on its own. |
| `-LiteralPath` | string[] | by name | Files to split, read as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | Files to split, each whole; wildcards expand. |

Writes `string`.

### New-TrexPrefilter

Builds a presence filter over a corpus, as an object whose MightContain method answers whether a literal might occur in it.

The files named, or the strings piped in, are read as one corpus. The filter keeps no copy of it, so the object leaves a yes unsettled, where Test-TrexPrefilter settles each with an exact search.

Alias: `New-TxPrefilter`

```powershell
New-TrexPrefilter [-Path] <string[]> [-Filter <FilterKind>] [<CommonParameters>]
New-TrexPrefilter -LiteralPath <string[]> [-Filter <FilterKind>] [<CommonParameters>]
New-TrexPrefilter -InputObject <string> [-Filter <FilterKind>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Filter` | [Trex.FilterKind](../types/#trexfilterkind) |  | Which filter to build: Bloom when absent. |
| `-InputObject` | string | by value | The corpus as text; every string piped in is read into the one corpus. |
| `-LiteralPath` | string[] | by name | The corpus files, read as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | The corpus files; wildcards expand, and every file is read into the one corpus. |

Writes [Trex.Prefilter](../types/#trexprefilter).

### Test-TrexPrefilter

Tests literals against a presence filter built over a corpus, or with -Verify every filter's contract over it.

Each literal gets a Trex.LiteralTest: whether the filter says it might occur, and whether an exact search finds it, so a false positive reads as one. -Verify probes all three filters with substrings of the corpus, which each must report as present, and with byte strings found nowhere in it, the share of which a filter rejects being the scanning it saves; it writes a Trex.FilterCheck for each.

Alias: `Test-TxPrefilter`

```powershell
Test-TrexPrefilter [-Path] <string[]> [-Literal <string[]>] [-Filter <FilterKind>] [-Verify] [<CommonParameters>]
Test-TrexPrefilter -LiteralPath <string[]> [-Literal <string[]>] [-Filter <FilterKind>] [-Verify] [<CommonParameters>]
Test-TrexPrefilter -InputObject <string> [-Literal <string[]>] [-Filter <FilterKind>] [-Verify] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Filter` | [Trex.FilterKind](../types/#trexfilterkind) |  | Which filter tests the literals: Bloom when absent. |
| `-InputObject` | string | by value | The corpus as text; every string piped in is read into the one corpus. |
| `-Literal` | string[] |  | The literals to test. |
| `-LiteralPath` | string[] | by name | The corpus files, read as written, as Get-ChildItem pipes them. |
| `-Path` | string[] |  | The corpus files; wildcards expand, and every file is read into the one corpus. |
| `-Verify` | switch |  | Probes the contract of all three filters over the corpus. |

Writes [Trex.LiteralTest](../types/#trexliteraltest), [Trex.FilterCheck](../types/#trexfiltercheck).
