"""Builds the parallel corpus in this folder from its sources at pinned revisions.

    python build.py

Each source is fetched at the revision named below and written as the
measurement examples read it, and MANIFEST.sha256 lists every file the build
writes with its digest, so a second build can be checked against the first.
SOURCES.md names each source and its terms.
"""

import hashlib
import io
import urllib.request
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
UDHR_REV = "588b3f4b2d0467aff54842a4b926551b69d5a66a"
BENCHMARKS_REV = "40296663ed350d5fe4a6ab5e367bab61cb77c219"
SQLITE_REV = "ccbdec8444e3a717dac8c154f17fdd7e80aa7827"

# UDHR in XML file stem -> the file written under prose/.
UDHR = {
    "afr": "afr",
    "arb": "arb",
    "bul": "bul",
    "cat": "cat",
    "ces": "ces",
    "cmn_hans": "cmn",
    "dan": "dan",
    "deu_1996": "deu",
    "ell_monotonic": "ell",
    "eng": "eng",
    "heb": "heb",
    "hin": "hin",
    "jpn": "jpn",
    "kor": "kor",
    "rus": "rus",
    "tha": "tha",
}

# Benchmarks Game language suffix -> the extension written under code/.
LANGUAGES = {
    "rust": "rs",
    "python3": "py",
    "node": "js",
    "gcc": "c",
    "go": "go",
    "ghc": "hs",
    "java": "java",
    "sbcl": "lisp",
    "csharp": "cs",
    "ruby": "rb",
    "php": "php",
    "perl": "pl",
}

SQLITE_FILES = {
    "ext/wasm/sql/000-mandelbrot.sql": "sql/mandelbrot.sql",
    "ext/wasm/sql/001-sudoku.sql": "sql/sudoku.sql",
    "test/fptest01.sql": "sql/fptest01.sql",
    "test/regexp1.sql": "sql/regexp1.sql",
    "test/sha1.sql": "sql/sha1.sql",
    "test/intck01.sql": "sql/intck01.sql",
    "test/imposter1.sql": "sql/imposter1.sql",
    "test/fossildelta.sql": "sql/fossildelta.sql",
}

BENCHMARKS = {
    "binarytrees",
    "fannkuchredux",
    "fasta",
    "knucleotide",
    "mandelbrot",
    "nbody",
    "pidigits",
    "regexredux",
    "revcomp",
    "spectralnorm",
}

NS = "{http://efele.net/udhr}"


def fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=120) as r:
        return r.read()


def udhr_text(xml: bytes) -> str:
    """Every title and paragraph of a UDHR translation in document order,
    one to a paragraph, each a single line."""
    root = ET.fromstring(xml)
    out = []
    for el in root.iter():
        if el.tag in (NS + "title", NS + "para"):
            text = " ".join("".join(el.itertext()).split())
            if text:
                out.append(text)
    return "\n\n".join(out) + "\n"


def write(rel: str, data: bytes, written: dict) -> None:
    path = HERE / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    written[rel] = hashlib.sha256(data).hexdigest()


def main() -> None:
    written: dict = {}
    base = f"https://raw.githubusercontent.com/eric-muller/udhr/{UDHR_REV}/data/udhr"
    for stem, name in UDHR.items():
        write(f"prose/{name}.txt", udhr_text(fetch(f"{base}/udhr_{stem}.xml")).encode("utf-8"), written)

    salsa = f"https://salsa.debian.org/benchmarksgame-team/benchmarksgame/-/raw/{BENCHMARKS_REV}"
    archive = zipfile.ZipFile(io.BytesIO(fetch(f"{salsa}/public/download/benchmarksgame-sourcecode.zip")))
    # The first implementation of each benchmark in each language. A
    # benchmark's first program in a language is the unnumbered
    # `bench/bench.lang` and the others are `bench/bench.lang-N.lang`, so the
    # unnumbered one is taken where the archive holds it and the lowest
    # numbered one where it does not.
    chosen: dict = {}
    for info in archive.infolist():
        parts = info.filename.split("/")
        if len(parts) != 2 or not parts[1]:
            continue
        bench, file = parts
        stem, _, lang = file.rpartition(".")
        if bench not in BENCHMARKS or lang not in LANGUAGES:
            continue
        if stem == bench:
            rank = -1
        elif stem.startswith(bench + "." + lang + "-") and stem.rsplit("-", 1)[1].isdigit():
            rank = int(stem.rsplit("-", 1)[1])
        else:
            continue
        key = (bench, lang)
        if key not in chosen or rank < chosen[key][0]:
            chosen[key] = (rank, info.filename)
    missing = [f"{b}-{l}" for b in sorted(BENCHMARKS) for l in LANGUAGES if (b, l) not in chosen]
    if missing:
        raise SystemExit(f"the archive holds no program for {', '.join(missing)}")
    for (bench, lang), (_, name) in sorted(chosen.items()):
        write(f"code/{bench}-{lang}.{LANGUAGES[lang]}", archive.read(name), written)
    write("code/LICENSE-benchmarksgame.md", fetch(f"{salsa}/LICENSE.md"), written)

    github = f"https://raw.githubusercontent.com/sqlite/sqlite/{SQLITE_REV}"
    for src, dst in SQLITE_FILES.items():
        write(dst, fetch(f"{github}/{src}"), written)

    manifest = "".join(f"{digest}  {rel}\n" for rel, digest in sorted(written.items()))
    (HERE / "MANIFEST.sha256").write_text(manifest, encoding="utf-8", newline="\n")
    print(f"{len(written)} files written")


if __name__ == "__main__":
    main()
