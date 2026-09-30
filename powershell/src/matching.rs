//! Scanning text, files and directories for a pattern.
//!
//! Text piped in is scanned one string at a time, which is what composes
//! with the rest of a pipeline. A file or a directory is read and scanned
//! in one call per file, which is what makes a large input cheap: the
//! boundary is crossed once per file rather than once per line. A directory
//! is walked as the `trex` command walks one, with `.gitignore` and
//! `.ignore` rules applied and hidden and binary files skipped.

use pwrs::prelude::*;

use crate::atoms::{TrexLibrary, atoms_for};
use crate::common::{Units, arg_err, read_err, shape_err};
use crate::pattern::{Compiled, Engine, TrexMatch, explanation_of, pattern_arg};
use crate::query::record_unit;
use crate::structure::{RegionKind, region_kind};

/// The files `paths` name: a file as itself, a directory walked under
/// `opts`, a wildcard expanded by the provider first unless `literal`.
pub(crate) fn files_of(
    ps: &Pipeline<'_>,
    paths: &[String],
    literal: bool,
    opts: &trex::files::WalkOptions,
) -> PsResult<Vec<trex::files::Source>> {
    let mut resolved = Vec::new();
    for given in paths {
        resolved.extend(ps.resolve_path(given, literal)?);
    }
    let (sources, errors) = trex::files::collect(&resolved, opts);
    for e in errors {
        ps.write_error(&PsError::new(ErrorCategory::ReadError, "TrexWalk", e))?;
    }
    Ok(sources)
}

/// A file's text, decoded from whatever UTF encoding its byte-order mark
/// declares, or none for a binary file unless `binary` asks for those too.
pub(crate) fn read_text(path: &std::path::Path, binary: bool) -> Result<Option<String>, PsError> {
    Ok(read_whole(path, binary)?.map(|(_, text)| text))
}

/// A file's bytes and its text, as [`read_text`] reads it: the bytes are
/// what edits of the text are written back into.
pub(crate) fn read_whole(path: &std::path::Path, binary: bool) -> Result<Option<(Vec<u8>, String)>, PsError> {
    let shown = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|e| read_err(&shown, e))?;
    if !binary && trex::files::is_binary(&bytes) {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&trex::encoding::text_of(&bytes)).into_owned();
    Ok(Some((bytes, text)))
}

/// What a scan runs: one pattern, or a set of them whose every match is
/// tagged with the member that made it.
pub(crate) enum Scanning {
    One(Box<Compiled>),
    Set(Box<trex::PatternSet>),
}

impl Scanning {
    /// The patterns a cmdlet was given: one, several given one by one and
    /// named by their text, or the members of a pattern file under their
    /// names.
    pub(crate) fn of(
        ps: &Pipeline<'_>,
        patterns: &[PsObject],
        pattern_file: &Option<String>,
        library: &Option<PsProxy<TrexLibrary>>,
    ) -> PsResult<Self> {
        match (pattern_file, patterns) {
            (Some(_), [_, ..]) => {
                Err(arg_err("TrexPatternFile", "-Pattern and -PatternFile each give the patterns; give one"))
            }
            (Some(file), []) => {
                let mut resolved = ps.resolve_path(file, false)?;
                let Some(path) = resolved.pop() else {
                    return Err(arg_err("TrexPatternFile", format!("{file} names no file")));
                };
                let mut shapes = atoms_for(ps, library)?;
                let set = trex::PatternSet::from_file(std::path::Path::new(&path), &mut shapes)
                    .map_err(|e| shape_err(&e))?;
                Ok(Scanning::Set(Box::new(set)))
            }
            (None, []) => Err(arg_err("TrexNoPattern", "a pattern is required: give -Pattern or -PatternFile")),
            (None, [one]) => Ok(Scanning::One(Box::new(pattern_arg(ps, one, library)?))),
            (None, many) => {
                let mut pats = Vec::with_capacity(many.len());
                let mut names = Vec::with_capacity(many.len());
                for given in many {
                    let c = pattern_arg(ps, given, library)?;
                    names.push(c.source.clone());
                    pats.push(c.inner);
                }
                Ok(Scanning::Set(Box::new(trex::PatternSet::named(pats, names).under(atoms_for(ps, library)?))))
            }
        }
    }

    /// Every match, each with the member that made it where a set ran, in
    /// position order.
    pub(crate) fn found(&self, bytes: &[u8]) -> Vec<(Option<usize>, trex::Match)> {
        match self {
            Scanning::One(c) => c.found(bytes).into_iter().map(|m| (None, m)).collect(),
            Scanning::Set(set) => set.scan_matches(bytes, true).into_iter().map(|(i, m)| (Some(i), m)).collect(),
        }
    }

    /// Every match, the one pattern's scan run as `engine` says, beside a
    /// note of how it ran where there is one; a set scans over one lex, which
    /// takes no engine choice.
    pub(crate) fn found_by(&self, bytes: &[u8], engine: Engine) -> (Vec<(Option<usize>, trex::Match)>, Option<String>) {
        match self {
            Scanning::One(c) => {
                let (found, note) = c.found_by(bytes, engine);
                (found.into_iter().map(|m| (None, m)).collect(), note)
            }
            Scanning::Set(_) => (self.found(bytes), None),
        }
    }

    /// Each member's first match, in position order: the one pattern's
    /// first, or the first of each member of a set.
    pub(crate) fn first_found(&self, bytes: &[u8]) -> Vec<(Option<usize>, trex::Match)> {
        match self {
            Scanning::One(c) => c.found(bytes).into_iter().take(1).map(|m| (None, m)).collect(),
            Scanning::Set(set) => set.first_matches(bytes, true).into_iter().map(|(i, m)| (Some(i), m)).collect(),
        }
    }

    /// Whether anything matches.
    pub(crate) fn is_match(&self, bytes: &[u8]) -> bool {
        match self {
            Scanning::One(c) => c.is_match(bytes),
            Scanning::Set(set) => !set.first_matches(bytes, false).is_empty(),
        }
    }

    /// The pattern `member` is: the one pattern, or the set's member.
    pub(crate) fn pattern(&self, member: Option<usize>) -> &trex::ast::Pattern {
        match (self, member) {
            (Scanning::Set(set), Some(i)) => &set.patterns()[i],
            (Scanning::One(c), _) => &c.inner,
            (Scanning::Set(set), None) => &set.patterns()[0],
        }
    }

    /// The atoms the patterns are lexed under.
    pub(crate) fn shapes(&self) -> &trex::ShapeSet {
        match self {
            Scanning::One(c) => &c.shapes,
            Scanning::Set(set) => set.shapes(),
        }
    }

    /// The name a match's member goes by, empty for the one pattern.
    pub(crate) fn name(&self, member: Option<usize>) -> String {
        match (self, member) {
            (Scanning::Set(set), Some(i)) => set.name(i),
            _ => String::new(),
        }
    }

    /// The kind each register binds, in the order the patterns write them.
    /// Across a set, a register two members bind on different kinds has no
    /// one kind: its values would be read as one kind for matches of the
    /// other.
    pub(crate) fn capture_kinds(&self) -> Vec<(String, Option<trex::token::TokenKind>)> {
        match self {
            Scanning::One(c) => c.inner.capture_kinds(),
            Scanning::Set(set) => {
                let mut kinds: Vec<(String, Option<trex::token::TokenKind>)> = Vec::new();
                for p in set.patterns() {
                    for (name, kind) in p.capture_kinds() {
                        match kinds.iter_mut().find(|(n, _)| *n == name) {
                            Some(seen) if seen.1 != kind => seen.1 = None,
                            Some(_) => {}
                            None => kinds.push((name, kind)),
                        }
                    }
                }
                kinds
            }
        }
    }

    /// The register names a report template may write: every pattern's,
    /// each once.
    pub(crate) fn capture_names(&self) -> Vec<String> {
        match self {
            Scanning::One(c) => c.inner.capture_names(),
            Scanning::Set(set) => {
                let mut names: Vec<String> = Vec::new();
                for p in set.patterns() {
                    for name in p.capture_names() {
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
                names
            }
        }
    }
}

/// A line's significant extent, as offsets within it: from its first
/// character that is not whitespace to just past its last.
fn significant_span(line: &str) -> (usize, usize) {
    let start = line.len() - line.trim_start().len();
    (start, start + line.trim().len())
}

/// A line's text without the carriage return a CRLF file ends it with.
fn line_text(text: &str, (s, e): (usize, usize)) -> String {
    text[s..e].strip_suffix('\r').unwrap_or(&text[s..e]).to_string()
}

/// The zero-based lines no match touches.
fn untouched_lines<'m>(found: impl Iterator<Item = &'m trex::Match>, index: &trex::files::LineIndex) -> Vec<usize> {
    let mut touched = vec![false; index.lines()];
    for m in found {
        let first = index.line_of(m.start);
        let last = index.line_of(m.end.saturating_sub(1).max(m.start));
        for line in first..=last {
            if let Some(slot) = touched.get_mut(line) {
                *slot = true;
            }
        }
    }
    touched.iter().enumerate().filter(|(_, t)| !**t).map(|(line, _)| line).collect()
}

/// The matches of `found` that cover their line, from its first character
/// that is not whitespace to its last, what -WholeLine keeps.
fn whole_lines(
    text: &str,
    index: &trex::files::LineIndex,
    mut found: Vec<(Option<usize>, trex::Match)>,
) -> Vec<(Option<usize>, trex::Match)> {
    found.retain(|(_, m)| {
        let (s, e) = index.line_span(index.line_of(m.start));
        let (from, to) = significant_span(&text[s..e]);
        m.start == s + from && m.end == s + to
    });
    found
}

/// A whole line as a Trex.Match, for -NotMatch: its text, where it stands,
/// its number the one `index` gives, and no registers.
fn line_match(text: &str, units: &mut Units<'_>, index: &trex::files::LineIndex, line: usize) -> PsResult<TrexMatch> {
    let (s, e) = index.line_span(line);
    let shown = line_text(text, (s, e));
    Ok(TrexMatch {
        line_number: index.number(line) as i64,
        column: 1,
        start: units.at(s) as i64,
        length: shown.encode_utf16().count() as i64,
        text: shown,
        captures: std::collections::HashMap::<String, String>::new().into_ps()?,
        ..TrexMatch::default()
    })
}

/// A walked file's texture, where its text reads mostly as one.
fn texture_of(path: &std::path::Path) -> Option<RegionKind> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        // A file the read cannot open says nothing about its texture; the
        // scan that follows reports the read itself.
        Err(_unreadable) => return None,
    };
    trex::shape::dominant_kind(&trex::encoding::decode(bytes)).map(|kind| region_kind(kind).0)
}

/// The engine a scan runs on.
#[psenum(name = "Trex.Backend")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Backend {
    /// Chosen per input: the CPU engine, or for a pattern that reads only
    /// token kinds where a device is present, the input split between the
    /// cores and the device, by what this process has measured at the
    /// input's size.
    #[default]
    Auto,
    /// The CPU engine, never probing the device.
    Cpu,
    /// The device, falling back to the CPU where the pattern is outside what
    /// it runs, no device is present, or the build carries none.
    Gpu,
}

impl Backend {
    fn trex(self) -> trex::Backend {
        match self {
            Backend::Auto => trex::Backend::Auto,
            Backend::Cpu => trex::Backend::Cpu,
            Backend::Gpu => trex::Backend::Gpu,
        }
    }
}

/// The depth -Color and -Passthru paint at, as the trex command's `--color`
/// names it.
#[psenum(name = "Trex.ColorDepth")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorDepth {
    /// No color: the report's text alone, as `--color never` prints it.
    None,
    /// The sixteen base colors, as `--color 16` or `--color ansi` paints.
    Ansi16,
    /// The 256-color table, as `--color 256` or `--color ansi256` paints.
    Ansi256,
    /// Any 24-bit value, as `--color truecolor` paints.
    #[default]
    TrueColor,
}

impl ColorDepth {
    fn trex(self) -> trex::paint::Depth {
        match self {
            ColorDepth::None => trex::paint::Depth::Off,
            ColorDepth::Ansi16 => trex::paint::Depth::Ansi16,
            ColorDepth::Ansi256 => trex::paint::Depth::Ansi256,
            ColorDepth::TrueColor => trex::paint::Depth::TrueColor,
        }
    }
}

/// How -Json spells a typed register's value, as the trex command's
/// `--values` names it.
#[psenum(name = "Trex.ValueSpelling")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ValueSpelling {
    /// A JSON number only where a double holds the value exactly; a string
    /// or a small object everywhere else.
    #[default]
    Exact,
    /// A JSON number throughout, rounding where a double must.
    Natural,
    /// `{"kind": ..., "exact": ...}` for every value.
    Tagged,
}

impl ValueSpelling {
    fn trex(self) -> trex::typed::ValueSpelling {
        match self {
            ValueSpelling::Exact => trex::typed::ValueSpelling::Exact,
            ValueSpelling::Natural => trex::typed::ValueSpelling::Natural,
            ValueSpelling::Tagged => trex::typed::ValueSpelling::Tagged,
        }
    }
}

/// The unit -Json writes a duration's value in, as the trex command's
/// `--duration-unit` names it.
#[psenum(name = "Trex.DurationUnit")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DurationUnit {
    /// The nanoseconds a duration is held and compared in.
    #[default]
    Nanoseconds,
    /// Milliseconds, by an exact decimal shift.
    Milliseconds,
    /// Seconds, by an exact decimal shift: `1500ms` is `1.5`.
    Seconds,
}

impl DurationUnit {
    fn trex(self) -> trex::typed::DurationUnit {
        match self {
            DurationUnit::Nanoseconds => trex::typed::DurationUnit::Nanoseconds,
            DurationUnit::Milliseconds => trex::typed::DurationUnit::Milliseconds,
            DurationUnit::Seconds => trex::typed::DurationUnit::Seconds,
        }
    }
}

/// What -Sort orders the files read by.
#[psenum(name = "Trex.SortKey")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortKey {
    /// The path, as text.
    #[default]
    Path,
    /// The time the file was last written.
    Modified,
    /// The time the file was last read.
    Accessed,
    /// The time the file was created.
    Created,
}

impl SortKey {
    pub(crate) fn key(self) -> trex::files::SortKey {
        match self {
            SortKey::Path => trex::files::SortKey::Path,
            SortKey::Modified => trex::files::SortKey::Modified,
            SortKey::Accessed => trex::files::SortKey::Accessed,
            SortKey::Created => trex::files::SortKey::Created,
        }
    }
}

/// How many records of an input hold a match, or how many matches it holds.
#[psclass(name = "Trex.MatchCount")]
#[derive(Clone, Default)]
pub struct TrexMatchCount {
    /// The file counted, empty for text.
    pub path: String,
    /// The records holding a match, or under -NotMatch holding none; under
    /// -CountMatches, the matches.
    pub count: i64,
}

/// What a scan read and found, which -Stats writes after the report.
#[psclass(name = "Trex.ScanStats")]
#[derive(Clone, Default)]
pub struct TrexScanStats {
    /// The matches the report read.
    pub matches: i64,
    /// The lines those matches touch.
    pub matched_lines: i64,
    /// The inputs holding a match.
    pub files_with_matches: i64,
    /// The inputs scanned; each string piped in is one.
    pub files_searched: i64,
    /// The bytes scanned, counted as UTF-8.
    pub bytes_searched: i64,
    /// The time the scans took.
    pub searching: PsTimeSpan,
    /// The time from the cmdlet's start to its end.
    pub elapsed: PsTimeSpan,
    /// The tokens the scans' lexes produced.
    pub tokens_lexed: i64,
    /// The time those lexes took, summed over the cores that ran them, which
    /// comes out of Searching rather than adding to it.
    pub lexing: PsTimeSpan,
    /// Searching less Lexing: the time left for matching.
    pub matching: PsTimeSpan,
    /// The bytes handed to the device.
    pub device_bytes: i64,
    /// How many inputs each rung of the scan ladder answered, in the order
    /// first seen.
    pub routes: Vec<TrexScanRoute>,
}

/// How many inputs one rung of the scan ladder answered, which -Stats reads
/// from the trace trex keeps while the cmdlet runs.
#[psclass(name = "Trex.ScanRoute", show = "{Rung}: {Inputs}")]
#[derive(Clone, Default)]
pub struct TrexScanRoute {
    /// The rung, as the trex command's `--stats` names it.
    pub rung: String,
    /// How many inputs it answered.
    pub inputs: i64,
}

/// A duration as a `TimeSpan`, which counts hundreds of nanoseconds.
fn span(d: std::time::Duration) -> PsTimeSpan {
    PsTimeSpan::from_ticks((d.as_nanos() / 100).min(i64::MAX as u128) as i64)
}

/// What Select-TrexMatch writes for the matches of an input.
#[derive(Default)]
enum Written {
    /// A Trex.Match each.
    #[default]
    Matches,
    /// The matched text of each.
    Raw,
    /// A report template rendered at each.
    Format(trex::Template),
    /// Whether any input matched, once, after the last.
    Quiet,
    /// How many records hold one, per file or over all the text.
    Count,
    /// How many there are, per file or over all the text.
    CountMatches,
    /// The path of each file holding one.
    FilesWithMatches,
    /// The path of each file holding none.
    FilesWithoutMatch,
    /// The matches as the trex command's `--json` writes them.
    Json,
    /// The report as the trex command prints it, painted as -Color asks.
    Text,
    /// Every line, as the trex command's `--passthru` prints it.
    Passthru,
}

impl Written {
    /// Whether an input's answer is only whether it matched, which a scan
    /// stopping at its first match can give.
    fn only_whether(&self) -> bool {
        matches!(self, Written::Quiet | Written::FilesWithMatches | Written::FilesWithoutMatch)
    }

    /// Whether this sums each input up, as a count or as whether it matched,
    /// rather than writing its matches.
    fn summarizes(&self) -> bool {
        matches!(
            self,
            Written::Quiet
                | Written::Count
                | Written::CountMatches
                | Written::FilesWithMatches
                | Written::FilesWithoutMatch
        )
    }
}

/// What one input came to, which the answers written after the last input
/// add up.
#[derive(Default)]
struct Seen {
    /// Whether it holds a match, or under -NotMatch a record none touches.
    hit: bool,
    /// What -Count or -CountMatches counts in it.
    count: usize,
    /// The matches the report read.
    matches: usize,
    /// The lines those matches touch, counted only for -Stats.
    matched_lines: usize,
    /// Its size, as UTF-8.
    bytes: usize,
    /// The time its scan took.
    searching: std::time::Duration,
    /// Its matches as the JSON objects of a -Json report that names its
    /// inputs, which is written whole after the last.
    json: Vec<String>,
}

/// What every input so far came to.
#[derive(Default)]
struct Totals {
    hit: bool,
    count: usize,
    matches: u64,
    matched_lines: u64,
    files_with_matches: u64,
    files_searched: u64,
    bytes_searched: u64,
    searching: std::time::Duration,
}

impl Totals {
    fn add(&mut self, seen: &Seen) {
        self.hit |= seen.hit;
        self.count += seen.count;
        self.matches += seen.matches as u64;
        self.matched_lines += seen.matched_lines as u64;
        self.files_with_matches += u64::from(seen.matches > 0);
        self.files_searched += 1;
        self.bytes_searched += seen.bytes as u64;
        self.searching += seen.searching;
    }
}

/// The one input strings piped into Select-TrexMatch make, each string a
/// line of it.
enum Joined {
    /// Scanned as the strings arrive, each match written once nothing that
    /// arrives later can change it.
    Streamed(Box<crate::follow::FollowedScan>),
    /// Gathered whole, for a report read over the whole input once it ends.
    Gathered(String),
}

/// What -Context adds around a match: a count of lines on each side, or on
/// a side that names a unit, the rest of the record of that unit holding
/// the match.
#[derive(Default)]
struct Around {
    before: usize,
    after: usize,
    /// The unit the context names, with whether the side before the match
    /// and the side after it name it.
    unit: Option<(trex::records::RecordUnit, bool, bool)>,
}

impl Around {
    /// Whether -Context asked for anything.
    fn any(&self) -> bool {
        self.before > 0 || self.after > 0 || self.unit.is_some()
    }

    /// The zero-based lines from `from` up to `to` that a match on line
    /// `line` spanning `span` is shown among: each side a count of lines, or
    /// the rest of the innermost record holding the match. A match inside
    /// no record of the unit adds nothing on that side.
    fn lines(
        &self,
        index: &trex::files::LineIndex,
        records: Option<&[(usize, usize)]>,
        line: usize,
        span: (usize, usize),
    ) -> (usize, usize) {
        let mut from = line.saturating_sub(self.before);
        let mut to = (line + 1 + self.after).min(index.lines());
        if let (Some((_, before, after)), Some(records)) = (&self.unit, records) {
            let holding = records
                .iter()
                .filter(|&&(s, e)| s <= span.0 && span.1 <= e)
                .max_by_key(|&&(s, e)| (s, std::cmp::Reverse(e)));
            let (first, last) = match holding {
                Some(&(s, e)) => (index.line_of(s), (index.line_of(e.saturating_sub(1).max(s)) + 1).min(index.lines())),
                None => (line, line + 1),
            };
            if *before {
                from = first.min(line);
            }
            if *after {
                to = last.max(line + 1);
            }
        }
        (from, to)
    }
}

/// How many records of `unit` a match touches, or under `invert` how many
/// none does, counting to `cap` at most.
///
/// The records and the matches both ascend, so one walk settles every
/// record. A match wider than a record is left in place rather than passed,
/// because it holds the records after it too.
fn records_touched(
    unit: &trex::records::RecordUnit,
    input: &[u8],
    found: &[(Option<usize>, trex::Match)],
    invert: bool,
    cap: usize,
) -> usize {
    let mut held = 0usize;
    let mut at = 0usize;
    for (start, end) in unit.records(input) {
        if held >= cap {
            break;
        }
        while at < found.len() && found[at].1.end <= start {
            at += 1;
        }
        let touched = at < found.len() && found[at].1.start < end;
        if touched != invert {
            held += 1;
        }
    }
    held
}

/// Finds every match of a trex pattern in text, files or directories and
/// writes each as a Trex.Match.
///
/// A pattern is trex source text or a Trex.Pattern from New-TrexPattern; a
/// source text is compiled against the session's atoms, or -Library's.
/// Strings piped in are the lines of one input, as Get-Content writes a
/// file's, each ending a line unless it ends with a newline of its own: a
/// match may run from one string into the next, LineNumber counts across
/// them, and -Context reads the strings around a match. Where the
/// report writes the matches alone, each is written once the text holding it
/// has arrived; a report read over the whole input, such as -Context,
/// -NotMatch or -Count, is written after the last string. -PerString scans
/// each string as an input of its own. -Path reads each file whole and scans
/// it in one call, so a large input crosses into trex once per file; a
/// directory is walked with .gitignore and .ignore rules, skipping hidden and
/// binary files, as the trex command walks one.
///
/// -Context adds the lines around each match, or the rest of the paragraph,
/// block or other record a unit names; -NotMatch writes the lines no match
/// touches instead, and -WholeLine keeps a match only where it covers its
/// line. -FileType and -Texture keep the walked files of a type or a
/// texture, and -Sort orders the files read.
///
/// In place of the matches, -Raw writes their text and -Format a report
/// template rendered at each, which reads `${path}`, `${line}` and `${col}`
/// beside the registers, and `${@axis}` for what -Explain reads. -Count
/// writes how many lines of each file hold a match and -CountMatches how
/// many matches it holds, each as a Trex.MatchCount for a file with any, and
/// over text as one count of every string, after the last. -FilesWithMatches
/// and -FilesWithoutMatch write the path of each file holding a match or
/// none, and -Quiet one boolean. -Stats adds a Trex.ScanStats after the
/// report.
///
/// -Json, -Color and -Passthru write what the trex command prints for the
/// same scan, a string a line: -Json its `--json` report, -Color its text
/// report painted as `--color` paints it, and -Passthru every line with the
/// matches painted, as `--passthru` prints it to a console. A report names
/// its inputs as the command's does, over a directory, several paths or
/// paths piped in.
///
/// Several patterns scan as one set, each match naming the one that made it
/// in its Pattern property: patterns given one by one are named by their
/// text, and the members of a -PatternFile by the names the file gives,
/// `let name = pattern`, or by their line numbers. -SingleMatch keeps each
/// member's first match. -RequireMatch writes an error when no input held
/// a match, for a script that stops on one.
///
/// One pattern's scan runs where -Backend says, the CPU engine or the
/// device, or with -DualGrain as the byte and token grains in a pipeline,
/// or with -ChunkSize over each input fed in chunks; every choice finds the
/// same matches, and how the scan ran is written as verbose output.
///
/// # Examples
/// Select-TrexMatch '\E:e' -InputObject 'ping bob@x.com'
/// 'ping bob@x.com', 'none here' | Select-TrexMatch '\E:e'
/// Select-TrexMatch '\N{>=1000}' -Path ./logs -Context 2
/// Select-TrexMatch '"panic"' -Path ./logs -Context record -RecordStart '\T'
/// Select-TrexMatch '\I:ip' -Path ./logs -Format '${path}:${line}: ${ip:octet1-2}'
/// Select-TrexMatch '<\W:t>.*</=t>' -Path page.html | ForEach-Object { $_.Captures.t }
/// Select-TrexMatch '"#"' -Path ./app.conf -NotMatch
/// Select-TrexMatch '\E', '\I' -Path ./logs | Group-Object Pattern
/// Select-TrexMatch -PatternFile ./secrets.trex -Path ./src
/// Select-TrexMatch '"ERROR"' -Path ./logs -Count | Sort-Object Count -Descending
/// Select-TrexMatch '\E' -Path ./src -FilesWithMatches -Sort Modified -Descending
/// Get-Content ./app.log | Select-TrexMatch '\T{age<1h}' -CountMatches
/// Select-TrexMatch '\I:ip' -Path ./logs -Json | Set-Content ./matches.json
/// Select-TrexMatch '"ERROR"' -Path ./logs -Color -Context 2
/// Select-TrexMatch '\N{>=500}' -Path ./app.log -Passthru -Color
#[cmdlet(verb = "Select", noun = "TrexMatch", alias = "Select-TxMatch", default_parameter_set = "Text", supports_should_process, output = ["Trex.Match", "System.String", "System.Boolean", "Trex.MatchCount", "Trex.ScanStats"])]
#[derive(Default)]
pub struct SelectTrexMatch {
    /// The patterns: trex source text or Trex.Pattern objects; several scan
    /// as one set.
    #[param(position = 0)]
    pub pattern: Vec<PsObject>,
    /// A pattern file whose members scan as one set: each `let name =
    /// pattern` line under its name and each bare pattern line under its
    /// line number, with the file's declarations in force. Text to scan
    /// beside it is piped in or named with -InputObject, since a first
    /// argument by position is read as -Pattern.
    #[param]
    pub pattern_file: Option<String>,
    /// The text to scan. Strings piped in are the lines of one input, as
    /// Get-Content writes a file's, unless -PerString is given.
    #[param(mandatory, position = 1, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Scans each string piped in as an input of its own, rather than as
    /// the next line of one input.
    #[param(set = "Text")]
    pub per_string: bool,
    /// Files or directories to scan; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to scan, read as written, as Get-ChildItem
    /// pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The atoms a source-text pattern is compiled against, in place of the
    /// session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    /// Writes only the first match of each input.
    #[param]
    pub list: bool,
    /// Writes the matched text rather than match objects.
    #[param]
    pub raw: bool,
    /// Writes only whether any input matched, once, after every input.
    #[param]
    pub quiet: bool,
    /// The lines to add around each match: one count for both sides, or two,
    /// the lines before and the lines after. A record unit in place of a
    /// count, such as `paragraph` or `block`, adds the rest of the record
    /// holding the match on that side, and `record` names the one -Unit,
    /// -RecordStart or -RecordSpan defines.
    #[param]
    pub context: Vec<String>,
    /// Writes the lines no match touches, each as a Trex.Match of the whole
    /// line with no registers.
    #[param]
    pub not_match: bool,
    /// Keeps a match only where it covers its line, from the first character
    /// that is not whitespace to the last.
    #[param]
    pub whole_line: bool,
    /// Writes or counts at most this many matches, or lines under -NotMatch,
    /// of each input.
    #[param]
    pub max_count: Option<u32>,
    /// A report template to write at each match in place of the match:
    /// `${name}` a register, `${path}`, `${line}` and `${col}` where it
    /// stands, `${@axis}` an axis the pattern read.
    #[param]
    pub format: Option<String>,
    /// Writes the matches as the trex command's `--json` writes them: an
    /// array for the text -InputObject gives or a string under -PerString;
    /// over strings piped in, each match's object as it settles, as the
    /// command writes a stream on its standard input, or one array after the
    /// last string where a flag reads the whole input; over a directory,
    /// several paths or paths piped in, one array of every match after the
    /// last input, each object naming its path, line and column.
    #[param]
    pub json: bool,
    /// How -Json spells a typed register's value, as the trex command's
    /// `--values` takes it: Exact when absent, a JSON number only where a
    /// double holds the value; Natural, a number throughout; or Tagged, its
    /// kind and its exact text.
    #[param]
    pub value_spelling: Option<ValueSpelling>,
    /// The unit -Json writes a duration in, as the trex command's
    /// `--duration-unit` takes it: Nanoseconds when absent, Milliseconds or
    /// Seconds.
    #[param]
    pub duration_unit: Option<DurationUnit>,
    /// Writes the report as the trex command prints it, a line a string,
    /// painted: each match's span and text, or with its path, line and
    /// column over a directory, several paths or paths piped in; -Context,
    /// -Explain and -NotMatch add their lines as the command's flags do.
    #[param]
    pub color: bool,
    /// The depth -Color and -Passthru paint at, as the trex command's
    /// `--color` names it: None, which paints nothing, as `--color never`;
    /// Ansi16, Ansi256 or TrueColor. What the environment says when absent,
    /// as `--color always` reads it.
    #[param]
    pub color_depth: Option<ColorDepth>,
    /// A role's paint for -Color and -Passthru, as the trex command's
    /// `--colors` takes it:
    /// `match:fg:red`, `path:bg:#202020`, `line:style:bold`,
    /// `kind:NAME:...` for a token kind, `capture:NAME:...` for a register.
    #[param]
    pub colors: Vec<String>,
    /// Writes every line of each input as the trex command's `--passthru`
    /// prints it, a line a string, the matches painted: where the report
    /// names its inputs, with its path and line number ahead of it, joined by
    /// `:` for a line holding a match and by `-` for the rest.
    /// -ColorDepth None writes the lines unpainted.
    #[param]
    pub passthru: bool,
    /// Writes how many records of each file hold a match, or under
    /// -NotMatch hold none, as a Trex.MatchCount for each file with any; over
    /// text, one count of every string, after the last. A record is a line
    /// unless -Unit, -RecordStart or -RecordSpan says otherwise.
    #[param]
    pub count: bool,
    /// Writes how many matches each file holds, as -Count writes the records
    /// holding one; under -NotMatch it counts the records none touches.
    #[param]
    pub count_matches: bool,
    /// Writes the path of each file holding a match, or under -NotMatch a
    /// record no match touches.
    #[param(set = ["Path", "LiteralPath"])]
    pub files_with_matches: bool,
    /// Writes the path of each file holding no match, or under -NotMatch
    /// each file whose every record holds one.
    #[param(set = ["Path", "LiteralPath"])]
    pub files_without_match: bool,
    /// What a record is for -Count and -Context record, and what -Head, -Tail
    /// and -Lines count: a line when absent, or a unit Find-TrexRecord reads,
    /// such as `paragraph`, `file` or `block`.
    #[param]
    pub unit: Option<String>,
    /// A pattern whose every match starts a record that runs to the next,
    /// for -Count and -Context record.
    #[param]
    pub record_start: Option<PsObject>,
    /// A pattern whose every match is a record, for -Count and -Context
    /// record.
    #[param]
    pub record_span: Option<PsObject>,
    /// Writes a Trex.ScanStats after the report: the matches, the lines they
    /// touch, the inputs scanned and those holding a match, the bytes
    /// scanned and the time taken.
    #[param]
    pub stats: bool,
    /// Keeps the first match of each pattern of a set in each input, where
    /// several patterns or -PatternFile give one.
    #[param]
    pub single_match: bool,
    /// Writes an error after the last input when none held a match, or
    /// under -NotMatch when every line of every input held one.
    #[param]
    pub require_match: bool,
    /// The engine one pattern's scan runs on: Auto when absent, Cpu, or
    /// Gpu, which falls back to the CPU with a note where the device cannot
    /// take the scan.
    #[param]
    pub backend: Option<Backend>,
    /// Runs one pattern's scan as the byte grain and the token grain in a
    /// pipeline, writing their timing as verbose output.
    #[param]
    pub dual_grain: bool,
    /// Feeds each input to one pattern's scan in chunks of this many bytes,
    /// as a stream arrives.
    #[param]
    pub chunk_size: Option<u32>,
    /// Adds what each match is made of: its tokens' kinds, the checks its
    /// guarded kinds passed, and every axis the pattern read at it.
    #[param]
    pub explain: bool,
    /// Reads hidden files and directories a walk would skip.
    #[param(set = ["Path", "LiteralPath"])]
    pub hidden: bool,
    /// Reads files an ignore rule excludes.
    #[param(set = ["Path", "LiteralPath"])]
    pub no_ignore: bool,
    /// Reads files that hold a NUL byte, which a walk treats as binary.
    #[param(set = ["Path", "LiteralPath"])]
    pub binary: bool,
    /// Keeps a walked file only when a glob matches it (`*.log`), or drops
    /// it for a glob that starts with `!`.
    #[param(set = ["Path", "LiteralPath"])]
    pub include: Vec<String>,
    /// Keeps a walked file only when it is of one of these types, under
    /// ripgrep's names: `rust`, `py`, `js`, `log` and the rest.
    #[param(set = ["Path", "LiteralPath"])]
    pub file_type: Vec<String>,
    /// Drops a walked file of one of these types.
    #[param(set = ["Path", "LiteralPath"])]
    pub exclude_file_type: Vec<String>,
    /// Orders the files read: by path, or by the time each was last written,
    /// read or created, oldest first.
    #[param(set = ["Path", "LiteralPath"])]
    pub sort: Option<SortKey>,
    /// Reverses the order -Sort names.
    #[param(set = ["Path", "LiteralPath"])]
    pub descending: bool,
    /// Keeps a walked file only when its text reads mostly as one of these.
    #[param(set = ["Path", "LiteralPath"])]
    pub texture: Vec<RegionKind>,
    /// Drops a walked file whose text reads mostly as one of these.
    #[param(set = ["Path", "LiteralPath"])]
    pub exclude_texture: Vec<RegionKind>,
    /// Writes each directory's index from the files this scan reads, so
    /// every later scan of the tree opens only the files that can match.
    #[param(set = ["Path", "LiteralPath"])]
    pub index: bool,
    /// Reads no index, whatever the trees hold.
    #[param(set = ["Path", "LiteralPath"])]
    pub no_index: bool,
    /// Scans the first this many lines of each input, or records of -Unit,
    /// reading a file no further.
    #[param(alias = ["First"])]
    pub head: Option<u32>,
    /// Scans the last this many lines of each input, or records of -Unit,
    /// reading a file backward from its end.
    #[param(alias = ["Last"])]
    pub tail: Option<u32>,
    /// Scans a range of lines of each input, or records of -Unit, counted
    /// from one: `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's
    /// `100..200`. A match counts only where it lies wholly inside the part
    /// read, and every match stands at the input's own line and offset.
    #[param]
    pub lines: Option<PsObject>,
    /// After each file's -Tail, its open -Lines range or the whole of it,
    /// scans what it gains as it grows, writing each match once nothing that
    /// arrives later can change it, until the pipeline is stopped.
    #[param(alias = ["Wait"], set = ["Path", "LiteralPath"])]
    pub follow: bool,
    window: Option<trex::window::Select>,
    followed: Vec<crate::follow::FollowedScan>,
    scanning: Option<Scanning>,
    written: Written,
    engine: Engine,
    records: Option<trex::records::RecordUnit>,
    around: Around,
    totals: Totals,
    started: Option<std::time::Instant>,
    recording: Option<trex::trace::Recording>,
    painter: Option<trex::paint::Painter>,
    piped: bool,
    prefixed: bool,
    json_objects: Vec<String>,
    joined: Option<Joined>,
}

impl SelectTrexMatch {
    /// Whether what is written reads each match's explanation: -Explain, or
    /// a -Format template naming an axis the explanation reads.
    fn reads_why(&self) -> bool {
        self.explain || matches!(&self.written, Written::Format(t) if t.reads_explanation())
    }

    /// Whether `read`, the part read of the input at `path`, is an empty
    /// string whose one line the window reaches: a string is a line even when
    /// it is empty, as each line Get-Content yields is, and an empty file has
    /// none.
    fn blank_string(&self, read: &crate::window::Read, path: &str) -> bool {
        if !path.is_empty() || read.input_len != 0 {
            return false;
        }
        let unit = match &self.records {
            Some(unit) => unit.clone(),
            None => trex::records::RecordUnit::Line,
        };
        self.window.is_none_or(|w| !trex::window::range_of(b"\n", w, &unit).is_empty())
    }

    /// Whether the matches of strings piped in are written as they settle,
    /// where the trex command streams its standard input: one scan on the
    /// CPU engine, writing the matches alone, with no window, context, grep
    /// flag, statistics or explanation, each of which reads the whole input.
    fn streams(&self) -> bool {
        self.engine.is_plain()
            && matches!(self.written, Written::Matches | Written::Raw | Written::Format(_) | Written::Json | Written::Text)
            && self.window.is_none()
            && !self.around.any()
            && !self.not_match
            && !self.whole_line
            && self.max_count.is_none()
            && !self.list
            && !self.single_match
            && !self.stats
            && !self.reads_why()
    }

    /// Reads the string piped in as the next line of the one input the
    /// strings make: scanned now, writing the matches it settles, where the
    /// report streams, and gathered for the end where it does not.
    fn join(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(sc) = &self.scanning else {
            return Ok(());
        };
        let mut joined = match self.joined.take() {
            Some(joined) => joined,
            None if self.streams() => {
                let empty = crate::window::Read::of_text("", None, &trex::records::RecordUnit::Line);
                Joined::Streamed(Box::new(crate::follow::FollowedScan::new(
                    std::path::PathBuf::new(),
                    String::new(),
                    &empty,
                    sc,
                )?))
            }
            None => Joined::Gathered(String::new()),
        };
        // Each string ends a line, where it does not end with a newline of
        // its own.
        let ended = self.input_object.ends_with('\n');
        match &mut joined {
            Joined::Streamed(stream) => {
                let line = if ended { self.input_object.clone() } else { format!("{}\n", self.input_object) };
                if let Some(batch) = stream.push_text(&line, sc) {
                    self.write_batch(ps, stream, batch)?;
                }
            }
            Joined::Gathered(text) => {
                text.push_str(&self.input_object);
                if !ended {
                    text.push('\n');
                }
            }
        }
        self.joined = Some(joined);
        Ok(())
    }

    /// Ends the one input strings piped in made: the matches its stream held
    /// until the end, or the whole of it reported at once.
    fn end_joined(&mut self, ps: &Pipeline<'_>, joined: Joined) -> PsResult<()> {
        match joined {
            Joined::Streamed(mut stream) => {
                let Some(sc) = &self.scanning else {
                    return Ok(());
                };
                if let Some(batch) = stream.finish(sc) {
                    self.write_batch(ps, &mut stream, batch)?;
                }
                self.totals.hit |= stream.written > 0;
            }
            Joined::Gathered(text) => {
                let unit = match &self.records {
                    Some(unit) => unit.clone(),
                    None => trex::records::RecordUnit::Line,
                };
                let read = crate::window::Read::of_text(&text, self.window, &unit);
                let seen = self.report(ps, &read, "")?;
                self.totals.add(&seen);
            }
        }
        Ok(())
    }

    /// The painter the report is written with, as the trex command builds
    /// one: at the depth -ColorDepth names, the console readied for its codes,
    /// or what the environment says as `--color always` reads it; the kinds
    /// painted as `TREX_KIND_COLOR` asks, then each -Colors spec in order.
    fn painter_of(&self) -> PsResult<trex::paint::Painter> {
        use trex::paint::{Depth, KindPaint, Painter, Palette};
        let mut palette = Palette::base();
        let depth = match self.color_depth {
            Some(ColorDepth::None) => Depth::Off,
            Some(asked) => {
                trex::paint::console_ready();
                asked.trex()
            }
            None => Depth::forced(),
        };
        match std::env::var("TREX_KIND_COLOR") {
            Ok(level) => {
                let Some(level) = KindPaint::parse(&level) else {
                    return Err(arg_err(
                        "TrexColor",
                        format!("TREX_KIND_COLOR takes none, values or all, not {level:?}"),
                    ));
                };
                palette.set_kind_paint(level);
            }
            Err(std::env::VarError::NotPresent) => {}
            Err(std::env::VarError::NotUnicode(raw)) => {
                return Err(arg_err("TrexColor", format!("TREX_KIND_COLOR is set to something that is not text: {raw:?}")));
            }
        }
        for spec in &self.colors {
            palette.set(spec).map_err(|e| arg_err("TrexColor", format!("-Colors: {e}")))?;
        }
        Ok(Painter::new(depth, palette))
    }

    /// Writes one input's report as the trex command prints it: under -Json
    /// its array, or where the report names its inputs its objects, kept for
    /// the array written after the last input; under -Color its text report;
    /// under -Passthru every line.
    // The eight are one input's report: the pipeline, the input and its name,
    // what the scan found, the input's line index, the route its scan took,
    // and what it came to.
    #[allow(clippy::too_many_arguments)]
    fn cli_report(
        &self,
        ps: &Pipeline<'_>,
        text: &str,
        path: &str,
        found: &[(Option<usize>, trex::Match)],
        index: &trex::files::LineIndex,
        route: &str,
        seen: &mut Seen,
    ) -> PsResult<()> {
        let Some(sc) = &self.scanning else {
            return Ok(());
        };
        let bytes = text.as_bytes();
        let matches: Vec<trex::Match> = found.iter().map(|(_, m)| m.clone()).collect();
        let of: Vec<usize> = found.iter().map(|(member, _)| member.unwrap_or(0)).collect();
        let (members, count) = match sc {
            Scanning::Set(set) => (Some(trex::report::Members { set: set.as_ref(), of: &of }), set.len()),
            Scanning::One(_) => (None, 1),
        };
        let explaining = self
            .explain
            .then(|| trex::report::Explaining::over(count, |i| sc.pattern(Some(i)), bytes, sc.shapes(), members, route));
        let prefix = (self.prefixed && !path.is_empty()).then_some(path);
        let max_count = self.max_count.map(|n| n as usize);
        let mut lines: Vec<String> = Vec::new();
        let mut sink = |line: &str| lines.push(line.to_string());
        let plain = trex::paint::Painter::new(trex::paint::Depth::Off, trex::paint::Palette::base());
        let painter = self.painter.as_ref().unwrap_or(&plain);
        match &self.written {
            Written::Json => {
                let kinds = sc.capture_kinds();
                let values = trex::report::ValueView {
                    kinds: &kinds,
                    style: trex::typed::ValueStyle {
                        spelling: self.value_spelling.unwrap_or(ValueSpelling::Exact).trex(),
                        duration: self.duration_unit.unwrap_or(DurationUnit::Nanoseconds).trex(),
                    },
                    clock: trex::Clock::current(),
                };
                match prefix {
                    Some(p) => {
                        for (k, m) in matches.iter().enumerate() {
                            let (line, col) = index.line_col(bytes, m.start);
                            let extra = trex::report::json_extras(m, members, k, explaining.as_ref());
                            seen.json.push(trex::report::match_json(
                                bytes,
                                m,
                                Some((p, line, col)),
                                index.offset(0),
                                extra.as_deref(),
                                Some(&values),
                            ));
                        }
                    }
                    None => sink(&trex::report::json_report(
                        bytes,
                        &matches,
                        index,
                        members,
                        explaining.as_ref(),
                        Some(&values),
                    )),
                }
            }
            Written::Passthru => trex::report::lines_report(prefix, bytes, &matches, index, false, max_count, painter, &mut sink),
            Written::Text if self.not_match => {
                trex::report::lines_report(prefix, bytes, &matches, index, true, max_count, painter, &mut sink);
            }
            Written::Text if prefix.is_some() || self.around.any() => {
                let context = trex::report::Context {
                    before: self.around.before,
                    after: self.around.after,
                    record: self.around.unit.as_ref().map(|(unit, before, after)| (unit, *before, *after)),
                };
                trex::report::context_report(
                    prefix,
                    bytes,
                    &matches,
                    index,
                    &context,
                    painter,
                    members,
                    explaining.as_ref(),
                    &mut sink,
                );
            }
            Written::Text => {
                trex::report::human_report(bytes, &matches, index, painter, members, explaining.as_ref(), &mut sink);
            }
            Written::Matches
            | Written::Raw
            | Written::Format(_)
            | Written::Quiet
            | Written::Count
            | Written::CountMatches
            | Written::FilesWithMatches
            | Written::FilesWithoutMatch => {}
        }
        for line in lines {
            ps.write(line)?;
        }
        Ok(())
    }

    /// How many matches or lines of one input are written or counted.
    fn cap(&self) -> usize {
        let first = if self.list || self.quiet { Some(1) } else { None };
        match (first, self.max_count) {
            (Some(a), Some(b)) => a.min(b as usize),
            (Some(a), None) => a,
            (None, Some(b)) => b as usize,
            (None, None) => usize::MAX,
        }
    }

    /// The lines -Context asks for around a match on zero-based line `line`
    /// spanning `span`, read from `records` where it names a unit.
    fn context_lines(
        &self,
        text: &str,
        index: &trex::files::LineIndex,
        records: Option<&[(usize, usize)]>,
        line: usize,
        span: (usize, usize),
    ) -> (Vec<String>, Vec<String>) {
        let (from, to) = self.around.lines(index, records, line, span);
        let pre = (from..line).map(|l| line_text(text, index.line_span(l))).collect();
        let post = (line + 1..to).map(|l| line_text(text, index.line_span(l))).collect();
        (pre, post)
    }

    /// What -Context asks for: each side a count, or a record unit, one unit
    /// at most; `record` is the unit -Unit, -RecordStart or -RecordSpan
    /// defines, given here as `defined`.
    fn around_asked(&self, defined: Option<&trex::records::RecordUnit>) -> PsResult<Around> {
        let (before, after) = match self.context.as_slice() {
            [] => return Ok(Around::default()),
            [both] => (both, both),
            [before, after] => (before, after),
            _ => {
                return Err(arg_err(
                    "TrexContext",
                    "-Context takes one count or unit for both sides, or two: before and after",
                ));
            }
        };
        let mut around = Around::default();
        let mut named: Option<&str> = None;
        for (side, given) in [(true, before), (false, after)] {
            let given = given.trim();
            if !given.is_empty() && given.bytes().all(|b| b.is_ascii_digit()) {
                let n: usize = given
                    .parse()
                    .map_err(|e| arg_err("TrexContext", format!("-Context {given}: {e}")))?;
                if side {
                    around.before = n;
                } else {
                    around.after = n;
                }
                continue;
            }
            match named {
                Some(earlier) if earlier != given => {
                    return Err(arg_err(
                        "TrexContext",
                        format!("-Context names one unit; {earlier:?} and {given:?} were both given"),
                    ));
                }
                Some(_) => {}
                None => {
                    let unit = match (given, defined) {
                        ("record", Some(unit)) => unit.clone(),
                        ("record", None) => {
                            return Err(arg_err(
                                "TrexContext",
                                "-Context record names the record -Unit, -RecordStart or -RecordSpan defines; give one",
                            ));
                        }
                        (other, _) => trex::records::RecordUnit::parse(other)
                            .map_err(|e| arg_err("TrexContext", format!("-Context: {e}")))?,
                    };
                    around.unit = Some((unit, false, false));
                    named = Some(given);
                }
            }
            if let Some((_, b, a)) = &mut around.unit {
                if side {
                    *b = true;
                } else {
                    *a = true;
                }
            }
        }
        Ok(around)
    }

    /// What the switches ask to be written in place of the matches, refusing
    /// two that each say it.
    fn written(&self, sc: &Scanning) -> PsResult<Written> {
        let mut asked: Vec<(&str, Written)> = Vec::new();
        if self.raw {
            asked.push(("-Raw", Written::Raw));
        }
        if let Some(src) = &self.format {
            let t = trex::Template::parse_report(src, &sc.capture_names())
                .map_err(|e| arg_err("TrexTemplateError", format!("-Format error at byte {}: {}", e.pos, e.msg)))?;
            asked.push(("-Format", Written::Format(t)));
        }
        if self.quiet {
            asked.push(("-Quiet", Written::Quiet));
        }
        if self.count {
            asked.push(("-Count", Written::Count));
        }
        if self.count_matches {
            asked.push(("-CountMatches", Written::CountMatches));
        }
        if self.files_with_matches {
            asked.push(("-FilesWithMatches", Written::FilesWithMatches));
        }
        if self.files_without_match {
            asked.push(("-FilesWithoutMatch", Written::FilesWithoutMatch));
        }
        if self.json {
            asked.push(("-Json", Written::Json));
        }
        if self.passthru {
            asked.push(("-Passthru", Written::Passthru));
        } else if self.color {
            asked.push(("-Color", Written::Text));
        }
        let mut asked = asked.into_iter();
        match (asked.next(), asked.next()) {
            (None, _) => Ok(Written::Matches),
            (Some((_, one)), None) => Ok(one),
            (Some((a, _)), Some((b, _))) => {
                Err(arg_err("TrexReport", format!("{a} and {b} each say what is written; give one")))
            }
        }
    }

    /// Writes what a report that sums each input up says of one: its count
    /// for a file, or its path where it is listed. A count of text is
    /// written after the last string, as one.
    fn sum_up(&self, ps: &Pipeline<'_>, path: &str, seen: &Seen) -> PsResult<()> {
        match self.written {
            Written::Count | Written::CountMatches if !path.is_empty() && seen.count > 0 => {
                ps.write(TrexMatchCount { path: path.to_string(), count: seen.count as i64 })
            }
            Written::FilesWithMatches if seen.hit => ps.write(path.to_string()),
            Written::FilesWithoutMatch if !seen.hit => ps.write(path.to_string()),
            _ => Ok(()),
        }
    }

    /// Scans one input, the part of it `read` holds, and writes what it comes
    /// to, answering what the answers after the last input add up; `path` is
    /// empty for a string.
    fn report(&self, ps: &Pipeline<'_>, read: &crate::window::Read, path: &str) -> PsResult<Seen> {
        let Some(sc) = &self.scanning else {
            return Ok(Seen::default());
        };
        let text = read.text.as_str();
        let bytes = text.as_bytes();
        let mut seen = Seen { bytes: bytes.len(), ..Seen::default() };
        // Whether the input matches is known at its first match, where the
        // report asks nothing more of the scan.
        if self.written.only_whether() && !self.not_match && !self.whole_line && !self.stats && self.engine.is_plain() {
            let began = std::time::Instant::now();
            seen.hit = sc.is_match(bytes);
            seen.searching = began.elapsed();
            self.sum_up(ps, path, &seen)?;
            return Ok(seen);
        }
        let index = read.index();
        let began = std::time::Instant::now();
        let mut found = if self.single_match {
            sc.first_found(bytes)
        } else {
            let (found, note) = sc.found_by(bytes, self.engine);
            if let Some(note) = note {
                let from = if path.is_empty() { "text" } else { path };
                ps.verbose(&format!("{from}: {note}"))?;
            }
            found
        };
        seen.searching = began.elapsed();
        // The rungs this input's scan kept, taken as soon as it is done so
        // the next input's scan starts from none.
        let reads_why = self.reads_why();
        let route = if reads_why { trex::explain::route_of(&trex::trace::take_recorded()) } else { String::new() };
        if self.whole_line {
            found = whole_lines(text, &index, found);
        }
        let cap = self.cap();
        // Under -NotMatch the cap is on the lines no match touches, so every
        // match is kept to find them.
        if !self.not_match {
            found.truncate(cap);
        }
        seen.matches = found.len();
        if self.stats {
            seen.matched_lines = index.lines() - untouched_lines(found.iter().map(|(_, m)| m), &index).len();
        }
        self.write_found(ps, read, path, found, &route, &mut seen)?;
        Ok(seen)
    }

    /// Follows every file read, writing each match its growth settles, until
    /// the pipeline is stopped or -MaxCount has written all it will from
    /// each; a file truncated, replaced or removed is said so as a warning
    /// and scanned again from its start.
    fn follow_files(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut followed = std::mem::take(&mut self.followed);
        let Some(sc) = &self.scanning else {
            return Ok(());
        };
        if followed.is_empty() {
            return Ok(());
        }
        let done = |followed: &[crate::follow::FollowedScan]| {
            self.max_count.is_some() && followed.iter().all(|f| f.written >= self.cap())
        };
        if done(&followed) {
            return Ok(());
        }
        let watched: Vec<(std::path::PathBuf, usize)> = followed.iter().map(|f| (f.path.clone(), f.offset())).collect();
        let failed = |e: std::io::Error| PsError::new(ErrorCategory::ReadError, "TrexFollow", format!("cannot follow {e}"));
        let mut follower = trex::follow::Follower::new(&watched).map_err(failed)?;
        if let Some(why) = follower.unnotified() {
            ps.warning(&format!("no change notifications ({why}); looking at the files once a second"))?;
        }
        while !ps.stopping() {
            let Some((i, change)) = follower.poll().map_err(failed)? else {
                continue;
            };
            let f = &mut followed[i];
            let batch = match change {
                trex::follow::Followed::Appended { bytes, .. } => f.push_bytes(&bytes, sc),
                trex::follow::Followed::Truncated => {
                    ps.warning(&format!("{}: truncated; scanning it from its start", f.shown))?;
                    f.restart(sc)
                }
                trex::follow::Followed::Replaced => {
                    ps.warning(&format!("{}: replaced by another file; scanning that from its start", f.shown))?;
                    f.restart(sc)
                }
                trex::follow::Followed::Gone => {
                    ps.warning(&format!("{}: removed; waiting for a file under its name", f.shown))?;
                    f.restart(sc)
                }
            };
            if let Some(batch) = batch {
                self.write_batch(ps, f, batch)?;
            }
            if done(&followed) {
                break;
            }
        }
        Ok(())
    }

    /// Writes a batch of the matches a followed file settled, under
    /// -WholeLine, -SingleMatch and -MaxCount as they read over the whole of
    /// what has been followed: -Json as one object a string, the rest as the
    /// report writes an input's matches.
    fn write_batch(
        &self,
        ps: &Pipeline<'_>,
        followed: &mut crate::follow::FollowedScan,
        batch: crate::follow::Batch,
    ) -> PsResult<()> {
        let Some(sc) = &self.scanning else {
            return Ok(());
        };
        let crate::follow::Batch { read, mut found } = batch;
        let index = read.index();
        if self.whole_line {
            found = whole_lines(&read.text, &index, found);
        }
        // -SingleMatch runs over a set alone, whose every match names its
        // member.
        if self.single_match {
            found.retain(|(member, _)| match member {
                Some(k) => {
                    let first = !followed.members_seen[*k];
                    followed.members_seen[*k] = true;
                    first
                }
                None => true,
            });
        }
        found.truncate(self.cap().saturating_sub(followed.written));
        if found.is_empty() {
            return Ok(());
        }
        followed.written += found.len();
        if matches!(self.written, Written::Json) {
            let kinds = sc.capture_kinds();
            let values = trex::report::ValueView {
                kinds: &kinds,
                style: trex::typed::ValueStyle {
                    spelling: self.value_spelling.unwrap_or(ValueSpelling::Exact).trex(),
                    duration: self.duration_unit.unwrap_or(DurationUnit::Nanoseconds).trex(),
                },
                clock: trex::Clock::current(),
            };
            let bytes = read.text.as_bytes();
            let base = read.base()?;
            for (member, m) in &found {
                let at = self.prefixed.then(|| {
                    let (line, col) = index.line_col(bytes, m.start);
                    (followed.shown.as_str(), line, col)
                });
                let extra = member.map(|k| format!("\"pattern\":\"{}\"", trex::report::json_escape(&sc.name(Some(k)))));
                ps.write(trex::report::match_json(bytes, m, at, base, extra.as_deref(), Some(&values)))?;
            }
            return Ok(());
        }
        let mut seen = Seen::default();
        self.write_found(ps, &read, &followed.shown, found, "", &mut seen)
    }

    /// Writes what the matches `found` of `read`, the part of an input
    /// scanned, come to, as the report asks; `path` is empty for a string.
    fn write_found(
        &self,
        ps: &Pipeline<'_>,
        read: &crate::window::Read,
        path: &str,
        found: Vec<(Option<usize>, trex::Match)>,
        route: &str,
        seen: &mut Seen,
    ) -> PsResult<()> {
        let Some(sc) = &self.scanning else {
            return Ok(());
        };
        let text = read.text.as_str();
        let bytes = text.as_bytes();
        let index = read.index();
        let reads_why = self.reads_why();
        let cap = self.cap();
        if matches!(self.written, Written::Json | Written::Text | Written::Passthru) {
            seen.hit = if self.not_match {
                !untouched_lines(found.iter().map(|(_, m)| m), &index).is_empty()
            } else {
                !found.is_empty()
            };
            return self.cli_report(ps, text, path, &found, &index, route, seen);
        }

        if self.written.summarizes() {
            let line = trex::records::RecordUnit::Line;
            let unit = self.records.as_ref().unwrap_or(&line);
            let records = if !self.not_match && !matches!(self.written, Written::Count) {
                0
            } else if *unit == line && self.blank_string(read, path) {
                usize::from(found.is_empty() == self.not_match).min(cap)
            } else {
                records_touched(unit, bytes, &found, self.not_match, cap)
            };
            seen.hit = if self.not_match { records > 0 } else { !found.is_empty() };
            seen.count = match self.written {
                Written::CountMatches if !self.not_match => found.len(),
                Written::Count | Written::CountMatches => records,
                _ => 0,
            };
            return self.sum_up(ps, path, seen);
        }

        let mut units = Units::of(bytes);
        // The records -Context reads, once for the input rather than once for
        // each match, since every match asks the same list.
        let around = self.around.unit.as_ref().map(|(unit, _, _)| unit.records(bytes));
        let around = around.as_deref();
        if self.not_match {
            let mut lines = if self.blank_string(read, path) {
                if found.is_empty() { vec![0] } else { Vec::new() }
            } else {
                untouched_lines(found.iter().map(|(_, m)| m), &index)
            };
            lines.truncate(cap);
            seen.hit = !lines.is_empty();
            for line in lines {
                if let Written::Format(t) = &self.written {
                    let (s, e) = index.line_span(line);
                    let bare: trex::Match =
                        trex::Span { start: s as u32, end: (s + line_text(text, (s, e)).len()) as u32 }.into();
                    let place = trex::ReportAt {
                        path,
                        line: index.number(line),
                        col: 1,
                        base: read.byte_base,
                        pattern: None,
                        rule: None,
                    };
                    ps.write(t.render_report(&bare, bytes, &place))?;
                    continue;
                }
                let mut m = line_match(text, &mut units, &index, line)?.moved(read.units_ahead()?);
                m.path = path.to_string();
                if self.around.any() {
                    (m.pre_context, m.post_context) =
                        self.context_lines(text, &index, around, line, index.line_span(line));
                }
                if matches!(self.written, Written::Raw) {
                    ps.write(m.text)?;
                } else {
                    ps.write(m)?;
                }
            }
            return Ok(());
        }

        if found.is_empty() {
            return Ok(());
        }
        seen.hit = true;
        let template = match &self.written {
            Written::Format(t) => Some(t),
            _ => None,
        };
        // One explainer for each pattern that matched, built from the lex
        // and the axes that pattern reads.
        let mut explainers: Vec<(Option<usize>, trex::explain::Explainer<'_>)> = Vec::new();
        for (member, m) in &found {
            let why = if reads_why {
                if !explainers.iter().any(|(k, _)| k == member) {
                    explainers.push((*member, trex::explain::Explainer::new(sc.pattern(*member), bytes, sc.shapes())));
                }
                explainers.iter().find(|(k, _)| k == member).map(|(_, x)| x.explain(m, route))
            } else {
                None
            };
            let name = sc.name(*member);
            let (line, col) = index.line_col(bytes, m.start);
            if let Some(t) = template {
                let pattern = if member.is_some() { Some(name.as_str()) } else { None };
                let place = trex::ReportAt { path, line, col, base: read.byte_base, pattern, rule: None };
                let rendered = match &why {
                    Some(why) => t.render_explained(m, bytes, Some(&place), why),
                    None => t.render_report(m, bytes, &place),
                };
                ps.write(rendered)?;
                continue;
            }
            if matches!(self.written, Written::Raw) {
                ps.write(String::from_utf8_lossy(&bytes[m.start..m.end]).into_owned())?;
                continue;
            }
            let mut out = TrexMatch::of(text, &mut units, m, &sc.pattern(*member).capture_kinds(), sc.shapes())?
                .moved(read.units_ahead()?);
            out.path = path.to_string();
            out.line_number = line as i64;
            out.column = col as i64;
            out.pattern = name;
            if self.around.any() {
                (out.pre_context, out.post_context) =
                    self.context_lines(text, &index, around, index.line_of(m.start), (m.start, m.end));
            }
            if self.explain
                && let Some(why) = &why
            {
                out.explanation = explanation_of(why)?;
            }
            ps.write(out)?;
        }
        Ok(())
    }

    /// Whether a walked file is kept by -Texture and -ExcludeTexture.
    fn keeps_texture(&self, path: &std::path::Path) -> bool {
        if self.texture.is_empty() && self.exclude_texture.is_empty() {
            return true;
        }
        let kind = texture_of(path);
        if kind.is_some_and(|k| self.exclude_texture.contains(&k)) {
            return false;
        }
        self.texture.is_empty() || kind.is_some_and(|k| self.texture.contains(&k))
    }
}

impl Cmdlet for SelectTrexMatch {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        self.started = Some(std::time::Instant::now());
        let sc = Scanning::of(ps, &self.pattern, &self.pattern_file, &self.library)?;
        let written = self.written(&sc)?;
        self.window = crate::window::select_of(self.head, self.tail, &self.lines)?;
        if self.index && (self.window.is_some() || self.follow) {
            return Err(arg_err(
                "TrexIndex",
                "-Index summarizes whole files, and -Head, -Tail, -Lines and -Follow read part of each",
            ));
        }
        if self.follow {
            if self.window.is_some_and(|w| !w.runs_to_end()) {
                return Err(arg_err(
                    "TrexFollow",
                    "-Follow scans what a file gains past the part read, and -Head or a -Lines range with a last line ends before the file does; follow a -Tail, a -Lines range with no last line, or the whole file",
                ));
            }
            if self.not_match
                || !self.context.is_empty()
                || self.stats
                || self.explain
                || written.summarizes()
                || matches!(written, Written::Passthru)
            {
                return Err(arg_err(
                    "TrexFollow",
                    "a followed file never ends, so -Follow writes each match as it is found and takes no report written once the input ends or read around a match: -NotMatch, -Context, -Stats, -Explain, -Quiet, -Count, -CountMatches, -FilesWithMatches, -FilesWithoutMatch or -Passthru",
                ));
            }
            if self.dual_grain || self.chunk_size.is_some() || self.backend == Some(Backend::Gpu) {
                return Err(arg_err(
                    "TrexFollow",
                    "a followed file is scanned as it grows by the stream scanner on the CPU, and takes no -Backend Gpu, -DualGrain or -ChunkSize",
                ));
            }
        }
        let defined = if self.unit.is_some() || self.record_start.is_some() || self.record_span.is_some() {
            Some(record_unit(ps, &self.unit, &self.record_start, &self.record_span, &self.library)?)
        } else {
            None
        };
        let around = self.around_asked(defined.as_ref())?;
        if around.any() && !matches!(written, Written::Matches | Written::Text) {
            return Err(arg_err(
                "TrexContext",
                "-Context adds lines to each Trex.Match or to the -Color report, and takes no -Raw, -Format, -Json, -Passthru, -Quiet, -Count, -CountMatches, -FilesWithMatches or -FilesWithoutMatch",
            ));
        }
        if self.explain
            && (self.not_match || !matches!(written, Written::Matches | Written::Format(_) | Written::Json | Written::Text))
        {
            return Err(arg_err(
                "TrexExplain",
                "-Explain says what each match is made of, in a Trex.Match, a -Format line, a -Json object or the -Color report, and takes no -NotMatch, -Raw, -Passthru, -Quiet, -Count, -CountMatches, -FilesWithMatches or -FilesWithoutMatch",
            ));
        }
        if self.not_match && matches!(written, Written::Json | Written::Passthru) {
            return Err(arg_err(
                "TrexReport",
                "-NotMatch writes the lines no match touches, as Trex.Match objects or with -Color as text, and takes no -Json or -Passthru",
            ));
        }
        if !self.color && !self.passthru && (self.color_depth.is_some() || !self.colors.is_empty()) {
            return Err(arg_err(
                "TrexColor",
                "-ColorDepth and -Colors paint what -Color and -Passthru write; give one of them",
            ));
        }
        if !self.json && (self.value_spelling.is_some() || self.duration_unit.is_some()) {
            return Err(arg_err("TrexValues", "-ValueSpelling and -DurationUnit say how -Json writes a value; give -Json"));
        }
        if matches!(written, Written::Text | Written::Passthru) {
            self.painter = Some(self.painter_of()?);
        }
        self.piped = bool::from_ps(&ps.invocation()?.get("ExpectingInput")?)?;
        let context_reads_it = self.context.iter().any(|side| side.trim() == "record");
        if defined.is_some()
            && !context_reads_it
            && self.window.is_none()
            && !(matches!(written, Written::Count) || (self.not_match && written.summarizes()))
        {
            return Err(arg_err(
                "TrexRecordUnit",
                "-Unit, -RecordStart and -RecordSpan say what -Count counts as a record, what -Context record adds and what -Head, -Tail and -Lines count, and under -NotMatch what -CountMatches, -Quiet and the file lists read",
            ));
        }
        self.records = defined;
        self.around = around;
        if self.descending && self.sort.is_none() {
            return Err(arg_err("TrexSort", "-Descending reverses the order -Sort names; give -Sort"));
        }
        if self.single_match && matches!(sc, Scanning::One(_)) {
            return Err(arg_err(
                "TrexSingleMatch",
                "-SingleMatch keeps the first match of each pattern of a set; give several patterns or -PatternFile",
            ));
        }
        let backend = self.backend.unwrap_or(Backend::Auto);
        let mut modes = Vec::new();
        if backend == Backend::Gpu {
            modes.push("-Backend Gpu");
        }
        if self.dual_grain {
            modes.push("-DualGrain");
        }
        if self.chunk_size.is_some() {
            modes.push("-ChunkSize");
        }
        if let [a, b, ..] = modes.as_slice() {
            return Err(arg_err("TrexEngine", format!("{a} and {b} each say how the scan runs; give one")));
        }
        if matches!(sc, Scanning::Set(_)) && !modes.is_empty() {
            return Err(arg_err(
                "TrexEngine",
                "a set's patterns scan over one lex on the CPU engines and take no -Backend Gpu, -DualGrain or -ChunkSize",
            ));
        }
        if self.chunk_size == Some(0) {
            return Err(arg_err("TrexEngine", "-ChunkSize is a count of bytes, 1 or more"));
        }
        self.engine = Engine {
            backend: backend.trex(),
            dual_grain: self.dual_grain,
            chunk_size: self.chunk_size.map(|n| n as usize),
        };
        self.written = written;
        self.scanning = Some(sc);
        // The trace is the process's, so what an earlier call left in it is
        // cleared before this one keeps its own rungs and lexing time.
        if self.stats || self.reads_why() {
            trex::trace::clear();
            self.recording = Some(trex::trace::Recording::start());
        }
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let unit = match &self.records {
            Some(unit) => unit.clone(),
            None => trex::records::RecordUnit::Line,
        };
        if self.path.is_empty() && self.literal_path.is_empty() {
            if self.piped && !self.per_string {
                return self.join(ps);
            }
            let read = crate::window::Read::of_text(&self.input_object, self.window, &unit);
            let seen = self.report(ps, &read, "")?;
            self.totals.add(&seen);
            return Ok(());
        }
        let (given, literal) =
            if self.literal_path.is_empty() { (self.path.clone(), false) } else { (self.literal_path.clone(), true) };
        let opts = trex::files::WalkOptions {
            hidden: self.hidden,
            no_ignore: self.no_ignore,
            globs: self.include.clone(),
            types: self.file_type.clone(),
            types_not: self.exclude_file_type.clone(),
            sort: self.sort.map(|k| trex::files::Sort { key: k.key(), reverse: self.descending }),
        };
        opts.check().map_err(|e| arg_err("TrexWalk", e))?;
        let mut named: Vec<std::path::PathBuf> = Vec::new();
        for g in &given {
            named.extend(ps.resolve_path(g, literal)?.into_iter().map(std::path::PathBuf::from));
        }
        let trees: Vec<std::path::PathBuf> = named.iter().filter(|p| p.is_dir()).cloned().collect();
        // As the trex command decides whether a report names its inputs: a
        // directory, several paths, or paths piped in one at a time do, and a
        // file named alone reports as the command's one input.
        self.prefixed = self.piped || named.len() > 1 || !trees.is_empty();
        // The index of each directory named, read where it can only save
        // work: -NotMatch and -FilesWithoutMatch report what a skipped file
        // would have held, and -Index is building it.
        let reads_index = !self.no_index
            && !self.index
            && !self.not_match
            && !matches!(self.written, Written::FilesWithoutMatch);
        let indexes: Vec<trex::index::Index> =
            if reads_index { trees.iter().filter_map(|t| trex::index::Index::load(t)).collect() } else { Vec::new() };
        let mut building: Vec<trex::index::Index> =
            if self.index { trees.iter().map(|t| trex::index::Index::under(t)).collect() } else { Vec::new() };
        let Some(sc) = &self.scanning else {
            return Ok(());
        };
        let patterns: Vec<&trex::ast::Pattern> = match sc {
            Scanning::One(c) => vec![&c.inner],
            Scanning::Set(set) => set.patterns().iter().collect(),
        };
        for source in files_of(ps, &given, literal, &opts)? {
            if ps.stopping() {
                break;
            }
            let trex::files::Source::File(path) = source else {
                continue;
            };
            // A file named outright is read whatever its texture and whatever
            // an index says, as the trex command reads one; the filters are
            // on what a walk found.
            let walked = !named.contains(&path);
            if walked && !self.keeps_texture(&path) {
                continue;
            }
            if walked && indexes.iter().any(|t| t.refuses_every(&patterns, &path)) {
                continue;
            }
            // A report that writes matches places each on its line, which a
            // tail counts the lines and the code units ahead of it for; a
            // count, a quiet answer and the file lists place nothing.
            let placed = self.follow || !self.written.summarizes();
            let read = match crate::window::read_file(&path, self.window, &unit, self.binary, placed) {
                Ok(Some(read)) => read,
                Ok(None) => continue,
                Err(e) => {
                    ps.write_error(&e)?;
                    continue;
                }
            };
            for index in &mut building {
                if path.starts_with(index.root()) {
                    index.observe(&path, read.text.as_bytes());
                }
            }
            let shown = path.display().to_string();
            if self.follow {
                // The part read is scanned as the head of the stream the file
                // goes on as, so a match reaching past where it ended is found
                // once the rest arrives.
                let mut followed = crate::follow::FollowedScan::new(path.clone(), shown, &read, sc)?;
                if let Some(batch) = followed.push_text(&read.text, sc) {
                    self.write_batch(ps, &mut followed, batch)?;
                }
                self.followed.push(followed);
                continue;
            }
            let mut seen = self.report(ps, &read, &shown)?;
            self.json_objects.append(&mut seen.json);
            self.totals.add(&seen);
        }
        for index in &building {
            let root = index.root().to_path_buf();
            if !ps.should_process(&root.join(trex::index::INDEX_FILE).display().to_string(), "Write the index")? {
                continue;
            }
            index.save(&root).map_err(|e| {
                PsError::new(
                    ErrorCategory::WriteError,
                    "TrexIndexWrite",
                    format!("{}: {e}", root.join(trex::index::INDEX_FILE).display()),
                )
            })?;
            ps.verbose(&format!("{}: indexed {} files; later scans of this tree prune with it", root.display(), index.len()))?;
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        if self.follow {
            return self.follow_files(ps);
        }
        if let Some(joined) = self.joined.take() {
            self.end_joined(ps, joined)?;
        }
        let text = self.path.is_empty() && self.literal_path.is_empty();
        match self.written {
            Written::Quiet => ps.write(self.totals.hit)?,
            Written::Count | Written::CountMatches if text => {
                ps.write(TrexMatchCount { path: String::new(), count: self.totals.count as i64 })?;
            }
            Written::Json if !text && self.prefixed => ps.write(format!("[{}]", self.json_objects.join(",")))?,
            _ => {}
        }
        if self.stats {
            let t = &self.totals;
            let (tokens, lexing) = trex::trace::take_lexing();
            let mut device_bytes = 0u64;
            let mut routes = Vec::new();
            for total in trex::trace::take_totals() {
                if total.ladder != "scan" {
                    continue;
                }
                if total.rung.contains("device") {
                    device_bytes += total.bytes;
                }
                routes.push(TrexScanRoute { rung: total.rung, inputs: total.calls as i64 });
            }
            ps.write(TrexScanStats {
                matches: t.matches as i64,
                matched_lines: t.matched_lines as i64,
                files_with_matches: t.files_with_matches as i64,
                files_searched: t.files_searched as i64,
                bytes_searched: t.bytes_searched as i64,
                searching: span(t.searching),
                elapsed: span(self.started.map_or(std::time::Duration::ZERO, |s| s.elapsed())),
                tokens_lexed: tokens as i64,
                lexing: span(lexing),
                matching: span(t.searching.saturating_sub(lexing)),
                device_bytes: device_bytes as i64,
                routes,
            })?;
        }
        self.recording = None;
        if self.require_match && !self.totals.hit {
            let what = if self.not_match {
                "-RequireMatch: every line of every input holds a match"
            } else {
                "-RequireMatch: no input holds a match"
            };
            ps.write_error(&PsError::new(ErrorCategory::ObjectNotFound, "TrexNoMatch", what))?;
        }
        Ok(())
    }
}

/// Tells whether a trex pattern matches text or the files named.
///
/// With several inputs the answer is whether any of them matched, and with
/// several patterns whether any of them did. -Head, -Tail and -Lines test
/// only the first lines of each input, its last, or a range of them,
/// counted in records of -Unit where it names one; a match counts only where
/// it lies wholly inside. -First and -Last are -Head and -Tail.
///
/// # Examples
/// Test-TrexMatch '\E' 'ping bob@x.com'
/// 'a', 'bob@x.com' | Test-TrexMatch '\E'
/// Test-TrexMatch -PatternFile ./secrets.trex -Path ./src
/// Test-TrexMatch '"ERROR"' -Path ./app.log -Tail 100
#[cmdlet(verb = "Test", noun = "TrexMatch", alias = "Test-TxMatch", default_parameter_set = "Text", output = ["System.Boolean"])]
#[derive(Default)]
pub struct TestTrexMatch {
    /// The patterns: trex source text or Trex.Pattern objects.
    #[param(position = 0)]
    pub pattern: Vec<PsObject>,
    /// A pattern file whose members are tested as one set. Text to test
    /// beside it is piped in or named with -InputObject, since a first
    /// argument by position is read as -Pattern.
    #[param]
    pub pattern_file: Option<String>,
    /// The text to test; each string piped in is tested on its own.
    #[param(mandatory, position = 1, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files to test; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// The atoms a source-text pattern is compiled against, in place of the
    /// session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    /// Tests the first this many lines of each input, or records of -Unit,
    /// reading a file no further.
    #[param(alias = ["First"])]
    pub head: Option<u32>,
    /// Tests the last this many lines of each input, or records of -Unit,
    /// reading a file backward from its end.
    #[param(alias = ["Last"])]
    pub tail: Option<u32>,
    /// Tests a range of lines, or records of -Unit, counted from one:
    /// `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`.
    #[param]
    pub lines: Option<PsObject>,
    /// What -Head, -Tail and -Lines count: a line when absent, or a unit
    /// Find-TrexRecord reads, such as `paragraph` or `block`.
    #[param]
    pub unit: Option<String>,
    scanning: Option<Scanning>,
    matched_any: bool,
    part: crate::window::Part,
}

impl Cmdlet for TestTrexMatch {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        self.scanning = Some(Scanning::of(ps, &self.pattern, &self.pattern_file, &self.library)?);
        self.part = crate::window::Part::of(self.head, self.tail, &self.lines, &self.unit)?;
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(sc) = &self.scanning else {
            return Ok(());
        };
        if self.path.is_empty() {
            let read = self.part.of_text(&self.input_object);
            self.matched_any |= sc.is_match(read.text.as_bytes());
            return Ok(());
        }
        let opts = trex::files::WalkOptions::default();
        for source in files_of(ps, &self.path, false, &opts)? {
            let trex::files::Source::File(path) = source else {
                continue;
            };
            match self.part.read(&path, false, false) {
                Ok(Some(read)) => self.matched_any |= sc.is_match(read.text.as_bytes()),
                Ok(None) => {}
                Err(e) => ps.write_error(&e)?,
            }
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        ps.write(self.matched_any)
    }
}
