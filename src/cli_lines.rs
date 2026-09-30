//! `trex head`, `trex tail` and `trex lines`: an input's first records, its
//! last, or a range of them, found with the vector newline search from the
//! end they sit at and read no further; numbered as the input numbers them
//! where `-n` asks; and, for a tail or a range that runs to the end,
//! followed as each file grows (`-f`).
//!
//! The text printed is the input's own, decoded where a byte order mark
//! declares UTF-16 or UTF-32. Several inputs are headed `==> name <==` as the
//! coreutils head and tail head them.

use std::io::Write;
use std::process::ExitCode;

use trex::encoding::Incremental;
use trex::files::{LineIndex, Source, WalkOptions, collect};
use trex::follow::{Followed as Change, Follower};
use trex::window::{Asked, Select};

use crate::cli_files::{RecordSpec, painter_for, record_unit_of, take_record};
use crate::cli_window::read_part;

/// Which part of each input a listing prints.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Listing {
    /// `trex head`: the first records.
    Head,
    /// `trex tail`: the last records.
    Tail,
    /// `trex lines`: a range of them.
    Lines,
}

impl Listing {
    fn name(self) -> &'static str {
        match self {
            Listing::Head => "head",
            Listing::Tail => "tail",
            Listing::Lines => "lines",
        }
    }
}

/// The usage of a listing, on the standard error.
fn usage(which: Listing) {
    let name = which.name();
    let selects = match which {
        Listing::Head | Listing::Tail => "N",
        Listing::Lines => "A..B",
    };
    eprintln!(
        "usage: trex {name} {selects} [FILE|DIR|-]... [-n] [--record UNIT|--record-start PATTERN|--record-span PATTERN] [-H|--no-filename] [--binary] [--hidden] [--no-ignore] [--color WHEN] [--colors SPEC]..."
    );
    match which {
        Listing::Head => {
            eprintln!("  the first N lines of each input, or records of the --record unit; --lines A..B in place of N reads a range");
        }
        Listing::Tail => {
            eprintln!("  the last N lines of each input, or records of the --record unit; --lines A..B in place of N reads a range");
            eprintln!("  -f, --follow     print what each file gains as it grows, through truncation and rotation");
        }
        Listing::Lines => {
            eprintln!("  lines A through B of each input, A.. to its end, ..B from its start, or A alone");
            eprintln!("  -f, --follow     for A.., print what each file gains as it grows");
        }
    }
    eprintln!("  -n, --line-number  each line after its number in the input, as N:text");
}

/// A count of records, as a listing takes it.
fn count_of(name: &str, value: &str) -> Result<usize, String> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("trex {name}: the number of records comes first, as in `trex {name} 10 FILE`, not {value:?}"));
    }
    value.parse::<usize>().map_err(|e| format!("trex {name}: {value}: {e}"))
}

/// `trex head`, `trex tail` or `trex lines` over `args`.
pub fn run_listing(which: Listing, args: &[String]) -> ExitCode {
    let name = which.name();
    if args.is_empty() {
        usage(which);
        return ExitCode::FAILURE;
    }
    let mut positionals: Vec<String> = Vec::new();
    let mut count: Option<usize> = None;
    let mut range: Option<Select> = None;
    let mut record: Option<RecordSpec> = None;
    let mut numbered = false;
    let mut follow = false;
    let mut with_filename: Option<bool> = None;
    let mut binary = false;
    let mut walk = WalkOptions::default();
    let mut color: Option<String> = None;
    let mut colors: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let (flag, attached) = match arg.split_once('=') {
            Some((f, v)) if matches!(f, "--lines" | "--color" | "--colors") => (f, Some(v.to_string())),
            _ => (arg, None),
        };
        let mut value = |what: &str| -> Result<String, String> {
            if let Some(v) = &attached {
                return Ok(v.clone());
            }
            i += 1;
            args.get(i).cloned().ok_or_else(|| format!("trex {name}: {flag} needs {what}"))
        };
        let taken = match flag {
            "-n" | "--line-number" => {
                numbered = true;
                Ok(())
            }
            "-f" | "--follow" => {
                follow = true;
                Ok(())
            }
            "-H" | "--with-filename" => {
                with_filename = Some(true);
                Ok(())
            }
            "--no-filename" => {
                with_filename = Some(false);
                Ok(())
            }
            "--binary" => {
                binary = true;
                Ok(())
            }
            "--hidden" => {
                walk.hidden = true;
                Ok(())
            }
            "--no-ignore" => {
                walk.no_ignore = true;
                Ok(())
            }
            "--color" => value("always, never, auto, 16, 256 or truecolor").map(|v| color = Some(v)),
            "--colors" => value("a spec, as in line:fg:green").map(|v| colors.push(v)),
            "--lines" if which != Listing::Lines => value("a range, as in 100..200, 100.. or ..200").and_then(|v| {
                Select::parse_range(&v).map(|s| range = Some(s)).map_err(|e| format!("trex {name}: --lines {e}"))
            }),
            "--record" | "--record-start" | "--record-span" => {
                take_record(flag, args, &mut i, &mut record).map_err(|e| format!("trex {name}: {e}"))
            }
            "-h" | "--help" => {
                usage(which);
                return ExitCode::FAILURE;
            }
            // The coreutils spelling of a count, `-20`.
            _ if which != Listing::Lines
                && count.is_none()
                && positionals.is_empty()
                && arg.len() > 1
                && arg.starts_with('-')
                && arg[1..].bytes().all(|b| b.is_ascii_digit()) =>
            {
                count_of(name, &arg[1..]).map(|n| count = Some(n))
            }
            _ => {
                positionals.push(arg.to_string());
                Ok(())
            }
        };
        if let Err(e) = taken {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
        i += 1;
    }

    // What each input is cut to: the range for `lines` and for `--lines`,
    // the count otherwise, each the first argument that is not a flag.
    let select = match (which, range, count) {
        (Listing::Lines, ..) => {
            if positionals.is_empty() {
                eprintln!("trex lines: a range comes first, as in `trex lines 100..200 FILE`");
                return ExitCode::FAILURE;
            }
            match Select::parse_range(&positionals.remove(0)) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("trex lines: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        (_, Some(r), None) => r,
        (_, Some(_), Some(_)) => {
            eprintln!("trex {name}: --lines names the range in place of the number of records; give one");
            return ExitCode::FAILURE;
        }
        (_, None, given) => {
            let n = match given {
                Some(n) => n,
                None => {
                    if positionals.is_empty() {
                        usage(which);
                        return ExitCode::FAILURE;
                    }
                    match count_of(name, &positionals.remove(0)) {
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("{e}");
                            return ExitCode::FAILURE;
                        }
                    }
                }
            };
            if which == Listing::Head { Select::Head(n) } else { Select::Tail(n) }
        }
    };
    if follow && !select.runs_to_end() {
        eprintln!(
            "trex {name}: -f reads on as a file grows, past the end of what is printed, and a head or a range with a last record ends before the file does; follow a tail or a range A.."
        );
        return ExitCode::FAILURE;
    }
    let unit = match record_unit_of(record.as_ref(), &trex::ShapeSet::new()) {
        Ok(unit) => unit,
        Err(e) => {
            eprintln!("trex {name}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let painter = match painter_for(color.as_deref(), &colors) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("trex: {e}");
            return ExitCode::FAILURE;
        }
    };

    let a_directory = positionals.iter().any(|p| std::path::Path::new(p).is_dir());
    let (mut sources, walk_errors) = collect(&positionals, &walk);
    if sources.is_empty() && positionals.is_empty() {
        sources.push(Source::Stdin);
    }
    for e in &walk_errors {
        eprintln!("trex: {e}");
    }
    if follow && sources.contains(&Source::Stdin) {
        eprintln!(
            "trex {name}: -f follows a file by name, and the standard input is read as it arrives already; name the file"
        );
        return ExitCode::FAILURE;
    }
    let headed = with_filename.unwrap_or(a_directory || sources.len() > 1);
    // Lines print with their numbers where -n asks and never with offsets,
    // so a tail is not placed.
    let asked = Asked { numbers: numbered, binary, ..Asked::default() };
    let mut failed = !walk_errors.is_empty();
    let mut printed_any = false;
    let mut followees: Vec<Followee> = Vec::new();
    for src in &sources {
        let label = src.name();
        let part = match read_part(src, Some(select), &unit, asked) {
            Ok(part) => part,
            Err(e) => {
                eprintln!("trex: cannot read {label}: {e}");
                failed = true;
                continue;
            }
        };
        if part.binary && !binary {
            eprintln!("trex: {label} holds a NUL byte and is binary; --binary prints it");
            failed = true;
            continue;
        }
        if headed {
            if printed_any {
                crate::out::line("");
            }
            crate::out::line(&trex::report::listing_header(&label, &painter));
        }
        printed_any = true;
        // A line the window ends in the middle of is still being written
        // where the file is followed; numbered, it waits for its newline so
        // it prints under one number.
        let complete = if follow && numbered {
            part.text.iter().rposition(|&b| b == b'\n').map_or(0, |at| at + 1)
        } else {
            part.text.len()
        };
        let shown = &part.text[..complete];
        if numbered {
            let index = LineIndex::new(shown).within(part.byte_base, part.line_base);
            trex::report::numbered_lines(shown, &index, &painter, &mut |l| crate::out::line(l));
        } else if let Err(e) = write_raw(shown) {
            eprintln!("trex: cannot write the standard output: {e}");
            return ExitCode::FAILURE;
        }
        if follow && let Source::File(path) = src {
            let printed_lines = trex::byte_simd::count_byte(shown, b'\n');
            followees.push(Followee {
                path: path.clone(),
                label,
                offset: part.input_len,
                decoder: Incremental::after(part.encoding),
                next_line: part.line_base.map(|before| before + printed_lines + 1),
                pending: part.text[complete..].to_vec(),
            });
        }
    }
    if !follow || followees.is_empty() {
        return if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    }
    follow_listing(followees, numbered, headed, &painter)
}

/// Write `bytes` to the standard output as they are.
fn write_raw(bytes: &[u8]) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    out.write_all(bytes)?;
    out.flush()
}

/// One file a listing follows: its name as given, where following it picks
/// up, how its bytes decode, and, numbered, the number its next line takes,
/// which an unnumbered listing never counted, and the line it is still
/// receiving.
struct Followee {
    path: std::path::PathBuf,
    label: String,
    offset: usize,
    decoder: Incremental,
    next_line: Option<usize>,
    pending: Vec<u8>,
}

impl Followee {
    /// Print what the file gained, `text` decoded: as it arrives, or, where
    /// the lines are numbered, each line once its newline arrives.
    fn print(&mut self, text: &[u8], numbered: bool, painter: &trex::paint::Painter) -> std::io::Result<()> {
        if !numbered {
            return write_raw(text);
        }
        self.pending.extend_from_slice(text);
        let Some(last) = self.pending.iter().rposition(|&b| b == b'\n') else {
            return Ok(());
        };
        let lines: Vec<u8> = self.pending.drain(..=last).collect();
        self.number(&lines, painter);
        Ok(())
    }

    /// Print `lines`, whole lines of the file, numbered from its next line.
    fn number(&mut self, lines: &[u8], painter: &trex::paint::Painter) {
        let index = LineIndex::new(lines).within(Some(0), self.next_line.map(|next| next - 1));
        trex::report::numbered_lines(lines, &index, painter, &mut |l| crate::out::line(l));
        self.next_line = self.next_line.map(|next| next + index.lines());
    }

    /// The file ended where it stood: print the line it was still receiving,
    /// and read what stands under its name from the start, its first line
    /// the first.
    fn restart(&mut self, numbered: bool, painter: &trex::paint::Painter) {
        if numbered && !self.pending.is_empty() {
            let partial = std::mem::take(&mut self.pending);
            self.number(&partial, painter);
        }
        self.next_line = Some(1);
        self.decoder = Incremental::from_start();
    }
}

/// Follow `files` as they grow, printing what each gains, a header before a
/// file whose output follows another's where the listing heads its inputs.
/// A file truncated, replaced or removed is said so on the standard error
/// and read again from its start. Runs until it is interrupted.
fn follow_listing(mut files: Vec<Followee>, numbered: bool, headed: bool, painter: &trex::paint::Painter) -> ExitCode {
    let watched: Vec<(std::path::PathBuf, usize)> = files.iter().map(|f| (f.path.clone(), f.offset)).collect();
    let mut follower = match Follower::new(&watched) {
        Ok(follower) => follower,
        Err(e) => {
            eprintln!("trex: cannot follow: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(why) = follower.unnotified() {
        eprintln!("trex: no change notifications ({why}); looking at the files once a second");
    }
    // The file whose lines were printed last, so a header marks a change.
    let mut last = files.len().saturating_sub(1);
    loop {
        let (i, change) = match follower.wait() {
            Ok(next) => next,
            Err(e) => {
                eprintln!("trex: cannot follow {e}");
                return ExitCode::FAILURE;
            }
        };
        let f = &mut files[i];
        match change {
            Change::Appended { bytes, .. } => {
                let text = f.decoder.decode(&bytes);
                if text.is_empty() {
                    continue;
                }
                if headed && last != i {
                    crate::out::line("");
                    crate::out::line(&trex::report::listing_header(&f.label, painter));
                    last = i;
                }
                if let Err(e) = f.print(&text, numbered, painter) {
                    eprintln!("trex: cannot write the standard output: {e}");
                    return ExitCode::FAILURE;
                }
            }
            Change::Truncated => {
                eprintln!("trex: {}: truncated; printing it from its start", f.label);
                f.restart(numbered, painter);
            }
            Change::Replaced => {
                eprintln!("trex: {}: replaced by another file; printing that from its start", f.label);
                f.restart(numbered, painter);
            }
            Change::Gone => {
                eprintln!("trex: {}: removed; waiting for a file under its name", f.label);
                f.restart(numbered, painter);
            }
        }
    }
}
