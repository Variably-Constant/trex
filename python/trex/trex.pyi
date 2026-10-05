import os
from collections.abc import Callable, Iterable, Iterator, Mapping, Sequence
from datetime import timedelta
from typing import Any, Literal, TypeVar, final, overload

from . import axes as axes

_T = TypeVar("_T", str, bytes)
_Text = str | bytes
_Path = str | os.PathLike[str]
_Paths = _Path | Sequence[_Path]
_Lib = Library | _Path | Sequence[_Path]
_Severity = Literal["error", "warning", "note"]
_Filter = Literal["bloom", "cuckoo", "xor"]
_Order = Literal["key", "count"]
_DurationUnit = Literal["ns", "ms", "s"]
_PercentileMethod = Literal["nearest", "linear", "lower", "hybrid"]
_Sort = Literal["path", "modified", "accessed", "created"]
_Registers = str | Sequence[str]
_Review = Callable[[Change], bool | str]
_Backend = Literal["auto", "cpu", "gpu"]

__all__ = [
    "Atom",
    "AtomTest",
    "Bpe",
    "Built",
    "Change",
    "FilterCheck",
    "Finding",
    "Follow",
    "Grammar",
    "Group",
    "Index",
    "Library",
    "Line",
    "LiteralTest",
    "Match",
    "Matches",
    "ParseNode",
    "Pattern",
    "PatternSet",
    "Prefilter",
    "Record",
    "Rule",
    "ScanRoute",
    "ScanStats",
    "StreamScanner",
    "Tiling",
    "Token",
    "TrexWarning",
    "__version__",
    "axes",
    "device_available",
    "escape",
    "files",
    "findings_json",
    "follow",
    "follow_lines",
    "follow_redact",
    "follow_rewrite",
    "head",
    "infer",
    "lines",
    "parse",
    "query",
    "read",
    "records",
    "sarif",
    "scan_stats",
    "set_date_order",
    "set_now",
    "set_tz_offset",
    "tail",
    "templates",
    "texture",
    "tokens",
    "version",
]

__version__: str

@final
class Match:
    @property
    def path(self) -> str | None: ...
    @property
    def line(self) -> int: ...
    @property
    def column(self) -> int: ...
    @property
    def start(self) -> int: ...
    @property
    def end(self) -> int: ...
    @property
    def byte_start(self) -> int: ...
    @property
    def byte_end(self) -> int: ...
    @property
    def text(self) -> str | bytes: ...
    @property
    def captures(self) -> dict[str, Any]: ...
    @property
    def pattern(self) -> str | None: ...
    def span(self) -> tuple[int, int]: ...
    def byte_span(self) -> tuple[int, int]: ...
    def group(self, name: str) -> Any: ...
    def __getitem__(self, name: str, /) -> Any: ...
    def capture_span(self, name: str) -> tuple[int, int]: ...
    def capture_byte_span(self, name: str) -> tuple[int, int]: ...
    def explain(self, input: _Text) -> dict[str, Any]: ...
    def value(self, name: str, unit: _DurationUnit = "ns") -> Any: ...
    def kind(self, name: str) -> str | None: ...
    def format(self, template: str, input: _Text) -> str: ...

@final
class ScanRoute:
    @property
    def rung(self) -> str: ...
    @property
    def inputs(self) -> int: ...

@final
class ScanStats:
    @property
    def matches(self) -> int: ...
    @property
    def matched_lines(self) -> int: ...
    @property
    def files_with_matches(self) -> int: ...
    @property
    def files_searched(self) -> int: ...
    @property
    def bytes_searched(self) -> int: ...
    @property
    def searching(self) -> timedelta: ...
    @property
    def elapsed(self) -> timedelta: ...
    @property
    def tokens_lexed(self) -> int: ...
    @property
    def lexing(self) -> timedelta: ...
    @property
    def matching(self) -> timedelta: ...
    @property
    def device_bytes(self) -> int: ...
    @property
    def routes(self) -> list[ScanRoute]: ...

@final
class Matches:
    def __iter__(self) -> Matches: ...
    def __next__(self) -> Match: ...
    def __len__(self) -> int: ...

@final
class Line:
    @property
    def path(self) -> str | None: ...
    @property
    def number(self) -> int: ...
    @property
    def text(self) -> str | bytes: ...
    @property
    def is_match(self) -> bool: ...
    @property
    def matches(self) -> list[Match]: ...

@final
class Pattern:
    def __new__(cls, source: str, lib: _Lib | None = None) -> Pattern: ...
    @property
    def source(self) -> str: ...
    @property
    def fields(self) -> list[dict[str, Any]]: ...
    def capture_names(self) -> list[str]: ...
    def is_match(
        self, input: _Text, *, head: int | None = None, tail: int | None = None, lines: str | None = None, unit: str = "line"
    ) -> bool: ...
    def find(
        self, input: _Text, *, head: int | None = None, tail: int | None = None, lines: str | None = None, unit: str = "line"
    ) -> Match | None: ...
    def scan(
        self,
        input: _Text,
        *,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        backend: _Backend = "auto",
        dual_grain: bool = False,
        chunk_size: int | None = None,
    ) -> list[Match]: ...
    def find_iter(
        self,
        input: _Text,
        *,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        backend: _Backend = "auto",
        dual_grain: bool = False,
        chunk_size: int | None = None,
    ) -> Matches: ...
    def grep(
        self,
        input: _Text | None = None,
        *,
        path: _Paths | None = None,
        before: int | str = 0,
        after: int | str = 0,
        context: int | str | None = None,
        invert: bool = False,
        whole_line: bool = False,
        max_count: int | None = None,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        hidden: bool = False,
        no_ignore: bool = False,
        binary: bool = False,
        globs: Sequence[str] | None = None,
        backend: _Backend = "auto",
    ) -> Iterator[Line]: ...
    def captures(
        self, input: _Text, *, head: int | None = None, tail: int | None = None, lines: str | None = None, unit: str = "line"
    ) -> Match | None: ...
    def captures_iter(
        self, input: _Text, *, head: int | None = None, tail: int | None = None, lines: str | None = None, unit: str = "line"
    ) -> Matches: ...
    def records(self, input: _Text) -> list[dict[str, Any]]: ...
    def count_by(
        self,
        key: str,
        input: _Text,
        order: _Order = "key",
        *,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
    ) -> list[tuple[str, int]]: ...
    def group_by(
        self,
        key: str,
        text: _Text | None = None,
        *,
        path: _Paths | None = None,
        order: _Order = "key",
        limit: int | None = None,
        sum: _Registers | None = None,
        avg: _Registers | None = None,
        min: _Registers | None = None,
        max: _Registers | None = None,
        percentiles: Mapping[str, int | Sequence[int]] | None = None,
        percentile_method: _PercentileMethod = "nearest",
        duration_unit: _DurationUnit = "ns",
        binary: bool = False,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
    ) -> list[Group]: ...
    def distinct(
        self,
        key: str,
        text: _Text | None = None,
        *,
        path: _Paths | None = None,
        order: _Order = "key",
        limit: int | None = None,
        binary: bool = False,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
    ) -> list[str]: ...
    def rewrite(
        self,
        repl: str | Callable[[Match], _T],
        input: _T,
        *,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        backend: _Backend = "auto",
        dual_grain: bool = False,
        chunk_size: int | None = None,
    ) -> _T: ...
    def rewrite_n(
        self,
        repl: str | Callable[[Match], _T],
        input: _T,
        n: int,
        *,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
    ) -> _T: ...
    def rewrite_first(
        self,
        repl: str | Callable[[Match], _T],
        input: _T,
        *,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
    ) -> _T: ...
    def redact(
        self,
        input: _T,
        *,
        keep: str | None = None,
        mask: str = "*",
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
    ) -> _T: ...
    def diff(
        self,
        repl: str | Callable[[Match], str],
        path: _Paths,
        *,
        context: int = 3,
        max_count: int | None = None,
        backend: _Backend = "auto",
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        hidden: bool = False,
        no_ignore: bool = False,
        binary: bool = False,
    ) -> str: ...
    def rewrite_file(
        self,
        repl: str | Callable[[Match], str],
        path: _Paths,
        *,
        review: _Review | None = None,
        show_skipped: bool = False,
        explain: bool = False,
        max_count: int | None = None,
        backend: _Backend = "auto",
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        hidden: bool = False,
        no_ignore: bool = False,
        binary: bool = False,
    ) -> int: ...
    def redact_file(
        self,
        path: _Paths,
        *,
        keep: str | None = None,
        mask: str = "*",
        review: _Review | None = None,
        show_skipped: bool = False,
        explain: bool = False,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        hidden: bool = False,
        no_ignore: bool = False,
        binary: bool = False,
    ) -> int: ...
    def redact_diff(
        self,
        path: _Paths,
        *,
        keep: str | None = None,
        mask: str = "*",
        context: int = 3,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        hidden: bool = False,
        no_ignore: bool = False,
        binary: bool = False,
    ) -> str: ...
    def split(self, input: _T) -> list[_T]: ...
    def splitn(self, input: _T, limit: int) -> list[_T]: ...

@final
class PatternSet:
    def __new__(cls, patterns: Iterable[Pattern | str]) -> PatternSet: ...
    @staticmethod
    def from_file(path: str) -> PatternSet: ...
    @property
    def names(self) -> list[str]: ...
    def __len__(self) -> int: ...
    def scan(self, input: _Text) -> list[tuple[int, Match]]: ...
    def is_match(self, input: _Text) -> bool: ...
    def matches(self, input: _Text) -> list[int]: ...
    def matches_at(self, input: _Text, at: int) -> list[int]: ...
    def matches_with_spans(self, input: _Text) -> list[tuple[int, Match]]: ...
    def group_by(
        self,
        key: str,
        text: _Text | None = None,
        *,
        path: _Paths | None = None,
        order: _Order = "key",
        limit: int | None = None,
        sum: _Registers | None = None,
        avg: _Registers | None = None,
        min: _Registers | None = None,
        max: _Registers | None = None,
        percentiles: Mapping[str, int | Sequence[int]] | None = None,
        percentile_method: _PercentileMethod = "nearest",
        duration_unit: _DurationUnit = "ns",
        binary: bool = False,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
    ) -> list[Group]: ...
    def distinct(
        self,
        key: str,
        text: _Text | None = None,
        *,
        path: _Paths | None = None,
        order: _Order = "key",
        limit: int | None = None,
        binary: bool = False,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
    ) -> list[str]: ...

@final
class Group:
    @property
    def key(self) -> str: ...
    @property
    def count(self) -> int: ...
    @property
    def sum(self) -> dict[str, Any]: ...
    @property
    def avg(self) -> dict[str, Any]: ...
    @property
    def min(self) -> dict[str, Any]: ...
    @property
    def max(self) -> dict[str, Any]: ...
    @property
    def percentiles(self) -> dict[str, dict[int, Any]]: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class StreamScanner:
    def __new__(cls, source: Pattern | PatternSet) -> StreamScanner: ...
    def push(self, chunk: _Text) -> list[tuple[int, int] | tuple[int, int, int]]: ...
    def finish(self) -> list[tuple[int, int] | tuple[int, int, int]]: ...

@final
class Built:
    @property
    def pattern(self) -> str: ...
    @property
    def format(self) -> str: ...
    @property
    def file(self) -> str: ...
    @property
    def declarations(self) -> list[str]: ...
    @property
    def suggestions(self) -> list[tuple[str, list[str]]]: ...
    @property
    def fields(self) -> list[dict[str, Any]]: ...
    @property
    def shapes(self) -> list[dict[str, Any]]: ...
    @property
    def rows(self) -> list[dict[str, Any]]: ...
    @property
    def records(self) -> list[dict[str, Any]]: ...

@final
class Atom:
    @property
    def name(self) -> str: ...
    @property
    def form(self) -> Literal["shape", "shape-after", "kind", "let", "rule"]: ...
    @property
    def definition(self) -> str: ...
    @property
    def source(self) -> str: ...
    @property
    def description(self) -> str: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class AtomTest:
    @property
    def name(self) -> str: ...
    @property
    def passed(self) -> bool: ...
    @property
    def accepts(self) -> list[str]: ...
    @property
    def rejects(self) -> list[str]: ...
    @property
    def failures(self) -> list[str]: ...
    @property
    def file(self) -> str | None: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class Rule:
    @property
    def name(self) -> str: ...
    @property
    def severity(self) -> _Severity: ...
    @property
    def message(self) -> str: ...
    @property
    def pattern(self) -> str: ...
    @property
    def fix(self) -> str | None: ...
    @property
    def files(self) -> list[str]: ...
    @property
    def meta(self) -> dict[str, Any]: ...
    @property
    def tags(self) -> list[str]: ...
    @property
    def file(self) -> str | None: ...
    @property
    def line(self) -> int: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class Library:
    def __new__(cls) -> Library: ...
    @staticmethod
    def load(*paths: _Path) -> Library: ...
    @staticmethod
    def shipped() -> Library: ...
    def shape(
        self,
        name: str,
        bytepat: str,
        *,
        after: bool = False,
        accepts: Sequence[str] = ...,
        rejects: Sequence[str] = ...,
        replace: bool = False,
    ) -> None: ...
    def kind(
        self, name: str, pattern: str, *, accepts: Sequence[str] = ..., rejects: Sequence[str] = ..., replace: bool = False
    ) -> None: ...
    def let(
        self, name: str, pattern: str, *, accepts: Sequence[str] = ..., rejects: Sequence[str] = ..., replace: bool = False
    ) -> None: ...
    def declare(self, line: str) -> None: ...
    def include(self, *paths: _Path) -> None: ...
    def remove(self, *items: _Path) -> None: ...
    def clear(self) -> None: ...
    def test(self) -> list[AtomTest]: ...
    @property
    def rules(self) -> list[Rule]: ...
    @property
    def names(self) -> list[str]: ...
    @property
    def files(self) -> list[str]: ...
    def check(
        self,
        text: _Text | None = None,
        *,
        path: _Paths | None = None,
        max_count: int | None = None,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        hidden: bool = False,
        no_ignore: bool = False,
        globs: Sequence[str] | None = None,
        binary: bool = False,
    ) -> list[Finding]: ...
    def fix(
        self,
        path: _Paths,
        *,
        dry_run: bool = False,
        context: int = 3,
        review: _Review | None = None,
        show_skipped: bool = False,
        head: int | None = None,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        hidden: bool = False,
        no_ignore: bool = False,
        globs: Sequence[str] | None = None,
        binary: bool = False,
    ) -> int | str: ...
    def follow(
        self,
        path: _Paths,
        *,
        max_count: int | None = None,
        keep_count: bool = False,
        tail: int | None = None,
        lines: str | None = None,
        unit: str = "line",
        hidden: bool = False,
        no_ignore: bool = False,
        globs: Sequence[str] | None = None,
        binary: bool = False,
    ) -> Follow: ...
    def __iter__(self) -> Iterator[Atom]: ...
    def __len__(self) -> int: ...
    def __contains__(self, name: str, /) -> bool: ...

@final
class Token:
    @property
    def kind(self) -> str: ...
    @property
    def start(self) -> int: ...
    @property
    def end(self) -> int: ...
    @property
    def text(self) -> str | bytes: ...
    @property
    def value(self) -> Any: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class Finding:
    @property
    def rule(self) -> str: ...
    @property
    def severity(self) -> _Severity: ...
    @property
    def message(self) -> str: ...
    @property
    def path(self) -> str | None: ...
    @property
    def line(self) -> int: ...
    @property
    def column(self) -> int: ...
    @property
    def end_line(self) -> int: ...
    @property
    def end_column(self) -> int: ...
    @property
    def start(self) -> int: ...
    @property
    def end(self) -> int: ...
    @property
    def byte_start(self) -> int: ...
    @property
    def byte_end(self) -> int: ...
    @property
    def text(self) -> str | bytes: ...
    @property
    def fix(self) -> str | bytes | None: ...
    @property
    def fix_span(self) -> tuple[int, int] | None: ...
    @property
    def captures(self) -> dict[str, Any]: ...
    @property
    def definition(self) -> Rule: ...
    def github(self) -> str: ...
    def format(self, template: str) -> str: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class Change:
    @property
    def path(self) -> str: ...
    @property
    def start(self) -> int: ...
    @property
    def end(self) -> int: ...
    @property
    def text(self) -> str: ...
    @property
    def replacement(self) -> str: ...
    @property
    def diff(self) -> str: ...
    @property
    def template(self) -> str: ...
    @property
    def later(self) -> int: ...
    @property
    def index(self) -> int: ...
    @property
    def total(self) -> int: ...
    @property
    def explanation(self) -> dict[str, Any] | None: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class Follow:
    def __iter__(self) -> Follow: ...
    def __next__(self) -> Finding: ...

class TrexWarning(UserWarning): ...

@final
class ParseNode:
    @property
    def path(self) -> str | None: ...
    @property
    def rule(self) -> str | None: ...
    @property
    def text(self) -> str: ...
    @property
    def children(self) -> list[ParseNode]: ...
    @property
    def expression(self) -> str: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class Tiling:
    @property
    def words(self) -> list[str]: ...
    @property
    def probability(self) -> float: ...
    @property
    def parses(self) -> int: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class Grammar:
    def __new__(cls, source: str, *, start: str | None = None) -> Grammar: ...
    @property
    def start(self) -> str: ...
    @property
    def rules(self) -> list[str]: ...
    def parse(self, text: _Text | None = None, *, path: _Path | None = None) -> ParseNode | None: ...
    def accepts(self, text: _Text | None = None, *, path: _Path | None = None) -> bool: ...
    def count(self, text: _Text | None = None, *, path: _Path | None = None) -> int: ...
    def best(self, text: _Text | None = None, *, path: _Path | None = None) -> float: ...
    def probability(self, text: _Text | None = None, *, path: _Path | None = None) -> float: ...
    def segment(self, text: str, dictionary: Sequence[str]) -> list[Tiling]: ...
    def count_segmentations(self, text: str, dictionary: Sequence[str]) -> int: ...
    def best_segmentation_probability(self, text: str, dictionary: Sequence[str]) -> float: ...

@final
class Bpe:
    def __new__(cls, model: str) -> Bpe: ...
    @staticmethod
    def train(
        text: _Text | None = None, *, path: _Paths | None = None, merges: int = 1000, max_bytes: int | None = None
    ) -> Bpe: ...
    @staticmethod
    def load(path: _Path) -> Bpe: ...
    @property
    def merges(self) -> int: ...
    @property
    def model(self) -> str: ...
    def save(self, path: _Path) -> None: ...
    def encode(self, text: _Text | None = None, *, path: _Path | None = None) -> list[str]: ...
    def __len__(self) -> int: ...

@final
class Index:
    @staticmethod
    def build(path: _Path, *, hidden: bool = False, no_ignore: bool = False, globs: Sequence[str] | None = None) -> Index: ...
    @staticmethod
    def load(path: _Path) -> Index: ...
    @property
    def root(self) -> str: ...
    @property
    def path(self) -> str: ...
    @property
    def files(self) -> int: ...
    def candidates(
        self, pattern: Pattern | str, *, hidden: bool = False, no_ignore: bool = False, globs: Sequence[str] | None = None
    ) -> list[str]: ...
    def to_dict(self) -> dict[str, Any]: ...
    def __len__(self) -> int: ...

@final
class LiteralTest:
    @property
    def literal(self) -> str | bytes: ...
    @property
    def filter(self) -> _Filter: ...
    @property
    def might_occur(self) -> bool: ...
    @property
    def occurs(self) -> bool: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class FilterCheck:
    @property
    def filter(self) -> _Filter: ...
    @property
    def present_probes(self) -> int: ...
    @property
    def false_negatives(self) -> int: ...
    @property
    def absent_probes(self) -> int: ...
    @property
    def absent_rejected(self) -> int: ...
    def to_dict(self) -> dict[str, Any]: ...

@final
class Prefilter:
    def __new__(cls, text: _Text | None = None, *, path: _Paths | None = None, kind: _Filter = "bloom") -> Prefilter: ...
    @property
    def filter(self) -> _Filter: ...
    @property
    def bytes(self) -> int: ...
    def might_contain(self, literal: _Text) -> bool: ...
    def test(self, *literals: _Text) -> list[LiteralTest]: ...
    def verify(self) -> list[FilterCheck]: ...

def parse(source: str, lib: _Lib | None = None) -> Pattern: ...
def escape(text: str) -> str: ...
def tokens(
    text: _Text | None = None, *, path: _Path | None = None, whitespace: bool = False, lib: _Lib | None = None, binary: bool = False
) -> list[Token]: ...
def sarif(findings: Iterable[Finding], *, lib: Library | None = None) -> str: ...
def findings_json(findings: Iterable[Finding]) -> str: ...
@overload
def head(n: int, input: _T, *, path: None = None, unit: str = "line") -> _T: ...
@overload
def head(n: int, input: None = None, *, path: str, unit: str = "line") -> str: ...
@overload
def tail(n: int, input: _T, *, path: None = None, unit: str = "line") -> _T: ...
@overload
def tail(n: int, input: None = None, *, path: str, unit: str = "line") -> str: ...
@overload
def lines(range: str, input: _T, *, path: None = None, unit: str = "line") -> _T: ...
@overload
def lines(range: str, input: None = None, *, path: str, unit: str = "line") -> str: ...
def files(
    paths: str | Sequence[str] = ".",
    *,
    hidden: bool = False,
    no_ignore: bool = False,
    binary: bool = False,
    globs: Sequence[str] | None = None,
    types: Sequence[str] | None = None,
    types_not: Sequence[str] | None = None,
    texture: Sequence[str] | None = None,
    texture_not: Sequence[str] | None = None,
    sort: _Sort | None = None,
    reverse: bool = False,
) -> list[str]: ...
def texture(input: _Text | None = None, *, path: str | None = None) -> tuple[str, int] | None: ...
def read(path: str) -> str: ...
def set_now(secs: int | None) -> None: ...
def set_tz_offset(secs: int) -> None: ...
def set_date_order(order: Literal["dmy", "mdy"]) -> None: ...
def scan_stats() -> ScanStats | None: ...
def follow(
    pattern: Pattern | str,
    *paths: _Path,
    from_end: bool = True,
    max_count: int | None = None,
    keep_count: bool = False,
    lib: _Lib | None = None,
) -> Iterator[Match]: ...
def follow_lines(
    *paths: _Path, tail: int | None = None, lines: str | None = None, unit: str = "line"
) -> Iterator[Line]: ...
def follow_rewrite(
    pattern: Pattern | str,
    repl: str | Callable[[Match], str],
    path: _Path,
    *,
    tail: int | None = None,
    lines: str | None = None,
    unit: str = "line",
    max_count: int | None = None,
    keep_count: bool = False,
    lib: _Lib | None = None,
) -> Iterator[str]: ...
def follow_redact(
    pattern: Pattern | str,
    path: _Path,
    *,
    keep: str | None = None,
    mask: str = "*",
    tail: int | None = None,
    lines: str | None = None,
    unit: str = "line",
    lib: _Lib | None = None,
) -> Iterator[str]: ...
@final
class Record:
    @property
    def path(self) -> str | None: ...
    @property
    def line(self) -> int: ...
    @property
    def start(self) -> int: ...
    @property
    def end(self) -> int: ...
    @property
    def byte_start(self) -> int: ...
    @property
    def byte_end(self) -> int: ...
    @property
    def text(self) -> str | bytes: ...
    @property
    def patterns(self) -> list[str]: ...
    def to_dict(self) -> dict[str, Any]: ...

def query(
    patterns: Pattern | str | Iterable[Pattern | str],
    input: _Text | None = None,
    *,
    path: _Paths | None = None,
    require: Literal["any", "all", "none"] = "any",
    at_least: int | None = None,
    exclude: Pattern | str | Iterable[Pattern | str] = (),
    unit: str = "line",
    record_start: Pattern | str | None = None,
    record_span: Pattern | str | None = None,
    max_count: int | None = None,
    lib: _Lib | None = None,
) -> list[Record]: ...
def templates(
    text: _Text | None = None,
    cut: str | None = None,
    *,
    path: _Paths | None = None,
    against: _Text | os.PathLike[str] | Sequence[_Path] | None = None,
    novel: bool = False,
    rare: bool = False,
    unit: str = "line",
    record_start: Pattern | str | None = None,
    record_span: Pattern | str | None = None,
    head: int | None = None,
    tail: int | None = None,
    lines: str | None = None,
    hidden: bool = False,
    no_ignore: bool = False,
    binary: bool = False,
    lib: _Lib | None = None,
) -> list[dict[str, Any]]: ...
def infer(
    examples: Sequence[_Text],
    anchored: bool = False,
    against: Sequence[_Text] = ...,
    marked: Sequence[str] = ...,
    fields: Mapping[str, str | Sequence[str]] | None = None,
    marks_in_lines: bool = False,
    unanchored: bool = False,
    no_mint: bool = False,
    mint_shapes: bool = False,
    lib: _Lib | None = None,
) -> str | Built: ...
def records(text: _Text, unit: str) -> list[tuple[int, int]]: ...
def version() -> str: ...
def device_available() -> bool: ...
