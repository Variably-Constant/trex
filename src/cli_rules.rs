//! `scan --rules`: the rules of pattern files scanned over the inputs through
//! [`trex::rule_scan`], every finding reported under its rule with the
//! message rendered from the match, its severity and its fix: as
//! compiler-style lines, JSON, one SARIF document or GitHub workflow
//! annotations, or with the fixes applied through the rewrite review.

use std::path::Path;
use std::process::ExitCode;

use trex::files::{LineIndex, Source, WalkOptions, collect};
use trex::rule_scan::{Finding, Found, Place, RuleScan, declare_rule_files};

use crate::cli_files::{Editing, across_cores, edit_sources, review_sources};
use crate::cli_window::{Part, Restrict, Windowing, read_part};

/// What `scan --rules` was asked.
pub(crate) struct RulesRun<'a> {
    /// The declarations `--lib` and `--shape` made, which the rule files
    /// add to.
    pub shapes: trex::ShapeSet,
    /// The `--rules` arguments: pattern files, and directories of them.
    pub rules: &'a [String],
    pub paths: Vec<String>,
    pub text: Option<Vec<u8>>,
    pub walk: WalkOptions,
    pub json: bool,
    pub sarif: bool,
    pub github: bool,
    pub format: Option<String>,
    pub count: bool,
    pub files_with_matches: bool,
    pub files_without_match: bool,
    pub max_count: Option<usize>,
    pub require_match: bool,
    pub binary: bool,
    pub fix: bool,
    pub dry_run: bool,
    pub interactive: bool,
    /// The lines of context a dry run's diff shows around each fix.
    pub context: usize,
    pub painter: trex::paint::Painter,
    /// `--head`, `--tail`, `--lines` and `--follow`: the part of each input
    /// scanned, and whether each file is followed as it grows.
    pub windowing: Windowing,
    /// What the window counts.
    pub unit: trex::records::RecordUnit,
}

/// What one input came to.
enum Outcome {
    Pending,
    Binary,
    Failed(String),
    Scanned(Found),
}

/// Which report the findings print as.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    Human,
    Json,
    Sarif,
    Github,
    Format,
    Count,
    FilesWith,
    FilesWithout,
}

pub(crate) fn run(mut run: RulesRun<'_>) -> ExitCode {
    if let Err(e) = declare_rule_files(&mut run.shapes, run.rules) {
        eprintln!("trex: {e}");
        return ExitCode::FAILURE;
    }
    if run.shapes.rules().is_empty() {
        eprintln!("trex scan: {} holds no rule", run.rules.join(", "));
        return ExitCode::FAILURE;
    }
    let scan = match RuleScan::new(run.shapes.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("trex: {e}");
            return ExitCode::FAILURE;
        }
    };
    let template = match &run.format {
        Some(src) => match trex::Template::parse_report(src, &scan.capture_names()) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("trex: --format error at byte {}: {}", e.pos, e.msg);
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let form = if run.json {
        Form::Json
    } else if run.sarif {
        Form::Sarif
    } else if run.github {
        Form::Github
    } else if template.is_some() {
        Form::Format
    } else if run.count {
        Form::Count
    } else if run.files_with_matches {
        Form::FilesWith
    } else if run.files_without_match {
        Form::FilesWithout
    } else {
        Form::Human
    };
    let report = Report { scan: &scan, form, template: template.as_ref(), painter: &run.painter };

    // An inline string is one input with no name; a window cuts it as it
    // cuts a file.
    if let Some(text) = &run.text {
        if !run.paths.is_empty() {
            eprintln!("trex scan: --text and a path cannot both be given");
            return ExitCode::FAILURE;
        }
        if run.fix {
            eprintln!("trex scan: --fix writes files and takes no --text");
            return ExitCode::FAILURE;
        }
        if run.files_with_matches || run.files_without_match {
            eprintln!("trex scan: -l and -L name files and take no --text");
            return ExitCode::FAILURE;
        }
        if run.windowing.follow {
            eprintln!("trex scan: --follow follows files by name and takes no --text");
            return ExitCode::FAILURE;
        }
        let part = Part::of_text(text.clone(), run.windowing.select, &run.unit);
        let findings = scan.findings(None, &part.text, &part.index(), run.max_count);
        let found = vec![found_of(None, part, findings)];
        let (total, errors) = report.print(&found, false);
        return exit(false, total, errors, run.require_match);
    }

    let a_directory = run.paths.iter().any(|p| Path::new(p).is_dir());
    let (mut sources, walk_errors) = collect(&run.paths, &run.walk);
    if sources.is_empty() && run.paths.is_empty() {
        sources.push(Source::Stdin);
    }
    for e in &walk_errors {
        eprintln!("trex: {e}");
    }
    let mut failed = !walk_errors.is_empty();

    if run.fix {
        if sources.contains(&Source::Stdin) {
            eprintln!("trex scan: --fix writes files and cannot write the standard input");
            return ExitCode::FAILURE;
        }
        let edits_of = |src: &Source, piece: crate::cli_window::Piece<'_>| {
            let name = src.name();
            let (edits, skipped) = scan.fix_edits(&name, piece.text, &piece.index());
            for s in skipped {
                eprintln!(
                    "trex: {name}:{}:{}: the fix of {} overlaps an earlier fix; skipped",
                    s.line,
                    s.col,
                    scan.rules()[s.rule].name
                );
            }
            edits
        };
        let how = Editing {
            failed,
            a_directory,
            binary: run.binary,
            verb: "fixes",
            dry_run: run.dry_run,
            context: run.context,
            restrict: Restrict::of(&run.windowing, &run.unit),
        };
        return if run.interactive {
            // A rule's review explains nothing under a fix: what a rule's
            // finding means is its own message, which the report already
            // prints, rather than the axes its pattern read.
            review_sources(&sources, how, None, false, edits_of)
        } else {
            // A rule's fixes carry nothing from one input to the next, so the
            // order the cores finish in is not visible in what they write.
            edit_sources(&sources, how, edits_of)
        };
    }

    // Every report here but the counts and the file lists places a finding
    // on its line and at its offsets, which a tail's lines and bytes ahead of
    // it are counted for, and a follow continues the tail's offsets.
    let numbers = !matches!(form, Form::Count | Form::FilesWith | Form::FilesWithout);
    let asked =
        trex::window::Asked { numbers, offsets: numbers || run.windowing.follow, binary: run.binary, units: false };
    if run.windowing.follow {
        if sources.contains(&Source::Stdin) {
            eprintln!(
                "trex scan: --follow follows files by name, and the standard input is read as it arrives already; name the file"
            );
            return ExitCode::FAILURE;
        }
        let prefixed = a_directory || sources.len() > 1;
        return crate::cli_follow::follow_rules(&sources, &run, &report, asked, prefixed, failed);
    }
    let mut outcomes: Vec<Outcome> = sources.iter().map(|_| Outcome::Pending).collect();
    across_cores(&mut outcomes, |k, slot| {
        let src = &sources[k];
        let part = match read_part(src, run.windowing.select, &run.unit, asked) {
            Ok(part) => part,
            Err(e) => {
                *slot = Outcome::Failed(format!("cannot read {}: {e}", src.name()));
                return;
            }
        };
        if !run.binary && part.binary {
            *slot = Outcome::Binary;
            return;
        }
        let name = src.name();
        let findings = scan.findings(Some(&name), &part.text, &part.index(), run.max_count);
        *slot = Outcome::Scanned(found_of(Some(name), part, findings));
    });
    let mut found: Vec<Found> = Vec::new();
    for (src, outcome) in sources.iter().zip(outcomes) {
        match outcome {
            Outcome::Pending => {}
            // A file named outright is refused for a NUL byte, as a scan
            // refuses it; a walk skips one without a word.
            Outcome::Binary => {
                if !a_directory {
                    eprintln!("trex: {} holds a NUL byte and is binary; --binary scans it", src.name());
                    failed = true;
                }
            }
            Outcome::Failed(e) => {
                eprintln!("trex: {e}");
                failed = true;
            }
            Outcome::Scanned(one) => found.push(one),
        }
    }
    let prefixed = a_directory || sources.len() > 1;
    let (total, errors) = report.print(&found, prefixed);
    exit(failed, total, errors, run.require_match)
}

/// One input's findings as the report reads them: its name, the part a
/// window took of it and where that stands, and the findings over it.
fn found_of(name: Option<String>, part: Part, findings: Vec<Finding>) -> Found {
    Found { name, input: part.text, byte_base: part.byte_base, line_base: part.line_base, findings }
}

/// The exit status: failure where an input could not be read, where a
/// finding is an error, or where `--require-match` found nothing.
fn exit(failed: bool, total: usize, errors: usize, require_match: bool) -> ExitCode {
    if failed || errors > 0 || (require_match && total == 0) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// How the findings print.
pub(crate) struct Report<'a> {
    pub(crate) scan: &'a RuleScan,
    form: Form,
    template: Option<&'a trex::Template>,
    painter: &'a trex::paint::Painter,
}

impl Report<'_> {
    /// Print every input's findings in the report's form, and count them
    /// and the errors among them. A count names its input where the scan
    /// runs over several.
    fn print(&self, found: &[Found], prefixed: bool) -> (usize, usize) {
        let mut objects: Vec<String> = Vec::new();
        let (total, errors) = self.print_findings(found, prefixed, &mut |object| objects.push(object));
        match self.form {
            Form::Json => crate::out::line(&format!("[{}]", objects.join(","))),
            Form::Sarif => crate::out::line(&self.scan.sarif(found)),
            Form::Human if total == 0 => crate::out::line("no finding"),
            _ => {}
        }
        (total, errors)
    }

    /// Print the findings of `found` in the report's form, each line as it
    /// is made, and count them and the errors among them; each JSON object
    /// goes to `json`, which [`Self::print`] gathers into one array and a
    /// followed file prints a line at a time as its findings settle. A SARIF
    /// document and the line saying there were none are written once, by
    /// [`Self::print`].
    pub(crate) fn print_findings(
        &self,
        found: &[Found],
        prefixed: bool,
        json: &mut dyn FnMut(String),
    ) -> (usize, usize) {
        use trex::paint::Role;
        let rules = self.scan.rules();
        let mut total = 0usize;
        let mut errors = 0usize;
        for one in found {
            let Found { name, input, findings, .. } = one;
            total += findings.len();
            errors += findings.iter().filter(|f| rules[f.rule].severity == trex::Severity::Error).count();
            let index = one.index();
            let name = name.as_deref();
            match self.form {
                Form::Count => match name {
                    Some(n) if prefixed => crate::out::line(&format!(
                        "{}{}{}",
                        self.painter.paint(Role::Path, n),
                        self.painter.paint(Role::Separator, ":"),
                        findings.len()
                    )),
                    _ => crate::out::line(&findings.len().to_string()),
                },
                Form::FilesWith => {
                    if let Some(n) = name
                        && !findings.is_empty()
                    {
                        crate::out::line(&self.painter.paint(Role::Path, n));
                    }
                }
                Form::FilesWithout => {
                    if let Some(n) = name
                        && findings.is_empty()
                    {
                        crate::out::line(&self.painter.paint(Role::Path, n));
                    }
                }
                Form::Json => {
                    for f in findings {
                        json(self.scan.finding_json(name, input, &index, f));
                    }
                }
                Form::Sarif => {}
                Form::Github => {
                    for f in findings {
                        crate::out::line(&self.scan.github_line(name, input, &index, f));
                    }
                }
                Form::Format => {
                    let Some(template) = self.template else {
                        continue;
                    };
                    for f in findings {
                        let rule = &rules[f.rule];
                        let place = Place::of(&index, input, f.start, f.end);
                        let at = trex::ReportAt {
                            path: name.unwrap_or(""),
                            line: place.line,
                            col: place.col,
                            base: index.base(),
                            pattern: Some(&rule.name),
                            rule: Some(trex::ReportRule {
                                name: &rule.name,
                                severity: rule.severity.name(),
                                message: &f.message,
                                fix: f.fix.as_deref().unwrap_or(""),
                            }),
                        };
                        crate::out::line(&template.render_report(&f.m, input, &at));
                    }
                }
                Form::Human => {
                    for f in findings {
                        crate::out::line(&self.human_line(name, input, &index, f));
                        if let Some(fix) = &f.fix {
                            crate::out::line(&format!("  fix: {fix:?}"));
                        }
                    }
                }
            }
        }
        (total, errors)
    }

    /// `path:line:col: severity: message [rule]`, the path where the input
    /// has a name, painted by role.
    fn human_line(&self, name: Option<&str>, input: &[u8], index: &LineIndex, f: &Finding) -> String {
        use trex::paint::Role;
        let rule = &self.scan.rules()[f.rule];
        let (line, col) = index.line_col(input, f.start);
        let sep = |s: &str| self.painter.paint(Role::Separator, s);
        let mut out = String::new();
        if let Some(n) = name {
            out.push_str(&self.painter.paint(Role::Path, n));
            out.push_str(&sep(":"));
        }
        out.push_str(&self.painter.paint(Role::Line, &line.to_string()));
        out.push_str(&sep(":"));
        out.push_str(&self.painter.paint(Role::Column, &col.to_string()));
        out.push_str(&sep(":"));
        out.push(' ');
        out.push_str(rule.severity.name());
        out.push_str(": ");
        out.push_str(&f.message);
        out.push_str(" [");
        out.push_str(&rule.name);
        out.push(']');
        out
    }
}
