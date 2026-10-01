//! The declarations a lex and a parse share: token shapes the lexer runs
//! alongside its built-in recognizers, token kinds declared from patterns
//! over the stream, and named sub-patterns a `\{name}` inlines.

use crate::ast::Pattern;
use crate::bytepat::{BytePat, parse as parse_bytepat};

/// The ids below this belong to declared shapes and kinds; the ids from it
/// up belong to the shipped library, so a pattern naming both never sees
/// two kinds under one id.
pub const LIBRARY_ID_BASE: u8 = 128;

/// Where a shape sits relative to the built-in recognizers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precedence {
    /// Tried before every built-in, so the shape wins an overlap.
    Before,
    /// Tried where no built-in matches.
    After,
}

/// One declared token shape.
#[derive(Clone, Debug)]
pub struct TokenShape {
    /// The name the `\{name}` atom uses.
    pub name: String,
    /// The whole-anchored byte-pattern, bounded by construction.
    pub pat: BytePat,
    /// Longest byte length this shape can match, from [`BytePat::max_len`].
    pub window: usize,
    /// Where it sits against the built-in cascade.
    pub precedence: Precedence,
    /// The payload of the [`crate::token::TokenKind::Custom`] the lexer
    /// emits for it.
    pub id: u8,
    /// A check on the bytes the pattern accepted, where the shape's standard
    /// defines one: a token the check refuses is not this shape.
    pub guard: Option<fn(&[u8]) -> bool>,
}

/// A check on the span a kind's pattern matched, where the kind's standard
/// defines one: the span is a number then one of these unit symbols,
/// attached or one separator apart, so a number and a symbol a line apart
/// are not fused.
#[derive(Clone, Copy, Debug)]
pub struct UnitGuard {
    /// The symbols the span may end in.
    pub symbols: &'static [&'static str],
}

impl UnitGuard {
    /// Whether the span's bytes are a number then one of the symbols.
    #[must_use]
    pub fn accepts(self, text: &[u8]) -> bool {
        crate::quantity::context_span(text, self.symbols)
    }
}

/// A token kind declared from a pattern over the stream: after the lex, the
/// tokens each match covers fuse into one token of this kind.
#[derive(Clone, Debug)]
pub struct PatternKind {
    /// The name the `\{name}` atom uses.
    pub name: String,
    /// The payload of the [`crate::token::TokenKind::Custom`] the fused
    /// token carries.
    pub id: u8,
    /// The pattern whose matches become tokens.
    pub pattern: Pattern,
    /// A check on the matched span, where the kind's standard defines one:
    /// a span the check refuses is not fused.
    pub guard: Option<UnitGuard>,
}

/// A rejected shape declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeError {
    /// The offending declaration text.
    pub decl: String,
    /// The reason it is not a valid shape.
    pub msg: String,
}

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "token shape {:?}: {}", self.decl, self.msg)
    }
}

impl std::error::Error for ShapeError {}

/// How serious a rule's finding is: SARIF's own levels, which a GitHub
/// annotation carries as `error`, `warning` and `notice`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    /// The severity `error`, `warning` or `note` names.
    #[must_use]
    pub fn parse(name: &str) -> Option<Severity> {
        match name {
            "error" => Some(Severity::Error),
            "warning" => Some(Severity::Warning),
            "note" => Some(Severity::Note),
            _ => None,
        }
    }

    /// The name, as a rule writes it and as SARIF's `level` carries it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }

    /// The level a GitHub workflow annotation carries: `notice` for a note.
    #[must_use]
    pub fn github(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "notice",
        }
    }
}

/// One `rule` of a pattern file: a named pattern with what a finding of it
/// says, how serious it is, the fix that replaces the match, the inputs it
/// reads, the record it holds and the metadata a pipeline filters on. The
/// rule is also a sub-pattern under its name, so `\{name}` reads it and a
/// `test` line checks it.
#[derive(Clone, Debug)]
pub struct Rule {
    /// The pattern file the rule is in, where the set was declared from one.
    pub file: Option<std::path::PathBuf>,
    /// The line the rule opens on, from one.
    pub line: usize,
    /// The rule's name, the id a finding is reported under.
    pub name: String,
    /// What the rule matches.
    pub pattern: Pattern,
    /// The pattern as written.
    pub source: String,
    /// What a finding says: a report template rendered at each match, with
    /// the match's registers and their typed slices, where it stands, and
    /// `${rule}` and `${severity}`.
    pub message: String,
    /// How serious a finding is; a warning where the rule says nothing.
    pub severity: Severity,
    /// The rewrite template a fix renders in the match's place, where the
    /// rule has one.
    pub fix: Option<String>,
    /// The globs the inputs the rule reads are kept by (`*.py`) or dropped
    /// by (`!test_*`), as `-g` takes them; every input where there are none.
    pub files: Vec<String>,
    /// The patterns the record must not hold for the rule to fire.
    pub unless: Vec<Pattern>,
    /// What a record is, for a rule that fires on a record holding the
    /// pattern and none of `unless`: a line where a rule names `unless` and
    /// no record; nothing for a rule that fires on each match.
    pub record: Option<crate::records::RecordUnit>,
    /// The `meta.KEY = VALUE` lines, in order.
    pub meta: Vec<(String, String)>,
}

impl Rule {
    /// Whether the rule fires on a record rather than on each match: it
    /// names a record or a pattern the record must not hold.
    #[must_use]
    pub fn on_records(&self) -> bool {
        self.record.is_some() || !self.unless.is_empty()
    }

    /// What a record is for this rule: what it names, or a line where it
    /// names only what the record must not hold.
    #[must_use]
    pub fn record_unit(&self) -> crate::records::RecordUnit {
        self.record.clone().unwrap_or(crate::records::RecordUnit::Line)
    }

    /// The values `meta.tags` lists, comma-separated.
    #[must_use]
    pub fn tags(&self) -> Vec<String> {
        tags_of(&self.meta)
    }
}

/// The values the `tags` entries of a rule's metadata list, comma-separated.
#[must_use]
pub fn tags_of(meta: &[(String, String)]) -> Vec<String> {
    meta.iter()
        .filter(|(k, _)| k == "tags")
        .flat_map(|(_, v)| v.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string))
        .collect()
}

/// One `test` line of a pattern file: the texts a name matches as a whole
/// and the texts it matches nowhere in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibTest {
    /// The pattern file the line is in, where the set was declared from one.
    pub file: Option<std::path::PathBuf>,
    /// The line's number in its text, from one.
    pub line: usize,
    /// The name under test: a declared shape, kind or sub-pattern, or a
    /// library entry.
    pub name: String,
    /// The texts the name matches as a whole, from the first significant
    /// token to the last.
    pub accepts: Vec<String>,
    /// The spans the name reads out of a larger text, as `(span, text)`.
    ///
    /// What `accepts` cannot say. A kind read from the tokens around it takes
    /// part of a line and leaves the rest, so the whole extent is the wrong
    /// expectation for it: the question is which bytes it took, and out of
    /// what.
    pub reads: Vec<(String, String)>,
    /// The texts the name matches nowhere in.
    pub rejects: Vec<String>,
}

/// One expectation of a `test` line that the name does not meet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestFailure {
    /// The pattern file the line is in, where the set was declared from one.
    pub file: Option<std::path::PathBuf>,
    /// The line's number in its text, from one.
    pub line: usize,
    /// The name under test.
    pub name: String,
    /// The expectation and what happened instead.
    pub msg: String,
}

impl std::fmt::Display for TestFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.file {
            Some(path) => write!(f, "{}:{}: {} {}", path.display(), self.line, self.name, self.msg),
            None => write!(f, "line {}: {} {}", self.line, self.name, self.msg),
        }
    }
}

/// Where the significant tokens of `input` lie under `shapes`: the start of
/// the first and the end of the last, or none where every token is
/// whitespace.
fn significant_extent(input: &[u8], shapes: &ShapeSet) -> Option<(usize, usize)> {
    let blobs = crate::lexer::blob_runs(input);
    let toks = crate::lexer::lex_with_shapes(input, &blobs, shapes, 0);
    let first = toks.iter().find(|t| t.is_significant())?.start();
    let last = toks.iter().rev().find(|t| t.is_significant())?.end();
    Some((first, last))
}

/// The double-quoted text opening at byte `open` of a `test` line, with
/// `\"`, `\\`, `\n` and `\t` read, and the byte after its closing quote.
fn read_test_text(line: &str, open: usize) -> Result<(String, usize), String> {
    let mut out = String::new();
    let body = open + 1;
    let mut chars = line[body..].char_indices();
    while let Some((k, c)) = chars.next() {
        match c {
            '"' => return Ok((out, body + k + 1)),
            '\\' => match chars.next() {
                Some((_, '"')) => out.push('"'),
                Some((_, '\\')) => out.push('\\'),
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, other)) => {
                    return Err(format!(
                        "unknown escape \\{other} in a test text; it reads \\\", \\\\, \\n and \\t"
                    ));
                }
                None => return Err("a backslash ends the test text".to_string()),
            },
            other => out.push(other),
        }
    }
    Err("unterminated test text; it closes with a double quote".to_string())
}

/// The declarations a lex and a parse share.
#[derive(Clone, Debug, Default)]
pub struct ShapeSet {
    shapes: Vec<TokenShape>,
    kinds: Vec<PatternKind>,
    /// Named sub-patterns, in declaration order; a later one shadows an
    /// earlier one of the same name.
    lets: Vec<(String, Pattern)>,
    /// The `test` lines declared, in declaration order.
    tests: Vec<LibTest>,
    /// The `rule` declarations, in declaration order.
    rules: Vec<Rule>,
    /// The fields a `fields` line gives a named sub-pattern, by its name:
    /// what the marks a pattern was built from said of each field beyond
    /// the pattern, in the order the fields are written.
    fields: Vec<(String, Vec<crate::infer::marks::FieldMark>)>,
    /// The ids handed to declared shapes and kinds so far; the shipped
    /// library's start at [`LIBRARY_ID_BASE`] and are placed by their entry.
    next_id: u8,
    /// Whether a parse against this set falls back to the shipped library
    /// for a name it does not declare. Off only for the set the library is
    /// itself parsed into.
    library_off: bool,
    /// The directory a relative `@file` set in a declaration is read from:
    /// the pattern file's own while one is being declared, else the one
    /// [`Self::set_base_dir`] gave, or none, which reads from the current
    /// directory.
    base: Option<std::path::PathBuf>,
}

impl ShapeSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        ShapeSet::default()
    }

    /// An empty set whose parses never consult the shipped library: the
    /// one the library is built into.
    pub(crate) fn without_library() -> Self {
        ShapeSet { library_off: true, ..ShapeSet::default() }
    }

    /// Whether a parse against this set resolves a name it does not declare
    /// from the shipped library.
    #[must_use]
    pub fn consults_library(&self) -> bool {
        !self.library_off
    }

    /// The directory a relative `@file` set is read from during a parse
    /// against this set, or none for the current directory.
    #[must_use]
    pub fn base_dir(&self) -> Option<&std::path::Path> {
        self.base.as_deref()
    }

    /// Read a relative `@file` in a pattern parsed against this set, or in a
    /// line declared into it, from `dir`, or from the current directory where
    /// it is `None`. A pattern file declared into the set reads its own from
    /// beside the file whatever this says.
    pub fn set_base_dir(&mut self, dir: Option<std::path::PathBuf>) {
        self.base = dir;
    }

    /// Declare everything the pattern file at `path` says, as
    /// [`Self::declare_text`] does, with a relative `@file` set in it read
    /// from beside the file. A file whose byte-order mark declares UTF-8,
    /// UTF-16 or UTF-32 is read as its text, as an input to a scan is.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or a line of it is refused.
    pub fn declare_file(&mut self, path: &std::path::Path) -> Result<(), ShapeError> {
        let text = match std::fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&crate::encoding::decode(bytes)).into_owned(),
            Err(e) => {
                return Err(ShapeError {
                    decl: path.display().to_string(),
                    msg: format!("cannot read the pattern file: {e}"),
                });
            }
        };
        let before = self.base.take();
        self.base = path.parent().map(std::path::Path::to_path_buf);
        let tests_before = self.tests.len();
        let rules_before = self.rules.len();
        let declared = self.declare_text(&text);
        self.base = before;
        for test in &mut self.tests[tests_before..] {
            test.file = Some(path.to_path_buf());
        }
        for rule in &mut self.rules[rules_before..] {
            rule.file = Some(path.to_path_buf());
        }
        declared.map_err(|mut e| {
            e.msg = format!("{}: {}", path.display(), e.msg);
            e
        })
    }

    /// The pattern file at `path` declared as [`Self::declare_file`]
    /// declares it, and the named patterns a set built from it holds, in
    /// order: each `let` and `rule` under its name, and each line that is no
    /// declaration under its line number.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or a line of it is refused.
    pub fn declare_file_members(&mut self, path: &std::path::Path) -> Result<Vec<(String, Pattern)>, ShapeError> {
        let text = match std::fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&crate::encoding::decode(bytes)).into_owned(),
            Err(e) => {
                return Err(ShapeError {
                    decl: path.display().to_string(),
                    msg: format!("cannot read the pattern file: {e}"),
                });
            }
        };
        let before = self.base.take();
        self.base = path.parent().map(std::path::Path::to_path_buf);
        let tests_before = self.tests.len();
        let rules_before = self.rules.len();
        let mut members = Vec::new();
        let declared = self.declare_lines(&text, Some(&mut members));
        self.base = before;
        for test in &mut self.tests[tests_before..] {
            test.file = Some(path.to_path_buf());
        }
        for rule in &mut self.rules[rules_before..] {
            rule.file = Some(path.to_path_buf());
        }
        match declared {
            Ok(()) => Ok(members),
            Err(mut e) => {
                e.msg = format!("{}: {}", path.display(), e.msg);
                Err(e)
            }
        }
    }

    /// Whether nothing here changes a lex: no shape and no kind from a
    /// pattern. Named sub-patterns bear on the parse alone.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty() && self.kinds.is_empty()
    }

    /// The declared shapes, in declaration order.
    #[must_use]
    pub fn shapes(&self) -> &[TokenShape] {
        &self.shapes
    }

    /// The kinds declared from patterns, in declaration order.
    #[must_use]
    pub fn pattern_kinds(&self) -> &[PatternKind] {
        &self.kinds
    }

    /// The named sub-patterns, in declaration order.
    #[must_use]
    pub fn lets(&self) -> &[(String, Pattern)] {
        &self.lets
    }

    /// The `test` lines declared, in declaration order.
    #[must_use]
    pub fn tests(&self) -> &[LibTest] {
        &self.tests
    }

    /// The rules declared, in declaration order.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// The rule declared under `name`.
    #[must_use]
    pub fn rule_of(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.name == name)
    }

    /// The id of the shape or kind called `name`: the payload of its
    /// [`crate::token::TokenKind::Custom`].
    #[must_use]
    pub fn id_of(&self, name: &str) -> Option<u8> {
        self.shapes
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.id)
            .or_else(|| self.kinds.iter().find(|k| k.name == name).map(|k| k.id))
    }

    /// The name of the shape or kind `id`, here or in the shipped library.
    #[must_use]
    pub fn name_of(&self, id: u8) -> Option<&str> {
        self.shapes
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.as_str())
            .or_else(|| self.kinds.iter().find(|k| k.id == id).map(|k| k.name.as_str()))
            .or_else(|| crate::library::name_of(id))
    }

    /// The sub-pattern declared under `name`, the last so declared.
    #[must_use]
    pub fn let_of(&self, name: &str) -> Option<&Pattern> {
        self.lets.iter().rev().find(|(n, _)| n == name).map(|(_, p)| p)
    }

    /// The fields a `fields` line gives the sub-pattern `name`, in order.
    #[must_use]
    pub fn fields_of(&self, name: &str) -> Option<&[crate::infer::marks::FieldMark]> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, f)| f.as_slice())
    }

    /// Record a `fields` line, `NAME {mark}...`: the fields of the
    /// sub-pattern declared as `NAME` before it, in order, each a mark with
    /// the example text left out, as [`crate::infer::marks::field_marks`]
    /// reads them.
    ///
    /// # Errors
    ///
    /// A name no sub-pattern is declared under, a second `fields` line for
    /// one, a line naming no field, a mark that does not read, a field named
    /// twice, and a field the sub-pattern binds no register under or reads
    /// through an accessor that is not one.
    pub fn declare_fields(&mut self, decl: &str) -> Result<(), ShapeError> {
        let err = |msg: String| ShapeError { decl: decl.to_string(), msg };
        let (name, rest) = decl.split_once(char::is_whitespace).unwrap_or((decl, ""));
        let pattern = self
            .let_of(name)
            .ok_or_else(|| err(format!("no sub-pattern is declared as {name} above; a fields line follows its let")))?;
        if self.fields_of(name).is_some() {
            return Err(err(format!("{name} has a fields line already")));
        }
        let marks = crate::infer::marks::field_marks(rest).map_err(|e| err(e.to_string()))?;
        if marks.is_empty() {
            return Err(err("a fields line names at least one field".to_string()));
        }
        let bound = pattern.capture_names();
        for (k, mark) in marks.iter().enumerate() {
            if marks[..k].iter().any(|m| m.name == mark.name) {
                return Err(err(format!("{} is named twice", mark.written())));
            }
            let body = match &mark.accessor {
                Some(accessor) => format!("{}:{accessor}", mark.name),
                None => mark.name.clone(),
            };
            crate::rewrite::Reference::parse(&body, &bound).map_err(|e| err(format!("{}: {}", mark.written(), e.msg)))?;
        }
        self.fields.push((name.to_string(), marks));
        Ok(())
    }

    /// Whether a declaration here can take `name` without shadowing any
    /// other: alphanumerics and underscores, not a built-in atom's, not a
    /// shape, kind or sub-pattern declared here, and not an entry of the
    /// shipped library.
    #[must_use]
    pub fn is_free(&self, name: &str) -> bool {
        !name.is_empty()
            && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
            && !BUILTIN_ATOM_NAMES.contains(&name)
            && self.id_of(name).is_none()
            && self.let_of(name).is_none()
            && crate::library::id_of(name).is_none()
            && crate::library::let_of(name).is_none()
    }

    /// `name` checked as a declaration name: alphanumerics and underscores,
    /// not a built-in atom's, and not already a shape or kind here.
    fn check_name(&self, decl: &str, name: &str) -> Result<(), ShapeError> {
        let err = |msg: &str| ShapeError { decl: decl.to_string(), msg: msg.to_string() };
        if name.is_empty() || !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
            return Err(err("a name is alphanumerics and underscores"));
        }
        if BUILTIN_ATOM_NAMES.contains(&name) {
            return Err(err("that name belongs to a built-in atom; pick another"));
        }
        if self.id_of(name).is_some() {
            return Err(err("a shape or kind with that name exists"));
        }
        Ok(())
    }

    /// The next id for a declared shape or kind.
    fn take_id(&mut self, decl: &str) -> Result<u8, ShapeError> {
        if self.next_id >= LIBRARY_ID_BASE {
            return Err(ShapeError {
                decl: decl.to_string(),
                msg: format!("at most {LIBRARY_ID_BASE} shapes and kinds may be declared"),
            });
        }
        let id = self.next_id;
        self.next_id += 1;
        Ok(id)
    }

    /// Declare a shape from `name = ` followed by a backtick byte-pattern.
    ///
    /// # Errors
    ///
    /// Rejects a malformed declaration, an unparseable byte-pattern, an
    /// unbounded one, a duplicate name, a name held by a built-in atom, or
    /// more shapes and kinds than the ids below [`LIBRARY_ID_BASE`] hold.
    pub fn declare(&mut self, decl: &str, precedence: Precedence) -> Result<u8, ShapeError> {
        let err = |msg: &str| ShapeError { decl: decl.to_string(), msg: msg.to_string() };
        let (name, rest) = decl.split_once('=').ok_or_else(|| err("expected `name = `pattern`"))?;
        let name = name.trim();
        self.check_name(decl, name)?;
        let body = rest.trim();
        let inner = body
            .strip_prefix('`')
            .and_then(|b| b.strip_suffix('`'))
            .ok_or_else(|| err("the pattern must be enclosed in backticks"))?;
        let pat = parse_bytepat(inner.as_bytes()).map_err(|m| err(&m))?;
        let window = pat.max_len().ok_or_else(|| {
            err("the pattern must be bounded; `*`, `+` and `{m,}` have no fixed length")
        })?;
        if window == 0 {
            return Err(err("the pattern matches nothing, so it would never end a token"));
        }
        let id = self.take_id(decl)?;
        self.shapes.push(TokenShape { name: name.to_string(), pat, window, precedence, id, guard: None });
        Ok(id)
    }

    /// Add a shape the shipped library defines, under the id its entry
    /// fixes and with its guard.
    pub(crate) fn push_library_shape(&mut self, shape: TokenShape) {
        self.shapes.push(shape);
    }

    /// Declare a kind from `name = ` followed by a pattern over the stream,
    /// parsed against the declarations so far.
    ///
    /// # Errors
    ///
    /// Rejects a malformed declaration, a pattern that does not parse, a
    /// duplicate name, a name held by a built-in atom, or more shapes and
    /// kinds than the ids below [`LIBRARY_ID_BASE`] hold.
    pub fn declare_kind(&mut self, decl: &str) -> Result<u8, ShapeError> {
        let err = |msg: &str| ShapeError { decl: decl.to_string(), msg: msg.to_string() };
        let (name, rest) = decl.split_once('=').ok_or_else(|| err("expected `name = pattern`"))?;
        let name = name.trim();
        self.check_name(decl, name)?;
        let pattern = crate::parser::parse_with_shapes(rest.trim(), self)
            .map_err(|e| err(&format!("pattern error at byte {}: {}", e.pos, e.msg)))?;
        let id = self.take_id(decl)?;
        self.kinds.push(PatternKind { name: name.to_string(), id, pattern, guard: None });
        Ok(id)
    }

    /// Add a kind the shipped library defines from a pattern, under the id
    /// its entry fixes and with its guard.
    pub(crate) fn push_library_kind(&mut self, kind: PatternKind) {
        self.kinds.push(kind);
    }

    /// Declare a named sub-pattern from `name = ` followed by a pattern,
    /// parsed against the declarations so far; a later declaration of the
    /// same name shadows this one.
    ///
    /// # Errors
    ///
    /// Rejects a malformed declaration, a pattern that does not parse, or a
    /// name held by a built-in atom or by a shape or kind here.
    pub fn declare_let(&mut self, decl: &str) -> Result<(), ShapeError> {
        let err = |msg: &str| ShapeError { decl: decl.to_string(), msg: msg.to_string() };
        let (name, rest) = decl.split_once('=').ok_or_else(|| err("expected `name = pattern`"))?;
        let name = name.trim();
        self.check_name(decl, name)?;
        let pattern = crate::parser::parse_with_shapes(rest.trim(), self)
            .map_err(|e| err(&format!("pattern error at byte {}: {}", e.pos, e.msg)))?;
        self.lets.push((name.to_string(), pattern));
        Ok(())
    }

    /// Record a `test` line from `NAME accepts "text"... reads "span" in
    /// "text"... rejects "text"...`: the name, then any number of clauses,
    /// `accepts` and `rejects` each followed by the double-quoted texts it
    /// covers and `reads` by one span and the one text it is read out of, at
    /// least one text in all. A text reads `\"`, `\\`, `\n` and `\t`.
    ///
    /// # Errors
    ///
    /// Rejects a malformed line: a name that is not alphanumerics and
    /// underscores, a word that is no keyword, a text before any keyword, a
    /// `reads` whose span stands without `in` or without the text after it,
    /// an unterminated text or an unknown escape, or no text at all.
    pub fn declare_test(&mut self, decl: &str, line: usize) -> Result<(), ShapeError> {
        let err = |msg: &str| ShapeError { decl: decl.to_string(), msg: msg.to_string() };
        let (name, rest) = decl.split_once(char::is_whitespace).unwrap_or((decl, ""));
        if name.is_empty() || !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
            return Err(err("expected `test NAME accepts \"text\"... rejects \"text\"...`"));
        }
        /// Which clause the texts that follow belong to.
        enum Side {
            Accepts,
            Rejects,
            /// `reads`, waiting on the span it names.
            Reads,
            /// `reads "span"`, waiting on the `in` that introduces the text.
            ReadsSpan(String),
            /// `reads "span" in`, waiting on the text the span is read from.
            ReadsIn(String),
        }
        let mut accepts = Vec::new();
        let mut rejects = Vec::new();
        let mut reads: Vec<(String, String)> = Vec::new();
        let mut side: Option<Side> = None;
        let mut i = 0;
        while i < rest.len() {
            let c = rest.as_bytes()[i];
            if c.is_ascii_whitespace() {
                i += 1;
            } else if c == b'"' {
                let (text, next) = read_test_text(rest, i).map_err(|m| err(&m))?;
                side = match side.take() {
                    Some(Side::Accepts) => {
                        accepts.push(text);
                        Some(Side::Accepts)
                    }
                    Some(Side::Rejects) => {
                        rejects.push(text);
                        Some(Side::Rejects)
                    }
                    // The span, waiting on the `in "text"` that says where it
                    // is read from.
                    Some(Side::Reads) => Some(Side::ReadsSpan(text)),
                    Some(Side::ReadsSpan(span)) => {
                        return Err(err(&format!(
                            "`in` stands between the span {span:?} and the text it is read from"
                        )));
                    }
                    Some(Side::ReadsIn(span)) => {
                        reads.push((span, text));
                        // A second `reads` repeats the keyword, so that
                        // `reads "a" in "b" "c" in "d"` is a mistake rather
                        // than a second pair read by position.
                        None
                    }
                    None => return Err(err("a text follows accepts, reads or rejects")),
                };
                i = next;
            } else {
                let end = rest[i..].find(char::is_whitespace).map_or(rest.len(), |n| i + n);
                let word = &rest[i..end];
                // `in` stands between a span and its text and takes no texts
                // of its own, so it leaves the side as it is.
                if word == "in" {
                    let Some(Side::ReadsSpan(span)) = side.take() else {
                        return Err(err("`in` follows the span a `reads` names"));
                    };
                    side = Some(Side::ReadsIn(span));
                    i = end;
                    continue;
                }
                side = Some(match word {
                    "accepts" => Side::Accepts,
                    "rejects" => Side::Rejects,
                    "reads" => Side::Reads,
                    other => {
                        return Err(err(&format!(
                            "expected accepts, reads, rejects or a quoted text, found `{other}`"
                        )));
                    }
                });
                i = end;
            }
        }
        if let Some(Side::ReadsSpan(span) | Side::ReadsIn(span)) = side {
            return Err(err(&format!("reads {span:?} names no text to read it from")));
        }
        if accepts.is_empty() && rejects.is_empty() && reads.is_empty() {
            return Err(err("a test names at least one text it accepts, reads or rejects"));
        }
        self.tests.push(LibTest {
            file: None,
            line,
            name: name.to_string(),
            accepts,
            reads,
            rejects,
        });
        Ok(())
    }

    /// Run every `test` line declared here against what the set finally
    /// declares, so a test stands anywhere in its file. A name accepts a
    /// text when its match in the text is the whole of it, from the first
    /// significant token to the last, and rejects a text when it matches
    /// nowhere in it. The expectations not met, in line order; a failing
    /// `accepts` carries the lex of its text on a second line of its message.
    #[must_use]
    pub fn run_tests(&self) -> Vec<TestFailure> {
        let mut failures = Vec::new();
        for test in &self.tests {
            let fail = |msg: String| TestFailure {
                file: test.file.clone(),
                line: test.line,
                name: test.name.clone(),
                msg,
            };
            let pattern = match crate::parser::parse_with_shapes(&format!("\\{{{}}}", test.name), self) {
                Ok(pattern) => pattern,
                Err(e) => {
                    failures.push(fail(format!("cannot be tested: {}", e.msg)));
                    continue;
                }
            };
            let lexed_under = self.with_library_shapes(&pattern.library_kinds());
            for text in &test.accepts {
                let input = text.as_bytes();
                let found = crate::engine::scan_with_shapes(&pattern, input, self);
                let whole = significant_extent(input, &lexed_under);
                // The lex of the text stands under the failure, because a text
                // that will not be taken whole usually lexed into something
                // other than what the writer of the line had in mind, and the
                // kinds say so where the bytes do not.
                let lexing = || {
                    let explainer = crate::explain::Explainer::new(&pattern, input, self);
                    let toks: Vec<String> = explainer
                        .tokens(0..input.len())
                        .iter()
                        .map(|(kind, text)| format!("{kind} {text:?}"))
                        .collect();
                    format!("\n  tokens: {}", toks.join(", "))
                };
                match (found.first(), whole) {
                    (Some(m), Some((first, last))) if m.start() == first && m.end() == last => {}
                    (Some(m), _) => failures.push(fail(format!(
                        "accepts {text:?}: matched only {:?} at {}..{}{}",
                        String::from_utf8_lossy(&input[m.range()]),
                        m.start,
                        m.end,
                        lexing()
                    ))),
                    (None, _) => failures.push(fail(format!("accepts {text:?}: no match{}", lexing()))),
                }
            }
            // A span read out of a larger text: the match must be exactly
            // those bytes, at the place the text puts them. A kind read from
            // the tokens around it takes part of a line and leaves the rest,
            // so `accepts` - which asks for the whole extent - is the wrong
            // question and this is the right one.
            for (span, text) in &test.reads {
                let input = text.as_bytes();
                let Some(at) = text.find(span.as_str()) else {
                    failures.push(fail(format!("reads {span:?}: {text:?} does not hold it")));
                    continue;
                };
                let want = at..at + span.len();
                match crate::engine::scan_with_shapes(&pattern, input, self).first() {
                    Some(m) if m.range() == want => {}
                    Some(m) => failures.push(fail(format!(
                        "reads {span:?} in {text:?}: read {:?} at {}..{}",
                        String::from_utf8_lossy(&input[m.range()]),
                        m.start,
                        m.end
                    ))),
                    None => failures.push(fail(format!("reads {span:?} in {text:?}: no match"))),
                }
            }
            for text in &test.rejects {
                let input = text.as_bytes();
                if let Some(m) = crate::engine::scan_with_shapes(&pattern, input, self).first() {
                    failures.push(fail(format!(
                        "rejects {text:?}: matched {:?} at {}..{}",
                        String::from_utf8_lossy(&input[m.range()]),
                        m.start,
                        m.end
                    )));
                }
            }
        }
        failures
    }

    /// Declare everything a pattern file says: `let NAME = PATTERN`, `kind
    /// NAME = PATTERN`, `shape NAME = `BYTES``, `shape-after NAME = `BYTES``
    /// and `test NAME accepts "text"... rejects "text"...` one a line, and a
    /// rule as a block, `rule NAME` over indented `field = value` lines, or
    /// as one line, `rule NAME [severity] "message" = PATTERN`, its other
    /// fields as `fix NAME = TEMPLATE`, `meta NAME KEY = VALUE`, `files NAME
    /// = GLOBS`, `unless NAME = PATTERN`, `record NAME = UNIT`, `record-start
    /// NAME = PATTERN` and `record-span NAME = PATTERN` lines below it. Blank
    /// lines and lines opening with `#` are skipped. A `fields` line gives a
    /// sub-pattern's fields, as [`Self::declare_fields`] reads it.
    ///
    /// # Errors
    ///
    /// The first line that is not a declaration, or whose declaration is
    /// refused, with its line number in the message.
    pub fn declare_text(&mut self, text: &str) -> Result<(), ShapeError> {
        self.declare_lines(text, None)
    }

    /// [`Self::declare_text`], collecting into `members` the named patterns
    /// a set built from the file holds, in order: each `let` and `rule`
    /// under its name, and each line that is no declaration, parsed as a
    /// pattern against the declarations so far, under its line number. A
    /// line that is no declaration is refused where no members are
    /// collected.
    ///
    /// # Errors
    ///
    /// As [`Self::declare_text`].
    pub fn declare_lines(
        &mut self,
        text: &str,
        mut members: Option<&mut Vec<(String, Pattern)>>,
    ) -> Result<(), ShapeError> {
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let n = i + 1;
            let line = lines[i].trim();
            i += 1;
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |mut e: ShapeError| {
                e.msg = format!("line {n}: {}", e.msg);
                e
            };
            let (keyword, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let rest = rest.trim();
            match keyword {
                "let" => {
                    self.declare_let(rest).map_err(at)?;
                    if let Some(members) = members.as_mut() {
                        let name = rest.split_once('=').map_or(rest, |(name, _)| name).trim();
                        let pattern = self.let_of(name).cloned().ok_or_else(|| {
                            at(ShapeError { decl: line.to_string(), msg: "the let declared nothing".to_string() })
                        })?;
                        members.push((name.to_string(), pattern));
                    }
                }
                "kind" => {
                    self.declare_kind(rest).map_err(at)?;
                }
                "shape" => {
                    self.declare(rest, Precedence::Before).map_err(at)?;
                }
                "shape-after" => {
                    self.declare(rest, Precedence::After).map_err(at)?;
                }
                "test" => self.declare_test(rest, n).map_err(at)?,
                "fields" => self.declare_fields(rest).map_err(at)?,
                "rule" => {
                    let is_block = !rest.is_empty() && rest.chars().all(|c| c == '_' || c.is_ascii_alphanumeric());
                    let rule = if is_block {
                        // The block's field lines: every indented line that
                        // follows, blank lines and comments among them skipped,
                        // up to the next line that opens at the margin.
                        let mut fields: Vec<(usize, &str)> = Vec::new();
                        while i < lines.len() {
                            let raw = lines[i];
                            let body = raw.trim();
                            if body.is_empty() || body.starts_with('#') {
                                i += 1;
                                continue;
                            }
                            if !raw.starts_with([' ', '\t']) {
                                break;
                            }
                            fields.push((i + 1, body));
                            i += 1;
                        }
                        self.rule_block(rest, n, &fields)?
                    } else {
                        self.rule_line(rest, n).map_err(at)?
                    };
                    let name = rule.name.clone();
                    let pattern = rule.pattern.clone();
                    self.add_rule(rule, line).map_err(at)?;
                    if let Some(members) = members.as_mut() {
                        members.push((name, pattern));
                    }
                }
                "fix" | "meta" | "files" | "unless" | "record" | "record-start" | "record-span" => {
                    self.rule_field_line(keyword, rest).map_err(at)?;
                }
                other => match members.as_mut() {
                    Some(members) => {
                        let pattern = crate::parser::parse_with_shapes(line, self).map_err(|e| {
                            at(ShapeError {
                                decl: line.to_string(),
                                msg: format!("pattern error at byte {}: {}", e.pos, e.msg),
                            })
                        })?;
                        members.push((n.to_string(), pattern));
                    }
                    None => {
                        return Err(at(ShapeError {
                            decl: line.to_string(),
                            msg: format!(
                                "unknown declaration `{other}`; a line opens with let, kind, shape, shape-after, test or rule"
                            ),
                        }));
                    }
                },
            }
        }
        Ok(())
    }

    /// A rule from its block: the name on the `rule NAME` line and the
    /// `field = value` lines under it, each with its line number. The
    /// pattern is read first, whichever line holds it, since the other
    /// fields are checked against its registers.
    fn rule_block(&self, name: &str, line: usize, fields: &[(usize, &str)]) -> Result<Rule, ShapeError> {
        let mut split: Vec<(usize, &str, &str)> = Vec::with_capacity(fields.len());
        for &(n, field_line) in fields {
            let (field, value) = field_line.split_once('=').ok_or_else(|| ShapeError {
                decl: field_line.to_string(),
                msg: format!("line {n}: a rule's field line is `field = value`"),
            })?;
            split.push((n, field.trim(), value.trim()));
        }
        let Some(&(n, _, source)) = split.iter().find(|(_, field, _)| *field == "pattern") else {
            return Err(ShapeError {
                decl: format!("rule {name}"),
                msg: format!("line {line}: rule {name} has no `pattern = PATTERN` line"),
            });
        };
        let pattern = self.rule_pattern(source).map_err(|mut e| {
            e.msg = format!("line {n}: {}", e.msg);
            e
        })?;
        let mut rule = Rule {
            file: None,
            line,
            name: name.to_string(),
            pattern,
            source: source.to_string(),
            message: String::new(),
            severity: Severity::Warning,
            fix: None,
            files: Vec::new(),
            unless: Vec::new(),
            record: None,
            meta: Vec::new(),
        };
        for &(n, field, value) in &split {
            let at = |mut e: ShapeError| {
                e.msg = format!("line {n}: {}", e.msg);
                e
            };
            match field {
                "pattern" => {}
                "message" => rule.message = value.to_string(),
                "severity" => {
                    rule.severity = Severity::parse(value).ok_or_else(|| {
                        at(ShapeError {
                            decl: format!("{field} = {value}"),
                            msg: format!("{value:?} is not a severity; write error, warning or note"),
                        })
                    })?;
                }
                _ => self.rule_field(&mut rule, field, value).map_err(at)?,
            }
        }
        if rule.message.is_empty() {
            return Err(ShapeError {
                decl: format!("rule {name}"),
                msg: format!("line {line}: rule {name} has no `message = TEXT` line"),
            });
        }
        Ok(rule)
    }

    /// A rule from one line: `NAME [severity] "message" = PATTERN`.
    fn rule_line(&self, decl: &str, line: usize) -> Result<Rule, ShapeError> {
        let err = |msg: &str| ShapeError { decl: decl.to_string(), msg: msg.to_string() };
        let form = "expected `rule NAME [error|warning|note] \"message\" = PATTERN`, or `rule NAME` over indented `field = value` lines";
        let (name, rest) = decl.split_once(char::is_whitespace).ok_or_else(|| err(form))?;
        let rest = rest.trim_start();
        let (severity, rest) = match rest.split_once(char::is_whitespace) {
            Some((word, after)) if !word.starts_with('"') => {
                let severity = Severity::parse(word)
                    .ok_or_else(|| err(&format!("{word:?} is not a severity; write error, warning or note")))?;
                (severity, after.trim_start())
            }
            _ => (Severity::Warning, rest),
        };
        if !rest.starts_with('"') {
            return Err(err(form));
        }
        let (message, next) = read_test_text(rest, 0).map_err(|m| err(&m))?;
        let rest = rest[next..].trim_start();
        let source = rest.strip_prefix('=').ok_or_else(|| err(form))?.trim();
        if source.is_empty() {
            return Err(err("the rule has no pattern after `=`"));
        }
        let pattern = self.rule_pattern(source)?;
        Ok(Rule {
            file: None,
            line,
            name: name.to_string(),
            pattern,
            source: source.to_string(),
            message,
            severity,
            fix: None,
            files: Vec::new(),
            unless: Vec::new(),
            record: None,
            meta: Vec::new(),
        })
    }

    /// A rule's pattern, parsed against the declarations so far.
    fn rule_pattern(&self, source: &str) -> Result<Pattern, ShapeError> {
        crate::parser::parse_with_shapes(source, self).map_err(|e| ShapeError {
            decl: source.to_string(),
            msg: format!("pattern error at byte {}: {}", e.pos, e.msg),
        })
    }

    /// A field of a rule beyond its pattern, message and severity, from a
    /// block line or a `FIELD NAME = VALUE` line: `fix`, `files`, `unless`,
    /// `record`, `record-start`, `record-span` or `meta.KEY`.
    fn rule_field(&self, rule: &mut Rule, field: &str, value: &str) -> Result<(), ShapeError> {
        let err = |msg: String| ShapeError { decl: format!("{field} = {value}"), msg };
        match field {
            "fix" => rule.fix = Some(value.to_string()),
            "files" => {
                rule.files = value.split(',').map(str::trim).filter(|g| !g.is_empty()).map(str::to_string).collect();
                if rule.files.is_empty() {
                    return Err(err("files names at least one glob, as -g takes them".to_string()));
                }
            }
            "unless" => rule.unless.push(self.rule_pattern(value)?),
            "record" => {
                rule.record = Some(crate::records::RecordUnit::parse(value).map_err(err)?);
            }
            "record-start" => rule.record = Some(crate::records::RecordUnit::Start(self.rule_pattern(value)?)),
            "record-span" => rule.record = Some(crate::records::RecordUnit::Span(self.rule_pattern(value)?)),
            _ => match field.strip_prefix("meta.") {
                Some(key) if !key.is_empty() && key.chars().all(|c| c == '_' || c == '-' || c.is_ascii_alphanumeric()) => {
                    rule.meta.push((key.to_string(), value.to_string()));
                }
                Some(key) => return Err(err(format!("{key:?} is not a metadata key; a key is alphanumerics, underscores and hyphens"))),
                None => {
                    return Err(err(format!(
                        "{field:?} is not a rule field; a rule takes pattern, message, severity, fix, files, unless, record, record-start, record-span and meta.KEY"
                    )));
                }
            },
        }
        Ok(())
    }

    /// A `FIELD NAME ... = VALUE` line below a one-line rule: the field set
    /// on the rule declared under NAME above it; `meta NAME KEY = VALUE`
    /// carries the key before the `=`.
    fn rule_field_line(&mut self, field: &str, rest: &str) -> Result<(), ShapeError> {
        let err = |msg: String| ShapeError { decl: format!("{field} {rest}"), msg };
        let (name, rest) = rest
            .split_once(char::is_whitespace)
            .ok_or_else(|| err(format!("expected `{field} NAME = VALUE`")))?;
        let rest = rest.trim_start();
        let (key, value) = match field {
            "meta" => {
                let (key, value) = rest.split_once('=').ok_or_else(|| err("expected `meta NAME KEY = VALUE`".to_string()))?;
                (format!("meta.{}", key.trim()), value.trim())
            }
            _ => {
                let value = rest.strip_prefix('=').ok_or_else(|| err(format!("expected `{field} NAME = VALUE`")))?;
                (field.to_string(), value.trim())
            }
        };
        let position = self
            .rules
            .iter()
            .position(|r| r.name == name)
            .ok_or_else(|| err(format!("no rule named {name} is declared above this line")))?;
        let mut rule = self.rules[position].clone();
        self.rule_field(&mut rule, &key, value)?;
        if let Some(fix) = &rule.fix {
            crate::rewrite::Template::parse(fix, &rule.pattern.capture_names()).map_err(|e| {
                err(format!("fix error at byte {}: {}", e.pos, e.msg))
            })?;
        }
        self.rules[position] = rule;
        Ok(())
    }

    /// Register `rule`: its name checked as a declaration's and against the
    /// rules so far, its message and fix checked as templates over its
    /// registers, and the rule made a sub-pattern under its name.
    fn add_rule(&mut self, rule: Rule, decl: &str) -> Result<(), ShapeError> {
        let err = |msg: String| ShapeError { decl: decl.to_string(), msg };
        self.check_name(decl, &rule.name)?;
        if self.rules.iter().any(|r| r.name == rule.name) {
            return Err(err(format!("rule {} is declared twice", rule.name)));
        }
        let bound = rule.pattern.capture_names();
        crate::rewrite::Template::parse_report(&rule.message, &bound)
            .map_err(|e| err(format!("message error at byte {}: {}", e.pos, e.msg)))?;
        if let Some(fix) = &rule.fix {
            crate::rewrite::Template::parse(fix, &bound)
                .map_err(|e| err(format!("fix error at byte {}: {}", e.pos, e.msg)))?;
        }
        self.lets.push((rule.name.clone(), rule.pattern.clone()));
        self.rules.push(rule);
        Ok(())
    }

    /// This set with the library shapes and kinds `ids` name added, for a
    /// lex that has to produce the tokens a pattern naming them reads.
    #[must_use]
    pub(crate) fn with_library_shapes(&self, ids: &[u8]) -> ShapeSet {
        let mut set = self.clone();
        for &id in ids {
            if set.shapes.iter().any(|s| s.id == id) || set.kinds.iter().any(|k| k.id == id) {
                continue;
            }
            if let Some(shape) = crate::library::shape_of(id) {
                set.shapes.push(shape.clone());
            } else if let Some(kind) = crate::library::kind_of(id) {
                set.kinds.push(kind.clone());
            }
        }
        set
    }

    /// The longest shape matching at `i`, as `(id, end)`, among those with the
    /// given precedence whose guard accepts the bytes. A longer span wins;
    /// equal spans go to the earlier declaration.
    #[must_use]
    pub fn longest_at(&self, input: &[u8], i: usize, when: Precedence) -> Option<(u8, usize)> {
        let mut best: Option<(u8, usize)> = None;
        for s in &self.shapes {
            if s.precedence != when {
                continue;
            }
            // The window is the shape's own maximum match length, bounding the
            // slice each position reads. A guarded shape takes the longest
            // end its guard accepts: the longest the pattern reaches may run
            // into what follows, and a shorter end can still be the token.
            let hi = (i + s.window).min(input.len());
            let len = match s.guard {
                None => match s.pat.longest_prefix(&input[i..hi]) {
                    Some(len) => len,
                    None => continue,
                },
                Some(guard) => {
                    let ends = s.pat.prefix_ends(&input[i..hi]);
                    match ends.iter().rev().find(|&&len| len > 0 && guard(&input[i..i + len])) {
                        Some(&len) => len,
                        None => continue,
                    }
                }
            };
            if len == 0 {
                continue;
            }
            if best.is_none_or(|(_, e)| i + len > e) {
                best = Some((s.id, i + len));
            }
        }
        best
    }
}

/// Names held by built-in `\{name}` atoms.
const BUILTIN_ATOM_NAMES: &[&str] = &[
    "number",
    "word",
    "quoted",
    "ip",
    "url",
    "email",
    "timestamp",
    "punct",
    "whitespace",
    "version",
    "uuid",
    "mac",
    "hexcolor",
    "cidr",
    "bytesize",
    "percent",
    "money",
    "hash",
    "hashdigest",
    "duration",
    "path",
    "jwt",
    "creditcard",
    "card",
    "base64",
    "b64",
    "geo",
    "coord",
    "phone",
    "tel",
    "quantity",
    "qty",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_and_matches_a_bounded_shape() {
        let mut set = ShapeSet::new();
        let id = set.declare("order = `[A-Z]{3}-[0-9]{4}`", Precedence::Before).expect("declares");
        assert_eq!(id, 0);
        assert_eq!(set.id_of("order"), Some(0));
        assert_eq!(set.name_of(0), Some("order"));
        let input = b"ABC-1234 rest";
        assert_eq!(set.longest_at(input, 0, Precedence::Before), Some((0, 8)));
        assert_eq!(set.longest_at(input, 9, Precedence::Before), None);
    }

    #[test]
    fn an_unbounded_shape_is_rejected_at_declaration() {
        let mut set = ShapeSet::new();
        let e = set.declare("wild = `[A-Z]+`", Precedence::Before).unwrap_err();
        assert!(e.msg.contains("bounded"), "got {:?}", e.msg);
        assert!(set.is_empty(), "a rejected shape is not stored");
    }

    #[test]
    fn malformed_declarations_are_rejected() {
        let mut set = ShapeSet::new();
        for bad in [
            "no equals sign",
            "name = missing backticks",
            "= `[A-Z]`",
            "bad name = `[A-Z]`",
            "x = ``",
        ] {
            assert!(set.declare(bad, Precedence::Before).is_err(), "should reject: {bad}");
        }
    }

    #[test]
    fn a_name_may_not_shadow_a_builtin_atom_or_repeat() {
        let mut set = ShapeSet::new();
        assert!(set.declare("ip = `[0-9]{3}`", Precedence::Before).is_err());
        set.declare("tag = `[A-Z]{2}`", Precedence::Before).expect("declares");
        assert!(set.declare("tag = `[a-z]{2}`", Precedence::Before).is_err());
    }

    #[test]
    fn the_longest_shape_wins_and_ties_go_to_declaration_order() {
        let mut set = ShapeSet::new();
        set.declare("short = `[A-Z]{2}`", Precedence::Before).expect("declares");
        set.declare("long = `[A-Z]{4}`", Precedence::Before).expect("declares");
        assert_eq!(set.longest_at(b"ABCD", 0, Precedence::Before), Some((1, 4)));
        assert_eq!(set.longest_at(b"AB c", 0, Precedence::Before), Some((0, 2)));
    }

    #[test]
    fn a_pattern_file_declares_sub_patterns_kinds_and_shapes() {
        let mut set = ShapeSet::new();
        set.declare_text(
            "# a file\nlet kv = \\W \"=\" \\N\nkind pair = \\{kv}\nshape tag = `[A-Z]{3}`\n\nshape-after code = `[0-9]{2}`\n",
        )
        .expect("declares");
        assert!(set.let_of("kv").is_some());
        assert_eq!(set.id_of("pair"), Some(0));
        assert_eq!(set.id_of("tag"), Some(1));
        assert_eq!(set.id_of("code"), Some(2));
        assert_eq!(set.name_of(0), Some("pair"));
        assert_eq!(set.pattern_kinds().len(), 1);
        assert!(!set.is_empty());
        let e = set.declare_text("shrug x = `a`").unwrap_err();
        assert!(e.msg.contains("line 1"), "{e}");
        let e = set.declare_text("let tag = \\W").unwrap_err();
        assert!(e.msg.contains("exists"), "{e}");
        set.declare_text("let kv = \\N").expect("a later let is accepted");
        assert!(matches!(set.let_of("kv"), Some(Pattern::Atom(_))), "a later let shadows an earlier one");
    }

    #[test]
    fn test_lines_state_what_a_name_matches_and_name_their_line_when_it_does_not() {
        let mut set = ShapeSet::new();
        set.declare_text(
            "let rhs = \\N | \\Q\ntest assign accepts \"x = 1\" \"y = \\\"bob\\\"\" rejects \"x == 1\"\nkind assign = \\W \"=\" \\{rhs}\ntest rhs accepts \"42\" rejects \"forty\"\n",
        )
        .expect("declares");
        assert_eq!(set.tests().len(), 2);
        assert_eq!(set.tests()[0].line, 2);
        assert_eq!(set.tests()[0].accepts, vec!["x = 1".to_string(), "y = \"bob\"".to_string()]);
        assert_eq!(set.tests()[0].rejects, vec!["x == 1".to_string()]);
        assert!(set.run_tests().is_empty(), "{:?}", set.run_tests());

        let mut set = ShapeSet::new();
        set.declare_text(
            "let rhs = \\N\ntest rhs accepts \"42\" \"\\\"bob\\\"\" rejects \"4 2\"\ntest nosuch accepts \"x\"\ntest iban accepts \"GB82 WEST 1234 5698 7654 32\" rejects \"GB82WEST12345698765433\"\n",
        )
        .expect("declares");
        let failures: Vec<String> = set.run_tests().iter().map(ToString::to_string).collect();
        assert_eq!(
            failures,
            vec![
                "line 2: rhs accepts \"\\\"bob\\\"\": no match\n  tokens: quoted \"\\\"bob\\\"\"".to_string(),
                "line 2: rhs rejects \"4 2\": matched \"4\" at 0..1".to_string(),
                "line 3: nosuch cannot be tested: unknown named atom \\{nosuch}; it is neither a built-in, a declared shape, kind or sub-pattern, nor a library entry".to_string(),
            ]
        );

        let e = ShapeSet::new().declare_text("test rhs \"42\"").expect_err("a text before a keyword");
        assert!(e.msg.contains("follows accepts, reads or rejects"), "{e}");
        let e = ShapeSet::new().declare_text("test rhs accepts").expect_err("no text");
        assert!(e.msg.contains("at least one text"), "{e}");
        let e = ShapeSet::new().declare_text("test rhs accepts \"4").expect_err("unterminated");
        assert!(e.msg.contains("unterminated"), "{e}");
        let e = ShapeSet::new().declare_text("test rhs accepts \"\\q\"").expect_err("unknown escape");
        assert!(e.msg.contains("unknown escape \\q"), "{e}");
        let e = ShapeSet::new().declare_text("test rhs matches \"4\"").expect_err("unknown word");
        assert!(e.msg.contains("found `matches`"), "{e}");

        // A `reads` names one span and the one text it is read out of, and
        // both the keyword between them and the text after it are required:
        // a pair read by position would take `reads "a" "b" "c" "d"` for two
        // pairs, which is the reading the writer of such a line least meant.
        let mut set = ShapeSet::new();
        set.declare_text("test kelvin reads \"4.2K\" in \"cooled to 4.2K overnight\"\n").expect("declares");
        assert_eq!(set.tests()[0].reads, vec![("4.2K".to_string(), "cooled to 4.2K overnight".to_string())]);
        assert!(set.run_tests().is_empty(), "{:?}", set.run_tests());
        let e = ShapeSet::new().declare_text("test rhs reads \"4\"").expect_err("no text to read from");
        assert!(e.msg.contains("names no text to read it from"), "{e}");
        let e = ShapeSet::new().declare_text("test rhs reads \"4\" in").expect_err("no text after in");
        assert!(e.msg.contains("names no text to read it from"), "{e}");
        let e = ShapeSet::new().declare_text("test rhs reads \"4\" \"a 4 b\"").expect_err("no in");
        assert!(e.msg.contains("`in` stands between the span \"4\""), "{e}");
        let e = ShapeSet::new().declare_text("test rhs accepts \"4\" in \"a\"").expect_err("in alone");
        assert!(e.msg.contains("`in` follows the span a `reads` names"), "{e}");
    }

    #[test]
    fn a_guarded_shape_takes_the_longest_end_its_check_accepts() {
        let mut set = ShapeSet::new();
        set.declare("digits = `[0-9]{4,8}`", Precedence::Before).expect("declares");
        set.shapes[0].guard = Some(|text: &[u8]| text.len() % 2 == 1);
        assert_eq!(set.longest_at(b"12345678 x", 0, Precedence::Before), Some((0, 7)));
        assert_eq!(set.longest_at(b"1234 x", 0, Precedence::Before), None);
    }

    #[test]
    fn precedence_partitions_the_set() {
        let mut set = ShapeSet::new();
        set.declare("early = `[A-Z]{2}`", Precedence::Before).expect("declares");
        set.declare("late = `[0-9]{2}`", Precedence::After).expect("declares");
        assert_eq!(set.longest_at(b"AB", 0, Precedence::Before), Some((0, 2)));
        assert_eq!(set.longest_at(b"AB", 0, Precedence::After), None);
        assert_eq!(set.longest_at(b"12", 0, Precedence::After), Some((1, 2)));
        assert_eq!(set.longest_at(b"12", 0, Precedence::Before), None);
    }
}
