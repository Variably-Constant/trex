//! Rules: named patterns a pattern file declares with a message, a severity
//! and a fix, scanned as findings through trex's own rule scan, the one the
//! trex command's `scan --rules` runs.
//!
//! Rules are atoms like any other: Register-TrexAtom and Import-TrexAtom
//! declare them for the session, a Trex.Library holds its own, and
//! -RuleFile imports pattern files for one call, a file the session already
//! imported read in place of that import.

use std::path::Path;

use pwrs::prelude::*;

use crate::atoms::{TrexLibrary, atoms_with_files, like};
use crate::axes::text_of;
use crate::common::{OffsetUnit, arg_err, units_of};
use crate::matching::{TrexMatchCount, files_of, read_whole};
use crate::transform::{Pending, review, write_reviewed};
use crate::window::{Part, Read};

/// How serious a rule's finding is.
#[psenum(name = "Trex.Severity")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Severity {
    /// Worth a look; what a rule is where it names no severity.
    #[default]
    Warning,
    /// A fault; a scan with one fails in the trex command.
    Error,
    /// For information.
    Note,
}

impl Severity {
    fn of(s: trex::Severity) -> Self {
        match s {
            trex::Severity::Error => Severity::Error,
            trex::Severity::Warning => Severity::Warning,
            trex::Severity::Note => Severity::Note,
        }
    }

    fn trex(self) -> trex::Severity {
        match self {
            Severity::Error => trex::Severity::Error,
            Severity::Warning => trex::Severity::Warning,
            Severity::Note => trex::Severity::Note,
        }
    }
}

/// One rule as its pattern file declares it.
#[psclass(name = "Trex.Rule", show = "{Name}")]
#[derive(Clone, Default)]
pub struct TrexRule {
    /// The rule's name, the id a finding is reported under.
    pub name: String,
    /// How serious a finding is.
    pub severity: Severity,
    /// What a finding says: a report template rendered at each match.
    pub message: String,
    /// What the rule matches, as written.
    pub pattern: String,
    /// The template a fix renders in the match's place, empty where the rule
    /// has none.
    pub fix: String,
    /// The globs the inputs the rule reads are kept or dropped by; every
    /// input where there are none.
    pub files: Vec<String>,
    /// The rule's `meta.KEY = VALUE` lines, as a hashtable.
    pub meta: PsObject,
    /// The values `meta.tags` lists.
    pub tags: Vec<String>,
    /// The pattern file the rule is in, empty for one declared here.
    pub file: String,
    /// The line the rule opens on, counting from 1.
    pub line: i64,
}

impl TrexRule {
    fn of(rule: &trex::Rule) -> PsResult<Self> {
        let mut meta = std::collections::HashMap::new();
        for (k, v) in &rule.meta {
            meta.insert(k.clone(), v.clone());
        }
        Ok(TrexRule {
            name: rule.name.clone(),
            severity: Severity::of(rule.severity),
            message: rule.message.clone(),
            pattern: rule.source.clone(),
            fix: match &rule.fix {
                Some(f) => f.clone(),
                None => String::new(),
            },
            files: rule.files.clone(),
            meta: meta.into_ps()?,
            tags: rule.tags(),
            file: match &rule.file {
                Some(p) => p.display().to_string(),
                None => String::new(),
            },
            line: rule.line as i64,
        })
    }
}

/// A span's position: its first line and column and the line and column
/// just past its end, each from 1.
#[psclass(name = "Trex.Region", show = "{Line}:{Column}")]
#[derive(Clone, Default)]
pub struct TrexRegion {
    /// The line it starts on.
    pub line: i64,
    /// The character it starts at within its line.
    pub column: i64,
    /// The line just past its end is on.
    pub end_line: i64,
    /// The character just past its end.
    pub end_column: i64,
}

impl TrexRegion {
    fn of(p: trex::rule_scan::Place) -> Self {
        TrexRegion { line: p.line as i64, column: p.col as i64, end_line: p.end_line as i64, end_column: p.end_col as i64 }
    }

    fn place(&self) -> trex::rule_scan::Place {
        trex::rule_scan::Place {
            line: self.line as usize,
            col: self.column as usize,
            end_line: self.end_line as usize,
            end_col: self.end_column as usize,
        }
    }
}

/// One finding of a rule.
#[psclass(name = "Trex.Finding")]
#[derive(Clone, Default)]
pub struct TrexFinding {
    /// The rule's name.
    pub rule: String,
    /// How serious it is.
    pub severity: Severity,
    /// What it says, rendered from the match.
    pub message: String,
    /// The file it is in, empty for a string.
    pub path: String,
    /// Where it starts and ends, by line and column.
    pub region: TrexRegion,
    /// Where it starts in the input, in UTF-16 code units.
    pub start: i64,
    /// How many UTF-16 code units it spans.
    pub length: i64,
    /// Its text: the match, or the record for a rule that fires on records.
    pub text: String,
    /// What a fix puts in the match's place; `$null` where the rule has no
    /// fix.
    pub fix: PsObject,
    /// The span a fix replaces, the match, a Trex.Region; `$null` where the
    /// rule has no fix.
    pub fix_region: PsObject,
    /// Each register the rule's pattern bound, as a hashtable of its name to
    /// its text.
    pub captures: PsObject,
    /// The rule itself, a Trex.Rule.
    pub definition: PsObject,
}

/// The atoms in force with the rule files named imported for this call: a
/// file the session or the library already imported is read in place of
/// that import rather than declared a second time.
fn rule_set(ps: &Pipeline<'_>, library: &Option<PsProxy<TrexLibrary>>, rule_files: &[String]) -> PsResult<trex::ShapeSet> {
    let mut resolved = Vec::new();
    for given in rule_files {
        resolved.extend(ps.resolve_path(given, false)?);
    }
    let files = trex::rule_scan::rule_files(&resolved).map_err(|e| arg_err("TrexRuleFile", e))?;
    atoms_with_files(ps, library, &files)
}

/// Lists the rules declared for the session, in a library, or in the rule
/// files named, in declaration order.
///
/// # Examples
/// Get-TrexRule
/// Get-TrexRule -RuleFile ./rules | Where-Object Severity -eq Error
#[cmdlet(verb = "Get", noun = "TrexRule", alias = "Get-TxRule", output = ["Trex.Rule"])]
#[derive(Default)]
pub struct GetTrexRule {
    /// Only rules whose name matches; wildcards apply.
    #[param(position = 0)]
    pub name: Option<String>,
    /// Pattern files, or directories of `.trex` files, imported for this call
    /// over the atoms in force; a file already imported is read in place of
    /// that import.
    #[param]
    pub rule_file: Vec<String>,
    /// Lists a library's rules in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
}

impl Cmdlet for GetTrexRule {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let shapes = rule_set(ps, &self.library, &self.rule_file)?;
        for rule in shapes.rules() {
            if self.name.as_deref().is_none_or(|pattern| like(&rule.name, pattern)) {
                ps.write(TrexRule::of(rule)?)?;
            }
        }
        Ok(())
    }
}

/// What Invoke-TrexRule writes.
enum Output {
    /// Each finding as a Trex.Finding.
    Findings,
    /// One JSON array of every finding, after the last input, as the trex
    /// command's `scan --rules --json` writes it.
    Json,
    /// Each finding as one JSON object, written as it is made, for a
    /// followed file, whose last input never comes.
    JsonLines,
    /// One SARIF document for every finding, after the last input.
    Sarif,
    /// Each finding as a GitHub workflow annotation.
    Github,
    /// A report template rendered at each finding.
    Format(trex::Template),
    /// How many findings each input holds.
    Count,
    /// The path of each file holding a finding.
    FilesWith,
    /// The path of each file holding none.
    FilesWithout,
}

/// What Invoke-TrexRule keeps across its inputs: the JSON objects and the
/// inputs of the SARIF document written after the last, and the counts.
#[derive(Default)]
struct Kept {
    json_objects: Vec<String>,
    sarif_found: Vec<trex::rule_scan::Found>,
    found_total: usize,
    text_found: usize,
}

/// The findings of one input as objects, `read` the part of it read, each
/// placed where the input places it.
fn finding_objects(
    scan: &trex::rule_scan::RuleScan,
    path: &str,
    read: &Read,
    found: &[trex::rule_scan::Finding],
) -> PsResult<Vec<TrexFinding>> {
    let bytes = read.text.as_bytes();
    let index = read.index();
    let ahead = read.units_ahead()?;
    let mut units = units_of(bytes);
    let mut out = Vec::with_capacity(found.len());
    for f in found {
        let rule = &scan.rules()[f.rule];
        let start = units.at(f.start);
        let end = units.at(f.end);
        let mut captures = std::collections::HashMap::new();
        for (name, span) in f.m.names().iter().zip(f.m.captures()) {
            captures.insert(name.clone(), text_of(bytes, (span.start(), span.end())));
        }
        let (fix, fix_region) = match &f.fix {
            Some(fix) => (
                fix.clone().into_ps()?,
                TrexRegion::of(trex::rule_scan::Place::of(&index, bytes, f.m.start, f.m.end)).into_ps()?,
            ),
            None => (PsObject::default(), PsObject::default()),
        };
        out.push(TrexFinding {
            rule: rule.name.clone(),
            severity: Severity::of(rule.severity),
            message: f.message.clone(),
            path: path.to_string(),
            region: TrexRegion::of(trex::rule_scan::Place::of(&index, bytes, f.start, f.end)),
            start: (ahead + start) as i64,
            length: (end - start) as i64,
            text: text_of(bytes, (f.start, f.end)),
            fix,
            fix_region,
            captures: captures.into_ps()?,
            definition: TrexRule::of(rule)?.into_ps()?,
        });
    }
    Ok(out)
}

/// Writes the findings `found` of `read`, the part of the input named
/// `name` read, as `output` asks, or keeps them in `kept` for what is
/// written after the last input.
fn write_found(
    ps: &Pipeline<'_>,
    scan: &trex::rule_scan::RuleScan,
    output: &Output,
    kept: &mut Kept,
    name: Option<&str>,
    read: &Read,
    found: Vec<trex::rule_scan::Finding>,
) -> PsResult<()> {
    let bytes = read.text.as_bytes();
    kept.found_total += found.len();
    match output {
        Output::Findings => ps.write(finding_objects(scan, name.unwrap_or(""), read, &found)?)?,
        Output::Count => match name {
            Some(n) => ps.write(TrexMatchCount { path: n.to_string(), count: found.len() as i64 })?,
            None => kept.text_found += found.len(),
        },
        Output::FilesWith => {
            if let Some(n) = name
                && !found.is_empty()
            {
                ps.write(n.to_string())?;
            }
        }
        Output::FilesWithout => {
            if let Some(n) = name
                && found.is_empty()
            {
                ps.write(n.to_string())?;
            }
        }
        Output::Json => {
            let index = read.index();
            for f in &found {
                kept.json_objects.push(scan.finding_json(name, bytes, &index, f));
            }
        }
        Output::JsonLines => {
            let index = read.index();
            for f in &found {
                ps.write(scan.finding_json(name, bytes, &index, f))?;
            }
        }
        Output::Github => {
            let index = read.index();
            for f in &found {
                ps.write(scan.github_line(name, bytes, &index, f))?;
            }
        }
        Output::Format(t) => {
            let index = read.index();
            let mut units = units_of(bytes);
            for f in &found {
                let rule = &scan.rules()[f.rule];
                let at = trex::rule_scan::Place::of(&index, bytes, f.start, f.end);
                let offsets = if t.reads_offsets() {
                    read.unit_base.map(|ahead| (ahead + units.at(f.m.start), ahead + units.at(f.m.end)))
                } else {
                    None
                };
                let place = trex::ReportAt {
                    path: name.unwrap_or(""),
                    line: at.line,
                    col: at.col,
                    offsets,
                    pattern: Some(&rule.name),
                    rule: Some(trex::ReportRule {
                        name: &rule.name,
                        severity: rule.severity.name(),
                        message: &f.message,
                        fix: f.fix.as_deref().unwrap_or(""),
                    }),
                };
                ps.write(t.render_report(&f.m, bytes, &place))?;
            }
        }
        Output::Sarif => {
            let index = read.index();
            kept.sarif_found.push(trex::rule_scan::Found {
                name: name.map(str::to_string),
                input: bytes.to_vec(),
                byte_base: index.base(),
                line_base: index.lines_before(),
                findings: found,
            });
        }
    }
    Ok(())
}

/// One file -Follow scans for findings as it grows: the part read and every
/// byte the file gains after it, as one stream of the rules that fire on
/// matches.
struct FollowedRules<'a> {
    path: std::path::PathBuf,
    shown: String,
    /// Where following the file picks up: its length when the part was read.
    offset: usize,
    decoder: trex::encoding::Incremental,
    /// The opening bytes of a UTF-8 character a piece ended inside, held
    /// until the rest arrives.
    carry: Vec<u8>,
    stream: trex::rule_scan::RuleStream<'a>,
    /// The UTF-16 code units of the file's text up to what has been pushed.
    units_through: usize,
    /// How many findings have been written, for -MaxCount.
    written: usize,
}

/// A batch of findings a followed file settled, beside the part of the file
/// it was found in, placed as the file places it.
type Batches = Vec<(Read, Vec<trex::rule_scan::Finding>)>;

impl<'a> FollowedRules<'a> {
    /// `found`, what the stream made, as batches beside the parts of the
    /// file each was found in.
    fn placed(&self, found: Vec<trex::rule_scan::Found>) -> PsResult<Batches> {
        let mut out = Vec::with_capacity(found.len());
        for one in found {
            let ahead = self.units_through - trex::encoding::utf16_units(&one.input);
            let base = one.byte_base.ok_or_else(crate::window::unplaced)?;
            let input_len = base + one.input.len();
            let text = String::from_utf8(one.input)
                .map_err(|e| PsError::new(ErrorCategory::InvalidData, "TrexFollow", format!("{}: {e}", self.shown)))?;
            let read = Read {
                text,
                byte_base: Some(base),
                line_base: one.line_base,
                unit_base: Some(ahead),
                input_len,
                encoding: trex::encoding::Encoding::Utf8,
            };
            out.push((read, one.findings));
        }
        Ok(out)
    }

    /// Scan `text`, the file's next text: the findings it settles.
    fn push_text(&mut self, text: &str) -> PsResult<Batches> {
        self.units_through += trex::encoding::utf16_units(text.as_bytes());
        let found = self.stream.push(text.as_bytes());
        self.placed(found)
    }

    /// Scan `bytes`, what the file gained, decoded as its opening declared;
    /// a character cut between two pieces is held until the rest arrives.
    fn push_bytes(&mut self, bytes: &[u8]) -> PsResult<Batches> {
        let mut text = std::mem::take(&mut self.carry);
        text.extend_from_slice(&self.decoder.decode(bytes));
        let whole = crate::follow::complete_prefix(&text);
        self.carry = text.split_off(whole);
        let text = String::from_utf8(text)
            .map_err(|e| PsError::new(ErrorCategory::InvalidData, "TrexFollow", format!("{}: {e}", self.shown)))?;
        self.push_text(&text)
    }

    /// The file was truncated, replaced or removed: the findings its stream
    /// held until then, and the stream begun again over `scan` at the start
    /// of the file now under its name.
    fn restart(&mut self, scan: &'a trex::rule_scan::RuleScan) -> PsResult<Batches> {
        let fresh = scan.stream(Some(self.shown.as_str()), 0, Some(0), Some((OffsetUnit::Utf16, 0)));
        let ended = std::mem::replace(&mut self.stream, fresh).finish();
        let batches = self.placed(ended)?;
        self.units_through = 0;
        self.carry.clear();
        self.decoder = trex::encoding::Incremental::from_start();
        Ok(batches)
    }
}

/// Scans the rules declared in pattern files over text, files or
/// directories, and writes each finding as a Trex.Finding.
///
/// A rule fires on each match of its pattern, or on each record holding it
/// where the rule names a record or a pattern the record must not hold; its
/// message is rendered from the match, and its fix, where it has one,
/// renders what replaces the match. -Fix writes the fixes into the files,
/// asking first under -Confirm and saying what it would do under -WhatIf,
/// and with -Interactive puts each fix to the person first, as
/// Edit-TrexText -Interactive puts a change; -Diff writes the unified diff
/// the fixes would make instead.
///
/// -Json writes every finding as the trex command's `scan --rules --json`
/// writes them, one array after the last input; -Sarif one SARIF 2.1.0
/// document for every finding, -GitHub one GitHub workflow annotation each,
/// and -Format a report template rendered at each, which reads `${rule}`,
/// `${severity}`, `${message}` and `${fix}` beside the registers. -Count
/// writes how many findings each file holds, as the trex
/// command's `--count` does, -FilesWithMatches and -FilesWithoutMatch the
/// path of each file holding a finding or none, and -RequireMatch an error
/// when no input held a finding.
///
/// -Head, -Tail and -Lines scan only the first lines of each input, its
/// last, or a range of them, counted in records of -Unit where it names one:
/// a finding counts only if it is wholly inside, and reports the
/// input's own line and offset; -Fix and -Diff change only that part. -First
/// and -Last are -Head and -Tail. -Follow scans what each file gains as it
/// grows, after its tail, its open range or the whole of it, writing each
/// finding once nothing that arrives later can change it, -Json as one
/// object a string, until the pipeline is stopped or -MaxCount has written
/// all it will from each file.
///
/// # Examples
/// Invoke-TrexRule -Path ./src -RuleFile ./rules
/// Invoke-TrexRule -Path ./src -RuleFile ./rules -Sarif | Set-Content ./trex.sarif
/// Invoke-TrexRule -Path ./src -RuleFile ./rules -Json | Set-Content ./findings.json
/// Invoke-TrexRule -Path ./src -RuleFile ./rules -Fix -WhatIf
/// Invoke-TrexRule -Path ./src -RuleFile ./rules -Fix -Interactive
/// Invoke-TrexRule -Path ./src | Group-Object Rule | Sort-Object Count -Descending
/// Invoke-TrexRule -Path ./app.log -RuleFile ./rules -Tail 100 -Follow
#[cmdlet(verb = "Invoke", noun = "TrexRule", alias = "Invoke-TxRule", supports_should_process, confirm_impact = "Medium", default_parameter_set = "Text", output = ["Trex.Finding", "System.String", "Trex.MatchCount"])]
#[derive(Default)]
pub struct InvokeTrexRule {
    /// The text to scan; each string piped in is scanned on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to scan; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to scan, read as written, as Get-ChildItem
    /// pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// Pattern files, or directories of `.trex` files, imported for this call
    /// over the atoms in force; a file already imported is read in place of
    /// that import.
    #[param]
    pub rule_file: Vec<String>,
    /// The atoms, rules among them, to read in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    /// Writes at most this many findings of each input.
    #[param]
    pub max_count: Option<u32>,
    /// Writes the fixes into the files.
    #[param(set = ["Path", "LiteralPath"])]
    pub fix: bool,
    /// Writes the unified diff the fixes would make, and nothing to the
    /// files.
    #[param(set = ["Path", "LiteralPath"])]
    pub diff: bool,
    /// Lines of context around each change in a diff; 3 when absent.
    #[param(set = ["Path", "LiteralPath"])]
    pub context: Option<u32>,
    /// With -Fix, puts each fix to the person before it is written, as
    /// Edit-TrexText -Interactive puts a change.
    #[param(set = ["Path", "LiteralPath"])]
    pub interactive: bool,
    /// Lists the position of each fix a template answer skipped, beside the
    /// count -Interactive always reports.
    #[param(set = ["Path", "LiteralPath"])]
    pub show_skipped: bool,
    /// Writes every finding as the trex command's `scan --rules --json`
    /// writes them: one array after the last input, each object naming the
    /// rule, its severity and message, the finding's position, its text,
    /// registers and fix, the rule's metadata, and the path of a file.
    #[param]
    pub json: bool,
    /// Writes every finding as one SARIF 2.1.0 document.
    #[param]
    pub sarif: bool,
    /// Writes each finding as a GitHub workflow annotation.
    #[param]
    pub git_hub: bool,
    /// A report template to write at each finding in place of the finding.
    #[param]
    pub format: Option<String>,
    /// Writes how many findings each file holds, as a Trex.MatchCount for
    /// every file read, none included; over text, one count of every string,
    /// after the last.
    #[param]
    pub count: bool,
    /// Writes the path of each file holding a finding.
    #[param(set = ["Path", "LiteralPath"])]
    pub files_with_matches: bool,
    /// Writes the path of each file holding no finding.
    #[param(set = ["Path", "LiteralPath"])]
    pub files_without_match: bool,
    /// Writes an error after the last input when no input held a finding,
    /// for a script that stops on one.
    #[param]
    pub require_match: bool,
    /// Reads hidden files and directories a walk would skip.
    #[param(set = ["Path", "LiteralPath"])]
    pub hidden: bool,
    /// Reads files an ignore rule excludes.
    #[param(set = ["Path", "LiteralPath"])]
    pub no_ignore: bool,
    /// Reads files that hold a NUL byte, which a walk treats as binary.
    #[param(set = ["Path", "LiteralPath"])]
    pub binary: bool,
    /// Keeps a walked file only when a glob matches it, or drops it for a
    /// glob that starts with `!`.
    #[param(set = ["Path", "LiteralPath"])]
    pub include: Vec<String>,
    /// Scans the first this many lines of each input, or records of -Unit,
    /// reading a file no further.
    #[param(alias = ["First"])]
    pub head: Option<u32>,
    /// Scans the last this many lines of each input, or records of -Unit,
    /// reading a file backward from its end.
    #[param(alias = ["Last"])]
    pub tail: Option<u32>,
    /// Scans a range of lines, or records of -Unit, counted from one:
    /// `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`.
    #[param]
    pub lines: Option<PsObject>,
    /// What -Head, -Tail and -Lines count: a line when absent, or a unit
    /// Find-TrexRecord reads, such as `paragraph` or `block`.
    #[param]
    pub unit: Option<String>,
    /// After each file's -Tail, its open -Lines range or the whole of it,
    /// scans what it gains as it grows, writing each finding once nothing
    /// that arrives later can change it, until the pipeline is stopped. A
    /// file truncated, replaced or removed is a new input, its -MaxCount
    /// count started again.
    #[param(alias = ["Wait"], set = ["Path", "LiteralPath"])]
    pub follow: bool,
    /// With -Follow and -MaxCount, keeps a file's count when it is
    /// truncated, replaced or removed, in place of starting it again.
    #[param(set = ["Path", "LiteralPath"])]
    pub keep_count: bool,
    scan: Option<trex::rule_scan::RuleScan>,
    output: Option<Output>,
    kept: Kept,
    pending: Vec<Pending>,
    part: Part,
    /// Each file -Follow scans, with the part of it read, followed once the
    /// pipeline's input ends.
    to_follow: Vec<(std::path::PathBuf, String, Read)>,
}

impl InvokeTrexRule {
    /// Scans `read`, the part of the input named `name` read, and writes
    /// what it comes to, or keeps its findings for what is written after the
    /// last input.
    fn report(&mut self, ps: &Pipeline<'_>, name: Option<&str>, read: &Read) -> PsResult<()> {
        let (Some(scan), Some(output)) = (&self.scan, &self.output) else {
            return Err(arg_err("TrexNoRules", "the rules were not read"));
        };
        let found = scan.findings(name, read.text.as_bytes(), &read.placing_index(), self.max_count.map(|n| n as usize));
        write_found(ps, scan, output, &mut self.kept, name, read, found)
    }

    /// Whether the output gives each finding's position, so a tail must count
    /// the lines and the UTF-16 code units before it; a count and the file
    /// lists give none, unless a rule's message writes its match's position.
    fn places(&self) -> bool {
        !matches!(self.output, Some(Output::Count | Output::FilesWith | Output::FilesWithout))
            || self.scan.as_ref().is_some_and(|s| s.messages_read_place() || s.messages_read_offsets())
    }

    /// Lines of context around each change in a diff.
    fn context_lines(&self) -> usize {
        match self.context {
            Some(n) => n as usize,
            None => 3,
        }
    }

    /// Writes the fixes into one file, or the diff they would make, or
    /// under -Interactive keeps them for the review that follows the last
    /// file: the fixes of the part -Head, -Tail or -Lines names, where one
    /// does, and of the whole file where none does. `raw` is the file's
    /// bytes, which the fixes of `text`, its text, are written into.
    fn fix_file(&mut self, ps: &Pipeline<'_>, path: &Path, raw: Vec<u8>, text: &str) -> PsResult<()> {
        let Some(scan) = &self.scan else {
            return Err(arg_err("TrexNoRules", "the rules were not read"));
        };
        let shown = path.display().to_string();
        let bytes = text.as_bytes();
        let range = match self.part.select {
            Some(select) => trex::window::range_of(bytes, select, &self.part.unit),
            None => 0..bytes.len(),
        };
        let lines_before = trex::byte_simd::count_byte(&bytes[..range.start], b'\n');
        let piece = &bytes[range.clone()];
        let index = trex::files::LineIndex::new(piece).within(Some(range.start), Some(lines_before));
        let (mut edits, skipped) = scan.fix_edits(&shown, piece, &index);
        for e in &mut edits {
            e.start += range.start;
            e.end += range.start;
        }
        for s in skipped {
            ps.warning(&format!(
                "{shown}:{}:{}: the fix of {} overlaps an earlier fix; skipped",
                s.line,
                s.col,
                scan.rules()[s.rule].name
            ))?;
        }
        if edits.is_empty() {
            return Ok(());
        }
        if self.diff {
            return ps.write(trex::files::unified_diff(&shown, bytes, &edits, self.context_lines()));
        }
        if self.interactive {
            self.pending.push(Pending { path: path.to_path_buf(), raw, text: text.to_string(), edits });
            return Ok(());
        }
        if ps.should_process(&shown, &format!("Apply {} fix(es)", edits.len()))? {
            std::fs::write(path, trex::files::written(&raw, &edits, trex::files::ReadAs::Lossy))
                .map_err(|e| PsError::new(ErrorCategory::WriteError, "TrexWrite", format!("{shown}: {e}")))?;
        }
        Ok(())
    }

    /// Scans every file -Follow read as it grows, from the part of it read
    /// on, writing each finding once nothing that arrives later can change
    /// it, until the pipeline is stopped or -MaxCount has written all it
    /// will from every file; a file truncated, replaced or removed is said
    /// so as a warning and scanned again from its start.
    fn follow_files(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let to_follow = std::mem::take(&mut self.to_follow);
        let (Some(scan), Some(output)) = (&self.scan, &self.output) else {
            return Err(arg_err("TrexNoRules", "the rules were not read"));
        };
        let cap = self.max_count.map(|n| n as usize);
        let keep_count = self.keep_count;
        let kept = &mut self.kept;
        let mut write = |f: &mut FollowedRules<'_>, batches: Batches| -> PsResult<()> {
            for (read, mut findings) in batches {
                if let Some(n) = cap {
                    findings.truncate(n.saturating_sub(f.written));
                }
                f.written += findings.len();
                write_found(ps, scan, output, kept, Some(f.shown.as_str()), &read, findings)?;
            }
            Ok(())
        };
        let mut followed: Vec<FollowedRules<'_>> = Vec::with_capacity(to_follow.len());
        for (path, shown, read) in to_follow {
            let stream =
                scan.stream(Some(shown.as_str()), read.base()?, read.line_base, Some((OffsetUnit::Utf16, read.units_ahead()?)));
            let mut f = FollowedRules {
                stream,
                path,
                shown,
                offset: read.input_len,
                decoder: trex::encoding::Incremental::after(read.encoding),
                carry: Vec::new(),
                units_through: read.units_ahead()?,
                written: 0,
            };
            let batches = f.push_text(&read.text)?;
            write(&mut f, batches)?;
            followed.push(f);
        }
        let done = |followed: &[FollowedRules<'_>]| cap.is_some_and(|n| followed.iter().all(|f| f.written >= n));
        if followed.is_empty() || done(&followed) {
            return Ok(());
        }
        let watched: Vec<(std::path::PathBuf, usize)> = followed.iter().map(|f| (f.path.clone(), f.offset)).collect();
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
            let restarted = !matches!(&change, trex::follow::Followed::Appended { .. });
            let batches = match change {
                trex::follow::Followed::Appended { bytes, .. } => f.push_bytes(&bytes)?,
                trex::follow::Followed::Truncated => {
                    ps.warning(&format!("{}: truncated; scanning it from its start", f.shown))?;
                    f.restart(scan)?
                }
                trex::follow::Followed::Replaced => {
                    ps.warning(&format!("{}: replaced by another file; scanning that from its start", f.shown))?;
                    f.restart(scan)?
                }
                trex::follow::Followed::Gone => {
                    ps.warning(&format!("{}: removed; waiting for a file under its name", f.shown))?;
                    f.restart(scan)?
                }
            };
            write(f, batches)?;
            // The file now under the name is a new input, whose -MaxCount
            // count starts again unless -KeepCount carries it on.
            if restarted && !keep_count {
                f.written = 0;
            }
            if done(&followed) {
                break;
            }
        }
        Ok(())
    }
}

impl Cmdlet for InvokeTrexRule {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let shapes = rule_set(ps, &self.library, &self.rule_file)?;
        if shapes.rules().is_empty() {
            return Err(arg_err(
                "TrexNoRules",
                "no rule is declared: import a pattern file with rule lines, or name one with -RuleFile",
            ));
        }
        let scan = trex::rule_scan::RuleScan::new(shapes).map_err(|e| arg_err("TrexRule", e))?;
        let chosen = [
            self.json,
            self.sarif,
            self.git_hub,
            self.format.is_some(),
            self.count,
            self.files_with_matches,
            self.files_without_match,
        ]
        .iter()
        .filter(|&&b| b)
        .count();
        if chosen > 1 {
            return Err(arg_err(
                "TrexRuleOutput",
                "-Json, -Sarif, -GitHub, -Format, -Count, -FilesWithMatches and -FilesWithoutMatch each say how findings are written; give one",
            ));
        }
        if (self.fix || self.diff) && chosen > 0 {
            return Err(arg_err(
                "TrexRuleOutput",
                "-Fix and -Diff write the fixes, and take no -Json, -Sarif, -GitHub, -Format, -Count, -FilesWithMatches or -FilesWithoutMatch",
            ));
        }
        if (self.fix || self.diff) && self.require_match {
            return Err(arg_err("TrexRuleOutput", "-RequireMatch fails a scan that found nothing, and takes no -Fix or -Diff"));
        }
        if self.interactive && (!self.fix || self.diff) {
            return Err(arg_err(
                "TrexRuleOutput",
                "-Interactive puts each fix to the person before -Fix writes it, and takes -Fix and no -Diff",
            ));
        }
        if self.show_skipped && !self.interactive {
            return Err(arg_err(
                "TrexRuleOutput",
                "-ShowSkipped lists the fixes a template answer skipped, which only -Interactive gives",
            ));
        }
        self.part = Part::of(self.head, self.tail, &self.lines, &self.unit)?;
        if self.follow {
            if self.part.select.is_some_and(|w| !w.runs_to_end()) {
                return Err(arg_err(
                    "TrexFollow",
                    "-Follow scans what a file gains past the part read, and -Head or a -Lines range with a last line ends before the file does; follow a -Tail, a -Lines range with no last line, or the whole file",
                ));
            }
            if self.fix
                || self.diff
                || self.sarif
                || self.count
                || self.files_with_matches
                || self.files_without_match
                || self.require_match
            {
                return Err(arg_err(
                    "TrexFollow",
                    "a followed file never ends, so -Follow writes each finding as it is made and takes no report written once the input ends: -Sarif, -Count, -FilesWithMatches, -FilesWithoutMatch, -RequireMatch, -Fix or -Diff",
                ));
            }
            let on_records = scan.record_rules();
            if !on_records.is_empty() {
                return Err(arg_err(
                    "TrexFollow",
                    format!(
                        "{} fire on records, and a record is whole only once the input holding it ends, which a followed file does not; follow the rules that fire on matches",
                        on_records.join(", ")
                    ),
                ));
            }
            if !scan.stream(None, 0, None, None).commits_early() {
                return Err(arg_err(
                    "TrexFollow",
                    "no match of these rules is final before its input ends, since one reads a content guard, a field anchor or a whole-input axis, and a followed file does not end; run it without -Follow",
                ));
            }
        }
        if self.keep_count && !self.follow {
            return Err(arg_err(
                "TrexKeepCount",
                "-KeepCount keeps a followed file's -MaxCount count when it is truncated, replaced or removed, and takes -Follow",
            ));
        }
        if self.keep_count && self.max_count.is_none() {
            return Err(arg_err("TrexKeepCount", "-KeepCount keeps the count -MaxCount caps, and no -MaxCount was given"));
        }
        self.output = Some(match &self.format {
            Some(src) => Output::Format(
                trex::Template::parse_report(src, &scan.capture_names())
                    .map_err(|e| arg_err("TrexTemplateError", format!("-Format error at byte {}: {}", e.pos, e.msg)))?,
            ),
            None if self.json && self.follow => Output::JsonLines,
            None if self.json => Output::Json,
            None if self.sarif => Output::Sarif,
            None if self.git_hub => Output::Github,
            None if self.count => Output::Count,
            None if self.files_with_matches => Output::FilesWith,
            None if self.files_without_match => Output::FilesWithout,
            None => Output::Findings,
        });
        self.scan = Some(scan);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.path.is_empty() && self.literal_path.is_empty() {
            let read = self.part.of_text(&self.input_object);
            return self.report(ps, None, &read);
        }
        let (given, literal) =
            if self.literal_path.is_empty() { (self.path.clone(), false) } else { (self.literal_path.clone(), true) };
        let opts = trex::files::WalkOptions {
            hidden: self.hidden,
            no_ignore: self.no_ignore,
            globs: self.include.clone(),
            ..trex::files::WalkOptions::default()
        };
        opts.check().map_err(|e| arg_err("TrexWalk", e))?;
        let placed = self.places();
        for source in files_of(ps, &given, literal, &opts)? {
            if ps.stopping() {
                break;
            }
            let trex::files::Source::File(path) = source else {
                continue;
            };
            if self.fix || self.diff {
                let (raw, text) = match read_whole(&path, self.binary) {
                    Ok(Some(read)) => read,
                    Ok(None) => continue,
                    Err(e) => {
                        ps.write_error(&e)?;
                        continue;
                    }
                };
                self.fix_file(ps, &path, raw, &text)?;
                continue;
            }
            let read = match self.part.read(&path, self.binary, placed) {
                Ok(Some(read)) => read,
                Ok(None) => continue,
                Err(e) => {
                    ps.write_error(&e)?;
                    continue;
                }
            };
            let shown = path.display().to_string();
            if self.follow {
                self.to_follow.push((path, shown, read));
                continue;
            }
            self.report(ps, Some(&shown), &read)?;
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        if self.follow {
            return self.follow_files(ps);
        }
        if self.interactive && !self.pending.is_empty() {
            return match review(ps, &self.pending, self.context_lines(), self.show_skipped, None)? {
                Some(kept) => write_reviewed(ps, &self.pending, kept, "Apply", "fix(es)"),
                None => Ok(()),
            };
        }
        let (Some(scan), Some(output)) = (&self.scan, &self.output) else {
            return Err(arg_err("TrexNoRules", "the rules were not read"));
        };
        let text = self.path.is_empty() && self.literal_path.is_empty();
        match output {
            Output::Sarif => ps.write(scan.sarif(&self.kept.sarif_found))?,
            Output::Json => ps.write(format!("[{}]", self.kept.json_objects.join(",")))?,
            Output::Count => {
                if text {
                    ps.write(TrexMatchCount { path: String::new(), count: self.kept.text_found as i64 })?;
                }
            }
            Output::Findings
            | Output::JsonLines
            | Output::Github
            | Output::Format(..)
            | Output::FilesWith
            | Output::FilesWithout => {}
        }
        if self.require_match && self.kept.found_total == 0 {
            ps.write_error(&PsError::new(
                ErrorCategory::ObjectNotFound,
                "TrexNoFinding",
                "-RequireMatch: no input holds a finding",
            ))?;
        }
        Ok(())
    }
}

/// Renders findings as one SARIF 2.1.0 document, for a CI system or an
/// editor that reads one.
///
/// The rules the document lists are the findings' own, in the order they
/// first appear. A finding with no file carries no location URI and no fix,
/// since a fix in SARIF names the file it changes.
///
/// # Examples
/// Invoke-TrexRule -Path ./src -RuleFile ./rules | Where-Object Severity -ne Note | ConvertTo-TrexSarif
#[cmdlet(verb = "ConvertTo", noun = "TrexSarif", alias = "ConvertTo-TxSarif", output = ["System.String"])]
#[derive(Default)]
pub struct ConvertToTrexSarif {
    /// The findings to render.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub finding: Vec<TrexFinding>,
    gathered: Vec<TrexFinding>,
}

impl Cmdlet for ConvertToTrexSarif {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        self.gathered.extend(self.finding.iter().cloned());
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        let mut rules: Vec<trex::rule_scan::SarifRule> = Vec::new();
        let mut results = Vec::with_capacity(self.gathered.len());
        for f in &self.gathered {
            let definition = TrexRule::from_ps(&f.definition)?;
            let index = match rules.iter().position(|r| r.name == definition.name) {
                Some(i) => i,
                None => {
                    let mut meta: Vec<(String, String)> = Vec::new();
                    for tag in &definition.tags {
                        meta.push(("tags".to_string(), tag.clone()));
                    }
                    let table = <std::collections::HashMap<String, String> as FromPs>::from_ps(&definition.meta)?;
                    let mut keys: Vec<&String> = table.keys().filter(|k| k.as_str() != "tags").collect();
                    keys.sort();
                    for k in keys {
                        meta.push((k.clone(), table[k].clone()));
                    }
                    rules.push(trex::rule_scan::SarifRule {
                        name: definition.name.clone(),
                        message: definition.message.clone(),
                        pattern: definition.pattern.clone(),
                        severity: definition.severity.trex(),
                        meta,
                    });
                    rules.len() - 1
                }
            };
            let fix = if f.fix.is_null() {
                None
            } else {
                Some((String::from_ps(&f.fix)?, TrexRegion::from_ps(&f.fix_region)?.place()))
            };
            results.push(trex::rule_scan::SarifResult {
                rule: index,
                uri: if f.path.is_empty() { None } else { Some(f.path.replace('\\', "/")) },
                at: f.region.place(),
                snippet: f.text.clone(),
                message: f.message.clone(),
                fix,
            });
        }
        ps.write(trex::rule_scan::sarif(&rules, &results))
    }
}
