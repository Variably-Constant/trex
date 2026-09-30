---
title: Pattern syntax
linkTitle: Pattern syntax
weight: 20
---

# Pattern syntax

The complete alphabet that `scan`, `rewrite`, and the `~"lit"` guards share. None of it
backtracks: the single-pass engine takes the regular core and the set-reachability engine
takes the rest, and both explore positions as a set.

## Token atoms

Each atom matches exactly one token of a kind. Every kind has a full name, and both columns
are live syntax: `\{ip}` matches exactly what `\I` matches. Kinds with no one-letter escape are
matched by name.

| Atom | Name | Matches |
|---|---|---|
| `\N` | `\{number}` | number |
| `\W` | `\{word}` | word / identifier |
| `\Q` | `\{quoted}` | quoted string: closed on its own line, or carried past it by a backslash; a quote its line does not close is punctuation |
| `\I` | `\{ip}` | IPv4 / IPv6 |
| `\U` | `\{url}` | URL |
| `\E` | `\{email}` | email |
| `\T` | `\{timestamp}` | timestamp / date |
| `\P` | `\{punct}` | punctuation |
| `\S` | `\{whitespace}` | whitespace (between atoms; see the note below) |
| `\V` | `\{version}` | semantic version |
|  | `\{uuid}` | UUID |
|  | `\{mac}` | MAC address |
| `\H` | `\{hexcolor}` | hex color (`#rgb` / `#rrggbb`) |
| `\C` | `\{cidr}` | CIDR block |
| `\%` | `\{percent}` | percentage |
| `\Z` | `\{bytesize}` | byte size (`10MB`, `1.5GiB`) |
| `\$` | `\{money}` | money (`$1,234.56`) |
| `\D` | `\{hash}` | hash digest (md5 / sha1 / sha256) |
| `\R` | `\{duration}` | duration (`1500ms`, `3h20m`) |
| `\L` | `\{path}` | filesystem path |
|  | `\{jwt}` | JSON Web Token: three base64url segments, `eyJ`-anchored |
|  | `\{creditcard}` | payment card: 13-19 digits, contiguous or grouped `4-4-4-4`, `4-6-5`, `4-6-4` or `4-4-4-4-3`, Luhn-valid |
|  | `\{base64}` | base64 / base64url blob: length a multiple of 4, at least 16, diverse charset |
|  | `\{geo}` | `lat,long` in decimal degrees, in range (`-90..90`, `-180..180`), a pair standing alone at four or more decimal places |
|  | `\{phone}` | `+` then an ITU-T E.164 country code, 7-15 digits in all; or `NPA-NXX-XXXX` hyphenated for North America |
|  | `\{quantity}` | a physical quantity: a number, signed where nothing alphanumeric precedes the sign, then a unit symbol attached (`5kg`, `-40°C`) or one space apart (`3.2 GHz`, `40 %`, `5 m/s`); see [quantities](#quantities) |
|  | `\{qty}` | the class of every kind a quantity predicate reads: `\{quantity}`, `\Z`, `\R`, `\%`, and the unit kinds the library reads from the context (`\{kelvin}`, `\{inch}`, `\{meter}` and the rest) |
|  | `\{name}` | a shape you declared (see [custom shapes](#custom-shapes)) |
| `\B` |  | balanced bracket group |
| `.` |  | any one token |
| `"lit"` |  | a literal token equal to `lit` |
| `"lit"~k` |  | a token within `k` edits of `lit`: `k` characters inserted, deleted or substituted, one edit each; `k` is always written |
| `` `re` `` |  | a byte-pattern matched whole against one token |

Three notes on the table. The five guarded kinds each carry a check (the Luhn checksum, the
coordinate ranges, the `eyJ` prefix) so they do not fire on ordinary words or numbers; a
custom format composes from atoms, byte classes, and quantifiers instead (`"+" \d+ "-" \d+`
for a specific phone shape), or is declared as a shape. Some names take a shorthand alias:
`\{card}`, `\{b64}`, `\{coord}`, `\{tel}`, and `\{hashdigest}` for hash. And whitespace atoms
cannot start a pattern - matches anchor at significant tokens - so `\S` matches the whitespace
between atoms (`\W \S \W`).

### Byte classes

The lowercase escapes drop below the token grain. Each matches a whole token whose every byte
satisfies the class:

| Class | Every byte is |
|---|---|
| `\d` | a digit |
| `\w` | a word byte (alphanumeric or `_`) |
| `\s` | whitespace (between atoms, like `\S`) |
| `\h` | a hex digit |
| `\a` | a letter |
| `\u` | an uppercase letter |
| `\l` | a lowercase letter |

The case is the grain: an uppercase escape is a whole typed token, a lowercase escape is a
regex-style byte class.

### Byte-patterns

A `` `re` `` between backticks is a regex-shaped pattern over the bytes of one token, anchored
to the whole token: `` `[a-z]+\d+` `` matches `abc12` and not `abc`. Inside it, `.`, `[...]`
and `\p{L}` match one *character* (a class range is over codepoints, so `[a-z]` and a range
of Greek letters are the same construct), while `\d`, `\w` and `\s` stay ASCII bytes. `\p{...}`
takes any Unicode general category the standard library decides exactly; an unknown name is a
parse error rather than an empty class.

```console
$ trex scan '`\p{L}+`' --text 'héllo wörld 123'
[0..6] "héllo"
[7..13] "wörld"
```

Words are unicode at the token grain too. A multi-byte letter joins its word and a CJK run
lexes as one token, so `\W` matches `中文分词` whole - the char can never shatter into byte
shards the way byte-level tokenizers split it.

Every construct on this page runs in one file,
[examples/patterns.rs](https://github.com/Variably-Constant/trex/blob/main/examples/patterns.rs),
where each demo asserts its match count against the real engine.

## Sequence, choice, and repetition

| Form | Meaning |
|---|---|
| `A B` | sequence (whitespace between atoms is insignificant) |
| `A \| B` | leftmost-first choice: the first branch that can match wins |
| `A \|\| B` | leftmost-longest choice: no preference, the longest overall match wins |
| `A \|> B` | committed choice: the first branch that matches wins outright and is never reconsidered |
| `A*` `A+` `A?` | zero-or-more, one-or-more, optional (greedy) |
| `A{m}` `A{m,n}` | counted repetition |
| `A*?` `A+?` `A??` `A{m,n}?` | the lazy forms: prefer the shortest |
| `A*+` `A++` `A?+` | the possessive forms: the greedy match with its other lengths discarded |
| `(?>A)` | atomic group: `A`'s preferred match, never backed out of |
| `(A)` | a logical group; it binds nothing |

The three choices are different answers, not shades of one. For `("a" | "a" "b") "c"` over
`a b c`, `|` and `||` both match the whole span while `|>` matches nothing, because it takes
the short branch and never reconsiders when `"c"` fails:

```console
$ trex scan '("a" | "a" "b") "c"' --text 'a b c'
[0..5] "a b c"

$ trex scan '("a" |> "a" "b") "c"' --text 'a b c'
no match
```

For `\W | \W \W` over `a b`, `|` matches `a` then `b` while `||` matches `a b`:

```console
$ trex scan '\W | \W \W' --text 'a b'
[0..1] "a"
[2..3] "b"

$ trex scan '\W || \W \W' --text 'a b'
[0..3] "a b"
```

An atomic group keeps only the length its body prefers, so `(?>\N+) \N` finds nothing where
`\N+ \N` gives a number back:

```console
$ trex scan '\N+ \N' --text '1 2 3'
[0..5] "1 2 3"

$ trex scan '(?>\N+) \N' --text '1 2 3'
no match
```

## Token classes

A class is a set expression over single-token atoms, so it composes with every atom the
language has. `[A B]` unions, `[^A B]` complements, `[A && B]` intersects and `[A -- B]`
subtracts; a literal hyphen or ampersand must be quoted (`["-"]`).

```console
$ trex scan '[\N \W]+' --text 'a 1 , b'
[0..3] "a 1"
[6..7] "b"

$ trex scan '[\W && \h]' --text 'deadbeef xyz cafe'
[0..8] "deadbeef"
[13..17] "cafe"
```

## Typed value predicates

A kind atom takes a `{...}` body that compares the token's value in the kind's own units.
The lexer has already recognized the token's shape, so the predicate reads that shape's
value and compares there: an address as an address, a version as a version, a size as a
count of bytes.

| Form | Meaning |
|---|---|
| `\N{>500}` `\N{500..599}` `\N{>=1.5,<2}` `\N{!=200}` | the number's value: `> >= < <= = !=`, an inclusive range `a..b`, clauses joined by `,` |
| `\I{in:10.0.0.0/8}` `\I{v4}` `\I{v6}` `\I{10.0.0.1..10.0.0.9}` | an address inside a block, of a family, in a range |
| `\C{contains:10.1.2.3}` `\C{in:10.0.0.0/8}` `\C{prefix>=24}` | a block holding an address, lying inside a block, or by its prefix length |
| `\T{age<24h}` `\T{>2026-09-01}` `\T{2026-09-01..2026-09-15}` `\T{<now}` `\T{hour>=22}` | an instant against the clock, an instant, a range; `year` `month` `day` `hour` `minute` `second` as fields |
| `\V{>=2.0,<3}` `\V{major=1,minor>=4}` `\V{pre:rc*}` | semver precedence; the numeric parts; the pre-release identifiers |
| `\Z{>1GiB}` `\R{>500ms}` `\${>1000}` `\%{>50}` | a byte count with the unit normalized (`KB` is 1000, `KiB` 1024), a duration (a value needs a unit), an amount, a percentage |
| `\{qty}{>5kg}` `\{qty}{=435cm}` `\{qty}{<0°C}` `\{qty}{5kg..6kg}` `\{qty}{unit:kg}` `\{qty}{family:mass}` `\{qty}{in:@limits.txt}` | a quantity's value in its family's base unit, whatever unit it was written in (`12 lb` is more than `5kg`, `0.1 kg` equals `100g`); the symbol as written; the family; a set of quantities |
| `\U{host:*.internal}` `\U{scheme:https}` `\U{port>1024}` `\U{path:/api/*}` `\U{query:*token=*}` | a URL's fields; a port a URL leaves out is its scheme's |
| `\E{domain:example.com}` `\E{user:admin*}` | an email's fields |
| `\L{ext:log}` `\L{name:app.*}` `\L{dir:/var/*}` | a path's fields |
| `\{creditcard}{issuer:visa}` `\{card}{last4:1111}` `\{card}{len=16}` | a card's issuer, last four digits, digit count |
| `\{phone}{cc=44}` `\{uuid}{version=4}` `\{mac}{oui:00:1a:2b}` `\D{algo:sha256}` `\{geo}{lat>0,long<0}` | a phone's country code, a UUID's version, a MAC's first three octets, a digest's algorithm, a coordinate's parts |
| `\W{len>8}` `\Q{len<=2}` | a word's or a string's length in characters |
| `\{jwt}{alg:none}` `\{jwt}{exp<now}` `\{base64}{bits>7}` | what a token encodes rather than how it is written; see [decoded content](#decoded-content) |
| `\T{>+1h:t}` `\T{<+500ms:t}` `\T{<-30m:t}` | this instant against the one the register holds: a signed duration compared with the difference between them, so `>+` is a gap, `<+` a burst and `<-` a stamp standing before the bound one |

A clause is `[field] op value`: the ordering operators on an ordered value, `field:value` on
a text field with `*` and `?` as globs, `a..b` an inclusive range, and `in:` / `contains:`
for containment. A kind with one value (a number, an address, an instant, a version, a
size, a duration, an amount) reads it when no field is named; a kind with several (a URL,
an email, a path, a card) names one every time, and naming a field the kind lacks is a parse
error that lists the fields it has. Host, domain, scheme, issuer and algorithm compare with
ASCII case folded; paths, queries, users and identifiers do not. A value never holds a
comma, since commas separate the clauses.

That same read is what a report prints. A register binding a single typed kind carries a
`value` beside its text in `--json`, reaches a rewrite closure through `Matched::value`, and
reaches Python through `Match.value`, all from one parse, so a value in a report cannot
disagree with the clause that selected the match. Each kind reports its base unit: a byte
size in bytes on `KB` of 1000 and `KiB` of 1024, a duration in nanoseconds on `d` of 24h,
`w` of 7d and `y` of 365d, a timestamp as its epoch second, an address as the integer it
orders by, a version as the parts and pre-release identifiers semver orders on with build
metadata absent, money and a percentage as exact decimals. A register binding a run, a
repetition or an alternation of two or more has no one kind and reports no value.

`\T{>+1h:t}` is the one form here that reads no clock. It compares this instant with the one
the register holds - the last instant bound to it - so a gap between records, a burst and a
clock running backwards are the same construct with different operators. A `<` form admits a
stamp standing before the bound one as well as one close after it, so a burst that must run
forward names the direction too: `\T:t \W (@order:asc \T{<+1m:t})`.

The clock `age` and `now` read is the wall clock at the start of the scan; `--now` on the
CLI and `trex::set_now` in the library fix it. A timestamp written with no zone is read in
UTC; `--tz +02:00` and `trex::set_tz_offset` change that. A syslog timestamp, which writes
no year, takes the clock's year, or the year before where that would put it more than a day
ahead. Every form the lexer recognizes reads: `2026-09-01`, `12:30:45.5`,
`2026-09-01T12:00:00Z`, `2026-09-01T12:00:00.123+02:00`, `2026-09-01 12:00:00`,
`Sep 14 18:50:53`, `10/Oct/2000:13:55:36 -0700`, and the slash dates `2026/09/15`,
`09/15/2026` and `15/09/2026`, each with a clock after it where one is written.

A slash date's year is four digits, and that is what tells a date from a run of figures:
`1/2/3` is three numbers. `2026/09/15` is read by its shape. In the other two the year stands
last, and which of the leading fields is the day is decided by which reading the calendar
holds - `15/09/2026` has no fifteenth month. Where both hold, as in `03/04/2026`, the day
comes first, which is the order of the one slash date trex read before these, Apache's;
`--date-order mdy` on the CLI and `trex::set_date_order_day_first` in the library reverse it.
Neither reading moves the extent, so the span a scan reports is the same either way.

The magnitude axis keeps its own spelling: `\N{mag>3}` is a number of three or more orders
of magnitude, and the relative forms `\N{>+1}`, `\N{>+2s:k}` read as before.

```console
$ trex scan '\I{in:10.0.0.0/8}' --text 'src=10.20.30.40 dst=8.8.8.8 lan=10.0.255.1'
[4..15] "10.20.30.40"
[32..42] "10.0.255.1"

$ trex scan '\V{>=2.0,<3}' --text 'app 2.5.1 lib 1.4.2 tool 3.0.0'
[4..9] "2.5.1"

$ trex --now 2026-09-15T00:00:00Z scan '\T{age<24h}' --text '2026-09-14T22:00:00Z ok 2026-09-10 old'
[0..20] "2026-09-14T22:00:00Z"
```

A value written `@file` is a set read from the file when the pattern is parsed, one member a
line, blank lines and `#` lines skipped, and the clause holds of a token that is one of them,
in the field's own type: `\I{in:@cidrs.txt}` is an address inside any listed block or equal
to any listed address, `\C{in:@blocks.txt}` a block inside a listed one, `\N{in:@ports.txt}`
a number equal to a listed one, `\V{in:@versions.txt}` a version, `\W{in:@words.txt}` a word
equal to a listed line, and `\E{domain:@blocklist.txt}` or `\U{host:@hosts.txt}` a text field
equal to a listed line under the field's own case rule. `in:GROUP:@file` folds the token and
every line under an orbit group first, so `\W{in:case:@words.txt}` takes the words in any
case. `!=@file` is the complement. A relative path is read from the current directory, and
from beside the pattern file for a `let` that names it; a missing file or a line that is not
a member is a parse error naming the line. A word list is one hashed lookup a token, an
address list one lookup a prefix length, however long the list.

```console
$ cat cidrs.txt
10.0.0.0/8
192.168.1.0/24

$ trex scan '\I{in:@cidrs.txt}' --text 'from 10.4.5.6 and 8.8.8.8 to 192.168.1.9'
[5..13] "10.4.5.6"
[29..40] "192.168.1.9"
```

## Quantities

A quantity is a number and a unit symbol read as one token, compared in its family's base
unit. `\{quantity}` is the kind the lexer makes; `\{qty}` is the class of every kind a
quantity predicate reads - `\{quantity}`, `\Z`, `\R`, `\%`, and the unit kinds the library
reads from the context - so `\{qty}{>5kg}` reads a mass however it was written and
`\{qty}{>1GiB}` a size.

The lexer reads a symbol **attached** to its number (`5kg`, `20°C`, `1.5GiB`), and **one
space apart** when the symbol has two or more characters or holds a non-letter (`5 kg`,
`3.2 GHz`, `40 %`, `5 m/s`, `2 kWh`). The space may be a space, a no-break space, a narrow
no-break space or a thin space. A leading `-`, `+` or `−` belongs to the quantity when
nothing alphanumeric precedes it, so `-40°C` is one token and the `-` of `10-20kg` is a range
dash. A single-letter symbol after a space (`5 m`), and `in`, `bar`, `cal` and `gal`, are
English words as often as units, so the lexer leaves them and the library reads them from the
tokens around them (below).

Every unit converts to its family's base by a terminating decimal, so a quantity compares
exactly as a number does: `0.1 kg` is `100g`, and `4.35 m` is `435cm` to the digit. The
families are mass, length, area, volume, temperature, time, frequency, data, data rate, ratio,
voltage, current, power, resistance, energy, charge, capacitance, inductance, pressure, force,
torque, speed, throughput, angle, luminous flux, illuminance, amount, level, power level and
resolution. Two quantities of different families are never ordered and never equal.

| Form | Meaning |
|---|---|
| `\{qty}{>5kg}` `\{qty}{5kg..6kg}` `\{qty}{=435cm}` | the value in the family's base unit, whatever unit it was written in |
| `\{qty}{unit:kg}` `\{qty}{unit:k*}` | the symbol as written, by glob |
| `\{qty}{family:mass}` | the family |
| `\{qty}{in:@limits.txt}` | a set of quantities read from a file |
| `${q:value}` `${q:unit}` | in a rewrite, the number and the symbol as written |

```console
$ trex scan '\{qty}{>5kg}' --text 'shipped 5000 g, 6 kg, 4.9kg and 12 lb'
[16..20] "6 kg"
[32..37] "12 lb"

$ trex scan '\{qty}{<0°C}' --text 'held at 20°C, then -40°C, then 305 K'
[20..26] "-40°C"

$ trex rewrite '\{qty}:q' '${q:value} [${q:unit}]' --text 'mass 5.50 kg at 3.2 GHz'
mass 5.50 [kg] at 3.2 [GHz]
```

The symbols the lexer leaves alone are library kinds written in this language: a number then
the symbol, with a word of the unit's family or another quantity of that family within
thirty-two significant tokens before the number or after the symbol. `\{kelvin}`, `\{inch}`,
`\{meter}`, `\{second}`, `\{hour}`, `\{gram}`, `\{tonne}`, `\{ampere}`, `\{volt}`, `\{watt}`,
`\{newton}`, `\{joule}`, `\{calorie}`, `\{liter}`, `\{gallon}` and `\{bar}` read that way, and
`\{qty}` holds them all. Each family's vocabulary is a sub-pattern of its own
(`\{temperature_word}`, `\{length_word}`, and so on) and the cue that admits a quantity as
well is `\{temperature_cue}` and its kin, so a corpus that spells things differently can
shadow any of them with a declaration of the same name.

```console
$ trex scan '\{qty}' --text 'cooled to 4.2K overnight'
[10..14] "4.2K"

$ trex scan '\{qty}' --text 'a 5K run this week'
no match

$ trex scan '\{qty}{>1h}' --text '2 h elapsed'
[0..3] "2 h"
```

## Decoded content

A token that carries an encoding is testable on what it encodes. `\{jwt}` reads its decoded
header and payload, `\{base64}` the bytes its blob decodes to. Nothing is verified: a token's
signature is not checked, because checking one needs a key.

| Form | Meaning |
|---|---|
| `\{jwt}{alg:none}` `\{jwt}{typ:JWT}` `\{jwt}{kid:k-*}` | a registered header parameter: `alg`, `typ`, `kid`, `cty`, `crit`, `enc`, `zip`, `jku`, `x5u` |
| `\{jwt}{role:admin}` `\{jwt}{iss:*.internal}` `\{jwt}{ver=2}` | any other name is a payload claim, compared as what it holds: a number numerically, anything else by glob |
| `\{jwt}{exp<now}` `\{jwt}{exp<2026-09-16}` `\{jwt}{iat>=2026-09-01}` | `exp`, `nbf` and `iat` are NumericDate, so they take the timestamp grammar and the same clock `\T{age<24h}` reads |
| `\{jwt}{header.alg:none}` `\{jwt}{payload.sub:123}` | a half named outright, for a claim whose name is also a header parameter's |
| `\{jwt}{header:*none*}` `\{jwt}{payload:*admin*}` | a half's whole decoded text |
| `\{base64}{text:*BEGIN*}` `\{jwt}{text:*admin*}` | decoded content as text (a JWT's is its payload) |
| `\{base64}{bits>7}` `\{base64}{bits<4}` | the Shannon entropy of the decoded bytes, in bits per byte, from 0 to 8 |
| `\{base64}{texture:code}` `\{base64}{period=16}` | the texture class and the dominant byte-period of the decoded bytes |
| `\{base64}{match:name}` | a declared sub-pattern matches somewhere in the decoded bytes |

`bits` is not `\F{entropy>k}`. The two read different things: `bits` is the entropy of what a
blob decodes to, in bits per byte, and `\F{entropy>k}` is the pooled spectral reading of the
token as it is written, normalized from 0 to 1.

Decoding is the whole cost of these, so it is paid once per token and only where a clause
asks for it. A predicate naming only header parameters never decodes the payload, the clauses
of one predicate are asked cheapest first, and a token failing a clause its own bytes answer
is never decoded at all. A `match:` clause searches the decoded bytes for the literals its
sub-pattern requires before the engine runs over them. Nothing here needs the decoded bytes to
be text: the byte readings are over the bytes, a glob reads them lossily, and a sub-pattern
lexes them as a scan lexes any input.

```console
$ trex scan '\{jwt}{alg:none}' --text 'auth eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig ok'
[5..69] "eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig"

$ trex scan '\{base64}{text:*BEGIN*}' --text 'key LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ== blob AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd end'
[4..48] "LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ=="

$ trex scan '\{base64}{bits>4.5}' --text 'key LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ== blob AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd end'
[54..94] "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd"
```

## Binding, back-reference, and symmetry

| Form | Meaning |
|---|---|
| `A:name` | bind the matched span to a register |
| `A::name` | bind, scoped to the enclosing balanced group (dropped at its close) |
| `(A:k B:v):pair` | a register bound inside a bound pattern is named through it: `pair`, `pair.k` and `pair.v`, whether the inner bindings were written in place or inlined from a `let`; every reference inside follows (`=pair.k`), a reference to a register bound outside does not |
| `(A:k)+`, `(A:k){2,4}` | a register bound under a repetition keeps every binding it made, oldest first, beside its last: `${k[0]}` and `${pair[1].k}` in a template, `${k[*]}` for every binding joined with a comma, `--json` as an array, `Match::list` in the library |
| `=name` | a later token equal to the bound value (exact back-reference) |
| `=shape name` / `=case name` / `=notation name` | a later token in the bound token's orbit (fuzzy back-reference) |
| `=editk name` | a later token within `k` edits of the bound one, `k` always written; under `(?orbit:G ...)` the two are folded first |
| `=ip name` `=url name` `=time name` `=path name` `=fold name` `=numeric name` | equality up to representation: the same address in any written form, the same URL under RFC 3986 normalization, the same instant in any form or zone, the same path under either separator, the same word with compatibility forms, diacritics and case folded, the same number in any notation |
| `=subnet name` / `=subnet/16 name` | a later address in the bound address's network: /24 or /64 by family, or the width written |
| `=domain e` `=user e` `=host u` `=port u` `=day t` `=hour t` `=month t` `=year t` `=major v` `=minor v` `=patch v` `=magnitude n` `=issuer c` `=last4 c` `=cc p` `=ext f` `=dir f` `=name f` `=oui m` `=algo h` `=family a` `=prefix b` `=len w` | a later token equal to the bound one under a typed relation: the same mail domain, URL host, calendar day as written, major version, order of magnitude, card issuer, and the rest |
| `=kin name` / `=kin:byte name` / `=kin:super name` | a later token whose unit is of the type the bound value starts in, or of a type the input's pair field places in one gravity class with it, at the named grain (the token by default) |
| `(?orbit:G A)` | every literal inside `A`, every plain `=name`, and every echo or join anchor compares under symmetry group `G`: `case`, `shape`, `notation`, `ip`, `url`, `time`, `path`, `fold`, `numeric`, or any typed relation, which takes its width where it has one (`subnet/24`). One scope at a time |
| `(A B C)~k` | a run of tokens within `k` token edits of the atoms: a token the run lacks, a token it has extra, or a token that matches no atom in its place. Every element is a single-token atom, optionally bound; `k` is always written and stands under the number of atoms |
| `\B(...)` `\B[...]` `\B{...}` | a balanced group of that bracket kind, interior matches `...` |

`(?orbit:case ...)` is what a regex spells `(?i)`; the other rungs have no regex counterpart,
which is why the axis is named rather than a flag added per equivalence.

A typed relation projects both the bound span and the later token through a field of the
kind and compares the projections, so `=subnet a` reads two addresses as addresses and
`=day t` two timestamps as calendar days. A keyword with no register after it is a register
of that name, as with the groups. The representation rungs fold a value's written forms onto
the value: two spellings of one IPv6 address, one URL with and without its default port, one
instant in two zones, a word with and without its diacritics, a number with and without its
thousands separators.

```console
$ trex scan '"login" \I:a .*? "login" =subnet a' --text 'login 10.0.0.5 ok; login 10.0.0.77 ok; login 10.0.9.1'
[0..34] "login 10.0.0.5 ok; login 10.0.0.77"  captures: a="10.0.0.5"

$ trex scan '\E:e .*? =domain e' --text 'bob@x.com ann@y.org eve@X.COM'
[0..29] "bob@x.com ann@y.org eve@X.COM"  captures: e="bob@x.com"

$ trex scan '(?orbit:fold "cafe")' --text 'Café CAFE cafe cafes'
[0..5] "Café"
[6..10] "CAFE"
[11..15] "cafe"
```

Each relation is also a **rung**, which is the same projection used as an equivalence rather
than as a pair: the orbit of a text is every text the projection reads alike. Named by a
scope it reaches everything inside at once - the literals, a plain `=name`, the echo anchors,
and a join against a second input:

```console
$ trex scan '(?orbit:domain \E:a . =a)' --text 'from bob@corp.example to amy@corp.example'
[5..41] "bob@corp.example to amy@corp.example"  captures: a="bob@corp.example"
```

A back-reference that names a comparison of its own keeps it, so `=case a` inside a
`(?orbit:domain ...)` scope still compares case-folded text; only a plain `=a`, which names
none, takes the scope's. A scope cannot stand inside another.

Where the two readings differ is the calendar. `=day t` does not hold an absent year against
a written one, so `Sep 15` relates to a day in 2026 and to the same day in 2025, which do not
relate to each other - that is no equivalence and has no representative. The rung keeps the
year as written and is the finer of the two: two days a year apart never fold, so a join
across inputs cannot report a match across years.

A text the projection cannot read stands for itself behind a `?`, which no projected key
carries, so a bare word never folds onto a value read out of an address.

A register bound inside a bound pattern is named through it, and a register bound under a
repetition keeps every binding: the report's captures show the last, `--json` the tree and the
arrays, and a template reads one by index:

```console
$ trex scan '(\W:k "=" \N:v):pair' --text 'x = 1; y = 22'
[0..5] "x = 1"  captures: pair="x = 1", pair.k="x", pair.v="1"
[7..13] "y = 22"  captures: pair="y = 22", pair.k="y", pair.v="22"

$ trex scan '((\W:k "=" \N:v):pair ";")+' --text 'a = 1; b = 2;'
[0..13] "a = 1; b = 2;"  captures: pair="b = 2", pair.k="b", pair.v="2"

$ trex rewrite '((\W:k "=" \N:v):pair ";")+' '${pair[0].k}..${pair[1].v}' --text 'a = 1; b = 2;'
a..2
```

```console
$ trex scan '(?orbit:case "cat")' --text 'Cat CAT dog cat'
[0..3] "Cat"
[4..7] "CAT"
[12..15] "cat"
```

An edit is one character inserted, deleted or substituted, so a swapped pair costs two, and
the count is written every time; spaCy's `FUZZY` and ugrep's `-Z` count the same edits.

`(A B C)~k` counts the same edits a grain up, over tokens rather than characters: a token the
run lacks, a token it has extra, or a token that matches no atom in its place. A phrase with a
word missing, a word inserted or a word replaced is one edit away from the pattern that names
it, and a swapped pair is two, as it is inside a token.

```console
$ trex scan '("user" "bob" "logged" "in")~1' --text 'user bob logged in'
[0..18] "user bob logged in"

$ trex scan '("user" "bob" "logged" "in")~1' --text 'user bob in'
[0..11] "user bob in"

$ trex scan '("user" "bob" "logged" "in")~1' --text 'user bob has logged in'
[0..22] "user bob has logged in"

$ trex scan '("user" "bob" "logged" "in")~1' --text 'user amy logged in'
[0..18] "user amy logged in"

$ trex scan '("user" "bob" "logged" "in")~1' --text 'user amy has logged in'
no match

$ trex scan '(\W \N \W)~1' --text 'alpha beta'
[0..10] "alpha beta"
```

Each element matches its token as it would alone, so a kind, a class, a predicate and a byte
pattern all work where a literal does, and one may be bound - the register takes the token its
atom aligned with:

```console
$ trex scan '("user" \W:who "logged" "in")~1' --text 'user bob logged in'
[0..18] "user bob logged in"  captures: who="bob"
```

Three things the group requires. Every element is a single-token atom: a quantifier, an
alternation of sequences, a nested group or an anchor inside is a parse error naming what the
group takes. The count stands under the number of atoms, because at the atom count every atom
could be dropped and the group would match a run resembling none of them. And the run always
holds at least one token.

Every run within the count is offered, cheapest first and then longest, so what follows the
group chooses among the alignments the way it chooses among an alternation's branches:
`("a" "b" "c")~1` takes three tokens of `a b c d` on its own, four when `"d"` follows it. Where
two alignments cost the same the walk prefers to align the atom at hand with the token beside
it, so a register may take a token a different equal-cost reading would have left it without.
A `=name` inside the group reads the registers as they stood before the group.

```console
$ trex scan '"vector"~1' --text 'vector vectr vecter vectors vectorial'
[0..6] "vector"
[7..12] "vectr"
[13..19] "vecter"
[20..27] "vectors"

$ trex scan '\W:w "and" =edit1 w' --text 'vector and vectors, tint and tone'
[0..18] "vector and vectors"  captures: w="vector"
```

## Assertions and guards

| Form | Meaning |
|---|---|
| `~"lit"` | the forward window must contain `lit` (zero-width; a prefilter answers it) |
| `!~"lit"` | the forward window must not contain `lit` |
| `~(A)` `!~(A)` | lookahead: `A` must (not) match starting here, consuming nothing |
| `~<(A)` `!~<(A)` | lookbehind: `A` must (not) match ending here; `A` needs a bounded length |
| `~>k(A)` `!~>k(A)` | proximity: `A` must (not) match at one of the next `k` significant tokens |
| `~>k{m,n}(A)` | of those `k` tokens, between `m` and `n` are positions where `A` starts a match |
| `~#(A)` `!~#(A)` | region: `A` must (not) match somewhere inside the balanced group opening here |
| `~#{m,n}(A)` | that group holds between `m` and `n` positions where `A` starts a match |

The last four ask what a regular expression cannot. Distance in tokens is a unit a
token stream has and a byte stream does not, counting how many times something
occurs nearby is not a regular property, and a region's extent is found by counting
brackets. `{m}` is an exact count, `{m,}` a floor, `{m,n}` both ends; with no brace
the assertion asks only whether there is any, which is one and no ceiling.

`~>k` bounds the count by a distance the pattern names, so a scanner holding `k`
tokens past a match can still finalize it. `~#` bounds it by a structure the input
has, so no fixed reserve covers it and the pattern leaves the prefix path. A
position that opens no group has an empty region and a count of zero, which keeps
`~#` and `!~#` complements everywhere rather than only at a bracket. Nested groups
lie inside the region, so their tokens count toward the enclosing one.

The two forms answer differently on the same input, which is the clearest way to
see what "region" buys. Over `f(1, 2) 3 4 5`, asking for three numbers:

| pattern | matches | why |
|---|---|---|
| `\W ~#{3,}(\N) \B` | nothing | the region is `(1, 2)` and stops at the bracket, so it counts two |
| `\W ~>9{3,}(\N) \B` | `f(1, 2)` | nine tokens of reach cross the bracket and count five |

And the region reaches its own close through nesting, which is what puts it past
a regular language. Over `f(a(1, 2), 3) g(b(1), 2)`, `\W ~#{3,}(\N) \B` matches
only `f(a(1, 2), 3)`: finding where f's region ends means counting brackets, and
a matcher that stopped at the first `)` would read it as `a(1, 2` and count two.

```console
$ trex scan '\W ~(\N)' --text 'x 1 y z 2'
[0..1] "x"
[6..7] "z"

$ trex scan '!~<(\N) \W' --text 'x 1 y z 2'
[0..1] "x"
[6..7] "z"
```

## Where a match stands

| Form | Holds when |
|---|---|
| `^` | the current token is the first significant token of its line |
| `$` | the current token is the last significant token of its line |
| `\A` | the current token is the first significant token of the input |
| `\z` | the current token is the last significant token of the input |
| `\G` | the match begins exactly where the previous one ended: a run that abuts, and stops at the first gap |
| `\K` | report the match as beginning here; everything before it is still required |
| `@k A` | anchor `A` to the k-th comma-delimited field |

```console
$ trex scan '\G \N' --text '1 2 x 3 4'
[0..1] "1"
[2..3] "2"

$ trex scan '"key" ":" \K \W' --text 'key: value key: other'
[5..10] "value"
[16..21] "other"
```

## Lenses

A lens expands a convergent structural shape into a token pattern.

| Lens | Shape |
|---|---|
| `@call` | an identifier followed by a balanced paren group |
| `@block` | a balanced brace group |
| `@nesting` | a balanced group of any bracket kind |
| `@string` / `@number` / `@ident` | a quoted, numeric, or word token |
| `@assignment` | an identifier immediately followed by `=` |
| `@kv` | a word followed by `:` or `=` (a key/value head) |
| `@flag` | a `-` or `--` command-line flag |
| `@list` | a comma-separated run |
| `@range` | two numbers joined by `..`, `-`, or `:` |

The `#"..."` silhouette matches any token sequence by its class shape: `W` a word, `N` a
number, `.` any token, and any other character a literal. `#"W(N,N)"` matches a word followed
by a parenthesised pair of numbers.

## Axis predicates and anchors

These query a [property axis](../axes/) from inside a pattern. A token matches when the
predicate holds of the axis field over that token's span.

| Form | Holds when |
|---|---|
| `\M{>k}` / `\M{<k}` | the token's magnitude (order of magnitude) crosses `k` |
| `\N{mag>k}` | a number atom intersected with a magnitude predicate; `\N{>k}` without `mag` compares the value ([typed value predicates](#typed-value-predicates)) |
| `\F{entropy>k}` / `\F{entropy<k}` | pooled [spectral](../axes/spectral/) entropy crosses the threshold |
| `\F{period=k}` / `\F{period}` | the dominant byte-period equals `k`, or any strong period is present |
| `\F{texture:code}` / `:prose` / `:math` / `:data` | the pooled texture class matches |
| `\F{onset}` | a spectral change-point lies within the token's span |
| `@seam` / `@seam:byte` / `@seam:super` | zero-width anchor at a predictive-segmentation break: in the sequence of token kinds, in the bytes, or in the sequence of construct roles. `@seam:token` spells the default |
| `@nested>k` / `@nested>=k` | zero-width anchor on tokens at least that many brackets deep |
| `@ambiguous` / `@ambiguous:token` / `@ambiguous:super` | zero-width anchor at a vantage-dependent (garden-path) point, at the named grain |
| `@strain>90` / `@strain:byte>99` / `@strain:super>2.5b` | the unit's past pushes it away under the input's pair field: its mean potential against the units before it, in bits, above a percentile of the input's own readings or, with `b`, a value in bits. The grain names the unit - the token, its first byte, or the supertoken holding it |
| `@bound<10` / `@bound:byte<1` / `@bound:super<=-1b` | a cut the input holds together least across: the attraction over the cut before the unit, below a percentile of the input's readings or a value in bits. At the supertoken grain it holds only at a token that opens its supertoken |
| `@kin("x")` / `@kin:byte("e")` / `@kin:super("f(x)")` | the unit is of `x`'s type, or of a type the pair field places in one gravity class with it; `x` is one byte at `:byte`, and otherwise names the first token or supertoken it forms |
| `@novel` | zero-width anchor on the first occurrence of a token's content (the echo axis) |
| `@echoed` | zero-width anchor on content that recurs elsewhere in the input |
| `@echo>k` `@echo=k` `@echo<k` `@echo!=k` | how many times the token's content occurs in the input, one for content that occurs once; bare `@echo` is `@echoed` |
| `@echo:nth=k` `@echo:nth>k` | which occurrence this one is, counting from one; `nth=1` is `@novel` |
| `@echo:nth=-1` `@echo:nth=-2` | counting back from the last occurrence |
| `@echo:period` `@echo:period=k` | the content recurs at a regular spacing, of `k` bytes |
| `@order:asc` `@order:desc` | this timestamp stands at or after the timestamp token before it in the stream, or before it |
| `@shape:rare` `@shape:rare<5` `@shape:rare<1%` | the line this token stands on has a rare template, lines being grouped by token-kind silhouette as `trex templates` groups them: one covering fewer lines than the mean template, or fewer than 5, or less than 1% of the lines with a template |
| `@echoed:@other.log` `@novel:@other.log` | the token's content occurs somewhere in a second input, or nowhere in it, keyed exactly or at the rung an `(?orbit:G ...)` scope names; the second input is read and lexed once when the pattern is parsed, from the path written, or from bytes a library caller supplies under that name to `parse_with_inputs` |
| `@super` | zero-width anchor on the first token of a construct (supertoken) |
| `@super:call` / `:assign` / `:kv` / `:list` / `:numeric` / `:plain` | the construct containing the token has that role |

The echo anchors count a token's content against the whole input, before and after it, and
count punctuation and brackets not at all: those recur by grammar rather than by content, so
the axis leaves them unkeyed and no reading of it holds. The rung they count at is the one an
`(?orbit:G ...)` scope around them names, so `(?orbit:case @echo>5)` counts `Error` and
`error` as one piece of content. A scan builds one recurrence field per rung a pattern names.

```console
$ trex scan '@echo>2 \W' --text 'alpha beta alpha gamma alpha beta'
[0..5] "alpha"
[11..16] "alpha"
[23..28] "alpha"

$ trex scan '@echo:nth=-1 \W' --text 'alpha beta alpha gamma alpha beta'
[17..22] "gamma"
[23..28] "alpha"
[29..33] "beta"

$ cat billing.log
billing c91d ok
billing e5e5 ok

$ trex scan '@echoed:@billing.log \W' --text 'req fa3b start; req c91d start; req E5E5 end'
[20..24] "c91d"

$ trex scan '(?orbit:case @echoed:@billing.log \W)' --text 'req fa3b start; req c91d start; req E5E5 end'
[20..24] "c91d"
[36..40] "E5E5"

$ cat alerts.log
alert 10.0.0.201 scanned the subnet

$ trex scan '@echoed:@alerts.log \I' --text 'login from 10.0.0.7 by bob'
no match

$ trex scan '(?orbit:subnet/24 @echoed:@alerts.log \I)' --text 'login from 10.0.0.7 by bob'
[11..19] "10.0.0.7"

$ trex scan '@super:call \W' --text 'x = 1 ; foo(a, b) ; y : 2'
[8..11] "foo"

$ trex scan '@seam:token \W' --text 'let x = 1 ; let y = 2 ; print x ; print y ;'
[0..3] "let"
[4..5] "x"
[34..39] "print"
```

### Thresholds read from the stream

A magnitude predicate can be relative to a [context](../axes/context/) the rolling context
keeps at every token, instead of a number written into the pattern. A signed delta makes it
relative; `:name` after the delta picks the context.

| Form | Holds when |
|---|---|
| `\N{>+1}` | the token is an order of magnitude above the mean of the window of tokens before it |
| `\N{>+2s}` | two of that window's standard deviations above its mean |
| `\N{<-1}` | an order of magnitude below the window's mean |
| `\N{>+1:phase}` | an order above the earlier values in the token's column of a periodic record |
| `\N{>+1:regime}` | an order above the tokens since the byte grain's last regime change |
| `\N{>+1:echo}` / `\N{>+1:enclosing}` | an order above the token's earlier occurrences / the heads of the brackets enclosing it |
| `"key":k "=" \N{>+1:k}` | an order above the values bound by `=` or `:` to earlier occurrences of the key register `k` holds |
| `@phase:k` | zero-width anchor at column `k` of a periodic record, counted in significant tokens, no delimiter named |
| `@phase:k/p` / `@phase:k#n` | column `k` of the `p`-token period, or of the `n`-th strongest, where that period is live in the stream by the gate the strongest must clear; `#1` is the period plain `@phase:k` counts in. Two periods of nearly one strength can trade ranks between slices of one input, so `#n` can name a different period on another slice where `/p` cannot |

```console
$ trex scan '"latency":k "=" \N{>+1:k}' --text 'latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;'
[64..79] "latency = 12000"  captures: k="latency"

$ trex scan '@phase:2 \N' --text 'a , 1 , x ; b , 2 , y ; c , 3 , z ; d , 4 , w ;'
[4..5] "1"
[16..17] "2"
[28..29] "3"
[40..41] "4"
```

A context with nothing in it is no baseline, so the first occurrence of a key and the first
token of a stream never match a relative form, and a sigma form needs a spread to measure in.

## Declared names and the library

`\{name}` resolves in this order: a built-in kind, a name you declared, then the shipped
library, so a declaration shadows a library entry of its name and nothing may take a
built-in's. A declaration is one of three things.

A **shape** is a token kind from a bounded byte-pattern the lexer runs alongside its built-in
recognizers. `--shape 'name = `pat`'` tries it before the built-ins and wins an overlap;
`--shape-after` tries it only where no built-in matched. Its pattern must have a fixed maximum
length (`{m,n}` rather than `+`).

```console
$ trex scan '\{ticket}' --shape 'ticket = `[A-Z]{2,4}-\d{1,4}`' --text 'see AB-12 and XYZ-9 now'
[4..9] "AB-12"
[14..19] "XYZ-9"
```

A **kind from a pattern** is a token kind from a pattern over the stream: after the lex, the
tokens each match covers fuse into one token of the kind, brackets are paired again over
what remains, and a later pattern reads the kind as one atom. A **sub-pattern** is a name for
a pattern the parser inlines where `\{name}` appears, so its bindings and quantifiers compose
as if written in place.

A pattern file declares all three, one a line: `let NAME = PATTERN`, `kind NAME = PATTERN`,
`shape NAME = `BYTES`` and `shape-after NAME = `BYTES``, with blank lines and `#` comments
skipped. `--lib FILE` reads it on `scan` and `rewrite`; in the library the same text goes
through `ShapeSet::declare_text`, and one declaration at a time through `declare`,
`declare_kind` and `declare_let`. A later `let` of a name shadows an earlier one. With
`defs.trex` holding the two lines `let rhs = \N | \Q` and `kind assign = \W "=" \{rhs}`:

```console
$ trex scan '\{assign}' --lib defs.trex --text 'let x = 1; name = "bob"'
[4..9] "x = 1"
[11..23] "name = \"bob\""
```

A `test NAME accepts "text"... rejects "text"...` line beside the declarations states what a
name matches: it accepts a text when its match in the text is the whole of it, from the first
significant token to the last, and rejects a text when it matches nowhere in it. Either
keyword takes any number of double-quoted texts, which read `\"`, `\\`, `\n` and `\t`, and a
name is any declared shape, kind or sub-pattern or a library entry, tested as the file finally
declares it, so a test may stand above the line it checks. `trex lib --test FILE` runs them and
prints each expectation not met as a `FILE:LINE:` line; the [command reference](../cli/#lib)
shows a run.

A **rule** is a named pattern with what a finding of it says. As a block:

```text
rule private_ip
  pattern = \{ip_private}:addr
  message = private address ${addr} in ${path:name}
  severity = warning
  fix = ${addr:octet1-2}.x.x
  files = *.conf, !test_*
  meta.cwe = CWE-200
  meta.tags = network, config
```

or as one line, `rule NAME [error|warning|note] "message" = PATTERN`, the message quoted as a
test's texts are, with `fix NAME = TEMPLATE`, `meta NAME KEY = VALUE`, `files NAME = GLOBS`,
`unless NAME = PATTERN`, `record NAME = UNIT`, `record-start NAME = PATTERN` and `record-span
NAME = PATTERN` lines below it. A rule has a `pattern` and a `message`; its `severity` is
`error`, `warning` or `note`, a warning where it says nothing. The message is a report
template as `scan --format` takes one, rendered at each finding with the match's registers
and their typed slices, `${path}`, `${line}`, `${col}`, `${rule}` and `${severity}`; the fix
is a rewrite template rendered in the match's place. `files` keeps the rule to the inputs its
globs keep, as `-g` reads them. `unless` names a pattern the record must not hold and
`record` what a record is, as `--record` takes it, so the rule fires on a record holding the
pattern and none of the `unless` patterns, a line where `unless` stands alone; `record-start`
and `record-span` define the record by a pattern as the flags do. `meta.KEY` lines are free
metadata a pipeline reads. The rule is a sub-pattern under its name, so `\{private_ip}` reads
it in a later pattern and a `test` line checks it, and its name is a declaration's: not a
built-in atom's, and declared once. `ShapeSet::rules` reads the rules back; the [command
reference](../cli/#scan) shows `scan --rules` reporting findings.

The **shipped library** is seventy-five entries reached with no declaration. Twenty-one are
kinds with a bounded shape, twelve of them checked as they are lexed: eleven by their
standard's checksum, `\{iban}` (mod 97-10 and the registry's length per country), `\{isbn}`
(ISBN-10 mod 11 or ISBN-13), `\{vin}`, `\{isin}`, `\{ean13}`, `\{upca}`, `\{ean8}`,
`\{imei}`, `\{ethaddr}` (EIP-55 case over Keccak-256), `\{btcaddr}` (base58check, bech32 and
bech32m) and `\{github_token}` (the CRC-32 its last six characters carry over the thirty before
them, for the `gh?_` forms; a fine-grained `github_pat_` token is laid out differently and
passes on its shape, since a guard that cannot check a token must not refuse it), and
`\{k8s_name}` by its length, at most sixty-three characters; by shape alone `\{awskey}`,
`\{slack_token}`, `\{google_key}`, `\{stripe_key}`, `\{twilio_key}`, `\{cve}`, `\{mime}`,
`\{docker_image}` and `\{git_sha}`. Sixteen are sub-patterns over tokens: `\{private_key}` (a
PEM block from BEGIN to END), `\{ip_private}`, `\{ip_loopback}` and `\{ip_linklocal}` through
`\I{in:...}`, `\{log_level}`, `\{http_method}`, `\{http_status}` and `\{http_1xx}` to
`\{http_5xx}` through `\N{a..b}`, `\{currency}`, `\{country}`, `\{weekday}` and `\{month}`.
Sixteen are the unit kinds a [quantity](#quantities) reads from the context: `\{kelvin}`,
`\{inch}`, `\{meter}`, `\{second}`, `\{hour}`, `\{gram}`, `\{tonne}`, `\{ampere}`, `\{volt}`,
`\{watt}`, `\{newton}`, `\{joule}`, `\{calorie}`, `\{liter}`, `\{gallon}` and `\{bar}`. The last
twenty-two are the cues those read, a word list and a sub-pattern for each of eleven families,
from `\{temperature_word}` and `\{temperature_cue}` to `\{pressure_word}` and
`\{pressure_cue}`. `trex lib` lists them with what each matches. A library kind is lexed only
for a pattern that names it: thirteen digits with a valid check are a number to `\N` and an
EAN-13 to `\{ean13}`.

```console
$ trex scan '\{iban}' --text 'pay GB82 WEST 1234 5698 7654 32 or DE89370400440532013000; not GB82WEST12345698765433'
[4..31] "GB82 WEST 1234 5698 7654 32"
[35..57] "DE89370400440532013000"

$ trex scan '\{log_level} \W+ \{http_5xx}' --text 'WARN upstream 502 retry; INFO served 200'
[0..17] "WARN upstream 502"
```

## Rewrite accessors

In a `rewrite` template, `${name}` renders a capture, `${0}` the whole match, and `${1}`,
`${2}`, ... a capture by position - the first, second, ... name the pattern binds, since a
logical group `(...)` binds nothing. `${name:acc}` (chained with `|`) transforms or slices a
capture. Accessors are the transforms `upper` / `lower`, the slices `trim` / `firstN` /
`lastN` of any capture, and the typed sub-field extractors that slice a captured atom by its
known shape:

| On | Accessors |
|---|---|
| any | `firstN` / `lastN`: the first or last N characters |
| `\I` (IPv4) | `octetN` / `octetN-M` |
| `\I` (IPv6) | `groupN` / `groupN-M` |
| `\U` | `scheme` / `host` / `port` / `path` / `query` |
| `\E` | `user` / `domain` |
| `\V` | `major` / `minor` / `patch` |
| `\T` | `year` / `month` / `day` / `hour` / `minute` / `second` |
| `\L` | `dir` / `name` / `ext` |
| `\{qty}` `\{quantity}` `\Z` `\R` `\%` | `value` / `unit`, each as written |

So `${ip:octet1-2}` keeps the first two IPv4 octets. Because the capture is a typed atom, the
slice is a lookup of a known shape, not a second match. The same accessors name what
`trex redact --keep` leaves readable, where a slice is the byte range it locates rather than
the text it renders: `--keep 'card:last4'` keeps the four characters `${card:last4}` would
render, where they stand. In a template `$$` is a dollar sign and `\n`, `\t` and `\\` are a
newline, a tab and a backslash; a report's template (`scan --format`) also writes where the
match stands as `${path}`, `${line}`, `${col}`, `${start}` and `${end}`, and any reading
`--explain` computes for the match as `${@axis}` or `${@axis.piece}` - `${@kind}`,
`${@guard}`, `${@route}` and one per property axis. A rewrite's template takes neither: the
bytes it splices in stand where the match stood, with no place and no explanation to read.

```console
$ trex rewrite '\W:a "=" \N:b' '${2} := ${1}' --text 'x = 1'
1 := x
```

## Precedence

Tightest to loosest: atoms / classes / groups, with assertions and guards among them;
quantifiers (postfix, then the lazy `?` or possessive `+` suffix); binding suffix `:name`;
concatenation; choice (`|`, `||`, `|>`). An assertion or guard (`~"lit"`, `~(P)`, `~<(P)`,
`!~...`) is a zero-width item of a sequence like any atom, so `"x" ~("y") "y"` is three items
and a quantifier or binding written after one applies to it.

## What regex has and trex spells differently

| Regex | trex |
|---|---|
| `[a-z]` | `` `[a-z]` `` inside a token |
| `[^\N]` | a token class |
| `a\|b` | `\|`, with `\|\|` and `\|>` for the other two choices |
| `\1` | `=name`; `=shape name` up to a symmetry; `=ip name` up to a representation; `=subnet name` up to a typed relation |
| `(?i)` | `(?orbit:case ...)` |
| `(?=P)` `(?!P)` `(?<=P)` `(?<!P)` | `~(P)` `!~(P)` `~<(P)` `!~<(P)` |
| nothing | `~>k{m,n}(P)` and `~#{m,n}(P)`: counting over a window and over a region |
| `\b` | `@seam`, a predictive boundary rather than a character-class edge |
| `^` `$` `\A` `\z` `\G` `\K` | the same spellings, over significant tokens |
| `(?>P)` `P*+` | the same spellings |
| `$1` | `${1}` in a template |
| `\p{L}` | inside a byte-pattern |
| `(?R)` recursion, `(?(c)a\|b)` conditionals, `(?&r)` subroutine calls | `\B(...)` as a primitive, and the grammar engine for the rest; conditionals and subroutines are deliberately absent |
