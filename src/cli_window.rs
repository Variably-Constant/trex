//! What part of each input a command reads: the `--head N`, `--tail N` and
//! `--lines A..B` flags every command that reads its inputs as a scan does
//! takes, and `--follow`; the reads that fetch that part of a file or of the
//! standard input and no more; and the edits a rewrite, a redaction or a
//! rule's fix makes to that part alone, placed in the whole input.

use trex::files::{Edit, LineIndex, Source, is_binary, read_source};
use trex::records::RecordUnit;
use trex::window::{Asked, Select};

/// The window flags a command was given.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Windowing {
    /// `--head N`, `--tail N` or `--lines A..B`: the part of each input read.
    pub(crate) select: Option<Select>,
    /// `--follow`: read on as each file grows.
    pub(crate) follow: bool,
}

impl Windowing {
    /// Take the window flag at `args[*i]`, its value the next argument or
    /// the one after `=`, moving `i` past what it took: `None` where the
    /// argument is not one, and an error naming the flag where its value is
    /// wrong or a second window is named.
    pub(crate) fn take(&mut self, args: &[String], i: &mut usize) -> Option<Result<(), String>> {
        let arg = args[*i].as_str();
        let (flag, attached) = match arg.split_once('=') {
            Some((f, v)) if matches!(f, "--head" | "--tail" | "--lines") => (f, Some(v.to_string())),
            _ => (arg, None),
        };
        if flag == "--follow" {
            self.follow = true;
            return Some(Ok(()));
        }
        if !matches!(flag, "--head" | "--tail" | "--lines") {
            return None;
        }
        let value = match attached {
            Some(v) => v,
            None => {
                *i += 1;
                match args.get(*i) {
                    Some(v) => v.clone(),
                    None => {
                        let wants = if flag == "--lines" { "a range, as in 100..200, 100.. or ..200" } else { "a number" };
                        return Some(Err(format!("{flag} needs {wants}")));
                    }
                }
            }
        };
        Some(self.set(flag, &value))
    }

    /// Name the window `flag` says with `value`.
    fn set(&mut self, flag: &str, value: &str) -> Result<(), String> {
        let select = match flag {
            "--head" => Select::Head(count(flag, value)?),
            "--tail" => Select::Tail(count(flag, value)?),
            _ => Select::parse_range(value).map_err(|e| format!("--lines {e}"))?,
        };
        if self.select.is_some() {
            return Err(format!("{flag} names a second window; --head, --tail and --lines name one between them"));
        }
        self.select = Some(select);
        Ok(())
    }

    /// Why `--follow` cannot run as given, where it cannot: it reads on as
    /// a file grows, past the end of the window, which a head and a range
    /// with a last record stop short of.
    pub(crate) fn follow_refusal(&self) -> Option<&'static str> {
        match self.select {
            Some(s) if self.follow && !s.runs_to_end() => Some(
                "--follow reads on as a file grows, past the end of the window, and --head and --lines A..B end at a record they name; follow a --tail N, a --lines A.. or the whole file",
            ),
            _ => None,
        }
    }
}

/// A count a window flag takes.
fn count(flag: &str, value: &str) -> Result<usize, String> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{flag} needs a number, not {value:?}"));
    }
    value.parse::<usize>().map_err(|e| format!("{flag} {value}: {e}"))
}

/// An input as a command reads it: its text, where the text stands in the
/// input, whether it was binary, and where a follow of it picks up.
pub(crate) struct Part {
    /// The text read, decoded; empty for a binary input not read as text.
    pub(crate) text: Vec<u8>,
    /// Where `text` begins in the input's text, where that was counted: a
    /// command reading a UTF-16 or UTF-32 file's tail without asking for its
    /// place prints no offset of it.
    pub(crate) byte_base: Option<usize>,
    /// How many lines of the input come before `text`, where they were
    /// counted: a report printing no line number reads a tail without.
    pub(crate) line_base: Option<usize>,
    /// Whether the bytes read hold a NUL byte with no byte order mark
    /// declaring them text.
    pub(crate) binary: bool,
    /// How long the input was, in its own bytes, when it was read.
    pub(crate) input_len: usize,
    /// The encoding the input's opening declared.
    pub(crate) encoding: trex::encoding::Encoding,
}

impl Part {
    /// The line index a report over this part reads positions from, the
    /// input's own lines and offsets.
    pub(crate) fn index(&self) -> LineIndex {
        LineIndex::new(&self.text).within(self.byte_base, self.line_base)
    }

    /// Where `text` begins in the input's text, for a command that asked for
    /// its place.
    ///
    /// # Panics
    ///
    /// The part was read without asking for its place: a command that
    /// continues or prints its offsets asks for it.
    pub(crate) fn base(&self) -> usize {
        self.byte_base.expect("a part is placed wherever its command continues or prints its offsets")
    }

    /// `text`, an inline input, cut to what `select` names under `unit`.
    pub(crate) fn of_text(text: Vec<u8>, select: Option<Select>, unit: &RecordUnit) -> Part {
        let input_len = text.len();
        match select {
            None => Part {
                text,
                byte_base: Some(0),
                line_base: Some(0),
                binary: false,
                input_len,
                encoding: trex::encoding::Encoding::Utf8,
            },
            Some(s) => Part::from(trex::window::window_of(&text, s, unit)),
        }
    }
}

impl From<trex::window::Window> for Part {
    fn from(w: trex::window::Window) -> Part {
        Part {
            text: w.bytes,
            byte_base: w.byte_base,
            line_base: w.line_base,
            binary: w.binary,
            input_len: w.input_len,
            encoding: w.encoding,
        }
    }
}

/// Read `src` as a command reads it: the part `select` names under `unit`,
/// or the whole input where no window is asked for; decoded, and marked
/// binary where it holds a NUL byte, its text read only where `asked.binary`
/// asks for binary input as text.
///
/// # Errors
///
/// The input cannot be read.
pub(crate) fn read_part(src: &Source, select: Option<Select>, unit: &RecordUnit, asked: Asked) -> std::io::Result<Part> {
    let Some(select) = select else {
        let raw = read_source(src)?;
        let binary = is_binary(&raw);
        let input_len = raw.len();
        let encoding = trex::encoding::Encoding::declared(&raw).0;
        let text = if binary && !asked.binary { Vec::new() } else { trex::encoding::decode(raw) };
        return Ok(Part { text, byte_base: Some(0), line_base: Some(0), binary, input_len, encoding });
    };
    let window = match src {
        Source::File(path) => trex::window::read_file(path, select, unit, asked)?,
        Source::Stdin => trex::window::read_stream(std::io::stdin().lock(), select, unit, asked)?,
    };
    Ok(Part::from(window))
}

/// The part of an input an edit is confined to: the window the flags name,
/// counted in the unit a record is.
#[derive(Clone, Copy)]
pub(crate) struct Restrict<'a> {
    pub(crate) select: Select,
    pub(crate) unit: &'a RecordUnit,
}

impl<'a> Restrict<'a> {
    /// The confinement `windowing` asks for, in `unit`, where it asks for
    /// one.
    pub(crate) fn of(windowing: &Windowing, unit: &'a RecordUnit) -> Option<Restrict<'a>> {
        windowing.select.map(|select| Restrict { select, unit })
    }
}

/// The part of an input an edit reads: its bytes, and where they stand in
/// the input.
#[derive(Clone, Copy)]
pub(crate) struct Piece<'a> {
    pub(crate) text: &'a [u8],
    pub(crate) byte_base: usize,
    pub(crate) line_base: usize,
}

impl Piece<'_> {
    /// The line index an edit placing what it reports reads, the input's own
    /// lines and offsets.
    pub(crate) fn index(&self) -> LineIndex {
        LineIndex::new(self.text).within(Some(self.byte_base), Some(self.line_base))
    }
}

/// The edits `edits` makes to the part of `input` `restrict` names, as edits
/// of the whole input: `edits` reads the part alone, placed as the input
/// places it, and what it makes is moved to where the part stands. With no
/// confinement the part is the whole input.
pub(crate) fn edits_within(
    input: &[u8],
    restrict: Option<Restrict<'_>>,
    edits: impl FnOnce(Piece<'_>) -> Vec<Edit>,
) -> Vec<Edit> {
    let Some(r) = restrict else {
        return edits(Piece { text: input, byte_base: 0, line_base: 0 });
    };
    let range = trex::window::range_of(input, r.select, r.unit);
    let line_base = trex::byte_simd::count_byte(&input[..range.start], b'\n');
    let mut made = edits(Piece { text: &input[range.clone()], byte_base: range.start, line_base });
    for e in &mut made {
        e.start += range.start;
        e.end += range.start;
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn the_window_flags_take_their_values_either_way_and_name_one_window() {
        let a = args(&["--head", "3", "--follow"]);
        let mut w = Windowing::default();
        let mut i = 0;
        assert!(matches!(w.take(&a, &mut i), Some(Ok(()))));
        assert_eq!((i, w.select), (1, Some(Select::Head(3))));
        i += 1;
        assert!(matches!(w.take(&a, &mut i), Some(Ok(()))));
        assert!(w.follow);
        assert!(w.follow_refusal().is_some(), "a head ends before the file does");

        let a = args(&["--lines=5..", "--tail", "2", "--other"]);
        let mut w = Windowing::default();
        let mut i = 0;
        assert!(matches!(w.take(&a, &mut i), Some(Ok(()))));
        assert_eq!(w.select, Some(Select::Range { from: Some(5), to: None }));
        i += 1;
        assert!(matches!(w.take(&a, &mut i), Some(Err(e)) if e.contains("second window")));
        let mut i = 3;
        assert!(w.take(&a, &mut i).is_none());
        let mut i = 0;
        assert!(matches!(Windowing::default().take(&args(&["--tail", "x"]), &mut i), Some(Err(e)) if e.contains("number")));
    }

    #[test]
    fn an_edit_confined_to_a_window_changes_only_that_part() {
        let input = b"a 1\nb 2\nc 3\nd 4\n";
        let unit = RecordUnit::Line;
        let restrict = Some(Restrict { select: Select::Tail(2), unit: &unit });
        let edits = edits_within(input, restrict, |piece| {
            assert_eq!((piece.text, piece.byte_base, piece.line_base), (&b"c 3\nd 4\n"[..], 8, 2));
            vec![Edit { start: 2, end: 3, replacement: b"X".to_vec() }]
        });
        assert_eq!(edits, vec![Edit { start: 10, end: 11, replacement: b"X".to_vec() }]);
    }
}
