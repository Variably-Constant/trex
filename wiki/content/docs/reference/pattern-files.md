---
title: Pattern files
linkTitle: Pattern files
weight: 90
---

# Pattern files

A pattern file declares the names a pattern reads as `\{name}`, states what they match, gives
a built pattern its fields, and holds the rules a scan reports findings for. One declaration a
line; blank lines and `#` comments are skipped. The command line reads one with `--lib FILE`
(`--rules FILE` for its rules), Rust with `ShapeSet::declare_text` or `declare_file`, Python
with `lib=` on `Pattern` and `infer`, and PowerShell with `Import-TrexAtom`, `-Library` or
`-RuleFile`.

| Line | Declares |
|---|---|
| `shape NAME = `BYTES`` | a token kind from a bounded byte-pattern, tried before the built-in recognizers |
| `shape-after NAME = `BYTES`` | the same, tried only where no built-in recognizer matched |
| `kind NAME = PATTERN` | a token kind from a pattern: the tokens each match covers fuse into one token of the kind |
| `let NAME = PATTERN` | a sub-pattern, inlined where `\{NAME}` appears |
| `test NAME accepts "text"... rejects "text"...` | what a name matches |
| `fields NAME {mark}...` | the fields of the sub-pattern `NAME` |
| `rule NAME ...` | a named pattern with what a finding of it says |

`\{name}` resolves to a built-in kind, then a name declared, then the shipped library, so a
declaration shadows a library entry of its name and nothing may take a built-in's. A later `let`
of a name shadows an earlier one.

## Shapes

A shape's byte-pattern must have a fixed maximum length (`{m,n}` rather than `+`). A shape
tried before the built-ins wins an overlap; `shape-after` leaves a built-in kind alone.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{ticket}' --shape 'ticket = `[A-Z]{2,4}-\d{1,4}`' --text 'see AB-12 and XYZ-9 now'
[4..9] "AB-12"
[14..19] "XYZ-9"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
use trex::{Precedence, ShapeSet, parser::parse_with_shapes, scan_with_shapes};

let mut shapes = ShapeSet::new();
shapes.declare("ticket = `[A-Z]{2,4}-\\d{1,4}`", Precedence::Before).expect("a bounded shape");
let pat = parse_with_shapes(r"\{ticket}", &shapes).expect("valid pattern");
let hits = scan_with_shapes(&pat, b"see AB-12 and XYZ-9 now", &shapes);
assert_eq!(hits.iter().map(|s| (s.start(), s.end())).collect::<Vec<_>>(), [(4, 9), (14, 19)]);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Register-TrexAtom ticket -Shape '[A-Z]{2,4}-\d{1,4}'
PS> Select-TrexMatch '\{ticket}' -InputObject 'see AB-12 and XYZ-9 now' | Select-Object Start, Text

Start Text
----- ----
    4 AB-12
   14 XYZ-9
```
{{< /tab >}}
{{< /tabs >}}

In PowerShell `Register-TrexAtom -After` declares a `shape-after`, and `Get-TrexToken` shows
the token a shape makes.

## Kinds and sub-patterns

A kind from a pattern runs after the lex: the tokens each match covers fuse into one token of
the kind, brackets are paired again over what remains, and a later pattern reads the kind as one
atom, a scan reporting it whole. A sub-pattern is inlined, so its bindings and quantifiers compose
as if written in place.

```console
$ cat defs.trex
let rhs = \N | \Q
kind assign = \W "=" \{rhs}
test assign accepts "x = 1" "name = \"bob\"" rejects "x == 1"
test rhs accepts "42" rejects "forty-two"
test iban accepts "GB82 WEST 1234 5698 7654 32" rejects "GB82WEST12345698765433"
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\{assign}' --lib defs.trex --text 'let x = 1; name = "bob"'
[4..9] "x = 1"
[11..23] "name = \"bob\""
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut shapes = trex::ShapeSet::new();
shapes.declare_text("let rhs = \\N | \\Q\nkind assign = \\W \"=\" \\{rhs}\n").expect("a pattern file");
let pat = trex::parser::parse_with_shapes(r"\{assign}", &shapes).expect("valid pattern");
let text = br#"let x = 1; name = "bob""#;
let found: Vec<&[u8]> = trex::scan_with_shapes(&pat, text, &shapes).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, [&b"x = 1"[..], br#"name = "bob""#]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> p = trex.Pattern(r"\{assign}", lib="defs.trex")
>>> [m.text for m in p.scan('let x = 1; name = "bob"')]
['x = 1', 'name = "bob"']
>>> p.rewrite("<${0}>", 'let x = 1; name = "bob"')
'let <x = 1>; <name = "bob">'
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Import-TrexAtom ./defs.trex
PS> Get-TrexToken 'let x = 1; name = "bob"' | Select-Object Kind, Text

Kind   Text
----   ----
word   let
assign x = 1
punct  ;
assign name = "bob"

PS> Select-TrexMatch '\{assign}' -InputObject 'let x = 1; name = "bob"' -Raw
x = 1
name = "bob"
```
{{< /tab >}}
{{< /tabs >}}

A declared shape or kind decides the token boundaries every scan of a pattern read under it is
made on, so a rewrite, a redaction, a table and a build under the same file lex the same way.

## Tests

A test line states what a name matches. It accepts a text when the name's match in the text is
the whole of it, from the first significant token to the last, and rejects a text when the name
matches nowhere in it. Either keyword takes any number of double-quoted texts, which read `\"`,
`\\`, `\n` and `\t`. The name is any declared shape, kind or sub-pattern, or a library entry,
tested as the file finally declares it, so a test may stand above the line it checks.

```console
$ cat wrong.trex
let rhs = \N
test rhs accepts "42" "\"bob\"" rejects "4 2"
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex lib --test defs.trex
defs.trex: 3 tests passed

$ trex lib --test wrong.trex
wrong.trex:2: rhs accepts "\"bob\"": no match
  tokens: quoted "\"bob\""
wrong.trex:2: rhs rejects "4 2": matched "4" at 0..1
wrong.trex: 1 of 1 test failed
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let mut shapes = trex::ShapeSet::new();
shapes.declare_text("let rhs = \\N\ntest rhs accepts \"42\" \"\\\"bob\\\"\" rejects \"4 2\"\n").expect("a pattern file");
let failures: Vec<(usize, String)> = shapes.run_tests().into_iter().map(|f| (f.line, f.name)).collect();
assert_eq!(failures, [(2, "rhs".to_string()), (2, "rhs".to_string())]);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Import-TrexAtom ./defs.trex
PS> Test-TrexAtom | Select-Object Name, Passed

Name   Passed
----   ------
assign   True
rhs      True
iban     True

PS> Unregister-TrexAtom -All -Confirm:$false
PS> Import-TrexAtom ./wrong.trex
PS> (Test-TrexAtom rhs).Failures
accepts "\"bob\"": no match
  tokens: quoted "\"bob\""
rejects "4 2": matched "4" at 0..1
```
{{< /tab >}}
{{< /tabs >}}

`lib --test` prints each expectation not met as a `FILE:LINE:` line and exits 1 when any test
fails; given several files, they declare into one set in order, so a later file's tests may name
an earlier file's declarations. `Test-TrexAtom -Quiet` writes one boolean, and `-Shipped` tests
the shipped library.

## Fields

A `fields NAME {mark}...` line, below the `let` declaring `NAME`, gives that sub-pattern's
fields in order, each written as a `ConvertFrom-String` mark with the example text left out:
`{[int]os}` casts the field to the type it names, `{Name*}` begins a record, `{Person.Name}` is a
field inside another, and `{host:host}` reads the field through an accessor, as `${host:host}`
does in a template. A field the sub-pattern binds no register under, a field named twice, an
accessor that is not one and a `[type]` no mark knows are refused.

```text
let extract = ^ "GET" (\U):host (\N):code ~<($ .)
fields extract {host:host} {[int]code}
```

A build writes the line for the pattern it builds, and `scan --fields`, Python's
`Pattern.records`, PowerShell's `ConvertFrom-TrexText` and Rust's `infer::build::fields_for`
read `\{NAME}` through it; [saving and reusing a build](../building-patterns/#saving-and-reusing-a-build)
shows each.

## Rules

A rule is a named pattern with what a finding of it says. As a block, `rule NAME` over indented
`field = value` lines; or as one line, `rule NAME [error|warning|note] "message" = PATTERN`,
with `fix NAME = TEMPLATE`, `meta NAME KEY = VALUE`, `files NAME = GLOBS`, `unless NAME =
PATTERN`, `record NAME = UNIT`, `record-start NAME = PATTERN` and `record-span NAME = PATTERN`
lines below it.

| Field | Holds |
|---|---|
| `pattern` | the pattern a finding is a match of |
| `message` | a report template rendered at each finding: the match's registers and their typed slices, `${path}`, `${line}`, `${col}`, `${rule}` and `${severity}` |
| `severity` | `error`, `warning` or `note`; a warning where it says nothing |
| `fix` | a rewrite template rendered in the match's place |
| `files` | globs the inputs it reads must pass, as `-g` reads them; a rule with globs reads no unnamed input |
| `unless` | a pattern the record must not hold |
| `record`, `record-start`, `record-span` | what a record is, as `--record` and its two companions take it |
| `meta.KEY` | free metadata a pipeline reads; `meta.tags` is split into tags |

A rule naming `unless` or a record fires on a record holding its pattern and none of the
`unless` patterns, a line where it names no record; its finding covers the record, and the
message and fix read the rule's first match in it. The rule is also a sub-pattern under its
name, so a later pattern reads it as `\{name}` and a `test` line checks it.

```console
$ cat rules.trex
# what a config file may not hold
rule private_ip
  pattern = \{ip_private}:addr
  message = private address ${addr} in ${path:name}
  severity = warning
  fix = ${addr:octet1-2}.x.x
  meta.cwe = CWE-200
  meta.tags = network, config

rule cardnum error "card number ending ${card:last4}" = \{card}:card
fix cardnum = ****
meta cardnum tags = pii
rule todo note "a TODO left in ${path:name}" = "TODO"
test private_ip accepts "10.0.0.5" rejects "8.8.8.8"
$ cat app.conf
host = 10.0.0.5
pay 4111 1111 1111 1111 now
# TODO rotate
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --rules rules.trex app.conf
app.conf:1:8: warning: private address 10.0.0.5 in app.conf [private_ip]
  fix: "10.0.x.x"
app.conf:2:5: error: card number ending 1111 [cardnum]
  fix: "****"
app.conf:3:3: note: a TODO left in app.conf [todo]

$ trex scan --rules rules.trex --format '${severity} ${rule} ${line}:${col} ${message}' app.conf
warning private_ip 1:8 private address 10.0.0.5 in app.conf
error cardnum 2:5 card number ending 1111
note todo 3:3 a TODO left in app.conf
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let rules = r#"rule cardnum error "card number ending ${card:last4}" = \{card}:card
fix cardnum = ****
rule todo note "a TODO left in ${path:name}" = "TODO"
"#;
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

Rule       Severity Region Message
----       -------- ------ -------
private_ip  Warning 1:8    private address 10.0.0.5 in app.conf
cardnum       Error 2:5    card number ending 1111
todo           Note 3:3    a TODO left in app.conf

PS> Get-TrexRule -RuleFile ./rules.trex | Select-Object Name, Severity, Fix

Name       Severity Fix
----       -------- ---
private_ip  Warning ${addr:octet1-2}.x.x
cardnum       Error ****
todo           Note
```
{{< /tab >}}
{{< /tabs >}}

A finding of an `error` rule fails the run. `--rules DIR` scans every `.trex` file under a
directory; the [lint with rules](../../how-to/lint-with-rules/) how-to shows the reports a CI
system reads, `--sarif` and `--github`, and the fixes. The rules that fire on each match scan as
one set per list of files; a rule on records scans on its own.

```console
$ cat vault.trex
rule unguarded
  pattern = "password"
  unless = "vault"
  record = paragraph
  message = a password outside the vault
  severity = error
$ cat notes.txt
password = x
vault: ok

password = y
plain
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan --rules vault.trex notes.txt
notes.txt:4:1: error: a password outside the vault [unguarded]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Invoke-TrexRule -Path ./notes.txt -RuleFile ./vault.trex | Select-Object Rule, Region, Message

Rule      Region Message
----      ------ -------
unguarded 4:1    a password outside the vault
```
{{< /tab >}}
{{< /tabs >}}

## Libraries

In PowerShell the session holds the atoms `Register-TrexAtom` and `Import-TrexAtom` declare,
in `$TrexSession`; a `Trex.Library` from `New-TrexLibrary` holds a set of its own, from pattern
files, declaration lines or a copy of the session's, and a cmdlet given `-Library` reads it in
their place. In Rust a `ShapeSet` is that set, handed to the parser and the scan; in Python a
`Pattern` reads the files `lib=` names.

{{< tabs >}}
{{< tab name="Rust" >}}
```rust
let mut lib = trex::ShapeSet::new();
lib.declare_text("let big = \\N{>1000}\n").expect("a pattern file");
let pat = trex::parser::parse_with_shapes(r"\{big}", &lib).expect("valid pattern");
let text = b"sizes 5 and 5000";
assert_eq!(trex::scan_with_shapes(&pat, text, &lib).iter().map(|s| &text[s.range()]).collect::<Vec<_>>(), [&b"5000"[..]]);
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $lib = New-TrexLibrary -Path ./defs.trex -Declaration 'let big = \N{>1000}'
PS> $lib.Names
rhs
assign
big

PS> Select-TrexMatch '\{big}' -InputObject 'sizes 5 and 5000' -Library $lib -Raw
5000
```
{{< /tab >}}
{{< /tabs >}}

## The shipped library

The shipped library is seventy-five entries every pattern reads with no declaration. Twenty-one
are kinds with a bounded shape, twelve of them checked as they are lexed: eleven by their
standard's checksum, `\{iban}` (mod 97-10 and the registry's length per country), `\{isbn}`
(ISBN-10 mod 11 or ISBN-13), `\{vin}`, `\{isin}`, `\{ean13}`, `\{upca}`, `\{ean8}`, `\{imei}`,
`\{ethaddr}` (EIP-55 case over Keccak-256), `\{btcaddr}` (base58check, bech32 and bech32m) and
`\{github_token}` (the CRC-32 its last six characters carry over the thirty before them, for the
`gh?_` forms; a fine-grained `github_pat_` token passes on its shape), and `\{k8s_name}` by its
length, at most sixty-three characters; by shape alone `\{awskey}`, `\{slack_token}`,
`\{google_key}`, `\{stripe_key}`, `\{twilio_key}`, `\{cve}`, `\{mime}`, `\{docker_image}` and
`\{git_sha}`. Sixteen are sub-patterns over tokens: `\{private_key}` (a PEM block from BEGIN to
END), `\{ip_private}`, `\{ip_loopback}` and `\{ip_linklocal}` through `\I{in:...}`,
`\{log_level}`, `\{http_method}`, `\{http_status}` and `\{http_1xx}` to `\{http_5xx}` through
`\N{a..b}`, `\{currency}`, `\{country}`, `\{weekday}` and `\{month}`. Sixteen are the unit kinds
a [quantity](../pattern-syntax/#quantities) reads from the context: `\{kelvin}`, `\{inch}`,
`\{meter}`, `\{second}`, `\{hour}`, `\{gram}`, `\{tonne}`, `\{ampere}`, `\{volt}`, `\{watt}`,
`\{newton}`, `\{joule}`, `\{calorie}`, `\{liter}`, `\{gallon}` and `\{bar}`. The last
twenty-two are the cues those read, a word list and a sub-pattern for each of eleven families,
from `\{temperature_word}` and `\{temperature_cue}` to `\{pressure_word}` and
`\{pressure_cue}`. A library kind is lexed only for a pattern that names it: thirteen digits
with a valid check are a number to `\N` and an EAN-13 to `\{ean13}`.

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex lib | head -4
name           form     guard    what
iban           kind     checked  an IBAN, compact or in groups of four, with its country's length and the mod 97-10 check
isbn           kind     checked  an ISBN-10 (mod 11, X as ten) or ISBN-13 (978 or 979, EAN check), hyphens or spaces allowed
vin            kind     checked  a vehicle identification number: seventeen characters without I, O or Q and the check digit ninth

$ trex scan '\{iban}' --text 'pay GB82 WEST 1234 5698 7654 32 or DE89370400440532013000; not GB82WEST12345698765433'
[4..31] "GB82 WEST 1234 5698 7654 32"
[35..57] "DE89370400440532013000"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\{iban}").expect("valid pattern");
let text = b"pay GB82 WEST 1234 5698 7654 32 or DE89370400440532013000; not GB82WEST12345698765433";
assert_eq!(trex::scan(&pat, text).iter().map(|s| (s.start(), s.end())).collect::<Vec<_>>(), [(4, 31), (35, 57)]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"\{iban}").scan("pay GB82 WEST 1234 5698 7654 32 or DE89370400440532013000; not GB82WEST12345698765433")]
['GB82 WEST 1234 5698 7654 32', 'DE89370400440532013000']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> @(Get-TrexAtom -Shipped).Count
75

PS> Get-TrexAtom iban -Shipped | Select-Object Name, Form, Description

Name Form Description
---- ---- -----------
iban Kind an IBAN, compact or in groups of four, with its country's length and the mod 97-10 check
```
{{< /tab >}}
{{< /tabs >}}
