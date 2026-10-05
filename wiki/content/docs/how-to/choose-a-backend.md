---
title: Choose a backend
linkTitle: Choose a backend
weight: 60
---

Every backend finds the same matches; the choice changes how long a scan takes. Without a
choice a scan places itself by what the process has measured.

## Keep a scan on the CPU

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\N' --text 'n 42' --cpu
[2..4] "42"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let pat = trex::parse(r"\N").expect("valid pattern");
let (spans, used) = trex::scan_with_backend(&pat, b"n 42", trex::Backend::Cpu);
assert_eq!(spans.len(), 1);
assert_eq!(used, trex::BackendUsed::Cpu);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> import trex
>>> [m.text for m in trex.Pattern(r"\N").scan("n 42", backend="cpu")]
['42']
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\N' -InputObject 'n 42' -Backend Cpu -Raw
42
```
{{< /tab >}}
{{< /tabs >}}

`--cpu` never probes a device. A build with `--no-default-features` carries no device code at
all.

## Use the device

For a pattern that reads only each token's kind, with a CUDA device present, the
automatic choice times the CPU engine against a split of the input's anchors between the cores
and the device in each power-of-two band of input size, runs whichever was faster in that band,
and times both again every thirty-second call. A pattern that tests a magnitude, a byte class, a
literal, a register or a spectral property also runs on the device with the same matches, but a
single device scan of it is no faster than the CPU engine (five spectral patterns over 16 MB:
0.98x on an RTX 5070), so the automatic choice keeps it on the cores. A text scanned many times
is uploaded once through `GpuTokens` in Rust, and the same five scans then run 4.8x
faster than the engine.

`--gpu`, `trex::Backend::Gpu`, `backend="gpu"` and `-Backend Gpu` force a device attempt, which
warns and falls back to the CPU where the device is absent or cannot take the pattern. `trex::device_available()`,
`trex.device_available()` and `Get-TrexInfo` say whether a device is present.

The device backend is on Windows and Linux, the systems NVIDIA ships a CUDA toolkit for. trex's
releases for them are built with CUDA 12.0 and load their kernel on an NVIDIA driver of the 525
series or later; the macOS and FreeBSD releases run every scan on the CPU.

## Feed the input in pieces

`--chunk-size N` feeds the input through the [stream scanner](../../reference/tools/#streams) in
N-byte chunks, and `--dual-grain` runs lexing and matching as a pipeline on two threads; both
give the matches of a whole scan.

```console
$ cat access.log
10.0.0.5 GET /index.html 200 5120
10.0.0.7 GET /login 302 0
10.0.1.9 GET /missing 404 312
10.0.0.5 POST /login 200 88
```

{{< tabs >}}
{{< tab name="CLI" >}}
```console
$ trex scan '\I' access.log --chunk-size 7
[0..8] "10.0.0.5"
[34..42] "10.0.0.7"
[60..68] "10.0.1.9"
[90..98] "10.0.0.5"
```
{{< /tab >}}
{{< tab name="Rust" >}}
```rust
let log = b"10.0.0.5 GET /index.html 200 5120\n10.0.0.7 GET /login 302 0\n10.0.1.9 GET /missing 404 312\n10.0.0.5 POST /login 200 88\n";
let pat = trex::parse(r"\I").expect("valid pattern");
let whole = trex::scan(&pat, log);
assert_eq!(trex::scan_chunked(&pat, log.chunks(7)), whole);
assert_eq!(trex::scan_dual_grain(&pat, log).0, whole);
```
{{< /tab >}}
{{< tab name="Python" >}}
```python
>>> log = open("access.log").read()
>>> p = trex.Pattern(r"\I")
>>> [m.text for m in p.scan(log, chunk_size=7)]
['10.0.0.5', '10.0.0.7', '10.0.1.9', '10.0.0.5']
>>> [m.span() for m in p.scan(log, dual_grain=True)] == [m.span() for m in p.scan(log)]
True
```
{{< /tab >}}
{{< tab name="PowerShell" >}}
```powershell
PS> Select-TrexMatch '\I' -Path ./access.log -ChunkSize 7 -Raw
10.0.0.5
10.0.0.7
10.0.1.9
10.0.0.5
```
{{< /tab >}}
{{< /tabs >}}

## Compressing

`trex compress`, in a build with the `compress` feature, has its own backend choice. The coder
learns as it reads, so splitting the input gives up some of what it learns, and the sequential
CPU coder gives the shortest code length. `--chunks N` codes N slices across the cores, each
seeded from the baked prior; `--gpu` codes one chunk a CUDA thread, with the prior projected into
the device's tables, and `--no-baked` runs it without the prior:

```console
$ trex compress --gpu moby.txt
trex compress: 1234609 bytes -> 357664 bytes  (2.318 bits/byte, 29.0% of original)
  coder: GPU chunked coder (context + orbit + match)   0.17 MB/s   (--compare for the full table)
$ trex compress --gpu --no-baked moby.txt
trex compress: 1234609 bytes -> 451180 bytes  (2.924 bits/byte, 36.5% of original)
  coder: GPU chunked coder (context + orbit + match)   0.45 MB/s   (--compare for the full table)
```

The sizes are the model's code length rounded up to bytes; no compressed stream is written. The
first device run in a process builds the projected prior, a 1.1 GB table, and holds it on the
device for later calls. `TREX_GPU_CHUNK` sets the chunk size, a larger chunk carrying more
context and fewer threads, and `TREX_GPU_OVERLAP` warms each chunk on the bytes before it.
`--hybrid` runs the CPU and the device at once on a head and tail split.
