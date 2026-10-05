"""The trex module through its public surface: typed tokens and binding,
offsets in the input's units, rewrites, splits, sets, streams and errors."""

import ast
import datetime
import inspect
import json
import os
from decimal import Decimal
from fractions import Fraction
from pathlib import Path

import pytest

import trex
import trex.axes


def test_a_typed_register_reports_its_value_in_python_types():
    # A byte size is its number of bytes.
    assert trex.Pattern(r"\Z:s").find("size 4KB").value("s") == 4000
    assert trex.Pattern(r"\Z:s").find("size 4KiB").value("s") == 4096
    # A duration is held and compared as nanoseconds, so that is the default;
    # ms and s shift it exactly, never through a float.
    assert trex.Pattern(r"\R:d").find("took 90s").value("d") == 90000000000
    assert trex.Pattern(r"\R:d").find("took 90s").value("d", unit="s") == 90
    assert trex.Pattern(r"\R:d").find("took 90s").value("d", unit="ms") == 90000
    assert trex.Pattern(r"\R:d").find("took 1500ms").value("d", unit="s") == Decimal("1.5")
    assert trex.Pattern(r"\R:d").find("took 2d").value("d", unit="s") == 172800
    with pytest.raises(ValueError):
        trex.Pattern(r"\R:d").find("took 90s").value("d", unit="fortnight")

    # An address as an int of its bits, so v4 and v6 read the same way and
    # neither overflows: a Python int is arbitrary precision.
    assert trex.Pattern(r"\I:h").find("from 192.168.1.7").value("h") == 3232235783

    # A timestamp as a timezone-aware datetime in UTC.
    when = trex.Pattern(r"\T:t").find("at 2026-09-15T00:00:00Z").value("t")
    assert when == datetime.datetime(2026, 9, 15, tzinfo=datetime.timezone.utc)
    assert when.tzinfo is not None

    # A version as the parts and the pre-release identifiers that decide
    # semver order; build metadata takes no part in it and is absent.
    ver = trex.Pattern(r"\V:v").find("v1.2.3-beta.2+build.7").value("v")
    assert ver == ((1, 2, 3), ("beta", "2"))

    # Decimal, not float, wherever the parser is exact. This is the whole
    # reason the exact spelling is the default: a float would round these,
    # and the digits past the point are usually why someone matched on them.
    price = trex.Pattern(r"\$:p").find("paid $12.50").value("p")
    assert price == Decimal("12.5")
    assert isinstance(price, Decimal)
    rate = trex.Pattern(r"\%:r").find("at 99.95%").value("r")
    assert rate == Decimal("99.95")
    big = trex.Pattern(r"\N:n").find("n 9007199254740993").value("n")
    assert big == 9007199254740993

    # A register that binds no single typed kind has no one value to report,
    # and a register the pattern does not bind is a KeyError, which is a
    # different answer from None.
    assert trex.Pattern(r"\W:w").find("hello").value("w") is None
    with pytest.raises(KeyError):
        trex.Pattern(r"\W:w").find("hello").value("nosuch")

    # A register bound under a repetition answers one value per binding,
    # matching what captures holds for it.
    m = trex.Pattern(r"(\Z:s \W)+").find("4KB a 8KB b")
    assert m.value("s") == [4000, 8000]


def test_the_module_reads_a_log_rather_than_standing_in_for_re():
    log = (
        "GET /a 200 12ms\n"
        "GET /b 404 30ms\n"
        "POST /c 200 9ms\n"
    )

    # Templates: the lines that share a shape, most lines first, each with a
    # readable form of the line and the pattern that matches it.
    ts = trex.templates(log)
    assert ts, "a log of three lines has at least one template"
    assert sum(t["count"] for t in ts) == ts[0]["covered"]
    assert all(set(t) == {"count", "records", "readable", "pattern", "rare", "novel", "covered"} for t in ts)
    # A cut the caller names decides what counts as rare; absent, it is the
    # mean template's coverage, read from the input rather than chosen.
    rare = trex.templates(log, cut="1")
    assert all(t["rare"] is False for t in rare), "nothing covers fewer than one line"

    # Infer: a pattern from examples, and anchored on request.
    src = trex.infer([b"GET /a 200", b"GET /b 404"])
    assert trex.Pattern(src).is_match("GET /a 200"), src
    anchored = trex.infer([b"GET /a 200", b"GET /b 404"], anchored=True)
    assert anchored != src

    # A value range is printed only where a counter-example rules it in: with
    # nothing to tell the examples from, a position reports its bare kind
    # however well the values agree.
    bare = trex.infer([b"code 200", b"code 204"])
    assert bare == '"code" \\N', bare
    banded = trex.infer([b"code 200", b"code 204"], against=[b"code 500"])
    assert banded == '"code" \\N{200..299}', banded
    assert trex.Pattern(banded).is_match("code 200")
    assert not trex.Pattern(banded).is_match("code 500")
    # A counter-example nothing separates is an error naming it, not a
    # pattern that matches what it was told to miss.
    with pytest.raises(ValueError):
        trex.infer([b"code 200", b"code 204"], against=[b"code 201"])

    # Records: the spans of a unit, in the input's units.
    spans = trex.records(log, "line")
    assert len(spans) == 3
    assert log[spans[0][0]:spans[0][1]].startswith("GET /a")

    # count_by: the rows a table would print, by key or by count. The verb is
    # anchored to the line, because `\W` alone would also bind the path's own
    # word and count it as a verb.
    p = trex.Pattern(r"^ \W:verb \P \W \N:code")
    by_key = p.count_by("${verb}", log)
    assert by_key == [("GET", 2), ("POST", 1)]
    by_count = p.count_by("${verb}", log, order="count")
    assert by_count[0] == ("GET", 2)
    with pytest.raises(ValueError):
        p.count_by("${verb}", log, order="alphabetical")

    # explain: why a match matched, as a dict. It takes the input again, since
    # keeping the input on every match would copy all of it on each scan.
    m = p.find(log)
    why = m.explain(log)
    assert {"tokens", "guards", "readings", "route"} <= set(why)
    assert any(kind == "word" for kind, _ in why["tokens"])
    assert isinstance(why["route"], str) and why["route"]


def test_explain_reports_the_rung_that_answered_and_reads_the_declarations(tmp_path):
    # The rungs `trex scan --explain` names for the same two scans.
    card = "pay 4111 1111 1111 1111 now"
    assert trex.Pattern(r"\{card}").find(card).explain(card)["route"] == "a route, not the engine"
    sizes = "sizes 12 15 9 4000"
    assert trex.Pattern(r"\N{>+1}").find(sizes).explain(sizes)["route"] == "the set engine over a whole lex"
    with pytest.raises(TypeError):
        trex.Pattern(r"\{card}").find(card).explain(card, route="scan")
    # A pattern read under lib= is explained with its declarations, so a
    # declared shape is the kind of the token it lexed.
    lib = tmp_path / "ticket.trex"
    lib.write_text("shape ticket = `[A-Z]{2,4}-\\d{1,4}`\n", encoding="utf-8")
    ticket = "see AB-12 now"
    why = trex.Pattern(r"\{ticket}", lib=str(lib)).find(ticket).explain(ticket)
    assert why["tokens"] == [("ticket", "AB-12")]


def test_typed_tokens_bind_and_back_reference():
    p = trex.Pattern(r"<\W:t>.*</=t>")
    m = p.find("say <div>hi</div> now")
    assert m is not None
    assert (m.start, m.end) == (4, 17)
    assert m.span() == (4, 17)
    assert m.text == "<div>hi</div>"
    assert m.captures == {"t": "div"}
    assert m["t"] == "div" and m.group("t") == "div"
    assert m.capture_span("t") == (5, 8)
    assert p.capture_names() == ["t"]
    assert p.find("<div>hi</span>") is None
    assert p.is_match("<b>x</b>") and not p.is_match("<b>x</i>")
    assert repr(p) == 'Pattern("<\\\\W:t>.*</=t>")'
    assert repr(m) == "Match(start=4, end=17, text='<div>hi</div>')"


def test_str_offsets_are_characters_and_bytes_offsets_are_bytes():
    p = trex.Pattern(r"\N")
    s = "héllo 42 wörld 7"
    ms = p.scan(s)
    assert [(m.start, m.end, m.text) for m in ms] == [(6, 8, "42"), (15, 16, "7")]
    assert [(m.byte_start, m.byte_end) for m in ms] == [(7, 9), (17, 18)]
    assert all(s[m.start : m.end] == m.text for m in ms)
    bs = p.scan(s.encode())
    assert [(m.start, m.end, m.text) for m in bs] == [(7, 9, b"42"), (17, 18, b"7")]
    assert [m.byte_span() for m in bs] == [(7, 9), (17, 18)]
    bound = trex.Pattern(r"\W:w \N:n").find("naïve über 12")
    assert bound.captures == {"n": "12", "w": "über"}
    assert bound.capture_span("w") == (6, 10)
    assert bound.capture_byte_span("w") == (7, 12)


def test_iteration_rewrite_and_split():
    p = trex.Pattern(r"\E:e")
    text = "mail bob@x.com and amy@y.org now"
    it = p.find_iter(text)
    assert len(it) == 2
    assert next(it)["e"] == "bob@x.com"
    assert [m["e"] for m in it] == ["amy@y.org"]
    assert [m.text for m in p.captures_iter(text)] == ["bob@x.com", "amy@y.org"]
    assert p.captures(text).text == "bob@x.com"
    assert p.rewrite("[${e:domain}]", text) == "mail [x.com] and [y.org] now"
    assert p.rewrite_first("<x>", text) == "mail <x> and amy@y.org now"
    assert p.rewrite_n("<x>", text, 1) == p.rewrite_first("<x>", text)
    assert p.rewrite("<x>", text.encode()) == b"mail <x> and <x> now"
    punct = trex.Pattern(r"\P")
    assert punct.split("a, b; c") == ["a", " b", " c"]
    assert punct.splitn("a, b; c", 2) == ["a", " b; c"]
    assert punct.split(b"a, b") == [b"a", b" b"]


def test_a_callable_replacement_reads_the_match_and_its_typed_slices():
    p = trex.Pattern(r"\E:e")
    text = "mail bob@x.com and amy@y.org now"
    seen = []

    def repl(m):
        seen.append((m.start, m.end, m["e"], m["e:domain"], m["0:user"], m["1:upper"]))
        return m["e:user"].upper()

    assert p.rewrite(repl, text) == "mail BOB and AMY now"
    assert seen == [
        (5, 14, "bob@x.com", "x.com", "bob", "BOB@X.COM"),
        (19, 28, "amy@y.org", "y.org", "amy", "AMY@Y.ORG"),
    ]
    assert p.rewrite_first(lambda m: "<x>", text) == "mail <x> and amy@y.org now"
    assert p.rewrite_n(lambda m: m["e:domain"], text, 1) == "mail x.com and amy@y.org now"
    assert p.rewrite(lambda m: m["e:domain"], text.encode()) == b"mail x.com and y.org now"
    m = trex.Pattern(r"\W:w \I:ip").find("from 10.1.2.3")
    assert (m["ip:octet1-2"], m["0:last1"], m["1"], m["2:octet4"], m.group("w:upper")) == (
        "10.1",
        "3",
        "from",
        "3",
        "FROM",
    )
    assert trex.Pattern(r"\N").find(b"x 42")["0:first1"] == b"4"
    with pytest.raises(TypeError, match="must be a str"):
        p.rewrite(lambda m: 1, text)
    with pytest.raises(TypeError, match="must be bytes"):
        p.rewrite(lambda m: "s", text.encode())
    with pytest.raises(TypeError, match="template str or a callable"):
        p.rewrite(3, text)
    with pytest.raises(KeyError):
        p.find(text)["nope:domain"]
    with pytest.raises(ValueError, match="unknown accessor"):
        p.find(text)["e:bogus"]
    with pytest.raises(ValueError, match="binds 1 capture"):
        p.find(text)["2"]


def test_nested_and_repeated_captures():
    m = trex.Pattern(r'(\W:k "=" \N:v):pair').find("x = 1")
    assert (m["pair"], m["pair.k"], m["pair.v"], m["pair.k:upper"]) == ("x = 1", "x", "1", "X")
    assert m.captures == {"pair": "x = 1", "pair.k": "x", "pair.v": "1"}
    m = trex.Pattern(r'((\W:k "=" \N:v):pair ";")+').find("a = 1; b = 2;")
    assert m.captures["pair"] == ["a = 1", "b = 2"]
    assert m.captures["pair.k"] == ["a", "b"]
    assert (m["pair"], m["pair.k"], m["pair[0].k"], m["pair.v[1]"], m["pair[1]:upper"]) == (
        "b = 2",
        "b",
        "a",
        "2",
        "B = 2",
    )
    with pytest.raises(IndexError):
        m["pair[5]"]
    with pytest.raises(ValueError, match="has no index"):
        trex.Pattern(r"\W:w").find("x")["w[0]"]
    assert trex.Pattern(r"(\N:n)+").find(b"1 2 3").captures["n"] == [b"1", b"2", b"3"]


def test_a_pattern_file_declares_what_a_pattern_reads(tmp_path):
    defs = tmp_path / "defs.trex"
    defs.write_text("let rhs = \\N | \\Q\nkind assign = \\W \"=\" \\{rhs}\n")
    p = trex.Pattern(r"\{assign}", lib=str(defs))
    text = 'let x = 1; name = "bob"'
    # A kind from a pattern fuses the tokens its pattern covers into one, so
    # the scan reports the whole assignment.
    assert [m.text for m in p.scan(text)] == ["x = 1", 'name = "bob"']
    assert p.is_match(text) and not p.is_match("nothing here")
    assert p.find(text).text == "x = 1"
    assert p.rewrite("<${0}>", text) == 'let <x = 1>; <name = "bob">'
    assert p.rewrite_first("<${0}>", text) == 'let <x = 1>; name = "bob"'
    assert p.split(text) == ["let ", "; ", ""]
    # A shaped pattern cuts on its own shaped scan rather than the library's
    # split, so it must keep the same limit contract: a limit of one leaves the
    # input whole, and a limit of none yields nothing at all.
    assert p.splitn(text, 2) == ["let ", '; name = "bob"']
    assert p.splitn(text, 1) == [text]
    assert p.splitn(text, 0) == []

    # A shape the lexer runs before its own recognizers, with a register
    # bound over it, and a list of files read in order.
    shapes = tmp_path / "shapes.trex"
    shapes.write_text("shape ticket = `[A-Z]{2,4}-\\d{1,4}`\n")
    t = trex.Pattern(r"\{ticket}:t", lib=[str(defs), str(shapes)])
    m = t.find("see AB-12 and XYZ-9 now")
    assert (m.text, m["t"], m.span()) == ("AB-12", "AB-12", (4, 9))
    assert t.rewrite("[${t}]", "see AB-12 now") == "see [AB-12] now"
    assert trex.parse(r"\{ticket}", lib=str(shapes)).is_match("AB-12")

    # A file of `let` lines alone declares nothing the lexer sees, but the
    # names it declares must still resolve.
    lets = tmp_path / "lets.trex"
    lets.write_text("let num = \\N\n")
    assert [m.text for m in trex.Pattern(r"\{num}", lib=str(lets)).scan("a 1 b 22")] == ["1", "22"]

    # A pattern naming what no file declares, and a file that is not one.
    with pytest.raises(ValueError, match="pattern error at byte"):
        trex.Pattern(r"\{nosuch}", lib=str(defs))
    with pytest.raises(ValueError):
        trex.Pattern(r"\N", lib=str(tmp_path / "missing.trex"))
    with pytest.raises(TypeError, match="pattern file's path"):
        trex.Pattern(r"\N", lib=3)


def test_typed_predicates_read_the_clock_set_here():
    trex.set_now(1_789_430_400)
    try:
        p = trex.Pattern(r"\T{age<24h}")
        got = [m.text for m in p.scan("at 2026-09-14T12:00:00Z and 2026-09-01T00:00:00Z")]
        assert got == ["2026-09-14T12:00:00Z"]
    finally:
        trex.set_now(None)
    assert [m.text for m in trex.Pattern(r"\N{>=100}").scan("5 50 500 5000")] == ["500", "5000"]
    assert trex.Pattern(r"\I{in:10.0.0.0/8}").find("from 192.168.1.1 to 10.4.5.6").text == "10.4.5.6"


def test_a_number_in_any_notation_is_one_token_read_by_its_value():
    text = "a 1e3 b 0x3e8 c 50 d 1_000 e 2.5e-4 f 6.02e23 g 0b1010"
    assert [m.text for m in trex.Pattern(r"\N{>100}").scan(text)] == ["1e3", "0x3e8", "1_000", "6.02e23"]
    assert trex.Pattern(r"\N:n").find("x 0x3e8").value("n") == 1000
    assert trex.Pattern(r"\N:n").find("x 2.5e-4").value("n") == Decimal("0.00025")
    assert trex.Pattern(r"\N:n").find("x 1e22").value("n") == Decimal(10) ** 22
    # A comma separates, and a letter right after a form leaves only the digits as the number.
    assert [t.text for t in trex.tokens("1,000 1e3a")] == ["1", ",", "000", "1", "e3a"]


def test_the_date_order_decides_a_slash_date_both_readings_hold():
    p = trex.Pattern(r"\T:t")
    fields = "${t:year}-${t:month}-${t:day}"
    try:
        assert p.rewrite(fields, "03/04/2026") == "2026-04-03"
        trex.set_date_order("mdy")
        assert p.rewrite(fields, "03/04/2026") == "2026-03-04"
        assert p.rewrite(fields, "15/09/2026") == "2026-09-15"
        with pytest.raises(ValueError, match="'dmy' or 'mdy'"):
            trex.set_date_order("ymd")
    finally:
        trex.set_date_order("dmy")


def test_every_way_of_running_a_scan_finds_the_same_matches():
    p = trex.Pattern(r"\W:k \N:v")
    text = "a 1 b 22 c 333 " * 50
    plain = [(m.start, m.text) for m in p.scan(text)]
    assert len(plain) == 150
    for way in ({"backend": "cpu"}, {"backend": "gpu"}, {"dual_grain": True}, {"chunk_size": 7}):
        assert [(m.start, m.text) for m in p.scan(text, **way)] == plain, way
        assert [(m.start, m.text) for m in p.find_iter(text, **way)] == plain, way
        assert p.rewrite("<${v}>", text, **way) == p.rewrite("<${v}>", text), way
        assert p.rewrite(lambda m: m["v"] * 2, text, **way) == p.rewrite(lambda m: m["v"] * 2, text), way
    with pytest.raises(ValueError, match="give one"):
        p.scan(text, dual_grain=True, chunk_size=4)
    with pytest.raises(ValueError, match="1 or more"):
        p.scan(text, chunk_size=0)
    with pytest.raises(ValueError, match="'auto', 'cpu' or 'gpu'"):
        p.scan(text, backend="tpu")


def test_scan_stats_read_the_last_scan_on_this_thread():
    p = trex.Pattern(r"\W \N")
    text = "a 1\nb 22 c 3\nnone here\nd 4\n"
    assert len(p.scan(text)) == 4
    s = trex.scan_stats()
    assert (s.matches, s.matched_lines, s.files_with_matches, s.files_searched, s.bytes_searched) == (4, 3, 1, 1, 27)
    assert s.routes and all(r.inputs >= 1 for r in s.routes)
    assert s.searching.total_seconds() >= s.matching.total_seconds() >= 0
    assert [m.text for m in p.scan(text, lines="2..2")] == ["b 22", "c 3"]
    s = trex.scan_stats()
    assert (s.matches, s.matched_lines, s.bytes_searched) == (2, 1, 9)


PLACED = "é\U0001f600 alpha 12\nbeta 345\n"


def test_a_match_knows_where_it_is():
    p = trex.Pattern(r"\W:w \N:n")
    first, second = p.scan(PLACED)
    assert (first.line, first.column, first.start, first.byte_start) == (1, 4, 3, 7)
    assert (second.line, second.column, second.start) == (2, 1, 12)
    assert (first.path, first.pattern) == (None, None)
    assert (first.kind("w"), first.kind("n")) == ("word", "number")
    with pytest.raises(KeyError):
        first.kind("nope")
    # A match found in a part of the input is reported where the whole input places it.
    (tail,) = p.scan(PLACED, tail=1)
    assert (tail.line, tail.column, tail.start) == (2, 1, 12)
    # A set's match names its member.
    s = trex.PatternSet([r"\W", r"\N"])
    assert [(i, m.pattern) for i, m in s.scan("alpha 12")] == [(0, "0"), (1, "1")]


def test_match_format_renders_a_report_template():
    p = trex.Pattern(r"\W:w \N:n")
    first, second = p.scan(PLACED)
    # Offsets count the units the input is indexed by: characters of a str.
    assert first.format("${line}:${col} ${w}=${n} at ${start}..${end}", PLACED) == "1:4 alpha=12 at 3..11"
    assert PLACED[3:11] == first.text
    assert second.format("${line}:${col} ${w:upper} ${0}", PLACED) == "2:1 BETA beta 345"
    # Bytes of bytes.
    data = PLACED.encode()
    assert p.scan(data)[0].format("${start}..${end} ${0}", data) == "7..15 alpha 12"
    # An axis reads what Match.explain reads.
    assert first.format("${0} kinds=${@kind}", PLACED) == "alpha 12 kinds=word, number"
    s = trex.PatternSet([r"\W", r"\N"])
    assert [m.format("${pattern}:${0}", "alpha 12") for _, m in s.scan("alpha 12")] == ["0:alpha", "1:12"]
    with pytest.raises(ValueError, match="template error"):
        first.format("${nope}", PLACED)
    with pytest.raises(ValueError, match="give the text it was found in"):
        second.format("${0}", "short")


GREP = "a 1\nb\nc 2 3\nd\ne\nf 4\n"


def test_grep_gives_the_lines_matches_start_on_with_their_context():
    p = trex.Pattern(r"\N")
    lines = list(p.grep(GREP))
    assert [(l.number, l.text, l.is_match, [m.text for m in l.matches]) for l in lines] == [
        (1, "a 1", True, ["1"]),
        (3, "c 2 3", True, ["2", "3"]),
        (6, "f 4", True, ["4"]),
    ]
    assert (lines[1].path, lines[1].matches[1].line, lines[1].matches[1].path) == (None, 3, None)
    assert [(l.number, l.is_match) for l in p.grep(GREP, context=1)] == [
        (1, True),
        (2, False),
        (3, True),
        (4, False),
        (5, False),
        (6, True),
    ]
    # max_count counts matches, so the third line keeps one of its two.
    capped = list(p.grep(GREP, after=1, max_count=2))
    assert [(l.number, l.is_match, len(l.matches)) for l in capped] == [(1, True, 1), (2, False, 0), (3, True, 1), (4, False, 0)]
    # Matches are reported at the input's own offsets, in its units.
    assert [(l.number, m.start, m.column) for l in p.grep(PLACED) for m in l.matches] == [(1, 9, 10), (2, 17, 6)]
    assert [l.text for l in p.grep(b"a 1\nb\n")] == [b"a 1"]


def test_grep_inverts_and_keeps_matches_covering_their_line():
    p = trex.Pattern(r"\N")
    assert [(l.number, l.text, l.is_match, l.matches) for l in p.grep(GREP, invert=True)] == [
        (2, "b", True, []),
        (4, "d", True, []),
        (5, "e", True, []),
    ]
    assert [(l.number, l.is_match) for l in p.grep(GREP, invert=True, before=1, max_count=1)] == [(1, False), (2, True)]
    assert [l.text for l in trex.Pattern(r"\W").grep("alpha\nbeta gamma\n", whole_line=True)] == ["alpha"]


def test_grep_carries_the_construct_a_line_is_in():
    body = "header line\nfn outer(a) {\n  let v = inner(a, 42);\n  return 99;\n}\ntrailer line\n"
    p = trex.Pattern(r"\N{99..99}")
    assert [(l.number, l.is_match) for l in p.grep(body, context="block")] == [(2, False), (3, False), (4, True), (5, False)]
    assert [(l.number, l.is_match) for l in p.grep(body, before="block", after=1)] == [
        (2, False),
        (3, False),
        (4, True),
        (5, False),
    ]
    with pytest.raises(ValueError, match="names one unit"):
        p.grep(body, before="block", after="paragraph")
    with pytest.raises(ValueError, match="no record="):
        p.grep(body, context="record")
    with pytest.raises(ValueError, match="a count or a record unit"):
        p.grep(body, context="nonsense")
    with pytest.raises(TypeError, match="one of them"):
        p.grep()


def test_grep_reads_files_one_at_a_time_and_counts_them(tmp_path):
    (tmp_path / "a.log").write_bytes(b"x 1\ny\n")
    (tmp_path / "b.log").write_bytes(b"z\nw 22\n")
    p = trex.Pattern(r"\N")
    lines = list(p.grep(path=tmp_path))
    assert [(os.path.basename(l.path), l.number, l.text, l.matches[0].path == l.path) for l in lines] == [
        ("a.log", 1, "x 1", True),
        ("b.log", 2, "w 22", True),
    ]
    s = trex.scan_stats()
    assert (s.files_searched, s.files_with_matches, s.matches, s.matched_lines) == (2, 2, 2, 2)
    # A tail is reported where the file places it.
    (tail,) = p.grep(path=tmp_path / "b.log", tail=1)
    assert (tail.number, tail.matches[0].line, tail.matches[0].start, tail.matches[0].byte_start) == (2, 2, 4, 4)
    assert trex.scan_stats().files_searched == 1
    # A file holding a NUL byte named alone is refused; walked, it is passed over.
    (tmp_path / "c.bin").write_bytes(b"v 3\x00\n")
    with pytest.raises(ValueError, match="binary=True reads it"):
        list(p.grep(path=tmp_path / "c.bin"))
    assert [l.text for l in p.grep(path=tmp_path / "c.bin", binary=True)] == ["v 3\x00"]
    assert len(list(p.grep(path=tmp_path))) == 2
    with pytest.raises(ValueError, match="no directory"):
        p.grep("text", hidden=True)


QUERIED = "error disk full\nerror net down\ndisk ok\nall well\n"


def test_query_keeps_the_records_holding_its_patterns(tmp_path):
    both = ['"error"', '"disk"']
    assert [(r.line, r.text, r.patterns) for r in trex.query(both, QUERIED, require="all")] == [
        (1, "error disk full", ['"error"', '"disk"'])
    ]
    # With no rule asked, a record holds at least one, as the command line reads it.
    assert [r.line for r in trex.query(both, QUERIED)] == [1, 2, 3]
    assert [r.line for r in trex.query(both, QUERIED, require="none")] == [4]
    assert [r.line for r in trex.query(['"error"', '"disk"', '"full"'], QUERIED, at_least=2)] == [1]
    assert [r.line for r in trex.query(trex.Pattern('"error"'), QUERIED, exclude='"net"')] == [1]
    assert [r.line for r in trex.query('"disk"', QUERIED, max_count=1)] == [1]
    # A record of another unit, or from one match of a pattern to the next.
    assert [r.text for r in trex.query(['"error"', '"b"'], "a error\nb\n\nc disk\n", unit="paragraph")] == ["a error\nb"]
    starts = "start one\nERROR here\nstart two\nfine\n"
    assert [r.text for r in trex.query('"ERROR"', starts, record_start='"start"')] == ["start one\nERROR here\n"]
    # Offsets count the input's units.
    (r,) = trex.query('"x"', "é\nx y\nz\n")
    assert (r.line, r.start, r.end, r.byte_start, r.byte_end, r.text) == (2, 2, 5, 3, 6, "x y")
    assert trex.query('"x"', "é\nx y\n".encode())[0].text == b"x y"
    (tmp_path / "app.log").write_bytes(QUERIED.encode())
    found = trex.query(both, path=tmp_path / "app.log", require="all")
    assert [(r.path, r.line) for r in found] == [(str(tmp_path / "app.log"), 1)]
    assert trex.query(both, QUERIED)[0].to_dict()["patterns"] == ['"error"', '"disk"']
    with pytest.raises(ValueError, match="'all', 'any' or 'none'"):
        trex.query(both, QUERIED, require="some")
    with pytest.raises(ValueError, match="two rules"):
        trex.query(both, QUERIED, require="all", at_least=1)
    with pytest.raises(ValueError, match="give one"):
        trex.query(both, QUERIED, unit="paragraph", record_start='"x"')
    with pytest.raises(TypeError, match="one of them"):
        trex.query(both)


STANZAS = b"job alpha\nstatus ok\ntook 12s\n\njob bravo\nstatus ok\ntook 30s\n\njob delta\nstatus failed\ntook 4s\n"


def test_templates_mark_what_another_input_lacks(tmp_path):
    app = tmp_path / "app.log"
    app.write_bytes(b"user bob logged in\nuser amy logged in\ndisk sda ok\ndisk sdb ok\nquota 90 exceeded\n")
    other = tmp_path / "other.log"
    other.write_bytes(b"user carl logged in\nuser dana logged in\nuser eve logged in\ndisk sda ok\n")
    # The other log's template has a slot where `user <word> logged in` does,
    # has seen only `sda` beside `disk`, and holds nothing like the quota line.
    ts = trex.templates(path=app, against=other)
    assert [(t["readable"], t["count"], t["novel"]) for t in ts] == [
        ("disk <word> ok", 2, True),
        ("user <word> logged in", 2, False),
        ("quota 90 exceeded", 1, True),
    ]
    assert trex.templates(path=app, against=other.read_bytes()) == ts
    assert trex.templates(app.read_text(), against=[other]) == ts
    novel = trex.templates(path=app, against=other, novel=True)
    assert [t["readable"] for t in novel] == ["disk <word> ok", "quota 90 exceeded"]
    assert all(t["covered"] == 5 for t in novel), "the total is the same whatever a filter keeps"
    assert all(t["novel"] is None for t in trex.templates(path=app))
    assert [t["readable"] for t in trex.templates(path=app, rare=True, cut="2")] == ["quota 90 exceeded"]
    with pytest.raises(ValueError, match="against= names none"):
        trex.templates(path=app, novel=True)
    with pytest.raises(ValueError, match="cut="):
        trex.templates(path=app, cut="five")


def test_templates_mine_records_of_a_unit_across_files(tmp_path):
    (tmp_path / "stanzas.log").write_bytes(STANZAS)
    stanzas = tmp_path / "stanzas.log"
    # By line the stanzas are three shapes; by paragraph they are one, whose
    # template spans the three lines of each record.
    assert [(t["count"], t["readable"]) for t in trex.templates(path=stanzas)] == [
        (6, "<word> <word>"),
        (3, "took <duration>"),
    ]
    whole = "job <word>\nstatus <word>\ntook <duration>"
    by_paragraph = trex.templates(path=stanzas, unit="paragraph")
    assert [(t["count"], t["records"], t["readable"]) for t in by_paragraph] == [(3, [0, 1, 2], whole)]
    assert [t["readable"] for t in trex.templates(path=stanzas, record_start='"job"')] == [whole]
    lib = trex.Library()
    lib.declare('let opener = "job"')
    assert [t["readable"] for t in trex.templates(path=stanzas, record_start=r"\{opener}", lib=lib)] == [whole]
    # A window counts records of the unit mined, in each input.
    assert [t["count"] for t in trex.templates(path=stanzas, unit="paragraph", head=2)] == [2]
    assert [t["count"] for t in trex.templates(STANZAS, unit="paragraph", tail=1)] == [1]
    # Several files are one stream, as a directory of them is.
    (tmp_path / "more.log").write_bytes(b"job echo\nstatus ok\ntook 1s\n\n")
    assert [t["count"] for t in trex.templates(path=tmp_path, unit="paragraph")] == [4]
    with pytest.raises(ValueError, match="give one"):
        trex.templates(STANZAS, unit="paragraph", record_start='"job"')
    with pytest.raises(ValueError, match="neither names one"):
        trex.templates(STANZAS, hidden=True)
    with pytest.raises(TypeError, match="one of them"):
        trex.templates()


def test_templates_refuse_a_binary_file_named_alone(tmp_path):
    blob = tmp_path / "blob.log"
    blob.write_bytes(b"user bob logged in\x00\n")
    with pytest.raises(ValueError, match="binary=True reads it"):
        trex.templates(path=blob)
    with pytest.raises(ValueError, match="binary=True reads it"):
        trex.templates("user amy logged in\n", against=blob)
    assert [t["count"] for t in trex.templates(path=blob, binary=True)] == [1]
    # One found by a walk is passed over.
    (tmp_path / "app.log").write_bytes(b"user amy logged in\n")
    assert [t["count"] for t in trex.templates(path=tmp_path)] == [1]


def test_offsets_written_by_a_template_count_the_input_s_units():
    lib = trex.Library()
    lib.declare('rule placed note "at ${start}..${end}" = "alpha"')
    (found,) = lib.check(PLACED)
    assert (found.message, found.format("${start}..${end}")) == ("at 3..8", "3..8")
    (found,) = lib.check(PLACED.encode())
    assert (found.message, found.format("${start}")) == ("at 7..12", "7")
    p = trex.Pattern(r"\W:w \N")
    assert dict(p.count_by("${start}", PLACED)) == {"3": 1, "12": 1}
    assert dict(p.count_by("${start}", PLACED.encode())) == {"7": 1, "16": 1}
    assert p.count_by("${start}", PLACED, lines="2..") == [("12", 1)]


def test_pattern_sets_and_streams(tmp_path):
    s = trex.PatternSet([r"\E", trex.Pattern(r"\I"), r"\U"])
    text = "from 10.0.0.1 to bob@x.com"
    assert len(s) == 3
    assert s.names == ["0", "1", "2"]
    assert s.matches(text) == [0, 1]
    assert s.is_match(text) and not s.is_match("nothing here")
    assert [(i, m.text) for i, m in s.matches_with_spans(text)] == [(0, "bob@x.com"), (1, "10.0.0.1")]
    assert s.matches_at(text, 14) == [0]
    assert [(i, m.text) for i, m in s.scan("10.0.0.1 and 10.0.0.2 mail bob@x.com")] == [
        (1, "10.0.0.1"),
        (1, "10.0.0.2"),
        (0, "bob@x.com"),
    ]
    rules = tmp_path / "rules.trex"
    rules.write_text("let host = \\I:addr\nlet mail = \\E:e\n\\N{>=100}\n")
    f = trex.PatternSet.from_file(str(rules))
    assert f.names == ["host", "mail", "3"]
    got = [(f.names[i], m.text, m.captures) for i, m in f.scan("from 10.0.0.1 at 500 to bob@x.com")]
    assert got == [
        ("host", "10.0.0.1", {"addr": "10.0.0.1"}),
        ("3", "500", {}),
        ("mail", "bob@x.com", {"e": "bob@x.com"}),
    ]
    assert f.scan("at 500")[0][1]["0"] == "500"
    bad = tmp_path / "bad.trex"
    bad.write_text("let host = \\I\n\\N{>x}\n")
    with pytest.raises(ValueError, match="line 2: pattern error"):
        trex.PatternSet.from_file(str(bad))
    st = trex.StreamScanner(trex.Pattern(r"\N"))
    got = st.push(b"one 1 two 2\nthree 3 fo")
    got += st.push(b"ur 4\n")
    got += st.finish()
    assert got == [(4, 5), (10, 11), (18, 19), (25, 26)]
    with pytest.raises(ValueError):
        st.push(b"more")
    st = trex.StreamScanner(f)
    got = st.push(b"from 10.0.0.1\n") + st.push(b"to bob@x.com\n") + st.finish()
    assert got == [(0, 5, 13), (1, 17, 26)]
    with pytest.raises(TypeError, match="a Pattern or a PatternSet"):
        trex.StreamScanner(r"\N")


FIVE = "one 1\ntwo 2\nthree 3\nfour 4\nfive 5\n"


def test_head_tail_and_lines_read_a_part_of_an_input(tmp_path):
    assert trex.head(2, FIVE) == "one 1\ntwo 2\n"
    assert trex.tail(2, FIVE) == "four 4\nfive 5\n"
    assert trex.lines("2..3", FIVE) == "two 2\nthree 3\n"
    assert trex.tail(1, FIVE.encode()) == b"five 5\n"
    f = tmp_path / "f.txt"
    f.write_bytes(FIVE.encode())
    assert trex.tail(2, path=str(f)) == "four 4\nfive 5\n"
    assert trex.head(1, path=str(f)) == "one 1\n"
    assert trex.lines("4..", path=str(f)) == "four 4\nfive 5\n"
    assert trex.head(1, "a one\na two\n\nb one\n", unit="paragraph") == "a one\na two\n"
    with pytest.raises(TypeError, match="one of them"):
        trex.head(1)
    with pytest.raises(ValueError, match="after its end"):
        trex.lines("3..2", FIVE)


def leaves(paths):
    return [p.replace("\\", "/").rsplit("/", 1)[-1] for p in paths]


def test_files_lists_what_a_scan_reads(tmp_path, monkeypatch):
    tree = tmp_path / "tree"
    tree.mkdir()
    (tree / "a.log").write_text("from 10.0.0.1\n")
    (tree / "b.txt").write_text("alpha 1\n")
    (tree / "c.rs").write_text("fn main() {}\n")
    (tree / "d.bin").write_bytes(b"ab\0cd\n")
    (tree / ".hidden").write_text("x\n")
    monkeypatch.chdir(tmp_path)
    assert leaves(trex.files("tree")) == ["a.log", "b.txt", "c.rs"]
    assert leaves(trex.files("tree", binary=True)) == ["a.log", "b.txt", "c.rs", "d.bin"]
    assert leaves(trex.files("tree", hidden=True)) == [".hidden", "a.log", "b.txt", "c.rs"]
    assert leaves(trex.files(["tree/c.rs", "tree/a.log"])) == ["c.rs", "a.log"]
    assert leaves(trex.files(["tree/d.bin", "tree/a.log"])) == ["a.log"]
    assert leaves(trex.files("tree", globs=["*.rs"])) == ["c.rs"]
    assert leaves(trex.files("tree", globs=["!*.rs"])) == ["a.log", "b.txt"]
    assert leaves(trex.files("tree", types=["rust"])) == ["c.rs"]
    assert leaves(trex.files("tree", types_not=["rust"])) == ["a.log", "b.txt"]
    assert leaves(trex.files("tree", sort="path", reverse=True)) == ["c.rs", "b.txt", "a.log"]
    assert leaves(trex.files()) == ["a.log", "b.txt", "c.rs"]
    with pytest.raises(ValueError, match="sort by"):
        trex.files("tree", sort="size")
    with pytest.raises(ValueError, match="give sort="):
        trex.files("tree", reverse=True)
    with pytest.raises(ValueError, match="no texture"):
        trex.files("tree", texture=["tabular"])
    with pytest.raises(ValueError, match="unrecognized file type"):
        trex.files("tree", types=["bogus"])
    with pytest.raises(OSError, match="missing"):
        trex.files("missing")


ROWS = "id,host,bytes,ms\n1,alpha,1024,12\n2,beta,2048,19\n3,gamma,4096,31\n4,delta,8192,44\n5,epsilon,16384,57\n6,zeta,32768,73\n"
PROSE = (
    "The walk reports what it found rather than what it was asked for. A filter that\n"
    "silently drops a file reads exactly the same as a directory that never held one, and\n"
    "the reader cannot tell the two apart afterwards.\n"
)


def test_texture_names_what_text_reads_as_mostly(tmp_path, monkeypatch):
    assert trex.texture(ROWS) == ("table", 7)
    assert trex.texture(ROWS.encode()) == ("table", 7)
    assert trex.texture(PROSE) == ("prose", 0)
    assert trex.texture("") is None
    (tmp_path / "rows.csv").write_text(ROWS)
    (tmp_path / "prose.md").write_text(PROSE)
    assert trex.texture(path=str(tmp_path / "rows.csv")) == ("table", 7)
    monkeypatch.chdir(tmp_path)
    assert leaves(trex.files(".", texture=["table"])) == ["rows.csv"]
    assert leaves(trex.files(".", texture_not=["table"])) == ["prose.md"]
    with pytest.raises(TypeError, match="one of them"):
        trex.texture()


def test_read_decodes_a_file_as_a_scan_reads_it(tmp_path):
    f = tmp_path / "wide.txt"
    f.write_bytes(b"\xff\xfe" + "café 42\n".encode("utf-16-le"))
    assert trex.read(str(f)) == "café 42\n"
    assert trex.Pattern(r"\N").find(trex.read(str(f))).text == "42"
    latin = tmp_path / "latin.txt"
    latin.write_bytes(b"caf\xe9\n")
    with pytest.raises(ValueError, match="not UTF-8"):
        trex.read(str(latin))
    with pytest.raises(OSError):
        trex.read(str(tmp_path / "missing.txt"))


def test_a_file_rewrite_diffs_or_writes_in_the_file_s_encoding(tmp_path):
    p = trex.Pattern('"legacy_call"')
    f = tmp_path / "notes.md"
    f.write_bytes(b"\xff\xfe" + "see legacy_call here\n".encode("utf-16-le"))
    shown = str(f)
    assert p.diff("current_call", shown) == (
        f"--- {shown}\n+++ {shown}\n@@ -1,1 +1,1 @@\n-see legacy_call here\n+see current_call here\n"
    )
    assert p.diff(lambda m: m.text.upper(), shown).endswith("+see LEGACY_CALL here\n")
    assert p.rewrite_file("current_call", shown) == 1
    assert f.read_bytes() == b"\xff\xfe" + "see current_call here\n".encode("utf-16-le")
    assert p.diff("current_call", shown) == ""
    assert p.rewrite_file("current_call", shown) == 0
    binary = tmp_path / "b.bin"
    binary.write_bytes(b"legacy_call\0")
    with pytest.raises(ValueError, match="binary"):
        p.diff("current_call", str(binary))
    with pytest.raises(ValueError, match="binary"):
        p.rewrite_file("current_call", str(binary))
    with pytest.raises(TypeError, match="template str or a callable"):
        p.rewrite_file(3, shown)


def test_file_edits_walk_a_tree_with_a_count_a_window_and_a_backend(tmp_path):
    (tmp_path / "a.log").write_bytes(b"a 1\nb 22\n")
    (tmp_path / "b.log").write_bytes(b"c 333\n")
    a, b = str(tmp_path / "a.log"), str(tmp_path / "b.log")
    p = trex.Pattern(r"\N")
    diff = p.diff("#", tmp_path)
    assert f"--- {a}\n" in diff and f"--- {b}\n" in diff and diff.index(a) < diff.index(b), diff
    assert "+a #\n" in diff and "+b #\n" in diff and "+c #\n" in diff, diff
    one = p.diff("#", [a, b], max_count=1)
    assert "+a #\n" in one and "+b #" not in one and "+c #\n" in one, one
    tailed = p.diff("#", a, tail=1)
    assert "+b #\n" in tailed and "+a #" not in tailed, tailed
    assert p.diff("#", tmp_path, backend="cpu") == diff
    assert p.diff("#", tmp_path, backend="gpu") == diff
    with pytest.raises(ValueError, match="'auto', 'cpu' or 'gpu'"):
        p.diff("#", tmp_path, backend="tpu")
    assert p.rewrite_file("#", tmp_path, max_count=1) == 2
    assert (tmp_path / "a.log").read_bytes() == b"a #\nb 22\n"
    assert (tmp_path / "b.log").read_bytes() == b"c #\n"
    # A walked file holding a NUL byte is passed over; one named alone is refused.
    (tmp_path / "c.bin").write_bytes(b"d 4\0")
    assert p.rewrite_file("#", tmp_path) == 1
    with pytest.raises(ValueError, match="binary=True rewrites it"):
        p.rewrite_file("#", tmp_path / "c.bin")
    assert p.rewrite_file("#", tmp_path / "c.bin", binary=True) == 1


def test_a_file_edit_puts_each_change_to_a_review(tmp_path):
    (tmp_path / "a.log").write_bytes(b"a 1\nb 22\n")
    (tmp_path / "b.log").write_bytes(b"c 333\n")
    p = trex.Pattern(r"\N")
    seen = []

    def review(change):
        seen.append(change)
        return change.text != "22"

    assert p.rewrite_file("#", tmp_path, review=review, explain=True) == 2
    assert (tmp_path / "a.log").read_bytes() == b"a #\nb 22\n"
    assert (tmp_path / "b.log").read_bytes() == b"c #\n"
    assert [(c.text, c.index, c.total) for c in seen] == [("1", 1, 3), ("22", 2, 3), ("333", 3, 3)]
    assert seen[0].explanation["tokens"] == [("number", "1")]
    assert seen[0].to_dict()["explanation"] == seen[0].explanation
    plain = []
    assert p.rewrite_file("#", tmp_path, review=lambda c: plain.append(c) or False) == 0
    assert plain and plain[0].explanation is None
    masked = []
    assert p.redact_file(tmp_path, review=lambda c: masked.append(c) or True) == 1
    assert (tmp_path / "a.log").read_bytes() == b"a #\nb **\n"
    assert [c.replacement for c in masked] == ["**"]
    with pytest.raises(ValueError, match="only review= gives"):
        p.rewrite_file("#", tmp_path, show_skipped=True)
    with pytest.raises(ValueError, match="give review="):
        p.redact_file(tmp_path, explain=True)
    with pytest.raises(TypeError, match="a callable"):
        p.rewrite_file("#", tmp_path, review="yes")


def test_file_redactions_name_a_value_alike_in_every_file(tmp_path):
    (tmp_path / "a.log").write_bytes(b"from 10.0.0.1 to 10.0.0.2\n")
    (tmp_path / "b.log").write_bytes(b"again 10.0.0.2\n")
    p = trex.Pattern(r"\I")
    diff = p.redact_diff(tmp_path, mask="pseudonym")
    assert "+from IP_1 to IP_2\n" in diff and "+again IP_2\n" in diff, diff
    assert p.redact_file(tmp_path, mask="pseudonym", tail=1) == 3
    assert (tmp_path / "b.log").read_bytes() == b"again IP_2\n"
    with pytest.raises(ValueError, match="keep= error"):
        p.redact_file(tmp_path, keep="z:octet1")


def test_follow_lines_gives_each_line_a_file_gains(tmp_path):
    log = tmp_path / "app.log"
    log.write_bytes(b"a 1\nb 2\n")
    lines = trex.follow_lines(log, tail=1)
    first = next(lines)
    assert (first.path, first.number, first.text, first.is_match) == (str(log), 2, "b 2", True)
    with log.open("ab") as grown:
        grown.write(b"c 3\nd")
    third = next(lines)
    assert (third.number, third.text) == (3, "c 3")
    # A line whose end has not arrived is held; a truncation gives it, and
    # the file is read again from its first line.
    log.write_bytes(b"e 5\n")
    with pytest.warns(trex.TrexWarning, match="app.log: truncated"):
        held = next(lines)
    assert (held.number, held.text) == (4, "d")
    again = next(lines)
    assert (again.number, again.text) == (1, "e 5")
    with pytest.raises(ValueError, match="neither was given"):
        trex.follow_lines(log)
    with pytest.raises(ValueError, match="ends before the file does"):
        trex.follow_lines(log, lines="1..2")


def test_follow_rewrite_and_redact_give_each_edited_line(tmp_path):
    log = tmp_path / "app.log"
    for keep in (False, True):
        log.write_bytes(b"a 1 2\n")
        edits = trex.follow_rewrite(r"\N", "#", log, max_count=1, keep_count=keep)
        assert next(edits) == "a # 2"
        with log.open("ab") as grown:
            grown.write(b"b 3\n")
        assert next(edits) == "b 3"
        log.write_bytes(b"c 4\n")
        # The file now under the name is a new input, its count begun again
        # unless keep_count= carries it on.
        with pytest.warns(trex.TrexWarning, match="app.log: truncated"):
            assert next(edits) == ("c 4" if keep else "c #")
    placed = trex.follow_rewrite(trex.Pattern(r"\N"), lambda m: f"<{m.text}@{m.line}>", log)
    assert next(placed) == "c <4@1>"
    with log.open("ab") as grown:
        grown.write(b"d 5\n")
    assert next(placed) == "d <5@2>"
    masks = trex.follow_redact(r"\I", log, mask="pseudonym", tail=0)
    with log.open("ab") as grown:
        grown.write(b"from 10.0.0.1 to 10.0.0.1\n")
    assert next(masks) == "from IP_1 to IP_1"
    with pytest.raises(ValueError, match="no max_count was given"):
        trex.follow_rewrite(r"\N", "#", log, keep_count=True)
    with pytest.raises(TypeError, match="template str or a callable"):
        trex.follow_rewrite(r"\N", 3, log)


def test_a_window_restricts_a_scan_a_rewrite_and_a_redaction():
    p = trex.Pattern(r"\N")
    assert [m.text for m in p.scan(FIVE, tail=2)] == ["4", "5"]
    assert [m.start for m in p.scan(FIVE, tail=2)] == [25, 32]
    assert [m.text for m in p.scan(FIVE, lines="2..3")] == ["2", "3"]
    assert [m.text for m in p.find_iter(FIVE, head=1)] == ["1"]
    assert p.find(FIVE, tail=1).start == 32
    assert p.is_match(FIVE, head=1)
    assert p.rewrite("<${0}>", FIVE, tail=1) == "five <5>\n"
    assert p.rewrite(lambda m: m.text * 2, FIVE, head=1) == "one 11\n"
    assert p.redact(FIVE, head=1) == "one *\n"
    assert p.redact(FIVE) == FIVE.replace("1", "*").replace("2", "*").replace("3", "*").replace("4", "*").replace("5", "*")
    assert p.count_by("${0}", FIVE, lines="4..") == [("4", 1), ("5", 1)]
    # Beyond ASCII, a str's offsets are characters of the whole input.
    wide = "é 1\n中 2\n"
    m = p.scan(wide, tail=1)[0]
    assert (m.start, wide[m.start], m.byte_start) == (6, "2", 9)
    with pytest.raises(ValueError, match="give one"):
        p.scan(FIVE, head=1, tail=1)
    with pytest.raises(ValueError, match="none was given"):
        p.scan(FIVE, unit="paragraph")
    assert [t["count"] for t in trex.templates(FIVE, head=2)] == [2]


TITLES = [
    "2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB5031354)",
    "2023-09 Cumulative Update Preview for Windows 11 Version 22H2 for x64-based Systems (KB5030310)",
    "2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)",
    "Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)",
]

MARKED_TITLE = (
    "{month:2023-10} Cumulative Update for Windows {os:11} Version {version:22H2} "
    "for x64-based Systems ({kb:KB5031354})"
)


def test_infer_builds_a_pattern_for_marked_and_hinted_fields(tmp_path):
    built = trex.infer(TITLES, marked=[MARKED_TITLE])
    assert isinstance(built, trex.Built)
    assert str(built) == built.pattern
    assert [f["name"] for f in built.fields] == ["month", "os", "version", "kb"]
    assert built.format == "${month}\\t${os}\\t${version}\\t${kb}"
    values = [r["values"] for r in built.rows]
    assert values[3] == {"month": None, "os": "10", "version": "1607", "kb": "KB4103720"}
    assert values[2]["version"] is None
    assert [s["reach"]["kb"] for s in built.shapes].count("marked") == 1
    got = [m["kb"] for m in trex.Pattern(built.pattern).scan("\n".join(TITLES))]
    assert got == ["KB5031354", "KB5030310", "KB4534310", "KB4103720"]

    # A field named by value: one value, or a list of them.
    hinted = trex.infer(TITLES, fields={"kb": "KB5031354"})
    assert [r["values"]["kb"] for r in hinted.rows] == got
    assert trex.infer(TITLES, fields={"kb": ["KB5031354", "KB4534310"]}).rows[3]["values"]["kb"] == "KB4103720"

    # Marks read inside the lines, a counter-example's range, and the file.
    ranged = trex.infer(["GET /a {status:200}", "GET /b 204"], marks_in_lines=True, against=["GET /c 500"])
    assert "\\N{200..299}" in ranged.pattern
    path = tmp_path / "status.trex"
    path.write_text(ranged.file)
    assert "extract" in trex.PatternSet.from_file(str(path)).names

    # A template's starred field begins a record that the lines after it join,
    # as ConvertFrom-String reads one; a record lacking a field holds None.
    template = (
        "Name: {Name*:Phoebe Cat}\nPhone: {phone:425-123-6789}\n\n"
        "Name: {Name*:Lucky Shot}\nPhone: {phone:206-987-4321}\n"
    )
    pets = trex.infer(["Name: Elephant Wise", "Phone: 425-888-7766", "Name: Wise Owl"], marked=[template])
    assert [f["starts_record"] for f in pets.fields] == [True, False]
    assert [r["values"] for r in pets.records][-2:] == [
        {"Name": "Elephant Wise", "phone": "425-888-7766"},
        {"Name": "Wise Owl", "phone": None},
    ]

    # A starred mark spanning lines is a record of the lines it spans, the
    # marks inside it its fields and its text the text of each line joined.
    people = trex.infer(
        ["Name: Wise Owl", "Phone: 425-888-7766"],
        marked=["{Person*:Name: {Name:Phoebe Cat}\nPhone: {Phone:425-123-6789}}"],
    )
    assert [f["name"] for f in people.fields] == ["Person", "Person.Name", "Person.Phone"]
    assert [f["starts_record"] for f in people.fields] == [False, True, False]
    assert people.records[-1]["values"] == {
        "Person": {"text": "Name: Wise Owl\nPhone: 425-888-7766", "Name": "Wise Owl", "Phone": "425-888-7766"}
    }

    # A saved build is read again without building: its file carries the
    # fields line, and a Pattern read under it gives the build's fields and
    # reads records as the build did.
    saved = tmp_path / "people.trex"
    saved.write_text(people.file)
    reused = trex.Pattern(r"\{extract}", lib=str(saved))
    assert reused.fields == people.fields
    assert reused.records("Name: Big Bird\nPhone: 206-555-0100\nName: Elmo Red\n") == [
        {"lines": [0, 1], "values": {"Person": {"text": "Name: Big Bird\nPhone: 206-555-0100", "Name": "Big Bird", "Phone": "206-555-0100"}}},
        {"lines": [2], "values": {"Person": {"text": "Name: Elmo Red", "Name": "Elmo Red", "Phone": None}}},
    ]
    assert [f["name"] for f in trex.Pattern(r"\W:w \N:n").fields] == ["w", "n"]

    # A field marked twice in a line is a list of every value it holds, and
    # `ip[*]` reads every binding joined with a comma.
    hops = trex.infer(
        ["from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok", "from 10.0.0.3 ok"],
        marked=["from {ip:10.0.0.1} -> {ip:10.0.0.2} ok"],
    )
    assert [(f["name"], f["list"]) for f in hops.fields] == [("ip", True)]
    assert hops.format == "${ip[*]}"
    by_text = {r["text"]: r["values"]["ip"] for r in hops.rows}
    assert by_text["from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok"] == ["10.0.0.7", "10.0.0.8", "10.0.0.9"]
    assert by_text["from 10.0.0.3 ok"] == ["10.0.0.3"]
    m = trex.Pattern(hops.pattern).find("from 1.1.1.1 -> 2.2.2.2 ok")
    assert m["ip[*]"] == "1.1.1.1,2.2.2.2"
    assert m["ip[*]:octet1"] == "1,2"
    assert m["ip"] == "2.2.2.2"

    # A word field whose values share a constant part is its byte shape,
    # unless no_mint.
    assert "(`KB[0-9]{7}`):kb" in built.pattern
    assert "(\\W):kb" in trex.infer(TITLES, marked=[MARKED_TITLE], no_mint=True).pattern
    assert built.declarations == []
    shaped = trex.infer(TITLES, marked=[MARKED_TITLE], mint_shapes=True)
    assert shaped.declarations == ["shape kb = `KB[0-9]{7}`"]
    assert "(\\{kb}):kb" in shaped.pattern
    lib = tmp_path / "titles.trex"
    lib.write_text(shaped.file)
    assert trex.Pattern(shaped.pattern, lib=str(lib)).find("\n".join(TITLES))["kb"] == "KB5031354"
    with pytest.raises(ValueError, match="opposite spellings"):
        trex.infer(TITLES, marked=[MARKED_TITLE], no_mint=True, mint_shapes=True)

    # A mark inside a mark is a field of an object holding the outer text.
    nested = trex.infer(["5 of 9", "6 of 9"], marked=["{Line:{[int]n:1} of {[int]m:3}}"])
    assert [(f["name"], f["parent"]) for f in nested.fields] == [("Line", None), ("Line.n", "Line"), ("Line.m", "Line")]
    assert nested.rows[1]["values"] == {"Line": {"text": "5 of 9", "n": "5", "m": "9"}}
    assert trex.Pattern(nested.pattern).find("7 of 8")["Line.m"] == "8"

    # A line repeating a starred record gives a record per repeat.
    people = trex.infer(
        ["Wise Owl, 87; Big Bird, 5"],
        marked=["{Name*:Phoebe Cat}, {[int]age:6}; {Name*:Lucky Shot}, {[int]age:12}"],
    )
    assert [f["repeats"] for f in people.fields] == [True, True]
    assert people.rows[1]["values"] == {"Name": ["Wise Owl", "Big Bird"], "age": ["87", "5"]}
    assert [r["values"] for r in people.records][2:] == [
        {"Name": "Wise Owl", "age": "87"},
        {"Name": "Big Bird", "age": "5"},
    ]

    # A library value class is suggested until an against line needs it.
    levels = trex.infer(["WARN disk 7"], marked=["{level:ERROR} disk {n:5}"])
    assert levels.suggestions == [("level", ["log_level"])]
    assert "(\\{log_level}):level" in trex.infer(["WARN disk 7"], marked=["{level:ERROR} disk {n:5}"], against=["HELLO disk 9"]).pattern

    # lib= lexes the examples under a file's shapes, as Pattern's does.
    ticket = tmp_path / "ticket.trex"
    ticket.write_text("shape ticket = `[A-Z]{2,4}-\\d{1,4}`\n")
    assert trex.infer(["see XYZ-9 now"], marked=["see {t:AB-12} now"]).pattern == '^ "see" (\\W "-" \\N):t "now" ~<($ .)'
    shaped = trex.infer(["see XYZ-9 now"], marked=["see {t:AB-12} now"], lib=str(ticket))
    assert shaped.pattern == '^ "see" (\\{ticket}):t "now" ~<($ .)'
    with pytest.raises(ValueError, match="built for fields"):
        trex.infer(["a 1", "b 2"], lib=str(ticket))

    # With no field, infer returns the pattern string.
    assert trex.infer(["code 200", "code 204"]) == '"code" \\N'
    with pytest.raises(ValueError, match="more than once"):
        trex.infer(["a 1 b 1"], fields={"n": "1"})
    with pytest.raises(TypeError, match="a str or a list of str"):
        trex.infer(["a 1 b"], fields={"n": 1})


SIZES = "alpha 1000B 80ms\nalpha 2000B 80ms\nalpha 4000B 80ms\nbeta 8000B 300ms\n"


def test_group_by_counts_each_key_and_aggregates_its_registers():
    p = trex.Pattern(r"\W:h \Z:s \R:d")
    groups = p.group_by("${h}", SIZES, sum=["s", "d"], avg="s", min="s", max="${s}", percentiles={"s": (50, 100)})
    assert [(g.key, g.count) for g in groups] == [("alpha", 3), ("beta", 1)]
    alpha = groups[0]
    # Sizes in bytes and durations in nanoseconds, as Match.value reads them.
    assert alpha.sum == {"s": 7000, "d": 240_000_000}
    # The average is exact.
    assert alpha.avg == {"s": Fraction(7000, 3)}
    assert alpha.min == {"s": 1000}
    assert alpha.max == {"s": 4000}
    assert alpha.percentiles == {"s": {50: 2000, 100: 4000}}
    ms = p.group_by("${h}", SIZES, sum="d", avg="d", duration_unit="ms")
    assert (ms[0].sum, ms[0].avg, ms[1].sum) == ({"d": 240}, {"d": 80}, {"d": 300})

    # A group whose matches bound no value holds None, which is not a zero.
    q = trex.Pattern(r"\W:k (\N:n)?")
    rows = q.group_by("${k}", "a 3 b 5 a 4 c - a 2", sum="n", order="count")
    assert [(g.key, g.count, g.sum["n"]) for g in rows] == [("a", 3, 9), ("b", 1, 5), ("c", 1, None)]
    assert [g.key for g in q.group_by("${k}", "a 3 b 5 a 4 c - a 2", order="count", limit=1)] == ["a"]
    assert q.distinct("${k}", "a 3 b 5 a 4 c - a 2") == ["a", "b", "c"]

    # A percentile between two values is decided by the method named.
    nums = trex.Pattern(r"\N:n")
    (g,) = nums.group_by("all", "10 20 30 40", percentiles={"n": 50})
    assert g.percentiles == {"n": {50: 20}}
    (g,) = nums.group_by("all", "10 20 30 40", percentiles={"n": [50]}, percentile_method="linear")
    assert g.percentiles == {"n": {50: 25}}
    assert repr(g) == "Group(key='all', count=4, percentiles={'n': {50: 25}})"
    assert g.to_dict() == {
        "key": "all", "count": 4, "sum": {}, "avg": {}, "min": {}, "max": {}, "percentiles": {"n": {50: 25}},
    }

    # A key reading a place renders it, a window's lines numbered as the input's.
    assert trex.Pattern(r"\N").count_by("${line}", "a\nb 1\nc 2 3\n") == [("2", 1), ("3", 2)]
    assert trex.Pattern(r"\N").count_by("${line}", "a\nb 1\nc 2 3\n", tail=1) == [("3", 2)]


def test_group_by_refuses_a_column_before_reading():
    p = trex.Pattern(r"\W:w \V:v \N:n")
    text = "x v1.2.3 3"
    with pytest.raises(ValueError, match="sum= over w, which binds a word"):
        p.group_by("${w}", text, sum="w")
    with pytest.raises(ValueError, match="names size, which the pattern does not bind"):
        p.group_by("${w}", text, max="size")
    with pytest.raises(ValueError, match="takes one register"):
        p.group_by("${w}", text, sum="${n}/${n}")
    with pytest.raises(ValueError, match="cannot interpolate a version"):
        p.group_by("${w}", text, percentiles={"v": 50}, percentile_method="linear")
    with pytest.raises(ValueError, match="sum= names"):
        p.group_by("${w}", text, sum=["n", "${n}"])
    with pytest.raises(ValueError, match="a percentile is 0 to 100"):
        p.group_by("${w}", text, percentiles={"n": 101})
    with pytest.raises(ValueError, match="percentile_method takes"):
        p.group_by("${w}", text, percentile_method="median")
    with pytest.raises(ValueError, match="duration_unit takes"):
        p.group_by("${w}", text, duration_unit="h")
    with pytest.raises(ValueError, match="reads no axis"):
        p.group_by("${@kind}", text)
    with pytest.raises(TypeError, match="one of them"):
        p.group_by("${w}")


def test_group_by_reads_files_trees_and_sets(tmp_path):
    (tmp_path / "a.log").write_text("x 1\nx 2\n")
    (tmp_path / "d.bin").write_bytes(b"x 3\0\nx 4\n")
    sub = tmp_path / "sub"
    sub.mkdir()
    (sub / "b.log").write_text("y 5\n")
    p = trex.Pattern(r"\W:k \N:n")

    # A directory is walked into one table; a binary file the walk finds is
    # passed over.
    rows = p.group_by("${k}", path=tmp_path, sum="n")
    assert [(g.key, g.count, g.sum["n"]) for g in rows] == [("x", 2, 3), ("y", 1, 5)]
    # ${path} keys each file, and a list names several, str or PathLike.
    by_file = p.group_by("${path}", path=[tmp_path / "a.log", str(sub / "b.log")])
    assert [(Path(g.key).name, g.count) for g in by_file] == [("a.log", 2), ("b.log", 1)]
    # A window of each file, its lines numbered as the file's.
    assert p.count_by("${k}", "x 1\nx 2\n", tail=1) == [("x", 1)]
    assert [(g.key, g.count) for g in p.group_by("${line}", path=tmp_path / "a.log", tail=1)] == [("2", 1)]

    # A binary file named alone is refused; binary=True reads it.
    with pytest.raises(ValueError, match="d.bin holds a NUL byte and is binary; binary=True reads it"):
        p.group_by("${k}", path=tmp_path / "d.bin")
    assert p.distinct("${k}", path=tmp_path / "d.bin", binary=True) == ["x"]
    assert p.group_by("${k}", path=[tmp_path / "a.log", tmp_path / "d.bin"], binary=True)[0].count == 4
    with pytest.raises(OSError, match="nope.log"):
        p.group_by("${k}", path=tmp_path / "nope.log")

    # A set groups by the member that made each match.
    s = trex.PatternSet([r"\E", r"\I"])
    found = "bob@x.com 10.0.0.1 ann@y.org"
    assert [(g.key, g.count) for g in s.group_by("${pattern}", found)] == [("0", 2), ("1", 1)]
    assert s.distinct("${pattern}", found, order="count") == ["0", "1"]


def test_a_library_declares_what_a_pattern_reads(tmp_path):
    lib = trex.Library()
    lib.shape("ticket", r"[A-Z]{2,4}-\d{1,4}", accepts=["AB-12"], rejects=["A-1"])
    lib.let("pair", r'\W:k "=" \N:v')
    assert lib.names == ["ticket", "pair"]
    assert "ticket" in lib and len(lib) == 2
    assert [(a.name, a.form, a.source) for a in lib] == [("ticket", "shape", "library"), ("pair", "let", "library")]
    assert trex.Pattern(r"\{ticket}", lib=lib).find("see AB-12 now").text == "AB-12"
    assert trex.Pattern(r"\{pair}", lib=lib).find("x a=1 y").text == "a=1"

    # A name declared twice is refused unless replaced, and a refused line
    # leaves the library as it was.
    with pytest.raises(ValueError, match="already declared here; replace=True replaces it"):
        lib.let("pair", r"\N")
    lib.let("pair", r"\N", replace=True)
    with pytest.raises(ValueError):
        lib.declare(r"let bad = \N{>")
    assert lib.names == ["ticket", "pair"]

    # Expectations run as test lines.
    (t,) = lib.test()
    assert (t.name, t.passed, t.accepts, t.rejects, t.failures) == ("ticket", True, ["AB-12"], ["A-1"], [])
    lib.declare('test ticket accepts "A-1"')
    assert [t.passed for t in lib.test()] == [True, False]
    assert lib.names == ["ticket", "pair"]

    # A file is included, its rules listed, and removed with what it declares.
    defs = tmp_path / "defs.trex"
    defs.write_text('let tag = \\W\nrule todo note "a TODO here" = "TODO"\n')
    lib.include(defs)
    assert lib.files == [str(defs.resolve())]
    assert lib.names == ["ticket", "pair", "tag", "todo"]
    (rule,) = lib.rules
    assert (rule.name, rule.severity, rule.message, rule.pattern, rule.fix) == ("todo", "note", "a TODO here", '"TODO"', None)
    assert rule.to_dict()["line"] == 2
    with pytest.raises(ValueError, match="tag is declared by"):
        lib.remove("tag")
    with pytest.raises(ValueError, match="no atom named nope"):
        lib.remove("nope", "pair")
    assert "pair" in lib
    lib.remove(defs, "pair")
    assert lib.names == ["ticket"] and lib.files == []
    assert trex.Library.load(defs).names == ["tag", "todo"]
    lib.clear()
    assert len(lib) == 0

    # The shipped atoms are a read-only listing whose tests pass.
    shipped = trex.Library.shipped()
    names = [a.name for a in shipped]
    assert "weekday" in names and "git_sha" in names
    assert all(a.source == "shipped" and a.description for a in shipped)
    shipped_tests = shipped.test()
    assert shipped_tests and all(t.passed for t in shipped_tests)
    with pytest.raises(TypeError, match="read-only"):
        shipped.let("x", r"\N")


def test_a_directory_reads_as_its_pattern_files(tmp_path):
    atoms = tmp_path / "atoms"
    (atoms / "more").mkdir(parents=True)
    (atoms / "a.trex").write_text("let rhs = \\N | \\Q\n")
    (atoms / "more" / "b.trex").write_text('kind assign = \\W "=" \\{rhs}\n')
    (atoms / "notes.txt").write_text("let ignored = \\N\n")
    lib = trex.Library.load(atoms)
    assert lib.files == [str((atoms / "a.trex").resolve()), str((atoms / "more" / "b.trex").resolve())]
    assert lib.names == ["rhs", "assign"]
    assert trex.Pattern(r"\{assign}", lib=lib).find("let x = 1").text == "x = 1"
    assert trex.Pattern(r"\{assign}", lib=atoms).find("let x = 1").text == "x = 1"
    assert trex.Pattern(r"\{assign}", lib=[str(atoms)]).find("let x = 1").text == "x = 1"

    lib.remove(atoms / "more")
    assert lib.names == ["rhs"]
    lib.remove(str(atoms))
    assert lib.names == [] and lib.files == []
    with pytest.raises(ValueError, match="nothing included here is"):
        lib.remove(atoms)

    empty = tmp_path / "empty"
    empty.mkdir()
    with pytest.raises(ValueError, match="no .trex file under it"):
        lib.include(empty)


# The pattern file and the input of the rules reference page; the findings
# below are checked against that page's command-line report.
RULES = r'''# what a config file may not hold
rule private_ip
  pattern = \{ip_private}:addr
  message = private address ${addr} in ${path:name}
  severity = warning
  fix = ${addr:octet1-2}.x.x
  meta.cwe = CWE-200
  meta.tags = network, config

rule cardnum error "card number ending ${card:last4}" = \{card}:card
fix cardnum = ****
meta cardnum tags = pii
rule todo note "a TODO left in ${path:name}" = "TODO"
test private_ip accepts "10.0.0.5" rejects "8.8.8.8"
'''
CONF = "host = 10.0.0.5\npay 4111 1111 1111 1111 now\n# TODO rotate\n"


def rule_files(tmp_path):
    rules = tmp_path / "rules.trex"
    rules.write_bytes(RULES.encode())
    conf = tmp_path / "app.conf"
    conf.write_bytes(CONF.encode())
    return trex.Library.load(rules), conf


def test_rules_report_findings_as_the_command_line_does(tmp_path):
    lib, conf = rule_files(tmp_path)
    found = lib.check(path=conf)
    assert [(f.rule, f.severity, f.line, f.column, f.message, f.fix) for f in found] == [
        ("private_ip", "warning", 1, 8, "private address 10.0.0.5 in app.conf", "10.0.x.x"),
        ("cardnum", "error", 2, 5, "card number ending 1111", "****"),
        ("todo", "note", 3, 3, "a TODO left in app.conf", None),
    ]
    ip = found[0]
    assert (ip.path, ip.start, ip.end, ip.byte_start, ip.byte_end, ip.text) == (str(conf), 7, 15, 7, 15, "10.0.0.5")
    assert (ip.end_line, ip.end_column, ip.fix_span, ip.captures) == (1, 16, (7, 15), {"addr": "10.0.0.5"})
    assert (ip.definition.name, ip.definition.fix, ip.definition.tags) == (
        "private_ip",
        "${addr:octet1-2}.x.x",
        ["network", "config"],
    )
    assert found[2].fix_span is None
    assert ip.to_dict()["definition"]["name"] == "private_ip"
    assert ip.format("${severity} ${rule} ${line}:${col} ${message}") == (
        "warning private_ip 1:8 private address 10.0.0.5 in app.conf"
    )
    assert ip.github().startswith("::warning file=")
    assert ip.github().endswith(
        "line=1,col=8,endLine=1,endColumn=16,title=private_ip::private address 10.0.0.5 in app.conf"
    )

    objects = json.loads(trex.findings_json(found))
    assert [(o["rule"], o["severity"], o["line"], o["col"], o["start"]) for o in objects] == [
        ("private_ip", "warning", 1, 8, 7),
        ("cardnum", "error", 2, 5, 20),
        ("todo", "note", 3, 3, 46),
    ]
    assert (objects[0]["path"], objects[0]["fix"], objects[0]["meta"]) == (
        str(conf),
        "10.0.x.x",
        {"cwe": "CWE-200", "tags": "network, config"},
    )
    assert "fix" not in objects[2]

    run = json.loads(trex.sarif(found))["runs"][0]
    assert [r["id"] for r in run["tool"]["driver"]["rules"]] == ["private_ip", "cardnum", "todo"]
    assert [(r["ruleId"], r["level"]) for r in run["results"]] == [
        ("private_ip", "warning"),
        ("cardnum", "error"),
        ("todo", "note"),
    ]
    assert run["results"][0]["locations"][0]["physicalLocation"]["region"]["startColumn"] == 8
    assert run["columnKind"] == "unicodeCodePoints"
    # A run with no finding describes the rules it checked where lib= names them.
    clean = json.loads(trex.sarif([], lib=lib))["runs"][0]
    assert [r["id"] for r in clean["tool"]["driver"]["rules"]] == ["private_ip", "cardnum", "todo"]
    assert clean["results"] == []
    assert json.loads(trex.sarif([]))["runs"][0]["tool"]["driver"]["rules"] == []

    # Text and bytes are read as the file is; bytes hand their texts back as bytes.
    assert [(f.rule, f.path, f.line) for f in lib.check(CONF)] == [
        ("private_ip", None, 1),
        ("cardnum", None, 2),
        ("todo", None, 3),
    ]
    assert lib.check(CONF.encode())[0].text == b"10.0.0.5"
    assert [f.rule for f in lib.check(CONF, max_count=1)] == ["private_ip"]
    # A directory is walked; globs keep the configuration and drop the rules file.
    assert [f.rule for f in lib.check(path=tmp_path, globs=["*.conf"])] == ["private_ip", "cardnum", "todo"]

    with pytest.raises(TypeError, match="one of them"):
        lib.check(CONF, path=conf)
    with pytest.raises(ValueError, match="no directory"):
        lib.check(CONF, globs=["*.conf"])
    with pytest.raises(ValueError, match="no rule is declared here"):
        trex.Library().check(CONF)
    binary = tmp_path / "x.bin"
    binary.write_bytes(b"TODO\0")
    with pytest.raises(ValueError, match="holds a NUL byte"):
        lib.check(path=binary)
    assert [f.rule for f in lib.check(path=binary, binary=True)] == ["todo"]


def test_a_finding_in_a_tail_is_at_the_file_s_own_place(tmp_path):
    lib = trex.Library()
    lib.declare('rule todo note "a TODO" = "TODO"')
    text = "café 中\n\U0001F600 two\n# TODO rotate\n"
    log = tmp_path / "app.log"
    log.write_bytes(text.encode())
    (f,) = lib.check(path=log, tail=1)
    assert (f.line, f.column) == (3, 3)
    assert text[f.start:f.end] == "TODO"
    assert f.byte_start == text.encode().index(b"TODO")
    # The same text in memory, and in UTF-16 behind its mark, is at the same place.
    (g,) = lib.check(text, tail=1)
    assert (g.start, g.end, g.line) == (f.start, f.end, f.line)
    wide = tmp_path / "wide.log"
    wide.write_bytes(b"\xff\xfe" + text.encode("utf-16-le"))
    (h,) = lib.check(path=wide, tail=1)
    assert (h.start, h.end, h.line, h.column) == (f.start, f.end, 3, 3)


def test_fixes_are_written_or_described_as_the_command_line_writes_them(tmp_path):
    lib, conf = rule_files(tmp_path)
    diff = lib.fix(conf, dry_run=True)
    assert "-host = 10.0.0.5" in diff and "+host = 10.0.x.x" in diff and "+pay **** now" in diff
    assert conf.read_bytes().decode() == CONF
    assert lib.fix(conf) == 2
    assert conf.read_bytes().decode() == "host = 10.0.x.x\npay **** now\n# TODO rotate\n"

    # A fix overlapping an earlier one is skipped, and said so.
    over = trex.Library()
    over.declare(r'rule one note "one" = \N:n')
    over.declare("fix one = <${n}>")
    over.declare(r'rule pair note "pair" = \N \N')
    over.declare("fix pair = PAIR")
    nums = tmp_path / "nums.txt"
    nums.write_bytes(b"1 2\n")
    with pytest.warns(trex.TrexWarning, match="1:1: the fix of pair overlaps an earlier fix; skipped"):
        assert over.fix(nums) == 2
    assert nums.read_bytes() == b"<1> <2>\n"


def test_a_review_decides_each_fix_before_any_is_written(tmp_path):
    lib = trex.Library()
    lib.declare(r'rule num note "a number" = \N:n')
    lib.declare("fix num = <${n}>")
    nums = tmp_path / "nums.txt"
    original = b"a 1\nb 2\nc 3\n"

    nums.write_bytes(original)
    asked = []

    def once_for_the_template(change):
        asked.append(change)
        return "template"

    assert lib.fix(nums, review=once_for_the_template) == 3
    assert nums.read_bytes() == b"a <1>\nb <2>\nc <3>\n"
    (c,) = asked
    assert (c.path, c.start, c.end, c.text, c.replacement) == (str(nums), 2, 3, "1", "<1>")
    assert (c.template, c.later, c.index, c.total) == ("number", 2, 1, 3)
    assert "-a 1" in c.diff and "+a <1>" in c.diff

    nums.write_bytes(original)
    answers = iter([False, "two", True])
    assert lib.fix(nums, review=lambda change: next(answers)) == 2
    assert nums.read_bytes() == b"a 1\nb two\nc <3>\n"

    nums.write_bytes(original)
    with pytest.warns(trex.TrexWarning) as told:
        assert lib.fix(nums, review=lambda change: "skip template", show_skipped=True) == 0
    assert [str(w.message) for w in told] == [
        "3 changes skipped by a template answer",
        f"  {nums} [2..3]",
        f"  {nums} [6..7]",
        f"  {nums} [10..11]",
    ]
    assert nums.read_bytes() == original

    answers = iter([True, "quit"])
    with pytest.warns(trex.TrexWarning, match="quit; every file is left as it was"):
        assert lib.fix(nums, review=lambda change: next(answers)) == 0
    assert nums.read_bytes() == original

    answers = iter([False, "all"])
    assert lib.fix(nums, review=lambda change: next(answers)) == 2
    assert nums.read_bytes() == b"a 1\nb <2>\nc <3>\n"

    nums.write_bytes(original)
    with pytest.raises(TypeError, match="review= returns True, False"):
        lib.fix(nums, review=lambda change: None)
    assert nums.read_bytes() == original
    with pytest.raises(ValueError, match="takes no dry_run=True"):
        lib.fix(nums, review=lambda change: True, dry_run=True)
    with pytest.raises(ValueError, match="only review= gives"):
        lib.fix(nums, show_skipped=True)


def test_a_follow_yields_each_finding_as_the_file_grows(tmp_path):
    lib = trex.Library()
    lib.declare('rule todo note "a TODO" = "TODO"')
    log = tmp_path / "app.log"
    log.write_bytes("café\n# TODO one\n".encode())
    follow = lib.follow(log, max_count=2)
    first = next(follow)
    assert (first.line, first.text, first.path) == (2, "TODO", str(log))
    with log.open("ab") as grown:
        grown.write("# TODO two 中\n".encode())
    second = next(follow)
    whole = log.read_bytes().decode()
    assert (second.line, second.start, second.end) == (3, whole.rindex("TODO"), whole.rindex("TODO") + 4)
    # Two from its one file is all max_count lets the follow give.
    with pytest.raises(StopIteration):
        next(follow)
    with pytest.raises(ValueError, match="ends before the file does"):
        lib.follow(log, lines="1..2")


def test_a_truncated_followed_file_starts_its_count_again_unless_kept(tmp_path):
    lib = trex.Library()
    lib.declare(r'rule num note "n" = \N')
    a = tmp_path / "a.log"
    b = tmp_path / "b.log"
    for keep in (False, True):
        a.write_bytes(b"x 11\n")
        b.write_bytes(b"none\n")
        follow = lib.follow([a, b], max_count=1, keep_count=keep)
        assert next(follow).text == "11"
        a.write_bytes(b"y 2\n")
        with b.open("ab") as grown:
            grown.write(b"z 3\n")
        with pytest.warns(trex.TrexWarning, match="a.log: truncated"):
            found = next(follow)
        if keep:
            # The truncated file's count is kept, so only the other file gives.
            assert (found.path, found.text) == (str(b), "3")
            with pytest.raises(StopIteration):
                next(follow)
        else:
            # The file now under the name is a new input, and gives its own.
            assert (found.path, found.text) == (str(a), "2")
    with pytest.raises(ValueError, match="no max_count was given"):
        lib.follow(a, keep_count=True)


def test_follow_yields_each_match_as_the_file_grows(tmp_path):
    log = tmp_path / "app.log"
    log.write_bytes("café 7\n".encode())
    # From the end, what the file holds now is passed over.
    follow = trex.follow(r"\N", log, max_count=1)
    with log.open("ab") as grown:
        grown.write("中 42\n".encode())
    found = next(follow)
    whole = log.read_bytes().decode()
    assert (found.text, found.path, found.line, found.column) == ("42", str(log), 2, 3)
    assert (found.start, found.byte_start) == (whole.rindex("42"), whole.encode().rindex(b"42"))
    with pytest.raises(StopIteration):
        next(follow)
    # From the start, what it holds is scanned first.
    assert next(trex.follow(trex.Pattern(r"\N"), log, from_end=False, max_count=1)).text == "7"
    with pytest.raises(ValueError, match="no max_count was given"):
        trex.follow(r"\N", log, keep_count=True)
    with pytest.raises(ValueError, match="read under its own"):
        trex.follow(trex.Pattern(r"\N"), log, lib=trex.Library())
    with pytest.raises(TypeError, match="none was given"):
        trex.follow(r"\N")


def test_a_truncated_file_a_pattern_follows_starts_its_count_again_unless_kept(tmp_path):
    a = tmp_path / "a.log"
    b = tmp_path / "b.log"
    for keep in (False, True):
        a.write_bytes(b"x 11\n")
        b.write_bytes(b"none\n")
        follow = trex.follow(r"\N", a, b, from_end=False, max_count=1, keep_count=keep)
        assert next(follow).text == "11"
        a.write_bytes(b"y 2\n")
        with b.open("ab") as grown:
            grown.write(b"z 3\n")
        with pytest.warns(trex.TrexWarning, match="a.log: truncated"):
            found = next(follow)
        if keep:
            assert (found.path, found.text) == (str(b), "3")
            with pytest.raises(StopIteration):
                next(follow)
        else:
            assert (found.path, found.text) == (str(a), "2")


ARITH = (
    'expr   := <expr> "+" <term> | <expr> "-" <term> | <term>\n'
    'term   := <term> "*" <factor> | <term> "/" <factor> | <factor>\n'
    'factor := number | ident | "(" <expr> ")"'
)


def test_a_grammar_parses_counts_and_lists_tilings(tmp_path):
    arith = trex.Grammar(ARITH)
    assert (arith.start, arith.rules) == ("expr", ["expr", "term", "factor"])
    tree = arith.parse("2 + 3 * 4")
    assert (tree.expression, tree.rule, tree.text, tree.path) == ("(expr 2 + (term 3 * 4))", "expr", "2 + 3 * 4", None)
    assert [c.text for c in tree.children] == ["2", "+", "3 * 4"]
    assert tree.children[1].rule is None
    assert arith.parse("2 - 3 - 4").expression == "(expr (expr 2 - 3) - 4)"
    assert arith.parse("2 +") is None
    assert (arith.accepts("2 + 3"), arith.accepts("2 +")) == (True, False)
    assert trex.Grammar(ARITH, start="term").start == "term"
    # An ambiguous grammar's derivations are the Catalan numbers.
    ambiguous = trex.Grammar('expr := <expr> "+" <expr> | number')
    assert [ambiguous.count(t) for t in ["1", "1 + 2 + 3", "1 + 2 + 3 + 4"]] == [1, 2, 5]
    weighted = trex.Grammar('expr := <expr> "+" <expr> @0.5 | number @0.5')
    assert (weighted.best("1 + 2 + 3"), weighted.probability("1 + 2 + 3")) == (0.03125, 0.0625)
    # A file's tree carries its path.
    source = tmp_path / "sum.txt"
    source.write_bytes(b"2 + 3 * 4")
    assert arith.parse(path=source).path == str(source)
    assert arith.count(path=source) == 1

    bracketed = trex.Grammar('s := <s> <s> | <w>\nw := "ab" @0.5 | "a" @0.25 | "b" @0.25')
    words = ["a", "b", "ab"]
    tilings = bracketed.segment("abab", words)
    assert [("-".join(t.words), t.parses, t.probability) for t in tilings] == [
        ("ab-ab", 1, 0.25),
        ("a-b-ab", 2, 0.03125),
        ("ab-a-b", 2, 0.03125),
        ("a-b-a-b", 5, 0.00390625),
    ]
    assert sum(t.parses for t in tilings) == bracketed.count_segmentations("abab", words)
    assert tilings[0].probability == bracketed.best_segmentation_probability("abab", words)
    assert bracketed.segment("abc", words) == []

    with pytest.raises(ValueError, match="grammar error at line 1"):
        trex.Grammar("nonsense")
    with pytest.raises(ValueError, match="declares no rule"):
        trex.Grammar("s := number", start="nope")


def test_a_bpe_learns_encodes_saves_and_loads(tmp_path):
    corpus = "low lower lowest newer newest wider " * 20
    bpe = trex.Bpe.train(corpus, merges=10)
    assert bpe.merges == len(bpe) == 10
    pieces = bpe.encode("lowest")
    assert "".join(pieces).replace("</w>", "") == "lowest" and pieces[-1].endswith("</w>")
    model = tmp_path / "corpus.bpe"
    bpe.save(model)
    again = trex.Bpe.load(model)
    assert (again.model, again.encode("newest")) == (bpe.model, bpe.encode("newest"))
    assert trex.Bpe(bpe.model).merges == 10
    text = tmp_path / "corpus.txt"
    text.write_bytes(corpus.encode())
    assert trex.Bpe.train(path=text, merges=10).model == bpe.model
    assert bpe.encode(path=text)[:3] == bpe.encode("low lower")[:3]
    with pytest.raises(ValueError):
        trex.Bpe("not a merge line")


def test_an_index_rules_out_the_files_a_pattern_cannot_match(tmp_path):
    tree = tmp_path / "logs"
    tree.mkdir()
    (tree / "a.log").write_bytes(b"error 404 at 10.0.0.1\n")
    (tree / "b.log").write_bytes(b"all quiet here\n")
    index = trex.Index.build(tree)
    assert (index.root, index.path, index.files, len(index)) == (str(tree), str(tree / ".trex-index"), 2, 2)
    assert trex.Index.load(tree).files == 2
    # A file holding no number cannot match one; both hold words.
    assert [Path(c).name for c in index.candidates(r"\N")] == ["a.log"]
    assert [Path(c).name for c in index.candidates(trex.Pattern(r"\W"))] == ["a.log", "b.log"]
    # A file changed since it was indexed is always a candidate.
    (tree / "b.log").write_bytes(b"now 42 here\n")
    assert [Path(c).name for c in index.candidates(r"\N")] == ["a.log", "b.log"]
    with pytest.raises(FileNotFoundError, match="holds no index"):
        trex.Index.load(tmp_path)
    with pytest.raises(ValueError, match="is not a directory"):
        trex.Index.build(tree / "a.log")


def test_a_prefilter_answers_and_keeps_its_contract(tmp_path):
    corpus = "GET /index.html 200\nPOST /login 302\nERROR timeout at db\n" * 10
    pf = trex.Prefilter(corpus, kind="xor")
    assert (pf.filter, pf.bytes) == ("xor", len(corpus.encode()))
    assert pf.might_contain("ERROR") and pf.might_contain(b"login")
    tests = pf.test("ERROR", "CRITICAL", b"timeout")
    assert [(t.literal, t.filter, t.occurs) for t in tests] == [
        ("ERROR", "xor", True),
        ("CRITICAL", "xor", False),
        (b"timeout", "xor", True),
    ]
    assert tests[0].might_occur and tests[2].might_occur
    # A filter never reports a literal that occurs as absent.
    checks = pf.verify()
    assert [c.filter for c in checks] == ["bloom", "cuckoo", "xor"]
    assert all(c.false_negatives == 0 and c.present_probes > 0 for c in checks)
    log = tmp_path / "access.log"
    log.write_bytes(corpus.encode())
    assert trex.Prefilter(path=log).might_contain("POST")
    with pytest.raises(ValueError, match="kind takes"):
        trex.Prefilter(corpus, kind="nope")


def test_tokens_read_as_the_lexer_reads_them(tmp_path):
    toks = trex.tokens("retry 3 times in 1500ms")
    assert [(t.kind, t.start, t.end, t.text) for t in toks] == [
        ("word", 0, 5, "retry"),
        ("number", 6, 7, "3"),
        ("word", 8, 13, "times"),
        ("word", 14, 16, "in"),
        ("duration", 17, 23, "1500ms"),
    ]
    assert (toks[1].value, toks[4].value, toks[0].value) == (3, 1_500_000_000, None)
    assert [t.kind for t in trex.tokens("a 1", whitespace=True)] == ["word", "whitespace", "number"]
    # Offsets count characters in a str and bytes in bytes.
    assert [(t.start, t.end) for t in trex.tokens("é 1")] == [(0, 1), (2, 3)]
    assert [(t.start, t.end) for t in trex.tokens("é 1".encode())] == [(0, 2), (3, 4)]

    lib = trex.Library()
    lib.shape("ticket", r"[A-Z]{2,4}-\d{1,4}")
    assert [t.kind for t in trex.tokens("see XYZ-9 now", lib=lib)] == ["word", "ticket", "word"]
    assert repr(trex.tokens("x 3")) == "[Token(kind='word', start=0, end=1, text='x'), Token(kind='number', start=2, end=3, text='3')]"
    assert repr(lib) == "Library(names=['ticket'])"
    with pytest.raises(ValueError, match="is not a byte-pattern escape"):
        trex.Library().shape("limb", r"\x00{4}")
    with pytest.raises(ValueError, match="class takes no backslash"):
        trex.Library().declare(r"shape limb = `[\x00-\xff]{4}`")
    with pytest.raises(ValueError, match="is not byte-pattern syntax"):
        trex.Pattern("`(?s:.)`")

    bin_file = tmp_path / "d.bin"
    bin_file.write_bytes(b"x 3\0\n")
    with pytest.raises(ValueError, match="holds a NUL byte and is binary; binary=True reads it"):
        trex.tokens(path=bin_file)
    assert [t.kind for t in trex.tokens(path=bin_file, binary=True)][:2] == ["word", "number"]
    text_file = tmp_path / "a.log"
    text_file.write_text("x 3\n")
    assert [t.text for t in trex.tokens(path=text_file)] == ["x", "3"]
    with pytest.raises(TypeError, match="one of them"):
        trex.tokens()


def test_the_token_axes_read_magnitude_nesting_and_trend():
    m = trex.axes.magnitude("size 12 then 12000000 then 3")
    assert (m.path, m.length, m.tokens) == (None, 28, 6)
    assert (m.peak.text, m.peak.offset) == ("12000000", 13)
    assert m.peak.magnitude == pytest.approx(7.0792, abs=1e-4)
    # A jump is a change of at least three orders from the token before.
    assert [(f.text, f.offset) for f in m.jumps] == [("12000000", 13), ("then", 22)]
    assert m.frames == []
    assert [f.text for f in trex.axes.magnitude("a 1", detail=True).frames] == ["a", "1"]

    s = trex.axes.stress('{"a": [1, {"b": [2, 3]}]}')
    assert (s.max_depth, s.peak_load.text) == (4, "]")
    assert [(f.text, f.depth) for f in s.peaks] == [("1", 2), ('"b"', 3), ("2", 4), ("3", 4)]
    assert [(f.text, f.offset) for f in s.fractures] == [("]", 21)]
    # An unpaired bracket opens no level.
    assert (trex.axes.stress("(x").max_depth, trex.axes.stress("(x)").max_depth) == (0, 1)

    f = trex.axes.flow("1 10 100 1000 10 1")
    assert (f.signal, f.peak_momentum.direction) == ("magnitude", "rising")
    assert [(r.text, r.direction) for r in f.reversals] == [("1", "falling")]
    lengths = trex.axes.flow("a bb ccc dddd", signal="length", detail=True)
    assert [r.direction for r in lengths.frames] == ["steady", "rising", "rising", "rising"]
    with pytest.raises(ValueError, match="signal takes 'magnitude', 'stress' or 'length'"):
        trex.axes.flow("x", signal="speed")


def test_the_relation_axis_reads_the_graph_and_its_canonical_form():
    # Two texts that differ only in the names their brackets bind read alike.
    a = trex.axes.relation("[ x ( x ) ]", canonical=True)
    b = trex.axes.relation("[ y ( y ) ]", canonical=True)
    assert a.canonical == b.canonical == "[ # ( ^0 ) ]"
    assert trex.axes.relation("[ x ( x ) ]").canonical is None
    # The reuse of x one level deeper is the holonomy a tree does not have.
    assert (a.max_depth, a.reuse_chords, a.holonomy, a.net_holonomy, a.boundary_closed) == (2, 1, 1, 1, True)
    r = trex.axes.relation("f(x) [y (y)]", detail=True)
    assert [(e.kind, e.from_, e.to) for e in r.edges[:3]] == [
        ("encloses", "f", "x"),
        ("encloses", "y", "y"),
        ("adjacent", "f", "("),
    ]
    assert [(c.from_, c.to, c.residual) for c in r.chords] == [("y", "y", 1)]
    assert (r.frames[0].text, r.frames[0].depth, r.frames[0].enclosure) == ("x", 1, ["f"])
    graph = trex.axes.relation("let x = f(y); g(x, [z (z)])")
    assert graph.cycle_rank == graph.graph_edges - graph.nodes + graph.components
    assert graph.euler == graph.nodes - graph.graph_edges
    assert graph.minimal_cut is not None and graph.minimal_cut <= graph.peak_entanglement


def test_the_recurrence_axes_read_echoes_orbits_and_templates():
    e = trex.axes.echo("GET /a GET /b POST /a GET /c", detail=True)
    assert (e.group, e.keyed, e.distinct, e.novel, e.echoed) == ("identity", 8, 5, 5, 5)
    assert [(x.key, x.count, x.offset) for x in e.echoes] == [("GET", 3, 0), ("a", 2, 5)]
    # Each keyed token's frame points back at its key's previous occurrence.
    gets = [(f.nth, f.previous) for f in e.frames if f.text == "GET"]
    assert gets == [(1, None), (2, 0), (3, 7)]
    case = trex.axes.echo("Error ERROR error warn", group="case")
    assert [(x.key, x.count) for x in case.echoes] == [("Error", 3)]
    records = trex.axes.echo("alpha: one\nbravo: two\ncharlie: three\ndelta: four\n", structure=True)
    assert [(x.key, x.count) for x in records.structures] == [("kv:W:W", 4)]
    with pytest.raises(ValueError, match="names no group"):
        trex.axes.echo("x", group="color")

    o = trex.axes.orbit("Cat cat CAT dog", group="case", same_as="CAT", detail=True)
    assert (o.tokens, o.forms, o.orbits) == (4, 4, 2)
    assert [(c.orbit, c.forms) for c in o.classes] == [("cat", ["CAT", "Cat", "cat"]), ("dog", ["dog"])]
    assert [(t.text, t.offset, t.length) for t in o.matches] == [("Cat", 0, 3), ("cat", 4, 3), ("CAT", 8, 3)]
    assert [s.text for s in o.segments] == ["Cat", " ", "cat", " ", "CAT", " ", "dog"]
    shapes = trex.axes.orbit("cat dog bat")
    assert (shapes.group, [(c.orbit, c.forms) for c in shapes.classes]) == ("shape", [("CVC", ["bat", "cat", "dog"])])

    sh = trex.axes.shape("id=1 ok\nid=2 ok\nid=3 fail\nid=4 ok", detail=True)
    assert (sh.group, sh.dominant_period, sh.dominant_strength) == (None, 4, 1.0)
    assert [(r.offset, r.length, r.period) for r in sh.regions] == [(16, 17, 4)]
    # Tokens of one shape share a class.
    words = [f for f in sh.frames if f.text in ("id", "ok", "=")]
    assert words[0].class_ == words[2].class_ != words[1].class_
    # The dominant strength is the strongest frame's.
    assert max(f.period_strength for f in sh.frames if f.period > 0) == sh.dominant_strength
    assert trex.axes.shape("id=1 ok", group="case").group == "case"


def test_the_pair_field_reads_strain_bound_and_classes():
    g = trex.axes.gravity("let x = f(a) ; let y = g(b) ; print x", detail=True)
    assert (g.grain, g.units, len(g.frames)) == ("token", 18, 18)
    # The first unit has nothing before it and no cut before it.
    first = g.frames[0]
    assert (first.text, first.strain, first.strain_percentile, first.bound, first.bound_percentile) == (
        "let",
        None,
        None,
        None,
        None,
    )
    assert all(f.strain is not None and f.bound is not None for f in g.frames[1:])
    assert [f.strain for f in g.strained] == sorted((f.strain for f in g.strained), reverse=True)
    assert [f.bound for f in g.weakest] == sorted(f.bound for f in g.weakest)
    assert (g.strained[0].offset, g.weakest[0].offset) == (4, 36)
    assert len(trex.axes.gravity("let x = f(a) ; let y = g(b) ; print x", top=3).strained) == 3
    assert sorted(g.frames[1].to_dict()) == [
        "bound",
        "bound_percentile",
        "class_",
        "offset",
        "strain",
        "strain_percentile",
        "text",
    ]
    # A byte inside a character takes the character's offset, and the byte grain
    # reads no declarations.
    assert [(f.offset, f.text) for f in trex.axes.gravity("éa", grain="byte", detail=True).frames] == [
        (0, "é"),
        (0, ""),
        (1, "a"),
    ]
    with pytest.raises(ValueError, match="byte grain reads bytes"):
        trex.axes.gravity("x", grain="byte", lib=trex.Library())


def test_the_context_axis_reads_each_token_against_its_fold():
    text = "latency = 100 ; latency = 120 ; size = 5000000 ; latency = 90 ; latency = 12000 ;"
    c = trex.axes.context(text, fold="key", detail=True)
    assert (c.fold, c.tokens, c.units, c.period, c.live_periods) == ("key", 20, 5, 4, [4, 8])
    # The last `latency` holds the three values its key was bound to before,
    # the baseline \N{>+1:k} compares 12000 with.
    last = [f for f in c.frames if f.text == "latency"][-1]
    assert (last.offset, last.over) == (64, 3)
    assert (round(last.magnitude.mean, 2), round(last.magnitude.spread, 2)) == (2.01, 0.05)
    assert last.flow.falling + last.flow.rising + last.flow.steady == 3
    # A context holding nothing reads no axis.
    first = c.frames[0]
    assert (first.over, first.magnitude, first.flow) == (0, None, None)
    assert [a.role for a in c.agreement] == ["assign"] * 5
    window = trex.axes.context(text, detail=True)
    assert (window.fold, window.frames[0].over, window.frames[1].over) == ("window", 0, 1)
    assert trex.axes.context(text).frames == []
    with pytest.raises(ValueError, match="fold takes"):
        trex.axes.context(text, fold="windows")
    with pytest.raises(ValueError, match="at least one unit"):
        trex.axes.context(text, token_window=0)


def test_the_byte_axes_read_vantage_texture_and_seams():
    o = trex.axes.observation("key=value; key=other; odd")
    # Byte 19 reads 0.21 and byte 21, two bytes on, 0.47: of two contested
    # points closer than the gap the stronger is kept.
    assert [c.offset for c in o.contested] == [2, 21]
    # The two read the same disagreement, the input repeating `key=`, and the
    # peak is the later of points that tie.
    assert o.peak.offset == 21 and o.peak.disagreement == max(c.disagreement for c in o.contested)
    # Every byte has a frame, and a byte inside a character takes the
    # character's offset.
    assert [f.offset for f in trex.axes.observation("éa", detail=True).frames] == [0, 0, 1]

    table = "\n".join(f"row {i}, value {i * 7}" for i in range(1, 41))
    sp = trex.axes.spectral(table, classify=True)
    assert (sp.hop, sp.length) == (16, len(table))
    assert [(p.period, round(p.strength, 2)) for p in sp.periods] == [(18, 0.82)]
    assert [(r.kind, r.offset, r.length) for r in sp.regions] == [("table", 0, len(table))]
    assert [r.texture for r in sp.timeline] == ["mixed"]
    assert 0 <= sp.entropy_minimum <= sp.entropy_mean <= sp.entropy_maximum <= 1
    empty = trex.axes.spectral("")
    assert (empty.entropy_mean, empty.timeline, empty.frames) == (None, [], [])
    # A frame whose last byte is inside a character takes the
    # character's offset: bytes 15, 31 and 35 of twelve three-byte characters.
    assert [f.offset for f in trex.axes.spectral("我" * 12, detail=True).frames] == [5, 10, 11]

    se = trex.axes.seam("thequickbrownfoxjumpsoverthelazydog", english=True)
    assert [s.text for s in se.segments] == ["the", "quick", "brown", "fox", "jumps", "over", "the", "lazy", "dog"]
    assert se.order == 3
    assert [(f.offset, f.text) for f in trex.axes.seam("éa", detail=True).frames] == [(0, "é"), (0, ""), (1, "a")]
    assert [f.text for f in trex.axes.seam(b"ab", detail=True).frames] == [b"a", b"b"]
    with pytest.raises(ValueError, match="order= is a context length"):
        trex.axes.seam("x", order=0)
    # The byte axes read no declarations.
    with pytest.raises(TypeError):
        trex.axes.seam("x", lib=trex.Library())


def test_an_axis_reads_files_libraries_and_bytes_and_gives_plain_dicts(tmp_path):
    # A file is decoded as a scan reads one, and its offsets count the
    # characters of its text.
    wide = tmp_path / "wide.log"
    wide.write_bytes("café 12000000\n".encode("utf-16"))
    m = trex.axes.magnitude(path=wide)
    assert (m.path, m.length, m.peak.offset, m.peak.text) == (str(wide), 14, 5, "12000000")
    # Bytes are read and reported as bytes.
    b = trex.axes.magnitude("café 12000000".encode())
    assert (b.length, b.peak.offset, b.peak.text) == (14, 6, b"12000000")
    binary = tmp_path / "x.bin"
    binary.write_bytes(b"1 2\0")
    with pytest.raises(ValueError, match="holds a NUL byte"):
        trex.axes.stress(path=binary)
    with pytest.raises(TypeError, match="one of them"):
        trex.axes.flow()

    # A declared shape is one token of its own.
    lib = trex.Library()
    lib.shape("ticket", r"[A-Z]{2,4}-\d{1,4}")
    assert trex.axes.orbit("see XYZ-9 now", lib=lib).tokens == 3
    assert trex.axes.orbit("see XYZ-9 now").tokens > 3

    d = trex.axes.relation("f(x) [y (y)]", detail=True).to_dict()
    assert d["edges"][0] == {"kind": "encloses", "from_": "f", "to": "x", "from_offset": 0, "to_offset": 2}
    assert json.loads(json.dumps(d)) == d
    frame = trex.axes.shape("id ok", detail=True).frames[0].to_dict()
    assert sorted(frame) == ["class_", "novelty", "offset", "period", "period_strength", "text"]
    assert repr(trex.axes.orbit("a")) == "OrbitReport(path=None, group='shape', tokens=1, forms=1, orbits=1)"
    assert repr(trex.axes.seam("ab").segments[0]) == "Segment(offset=0, length=2, text='ab')"
    assert trex.axes.__name__ == "trex.axes" and type(m).__module__ == "trex.axes"


def stub_parameters(node):
    """A stub function's positional names, its `*args` name and its keyword-only names."""
    a = node.args
    positional = [x.arg for x in a.posonlyargs + a.args if x.arg not in ("self", "cls")]
    return positional, [a.vararg.arg] if a.vararg else [], {x.arg for x in a.kwonlyargs}


def runtime_parameters(obj):
    """The same three of a runtime callable, from its signature."""
    kinds = inspect.Parameter
    params = [p for p in inspect.signature(obj).parameters.values() if p.name != "self"]
    positional = [p.name for p in params if p.kind in (kinds.POSITIONAL_ONLY, kinds.POSITIONAL_OR_KEYWORD)]
    star = [p.name for p in params if p.kind == kinds.VAR_POSITIONAL]
    return positional, star, {p.name for p in params if p.kind == kinds.KEYWORD_ONLY}


def merged(overloads):
    """The parameters of a function's overloads together, as the runtime takes them all."""
    positional, star, keywords = [], [], set()
    for node in overloads:
        p, s, k = stub_parameters(node)
        positional += [name for name in p if name not in positional]
        star = star or s
        keywords |= k
    return positional, star, keywords


def test_the_stubs_name_every_class_function_and_parameter():
    stubs = Path(trex.__file__).parent
    assert (stubs / "py.typed").is_file()
    protocols = {"__iter__", "__next__", "__len__", "__getitem__", "__contains__"}
    for module, stub in ((trex.trex, "trex.pyi"), (trex.axes, "axes.pyi")):
        tree = ast.parse((stubs / stub).read_text(encoding="utf-8"))
        functions, classes = {}, {}
        for node in tree.body:
            if isinstance(node, ast.FunctionDef):
                functions.setdefault(node.name, []).append(node)
            elif isinstance(node, ast.ClassDef):
                classes[node.name] = {n.name: n for n in node.body if isinstance(n, ast.FunctionDef)}
        runtime = {n for n in vars(module) if not n.startswith("_")} - {"trex", "axes"}
        assert set(functions) | set(classes) == runtime, sorted(runtime ^ (set(functions) | set(classes)))
        for name, overloads in functions.items():
            assert merged(overloads) == runtime_parameters(getattr(module, name)), name
        for name, members in classes.items():
            cls = getattr(module, name)
            public = {m for m in vars(cls) if not m.startswith("_") or m in protocols}
            assert set(members) - {"__new__"} == public, (name, sorted(public ^ (set(members) - {"__new__"})))
            assert ("__new__" in members) == ("__new__" in vars(cls)), name
            if "__new__" in members:
                assert stub_parameters(members["__new__"]) == runtime_parameters(cls), name
            for member, node in members.items():
                if member.startswith("_") or any(getattr(d, "id", "") == "property" for d in node.decorator_list):
                    continue
                assert stub_parameters(node) == runtime_parameters(getattr(cls, member)), f"{name}.{member}"
    assert "__version__" in (stubs / "trex.pyi").read_text(encoding="utf-8")
    # The stub's __all__ lists what the native module's does.
    native = ast.parse((stubs / "trex.pyi").read_text(encoding="utf-8"))
    listed = next(
        ast.literal_eval(n.value)
        for n in native.body
        if isinstance(n, ast.Assign) and any(getattr(t, "id", "") == "__all__" for t in n.targets)
    )
    assert sorted(listed) == sorted(trex.trex.__all__)
    # The package's stub re-exports the native module's, as the package does.
    package = (stubs / "__init__.pyi").read_text(encoding="utf-8")
    assert "from .trex import *" in package and "__version__" in package and "axes" in package


def test_errors_and_module_facts():
    with pytest.raises(ValueError, match="pattern error at byte"):
        trex.Pattern(r"\N{>x}")
    with pytest.raises(ValueError, match="template error"):
        trex.Pattern(r"\N").rewrite("${nope}", "1")
    with pytest.raises(TypeError):
        trex.Pattern(r"\N").scan(3)
    with pytest.raises(KeyError):
        trex.Pattern(r"\N").find("1")["x"]
    assert trex.parse(r"\N").is_match("7")
    assert trex.Pattern(trex.escape("x.y")).is_match("say x.y now")
    assert trex.version() == trex.__version__
    assert isinstance(trex.device_available(), bool)
