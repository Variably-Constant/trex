"""Every Python example the READMEs and the wiki show, run against the
installed module and compared with the output the page shows, as
tests/wiki_examples.rs holds the console examples to the trex binary.

A `python` block whose first line begins `>>> ` is read as doctest reads a
docstring. A page's blocks run in page order in one namespace, in a new
folder. A `$ cat NAME` line in a `console` block is a file, as
tests/wiki_examples.rs reads one: the lines under it up to the next `$ `
line are the text of NAME, written before any block below it. The files
under tests/documented that a page's blocks name are placed before any of
them."""

import doctest
import pathlib
import re
import shutil

import pytest

import trex

REPO = pathlib.Path(__file__).resolve().parents[2]
SEEDS = REPO / "tests" / "documented"


def documented_pages():
    """Each README and wiki page holding a `>>> ` line."""
    candidates = [REPO / "README.md", REPO / "python" / "README.md"]
    candidates += sorted((REPO / "wiki" / "content" / "docs").rglob("*.md"))
    return [p for p in candidates if re.search(r"^>>> ", p.read_text(encoding="utf-8"), re.M)]


def page_items(path):
    """The page's files and examples in page order: `("file", line, name,
    text)` for each `$ cat NAME` and `("example", line, source)` for each
    `python` block beginning `>>> `, lines counted from one."""
    lines = path.read_text(encoding="utf-8").splitlines()
    items = []
    i = 0
    while i < len(lines):
        fence = lines[i]
        if not fence.startswith("```"):
            i += 1
            continue
        end = i + 1
        while end < len(lines) and lines[end] != "```":
            end += 1
        body = lines[i + 1 : end]
        if fence == "```console":
            items += shown_files(path, body, i + 2)
        elif fence == "```python" and body and body[0].startswith(">>> "):
            items.append(("example", i + 2, "\n".join(body) + "\n"))
        i = end + 1
    return items


def shown_files(path, body, first):
    """The `$ cat NAME` files of one console block whose lines are `body`,
    the first of them at line `first`."""
    files = []
    j = 0
    while j < len(body):
        if not body[j].startswith("$ cat "):
            j += 1
            continue
        at = first + j
        name = body[j][len("$ cat ") :].strip()
        parts = pathlib.PurePosixPath(name).parts
        if pathlib.PurePosixPath(name).is_absolute() or ".." in parts or not parts:
            raise ValueError(f"{path}:{at}: $ cat {name} names a file outside the folder the examples run in")
        j += 1
        text = []
        while j < len(body) and not body[j].startswith("$ "):
            text.append(body[j])
            j += 1
        while text and not text[-1].strip():
            text.pop()
        files.append(("file", at, name, "\n".join(text) + "\n"))
    return files


def seed(items, root):
    """Copies into `root` each file under tests/documented that a quoted
    string of the examples names, by its path under that folder or by its
    file name, or that lies under a folder one names."""
    words = set()
    for item in items:
        if item[0] == "example":
            for quoted in re.findall(r"""["']([^"'\n]+)["']""", item[2]):
                words.add(quoted.removeprefix("./").rstrip("/"))
    for source in sorted(p for p in SEEDS.rglob("*") if p.is_file()):
        named = source.relative_to(SEEDS).as_posix()
        if named in words or source.name in words or any(named.startswith(w + "/") for w in words):
            target = root / named
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)


@pytest.fixture
def system_clock():
    """The clock a new process reads: the wall clock, timestamps with no
    zone read in UTC."""
    trex.set_now(None)
    trex.set_tz_offset(0)
    yield
    trex.set_now(None)
    trex.set_tz_offset(0)


@pytest.mark.parametrize("page", documented_pages(), ids=lambda p: p.relative_to(REPO).as_posix())
def test_shows_what_the_page_runs(page, tmp_path, monkeypatch, system_clock):
    items = page_items(page)
    seed(items, tmp_path)
    monkeypatch.chdir(tmp_path)
    parser = doctest.DocTestParser()
    runner = doctest.DocTestRunner(optionflags=0)
    namespace = {}
    report = []
    for item in items:
        if item[0] == "file":
            _, _, name, text = item
            target = tmp_path / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text, encoding="utf-8", newline="\n")
            continue
        _, line, source = item
        name = f"{page.relative_to(REPO).as_posix()}:{line}"
        test = parser.get_doctest(source, namespace, name, str(page), line - 1)
        runner.run(test, out=report.append, clear_globs=False)
        namespace = test.globs
    if report:
        pytest.fail("".join(report), pytrace=False)


def test_finds_a_tabbed_page_among_the_pages_it_runs():
    tabbed = REPO / "wiki" / "content" / "docs" / "reference" / "matching.md"
    assert tabbed in documented_pages()
    assert any(item[0] == "example" for item in page_items(tabbed))
