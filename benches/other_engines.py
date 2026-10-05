"""Python's `re` over the corpus and the patterns `vs_regex_full` uses.

The Rust comparison times trex against the regex crate, which are the two
fastest things in the room. A reader wants to know what the same work costs in
the tools they already reach for, so this runs the same patterns over the same
bytes and prints rows in the same shape.

The corpus is built by the formula in `vs_regex_full.rs`, statement for
statement, so the two harnesses read identical bytes. The patterns are that
file's regex spellings; where Python spells one differently the difference is
noted beside it.

Run: `python benches/other_engines.py [statements]`, default 200000, which is
the 7.34 MB the comparison uses. Output is one row per operation per pattern:

    engine  pattern  operation  ms  answer

`ms` is the median of the timed rounds, per call. A row whose answer differs
from the Rust harness's is a row the two are not asking the same question of,
and is a defect in this file rather than a finding.
"""

import re
import sys
import time

# Statements of four shapes, the corpus `benches/vs_regex_full.rs` builds. The
# formula is that file's, and the two must stay identical: a comparison over
# different bytes is not one.
def corpus(statements):
    out = []
    for i in range(statements):
        k = i % 4
        if k == 0:
            out.append(f"let value_{i} = {i * 37} ;\n")
        elif k == 1:
            out.append(f"call_{i}(alpha, beta, {i}) ;\n")
        elif k == 2:
            out.append(f"key_{i}: item_{i}, item_{i + 1}, item_{i + 2} ;\n")
        else:
            out.append(f"if (cond_{i}) {{ do_{i}(x) ; }}\n")
    return "".join(out)


# The regex spellings `vs_regex_full.rs` pairs with each trex pattern, named as
# that file names them. Python's `re` has no possessive or atomic forms here and
# needs none: every one of these is a plain regular expression in both.
PAIRS = [
    ("a literal word", r"\balpha\b"),
    ("a literal word that is absent", r"\bzzzqqq\b"),
    ("any word token", r"\b[A-Za-z_][A-Za-z_0-9]*\b"),
    ("any number token", r"\b[0-9]+\b"),
    ("either of two literals", r"\b(?:alpha|beta)\b"),
    ("a word then punctuation", r"\b[A-Za-z_][A-Za-z_0-9]*\b ="),
    ("a byte-pattern inside a token", r"\bcond_[0-9]+\b"),
    ("a literal, a word and punctuation", r"\blet\b [A-Za-z_][A-Za-z_0-9]* ="),
    ("the same, with the word bound", r"\blet\b (?P<v>[A-Za-z_][A-Za-z_0-9]*) ="),
    ("a bounded repeat of a token", r"\b[A-Za-z_][A-Za-z_0-9]*\b [A-Za-z_][A-Za-z_0-9]*\b"),
    ("a line-anchored literal", r"(?m)^let\b"),
    ("a named capture then punctuation", r"(?P<name>[A-Za-z_][A-Za-z_0-9]*) ="),
]

ROUNDS = 5
BUDGET_MS = 25.0


def time_budget(f):
    """Call `f` until BUDGET_MS has passed and at least five calls have run.

    The Rust harness's own shape, so the milliseconds are comparable: a mean
    call over a budget rather than a single reading.
    """
    answer = f()
    t0 = time.perf_counter()
    iters = 0
    while True:
        f()
        iters += 1
        if iters >= 5 and (time.perf_counter() - t0) * 1e3 >= BUDGET_MS:
            break
    return ((time.perf_counter() - t0) * 1e3 / iters, answer)


def median(v):
    return sorted(v)[len(v) // 2]


def sweep(name, pattern, ops):
    """Every operation timed once a round, in an order that rotates.

    Rotating matters for the same reason it does in the Rust harness: position
    inside a run is worth a few percent, and an operation always timed first
    would read that as its own.
    """
    times = {k: [] for k in ops}
    answers = {}
    keys = list(ops)
    for r in range(ROUNDS):
        for step in range(len(keys)):
            k = keys[(r + step) % len(keys)]
            ms, answer = time_budget(ops[k])
            times[k].append(ms)
            answers[k] = answer
    for k in keys:
        print(f"python\t{name}\t{k}\t{median(times[k]):.5f}\t{answers[k]}")


def main():
    statements = int(sys.argv[1]) if len(sys.argv) > 1 else 200_000
    text = corpus(statements)
    print(f"# python {sys.version.split()[0]}, corpus {len(text)} bytes, {statements} statements")
    print("engine\tpattern\toperation\tms\tanswer")
    for name, src in PAIRS:
        rx = re.compile(src)
        has_groups = rx.groups > 0
        ops = {
            "compile the pattern": lambda s=src: 1 if re.compile(s) else 0,
            "does it match at all": lambda r=rx: 1 if r.search(text) else 0,
            "where it first matches": lambda r=rx: (m.start() + 1 if (m := r.search(text)) else 0),
            "every match": lambda r=rx: sum(1 for _ in r.finditer(text)),
            "replace every match": lambda r=rx: len(r.sub("X", text)),
        }
        if not has_groups:
            # `re.split` yields the captured groups between the pieces, so a
            # pattern with one answers a different question from the Rust
            # harness's split and the row would not be comparable. Those
            # patterns are left without a split row rather than given a wrong
            # one.
            ops["split on every match"] = lambda r=rx: len(r.split(text))
            ops["split into at most four pieces"] = lambda r=rx: len(r.split(text, maxsplit=3))
        if has_groups:
            # The captures of every match, which only a pattern with a group
            # has. `finditer` already carries them, so reading one is what
            # separates this from counting.
            ops["captures at every match"] = lambda r=rx: sum(
                1 for m in r.finditer(text) if m.group(1) is not None
            )
        sweep(name, src, ops)


if __name__ == "__main__":
    main()
