//! Declarations a pattern reads by name, kept as the ordered list they were
//! made in: lines as a pattern file writes them, and whole pattern files.
//!
//! A [`ShapeSet`] has no way to take a declaration back, so the set is built
//! again from the list after each change, and a change the set refuses
//! leaves the list as it was. PowerShell's `Trex.Library` and Python's
//! `trex.Library` each hold one.

use std::path::{Path, PathBuf};

use crate::custom::{ShapeError, ShapeSet};

/// What a declaration declares, by the keyword opening its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `shape`: a bounded byte-pattern the lexer tries before its built-in
    /// recognizers, so where it matches it wins.
    Shape,
    /// `shape-after`: a bounded byte-pattern tried only where no built-in
    /// recognizer matched.
    ShapeAfter,
    /// `kind`: a pattern whose every match, after the lex, becomes one token
    /// of this kind.
    Kind,
    /// `let`: a pattern read in place wherever the name appears.
    Let,
    /// `rule`: a named pattern with a message and a severity, reported by a
    /// rule scan and read as a pattern elsewhere.
    Rule,
}

impl Form {
    /// The keyword a pattern file opens the line with.
    #[must_use]
    pub fn keyword(self) -> &'static str {
        match self {
            Form::Shape => "shape",
            Form::ShapeAfter => "shape-after",
            Form::Kind => "kind",
            Form::Let => "let",
            Form::Rule => "rule",
        }
    }

    /// The form a line's opening keyword names; `None` for any other word.
    #[must_use]
    pub fn of_keyword(keyword: &str) -> Option<Form> {
        match keyword {
            "shape" => Some(Form::Shape),
            "shape-after" => Some(Form::ShapeAfter),
            "kind" => Some(Form::Kind),
            "let" => Some(Form::Let),
            "rule" => Some(Form::Rule),
            _ => None,
        }
    }
}

/// One atom a declaration makes, as a listing names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atom {
    /// The name a pattern reads it by, as `\{name}`.
    pub name: String,
    /// What it declares.
    pub form: Form,
    /// The byte-pattern or pattern the declaration gives, the text after its
    /// `=`.
    pub definition: String,
    /// The pattern file that declares it; `None` for a line declared here.
    pub file: Option<PathBuf>,
}

/// One atom to declare: its name, what it declares, its definition, and the
/// texts a `test` line beside it holds.
#[derive(Clone, Copy, Debug)]
pub struct AtomDecl<'a> {
    /// The name a pattern reads it by.
    pub name: &'a str,
    /// What it declares.
    pub form: Form,
    /// The byte-pattern of a shape, or the pattern of the rest.
    pub definition: &'a str,
    /// Texts the atom must match whole, from the first token to the last.
    pub accepts: &'a [String],
    /// Texts the atom must match nowhere.
    pub rejects: &'a [String],
}

/// Why a declaration, or the set the declarations build, was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclareError {
    /// A declaration is one line, and the text `what` names holds a line
    /// break.
    LineBreak {
        /// What the text is, as `a definition`.
        what: &'static str,
    },
    /// A shape's byte-pattern is written between backticks, so it cannot
    /// hold one.
    Backtick,
    /// A line declaring `name` was made here already, and replacing it was
    /// not asked for.
    Exists {
        /// The name declared twice.
        name: String,
    },
    /// The line opens with `word`, which opens no declaration.
    NoKeyword {
        /// The line's first word.
        word: String,
    },
    /// A pattern file could not be read.
    Read {
        /// The file.
        file: PathBuf,
        /// What the read met.
        reason: String,
    },
    /// A directory named for its pattern files holds no `.trex` file.
    NoPatternFile {
        /// The directory, as it was named.
        dir: PathBuf,
    },
    /// A directory named for its pattern files could not be read.
    ReadDir {
        /// The directory.
        dir: PathBuf,
        /// What the read met.
        reason: String,
    },
    /// The set refused what the declarations build.
    Refused(ShapeError),
}

impl std::fmt::Display for DeclareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeclareError::LineBreak { what } => write!(f, "{what} is one line; this one holds a line break"),
            DeclareError::Backtick => {
                write!(f, "a shape's byte-pattern is written between backticks, so it cannot hold one")
            }
            DeclareError::Exists { name } => write!(f, "an atom named {name} is already declared here"),
            DeclareError::NoKeyword { word } => write!(
                f,
                "{word:?} opens no declaration; a line opens with let, kind, shape, shape-after, test, fields or rule, or one of a rule's fix, meta, files, unless, record, record-start or record-span"
            ),
            DeclareError::Read { file, reason } => write!(f, "{}: {reason}", file.display()),
            DeclareError::NoPatternFile { dir } => write!(f, "{}: no .trex file under it", dir.display()),
            DeclareError::ReadDir { dir, reason } => write!(f, "cannot read {}: {reason}", dir.display()),
            DeclareError::Refused(e) => write!(f, "{}", e.msg),
        }
    }
}

impl std::error::Error for DeclareError {}

/// Why a removal left a name or a file where it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    /// An imported file declares the name, and removing the file takes it
    /// out.
    InFile {
        /// The name asked for.
        name: String,
        /// The file that declares it.
        file: PathBuf,
    },
    /// Nothing declared here goes by the name.
    NoName {
        /// The name asked for.
        name: String,
    },
    /// No file imported here is the path asked for or under it.
    NoFile {
        /// The path asked for.
        file: PathBuf,
    },
}

/// Where one declaration came from.
#[derive(Clone, Debug)]
enum Source {
    /// One line made here: the name it declares, its form (`None` for a
    /// `test` line, which declares no atom), its definition, the text
    /// declared, which holds a `test` line too where the atom has
    /// expectations, and the directory a relative `@file` in it is read
    /// from, or none for the process's.
    Line { name: String, form: Option<Form>, definition: String, text: String, base: Option<PathBuf> },
    /// A pattern file, declared whole, a relative `@file` set in it read from
    /// beside it.
    File(PathBuf),
}

/// An ordered list of declarations and the set built from them.
#[derive(Clone, Debug, Default)]
pub struct Declarations {
    sources: Vec<Source>,
    set: ShapeSet,
}

impl Declarations {
    /// No declarations, and the empty set.
    #[must_use]
    pub fn new() -> Self {
        Declarations::default()
    }

    /// The set every declaration here builds.
    #[must_use]
    pub fn set(&self) -> &ShapeSet {
        &self.set
    }

    fn build(sources: &[Source]) -> Result<ShapeSet, DeclareError> {
        let mut set = ShapeSet::new();
        for source in sources {
            match source {
                Source::Line { text, base, .. } => {
                    set.set_base_dir(base.clone());
                    let declared = set.declare_text(text);
                    set.set_base_dir(None);
                    declared.map_err(DeclareError::Refused)?;
                }
                Source::File(path) => set.declare_file(path).map_err(DeclareError::Refused)?,
            }
        }
        Ok(set)
    }

    /// Replace the list with `sources` where they build, leaving it as it was
    /// where they do not.
    fn commit(&mut self, sources: Vec<Source>) -> Result<(), DeclareError> {
        let set = Self::build(&sources)?;
        self.sources = sources;
        self.set = set;
        Ok(())
    }

    /// Whether a line made here declares the atom `name`.
    #[must_use]
    pub fn declares(&self, name: &str) -> bool {
        self.sources.iter().any(|s| matches!(s, Source::Line { name: n, form: Some(_), .. } if n == name))
    }

    /// Declare one line as a pattern file writes it: `shape name =
    /// \`bytes\``, `shape-after`, `kind`, `let` or `rule`, which declare an
    /// atom, or a line that adds to one: a `test` line, a `fields` line, or a
    /// rule's `fix`, `meta`, `files`, `unless`, `record`, `record-start` or
    /// `record-span` line. A relative `@file` in it is read from `base`, or
    /// from the process's directory where that is `None`.
    ///
    /// # Errors
    ///
    /// The line holds a line break or opens with no declaration's keyword,
    /// or the set refuses it; the declarations are as they were.
    pub fn declare(&mut self, line: &str, base: Option<PathBuf>) -> Result<(), DeclareError> {
        let trimmed = line.trim();
        one_line("a declaration", trimmed)?;
        let (keyword, rest) = trimmed.split_once(char::is_whitespace).unwrap_or((trimmed, ""));
        let (name, definition) = name_and_definition(rest);
        let form = match (keyword, Form::of_keyword(keyword)) {
            (_, Some(form)) => Some(form),
            (
                "test" | "fields" | "fix" | "meta" | "files" | "unless" | "record" | "record-start" | "record-span",
                None,
            ) => None,
            (word, None) => return Err(DeclareError::NoKeyword { word: word.to_string() }),
        };
        let mut next = self.sources.clone();
        next.push(Source::Line { name, form, definition, text: trimmed.to_string(), base });
        self.commit(next)
    }

    /// Declare `atom`, its expectations written as a `test` line beside it,
    /// a relative `@file` in its definition read from `base`. A line of the
    /// same name made here is replaced where `replace` is set.
    ///
    /// # Errors
    ///
    /// A text holds a line break, a shape's byte-pattern holds a backtick, a
    /// line made here declares the name and `replace` is not set, or the set
    /// refuses the declaration; the declarations are as they were.
    pub fn add(&mut self, atom: &AtomDecl<'_>, base: Option<PathBuf>, replace: bool) -> Result<(), DeclareError> {
        one_line("a definition", atom.definition)?;
        let mut text = match atom.form {
            Form::Shape | Form::ShapeAfter => {
                if atom.definition.contains('`') {
                    return Err(DeclareError::Backtick);
                }
                format!("{} {} = `{}`", atom.form.keyword(), atom.name, atom.definition)
            }
            Form::Kind | Form::Let | Form::Rule => {
                format!("{} {} = {}", atom.form.keyword(), atom.name, atom.definition)
            }
        };
        for given in atom.accepts.iter().chain(atom.rejects) {
            one_line("an accepted or rejected text", given)?;
        }
        if !atom.accepts.is_empty() || !atom.rejects.is_empty() {
            let mut test = format!("test {}", atom.name);
            if !atom.accepts.is_empty() {
                test.push_str(" accepts");
                for a in atom.accepts {
                    test.push(' ');
                    test.push_str(&quoted(a));
                }
            }
            if !atom.rejects.is_empty() {
                test.push_str(" rejects");
                for r in atom.rejects {
                    test.push(' ');
                    test.push_str(&quoted(r));
                }
            }
            text.push('\n');
            text.push_str(&test);
        }
        if self.declares(atom.name) && !replace {
            return Err(DeclareError::Exists { name: atom.name.to_string() });
        }
        let mut next: Vec<Source> = self
            .sources
            .iter()
            .filter(|s| !matches!(s, Source::Line { name, form: Some(_), .. } if name == atom.name))
            .cloned()
            .collect();
        next.push(Source::Line {
            name: atom.name.to_string(),
            form: Some(atom.form),
            definition: atom.definition.to_string(),
            text,
            base,
        });
        self.commit(next)
    }

    /// Declare pattern files in order, a directory as every `.trex` file
    /// under it in path order, each in place of an earlier import of the
    /// same file, so importing one again reads its current text. A file
    /// named twice is read once, where it is first named; the set is built
    /// once.
    ///
    /// # Errors
    ///
    /// A directory holds no `.trex` file or cannot be read, a file cannot be
    /// read, or the set refuses what it declares; the declarations are as
    /// they were.
    pub fn include<P: AsRef<Path>>(&mut self, paths: &[P]) -> Result<(), DeclareError> {
        let mut files: Vec<PathBuf> = Vec::new();
        for file in pattern_files(paths)? {
            if !files.contains(&file) {
                files.push(file);
            }
        }
        let mut next: Vec<Source> =
            self.sources.iter().filter(|s| !matches!(s, Source::File(p) if files.contains(p))).cloned().collect();
        next.extend(files.into_iter().map(Source::File));
        self.commit(next)
    }

    /// Take out the lines declaring `names`, with the lines made here that
    /// add to them (their `test` lines, and a rule's `fix` or `meta`), and
    /// the imports of `files` as one change, a path taking out the imported
    /// file it is and every one imported from under it, so atoms that read
    /// one another go together whatever order they are named in, and give
    /// back what stayed and why: a name only an imported file declares, a
    /// name nothing here declares, a path no import is or is under.
    ///
    /// # Errors
    ///
    /// An imported file cannot be read to say whether it declares a name, or
    /// what is left does not build because an atom that stays reads one
    /// taken out; nothing is taken out.
    pub fn remove(&mut self, names: &[String], files: &[PathBuf]) -> Result<Vec<Kept>, DeclareError> {
        let mut kept = Vec::new();
        let mut removed: Vec<&String> = Vec::new();
        for name in names {
            if self.declares(name) {
                removed.push(name);
            } else {
                kept.push(self.why_kept(name)?);
            }
        }
        for file in files {
            if !self.sources.iter().any(|s| matches!(s, Source::File(p) if p.starts_with(file))) {
                kept.push(Kept::NoFile { file: file.clone() });
            }
        }
        let next: Vec<Source> = self
            .sources
            .iter()
            .filter(|s| match s {
                Source::Line { name, .. } => !removed.contains(&name),
                Source::File(p) => !files.iter().any(|f| p.starts_with(f)),
            })
            .cloned()
            .collect();
        if next.len() != self.sources.len() {
            self.commit(next)?;
        }
        Ok(kept)
    }

    /// Why `name`, which no line made here declares, stays: the imported file
    /// that declares it, or that nothing here does.
    fn why_kept(&self, name: &str) -> Result<Kept, DeclareError> {
        for source in &self.sources {
            if let Source::File(path) = source
                && atoms_in_file(path)?.iter().any(|a| a.name == name)
            {
                return Ok(Kept::InFile { name: name.to_string(), file: path.clone() });
            }
        }
        Ok(Kept::NoName { name: name.to_string() })
    }

    /// Remove every line and every imported file.
    pub fn clear(&mut self) {
        self.sources.clear();
        self.set = ShapeSet::new();
    }

    /// Every atom declared here, in declaration order, an imported file's
    /// read from the file's current text.
    ///
    /// # Errors
    ///
    /// An imported file cannot be read.
    pub fn atoms(&self) -> Result<Vec<Atom>, DeclareError> {
        let mut out = Vec::new();
        for source in &self.sources {
            match source {
                Source::Line { name, form: Some(form), definition, .. } => out.push(Atom {
                    name: name.clone(),
                    form: *form,
                    definition: definition.clone(),
                    file: None,
                }),
                Source::Line { form: None, .. } => {}
                Source::File(path) => out.extend(atoms_in_file(path)?),
            }
        }
        Ok(out)
    }

    /// The imported files, in import order.
    #[must_use]
    pub fn files(&self) -> Vec<PathBuf> {
        self.sources
            .iter()
            .filter_map(|s| match s {
                Source::File(path) => Some(path.clone()),
                Source::Line { .. } => None,
            })
            .collect()
    }

    /// These declarations with `files` imported too, for one call that reads
    /// them; `self` is unchanged.
    ///
    /// # Errors
    ///
    /// As [`Declarations::include`].
    pub fn with_files(&self, files: &[PathBuf]) -> Result<Declarations, DeclareError> {
        let mut out = self.clone();
        if !files.is_empty() {
            out.include(files)?;
        }
        Ok(out)
    }
}

/// The atoms the pattern file at `path` declares, read line by line as TREX
/// reads them, in file order.
///
/// # Errors
///
/// The file cannot be read.
pub fn atoms_in_file(path: &Path) -> Result<Vec<Atom>, DeclareError> {
    let bytes =
        std::fs::read(path).map_err(|e| DeclareError::Read { file: path.to_path_buf(), reason: e.to_string() })?;
    let text = String::from_utf8_lossy(&crate::encoding::decode(bytes)).into_owned();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let (keyword, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let Some(form) = Form::of_keyword(keyword) else {
            continue;
        };
        let (name, definition) = name_and_definition(rest);
        out.push(Atom { name, form, definition, file: Some(path.to_path_buf()) });
    }
    Ok(out)
}

/// The pattern files `paths` name, in order: a file as it is, a directory as
/// every `.trex` file under it, its subdirectories entered, in path order.
///
/// # Errors
///
/// A directory with no `.trex` file under it, or one that cannot be read.
pub fn pattern_files<P: AsRef<Path>>(paths: &[P]) -> Result<Vec<PathBuf>, DeclareError> {
    let mut out = Vec::new();
    for given in paths {
        let path = given.as_ref();
        if path.is_dir() {
            let mut found = Vec::new();
            files_under(path, &mut found)?;
            if found.is_empty() {
                return Err(DeclareError::NoPatternFile { dir: path.to_path_buf() });
            }
            found.sort();
            out.extend(found);
        } else {
            out.push(path.to_path_buf());
        }
    }
    Ok(out)
}

/// Every `.trex` file under `dir`, into `out`, subdirectories entered as met.
fn files_under(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), DeclareError> {
    let unread = |e: std::io::Error| DeclareError::ReadDir { dir: dir.to_path_buf(), reason: e.to_string() };
    for entry in std::fs::read_dir(dir).map_err(unread)? {
        let path = entry.map_err(unread)?.path();
        if path.is_dir() {
            files_under(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "trex") {
            out.push(path);
        }
    }
    Ok(())
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
fn one_line(what: &'static str, text: &str) -> Result<(), DeclareError> {
    if text.contains(['\n', '\r']) {
        return Err(DeclareError::LineBreak { what });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom<'a>(name: &'a str, form: Form, definition: &'a str) -> AtomDecl<'a> {
        AtomDecl { name, form, definition, accepts: &[], rejects: &[] }
    }

    #[test]
    fn a_declared_line_builds_the_set_and_lists_as_an_atom() {
        let mut d = Declarations::new();
        d.declare("shape ticket = `[A-Z]{2,4}-\\d{1,4}`", None).expect("a valid shape");
        assert!(d.declares("ticket"));
        let atoms = d.atoms().expect("no file to read");
        assert_eq!(atoms.len(), 1);
        assert_eq!((atoms[0].name.as_str(), atoms[0].form), ("ticket", Form::Shape));
        let p = crate::parser::parse_with_shapes(r"\{ticket}", d.set()).expect("the shape is declared");
        let spans = crate::scan_with_shapes(&p, b"see AB-12 now", d.set());
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn a_refused_change_leaves_the_declarations_as_they_were() {
        let mut d = Declarations::new();
        d.declare("let n = \\N", None).expect("a valid let");
        let refused = d.declare("let bad = \\N{>", None);
        assert!(matches!(refused, Err(DeclareError::Refused(_))), "{refused:?}");
        assert_eq!(d.atoms().expect("no file").len(), 1);
        assert!(matches!(d.declare("nonsense x = 1", None), Err(DeclareError::NoKeyword { .. })));
        assert!(matches!(d.declare("let a = \\N\nlet b = \\W", None), Err(DeclareError::LineBreak { .. })));
    }

    #[test]
    fn a_name_declared_twice_is_refused_unless_replaced() {
        let mut d = Declarations::new();
        d.add(&atom("n", Form::Let, "\\N"), None, false).expect("first declaration");
        assert_eq!(
            d.add(&atom("n", Form::Let, "\\W"), None, false),
            Err(DeclareError::Exists { name: "n".to_string() })
        );
        d.add(&atom("n", Form::Let, "\\W"), None, true).expect("replaced");
        let atoms = d.atoms().expect("no file");
        assert_eq!(atoms.len(), 1);
        assert_eq!(atoms[0].definition, "\\W");
        assert_eq!(d.add(&atom("s", Form::Shape, "a`b"), None, false), Err(DeclareError::Backtick));
    }

    #[test]
    fn expectations_become_a_test_line_the_set_runs() {
        let mut d = Declarations::new();
        let accepts = ["AB-12".to_string()];
        let rejects = ["A-1".to_string()];
        let ticket = AtomDecl {
            name: "ticket",
            form: Form::Shape,
            definition: "[A-Z]{2,4}-\\d{1,4}",
            accepts: &accepts,
            rejects: &rejects,
        };
        d.add(&ticket, None, false).expect("a valid shape and test");
        assert_eq!(d.set().tests().len(), 1);
        assert!(d.set().run_tests().is_empty());
        // A test line declared on its own is run, and is no atom.
        d.declare("test ticket accepts \"XYZ-9\"", None).expect("a valid test line");
        assert_eq!(d.set().tests().len(), 2);
        assert_eq!(d.atoms().expect("no file").len(), 1);
    }

    #[test]
    fn removal_takes_names_and_files_out_together_and_says_what_stayed() {
        let dir = std::env::temp_dir().join(format!("trex-declarations-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("make a folder");
        let file = dir.join("defs.trex");
        std::fs::write(&file, "let tag = \\W\nlet count = \\N\n").expect("write the file");
        let mut d = Declarations::new();
        d.include(std::slice::from_ref(&file)).expect("the file declares");
        d.add(&atom("pair", Form::Let, "\\{tag} \\{count}"), None, false).expect("reads the file's atoms");
        assert_eq!(d.files(), std::slice::from_ref(&file));
        let names: Vec<String> = d.atoms().expect("the file reads").into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["tag", "count", "pair"]);

        // Taking the file out alone would leave `pair` reading what it
        // declared, so nothing is taken out.
        assert!(matches!(d.remove(&[], std::slice::from_ref(&file)), Err(DeclareError::Refused(_))));
        assert_eq!(d.files(), std::slice::from_ref(&file));

        // Together they go; a name only the file declared, one nothing
        // declares and a file never imported each stay with a reason.
        let kept = d
            .remove(
                &["pair".to_string(), "tag".to_string(), "nope".to_string()],
                &[file.clone(), dir.join("other.trex")],
            )
            .expect("pair and the file go together");
        assert_eq!(
            kept,
            [
                Kept::InFile { name: "tag".to_string(), file: file.clone() },
                Kept::NoName { name: "nope".to_string() },
                Kept::NoFile { file: dir.join("other.trex") },
            ]
        );
        assert!(d.atoms().expect("nothing to read").is_empty());
        std::fs::remove_dir_all(&dir).expect("remove the folder");
    }

    #[test]
    fn a_rule_declared_a_line_at_a_time_goes_with_the_lines_that_add_to_it() {
        let mut d = Declarations::new();
        d.declare(r#"rule num note "a number" = \N:n"#, None).expect("a rule line");
        d.declare("fix num = <${n}>", None).expect("a fix line");
        d.declare("meta num team = core", None).expect("a meta line");
        d.declare("test num accepts \"7\"", None).expect("a test line");
        let rule = &d.set().rules()[0];
        assert_eq!(rule.fix.as_deref(), Some("<${n}>"));
        assert_eq!(rule.meta, [("team".to_string(), "core".to_string())]);
        // The rule is the one atom; the lines adding to it are not listed.
        assert_eq!(d.atoms().expect("no file").len(), 1);
        assert!(matches!(d.declare("bogus num = 1", None), Err(DeclareError::NoKeyword { .. })));
        // Taking the rule out takes the lines that add to it, so the set the
        // rest builds holds no fix or test of a rule that is gone.
        assert!(d.remove(&["num".to_string()], &[]).expect("the rule goes with its lines").is_empty());
        assert!(d.set().rules().is_empty());
        assert!(d.set().tests().is_empty());
    }

    #[test]
    fn a_directory_includes_and_removes_as_its_pattern_files() {
        let dir = std::env::temp_dir().join(format!("trex-declarations-dir-{}", std::process::id()));
        let nested = dir.join("more");
        std::fs::create_dir_all(&nested).expect("make the folders");
        std::fs::write(dir.join("b.trex"), "let second = \\W\n").expect("write b.trex");
        std::fs::write(dir.join("a.trex"), "let first = \\N\n").expect("write a.trex");
        std::fs::write(nested.join("a.trex"), "let third = \\N\n").expect("write more/a.trex");
        std::fs::write(dir.join("notes.txt"), "let ignored = \\N\n").expect("write notes.txt");
        let mut d = Declarations::new();
        // A file named beside its directory is read once, where it is first
        // named; the rest follow in path order.
        d.include(&[dir.clone(), dir.join("a.trex")]).expect("the directory declares");
        assert_eq!(d.files(), [dir.join("a.trex"), dir.join("b.trex"), nested.join("a.trex")]);
        let names: Vec<String> = d.atoms().expect("the files read").into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["first", "second", "third"]);

        assert!(d.remove(&[], std::slice::from_ref(&nested)).expect("nothing reads third").is_empty());
        assert_eq!(d.files(), [dir.join("a.trex"), dir.join("b.trex")]);
        assert!(d.remove(&[], std::slice::from_ref(&dir)).expect("nothing reads them").is_empty());
        assert!(d.files().is_empty());
        assert_eq!(
            d.remove(&[], std::slice::from_ref(&dir)).expect("nothing to build"),
            [Kept::NoFile { file: dir.clone() }]
        );

        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).expect("make the empty folder");
        assert_eq!(d.include(std::slice::from_ref(&empty)), Err(DeclareError::NoPatternFile { dir: empty.clone() }));
        std::fs::remove_dir_all(&dir).expect("remove the folders");
    }
}
