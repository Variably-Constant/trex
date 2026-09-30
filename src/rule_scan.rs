//! The rules of pattern files scanned over an input: the rules that fire on
//! each match as one set per list of files they read, and the rules that
//! fire on records each as a record query, every finding reported under its
//! rule with the message rendered from the match, its severity and its fix.
//!
//! What the trex command's `scan --rules` reports, and what every surface
//! reads findings through, so a finding, a fix and a SARIF document read
//! alike wherever they are asked for.

use std::path::{Path, PathBuf};

use crate::custom::{Rule, Severity, ShapeSet};
use crate::files::{Edit, LineIndex};
use crate::records::{Quantifier, Query};
use crate::typed::json_string;

/// One finding: the rule, by its index among the set's rules, the span the
/// finding covers (the match, or the record for a rule that fires on
/// records), the match the message and the fix read, and both rendered.
#[derive(Clone, Debug)]
pub struct Finding {
    pub rule: usize,
    pub start: usize,
    pub end: usize,
    pub m: crate::Match,
    pub message: String,
    pub fix: Option<String>,
}

/// Where a span stands: its first line and column and the line and column
/// just past its end, each from one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

impl Place {
    /// Where `start..end` stands in `input`.
    #[must_use]
    pub fn of(index: &LineIndex, input: &[u8], start: usize, end: usize) -> Place {
        let (line, col) = index.line_col(input, start);
        let (end_line, end_col) = index.line_col(input, end);
        Place { line, col, end_line, end_col }
    }
}

/// One input's findings as a report reads them: its name where it has one,
/// its text, where that text stands in the input it was read from, and the
/// findings over it.
#[derive(Clone, Debug)]
pub struct Found {
    pub name: Option<String>,
    pub input: Vec<u8>,
    /// Where `input` begins in the input it was read from: 0 for a whole
    /// input, the window's first byte for a head, a tail or a range of one,
    /// where that was counted.
    pub byte_base: Option<usize>,
    /// How many lines of that input come before `input`, where they were
    /// counted: a report placing no finding on its line reads a tail
    /// without them.
    pub line_base: Option<usize>,
    pub findings: Vec<Finding>,
}

impl Found {
    /// The index a report of these findings reads lines, columns and offsets
    /// from, numbered as the input they were read from numbers them.
    #[must_use]
    pub fn index(&self) -> LineIndex {
        LineIndex::new(&self.input).within(self.byte_base, self.line_base)
    }
}

/// A fix left out because it overlaps an earlier one: the rule whose fix it
/// is and where its match starts, from one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkippedFix {
    pub rule: usize,
    pub line: usize,
    pub col: usize,
}

/// The rules that fire on each match and share one list of files, as one
/// set, and the rules that fire on records under the same list, each its
/// own query.
struct Group {
    files: Vec<String>,
    /// The globs, built; nothing where the group reads every input.
    accepts: Option<ignore::overrides::Override>,
    /// The rules firing on each match: member `k` of the set is rule
    /// `plain[k]`.
    plain: Vec<usize>,
    set: Option<crate::PatternSet>,
    /// Whether the set's matches carry every binding under a repetition,
    /// because a message or a fix reads one by index.
    lists: bool,
    /// The rules firing on records.
    records: Vec<usize>,
}

impl Group {
    fn new(files: &[String]) -> Result<Group, String> {
        let accepts = if files.is_empty() {
            None
        } else {
            let mut builder = ignore::overrides::OverrideBuilder::new(".");
            for glob in files {
                builder.add(glob).map_err(|e| format!("files glob {glob:?}: {e}"))?;
            }
            Some(builder.build().map_err(|e| format!("files globs: {e}"))?)
        };
        Ok(Group { files: files.to_vec(), accepts, plain: Vec::new(), set: None, lists: false, records: Vec::new() })
    }

    /// Whether the group reads the input named `name`: every input where it
    /// names no files; a named one its globs keep; never an unnamed one.
    fn reads(&self, name: Option<&str>) -> bool {
        match (&self.accepts, name) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(globs), Some(name)) => !globs.matched(name, false).is_ignore(),
        }
    }
}

/// The rules of a set, prepared to scan: each rule's message and fix as
/// templates parsed once, its record query where it fires on records, and
/// the rules grouped by the files they read.
pub struct RuleScan {
    shapes: ShapeSet,
    messages: Vec<crate::Template>,
    fixes: Vec<Option<crate::Template>>,
    queries: Vec<Option<Query>>,
    groups: Vec<Group>,
}

impl RuleScan {
    /// The rules `shapes` declares, prepared to scan.
    ///
    /// # Errors
    ///
    /// A rule's message or fix that does not parse, or a files glob that
    /// does not, each named with its rule.
    pub fn new(shapes: ShapeSet) -> Result<Self, String> {
        let rules = shapes.rules();
        let mut messages = Vec::with_capacity(rules.len());
        let mut fixes = Vec::with_capacity(rules.len());
        let mut queries = Vec::with_capacity(rules.len());
        for rule in rules {
            let bound = rule.pattern.capture_names();
            messages.push(
                crate::Template::parse_report(&rule.message, &bound)
                    .map_err(|e| format!("rule {}: message error at byte {}: {}", rule.name, e.pos, e.msg))?,
            );
            fixes.push(match &rule.fix {
                Some(fix) => Some(
                    crate::Template::parse(fix, &bound)
                        .map_err(|e| format!("rule {}: fix error at byte {}: {}", rule.name, e.pos, e.msg))?,
                ),
                None => None,
            });
            queries.push(rule.on_records().then(|| Query {
                quantifier: Quantifier::All,
                positives: vec![rule.pattern.clone()],
                set: None,
                negatives: rule.unless.clone(),
                unit: rule.record_unit(),
            }));
        }
        let mut groups: Vec<Group> = Vec::new();
        for (i, rule) in rules.iter().enumerate() {
            let k = match groups.iter().position(|g| g.files == rule.files) {
                Some(k) => k,
                None => {
                    groups.push(Group::new(&rule.files).map_err(|e| format!("rule {}: {e}", rule.name))?);
                    groups.len() - 1
                }
            };
            if rule.on_records() {
                groups[k].records.push(i);
            } else {
                groups[k].plain.push(i);
            }
        }
        for group in &mut groups {
            if group.plain.is_empty() {
                continue;
            }
            let pats = group.plain.iter().map(|&i| rules[i].pattern.clone()).collect();
            let names = group.plain.iter().map(|&i| rules[i].name.clone()).collect();
            group.set = Some(crate::PatternSet::named(pats, names).under(shapes.clone()));
            group.lists = group.plain.iter().any(|&i| reads_lists(rules, &messages, &fixes, i));
        }
        Ok(RuleScan { shapes, messages, fixes, queries, groups })
    }

    /// The rules, in declaration order; a finding's `rule` indexes them.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        self.shapes.rules()
    }

    /// The register names a report template over the findings may write:
    /// every rule's, in rule order, each once.
    #[must_use]
    pub fn capture_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for rule in self.rules() {
            for name in rule.pattern.capture_names() {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        names
    }

    /// Every finding over `input`, named `name` where it has a name, in
    /// position order, then by rule; the first `cap` where one is given.
    #[must_use]
    pub fn findings(&self, name: Option<&str>, input: &[u8], index: &LineIndex, cap: Option<usize>) -> Vec<Finding> {
        let mut out = Vec::new();
        for group in &self.groups {
            if !group.reads(name) {
                continue;
            }
            if let Some(set) = &group.set {
                for (k, m) in set.scan_matches(input, group.lists) {
                    let i = group.plain[k];
                    out.push(self.finding(i, m.start, m.end, m, name, input, index));
                }
            }
            for &i in &group.records {
                let Some(query) = &self.queries[i] else {
                    continue;
                };
                let lists = reads_lists(self.rules(), &self.messages, &self.fixes, i);
                for hit in query.hits(input, &self.shapes, true) {
                    // The message and the fix read the rule's first match in
                    // the record.
                    let Some(&(s, e)) = hit.spans.first() else {
                        continue;
                    };
                    let at = |offset: usize| u32::try_from(offset).expect("an input offset fits a span");
                    let span = [crate::Span { start: at(s), end: at(e) }];
                    let pattern = &self.rules()[i].pattern;
                    let resolved = if lists {
                        crate::captures_with_shapes_and_lists(pattern, input, &self.shapes, &span)
                    } else {
                        crate::captures_with_shapes(pattern, input, &self.shapes, &span)
                    };
                    let Some(m) = resolved.into_iter().next() else {
                        continue;
                    };
                    out.push(self.finding(i, hit.start, hit.end, m, name, input, index));
                }
            }
        }
        out.sort_by_key(|f| (f.start, f.end, f.rule));
        if let Some(n) = cap {
            out.truncate(n);
        }
        out
    }

    /// One finding of rule `i` covering `start..end`, its message and fix
    /// rendered from the match `m`.
    #[allow(clippy::too_many_arguments)]
    fn finding(
        &self,
        i: usize,
        start: usize,
        end: usize,
        m: crate::Match,
        name: Option<&str>,
        input: &[u8],
        index: &LineIndex,
    ) -> Finding {
        let rule = &self.rules()[i];
        let (line, col) = index.line_col(input, m.start);
        let place = crate::ReportAt {
            path: name.unwrap_or(""),
            line,
            col,
            base: index.base(),
            pattern: Some(&rule.name),
            rule: Some(crate::ReportRule { name: &rule.name, severity: rule.severity.name(), message: "", fix: "" }),
        };
        let message = self.messages[i].render_report(&m, input, &place);
        let fix = self.fixes[i].as_ref().map(|t| t.render(&m, input));
        Finding { rule: i, start, end, m, message, fix }
    }

    /// The rules that fire on records, by name: a stream scans none of them,
    /// a record being whole only once the input holding it has ended.
    #[must_use]
    pub fn record_rules(&self) -> Vec<&str> {
        self.rules().iter().filter(|r| r.on_records()).map(|r| r.name.as_str()).collect()
    }

    /// A stream of the input named `name`, where it has one, whose first
    /// byte stands at `origin` after `lines_before` of its lines, where those
    /// were counted: the rules firing on each match that read it, each list
    /// of files' rules scanned as one set.
    #[must_use]
    pub fn stream(&self, name: Option<&str>, origin: usize, lines_before: Option<usize>) -> RuleStream<'_> {
        let groups = self
            .groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.reads(name))
            .filter_map(|(k, g)| {
                g.set.as_ref().map(|set| {
                    (k, crate::HeldStream::new(crate::StreamScanner::over_set(set.clone()), origin, lines_before))
                })
            })
            .collect();
        RuleStream { scan: self, name: name.map(str::to_string), groups }
    }

    /// The findings group `group`'s set makes at `committed`, spans over
    /// `held` with the member that made each, placed by `index`, in position
    /// order and then by rule.
    fn findings_at(
        &self,
        group: usize,
        held: &[u8],
        index: &LineIndex,
        committed: &[(usize, crate::Span)],
        name: Option<&str>,
    ) -> Vec<Finding> {
        let g = &self.groups[group];
        let mut members: Vec<usize> = committed.iter().map(|&(member, _)| member).collect();
        members.sort_unstable();
        members.dedup();
        let mut out = Vec::with_capacity(committed.len());
        for member in members {
            let spans: Vec<crate::Span> =
                committed.iter().filter(|&&(m, _)| m == member).map(|&(_, span)| span).collect();
            let i = g.plain[member];
            let pattern = &self.rules()[i].pattern;
            let resolved = if g.lists {
                crate::captures_with_shapes_and_lists(pattern, held, &self.shapes, &spans)
            } else {
                crate::captures_with_shapes(pattern, held, &self.shapes, &spans)
            };
            for m in resolved {
                out.push(self.finding(i, m.start, m.end, m, name, held, index));
            }
        }
        out.sort_by_key(|f| (f.start, f.end, f.rule));
        out
    }

    /// The edits the fixes make to the input named `name`: each finding's
    /// fix in its match's place, in position order, and the fixes left out
    /// for overlapping an earlier one, each placed by `index`, the line
    /// index of `input`.
    #[must_use]
    pub fn fix_edits(&self, name: &str, input: &[u8], index: &LineIndex) -> (Vec<Edit>, Vec<SkippedFix>) {
        let mut edits = Vec::new();
        let mut skipped = Vec::new();
        let mut last_end = 0usize;
        for f in self.findings(Some(name), input, index, None) {
            let Some(fix) = f.fix else {
                continue;
            };
            if f.m.start < last_end {
                let (line, col) = index.line_col(input, f.m.start);
                skipped.push(SkippedFix { rule: f.rule, line, col });
                continue;
            }
            last_end = f.m.end;
            edits.push(Edit { start: f.m.start, end: f.m.end, replacement: fix.into_bytes() });
        }
        (edits, skipped)
    }

    /// Every input's findings as one SARIF 2.1.0 document.
    #[must_use]
    pub fn sarif(&self, found: &[Found]) -> String {
        let rules: Vec<SarifRule> = self.rules().iter().map(SarifRule::of).collect();
        let mut results: Vec<SarifResult> = Vec::new();
        for one in found {
            let index = one.index();
            let Found { name, input, findings, .. } = one;
            let uri = name.as_deref().map(|n| n.replace('\\', "/"));
            for f in findings {
                results.push(SarifResult {
                    rule: f.rule,
                    uri: uri.clone(),
                    at: Place::of(&index, input, f.start, f.end),
                    snippet: String::from_utf8_lossy(&input[f.start..f.end]).into_owned(),
                    message: f.message.clone(),
                    fix: f.fix.clone().map(|fix| (fix, Place::of(&index, input, f.m.start, f.m.end))),
                });
            }
        }
        sarif(&rules, &results)
    }

    /// One finding as a GitHub workflow annotation: `::LEVEL
    /// file=,line=,col=,endLine=,endColumn=,title=RULE::message`, the file
    /// where the input has a name.
    #[must_use]
    pub fn github_line(&self, name: Option<&str>, input: &[u8], index: &LineIndex, f: &Finding) -> String {
        let rule = &self.rules()[f.rule];
        github_annotation(
            rule.severity,
            &rule.name,
            name,
            Place::of(index, input, f.start, f.end),
            &f.message,
        )
    }

    /// One finding as the JSON object `scan --rules --json` writes: its rule,
    /// severity and message, its path where the input has a name, where it
    /// stands, its text and captures, its fix and the rule's metadata.
    #[must_use]
    pub fn finding_json(&self, name: Option<&str>, input: &[u8], index: &LineIndex, f: &Finding) -> String {
        use crate::report::{captures_json, json_escape};
        let rule = &self.rules()[f.rule];
        let place = Place::of(index, input, f.start, f.end);
        let mut out = format!(
            "{{\"rule\":\"{}\",\"severity\":\"{}\",\"message\":\"{}\"",
            json_escape(&rule.name),
            rule.severity.name(),
            json_escape(&f.message)
        );
        if let Some(n) = name {
            out.push_str(&format!(",\"path\":\"{}\"", json_escape(n)));
        }
        out.push_str(&format!(
            ",\"line\":{},\"col\":{},\"end_line\":{},\"end_col\":{},\"start\":{},\"end\":{},\"text\":\"{}\",\"captures\":{}",
            place.line,
            place.col,
            place.end_line,
            place.end_col,
            index.offset(f.start),
            index.offset(f.end),
            json_escape(&String::from_utf8_lossy(&input[f.start..f.end])),
            // A rule finding reports its captures as text. The rule file
            // names the kinds, not the caller, so there is no choice of value
            // spelling to read here.
            captures_json(input, &f.m, "", None, None)
        ));
        if let Some(fix) = &f.fix {
            out.push_str(&format!(",\"fix\":\"{}\"", json_escape(fix)));
        }
        if !rule.meta.is_empty() {
            let members: Vec<String> = rule
                .meta
                .iter()
                .map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v)))
                .collect();
            out.push_str(&format!(",\"meta\":{{{}}}", members.join(",")));
        }
        out.push('}');
        out
    }
}

/// The rules that fire on each match, scanned over one input as it arrives:
/// each list of files' rules streamed as one set, and each finding made once
/// nothing that arrives later can change its match.
pub struct RuleStream<'a> {
    scan: &'a RuleScan,
    name: Option<String>,
    /// Each group reading the input that holds rules firing on matches, by
    /// its place among the scan's groups, with its stream.
    groups: Vec<(usize, crate::HeldStream)>,
}

impl RuleStream<'_> {
    /// Whether a finding can be made before the input ends.
    #[must_use]
    pub fn commits_early(&self) -> bool {
        self.groups.iter().all(|(_, stream)| stream.commits_early())
    }

    /// Scan the input's next bytes: the findings they settle, a batch for
    /// each list of files, each with the bytes it was found in and where
    /// those stand in the input.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Found> {
        let scan = self.scan;
        let name = self.name.as_deref();
        let mut out = Vec::new();
        for (group, stream) in &mut self.groups {
            let committed = stream.push(bytes);
            if committed.is_empty() {
                continue;
            }
            let index = LineIndex::new(stream.held()).within(Some(stream.base()), stream.lines_before());
            let findings = scan.findings_at(*group, stream.held(), &index, &committed, name);
            out.push(Found {
                name: name.map(str::to_string),
                input: stream.held().to_vec(),
                byte_base: Some(stream.base()),
                line_base: stream.lines_before(),
                findings,
            });
        }
        out
    }

    /// End the input: the findings the scanners held until then.
    #[must_use]
    pub fn finish(self) -> Vec<Found> {
        let RuleStream { scan, name, groups } = self;
        let mut out = Vec::new();
        for (group, stream) in groups {
            let ended = stream.finish();
            if ended.matches.is_empty() {
                continue;
            }
            let index = LineIndex::new(&ended.held).within(Some(ended.base), ended.lines_before);
            let findings = scan.findings_at(group, &ended.held, &index, &ended.matches, name.as_deref());
            out.push(Found {
                name: name.clone(),
                input: ended.held,
                byte_base: Some(ended.base),
                line_base: ended.lines_before,
                findings,
            });
        }
        out
    }
}

/// Whether rule `i`'s matches must carry every binding under a repetition:
/// its pattern binds one and its message or its fix reads one by index.
fn reads_lists(rules: &[Rule], messages: &[crate::Template], fixes: &[Option<crate::Template>], i: usize) -> bool {
    rules[i].pattern.has_list_registers()
        && (messages[i].reads_lists() || fixes[i].as_ref().is_some_and(crate::Template::reads_lists))
}

/// The rule files `paths` name, in order: a file as it is, a directory as
/// every `.trex` file under it in path order.
///
/// # Errors
///
/// A directory with no `.trex` file under it, or one that cannot be read.
pub fn rule_files(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for arg in paths {
        let path = Path::new(arg);
        if path.is_dir() {
            let mut found = Vec::new();
            rule_files_under(path, &mut found)?;
            found.sort();
            if found.is_empty() {
                return Err(format!("{arg}: no .trex file under it"));
            }
            out.extend(found);
        } else {
            out.push(path.to_path_buf());
        }
    }
    Ok(out)
}

/// Declare every rule file named into `shapes`, as [`rule_files`] reads the
/// paths.
///
/// # Errors
///
/// What [`rule_files`] refuses, or a file the set refuses.
pub fn declare_rule_files(shapes: &mut ShapeSet, paths: &[String]) -> Result<(), String> {
    for file in rule_files(paths)? {
        shapes.declare_file(&file).map_err(|e| e.msg)?;
    }
    Ok(())
}

/// Every `.trex` file under `dir`, into `out`, directories entered as met.
fn rule_files_under(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            rule_files_under(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "trex") {
            out.push(path);
        }
    }
    Ok(())
}

/// A rule as a SARIF document describes it: its id, its message template as
/// the short description, its pattern as the full one, its severity as the
/// default level, and its metadata as properties.
#[derive(Clone, Debug)]
pub struct SarifRule {
    pub name: String,
    pub message: String,
    pub pattern: String,
    pub severity: Severity,
    pub meta: Vec<(String, String)>,
}

impl SarifRule {
    /// The rule as SARIF describes it.
    #[must_use]
    pub fn of(rule: &Rule) -> SarifRule {
        SarifRule {
            name: rule.name.clone(),
            message: rule.message.clone(),
            pattern: rule.source.clone(),
            severity: rule.severity,
            meta: rule.meta.clone(),
        }
    }
}

/// One finding as a SARIF result reads it: its rule, by index among the
/// document's rules, the input's URI where it has a name, where it stands,
/// its text, its message, and its fix with the region the fix replaces.
#[derive(Clone, Debug)]
pub struct SarifResult {
    pub rule: usize,
    pub uri: Option<String>,
    pub at: Place,
    pub snippet: String,
    pub message: String,
    pub fix: Option<(String, Place)>,
}

/// Findings as one SARIF 2.1.0 document: the rules under the tool with
/// their severity, pattern and metadata, `meta.tags` as the tags, and each
/// finding a result with its rule, level, message, region and fix. A fix is
/// an artifact change, which names the artifact, so a result with no URI
/// carries none.
#[must_use]
pub fn sarif(rules: &[SarifRule], results: &[SarifResult]) -> String {
    let rule_objects: Vec<String> = rules.iter().map(sarif_rule).collect();
    let result_objects: Vec<String> = results.iter().map(|r| sarif_result(rules, r)).collect();
    format!(
        "{{\"$schema\":\"https://json.schemastore.org/sarif-2.1.0.json\",\"version\":\"2.1.0\",\"runs\":[{{\"tool\":{{\"driver\":{{\"name\":\"trex\",\"version\":{},\"informationUri\":\"https://github.com/Variably-Constant/trex\",\"rules\":[{}]}}}},\"results\":[{}]}}]}}",
        json_string(crate::version()),
        rule_objects.join(","),
        result_objects.join(",")
    )
}

fn sarif_rule(rule: &SarifRule) -> String {
    let mut out = format!(
        "{{\"id\":{},\"shortDescription\":{{\"text\":{}}},\"fullDescription\":{{\"text\":{}}},\"defaultConfiguration\":{{\"level\":\"{}\"}}",
        json_string(&rule.name),
        json_string(&rule.message),
        json_string(&rule.pattern),
        rule.severity.name()
    );
    if !rule.meta.is_empty() {
        let mut members: Vec<String> = Vec::new();
        let tags = crate::custom::tags_of(&rule.meta);
        if !tags.is_empty() {
            let quoted: Vec<String> = tags.iter().map(|t| json_string(t)).collect();
            members.push(format!("\"tags\":[{}]", quoted.join(",")));
        }
        for (k, v) in &rule.meta {
            if k != "tags" {
                members.push(format!("{}:{}", json_string(k), json_string(v)));
            }
        }
        out.push_str(&format!(",\"properties\":{{{}}}", members.join(",")));
    }
    out.push('}');
    out
}

fn sarif_result(rules: &[SarifRule], r: &SarifResult) -> String {
    let (name, level) = match rules.get(r.rule) {
        Some(rule) => (rule.name.as_str(), rule.severity.name()),
        None => ("", Severity::Warning.name()),
    };
    let region = format!(
        "{{\"startLine\":{},\"startColumn\":{},\"endLine\":{},\"endColumn\":{},\"snippet\":{{\"text\":{}}}}}",
        r.at.line,
        r.at.col,
        r.at.end_line,
        r.at.end_col,
        json_string(&r.snippet)
    );
    let artifact = r.uri.as_deref().map(|u| format!("\"artifactLocation\":{{\"uri\":{}}},", json_string(u)));
    let mut out = format!(
        "{{\"ruleId\":{},\"ruleIndex\":{},\"level\":\"{level}\",\"message\":{{\"text\":{}}},\"locations\":[{{\"physicalLocation\":{{{}\"region\":{region}}}}}]",
        json_string(name),
        r.rule,
        json_string(&r.message),
        artifact.as_deref().unwrap_or("")
    );
    if let (Some((fix, deleted)), Some(u)) = (&r.fix, &r.uri) {
        out.push_str(&format!(
            ",\"fixes\":[{{\"description\":{{\"text\":{}}},\"artifactChanges\":[{{\"artifactLocation\":{{\"uri\":{}}},\"replacements\":[{{\"deletedRegion\":{{\"startLine\":{},\"startColumn\":{},\"endLine\":{},\"endColumn\":{}}},\"insertedContent\":{{\"text\":{}}}}}]}}]}}]",
            json_string(name),
            json_string(u),
            deleted.line,
            deleted.col,
            deleted.end_line,
            deleted.end_col,
            json_string(fix)
        ));
    }
    out.push('}');
    out
}

/// A finding as a GitHub workflow annotation: `::LEVEL
/// file=,line=,col=,endLine=,endColumn=,title=RULE::message`, the file
/// where the input has a name.
#[must_use]
pub fn github_annotation(severity: Severity, rule: &str, file: Option<&str>, at: Place, message: &str) -> String {
    let mut props: Vec<String> = Vec::new();
    if let Some(n) = file {
        props.push(format!("file={}", github_property(&n.replace('\\', "/"))));
    }
    props.push(format!("line={}", at.line));
    props.push(format!("col={}", at.col));
    props.push(format!("endLine={}", at.end_line));
    props.push(format!("endColumn={}", at.end_col));
    props.push(format!("title={}", github_property(rule)));
    format!("::{} {}::{}", severity.github(), props.join(","), github_data(message))
}

/// A message as a workflow command carries it: `%`, a carriage return and a
/// newline escaped.
fn github_data(text: &str) -> String {
    text.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A")
}

/// A property value as a workflow command carries it: as the data, with `:`
/// and `,` escaped too.
fn github_property(text: &str) -> String {
    github_data(text).replace(':', "%3A").replace(',', "%2C")
}
