//! Custom atoms: the declarations a pattern reads by name as `\{name}`.
//!
//! A declaration is a byte shape the lexer tries beside its own recognizers,
//! a kind fused from every match of a pattern, or a named sub-pattern read in
//! place, each with `accepts` and `rejects` expectations beside it if the
//! caller gives any. They live in a `Trex.Library`. The session's is the one
//! `$TrexSession` holds, in the global scope of the runspace, and every
//! cmdlet reads it when no `-Library` is given, so a declaration made once
//! reaches every later command in the session; a runspace of its own, such
//! as ForEach-Object -Parallel starts, begins with none. Any other library
//! is passed with `-Library` by a script that must not depend on what the
//! session declared.
//!
//! A library keeps its declarations as the ordered list they were made in
//! and builds trex's `ShapeSet` from that list after each change, since a
//! set has no way to take a declaration back. A change the set refuses
//! leaves the list as it was.

use std::path::{Path, PathBuf};

use pwrs::prelude::*;

use crate::common::{arg_err, read_err, shape_err};

/// What a declaration declares.
#[psenum(name = "Trex.AtomForm")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AtomForm {
    /// A bounded byte-pattern the lexer tries before its built-in
    /// recognizers, so where it matches it wins.
    #[default]
    Shape,
    /// A bounded byte-pattern the lexer tries only where no built-in
    /// recognizer matched.
    ShapeAfter,
    /// A pattern whose every match, after the lex, becomes one token of
    /// this kind.
    Kind,
    /// A pattern read in place wherever the name appears.
    Pattern,
    /// A named pattern with a message and a severity, reported by a rule
    /// scan and read as a pattern elsewhere.
    Rule,
}

impl AtomForm {
    fn keyword(self) -> &'static str {
        match self {
            AtomForm::Shape => "shape",
            AtomForm::ShapeAfter => "shape-after",
            AtomForm::Kind => "kind",
            AtomForm::Pattern => "let",
            AtomForm::Rule => "rule",
        }
    }

    /// The form a declaration line's opening keyword names.
    fn of_keyword(keyword: &str) -> Option<AtomForm> {
        match keyword {
            "shape" => Some(AtomForm::Shape),
            "shape-after" => Some(AtomForm::ShapeAfter),
            "kind" => Some(AtomForm::Kind),
            "let" => Some(AtomForm::Pattern),
            "rule" => Some(AtomForm::Rule),
            _ => None,
        }
    }
}

/// One declared atom, as `Get-TrexAtom` lists it.
#[psclass(name = "Trex.Atom")]
#[derive(Clone, Default)]
pub struct TrexAtom {
    /// The name a pattern reads the atom by, as `\{name}`.
    pub name: String,
    /// What the declaration declares.
    pub form: AtomForm,
    /// The byte-pattern or pattern the declaration gives.
    pub definition: String,
    /// Where it was declared: `session`, `library`, the pattern file's
    /// path, or `shipped` for the library trex carries.
    pub source: String,
    /// For a shipped atom, what it matches.
    pub description: String,
}

/// The outcome of one `test` line: an atom's expectations, run against what
/// the set finally declares.
#[psclass(name = "Trex.AtomTest")]
#[derive(Clone, Default)]
pub struct TrexAtomTest {
    /// The atom the line tests.
    pub name: String,
    /// Whether every expectation on the line held.
    pub passed: bool,
    /// Texts the atom must match whole.
    pub accepts: Vec<String>,
    /// Texts the atom must match nowhere.
    pub rejects: Vec<String>,
    /// What trex reported for each expectation that failed.
    pub failures: Vec<String>,
    /// The pattern file the line is in, or empty for one declared here.
    pub file: String,
}

/// Where one declaration came from.
#[derive(Clone, Debug)]
enum Source {
    /// One line made here: its name, its form, and the declaration text,
    /// which holds its `test` line too when it has expectations.
    Line { name: String, form: AtomForm, definition: String, text: String },
    /// A pattern file, declared whole with a relative `@file` set in it read
    /// from beside it.
    File(PathBuf),
}

/// An ordered list of declarations and the set built from them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Declarations {
    sources: Vec<Source>,
    set: trex::ShapeSet,
}

impl Declarations {
    /// The set every declaration here builds.
    pub(crate) fn set(&self) -> &trex::ShapeSet {
        &self.set
    }

    fn build(sources: &[Source]) -> PsResult<trex::ShapeSet> {
        let mut set = trex::ShapeSet::new();
        for source in sources {
            match source {
                Source::Line { text, .. } => set.declare_text(text).map_err(|e| shape_err(&e))?,
                Source::File(path) => set.declare_file(path).map_err(|e| shape_err(&e))?,
            }
        }
        Ok(set)
    }

    /// Replace the list with `sources` when they build, leaving it as it was
    /// when they do not.
    fn commit(&mut self, sources: Vec<Source>) -> PsResult<()> {
        let set = Self::build(&sources)?;
        self.sources = sources;
        self.set = set;
        Ok(())
    }

    /// Whether a line made here already declares `name`.
    fn declares_line(&self, name: &str) -> bool {
        self.sources.iter().any(|s| matches!(s, Source::Line { name: n, .. } if n == name))
    }

    /// The list without the lines declaring `name`.
    fn without_line(&self, name: &str) -> Vec<Source> {
        self.sources.iter().filter(|s| !matches!(s, Source::Line { name: n, .. } if n == name)).cloned().collect()
    }

    /// Declare one line, replacing a line of the same name when `force` is
    /// set and refusing it otherwise.
    fn add_line(&mut self, name: &str, form: AtomForm, definition: &str, text: String, force: bool) -> PsResult<()> {
        if self.declares_line(name) && !force {
            return Err(arg_err(
                "TrexAtomExists",
                format!("an atom named {name} is already declared here; -Force replaces it"),
            ));
        }
        let mut next = self.without_line(name);
        next.push(Source::Line { name: name.to_string(), form, definition: definition.to_string(), text });
        self.commit(next)
    }

    /// Declare a pattern file, in place of an earlier import of the same
    /// file, so importing it again reads its current text.
    fn add_file(&mut self, path: PathBuf) -> PsResult<()> {
        self.add_files(vec![path])
    }

    /// Declare pattern files in order, each in place of an earlier import of
    /// the same file, building the set once.
    fn add_files(&mut self, paths: Vec<PathBuf>) -> PsResult<()> {
        let mut next: Vec<Source> =
            self.sources.iter().filter(|s| !matches!(s, Source::File(p) if paths.contains(p))).cloned().collect();
        next.extend(paths.into_iter().map(Source::File));
        self.commit(next)
    }

    /// Why `name`, which no line made here declares, cannot be removed by
    /// name: the imported file that declares it, or that nothing here does.
    fn not_removable(&self, name: &str) -> PsResult<PsError> {
        for source in &self.sources {
            if let Source::File(path) = source {
                let shown = path.display().to_string();
                if atoms_in_file(path, &shown)?.iter().any(|a| a.name == name) {
                    return Ok(arg_err(
                        "TrexAtomInFile",
                        format!("{name} is declared by {shown}; Unregister-TrexAtom -Path {shown} removes the file"),
                    ));
                }
            }
        }
        Ok(arg_err("TrexNoAtom", format!("no atom named {name} is declared here")))
    }

    /// Take out the lines declaring `names` and the imports of `files` as one
    /// change, so atoms that read one another go together whatever order
    /// they are named in, and give back why each name no line here declares
    /// and each file never imported stayed. Where what is left does not
    /// build, because an atom that stays reads one taken out, nothing is
    /// taken out and that is the error.
    fn remove(&mut self, names: &[String], files: &[PathBuf]) -> PsResult<Vec<PsError>> {
        let mut refused = Vec::new();
        for name in names {
            if !self.declares_line(name) {
                refused.push(self.not_removable(name)?);
            }
        }
        for path in files {
            if !self.sources.iter().any(|s| matches!(s, Source::File(p) if p == path)) {
                refused.push(arg_err("TrexNoFile", format!("{} was not imported here", path.display())));
            }
        }
        let next: Vec<Source> = self
            .sources
            .iter()
            .filter(|s| match s {
                Source::Line { name, .. } => !names.contains(name),
                Source::File(p) => !files.contains(p),
            })
            .cloned()
            .collect();
        if next.len() != self.sources.len() {
            self.commit(next)?;
        }
        Ok(refused)
    }

    fn clear(&mut self) {
        self.sources.clear();
        self.set = trex::ShapeSet::new();
    }

    /// Every atom declared here, in declaration order.
    fn atoms(&self, place: &str) -> PsResult<Vec<TrexAtom>> {
        let mut out = Vec::new();
        for source in &self.sources {
            match source {
                Source::Line { name, form, definition, .. } => out.push(TrexAtom {
                    name: name.clone(),
                    form: *form,
                    definition: definition.clone(),
                    source: place.to_string(),
                    description: String::new(),
                }),
                Source::File(path) => out.extend(atoms_in_file(path, &path.display().to_string())?),
            }
        }
        Ok(out)
    }

    /// The imported files, in import order.
    fn files(&self) -> Vec<String> {
        let mut out = Vec::new();
        for source in &self.sources {
            if let Source::File(path) = source {
                out.push(path.display().to_string());
            }
        }
        out
    }
}

/// The declarations a pattern file makes, read line by line as trex reads
/// them, for listing.
fn atoms_in_file(path: &Path, shown: &str) -> PsResult<Vec<TrexAtom>> {
    let bytes = std::fs::read(path).map_err(|e| read_err(shown, e))?;
    let text = String::from_utf8_lossy(&trex::encoding::decode(bytes)).into_owned();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let (keyword, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let Some(form) = AtomForm::of_keyword(keyword) else {
            continue;
        };
        let (name, definition) = name_and_definition(rest);
        out.push(TrexAtom { name, form, definition, source: shown.to_string(), description: String::new() });
    }
    Ok(out)
}

/// The name and the definition of a declaration's text after its keyword:
/// `name = definition`, or a rule's `name [severity] "message" = pattern`.
fn name_and_definition(rest: &str) -> (String, String) {
    let rest = rest.trim();
    let (head, definition) = match rest.split_once('=') {
        Some((head, definition)) => (head.trim(), definition.trim()),
        None => (rest, ""),
    };
    let name = head.split_whitespace().next().unwrap_or(head);
    (name.to_string(), definition.to_string())
}

/// The variable the session's atoms are held in: a `Trex.Library` in the
/// global scope of the runspace.
const SESSION_VARIABLE: &str = "global:TrexSession";

/// The session's library, `None` until a declaration makes one.
fn session_if_any(ps: &Pipeline<'_>) -> PsResult<Option<PsProxy<TrexLibrary>>> {
    let held = ps.variable(SESSION_VARIABLE)?;
    if held.is_null() {
        return Ok(None);
    }
    if held.type_name()? != "Trex.Library" {
        return Err(arg_err(
            "TrexSession",
            "$TrexSession holds the session's atoms and must be a Trex.Library; remove it and the next declaration makes a new one",
        ));
    }
    Ok(Some(PsProxy::from_ps(&held)?))
}

/// The session's library, made in `$TrexSession` when there is none yet.
fn session(ps: &Pipeline<'_>) -> PsResult<PsProxy<TrexLibrary>> {
    if let Some(held) = session_if_any(ps)? {
        return Ok(held);
    }
    let made = TrexLibrary::new()?.into_ps()?;
    ps.set_variable(SESSION_VARIABLE, &made)?;
    PsProxy::from_ps(&made)
}

/// The atoms a call reads with `files` imported for that call alone: a file
/// the library or the session already imported is read in its place, as
/// Import-TrexAtom reads a file again, and neither is changed.
pub(crate) fn atoms_with_files(
    ps: &Pipeline<'_>,
    library: &Option<PsProxy<TrexLibrary>>,
    files: &[PathBuf],
) -> PsResult<trex::ShapeSet> {
    let mut decls = match library {
        Some(lib) => lib.with(|l| l.decls.clone())?,
        None => match session_if_any(ps)? {
            Some(held) => held.with(|l| l.decls.clone())?,
            None => Declarations::default(),
        },
    };
    if !files.is_empty() {
        decls.add_files(files.to_vec())?;
    }
    Ok(decls.set)
}

/// The atoms a call reads: the library it was given, or else the session's.
pub(crate) fn atoms_for(ps: &Pipeline<'_>, library: &Option<PsProxy<TrexLibrary>>) -> PsResult<trex::ShapeSet> {
    match library {
        Some(lib) => lib.with(|l| l.decls.set().clone()),
        None => match session_if_any(ps)? {
            Some(held) => held.with(|l| l.decls.set().clone()),
            None => Ok(trex::ShapeSet::new()),
        },
    }
}

/// A set of atoms held in an object rather than the session, passed to a
/// cmdlet with `-Library`.
#[psclass(name = "Trex.Library", mode = proxy)]
pub struct TrexLibrary {
    /// The names declared in this library, in declaration order.
    pub names: Vec<String>,
    /// The pattern files imported into it.
    pub files: Vec<String>,
    #[psfield(skip)]
    pub(crate) decls: Declarations,
}

impl TrexLibrary {
    fn of(decls: Declarations) -> PsResult<Self> {
        let mut lib = TrexLibrary { names: Vec::new(), files: Vec::new(), decls };
        lib.refresh()?;
        Ok(lib)
    }

    fn refresh(&mut self) -> PsResult<()> {
        self.names = self.decls.atoms("library")?.into_iter().map(|a| a.name).collect();
        self.files = self.decls.files();
        Ok(())
    }
}

#[psmethods]
impl TrexLibrary {
    /// An empty library.
    pub fn new() -> PsResult<Self> {
        TrexLibrary::of(Declarations::default())
    }

    /// Declares one line as a pattern file writes it: `shape name =
    /// \`bytes\``, `kind name = pattern`, `let name = pattern`, or a `test`
    /// line.
    pub fn declare(&mut self, line: String) -> PsResult<()> {
        let mut next = self.decls.sources.clone();
        next.push(line_source(&line)?);
        self.decls.commit(next)?;
        self.refresh()
    }
}

/// A declaration line as a source, its name and form read from it.
fn line_source(line: &str) -> PsResult<Source> {
    let trimmed = line.trim();
    one_line("a declaration", trimmed)?;
    let (keyword, rest) = trimmed.split_once(char::is_whitespace).unwrap_or((trimmed, ""));
    let (name, definition) = name_and_definition(rest);
    let form = match (keyword, AtomForm::of_keyword(keyword)) {
        (_, Some(form)) => form,
        ("test", None) => {
            return Ok(Source::Line {
                name: format!("test {name}"),
                form: AtomForm::Pattern,
                definition: String::new(),
                text: trimmed.to_string(),
            });
        }
        (other, None) => {
            return Err(arg_err(
                "TrexDeclarationError",
                format!("{other:?} opens no declaration; a line opens with let, kind, shape, shape-after, test or rule"),
            ));
        }
    };
    Ok(Source::Line { name, form, definition, text: trimmed.to_string() })
}

/// `s` as a double-quoted text of a `test` line, which reads `\"`, `\\`,
/// `\n` and `\t`.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Refuses a text that a single line of a declaration cannot hold.
fn one_line(what: &str, text: &str) -> PsResult<()> {
    if text.contains(['\n', '\r']) {
        return Err(arg_err("TrexDeclarationError", format!("{what} is one line; this one holds a line break")));
    }
    Ok(())
}

/// Whether `name` matches the wildcard `pattern`, `*` and `?` read as a
/// PowerShell `-like` reads them, case-insensitively.
pub(crate) fn like(name: &str, pattern: &str) -> bool {
    fn go(n: &[char], p: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => go(n, &p[1..]) || (!n.is_empty() && go(&n[1..], p)),
            Some('?') => !n.is_empty() && go(&n[1..], &p[1..]),
            Some(pc) => match n.first() {
                Some(nc) => pc.to_lowercase().eq(nc.to_lowercase()) && go(&n[1..], &p[1..]),
                None => false,
            },
        }
    }
    let n: Vec<char> = name.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    go(&n, &p)
}

/// Declares an atom a pattern reads by name as `\{name}`: a byte shape, a
/// kind fused from a pattern, or a named sub-pattern.
///
/// With no -Library the atom is declared for this PowerShell session, and
/// every later cmdlet that takes a pattern reads it. -Accepts and -Rejects
/// add expectations that Test-TrexAtom checks.
///
/// # Examples
/// Register-TrexAtom ticket -Shape '[A-Z]{2,4}-\d{1,4}' -Accepts 'AB-12' -Rejects 'A-1'
/// Register-TrexAtom rhs -Pattern '\N | \Q'
/// Register-TrexAtom assign -Kind '\W "=" \{rhs}'
#[cmdlet(verb = "Register", noun = "TrexAtom", alias = "Register-TxAtom", default_parameter_set = "Shape", output = ["Trex.Atom"])]
#[derive(Default)]
pub struct RegisterTrexAtom {
    /// The name a pattern reads the atom by, as `\{name}`: letters, digits
    /// and underscores.
    #[param(mandatory, position = 0, validate_pattern = "^[A-Za-z0-9_]+$")]
    pub name: String,
    /// A bounded byte-pattern the lexer tries at each position before its
    /// built-in recognizers, such as `[A-Z]{2,4}-\d{1,4}`; an unbounded `*`,
    /// `+` or `{m,}` is refused.
    #[param(mandatory, position = 1, set = "Shape")]
    pub shape: String,
    /// Tries the shape only where no built-in recognizer matched.
    #[param(set = "Shape")]
    pub after: bool,
    /// A pattern whose every match, after the lex, becomes one token of
    /// this kind.
    #[param(mandatory, set = "Kind")]
    pub kind: String,
    /// A pattern read in place wherever `\{name}` appears.
    #[param(mandatory, set = "Pattern")]
    pub pattern: String,
    /// Texts the atom must match whole, from the first token to the last.
    #[param]
    pub accepts: Vec<String>,
    /// Texts the atom must match nowhere.
    #[param]
    pub rejects: Vec<String>,
    /// The library to declare into, in place of the session.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    /// Replaces an atom of the same name declared here.
    #[param]
    pub force: bool,
    /// Writes the declared atom.
    #[param]
    pub pass_thru: bool,
}

impl Cmdlet for RegisterTrexAtom {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (form, definition) = if !self.kind.is_empty() {
            (AtomForm::Kind, self.kind.clone())
        } else if !self.pattern.is_empty() {
            (AtomForm::Pattern, self.pattern.clone())
        } else if self.after {
            (AtomForm::ShapeAfter, self.shape.clone())
        } else {
            (AtomForm::Shape, self.shape.clone())
        };
        one_line("a definition", &definition)?;
        let mut text = match form {
            AtomForm::Shape | AtomForm::ShapeAfter => {
                if definition.contains('`') {
                    return Err(arg_err(
                        "TrexDeclarationError",
                        "a shape's byte-pattern is written between backticks, so it cannot hold one",
                    ));
                }
                format!("{} {} = `{}`", form.keyword(), self.name, definition)
            }
            AtomForm::Kind | AtomForm::Pattern | AtomForm::Rule => {
                format!("{} {} = {}", form.keyword(), self.name, definition)
            }
        };
        for given in self.accepts.iter().chain(&self.rejects) {
            one_line("an -Accepts or -Rejects text", given)?;
        }
        if !self.accepts.is_empty() || !self.rejects.is_empty() {
            let mut test = format!("test {}", self.name);
            if !self.accepts.is_empty() {
                test.push_str(" accepts");
                for a in &self.accepts {
                    test.push(' ');
                    test.push_str(&quoted(a));
                }
            }
            if !self.rejects.is_empty() {
                test.push_str(" rejects");
                for r in &self.rejects {
                    test.push(' ');
                    test.push_str(&quoted(r));
                }
            }
            text.push('\n');
            text.push_str(&test);
        }
        let place = match &self.library {
            Some(lib) => {
                lib.with_mut(|l| -> PsResult<()> {
                    l.decls.add_line(&self.name, form, &definition, text, self.force)?;
                    l.refresh()
                })??;
                "library"
            }
            None => {
                session(ps)?.with_mut(|l| -> PsResult<()> {
                    l.decls.add_line(&self.name, form, &definition, text, self.force)?;
                    l.refresh()
                })??;
                "session"
            }
        };
        if self.pass_thru {
            ps.write(TrexAtom {
                name: self.name.clone(),
                form,
                definition,
                source: place.to_string(),
                description: String::new(),
            })?;
        }
        Ok(())
    }
}

/// Declares every atom the pattern files say: their `let`, `kind`, `shape`,
/// `shape-after`, `rule` and `test` lines, with a relative `@file` set read
/// from beside each file.
///
/// A file saved as UTF-8, UTF-16 or UTF-32 with a byte-order mark reads as
/// its text. Importing a file again reads its current text in place of the
/// earlier import.
///
/// # Examples
/// Import-TrexAtom ./defs.trex
/// Get-ChildItem ./atoms -Filter *.trex | Import-TrexAtom
#[cmdlet(verb = "Import", noun = "TrexAtom", alias = "Import-TxAtom", default_parameter_set = "Path", output = ["Trex.Atom"])]
#[derive(Default)]
pub struct ImportTrexAtom {
    /// Pattern files to import; wildcards expand.
    #[param(mandatory, position = 0, set = "Path")]
    pub path: Vec<String>,
    /// Pattern files to import, read as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The library to declare into, in place of the session.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    /// Writes each atom the files declare.
    #[param]
    pub pass_thru: bool,
}

impl Cmdlet for ImportTrexAtom {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (names, literal) =
            if self.literal_path.is_empty() { (&self.path, false) } else { (&self.literal_path, true) };
        for given in names {
            for resolved in ps.resolve_path(given, literal)? {
                let path = PathBuf::from(&resolved);
                let lib = match &self.library {
                    Some(lib) => PsProxy::from_ps(lib.object())?,
                    None => session(ps)?,
                };
                lib.with_mut(|l| -> PsResult<()> {
                    l.decls.add_file(path.clone())?;
                    l.refresh()
                })??;
                if self.pass_thru {
                    for atom in atoms_in_file(&path, &resolved)? {
                        ps.write(atom)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Lists the atoms declared for this session, in a library, or shipped with
/// trex, in declaration order.
///
/// # Examples
/// Get-TrexAtom
/// Get-TrexAtom -Shipped | Where-Object Name -like 'http*'
#[cmdlet(verb = "Get", noun = "TrexAtom", alias = "Get-TxAtom", output = ["Trex.Atom"])]
#[derive(Default)]
pub struct GetTrexAtom {
    /// Only atoms whose name matches; wildcards apply.
    #[param(position = 0)]
    pub name: Option<String>,
    /// Lists a library's atoms in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    /// Lists the atoms trex ships, which every pattern reads with no
    /// declaration.
    #[param]
    pub shipped: bool,
}

impl Cmdlet for GetTrexAtom {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let atoms: Vec<TrexAtom> = if self.shipped {
            trex::library::entries()
                .iter()
                .map(|e| TrexAtom {
                    name: e.name.to_string(),
                    form: if e.is_kind() { AtomForm::Kind } else { AtomForm::Pattern },
                    definition: String::new(),
                    source: "shipped".to_string(),
                    description: e.what.to_string(),
                })
                .collect()
        } else {
            match &self.library {
                Some(lib) => lib.with(|l| l.decls.atoms("library"))??,
                None => match session_if_any(ps)? {
                    Some(held) => held.with(|l| l.decls.atoms("session"))??,
                    None => Vec::new(),
                },
            }
        };
        for atom in atoms {
            if self.name.as_deref().is_none_or(|pattern| like(&atom.name, pattern)) {
                ps.write(atom)?;
            }
        }
        Ok(())
    }
}

/// Removes atoms from the session or a library: by name, by the pattern
/// file that declared them, or all of them.
///
/// Every name and file given, and every one piped in, is removed as one
/// change after the last, so atoms that read one another go together in any
/// order. An atom another one still reads stays, with an error naming it.
///
/// # Examples
/// Unregister-TrexAtom ticket
/// Get-TrexAtom t* | Unregister-TrexAtom
/// Unregister-TrexAtom -Path ./defs.trex
/// Unregister-TrexAtom -All
#[cmdlet(verb = "Unregister", noun = "TrexAtom", alias = "Unregister-TxAtom", supports_should_process, default_parameter_set = "Name")]
#[derive(Default)]
pub struct UnregisterTrexAtom {
    /// The names to remove.
    #[param(mandatory, position = 0, set = "Name", value_from_pipeline_by_property_name)]
    pub name: Vec<String>,
    /// Imported pattern files to remove, with every atom they declared.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Removes every atom and every imported file.
    #[param(mandatory, set = "All")]
    pub all: bool,
    /// The library to remove from, in place of the session.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    names_asked: Vec<String>,
    files_asked: Vec<PathBuf>,
}

impl UnregisterTrexAtom {
    /// The library a removal changes, `None` for a session no declaration
    /// has made yet, which holds nothing to remove.
    fn target(&self, ps: &Pipeline<'_>) -> PsResult<Option<PsProxy<TrexLibrary>>> {
        match &self.library {
            Some(lib) => Ok(Some(PsProxy::from_ps(lib.object())?)),
            None => session_if_any(ps),
        }
    }

    fn change(&self, ps: &Pipeline<'_>, f: impl FnOnce(&mut Declarations) -> PsResult<()>) -> PsResult<()> {
        match self.target(ps)? {
            Some(lib) => lib.with_mut(|l| -> PsResult<()> {
                f(&mut l.decls)?;
                l.refresh()
            })?,
            None => f(&mut Declarations::default()),
        }
    }
}

impl Cmdlet for UnregisterTrexAtom {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let place = if self.library.is_some() { "the library" } else { "the session" };
        if self.all {
            if ps.should_process(place, "Remove every atom")? {
                self.change(ps, |d| {
                    d.clear();
                    Ok(())
                })?;
            }
            return Ok(());
        }
        for given in &self.path {
            for resolved in ps.resolve_path(given, false)? {
                if ps.should_process(&resolved, "Remove the atoms this file declares")? {
                    self.files_asked.push(PathBuf::from(&resolved));
                }
            }
        }
        for name in &self.name {
            if ps.should_process(name, "Remove the atom")? {
                self.names_asked.push(name.clone());
            }
        }
        Ok(())
    }

    /// Every name and file asked for, from every record piped in, is removed
    /// as one change, so atoms that read one another go together.
    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        if self.names_asked.is_empty() && self.files_asked.is_empty() {
            return Ok(());
        }
        let (names, files) = (std::mem::take(&mut self.names_asked), std::mem::take(&mut self.files_asked));
        let mut refused = Vec::new();
        let outcome = self.change(ps, |d| {
            refused = d.remove(&names, &files)?;
            Ok(())
        });
        for e in &refused {
            ps.write_error(e)?;
        }
        if let Err(e) = outcome {
            ps.write_error(&e)?;
        }
        Ok(())
    }
}

/// Runs the `test` lines declared for the session, a library, or the atoms
/// trex ships, and writes one result per line.
///
/// A line's `accepts` texts must each be matched whole by the atom, and its
/// `rejects` texts matched nowhere. Register-TrexAtom writes a test line
/// from -Accepts and -Rejects; a pattern file writes its own.
///
/// # Examples
/// Test-TrexAtom
/// Test-TrexAtom -Shipped -Quiet
#[cmdlet(verb = "Test", noun = "TrexAtom", alias = "Test-TxAtom", output = ["Trex.AtomTest", "System.Boolean"])]
#[derive(Default)]
pub struct TestTrexAtom {
    /// Only the test lines for atoms whose name matches; wildcards apply.
    #[param(position = 0)]
    pub name: Option<String>,
    /// Tests a library's atoms in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    /// Tests the atoms trex ships.
    #[param]
    pub shipped: bool,
    /// Writes only whether every test line passed.
    #[param]
    pub quiet: bool,
}

impl Cmdlet for TestTrexAtom {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let set = if self.shipped {
            let mut shipped = trex::ShapeSet::new();
            for (n, line) in trex::library::TESTS.iter().enumerate() {
                shipped.declare_test(line, n + 1).map_err(|e| shape_err(&e))?;
            }
            shipped
        } else {
            atoms_for(ps, &self.library)?
        };
        let failures = set.run_tests();
        let mut all_passed = true;
        for test in set.tests() {
            if !self.name.as_deref().is_none_or(|pattern| like(&test.name, pattern)) {
                continue;
            }
            let failed: Vec<String> = failures
                .iter()
                .filter(|f| f.line == test.line && f.file == test.file && f.name == test.name)
                .map(|f| f.msg.clone())
                .collect();
            all_passed &= failed.is_empty();
            if !self.quiet {
                let file = match &test.file {
                    Some(path) => path.display().to_string(),
                    None => String::new(),
                };
                ps.write(TrexAtomTest {
                    name: test.name.clone(),
                    passed: failed.is_empty(),
                    accepts: test.accepts.clone(),
                    rejects: test.rejects.clone(),
                    failures: failed,
                    file,
                })?;
            }
        }
        if self.quiet {
            ps.write(all_passed)?;
        }
        Ok(())
    }
}

/// Makes a library: a set of atoms held in an object, passed to a cmdlet
/// with -Library, which reads it in place of the session's atoms.
///
/// # Examples
/// $lib = New-TrexLibrary ./defs.trex
/// $lib = New-TrexLibrary -Declaration 'shape ticket = `[A-Z]{2,4}-\d{1,4}`'
/// $lib = New-TrexLibrary -FromSession
#[cmdlet(verb = "New", noun = "TrexLibrary", alias = "New-TxLibrary", output = ["Trex.Library"])]
#[derive(Default)]
pub struct NewTrexLibrary {
    /// Pattern files to declare; wildcards expand.
    #[param(position = 0)]
    pub path: Vec<String>,
    /// Declaration lines, each as a pattern file writes one.
    #[param]
    pub declaration: Vec<String>,
    /// Starts from a copy of the session's atoms.
    #[param]
    pub from_session: bool,
}

impl Cmdlet for NewTrexLibrary {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let held = if self.from_session { session_if_any(ps)? } else { None };
        let mut decls = match held {
            Some(held) => held.with(|l| l.decls.clone())?,
            None => Declarations::default(),
        };
        let mut sources = decls.sources.clone();
        for given in &self.path {
            for resolved in ps.resolve_path(given, false)? {
                sources.push(Source::File(PathBuf::from(resolved)));
            }
        }
        for line in &self.declaration {
            sources.push(line_source(line)?);
        }
        decls.commit(sources)?;
        ps.write(TrexLibrary::of(decls)?)
    }
}
