//! The scan, rewrite, redact and aggregation commands over one input or
//! many: a file, `-` for the standard input, a directory walked under ignore
//! rules, or the standard input when no path is given at all.
//!
//! One input reports as it always has: the span and its text per match. Many
//! inputs, or a directory, or `-H`, prefix each match with `path:line:col:`,
//! and `-A`, `-B` and `-C` print the lines around a match's first line as
//! `path-line-` context. The files are read and scanned across the cores and
//! reported in path order; an error reading one is reported and the rest are
//! still scanned.

use std::process::ExitCode;

use trex::files::{LineIndex, Source, WalkOptions, collect, is_binary, read_source};
use trex::records::{Quantifier, Query, RecordHit};
use trex::report::{Explaining, Members};

use crate::cli_window::{Part, Piece, Restrict, Windowing, edits_within, read_part};

/// The lines of context a dry run's diff shows around each change where
/// `-C` names no other number.
pub(crate) const DIFF_CONTEXT: usize = 3;

/// A number a flag takes.
fn count_arg(flag: &str, value: Option<&String>) -> Result<usize, String> {
    let Some(v) = value else {
        return Err(format!("{flag} needs a number"));
    };
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{flag} needs a number, not {v:?}"));
    }
    let mut n = 0usize;
    for b in v.bytes() {
        n = n
            .checked_mul(10)
            .and_then(|n| n.checked_add(usize::from(b - b'0')))
            .ok_or_else(|| format!("{flag} {v} is too large"))?;
    }
    Ok(n)
}

/// What one input came to.
enum Outcome {
    /// The tree's index says no pattern can match here, so it was never
    /// opened.
    Skipped,
    /// Not read yet.
    Pending,
    /// Held a NUL byte and `--binary` was not given.
    Binary,
    /// Could not be read.
    Failed(String),
    /// Read and scanned; the input is the part a window took, standing at
    /// `byte_base` after `line_base` lines of the whole where each was
    /// counted; the members are which set member made each match, parallel
    /// to the matches and empty for a scan of one pattern; the route is the
    /// rung that answered, kept only when the report explains its matches.
    Scanned {
        input: Vec<u8>,
        byte_base: Option<usize>,
        line_base: Option<usize>,
        matches: Vec<trex::Match>,
        members: Vec<usize>,
        route: String,
    },
    /// Read and asked only whether anything matched, for a report that wants
    /// no more than that. The matches were never collected, so nothing here
    /// can report how many there were.
    Answered { any: bool },
    /// Read and asked a record-level query: the records that qualify.
    Queried { input: Vec<u8>, byte_base: Option<usize>, line_base: Option<usize>, hits: Vec<RecordHit> },
}

/// Scan `input` the way the flags ask, returning matches with their
/// registers resolved; under `lists`, with every binding a register made
/// under a repetition, which the report reads.
#[allow(clippy::too_many_arguments)]
fn scan_one(
    pattern: &trex::ast::Pattern,
    input: &[u8],
    shapes: &trex::ShapeSet,
    backend: trex::Backend,
    dual_grain: bool,
    chunk_size: Option<usize>,
    say: bool,
    lists: bool,
    take: Option<usize>,
) -> Vec<trex::Match> {
    // Declared shapes and kinds decide token boundaries, as a library kind
    // does, so they take the lexing path that knows about them. The pipeline
    // modes below lex for themselves and do not carry a shape set.
    if !shapes.is_empty() || !pattern.library_kinds().is_empty() {
        let spans = trex::scan_with_shapes(pattern, input, shapes);
        return if lists {
            trex::captures_with_shapes_and_lists(pattern, input, shapes, &spans)
        } else {
            trex::captures_with_shapes(pattern, input, shapes, &spans)
        };
    }
    // A forced `--gpu` takes precedence; otherwise the explicit pipeline
    // modes (`--dual-grain`, `--chunk-size`) win, and the plain whole-input
    // scan is routed by the chosen backend (`Auto` places a kind-only pattern
    // on the CPU engine or splits it across the cores and a present device,
    // by what it has measured at the input's size; `--cpu` / `--nogpu` stay
    // on the CPU). All return the same matches.
    let spans = if backend == trex::Backend::Gpu {
        let (m, used) = trex::scan_with_backend(pattern, input, backend);
        if say {
            if used == trex::BackendUsed::Gpu {
                eprintln!("gpu: device scan ({} matches)", m.len());
            } else {
                eprintln!("gpu: not applicable (pattern outside device subset, no device, or built without --features gpu); using CPU");
            }
        }
        m
    } else if dual_grain {
        let (m, timing) = trex::scan_dual_grain(pattern, input);
        if say {
            // The timing goes to stderr so --json output stays clean.
            eprintln!(
                "dual-grain: {} chunks; byte grain busy {:.2} ms, token grain busy {:.2} ms, wall {:.2} ms; overlap >= {:.2} ms ({})",
                timing.chunks,
                timing.producer_busy.as_secs_f64() * 1000.0,
                timing.consumer_busy.as_secs_f64() * 1000.0,
                timing.wall.as_secs_f64() * 1000.0,
                timing.overlap().as_secs_f64() * 1000.0,
                if timing.overlapped() { "grains overlapped in time" } else { "no overlap measured" },
            );
        }
        m
    } else if let Some(cs) = chunk_size {
        trex::scan_chunked(pattern, input.chunks(cs).collect::<Vec<_>>())
    } else if let Some(n) = take {
        // A report that prints at most `n` matches walks only that far. The
        // cursor stops where the take does, so the rest of the input is never
        // scanned; a whole-input scan would find every match and then throw
        // all but the first `n` away.
        //
        // Only the plain path takes this. The device and chunked paths scan
        // a whole input by construction, and a shaped scan lexes first, so
        // neither has a walk to stop early.
        trex::find_iter(pattern, input).take(n).collect()
    } else {
        let (m, used) = trex::scan_with_backend(pattern, input, backend);
        if say && used == trex::BackendUsed::Split {
            eprintln!("auto: split across the cores and the GPU device ({} matches)", m.len());
        }
        m
    };
    // A scan reports spans; the registers a binding pattern bound are
    // resolved over them here, once, for the report.
    if lists {
        trex::captures_with_lists(pattern, input, &spans)
    } else {
        trex::captures(pattern, input, &spans)
    }
}

/// The indexes a scan of these paths may consult: one per directory named
/// on the command line that holds one, in the order given.
///
/// A file is answered by the first index whose root contains it. A path
/// named outright rather than walked is scanned whatever any index says,
/// as a glob does not filter one either: naming a file is asking for it.
pub(crate) struct Indexes {
    trees: Vec<trex::index::Index>,
}

impl Indexes {
    /// The indexes found under the directories among `paths`, or none where
    /// the scan was told to read none.
    fn found(paths: &[String], read_them: bool) -> Indexes {
        let mut trees = Vec::new();
        if read_them {
            for path in paths {
                let dir = std::path::Path::new(path);
                if dir.is_dir()
                    && let Some(index) = trex::index::Index::load(dir)
                {
                    trees.push(index);
                }
            }
        }
        Indexes { trees }
    }

    /// Whether every one of `patterns` certainly cannot match `path`, so the
    /// file need not be opened.
    fn refuses(&self, patterns: &[&trex::ast::Pattern], path: &Source) -> bool {
        let Source::File(path) = path else {
            return false;
        };
        self.trees.iter().any(|t| t.refuses_every(patterns, path))
    }
}

/// What a scan runs: one pattern, or the set `--patterns` names, whose
/// every match carries the member that made it.
pub(crate) enum Scanning {
    One(trex::ast::Pattern),
    Set(Box<trex::PatternSet>),
}

impl Scanning {
    /// The set, where the scan runs one.
    pub(crate) fn set(&self) -> Option<&trex::PatternSet> {
        match self {
            Scanning::One(_) => None,
            Scanning::Set(set) => Some(set.as_ref()),
        }
    }

    /// The pattern member `i` is: the one pattern, or the set's member.
    pub(crate) fn pattern(&self, i: usize) -> &trex::ast::Pattern {
        match self {
            Scanning::One(p) => p,
            Scanning::Set(set) => &set.patterns()[i],
        }
    }

    /// The register names a report's template may write: the pattern's, or
    /// every member's in member order, each once, so `${1}` counts through
    /// the set's registers as it counts through one pattern's.
    fn capture_names(&self) -> Vec<String> {
        match self {
            Scanning::One(p) => p.capture_names(),
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

    /// The kind each register binds, in the order [`Self::capture_names`]
    /// reports them.
    ///
    /// Across a set, a name two members bind on different kinds has no one
    /// kind and reports `None`: the same register would otherwise carry a
    /// value read as one kind for matches of the other.
    pub(crate) fn capture_kinds(&self) -> Vec<(String, Option<trex::token::TokenKind>)> {
        match self {
            Scanning::One(p) => p.capture_kinds(),
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

    /// Whether any pattern binds a register under a repetition.
    fn has_list_registers(&self) -> bool {
        match self {
            Scanning::One(p) => p.has_list_registers(),
            Scanning::Set(set) => set.patterns().iter().any(trex::ast::Pattern::has_list_registers),
        }
    }

    /// Whether any pattern names a library kind, which only a lex under the
    /// library's shapes produces.
    fn names_library_kinds(&self) -> bool {
        match self {
            Scanning::One(p) => !p.library_kinds().is_empty(),
            Scanning::Set(set) => set.patterns().iter().any(|p| !p.library_kinds().is_empty()),
        }
    }

    /// The patterns an index is asked about: the one pattern, or every
    /// member of the set, since a file is skipped only where none can match.
    fn patterns(&self) -> Vec<&trex::ast::Pattern> {
        match self {
            Scanning::One(p) => vec![p],
            Scanning::Set(set) => set.patterns().iter().collect(),
        }
    }
}

/// How a scan runs beyond the pattern: the backend and pipeline modes it is
/// routed through, whether it says which route it took, whether it keeps
/// every binding under a repetition, and whether a set stops each member at
/// its first match.
#[derive(Clone, Copy)]
pub(crate) struct ScanHow {
    pub(crate) backend: trex::Backend,
    pub(crate) dual_grain: bool,
    pub(crate) chunk_size: Option<usize>,
    pub(crate) say: bool,
    pub(crate) lists: bool,
    pub(crate) single: bool,
    /// At most this many matches, where the report prints no more and the
    /// path it takes can stop early. Absent for every other report.
    pub(crate) take: Option<usize>,
}

/// Scan `input` under what the scan runs: the one pattern the way the flags
/// ask, or the set's members over one lex, resolved to matches in position
/// order with the member that made each beside it.
pub(crate) fn scan_found(
    scanning: &Scanning,
    input: &[u8],
    shapes: &trex::ShapeSet,
    how: ScanHow,
) -> (Vec<trex::Match>, Vec<usize>) {
    match scanning {
        Scanning::One(p) => {
            (scan_one(p, input, shapes, how.backend, how.dual_grain, how.chunk_size, how.say, how.lists, how.take), Vec::new())
        }
        Scanning::Set(set) => {
            let found = if how.single { set.first_matches(input, how.lists) } else { set.scan_matches(input, how.lists) };
            let (members, matches): (Vec<usize>, Vec<trex::Match>) = found.into_iter().unzip();
            (matches, members)
        }
    }
}

/// Explainers for the patterns whose matches a report prints: the one
/// pattern, or every member of the set with a match.
fn explaining_over<'a>(
    scanning: &'a Scanning,
    input: &'a [u8],
    shapes: &trex::ShapeSet,
    members: Option<Members<'_>>,
    route: &'a str,
) -> Explaining<'a> {
    let count = scanning.set().map_or(1, trex::PatternSet::len);
    Explaining::over(count, |i| scanning.pattern(i), input, shapes, members, route)
}

/// The lines an explanation adds under a match, as
/// [`trex::report::explanation_lines`] writes them.
fn print_explanation(e: &trex::explain::Explanation) {
    for line in trex::report::explanation_lines(e) {
        crate::out::line(&line);
    }
}

/// The one-input report, as [`trex::report::human_report`] writes it, the
/// offsets the ones `index` gives.
pub(crate) fn print_human_about(
    input: &[u8],
    matches: &[trex::Match],
    index: &LineIndex,
    painter: &trex::paint::Painter,
    members: Option<Members<'_>>,
    explaining: Option<&Explaining<'_>>,
) {
    trex::report::human_report(input, matches, index, painter, members, explaining, &mut |l| crate::out::line(l));
}

/// The one-input JSON report, as [`trex::report::json_report`] writes it,
/// the offsets the ones `index` gives.
pub(crate) fn print_json_about(
    input: &[u8],
    matches: &[trex::Match],
    index: &LineIndex,
    members: Option<Members<'_>>,
    explaining: Option<&Explaining<'_>>,
    values: Option<&crate::ValueView<'_>>,
) {
    crate::out::line(&trex::report::json_report(input, matches, index, members, explaining, values));
}

/// Declare everything the pattern file at `path` says into `shapes`.
fn declare_file(shapes: &mut trex::ShapeSet, path: &str) -> Result<(), String> {
    match shapes.declare_file(std::path::Path::new(path)) {
        Ok(()) => Ok(()),
        Err(e) => Err(e.msg),
    }
}

/// Which table an aggregation prints.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Aggregate {
    /// Every key with its count, ordered by key.
    CountBy,
    /// Every key with its count, most frequent first.
    Top,
    /// The distinct keys alone, ordered by key.
    Uniq,
}

impl Aggregate {
    fn name(self) -> &'static str {
        match self {
            Aggregate::CountBy => "count-by",
            Aggregate::Top => "top",
            Aggregate::Uniq => "uniq",
        }
    }
}

/// `trex index`: write each named tree's index, so a later scan of it opens
/// only the files that can match.
///
/// The index holds one summary a file - which token kinds its lex made, a
/// filter over its words, the range its numbers span and the range its
/// timestamps span - and a scan of the tree skips any file every one of its
/// patterns is refused by. It is an optimization only: a file the index has
/// not seen or that has changed since is scanned, so an index that is stale
/// or absent costs time and never a match.
pub fn run_index(args: &[String]) -> ExitCode {
    let mut paths: Vec<String> = Vec::new();
    let mut walk = WalkOptions::default();
    let mut list = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--hidden" => walk.hidden = true,
            "--no-ignore" => walk.no_ignore = true,
            "--list" => list = true,
            "-g" | "--glob" => {
                i += 1;
                match args.get(i) {
                    Some(g) => walk.globs.push(g.clone()),
                    None => {
                        eprintln!("trex index: -g needs a glob");
                        return ExitCode::FAILURE;
                    }
                }
            }
            other => paths.push(other.to_string()),
        }
        i += 1;
    }
    if paths.is_empty() {
        eprintln!("usage: trex index DIR... [--list] [-g GLOB]... [--hidden] [--no-ignore]");
        eprintln!("  writes {} at each tree's root; --list reports what one holds", trex::index::INDEX_FILE);
        return ExitCode::FAILURE;
    }
    if let Err(e) = walk.check() {
        eprintln!("trex: {e}");
        return ExitCode::FAILURE;
    }
    let mut failed = false;
    for path in &paths {
        let dir = std::path::Path::new(path);
        if !dir.is_dir() {
            eprintln!("trex index: {path} is not a directory; an index covers a tree");
            failed = true;
            continue;
        }
        if list {
            match trex::index::Index::load(dir) {
                Some(index) => crate::out::line(&format!(
                    "{}: {} files indexed",
                    dir.join(trex::index::INDEX_FILE).display(),
                    index.len()
                )),
                None => {
                    eprintln!("trex index: {path} holds no index this trex wrote");
                    failed = true;
                }
            }
            continue;
        }
        let (sources, errors) = collect(std::slice::from_ref(path), &walk);
        for e in &errors {
            eprintln!("trex: {e}");
            failed = true;
        }
        let files: Vec<std::path::PathBuf> = sources
            .iter()
            .filter_map(|s| match s {
                Source::File(p) => Some(p.clone()),
                Source::Stdin => None,
            })
            .collect();
        let index = trex::index::Index::build(dir, &files);
        match index.save(dir) {
            Ok(()) => crate::out::line(&format!("{path}: {} files indexed", index.len())),
            Err(e) => {
                eprintln!("trex: cannot write {}: {e}", dir.join(trex::index::INDEX_FILE).display());
                failed = true;
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// The single register `spec` names, written as a report reference
/// (`${size}`) or bare (`size`), or `None` where it names anything else.
///
/// An aggregate reads one capture's value, so a composite like
/// `${a}/${b}` and an accessor chain like `${u:host}` are refused rather
/// than guessed at: the first names two values and the second names a slice
/// of text, and neither is a number to sum.
fn one_register(spec: &str) -> Option<String> {
    let body = match spec.strip_prefix("${") {
        Some(rest) => rest.strip_suffix('}')?,
        None => spec,
    };
    let name = body.trim();
    if name.is_empty() || name.contains(['$', '{', '}', ':', '|']) {
        return None;
    }
    Some(name.to_string())
}

/// `trex count-by` / `top` / `uniq`: scan a pattern, or the set `--patterns`
/// names, and group the matches by a key rendered from the report template
/// language, so a capture's typed slice (`${ip:octet1-2}`, `${u:host}`,
/// `${e:domain}`), the input a match stands in (`${path}`) or the set member
/// that made it (`${pattern}`) is what the rows count.
///
/// Every key is printed. `-n` cuts the table to the caller's own number, and
/// nothing cuts it otherwise: a row missing from a table reads exactly like a
/// key that never occurred.
pub fn run_aggregate(which: Aggregate, args: &[String]) -> ExitCode {
    let name = which.name();
    if args.is_empty() {
        eprintln!(
            "usage: trex {name} PATTERN KEY [FILE|DIR|-]... [--text STRING] [--lib FILE] [--shape DECL] [--json] [-n N] [--hidden] [--no-ignore] [--head N|--tail N|--lines A..B] [--record UNIT]"
        );
        eprintln!("       trex {name} --patterns FILE KEY [FILE|DIR|-]...");
        eprintln!(
            "  KEY is a report template over the pattern's captures and the match's place, e.g. '${{ip:octet1-2}}', '${{path}}' or '${{pattern}}'"
        );
        return ExitCode::FAILURE;
    }
    let mut positionals: Vec<String> = Vec::new();
    let mut text: Option<Vec<u8>> = None;
    let mut json = false;
    let mut rows: Option<usize> = None;
    let mut shapes = trex::ShapeSet::new();
    let mut walk = WalkOptions::default();
    let mut patterns_file: Option<String> = None;
    // Each aggregate and the capture it reads, in the order asked for, which
    // is the order the columns print in.
    let mut aggs: Vec<(trex::typed::Agg, String)> = Vec::new();
    let mut pct = trex::typed::Percentile::Nearest;
    let mut form = trex::typed::QuotientForm::Repetend;
    let mut style = trex::typed::ValueStyle::new();
    let mut windowing = Windowing::default();
    let mut record: Option<RecordSpec> = None;
    let mut i = 0;
    while i < args.len() {
        match windowing.take(args, &mut i) {
            Some(Ok(())) => {
                i += 1;
                continue;
            }
            Some(Err(e)) => {
                eprintln!("trex: {e}");
                return ExitCode::FAILURE;
            }
            None => {}
        }
        match args[i].as_str() {
            flag @ ("--record" | "--record-start" | "--record-span") => {
                if let Err(e) = take_record(flag, args, &mut i, &mut record) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--patterns" => {
                i += 1;
                let Some(file) = args.get(i) else {
                    eprintln!("trex: --patterns needs a pattern file");
                    return ExitCode::FAILURE;
                };
                if patterns_file.is_some() {
                    eprintln!("trex: --patterns names a second file; a table runs one set");
                    return ExitCode::FAILURE;
                }
                patterns_file = Some(file.clone());
            }
            "--shape" | "--shape-after" => {
                let when = if args[i] == "--shape" {
                    trex::Precedence::Before
                } else {
                    trex::Precedence::After
                };
                i += 1;
                let Some(decl) = args.get(i) else {
                    eprintln!("trex: {} needs a `name = `pattern`` declaration", args[i - 1]);
                    return ExitCode::FAILURE;
                };
                if let Err(e) = shapes.declare(decl, when) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--lib" => {
                i += 1;
                let Some(path) = args.get(i) else {
                    eprintln!("trex: --lib needs a pattern file");
                    return ExitCode::FAILURE;
                };
                if let Err(e) = declare_file(&mut shapes, path) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                text = Some(v.clone().into_bytes());
            }
            "-n" => {
                i += 1;
                match count_arg("-n", args.get(i)) {
                    Ok(v) => rows = Some(v),
                    Err(e) => {
                        eprintln!("trex: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--json" => json = true,
            "--hidden" => walk.hidden = true,
            "--no-ignore" => walk.no_ignore = true,
            "--sum" | "--avg" | "--min" | "--max" | "--p50" | "--p95" => {
                let agg = match args[i].as_str() {
                    "--sum" => trex::typed::Agg::Sum,
                    "--avg" => trex::typed::Agg::Avg,
                    "--min" => trex::typed::Agg::Min,
                    "--max" => trex::typed::Agg::Max,
                    "--p50" => trex::typed::Agg::Pct(50),
                    _ => trex::typed::Agg::Pct(95),
                };
                let flag = args[i].clone();
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: {flag} needs a capture, as in '${{size}}'");
                    return ExitCode::FAILURE;
                };
                aggs.push((agg, v.clone()));
            }
            "--percentile" => {
                i += 1;
                match args.get(i).map(String::as_str) {
                    Some("nearest") => pct = trex::typed::Percentile::Nearest,
                    Some("linear") => pct = trex::typed::Percentile::Linear,
                    Some("lower") => pct = trex::typed::Percentile::Lower,
                    Some("hybrid") => pct = trex::typed::Percentile::Hybrid,
                    other => {
                        let given = other.unwrap_or("nothing");
                        eprintln!(
                            "trex: --percentile takes nearest, linear, lower or hybrid, not \
                             {given}. nearest names a value that occurred; linear interpolates \
                             and takes numeric kinds only; lower names the value at or below; \
                             hybrid interpolates where the kind allows it and names an observed \
                             value where it does not."
                        );
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--avg-form" => {
                i += 1;
                match args.get(i).map(String::as_str) {
                    Some("repetend") => form = trex::typed::QuotientForm::Repetend,
                    Some("rational") => form = trex::typed::QuotientForm::Rational,
                    other => {
                        let given = other.unwrap_or("nothing");
                        eprintln!(
                            "trex: --avg-form takes repetend or rational, not {given}. \
                             repetend writes 10/3 as 3.(3), rational as 10/3; both are exact."
                        );
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--values" => {
                i += 1;
                match args.get(i).map(String::as_str) {
                    Some("exact") => style.spelling = trex::typed::ValueSpelling::Exact,
                    Some("natural") => style.spelling = trex::typed::ValueSpelling::Natural,
                    Some("tagged") => style.spelling = trex::typed::ValueSpelling::Tagged,
                    other => {
                        let given = other.unwrap_or("nothing");
                        eprintln!("trex: --values takes exact, natural or tagged, not {given}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--duration-unit" => {
                i += 1;
                match args.get(i).map(String::as_str) {
                    Some("ns") => style.duration = trex::typed::DurationUnit::Nanoseconds,
                    Some("ms") => style.duration = trex::typed::DurationUnit::Milliseconds,
                    Some("s") => style.duration = trex::typed::DurationUnit::Seconds,
                    other => {
                        let given = other.unwrap_or("nothing");
                        eprintln!("trex: --duration-unit takes ns, ms or s, not {given}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            other => positionals.push(other.to_string()),
        }
        i += 1;
    }
    // What the table scans: the set `--patterns` names, or the first
    // positional as the pattern; then the key, and the inputs.
    let scanning = match &patterns_file {
        Some(file) => match trex::PatternSet::from_file(std::path::Path::new(file), &mut shapes) {
            Ok(set) if set.is_empty() => {
                eprintln!("trex {name}: {file} holds no pattern");
                return ExitCode::FAILURE;
            }
            Ok(set) => Scanning::Set(Box::new(set)),
            Err(e) => {
                eprintln!("trex: {}", e.msg);
                return ExitCode::FAILURE;
            }
        },
        None => {
            if positionals.is_empty() {
                eprintln!("trex {name}: a pattern is needed: PATTERN or --patterns FILE");
                return ExitCode::FAILURE;
            }
            let pattern_src = positionals.remove(0);
            match trex::parser::parse_with_shapes(&pattern_src, &shapes) {
                Ok(p) => Scanning::One(p),
                Err(e) => {
                    eprintln!("trex: pattern error at byte {}: {}", e.pos, e.msg);
                    return ExitCode::FAILURE;
                }
            }
        }
    };
    if positionals.is_empty() {
        eprintln!("trex {name}: a KEY template is needed, e.g. '${{0}}' or '${{pattern}}'");
        return ExitCode::FAILURE;
    }
    let key_src = positionals.remove(0);
    let paths = positionals;
    let template = match trex::Template::parse_report(&key_src, &scanning.capture_names()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("trex: key error at byte {}: {}", e.pos, e.msg);
            return ExitCode::FAILURE;
        }
    };
    // A key groups matches, and an axis reading is not a key: nothing here
    // builds an explainer, so every `${@axis}` would render empty and every
    // match would land under the same key. Refused rather than answered
    // wrongly.
    if template.reads_explanation() {
        eprintln!(
            "trex {name}: a KEY groups matches and reads no axis; \
             ${{@axis}} is a scan's --format field, where --explain computes it"
        );
        return ExitCode::FAILURE;
    }
    // An aggregate is refused here, before a byte is read: the pattern
    // already says which kind each register binds, so a mistake visible in
    // the arguments never costs a scan to discover.
    let capture_kinds = scanning.capture_kinds();
    let mut columns: Vec<(trex::typed::Agg, String, trex::token::TokenKind)> = Vec::new();
    for (agg, spec) in &aggs {
        let flag = agg.flag();
        let Some(register) = one_register(spec) else {
            eprintln!(
                "trex: {flag} takes one capture, as in '${{size}}', not {spec:?}"
            );
            return ExitCode::FAILURE;
        };
        let Some(found) = capture_kinds.iter().find(|(n, _)| *n == register) else {
            eprintln!("trex: {flag} names ${{{register}}}, which the pattern does not bind");
            return ExitCode::FAILURE;
        };
        let Some(kind) = found.1 else {
            eprintln!(
                "trex: {flag} needs a capture binding one typed kind, and ${{{register}}} binds \
                 a run, a repetition or an alternation, whose kind varies between matches"
            );
            return ExitCode::FAILURE;
        };
        if !agg.admits(kind) {
            let what = kind.name();
            let why = if agg.needs_arithmetic() {
                "adding one is not defined. Numeric kinds are number, bytesize, duration, money \
                 and percent"
            } else {
                "it carries no value to order. Ordered kinds are the numeric ones plus timestamp, \
                 version and address"
            };
            eprintln!("trex: {flag} over ${{{register}}}, which binds a {what}, and {why}.");
            return ExitCode::FAILURE;
        }
        if pct == trex::typed::Percentile::Linear
            && matches!(agg, trex::typed::Agg::Pct(_))
            && trex::typed::aggregable(kind) != trex::typed::Aggregable::Numeric
        {
            eprintln!(
                "trex: --percentile linear cannot interpolate a {}, which ${{{register}}} binds. \
                 Use nearest, lower, or hybrid, which interpolates only where a kind allows it.",
                kind.name()
            );
            return ExitCode::FAILURE;
        }
        columns.push((*agg, register, kind));
    }
    if windowing.follow {
        eprintln!("trex {name}: a table is printed once its inputs end, and a followed file does not end; drop --follow");
        return ExitCode::FAILURE;
    }
    let unit = match window_unit(name, &windowing, record.as_ref(), &shapes) {
        Ok(unit) => unit,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let how = ScanHow {
        backend: trex::Backend::Auto,
        dual_grain: false,
        chunk_size: None,
        say: false,
        lists: template.reads_lists(),
        single: false,
        // A table counts every match, so there is nothing to stop short of.
        take: None,
    };
    let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    // One collection per key per aggregate column, in the columns' order.
    let mut gathered: std::collections::HashMap<String, Vec<trex::typed::Collected>> =
        std::collections::HashMap::new();
    let mut matched = 0u64;
    let mut failed = false;
    let mut tally = |path: &str, part: &Part| {
        let input = part.text.as_slice();
        let (matches, of_member) = scan_found(&scanning, input, &shapes, how);
        // The line and column are counted only for a key that writes them.
        let index = template.reads_place().then(|| part.index());
        for (k, m) in matches.iter().enumerate() {
            matched += 1;
            let (line, col) = index.as_ref().map_or((0, 0), |ix| ix.line_col(input, m.start));
            let member = scanning.set().map(|set| set.name(of_member[k]));
            let place =
                trex::ReportAt { path, line, col, base: part.byte_base, pattern: member.as_deref(), rule: None };
            let key = template.render_report(m, input, &place);
            *counts.entry(key.clone()).or_insert(0) += 1;
            if columns.is_empty() {
                continue;
            }
            let slot = gathered
                .entry(key)
                .or_insert_with(|| vec![trex::typed::Collected::default(); columns.len()]);
            for (c, (_, register, kind)) in columns.iter().enumerate() {
                // A match that does not bind the register contributes no
                // value to it, which is not the same as contributing a zero.
                let Some(text) = m.group(register, input) else { continue };
                let Some(v) = trex::typed::value_of(*kind, &String::from_utf8_lossy(text)) else {
                    continue;
                };
                slot[c].push(v);
            }
        }
    };
    // Every input is counted, a binary one as the bytes it holds, and read as
    // a scan reads it: decoded, and cut to the window where one is named.
    let asked = trex::window::Asked {
        numbers: template.reads_place(),
        offsets: template.reads_offsets(),
        binary: true,
        units: false,
    };
    match (&text, paths.is_empty()) {
        (Some(bytes), _) => tally("", &Part::of_text(bytes.clone(), windowing.select, &unit)),
        (None, true) => match read_part(&Source::Stdin, windowing.select, &unit, asked) {
            Ok(part) => tally("-", &part),
            Err(e) => {
                eprintln!("trex: -: {e}");
                failed = true;
            }
        },
        (None, false) => {
            let (sources, errors) = trex::files::collect(&paths, &walk);
            for e in &errors {
                eprintln!("trex: {e}");
                failed = true;
            }
            for src in &sources {
                match read_part(src, windowing.select, &unit, asked) {
                    Ok(part) => tally(&src.name(), &part),
                    Err(e) => {
                        eprintln!("trex: {}: {e}", src.name());
                        failed = true;
                    }
                }
            }
        }
    }
    let mut table: Vec<(String, u64)> = counts.into_iter().collect();
    match which {
        // Most frequent first, and by key among equal counts so the order is
        // the same on every run.
        Aggregate::Top => table.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0))),
        Aggregate::CountBy | Aggregate::Uniq => table.sort_by(|a, b| a.0.cmp(&b.0)),
    }
    let shown = rows.unwrap_or(table.len()).min(table.len());
    // Each shown key's aggregate cells, in the columns' order. A key whose
    // matches bound no value prints `-` rather than a zero, which would read
    // as a sum of nothing rather than as nothing to sum.
    let clock = trex::Clock::current();
    let cells: Vec<Vec<String>> = table[..shown]
        .iter()
        .map(|(key, _)| {
            let held = gathered.get(key);
            columns
                .iter()
                .enumerate()
                .map(|(c, (agg, _, kind))| {
                    held.and_then(|v| v[c].report(*agg, *kind, style, form, pct, clock))
                        .unwrap_or_else(|| "-".to_string())
                })
                .collect()
        })
        .collect();
    let headings: Vec<String> =
        columns.iter().map(|(agg, register, _)| format!("{} {register}", agg.flag())).collect();
    print_aggregate(which, &table[..shown], table.len(), matched, json, &headings, &cells);
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// Print an aggregation's rows. A key the accessor left empty prints as `-`,
/// so the counts still sum to the matches and a field absent from most of the
/// input reads as absent rather than as rare.
///
/// `headings` names each aggregate column and `cells` holds one row of them
/// per printed key, both empty where no aggregate was asked for, which is the
/// table as it has always printed.
fn print_aggregate(
    which: Aggregate,
    rows: &[(String, u64)],
    keys: usize,
    matched: u64,
    json: bool,
    headings: &[String],
    cells: &[Vec<String>],
) {
    let shown = |k: &str| if k.is_empty() { "-".to_string() } else { k.to_string() };
    if json {
        let mut out = String::from("[");
        for (i, (key, count)) in rows.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            if which == Aggregate::Uniq {
                out.push_str(&format!("{{\"key\":\"{}\"}}", crate::json_escape(&shown(key))));
                continue;
            }
            out.push_str(&format!(
                "{{\"key\":\"{}\",\"count\":{count}",
                crate::json_escape(&shown(key))
            ));
            for (h, cell) in headings.iter().zip(cells.get(i).map_or(&[][..], Vec::as_slice)) {
                // A cell is a rendered value, so a number arrives as one and
                // an absent aggregate as null rather than as the dash the
                // table prints.
                let member = crate::json_escape(h);
                if cell == "-" {
                    out.push_str(&format!(",\"{member}\":null"));
                } else if cell.starts_with(['{', '"']) || cell.parse::<f64>().is_ok() {
                    out.push_str(&format!(",\"{member}\":{cell}"));
                } else {
                    out.push_str(&format!(",\"{member}\":\"{}\"", crate::json_escape(cell)));
                }
            }
            out.push('}');
        }
        out.push(']');
        println!("{out}");
        return;
    }
    if which == Aggregate::Uniq {
        for (key, _) in rows {
            println!("{}", shown(key));
        }
        return;
    }
    let width = rows.iter().map(|(k, _)| shown(k).chars().count()).max().unwrap_or(3).max(3);
    // Each aggregate column is as wide as its heading or its widest cell, so
    // the columns line up under names a reader can tell apart.
    let widths: Vec<usize> = headings
        .iter()
        .enumerate()
        .map(|(c, h)| {
            cells
                .iter()
                .filter_map(|row| row.get(c))
                .map(|v| v.chars().count())
                .chain(std::iter::once(h.chars().count()))
                .max()
                .unwrap_or(1)
        })
        .collect();
    if !headings.is_empty() {
        let mut head = format!("{:<width$}  {:>5}", "", "count", width = width);
        for (h, w) in headings.iter().zip(&widths) {
            head.push_str(&format!("  {h:>w$}", w = *w));
        }
        println!("{head}");
    }
    for (i, (key, count)) in rows.iter().enumerate() {
        if headings.is_empty() {
            println!("{:<width$}  {count}", shown(key), width = width);
            continue;
        }
        let mut line = format!("{:<width$}  {count:>5}", shown(key), width = width);
        for (cell, w) in cells.get(i).map_or(&[][..], Vec::as_slice).iter().zip(&widths) {
            line.push_str(&format!("  {cell:>w$}", w = *w));
        }
        println!("{line}");
    }
    if rows.len() < keys {
        println!("{keys} keys over {matched} matches; {} shown", rows.len());
    }
}

/// Run `work` on every slot across the cores, in place.
pub(crate) fn across_cores<T: Send>(slots: &mut [T], work: impl Fn(usize, &mut T) + Sync) {
    if slots.is_empty() {
        return;
    }
    let plan = flynnel::JobPlan::new(0, slots.len() as u32)
        .with_leaf_shape(flynnel::LeafShape::PortCompute);
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, slots, 1, |start, part| {
        for (k, slot) in part.iter_mut().enumerate() {
            work(start + k, slot);
        }
    });
}

/// The grep flags a scan takes beside its own, as ripgrep and ugrep spell
/// them.
#[derive(Default)]
struct GrepFlags {
    /// `-v`: the lines no match touches are the report.
    invert: bool,
    /// `-x`: a match counts only where it covers its line's significant
    /// extent, from the first non-whitespace byte to just past the last.
    whole_line: bool,
    /// `-m N`: at most this many matches, or unmatched lines, per input.
    max_count: Option<usize>,
    /// `--passthru`: every line of each input, the matched ones painted.
    passthru: bool,
    /// `-L`: the inputs with no match.
    files_without_match: bool,
    /// `--files`: the files the walk finds, and no scan.
    files_only: bool,
    /// `--texture KIND`, repeatable, each with whether it keeps or drops: the
    /// region kinds a file the walk found must read as. A file named on the
    /// command line is read whatever it says, so this filters what the walk
    /// turned up and never what was asked for by name.
    textures: Vec<(String, bool)>,
    /// `--texture` with no kind after it: name what each file reads as rather
    /// than filter by it.
    texture_listing: bool,
    /// `--type-list`: the file types the walk knows.
    type_list: bool,
    /// `--stats`, in the form asked for.
    stats: Option<StatsForm>,
    /// `--color WHEN`.
    color: Option<String>,
    /// `--colors SPEC`, in order.
    colors: Vec<String>,
    /// `-e PATTERN`, in order.
    patterns: Vec<String>,
    /// `-f FILE`, in order.
    pattern_files: Vec<String>,
    /// `--all`, `--any`, `--none` or `--at-least N`: the quantifier of a
    /// record-level query.
    rule: Option<Quantifier>,
    /// `--not PATTERN`, in order: what a record must not hold.
    nots: Vec<String>,
    /// `--record UNIT`, `--record-start PATTERN` or `--record-span PATTERN`.
    record: Option<RecordSpec>,
    /// Whether a context flag wrote `record`, which is the one case where
    /// the flags above say what a record IS without asking for a record
    /// query: the report is still one line per match, with the record the
    /// match sits in printed around it.
    record_is_the_context: bool,
    /// `--patterns FILE`: the members of a set, from a pattern file.
    patterns_file: Option<String>,
    /// `--single-match`: each member's first match per input and no more.
    single_match: bool,
    /// `--rules FILE|DIR`, in order: the pattern files whose rules are
    /// scanned, a directory for every `.trex` file under it.
    rules: Vec<String>,
    /// `--index`: build the tree's index as the scan reads it, and write it
    /// when the scan is done, so every later scan of that tree prunes with
    /// it.
    index: bool,
    /// `--no-index`: read no index, whatever the tree holds.
    no_index: bool,
    /// `--sarif`: the findings as a SARIF document.
    sarif: bool,
    /// `--github`: the findings as GitHub workflow annotations.
    github: bool,
    /// `--fix`: the rules' fixes applied in place.
    fix: bool,
    /// `--dry-run`: the fixes as unified diffs, nothing written.
    dry_run: bool,
    /// `--interactive`: each fix reviewed before it is written.
    interactive: bool,
    /// `--head N`, `--tail N`, `--lines A..B` and `--follow`: the part of
    /// each input scanned, and whether each file is followed as it grows.
    windowing: Windowing,
}

impl GrepFlags {
    /// Whether the scan is a record-level query: a rule, a `--not` or a
    /// record definition was given.
    fn queries(&self) -> bool {
        self.rule.is_some()
            || !self.nots.is_empty()
            || (self.record.is_some() && !self.record_is_the_context)
    }
}

/// The flag that names a record query's quantifier.
fn quantifier_flag(q: Quantifier) -> String {
    match q {
        Quantifier::All => "--all".to_string(),
        Quantifier::Any => "--any".to_string(),
        Quantifier::None => "--none".to_string(),
        Quantifier::AtLeast(n) => format!("--at-least {n}"),
    }
}

/// How a record-level query's records are defined, as written.
pub(crate) enum RecordSpec {
    /// `--record UNIT`.
    Named(String),
    /// `--record-start PATTERN`.
    Start(String),
    /// `--record-span PATTERN`.
    Span(String),
}

/// What a record is under the record flags given, their patterns read under
/// `shapes`: a line where none names one.
pub(crate) fn record_unit_of(
    record: Option<&RecordSpec>,
    shapes: &trex::ShapeSet,
) -> Result<trex::records::RecordUnit, String> {
    use trex::records::RecordUnit;
    let parse = |flag: &str, src: &str| {
        trex::parser::parse_with_shapes(src, shapes)
            .map_err(|e| format!("{flag}: pattern error in {src:?} at byte {}: {}", e.pos, e.msg))
    };
    match record {
        None => Ok(RecordUnit::Line),
        Some(RecordSpec::Named(name)) => RecordUnit::parse(name).map_err(|e| format!("--record: {e}")),
        Some(RecordSpec::Start(src)) => parse("--record-start", src).map(RecordUnit::Start),
        Some(RecordSpec::Span(src)) => parse("--record-span", src).map(RecordUnit::Span),
    }
}

/// The lines a record covers, as zero-based indices, clamped to the input.
fn record_lines(index: &LineIndex, hit: &RecordHit) -> (usize, usize) {
    let lines = index.lines();
    if lines == 0 {
        return (0, 0);
    }
    let first = index.line_of(hit.start).min(lines - 1);
    let last = index.line_of(hit.end.saturating_sub(1).max(hit.start)).min(lines - 1);
    (first, last)
}

/// One qualifying record as a JSON object: where it stands, its text and
/// the patterns present in it, by name where they are the members of a set
/// and by index otherwise.
fn record_json(path: Option<&str>, input: &[u8], index: &LineIndex, hit: &RecordHit, names: &[String]) -> String {
    let (first, _) = record_lines(index, hit);
    let mut out = String::from("{");
    if let Some(path) = path {
        out.push_str(&format!("\"path\":\"{}\",", crate::json_escape(path)));
    }
    let patterns: Vec<String> = hit
        .present
        .iter()
        .map(|&i| match names.get(i) {
            Some(name) => format!("\"{}\"", crate::json_escape(name)),
            None => i.to_string(),
        })
        .collect();
    out.push_str(&format!(
        "\"line\":{},\"start\":{},\"end\":{},\"text\":\"{}\",\"patterns\":[{}]}}",
        index.number(first),
        index.offset(hit.start),
        index.offset(hit.end),
        crate::json_escape(&String::from_utf8_lossy(&input[hit.start..hit.end])),
        patterns.join(",")
    ));
    out
}

/// Print the qualifying records of one input as the lines they cover, each
/// as `path:line:text` where the report names its inputs and bare where it
/// does not, the matches of the present patterns painted, no line printed
/// twice, and `--` between records that do not touch.
fn print_records(
    prefix: Option<&str>,
    input: &[u8],
    index: &LineIndex,
    hits: &[RecordHit],
    painter: &trex::paint::Painter,
) {
    let mut painted: Vec<trex::Match> =
        hits.iter().flat_map(|h| h.spans.iter().map(|&(s, e)| trex::Match::plain(s, e))).collect();
    painted.sort_by_key(|m| (m.start, m.end));
    // As in the lines report: lexed here because a record query answers from
    // the records rather than from a lex, and only a report that paints the
    // kinds pays for one.
    let tokens = if painter.paints_kinds() { trex::lexer::lex(input) } else { Vec::new() };
    let mut last_printed: Option<usize> = None;
    for hit in hits {
        let (first, last) = record_lines(index, hit);
        if let Some(done) = last_printed
            && first > done + 1
        {
            crate::out::line("--");
        }
        for line in first..=last {
            if last_printed.is_some_and(|done| line <= done) {
                continue;
            }
            let (s, e) = index.line_span(line);
            let here: Vec<trex::Match> = painted
                .iter()
                .filter(|m| m.start < e && (m.end > s || m.start >= s))
                .cloned()
                .collect();
            print_line(prefix, input, index, line, true, &here, painter, &tokens);
            last_printed = Some(line);
        }
    }
}

/// How `--stats` prints.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StatsForm {
    /// ripgrep's eight lines.
    Lines,
    /// One line.
    Line,
}

/// What a scan counts for `--stats`.
#[derive(Default)]
struct Stats {
    matches: u64,
    matched_lines: u64,
    files_with_matches: u64,
    files_searched: u64,
    bytes_searched: u64,
    searching: std::time::Duration,
    /// The tokens every lex produced, from the trace.
    tokens: u64,
    /// The time those lexes took, which comes out of `searching` rather than
    /// adding to it: a scan that spends most of its time lexing and one that
    /// spends it matching read the same on the eight lines above and differ
    /// only here.
    ///
    /// Summed over the lexes, and a parallel lex runs one per chunk at once,
    /// so on a large input this is the time the cores spent between them and
    /// not the time the clock on the wall spent. That is why the line that
    /// reports what is left for matching takes the difference rather than
    /// being measured on its own: only one of the two can be a wall-clock
    /// reading, and the searching time above already is.
    lexing: std::time::Duration,
    /// How many inputs each scan rung answered, in the order first seen.
    routes: Vec<(String, u64)>,
    /// The bytes handed to the device, which is the one rung whose cost is
    /// not the host's to spend.
    device_bytes: u64,
}

impl Stats {
    /// Count one scanned input: its size, its matches and the lines they
    /// touch.
    fn count(&mut self, input: &[u8], matches: &[trex::Match], index: &LineIndex) {
        self.files_searched += 1;
        self.bytes_searched += input.len() as u64;
        self.matches += matches.len() as u64;
        self.matched_lines += touched_lines(matches, index).iter().filter(|&&t| t).count() as u64;
        if !matches.is_empty() {
            self.files_with_matches += 1;
        }
    }

    /// Count one queried input: its size, the records that qualify and the
    /// lines they cover.
    fn count_records(&mut self, input: &[u8], hits: &[RecordHit], index: &LineIndex) {
        self.files_searched += 1;
        self.bytes_searched += input.len() as u64;
        self.matches += hits.len() as u64;
        let mut covered = vec![false; index.lines()];
        for hit in hits {
            let (first, last) = record_lines(index, hit);
            for slot in covered.iter_mut().take(last + 1).skip(first) {
                *slot = true;
            }
        }
        self.matched_lines += covered.iter().filter(|&&c| c).count() as u64;
        if !hits.is_empty() {
            self.files_with_matches += 1;
        }
    }

    /// Print the statistics after the report: ripgrep's eight lines, or one.
    /// The bytes printed are the report's, counted before these lines.
    /// Take what the trace kept since the last take: the rungs that answered
    /// and what the lexes cost.
    ///
    /// Called once per report rather than per input, because the trace is a
    /// process-wide record and taking it per input would race the inputs
    /// scanned across cores.
    fn read_the_trace(&mut self) {
        let (tokens, lexing) = trex::trace::take_lexing();
        self.tokens += tokens;
        self.lexing += lexing;
        // Read from the totals rather than the queue of rungs, which
        // `--explain` takes to name the route of the input it is explaining.
        // Reading the queue here would report no routes whenever both flags
        // are given, and the one that took it first would decide which.
        for total in trex::trace::take_totals() {
            if total.ladder != "scan" {
                continue;
            }
            if total.rung.contains("device") {
                self.device_bytes += total.bytes;
            }
            self.routes.push((total.rung, total.calls));
        }
    }

    fn print(&self, form: StatsForm, elapsed: std::time::Duration) {
        let printed = crate::out::printed();
        match form {
            StatsForm::Lines => {
                crate::out::line("");
                crate::out::line(&format!("{} matches", self.matches));
                crate::out::line(&format!("{} matched lines", self.matched_lines));
                crate::out::line(&format!("{} files contained matches", self.files_with_matches));
                crate::out::line(&format!("{} files searched", self.files_searched));
                crate::out::line(&format!("{printed} bytes printed"));
                crate::out::line(&format!("{} bytes searched", self.bytes_searched));
                crate::out::line(&format!("{:.6} seconds spent searching", self.searching.as_secs_f64()));
                crate::out::line(&format!("{:.6} seconds", elapsed.as_secs_f64()));
                // Everything below is why the eight lines above came out as
                // they did: which rungs answered, and how the searching time
                // split between reading the bytes into tokens and matching
                // over them.
                crate::out::line(&format!("{} tokens lexed", self.tokens));
                crate::out::line(&format!("{:.6} seconds spent lexing", self.lexing.as_secs_f64()));
                crate::out::line(&format!(
                    "{:.6} seconds spent matching",
                    self.searching.saturating_sub(self.lexing).as_secs_f64()
                ));
                if self.device_bytes > 0 {
                    crate::out::line(&format!("{} bytes the device scanned", self.device_bytes));
                }
                for (rung, files) in &self.routes {
                    crate::out::line(&format!("{files} files answered by {rung}"));
                }
            }
            StatsForm::Line => crate::out::line(&format!(
                "{} matches in {} of {} files, {} bytes searched, {:.3} ms",
                self.matches,
                self.files_with_matches,
                self.files_searched,
                self.bytes_searched,
                elapsed.as_secs_f64() * 1000.0
            )),
        }
    }
}

/// The value a flag takes: the rest of `--flag=value`, or the next argument.
fn flag_value(
    args: &[String],
    i: &mut usize,
    attached: Option<&str>,
    flag: &str,
    what: &str,
) -> Result<String, String> {
    if let Some(v) = attached {
        return Ok(v.to_string());
    }
    *i += 1;
    args.get(*i).cloned().ok_or_else(|| format!("{flag} needs {what}"))
}

/// Which lines of an input a match touches.
fn touched_lines(matches: &[trex::Match], index: &LineIndex) -> Vec<bool> {
    let mut touched = vec![false; index.lines()];
    for m in matches {
        let first = index.line_of(m.start);
        let last = index.line_of(m.end.saturating_sub(1).max(m.start));
        for line in first..=last {
            if let Some(slot) = touched.get_mut(line) {
                *slot = true;
            }
        }
    }
    touched
}

/// How many records of `unit` hold a match, or under `invert` how many hold
/// none.
///
/// What `--count` answers. A line is the default record, so with no record
/// flag this is `grep -c`, and with one it is the same question over
/// paragraphs, periods, seams, texture regions or a declared span - a line
/// being one record among others rather than a case of its own.
///
/// Both sequences ascend, so one walk settles every record. A match wider
/// than a record is left in place rather than consumed, because it holds the
/// records that follow it too. The count stops at `cap`, which `-m` sets, as
/// grep's does: `-vm 1 -c` answers at most one, the line `-vm 1` prints.
fn records_touched(
    unit: &trex::records::RecordUnit,
    input: &[u8],
    matches: &[trex::Match],
    invert: bool,
    cap: Option<usize>,
) -> usize {
    let mut held = 0usize;
    let mut at = 0usize;
    for (start, end) in unit.records(input) {
        if cap.is_some_and(|n| held >= n) {
            break;
        }
        while at < matches.len() && matches[at].end <= start {
            at += 1;
        }
        let touched = at < matches.len() && matches[at].start < end;
        if touched != invert {
            held += 1;
        }
    }
    held
}

/// A line's significant extent, as offsets within it: from its first
/// non-whitespace byte to just past its last, and empty at its end where it
/// holds nothing else.
fn significant_span(line: &[u8]) -> (usize, usize) {
    match std::str::from_utf8(line) {
        Ok(text) => {
            let start = text.len() - text.trim_start().len();
            (start, start + text.trim().len())
        }
        Err(_not_utf8) => {
            let start = line.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(line.len());
            let end = line.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(start, |p| p + 1);
            (start, end)
        }
    }
}

/// The matches that cover their line's significant extent, what `-x` keeps,
/// with the members beside them where the scan ran a set.
pub(crate) fn whole_line_matches(
    input: &[u8],
    matches: Vec<trex::Match>,
    members: Vec<usize>,
    index: &LineIndex,
) -> (Vec<trex::Match>, Vec<usize>) {
    let keep: Vec<bool> = matches
        .iter()
        .map(|m| {
            let (s, e) = index.line_span(index.line_of(m.start));
            let (from, to) = significant_span(&input[s..e]);
            m.start == s + from && m.end == s + to
        })
        .collect();
    let matches = matches.into_iter().zip(&keep).filter(|(_, k)| **k).map(|(m, _)| m).collect();
    let members = members.into_iter().zip(&keep).filter(|(_, k)| **k).map(|(i, _)| i).collect();
    (matches, members)
}

/// Whether the argument after `--texture` at `i` names a region kind, with or
/// without the `!` that drops instead of keeping.
fn next_is_a_texture(args: &[String], i: usize) -> bool {
    args.get(i + 1).is_some_and(|next| {
        let name = next.strip_prefix('!').unwrap_or(next);
        trex::shape::RegionKind::NAMES.contains(&name)
    })
}

/// The kind a file reads as, or `None` for one that cannot be read or holds
/// no region.
///
/// The pass runs once for the file and the field is not rebuilt: a file is
/// classified here or not at all, and a scan that follows reads its own.
fn texture_of(src: &Source) -> Option<trex::shape::RegionKind> {
    let raw = match read_source(src) {
        Ok(raw) => raw,
        // A file the walk found and the read cannot open says nothing about
        // its texture, so it is not kept by a filter that names one. The scan
        // that follows reports the read error itself.
        Err(_unreadable) => return None,
    };
    trex::shape::dominant_kind(&trex::encoding::decode(raw))
}

/// The kind as `--files --texture` prints it: its name, and for a table the
/// period that names the table it found.
fn spelled(kind: trex::shape::RegionKind) -> String {
    match kind {
        trex::shape::RegionKind::Table(period) => format!("table, period {period}"),
        other => other.label().to_string(),
    }
}

/// The painter `--color` and `--colors` ask for: the depth the console
/// renders under `auto` or nothing, none under `never`, what the environment
/// says under `always`, or the depth named; the base palette with the specs
/// applied in order.
pub(crate) fn painter_for(color: Option<&str>, specs: &[String]) -> Result<trex::paint::Painter, String> {
    use trex::paint::{Depth, Painter, Palette};
    let depth = match color {
        None | Some("auto") => Depth::detect(),
        Some("never") => Depth::Off,
        Some("always") => Depth::forced(),
        Some(named) => {
            let depth = Depth::parse(named).ok_or_else(|| {
                format!("--color takes always, never, auto, 16, 256 or truecolor, not {named:?}")
            })?;
            trex::paint::console_ready();
            depth
        }
    };
    let mut palette = Palette::base();
    // The environment is read before the specs, so a `--colors kind:*` on the
    // command line still wins over a level set once in a shell profile.
    match std::env::var(KIND_COLOR_ENV) {
        Ok(level) => {
            let level = trex::paint::KindPaint::parse(&level).ok_or_else(|| {
                format!("{KIND_COLOR_ENV} takes none, values or all, not {level:?}")
            })?;
            palette.set_kind_paint(level);
        }
        Err(std::env::VarError::NotPresent) => {}
        Err(std::env::VarError::NotUnicode(raw)) => {
            return Err(format!("{KIND_COLOR_ENV} is set to something that is not text: {raw:?}"));
        }
    }
    for spec in specs {
        palette.set(spec).map_err(|e| format!("--colors: {e}"))?;
    }
    Ok(Painter::new(depth, palette))
}

/// The variable that sets how much of a line the token kinds paint, for a
/// reader who wants one level every time and does not want to write it on
/// every command. `--colors kind:*:LEVEL` says the same thing for one run.
const KIND_COLOR_ENV: &str = "TREX_KIND_COLOR";

/// The construct the context flags print around a match, in place of a count
/// of lines: the whole record of `unit` the match sits in, clipped to the
/// sides the flags asked for.
///
/// `-C` speaks for both sides, `-B` for the lines ahead of the match and
/// `-A` for those after, so `-B unit` shows the construct up to the match and
/// `-C unit` shows all of it. The match's own line is printed either way,
/// which is what makes `-A` and `-B` narrower than `-C` rather than empty.
struct Around {
    unit: trex::records::RecordUnit,
    before: bool,
    after: bool,
}

/// What a report prints for every input, beyond what the scan itself takes.
struct Report<'a> {
    json: bool,
    /// `--count`: how many records hold a match, which are lines until
    /// `--record` names another unit. The spelling `grep -c` and `rg
    /// --count` both use.
    count: bool,
    /// `--count-matches`: how many matches there are, which is a different
    /// number wherever two matches share a record.
    count_matches: bool,
    /// What `--count` counts one of, which `--record` names and which is a
    /// line where it named nothing.
    record_unit: &'a trex::records::RecordUnit,
    files_with_matches: bool,
    before: usize,
    after: usize,
    /// The construct printed instead of a count of lines, where the context
    /// flags named one.
    around: Option<&'a Around>,
    template: Option<&'a trex::Template>,
    /// Whether `--explain` was asked for, which is what prints the
    /// explanation under each match. An explainer may exist without it, for
    /// a template that names an axis.
    explain: bool,
    grep: &'a GrepFlags,
    painter: &'a trex::paint::Painter,
    /// What a typed register's value is read and spelled as, absent for a
    /// report that prints no values.
    values: Option<&'a crate::ValueView<'a>>,
}

/// One input's report with no path ahead of it: the span and its text per
/// match, or what the flags ask for instead. The hits: the matches, or under
/// `-v` the lines none touches.
fn report_unprefixed(
    name: &str,
    input: &[u8],
    matches: &[trex::Match],
    index: &LineIndex,
    report: &Report<'_>,
    members: Option<Members<'_>>,
    explaining: Option<&Explaining<'_>>,
) -> usize {
    use trex::paint::Role;
    let grep = report.grep;
    let painter = report.painter;
    // Records holding a match, under whatever `--record` said a record is
    // and lines where it said nothing. Under `-v` it is the records none
    // touches, which is what `grep -vc` answers. Walked only where a report
    // reads it, because it builds the input's records.
    let records_hit = if report.count || grep.invert {
        records_touched(report.record_unit, input, matches, grep.invert, grep.max_count)
    } else {
        0
    };
    let hits = if grep.invert { records_hit } else { matches.len() };
    if report.count {
        // `--count` answers records, as `grep -c`, `rg --count` and `ugrep
        // -c` do. `--count-matches` answers matches, as `rg
        // --count-matches` does.
        crate::out::line(&records_hit.to_string());
    } else if report.count_matches {
        crate::out::line(&hits.to_string());
    } else if report.files_with_matches {
        if hits > 0 {
            crate::out::line(&painter.paint(Role::Path, name));
        }
    } else if grep.files_without_match {
        if hits == 0 {
            crate::out::line(&painter.paint(Role::Path, name));
        }
    } else if grep.invert || grep.passthru {
        print_lines_report(None, input, matches, index, grep, painter);
    } else if let Some(t) = report.template {
        print_formatted(name, input, matches, index, t, members, explaining, report.explain);
    } else if report.json {
        print_json_about(input, matches, index, members, explaining, report.values);
    } else if report.before > 0 || report.after > 0 || report.around.is_some() {
        print_with_context(None, input, matches, index, report, members, explaining);
    } else {
        print_human_about(input, matches, index, painter, members, explaining);
    }
    hits
}

/// One input's record report with no path ahead of it: the count, the
/// JSON objects, or the records' lines.
fn report_records_unprefixed(
    input: &[u8],
    index: &LineIndex,
    hits: &[RecordHit],
    report: &Report<'_>,
    names: &[String],
) {
    if report.count {
        crate::out::line(&hits.len().to_string());
    } else if report.json {
        let objects: Vec<String> = hits.iter().map(|h| record_json(None, input, index, h, names)).collect();
        crate::out::line(&format!("[{}]", objects.join(",")));
    } else if hits.is_empty() {
        crate::out::line("no match");
    } else {
        print_records(None, input, index, hits, report.painter);
    }
}

/// Print one line as the `-v` and `--passthru` reports write it, as
/// [`trex::report::painted_line`] spells it.
// The eight are one line and everything needed to paint it: where it came
// from, the input and its line index, which line, whether the report selected
// it or is carrying it along, the matches in it, the painter, and the tokens.
#[allow(clippy::too_many_arguments)]
fn print_line(
    prefix: Option<&str>,
    input: &[u8],
    index: &LineIndex,
    line: usize,
    selected: bool,
    matches: &[trex::Match],
    painter: &trex::paint::Painter,
    tokens: &[trex::token::Token],
) {
    crate::out::line(&trex::report::painted_line(prefix, input, index, line, selected, matches, painter, tokens));
}

/// The lines report: under `-v` the lines no match touches, each selected;
/// under `--passthru` every line, the touched ones selected with their
/// matches painted and the rest carried along. `-m` caps the selected lines.
/// As [`trex::report::lines_report`] writes it.
fn print_lines_report(
    prefix: Option<&str>,
    input: &[u8],
    matches: &[trex::Match],
    index: &LineIndex,
    grep: &GrepFlags,
    painter: &trex::paint::Painter,
) {
    trex::report::lines_report(prefix, input, matches, index, grep.invert, grep.max_count, painter, &mut |l| {
        crate::out::line(l)
    });
}

/// How `scan --fields` reads its inputs.
struct FieldsScan<'a> {
    pattern: &'a trex::ast::Pattern,
    /// The pattern as written: `\{name}` takes the fields a `fields` line
    /// gives `name`.
    source: &'a str,
    shapes: &'a trex::ShapeSet,
    paths: &'a [String],
    text: Option<Vec<u8>>,
    walk: &'a WalkOptions,
    windowing: &'a Windowing,
    unit: &'a trex::records::RecordUnit,
    binary: bool,
    json: bool,
    require_match: bool,
}

/// `scan --fields`: the records the pattern's fields read from each input,
/// as `infer` reports the records of the lines it built from, the fields as
/// [`trex::infer::build::fields_for`] chooses them. The table numbers the
/// records from one and gives the lines each stands on in its input, and
/// the path of each where there is more than one input; `--json` writes
/// them as one array, each with its path where it was read from one.
fn scan_fields(how: FieldsScan<'_>) -> ExitCode {
    let fields = trex::infer::build::fields_for(how.source, how.pattern, how.shapes);
    let asked = trex::window::Asked { numbers: true, offsets: false, binary: how.binary, units: false };
    let mut parts: Vec<(Option<String>, Part)> = Vec::new();
    let mut failed = false;
    match (&how.text, how.paths.is_empty()) {
        (Some(bytes), _) => parts.push((None, Part::of_text(bytes.clone(), how.windowing.select, how.unit))),
        (None, true) => match read_part(&Source::Stdin, how.windowing.select, how.unit, asked) {
            Ok(part) => parts.push((Some("-".to_string()), part)),
            Err(e) => {
                eprintln!("trex: -: {e}");
                failed = true;
            }
        },
        (None, false) => {
            let (sources, errors) = trex::files::collect(how.paths, how.walk);
            for e in &errors {
                eprintln!("trex: {e}");
                failed = true;
            }
            for src in &sources {
                match read_part(src, how.windowing.select, how.unit, asked) {
                    Ok(part) => parts.push((Some(src.name()), part)),
                    Err(e) => {
                        eprintln!("trex: {}: {e}", src.name());
                        failed = true;
                    }
                }
            }
        }
    }
    let mut records: Vec<(Option<String>, trex::infer::build::Record)> = Vec::new();
    for (path, part) in &parts {
        if part.binary && !how.binary {
            continue;
        }
        let text = String::from_utf8_lossy(&part.text);
        match trex::infer::build::read_records(&fields, how.pattern, how.shapes, &text) {
            Ok(read) => {
                let base = part.line_base.expect("--fields asks for line numbers, so the window counts them");
                for mut record in read {
                    for line in &mut record.lines {
                        *line += base;
                    }
                    records.push((path.clone(), record));
                }
            }
            Err(e) => {
                match path {
                    Some(p) => eprintln!("trex: {p}: {e}"),
                    None => eprintln!("trex: --text: {e}"),
                }
                failed = true;
            }
        }
    }
    if how.json {
        let objects: Vec<String> = records
            .iter()
            .map(|(path, record)| {
                let lines: Vec<String> = record.lines.iter().map(|l| (l + 1).to_string()).collect();
                let values = json_members(&fields, &record.values, None);
                let path = match path {
                    Some(p) => format!("\"path\":\"{}\",", crate::json_escape(p)),
                    None => String::new(),
                };
                format!("{{{path}\"lines\":[{}],\"values\":{{{}}}}}", lines.join(","), values.join(","))
            })
            .collect();
        println!("[{}]", objects.join(","));
    } else {
        let several = parts.len() > 1;
        let mut table: Vec<Vec<String>> = vec![
            several
                .then(|| "path".to_string())
                .into_iter()
                .chain(["record", "lines"].into_iter().map(str::to_string))
                .chain(fields.iter().map(|f| f.name.clone()))
                .collect(),
        ];
        for (i, (path, record)) in records.iter().enumerate() {
            let values = record.values.iter().map(|v| match v {
                Some(v) => v.joined().replace('\n', "\\n"),
                None => "-".to_string(),
            });
            let named = several.then(|| path.clone().expect("several inputs are each read from a path"));
            table.push(named.into_iter().chain([(i + 1).to_string(), line_runs(&record.lines)]).chain(values).collect());
        }
        print!("{}", aligned_table(&table));
    }
    if failed || (how.require_match && records.is_empty()) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

pub fn run_scan(args: &[String]) -> ExitCode {
    let started = std::time::Instant::now();
    if args.is_empty() {
        eprintln!(
            "usage: trex scan PATTERN [FILE|DIR|-]... [--text STRING] [--lib FILE] [--json] [--format TEMPLATE] [--fields] [--require-match] [-A N] [-B N] [-C N] [--count] [-l] [-L] [-H] [--hidden] [--no-ignore] [--binary] [--explain] [-v] [-x] [-o] [-m N] [-e PATTERN]... [-f FILE]... [--patterns FILE] [--single-match] [--rules FILE|DIR]... [--sarif] [--github] [--fix] [--index] [--no-index] [-g GLOB]... [-t TYPE]... [-T TYPE]... [--type-list] [--files] [--sort KEY] [--sortr KEY] [--color WHEN] [--colors SPEC]... [--stats[=line]] [--passthru] [--head N|--tail N|--lines A..B] [--follow]"
        );
        return ExitCode::FAILURE;
    }
    let mut positionals: Vec<String> = Vec::new();
    let mut text: Option<Vec<u8>> = None;
    let mut json = false;
    let mut require_match = false;
    let mut chunk_size: Option<usize> = None;
    let mut dual_grain = false;
    let mut backend = trex::Backend::Auto;
    let mut shapes = trex::ShapeSet::new();
    let mut before = 0usize;
    let mut after = 0usize;
    // The construct `-A`, `-B` and `-C` print instead of a count of lines,
    // held as the word written until the record flags have been read, since
    // `record` names whatever those defined.
    let mut around: Option<(String, bool, bool)> = None;
    let mut count = false;
    let mut count_matches = false;
    let mut files_with_matches = false;
    let mut with_filename: Option<bool> = None;
    let mut walk = WalkOptions::default();
    let mut binary = false;
    let mut explain = false;
    let mut fields_table = false;
    let mut format: Option<String> = None;
    // Exact by default: the typed parsers hold a value exactly and a JSON
    // number is a double, so a rounded value could disagree with the very
    // predicate that selected the match with nothing in the output saying so.
    // A duration defaults to the nanoseconds it is held and compared in.
    let mut style = trex::typed::ValueStyle::new();
    let mut grep = GrepFlags::default();

    let mut i = 0;
    while i < args.len() {
        match grep.windowing.take(args, &mut i) {
            Some(Ok(())) => {
                i += 1;
                continue;
            }
            Some(Err(e)) => {
                eprintln!("trex: {e}");
                return ExitCode::FAILURE;
            }
            None => {}
        }
        let arg = args[i].as_str();
        // The grep flags take their value after `=` as well as as the next
        // argument, as ripgrep spells them.
        let (flag, attached) = match arg.split_once('=') {
            Some((f, v))
                if matches!(
                    f,
                    "--color"
                        | "--colors"
                        | "--glob"
                        | "--type"
                        | "--type-not"
                        | "--regexp"
                        | "--pattern"
                        | "--file"
                        | "--max-count"
                        | "--sort"
                        | "--sortr"
                        | "--stats"
                        | "--at-least"
                        | "--not"
                        | "--record"
                        | "--record-start"
                        | "--record-span"
                        | "--patterns"
                        | "--rules"
                ) =>
            {
                (f, Some(v))
            }
            _ => (arg, None),
        };
        macro_rules! value {
            ($what:expr) => {
                match flag_value(args, &mut i, attached, flag, $what) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("trex: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            };
        }
        match flag {
            "--shape" | "--shape-after" => {
                let when = if args[i] == "--shape" {
                    trex::Precedence::Before
                } else {
                    trex::Precedence::After
                };
                i += 1;
                let Some(decl) = args.get(i) else {
                    eprintln!("trex: {} needs a `name = `pattern`` declaration", args[i - 1]);
                    return ExitCode::FAILURE;
                };
                if let Err(e) = shapes.declare(decl, when) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--lib" => {
                i += 1;
                let Some(path) = args.get(i) else {
                    eprintln!("trex: --lib needs a pattern file");
                    return ExitCode::FAILURE;
                };
                if let Err(e) = declare_file(&mut shapes, path) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--json" => json = true,
            "--require-match" => require_match = true,
            "--dual-grain" => dual_grain = true,
            "--gpu" => backend = trex::Backend::Gpu,
            "--cpu" | "--nogpu" => backend = trex::Backend::Cpu,
            "--count" => count = true,
            "--count-matches" => count_matches = true,
            "-l" | "--files-with-matches" => files_with_matches = true,
            "-H" | "--with-filename" => with_filename = Some(true),
            "--no-filename" => with_filename = Some(false),
            "--hidden" => walk.hidden = true,
            "--no-ignore" => walk.no_ignore = true,
            "--binary" => binary = true,
            "--explain" => explain = true,
            "--fields" => fields_table = true,
            "--format" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --format needs a template, as in '${{path}}:${{line}}: ${{0}}'");
                    return ExitCode::FAILURE;
                };
                format = Some(v.clone());
            }
            "--duration-unit" => {
                i += 1;
                match args.get(i).map(String::as_str) {
                    Some("ns") => style.duration = trex::typed::DurationUnit::Nanoseconds,
                    Some("ms") => style.duration = trex::typed::DurationUnit::Milliseconds,
                    Some("s") => style.duration = trex::typed::DurationUnit::Seconds,
                    other => {
                        let given = other.unwrap_or("nothing");
                        eprintln!(
                            "trex: --duration-unit takes ns, ms or s, not {given}. \
                             A duration is held and compared as nanoseconds; ms and s \
                             shift it exactly for a reader."
                        );
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--values" => {
                i += 1;
                match args.get(i).map(String::as_str) {
                    Some("exact") => style.spelling = trex::typed::ValueSpelling::Exact,
                    Some("natural") => style.spelling = trex::typed::ValueSpelling::Natural,
                    Some("tagged") => style.spelling = trex::typed::ValueSpelling::Tagged,
                    other => {
                        // The word is named back, because a typo here changes
                        // what the numbers mean rather than failing to run.
                        let given = other.unwrap_or("nothing");
                        eprintln!(
                            "trex: --values takes exact, natural or tagged, not {given}. \
                             exact keeps a value a double cannot hold as a string; \
                             natural rounds it into a JSON number; \
                             tagged writes every value as its kind and its exact text."
                        );
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--chunk-size" => {
                i += 1;
                match count_arg("--chunk-size", args.get(i)) {
                    Ok(v) if v > 0 => chunk_size = Some(v),
                    Ok(_) => {
                        eprintln!("trex: --chunk-size needs a positive integer");
                        return ExitCode::FAILURE;
                    }
                    Err(e) => {
                        eprintln!("trex: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "-A" | "--after-context" | "-B" | "--before-context" | "-C" | "--context" => {
                let flag = args[i].clone();
                i += 1;
                let Some(value) = args.get(i).cloned() else {
                    eprintln!("trex: {flag} needs a number or a record unit");
                    return ExitCode::FAILURE;
                };
                // Which side of the match the flag speaks for: `-B` the lines
                // ahead of it, `-A` those after, `-C` both.
                let (says_before, says_after) = match flag.as_str() {
                    "-A" | "--after-context" => (false, true),
                    "-B" | "--before-context" => (true, false),
                    _ => (true, true),
                };
                if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
                    let n = match count_arg(&flag, Some(&value)) {
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("trex: {e}");
                            return ExitCode::FAILURE;
                        }
                    };
                    if says_before {
                        before = n;
                    }
                    if says_after {
                        after = n;
                    }
                } else {
                    match &mut around {
                        Some((named, b, a)) if *named == value => {
                            *b |= says_before;
                            *a |= says_after;
                        }
                        Some((named, _, _)) => {
                            eprintln!(
                                "trex: the context flags name one unit; {named:?} and {value:?} were both given"
                            );
                            return ExitCode::FAILURE;
                        }
                        none => *none = Some((value, says_before, says_after)),
                    }
                }
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                text = Some(v.clone().into_bytes());
            }
            "-v" | "--invert-match" => grep.invert = true,
            "-x" | "--line-regexp" => grep.whole_line = true,
            // The matched text alone is what every report prints already.
            "-o" | "--only-matching" => {}
            "--passthru" => grep.passthru = true,
            "-L" | "--files-without-match" => grep.files_without_match = true,
            "--files" => grep.files_only = true,
            // `--texture` with no kind after it asks what each file reads as
            // rather than filtering by it, which is the reading a user wants
            // before choosing a filter.
            //
            // What follows is a kind, or it is the rest of the command line.
            // The kinds are a closed set of six, so a word that names one is
            // the value; a word that is absent, opens with a dash, or is a
            // path that exists is the rest of the line. Anything else is a
            // misspelled kind and says so, rather than being taken for a path
            // and reported later as a file that could not be read.
            "--texture" if attached.is_none() && !next_is_a_texture(args, i) => {
                match args.get(i + 1) {
                    None => grep.texture_listing = true,
                    Some(next)
                        if next.starts_with('-') || std::path::Path::new(next).exists() =>
                    {
                        grep.texture_listing = true;
                    }
                    Some(next) => {
                        eprintln!(
                            "trex: --texture takes {}, not {next:?}",
                            trex::shape::RegionKind::NAMES.join(", ")
                        );
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--texture" => {
                let value = match flag_value(args, &mut i, attached, "--texture", "a region kind") {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("trex: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                // `!kind` drops instead of keeping, as the walk's other
                // filters spell a negation.
                let (name, keeps) = match value.strip_prefix('!') {
                    Some(rest) => (rest.to_string(), false),
                    None => (value, true),
                };
                if !trex::shape::RegionKind::NAMES.contains(&name.as_str()) {
                    eprintln!(
                        "trex: --texture takes {}, not {name:?}",
                        trex::shape::RegionKind::NAMES.join(", ")
                    );
                    return ExitCode::FAILURE;
                }
                grep.textures.push((name, keeps));
            }
            "--type-list" => grep.type_list = true,
            "--stats" => {
                grep.stats = Some(match attached {
                    None => StatsForm::Lines,
                    Some("line") => StatsForm::Line,
                    Some(other) => {
                        eprintln!("trex: --stats takes no value but line, not {other:?}");
                        return ExitCode::FAILURE;
                    }
                });
            }
            "--color" => grep.color = Some(value!("always, never, auto, 16, 256 or truecolor")),
            "--colors" => grep.colors.push(value!("a spec, as in match:fg:red")),
            "-g" | "--glob" => walk.globs.push(value!("a glob, as in '*.rs' or '!*.min.js'")),
            "-t" | "--type" => walk.types.push(value!("a file type; --type-list names them")),
            "-T" | "--type-not" => walk.types_not.push(value!("a file type; --type-list names them")),
            "-e" | "--regexp" | "--pattern" => grep.patterns.push(value!("a pattern")),
            "-f" | "--file" => grep.pattern_files.push(value!("a file of patterns, one a line")),
            "-m" | "--max-count" => {
                let v = value!("a number");
                grep.max_count = Some(match count_arg(flag, Some(&v)) {
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("trex: {e}");
                        return ExitCode::FAILURE;
                    }
                });
            }
            "--sort" | "--sortr" => {
                let key = value!("path, modified, accessed or created");
                match trex::files::SortKey::parse(&key) {
                    Some(k) => walk.sort = Some(trex::files::Sort { key: k, reverse: flag == "--sortr" }),
                    None => {
                        eprintln!("trex: {flag} takes path, modified, accessed or created, not {key:?}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--all" | "--any" | "--none" | "--at-least" => {
                let rule = match flag {
                    "--all" => Quantifier::All,
                    "--any" => Quantifier::Any,
                    "--none" => Quantifier::None,
                    _ => {
                        let n = value!("a number");
                        match count_arg(flag, Some(&n)) {
                            Ok(n) => Quantifier::AtLeast(n),
                            Err(e) => {
                                eprintln!("trex: {e}");
                                return ExitCode::FAILURE;
                            }
                        }
                    }
                };
                if let Some(earlier) = grep.rule {
                    eprintln!(
                        "trex: {} and {} are two rules; a query holds one",
                        quantifier_flag(earlier),
                        quantifier_flag(rule)
                    );
                    return ExitCode::FAILURE;
                }
                grep.rule = Some(rule);
            }
            "--not" => grep.nots.push(value!("a pattern the record must not hold")),
            "--record" | "--record-start" | "--record-span" => {
                let what = value!(if flag == "--record" {
                    "line, paragraph, file, period, seam, bind, bind:Q, auto, texture, shape, block, unit or unit:ROLE"
                } else {
                    "a pattern"
                });
                if grep.record.is_some() {
                    eprintln!("trex: {flag} names a second record definition; a query holds one");
                    return ExitCode::FAILURE;
                }
                grep.record = Some(match flag {
                    "--record" => RecordSpec::Named(what),
                    "--record-start" => RecordSpec::Start(what),
                    _ => RecordSpec::Span(what),
                });
            }
            "--patterns" => {
                let file = value!("a pattern file: let lines and bare patterns as the members");
                if grep.patterns_file.is_some() {
                    eprintln!("trex: --patterns names a second file; a scan runs one set");
                    return ExitCode::FAILURE;
                }
                grep.patterns_file = Some(file);
            }
            "--single-match" => grep.single_match = true,
            "--rules" => grep.rules.push(value!("a pattern file of rules, or a directory of .trex files")),
            "--index" => grep.index = true,
            "--no-index" => grep.no_index = true,
            "--sarif" => grep.sarif = true,
            "--github" => grep.github = true,
            "--fix" => grep.fix = true,
            "--dry-run" => grep.dry_run = true,
            "-i" | "--interactive" => grep.interactive = true,
            // Every fix applied without a question: what a review that
            // accepted all would have done.
            "-U" | "--update-all" => {
                grep.fix = true;
                grep.interactive = false;
            }
            // Named before the catch-all, or it becomes a path: a scan with
            // no readable input reads standard input, so `trex scan --help`
            // blocks instead of answering, and a blocked process holds
            // target\release\trex.exe against the next build.
            "-h" | "--help" => {
                eprintln!(
                    "usage: trex scan PATTERN [FILE|DIR|-]... [--text STRING] [--count|--count-matches|-l|-L|--json|--format TEMPLATE] [--record UNIT|--record-start PATTERN|--record-span PATTERN] [-v] [-w] [-A N|-B N|-C N|-C UNIT] [--max-count N] [--stats] [--hidden] [--no-ignore] [--binary] [--explain] [--head N|--tail N|--lines A..B] [--follow]"
                );
                eprintln!();
                eprintln!(
                    "  --count          how many records hold a match, a line unless --record says otherwise, as grep -c"
                );
                eprintln!("  --count-matches  how many matches there are, as rg --count-matches");
                eprintln!(
                    "  --head N         scan the first N records of each input, lines unless --record says otherwise;"
                );
                eprintln!("                   --tail N the last N, --lines A..B records A through B");
                eprintln!("  --follow         scan what each file gains as it grows, after its --tail");
                return ExitCode::FAILURE;
            }
            _ => positionals.push(arg.to_string()),
        }
        i += 1;
    }

    if grep.type_list {
        for (name, globs) in trex::files::type_list() {
            crate::out::line(&format!("{name}: {}", globs.join(", ")));
        }
        return ExitCode::SUCCESS;
    }
    if let Err(e) = walk.check() {
        eprintln!("trex: {e}");
        return ExitCode::FAILURE;
    }
    let painter = match painter_for(grep.color.as_deref(), &grep.colors) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("trex: {e}");
            return ExitCode::FAILURE;
        }
    };
    if grep.files_only {
        // The files a scan would read, and no scan: under the paths named,
        // or the current directory where none is.
        let roots = if positionals.is_empty() { vec![".".to_string()] } else { positionals };
        let a_directory = roots.iter().any(|p| std::path::Path::new(p).is_dir());
        let (sources, errors) = collect(&roots, &walk);
        let mut failed = !errors.is_empty();
        for e in &errors {
            eprintln!("trex: {e}");
        }
        // A scan skips a binary file unless `--binary` asks for it, refusing
        // a lone named one aloud and passing over the rest, so a listing
        // drops the same files the same way.
        let lone = !a_directory && sources.len() == 1;
        // `--files --texture` names what each file reads as. The kinds given
        // still filter, so the two together list the files of one kind and
        // say which it is; `--texture` with no kind after it is the listing
        // alone, which is the reading a user wants before choosing a filter.
        let naming = grep.texture_listing;
        for src in &sources {
            if !binary && let Source::File(path) = src {
                match trex::files::is_binary_file(path) {
                    Ok(false) => {}
                    Ok(true) => {
                        if lone {
                            eprintln!("trex: {} holds a NUL byte and is binary; --binary scans it", src.name());
                            failed = true;
                        }
                        continue;
                    }
                    Err(e) => {
                        eprintln!("trex: cannot read {}: {e}", src.name());
                        failed = true;
                        continue;
                    }
                }
            }
            let kind = (naming || !grep.textures.is_empty()).then(|| texture_of(src)).flatten();
            if !grep.textures.is_empty() && !trex::shape::keeps_texture(&grep.textures, kind) {
                continue;
            }
            let name = painter.paint(trex::paint::Role::Path, &src.name());
            match (naming, kind) {
                (true, Some(kind)) => crate::out::line(&format!("{name}: {}", spelled(kind))),
                (true, None) => crate::out::line(&format!("{name}: no region")),
                _ => crate::out::line(&name),
            }
        }
        return if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    }
    if let Some(why) = grep.windowing.follow_refusal() {
        eprintln!("trex scan: {why}");
        return ExitCode::FAILURE;
    }
    // A window is read input by input, and an index summarizes each file
    // whole: one built from part of a file would refuse the file for what
    // only the rest of it holds.
    if grep.index && (grep.windowing.select.is_some() || grep.windowing.follow) {
        eprintln!("trex scan: --index summarizes whole files and takes no --head, --tail, --lines or --follow");
        return ExitCode::FAILURE;
    }
    // Rules are their own scan: each finding under its rule, with the
    // reports and the fixes a rule carries, and none of the flags that read
    // one pattern's matches.
    if !grep.rules.is_empty() {
        if !grep.patterns.is_empty() || !grep.pattern_files.is_empty() || grep.patterns_file.is_some() {
            eprintln!("trex scan: --rules names the rules and takes no -e, -f or --patterns");
            return ExitCode::FAILURE;
        }
        // Under rules, `--record` names only the unit a window counts, the
        // rules carrying their own record units.
        let record_query = grep.rule.is_some()
            || !grep.nots.is_empty()
            || (grep.record.is_some() && grep.windowing.select.is_none());
        if record_query
            || grep.invert
            || grep.whole_line
            || grep.passthru
            || grep.single_match
            || explain
            || grep.stats.is_some()
        {
            eprintln!(
                "trex scan: --rules reports findings and takes no record query, -v, -x, --passthru, --single-match, --explain or --stats; --record names the unit --head, --tail and --lines count"
            );
            return ExitCode::FAILURE;
        }
        if grep.windowing.follow
            && (grep.fix || count || count_matches || files_with_matches || grep.files_without_match || grep.sarif)
        {
            eprintln!(
                "trex scan: a followed file never ends, so --follow takes no report that is written once the input ends: --fix, --count, --count-matches, -l, -L or --sarif"
            );
            return ExitCode::FAILURE;
        }
        let record_unit = match record_unit_of(grep.record.as_ref(), &shapes) {
            Ok(unit) => unit,
            Err(e) => {
                eprintln!("trex: {e}");
                return ExitCode::FAILURE;
            }
        };
        if backend == trex::Backend::Gpu || dual_grain || chunk_size.is_some() {
            eprintln!(
                "trex scan: rules scan over one lex on the CPU engines and take no --gpu, --dual-grain or --chunk-size"
            );
            return ExitCode::FAILURE;
        }
        if (before > 0 || after > 0) && !grep.fix {
            eprintln!(
                "trex scan: --rules prints one line per finding and takes no context lines; -C sets the diff context under --fix --dry-run"
            );
            return ExitCode::FAILURE;
        }
        let reports = [
            json,
            grep.sarif,
            grep.github,
            format.is_some(),
            count,
            count_matches,
            files_with_matches,
            grep.files_without_match,
        ]
        .iter()
        .filter(|&&asked| asked)
        .count();
        if reports > 1 {
            eprintln!(
                "trex scan: --json, --sarif, --github, --format, --count, --count-matches, -l and -L are each a report of their own; give one"
            );
            return ExitCode::FAILURE;
        }
        if grep.fix && reports > 0 {
            eprintln!(
                "trex scan: --fix applies the fixes and prints no report; drop --json, --sarif, --github, --format, --count, -l or -L"
            );
            return ExitCode::FAILURE;
        }
        if (grep.dry_run || grep.interactive) && !grep.fix {
            eprintln!("trex scan: --dry-run and --interactive review fixes and take --fix");
            return ExitCode::FAILURE;
        }
        if grep.interactive && grep.dry_run {
            eprintln!("trex scan: --interactive reviews each fix and takes no --dry-run; -U applies every fix");
            return ExitCode::FAILURE;
        }
        return crate::cli_rules::run(crate::cli_rules::RulesRun {
            shapes,
            rules: &grep.rules,
            paths: positionals,
            text,
            walk,
            json,
            sarif: grep.sarif,
            github: grep.github,
            format,
            count,
            files_with_matches,
            files_without_match: grep.files_without_match,
            max_count: grep.max_count,
            require_match,
            binary,
            fix: grep.fix,
            dry_run: grep.dry_run,
            interactive: grep.interactive,
            context: if before > 0 || after > 0 { before.max(after) } else { DIFF_CONTEXT },
            painter,
            windowing: grep.windowing,
            unit: record_unit,
        });
    }

    // What the scan runs: the set `--patterns` names, whose members are what
    // is scanned, each match reported under its member's name; or one
    // pattern, the -e patterns and the -f files' lines joined as an
    // alternation where there are several, or else the first positional.
    let mut patterns = grep.patterns.clone();
    for file in &grep.pattern_files {
        match std::fs::read_to_string(file) {
            Ok(lines) => {
                patterns.extend(lines.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string));
            }
            Err(e) => {
                eprintln!("trex: cannot read the pattern file {file}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    // The pattern as written, which `--fields` reads a `\{name}` from.
    let mut pattern_text = String::new();
    let scanning = if let Some(file) = &grep.patterns_file {
        if !grep.patterns.is_empty() || !grep.pattern_files.is_empty() {
            eprintln!("trex scan: --patterns names the members and takes no -e or -f");
            return ExitCode::FAILURE;
        }
        match trex::PatternSet::from_file(std::path::Path::new(file), &mut shapes) {
            Ok(set) if set.is_empty() => {
                eprintln!("trex scan: {file} holds no pattern");
                return ExitCode::FAILURE;
            }
            Ok(set) => Scanning::Set(Box::new(set)),
            Err(e) => {
                eprintln!("trex: {}", e.msg);
                return ExitCode::FAILURE;
            }
        }
    } else {
        let pattern_src = if grep.patterns.is_empty() && grep.pattern_files.is_empty() {
            if positionals.is_empty() {
                eprintln!("trex scan: a pattern is needed: PATTERN, -e PATTERN, -f FILE or --patterns FILE");
                return ExitCode::FAILURE;
            }
            positionals.remove(0)
        } else if patterns.is_empty() {
            eprintln!("trex scan: the pattern files hold no pattern");
            return ExitCode::FAILURE;
        } else if patterns.len() == 1 {
            patterns.remove(0)
        } else {
            patterns.iter().map(|p| format!("({p})")).collect::<Vec<_>>().join(" | ")
        };
        pattern_text.clone_from(&pattern_src);
        match trex::parser::parse_with_shapes(&pattern_src, &shapes) {
            Ok(p) => Scanning::One(p),
            Err(e) => {
                eprintln!("trex: pattern error at byte {}: {}", e.pos, e.msg);
                return ExitCode::FAILURE;
            }
        }
    };
    let paths = positionals;

    if fields_table {
        let Scanning::One(pattern) = &scanning else {
            eprintln!("trex scan: --fields reads one pattern's fields and takes no --patterns set");
            return ExitCode::FAILURE;
        };
        let others = format.is_some()
            || count
            || count_matches
            || files_with_matches
            || explain
            || before > 0
            || after > 0
            || around.is_some()
            || grep.queries()
            || grep.invert
            || grep.passthru
            || grep.files_without_match
            || grep.files_only
            || grep.windowing.follow;
        if others {
            eprintln!(
                "trex scan: --fields prints its own table of records and takes no --format, count, file listing, \
                 context, --explain, record query, -v, --passthru or --follow"
            );
            return ExitCode::FAILURE;
        }
        let unit = match window_unit("scan", &grep.windowing, None, &shapes) {
            Ok(unit) => unit,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        return scan_fields(FieldsScan {
            pattern,
            source: &pattern_text,
            shapes: &shapes,
            paths: &paths,
            text,
            walk: &walk,
            windowing: &grep.windowing,
            unit: &unit,
            binary,
            json,
            require_match,
        });
    }

    // `-C record` asks what a record IS, not for a record query. Naming it in
    // a context flag is the whole signal: the record flags still say what a
    // record is, and the report stays one line per match with that record
    // printed around it. A query asked for outright, by `--rule` or `--not`,
    // still refuses context, because those two reports cannot both be printed.
    grep.record_is_the_context = around.as_ref().is_some_and(|(named, _, _)| named == "record");

    // What a record is, read here because three separate things need it and
    // none implies another: a record query groups by it, `--count` counts
    // the records holding a match, and `--head`, `--tail` and `--lines`
    // count it. Read after the set, whose file may declare a kind a
    // `--record-start` pattern names.
    let record_unit = match record_unit_of(grep.record.as_ref(), &shapes) {
        Ok(unit) => unit,
        Err(e) => {
            eprintln!("trex: {e}");
            return ExitCode::FAILURE;
        }
    };

    // A record-level query holds its patterns apart rather than joined, so
    // each is parsed on its own, with what a record must not hold and what
    // a record is; the members of a set are its patterns, under their names.
    let query = if grep.queries() {
        let parse = |src: &str| match trex::parser::parse_with_shapes(src, &shapes) {
            Ok(p) => Ok(p),
            Err(e) => Err(format!("pattern error in {src:?} at byte {}: {}", e.pos, e.msg)),
        };
        let (positives, set) = match &scanning {
            Scanning::Set(set) => (Vec::new(), Some((**set).clone())),
            Scanning::One(p) if patterns.is_empty() => (vec![p.clone()], None),
            Scanning::One(_) => {
                let mut positives = Vec::with_capacity(patterns.len());
                for src in &patterns {
                    match parse(src) {
                        Ok(p) => positives.push(p),
                        Err(e) => {
                            eprintln!("trex: {e}");
                            return ExitCode::FAILURE;
                        }
                    }
                }
                (positives, None)
            }
        };
        let mut negatives = Vec::with_capacity(grep.nots.len());
        for src in &grep.nots {
            match parse(src) {
                Ok(p) => negatives.push(p),
                Err(e) => {
                    eprintln!("trex: --not: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        Some(Query {
            quantifier: grep.rule.unwrap_or(Quantifier::Any),
            positives,
            set,
            negatives,
            unit: record_unit.clone(),
        })
    } else {
        None
    };
    // The construct the context flags print, resolved now that the record
    // flags have been read: `record` is whatever they defined, and every
    // other word is a unit in its own right.
    let around = match around {
        None => None,
        Some((named, says_before, says_after)) => {
            let unit = if named == "record" {
                match &grep.record {
                    Some(RecordSpec::Named(name)) => match trex::records::RecordUnit::parse(name) {
                        Ok(unit) => unit,
                        Err(e) => {
                            eprintln!("trex: --record: {e}");
                            return ExitCode::FAILURE;
                        }
                    },
                    Some(RecordSpec::Start(src)) => {
                        match trex::parser::parse_with_shapes(src, &shapes) {
                            Ok(p) => trex::records::RecordUnit::Start(p),
                            Err(e) => {
                                eprintln!("trex: --record-start: pattern error at byte {}: {}", e.pos, e.msg);
                                return ExitCode::FAILURE;
                            }
                        }
                    }
                    Some(RecordSpec::Span(src)) => {
                        match trex::parser::parse_with_shapes(src, &shapes) {
                            Ok(p) => trex::records::RecordUnit::Span(p),
                            Err(e) => {
                                eprintln!("trex: --record-span: pattern error at byte {}: {}", e.pos, e.msg);
                                return ExitCode::FAILURE;
                            }
                        }
                    }
                    None => {
                        eprintln!(
                            "trex: the context flags read `record` from --record, --record-start or --record-span, and none was given"
                        );
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                match trex::records::RecordUnit::parse(&named) {
                    Ok(unit) => unit,
                    Err(e) => {
                        eprintln!("trex: the context flags take a number or a record unit: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            };
            Some(Around { unit, before: says_before, after: says_after })
        }
    };
    let report_template = match &format {
        Some(src) => match trex::Template::parse_report(src, &scanning.capture_names()) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("trex: --format error at byte {}: {}", e.pos, e.msg);
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    // The explainer is built once per input, for `--explain` and for a
    // template that names an axis; a template that names none costs what it
    // always did, because the analyses behind the axes are built only here.
    let explaining_wanted =
        explain || report_template.as_ref().is_some_and(trex::Template::reads_explanation);
    // The statistics read which rung answered and what the lexes cost, both
    // of which the trace keeps only when asked. Turning it on here is what
    // makes those lines cost nothing to a run that does not print them.
    if grep.stats.is_some() {
        trex::trace::record();
    }
    if count && count_matches {
        eprintln!(
            "trex scan: --count counts the records holding a match and --count-matches counts the matches; give one"
        );
        return ExitCode::FAILURE;
    }
    if grep.single_match && scanning.set().is_none() {
        eprintln!("trex scan: --single-match keeps each member's first match and takes --patterns");
        return ExitCode::FAILURE;
    }
    if scanning.set().is_some() && (backend == trex::Backend::Gpu || dual_grain || chunk_size.is_some()) {
        eprintln!(
            "trex scan: a set's members scan over one lex on the CPU engines and take no --gpu, --dual-grain or --chunk-size"
        );
        return ExitCode::FAILURE;
    }
    let context = before > 0 || after > 0 || around.is_some();
    if report_template.is_some() && (json || count || count_matches || files_with_matches || context) {
        eprintln!(
            "trex: --format prints one line per match and takes no --json, --count, --count-matches, -l or context lines"
        );
        return ExitCode::FAILURE;
    }
    if grep.invert && (json || format.is_some() || explain || context || grep.passthru) {
        eprintln!(
            "trex: -v prints the lines no match touches and takes no --json, --format, --explain, context lines or --passthru"
        );
        return ExitCode::FAILURE;
    }
    if grep.passthru
        && (json
            || format.is_some()
            || count
            || count_matches
            || files_with_matches
            || grep.files_without_match
            || explain
            || context)
    {
        eprintln!(
            "trex: --passthru prints every line and takes no --json, --format, --count, --count-matches, -l, -L, --explain or context lines"
        );
        return ExitCode::FAILURE;
    }
    if grep.files_without_match && (files_with_matches || count || count_matches || json || format.is_some()) {
        eprintln!(
            "trex: -L names the inputs with no match and takes no -l, --count, --count-matches, --json or --format"
        );
        return ExitCode::FAILURE;
    }
    if grep.stats.is_some() && json {
        eprintln!("trex: --stats prints after the report and takes no --json");
        return ExitCode::FAILURE;
    }
    if query.is_some()
        && (format.is_some()
            || explain
            || context
            || grep.invert
            || grep.passthru
            || grep.whole_line
            || grep.single_match)
    {
        eprintln!(
            "trex: a record query prints the records that qualify and takes no --format, --explain, context lines, -v, --passthru, -x or --single-match"
        );
        return ExitCode::FAILURE;
    }
    // A followed file never ends, so it is scanned as a stream is, each
    // match printed once nothing that arrives later can change it. What a
    // report says once its input has ended, or reads around a match or over
    // the whole input, it cannot say of one.
    if grep.windowing.follow {
        if text.is_some() {
            eprintln!("trex scan: --follow follows files by name and takes no --text");
            return ExitCode::FAILURE;
        }
        if count
            || count_matches
            || files_with_matches
            || grep.files_without_match
            || grep.stats.is_some()
            || grep.invert
            || grep.passthru
            || context
            || query.is_some()
            || explaining_wanted
        {
            eprintln!(
                "trex scan: a followed file never ends, so --follow takes no report that is written once the input ends or reads around a match: --count, --count-matches, -l, -L, --stats, -v, --passthru, context lines, a record query, --explain or a template naming an axis"
            );
            return ExitCode::FAILURE;
        }
        if backend == trex::Backend::Gpu || dual_grain || chunk_size.is_some() {
            eprintln!(
                "trex scan: a followed file is scanned as it grows by the stream scanner on the CPU and takes no --gpu, --dual-grain or --chunk-size"
            );
            return ExitCode::FAILURE;
        }
    }
    // Read once a scan rather than once a match: the kinds are a property of
    // the patterns, and a tree walk reports thousands of matches over them.
    let capture_kinds = scanning.capture_kinds();
    let value_view = crate::ValueView {
        kinds: &capture_kinds,
        style,
        clock: trex::Clock::current(),
    };
    let report = Report {
        json,
        count,
        count_matches,
        record_unit: &record_unit,
        files_with_matches,
        before,
        after,
        around: around.as_ref(),
        template: report_template.as_ref(),
        explain,
        grep: &grep,
        painter: &painter,
        // Only the JSON report prints values today, so nothing else pays to
        // look one up.
        values: json.then_some(&value_view),
    };
    // Every binding under a repetition is resolved only where a report reads
    // it: the JSON tree, or a template reading one by index.
    let keep_lists = scanning.has_list_registers()
        && (json || report_template.as_ref().is_some_and(trex::Template::reads_lists));
    // `-m N` walks only as far as it prints, where nothing downstream can
    // drop a match it already counted. Inversion reports the lines no match
    // touched, so it needs every match to know which lines those are, and
    // `--whole-line` discards matches that do not cover their line, so the
    // first N found are not the first N kept. Both keep the full scan.
    let take = grep.max_count.filter(|_| !grep.invert && !grep.whole_line);
    let how =
        ScanHow { backend, dual_grain, chunk_size, say: true, lists: keep_lists, single: grep.single_match, take };

    // An inline string is one input with no name, reported as it always was;
    // a window cuts it as it cuts a file.
    if let Some(text) = text {
        if !paths.is_empty() {
            eprintln!("trex scan: --text and a path cannot both be given");
            return ExitCode::FAILURE;
        }
        if files_with_matches || grep.files_without_match {
            eprintln!("trex scan: -l and -L name files and take no --text");
            return ExitCode::FAILURE;
        }
        let part = Part::of_text(text, grep.windowing.select, &record_unit);
        let index = part.index();
        let input = part.text;
        if let Some(q) = &query {
            let began = std::time::Instant::now();
            let mut hits = q.hits(&input, &shapes, painter.is_on());
            if let Some(n) = grep.max_count {
                hits.truncate(n);
            }
            let mut stats = Stats { searching: began.elapsed(), ..Stats::default() };
            stats.count_records(&input, &hits, &index);
            report_records_unprefixed(&input, &index, &hits, &report, q.names());
            if let Some(form) = grep.stats {
                stats.read_the_trace();
                stats.print(form, started.elapsed());
            }
            return if require_match && hits.is_empty() { ExitCode::FAILURE } else { ExitCode::SUCCESS };
        }
        if explaining_wanted {
            trex::trace::record();
        }
        let began = std::time::Instant::now();
        let (mut matches, mut of_member) = scan_found(&scanning, &input, &shapes, how);
        if grep.whole_line {
            (matches, of_member) = whole_line_matches(&input, matches, of_member, &index);
        }
        if let Some(n) = grep.max_count
            && !grep.invert
        {
            matches.truncate(n);
            of_member.truncate(n);
        }
        let mut stats = Stats { searching: began.elapsed(), ..Stats::default() };
        stats.count(&input, &matches, &index);
        let route = if explaining_wanted {
            trex::explain::route_of(&trex::trace::take_recorded())
        } else {
            String::new()
        };
        let members = scanning.set().map(|set| Members { set, of: &of_member });
        let explaining =
            explaining_wanted.then(|| explaining_over(&scanning, &input, &shapes, members, &route));
        let hits = report_unprefixed("", &input, &matches, &index, &report, members, explaining.as_ref());
        if let Some(form) = grep.stats {
            stats.read_the_trace();
            stats.print(form, started.elapsed());
        }
        return if require_match && hits == 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    }

    let a_directory = paths.iter().any(|p| std::path::Path::new(p).is_dir());
    let (mut sources, walk_errors) = collect(&paths, &walk);
    if sources.is_empty() && paths.is_empty() {
        sources.push(Source::Stdin);
    }
    for e in &walk_errors {
        eprintln!("trex: {e}");
    }
    // A file named on the command line is read whatever it says: the reader
    // asked for that file, and a filter over the walk is not an argument
    // about what they asked for. Only what the walk turned up is filtered,
    // which is every source under a directory that was named.
    if !grep.textures.is_empty() {
        let named: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
        sources.retain(|src| {
            named.contains(src.name().as_str()) || trex::shape::keeps_texture(&grep.textures, texture_of(src))
        });
    }
    let prefixed = with_filename.unwrap_or(a_directory || sources.len() > 1);

    // Each file scanned from its window on, then followed as it grows, each
    // match printed once nothing that arrives later can change it.
    if grep.windowing.follow {
        if sources.contains(&Source::Stdin) {
            eprintln!(
                "trex scan: --follow follows files by name, and the standard input is read as it arrives already; name the file"
            );
            return ExitCode::FAILURE;
        }
        let print = crate::cli_follow::StreamPrint {
            json,
            format: report_template.as_ref(),
            painter: &painter,
            lists: keep_lists,
            values: json.then_some(&value_view),
            prefixed,
            whole_line: grep.whole_line,
            max_count: grep.max_count,
            single: grep.single_match,
        };
        let followed = crate::cli_follow::Followed {
            sources: &sources,
            select: grep.windowing.select,
            unit: &record_unit,
            binary,
            failed: !walk_errors.is_empty(),
        };
        return crate::cli_follow::follow_scan(&followed, &scanning, &shapes, &print);
    }

    // The standard input alone, with the plain report and the plain scan:
    // read as it arrives, each match printed as the scanner commits it.
    let plain_scan = !dual_grain
        && chunk_size.is_none()
        && backend == trex::Backend::Auto
        && shapes.is_empty()
        && !scanning.names_library_kinds();
    let grep_active = grep.invert
        || grep.whole_line
        || grep.max_count.is_some()
        || grep.passthru
        || grep.files_without_match
        || grep.stats.is_some()
        || grep.queries()
        || grep.single_match;
    let plain_report = !prefixed
        && !count
        && !count_matches
        && !files_with_matches
        && before == 0
        && after == 0
        // A construct is read from records over the whole input, which a
        // stream that drops the bytes behind its window does not have.
        && around.is_none()
        // An axis is read over the whole input, which a stream that drops the
        // bytes behind its window does not have, so a template naming one
        // leaves the streaming path exactly as `--explain` does.
        && !explaining_wanted
        && !grep_active;
    if sources.len() == 1
        && sources[0] == Source::Stdin
        && plain_scan
        && plain_report
        && grep.windowing.select.is_none()
    {
        let streamed = crate::cli_follow::StreamReport {
            json,
            require_match,
            binary,
            format: report_template.as_ref(),
            painter: &painter,
            lists: keep_lists,
            style,
        };
        return crate::cli_follow::stream_stdin(&scanning, &streamed);
    }

    // The tree's index, where one is there to read: a file it refuses for
    // every pattern is never opened, which is the whole of what it saves.
    //
    // Not consulted where the report says something about the files that do
    // not match - `-v`, `-L` and `--passthru` all print for those - since a
    // file skipped unopened would go unreported rather than reported as
    // holding nothing.
    let reports_misses = grep.invert || grep.files_without_match || grep.passthru;
    let indexes = Indexes::found(&paths, !grep.no_index && !grep.index && !reports_misses);
    // Whether the whole report is "name the inputs that matched", or its
    // negative, and nothing else. Such a report reads only `hits > 0`, so the
    // scan can stop at the first match; measured on an 8.1 MB corpus, -l and
    // -L cost the same 138.5 ms as counting all 240,000 matches.
    //
    // Every condition below is a report that reads the matches themselves or
    // changes what counts as one, and each would be wrong under a short
    // circuit rather than merely slower. `-v` makes `hits` the lines left
    // untouched, so a file holding a match can still print under `-l -v`;
    // `--whole-line` drops matches that do not cover their line, so a match
    // found is not yet a match kept; a query counts records, not matches;
    // and `--stats` reads every match to count them.
    let only_asks_whether = (files_with_matches || grep.files_without_match)
        && !grep.invert
        && !grep.whole_line
        && !grep.passthru
        && !count
        && !count_matches
        && !json
        && !explain
        && report_template.is_none()
        && before == 0
        && after == 0
        && around.is_none()
        && grep.stats.is_none()
        && !grep.queries()
        && !grep.sarif
        && !grep.github;
    // Whether the report prints line numbers, which a tail's lines ahead of
    // it are counted for. A count, a list of files and the one-input report
    // of spans print none, and read a file's tail alone.
    let places = !(only_asks_whether || count || count_matches || files_with_matches || grep.files_without_match);
    let numbers = places
        && (prefixed
            || context
            || query.is_some()
            || report_template.as_ref().is_some_and(trex::Template::reads_place));
    // Every report that writes a match writes its span or its offsets, so it
    // places a tail whose place is not known without counting.
    let asked_read = trex::window::Asked { numbers, offsets: places, binary, units: false };
    let mut failed_to_index = false;
    let asked = scanning.patterns();
    let mut outcomes: Vec<Outcome> = sources.iter().map(|_| Outcome::Pending).collect();
    let scan_slot = |k: usize, slot: &mut Outcome| {
        if indexes.refuses(&asked, &sources[k]) {
            *slot = Outcome::Skipped;
            return;
        }
        let part = match read_part(&sources[k], grep.windowing.select, &record_unit, asked_read) {
            Ok(p) => p,
            Err(e) => {
                *slot = Outcome::Failed(format!("cannot read {}: {e}", sources[k].name()));
                return;
            }
        };
        if !binary && part.binary {
            *slot = Outcome::Binary;
            return;
        }
        let (byte_base, line_base) = (part.byte_base, part.line_base);
        let input = part.text;
        // A report that prints a name and nothing else asks only whether the
        // input matched, so it stops at the first match rather than
        // collecting every one of them to look at the length.
        if only_asks_whether {
            let any = match &scanning {
                Scanning::One(p) => trex::is_match(p, &input),
                Scanning::Set(set) => set.is_match(&input),
            };
            *slot = Outcome::Answered { any };
            return;
        }
        if let Some(q) = &query {
            let mut hits = q.hits(&input, &shapes, painter.is_on());
            if let Some(n) = grep.max_count {
                hits.truncate(n);
            }
            *slot = Outcome::Queried { input, byte_base, line_base, hits };
            return;
        }
        let (mut matches, mut members) = scan_found(&scanning, &input, &shapes, ScanHow { say: !prefixed, ..how });
        if grep.whole_line {
            let index = LineIndex::new(&input);
            (matches, members) = whole_line_matches(&input, matches, members, &index);
        }
        if let Some(n) = grep.max_count
            && !grep.invert
        {
            matches.truncate(n);
            members.truncate(n);
        }
        // The rungs kept since the last scan are this scan's, since an
        // explained report scans one input at a time.
        let route = if explaining_wanted {
            trex::explain::route_of(&trex::trace::take_recorded())
        } else {
            String::new()
        };
        *slot = Outcome::Scanned { input, byte_base, line_base, matches, members, route };
    };
    let began = std::time::Instant::now();
    if explaining_wanted {
        trex::trace::record();
        for (k, slot) in outcomes.iter_mut().enumerate() {
            scan_slot(k, slot);
        }
    } else {
        across_cores(&mut outcomes, scan_slot);
    }
    let mut stats = Stats { searching: began.elapsed(), ..Stats::default() };

    // `--index` writes the tree's index from the bytes this scan has already
    // read, so building it costs no second pass, and every later scan of
    // that tree prunes with it without being asked to.
    if grep.index {
        for path in &paths {
            let dir = std::path::Path::new(path);
            if !dir.is_dir() {
                continue;
            }
            let mut index = trex::index::Index::under(dir);
            for (src, outcome) in sources.iter().zip(&outcomes) {
                if let (Source::File(file), Outcome::Scanned { input, .. }) = (src, outcome)
                    && file.starts_with(dir)
                {
                    index.observe(file, input);
                }
            }
            match index.save(dir) {
                Ok(()) => eprintln!(
                    "{}: indexed {} files; later scans of this tree prune with it",
                    dir.display(),
                    index.len()
                ),
                Err(e) => {
                    eprintln!("trex: cannot write {}: {e}", dir.join(trex::index::INDEX_FILE).display());
                    failed_to_index = true;
                }
            }
        }
    }

    let mut failed = !walk_errors.is_empty() || failed_to_index;
    let mut total = 0usize;
    if !prefixed {
        // One input, and it is not a directory: the report every scan path has
        // always given, the span and its text, unless the count, the file
        // list or lines of context were asked for.
        for (src, outcome) in sources.iter().zip(&outcomes) {
            match outcome {
                Outcome::Pending | Outcome::Skipped => {}
                Outcome::Answered { any } => {
                    // The scan stopped at the first match, so this knows
                    // whether the input matched and not how often. Only the
                    // two reports asking exactly that reach here.
                    if (files_with_matches && *any) || (grep.files_without_match && !*any) {
                        crate::out::line(&painter.paint(trex::paint::Role::Path, &src.name()));
                    }
                    if *any {
                        total += 1;
                    }
                }
                Outcome::Binary => {
                    eprintln!("trex: {} holds a NUL byte and is binary; --binary scans it", src.name());
                    failed = true;
                }
                Outcome::Failed(e) => {
                    eprintln!("trex: {e}");
                    failed = true;
                }
                Outcome::Scanned { input, byte_base, line_base, matches, members, route } => {
                    let index = LineIndex::new(input).within(*byte_base, *line_base);
                    stats.count(input, matches, &index);
                    let members = scanning.set().map(|set| Members { set, of: members });
                    let explaining = explaining_wanted
                        .then(|| explaining_over(&scanning, input, &shapes, members, route));
                    total += report_unprefixed(
                        &src.name(),
                        input,
                        matches,
                        &index,
                        &report,
                        members,
                        explaining.as_ref(),
                    );
                }
                Outcome::Queried { input, byte_base, line_base, hits } => {
                    let index = LineIndex::new(input).within(*byte_base, *line_base);
                    stats.count_records(input, hits, &index);
                    total += hits.len();
                    if count {
                        crate::out::line(&hits.len().to_string());
                    } else if files_with_matches {
                        if !hits.is_empty() {
                            crate::out::line(&painter.paint(trex::paint::Role::Path, &src.name()));
                        }
                    } else if grep.files_without_match {
                        if hits.is_empty() {
                            crate::out::line(&painter.paint(trex::paint::Role::Path, &src.name()));
                        }
                    } else {
                        let names = query.as_ref().map_or(&[][..], Query::names);
                        report_records_unprefixed(input, &index, hits, &report, names);
                    }
                }
            }
        }
    } else {
        use trex::paint::Role;
        let mut json_out = String::from("[");
        let mut first_json = true;
        for (src, outcome) in sources.iter().zip(&outcomes) {
            let name = src.name();
            match outcome {
                Outcome::Pending | Outcome::Skipped | Outcome::Binary => {}
                Outcome::Answered { any } => {
                    // The scan stopped at the first match, so this knows
                    // whether the input matched and not how often.
                    if (files_with_matches && *any) || (grep.files_without_match && !*any) {
                        crate::out::line(&painter.paint(Role::Path, &name));
                    }
                    if *any {
                        total += 1;
                    }
                }
                Outcome::Failed(e) => {
                    eprintln!("trex: {e}");
                    failed = true;
                }
                Outcome::Queried { input, byte_base, line_base, hits } => {
                    let index = LineIndex::new(input).within(*byte_base, *line_base);
                    stats.count_records(input, hits, &index);
                    total += hits.len();
                    if grep.files_without_match {
                        if hits.is_empty() {
                            crate::out::line(&painter.paint(Role::Path, &name));
                        }
                        continue;
                    }
                    if hits.is_empty() {
                        continue;
                    }
                    if count {
                        crate::out::line(&format!(
                            "{}{}{}",
                            painter.paint(Role::Path, &name),
                            painter.paint(Role::Separator, ":"),
                            hits.len()
                        ));
                    } else if files_with_matches {
                        crate::out::line(&painter.paint(Role::Path, &name));
                    } else if json {
                        let names = query.as_ref().map_or(&[][..], Query::names);
                        for hit in hits {
                            if !first_json {
                                json_out.push(',');
                            }
                            first_json = false;
                            json_out.push_str(&record_json(Some(&name), input, &index, hit, names));
                        }
                    } else {
                        print_records(Some(&name), input, &index, hits, &painter);
                    }
                }
                Outcome::Scanned { input, byte_base, line_base, matches, members, route } => {
                    let index = LineIndex::new(input).within(*byte_base, *line_base);
                    stats.count(input, matches, &index);
                    // Records holding a match, under whatever `--record`
                    // said a record is and lines where it said nothing.
                    // Walked only where a report reads it, because it builds
                    // the records of every file in the tree.
                    let records_hit = if count || grep.invert {
                        records_touched(&record_unit, input, matches, grep.invert, grep.max_count)
                    } else {
                        0
                    };
                    let hits = if grep.invert { records_hit } else { matches.len() };
                    total += hits;
                    if grep.files_without_match {
                        if hits == 0 {
                            crate::out::line(&painter.paint(Role::Path, &name));
                        }
                        continue;
                    }
                    if grep.passthru {
                        print_lines_report(Some(&name), input, matches, &index, &grep, &painter);
                        continue;
                    }
                    if hits == 0 {
                        continue;
                    }
                    // `--count` answers records, as grep, ripgrep and ugrep
                    // do, and `--count-matches` answers matches. Both, and
                    // `-l`, answer before `-v` prints lines, so that `-vc`
                    // counts the lines no match touches here as it does for
                    // one input.
                    if count || count_matches {
                        let n = if count { records_hit } else { hits };
                        crate::out::line(&format!(
                            "{}{}{n}",
                            painter.paint(Role::Path, &name),
                            painter.paint(Role::Separator, ":")
                        ));
                        continue;
                    }
                    if files_with_matches {
                        crate::out::line(&painter.paint(Role::Path, &name));
                        continue;
                    }
                    if grep.invert {
                        print_lines_report(Some(&name), input, matches, &index, &grep, &painter);
                        continue;
                    }
                    let members = scanning.set().map(|set| Members { set, of: members });
                    let explaining = explaining_wanted
                        .then(|| explaining_over(&scanning, input, &shapes, members, route));
                    if let Some(t) = &report_template {
                        print_formatted(&name, input, matches, &index, t, members, explaining.as_ref(), explain);
                        continue;
                    }
                    if json {
                        for (k, m) in matches.iter().enumerate() {
                            if !first_json {
                                json_out.push(',');
                            }
                            first_json = false;
                            let (line, col) = index.line_col(input, m.start);
                            let at = Some((name.as_str(), line, col));
                            let extra = trex::report::json_extras(m, members, k, explaining.as_ref());
                            json_out.push_str(&crate::json_match_full(
                                input,
                                m,
                                at,
                                index.offset(0),
                                extra.as_deref(),
                                Some(&value_view),
                            ));
                        }
                    } else {
                        print_with_context(
                            Some(&name),
                            input,
                            matches,
                            &index,
                            &report,
                            members,
                            explaining.as_ref(),
                        );
                    }
                }
            }
        }
        if json {
            json_out.push(']');
            crate::out::line(&json_out);
        }
    }
    if let Some(form) = grep.stats {
        stats.read_the_trace();
        stats.print(form, started.elapsed());
    }
    if failed || (require_match && total == 0) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Print each match as `line:col: text`, with the report's `before` lines
/// above its first line and `after` lines below as `line-text`, a `--`
/// between groups of lines that do not touch, and no context line printed
/// twice or printed where a match line will be. A prefix names the input
/// ahead of each line, an explainer puts the match's explanation under its
/// line, and the report's painter colors the path, the numbers, the
/// separators and the match.
fn print_with_context(
    prefix: Option<&str>,
    input: &[u8],
    matches: &[trex::Match],
    index: &LineIndex,
    report: &Report<'_>,
    members: Option<Members<'_>>,
    explaining: Option<&Explaining<'_>>,
) {
    let context = trex::report::Context {
        before: report.before,
        after: report.after,
        record: report.around.map(|a| (&a.unit, a.before, a.after)),
    };
    trex::report::context_report(prefix, input, matches, index, &context, report.painter, members, explaining, &mut |l| {
        crate::out::line(l)
    });
}

/// What one input came to under a rewrite.
enum Change {
    Pending,
    Binary,
    Failed(String),
    Unchanged,
    /// The unified diff a dry run prints.
    Diff(String),
    /// Written in place, with this many replacements.
    Written(usize),
}

/// The report `--format` asks for: one line per match from the template,
/// with where the match stands, as `index` places it, and the set member
/// that made it, and its explanation under it when one is asked for.
// The eight are one report: where the input came from, its text, matches and
// line index, the template, the members and the explainer that annotate the
// matches, and whether the explanation is printed as well as read.
#[allow(clippy::too_many_arguments)]
pub(crate) fn print_formatted(
    path: &str,
    input: &[u8],
    matches: &[trex::Match],
    index: &LineIndex,
    template: &trex::Template,
    members: Option<Members<'_>>,
    explaining: Option<&Explaining<'_>>,
    show: bool,
) {
    // An explanation is computed per match, so a template naming an axis
    // pays for it once and `--explain` under the same run reads the one it
    // already has rather than explaining the match a second time.
    let reads_axes = template.reads_explanation();
    for (k, m) in matches.iter().enumerate() {
        let (line, col) = index.line_col(input, m.start);
        let name = members.map(|ms| ms.name(k));
        let place = trex::ReportAt { path, line, col, base: index.base(), pattern: name.as_deref(), rule: None };
        let why = (reads_axes || show)
            .then(|| explaining.map(|ex| ex.explain(m, members, k)))
            .flatten();
        match &why {
            Some(e) => crate::out::line(&template.render_explained(m, input, Some(&place), e)),
            None => crate::out::line(&template.render_report(m, input, &place)),
        }
        if show && let Some(e) = &why {
            print_explanation(e);
        }
    }
}

/// `trex redact`: mask every match of a pattern, leaving the fields `--keep`
/// names where they stand. The pattern's guarded kinds do the false-positive
/// work: a run of digits is a card only under the Luhn check, a token a JWT
/// only under its header, so what is masked is what the kinds recognize.
pub fn run_redact(args: &[String]) -> ExitCode {
    use std::io::Write;

    if args.is_empty() {
        eprintln!(
            "usage: trex redact PATTERN [FILE|DIR|-]... [--text STRING] [--keep 'name:acc, ...'] [--mask C] [--lib FILE] [--shape DECL] [--in-place] [--dry-run] [-C N] [--hidden] [--no-ignore] [--binary] [--head N|--tail N|--lines A..B] [--record UNIT] [--follow]"
        );
        eprintln!(
            "  --keep names the fields left readable, written as a template writes them: card:last4, ip:octet1-2, email:domain"
        );
        return ExitCode::FAILURE;
    }
    let pattern_src = &args[0];
    let mut text: Option<Vec<u8>> = None;
    let mut paths: Vec<String> = Vec::new();
    let mut keep_srcs: Vec<String> = Vec::new();
    let mut mask_src = "*".to_string();
    let mut in_place = false;
    let mut dry_run = false;
    let mut walk = WalkOptions::default();
    let mut binary = false;
    let mut shapes = trex::ShapeSet::new();
    let mut windowing = Windowing::default();
    let mut record: Option<RecordSpec> = None;
    // The lines of context a dry run's diff shows around each change.
    let mut context = DIFF_CONTEXT;

    let mut i = 1;
    while i < args.len() {
        match windowing.take(args, &mut i) {
            Some(Ok(())) => {
                i += 1;
                continue;
            }
            Some(Err(e)) => {
                eprintln!("trex: {e}");
                return ExitCode::FAILURE;
            }
            None => {}
        }
        match args[i].as_str() {
            flag @ ("--record" | "--record-start" | "--record-span") => {
                if let Err(e) = take_record(flag, args, &mut i, &mut record) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--shape" | "--shape-after" => {
                let when = if args[i] == "--shape" {
                    trex::Precedence::Before
                } else {
                    trex::Precedence::After
                };
                i += 1;
                let Some(decl) = args.get(i) else {
                    eprintln!("trex: {} needs a `name = `pattern`` declaration", args[i - 1]);
                    return ExitCode::FAILURE;
                };
                if let Err(e) = shapes.declare(decl, when) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--lib" => {
                i += 1;
                let Some(path) = args.get(i) else {
                    eprintln!("trex: --lib needs a pattern file");
                    return ExitCode::FAILURE;
                };
                if let Err(e) = declare_file(&mut shapes, path) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--keep" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --keep needs a list of fields, as in 'card:last4, ip:octet1-2'");
                    return ExitCode::FAILURE;
                };
                keep_srcs.push(v.clone());
            }
            "--mask" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --mask needs a character or a token");
                    return ExitCode::FAILURE;
                };
                mask_src = v.clone();
            }
            "--in-place" => in_place = true,
            "--dry-run" => dry_run = true,
            "--hidden" => walk.hidden = true,
            "--no-ignore" => walk.no_ignore = true,
            "--binary" => binary = true,
            "-C" | "--context" => {
                i += 1;
                context = match count_arg("-C", args.get(i)) {
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("trex: {e}");
                        return ExitCode::FAILURE;
                    }
                };
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                text = Some(v.clone().into_bytes());
            }
            path => paths.push(path.to_string()),
        }
        i += 1;
    }

    let pattern = match trex::parser::parse_with_shapes(pattern_src, &shapes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("trex: pattern error at byte {}: {}", e.pos, e.msg);
            return ExitCode::FAILURE;
        }
    };
    let names = pattern.capture_names();
    let mut keeps: Vec<trex::Keep> = Vec::new();
    for src in &keep_srcs {
        match trex::Keep::parse_list(src, &names) {
            Ok(list) => keeps.extend(list),
            Err(e) => {
                eprintln!("trex: --keep error at byte {}: {}", e.pos, e.msg);
                return ExitCode::FAILURE;
            }
        }
    }
    let mut mask = match trex::Mask::parse(&mask_src) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("trex: --mask: {e}");
            return ExitCode::FAILURE;
        }
    };
    let unit = match window_unit("redact", &windowing, record.as_ref(), &shapes) {
        Ok(unit) => unit,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    if windowing.follow && (in_place || dry_run) {
        eprintln!(
            "trex redact: --follow writes what a growing file gains to the standard output, redacted; a file another process appends to is not redacted in place, so it takes no --in-place or --dry-run"
        );
        return ExitCode::FAILURE;
    }

    let a_directory = paths.iter().any(|p| std::path::Path::new(p).is_dir());
    let (mut sources, walk_errors) = match text {
        Some(_) if !paths.is_empty() => {
            eprintln!("trex redact: --text and a path cannot both be given");
            return ExitCode::FAILURE;
        }
        Some(_) => (Vec::new(), Vec::new()),
        None => collect(&paths, &walk),
    };
    if text.is_none() && sources.is_empty() && paths.is_empty() {
        sources.push(Source::Stdin);
    }
    for e in &walk_errors {
        eprintln!("trex: {e}");
    }

    // A redaction rewrites every match, so it stops short of none.
    let found = |input: &[u8]| {
        scan_one(
            &pattern,
            input,
            &shapes,
            trex::Backend::Auto,
            false,
            None,
            false,
            pattern.has_list_registers(),
            None,
        )
    };
    // One input and no request to edit it in place: the redacted bytes go
    // to the standard output. Where a window is named, the window alone is
    // read, redacted and printed, so nothing outside it prints unredacted.
    if !in_place && !dry_run {
        let followed = match followed_file("redact", &windowing, &text, &sources) {
            Ok(followed) => followed,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        // The standard output carries text, never line numbers, so a tail is
        // read without counting the lines ahead of it, and placed only for a
        // follow, which continues its offsets.
        let asked = trex::window::Asked { offsets: followed.is_some(), binary, ..trex::window::Asked::default() };
        let part = match one_input("redact", "redacts", &text, &sources, asked, windowing.select, &unit) {
            Ok(part) => part,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        if let Some(path) = followed {
            let lists = pattern.has_list_registers();
            let mut edits = |held: &[u8], spans: &[trex::Span]| {
                let matches = if lists {
                    trex::captures_with_shapes_and_lists(&pattern, held, &shapes, spans)
                } else {
                    trex::captures_with_shapes(&pattern, held, &shapes, spans)
                };
                trex::redactions_with_shapes(held, &matches, &keeps, &mut mask, &pattern, &shapes)
            };
            return crate::cli_follow::follow_edit("redact", path, &part, &pattern, &shapes, &mut edits);
        }
        let input = part.text;
        let edits = trex::redactions_with_shapes(&input, &found(&input), &keeps, &mut mask, &pattern, &shapes);
        let out = apply(&input, &edits);
        if let Err(e) = std::io::stdout().write_all(&out) {
            eprintln!("trex: cannot write the standard output: {e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    if text.is_some() {
        eprintln!("trex: --in-place and --dry-run need a FILE (not --text)");
        return ExitCode::FAILURE;
    }
    if in_place && sources.contains(&Source::Stdin) {
        eprintln!("trex: --in-place cannot write the standard input");
        return ExitCode::FAILURE;
    }
    let how = Editing {
        failed: !walk_errors.is_empty(),
        a_directory,
        binary,
        verb: "redacts",
        dry_run,
        context,
        restrict: Restrict::of(&windowing, &unit),
    };
    // A pseudonym carries a book from one input to the next, so its inputs
    // are redacted one after another in the order they were walked, the book
    // held by this thread alone: two runs of one command over one tree must
    // name every value alike, or a reader cannot diff a redacted copy against
    // itself. Every other mask carries nothing between inputs, so each input
    // takes its own copy and the cores share nothing.
    if matches!(mask, trex::Mask::Pseudonym(_)) {
        return edit_sources_in_order(&sources, how, |_, piece| {
            trex::redactions_with_shapes(piece.text, &found(piece.text), &keeps, &mut mask, &pattern, &shapes)
        });
    }
    edit_sources(&sources, how, |_, piece| {
        let mut copy = mask.clone();
        trex::redactions_with_shapes(piece.text, &found(piece.text), &keeps, &mut copy, &pattern, &shapes)
    })
}

/// `trex templates`: the distinct record shapes of the inputs, each printed
/// once with the records it covers, most frequent first; the readable form by
/// default, the pattern form on request, and the rare ones alone under a
/// cut. Several inputs are one stream, so a directory of logs of one kind
/// yields one set of templates.
///
/// A record is a line until `--record` names another unit, and with
/// `--against` each template is marked shared or novel by whether the other
/// input holds a template that would accept its records.
pub fn run_templates(args: &[String]) -> ExitCode {
    let mut text: Option<Vec<u8>> = None;
    let mut paths: Vec<String> = Vec::new();
    let mut json = false;
    let mut as_pattern = false;
    let mut rare_only = false;
    let mut novel_only = false;
    let mut against: Vec<String> = Vec::new();
    let mut record: Option<RecordSpec> = None;
    let mut cut = trex::templates::Rarity::Mean;
    let mut walk = WalkOptions::default();
    let mut binary = false;
    let mut windowing = Windowing::default();
    let mut i = 0;
    while i < args.len() {
        match windowing.take(args, &mut i) {
            Some(Ok(())) => {
                i += 1;
                continue;
            }
            Some(Err(e)) => {
                eprintln!("trex: {e}");
                return ExitCode::FAILURE;
            }
            None => {}
        }
        match args[i].as_str() {
            "--json" => json = true,
            "--pattern" => as_pattern = true,
            "--rare" => rare_only = true,
            "--novel" => novel_only = true,
            "--against" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --against needs a file or a directory to mine beside this one");
                    return ExitCode::FAILURE;
                };
                against.push(v.clone());
            }
            flag @ ("--record" | "--record-start" | "--record-span") => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!(
                        "trex: {flag} needs {}",
                        if flag == "--record" {
                            "line, paragraph, file, period, seam, bind, bind:Q, auto, texture, shape, block, unit or unit:ROLE"
                        } else {
                            "a pattern"
                        }
                    );
                    return ExitCode::FAILURE;
                };
                if record.is_some() {
                    eprintln!("trex: {flag} names a second record definition; a mining holds one");
                    return ExitCode::FAILURE;
                }
                record = Some(match flag {
                    "--record" => RecordSpec::Named(v.clone()),
                    "--record-start" => RecordSpec::Start(v.clone()),
                    _ => RecordSpec::Span(v.clone()),
                });
            }
            "--cut" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --cut needs a count of lines or a share, as in 5 or 1%");
                    return ExitCode::FAILURE;
                };
                cut = match trex::templates::Rarity::parse(v) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("trex: --cut: {e}");
                        return ExitCode::FAILURE;
                    }
                };
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                text = Some(v.clone().into_bytes());
            }
            "--hidden" => walk.hidden = true,
            "--no-ignore" => walk.no_ignore = true,
            "--binary" => binary = true,
            "-h" | "--help" => {
                eprintln!(
                    "usage: trex templates [FILE|DIR|-]... [--text STRING] [--record UNIT|--record-start PATTERN|--record-span PATTERN] [--against FILE|DIR] [--novel] [--pattern] [--rare] [--cut N|P%] [--json] [--hidden] [--no-ignore] [--binary] [--head N|--tail N|--lines A..B]"
                );
                eprintln!(
                    "  --head N, --tail N and --lines A..B mine that part of each input, in records of the --record unit; --against reads its inputs whole"
                );
                return ExitCode::FAILURE;
            }
            other => paths.push(other.to_string()),
        }
        i += 1;
    }
    if novel_only && against.is_empty() {
        eprintln!("trex: --novel needs --against; novel is against another input");
        return ExitCode::FAILURE;
    }
    if windowing.follow {
        eprintln!("trex templates: the templates are printed once the inputs end, and a followed file does not end; drop --follow");
        return ExitCode::FAILURE;
    }

    let unit = match &record {
        None => trex::records::RecordUnit::Line,
        Some(RecordSpec::Named(name)) => match trex::records::RecordUnit::parse(name) {
            Ok(unit) => unit,
            Err(e) => {
                eprintln!("trex: --record: {e}");
                return ExitCode::FAILURE;
            }
        },
        Some(RecordSpec::Start(src) | RecordSpec::Span(src)) => match trex::parse(src) {
            Ok(p) => {
                if matches!(record, Some(RecordSpec::Start(_))) {
                    trex::records::RecordUnit::Start(p)
                } else {
                    trex::records::RecordUnit::Span(p)
                }
            }
            Err(e) => {
                let flag =
                    if matches!(record, Some(RecordSpec::Start(_))) { "--record-start" } else { "--record-span" };
                eprintln!("trex: {flag}: pattern error in {src:?} at byte {}: {}", e.pos, e.msg);
                return ExitCode::FAILURE;
            }
        },
    };

    let mut failed = false;
    let restrict = Restrict::of(&windowing, &unit);
    let input = match (text, paths.is_empty()) {
        (Some(t), _) => Part::of_text(t, windowing.select, &unit).text,
        (None, true) => {
            let asked = trex::window::Asked { binary: true, ..trex::window::Asked::default() };
            match read_part(&Source::Stdin, windowing.select, &unit, asked) {
                Ok(part) => part.text,
                Err(e) => {
                    eprintln!("trex: -: {e}");
                    failed = true;
                    Vec::new()
                }
            }
        }
        (None, false) => read_one_stream(&paths, &walk, binary, restrict, &mut failed),
    };
    let other = (!against.is_empty())
        .then(|| read_one_stream(&against, &walk, binary, None, &mut failed))
        .map(|bytes| {
            let mining = trex::templates::Mining::mine_records(
                &trex::lexer::lex(&bytes),
                &bytes,
                &unit.records(&bytes),
            );
            (bytes, mining)
        });

    let mining = trex::templates::Mining::mine_records(
        &trex::lexer::lex(&input),
        &input,
        &unit.records(&input),
    );
    // Shared where the other input holds a template that would accept these
    // records; every template is shared when there is no other input, and the
    // mark is printed only where one was given.
    let shared: Vec<bool> = match &other {
        Some((_, theirs)) => mining.shared_with(theirs),
        None => vec![true; mining.templates.len()],
    };
    let rows: Vec<(usize, &trex::templates::Template)> = mining
        .templates
        .iter()
        .enumerate()
        .filter(|(index, _)| !rare_only || mining.is_rare(*index, cut))
        .filter(|(index, _)| !novel_only || !shared[*index])
        .collect();
    let mark = |index: usize| if shared[index] { "shared" } else { "novel" };
    if json {
        let mut out = String::from("[");
        for (k, (index, t)) in rows.iter().enumerate() {
            if k > 0 {
                out.push(',');
            }
            let records: Vec<String> = t.records.iter().map(usize::to_string).collect();
            let marked =
                if other.is_some() { format!(",\"mark\":\"{}\"", mark(*index)) } else { String::new() };
            out.push_str(&format!(
                "{{\"count\":{},\"records\":[{}],\"template\":\"{}\",\"pattern\":\"{}\",\"rare\":{}{marked}}}",
                t.count(),
                records.join(","),
                crate::json_escape(&t.readable()),
                crate::json_escape(&t.pattern()),
                mining.is_rare(*index, cut)
            ));
        }
        out.push(']');
        println!("{out}");
    } else {
        let width = rows.iter().map(|(_, t)| t.count().to_string().len()).max().unwrap_or(1);
        for (index, t) in &rows {
            let spelled = if as_pattern { t.pattern() } else { t.readable() };
            if other.is_some() {
                println!("{:<7} {:>width$}  {spelled}", mark(*index), t.count(), width = width);
            } else {
                println!("{:>width$}  {spelled}", t.count(), width = width);
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// The paths read as one stream, each input cut to the window `restrict`
/// names where it names one, a newline between two inputs where the first
/// does not end in one, with each failure reported and `failed` set.
///
/// What `templates` reads both its inputs and its `--against` inputs through,
/// so the two are walked, decoded and refused by the same rules.
fn read_one_stream(
    paths: &[String],
    walk: &WalkOptions,
    binary: bool,
    restrict: Option<Restrict<'_>>,
    failed: &mut bool,
) -> Vec<u8> {
    let mut input: Vec<u8> = Vec::new();
    let mut append = |bytes: Vec<u8>| {
        if !input.is_empty() && !input.ends_with(b"\n") {
            input.push(b'\n');
        }
        input.extend_from_slice(&bytes);
    };
    let (sources, errors) = collect(paths, walk);
    for e in &errors {
        eprintln!("trex: {e}");
        *failed = true;
    }
    let line = trex::records::RecordUnit::Line;
    let (select, unit) = match restrict {
        Some(r) => (Some(r.select), r.unit),
        None => (None, &line),
    };
    let asked = trex::window::Asked { binary, ..trex::window::Asked::default() };
    for src in &sources {
        match read_part(src, select, unit, asked) {
            // A file holding a NUL byte is binary and has no records to read,
            // unless --binary says to read them.
            Ok(part) if !binary && part.binary => {}
            Ok(part) => append(part.text),
            Err(e) => {
                eprintln!("trex: {}: {e}", src.name());
                *failed = true;
            }
        }
    }
    input
}

/// `trex infer`: the most specific pattern every example matches, the
/// examples given as arguments, as the lines of a file, or as the lines of
/// the standard input, verified against each before it is printed. With a
/// field marked, the pattern extracting every field from every shape of
/// the lines, and the report on it.
pub fn run_infer(args: &[String]) -> ExitCode {
    let mut examples: Vec<Vec<u8>> = Vec::new();
    let mut counters: Vec<Vec<u8>> = Vec::new();
    let mut anchored = false;
    let mut marked: Vec<trex::infer::marks::Marked> = Vec::new();
    let mut hints: Vec<(String, Vec<String>)> = Vec::new();
    let mut marks_in_lines = false;
    let mut unanchored = false;
    let mut no_mint = false;
    let mut mint_shapes = false;
    let mut shapes = trex::ShapeSet::new();
    let mut declared = false;
    let mut output = BuildOutput::Report;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--shape" | "--shape-after" => {
                let when = if args[i] == "--shape" {
                    trex::Precedence::Before
                } else {
                    trex::Precedence::After
                };
                i += 1;
                let Some(decl) = args.get(i) else {
                    eprintln!("trex: {} needs a `name = `pattern`` declaration", args[i - 1]);
                    return ExitCode::FAILURE;
                };
                if let Err(e) = shapes.declare(decl, when) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
                declared = true;
            }
            "--lib" => {
                i += 1;
                let Some(path) = args.get(i) else {
                    eprintln!("trex: --lib needs a pattern file");
                    return ExitCode::FAILURE;
                };
                if let Err(e) = declare_file(&mut shapes, path) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
                declared = true;
            }
            "--anchored" => anchored = true,
            "--unanchored" => unanchored = true,
            "--no-mint" => no_mint = true,
            "--mint-shapes" => mint_shapes = true,
            "--field" => {
                i += 1;
                let Some((name, value)) = args.get(i).and_then(|a| a.split_once('=')) else {
                    eprintln!("trex: --field needs NAME=VALUE, a field and one value it takes in the lines");
                    return ExitCode::FAILURE;
                };
                match hints.iter_mut().find(|(n, _)| n == name) {
                    Some((_, values)) => values.push(value.to_string()),
                    None => hints.push((name.to_string(), vec![value.to_string()])),
                }
            }
            "--marked" => marks_in_lines = true,
            "--json" => output = BuildOutput::Json,
            "--pattern" => output = BuildOutput::Pattern,
            "--lib-file" => output = BuildOutput::Lib,
            "--mark" => {
                i += 1;
                let Some(src) = args.get(i) else {
                    eprintln!("trex: --mark needs a line with each field marked as {{name:text}}");
                    return ExitCode::FAILURE;
                };
                match trex::infer::marks::parse_lines(src) {
                    Ok(lines) => marked.extend(lines),
                    Err((n, e)) => {
                        eprintln!("trex infer: --mark {src:?}: line {}: column {}: {}", n + 1, e.col, e.msg);
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--marks" => {
                i += 1;
                let Some(path) = args.get(i) else {
                    eprintln!("trex: --marks needs a file of marked lines, one per line");
                    return ExitCode::FAILURE;
                };
                let src = if path == "-" { Source::Stdin } else { Source::File(std::path::PathBuf::from(path)) };
                let raw = match read_source(&src) {
                    Ok(raw) => raw,
                    Err(e) => {
                        eprintln!("trex: cannot read {}: {e}", src.name());
                        return ExitCode::FAILURE;
                    }
                };
                match trex::infer::marks::parse_lines(&String::from_utf8_lossy(&trex::encoding::decode(raw))) {
                    Ok(lines) => marked.extend(lines),
                    Err((n, e)) => {
                        eprintln!("trex infer: {}: line {}: column {}: {}", src.name(), n + 1, e.col, e.msg);
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--not" => {
                i += 1;
                let Some(counter) = args.get(i) else {
                    eprintln!("trex: --not needs an example the pattern must miss");
                    return ExitCode::FAILURE;
                };
                counters.push(counter.as_bytes().to_vec());
            }
            "-f" | "--file" => {
                i += 1;
                let Some(path) = args.get(i) else {
                    eprintln!("trex: -f needs a file of examples, one per line");
                    return ExitCode::FAILURE;
                };
                let src = if path == "-" {
                    Source::Stdin
                } else {
                    Source::File(std::path::PathBuf::from(path))
                };
                match read_source(&src) {
                    Ok(raw) => examples.extend(example_lines(&trex::encoding::decode(raw))),
                    Err(e) => {
                        eprintln!("trex: cannot read {}: {e}", src.name());
                        return ExitCode::FAILURE;
                    }
                }
            }
            "-h" | "--help" => {
                eprintln!("usage: trex infer EXAMPLE EXAMPLE... [--not EXAMPLE]... [-f FILE] [--anchored]");
                eprintln!("       trex infer --mark LINE... [--marks FILE] [--field NAME=VALUE]...");
                eprintln!("                  [LINE... | -f FILE] [--marked] [--not LINE]... [--unanchored]");
                eprintln!("                  [--no-mint | --mint-shapes] [--lib FILE] [--shape DECL]...");
                eprintln!("                  [--pattern | --json | --lib-file]");
                eprintln!("  with no example and no -f, the lines of the standard input are the examples");
                eprintln!("  --not gives an example the pattern must miss, which is what decides");
                eprintln!("  whether a position reports a value range or its bare kind");
                eprintln!("  --mark gives a line with each field to extract marked as {{name:text}}; the");
                eprintln!("  pattern then extracts every field from every shape of the lines, whole lines");
                eprintln!("  unless --unanchored, and a report shows what it reads from each line");
                eprintln!("  --field names a field by one value it takes, found wherever it stands in the");
                eprintln!("  lines; repeat it for more values or fields");
                eprintln!("  a template may run over several lines, and a field written {{name*:text}} begins");
                eprintln!("  a record the lines after it join, as a ConvertFrom-String template writes it");
                eprintln!("  a word field whose values share a constant part is spelled as its byte shape,");
                eprintln!("  `KB[0-9]{{7}}` for KB5031354, unless --no-mint; --mint-shapes declares it as a");
                eprintln!("  named shape instead, which the pattern reads only beside its declaration");
                eprintln!("  --lib, --shape and --shape-after lex the lines under their declarations, as");
                eprintln!("  scan does, so a declared shape is one token the pattern names");
                eprintln!("  --marked reads marks in the lines themselves; --pattern prints the pattern");
                eprintln!("  alone, --json the report as JSON, --lib-file the pattern as a --lib file");
                return ExitCode::FAILURE;
            }
            example => examples.push(example.as_bytes().to_vec()),
        }
        i += 1;
    }
    let building = !marked.is_empty() || !hints.is_empty() || marks_in_lines;
    // A builder given its marks on the command line reads the terminal for
    // lines only when something is piped to it.
    let from_stdin = examples.is_empty()
        && !(!marked.is_empty() && std::io::IsTerminal::is_terminal(&std::io::stdin()));
    if from_stdin {
        match read_source(&Source::Stdin) {
            Ok(raw) => examples.extend(example_lines(&trex::encoding::decode(raw))),
            Err(e) => {
                eprintln!("trex: -: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if building {
        let mint = match (no_mint, mint_shapes) {
            (true, true) => {
                eprintln!("trex infer: --no-mint and --mint-shapes ask for opposite spellings; give one");
                return ExitCode::FAILURE;
            }
            (true, false) => trex::infer::build::Mint::Off,
            (false, true) => trex::infer::build::Mint::Shapes,
            (false, false) => trex::infer::build::Mint::Inline,
        };
        let fields = Fields { marked, hints, marks_in_lines };
        return run_build(&examples, &counters, fields, unanchored, mint, &shapes, output);
    }
    if declared {
        eprintln!("trex infer: --lib and --shape lex the lines of a pattern built for fields; mark a field or give --field");
        return ExitCode::FAILURE;
    }
    let refs: Vec<&[u8]> = examples.iter().map(Vec::as_slice).collect();
    let against: Vec<&[u8]> = counters.iter().map(Vec::as_slice).collect();
    match trex::infer::infer_against(&refs, &against) {
        Ok(inferred) => {
            println!("{}", if anchored { inferred.anchored() } else { inferred.pattern().to_string() });
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("trex infer: {e}");
            ExitCode::FAILURE
        }
    }
}

/// What `trex infer` prints of a built pattern.
#[derive(Clone, Copy)]
enum BuildOutput {
    /// The pattern, the format template, the shapes and each line's values.
    Report,
    /// The pattern alone.
    Pattern,
    /// The report as JSON.
    Json,
    /// The pattern as a file `--lib` reads.
    Lib,
}

/// The fields `trex infer` is asked to build a pattern for.
struct Fields {
    /// The marked examples `--mark` and `--marks` give.
    marked: Vec<trex::infer::marks::Marked>,
    /// Each `--field` name with the values given for it.
    hints: Vec<(String, Vec<String>)>,
    /// Whether the lines themselves read in the markup, as `--marked` says.
    marks_in_lines: bool,
}

/// `trex infer` with a field marked or hinted: the pattern extracting every
/// field from every shape of `lines` lexed under `shapes`, a shared byte
/// shape spelled as `mint` says, printed as `output` asks. Where the lines
/// read in the markup, a line with a mark is a marked example.
fn run_build(
    lines: &[Vec<u8>],
    counters: &[Vec<u8>],
    fields: Fields,
    unanchored: bool,
    mint: trex::infer::build::Mint,
    shapes: &trex::ShapeSet,
    output: BuildOutput,
) -> ExitCode {
    let Fields { mut marked, hints, marks_in_lines } = fields;
    let mut data: Vec<String> = Vec::with_capacity(lines.len());
    for (n, line) in lines.iter().enumerate() {
        let text = String::from_utf8_lossy(line).into_owned();
        if !marks_in_lines {
            data.push(text);
            continue;
        }
        match trex::infer::marks::parse(&text) {
            Ok(m) if m.marks.is_empty() => data.push(m.text),
            Ok(m) => marked.push(m),
            Err(e) => {
                eprintln!("trex infer: line {}: column {}: {}", n + 1, e.col, e.msg);
                return ExitCode::FAILURE;
            }
        }
    }
    let counters: Vec<String> = counters.iter().map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    let spec = trex::infer::build::Spec {
        lines: &data,
        marked: &marked,
        hints: &hints,
        counters: &counters,
        shapes,
        unanchored,
        mint,
    };
    let built = match trex::infer::build::build(&spec) {
        Ok(built) => built,
        Err(e) => {
            eprintln!("trex infer: {e}");
            if let Some(row) = e.line() {
                let mut texts = marked
                    .iter()
                    .map(|m| m.text.as_str())
                    .chain(data.iter().map(String::as_str).filter(|l| !marked.iter().any(|m| m.text == *l)));
                if let Some(text) = texts.nth(row) {
                    eprintln!("  line {}: {text}", row + 1);
                }
            }
            return ExitCode::FAILURE;
        }
    };
    match output {
        BuildOutput::Pattern => {
            println!("{}", built.pattern);
            if !built.declarations.is_empty() {
                let shapes: Vec<String> = built
                    .declarations
                    .iter()
                    .map(|d| format!("--shape '{}'", d.strip_prefix("shape ").expect("a minted declaration is a shape")))
                    .collect();
                eprintln!(
                    "trex infer: the pattern reads the shapes it declares; scan it with {}, or print them with it by --lib-file",
                    shapes.join(" ")
                );
            }
        }
        BuildOutput::Lib => print!("{}", built.file()),
        BuildOutput::Json => println!("{}", built_json(&built)),
        BuildOutput::Report => print!("{}", built_report(&built)),
    }
    ExitCode::SUCCESS
}

/// Line numbers from one, runs of consecutive numbers written `a-b`.
fn line_runs(rows: &[usize]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let mut j = i;
        while j + 1 < rows.len() && rows[j + 1] == rows[j] + 1 {
            j += 1;
        }
        out.push(if j > i { format!("{}-{}", rows[i] + 1, rows[j] + 1) } else { (rows[i] + 1).to_string() });
        i = j + 1;
    }
    out.join(" ")
}

/// How a shape reaches its fields, as the report says it: the fields its
/// lines mark, those the literals of another shape place, and those it
/// does not hold.
fn reach_phrase(built: &trex::infer::build::Built, reach: &[trex::infer::build::Reach]) -> String {
    use trex::infer::build::Reach;

    let name = |f: usize| built.fields[f].name.as_str();
    let mut parts: Vec<String> = Vec::new();
    let marked: Vec<&str> = (0..reach.len()).filter(|&f| reach[f] == Reach::Marked).map(name).collect();
    if !marked.is_empty() {
        parts.push(format!("{} marked", marked.join(" ")));
    }
    let mut sources: Vec<usize> = reach
        .iter()
        .filter_map(|r| match r {
            Reach::Anchored(s) => Some(*s),
            _ => None,
        })
        .collect();
    sources.sort_unstable();
    sources.dedup();
    for s in sources {
        let placed: Vec<&str> = (0..reach.len()).filter(|&f| reach[f] == Reach::Anchored(s)).map(name).collect();
        parts.push(format!("{} by the literals of shape {}", placed.join(" "), s + 1));
    }
    let missing: Vec<&str> = (0..reach.len()).filter(|&f| reach[f] == Reach::Missing).map(name).collect();
    if !missing.is_empty() {
        parts.push(format!("no {}", missing.join(" ")));
    }
    parts.join("; ")
}

/// The report `trex infer` prints on a built pattern: the pattern, the
/// `--format` template writing every field, the shapes it declares, each
/// shape's lines and how it reaches the fields, and what the pattern reads
/// from each line.
fn built_report(built: &trex::infer::build::Built) -> String {
    let mut out = format!("pattern  {}\nformat   {}\n", built.pattern, built.format());
    for decl in &built.declarations {
        out.push_str(&format!("declare  {decl}\n"));
    }
    for (field, classes) in &built.suggestions {
        let atoms: Vec<String> = classes.iter().map(|c| format!("\\{{{c}}}")).collect();
        out.push_str(&format!("suggest  {field} as {}\n", atoms.join(" or ")));
    }
    out.push('\n');
    let lines: Vec<String> = built.shapes.iter().map(|s| line_runs(&s.lines)).collect();
    let width = lines.iter().map(String::len).chain(std::iter::once("lines".len())).max().unwrap_or(5);
    out.push_str(&format!("shape  {:<width$}  reads\n", "lines"));
    for (i, shape) in built.shapes.iter().enumerate() {
        out.push_str(&format!("{:<5}  {:<width$}  {}\n", i + 1, lines[i], reach_phrase(built, &shape.reach)));
    }
    let mut table: Vec<Vec<String>> = vec![
        ["line", "shape"]
            .into_iter()
            .map(str::to_string)
            .chain(built.fields.iter().map(|f| f.name.clone()))
            .chain(std::iter::once("text".to_string()))
            .collect(),
    ];
    for (r, row) in built.rows.iter().enumerate() {
        let shape = match row.shape {
            Some(s) => (s + 1).to_string(),
            None => "-".to_string(),
        };
        let values = row.values.iter().map(|v| match v {
            Some(v) => v.joined(),
            None => "-".to_string(),
        });
        table.push(
            [(r + 1).to_string(), shape].into_iter().chain(values).chain(std::iter::once(row.text.clone())).collect(),
        );
    }
    out.push('\n');
    out.push_str(&aligned_table(&table));
    if built.fields.iter().any(|f| f.starts_record) {
        let mut records: Vec<Vec<String>> = vec![
            ["record", "lines"].into_iter().map(str::to_string).chain(built.fields.iter().map(|f| f.name.clone())).collect(),
        ];
        for (i, record) in built.records.iter().enumerate() {
            let values = record.values.iter().map(|v| match v {
                Some(v) => v.joined().replace('\n', "\\n"),
                None => "-".to_string(),
            });
            records.push([(i + 1).to_string(), line_runs(&record.lines)].into_iter().chain(values).collect());
        }
        out.push('\n');
        out.push_str(&aligned_table(&records));
    }
    out
}

/// `table`'s rows with each column padded to its widest cell, two spaces
/// between columns and nothing after the last.
fn aligned_table(table: &[Vec<String>]) -> String {
    let columns = table[0].len();
    let widths: Vec<usize> =
        (0..columns).map(|c| table.iter().map(|row| row[c].chars().count()).max().unwrap_or(0)).collect();
    let mut out = String::new();
    for row in table {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(c, cell)| if c + 1 == columns { cell.clone() } else { format!("{cell:<w$}", w = widths[c]) })
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// A JSON string, or `null`.
fn json_or_null(value: Option<&str>) -> String {
    match value {
        Some(v) => format!("\"{}\"", crate::json_escape(v)),
        None => "null".to_string(),
    }
}

/// A field's value as JSON: a string, an array of the strings of a list,
/// or `null`.
fn json_value(value: Option<&trex::infer::build::Value>) -> String {
    use trex::infer::build::Value;

    match value {
        Some(Value::One(v)) => json_or_null(Some(v)),
        Some(Value::Many(vs)) => {
            let items: Vec<String> = vs.iter().map(|v| json_or_null(Some(v))).collect();
            format!("[{}]", items.join(","))
        }
        None => "null".to_string(),
    }
}

/// `values`, one per field in order, as the JSON members of the fields
/// inside `parent` (at the top where it is `None`) by their keys: a field
/// holding others as an object of its `text` and theirs, `null` where it
/// is absent.
fn json_members(
    fields: &[trex::infer::build::Field],
    values: &[Option<trex::infer::build::Value>],
    parent: Option<usize>,
) -> Vec<String> {
    (0..fields.len())
        .filter(|&f| fields[f].parent == parent)
        .map(|f| {
            let holds = fields.iter().any(|g| g.parent == Some(f));
            let value = match &values[f] {
                Some(v) if holds => {
                    let mut members = vec![format!("\"text\":{}", json_value(Some(v)))];
                    members.extend(json_members(fields, values, Some(f)));
                    format!("{{{}}}", members.join(","))
                }
                v => json_value(v.as_ref()),
            };
            format!("\"{}\":{value}", crate::json_escape(fields[f].key()))
        })
        .collect()
}

/// The report on a built pattern as one JSON object. Lines and shapes are
/// numbered from one, as the report numbers them.
fn built_json(built: &trex::infer::build::Built) -> String {
    use trex::infer::build::Reach;

    let fields: Vec<String> = built
        .fields
        .iter()
        .map(|f| {
            let parent = f.parent.map(|p| built.fields[p].name.as_str());
            format!(
                "{{\"name\":\"{}\",\"accessor\":{},\"type\":{},\"starts_record\":{},\"list\":{},\"repeats\":{},\"parent\":{},\"template\":\"{}\"}}",
                crate::json_escape(&f.name),
                json_or_null(f.accessor.as_deref()),
                json_or_null(f.type_name.as_deref()),
                f.starts_record,
                f.list,
                f.repeats,
                json_or_null(parent),
                crate::json_escape(&f.template)
            )
        })
        .collect();
    let shapes: Vec<String> = built
        .shapes
        .iter()
        .map(|s| {
            let lines: Vec<String> = s.lines.iter().map(|r| (r + 1).to_string()).collect();
            let reach: Vec<String> = s
                .reach
                .iter()
                .enumerate()
                .map(|(f, r)| {
                    let name = crate::json_escape(&built.fields[f].name);
                    match r {
                        Reach::Marked => format!("{{\"field\":\"{name}\",\"reach\":\"marked\"}}"),
                        Reach::Anchored(from) => {
                            format!("{{\"field\":\"{name}\",\"reach\":\"anchored\",\"shape\":{}}}", from + 1)
                        }
                        Reach::Missing => format!("{{\"field\":\"{name}\",\"reach\":\"missing\"}}"),
                    }
                })
                .collect();
            format!(
                "{{\"pattern\":\"{}\",\"lines\":[{}],\"reach\":[{}]}}",
                crate::json_escape(&s.pattern),
                lines.join(","),
                reach.join(",")
            )
        })
        .collect();
    let rows: Vec<String> = built
        .rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let shape = match row.shape {
                Some(s) => (s + 1).to_string(),
                None => "null".to_string(),
            };
            let values = json_members(&built.fields, &row.values, None);
            format!(
                "{{\"line\":{},\"text\":\"{}\",\"shape\":{shape},\"values\":{{{}}}}}",
                r + 1,
                crate::json_escape(&row.text),
                values.join(",")
            )
        })
        .collect();
    let records: Vec<String> = built
        .records
        .iter()
        .map(|record| {
            let lines: Vec<String> = record.lines.iter().map(|r| (r + 1).to_string()).collect();
            let values = json_members(&built.fields, &record.values, None);
            format!("{{\"lines\":[{}],\"values\":{{{}}}}}", lines.join(","), values.join(","))
        })
        .collect();
    let declarations: Vec<String> =
        built.declarations.iter().map(|d| format!("\"{}\"", crate::json_escape(d))).collect();
    let suggestions: Vec<String> = built
        .suggestions
        .iter()
        .map(|(field, classes)| {
            let classes: Vec<String> = classes.iter().map(|c| format!("\"{}\"", crate::json_escape(c))).collect();
            format!("{{\"field\":\"{}\",\"classes\":[{}]}}", crate::json_escape(field), classes.join(","))
        })
        .collect();
    format!(
        "{{\"pattern\":\"{}\",\"format\":\"{}\",\"declarations\":[{}],\"suggestions\":[{}],\"fields\":[{}],\"shapes\":[{}],\"rows\":[{}],\"records\":[{}]}}",
        crate::json_escape(&built.pattern),
        crate::json_escape(&built.format()),
        declarations.join(","),
        suggestions.join(","),
        fields.join(","),
        shapes.join(","),
        rows.join(","),
        records.join(",")
    )
}

/// The lines of an example file, a carriage return before a newline left
/// off and a blank line left out.
fn example_lines(text: &[u8]) -> Vec<Vec<u8>> {
    text.split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
        .map(<[u8]>::to_vec)
        .collect()
}

/// `input` with `edits` applied, in order.
fn apply(input: &[u8], edits: &[trex::files::Edit]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut cursor = 0;
    for e in edits {
        out.extend_from_slice(&input[cursor..e.start]);
        out.extend_from_slice(&e.replacement);
        cursor = e.end;
    }
    out.extend_from_slice(&input[cursor..]);
    out
}

pub fn run_rewrite(args: &[String]) -> ExitCode {
    use std::io::Write;

    if args.len() < 2 {
        eprintln!(
            "usage: trex rewrite PATTERN TEMPLATE [FILE|DIR|-]... [--text STRING] [--lib FILE] [--in-place] [--dry-run] [--interactive] [-U] [-C N] [--hidden] [--no-ignore] [--binary] [--head N|--tail N|--lines A..B] [--record UNIT] [--follow]"
        );
        return ExitCode::FAILURE;
    }
    let pattern_src = &args[0];
    let template_src = &args[1];
    let mut text: Option<Vec<u8>> = None;
    let mut paths: Vec<String> = Vec::new();
    let mut windowing = Windowing::default();
    let mut record: Option<RecordSpec> = None;
    let mut in_place = false;
    let mut dry_run = false;
    let mut interactive = false;
    // `--explain` puts under each change what `scan --explain` puts under
    // each match, so a reviewer answering for a template can see what the
    // pattern actually read there.
    let mut explain = false;
    // `--show-skipped` names the places a `T` answer passed over. The count
    // is always reported; this asks for the list.
    let mut show_skipped = false;
    let mut backend = trex::Backend::Auto;
    let mut walk = WalkOptions::default();
    let mut binary = false;
    let mut shapes = trex::ShapeSet::new();
    // The lines of context a dry run's diff shows around each change.
    let mut context = DIFF_CONTEXT;

    let mut i = 2;
    while i < args.len() {
        match windowing.take(args, &mut i) {
            Some(Ok(())) => {
                i += 1;
                continue;
            }
            Some(Err(e)) => {
                eprintln!("trex: {e}");
                return ExitCode::FAILURE;
            }
            None => {}
        }
        match args[i].as_str() {
            "--lib" => {
                i += 1;
                let Some(path) = args.get(i) else {
                    eprintln!("trex: --lib needs a pattern file");
                    return ExitCode::FAILURE;
                };
                if let Err(e) = declare_file(&mut shapes, path) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            flag @ ("--record" | "--record-start" | "--record-span") => {
                if let Err(e) = take_record(flag, args, &mut i, &mut record) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--in-place" => in_place = true,
            "--dry-run" => dry_run = true,
            "--explain" => explain = true,
            "--show-skipped" => show_skipped = true,
            "-i" | "--interactive" => interactive = true,
            // Every change applied without a question: what a review that
            // accepted all would have done.
            "-U" | "--update-all" => {
                in_place = true;
                interactive = false;
            }
            "--gpu" => backend = trex::Backend::Gpu,
            "--cpu" | "--nogpu" => backend = trex::Backend::Cpu,
            "--hidden" => walk.hidden = true,
            "--no-ignore" => walk.no_ignore = true,
            "--binary" => binary = true,
            "-C" | "--context" => {
                i += 1;
                context = match count_arg("-C", args.get(i)) {
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("trex: {e}");
                        return ExitCode::FAILURE;
                    }
                };
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                text = Some(v.clone().into_bytes());
            }
            path => paths.push(path.to_string()),
        }
        i += 1;
    }

    if interactive && (in_place || dry_run) {
        eprintln!(
            "trex rewrite: --interactive reviews each change and takes no --in-place or --dry-run; -U applies every change"
        );
        return ExitCode::FAILURE;
    }
    let pattern = match trex::parser::parse_with_shapes(pattern_src, &shapes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("trex: pattern error at byte {}: {}", e.pos, e.msg);
            return ExitCode::FAILURE;
        }
    };
    let template = match trex::Template::parse(template_src, &pattern.capture_names()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("trex: template error at byte {}: {}", e.pos, e.msg);
            return ExitCode::FAILURE;
        }
    };
    let unit = match window_unit("rewrite", &windowing, record.as_ref(), &shapes) {
        Ok(unit) => unit,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    if windowing.follow && (in_place || dry_run || interactive || backend == trex::Backend::Gpu) {
        eprintln!(
            "trex rewrite: --follow writes what a growing file gains to the standard output, rewritten by the stream scanner on the CPU; a file another process appends to is not rewritten in place, so it takes no --in-place, --dry-run, --interactive or --gpu"
        );
        return ExitCode::FAILURE;
    }

    let a_directory = paths.iter().any(|p| std::path::Path::new(p).is_dir());
    let (mut sources, walk_errors) = match text {
        Some(_) if !paths.is_empty() => {
            eprintln!("trex rewrite: --text and a path cannot both be given");
            return ExitCode::FAILURE;
        }
        Some(_) => (Vec::new(), Vec::new()),
        None => collect(&paths, &walk),
    };
    if text.is_none() && sources.is_empty() && paths.is_empty() {
        sources.push(Source::Stdin);
    }
    for e in &walk_errors {
        eprintln!("trex: {e}");
    }

    // One input and no request to edit it in place: the rewritten bytes go
    // to the standard output, as they always have. Where a window is named,
    // the window alone is read, rewritten and printed.
    if !in_place && !dry_run && !interactive {
        let followed = match followed_file("rewrite", &windowing, &text, &sources) {
            Ok(followed) => followed,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        // The standard output carries text, never line numbers, so a tail is
        // read without counting the lines ahead of it, and placed only for a
        // follow, which continues its offsets.
        let asked = trex::window::Asked { offsets: followed.is_some(), binary, ..trex::window::Asked::default() };
        let part = match one_input("rewrite", "rewrites", &text, &sources, asked, windowing.select, &unit) {
            Ok(part) => part,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        if let Some(path) = followed {
            let mut edits =
                |held: &[u8], spans: &[trex::Span]| trex::rewrite::edits_at(&pattern, &template, held, &shapes, spans);
            return crate::cli_follow::follow_edit("rewrite", path, &part, &pattern, &shapes, &mut edits);
        }
        let input = part.text;
        // Declared shapes and kinds decide token boundaries, so a rewrite
        // under them lexes the way the scan that finds the matches does,
        // and takes no device: the kernel lexes for itself and knows
        // nothing of a shape a pattern file declared.
        let (out, used) = if shapes.is_empty() {
            trex::rewrite_with_backend(&pattern, &template, &input, backend)
        } else {
            (trex::rewrite::rewrite_with_shapes(&pattern, &template, &input, &shapes), trex::BackendUsed::Cpu)
        };
        match (backend, used) {
            (trex::Backend::Gpu, trex::BackendUsed::Cpu) => eprintln!(
                "gpu: not applicable (pattern outside device subset, no device, or built without --features gpu); using CPU"
            ),
            (_, trex::BackendUsed::Gpu) => eprintln!("gpu: device-matched rewrite"),
            (_, trex::BackendUsed::Split) => {
                eprintln!("auto: rewrite matched across the cores and the GPU device");
            }
            _ => {}
        }
        if let Err(e) = std::io::stdout().write_all(&out) {
            eprintln!("trex: cannot write the standard output: {e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    if text.is_some() {
        eprintln!("trex: --in-place, --dry-run and --interactive need a FILE (not --text)");
        return ExitCode::FAILURE;
    }
    if (in_place || interactive) && sources.contains(&Source::Stdin) {
        eprintln!("trex: --in-place and --interactive cannot write the standard input");
        return ExitCode::FAILURE;
    }

    let edits_of = |_: &Source, piece: Piece<'_>| {
        if shapes.is_empty() {
            trex::rewrite::edits(&pattern, &template, piece.text)
        } else {
            trex::rewrite::edits_with_shapes(&pattern, &template, piece.text, &shapes)
        }
    };
    let how = Editing {
        failed: !walk_errors.is_empty(),
        a_directory,
        binary,
        verb: "rewrites",
        dry_run,
        context,
        restrict: Restrict::of(&windowing, &unit),
    };
    if interactive {
        // An explainer is built for the input the span belongs to, once per
        // change rather than once per file: a review asks about few changes
        // out of a file's many, so building one per file would analyze inputs
        // the reviewer never reaches.
        let explain_one = |input: &[u8], start: usize, end: usize| {
            let explainer = trex::explain::Explainer::new(&pattern, input, &shapes);
            let m = trex::Match::plain(start, end);
            print_explanation(&explainer.explain(&m, "the rewrite's own scan"));
        };
        let explaining: Option<&ExplainSpan<'_>> =
            explain.then_some(&explain_one as &ExplainSpan<'_>);
        return review_sources(&sources, how, explaining, show_skipped, edits_of);
    }
    // A rewrite carries nothing from one input to the next, so the order the
    // cores finish in is not visible in what it writes.
    edit_sources(&sources, how, edits_of)
}

/// Take the record flag `flag` at `args[*i]` into `record`, its value the
/// next argument, moving `i` past it.
pub(crate) fn take_record(
    flag: &str,
    args: &[String],
    i: &mut usize,
    record: &mut Option<RecordSpec>,
) -> Result<(), String> {
    *i += 1;
    let Some(v) = args.get(*i) else {
        let wants = if flag == "--record" {
            "line, paragraph, file, period, seam, bind, bind:Q, auto, texture, shape, block, unit or unit:ROLE"
        } else {
            "a pattern"
        };
        return Err(format!("{flag} needs {wants}"));
    };
    if record.is_some() {
        return Err(format!("{flag} names a second record definition; a window counts one unit"));
    }
    *record = Some(match flag {
        "--record" => RecordSpec::Named(v.clone()),
        "--record-start" => RecordSpec::Start(v.clone()),
        _ => RecordSpec::Span(v.clone()),
    });
    Ok(())
}

/// The unit a rewrite's or a redaction's window counts: the record the
/// record flags name, a line where they name none. Refused where the record
/// flags are given with no window to count, or `--follow` names a window
/// that ends before the file does.
fn window_unit(
    command: &str,
    windowing: &Windowing,
    record: Option<&RecordSpec>,
    shapes: &trex::ShapeSet,
) -> Result<trex::records::RecordUnit, String> {
    if record.is_some() && windowing.select.is_none() {
        return Err(format!(
            "trex {command}: --record, --record-start and --record-span name the unit --head, --tail and --lines count, and none was given"
        ));
    }
    if let Some(why) = windowing.follow_refusal() {
        return Err(format!("trex {command}: {why}"));
    }
    record_unit_of(record, shapes).map_err(|e| format!("trex: {e}"))
}

/// The file `--follow` follows for a command writing one input to the
/// standard output, where it was given: `None` without the flag, and a
/// refusal for inline text or the standard input, which have no name to
/// follow.
fn followed_file<'s>(
    command: &str,
    windowing: &Windowing,
    text: &Option<Vec<u8>>,
    sources: &'s [Source],
) -> Result<Option<&'s std::path::Path>, String> {
    if !windowing.follow {
        return Ok(None);
    }
    match (text, sources) {
        (None, [Source::File(path)]) => Ok(Some(path.as_path())),
        (Some(_), _) => Err(format!("trex {command}: --follow follows a file by name and takes no --text")),
        (None, [Source::Stdin]) => Err(format!(
            "trex {command}: --follow follows a file by name, and the standard input is read as it arrives already; name the file"
        )),
        (None, many) => Err(format!(
            "trex {command}: --follow writes one file to the standard output, and {} inputs were given",
            many.len()
        )),
    }
}

/// Prints the explanation of one span of one input: the whole input, the
/// span's start, and its end.
pub(crate) type ExplainSpan<'a> = dyn Fn(&[u8], usize, usize) + 'a;

/// Review the edits `edits_of` makes to every source, one at a time: each
/// is shown as its unified diff and, at the prompt, accepted, skipped or
/// given another replacement; `a` accepts it and every one after; `q`, or
/// the end of the answers, ends the session. The accepted edits are written
/// once the session has seen the last one, so a session that ends early
/// leaves every file as it was. The answers are read from the standard
/// input, the diffs and the prompts go to the standard output, and what was
/// written is reported on the standard error as `--in-place` reports it.
/// The walk's failure, `--binary`, the diff's context and the window each
/// edit is confined to are read from `how`, and `a_directory` with the
/// sources' count says whether one named file holding a NUL byte is refused
/// aloud; `dry_run` does not apply to a review.
pub(crate) fn review_sources(
    sources: &[Source],
    how: Editing<'_>,
    // Prints the explanation of one span of one input, under its diff. A
    // closure rather than an explainer, because an explainer is built per
    // input and a review holds several: the caller knows the pattern and can
    // build one where it is wanted, and this function knows only the edits.
    explain_span: Option<&ExplainSpan<'_>>,
    show_skipped: bool,
    edits_of: impl Fn(&Source, Piece<'_>) -> Vec<trex::files::Edit>,
) -> ExitCode {
    use std::io::{BufRead, Write};

    let Editing { failed, a_directory, binary, verb, context, restrict, .. } = how;
    let mut failed = failed;
    // One named file holding a NUL byte is refused aloud, as a scan or an
    // edit to the standard output refuses it; a tree or a list of files
    // passes over one without a word.
    let lone = !a_directory && sources.len() == 1;
    let mut refused = false;
    // Every file's bytes with the edits of its text, read before the first
    // question, so the session knows how many it holds.
    let mut queue: Vec<(&Source, Vec<u8>, Vec<trex::files::Edit>)> = Vec::new();
    for src in sources {
        let raw = match read_source(src) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("trex: cannot read {}: {e}", src.name());
                failed = true;
                continue;
            }
        };
        if !binary && is_binary(&raw) {
            if lone {
                eprintln!("trex: {} holds a NUL byte and is binary; --binary {verb} it", src.name());
                failed = true;
                refused = true;
            }
            continue;
        }
        let edits = edits_within(&trex::encoding::text_of(&raw), restrict, |piece| edits_of(src, piece));
        if !edits.is_empty() {
            queue.push((src, raw, edits));
        }
    }
    let total: usize = queue.iter().map(|(_, _, edits)| edits.len()).sum();
    if total == 0 {
        // A refused file was never searched, so there is no match to deny.
        if !refused {
            println!("no match");
        }
        return if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    }
    let stdin = std::io::stdin();
    let mut answers = stdin.lock();
    let mut kept: Vec<Vec<trex::files::Edit>> = queue.iter().map(|_| Vec::new()).collect();
    let mut all = false;
    let mut seen = 0usize;
    // The silhouette of every change, in the order they are offered, so the
    // prompt can say how many later ones an answer for the template would
    // carry. Read once: the lex is per change and a review of a thousand
    // would otherwise pay for it at every prompt.
    let shapes: Vec<Vec<Vec<trex::token::TokenKind>>> = queue
        .iter()
        .map(|(_, raw, edits)| {
            let input = trex::encoding::text_of(raw);
            edits.iter().map(|e| trex::templates::silhouette(&input, e.start..e.end)).collect()
        })
        .collect();
    // The templates answered for, and what each answer was. A later change
    // whose silhouette is here is decided without being offered.
    let mut answered: Vec<(Vec<trex::token::TokenKind>, bool)> = Vec::new();
    // What `T` passed over, kept whether or not it will be printed: the flag
    // decides the printing, and a reviewer who did not ask still quit knowing
    // the count.
    let mut skipped_by_template: Vec<(String, usize, usize)> = Vec::new();
    for (k, (src, raw, edits)) in queue.iter().enumerate() {
        let input = trex::encoding::text_of(raw);
        for (e, edit) in edits.iter().enumerate() {
            seen += 1;
            if all {
                kept[k].push(edit.clone());
                continue;
            }
            let shape = &shapes[k][e];
            // A template already answered for decides this change without
            // offering it, which is the whole of what `t` and `T` buy.
            if let Some((_, accepted)) = answered.iter().find(|(s, _)| s == shape) {
                if *accepted {
                    kept[k].push(edit.clone());
                } else {
                    skipped_by_template.push((src.name(), edit.start, edit.end));
                }
                continue;
            }
            // How many changes after this one share its silhouette, which is
            // what an answer for the template would carry.
            let later = shapes
                .iter()
                .enumerate()
                .flat_map(|(j, per_file)| {
                    per_file.iter().enumerate().map(move |(i, s)| (j, i, s))
                })
                .filter(|&(j, i, s)| (j > k || (j == k && i > e)) && s == shape)
                .count();
            print!("{}", trex::files::unified_diff(&src.name(), &input, std::slice::from_ref(edit), context));
            if let Some(explain) = explain_span {
                explain(&input, edit.start, edit.end);
            }
            println!(
                "  template: {} ({later} later change{} share{} it)",
                trex::templates::silhouette_name(shape),
                if later == 1 { "" } else { "s" },
                if later == 1 { "s" } else { "" }
            );
            loop {
                print!("[{seen}/{total}] accept, skip or edit this change, all of this template, none of it, accept all, or quit [y/n/e/t/T/a/q]? ");
                if let Err(e) = std::io::stdout().flush() {
                    eprintln!("trex rewrite: cannot write the standard output: {e}");
                    return ExitCode::FAILURE;
                }
                let mut line = String::new();
                match answers.read_line(&mut line) {
                    Ok(0) => {
                        println!();
                        eprintln!("trex rewrite: the answers ended; every file is left as it was");
                        return ExitCode::SUCCESS;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("trex rewrite: cannot read an answer: {e}");
                        return ExitCode::FAILURE;
                    }
                }
                match line.trim().to_ascii_lowercase().as_str() {
                    "y" | "yes" => {
                        kept[k].push(edit.clone());
                        break;
                    }
                    "n" | "no" => break,
                    "e" | "edit" => match edited_replacement(&edit.replacement, &mut answers) {
                        Ok(replacement) => {
                            kept[k].push(trex::files::Edit { start: edit.start, end: edit.end, replacement });
                            break;
                        }
                        Err(e) => eprintln!("trex rewrite: {e}"),
                    },
                    // `t` and `T` are case sensitive, so the answer is read
                    // from the untouched line rather than the folded one: a
                    // reviewer typing `T` means the opposite of `t`, and
                    // folding the case would silently accept what they meant
                    // to skip.
                    _ if line.trim() == "t" => {
                        kept[k].push(edit.clone());
                        answered.push((shape.clone(), true));
                        break;
                    }
                    _ if line.trim() == "T" => {
                        answered.push((shape.clone(), false));
                        skipped_by_template.push((src.name(), edit.start, edit.end));
                        break;
                    }
                    "a" | "all" => {
                        kept[k].push(edit.clone());
                        all = true;
                        break;
                    }
                    "q" | "quit" => {
                        eprintln!("trex rewrite: quit; every file is left as it was");
                        return ExitCode::SUCCESS;
                    }
                    other => println!(
                        "{other:?} is not an answer: y accepts, n skips, e edits, t takes every change of this template, T skips them, a accepts all, q quits"
                    ),
                }
            }
        }
    }
    // What a template answer passed over. The count is always said, because a
    // reviewer who answered for a template should know how much it carried;
    // the places are said only where they were asked for, since a list at the
    // end of a long review is where a reviewer is least likely to read one.
    if !skipped_by_template.is_empty() {
        let n = skipped_by_template.len();
        let plural = if n == 1 { "" } else { "s" };
        eprintln!("trex rewrite: {n} change{plural} skipped by a template answer");
        if show_skipped {
            for (name, start, end) in &skipped_by_template {
                eprintln!("  {name} [{start}..{end}]");
            }
        }
    }
    for ((src, raw, _), kept) in queue.iter().zip(&kept) {
        if kept.is_empty() {
            continue;
        }
        let Source::File(path) = src else {
            eprintln!("trex: {} is not a file", src.name());
            failed = true;
            continue;
        };
        match std::fs::write(path, trex::files::written(raw, kept, trex::files::ReadAs::Decoded)) {
            Ok(()) => {
                let plural = if kept.len() == 1 { "" } else { "s" };
                eprintln!("{}: {} replacement{plural}", src.name(), kept.len());
            }
            Err(e) => {
                eprintln!("trex: cannot write {}: {e}", src.name());
                failed = true;
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// The replacement the user gives an edit under review: through the editor
/// `VISUAL` or `EDITOR` names, opened on a file holding the proposed
/// replacement and read back with its final newline dropped, or as one line
/// typed at the prompt where neither is set.
fn edited_replacement(proposed: &[u8], answers: &mut impl std::io::BufRead) -> Result<Vec<u8>, String> {
    use std::io::Write;

    let editor = ["VISUAL", "EDITOR"].iter().find_map(|name| std::env::var_os(name).filter(|v| !v.is_empty()));
    let Some(editor) = editor else {
        print!("replacement: ");
        std::io::stdout().flush().map_err(|e| format!("cannot write the standard output: {e}"))?;
        let mut line = String::new();
        match answers.read_line(&mut line) {
            Ok(0) => return Err("the answers ended before a replacement was given".to_string()),
            Ok(_) => {}
            Err(e) => return Err(format!("cannot read the replacement: {e}")),
        }
        let line = line.strip_suffix('\n').unwrap_or(&line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        return Ok(line.as_bytes().to_vec());
    };
    let editor = editor.to_string_lossy().into_owned();
    let mut words = editor.split_whitespace();
    let Some(program) = words.next() else {
        return Err("the editor variable holds only whitespace".to_string());
    };
    // Under one parent with the rest of the crate's scratch, so what trex
    // leaves in a shared temp directory is one place to look rather than a name
    // per caller. `write` does not make a parent, so the parent is made first.
    let root = std::env::temp_dir().join("trex");
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("cannot make {}: {e}", root.display()))?;
    let path = root.join(format!("edit-{}.txt", std::process::id()));
    std::fs::write(&path, proposed).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let status = std::process::Command::new(program)
        .args(words)
        .arg(&path)
        .status()
        .map_err(|e| format!("cannot run the editor {program}: {e}"))?;
    if !status.success() {
        return Err(format!("the editor {program} exited with {status}; the change is asked about again"));
    }
    let edited = std::fs::read(&path).map_err(|e| format!("cannot read {} back: {e}", path.display()))?;
    if let Err(e) = std::fs::remove_file(&path) {
        eprintln!("trex rewrite: cannot remove {}: {e}", path.display());
    }
    let edited = edited.strip_suffix(b"\r\n").or_else(|| edited.strip_suffix(b"\n")).unwrap_or(&edited);
    Ok(edited.to_vec())
}

/// The one input a command writing to the standard output reads: the inline
/// text, or the single source, decoded, cut to the window `select` names in
/// `unit` where it names one, read as `asked` says. A directory or several
/// inputs have no one output to go to, so they are refused naming the flags
/// that take them; `verb` is what `--binary` would let the command do to a
/// file that holds a NUL byte.
fn one_input(
    command: &str,
    verb: &str,
    text: &Option<Vec<u8>>,
    sources: &[Source],
    asked: trex::window::Asked,
    select: Option<trex::window::Select>,
    unit: &trex::records::RecordUnit,
) -> Result<Part, String> {
    match (text, sources) {
        (Some(t), _) => Ok(Part::of_text(t.clone(), select, unit)),
        (None, [one]) => {
            let part = match read_part(one, select, unit, asked) {
                Ok(part) => part,
                Err(e) => return Err(format!("trex: cannot read {}: {e}", one.name())),
            };
            if !asked.binary && part.binary {
                return Err(format!(
                    "trex: {} holds a NUL byte and is binary; --binary {verb} it",
                    one.name()
                ));
            }
            Ok(part)
        }
        (None, []) => Err(format!("trex {command}: no input to {command}")),
        (None, many) => Err(format!(
            "trex {command}: {} inputs need --in-place or --dry-run; the standard output holds one",
            many.len()
        )),
    }
}

/// How an edit of many sources runs: what the walk already reported, whether
/// the report names each file changed, which inputs it reads, and what it
/// writes.
#[derive(Clone, Copy)]
pub(crate) struct Editing<'a> {
    /// Whether the walk that found the sources already reported an error.
    pub(crate) failed: bool,
    pub(crate) a_directory: bool,
    /// `--binary`: a file holding a NUL byte is edited all the same.
    pub(crate) binary: bool,
    /// What `--binary` lets the command do to a file holding a NUL byte, as
    /// the notice refusing one named file says it: `rewrites`, `redacts` or
    /// `fixes`.
    pub(crate) verb: &'a str,
    /// `--dry-run`: the unified diff, and nothing written.
    pub(crate) dry_run: bool,
    /// The lines of context the diff shows around each change.
    pub(crate) context: usize,
    /// The part of each file an edit is confined to, where a window was
    /// named; the file keeps every other byte as it was.
    pub(crate) restrict: Option<Restrict<'a>>,
}

/// Apply the edits `edits_of` makes to every source: in place, or as the
/// unified diff a dry run prints. The files are read and edited across the
/// cores and reported in path order; one that cannot be read or written is
/// reported and the rest are still edited.
pub(crate) fn edit_sources(
    sources: &[Source],
    how: Editing<'_>,
    edits_of: impl Fn(&Source, Piece<'_>) -> Vec<trex::files::Edit> + Sync,
) -> ExitCode {
    let mut changes: Vec<Change> = sources.iter().map(|_| Change::Pending).collect();
    across_cores(&mut changes, |k, slot| {
        let src = &sources[k];
        *slot = change_of(src, how, |piece| edits_of(src, piece));
    });
    report_changes(sources, &changes, how.failed, how.a_directory, how.verb)
}

/// As [`edit_sources`], one source after another on this thread, for edits
/// that carry state from one source to the next and so need the order the
/// sources are in rather than the order the cores finish: a pseudonym
/// numbered by when its value was first seen is a different name under a
/// different schedule, and a redaction that names the same value differently
/// on two runs cannot be diffed against itself.
pub(crate) fn edit_sources_in_order(
    sources: &[Source],
    how: Editing<'_>,
    mut edits_of: impl FnMut(&Source, Piece<'_>) -> Vec<trex::files::Edit>,
) -> ExitCode {
    let changes: Vec<Change> = sources.iter().map(|src| change_of(src, how, |piece| edits_of(src, piece))).collect();
    report_changes(sources, &changes, how.failed, how.a_directory, how.verb)
}

/// What editing one source came to: unreadable, binary and so skipped,
/// unchanged, the diff a dry run prints, or written with the edits `edits`
/// makes of its decoded text, confined to its window where one was named.
fn change_of(src: &Source, how: Editing<'_>, edits: impl FnOnce(Piece<'_>) -> Vec<trex::files::Edit>) -> Change {
    let Editing { binary, dry_run, context, restrict, .. } = how;
    let raw = match read_source(src) {
        Ok(b) => b,
        Err(e) => return Change::Failed(format!("cannot read {}: {e}", src.name())),
    };
    if !binary && is_binary(&raw) {
        return Change::Binary;
    }
    let input = trex::encoding::text_of(&raw);
    let edits = edits_within(&input, restrict, edits);
    if edits.is_empty() {
        return Change::Unchanged;
    }
    if dry_run {
        return Change::Diff(trex::files::unified_diff(&src.name(), &input, &edits, context));
    }
    let out = trex::files::written(&raw, &edits, trex::files::ReadAs::Decoded);
    let Source::File(path) = src else {
        return Change::Failed(format!("{} is not a file", src.name()));
    };
    match std::fs::write(path, &out) {
        Ok(()) => Change::Written(edits.len()),
        Err(e) => Change::Failed(format!("cannot write {}: {e}", src.name())),
    }
}

/// Report each source's change in path order, and say whether the run
/// failed: the walk that found them failed, or one could not be read or
/// written.
fn report_changes(sources: &[Source], changes: &[Change], failed: bool, a_directory: bool, verb: &str) -> ExitCode {
    // One named file is rewritten quietly; a tree or a list of files reports
    // each file it changed.
    let announce = a_directory || sources.len() > 1;
    let mut failed = failed;
    for (src, change) in sources.iter().zip(changes) {
        match change {
            // One named file holding a NUL byte is refused aloud, as a scan
            // or an edit to the standard output refuses it; a tree or a list
            // of files passes over one without a word.
            Change::Binary if !announce => {
                eprintln!("trex: {} holds a NUL byte and is binary; --binary {verb} it", src.name());
                failed = true;
            }
            Change::Pending | Change::Binary | Change::Unchanged => {}
            Change::Failed(e) => {
                eprintln!("trex: {e}");
                failed = true;
            }
            Change::Diff(d) => print!("{d}"),
            Change::Written(n) => {
                if announce {
                    let plural = if *n == 1 { "" } else { "s" };
                    eprintln!("{}: {n} replacement{plural}", src.name());
                }
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
