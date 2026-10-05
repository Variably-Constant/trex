---
title: Parse with a token grammar
linkTitle: Parse with a token grammar
weight: 19
---

Parse input against named rules written over tokens rather than characters: `number` is a number
token, `ident` a word, `"+"` a literal, and a left-recursive rule gives an operator its
precedence. The grammar form is on [grammars](../../reference/tools/#grammars).

```console
$ cat arith.grammar
expr   := <expr> "+" <term> | <expr> "-" <term> | <term>
term   := <term> "*" <factor> | <term> "/" <factor> | <factor>
factor := number | ident | "(" <expr> ")"
```

## Parse and check

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex grammar arith.grammar --text '2 + 3 * 4'
(expr 2 + (term 3 * 4))
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
assert_eq!(grammar.parse_input(b"2 + 3 * 4").expect("a parse").sexpr(), "(expr 2 + (term 3 * 4))");
assert!(!grammar.recognizes(b"2 +"));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> arith = trex.Grammar(trex.read("arith.grammar"))
>>> arith.parse("2 + 3 * 4").expression
'(expr 2 + (term 3 * 4))'
>>> arith.accepts("2 +")
False
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $arith = New-TrexGrammar -Path ./arith.grammar
PS> $arith.Parse('2 + 3 * 4').Expression
(expr 2 + (term 3 * 4))

PS> $arith.Test('2 +')
False
```
{{< /tab >}}
{{< /tabs >}}

`*` binds tighter than `+` because `term` is below `expr`.

## Measure how ambiguous a grammar is

`--count` gives the number of derivations; a grammar with one rule for every sum reads
`1 + 2 + 3 + 4` five ways:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex grammar --grammar-text 'expr := <expr> "+" <expr> | number' --text '1 + 2 + 3 + 4' --count
5
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let ambiguous = trex::Grammar::parse(r#"expr := <expr> "+" <expr> | number"#).expect("a grammar");
assert_eq!(ambiguous.count_parses(b"1 + 2 + 3 + 4"), 5);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> trex.Grammar('expr := <expr> "+" <expr> | number').count("1 + 2 + 3 + 4")
5
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Invoke-TrexGrammar 'expr := <expr> "+" <expr> | number' '1 + 2 + 3 + 4' -Count
5
```
{{< /tab >}}
{{< /tabs >}}

`@p` after an alternative weights it, and `--best` and `--prob` give the probability of the most
probable derivation and of all of them.
