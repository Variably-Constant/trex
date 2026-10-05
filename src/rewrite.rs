//! Rewrite and transform: match, render a replacement template per
//! match, splice the renders into the input.
//!
//! A rewrite is `match -> render -> splice`. The match stage is the same
//! structure-aware scan as everywhere else, so a rewrite expresses
//! transforms a byte regex cannot reach safely: it substitutes spans
//! found by typed-token, balanced, and bound matching, not by a greedy
//! byte search. Renaming a balanced tag, reformatting a typed atom, or
//! reordering a captured argument list are all just templates over the
//! spans the scan finds and the registers they bound.
//!
//! ## Template syntax
//!
//! - `${name}` renders a named capture's value; for a register bound under
//!   a repetition, its last binding. `${name[i]}` renders the i-th binding,
//!   and `${name[*]}` every binding joined with a comma.
//! - `${0}` renders the whole matched span.
//! - `${1}`, `${2}` and so on render a capture by position: `${1}` is the
//!   first name the pattern binds. TREX's `(...)` is a logical group and
//!   binds nothing, so what is numbered is the bindings in written order,
//!   rather than a second anonymous kind of capture alongside `:name`.
//! - `${name:acc}` applies an accessor, and `${name:a|b}` chains them.
//!   Accessors are the transforms `upper` / `lower`, the slices `trim` /
//!   `firstN` / `lastN`, and the typed sub-field extractors that slice a
//!   captured atom by its known structure:
//!   `octetN[-M]` (IPv4) / `groupN[-M]` (IPv6) on an `\I`, `scheme` / `host` /
//!   `port` / `path` / `query` on a `\U`, `user` / `domain` on an `\E`,
//!   `major` / `minor` / `patch` on a `\V`, `year` / `month` / `day` / `hour` /
//!   `minute` / `second` on a `\T`, and `dir` / `name` / `ext` on a `\L`.
//! - `$$` is a literal dollar sign; `\n`, `\t` and `\\` are a newline, a tab
//!   and a backslash; every other byte is literal, and a backslash before
//!   anything else is an error.
//! - `${path}`, `${line}`, `${col}`, `${start}` and `${end}`, in a report's
//!   template ([`Template::parse_report`]), are the match's position: the
//!   input's name, its line and column from one, and its offsets, counted in
//!   the unit the surface's strings are indexed by ([`ReportAt::offsets`]).
//!   They take accessors too (`${path:name}`).
//! - `${@axis}` is a reading `--explain` computes for the match: the kinds it
//!   spans (`${@kind}`), the guard each guarded kind passed (`${@guard}`),
//!   the rung that answered (`${@route}`), and every axis the pattern read -
//!   `${@magnitude}`, `${@baseline}`, `${@spectral}`, `${@echo}`,
//!   `${@order}`, `${@template}`, `${@nesting}`, `${@seam}`,
//!   `${@ambiguous}`, `${@super}`, `${@phase}`, `${@field}`,
//!   `${@join}`. A dot names one value of the reading rather than the whole
//!   sentence: `${@spectral.entropy}`, `${@template.rarity}`,
//!   `${@super.role}`, and the two depths the axes tell apart,
//!   `${@nesting.depth}` for bracket nesting and `${@super.depth}` for
//!   the enclosing supertoken. An axis reads at every token the match spans, so
//!   the reference renders them joined and an index picks one:
//!   `${@magnitude[0]}`. Accessors apply as they do elsewhere. An axis the
//!   pattern never read renders empty; an axis that does not exist is a
//!   parse error. [`Template::reads_explanation`] is what tells a caller to
//!   build the explainer, so a template naming none costs nothing for it.
//!
//! A template is validated against the pattern's capture names when it
//! is parsed, so a reference to a name the pattern never binds is an
//! error rather than a silent empty render. The `@` keeps the axes out of
//! that namespace: a pattern may bind `:kind`, and `${kind}` is its
//! register under every pattern, whether or not the pattern also reads an
//! axis by that name.
//!
//! ## Redaction
//!
//! [`redactions`] masks each match and leaves the fields a [`Keep`] names
//! unmasked, so the slicing accessors have a second reading: a kept
//! field is the byte range an accessor locates rather than the text it
//! renders. Every slicing accessor renders exactly the bytes it locates, so
//! `${card:last4}` in a template and `card:last4` in a redaction are the
//! same four characters.
//!
//! ## Callback
//!
//! [`rewrite_with`] and [`rewrite_n_with`] take a closure in a template's
//! place: each match arrives as a [`Matched`], and [`Matched::get`] reads
//! the reference a template writes inside `${...}` - `0`, a register, a
//! typed slice such as `e:domain` - through the same [`Reference`], so a
//! closure and a template read a match the same way.

use std::borrow::Cow;
use std::ops::Range;

use crate::ast::Pattern;
use crate::engine::{Match, Span, captures, captures_with_lists, scan};
use crate::explain::Explanation;
use crate::gpu::{Backend, BackendUsed, Engine, Ran, scan_engine, scan_gpu, scan_with_backend};
use crate::token::TokenKind;

/// One step applied to a captured value before it is rendered: a pure
/// transform, or a typed sub-field extractor that decomposes a captured atom by
/// its known structure - an IP into octets or IPv6 groups, a URL into host and
/// path, an email into user and domain, a version into major/minor/patch, a
/// timestamp into date fields, a path into dir/name/ext. Because TREX captured
/// a *typed* atom, the sub-field is a slice of a known shape, not a second
/// match. Steps chain left to right with `|`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Accessor {
    Upper,
    Lower,
    Trim,
    /// The first `n` characters.
    First(usize),
    /// The last `n` characters.
    Last(usize),
    /// IPv4 octets `a..=b` (1-based inclusive), rejoined with `.`.
    Octet(usize, usize),
    /// IPv6 groups `a..=b` (1-based inclusive), rejoined with `:`.
    Group(usize, usize),
    UrlScheme,
    UrlHost,
    UrlPort,
    UrlPath,
    UrlQuery,
    EmailUser,
    EmailDomain,
    VerMajor,
    VerMinor,
    VerPatch,
    /// The i-th numeric field of a timestamp: year, month, day, hour, ...
    TsField(usize),
    PathDir,
    PathName,
    PathExt,
    /// A quantity's number as written, sign included.
    QtyValue,
    /// A quantity's unit symbol as written.
    QtyUnit,
}

/// A byte range within the text an accessor read.
type ByteRange = Range<usize>;

impl Accessor {
    /// Whether the accessor reads a slice of its input rather than
    /// transforming it, so its value has a place in the original text.
    fn slices(&self) -> bool {
        !matches!(self, Accessor::Upper | Accessor::Lower)
    }

    /// The byte range of the value within `v`: `None` for a transform, and
    /// where the value is absent or empty. Every slicing accessor renders
    /// exactly the bytes it locates, so [`Self::apply`] reads through here
    /// and a kept field is the same bytes a template would render.
    fn locate(&self, v: &str) -> Option<ByteRange> {
        let r = match self {
            Accessor::Upper | Accessor::Lower => return None,
            Accessor::Trim => {
                let start = v.len() - v.trim_start().len();
                start..start + v.trim().len()
            }
            Accessor::First(n) => 0..v.char_indices().nth(*n).map_or(v.len(), |(i, _)| i),
            Accessor::Last(n) => {
                v.char_indices().rev().nth(n.checked_sub(1)?).map_or(0, |(i, _)| i)..v.len()
            }
            Accessor::Octet(a, b) => parts_range(v, '.', *a, *b)?,
            Accessor::Group(a, b) => parts_range(v, ':', *a, *b)?,
            Accessor::UrlScheme => url_range(v, UrlPart::Scheme),
            Accessor::UrlHost => url_range(v, UrlPart::Host),
            Accessor::UrlPort => url_range(v, UrlPart::Port),
            Accessor::UrlPath => url_range(v, UrlPart::Path),
            Accessor::UrlQuery => url_range(v, UrlPart::Query),
            Accessor::EmailUser => 0..v.find('@')?,
            Accessor::EmailDomain => v.find('@')? + 1..v.len(),
            Accessor::VerMajor => ver_range(v, 0)?,
            Accessor::VerMinor => ver_range(v, 1)?,
            Accessor::VerPatch => ver_range(v, 2)?,
            Accessor::TsField(i) => ts_range(v, *i)?,
            Accessor::PathDir => path_range(v, PathPart::Dir),
            Accessor::PathName => path_range(v, PathPart::Name),
            Accessor::PathExt => path_range(v, PathPart::Ext),
            Accessor::QtyValue => 0..crate::quantity::split(v)?.0.len(),
            Accessor::QtyUnit => v.len() - crate::quantity::split(v)?.1.len()..v.len(),
        };
        (!r.is_empty()).then_some(r)
    }

    fn apply(&self, v: &str) -> String {
        match self {
            Accessor::Upper => v.to_uppercase(),
            Accessor::Lower => v.to_lowercase(),
            _ => self.locate(v).map_or_else(String::new, |r| v[r].to_string()),
        }
    }
}

/// The accessors a template writes after a capture of a `kind` token to read
/// exactly `part` of the token's text `text`: the typed accessors the kind
/// has first, then `firstN` where the part runs from the text's start or
/// `lastN` where it runs to the end. Empty where no accessor reads exactly
/// that part.
pub(crate) fn accessors_locating(kind: TokenKind, text: &str, part: Range<usize>) -> Vec<String> {
    let mut typed: Vec<(String, Accessor)> = Vec::new();
    let named = |names: &[(&str, Accessor)]| -> Vec<(String, Accessor)> {
        names.iter().map(|(n, a)| ((*n).to_string(), a.clone())).collect()
    };
    match kind {
        TokenKind::Url => typed = named(&[
            ("scheme", Accessor::UrlScheme),
            ("host", Accessor::UrlHost),
            ("port", Accessor::UrlPort),
            ("path", Accessor::UrlPath),
            ("query", Accessor::UrlQuery),
        ]),
        TokenKind::Email => typed = named(&[("user", Accessor::EmailUser), ("domain", Accessor::EmailDomain)]),
        TokenKind::Version => typed = named(&[
            ("major", Accessor::VerMajor),
            ("minor", Accessor::VerMinor),
            ("patch", Accessor::VerPatch),
        ]),
        TokenKind::Timestamp => {
            for (i, n) in ["year", "month", "day", "hour", "minute", "second"].into_iter().enumerate() {
                typed.push((n.to_string(), Accessor::TsField(i)));
            }
        }
        TokenKind::Path => typed = named(&[
            ("dir", Accessor::PathDir),
            ("name", Accessor::PathName),
            ("ext", Accessor::PathExt),
        ]),
        TokenKind::Quantity | TokenKind::ByteSize | TokenKind::Duration | TokenKind::Percent => {
            typed = named(&[("value", Accessor::QtyValue), ("unit", Accessor::QtyUnit)]);
        }
        TokenKind::Ip => {
            for a in 1..=8 {
                for b in a..=8 {
                    let span = if a == b { a.to_string() } else { format!("{a}-{b}") };
                    if b <= 4 {
                        typed.push((format!("octet{span}"), Accessor::Octet(a, b)));
                    }
                    typed.push((format!("group{span}"), Accessor::Group(a, b)));
                }
            }
        }
        _ => {}
    }
    let mut found: Vec<String> =
        typed.into_iter().filter(|(_, a)| a.locate(text) == Some(part.clone())).map(|(n, _)| n).collect();
    let chars = text.get(part.clone()).map_or(0, |p| p.chars().count());
    if chars > 0 && part.start == 0 && part.end < text.len() {
        found.push(format!("first{chars}"));
    }
    if chars > 0 && part.end == text.len() && part.start > 0 {
        found.push(format!("last{chars}"));
    }
    found
}

/// What the accessor pipeline `accs`, as a template writes it after the
/// capture's name and `:`, renders from `text`.
///
/// # Errors
///
/// `accs` names an accessor a template does not know.
pub(crate) fn apply_named(accs: &str, text: &str) -> Result<String, String> {
    match parse_accessors(accs, 0) {
        Ok(parsed) => Ok(apply_all(&parsed, text)),
        Err(e) => Err(e.msg),
    }
}

/// Whether a report template reads `${name}` as a field of the match's
/// report (its file, place, pattern or rule) rather than as a capture, so a
/// capture of that name is reached only by its position.
pub(crate) fn is_report_field(name: &str) -> bool {
    ReportField::parse(name).is_some()
}

/// Apply an accessor pipeline to a value, left to right.
fn apply_all(accs: &[Accessor], v: &str) -> String {
    let mut s = v.to_string();
    for a in accs {
        s = a.apply(&s);
    }
    s
}

/// The byte range of an accessor pipeline's value within `v`, each accessor
/// reading the slice the one before it located; `None` where any accessor
/// transforms rather than slices, or finds nothing.
fn locate_all(accs: &[Accessor], v: &str) -> Option<ByteRange> {
    let mut r = 0..v.len();
    for a in accs {
        let sub = a.locate(&v[r.clone()])?;
        r = r.start + sub.start..r.start + sub.end;
    }
    (!r.is_empty()).then_some(r)
}

/// The byte range of the 1-based inclusive run of parts `a..=b` of `v`
/// split on `sep`, the separators between them included; `None` when the
/// range is out of bounds.
fn parts_range(v: &str, sep: char, a: usize, b: usize) -> Option<ByteRange> {
    if a == 0 || b < a {
        return None;
    }
    let mut parts: Vec<ByteRange> = Vec::new();
    let mut start = 0;
    for (i, _) in v.match_indices(sep) {
        parts.push(start..i);
        start = i + sep.len_utf8();
    }
    parts.push(start..v.len());
    if b > parts.len() {
        return None;
    }
    Some(parts[a - 1].start..parts[b - 1].end)
}

/// The part of a URL an accessor or a typed predicate reads.
pub(crate) enum UrlPart {
    Scheme,
    Host,
    Port,
    Path,
    Query,
}

/// The byte range of one part of `scheme://host[:port][/path][?query]`
/// within `v`, empty where the part is absent.
fn url_range(v: &str, part: UrlPart) -> ByteRange {
    let (scheme_end, rest_start) = match v.find("://") {
        Some(i) => (i, i + 3),
        None => (0, 0),
    };
    let rest = &v[rest_start..];
    let auth_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    let host_end = match authority.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) => h.len(),
        _ => auth_end,
    };
    let after = &rest[auth_end..];
    let path_end = after.find('?').unwrap_or(after.len());
    let at = |i: usize| rest_start + i;
    match part {
        UrlPart::Scheme => 0..scheme_end,
        UrlPart::Host => at(0)..at(host_end),
        UrlPart::Port if host_end < auth_end => at(host_end + 1)..at(auth_end),
        UrlPart::Path => at(auth_end)..at(auth_end + path_end),
        UrlPart::Query if path_end < after.len() => at(auth_end + path_end + 1)..v.len(),
        UrlPart::Port | UrlPart::Query => 0..0,
    }
}

/// Decompose `scheme://host[:port][/path][?query]` into one part.
pub(crate) fn url_part(v: &str, part: UrlPart) -> String {
    v[url_range(v, part)].to_string()
}

/// The byte range of the i-th dotted field of `MAJOR.MINOR.PATCH`, before
/// any `-pre` or `+build`.
fn ver_range(v: &str, i: usize) -> Option<ByteRange> {
    let core_end = v.find(['-', '+']).unwrap_or(v.len());
    parts_range(&v[..core_end], '.', i + 1, i + 1)
}

/// The byte ranges of the runs of digits in `v`, in order.
fn digit_runs(v: &str) -> Vec<ByteRange> {
    let b = v.as_bytes();
    let mut runs = Vec::new();
    let mut j = 0;
    while j < b.len() {
        if !b[j].is_ascii_digit() {
            j += 1;
            continue;
        }
        let start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        runs.push(start..j);
    }
    runs
}

/// The byte range of one calendar field of a timestamp - year, month, day,
/// hour, minute, second, in that order - where the form the text is written
/// in puts it.
///
/// Read from the form rather than by counting digit runs, because the runs
/// are in a different order in every form a log writes: `15/09/2026` opens
/// with the day, Apache's `15/Sep/2026:10:00:00` writes the year third, and
/// a bare clock writes no date at all, so its first run is the hour. A field
/// the text does not write in digits - syslog's named month, the year it
/// leaves out - has no range and renders empty.
fn ts_range(v: &str, i: usize) -> Option<ByteRange> {
    let b = v.as_bytes();
    let runs = digit_runs(v);
    // Which run holds the year, the month and the day, and which run after
    // them holds the clock's hour.
    let (date, clock) = if crate::typed::month_abbrev(b).is_some() && b.get(3) == Some(&b' ') {
        ((None, None, Some(0)), 1)
    } else if b.get(2) == Some(&b'/')
        && b.get(3..).is_some_and(|rest| crate::typed::month_abbrev(rest).is_some())
    {
        ((Some(1), None, Some(0)), 2)
    } else if let Some(d) = crate::typed::slash_date(b, 0) {
        let (year, month, day) = d.runs;
        ((Some(year), Some(month), Some(day)), 3)
    } else if runs.first().is_some_and(|r| r.len() == 4) && b.get(4) == Some(&b'-') {
        ((Some(0), Some(1), Some(2)), 3)
    } else {
        ((None, None, None), 0)
    };
    let run = match i {
        0 => date.0?,
        1 => date.1?,
        2 => date.2?,
        _ => clock + i - 3,
    };
    runs.get(run).cloned()
}

/// The part of a filesystem path an accessor or a typed predicate reads.
pub(crate) enum PathPart {
    Dir,
    Name,
    Ext,
}

/// The byte range of one part of a filesystem path within `v`: its
/// directory, basename, or extension, empty where the part is absent.
fn path_range(v: &str, part: PathPart) -> ByteRange {
    let sep = if v.contains('\\') { '\\' } else { '/' };
    let name_start = v.rfind(sep).map_or(0, |i| i + 1);
    match part {
        PathPart::Dir => 0..name_start.saturating_sub(1),
        PathPart::Name => name_start..v.len(),
        PathPart::Ext => match v[name_start..].rfind('.') {
            Some(dot) => name_start + dot + 1..v.len(),
            None => 0..0,
        },
    }
}

/// Decompose a filesystem path into its directory, basename, or extension.
pub(crate) fn path_field(v: &str, part: PathPart) -> String {
    v[path_range(v, part)].to_string()
}

/// One piece of a parsed template.
#[derive(Debug)]
enum Part {
    Literal(String),
    /// `${0}`: the whole matched span, with an optional accessor pipeline.
    WholeMatch(Vec<Accessor>),
    /// `${name}`: a named capture, with an optional accessor pipeline.
    Capture(String, Vec<Accessor>),
    /// `${name[i]}`: the i-th binding of a register bound under a
    /// repetition, counted from zero, with an optional accessor pipeline.
    Item(String, usize, Vec<Accessor>),
    /// `${name[*]}`: every binding of a register, each through the accessor
    /// pipeline, joined with a comma.
    All(String, Vec<Accessor>),
    /// `${path}` and the rest: the match's position, in a report's
    /// template, with an optional accessor pipeline.
    Where(ReportField, Vec<Accessor>),
    /// `${@axis}`, `${@axis.piece}`, `${@axis[i]}`: a reading the explainer
    /// computes at the tokens the match spans, with an optional accessor
    /// pipeline.
    Explain(ExplainRef, Vec<Accessor>),
}

/// What a `${@...}` reference names: an axis the explainer reads, the piece
/// of its reading the reference asks for, and which token's reading where it
/// carries an index.
///
/// The `@` is what keeps this apart from a register: a template is validated
/// against the pattern's capture names, so a bare `${kind}` would name the
/// register a pattern writing `:kind` binds, and the same template text
/// would mean one thing under one pattern and another under the next with
/// nothing reporting the difference. `@` already reads as "an axis" in the
/// pattern language, where `@echo`, `@seam` and `@k` are axes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplainRef {
    /// The axis, or one of `kind`, `guard` and `route`, which the
    /// explanation carries whole rather than at each token.
    pub axis: String,
    /// The value of that axis's reading the reference names, where it names
    /// one rather than the whole sentence.
    pub piece: Option<String>,
    /// Which token's reading, counted from zero over the tokens the match
    /// spans. Without one the reference renders every reading, joined.
    pub at: Option<usize>,
}

/// A match's position, as a report's template writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportField {
    /// The input's name: its path, `-` for the standard input, nothing for
    /// an inline string.
    Path,
    /// The line the match begins on, from one.
    Line,
    /// The column the match begins at, from one.
    Col,
    /// The offset the match begins at, in the unit [`ReportAt::offsets`]
    /// counts.
    Start,
    /// The offset just past it.
    End,
    /// The member of a set the match belongs to, by name, where the scan
    /// ran a set; nothing otherwise.
    Pattern,
    /// The rule a finding is of, by name, where the scan ran rules; nothing
    /// otherwise.
    Rule,
    /// The finding's severity.
    Severity,
    /// The finding's message, rendered.
    Message,
    /// The finding's fix, rendered; nothing where the rule has none.
    Fix,
}

impl ReportField {
    fn parse(name: &str) -> Option<ReportField> {
        Some(match name {
            "path" => ReportField::Path,
            "line" => ReportField::Line,
            "col" => ReportField::Col,
            "start" => ReportField::Start,
            "end" => ReportField::End,
            "pattern" => ReportField::Pattern,
            "rule" => ReportField::Rule,
            "severity" => ReportField::Severity,
            "message" => ReportField::Message,
            "fix" => ReportField::Fix,
            _ => return None,
        })
    }
}

/// The rule a finding is of, for a report's template to write: its name,
/// its severity, and the message and fix rendered for the finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportRule<'a> {
    pub name: &'a str,
    pub severity: &'a str,
    pub message: &'a str,
    pub fix: &'a str,
}

/// One match's position, for a report's template to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportAt<'a> {
    /// The input's name.
    pub path: &'a str,
    /// The line the match begins on, from one.
    pub line: usize,
    /// The column the match begins at, from one.
    pub col: usize,
    /// Where the match starts and ends in the input it was found in, counted
    /// in the unit the surface's strings are indexed by: bytes on the command
    /// line, code points of a Python `str`, UTF-16 code units in PowerShell.
    /// `None` where that place was not counted, which only a template
    /// writing no offset is rendered over.
    pub offsets: Option<(usize, usize)>,
    /// The member of a set the match belongs to, where the scan ran one.
    pub pattern: Option<&'a str>,
    /// The rule the match is a finding of, where the scan ran rules.
    pub rule: Option<ReportRule<'a>>,
}

impl ReportAt<'_> {
    /// The match's position, for a template writing an offset.
    ///
    /// # Panics
    ///
    /// The place was not counted: a report whose template writes an offset
    /// asks for it, so this is a template writing an offset its report chose
    /// not to count.
    fn placed(&self) -> (usize, usize) {
        self.offsets.expect("a match is placed wherever a template writes its offsets")
    }

    fn value(&self, field: ReportField) -> String {
        match field {
            ReportField::Path => self.path.to_string(),
            ReportField::Line => self.line.to_string(),
            ReportField::Col => self.col.to_string(),
            ReportField::Start => self.placed().0.to_string(),
            ReportField::End => self.placed().1.to_string(),
            ReportField::Pattern => self.pattern.unwrap_or("").to_string(),
            ReportField::Rule => self.rule.map_or("", |r| r.name).to_string(),
            ReportField::Severity => self.rule.map_or("", |r| r.severity).to_string(),
            ReportField::Message => self.rule.map_or("", |r| r.message).to_string(),
            ReportField::Fix => self.rule.map_or("", |r| r.fix).to_string(),
        }
    }
}

/// A parsed replacement template.
#[derive(Debug)]
pub struct Template {
    parts: Vec<Part>,
}

/// A template parse failure: the byte offset into the template and a
/// reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateError {
    /// Byte offset into the template string where parsing stopped.
    pub pos: usize,
    /// Human-readable reason.
    pub msg: String,
}

impl Template {
    /// Parse `src` into a template, validating every `${name}` against
    /// `bound` (the pattern's capture names). `${0}` is always valid.
    ///
    /// # Errors
    ///
    /// Returns a [`TemplateError`] for an unterminated `${...}`, an
    /// unknown transform, or a capture name the pattern does not bind.
    pub fn parse(src: &str, bound: &[String]) -> Result<Template, TemplateError> {
        Self::parse_full(src, bound, false)
    }

    /// [`Self::parse`] for a report's template, which may also write the
    /// match's position - `${path}`, `${line}`, `${col}`, `${start}`,
    /// `${end}` - and the readings `--explain` computes for it,
    /// `${@axis}` and `${@axis.piece}`. Both take accessors.
    ///
    /// A template written for a rewrite takes neither, because the bytes
    /// spliced in have no place and no explanation to read.
    ///
    /// # Errors
    ///
    /// As [`Self::parse`], and an axis or a piece of one that the explainer
    /// does not read.
    pub fn parse_report(src: &str, bound: &[String]) -> Result<Template, TemplateError> {
        Self::parse_full(src, bound, true)
    }

    fn parse_full(src: &str, bound: &[String], report: bool) -> Result<Template, TemplateError> {
        let b = src.as_bytes();
        let mut parts: Vec<Part> = Vec::new();
        let mut lit = String::new();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'$' if i + 1 < b.len() && b[i + 1] == b'$' => {
                    lit.push('$');
                    i += 2;
                }
                b'$' if i + 1 < b.len() && b[i + 1] == b'{' => {
                    if !lit.is_empty() {
                        parts.push(Part::Literal(std::mem::take(&mut lit)));
                    }
                    let start = i;
                    let close = b[i + 2..]
                        .iter()
                        .position(|&c| c == b'}')
                        .map(|p| i + 2 + p)
                        .ok_or(TemplateError { pos: start, msg: "unterminated ${...}".into() })?;
                    let body = &src[i + 2..close];
                    let (name, accs) = match body.split_once(':') {
                        Some((n, a)) => (n, a),
                        None => (body, ""),
                    };
                    match ReportField::parse(name) {
                        Some(field) if report => {
                            let accs = if accs.is_empty() { Vec::new() } else { parse_accessors(accs, start)? };
                            parts.push(Part::Where(field, accs));
                        }
                        _ => parts.push(parse_ref(body, start, bound, report)?),
                    }
                    i = close + 1;
                }
                b'\\' => {
                    let escaped = match b.get(i + 1) {
                        Some(b'n') => '\n',
                        Some(b't') => '\t',
                        Some(b'\\') => '\\',
                        Some(&other) => {
                            return Err(TemplateError {
                                pos: i,
                                msg: format!(
                                    "unknown escape \\{}; a template knows \\n, \\t and \\\\",
                                    other as char
                                ),
                            });
                        }
                        None => {
                            return Err(TemplateError {
                                pos: i,
                                msg: "a backslash ends the template; write \\\\ for a backslash".into(),
                            });
                        }
                    };
                    lit.push(escaped);
                    i += 2;
                }
                c => {
                    lit.push(c as char);
                    i += 1;
                }
            }
        }
        if !lit.is_empty() {
            parts.push(Part::Literal(lit));
        }
        Ok(Template { parts })
    }

    /// Render this template for one match over `input`.
    ///
    /// A rewrite splices the render into the match's place; a caller
    /// grouping matches by a key renders the same way and keeps the string.
    #[must_use]
    pub fn render<M: Spanned>(&self, m: &M, input: &[u8]) -> String {
        self.render_at(m, input, None, None)
    }

    /// Render a report's template for one match, `at` being the match's
    /// position.
    #[must_use]
    pub fn render_report<M: Spanned>(&self, m: &M, input: &[u8], at: &ReportAt<'_>) -> String {
        self.render_at(m, input, Some(at), None)
    }

    /// Render this template for one match, `why` being the explanation of
    /// that match, which every `${@...}` part reads.
    ///
    /// A template [`Self::reads_explanation`] rejects renders the same bytes
    /// through [`Self::render`], so a caller builds the explainer only for
    /// the templates that name an axis.
    #[must_use]
    pub fn render_explained<M: Spanned>(
        &self,
        m: &M,
        input: &[u8],
        at: Option<&ReportAt<'_>>,
        why: &Explanation,
    ) -> String {
        self.render_at(m, input, at, Some(why))
    }

    fn render_at<M: Spanned>(
        &self,
        m: &M,
        input: &[u8],
        at: Option<&ReportAt<'_>>,
        why: Option<&Explanation>,
    ) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Literal(s) => out.push_str(s),
                // A place is written only where a report supplies one; a
                // template parsed for a rewrite never holds a place.
                Part::Where(field, accs) => {
                    let value = at.map_or_else(String::new, |a| a.value(*field));
                    out.push_str(&apply_all(accs, &value));
                }
                // An axis is read only where the caller supplies an
                // explanation, which it builds when the template names one.
                Part::Explain(named, accs) => {
                    let value = why.map_or_else(String::new, |e| {
                        e.field(&named.axis, named.piece.as_deref(), named.at)
                    });
                    out.push_str(&apply_all(accs, &value));
                }
                Part::WholeMatch(accs) => {
                    let whole = String::from_utf8_lossy(&input[m.start()..m.end()]);
                    out.push_str(&apply_all(accs, &whole));
                }
                Part::Capture(name, accs) => {
                    let bytes = m
                        .names()
                        .iter()
                        .position(|k| k == name)
                        .and_then(|i| m.captures().get(i))
                        .map_or(&[] as &[u8], |s| &input[s.range()]);
                    let v = String::from_utf8_lossy(bytes);
                    out.push_str(&apply_all(accs, &v));
                }
                Part::Item(name, index, accs) => {
                    let bytes = m
                        .history(name)
                        .and_then(|all| all.get(*index))
                        .map_or(&[] as &[u8], |s| &input[s.range()]);
                    let v = String::from_utf8_lossy(bytes);
                    out.push_str(&apply_all(accs, &v));
                }
                Part::All(name, accs) => out.push_str(&every_binding(m, input, name, accs)),
            }
        }
        out
    }

    /// Whether a part reads a named capture, so the matches rendered must
    /// carry their registers.
    fn reads_captures(&self) -> bool {
        self.parts.iter().any(|p| matches!(p, Part::Capture(..) | Part::Item(..) | Part::All(..)))
    }

    /// Whether a part reads one binding of a register bound under a
    /// repetition by its index, so the matches rendered must carry every
    /// binding, as [`captures_with_lists`] resolves them.
    #[must_use]
    pub fn reads_lists(&self) -> bool {
        self.parts.iter().any(|p| matches!(p, Part::Item(..) | Part::All(..)))
    }

    /// Whether a part writes a match's line or column, so a
    /// report rendering it must count them.
    #[must_use]
    pub fn reads_place(&self) -> bool {
        self.parts.iter().any(|p| matches!(p, Part::Where(ReportField::Line | ReportField::Col, _)))
    }

    /// Whether a part writes the offset a match starts or ends at, so a
    /// report rendering it over a window must place the window.
    #[must_use]
    pub fn reads_offsets(&self) -> bool {
        self.parts.iter().any(|p| matches!(p, Part::Where(ReportField::Start | ReportField::End, _)))
    }

    /// Whether a part names an axis, so the caller must build an explainer
    /// over the input and render through [`Self::render_explained`].
    ///
    /// This is what keeps a template that names none costing what it always
    /// did: the analyses behind the axes are built once per input, and only
    /// where this answers true.
    #[must_use]
    pub fn reads_explanation(&self) -> bool {
        self.parts.iter().any(|p| matches!(p, Part::Explain(..)))
    }

    /// Whether every match renders the same bytes: no part reads the match or a
    /// register it bound, so the render is the template's literals and nothing
    /// else.
    ///
    /// Stricter than [`Self::reads_captures`], which asks whether the matches
    /// must carry their registers: `${0}` carries none and still renders
    /// differently at every match.
    fn renders_one_string(&self) -> bool {
        self.parts.iter().all(|p| matches!(p, Part::Literal(_)))
    }

    /// The bytes this template renders at every match, for a template
    /// [`Self::renders_one_string`] accepts.
    fn one_string(&self) -> String {
        self.parts
            .iter()
            .map(|p| match p {
                Part::Literal(s) => s.as_str(),
                Part::WholeMatch(_)
                | Part::Capture(..)
                | Part::Item(..)
                | Part::All(..)
                | Part::Where(..)
                | Part::Explain(..) => "",
            })
            .collect()
    }
}

/// The count `firstN` / `lastN` writes: a positive integer.
fn parse_count(r: &str, name: &str, pos: usize) -> Result<usize, TemplateError> {
    match r.parse::<usize>() {
        Ok(n) if n > 0 => Ok(n),
        Ok(_) => Err(TemplateError {
            pos,
            msg: format!("{name}0 names no characters; write {name}N with N at least 1"),
        }),
        Err(e) => Err(TemplateError {
            pos,
            msg: format!("{name}{r}: {e}; write {name}N, as in {name}4"),
        }),
    }
}

/// One field a redaction leaves readable: the whole match or a named
/// register, and the slicing accessors that locate the field within it.
/// Written as a reference is written inside `${...}`: `card:last4`,
/// `ip:octet1-2`, `email:domain`, `0:last4`.
#[derive(Clone, Debug)]
pub struct Keep {
    field: Field,
    accessors: Vec<Accessor>,
}

impl Keep {
    /// Parse a comma-separated list of kept fields against the pattern's
    /// capture names; an empty list keeps nothing.
    ///
    /// # Errors
    ///
    /// A reference the pattern does not bind, an unknown accessor, or a
    /// transforming accessor (`upper`, `lower`), whose value has no place in
    /// the original text.
    pub fn parse_list(src: &str, bound: &[String]) -> Result<Vec<Keep>, TemplateError> {
        let mut out = Vec::new();
        let mut pos = 0;
        for entry in src.split(',') {
            let at = pos + (entry.len() - entry.trim_start().len());
            pos += entry.len() + 1;
            let body = entry.trim();
            if body.is_empty() {
                continue;
            }
            let (field, accessors) = parse_field(body, at, bound)?;
            if let Some(transform) = accessors.iter().find(|a| !a.slices()) {
                let name = if *transform == Accessor::Upper { "upper" } else { "lower" };
                return Err(TemplateError {
                    pos: at,
                    msg: format!(
                        "`:{name}` transforms the text rather than slicing it, so it names nothing to keep in place"
                    ),
                });
            }
            out.push(Keep { field, accessors });
        }
        Ok(out)
    }

    /// The byte range of this field within `input` for one match: `None`
    /// where the match did not bind the register, the field is absent, the
    /// span is not UTF-8, whose offsets no character count reaches, or the
    /// field is every binding, which are at several ranges and not one.
    fn locate<M: Spanned>(&self, m: &M, input: &[u8]) -> Option<ByteRange> {
        let base = match &self.field {
            Field::All(_) => return None,
            Field::Whole => m.start()..m.end(),
            Field::Register(name) => {
                let i = m.names().iter().position(|k| k == name)?;
                m.captures().get(i)?.range()
            }
            Field::Item(name, index) => m.history(name)?.get(*index)?.range(),
        };
        let text = match String::from_utf8_lossy(&input[base.clone()]) {
            Cow::Borrowed(t) => t,
            Cow::Owned(_) => return None,
        };
        let r = locate_all(&self.accessors, text)?;
        Some(base.start + r.start..base.start + r.end)
    }
}

/// The digit every digit is masked to, and the letter every letter is.
///
/// Measured against the lexer rather than chosen for looks, which is what
/// decided the letter: `a` is both a letter and a hex digit, so one rule
/// covers both families. Masking to `x` keeps an email and a word reading as
/// themselves and breaks every hex-shaped kind - `#A3F2B1` becomes `#x0xxxx`,
/// which is a punctuation mark and a word rather than a color, and a uuid and
/// a mac fall apart into a dozen tokens each. With `a` the shapes hold:
/// hexcolor, uuid, mac, email, ip, timestamp, version and url all still lex
/// as their kind.
const MASKED_DIGIT: u8 = b'0';
const MASKED_LETTER: u8 = b'a';

/// The book a pseudonym mask keeps: for each kind, the values seen under it
/// in the order they were first seen.
///
/// One book for a whole run, so a value gives the same name in every input of
/// it - which is what lets a reader join the redacted copies of two files
/// that shared a value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pseudonyms {
    seen: Vec<(String, Vec<Vec<u8>>)>,
}

impl Pseudonyms {
    /// The pseudonym for `value`, minting one where it is new.
    fn name(&mut self, kind: &str, value: &[u8]) -> String {
        let values = match self.seen.iter().position(|(k, _)| k == kind) {
            Some(at) => &mut self.seen[at].1,
            None => {
                self.seen.push((kind.to_string(), Vec::new()));
                let last = self.seen.len() - 1;
                &mut self.seen[last].1
            }
        };
        let at = match values.iter().position(|v| v == value) {
            Some(at) => at,
            None => {
                values.push(value.to_vec());
                values.len() - 1
            }
        };
        // Numbered from one, in order of first sight, and the kind in
        // capitals. The whole name is one Word token to the lexer, which is
        // what carries the recurrence into the redacted copy: a name that lexed
        // as several tokens would lose exactly what this mask exists to keep.
        format!("{}_{}", kind.to_uppercase(), at + 1)
    }
}

/// What a redaction writes over the characters it removes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mask {
    /// One copy of the character for each character removed, so every
    /// offset and column after the span survives.
    PerChar(char),
    /// One copy of the token for each masked run, whatever the run's length.
    Token(String),
    /// Every letter and digit masked and every other byte kept, so the run
    /// keeps the shape its kind is recognized by and the redacted copy still
    /// lexes as the original did.
    Shape,
    /// Each distinct value replaced by a stable name per kind, so a value
    /// that recurred still recurs and the axes that read recurrence - echo,
    /// the joins, the templates - read the redacted copy as they read the
    /// original.
    Pseudonym(Pseudonyms),
}

impl Mask {
    /// A one-character string masks per character; `shape` and `pseudonym`
    /// name the two masks that read the run; any other longer string is the
    /// token each masked run becomes.
    ///
    /// # Errors
    ///
    /// An empty mask, which would leave the removed characters nothing in
    /// their place.
    pub fn parse(src: &str) -> Result<Mask, String> {
        if src == "shape" {
            return Ok(Mask::Shape);
        }
        if src == "pseudonym" {
            return Ok(Mask::Pseudonym(Pseudonyms::default()));
        }
        let mut chars = src.chars();
        match (chars.next(), chars.next()) {
            (None, _) => Err("the mask is empty; give a character or a token".to_string()),
            (Some(c), None) => Ok(Mask::PerChar(c)),
            (Some(_), Some(_)) => Ok(Mask::Token(src.to_string())),
        }
    }

    /// The kind a masked run reads as, for the masks that need one: the kind
    /// of its only significant token as `lexing` lexes it, a declared or
    /// library kind by the name its declaration gives it, or `value` where
    /// the run is not one token.
    fn kind_of(run: &[u8], lexing: &crate::ShapeSet) -> String {
        let toks = if lexing.is_empty() {
            crate::lexer::lex(run)
        } else {
            crate::lexer::lex_with_shapes(run, &crate::lexer::blob_runs(run), lexing, 0)
        };
        let mut significant = toks.iter().filter(|t| t.is_significant());
        match (significant.next(), significant.next()) {
            (Some(one), None) => match one.kind {
                crate::token::TokenKind::Custom(id) => match lexing.name_of(id) {
                    Some(name) => name.to_string(),
                    None => one.kind.name().to_string(),
                },
                other => other.name().to_string(),
            },
            _ => "value".to_string(),
        }
    }

    /// Write what stands in for the bytes of `run`, which is never empty,
    /// reading its kind under `lexing` where the mask names the kind.
    fn cover(&mut self, run: &[u8], out: &mut Vec<u8>, lexing: &crate::ShapeSet) {
        match self {
            Mask::PerChar(c) => {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf).as_bytes();
                let chars = run.iter().filter(|&&b| b & 0xC0 != 0x80).count();
                for _ in 0..chars {
                    out.extend_from_slice(encoded);
                }
            }
            Mask::Token(t) => out.extend_from_slice(t.as_bytes()),
            Mask::Shape => out.extend(run.iter().map(|&b| match b {
                b'0'..=b'9' => MASKED_DIGIT,
                b'A'..=b'Z' | b'a'..=b'z' => MASKED_LETTER,
                other => other,
            })),
            Mask::Pseudonym(book) => {
                let kind = Mask::kind_of(run, lexing);
                out.extend_from_slice(book.name(&kind, run).as_bytes());
            }
        }
    }
}

/// The edits that redact `matches` over `input`: each match's span becomes
/// the mask, with the fields `keeps` locate left unmasked. A kept
/// field outside its match, which a register bound in an assertion can be,
/// is clipped to the match; overlapping fields are kept once. A pseudonym
/// reads each run's kind with no declarations; a pattern compiled against
/// some redacts through [`redactions_with_shapes`].
#[must_use]
pub fn redactions<M: Spanned>(
    input: &[u8],
    matches: &[M],
    keeps: &[Keep],
    mask: &mut Mask,
) -> Vec<crate::files::Edit> {
    redactions_under(input, matches, keeps, mask, &crate::ShapeSet::new())
}

/// As [`redactions`], for matches of `pattern` compiled against `shapes`: a
/// pseudonym names a run of a declared shape or kind, or of a library kind
/// the pattern names, by that kind's own name, as `CUSTOMER_1`.
#[must_use]
pub fn redactions_with_shapes<M: Spanned>(
    input: &[u8],
    matches: &[M],
    keeps: &[Keep],
    mask: &mut Mask,
    pattern: &Pattern,
    shapes: &crate::ShapeSet,
) -> Vec<crate::files::Edit> {
    redactions_under(input, matches, keeps, mask, &shapes.with_library_shapes(&pattern.library_kinds()))
}

/// The redaction both forms make, a run's kind read under `lexing`.
fn redactions_under<M: Spanned>(
    input: &[u8],
    matches: &[M],
    keeps: &[Keep],
    mask: &mut Mask,
    lexing: &crate::ShapeSet,
) -> Vec<crate::files::Edit> {
    matches
        .iter()
        .map(|m| {
            let (start, end) = (m.start(), m.end());
            let mut kept: Vec<ByteRange> = keeps
                .iter()
                .filter_map(|k| k.locate(m, input))
                .map(|r| r.start.max(start)..r.end.min(end))
                .filter(|r| !r.is_empty())
                .collect();
            kept.sort_by_key(|r| (r.start, r.end));
            let mut replacement = Vec::with_capacity(end - start);
            let mut cursor = start;
            for r in kept {
                if r.start > cursor {
                    mask.cover(&input[cursor..r.start], &mut replacement, lexing);
                }
                if r.end > cursor {
                    replacement.extend_from_slice(&input[r.start.max(cursor)..r.end]);
                    cursor = r.end;
                }
            }
            if cursor < end {
                mask.cover(&input[cursor..end], &mut replacement, lexing);
            }
            crate::files::Edit { start, end, replacement }
        })
        .collect()
}

/// A match as a render reads it: its byte range and the registers it bound,
/// none for a plain span.
pub trait Spanned: Sync {
    /// The byte offset the match begins at.
    fn start(&self) -> usize;
    /// The byte offset just past it.
    fn end(&self) -> usize;
    /// The register spans, in the order [`Self::names`] holds their names.
    fn captures(&self) -> &[Span];
    /// The register names, which belong to the pattern rather than to any one
    /// of its matches.
    fn names(&self) -> &[String];
    /// Every binding the register called `name` made, oldest first, where it
    /// is bound under a repetition; `None` where it is not, or where the
    /// match carries no such bindings.
    fn history(&self, _name: &str) -> Option<&[Span]> {
        None
    }
}

impl Spanned for Span {
    fn start(&self) -> usize {
        Span::start(self)
    }

    fn end(&self) -> usize {
        Span::end(self)
    }

    fn captures(&self) -> &[Span] {
        &[]
    }

    fn names(&self) -> &[String] {
        &[]
    }
}

impl Spanned for Match {
    fn start(&self) -> usize {
        self.start
    }

    fn end(&self) -> usize {
        self.end
    }

    fn captures(&self) -> &[Span] {
        &self.captures
    }

    fn names(&self) -> &[String] {
        Match::names(self)
    }

    fn history(&self, name: &str) -> Option<&[Span]> {
        self.list(name)
    }
}

/// Parse the body of a `${...}` reference: a name (`0` for the whole
/// match) and an optional `:transform`.
/// What a `${...}` reference names: the whole match, or one register.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    /// `${0}`: the whole matched span.
    Whole,
    /// `${name}` or `${n}`: the register the pattern binds under that name,
    /// or at that position.
    Register(String),
    /// `${name[i]}`, `${a[i].b}`: the i-th binding, from zero, of a register
    /// bound under a repetition.
    Item(String, usize),
    /// `${name[*]}`: every binding of a register, joined with a comma; the
    /// one binding of a register bound once.
    All(String),
}

/// Every binding of the register `name` in `m`, each read through `accs`,
/// joined with a comma: the bindings a repetition made, or the register's
/// one binding where it was bound once.
fn every_binding<M: Spanned>(m: &M, input: &[u8], name: &str, accs: &[Accessor]) -> String {
    let read = |range: std::ops::Range<usize>| apply_all(accs, &String::from_utf8_lossy(&input[range]));
    match m.history(name) {
        Some(all) => all.iter().map(|s| read(s.range())).collect::<Vec<_>>().join(","),
        None => match m.names().iter().position(|k| k == name).and_then(|i| m.captures().get(i)) {
            Some(s) => read(s.range()),
            None => String::new(),
        },
    }
}

/// The `${@...}` reference `body` writes, or `None` where it writes no `@`
/// and so names a register.
///
/// The axis and the piece are checked here, against the axes the explainer
/// reads, so `${@entrpoy}` stops at parse time as a misspelled register
/// name does. An axis the pattern never reads is not an error: it renders
/// empty, which is what the match has to say about it.
fn parse_explain(body: &str, pos: usize, allowed: bool) -> Result<Option<Part>, TemplateError> {
    if !body.starts_with('@') {
        return Ok(None);
    }
    if !allowed {
        return Err(TemplateError {
            pos,
            msg: format!(
                "${{{body}}} reads an axis, which a scan's --format renders; \
                 the bytes a rewrite splices in have no explanation to read"
            ),
        });
    }
    let (name, accs) = match body.split_once(':') {
        Some((n, a)) => (n, parse_accessors(a, pos)?),
        None => (body, Vec::new()),
    };
    let (name, at) = match split_index(&name[1..], pos)? {
        (name, Some(Pick::All)) => {
            return Err(TemplateError {
                pos,
                msg: format!("${{@{name}[*]}}: an axis written bare already joins every reading; [i] picks one"),
            });
        }
        (name, Some(Pick::At(i))) => (name, Some(i)),
        (name, None) => (name, None),
    };
    let (axis, piece) = match name.split_once('.') {
        Some((axis, piece)) => (axis, Some(piece)),
        None => (name.as_str(), None),
    };
    crate::explain::check_explain_field(axis, piece)
        .map_err(|msg| TemplateError { pos, msg })?;
    let named =
        ExplainRef { axis: axis.to_string(), piece: piece.map(str::to_string), at };
    Ok(Some(Part::Explain(named, accs)))
}

fn parse_ref(
    body: &str,
    pos: usize,
    bound: &[String],
    axes: bool,
) -> Result<Part, TemplateError> {
    if let Some(part) = parse_explain(body, pos, axes)? {
        return Ok(part);
    }
    let (field, accs) = parse_field(body, pos, bound)?;
    Ok(match field {
        Field::Whole => Part::WholeMatch(accs),
        Field::Register(name) => Part::Capture(name, accs),
        Field::Item(name, index) => Part::Item(name, index, accs),
        Field::All(name) => Part::All(name, accs),
    })
}

/// Which bindings of a register a reference's index picks: one by its
/// place, or `[*]`, every one.
enum Pick {
    At(usize),
    All,
}

/// A reference's name with its index taken out: `a[i].b` and `a.b[i]` both
/// name the register `a.b` at `i`, `[*]` names every binding, and a
/// reference carries one index at most.
fn split_index(name: &str, pos: usize) -> Result<(String, Option<Pick>), TemplateError> {
    let mut out = String::new();
    let mut index = None;
    for segment in name.split('.') {
        let (base, at) = match segment.split_once('[') {
            Some((base, rest)) => {
                let digits = rest.strip_suffix(']').ok_or_else(|| TemplateError {
                    pos,
                    msg: format!("{segment:?}: an index closes with ], as in {base}[0]"),
                })?;
                if digits == "*" {
                    (base, Some(Pick::All))
                } else {
                    let i = digits.parse::<usize>().map_err(|e| TemplateError {
                        pos,
                        msg: format!("{segment:?}: an index is a number counted from 0, or * for every one: {e}"),
                    })?;
                    (base, Some(Pick::At(i)))
                }
            }
            None => (segment, None),
        };
        if let Some(i) = at {
            if index.is_some() {
                return Err(TemplateError {
                    pos,
                    msg: format!("{name:?} carries two indexes; a reference carries one"),
                });
            }
            index = Some(i);
        }
        if !out.is_empty() {
            out.push('.');
        }
        out.push_str(base);
    }
    Ok((out, index))
}

/// The reference and the accessor pipeline a `${...}` body writes,
/// validated against the pattern's capture names.
fn parse_field(
    body: &str,
    pos: usize,
    bound: &[String],
) -> Result<(Field, Vec<Accessor>), TemplateError> {
    let (name, accs) = match body.split_once(':') {
        Some((n, a)) => (n, parse_accessors(a, pos)?),
        None => (body, Vec::new()),
    };
    if name.is_empty() {
        return Err(TemplateError { pos, msg: "empty capture name in ${...}".into() });
    }
    // Reached with an `@` only from a reference read outside a template,
    // which resolves to bytes of the match; a template's own parse takes the
    // axis path before here.
    if let Some(axis) = name.strip_prefix('@') {
        return Err(TemplateError {
            pos,
            msg: format!(
                "${{@{axis}}} reads an axis, which a --format or report template renders and a reference to the match's bytes cannot"
            ),
        });
    }
    let (name, index) = split_index(name, pos)?;
    if name == "0" {
        return match index {
            None => Ok((Field::Whole, accs)),
            Some(_) => Err(TemplateError { pos, msg: "${0} is the whole match and takes no index".into() }),
        };
    }
    let field = |name: String| match index {
        Some(Pick::At(i)) => Field::Item(name, i),
        Some(Pick::All) => Field::All(name),
        None => Field::Register(name),
    };
    // A number is the capture's position: `${1}` is the first name the
    // pattern binds. trex's `(...)` is a logical group and binds nothing, so
    // there is no group to count; what is numbered is the bindings, in the
    // order they are written. That keeps one meaning of "capture" rather than
    // adding a second, anonymous kind alongside `:name`.
    if name.bytes().all(|b| b.is_ascii_digit()) {
        let n: usize = match name.parse() {
            Ok(n) => n,
            Err(e) => {
                return Err(TemplateError {
                    pos,
                    msg: format!("capture number {name:?} is out of range: {e}"),
                });
            }
        };
        return match bound.get(n.wrapping_sub(1)) {
            Some(found) => Ok((field(found.clone()), accs)),
            None => Err(TemplateError {
                pos,
                msg: format!(
                    "template references ${{{n}}} but the pattern binds {} capture(s)",
                    bound.len()
                ),
            }),
        };
    }
    if !bound.contains(&name) {
        return Err(TemplateError {
            pos,
            msg: format!("template references ${{{name}}} but the pattern binds no such capture"),
        });
    }
    Ok((field(name), accs))
}

/// A `${...}` reference read outside a template: the field it names and the
/// accessors that slice it, validated against the pattern's capture names as
/// a template's are, so a callback reads `e:domain` or `0:last4` as a
/// template renders it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    field: Field,
    accessors: Vec<Accessor>,
}

impl Reference {
    /// Parse the body a template writes inside `${...}`: `0`, a register's
    /// name or its position among `bound`, and the accessors after a colon.
    ///
    /// # Errors
    ///
    /// A name the pattern binds no register under, a position past the last
    /// it binds, or an accessor that is not one.
    pub fn parse(body: &str, bound: &[String]) -> Result<Reference, TemplateError> {
        let (field, accessors) = parse_field(body, 0, bound)?;
        Ok(Reference { field, accessors })
    }

    /// What the reference names: the whole match or one register.
    #[must_use]
    pub fn field(&self) -> &Field {
        &self.field
    }

    /// The accessors applied to `text`, left to right: `text` itself where
    /// there are none, and nothing where a slicing accessor finds nothing.
    #[must_use]
    pub fn apply(&self, text: &str) -> String {
        apply_all(&self.accessors, text)
    }

    /// The reference read over one match: the field's bytes, then the
    /// accessors, as a template renders `${body}` there.
    #[must_use]
    pub fn read<M: Spanned>(&self, m: &M, input: &[u8]) -> String {
        let bytes = match &self.field {
            Field::All(name) => return every_binding(m, input, name, &self.accessors),
            Field::Whole => &input[m.start()..m.end()],
            Field::Register(name) => m
                .names()
                .iter()
                .position(|k| k == name)
                .and_then(|i| m.captures().get(i))
                .map_or(&[] as &[u8], |s| &input[s.range()]),
            Field::Item(name, index) => m
                .history(name)
                .and_then(|all| all.get(*index))
                .map_or(&[] as &[u8], |s| &input[s.range()]),
        };
        self.apply(&String::from_utf8_lossy(bytes))
    }
}

/// Parse a `|`-separated accessor pipeline (the part after `:`).
fn parse_accessors(s: &str, pos: usize) -> Result<Vec<Accessor>, TemplateError> {
    s.split('|').map(|t| parse_accessor(t.trim(), pos)).collect()
}

fn parse_accessor(t: &str, pos: usize) -> Result<Accessor, TemplateError> {
    let a = match t {
        "upper" => Accessor::Upper,
        "lower" => Accessor::Lower,
        "trim" => Accessor::Trim,
        "scheme" => Accessor::UrlScheme,
        "host" => Accessor::UrlHost,
        "port" => Accessor::UrlPort,
        "path" => Accessor::UrlPath,
        "query" => Accessor::UrlQuery,
        "user" => Accessor::EmailUser,
        "domain" => Accessor::EmailDomain,
        "major" => Accessor::VerMajor,
        "minor" => Accessor::VerMinor,
        "patch" => Accessor::VerPatch,
        "year" => Accessor::TsField(0),
        "month" => Accessor::TsField(1),
        "day" => Accessor::TsField(2),
        "hour" => Accessor::TsField(3),
        "minute" => Accessor::TsField(4),
        "second" => Accessor::TsField(5),
        "dir" => Accessor::PathDir,
        "name" => Accessor::PathName,
        "ext" => Accessor::PathExt,
        "value" => Accessor::QtyValue,
        "unit" => Accessor::QtyUnit,
        _ => {
            if let Some(r) = t.strip_prefix("octet") {
                let (x, y) = parse_range(r, pos)?;
                Accessor::Octet(x, y)
            } else if let Some(r) = t.strip_prefix("group") {
                let (x, y) = parse_range(r, pos)?;
                Accessor::Group(x, y)
            } else if let Some(r) = t.strip_prefix("first") {
                Accessor::First(parse_count(r, "first", pos)?)
            } else if let Some(r) = t.strip_prefix("last") {
                Accessor::Last(parse_count(r, "last", pos)?)
            } else {
                return Err(TemplateError {
                    pos,
                    msg: format!(
                        "unknown accessor ':{t}' (use upper/lower/trim; firstN/lastN; octetN[-M]; groupN[-M]; \
                         scheme/host/port/path/query; user/domain; major/minor/patch; \
                         year/month/day/hour/minute/second; dir/name/ext; value/unit)"
                    ),
                });
            }
        }
    };
    Ok(a)
}

/// Parse a 1-based index `N` or inclusive range `N-M`.
fn parse_range(r: &str, pos: usize) -> Result<(usize, usize), TemplateError> {
    let bad = || TemplateError { pos, msg: format!("bad index '{r}' (use N or N-M, 1-based)") };
    match r.split_once('-') {
        Some((a, b)) => {
            let a = a.parse::<usize>().map_err(|_| bad())?;
            let b = b.parse::<usize>().map_err(|_| bad())?;
            if a == 0 || b < a {
                return Err(bad());
            }
            Ok((a, b))
        }
        None => {
            let a = r.parse::<usize>().map_err(|_| bad())?;
            if a == 0 {
                return Err(bad());
            }
            Ok((a, a))
        }
    }
}

/// The spans with their registers resolved as `template` reads them: every
/// binding under a repetition where it reads one by index, the last alone
/// otherwise, which the cheaper rungs answer.
fn resolve(pattern: &Pattern, template: &Template, input: &[u8], spans: &[Span]) -> Vec<Match> {
    if template.reads_lists() {
        captures_with_lists(pattern, input, spans)
    } else {
        captures(pattern, input, spans)
    }
}

/// [`resolve`] under declared shapes, which decide the token boundaries a
/// match was found on and so must decide the ones its registers are read on.
fn resolve_with_shapes(
    pattern: &Pattern,
    template: &Template,
    input: &[u8],
    shapes: &crate::custom::ShapeSet,
    spans: &[Span],
) -> Vec<Match> {
    if template.reads_lists() {
        crate::engine::captures_with_shapes_and_lists(pattern, input, shapes, spans)
    } else {
        crate::engine::captures_with_shapes(pattern, input, shapes, spans)
    }
}

/// [`edits`] with `shapes` in force, for a caller whose pattern file
/// declares a shape or a kind: the scan lexes under them and the registers
/// are resolved on the same boundaries, so a rewrite over a declared kind
/// replaces what a scan over it reports.
#[must_use]
pub fn edits_with_shapes(
    pattern: &Pattern,
    template: &Template,
    input: &[u8],
    shapes: &crate::custom::ShapeSet,
) -> Vec<crate::files::Edit> {
    edits_at(pattern, template, input, shapes, &crate::engine::scan_with_shapes(pattern, input, shapes))
}

/// The edits `template` makes at `spans`, matches of `pattern` over `input`
/// found under `shapes`: each span and the bytes the template renders for
/// it, the registers resolved on the boundaries the shapes decide. What a
/// rewrite of a stream renders once the stream has committed its matches.
#[must_use]
pub fn edits_at(
    pattern: &Pattern,
    template: &Template,
    input: &[u8],
    shapes: &crate::custom::ShapeSet,
    spans: &[Span],
) -> Vec<crate::files::Edit> {
    if template.reads_captures() {
        resolve_with_shapes(pattern, template, input, shapes, spans)
            .iter()
            .map(|m| crate::files::Edit {
                start: m.start,
                end: m.end,
                replacement: template.render(m, input).into_bytes(),
            })
            .collect()
    } else {
        spans
            .iter()
            .map(|s| crate::files::Edit {
                start: s.start(),
                end: s.end(),
                replacement: template.render(s, input).into_bytes(),
            })
            .collect()
    }
}

/// The edits a rewrite of `input` makes, as every surface rewrites a file:
/// the matches found under `shapes` where any are declared, which lex with
/// them and take no device, and otherwise as `engine` says, the first
/// `max_count` of them where a count is given, each replaced by `template`.
/// Beside them, what a scan the engine forced ran: a device declined where a
/// device was requested for a pattern under declared shapes.
#[must_use]
pub fn edits_by(
    pattern: &Pattern,
    template: &Template,
    input: &[u8],
    shapes: &crate::custom::ShapeSet,
    engine: &Engine,
    max_count: Option<usize>,
) -> (Vec<crate::files::Edit>, Option<Ran>) {
    let (mut spans, ran) = if !shapes.is_empty() {
        let declined = (engine.backend == Backend::Gpu).then_some(Ran::DeviceDeclined);
        (crate::engine::scan_with_shapes(pattern, input, shapes), declined)
    } else if engine.is_plain() {
        // The first few stop where a route lets them, as `rewrite_n` does.
        let spans: Vec<Span> = match max_count {
            Some(1) => crate::cursor::find(pattern, input).into_iter().collect(),
            Some(n) => crate::cursor::find_iter(pattern, input).take(n).collect(),
            None => scan(pattern, input),
        };
        (spans, None)
    } else {
        let (spans, ran) = scan_engine(pattern, input, engine);
        (spans, Some(ran))
    };
    if let Some(n) = max_count {
        spans.truncate(n);
    }
    if shapes.is_empty() {
        return (rendered(pattern, template, input, &spans), ran);
    }
    (edits_at(pattern, template, input, shapes, &spans), ran)
}

/// The edits `template` makes at `spans`, matches of `pattern` over `input`
/// found with nothing declared: each span and the bytes rendered for it, the
/// registers resolved as [`rewrite`] resolves them.
fn rendered(pattern: &Pattern, template: &Template, input: &[u8], spans: &[Span]) -> Vec<crate::files::Edit> {
    if template.reads_captures() {
        resolve(pattern, template, input, spans)
            .iter()
            .map(|m| crate::files::Edit { start: m.start, end: m.end, replacement: template.render(m, input).into_bytes() })
            .collect()
    } else {
        spans
            .iter()
            .map(|s| crate::files::Edit {
                start: s.start(),
                end: s.end(),
                replacement: template.render(s, input).into_bytes(),
            })
            .collect()
    }
}

/// [`rewrite`] with `shapes` in force. A set with nothing in it is the
/// plain rewrite, which keeps the backend routing a declared shape rules
/// out.
#[must_use]
pub fn rewrite_with_shapes(
    pattern: &Pattern,
    template: &Template,
    input: &[u8],
    shapes: &crate::custom::ShapeSet,
) -> Vec<u8> {
    if shapes.is_empty() && pattern.library_kinds().is_empty() {
        return rewrite(pattern, template, input);
    }
    let spans = crate::engine::scan_with_shapes(pattern, input, shapes);
    if template.reads_captures() {
        return splice_parallel(input, &resolve_with_shapes(pattern, template, input, shapes, &spans), template);
    }
    splice_parallel(input, &spans, template)
}

/// The edits [`rewrite`] would make: each match's span and the bytes the
/// template renders for it, in input order. What a dry run diffs.
#[must_use]
pub fn edits(pattern: &Pattern, template: &Template, input: &[u8]) -> Vec<crate::files::Edit> {
    rendered(pattern, template, input, &scan(pattern, input))
}

/// Rewrite `input` for `pattern` with `template`: every leftmost,
/// non-overlapping match is replaced by the rendered template, and the
/// gaps between matches are copied verbatim.
#[must_use]
pub fn rewrite(pattern: &Pattern, template: &Template, input: &[u8]) -> Vec<u8> {
    let spans = scan(pattern, input);
    if template.reads_captures() {
        return splice_parallel(input, &resolve(pattern, template, input, &spans), template);
    }
    splice_parallel(input, &spans, template)
}

/// [`rewrite`] stopping after `n` matches, leaving the rest of `input`
/// unchanged.
///
/// A rewrite of a single match reads no further than that match, because it
/// takes the same early-stopping path [`crate::find`] takes. A rewrite of the
/// first few reads the whole input where a route answers the pattern: a route
/// reports its matches together and has no form that stops at the n-th, so a
/// cursor over one has already computed them all before the first is taken.
///
/// The splice is the same one a full rewrite uses: a partial rewrite differs
/// in how many matches it is given, not in how they are rendered or copied
/// around.
#[must_use]
pub fn rewrite_n(pattern: &Pattern, template: &Template, input: &[u8], n: usize) -> Vec<u8> {
    // A cursor over a routed pattern computes every match before the first is
    // taken, so `take(1)` discards work already done. `find` stops at the
    // first match where a route can. A larger `n` still computes every match.
    let spans: Vec<crate::engine::Span> = if n == 1 {
        crate::cursor::find(pattern, input).into_iter().collect()
    } else {
        crate::cursor::find_iter(pattern, input).take(n).collect()
    };
    if template.reads_captures() {
        return splice_parallel(input, &resolve(pattern, template, input, &spans), template);
    }
    splice_parallel(input, &spans, template)
}

/// [`rewrite`] of the first match only.
#[must_use]
pub fn rewrite_first(pattern: &Pattern, template: &Template, input: &[u8]) -> Vec<u8> {
    rewrite_n(pattern, template, input, 1)
}

/// One match as a rewrite's callback sees it: its span and bytes in the
/// input, what each register bound, and any slice a template reference
/// names.
#[derive(Clone, Copy, Debug)]
pub struct Matched<'a> {
    input: &'a [u8],
    m: &'a Match,
    /// The pattern's capture names in written order, which a numbered
    /// reference counts through.
    bound: &'a [String],
    /// The kind each register binds, for [`Matched::value`]. Empty where the
    /// caller supplied none, in which case no register reports a value.
    kinds: &'a [(String, Option<crate::token::TokenKind>)],
}

impl<'a> Matched<'a> {
    /// The match `m` over `input`, with `bound` the pattern's
    /// [`Pattern::capture_names`], which a reference such as `1:upper`
    /// numbers through.
    /// Built this way no register reports a typed value, because nothing
    /// here says which kind any of them binds. [`Matched::with_kinds`]
    /// supplies that.
    #[must_use]
    pub fn new(m: &'a Match, input: &'a [u8], bound: &'a [String]) -> Self {
        Matched { input, m, bound, kinds: &[] }
    }

    /// [`Matched::new`] knowing which kind each register binds, from
    /// [`Pattern::capture_kinds`], so [`Matched::value`] can answer.
    #[must_use]
    pub fn with_kinds(
        m: &'a Match,
        input: &'a [u8],
        bound: &'a [String],
        kinds: &'a [(String, Option<crate::token::TokenKind>)],
    ) -> Self {
        Matched { input, m, bound, kinds }
    }

    /// The parsed value the register `name` bound, in its base unit, or
    /// `None` where it bound no single typed kind or its text does not parse
    /// as one.
    ///
    /// This is the read a `:value` clause makes, so a rewrite computing from
    /// a value and a predicate selecting on one cannot disagree. A byte size
    /// arrives in bytes, a duration in nanoseconds, a timestamp as a calendar
    /// instant; nothing is parsed twice.
    #[must_use]
    pub fn value(&self, name: &str) -> Option<crate::typed::TypedValue> {
        let kind = self.kinds.iter().find(|(n, _)| n == name).and_then(|(_, k)| *k)?;
        let text = self.group(name)?;
        crate::typed::value_of(kind, &String::from_utf8_lossy(text))
    }

    /// The byte offset the match begins at.
    #[must_use]
    pub fn start(&self) -> usize {
        self.m.start
    }

    /// The byte offset just past it.
    #[must_use]
    pub fn end(&self) -> usize {
        self.m.end
    }

    /// The matched bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &'a [u8] {
        &self.input[self.m.start..self.m.end]
    }

    /// The matched bytes as text, with replacement characters where they
    /// are not UTF-8.
    #[must_use]
    pub fn text(&self) -> Cow<'a, str> {
        String::from_utf8_lossy(self.as_bytes())
    }

    /// The whole input the match was found in.
    #[must_use]
    pub fn input(&self) -> &'a [u8] {
        self.input
    }

    /// The match itself, its registers as spans through [`Match::captures`]
    /// and [`Match::names`].
    #[must_use]
    pub fn inner(&self) -> &'a Match {
        self.m
    }

    /// The bytes the register called `name` bound, or `None` where the
    /// pattern names no such register.
    #[must_use]
    pub fn group(&self, name: &str) -> Option<&'a [u8]> {
        self.m.group(name, self.input)
    }

    /// The register names, in the order [`Match::captures`] holds their
    /// spans.
    #[must_use]
    pub fn names(&self) -> &'a [String] {
        self.m.names()
    }

    /// A template reference read over this match, as `${...}` renders it:
    /// `0` the whole match, `e` a register, `1` the first the pattern
    /// binds, and `e:domain`, `0:last4` or `ip:octet1-2` a slice of one.
    ///
    /// # Errors
    ///
    /// A name the pattern binds no register under, a position past the last
    /// it binds, or an accessor that is not one.
    pub fn get(&self, reference: &str) -> Result<String, TemplateError> {
        Ok(Reference::parse(reference, self.bound)?.read(self.m, self.input))
    }
}

/// Splice what `replace` returns for each match into the input, serially:
/// the callback decides every replacement in turn, so there is no render to
/// share out.
fn splice_with<F, R>(
    input: &[u8],
    matches: &[Match],
    bound: &[String],
    kinds: &[(String, Option<crate::token::TokenKind>)],
    mut replace: F,
) -> Vec<u8>
where
    F: FnMut(&Matched<'_>) -> R,
    R: AsRef<[u8]>,
{
    let mut out = Vec::with_capacity(input.len());
    let mut at = 0;
    for m in matches {
        out.extend_from_slice(&input[at..m.start]);
        out.extend_from_slice(replace(&Matched { input, m, bound, kinds }).as_ref());
        at = m.end;
    }
    out.extend_from_slice(&input[at..]);
    out
}

/// Rewrite `input` for `pattern` with `replace` deciding each replacement:
/// every leftmost, non-overlapping match, its registers resolved, is handed
/// to `replace` as a [`Matched`], the bytes it returns take the match's
/// place, and the gaps between matches are copied verbatim. What a template
/// cannot say - a replacement computed from the match, a count, a lookup -
/// is written here; a template is [`rewrite`].
pub fn rewrite_with<F, R>(pattern: &Pattern, input: &[u8], replace: F) -> Vec<u8>
where
    F: FnMut(&Matched<'_>) -> R,
    R: AsRef<[u8]>,
{
    let spans = scan(pattern, input);
    let bound = pattern.capture_names();
    let kinds = pattern.capture_kinds();
    // The closure may read any binding, so every one is kept.
    splice_with(input, &captures_with_lists(pattern, input, &spans), &bound, &kinds, replace)
}

/// [`rewrite_with`] stopping after `n` matches, leaving the rest of `input`
/// unchanged; the matches are found as [`rewrite_n`] finds them.
pub fn rewrite_n_with<F, R>(pattern: &Pattern, input: &[u8], n: usize, replace: F) -> Vec<u8>
where
    F: FnMut(&Matched<'_>) -> R,
    R: AsRef<[u8]>,
{
    let spans: Vec<Span> = if n == 1 {
        crate::cursor::find(pattern, input).into_iter().collect()
    } else {
        crate::cursor::find_iter(pattern, input).take(n).collect()
    };
    let bound = pattern.capture_names();
    let kinds = pattern.capture_kinds();
    // The closure may read any binding, so every one is kept.
    splice_with(input, &captures_with_lists(pattern, input, &spans), &bound, &kinds, replace)
}

/// Rewrite on the GPU, or `None` when the device path does not apply
/// (the pattern is outside the device subset, no device, or a build
/// without the `gpu` feature). The match runs on the device; rendering
/// and the splice run on the host. The output is identical to
/// [`rewrite`]. The caller falls back to [`rewrite`] on `None`.
#[must_use]
pub fn rewrite_gpu(pattern: &Pattern, template: &Template, input: &[u8]) -> Option<Vec<u8>> {
    let spans = scan_gpu(pattern, input)?;
    if template.reads_captures() {
        return Some(splice_parallel(input, &resolve(pattern, template, input, &spans), template));
    }
    Some(splice_parallel(input, &spans, template))
}

/// Rewrite on the backend chosen by `backend`, and report which one ran.
/// `Auto` matches where [`scan_with_backend`] places the scan; `Gpu` forces
/// the device and falls back when it cannot run; `Cpu` never touches the
/// device. The rendering and the splice always run on the host, so the
/// output is identical to [`rewrite`] on every backend.
#[must_use]
pub fn rewrite_with_backend(
    pattern: &Pattern,
    template: &Template,
    input: &[u8],
    backend: Backend,
) -> (Vec<u8>, BackendUsed) {
    match backend {
        Backend::Cpu => (rewrite(pattern, template, input), BackendUsed::Cpu),
        Backend::Gpu => match rewrite_gpu(pattern, template, input) {
            Some(o) => (o, BackendUsed::Gpu),
            None => (rewrite(pattern, template, input), BackendUsed::Cpu),
        },
        Backend::Auto => {
            let (spans, used) = scan_with_backend(pattern, input, Backend::Auto);
            let out = if template.reads_captures() {
                splice_parallel(input, &resolve(pattern, template, input, &spans), template)
            } else {
                splice_parallel(input, &spans, template)
            };
            (out, used)
        }
    }
}

/// Splice rendered matches into the input, serially. The canonical
/// output the parallel path is checked against.
#[must_use]
pub(crate) fn splice<M: Spanned>(input: &[u8], matches: &[M], template: &Template) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(input.len());
    let mut pos = 0;
    for m in matches {
        out.extend_from_slice(&input[pos..m.start()]);
        out.extend_from_slice(template.render(m, input).as_bytes());
        pos = m.end();
    }
    out.extend_from_slice(&input[pos..]);
    out
}

/// Splice `rendered` into every match's place, for a template that renders the
/// same bytes at each.
///
/// The output is what [`splice`] produces for such a template, reached without
/// a render a match. The whole output is sized before the first copy, because
/// the matches say exactly how many bytes they replace.
#[must_use]
fn splice_one_string<M: Spanned>(input: &[u8], matches: &[M], rendered: &[u8]) -> Vec<u8> {
    let replaced: usize = matches.iter().map(|m| m.end() - m.start()).sum();
    let mut out: Vec<u8> =
        Vec::with_capacity(input.len() - replaced + matches.len() * rendered.len());
    let mut pos = 0;
    for m in matches {
        out.extend_from_slice(&input[pos..m.start()]);
        out.extend_from_slice(rendered);
        pos = m.end();
    }
    out.extend_from_slice(&input[pos..]);
    out
}

/// Above this many matches the renders are computed across cores. Each
/// match's render is independent (it reads only its own captures), so
/// the render phase is an embarrassingly parallel map; the assembly that
/// follows is one sequential copy.
const PARALLEL_REWRITE_THRESHOLD: usize = 1024;

/// Splice with the per-match renders computed in parallel. Identical
/// output to [`splice`]: the match set is non-overlapping and each
/// render depends only on its own match, so order is preserved and the
/// renders never interact.
#[must_use]
pub(crate) fn splice_parallel<M: Spanned>(input: &[u8], matches: &[M], template: &Template) -> Vec<u8> {
    // A template of literals alone renders the same bytes at every match, so
    // the render is computed once and copied. Rendering it a match allocates a
    // string a match here and a buffer a match below, for bytes that never
    // differ - and a constant replacement is the commonest rewrite there is.
    if template.renders_one_string() {
        return splice_one_string(input, matches, template.one_string().as_bytes());
    }
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    if matches.len() < PARALLEL_REWRITE_THRESHOLD || cores <= 1 {
        return splice(input, matches, template);
    }

    // Render every match into its own byte buffer across cores through the
    // work-stealing pool. Each render reads only its own match, so the
    // renders are independent and the leaf writes only its own slots. The
    // per-render estimate makes the pool's serial-vs-parallel choice depend
    // on total work, not on K_outer (which a rewrite has no meaning for).
    let mut renders: Vec<Vec<u8>> = matches.iter().map(|_| Vec::new()).collect();
    let min_leaf = matches.len().div_ceil(cores * 4).max(64);
    let plan = flynnel::JobPlan::new(0, matches.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::PortCompute);
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(
        &plan,
        &mut renders,
        min_leaf,
        |start, slots| {
            for (i, slot) in slots.iter_mut().enumerate() {
                *slot = template.render(&matches[start + i], input).into_bytes();
            }
        },
    );

    // Assemble: gaps verbatim, renders in order. One sequential copy.
    let total: usize =
        input.len() + renders.iter().map(Vec::len).sum::<usize>() - spanned_len(matches);
    let mut out: Vec<u8> = Vec::with_capacity(total);
    let mut pos = 0;
    for (m, r) in matches.iter().zip(&renders) {
        out.extend_from_slice(&input[pos..m.start()]);
        out.extend_from_slice(r);
        pos = m.end();
    }
    out.extend_from_slice(&input[pos..]);
    out
}

/// Total input bytes covered by the matches, subtracted when sizing the
/// output (the matched spans are replaced by their renders).
fn spanned_len<M: Spanned>(matches: &[M]) -> usize {
    matches.iter().map(|m| m.end() - m.start()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    /// A timestamp's fields come from the form it is written in: the runs of
    /// digits are in a different order in every form a log writes, and a
    /// field written as a name or not written at all reads empty.
    #[test]
    fn a_timestamp_field_is_read_where_its_form_puts_it() {
        let fields =
            |v: &str| (0..6).map(|i| Accessor::TsField(i).apply(v)).collect::<Vec<_>>().join("|");
        assert_eq!(fields("2026-09-15T10:11:12"), "2026|09|15|10|11|12");
        assert_eq!(fields("2026/09/15 10:11:12"), "2026|09|15|10|11|12");
        assert_eq!(fields("15/09/2026 10:11:12"), "2026|09|15|10|11|12");
        assert_eq!(fields("09/15/2026 10:11:12"), "2026|09|15|10|11|12");
        // Apache writes the year third and names the month; syslog writes no
        // year at all; a bare clock opens with the hour and not the year.
        assert_eq!(fields("15/Sep/2026:10:11:12"), "2026||15|10|11|12");
        assert_eq!(fields("Sep 15 10:11:12"), "||15|10|11|12");
        assert_eq!(fields("10:11:12"), "|||10|11|12");
    }

    /// Rendering once and copying produces what rendering at every match
    /// produces, and only the templates whose every part is a literal take that
    /// path: `${0}` carries no register and still renders differently at each.
    #[test]
    fn a_template_of_literals_splices_what_rendering_each_match_splices() {
        let mut text = String::new();
        for i in 0..2000u32 {
            text.push_str(&format!("let value_{i} = {} ; call_{i}(alpha, beta) ;\n", i * 7));
        }
        let input = text.as_bytes();
        let pat = parse("\"let\" \\W:v \"=\"").expect("pattern parses");
        let names = pat.capture_names();
        let spans = crate::scan(&pat, input);
        let ms = crate::captures(&pat, input, &spans);
        assert!(ms.len() > PARALLEL_REWRITE_THRESHOLD, "the corpus crosses the parallel threshold");
        for (src, one) in [
            ("X", true),
            ("", true),
            ("<>", true),
            ("[${0}]", false),
            ("${v}", false),
            ("a${v}b", false),
            ("${0}${v}", false),
        ] {
            let tpl = Template::parse(src, &names).expect("template parses");
            assert_eq!(tpl.renders_one_string(), one, "{src:?}");
            assert_eq!(splice_parallel(input, &ms, &tpl), splice(input, &ms, &tpl), "{src:?}");
            // The spans alone take the same path, and a template reading a
            // register renders it empty over them rather than differently.
            if !tpl.reads_captures() {
                assert_eq!(
                    splice_parallel(input, &spans, &tpl),
                    splice(input, &spans, &tpl),
                    "{src:?} over spans"
                );
            }
        }
    }

    #[test]
    fn rewriting_one_match_takes_a_different_path_and_the_same_answer() {
        // A single match comes from find, which stops at the first, and not
        // from a cursor, which computes every match of a routed pattern before
        // the first is taken. Those are different code paths and they have to
        // agree, across the routes and the engine alike.
        let inputs = [
            "alpha beta alpha gamma alpha",
            "let a = 1 ; let b = 2 ; let c = 3 ;",
            "nothing here matches at all",
        ];
        for src in ["\"alpha\"", "\\W \"=\"", "\\N", "\\W:k \"=\""] {
            for input in inputs {
                let pat = parse(src).expect("pattern parses");
                let tpl = Template::parse("X", &pat.capture_names()).expect("template parses");
                let one = rewrite_first(&pat, &tpl, input.as_bytes());
                let by_n = rewrite_n(&pat, &tpl, input.as_bytes(), 1);
                assert_eq!(one, by_n, "{src} over {input:?}");

                // And against the spans taken one at a time, which is the
                // arm that did not change.
                let first: Vec<_> = crate::cursor::find_iter(&pat, input.as_bytes()).take(1).collect();
                let want = splice_parallel(input.as_bytes(), &first, &tpl);
                assert_eq!(one, want, "{src} over {input:?}: the paths disagree");
            }
        }
    }

    fn rw(pattern_src: &str, template_src: &str, input: &str) -> String {
        let pat = parse(pattern_src).expect("pattern parses");
        let tpl = Template::parse(template_src, &pat.capture_names()).expect("template parses");
        String::from_utf8(rewrite(&pat, &tpl, input.as_bytes())).expect("utf8")
    }

    #[test]
    fn a_capture_can_be_referenced_by_position() {
        // `${1}` is the first binding the pattern writes, so the two
        // spellings render the same thing and can be mixed.
        assert_eq!(
            rw("\\W:first \\W:second", "${2} ${1}", "alpha beta"),
            rw("\\W:first \\W:second", "${second} ${first}", "alpha beta"),
        );
        assert_eq!(rw("\\W:first \\W:second", "${2} ${1}", "alpha beta"), "beta alpha");
        // Accessors chain off a numbered reference the same way.
        assert_eq!(rw("\\W:a \\W:b", "${1:upper}", "alpha beta"), "ALPHA");
        // `${0}` stays the whole match, as it is in a regular expression.
        assert_eq!(rw("\\W:a \\W:b", "[${0}]", "alpha beta"), "[alpha beta]");
    }

    #[test]
    fn a_position_past_the_last_capture_is_a_template_error() {
        let pat = parse("\\W:only").expect("parses");
        let err = Template::parse("${2}", &pat.capture_names()).expect_err("refused");
        let msg = format!("{err:?}");
        assert!(msg.contains('2'), "names the position: {msg}");
        assert!(msg.contains('1'), "and says how many there are: {msg}");
    }

    #[test]
    fn redacts_typed_atom() {
        assert_eq!(rw("\\E:e", "[redacted]", "mail bob@x.com now"), "mail [redacted] now");
    }

    #[test]
    fn a_pseudonym_names_a_declared_or_library_kind_by_its_own_name() {
        fn replaced(pattern: &Pattern, shapes: &crate::ShapeSet, input: &[u8]) -> Vec<Vec<u8>> {
            let spans = crate::engine::scan_with_shapes(pattern, input, shapes);
            let mut mask = Mask::parse("pseudonym").expect("a mask");
            redactions_with_shapes(input, &spans, &[], &mut mask, pattern, shapes)
                .into_iter()
                .map(|e| e.replacement)
                .collect()
        }
        let mut shapes = crate::ShapeSet::new();
        shapes.declare_text("shape customer = `C\\d{5}`").expect("declares");
        let customer = crate::parser::parse_with_shapes("\\{customer}", &shapes).expect("parses");
        assert_eq!(
            replaced(&customer, &shapes, b"for C00042 and C00077 then C00042"),
            [b"CUSTOMER_1".to_vec(), b"CUSTOMER_2".to_vec(), b"CUSTOMER_1".to_vec()]
        );
        let iban = parse("\\{iban}").expect("parses");
        assert_eq!(
            replaced(&iban, &crate::ShapeSet::new(), b"pay DE89370400440532013000 now"),
            [b"IBAN_1".to_vec()]
        );
    }

    #[test]
    fn renames_balanced_tag_and_uppercases_body() {
        // The close tag is rewritten from the captured open, so balance
        // is preserved; the body is uppercased.
        assert_eq!(
            rw("<\\W:t>(.*):body</=t>", "<${t}>${body:upper}</${t}>", "<div>hi there</div>"),
            "<div>HI THERE</div>"
        );
    }

    #[test]
    fn reorders_captures() {
        assert_eq!(rw("\\W:a \\N:b", "${b}=${a}", "width 50"), "50=width");
    }

    #[test]
    fn whole_match_reference() {
        assert_eq!(rw("\\N", "[${0}]", "a 12 b 34"), "a [12] b [34]");
    }

    #[test]
    fn literal_dollar_and_gaps_preserved() {
        assert_eq!(rw("\\N:n", "$$${n}", "cost 5 dollars"), "cost $5 dollars");
    }

    #[test]
    fn unbound_capture_is_a_template_error() {
        let pat = parse("\\W:a").unwrap();
        let e = Template::parse("${b}", &pat.capture_names()).unwrap_err();
        assert!(e.msg.contains("binds no such capture"));
    }

    #[test]
    fn unknown_accessor_is_an_error() {
        let pat = parse("\\W:a").unwrap();
        let e = Template::parse("${a:shout}", &pat.capture_names()).unwrap_err();
        assert!(e.msg.contains("unknown accessor"));
    }

    #[test]
    fn ipv4_octet_slice() {
        // The user's case: capture an IP, keep the first two octets in one swoop.
        assert_eq!(rw("\\I:ip", "${ip:octet1-2}.0.0/16", "from 192.168.5.9"), "from 192.168.0.0/16");
        assert_eq!(rw("\\I:ip", "${ip:octet4}", "from 192.168.5.9"), "from 9");
    }

    #[test]
    fn ipv6_group_slice() {
        // The same \I atom matches IPv6, decomposed into groups, not octets.
        assert_eq!(
            rw("\\I:ip", "${ip:group1-3}", "addr 2001:db8:85a3:0:0:8a2e:370:7334"),
            "addr 2001:db8:85a3"
        );
    }

    #[test]
    fn url_email_version_fields() {
        assert_eq!(rw("\\U:u", "${u:host}", "get https://example.com:8080/a?q=1"), "get example.com");
        assert_eq!(rw("\\U:u", "${u:port}", "get https://example.com:8080/a"), "get 8080");
        assert_eq!(rw("\\E:e", "${e:user}@X", "to bob@x.com"), "to bob@X");
        assert_eq!(rw("\\V:v", "${v:major}", "v 1.2.3-rc1"), "v 1");
    }

    #[test]
    fn accessor_pipeline_chains() {
        assert_eq!(rw("\\E:e", "${e:domain|upper}", "to bob@x.com"), "to X.COM");
    }

    #[test]
    fn no_match_leaves_input_unchanged() {
        assert_eq!(rw("\\N", "X", "no digits here"), "no digits here");
    }

    #[test]
    fn parallel_splice_matches_serial_on_many_matches() {
        // Enough matches to cross the parallel threshold; the parallel
        // render must produce exactly the serial output.
        let mut input = String::new();
        for i in 0..5000 {
            input.push_str(&format!("row {i} val {} end\n", i * 3));
        }
        let pat = parse("\\W:k \\N:v").expect("pattern parses");
        let tpl = Template::parse("${k:upper}=${v}", &pat.capture_names()).expect("template");
        let matches = captures(&pat, input.as_bytes(), &scan(&pat, input.as_bytes()));
        let serial = splice(input.as_bytes(), &matches, &tpl);
        let parallel = splice_parallel(input.as_bytes(), &matches, &tpl);
        assert_eq!(serial, parallel);
        assert_eq!(rewrite(&pat, &tpl, input.as_bytes()), serial);
        assert!(
            serial.starts_with(b"ROW=0 VAL=0 end\nROW=1 VAL=3 end\n"),
            "{}",
            String::from_utf8_lossy(&serial[..32])
        );
    }
}
