//! Rewriting and redacting what a pattern matches.
//!
//! Text piped in comes back rewritten. A file named with `-Path` comes back
//! as its rewritten text, or with `-InPlace` is written back where it has a
//! match, or with `-Diff` is described by the unified diff it would take, or
//! with `-Interactive` has each change put to the person at the host's
//! prompt first. A script block given in place of a template runs once per
//! match on the pipeline thread, with the match as `$_` and as its first
//! argument, the way a `-replace` script block reads one.
//!
//! -Head, -Tail and -Lines confine the edits to a part of each input: text
//! written out is that part alone, and a file written back or described by
//! its diff changes only there. -Follow writes a file's edited text as it
//! grows, a line at a time.

use std::path::{Path, PathBuf};

use pwrs::prelude::*;

use crate::atoms::TrexLibrary;
use crate::common::{arg_err, units_of};
use crate::follow::{EditsAt, FollowedEdit};
use crate::matching::{Backend, files_of, read_whole, resolved};
use crate::pattern::{Compiled, Engine, TrexMatch, pattern_arg, splice};
use crate::window::{Part, read_file};

/// The edits made to a text: from the text, and the UTF-16 code units of the
/// input ahead of it where they are counted, to edits of the text.
type EditsOf<'a> = dyn FnMut(&str, Option<usize>) -> PsResult<Vec<trex::files::Edit>> + 'a;

/// The template -Template gives, read against the pattern's registers.
fn parse_template(c: &Compiled, template: &str) -> PsResult<trex::Template> {
    trex::Template::parse(template, &c.inner.capture_names())
        .map_err(|e| arg_err("TrexTemplateError", format!("template error at byte {}: {}", e.pos, e.msg)))
}

/// The edits a script block makes at `found`, matches each beside the byte
/// offset it starts at: it runs once per match with the match as `$_` and as
/// `$args[0]`, and its last output, as a string, replaces it. A match's Start
/// counts from the input's start, `ahead` UTF-16 code units before the text
/// the matches were found in.
fn block_edits(
    block: &PsScriptBlock,
    found: Vec<(usize, TrexMatch)>,
    ahead: Option<usize>,
) -> PsResult<Vec<trex::files::Edit>> {
    let ahead = ahead.ok_or_else(crate::window::unplaced)?;
    let mut edits = Vec::with_capacity(found.len());
    for (start, m) in found {
        let end = start + m.text.len();
        let replacement = run_block(block, m.moved(ahead))?;
        edits.push(trex::files::Edit { start, end, replacement: replacement.into_bytes() });
    }
    Ok(edits)
}

/// The block run with `m` as `$_` and as its first argument, and its last
/// output as a string; an empty string where it wrote nothing.
fn run_block(block: &PsScriptBlock, m: TrexMatch) -> PsResult<String> {
    let arg = m.into_ps()?;
    let variables =
        PsType::from_name("System.Collections.Generic.List[System.Management.Automation.PSVariable]").new(&[])?;
    let dollar_under =
        PsType::from_name("System.Management.Automation.PSVariable").new(&["_".to_string().into_ps()?, arg.clone()])?;
    variables.call("Add", &[dollar_under])?;
    let functions = PsHashtable::new()?.into_ps()?;
    let results = block.0.call("InvokeWithContext", &[functions, variables, arg])?;
    let outputs = <Vec<PsObject> as FromPs>::from_ps(&results)?;
    match outputs.last() {
        Some(last) if !last.is_null() => String::from_ps(last),
        Some(_) | None => Ok(String::new()),
    }
}

/// `held`, the text a followed file's stream holds, as a string.
fn held_text(held: &[u8]) -> PsResult<&str> {
    std::str::from_utf8(held)
        .map_err(|e| PsError::new(ErrorCategory::InvalidData, "TrexFollow", format!("the text followed is not UTF-8: {e}")))
}

/// Where the result for a file goes.
#[derive(Clone, Copy)]
enum FileOut {
    /// Out as the file's rewritten text.
    Text,
    /// Out as the unified diff the file would take.
    Diff,
    /// Back into the file, where it has a match.
    InPlace,
    /// Back into the file, each change first put to the person at the
    /// host's prompt; with the places a template answer skipped listed, and a
    /// scan's explanation under each change, where asked.
    Review { show_skipped: bool, explain: bool },
}

/// The lines a review writes under a change to explain it: from the text the
/// change is in and where its match starts and ends.
pub(crate) type ExplainChange<'a> = dyn Fn(&[u8], usize, usize) -> Vec<String> + 'a;

/// The explanation of a match of `c` at `start..end` of `text`, as the lines
/// `scan --explain` prints under a match, `what` naming the scan that found it.
fn explanation_lines(c: &Compiled, text: &[u8], start: usize, end: usize, what: &str) -> Vec<String> {
    let explainer = trex::explain::Explainer::new(&c.inner, text, &c.shapes);
    trex::report::explanation_lines(&explainer.explain(&trex::Match::plain(start, end), what))
}

/// Which of -InPlace, -Diff and -Interactive was asked for, and what a review
/// writes beside each change: -ShowSkipped and -Explain, which only a review
/// reads.
fn file_out(in_place: bool, diff: bool, interactive: bool, show_skipped: bool, explain: bool) -> PsResult<FileOut> {
    let mut asked = Vec::new();
    if in_place {
        asked.push("-InPlace");
    }
    if diff {
        asked.push("-Diff");
    }
    if interactive {
        asked.push("-Interactive");
    }
    if let [a, b, ..] = asked.as_slice() {
        return Err(arg_err("TrexFileOut", format!("{a} and {b} each say what becomes of the files; give one")));
    }
    if show_skipped && !interactive {
        return Err(arg_err(
            "TrexShowSkipped",
            "-ShowSkipped lists the changes a template answer skipped, which only -Interactive gives",
        ));
    }
    if explain && !interactive {
        return Err(arg_err(
            "TrexExplain",
            "-Explain puts a scan's explanation under each change -Interactive asks about; give -Interactive",
        ));
    }
    Ok(if in_place {
        FileOut::InPlace
    } else if diff {
        FileOut::Diff
    } else if interactive {
        FileOut::Review { show_skipped, explain }
    } else {
        FileOut::Text
    })
}

/// Write `bytes` over the file at `path`.
fn write_back(path: &Path, bytes: &[u8]) -> PsResult<()> {
    std::fs::write(path, bytes)
        .map_err(|e| PsError::new(ErrorCategory::WriteError, "TrexWrite", format!("{}: {e}", path.display())))
}

/// What a read of one file found, its error written to the error stream as a
/// read that found nothing, so the files after it are still read. A file left
/// unread for a NUL byte is named in a warning where it was `named` outright,
/// as Get-TrexLine names one, and passed over where a walk found it.
fn read_or_warned<T>(ps: &Pipeline<'_>, read: Result<Option<T>, PsError>, named: bool, shown: &str) -> PsResult<Option<T>> {
    match read {
        Ok(Some(found)) => Ok(Some(found)),
        Ok(None) => {
            if named {
                ps.warning(&crate::common::binary_notice(shown))?;
            }
            Ok(None)
        }
        Err(e) => {
            ps.write_error(&e)?;
            Ok(None)
        }
    }
}

impl Part {
    /// Refuse -Follow where it cannot follow: a part that ends before the
    /// file does, or files written back or described rather than written
    /// out. `what` is what the text written is, as "rewritten".
    fn check_follow(&self, out: FileOut, what: &str) -> PsResult<()> {
        if self.select.is_some_and(|w| !w.runs_to_end()) {
            return Err(arg_err(
                "TrexFollow",
                format!(
                    "-Follow writes the {what} text a file gains past the part read, and -Head or a -Lines range with a last line ends before the file does; follow a -Tail, a -Lines range with no last line, or the whole file"
                ),
            ));
        }
        if !matches!(out, FileOut::Text) {
            return Err(arg_err(
                "TrexFollow",
                format!("-Follow writes the {what} text of a file as it grows, and takes no -InPlace, -Diff or -Interactive"),
            ));
        }
        Ok(())
    }

    /// The edits `edits_of` makes to this part of `text`, a whole input, as
    /// edits of the whole: it reads the part alone, with the UTF-16 code
    /// units ahead of it where `placed` counts them.
    fn edits_within(&self, text: &str, placed: bool, edits_of: &mut EditsOf<'_>) -> PsResult<Vec<trex::files::Edit>> {
        let Some(select) = self.select else {
            return edits_of(text, Some(0));
        };
        let range = trex::window::range_of(text.as_bytes(), select, &self.unit);
        let part = text.get(range.clone()).ok_or_else(|| {
            PsError::new(ErrorCategory::InvalidData, "TrexWindow", "the part named does not begin and end between characters")
        })?;
        let ahead = placed.then(|| trex::encoding::utf16_units(&text.as_bytes()[..range.start]));
        let mut edits = edits_of(part, ahead)?;
        for e in &mut edits {
            e.start += range.start;
            e.end += range.start;
        }
        Ok(edits)
    }
}

/// How a cmdlet reads its files: the paths, whether read as written, the
/// walk's options, whether a file holding a NUL byte is read, the part of
/// each file edited, and whether the edits place what they see, which
/// counts the UTF-16 code units ahead of a tail for them.
struct Reading {
    paths: Vec<String>,
    literal: bool,
    opts: trex::files::WalkOptions,
    binary: bool,
    part: Part,
    placed: bool,
}

impl Reading {
    fn of(
        path: &[String],
        literal_path: &[String],
        hidden: bool,
        no_ignore: bool,
        binary: bool,
        part: &Part,
        placed: bool,
    ) -> Self {
        let (paths, literal) =
            if literal_path.is_empty() { (path.to_vec(), false) } else { (literal_path.to_vec(), true) };
        Reading {
            paths,
            literal,
            opts: trex::files::WalkOptions { hidden, no_ignore, ..trex::files::WalkOptions::default() },
            binary,
            part: part.clone(),
            placed,
        }
    }
}

/// One file's changes waiting for a review: its bytes, which the accepted
/// ones are written into, and the edits of its text.
pub(crate) struct Pending {
    pub(crate) path: PathBuf,
    pub(crate) raw: Vec<u8>,
    pub(crate) text: String,
    pub(crate) edits: Vec<trex::files::Edit>,
}

/// Writes the edits a review accepted back into each file, asking first
/// under -Confirm and saying what it would do under -WhatIf: `action` so
/// many `what`, as "Rewrite 2 match(es)".
pub(crate) fn write_reviewed(
    ps: &Pipeline<'_>,
    queue: &[Pending],
    kept: Vec<Vec<trex::files::Edit>>,
    action: &str,
    what: &str,
) -> PsResult<()> {
    for (p, kept) in queue.iter().zip(kept) {
        if kept.is_empty() {
            continue;
        }
        let shown = p.path.display().to_string();
        if ps.should_process(&shown, &format!("{action} {} {what}", kept.len()))? {
            write_back(&p.path, &trex::files::written(&p.raw, &kept, trex::files::ReadAs::Lossy))?;
        }
    }
    Ok(())
}

/// The answers the review offers, each a label with its hot key marked and
/// what it does.
const ANSWERS: [(&str, &str); 7] = [
    ("&Yes", "Apply this change"),
    ("&No", "Skip this change"),
    ("&Edit", "Type another replacement for this change"),
    ("&Template", "Apply this change and every later one of the same template"),
    ("&Skip template", "Skip this change and every later one of the same template"),
    ("Yes to &All", "Apply this change and every one after it"),
    ("&Quit", "Stop, leaving every file as it was"),
];

/// Puts each change to the person at the host's prompt, file by file, and
/// gives back the edits accepted for each file, or none where they quit.
///
/// Each change is shown as its unified diff, `explain`'s lines under it where
/// given, and its template, the kinds of the tokens it replaces, and how many
/// later changes share that template, which is what an answer for the
/// template carries. A change whose template was answered for is decided by
/// that answer without being shown. The count of changes a template answer
/// skipped is always reported, and with `show_skipped` their positions too,
/// in UTF-16 code units.
pub(crate) fn review(
    ps: &Pipeline<'_>,
    queue: &[Pending],
    context: usize,
    show_skipped: bool,
    explain: Option<&ExplainChange<'_>>,
) -> PsResult<Option<Vec<Vec<trex::files::Edit>>>> {
    use trex::review::Answer;
    let ui = ps.host_ui()?;
    let names: Vec<String> = queue.iter().map(|p| p.path.display().to_string()).collect();
    let queued: Vec<trex::review::Queued<'_>> = queue
        .iter()
        .zip(&names)
        .map(|(p, name)| trex::review::Queued { name, text: p.text.as_bytes(), edits: &p.edits })
        .collect();
    let reviewed = trex::review::review(&queued, |c| -> PsResult<Answer> {
        ui.write_line(trex::files::unified_diff(c.name, c.text, std::slice::from_ref(c.edit), context).trim_end())?;
        if let Some(explain) = explain {
            for line in explain(c.text, c.edit.start, c.edit.end) {
                ui.write_line(&line)?;
            }
        }
        ui.write_line(&format!(
            "  template: {} ({} later change{} share{} it)",
            trex::templates::silhouette_name(c.template),
            c.later,
            if c.later == 1 { "" } else { "s" },
            if c.later == 1 { "s" } else { "" }
        ))?;
        Ok(match ui.prompt_for_choice(&format!("[{}/{}] {}", c.index, c.total, c.name), "Apply this change?", &ANSWERS, 1)? {
            0 => Answer::Accept,
            1 => Answer::Skip,
            2 => {
                ui.write_line("The replacement, on one line:")?;
                Answer::Replace(ui.read_line()?.into_bytes())
            }
            3 => Answer::AcceptTemplate,
            4 => Answer::SkipTemplate,
            5 => Answer::AcceptAll,
            _ => Answer::Quit,
        })
    })?;
    let Some(reviewed) = reviewed else {
        ps.warning("quit; every file is left as it was")?;
        return Ok(None);
    };
    if !reviewed.skipped.is_empty() {
        let n = reviewed.skipped.len();
        ps.warning(&format!("{n} change{} skipped by a template answer", if n == 1 { "" } else { "s" }))?;
        if show_skipped {
            // The skipped changes come input by input in position order, so
            // one forward count of UTF-16 units per input places them all.
            let mut units = units_of(&[]);
            let mut counting = None;
            for s in &reviewed.skipped {
                if counting != Some(s.input) {
                    counting = Some(s.input);
                    units = units_of(queue[s.input].text.as_bytes());
                }
                ps.warning(&format!("  {} [{}..{}]", names[s.input], units.at(s.start), units.at(s.end)))?;
            }
        }
    }
    Ok(Some(reviewed.kept))
}

/// Each file's edited text, its diff, or the file itself written back, from
/// the edits `edits_of` makes: written out, a file's part is read alone and
/// written edited; written back or described by its diff, the whole file is
/// read and the edits are confined to its part. Under a review, every
/// file's changes are read before the first question, so the review knows
/// how many it holds, and the accepted ones are written once it has seen
/// the last.
fn over_files(
    ps: &Pipeline<'_>,
    reading: &Reading,
    out: FileOut,
    context: usize,
    action: &str,
    edits_of: &mut EditsOf<'_>,
    explain: &ExplainChange<'_>,
) -> PsResult<()> {
    let mut queue: Vec<Pending> = Vec::new();
    let given: Vec<PathBuf> =
        resolved(ps, &reading.paths, reading.literal)?.into_iter().map(PathBuf::from).collect();
    for source in files_of(ps, &reading.paths, reading.literal, &reading.opts)? {
        if ps.stopping() {
            break;
        }
        let trex::files::Source::File(path) = source else {
            continue;
        };
        let shown = path.display().to_string();
        let named = given.contains(&path);
        match out {
            FileOut::Text => {
                let read = read_file(&path, reading.part.select, &reading.part.unit, reading.binary, reading.placed);
                let Some(read) = read_or_warned(ps, read, named, &shown)? else {
                    continue;
                };
                let edits = edits_of(&read.text, read.unit_base)?;
                ps.write(String::from_utf8_lossy(&splice(read.text.as_bytes(), &edits)).into_owned())?;
            }
            FileOut::Diff => {
                let Some((_, text)) = read_or_warned(ps, read_whole(&path, reading.binary), named, &shown)? else {
                    continue;
                };
                let edits = reading.part.edits_within(&text, reading.placed, edits_of)?;
                if !edits.is_empty() {
                    ps.write(trex::files::unified_diff(&shown, text.as_bytes(), &edits, context))?;
                }
            }
            FileOut::InPlace => {
                let Some((raw, text)) = read_or_warned(ps, read_whole(&path, reading.binary), named, &shown)? else {
                    continue;
                };
                let edits = reading.part.edits_within(&text, reading.placed, edits_of)?;
                if !edits.is_empty() && ps.should_process(&shown, &format!("{action} {} match(es)", edits.len()))? {
                    write_back(&path, &trex::files::written(&raw, &edits, trex::files::ReadAs::Lossy))?;
                }
            }
            FileOut::Review { .. } => {
                let Some((raw, text)) = read_or_warned(ps, read_whole(&path, reading.binary), named, &shown)? else {
                    continue;
                };
                let edits = reading.part.edits_within(&text, reading.placed, edits_of)?;
                if !edits.is_empty() {
                    queue.push(Pending { path, raw, text, edits });
                }
            }
        }
    }
    let FileOut::Review { show_skipped, explain: explaining } = out else {
        return Ok(());
    };
    if queue.is_empty() {
        return ps.verbose("no match to review");
    }
    match review(ps, &queue, context, show_skipped, explaining.then_some(explain))? {
        Some(kept) => write_reviewed(ps, &queue, kept, action, "match(es)"),
        None => Ok(()),
    }
}

/// Reads the one file -Follow edits from `reading`, writes the lines of its
/// part read that nothing arriving later can change, and hands back the
/// file, to follow on from once the pipeline's input ends; none where the
/// paths name no file. `already` says an earlier input named the file
/// followed, which makes any file here a second.
fn start_following(
    ps: &Pipeline<'_>,
    reading: &Reading,
    already: bool,
    c: &Compiled,
    limit: Option<usize>,
    edits: &mut EditsAt<'_>,
) -> PsResult<Option<FollowedEdit>> {
    let files: Vec<PathBuf> = files_of(ps, &reading.paths, reading.literal, &reading.opts)?
        .into_iter()
        .filter_map(|source| match source {
            trex::files::Source::File(path) => Some(path),
            trex::files::Source::Stdin => None,
        })
        .collect();
    let path = match files.as_slice() {
        [] => return Ok(None),
        [path] if !already => path.clone(),
        more => {
            return Err(arg_err(
                "TrexFollow",
                format!(
                    "-Follow writes one file's text as it grows, and {} files were given",
                    more.len() + usize::from(already)
                ),
            ));
        }
    };
    let shown = path.display().to_string();
    let Some(read) = read_file(&path, reading.part.select, &reading.part.unit, reading.binary, reading.placed)? else {
        return Err(PsError::new(
            ErrorCategory::InvalidData,
            "TrexBinary",
            crate::common::binary_notice(&shown),
        ));
    };
    let mut followed = FollowedEdit::new(path, shown, &read, c);
    if !followed.commits_early() {
        return Err(PsError::new(
            ErrorCategory::InvalidOperation,
            "TrexFollow",
            "no match of this pattern is final before its input ends, since it reads a content guard, a field anchor or a whole-input axis, and a followed file does not end; run it without -Follow",
        ));
    }
    for line in followed.push_text(&read.text, limit, edits)? {
        ps.write(line)?;
    }
    Ok(Some(followed))
}

/// Follows `f`, the file -Follow edits, writing each line of its edited
/// text once nothing that arrives later can change it, until the pipeline
/// is stopped; a file truncated, replaced or removed is said so as a warning
/// and read again from its start, its -MaxCount count begun again unless
/// `keep_count` carries it on.
fn follow_on(
    ps: &Pipeline<'_>,
    f: &mut FollowedEdit,
    c: &Compiled,
    limit: Option<usize>,
    keep_count: bool,
    edits: &mut EditsAt<'_>,
) -> PsResult<()> {
    let failed = |e: std::io::Error| PsError::new(ErrorCategory::ReadError, "TrexFollow", format!("cannot follow {e}"));
    let mut follower = trex::follow::Follower::new(&[(f.path.clone(), f.offset())]).map_err(failed)?;
    if let Some(why) = follower.unnotified() {
        ps.warning(&format!("no change notifications ({why}); looking at the file once a second"))?;
    }
    while !ps.stopping() {
        let Some((_, change)) = follower.poll().map_err(failed)? else {
            continue;
        };
        let lines = match change {
            trex::follow::Followed::Appended { bytes, .. } => f.push_bytes(&bytes, limit, edits)?,
            trex::follow::Followed::Truncated => {
                ps.warning(&format!("{}: truncated; reading it from its start", f.shown))?;
                f.restart(c, limit, keep_count, edits)?
            }
            trex::follow::Followed::Replaced => {
                ps.warning(&format!("{}: replaced by another file; reading that from its start", f.shown))?;
                f.restart(c, limit, keep_count, edits)?
            }
            trex::follow::Followed::Gone => {
                ps.warning(&format!("{}: removed; waiting for a file under its name", f.shown))?;
                f.restart(c, limit, keep_count, edits)?
            }
        };
        for line in lines {
            ps.write(line)?;
        }
    }
    Ok(())
}

/// Replaces each match of a trex pattern with a rendered template or with
/// what a script block returns for it.
///
/// With -Template, `${name}` renders a register, `${0}` the whole match,
/// `${name:upper}` a transform and `${ip:octet1-2}` a typed slice. With
/// -ScriptBlock, the block runs once per match with the Trex.Match as `$_`,
/// and its last output replaces the match. Text piped in comes back
/// rewritten; -Path writes each file's rewritten text, -InPlace writes the
/// files that have a match back where they are, and -Diff writes the unified
/// diff each would take. -MaxCount replaces only the first matches of each
/// input. -Backend says which engine finds them, as Select-TrexMatch's does.
/// A file named outright that holds a NUL byte is left as it is and named in
/// a warning, unless -Binary asks for it; a walk passes over one.
///
/// -Head, -Tail and -Lines rewrite the first lines of each input, its last,
/// or a range of them, counted in records of -Unit where it names one: the
/// text written is that part alone, rewritten, and a file written back or
/// described by its diff changes only there. -First and -Last are -Head and
/// -Tail. -Follow writes a file's rewritten text as it grows, after its
/// tail, its open range or the whole of it, a line at a time, until the
/// pipeline is stopped; a file truncated, replaced or removed starts its
/// -MaxCount count again unless -KeepCount carries it on.
///
/// -Interactive puts each change to the person first, as its diff and the
/// template of the tokens it replaces, with a scan's explanation of its match
/// under -Explain: they apply it, skip it, type another replacement, answer
/// for every later change of the same template, apply everything after it,
/// or quit, which leaves every file as it was. The changes accepted are
/// written once the last has been answered, and how many a template answer
/// skipped is said as a warning, with where under -ShowSkipped.
///
/// # Examples
/// 'mail bob@x.com now' | Edit-TrexText '\E:e' '[${e:domain}]'
/// 'a 1 b 22' | Edit-TrexText '\N' -ScriptBlock { [int]$_.Text * 2 }
/// Edit-TrexText '\E:e' '<redacted>' -Path ./logs -InPlace -WhatIf
/// Edit-TrexText '"http:"' 'https:' -Path ./docs -Interactive
/// Edit-TrexText '\E:e' '<redacted>' -Path app.log -Tail 20 -Follow
#[cmdlet(verb = "Edit", noun = "TrexText", alias = "Edit-TxText", supports_should_process, confirm_impact = "Medium", default_parameter_set = "Template", output = ["System.String"])]
#[derive(Default)]
pub struct EditTrexText {
    /// The pattern: trex source text, or a Trex.Pattern.
    #[param(mandatory, position = 0)]
    pub pattern: PsObject,
    /// The template rendered in place of each match.
    #[param(mandatory, position = 1, set = ["Template", "TemplatePath", "TemplateLiteralPath"])]
    pub template: String,
    /// A script block run once per match, with the match as `$_`; its last
    /// output replaces the match.
    #[param(mandatory, set = ["Block", "BlockPath", "BlockLiteralPath"])]
    pub script_block: PsScriptBlock,
    /// The text to rewrite; each string piped in is rewritten on its own.
    #[param(mandatory, value_from_pipeline, set = ["Template", "Block"], allow_empty_string)]
    pub input_object: String,
    /// Files or directories to rewrite; wildcards expand.
    #[param(mandatory, set = ["TemplatePath", "BlockPath"])]
    pub path: Vec<String>,
    /// Files or directories to rewrite, read as written, as Get-ChildItem
    /// pipes them.
    #[param(mandatory, set = ["TemplateLiteralPath", "BlockLiteralPath"], literal_path)]
    pub literal_path: Vec<String>,
    /// Writes each file that has a match back where it is.
    #[param(set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub in_place: bool,
    /// Writes the unified diff each file would take instead of its text.
    #[param(set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub diff: bool,
    /// Puts each change to the person before it is written.
    #[param(set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub interactive: bool,
    /// Lists the position of each change a template answer skipped, beside
    /// the count -Interactive always reports.
    #[param(set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub show_skipped: bool,
    /// Writes under each change -Interactive asks about what a scan's
    /// explanation says of its match.
    #[param(set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub explain: bool,
    /// Lines of context around each change in a diff; 3 when absent.
    #[param(set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub context: Option<u32>,
    /// Reads hidden files and directories a walk would skip.
    #[param(set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub hidden: bool,
    /// Reads files an ignore rule excludes.
    #[param(set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub no_ignore: bool,
    /// Reads files that hold a NUL byte, which a walk treats as binary.
    #[param(set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub binary: bool,
    /// Replaces only the first this many matches of each input.
    #[param]
    pub max_count: Option<u32>,
    /// The engine that finds the matches: Auto when absent, Cpu, or Gpu,
    /// which falls back to the CPU with a note where the device cannot take
    /// the scan.
    #[param]
    pub backend: Option<Backend>,
    /// Keeps a followed file's -MaxCount count when it is truncated, replaced
    /// or removed, in place of starting it again for the file then under its
    /// name.
    #[param(set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub keep_count: bool,
    /// Rewrites the first this many lines of each input, or records of
    /// -Unit, reading a file no further.
    #[param(alias = ["First"])]
    pub head: Option<u32>,
    /// Rewrites the last this many lines of each input, or records of -Unit,
    /// reading a file backward from its end.
    #[param(alias = ["Last"])]
    pub tail: Option<u32>,
    /// Rewrites a range of lines, or records of -Unit, counted from one:
    /// `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`.
    #[param]
    pub lines: Option<PsObject>,
    /// What -Head, -Tail and -Lines count: a line when absent, or a unit
    /// Find-TrexRecord reads, such as `paragraph` or `block`.
    #[param]
    pub unit: Option<String>,
    /// Writes a file's rewritten text as it grows, a line at a time, after
    /// its tail, its open range or the whole of it, until the pipeline is
    /// stopped.
    #[param(alias = ["Wait"], set = ["TemplatePath", "BlockPath", "TemplateLiteralPath", "BlockLiteralPath"])]
    pub follow: bool,
    /// The atoms a source-text pattern is compiled against, in place of the
    /// session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    compiled: Option<Compiled>,
    template_read: Option<trex::Template>,
    engine: Engine,
    part: Part,
    followed: Option<FollowedEdit>,
}

impl EditTrexText {
    /// The edits a followed rewrite makes at the matches its stream
    /// commits: the template rendered at each, or the script block run for
    /// each.
    fn stream_edits<'a>(&'a self, c: &'a Compiled) -> Box<EditsAt<'a>> {
        let block = (!self.script_block.0.is_null()).then_some(&self.script_block);
        let template = self.template_read.as_ref();
        Box::new(move |held: &[u8], spans: &[trex::Span], ahead: Option<usize>| match (block, template) {
            (Some(block), _) => block_edits(block, c.matches_at(held_text(held)?, spans)?, ahead),
            (None, Some(t)) => Ok(trex::rewrite::edits_at(&c.inner, t, held, &c.shapes, spans)),
            (None, None) => Err(arg_err("TrexTemplateError", "the template was not read")),
        })
    }
}

impl Cmdlet for EditTrexText {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let c = pattern_arg(ps, &self.pattern, &self.library)?;
        let out = file_out(self.in_place, self.diff, self.interactive, self.show_skipped, self.explain)?;
        self.part = Part::of(self.head, self.tail, &self.lines, &self.unit)?;
        let backend = self.backend.unwrap_or(Backend::Auto);
        if self.follow {
            self.part.check_follow(out, "rewritten")?;
            if backend == Backend::Gpu {
                return Err(arg_err(
                    "TrexFollow",
                    "-Follow rewrites what a file gains with the stream scanner on the CPU, and takes no -Backend Gpu",
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
        self.engine = Engine { backend: backend.trex(), ..Engine::PLAIN };
        if self.script_block.0.is_null() {
            self.template_read = Some(parse_template(&c, &self.template)?);
        }
        self.compiled = Some(c);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(c) = &self.compiled else {
            return Err(arg_err("TrexNoPattern", "the pattern was not compiled"));
        };
        let limit = self.max_count.map(|n| n as usize);
        let block = (!self.script_block.0.is_null()).then_some(&self.script_block);
        let template = self.template_read.as_ref();
        let engine = self.engine;
        let mut edits_of = |text: &str, ahead: Option<usize>| -> PsResult<Vec<trex::files::Edit>> {
            let (edits, note) = match (block, template) {
                (Some(block), _) => {
                    let (found, note) = c.matches_by(text, limit, engine)?;
                    (block_edits(block, found, ahead)?, note)
                }
                (None, Some(t)) => c.template_edits_by(text, t, limit, engine),
                (None, None) => return Err(arg_err("TrexTemplateError", "the template was not read")),
            };
            if let Some(note) = note {
                ps.verbose(&note)?;
            }
            Ok(edits)
        };
        if self.path.is_empty() && self.literal_path.is_empty() {
            let read = self.part.of_text(&self.input_object);
            let edits = edits_of(&read.text, read.unit_base)?;
            return ps.write(String::from_utf8_lossy(&splice(read.text.as_bytes(), &edits)).into_owned());
        }
        let out = file_out(self.in_place, self.diff, self.interactive, self.show_skipped, self.explain)?;
        let context = self.context.map_or(3, |n| n as usize);
        // A script block reads each match's Start, which counts the text
        // ahead of a tail.
        let reading =
            Reading::of(&self.path, &self.literal_path, self.hidden, self.no_ignore, self.binary, &self.part, block.is_some());
        if self.follow {
            let mut edits = self.stream_edits(c);
            let started = start_following(ps, &reading, self.followed.is_some(), c, limit, &mut *edits)?;
            drop(edits);
            if started.is_some() {
                self.followed = started;
            }
            return Ok(());
        }
        let explain = |text: &[u8], start: usize, end: usize| {
            explanation_lines(c, text, start, end, "the rewrite's own scan")
        };
        over_files(ps, &reading, out, context, "Rewrite", &mut edits_of, &explain)
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        let Some(mut followed) = self.followed.take() else {
            return Ok(());
        };
        let Some(c) = &self.compiled else {
            return Err(arg_err("TrexNoPattern", "the pattern was not compiled"));
        };
        let limit = self.max_count.map(|n| n as usize);
        let mut edits = self.stream_edits(c);
        follow_on(ps, &mut followed, c, limit, self.keep_count, &mut *edits)
    }
}

/// Masks each match of a trex pattern, keeping the fields named.
///
/// Every character of a match becomes the mask, `*` when -Mask is not given,
/// except the fields -Keep names, which stay unmasked: `card:last4`
/// keeps a card's last four digits, `ip:octet1-2` an address's first two
/// octets, `e:domain` a mail domain. -Mask takes one character, a token that
/// stands in for each masked run, `shape`, which masks letters and digits and
/// keeps every other character so the text still lexes as it did, or
/// `pseudonym`, which gives each distinct value a stable name per kind. Text
/// piped in comes back masked; -Path writes each file's masked text,
/// -InPlace writes the files back, and -Diff writes the unified diff. A file
/// named outright that holds a NUL byte is left as it is and named in a
/// warning, unless -Binary asks for it; a walk passes over one.
///
/// -Interactive puts each masking to the person first, as Edit-TrexText's
/// -Interactive puts a change, with a scan's explanation of its match under
/// -Explain and where a template answer skipped one under -ShowSkipped.
///
/// -Head, -Tail and -Lines mask the first lines of each input, its last, or
/// a range of them, counted in records of -Unit where it names one: the text
/// written is that part alone, masked, so nothing outside it is written
/// unmasked, and a file written back or described by its diff changes only
/// there. -First and -Last are -Head and -Tail. -Follow writes a file's
/// masked text as it grows, after its tail, its open range or the whole of
/// it, a line at a time, until the pipeline is stopped.
///
/// # Examples
/// 'mail bob@x.com now' | Protect-TrexText '\E:e' -Keep e:domain
/// 'card 4111 1111 1111 1111' | Protect-TrexText '\{creditcard}:c' -Keep c:last4
/// Protect-TrexText '\I' -Mask pseudonym -Path ./access.log -InPlace
/// Protect-TrexText '\I' -Mask pseudonym -Path ./access.log -Tail 50 -Follow
#[cmdlet(verb = "Protect", noun = "TrexText", alias = "Protect-TxText", supports_should_process, confirm_impact = "Medium", default_parameter_set = "Text", output = ["System.String"])]
#[derive(Default)]
pub struct ProtectTrexText {
    /// The pattern: trex source text, or a Trex.Pattern.
    #[param(mandatory, position = 0)]
    pub pattern: PsObject,
    /// The text to mask; each string piped in is masked on its own.
    #[param(mandatory, value_from_pipeline, set = "Text", allow_empty_string)]
    pub input_object: String,
    /// Files or directories to mask; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to mask, read as written, as Get-ChildItem pipes
    /// them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The fields to leave in place, as a register and an accessor:
    /// `card:last4`, `ip:octet1-2`, `e:domain`.
    #[param]
    pub keep: Vec<String>,
    /// What stands in for a masked character: one character, a token for
    /// each run, `shape`, or `pseudonym`; `*` when absent.
    #[param]
    pub mask: Option<String>,
    /// Writes each file that has a match back where it is.
    #[param(set = ["Path", "LiteralPath"])]
    pub in_place: bool,
    /// Writes the unified diff each file would take instead of its text.
    #[param(set = ["Path", "LiteralPath"])]
    pub diff: bool,
    /// Puts each masking to the person before it is written.
    #[param(set = ["Path", "LiteralPath"])]
    pub interactive: bool,
    /// Lists the position of each masking a template answer skipped, beside
    /// the count -Interactive always reports.
    #[param(set = ["Path", "LiteralPath"])]
    pub show_skipped: bool,
    /// Writes under each masking -Interactive asks about what a scan's
    /// explanation says of its match.
    #[param(set = ["Path", "LiteralPath"])]
    pub explain: bool,
    /// Lines of context around each change in a diff; 3 when absent.
    #[param(set = ["Path", "LiteralPath"])]
    pub context: Option<u32>,
    /// Reads hidden files and directories a walk would skip.
    #[param(set = ["Path", "LiteralPath"])]
    pub hidden: bool,
    /// Reads files an ignore rule excludes.
    #[param(set = ["Path", "LiteralPath"])]
    pub no_ignore: bool,
    /// Reads files that hold a NUL byte, which a walk treats as binary.
    #[param(set = ["Path", "LiteralPath"])]
    pub binary: bool,
    /// Masks the first this many lines of each input, or records of -Unit,
    /// reading a file no further.
    #[param(alias = ["First"])]
    pub head: Option<u32>,
    /// Masks the last this many lines of each input, or records of -Unit,
    /// reading a file backward from its end.
    #[param(alias = ["Last"])]
    pub tail: Option<u32>,
    /// Masks a range of lines, or records of -Unit, counted from one:
    /// `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`.
    #[param]
    pub lines: Option<PsObject>,
    /// What -Head, -Tail and -Lines count: a line when absent, or a unit
    /// Find-TrexRecord reads, such as `paragraph` or `block`.
    #[param]
    pub unit: Option<String>,
    /// Writes a file's masked text as it grows, a line at a time, after its
    /// tail, its open range or the whole of it, until the pipeline is
    /// stopped.
    #[param(alias = ["Wait"], set = ["Path", "LiteralPath"])]
    pub follow: bool,
    /// The atoms a source-text pattern is compiled against, in place of the
    /// session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    compiled: Option<Compiled>,
    keeps: Vec<trex::Keep>,
    masker: Option<trex::Mask>,
    part: Part,
    followed: Option<FollowedEdit>,
}

/// The edits that mask the matches of `text`, a pseudonym naming a declared
/// kind by its declaration.
fn mask_edits(c: &Compiled, keeps: &[trex::Keep], mask: &mut trex::Mask, text: &str) -> Vec<trex::files::Edit> {
    let bytes = text.as_bytes();
    mask_edits_at(c, keeps, mask, bytes, &c.spans(bytes))
}

/// The edits that mask the matches at `spans` of `bytes`.
fn mask_edits_at(
    c: &Compiled,
    keeps: &[trex::Keep],
    mask: &mut trex::Mask,
    bytes: &[u8],
    spans: &[trex::Span],
) -> Vec<trex::files::Edit> {
    let found = c.resolved(bytes, spans);
    trex::redactions_with_shapes(bytes, &found, keeps, mask, &c.inner, &c.shapes)
}

impl Cmdlet for ProtectTrexText {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let c = pattern_arg(ps, &self.pattern, &self.library)?;
        let joined = self.keep.join(",");
        self.keeps = trex::Keep::parse_list(&joined, &c.inner.capture_names())
            .map_err(|e| arg_err("TrexKeepError", format!("-Keep error at byte {}: {}", e.pos, e.msg)))?;
        let mask = match &self.mask {
            Some(m) => m.clone(),
            None => "*".to_string(),
        };
        self.masker = Some(trex::Mask::parse(&mask).map_err(|e| arg_err("TrexMaskError", e))?);
        let out = file_out(self.in_place, self.diff, self.interactive, self.show_skipped, self.explain)?;
        self.part = Part::of(self.head, self.tail, &self.lines, &self.unit)?;
        if self.follow {
            self.part.check_follow(out, "masked")?;
        }
        self.compiled = Some(c);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (Some(c), Some(mask)) = (&self.compiled, &mut self.masker) else {
            return Err(arg_err("TrexNoPattern", "the pattern was not compiled"));
        };
        let keeps = &self.keeps;
        if self.path.is_empty() && self.literal_path.is_empty() {
            let read = self.part.of_text(&self.input_object);
            let edits = mask_edits(c, keeps, mask, &read.text);
            return ps.write(String::from_utf8_lossy(&splice(read.text.as_bytes(), &edits)).into_owned());
        }
        let out = file_out(self.in_place, self.diff, self.interactive, self.show_skipped, self.explain)?;
        let context = self.context.map_or(3, |n| n as usize);
        let reading =
            Reading::of(&self.path, &self.literal_path, self.hidden, self.no_ignore, self.binary, &self.part, false);
        if self.follow {
            let mut edits = |held: &[u8], spans: &[trex::Span], _: Option<usize>| -> PsResult<Vec<trex::files::Edit>> {
                Ok(mask_edits_at(c, keeps, mask, held, spans))
            };
            let started = start_following(ps, &reading, self.followed.is_some(), c, None, &mut edits)?;
            if started.is_some() {
                self.followed = started;
            }
            return Ok(());
        }
        let mut edits_of = |text: &str, _: Option<usize>| -> PsResult<Vec<trex::files::Edit>> {
            Ok(mask_edits(c, keeps, mask, text))
        };
        let explain = |text: &[u8], start: usize, end: usize| {
            explanation_lines(c, text, start, end, "the redaction's own scan")
        };
        over_files(ps, &reading, out, context, "Mask", &mut edits_of, &explain)
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        let Some(mut followed) = self.followed.take() else {
            return Ok(());
        };
        let (Some(c), Some(mask)) = (&self.compiled, &mut self.masker) else {
            return Err(arg_err("TrexNoPattern", "the pattern was not compiled"));
        };
        let keeps = &self.keeps;
        let mut edits = |held: &[u8], spans: &[trex::Span], _: Option<usize>| -> PsResult<Vec<trex::files::Edit>> {
            Ok(mask_edits_at(c, keeps, mask, held, spans))
        };
        follow_on(ps, &mut followed, c, None, false, &mut edits)
    }
}
