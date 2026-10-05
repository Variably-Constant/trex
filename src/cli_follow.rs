//! Inputs read as they arrive: the standard input scanned as it streams in,
//! and files followed as they grow (`--follow`), each match printed, and
//! each rewrite or redaction written, once nothing that arrives later can
//! change it.
//!
//! One stream scanner reads each input from where its report begins, so a
//! match reaching across what arrived in two reads is found as a scan of
//! the whole input finds it, and a followed file's own offsets and line
//! numbers are reported: its window's, then those of each byte it gains. A
//! file truncated, replaced under its name or removed is said so on the
//! standard error, and read again from its start.

use std::io::Write;
use std::process::ExitCode;

use trex::encoding::Incremental;
use trex::files::{Edit, LineIndex, Source};
use trex::follow::{Followed as Change, Follower};
use trex::records::RecordUnit;
use trex::report::Members;
use trex::window::{Asked, Select};

use crate::cli_files::{ScanHow, Scanning, scan_found, whole_line_matches};
use crate::cli_window::{Part, read_part};

/// How a stream's committed matches print: the report a scan asked for, as
/// the same printers write it for a whole input.
pub(crate) struct StreamPrint<'a> {
    pub(crate) json: bool,
    pub(crate) format: Option<&'a trex::Template>,
    pub(crate) painter: &'a trex::paint::Painter,
    /// Whether every binding under a repetition is resolved, for a report
    /// that reads them.
    pub(crate) lists: bool,
    /// What a typed register's value is read and spelled as, for the JSON
    /// report.
    pub(crate) values: Option<&'a crate::ValueView<'a>>,
    /// Whether each match is printed after its input's name, line and column.
    pub(crate) prefixed: bool,
    /// `-x`: only the matches covering their line.
    pub(crate) whole_line: bool,
    /// `-m N`: at most this many matches an input.
    pub(crate) max_count: Option<usize>,
    /// `--keep-count`: a followed file truncated, replaced or removed keeps
    /// its `-m` count rather than starting it again.
    pub(crate) keep_count: bool,
    /// `--single-match`: each member's first match and no more.
    pub(crate) single: bool,
}

/// One input scanned as it arrives, with what has been printed of it.
pub(crate) struct StreamRun<'a> {
    scanning: &'a Scanning,
    shapes: &'a trex::ShapeSet,
    /// The input's name, printed ahead of each match where the report names
    /// its inputs, and what a template's `${path}` reads.
    name: &'a str,
    stream: trex::HeldStream,
    printed: usize,
    /// Which members have printed a match, for `--single-match`.
    seen: Vec<bool>,
}

impl<'a> StreamRun<'a> {
    /// A stream of the input named `name` from `origin`, the offset of its
    /// first byte, after `lines_before` of its lines where those were
    /// counted, which a report placing matches on their lines asks for.
    pub(crate) fn new(
        scanning: &'a Scanning,
        shapes: &'a trex::ShapeSet,
        name: &'a str,
        origin: usize,
        lines_before: Option<usize>,
    ) -> Self {
        let scanner = match scanning {
            Scanning::One(p) => trex::StreamScanner::with_shapes(p.clone(), shapes.clone()),
            Scanning::Set(set) => trex::StreamScanner::over_set((**set).clone()),
        };
        let members = scanning.set().map_or(1, trex::PatternSet::len);
        StreamRun {
            scanning,
            shapes,
            name,
            stream: trex::HeldStream::new(scanner, origin, lines_before),
            printed: 0,
            seen: vec![false; members],
        }
    }

    /// Whether a match can be printed before the input ends.
    pub(crate) fn commits_early(&self) -> bool {
        self.stream.commits_early()
    }

    /// Whether the report has printed all it will: `-m`'s count, or under
    /// `--single-match` a match of every member.
    pub(crate) fn done(&self, p: &StreamPrint<'_>) -> bool {
        p.max_count.is_some_and(|n| self.printed >= n) || (p.single && self.seen.iter().all(|&s| s))
    }

    /// Scan the input's next bytes, printing the matches they settle.
    pub(crate) fn push(&mut self, bytes: &[u8], p: &StreamPrint<'_>, out: &mut dyn FnMut(&str)) {
        let committed = self.stream.push(bytes);
        let view = Held {
            scanning: self.scanning,
            shapes: self.shapes,
            name: self.name,
            held: self.stream.held(),
            base: self.stream.base(),
            lines_before: self.stream.lines_before(),
        };
        view.print(&committed, p, &mut self.printed, &mut self.seen, out);
    }

    /// End the input: print the matches the scanner held until the end, and
    /// say how many were printed in all.
    pub(crate) fn finish(self, p: &StreamPrint<'_>, out: &mut dyn FnMut(&str)) -> usize {
        let StreamRun { scanning, shapes, name, stream, mut printed, mut seen } = self;
        let ended = stream.finish();
        let view = Held { scanning, shapes, name, held: &ended.held, base: ended.base, lines_before: ended.lines_before };
        view.print(&ended.matches, p, &mut printed, &mut seen, out);
        printed
    }
}

/// The bytes a stream holds and their offset in the input, from which a
/// batch of its committed matches is read and placed.
struct Held<'b> {
    scanning: &'b Scanning,
    shapes: &'b trex::ShapeSet,
    name: &'b str,
    held: &'b [u8],
    /// Where `held` begins in the input.
    base: usize,
    /// How many of the input's lines come before `held`, where they were
    /// counted.
    lines_before: Option<usize>,
}

impl Held<'_> {
    /// Print `committed`, spans over the held bytes with the member that made
    /// each, as the report prints a whole input's: each member's registers
    /// resolved at once, `-x`, `--single-match` and `-m` applied, and each
    /// placed at the input's own offset and line.
    fn print(
        &self,
        committed: &[(usize, trex::Span)],
        p: &StreamPrint<'_>,
        printed: &mut usize,
        seen: &mut [bool],
        out: &mut dyn FnMut(&str),
    ) {
        if committed.is_empty() {
            return;
        }
        let mut members_in: Vec<usize> = committed.iter().map(|&(member, _)| member).collect();
        members_in.sort_unstable();
        members_in.dedup();
        let mut found: Vec<(usize, usize, trex::Match)> = Vec::with_capacity(committed.len());
        for member in members_in {
            let at: Vec<usize> = (0..committed.len()).filter(|&k| committed[k].0 == member).collect();
            let spans: Vec<trex::Span> = at.iter().map(|&k| committed[k].1).collect();
            let pattern = self.scanning.pattern(member);
            let resolved = if p.lists {
                trex::captures_with_shapes_and_lists(pattern, self.held, self.shapes, &spans)
            } else {
                trex::captures_with_shapes(pattern, self.held, self.shapes, &spans)
            };
            found.extend(at.into_iter().zip(resolved).map(|(k, m)| (k, member, m)));
        }
        found.sort_by_key(|&(k, _, _)| k);
        let (mut members, mut matches): (Vec<usize>, Vec<trex::Match>) =
            found.into_iter().map(|(_, member, m)| (member, m)).unzip();
        let index = LineIndex::new(self.held).within(Some(self.base), self.lines_before);
        if p.whole_line {
            (matches, members) = whole_line_matches(self.held, matches, members, &index);
        }
        if p.single {
            let first: Vec<bool> = members
                .iter()
                .map(|&m| {
                    let new = !seen[m];
                    seen[m] = true;
                    new
                })
                .collect();
            matches = matches.into_iter().zip(&first).filter(|(_, f)| **f).map(|(m, _)| m).collect();
            members = members.into_iter().zip(&first).filter(|(_, f)| **f).map(|(m, _)| m).collect();
        }
        if let Some(n) = p.max_count {
            let room = n.saturating_sub(*printed);
            matches.truncate(room);
            members.truncate(room);
        }
        if matches.is_empty() {
            return;
        }
        let set_members = self.scanning.set().map(|set| Members { set, of: &members });
        if let Some(t) = p.format {
            for (k, m) in matches.iter().enumerate() {
                let (line, col) = index.line_col(self.held, m.start);
                let name = set_members.map(|ms| ms.name(k));
                let place = trex::ReportAt {
                    path: self.name,
                    line,
                    col,
                    offsets: index.offsets(self.held, m.start, m.end),
                    pattern: name.as_deref(),
                    rule: None,
                };
                out(&t.render_report(m, self.held, &place));
            }
        } else if p.json {
            for (k, m) in matches.iter().enumerate() {
                let at = p.prefixed.then(|| {
                    let (line, col) = index.line_col(self.held, m.start);
                    (self.name, line, col)
                });
                let extra = trex::report::json_extras(m, set_members, k, None);
                out(&crate::json_match_full(self.held, m, at, index.offset(0), extra.as_deref(), p.values));
            }
        } else if p.prefixed {
            let context = trex::report::Context { before: 0, after: 0, record: None };
            trex::report::context_report(
                Some(self.name),
                self.held,
                &matches,
                &index,
                &context,
                p.painter,
                set_members,
                None,
                out,
            );
        } else {
            trex::report::human_report(self.held, &matches, &index, p.painter, set_members, None, out);
        }
        *printed += matches.len();
    }
}

/// What a stream's report prints, beyond the scan itself.
pub(crate) struct StreamReport<'a> {
    pub(crate) json: bool,
    pub(crate) require_match: bool,
    pub(crate) binary: bool,
    pub(crate) format: Option<&'a trex::Template>,
    pub(crate) painter: &'a trex::paint::Painter,
    /// Whether every binding under a repetition is resolved, for a report
    /// that reads them.
    pub(crate) lists: bool,
    /// How a typed register's value is rendered, so a stream reports the same
    /// values a scan over files does.
    pub(crate) style: trex::typed::ValueStyle,
}

/// The standard input scanned as it arrives: each chunk the standard
/// library's buffer holds is pushed to the stream scanner, each match is
/// printed as the scanner commits it, the bytes behind the scanner's
/// retained window are dropped, and at exit a pattern that could not commit
/// early reports how much it retained. A set streams under one window, its
/// most conservative member deciding what commits, and each match prints
/// under its member. A stream opening with a UTF-16 or UTF-32 byte order
/// mark is read whole and decoded, one opening with a UTF-8 mark streams the
/// text after it, and a NUL byte makes it binary as it does a file.
pub(crate) fn stream_stdin(scanning: &Scanning, report: &StreamReport<'_>) -> ExitCode {
    use std::io::BufRead;

    let StreamReport { json, require_match, binary, format, painter, lists, style } = *report;
    // Read once for the whole stream: the kinds come from the patterns, and a
    // stream reports a match at a time for as long as input arrives.
    let capture_kinds = scanning.capture_kinds();
    let value_view = crate::ValueView { kinds: &capture_kinds, style, clock: trex::Clock::current() };
    let print = StreamPrint {
        json,
        format,
        painter,
        lists,
        values: Some(&value_view),
        prefixed: false,
        whole_line: false,
        max_count: None,
        keep_count: false,
        single: false,
    };
    let shapes = trex::ShapeSet::new();
    let mut run = StreamRun::new(scanning, &shapes, "-", 0, Some(0));
    let mut out = |l: &str| crate::out::line(l);
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut total = 0usize;
    // The opening bytes, held until there are enough of them to read a byte
    // order mark from.
    let mut opening: Vec<u8> = Vec::new();
    let mut opened = false;
    loop {
        let chunk = match reader.fill_buf() {
            Ok(c) => c.to_vec(),
            Err(e) => {
                eprintln!("trex: -: {e}");
                return ExitCode::FAILURE;
            }
        };
        if chunk.is_empty() {
            break;
        }
        reader.consume(chunk.len());
        total += chunk.len();
        let piece = if opened {
            chunk
        } else {
            opening.extend_from_slice(&chunk);
            if opening.len() < 4 {
                continue;
            }
            opened = true;
            if trex::files::has_bom(&opening) {
                return scan_stdin_whole(&mut reader, opening, scanning, report);
            }
            drop_utf8_mark(&mut opening);
            std::mem::take(&mut opening)
        };
        if !binary && piece.contains(&0) {
            eprintln!("trex: - holds a NUL byte and is binary; --binary scans it");
            return ExitCode::FAILURE;
        }
        run.push(&piece, &print, &mut out);
    }
    if !opened {
        if trex::files::has_bom(&opening) {
            return scan_stdin_whole(&mut reader, opening, scanning, report);
        }
        drop_utf8_mark(&mut opening);
        if !binary && opening.contains(&0) {
            eprintln!("trex: - holds a NUL byte and is binary; --binary scans it");
            return ExitCode::FAILURE;
        }
        run.push(&opening, &print, &mut out);
    }
    let early = run.commits_early();
    let printed = run.finish(&print, &mut out);
    if !early && total > 0 {
        eprintln!(
            "trex: {total} bytes retained until the stream ended; a match of this pattern cannot commit before the end, since it reads a content guard, a lookaround reaching past its lines, a field anchor or a whole-stream axis"
        );
    }
    if !json && format.is_none() && printed == 0 {
        crate::out::line("no match");
    }
    if require_match && printed == 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// Drops a UTF-8 byte order mark from the front of `opening`, a stream's first
/// bytes where they carry no UTF-16 or UTF-32 mark: the mark is not text, as it
/// is not in a marked file.
fn drop_utf8_mark(opening: &mut Vec<u8>) {
    opening.drain(..trex::encoding::Encoding::declared(opening).1);
}

/// The rest of a byte-order-marked stream read whole, decoded and scanned
/// as a file is, `opening` being the bytes already read.
fn scan_stdin_whole(
    reader: &mut impl std::io::Read,
    mut opening: Vec<u8>,
    scanning: &Scanning,
    report: &StreamReport<'_>,
) -> ExitCode {
    if let Err(e) = reader.read_to_end(&mut opening) {
        eprintln!("trex: -: {e}");
        return ExitCode::FAILURE;
    }
    let input = trex::encoding::decode(opening);
    let how = ScanHow {
        backend: trex::Backend::Auto,
        dual_grain: false,
        chunk_size: None,
        say: false,
        take: None,
        lists: report.lists,
        single: false,
    };
    let (matches, of_member) = scan_found(scanning, &input, &trex::ShapeSet::new(), how);
    let members = scanning.set().map(|set| Members { set, of: &of_member });
    let capture_kinds = scanning.capture_kinds();
    let value_view = crate::ValueView { kinds: &capture_kinds, style: report.style, clock: trex::Clock::current() };
    let index = LineIndex::new(&input);
    if let Some(t) = report.format {
        crate::cli_files::print_formatted("-", &input, &matches, &index, t, members, None, false);
    } else if report.json {
        crate::cli_files::print_json_about(&input, &matches, &index, members, None, Some(&value_view));
    } else {
        crate::cli_files::print_human_about(&input, &matches, &index, report.painter, members, None);
    }
    if report.require_match && matches.is_empty() { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// The files a follow reads, and how each is read before it is followed.
pub(crate) struct Followed<'a> {
    pub(crate) sources: &'a [Source],
    /// The window each file is read from before it is followed; the whole
    /// file where none is named.
    pub(crate) select: Option<Select>,
    /// What the window counts.
    pub(crate) unit: &'a RecordUnit,
    /// `--binary`: a file holding a NUL byte is scanned all the same.
    pub(crate) binary: bool,
    /// Whether the walk that found the files already failed.
    pub(crate) failed: bool,
}

/// The note a follow writes on the standard error when a followed file
/// changes under it other than by growing, `doing` saying what the follow
/// does about it.
fn say_change(name: &str, change: &Change, doing: &str) {
    match change {
        Change::Appended { .. } => {}
        Change::Truncated => eprintln!("trex: {name}: truncated; {doing} from its start"),
        Change::Replaced => eprintln!("trex: {name}: replaced by another file; {doing} from its start"),
        Change::Gone => eprintln!("trex: {name}: removed; waiting for a file under its name"),
    }
}

/// Say that no match of the pattern can be final before its input ends,
/// which a followed file never does.
fn say_uncommittable(command: &str) {
    eprintln!(
        "trex {command}: no match of this pattern is final before its input ends, since it reads a content guard, a field anchor or a whole-input axis, and a followed file does not end; run it without --follow"
    );
}

/// Scan each file from its window on, then follow it as it grows, printing
/// each match once nothing that arrives later can change it. Runs until
/// `-m` or `--single-match` has printed all it will from every file, and
/// otherwise until it is interrupted.
pub(crate) fn follow_scan(
    f: &Followed<'_>,
    scanning: &Scanning,
    shapes: &trex::ShapeSet,
    p: &StreamPrint<'_>,
) -> ExitCode {
    if !StreamRun::new(scanning, shapes, "", 0, None).commits_early() {
        say_uncommittable("scan");
        return ExitCode::FAILURE;
    }
    // A report placing a match on its line needs the lines ahead of a tail,
    // and the follow continues the tail's offsets, so it is placed.
    let numbers = p.prefixed || p.format.is_some_and(trex::Template::reads_place);
    let asked = Asked { numbers, offsets: true, binary: f.binary, units: false };
    let names: Vec<String> = f.sources.iter().map(Source::name).collect();
    let mut failed = f.failed;
    let mut out = |l: &str| crate::out::line(l);
    let mut runs: Vec<StreamRun<'_>> = Vec::new();
    let mut decoders: Vec<Incremental> = Vec::new();
    let mut owners: Vec<usize> = Vec::new();
    let mut followees: Vec<(std::path::PathBuf, usize)> = Vec::new();
    for (k, src) in f.sources.iter().enumerate() {
        let Source::File(path) = src else {
            continue;
        };
        let part = match read_part(src, f.select, f.unit, asked) {
            Ok(part) => part,
            Err(e) => {
                eprintln!("trex: cannot read {}: {e}", names[k]);
                failed = true;
                continue;
            }
        };
        if part.binary && !f.binary {
            eprintln!("trex: {} holds a NUL byte and is binary; --binary scans it", names[k]);
            failed = true;
            continue;
        }
        let mut run = StreamRun::new(scanning, shapes, &names[k], part.base(), part.line_base);
        run.push(&part.text, p, &mut out);
        runs.push(run);
        decoders.push(Incremental::after(part.encoding));
        owners.push(k);
        followees.push((path.clone(), part.input_len));
    }
    let exit = |failed: bool| if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    if runs.is_empty() || runs.iter().all(|r| r.done(p)) {
        return exit(failed);
    }
    let mut follower = match Follower::new(&followees) {
        Ok(follower) => follower,
        Err(e) => {
            eprintln!("trex: cannot follow: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(why) = follower.unnotified() {
        eprintln!("trex: no change notifications ({why}); looking at the files once a second");
    }
    loop {
        let (i, change) = match follower.wait() {
            Ok(next) => next,
            Err(e) => {
                eprintln!("trex: cannot follow {e}");
                return ExitCode::FAILURE;
            }
        };
        let name = &names[owners[i]];
        match change {
            Change::Appended { bytes, .. } => {
                let text = decoders[i].decode(&bytes);
                runs[i].push(&text, p, &mut out);
                if runs.iter().all(|r| r.done(p)) {
                    return exit(failed);
                }
            }
            other => {
                say_change(name, &other, "scanning it");
                let fresh = StreamRun::new(scanning, shapes, name, 0, Some(0));
                let printed = std::mem::replace(&mut runs[i], fresh).finish(p, &mut out);
                decoders[i] = Incremental::from_start();
                // The file now under the name is a new input, whose `-m` count
                // starts again unless `--keep-count` carries the old one's on.
                if p.keep_count {
                    runs[i].printed = printed;
                    if runs.iter().all(|r| r.done(p)) {
                        return exit(failed);
                    }
                }
            }
        }
    }
}

/// Scan each file's rules from its window on, then follow it as it grows,
/// printing each finding once nothing that arrives later can change its
/// match; a JSON report writes one object a line. Runs until `-m` has
/// printed all it will from every file, and otherwise until it is
/// interrupted.
pub(crate) fn follow_rules(
    sources: &[Source],
    run: &crate::cli_rules::RulesRun<'_>,
    report: &crate::cli_rules::Report<'_>,
    asked: Asked,
    prefixed: bool,
    failed: bool,
) -> ExitCode {
    let scan = report.scan;
    let on_records = scan.record_rules();
    if !on_records.is_empty() {
        eprintln!(
            "trex scan: {} fire on records, and a record is whole only once the input holding it ends, which a followed file does not; follow the rules that fire on matches",
            on_records.join(", ")
        );
        return ExitCode::FAILURE;
    }
    if !scan.stream(None, 0, None, None).commits_early() {
        say_uncommittable("scan");
        return ExitCode::FAILURE;
    }
    let names: Vec<String> = sources.iter().map(Source::name).collect();
    let mut failed = failed;
    let mut errors = 0usize;
    let mut streams: Vec<trex::rule_scan::RuleStream<'_>> = Vec::new();
    let mut printed: Vec<usize> = Vec::new();
    let mut decoders: Vec<Incremental> = Vec::new();
    let mut owners: Vec<usize> = Vec::new();
    let mut followees: Vec<(std::path::PathBuf, usize)> = Vec::new();
    // Print a batch of findings, at most as many as `-m` leaves the file.
    let print = |found: Vec<trex::rule_scan::Found>, count: &mut usize, errors: &mut usize| {
        for mut one in found {
            if let Some(n) = run.max_count {
                one.findings.truncate(n.saturating_sub(*count));
            }
            let (total, errs) =
                report.print_findings(std::slice::from_ref(&one), prefixed, &mut |object| crate::out::line(&object));
            *count += total;
            *errors += errs;
        }
    };
    for (k, src) in sources.iter().enumerate() {
        let Source::File(path) = src else {
            continue;
        };
        let part = match read_part(src, run.windowing.select, &run.unit, asked) {
            Ok(part) => part,
            Err(e) => {
                eprintln!("trex: cannot read {}: {e}", names[k]);
                failed = true;
                continue;
            }
        };
        if part.binary && !run.binary {
            eprintln!("trex: {} holds a NUL byte and is binary; --binary scans it", names[k]);
            failed = true;
            continue;
        }
        let mut stream = scan.stream(Some(names[k].as_str()), part.base(), part.line_base, None);
        let mut count = 0usize;
        print(stream.push(&part.text), &mut count, &mut errors);
        streams.push(stream);
        printed.push(count);
        decoders.push(Incremental::after(part.encoding));
        owners.push(k);
        followees.push((path.clone(), part.input_len));
    }
    let all_done = |printed: &[usize]| run.max_count.is_some_and(|n| printed.iter().all(|&c| c >= n));
    let exit = |failed: bool, errors: usize| if failed || errors > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    if streams.is_empty() || all_done(&printed) {
        return exit(failed, errors);
    }
    let mut follower = match Follower::new(&followees) {
        Ok(follower) => follower,
        Err(e) => {
            eprintln!("trex: cannot follow: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(why) = follower.unnotified() {
        eprintln!("trex: no change notifications ({why}); looking at the files once a second");
    }
    loop {
        let (i, change) = match follower.wait() {
            Ok(next) => next,
            Err(e) => {
                eprintln!("trex: cannot follow {e}");
                return ExitCode::FAILURE;
            }
        };
        let name = &names[owners[i]];
        match change {
            Change::Appended { bytes, .. } => {
                let text = decoders[i].decode(&bytes);
                print(streams[i].push(&text), &mut printed[i], &mut errors);
                if all_done(&printed) {
                    return exit(failed, errors);
                }
            }
            other => {
                say_change(name, &other, "scanning it");
                let fresh = scan.stream(Some(name.as_str()), 0, Some(0), None);
                print(std::mem::replace(&mut streams[i], fresh).finish(), &mut printed[i], &mut errors);
                decoders[i] = Incremental::from_start();
                // The file now under the name is a new input, whose `-m` count
                // starts again unless `--keep-count` carries the old one's on.
                if !run.keep_count {
                    printed[i] = 0;
                } else if all_done(&printed) {
                    return exit(failed, errors);
                }
            }
        }
    }
}

/// The edits a followed rewrite or redaction makes at a batch of committed
/// matches: the bytes held and the matches' spans in them, to edits of those
/// bytes.
pub(crate) type EditsAt<'a> = dyn FnMut(&[u8], &[trex::Span]) -> Vec<Edit> + 'a;

/// How many matches a followed edit makes: `-m`, and whether `--keep-count`
/// carries the count through a truncation or rotation rather than starting it
/// again for the file then under the name.
#[derive(Clone, Copy, Default)]
pub(crate) struct EditCount {
    pub(crate) max_count: Option<usize>,
    pub(crate) keep_count: bool,
}

/// Rewrite the file at `path` from its window on, `part`, then follow it as
/// it grows, writing each byte to the standard output once nothing that
/// arrives later can change it, each match as `edits` rewrites it until
/// `count` caps them. Runs until it is interrupted.
pub(crate) fn follow_edit(
    command: &str,
    path: &std::path::Path,
    part: &Part,
    pattern: &trex::ast::Pattern,
    shapes: &trex::ShapeSet,
    count: EditCount,
    edits: &mut EditsAt<'_>,
) -> ExitCode {
    let name = path.display().to_string();
    let scanner = || trex::StreamScanner::with_shapes(pattern.clone(), shapes.clone());
    let mut stream = trex::EditStream::new(scanner(), part.base(), 0);
    if !stream.commits_early() {
        say_uncommittable(command);
        return ExitCode::FAILURE;
    }
    let mut edits_at = |held: &[u8], _base: usize, spans: &[trex::Span]| -> Result<Vec<Edit>, std::convert::Infallible> {
        Ok(edits(held, spans))
    };
    let limit = count.max_count;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut written = |bytes: Result<Vec<u8>, std::convert::Infallible>| {
        let bytes = bytes.unwrap_or_else(|never| match never {});
        match out.write_all(&bytes).and_then(|()| out.flush()) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("trex: cannot write the standard output: {e}");
                false
            }
        }
    };
    if !written(stream.push(&part.text, limit, &mut edits_at)) {
        return ExitCode::FAILURE;
    }
    let mut follower = match Follower::new(&[(path.to_path_buf(), part.input_len)]) {
        Ok(follower) => follower,
        Err(e) => {
            eprintln!("trex: cannot follow {name}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(why) = follower.unnotified() {
        eprintln!("trex: no change notifications ({why}); looking at the file once a second");
    }
    let mut decoder = Incremental::after(part.encoding);
    loop {
        let change = match follower.wait() {
            Ok((_, change)) => change,
            Err(e) => {
                eprintln!("trex: cannot follow {e}");
                return ExitCode::FAILURE;
            }
        };
        match change {
            Change::Appended { bytes, .. } => {
                let text = decoder.decode(&bytes);
                if !written(stream.push(&text, limit, &mut edits_at)) {
                    return ExitCode::FAILURE;
                }
            }
            other => {
                say_change(&name, &other, "reading it");
                let old = std::mem::replace(&mut stream, trex::EditStream::new(scanner(), 0, 0));
                let ended = old.finish(limit, &mut edits_at);
                // The file now under the name is a new input, whose `-m` count
                // starts again unless `--keep-count` carries the old one's on.
                let edited = ended.as_ref().map_or(0, |(_, edited)| *edited);
                if !written(ended.map(|(rest, _)| rest)) {
                    return ExitCode::FAILURE;
                }
                if count.keep_count {
                    stream = trex::EditStream::new(scanner(), 0, edited);
                }
                decoder = Incremental::from_start();
            }
        }
    }
}
