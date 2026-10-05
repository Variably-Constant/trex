---
title: Match balanced groups and what they hold
linkTitle: Match balanced groups
weight: 13
---

Match a bracket group whole however deeply it nests, keep the groups that hold something, and
bind a name only inside its own group ([pattern syntax](../../reference/pattern-syntax/#binding-back-reference-and-symmetry)).

```console
$ cat calls.py
log(fetch(url, retries=3), level="info")
save(record)
retry(fetch(url, retries=5), backoff=2.5)
$ cat keys.json
[{"id": 1, "name": "ann", "id": 7}, {"id": 2, "name": "bob"}, {"name": "cy", "id": 3}]
```

## A call, whatever its arguments nest

`\B(...)` is a balanced paren group whose interior matches the pattern inside it, and `@call` an
identifier followed by one. A call nested in an argument stays inside the outer match:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@call' calls.py
[0..40] "log(fetch(url, retries=3), level=\"info\")"
[41..53] "save(record)"
[54..95] "retry(fetch(url, retries=5), backoff=2.5)"

$ trex scan '"fetch" \B(.*)' calls.py
[4..25] "fetch(url, retries=3)"
[60..81] "fetch(url, retries=5)"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let code = "log(fetch(url, retries=3), level=\"info\")\nsave(record)\nretry(fetch(url, retries=5), backoff=2.5)\n";
let found = |src: &str| -> Vec<&str> {
    let pat = trex::parse(src).expect("valid pattern");
    trex::scan(&pat, code.as_bytes()).iter().map(|s| &code[s.range()]).collect()
};
assert_eq!(found("@call"), ["log(fetch(url, retries=3), level=\"info\")", "save(record)", "retry(fetch(url, retries=5), backoff=2.5)"]);
assert_eq!(found(r#""fetch" \B(.*)"#), ["fetch(url, retries=3)", "fetch(url, retries=5)"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> code = trex.read("calls.py")
>>> [m.text for m in trex.Pattern("@call").scan(code)]
['log(fetch(url, retries=3), level="info")', 'save(record)', 'retry(fetch(url, retries=5), backoff=2.5)']
>>> [m.text for m in trex.Pattern(r'"fetch" \B(.*)').scan(code)]
['fetch(url, retries=3)', 'fetch(url, retries=5)']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@call' -Path ./calls.py -Raw
log(fetch(url, retries=3), level="info")
save(record)
retry(fetch(url, retries=5), backoff=2.5)

PS> Select-TrexMatch '"fetch" \B(.*)' -Path ./calls.py -Raw
fetch(url, retries=3)
fetch(url, retries=5)
```
{{< /tab >}}
{{< /tabs >}}

`\B[...]` and `\B{...}` are the other bracket kinds, `@block` a balanced brace group and
`@nesting` a group of any kind.

## The groups that hold something

`~#(A)`, written where a group opens, holds when `A` matches somewhere inside that group, and
`!~#(A)` when it matches nowhere, however deep:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\W ~#(\N{>=5}) \B(.*)' calls.py
[54..95] "retry(fetch(url, retries=5), backoff=2.5)"

$ trex scan '\W !~#("retries") \B(.*)' calls.py
[41..53] "save(record)"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let code = "log(fetch(url, retries=3), level=\"info\")\nsave(record)\nretry(fetch(url, retries=5), backoff=2.5)\n";
let found = |src: &str| -> Vec<&str> {
    let pat = trex::parse(src).expect("valid pattern");
    trex::scan(&pat, code.as_bytes()).iter().map(|s| &code[s.range()]).collect()
};
assert_eq!(found(r"\W ~#(\N{>=5}) \B(.*)"), ["retry(fetch(url, retries=5), backoff=2.5)"]);
assert_eq!(found(r#"\W !~#("retries") \B(.*)"#), ["save(record)"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"\W ~#(\N{>=5}) \B(.*)").scan(code)]
['retry(fetch(url, retries=5), backoff=2.5)']
>>> [m.text for m in trex.Pattern(r'\W !~#("retries") \B(.*)').scan(code)]
['save(record)']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\W ~#(\N{>=5}) \B(.*)' -Path ./calls.py -Raw
retry(fetch(url, retries=5), backoff=2.5)

PS> Select-TrexMatch '\W !~#("retries") \B(.*)' -Path ./calls.py -Raw
save(record)
```
{{< /tab >}}
{{< /tabs >}}

`~#{m,n}(A)` asks for between `m` and `n` places inside the group where `A` starts a match.

## A name bound inside one group

`A::name` binds a register for the rest of the enclosing balanced group and drops it at the
group's close, so a back-reference to it never reaches past the group. A key repeated inside one
object:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\B{ .* \Q::k ":" .* =k ":" .* }' keys.json
[1..34] "{\"id\": 1, \"name\": \"ann\", \"id\": 7}"  captures: k=""
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let json = r#"[{"id": 1, "name": "ann", "id": 7}, {"id": 2, "name": "bob"}, {"name": "cy", "id": 3}]"#;
let pat = trex::parse(r#"\B{ .* \Q::k ":" .* =k ":" .* }"#).expect("valid pattern");
let found: Vec<&str> = trex::scan(&pat, json.as_bytes()).iter().map(|s| &json[s.range()]).collect();
assert_eq!(found, [r#"{"id": 1, "name": "ann", "id": 7}"#]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> keys = trex.read("keys.json")
>>> [m.text for m in trex.Pattern(r'\B{ .* \Q::k ":" .* =k ":" .* }').scan(keys)]
['{"id": 1, "name": "ann", "id": 7}']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\B{ .* \Q::k ":" .* =k ":" .* }' -Path ./keys.json -Raw
{"id": 1, "name": "ann", "id": 7}
```
{{< /tab >}}
{{< /tabs >}}

The binding is dropped when its group closes, so the match reports `k` empty. A plain `\Q:k`
outside a group reaches across objects instead, from the first `"id"` to one in the third
object:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\Q:k ":" .* =k ":"' keys.json
[2..82] "\"id\": 1, \"name\": \"ann\", \"id\": 7}, {\"id\": 2, \"name\": \"bob\"}, {\"name\": \"cy\", \"id\":"  captures: k="\"id\""
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let json = r#"[{"id": 1, "name": "ann", "id": 7}, {"id": 2, "name": "bob"}, {"name": "cy", "id": 3}]"#;
let pat = trex::parse(r#"\Q:k ":" .* =k ":""#).expect("valid pattern");
let m = &trex::captures(&pat, json.as_bytes(), &trex::scan(&pat, json.as_bytes()))[0];
assert_eq!((m.start, m.end, m.group("k", json.as_bytes())), (2, 82, Some(&br#""id""#[..])));
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> m = trex.Pattern(r'\Q:k ":" .* =k ":"').find(keys)
>>> m.start, m.end, m["k"]
(2, 82, '"id"')
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\Q:k ":" .* =k ":"' -Path ./keys.json | Select-Object Start, Length, @{ n = 'k'; e = { $_.Captures.k } }

Start Length k
----- ------ -
    2     80 "id"
```
{{< /tab >}}
{{< /tabs >}}
