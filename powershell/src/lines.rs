//! An input's first lines, its last, or a range of them, as the trex
//! command's `head`, `tail` and `lines` read them: from the end of a file
//! they sit at and no further, each line at its number in the input; and
//! with -Follow, what each file gains as it grows.

use pwrs::prelude::*;

use crate::common::arg_err;
use crate::matching::files_of;
use crate::window::{Read, read_file, select_of};

/// One line of an input.
#[psclass(name = "Trex.Line", show = "{LineNumber}:{Text}")]
#[derive(Clone, Default)]
pub struct TrexLine {
    /// The file the line is in, or empty for text.
    pub path: String,
    /// The line's number in the input, counting from 1.
    pub line_number: i64,
    /// The line's text, without its line ending.
    pub text: String,
}

/// Writes an input's first lines, its last, or a range of them, as the trex
/// command's head, tail and lines print them.
///
/// -Head N writes the first N lines, read no further than the Nth newline;
/// -Tail N the last N, read backward from the file's end; -Lines a range,
/// `"100..200"`, `"100.."`, `"..200"`, or PowerShell's own `100..200`. -Unit
/// counts paragraphs, blocks or another record unit instead of lines. -First
/// and -Last are -Head and -Tail. Each line is a Trex.Line with its number
/// in the input; -Passthru writes the lines as strings, a `==> path <==`
/// header ahead of each file's where several are read, as the trex command
/// prints them.
///
/// -Follow writes the lines each file gains as it grows, after its tail or
/// its open range, through truncation and rotation, which it writes as a
/// warning, until the pipeline is stopped.
///
/// # Examples
/// Get-TrexLine -Path app.log -Tail 20
/// Get-TrexLine -Path app.log -Lines 100..200
/// Get-TrexLine -Path notes.md -First 2 -Unit paragraph
/// 'a', 'b' -join "`n" | Get-TrexLine -Last 1
/// Get-TrexLine -Path app.log -Tail 0 -Follow | Where-Object Text -Match 'ERROR'
#[cmdlet(verb = "Get", noun = "TrexLine", alias = "Get-TxLine", default_parameter_set = "Path", output = ["Trex.Line", "System.String"])]
#[derive(Default)]
pub struct GetTrexLine {
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, position = 0, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, read as written, as Get-ChildItem pipes
    /// them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// Text to read; each string piped in is read on its own.
    #[param(mandatory, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Writes the first this many lines, or records of -Unit.
    #[param(alias = ["First"])]
    pub head: Option<u32>,
    /// Writes the last this many lines, or records of -Unit.
    #[param(alias = ["Last"])]
    pub tail: Option<u32>,
    /// Writes a range of lines, or records of -Unit, counted from one:
    /// `"100..200"`, `"100.."`, `"..200"`, `"7"`, or PowerShell's `100..200`.
    #[param]
    pub lines: Option<PsObject>,
    /// What -Head, -Tail and -Lines count: a line when absent, or a unit
    /// Find-TrexRecord reads, such as `paragraph` or `block`.
    #[param]
    pub unit: Option<String>,
    /// Writes the lines each file gains as it grows, after its tail or its
    /// open range, until the pipeline is stopped.
    #[param(alias = ["Wait"], set = ["Path", "LiteralPath"])]
    pub follow: bool,
    /// Writes the lines as strings, as the trex command prints them.
    #[param]
    pub passthru: bool,
    /// Reads files that hold a NUL byte, which a walk treats as binary.
    #[param(set = ["Path", "LiteralPath"])]
    pub binary: bool,
    /// Reads hidden files and directories a walk would skip.
    #[param(set = ["Path", "LiteralPath"])]
    pub hidden: bool,
    /// Reads files an ignore rule excludes.
    #[param(set = ["Path", "LiteralPath"])]
    pub no_ignore: bool,
    select: Option<trex::window::Select>,
    counted: Option<trex::records::RecordUnit>,
    headed_any: bool,
    followed: Vec<Followee>,
}

impl GetTrexLine {
    /// Write the lines of `read`, the part of the input at `path` read, as
    /// Trex.Line objects or, with -Passthru, as strings, each line's text
    /// without its ending.
    fn write(&self, ps: &Pipeline<'_>, read: &Read, path: &str) -> PsResult<()> {
        let index = read.index();
        // A string is a line even when it is empty, as each line Get-Content
        // yields is, where the window reaches it; an empty file has none.
        let lines = if path.is_empty() && read.input_len == 0 {
            let unit = self.counted()?;
            usize::from(self.select.is_none_or(|w| !trex::window::range_of(b"\n", w, unit).is_empty()))
        } else {
            index.lines()
        };
        for line in 0..lines {
            let (s, e) = index.line_span(line);
            let text = read.text[s..e].strip_suffix('\r').unwrap_or(&read.text[s..e]).to_string();
            if self.passthru {
                ps.write(text)?;
            } else {
                ps.write(TrexLine { path: path.to_string(), line_number: index.number(line) as i64, text })?;
            }
        }
        Ok(())
    }

    /// The unit -Head, -Tail and -Lines count.
    fn counted(&self) -> PsResult<&trex::records::RecordUnit> {
        self.counted.as_ref().ok_or_else(|| arg_err("TrexUnit", "the unit was not read"))
    }
}

impl Cmdlet for GetTrexLine {
    fn begin(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        self.select = select_of(self.head, self.tail, &self.lines)?;
        let Some(select) = self.select else {
            return Err(arg_err("TrexWindow", "give -Head, -Tail or -Lines: the part of each input written"));
        };
        if self.follow && !select.runs_to_end() {
            return Err(arg_err(
                "TrexFollow",
                "-Follow writes what a file gains past the lines read, and -Head or a -Lines range with a last line ends before the file does; follow a -Tail or a -Lines range with no last line",
            ));
        }
        self.counted = Some(match &self.unit {
            Some(name) => trex::records::RecordUnit::parse(name).map_err(|e| arg_err("TrexUnit", format!("-Unit: {e}")))?,
            None => trex::records::RecordUnit::Line,
        });
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let unit = self.counted()?.clone();
        if self.path.is_empty() && self.literal_path.is_empty() {
            let read = Read::of_text(&self.input_object, self.select, &unit);
            return self.write(ps, &read, "");
        }
        let (given, literal) =
            if self.literal_path.is_empty() { (self.path.clone(), false) } else { (self.literal_path.clone(), true) };
        let opts = trex::files::WalkOptions { hidden: self.hidden, no_ignore: self.no_ignore, ..Default::default() };
        let a_directory = given.iter().any(|p| std::path::Path::new(p).is_dir());
        let sources = files_of(ps, &given, literal, &opts)?;
        let headed = self.passthru && (a_directory || sources.len() > 1 || self.headed_any);
        for source in sources {
            if ps.stopping() {
                break;
            }
            let trex::files::Source::File(path) = source else {
                continue;
            };
            let shown = path.display().to_string();
            // A Trex.Line carries its number, which a tail counts the lines
            // ahead of it for; the strings of -Passthru carry none.
            let read = match read_file(&path, self.select, &unit, self.binary, !self.passthru) {
                Ok(Some(read)) => read,
                Ok(None) => {
                    ps.warning(&format!("{shown} holds a NUL byte and is binary; -Binary reads it"))?;
                    continue;
                }
                Err(e) => {
                    ps.write_error(&e)?;
                    continue;
                }
            };
            if headed {
                if self.headed_any {
                    ps.write(String::new())?;
                }
                ps.write(format!("==> {shown} <=="))?;
                self.headed_any = true;
            }
            self.write(ps, &read, &shown)?;
            if self.follow {
                self.followed.push(Followee::after(path, shown, &read));
            }
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        if !self.follow || self.followed.is_empty() {
            return Ok(());
        }
        let watched: Vec<(std::path::PathBuf, usize)> =
            self.followed.iter().map(|f| (f.path.clone(), f.offset)).collect();
        let mut follower = trex::follow::Follower::new(&watched)
            .map_err(|e| PsError::new(ErrorCategory::ReadError, "TrexFollow", format!("cannot follow: {e}")))?;
        if let Some(why) = follower.unnotified() {
            ps.warning(&format!("no change notifications ({why}); looking at the files once a second"))?;
        }
        let headed = self.passthru && self.followed.len() > 1;
        let mut last = self.followed.len() - 1;
        while !ps.stopping() {
            let polled = follower
                .poll()
                .map_err(|e| PsError::new(ErrorCategory::ReadError, "TrexFollow", format!("cannot follow {e}")))?;
            let Some((i, change)) = polled else {
                continue;
            };
            let f = &mut self.followed[i];
            match change {
                trex::follow::Followed::Appended { bytes, .. } => {
                    let text = f.decoder.decode(&bytes);
                    for (number, line) in f.take_lines(&text) {
                        if headed && last != i {
                            ps.write(String::new())?;
                            ps.write(format!("==> {} <==", f.shown))?;
                            last = i;
                        }
                        if self.passthru {
                            ps.write(line)?;
                            continue;
                        }
                        let Some(number) = number else {
                            return Err(PsError::new(
                                ErrorCategory::InvalidOperation,
                                "TrexUnnumbered",
                                "the lines ahead of the tail were not counted, and a Trex.Line carries its number",
                            ));
                        };
                        ps.write(TrexLine { path: f.shown.clone(), line_number: number, text: line })?;
                    }
                }
                trex::follow::Followed::Truncated => {
                    ps.warning(&format!("{}: truncated; reading it from its start", f.shown))?;
                    f.restart();
                }
                trex::follow::Followed::Replaced => {
                    ps.warning(&format!("{}: replaced by another file; reading that from its start", f.shown))?;
                    f.restart();
                }
                trex::follow::Followed::Gone => {
                    ps.warning(&format!("{}: removed; waiting for a file under its name", f.shown))?;
                    f.restart();
                }
            }
        }
        Ok(())
    }
}

/// One file -Follow reads on: its path, where following it picks up, how its
/// bytes decode, the number its next line takes where the lines were
/// counted, and the line it is still receiving.
#[derive(Default)]
struct Followee {
    path: std::path::PathBuf,
    shown: String,
    offset: usize,
    decoder: trex::encoding::Incremental,
    /// `None` where -Passthru, which writes no number, left a tail's lines
    /// ahead uncounted.
    next_line: Option<i64>,
    pending: Vec<u8>,
}

impl Followee {
    /// The file at `path` followed from where `read`, the part of it written,
    /// ends.
    fn after(path: std::path::PathBuf, shown: String, read: &Read) -> Followee {
        let lines_read = trex::byte_simd::count_byte(read.text.as_bytes(), b'\n');
        Followee {
            path,
            shown,
            offset: read.input_len,
            decoder: trex::encoding::Incremental::after(read.encoding),
            next_line: read.line_base.map(|ahead| (ahead + lines_read + 1) as i64),
            pending: Vec::new(),
        }
    }

    /// The lines `text`, what the file gained, completes, each with its
    /// number where the lines are counted and its text without its ending; a
    /// line still arriving is kept for the next.
    fn take_lines(&mut self, text: &[u8]) -> Vec<(Option<i64>, String)> {
        self.pending.extend_from_slice(text);
        let Some(last) = self.pending.iter().rposition(|&b| b == b'\n') else {
            return Vec::new();
        };
        let done: Vec<u8> = self.pending.drain(..=last).collect();
        let mut out = Vec::new();
        for line in done.split(|&b| b == b'\n').take(trex::byte_simd::count_byte(&done, b'\n')) {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            out.push((self.next_line, String::from_utf8_lossy(line).into_owned()));
            self.next_line = self.next_line.map(|n| n + 1);
        }
        out
    }

    /// The file ended where it stood: what stands under its name is read
    /// from its start, its first line the first.
    fn restart(&mut self) {
        self.pending.clear();
        self.next_line = Some(1);
        self.decoder = trex::encoding::Incremental::from_start();
    }
}
