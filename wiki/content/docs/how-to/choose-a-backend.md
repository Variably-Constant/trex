---
weight: 60
---

# Choose a backend

`scan` and `rewrite` pick a backend automatically. You rarely need to override it, but the
knobs are here when you do.

## CPU (the default engine)

The default path runs the single-pass engine, falling back to set-reachability for balanced,
field, or axis patterns. Force it - never probing the device - with `--cpu` (or `--nogpu`):

```console
$ trex scan '\N' --text 'n 42' --cpu
[2..4] "42"
```

## GPU

The automatic choice places each scan by what the process has measured. For a pattern that reads
nothing of a token but its kind, when a CUDA device is present, it times the CPU engine against a
split of the input's anchors between the cores and the device in each power-of-two band of input
size, runs whichever it measured faster in that band, and times both again every thirty-second
call. A pattern that tests a magnitude, a
byte class, a literal, a register or a spectral property can also run on the device, with the
same matches, but a per-call device scan of it is no faster than the CPU engine (five spectral
patterns over 16 MB: 0.98x on an RTX 5070), so the automatic choice keeps it on the cores. A
text scanned many times pays for its upload once through `GpuTokens` in the library
(`upload_with_spectral` holds the spectral reading too): the same five scans run 4.8x faster
than the engine on held tokens. Force a device attempt with `--gpu` (it warns and falls back if the device is
unavailable or cannot represent the pattern). The GPU backend is baked in by default and auto-detects the device;
build with `--no-default-features` for a pure-CPU binary that pulls in no CUDA dependency at
all.

## Compressing

The `compress` coder is in a build with the `compress` feature only
(`cargo build --release --features compress`). It has its own backend dial, and it trades
differently than scanning: a context mixer learns as it reads, so anything that splits the input
into independent chunks gives up some of that learning. Sequential CPU is the best ratio.
Everything else buys speed.

`--chunks N` codes N slices across cores (0 = every logical core), each seeded from the baked
prior so a chunk does not start cold; the ratio cost is a few percent. `--gpu` maps one chunk
to one CUDA thread, thousands at once. It reads the baked prior too, projected into the
device's tables, and `--no-baked` runs it without:

```console
$ trex compress --gpu moby.txt
trex compress: 1234609 bytes -> 357664 bytes  (2.318 bits/byte, 29.0% of original)
  coder: GPU chunked coder (context + orbit + match)   0.17 MB/s   (--compare for the full table)
$ trex compress --gpu --no-baked moby.txt
trex compress: 1234609 bytes -> 451180 bytes  (2.924 bits/byte, 36.5% of original)
  coder: GPU chunked coder (context + orbit + match)   0.45 MB/s   (--compare for the full table)
```

The first device compress in a process builds the projected prior - a 1.1 GB table - and holds
it on the device for later calls, which is most of the gap in speed on an input this small.

Two dials govern it. `TREX_GPU_CHUNK` sets the chunk size, and bigger chunks carry more
context, so the ratio improves as the chunk grows while the thread count (and the speedup)
shrinks. `TREX_GPU_OVERLAP` warms each chunk on the bytes just before it, which recovers most
of what chunking loses. The defaults scale both with the input. `--hybrid` runs the CPU and
the device at the same time on a head/tail split; it earns its keep only on inputs large
enough that the device's throughput dominates.

## Streaming

Feed the input in fixed-size chunks and recover the exact whole-input match set with
`--chunk-size N`. A match commits only once no later byte can change it, so the streamed result
equals a whole-input scan.

## Dual-grain

`--dual-grain` runs the byte grain (lexing) and the token grain (matching) as a
producer/consumer pipeline on two threads. The matches are identical to a plain scan; the
overlap is what the pipeline buys on a large input.
