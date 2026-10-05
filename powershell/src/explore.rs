//! Reading an input's structure without a pattern: its tokens, its records,
//! the record shapes it repeats, the pattern a set of examples share, and a
//! text escaped so a pattern matches it literally.

use pwrs::prelude::*;
use trex::infer::build::Value;

use crate::atoms::{TrexLibrary, atoms_for};
use crate::common::{arg_err, units_of, value_of};
use crate::matching::{files_of, read_text, resolved};
use crate::query::record_unit;

/// One token as the lexer read it.
#[psclass(name = "Trex.Token")]
#[derive(Clone, Default)]
pub struct TrexToken {
    /// The token's kind: `number`, `word`, `quoted`, `ip`, `email`,
    /// `timestamp` and the rest, `open` and `close` for a bracket, or the
    /// name a declared shape or kind gave it.
    pub kind: String,
    /// The token's text.
    pub text: String,
    /// Where the token starts in the input, in UTF-16 code units.
    pub start: i64,
    /// How many UTF-16 code units the token spans.
    pub length: i64,
    /// The text parsed as its kind, as a capture's Value reads it; `$null`
    /// where the kind carries none.
    pub value: PsObject,
}

/// The tokens of `text`, lexed with `shapes` where they declare a shape or a
/// kind.
fn tokens_of(text: &str, shapes: &trex::ShapeSet, whitespace: bool) -> PsResult<Vec<TrexToken>> {
    let bytes = text.as_bytes();
    let toks = if shapes.is_empty() {
        trex::lexer::lex(bytes)
    } else {
        trex::lexer::lex_with_shapes(bytes, &trex::lexer::blob_runs(bytes), shapes, 0)
    };
    let mut units = units_of(bytes);
    let mut out = Vec::with_capacity(toks.len());
    for t in &toks {
        if !whitespace && !t.is_significant() {
            continue;
        }
        let (s, e) = (t.start(), t.end());
        let piece = String::from_utf8_lossy(&bytes[s..e]).into_owned();
        let value = match t.kind {
            trex::token::TokenKind::Custom(_) => PsObject::default(),
            kind => value_of(kind, &piece)?,
        };
        let us = units.at(s);
        let ue = units.at(e);
        out.push(TrexToken {
            kind: shapes.kind_name(t.kind),
            text: piece,
            start: us as i64,
            length: (ue - us) as i64,
            value,
        });
    }
    Ok(out)
}

/// Lists the tokens trex reads an input as, with each token's kind and its
/// value parsed as that kind.
///
/// The atoms in force decide the lex: a declared shape or kind is one token
/// of its own name. Whitespace is left out unless -IncludeWhitespace asks
/// for it, since a pattern never matches it. A file named outright that
/// holds a NUL byte is left unread and named in a warning, unless -Binary
/// asks for it; a walk passes over one.
///
/// # Examples
/// Get-TrexToken 'retry 3 times in 1500ms from 10.0.0.1'
/// Get-TrexToken -Path ./app.log | Group-Object Kind
#[cmdlet(verb = "Get", noun = "TrexToken", alias = "Get-TxToken", default_parameter_set = "Text", output = ["Trex.Token"])]
#[derive(Default)]
pub struct GetTrexToken {
    /// The text to lex; each string piped in is lexed on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files to lex; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Reads files that hold a NUL byte, which a walk treats as binary.
    #[param(set = "Path")]
    pub binary: bool,
    /// Writes the whitespace between tokens too.
    #[param]
    pub include_whitespace: bool,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
}

impl Cmdlet for GetTrexToken {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        self.shapes = Some(atoms_for(ps, &self.library)?);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shapes) = &self.shapes else {
            return Err(arg_err("TrexNoAtoms", "the atoms were not read"));
        };
        if self.path.is_empty() {
            return ps.write(tokens_of(&self.input_object, shapes, self.include_whitespace)?);
        }
        let named: Vec<std::path::PathBuf> =
            resolved(ps, &self.path, false)?.into_iter().map(std::path::PathBuf::from).collect();
        for source in files_of(ps, &self.path, false, &trex::files::WalkOptions::default())? {
            let trex::files::Source::File(path) = source else {
                continue;
            };
            match read_text(&path, self.binary) {
                Ok(Some(text)) => ps.write(tokens_of(&text, shapes, self.include_whitespace)?)?,
                // A file named outright that holds a NUL byte is named in a
                // warning, as Select-TrexMatch names one; a walk passes over one.
                Ok(None) => {
                    if named.contains(&path) {
                        ps.warning(&crate::common::binary_notice(&path.display().to_string()))?;
                    }
                }
                Err(e) => ps.write_error(&e)?,
            }
        }
        Ok(())
    }
}

/// One record of an input.
#[psclass(name = "Trex.Record")]
#[derive(Clone, Default)]
pub struct TrexRecord {
    /// The record's text.
    pub text: String,
    /// Where it starts in the input, in UTF-16 code units.
    pub start: i64,
    /// How many UTF-16 code units it spans.
    pub length: i64,
}

/// Splits an input into records: lines, paragraphs, blocks, or the other
/// units trex reads a record as.
///
/// -Unit takes `line`, `paragraph`, `file`, `period`, `seam`, `bind`,
/// `auto`, `texture`, `shape`, `block` or `unit`, as the trex command's
/// --record does.
///
/// # Examples
/// Get-TrexRecord (Get-Content ./notes.txt -Raw) -Unit paragraph
/// Get-TrexRecord $log -Unit auto | Select-Object -First 3
#[cmdlet(verb = "Get", noun = "TrexRecord", alias = "Get-TxRecord", output = ["Trex.Record"])]
#[derive(Default)]
pub struct GetTrexRecord {
    /// The text to split.
    #[param(mandatory, position = 0, value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// What a record is; `line` when absent.
    #[param(position = 1)]
    pub unit: Option<String>,
}

impl Cmdlet for GetTrexRecord {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let name = match &self.unit {
            Some(u) => u.clone(),
            None => "line".to_string(),
        };
        let unit = trex::records::RecordUnit::parse(&name).map_err(|e| arg_err("TrexRecordUnit", e))?;
        let bytes = self.input_object.as_bytes();
        let mut units = units_of(bytes);
        let mut out = Vec::new();
        for (s, e) in unit.records(bytes) {
            let us = units.at(s);
            let ue = units.at(e);
            out.push(TrexRecord {
                text: String::from_utf8_lossy(&bytes[s..e]).into_owned(),
                start: us as i64,
                length: (ue - us) as i64,
            });
        }
        ps.write(out)
    }
}

/// One record shape: the records that share a sequence of token kinds, with
/// the positions that vary written as their kind.
#[psclass(name = "Trex.RecordShape")]
#[derive(Clone, Default)]
pub struct TrexRecordShape {
    /// How many records have this shape.
    pub count: i64,
    /// The shape with its varying positions written as `<kind>`.
    pub readable: String,
    /// The shape as a trex pattern that matches its records.
    pub pattern: String,
    /// Whether the shape covers fewer records than the cut.
    pub rare: bool,
    /// With -Against, whether no template of the other input would accept
    /// these records; `$null` where nothing was compared.
    pub novel: PsObject,
    /// The records of this shape, counted from 0.
    pub records: Vec<i64>,
    /// How many records of the input had a shape at all, the total a share
    /// such as `-Cut 1%` is taken of.
    pub covered: i64,
}

/// Adds `text` to one stream of inputs, a newline between two where the
/// first does not end in one, so the records of one never run into the
/// next's.
fn append_input(stream: &mut Vec<u8>, text: &str) {
    if !stream.is_empty() && !stream.ends_with(b"\n") {
        stream.push(b'\n');
    }
    stream.extend_from_slice(text.as_bytes());
}

/// Lists the record shapes an input repeats, most frequent first: each
/// distinct sequence of token kinds once, with how many records have it.
///
/// A position whose text varies across the records is written as its kind.
/// A record is a line unless -Unit, -RecordStart or -RecordSpan says what
/// it is, and every string piped in, or every file named, is read as one
/// stream. -Rare keeps only the shapes that cover fewer records than the
/// cut: the mean when -Cut is absent, a count such as `5`, or a share such
/// as `1%`. -Against reads other inputs the same way and marks each shape
/// novel where no shape of theirs would accept its records, and -Novel
/// keeps only those.
///
/// -Head, -Tail and -Lines read only the first records of each input, its
/// last, or a range of them, counted in the unit a record is; -First and
/// -Last are -Head and -Tail. -Against reads its inputs whole. A file named
/// outright that holds a NUL byte is left unread and named in a warning,
/// unless -Binary asks for it; a walk passes over one.
///
/// # Examples
/// Get-TrexRecordShape -Path ./app.log | Select-Object -First 5
/// Get-TrexRecordShape -Path ./app.log -Rare -Cut 1%
/// Get-TrexRecordShape -Path ./today.log -Against ./yesterday.log -Novel
/// Get-TrexRecordShape -Path ./server.log -RecordStart '\T'
/// Get-TrexRecordShape -Path ./app.log -Tail 10000 -Against ./app.log.1 -Novel
#[cmdlet(verb = "Get", noun = "TrexRecordShape", alias = "Get-TxRecordShape", default_parameter_set = "Text", output = ["Trex.RecordShape"])]
#[derive(Default)]
pub struct GetTrexRecordShape {
    /// The text to read; every string piped in is read into one stream.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read as one stream; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read as one stream, read as written, as
    /// Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// What a record is: a line when absent, or a unit Find-TrexRecord
    /// reads, such as `paragraph`, `file` or `block`.
    #[param]
    pub unit: Option<String>,
    /// A pattern whose every match starts a record that runs to the next.
    #[param]
    pub record_start: Option<PsObject>,
    /// A pattern whose every match is a record.
    #[param]
    pub record_span: Option<PsObject>,
    /// Files or directories whose records each shape is compared with, read
    /// as one stream as the input is.
    #[param]
    pub against: Vec<String>,
    /// Writes only the shapes no shape of -Against would accept.
    #[param]
    pub novel: bool,
    /// Writes only the rare shapes.
    #[param]
    pub rare: bool,
    /// The cut a rare shape falls under: a count, a share such as `1%`, or
    /// `mean`.
    #[param]
    pub cut: Option<String>,
    /// Reads hidden files and directories a walk would skip.
    #[param]
    pub hidden: bool,
    /// Reads files an ignore rule excludes.
    #[param]
    pub no_ignore: bool,
    /// Reads files that hold a NUL byte, which a walk treats as binary.
    #[param]
    pub binary: bool,
    /// The atoms a record pattern is compiled against, in place of the
    /// session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    /// Reads the first this many records of each input, reading a file no
    /// further.
    #[param(alias = ["First"])]
    pub head: Option<u32>,
    /// Reads the last this many records of each input, reading a file
    /// backward from its end.
    #[param(alias = ["Last"])]
    pub tail: Option<u32>,
    /// Reads a range of records of each input, counted from one:
    /// `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`.
    #[param]
    pub lines: Option<PsObject>,
    records: Option<trex::records::RecordUnit>,
    rarity: Option<trex::templates::Rarity>,
    window: Option<trex::window::Select>,
    stream: Vec<u8>,
}

impl GetTrexRecordShape {
    fn walk(&self) -> trex::files::WalkOptions {
        trex::files::WalkOptions { hidden: self.hidden, no_ignore: self.no_ignore, ..trex::files::WalkOptions::default() }
    }

    /// The unit a record is.
    fn unit(&self) -> PsResult<&trex::records::RecordUnit> {
        self.records.as_ref().ok_or_else(|| arg_err("TrexRecordUnit", "the record unit was not read"))
    }

    /// The files `paths` name read into `stream`, each after the last, each
    /// cut to the records `select` names where it names some.
    fn read_into(
        &self,
        ps: &Pipeline<'_>,
        paths: &[String],
        literal: bool,
        select: Option<trex::window::Select>,
        stream: &mut Vec<u8>,
    ) -> PsResult<()> {
        let unit = self.unit()?;
        let named: Vec<std::path::PathBuf> =
            resolved(ps, paths, literal)?.into_iter().map(std::path::PathBuf::from).collect();
        for source in files_of(ps, paths, literal, &self.walk())? {
            if ps.stopping() {
                break;
            }
            let trex::files::Source::File(file) = source else {
                continue;
            };
            match crate::window::read_file(&file, select, unit, self.binary, false) {
                Ok(Some(read)) => append_input(stream, &read.text),
                // A file named outright that holds a NUL byte is named in a
                // warning, as Select-TrexMatch names one; a walk passes over one.
                Ok(None) => {
                    if named.contains(&file) {
                        ps.warning(&crate::common::binary_notice(&file.display().to_string()))?;
                    }
                }
                Err(e) => ps.write_error(&e)?,
            }
        }
        Ok(())
    }
}

impl Cmdlet for GetTrexRecordShape {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.novel && self.against.is_empty() {
            return Err(arg_err("TrexNovel", "-Novel keeps the shapes -Against holds none of; give -Against"));
        }
        self.rarity = Some(match &self.cut {
            Some(spec) => trex::templates::Rarity::parse(spec).map_err(|e| arg_err("TrexCut", e))?,
            None => trex::templates::Rarity::Mean,
        });
        self.records = Some(record_unit(ps, &self.unit, &self.record_start, &self.record_span, &self.library)?);
        self.window = crate::window::select_of(self.head, self.tail, &self.lines)?;
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut stream = std::mem::take(&mut self.stream);
        if self.path.is_empty() && self.literal_path.is_empty() {
            let read = crate::window::Read::of_text(&self.input_object, self.window, self.unit()?);
            append_input(&mut stream, &read.text);
        } else if self.literal_path.is_empty() {
            self.read_into(ps, &self.path, false, self.window, &mut stream)?;
        } else {
            self.read_into(ps, &self.literal_path, true, self.window, &mut stream)?;
        }
        self.stream = stream;
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        let (Some(unit), Some(cut)) = (&self.records, self.rarity) else {
            return Ok(());
        };
        let mine = |bytes: &[u8]| {
            trex::templates::Mining::mine_records(&trex::lexer::lex(bytes), bytes, &unit.records(bytes))
        };
        let mining = mine(&self.stream);
        let shared: Option<Vec<bool>> = if self.against.is_empty() {
            None
        } else {
            let mut other = Vec::new();
            self.read_into(ps, &self.against, false, None, &mut other)?;
            Some(mining.shared_with(&mine(&other)))
        };
        let covered = mining.covered() as i64;
        let mut shapes = Vec::with_capacity(mining.templates.len());
        for (i, t) in mining.templates.iter().enumerate() {
            let rare = mining.is_rare(i, cut);
            let novel = shared.as_ref().map(|s| !s[i]);
            if (self.rare && !rare) || (self.novel && novel != Some(true)) {
                continue;
            }
            shapes.push(TrexRecordShape {
                count: t.count() as i64,
                readable: t.readable(),
                pattern: t.pattern(),
                rare,
                novel: match novel {
                    Some(n) => n.into_ps()?,
                    None => PsObject::default(),
                },
                records: t.records.iter().map(|&r| r as i64).collect(),
                covered,
            });
        }
        shapes.sort_by_key(|s| std::cmp::Reverse(s.count));
        ps.write(shapes)
    }
}

/// One field of a built pattern.
#[psclass(name = "Trex.BuiltField", show = "{Name}")]
#[derive(Clone, Default)]
pub struct TrexBuiltField {
    /// The capture's name.
    pub name: String,
    /// The accessor a template writes after the capture where the field is
    /// part of one token, `host` of a URL; `$null` otherwise.
    pub accessor: PsObject,
    /// The type a mark gave the field as the mark writes it, `int` for
    /// `{[int]age:6}`, which its values are cast to; `$null` where none.
    pub type_name: PsObject,
    /// Whether a mark writes the field `{name*:text}`, so a line holding it
    /// begins a record.
    pub starts_record: bool,
    /// Whether a line marks the field more than once, so its value is every
    /// text it holds, in order, as an array.
    pub list: bool,
    /// Whether the field is inside records a line repeats, so a row holds
    /// all its values as an array and each record holds one.
    pub repeats: bool,
    /// The name of the field whose mark holds this one's, whose value then
    /// holds this field's under its last name part; `$null` at the top.
    pub parent: PsObject,
    /// How a report template writes the field.
    pub template: String,
}

/// One shape of the lines a built pattern reads: a branch of the pattern.
#[psclass(name = "Trex.BuiltShape", show = "{Pattern}")]
#[derive(Clone, Default)]
pub struct TrexBuiltShape {
    /// The branch.
    pub pattern: String,
    /// The indexes in Rows of the lines of this shape.
    pub lines: Vec<i64>,
    /// How the shape reaches each field, by name: `marked`, `missing`, or
    /// the index in Shapes of the shape whose literals place it here.
    pub reach: PsObject,
}

/// A pattern built for named fields, with the report on it. Its string form
/// is the pattern.
#[psclass(name = "Trex.BuiltPattern", show = "{Pattern}")]
#[derive(Clone, Default)]
pub struct TrexBuiltPattern {
    /// The pattern, one branch per shape.
    pub pattern: String,
    /// A report template writing every field, tab-separated.
    pub format: String,
    /// The pattern as a file -PatternFile and `trex lib` read, its `fields` line keeping each field's type, record start, accessor and order, so `\{extract}` read under it after Import-TrexAtom writes the objects this pattern writes.
    pub file: String,
    /// The shapes -MintShapes declared, each a line of File, `shape kb =
    /// `KB[0-9]{7}``; the pattern reads only under them, as
    /// ConvertFrom-TrexText reads it.
    pub declarations: Vec<String>,
    /// Each field every value of which a value class of the library holds,
    /// where no counter-example called for one, as `field: \{class} ...`;
    /// a -NotExample one of them refuses prints it in the pattern.
    pub suggestions: Vec<String>,
    /// The fields, in the order first marked or named.
    pub fields: Vec<TrexBuiltField>,
    /// The shapes, in the order of their branches in the pattern.
    pub shapes: Vec<TrexBuiltShape>,
    /// Each line: its Line index, the index of its Shape (`$null` for a
    /// line with no token), its Text, and Values, each field's value by
    /// name (an array of strings for a list field, a hashtable of Text and
    /// the fields inside for a field holding others), `$null` where the line
    /// does not hold it.
    pub rows: Vec<PsObject>,
    /// Each record: the indexes in Rows of its Lines, and Values, each
    /// field's first value among them by name, and for a field holding the
    /// one that begins the record, as a mark spanning lines does, its text on
    /// each line joined with a newline. A line holding a field that begins a
    /// record begins one and the lines after it join it; with no such field,
    /// each line is one.
    pub records: Vec<PsObject>,
}

impl TrexBuiltPattern {
    fn of(built: &trex::infer::build::Built) -> PsResult<Self> {
        use trex::infer::build::Reach;

        let or_null = |v: Option<&str>| -> PsResult<PsObject> {
            match v {
                Some(v) => v.into_ps(),
                None => Ok(PsObject::default()),
            }
        };
        let mut fields = Vec::with_capacity(built.fields.len());
        for f in &built.fields {
            fields.push(TrexBuiltField {
                name: f.name.clone(),
                accessor: or_null(f.accessor.as_deref())?,
                type_name: or_null(f.type_name.as_deref())?,
                starts_record: f.starts_record,
                list: f.list,
                repeats: f.repeats,
                parent: or_null(f.parent.map(|p| built.fields[p].name.as_str()))?,
                template: f.template.clone(),
            });
        }
        let mut shapes = Vec::with_capacity(built.shapes.len());
        for s in &built.shapes {
            let reach = PsHashtable::new()?;
            for (f, r) in built.fields.iter().zip(&s.reach) {
                let value = match r {
                    Reach::Marked => "marked".into_ps()?,
                    Reach::Anchored(from) => (*from as i64).into_ps()?,
                    Reach::Missing => "missing".into_ps()?,
                };
                reach.set(&f.name, value)?;
            }
            shapes.push(TrexBuiltShape {
                pattern: s.pattern.clone(),
                lines: s.lines.iter().map(|&l| l as i64).collect(),
                reach: reach.into_ps()?,
            });
        }
        let table = |values: &[Option<Value>]| values_table(built, values, None);
        let mut records = Vec::with_capacity(built.records.len());
        for r in &built.records {
            let record = pwrs::object::new_psobject("Trex.BuiltRecord");
            let lines: Vec<i64> = r.lines.iter().map(|&l| l as i64).collect();
            pwrs::object::add_note(&record, "Lines", lines.into_ps()?)?;
            pwrs::object::add_note(&record, "Values", table(&r.values)?)?;
            records.push(record);
        }
        let mut rows = Vec::with_capacity(built.rows.len());
        for (i, r) in built.rows.iter().enumerate() {
            let values = table(&r.values)?;
            let row = pwrs::object::new_psobject("Trex.BuiltRow");
            pwrs::object::add_note(&row, "Line", (i as i64).into_ps()?)?;
            let shape = match r.shape {
                Some(s) => (s as i64).into_ps()?,
                None => PsObject::default(),
            };
            pwrs::object::add_note(&row, "Shape", shape)?;
            pwrs::object::add_note(&row, "Text", r.text.as_str().into_ps()?)?;
            pwrs::object::add_note(&row, "Values", values)?;
            rows.push(row);
        }
        Ok(TrexBuiltPattern {
            pattern: built.pattern.clone(),
            format: built.format(),
            file: built.file(),
            declarations: built.declarations.clone(),
            suggestions: built
                .suggestions
                .iter()
                .map(|(field, classes)| {
                    let atoms: Vec<String> = classes.iter().map(|c| format!("\\{{{c}}}")).collect();
                    format!("{field}: {}", atoms.join(" "))
                })
                .collect(),
            fields,
            shapes,
            rows,
            records,
        })
    }
}

/// `values`, one per field of `built` in order, as a hashtable of the fields
/// inside `parent` (at the top where it is `None`) by their keys: a string,
/// a string array for a list field, `$null` where absent, and for a field
/// holding others a hashtable of its Text and theirs.
fn values_table(built: &trex::infer::build::Built, values: &[Option<Value>], parent: Option<usize>) -> PsResult<PsObject> {
    let table = PsHashtable::new()?;
    for f in built.inside(parent) {
        let one = match &values[f] {
            Some(Value::One(one)) => one.as_str().into_ps()?,
            Some(Value::Many(many)) => many.clone().into_ps()?,
            None => PsObject::default(),
        };
        let value = if values[f].is_some() && !built.inside(Some(f)).is_empty() {
            let inner = PsHashtable::from_ps(&values_table(built, values, Some(f))?)?;
            inner.set("Text", one)?;
            inner.into_ps()?
        } else {
            one
        };
        table.set(built.fields[f].key(), value)?;
    }
    table.into_ps()
}

/// Each field -Field names with the values given for it: a dictionary of a
/// name and a value, or several, each written as its text. An ordered
/// dictionary keeps its order; a hashtable, which has none, is read in the
/// order of its names.
fn hints_of(field: Option<&PsObject>) -> PsResult<Vec<(String, Vec<String>)>> {
    let Some(field) = field else { return Ok(Vec::new()) };
    let text = |v: &PsObject| String::from_ps(&v.call("ToString", &[])?);
    let table = PsHashtable::from_ps(field)?;
    let mut names = table.keys()?;
    if field.type_name()? == "System.Collections.Hashtable" {
        names.sort();
    }
    let mut hints = Vec::with_capacity(names.len());
    for name in names {
        let value = table.get(&name)?;
        let values = if value.type_name()?.ends_with("[]") {
            Vec::<PsObject>::from_ps(&value)?.iter().map(text).collect::<PsResult<Vec<_>>>()?
        } else {
            vec![text(&value)?]
        };
        hints.push((name, values));
    }
    Ok(hints)
}

/// How the builder builds a pattern beside the lines and fields it is
/// given.
#[derive(Clone, Copy, Default)]
struct How {
    /// Whether the lines themselves read in the markup.
    marks_in_lines: bool,
    /// Whether a match may start and end inside a longer line.
    unanchored: bool,
    /// How a shared byte shape is spelled.
    mint: trex::infer::build::Mint,
}

impl How {
    /// The mint -NoMint and -MintShapes ask for; both is refused.
    fn mint_of(no_mint: bool, mint_shapes: bool) -> PsResult<trex::infer::build::Mint> {
        match (no_mint, mint_shapes) {
            (true, true) => Err(arg_err("TrexMint", "-NoMint and -MintShapes ask for opposite spellings; give one")),
            (true, false) => Ok(trex::infer::build::Mint::Off),
            (false, true) => Ok(trex::infer::build::Mint::Shapes),
            (false, false) => Ok(trex::infer::build::Mint::Inline),
        }
    }
}

/// The pattern the builder makes of the lines of `lines`, lexed under
/// `shapes`: `marked` lines with each value written `{name:text}`, fields
/// named by value, and marks read in the lines themselves where `how` says.
/// A text of several lines is read as its lines, as ConvertFrom-String
/// reads one.
fn build_pattern(
    lines: &[String],
    counters: &[String],
    marked: &[String],
    hints: &[(String, Vec<String>)],
    how: How,
    shapes: &trex::ShapeSet,
) -> PsResult<trex::infer::build::Built> {
    let How { marks_in_lines, unanchored, mint } = how;
    let read = |src: &str| {
        trex::infer::marks::parse(src)
            .map_err(|e| arg_err("TrexMark", format!("{src:?}: column {}: {}", e.col, e.msg)))
    };
    let mut marks: Vec<trex::infer::marks::Marked> = Vec::new();
    for template in marked {
        let lines = trex::infer::marks::parse_lines(template).map_err(|(n, e)| {
            arg_err("TrexMark", format!("{template:?}: line {}: column {}: {}", n + 1, e.col, e.msg))
        })?;
        marks.extend(lines);
    }
    let mut data: Vec<String> = Vec::with_capacity(lines.len());
    let split = lines.iter().flat_map(|text| text.split('\n')).map(|l| l.strip_suffix('\r').unwrap_or(l));
    for line in split {
        let line = &line.to_string();
        if !marks_in_lines {
            data.push(line.clone());
            continue;
        }
        let m = read(line)?;
        if m.marks.is_empty() {
            data.push(m.text);
        } else {
            marks.push(m);
        }
    }
    let spec = trex::infer::build::Spec { lines: &data, marked: &marks, hints, counters, shapes, unanchored, mint };
    trex::infer::build::build(&spec).map_err(|e| arg_err("TrexBuild", e.to_string()))
}

/// Infers the most specific trex pattern every example matches, verified
/// against each.
///
/// Examples piped in are gathered and the pattern is written once, after
/// the last. -NotExample gives texts the pattern must not match, which is
/// what lets a position report a range of values rather than its kind.
///
/// With -Marked, -Field or -MarksInLines it builds the pattern that
/// extracts named fields from every shape of the examples instead, and
/// writes a Trex.BuiltPattern: -Marked gives lines with each value to
/// extract written `{name:text}`, ConvertFrom-String's markup; -Field names
/// a field by a value it takes, or several, found anywhere in the examples;
/// and -MarksInLines reads marks in the examples themselves. The pattern matches
/// whole lines unless -Unanchored, and is verified against every example. A
/// word field whose values share a constant part is spelled as its byte
/// shape, `` `KB[0-9]{7}` `` for KB5031354, unless -NoMint; -MintShapes
/// declares it as a named shape instead, held in Declarations. The lines are
/// lexed under the session's atoms, or -Library's, so a declared shape is one
/// token the pattern names.
///
/// # Examples
/// 'GET /a 200', 'POST /b 404' | ConvertTo-TrexPattern
/// ConvertTo-TrexPattern -Example 'id=12', 'id=40' -NotExample 'id=x' -Anchored
/// Get-Content titles.txt | ConvertTo-TrexPattern -Marked '{month:2023-10} Cumulative Update ({kb:KB5031354})'
/// ConvertTo-TrexPattern -Example (Get-Content titles.txt) -Field @{ kb = 'KB5031354' }
/// ConvertTo-TrexPattern -Example 'see XYZ-9 now' -Marked 'see {t:AB-12} now' -Library (New-TrexLibrary -Path ./ops.trex)
#[cmdlet(verb = "ConvertTo", noun = "TrexPattern", alias = "ConvertTo-TxPattern", output = ["System.String", "Trex.BuiltPattern"])]
#[derive(Default)]
pub struct ConvertToTrexPattern {
    /// The texts the pattern must match; a blank line among them holds no
    /// token and is left out.
    #[param(position = 0, value_from_pipeline)]
    pub example: Vec<String>,
    /// Texts the pattern must not match.
    #[param]
    pub not_example: Vec<String>,
    /// Wraps the pattern so it must match a whole input.
    #[param]
    pub anchored: bool,
    /// Lines with each value to extract written `{name:text}`; given, the
    /// pattern extracts those fields from every shape of the examples.
    #[param]
    pub marked: Vec<String>,
    /// Fields named by value: a dictionary of each field's name and a value
    /// it takes, or several, found anywhere in the examples.
    #[param]
    pub field: Option<PsObject>,
    /// Reads marks inside the examples themselves.
    #[param]
    pub marks_in_lines: bool,
    /// Lets a built pattern's match start and end inside a longer line.
    #[param]
    pub unanchored: bool,
    /// Spells every word field as its kind, with no byte shape.
    #[param]
    pub no_mint: bool,
    /// Declares each word field's byte shape as a named shape the pattern
    /// reads, in place of an inline byte atom.
    #[param]
    pub mint_shapes: bool,
    /// The atoms the lines of a pattern built for fields are lexed under, in
    /// place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    gathered: Vec<String>,
}

impl ConvertToTrexPattern {
    /// Whether a field is named, so a pattern is built for fields.
    fn builds(&self) -> bool {
        !self.marked.is_empty() || self.field.is_some() || self.marks_in_lines
    }
}

impl Cmdlet for ConvertToTrexPattern {
    fn begin(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        if self.library.is_some() && !self.builds() {
            return Err(arg_err(
                "TrexLibrary",
                "-Library lexes the lines of a pattern built for fields; give -Marked, -Field or -MarksInLines",
            ));
        }
        Ok(())
    }

    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        self.gathered.extend(self.example.iter().cloned());
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        if self.builds() {
            let hints = hints_of(self.field.as_ref())?;
            let shapes = atoms_for(ps, &self.library)?;
            let how = How {
                marks_in_lines: self.marks_in_lines,
                unanchored: self.unanchored,
                mint: How::mint_of(self.no_mint, self.mint_shapes)?,
            };
            let built = build_pattern(&self.gathered, &self.not_example, &self.marked, &hints, how, &shapes)?;
            return ps.write(TrexBuiltPattern::of(&built)?);
        }
        let examples: Vec<&[u8]> = self.gathered.iter().map(String::as_bytes).collect();
        let counters: Vec<&[u8]> = self.not_example.iter().map(String::as_bytes).collect();
        match trex::infer::infer_against(&examples, &counters) {
            Ok(inferred) => ps.write(if self.anchored { inferred.anchored() } else { inferred.pattern().to_string() }),
            Err(e) => Err(arg_err("TrexInfer", e.to_string())),
        }
    }
}

/// A pattern and the fields ConvertFrom-TrexText reads from each match, the
/// type each is cast to, and the record the matches read so far make.
struct Reader {
    compiled: crate::pattern::Compiled,
    fields: Vec<trex::infer::build::Field>,
    /// The type each field's values are cast to, resolved once.
    casts: Vec<Option<PsObject>>,
    /// Each field's value in the record still open, while one is.
    record: Option<Vec<Option<Value>>>,
}

impl Reader {
    /// The reader of `compiled` for `fields`, each field's parent the one
    /// its name, up to its last dot, names: `Line` for `Line.n`, as trex
    /// names a capture inside a capture.
    fn new(compiled: crate::pattern::Compiled, mut fields: Vec<trex::infer::build::Field>) -> PsResult<Reader> {
        for f in 0..fields.len() {
            fields[f].parent = match fields[f].name.rsplit_once('.') {
                Some((outer, _)) => fields.iter().position(|g| g.name == outer),
                None => None,
            };
        }
        let mut casts = Vec::with_capacity(fields.len());
        for field in &fields {
            casts.push(match &field.type_name {
                Some(name) => Some(type_named(name)?),
                None => None,
            });
        }
        Ok(Reader { compiled, fields, casts, record: None })
    }

    /// The reader of a pattern the builder made, compiled against `shapes`
    /// and the shapes it declares.
    fn built(built: &trex::infer::build::Built, shapes: trex::ShapeSet) -> PsResult<Reader> {
        let shapes = declared(shapes, &built.declarations)?;
        Reader::new(crate::pattern::Compiled::new(&built.pattern, shapes)?, built.fields.clone())
    }

    /// The reader of a Trex.BuiltPattern, whose fields keep the accessor
    /// and the type the builder gave them; of `\{name}`, a sub-pattern a
    /// `fields` line gives its fields, which it reads as the builder did; and
    /// of any other Trex.Pattern or source text, whose registers are the
    /// fields as they read.
    fn of(ps: &Pipeline<'_>, given: &PsObject, library: &Option<PsProxy<TrexLibrary>>) -> PsResult<Reader> {
        if given.type_name()? != "Trex.BuiltPattern" {
            let compiled = crate::pattern::pattern_arg(ps, given, library)?;
            let fields = trex::infer::build::fields_for(&compiled.source, &compiled.inner, &compiled.shapes);
            return Reader::new(compiled, fields);
        }
        let text_of = |obj: &PsObject, name: &str| -> PsResult<Option<String>> {
            let v = pwrs::object::property(obj, name)?;
            if v.is_null() { Ok(None) } else { Ok(Some(String::from_ps(&v)?)) }
        };
        let pattern = text_of(given, "Pattern")?.ok_or_else(|| arg_err("TrexNoPattern", "the built pattern has none"))?;
        let declarations = Vec::<String>::from_ps(&pwrs::object::property(given, "Declarations")?)?;
        let compiled = crate::pattern::Compiled::new(&pattern, declared(atoms_for(ps, library)?, &declarations)?)?;
        let mut fields = Vec::new();
        for f in Vec::<PsObject>::from_ps(&pwrs::object::property(given, "Fields")?)? {
            let name = text_of(&f, "Name")?.ok_or_else(|| arg_err("TrexField", "a field of the built pattern has no name"))?;
            let type_name = text_of(&f, "TypeName")?;
            fields.push(trex::infer::build::Field {
                accessor: text_of(&f, "Accessor")?,
                template: format!("${{{name}}}"),
                hint: type_name.as_deref().and_then(trex::infer::marks::Hint::parse),
                starts_record: bool::from_ps(&pwrs::object::property(&f, "StartsRecord")?)?,
                list: bool::from_ps(&pwrs::object::property(&f, "List")?)?,
                repeats: bool::from_ps(&pwrs::object::property(&f, "Repeats")?)?,
                parent: None,
                type_name,
                name,
            });
        }
        Reader::new(compiled, fields)
    }

    /// Each field's value in each record the matches in `text` hold, as
    /// trex::infer::build::record_parts cuts a match: a match of records a
    /// line repeats gives one per record, each carrying the line's other
    /// fields, and any other match one; `None` where it holds none.
    fn values(&self, text: &str) -> PsResult<Vec<Vec<Option<Value>>>> {
        let mut out = Vec::new();
        for m in self.compiled.found(text.as_bytes()) {
            let parts = trex::infer::build::record_parts(&self.fields, &m, text).map_err(|e| arg_err("TrexField", e))?;
            out.extend(parts);
        }
        Ok(out)
    }

    /// The objects the matches in `text` complete, as ConvertFrom-String
    /// reads records: where no field begins a record, one per match; where
    /// one does, a match holding it begins a record, a match after it joins
    /// it as trex::infer::build::join_record joins one, and a match before
    /// the first joins none; records a line repeats are one each. The record
    /// still open is left for the next text.
    fn read(&mut self, text: &str) -> PsResult<Vec<PsObject>> {
        let records = self.fields.iter().any(|f| f.starts_record);
        let mut out = Vec::new();
        for values in self.values(text)? {
            if !records {
                out.push(self.object(&values)?);
                continue;
            }
            let begins = self.fields.iter().zip(&values).any(|(f, v)| f.starts_record && v.is_some());
            if begins {
                if let Some(done) = self.record.replace(values) {
                    out.push(self.object(&done)?);
                }
            } else if let Some(record) = self.record.as_mut() {
                trex::infer::build::join_record(&self.fields, record, &values);
            }
        }
        Ok(out)
    }

    /// The record still open, as its object.
    fn finish(&mut self) -> PsResult<Option<PsObject>> {
        match self.record.take() {
            Some(values) => Ok(Some(self.object(&values)?)),
            None => Ok(None),
        }
    }

    /// An object with a property per field at the top, in order.
    fn object(&self, values: &[Option<Value>]) -> PsResult<PsObject> {
        self.object_of(values, None)
    }

    /// An object with a property per field inside `parent` (at the top where
    /// it is `None`), by its key: its value cast to the type its mark names,
    /// the text where it names none, an array of them for a list field,
    /// `$null` where there is no value, and for a field holding others an
    /// object of its Text and theirs. An object of a field inside another
    /// begins with that field's Text.
    fn object_of(&self, values: &[Option<Value>], parent: Option<usize>) -> PsResult<PsObject> {
        let obj = pwrs::object::new_psobject("Trex.TextObject");
        if let Some(p) = parent {
            pwrs::object::add_note(&obj, "Text", self.cast(p, &values[p])?)?;
        }
        for (f, field) in self.fields.iter().enumerate().filter(|(_, field)| field.parent == parent) {
            let holds = self.fields.iter().any(|g| g.parent == Some(f));
            let value = if holds && values[f].is_some() { self.object_of(values, Some(f))? } else { self.cast(f, &values[f])? };
            pwrs::object::add_note(&obj, field.key(), value)?;
        }
        Ok(obj)
    }

    /// Field `f`'s value cast to the type its mark names, the text where it
    /// names none, an array of them for a list field, `$null` where absent.
    fn cast(&self, f: usize, value: &Option<Value>) -> PsResult<PsObject> {
        let one = |text: &str| -> PsResult<PsObject> {
            match &self.casts[f] {
                Some(to) => cast_to(text, to),
                None => text.into_ps(),
            }
        };
        match value {
            Some(Value::One(text)) => one(text),
            Some(Value::Many(texts)) => texts.iter().map(|t| one(t)).collect::<PsResult<Vec<PsObject>>>()?.into_ps(),
            None => Ok(PsObject::default()),
        }
    }
}

/// `shapes` with the `shape` lines of a built pattern declared in it.
fn declared(mut shapes: trex::ShapeSet, declarations: &[String]) -> PsResult<trex::ShapeSet> {
    shapes
        .declare_text(&declarations.join("\n"))
        .map_err(|e| arg_err("TrexDeclare", format!("the built pattern's shapes: {e}")))?;
    Ok(shapes)
}

/// The .NET type a mark's `[type]` names, resolved as PowerShell resolves
/// the type of a cast: `int` is `System.Int32`.
fn type_named(name: &str) -> PsResult<PsObject> {
    let of_type = PsType::from_name("System.Type").call_static("GetType", &["System.Type".into_ps()?])?;
    PsType::from_name("System.Management.Automation.LanguagePrimitives").call_static("ConvertTo", &[name.into_ps()?, of_type])
}

/// `text` cast to the type `to` as PowerShell casts it, the cast
/// ConvertFrom-String makes: `[bool]` is true for any text but the empty.
fn cast_to(text: &str, to: &PsObject) -> PsResult<PsObject> {
    PsType::from_name("System.Management.Automation.LanguagePrimitives")
        .call_static("ConvertTo", &[text.into_ps()?, to.clone()])
}

/// Converts lines of text to objects, one per line a pattern reads, with a
/// property per field, as ConvertFrom-String does from a template.
///
/// -Pattern is a pattern ConvertTo-TrexPattern built, whose fields are read
/// as it placed them and cast to their marks' [type] as PowerShell casts,
/// or a Trex.Pattern or source text, whose registers are the fields.
/// -Marked builds the pattern from the marked lines, a template of several
/// lines included, and every line piped in, as ConvertFrom-String
/// -TemplateContent learns from its template, then converts every line
/// piped in. A string of several lines gives an object per line the pattern
/// reads, and a line it does not read gives none. Where a mark writes a
/// field `{name*:text}`, a line holding it begins a record, the lines after
/// it join the record across the strings piped in, and one object is
/// written per record, as ConvertFrom-String reads one; a field a record
/// lacks is `$null`. A field a line marks more than once, and a register
/// bound under a repetition, is an array of every value it holds, each cast.
/// A field marked inside another, and a register bound inside another, is
/// a property of an object holding the outer one's Text. Where a template
/// line marks a starred field more than once, each record a line repeats is
/// an object, carrying the line's other fields.
///
/// # Examples
/// $built = Get-Content titles.txt | ConvertTo-TrexPattern -Marked '{month:2023-10} Update ({kb:KB5031354})'
/// Get-Content new-titles.txt | ConvertFrom-TrexText -Pattern $built
/// Get-Content stock.txt | ConvertFrom-TrexText -Marked '{fruit:apples} {[int]qty:42}'
#[cmdlet(verb = "ConvertFrom", noun = "TrexText", alias = "ConvertFrom-TxText", default_parameter_set = "Pattern", output = ["System.Management.Automation.PSObject"])]
#[derive(Default)]
pub struct ConvertFromTrexText {
    /// The pattern: one ConvertTo-TrexPattern built, a Trex.Pattern, or
    /// source text.
    #[param(mandatory, position = 0, set = "Pattern")]
    pub pattern: PsObject,
    /// The text to convert; each string piped in is read on its own.
    #[param(mandatory, position = 1, value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Lines with each value to extract written `{name:text}`; the pattern
    /// is built from them and every line piped in.
    #[param(mandatory, set = "Marked")]
    pub marked: Vec<String>,
    /// Fields named by value, as ConvertTo-TrexPattern -Field takes them.
    #[param(set = "Marked")]
    pub field: Option<PsObject>,
    /// Texts the built pattern must not match.
    #[param(set = "Marked")]
    pub not_example: Vec<String>,
    /// Spells every word field of the built pattern as its kind, with no
    /// byte shape.
    #[param(set = "Marked")]
    pub no_mint: bool,
    /// Declares each word field's byte shape as a named shape the built
    /// pattern reads, in place of an inline byte atom.
    #[param(set = "Marked")]
    pub mint_shapes: bool,
    /// The atoms a pattern is compiled against, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    reader: Option<Reader>,
    gathered: Vec<String>,
}

impl Cmdlet for ConvertFromTrexText {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.marked.is_empty() {
            self.reader = Some(Reader::of(ps, &self.pattern, &self.library)?);
        }
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        match &mut self.reader {
            Some(reader) => ps.write(reader.read(&self.input_object)?),
            None => {
                self.gathered.push(self.input_object.clone());
                Ok(())
            }
        }
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        if !self.marked.is_empty() {
            let hints = hints_of(self.field.as_ref())?;
            let shapes = atoms_for(ps, &self.library)?;
            let how = How { mint: How::mint_of(self.no_mint, self.mint_shapes)?, ..How::default() };
            let built = build_pattern(&self.gathered, &self.not_example, &self.marked, &hints, how, &shapes)?;
            let mut reader = Reader::built(&built, shapes)?;
            for line in &self.gathered {
                ps.write(reader.read(line)?)?;
            }
            self.reader = Some(reader);
        }
        if let Some(reader) = &mut self.reader
            && let Some(last) = reader.finish()?
        {
            ps.write(last)?;
        }
        Ok(())
    }
}

/// Escapes a text so a trex pattern matches it literally.
///
/// # Examples
/// ConvertTo-TrexLiteral 'price: $5 (net)'
#[cmdlet(verb = "ConvertTo", noun = "TrexLiteral", alias = "ConvertTo-TxLiteral", output = ["System.String"])]
#[derive(Default)]
pub struct ConvertToTrexLiteral {
    /// The text to escape.
    #[param(mandatory, position = 0, value_from_pipeline, allow_empty_string)]
    pub input_object: String,
}

impl Cmdlet for ConvertToTrexLiteral {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        ps.write(trex::escape(&self.input_object))
    }
}
