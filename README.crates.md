# trex-re

[![crates.io](https://img.shields.io/crates/v/trex-re?style=flat-square)](https://crates.io/crates/trex-re)
[![docs.rs](https://img.shields.io/docsrs/trex-re?style=flat-square)](https://docs.rs/trex-re)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square)](https://github.com/Variably-Constant/trex/blob/main/LICENSE-MIT)

TREX (Token Regular EXpression) is a regex-shaped pattern language over typed tokens. The lexer reads the input once into typed tokens and a pattern matches over them: a whole number, word, quoted string, IP address, URL, email address, timestamp, version and 18 more kinds are one atom each, and the whitespace between tokens is never written. A capture binds a named register that a later token must equal, a balanced bracket group matches whole, and matching does not backtrack. Twelve property axes read what the input does at each position, and a pattern can ask any of them there.

This crate is the Rust library and the `trex` command. The package is `trex-re`, and the library it names in code is `trex`:

```toml
[dependencies]
trex-re = "0.3.0"
```

A register binds a token that a later `=name` must equal, so `<\W:t>.*</=t>` matches an element closed by its own tag. A value predicate compares a token's value, so `\N{>=100}` finds the numbers of 100 or more. A rewrite template reads a capture's typed fields, such as an email address's domain:

```rust
let tag = trex::parse(r"<\W:t>.*</=t>").expect("valid pattern");
let html = b"say <div>hi</div> now";
let m = &trex::captures(&tag, html, &trex::scan(&tag, html))[0];
assert_eq!((&html[m.start..m.end], m.group("t", html)), (&b"<div>hi</div>"[..], Some(&b"div"[..])));

let big = trex::parse(r"\N{>=100}").expect("valid pattern");
let nums = b"5 50 500 5000";
let found: Vec<&[u8]> = trex::scan(&big, nums).iter().map(|s| &nums[s.range()]).collect();
assert_eq!(found, [&b"500"[..], &b"5000"[..]]);

let mail = trex::parse(r"\E:e").expect("valid pattern");
let tmpl = trex::Template::parse("[${e:domain}]", &mail.capture_names()).expect("valid template");
assert_eq!(trex::rewrite(&mail, &tmpl, b"mail bob@x.com now"), b"mail [x.com] now");
```

## The command

`cargo install trex-re` installs `trex`, which scans files, directories, standard input or `--text`:

```console
$ trex scan '\N{>=100}' --text '5 50 500 5000'
[5..8] "500"
[9..13] "5000"
$ trex rewrite '\E:e' '[${e:domain}]' --text 'mail bob@x.com now'
mail [x.com] now
```

Binaries for Windows x64, Linux x64, macOS arm64 and FreeBSD x64 are on the [latest GitHub release](https://github.com/Variably-Constant/trex/releases/latest).

## Feature flags

| Feature | Default | Adds |
|---|---|---|
| `gpu` | on | the CUDA scan backend through `cudarc`; without an `nvcc` of CUDA 12.0 or later at build time the binary runs on the CPU |
| `tandem` | on | CPU and GPU batch dispatch through the scheduler's hybrid join; implies `gpu` |
| `compress` | off | `trex compress`, the coder options of `trex seam`, and the 21.6 MB byte-ngram prior they read; it builds from a checkout, since the published crate leaves the prior out |

`--no-default-features` builds with no CUDA dependency at all.

## Requirements

Rust 1.96 or later, edition 2024. The `gpu` feature compiles the CUDA kernel with an `nvcc` of CUDA 12.0 or later where one is installed and loads it through the NVIDIA driver at run time; a build without one, or a machine without a device, runs on the CPU.

## Documentation

- [The documentation](https://variably-constant.github.io/trex/): tutorials, how-to guides and the reference
- [Pattern syntax](https://variably-constant.github.io/trex/docs/reference/pattern-syntax/)
- [The Rust API by task](https://variably-constant.github.io/trex/docs/reference/rust/) and the [generated reference](https://docs.rs/trex-re)
- The [GitHub README](https://github.com/Variably-Constant/trex#readme) for the Python and PowerShell modules, the architecture and the benchmarks

## License

MIT. See [LICENSE-MIT](https://github.com/Variably-Constant/trex/blob/main/LICENSE-MIT).
