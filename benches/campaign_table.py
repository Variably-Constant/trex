"""Turn a campaign log into one table a size, joining the four engines.

    python campaign_table.py <campaign.log>

The log holds one block a corpus size. Inside a block the Rust comparison prints
a section a pattern, each with rows carrying trex's and regex's milliseconds;
python and perl print tab-separated rows of engine, pattern, operation,
milliseconds and answer. The pattern and operation names are the same strings in
all four, which is what lets them be joined.

What it will not do is fill a gap. An engine that did not report a row is
printed as absent, because a missing row and a zero row read alike in a table
and only one of them is honest, and an engine that refused is carried through as
its refusal so the reason survives into the artifact.
"""

import re
import sys
from collections import OrderedDict

SIZE = re.compile(r"^#+ STATEMENTS (\d+) #+$")
SECTION = re.compile(r"^=== (.+?)\s+trex `(.+?)`\s+regex `(.+?)` ===$")
BUILD = re.compile(r"^build: (.+)$")
CORPUS = re.compile(r"^corpus: (\d+) bytes")

# A section that names no pattern pair. Three of the bench's blocks are shaped
# this way: the long-input block carries a byte count instead, and the
# many-patterns and no-counterpart blocks carry only a title. Without these the
# rows under them are attributed to whichever pattern was named last, which
# misfiles seventeen rows a size under the pattern above them.
SECTION_ANY = re.compile(r"^=== (.+?) ===$")
SECTION_BYTES = re.compile(r"^(.+?)\s+(\d+) bytes$")
NOT_A_SECTION = ("TREE", "INTERPRETERS", "PASS", "BUILD", "HOST")

# A run that times the same size several ways names each one. Without this the
# four bench blocks a size would share every key and the last would stand for
# all of them, reporting one arm as though it were the measurement.
ARM = re.compile(r"^-+ ARM (.+?) at (\d+), [\d:]+ -+$")


def encoding_of(path):
    """The text encoding of a log, from its byte-order mark.

    PowerShell's Tee-Object writes UTF-16LE with a mark, while a log captured
    another way is UTF-8. Reading a UTF-16 file as UTF-8 does not fail: every
    other byte is a NUL that error handling replaces, so the parse returns no
    rows and reports an empty campaign rather than an unreadable one.
    """
    with open(path, "rb") as f:
        head = f.read(4)
    if head[:2] == b"\xff\xfe":
        return "utf-16"
    if head[:2] == b"\xfe\xff":
        return "utf-16"
    if head[:3] == b"\xef\xbb\xbf":
        return "utf-8-sig"
    return "utf-8"


def number(tok):
    """A millisecond column, or None where the engine reported none."""
    if tok in ("none", "-", ""):
        return None
    try:
        return float(tok)
    except ValueError:
        return None


def spread_of(tok):
    """A spread column, printed as a multiple: `1.04x` is the slowest repeat
    of that cell against its fastest. None where the engine reported none."""
    if not tok or not tok.endswith("x"):
        return None
    return number(tok[:-1])


def parse(path):
    """Every row of the log as (size, pattern, operation, engine) -> ms.

    The comparison indents a row by two spaces for an operation and by four
    for a further arm of the operation above it. A four-space row is keyed
    under `parent -> arm`, which both records that it is a second way of
    asking the same question and keeps two arms that share a name apart: the
    log carries "the same, from an offset" under two different parents, and a
    bare name would file the second over the first.
    """
    rows = OrderedDict()
    meta = OrderedDict()
    subarms = set()
    spreads = OrderedDict()
    size = None
    pattern = None
    parent_op = None
    with open(path, "r", encoding=encoding_of(path), errors="replace") as f:
        for line in f:
            line = line.rstrip("\n")
            m = SIZE.match(line.strip())
            if m:
                size, pattern, parent_op = int(m.group(1)), None, None
                meta.setdefault(size, {})
                continue
            if size is None:
                # A log from a single run carries no size block. It is still a
                # reading and is parsed as one unlabeled size, so the same
                # table can be built over the runs taken before the ladder.
                size = 0
                meta.setdefault(size, {})
            if ARM.match(line.strip()):
                # A run that times one size several ways starts a fresh block
                # here. Splitting such a log into a file an arm is what keeps
                # the arms apart; within one file this only resets the section
                # so rows cannot attach to the previous block's last pattern.
                pattern, parent_op = None, None
                continue
            m = CORPUS.match(line.strip())
            if m:
                meta[size]["bytes"] = int(m.group(1))
                continue
            m = BUILD.match(line.strip())
            if m:
                meta[size]["build"] = m.group(1)
                continue
            m = SECTION.match(line.strip())
            if m:
                pattern, parent_op = m.group(1), None
                meta[size].setdefault("patterns", OrderedDict())[pattern] = (
                    m.group(2),
                    m.group(3),
                )
                continue
            m = SECTION_ANY.match(line.strip())
            if m and not m.group(1).startswith(NOT_A_SECTION):
                title = m.group(1)
                byte_count = SECTION_BYTES.match(title)
                if byte_count:
                    title = byte_count.group(1)
                pattern, parent_op = title, None
                meta[size].setdefault("patterns", OrderedDict()).setdefault(
                    pattern, (None, None)
                )
                continue
            # A tab-separated row from python or perl.
            if "\t" in line:
                parts = line.split("\t")
                if len(parts) >= 4 and parts[0] in ("python", "perl"):
                    ms = number(parts[3])
                    if ms is not None:
                        rows[(size, parts[1], parts[2], parts[0])] = ms
                continue
            # A row of the Rust comparison: the operation, then seven columns.
            if pattern and line.startswith("  ") and not line.strip().startswith("operation"):
                cols = line.split()
                if len(cols) < 8:
                    continue
                trex, regex = number(cols[-7]), number(cols[-6])
                if trex is None and regex is None:
                    continue
                op = " ".join(cols[: len(cols) - 7]).strip()
                indent = len(line) - len(line.lstrip(" "))
                if indent >= 4 and parent_op:
                    op = f"{parent_op} -> {op}"
                    subarms.add((size, pattern, op))
                else:
                    parent_op = op
                # The comparison prints seven columns after the operation:
                # trex ms, regex ms, their ratio, each engine's spread, and
                # each engine's answer count. The spread is the slowest repeat
                # of that cell against its fastest, so it is the cell's own
                # account of how steady it was.
                if trex is not None:
                    rows[(size, pattern, op, "trex")] = trex
                    spreads[(size, pattern, op, "trex")] = spread_of(cols[-4])
                if regex is not None:
                    rows[(size, pattern, op, "regex")] = regex
                    spreads[(size, pattern, op, "regex")] = spread_of(cols[-3])
    return rows, meta, subarms, spreads


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    rows, meta, subarms, cellspread = parse(argv[1])
    if not rows:
        print(f"no rows parsed from {argv[1]}")
        return 1
    sizes = sorted({k[0] for k in rows})
    engines = ("trex", "regex", "python", "perl")
    for size in sizes:
        info = meta.get(size, {})
        named = f"{size} statements" if size else "one run, size not named in the log"
        print(f"\n################ {named}, {info.get('bytes', '?')} bytes")
        print(f"build: {info.get('build', 'NOT REPORTED - the log does not say')}")
        keys = [k for k in rows if k[0] == size]
        pairs = list(OrderedDict.fromkeys((k[1], k[2]) for k in keys))
        wide_pat = max([len(p) for p, _ in pairs] + [len("pattern")])
        wide_op = max([len(o) for _, o in pairs] + [len("operation")])
        print(
            f"{'pattern':<{wide_pat}} {'operation':<{wide_op}} "
            + " ".join(f"{e:>11}" for e in engines)
        )
        for pat, op in pairs:
            cells = []
            for e in engines:
                v = rows.get((size, pat, op, e))
                cells.append(f"{v:>11.5f}" if v is not None else f"{'absent':>11}")
            print(f"{pat:<{wide_pat}} {op:<{wide_op}} " + " ".join(cells))
    summary(rows, meta, sizes, engines)
    print(f"\nsizes: {sizes}")
    missing = [e for e in engines if not any(k[3] == e for k in rows)]
    if missing:
        print(f"ENGINES ABSENT FROM THE WHOLE LOG: {', '.join(missing)}")
    return 0


def summary(rows, meta, sizes, engines):
    """Totals over the rows every engine answered, and trex against each.

    The comparison set is the rows ALL FOUR reported. python and perl cannot
    express a token pattern, so most of trex's surface has no counterpart in
    them, and a total over everything each engine happened to answer would
    compare four different workloads. Restricting to the common rows compares
    one workload, and the count of them is printed so a reader can see how much
    of the surface that is.
    """
    print("\n\n################ totals over the rows every engine answered")
    print(
        f"{'statements':>11} {'bytes':>10} {'rows':>6} "
        + " ".join(f"{e:>11}" for e in engines)
        + "   trex vs each"
    )
    for size in sizes:
        present = {}
        for e in engines:
            present[e] = {(k[1], k[2]) for k in rows if k[0] == size and k[3] == e}
        common = set.intersection(*present.values()) if all(present.values()) else set()
        if not common:
            named = size if size else "unnamed"
            have = ", ".join(e for e in engines if present[e]) or "none"
            print(f"{named:>11} {'':>10} {0:>6}  no row is answered by all four; present: {have}")
            continue
        totals = {
            e: sum(rows[(size, p, o, e)] for (p, o) in common) for e in engines
        }
        ratios = " ".join(
            f"{e} {totals[e] / totals['trex']:.2f}x" for e in engines if e != "trex"
        )
        print(
            f"{size:>11} {meta.get(size, {}).get('bytes', 0):>10} {len(common):>6} "
            + " ".join(f"{totals[e]:>11.3f}" for e in engines)
            + f"   {ratios}"
        )


if __name__ == "__main__":
    sys.exit(main(sys.argv))
