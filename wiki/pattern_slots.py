#!/usr/bin/env python3
"""The TREX text in the wiki's code blocks, for the codeblock partial to color.

Every console, powershell, python and rust block under content/ is read with
its surface's grammar, and each string literal holding TREX text is a slot: a
pattern argument (pattern), a pattern-file line (decl) or a shape's
byte-pattern (bytes). The slots go to data/pattern_slots.json, keyed by the
MD5 of the block's text with carriage returns removed and trailing newlines
cut, each slot as [start, end, kind, quoting] in characters of that text. A
literal that spans lines gives one slot per line.

    python3 pattern_slots.py             write data/pattern_slots.json
    python3 pattern_slots.py --check     exit 1 when the file differs from
                                         what this run writes
    python3 pattern_slots.py --surfaces TREX MANIFEST
                                         exit 1 when CLI or CMDLETS below
                                         differ from the usage lines of the
                                         trex binary TREX or the parameters
                                         of the module MANIFEST
"""
import ast
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import tokenize

HERE = os.path.dirname(os.path.abspath(__file__))
CONTENT = os.path.join(HERE, "content")
OUT = os.path.join(HERE, "data", "pattern_slots.json")
LANGS = ("console", "powershell", "python", "rust")

# The trex subcommands with an argument holding TREX text, each with the
# metavars of its positional arguments in order and every flag that takes a
# value, as its usage line names them; the aliases the parser also accepts
# are listed beside the flag they stand for.
CLI = {
    "scan": (["PATTERN"], {
        "--text": "STRING", "--lib": "FILE|DIR", "--shape": "DECL", "--shape-after": "DECL",
        "--kind": "DECL", "--let": "DECL", "--declare": "LINE", "-e": "PATTERN", "--regexp": "PATTERN",
        "--pattern": "PATTERN", "-f": "FILE", "--file": "FILE", "--patterns": "FILE",
        "--format": "TEMPLATE", "--values": "SPELLING", "--duration-unit": "UNIT", "-m": "N",
        "--max-count": "N", "-A": "N", "--after-context": "N", "-B": "N", "--before-context": "N",
        "-C": "N|UNIT", "--context": "N|UNIT", "--color": "WHEN", "--colors": "SPEC",
        "--record": "UNIT", "--record-start": "PATTERN", "--record-span": "PATTERN",
        "--at-least": "N", "--not": "PATTERN", "--rules": "FILE|DIR", "-g": "GLOB", "--glob": "GLOB",
        "-t": "TYPE", "--type": "TYPE", "-T": "TYPE", "--type-not": "TYPE", "--texture": "KIND",
        "--sort": "KEY", "--sortr": "KEY", "--chunk-size": "N", "--head": "N", "--tail": "N",
        "--lines": "A..B",
    }),
    "rewrite": (["PATTERN", "TEMPLATE"], {
        "--text": "STRING", "--lib": "FILE|DIR", "--shape": "DECL", "--shape-after": "DECL",
        "--kind": "DECL", "--let": "DECL", "--declare": "LINE", "-C": "N", "-m": "N", "--head": "N",
        "--tail": "N", "--lines": "A..B", "--record": "UNIT",
    }),
    "redact": (["PATTERN"], {
        "--text": "STRING", "--keep": "'name:acc, ...'", "--mask": "C", "--lib": "FILE|DIR",
        "--shape": "DECL", "--shape-after": "DECL", "--kind": "DECL", "--let": "DECL",
        "--declare": "LINE", "-C": "N", "--head": "N", "--tail": "N", "--lines": "A..B",
        "--record": "UNIT",
    }),
    "templates": ([], {
        "--text": "STRING", "--record": "UNIT", "--record-start": "PATTERN",
        "--record-span": "PATTERN", "--lib": "FILE|DIR", "--shape": "'NAME = PATTERN'",
        "--shape-after": "'NAME = PATTERN'", "--kind": "'NAME = PATTERN'",
        "--let": "'NAME = PATTERN'", "--declare": "LINE", "--against": "FILE|DIR", "--cut": "N|P%",
        "--head": "N", "--tail": "N", "--lines": "A..B",
    }),
    "tokens": ([], {
        "--text": "STRING", "--lib": "FILE|DIR", "--shape": "DECL", "--shape-after": "DECL",
        "--kind": "DECL", "--let": "DECL", "--declare": "LINE",
    }),
}
for _name in ("count-by", "top", "uniq"):
    CLI[_name] = (["PATTERN", "KEY"], {
        "--text": "STRING", "--lib": "FILE|DIR", "--shape": "DECL", "--shape-after": "DECL",
        "--kind": "DECL", "--let": "DECL", "--declare": "LINE", "-n": "N", "--head": "N",
        "--tail": "N", "--lines": "A..B", "--record": "UNIT", "--patterns": "FILE",
        "--sum": "COLUMN", "--avg": "COLUMN", "--min": "COLUMN", "--max": "COLUMN",
        "--percentile": "N", "--avg-form": "FORM", "--values": "SPELLING", "--duration-unit": "UNIT",
    })
for _name, _select in (("head", "N"), ("tail", "N"), ("lines", "A..B")):
    CLI[_name] = ([_select], {
        "--record": "UNIT", "--record-start": "PATTERN", "--record-span": "PATTERN",
        "--color": "WHEN", "--colors": "SPEC",
    })

# The flags trex reads with their value from anywhere before --, ahead of the
# subcommand.
GLOBAL_FLAGS = ("--now", "--tz", "--date-order")

# The flags whose value is TREX text, by what it holds.
CLI_ROLE = {
    "-e": "pattern", "--regexp": "pattern", "--pattern": "pattern", "--not": "pattern",
    "--record-start": "pattern", "--record-span": "pattern", "--shape": "decl",
    "--shape-after": "decl", "--kind": "decl", "--let": "decl", "--declare": "decl",
}
TREX_METAVARS = ("PATTERN", "DECL", "LINE", "'NAME = PATTERN'")

# The cmdlets with a parameter holding TREX text: the parameters bound by
# position, the switch parameters, and the parameters whose value is TREX
# text, by what it holds. Each -Tx name binds as its -Trex cmdlet.
CMDLETS = {
    "Select-TrexMatch": ({0: "Pattern", 1: "InputObject"}, {
        "Binary", "Color", "Confirm", "Count", "CountMatches", "Descending", "DualGrain",
        "Explain", "FilesWithMatches", "FilesWithoutMatch", "Follow", "Hidden", "Index", "Json",
        "KeepCount", "List", "NoIgnore", "NoIndex", "NotMatch", "Passthru", "PerString", "Quiet",
        "Raw", "RequireMatch", "SingleMatch", "Stats", "WhatIf", "WholeLine"},
        {"Pattern": "pattern", "RecordStart": "pattern", "RecordSpan": "pattern"}),
    "Test-TrexMatch": ({0: "Pattern", 1: "InputObject"}, set(), {"Pattern": "pattern"}),
    "Edit-TrexText": ({0: "Pattern", 1: "Template"}, {
        "Binary", "Confirm", "Diff", "Explain", "Follow", "Hidden", "InPlace", "Interactive",
        "KeepCount", "NoIgnore", "ShowSkipped", "WhatIf"}, {"Pattern": "pattern"}),
    "Protect-TrexText": ({0: "Pattern"}, {
        "Binary", "Confirm", "Diff", "Explain", "Follow", "Hidden", "InPlace", "Interactive",
        "NoIgnore", "ShowSkipped", "WhatIf"}, {"Pattern": "pattern"}),
    "Group-TrexMatch": ({0: "Pattern", 1: "Key"}, {"Binary", "Hidden", "NoIgnore", "Unique"},
                        {"Pattern": "pattern"}),
    "Find-TrexRecord": ({0: "Pattern", 1: "InputObject"}, {"All", "Any", "None"},
                        {"Pattern": "pattern", "Not": "pattern", "RecordStart": "pattern",
                         "RecordSpan": "pattern"}),
    "ConvertFrom-TrexText": ({0: "Pattern", 1: "InputObject"}, {"MintShapes", "NoMint"},
                             {"Pattern": "pattern"}),
    "New-TrexPattern": ({0: "Pattern"}, set(), {"Pattern": "pattern"}),
    "New-TrexStreamScanner": ({0: "Pattern"}, set(), {"Pattern": "pattern"}),
    "Get-TrexRecordShape": ({0: "InputObject"}, {"Binary", "Hidden", "NoIgnore", "Novel", "Rare"},
                            {"RecordStart": "pattern", "RecordSpan": "pattern"}),
    "Register-TrexAtom": ({0: "Name", 1: "Shape"}, {"After", "Force", "PassThru"},
                          {"Shape": "bytes", "Kind": "pattern", "Pattern": "pattern"}),
    "New-TrexLibrary": ({0: "Path"}, {"FromSession"}, {"Declaration": "decl"}),
}
PS_COMMON_SWITCHES = {"Verbose", "Debug"}


class SlotError(Exception):
    """A literal the partial cannot color and write back exactly."""


# ------------------------------------------------------------ quoting

def unescape(text, quoting):
    """The value a literal's text holds under its quoting."""
    if quoting == "raw":
        return text
    if quoting == "ps-sq":
        return text.replace("''", "'")
    out, i = [], 0
    while i < len(text):
        c = text[i]
        if c == "\\" and i + 1 < len(text):
            nxt = text[i + 1]
            out.append({"n": "\n", "t": "\t"}.get(nxt, nxt) if nxt in "\\\"'nt" else c + nxt)
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out)


def escape(value, quoting):
    """The text the partial writes for a value under its quoting."""
    if quoting == "raw":
        return value
    if quoting == "ps-sq":
        return value.replace("'", "''")
    quote = '"' if quoting == "esc-dq" else "'"
    return (value.replace("\\", "\\\\").replace(quote, "\\" + quote)
            .replace("\n", "\\n").replace("\t", "\\t"))


# ------------------------------------------------------------ console

SH_OPS = ("&&", "||", "2>&1", "2>", ">>", "|", ";", "&", ">", "<", "(", ")")


def sh_words(cmd, base):
    """The words of a shell command: (start, end, value, [(start, end, quoting)])
    for a word with the span and quoting of each quoted piece, or (start, end,
    operator, None)."""
    words, i, n = [], 0, len(cmd)
    while i < n:
        c = cmd[i]
        if c in " \t\n":
            i += 1
            continue
        if c == "\\" and i + 1 < n and cmd[i + 1] == "\n":
            i += 2
            continue
        op = next((o for o in SH_OPS if cmd.startswith(o, i)), None)
        if op:
            words.append((base + i, base + i + len(op), op, None))
            i += len(op)
            continue
        if c == "#":
            j = cmd.find("\n", i)
            i = n if j < 0 else j
            continue
        start, value, quoted = i, [], []
        while i < n and cmd[i] not in " \t\n" and not any(cmd.startswith(o, i) for o in SH_OPS):
            c = cmd[i]
            if c == "'":
                j = cmd.find("'", i + 1)
                j = n if j < 0 else j
                quoted.append((base + i + 1, base + j, "raw"))
                value.append(cmd[i + 1:j])
                i = j + 1
            elif c == '"':
                j, buf = i + 1, []
                while j < n and cmd[j] != '"':
                    if cmd[j] == "\\" and j + 1 < n and cmd[j + 1] in '"\\$`\n':
                        buf.append(cmd[j + 1])
                        j += 2
                    else:
                        buf.append(cmd[j])
                        j += 1
                quoted.append((base + i + 1, base + j, "sh-dq"))
                value.append("".join(buf))
                i = j + 1
            elif c == "\\" and i + 1 < n:
                value.append(cmd[i + 1])
                i += 2
            else:
                value.append(c)
                i += 1
        words.append((base + start, base + i, "".join(value), quoted))
    return words


def lines_with_offsets(text):
    lines = text.split("\n")
    offsets, pos = [], 0
    for line in lines:
        offsets.append(pos)
        pos += len(line) + 1
    return lines, offsets


def session_commands(text, prompt):
    """(command text, offset) for each prompt line, joined with the lines after
    it while its single quotes are unbalanced, as the partial reads them; and
    the index of the line after each command."""
    lines, offsets = lines_with_offsets(text)
    i = 0
    while i < len(lines):
        line = lines[i]
        if not line.startswith(prompt):
            i += 1
            continue
        cmd, j = line[len(prompt):], i
        while cmd.count("'") % 2 == 1 and j + 1 < len(lines):
            j += 1
            cmd += "\n" + lines[j]
        yield cmd, offsets[i] + len(prompt), j + 1
        i = j + 1


def console_slots(text):
    slots = {}
    lines, offsets = lines_with_offsets(text)
    for cmd, start, after in session_commands(text, "$ "):
        if re.match(r"cat\s+\S+\.trex\s*$", cmd):
            k = after
            while k < len(lines) and not lines[k].startswith("$ "):
                if lines[k]:
                    slots[(offsets[k], offsets[k] + len(lines[k]))] = ("decl", "raw")
                k += 1
        words = sh_words(cmd, start)
        simple, cur = [], []
        for w in words:
            if w[3] is None and w[2] in ("|", ";", "&&", "||", "&", "(", ")"):
                simple.append(cur)
                cur = []
            elif w[3] is not None:
                cur.append(w)
        simple.append(cur)
        for sc in simple:
            while sc and re.match(r"[A-Za-z_]\w*=", sc[0][2]) and not sc[0][3]:
                sc = sc[1:]
            if not sc or sc[0][2] != "trex":
                continue
            rest, k = list(sc[1:]), 0
            while k < len(rest) and rest[k][2] != "--":
                flag, eq, _ = rest[k][2].partition("=")
                if flag in GLOBAL_FLAGS:
                    del rest[k:k + (1 if eq else 2)]
                    continue
                k += 1
            if not rest or rest[0][2] not in CLI:
                continue
            sub = rest[0][2]
            positionals, flags = CLI[sub]
            args = rest[1:]
            if any(a[2] in ("-e", "--regexp", "--pattern", "-f", "--file", "--patterns", "--rules",
                            "--files", "--type-list") for a in args) and sub == "scan":
                positionals = []
            if any(a[2] == "--patterns" for a in args) and sub in ("count-by", "top", "uniq"):
                positionals = ["KEY"]
            k, p, ended = 0, 0, False
            while k < len(args):
                val, quoted = args[k][2], args[k][3]
                if not ended and val == "--":
                    ended = True
                    k += 1
                    continue
                if not ended and val.startswith("-") and len(val) > 1:
                    flag, eq, _ = val.partition("=")
                    meta = flags.get(flag)
                    if meta is None or eq:
                        k += 1
                        continue
                    if k + 1 < len(args):
                        role = CLI_ROLE.get(flag) if meta in TREX_METAVARS else None
                        if role:
                            for qs, qe, quoting in args[k + 1][3]:
                                slots[(qs, qe)] = (role, quoting)
                    k += 2
                    continue
                meta = positionals[p] if p < len(positionals) else None
                p += 1
                if meta == "PATTERN":
                    for qs, qe, quoting in quoted:
                        slots[(qs, qe)] = ("pattern", quoting)
                k += 1
    return slots


# ------------------------------------------------------------ PowerShell

def ps_tokens(src, base):
    toks, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c in " \t\r":
            i += 1
            continue
        if c == "\n":
            toks.append(("nl", base + i, base + i + 1, "\n", None))
            i += 1
            continue
        if c == "`" and i + 1 < n and src[i + 1] == "\n":
            i += 2
            continue
        if c == "#":
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if src.startswith("@'\n", i) or src.startswith('@"\n', i):
            q = src[i + 1]
            j = src.find("\n" + q + "@", i + 2)
            j = n if j < 0 else j
            quoting = "raw" if q == "'" else "ps-dq"
            toks.append(("str", base + i, base + j + 3, src[i + 3:j], (base + i + 3, base + j, quoting)))
            i = j + 3
            continue
        if c == "'":
            j = i + 1
            while j < n:
                if src[j] == "'" and j + 1 < n and src[j + 1] == "'":
                    j += 2
                    continue
                if src[j] == "'":
                    break
                j += 1
            toks.append(("str", base + i, base + j + 1, src[i + 1:j], (base + i + 1, base + j, "ps-sq")))
            i = j + 1
            continue
        if c == '"':
            j = i + 1
            while j < n:
                if src[j] == "`" and j + 1 < n:
                    j += 2
                    continue
                if src[j] == '"' and j + 1 < n and src[j + 1] == '"':
                    j += 2
                    continue
                if src[j] == '"':
                    break
                j += 1
            toks.append(("str", base + i, base + j + 1, src[i + 1:j], (base + i + 1, base + j, "ps-dq")))
            i = j + 1
            continue
        if src.startswith("@(", i) or src.startswith("@{", i) or src.startswith("$(", i):
            toks.append(("open", base + i, base + i + 2, src[i:i + 2], None))
            i += 2
            continue
        if c in "({[":
            toks.append(("open", base + i, base + i + 1, c, None))
            i += 1
            continue
        if c in ")}]":
            toks.append(("close", base + i, base + i + 1, c, None))
            i += 1
            continue
        if c in "|;,":
            toks.append(("op", base + i, base + i + 1, c, None))
            i += 1
            continue
        if c == "-" and i + 1 < n and src[i + 1].isalpha():
            j = i + 1
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            name = src[i + 1:j]
            attached = j < n and src[j] == ":"
            if attached:
                j += 1
            toks.append(("param", base + i, base + j, name, attached))
            i = j
            continue
        if c == "$":
            j = i + 1
            while j < n and (src[j].isalnum() or src[j] in "_:"):
                j += 1
            toks.append(("var", base + i, base + j, src[i:j], None))
            i = j
            continue
        j = i
        while j < n and src[j] not in " \t\r\n|;,(){}[]'\"":
            j += 1
        if j == i:
            j = i + 1
        toks.append(("word", base + i, base + j, src[i:j], None))
        i = j
    return toks


def ps_group_end(toks, k):
    depth = 0
    for m in range(k, len(toks)):
        if toks[m][0] == "open":
            depth += 1
        elif toks[m][0] == "close":
            depth -= 1
            if depth == 0:
                return m
    return len(toks) - 1


def ps_args(toks, m, hi, slots, underscore):
    """The arguments of a command whose first argument token is toks[m], up to
    the end of its pipeline element: each a list of pieces (a token, or a
    group with its brackets), pieces with no space between them forming one
    argument and pieces joined by commas one array. Groups are scanned for
    commands of their own. Returns (arguments, index after the last)."""
    args, joined = [], False
    while m < hi:
        u = toks[m]
        if u[0] == "nl" or (u[0] == "op" and u[3] in "|;") or u[0] == "close":
            break
        if u[0] == "open":
            end = ps_group_end(toks, m)
            ps_scan(toks, m + 1, end, slots, underscore)
            piece, m = list(range(m, end + 1)), end + 1
        else:
            piece, m = [m], m + 1
        adjacent = args and toks[args[-1][-1][-1]][2] == toks[piece[0]][1]
        if args and (joined or adjacent):
            args[-1].append(piece)
        else:
            args.append([piece])
        joined = m < hi and toks[m][0] == "op" and toks[m][3] == ","
        if joined:
            m += 1
    return args, m


def ps_scan(toks, lo, hi, slots, underscore=None):
    """Bind the arguments of every Trex cmdlet in toks[lo:hi]. A pattern
    argument written $_ inside a ForEach-Object block marks the string
    literals piped into that ForEach-Object; underscore collects what such an
    argument holds for the caller scanning the block."""
    k, cmd_pos, element = lo, True, lo
    previous = None
    while k < hi:
        t = toks[k]
        if t[0] == "open":
            end = ps_group_end(toks, k)
            ps_scan(toks, k + 1, end, slots, underscore)
            k, cmd_pos = end + 1, False
            continue
        if t[0] == "nl" or (t[0] == "op" and t[3] in "|;") or (t[0] == "word" and t[3] == "="):
            previous = (element, k) if t[3] == "|" else None
            k, cmd_pos, element = k + 1, True, k + 1
            continue
        if cmd_pos and t[0] == "word" and t[3] in ("ForEach-Object", "%") and previous:
            piped = [toks[i] for i in range(*previous)]
            if piped and all(p[0] == "str" or (p[0] == "op" and p[3] == ",") for p in piped) \
                    and k + 1 < hi and toks[k + 1][0] == "open" and toks[k + 1][3] == "{":
                end = ps_group_end(toks, k + 1)
                roles = []
                ps_scan(toks, k + 2, end, slots, roles)
                for p in piped:
                    if p[0] == "str" and roles:
                        s, e, quoting = p[4]
                        slots[(s, e)] = (roles[0], quoting)
                k, cmd_pos = end + 1, False
                continue
        if cmd_pos and t[0] == "word" and re.match(r"^[A-Z][a-z]+-T(?:re)?x[A-Za-z]+$", t[3]):
            name = re.sub(r"-Tx(?=[A-Z])", "-Trex", t[3], count=1)
            args, m = ps_args(toks, k + 1, hi, slots, underscore)
            if name in CMDLETS:
                ps_bind(name, toks, args, slots, underscore)
            k, cmd_pos = m, False
            continue
        k, cmd_pos = k + 1, False


def ps_bind(name, toks, args, slots, underscore):
    positions, switches, roles = CMDLETS[name]
    known = set(positions.values()) | switches | set(roles)
    pos, a = 0, 0
    while a < len(args):
        pieces = args[a]
        first = toks[pieces[0][0]]
        if len(pieces) == 1 and len(pieces[0]) == 1 and first[0] == "param":
            given = first[3]
            hits = [p for p in known if p.lower() == given.lower()] or \
                   [p for p in known if p.lower().startswith(given.lower())]
            pname = hits[0] if len(hits) == 1 else given
            if pname in switches or pname in PS_COMMON_SWITCHES:
                a += 1
                continue
            if a + 1 < len(args):
                ps_mark(roles.get(pname), toks, args[a + 1], slots, underscore)
            a += 2
            continue
        ps_mark(roles.get(positions.get(pos)), toks, pieces, slots, underscore)
        pos += 1
        a += 1


def ps_mark(role, toks, pieces, slots, underscore):
    """Mark the string literals an argument holds as TREX text: a string, the
    members of an array or an @(...) group, and the arguments of a .Replace
    call on it; record the role of a $_ argument in underscore."""
    if not role:
        return
    for pi, piece in enumerate(pieces):
        first = toks[piece[0]]
        if len(piece) == 1 and first[0] == "str":
            s, e, quoting = first[4]
            slots[(s, e)] = (role, quoting)
        elif len(piece) == 1 and first[0] == "var" and first[3] in ("$_", "$PSItem"):
            if underscore is not None:
                underscore.append(role)
        elif first[0] == "open" and (first[3] == "@(" or (
                pi > 0 and toks[pieces[pi - 1][-1]][0] == "word"
                and toks[pieces[pi - 1][-1]][3].lower() == ".replace")):
            depth = 0
            for idx in piece:
                t = toks[idx]
                if t[0] == "open":
                    depth += 1
                elif t[0] == "close":
                    depth -= 1
                elif t[0] == "str" and depth == 1:
                    s, e, quoting = t[4]
                    slots[(s, e)] = (role, quoting)


def powershell_slots(text):
    slots = {}
    if any(line.startswith("PS> ") for line in text.split("\n")):
        for cmd, start, _ in session_commands(text, "PS> "):
            toks = ps_tokens(cmd, start)
            ps_scan(toks, 0, len(toks), slots)
    else:
        toks = ps_tokens(text, 0)
        ps_scan(toks, 0, len(toks), slots)
    return slots


# ------------------------------------------------------------ Python

def dotted(node):
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        inner = dotted(node.value)
        return f"{inner}.{node.attr}" if inner else f"?.{node.attr}"
    if isinstance(node, ast.Call):
        inner = dotted(node.func)
        return f"{inner}()" if inner else "?()"
    return "?"


def py_arg(call, i, kw):
    if i is not None and len(call.args) > i:
        return call.args[i]
    for k in call.keywords:
        if k.arg == kw:
            return k.value
    return None


def py_slot_exprs(call, helpers):
    """(expression, kind) for each argument of this call holding TREX text."""
    name = dotted(call.func)
    last = name.rsplit(".", 1)[-1]
    method = isinstance(call.func, ast.Attribute)
    found = []
    if name in ("trex.Pattern", "Pattern", "trex.parse"):
        found.append((py_arg(call, 0, "source"), "pattern"))
    elif name in ("trex.PatternSet", "PatternSet"):
        found.append((py_arg(call, 0, "patterns"), "pattern"))
    elif name == "trex.query":
        found += [(py_arg(call, 0, "patterns"), "pattern")] + \
                 [(py_arg(call, None, kw), "pattern") for kw in ("exclude", "record_start", "record_span")]
    elif name == "trex.templates":
        found += [(py_arg(call, None, kw), "pattern") for kw in ("record_start", "record_span")]
    elif name in ("trex.follow", "trex.follow_rewrite", "trex.follow_redact"):
        found.append((py_arg(call, 0, "pattern"), "pattern"))
    elif method and last == "candidates":
        found.append((py_arg(call, 0, "pattern"), "pattern"))
    elif method and last in ("kind", "let") and (len(call.args) >= 2 or py_arg(call, None, "pattern")):
        found.append((py_arg(call, 1, "pattern"), "pattern"))
    elif method and last == "shape" and not name.startswith("trex.axes") and \
            (len(call.args) >= 2 or py_arg(call, None, "bytepat")):
        found.append((py_arg(call, 1, "bytepat"), "bytes"))
    elif method and last == "declare":
        found.append((py_arg(call, 0, "line"), "decl"))
    elif name in helpers:
        found.append((py_arg(call, helpers[name], None), "pattern"))
    elif method and last == "write" and isinstance(call.func.value, ast.Call) \
            and dotted(call.func.value.func) == "open" and call.func.value.args \
            and isinstance(call.func.value.args[0], ast.Constant) \
            and str(call.func.value.args[0].value).endswith(".trex"):
        found.append((py_arg(call, 0, None), "decl"))
    return [(e, kind) for e, kind in found if e is not None]


def py_strings(e):
    """The string constants an argument contributes as TREX text: itself, the
    members of a list or tuple (the second of a (name, pattern) pair), the
    operands of +, and the arguments of a .replace on the text."""
    if isinstance(e, ast.Constant) and isinstance(e.value, str):
        return [e]
    if isinstance(e, (ast.List, ast.Tuple)):
        out = []
        for x in e.elts:
            out += py_strings(x.elts[1]) if isinstance(x, ast.Tuple) and len(x.elts) == 2 else py_strings(x)
        return out
    if isinstance(e, ast.Call) and isinstance(e.func, ast.Attribute) and e.func.attr == "replace":
        return py_strings(e.func.value) + [a for a in e.args if isinstance(a, ast.Constant) and isinstance(a.value, str)]
    if isinstance(e, ast.BinOp) and isinstance(e.op, ast.Add):
        return py_strings(e.left) + py_strings(e.right)
    return []


def py_iterated(name, node, parents):
    """The string constants a for loop or comprehension enclosing node binds
    to name by iterating a literal list or tuple."""
    anc = parents.get(node)
    while anc is not None:
        gens = []
        if isinstance(anc, ast.For):
            gens = [anc]
        elif isinstance(anc, (ast.ListComp, ast.SetComp, ast.GeneratorExp, ast.DictComp)):
            gens = anc.generators
        for g in gens:
            if isinstance(g.target, ast.Name) and g.target.id == name and isinstance(g.iter, (ast.List, ast.Tuple)):
                return py_strings(g.iter)
        anc = parents.get(anc)
    return []


def py_find_helpers(tree, helpers):
    """NAME -> index for a lambda or def that hands that parameter to a call
    taking a pattern."""
    for node in ast.walk(tree):
        fn, name = None, None
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name) \
                and isinstance(node.value, ast.Lambda):
            fn, name = node.value, node.targets[0].id
        elif isinstance(node, ast.FunctionDef):
            fn, name = node, node.name
        if fn is None:
            continue
        params = [a.arg for a in fn.args.args]
        for inner in ast.walk(fn):
            if isinstance(inner, ast.Call):
                for e, kind in py_slot_exprs(inner, {}):
                    if kind == "pattern" and isinstance(e, ast.Name) and e.id in params:
                        helpers[name] = params.index(e.id)


def python_units(text):
    """(source, offset of each source line) for each unit ast reads: the block,
    or each >>> command with its ... lines."""
    lines, offsets = lines_with_offsets(text)
    if not any(line.startswith(">>> ") for line in lines):
        return [("\n".join(lines), offsets)]
    units, cur, cur_off = [], None, None
    for line, off in zip(lines, offsets):
        if line.startswith(">>> ") or line == ">>>":
            if cur is not None:
                units.append(("\n".join(cur), cur_off))
            cur, cur_off = [line[4:]], [off + 4]
        elif (line.startswith("... ") or line == "...") and cur is not None:
            cur.append(line[4:])
            cur_off.append(off + 4)
        elif cur is not None:
            units.append(("\n".join(cur), cur_off))
            cur, cur_off = None, None
    if cur is not None:
        units.append(("\n".join(cur), cur_off))
    return units


def python_slots(text, helpers, where):
    slots = {}
    for src, line_off in python_units(text):
        try:
            tree = ast.parse(src)
        except SyntaxError as e:
            raise SlotError(f"{where}: ast cannot read {src[:60]!r}: {e.msg}")
        src_lines = src.split("\n")
        literals = []
        for tok in tokenize.generate_tokens(io.StringIO(src).readline):
            if tok.type == tokenize.STRING:
                body = tok.string
                prefix = re.match(r"[A-Za-z]*", body).group(0)
                q = 3 if body[len(prefix):len(prefix) + 3] in ('"""', "'''") else 1
                quote = body[len(prefix)]
                if "f" in prefix.lower():
                    quoting = None
                elif "r" in prefix.lower():
                    quoting = "raw"
                else:
                    quoting = "esc-dq" if quote == '"' else "esc-sq"
                s = line_off[tok.start[0] - 1] + tok.start[1] + len(prefix) + q
                e = line_off[tok.end[0] - 1] + tok.end[1] - q
                literals.append((tok.start, tok.end, s, e, quoting))
        py_find_helpers(tree, helpers)
        parents = {child: node for node in ast.walk(tree) for child in ast.iter_child_nodes(node)}
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call):
                continue
            for expr, kind in py_slot_exprs(node, helpers):
                held = py_strings(expr)
                if not held and isinstance(expr, ast.Name):
                    held = py_iterated(expr.id, node, parents)
                for c in held:
                    col = len(src_lines[c.lineno - 1].encode("utf-8")[:c.col_offset].decode("utf-8"))
                    ecol = len(src_lines[c.end_lineno - 1].encode("utf-8")[:c.end_col_offset].decode("utf-8"))
                    for ts, te, s, e, quoting in literals:
                        if ts >= (c.lineno, col) and te <= (c.end_lineno, ecol):
                            if quoting is None:
                                raise SlotError(f"{where}: an f-string holds TREX text: {text[s:e]!r}")
                            slots[(s, e)] = (kind, quoting)
    return slots


# ------------------------------------------------------------ Rust

RUST_RAW = re.compile(r'b?r(#*)"')
RUST_CHAR = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'])'")


def rust_tokens(src):
    """Tokens (kind, start, end, text, (interior start, end, quoting) for a string)."""
    toks, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c.isspace():
            i += 1
            continue
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if src.startswith("/*", i):
            j = src.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        boundary = i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")
        m = RUST_RAW.match(src, i)
        if m and boundary:
            close = '"' + m.group(1)
            j = src.find(close, m.end())
            j = n if j < 0 else j
            toks.append(("str", i, j + len(close), src[m.end():j], (m.end(), j, "raw")))
            i = j + len(close)
            continue
        if c == '"' or (src.startswith('b"', i) and boundary):
            k = i + (2 if c == "b" else 1)
            j = k
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            toks.append(("str", i, j + 1, src[k:j], (k, j, "esc-dq")))
            i = j + 1
            continue
        if c == "'":
            m = RUST_CHAR.match(src, i)
            if m:
                toks.append(("char", i, m.end(), m.group(0), None))
                i = m.end()
                continue
            j = i + 1
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            toks.append(("lifetime", i, j, src[i:j], None))
            i = j
            continue
        if c.isalpha() or c == "_":
            j = i
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            toks.append(("ident", i, j, src[i:j], None))
            i = j
            continue
        if c.isdigit():
            j = i
            while j < n and (src[j].isalnum() or src[j] in "._"):
                if src[j] == "." and j + 1 < n and src[j + 1] == ".":
                    break
                j += 1
            toks.append(("num", i, j, src[i:j], None))
            i = j
            continue
        if src.startswith("::", i):
            toks.append(("punct", i, i + 2, "::", None))
            i += 2
            continue
        toks.append(("punct", i, i + 1, c, None))
        i += 1
    return toks


def rust_calls(toks):
    """(path, is_method, [argument token indexes], index of the open paren)."""
    calls = []
    for k, t in enumerate(toks):
        if t[3] != "(" or k == 0 or toks[k - 1][0] != "ident":
            continue
        p = k - 1
        path, j = [toks[p][3]], p
        while j >= 2 and toks[j - 1][3] == "::" and toks[j - 2][0] == "ident":
            path.insert(0, toks[j - 2][3])
            j -= 2
        method = j >= 1 and toks[j - 1][3] == "."
        depth, args, cur = 0, [], []
        for m in range(k + 1, len(toks)):
            tt = toks[m]
            if tt[0] == "punct" and tt[3] in "([{":
                depth += 1
            elif tt[0] == "punct" and tt[3] in ")]}":
                if depth == 0:
                    break
                depth -= 1
            if tt[0] == "punct" and tt[3] == "," and depth == 0:
                args.append(cur)
                cur = []
                continue
            cur.append(m)
        if cur:
            args.append(cur)
        calls.append((path, method, args, k))
    return calls


def rust_slot(path, method, imports_parse):
    """(argument index, kind) where this call takes TREX text, or None."""
    last = path[-1]
    if method:
        return (0, "decl") if last in ("declare", "declare_text", "declare_kind", "declare_let") else None
    if last == "parse" and ((len(path) >= 2 and path[-2] in ("trex", "parser")) or (len(path) == 1 and imports_parse)):
        return 0, "pattern"
    if last in ("parse_with_shapes", "parse_with_inputs"):
        return 0, "pattern"
    return None


def rust_arg_strings(toks, arg, calls_by_open):
    """The string tokens an argument holds as TREX text: a literal, a borrowed
    literal, or the literal arguments of a .replace call inside it."""
    real = [m for m in arg if not (toks[m][0] == "punct" and toks[m][3] == "&")]
    if len(real) == 1 and toks[real[0]][0] == "str":
        return [real[0]]
    out = []
    for m in arg:
        call = calls_by_open.get(m)
        if call and call[1] and call[0][-1] == "replace":
            for a in call[2]:
                out += [x for x in a if toks[x][0] == "str"]
    return out


def rust_helpers(toks, calls):
    """NAME -> index for a closure bound by let, or an fn, that hands that
    parameter to a call taking a pattern."""
    helpers = {}
    for k in range(len(toks) - 4):
        name, params = None, []
        if toks[k][3] == "let" and toks[k + 1][0] == "ident" and toks[k + 2][3] == "=" and toks[k + 3][3] == "|":
            name, m = toks[k + 1][3], k + 4
            while m < len(toks) and toks[m][3] != "|":
                if toks[m][0] == "ident" and toks[m - 1][3] in ("|", ","):
                    params.append(toks[m][3])
                m += 1
            depth, end = 0, len(toks)
            for q in range(m + 1, len(toks)):
                if toks[q][0] != "punct":
                    continue
                if toks[q][3] in "([{":
                    depth += 1
                elif toks[q][3] in ")]}":
                    depth -= 1
                elif toks[q][3] == ";" and depth == 0:
                    end = q
                    break
        elif toks[k][3] == "fn" and toks[k + 1][0] == "ident" and toks[k + 2][3] == "(":
            name, m = toks[k + 1][3], k + 3
            while m < len(toks) and toks[m][3] != ")":
                if toks[m][0] == "ident" and toks[m - 1][3] in ("(", ","):
                    params.append(toks[m][3])
                m += 1
            opening = next((q for q in range(m, len(toks)) if toks[q][3] == "{"), len(toks) - 1)
            depth, end = 0, len(toks)
            for q in range(opening, len(toks)):
                if toks[q][0] == "punct" and toks[q][3] == "{":
                    depth += 1
                elif toks[q][0] == "punct" and toks[q][3] == "}":
                    depth -= 1
                    if depth == 0:
                        end = q
                        break
        else:
            continue
        for path, method, args, open_k in calls:
            if not (m < open_k < end):
                continue
            slot = rust_slot(path, method, True)
            if slot and slot[1] == "pattern" and len(args) > slot[0]:
                real = [toks[x][3] for x in args[slot[0]] if toks[x][3] != "&"]
                if len(real) == 1 and real[0] in params:
                    helpers.setdefault(name, params.index(real[0]))
                    break
    return helpers


def rust_bound_literal(toks, name, before):
    """The string token the last `let NAME = "...";` before toks[before] binds."""
    found = None
    for k in range(before):
        if toks[k][3] != "let":
            continue
        j = k + 1
        if j < before and toks[j][3] == "mut":
            j += 1
        if j >= before or toks[j][3] != name:
            continue
        while j < before and toks[j][3] not in ("=", ";"):
            j += 1
        if j + 2 < len(toks) and toks[j][3] == "=" and toks[j + 1][0] == "str" and toks[j + 2][3] == ";":
            found = j + 1
    return found


def rust_slots(text):
    slots = {}
    toks = rust_tokens(text)
    calls = rust_calls(toks)
    by_open = {c[3]: c for c in calls}
    imports_parse = bool(re.search(r"use trex::(?:\{[^}]*\bparse\b[^}]*\}|parse\b)", text))
    helpers = rust_helpers(toks, calls)
    for path, method, args, open_k in calls:
        slot = rust_slot(path, method, imports_parse)
        if slot is None and not method and len(path) == 1 and path[0] in helpers:
            slot = (helpers[path[0]], "pattern")
        if slot is None and "::".join(path).endswith("fs::write") and len(args) >= 2 and \
                any(toks[x][0] == "str" and toks[x][3].endswith(".trex") for x in args[0]):
            slot = (1, "decl")
        if slot is None or len(args) <= slot[0]:
            continue
        held = rust_arg_strings(toks, args[slot[0]], by_open)
        real = [x for x in args[slot[0]] if toks[x][3] != "&"]
        if not held and len(real) == 1 and toks[real[0]][0] == "ident":
            bound = rust_bound_literal(toks, toks[real[0]][3], open_k)
            held = [bound] if bound is not None else []
        for x in held:
            s, e, quoting = toks[x][4]
            slots[(s, e)] = (slot[1], quoting)
    return slots


# ------------------------------------------------------------ blocks

def blocks(path):
    """(language, text) for each fenced code block, the text as the codeblock
    partial receives it: the lines between the fences with the fence's
    indentation removed, carriage returns dropped."""
    with open(path, encoding="utf-8") as f:
        lines = f.read().replace("\r", "").split("\n")
    i = 0
    while i < len(lines):
        m = re.match(r"^( {0,3})(`{3,}|~{3,})\s*([^`\s{]*)", lines[i])
        if not m:
            i += 1
            continue
        indent, fence, info = len(m.group(1)), m.group(2), m.group(3)
        body, j = [], i + 1
        while j < len(lines) and not re.match(r"^ {0,3}" + re.escape(fence[0]) + "{" + str(len(fence)) + r",}\s*$", lines[j]):
            line = lines[j]
            strip = len(line) - len(line.lstrip(" "))
            body.append(line[min(strip, indent):])
            j += 1
        yield info.split(",")[0].lower(), "\n".join(body)
        i = j + 1


def segments(text, start, end, kind, quoting, where, sessions):
    """The slot as one entry per line of its literal, each checked so the
    partial can mask it, lex it and write it back exactly."""
    piece = text[start:end]
    if quoting == "sh-dq" and ("$" in piece or "`" in piece or re.search(r'\\["\\$`\n]', piece)):
        raise SlotError(f"{where}: TREX text in a shell double-quoted string that expands: {piece!r}")
    if quoting == "ps-dq" and ("$" in piece or "`" in piece or '""' in piece):
        raise SlotError(f"{where}: TREX text in a PowerShell double-quoted string that expands: {piece!r}")
    if quoting in ("sh-dq", "ps-dq"):
        quoting = "raw"
    if escape(unescape(piece, quoting), quoting) != piece:
        raise SlotError(f"{where}: {quoting} text that does not write back exactly: {piece!r}")
    if sessions and piece.count("'") % 2:
        raise SlotError(f"{where}: TREX text with an odd count of single quotes: {piece!r}")
    out, pos = [], start
    for line in piece.split("\n"):
        if line:
            out.append([pos, pos + len(line), kind, quoting])
        pos += len(line) + 1
    return out


def collect():
    data, errors, pages = {}, [], {}
    for root, _, files in os.walk(CONTENT):
        for name in sorted(files):
            if not name.endswith(".md"):
                continue
            path = os.path.join(root, name)
            rel = os.path.relpath(path, HERE).replace("\\", "/")
            py_helpers = {}
            for lang, text in blocks(path):
                if lang not in LANGS:
                    continue
                key = hashlib.md5(text.rstrip("\n").encode("utf-8")).hexdigest()
                where = f"{rel}: {lang} block {text.split(chr(10), 1)[0][:60]!r}"
                try:
                    if lang == "console":
                        found = console_slots(text)
                    elif lang == "powershell":
                        found = powershell_slots(text)
                    elif lang == "python":
                        found = python_slots(text, py_helpers, where)
                    else:
                        found = rust_slots(text)
                    sessions = lang != "rust" and any(
                        line.startswith(p) for line in text.split("\n") for p in ("$ ", "PS> ", ">>> "))
                    slots = []
                    for (s, e), (kind, quoting) in sorted(found.items()):
                        if e > s:
                            slots += segments(text, s, e, kind, quoting, where, sessions)
                except SlotError as err:
                    errors.append(str(err))
                    continue
                if key in data and data[key] != slots:
                    errors.append(f"{where}: the same text as a block in {pages[key]} gives other slots")
                    continue
                data[key], pages[key] = slots, rel
    return data, errors


def render(data):
    lines = [f"  {json.dumps(k)}: {json.dumps(v, separators=(',', ':'))}" for k, v in sorted(data.items())]
    return "{\n" + ",\n".join(lines) + "\n}\n"


# ------------------------------------------------------------ surfaces

FLAG_META = re.compile(
    r"((?:-{1,2}[A-Za-z][\w-]*\|)*-{1,2}[A-Za-z][\w-]*)(?:\[=\w+\])?\s+"
    r"('[^']*'|[A-Z][A-Z0-9_.]*(?:\|[A-Z][A-Z0-9_.%]*)*)"
)


def check_surfaces(trex, manifest):
    """Where the usage lines and the module's parameters name TREX text that
    CLI or CMDLETS do not hold, or bind otherwise."""
    problems = []
    listing = subprocess.run([trex, "--help"], capture_output=True, text=True, encoding="utf-8")
    subs = sorted(set(re.findall(r"^\s+trex ([a-z][a-z-]*)", listing.stdout + listing.stderr, re.M)) | set(CLI))
    for sub in subs:
        r = subprocess.run([trex, sub, "--help"], capture_output=True, text=True, encoding="utf-8")
        usage = [l for l in (r.stdout + r.stderr).splitlines()
                 if re.match(r"\s*(?:usage: )?trex " + re.escape(sub) + r"\b", l)]
        for line in usage:
            rest = line.split("trex " + sub, 1)[1]
            head = re.split(r"\s[\[(]", " " + rest, maxsplit=1)[0].split()
            heads = [t.rstrip(".") for t in head if not t.startswith("-")]
            if "PATTERN" in heads and (sub not in CLI or "PATTERN" not in CLI[sub][0]):
                problems.append(f"trex {sub} takes a PATTERN argument that CLI does not hold")
            for fm in FLAG_META.finditer(rest):
                chain, meta = fm.group(1).split("|"), fm.group(2)
                for idx, flag in enumerate(chain):
                    if meta in TREX_METAVARS:
                        if not CLI_ROLE.get(flag) or sub not in CLI:
                            problems.append(f"trex {sub} {flag} {meta} holds TREX text that CLI does not hold")
                        elif CLI[sub][1].get(flag) != meta:
                            problems.append(f"trex {sub} {flag} {meta}: CLI has {CLI[sub][1].get(flag)}")
                    elif idx == len(chain) - 1 and sub in CLI and CLI[sub][1].get(flag) != meta:
                        problems.append(f"trex {sub} {flag} {meta}: CLI has {CLI[sub][1].get(flag)}")
    script = (
        "Import-Module '" + manifest.replace("'", "''") + "' -Force; "
        "foreach ($c in Get-Command -Module Trex -CommandType Cmdlet,Function) { "
        "foreach ($s in $c.ParameterSets) { foreach ($p in $s.Parameters) { "
        "'{0}|{1}|{2}|{3}' -f $c.Name, $p.Name, $p.ParameterType.Name, $p.Position } } }"
    )
    r = subprocess.run(["pwsh", "-NoProfile", "-Command", script],
                       capture_output=True, text=True, encoding="utf-8")
    seen = {}
    for line in r.stdout.splitlines():
        parts = line.strip().split("|")
        if len(parts) == 4:
            seen.setdefault(parts[0], []).append((parts[1], parts[2], int(parts[3])))
    if not seen:
        problems.append(f"no parameters read from {manifest}: {r.stderr.strip()[:200]}")
    for cmdlet, params in seen.items():
        positions, switches, roles = CMDLETS.get(cmdlet, ({}, set(), {}))
        names = {p for p, _, _ in params}
        for trex_param in ("Pattern", "Not", "RecordStart", "RecordSpan", "Declaration"):
            if trex_param in names and trex_param not in roles:
                problems.append(f"{cmdlet} -{trex_param} holds TREX text and CMDLETS does not say so")
        if cmdlet not in CMDLETS:
            continue
        for pname, ptype, pos in params:
            if ptype == "SwitchParameter" and pname not in switches and pname not in PS_COMMON_SWITCHES \
                    and pname not in ("Verbose", "Debug"):
                problems.append(f"{cmdlet} -{pname} is a switch that CMDLETS does not list")
            if pos >= 0 and positions.get(pos) != pname:
                problems.append(f"{cmdlet} binds -{pname} at position {pos}; CMDLETS has {positions.get(pos)}")
    return sorted(set(problems))


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--surfaces":
        problems = check_surfaces(sys.argv[2], sys.argv[3])
        for p in problems:
            print(p)
        print(f"{len(problems)} differences between the tables and the surfaces")
        return 1 if problems else 0
    data, errors = collect()
    for e in errors:
        print("ERROR", e)
    if errors:
        return 1
    text = render(data)
    count = sum(len(v) for v in data.values())
    if len(sys.argv) == 2 and sys.argv[1] == "--check":
        current = open(OUT, encoding="utf-8").read() if os.path.exists(OUT) else ""
        if current != text:
            print(f"{os.path.relpath(OUT, HERE)} is stale: run python3 pattern_slots.py")
            return 1
        print(f"{len(data)} blocks, {count} slots, {os.path.relpath(OUT, HERE)} is current")
        return 0
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"{len(data)} blocks, {count} slots written to {os.path.relpath(OUT, HERE)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
