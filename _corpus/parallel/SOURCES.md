# The parallel corpus

Real text in sixteen natural languages and real code in twelve programming
languages, each set saying the same thing in every language: the Universal
Declaration of Human Rights in each translation, and the same ten benchmark
programs in each programming language. A reading compared across the
languages of a set compares scripts and encodings, not topics. The
measurement examples read it offline; `build.py` fetches every file again at
the revisions below, and `MANIFEST.sha256` holds each file's digest.

The folder is not part of the published crate.

## prose/ - the Universal Declaration of Human Rights

One translation a language, as plain text: every title and paragraph in
document order, one to a paragraph.

| File | Language | Script |
|---|---|---|
| `afr.txt` | Afrikaans | Latin |
| `arb.txt` | Arabic, Standard | Arabic |
| `bul.txt` | Bulgarian | Cyrillic |
| `cat.txt` | Catalan | Latin |
| `ces.txt` | Czech | Latin |
| `cmn.txt` | Chinese, Mandarin | Han (simplified) |
| `dan.txt` | Danish | Latin |
| `deu.txt` | German (1996 orthography) | Latin |
| `ell.txt` | Greek (monotonic) | Greek |
| `eng.txt` | English | Latin |
| `heb.txt` | Hebrew | Hebrew |
| `hin.txt` | Hindi | Devanagari |
| `jpn.txt` | Japanese | Han and kana |
| `kor.txt` | Korean | Hangul |
| `rus.txt` | Russian | Cyrillic |
| `tha.txt` | Thai | Thai |

Source: the UDHR in XML project, <https://github.com/eric-muller/udhr>,
`data/udhr/udhr_*.xml` at commit `588b3f4b2d0467aff54842a4b926551b69d5a66a`
(<http://efele.net/udhr>). The translations come from the UDHR Translation
Project of the Office of the United Nations High Commissioner for Human
Rights, <https://www.ohchr.org/en/human-rights/universal-declaration/universal-declaration-human-rights/about-universal-declaration-human-rights-translation-project>,
which asks that a reproduction name its website as the source.

## code/ - The Computer Language Benchmarks Game

The first implementation of binarytrees, fannkuchredux, fasta, knucleotide,
mandelbrot, nbody, pidigits, regexredux, revcomp and spectralnorm in C, C#,
Common Lisp (SBCL), Go, Haskell (GHC), Java, JavaScript (Node.js), Perl,
PHP, Python 3, Ruby and Rust, named `benchmark-language.extension`: 120
programs. The archive names a benchmark's first program in a language
`benchmark.language`, without a number, and the others
`benchmark.language-N.language`; where it holds no unnumbered program, the
lowest-numbered one is taken.

Source: <https://salsa.debian.org/benchmarksgame-team/benchmarksgame>,
`public/download/benchmarksgame-sourcecode.zip` at commit
`40296663ed350d5fe4a6ab5e367bab61cb77c219`. Under the BSD 3-Clause License,
copyright 2004-2008 Brent Fulgham and 2005-2024 Isaac Gouy, reproduced in
`code/LICENSE-benchmarksgame.md`.

## sql/ - SQLite

Eight SQL scripts from SQLite: `mandelbrot.sql` and `sudoku.sql` from
`ext/wasm/sql/`, the rest from `test/`.

Source: <https://github.com/sqlite/sqlite> at commit
`ccbdec8444e3a717dac8c154f17fdd7e80aa7827`. SQLite is in the public domain
(<https://www.sqlite.org/copyright.html>).
