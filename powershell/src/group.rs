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

use std::collections::HashMap;

use pwrs::prelude::*;

use crate::atoms::TrexLibrary;
use crate::common::{arg_err, duration, number, repeating, typed_value};
use crate::matching::{Scanning, files_of};
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

/// How a percentile falling between two observed values is decided.
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

/// One aggregate column: what it computes, the register it reads, the kind
/// that register binds, and the property it is written as.
struct Column {
    agg: trex::typed::Agg,
    register: String,
    kind: trex::token::TokenKind,
    property: String,
}

/// The register a capture reference names: `size` or `${size}`.
fn register_of(spec: &str) -> PsResult<String> {
    let body = spec.trim();
    let body = match body.strip_prefix("${").and_then(|b| b.strip_suffix('}')) {
        Some(inner) => inner,
        None => body,
    };
    if body.is_empty() || body.contains(['$', '{', '}', ':', '|']) {
        return Err(arg_err(
            "TrexAggregate",
            format!("an aggregate reads one register, as `size` or `${{size}}`, not {spec:?}"),
        ));
    }
    Ok(body.to_string())
}

/// One column's aggregate over the values a key collected, as the .NET type
/// that holds it: the value itself where the aggregate picks one, a minimum,
/// a maximum or a percentile landing on an observed value, and the figure
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
/// read one register whose kind orders or adds, and -Percentile reads the
/// register -PercentileOf names. -Unique writes the keys alone.
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
/// Group-TrexMatch '\R:t' -Key all -Percentile 50, 95 -PercentileOf t -Path ./timing.log
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
    /// `100..200`. A match counts only where it lies wholly inside.
    #[param]
    pub lines: Option<PsObject>,
    /// What -Head, -Tail and -Lines count: a line when absent, or a unit
    /// Find-TrexRecord reads, such as `paragraph` or `block`.
    #[param]
    pub unit: Option<String>,
    /// Writes the distinct keys alone.
    #[param]
    pub unique: bool,
    /// The register whose values each group sums.
    #[param]
    pub sum: Option<String>,
    /// The register whose values each group averages.
    #[param]
    pub average: Option<String>,
    /// The register whose least value each group reports.
    #[param]
    pub minimum: Option<String>,
    /// The register whose greatest value each group reports.
    #[param]
    pub maximum: Option<String>,
    /// Percentiles to report, such as 50 and 95, of the register
    /// -PercentileOf names.
    #[param]
    pub percentile: Vec<u32>,
    /// The register the percentiles read.
    #[param]
    pub percentile_of: Option<String>,
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
    columns: Vec<Column>,
    counts: HashMap<String, u64>,
    gathered: HashMap<String, Vec<trex::typed::Collected>>,
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

    fn columns_of(&self, sc: &Scanning) -> PsResult<Vec<Column>> {
        let kinds = sc.capture_kinds();
        let mut asked: Vec<(trex::typed::Agg, &str, String)> = Vec::new();
        for (agg, spec, property) in [
            (trex::typed::Agg::Sum, &self.sum, "Sum"),
            (trex::typed::Agg::Avg, &self.average, "Average"),
            (trex::typed::Agg::Min, &self.minimum, "Minimum"),
            (trex::typed::Agg::Max, &self.maximum, "Maximum"),
        ] {
            if let Some(spec) = spec {
                asked.push((agg, spec.as_str(), property.to_string()));
            }
        }
        if !self.percentile.is_empty() {
            let Some(spec) = &self.percentile_of else {
                return Err(arg_err("TrexAggregate", "-Percentile needs -PercentileOf to name the register it reads"));
            };
            for &p in &self.percentile {
                if p > 100 {
                    return Err(arg_err("TrexAggregate", format!("a percentile is 0 to 100, not {p}")));
                }
                asked.push((trex::typed::Agg::Pct(p), spec.as_str(), format!("P{p}")));
            }
        }
        let method = self.method();
        let mut columns = Vec::with_capacity(asked.len());
        for (agg, spec, property) in asked {
            let register = register_of(spec)?;
            let Some((_, found)) = kinds.iter().find(|(n, _)| *n == register) else {
                return Err(arg_err("TrexAggregate", format!("the pattern binds no register named {register}")));
            };
            let Some(kind) = *found else {
                return Err(arg_err(
                    "TrexAggregate",
                    format!(
                        "{register} binds a run, a repetition or an alternation, or two patterns of the set bind it on different kinds, so its kind varies between matches"
                    ),
                ));
            };
            if !agg.admits(kind) {
                return Err(arg_err(
                    "TrexAggregate",
                    format!("{property} of {register}, which binds a {}, is not defined for that kind", kind.name()),
                ));
            }
            if method == PercentileMethod::Linear
                && matches!(agg, trex::typed::Agg::Pct(_))
                && trex::typed::aggregable(kind) != trex::typed::Aggregable::Numeric
            {
                return Err(arg_err(
                    "TrexAggregate",
                    format!("a linear percentile cannot interpolate a {}; use Nearest, Lower or Hybrid", kind.name()),
                ));
            }
            columns.push(Column { agg, register, kind, property });
        }
        Ok(columns)
    }

    /// Whether the key reads where a match stands, its line, its column or
    /// its offsets, which a tail counts what is ahead of it for.
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
        for (member, m) in &found {
            let (line, col) = match &index {
                Some(ix) => ix.line_col(bytes, m.start),
                None => (0, 0),
            };
            let name = sc.name(*member);
            let pattern = if member.is_some() { Some(name.as_str()) } else { None };
            let place = trex::ReportAt { path, line, col, base: read.byte_base, pattern, rule: None };
            let key = template.render_report(m, bytes, &place);
            *self.counts.entry(key.clone()).or_insert(0) += 1;
            if self.columns.is_empty() {
                continue;
            }
            let slot =
                self.gathered.entry(key).or_insert_with(|| vec![trex::typed::Collected::default(); self.columns.len()]);
            // A match that does not bind the register adds no value to its
            // column, which is not the same as adding a zero.
            for (i, col) in self.columns.iter().enumerate() {
                if let Some(bound) = m.group(&col.register, bytes)
                    && let Some(v) = trex::typed::value_of(col.kind, &String::from_utf8_lossy(bound))
                {
                    slot[i].push(v);
                }
            }
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
        self.columns = self.columns_of(&sc)?;
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
        for source in files_of(ps, &paths, literal, &opts)? {
            let trex::files::Source::File(path) = source else {
                continue;
            };
            match self.part.read(&path, false, placed) {
                Ok(Some(read)) => self.tally(&path.display().to_string(), &read)?,
                Ok(None) => {}
                Err(e) => ps.write_error(&e)?,
            }
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        let mut table: Vec<(String, u64)> = self.counts.drain().collect();
        let order = match self.sort_by {
            Some(o) => o,
            None => GroupOrder::Count,
        };
        match order {
            GroupOrder::Count => table.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0))),
            GroupOrder::Key => table.sort_by(|a, b| a.0.cmp(&b.0)),
        }
        if let Some(n) = self.max_count {
            table.truncate(n as usize);
        }
        if self.unique {
            for (key, _) in table {
                ps.write(key)?;
            }
            return Ok(());
        }
        let method = self.method().trex();
        let clock = trex::Clock::current();
        for (key, count) in table {
            let row = pwrs::object::new_psobject("Trex.Group");
            pwrs::object::add_note(&row, "Key", key.clone().into_ps()?)?;
            pwrs::object::add_note(&row, "Count", (count as i64).into_ps()?)?;
            let held = self.gathered.get(&key);
            for (i, col) in self.columns.iter().enumerate() {
                let value = match held {
                    Some(values) => aggregate(&values[i], col, method, clock)?,
                    None => PsObject::default(),
                };
                pwrs::object::add_note(&row, &col.property, value)?;
            }
            ps.write_object(&row)?;
        }
        Ok(())
    }
}
