---
title: Tools
linkTitle: Tools
weight: 100
---

The rest of TREX: indexes that let a tree scan skip files, scans of input that arrives in
pieces, token grammars, byte-pair subword encoders, presence filters and the compressor. The
flags and parameters are on the [CLI](../cli/), [Python](../python/) and
[PowerShell](../powershell/streams/) pages.

## Indexes

An index is one file, `.trex-index`, at a tree's root, holding a summary of each file: the token
kinds its lex made, a filter over the words it holds, the range its numbers span and the range its
timestamps span. A scan of the tree skips any file every one of its patterns is refused by, which
skips the read, the lex and the walk together. A summary says "cannot match" only where that is
certain: the kind mask and the ranges are exact, the word filter's only error is a false present,
and an alternation or a `P*` requires nothing. An entry records its file's length and modified
time, and a file that differs, or that the index has never seen, is scanned. An index changes how
long a scan takes and never what it reports.

Measured over TREX's own source in `benches/index_pruning.rs` (70 files, 3.2 MB): 50x for `\E`,
23x for `\T`, 12x for `\I`, 9x for `\N{>=100000}`, 2x for a rare word, and 0.99 to 1.02x for a
pattern present everywhere. The index is 320 bytes a file, 0.0071x the tree it covers.

```console
$ cat tree/logs/a.log
from 10.0.0.1 at 09:14
retry once
$ cat tree/logs/b.log
queue drained
nothing to report
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex index tree/logs/
tree/logs/: 2 files indexed

$ trex index tree/logs/ --list
tree/logs/.trex-index: 2 files indexed

$ trex scan '\I' tree/logs/
tree/logs/a.log:1:6: "10.0.0.1"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::index::{Needs, Summary};

let a = Summary::of(b"from 10.0.0.1 at 09:14\nretry once\n");
let b = Summary::of(b"queue drained\nnothing to report\n");
let needs = Needs::of(&trex::parse(r"\I").expect("valid pattern"));
assert!(!needs.refused_by(&a));
assert!(needs.refused_by(&b));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> index = trex.Index.build("tree/logs/")
>>> len(index)
2
>>> index.candidates(r"\I")
['tree/logs/a.log']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> New-TrexIndex ./tree/logs | Select-Object Files

Files
-----
    2

PS> Select-TrexMatch '\I' -Path ./tree/logs | Select-Object LineNumber, Text

LineNumber Text
---------- ----
         1 10.0.0.1
```
{{< /tab >}}
{{< /tabs >}}

A scan of a tree holding an index uses it without being asked; `--no-index` reads none, and
`scan --index` builds one as the scan reads the files. A file named on the command line is
scanned whatever the index says, and `-v`, `-L` and `--passthru` read no index. A file changed
since the index was built is scanned, so the report follows the tree:

```console
$ cat tree/logs/b.log
queue drained at 10.0.0.9
nothing to report
$ trex scan '\I' tree/logs/
tree/logs/a.log:1:6: "10.0.0.1"
tree/logs/b.log:1:18: "10.0.0.9"
```

In Rust `index::Index::build` summarizes a tree's files, `save` and `load` write and read
`.trex-index`, and `refuses` answers for one file and one pattern. In Python `trex.Index.build`
writes the index, `Index.load` reads one, and `candidates` lists the files a scan of a pattern
must read.

## Streams

A stream scanner takes input in chunks and gives back the matches no later chunk can change, and
the rest when the stream ends, so the matches over the whole stream are those of one scan over
all of it. A chunk ending inside a token holds its match until the token ends. With the standard
input alone and the plain report, `scan` reads the stream as it arrives and prints each match
the moment it commits, so `tail -f app.log | trex scan '\T:t \W \T{>+1h:t}'` reports a gap as
soon as the line that closes it ends; `--json` on a stream prints one object a line.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ printf 'mail bob@x.com\nand ann@y.org now\nthen ted@z.net\n' | trex scan '\E'
[5..14] "bob@x.com"
[19..28] "ann@y.org"
[38..47] "ted@z.net"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut scan = trex::StreamScanner::new(trex::parse(r"\E").expect("valid pattern"));
scan.push(b"mail bob@x.com\nand ann@y.");
let first = scan.drain_committed();
scan.push(b"org now\nthen ted@z.net");
let second = scan.drain_committed();
let rest = scan.finish();
let at = |s: &[trex::Span]| s.iter().map(|s| s.start()).collect::<Vec<_>>();
assert_eq!((at(&first), at(&second), at(&rest)), (vec![5], vec![19], vec![38]));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> st = trex.StreamScanner(trex.Pattern(r"\E"))
>>> st.push(b"mail bob@x.com\nand ann@y."), st.push(b"org now\nthen ted@z.net"), st.finish()
([(5, 14)], [(19, 28)], [(38, 47)])
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $scan = New-TrexStreamScanner '\E'
PS> $scan.Push("mail bob@x.com`nand ann@y.")

Start Length Text      Pattern
----- ------ ----      -------
5     9      bob@x.com

PS> $scan.Push("org now`nthen ted@z.net")

Start Length Text      Pattern
----- ------ ----      -------
19    9      ann@y.org

PS> $scan.Finish()

Start Length Text      Pattern
----- ------ ----      -------
38    9      ted@z.net
```
{{< /tab >}}
{{< /tabs >}}

A pattern that cannot commit a match before the end of the stream is scanned when the stream
ends, and the command says on the standard error how many bytes it retained: one with a content
guard, a lookahead, a lookbehind reaching before its match, a field anchor or a whole-stream
axis, and one with no bounded length that the set engine walks, such as a balanced group. An
unbounded pattern the single-pass engine walks commits early, keeping only the bytes some attempt
still running can reach back over. The stream cuts only just after a newline, so `^` and `$`
commit as their line ends. `--chunk-size N` feeds a file in N-byte chunks through the same
scanner.

Several patterns stream as one set through one window, the most conservative member deciding
what commits, each match naming its member. On the command line the set is a pattern file read
by `--patterns`, its members numbered by line:

```console
$ cat pair.trex
\E
\I
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ printf 'from 10.0.0.1 to bob@x.com' | trex scan --patterns pair.trex
[5..13] "10.0.0.1"  pattern: 2
[17..26] "bob@x.com"  pattern: 1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let set = trex::PatternSet::new(vec![trex::parse(r"\E").expect("valid"), trex::parse(r"\I").expect("valid")]);
let mut scan = trex::StreamScanner::over_set(set);
scan.push(b"from 10.0.0.1 to bob@");
scan.push(b"x.com");
let found: Vec<(usize, usize)> = scan.finish_with_members().iter().map(|(m, s)| (*m, s.start())).collect();
assert_eq!(found, [(1, 5), (0, 17)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> st = trex.StreamScanner(trex.PatternSet([r"\E", r"\I"]))
>>> st.push(b"from 10.0.0.1 to bob@") + st.push(b"x.com") + st.finish()
[(1, 5, 13), (0, 17, 26)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $set = New-TrexStreamScanner '\E', '\I'
PS> @($set.Push('from 10.0.0.1 to bob@')) + @($set.Push('x.com')) + @($set.Finish()) | Select-Object Pattern, Start, Text

Pattern Start Text
------- ----- ----
\I          5 10.0.0.1
\E         17 bob@x.com
```
{{< /tab >}}
{{< /tabs >}}

## Grammars

A token grammar is named rules over the token stream, with no per-language parser. A rule is
`name := alt | alt`, one a line: `<name>` references another rule, `"lit"` matches a token by its
text, a kind keyword matches a token by its kind (`number`, `ident`, `string`, `ip`, `url`,
`email`, `time`, `punct`), any symbol takes `*`, `+` or `?`, `( a b | c )` is a group, and `@p`
after an alternative weights it, 1 where absent. `#` starts a comment. A rule that begins with
itself is left recursive, which is how an operator grammar writes precedence and associativity.
The first rule is the start unless `--start` names another.

```console
$ cat arith.grammar
expr   := <expr> "+" <term> | <expr> "-" <term> | <term>
term   := <term> "*" <factor> | <term> "/" <factor> | <factor>
factor := number | ident | "(" <expr> ")"
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex grammar arith.grammar --text '2 + 3 * 4'
(expr 2 + (term 3 * 4))

$ trex grammar arith.grammar --text '2 - 3 - 4'
(expr (expr 2 - 3) - 4)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let grammar = trex::Grammar::parse(
    "expr   := <expr> \"+\" <term> | <expr> \"-\" <term> | <term>
term   := <term> \"*\" <factor> | <term> \"/\" <factor> | <factor>
factor := number | ident | \"(\" <expr> \")\"",
)
.expect("a grammar");
let tree = grammar.parse_input(b"2 + 3 * 4").expect("a parse");
assert_eq!(tree.sexpr(), "(expr 2 + (term 3 * 4))");
assert!(!grammar.recognizes(b"2 +"));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> arith = trex.Grammar(trex.read("arith.grammar"))
>>> arith.parse("2 + 3 * 4").expression
'(expr 2 + (term 3 * 4))'
>>> arith.parse("2 - 3 - 4").expression
'(expr (expr 2 - 3) - 4)'
>>> arith.accepts("2 +")
False
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $arith = New-TrexGrammar -Path ./arith.grammar
PS> (Invoke-TrexGrammar $arith '2 + 3 * 4').Expression
(expr 2 + (term 3 * 4))

PS> $arith.Parse('2 - 3 - 4').Expression
(expr (expr 2 - 3) - 4)

PS> $arith.Test('2 +')
False
```
{{< /tab >}}
{{< /tabs >}}

`--count` prints the number of derivations, which says how ambiguous the grammar is over the
input; `--best` the probability of the most probable one under the `@p` weights, and `--prob`
the total over all of them. A count or a probability of zero exits non-zero.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex grammar --grammar-text 'expr := <expr> "+" <expr> | number' --text '1 + 2 + 3 + 4' --count
5

$ trex grammar --grammar-text 'expr := <expr> "+" <expr> @0.5 | number @0.5' --text '1 + 2 + 3' --best
0.03125

$ trex grammar --grammar-text 'expr := <expr> "+" <expr> @0.5 | number @0.5' --text '1 + 2 + 3' --prob
0.0625
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let ambiguous = trex::Grammar::parse(r#"expr := <expr> "+" <expr> | number"#).expect("a grammar");
assert_eq!(ambiguous.count_parses(b"1 + 2 + 3 + 4"), 5);
let weighted = trex::Grammar::parse(r#"expr := <expr> "+" <expr> @0.5 | number @0.5"#).expect("a grammar");
assert_eq!(weighted.best_parse_prob(b"1 + 2 + 3"), 0.03125);
assert_eq!(weighted.total_prob(b"1 + 2 + 3"), 0.0625);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Grammar('expr := <expr> "+" <expr> | number').count("1 + 2 + 3 + 4")
5
>>> weighted = trex.Grammar('expr := <expr> "+" <expr> @0.5 | number @0.5')
>>> weighted.best("1 + 2 + 3"), weighted.probability("1 + 2 + 3")
(0.03125, 0.0625)
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Invoke-TrexGrammar 'expr := <expr> "+" <expr> | number' '1 + 2 + 3 + 4' -Count
5

PS> $weighted = New-TrexGrammar 'expr := <expr> "+" <expr> @0.5 | number @0.5'
PS> Invoke-TrexGrammar $weighted '1 + 2 + 3' -Best
0.03125

PS> Invoke-TrexGrammar $weighted '1 + 2 + 3' -Probability
0.0625
```
{{< /tab >}}
{{< /tabs >}}

### Segmentation

`--segment TEXT --dict FILE` reads a run-together string over the words of a dictionary, every
way it tiles into them, and parses each tiling: `--count` counts every parse of every tiling the
grammar accepts, so ambiguity in the tiling and in the parse count together; `--best` gives the
probability of the most probable; and `--list` lists each tiling the grammar accepts, most
probable first, with its words, its best parse's probability and its parse count.

```console
$ cat words.txt
a
b
ab
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex grammar --grammar-text 's := <s> ident | ident' --segment abab --dict words.txt --count
4

$ trex grammar --grammar-text 's := "ab" <s> | "ab"' --segment abab --dict words.txt --count
1

$ trex grammar --grammar-text 's := <s> ident @0.5 | ident @0.5' --segment abab --dict words.txt --list
ab ab	0.25	1
a b ab	0.125	1
ab a b	0.125	1
a b a b	0.0625	1
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let words = ["a".to_string(), "b".to_string(), "ab".to_string()];
let any = trex::Grammar::parse("s := <s> ident | ident").expect("a grammar");
assert_eq!(any.count_segmentations("abab", &words), 4);
let pairs = trex::Grammar::parse(r#"s := "ab" <s> | "ab""#).expect("a grammar");
assert_eq!(pairs.count_segmentations("abab", &words), 1);
let weighted = trex::Grammar::parse("s := <s> ident @0.5 | ident @0.5").expect("a grammar");
let tilings = weighted.segment("abab", &words);
let listed: Vec<String> = tilings.iter().map(|t| t.words.join(" ")).collect();
assert_eq!(listed, ["ab ab", "a b ab", "ab a b", "a b a b"]);
assert_eq!((tilings[0].probability, tilings[0].parses), (0.25, 1));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> words = ["a", "b", "ab"]
>>> trex.Grammar("s := <s> ident | ident").count_segmentations("abab", words)
4
>>> trex.Grammar('s := "ab" <s> | "ab"').count_segmentations("abab", words)
1
>>> weighted = trex.Grammar("s := <s> ident @0.5 | ident @0.5")
>>> [(" ".join(t.words), t.probability, t.parses) for t in weighted.segment("abab", words)]
[('ab ab', 0.25, 1), ('a b ab', 0.125, 1), ('ab a b', 0.125, 1), ('a b a b', 0.0625, 1)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Invoke-TrexGrammar 's := <s> ident | ident' -Segment 'abab' -Dictionary a, b, ab -Count
4

PS> Invoke-TrexGrammar 's := "ab" <s> | "ab"' -Segment 'abab' -Dictionary a, b, ab -Count
1

PS> Invoke-TrexGrammar 's := <s> ident @0.5 | ident @0.5' -Segment 'abab' -Dictionary a, b, ab

Words        Probability Parses
-----        ----------- ------
{ab, ab}     0.25        1
{a, b, ab}   0.125       1
{ab, a, b}   0.125       1
{a, b, a, b} 0.0625      1
```
{{< /tab >}}
{{< /tabs >}}

The listing agrees with the other two readings: its parse counts sum to what `--count` counts,
and its first probability is what `--best` gives. Tilings of one probability keep the order the
string reads them in. In PowerShell `-Segment` with neither `-Count` nor `-Best` writes a
`Trex.Tiling` for each tiling, as `Grammar.Segment(text, dictionary)` returns them; in Python
`Grammar.segment(text, dictionary)` returns a `trex.Tiling` for each, and in Rust
`Grammar::segment` a `grammar::Tiling`.

## Byte-pair encoders

A byte-pair encoder is learned from a corpus: it merges the most frequent adjacent pair of
symbols, a given number of times, and splits text into the subwords those merges make, `</w>`
marking the end of a word. A model is one merge a line, its two symbols separated by a tab.

```console
$ cat corpus.txt
the cat sat at the mat at the hat
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex bpe train corpus.txt --merges 5
trex bpe: learned 5 merges from 34 bytes in 0.0s
a	t
at	</w>
e	</w>
h	e</w>
t	he</w>
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let bpe = trex::bpe::Bpe::train(b"low low low low lower lower newest newest newest widest", 10);
assert_eq!(bpe.len(), 10);
assert_eq!(bpe.encode(b"lower").join(" "), "low er </w>");
let again = trex::bpe::Bpe::parse(&bpe.to_lines()).expect("a model");
assert_eq!(again.encode(b"lower"), bpe.encode(b"lower"));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> bpe = trex.Bpe.train("low low low low lower lower newest newest newest widest", merges=10)
>>> " ".join(bpe.encode("lower"))
'low er </w>'
>>> bpe.save("words.bpe")
>>> " ".join(trex.Bpe.load("words.bpe").encode("lower"))
'low er </w>'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $bpe = 'low low low low lower lower newest newest newest widest' | New-TrexBpe -Merges 10
PS> $bpe.Encode('lower') -join ' '
low er </w>

PS> Export-TrexBpe ./words.bpe -Bpe $bpe
PS> (Import-TrexBpe ./words.bpe).Encode('lower') -join ' '
low er </w>
```
{{< /tab >}}
{{< /tabs >}}

`bpe encode --model M` splits text with a model `bpe train` printed, and `--max-bytes N` trains on
a sample of the corpus. In PowerShell `ConvertTo-TrexBpe` writes each subword, and
`Import-TrexBpe` reads a model any surface wrote; in Python `Bpe.train(path=)` learns from files,
`max_bytes=` samples, and `Bpe.load` reads a model any surface wrote.

## Prefilters

A presence filter over a corpus's n-grams answers whether a literal might occur in it without
scanning it: a literal it rejects is absent, and one it passes might be present, which an exact
search confirms. A literal shorter than one n-gram is never rejected. The filters are a Bloom
filter (the default), a cuckoo filter and an xor filter.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex prefilter --text 'the quick brown fox ERROR here' --literal ERROR --literal MISSING
bloom: "ERROR" -> might occur; present (confirmed)
bloom: "MISSING" -> ABSENT (rejected with no corpus scan)
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::prefilter::{BloomFilter, Membership};

let filter = BloomFilter::build(b"the quick brown fox ERROR here");
assert!(filter.might_contain(b"ERROR"));
assert!(!filter.might_contain(b"MISSING"));
assert!(trex::prefilter::verify(b"the quick brown fox ERROR here").iter().all(|c| c.false_negatives == 0));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> pf = trex.Prefilter("the quick brown fox ERROR here")
>>> pf.might_contain("ERROR"), pf.might_contain("MISSING")
(True, False)
>>> [(t.literal, t.filter, t.might_occur, t.occurs) for t in pf.test("ERROR", "MISSING")]
[('ERROR', 'bloom', True, True), ('MISSING', 'bloom', False, False)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Test-TrexPrefilter -InputObject 'the quick brown fox ERROR here' -Literal ERROR, MISSING

Literal Filter MightOccur Occurs
------- ------ ---------- ------
ERROR   Bloom  True       True
MISSING Bloom  False      False
```
{{< /tab >}}
{{< /tabs >}}

`--filter bloom|cuckoo|xor` picks the filter; `--verify` probes every filter's contract over the
corpus - no literal present is ever rejected - and exits non-zero where one is broken. In
PowerShell `New-TrexPrefilter` writes the filter as an object with a `MightContain` method, and
`Test-TrexPrefilter -Verify` writes each filter's probes; in Python `trex.Prefilter(text, kind=)`
is the filter, and `verify()` gives each filter's probes.

## Compression

`compress` comes with the `compress` feature, the one TREX feature outside the default build; a
default binary leaves out the coder and the 21.6 MB prior it reads, and answers the command with
the build that has it:

```text
cargo build --release --features compress
```

It reports the code length the default context-mixing model, a logistic mix with the orbit
model using the baked prior, gives the input: the size is that length in bits rounded up to
bytes. No compressed stream is written and nothing decodes one.

```console
$ trex compress --text 'the quick brown fox jumps over the lazy dog the quick brown fox jumps'
trex compress: 69 bytes -> 15 bytes  (1.626 bits/byte, 21.7% of original)
  coder: logistic mix + orbit + baked prior   0.00 MB/s   (--compare for the full table)
```

`--compare` prints every coder's bits per byte and encode speed, the BPE baseline included;
`--no-baked` runs with no prior. Decoding the prior costs every run about 1.9 s and a gigabyte of
memory before the first byte is coded; `--prior-cache DIR` writes it once into DIR as the coder's
own tables (1.4 GB for the shipped prior, 2.4 s on an AMD Ryzen 9 7900X) and maps them on every later run in under
a millisecond, coding to the same bits. A read of a mapped table costs 1.4-1.7x a decoded one, so
the cache is faster on inputs under roughly 7-10 MB and the decode is faster above.

Three parallel backends trade ratio for speed. `--chunks N` slices the input across N cores (0
for every logical core), each chunk seeded from the baked prior; `--gpu` runs the device coder, one
CUDA thread per chunk, with `TREX_GPU_CHUNK` and `TREX_GPU_OVERLAP` setting its chunk size and
warmup; `--hybrid` runs the CPU and the device at once on a head and tail split. The sequential CPU
coder gives the best ratio, which is why none of the three is the default; the
[choose a backend](../../how-to/choose-a-backend/#compressing) how-to compares them.
