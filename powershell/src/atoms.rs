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

use std::path::PathBuf;

use pwrs::prelude::*;
use trex::declarations::{Atom, AtomDecl, DeclareError, Form, Kept};

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
    fn of(form: Form) -> AtomForm {
        match form {
            Form::Shape => AtomForm::Shape,
            Form::ShapeAfter => AtomForm::ShapeAfter,
            Form::Kind => AtomForm::Kind,
            Form::Let => AtomForm::Pattern,
            Form::Rule => AtomForm::Rule,
        }
    }

    fn trex(self) -> Form {
        match self {
            AtomForm::Shape => Form::Shape,
            AtomForm::ShapeAfter => Form::ShapeAfter,
            AtomForm::Kind => Form::Kind,
            AtomForm::Pattern => Form::Let,
            AtomForm::Rule => Form::Rule,
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

/// Why a declaration was refused, in the cmdlets' terms.
fn declare_err(e: &DeclareError) -> PsError {
    match e {
        DeclareError::Exists { name } => {
            arg_err("TrexAtomExists", format!("an atom named {name} is already declared here; -Force replaces it"))
        }
        DeclareError::LineBreak { what: "an accepted or rejected text" } => {
            arg_err("TrexDeclarationError", "an -Accepts or -Rejects text is one line; this one holds a line break")
        }
        DeclareError::Read { file, reason } => read_err(&file.display().to_string(), reason),
        DeclareError::ReadDir { dir, reason } => read_err(&dir.display().to_string(), reason),
        DeclareError::NoPatternFile { .. } => arg_err("TrexNoPatternFile", e.to_string()),
        DeclareError::Refused(e) => shape_err(e),
        DeclareError::LineBreak { .. } | DeclareError::Backtick | DeclareError::NoKeyword { .. } => {
            arg_err("TrexDeclarationError", e.to_string())
        }
    }
}

/// Why a removal left a name or a file in place, in the cmdlets' terms.
fn kept_err(kept: &Kept) -> PsError {
    match kept {
        Kept::InFile { name, file } => {
            let shown = file.display();
            arg_err(
                "TrexAtomInFile",
                format!("{name} is declared by {shown}; Unregister-TrexAtom -Path {shown} removes the file"),
            )
        }
        Kept::NoName { name } => arg_err("TrexNoAtom", format!("no atom named {name} is declared here")),
        Kept::NoFile { file } => {
            arg_err("TrexNoFile", format!("nothing imported here is {} or under it", file.display()))
        }
    }
}

/// `atom` as a listing writes it, a line declared here named as from
/// `place`, the session or the library.
fn listed(atom: Atom, place: &str) -> TrexAtom {
    let source = match &atom.file {
        Some(file) => file.display().to_string(),
        None => place.to_string(),
    };
    TrexAtom {
        name: atom.name,
        form: AtomForm::of(atom.form),
        definition: atom.definition,
        source,
        description: String::new(),
    }
}

/// Every atom `decls` holds, in declaration order.
fn atoms_of(decls: &trex::Declarations, place: &str) -> PsResult<Vec<TrexAtom>> {
    Ok(decls.atoms().map_err(|e| declare_err(&e))?.into_iter().map(|a| listed(a, place)).collect())
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
    let made = TrexLibrary::of(trex::Declarations::new(), Some(location(ps)?))?.into_ps()?;
    ps.set_variable(SESSION_VARIABLE, &made)?;
    PsProxy::from_ps(&made)
}

/// PowerShell's current filesystem location, which a relative `@file` in a
/// pattern is read from, as a relative `-Path` is. It is the filesystem's
/// even where the current location is another provider's.
pub(crate) fn location(ps: &Pipeline<'_>) -> PsResult<PathBuf> {
    let context = ps.variable("ExecutionContext")?;
    let state = pwrs::object::property(&context, "SessionState")?;
    let paths = pwrs::object::property(&state, "Path")?;
    let here = pwrs::object::property(&paths, "CurrentFileSystemLocation")?;
    let provider_path = pwrs::object::property(&here, "ProviderPath")?;
    Ok(PathBuf::from(String::from_ps(&provider_path)?))
}

/// The atoms a call reads with `files` imported for that call alone: a file
/// the library or the session already imported is read in its place, as
/// Import-TrexAtom reads a file again, and neither is changed.
pub(crate) fn atoms_with_files(
    ps: &Pipeline<'_>,
    library: &Option<PsProxy<TrexLibrary>>,
    files: &[PathBuf],
) -> PsResult<trex::ShapeSet> {
    let decls = match library {
        Some(lib) => lib.with(|l| l.decls.clone())?,
        None => match session_if_any(ps)? {
            Some(held) => held.with(|l| l.decls.clone())?,
            None => trex::Declarations::new(),
        },
    };
    let mut set = decls.with_files(files).map_err(|e| declare_err(&e))?.set().clone();
    set.set_base_dir(Some(location(ps)?));
    Ok(set)
}

/// The atoms a call reads: the library it was given, or else the session's,
/// a relative `@file` in a pattern compiled against them read from
/// PowerShell's current location.
pub(crate) fn atoms_for(ps: &Pipeline<'_>, library: &Option<PsProxy<TrexLibrary>>) -> PsResult<trex::ShapeSet> {
    let mut set = match library {
        Some(lib) => lib.with(|l| l.decls.set().clone())?,
        None => match session_if_any(ps)? {
            Some(held) => held.with(|l| l.decls.set().clone())?,
            None => trex::ShapeSet::new(),
        },
    };
    set.set_base_dir(Some(location(ps)?));
    Ok(set)
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
    pub(crate) decls: trex::Declarations,
    /// The PowerShell location the library was made at, which a relative
    /// `@file` in a line its `Declare` method takes is read from; none for
    /// a library made with no PowerShell location to read.
    #[psfield(skip)]
    base: Option<PathBuf>,
}

impl TrexLibrary {
    fn of(decls: trex::Declarations, base: Option<PathBuf>) -> PsResult<Self> {
        let mut lib = TrexLibrary { names: Vec::new(), files: Vec::new(), decls, base };
        lib.refresh()?;
        Ok(lib)
    }

    fn refresh(&mut self) -> PsResult<()> {
        self.names = atoms_of(&self.decls, "library")?.into_iter().map(|a| a.name).collect();
        self.files = self.decls.files().iter().map(|f| f.display().to_string()).collect();
        Ok(())
    }
}

#[psmethods]
impl TrexLibrary {
    /// An empty library.
    pub fn new() -> PsResult<Self> {
        TrexLibrary::of(trex::Declarations::new(), None)
    }

    /// Declares one line as a pattern file writes it: `shape name =
    /// \`bytes\``, `kind name = pattern`, `let name = pattern`, or a `test`
    /// line. A relative `@file` in it is read from the location the library
    /// was made at.
    pub fn declare(&mut self, line: String) -> PsResult<()> {
        self.decls.declare(&line, self.base.clone()).map_err(|e| declare_err(&e))?;
        self.refresh()
    }
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
        let atom = AtomDecl {
            name: &self.name,
            form: form.trex(),
            definition: &definition,
            accepts: &self.accepts,
            rejects: &self.rejects,
        };
        let here = Some(location(ps)?);
        let declare = |l: &mut TrexLibrary| -> PsResult<()> {
            l.decls.add(&atom, here.clone(), self.force).map_err(|e| declare_err(&e))?;
            l.refresh()
        };
        let place = match &self.library {
            Some(lib) => {
                lib.with_mut(declare)??;
                "library"
            }
            None => {
                session(ps)?.with_mut(declare)??;
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
/// from beside each file. A directory reads as every `.trex` file under it,
/// in path order.
///
/// A file saved as UTF-8, UTF-16 or UTF-32 with a byte-order mark reads as
/// its text. Importing a file again reads its current text in place of the
/// earlier import.
///
/// # Examples
/// Import-TrexAtom ./defs.trex
/// Import-TrexAtom ./atoms
/// Get-ChildItem ./atoms -Filter *.trex | Import-TrexAtom
#[cmdlet(verb = "Import", noun = "TrexAtom", alias = "Import-TxAtom", default_parameter_set = "Path", output = ["Trex.Atom"])]
#[derive(Default)]
pub struct ImportTrexAtom {
    /// Pattern files to import, a directory as every `.trex` file under it;
    /// wildcards expand.
    #[param(mandatory, position = 0, set = "Path")]
    pub path: Vec<String>,
    /// Pattern files to import, read as written, as Get-ChildItem pipes them;
    /// a directory as every `.trex` file under it.
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
                    l.decls.include(std::slice::from_ref(&path)).map_err(|e| declare_err(&e))?;
                    l.refresh()
                })??;
                if self.pass_thru {
                    let files =
                        trex::declarations::pattern_files(std::slice::from_ref(&path)).map_err(|e| declare_err(&e))?;
                    for file in files {
                        for atom in trex::declarations::atoms_in_file(&file).map_err(|e| declare_err(&e))? {
                            ps.write(listed(atom, "library"))?;
                        }
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
                Some(lib) => lib.with(|l| atoms_of(&l.decls, "library"))??,
                None => match session_if_any(ps)? {
                    Some(held) => held.with(|l| atoms_of(&l.decls, "session"))??,
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
/// file that declared them, by a directory files were imported from under,
/// or all of them.
///
/// Every name and path given, and every one piped in, is removed as one
/// change after the last, so atoms that read one another go together in any
/// order, each with the lines declared that add to it: its `test` lines, and
/// a rule's `fix` and `meta`. An atom another one still reads stays, with an
/// error naming it.
///
/// # Examples
/// Unregister-TrexAtom ticket
/// Get-TrexAtom t* | Unregister-TrexAtom
/// Unregister-TrexAtom -Path ./defs.trex
/// Unregister-TrexAtom -Path ./atoms
/// Unregister-TrexAtom -All
#[cmdlet(verb = "Unregister", noun = "TrexAtom", alias = "Unregister-TxAtom", supports_should_process, default_parameter_set = "Name")]
#[derive(Default)]
pub struct UnregisterTrexAtom {
    /// The names to remove.
    #[param(mandatory, position = 0, set = "Name", value_from_pipeline_by_property_name)]
    pub name: Vec<String>,
    /// Imported pattern files to remove, with every atom they declared; a
    /// directory removes every file imported from under it.
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

    fn change(&self, ps: &Pipeline<'_>, f: impl FnOnce(&mut trex::Declarations) -> PsResult<()>) -> PsResult<()> {
        match self.target(ps)? {
            Some(lib) => lib.with_mut(|l| -> PsResult<()> {
                f(&mut l.decls)?;
                l.refresh()
            })?,
            None => f(&mut trex::Declarations::new()),
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
                let path = PathBuf::from(&resolved);
                let action = if path.is_dir() {
                    "Remove the atoms of every file imported from under it"
                } else {
                    "Remove the atoms this file declares"
                };
                if ps.should_process(&resolved, action)? {
                    self.files_asked.push(path);
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
            refused = d.remove(&names, &files).map_err(|e| declare_err(&e))?.iter().map(kept_err).collect();
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
    /// Pattern files to declare, a directory as every `.trex` file under it;
    /// wildcards expand.
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
            None => trex::Declarations::new(),
        };
        let mut files = Vec::new();
        for given in &self.path {
            for resolved in ps.resolve_path(given, false)? {
                files.push(PathBuf::from(resolved));
            }
        }
        decls.include(&files).map_err(|e| declare_err(&e))?;
        let here = location(ps)?;
        for line in &self.declaration {
            decls.declare(line, Some(here.clone())).map_err(|e| declare_err(&e))?;
        }
        ps.write(TrexLibrary::of(decls, Some(here))?)
    }
}
