"""Split a campaign log that times each size several ways into a log an arm.

    python split_arms.py <host2.log> <out-prefix>

A run with arms interleaves them inside each size block: device absent, device
present, absent again, present again. Every arm is a complete reading of the
whole pattern set, so each becomes its own file and is then a pass like any
other, parsed by the verified parser rather than by a second one written for
this shape.

An arm repeated at one size is a replicate of that arm and keeps its own file,
because two readings that agree are the evidence the arm is steady and two
averaged into one are not.

The size headers and the interpreter rows are copied into every arm's file, so
each is a standalone log rather than a fragment that only parses beside its
siblings.
"""

import re
import sys
from collections import OrderedDict

from campaign_table import encoding_of

ARM = re.compile(r"^-+ ARM (.+?) at (\d+), [\d:]+ -+$")
SIZE = re.compile(r"^#+ STATEMENTS (\d+) #+$")
TSV = re.compile(r"^(python|perl)\t")


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    src, prefix = argv[1], argv[2]

    with open(src, "r", encoding=encoding_of(src), errors="replace") as f:
        lines = f.read().splitlines()

    arms = OrderedDict()
    seen = {}
    size_line = None
    current = None
    # The interpreter rows are printed once a size, after every arm has run,
    # so they are collected and appended to each arm's file rather than landing
    # only in whichever arm happened to be current when they appeared.
    tsv_by_size = {}
    size = None

    for line in lines:
        m = SIZE.match(line.strip())
        if m:
            size, size_line, current = int(m.group(1)), line, None
            continue
        m = ARM.match(line.strip())
        if m:
            base = m.group(1).strip()
            seen[(size, base)] = seen.get((size, base), 0) + 1
            n = seen[(size, base)]
            current = base if n == 1 else f"{base} #{n}"
            arms.setdefault(current, [])
            if size_line:
                arms[current].append(size_line)
            continue
        if TSV.match(line):
            tsv_by_size.setdefault(size, []).append(line)
            continue
        if current:
            arms[current].append(line)

    if not arms:
        raise SystemExit(f"REFUSED: {src} names no arms, so there is nothing to split")

    written = []
    for arm, body in arms.items():
        slug = re.sub(r"[^a-z0-9]+", "_", arm.lower()).strip("_")
        path = f"{prefix}_{slug}.log"
        out = []
        for line in body:
            out.append(line)
            m = SIZE.match(line.strip())
            if m:
                continue
        # Append every engine row under the size block it belongs to.
        merged = []
        size = None
        for line in out:
            m = SIZE.match(line.strip())
            if m:
                if size is not None:
                    merged.extend(tsv_by_size.get(size, []))
                size = int(m.group(1))
            merged.append(line)
        if size is not None:
            merged.extend(tsv_by_size.get(size, []))
        with open(path, "w", encoding="utf-8") as f:
            f.write("\n".join(merged) + "\n")
        written.append((arm, path, len(merged)))

    for arm, path, n in written:
        print(f"{arm:<24} -> {path}  ({n} lines)")
    print(f"\n{len(written)} arms, sizes seen: {sorted(tsv_by_size)}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
