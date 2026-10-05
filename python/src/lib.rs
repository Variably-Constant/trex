//! The trex library as the Python module `trex`, built into the `trex-re`
//! wheel by maturin.
//!
//! A `str` is scanned as its UTF-8 bytes and reported in characters, so
//! `s[m.start:m.end] == m.text`; `bytes` are scanned and reported as bytes.
//! Every match also carries its UTF-8 byte offsets, and results come back in
//! the input's type. Scans run with the interpreter lock released.

mod axes;
mod library;
mod rules;
mod tools;

use std::sync::Arc;

use pyo3::exceptions::{PyKeyError, PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyString, PyTuple};
use trex::encoding::{OffsetUnit, Offsets};

/// A str or bytes handed to a scan: the UTF-8 bytes it reads, and the type
/// the answer is handed back in.
enum Input<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
}

impl<'a> Input<'a> {
    fn of(obj: &'a Bound<'_, PyAny>) -> PyResult<Self> {
        if obj.is_instance_of::<PyString>() {
            let s: &Bound<'_, PyString> = obj.cast()?;
            return Ok(Input::Text(s.to_str()?));
        }
        if obj.is_instance_of::<PyBytes>() {
            let b: &Bound<'_, PyBytes> = obj.cast()?;
            return Ok(Input::Bytes(b.as_bytes()));
        }
        Err(PyTypeError::new_err("the input must be a str or bytes"))
    }

    fn bytes(&self) -> &'a [u8] {
        match self {
            Input::Text(s) => s.as_bytes(),
            Input::Bytes(b) => b,
        }
    }

    /// The bytes at `start..end`, in the input's type. A span that cuts a
    /// character is decoded with replacement characters at the cut.
    fn slice(&self, py: Python<'_>, start: usize, end: usize) -> Py<PyAny> {
        match self {
            Input::Text(s) => match s.get(start..end) {
                Some(t) => PyString::new(py, t).into_any().unbind(),
                None => PyString::new(py, &String::from_utf8_lossy(&s.as_bytes()[start..end]))
                    .into_any()
                    .unbind(),
            },
            Input::Bytes(b) => PyBytes::new(py, &b[start..end]).into_any().unbind(),
        }
    }

    /// `bytes`, in the input's type.
    fn wrap(&self, py: Python<'_>, bytes: &[u8]) -> PyResult<Py<PyAny>> {
        match self {
            Input::Text(_) => match std::str::from_utf8(bytes) {
                Ok(s) => Ok(PyString::new(py, s).into_any().unbind()),
                Err(e) => Err(PyValueError::new_err(format!("the result is not UTF-8: {e}"))),
            },
            Input::Bytes(_) => Ok(PyBytes::new(py, bytes).into_any().unbind()),
        }
    }

    /// The byte offset of position `at` in the input's units; the end when
    /// `at` is past it.
    fn byte_at(&self, at: usize) -> usize {
        match self {
            Input::Text(s) => s.char_indices().nth(at).map_or(s.len(), |(b, _)| b),
            Input::Bytes(b) => at.min(b.len()),
        }
    }
}

impl Input<'_> {
    /// The unit this input's offsets are reported in: characters of a str,
    /// bytes of bytes.
    fn unit(&self) -> OffsetUnit {
        match self {
            Input::Text(_) => OffsetUnit::CodePoints,
            Input::Bytes(_) => OffsetUnit::Bytes,
        }
    }
}

/// Byte offsets of an input read in its units, characters of a str and
/// bytes of bytes.
fn units_of<'a>(input: &Input<'a>) -> Offsets<'a> {
    Offsets::new(input.bytes(), input.unit())
}

/// Byte offsets of `text`, UTF-8 a file decodes to, read as character
/// offsets, as a str of it would count them.
fn chars_of(text: &[u8]) -> Offsets<'_> {
    Offsets::new(text, OffsetUnit::CodePoints)
}

/// The position of each match of a scan, counted as the matches are read in
/// order: its offsets in the input's units, its line and column, the file it
/// is in, and what comes before the part of the input read, which every
/// match's place adds.
struct Placing<'a> {
    units: Offsets<'a>,
    lines: trex::files::LineCursor<'a>,
    path: Option<String>,
    bytes_ahead: usize,
    units_ahead: usize,
    lines_ahead: usize,
}

impl<'a> Placing<'a> {
    /// The places of matches in the whole of `input`.
    fn of(input: &Input<'a>) -> Self {
        Placing {
            units: units_of(input),
            lines: trex::files::LineCursor::new(input.bytes()),
            path: None,
            bytes_ahead: 0,
            units_ahead: 0,
            lines_ahead: 0,
        }
    }
}

/// The byte range `part` occupies inside `whole`, which it is a slice of.
fn offsets_of(whole: &[u8], part: &[u8]) -> (usize, usize) {
    let start = part.as_ptr().addr() - whole.as_ptr().addr();
    (start, start + part.len())
}

fn parse_error(e: trex::ParseError) -> PyErr {
    PyValueError::new_err(format!("pattern error at byte {}: {}", e.pos, e.msg))
}

fn template_of(pattern: &trex::ast::Pattern, src: &str) -> PyResult<trex::Template> {
    trex::Template::parse(src, &pattern.capture_names())
        .map_err(|e| PyValueError::new_err(format!("template error at byte {}: {}", e.pos, e.msg)))
}

/// The way of running a scan `backend`, `dual_grain` and `chunk_size` name,
/// refused as PowerShell's `-Backend`, `-DualGrain` and `-ChunkSize` are: a
/// backend other than the three, a chunk of no bytes, or two ways at once.
fn engine_of(backend: &str, dual_grain: bool, chunk_size: Option<usize>) -> PyResult<trex::Engine> {
    let backend = match backend {
        "auto" => trex::Backend::Auto,
        "cpu" => trex::Backend::Cpu,
        "gpu" => trex::Backend::Gpu,
        other => return Err(PyValueError::new_err(format!("backend is 'auto', 'cpu' or 'gpu', not {other:?}"))),
    };
    if chunk_size == Some(0) {
        return Err(PyValueError::new_err("chunk_size is a count of bytes, 1 or more"));
    }
    let mut ways = Vec::new();
    if backend == trex::Backend::Gpu {
        ways.push("backend='gpu'");
    }
    if dual_grain {
        ways.push("dual_grain=True");
    }
    if chunk_size.is_some() {
        ways.push("chunk_size");
    }
    if let [a, b, ..] = ways.as_slice() {
        return Err(PyValueError::new_err(format!("{a} and {b} each say how the scan runs; give one")));
    }
    Ok(trex::Engine { backend, dual_grain, chunk_size })
}

fn no_register(name: &str) -> PyErr {
    PyKeyError::new_err(format!("no register named {name:?}"))
}

fn not_a_replacement() -> PyErr {
    PyTypeError::new_err("the replacement is a template str or a callable taking a Match")
}

/// The file at `path`: its bytes, and its text as a scan reads it, decoded
/// from the UTF encoding its byte order mark names. Text that is not UTF-8
/// once decoded is refused, since a str cannot hold it.
fn file_text(py: Python<'_>, path: &str) -> PyResult<(Vec<u8>, String)> {
    let raw = py.detach(|| std::fs::read(path)).map_err(|e| PyOSError::new_err(format!("{path}: {e}")))?;
    let text = String::from_utf8(trex::encoding::text_of(&raw).into_owned())
        .map_err(|e| PyValueError::new_err(format!("{path}: its text is not UTF-8: {e}")))?;
    Ok((raw, text))
}

/// Why a match matched, as `Match.explain` gives it: `tokens` the
/// significant tokens it spans as `(kind, text)`, `guards` what each guarded
/// token passed, `readings` each axis read at each token as `(axis, text,
/// value)`, and `route` the rung that answered.
fn explanation_dict(py: Python<'_>, e: &trex::explain::Explanation) -> PyResult<Py<PyDict>> {
    let d = PyDict::new(py);
    d.set_item("tokens", e.tokens.clone())?;
    d.set_item("guards", e.guards.clone())?;
    let readings: Vec<(String, String, String)> =
        e.readings.iter().map(|r| (r.axis.to_string(), r.text.clone(), r.value.clone())).collect();
    d.set_item("readings", readings)?;
    d.set_item("route", e.route.clone())?;
    Ok(d.unbind())
}

/// One file a file method edits: the name it reports, its path, its bytes,
/// which the edits are written into, and its text as a scan decodes it.
struct EditedFile {
    name: String,
    path: std::path::PathBuf,
    raw: Vec<u8>,
    text: String,
}

/// The files `paths` names, walked as `reading` walks them, each read whole
/// and decoded as a scan decodes it. A file holding a NUL byte is read under
/// `binary=True`; otherwise one named alone raises `ValueError`, as a scan
/// refuses one, naming what `binary=True` would let `verb` do, and the rest
/// are passed over. A file that cannot be read is named in `errors`.
fn files_edited(
    py: Python<'_>,
    paths: PathArg,
    reading: &rules::Reading,
    verb: &str,
    errors: &mut Vec<String>,
) -> PyResult<Vec<EditedFile>> {
    let walked = rules::walked(py, paths, &reading.walk)?;
    errors.extend(walked.errors);
    let mut out = Vec::with_capacity(walked.files.len());
    for (name, path) in walked.files {
        let raw = match py.detach(|| std::fs::read(&path)) {
            Ok(raw) => raw,
            Err(e) => {
                errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        if !reading.binary && trex::files::is_binary(&raw) {
            if walked.lone {
                return Err(PyValueError::new_err(format!(
                    "{name} holds a NUL byte and is binary; binary=True {verb} it"
                )));
            }
            continue;
        }
        match String::from_utf8(trex::encoding::text_of(&raw).into_owned()) {
            Ok(text) => out.push(EditedFile { name, path, raw, text }),
            Err(e) => errors.push(format!("{name}: its text is not UTF-8: {e}")),
        }
    }
    Ok(out)
}

/// The byte range of `text` the window `reading` names, the whole text where
/// it names none.
fn window_of(text: &[u8], reading: &rules::Reading) -> std::ops::Range<usize> {
    match reading.select {
        Some(select) => trex::window::range_of(text, select, &reading.unit),
        None => 0..text.len(),
    }
}

/// Refuse a `review=` that is not a callable, and what only a review reads,
/// given without one.
fn review_asked(review: Option<&Bound<'_, PyAny>>, show_skipped: bool, explain: bool) -> PyResult<()> {
    match review {
        Some(r) if !r.is_callable() => {
            Err(PyTypeError::new_err("review= takes a callable, handed each change as a trex.Change"))
        }
        Some(_) => Ok(()),
        None if show_skipped => Err(PyValueError::new_err(
            "show_skipped= lists the changes a template answer skipped, which only review= gives",
        )),
        None if explain => Err(PyValueError::new_err(
            "explain= puts a scan's explanation on each change review= is handed; give review=",
        )),
        None => Ok(()),
    }
}

/// Write each file's `edits`, or with `review` put each change to it first
/// as `lib.fix` does, and give how many were written; `explain`, where
/// given, explains each change to the review. The errors met reading the
/// files and writing them raise `OSError` once the rest are written.
fn write_edited(
    py: Python<'_>,
    edited: Vec<(EditedFile, Vec<trex::files::Edit>)>,
    review: Option<&Bound<'_, PyAny>>,
    show_skipped: bool,
    explain: Option<&rules::ExplainChange<'_>>,
    mut errors: Vec<String>,
) -> PyResult<usize> {
    let mut written = 0usize;
    let mut queue: Vec<rules::Pending> = Vec::new();
    for (f, edits) in edited {
        if edits.is_empty() {
            continue;
        }
        if review.is_some() {
            queue.push(rules::Pending { name: f.name, path: f.path, raw: f.raw, text: f.text.into_bytes(), edits });
            continue;
        }
        let out = trex::files::written(&f.raw, &edits, trex::files::ReadAs::Decoded);
        match py.detach(|| std::fs::write(&f.path, out)) {
            Ok(()) => written += edits.len(),
            Err(e) => errors.push(format!("{}: cannot write it: {e}", f.name)),
        }
    }
    if !errors.is_empty() {
        return Err(PyOSError::new_err(errors.join("; ")));
    }
    if let Some(review) = review {
        let fixing = rules::Fixing { dry_run: false, context: 3, review: Some(review), show_skipped, explain };
        written = rules::review_fixes(py, review, &queue, &fixing)?;
    }
    Ok(written)
}

/// Every file's unified diff from `edited`, in walk order, with `context`
/// lines around each change, as `--dry-run` prints them; the errors met
/// reading the files raise `OSError`.
fn diffs_of(edited: &[(EditedFile, Vec<trex::files::Edit>)], context: usize, errors: Vec<String>) -> PyResult<String> {
    if !errors.is_empty() {
        return Err(PyOSError::new_err(errors.join("; ")));
    }
    let mut out = String::new();
    for (f, edits) in edited {
        if !edits.is_empty() {
            out.push_str(&trex::files::unified_diff(&f.name, f.text.as_bytes(), edits, context));
        }
    }
    Ok(out)
}

/// `input` with each edit's bytes in its place, the gaps copied verbatim.
fn splice(input: &[u8], edits: &[trex::files::Edit]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut at = 0;
    for e in edits {
        out.extend_from_slice(&input[at..e.start]);
        out.extend_from_slice(&e.replacement);
        at = e.end;
    }
    out.extend_from_slice(&input[at..]);
    out
}

/// The part of an input `head`, `tail` or `lines` names, where one of them
/// names one: the first `head` records, the last `tail`, or `lines` as the
/// command line's `--lines` takes a range, `"A..B"`, `"A.."`, `"..B"` or
/// `"A"`.
fn select_of(head: Option<usize>, tail: Option<usize>, lines: Option<&str>) -> PyResult<Option<trex::window::Select>> {
    use trex::window::Select;
    match (head, tail, lines) {
        (None, None, None) => Ok(None),
        (Some(n), None, None) => Ok(Some(Select::Head(n))),
        (None, Some(n), None) => Ok(Some(Select::Tail(n))),
        (None, None, Some(range)) => {
            Select::parse_range(range).map(Some).map_err(|e| PyValueError::new_err(format!("lines= {e}")))
        }
        _ => Err(PyValueError::new_err("head=, tail= and lines= each name the part read; give one")),
    }
}

/// The record unit `unit` names, which a window counts.
fn unit_of(unit: &str) -> PyResult<trex::records::RecordUnit> {
    trex::records::RecordUnit::parse(unit).map_err(|e| PyValueError::new_err(format!("unit= {e}")))
}

/// The unit a duration is given in, `name` as the keyword `keyword` took it.
fn duration_unit_of(keyword: &str, name: &str) -> PyResult<trex::typed::DurationUnit> {
    match name {
        "ns" => Ok(trex::typed::DurationUnit::Nanoseconds),
        "ms" => Ok(trex::typed::DurationUnit::Milliseconds),
        "s" => Ok(trex::typed::DurationUnit::Seconds),
        other => Err(PyValueError::new_err(format!("{keyword} takes 'ns', 'ms' or 's', not {other:?}"))),
    }
}

/// The byte range of `bytes` the window `head`, `tail` or `lines` names in
/// records of `unit`; the whole input where none is named.
fn window_range(
    bytes: &[u8],
    head: Option<usize>,
    tail: Option<usize>,
    lines: Option<&str>,
    unit: &str,
) -> PyResult<std::ops::Range<usize>> {
    match select_of(head, tail, lines)? {
        Some(select) => Ok(trex::window::range_of(bytes, select, &unit_of(unit)?)),
        None if unit == "line" => Ok(0..bytes.len()),
        None => Err(PyValueError::new_err(format!(
            "unit={unit:?} names what head=, tail= and lines= count, and none was given"
        ))),
    }
}

/// The declarations a `lib` argument names: a `Library`, or the path of a
/// pattern file or of a directory read as its `.trex` files, or a list of
/// them, read as `Library.load` reads them. Nothing where none was given,
/// and the pattern is then read with no declarations.
fn shapes_of(lib: Option<&Bound<'_, PyAny>>) -> PyResult<trex::ShapeSet> {
    let Some(lib) = lib else {
        return Ok(trex::ShapeSet::new());
    };
    if lib.is_instance_of::<library::Library>() {
        return Ok(lib.cast::<library::Library>()?.borrow().shapes());
    }
    let refused = |e: PyErr| {
        PyTypeError::new_err(format!(
            "lib= takes a trex.Library, a pattern file's path or a directory's, or a list of them: {e}"
        ))
    };
    let paths: Vec<std::path::PathBuf> = if lib.is_instance_of::<PyList>() || lib.is_instance_of::<PyTuple>() {
        lib.extract().map_err(refused)?
    } else {
        vec![lib.extract().map_err(refused)?]
    };
    let mut decls = trex::Declarations::new();
    decls.include(&paths).map_err(|e| library::declare_error(&e))?;
    Ok(decls.set().clone())
}

/// Each register a pattern binds, by name, with the kind of token it binds
/// where the pattern fixes one.
type CaptureKinds = Arc<[(String, Option<trex::token::TokenKind>)]>;

/// A pattern and the declarations it was read under, shared by the matches
/// of one call, which `Match.explain` scans with again, and the name of the
/// member it is of a set, which `Match.pattern` reports.
struct Compiled {
    pattern: trex::ast::Pattern,
    shapes: trex::ShapeSet,
    member: Option<String>,
}

/// One match: its position, its span in the input's units and in bytes,
/// its text, and what the pattern's registers bound.
#[pyclass(frozen, module = "trex")]
struct Match {
    /// The file the match is in; `None` for text.
    #[pyo3(get)]
    path: Option<String>,
    /// The line the match starts on, from 1.
    #[pyo3(get)]
    line: usize,
    /// The character the match starts at within its line, from 1.
    #[pyo3(get)]
    column: usize,
    /// The start, in the input's units.
    #[pyo3(get)]
    start: usize,
    /// The end, exclusive, in the input's units.
    #[pyo3(get)]
    end: usize,
    /// The start, as a UTF-8 byte offset.
    #[pyo3(get)]
    byte_start: usize,
    /// The end, exclusive, as a UTF-8 byte offset.
    #[pyo3(get)]
    byte_end: usize,
    /// The matched text, in the input's type.
    #[pyo3(get)]
    text: Py<PyAny>,
    /// Register name to the text it bound, in the input's type.
    #[pyo3(get)]
    captures: Py<PyDict>,
    /// Each register's name, its span in the input's units, and in bytes.
    spans: Vec<(String, usize, usize, usize, usize)>,
    /// The pattern's capture names in written order, which a numbered
    /// reference such as `1:upper` counts through.
    bound: Arc<[String]>,
    /// The kind each register binds, for `value`. Empty where the caller
    /// built this without them, in which case no register reports a value.
    kinds: CaptureKinds,
    /// The pattern that made this match and its declarations, for `explain`,
    /// shared by the matches of one call. The input is not kept beside it:
    /// holding one would copy the whole of it per scan for a report most
    /// callers never ask for, so `explain` takes it again instead.
    pattern: Arc<Compiled>,
    /// The match as the engine reported it, which the explainer reads.
    inner: trex::Match,
}

impl Match {
    fn of(
        py: Python<'_>,
        input: &Input<'_>,
        placing: &mut Placing<'_>,
        m: &trex::Match,
        bound: &Arc<[String]>,
        kinds: &CaptureKinds,
        pattern: &Arc<Compiled>,
    ) -> PyResult<Self> {
        let (line, column) = placing.lines.at(m.start);
        let line = line + placing.lines_ahead;
        let (bytes_ahead, units_ahead) = (placing.bytes_ahead, placing.units_ahead);
        let units = &mut placing.units;
        let start = units_ahead + units.at(m.start);
        let end = units_ahead + units.at(m.end);
        let captures = PyDict::new(py);
        let mut spans = Vec::with_capacity(m.names().len());
        for (name, span) in m.names().iter().zip(m.captures()) {
            let (bs, be) = (span.start(), span.end());
            let s = units.at(bs);
            let e = units.at(be);
            // A register bound under a repetition is every binding it made,
            // oldest first; any other is its one binding.
            match m.list(name) {
                Some(all) => {
                    let items: Vec<Py<PyAny>> = all.iter().map(|b| input.slice(py, b.start(), b.end())).collect();
                    captures.set_item(name, PyList::new(py, items)?)?;
                }
                None => captures.set_item(name, input.slice(py, bs, be))?,
            }
            spans.push((name.clone(), units_ahead + s, units_ahead + e, bytes_ahead + bs, bytes_ahead + be));
        }
        Ok(Match {
            path: placing.path.clone(),
            line,
            column,
            start,
            end,
            byte_start: bytes_ahead + m.start,
            byte_end: bytes_ahead + m.end,
            text: input.slice(py, m.start, m.end),
            captures: captures.unbind(),
            spans,
            bound: Arc::clone(bound),
            kinds: Arc::clone(kinds),
            pattern: Arc::clone(pattern),
            inner: m.clone().shifted(bytes_ahead),
        })
    }

    /// One parsed value as the Python type that holds it exactly.
    ///
    /// Decimal rather than float wherever the parser is exact, because the
    /// digits past the point are usually why someone matched on it; a
    /// timezone-aware datetime in UTC for an instant; a plain int for an
    /// address, since a Python int is arbitrary precision and IPv6 is 128
    /// bits.
    fn py_value(
        py: Python<'_>,
        kind: trex::token::TokenKind,
        v: &trex::typed::TypedValue,
        duration: trex::typed::DurationUnit,
    ) -> PyResult<Py<PyAny>> {
        use trex::typed::TypedValue as V;
        let decimal = |d: &trex::typed::Decimal| -> PyResult<Py<PyAny>> {
            let text = d.to_text();
            match text.parse::<i64>() {
                Ok(n) => Ok(n.into_pyobject(py)?.into_any().unbind()),
                Err(_has_a_fraction) => Ok(py
                    .import("decimal")?
                    .getattr("Decimal")?
                    .call1((text,))?
                    .unbind()),
            }
        };
        match v {
            // A duration is nanoseconds unless the caller asked for another
            // unit; the shift is exact, so seconds of 1500ms is 1.5.
            V::Num(d) if kind == trex::token::TokenKind::Duration => {
                decimal(&d.shift(duration.digits()))
            }
            V::Num(d) => decimal(d),
            V::Instant(civil) => match civil.epoch(trex::Clock::current()) {
                Some((secs, nanos)) => {
                    let dt = py.import("datetime")?;
                    let utc = dt.getattr("timezone")?.getattr("utc")?;
                    let secs = secs as f64 + f64::from(nanos) / 1e9;
                    Ok(dt.getattr("datetime")?.call_method1("fromtimestamp", (secs, utc))?.unbind())
                }
                // A time with no date places no instant, and a guessed one
                // would be worse than none.
                None => Ok(py.None()),
            },
            V::Ip(ip) => {
                let n: u128 = match ip {
                    std::net::IpAddr::V4(v4) => u128::from(u32::from(*v4)),
                    std::net::IpAddr::V6(v6) => u128::from(*v6),
                };
                Ok(n.into_pyobject(py)?.into_any().unbind())
            }
            V::Cidr(ip, bits) => {
                let n: u128 = match ip {
                    std::net::IpAddr::V4(v4) => u128::from(u32::from(*v4)),
                    std::net::IpAddr::V6(v6) => u128::from(*v6),
                };
                Ok((n, *bits).into_pyobject(py)?.into_any().unbind())
            }
            V::Version(ver) => {
                let parts: Vec<Py<PyAny>> =
                    ver.parts().iter().map(decimal).collect::<PyResult<_>>()?;
                let pre: Vec<String> = ver.pre().unwrap_or(&[]).to_vec();
                Ok((PyTuple::new(py, parts)?, PyTuple::new(py, pre)?).into_pyobject(py)?.into_any().unbind())
            }
            V::Text(s) => Ok(PyString::new(py, s).into_any().unbind()),
            V::Family(v6) => {
                Ok(PyString::new(py, if *v6 { "v6" } else { "v4" }).into_any().unbind())
            }
            V::Quantity(family, base) => {
                Ok((family.name(), decimal(base)?).into_pyobject(py)?.into_any().unbind())
            }
        }
    }

    /// `key` read over this match: a register's name, or a template
    /// reference - `0` the whole match, `1` the first register the pattern
    /// binds, `e:domain`, `0:last4`, `ip:octet1-2` a slice - in the input's
    /// type, as `${key}` renders it. An unknown register is a `KeyError`, a
    /// bad accessor or position a `ValueError`.
    fn read(&self, py: Python<'_>, key: &str) -> PyResult<Py<PyAny>> {
        if let Some(v) = self.captures.bind(py).get_item(key)? {
            return self.last_of(py, &v);
        }
        let (name, _) = key.split_once(':').unwrap_or((key, ""));
        let name = name.split('[').next().unwrap_or(name);
        let known = name == "0"
            || name.bytes().all(|b| b.is_ascii_digit())
            || self.bound.iter().any(|b| b == name);
        if !known {
            return Err(no_register(name));
        }
        let reference = trex::Reference::parse(key, &self.bound)
            .map_err(|e| PyValueError::new_err(format!("reference error at byte {}: {}", e.pos, e.msg)))?;
        let base = match reference.field() {
            trex::Field::All(n) => return self.every_binding(py, n, &reference),
            trex::Field::Whole => self.text.clone_ref(py),
            trex::Field::Register(n) => match self.captures.bind(py).get_item(n.as_str())? {
                Some(v) => self.last_of(py, &v)?,
                None => return Err(no_register(n)),
            },
            trex::Field::Item(n, i) => match self.captures.bind(py).get_item(n.as_str())? {
                Some(v) if v.is_instance_of::<PyList>() => v.get_item(*i)?.unbind(),
                Some(_) => {
                    return Err(PyValueError::new_err(format!(
                        "{n:?} is bound under no repetition, so it has no index"
                    )));
                }
                None => return Err(no_register(n)),
            },
        };
        let base = base.bind(py);
        if base.is_instance_of::<PyString>() {
            let s: &Bound<'_, PyString> = base.cast()?;
            return Ok(PyString::new(py, &reference.apply(s.to_str()?)).into_any().unbind());
        }
        let b: &Bound<'_, PyBytes> = base.cast()?;
        let sliced = reference.apply(&String::from_utf8_lossy(b.as_bytes()));
        Ok(PyBytes::new(py, sliced.as_bytes()).into_any().unbind())
    }

    /// `${name[*]}` over this match: every binding of `name` read through
    /// the reference's accessors and joined with a comma, in the input's
    /// type; a register bound under no repetition is its one binding.
    fn every_binding(&self, py: Python<'_>, name: &str, reference: &trex::Reference) -> PyResult<Py<PyAny>> {
        let Some(value) = self.captures.bind(py).get_item(name)? else {
            return Err(no_register(name));
        };
        let items: Vec<Bound<'_, PyAny>> = if value.is_instance_of::<PyList>() {
            value.cast::<PyList>()?.iter().collect()
        } else {
            vec![value]
        };
        let mut texts: Vec<String> = Vec::with_capacity(items.len());
        for item in &items {
            if item.is_instance_of::<PyString>() {
                texts.push(reference.apply(item.cast::<PyString>()?.to_str()?));
            } else {
                texts.push(reference.apply(&String::from_utf8_lossy(item.cast::<PyBytes>()?.as_bytes())));
            }
        }
        let joined = texts.join(",");
        if self.text.bind(py).is_instance_of::<PyBytes>() {
            Ok(PyBytes::new(py, joined.as_bytes()).into_any().unbind())
        } else {
            Ok(PyString::new(py, &joined).into_any().unbind())
        }
    }

    /// A register's value as one text: the value itself, or the last item of
    /// a register bound under a repetition, and nothing in the input's type
    /// where it made no binding.
    fn last_of(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if !value.is_instance_of::<PyList>() {
            return Ok(value.clone().unbind());
        }
        let list: &Bound<'_, PyList> = value.cast()?;
        match list.len() {
            0 => {
                if self.text.bind(py).is_instance_of::<PyBytes>() {
                    Ok(PyBytes::new(py, b"").into_any().unbind())
                } else {
                    Ok(PyString::new(py, "").into_any().unbind())
                }
            }
            n => Ok(list.get_item(n - 1)?.unbind()),
        }
    }

    /// The input this match was found in, `bytes`, refused where the match
    /// does not fit inside it.
    fn holding<'b>(&self, bytes: &'b [u8]) -> PyResult<&'b [u8]> {
        if self.byte_end > bytes.len() {
            return Err(PyValueError::new_err(format!(
                "the match ends at byte {} and the input given holds {}; give the text it was found in",
                self.byte_end,
                bytes.len()
            )));
        }
        Ok(bytes)
    }

    /// Why this match matched in `bytes`, the input it was found in, as
    /// `scan --explain` reads it: the route by scanning the input again with
    /// the trace recording, and the readings by the explainer.
    fn explanation(&self, py: Python<'_>, bytes: &[u8]) -> PyResult<trex::explain::Explanation> {
        let bytes = self.holding(bytes)?;
        let read = &*self.pattern;
        Ok(py.detach(|| {
            trex::trace::clear();
            let route = {
                // The recording is held to the end of this block, so the
                // rescan's rungs are kept; the rescan runs for the rung it
                // records.
                let _recording = trex::trace::Recording::start();
                let _rescanned = if read.shapes.is_empty() {
                    trex::scan_with_backend(&read.pattern, bytes, trex::Backend::Auto).0
                } else {
                    trex::scan_with_shapes(&read.pattern, bytes, &read.shapes)
                };
                trex::explain::route_of(&trex::trace::take_recorded())
            };
            trex::explain::Explainer::new(&read.pattern, bytes, &read.shapes).explain(&self.inner, &route)
        }))
    }
}

#[pymethods]
impl Match {
    /// `(start, end)` in the input's units.
    fn span(&self) -> (usize, usize) {
        (self.start, self.end)
    }

    /// `(byte_start, byte_end)`.
    fn byte_span(&self) -> (usize, usize) {
        (self.byte_start, self.byte_end)
    }

    /// The text the register `name` bound, or the slice a template
    /// reference such as `e:domain` or `0:last4` names.
    fn group(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        self.read(py, name)
    }

    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        self.read(py, name)
    }

    /// `(start, end)` of what the register `name` bound, in the input's units.
    fn capture_span(&self, name: &str) -> PyResult<(usize, usize)> {
        self.spans.iter().find(|s| s.0 == name).map(|s| (s.1, s.2)).ok_or_else(|| no_register(name))
    }

    /// `(byte_start, byte_end)` of what the register `name` bound.
    fn capture_byte_span(&self, name: &str) -> PyResult<(usize, usize)> {
        self.spans.iter().find(|s| s.0 == name).map(|s| (s.3, s.4)).ok_or_else(|| no_register(name))
    }

    /// Why this match matched, as a dict.
    ///
    /// `tokens` each significant token the match spans, as `(kind, text)`;
    /// `guards` what each guarded token passed to be its kind; `readings`
    /// each axis the pattern reads, at each token, as `(axis, text, value)`;
    /// and `route` the rung of the scan ladder that answered, read by
    /// scanning `input` again with the trace recording, as `scan --explain`
    /// reads it.
    ///
    /// `input` is the text this match was found in, and is taken again rather
    /// than held: keeping it on every match would copy the whole input per
    /// scan for a report most callers never ask for. Passing a different text
    /// explains the match against that text, which is a question about
    /// offsets, not about this match; a text too short to hold it is a
    /// `ValueError`.
    fn explain(&self, py: Python<'_>, input: &Bound<'_, PyAny>) -> PyResult<Py<PyDict>> {
        let held = Input::of(input)?;
        explanation_dict(py, &self.explanation(py, held.bytes())?)
    }

    /// The parsed value the register `name` bound, in its base unit, or
    /// `None` where it binds no single typed kind or its text does not parse
    /// as one.
    ///
    /// A byte size is given in bytes, a duration in nanoseconds, a timestamp
    /// as a timezone-aware `datetime` in UTC, money and a percentage as `Decimal`
    /// so the digits past the point survive, an address as an `int` of its
    /// 32 or 128 bits, a version as its parts and its pre-release
    /// identifiers. This is the read a `:value` clause makes, so a value here
    /// and a predicate selecting on one cannot disagree.
    ///
    /// A duration is held and compared as nanoseconds, so that is what it
    /// answers; `unit="ms"` or `unit="s"` shifts it exactly for a reader,
    /// and `1500ms` in seconds is `Decimal("1.5")`, never a rounded float.
    ///
    /// A register bound under a repetition answers a list, one value per
    /// binding, matching what `captures` holds for it. An unknown register
    /// is a `KeyError`.
    #[pyo3(signature = (name, unit = "ns"))]
    fn value(&self, py: Python<'_>, name: &str, unit: &str) -> PyResult<Py<PyAny>> {
        let duration = duration_unit_of("unit", unit)?;
        if !self.spans.iter().any(|s| s.0 == name) {
            return Err(no_register(name));
        }
        let Some(kind) = self.kinds.iter().find(|(n, _)| n == name).and_then(|(_, k)| *k) else {
            return Ok(py.None());
        };
        let read = |item: &Bound<'_, PyAny>| -> PyResult<Py<PyAny>> {
            // A str input reports its captures as str and a bytes input as
            // bytes, so the text is taken by which one this is.
            let text = if item.is_instance_of::<PyString>() {
                item.extract::<String>()?
            } else {
                String::from_utf8_lossy(&item.extract::<Vec<u8>>()?).into_owned()
            };
            match trex::typed::value_of(kind, &text) {
                Some(v) => Match::py_value(py, kind, &v, duration),
                None => Ok(py.None()),
            }
        };
        let bound = self.captures.bind(py);
        let Some(item) = bound.get_item(name)? else {
            return Ok(py.None());
        };
        // A register bound under a repetition holds a list of its bindings,
        // so it answers a list of their values.
        if item.is_instance_of::<PyList>() {
            let list = item.cast::<PyList>()?;
            let mut out: Vec<Py<PyAny>> = Vec::with_capacity(list.len());
            for one in list.iter() {
                out.push(read(&one)?);
            }
            return Ok(PyList::new(py, out)?.into_any().unbind());
        }
        read(&item)
    }

    /// The name of the member of a `PatternSet` that made this match;
    /// `None` for a match of one pattern.
    #[getter]
    fn pattern(&self) -> Option<String> {
        self.pattern.member.clone()
    }

    /// The kind of token the register `name` binds, such as `"ip"` or
    /// `"timestamp"`, a declared shape or kind by the name its declaration
    /// gave it; `None` where it binds more than one kind. An unknown register
    /// is a `KeyError`.
    fn kind(&self, name: &str) -> PyResult<Option<String>> {
        if !self.spans.iter().any(|s| s.0 == name) {
            return Err(no_register(name));
        }
        let kind = self.kinds.iter().find(|(n, _)| n == name).and_then(|(_, k)| *k);
        Ok(kind.map(|k| self.pattern.shapes.kind_name(k)))
    }

    /// `template`, a report template, rendered at this match, as `scan
    /// --format` renders one: `${0}`, a register by name with its accessors
    /// (`${ip:octet1-2}`), `${line}`, `${col}`, `${path}` and `${pattern}`,
    /// `${start}` and `${end}` in the input's units, and `${@axis}`, a
    /// reading `--explain` computes. `input` is the text this match was found
    /// in, taken again as `explain` takes it. A template that does not parse,
    /// or an input too short to hold the match, is a `ValueError`.
    fn format(&self, py: Python<'_>, template: &str, input: &Bound<'_, PyAny>) -> PyResult<String> {
        let t = trex::Template::parse_report(template, &self.bound)
            .map_err(|e| PyValueError::new_err(format!("template error at byte {}: {}", e.pos, e.msg)))?;
        let held = Input::of(input)?;
        let bytes = self.holding(held.bytes())?;
        let at = trex::ReportAt {
            path: self.path.as_deref().unwrap_or(""),
            line: self.line,
            col: self.column,
            offsets: Some((self.start, self.end)),
            pattern: self.pattern.member.as_deref(),
            rule: None,
        };
        if t.reads_explanation() {
            let why = self.explanation(py, bytes)?;
            return Ok(t.render_explained(&self.inner, bytes, Some(&at), &why));
        }
        Ok(t.render_report(&self.inner, bytes, &at))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("Match(start={}, end={}, text={})", self.start, self.end, self.text.bind(py).repr()?))
    }
}

/// The matches of one scan, handed out one at a time.
#[pyclass(module = "trex")]
struct Matches {
    items: std::vec::IntoIter<Match>,
}

#[pymethods]
impl Matches {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<Match> {
        self.items.next()
    }

    fn __len__(&self) -> usize {
        self.items.len()
    }
}

/// One side of a grep's context: a count of lines, or a record unit's name
/// for the record a line is in.
#[derive(Clone, FromPyObject)]
enum Side {
    Count(usize),
    Unit(String),
}

/// The lines a grep carries around each line it selects: each side a count
/// of lines, or the rest of the record of one unit holding the line.
struct GrepContext {
    before: usize,
    after: usize,
    unit: Option<(trex::records::RecordUnit, bool, bool)>,
}

impl GrepContext {
    /// From `before=`, `after=` and `context=`, which gives both sides
    /// where `before=` and `after=` give none: one record unit at most, and
    /// not `record`, which a grep has nothing to define.
    fn of(before: Side, after: Side, context: Option<Side>) -> PyResult<Self> {
        let side = |given: Side| match (given, &context) {
            (Side::Count(0), Some(both)) => both.clone(),
            (given, _) => given,
        };
        let mut out = GrepContext { before: 0, after: 0, unit: None };
        let mut named: Option<String> = None;
        for (ahead, given) in [(true, side(before)), (false, side(after))] {
            let name = match given {
                Side::Count(n) => {
                    if ahead {
                        out.before = n;
                    } else {
                        out.after = n;
                    }
                    continue;
                }
                Side::Unit(name) => name,
            };
            if name == "record" {
                return Err(PyValueError::new_err(
                    "a grep takes no record=, so its context names a record unit itself: line, paragraph, block, unit, unit:ROLE and the rest",
                ));
            }
            match &named {
                Some(earlier) if *earlier != name => {
                    return Err(PyValueError::new_err(format!(
                        "the context names one unit; {earlier:?} and {name:?} were both given"
                    )));
                }
                Some(_) => {}
                None => {
                    let unit = trex::records::RecordUnit::parse(&name).map_err(|e| {
                        PyValueError::new_err(format!("before=, after= and context= take a count or a record unit: {e}"))
                    })?;
                    out.unit = Some((unit, false, false));
                    named = Some(name);
                }
            }
            if let Some((_, b, a)) = &mut out.unit {
                if ahead {
                    *b = true;
                } else {
                    *a = true;
                }
            }
        }
        Ok(out)
    }

    /// The context as trex's reports read it.
    fn context(&self) -> trex::report::Context<'_> {
        trex::report::Context {
            before: self.before,
            after: self.after,
            record: self.unit.as_ref().map(|(unit, before, after)| (unit, *before, *after)),
        }
    }
}

/// The options of one grep, applied to each input: the lines around each
/// line it selects, whether it selects the lines no match touches, whether a
/// match must cover its line, how many matches or lines it takes, and how the
/// scan runs.
struct GrepAsk {
    context: GrepContext,
    invert: bool,
    whole_line: bool,
    max_count: Option<usize>,
    engine: trex::Engine,
}

/// What comes before the part of an input a grep read: its bytes, its
/// units and its lines.
#[derive(Clone, Copy)]
struct Ahead {
    bytes: usize,
    units: usize,
    lines: usize,
}

/// One line a grep gives: the file it is in, its number from 1, its text in
/// the input's type without its line ending, whether the grep selected it
/// rather than carrying it as context, and the matches that start on it.
#[pyclass(frozen, module = "trex")]
struct Line {
    /// The file the line is in; `None` for text.
    #[pyo3(get)]
    path: Option<String>,
    /// The line's number, from 1, as the input numbers it.
    #[pyo3(get)]
    number: usize,
    /// The line's text, in the input's type, without its line ending.
    #[pyo3(get)]
    text: Py<PyAny>,
    /// Whether the grep selected the line, a line a match starts on or under
    /// `invert=True` one no match touches; `False` for a context line.
    #[pyo3(get)]
    is_match: bool,
    matches: Vec<Py<Match>>,
}

#[pymethods]
impl Line {
    /// The matches that start on the line, in order; empty for a context
    /// line and for a line selected because no match touches it.
    #[getter]
    fn matches(&self, py: Python<'_>) -> Vec<Py<Match>> {
        self.matches.iter().map(|m| m.clone_ref(py)).collect()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let path = match &self.path {
            Some(p) => PyString::new(py, p).repr()?.to_string(),
            None => "None".to_string(),
        };
        Ok(format!(
            "Line(path={path}, number={}, is_match={}, text={})",
            self.number,
            if self.is_match { "True" } else { "False" },
            self.text.bind(py).repr()?
        ))
    }
}

/// The lines of a grep, handed out one at a time: the files it names read
/// one after another as their lines are taken, and the grep's statistics
/// kept as this thread's last scan once the last file is read.
#[pyclass(module = "trex")]
struct Lines {
    ready: std::collections::VecDeque<Line>,
    files: std::collections::VecDeque<(String, std::path::PathBuf)>,
    /// Whether one file was named alone, which a NUL byte refuses aloud.
    lone: bool,
    pattern: Py<Pattern>,
    reading: rules::Reading,
    ask: GrepAsk,
    /// What the grep has read so far, until it is kept as the last scan.
    stats: Option<ScanStats>,
    started: std::time::Instant,
}

#[pymethods]
impl Lines {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Line>> {
        loop {
            if let Some(line) = self.ready.pop_front() {
                return Ok(Some(line));
            }
            let Some((name, file)) = self.files.pop_front() else {
                if let Some(mut stats) = self.stats.take() {
                    stats.elapsed = self.started.elapsed();
                    publish_stats(stats);
                }
                return Ok(None);
            };
            let reading = &self.reading;
            let part = py
                .detach(|| rules::Part::of_file(&file, reading))
                .map_err(|e| PyOSError::new_err(format!("{name}: {e}")))?;
            // As a scan does: a file named alone that holds a NUL byte is
            // refused aloud, and one among several or found by a walk is
            // passed over.
            if part.binary && !self.reading.binary {
                if self.lone {
                    return Err(PyValueError::new_err(format!(
                        "{name} holds a NUL byte and is binary; binary=True reads it"
                    )));
                }
                continue;
            }
            let ahead = Ahead { bytes: part.byte_base, units: part.units_ahead, lines: part.line_base.unwrap_or(0) };
            let text = match String::from_utf8(part.text) {
                Ok(text) => text,
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
            };
            let Some(stats) = self.stats.as_mut() else {
                return Ok(None);
            };
            let lines =
                grep_input(py, self.pattern.get(), &Input::Text(&text), Some(name), ahead, &self.ask, stats)?;
            self.ready.extend(lines);
        }
    }
}

/// The lines a grep of `pattern` gives over `shown`, the part of an input
/// read, the file named `path` where it is one, `ahead` coming before it,
/// as `ask` asks; what it read added to `stats`.
fn grep_input(
    py: Python<'_>,
    pattern: &Pattern,
    shown: &Input<'_>,
    path: Option<String>,
    ahead: Ahead,
    ask: &GrepAsk,
    stats: &mut ScanStats,
) -> PyResult<Vec<Line>> {
    let bytes = shown.bytes();
    let count = StatsCount::start();
    let (mut found, _ran) = pattern.within_by(py, shown, 0..bytes.len(), &ask.engine);
    let index = trex::files::LineIndex::new(bytes).within(Some(ahead.bytes), Some(ahead.lines));
    if ask.whole_line {
        found.retain(|m| trex::report::covers_its_line(bytes, &index, m));
    }
    if !ask.invert
        && let Some(n) = ask.max_count
    {
        found.truncate(n);
    }
    count.add_to(stats, &[(bytes, bytes.len(), &found)]);
    let context = ask.context.context();
    let lines = py.detach(|| trex::report::grep_lines(bytes, &found, &index, &context, ask.invert, ask.max_count));
    let bound = pattern.bound();
    let kinds = pattern.kinds();
    let shared = pattern.shared();
    let mut placing = Placing::of(shown);
    placing.path = path.clone();
    placing.bytes_ahead = ahead.bytes;
    placing.units_ahead = ahead.units;
    placing.lines_ahead = ahead.lines;
    let mut out = Vec::with_capacity(lines.len());
    for g in lines {
        let (s, e) = index.line_span(g.line);
        let e = if e > s && bytes[e - 1] == b'\r' { e - 1 } else { e };
        let mut matches = Vec::with_capacity(g.matches.len());
        for m in &found[g.matches] {
            matches.push(Py::new(py, Match::of(py, shown, &mut placing, m, &bound, &kinds, &shared)?)?);
        }
        out.push(Line {
            path: path.clone(),
            number: index.number(g.line),
            text: shown.slice(py, s, e),
            is_match: g.selected,
            matches,
        });
    }
    Ok(out)
}

/// A file `trex.follow` follows: its name, the stream of its text, the
/// decoder its bytes pass through, the opening of a character a piece ended
/// inside, the characters of its text pushed so far, and how many matches it
/// has given.
struct FollowedText {
    name: String,
    stream: trex::HeldStream,
    decoder: trex::encoding::Incremental,
    carry: Vec<u8>,
    chars_through: usize,
    given: usize,
}

/// The stream a file is followed through by `pattern`, from `origin` after
/// `lines_before` of its lines.
fn followed_stream(pattern: &Pattern, origin: usize, lines_before: Option<usize>) -> trex::HeldStream {
    let scanner = if pattern.shaped() {
        trex::StreamScanner::with_shapes(pattern.inner.clone(), pattern.shapes.clone())
    } else {
        trex::StreamScanner::new(pattern.inner.clone())
    };
    trex::HeldStream::new(scanner, origin, lines_before)
}

/// The matches `committed`, spans over `held`, the bytes a followed file's
/// stream holds from byte `base` after `lines_before` of its lines and
/// through `chars_through` characters of its text, as `Match` objects of the
/// file `name`.
// The eight are one batch and everything that places it: the pattern, the
// file, the bytes held and the three counts of what comes before them.
#[allow(clippy::too_many_arguments)]
fn matches_held(
    py: Python<'_>,
    pattern: &Pattern,
    name: &str,
    held: &[u8],
    base: usize,
    lines_before: Option<usize>,
    chars_through: usize,
    committed: &[(usize, trex::Span)],
) -> PyResult<Vec<Match>> {
    if committed.is_empty() {
        return Ok(Vec::new());
    }
    let spans: Vec<trex::Span> = committed.iter().map(|&(_, span)| span).collect();
    let found = py.detach(|| pattern.resolved(held, &spans));
    let text = String::from_utf8_lossy(held);
    let shown = Input::Text(&text);
    let mut placing = Placing::of(&shown);
    placing.path = Some(name.to_string());
    placing.bytes_ahead = base;
    placing.units_ahead = chars_through - OffsetUnit::CodePoints.count(held);
    placing.lines_ahead = lines_before.unwrap_or(0);
    let bound = pattern.bound();
    let kinds = pattern.kinds();
    let shared = pattern.shared();
    found.iter().map(|m| Match::of(py, &shown, &mut placing, m, &bound, &kinds, &shared)).collect()
}

/// Files a pattern follows as they grow, each match handed out once nothing
/// that arrives later can change it, as `scan --follow` prints it.
///
/// Iterating it waits for the files to grow, a second at a time, and ends
/// only once `max_count` matches have come from every file; interrupting it
/// stops the wait. A file truncated, replaced or removed is told as a
/// `TrexWarning` and scanned again from its start. It is iterated in the
/// thread that made it, since the change notifications it waits on are
/// received by one thread.
#[pyclass(module = "trex", unsendable)]
struct Following {
    pattern: Py<Pattern>,
    files: Vec<FollowedText>,
    follower: Option<trex::follow::Follower>,
    ready: std::collections::VecDeque<Match>,
    cap: Option<usize>,
    /// Whether a file truncated, replaced or removed keeps its count toward
    /// `cap` rather than starting it again.
    keep_count: bool,
}

impl Following {
    /// Whether every file has given all `max_count` lets it.
    fn done(&self) -> bool {
        self.follower.is_none() || self.cap.is_some_and(|n| self.files.iter().all(|f| f.given >= n))
    }

    /// Queue `found`, matches of file `k`, as many as `max_count` leaves it.
    fn queue(&mut self, k: usize, mut found: Vec<Match>) {
        let file = &mut self.files[k];
        if let Some(n) = self.cap {
            found.truncate(n.saturating_sub(file.given));
        }
        file.given += found.len();
        self.ready.extend(found);
    }

    /// Scan `bytes`, what file `k` gained, decoded as its opening declared; a
    /// character cut between two pieces is held until the rest arrives.
    fn push(&mut self, py: Python<'_>, k: usize, bytes: &[u8]) -> PyResult<()> {
        let file = &mut self.files[k];
        let mut text = std::mem::take(&mut file.carry);
        text.extend_from_slice(&file.decoder.decode(bytes));
        let whole = trex::encoding::complete_prefix(&text);
        file.carry = text.split_off(whole);
        self.push_text(py, k, &text)
    }

    /// Scan `text`, file `k`'s next whole characters.
    fn push_text(&mut self, py: Python<'_>, k: usize, text: &[u8]) -> PyResult<()> {
        let text = String::from_utf8_lossy(text);
        let pattern = self.pattern.get();
        let file = &mut self.files[k];
        file.chars_through += OffsetUnit::CodePoints.count(text.as_bytes());
        let committed = file.stream.push(text.as_bytes());
        let found = matches_held(
            py,
            pattern,
            &file.name,
            file.stream.held(),
            file.stream.base(),
            file.stream.lines_before(),
            file.chars_through,
            &committed,
        )?;
        self.queue(k, found);
        Ok(())
    }

    /// File `k` was truncated, replaced or removed: queue what its stream held
    /// until then, and begin it again at the start of the file now under its
    /// name, a new input whose count starts again unless `keep_count` carries it on.
    fn restart(&mut self, py: Python<'_>, k: usize) -> PyResult<()> {
        let pattern = self.pattern.get();
        let file = &mut self.files[k];
        let fresh = followed_stream(pattern, 0, Some(0));
        let ended = std::mem::replace(&mut file.stream, fresh).finish();
        let found = matches_held(
            py,
            pattern,
            &file.name,
            &ended.held,
            ended.base,
            ended.lines_before,
            file.chars_through,
            &ended.matches,
        )?;
        self.queue(k, found);
        let keep_count = self.keep_count;
        let file = &mut self.files[k];
        file.chars_through = 0;
        file.carry.clear();
        if !keep_count {
            file.given = 0;
        }
        file.decoder = trex::encoding::Incremental::from_start();
        Ok(())
    }
}

#[pymethods]
impl Following {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Match>> {
        loop {
            if let Some(found) = self.ready.pop_front() {
                return Ok(Some(found));
            }
            if self.done() {
                return Ok(None);
            }
            let Some(follower) = self.follower.as_mut() else {
                return Ok(None);
            };
            let polled = py
                .detach(|| follower.poll())
                .map_err(|e| PyOSError::new_err(format!("cannot follow {e}")))?;
            py.check_signals()?;
            let Some((k, change)) = polled else {
                continue;
            };
            let name = self.files[k].name.clone();
            match change {
                trex::follow::Followed::Appended { bytes, .. } => self.push(py, k, &bytes)?,
                trex::follow::Followed::Truncated => {
                    rules::warn(py, &format!("{name}: truncated; scanning it from its start"))?;
                    self.restart(py, k)?;
                }
                trex::follow::Followed::Replaced => {
                    rules::warn(py, &format!("{name}: replaced by another file; scanning that from its start"))?;
                    self.restart(py, k)?;
                }
                trex::follow::Followed::Gone => {
                    rules::warn(py, &format!("{name}: removed; waiting for a file under its name"))?;
                    self.restart(py, k)?;
                }
            }
        }
    }
}

/// The matches of `pattern`, a `Pattern` or its source read under `lib`, in
/// the files `paths` name, as they grow, each handed out once nothing that
/// arrives later can change it, with its file in `path`; as `scan --follow`
/// prints them.
///
/// `from_end=True` follows each file from where it ends now, and
/// `from_end=False` scans it from its start first. A directory is walked as
/// `trex.files` walks one. `max_count=N` ends the follow once every file has
/// given N matches; a file truncated, replaced or removed is told as a
/// `TrexWarning` and read again from its start, its count starting again
/// unless `keep_count=True`.
#[pyfunction]
#[pyo3(signature = (pattern, *paths, from_end = true, max_count = None, keep_count = false, lib = None))]
fn follow(
    py: Python<'_>,
    pattern: &Bound<'_, PyAny>,
    paths: &Bound<'_, PyTuple>,
    from_end: bool,
    max_count: Option<usize>,
    keep_count: bool,
    lib: Option<&Bound<'_, PyAny>>,
) -> PyResult<Following> {
    if keep_count && max_count.is_none() {
        return Err(PyValueError::new_err("keep_count= keeps the count max_count= caps, and no max_count was given"));
    }
    let pattern = followed_pattern(py, pattern, lib)?;
    let roots: Vec<std::path::PathBuf> = paths
        .iter()
        .map(|p| p.extract())
        .collect::<PyResult<_>>()
        .map_err(|e| PyTypeError::new_err(format!("each path is a str or an os.PathLike: {e}")))?;
    if roots.is_empty() {
        return Err(PyTypeError::new_err("follow() follows the files the paths name, and none was given"));
    }
    let compiled = pattern.get();
    if !followed_stream(compiled, 0, None).commits_early() {
        return Err(PyValueError::new_err(
            "no match of this pattern is final before its input ends, since it reads a content guard, a field anchor or a whole-input axis, and a followed file does not end; scan it without following it",
        ));
    }
    let reading =
        rules::Reading::of(None, from_end.then_some(0), None, "line", false, false, None, false)?;
    let walked = rules::walked(py, PathArg::Many(roots), &reading.walk)?;
    if !walked.errors.is_empty() {
        return Err(PyOSError::new_err(walked.errors.join("; ")));
    }
    let mut following = Following {
        pattern: pattern.clone_ref(py),
        files: Vec::new(),
        follower: None,
        ready: std::collections::VecDeque::new(),
        cap: max_count,
        keep_count,
    };
    let mut watched: Vec<(std::path::PathBuf, usize)> = Vec::new();
    for (name, path) in &walked.files {
        let part = py
            .detach(|| rules::Part::of_file(path, &reading))
            .map_err(|e| PyOSError::new_err(format!("{name}: {e}")))?;
        if part.binary {
            if walked.lone {
                return Err(PyValueError::new_err(format!("{name} holds a NUL byte and is binary; a follow reads text")));
            }
            continue;
        }
        following.files.push(FollowedText {
            name: name.clone(),
            stream: followed_stream(compiled, part.byte_base, part.line_base),
            decoder: trex::encoding::Incremental::after(part.encoding),
            carry: Vec::new(),
            chars_through: part.units_ahead,
            given: 0,
        });
        let k = following.files.len() - 1;
        following.push_text(py, k, &part.text)?;
        watched.push((path.clone(), part.input_len));
    }
    if following.files.is_empty() {
        return Ok(following);
    }
    let follower =
        trex::follow::Follower::new(&watched).map_err(|e| PyOSError::new_err(format!("cannot follow {e}")))?;
    if let Some(why) = follower.unnotified() {
        rules::warn(py, &format!("no change notifications ({why}); looking at the files once a second"))?;
    }
    following.follower = Some(follower);
    Ok(following)
}

/// The pattern a follow reads: a `Pattern`, or its source read under `lib`.
fn followed_pattern(py: Python<'_>, pattern: &Bound<'_, PyAny>, lib: Option<&Bound<'_, PyAny>>) -> PyResult<Py<Pattern>> {
    if pattern.is_instance_of::<Pattern>() {
        if lib.is_some() {
            return Err(PyValueError::new_err(
                "lib= reads a pattern given as text, and this Pattern was read under its own declarations",
            ));
        }
        return Ok(pattern.cast::<Pattern>()?.clone().unbind());
    }
    let source: String = pattern
        .extract()
        .map_err(|e| PyTypeError::new_err(format!("the pattern is a trex.Pattern or its source: {e}")))?;
    Py::new(py, Pattern::new(&source, lib)?)
}

/// The window a follow reads a file from before following it: `tail` or an
/// open `lines` range, in records of `unit`; refused where it ends before
/// the file does, which a followed file never stops growing past.
fn followed_window(tail: Option<usize>, lines: Option<&str>, unit: &str) -> PyResult<rules::Reading> {
    let reading = rules::Reading::of(None, tail, lines, unit, false, false, None, false)?;
    if reading.select.is_some_and(|w| !w.runs_to_end()) {
        return Err(PyValueError::new_err(
            "lines= with a last line ends before the file does, and a followed file grows past it; follow a tail= or a lines= range with no last line",
        ));
    }
    Ok(reading)
}

/// What a file a follow reads came to when it changed other than by
/// growing, as a `TrexWarning` says it; nothing for a file that grew.
fn change_told(name: &str, change: &trex::follow::Followed) -> Option<String> {
    match change {
        trex::follow::Followed::Appended { .. } => None,
        trex::follow::Followed::Truncated => Some(format!("{name}: truncated; reading it from its start")),
        trex::follow::Followed::Replaced => {
            Some(format!("{name}: replaced by another file; reading that from its start"))
        }
        trex::follow::Followed::Gone => Some(format!("{name}: removed; waiting for a file under its name")),
    }
}

/// The text `pending` holds up to its last line ending, as lines without
/// their endings, `pending` keeping the line whose end has not arrived.
fn ended_lines(pending: &mut String) -> Vec<String> {
    let Some(last) = pending.rfind('\n') else {
        return Vec::new();
    };
    let done: String = pending.drain(..=last).collect();
    done[..last].split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line).to_string()).collect()
}

/// A file `trex.follow_lines` follows: its name, the decoder its bytes pass
/// through, the opening of a character a piece ended inside, the text of a
/// line whose end has not arrived, and the number the next line takes.
struct FollowedLines {
    name: String,
    decoder: trex::encoding::Incremental,
    carry: Vec<u8>,
    pending: String,
    next: usize,
}

impl FollowedLines {
    /// The lines `text`, the file's next whole characters, ends.
    fn lines_of(&mut self, py: Python<'_>, text: &str) -> Vec<Line> {
        self.pending.push_str(text);
        ended_lines(&mut self.pending).into_iter().map(|line| self.line(py, &line)).collect()
    }

    /// `text` as this file's next line.
    fn line(&mut self, py: Python<'_>, text: &str) -> Line {
        let number = self.next;
        self.next += 1;
        Line {
            path: Some(self.name.clone()),
            number,
            text: PyString::new(py, text).into_any().unbind(),
            is_match: true,
            matches: Vec::new(),
        }
    }
}

/// Files followed line by line as they grow, as `trex tail -f` prints them.
///
/// Iterating it waits for the files to grow, a second at a time, until it is
/// interrupted. A file truncated, replaced or removed is told as a
/// `TrexWarning`, its last line given whether or not its end arrived, and it
/// is read again from its first line. It is iterated in the thread that made
/// it.
#[pyclass(module = "trex", unsendable)]
struct FollowingLines {
    files: Vec<FollowedLines>,
    follower: Option<trex::follow::Follower>,
    ready: std::collections::VecDeque<Line>,
}

#[pymethods]
impl FollowingLines {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Line>> {
        loop {
            if let Some(line) = self.ready.pop_front() {
                return Ok(Some(line));
            }
            let Some(follower) = self.follower.as_mut() else {
                return Ok(None);
            };
            let polled = py
                .detach(|| follower.poll())
                .map_err(|e| PyOSError::new_err(format!("cannot follow {e}")))?;
            py.check_signals()?;
            let Some((k, change)) = polled else {
                continue;
            };
            let file = &mut self.files[k];
            if let trex::follow::Followed::Appended { bytes, .. } = &change {
                let mut text = std::mem::take(&mut file.carry);
                text.extend_from_slice(&file.decoder.decode(bytes));
                let whole = trex::encoding::complete_prefix(&text);
                file.carry = text.split_off(whole);
                let lines = file.lines_of(py, &String::from_utf8_lossy(&text));
                self.ready.extend(lines);
                continue;
            }
            if let Some(told) = change_told(&file.name, &change) {
                rules::warn(py, &told)?;
            }
            if !file.pending.is_empty() {
                let last = std::mem::take(&mut file.pending);
                let line = file.line(py, last.strip_suffix('\r').unwrap_or(&last));
                self.ready.push_back(line);
            }
            file.next = 1;
            file.carry.clear();
            file.decoder = trex::encoding::Incremental::from_start();
        }
    }
}

/// The lines the files `paths` name gain as they grow, each a `Line` with
/// its file and its number in the file, after the file's last `tail` lines
/// or its open `lines` range, `"100.."`, counted in records of `unit`; as
/// `trex tail -f` and Get-TrexLine -Follow write them. One of `tail` and
/// `lines` is given, `tail=0` following each file from its end. A directory
/// is walked as `trex.files` walks one; a file holding a NUL byte is passed
/// over, one named alone raising `ValueError`.
#[pyfunction]
#[pyo3(signature = (*paths, tail = None, lines = None, unit = "line"))]
fn follow_lines(
    py: Python<'_>,
    paths: &Bound<'_, PyTuple>,
    tail: Option<usize>,
    lines: Option<&str>,
    unit: &str,
) -> PyResult<FollowingLines> {
    if tail.is_none() && lines.is_none() {
        return Err(PyValueError::new_err(
            "follow_lines() writes what each file gains after its tail= or its lines= range, and neither was given; tail=0 follows from the end",
        ));
    }
    let reading = followed_window(tail, lines, unit)?;
    let roots: Vec<std::path::PathBuf> = paths
        .iter()
        .map(|p| p.extract())
        .collect::<PyResult<_>>()
        .map_err(|e| PyTypeError::new_err(format!("each path is a str or an os.PathLike: {e}")))?;
    if roots.is_empty() {
        return Err(PyTypeError::new_err("follow_lines() follows the files the paths name, and none was given"));
    }
    let walked = rules::walked(py, PathArg::Many(roots), &reading.walk)?;
    if !walked.errors.is_empty() {
        return Err(PyOSError::new_err(walked.errors.join("; ")));
    }
    let mut following = FollowingLines { files: Vec::new(), follower: None, ready: std::collections::VecDeque::new() };
    let mut watched: Vec<(std::path::PathBuf, usize)> = Vec::new();
    for (name, path) in &walked.files {
        let part = py
            .detach(|| rules::Part::of_file(path, &reading))
            .map_err(|e| PyOSError::new_err(format!("{name}: {e}")))?;
        if part.binary {
            if walked.lone {
                return Err(PyValueError::new_err(format!("{name} holds a NUL byte and is binary; a follow reads text")));
            }
            continue;
        }
        let mut file = FollowedLines {
            name: name.clone(),
            decoder: trex::encoding::Incremental::after(part.encoding),
            carry: Vec::new(),
            pending: String::new(),
            next: part.line_base.unwrap_or(0) + 1,
        };
        let lines = file.lines_of(py, &String::from_utf8_lossy(&part.text));
        following.ready.extend(lines);
        following.files.push(file);
        watched.push((path.clone(), part.input_len));
    }
    if following.files.is_empty() {
        return Ok(following);
    }
    let follower =
        trex::follow::Follower::new(&watched).map_err(|e| PyOSError::new_err(format!("cannot follow {e}")))?;
    if let Some(why) = follower.unnotified() {
        rules::warn(py, &format!("no change notifications ({why}); looking at the files once a second"))?;
    }
    following.follower = Some(follower);
    Ok(following)
}

/// How a followed edit edits each committed match: a template rendered at
/// it, a callable handed it, or a mask with the fields it keeps.
enum FollowEdit {
    Template(trex::Template),
    Callable(Py<PyAny>),
    Mask { keeps: Vec<trex::Keep>, mask: trex::Mask },
}

/// A file a followed rewrite or redaction edits as it grows, each line of the
/// edited text handed out once nothing that arrives later can change it.
///
/// Iterating it waits for the file to grow, a second at a time, until it is
/// interrupted. A file truncated, replaced or removed is told as a
/// `TrexWarning`, the rest of its text edited and its last line given, and it
/// is read again from its start, its count starting again unless
/// `keep_count=True`. It is iterated in the thread that made it.
#[pyclass(module = "trex", unsendable)]
struct FollowingEdit {
    pattern: Py<Pattern>,
    edit: FollowEdit,
    name: String,
    stream: trex::EditStream,
    decoder: trex::encoding::Incremental,
    carry: Vec<u8>,
    /// The characters and the lines of the file's text pushed so far, which
    /// place a match a callable is handed.
    chars_through: usize,
    lines_through: usize,
    pending: String,
    ready: std::collections::VecDeque<String>,
    follower: Option<trex::follow::Follower>,
    limit: Option<usize>,
    keep_count: bool,
}

/// The edits `edit` makes at `spans`, committed matches over `held`, the
/// bytes a followed file's stream holds from byte `base`, `chars_through`
/// characters and `lines_through` lines of the file's text pushed so far.
// The eight are one batch and everything that places it.
#[allow(clippy::too_many_arguments)]
fn edits_held(
    py: Python<'_>,
    pattern: &Pattern,
    edit: &mut FollowEdit,
    name: &str,
    held: &[u8],
    base: usize,
    chars_through: usize,
    lines_through: usize,
    spans: &[trex::Span],
) -> PyResult<Vec<trex::files::Edit>> {
    match edit {
        FollowEdit::Template(t) => Ok(trex::rewrite::edits_at(&pattern.inner, t, held, &pattern.shapes, spans)),
        FollowEdit::Mask { keeps, mask } => {
            let matches = pattern.resolved(held, spans);
            Ok(trex::redactions_with_shapes(held, &matches, keeps, mask, &pattern.inner, &pattern.shapes))
        }
        FollowEdit::Callable(repl) => {
            let committed: Vec<(usize, trex::Span)> = spans.iter().map(|&s| (0, s)).collect();
            let lines_before = lines_through - trex::byte_simd::count_byte(held, b'\n');
            let found = matches_held(py, pattern, name, held, base, Some(lines_before), chars_through, &committed)?;
            let mut edits = Vec::with_capacity(found.len());
            for (m, span) in found.into_iter().zip(spans) {
                let replacement: String = repl.bind(py).call1((m,))?.extract().map_err(|e| {
                    PyTypeError::new_err(format!("the replacement for a followed file's match is a str: {e}"))
                })?;
                edits.push(trex::files::Edit { start: span.start(), end: span.end(), replacement: replacement.into_bytes() });
            }
            Ok(edits)
        }
    }
}

impl FollowingEdit {
    /// Edit `text`, the file's next whole characters, queueing the lines it
    /// completes.
    fn push_text(&mut self, py: Python<'_>, text: &str) -> PyResult<()> {
        self.chars_through += OffsetUnit::CodePoints.count(text.as_bytes());
        self.lines_through += trex::byte_simd::count_byte(text.as_bytes(), b'\n');
        let (chars_through, lines_through) = (self.chars_through, self.lines_through);
        let pattern = self.pattern.get();
        let (name, edit) = (&self.name, &mut self.edit);
        let mut at = |held: &[u8], base: usize, spans: &[trex::Span]| {
            edits_held(py, pattern, edit, name, held, base, chars_through, lines_through, spans)
        };
        let out = self.stream.push(text.as_bytes(), self.limit, &mut at)?;
        self.pending.push_str(&String::from_utf8_lossy(&out));
        self.ready.extend(ended_lines(&mut self.pending));
        Ok(())
    }

    /// The file was truncated, replaced or removed: the rest of its text
    /// edited, every line of it the last whether or not its end arrived, and
    /// the stream begun again at the start of the file now under its name,
    /// its count starting again unless `keep_count` carries it on.
    fn restart(&mut self, py: Python<'_>) -> PyResult<()> {
        let pattern = self.pattern.get();
        let scanner = edit_scanner(pattern);
        let ended = std::mem::replace(&mut self.stream, trex::EditStream::new(scanner, 0, 0));
        let (chars_through, lines_through) = (self.chars_through, self.lines_through);
        let (name, edit) = (&self.name, &mut self.edit);
        let mut at = |held: &[u8], base: usize, spans: &[trex::Span]| {
            edits_held(py, pattern, edit, name, held, base, chars_through, lines_through, spans)
        };
        let (out, edited) = ended.finish(self.limit, &mut at)?;
        if self.keep_count {
            self.stream = trex::EditStream::new(edit_scanner(pattern), 0, edited);
        }
        self.pending.push_str(&String::from_utf8_lossy(&out));
        self.ready.extend(ended_lines(&mut self.pending));
        if !self.pending.is_empty() {
            let last = std::mem::take(&mut self.pending);
            self.ready.push_back(last.strip_suffix('\r').unwrap_or(&last).to_string());
        }
        self.chars_through = 0;
        self.lines_through = 0;
        self.carry.clear();
        self.decoder = trex::encoding::Incremental::from_start();
        Ok(())
    }
}

#[pymethods]
impl FollowingEdit {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<String>> {
        loop {
            if let Some(line) = self.ready.pop_front() {
                return Ok(Some(line));
            }
            let Some(follower) = self.follower.as_mut() else {
                return Ok(None);
            };
            let polled = py
                .detach(|| follower.poll())
                .map_err(|e| PyOSError::new_err(format!("cannot follow {e}")))?;
            py.check_signals()?;
            let Some((_, change)) = polled else {
                continue;
            };
            if let trex::follow::Followed::Appended { bytes, .. } = &change {
                let mut text = std::mem::take(&mut self.carry);
                text.extend_from_slice(&self.decoder.decode(bytes));
                let whole = trex::encoding::complete_prefix(&text);
                self.carry = text.split_off(whole);
                self.push_text(py, &String::from_utf8_lossy(&text))?;
                continue;
            }
            if let Some(told) = change_told(&self.name, &change) {
                rules::warn(py, &told)?;
            }
            self.restart(py)?;
        }
    }
}

/// The stream scanner a followed edit runs: the pattern under its
/// declarations.
fn edit_scanner(pattern: &Pattern) -> trex::StreamScanner {
    trex::StreamScanner::with_shapes(pattern.inner.clone(), pattern.shapes.clone())
}

/// The follow `follow_rewrite` and `follow_redact` make of the file at
/// `path`: read from its window, then edited as it grows by `edit`, no more
/// than `limit` matches until it is truncated, replaced or removed.
// Each argument is one of the follow's own options, read once here.
#[allow(clippy::too_many_arguments)]
fn following_edit(
    py: Python<'_>,
    pattern: Py<Pattern>,
    edit: FollowEdit,
    path: std::path::PathBuf,
    reading: &rules::Reading,
    limit: Option<usize>,
    keep_count: bool,
) -> PyResult<FollowingEdit> {
    let name = path.display().to_string();
    let compiled = pattern.get();
    let scanner = edit_scanner(compiled);
    if !scanner.commits_early() {
        return Err(PyValueError::new_err(
            "no match of this pattern is final before its input ends, since it reads a content guard, a field anchor or a whole-input axis, and a followed file does not end; edit it without following it",
        ));
    }
    let part = py
        .detach(|| rules::Part::of_file(&path, reading))
        .map_err(|e| PyOSError::new_err(format!("{name}: {e}")))?;
    if part.binary {
        return Err(PyValueError::new_err(format!("{name} holds a NUL byte and is binary; a follow reads text")));
    }
    let mut following = FollowingEdit {
        pattern,
        edit,
        name,
        stream: trex::EditStream::new(scanner, part.byte_base, 0),
        decoder: trex::encoding::Incremental::after(part.encoding),
        carry: Vec::new(),
        chars_through: part.units_ahead,
        lines_through: part.line_base.unwrap_or(0),
        pending: String::new(),
        ready: std::collections::VecDeque::new(),
        follower: None,
        limit,
        keep_count,
    };
    following.push_text(py, &String::from_utf8_lossy(&part.text))?;
    let follower = trex::follow::Follower::new(&[(path, part.input_len)])
        .map_err(|e| PyOSError::new_err(format!("cannot follow {e}")))?;
    if let Some(why) = follower.unnotified() {
        rules::warn(py, &format!("no change notifications ({why}); looking at the file once a second"))?;
    }
    following.follower = Some(follower);
    Ok(following)
}

/// The file at `path` rewritten by `repl` as it grows, one str a line, each
/// without its ending, as `trex rewrite --follow` and Edit-TrexText -Follow
/// write it: its last `tail` lines, its open `lines` range or the whole of
/// it first, in records of `unit`, then each line it gains. `repl` is a
/// template, or a callable handed each `Match`, placed in the file, and
/// returning a str. `max_count` rewrites the first matches alone, and a file
/// truncated, replaced or removed starts that count again unless
/// `keep_count=True`.
#[pyfunction]
#[pyo3(signature = (pattern, repl, path, *, tail = None, lines = None, unit = "line", max_count = None, keep_count = false, lib = None))]
// Each keyword is one of the follow's own options, read once here.
#[allow(clippy::too_many_arguments)]
fn follow_rewrite(
    py: Python<'_>,
    pattern: &Bound<'_, PyAny>,
    repl: &Bound<'_, PyAny>,
    path: std::path::PathBuf,
    tail: Option<usize>,
    lines: Option<&str>,
    unit: &str,
    max_count: Option<usize>,
    keep_count: bool,
    lib: Option<&Bound<'_, PyAny>>,
) -> PyResult<FollowingEdit> {
    if keep_count && max_count.is_none() {
        return Err(PyValueError::new_err("keep_count= keeps the count max_count= caps, and no max_count was given"));
    }
    let pattern = followed_pattern(py, pattern, lib)?;
    let edit = if repl.is_instance_of::<PyString>() {
        let template: &Bound<'_, PyString> = repl.cast()?;
        FollowEdit::Template(template_of(&pattern.get().inner, template.to_str()?)?)
    } else if repl.is_callable() {
        FollowEdit::Callable(repl.clone().unbind())
    } else {
        return Err(not_a_replacement());
    };
    let reading = followed_window(tail, lines, unit)?;
    following_edit(py, pattern, edit, path, &reading, max_count, keep_count)
}

/// The file at `path` redacted as it grows, one str a line, each without its
/// ending, as `trex redact --follow` and Protect-TrexText -Follow write it:
/// its last `tail` lines, its open `lines` range or the whole of it first,
/// then each line it gains, each match masked as `Pattern.redact` masks one.
#[pyfunction]
#[pyo3(signature = (pattern, path, *, keep = None, mask = "*", tail = None, lines = None, unit = "line", lib = None))]
// Each keyword is one of the follow's own options, read once here.
#[allow(clippy::too_many_arguments)]
fn follow_redact(
    py: Python<'_>,
    pattern: &Bound<'_, PyAny>,
    path: std::path::PathBuf,
    keep: Option<&str>,
    mask: &str,
    tail: Option<usize>,
    lines: Option<&str>,
    unit: &str,
    lib: Option<&Bound<'_, PyAny>>,
) -> PyResult<FollowingEdit> {
    let pattern = followed_pattern(py, pattern, lib)?;
    let keeps = match keep {
        Some(list) => trex::Keep::parse_list(list, &pattern.get().inner.capture_names())
            .map_err(|e| PyValueError::new_err(format!("keep= error at byte {}: {}", e.pos, e.msg)))?,
        None => Vec::new(),
    };
    let mask = trex::Mask::parse(mask).map_err(|e| PyValueError::new_err(format!("mask= {e}")))?;
    let reading = followed_window(tail, lines, unit)?;
    following_edit(py, pattern, FollowEdit::Mask { keeps, mask }, path, &reading, None, false)
}

/// One record a query keeps: the file it is in, the line it starts on,
/// its position in the input's units and in bytes, its text in the
/// input's type, and the patterns it holds, as they were given.
#[pyclass(frozen, module = "trex")]
struct Record {
    /// The file the record is in; `None` for text.
    #[pyo3(get)]
    path: Option<String>,
    /// The line it starts on, from 1.
    #[pyo3(get)]
    line: usize,
    /// Where it starts, in the input's units.
    #[pyo3(get)]
    start: usize,
    /// Where it ends, exclusive, in the input's units.
    #[pyo3(get)]
    end: usize,
    /// Where it starts, as a UTF-8 byte offset of the input's text.
    #[pyo3(get)]
    byte_start: usize,
    /// Where it ends, exclusive, as a UTF-8 byte offset.
    #[pyo3(get)]
    byte_end: usize,
    /// Its text, in the input's type.
    #[pyo3(get)]
    text: Py<PyAny>,
    /// The patterns it holds, each as it was given.
    #[pyo3(get)]
    patterns: Vec<String>,
}

#[pymethods]
impl Record {
    /// The record as a plain dict of its fields.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("path", &self.path)?;
        d.set_item("line", self.line)?;
        d.set_item("start", self.start)?;
        d.set_item("end", self.end)?;
        d.set_item("byte_start", self.byte_start)?;
        d.set_item("byte_end", self.byte_end)?;
        d.set_item("text", self.text.bind(py))?;
        d.set_item("patterns", self.patterns.clone())?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let path = match &self.path {
            Some(p) => PyString::new(py, p).repr()?.to_string(),
            None => "None".to_string(),
        };
        Ok(format!("Record(path={path}, line={}, text={})", self.line, self.text.bind(py).repr()?))
    }
}

/// The patterns `given` names, a `Pattern` or its source or a sequence of
/// them, each compiled under `shapes`, with the text each was given as.
fn query_patterns(given: &Bound<'_, PyAny>, shapes: &trex::ShapeSet) -> PyResult<Vec<(String, trex::ast::Pattern)>> {
    let one = |p: &Bound<'_, PyAny>| -> PyResult<(String, trex::ast::Pattern)> {
        if p.is_instance_of::<Pattern>() {
            let p = p.cast::<Pattern>()?.get();
            return Ok((p.source.clone(), p.inner.clone()));
        }
        let source: String =
            p.extract().map_err(|e| PyTypeError::new_err(format!("a pattern is a trex.Pattern or its source: {e}")))?;
        let parsed = if shapes.is_empty() && shapes.lets().is_empty() {
            trex::parse(&source)
        } else {
            trex::parser::parse_with_shapes(&source, shapes)
        };
        Ok((source, parsed.map_err(parse_error)?))
    };
    if given.is_instance_of::<Pattern>() || given.is_instance_of::<PyString>() {
        return Ok(vec![one(given)?]);
    }
    given.try_iter()?.map(|p| one(&p?)).collect()
}

/// The records of `input`, a str or bytes, or of the files `path=` names,
/// that hold `patterns` as `require` asks, and none of `exclude`, as `scan
/// --all`, `--any`, `--none` and `--at-least` keep them.
///
/// `patterns` is a `Pattern` or its source, or a sequence of them, read under
/// `lib`'s declarations. `require` is `"any"`, `"all"` or `"none"`, and
/// `at_least=N` keeps the records holding N of them in its place. A record is
/// a line unless `unit` names another record unit, `record_start` makes it
/// the run from one match of a pattern to the next, or `record_span` each
/// match of one. `max_count` keeps the first records of each input. A
/// directory is walked as `trex.files` walks one, and a file holding a NUL
/// byte is passed over, one named alone raising `ValueError`.
#[pyfunction]
#[pyo3(
    signature = (patterns, input = None, *, path = None, require = "any", at_least = None, exclude = None, unit = "line", record_start = None, record_span = None, max_count = None, lib = None),
    text_signature = "(patterns, input=None, *, path=None, require='any', at_least=None, exclude=(), unit='line', record_start=None, record_span=None, max_count=None, lib=None)"
)]
// Each keyword is one of the query's own options, read once here.
#[allow(clippy::too_many_arguments)]
fn query(
    py: Python<'_>,
    patterns: &Bound<'_, PyAny>,
    input: Option<&Bound<'_, PyAny>>,
    path: Option<PathArg>,
    require: &str,
    at_least: Option<usize>,
    exclude: Option<&Bound<'_, PyAny>>,
    unit: &str,
    record_start: Option<&Bound<'_, PyAny>>,
    record_span: Option<&Bound<'_, PyAny>>,
    max_count: Option<usize>,
    lib: Option<&Bound<'_, PyAny>>,
) -> PyResult<Vec<Record>> {
    use trex::records::{Quantifier, RecordUnit};
    let quantifier = match (require, at_least) {
        ("all", None) => Quantifier::All,
        ("any", None) => Quantifier::Any,
        ("none", None) => Quantifier::None,
        ("any", Some(n)) => Quantifier::AtLeast(n),
        (_, Some(_)) => {
            return Err(PyValueError::new_err(format!(
                "require={require:?} and at_least= are two rules; a query holds one"
            )));
        }
        (other, None) => {
            return Err(PyValueError::new_err(format!("require is 'all', 'any' or 'none', not {other:?}")));
        }
    };
    let shapes = shapes_of(lib)?;
    let given = query_patterns(patterns, &shapes)?;
    if given.is_empty() {
        return Err(PyValueError::new_err("a query holds its records to one pattern at least"));
    }
    let negatives = match exclude {
        Some(e) => query_patterns(e, &shapes)?.into_iter().map(|(_, p)| p).collect(),
        None => Vec::new(),
    };
    let unit = match (unit, record_start, record_span) {
        (name, None, None) => RecordUnit::parse(name).map_err(|e| PyValueError::new_err(format!("unit= {e}")))?,
        ("line", Some(p), None) => RecordUnit::Start(one_pattern(p, &shapes)?),
        ("line", None, Some(p)) => RecordUnit::Span(one_pattern(p, &shapes)?),
        _ => {
            return Err(PyValueError::new_err(
                "unit=, record_start= and record_span= each say what a record is; give one",
            ));
        }
    };
    let names: Vec<String> = given.iter().map(|(name, _)| name.clone()).collect();
    let q = trex::records::Query {
        quantifier,
        positives: given.into_iter().map(|(_, p)| p).collect(),
        set: None,
        negatives,
        unit,
    };
    let records_of = |py: Python<'_>, shown: &Input<'_>, path: Option<&str>, out: &mut Vec<Record>| -> PyResult<()> {
        let bytes = shown.bytes();
        let mut hits = py.detach(|| q.hits(bytes, &shapes, false));
        if let Some(n) = max_count {
            hits.truncate(n);
        }
        let index = trex::files::LineIndex::new(bytes);
        let mut units = units_of(shown);
        for hit in hits {
            out.push(Record {
                path: path.map(str::to_string),
                line: index.line_of(hit.start) + 1,
                start: units.at(hit.start),
                end: units.at(hit.end),
                byte_start: hit.start,
                byte_end: hit.end,
                text: shown.slice(py, hit.start, hit.end),
                patterns: hit.present.iter().map(|&i| names[i].clone()).collect(),
            });
        }
        Ok(())
    };
    let mut out = Vec::new();
    match (input, path) {
        (Some(obj), None) => records_of(py, &Input::of(obj)?, None, &mut out)?,
        (None, Some(paths)) => {
            let reading = rules::Reading::of(None, None, None, "line", false, false, None, false)?;
            let walked = rules::walked(py, paths, &reading.walk)?;
            let mut errors = walked.errors;
            for (name, file) in &walked.files {
                match py.detach(|| rules::Part::of_file(file, &reading)) {
                    Err(e) => errors.push(format!("{name}: {e}")),
                    Ok(part) if part.binary => {
                        if walked.lone {
                            return Err(PyValueError::new_err(format!(
                                "{name} holds a NUL byte and is binary; a query reads text"
                            )));
                        }
                    }
                    Ok(part) => {
                        let text = String::from_utf8_lossy(&part.text);
                        records_of(py, &Input::Text(&text), Some(name), &mut out)?;
                    }
                }
            }
            if !errors.is_empty() {
                return Err(PyOSError::new_err(errors.join("; ")));
            }
        }
        (Some(_), Some(_)) | (None, None) => {
            return Err(PyTypeError::new_err(
                "give the text, a str or bytes, or path=, a file or directory's path or a list of them; one of them",
            ));
        }
    }
    Ok(out)
}

/// The one pattern `given` names, a `Pattern` or its source, compiled under
/// `shapes`.
fn one_pattern(given: &Bound<'_, PyAny>, shapes: &trex::ShapeSet) -> PyResult<trex::ast::Pattern> {
    let mut all = query_patterns(given, shapes)?;
    match (all.pop(), all.is_empty()) {
        (Some((_, p)), true) => Ok(p),
        _ => Err(PyValueError::new_err("record_start= and record_span= take one pattern")),
    }
}

/// A compiled pattern.
#[pyclass(frozen, module = "trex")]
struct Pattern {
    inner: trex::ast::Pattern,
    source: String,
    /// What `lib=` declared, empty for a pattern read without one.
    shapes: trex::ShapeSet,
}

impl Pattern {
    /// The capture names in written order, shared by the matches of one
    /// call.
    fn bound(&self) -> Arc<[String]> {
        Arc::from(self.inner.capture_names())
    }

    /// The kind each register binds, shared by the matches of one call, for
    /// `Match.value`. Read from the pattern rather than from a match,
    /// because it is a property of the pattern.
    fn kinds(&self) -> CaptureKinds {
        Arc::from(self.inner.capture_kinds())
    }

    /// The pattern and its declarations, shared by the matches of one call,
    /// for `explain`. Cloned once a call rather than once a match.
    fn shared(&self) -> Arc<Compiled> {
        Arc::new(Compiled { pattern: self.inner.clone(), shapes: self.shapes.clone(), member: None })
    }

    /// Whether this pattern is read under declarations that change the lex,
    /// so every scan of it must take the shaped path rather than the routed
    /// one: a shape or a kind decides where tokens begin and end, and a
    /// route that never lexes cannot see them.
    fn shaped(&self) -> bool {
        !self.shapes.is_empty()
    }

    /// The fields `fields` and `records` read: where the pattern is one
    /// reference `\{name}` to a sub-pattern `lib=` declared and a `fields`
    /// line gives that one its fields, the fields the builder read; else one
    /// field per register, in written order, a list where it is bound under a
    /// repetition.
    fn read_fields(&self) -> Vec<trex::infer::build::Field> {
        trex::infer::build::fields_for(&self.source, &self.inner, &self.shapes)
    }

    /// Every leftmost, non-overlapping match, as spans.
    fn spans(&self, bytes: &[u8]) -> Vec<trex::Span> {
        if self.shaped() {
            trex::scan_with_shapes(&self.inner, bytes, &self.shapes)
        } else {
            trex::scan_with_backend(&self.inner, bytes, trex::Backend::Auto).0
        }
    }

    /// The pieces between the matches, at most `limit` of them, the last
    /// holding whatever the cut stopped short of.
    ///
    /// A shaped pattern cuts on the spans its own shaped scan found, since
    /// the library's split lexes without the declarations; the pieces are
    /// the same ones the library would give for a pattern that has none.
    fn split_upto(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
        limit: usize,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let input = Input::of(input)?;
        let bytes = input.bytes();
        let pat = &self.inner;
        let pieces: Vec<(usize, usize)> = py.detach(|| {
            if !self.shaped() {
                return if limit == usize::MAX {
                    trex::split(pat, bytes).map(|p| offsets_of(bytes, p)).collect()
                } else {
                    trex::splitn(pat, bytes, limit).map(|p| offsets_of(bytes, p)).collect()
                };
            }
            let mut out = Vec::new();
            // A limit of zero asks for no pieces at all, which is what
            // `splitn` yields; the trailing remainder below would make one.
            if limit == 0 {
                return out;
            }
            let mut at = 0usize;
            for span in self.spans(bytes) {
                if out.len() + 1 >= limit {
                    break;
                }
                out.push((at, span.start()));
                at = span.end();
            }
            out.push((at, bytes.len()));
            out
        });
        Ok(pieces.into_iter().map(|(s, e)| input.slice(py, s, e)).collect())
    }

    /// The first match, or none. A shaped pattern scans and takes the
    /// first, having no cursor that stops early under the declarations.
    fn first(&self, bytes: &[u8]) -> Option<trex::Span> {
        if self.shaped() {
            self.spans(bytes).into_iter().next()
        } else {
            trex::find(&self.inner, bytes)
        }
    }

    /// `spans` with their registers resolved, every binding a register made
    /// under a repetition kept.
    fn resolved(&self, bytes: &[u8], spans: &[trex::Span]) -> Vec<trex::Match> {
        if self.shaped() {
            trex::captures_with_shapes_and_lists(&self.inner, bytes, &self.shapes, spans)
        } else {
            trex::captures_with_lists(&self.inner, bytes, spans)
        }
    }

    /// Every match of `scan` with its registers resolved.
    fn all(&self, py: Python<'_>, input: &Input<'_>) -> Vec<trex::Match> {
        let bytes = input.bytes();
        py.detach(|| {
            let spans = self.spans(bytes);
            self.resolved(bytes, &spans)
        })
    }

    /// Every match within `range` of the input, the bytes there scanned
    /// alone and each match placed at the input's own offsets.
    fn within(&self, py: Python<'_>, input: &Input<'_>, range: std::ops::Range<usize>) -> Vec<trex::Match> {
        let whole = input.bytes();
        if range == (0..whole.len()) {
            return self.all(py, input);
        }
        let bytes = &whole[range.clone()];
        py.detach(|| {
            let spans = self.spans(bytes);
            self.resolved(bytes, &spans).into_iter().map(|m| m.shifted(range.start)).collect()
        })
    }

    /// [`Self::within`], the scan run as `engine` says, beside what ran. A
    /// pattern with declared shapes or kinds lexes with them whatever the
    /// engine, since none of its ways carries them, and runs nothing else.
    fn within_by(
        &self,
        py: Python<'_>,
        input: &Input<'_>,
        range: std::ops::Range<usize>,
        engine: &trex::Engine,
    ) -> (Vec<trex::Match>, Option<trex::Ran>) {
        if engine.is_plain() || self.shaped() {
            return (self.within(py, input, range), None);
        }
        let bytes = &input.bytes()[range.clone()];
        let pat = &self.inner;
        py.detach(|| {
            let (spans, ran) = trex::scan_engine(pat, bytes, engine);
            let found = self.resolved(bytes, &spans).into_iter().map(|m| m.shifted(range.start)).collect();
            (found, Some(ran))
        })
    }

    /// The first `take` matches with their registers resolved, or every
    /// match, found as a rewrite finds them.
    fn taken(&self, py: Python<'_>, input: &Input<'_>, take: Option<usize>) -> Vec<trex::Match> {
        let Some(n) = take else {
            return self.all(py, input);
        };
        let bytes = input.bytes();
        let pat = &self.inner;
        py.detach(|| {
            // A shaped pattern has no early-stopping cursor of its own, so
            // it scans and then keeps the first `n`; the matches are the
            // same either way.
            let spans: Vec<trex::Span> = if self.shaped() {
                self.spans(bytes).into_iter().take(n).collect()
            } else if n == 1 {
                trex::find(pat, bytes).into_iter().collect()
            } else {
                trex::find_iter(pat, bytes).take(n).collect()
            };
            self.resolved(bytes, &spans)
        })
    }

    /// `input` with its matches, the first `take` or every one, replaced by
    /// `repl`: a template rendered at each, or a callable handed each match
    /// and returning its replacement in the input's type. Where `range` is
    /// part of the input, a window of it, that part alone is rewritten and
    /// given back, and a callable reads each match at the input's own
    /// offsets. The matches are found as `engine` says.
    fn rewritten(
        &self,
        py: Python<'_>,
        repl: &Bound<'_, PyAny>,
        input: &Input<'_>,
        range: std::ops::Range<usize>,
        take: Option<usize>,
        engine: &trex::Engine,
    ) -> PyResult<Py<PyAny>> {
        let whole = input.bytes();
        let bytes = &whole[range.clone()];
        let pat = &self.inner;
        let by_engine = !engine.is_plain() && !self.shaped();
        if repl.is_instance_of::<PyString>() {
            let template: &Bound<'_, PyString> = repl.cast()?;
            let t = template_of(pat, template.to_str()?)?;
            if by_engine {
                // The template renders on the host over the spans the engine
                // found, which are the spans every way finds.
                let out = py.detach(|| {
                    let (spans, _ran) = trex::scan_engine(pat, bytes, engine);
                    let mut edits = trex::rewrite::edits_at(pat, &t, bytes, &self.shapes, &spans);
                    if let Some(n) = take {
                        edits.truncate(n);
                    }
                    splice(bytes, &edits)
                });
                return input.wrap(py, &out);
            }
            let out = py.detach(|| match (self.shaped(), take) {
                // A shaped rewrite splices the edits its own shaped scan
                // found, since the library's rewrite lexes without the
                // declarations.
                (true, take) => {
                    let mut edits = trex::rewrite::edits_with_shapes(pat, &t, bytes, &self.shapes);
                    if let Some(n) = take {
                        edits.truncate(n);
                    }
                    splice(bytes, &edits)
                }
                (false, Some(n)) => trex::rewrite_n(pat, &t, bytes, n),
                (false, None) => trex::rewrite(pat, &t, bytes),
            });
            return input.wrap(py, &out);
        }
        if !repl.is_callable() {
            return Err(not_a_replacement());
        }
        // The whole input stops at its `take`th match; a window is scanned
        // whole first, since its matches are found over its bytes alone, and
        // so is an input scanned another way than the routed one.
        let found = if by_engine {
            let (mut found, _ran) = self.within_by(py, input, range.clone(), engine);
            if let Some(n) = take {
                found.truncate(n);
            }
            found
        } else if range == (0..whole.len()) {
            self.taken(py, input, take)
        } else {
            let mut found = self.within(py, input, range.clone());
            if let Some(n) = take {
                found.truncate(n);
            }
            found
        };
        let edits = self.called(py, repl, input, &found)?;
        let mut out = Vec::with_capacity(bytes.len());
        let mut at = range.start;
        for e in &edits {
            out.extend_from_slice(&whole[at..e.start]);
            out.extend_from_slice(&e.replacement);
            at = e.end;
        }
        out.extend_from_slice(&whole[at..range.end]);
        input.wrap(py, &out)
    }

    /// The edits replacing each of `found` with what `repl`, a callable,
    /// returns for it: each match handed over, its replacement taken back in
    /// the input's type.
    fn called(
        &self,
        py: Python<'_>,
        repl: &Bound<'_, PyAny>,
        input: &Input<'_>,
        found: &[trex::Match],
    ) -> PyResult<Vec<trex::files::Edit>> {
        let bound = self.bound();
        let kinds = self.kinds();
        let shared = self.shared();
        let mut placing = Placing::of(input);
        let mut edits = Vec::with_capacity(found.len());
        for m in found {
            let piece = repl.call1((Match::of(py, input, &mut placing, m, &bound, &kinds, &shared)?,))?;
            let replacement = match input {
                Input::Text(_) if piece.is_instance_of::<PyString>() => {
                    let s: &Bound<'_, PyString> = piece.cast()?;
                    s.to_str()?.as_bytes().to_vec()
                }
                Input::Bytes(_) if piece.is_instance_of::<PyBytes>() => {
                    let b: &Bound<'_, PyBytes> = piece.cast()?;
                    b.as_bytes().to_vec()
                }
                Input::Text(_) => {
                    return Err(PyTypeError::new_err("the replacement for a str input must be a str"));
                }
                Input::Bytes(_) => {
                    return Err(PyTypeError::new_err("the replacement for a bytes input must be bytes"));
                }
            };
            edits.push(trex::files::Edit { start: m.start, end: m.end, replacement });
        }
        Ok(edits)
    }

    /// The edits `repl` makes of the part of `text`, a file's text, that
    /// `reading` names: a template rendered at each match, as every surface
    /// rewrites a file, or a callable handed each match at the file's own
    /// offsets; the first `max_count` where a count is given, found as
    /// `engine` says.
    fn rewrite_edits(
        &self,
        py: Python<'_>,
        repl: &Bound<'_, PyAny>,
        text: &str,
        reading: &rules::Reading,
        max_count: Option<usize>,
        engine: &trex::Engine,
    ) -> PyResult<Vec<trex::files::Edit>> {
        let bytes = text.as_bytes();
        let range = window_of(bytes, reading);
        if repl.is_instance_of::<PyString>() {
            let template: &Bound<'_, PyString> = repl.cast()?;
            let t = template_of(&self.inner, template.to_str()?)?;
            let piece = &bytes[range.clone()];
            let (mut edits, _ran) =
                py.detach(|| trex::rewrite::edits_by(&self.inner, &t, piece, &self.shapes, engine, max_count));
            for e in &mut edits {
                e.start += range.start;
                e.end += range.start;
            }
            return Ok(edits);
        }
        if !repl.is_callable() {
            return Err(not_a_replacement());
        }
        let input = Input::Text(text);
        let (mut found, _ran) = self.within_by(py, &input, range, engine);
        if let Some(n) = max_count {
            found.truncate(n);
        }
        self.called(py, repl, &input, &found)
    }

    /// Each file `path` names, walked as `reading` walks, with the edits
    /// redacting the matches in the part `reading` names, `keep` and `mask`
    /// read as `redact` reads them. One mask runs through the files in walk
    /// order, so a pseudonym names each value alike in every one.
    fn redactions(
        &self,
        py: Python<'_>,
        path: PathArg,
        keep: Option<&str>,
        mask: &str,
        reading: &rules::Reading,
        errors: &mut Vec<String>,
    ) -> PyResult<Vec<(EditedFile, Vec<trex::files::Edit>)>> {
        let keeps = match keep {
            Some(list) => trex::Keep::parse_list(list, &self.inner.capture_names())
                .map_err(|e| PyValueError::new_err(format!("keep= error at byte {}: {}", e.pos, e.msg)))?,
            None => Vec::new(),
        };
        let mut mask = trex::Mask::parse(mask).map_err(|e| PyValueError::new_err(format!("mask= {e}")))?;
        let files = files_edited(py, path, reading, "redacts", errors)?;
        let mut out = Vec::with_capacity(files.len());
        for f in files {
            let bytes = f.text.as_bytes();
            let range = window_of(bytes, reading);
            let piece = &bytes[range.clone()];
            let mut edits = py.detach(|| {
                let spans = self.spans(piece);
                let matches = self.resolved(piece, &spans);
                trex::redactions_with_shapes(piece, &matches, &keeps, &mut mask, &self.inner, &self.shapes)
            });
            for e in &mut edits {
                e.start += range.start;
                e.end += range.start;
            }
            out.push((f, edits));
        }
        Ok(out)
    }

    /// The explanation of a change's match at `start..end` of `text`, as
    /// `Match.explain` gives one, `what` naming the scan that found it.
    fn change_explained(&self, py: Python<'_>, text: &[u8], start: usize, end: usize, what: &str) -> PyResult<Py<PyDict>> {
        let explainer = trex::explain::Explainer::new(&self.inner, text, &self.shapes);
        explanation_dict(py, &explainer.explain(&trex::Match::plain(start, end), what))
    }
}

#[pymethods]
impl Pattern {
    /// Compile `source`; a pattern error is a `ValueError` naming the byte.
    ///
    /// `lib` is a `Library`, or names a pattern file, a directory read as
    /// its `.trex` files, or a list of them, whose `let`, `kind`, `shape`
    /// and `shape-after` lines the pattern is read under, as `--lib`
    /// supplies them on the command line: `\{name}` resolves to
    /// what they declare, and a declared shape or kind decides the token
    /// boundaries every scan of this pattern is made on.
    #[new]
    #[pyo3(signature = (source, lib = None))]
    fn new(source: &str, lib: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let shapes = shapes_of(lib)?;
        // With nothing declared at all this is the plain parse, which also
        // reads the `(?empty:...)` prefix; with anything declared it is the
        // parse the command line makes under `--lib`, so a pattern reads
        // the same way in both. A file of `let` lines alone declares
        // nothing the lexer sees, which is what `is_empty` reports, but it
        // declares names the parser must resolve, so both are asked here.
        let declares = !shapes.is_empty() || !shapes.lets().is_empty();
        let inner = if declares {
            trex::parser::parse_with_shapes(source, &shapes).map_err(parse_error)?
        } else {
            trex::parse(source).map_err(parse_error)?
        };
        Ok(Pattern { inner, source: source.to_string(), shapes })
    }

    /// The fields `records` reads, each as a dict as `Built.fields` gives
    /// it. For `\{name}` read under a file whose `fields` line gives `name`
    /// its fields, as a pattern file `infer` writes does, they are the
    /// fields the build read; for any other pattern, one per register.
    #[getter]
    fn fields<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        field_dicts(py, &self.read_fields())
    }

    /// The records of `input`, each as a dict as `Built.records` gives it:
    /// the `lines` its matches are on, counted from zero, and `values`,
    /// each field of `fields` by its key, read through its accessor, `None`
    /// where absent. A match holding a field written `{name*}` begins a
    /// record and the matches after it join it; with no such field, each
    /// match is one, and a line repeating its record gives one per repeat.
    fn records<'py>(&self, py: Python<'py>, input: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let input = Input::of(input)?;
        let text = String::from_utf8_lossy(input.bytes()).into_owned();
        let fields = self.read_fields();
        let records = py
            .detach(|| trex::infer::build::read_records(&fields, &self.inner, &self.shapes, &text))
            .map_err(PyValueError::new_err)?;
        records
            .iter()
            .map(|r| {
                let d = PyDict::new(py);
                d.set_item("lines", &r.lines)?;
                d.set_item("values", values_dict(py, &fields, &r.values, None)?)?;
                Ok(d)
            })
            .collect()
    }

    /// The matches of `input` grouped by `key`, as `(key, count)` rows.
    ///
    /// `key` is a report template as `--format` takes one, rendered once per
    /// match, so a capture (`${host}`), a typed slice (`${ip:octet1-2}`) or a
    /// composite is what the rows count. `order` is `"key"` to sort by key as
    /// `count-by` does, or `"count"` for most frequent first with the key
    /// breaking ties as `top` does.
    ///
    /// Every key is returned. A caller wanting the head of the table takes
    /// it, because a truncated table and a complete one read alike.
    ///
    /// `head`, `tail` or `lines` counts the matches of that part of the
    /// input alone, as the scan does.
    #[pyo3(signature = (key, input, order = "key", *, head = None, tail = None, lines = None, unit = "line"))]
    // The eight are the call's own arguments, the four window keywords among
    // them, each read once here.
    #[allow(clippy::too_many_arguments)]
    fn count_by(
        &self,
        py: Python<'_>,
        key: &str,
        input: &Bound<'_, PyAny>,
        order: &str,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Vec<(String, u64)>> {
        let ask = TableAsk::of(order, None, false, head, tail, lines, unit)?;
        let table = table_of(py, &Grouping::One(self), key, Vec::new(), Some(input), None, &ask)?;
        Ok(table.rows(ask.order).iter().map(|row| (row.key.to_string(), row.count)).collect())
    }

    /// The matches of `text`, a str or bytes, or of the files `path=` names,
    /// grouped by `key`, as a list of `Group`s.
    ///
    /// `key` is a report template, as `count_by` takes one: `${host}`,
    /// `${ip:octet1-3}`, or `${path}` for the file a match is in. `path=` is
    /// a file, a directory walked as `trex count-by` walks one, or a list of
    /// them, each a str or an `os.PathLike`; `trex.files(...)` gives a walk
    /// under other rules. A file holding a NUL byte is passed over unless
    /// `binary=True` asks for it, and one named alone raises `ValueError`.
    ///
    /// `sum`, `avg`, `min` and `max` each take a register or a list of them,
    /// and `percentiles` maps a register to a percent or a list of them, as
    /// `{"t": (50, 95, 99)}`. Each group holds them by register: a sum and
    /// an interpolated percentile as an int or a Decimal, an average as an
    /// exact `fractions.Fraction`, and a least or greatest value or a
    /// percentile that is one of the values as `Match.value` gives a value,
    /// durations in `duration_unit`; None where the group's matches bound no
    /// value.
    /// `percentile_method` decides a percentile between two values:
    /// "nearest", "linear", "lower" or "hybrid". An aggregate the register's
    /// kind cannot carry raises `ValueError` before anything is read.
    ///
    /// `order` is "key", or "count" for most first with the key breaking
    /// ties, and `limit` keeps the first groups. `head`, `tail` or `lines`
    /// groups the matches of that part of each input, counted in records of
    /// `unit`.
    #[pyo3(signature = (key, text = None, *, path = None, order = "key", limit = None, sum = None, avg = None, min = None, max = None, percentiles = None, percentile_method = "nearest", duration_unit = "ns", binary = false, head = None, tail = None, lines = None, unit = "line"))]
    // Each keyword is one of the table's own options, read once here.
    #[allow(clippy::too_many_arguments)]
    fn group_by(
        &self,
        py: Python<'_>,
        key: &str,
        text: Option<&Bound<'_, PyAny>>,
        path: Option<PathArg>,
        order: &str,
        limit: Option<usize>,
        sum: Option<Registers>,
        avg: Option<Registers>,
        min: Option<Registers>,
        max: Option<Registers>,
        percentiles: Option<&Bound<'_, PyDict>>,
        percentile_method: &str,
        duration_unit: &str,
        binary: bool,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Vec<Group>> {
        use trex::typed::Agg;
        let mut ask = TableAsk::of(order, limit, binary, head, tail, lines, unit)?;
        ask.how = percentile_method_of(percentile_method)?;
        ask.duration = duration_unit_of("duration_unit", duration_unit)?;
        let asked = asked_columns([(Agg::Sum, sum), (Agg::Avg, avg), (Agg::Min, min), (Agg::Max, max)], percentiles)?;
        grouped(py, &Grouping::One(self), key, text, path, &ask, &asked)
    }

    /// The distinct keys the matches of `text`, or of the files `path=`
    /// names, render under `key`, as `trex uniq` prints them: in key order,
    /// or most frequent first where `order` is "count", as far as `limit`.
    /// The input keywords are `group_by`'s.
    #[pyo3(signature = (key, text = None, *, path = None, order = "key", limit = None, binary = false, head = None, tail = None, lines = None, unit = "line"))]
    // Each keyword is one of the table's own options, read once here.
    #[allow(clippy::too_many_arguments)]
    fn distinct(
        &self,
        py: Python<'_>,
        key: &str,
        text: Option<&Bound<'_, PyAny>>,
        path: Option<PathArg>,
        order: &str,
        limit: Option<usize>,
        binary: bool,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Vec<String>> {
        let ask = TableAsk::of(order, limit, binary, head, tail, lines, unit)?;
        distinct_keys(py, &Grouping::One(self), key, text, path, &ask)
    }

    /// The pattern's source text.
    #[getter]
    fn source(&self) -> &str {
        &self.source
    }

    /// The register names the pattern binds.
    fn capture_names(&self) -> Vec<String> {
        self.inner.capture_names()
    }

    /// Whether the pattern matches anywhere in `input`, or in the part
    /// `head`, `tail` or `lines` names.
    #[pyo3(signature = (input, *, head = None, tail = None, lines = None, unit = "line"))]
    fn is_match(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<bool> {
        let input = Input::of(input)?;
        let range = window_range(input.bytes(), head, tail, lines, unit)?;
        let bytes = &input.bytes()[range];
        let pat = &self.inner;
        Ok(py.detach(|| {
            if self.shaped() { !self.spans(bytes).is_empty() } else { trex::is_match(pat, bytes) }
        }))
    }

    /// The first match, or None; within the part `head`, `tail` or `lines`
    /// names where one does, at the input's own offsets.
    #[pyo3(signature = (input, *, head = None, tail = None, lines = None, unit = "line"))]
    fn find(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Option<Match>> {
        let input = Input::of(input)?;
        let range = window_range(input.bytes(), head, tail, lines, unit)?;
        let bytes = &input.bytes()[range.clone()];
        let found = py.detach(|| match self.first(bytes) {
            Some(span) => self.resolved(bytes, &[span]).into_iter().map(|m| m.shifted(range.start)).collect(),
            None => Vec::new(),
        });
        let bound = self.bound();
        let kinds = self.kinds();
        let shared = self.shared();
        let mut placing = Placing::of(&input);
        found.first().map(|m| Match::of(py, &input, &mut placing, m, &bound, &kinds, &shared)).transpose()
    }

    /// Every leftmost, non-overlapping match, as a list.
    ///
    /// `head=N` scans the first N lines alone, `tail=N` the last N, and
    /// `lines="A..B"` lines A through B, counted from one; `unit` counts
    /// paragraphs or another record unit instead of lines. A match counts
    /// only if it is wholly inside that part, and every match is
    /// reported at the input's own offsets.
    ///
    /// `backend` places the scan: "auto" routes it, "gpu" runs it on the
    /// device where the device can take it, "cpu" never touches the device.
    /// `dual_grain=True` runs the lex and the match as a pipeline on two
    /// threads, and `chunk_size=N` feeds the input in chunks of N bytes; one
    /// of the three ways at a time. Every way finds the same matches.
    #[pyo3(signature = (input, *, head = None, tail = None, lines = None, unit = "line", backend = "auto", dual_grain = false, chunk_size = None))]
    // The eight are the call's own arguments, the window and engine
    // keywords among them, each read once here.
    #[allow(clippy::too_many_arguments)]
    fn scan(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        backend: &str,
        dual_grain: bool,
        chunk_size: Option<usize>,
    ) -> PyResult<Vec<Match>> {
        let engine = engine_of(backend, dual_grain, chunk_size)?;
        let input = Input::of(input)?;
        let range = window_range(input.bytes(), head, tail, lines, unit)?;
        let count = StatsCount::start();
        let scanned = range.len();
        let (found, _ran) = self.within_by(py, &input, range, &engine);
        count.finish(&[(input.bytes(), scanned, &found)]);
        let bound = self.bound();
        let kinds = self.kinds();
        let shared = self.shared();
        let mut placing = Placing::of(&input);
        found.iter().map(|m| Match::of(py, &input, &mut placing, m, &bound, &kinds, &shared)).collect()
    }

    /// The matches of `scan`, one at a time.
    #[pyo3(signature = (input, *, head = None, tail = None, lines = None, unit = "line", backend = "auto", dual_grain = false, chunk_size = None))]
    // The eight are the call's own arguments, the window and engine
    // keywords among them, each read once here.
    #[allow(clippy::too_many_arguments)]
    fn find_iter(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        backend: &str,
        dual_grain: bool,
        chunk_size: Option<usize>,
    ) -> PyResult<Matches> {
        let found = self.scan(py, input, head, tail, lines, unit, backend, dual_grain, chunk_size)?;
        Ok(Matches { items: found.into_iter() })
    }

    /// The lines of `input`, a str or bytes, or of the files `path=` names,
    /// as grep reads them, one `Line` at a time: each line a match starts on
    /// with its matches, or with `invert=True` each line no match touches;
    /// and the lines `before=`, `after=` or `context=` carry around each as
    /// context, a count or a record unit's name (`"block"`, `"paragraph"`,
    /// `"unit:ROLE"`) for the record the line is in, `context=` giving
    /// both sides where `before=` and `after=` give none.
    ///
    /// `whole_line=True` keeps a match only where it covers its line, and
    /// `max_count=N` takes the first N matches of each input, or N lines
    /// under `invert=True`. `head`, `tail` or `lines` reads that part of each
    /// input, numbered as the input numbers it, and `backend` places the scan
    /// as `scan` takes it.
    ///
    /// `path=` is a file, a directory walked as `trex.files` walks one, or a
    /// list of them, `hidden`, `no_ignore` and `globs` saying how; a file
    /// holding a NUL byte is passed over unless `binary=True`, and one named
    /// alone raises `ValueError`. The files are read one at a time as their
    /// lines are taken, and `trex.scan_stats()` reads the grep once its last
    /// line is.
    #[pyo3(
        signature = (input = None, *, path = None, before = Side::Count(0), after = Side::Count(0), context = None, invert = false, whole_line = false, max_count = None, head = None, tail = None, lines = None, unit = "line", hidden = false, no_ignore = false, binary = false, globs = None, backend = "auto"),
        text_signature = "($self, input=None, *, path=None, before=0, after=0, context=None, invert=False, whole_line=False, max_count=None, head=None, tail=None, lines=None, unit='line', hidden=False, no_ignore=False, binary=False, globs=None, backend='auto')"
    )]
    // Each keyword is one of grep's own options, read once here.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    fn grep(
        slf: &Bound<'_, Self>,
        py: Python<'_>,
        input: Option<&Bound<'_, PyAny>>,
        path: Option<PathArg>,
        before: Side,
        after: Side,
        context: Option<Side>,
        invert: bool,
        whole_line: bool,
        max_count: Option<usize>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        hidden: bool,
        no_ignore: bool,
        binary: bool,
        globs: Option<Vec<String>>,
        backend: &str,
    ) -> PyResult<Lines> {
        let engine = engine_of(backend, false, None)?;
        let reading = rules::Reading::of(head, tail, lines, unit, hidden, no_ignore, globs, binary)?;
        let ask = GrepAsk { context: GrepContext::of(before, after, context)?, invert, whole_line, max_count, engine };
        let started = std::time::Instant::now();
        let mut stats = ScanStats::default();
        let mut ready = std::collections::VecDeque::new();
        let mut files = std::collections::VecDeque::new();
        let mut lone = false;
        match (input, path) {
            (Some(obj), None) => {
                if reading.walks() {
                    return Err(PyValueError::new_err(
                        "hidden=, no_ignore= and globs= say how path= walks a directory, and the text is no directory",
                    ));
                }
                let held = Input::of(obj)?;
                let part = rules::Part::of_text(&held, &reading)?;
                let ahead = Ahead { bytes: part.byte_base, units: part.units_ahead, lines: part.line_base.unwrap_or(0) };
                let shown = match held {
                    Input::Text(_) => Input::Text(
                        std::str::from_utf8(&part.text).map_err(|e| PyValueError::new_err(e.to_string()))?,
                    ),
                    Input::Bytes(_) => Input::Bytes(&part.text),
                };
                ready.extend(grep_input(py, slf.get(), &shown, None, ahead, &ask, &mut stats)?);
            }
            (None, Some(paths)) => {
                let walked = rules::walked(py, paths, &reading.walk)?;
                if !walked.errors.is_empty() {
                    return Err(PyOSError::new_err(walked.errors.join("; ")));
                }
                lone = walked.lone;
                files.extend(walked.files);
            }
            (Some(_), Some(_)) | (None, None) => {
                return Err(PyTypeError::new_err(
                    "give the text, a str or bytes, or path=, a file or directory's path or a list of them; one of them",
                ));
            }
        }
        let stats = if files.is_empty() {
            stats.elapsed = started.elapsed();
            publish_stats(stats);
            None
        } else {
            Some(stats)
        };
        Ok(Lines { ready, files, lone, pattern: slf.clone().unbind(), reading, ask, stats, started })
    }

    /// The first match with its registers, or None.
    #[pyo3(signature = (input, *, head = None, tail = None, lines = None, unit = "line"))]
    fn captures(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Option<Match>> {
        self.find(py, input, head, tail, lines, unit)
    }

    /// Every match with its registers, one at a time.
    #[pyo3(signature = (input, *, head = None, tail = None, lines = None, unit = "line"))]
    fn captures_iter(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Matches> {
        self.find_iter(py, input, head, tail, lines, unit, "auto", false, None)
    }

    /// `input` with every match replaced by `repl`: a template rendered at
    /// each match, or a callable handed each match and returning its
    /// replacement in the input's type.
    ///
    /// Under `head`, `tail` or `lines`, that part of the input alone is
    /// rewritten and given back, as the command line prints a rewritten
    /// window alone. `backend`, `dual_grain` and `chunk_size` say how the
    /// matches are found, as `scan` takes them.
    #[pyo3(signature = (repl, input, *, head = None, tail = None, lines = None, unit = "line", backend = "auto", dual_grain = false, chunk_size = None))]
    // The ten are the call's own arguments, the window and engine keywords
    // among them, each read once here.
    #[allow(clippy::too_many_arguments)]
    fn rewrite(
        &self,
        py: Python<'_>,
        repl: &Bound<'_, PyAny>,
        input: &Bound<'_, PyAny>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        backend: &str,
        dual_grain: bool,
        chunk_size: Option<usize>,
    ) -> PyResult<Py<PyAny>> {
        let engine = engine_of(backend, dual_grain, chunk_size)?;
        let input = Input::of(input)?;
        let range = window_range(input.bytes(), head, tail, lines, unit)?;
        self.rewritten(py, repl, &input, range, None, &engine)
    }

    /// `input` with its first `n` matches replaced; under `head`, `tail` or
    /// `lines`, the first `n` of that part, given back alone.
    #[pyo3(signature = (repl, input, n, *, head = None, tail = None, lines = None, unit = "line"))]
    // The eight are the call's own arguments, the four window keywords among
    // them, each read once here.
    #[allow(clippy::too_many_arguments)]
    fn rewrite_n(
        &self,
        py: Python<'_>,
        repl: &Bound<'_, PyAny>,
        input: &Bound<'_, PyAny>,
        n: usize,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Py<PyAny>> {
        let input = Input::of(input)?;
        let range = window_range(input.bytes(), head, tail, lines, unit)?;
        self.rewritten(py, repl, &input, range, Some(n), &trex::Engine::PLAIN)
    }

    /// `input` with its first match replaced; under `head`, `tail` or
    /// `lines`, the first of that part, given back alone.
    #[pyo3(signature = (repl, input, *, head = None, tail = None, lines = None, unit = "line"))]
    // The seven are the call's own arguments, the four window keywords among
    // them, each read once here.
    #[allow(clippy::too_many_arguments)]
    fn rewrite_first(
        &self,
        py: Python<'_>,
        repl: &Bound<'_, PyAny>,
        input: &Bound<'_, PyAny>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Py<PyAny>> {
        let input = Input::of(input)?;
        let range = window_range(input.bytes(), head, tail, lines, unit)?;
        self.rewritten(py, repl, &input, range, Some(1), &trex::Engine::PLAIN)
    }

    /// `input` with every match masked, as `trex redact` masks one: each
    /// character of a match by `mask`, a character, or a token per masked
    /// run as the command line's `--mask` takes one; the fields `keep` names
    /// (`"card:last4, ip:octet1-2"`) left unmasked. Under `head`,
    /// `tail` or `lines`, that part of the input alone is redacted and given
    /// back, so nothing outside it comes back unmasked.
    #[pyo3(signature = (input, *, keep = None, mask = "*", head = None, tail = None, lines = None, unit = "line"))]
    // The eight are the call's own arguments, the four window keywords among
    // them, each read once here.
    #[allow(clippy::too_many_arguments)]
    fn redact(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
        keep: Option<&str>,
        mask: &str,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Py<PyAny>> {
        let input = Input::of(input)?;
        let range = window_range(input.bytes(), head, tail, lines, unit)?;
        let keeps = match keep {
            Some(list) => trex::Keep::parse_list(list, &self.inner.capture_names())
                .map_err(|e| PyValueError::new_err(format!("keep= error at byte {}: {}", e.pos, e.msg)))?,
            None => Vec::new(),
        };
        let mut mask = trex::Mask::parse(mask).map_err(|e| PyValueError::new_err(format!("mask= {e}")))?;
        let bytes = &input.bytes()[range];
        let out = py.detach(|| {
            let spans = self.spans(bytes);
            let matches = self.resolved(bytes, &spans);
            let edits = trex::redactions_with_shapes(bytes, &matches, &keeps, &mut mask, &self.inner, &self.shapes);
            splice(bytes, &edits)
        });
        input.wrap(py, &out)
    }

    /// Redact the files `path` names in place, as `trex redact --in-place`
    /// writes them: each match masked as `redact` masks one, `keep` and
    /// `mask` read as it reads them, in the encoding the file is read in,
    /// every byte outside a match as it was. Returns how many matches were
    /// masked. A pseudonym names each value alike across the files, read in
    /// walk order. `path`, the window and the walk's switches are read as
    /// `diff` reads them, and `review`, `explain` and `show_skipped` as
    /// `rewrite_file` reads them.
    #[pyo3(signature = (path, *, keep = None, mask = "*", review = None, show_skipped = false, explain = false, head = None, tail = None, lines = None, unit = "line", hidden = false, no_ignore = false, binary = false))]
    // Each keyword is one of the redaction's own options, read once here.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    fn redact_file(
        &self,
        py: Python<'_>,
        path: PathArg,
        keep: Option<&str>,
        mask: &str,
        review: Option<&Bound<'_, PyAny>>,
        show_skipped: bool,
        explain: bool,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        hidden: bool,
        no_ignore: bool,
        binary: bool,
    ) -> PyResult<usize> {
        review_asked(review, show_skipped, explain)?;
        let reading = rules::Reading::of(head, tail, lines, unit, hidden, no_ignore, None, binary)?;
        let mut errors = Vec::new();
        let edited = self.redactions(py, path, keep, mask, &reading, &mut errors)?;
        let explain_change = |py: Python<'_>, text: &[u8], start: usize, end: usize| {
            self.change_explained(py, text, start, end, "the redaction's own scan")
        };
        let explaining = explain.then_some(&explain_change as &rules::ExplainChange<'_>);
        write_edited(py, edited, review, show_skipped, explaining, errors)
    }

    /// The unified diff a redaction of the files `path` names would make, as
    /// `trex redact --dry-run` prints it: each file's in walk order, with
    /// `context` lines around each change. Everything else is read as
    /// `redact_file` reads it.
    #[pyo3(signature = (path, *, keep = None, mask = "*", context = 3, head = None, tail = None, lines = None, unit = "line", hidden = false, no_ignore = false, binary = false))]
    // Each keyword is one of the redaction's own options, read once here.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    fn redact_diff(
        &self,
        py: Python<'_>,
        path: PathArg,
        keep: Option<&str>,
        mask: &str,
        context: usize,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        hidden: bool,
        no_ignore: bool,
        binary: bool,
    ) -> PyResult<String> {
        let reading = rules::Reading::of(head, tail, lines, unit, hidden, no_ignore, None, binary)?;
        let mut errors = Vec::new();
        let edited = self.redactions(py, path, keep, mask, &reading, &mut errors)?;
        diffs_of(&edited, context, errors)
    }

    /// The unified diff a rewrite of the files `path` names by `repl` would
    /// make, as `trex rewrite --dry-run` prints it: each file's in walk order,
    /// with `context` lines around each change; empty where nothing matches.
    /// `repl` is a template, or a callable taking a `Match` and returning a
    /// str. `path` is a file, a directory walked as `trex.files` walks one,
    /// or a list of them; `max_count` replaces the first matches of each file
    /// alone, `backend` names the engine that finds them, and `head`, `tail`
    /// or `lines` confines the changes to that part of each file. A file
    /// holding a NUL byte is read under `binary=True`, and otherwise passed
    /// over, one named alone raising `ValueError`.
    #[pyo3(signature = (repl, path, *, context = 3, max_count = None, backend = "auto", head = None, tail = None, lines = None, unit = "line", hidden = false, no_ignore = false, binary = false))]
    // Each keyword is one of the rewrite's own options, read once here.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    fn diff(
        &self,
        py: Python<'_>,
        repl: &Bound<'_, PyAny>,
        path: PathArg,
        context: usize,
        max_count: Option<usize>,
        backend: &str,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        hidden: bool,
        no_ignore: bool,
        binary: bool,
    ) -> PyResult<String> {
        let engine = engine_of(backend, false, None)?;
        let reading = rules::Reading::of(head, tail, lines, unit, hidden, no_ignore, None, binary)?;
        let mut errors = Vec::new();
        let mut edited = Vec::new();
        for f in files_edited(py, path, &reading, "rewrites", &mut errors)? {
            let edits = self.rewrite_edits(py, repl, &f.text, &reading, max_count, &engine)?;
            edited.push((f, edits));
        }
        diffs_of(&edited, context, errors)
    }

    /// Rewrite the files `path` names in place by `repl`, as `trex rewrite
    /// --in-place` writes them: each replacement in the encoding the file is
    /// read in, behind its own byte order mark, every byte outside a match as
    /// it was. Returns how many replacements were made; a file with none is
    /// not written. `path`, `max_count`, `backend`, the window and the walk's
    /// switches are read as `diff` reads them.
    ///
    /// `review`, a callable, is handed each change as a `trex.Change` first,
    /// as `lib.fix` hands a fix, and nothing is written until it has seen the
    /// last; `explain=True` gives each change the explanation of its match,
    /// and `show_skipped=True` names where a template answer skipped one.
    #[pyo3(signature = (repl, path, *, review = None, show_skipped = false, explain = false, max_count = None, backend = "auto", head = None, tail = None, lines = None, unit = "line", hidden = false, no_ignore = false, binary = false))]
    // Each keyword is one of the rewrite's own options, read once here.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    fn rewrite_file(
        &self,
        py: Python<'_>,
        repl: &Bound<'_, PyAny>,
        path: PathArg,
        review: Option<&Bound<'_, PyAny>>,
        show_skipped: bool,
        explain: bool,
        max_count: Option<usize>,
        backend: &str,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        hidden: bool,
        no_ignore: bool,
        binary: bool,
    ) -> PyResult<usize> {
        review_asked(review, show_skipped, explain)?;
        let engine = engine_of(backend, false, None)?;
        let reading = rules::Reading::of(head, tail, lines, unit, hidden, no_ignore, None, binary)?;
        let mut errors = Vec::new();
        let mut edited = Vec::new();
        for f in files_edited(py, path, &reading, "rewrites", &mut errors)? {
            let edits = self.rewrite_edits(py, repl, &f.text, &reading, max_count, &engine)?;
            edited.push((f, edits));
        }
        let explain_change = |py: Python<'_>, text: &[u8], start: usize, end: usize| {
            self.change_explained(py, text, start, end, "the rewrite's own scan")
        };
        let explaining = explain.then_some(&explain_change as &rules::ExplainChange<'_>);
        write_edited(py, edited, review, show_skipped, explaining, errors)
    }

    /// The pieces of `input` between its matches.
    fn split(&self, py: Python<'_>, input: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyAny>>> {
        self.split_upto(py, input, usize::MAX)
    }

    /// At most `limit` pieces, the last being the unsplit remainder.
    fn splitn(&self, py: Python<'_>, input: &Bound<'_, PyAny>, limit: usize) -> PyResult<Vec<Py<PyAny>>> {
        self.split_upto(py, input, limit)
    }

    fn __repr__(&self) -> String {
        format!("Pattern({:?})", self.source)
    }
}

/// Patterns asked together over one input.
#[pyclass(frozen, module = "trex")]
struct PatternSet {
    inner: trex::PatternSet,
    /// Each member's capture names in written order, which its matches
    /// count a numbered reference through.
    bounds: Vec<Arc<[String]>>,
    /// Each member's capture kinds, in member order, for `Match.value`.
    kinds: Vec<CaptureKinds>,
    /// Each member's own pattern with the set's declarations, in member
    /// order, for `Match.explain`.
    shared: Vec<Arc<Compiled>>,
}

impl PatternSet {
    fn over(inner: trex::PatternSet) -> Self {
        let bounds = inner.patterns().iter().map(|p| Arc::from(p.capture_names())).collect();
        let kinds = inner.patterns().iter().map(|p| Arc::from(p.capture_kinds())).collect();
        let shared = inner
            .patterns()
            .iter()
            .enumerate()
            .map(|(i, p)| {
                Arc::new(Compiled { pattern: p.clone(), shapes: inner.shapes().clone(), member: Some(inner.name(i)) })
            })
            .collect();
        PatternSet { inner, bounds, kinds, shared }
    }

    /// Whether any member binds a register under a repetition, so its
    /// matches carry every binding.
    fn lists(&self) -> bool {
        self.inner.patterns().iter().any(trex::ast::Pattern::has_list_registers)
    }

    /// The matches found with their members, as `Match` objects under each
    /// member's own register names.
    fn matches_of(
        &self,
        py: Python<'_>,
        input: &Input<'_>,
        found: Vec<(usize, trex::Match)>,
    ) -> PyResult<Vec<(usize, Match)>> {
        let mut placing = Placing::of(input);
        found
            .into_iter()
            .map(|(i, m)| Ok((i, Match::of(py, input, &mut placing, &m, &self.bounds[i], &self.kinds[i], &self.shared[i])?)))
            .collect()
    }
}

#[pymethods]
impl PatternSet {
    /// From patterns or their sources, in the order the answers index.
    #[new]
    fn new(patterns: Vec<Bound<'_, PyAny>>) -> PyResult<Self> {
        let mut pats = Vec::with_capacity(patterns.len());
        for p in &patterns {
            if p.is_instance_of::<Pattern>() {
                let p: PyRef<'_, Pattern> = p.extract()?;
                pats.push(p.inner.clone());
            } else {
                let src: String = p.extract()?;
                pats.push(trex::parse(&src).map_err(parse_error)?);
            }
        }
        Ok(PatternSet::over(trex::PatternSet::new(pats)))
    }

    /// The set a pattern file declares: a `let` line is a member under its
    /// name, a bare pattern line a member under its line number, and the
    /// file's `kind` and `shape` lines serve every member. A line that is
    /// neither is a `ValueError` naming the line.
    #[staticmethod]
    fn from_file(path: &str) -> PyResult<Self> {
        let mut shapes = trex::ShapeSet::new();
        let set = trex::PatternSet::from_file(std::path::Path::new(path), &mut shapes)
            .map_err(|e| PyValueError::new_err(e.msg))?;
        Ok(PatternSet::over(set))
    }

    /// Each member's name: a let's name or a line number for a set from a
    /// file, the index as text for a set built from patterns.
    #[getter]
    fn names(&self) -> Vec<String> {
        (0..self.inner.len()).map(|i| self.inner.name(i)).collect()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Every match of every member, each with its member's index, in
    /// position order, the registers resolved under the member's own names.
    fn scan(&self, py: Python<'_>, input: &Bound<'_, PyAny>) -> PyResult<Vec<(usize, Match)>> {
        let input = Input::of(input)?;
        let bytes = input.bytes();
        let set = &self.inner;
        let lists = self.lists();
        let found = py.detach(|| set.scan_matches(bytes, lists));
        self.matches_of(py, &input, found)
    }

    /// Whether any pattern matches.
    fn is_match(&self, py: Python<'_>, input: &Bound<'_, PyAny>) -> PyResult<bool> {
        let input = Input::of(input)?;
        let bytes = input.bytes();
        let set = &self.inner;
        Ok(py.detach(|| set.is_match(bytes)))
    }

    /// The indices of the patterns that match, in order.
    fn matches(&self, py: Python<'_>, input: &Bound<'_, PyAny>) -> PyResult<Vec<usize>> {
        let input = Input::of(input)?;
        let bytes = input.bytes();
        let set = &self.inner;
        Ok(py.detach(|| set.matches(bytes)))
    }

    /// The indices of the patterns that match at or after `at`, a position
    /// in the input's units.
    fn matches_at(&self, py: Python<'_>, input: &Bound<'_, PyAny>, at: usize) -> PyResult<Vec<usize>> {
        let input = Input::of(input)?;
        let bytes = input.bytes();
        let from = input.byte_at(at);
        let set = &self.inner;
        Ok(py.detach(|| set.matches_at(bytes, from).iter().collect()))
    }

    /// Each matching pattern's index with its first match, in member order,
    /// the registers resolved under the member's own names.
    fn matches_with_spans(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<(usize, Match)>> {
        let input = Input::of(input)?;
        let bytes = input.bytes();
        let set = &self.inner;
        let lists = self.lists();
        let mut found = py.detach(|| set.first_matches(bytes, lists));
        found.sort_by_key(|(i, _)| *i);
        self.matches_of(py, &input, found)
    }

    /// The matches of every member in `text`, or in the files `path=` names,
    /// grouped by `key`, as `Pattern.group_by` groups one pattern's, with
    /// `${pattern}` the name of the member that made each match. A register
    /// two members bind on different kinds takes no aggregate.
    #[pyo3(signature = (key, text = None, *, path = None, order = "key", limit = None, sum = None, avg = None, min = None, max = None, percentiles = None, percentile_method = "nearest", duration_unit = "ns", binary = false, head = None, tail = None, lines = None, unit = "line"))]
    // Each keyword is one of the table's own options, read once here.
    #[allow(clippy::too_many_arguments)]
    fn group_by(
        &self,
        py: Python<'_>,
        key: &str,
        text: Option<&Bound<'_, PyAny>>,
        path: Option<PathArg>,
        order: &str,
        limit: Option<usize>,
        sum: Option<Registers>,
        avg: Option<Registers>,
        min: Option<Registers>,
        max: Option<Registers>,
        percentiles: Option<&Bound<'_, PyDict>>,
        percentile_method: &str,
        duration_unit: &str,
        binary: bool,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Vec<Group>> {
        use trex::typed::Agg;
        let mut ask = TableAsk::of(order, limit, binary, head, tail, lines, unit)?;
        ask.how = percentile_method_of(percentile_method)?;
        ask.duration = duration_unit_of("duration_unit", duration_unit)?;
        let asked = asked_columns([(Agg::Sum, sum), (Agg::Avg, avg), (Agg::Min, min), (Agg::Max, max)], percentiles)?;
        grouped(py, &Grouping::Set(self), key, text, path, &ask, &asked)
    }

    /// The distinct keys every member's matches render under `key`, as
    /// `Pattern.distinct` gives one pattern's.
    #[pyo3(signature = (key, text = None, *, path = None, order = "key", limit = None, binary = false, head = None, tail = None, lines = None, unit = "line"))]
    // Each keyword is one of the table's own options, read once here.
    #[allow(clippy::too_many_arguments)]
    fn distinct(
        &self,
        py: Python<'_>,
        key: &str,
        text: Option<&Bound<'_, PyAny>>,
        path: Option<PathArg>,
        order: &str,
        limit: Option<usize>,
        binary: bool,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Vec<String>> {
        let ask = TableAsk::of(order, limit, binary, head, tail, lines, unit)?;
        distinct_keys(py, &Grouping::Set(self), key, text, path, &ask)
    }
}

/// One register, or a list of them, as `sum=`, `avg=`, `min=` and `max=`
/// take them.
#[derive(FromPyObject)]
enum Registers {
    One(String),
    Many(Vec<String>),
}

/// A percent, or a list of them, as `percentiles=` maps a register to them.
#[derive(FromPyObject)]
enum Percents {
    One(u32),
    Many(Vec<u32>),
}

/// The files a table reads under `path=`: one path, or a list of them, each
/// a str or an `os.PathLike`.
#[derive(FromPyObject)]
enum PathArg {
    One(std::path::PathBuf),
    Many(Vec<std::path::PathBuf>),
}

/// What a table scans: one pattern, or a set whose every match carries the
/// member that made it.
enum Grouping<'a> {
    One(&'a Pattern),
    Set(&'a PatternSet),
}

impl Grouping<'_> {
    /// The register names a key may write.
    fn capture_names(&self) -> Vec<String> {
        match self {
            Grouping::One(p) => p.inner.capture_names(),
            Grouping::Set(s) => s.inner.capture_names(),
        }
    }

    /// The kind each register binds, which decides the aggregates it takes.
    fn capture_kinds(&self) -> Vec<(String, Option<trex::token::TokenKind>)> {
        match self {
            Grouping::One(p) => p.inner.capture_kinds(),
            Grouping::Set(s) => s.inner.capture_kinds(),
        }
    }

    /// Every match of `bytes` with its registers resolved, and the name of
    /// the member that made it where a set did.
    fn found(&self, bytes: &[u8]) -> Vec<(Option<String>, trex::Match)> {
        match self {
            Grouping::One(p) => {
                let spans = p.spans(bytes);
                p.resolved(bytes, &spans).into_iter().map(|m| (None, m)).collect()
            }
            Grouping::Set(s) => s
                .inner
                .scan_matches(bytes, s.lists())
                .into_iter()
                .map(|(i, m)| (Some(s.inner.name(i)), m))
                .collect(),
        }
    }
}

/// What a table reads and how its rows come back, from the keywords the
/// table methods take.
struct TableAsk {
    order: trex::aggregate::Order,
    limit: Option<usize>,
    select: Option<trex::window::Select>,
    unit: trex::records::RecordUnit,
    binary: bool,
    how: trex::typed::Percentile,
    duration: trex::typed::DurationUnit,
}

impl TableAsk {
    /// The rows in `order`, `"key"` or `"count"`, as far as `limit`, over the
    /// part of each input `head`, `tail` or `lines` names in records of
    /// `unit`, a file holding a NUL byte read where `binary` asks.
    fn of(
        order: &str,
        limit: Option<usize>,
        binary: bool,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<TableAsk> {
        let order = match order {
            "key" => trex::aggregate::Order::Key,
            "count" => trex::aggregate::Order::Count,
            other => return Err(PyValueError::new_err(format!("order takes 'key' or 'count', not {other:?}"))),
        };
        let select = select_of(head, tail, lines)?;
        if select.is_none() && unit != "line" {
            return Err(PyValueError::new_err(format!(
                "unit={unit:?} names what head=, tail= and lines= count, and none was given"
            )));
        }
        Ok(TableAsk {
            order,
            limit,
            select,
            unit: unit_of(unit)?,
            binary,
            how: trex::typed::Percentile::Nearest,
            duration: trex::typed::DurationUnit::Nanoseconds,
        })
    }
}

/// One input a table reads: its name, the part read, that part's offset in
/// it, and the unit the input's offsets count with how many of it come
/// before the part, where they were counted.
struct Piece<'a> {
    name: &'a str,
    bytes: &'a [u8],
    byte_base: Option<usize>,
    line_base: Option<usize>,
    unit: OffsetUnit,
    units_ahead: Option<usize>,
}

/// Count every match of `grouping` in `piece` into `table`, under the key
/// `template` renders at it.
fn tally(
    py: Python<'_>,
    grouping: &Grouping<'_>,
    template: &trex::Template,
    table: &mut trex::aggregate::Table,
    piece: &Piece<'_>,
) {
    py.detach(|| {
        let index = template
            .reads_place()
            .then(|| trex::files::LineIndex::new(piece.bytes).within(piece.byte_base, piece.line_base));
        let mut units = template.reads_offsets().then(|| Offsets::new(piece.bytes, piece.unit));
        for (member, m) in grouping.found(piece.bytes) {
            let (line, col) = index.as_ref().map_or((0, 0), |ix| ix.line_col(piece.bytes, m.start));
            let offsets = match units.as_mut() {
                Some(u) => piece.units_ahead.map(|ahead| (ahead + u.at(m.start), ahead + u.at(m.end))),
                None => None,
            };
            let place = trex::ReportAt { path: piece.name, line, col, offsets, pattern: member.as_deref(), rule: None };
            table.add(template.render_report(&m, piece.bytes, &place), &m, piece.bytes);
        }
    });
}

/// The file at `path` as a table reads it: the part `ask` names, or the
/// whole file decoded, marked binary where it holds a NUL byte and read as
/// text only where `asked` asks for that; and the characters ahead of the
/// part, counted where `asked` places it.
fn read_for_table(
    path: &std::path::Path,
    ask: &TableAsk,
    asked: trex::window::Asked,
) -> std::io::Result<(trex::window::Window, Option<usize>)> {
    if let Some(select) = ask.select {
        if asked.offsets {
            return trex::window::read_file_with_chars(path, select, &ask.unit, asked);
        }
        return trex::window::read_file(path, select, &ask.unit, asked).map(|w| (w, None));
    }
    let raw = std::fs::read(path)?;
    let binary = trex::files::is_binary(&raw);
    let bytes = if binary && !asked.binary { Vec::new() } else { trex::encoding::decode(raw) };
    let window = trex::window::Window {
        bytes,
        byte_base: Some(0),
        line_base: Some(0),
        binary,
        ..trex::window::Window::default()
    };
    Ok((window, Some(0)))
}

/// The matches of `grouping` in `text`, or in the files `path` names, under
/// the keys `key` renders, with the values `columns` read.
fn table_of(
    py: Python<'_>,
    grouping: &Grouping<'_>,
    key: &str,
    columns: Vec<trex::aggregate::Column>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathArg>,
    ask: &TableAsk,
) -> PyResult<trex::aggregate::Table> {
    let template = trex::Template::parse_report(key, &grouping.capture_names())
        .map_err(|e| PyValueError::new_err(format!("key error at byte {}: {}", e.pos, e.msg)))?;
    // Nothing here explains a match, so every `${@axis}` would render empty
    // and put every match under one key.
    if template.reads_explanation() {
        return Err(PyValueError::new_err(
            "a key groups matches and reads no axis; ${@axis} explains one match, as Match.explain does",
        ));
    }
    let mut table = trex::aggregate::Table::new(columns);
    match (text, path) {
        (Some(obj), None) => {
            let input = Input::of(obj)?;
            let bytes = input.bytes();
            match ask.select {
                Some(select) => {
                    let window = trex::window::window_of(bytes, select, &ask.unit);
                    let piece = Piece {
                        name: "",
                        bytes: &window.bytes,
                        byte_base: window.byte_base,
                        line_base: window.line_base,
                        unit: input.unit(),
                        units_ahead: window.byte_base.map(|b| units_of(&input).at(b)),
                    };
                    tally(py, grouping, &template, &mut table, &piece);
                }
                None => {
                    let piece = Piece {
                        name: "",
                        bytes,
                        byte_base: Some(0),
                        line_base: Some(0),
                        unit: input.unit(),
                        units_ahead: Some(0),
                    };
                    tally(py, grouping, &template, &mut table, &piece);
                }
            }
        }
        (None, Some(paths)) => {
            let roots: Vec<String> = match paths {
                PathArg::One(p) => vec![p.to_string_lossy().into_owned()],
                PathArg::Many(ps) => ps.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
            };
            let asked = trex::window::Asked {
                numbers: template.reads_place(),
                offsets: template.reads_offsets(),
                binary: ask.binary,
                units: false,
            };
            let (sources, mut errors) =
                py.detach(|| trex::files::collect(&roots, &trex::files::WalkOptions::default()));
            // As a scan does: a file named alone that holds a NUL byte is
            // refused aloud, and one among several or found by a walk is
            // passed over.
            let lone = sources.len() == 1 && !roots.iter().any(|r| std::path::Path::new(r).is_dir());
            for src in &sources {
                let trex::files::Source::File(file) = src else {
                    continue;
                };
                let name = src.name();
                match py.detach(|| read_for_table(file, ask, asked)) {
                    Err(e) => errors.push(format!("{name}: {e}")),
                    Ok((window, _)) if window.binary && !ask.binary => {
                        if lone {
                            return Err(PyValueError::new_err(format!(
                                "{name} holds a NUL byte and is binary; binary=True reads it"
                            )));
                        }
                    }
                    Ok((window, chars_ahead)) => {
                        let piece = Piece {
                            name: &name,
                            bytes: &window.bytes,
                            byte_base: window.byte_base,
                            line_base: window.line_base,
                            unit: OffsetUnit::CodePoints,
                            units_ahead: chars_ahead,
                        };
                        tally(py, grouping, &template, &mut table, &piece);
                    }
                }
            }
            if !errors.is_empty() {
                return Err(PyOSError::new_err(errors.join("; ")));
            }
        }
        (Some(_), Some(_)) | (None, None) => {
            return Err(PyTypeError::new_err(
                "give the text, a str or bytes, or path=, a file or directory's path or a list of them; one of them",
            ));
        }
    }
    Ok(table)
}

/// The keyword that asks for `agg`.
fn keyword(agg: trex::typed::Agg) -> &'static str {
    use trex::typed::Agg;
    match agg {
        Agg::Sum => "sum=",
        Agg::Avg => "avg=",
        Agg::Min => "min=",
        Agg::Max => "max=",
        Agg::Pct(_) => "percentiles=",
    }
}

/// Why a column cannot be computed, in the keywords' terms.
fn column_error(e: &trex::aggregate::ColumnError) -> PyErr {
    use trex::aggregate::ColumnError;
    PyValueError::new_err(match e {
        ColumnError::NotOneRegister { agg, spec } => {
            format!("{} takes one register, as 'size' or '${{size}}', not {spec:?}", keyword(*agg))
        }
        ColumnError::NotBound { agg, register } => {
            format!("{} names {register}, which the pattern does not bind", keyword(*agg))
        }
        ColumnError::NoSingleKind { agg, register } => format!(
            "{} needs a register binding one typed kind, and {register} binds a run, a repetition or an \
             alternation, or two patterns of the set bind it on different kinds, so its kind varies between matches",
            keyword(*agg)
        ),
        ColumnError::NotAdmitted { agg, register, kind } => {
            let why = if agg.needs_arithmetic() {
                "adding one is not defined. Numeric kinds are number, bytesize, duration, money and percent"
            } else {
                "it carries no value to order. Ordered kinds are the numeric ones plus timestamp, version and address"
            };
            format!("{} over {register}, which binds a {}, and {why}.", keyword(*agg), kind.name())
        }
        ColumnError::LinearOnOrdered { register, kind } => format!(
            "percentile_method='linear' cannot interpolate a {}, which {register} binds. Use 'nearest', 'lower', \
             or 'hybrid', which interpolates only where a kind allows it.",
            kind.name()
        ),
    })
}

/// The percentile method `name` names.
fn percentile_method_of(name: &str) -> PyResult<trex::typed::Percentile> {
    use trex::typed::Percentile;
    match name {
        "nearest" => Ok(Percentile::Nearest),
        "linear" => Ok(Percentile::Linear),
        "lower" => Ok(Percentile::Lower),
        "hybrid" => Ok(Percentile::Hybrid),
        other => Err(PyValueError::new_err(format!(
            "percentile_method takes 'nearest', 'linear', 'lower' or 'hybrid', not {other:?}"
        ))),
    }
}

/// Whether `a` and `b` name one register, `t` and `${t}` alike.
fn same_register(a: &str, b: &str) -> bool {
    match (trex::aggregate::one_register(a), trex::aggregate::one_register(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// The columns `sum=`, `avg=`, `min=`, `max=` and `percentiles=` ask for,
/// in that order, each an aggregate and the register it reads. A register
/// one aggregate names twice is refused, since its value has one place.
fn asked_columns(
    by_register: [(trex::typed::Agg, Option<Registers>); 4],
    percentiles: Option<&Bound<'_, PyDict>>,
) -> PyResult<Vec<(trex::typed::Agg, String)>> {
    use trex::typed::Agg;
    let mut asked: Vec<(Agg, String)> = Vec::new();
    for (agg, registers) in by_register {
        let specs = match registers {
            Some(Registers::One(spec)) => vec![spec],
            Some(Registers::Many(specs)) => specs,
            None => Vec::new(),
        };
        for spec in specs {
            if asked.iter().any(|(a, s)| *a == agg && same_register(s, &spec)) {
                return Err(PyValueError::new_err(format!("{} names {spec} twice", keyword(agg))));
            }
            asked.push((agg, spec));
        }
    }
    let Some(percentiles) = percentiles else {
        return Ok(asked);
    };
    for (register, percents) in percentiles.iter() {
        let spec: String = register
            .extract()
            .map_err(|e: PyErr| PyTypeError::new_err(format!("percentiles= is keyed by register names: {e}")))?;
        let percents: Percents = percents.extract().map_err(|e: PyErr| {
            PyTypeError::new_err(format!("percentiles= maps {spec} to a percent or a list of them: {e}"))
        })?;
        let percents = match percents {
            Percents::One(p) => vec![p],
            Percents::Many(ps) => ps,
        };
        for p in percents {
            if p > 100 {
                return Err(PyValueError::new_err(format!(
                    "percentiles= asks {spec} for its {p}th percentile; a percentile is 0 to 100"
                )));
            }
            if asked.iter().any(|(a, s)| *a == Agg::Pct(p) && same_register(s, &spec)) {
                return Err(PyValueError::new_err(format!("percentiles= names {p} of {spec} twice")));
            }
            asked.push((Agg::Pct(p), spec.clone()));
        }
    }
    Ok(asked)
}

/// `text`, a number as trex writes one, in digits or as digits and a power
/// of ten, as the Python type a value of it is: an int where it is whole and
/// fits one, a Decimal otherwise.
fn number_of(py: Python<'_>, text: &str) -> PyResult<Py<PyAny>> {
    let Some(d) = trex::typed::number_value(text) else {
        return Err(PyValueError::new_err(format!("{text:?} is not a number")));
    };
    Match::py_value(
        py,
        trex::token::TokenKind::Number,
        &trex::typed::TypedValue::Num(d),
        trex::typed::DurationUnit::Nanoseconds,
    )
}

/// One column's aggregate over the values a group collected: the value
/// itself where the aggregate picks one, a least or greatest value or a
/// percentile that is one of the observed values, as `Match.value` gives it;
/// an average as an exact `fractions.Fraction`; and a sum or an interpolated
/// percentile as an int or a Decimal. Durations are in `ask.duration`.
fn aggregate_value(
    py: Python<'_>,
    held: &trex::typed::Collected,
    col: &trex::aggregate::Column,
    ask: &TableAsk,
) -> PyResult<Py<PyAny>> {
    use trex::typed::{Agg, QuotientForm};
    if let Some(v) = held.picked(col.agg, col.kind, ask.how) {
        return Match::py_value(py, col.kind, v, ask.duration);
    }
    let mut style = trex::typed::ValueStyle::new();
    style.duration = ask.duration;
    let form = if col.agg == Agg::Avg { QuotientForm::Rational } else { QuotientForm::Repetend };
    let Some(rendered) = held.report(col.agg, col.kind, style, form, ask.how, trex::Clock::current()) else {
        return Ok(py.None());
    };
    // A figure a double does not hold exactly is rendered as a JSON string.
    let figure = match rendered.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Some(inner) => inner,
        None => rendered.as_str(),
    };
    if col.agg == Agg::Avg {
        return Ok(py.import("fractions")?.getattr("Fraction")?.call1((figure,))?.unbind());
    }
    number_of(py, figure)
}

/// One group of a table: the key its matches rendered, how many there are,
/// and each aggregate asked for, by the register it read.
#[pyclass(frozen, module = "trex")]
struct Group {
    /// The key the group's matches rendered.
    #[pyo3(get)]
    key: String,
    /// How many matches the group holds.
    #[pyo3(get)]
    count: u64,
    /// Each register `sum=` named, to the sum of the values the group's
    /// matches bound to it; None where they bound none.
    #[pyo3(get)]
    sum: Py<PyDict>,
    /// Each register `avg=` named, to the exact average as a `Fraction`.
    #[pyo3(get)]
    avg: Py<PyDict>,
    /// Each register `min=` named, to its least value.
    #[pyo3(get)]
    min: Py<PyDict>,
    /// Each register `max=` named, to its greatest value.
    #[pyo3(get)]
    max: Py<PyDict>,
    /// Each register `percentiles=` named, to a dict of each percent asked
    /// to its value.
    #[pyo3(get)]
    percentiles: Py<PyDict>,
}

impl Group {
    /// `row` of a table computing `columns`, its aggregates read as `ask`
    /// says.
    fn of(
        py: Python<'_>,
        columns: &[trex::aggregate::Column],
        row: &trex::aggregate::Row<'_>,
        ask: &TableAsk,
    ) -> PyResult<Group> {
        use trex::typed::Agg;
        let (sum, avg, min, max) = (PyDict::new(py), PyDict::new(py), PyDict::new(py), PyDict::new(py));
        let percentiles = PyDict::new(py);
        for (col, held) in columns.iter().zip(&row.values) {
            let value = match held {
                Some(values) => aggregate_value(py, values, col, ask)?,
                None => py.None(),
            };
            let into = match col.agg {
                Agg::Sum => &sum,
                Agg::Avg => &avg,
                Agg::Min => &min,
                Agg::Max => &max,
                Agg::Pct(p) => {
                    let per = match percentiles.get_item(&col.register)? {
                        Some(per) => per.cast_into::<PyDict>()?,
                        None => {
                            let per = PyDict::new(py);
                            percentiles.set_item(&col.register, &per)?;
                            per
                        }
                    };
                    per.set_item(p, value)?;
                    continue;
                }
            };
            into.set_item(&col.register, value)?;
        }
        Ok(Group {
            key: row.key.to_string(),
            count: row.count,
            sum: sum.unbind(),
            avg: avg.unbind(),
            min: min.unbind(),
            max: max.unbind(),
            percentiles: percentiles.unbind(),
        })
    }
}

#[pymethods]
impl Group {
    /// The group as a plain dict: `key`, `count` and a copy of each
    /// aggregate's dict.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("key", &self.key)?;
        d.set_item("count", self.count)?;
        for (name, held) in [("sum", &self.sum), ("avg", &self.avg), ("min", &self.min), ("max", &self.max)] {
            d.set_item(name, held.bind(py).copy()?)?;
        }
        let percentiles = PyDict::new(py);
        for (register, per) in self.percentiles.bind(py).iter() {
            percentiles.set_item(register, per.cast_into::<PyDict>()?.copy()?)?;
        }
        d.set_item("percentiles", percentiles)?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let mut out = format!("Group(key={}, count={}", PyString::new(py, &self.key).repr()?, self.count);
        for (name, held) in [
            ("sum", &self.sum),
            ("avg", &self.avg),
            ("min", &self.min),
            ("max", &self.max),
            ("percentiles", &self.percentiles),
        ] {
            let held = held.bind(py);
            if !held.is_empty() {
                out.push_str(&format!(", {name}={}", held.repr()?));
            }
        }
        out.push(')');
        Ok(out)
    }
}

/// The groups of `grouping`'s matches in `text` or the files `path` names,
/// under the keys `key` renders, with the columns `asked` names.
fn grouped(
    py: Python<'_>,
    grouping: &Grouping<'_>,
    key: &str,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathArg>,
    ask: &TableAsk,
    asked: &[(trex::typed::Agg, String)],
) -> PyResult<Vec<Group>> {
    // Every column is checked against the pattern before anything is read.
    let columns = trex::aggregate::columns(asked, &grouping.capture_kinds(), ask.how).map_err(|e| column_error(&e))?;
    let table = table_of(py, grouping, key, columns, text, path, ask)?;
    let mut rows = table.rows(ask.order);
    if let Some(n) = ask.limit {
        rows.truncate(n);
    }
    rows.iter().map(|row| Group::of(py, table.columns(), row, ask)).collect()
}

/// The distinct keys of `grouping`'s matches in `text` or the files `path`
/// names, in `ask`'s order.
fn distinct_keys(
    py: Python<'_>,
    grouping: &Grouping<'_>,
    key: &str,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathArg>,
    ask: &TableAsk,
) -> PyResult<Vec<String>> {
    let table = table_of(py, grouping, key, Vec::new(), text, path, ask)?;
    let mut rows = table.rows(ask.order);
    if let Some(n) = ask.limit {
        rows.truncate(n);
    }
    Ok(rows.iter().map(|row| row.key.to_string()).collect())
}

/// A streaming scan: push chunks, take the matches that can no longer
/// change, finish for the rest. Spans are `(start, end)` byte offsets into
/// the whole stream, or `(member, start, end)` for a stream over a set.
#[pyclass(module = "trex")]
struct StreamScanner {
    inner: Option<trex::StreamScanner>,
    /// Whether the stream runs a set, whose spans carry their member.
    set: bool,
}

/// Committed spans as tuples: `(start, end)`, or `(member, start, end)`
/// for a stream over a set.
fn spans_py(py: Python<'_>, found: Vec<(usize, trex::Span)>, with_member: bool) -> PyResult<Vec<Py<PyAny>>> {
    found
        .into_iter()
        .map(|(i, s)| {
            let tuple = if with_member {
                (i, s.start(), s.end()).into_pyobject(py)?.into_any()
            } else {
                (s.start(), s.end()).into_pyobject(py)?.into_any()
            };
            Ok(tuple.unbind())
        })
        .collect()
}

#[pymethods]
impl StreamScanner {
    /// Over a pattern, whose spans are `(start, end)`, or a set, whose
    /// spans are `(member, start, end)` and which commits under its most
    /// conservative member.
    #[new]
    fn new(source: &Bound<'_, PyAny>) -> PyResult<Self> {
        if source.is_instance_of::<Pattern>() {
            let p: PyRef<'_, Pattern> = source.extract()?;
            return Ok(StreamScanner { inner: Some(trex::StreamScanner::new(p.inner.clone())), set: false });
        }
        if source.is_instance_of::<PatternSet>() {
            let s: PyRef<'_, PatternSet> = source.extract()?;
            return Ok(StreamScanner { inner: Some(trex::StreamScanner::over_set(s.inner.clone())), set: true });
        }
        Err(PyTypeError::new_err("a StreamScanner runs a Pattern or a PatternSet"))
    }

    /// Feed the next chunk and take the matches it settled.
    fn push(&mut self, py: Python<'_>, chunk: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyAny>>> {
        let input = Input::of(chunk)?;
        let bytes = input.bytes();
        let Some(s) = self.inner.as_mut() else {
            return Err(PyValueError::new_err("the stream is finished"));
        };
        py.detach(|| s.push(bytes));
        spans_py(py, s.drain_committed_with_members(), self.set)
    }

    /// End the stream and take every match not yet handed over.
    fn finish(&mut self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let Some(s) = self.inner.take() else {
            return Err(PyValueError::new_err("the stream is finished"));
        };
        let found = py.detach(|| s.finish_with_members());
        spans_py(py, found, self.set)
    }
}

/// The pattern `source`, compiled.
#[pyfunction]
#[pyo3(signature = (source, lib = None))]
fn parse(source: &str, lib: Option<&Bound<'_, PyAny>>) -> PyResult<Pattern> {
    Pattern::new(source, lib)
}

/// `text` as a pattern that matches it literally.
#[pyfunction]
fn escape(text: &str) -> String {
    trex::escape(text)
}

/// The part `select` names in records of `unit`: of `input`, a str or bytes,
/// in its type; or of the file at `path`, read from the end nearest the part
/// and no further, as `trex head` and `trex tail` read one, as a str.
fn listed(
    py: Python<'_>,
    select: trex::window::Select,
    input: Option<&Bound<'_, PyAny>>,
    path: Option<&str>,
    unit: &str,
) -> PyResult<Py<PyAny>> {
    let unit = unit_of(unit)?;
    match (input, path) {
        (Some(obj), None) => {
            let input = Input::of(obj)?;
            let r = trex::window::range_of(input.bytes(), select, &unit);
            Ok(input.slice(py, r.start, r.end))
        }
        (None, Some(p)) => {
            let asked = trex::window::Asked { binary: true, ..trex::window::Asked::default() };
            let window = py
                .detach(|| trex::window::read_file(std::path::Path::new(p), select, &unit, asked))
                .map_err(|e| PyOSError::new_err(format!("{p}: {e}")))?;
            let text = String::from_utf8(window.bytes)
                .map_err(|e| PyValueError::new_err(format!("{p}: what was read is not UTF-8: {e}")))?;
            Ok(PyString::new(py, &text).into_any().unbind())
        }
        (Some(_), Some(_)) | (None, None) => {
            Err(PyTypeError::new_err("give the input, a str or bytes, or path=, a file's path; one of them"))
        }
    }
}

/// The first `n` lines of `input`, a str or bytes, in its type; or of the
/// file at `path=`, read no further than its `n`th newline, as a str. `unit`
/// counts paragraphs or another record unit instead of lines.
#[pyfunction]
#[pyo3(signature = (n, input = None, *, path = None, unit = "line"))]
fn head(
    py: Python<'_>,
    n: usize,
    input: Option<&Bound<'_, PyAny>>,
    path: Option<&str>,
    unit: &str,
) -> PyResult<Py<PyAny>> {
    listed(py, trex::window::Select::Head(n), input, path, unit)
}

/// The last `n` lines of `input`, a str or bytes, in its type; or of the
/// file at `path=`, read backward from its end, as a str. `unit` counts
/// paragraphs or another record unit instead of lines.
#[pyfunction]
#[pyo3(signature = (n, input = None, *, path = None, unit = "line"))]
fn tail(
    py: Python<'_>,
    n: usize,
    input: Option<&Bound<'_, PyAny>>,
    path: Option<&str>,
    unit: &str,
) -> PyResult<Py<PyAny>> {
    listed(py, trex::window::Select::Tail(n), input, path, unit)
}

/// Lines `range` of `input`, a str or bytes, in its type; or of the file at
/// `path=`, as a str. `range` is `"A..B"`, `"A.."`, `"..B"` or `"A"`,
/// counted from one with both ends included; `unit` counts paragraphs or
/// another record unit instead of lines.
#[pyfunction]
#[pyo3(signature = (range, input = None, *, path = None, unit = "line"))]
fn lines(
    py: Python<'_>,
    range: &str,
    input: Option<&Bound<'_, PyAny>>,
    path: Option<&str>,
    unit: &str,
) -> PyResult<Py<PyAny>> {
    let select = trex::window::Select::parse_range(range).map_err(|e| PyValueError::new_err(format!("lines {e}")))?;
    listed(py, select, input, path, unit)
}

/// The paths `files` walks: one, or a list of them.
#[derive(FromPyObject)]
enum Paths {
    One(String),
    Many(Vec<String>),
}

/// The texture filters `files` takes, each a kind's name and whether it
/// keeps or drops a file of that kind; a name that is no kind raises
/// `ValueError`.
fn texture_filters(keep: Option<Vec<String>>, drop: Option<Vec<String>>) -> PyResult<Vec<(String, bool)>> {
    let asked: Vec<(String, bool)> = keep
        .into_iter()
        .flatten()
        .map(|n| (n, true))
        .chain(drop.into_iter().flatten().map(|n| (n, false)))
        .collect();
    if let Some((name, _)) = asked.iter().find(|(n, _)| !trex::shape::RegionKind::NAMES.contains(&n.as_str())) {
        return Err(PyValueError::new_err(format!(
            "{name:?} is no texture; the textures are {}",
            trex::shape::RegionKind::NAMES.join(", ")
        )));
    }
    Ok(asked)
}

/// The files a scan of `paths` reads, as `trex scan --files` lists them: a
/// file as itself and a directory walked with `.gitignore`, `.ignore` and
/// `.rgignore` rules applied and hidden entries skipped, `hidden=` and
/// `no_ignore=` widening the walk; a file holding a NUL byte left out unless
/// `binary=` asks for it. `globs` keep a walked file by a glob (`*.log`) or
/// drop it (`!*.min.js`), `types` and `types_not` keep or drop ripgrep's file
/// types, and `texture` and `texture_not` keep or drop every listed file by
/// what its text reads as mostly. `sort` orders the files by `"path"`,
/// `"modified"`, `"accessed"` or `"created"`, `reverse=` largest first. An
/// error the walk or a read meets raises `OSError` naming every one.
#[pyfunction]
#[pyo3(
    signature = (paths = Paths::One(".".to_string()), *, hidden = false, no_ignore = false, binary = false, globs = None, types = None, types_not = None, texture = None, texture_not = None, sort = None, reverse = false),
    text_signature = "(paths='.', *, hidden=False, no_ignore=False, binary=False, globs=None, types=None, types_not=None, texture=None, texture_not=None, sort=None, reverse=False)"
)]
// Each keyword is one of the walk's own switches, read once here.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn files(
    py: Python<'_>,
    paths: Paths,
    hidden: bool,
    no_ignore: bool,
    binary: bool,
    globs: Option<Vec<String>>,
    types: Option<Vec<String>>,
    types_not: Option<Vec<String>>,
    texture: Option<Vec<String>>,
    texture_not: Option<Vec<String>>,
    sort: Option<&str>,
    reverse: bool,
) -> PyResult<Vec<String>> {
    let roots = match paths {
        Paths::One(p) => vec![p],
        Paths::Many(ps) => ps,
    };
    let sort = match sort {
        Some(name) => Some(trex::files::Sort {
            key: trex::files::SortKey::parse(name).ok_or_else(|| {
                PyValueError::new_err(format!("sort={name:?}: sort by \"path\", \"modified\", \"accessed\" or \"created\""))
            })?,
            reverse,
        }),
        None if reverse => return Err(PyValueError::new_err("reverse= reverses the order sort= names; give sort=")),
        None => None,
    };
    // A keyword not given is no filter of that kind.
    let walk = trex::files::WalkOptions {
        hidden,
        no_ignore,
        globs: globs.unwrap_or_default(),
        types: types.unwrap_or_default(),
        types_not: types_not.unwrap_or_default(),
        sort,
    };
    walk.check().map_err(PyValueError::new_err)?;
    let asked = texture_filters(texture, texture_not)?;
    let (listed, errors) = py.detach(|| {
        let (sources, mut errors) = trex::files::collect(&roots, &walk);
        let mut listed = Vec::new();
        for src in &sources {
            if let trex::files::Source::File(path) = src {
                if !binary {
                    match trex::files::is_binary_file(path) {
                        Ok(false) => {}
                        Ok(true) => continue,
                        Err(e) => {
                            errors.push(format!("{}: {e}", src.name()));
                            continue;
                        }
                    }
                }
                if !asked.is_empty() {
                    let kind = match std::fs::read(path) {
                        Ok(raw) => trex::shape::dominant_kind(&trex::encoding::decode(raw)),
                        Err(e) => {
                            errors.push(format!("{}: {e}", src.name()));
                            continue;
                        }
                    };
                    if !trex::shape::keeps_texture(&asked, kind) {
                        continue;
                    }
                }
            }
            listed.push(src.name());
        }
        (listed, errors)
    });
    if !errors.is_empty() {
        return Err(PyOSError::new_err(errors.join("; ")));
    }
    Ok(listed)
}

/// What the text of `input`, a str or bytes, or of the file at `path=`,
/// reads as mostly: `("table", period)` with the table's period in tokens,
/// or the name of another texture with 0, as `trex scan --files --texture`
/// names it; None for text holding no region.
#[pyfunction]
#[pyo3(signature = (input = None, *, path = None))]
fn texture(py: Python<'_>, input: Option<&Bound<'_, PyAny>>, path: Option<&str>) -> PyResult<Option<(&'static str, u16)>> {
    let raw = match (input, path) {
        (Some(obj), None) => Input::of(obj)?.bytes().to_vec(),
        (None, Some(p)) => py.detach(|| std::fs::read(p)).map_err(|e| PyOSError::new_err(format!("{p}: {e}")))?,
        (Some(_), Some(_)) | (None, None) => {
            return Err(PyTypeError::new_err("give the input, a str or bytes, or path=, a file's path; one of them"));
        }
    };
    let kind = py.detach(|| trex::shape::dominant_kind(&trex::encoding::decode(raw)));
    Ok(kind.map(|k| match k {
        trex::shape::RegionKind::Table(period) => ("table", period),
        other => (other.label(), 0),
    }))
}

/// The text of the file at `path` as a scan reads it, decoded from the UTF
/// encoding its byte order mark names. Text that is not UTF-8 once decoded
/// raises `ValueError`.
#[pyfunction]
fn read(py: Python<'_>, path: &str) -> PyResult<String> {
    Ok(file_text(py, path)?.1)
}

/// The instant `now` reads in clock clauses such as `\T{age<24h}`, as
/// seconds since the epoch; None reads the wall clock at each scan.
#[pyfunction]
#[pyo3(signature = (secs))]
fn set_now(secs: Option<i64>) {
    trex::set_now(secs);
}

/// The offset, in seconds east of UTC, given to a timestamp written with no
/// zone.
#[pyfunction]
fn set_tz_offset(secs: i32) {
    trex::set_tz_offset(secs);
}

/// Which field of an all-numeric slash date is the day: `"dmy"` reads
/// `03/04/2026` as the 3rd of April, `"mdy"` as March 4th, as the command
/// line's `--date-order` does. Anything else raises `ValueError`.
#[pyfunction]
fn set_date_order(order: &str) -> PyResult<()> {
    let day_first = trex::typed::parse_date_order_arg(order)
        .ok_or_else(|| PyValueError::new_err(format!("the date order is 'dmy' or 'mdy', not {order:?}")))?;
    trex::set_date_order_day_first(day_first);
    Ok(())
}

/// How many inputs one rung of the scan ladder answered.
#[pyclass(name = "ScanRoute", module = "trex", frozen, get_all, skip_from_py_object)]
#[derive(Clone)]
struct ScanRoute {
    /// The rung, as the command line's `--stats` names it.
    rung: String,
    /// How many inputs it answered.
    inputs: u64,
}

#[pymethods]
impl ScanRoute {
    fn __repr__(&self) -> String {
        format!("ScanRoute(rung={:?}, inputs={})", self.rung, self.inputs)
    }
}

/// What the last scan on this thread read and found, as the command line's
/// `--stats` and PowerShell's `-Stats` report it. `matches` through `elapsed`
/// are the call's own; `tokens_lexed`, `lexing`, `matching`, `device_bytes`
/// and `routes` are read from trex's trace, which counts everything trex did
/// in the process during the call, so they are the call's own whenever one
/// thread scans at a time.
#[pyclass(name = "ScanStats", module = "trex", frozen, get_all, skip_from_py_object)]
#[derive(Clone, Default)]
struct ScanStats {
    /// The matches found.
    matches: u64,
    /// The lines those matches touch.
    matched_lines: u64,
    /// The inputs holding a match.
    files_with_matches: u64,
    /// The inputs scanned.
    files_searched: u64,
    /// The bytes scanned.
    bytes_searched: u64,
    /// The time the scans took.
    searching: std::time::Duration,
    /// The time from the call's start to its end.
    elapsed: std::time::Duration,
    /// The tokens the scans' lexes produced.
    tokens_lexed: u64,
    /// The time those lexes took, summed over the cores that ran them.
    lexing: std::time::Duration,
    /// `searching` less `lexing`: the time left for matching.
    matching: std::time::Duration,
    /// The bytes handed to the device.
    device_bytes: u64,
    /// How many inputs each rung of the scan ladder answered, in the order
    /// first seen.
    routes: Vec<ScanRoute>,
}

#[pymethods]
impl ScanStats {
    fn __repr__(&self) -> String {
        format!(
            "ScanStats(matches={}, matched_lines={}, files_searched={}, bytes_searched={}, tokens_lexed={})",
            self.matches, self.matched_lines, self.files_searched, self.bytes_searched, self.tokens_lexed
        )
    }
}

thread_local! {
    /// The statistics of the last scan this thread ran.
    static LAST_STATS: std::cell::RefCell<Option<ScanStats>> = const { std::cell::RefCell::new(None) };
}

/// A scan being counted by trex's own count: the trace cleared and recording
/// for the call, and the clock started, so [`StatsCount::finish`] can say what
/// the call read.
struct StatsCount(trex::stats::Counting);

impl StatsCount {
    fn start() -> Self {
        StatsCount(trex::stats::Counting::start())
    }

    /// Keep what the call read and found as this thread's last scan: each
    /// input's text, the bytes of it scanned, and its matches at the text's
    /// own offsets.
    fn finish(self, inputs: &[(&[u8], usize, &[trex::Match])]) {
        let mut stats = ScanStats::default();
        self.add_to(&mut stats, inputs);
        publish_stats(stats);
    }

    /// Add what the counted scan read and found to `stats`, as
    /// [`Self::finish`] reads it, the rungs that answered summed by rung.
    fn add_to(self, stats: &mut ScanStats, inputs: &[(&[u8], usize, &[trex::Match])]) {
        let mut counted = trex::stats::ScanStats::default();
        self.0.finish(&mut counted);
        for &(text, scanned, found) in inputs {
            counted.count(&trex::files::LineIndex::new(text), scanned, found);
        }
        stats.matches += counted.matches;
        stats.matched_lines += counted.matched_lines;
        stats.files_with_matches += counted.files_with_matches;
        stats.files_searched += counted.files_searched;
        stats.bytes_searched += counted.bytes_searched;
        stats.searching += counted.searching;
        stats.elapsed += counted.elapsed;
        stats.tokens_lexed += counted.tokens_lexed;
        stats.lexing += counted.lexing;
        stats.device_bytes += counted.device_bytes;
        stats.matching = stats.searching.saturating_sub(stats.lexing);
        for route in counted.routes {
            match stats.routes.iter_mut().find(|r| r.rung == route.rung) {
                Some(kept) => kept.inputs += route.inputs,
                None => stats.routes.push(ScanRoute { rung: route.rung, inputs: route.inputs }),
            }
        }
    }
}

/// Keep `stats` as this thread's last scan.
fn publish_stats(stats: ScanStats) {
    LAST_STATS.with(|last| *last.borrow_mut() = Some(stats));
}

/// What the last scan on this thread read and found, or None before any.
#[pyfunction]
fn scan_stats() -> Option<ScanStats> {
    LAST_STATS.with(|last| last.borrow().clone())
}

/// The text of the file at `path` a mining reads: the records `select` names,
/// counted in `unit`, or the whole file where it names none, decoded. `None`
/// where the file holds a NUL byte and `binary` does not ask for it.
fn mined_text(
    path: &std::path::Path,
    select: Option<trex::window::Select>,
    unit: &trex::records::RecordUnit,
    binary: bool,
) -> std::io::Result<Option<Vec<u8>>> {
    let Some(select) = select else {
        let raw = std::fs::read(path)?;
        if !binary && trex::files::is_binary(&raw) {
            return Ok(None);
        }
        return Ok(Some(trex::encoding::decode(raw)));
    };
    let asked = trex::window::Asked { binary, ..trex::window::Asked::default() };
    let window = trex::window::read_file(path, select, unit, asked)?;
    Ok((binary || !window.binary).then_some(window.bytes))
}

/// The text of the files `paths` name, walked under `walk` and read as one
/// stream, each cut to the records `select` names in records of `unit`, a
/// newline between two where the first does not end in one. A file holding
/// a NUL byte is read under `binary`; otherwise one named alone is refused,
/// as a scan refuses one, and the rest are passed over.
fn one_stream(
    py: Python<'_>,
    paths: PathArg,
    walk: &trex::files::WalkOptions,
    select: Option<trex::window::Select>,
    unit: &trex::records::RecordUnit,
    binary: bool,
) -> PyResult<Vec<u8>> {
    let walked = rules::walked(py, paths, walk)?;
    let mut errors = walked.errors;
    let mut out = Vec::new();
    for (name, file) in &walked.files {
        match py.detach(|| mined_text(file, select, unit, binary)) {
            Err(e) => errors.push(format!("{name}: {e}")),
            Ok(None) => {
                if walked.lone {
                    return Err(PyValueError::new_err(format!(
                        "{name} holds a NUL byte and is binary; binary=True reads it"
                    )));
                }
            }
            Ok(Some(text)) => {
                if !out.is_empty() && !out.ends_with(b"\n") {
                    out.push(b'\n');
                }
                out.extend_from_slice(&text);
            }
        }
    }
    if !errors.is_empty() {
        return Err(PyOSError::new_err(errors.join("; ")));
    }
    Ok(out)
}

/// The record templates of `text`, a str or bytes, or of the files `path=`
/// names read as one stream, most records first, as `trex templates` mines
/// them.
///
/// Each is a dict: `count` the records it covers, `records` their zero-based
/// numbers, `readable` a readable form of the record, `pattern` the trex
/// pattern that matches the group, `rare` whether it is under the cut, `novel` whether the `against` input holds no template that would
/// accept its records (`None` without `against`), and `covered` how many
/// records had a template at all, the total a share is taken of.
///
/// `cut` is written as `--cut` takes it: a whole count of records (`"5"`), or
/// a share of the records with a template (`"1%"`, `"0.25%"`). Left out, a
/// template is rare when it covers fewer records than the mean template does,
/// which is read from the input rather than chosen. `rare=True` keeps the
/// rare templates alone, and `novel=True` the novel ones.
///
/// A record is a line unless `unit` names another record unit, `record_start`
/// makes it the run from one match of a pattern to the next, or `record_span`
/// each match of one, a `Pattern` or its source read under `lib`'s
/// declarations. `against` is the other input, a str or bytes, or a path or a
/// list of them read whole as one stream. `head`, `tail` or `lines` mines that
/// part of each input alone, in records. A directory is walked as
/// `trex.files` walks one, under `hidden` and `no_ignore`, and a file holding
/// a NUL byte is read under `binary`, passed over otherwise, one named alone
/// raising `ValueError`.
#[pyfunction]
#[pyo3(signature = (text = None, cut = None, *, path = None, against = None, novel = false, rare = false, unit = "line", record_start = None, record_span = None, head = None, tail = None, lines = None, hidden = false, no_ignore = false, binary = false, lib = None))]
// Each keyword is one of the mining's own options, read once here.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn templates(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    cut: Option<&str>,
    path: Option<PathArg>,
    against: Option<&Bound<'_, PyAny>>,
    novel: bool,
    rare: bool,
    unit: &str,
    record_start: Option<&Bound<'_, PyAny>>,
    record_span: Option<&Bound<'_, PyAny>>,
    head: Option<usize>,
    tail: Option<usize>,
    lines: Option<&str>,
    hidden: bool,
    no_ignore: bool,
    binary: bool,
    lib: Option<&Bound<'_, PyAny>>,
) -> PyResult<Vec<Py<PyDict>>> {
    use trex::records::RecordUnit;
    let cut = match cut {
        Some(spec) => trex::templates::Rarity::parse(spec).map_err(|e| PyValueError::new_err(format!("cut= {e}")))?,
        None => trex::templates::Rarity::Mean,
    };
    if novel && against.is_none() {
        return Err(PyValueError::new_err("novel=True keeps what another input lacks, and against= names none"));
    }
    let shapes = shapes_of(lib)?;
    let record = match (unit, record_start, record_span) {
        (name, None, None) => RecordUnit::parse(name).map_err(|e| PyValueError::new_err(format!("unit= {e}")))?,
        ("line", Some(p), None) => RecordUnit::Start(one_pattern(p, &shapes)?),
        ("line", None, Some(p)) => RecordUnit::Span(one_pattern(p, &shapes)?),
        _ => {
            return Err(PyValueError::new_err(
                "unit=, record_start= and record_span= each say what a record is; give one",
            ));
        }
    };
    let select = select_of(head, tail, lines)?;
    let walk = trex::files::WalkOptions { hidden, no_ignore, ..trex::files::WalkOptions::default() };
    let against_text = against.is_some_and(|a| a.is_instance_of::<PyString>() || a.is_instance_of::<PyBytes>());
    if path.is_none() && (against.is_none() || against_text) && walk != trex::files::WalkOptions::default() {
        return Err(PyValueError::new_err(
            "hidden= and no_ignore= say how path= and against= walk a directory, and neither names one",
        ));
    }
    let mined: Vec<u8> = match (text, path) {
        (Some(obj), None) => {
            let input = Input::of(obj)?;
            let bytes = input.bytes();
            match select {
                Some(select) => bytes[trex::window::range_of(bytes, select, &record)].to_vec(),
                None => bytes.to_vec(),
            }
        }
        (None, Some(paths)) => one_stream(py, paths, &walk, select, &record, binary)?,
        (Some(_), Some(_)) | (None, None) => {
            return Err(PyTypeError::new_err(
                "give the text, a str or bytes, or path=, a file or directory's path or a list of them; one of them",
            ));
        }
    };
    let other: Option<Vec<u8>> = match against {
        None => None,
        Some(a) if against_text => Some(Input::of(a)?.bytes().to_vec()),
        Some(a) => {
            let paths: PathArg = a.extract().map_err(|e| {
                PyTypeError::new_err(format!("against= is a str or bytes, or a path or a list of them: {e}"))
            })?;
            Some(one_stream(py, paths, &walk, None, &record, binary)?)
        }
    };
    let mine = |bytes: &[u8]| {
        trex::templates::Mining::mine_records(&trex::lexer::lex(bytes), bytes, &record.records(bytes))
    };
    let mining = py.detach(|| mine(&mined));
    let theirs = other.as_deref().map(|bytes| py.detach(|| mine(bytes)));
    let shared: Option<Vec<bool>> = theirs.as_ref().map(|t| mining.shared_with(t));
    let covered = mining.covered();
    let mut out = Vec::with_capacity(mining.templates.len());
    for (i, t) in mining.templates.iter().enumerate() {
        let is_rare = mining.is_rare(i, cut);
        let is_novel = shared.as_ref().map(|s| !s[i]);
        if (rare && !is_rare) || (novel && is_novel != Some(true)) {
            continue;
        }
        let d = PyDict::new(py);
        d.set_item("count", t.count())?;
        d.set_item("records", t.records.clone())?;
        d.set_item("readable", t.readable())?;
        d.set_item("pattern", t.pattern())?;
        d.set_item("rare", is_rare)?;
        d.set_item("novel", is_novel)?;
        d.set_item("covered", covered)?;
        out.push(d.unbind());
    }
    Ok(out)
}

/// The pattern inferred from `examples`, as source.
///
/// `anchored` wraps it so it must match a whole input rather than a run
/// inside one. A set of examples with nothing in common is a `ValueError`
/// naming what could not be reconciled, rather than a pattern that matches
/// everything.
///
/// `against` holds examples the pattern must miss, and is what decides
/// whether a position reports a value range or its bare kind: with none, a
/// position reports its kind however well the values agree, since nothing
/// says a range around them means anything. A counter-example nothing
/// separates is a `ValueError` naming it, rather than a pattern that matches
/// what it was told to miss.
///
/// With a field given, the pattern that extracts it from every shape of the
/// examples instead, as a `Built`: `marked` holds lines with each value to
/// extract written `{name:text}`, `fields` maps a field's name to a value it
/// takes or a list of them, found anywhere in the examples, and
/// `marks_in_lines` reads marks in the examples themselves. The pattern
/// matches whole lines unless `unanchored`. A word field whose values share
/// a constant part is spelled as its byte shape, `` `KB[0-9]{7}` `` for
/// KB5031354, unless `no_mint`; `mint_shapes` declares it as a named shape
/// instead, held in the result's `declarations` and its `file`. `lib` names
/// pattern files, as `Pattern`'s does, whose shapes the examples are lexed
/// under, so a declared shape is one token the pattern names. A mark or
/// value that places no field, or a pattern that fails its verification, is
/// a `ValueError`, as is asking for both `no_mint` and `mint_shapes`, or
/// giving `lib` with no field to build for.
#[pyfunction]
#[pyo3(signature = (
    examples,
    anchored = false,
    against = Vec::new(),
    marked = Vec::new(),
    fields = None,
    marks_in_lines = false,
    unanchored = false,
    no_mint = false,
    mint_shapes = false,
    lib = None,
))]
#[allow(clippy::too_many_arguments)]
fn infer(
    py: Python<'_>,
    examples: Vec<Bound<'_, PyAny>>,
    anchored: bool,
    against: Vec<Bound<'_, PyAny>>,
    marked: Vec<String>,
    fields: Option<Bound<'_, PyDict>>,
    marks_in_lines: bool,
    unanchored: bool,
    no_mint: bool,
    mint_shapes: bool,
    lib: Option<&Bound<'_, PyAny>>,
) -> PyResult<Py<PyAny>> {
    let texts = |items: &[Bound<'_, PyAny>]| -> PyResult<Vec<Vec<u8>>> {
        items.iter().map(|item| Ok(Input::of(item)?.bytes().to_vec())).collect()
    };
    let examples = texts(&examples)?;
    let against = texts(&against)?;
    if !marked.is_empty() || fields.is_some() || marks_in_lines {
        let hints = hints_of(fields.as_ref())?;
        let mint = match (no_mint, mint_shapes) {
            (true, true) => {
                return Err(PyValueError::new_err("no_mint and mint_shapes ask for opposite spellings; give one"));
            }
            (true, false) => trex::infer::build::Mint::Off,
            (false, true) => trex::infer::build::Mint::Shapes,
            (false, false) => trex::infer::build::Mint::Inline,
        };
        let how = How { marks_in_lines, unanchored, mint };
        let shapes = shapes_of(lib)?;
        let built = build(&examples, &against, &marked, &hints, how, &shapes)?;
        return Ok(Py::new(py, built)?.into_any());
    }
    if lib.is_some() {
        return Err(PyValueError::new_err(
            "lib= lexes the examples of a pattern built for fields; give marked=, fields= or marks_in_lines=True",
        ));
    }
    let refs: Vec<&[u8]> = examples.iter().map(Vec::as_slice).collect();
    let counters: Vec<&[u8]> = against.iter().map(Vec::as_slice).collect();
    match trex::infer::infer_against(&refs, &counters) {
        Ok(i) => {
            let pattern = if anchored { i.anchored() } else { i.pattern().to_string() };
            Ok(PyString::new(py, &pattern).into_any().unbind())
        }
        Err(e) => Err(PyValueError::new_err(format!("{e:?}"))),
    }
}

/// `infer`'s `fields`: each name with the values given for it, a lone
/// value read as a list of one.
fn hints_of(fields: Option<&Bound<'_, PyDict>>) -> PyResult<Vec<(String, Vec<String>)>> {
    let mut hints = Vec::new();
    let Some(fields) = fields else { return Ok(hints) };
    for (name, values) in fields.iter() {
        let name: String = name.extract()?;
        let values: Vec<String> = if values.is_instance_of::<PyString>() {
            vec![values.extract()?]
        } else {
            values.extract().map_err(|e: PyErr| {
                PyTypeError::new_err(format!("fields[{name:?}] must be a str or a list of str: {e}"))
            })?
        };
        hints.push((name, values));
    }
    Ok(hints)
}

/// How `infer` builds a pattern beside the lines and fields it is given.
#[derive(Clone, Copy)]
struct How {
    /// Whether the examples themselves read in the markup.
    marks_in_lines: bool,
    /// Whether a match may start and end inside a longer line.
    unanchored: bool,
    /// How a shared byte shape is spelled.
    mint: trex::infer::build::Mint,
}

/// The pattern the builder makes of `examples` lexed under `shapes`, as a
/// `Built`.
fn build(
    examples: &[Vec<u8>],
    against: &[Vec<u8>],
    marked: &[String],
    hints: &[(String, Vec<String>)],
    how: How,
    shapes: &trex::ShapeSet,
) -> PyResult<Built> {
    let How { marks_in_lines, unanchored, mint } = how;
    let read = |src: &str| {
        trex::infer::marks::parse(src)
            .map_err(|e| PyValueError::new_err(format!("{src:?}: column {}: {}", e.col, e.msg)))
    };
    let mut marks: Vec<trex::infer::marks::Marked> = Vec::new();
    for template in marked {
        let lines = trex::infer::marks::parse_lines(template).map_err(|(n, e)| {
            PyValueError::new_err(format!("{template:?}: line {}: column {}: {}", n + 1, e.col, e.msg))
        })?;
        marks.extend(lines);
    }
    let mut lines: Vec<String> = Vec::with_capacity(examples.len());
    let texts: Vec<String> = examples.iter().map(|e| String::from_utf8_lossy(e).into_owned()).collect();
    let split = texts.iter().flat_map(|text| text.split('\n')).map(|l| l.strip_suffix('\r').unwrap_or(l));
    for text in split {
        let text = text.to_string();
        if !marks_in_lines {
            lines.push(text);
            continue;
        }
        let m = read(&text)?;
        if m.marks.is_empty() {
            lines.push(m.text);
        } else {
            marks.push(m);
        }
    }
    let counters: Vec<String> = against.iter().map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    let spec = trex::infer::build::Spec {
        lines: &lines,
        marked: &marks,
        hints,
        counters: &counters,
        shapes,
        unanchored,
        mint,
    };
    match trex::infer::build::build(&spec) {
        Ok(built) => Ok(Built::of(&built)),
        Err(e) => Err(PyValueError::new_err(e.to_string())),
    }
}

/// A pattern `infer` built for named fields, with the report on it. Its
/// string form is the pattern. `shape`, `lines` and a reach's shape are
/// indexes into this result's own `shapes` and `rows`.
#[pyclass(frozen, module = "trex")]
struct Built {
    /// The pattern, one branch per shape.
    #[pyo3(get)]
    pattern: String,
    /// A `--format` template writing every field, tab-separated.
    #[pyo3(get)]
    format: String,
    /// The pattern as a file `PatternSet.from_file` and `trex lib` read.
    #[pyo3(get)]
    file: String,
    /// The shapes `mint_shapes` declared, each a line of `file`, `shape kb =
    /// `KB[0-9]{7}``; the pattern reads only under them, as `lib=` a file
    /// holding them gives.
    #[pyo3(get)]
    declarations: Vec<String>,
    /// Each field every value of which a value class of the library holds,
    /// as `(field, [class, ...])`, where no counter-example called for one;
    /// an `against` line one of them refuses prints it in the pattern.
    #[pyo3(get)]
    suggestions: Vec<(String, Vec<String>)>,
    fields: Vec<trex::infer::build::Field>,
    shapes: Vec<trex::infer::build::Shape>,
    rows: Vec<trex::infer::build::Row>,
    records: Vec<trex::infer::build::Record>,
}

impl Built {
    fn of(built: &trex::infer::build::Built) -> Built {
        Built {
            pattern: built.pattern.clone(),
            format: built.format(),
            file: built.file(),
            declarations: built.declarations.clone(),
            suggestions: built.suggestions.clone(),
            fields: built.fields.clone(),
            shapes: built.shapes.clone(),
            rows: built.rows.clone(),
            records: built.records.clone(),
        }
    }

    fn values<'py>(
        &self,
        py: Python<'py>,
        values: &[Option<trex::infer::build::Value>],
        parent: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        values_dict(py, &self.fields, values, parent)
    }
}

/// `values`, one per field of `fields` in order, as a dict of the fields
/// inside `parent` (at the top where it is `None`) by their keys: a str, a
/// list of str for a list field, `None` where absent, and for a field
/// holding others a dict of its `text` and theirs.
fn values_dict<'py>(
    py: Python<'py>,
    fields: &[trex::infer::build::Field],
    values: &[Option<trex::infer::build::Value>],
    parent: Option<usize>,
) -> PyResult<Bound<'py, PyDict>> {
    use trex::infer::build::Value;

    let d = PyDict::new(py);
    for (f, field) in fields.iter().enumerate().filter(|(_, field)| field.parent == parent) {
        let holds = fields.iter().any(|g| g.parent == Some(f));
        let one = |v: &Option<Value>| -> PyResult<Py<PyAny>> {
            Ok(match v {
                Some(Value::One(one)) => PyString::new(py, one).into_any().unbind(),
                Some(Value::Many(many)) => PyList::new(py, many)?.into_any().unbind(),
                None => py.None(),
            })
        };
        match &values[f] {
            v @ Some(_) if holds => {
                let inner = values_dict(py, fields, values, Some(f))?;
                let object = PyDict::new(py);
                object.set_item("text", one(v)?)?;
                object.update(inner.as_mapping())?;
                d.set_item(field.key(), object)?;
            }
            v => d.set_item(field.key(), one(v)?)?,
        }
    }
    Ok(d)
}

/// Each of `fields` as the dict `Built.fields` gives.
fn field_dicts<'py>(py: Python<'py>, fields: &[trex::infer::build::Field]) -> PyResult<Vec<Bound<'py, PyDict>>> {
    fields
        .iter()
        .map(|f| {
            let d = PyDict::new(py);
            d.set_item("name", &f.name)?;
            d.set_item("accessor", &f.accessor)?;
            d.set_item("type", &f.type_name)?;
            d.set_item("starts_record", f.starts_record)?;
            d.set_item("list", f.list)?;
            d.set_item("repeats", f.repeats)?;
            d.set_item("parent", f.parent.map(|p| fields[p].name.as_str()))?;
            d.set_item("template", &f.template)?;
            Ok(d)
        })
        .collect()
}

#[pymethods]
impl Built {
    /// Each field as a dict: `name`, the `accessor` reading it where it is
    /// part of one token, the `type` a mark gave it, whether it
    /// `starts_record`, whether it is a `list` (a field named more than once
    /// in a line, whose value is every text it holds), whether it `repeats`
    /// with records a line repeats (every value in a row, one in each
    /// record), the name of its `parent`, the field whose mark holds its
    /// mark (its own name is then the parent's, a dot, and its key), and
    /// the `template` writing it.
    #[getter]
    fn fields<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        field_dicts(py, &self.fields)
    }

    /// Each shape as a dict: its branch `pattern`, the indexes of its
    /// `lines` in `rows`, and `reach`, each field's name mapped to
    /// `"marked"`, `("anchored", shape)` or `"missing"`.
    #[getter]
    fn shapes<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        use trex::infer::build::Reach;

        self.shapes
            .iter()
            .map(|s| {
                let d = PyDict::new(py);
                d.set_item("pattern", &s.pattern)?;
                d.set_item("lines", &s.lines)?;
                let reach = PyDict::new(py);
                for (f, r) in self.fields.iter().zip(&s.reach) {
                    match r {
                        Reach::Marked => reach.set_item(&f.name, "marked")?,
                        Reach::Anchored(from) => reach.set_item(&f.name, ("anchored", *from))?,
                        Reach::Missing => reach.set_item(&f.name, "missing")?,
                    }
                }
                d.set_item("reach", reach)?;
                Ok(d)
            })
            .collect()
    }

    /// Each line as a dict: its `text`, the index of its `shape` (`None`
    /// for a line with no token), and `values`, each field's name mapped to
    /// what the pattern reads there (a list of str for a list field) or
    /// `None`.
    #[getter]
    fn rows<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        self.rows
            .iter()
            .map(|r| {
                let d = PyDict::new(py);
                d.set_item("text", &r.text)?;
                d.set_item("shape", r.shape)?;
                d.set_item("values", self.values(py, &r.values, None)?)?;
                Ok(d)
            })
            .collect()
    }

    /// Each record as a dict: the indexes of its `lines` in `rows`, and
    /// `values`, each field's name mapped to the first value its lines hold
    /// or `None`. A line holding a `{name*:text}` field begins a record and
    /// the lines after it join it, as `ConvertFrom-String` reads a template;
    /// with no such field, each line is a record.
    #[getter]
    fn records<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        self.records
            .iter()
            .map(|r| {
                let d = PyDict::new(py);
                d.set_item("lines", &r.lines)?;
                d.set_item("values", self.values(py, &r.values, None)?)?;
                Ok(d)
            })
            .collect()
    }

    fn __str__(&self) -> String {
        self.pattern.clone()
    }

    fn __repr__(&self) -> String {
        format!("Built({:?})", self.pattern)
    }
}

/// The record spans of `text` under `unit`, as `(start, end)` in the input's
/// units.
///
/// `unit` is a record unit's name, as `--record` takes one: `line`,
/// `paragraph`, `file`, `period`, `seam`, `bind`, `bind:Q`, `auto`,
/// `texture`, `shape`, `block`, `unit` or `unit:ROLE`. Any other is a
/// `ValueError` listing the names.
#[pyfunction]
fn records(
    py: Python<'_>,
    text: &Bound<'_, PyAny>,
    unit: &str,
) -> PyResult<Vec<(usize, usize)>> {
    let input = Input::of(text)?;
    let parsed = trex::records::RecordUnit::parse(unit).map_err(PyValueError::new_err)?;
    let spans = py.detach(|| parsed.records(input.bytes()));
    let mut units = units_of(&input);
    Ok(spans.into_iter().map(|(s, e)| (units.at(s), units.at(e))).collect())
}

/// The engine's version.
#[pyfunction]
fn version() -> &'static str {
    trex::version()
}

/// Whether a CUDA device is present for the scans that can use one.
#[pyfunction]
fn device_available() -> bool {
    trex::device_available()
}

/// Where the scheduler's notes go: nowhere, since a Python program's console
/// carries only what the program prints and the module keeps no log.
fn scheduler_note(_note: &str) {}

#[pymodule(name = "trex")]
fn trex_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    trex::notice::set_sink(Some(scheduler_note));
    m.add_class::<Pattern>()?;
    m.add_class::<Match>()?;
    m.add_class::<Matches>()?;
    m.add_class::<Line>()?;
    m.add_class::<PatternSet>()?;
    m.add_class::<Group>()?;
    m.add_class::<library::Library>()?;
    m.add_class::<library::Atom>()?;
    m.add_class::<library::AtomTest>()?;
    m.add_class::<library::Rule>()?;
    m.add_class::<library::Token>()?;
    m.add_function(wrap_pyfunction!(library::tokens, m)?)?;
    m.add_class::<rules::Finding>()?;
    m.add_class::<rules::Change>()?;
    m.add_class::<rules::Follow>()?;
    m.add_function(wrap_pyfunction!(rules::sarif, m)?)?;
    m.add_function(wrap_pyfunction!(rules::findings_json, m)?)?;
    m.add("TrexWarning", m.py().get_type::<rules::TrexWarning>())?;
    m.add_class::<tools::Grammar>()?;
    m.add_class::<tools::ParseNode>()?;
    m.add_class::<tools::Tiling>()?;
    m.add_class::<tools::Bpe>()?;
    m.add_class::<tools::Index>()?;
    m.add_class::<tools::Prefilter>()?;
    m.add_class::<tools::LiteralTest>()?;
    m.add_class::<tools::FilterCheck>()?;
    // `trex.axes` is entered in `sys.modules` so `import trex.axes` finds it.
    let axes_module = PyModule::new(m.py(), "axes")?;
    axes::register(&axes_module)?;
    m.add_submodule(&axes_module)?;
    axes_module.setattr("__name__", "trex.axes")?;
    m.py().import("sys")?.getattr("modules")?.set_item("trex.axes", &axes_module)?;
    m.add_class::<StreamScanner>()?;
    m.add_class::<Built>()?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(escape, m)?)?;
    m.add_function(wrap_pyfunction!(set_now, m)?)?;
    m.add_function(wrap_pyfunction!(set_tz_offset, m)?)?;
    m.add_function(wrap_pyfunction!(set_date_order, m)?)?;
    m.add_function(wrap_pyfunction!(scan_stats, m)?)?;
    m.add_function(wrap_pyfunction!(follow, m)?)?;
    m.add_function(wrap_pyfunction!(follow_lines, m)?)?;
    m.add_function(wrap_pyfunction!(follow_rewrite, m)?)?;
    m.add_function(wrap_pyfunction!(follow_redact, m)?)?;
    m.add_function(wrap_pyfunction!(query, m)?)?;
    m.add_class::<Record>()?;
    m.add_class::<ScanStats>()?;
    m.add_class::<ScanRoute>()?;
    m.add_function(wrap_pyfunction!(templates, m)?)?;
    m.add_function(wrap_pyfunction!(head, m)?)?;
    m.add_function(wrap_pyfunction!(tail, m)?)?;
    m.add_function(wrap_pyfunction!(lines, m)?)?;
    m.add_function(wrap_pyfunction!(files, m)?)?;
    m.add_function(wrap_pyfunction!(texture, m)?)?;
    m.add_function(wrap_pyfunction!(read, m)?)?;
    m.add_function(wrap_pyfunction!(infer, m)?)?;
    m.add_function(wrap_pyfunction!(records, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(device_available, m)?)?;
    m.add("__version__", trex::version())?;
    Ok(())
}
