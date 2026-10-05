//! The rules a library declares, scanned as findings over text or files,
//! each finding placed and rendered as the `trex` command's `scan --rules`
//! reports it; the fixes written into the files, described by the diff they
//! would make, or put to a review first; and files followed as they grow.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pyo3::exceptions::{PyOSError, PyTypeError, PyUserWarning, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyString};
use trex::files::LineIndex;
use trex::rule_scan::{Found, Place, RuleScan, SarifResult, SarifRule, SharedRuleStream};

use crate::library::{Library, Rule, repr_of, severity_name};
use trex::encoding::{OffsetUnit, Offsets};

use crate::{Input, PathArg, chars_of, select_of, unit_of, units_of};

pyo3::create_exception!(
    trex,
    TrexWarning,
    PyUserWarning,
    "A notice beside a result: a fix skipped for overlapping an earlier one, the changes a template answer passed over, a review that quit, or a followed file that was truncated, replaced or removed."
);

/// Issue `message` as a `TrexWarning`.
pub(crate) fn warn(py: Python<'_>, message: &str) -> PyResult<()> {
    let message = std::ffi::CString::new(message).map_err(|e| PyValueError::new_err(e.to_string()))?;
    PyErr::warn(py, py.get_type::<TrexWarning>().as_any(), &message, 1)
}

/// The rules `lib` declares, prepared to scan.
fn scan_of(lib: &Library) -> PyResult<Arc<RuleScan>> {
    let shapes = lib.shapes();
    if shapes.rules().is_empty() {
        return Err(PyValueError::new_err(
            "no rule is declared here: declare a rule line, or load a pattern file holding one",
        ));
    }
    RuleScan::new(shapes).map(Arc::new).map_err(PyValueError::new_err)
}

/// Each rule of `scan` as a `Rule`, in rule order, which its findings share.
fn definitions(py: Python<'_>, scan: &RuleScan) -> PyResult<Vec<Py<Rule>>> {
    scan.rules().iter().map(|r| Py::new(py, Rule::of(py, r)?)).collect()
}

/// How a rule call reads its inputs: the part of each it reads, how files
/// are walked, and whether a file holding a NUL byte is read.
pub(crate) struct Reading {
    pub(crate) select: Option<trex::window::Select>,
    pub(crate) unit: trex::records::RecordUnit,
    pub(crate) walk: trex::files::WalkOptions,
    pub(crate) binary: bool,
}

impl Reading {
    /// From the keywords every rule call takes: `head`, `tail` or `lines`
    /// naming the part read in records of `unit`, the walk's switches, and
    /// `binary`.
    // The eight are the calls' own keywords, each read once here.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    pub(crate) fn of(
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        hidden: bool,
        no_ignore: bool,
        globs: Option<Vec<String>>,
        binary: bool,
    ) -> PyResult<Reading> {
        let select = select_of(head, tail, lines)?;
        if select.is_none() && unit != "line" {
            return Err(PyValueError::new_err(format!(
                "unit={unit:?} names what head=, tail= and lines= count, and none was given"
            )));
        }
        let walk = trex::files::WalkOptions {
            hidden,
            no_ignore,
            globs: globs.unwrap_or_default(),
            ..trex::files::WalkOptions::default()
        };
        walk.check().map_err(PyValueError::new_err)?;
        Ok(Reading { select, unit: unit_of(unit)?, walk, binary })
    }

    /// Whether a walk's switches were given, which only `path=` reads.
    pub(crate) fn walks(&self) -> bool {
        self.walk != trex::files::WalkOptions::default()
    }
}

/// What a rule call reads under `path=`: each file the paths name, walked
/// as `trex.files` walks them, by the name a finding reports, the errors the
/// walk met, and whether one file was named alone, which a NUL byte refuses
/// aloud rather than passing over.
pub(crate) struct Walked {
    pub(crate) files: Vec<(String, PathBuf)>,
    pub(crate) errors: Vec<String>,
    pub(crate) lone: bool,
}

pub(crate) fn walked(py: Python<'_>, paths: PathArg, walk: &trex::files::WalkOptions) -> PyResult<Walked> {
    let roots: Vec<String> = match paths {
        PathArg::One(p) => vec![p.to_string_lossy().into_owned()],
        PathArg::Many(ps) => ps.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
    };
    let (sources, errors) = py.detach(|| trex::files::collect(&roots, walk));
    let lone = sources.len() == 1 && !roots.iter().any(|r| Path::new(r).is_dir());
    let mut files = Vec::with_capacity(sources.len());
    for src in sources {
        let name = src.name();
        match src {
            trex::files::Source::File(path) => files.push((name, path)),
            trex::files::Source::Stdin => {
                return Err(PyValueError::new_err(format!(
                    "{name} names the standard input; give what it holds as the text"
                )));
            }
        }
    }
    Ok(Walked { files, errors, lone })
}

/// The part of an input a rule call read: its text, its offset in the
/// input's text, the lines and the input's units ahead of it and the unit
/// they count, how long the input was and the encoding it opened with, which
/// a follow picks up from, and whether it holds a NUL byte.
pub(crate) struct Part {
    pub(crate) text: Vec<u8>,
    pub(crate) byte_base: usize,
    pub(crate) line_base: Option<usize>,
    pub(crate) units_ahead: usize,
    unit: OffsetUnit,
    pub(crate) input_len: usize,
    pub(crate) encoding: trex::encoding::Encoding,
    pub(crate) binary: bool,
}

/// What a window that a reader handed back unplaced is refused as; every
/// read here asks for the window's place.
fn unplaced() -> std::io::Error {
    std::io::Error::other("the part read was not placed in the input's text")
}

impl Part {
    /// The part of `input`, text held in memory, that `reading` names.
    pub(crate) fn of_text(input: &Input<'_>, reading: &Reading) -> PyResult<Part> {
        let bytes = input.bytes();
        let Some(select) = reading.select else {
            return Ok(Part {
                text: bytes.to_vec(),
                byte_base: 0,
                line_base: Some(0),
                units_ahead: 0,
                unit: input.unit(),
                input_len: bytes.len(),
                encoding: trex::encoding::Encoding::Utf8,
                binary: false,
            });
        };
        let w = trex::window::window_of(bytes, select, &reading.unit);
        let byte_base = w.byte_base.ok_or_else(|| PyValueError::new_err(unplaced().to_string()))?;
        Ok(Part {
            units_ahead: units_of(input).at(byte_base),
            unit: input.unit(),
            text: w.bytes,
            byte_base,
            line_base: w.line_base,
            input_len: bytes.len(),
            encoding: trex::encoding::Encoding::Utf8,
            binary: false,
        })
    }

    /// The part of the file at `path` that `reading` names, the characters
    /// ahead of it counted; the whole file decoded where it names none.
    pub(crate) fn of_file(path: &Path, reading: &Reading) -> std::io::Result<Part> {
        if let Some(select) = reading.select {
            let asked = trex::window::Asked { numbers: true, offsets: true, binary: reading.binary, units: false };
            let (w, chars) = trex::window::read_file_with_chars(path, select, &reading.unit, asked)?;
            // A window handed back binary and unread is passed over unread,
            // so nothing ahead of it was counted or is asked for.
            let (byte_base, units_ahead) = match (w.byte_base, chars) {
                (Some(base), Some(chars)) => (base, chars),
                _ if w.binary && !reading.binary => (0, 0),
                _ => return Err(unplaced()),
            };
            return Ok(Part {
                units_ahead,
                unit: OffsetUnit::CodePoints,
                text: w.bytes,
                byte_base,
                line_base: w.line_base,
                input_len: w.input_len,
                encoding: w.encoding,
                binary: w.binary,
            });
        }
        let raw = std::fs::read(path)?;
        let binary = trex::files::is_binary(&raw);
        let input_len = raw.len();
        let encoding = trex::encoding::Encoding::declared(&raw).0;
        let text = if binary && !reading.binary { Vec::new() } else { trex::encoding::decode(raw) };
        Ok(Part {
            text,
            byte_base: 0,
            line_base: Some(0),
            units_ahead: 0,
            unit: OffsetUnit::CodePoints,
            input_len,
            encoding,
            binary,
        })
    }

    /// The index a finding over the part is placed by: its lines and bytes
    /// in the input, and the offsets a rule's message writes in the input's
    /// units.
    fn index(&self) -> LineIndex {
        LineIndex::new(&self.text).within(Some(self.byte_base), self.line_base).counting(
            &self.text,
            self.unit,
            Some(self.units_ahead),
        )
    }
}

/// What the findings of one input share: the scan whose rules made them,
/// the input's name, the part of it read, that part's offset in the input's
/// text, its line index, and whether the input was bytes, which their texts
/// come back as.
struct Scanned {
    scan: Arc<RuleScan>,
    name: Option<String>,
    text: Vec<u8>,
    byte_base: usize,
    index: LineIndex,
    as_bytes: bool,
}

impl Scanned {
    /// The bytes at `start..end` of the part read, in the input's type.
    fn slice(&self, py: Python<'_>, start: usize, end: usize) -> Py<PyAny> {
        let bytes = &self.text[start..end];
        if self.as_bytes {
            PyBytes::new(py, bytes).into_any().unbind()
        } else {
            PyString::new(py, &String::from_utf8_lossy(bytes)).into_any().unbind()
        }
    }

    /// `text`, which a rule rendered, in the input's type.
    fn rendered(&self, py: Python<'_>, text: &str) -> Py<PyAny> {
        if self.as_bytes {
            PyBytes::new(py, text.as_bytes()).into_any().unbind()
        } else {
            PyString::new(py, text).into_any().unbind()
        }
    }

    /// Byte offsets of the part read in the input's units, from its start.
    fn units(&self) -> Offsets<'_> {
        Offsets::new(&self.text, if self.as_bytes { OffsetUnit::Bytes } else { OffsetUnit::CodePoints })
    }
}

/// One finding of a rule: the rule and what it says, the finding's
/// position, its text, and the fix where the rule has one.
#[pyclass(frozen, module = "trex")]
pub(crate) struct Finding {
    /// The rule's name.
    #[pyo3(get)]
    rule: String,
    /// How serious it is: `"error"`, `"warning"` or `"note"`.
    #[pyo3(get)]
    severity: &'static str,
    /// What it says, rendered from the match.
    #[pyo3(get)]
    message: String,
    /// The file it is in; `None` for text.
    #[pyo3(get)]
    path: Option<String>,
    /// The line it starts on, from 1.
    #[pyo3(get)]
    line: usize,
    /// The character it starts at within its line, from 1.
    #[pyo3(get)]
    column: usize,
    /// The line its end is on.
    #[pyo3(get)]
    end_line: usize,
    /// The character just past its end, within that line.
    #[pyo3(get)]
    end_column: usize,
    /// Where it starts, in the input's units: characters of a str or a
    /// file's text, bytes of bytes.
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
    /// Its text, in the input's type: the match, or the record for a rule
    /// that fires on records.
    #[pyo3(get)]
    text: Py<PyAny>,
    /// What a fix puts in the match's place, in the input's type; `None`
    /// where the rule has no fix.
    #[pyo3(get)]
    fix: Py<PyAny>,
    /// The span a fix replaces, the match, in the input's units; `None`
    /// where the rule has no fix.
    #[pyo3(get)]
    fix_span: Option<(usize, usize)>,
    /// Each register the rule's pattern bound, by name, to its text in the
    /// input's type.
    #[pyo3(get)]
    captures: Py<PyDict>,
    /// The rule itself.
    #[pyo3(get)]
    definition: Py<Rule>,
    /// The span of the match the message and the fix read, in the input's
    /// units, which a report template's `${start}` and `${end}` write.
    match_span: (usize, usize),
    scanned: Arc<Scanned>,
    inner: trex::rule_scan::Finding,
}

/// The findings `found` of what `scanned` holds, as Python objects sharing
/// it, `defs` each rule's definition and `units_ahead` the input's units
/// before the part read.
fn findings_of(
    py: Python<'_>,
    scanned: &Arc<Scanned>,
    defs: &[Py<Rule>],
    units_ahead: usize,
    found: Vec<trex::rule_scan::Finding>,
) -> PyResult<Vec<Finding>> {
    let s = scanned;
    let mut units = s.units();
    let byte_base = s.byte_base;
    let mut out = Vec::with_capacity(found.len());
    for f in found {
        let rule = &s.scan.rules()[f.rule];
        let place = Place::of(&s.index, &s.text, f.start, f.end);
        let start = units_ahead + units.at(f.start);
        let end = units_ahead + units.at(f.end);
        let match_span = (units_ahead + units.at(f.m.start), units_ahead + units.at(f.m.end));
        let fix_span = f.fix.is_some().then_some(match_span);
        let captures = PyDict::new(py);
        for (name, span) in f.m.names().iter().zip(f.m.captures()) {
            captures.set_item(name, s.slice(py, span.start(), span.end()))?;
        }
        let fix = match &f.fix {
            Some(text) => s.rendered(py, text),
            None => py.None(),
        };
        out.push(Finding {
            rule: rule.name.clone(),
            severity: severity_name(rule.severity),
            message: f.message.clone(),
            path: s.name.clone(),
            line: place.line,
            column: place.col,
            end_line: place.end_line,
            end_column: place.end_col,
            start,
            end,
            byte_start: byte_base + f.start,
            byte_end: byte_base + f.end,
            text: s.slice(py, f.start, f.end),
            fix,
            fix_span,
            captures: captures.unbind(),
            definition: defs[f.rule].clone_ref(py),
            match_span,
            scanned: Arc::clone(s),
            inner: f,
        });
    }
    Ok(out)
}

#[pymethods]
impl Finding {
    /// The finding as a GitHub workflow annotation, as `scan --rules
    /// --github` writes one: `::error file=...,line=...::message`.
    fn github(&self) -> String {
        let s = &self.scanned;
        s.scan.github_line(s.name.as_deref(), &s.text, &s.index, &self.inner)
    }

    /// `template`, a report template, rendered at the finding as `scan
    /// --rules --format` renders one: the registers by name, and `${rule}`,
    /// `${severity}`, `${message}`, `${fix}`, `${path}`, `${line}` and
    /// `${col}`, and `${start}` and `${end}`, its match's position, in the
    /// input's units. A template that does not parse raises `ValueError`.
    fn format(&self, template: &str) -> PyResult<String> {
        let s = &self.scanned;
        let t = trex::Template::parse_report(template, &s.scan.capture_names())
            .map_err(|e| PyValueError::new_err(format!("template error at byte {}: {}", e.pos, e.msg)))?;
        let rule = &s.scan.rules()[self.inner.rule];
        let place = Place::of(&s.index, &s.text, self.inner.start, self.inner.end);
        let at = trex::ReportAt {
            path: s.name.as_deref().unwrap_or(""),
            line: place.line,
            col: place.col,
            offsets: Some(self.match_span),
            pattern: Some(&rule.name),
            rule: Some(trex::ReportRule {
                name: &rule.name,
                severity: rule.severity.name(),
                message: &self.inner.message,
                fix: self.inner.fix.as_deref().unwrap_or(""),
            }),
        };
        Ok(t.render_report(&self.inner.m, &s.text, &at))
    }

    /// The finding as a plain dict of its fields, the definition and the
    /// captures as dicts of their own.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("rule", &self.rule)?;
        d.set_item("severity", self.severity)?;
        d.set_item("message", &self.message)?;
        d.set_item("path", &self.path)?;
        d.set_item("line", self.line)?;
        d.set_item("column", self.column)?;
        d.set_item("end_line", self.end_line)?;
        d.set_item("end_column", self.end_column)?;
        d.set_item("start", self.start)?;
        d.set_item("end", self.end)?;
        d.set_item("byte_start", self.byte_start)?;
        d.set_item("byte_end", self.byte_end)?;
        d.set_item("text", self.text.bind(py))?;
        d.set_item("fix", self.fix.bind(py))?;
        d.set_item("fix_span", self.fix_span)?;
        d.set_item("captures", self.captures.bind(py).copy()?)?;
        d.set_item("definition", self.definition.bind(py).call_method0("to_dict")?)?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let path = match &self.path {
            Some(p) => repr_of(py, p)?,
            None => "None".to_string(),
        };
        Ok(format!(
            "Finding(rule={}, severity={}, path={path}, line={}, column={})",
            repr_of(py, &self.rule)?,
            repr_of(py, self.severity)?,
            self.line,
            self.column
        ))
    }
}

/// The findings an iterable holds, refusing anything that is not one.
fn finding_list<'py>(findings: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, Finding>>> {
    findings
        .try_iter()?
        .map(|item| {
            item?
                .cast_into::<Finding>()
                .map_err(|e| PyTypeError::new_err(format!("findings are trex.Finding objects: {e}")))
        })
        .collect()
}

/// Every finding of `findings` as one JSON array, each object as `scan
/// --rules --json` writes it: the rule, its severity and message, the path,
/// the finding's position, its text and captures, its fix and the rule's
/// metadata.
#[pyfunction]
pub(crate) fn findings_json(findings: &Bound<'_, PyAny>) -> PyResult<String> {
    let objects: Vec<String> = finding_list(findings)?
        .iter()
        .map(|f| {
            let f = f.get();
            let s = &f.scanned;
            s.scan.finding_json(s.name.as_deref(), &s.text, &s.index, &f.inner)
        })
        .collect();
    Ok(format!("[{}]", objects.join(",")))
}

/// Every finding of `findings` as one SARIF 2.1.0 document, as `scan
/// --rules --sarif` writes one. The document lists the rules of `lib`, so a
/// run with no finding still describes every rule it checked; without it,
/// the rules of the libraries the findings came from.
#[pyfunction]
#[pyo3(signature = (findings, *, lib = None))]
pub(crate) fn sarif(findings: &Bound<'_, PyAny>, lib: Option<PyRef<'_, Library>>) -> PyResult<String> {
    let findings = finding_list(findings)?;
    let mut rules: Vec<SarifRule> = Vec::new();
    let place = |rules: &mut Vec<SarifRule>, rule: &trex::Rule| match rules.iter().position(|r| r.name == rule.name) {
        Some(k) => k,
        None => {
            rules.push(SarifRule::of(rule));
            rules.len() - 1
        }
    };
    match &lib {
        Some(lib) => {
            let shapes = lib.shapes();
            for rule in shapes.rules() {
                place(&mut rules, rule);
            }
        }
        None => {
            let mut seen: Vec<&Arc<RuleScan>> = Vec::new();
            for f in &findings {
                let scan = &f.get().scanned.scan;
                if !seen.iter().any(|s| Arc::ptr_eq(s, scan)) {
                    seen.push(scan);
                    for rule in scan.rules() {
                        place(&mut rules, rule);
                    }
                }
            }
        }
    }
    let mut results = Vec::with_capacity(findings.len());
    for f in &findings {
        let f = f.get();
        let s = &f.scanned;
        let rule = place(&mut rules, &s.scan.rules()[f.inner.rule]);
        results.push(SarifResult {
            rule,
            uri: s.name.as_deref().map(|n| n.replace('\\', "/")),
            at: Place::of(&s.index, &s.text, f.inner.start, f.inner.end),
            snippet: String::from_utf8_lossy(&s.text[f.inner.start..f.inner.end]).into_owned(),
            message: f.inner.message.clone(),
            fix: f.inner.fix.clone().map(|fix| (fix, Place::of(&s.index, &s.text, f.inner.m.start, f.inner.m.end))),
        });
    }
    Ok(trex::rule_scan::sarif(&rules, &results))
}

/// The findings of `lib`'s rules over `text`, or over the files `path`
/// names, the first `max_count` of each input where it is given.
pub(crate) fn check(
    py: Python<'_>,
    lib: &Library,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathArg>,
    max_count: Option<usize>,
    reading: &Reading,
) -> PyResult<Vec<Finding>> {
    let scan = scan_of(lib)?;
    let defs = definitions(py, &scan)?;
    match (text, path) {
        (Some(obj), None) => {
            if reading.walks() {
                return Err(PyValueError::new_err(
                    "hidden=, no_ignore= and globs= say how path= walks a directory, and the text is no directory",
                ));
            }
            let input = Input::of(obj)?;
            let part = Part::of_text(&input, reading)?;
            findings_in(py, &scan, &defs, None, part, max_count, matches!(input, Input::Bytes(_)))
        }
        (None, Some(paths)) => {
            let walked = walked(py, paths, &reading.walk)?;
            let mut errors = walked.errors;
            let mut out = Vec::new();
            for (name, file) in &walked.files {
                match py.detach(|| Part::of_file(file, reading)) {
                    Err(e) => errors.push(format!("{name}: {e}")),
                    // As a scan does: a file named alone that holds a NUL
                    // byte is refused aloud, and one among several or found
                    // by a walk is passed over.
                    Ok(part) if part.binary && !reading.binary => {
                        if walked.lone {
                            return Err(PyValueError::new_err(format!(
                                "{name} holds a NUL byte and is binary; binary=True reads it"
                            )));
                        }
                    }
                    Ok(part) => out.extend(findings_in(py, &scan, &defs, Some(name.clone()), part, max_count, false)?),
                }
            }
            if !errors.is_empty() {
                return Err(PyOSError::new_err(errors.join("; ")));
            }
            Ok(out)
        }
        (Some(_), Some(_)) | (None, None) => Err(PyTypeError::new_err(
            "give the text, a str or bytes, or path=, a file or directory's path or a list of them; one of them",
        )),
    }
}

/// Every finding of `scan` over `part`, the input named `name`, as Python
/// objects; `as_bytes` where the input was bytes.
fn findings_in(
    py: Python<'_>,
    scan: &Arc<RuleScan>,
    defs: &[Py<Rule>],
    name: Option<String>,
    part: Part,
    max_count: Option<usize>,
    as_bytes: bool,
) -> PyResult<Vec<Finding>> {
    let index = part.index();
    let found = py.detach(|| scan.findings(name.as_deref(), &part.text, &index, max_count));
    let scanned =
        Arc::new(Scanned { scan: Arc::clone(scan), name, text: part.text, byte_base: part.byte_base, index, as_bytes });
    findings_of(py, &scanned, defs, part.units_ahead, found)
}

/// One change put to a review, a fix, a rewrite or a masking: the file and
/// the span it replaces, the text there and what replaces it, its unified
/// diff, its template, how many later changes share that template, which an
/// answer for the template decides with it, its place in the review,
/// and where asked a scan's explanation of its match.
#[pyclass(frozen, module = "trex")]
pub(crate) struct Change {
    /// The file the fix is in.
    #[pyo3(get)]
    path: String,
    /// Where the span it replaces starts, in characters of the file's text.
    #[pyo3(get)]
    start: usize,
    /// Where that span ends, exclusive.
    #[pyo3(get)]
    end: usize,
    /// The text the fix replaces.
    #[pyo3(get)]
    text: String,
    /// What the fix puts in its place.
    #[pyo3(get)]
    replacement: String,
    /// The unified diff of this fix alone, as a review shows it.
    #[pyo3(get)]
    diff: String,
    /// The fix's template: the kinds of the tokens it replaces.
    #[pyo3(get)]
    template: String,
    /// How many fixes after this one share its template.
    #[pyo3(get)]
    later: usize,
    /// Which fix this is, from 1.
    #[pyo3(get)]
    index: usize,
    /// How many fixes the review holds.
    #[pyo3(get)]
    total: usize,
    /// Why the change's match matched, as `Match.explain` gives it, where the
    /// review was asked to explain its changes; `None` otherwise.
    #[pyo3(get)]
    explanation: Option<Py<PyDict>>,
}

#[pymethods]
impl Change {
    /// The change as a plain dict of its fields.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("path", &self.path)?;
        d.set_item("start", self.start)?;
        d.set_item("end", self.end)?;
        d.set_item("text", &self.text)?;
        d.set_item("replacement", &self.replacement)?;
        d.set_item("diff", &self.diff)?;
        d.set_item("template", &self.template)?;
        d.set_item("later", self.later)?;
        d.set_item("index", self.index)?;
        d.set_item("total", self.total)?;
        d.set_item("explanation", self.explanation.as_ref().map(|e| e.clone_ref(py)))?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Change(path={}, start={}, end={}, text={}, replacement={}, index={}, total={})",
            repr_of(py, &self.path)?,
            self.start,
            self.end,
            repr_of(py, &self.text)?,
            repr_of(py, &self.replacement)?,
            self.index,
            self.total
        ))
    }
}

/// What a review callable's return value answers: `True` applies the fix,
/// `False` skips it, `"template"` and `"skip template"` apply or skip it and
/// every later fix of its template, `"all"` applies it and every fix after
/// it, `"quit"` keeps no fix, and any other str is put in its place instead.
fn answer_of(value: &Bound<'_, PyAny>) -> PyResult<trex::review::Answer> {
    use trex::review::Answer;
    if value.is_instance_of::<PyBool>() {
        return Ok(if value.extract::<bool>()? { Answer::Accept } else { Answer::Skip });
    }
    if value.is_instance_of::<PyString>() {
        let answer: String = value.extract()?;
        return Ok(match answer.as_str() {
            "template" => Answer::AcceptTemplate,
            "skip template" => Answer::SkipTemplate,
            "all" => Answer::AcceptAll,
            "quit" => Answer::Quit,
            _ => Answer::Replace(answer.into_bytes()),
        });
    }
    Err(PyTypeError::new_err(format!(
        "review= returns True, False, a replacement str, 'template', 'skip template', 'all' or 'quit', not {}",
        value.get_type().name()?
    )))
}

/// One file's changes waiting for a review: its name and path, its bytes,
/// which the accepted ones are written into, its text, and the changes.
pub(crate) struct Pending {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) raw: Vec<u8>,
    pub(crate) text: Vec<u8>,
    pub(crate) edits: Vec<trex::files::Edit>,
}

/// The explanation a review hands with each change where asked: from the
/// text the change is in and the span of its match, `Match.explain`'s dict.
pub(crate) type ExplainChange<'a> = dyn Fn(Python<'_>, &[u8], usize, usize) -> PyResult<Py<PyDict>> + 'a;

/// What an edit of files was asked beyond the files: the diff in place of
/// writing, the lines of context around each change in it, the callable each
/// change is put to first, whether the places a template answer skipped are
/// told, and what explains each change to the callable.
pub(crate) struct Fixing<'a, 'py> {
    pub(crate) dry_run: bool,
    pub(crate) context: usize,
    pub(crate) review: Option<&'a Bound<'py, PyAny>>,
    pub(crate) show_skipped: bool,
    pub(crate) explain: Option<&'a ExplainChange<'a>>,
}

/// The fixes of `lib`'s rules written into the files `paths` names, as
/// `scan --rules --fix` writes them: how many were written, or with
/// `dry_run` the unified diff they would make.
pub(crate) fn fix(
    py: Python<'_>,
    lib: &Library,
    paths: PathArg,
    fixing: &Fixing<'_, '_>,
    reading: &Reading,
) -> PyResult<Py<PyAny>> {
    if fixing.review.is_some() && fixing.dry_run {
        return Err(PyValueError::new_err(
            "review= puts each fix to the callable before it is written, and takes no dry_run=True",
        ));
    }
    if fixing.show_skipped && fixing.review.is_none() {
        return Err(PyValueError::new_err(
            "show_skipped= lists the fixes a template answer skipped, which only review= gives",
        ));
    }
    if let Some(review) = fixing.review
        && !review.is_callable()
    {
        return Err(PyTypeError::new_err("review= takes a callable, handed each fix as a trex.Change"));
    }
    let scan = scan_of(lib)?;
    let walked = walked(py, paths, &reading.walk)?;
    let mut errors = walked.errors;
    let mut diffs = String::new();
    let mut queue: Vec<Pending> = Vec::new();
    let mut written = 0usize;
    for (name, file) in &walked.files {
        let raw = match py.detach(|| std::fs::read(file)) {
            Ok(raw) => raw,
            Err(e) => {
                errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        if !reading.binary && trex::files::is_binary(&raw) {
            if walked.lone {
                return Err(PyValueError::new_err(format!(
                    "{name} holds a NUL byte and is binary; binary=True fixes it"
                )));
            }
            continue;
        }
        let text = trex::encoding::text_of(&raw).into_owned();
        let range = match reading.select {
            Some(select) => trex::window::range_of(&text, select, &reading.unit),
            None => 0..text.len(),
        };
        let lines_before = trex::byte_simd::count_byte(&text[..range.start], b'\n');
        let piece = &text[range.clone()];
        let index = LineIndex::new(piece).within(Some(range.start), Some(lines_before));
        let (mut edits, skipped) = py.detach(|| scan.fix_edits(name, piece, &index));
        for e in &mut edits {
            e.start += range.start;
            e.end += range.start;
        }
        for s in skipped {
            warn(
                py,
                &format!(
                    "{name}:{}:{}: the fix of {} overlaps an earlier fix; skipped",
                    s.line,
                    s.col,
                    scan.rules()[s.rule].name
                ),
            )?;
        }
        if edits.is_empty() {
            continue;
        }
        if fixing.dry_run {
            diffs.push_str(&trex::files::unified_diff(name, &text, &edits, fixing.context));
            continue;
        }
        if fixing.review.is_some() {
            queue.push(Pending { name: name.clone(), path: file.clone(), raw, text, edits });
            continue;
        }
        let out = trex::files::written(&raw, &edits, trex::files::ReadAs::Decoded);
        match py.detach(|| std::fs::write(file, out)) {
            Ok(()) => written += edits.len(),
            Err(e) => errors.push(format!("{name}: cannot write it: {e}")),
        }
    }
    if !errors.is_empty() {
        return Err(PyOSError::new_err(errors.join("; ")));
    }
    if fixing.dry_run {
        return Ok(PyString::new(py, &diffs).into_any().unbind());
    }
    if let Some(review) = fixing.review {
        written = review_fixes(py, review, &queue, fixing)?;
    }
    Ok(written.into_pyobject(py)?.into_any().unbind())
}

/// Put each change of `queue` to `review` in turn and write the ones it
/// keeps, giving how many were written: nothing is written until the review
/// has seen the last change, and nothing at all where it quits.
pub(crate) fn review_fixes(
    py: Python<'_>,
    review: &Bound<'_, PyAny>,
    queue: &[Pending],
    fixing: &Fixing<'_, '_>,
) -> PyResult<usize> {
    let queued: Vec<trex::review::Queued<'_>> =
        queue.iter().map(|p| trex::review::Queued { name: &p.name, text: &p.text, edits: &p.edits }).collect();
    let mut cursors: Vec<Offsets<'_>> = queue.iter().map(|p| chars_of(&p.text)).collect();
    let reviewed = trex::review::review(&queued, |c| -> PyResult<trex::review::Answer> {
        let units = &mut cursors[c.input];
        let explanation = match fixing.explain {
            Some(explain) => Some(explain(py, c.text, c.edit.start, c.edit.end)?),
            None => None,
        };
        let change = Change {
            path: c.name.to_string(),
            start: units.at(c.edit.start),
            end: units.at(c.edit.end),
            text: String::from_utf8_lossy(&c.text[c.edit.start..c.edit.end]).into_owned(),
            replacement: String::from_utf8_lossy(&c.edit.replacement).into_owned(),
            diff: trex::files::unified_diff(c.name, c.text, std::slice::from_ref(c.edit), fixing.context),
            template: trex::templates::silhouette_name(c.template),
            later: c.later,
            index: c.index,
            total: c.total,
            explanation,
        };
        answer_of(&review.call1((change,))?)
    })?;
    let Some(reviewed) = reviewed else {
        warn(py, "quit; every file is left as it was")?;
        return Ok(0);
    };
    if !reviewed.skipped.is_empty() {
        let n = reviewed.skipped.len();
        warn(py, &format!("{n} change{} skipped by a template answer", if n == 1 { "" } else { "s" }))?;
        if fixing.show_skipped {
            // The skipped fixes come file by file in position order, so one
            // forward count of characters per file places them all.
            let mut units = chars_of(&[]);
            let mut counting = None;
            for s in &reviewed.skipped {
                if counting != Some(s.input) {
                    counting = Some(s.input);
                    units = chars_of(&queue[s.input].text);
                }
                warn(py, &format!("  {} [{}..{}]", queue[s.input].name, units.at(s.start), units.at(s.end)))?;
            }
        }
    }
    let mut written = 0usize;
    for (p, kept) in queue.iter().zip(&reviewed.kept) {
        if kept.is_empty() {
            continue;
        }
        let out = trex::files::written(&p.raw, kept, trex::files::ReadAs::Decoded);
        py.detach(|| std::fs::write(&p.path, out))
            .map_err(|e| PyOSError::new_err(format!("{}: cannot write it: {e}", p.name)))?;
        written += kept.len();
    }
    Ok(written)
}

/// One file a follow scans: its name, its stream, the decoder its opening
/// declared, the characters of its text pushed so far, and how many
/// findings it has handed out since it last began.
struct FollowedFile {
    name: String,
    stream: SharedRuleStream,
    decoder: trex::encoding::Incremental,
    chars_through: usize,
    given: usize,
}

/// Files followed as they grow, each finding of a library's rules handed
/// out once nothing that arrives later can change it.
///
/// Iterating it waits for the files to grow, a second at a time, and ends
/// only once `max_count` findings have come from every file; interrupting
/// it stops the wait. A file truncated, replaced or removed is told as a
/// `TrexWarning` and scanned again from its start. It is iterated in the
/// thread that made it, since the change notifications it waits on are
/// received by one thread.
#[pyclass(module = "trex", unsendable)]
pub(crate) struct Follow {
    defs: Vec<Py<Rule>>,
    files: Vec<FollowedFile>,
    follower: Option<trex::follow::Follower>,
    ready: VecDeque<Finding>,
    cap: Option<usize>,
    /// Whether a file truncated, replaced or removed keeps its count toward
    /// `cap` rather than starting it again.
    keep_count: bool,
}

impl Follow {
    /// Whether every file has given all `max_count` lets it.
    fn done(&self) -> bool {
        self.follower.is_none() || self.cap.is_some_and(|n| self.files.iter().all(|f| f.given >= n))
    }

    /// Queue the findings of `found`, what file `k`'s stream made, as many
    /// as `max_count` leaves it.
    fn take(&mut self, py: Python<'_>, k: usize, found: Vec<Found>) -> PyResult<()> {
        let Follow { defs, files, ready, cap, .. } = self;
        let file = &mut files[k];
        for mut one in found {
            if let Some(n) = *cap {
                one.findings.truncate(n.saturating_sub(file.given));
            }
            file.given += one.findings.len();
            if one.findings.is_empty() {
                continue;
            }
            // What the stream holds ends where the text pushed so far does.
            let ahead = file.chars_through - OffsetUnit::CodePoints.count(&one.input);
            let byte_base = one.byte_base.ok_or_else(|| PyOSError::new_err(unplaced().to_string()))?;
            let index = one.index();
            let scanned = Arc::new(Scanned {
                scan: Arc::clone(file.stream.scan()),
                name: one.name.clone(),
                text: one.input,
                byte_base,
                index,
                as_bytes: false,
            });
            ready.extend(findings_of(py, &scanned, defs, ahead, one.findings)?);
        }
        Ok(())
    }

    /// Scan `text`, file `k`'s next text.
    fn push(&mut self, py: Python<'_>, k: usize, text: &[u8]) -> PyResult<()> {
        let file = &mut self.files[k];
        file.chars_through += OffsetUnit::CodePoints.count(text);
        let found = file.stream.push(text);
        self.take(py, k, found)
    }

    /// File `k` was truncated, replaced or removed: queue what its stream held
    /// until then, and begin it again at the start of the file now under its
    /// name, a new input whose count starts again unless `keep_count` carries it on.
    fn restart(&mut self, py: Python<'_>, k: usize) -> PyResult<()> {
        let file = &mut self.files[k];
        let fresh = SharedRuleStream::new(
            Arc::clone(file.stream.scan()),
            Some(&file.name),
            0,
            Some(0),
            Some((OffsetUnit::CodePoints, 0)),
        );
        let ended = std::mem::replace(&mut file.stream, fresh).finish();
        self.take(py, k, ended)?;
        let keep_count = self.keep_count;
        let file = &mut self.files[k];
        file.chars_through = 0;
        if !keep_count {
            file.given = 0;
        }
        file.decoder = trex::encoding::Incremental::from_start();
        Ok(())
    }
}

#[pymethods]
impl Follow {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Finding>> {
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
                trex::follow::Followed::Appended { bytes, .. } => {
                    let text = self.files[k].decoder.decode(&bytes);
                    self.push(py, k, &text)?;
                }
                trex::follow::Followed::Truncated => {
                    warn(py, &format!("{name}: truncated; scanning it from its start"))?;
                    self.restart(py, k)?;
                }
                trex::follow::Followed::Replaced => {
                    warn(py, &format!("{name}: replaced by another file; scanning that from its start"))?;
                    self.restart(py, k)?;
                }
                trex::follow::Followed::Gone => {
                    warn(py, &format!("{name}: removed; waiting for a file under its name"))?;
                    self.restart(py, k)?;
                }
            }
        }
    }
}

/// A follow of the files `paths` names under `lib`'s rules: each read as
/// `reading` names, its tail, an open range or the whole of it, and then
/// what it gains as it grows.
pub(crate) fn follow(
    py: Python<'_>,
    lib: &Library,
    paths: PathArg,
    max_count: Option<usize>,
    keep_count: bool,
    reading: &Reading,
) -> PyResult<Follow> {
    if keep_count && max_count.is_none() {
        return Err(PyValueError::new_err("keep_count= keeps the count max_count= caps, and no max_count was given"));
    }
    if reading.select.is_some_and(|w| !w.runs_to_end()) {
        return Err(PyValueError::new_err(
            "follow() reads on as a file grows, and a lines= range with a last line ends before the file does; follow a tail=, a lines='A..' range or the whole file",
        ));
    }
    let scan = scan_of(lib)?;
    let on_records = scan.record_rules();
    if !on_records.is_empty() {
        return Err(PyValueError::new_err(format!(
            "{} fire on records, and a record is whole only once the input holding it ends, which a followed file does not; follow the rules that fire on matches",
            on_records.join(", ")
        )));
    }
    if !scan.stream(None, 0, None, None).commits_early() {
        return Err(PyValueError::new_err(
            "no match of these rules is final before its input ends, since one reads a content guard, a field anchor or a whole-input axis, and a followed file does not end; check it without following it",
        ));
    }
    let defs = definitions(py, &scan)?;
    let walked = walked(py, paths, &reading.walk)?;
    if !walked.errors.is_empty() {
        return Err(PyOSError::new_err(walked.errors.join("; ")));
    }
    let mut follow =
        Follow { defs, files: Vec::new(), follower: None, ready: VecDeque::new(), cap: max_count, keep_count };
    let mut watched: Vec<(PathBuf, usize)> = Vec::new();
    for (name, path) in &walked.files {
        let part = py
            .detach(|| Part::of_file(path, reading))
            .map_err(|e| PyOSError::new_err(format!("{name}: {e}")))?;
        if part.binary && !reading.binary {
            if walked.lone {
                return Err(PyValueError::new_err(format!(
                    "{name} holds a NUL byte and is binary; binary=True reads it"
                )));
            }
            continue;
        }
        let stream = SharedRuleStream::new(
            Arc::clone(&scan),
            Some(name),
            part.byte_base,
            part.line_base,
            Some((OffsetUnit::CodePoints, part.units_ahead)),
        );
        follow.files.push(FollowedFile {
            name: name.clone(),
            stream,
            decoder: trex::encoding::Incremental::after(part.encoding),
            chars_through: part.units_ahead,
            given: 0,
        });
        let k = follow.files.len() - 1;
        follow.push(py, k, &part.text)?;
        watched.push((path.clone(), part.input_len));
    }
    if follow.files.is_empty() {
        return Ok(follow);
    }
    let follower = trex::follow::Follower::new(&watched).map_err(|e| PyOSError::new_err(format!("cannot follow {e}")))?;
    if let Some(why) = follower.unnotified() {
        warn(py, &format!("no change notifications ({why}); looking at the files once a second"))?;
    }
    follow.follower = Some(follower);
    Ok(follow)
}
