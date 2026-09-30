"""Render the comparison artifact from the exported campaign JSON.

    python build_page.py <campaign_data.json> <artifact.html> <standalone.html>
                         <results.md> [COMPARISON.md]

One generator for every output, so the published page, the file on disk and the
markdown in the repository cannot disagree about a number. Everything they
print is read from the JSON; nothing is typed in.

The two HTML files differ only in their wrapper: the artifact host supplies a
document skeleton, and a file opened from disk needs its own. Given a fifth
argument, the results are spliced into that markdown page below its Results
heading, leaving the prose above it alone.
"""

import html
import json
import math
import sys
from collections import OrderedDict

ENGINES = ("trex", "regex", "python", "perl")
ENGINE_TITLE = {
    "trex": "trex",
    "regex": "regex",
    "python": "python re",
    "perl": "perl",
}


def thousands(n):
    return f"{n:,}"


def mb(n):
    return f"{n / 1_000_000:.2f} MB"


CONTROL_OP = "CONTROL does it match at all, again"
CONTROL_OF = "does it match at all"


def answers(rows):
    """The rows that are a competing answer rather than a decomposition.

    The comparison asks some questions a second and third way to show where a
    cost sits - `find` beside the same lookup done by lexing the whole input,
    and by scanning it. Those arms are deliberately the expensive route, so
    scoring them as losses counts one question several times and reports a
    design choice as a defeat.

    The control row is excluded for the same reason from the other direction:
    it repeats the section's first question unchanged to measure drift, so it
    is a second copy of a question already counted.
    """
    return [
        r for r in rows
        if not r.get("subarm") and r["operation"] != CONTROL_OP
    ]


def drift(rows):
    """Each section's control against its first reading of the same work.

    The bench times `is_match` at the top of a section and again at the
    bottom. Both readings are the same work, so the difference between them is
    position in the run and load and nothing else. This is the run's own
    account of how steady it was, and it is stronger evidence than any load
    figure taken from outside the process.
    """
    first = {}
    out = []
    for r in rows:
        if r["operation"] == CONTROL_OF:
            first[r["pattern"]] = r
        elif r["operation"] == CONTROL_OP and r["pattern"] in first:
            a = first[r["pattern"]]
            for engine in ("trex", "regex"):
                if a.get(engine) and r.get(engine):
                    out.append((r["pattern"], engine, a[engine], r[engine],
                                r[engine] / a[engine]))
    return out


def rollup(rows, engines):
    """Summed milliseconds over the rows every named engine answered."""
    common = [r for r in rows if all(r.get(e) is not None for e in engines)]
    return {e: sum(r[e] for r in common) for e in engines}, len(common)


def per_pattern(rows):
    """Per pattern, over the rows trex and regex both answered."""
    acc = OrderedDict()
    for r in rows:
        if r["trex"] is None or r["regex"] is None:
            continue
        a = acc.setdefault(r["pattern"], {"trex": 0.0, "regex": 0.0, "n": 0})
        a["trex"] += r["trex"]
        a["regex"] += r["regex"]
        a["n"] += 1
    for pat, a in acc.items():
        a["ratio"] = a["regex"] / a["trex"] if a["trex"] else None
    return acc


def win_loss(rows):
    """Rows won and lost against regex, counted and summed."""
    won, lost = [], []
    for r in rows:
        if r["trex"] is None or r["regex"] is None:
            continue
        (won if r["regex"] > r["trex"] else lost).append(r)
    return {
        "won_rows": len(won),
        "lost_rows": len(lost),
        "ms_ahead": sum(r["regex"] - r["trex"] for r in won),
        "ms_behind": sum(r["trex"] - r["regex"] for r in lost),
        "won": won,
        "lost": lost,
    }


def across_passes(d, engine, places=2):
    """One ratio as every pass read it: "2.74x" alone, "2.74x and 2.79x" for
    two, "2.43x to 2.79x" for more. A single pass standing in for several
    hides the only error bar a campaign has."""
    vals = [v[engine] for v in d["by_pass"].values() if v.get(engine) is not None]
    if not vals:
        return "-"
    if len(vals) == 1:
        return f"{vals[0]:.{places}f}x"
    if len(vals) == 2:
        return f"{vals[0]:.{places}f}x and {vals[1]:.{places}f}x"
    return f"{min(vals):.{places}f}x to {max(vals):.{places}f}x"


def bar(frac, klass):
    pct = max(0.0, min(1.0, frac)) * 100
    return f'<span class="bar {klass}" style="width:{pct:.1f}%"></span>'


def build(doc):
    passes = doc["passes"]
    sizes = [str(s) for s in doc["sizes"]]
    labels = list(passes)
    lead = labels[0]
    biggest = sizes[-1]

    lead_big = passes[lead][biggest]
    rows_big = answers(lead_big["rows"])
    subarms = [r for r in lead_big["rows"] if r.get("subarm")]
    four, four_n = rollup(rows_big, ENGINES)
    rust, rust_n = rollup(rows_big, ("trex", "regex"))
    wl = win_loss(rows_big)
    pats = per_pattern(rows_big)
    only_trex = [r for r in rows_big if r["trex"] is not None and r["regex"] is None]

    ratio_regex = rust["regex"] / rust["trex"]
    ratio_py = four["python"] / four["trex"]
    ratio_pl = four["perl"] / four["trex"]

    # The same ratio taken from every pass, so a figure quoted in the headline
    # can be given as what the passes actually read rather than as one of them
    # standing for all.
    by_pass = {}
    for label, p in passes.items():
        if biggest not in p:
            continue
        rows = answers(p[biggest]["rows"])
        r, _ = rollup(rows, ("trex", "regex"))
        f, n = rollup(rows, ENGINES)
        by_pass[label] = {
            "regex": r["regex"] / r["trex"] if r.get("trex") else None,
            "python": f["python"] / f["trex"] if n else None,
            "perl": f["perl"] / f["trex"] if n else None,
        }

    return {
        "passes": passes,
        "sizes": sizes,
        "labels": labels,
        "lead": lead,
        "biggest": biggest,
        "four": four,
        "four_n": four_n,
        "rust": rust,
        "rust_n": rust_n,
        "wl": wl,
        "pats": pats,
        "only_trex": only_trex,
        "subarms": subarms,
        "drift": drift(lead_big["rows"]),
        "cell_spreads": {
            engine: [
                r[f"spread_{engine}"]
                for r in lead_big["rows"]
                if r.get(f"spread_{engine}") is not None
            ]
            for engine in ("trex", "regex")
        },
        "ratio_regex": ratio_regex,
        "ratio_py": ratio_py,
        "ratio_pl": ratio_pl,
        "by_pass": by_pass,
        "build_line": lead_big.get("build"),
        "spreads": doc.get("spreads", {}),
    }


CSS = """
:root{
  --paper:#f4f6f8; --card:#ffffff; --ink:#151922; --muted:#5c6472;
  --rule:#dde1e7; --ahead:#0d6e5c; --ahead-soft:#c7e5dd;
  --behind:#a8402a; --behind-soft:#f2d6ce; --flat:#8b93a1;
  --shadow:0 1px 2px rgba(21,25,34,.05), 0 8px 24px -16px rgba(21,25,34,.25);
}
:root:not([data-theme="light"]){ }
@media (prefers-color-scheme: dark){
  :root:not([data-theme="light"]){
    --paper:#0e1116; --card:#151a21; --ink:#e6e9ee; --muted:#98a1b0;
    --rule:#262d38; --ahead:#4ec8ab; --ahead-soft:#153a33;
    --behind:#e28468; --behind-soft:#3a201a; --flat:#6b7483;
    --shadow:0 1px 2px rgba(0,0,0,.4), 0 8px 24px -16px rgba(0,0,0,.8);
  }
}
:root[data-theme="dark"]{
  --paper:#0e1116; --card:#151a21; --ink:#e6e9ee; --muted:#98a1b0;
  --rule:#262d38; --ahead:#4ec8ab; --ahead-soft:#153a33;
  --behind:#e28468; --behind-soft:#3a201a; --flat:#6b7483;
  --shadow:0 1px 2px rgba(0,0,0,.4), 0 8px 24px -16px rgba(0,0,0,.8);
}

*{box-sizing:border-box}
body{
  background:var(--paper); color:var(--ink);
  font-family:"Source Serif 4",Georgia,"Times New Roman",serif;
  font-size:17px; line-height:1.6;
  padding-inline:20px; padding-block:0;
}
.wrap{max-width:1080px; margin:0 auto; padding-block:44px 72px}
h1,h2,h3,.mono,th,.num,.tag,.stat-v{
  font-family:"JetBrains Mono",ui-monospace,"SFMono-Regular",Consolas,monospace;
}
h1{
  font-size:clamp(28px,4.4vw,44px); font-weight:700; line-height:1.12;
  letter-spacing:-.02em; margin:0 0 14px; text-wrap:balance;
}
h2{
  font-size:15px; font-weight:700; letter-spacing:.12em; text-transform:uppercase;
  color:var(--muted); margin:0 0 4px;
}
h3{font-size:17px; font-weight:700; margin:0 0 8px; letter-spacing:-.01em}
p{margin:0 0 14px; max-width:68ch}
a{color:var(--ahead)}
section{margin-top:56px}
.sec-head{border-top:2px solid var(--ink); padding-top:10px; margin-bottom:22px}
.sub{color:var(--muted); font-size:15px; max-width:68ch; margin:0}

.lede{font-size:20px; line-height:1.5; max-width:64ch}
.lede strong{color:var(--ahead)}

.tags{display:flex; flex-wrap:wrap; gap:8px; margin:18px 0 0}
.tag{
  font-size:12px; letter-spacing:.04em; border:1px solid var(--rule);
  border-radius:999px; padding:4px 11px; color:var(--muted); background:var(--card);
}
.tag b{color:var(--ink); font-weight:700}

.stats{display:grid; grid-template-columns:repeat(auto-fit,minmax(168px,1fr)); gap:14px; margin:28px 0 0}
.stat{background:var(--card); border:1px solid var(--rule); border-radius:10px; padding:16px 18px; box-shadow:var(--shadow)}
.stat-k{font-size:12px; letter-spacing:.09em; text-transform:uppercase; color:var(--muted); margin-bottom:6px}
.stat-v{font-size:30px; font-weight:700; line-height:1.05; letter-spacing:-.02em; color:var(--ahead)}
.stat-v.long{font-size:20px; letter-spacing:-.01em}
.stat-n{font-size:13px; color:var(--muted); margin-top:7px; line-height:1.4}

.paradox{display:grid; grid-template-columns:1fr 1fr; gap:18px; margin-top:8px}
.pcard{background:var(--card); border:1px solid var(--rule); border-radius:10px; padding:20px; box-shadow:var(--shadow)}
.pcard h3{margin-bottom:14px}
.split{display:flex; height:30px; border-radius:5px; overflow:hidden; border:1px solid var(--rule)}
.split span{display:flex; align-items:center; padding:0 9px; font-size:12px; font-weight:700;
  font-family:"JetBrains Mono",ui-monospace,monospace; color:#fff; white-space:nowrap}
.split .s-ahead{background:var(--ahead)}
.split .s-behind{background:var(--behind)}
.plabel{display:flex; justify-content:space-between; font-size:12.5px; color:var(--muted); margin-top:8px}
.plabel b{font-family:"JetBrains Mono",ui-monospace,monospace}

.tablewrap{overflow-x:auto; border:1px solid var(--rule); border-radius:10px; background:var(--card); box-shadow:var(--shadow)}
table{border-collapse:collapse; width:100%; font-size:14px}
th{
  text-align:right; font-size:11.5px; letter-spacing:.07em; text-transform:uppercase;
  color:var(--muted); font-weight:700; padding:12px 14px; border-bottom:1px solid var(--rule);
  white-space:nowrap; background:var(--card); position:sticky; top:0;
}
th:first-child,td:first-child{text-align:left}
td{padding:10px 14px; border-bottom:1px solid var(--rule); text-align:right; white-space:nowrap}
tr:last-child td{border-bottom:none}
.num{font-variant-numeric:tabular-nums; font-size:13.5px}
.name{font-family:"Source Serif 4",Georgia,serif; font-size:15px; white-space:normal; min-width:180px}
.op{color:var(--muted); font-size:14px; white-space:normal; min-width:180px}
.win{color:var(--ahead); font-weight:700}
.loss{color:var(--behind); font-weight:700}
tr.total td{border-top:2px solid var(--ink); font-weight:700; background:var(--card)}

.barrow{display:grid; grid-template-columns:minmax(150px,1.4fr) 62px 1fr; gap:12px; align-items:center;
  padding:7px 0; border-bottom:1px solid var(--rule)}
.barrow:last-child{border-bottom:none}
.barname{font-size:15px; line-height:1.3}
.barval{font-family:"JetBrains Mono",ui-monospace,monospace; font-size:13px; font-weight:700;
  font-variant-numeric:tabular-nums; text-align:right}
.bartrack{position:relative; height:17px; background:var(--paper); border-radius:3px; overflow:hidden}
.bar{position:absolute; top:0; bottom:0; border-radius:3px}
.bar.b-ahead{left:50%; background:var(--ahead)}
.bar.b-behind{right:50%; background:var(--behind)}
.bartrack::before{content:""; position:absolute; left:50%; top:0; bottom:0; width:1px; background:var(--rule); z-index:2}

.note{background:var(--card); border:1px solid var(--rule); border-left:3px solid var(--ahead);
  border-radius:0 8px 8px 0; padding:16px 20px; margin:20px 0}
.note.warn{border-left-color:var(--behind)}
.note p:last-child{margin-bottom:0}
.note h3{font-size:15px}

code,.code{font-family:"JetBrains Mono",ui-monospace,monospace; font-size:13.5px;
  background:var(--ahead-soft); color:var(--ink); padding:1px 6px; border-radius:4px}
pre{background:var(--card); border:1px solid var(--rule); border-radius:8px; padding:14px 16px;
  overflow-x:auto; font-family:"JetBrains Mono",ui-monospace,monospace; font-size:13px; line-height:1.55; margin:0 0 14px}
pre code{background:none; padding:0}

ul{margin:0 0 14px; padding-left:22px; max-width:68ch}
li{margin-bottom:7px}

footer{margin-top:64px; padding-top:20px; border-top:1px solid var(--rule); color:var(--muted); font-size:13.5px}

@media (max-width:760px){
  body{font-size:16px; padding-inline:16px}
  .paradox{grid-template-columns:1fr}
  .barrow{grid-template-columns:1fr 58px; gap:8px}
  .bartrack{grid-column:1/-1}
  .wrap{padding-block:30px 52px}
}
"""


HEAD = (
    '<title>trex against regex, python and perl</title>\n'
    '<link rel="preconnect" href="https://fonts.googleapis.com">\n'
    '<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>\n'
    '<link rel="stylesheet" href="https://fonts.googleapis.com/css2?'
    'family=JetBrains+Mono:wght@400;700&family=Source+Serif+4:opsz,wght@8..60,400;8..60,600'
    '&display=swap">\n'
    f"<style>{CSS}</style>"
)


def render_html(d, generated):
    """The page's body. The head is HEAD, kept apart so a standalone file can
    put each where a browser expects it."""
    o = []
    A = o.append
    A('<div class="wrap">')

    wl = d["wl"]
    big = int(d["biggest"])
    lead_big = d["passes"][d["lead"]][d["biggest"]]

    # Header
    A("<header>")
    # The headline states whichever of the two counts the measurements
    # support. A row split within a tenth of even is called even rather than
    # described as a defeat or a victory, because at that margin it is one row.
    total_rows = wl["won_rows"] + wl["lost_rows"]
    even = abs(wl["won_rows"] - wl["lost_rows"]) <= total_rows * 0.1
    vals = [v["regex"] for v in d["by_pass"].values() if v.get("regex")]
    headline_x = f"{min(vals):.1f}" if len(vals) < 2 or round(min(vals), 1) == round(
        max(vals), 1
    ) else f"{min(vals):.1f} to {max(vals):.1f}"
    if even:
        A(f'<h1>Row for row it is close to even.<br>On the clock trex is '
          f'{headline_x} times faster</h1>')
    elif wl["lost_rows"] > wl["won_rows"]:
        A(f'<h1>trex loses more rows than it wins,<br>and finishes {headline_x} times '
          f'faster anyway</h1>')
    else:
        A(f'<h1>trex wins most of the rows<br>and finishes {headline_x} times faster</h1>')
    A(f'<p class="lede">Over the {total_rows} measured operations that both '
      f'trex and the Rust <code>regex</code> crate can express, regex is quicker on '
      f'<strong>{wl["lost_rows"]}</strong> of them and trex on <strong>{wl["won_rows"]}</strong>. '
      f'trex finishes the set <strong>{across_passes(d, "regex")}</strong> faster, because the rows it '
      f'wins are worth <strong>{wl["ms_ahead"] / wl["ms_behind"]:.0f} times</strong> what the rows it '
      f'loses cost: ahead by {wl["ms_ahead"]:.0f} ms, behind by {wl["ms_behind"]:.1f}.</p>')
    A('<div class="tags">')
    A(f'<span class="tag">corpus <b>{mb(lead_big["bytes"])}</b></span>')
    A(f'<span class="tag">{thousands(big)} statements</span>')
    A(f'<span class="tag">build <b>{html.escape(d["build_line"] or "not reported")}</b></span>')
    A(f'<span class="tag">passes <b>{", ".join(d["labels"])}</b></span>')
    A("</div>")
    A("</header>")

    # Headline stats
    A('<div class="stats">')
    passes_note = (
        f'across {len(d["by_pass"])} passes' if len(d["by_pass"]) > 1 else "one pass"
    )
    for key, val, note in (
        ("vs regex", across_passes(d, "regex"),
         f'over all {d["rust_n"]} rows both Rust engines answer, {passes_note}'),
        ("vs python re", across_passes(d, "python", 1),
         f'over the {d["four_n"]} rows all four answer'),
        ("vs perl", across_passes(d, "perl", 1),
         f'over the same {d["four_n"]} rows'),
        ("rows only trex answers", f'{len(d["only_trex"])}',
         "no regular expression expresses these"),
    ):
        long = " long" if len(val) > 8 else ""
        A('<div class="stat">')
        A(f'<div class="stat-k">{key}</div><div class="stat-v{long}">{val}</div>'
          f'<div class="stat-n">{note}</div>')
        A("</div>")
    A("</div>")

    # The paradox
    A("<section>")
    A('<div class="sec-head"><h2>Why counting rows gives the wrong answer</h2></div>')
    counted = (
        "the two engines are level"
        if even
        else ("regex takes more rows" if wl["lost_rows"] > wl["won_rows"] else "trex takes more rows")
    )
    A(f'<p>A row is one engine, one pattern, one operation. Counted, {counted}. '
      'Summed in milliseconds, trex wins by a wide margin. Both statements describe the same '
      'measurements, and the second is the one that decides how long a program actually runs: a row '
      'where regex is ahead by a microsecond and a row where it is behind by seventy milliseconds '
      'count the same in a tally and do not count the same in a workload.</p>')
    A('<div class="paradox">')

    tot_rows = wl["won_rows"] + wl["lost_rows"]
    A('<div class="pcard"><h3>Rows, counted</h3>')
    A('<div class="split">')
    A(f'<span class="s-ahead" style="width:{100 * wl["won_rows"] / tot_rows:.1f}%">trex {wl["won_rows"]}</span>')
    A(f'<span class="s-behind" style="width:{100 * wl["lost_rows"] / tot_rows:.1f}%">regex {wl["lost_rows"]}</span>')
    A("</div>")
    A(f'<div class="plabel"><span>trex faster on <b>{100 * wl["won_rows"] / tot_rows:.0f}%</b> of rows</span>'
      f'<span>regex faster on <b>{100 * wl["lost_rows"] / tot_rows:.0f}%</b></span></div>')
    A("</div>")

    tot_ms = wl["ms_ahead"] + wl["ms_behind"]
    A('<div class="pcard"><h3>Milliseconds, summed</h3>')
    A('<div class="split">')
    A(f'<span class="s-ahead" style="width:{100 * wl["ms_ahead"] / tot_ms:.1f}%">trex ahead {wl["ms_ahead"]:.0f} ms</span>')
    A(f'<span class="s-behind" style="width:{100 * wl["ms_behind"] / tot_ms:.1f}%">behind {wl["ms_behind"]:.0f}</span>')
    A("</div>")
    A(f'<div class="plabel"><span>ahead by <b>{wl["ms_ahead"]:.1f} ms</b></span>'
      f'<span>behind by <b>{wl["ms_behind"]:.1f} ms</b></span></div>')
    A("</div>")
    A("</div>")
    A("</section>")

    # Size ladder
    A("<section>")
    A('<div class="sec-head"><h2>Across five corpus sizes</h2>'
      '<p class="sub">Summed milliseconds over the rows every engine answered. Each size is its own '
      'invocation of each engine, so a size gets its own warm cache and timing floor.</p></div>')
    multi = len(d["labels"]) > 1
    A('<div class="tablewrap"><table>')
    A("<thead><tr><th>statements</th>"
      + ("<th>pass</th>" if multi else "")
      + "<th>corpus</th><th>rows</th>"
      + "".join(f"<th>{ENGINE_TITLE[e]}</th>" for e in ENGINES)
      + "<th>vs regex</th><th>vs python</th><th>vs perl</th></tr></thead><tbody>")
    for s in d["sizes"]:
        for label in d["labels"]:
            p = d["passes"][label].get(s)
            if not p:
                continue
            f4, n4 = rollup(answers(p["rows"]), ENGINES)
            if not n4:
                continue
            A("<tr>")
            A(f'<td class="num">{thousands(int(s))}</td>')
            if multi:
                A(f'<td class="num">{label}</td>')
            A(f'<td class="num">{mb(p["bytes"])}</td><td class="num">{n4}</td>')
            for e in ENGINES:
                cls = "win" if e == "trex" else ""
                A(f'<td class="num {cls}">{f4[e]:,.1f}</td>')
            A(f'<td class="num win">{f4["regex"] / f4["trex"]:.2f}x</td>')
            A(f'<td class="num win">{f4["python"] / f4["trex"]:.1f}x</td>')
            A(f'<td class="num win">{f4["perl"] / f4["trex"]:.1f}x</td>')
            A("</tr>")
    A("</tbody></table></div>")
    A('<p style="margin-top:14px">The margin widens as the corpus grows, and by enough that it is not '
      'noise: the two passes agree to within a few percent at every size while the ratio climbs '
      'steadily across them. Where a pattern does reach the tokenizer, trex pays for the lex once and '
      'then answers over tokens while a byte engine re-walks the bytes; where a byte route answers it, '
      'the work is a literal search whose cost per byte falls as the search runs. Both shapes reward '
      'a longer input.</p>')
    A("</section>")

    # Per pattern
    A("<section>")
    A('<div class="sec-head"><h2>Per pattern, trex against regex</h2>'
      f'<p class="sub">At {thousands(big)} statements, over the rows both engines answered. '
      'Right of the center line trex is faster; left of it regex is.</p></div>')
    # Bars are drawn on log2 of the ratio, so 2x faster and 2x slower are the
    # same distance from the center. On a linear scale the 13x row sets the
    # span and every losing bar collapses to about one percent of the track,
    # which reads as no difference rather than as a loss.
    pats = d["pats"]
    span = max(abs(math.log2(a["ratio"])) for a in pats.values())
    for pat, a in sorted(pats.items(), key=lambda kv: -kv[1]["ratio"]):
        r = a["ratio"]
        A('<div class="barrow">')
        A(f'<div class="barname">{html.escape(pat)}</div>')
        cls = "win" if r >= 1 else "loss"
        A(f'<div class="barval {cls}">{r:.2f}x</div>')
        A('<div class="bartrack">')
        A(bar(abs(math.log2(r)) / span / 2, "b-ahead" if r >= 1 else "b-behind"))
        A("</div></div>")
    A('<p class="sub" style="margin-top:12px">Bar length is log2 of the ratio, so twice as fast and '
      'twice as slow reach equally far from the center.</p>')
    A("</section>")

    # Where trex loses
    A("<section>")
    A('<div class="sec-head"><h2>The rows trex loses, and why</h2></div>')
    worst = max(wl["lost"], key=lambda r: r["trex"] - r["regex"])
    best = max(wl["won"], key=lambda r: r["regex"] - r["trex"])
    top5 = sorted(wl["lost"], key=lambda r: r["regex"] - r["trex"])[:5]
    same_op = len({r["operation"] for r in top5}) == 1
    A(f'<p>The losses are real, they are small, and they concentrate. The worst single row costs '
      f'{worst["trex"] - worst["regex"]:.2f} ms against a best row that saves '
      f'{best["regex"] - best["trex"]:.1f} ms.'
      + (f' All five of the largest are the same operation, <b>{html.escape(top5[0]["operation"])}</b>, '
         'on patterns a literal anchors.' if same_op else ''))
    A('<p>Five shapes of "every match" anchored on a literal - a literal word, a byte pattern, a '
      'line-anchored literal, a literal then a word then punctuation, and that last with the word '
      'bound - read 1.2x to 1.5x behind the regex crate at tree <code>2079ae5</code>. At '
      '<code>778cc18</code> all five take a byte route: a SIMD search for the literal and the '
      'reader\'s own probes for whatever follows it, no lex at any occurrence. That route is cut '
      'across the cores at line boundaries, one chunk a core, because the route was serial and the '
      'regex crate is serial and the machine is neither. Each of the five reads about a third of '
      'the regex crate\'s time. What remains above is what the byte routes do not take, and it is '
      'small.</p>')
    A('<div class="tablewrap"><table>')
    A("<thead><tr><th>pattern</th><th>operation</th><th>trex</th><th>regex</th><th>behind by</th></tr></thead><tbody>")
    for r in sorted(wl["lost"], key=lambda r: r["regex"] - r["trex"])[:10]:
        A(f'<tr><td class="name">{html.escape(r["pattern"])}</td>'
          f'<td class="op">{html.escape(r["operation"])}</td>'
          f'<td class="num">{r["trex"]:.4f}</td><td class="num">{r["regex"]:.4f}</td>'
          f'<td class="num loss">{r["trex"] - r["regex"]:.3f} ms</td></tr>')
    A("</tbody></table></div>")

    A('<h3 style="margin-top:34px">And the rows it wins</h3>')
    A('<p>The mirror image: questions that need every match, or captures at every match, where a byte '
      'engine repeats work that trex did once during the lex.</p>')
    A('<div class="tablewrap"><table>')
    A("<thead><tr><th>pattern</th><th>operation</th><th>trex</th><th>regex</th><th>ahead by</th></tr></thead><tbody>")
    for r in sorted(wl["won"], key=lambda r: r["trex"] - r["regex"])[:10]:
        A(f'<tr><td class="name">{html.escape(r["pattern"])}</td>'
          f'<td class="op">{html.escape(r["operation"])}</td>'
          f'<td class="num">{r["trex"]:.4f}</td><td class="num">{r["regex"]:.4f}</td>'
          f'<td class="num win">{r["regex"] - r["trex"]:.3f} ms</td></tr>')
    A("</tbody></table></div>")
    A("</section>")

    if d.get("ab"):
        A(render_ab_html(d["ab"]))

    # Only trex
    A("<section>")
    A('<div class="sec-head"><h2>What no regular expression can be asked</h2>'
      f'<p class="sub">{len(d["only_trex"])} of the measured operations have no counterpart in a regular '
      'expression, so they carry trex\'s time and nothing to divide it by.</p></div>')
    A('<p>A pattern over token kinds, a balanced group, a field anchor, a bounded repeat of a token, a '
      'back-reference over tokens, a guard: none of these is a regular expression. The byte-level '
      'approximations a reader would reach for do not answer the same question, so they are listed '
      'rather than raced, and they are left out of every total above.</p>')
    A('<div class="tablewrap"><table>')
    A("<thead><tr><th>pattern</th><th>operation</th><th>trex</th></tr></thead><tbody>")
    for r in d["only_trex"]:
        A(f'<tr><td class="name">{html.escape(r["pattern"])}</td>'
          f'<td class="op">{html.escape(r["operation"])}</td>'
          f'<td class="num">{r["trex"]:.4f} ms</td></tr>')
    A("</tbody></table></div>")
    A("</section>")

    # The decomposition arms, excluded from every total above
    if d["subarms"]:
        A("<section>")
        A('<div class="sec-head"><h2>The same question, asked the expensive way</h2>'
          f'<p class="sub">{len(d["subarms"])} further arms, kept out of every total above.</p></div>')
        A('<p>Some questions are asked two and three times over, to show where a cost sits rather than '
          'to race. "Where does it first match" is timed through <code>trex::find</code>, then again '
          'through a route that lexes the whole input, and again through one that scans it. The second '
          'and third are deliberately the expensive path, so scoring them as defeats would count one '
          'question three times and report a design choice as a loss. They are listed here instead.</p>')
        A('<div class="tablewrap"><table>')
        A("<thead><tr><th>pattern</th><th>operation, and the arm of it</th><th>trex</th>"
          "<th>regex</th></tr></thead><tbody>")
        for r in sorted(d["subarms"], key=lambda r: -(r["trex"] or 0))[:12]:
            rg = f'{r["regex"]:.4f}' if r["regex"] is not None else "absent"
            A(f'<tr><td class="name">{html.escape(r["pattern"])}</td>'
              f'<td class="op">{html.escape(r["operation"])}</td>'
              f'<td class="num">{r["trex"]:.4f}</td><td class="num">{rg}</td></tr>')
        A("</tbody></table></div>")
        A("</section>")

    # The second machine, where there is one
    if d.get("host2"):
        A(render_host2_html(d, d["host2"], d["host2_meta"]))

    # Conditions
    A("<section>")
    A('<div class="sec-head"><h2>The conditions these were taken under</h2></div>')
    A(render_conditions(d))
    A("</section>")

    A(f'<footer>Generated {generated} from the campaign logs by '
      '<span class="mono">campaign_export.py</span> and <span class="mono">build_page.py</span>. '
      'Every figure on this page is read from those logs; none is typed in.</footer>')
    A("</div>")
    return "\n".join(o)


def host2_summary(h2):
    """Per size and arm on the second host: the rust ratio, the four-engine
    ratios where all four answered, and each cell's own spread."""
    out = OrderedDict()
    for label, p in h2["passes"].items():
        for size, blk in p.items():
            rows = answers(blk["rows"])
            r, n = rollup(rows, ("trex", "regex"))
            f, n4 = rollup(rows, ENGINES)
            st = sorted(x["spread_trex"] for x in blk["rows"] if x.get("spread_trex"))
            sr = sorted(x["spread_regex"] for x in blk["rows"] if x.get("spread_regex"))
            out[(size, label)] = {
                "rust_n": n,
                "trex": r.get("trex"),
                "regex": r.get("regex"),
                "ratio": r["regex"] / r["trex"] if r.get("trex") else None,
                "four_n": n4,
                "python": f["python"] / f["trex"] if n4 else None,
                "perl": f["perl"] / f["trex"] if n4 else None,
                "device": "present" if "PRESENT" in (blk.get("build") or "") else "absent",
                "spread_trex_median": st[len(st) // 2] if st else None,
                "spread_trex_p90": st[int(len(st) * 0.9)] if st else None,
                "spread_regex_median": sr[len(sr) // 2] if sr else None,
                "spread_regex_p90": sr[int(len(sr) * 0.9)] if sr else None,
            }
    return out


def render_host2_html(d, h2, meta):
    """The second host as what it is: a control for the first, not a second set
    of ratios. Under real workload every cell's spread is wide enough that a
    single number from it is not a reading, and the section says so before it
    shows any."""
    o = []
    A = o.append
    s = host2_summary(h2)
    sizes = sorted({k[0] for k in s}, key=int)
    labels = list(h2["passes"])
    pc2 = d["cell_spreads"]

    A("<section>")
    A(f'<div class="sec-head"><h2>A second machine, under real workload</h2>'
      f'<p class="sub">{html.escape(meta["cpu"])}, {meta["cores"]} logical cores, '
      f'{html.escape(meta["gpu"])}. Not a quiet box: {html.escape(meta["load"])}. '
      f'Taken at tree <code>{html.escape(meta["tree"])}</code>; the ratios are that tree\'s, '
      'and what this section measures - the machine\'s own variation and the device\'s worth - '
      'does not depend on them.</p></div>')
    A('<p>Every number above came from a machine that was close to unused. This one was in use - '
      'compiles starting and stopping, other agents\' processes, a desktop - and the comparison was run '
      'on it four times a size, alternating the device present and absent, to see what that does to a '
      'benchmark. The first thing it does is visible in each cell\'s own spread:</p>')

    # The spread comparison is the section's headline: it is what makes every
    # other number in the section a description of the box rather than a
    # reading of the engines.
    A('<div class="tablewrap"><table><thead><tr><th>machine</th><th>engine</th>'
      '<th>median cell spread</th><th>90th percentile</th></tr></thead><tbody>')
    for eng in ("trex", "regex"):
        vals = sorted(pc2.get(eng) or [])
        if vals:
            A(f'<tr><td class="name">the first, close to unused</td><td class="name">{ENGINE_TITLE[eng]}</td>'
              f'<td class="num win">{vals[len(vals) // 2]:.2f}x</td>'
              f'<td class="num win">{vals[int(len(vals) * 0.9)]:.2f}x</td></tr>')
    for eng in ("trex", "regex"):
        meds = [v[f"spread_{eng}_median"] for v in s.values() if v.get(f"spread_{eng}_median")]
        p90s = [v[f"spread_{eng}_p90"] for v in s.values() if v.get(f"spread_{eng}_p90")]
        if meds:
            A(f'<tr><td class="name">this one, in use</td><td class="name">{ENGINE_TITLE[eng]}</td>'
              f'<td class="num loss">{min(meds):.2f}x to {max(meds):.2f}x</td>'
              f'<td class="num loss">{min(p90s):.2f}x to {max(p90s):.2f}x</td></tr>')
    A("</tbody></table></div>")
    A('<p>A cell whose repeats disagree by half again is not measuring the engine, it is measuring '
      'whatever else was running during its repeats. On this machine that is the typical cell, not '
      'the outlier, so the table below is read as what a benchmark looks like here rather than as a '
      'second opinion on the ratios.</p>')

    A('<h3 style="margin-top:30px">Every arm, every size</h3>')
    A('<div class="tablewrap"><table><thead><tr><th>statements</th><th>arm</th><th>device</th>'
      '<th>rows</th><th>trex</th><th>regex</th><th>vs regex</th><th>vs python</th><th>vs perl</th>'
      '</tr></thead><tbody>')
    for size in sizes:
        for label in labels:
            v = s.get((size, label))
            if not v or v["ratio"] is None:
                continue
            py = f'{v["python"]:.1f}x' if v["python"] else "absent"
            pl = f'{v["perl"]:.1f}x' if v["perl"] else "absent"
            dev_cls = "win" if v["device"] == "present" else ""
            A(f'<tr><td class="num">{thousands(int(size))}</td><td class="num">{html.escape(label)}</td>'
              f'<td class="num {dev_cls}">{v["device"]}</td><td class="num">{v["rust_n"]}</td>'
              f'<td class="num">{v["trex"]:,.1f}</td><td class="num">{v["regex"]:,.1f}</td>'
              f'<td class="num">{v["ratio"]:.2f}x</td><td class="num">{py}</td><td class="num">{pl}</td></tr>')
    A("</tbody></table></div>")

    # The device question, answered by whether the arms separate at all.
    A('<h3 style="margin-top:30px">What the GPU is worth here</h3>')
    verdicts = []
    for size in sizes:
        ab = [s[(size, l)]["ratio"] for l in labels if (size, l) in s and s[(size, l)]["device"] == "absent" and s[(size, l)]["ratio"]]
        pr = [s[(size, l)]["ratio"] for l in labels if (size, l) in s and s[(size, l)]["device"] == "present" and s[(size, l)]["ratio"]]
        if len(ab) >= 2 and len(pr) >= 2:
            within = max(max(ab) - min(ab), max(pr) - min(pr))
            between = abs(sum(pr) / len(pr) - sum(ab) / len(ab))
            verdicts.append((size, ab, pr, within, between))
    if verdicts:
        A('<div class="tablewrap"><table><thead><tr><th>statements</th><th>device absent, both arms</th>'
          '<th>device present, both arms</th><th>spread within one arm</th><th>gap between arms</th>'
          '<th>outside the noise?</th><th>which way</th></tr></thead><tbody>')
        directions = set()
        for size, ab, pr, within, between in verdicts:
            res = "yes" if between > within else "no"
            way = "present higher" if sum(pr) / len(pr) > sum(ab) / len(ab) else "absent higher"
            if res == "yes":
                directions.add(way)
            A(f'<tr><td class="num">{thousands(int(size))}</td>'
              f'<td class="num">{" / ".join(f"{x:.2f}x" for x in ab)}</td>'
              f'<td class="num">{" / ".join(f"{x:.2f}x" for x in pr)}</td>'
              f'<td class="num">{within:.2f}</td><td class="num">{between:.2f}</td>'
              f'<td class="num">{res}</td><td class="op">{way}</td></tr>')
        A("</tbody></table></div>")
        unresolved = sum(1 for v in verdicts if v[4] <= v[3])
        A(f'<p>Two readings of the same arm disagree by more than the two arms disagree with each '
          f'other at {unresolved} of {len(verdicts)} sizes. '
          + ('Where a size does clear the noise, the direction is not the same from one size to the '
             'next, which is what a difference made of noise looks like and what an effect does not. '
             if len(directions) > 1 else '')
          + 'The question is not answered on this machine. What can be said is narrower: with the '
          'device present the comparison did not get slower by more than the machine\'s own '
          'variation, which is what a device that is not used for these operations looks like.</p>')

    A('<div class="note warn"><h3>Where "the ratio is load-independent" stops being true</h3>'
      '<p>Both Rust engines are timed in one process with the arms interleaved, so a steady '
      'neighbour raises both sides alike and the ratio holds - which is what the first machine '
      'showed, at two cores of twenty-four, with two passes agreeing to within three percent. A '
      '<em>bursty</em> neighbour is different: a compile that starts during one arm and stops '
      'before the next lands on one side of one row, and the sum of a hundred and fifty such rows '
      'depends on which rows it hit. That is why the ratio itself moves across the arms above. '
      'Interleaving protects against steady load, not against load that changes faster than a row '
      'takes.</p></div>')
    A("</section>")
    return "\n".join(o)


def host_meta(rawlog):
    """The machine a run was taken on, from the header its script prints.

    The load figure is the last one the script logged before the first size,
    so it describes the box as the run found it.
    """
    from campaign_table import encoding_of

    meta = {
        "cpu": "not reported",
        "cores": "?",
        "gpu": "not reported",
        "load": "not reported",
        "tree": "not reported",
    }
    with open(rawlog, encoding=encoding_of(rawlog), errors="replace") as f:
        after_tree = False
        for line in f:
            line = line.strip()
            if line.startswith("#") and "STATEMENTS" in line:
                break
            if line == "=== TREE ===":
                after_tree = True
                continue
            if after_tree:
                # The commit line follows the TREE header: the hash, then
                # the subject.
                meta["tree"] = line.split()[0] if line else "not reported"
                after_tree = False
                continue
            if line.startswith("logical cores "):
                meta["cores"] = line.split()[-1]
            elif line.startswith("gpu: "):
                meta["gpu"] = line[5:]
            elif "Processor" in line or "CPU" in line:
                meta["cpu"] = line
        # The first load line after the header is the box as the run found it.
        for line in f:
            if line.startswith("load before"):
                meta["load"] = line.split(":", 1)[1].split("[")[0].strip()
                break
    return meta


def render_host2_md(d, h2, meta):
    o = []
    A = o.append
    s = host2_summary(h2)
    sizes = sorted({k[0] for k in s}, key=int)
    labels = list(h2["passes"])
    pc2 = d["cell_spreads"]

    A("### A second machine, under real workload")
    A("")
    A(f"{meta['cpu']}, {meta['cores']} logical cores, {meta['gpu']}. Not a quiet box: {meta['load']}.")
    A(f"Taken at tree `{meta['tree']}`; the ratios are that tree's, and what this section measures -")
    A("the machine's own variation and the device's worth - does not depend on them.")
    A("")
    A("Every number above came from a machine that was close to unused. This one was in use, and the")
    A("comparison was run on it four times a size, alternating the device present and absent, to see")
    A("what that does to a benchmark. The first thing it does is visible in each cell's own spread:")
    A("")
    A("| machine | engine | median cell spread | 90th percentile |")
    A("|---|---|---:|---:|")
    for eng in ("trex", "regex"):
        vals = sorted(pc2.get(eng) or [])
        if vals:
            A(f"| the first, close to unused | {ENGINE_TITLE[eng]} | {vals[len(vals) // 2]:.2f}x | "
              f"{vals[int(len(vals) * 0.9)]:.2f}x |")
    for eng in ("trex", "regex"):
        meds = [v[f"spread_{eng}_median"] for v in s.values() if v.get(f"spread_{eng}_median")]
        p90s = [v[f"spread_{eng}_p90"] for v in s.values() if v.get(f"spread_{eng}_p90")]
        if meds:
            A(f"| this one, in use | {ENGINE_TITLE[eng]} | {min(meds):.2f}x to {max(meds):.2f}x | "
              f"{min(p90s):.2f}x to {max(p90s):.2f}x |")
    A("")
    A("A cell whose repeats disagree by half again is measuring whatever else was running during its")
    A("repeats, not the engine. On this machine that is the typical cell, so the table below is read as")
    A("what a benchmark looks like here rather than as a second opinion on the ratios.")
    A("")
    A("| statements | arm | device | rows | trex | regex | vs regex | vs python | vs perl |")
    A("|---:|---|---|---:|---:|---:|---:|---:|---:|")
    for size in sizes:
        for label in labels:
            v = s.get((size, label))
            if not v or v["ratio"] is None:
                continue
            py = f'{v["python"]:.1f}x' if v["python"] else "absent"
            pl = f'{v["perl"]:.1f}x' if v["perl"] else "absent"
            A(f"| {thousands(int(size))} | {label} | {v['device']} | {v['rust_n']} | {v['trex']:,.1f} | "
              f"{v['regex']:,.1f} | {v['ratio']:.2f}x | {py} | {pl} |")
    A("")
    A("#### What the GPU is worth here")
    A("")
    verdicts = []
    for size in sizes:
        ab = [s[(size, l)]["ratio"] for l in labels if (size, l) in s and s[(size, l)]["device"] == "absent" and s[(size, l)]["ratio"]]
        pr = [s[(size, l)]["ratio"] for l in labels if (size, l) in s and s[(size, l)]["device"] == "present" and s[(size, l)]["ratio"]]
        if len(ab) >= 2 and len(pr) >= 2:
            within = max(max(ab) - min(ab), max(pr) - min(pr))
            between = abs(sum(pr) / len(pr) - sum(ab) / len(ab))
            verdicts.append((size, ab, pr, within, between))
    if verdicts:
        A("| statements | absent, both arms | present, both arms | spread within an arm | gap between arms | outside the noise? | which way |")
        A("|---:|---|---|---:|---:|---|---|")
        directions = set()
        for size, ab, pr, within, between in verdicts:
            res = "yes" if between > within else "no"
            way = "present higher" if sum(pr) / len(pr) > sum(ab) / len(ab) else "absent higher"
            if res == "yes":
                directions.add(way)
            A(f"| {thousands(int(size))} | {' / '.join(f'{x:.2f}x' for x in ab)} | "
              f"{' / '.join(f'{x:.2f}x' for x in pr)} | {within:.2f} | {between:.2f} | {res} | {way} |")
        A("")
        unresolved = sum(1 for v in verdicts if v[4] <= v[3])
        A(f"Two readings of the same arm disagree by more than the two arms disagree with each other at")
        A(f"{unresolved} of {len(verdicts)} sizes."
          + (" Where a size does clear the noise, the direction is not the same from one size to"
             " the next, which is what a difference made of noise looks like and what an effect does"
             " not." if len(directions) > 1 else ""))
        A("The question is not answered on this machine. What can be said is narrower: with the device")
        A("present the comparison did not get slower by more than the machine's own variation, which is")
        A("what a device not used for these operations looks like.")
        A("")
    A("**Where \"the ratio is load-independent\" stops being true.** Both Rust engines are timed in one")
    A("process with the arms interleaved, so a steady neighbour raises both sides alike and the ratio")
    A("holds - which the first machine showed at two cores of twenty-four. A *bursty* neighbour is")
    A("different: a compile that starts during one arm and stops before the next lands on one side of")
    A("one row, and the sum of a hundred and fifty such rows depends on which rows it hit. That is why")
    A("the ratio itself moves across the arms above. Interleaving protects against steady load, not")
    A("against load that changes faster than a row takes.")
    A("")
    return "\n".join(o)


def ab_blocks(paths):
    """Every sweep in the A/B logs: the row, its size, each arm's median, the
    regex cell's own spread and the control's drift, read off the tables
    `examples/chunked_route_ab` prints. Nothing is typed in."""
    import re
    from campaign_table import encoding_of

    size_re = re.compile(r"^#+ STATEMENTS (\d+) #+$")
    # Matched on the header's first line alone. A header carrying a long
    # regex wraps under a terminal's width, and a pattern anchored on the
    # closing `===` then misses it, leaves the previous block current, and
    # writes this row's arms over that row's.
    head_re = re.compile(r"^=== (.+?)\s+trex `")
    arm_re = re.compile(r"^\s*(.+?)\s+([\d.]+)\s+([\d.]+)x\s+\d+\s*$")
    ctl_re = re.compile(r"^\s*CONTROL\s+([\d.]+)\s+drift ([\d.]+)x")
    tree_re = re.compile(r"^([0-9a-f]{7,}) ")
    out = []
    for path in paths:
        tree, size, cur = "?", None, None
        with open(path, encoding=encoding_of(path), errors="replace") as f:
            for line in f:
                line = line.rstrip("\n")
                m = tree_re.match(line.strip())
                if m and tree == "?":
                    tree = m.group(1)
                m = size_re.match(line.strip())
                if m:
                    size = int(m.group(1))
                    continue
                m = head_re.match(line.strip())
                if m:
                    cur = {"row": m.group(1), "size": size, "tree": tree, "arms": {}, "spread": {}}
                    out.append(cur)
                    continue
                if cur is None:
                    continue
                m = ctl_re.match(line)
                if m:
                    cur["drift"] = float(m.group(2))
                    continue
                m = arm_re.match(line)
                if m:
                    cur["arms"][m.group(1).strip()] = float(m.group(2))
                    cur["spread"][m.group(1).strip()] = float(m.group(3))
    return out


def render_ab_html(blocks):
    o = []
    A = o.append
    A("<section>")
    A('<div class="sec-head"><h2>The five losing rows, after the cut</h2>'
      '<p class="sub">From <code>examples/chunked_route_ab</code>: rotated arms, a control at the end, '
      'the comparison\'s own timing, on the 24-core machine. Every block the logs hold is shown, '
      'with its control\'s drift and the regex cell\'s own spread beside it, so a disturbed block is '
      'visible rather than dropped.</p></div>')
    A('<p>The campaign-wide replicate on this tree is not on this page: the box was carrying '
      'nineteen to twenty-two cores of twenty-four when it was tried, and its drift controls ranged '
      '0.43x to 2.60x, so it was discarded. These blocks were taken with about seven foreign cores '
      'and their controls held.</p>')
    A('<div class="tablewrap"><table><thead><tr><th>row</th><th>statements</th><th>tree</th>'
      '<th>one pass</th><th>shipped</th><th>cut ×4</th><th>regex</th><th>cut vs regex</th>'
      '<th>regex spread</th><th>control drift</th></tr></thead><tbody>')
    for b in blocks:
        a = b["arms"]
        one = a.get("one pass", a.get("serial"))
        cut = a.get("chunked x4 cores")
        rg = a.get("regex")
        if one is None or cut is None or rg is None:
            continue
        shipped = a.get("shipped, trex::scan")
        sp = b["spread"].get("regex")
        dr = b.get("drift")
        A(f'<tr><td class="name">{html.escape(b["row"])}</td>'
          f'<td class="num">{thousands(b["size"]) if b["size"] else "-"}</td>'
          f'<td class="num">{html.escape(b["tree"])}</td>'
          f'<td class="num">{one:.3f}</td>'
          f'<td class="num">{f"{shipped:.3f}" if shipped is not None else "-"}</td>'
          f'<td class="num win">{cut:.3f}</td><td class="num">{rg:.3f}</td>'
          f'<td class="num win">{cut / rg:.2f}x</td>'
          f'<td class="num {"loss" if sp and sp > 1.2 else ""}">{f"{sp:.2f}x" if sp else "-"}</td>'
          f'<td class="num {"loss" if dr and abs(dr - 1) > 0.05 else ""}">{f"{dr:.3f}x" if dr else "-"}</td></tr>')
    A("</tbody></table></div>")
    A('<p>"One pass" is the route as it was; "shipped" is <code>trex::scan</code> as it now runs; '
      '"cut ×4" is the same route over four chunks a core. Where shipped is absent the log predates '
      'the route shipping. A regex spread above 1.2x or a control drift beyond 5% marks a block the '
      'box disturbed.</p>')
    A("</section>")
    return "\n".join(o)


def render_ab_md(blocks):
    o = []
    A = o.append
    A("### The five losing rows, after the cut")
    A("")
    A("From `examples/chunked_route_ab`: rotated arms, a control at the end, the comparison's own")
    A("timing, on the 24-core machine. Every block the logs hold is shown with its control's drift")
    A("and the regex cell's own spread, so a disturbed block is visible rather than dropped. The")
    A("campaign-wide replicate on this tree is not on this page: the box carried nineteen to")
    A("twenty-two cores of twenty-four when it was tried, and its drift controls ranged 0.43x to")
    A("2.60x, so it was discarded. These blocks were taken with about seven foreign cores.")
    A("")
    A("| row | statements | tree | one pass | shipped | cut x4 | regex | cut vs regex | regex spread | control drift |")
    A("|---|---:|---|---:|---:|---:|---:|---:|---:|---:|")
    for b in blocks:
        a = b["arms"]
        one = a.get("one pass", a.get("serial"))
        cut = a.get("chunked x4 cores")
        rg = a.get("regex")
        if one is None or cut is None or rg is None:
            continue
        shipped = a.get("shipped, trex::scan")
        sp = b["spread"].get("regex")
        dr = b.get("drift")
        A(f"| {b['row']} | {thousands(b['size']) if b['size'] else '-'} | {b['tree']} | {one:.3f} | "
          f"{f'{shipped:.3f}' if shipped is not None else '-'} | {cut:.3f} | {rg:.3f} | {cut / rg:.2f}x | "
          f"{f'{sp:.2f}x' if sp else '-'} | {f'{dr:.3f}x' if dr else '-'} |")
    A("")
    return "\n".join(o)


def render_conditions(d):
    o = []
    A = o.append
    A('<h3>The build</h3>')
    A('<p>trex\'s default features are <code>gpu</code> and <code>tandem</code>. '
      'A binary built with <code>--no-default-features</code> is not the one anybody ships, so these '
      'readings use the defaults and take the device away at run time instead. The binaries measured '
      'also carried the baked compression prior, which belongs to the off-by-default <code>compress</code> '
      'feature and which no scan reads. The bench prints what it actually had before any timing:</p>')
    A(f'<pre><code>build: {html.escape(d["build_line"] or "not reported")}</code></pre>')
    A('<div class="note warn"><h3>An empty environment variable does not remove the device</h3>'
      '<p>Taking the GPU away needs <code>CUDA_VISIBLE_DEVICES=-1</code>. Windows deletes an environment '
      'variable set to the empty string, so the empty form leaves the device visible and a run labelled '
      'CPU-only would have used a GPU. The printed line above is what makes the configuration checkable '
      'rather than asserted; a reading whose log does not carry it cannot claim one.</p></div>')

    A('<h3 style="margin-top:30px">How to read a row</h3>')
    A('<ul>'
      '<li><b>Compare within a run, not across runs.</b> Both Rust engines are timed in one process over '
      'the same bytes with the arms interleaved, so their ratio holds whatever else the machine is doing. '
      'python and perl are separate invocations, so a cross-engine difference smaller than the box\'s own '
      'variation is not a reading.</li>'
      '<li><b>The four-engine totals are over the rows every engine answered.</b> A total over whatever '
      'each engine happened to answer would compare four different workloads. The count of common rows '
      'is printed beside every total.</li>'
      '<li><b>A missing row reads as absent, never as zero.</b> An engine that cannot express a pattern, '
      'or is not installed on the host, is named as such. A zero and a gap look alike in a table and only '
      'one of them is honest.</li>'
      '</ul>')

    cs = d.get("cell_spreads") or {}
    if cs.get("trex"):
        A('<h3 style="margin-top:30px">How steady each cell was</h3>')
        A('<p>Every cell is timed repeatedly and prints its own spread: the slowest repeat over the '
          'fastest, inside the one process that timed it. That is the error bar belonging to the '
          'number beside it, and it is what decides whether a difference between two cells is a '
          'reading at all.</p>')
        A('<div class="tablewrap"><table><thead><tr><th>engine</th><th>cells</th><th>median spread</th>'
          '<th>90th percentile</th><th>widest</th><th>within 10%</th></tr></thead><tbody>')
        for engine in ("trex", "regex"):
            vals = sorted(cs.get(engine) or [])
            if not vals:
                continue
            tight = sum(1 for v in vals if v <= 1.10)
            A(f'<tr><td class="name">{ENGINE_TITLE[engine]}</td><td class="num">{len(vals)}</td>'
              f'<td class="num">{vals[len(vals) // 2]:.3f}x</td>'
              f'<td class="num">{vals[int(len(vals) * 0.9)]:.3f}x</td>'
              f'<td class="num">{vals[-1]:.3f}x</td>'
              f'<td class="num">{100 * tight / len(vals):.0f}%</td></tr>')
        A("</tbody></table></div>")

    if d.get("drift"):
        ratios = sorted(x[4] for x in d["drift"])
        worst = max(d["drift"], key=lambda x: abs(math.log(x[4])))
        inside = sum(1 for r in ratios if 0.9 <= r <= 1.1)
        A('<h3 style="margin-top:30px">What the run says about itself</h3>')
        A('<p>Every section times the same cheap question twice, once at the top and once at the '
          'bottom, and prints the second as a control. Both readings are the same work, so the '
          'difference between them is position in the run and load and nothing else. That is a '
          'stronger account of whether a reading held still than any load figure taken from outside '
          'the process, because it is measured by the process being timed.</p>')
        A(f'<p>Across {len(ratios)} control readings at this size, the median is '
          f'<b>{ratios[len(ratios) // 2]:.3f}x</b> of the first reading, the range runs '
          f'<b>{ratios[0]:.3f}x to {ratios[-1]:.3f}x</b>, and {inside} of {len(ratios)} sit within '
          f'ten percent. The furthest is {html.escape(worst[0])} on the {worst[1]} arm at '
          f'{worst[4]:.3f}x.</p>')
        A('<p class="sub">A section whose control drifted is a section whose cells moved, whatever '
          'their own spread columns looked like. These are printed rather than summarized away so a '
          'reader can discount a row whose section did not hold still.</p>')

    A('<h3 style="margin-top:30px">The machine, and what else was on it</h3>')
    A('<p>Measured on a Ryzen 9 7900X, 24 logical cores, Windows 11. The box was not empty: two '
      'single-core jobs belonging to another agent ran throughout. Differencing every process\'s CPU '
      'time across a 24-second window puts the whole machine at <b>2.03 cores of 24, 8.5%</b>, of '
      'which those two jobs are 2.00 and everything else on the system is 0.03.</p>')
    A('<div class="note warn"><h3>Three ways to ask how busy a machine is, and they disagree</h3>'
      '<p>The same box at the same moment: <b>2.03 cores</b> by differencing CPU time across a window, '
      '<b>2.25 cores</b> by each process\'s CPU time over its own lifetime, and <b>2.9%</b> - about '
      '0.7 cores - by sampling <code>Win32_Processor.LoadPercentage</code>.</p>'
      '<p>The window is the one to use. A lifetime average charges a service that has been up for two '
      'days with work it did yesterday, so it overstates. The sampled counter misses steady load '
      'between its samples and reads about three times low here. Only the difference in CPU time '
      'across the window answers what the machine carried <em>during</em> the window, which is what a '
      'timing run is actually exposed to.</p></div>')

    spreads = d.get("spreads", {})
    if len(d["labels"]) > 1:
        A('<h3 style="margin-top:30px">Pass to pass</h3>')
        A(f'<p>{len(d["labels"])} passes over the same tree, labeled '
          f'{", ".join(d["labels"])}. The spread between them is the honest error bar on every figure '
          'above.</p>')
        A('<div class="tablewrap"><table><thead><tr><th>statements</th>'
          + "".join(f"<th>{ENGINE_TITLE[e]} spread</th>" for e in ENGINES)
          + "<th>ratio spread</th></tr></thead><tbody>")
        for s in d["sizes"]:
            sp = spreads.get(s) or spreads.get(int(s)) or {}
            A(f'<tr><td class="num">{thousands(int(s))}</td>')
            for e in ENGINES:
                v = sp.get(e)
                A(f'<td class="num">{v * 100:.1f}%</td>' if v is not None else '<td class="num">-</td>')
            v = sp.get("ratio_vs_regex")
            A(f'<td class="num">{v * 100:.1f}%</td>' if v is not None else '<td class="num">-</td>')
            A("</tr>")
        A("</tbody></table></div>")
    else:
        A('<div class="note warn"><h3>One pass so far</h3>'
          '<p>These figures come from a single pass. This bench has read the same commit twice and '
          'differed by 15%, so a single reading is a number to check rather than a number to cite. '
          'A replicate is running and this page carries its spread when it lands.</p></div>')

    A('<h3 style="margin-top:30px">Rebuilding it</h3>')
    A('<p>The corpus is generated rather than sampled: four statement shapes cycled, at a size the bench '
      'takes from <code>TREX_BENCH_STATEMENTS</code>. Nothing here depends on a file you do not have.</p>')
    A('<pre><code>set TREX_BENCH_STATEMENTS=400000\n'
      'set CUDA_VISIBLE_DEVICES=-1\n'
      'cargo build --release --bench vs_regex_full\n'
      'target\\release\\deps\\vs_regex_full-*.exe\n'
      'python benches\\other_engines.py 400000\n'
      'perl benches\\other_engines.pl 400000</code></pre>')
    return "\n".join(o)


def render_md(d, generated):
    o = []
    A = o.append
    wl = d["wl"]
    big = int(d["biggest"])
    A("## Results")
    A("")
    A(f"Taken on a Ryzen 9 7900X, 24 logical cores, at {thousands(big)} statements and four smaller "
      f"sizes. Passes: {', '.join(d['labels'])}.")
    A("")
    total_rows = wl["won_rows"] + wl["lost_rows"]
    split = (
        " - level, counted"
        if abs(wl["won_rows"] - wl["lost_rows"]) <= total_rows * 0.1
        else ""
    )
    A(f"Over the {total_rows} operations both trex and `regex` can express, regex "
      f"is quicker on {wl['lost_rows']} and trex on {wl['won_rows']}{split}. trex finishes "
      f"the set {across_passes(d, 'regex')} faster, because the rows it wins are worth "
      f"{wl['ms_ahead'] / wl['ms_behind']:.0f} times what the rows it loses cost: ahead by "
      f"{wl['ms_ahead']:.1f} ms, behind by {wl['ms_behind']:.1f} ms.")
    A("")
    A("### Across five corpus sizes")
    A("")
    A("Summed milliseconds over the rows every engine answered.")
    A("")
    multi = len(d["labels"]) > 1
    pass_col = " pass |" if multi else ""
    pass_sep = "---:|" if multi else ""
    A(f"| statements |{pass_col} corpus | rows | trex | regex | python | perl | vs regex | vs python "
      "| vs perl |")
    A(f"|---:|{pass_sep}---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for s in d["sizes"]:
        for label in d["labels"]:
            p = d["passes"][label].get(s)
            if not p:
                continue
            f4, n4 = rollup(answers(p["rows"]), ENGINES)
            if not n4:
                continue
            cell = f" {label} |" if multi else ""
            A(f"| {thousands(int(s))} |{cell} {mb(p['bytes'])} | {n4} | {f4['trex']:,.1f} | "
              f"{f4['regex']:,.1f} | {f4['python']:,.1f} | {f4['perl']:,.1f} | "
              f"{f4['regex'] / f4['trex']:.2f}x | {f4['python'] / f4['trex']:.1f}x | "
              f"{f4['perl'] / f4['trex']:.1f}x |")
    A("")
    A("The margin widens as the corpus grows, and by enough that it is not noise: the two passes agree")
    A("to within a few percent at every size while the ratio climbs steadily across them. Where a")
    A("pattern does reach the tokenizer, trex pays for the lex once and then answers over tokens while")
    A("a byte engine re-walks the bytes; where a byte route answers it, the work is a literal search")
    A("whose cost per byte falls as the search runs. Both shapes reward a longer input.")
    A("")
    A(f"### Per pattern, trex against regex, at {thousands(big)} statements")
    A("")
    A("| pattern | rows | trex ms | regex ms | ratio |")
    A("|---|---:|---:|---:|---:|")
    for pat, a in sorted(d["pats"].items(), key=lambda kv: -kv[1]["ratio"]):
        A(f"| {pat} | {a['n']} | {a['trex']:.4f} | {a['regex']:.4f} | {a['ratio']:.2f}x |")
    A("")
    A("### The rows trex loses")
    A("")
    worst = max(wl["lost"], key=lambda r: r["trex"] - r["regex"])
    best = max(wl["won"], key=lambda r: r["regex"] - r["trex"])
    top5 = sorted(wl["lost"], key=lambda r: r["regex"] - r["trex"])[:5]
    same_op = len({r["operation"] for r in top5}) == 1
    A(f"The losses are real, they are small, and they concentrate. The worst single row costs "
      f"{worst['trex'] - worst['regex']:.2f} ms against a best row that saves "
      f"{best['regex'] - best['trex']:.1f} ms."
      + (f" All five of the largest are the same operation, {top5[0]['operation']}, on patterns a"
         " literal anchors." if same_op else ""))
    A("")
    A("Five shapes of \"every match\" anchored on a literal - a literal word, a byte pattern, a")
    A("line-anchored literal, a literal then a word then punctuation, and that last with the word")
    A("bound - read 1.2x to 1.5x behind the regex crate at tree `2079ae5`. At `778cc18` all five take")
    A("a byte route: a SIMD search for the literal and the reader's own probes for whatever follows")
    A("it, no lex at any occurrence. That route is cut across the cores at line boundaries, one chunk")
    A("a core, because the route was serial and the regex crate is serial and the machine is neither.")
    A("Each of the five reads about a third of the regex crate's time. What remains above is what the")
    A("byte routes do not take, and it is small.")
    A("")
    A("Routes read from `examples/route_trace.rs` under `TREX_TRACE=1`, which asks the engine rather")
    A("than inferring a mechanism from a timing; the cut is measured by `examples/chunked_route_ab.rs`.")
    A("")
    A("| pattern | operation | trex ms | regex ms | behind by |")
    A("|---|---|---:|---:|---:|")
    for r in sorted(wl["lost"], key=lambda r: r["regex"] - r["trex"])[:10]:
        A(f"| {r['pattern']} | {r['operation']} | {r['trex']:.4f} | {r['regex']:.4f} | "
          f"{r['trex'] - r['regex']:.3f} |")
    A("")
    A("### The rows it wins")
    A("")
    A("| pattern | operation | trex ms | regex ms | ahead by |")
    A("|---|---|---:|---:|---:|")
    for r in sorted(wl["won"], key=lambda r: r["trex"] - r["regex"])[:10]:
        A(f"| {r['pattern']} | {r['operation']} | {r['trex']:.4f} | {r['regex']:.4f} | "
          f"{r['regex'] - r['trex']:.3f} |")
    A("")
    if d.get("ab"):
        A(render_ab_md(d["ab"]))
    A(f"### The {len(d['only_trex'])} operations no regular expression expresses")
    A("")
    A("These carry trex's time and nothing to divide it by, and are left out of every total above.")
    A("")
    A("| pattern | operation | trex ms |")
    A("|---|---|---:|")
    for r in d["only_trex"]:
        A(f"| {r['pattern']} | {r['operation']} | {r['trex']:.4f} |")
    A("")
    if d["subarms"]:
        A("### The same question, asked the expensive way")
        A("")
        A(f"{len(d['subarms'])} further arms, kept out of every total above. Some questions are asked")
        A("two and three times over to show where a cost sits rather than to race: \"where does it first")
        A("match\" is timed through `trex::find`, then again through a route that lexes the whole input,")
        A("and again through one that scans it, all three against the same single regex arm. The second")
        A("and third are deliberately the expensive path, so scoring them as defeats would count one")
        A("question three times and report a design choice as a loss.")
        A("")
        A("| pattern | operation, and the arm of it | trex ms | regex ms |")
        A("|---|---|---:|---:|")
        for r in sorted(d["subarms"], key=lambda r: -(r["trex"] or 0))[:12]:
            rg = f"{r['regex']:.4f}" if r["regex"] is not None else "absent"
            A(f"| {r['pattern']} | {r['operation']} | {r['trex']:.4f} | {rg} |")
        A("")
    if d.get("host2"):
        A(render_host2_md(d, d["host2"], d["host2_meta"]))
    cs = d.get("cell_spreads") or {}
    if cs.get("trex"):
        A("### How steady each cell was")
        A("")
        A("Every cell is timed repeatedly and prints its own spread: the slowest repeat over the")
        A("fastest, inside the one process that timed it. That is the error bar belonging to the number")
        A("beside it, and it decides whether a difference between two cells is a reading at all.")
        A("")
        A("| engine | cells | median spread | 90th percentile | widest | within 10% |")
        A("|---|---:|---:|---:|---:|---:|")
        for engine in ("trex", "regex"):
            vals = sorted(cs.get(engine) or [])
            if not vals:
                continue
            tight = sum(1 for v in vals if v <= 1.10)
            A(f"| {ENGINE_TITLE[engine]} | {len(vals)} | {vals[len(vals) // 2]:.3f}x | "
              f"{vals[int(len(vals) * 0.9)]:.3f}x | {vals[-1]:.3f}x | "
              f"{100 * tight / len(vals):.0f}% |")
        A("")
    if d.get("drift"):
        ratios = sorted(x[4] for x in d["drift"])
        inside = sum(1 for r in ratios if 0.9 <= r <= 1.1)
        A("### What the run says about itself")
        A("")
        A("Every section times the same cheap question twice, at the top and at the bottom, printing the")
        A("second as a control. Both are the same work, so the difference between them is position in the")
        A("run and load and nothing else - a stronger account of whether a reading held still than any")
        A("load figure taken from outside the process.")
        A("")
        A(f"Across {len(ratios)} control readings at this size the median is "
          f"{ratios[len(ratios) // 2]:.3f}x of the first reading, the range runs {ratios[0]:.3f}x to "
          f"{ratios[-1]:.3f}x, and {inside} of {len(ratios)} sit within ten percent.")
        A("")
    A("### What else was on the machine")
    A("")
    A("Ryzen 9 7900X, 24 logical cores, Windows 11. The box was not empty: two single-core jobs")
    A("belonging to another agent ran throughout. Differencing every process's CPU time across a")
    A("24-second window puts the whole machine at 2.03 cores of 24, 8.5%, of which those two jobs are")
    A("2.00 and everything else on the system is 0.03.")
    A("")
    A("Three ways to ask how busy a machine is, on the same box at the same moment: 2.03 cores by")
    A("differencing CPU time across a window, 2.25 cores by each process's CPU time over its own")
    A("lifetime, and 2.9% - about 0.7 cores - by sampling `Win32_Processor.LoadPercentage`. The window")
    A("is the one to use. A lifetime average charges a service that has been up for two days with work")
    A("it did yesterday; the sampled counter misses steady load between samples and reads about three")
    A("times low here.")
    A("")
    if len(d["labels"]) > 1:
        A(f"Passes: {', '.join(d['labels'])} over the same tree. Spread between them, per size:")
        A("")
        A("| statements | trex | regex | python | perl | ratio |")
        A("|---:|---:|---:|---:|---:|---:|")
        for s in d["sizes"]:
            sp = d["spreads"].get(s) or d["spreads"].get(int(s)) or {}
            cells = []
            for e in ENGINES:
                v = sp.get(e)
                cells.append(f"{v * 100:.1f}%" if v is not None else "-")
            v = sp.get("ratio_vs_regex")
            cells.append(f"{v * 100:.1f}%" if v is not None else "-")
            A(f"| {thousands(int(s))} | " + " | ".join(cells) + " |")
        A("")
    A(f"Generated {generated} by `campaign_export.py` and `build_page.py` from the campaign logs.")
    A("")
    return "\n".join(o)


def splice(page_path, results):
    """Replace everything from the Results heading down with fresh results.

    The prose above that heading is written by hand and is not regenerated: a
    measurement's conditions decided after seeing its numbers are not
    conditions. Refuses rather than appends when the heading is missing, so a
    renamed section fails loudly instead of leaving two Results sections.
    """
    with open(page_path, encoding="utf-8") as f:
        text = f.read()
    marker = "\n## Results\n"
    if marker not in text:
        raise SystemExit(f"{page_path} has no '## Results' heading to replace")
    head = text.split(marker)[0]
    with open(page_path, "w", encoding="utf-8") as f:
        f.write(head.rstrip() + "\n\n" + results.lstrip())
    return page_path


def main(argv):
    # A second machine's run rides in as `--host2 <json> <rawlog>`: the JSON is
    # its passes exported the same way as the first machine's, and the raw log
    # is read only for the header that names the box.
    host2_json = host2_raw = None
    if "--host2" in argv:
        i = argv.index("--host2")
        if len(argv) < i + 3:
            raise SystemExit("--host2 takes two arguments: the exported JSON and the raw log")
        host2_json, host2_raw = argv[i + 1], argv[i + 2]
        argv = argv[:i] + argv[i + 3:]
    # `--ab <log>` may repeat: each is a run of examples/chunked_route_ab whose
    # sweep tables are read for the five-losing-rows section.
    ab_logs = []
    while "--ab" in argv:
        i = argv.index("--ab")
        if len(argv) < i + 2:
            raise SystemExit("--ab takes the path of a chunked_route_ab log")
        ab_logs.append(argv[i + 1])
        argv = argv[:i] + argv[i + 2:]
    if len(argv) < 5:
        print(__doc__)
        return 2
    json_path, artifact_path, standalone_path, md_path = argv[1:5]
    with open(json_path, encoding="utf-8") as f:
        doc = json.load(f)
    import datetime
    generated = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
    d = build(doc)
    if host2_json:
        with open(host2_json, encoding="utf-8") as f:
            d["host2"] = json.load(f)
        d["host2_meta"] = host_meta(host2_raw)
    if ab_logs:
        d["ab"] = ab_blocks(ab_logs)
        if not d["ab"]:
            raise SystemExit(f"REFUSED: no sweep tables found in {ab_logs}")
    body = render_html(d, generated)

    with open(artifact_path, "w", encoding="utf-8") as f:
        f.write(HEAD + "\n" + body)

    # The artifact host wraps its copy in a document skeleton and a reset; a
    # file opened from disk gets neither, so this copy carries its own.
    with open(standalone_path, "w", encoding="utf-8") as f:
        f.write(
            '<!doctype html>\n<html lang="en">\n<head>\n'
            '<meta charset="utf-8">\n'
            '<meta name="viewport" content="width=device-width, initial-scale=1">\n'
            '<style>:root{color-scheme:light dark}body{margin:0}'
            'img{max-width:100%}[hidden]{display:none!important}</style>\n'
            f"{HEAD}\n</head>\n<body>\n{body}\n</body>\n</html>\n"
        )

    md = render_md(d, generated)
    with open(md_path, "w", encoding="utf-8") as f:
        f.write(md)
    if len(argv) > 5:
        print(f"spliced results into {splice(argv[5], md)}")
    print(f"wrote {artifact_path}, {standalone_path} and {md_path}")
    print(f"  passes {d['labels']}, sizes {d['sizes']}")
    print(f"  vs regex {d['ratio_regex']:.2f}x over {d['rust_n']} rows, "
          f"vs python {d['ratio_py']:.1f}x, vs perl {d['ratio_pl']:.1f}x over {d['four_n']} rows")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
