//! The trex library as the Python module `trex`, built into the `trex-re`
//! wheel by maturin.
//!
//! A `str` is scanned as its UTF-8 bytes and reported in characters, so
//! `s[m.start:m.end] == m.text`; `bytes` are scanned and reported as bytes.
//! Every match also carries its UTF-8 byte offsets, and results come back in
//! the input's type. Scans run with the interpreter lock released.

use std::sync::Arc;

use pyo3::exceptions::{PyKeyError, PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyString, PyTuple};

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
    /// `at` lies past it.
    fn byte_at(&self, at: usize) -> usize {
        match self {
            Input::Text(s) => s.char_indices().nth(at).map_or(s.len(), |(b, _)| b),
            Input::Bytes(b) => at.min(b.len()),
        }
    }
}

/// Byte offsets of a str read as character offsets; the identity for bytes
/// and for an ASCII str. The cursor moves through the text as offsets are
/// asked, so a scan's matches in order cost one pass over it.
struct Units<'a> {
    text: Option<&'a str>,
    byte: usize,
    unit: usize,
}

impl<'a> Units<'a> {
    fn of(input: &Input<'a>) -> Self {
        let text = match input {
            Input::Text(s) if !s.is_ascii() => Some(*s),
            Input::Text(_) | Input::Bytes(_) => None,
        };
        Units { text, byte: 0, unit: 0 }
    }

    /// The offset of byte `b` in the input's units.
    fn at(&mut self, b: usize) -> usize {
        let Some(s) = self.text else {
            return b;
        };
        if b < self.byte {
            self.unit -= chars_between(s, b, self.byte);
        } else {
            self.unit += chars_between(s, self.byte, b);
        }
        self.byte = b;
        self.unit
    }
}

/// How many characters start between byte offsets `from` and `to` of `s`.
fn chars_between(s: &str, from: usize, to: usize) -> usize {
    s.as_bytes()[from..to].iter().filter(|&&b| !(128..192).contains(&b)).count()
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

/// The declarations a `lib` argument names: one pattern file, or several.
/// Nothing where none was given, which is a pattern read as it always was.
fn shapes_of(lib: Option<&Bound<'_, PyAny>>) -> PyResult<trex::ShapeSet> {
    let mut shapes = trex::ShapeSet::new();
    let Some(lib) = lib else {
        return Ok(shapes);
    };
    let paths: Vec<String> = if lib.is_instance_of::<PyString>() {
        vec![lib.extract()?]
    } else {
        lib.extract().map_err(|e: PyErr| {
            PyTypeError::new_err(format!(
                "lib= takes a pattern file's path, or a list of them: {e}"
            ))
        })?
    };
    for path in &paths {
        shapes
            .declare_file(std::path::Path::new(path))
            .map_err(|e| PyValueError::new_err(e.msg))?;
    }
    Ok(shapes)
}

/// Each register a pattern binds, by name, with the kind of token it binds
/// where the pattern fixes one.
type CaptureKinds = Arc<[(String, Option<trex::token::TokenKind>)]>;

/// A pattern and the declarations it was read under, shared by the matches
/// of one call, which `Match.explain` scans with again.
struct Compiled {
    pattern: trex::ast::Pattern,
    shapes: trex::ShapeSet,
}

/// One match: its span in the input's units and in bytes, its text, and what
/// the pattern's registers bound.
#[pyclass(frozen, module = "trex")]
struct Match {
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
    /// callers never ask for, so `explain` takes it back instead.
    pattern: Arc<Compiled>,
    /// The match as the engine reported it, which the explainer reads.
    inner: trex::Match,
}

impl Match {
    fn of(
        py: Python<'_>,
        input: &Input<'_>,
        units: &mut Units<'_>,
        m: &trex::Match,
        bound: &Arc<[String]>,
        kinds: &CaptureKinds,
        pattern: &Arc<Compiled>,
    ) -> PyResult<Self> {
        let start = units.at(m.start);
        let end = units.at(m.end);
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
            spans.push((name.clone(), s, e, bs, be));
        }
        Ok(Match {
            start,
            end,
            byte_start: m.start,
            byte_end: m.end,
            text: input.slice(py, m.start, m.end),
            captures: captures.unbind(),
            spans,
            bound: Arc::clone(bound),
            kinds: Arc::clone(kinds),
            pattern: Arc::clone(pattern),
            inner: m.clone(),
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
    /// offsets, not about this match.
    fn explain(&self, py: Python<'_>, input: &Bound<'_, PyAny>) -> PyResult<Py<PyDict>> {
        let held = Input::of(input)?;
        let bytes = held.bytes();
        let read = &*self.pattern;
        let e = py.detach(|| {
            trex::trace::clear();
            let route = {
                // Held to the end of this block, so the rescan's rungs are
                // kept; the rescan runs for the rung it records.
                let _recording = trex::trace::Recording::start();
                let _rescanned = if read.shapes.is_empty() {
                    trex::scan_with_backend(&read.pattern, bytes, trex::Backend::Auto).0
                } else {
                    trex::scan_with_shapes(&read.pattern, bytes, &read.shapes)
                };
                trex::explain::route_of(&trex::trace::take_recorded())
            };
            trex::explain::Explainer::new(&read.pattern, bytes, &read.shapes).explain(&self.inner, &route)
        });
        let d = PyDict::new(py);
        d.set_item("tokens", e.tokens.clone())?;
        d.set_item("guards", e.guards.clone())?;
        let readings: Vec<(String, String, String)> = e
            .readings
            .iter()
            .map(|r| (r.axis.to_string(), r.text.clone(), r.value.clone()))
            .collect();
        d.set_item("readings", readings)?;
        d.set_item("route", e.route.clone())?;
        Ok(d.unbind())
    }

    /// The parsed value the register `name` bound, in its base unit, or
    /// `None` where it binds no single typed kind or its text does not parse
    /// as one.
    ///
    /// A byte size arrives in bytes, a duration in nanoseconds, a timestamp
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
        let duration = match unit {
            "ns" => trex::typed::DurationUnit::Nanoseconds,
            "ms" => trex::typed::DurationUnit::Milliseconds,
            "s" => trex::typed::DurationUnit::Seconds,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unit takes 'ns', 'ms' or 's', not {other:?}"
                )));
            }
        };
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
        Arc::new(Compiled { pattern: self.inner.clone(), shapes: self.shapes.clone() })
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
    /// line gives that one its fields, those, as the builder read them; else
    /// one field per register, in written order, a list where it is bound
    /// under a repetition.
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
    /// offsets.
    fn rewritten(
        &self,
        py: Python<'_>,
        repl: &Bound<'_, PyAny>,
        input: &Input<'_>,
        range: std::ops::Range<usize>,
        take: Option<usize>,
    ) -> PyResult<Py<PyAny>> {
        let whole = input.bytes();
        let bytes = &whole[range.clone()];
        let pat = &self.inner;
        if repl.is_instance_of::<PyString>() {
            let template: &Bound<'_, PyString> = repl.cast()?;
            let t = template_of(pat, template.to_str()?)?;
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
        // whole first, since its matches are found over its bytes alone.
        let found = if range == (0..whole.len()) {
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
        let mut units = Units::of(input);
        let mut edits = Vec::with_capacity(found.len());
        for m in found {
            let piece = repl.call1((Match::of(py, input, &mut units, m, &bound, &kinds, &shared)?,))?;
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

    /// The edits `repl` makes of a file's whole text: a template rendered at
    /// each match, or a callable handed each match.
    fn file_edits(&self, py: Python<'_>, repl: &Bound<'_, PyAny>, text: &str) -> PyResult<Vec<trex::files::Edit>> {
        let bytes = text.as_bytes();
        if repl.is_instance_of::<PyString>() {
            let template: &Bound<'_, PyString> = repl.cast()?;
            let t = template_of(&self.inner, template.to_str()?)?;
            return Ok(py.detach(|| {
                if self.shaped() {
                    trex::rewrite::edits_with_shapes(&self.inner, &t, bytes, &self.shapes)
                } else {
                    trex::rewrite::edits(&self.inner, &t, bytes)
                }
            }));
        }
        if !repl.is_callable() {
            return Err(not_a_replacement());
        }
        let input = Input::Text(text);
        let found = self.all(py, &input);
        self.called(py, repl, &input, &found)
    }

    /// The file at `path` as a rewrite reads it: its bytes and its text. A
    /// file holding a NUL byte is refused as binary, as a rewrite of one named
    /// file refuses it.
    fn rewritable(py: Python<'_>, path: &str) -> PyResult<(Vec<u8>, String)> {
        let (raw, text) = file_text(py, path)?;
        if trex::files::is_binary(&raw) {
            return Err(PyValueError::new_err(format!("{path} holds a NUL byte and is binary")));
        }
        Ok((raw, text))
    }
}

#[pymethods]
impl Pattern {
    /// Compile `source`; a pattern error is a `ValueError` naming the byte.
    ///
    /// `lib` names a pattern file, or a list of them, whose `let`, `kind`,
    /// `shape` and `shape-after` lines the pattern is read under, as
    /// `--lib` supplies them on the command line: `\{name}` resolves to
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
        // nothing the LEXER sees, which is what `is_empty` reports, but it
        // declares names the PARSER must resolve, so both are asked here.
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
    /// the `lines` its matches stand on, counted from zero, and `values`,
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
        let held = Input::of(input)?;
        let range = window_range(held.bytes(), head, tail, lines, unit)?;
        let base = range.start;
        let bytes = &held.bytes()[range];
        let by_count = match order {
            "key" => false,
            "count" => true,
            other => {
                return Err(PyValueError::new_err(format!(
                    "order takes 'key' or 'count', not {other:?}"
                )));
            }
        };
        let template = trex::Template::parse_report(key, &self.inner.capture_names())
            .map_err(|e| PyValueError::new_err(format!("key error at byte {}: {}", e.pos, e.msg)))?;
        let rows = py.detach(|| {
            let spans = self.spans(bytes);
            let matches = self.resolved(bytes, &spans);
            let mut counts: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();
            for m in &matches {
                // A key naming a place reads one from a report over files; a
                // string has no path, so the place is the match's own offset,
                // the input's own where a window of it is counted.
                let place = trex::ReportAt {
                    path: "",
                    line: 0,
                    col: 0,
                    base: Some(base),
                    pattern: None,
                    rule: None,
                };
                *counts.entry(template.render_report(m, bytes, &place)).or_insert(0) += 1;
            }
            let mut table: Vec<(String, u64)> = counts.into_iter().collect();
            if by_count {
                table.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            } else {
                table.sort_by(|a, b| a.0.cmp(&b.0));
            }
            table
        });
        Ok(rows)
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
        let mut units = Units::of(&input);
        found.first().map(|m| Match::of(py, &input, &mut units, m, &bound, &kinds, &shared)).transpose()
    }

    /// Every leftmost, non-overlapping match, as a list.
    ///
    /// `head=N` scans the first N lines alone, `tail=N` the last N, and
    /// `lines="A..B"` lines A through B, counted from one; `unit` counts
    /// paragraphs or another record unit instead of lines. A match counts
    /// only where it lies wholly inside that part, and every match is
    /// reported at the input's own offsets.
    #[pyo3(signature = (input, *, head = None, tail = None, lines = None, unit = "line"))]
    fn scan(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Vec<Match>> {
        let input = Input::of(input)?;
        let range = window_range(input.bytes(), head, tail, lines, unit)?;
        let found = self.within(py, &input, range);
        let bound = self.bound();
        let kinds = self.kinds();
        let shared = self.shared();
        let mut units = Units::of(&input);
        found.iter().map(|m| Match::of(py, &input, &mut units, m, &bound, &kinds, &shared)).collect()
    }

    /// The matches of `scan`, one at a time.
    #[pyo3(signature = (input, *, head = None, tail = None, lines = None, unit = "line"))]
    fn find_iter(
        &self,
        py: Python<'_>,
        input: &Bound<'_, PyAny>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
    ) -> PyResult<Matches> {
        Ok(Matches { items: self.scan(py, input, head, tail, lines, unit)?.into_iter() })
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
        self.find_iter(py, input, head, tail, lines, unit)
    }

    /// `input` with every match replaced by `repl`: a template rendered at
    /// each match, or a callable handed each match and returning its
    /// replacement in the input's type.
    ///
    /// Under `head`, `tail` or `lines`, that part of the input alone is
    /// rewritten and given back, as the command line prints a rewritten
    /// window alone.
    #[pyo3(signature = (repl, input, *, head = None, tail = None, lines = None, unit = "line"))]
    // The seven are the call's own arguments, the four window keywords among
    // them, each read once here.
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
    ) -> PyResult<Py<PyAny>> {
        let input = Input::of(input)?;
        let range = window_range(input.bytes(), head, tail, lines, unit)?;
        self.rewritten(py, repl, &input, range, None)
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
        self.rewritten(py, repl, &input, range, Some(n))
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
        self.rewritten(py, repl, &input, range, Some(1))
    }

    /// `input` with every match masked, as `trex redact` masks one: each
    /// character of a match by `mask`, a character, or a token per masked
    /// run as the command line's `--mask` takes one; the fields `keep` names
    /// (`"card:last4, ip:octet1-2"`) left where they stand. Under `head`,
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

    /// The unified diff a rewrite of the file at `path` by `repl` would make,
    /// as `trex rewrite --dry-run` prints it, with `context` lines around each
    /// change; empty where nothing matches. `repl` is a template, or a
    /// callable taking a `Match` and returning a str. A binary file raises
    /// `ValueError`.
    #[pyo3(signature = (repl, path, *, context = 3))]
    fn diff(&self, py: Python<'_>, repl: &Bound<'_, PyAny>, path: &str, context: usize) -> PyResult<String> {
        let (_, text) = Self::rewritable(py, path)?;
        let edits = self.file_edits(py, repl, &text)?;
        Ok(trex::files::unified_diff(path, text.as_bytes(), &edits, context))
    }

    /// Rewrite the file at `path` in place by `repl`, as `trex rewrite
    /// --in-place` writes one: each replacement in the encoding the file is
    /// read in, behind its own byte order mark, every byte outside a match
    /// as it was. Returns how many replacements were made; a file with none
    /// is not written. A binary file raises `ValueError`.
    fn rewrite_file(&self, py: Python<'_>, repl: &Bound<'_, PyAny>, path: &str) -> PyResult<usize> {
        let (raw, text) = Self::rewritable(py, path)?;
        let edits = self.file_edits(py, repl, &text)?;
        if edits.is_empty() {
            return Ok(0);
        }
        let out = trex::files::written(&raw, &edits, trex::files::ReadAs::Decoded);
        py.detach(|| std::fs::write(path, out))
            .map_err(|e| PyOSError::new_err(format!("{path}: cannot write it: {e}")))?;
        Ok(edits.len())
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
            .map(|p| Arc::new(Compiled { pattern: p.clone(), shapes: inner.shapes().clone() }))
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
        let mut units = Units::of(input);
        found
            .into_iter()
            .map(|(i, m)| Ok((i, Match::of(py, input, &mut units, &m, &self.bounds[i], &self.kinds[i], &self.shared[i])?)))
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
/// in its type; or of the file at `path`, read from the end the part sits
/// at and no further, as `trex head` and `trex tail` read one, as a str.
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

/// The zone a timestamp written with no zone is read in, as seconds east
/// of UTC.
#[pyfunction]
fn set_tz_offset(secs: i32) {
    trex::set_tz_offset(secs);
}

/// The line templates `text` holds, most lines first.
///
/// Each is a dict: `count` the lines it covers, `records` their zero-based
/// numbers, `readable` the spelling that reads as the line did, `pattern` the
/// trex pattern that matches the group, `rare` whether it falls under the
/// cut, and `covered` how many lines had a template at all.
///
/// `rare_under` is the cut, written as `--cut` takes it: a whole count of
/// lines (`"5"`), or a share of the lines with a template (`"1%"`,
/// `"0.25%"`). Left out, a template is rare when it covers fewer lines than
/// the mean template does, which is read from the input rather than chosen.
///
/// A line with no significant token belongs to no template, so the counts
/// need not sum to the lines, and `covered` is what they do sum to.
///
/// `head`, `tail` or `lines` mines that part of the text alone.
#[pyfunction]
#[pyo3(signature = (text, rare_under = None, *, head = None, tail = None, lines = None))]
fn templates(
    py: Python<'_>,
    text: &Bound<'_, PyAny>,
    rare_under: Option<&str>,
    head: Option<usize>,
    tail: Option<usize>,
    lines: Option<&str>,
) -> PyResult<Vec<Py<PyDict>>> {
    let input = Input::of(text)?;
    let cut = match rare_under {
        Some(spec) => trex::templates::Rarity::parse(spec).map_err(PyValueError::new_err)?,
        None => trex::templates::Rarity::Mean,
    };
    let range = window_range(input.bytes(), head, tail, lines, "line")?;
    let bytes = &input.bytes()[range];
    let mining = py.detach(|| trex::templates::Mining::mine(bytes));
    let covered = mining.covered();
    let mut out = Vec::with_capacity(mining.templates.len());
    for (i, t) in mining.templates.iter().enumerate() {
        let d = PyDict::new(py);
        d.set_item("count", t.count())?;
        d.set_item("records", t.records.clone())?;
        d.set_item("readable", t.readable())?;
        d.set_item("pattern", t.pattern())?;
        d.set_item("rare", mining.is_rare(i, cut))?;
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
/// takes or a list of them, found wherever they stand in the examples, and
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
    let mut units = Units::of(&input);
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
    m.add_class::<PatternSet>()?;
    m.add_class::<StreamScanner>()?;
    m.add_class::<Built>()?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(escape, m)?)?;
    m.add_function(wrap_pyfunction!(set_now, m)?)?;
    m.add_function(wrap_pyfunction!(set_tz_offset, m)?)?;
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
