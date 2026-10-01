---
title: Lint files with rules and fix them
linkTitle: Lint with rules
weight: 11
---

# Lint files with rules and fix them

Write rules that say what a file may not hold, report each finding in the form a CI system reads,
and apply the fixes. The rule fields are on [pattern files](../../reference/pattern-files/#rules).

```console
$ cat rules.trex
rule cardnum error "card number ending ${card:last4}" = \{card}:card
fix cardnum = ****
rule todo note "a TODO left in ${path:name}" = "TODO"
$ cat app.conf
host = 10.0.0.5
pay 4111 1111 1111 1111 now
# TODO rotate
```

## Report the findings

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --rules rules.trex app.conf
app.conf:2:5: error: card number ending 1111 [cardnum]
  fix: "****"
app.conf:3:3: note: a TODO left in app.conf [todo]
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let rules = "rule cardnum error \"card number ending ${card:last4}\" = \\{card}:card\nfix cardnum = ****\nrule todo note \"a TODO left in ${path:name}\" = \"TODO\"\n";
let conf = b"host = 10.0.0.5\npay 4111 1111 1111 1111 now\n# TODO rotate\n";
let mut shapes = trex::ShapeSet::new();
shapes.declare_text(rules).expect("a pattern file");
let scan = trex::rule_scan::RuleScan::new(shapes).expect("rules that parse");
let index = trex::files::LineIndex::new(conf);
let found: Vec<(String, String)> = scan
    .findings(Some("app.conf"), conf, &index, None)
    .into_iter()
    .map(|f| (scan.rules()[f.rule].name.clone(), f.message))
    .collect();
assert_eq!(found, [
    ("cardnum".to_string(), "card number ending 1111".to_string()),
    ("todo".to_string(), "a TODO left in app.conf".to_string()),
]);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Invoke-TrexRule -Path ./app.conf -RuleFile ./rules.trex | Select-Object Rule, Severity, Region, Message

Rule    Severity Region Message
----    -------- ------ -------
cardnum    Error 2:5    card number ending 1111
todo        Note 3:3    a TODO left in app.conf
```
{{< /tab >}}
{{< /tabs >}}

A finding of an `error` rule fails the run, so a CI step stops on it.

## Annotate a CI run

`--github` writes each finding as a GitHub workflow annotation, and `--sarif` one SARIF 2.1.0
document for a code-scanning upload. The CLI writes a path as it was given; PowerShell writes the
full path of the file it resolved, run here in `C:\Temp\demo`:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --rules rules.trex --github app.conf
::error file=app.conf,line=2,col=5,endLine=2,endColumn=24,title=cardnum::card number ending 1111
::notice file=app.conf,line=3,col=3,endLine=3,endColumn=7,title=todo::a TODO left in app.conf
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Invoke-TrexRule -Path ./app.conf -RuleFile ./rules.trex -GitHub
::error file=C%3A/Temp/demo/app.conf,line=2,col=5,endLine=2,endColumn=24,title=cardnum::card number ending 1111
::notice file=C%3A/Temp/demo/app.conf,line=3,col=3,endLine=3,endColumn=7,title=todo::a TODO left in app.conf
```
{{< /tab >}}
{{< /tabs >}}

## Apply the fixes

Look at the diff the fixes make, then write them:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --rules rules.trex --fix --dry-run app.conf
--- app.conf
+++ app.conf
@@ -1,3 +1,3 @@
 host = 10.0.0.5
-pay 4111 1111 1111 1111 now
+pay **** now
 # TODO rotate
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Invoke-TrexRule -Path ./app.conf -RuleFile ./rules.trex -Fix
PS> Get-Content -Path ./app.conf
host = 10.0.0.5
pay **** now
# TODO rotate
```
{{< /tab >}}
{{< /tabs >}}

`--fix` without `--dry-run` writes them, and `--interactive` puts each to you.
