//! Declarations a pattern reads by name, held in a `Library`, the shipped
//! atoms every pattern reads without one, the rules a library declares, and
//! the tokens an input is read as under a library.

use std::path::PathBuf;

use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString};
use trex::declarations::{AtomDecl, DeclareError, Form, Kept};

use crate::{Input, Match, units_of};

/// A declaration refused, as the Python error that says why: a pattern file
/// or a directory that cannot be read is an `OSError`, the rest a
/// `ValueError`.
pub(crate) fn declare_error(e: &DeclareError) -> PyErr {
    match e {
        DeclareError::Exists { name } => PyValueError::new_err(format!(
            "an atom named {name} is already declared here; replace=True replaces it"
        )),
        DeclareError::Read { .. } | DeclareError::ReadDir { .. } => PyOSError::new_err(e.to_string()),
        DeclareError::LineBreak { .. }
        | DeclareError::Backtick
        | DeclareError::NoKeyword { .. }
        | DeclareError::NoPatternFile { .. }
        | DeclareError::Refused(_) => PyValueError::new_err(e.to_string()),
    }
}

/// `s` as Python's `repr` writes a str.
pub(crate) fn repr_of(py: Python<'_>, s: &str) -> PyResult<String> {
    Ok(PyString::new(py, s).repr()?.to_string())
}

/// `path` as an absolute path, so a library built again after the working
/// directory changes reads the same file.
fn absolute(path: &std::path::Path) -> PyResult<PathBuf> {
    std::path::absolute(path).map_err(|e| PyOSError::new_err(format!("{}: {e}", path.display())))
}

/// One declared atom.
#[pyclass(frozen, module = "trex")]
pub(crate) struct Atom {
    /// The name a pattern reads it by, as `\{name}`.
    #[pyo3(get)]
    name: String,
    /// What it declares: `"shape"`, `"shape-after"`, `"kind"`, `"let"` or
    /// `"rule"`.
    #[pyo3(get)]
    form: &'static str,
    /// The byte-pattern or pattern the declaration gives.
    #[pyo3(get)]
    definition: String,
    /// Where it was declared: `"library"`, the pattern file's path, or
    /// `"shipped"`.
    #[pyo3(get)]
    source: String,
    /// What a shipped atom matches; empty for any other.
    #[pyo3(get)]
    description: String,
}

#[pymethods]
impl Atom {
    /// The atom as a plain dict of its fields.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("name", &self.name)?;
        d.set_item("form", self.form)?;
        d.set_item("definition", &self.definition)?;
        d.set_item("source", &self.source)?;
        d.set_item("description", &self.description)?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Atom(name={}, form={}, source={})",
            repr_of(py, &self.name)?,
            repr_of(py, self.form)?,
            repr_of(py, &self.source)?
        ))
    }
}

/// The outcome of one `test` line: an atom's expectations, run against what
/// the library finally declares.
#[pyclass(frozen, module = "trex")]
pub(crate) struct AtomTest {
    /// The atom the line tests.
    #[pyo3(get)]
    name: String,
    /// Whether every expectation on the line held.
    #[pyo3(get)]
    passed: bool,
    /// Texts the atom must match whole.
    #[pyo3(get)]
    accepts: Vec<String>,
    /// Texts the atom must match nowhere.
    #[pyo3(get)]
    rejects: Vec<String>,
    /// What trex reported for each expectation that failed.
    #[pyo3(get)]
    failures: Vec<String>,
    /// The pattern file the line is in; `None` for one declared here.
    #[pyo3(get)]
    file: Option<String>,
}

#[pymethods]
impl AtomTest {
    /// The outcome as a plain dict of its fields.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("name", &self.name)?;
        d.set_item("passed", self.passed)?;
        d.set_item("accepts", &self.accepts)?;
        d.set_item("rejects", &self.rejects)?;
        d.set_item("failures", &self.failures)?;
        d.set_item("file", &self.file)?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("AtomTest(name={}, passed={})", repr_of(py, &self.name)?, if self.passed { "True" } else { "False" }))
    }
}

/// How serious a rule's finding is, as Python names it.
pub(crate) fn severity_name(s: trex::Severity) -> &'static str {
    match s {
        trex::Severity::Error => "error",
        trex::Severity::Warning => "warning",
        trex::Severity::Note => "note",
    }
}

/// One rule as its declaration makes it.
#[pyclass(frozen, module = "trex")]
pub(crate) struct Rule {
    /// The rule's name, the id a finding is reported under.
    #[pyo3(get)]
    name: String,
    /// How serious a finding is: `"error"`, `"warning"` or `"note"`.
    #[pyo3(get)]
    severity: &'static str,
    /// What a finding says: a report template rendered at each match.
    #[pyo3(get)]
    message: String,
    /// What the rule matches, as written.
    #[pyo3(get)]
    pattern: String,
    /// The template a fix renders in the match's place; `None` where the
    /// rule has none.
    #[pyo3(get)]
    fix: Option<String>,
    /// The globs the inputs the rule reads are kept or dropped by; every
    /// input where there are none.
    #[pyo3(get)]
    files: Vec<String>,
    /// The rule's `meta.KEY = VALUE` lines.
    #[pyo3(get)]
    meta: Py<PyDict>,
    /// The values `meta.tags` lists.
    #[pyo3(get)]
    tags: Vec<String>,
    /// The pattern file the rule is in; `None` for one declared here.
    #[pyo3(get)]
    file: Option<String>,
    /// The line the rule opens on, counting from 1.
    #[pyo3(get)]
    line: usize,
}

impl Rule {
    pub(crate) fn of(py: Python<'_>, rule: &trex::Rule) -> PyResult<Rule> {
        let meta = PyDict::new(py);
        for (k, v) in &rule.meta {
            meta.set_item(k, v)?;
        }
        Ok(Rule {
            name: rule.name.clone(),
            severity: severity_name(rule.severity),
            message: rule.message.clone(),
            pattern: rule.source.clone(),
            fix: rule.fix.clone(),
            files: rule.files.clone(),
            meta: meta.unbind(),
            tags: rule.tags(),
            file: rule.file.as_ref().map(|p| p.display().to_string()),
            line: rule.line,
        })
    }
}

#[pymethods]
impl Rule {
    /// The rule as a plain dict of its fields, `meta` a copy.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("name", &self.name)?;
        d.set_item("severity", self.severity)?;
        d.set_item("message", &self.message)?;
        d.set_item("pattern", &self.pattern)?;
        d.set_item("fix", &self.fix)?;
        d.set_item("files", &self.files)?;
        d.set_item("meta", self.meta.bind(py).copy()?)?;
        d.set_item("tags", &self.tags)?;
        d.set_item("file", &self.file)?;
        d.set_item("line", self.line)?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("Rule(name={}, severity={})", repr_of(py, &self.name)?, repr_of(py, self.severity)?))
    }
}

/// A set of declarations: shapes the lexer tries, kinds fused from patterns,
/// named sub-patterns and rules, which a pattern reads by name as `\{name}`.
/// A pattern given `lib=` this library reads what it declares.
///
/// `Library.shipped()` is the shipped atoms, which every pattern reads with
/// no library: a listing to iterate and `test()`, refusing declarations.
#[pyclass(module = "trex")]
pub(crate) struct Library {
    decls: trex::Declarations,
    /// Whether this is the read-only listing of the shipped atoms.
    shipped: bool,
}

impl Library {
    /// The set a pattern given this library reads.
    pub(crate) fn shapes(&self) -> trex::ShapeSet {
        self.decls.set().clone()
    }

    /// Refuses a change to the shipped listing.
    fn writable(&self) -> PyResult<()> {
        if self.shipped {
            return Err(PyTypeError::new_err(
                "the shipped library is read-only; trex.Library() makes one to declare into",
            ));
        }
        Ok(())
    }

    fn add(&mut self, atom: &AtomDecl<'_>, replace: bool) -> PyResult<()> {
        self.writable()?;
        let here = std::env::current_dir().map_err(|e| PyOSError::new_err(format!("the working directory: {e}")))?;
        self.decls.add(atom, Some(here), replace).map_err(|e| declare_error(&e))
    }

    fn atom_list(&self) -> PyResult<Vec<Atom>> {
        if self.shipped {
            return Ok(trex::library::entries()
                .iter()
                .map(|e| Atom {
                    name: e.name.to_string(),
                    form: if e.is_kind() { "kind" } else { "let" },
                    definition: String::new(),
                    source: "shipped".to_string(),
                    description: e.what.to_string(),
                })
                .collect());
        }
        Ok(self
            .decls
            .atoms()
            .map_err(|e| declare_error(&e))?
            .into_iter()
            .map(|a| Atom {
                name: a.name,
                form: a.form.keyword(),
                definition: a.definition,
                source: match &a.file {
                    Some(file) => file.display().to_string(),
                    None => "library".to_string(),
                },
                description: String::new(),
            })
            .collect())
    }
}

#[pymethods]
impl Library {
    /// An empty library.
    #[new]
    fn new() -> Self {
        Library { decls: trex::Declarations::new(), shipped: false }
    }

    /// A library declaring what the pattern files `paths` say, each a str
    /// or an `os.PathLike`, in order, a directory as every `.trex` file
    /// under it in path order.
    #[staticmethod]
    #[pyo3(signature = (*paths))]
    fn load(paths: Vec<PathBuf>) -> PyResult<Library> {
        let mut lib = Library::new();
        lib.include(paths)?;
        Ok(lib)
    }

    /// The atoms trex ships, which every pattern reads with no library: to
    /// iterate, each with what it matches, and to `test()`. It takes no
    /// declarations.
    #[staticmethod]
    fn shipped() -> Library {
        Library { decls: trex::Declarations::new(), shipped: true }
    }

    /// Declares a shape: a bounded byte-pattern the lexer tries at each
    /// position before its built-in recognizers, or with `after=True` only
    /// where none of them matched, as `[A-Z]{2,4}-\d{1,4}`. `accepts` and
    /// `rejects` are texts it must match whole and match nowhere, which
    /// `test()` checks. A name declared here already raises `ValueError`
    /// unless `replace=True`.
    #[pyo3(signature = (name, bytepat, *, after = false, accepts = Vec::new(), rejects = Vec::new(), replace = false))]
    fn shape(
        &mut self,
        name: &str,
        bytepat: &str,
        after: bool,
        accepts: Vec<String>,
        rejects: Vec<String>,
        replace: bool,
    ) -> PyResult<()> {
        let form = if after { Form::ShapeAfter } else { Form::Shape };
        self.add(&AtomDecl { name, form, definition: bytepat, accepts: &accepts, rejects: &rejects }, replace)
    }

    /// Declares a kind: a pattern whose every match, after the lex, becomes
    /// one token of this kind. The keywords are `shape`'s.
    #[pyo3(signature = (name, pattern, *, accepts = Vec::new(), rejects = Vec::new(), replace = false))]
    fn kind(
        &mut self,
        name: &str,
        pattern: &str,
        accepts: Vec<String>,
        rejects: Vec<String>,
        replace: bool,
    ) -> PyResult<()> {
        self.add(&AtomDecl { name, form: Form::Kind, definition: pattern, accepts: &accepts, rejects: &rejects }, replace)
    }

    /// Declares a named sub-pattern, read in place wherever `\{name}`
    /// appears. The keywords are `shape`'s.
    #[pyo3(name = "let", signature = (name, pattern, *, accepts = Vec::new(), rejects = Vec::new(), replace = false))]
    fn let_(
        &mut self,
        name: &str,
        pattern: &str,
        accepts: Vec<String>,
        rejects: Vec<String>,
        replace: bool,
    ) -> PyResult<()> {
        self.add(&AtomDecl { name, form: Form::Let, definition: pattern, accepts: &accepts, rejects: &rejects }, replace)
    }

    /// Declares one line as a pattern file writes it: `shape`,
    /// `shape-after`, `kind`, `let` or `rule`, or a line adding to one, a
    /// `test` or `fields` line or a rule's `fix`, `meta`, `files`, `unless`,
    /// `record`, `record-start` or `record-span` line. A relative `@file`
    /// in it is read from the working directory as it is now. A refused
    /// line raises `ValueError` and leaves the library as it was.
    fn declare(&mut self, line: &str) -> PyResult<()> {
        self.writable()?;
        let here = std::env::current_dir().map_err(|e| PyOSError::new_err(format!("the working directory: {e}")))?;
        self.decls.declare(line, Some(here)).map_err(|e| declare_error(&e))
    }

    /// Declares what the pattern files `paths` say, in order, a directory as
    /// every `.trex` file under it in path order, each in place of an
    /// earlier import of the same file, so including one again reads its
    /// current text. A directory with no `.trex` file under it raises
    /// `ValueError`.
    #[pyo3(signature = (*paths))]
    fn include(&mut self, paths: Vec<PathBuf>) -> PyResult<()> {
        self.writable()?;
        let paths = paths.iter().map(|p| absolute(p)).collect::<PyResult<Vec<_>>>()?;
        self.decls.include(&paths).map_err(|e| declare_error(&e))
    }

    /// Takes atoms out by name, each with the lines declared here that add
    /// to it (its `test` lines, a rule's `fix` and `meta`), and the atoms of
    /// included pattern files by path, as one change, so atoms that read one
    /// another go together. A
    /// path takes out the file it is, or every file included from under a
    /// directory. An `os.PathLike` is a path, and so is a str naming an
    /// included file or a directory one was included from under; any other
    /// str is a name. Where one cannot be taken out, or an atom that would
    /// stay reads one taken out, `ValueError` says why and nothing is taken
    /// out.
    #[pyo3(signature = (*items))]
    fn remove(&mut self, items: Vec<Bound<'_, PyAny>>) -> PyResult<()> {
        self.writable()?;
        let included = self.decls.files();
        let mut names = Vec::new();
        let mut files = Vec::new();
        for item in &items {
            if item.is_instance_of::<PyString>() {
                let given: String = item.extract()?;
                let path = absolute(std::path::Path::new(&given))?;
                if included.iter().any(|f| f.starts_with(&path)) { files.push(path) } else { names.push(given) }
            } else {
                let path: PathBuf = item.extract().map_err(|e: PyErr| {
                    PyTypeError::new_err(format!("remove() takes atom names and pattern file paths: {e}"))
                })?;
                files.push(absolute(&path)?);
            }
        }
        let mut next = self.decls.clone();
        let kept = next.remove(&names, &files).map_err(|e| declare_error(&e))?;
        if !kept.is_empty() {
            let why: Vec<String> = kept
                .iter()
                .map(|k| match k {
                    Kept::InFile { name, file } => {
                        format!("{name} is declared by {}; removing that file takes it out", file.display())
                    }
                    Kept::NoName { name } => format!("no atom named {name} is declared here"),
                    Kept::NoFile { file } => format!("nothing included here is {} or under it", file.display()),
                })
                .collect();
            return Err(PyValueError::new_err(why.join("; ")));
        }
        self.decls = next;
        Ok(())
    }

    /// Takes every declaration and every included file out.
    fn clear(&mut self) -> PyResult<()> {
        self.writable()?;
        self.decls.clear();
        Ok(())
    }

    /// Runs every `test` line, in declaration order, and gives one outcome
    /// per line; the shipped listing runs the shipped atoms' own.
    fn test(&self) -> PyResult<Vec<AtomTest>> {
        let set = if self.shipped {
            let mut shipped = trex::ShapeSet::new();
            for (n, line) in trex::library::TESTS.iter().enumerate() {
                shipped.declare_test(line, n + 1).map_err(|e| PyValueError::new_err(e.msg))?;
            }
            shipped
        } else {
            self.decls.set().clone()
        };
        let failures = set.run_tests();
        Ok(set
            .tests()
            .iter()
            .map(|test| {
                let failed: Vec<String> = failures
                    .iter()
                    .filter(|f| f.line == test.line && f.file == test.file && f.name == test.name)
                    .map(|f| f.msg.clone())
                    .collect();
                AtomTest {
                    name: test.name.clone(),
                    passed: failed.is_empty(),
                    accepts: test.accepts.clone(),
                    rejects: test.rejects.clone(),
                    failures: failed,
                    file: test.file.as_ref().map(|p| p.display().to_string()),
                }
            })
            .collect())
    }

    /// The rules declared, in declaration order.
    #[getter]
    fn rules(&self, py: Python<'_>) -> PyResult<Vec<Rule>> {
        self.decls.set().rules().iter().map(|r| Rule::of(py, r)).collect()
    }

    /// The findings of the rules declared here over `text`, a str or bytes,
    /// or over the files `path=` names, one path or a list of them, a
    /// directory walked as `trex.files` walks one, as `trex scan --rules`
    /// finds them: in position order within each input, then by rule.
    ///
    /// `max_count` keeps the first findings of each input. `head`, `tail`
    /// or `lines` scan only that part of each input, counted in records of
    /// `unit`, each finding still placed at the input's own line and
    /// offset. A file holding a NUL byte is passed over unless
    /// `binary=True`; named alone, it raises `ValueError`.
    #[pyo3(signature = (text = None, *, path = None, max_count = None, head = None, tail = None, lines = None, unit = "line", hidden = false, no_ignore = false, globs = None, binary = false))]
    // Each keyword is the call's own, read once here.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    fn check(
        &self,
        py: Python<'_>,
        text: Option<&Bound<'_, PyAny>>,
        path: Option<crate::PathArg>,
        max_count: Option<usize>,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        hidden: bool,
        no_ignore: bool,
        globs: Option<Vec<String>>,
        binary: bool,
    ) -> PyResult<Vec<crate::rules::Finding>> {
        let reading = crate::rules::Reading::of(head, tail, lines, unit, hidden, no_ignore, globs, binary)?;
        crate::rules::check(py, self, text, path, max_count, &reading)
    }

    /// Writes the fixes of the rules declared here into the files `path`
    /// names, as `trex scan --rules --fix` writes them, and gives how many
    /// it wrote. A fix overlapping an earlier one is skipped with a
    /// `TrexWarning`.
    ///
    /// `dry_run=True` writes nothing and gives the unified diff the fixes
    /// would make, with `context` lines around each change. `review=` takes
    /// a callable handed each fix as a `Change`, which answers `True` to
    /// apply it, `False` to skip it, a str to put in its place,
    /// `"template"` or `"skip template"` to apply or skip it and every later
    /// fix of its template, `"all"` to apply it and every fix after it, or
    /// `"quit"` to write nothing; nothing is written until it has answered
    /// for the last. How many fixes a template answer skipped is told as a
    /// `TrexWarning`, and with `show_skipped=True` the position of each.
    /// `head`, `tail` or `lines` fix only that part of each file.
    #[pyo3(signature = (path, *, dry_run = false, context = 3, review = None, show_skipped = false, head = None, tail = None, lines = None, unit = "line", hidden = false, no_ignore = false, globs = None, binary = false))]
    // Each keyword is the call's own, read once here.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    fn fix(
        &self,
        py: Python<'_>,
        path: crate::PathArg,
        dry_run: bool,
        context: usize,
        review: Option<&Bound<'_, PyAny>>,
        show_skipped: bool,
        head: Option<usize>,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        hidden: bool,
        no_ignore: bool,
        globs: Option<Vec<String>>,
        binary: bool,
    ) -> PyResult<Py<PyAny>> {
        let reading = crate::rules::Reading::of(head, tail, lines, unit, hidden, no_ignore, globs, binary)?;
        // A rule's own message says what its finding means, so its review
        // explains nothing under a fix, as the command line's does not.
        let fixing = crate::rules::Fixing { dry_run, context, review, show_skipped, explain: None };
        crate::rules::fix(py, self, path, &fixing, &reading)
    }

    /// Follows the files `path` names as they grow, after each one's
    /// `tail`, its open `lines="A.."` range or the whole of it, and yields
    /// each finding of the rules declared here once nothing that arrives
    /// later can change it. Iterating waits for the files to grow and ends
    /// once `max_count` findings have come from every file; a file
    /// truncated, replaced or removed is told as a `TrexWarning` and read
    /// again from its start, as a new input whose count starts again unless
    /// `keep_count=True` keeps it.
    #[pyo3(signature = (path, *, max_count = None, keep_count = false, tail = None, lines = None, unit = "line", hidden = false, no_ignore = false, globs = None, binary = false))]
    // Each keyword is the call's own, read once here.
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    fn follow(
        &self,
        py: Python<'_>,
        path: crate::PathArg,
        max_count: Option<usize>,
        keep_count: bool,
        tail: Option<usize>,
        lines: Option<&str>,
        unit: &str,
        hidden: bool,
        no_ignore: bool,
        globs: Option<Vec<String>>,
        binary: bool,
    ) -> PyResult<crate::rules::Follow> {
        let reading = crate::rules::Reading::of(None, tail, lines, unit, hidden, no_ignore, globs, binary)?;
        crate::rules::follow(py, self, path, max_count, keep_count, &reading)
    }

    /// The names declared, in declaration order.
    #[getter]
    fn names(&self) -> PyResult<Vec<String>> {
        Ok(self.atom_list()?.into_iter().map(|a| a.name).collect())
    }

    /// The pattern files included, in order, as absolute paths.
    #[getter]
    fn files(&self) -> Vec<String> {
        self.decls.files().iter().map(|f| f.display().to_string()).collect()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(PyList::new(py, self.atom_list()?)?.as_any().try_iter()?.into_any())
    }

    fn __len__(&self) -> PyResult<usize> {
        Ok(self.atom_list()?.len())
    }

    fn __contains__(&self, name: &str) -> PyResult<bool> {
        Ok(self.atom_list()?.iter().any(|a| a.name == name))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        if self.shipped {
            return Ok("Library.shipped()".to_string());
        }
        Ok(format!("Library(names={})", PyList::new(py, self.names()?)?.repr()?))
    }
}

/// One token as the lexer read it.
#[pyclass(frozen, module = "trex")]
pub(crate) struct Token {
    /// The token's kind: `"number"`, `"word"`, `"quoted"`, `"ip"` and the
    /// rest, `"open"` and `"close"` for a bracket, or the name a declared
    /// shape or kind gave it.
    #[pyo3(get)]
    kind: String,
    /// Where it starts, in the input's units.
    #[pyo3(get)]
    start: usize,
    /// Where it ends, exclusive, in the input's units.
    #[pyo3(get)]
    end: usize,
    /// Its text, in the input's type.
    #[pyo3(get)]
    text: Py<PyAny>,
    /// Its text parsed as its kind, as `Match.value` reads a register;
    /// `None` where the kind carries no value.
    #[pyo3(get)]
    value: Py<PyAny>,
}

#[pymethods]
impl Token {
    /// The token as a plain dict of its fields.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("kind", &self.kind)?;
        d.set_item("start", self.start)?;
        d.set_item("end", self.end)?;
        d.set_item("text", self.text.bind(py))?;
        d.set_item("value", self.value.bind(py))?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Token(kind={}, start={}, end={}, text={})",
            repr_of(py, &self.kind)?,
            self.start,
            self.end,
            self.text.bind(py).repr()?
        ))
    }
}

/// The tokens trex reads `text`, a str or bytes, or the file at `path=` as,
/// each with its kind, its span in the input's units, its text and the
/// value it parses to. Whitespace is left out unless `whitespace=True`,
/// since a pattern never matches it. `lib=` lexes under a library's
/// declarations, a pattern file's path, a directory's, read as its `.trex`
/// files, or a list of them. A file holding a NUL byte raises `ValueError`
/// unless `binary=True`.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, whitespace = false, lib = None, binary = false))]
pub(crate) fn tokens(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    whitespace: bool,
    lib: Option<&Bound<'_, PyAny>>,
    binary: bool,
) -> PyResult<Vec<Token>> {
    let shapes = crate::shapes_of(lib)?;
    let read;
    let held = match (text, &path) {
        (Some(obj), None) => Input::of(obj)?,
        (None, Some(p)) => {
            let raw = py.detach(|| std::fs::read(p)).map_err(|e| PyOSError::new_err(format!("{}: {e}", p.display())))?;
            if !binary && trex::files::is_binary(&raw) {
                return Err(PyValueError::new_err(format!(
                    "{} holds a NUL byte and is binary; binary=True reads it",
                    p.display()
                )));
            }
            read = String::from_utf8_lossy(&trex::encoding::decode(raw)).into_owned();
            Input::Text(&read)
        }
        (Some(_), Some(_)) | (None, None) => {
            return Err(PyTypeError::new_err("give the text, a str or bytes, or path=, a file's path; one of them"));
        }
    };
    let bytes = held.bytes();
    let toks = py.detach(|| {
        if shapes.is_empty() {
            trex::lexer::lex(bytes)
        } else {
            trex::lexer::lex_with_shapes(bytes, &trex::lexer::blob_runs(bytes), &shapes, 0)
        }
    });
    let mut units = units_of(&held);
    let mut out = Vec::with_capacity(toks.len());
    for t in &toks {
        if !whitespace && !t.is_significant() {
            continue;
        }
        let (s, e) = (t.start(), t.end());
        let piece = String::from_utf8_lossy(&bytes[s..e]);
        let value = match t.kind {
            trex::token::TokenKind::Custom(_) => None,
            kind => trex::typed::value_of(kind, &piece),
        };
        let value = match value {
            Some(v) => Match::py_value(py, t.kind, &v, trex::typed::DurationUnit::Nanoseconds)?,
            None => py.None(),
        };
        out.push(Token {
            kind: shapes.kind_name(t.kind),
            start: units.at(s),
            end: units.at(e),
            text: held.slice(py, s, e),
            value,
        });
    }
    Ok(out)
}
