//! Record-level queries: which records of an input hold all, any, none or
//! at least some number of a set of patterns, and none of the patterns they
//! exclude. The query is trex's own, the one the trex command's `scan
//! --all`, `--any`, `--none` and `--at-least` ask.

use pwrs::prelude::*;

use crate::atoms::{TrexLibrary, atoms_for};
use crate::axes::each_input;
use crate::common::{arg_err, units_of};
use crate::pattern::pattern_arg;

/// What a record is, as -Unit, -RecordStart and -RecordSpan say: a line
/// where none of them does.
pub(crate) fn record_unit(
    ps: &Pipeline<'_>,
    unit: &Option<String>,
    start: &Option<PsObject>,
    span: &Option<PsObject>,
    library: &Option<PsProxy<TrexLibrary>>,
) -> PsResult<trex::records::RecordUnit> {
    use trex::records::RecordUnit as U;
    match (unit, start, span) {
        (None, None, None) => Ok(U::Line),
        (Some(u), None, None) => U::parse(u).map_err(|e| arg_err("TrexRecordUnit", e)),
        (None, Some(p), None) => Ok(U::Start(pattern_arg(ps, p, library)?.inner)),
        (None, None, Some(p)) => Ok(U::Span(pattern_arg(ps, p, library)?.inner)),
        _ => Err(arg_err("TrexRecordUnit", "-Unit, -RecordStart and -RecordSpan each say what a record is; give one")),
    }
}

/// One record a query keeps.
#[psclass(name = "Trex.RecordHit")]
#[derive(Clone, Default)]
pub struct TrexRecordHit {
    /// The file the record is in, empty for a string.
    pub path: String,
    /// The line the record starts on, counting from 1.
    pub line_number: i64,
    /// Where the record starts in the input, in UTF-16 code units.
    pub start: i64,
    /// How many UTF-16 code units it spans.
    pub length: i64,
    /// The record's text.
    pub text: String,
    /// The patterns it holds, as they were given.
    pub patterns: Vec<String>,
}

/// Finds the records of an input that hold all, any, none or at least some
/// number of the patterns given, and none of the patterns -Not gives.
///
/// A record is a line unless -Unit names another unit: `paragraph`, `file`,
/// `period`, `seam`, `bind`, `auto`, `texture`, `shape`, `block` or `unit`,
/// as Get-TrexRecord reads them. -RecordStart makes a record the run from
/// one match of a pattern to the next, and -RecordSpan each match of one.
/// -Any is the rule when none is named. Every pattern is scanned once over
/// the whole input under the atoms in force.
///
/// # Examples
/// Find-TrexRecord '"error"', '"disk"' -Path ./app.log -All
/// Find-TrexRecord '"timeout"', '"refused"', '"reset"' -Path ./app.log -AtLeast 2 -Unit paragraph
/// Find-TrexRecord '\I' -Path ./access.log -Not '"200"'
/// Find-TrexRecord '"ERROR"' -Path ./app.log -RecordStart '\T' | Select-Object LineNumber, Text
#[cmdlet(verb = "Find", noun = "TrexRecord", alias = "Find-TxRecord", default_parameter_set = "Text", output = ["Trex.RecordHit"])]
#[derive(Default)]
pub struct FindTrexRecord {
    /// The patterns a record is held to: trex source text or Trex.Pattern
    /// objects.
    #[param(mandatory, position = 0)]
    pub pattern: Vec<PsObject>,
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 1, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// Keeps the records holding every pattern.
    #[param]
    pub all: bool,
    /// Keeps the records holding at least one pattern.
    #[param]
    pub any: bool,
    /// Keeps the records holding none of the patterns.
    #[param]
    pub none: bool,
    /// Keeps the records holding at least this many of the patterns.
    #[param]
    pub at_least: Option<u32>,
    /// Patterns a record must not hold.
    #[param]
    pub not: Vec<PsObject>,
    /// What a record is: `line` when absent.
    #[param]
    pub unit: Option<String>,
    /// A pattern whose every match starts a record that runs to the next.
    #[param]
    pub record_start: Option<PsObject>,
    /// A pattern whose every match is a record.
    #[param]
    pub record_span: Option<PsObject>,
    /// Writes at most this many records of each input.
    #[param]
    pub max_count: Option<u32>,
    /// The atoms the patterns are compiled against and scanned under, in
    /// place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    query: Option<trex::records::Query>,
    shapes: Option<trex::ShapeSet>,
    names: Vec<String>,
}

impl FindTrexRecord {
    fn quantifier(&self) -> PsResult<trex::records::Quantifier> {
        use trex::records::Quantifier as Q;
        let mut chosen = Vec::new();
        if self.all {
            chosen.push(("-All", Q::All));
        }
        if self.any {
            chosen.push(("-Any", Q::Any));
        }
        if self.none {
            chosen.push(("-None", Q::None));
        }
        if let Some(n) = self.at_least {
            chosen.push(("-AtLeast", Q::AtLeast(n as usize)));
        }
        match chosen.as_slice() {
            [] => Ok(Q::Any),
            [(_, q)] => Ok(*q),
            [(a, _), (b, _), ..] => {
                Err(arg_err("TrexQuantifier", format!("{a} and {b} are two rules; a query holds one")))
            }
        }
    }

    fn hits(&self, path: &str, text: &str) -> PsResult<Vec<TrexRecordHit>> {
        let (Some(query), Some(shapes)) = (&self.query, &self.shapes) else {
            return Err(arg_err("TrexNoQuery", "the query was not read"));
        };
        let bytes = text.as_bytes();
        let mut hits = query.hits(bytes, shapes, false);
        if let Some(n) = self.max_count {
            hits.truncate(n as usize);
        }
        let index = trex::files::LineIndex::new(bytes);
        let mut units = units_of(bytes);
        let mut out = Vec::with_capacity(hits.len());
        for hit in hits {
            let start = units.at(hit.start);
            let end = units.at(hit.end);
            out.push(TrexRecordHit {
                path: path.to_string(),
                line_number: index.line_of(hit.start) as i64 + 1,
                start: start as i64,
                length: (end - start) as i64,
                text: String::from_utf8_lossy(&bytes[hit.start..hit.end]).into_owned(),
                patterns: hit.present.iter().map(|&i| self.names[i].clone()).collect(),
            });
        }
        Ok(out)
    }
}

impl Cmdlet for FindTrexRecord {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let quantifier = self.quantifier()?;
        let unit = record_unit(ps, &self.unit, &self.record_start, &self.record_span, &self.library)?;
        let mut positives = Vec::with_capacity(self.pattern.len());
        for given in &self.pattern {
            let c = pattern_arg(ps, given, &self.library)?;
            self.names.push(c.source.clone());
            positives.push(c.inner);
        }
        let mut negatives = Vec::with_capacity(self.not.len());
        for given in &self.not {
            negatives.push(pattern_arg(ps, given, &self.library)?.inner);
        }
        self.query = Some(trex::records::Query { quantifier, positives, set: None, negatives, unit });
        self.shapes = Some(atoms_for(ps, &self.library)?);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| {
            ps.write(self.hits(&input.path, &input.text)?)
        })
    }
}
