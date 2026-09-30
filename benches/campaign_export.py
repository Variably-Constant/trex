"""Turn one or more campaign logs into the JSON an artifact reads.

    python campaign_export.py <out.json> <label>=<log> [<label>=<log> ...]

Parsing is campaign_table's, imported rather than copied, so the table a reader
can rebuild by hand and the page they are shown come from one parser and cannot
drift apart.

Each log is a pass. A pass is named so a replicate sits beside its first
reading rather than over it, and the spread between passes is reported as its
own figure: this bench has read one commit twice and differed by fifteen
percent, so a single reading is a number to check rather than a number to
publish.

Three comparisons come out, and they are not equally strong:

  * trex against regex is timed in ONE process over the same bytes with the
    arms interleaved, so the ratio holds whatever else the box is doing.
  * trex against python and against perl crosses process boundaries, so those
    ratios carry the box's own variation and are reported only over rows all
    four engines answered.
  * the surface regex cannot express carries no ratio at all, because there is
    nothing to divide by. It is listed, with trex's time, and left at that.
"""

import json
import statistics
import sys
from collections import OrderedDict

from campaign_table import parse

ENGINES = ("trex", "regex", "python", "perl")


def pass_data(path):
    """One pass: per-size rows, totals over the common set, and the build."""
    rows, meta, subarms, cellspread = parse(path)
    if not rows:
        raise SystemExit(
            f"REFUSED: {path} parsed to no rows. A pass that reads as empty would "
            f"enter the artifact as a pass with nothing in it and move every "
            f"spread toward zero, which reads as agreement rather than as a file "
            f"that could not be read."
        )
    sizes = sorted({k[0] for k in rows})
    out = OrderedDict()
    for size in sizes:
        # Totals are over competing answers only. A sub-arm is the same
        # question asked a second way to show where a cost sits, so counting
        # it alongside its parent totals one question more than once.
        present = {
            e: {
                (k[1], k[2])
                for k in rows
                if k[0] == size and k[3] == e and (k[0], k[1], k[2]) not in subarms
            }
            for e in ENGINES
        }
        common = (
            set.intersection(*present.values()) if all(present.values()) else set()
        )
        pairs = list(OrderedDict.fromkeys((k[1], k[2]) for k in rows if k[0] == size))
        table = []
        for pat, op in pairs:
            cells = {e: rows.get((size, pat, op, e)) for e in ENGINES}
            table.append(
                {
                    "pattern": pat,
                    "operation": op,
                    "common": (pat, op) in common,
                    # A further arm of the operation above it rather than a
                    # competing answer: the comparison asks the same question
                    # a second way to decompose a cost, and scoring those arms
                    # as wins or losses counts one question several times.
                    "subarm": (size, pat, op) in subarms,
                    **{e: cells[e] for e in ENGINES},
                    # Each Rust cell's own spread: its slowest repeat over its
                    # fastest, within the one process that timed it.
                    "spread_trex": cellspread.get((size, pat, op, "trex")),
                    "spread_regex": cellspread.get((size, pat, op, "regex")),
                }
            )
        totals = {
            e: sum(rows[(size, p, o, e)] for (p, o) in common) for e in ENGINES
        } if common else {}
        # Both Rust engines answer far more of the surface than the other two,
        # and that wider set is the one whose ratio is load-independent, so it
        # is totalled separately rather than folded into the common four.
        rust_common = present["trex"] & present["regex"]
        rust_totals = {
            e: sum(rows[(size, p, o, e)] for (p, o) in rust_common)
            for e in ("trex", "regex")
        }
        out[size] = {
            "bytes": meta.get(size, {}).get("bytes"),
            "build": meta.get(size, {}).get("build"),
            "rows": table,
            "common_rows": len(common),
            "totals": totals,
            "rust_rows": len(rust_common),
            "rust_totals": rust_totals,
        }
    return out


def spread(values):
    """How far apart repeated readings of one thing are, as a fraction."""
    values = [v for v in values if v is not None]
    if len(values) < 2:
        return None
    return (max(values) - min(values)) / statistics.mean(values)


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    out_path = argv[1]
    passes = OrderedDict()
    for arg in argv[2:]:
        if "=" not in arg:
            raise SystemExit(f"expected <label>=<log>, got {arg!r}")
        label, path = arg.split("=", 1)
        passes[label] = pass_data(path)
        print(f"pass {label}: {len(passes[label])} sizes from {path}")

    # The spread between passes, per size and per engine. A figure with one
    # pass behind it is reported as absent rather than as zero spread: one
    # reading agrees with itself perfectly and that is not a measurement.
    sizes = sorted({s for p in passes.values() for s in p})
    spreads = {}
    for size in sizes:
        spreads[size] = {
            e: spread([p.get(size, {}).get("totals", {}).get(e) for p in passes.values()])
            for e in ENGINES
        }
        spreads[size]["ratio_vs_regex"] = spread(
            [
                (
                    p[size]["rust_totals"]["regex"] / p[size]["rust_totals"]["trex"]
                    if size in p and p[size]["rust_totals"].get("trex")
                    else None
                )
                for p in passes.values()
            ]
        )

    doc = {"passes": passes, "spreads": spreads, "sizes": sizes}
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(doc, f, indent=1)
    print(f"wrote {out_path}")

    for size in sizes:
        for label, p in passes.items():
            if size not in p:
                continue
            t = p[size]["totals"]
            r = p[size]["rust_totals"]
            if not t:
                continue
            print(
                f"  {size:>7} pass {label}: "
                f"common {p[size]['common_rows']:>3} rows, trex {t['trex']:8.3f} "
                f"regex {t['regex']:8.3f} python {t['python']:9.3f} perl {t['perl']:9.3f}"
                f"   |  all {p[size]['rust_rows']:>3} rust rows: "
                f"trex {r['trex']:8.3f} regex {r['regex']:8.3f} "
                f"= {r['regex'] / r['trex']:.2f}x"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
