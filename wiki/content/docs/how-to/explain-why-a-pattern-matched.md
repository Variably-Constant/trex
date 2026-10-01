---
title: Explain why a pattern matched
linkTitle: Explain a match
weight: 15
---

# Explain why a pattern matched

See, for a match, the kind of every token it spans, what each checked kind passed to be that kind,
the value every axis the pattern read had there, and which rung of the scan answered
([explanations](../../reference/matching/#explanations)).

## A checked kind

A card number is a card only where it passes the Luhn check, and the explanation says so:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{card}' --explain --text 'pay 4111 1111 1111 1111 now'
[4..23] "4111 1111 1111 1111"
  tokens: creditcard "4111 1111 1111 1111"
  guard: creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check
  route: a route, not the engine
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust,standalone_crate
let pat = trex::parse(r"\{card}").expect("valid pattern");
let text = b"pay 4111 1111 1111 1111 now";
trex::trace::clear();
let route = {
    let _recording = trex::trace::Recording::start();
    let _ = trex::scan_with_backend(&pat, text, trex::Backend::Auto);
    trex::explain::route_of(&trex::trace::take_recorded())
};
let m = &trex::captures(&pat, text, &trex::scan(&pat, text))[0];
let e = trex::explain::Explainer::new(&pat, text, &trex::ShapeSet::new()).explain(m, &route);
assert_eq!(e.guards, ["creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check"]);
assert_eq!(e.route, "a route, not the engine");
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> text = "pay 4111 1111 1111 1111 now"
>>> e = trex.Pattern(r"\{card}").find(text).explain(text)
>>> e["guards"], e["route"]
(["creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check"], 'a route, not the engine')
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $e = (Select-TrexMatch '\{card}' -InputObject 'pay 4111 1111 1111 1111 now' -Explain).Explanation
PS> $e.Guards
creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check
PS> $e.Route
a route, not the engine
```
{{< /tab >}}
{{< /tabs >}}

## A threshold read from the input

A relative predicate compares with a baseline the input supplies, and the explanation gives the
value and the baseline it was compared with:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N{>+1}' --explain --text 'sizes 12 15 9 4000'
[14..18] "4000"
  tokens: number "4000"
  magnitude: 3.60 "4000"
  baseline: window: mean 1.38, spread 0.55, over 4 "4000"
  route: the set engine over a whole lex
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> text = "sizes 12 15 9 4000"
>>> e = trex.Pattern(r"\N{>+1}").find(text).explain(text)
>>> e["route"]
'the set engine over a whole lex'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> (Select-TrexMatch '\N{>+1}' -InputObject 'sizes 12 15 9 4000' -Explain).Explanation.Route
the set engine over a whole lex
```
{{< /tab >}}
{{< /tabs >}}

`--format` writes any one reading of an explanation into a line of your own:
`${@kind}`, `${@guard}`, `${@route}`, and `${@magnitude}`, `${@spectral.entropy}` and the other
axes the pattern read ([report templates](../../reference/matching/#report-templates)).
