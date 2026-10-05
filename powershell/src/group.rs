//! Grouping matches by a rendered key, with aggregates over a register's
//! typed values: what the trex command's `top`, `count-by` and `uniq` print,
//! as objects.
//!
//! The aggregates are trex's own, computed exactly over every value a key
//! collected, and each arrives as the .NET type that holds it: a sum, an
//! extreme or a percentile of numbers as `long` or `decimal`, of durations
//! as `TimeSpan`, of instants as `DateTimeOffset`, and of versions or
//! addresses as their text. An average whose decimal repeats arrives as the
//! `decimal` nearest it.

use pwrs::prelude::*;
use trex::aggregate::{Column, ColumnError};
use trex::typed::Agg;

use crate::atoms::TrexLibrary;
use crate::common::{arg_err, duration, number, repeating, typed_value, units_of};
use crate::matching::{Scanning, files_of, resolved};
use crate::window::{Part, Read};

/// The order groups are written in.
#[psenum(name = "Trex.GroupOrder")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GroupOrder {
    /// Most matches first, and by key among equal counts.
    #[default]
    Count,
    /// By key.
    Key,
}

/// How a percentile is decided when its rank is between two observed values.
#[psenum(name = "Trex.PercentileMethod")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PercentileMethod {
    /// The value at rank ceil(p * n), always one that occurred.
    #[default]
    Nearest,
    /// Interpolated between the two neighbors; numeric kinds only.
    Linear,
    /// The value at or below the rank.
    Lower,
    /// Interpolated where the kind allows it, the nearest value otherwise.
    Hybrid,
}

impl PercentileMethod {
    fn trex(self) -> trex::typed::Percentile {
        match self {
            PercentileMethod::Nearest => trex::typed::Percentile::Nearest,
            PercentileMethod::Linear => trex::typed::Percentile::Linear,
            PercentileMethod::Lower => trex::typed::Percentile::Lower,
            PercentileMethod::Hybrid => trex::typed::Percentile::Hybrid,
        }
    }
}

/// The property a group writes an aggregate as.
fn property(agg: Agg) -> String {
    match agg {
        Agg::Sum => "Sum".to_string(),
        Agg::Avg => "Average".to_string(),
        Agg::Min => "Minimum".to_string(),
        Agg::Max => "Maximum".to_string(),
        Agg::Pct(p) => format!("P{p}"),
    }
}

/// The property a column is written as: `name` alone where its parameter
/// names one register, and `name_register` where it names `of` of them, so
/// each register's column is told apart.
fn named(name: &str, spec: &str, of: usize, agg: Agg) -> PsResult<String> {
    if of == 1 {
        return Ok(name.to_string());
    }
    match trex::aggregate::one_register(spec) {
        Some(register) => Ok(format!("{name}_{register}")),
        None => Err(column_err(&ColumnError::NotOneRegister { agg, spec: spec.to_string() })),
    }
}

/// Why a column cannot be computed, in the parameters' terms.
fn column_err(e: &ColumnError) -> PsError {
    let message = match e {
        ColumnError::NotOneRegister { spec, .. } => {
            format!("an aggregate reads one register, as `size` or `${{size}}`, not {spec:?}")
        }
        ColumnError::NotBound { register, .. } => format!("the pattern binds no register named {register}"),
        ColumnError::NoSingleKind { register, .. } => format!(
            "{register} binds a run, a repetition or an alternation, or two patterns of the set bind it on different kinds, so its kind varies between matches"
        ),
        ColumnError::NotAdmitted { agg, register, kind } => format!(
            "{} of {register}, which binds a {}, is not defined for that kind",
            property(*agg),
            kind.name()
        ),
        ColumnError::LinearOnOrdered { kind, .. } => {
            format!("a linear percentile cannot interpolate a {}; use Nearest, Lower or Hybrid", kind.name())
        }
    };
    arg_err("TrexAggregate", message)
}

/// One column's aggregate over the values a key collected, as the .NET type
/// that holds it: the value itself where the aggregate picks one, a minimum,
/// a maximum or a percentile that is one of the observed values, and the figure
/// trex computes where it does not, a sum, an average or an interpolated
/// percentile, all of which are numbers.
fn aggregate(
    held: &trex::typed::Collected,
    col: &Column,
    method: trex::typed::Percentile,
    clock: trex::Clock,
) -> PsResult<PsObject> {
    if let Some(v) = held.picked(col.agg, col.kind, method) {
        return typed_value(col.kind, v);
    }
    let style = trex::typed::ValueStyle::new();
    let form = trex::typed::QuotientForm::Repetend;
    let Some(rendered) = held.report(col.agg, col.kind, style, form, method, clock) else {
        return Ok(PsObject::default());
    };
    // A figure a double does not hold exactly is rendered as a JSON string.
    let figure = match rendered.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Some(inner) => inner,
        None => rendered.as_str(),
    };
    if col.kind == trex::token::TokenKind::Duration {
        duration(figure)
    } else if figure.ends_with(')') {
        repeating(figure)
    } else {
        number(figure)
    }
}

/// Groups the matches of a trex pattern by a rendered key, with the count of
/// each and aggregates over a register's typed values.
///
/// -Key is a report template: `${ip:octet1-2}`, `${u:host}`, `${e:domain}`,
/// `${path}` for the file a match is in. Matches from every string piped in
/// and every file named are grouped together, and the groups are written
/// once, after the last input. -Sum, -Average, -Minimum and -Maximum each
/// read the registers they name, whose kind orders or adds, and -Percentile
/// reads each register -PercentileOf names. An aggregate over one register
/// is written under its own name, `Sum` or `P95`, and over several under its
/// name and each register's, `Sum_t` and `Sum_size`. -Unique writes the keys
/// alone. A file named outright that holds a NUL byte is left unread and
/// named in a warning, unless -Binary asks for it; a walk passes over one.
///
/// Several patterns, or the members of a -PatternFile, group as one set,
/// and a key reads the member that made each match as `${pattern}`.
///
/// -Head, -Tail and -Lines group only the first lines of each input, its
/// last, or a range of them, counted in records of -Unit where it names one,
/// a key reading each match at its input's own line; -First and -Last are
/// -Head and -Tail. -MaxCount writes only the first groups.
///
/// # Examples
/// Group-TrexMatch '\I:ip' -Key '${ip:octet1-2}' -Path ./access.log
/// Group-TrexMatch '\W:u \R:t' -Key '${u}' -Sum t -Maximum t -Path ./timing.log
/// Group-TrexMatch '\W:u \R:t \Z:size' -Key '${u}' -Sum t, size -Path ./timing.log
/// Group-TrexMatch '\R:t' -Key all -Percentile 50, 95, 99 -PercentileOf t -Path ./timing.log
/// Group-TrexMatch -PatternFile ./secrets.trex -Key '${pattern}' -Path ./src
/// Group-TrexMatch '\I:ip' -Key '${ip}' -Path ./access.log -Tail 1000 -MaxCount 10
#[cmdlet(verb = "Group", noun = "TrexMatch", alias = "Group-TxMatch", default_parameter_set = "Text", output = ["Trex.Group", "System.String"])]
#[derive(Default)]
pub struct GroupTrexMatch {
    /// The patterns: trex source text or Trex.Pattern objects; several group
    /// as one set.
    #[param(position = 0)]
    pub pattern: Vec<PsObject>,
    /// A pattern file whose members group as one set: each `let name =
    /// pattern` line under its name and each bare pattern line under its
    /// line number, with the file's declarations in force. -Key is named
    /// beside it, since a first argument by position is read as -Pattern.
    #[param]
    pub pattern_file: Option<String>,
    /// The key each match is grouped by, a report template.
    #[param(mandatory, position = 1)]
    pub key: String,
    /// The text to scan; every string piped in is grouped with the rest.
    #[param(mandatory, value_from_pipeline, set = "Text", allow_empty_string)]
    pub input_object: String,
    /// Files or directories to scan; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to scan, read as written, as Get-ChildItem
    /// pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// Reads hidden files and directories a walk would skip.
    #[param(set = ["Path", "LiteralPath"])]
    pub hidden: bool,
    /// Reads files an ignore rule excludes.
    #[param(set = ["Path", "LiteralPath"])]
    pub no_ignore: bool,
    /// Reads files that hold a NUL byte, which a walk treats as binary.
    #[param(set = ["Path", "LiteralPath"])]
    pub binary: bool,
    /// The order the groups are written in: Count, most first, when absent.
    #[param]
    pub sort_by: Option<GroupOrder>,
    /// Writes only the first this many groups.
    #[param]
    pub max_count: Option<u32>,
    /// Groups the matches of the first this many lines of each input, or
    /// records of -Unit, reading a file no further.
    #[param(alias = ["First"])]
    pub head: Option<u32>,
    /// Groups the matches of the last this many lines of each input, or
    /// records of -Unit, reading a file backward from its end.
    #[param(alias = ["Last"])]
    pub tail: Option<u32>,
    /// Groups the matches of a range of lines, or records of -Unit, counted
    /// from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's
    /// `100..200`. A match counts only if it is wholly inside.
    #[param]
    pub lines: Option<PsObject>,
    /// What -Head, -Tail and -Lines count: a line when absent, or a unit
    /// Find-TrexRecord reads, such as `paragraph` or `block`.
    #[param]
    pub unit: Option<String>,
    /// Writes the distinct keys alone.
    #[param]
    pub unique: bool,
    /// The registers whose values each group sums: one is written as `Sum`,
    /// several as `Sum_<register>` each.
    #[param]
    pub sum: Vec<String>,
    /// The registers whose values each group averages: one is written as
    /// `Average`, several as `Average_<register>` each.
    #[param]
    pub average: Vec<String>,
    /// The registers whose least value each group reports: one is written as
    /// `Minimum`, several as `Minimum_<register>` each.
    #[param]
    pub minimum: Vec<String>,
    /// The registers whose greatest value each group reports: one is written
    /// as `Maximum`, several as `Maximum_<register>` each.
    #[param]
    pub maximum: Vec<String>,
    /// Percentiles to report, such as 50 and 95, of each register
    /// -PercentileOf names.
    #[param]
    pub percentile: Vec<u32>,
    /// The registers the percentiles read: one writes `P50`, `P95` and so
    /// on, several write `P50_<register>` for each.
    #[param]
    pub percentile_of: Vec<String>,
    /// How a percentile between two observed values is decided: Nearest
    /// when absent.
    #[param]
    pub percentile_method: Option<PercentileMethod>,
    /// The atoms a source-text pattern is compiled against, in place of the
    /// session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    scanning: Option<Scanning>,
    template: Option<trex::Template>,
    table: trex::aggregate::Table,
    /// The property each of the table's columns is written as.
    properties: Vec<String>,
    part: Part,
}

impl GroupTrexMatch {
    /// The percentile method asked for, Nearest where none was.
    fn method(&self) -> PercentileMethod {
        match self.percentile_method {
            Some(m) => m,
            None => PercentileMethod::Nearest,
        }
    }

    /// The columns asked for, checked against the pattern, each with the
    /// property it is written as.
    fn columns_of(&self, sc: &Scanning) -> PsResult<(Vec<Column>, Vec<String>)> {
        let mut asked: Vec<(Agg, String)> = Vec::new();
        let mut properties: Vec<String> = Vec::new();
        for (agg, specs) in
            [(Agg::Sum, &self.sum), (Agg::Avg, &self.average), (Agg::Min, &self.minimum), (Agg::Max, &self.maximum)]
        {
            for spec in specs {
                properties.push(named(&property(agg), spec, specs.len(), agg)?);
                asked.push((agg, spec.clone()));
            }
        }
        if !self.percentile.is_empty() {
            if self.percentile_of.is_empty() {
                return Err(arg_err("TrexAggregate", "-Percentile needs -PercentileOf to name the register it reads"));
            }
            for spec in &self.percentile_of {
                for &p in &self.percentile {
                    if p > 100 {
                        return Err(arg_err("TrexAggregate", format!("a percentile is 0 to 100, not {p}")));
                    }
                    let agg = Agg::Pct(p);
                    properties.push(named(&property(agg), spec, self.percentile_of.len(), agg)?);
                    asked.push((agg, spec.clone()));
                }
            }
        }
        for (i, name) in properties.iter().enumerate() {
            if properties[..i].contains(name) {
                return Err(arg_err(
                    "TrexAggregate",
                    format!("{name} is asked for twice; name each register and each percentile once"),
                ));
            }
        }
        let columns =
            trex::aggregate::columns(&asked, &sc.capture_kinds(), self.method().trex()).map_err(|e| column_err(&e))?;
        Ok((columns, properties))
    }

    /// Whether the key reads a match's line, column or offsets, so a tail
    /// must count what comes before it.
    fn reads_place(&self) -> bool {
        self.template.as_ref().is_some_and(|t| t.reads_place() || t.reads_offsets())
    }

    /// Count and gather the matches of `read`, the part of the input at
    /// `path` read, each under its key.
    fn tally(&mut self, path: &str, read: &Read) -> PsResult<()> {
        let (Some(sc), Some(template)) = (&self.scanning, &self.template) else {
            return Err(arg_err("TrexNoPattern", "the pattern was not compiled"));
        };
        let bytes = read.text.as_bytes();
        let found = sc.found(bytes);
        let index = template.reads_place().then(|| read.index());
        let mut units = template.reads_offsets().then(|| units_of(bytes));
        for (member, m) in &found {
            let (line, col) = match &index {
                Some(ix) => ix.line_col(bytes, m.start),
                None => (0, 0),
            };
            let offsets = match units.as_mut() {
                Some(u) => read.unit_base.map(|ahead| (ahead + u.at(m.start), ahead + u.at(m.end))),
                None => None,
            };
            let name = sc.name(*member);
            let pattern = if member.is_some() { Some(name.as_str()) } else { None };
            let place = trex::ReportAt { path, line, col, offsets, pattern, rule: None };
            self.table.add(template.render_report(m, bytes, &place), m, bytes);
        }
        Ok(())
    }
}

impl Cmdlet for GroupTrexMatch {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let sc = Scanning::of(ps, &self.pattern, &self.pattern_file, &self.library)?;
        let template = trex::Template::parse_report(&self.key, &sc.capture_names())
            .map_err(|e| arg_err("TrexTemplateError", format!("-Key error at byte {}: {}", e.pos, e.msg)))?;
        // A key groups matches, and an axis reading is not one: nothing here
        // explains a match, so every `${@axis}` would render empty and put
        // every match under one key.
        if template.reads_explanation() {
            return Err(arg_err(
                "TrexTemplateError",
                "-Key groups matches and reads no axis; ${@axis} is a -Format field of Select-TrexMatch, where -Explain reads it",
            ));
        }
        let (columns, properties) = self.columns_of(&sc)?;
        self.table = trex::aggregate::Table::new(columns);
        self.properties = properties;
        self.part = Part::of(self.head, self.tail, &self.lines, &self.unit)?;
        self.template = Some(template);
        self.scanning = Some(sc);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.path.is_empty() && self.literal_path.is_empty() {
            let read = self.part.of_text(&self.input_object);
            return self.tally("", &read);
        }
        let (paths, literal) =
            if self.literal_path.is_empty() { (self.path.clone(), false) } else { (self.literal_path.clone(), true) };
        let opts = trex::files::WalkOptions {
            hidden: self.hidden,
            no_ignore: self.no_ignore,
            ..trex::files::WalkOptions::default()
        };
        let placed = self.reads_place();
        let named: Vec<std::path::PathBuf> =
            resolved(ps, &paths, literal)?.into_iter().map(std::path::PathBuf::from).collect();
        for source in files_of(ps, &paths, literal, &opts)? {
            let trex::files::Source::File(path) = source else {
                continue;
            };
            match self.part.read(&path, self.binary, placed) {
                Ok(Some(read)) => self.tally(&path.display().to_string(), &read)?,
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

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        let order = match self.sort_by {
            Some(GroupOrder::Key) => trex::aggregate::Order::Key,
            Some(GroupOrder::Count) | None => trex::aggregate::Order::Count,
        };
        let mut rows = self.table.rows(order);
        if let Some(n) = self.max_count {
            rows.truncate(n as usize);
        }
        if self.unique {
            for row in rows {
                ps.write(row.key.to_string())?;
            }
            return Ok(());
        }
        let method = self.method().trex();
        let clock = trex::Clock::current();
        for row in rows {
            let group = pwrs::object::new_psobject("Trex.Group");
            pwrs::object::add_note(&group, "Key", row.key.to_string().into_ps()?)?;
            pwrs::object::add_note(&group, "Count", (row.count as i64).into_ps()?)?;
            for ((col, held), name) in self.table.columns().iter().zip(&row.values).zip(&self.properties) {
                let value = match held {
                    Some(values) => aggregate(values, col, method, clock)?,
                    None => PsObject::default(),
                };
                pwrs::object::add_note(&group, name, value)?;
            }
            ps.write_object(&group)?;
        }
        Ok(())
    }
}
