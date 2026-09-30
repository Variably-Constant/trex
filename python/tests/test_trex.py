"""The trex module through its public surface: typed tokens and binding,
offsets in the input's units, rewrites, splits, sets, streams and errors."""

import datetime
from decimal import Decimal

import pytest

import trex


def test_a_typed_register_reports_its_value_in_python_types():
    # A byte size in bytes and a duration in seconds, on the owner's units.
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

    # Templates: the lines that share a shape, most lines first, with the
    # spelling that reads as the line did and the pattern that matches it.
    ts = trex.templates(log)
    assert ts, "a log of three lines has at least one template"
    assert sum(t["count"] for t in ts) == ts[0]["covered"]
    assert all({"count", "records", "readable", "pattern", "rare"} <= set(t) for t in ts)
    # A cut the caller names decides what counts as rare; absent, it is the
    # mean template's coverage, read from the input rather than chosen.
    rare = trex.templates(log, rare_under="1")
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

    # explain: why a match matched, as a dict. The input comes back because
    # holding it on every match would copy the whole of it per scan.
    m = p.find(log)
    why = m.explain(log)
    assert {"tokens", "guards", "readings", "route"} <= set(why)
    assert any(kind == "word" for kind, _ in why["tokens"])
    assert isinstance(why["route"], str) and why["route"]


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
    # split, so it owes the same limit contract: a limit of one leaves the
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

    # With no field, infer returns the pattern string as it always has.
    assert trex.infer(["code 200", "code 204"]) == '"code" \\N'
    with pytest.raises(ValueError, match="more than once"):
        trex.infer(["a 1 b 1"], fields={"n": "1"})
    with pytest.raises(TypeError, match="a str or a list of str"):
        trex.infer(["a 1 b"], fields={"n": 1})


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
