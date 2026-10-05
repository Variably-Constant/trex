---
title: Find where text changes kind
linkTitle: Find where text changes kind
weight: 25
---

Find the stretch of an input that is code inside prose, or data inside code, from the bytes
alone, with no delimiter or fence named ([spectral](../../reference/axes/spectral/)).

The examples read one passage with a line of code in the middle of a sentence:

```text
The nightly report lists every host that answered within the window and the latency it measured
for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the
payload, and it sends the totals back to the collector before the next window opens, so the
morning shift starts with fresh numbers.
```

## The tokens of one texture

`\F{texture:code}+` is a run of tokens whose surrounding bytes read as code:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\F{texture:code}+' --text 'The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.'
[144..207] "*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\F{texture:code}+").expect("valid pattern");
let text = "The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> text = "The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers."
>>> [m.text for m in trex.Pattern(r"\F{texture:code}+").scan(text)]
['*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> $text = 'The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.'
PS> Select-TrexMatch '\F{texture:code}+' -InputObject $text -Raw
*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it
```
{{< /tab >}}
{{< /tabs >}}

The texture is read forward, from a running average whose memory is about thirty-two bytes, so
the run starts a few tokens into the code, once enough of it has arrived, and runs a few words
past it, until the prose has pushed it back out. `\F{texture:prose}`, `:math` and `:data` name
the other textures.

## Where it changes

`\F{onset}` holds on a token with a change point inside it. The reader places one where the code
begins to read as code and one where the reading has come back to the prose before it:

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\F{onset}' --text 'The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.'
[138..139] "="
[192..199] "payload"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\F{onset}").expect("valid pattern");
let text = "The nightly report lists every host that answered within the window and the latency it measured for each one. For every reply it runs a[i]=(b[j]*c[k]+d[i-1])>>2;if(a[i]>m){m=a[i];k=i;} on the payload, and it sends the totals back to the collector before the next window opens, so the morning shift starts with fresh numbers.";
let found: Vec<&str> = trex::scan(&pat, text.as_bytes()).iter().map(|s| &text[s.range()]).collect();
assert_eq!(found, ["=", "payload"]);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> [(m.text, m.start) for m in trex.Pattern(r"\F{onset}").scan(text)]
[('=', 138), ('payload', 192)]
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\F{onset}' -InputObject $text -Raw
=
payload
```
{{< /tab >}}
{{< /tabs >}}

`trex spectral --segment` lists the change points themselves, 138 and 192 here; the
[spectral](../../reference/axes/spectral/#change-points) page explains where each one is and
why the second comes a few bytes after the code ends.

## The regions of a file

`trex spectral --classify`, `Measure-TrexSpectral -Classify` and
`trex.axes.spectral(classify=True)` cut a whole input into its regions and name each: `table`,
`code`, `prose`, `blob`, `numeric` or `mixed`. The
[find tables](../find-tables/) how-to walks a tree by the same reading.
