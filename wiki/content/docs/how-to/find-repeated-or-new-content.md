---
title: Find repeated or new content
linkTitle: Find repeated or new content
weight: 23
---

# Find repeated or new content

Find the content that recurs in an input, the content seen for the first time, and the content
one log holds that another does not ([echo](../../reference/axes/echo/)).

## First sightings within an input

`@novel` holds on the first occurrence of a token's content, and `@echoed` on content that occurs
again elsewhere:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@novel \W' --text 'cat dog cat bird dog cat'
[0..3] "cat"
[4..7] "dog"
[12..16] "bird"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"@novel \W").expect("valid pattern");
let text = "cat dog cat bird dog cat";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["cat", "dog", "bird"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"@novel \W").scan("cat dog cat bird dog cat")]
['cat', 'dog', 'bird']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@novel \W' -InputObject 'cat dog cat bird dog cat' -Raw
cat
dog
bird
```
{{< /tab >}}
{{< /tabs >}}

`@echo>k` counts the occurrences and `@echo:period` holds on content recurring at a regular
spacing.

## What one log has that another lacks

`@novel:@FILE` holds on content found nowhere in a second input, read once when the pattern is
parsed. The address new since yesterday:

```console
$ cat today.log
login from 10.0.0.5
login from 10.0.0.9
login from 10.0.0.12
$ cat yesterday.log
login from 10.0.0.5
login from 10.0.0.9
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '@novel:@yesterday.log \I' today.log
[51..60] "10.0.0.12"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let yesterday = b"login from 10.0.0.5\nlogin from 10.0.0.9\n";
let today = "login from 10.0.0.5\nlogin from 10.0.0.9\nlogin from 10.0.0.12\n";
let pat = trex::parser::parse_with_inputs(r"@novel:@yesterday.log \I", &trex::ShapeSet::new(), &[("yesterday.log", &yesterday[..])]).expect("valid pattern");
let found: Vec<&str> = trex::scan(&pat, today.as_bytes()).iter().map(|s| &today[s.range()]).collect();
assert_eq!(found, ["10.0.0.12"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [m.text for m in trex.Pattern(r"@novel:@yesterday.log \I").scan(open("today.log").read())]
['10.0.0.12']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '@novel:@yesterday.log \I' -Path ./today.log -Raw
10.0.0.12
```
{{< /tab >}}
{{< /tabs >}}

`@echoed:@FILE` holds on content the second input also has. Inside `(?orbit:subnet/24 ...)` the
comparison reads an address by its network.
