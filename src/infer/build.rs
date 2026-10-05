//! The pattern builder: named captures over every shape a set of lines
//! takes. Fields are pointed at by marked examples, lines with
//! `{name:text}` around each value ([`super::marks`]), and by value hints, a
//! field's name and values it takes. Lines group by the kinds of their
//! tokens; a group holding a field is a shape, a group holding none joins the
//! shape its tokens align with, and a group aligning with none is a shape of
//! its own whose fields are found between the literals that surround them in
//! a marked shape. Each shape is one branch of the pattern, verified against
//! every line and every counter-example before it is returned.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::ops::Range;

use super::marks::{Hint, Marked};
use super::{atom_of, narrowing_of, range_of, Column, Narrowing, Slot};
use crate::custom::ShapeSet;
use crate::token::TokenKind;

/// What a pattern is built from.
#[derive(Clone, Copy, Debug)]
pub struct Spec<'a> {
    /// The lines the pattern is for, as they read.
    pub lines: &'a [String],
    /// Lines with the fields to extract marked.
    pub marked: &'a [Marked],
    /// Fields named by value: a field's name and values it takes in the
    /// lines.
    pub hints: &'a [(String, Vec<String>)],
    /// Lines the pattern must not match.
    pub counters: &'a [String],
    /// The declared shapes the lines are lexed under.
    pub shapes: &'a ShapeSet,
    /// Whether a match may start and end inside a longer line.
    pub unanchored: bool,
    /// How a field's token whose values share a constant part is spelled.
    pub mint: Mint,
}

/// How the builder spells a word token of a field whose values share a
/// constant part: `KB5031354` and `KB4534310` share `KB` then seven
/// digits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mint {
    /// An inline byte atom, `` `KB[0-9]{7}` ``, matched whole against the
    /// token, so nothing else in the line lexes differently.
    #[default]
    Inline,
    /// A shape declared for a field of one word token, `shape kb =
    /// `KB[0-9]{7}``, read as `\{kb}`; [`Built::declarations`] holds each,
    /// and the pattern reads only under them. A declared shape is tried at
    /// every token's start, so it lexes the whole line it is scanned over.
    /// A word inside a longer field stays an inline byte atom.
    Shapes,
    /// The token's kind, `\W`.
    Off,
}

/// One field of a built pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The capture's name.
    pub name: String,
    /// The accessor a template writes after the capture where the field is
    /// part of one token, `host` of a URL.
    pub accessor: Option<String>,
    /// The type a mark gave the field, as `{[int]age:6}` does.
    pub hint: Option<Hint>,
    /// The type's name as the mark writes it, `int` or `System.Int32`.
    pub type_name: Option<String>,
    /// Whether a mark writes the field `{name*:text}`, so a line holding it
    /// begins a record.
    pub starts_record: bool,
    /// Whether a line marks the field several times, so its value is every
    /// one of them in order.
    pub list: bool,
    /// Whether the field is inside records a line repeats, so a line holds
    /// one value of it per record: all of them in the line's row, one in
    /// each record.
    pub repeats: bool,
    /// The field whose mark encloses this one's, by its index in
    /// [`Built::fields`]; the name is then that field's, a dot, and the
    /// mark's own, `Line.n`, as TREX names a capture inside a capture.
    pub parent: Option<usize>,
    /// How a `--format` template writes the field.
    pub template: String,
}

/// A field's value in one line or record: one text, or for a list field
/// every value in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    One(String),
    Many(Vec<String>),
}

impl Value {
    /// The value as the report's table writes it: its text, or its texts
    /// joined with a comma.
    #[must_use]
    pub fn joined(&self) -> String {
        match self {
            Value::One(v) => v.clone(),
            Value::Many(vs) => vs.join(","),
        }
    }
}

impl Field {
    /// The name the field has inside its parent: the last part of its
    /// name, `n` for `Line.n`, and the whole name of a field at the top.
    #[must_use]
    pub fn key(&self) -> &str {
        self.name.rsplit('.').next().expect("a split yields a part")
    }

    /// The field's value in the text its capture bound: that text, or the
    /// part of it the field's accessor reads; `None` where it is empty.
    ///
    /// # Errors
    ///
    /// The accessor is not one a template knows, which a field the builder
    /// made never names.
    pub fn read(&self, capture: &str) -> Result<Option<String>, String> {
        let value = match &self.accessor {
            Some(accessor) => crate::rewrite::apply_named(accessor, capture)?,
            None => capture.to_string(),
        };
        Ok((!value.is_empty()).then_some(value))
    }

    /// The field's value in `m`, a match over `input` resolved with its
    /// lists: for a list field or one that repeats with its records every
    /// binding its register made, in order, and otherwise its one binding,
    /// each read as [`Field::read`] reads it; `None` where no binding holds
    /// text.
    ///
    /// # Errors
    ///
    /// The accessor is not one a template knows, as [`Field::read`] says.
    pub fn value_in(&self, m: &crate::Match, input: &str) -> Result<Option<Value>, String> {
        let read = |s: &crate::Span| -> Result<Option<String>, String> {
            if s.range().is_empty() {
                return Ok(None);
            }
            self.read(&input[s.range()])
        };
        if self.list || self.repeats {
            let once = m.group_span(&self.name);
            let spans: &[crate::Span] = match m.list(&self.name) {
                Some(all) => all,
                None => once.as_slice(),
            };
            let mut values = Vec::with_capacity(spans.len());
            for s in spans {
                if let Some(v) = read(s)? {
                    values.push(v);
                }
            }
            return Ok((!values.is_empty()).then_some(Value::Many(values)));
        }
        match m.group_span(&self.name) {
            Some(span) => Ok(read(&span)?.map(Value::One)),
            None => Ok(None),
        }
    }
}

/// The values of `fields` in `m`, a match over `input` resolved with its
/// lists, cut into the records the match holds. Where a field beginning a
/// record repeats with its records, a record begins at each of its
/// bindings; each binding of a field that repeats with the records goes to
/// the record it is in, and a field outside them describes the line, so every record of the match carries it. A record holding a
/// field several times keeps the first, and every one for a list field.
/// Otherwise the match's values whole are its one part.
///
/// # Errors
///
/// A field's accessor is not one a template knows, as [`Field::read`] says,
/// the message naming the field.
pub fn record_parts(fields: &[Field], m: &crate::Match, input: &str) -> Result<Vec<Vec<Option<Value>>>, String> {
    fn named(f: &Field) -> impl Fn(String) -> String + '_ {
        move |e| format!("field {}: {e}", f.name)
    }
    let Some(cut) = fields.iter().position(|f| f.starts_record && f.repeats) else {
        let whole = fields.iter().map(|f| f.value_in(m, input).map_err(named(f))).collect::<Result<Vec<_>, _>>()?;
        return Ok(vec![whole]);
    };
    let bindings = |f: &Field| -> Vec<crate::Span> {
        let spans: Vec<crate::Span> = match m.list(&f.name) {
            Some(all) => all.to_vec(),
            None => m.group_span(&f.name).into_iter().collect(),
        };
        spans.into_iter().filter(|s| !s.range().is_empty()).collect()
    };
    let starts: Vec<usize> = bindings(&fields[cut]).iter().map(crate::Span::start).collect();
    let mut texts: Vec<Vec<Vec<String>>> = vec![vec![Vec::new(); fields.len()]; starts.len() + 1];
    for (f, field) in fields.iter().enumerate() {
        for s in bindings(field) {
            let Some(v) = field.read(&input[s.range()]).map_err(named(field))? else { continue };
            if field.repeats {
                texts[starts.iter().filter(|&&b| b <= s.start()).count()][f].push(v);
            } else {
                for part in texts.iter_mut().skip(1) {
                    part[f].push(v.clone());
                }
            }
        }
    }
    let parts = texts
        .into_iter()
        .enumerate()
        .filter(|(p, values)| *p > 0 || values.iter().any(|v| !v.is_empty()))
        .map(|(_, values)| {
            values
                .into_iter()
                .zip(fields)
                .map(|(mut vs, field)| match vs.len() {
                    0 => None,
                    _ if field.list => Some(Value::Many(vs)),
                    _ => Some(Value::One(vs.swap_remove(0))),
                })
                .collect()
        })
        .collect();
    Ok(parts)
}

/// How a shape reaches a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// A line of the shape marks it or holds a hinted value of it.
    Marked,
    /// The literals around it in the shape at this index surround it here
    /// too.
    Anchored(usize),
    /// The shape does not hold it.
    Missing,
}

/// One shape of the lines: a branch of the pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    /// The branch.
    pub pattern: String,
    /// The lines of the shape, as indexes into [`Built::rows`].
    pub lines: Vec<usize>,
    /// How the shape reaches each field, in the order of [`Built::fields`].
    pub reach: Vec<Reach>,
}

/// One line and what the pattern reads from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The line as it reads.
    pub text: String,
    /// The shape the line is of, `None` for a line with no token.
    pub shape: Option<usize>,
    /// Each field's value, `None` where the line does not hold it.
    pub values: Vec<Option<Value>>,
}

/// One record: the lines from one holding a field that begins a record to
/// the next, or one line where no field begins one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// The record's lines, as indexes into [`Built::rows`].
    pub lines: Vec<usize>,
    /// Each field's value, as [`join_record`] joins its lines: the first of
    /// them that holds one, `None` where none does.
    pub values: Vec<Option<Value>>,
}

/// A built pattern and the report on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The pattern, one branch per shape.
    pub pattern: String,
    /// The fields, in the order first marked or hinted.
    pub fields: Vec<Field>,
    /// The shapes, in the order of their branches in the pattern.
    pub shapes: Vec<Shape>,
    /// The marked examples, then every line that is not one of them.
    pub rows: Vec<Row>,
    /// The records of the marked examples, then those of the lines, each
    /// read in order as `ConvertFrom-String` reads them: a line holding a
    /// field that begins a record begins one, the lines after it join it,
    /// and a line before the first begins none. Where no field begins a
    /// record, each line with a shape is one.
    pub records: Vec<Record>,
    /// The name a pattern file gives the pattern.
    pub name: String,
    /// The counter-examples the pattern rejects.
    pub counters: Vec<String>,
    /// Whether a match may start and end inside a longer line.
    pub unanchored: bool,
    /// The shapes minted under [`Mint::Shapes`], each as a pattern file
    /// declares it, `shape kb = `KB[0-9]{7}``; the pattern reads only
    /// under them.
    pub declarations: Vec<String>,
    /// Each field every value of which a value class of the library holds,
    /// with those classes, `status` with `http_2xx` and `http_status`,
    /// where no counter-example called for one; a `--not` line one of them
    /// refuses prints it in the pattern instead.
    pub suggestions: Vec<(String, Vec<String>)>,
}

impl Built {
    /// The indexes of the fields whose marks are inside field `parent`'s,
    /// or of the fields at the top where it is `None`, in order.
    #[must_use]
    pub fn inside(&self, parent: Option<usize>) -> Vec<usize> {
        (0..self.fields.len()).filter(|&f| self.fields[f].parent == parent).collect()
    }

    /// A `--format` template writing every field, tab-separated, as the
    /// option takes it: the tab written `\t`.
    #[must_use]
    pub fn format(&self) -> String {
        self.fields.iter().map(|f| f.template.as_str()).collect::<Vec<_>>().join("\\t")
    }

    /// Each field as a `fields` line writes it: its mark with the example
    /// text left out, `{[int]os}`, `{Name*}`, `{host:host}`.
    #[must_use]
    pub fn field_marks(&self) -> Vec<crate::infer::marks::FieldMark> {
        self.fields
            .iter()
            .map(|f| crate::infer::marks::FieldMark {
                name: f.name.clone(),
                hint: f.hint,
                type_name: f.type_name.clone(),
                starts_record: f.starts_record,
                accessor: f.accessor.clone(),
            })
            .collect()
    }

    /// The pattern as a pattern file: the shapes it declares, a `let` per
    /// shape of the lines, one naming them in order, the `fields` line
    /// giving that one its fields as the marks said them, and a test
    /// accepting the first line of each shape and rejecting each
    /// counter-example.
    #[must_use]
    pub fn file(&self) -> String {
        let counted = |n: usize, one: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") };
        let lines = counted(self.rows.iter().filter(|r| r.shape.is_some()).count(), "line");
        let mut out = format!(
            "# Built by trex infer from {lines} in {}; `{}` names every shape in order.\n",
            counted(self.shapes.len(), "shape"),
            self.name
        );
        for decl in &self.declarations {
            out.push_str(decl);
            out.push('\n');
        }
        for (i, shape) in self.shapes.iter().enumerate() {
            out.push_str(&format!("let {}_{} = {}\n", self.name, i + 1, shape.pattern));
        }
        let parts: Vec<String> = (1..=self.shapes.len()).map(|i| format!("\\{{{}_{i}}}", self.name)).collect();
        out.push_str(&format!("let {} = {}\n", self.name, parts.join(joiner(self.unanchored))));
        let marks: Vec<String> = self.field_marks().iter().map(crate::infer::marks::FieldMark::written).collect();
        out.push_str(&format!("fields {} {}\n", self.name, marks.join(" ")));
        let accepts: Vec<String> =
            self.shapes.iter().filter_map(|s| s.lines.first()).map(|&r| quoted(&self.rows[r].text)).collect();
        out.push_str(&format!("test {} accepts {}", self.name, accepts.join(" ")));
        if !self.counters.is_empty() {
            let rejects: Vec<String> = self.counters.iter().map(|c| quoted(c)).collect();
            out.push_str(&format!(" rejects {}", rejects.join(" ")));
        }
        out.push('\n');
        out
    }
}

/// A text as a `test` line quotes it.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
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

/// What goes between two branches: ordered choice for whole lines, the
/// longest match where a match may start inside a line.
fn joiner(unanchored: bool) -> &'static str {
    if unanchored { " || " } else { " | " }
}

/// Why no pattern was built. Lines are numbered from zero in the order
/// [`Built::rows`] holds them: the marked examples, then the lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// No line holds a token.
    NoLines,
    /// A field's name is not one a capture can take.
    Name(String),
    /// No line holds a hinted value as whole tokens or as a part an
    /// accessor reads.
    Absent { field: String, value: String },
    /// A hinted value appears twice in a line and no line of its shape says
    /// which is the field.
    Twice { field: String, value: String, line: usize },
    /// A marked example marks one field several times with different words
    /// between its values, or marks part of a token more than once.
    Repeated { field: String, line: usize },
    /// A mark holding marks takes part of a token, or several values.
    Nested { field: String, line: usize },
    /// A line marks a field that begins a record more than once, and the
    /// records it cuts the line into differ.
    Records { field: String, line: usize },
    /// A mark cuts a token where no accessor reads the part it holds.
    Split { field: String, line: usize },
    /// Two fields claim the same text in a line.
    Overlap { first: String, second: String, line: usize },
    /// Lines of one shape place a field at different tokens, or the field
    /// is read as tokens in one shape and as part of one in another.
    Disagree { field: String, line: usize },
    /// A typed mark holds a value its type does not read as one token of.
    Mistyped { field: String, hint: Hint, value: String, line: usize },
    /// The pattern built does not parse, which is a defect in its spelling.
    Unparsed { pattern: String, msg: String },
    /// The pattern built does not match this line whole.
    Uncovered { line: usize, pattern: String },
    /// The pattern built reads a field of this line other than as it was
    /// placed.
    Wrong { line: usize, field: String, want: Option<Value>, got: Option<Value>, pattern: String },
    /// The pattern built still matches this counter-example, and no range
    /// the lines support excludes it.
    Matched { counter: usize, pattern: String },
    /// A shape minted for a field could not be declared beside the ones the
    /// lines are lexed under.
    Undeclared { decl: String, msg: String },
}

impl BuildError {
    /// The line the refusal names, numbered as [`Built::rows`] numbers them.
    #[must_use]
    pub fn line(&self) -> Option<usize> {
        match self {
            BuildError::Twice { line, .. }
            | BuildError::Repeated { line, .. }
            | BuildError::Nested { line, .. }
            | BuildError::Records { line, .. }
            | BuildError::Split { line, .. }
            | BuildError::Overlap { line, .. }
            | BuildError::Disagree { line, .. }
            | BuildError::Mistyped { line, .. }
            | BuildError::Uncovered { line, .. }
            | BuildError::Wrong { line, .. } => Some(*line),
            _ => None,
        }
    }
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shown = |v: &Option<Value>| match v {
            Some(v) => format!("`{}`", v.joined()),
            None => "nothing".to_string(),
        };
        match self {
            BuildError::NoLines => write!(f, "no line holds a token to build a pattern from"),
            BuildError::Name(name) => write!(
                f,
                "`{name}` is not a field name: a name is a letter or `_`, then letters, digits and `_`"
            ),
            BuildError::Absent { field, value } => write!(
                f,
                "no line holds `{value}`, a value of {field}, as whole tokens or as a part an accessor reads"
            ),
            BuildError::Twice { field, value, line } => write!(
                f,
                "line {} holds `{value}`, a value of {field}, more than once and no line of its shape \
                 says which is the field; mark it in one line",
                line + 1
            ),
            BuildError::Repeated { field, line } => write!(
                f,
                "line {} marks {field} several times; a list needs whole tokens for each value and the same \
                 words between each two",
                line + 1
            ),
            BuildError::Nested { field, line } => write!(
                f,
                "line {} marks inside {field}, which takes part of a token or several values; a mark holding \
                 marks takes whole tokens, once in a line",
                line + 1
            ),
            BuildError::Records { field, line } => write!(
                f,
                "line {} repeats the record {field} begins, and its records differ: each marks the same fields in \
                 the same order, as whole tokens, with the same words between each record and the next",
                line + 1
            ),
            BuildError::Split { field, line } => write!(
                f,
                "line {} marks {field} across part of a token no accessor reads; mark whole tokens, \
                 or a part an accessor names",
                line + 1
            ),
            BuildError::Overlap { first, second, line } => {
                write!(f, "{first} and {second} claim the same text in line {}", line + 1)
            }
            BuildError::Disagree { field, line } => {
                write!(f, "line {} places {field} other than a line of its shape does", line + 1)
            }
            BuildError::Mistyped { field, hint, value, line } => write!(
                f,
                "line {} types {field} as [{}], and `{value}` does not read as one",
                line + 1,
                hint.name()
            ),
            BuildError::Unparsed { pattern, msg } => {
                write!(f, "the built pattern `{pattern}` does not parse: {msg}")
            }
            BuildError::Uncovered { line, pattern } => {
                write!(f, "the built pattern `{pattern}` does not match line {} whole", line + 1)
            }
            BuildError::Wrong { line, field, want, got, pattern } => write!(
                f,
                "the built pattern `{pattern}` reads {field} from line {} as {}, where the line holds {}",
                line + 1,
                shown(got),
                shown(want)
            ),
            BuildError::Matched { counter, pattern } => write!(
                f,
                "the built pattern `{pattern}` still matches counter-example {}; nothing the lines \
                 have in common tells them apart",
                counter + 1
            ),
            BuildError::Undeclared { decl, msg } => write!(f, "the minted `{decl}` cannot be declared: {msg}"),
        }
    }
}

/// A token as the builder reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tok {
    kind: TokenKind,
    start: usize,
    end: usize,
}

/// A field's position in a line.
#[derive(Clone, Debug, PartialEq, Eq)]
enum At {
    /// These tokens, whole.
    Run(Range<usize>),
    /// Part of this token, read by any of these accessors.
    Part(usize, Vec<String>),
    /// These tokens, holding several values of one field with the same
    /// words, these texts, between each two.
    List(Range<usize>, Vec<String>),
}

impl At {
    /// The tokens it covers.
    fn tokens(&self) -> Range<usize> {
        match self {
            At::Run(run) | At::List(run, _) => run.clone(),
            At::Part(t, _) => *t..*t + 1,
        }
    }

    /// Whether `other` is at the same position.
    fn same(&self, other: &At) -> bool {
        match (self, other) {
            (At::Run(a), At::Run(b)) => a == b,
            (At::Part(a, _), At::Part(b, _)) => a == b,
            (At::List(a, x), At::List(b, y)) => a == b && x == y,
            _ => false,
        }
    }

    /// The position both share, read by the accessors both read it with;
    /// `None` where their positions differ or they share no accessor.
    fn meet(&self, other: &At) -> Option<At> {
        match (self, other) {
            (At::Run(a), At::Run(b)) if a == b => Some(At::Run(a.clone())),
            (At::List(a, x), At::List(b, y)) if a == b && x == y => Some(At::List(a.clone(), x.clone())),
            (At::Part(a, x), At::Part(b, y)) if a == b => {
                let both: Vec<String> = x.iter().filter(|n| y.contains(n)).cloned().collect();
                (!both.is_empty()).then_some(At::Part(*a, both))
            }
            _ => None,
        }
    }
}

/// What a line says of one field before shapes form.
#[derive(Clone, Debug)]
enum Want {
    /// A mark places it here.
    Marked(At),
    /// A hinted value is at each of these; with more than one, the
    /// line's shape decides. The value is the first found.
    Found(Vec<At>, String),
}

/// A line the pattern is built for.
struct Line {
    text: String,
    toks: Vec<Tok>,
    /// Per field, what the marks and the hints say of it here.
    want: Vec<Option<Want>>,
    /// The records a template marks as repeating in this line: the tokens
    /// they take and the record they repeat.
    records: Option<(Range<usize>, Body)>,
}

impl Line {
    fn tok_text(&self, i: usize) -> &str {
        &self.text[self.toks[i].start..self.toks[i].end]
    }

    /// Where the bytes `span` are among the line's tokens, the space
    /// around them trimmed: the tokens they are, or the part of one token an
    /// accessor reads. `None` where they cut a token otherwise or hold
    /// nothing.
    fn at_of(&self, span: Range<usize>) -> Option<At> {
        let bytes = self.text.as_bytes();
        let (mut start, mut end) = (span.start, span.end);
        while start < end && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        while end > start && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        if start == end {
            return None;
        }
        let first = self.toks.iter().position(|t| t.start == start);
        let last = self.toks.iter().position(|t| t.end == end);
        if let (Some(i), Some(j)) = (first, last)
            && i <= j
        {
            return Some(At::Run(i..j + 1));
        }
        let t = self.toks.iter().position(|t| t.start <= start && end <= t.end)?;
        let tok = self.toks[t];
        let accessors =
            crate::rewrite::accessors_locating(tok.kind, self.tok_text(t), start - tok.start..end - tok.start);
        (!accessors.is_empty()).then_some(At::Part(t, accessors))
    }

    /// The text of tokens `run`, from the first's start to the last's end;
    /// `None` for no tokens.
    fn run_text(&self, run: &Range<usize>) -> Option<&str> {
        (!run.is_empty()).then(|| &self.text[self.toks[run.start].start..self.toks[run.end - 1].end])
    }

    /// The tokens of each value in the list `run` holds: the runs between
    /// occurrences of the separator words `sep`, empty runs left out.
    fn list_runs(&self, run: &Range<usize>, sep: &[String]) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let (mut from, mut t) = (run.start, run.start);
        while t < run.end {
            let at_sep = t + sep.len() <= run.end && sep.iter().enumerate().all(|(k, s)| self.tok_text(t + k) == s);
            if at_sep && t > from {
                out.push(from..t);
                t += sep.len();
                from = t;
            } else {
                t += 1;
            }
        }
        if from < run.end {
            out.push(from..run.end);
        }
        out
    }

    /// The records `run` holds of `body`: the runs between occurrences of
    /// its separator words, each aligned with the record's elements, as the
    /// tokens each element takes there; `None` where one does not align or
    /// there is none.
    fn pieces(&self, run: &Range<usize>, body: &Body) -> Option<Vec<Vec<Range<usize>>>> {
        let mut out = Vec::new();
        for piece in self.list_runs(run, &body.sep) {
            let kinds: Vec<TokenKind> = self.toks[piece.clone()].iter().map(|t| t.kind).collect();
            let lits: Vec<Option<String>> = piece.clone().map(|t| Some(self.tok_text(t).to_string())).collect();
            let (layout, _) = place(&body.skeleton, &body.lits, &kinds, &lits)?;
            out.push(layout.into_iter().map(|r| r.start + piece.start..r.end + piece.start).collect());
        }
        (!out.is_empty()).then_some(out)
    }

    /// A field marked at `before` and marked again at `next`, later in the
    /// line, as one list: the tokens from its first value to `next`, with
    /// the words between the first two values as the separator every later
    /// pair must share. `None` where a mark is part of a token, or the words
    /// between two values differ from the separator.
    fn listed(&self, before: At, next: At) -> Option<At> {
        let At::Run(next) = next else { return None };
        match before {
            At::Run(first) if first.end <= next.start => {
                let sep = (first.end..next.start).map(|t| self.tok_text(t).to_string()).collect();
                Some(At::List(first.start..next.end, sep))
            }
            At::List(run, sep) if run.end <= next.start => {
                let between: Vec<&str> = (run.end..next.start).map(|t| self.tok_text(t)).collect();
                (between == sep).then_some(At::List(run.start..next.end, sep))
            }
            _ => None,
        }
    }

    /// The value a field placed at `at` holds here, `None` where it holds
    /// no text.
    fn value(&self, at: &At) -> Option<Value> {
        match at {
            At::Run(run) => self.run_text(run).map(|v| Value::One(v.to_string())),
            At::Part(t, accessors) => {
                let v = crate::rewrite::apply_named(&accessors[0], self.tok_text(*t))
                    .expect("an accessor the rewrite module named reads its own part");
                (!v.is_empty()).then_some(Value::One(v))
            }
            At::List(run, sep) => {
                let values: Vec<String> = self
                    .list_runs(run, sep)
                    .iter()
                    .filter_map(|r| self.run_text(r).map(str::to_string))
                    .collect();
                (!values.is_empty()).then_some(Value::Many(values))
            }
        }
    }
}

/// Lines whose tokens are of the same kinds in the same order.
struct Group {
    rows: Vec<usize>,
    kinds: Vec<TokenKind>,
}

/// One element of a shape.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Elem {
    /// One token of this kind, beside the fields.
    Col(TokenKind),
    /// The field at this index, as the tokens it takes.
    Field(usize),
    /// The field at this index, as part of one token of this kind.
    Part(usize, TokenKind),
    /// The list field at this index, as the tokens its values and the
    /// separator words between them, these texts, take.
    List(usize, Vec<String>),
    /// Records repeating in the line, as the tokens they and the separator
    /// words between them take.
    Records(Box<Body>),
}

/// The record a line repeats: its elements once, and the words between
/// each record and the next.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Body {
    skeleton: Vec<Elem>,
    /// The text each column element has in the record a template marks.
    lits: Vec<Option<String>>,
    /// The separator words, these texts.
    sep: Vec<String>,
}

impl Body {
    /// The fields the record holds, in order.
    fn fields(&self) -> Vec<usize> {
        self.skeleton.iter().filter_map(Elem::field).collect()
    }
}

impl Elem {
    fn field(&self) -> Option<usize> {
        match self {
            Elem::Field(f) | Elem::Part(f, _) | Elem::List(f, _) => Some(*f),
            Elem::Col(_) | Elem::Records(_) => None,
        }
    }

    /// The position among `layout`'s tokens of the field this element is.
    fn at(&self, run: &Range<usize>, parts: &HashMap<usize, Vec<String>>) -> Option<At> {
        match self {
            Elem::Col(_) | Elem::Records(_) => None,
            Elem::Field(_) => Some(At::Run(run.clone())),
            Elem::Part(f, _) => Some(At::Part(run.start, parts[f].clone())),
            Elem::List(_, sep) => Some(At::List(run.clone(), sep.clone())),
        }
    }
}

/// A shape while lines join it.
#[derive(Clone)]
struct Forming {
    skeleton: Vec<Elem>,
    /// Each field whose mark holds marks, with the elements of the skeleton
    /// it spans, outer before inner where two start together.
    spans: Vec<(usize, Range<usize>)>,
    /// The accessors each part field is read with, which every member's
    /// token takes.
    parts: HashMap<usize, Vec<String>>,
    /// Each member line and the tokens each element takes in it.
    members: Vec<(usize, Vec<Range<usize>>)>,
    reach: Vec<Reach>,
}

impl Forming {
    /// Where field `f` is among a member's tokens `layout`: its
    /// element's place, or for a field spanning elements the tokens from
    /// the first they take to the last; `None` where it takes none.
    fn at_of(&self, layout: &[Range<usize>], f: usize) -> Option<At> {
        if let Some(e) = self.skeleton.iter().position(|el| el.field() == Some(f)) {
            return self.skeleton[e].at(&layout[e], &self.parts);
        }
        let (_, span) = self.spans.iter().find(|(g, _)| *g == f)?;
        let taken: Vec<&Range<usize>> = layout[span.clone()].iter().filter(|r| !r.is_empty()).collect();
        let (first, last) = (taken.first()?, taken.last()?);
        Some(At::Run(first.start..last.end))
    }

    /// The value field `f` holds in `line`, whose tokens the shape's
    /// elements take as `layout` says: for a field of repeating records,
    /// its value in each record, in order.
    fn value_of(&self, line: &Line, layout: &[Range<usize>], f: usize) -> Option<Value> {
        for (e, elem) in self.skeleton.iter().enumerate() {
            let Elem::Records(body) = elem else { continue };
            let Some(i) = body.skeleton.iter().position(|el| el.field() == Some(f)) else { continue };
            let pieces = line.pieces(&layout[e], body)?;
            let values: Vec<String> =
                pieces.iter().filter_map(|piece| line.run_text(&piece[i]).map(str::to_string)).collect();
            return (!values.is_empty()).then_some(Value::Many(values));
        }
        self.at_of(layout, f).and_then(|at| line.value(&at))
    }
}

/// An alignment's score: the literals it agrees on, then the fewer
/// fields it leaves empty.
type Score = (u32, Reverse<u32>);

/// Build the pattern `spec` asks for.
///
/// # Errors
///
/// A field name a capture cannot take, a mark or hint that places no field
/// or places one ambiguously, lines of one shape that disagree, and a
/// pattern failing its own verification; see [`BuildError`].
pub fn build(spec: &Spec<'_>) -> Result<Built, BuildError> {
    let (names, given) = fields_of(spec)?;
    let (mut lines, data_rows) = lines_of(spec, names.len());
    mark(spec, &names, &mut lines)?;
    hint(spec, &names, &mut lines)?;
    if lines.iter().all(|l| l.toks.is_empty()) {
        return Err(BuildError::NoLines);
    }
    let groups = groups_of(&lines);
    let mut places: Vec<Placed> = Vec::with_capacity(groups.len());
    for group in &groups {
        places.push(places_of(&lines, group, &names, &given)?);
    }
    let mut shapes: Vec<Forming> = Vec::new();
    for (group, (at, records)) in groups.iter().zip(&places) {
        if at.iter().any(Option::is_some) || records.is_some() {
            shapes.push(forming(group, at, records.as_ref(), Reach::Marked, &given));
        }
    }
    merge_equal(&mut shapes);
    let marked = shapes.len();
    carry_into_marked(&lines, &groups, &places, &given, &mut shapes[..marked]);
    let open_group = |(at, records): &Placed| at.iter().all(Option::is_none) && records.is_none();
    let mut open: Vec<usize> = (0..groups.len()).filter(|&g| open_group(&places[g])).collect();
    join(&lines, &groups, &mut open, &mut shapes);
    let sources: Vec<Vec<Option<String>>> = shapes.iter().map(|s| literals(&lines, s)).collect();
    for g in open {
        let (mut at, mut reach) = carry(&lines, &groups[g], &shapes[..marked], &sources, names.len());
        // A field inside another is read only through it, so it stays out
        // of a shape that does not place the other.
        for f in 0..at.len() {
            if given[f].parent.is_some_and(|p| at[p].is_none()) {
                at[f] = None;
                reach[f] = Reach::Missing;
            }
        }
        let mut shape = forming(&groups[g], &at, None, Reach::Missing, &given);
        shape.reach = reach;
        shapes.push(shape);
    }
    merge_equal(&mut shapes);
    let shapes = merge_near(spec, &lines, &names, &given, shapes);
    let (mut built, parts) = finish(spec, &lines, &names, &given, &shapes)?;
    let examples: Vec<usize> = (0..spec.marked.len()).collect();
    let mut records = records_of(&built, &examples, &parts);
    records.extend(records_of(&built, &data_rows, &parts));
    built.records = records;
    Ok(built)
}

/// Each field's value in one line or record, in order.
type Values = Vec<Option<Value>>;

/// Each marked shape of one group, rebuilt with the fields its marks leave
/// out that another marked shape holds, carried as a line with no mark
/// carries them: by the literals around them at the same occurrence, from a
/// shape it shares a word with. A mark always wins, so a carried field
/// taking a token a marked one takes, or one the line's records take, is
/// left out, as is a field inside another the shape does not place.
fn carry_into_marked(lines: &[Line], groups: &[Group], places: &[Placed], given: &[Given], shapes: &mut [Forming]) {
    let lits: Vec<Vec<Option<String>>> = shapes.iter().map(|s| literals(lines, s)).collect();
    let fields = given.len();
    for i in 0..shapes.len() {
        let rows: Vec<usize> = shapes[i].members.iter().map(|(r, _)| *r).collect();
        let owned: Vec<usize> = (0..groups.len()).filter(|&g| groups[g].rows.iter().all(|r| rows.contains(r))).collect();
        let [g] = owned[..] else { continue };
        if groups[g].rows.len() != rows.len() {
            continue;
        }
        let (at, records) = &places[g];
        let recorded = |f: usize| records.as_ref().is_some_and(|(_, body)| body.fields().contains(&f));
        let (carried, reach) = carry(lines, &groups[g], shapes, &lits, fields);
        let mut merged = at.clone();
        let mut reaches: Vec<Reach> =
            (0..fields).map(|f| if at[f].is_some() || recorded(f) { Reach::Marked } else { Reach::Missing }).collect();
        let mut changed = false;
        for f in 0..fields {
            let Some(c) = &carried[f] else { continue };
            if at[f].is_some() || recorded(f) {
                continue;
            }
            let t = c.tokens();
            let overlaps = |u: Range<usize>| t.start < u.end && u.start < t.end;
            let clashes = at.iter().flatten().any(|a| overlaps(a.tokens()))
                || records.as_ref().is_some_and(|(run, _)| overlaps(run.clone()));
            if !clashes {
                merged[f] = Some(c.clone());
                reaches[f] = reach[f];
                changed = true;
            }
        }
        for f in 0..fields {
            if at[f].is_none() && given[f].parent.is_some_and(|p| merged[p].is_none()) {
                merged[f] = None;
                reaches[f] = Reach::Missing;
            }
        }
        if changed {
            let mut shape = forming(&groups[g], &merged, records.as_ref(), Reach::Marked, given);
            shape.reach = reaches;
            shapes[i] = shape;
        }
    }
}

/// Each row's values cut into the records it holds, as [`record_parts`]
/// cuts a match's.
type Parts = Vec<Vec<Values>>;

/// The records `rows`, a sequence of indexes into `built.rows`, hold, read
/// from each row's `parts` as [`records_from`] reads them. A line with no
/// shape belongs to no record.
fn records_of(built: &Built, rows: &[usize], parts: &Parts) -> Vec<Record> {
    let shaped = rows.iter().filter(|&&r| built.rows[r].shape.is_some());
    records_from(&built.fields, shaped.flat_map(|&r| parts[r].iter().map(move |part| (r, part))))
}

/// The records a sequence of parts holds, each part a line and one record's
/// values in it as [`record_parts`] cuts a match's, read as
/// `ConvertFrom-String` reads records: a part holding a field that begins a
/// record begins one, a part after it joins it as [`join_record`] joins
/// one, and a part before the first begins none. Where no field begins a
/// record, each part is one.
pub fn records_from<'a>(fields: &[Field], parts: impl IntoIterator<Item = (usize, &'a Values)>) -> Vec<Record> {
    let starters: Vec<usize> = (0..fields.len()).filter(|&f| fields[f].starts_record).collect();
    let mut records: Vec<Record> = Vec::new();
    let mut open = false;
    for (line, part) in parts {
        let begins = starters.is_empty() || starters.iter().any(|&f| part[f].is_some());
        if begins {
            records.push(Record { lines: vec![line], values: part.clone() });
            open = true;
        } else if open {
            let record = records.last_mut().expect("an open record is the last");
            if record.lines.last() != Some(&line) {
                record.lines.push(line);
            }
            join_record(fields, &mut record.values, part);
        }
    }
    records
}

/// The records `fields` read from `input` through `pattern` lexed under
/// `shapes`: each match's values cut into the records it holds as
/// [`record_parts`] cuts them, on the line the match starts on, counted from
/// zero, and those parts made into records as [`records_from`] makes them.
///
/// # Errors
///
/// A field reads through an accessor that is not one.
pub fn read_records(
    fields: &[Field],
    pattern: &crate::ast::Pattern,
    shapes: &ShapeSet,
    input: &str,
) -> Result<Vec<Record>, String> {
    let bytes = input.as_bytes();
    let spans = crate::engine::scan_with_shapes(pattern, bytes, shapes);
    let matches = if shapes.is_empty() {
        crate::engine::captures_with_lists(pattern, bytes, &spans)
    } else {
        crate::engine::captures_with_shapes_and_lists(pattern, bytes, shapes, &spans)
    };
    let mut parts: Vec<(usize, Values)> = Vec::new();
    let (mut line, mut counted) = (0, 0);
    for m in &matches {
        line += bytes[counted..m.start].iter().filter(|&&b| b == b'\n').count();
        counted = m.start;
        for part in record_parts(fields, m, input)? {
            parts.push((line, part));
        }
    }
    Ok(records_from(fields, parts.iter().map(|(line, part)| (*line, part))))
}

/// Join `part`, a later line's values, to the open `record`, as
/// `ConvertFrom-String` does: a field keeps its first value in a record,
/// but a field holding the one that begins the record, as a mark spanning
/// the record's lines does, takes its text on each line, joined with a
/// newline.
pub fn join_record(fields: &[Field], record: &mut [Option<Value>], part: &[Option<Value>]) {
    for (f, (mine, theirs)) in record.iter_mut().zip(part).enumerate() {
        let spans = fields.iter().any(|g| g.starts_record && g.parent == Some(f));
        match (mine.as_mut(), theirs) {
            (None, _) => mine.clone_from(theirs),
            (Some(Value::One(text)), Some(Value::One(more))) if spans => {
                text.push('\n');
                text.push_str(more);
            }
            _ => {}
        }
    }
}

/// What the marks say of a field beyond its name.
#[derive(Clone, Default)]
struct Given {
    hint: Option<Hint>,
    type_name: Option<String>,
    starts_record: bool,
    /// The field whose mark encloses this one's.
    parent: Option<usize>,
}

/// Each mark of `marked` by its field's name: its own, or for a mark inside
/// another the other's field name, a dot, and its own.
fn field_names(marked: &Marked) -> Vec<String> {
    let mut full: Vec<String> = Vec::with_capacity(marked.marks.len());
    for m in &marked.marks {
        let name = match m.parent {
            Some(p) => format!("{}.{}", full[p], m.name),
            None => m.name.clone(),
        };
        full.push(name);
    }
    full
}

/// `name` added to the fields with what `said` says of it, or what it says
/// merged into the field already there: the first type given, and a record
/// begun by any mark.
fn add_field(names: &mut Vec<String>, given: &mut Vec<Given>, name: String, said: Given) {
    match names.iter().position(|n| *n == name) {
        Some(i) => {
            let known = &mut given[i];
            if known.hint.is_none() {
                known.hint = said.hint;
                known.type_name = said.type_name;
            }
            known.starts_record |= said.starts_record;
        }
        None => {
            names.push(name);
            given.push(said);
        }
    }
}

/// The fields in the order first marked or hinted, with what the marks say
/// of each: the first type one gives it, whether any writes it as
/// beginning a record, and the field whose mark encloses it.
fn fields_of(spec: &Spec<'_>) -> Result<(Vec<String>, Vec<Given>), BuildError> {
    let mut names: Vec<String> = Vec::new();
    let mut given: Vec<Given> = Vec::new();
    for example in spec.marked {
        let full = field_names(example);
        for (k, name) in example.marks.iter().zip(&full) {
            if !is_name(&k.name) {
                return Err(BuildError::Name(k.name.clone()));
            }
            let parent = k.parent.map(|p| {
                names.iter().position(|n| *n == full[p]).expect("a mark's field is known before the marks inside it")
            });
            let said = Given { hint: k.hint, type_name: k.type_name.clone(), starts_record: k.starts_record, parent };
            add_field(&mut names, &mut given, name.clone(), said);
        }
    }
    for (name, _) in spec.hints {
        if !is_name(name) {
            return Err(BuildError::Name(name.clone()));
        }
        add_field(&mut names, &mut given, name.clone(), Given::default());
    }
    Ok((names, given))
}

/// Whether a capture can take `name`: a letter or `_`, then letters, digits
/// and `_`.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// The significant tokens of `text`, lexed under `shapes`.
fn tokens_of(text: &str, shapes: &ShapeSet) -> Vec<Tok> {
    let bytes = text.as_bytes();
    let toks = if shapes.is_empty() {
        crate::lexer::lex(bytes)
    } else {
        crate::lexer::lex_with_shapes(bytes, &crate::lexer::blob_runs(bytes), shapes, 0)
    };
    toks.iter()
        .filter(|t| t.is_significant())
        .map(|t| Tok { kind: t.kind, start: t.start(), end: t.end() })
        .collect()
}

/// The marked examples, then every line that is not one of them, lexed;
/// with the row of each of `spec.lines`, a line that is a marked example
/// taking that example's row.
fn lines_of(spec: &Spec<'_>, fields: usize) -> (Vec<Line>, Vec<usize>) {
    let line = |text: &str| Line {
        text: text.to_string(),
        toks: tokens_of(text, spec.shapes),
        want: vec![None; fields],
        records: None,
    };
    let mut lines: Vec<Line> = spec.marked.iter().map(|m| line(&m.text)).collect();
    let mut rows = Vec::with_capacity(spec.lines.len());
    for text in spec.lines {
        match spec.marked.iter().position(|m| m.text == *text) {
            Some(example) => rows.push(example),
            None => {
                rows.push(lines.len());
                lines.push(line(text));
            }
        }
    }
    (lines, rows)
}

/// Records a marked line repeats: the tokens they take, the record they
/// repeat, and the indexes of the marks inside them.
type Repeat = (Range<usize>, Body, Vec<usize>);

/// The records a marked example repeats, where a field that begins a record
/// is marked more than once at its top: the line cut at each of those marks
/// into records of the same fields in the same order, each record reaching
/// past its last field over the words every record has there, the words
/// between each record and the next its separator. With the tokens the
/// records take, the record the first holds, and the marks inside them.
fn repeated(
    line: &Line,
    example: &Marked,
    full: &[String],
    names: &[String],
    row: usize,
) -> Result<Option<Repeat>, BuildError> {
    let marks = &example.marks;
    let top: Vec<usize> = (0..marks.len()).filter(|&k| marks[k].parent.is_none()).collect();
    let marked = |name: &str| top.iter().filter(|&&k| marks[k].name == name).count();
    let Some(cut) = top.iter().map(|&k| &marks[k]).find(|m| m.starts_record && marked(&m.name) > 1) else {
        return Ok(None);
    };
    let refuse = || BuildError::Records { field: cut.name.clone(), line: row };
    let at_top: Vec<usize> = (0..top.len()).filter(|&p| marks[top[p]].name == cut.name).collect();
    let width = at_top[1] - at_top[0];
    let first: Vec<&str> = top[at_top[0]..at_top[0] + width].iter().map(|&k| marks[k].name.as_str()).collect();
    if (1..first.len()).any(|i| first[..i].contains(&first[i])) {
        return Err(refuse());
    }
    let mut segments: Vec<&[usize]> = Vec::with_capacity(at_top.len());
    for (i, &p) in at_top.iter().enumerate() {
        let segment = top.get(p..p + width).ok_or_else(refuse)?;
        let follows = at_top.get(i + 1).is_none_or(|&next| next == p + width);
        let same = segment.iter().map(|&k| marks[k].name.as_str()).eq(first.iter().copied());
        let holds = segment.iter().any(|&k| marks.iter().any(|m| m.parent == Some(k)));
        if !follows || !same || holds {
            return Err(refuse());
        }
        segments.push(segment);
    }
    let mut runs: Vec<Vec<Range<usize>>> = Vec::with_capacity(segments.len());
    for segment in &segments {
        let mut here = Vec::with_capacity(segment.len());
        for &k in *segment {
            match line.at_of(marks[k].span.clone()) {
                Some(At::Run(run)) => here.push(run),
                _ => return Err(refuse()),
            }
        }
        runs.push(here);
    }
    let start = |i: usize| runs[i][0].start;
    let end = |i: usize| runs[i][runs[i].len() - 1].end;
    let texts = |from: usize, to: usize| -> Vec<&str> { (from..to).map(|t| line.tok_text(t)).collect() };
    let last = segments.len() - 1;
    let between: Vec<Vec<&str>> = (0..last).map(|i| texts(end(i), start(i + 1))).collect();
    let tail = texts(end(last), line.toks.len());
    let fewest = between.iter().map(Vec::len).min().expect("a record repeats at least twice");
    let mut shared = fewest.saturating_sub(1).min(tail.len());
    while shared > 0 && !between.iter().all(|b| b[..shared] == tail[..shared]) {
        shared -= 1;
    }
    let sep: Vec<String> = between[0][shared..].iter().map(|s| (*s).to_string()).collect();
    if sep.is_empty() || between.iter().any(|b| b[shared..] != between[0][shared..]) {
        return Err(refuse());
    }
    let mut skeleton = Vec::new();
    let mut lits = Vec::new();
    let (mut t, mut k) = (start(0), 0);
    while t < end(0) + shared {
        if k < runs[0].len() && runs[0][k].start == t {
            let name = &full[segments[0][k]];
            skeleton.push(Elem::Field(names.iter().position(|n| n == name).expect("every mark's name is a field")));
            lits.push(None);
            t = runs[0][k].end;
            k += 1;
        } else {
            skeleton.push(Elem::Col(line.toks[t].kind));
            lits.push(Some(line.tok_text(t).to_string()));
            t += 1;
        }
    }
    let body = Body { skeleton, lits, sep };
    let run = start(0)..end(last) + shared;
    let pieces = line.pieces(&run, &body).ok_or_else(refuse)?;
    let agree = pieces.len() == runs.len()
        && pieces.iter().zip(&runs).all(|(piece, marked)| {
            let placed: Vec<&Range<usize>> =
                body.skeleton.iter().zip(piece).filter(|(e, _)| e.field().is_some()).map(|(_, r)| r).collect();
            placed.len() == marked.len() && placed.iter().zip(marked).all(|(a, b)| *a == b)
        });
    if !agree {
        return Err(refuse());
    }
    let inside = segments.iter().flat_map(|s| s.iter().copied()).collect();
    Ok(Some((run, body, inside)))
}

/// Place every mark of the marked examples, which are the first lines; the
/// marks inside records an example repeats are placed by the records.
fn mark(spec: &Spec<'_>, names: &[String], lines: &mut [Line]) -> Result<(), BuildError> {
    for (row, example) in spec.marked.iter().enumerate() {
        let full = field_names(example);
        let mut inside: Vec<usize> = Vec::new();
        if let Some((run, body, marks)) = repeated(&lines[row], example, &full, names, row)? {
            lines[row].records = Some((run, body));
            inside = marks;
        }
        for (k, (m, name)) in example.marks.iter().zip(&full).enumerate() {
            let field = || name.clone();
            let f = names.iter().position(|n| n == name).expect("every mark's name is a field");
            let Some(at) = lines[row].at_of(m.span.clone()) else {
                return Err(BuildError::Split { field: field(), line: row });
            };
            if let Some(hint) = m.hint {
                let value = example.text[m.span.clone()].trim();
                let toks = tokens_of(value, spec.shapes);
                let fits = match (hint.kind(), &toks[..]) {
                    (None, _) => true,
                    (Some(kind), [one]) => one.kind == kind,
                    (Some(_), _) => false,
                };
                if !fits {
                    return Err(BuildError::Mistyped { field: field(), hint, value: value.to_string(), line: row });
                }
            }
            if inside.contains(&k) {
                continue;
            }
            let at = match lines[row].want[f].take() {
                None => at,
                Some(Want::Marked(before)) => {
                    lines[row].listed(before, at).ok_or_else(|| BuildError::Repeated { field: field(), line: row })?
                }
                Some(Want::Found(..)) => unreachable!("marks are placed before any hint"),
            };
            lines[row].want[f] = Some(Want::Marked(at));
        }
    }
    Ok(())
}

/// Place every hinted value anywhere it occurs in a line.
fn hint(spec: &Spec<'_>, names: &[String], lines: &mut [Line]) -> Result<(), BuildError> {
    for (name, values) in spec.hints {
        let f = names.iter().position(|n| n == name).expect("every hint's name is a field");
        for value in values {
            let value = value.trim();
            let mut found = false;
            for (row, line) in lines.iter_mut().enumerate() {
                let mut here: Vec<At> = Vec::new();
                if !value.is_empty() {
                    for (at, _) in line.text.match_indices(value) {
                        if let Some(place) = line.at_of(at..at + value.len())
                            && !here.iter().any(|h| h.same(&place))
                        {
                            here.push(place);
                        }
                    }
                }
                if here.is_empty() {
                    continue;
                }
                found = true;
                line.want[f] = Some(match line.want[f].take() {
                    None => Want::Found(here, value.to_string()),
                    Some(Want::Marked(at)) if here.iter().any(|h| h.same(&at)) => Want::Marked(at),
                    Some(Want::Marked(_)) => return Err(BuildError::Disagree { field: name.clone(), line: row }),
                    Some(Want::Found(mut seen, first)) => {
                        for place in here {
                            if !seen.iter().any(|s| s.same(&place)) {
                                seen.push(place);
                            }
                        }
                        Want::Found(seen, first)
                    }
                });
            }
            if !found {
                return Err(BuildError::Absent { field: name.clone(), value: value.to_string() });
            }
        }
    }
    Ok(())
}

/// The lines holding a token, grouped by the kinds of their tokens, the
/// groups in the order of their first lines.
fn groups_of(lines: &[Line]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut index: HashMap<Vec<TokenKind>, usize> = HashMap::new();
    for (row, line) in lines.iter().enumerate() {
        if line.toks.is_empty() {
            continue;
        }
        let kinds: Vec<TokenKind> = line.toks.iter().map(|t| t.kind).collect();
        match index.get(&kinds) {
            Some(&g) => groups[g].rows.push(row),
            None => {
                index.insert(kinds.clone(), groups.len());
                groups.push(Group { rows: vec![row], kinds });
            }
        }
    }
    groups
}

/// Whether field `a` is `b`'s parent, or its parent's, and so on.
fn encloses(given: &[Given], a: usize, b: usize) -> bool {
    let mut at = given[b].parent;
    while let Some(p) = at {
        if p == a {
            return true;
        }
        at = given[p].parent;
    }
    false
}

/// The position of each field in a group's lines, and the records they
/// repeat with the tokens those take.
type Placed = (Vec<Option<At>>, Option<(Range<usize>, Body)>);

/// The position of each field in every line of `group`, from what its lines
/// say: a mark or a value found once fixes it, and a value found at several
/// places is the one of them every line of the group has. Two fields share
/// no token unless one's mark is inside the other's, which then takes whole
/// tokens. The records a marked line repeats are the group's, which every
/// marked line of it repeats alike and no other field is in.
fn places_of(lines: &[Line], group: &Group, names: &[String], given: &[Given]) -> Result<Placed, BuildError> {
    let mut out: Vec<Option<At>> = Vec::with_capacity(names.len());
    for (f, name) in names.iter().enumerate() {
        let disagree = |line: usize| BuildError::Disagree { field: name.clone(), line };
        let mut fixed: Option<At> = None;
        let mut open: Vec<(usize, &[At], &str)> = Vec::new();
        for &row in &group.rows {
            let at = match &lines[row].want[f] {
                None => continue,
                Some(Want::Marked(at)) => at,
                Some(Want::Found(ats, _)) if ats.len() == 1 => &ats[0],
                Some(Want::Found(ats, value)) => {
                    open.push((row, ats, value));
                    continue;
                }
            };
            fixed = Some(match &fixed {
                None => at.clone(),
                Some(prev) => prev.meet(at).ok_or_else(|| disagree(row))?,
            });
        }
        if fixed.is_none()
            && let Some(&(row, first, value)) = open.first()
        {
            let common: Vec<&At> =
                first.iter().filter(|a| open.iter().all(|(_, ats, _)| ats.iter().any(|b| b.same(a)))).collect();
            fixed = match common[..] {
                [one] => Some(one.clone()),
                [] => return Err(disagree(row)),
                _ => {
                    return Err(BuildError::Twice { field: name.clone(), value: value.to_string(), line: row });
                }
            };
        }
        if let Some(mut at) = fixed {
            for &(row, ats, _) in &open {
                at = ats.iter().find_map(|b| at.meet(b)).ok_or_else(|| disagree(row))?;
            }
            fixed = Some(at);
        }
        out.push(fixed);
    }
    for a in 0..out.len() {
        for b in a + 1..out.len() {
            if let (Some(x), Some(y)) = (&out[a], &out[b]) {
                let nested = encloses(given, a, b) || encloses(given, b, a);
                let (x, y) = (x.tokens(), y.tokens());
                if !nested && x.start < y.end && y.start < x.end {
                    return Err(BuildError::Overlap {
                        first: names[a].clone(),
                        second: names[b].clone(),
                        line: group.rows[0],
                    });
                }
            }
        }
    }
    for (f, at) in out.iter().enumerate() {
        let holds = given.iter().any(|g| g.parent == Some(f));
        if holds && !matches!(at, None | Some(At::Run(_))) {
            return Err(BuildError::Nested { field: names[f].clone(), line: group.rows[0] });
        }
    }
    let mut records: Option<(Range<usize>, Body)> = None;
    for &row in &group.rows {
        let Some(here) = &lines[row].records else { continue };
        match &records {
            None => records = Some(here.clone()),
            Some(seen) if seen == here => {}
            Some((_, body)) => {
                let field = names[body.fields()[0]].clone();
                return Err(BuildError::Records { field, line: row });
            }
        }
    }
    if let Some((run, body)) = &records {
        for (f, at) in out.iter().enumerate() {
            let t = at.as_ref().map(At::tokens);
            if t.is_some_and(|t| t.start < run.end && run.start < t.end) {
                let second = names[body.fields()[0]].clone();
                return Err(BuildError::Overlap { first: names[f].clone(), second, line: group.rows[0] });
            }
        }
    }
    Ok((out, records))
}

/// The shape a group's lines make with its fields at `at` and the records
/// they repeat, every line a member, and `reach` for each field it holds. A
/// field holding a field placed here spans the elements inside it rather
/// than being one.
fn forming(
    group: &Group,
    at: &[Option<At>],
    records: Option<&(Range<usize>, Body)>,
    reach: Reach,
    given: &[Given],
) -> Forming {
    let spanning: Vec<bool> = (0..at.len())
        .map(|f| at[f].is_some() && (0..at.len()).any(|c| given[c].parent == Some(f) && at[c].is_some()))
        .collect();
    let mut skeleton: Vec<Elem> = Vec::new();
    let mut layout: Vec<Range<usize>> = Vec::new();
    let mut parts: HashMap<usize, Vec<String>> = HashMap::new();
    let mut i = 0;
    while i < group.kinds.len() {
        if let Some((run, body)) = records
            && run.start == i
        {
            skeleton.push(Elem::Records(Box::new(body.clone())));
            layout.push(run.clone());
            i = run.end;
            continue;
        }
        let here = at.iter().enumerate().find_map(|(f, a)| match a {
            Some(a) if a.tokens().start == i && !spanning[f] => Some((f, a)),
            _ => None,
        });
        match here {
            Some((f, At::Run(run))) => {
                skeleton.push(Elem::Field(f));
                layout.push(run.clone());
                i = run.end;
            }
            Some((f, At::List(run, sep))) => {
                skeleton.push(Elem::List(f, sep.clone()));
                layout.push(run.clone());
                i = run.end;
            }
            Some((f, At::Part(t, accessors))) => {
                skeleton.push(Elem::Part(f, group.kinds[*t]));
                layout.push(*t..*t + 1);
                parts.insert(f, accessors.clone());
                i += 1;
            }
            None => {
                skeleton.push(Elem::Col(group.kinds[i]));
                layout.push(i..i + 1);
                i += 1;
            }
        }
    }
    let mut spans: Vec<(usize, Range<usize>)> = Vec::new();
    for (f, a) in at.iter().enumerate() {
        if let (true, Some(At::Run(run))) = (spanning[f], a) {
            let first = layout.iter().position(|l| l.start == run.start).expect("a spanning field starts at an element");
            let last = layout.iter().rposition(|l| l.end == run.end).expect("a spanning field ends at an element");
            spans.push((f, first..last + 1));
        }
    }
    spans.sort_by_key(|(_, span)| (span.start, Reverse(span.len())));
    let recorded = |f: usize| records.is_some_and(|(_, body)| body.fields().contains(&f));
    let reach = (0..at.len()).map(|f| if at[f].is_some() || recorded(f) { reach } else { Reach::Missing }).collect();
    let members = group.rows.iter().map(|&r| (r, layout.clone())).collect();
    Forming { skeleton, spans, parts, members, reach }
}

/// Merge each shape into the first before it with the same elements.
fn merge_equal(shapes: &mut Vec<Forming>) {
    let mut i = 0;
    while i < shapes.len() {
        let mut j = i + 1;
        while j < shapes.len() {
            let parts = if shapes[j].skeleton == shapes[i].skeleton && shapes[j].spans == shapes[i].spans {
                meet_parts(&shapes[i].parts, &shapes[j].parts)
            } else {
                None
            };
            match parts {
                Some(parts) => {
                    let other = shapes.remove(j);
                    let shape = &mut shapes[i];
                    shape.parts = parts;
                    shape.members.extend(other.members);
                    for (mine, theirs) in shape.reach.iter_mut().zip(other.reach) {
                        *mine = stronger(*mine, theirs);
                    }
                }
                None => j += 1,
            }
        }
        i += 1;
    }
}

/// The accessors two shapes of the same elements both read each part field
/// with, `None` where a field shares none.
fn meet_parts(a: &HashMap<usize, Vec<String>>, b: &HashMap<usize, Vec<String>>) -> Option<HashMap<usize, Vec<String>>> {
    let mut out = HashMap::with_capacity(a.len());
    for (f, x) in a {
        let both: Vec<String> = x.iter().filter(|n| b.get(f).is_some_and(|y| y.contains(n))).cloned().collect();
        if both.is_empty() {
            return None;
        }
        out.insert(*f, both);
    }
    Some(out)
}

/// The more direct of two reaches: a mark, then an anchor, then none.
fn stronger(a: Reach, b: Reach) -> Reach {
    match (a, b) {
        (Reach::Marked, _) | (_, Reach::Marked) => Reach::Marked,
        (Reach::Anchored(s), _) | (_, Reach::Anchored(s)) => Reach::Anchored(s),
        _ => Reach::Missing,
    }
}

/// The text every member of `shape` that has a token at a column element has
/// there, where they all have one.
fn literals(lines: &[Line], shape: &Forming) -> Vec<Option<String>> {
    (0..shape.skeleton.len())
        .map(|e| {
            if !matches!(shape.skeleton[e], Elem::Col(_)) {
                return None;
            }
            let mut texts = shape
                .members
                .iter()
                .filter(|(_, layout)| !layout[e].is_empty())
                .map(|(r, layout)| lines[*r].tok_text(layout[e].start));
            let first = texts.next()?;
            texts.all(|t| t == first).then(|| first.to_string())
        })
        .collect()
}

/// The shapes with each pair merged whose elements align but for literal
/// words one of them has and the other lacks, those words made optional; a
/// merge is kept only where the pattern it gives still reads every line's
/// fields back as placed and rejects every counter-example.
fn merge_near(
    spec: &Spec<'_>,
    lines: &[Line],
    names: &[String],
    given: &[Given],
    mut shapes: Vec<Forming>,
) -> Vec<Forming> {
    let mut i = 0;
    while i < shapes.len() {
        let mut j = i + 1;
        while j < shapes.len() {
            let merged = merged_near(lines, &shapes[i], &shapes[j]);
            let kept = merged.and_then(|merged| {
                let mut trial = shapes.clone();
                trial[i] = merged;
                trial.remove(j);
                finish(spec, lines, names, given, &trial).is_ok().then_some(trial)
            });
            match kept {
                Some(trial) => shapes = trial,
                None => j += 1,
            }
        }
        i += 1;
    }
    shapes
}

/// `a` and `b` as one shape, where every element of the one pairs with the
/// like element of the other, a field with the same field and a word with a
/// word of the same kind and text, and the only elements left over are
/// literal words of one of them, which become optional; `None` otherwise,
/// where they already have the same elements, or where either holds a field
/// spanning others or records repeating.
fn merged_near(lines: &[Line], a: &Forming, b: &Forming) -> Option<Forming> {
    let records = |s: &Forming| s.skeleton.iter().any(|e| matches!(e, Elem::Records(_)));
    if !a.spans.is_empty() || !b.spans.is_empty() || records(a) || records(b) {
        return None;
    }
    let (a_lits, b_lits) = (literals(lines, a), literals(lines, b));
    let pairs = match aligned_skipping(a, &a_lits, b, &b_lits) {
        Some(pairs) => pairs,
        None => aligned_skipping(b, &b_lits, a, &a_lits)?.into_iter().map(|(x, y)| (y, x)).collect(),
    };
    let parts = meet_parts(&a.parts, &b.parts)?;
    let skeleton: Vec<Elem> = pairs
        .iter()
        .map(|&(x, y)| match (x, y) {
            (Some(x), _) => a.skeleton[x].clone(),
            (None, Some(y)) => b.skeleton[y].clone(),
            (None, None) => unreachable!("every merged element is in one shape at least"),
        })
        .collect();
    let lay = |shape: &Forming, side: fn(&Paired) -> Option<usize>| {
        shape
            .members
            .iter()
            .map(|(r, layout)| {
                let mut at = 0;
                let merged: Vec<Range<usize>> = pairs
                    .iter()
                    .map(|p| match side(p) {
                        Some(e) => {
                            at = layout[e].end;
                            layout[e].clone()
                        }
                        None => at..at,
                    })
                    .collect();
                (*r, merged)
            })
            .collect::<Vec<_>>()
    };
    let mut members = lay(a, |p| p.0);
    members.extend(lay(b, |p| p.1));
    let reach = a.reach.iter().zip(&b.reach).map(|(x, y)| stronger(*x, *y)).collect();
    Some(Forming { skeleton, spans: Vec::new(), parts, members, reach })
}

/// One element of a merged shape: its index in each of the two shapes, `None`
/// in the one that lacks it.
type Paired = (Option<usize>, Option<usize>);

/// How `long`'s elements pair with `short`'s when `short` is `long` with
/// some of `long`'s literal words taken out: each pair of indexes, `None`
/// on `short`'s side for a word it lacks. `None` where no such pairing
/// exists or no word is left over.
fn aligned_skipping(
    long: &Forming,
    long_lits: &[Option<String>],
    short: &Forming,
    short_lits: &[Option<String>],
) -> Option<Vec<Paired>> {
    if long.skeleton.len() <= short.skeleton.len() {
        return None;
    }
    let like = |x: usize, y: usize| match (&long.skeleton[x], &short.skeleton[y]) {
        (Elem::Col(k), Elem::Col(l)) => {
            k == l
                && match (&long_lits[x], &short_lits[y]) {
                    (Some(p), Some(q)) => fold_eq(p, q),
                    (None, None) => true,
                    _ => false,
                }
        }
        (p, q) => p == q,
    };
    let (m, n) = (long.skeleton.len(), short.skeleton.len());
    // Whether long[x..] pairs with short[y..], skipping only literal words
    // of long's.
    let mut can = vec![vec![false; n + 1]; m + 1];
    can[m][n] = true;
    for x in (0..m).rev() {
        for y in (0..=n).rev() {
            let skip = matches!(long.skeleton[x], Elem::Col(_)) && long_lits[x].is_some() && can[x + 1][y];
            let pair = y < n && like(x, y) && can[x + 1][y + 1];
            can[x][y] = skip || pair;
        }
    }
    if !can[0][0] {
        return None;
    }
    let mut pairs = Vec::with_capacity(m);
    let (mut x, mut y) = (0, 0);
    while x < m {
        if y < n && like(x, y) && can[x + 1][y + 1] {
            pairs.push((Some(x), Some(y)));
            y += 1;
        } else {
            pairs.push((Some(x), None));
        }
        x += 1;
    }
    Some(pairs)
}

/// The text every line of `group` has at each token, where they all have
/// one.
fn group_literals(lines: &[Line], group: &Group) -> Vec<Option<String>> {
    (0..group.kinds.len())
        .map(|c| {
            let first = lines[group.rows[0]].tok_text(c);
            group.rows.iter().all(|&r| lines[r].tok_text(c) == first).then(|| first.to_string())
        })
        .collect()
}

fn fold_eq(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Join each group with no field to the shape it aligns with best, until no
/// group joins: a join can only widen a shape, so a group refused earlier is
/// tried again. The groups still open are left in `open`.
fn join(lines: &[Line], groups: &[Group], open: &mut Vec<usize>, shapes: &mut [Forming]) {
    loop {
        let mut joined = false;
        let mut still: Vec<usize> = Vec::with_capacity(open.len());
        for &g in open.iter() {
            let group = &groups[g];
            let glits = group_literals(lines, group);
            let mut best: Option<(usize, Vec<Range<usize>>, Score)> = None;
            for (s, shape) in shapes.iter().enumerate() {
                let lits = literals(lines, shape);
                if let Some((layout, score)) = place(&shape.skeleton, &lits, &group.kinds, &glits)
                    && best.as_ref().is_none_or(|(_, _, b)| score > *b)
                    && records_align(lines, &group.rows, &shape.skeleton, &layout)
                {
                    best = Some((s, layout, score));
                }
            }
            match best {
                Some((s, layout, _)) => {
                    shapes[s].members.extend(group.rows.iter().map(|&r| (r, layout.clone())));
                    joined = true;
                }
                None => still.push(g),
            }
        }
        *open = still;
        if !joined {
            return;
        }
    }
}

/// Whether every records element of `skeleton` takes, in each line of
/// `rows`, tokens `layout` says that cut into records of its body.
fn records_align(lines: &[Line], rows: &[usize], skeleton: &[Elem], layout: &[Range<usize>]) -> bool {
    skeleton.iter().zip(layout).all(|(elem, run)| match elem {
        Elem::Records(body) => rows.iter().all(|&r| lines[r].pieces(run, body).is_some()),
        _ => true,
    })
}

/// One state of an alignment: its best score, how many alignments reach
/// it with that score (two meaning more than one), and the column the best
/// came from.
#[derive(Clone, Copy)]
struct Cell {
    score: Score,
    ways: u8,
    from: usize,
}

/// Offer `slot` the state `from` is at, moved on by `agree` literals and
/// `empty` empty fields, arriving from column `column`.
fn offer(slot: &mut Option<Cell>, from: Cell, agree: u32, empty: u32, column: usize) {
    let score = (from.score.0 + agree, Reverse((from.score.1).0 + empty));
    match slot {
        Some(cell) if cell.score > score => {}
        Some(cell) if cell.score == score => cell.ways = (cell.ways + from.ways).min(2),
        _ => *slot = Some(Cell { score, ways: from.ways, from: column }),
    }
}

/// The tokens each element of a shape takes among a group's columns: the
/// alignment agreeing on the most literals, then leaving the fewest fields
/// empty, where exactly one alignment does. A column element takes one
/// token of its kind and is refused where both sides are literals that
/// differ under case; a part takes one token of its kind; a field takes any
/// run, empty included.
fn place(
    skeleton: &[Elem],
    shape_lits: &[Option<String>],
    kinds: &[TokenKind],
    group_lits: &[Option<String>],
) -> Option<(Vec<Range<usize>>, Score)> {
    let (m, n) = (skeleton.len(), kinds.len());
    let mut dp: Vec<Vec<Option<Cell>>> = vec![vec![None; n + 1]; m + 1];
    dp[0][0] = Some(Cell { score: (0, Reverse(0)), ways: 1, from: 0 });
    for e in 0..m {
        for c in 0..=n {
            let Some(cell) = dp[e][c] else { continue };
            match &skeleton[e] {
                Elem::Col(kind) if c < n && kinds[c] == *kind => {
                    let agree = match (&shape_lits[e], &group_lits[c]) {
                        (Some(a), Some(b)) if fold_eq(a, b) => Some(1),
                        (Some(_), Some(_)) => None,
                        _ => Some(0),
                    };
                    if let Some(agree) = agree {
                        offer(&mut dp[e + 1][c + 1], cell, agree, 0, c);
                    }
                }
                Elem::Part(_, kind) if c < n && kinds[c] == *kind => offer(&mut dp[e + 1][c + 1], cell, 0, 0, c),
                Elem::Field(_) | Elem::List(..) | Elem::Records(_) => {
                    for (to, slot) in dp[e + 1].iter_mut().enumerate().skip(c) {
                        offer(slot, cell, 0, u32::from(to == c), c);
                    }
                }
                _ => {}
            }
        }
    }
    let end = dp[m][n]?;
    if end.ways != 1 {
        return None;
    }
    let mut layout = vec![0..0; m];
    let mut c = n;
    for e in (0..m).rev() {
        let cell = dp[e + 1][c].expect("every state on the one best alignment is reached");
        layout[e] = cell.from..c;
        c = cell.from;
    }
    Some((layout, end.score))
}

/// What is next to a field in a shape.
enum Anchor {
    /// The line's start or end.
    Edge,
    /// A literal token, the given occurrence of its text counted from the
    /// line's start.
    Lit(String, usize),
    /// Nothing that says where the field stops.
    Loose,
}

/// How many tokens of `line` up to and including token `col` read `text`,
/// under case folding.
fn occurrence(line: &Line, col: usize, text: &str) -> usize {
    (0..=col).filter(|&c| fold_eq(line.tok_text(c), text)).count()
}

/// The fields of a group no shape took, each placed between the literals
/// that surround it in a marked shape, found in the group, or next to one
/// of them where the field's tokens are of the same kinds in every line of
/// that shape; with the reach of each. A literal is found in the group only
/// as the same occurrence of its text, counted from the line's start, as
/// in every line of the shape, so a separator the shape repeats places a
/// field between the same two of its occurrences, and a line cut short
/// before one places nothing beside it. A shape places fields only in a
/// group holding one of its word literals: punctuation and the line's
/// edges alone say nothing of which record a line is.
fn carry(
    lines: &[Line],
    group: &Group,
    sources: &[Forming],
    source_lits: &[Vec<Option<String>>],
    fields: usize,
) -> (Vec<Option<At>>, Vec<Reach>) {
    let glits = group_literals(lines, group);
    let n = group.kinds.len();
    let cols_with = |t: &str, k: usize| -> Vec<usize> {
        (0..n)
            .filter(|&c| glits[c].as_deref().is_some_and(|g| fold_eq(g, t)))
            .filter(|&c| group.rows.iter().all(|&r| occurrence(&lines[r], c, t) == k))
            .collect()
    };
    let mut at: Vec<Option<At>> = vec![None; fields];
    let mut reach = vec![Reach::Missing; fields];
    // Whether the group holds a literal of shape `s` with a letter or digit.
    let shares_a_word = |s: usize| {
        source_lits[s].iter().flatten().any(|t| {
            t.chars().any(char::is_alphanumeric) && glits.iter().flatten().any(|g| fold_eq(g, t))
        })
    };
    for f in 0..fields {
        for (s, shape) in sources.iter().enumerate() {
            if !shares_a_word(s) {
                continue;
            }
            let Some(e) = shape.skeleton.iter().position(|el| el.field() == Some(f)) else { continue };
            // The occurrence of the literal at element `i` in every member
            // holding it, where they agree.
            let occurrence_at = |i: usize, t: &str| -> Option<usize> {
                let mut seen: Option<usize> = None;
                for (r, layout) in shape.members.iter().filter(|(_, layout)| !layout[i].is_empty()) {
                    let k = occurrence(&lines[*r], layout[i].start, t);
                    if seen.is_some_and(|prev| prev != k) {
                        return None;
                    }
                    seen = Some(k);
                }
                seen
            };
            let beside = |i: Option<usize>| match i {
                None => Anchor::Edge,
                Some(i) => match (&shape.skeleton[i], &source_lits[s][i]) {
                    (Elem::Col(_), Some(t)) => match occurrence_at(i, t) {
                        Some(k) => Anchor::Lit(t.clone(), k),
                        None => Anchor::Loose,
                    },
                    _ => Anchor::Loose,
                },
            };
            let starts: Option<Vec<usize>> = match beside(e.checked_sub(1)) {
                Anchor::Edge => Some(vec![0]),
                Anchor::Lit(t, k) => Some(cols_with(&t, k).into_iter().map(|c| c + 1).collect()),
                Anchor::Loose => None,
            };
            let ends: Option<Vec<usize>> = match beside((e + 1 < shape.skeleton.len()).then_some(e + 1)) {
                Anchor::Edge => Some(vec![n]),
                Anchor::Lit(t, k) => Some(cols_with(&t, k)),
                Anchor::Loose => None,
            };
            let between = match (starts.as_deref(), ends.as_deref()) {
                (Some(&[l]), Some(rs)) => rs.iter().copied().find(|&r| r > l).map(|r| l..r),
                (Some(ls), Some(&[r])) => ls.iter().copied().rfind(|&l| l < r).map(|l| l..r),
                _ => None,
            };
            let kinds = fixed_kinds(lines, shape, e);
            let beside_one = || -> Option<Range<usize>> {
                let seq = kinds.as_ref()?;
                let k = seq.len();
                if let Some(&[l]) = starts.as_deref()
                    && l + k <= n
                    && group.kinds[l..l + k] == seq[..]
                {
                    return Some(l..l + k);
                }
                if let Some(&[r]) = ends.as_deref()
                    && r >= k
                    && group.kinds[r - k..r] == seq[..]
                {
                    return Some(r - k..r);
                }
                None
            };
            let Some(run) = between.or_else(beside_one) else { continue };
            let taken = at.iter().flatten().any(|a| {
                let t = a.tokens();
                t.start < run.end && run.start < t.end
            });
            if taken {
                continue;
            }
            let placed = match &shape.skeleton[e] {
                Elem::Part(_, kind) if run.len() == 1 && group.kinds[run.start] == *kind => {
                    let reads: Vec<String> = shape.parts[&f]
                        .iter()
                        .filter(|acc| {
                            group.rows.iter().all(|&r| {
                                crate::rewrite::apply_named(acc, lines[r].tok_text(run.start))
                                    .is_ok_and(|v| !v.is_empty())
                            })
                        })
                        .cloned()
                        .collect();
                    (!reads.is_empty()).then_some(At::Part(run.start, reads))
                }
                Elem::Part(..) => None,
                Elem::List(_, sep) => Some(At::List(run, sep.clone())),
                _ => Some(At::Run(run)),
            };
            if let Some(placed) = placed {
                at[f] = Some(placed);
                reach[f] = Reach::Anchored(s);
                break;
            }
        }
    }
    (at, reach)
}

/// The kinds of the tokens element `e` of `shape` takes, where they are the
/// same, and not none, in every member that has it.
fn fixed_kinds(lines: &[Line], shape: &Forming, e: usize) -> Option<Vec<TokenKind>> {
    let mut runs = shape
        .members
        .iter()
        .filter(|(_, layout)| !layout[e].is_empty())
        .map(|(r, layout)| lines[*r].toks[layout[e].clone()].iter().map(|t| t.kind).collect::<Vec<_>>());
    let first = runs.next()?;
    runs.all(|k| k == first).then_some(first)
}

/// One element of a branch as the pattern spells it.
#[derive(Default)]
struct Spelled {
    /// The element with no value range.
    plain: String,
    /// The element with the range its values share, where one exists.
    ranged: Option<String>,
    /// Each value class of the library every value of a field element
    /// belongs to, with the element spelled as it.
    classes: Vec<(&'static str, String)>,
    /// Whether the element is a literal.
    literal: bool,
    /// Whether the element is a free-text field.
    free: bool,
}

impl Spelled {
    /// The spellings a counter-example can call for in place of the plain
    /// one: the range, then each class.
    fn alternatives(&self) -> Vec<&str> {
        self.ranged.iter().map(String::as_str).chain(self.classes.iter().map(|(_, s)| s.as_str())).collect()
    }
}

/// The atom naming `kind`, a declared shape by its name, and `.` for a kind
/// no atom names.
fn atom(kind: TokenKind, shapes: &ShapeSet) -> String {
    if let TokenKind::Custom(id) = kind
        && let Some(name) = shapes.name_of(id)
    {
        return format!("\\{{{name}}}");
    }
    match atom_of(kind) {
        Some(atom) => atom,
        None => ".".to_string(),
    }
}

/// Each element of `shape` as its branch spells it, a field spanning
/// others opening before its first element and bound after its last. A
/// field is bound under the last part of its name, since TREX names a
/// capture inside a capture after both.
fn spell(
    lines: &[Line],
    shape: &Forming,
    names: &[String],
    shapes: &ShapeSet,
    mint: Mint,
    mints: &[Option<String>],
) -> Vec<Spelled> {
    let bound: Vec<&str> =
        names.iter().map(|n| n.rsplit('.').next().expect("a split yields a part")).collect();
    let mut out = spell_elements(lines, shape, &bound, shapes, mint, mints, true);
    let mut opens: Vec<String> = vec![String::new(); out.len()];
    let mut closes: Vec<String> = vec![String::new(); out.len()];
    for (f, span) in &shape.spans {
        let absent = shape.members.iter().any(|(_, layout)| layout[span.clone()].iter().all(Range::is_empty));
        let close = if absent { format!(")?:{}", bound[*f]) } else { format!("):{}", bound[*f]) };
        opens[span.start].push('(');
        closes[span.end - 1].insert_str(0, &close);
    }
    for (e, spelled) in out.iter_mut().enumerate() {
        let (open, close) = (&opens[e], &closes[e]);
        spelled.plain = format!("{open}{}{close}", spelled.plain);
        spelled.ranged = spelled.ranged.as_deref().map(|r| format!("{open}{r}{close}"));
        for (_, class) in &mut spelled.classes {
            *class = format!("{open}{class}{close}");
        }
    }
    out
}

/// Each element of `shape` as its branch spells it, each field bound under
/// `bound`'s name for it; the last element ends the line where `ends_line`
/// says, as a record's does not.
fn spell_elements(
    lines: &[Line],
    shape: &Forming,
    names: &[&str],
    shapes: &ShapeSet,
    mint: Mint,
    mints: &[Option<String>],
    ends_line: bool,
) -> Vec<Spelled> {
    let last = if ends_line { shape.skeleton.len() - 1 } else { usize::MAX };
    let mut out: Vec<Spelled> = Vec::with_capacity(shape.skeleton.len());
    for (e, elem) in shape.skeleton.iter().enumerate() {
        let runs: Vec<(usize, &[Tok])> =
            shape.members.iter().map(|(r, layout)| (*r, &lines[*r].toks[layout[e].clone()])).collect();
        out.push(match elem {
            Elem::Col(kind) => {
                let texts: Vec<&str> = runs
                    .iter()
                    .filter(|(_, run)| !run.is_empty())
                    .map(|(r, run)| &lines[*r].text[run[0].start..run[0].end])
                    .collect();
                let mut column = spell_column(*kind, &texts, shapes);
                if texts.len() < runs.len() {
                    column.plain.push('?');
                    column.ranged = column.ranged.map(|r| format!("{r}?"));
                } else if !column.literal
                    && let Some(f) = echoed(lines, shape, e)
                {
                    column = Spelled { plain: format!("={}", names[f]), ..Spelled::default() };
                }
                column
            }
            Elem::Part(f, kind) => {
                Spelled { plain: format!("({}):{}", atom(*kind, shapes), names[*f]), ..Spelled::default() }
            }
            Elem::Field(f) => {
                let present: Vec<(Vec<TokenKind>, Vec<&str>)> = runs
                    .iter()
                    .filter(|(_, run)| !run.is_empty())
                    .map(|(r, run)| {
                        let kinds = run.iter().map(|t| t.kind).collect();
                        let texts = run.iter().map(|t| &lines[*r].text[t.start..t.end]).collect();
                        (kinds, texts)
                    })
                    .collect();
                let held: Vec<(usize, Range<usize>)> = shape
                    .members
                    .iter()
                    .filter(|(_, layout)| !layout[e].is_empty())
                    .map(|(r, layout)| (*r, layout[e].clone()))
                    .collect();
                let several = present.iter().all(|(kinds, _)| kinds.len() > 1);
                let library = if several { library_kind(lines, &held, shapes) } else { None };
                let (inner, ranged, free) =
                    field_body(&present, library, e == last, shapes, mint, mints[*f].as_deref());
                let optional = present.len() < runs.len();
                let wrap = |inner: &str| {
                    if optional { format!("({inner})?:{}", names[*f]) } else { format!("({inner}):{}", names[*f]) }
                };
                let classes = if present.iter().all(|(kinds, _)| kinds.len() == 1) {
                    let texts: Vec<&str> = present.iter().map(|(_, t)| t[0]).collect();
                    classes_of(&texts).into_iter().map(|c| (c, wrap(&format!("\\{{{c}}}")))).collect()
                } else {
                    Vec::new()
                };
                Spelled { plain: wrap(&inner), ranged: ranged.as_deref().map(wrap), classes, literal: false, free }
            }
            Elem::List(f, sep) => {
                let mut present: Vec<(Vec<TokenKind>, Vec<&str>)> = Vec::new();
                let mut held: Vec<(usize, Range<usize>)> = Vec::new();
                let mut optional = false;
                for (r, layout) in &shape.members {
                    let line = &lines[*r];
                    let values = line.list_runs(&layout[e], sep);
                    optional |= values.is_empty();
                    for v in values {
                        held.push((*r, v.clone()));
                        let run = &line.toks[v];
                        let kinds = run.iter().map(|t| t.kind).collect();
                        let texts = run.iter().map(|t| &line.text[t.start..t.end]).collect();
                        present.push((kinds, texts));
                    }
                }
                let several = present.iter().all(|(kinds, _)| kinds.len() > 1);
                let library = if several { library_kind(lines, &held, shapes) } else { None };
                let (value, _, free) = field_body(&present, library, false, shapes, mint, mints[*f].as_deref());
                let between: Vec<String> = sep.iter().map(|s| crate::templates::quote(s)).collect();
                let name = names[*f];
                let list = format!("({value}):{name} ({} ({value}):{name})*", between.join(" "));
                let plain = if optional { format!("({list})?") } else { list };
                Spelled { plain, free, ..Spelled::default() }
            }
            Elem::Records(body) => {
                let mut pieces: Vec<(usize, Vec<Range<usize>>)> = Vec::new();
                for (r, layout) in &shape.members {
                    let found = lines[*r].pieces(&layout[e], body).expect("a member's records align with the record");
                    pieces.extend(found.into_iter().map(|piece| (*r, piece)));
                }
                let record = Forming {
                    skeleton: body.skeleton.clone(),
                    spans: Vec::new(),
                    parts: HashMap::new(),
                    members: pieces,
                    reach: Vec::new(),
                };
                let elems = spell_elements(lines, &record, names, shapes, mint, mints, false);
                let one: Vec<&str> = elems.iter().map(|s| s.plain.as_str()).collect();
                let one = one.join(" ");
                let between: Vec<String> = body.sep.iter().map(|s| crate::templates::quote(s)).collect();
                let plain = format!("{one} ({} {one})*", between.join(" "));
                Spelled { plain, free: elems.iter().any(|s| s.free), ..Spelled::default() }
            }
        });
    }
    out
}

/// The field a column element `e` of `shape` repeats: one bound earlier in
/// the branch as one token, whose text the column holds in every member
/// while the field takes at least two values among them, so the equality
/// has held as the value changed. The nearest such field; none inside a
/// field spanning others, nor for a column inside one.
fn echoed(lines: &[Line], shape: &Forming, e: usize) -> Option<usize> {
    let inside = |i: usize| shape.spans.iter().any(|(_, span)| span.contains(&i));
    if inside(e) {
        return None;
    }
    (0..e).rev().find_map(|p| {
        let Elem::Field(f) = shape.skeleton[p] else { return None };
        if inside(p) {
            return None;
        }
        let mut values: Vec<&str> = Vec::with_capacity(shape.members.len());
        for (r, layout) in &shape.members {
            let line = &lines[*r];
            if layout[p].len() != 1 || line.run_text(&layout[p])? != line.run_text(&layout[e])? {
                return None;
            }
            values.push(line.run_text(&layout[p])?);
        }
        values.iter().any(|v| *v != values[0]).then_some(f)
    })
}

/// A column element: the text every member has, the text they share under
/// an orbit rung, else the kind with the range the texts share as a
/// candidate.
fn spell_column(kind: TokenKind, texts: &[&str], shapes: &ShapeSet) -> Spelled {
    let text = texts.iter().all(|t| *t == texts[0]).then(|| texts[0].to_string());
    if let Some(text) = text {
        return Spelled { plain: crate::templates::quote(&text), literal: true, ..Spelled::default() };
    }
    let texts = texts.iter().map(|t| Some((*t).to_string())).collect();
    let column = Column { kinds: vec![kind], text: None, texts };
    let a = atom(kind, shapes);
    match narrowing_of(&column) {
        Some(Narrowing::Folded(group, text)) => {
            let narrowing = Some(Narrowing::Folded(group, text));
            let slot = Slot { kinds: vec![kind], text: None, narrowing, min: 1, max: 1 };
            Spelled { plain: slot.spelled(), literal: true, ..Spelled::default() }
        }
        Some(Narrowing::Range(range)) => {
            Spelled { plain: a.clone(), ranged: Some(format!("{a}{{{range}}}")), ..Spelled::default() }
        }
        None => Spelled { plain: a, ..Spelled::default() },
    }
}

/// One run of a token's bytes as [`minted`] reads it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Run {
    Upper,
    Lower,
    Digit,
    Punct(u8),
}

/// `text` as its runs: upper-case letters, lower-case letters and digits
/// each one run, and each punctuation byte a run of its own; `None` for a
/// text holding any other byte, or a backtick, which ends an inline atom.
fn runs_of(text: &str) -> Option<Vec<(Run, &str)>> {
    let bytes = text.as_bytes();
    let class = |b: u8| match b {
        b'A'..=b'Z' => Some(Run::Upper),
        b'a'..=b'z' => Some(Run::Lower),
        b'0'..=b'9' => Some(Run::Digit),
        b'`' => None,
        b if b.is_ascii_punctuation() => Some(Run::Punct(b)),
        _ => None,
    };
    let mut runs = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let run = class(bytes[i])?;
        let start = i;
        i += 1;
        if !matches!(run, Run::Punct(_)) {
            while i < bytes.len() && class(bytes[i]) == Some(run) {
                i += 1;
            }
        }
        runs.push((run, &text[start..i]));
    }
    Some(runs)
}

/// The byte-pattern a word token's `texts` share, where they share a
/// constant part the word kind does not say: every text splits into the
/// same runs, a run of digits among them, and a run of letters is the same
/// text in every one. That run is written as its text, a punctuation byte
/// escaped, and every other run as its class with the lengths seen, so
/// `KB5031354` and `KB4534310` give `KB[0-9]{7}` and `v2` and `v10` give
/// `v[0-9]{1,2}`.
fn minted(texts: &[&str]) -> Option<String> {
    let runs: Vec<Vec<(Run, &str)>> = texts.iter().map(|t| runs_of(t)).collect::<Option<_>>()?;
    let first = &runs[0];
    let aligned = runs.iter().all(|r| r.len() == first.len() && r.iter().zip(first).all(|(a, b)| a.0 == b.0));
    if !aligned || !first.iter().any(|(run, _)| *run == Run::Digit) {
        return None;
    }
    let mut constant = false;
    let mut out = String::new();
    for (p, &(run, text)) in first.iter().enumerate() {
        let same = runs.iter().all(|r| r[p].1 == text);
        let class = match run {
            Run::Punct(b) => {
                out.push('\\');
                out.push(char::from(b));
                continue;
            }
            Run::Upper | Run::Lower if same => {
                constant = true;
                out.push_str(text);
                continue;
            }
            Run::Upper => "[A-Z]",
            Run::Lower => "[a-z]",
            Run::Digit => "[0-9]",
        };
        out.push_str(class);
        let lo = runs.iter().map(|r| r[p].1.len()).min().expect("a run in every text");
        let hi = runs.iter().map(|r| r[p].1.len()).max().expect("a run in every text");
        if lo != hi {
            out.push_str(&format!("{{{lo},{hi}}}"));
        } else if lo > 1 {
            out.push_str(&format!("{{{lo}}}"));
        }
    }
    constant.then_some(out)
}

/// Each field's byte shape as one word token, read over every line of
/// every shape where it is one word token, a list's values among them;
/// `None` where those values share no constant part or are all one text.
fn field_mints(lines: &[Line], shapes: &[Forming], fields: usize) -> Vec<Option<String>> {
    let mut texts: Vec<Vec<&str>> = vec![Vec::new(); fields];
    for shape in shapes {
        for (e, elem) in shape.skeleton.iter().enumerate() {
            for (r, layout) in &shape.members {
                let line = &lines[*r];
                let runs: Vec<(usize, Range<usize>)> = match elem {
                    Elem::Field(f) => vec![(*f, layout[e].clone())],
                    Elem::List(f, sep) => line.list_runs(&layout[e], sep).into_iter().map(|run| (*f, run)).collect(),
                    Elem::Records(body) => {
                        let pieces = line.pieces(&layout[e], body).expect("a member's records align with the record");
                        pieces
                            .iter()
                            .flat_map(|piece| {
                                body.skeleton.iter().zip(piece).filter_map(|(el, run)| el.field().map(|f| (f, run.clone())))
                            })
                            .collect()
                    }
                    Elem::Col(_) | Elem::Part(..) => continue,
                };
                for (f, run) in runs {
                    if let [t] = &line.toks[run]
                        && t.kind == TokenKind::Word
                    {
                        texts[f].push(&line.text[t.start..t.end]);
                    }
                }
            }
        }
    }
    texts
        .iter()
        .map(|t| if t.iter().any(|x| *x != t[0]) { minted(t) } else { None })
        .collect()
}

/// The value classes of the shipped library, its word lists and named
/// sub-patterns, that read every one of `texts` whole, in the library's
/// order: `log_level` for ERROR and WARN, `http_2xx` for 200 and 204.
fn classes_of(texts: &[&str]) -> Vec<&'static str> {
    use crate::library::Form;
    crate::library::ENTRIES
        .iter()
        .filter(|e| matches!(e.form, Form::Let(_) | Form::Words { .. }))
        .map(|e| e.name)
        .filter(|name| {
            let pattern = crate::parse(&format!("\\{{{name}}}")).expect("a library class parses by its name");
            texts.iter().all(|t| matches!(&crate::scan(&pattern, t.as_bytes())[..], [one] if one.range() == (0..t.len())))
        })
        .collect()
}

/// The shape of the shipped library every value `runs` holds is exactly one
/// token of, where lexing each value's line with it changes no token
/// outside the value, since a pattern naming it lexes the whole line with
/// it: the first in the library's order, `cve` for CVE-2023-1234. `runs`
/// is each value's line and tokens there; `shapes` what the lines are
/// lexed under.
fn library_kind(lines: &[Line], runs: &[(usize, Range<usize>)], shapes: &ShapeSet) -> Option<&'static str> {
    let fits = |name: &str| -> bool {
        let pattern = crate::parse(&format!("\\{{{name}}}")).expect("a library shape parses by its name");
        let id = crate::library::id_of(name).expect("a library shape has an id");
        let set = shapes.with_library_shapes(&[id]);
        runs.iter().all(|(r, run)| {
            let line = &lines[*r];
            let Some(value) = line.run_text(run) else { return false };
            let whole = matches!(&crate::scan(&pattern, value.as_bytes())[..], [one] if one.range() == (0..value.len()));
            let (from, to) = (line.toks[run.start].start, line.toks[run.end - 1].end);
            let outside = |toks: &[Tok]| -> Vec<Tok> { toks.iter().copied().filter(|t| t.end <= from || t.start >= to).collect() };
            whole && outside(&tokens_of(&line.text, &set)) == outside(&line.toks)
        })
    };
    let (r, run) = runs.first()?;
    let sample = lines[*r].run_text(run)?;
    crate::library::ENTRIES.iter().filter(|e| evidenced(e, sample)).map(|e| e.name).find(|name| fits(name))
}

/// Whether a library entry is a shape carrying evidence of its own about
/// `sample`, a value it is tried on: a letter or digit its byte-pattern
/// holds as written, as `CVE-` or `AKIA`, or a check on the bytes it reads
/// that refuses `sample` with one letter or digit changed to another of its
/// class, as a check digit does. A shape of bare character classes, or one
/// whose check reads only their layout, says nothing a field's own
/// spelling does not.
fn evidenced(entry: &crate::library::Entry, sample: &str) -> bool {
    fn literal(p: &crate::bytepat::BytePat) -> bool {
        use crate::bytepat::BytePat as B;
        match p {
            B::Byte(b) => b.is_ascii_alphanumeric(),
            B::Concat(ps) | B::Alt(ps) => ps.iter().any(literal),
            B::Star(p) | B::Plus(p) | B::Opt(p) | B::Repeat(p, ..) => literal(p),
            B::Empty | B::Any | B::Class(..) | B::Property(..) | B::Builtin(_) => false,
        }
    }
    let crate::library::Form::Shape { pat, guard } = entry.form else { return false };
    if literal(&crate::bytepat::parse(pat.as_bytes()).expect("a library shape's pattern parses")) {
        return true;
    }
    if guard.is_none() {
        return false;
    }
    let pattern = crate::parse(&format!("\\{{{}}}", entry.name)).expect("a library shape parses by its name");
    let reads = |text: &str| matches!(&crate::scan(&pattern, text.as_bytes())[..], [one] if one.range() == (0..text.len()));
    let next = |b: u8| match b {
        b'0'..=b'8' | b'a'..=b'y' | b'A'..=b'Y' => b + 1,
        b'9' => b'0',
        b'z' => b'a',
        b'Z' => b'A',
        other => other,
    };
    (0..sample.len()).filter(|&i| sample.as_bytes()[i].is_ascii_alphanumeric()).any(|i| {
        let mut changed = sample.as_bytes().to_vec();
        changed[i] = next(changed[i]);
        !reads(&String::from_utf8_lossy(&changed))
    })
}

/// A field's text as its capture holds it, from the tokens each member
/// holding it gives: its kind, with the range its values share as a
/// candidate; a group of kinds with constant punctuation kept; the class of
/// the kinds; else free text to the next element, or to the line's end
/// where the field is last, which the third value says. Never a literal,
/// since the field is what varies. A field of one word token is `whole`,
/// the atom its byte shape over every shape is spelled as, where it has
/// one; unless [`Mint::Off`], a word token of a longer field is the inline
/// byte atom its values here share, where they are not all one text. A
/// field every value of which the lex reads as several tokens is
/// `library`, the shape of the shipped library that reads each as one,
/// where there is one.
fn field_body(
    present: &[(Vec<TokenKind>, Vec<&str>)],
    library: Option<&str>,
    last: bool,
    shapes: &ShapeSet,
    mint: Mint,
    whole: Option<&str>,
) -> (String, Option<String>, bool) {
    let (first, _) = present.first().expect("a field holds text in a line of its shape");
    if present.iter().all(|(kinds, _)| kinds.len() > 1)
        && let Some(name) = library
    {
        return (format!("\\{{{name}}}"), None, false);
    }
    if present.iter().all(|(k, _)| k == first) {
        let varies = present.iter().any(|(_, t)| *t != present[0].1);
        let mint_at = |p: usize| -> Option<String> {
            if mint == Mint::Off || !varies || first[p] != TokenKind::Word {
                return None;
            }
            let texts: Vec<&str> = present.iter().map(|(_, t)| t[p]).collect();
            minted(&texts).map(|m| format!("`{m}`"))
        };
        if let [kind] = first[..] {
            if kind == TokenKind::Word
                && let Some(atom) = whole
            {
                return (atom.to_string(), None, false);
            }
            let a = atom(kind, shapes);
            let texts: Vec<&str> = present.iter().map(|(_, t)| t[0]).collect();
            let ranged = range_of(kind, &texts).map(|r| format!("{a}{{{r}}}"));
            return (a, ranged, false);
        }
        let parts: Vec<String> = first
            .iter()
            .enumerate()
            .map(|(p, kind)| {
                let text = present[0].1[p];
                let constant = present.iter().all(|(_, t)| t[p] == text);
                let punct = matches!(kind, TokenKind::Punct | TokenKind::Open(_) | TokenKind::Close(_));
                if constant && punct {
                    return crate::templates::quote(text);
                }
                match mint_at(p) {
                    Some(m) => m,
                    None => atom(*kind, shapes),
                }
            })
            .collect();
        return (parts.join(" "), None, false);
    }
    if present.iter().all(|(k, _)| k.len() == 1) {
        let mut atoms: Vec<String> = Vec::new();
        for (k, _) in present {
            let a = atom(k[0], shapes);
            if !atoms.contains(&a) {
                atoms.push(a);
            }
        }
        if atoms.iter().any(|a| a == ".") {
            return (".".to_string(), None, false);
        }
        return (format!("[{}]", atoms.join(" ")), None, false);
    }
    if last {
        return (".*? $ .".to_string(), None, true);
    }
    (aligned(present), None, false)
}

/// Runs of differing kinds aligned as [`super::infer`] aligns examples, a
/// position some runs lack optional and a run of one kind repeated with its
/// bounds; a constant text is kept only where it is punctuation, since the
/// field is what varies.
fn aligned(present: &[(Vec<TokenKind>, Vec<&str>)]) -> String {
    let n = present.len();
    let seqs: Vec<Vec<super::Tok>> = present
        .iter()
        .map(|(kinds, texts)| {
            kinds
                .iter()
                .zip(texts)
                .map(|(kind, text)| super::Tok { kind: *kind, text: (*text).to_string(), start: 0, end: 0 })
                .collect()
        })
        .collect();
    let mut columns: Vec<Column> = seqs[0].iter().map(|t| Column::new(n, 0, t)).collect();
    for (e, seq) in seqs.iter().enumerate().skip(1) {
        columns = super::fold(columns, e, seq, n);
    }
    super::slots_of(&columns)
        .into_iter()
        .map(|mut slot| {
            let punct = matches!(slot.kinds[..], [TokenKind::Punct | TokenKind::Open(_) | TokenKind::Close(_)]);
            if !punct {
                slot.text = None;
            }
            slot.narrowing = None;
            slot.spelled()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The name a shape minted for field `name` is declared under in `set`:
/// the field's own, else the field's with `_shape` after it, whichever
/// shadows nothing; `None` where both do.
fn mint_name(name: &str, set: &ShapeSet) -> Option<String> {
    [name.to_string(), format!("{name}_shape")].into_iter().find(|n| set.is_free(n))
}

/// The byte shapes minted for the fields, as the pattern spells them.
struct Minted {
    /// Each field's atom as one word token, where its values share a
    /// constant part.
    atoms: Vec<Option<String>>,
    /// The `shape` lines the atoms need.
    declarations: Vec<String>,
    /// The shapes the lines are lexed under, those lines declared in it.
    set: ShapeSet,
}

/// Each field's atom for the byte shape its values share as one word
/// token, spelled as `mint` says, with the declarations it needs and the
/// set the lines are read under with them.
fn minted_atoms(spec: &Spec<'_>, lines: &[Line], names: &[String], shapes: &[Forming]) -> Result<Minted, BuildError> {
    let mut set = spec.shapes.clone();
    let mut declarations = Vec::new();
    if spec.mint == Mint::Off {
        return Ok(Minted { atoms: vec![None; names.len()], declarations, set });
    }
    let mut atoms = Vec::with_capacity(names.len());
    for (name, bytes) in names.iter().zip(field_mints(lines, shapes, names.len())) {
        let Some(bytes) = bytes else {
            atoms.push(None);
            continue;
        };
        let declared = match (spec.mint, mint_name(name, &set)) {
            (Mint::Shapes, Some(shape)) => Some(shape),
            _ => None,
        };
        let Some(shape) = declared else {
            atoms.push(Some(format!("`{bytes}`")));
            continue;
        };
        let decl = format!("{shape} = `{bytes}`");
        if let Err(e) = set.declare(&decl, crate::custom::Precedence::Before) {
            return Err(BuildError::Undeclared { decl: format!("shape {decl}"), msg: e.msg });
        }
        declarations.push(format!("shape {decl}"));
        atoms.push(Some(format!("\\{{{shape}}}")));
    }
    Ok(Minted { atoms, declarations, set })
}

/// Spell, order, range and verify the shapes into the built pattern and
/// its report, with each row's values cut into the records it holds.
fn finish(
    spec: &Spec<'_>,
    lines: &[Line],
    names: &[String],
    given: &[Given],
    shapes: &[Forming],
) -> Result<(Built, Parts), BuildError> {
    let Minted { atoms: mints, declarations, set } = minted_atoms(spec, lines, names, shapes)?;
    let spelled: Vec<Vec<Spelled>> =
        shapes.iter().map(|s| spell(lines, s, names, spec.shapes, spec.mint, &mints)).collect();
    let mut order: Vec<usize> = (0..shapes.len()).collect();
    order.sort_by_key(|&s| {
        let free = spelled[s].iter().filter(|e| e.free).count();
        let literal = spelled[s].iter().filter(|e| e.literal).count();
        (free, Reverse(literal), Reverse(shapes[s].members.len()), s)
    });
    let branch = |s: usize, enabled: &[Option<usize>]| -> String {
        let body: Vec<&str> = spelled[s]
            .iter()
            .zip(enabled)
            .map(|(e, on)| match on {
                Some(k) => e.alternatives()[*k],
                None => e.plain.as_str(),
            })
            .collect();
        let body = body.join(" ");
        if spec.unanchored { body } else { format!("^ {body} ~<($ .)") }
    };
    let pattern_of = |enabled: &[Vec<Option<usize>>]| -> String {
        order.iter().map(|&s| branch(s, &enabled[s])).collect::<Vec<_>>().join(joiner(spec.unanchored))
    };
    let parse = |pattern: &str| {
        crate::parser::parse_with_shapes(pattern, &set)
            .map_err(|e| BuildError::Unparsed { pattern: pattern.to_string(), msg: e.msg })
    };
    let missed = |enabled: &[Vec<Option<usize>>]| -> Result<Vec<usize>, BuildError> {
        let parsed = parse(&pattern_of(enabled))?;
        Ok(spec
            .counters
            .iter()
            .enumerate()
            .filter(|(_, c)| !crate::engine::scan_with_shapes(&parsed, c.as_bytes(), &set).is_empty())
            .map(|(i, _)| i)
            .collect())
    };
    let candidates: Vec<(usize, usize, usize)> = spelled
        .iter()
        .enumerate()
        .flat_map(|(s, elems)| {
            elems.iter().enumerate().flat_map(move |(e, el)| (0..el.alternatives().len()).map(move |k| (s, e, k)))
        })
        .collect();
    let mut enabled: Vec<Vec<Option<usize>>> = spelled.iter().map(|s| vec![None; s.len()]).collect();
    let mut hit = missed(&enabled)?;
    while !hit.is_empty() {
        let mut best: Option<(usize, usize)> = None;
        for (c, &(s, e, k)) in candidates.iter().enumerate() {
            if enabled[s][e].is_some() {
                continue;
            }
            enabled[s][e] = Some(k);
            let left = missed(&enabled)?.len();
            enabled[s][e] = None;
            if best.is_none_or(|(_, fewest)| left < fewest) {
                best = Some((c, left));
            }
        }
        let Some((c, left)) = best else { break };
        if left >= hit.len() {
            break;
        }
        let (s, e, k) = candidates[c];
        enabled[s][e] = Some(k);
        hit = missed(&enabled)?;
    }
    let pattern = pattern_of(&enabled);
    if let Some(&counter) = hit.first() {
        return Err(BuildError::Matched { counter, pattern });
    }
    let mut suggestions: Vec<(String, Vec<String>)> = Vec::new();
    for (f, name) in names.iter().enumerate() {
        let mut shared: Option<Vec<&str>> = None;
        let mut printed = false;
        for (s, shape) in shapes.iter().enumerate() {
            for (e, _) in shape.skeleton.iter().enumerate().filter(|(_, el)| **el == Elem::Field(f)) {
                let el = &spelled[s][e];
                printed |= enabled[s][e].is_some_and(|k| k >= usize::from(el.ranged.is_some()));
                let here: Vec<&str> = el.classes.iter().map(|(c, _)| *c).collect();
                shared = Some(match shared {
                    None => here,
                    Some(seen) => seen.into_iter().filter(|c| here.contains(c)).collect(),
                });
            }
        }
        if let Some(classes) = shared
            && !classes.is_empty()
            && !printed
        {
            suggestions.push((name.clone(), classes.into_iter().map(str::to_string).collect()));
        }
    }
    let parsed = parse(&pattern)?;
    let fields = fields_built(names, given, shapes, &parsed)?;
    let mut position = vec![0; shapes.len()];
    for (p, &s) in order.iter().enumerate() {
        position[s] = p;
    }
    let mut rows: Vec<Row> =
        lines.iter().map(|l| Row { text: l.text.clone(), shape: None, values: vec![None; names.len()] }).collect();
    let mut parts: Parts = vec![Vec::new(); lines.len()];
    for (s, shape) in shapes.iter().enumerate() {
        for (row, layout) in &shape.members {
            let line = &lines[*row];
            let (got, cut) = read_back(&set, line, &parsed, &fields)
                .ok_or_else(|| BuildError::Uncovered { line: *row, pattern: pattern.clone() })?;
            parts[*row] = cut;
            for (f, name) in names.iter().enumerate() {
                let want = shape
                    .value_of(line, layout, f)
                    .map(|v| match v {
                        Value::One(one) if fields[f].list => Value::Many(vec![one]),
                        v => v,
                    });
                if want != got[f] {
                    let got = got[f].clone();
                    return Err(BuildError::Wrong { line: *row, field: name.clone(), want, got, pattern });
                }
            }
            rows[*row] = Row { text: line.text.clone(), shape: Some(position[s]), values: got };
        }
    }
    let built_shapes = order
        .iter()
        .map(|&s| {
            let mut members: Vec<usize> = shapes[s].members.iter().map(|(r, _)| *r).collect();
            members.sort_unstable();
            let reach = shapes[s]
                .reach
                .iter()
                .map(|r| match r {
                    Reach::Anchored(from) => Reach::Anchored(position[*from]),
                    other => *other,
                })
                .collect();
            Shape { pattern: branch(s, &enabled[s]), lines: members, reach }
        })
        .collect();
    let built = Built {
        pattern,
        fields,
        shapes: built_shapes,
        rows,
        records: Vec::new(),
        name: "extract".to_string(),
        counters: spec.counters.to_vec(),
        unanchored: spec.unanchored,
        declarations,
        suggestions,
    };
    Ok((built, parts))
}

/// How a `--format` template writes field `name` of a pattern binding
/// `bound`: by its position where a report template reads its name as a
/// field of the report, `[*]` after it for a field holding every binding,
/// and its accessor after a colon.
fn template_for(name: &str, bound: &[String], every: bool, accessor: Option<&str>) -> String {
    let reference = match bound.iter().position(|b| b == name) {
        Some(i) if crate::rewrite::is_report_field(name) => (i + 1).to_string(),
        _ => name.to_string(),
    };
    let every = if every { "[*]" } else { "" };
    match accessor {
        Some(accessor) => format!("${{{reference}{every}:{accessor}}}"),
        None => format!("${{{reference}{every}}}"),
    }
}

/// The fields a `fields` line gives `pattern`, as the builder made them:
/// each mark's name, type, record start and accessor, in order. A field
/// bound inside a repetition that also binds a field beginning a record
/// is inside records a line repeats; any other bound under a repetition is
/// a list. Each field's parent is the one its name, up to its last dot,
/// names.
#[must_use]
pub fn fields_from_marks(marks: &[crate::infer::marks::FieldMark], pattern: &crate::ast::Pattern) -> Vec<Field> {
    let bound = pattern.capture_names();
    let lists = pattern.list_registers();
    let starters: Vec<&str> = marks.iter().filter(|m| m.starts_record).map(|m| m.name.as_str()).collect();
    let records: Vec<String> = pattern
        .repetition_registers()
        .into_iter()
        .filter(|names| names.iter().any(|n| starters.contains(&n.as_str())))
        .flatten()
        .collect();
    marks
        .iter()
        .map(|m| {
            let repeats = records.contains(&m.name);
            let list = !repeats && lists.contains(&m.name);
            Field {
                name: m.name.clone(),
                accessor: m.accessor.clone(),
                hint: m.hint,
                type_name: m.type_name.clone(),
                starts_record: m.starts_record,
                list,
                repeats,
                parent: m.name.rsplit_once('.').and_then(|(outer, _)| marks.iter().position(|o| o.name == outer)),
                template: template_for(&m.name, &bound, list || repeats, m.accessor.as_deref()),
            }
        })
        .collect()
}

/// The fields a pattern reads where it is applied again: where `source`,
/// the pattern as written and parsed under `shapes` into `pattern`, is one
/// reference `\{name}` to a sub-pattern a `fields` line gives its fields,
/// those, as [`fields_from_marks`] reads them; otherwise one field per
/// register, in written order, a list where it is bound under a
/// repetition.
#[must_use]
pub fn fields_for(source: &str, pattern: &crate::ast::Pattern, shapes: &ShapeSet) -> Vec<Field> {
    let saved = source
        .trim()
        .strip_prefix("\\{")
        .and_then(|s| s.strip_suffix('}'))
        .and_then(|name| Some((shapes.fields_of(name)?, shapes.let_of(name)?)));
    if let Some((marks, named)) = saved {
        return fields_from_marks(marks, named);
    }
    let lists = pattern.list_registers();
    pattern
        .capture_names()
        .into_iter()
        .map(|name| Field {
            list: lists.contains(&name),
            template: format!("${{{name}}}"),
            name,
            accessor: None,
            hint: None,
            type_name: None,
            starts_record: false,
            repeats: false,
            parent: None,
        })
        .collect()
}

/// The fields as the report gives them: the accessor a part field is read
/// with, which every shape reading it as a part shares, and the template
/// writing each, by position where a report template reads its name as a
/// field of the report.
fn fields_built(
    names: &[String],
    given: &[Given],
    shapes: &[Forming],
    parsed: &crate::ast::Pattern,
) -> Result<Vec<Field>, BuildError> {
    let bound = parsed.capture_names();
    let mut fields = Vec::with_capacity(names.len());
    for (f, name) in names.iter().enumerate() {
        let mut accessors: Option<Vec<String>> = None;
        let mut whole: Option<usize> = None;
        let mut first_part: Option<usize> = None;
        let list = shapes.iter().any(|s| s.skeleton.iter().any(|el| matches!(el, Elem::List(g, _) if *g == f)));
        let repeats = shapes
            .iter()
            .any(|s| s.skeleton.iter().any(|el| matches!(el, Elem::Records(body) if body.fields().contains(&f))));
        for shape in shapes {
            match shape.skeleton.iter().find(|el| el.field() == Some(f)) {
                Some(Elem::Part(..)) => {
                    let mine = &shape.parts[&f];
                    first_part.get_or_insert(shape.members[0].0);
                    accessors = Some(match accessors {
                        None => mine.clone(),
                        Some(seen) => seen.into_iter().filter(|a| mine.contains(a)).collect(),
                    });
                }
                Some(_) => {
                    whole.get_or_insert(shape.members[0].0);
                }
                None => {}
            }
        }
        let accessor = match (accessors, whole, first_part) {
            (Some(_), Some(line), _) => return Err(BuildError::Disagree { field: name.clone(), line }),
            (Some(accessors), None, Some(line)) => match accessors.into_iter().next() {
                Some(accessor) => Some(accessor),
                None => return Err(BuildError::Disagree { field: name.clone(), line }),
            },
            _ => None,
        };
        let template = template_for(name, &bound, list || repeats, accessor.as_deref());
        let said = &given[f];
        fields.push(Field {
            name: name.clone(),
            accessor,
            hint: said.hint,
            type_name: said.type_name.clone(),
            starts_record: said.starts_record,
            list,
            repeats,
            parent: said.parent,
            template,
        });
    }
    Ok(fields)
}

/// Each field's value as the pattern reads it from `line` lexed under
/// `shapes`, matched whole, and the values cut into the records the line
/// holds; `None` where no match holds the whole line.
fn read_back(
    shapes: &ShapeSet,
    line: &Line,
    parsed: &crate::ast::Pattern,
    fields: &[Field],
) -> Option<(Values, Vec<Values>)> {
    let bytes = line.text.as_bytes();
    let whole = line.toks[0].start..line.toks[line.toks.len() - 1].end;
    let spans = crate::engine::scan_with_shapes(parsed, bytes, shapes);
    let span = spans.iter().find(|s| s.range() == whole)?;
    let one = std::slice::from_ref(span);
    let matches = if shapes.is_empty() {
        crate::engine::captures_with_lists(parsed, bytes, one)
    } else {
        crate::engine::captures_with_shapes_and_lists(parsed, bytes, shapes, one)
    };
    let m = matches.first().expect("a span the scan returned has its captures");
    let values = fields
        .iter()
        .map(|field| field.value_in(m, &line.text).expect("an accessor the builder named reads its own part"))
        .collect();
    let parts = record_parts(fields, m, &line.text).expect("an accessor the builder named reads its own part");
    Some((values, parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cumulative update titles as Windows 7 through 11 and Server 2022
    /// write them.
    const TITLES: [&str; 7] = [
        "2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB5031354)",
        "2023-10 Cumulative Update for Windows 10 Version 22H2 for x64-based Systems (KB5031356)",
        "2023-09 Cumulative Update Preview for Windows 11 Version 22H2 for x64-based Systems (KB5030310)",
        "2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems (KB4534310)",
        "2023-01 Security Monthly Quality Rollup for Windows 8.1 for x64-based Systems (KB5022352)",
        "Cumulative Update for Windows 10 Version 1607 for x64-based Systems (KB4103720)",
        "2023-10 Cumulative Update for Microsoft server operating system version 21H2 for x64-based Systems (KB5031364)",
    ];

    const MARKED_TITLE: &str = "{month:2023-10} Cumulative Update for Windows {os:11} Version {version:22H2} \
                                for x64-based Systems ({kb:KB5031354})";

    fn strings(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| (*t).to_string()).collect()
    }

    fn build_from(
        marked: &[&str],
        lines: &[&str],
        hints: &[(&str, &[&str])],
        counters: &[&str],
    ) -> Result<Built, BuildError> {
        build_minted(marked, lines, hints, counters, Mint::default())
    }

    fn build_minted(
        marked: &[&str],
        lines: &[&str],
        hints: &[(&str, &[&str])],
        counters: &[&str],
        mint: Mint,
    ) -> Result<Built, BuildError> {
        let marked: Vec<Marked> =
            marked.iter().map(|m| crate::infer::marks::parse(m).expect("the example's marks read")).collect();
        let lines = strings(lines);
        let hints: Vec<(String, Vec<String>)> = hints.iter().map(|(n, v)| ((*n).to_string(), strings(v))).collect();
        let counters = strings(counters);
        let shapes = ShapeSet::new();
        let spec = Spec {
            lines: &lines,
            marked: &marked,
            hints: &hints,
            counters: &counters,
            shapes: &shapes,
            unanchored: false,
            mint,
        };
        build(&spec)
    }

    fn column(built: &Built, field: &str) -> Vec<Option<String>> {
        let f = built.fields.iter().position(|x| x.name == field).expect("the field is built");
        built.rows.iter().map(|r| r.values[f].as_ref().map(Value::joined)).collect()
    }

    fn some(values: &[Option<&str>]) -> Vec<Option<String>> {
        values.iter().map(|v| v.map(str::to_string)).collect()
    }

    #[test]
    fn one_hinted_value_reaches_every_shape_whose_literals_surround_it() {
        let built = build_from(&[], &TITLES, &[("kb", &["KB5031354"])], &[]).expect("a pattern is built");
        let kbs: Vec<Option<String>> = TITLES
            .iter()
            .map(|t| {
                let open = t.rfind('(').expect("a title ends in its KB");
                Some(t[open + 1..t.len() - 1].to_string())
            })
            .collect();
        assert_eq!(column(&built, "kb"), kbs, "{}", built.pattern);
        assert!(built.shapes.iter().all(|s| s.reach[0] != Reach::Missing), "{built:#?}");
    }

    #[test]
    fn marked_fields_reach_the_shapes_that_hold_them_and_no_other() {
        let built = build_from(&[MARKED_TITLE], &TITLES, &[], &[]).expect("a pattern is built");
        let month =
            [Some("2023-10"), Some("2023-10"), Some("2023-09"), Some("2020-01"), Some("2023-01"), None, Some("2023-10")];
        let os = [Some("11"), Some("10"), Some("11"), Some("7"), Some("8.1"), Some("10"), None];
        let version = [Some("22H2"), Some("22H2"), Some("22H2"), None, None, Some("1607"), Some("21H2")];
        assert_eq!(column(&built, "month"), some(&month), "{}", built.pattern);
        assert_eq!(column(&built, "os"), some(&os), "{}", built.pattern);
        assert_eq!(column(&built, "version"), some(&version), "{}", built.pattern);
        assert_eq!(column(&built, "kb")[6].as_deref(), Some("KB5031364"));
        assert!(built.pattern.contains("(\\N \"-\" \\N)?:month"), "{}", built.pattern);
        assert!(built.pattern.contains("(\\N \\W?):version"), "{}", built.pattern);
        // The Preview title differs from the plain one by one word, so the two
        // are one branch with the word optional.
        assert!(built.pattern.contains("\"Update\" \"Preview\"? \"for\""), "{}", built.pattern);
        assert_eq!(built.shapes.len(), 3, "{}", built.pattern);
    }

    #[test]
    fn a_field_is_never_spelled_as_its_text() {
        let built = build_from(&["{[System.Int32]n:42} apples"], &["42 apples"], &[], &[]).expect("a pattern is built");
        assert_eq!(built.fields[0].type_name.as_deref(), Some("System.Int32"));
        assert!(built.pattern.contains("(\\N):n"), "{}", built.pattern);
        assert!(!built.pattern.contains("\"42\""), "{}", built.pattern);
    }

    #[test]
    fn a_word_whose_values_share_a_constant_part_is_its_byte_shape() {
        let built = build_from(&[MARKED_TITLE], &TITLES, &[], &[]).expect("a pattern is built");
        assert!(built.pattern.contains("(`KB[0-9]{7}`):kb"), "{}", built.pattern);
        assert!(!built.pattern.contains("(\\W):kb"), "a branch with one kb reads the field's shape: {}", built.pattern);
        assert!(built.pattern.contains("(\\N \"-\" \\N)?:month"), "digit counts alone mint nothing: {}", built.pattern);
        let plain = build_minted(&[MARKED_TITLE], &TITLES, &[], &[], Mint::Off).expect("a pattern is built");
        assert!(plain.pattern.contains("(\\W):kb"), "{}", plain.pattern);
        let shapes = ShapeSet::new();
        let hits = |pattern: &str, text: &str| {
            let parsed = crate::parser::parse_with_shapes(pattern, &shapes).expect("the pattern parses");
            crate::engine::scan_with_shapes(&parsed, text.as_bytes(), &shapes).len()
        };
        for near in [
            "2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (KB50313541)",
            "2023-10 Cumulative Update for Windows 11 Version 22H2 for x64-based Systems (XB5031354)",
        ] {
            assert_eq!(hits(&built.pattern, near), 0, "{near}");
            assert_eq!(hits(&plain.pattern, near), 1, "{near}");
        }
        assert_eq!(minted(&["KB5031354", "KB4534310"]).as_deref(), Some("KB[0-9]{7}"));
        assert_eq!(minted(&["v2", "v10"]).as_deref(), Some("v[0-9]{1,2}"));
        assert_eq!(minted(&["INC0012345", "INC0099_99"]), None, "the runs differ");
        assert_eq!(minted(&["web01", "db02"]), None, "no letters shared");
        assert_eq!(minted(&["Monday", "Mars"]), None, "no digits");
        assert_eq!(minted(&["a_1", "a_2"]).as_deref(), Some("a\\_[0-9]"));
    }

    #[test]
    fn under_mint_shapes_a_word_field_reads_a_shape_its_file_declares() {
        let built = build_minted(&[MARKED_TITLE], &TITLES, &[], &[], Mint::Shapes).expect("a pattern is built");
        assert_eq!(built.declarations, ["shape kb = `KB[0-9]{7}`"]);
        assert!(built.pattern.contains("(\\{kb}):kb"), "{}", built.pattern);
        assert!(!built.pattern.contains("`KB"), "{}", built.pattern);
        let mut set = ShapeSet::new();
        set.declare_text(&built.file()).expect("the file declares");
        assert_eq!(set.run_tests().len(), 0, "{}", built.file());
        let inline = build_from(&[MARKED_TITLE], &TITLES, &[], &[]).expect("a pattern is built");
        assert!(inline.declarations.is_empty());
        assert_eq!(column(&built, "kb"), column(&inline, "kb"));
        // A field named as a built-in atom mints under its name with `_shape`.
        let word = build_minted(&["user {word:ab12} up"], &["user ab34 up"], &[], &[], Mint::Shapes)
            .expect("a pattern is built");
        assert_eq!(word.declarations, ["shape word_shape = `ab[0-9]{2}`"]);
        assert!(word.pattern.contains("(\\{word_shape}):word"), "{}", word.pattern);
    }

    #[test]
    fn a_field_is_carried_only_beside_the_same_occurrence_of_its_separator() {
        // Lines cut short mid-write, from leaf_pairs_tip.log: the comma
        // before `ts` is the fourth in the marked line, so a line holding
        // three or one reads no `ts`, and its other fields are between the
        // commas that surround them in the marked line.
        let built = build_from(
            &["TRACE,{worker:flynnel-worker-22},{a:5},{b:0},{ts:985181655377765}"],
            &["TRACE,flynnel-worker-22,3,0,985181656289399", "TRACE,flynnel-worker-5,3,0", "TRACE,flynnel-worker-11", "TRACE,"],
            &[],
            &[],
        )
        .expect("a pattern is built");
        let ts = [Some("985181655377765"), Some("985181656289399"), None, None, None];
        assert_eq!(column(&built, "ts"), some(&ts), "{}", built.pattern);
        let b = [Some("0"), Some("0"), Some("0"), None, None];
        assert_eq!(column(&built, "b"), some(&b), "{}", built.pattern);
        let a = [Some("5"), Some("3"), Some("3"), None, None];
        assert_eq!(column(&built, "a"), some(&a), "{}", built.pattern);
        let worker = [
            Some("flynnel-worker-22"),
            Some("flynnel-worker-22"),
            Some("flynnel-worker-5"),
            Some("flynnel-worker-11"),
            None,
        ];
        assert_eq!(column(&built, "worker"), some(&worker), "{}", built.pattern);
    }

    #[test]
    fn a_line_sharing_only_punctuation_or_an_edge_with_a_marked_shape_takes_none_of_its_fields() {
        // Preamble lines of oversub_trace_13be54b.log, which share only a
        // comma or the line's end with the marked shapes.
        let built = build_from(
            &["TRACE,{worker:flynnel-worker-4},{a:5},{b:0},{ts:3096921515133320}", "CALL {idx:0} {ns:92405600}"],
            &[
                "TRACE,flynnel-worker-7,3,0,3096921515140000",
                "CALL 1 92405700",
                "logical processors 24, pool workers 24, spinners 18, rounds 4, calls 200 per arm",
                "TRACE_CLOCK_PER_NS 4.699900",
            ],
            &[],
            &[],
        )
        .expect("a pattern is built");
        for f in ["worker", "a", "b", "ts", "idx", "ns"] {
            let read = column(&built, f);
            assert_eq!(read[4..], [None, None], "{f}: {}", built.pattern);
        }
        assert_eq!(column(&built, "ns")[3].as_deref(), Some("92405700"));
    }

    #[test]
    fn a_mark_inside_a_mark_is_a_field_read_through_the_outer_one() {
        let built =
            build_from(&["{Line:{[int]n:1} of {[int]m:3}}"], &["5 of 9", "6 of 9"], &[], &[]).expect("a pattern is built");
        assert_eq!(built.pattern, "^ ((\\N):n \"of\" (\\N):m):Line ~<($ .)");
        let names: Vec<&str> = built.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Line", "Line.n", "Line.m"]);
        let keys: Vec<&str> = built.fields.iter().map(Field::key).collect();
        assert_eq!(keys, ["Line", "n", "m"]);
        assert_eq!((built.inside(None), built.inside(Some(0))), (vec![0], vec![1, 2]));
        assert_eq!(built.fields[1].type_name.as_deref(), Some("int"));
        assert_eq!(built.format(), "${Line}\\t${Line.n}\\t${Line.m}");
        assert_eq!(column(&built, "Line"), some(&[Some("1 of 3"), Some("5 of 9"), Some("6 of 9")]));
        assert_eq!(column(&built, "Line.m"), some(&[Some("3"), Some("9"), Some("9")]));
        // Two deep, the middle field spanning the innermost.
        let deep = build_from(&["id {a:x {b:1 {c:2}}}"], &["id y 3 4"], &[], &[]).expect("a pattern is built");
        assert_eq!(column(&deep, "a"), some(&[Some("x 1 2"), Some("y 3 4")]), "{}", deep.pattern);
        assert_eq!(column(&deep, "a.b"), some(&[Some("1 2"), Some("3 4")]), "{}", deep.pattern);
        assert_eq!(column(&deep, "a.b.c"), some(&[Some("2"), Some("4")]), "{}", deep.pattern);
        // A line joining the shape reads the fields inside as it reads the
        // outer one; a line carried by the shape's literals holds no field
        // whose outer field it does not place.
        let joined = build_from(&["{Line:{n:1} of {m:3}}"], &["page 5 of 9 end"], &[], &[]).expect("a pattern is built");
        assert_eq!(column(&joined, "Line")[1].as_deref(), Some("page 5 of 9 end"), "{}", joined.pattern);
        let carried = build_from(&["total {Line:{n:1} of {m:3}} pages"], &["total 5 of 9 pages", "5 of 9"], &[], &[])
            .expect("a pattern is built");
        assert_eq!(column(&carried, "Line.n")[1].as_deref(), Some("5"), "{}", carried.pattern);
        assert_eq!(column(&carried, "Line.n")[2], None, "{}", carried.pattern);
        assert_eq!(column(&carried, "Line")[2], None, "{}", carried.pattern);
    }

    #[test]
    fn a_line_repeating_a_starred_record_reads_a_record_per_repeat() {
        let template = "{Name*:Phoebe Cat}, {[int]age:6}; {Name*:Lucky Shot}, {[int]age:12}";
        let built = build_from(&[template], &["Wise Owl, 87; Big Bird, 5", "Elmo Red, 3"], &[], &[])
            .expect("a pattern is built");
        assert_eq!(built.pattern, "^ (\\W \\W):Name \",\" (\\N):age (\";\" (\\W \\W):Name \",\" (\\N):age)* ~<($ .)");
        assert!(built.fields.iter().all(|f| f.repeats && !f.list), "{:?}", built.fields);
        assert_eq!(built.format(), "${Name[*]}\\t${age[*]}");
        let many = |vs: &[&str]| Some(Value::Many(vs.iter().map(|v| (*v).to_string()).collect()));
        assert_eq!(built.rows[1].values[0], many(&["Wise Owl", "Big Bird"]));
        let names: Vec<Option<String>> = built.records.iter().map(|r| r.values[0].as_ref().map(Value::joined)).collect();
        assert_eq!(names, some(&[Some("Phoebe Cat"), Some("Lucky Shot"), Some("Wise Owl"), Some("Big Bird"), Some("Elmo Red")]));
        assert_eq!(built.records[3].values[1], Some(Value::One("5".to_string())));
        // A field outside the records describes the line, so each record
        // of the line carries it.
        let dated = build_from(
            &["day {day:Mon}: {Name*:Phoebe Cat} ({[int]age:6}); {Name*:Lucky Shot} ({[int]age:12}) end"],
            &["day Wed: Elmo Red (3); Oscar Grouch (9); Big Bird (5) end"],
            &[],
            &[],
        )
        .expect("a pattern is built");
        let days: Vec<Option<String>> = dated.records.iter().map(|r| r.values[0].as_ref().map(Value::joined)).collect();
        assert_eq!(days, some(&[Some("Mon"), Some("Mon"), Some("Wed"), Some("Wed"), Some("Wed")]), "{}", dated.pattern);
        // Records that differ, and records with no word between them, are
        // refused.
        let uneven = build_from(&["{Name*:Phoebe Cat}, {age:6}; {Name*:Lucky Shot}"], &[], &[], &[]);
        assert_eq!(uneven, Err(BuildError::Records { field: "Name".into(), line: 0 }));
        let joined = build_from(&["{a*:x} {b:1} {a*:y} {b:2}"], &[], &[], &[]);
        assert_eq!(joined, Err(BuildError::Records { field: "a".into(), line: 0 }));
    }

    #[test]
    fn a_field_a_library_shape_reads_as_one_token_is_that_shape() {
        let built = build_from(&["patched {cve:CVE-2023-1234} in {pkg:openssl}"], &["patched CVE-2024-56789 in zlib"], &[], &[])
            .expect("a pattern is built");
        assert_eq!(built.pattern, "^ \"patched\" (\\{cve}):cve \"in\" (\\W):pkg ~<($ .)");
        assert_eq!(column(&built, "cve"), some(&[Some("CVE-2023-1234"), Some("CVE-2024-56789")]));
        let spaced = build_from(&["acct {iban:DE89 3704 0044 0532 0130 00} ok"], &["acct GB82 WEST 1234 5698 7654 32 ok"], &[], &[])
            .expect("a pattern is built");
        assert!(spaced.pattern.contains("(\\{iban}):iban"), "{}", spaced.pattern);
        // A value the lex already reads as one token keeps its kind,
        // whatever check its digits happen to pass.
        let plain = build_from(&["ts {ts:985181655377765} x"], &["ts 985181656288835 x"], &[], &[]).expect("a pattern is built");
        assert!(plain.pattern.contains("(\\N):ts"), "{}", plain.pattern);
        // A shape that would lex other words of the line differently spells
        // nothing: the builder's lines would no longer read as they did.
        let dated = build_from(&["{month:2023-10} for x64-based systems"], &["2020-01 for x64-based systems"], &[], &[])
            .expect("a pattern is built");
        assert!(dated.pattern.contains("(\\N \"-\" \\N):month"), "{}", dated.pattern);
        // Nor does a shape of bare character classes, which k8s_name is.
        let bare = build_from(&["{month:2023-10} ok"], &["2020-01 ok"], &[], &[]).expect("a pattern is built");
        assert!(bare.pattern.contains("(\\N \"-\" \\N):month"), "{}", bare.pattern);
    }

    #[test]
    fn a_library_value_class_is_suggested_and_printed_only_where_a_counter_example_needs_it() {
        let plain = build_from(&["{level:ERROR} disk {n:5}"], &["WARN disk 7"], &[], &[]).expect("a pattern is built");
        assert!(plain.pattern.contains("(\\W):level"), "{}", plain.pattern);
        assert_eq!(plain.suggestions, [("level".to_string(), vec!["log_level".to_string()])]);
        let refused =
            build_from(&["{level:ERROR} disk {n:5}"], &["WARN disk 7"], &[], &["HELLO disk 9"]).expect("a pattern is built");
        assert!(refused.pattern.contains("(\\{log_level}):level"), "{}", refused.pattern);
        assert!(refused.suggestions.is_empty(), "{:?}", refused.suggestions);
        // The range still comes first where one excludes the counter-example.
        let status = build_from(&["GET /a {status:200}"], &["GET /b 204"], &[], &["GET /c 500"]).expect("a pattern is built");
        assert!(status.pattern.contains("\\N{200..299}"), "{}", status.pattern);
        assert_eq!(classes_of(&["200", "204"]), ["http_status", "http_2xx"]);
    }

    #[test]
    fn a_column_repeating_a_field_whose_values_vary_is_its_back_reference() {
        let built = build_from(&["<{tag:b}>bold</b>"], &["<i>it</i>", "<em>x</em>"], &[], &[]).expect("a pattern is built");
        assert_eq!(built.pattern, "^ \"<\" (\\W):tag \">\" \\W \"<\" \"/\" =tag \">\" ~<($ .)");
        let hits = |text: &str| {
            let parsed = crate::parse(&built.pattern).expect("the pattern parses");
            crate::scan(&parsed, text.as_bytes()).len()
        };
        assert_eq!((hits("<u>under</u>"), hits("<b>bold</i>")), (1, 0));
        // An equality seen only while the field kept one value stays the kind.
        let same = build_from(&["x {n:1} y 1"], &["x 1 y 1"], &[], &[]).expect("a pattern is built");
        assert!(!same.pattern.contains("=n"), "{}", same.pattern);
    }

    #[test]
    fn a_partly_marked_line_carries_the_fields_it_leaves_out_from_a_fuller_shape() {
        let rollup = "2020-01 Security Monthly Quality Rollup for Windows 7 for x64-based Systems ({kb:KB4534310})";
        let built = build_from(&[MARKED_TITLE, rollup], &[], &[], &[]).expect("a pattern is built");
        assert_eq!(column(&built, "month")[1].as_deref(), Some("2020-01"), "{}", built.pattern);
        assert_eq!(column(&built, "os")[1].as_deref(), Some("7"), "{}", built.pattern);
        assert_eq!(column(&built, "kb")[1].as_deref(), Some("KB4534310"));
        assert_eq!(column(&built, "version")[1], None);
        let rollup_shape = built.shapes.iter().find(|s| s.lines == [1]).expect("the rollup is a shape");
        assert_eq!(rollup_shape.reach[3], Reach::Marked);
        assert!(matches!(rollup_shape.reach[0], Reach::Anchored(_)), "{:?}", rollup_shape.reach);
    }

    #[test]
    fn a_mark_on_part_of_a_token_reads_it_through_an_accessor() {
        let built = build_from(
            &["GET https://{host:example.com}/index.html 200"],
            &["GET https://trex.dev/a/b 404"],
            &[],
            &[],
        )
        .expect("a pattern is built");
        assert_eq!(built.fields[0].template, "${host:host}");
        assert_eq!(column(&built, "host"), some(&[Some("example.com"), Some("trex.dev")]));
    }

    #[test]
    fn a_last_field_of_varying_tokens_is_free_text_to_the_line_end() {
        let built = build_from(
            &["{when:12:00:01} ERROR {msg:disk is full}"],
            &["12:00:02 ERROR retry in 5 seconds", "12:00:03 ERROR out of memory"],
            &[],
            &[],
        )
        .expect("a pattern is built");
        assert!(built.pattern.contains("(.*? $ .):msg"), "{}", built.pattern);
        let msg = [Some("disk is full"), Some("retry in 5 seconds"), Some("out of memory")];
        assert_eq!(column(&built, "msg"), some(&msg));
    }

    #[test]
    fn a_counter_example_adds_the_range_that_excludes_it_and_none_other() {
        let lines = ["GET /b 204"];
        let plain = build_from(&["GET /a {status:200}"], &lines, &[], &[]).expect("a pattern is built");
        assert!(!plain.pattern.contains(".."), "{}", plain.pattern);
        let ranged = build_from(&["GET /a {status:200}"], &lines, &[], &["GET /c 500"]).expect("a pattern is built");
        assert!(ranged.pattern.contains("\\N{200..299}"), "{}", ranged.pattern);
    }

    #[test]
    fn a_value_hinted_twice_in_one_line_or_in_none_is_refused() {
        let twice = build_from(&[], &["a 1 b 1"], &[("n", &["1"])], &[]);
        assert_eq!(twice, Err(BuildError::Twice { field: "n".into(), value: "1".into(), line: 0 }));
        let absent = build_from(&[], &["a 1 b"], &[("n", &["7"])], &[]);
        assert_eq!(absent, Err(BuildError::Absent { field: "n".into(), value: "7".into() }));
    }

    #[test]
    fn the_pattern_file_declares_every_shape_and_its_test_passes() {
        let counter = "Windows Malicious Software Removal Tool x64 - v5.118 (KB890830)";
        let built = build_from(&[MARKED_TITLE], &TITLES, &[], &[counter]).expect("a pattern is built");
        assert_eq!(built.format(), "${month}\\t${os}\\t${version}\\t${kb}");
        let mut set = ShapeSet::new();
        set.declare_text(&built.file()).expect("the file declares");
        assert_eq!(set.run_tests().len(), 0, "{}", built.file());
        let whole = crate::parser::parse_with_shapes("\\{extract}", &set).expect("the pattern named parses");
        let bound = whole.capture_names();
        assert!(built.fields.iter().all(|f| bound.contains(&f.name)), "{bound:?}");
    }

    #[test]
    fn a_starred_field_begins_a_record_that_the_lines_after_it_join() {
        let template = "Name: {Name*:Phoebe Cat}\r\nPhone: {phone:425-123-6789}\n\n\
                        Name: {Name*:Lucky Shot}\nPhone: {phone:206-987-4321}\n";
        let marked = crate::infer::marks::parse_lines(template).expect("the template reads");
        assert_eq!(marked.len(), 4);
        let lines = strings(&[
            "Phone: 111-222-3333",
            "Name: Phoebe Cat",
            "Phone: 425-123-6789",
            "Phone: 425-000-1111",
            "Name: Elephant Wise",
            "Name: Lucky Shot",
            "Phone: 206-987-4321",
        ]);
        let shapes = ShapeSet::new();
        let spec = Spec {
            lines: &lines,
            marked: &marked,
            hints: &[],
            counters: &[],
            shapes: &shapes,
            unanchored: false,
            mint: Mint::default(),
        };
        let built = build(&spec).expect("a pattern is built");
        assert!(built.fields[0].starts_record && !built.fields[1].starts_record);
        let records: Vec<Vec<Option<String>>> = built
            .records
            .iter()
            .map(|r| r.values.iter().map(|v| v.as_ref().map(Value::joined)).collect())
            .collect();
        let want = |name: &str, phone: Option<&str>| vec![Some(name.to_string()), phone.map(str::to_string)];
        assert_eq!(
            records,
            [
                want("Phoebe Cat", Some("425-123-6789")),
                want("Lucky Shot", Some("206-987-4321")),
                want("Phoebe Cat", Some("425-123-6789")),
                want("Elephant Wise", None),
                want("Lucky Shot", Some("206-987-4321")),
            ]
        );
        assert_eq!(built.records[2].lines.len(), 3, "{:?}", built.records[2]);
    }

    #[test]
    fn a_field_marked_twice_in_a_line_is_a_list_of_its_values() {
        let built = build_from(
            &["from {ip:10.0.0.1} -> {ip:10.0.0.2} ok"],
            &["from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok", "from 10.0.0.3 ok"],
            &[],
            &[],
        )
        .expect("a pattern is built");
        assert!(built.fields[0].list);
        assert_eq!(built.fields[0].template, "${ip[*]}");
        let many = |vs: &[&str]| Some(Value::Many(vs.iter().map(|v| (*v).to_string()).collect()));
        let values: Vec<Option<Value>> = built.rows.iter().map(|r| r.values[0].clone()).collect();
        assert_eq!(
            values,
            [many(&["10.0.0.1", "10.0.0.2"]), many(&["10.0.0.7", "10.0.0.8", "10.0.0.9"]), many(&["10.0.0.3"])],
            "{}",
            built.pattern
        );
        let refused = build_from(&["{n:1} and {n:2} or {n:3}"], &[], &[], &[]);
        assert_eq!(refused, Err(BuildError::Repeated { field: "n".into(), line: 0 }));
    }

    #[test]
    fn a_starred_mark_spanning_lines_reads_a_nested_record_of_the_lines_it_spans() {
        let template = "{Person*:Name: {Name:Phoebe Cat}\r\nPhone: {Phone:425-123-6789}}\n";
        let marked = crate::infer::marks::parse_lines(template).expect("the template reads");
        let lines = strings(&["Name: Wise Owl", "Phone: 425-888-7766", "Name: Big Bird", "Phone: 206-555-0100"]);
        let shapes = ShapeSet::new();
        let spec = Spec {
            lines: &lines,
            marked: &marked,
            hints: &[],
            counters: &[],
            shapes: &shapes,
            unanchored: false,
            mint: Mint::default(),
        };
        let built = build(&spec).expect("a pattern is built");
        let names: Vec<&str> = built.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Person", "Person.Name", "Person.Phone"]);
        let starts: Vec<bool> = built.fields.iter().map(|f| f.starts_record).collect();
        assert_eq!(starts, [false, true, false]);
        let records: Vec<(Vec<usize>, Vec<Option<String>>)> = built
            .records
            .iter()
            .map(|r| (r.lines.clone(), r.values.iter().map(|v| v.as_ref().map(Value::joined)).collect()))
            .collect();
        let want = |lines: [usize; 2], name: &str, phone: &str| {
            (lines.to_vec(), some(&[Some(format!("Name: {name}\nPhone: {phone}").as_str()), Some(name), Some(phone)]))
        };
        assert_eq!(
            records,
            [
                want([0, 1], "Phoebe Cat", "425-123-6789"),
                want([2, 3], "Wise Owl", "425-888-7766"),
                want([4, 5], "Big Bird", "206-555-0100"),
            ]
        );
    }

    #[test]
    fn a_field_named_as_a_report_field_is_written_by_position() {
        let built = build_from(&["at {line:42} in {path:src/main.rs}"], &["at 7 in lib.rs"], &[], &[])
            .expect("a pattern is built");
        assert_eq!(built.fields[0].template, "${1}");
        assert_eq!(built.fields[1].template, "${2}");
        assert_eq!(column(&built, "path"), some(&[Some("src/main.rs"), Some("lib.rs")]));
    }

    /// A pattern built from a template of several lines and `lines`.
    fn build_template(template: &str, lines: &[&str]) -> Built {
        let marked = crate::infer::marks::parse_lines(template).expect("the template reads");
        let lines = strings(lines);
        let shapes = ShapeSet::new();
        let spec = Spec {
            lines: &lines,
            marked: &marked,
            hints: &[],
            counters: &[],
            shapes: &shapes,
            unanchored: false,
            mint: Mint::default(),
        };
        build(&spec).expect("a pattern is built")
    }

    /// The pattern file `built` writes, read back: the set it declares, and
    /// the fields its `fields` line gives `extract`.
    fn read_file(built: &Built) -> (ShapeSet, Vec<Field>) {
        let mut set = ShapeSet::new();
        set.declare_text(&built.file()).expect("the file reads");
        let marks = set.fields_of("extract").expect("the file gives extract its fields").to_vec();
        let fields = fields_from_marks(&marks, set.let_of("extract").expect("the file declares extract"));
        (set, fields)
    }

    #[test]
    fn a_built_patterns_file_gives_back_every_field_as_the_build_made_it() {
        let builds = [
            build_from(&["{month:2023-10} Cumulative Update for Windows {[int]os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})"], &TITLES, &[], &[]).expect("titles"),
            build_minted(&["{month:2023-10} Cumulative Update for Windows {os:11} Version {version:22H2} for x64-based Systems ({kb:KB5031354})"], &TITLES, &[], &[], Mint::Shapes).expect("declared shapes"),
            build_from(&["from {ip:10.0.0.1} -> {ip:10.0.0.2} ok"], &["from 10.0.0.7 -> 10.0.0.8 -> 10.0.0.9 ok", "from 10.0.0.3 ok"], &[], &[]).expect("a list"),
            build_from(&["day {day:Mon}: {Name*:Phoebe Cat} ({[int]age:6}); {Name*:Lucky Shot} ({[int]age:12}) end"], &["day Tue: Wise Owl (87) end"], &[], &[]).expect("repeating records"),
            build_from(&["{Line:{[int]n:1} of {[int]m:3}}"], &["5 of 9", "6 of 9"], &[], &[]).expect("nested marks"),
            build_from(&["GET https://{host:example.com}/a {[int]code:200}"], &["GET https://trex.dev/b 404"], &[], &[]).expect("an accessor"),
            build_from(&["at {line:42} in {path:src/main.rs}"], &["at 7 in lib.rs"], &[], &[]).expect("report names"),
            build_template("{Person*:Name: {Name:Phoebe Cat}\nPhone: {Phone:425-123-6789}}", &["Name: Wise Owl", "Phone: 425-888-7766"]),
        ];
        for built in &builds {
            assert!(built.file().contains("\nfields extract {"), "{}", built.file());
            assert_eq!(read_file(built).1, built.fields, "{}", built.file());
        }
    }

    #[test]
    fn a_saved_build_reads_the_records_the_build_did() {
        let template = "{Person*:Name: {Name:Phoebe Cat}\nPhone: {Phone:425-123-6789}}";
        let built = build_template(template, &["Name: Wise Owl", "Phone: 425-888-7766", "Name: Big Bird", "Phone: 206-555-0100"]);
        let (set, fields) = read_file(&built);
        let pattern = set.let_of("extract").expect("the file declares extract");
        let text = "Name: Wise Owl\nPhone: 425-888-7766\nName: Big Bird\nPhone: 206-555-0100\n";
        let read = read_records(&fields, pattern, &set, text).expect("the fields read");
        let built_values: Vec<&Values> = built.records[1..].iter().map(|r| &r.values).collect();
        let read_values: Vec<&Values> = read.iter().map(|r| &r.values).collect();
        assert_eq!(read_values, built_values);
        assert_eq!(read.iter().map(|r| r.lines.clone()).collect::<Vec<_>>(), [vec![0, 1], vec![2, 3]]);
        let people = build_from(&["day {day:Mon}: {Name*:Phoebe Cat} ({[int]age:6}); {Name*:Lucky Shot} ({[int]age:12}) end"], &["day Tue: Wise Owl (87) end"], &[], &[]).expect("repeating records");
        let (set, fields) = read_file(&people);
        let read = read_records(&fields, set.let_of("extract").expect("extract"), &set, "day Wed: Elmo Red (3); Oscar Grouch (9) end\n").expect("the fields read");
        let names: Vec<Option<String>> = read.iter().map(|r| r.values[1].as_ref().map(Value::joined)).collect();
        assert_eq!(names, some(&[Some("Elmo Red"), Some("Oscar Grouch")]));
        assert!(read.iter().all(|r| r.values[0] == Some(Value::One("Wed".to_string()))));
    }

    #[test]
    fn a_fields_line_names_only_what_its_pattern_binds() {
        let base = "let p = (\\N):n (\\U):u\n";
        let refused = |line: &str| {
            let mut set = ShapeSet::new();
            set.declare_text(&format!("{base}{line}\n")).expect_err(line).msg
        };
        assert!(refused("fields q {n}").contains("no sub-pattern is declared as q"));
        assert!(refused("fields p {m}").contains("{m}"));
        assert!(refused("fields p {n} {n}").contains("named twice"));
        assert!(refused("fields p {[widget]n}").contains("[widget]"));
        assert!(refused("fields p {u:nosuch}").contains("{u:nosuch}"));
        assert!(refused("fields p n").contains("holds marks"));
        assert!(refused("fields p {n}\nfields p {u}").contains("has a fields line already"));
        let mut set = ShapeSet::new();
        set.declare_text(&format!("{base}fields p {{[int]n}} {{u:host}}\n")).expect("the line reads");
        let marks = set.fields_of("p").expect("p has fields");
        assert_eq!(marks.iter().map(crate::infer::marks::FieldMark::written).collect::<Vec<_>>(), ["{[int]n}", "{u:host}"]);
    }
}
