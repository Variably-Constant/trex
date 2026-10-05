//! The part of each input a cmdlet reads: -Head, -Tail and -Lines, counted
//! in lines or in records of -Unit, read from the end of a file nearest them
//! and no further, and placed at the input's own line numbers and UTF-16
//! offsets.

use std::path::Path;

use pwrs::prelude::*;

use crate::common::{arg_err, read_err};

/// The window -Head, -Tail or -Lines names, where one of them names one.
///
/// -Lines is a range as the `trex` command's `--lines` takes one, `"100..200"`,
/// `"100.."`, `"..200"` or `"7"`, or PowerShell's own range of line numbers,
/// `100..200`, read from its first number to its last.
pub(crate) fn select_of(
    head: Option<u32>,
    tail: Option<u32>,
    lines: &Option<PsObject>,
) -> PsResult<Option<trex::window::Select>> {
    use trex::window::Select;
    let range = match lines {
        None => None,
        Some(given) => Some(range_of(given)?),
    };
    match (head, tail, range) {
        (None, None, None) => Ok(None),
        (Some(n), None, None) => Ok(Some(Select::Head(n as usize))),
        (None, Some(n), None) => Ok(Some(Select::Tail(n as usize))),
        (None, None, Some(r)) => Ok(Some(r)),
        _ => Err(arg_err("TrexWindow", "-Head, -Tail and -Lines each name the part read; give one")),
    }
}

/// The part of each input a cmdlet reads: the window -Head, -Tail or -Lines
/// names, counted in records of -Unit, or the whole input.
#[derive(Clone)]
pub(crate) struct Part {
    pub(crate) select: Option<trex::window::Select>,
    pub(crate) unit: trex::records::RecordUnit,
}

impl Default for Part {
    fn default() -> Self {
        Part { select: None, unit: trex::records::RecordUnit::Line }
    }
}

impl Part {
    /// The part -Head, -Tail, -Lines and -Unit name; -Unit counts what the
    /// other three name, so it takes one of them.
    pub(crate) fn of(
        head: Option<u32>,
        tail: Option<u32>,
        lines: &Option<PsObject>,
        unit: &Option<String>,
    ) -> PsResult<Part> {
        let select = select_of(head, tail, lines)?;
        let unit = match unit {
            None => trex::records::RecordUnit::Line,
            Some(_) if select.is_none() => {
                return Err(arg_err("TrexUnit", "-Unit says what -Head, -Tail and -Lines count; give one of them"));
            }
            Some(name) => {
                trex::records::RecordUnit::parse(name).map_err(|e| arg_err("TrexUnit", format!("-Unit: {e}")))?
            }
        };
        Ok(Part { select, unit })
    }

    /// `text`, a string handed in whole, cut to this part.
    pub(crate) fn of_text(&self, text: &str) -> Read {
        Read::of_text(text, self.select, &self.unit)
    }

    /// This part of the file at `path`, as [`read_file`] reads it.
    pub(crate) fn read(&self, path: &Path, binary: bool, placed: bool) -> Result<Option<Read>, PsError> {
        read_file(path, self.select, &self.unit, binary, placed)
    }
}

/// A line number -Lines names, which counts from one.
fn line_number(n: i64) -> PsResult<usize> {
    if n < 1 {
        return Err(arg_err("TrexLines", format!("-Lines counts lines from 1, and {n} names none")));
    }
    usize::try_from(n).map_err(|e| arg_err("TrexLines", format!("-Lines {n}: {e}")))
}

/// The range -Lines names: a string, or the numbers of PowerShell's range.
fn range_of(given: &PsObject) -> PsResult<trex::window::Select> {
    use trex::window::Select;
    if given.type_name()? == "System.String" {
        let text = String::from_ps(given)?;
        return Select::parse_range(&text).map_err(|e| arg_err("TrexLines", format!("-Lines {e}")));
    }
    let numbers = <Vec<i64> as FromPs>::from_ps(given)
        .map_err(|e| arg_err("TrexLines", format!("-Lines takes a range as 100..200 or \"100..\": {e}")))?;
    let (Some(&first), Some(&last)) = (numbers.first(), numbers.last()) else {
        return Err(arg_err("TrexLines", "-Lines names no line"));
    };
    let (from, to) = (line_number(first)?, line_number(last)?);
    if from > to {
        return Err(arg_err("TrexLines", format!("-Lines runs from {from} back to {to}; give it ascending")));
    }
    Ok(Select::Range { from: Some(from), to: Some(to) })
}

/// One input as a cmdlet reads it: its text, and the text's offset in the
/// input.
pub(crate) struct Read {
    pub(crate) text: String,
    /// Where `text` begins in the input's UTF-8 text, where that was
    /// counted: a report placing nothing reads a UTF-16 or UTF-32 file's
    /// tail without it.
    pub(crate) byte_base: Option<usize>,
    /// How many lines of the input come before `text`, where they were
    /// counted: a report placing nothing reads a tail without them.
    pub(crate) line_base: Option<usize>,
    /// How many UTF-16 code units of the input's text come before `text`,
    /// where they were counted.
    pub(crate) unit_base: Option<usize>,
    /// How long the input was, in its own bytes, when it was read: where a
    /// follow of it picks up.
    pub(crate) input_len: usize,
    /// The encoding the input's opening declared.
    pub(crate) encoding: trex::encoding::Encoding,
}

impl Read {
    /// A string handed in whole, with nothing before it.
    fn whole(text: String) -> Read {
        Read {
            input_len: text.len(),
            text,
            byte_base: Some(0),
            line_base: Some(0),
            unit_base: Some(0),
            encoding: trex::encoding::Encoding::Utf8,
        }
    }

    /// The line index a report over this text reads positions from, the
    /// input's own lines and offsets.
    pub(crate) fn index(&self) -> trex::files::LineIndex {
        trex::files::LineIndex::new(self.text.as_bytes()).within(self.byte_base, self.line_base)
    }

    /// [`Self::index`] placing offsets in UTF-16 code units, as a rule's
    /// message writes `${start}` and `${end}` in this module.
    pub(crate) fn placing_index(&self) -> trex::files::LineIndex {
        let bytes = self.text.as_bytes();
        self.index().counting(bytes, trex::encoding::OffsetUnit::Utf16, self.unit_base)
    }

    /// How many UTF-16 code units of the input come before this text, which
    /// a match's Start adds to its own offset in the text.
    ///
    /// # Errors
    ///
    /// They were not counted: a report placing its matches asks for them, so
    /// this is one placing a match it chose not to place.
    pub(crate) fn units_ahead(&self) -> PsResult<usize> {
        self.unit_base.ok_or_else(unplaced)
    }

    /// Where this text begins in the input's UTF-8 text, which a match's
    /// byte offsets add to their own.
    ///
    /// # Errors
    ///
    /// It was not counted: a report placing its matches asks for it, so this
    /// is one placing a match it chose not to place.
    pub(crate) fn base(&self) -> PsResult<usize> {
        self.byte_base.ok_or_else(unplaced)
    }

    /// `text`, a string handed in whole, cut to the window `select` names in
    /// records of `unit` where it names one.
    pub(crate) fn of_text(text: &str, select: Option<trex::window::Select>, unit: &trex::records::RecordUnit) -> Read {
        match select {
            None => Read::whole(text.to_string()),
            Some(select) => from_window(trex::window::window_of(text.as_bytes(), select, unit)),
        }
    }
}

/// The error for a match placed in an input whose text ahead of the part
/// read was not counted: whatever places what it writes asks for them, so
/// this is one placing a match it chose not to place.
pub(crate) fn unplaced() -> PsError {
    PsError::new(
        ErrorCategory::InvalidOperation,
        "TrexUnplaced",
        "the text ahead of the part read was not counted, and this places what it writes",
    )
}

/// A window as the module holds it: its text read lossily where it is not
/// UTF-8, as a file read whole is.
fn from_window(w: trex::window::Window) -> Read {
    Read {
        text: String::from_utf8_lossy(&w.bytes).into_owned(),
        byte_base: w.byte_base,
        line_base: w.line_base,
        unit_base: w.unit_base,
        input_len: w.input_len,
        encoding: w.encoding,
    }
}

/// The file at `path`, decoded from whatever UTF encoding its byte-order
/// mark declares: the part `select` names in records of `unit` where it
/// names one, read from the end nearest it, and the whole file where it
/// does not; none for a binary file unless `binary` asks for those too.
/// `placed` counts the lines, the UTF-16 code units and, for a UTF-16 or
/// UTF-32 file, the UTF-8 bytes ahead of a tail, for a report that places
/// what it writes; a head and a range know theirs without it.
pub(crate) fn read_file(
    path: &Path,
    select: Option<trex::window::Select>,
    unit: &trex::records::RecordUnit,
    binary: bool,
    placed: bool,
) -> Result<Option<Read>, PsError> {
    let shown = path.display().to_string();
    let Some(select) = select else {
        let bytes = std::fs::read(path).map_err(|e| read_err(&shown, e))?;
        if !binary && trex::files::is_binary(&bytes) {
            return Ok(None);
        }
        let input_len = bytes.len();
        let encoding = trex::encoding::Encoding::declared(&bytes).0;
        let decoded = trex::encoding::decode(bytes);
        let text = String::from_utf8_lossy(&decoded).into_owned();
        return Ok(Some(Read { text, byte_base: Some(0), line_base: Some(0), unit_base: Some(0), input_len, encoding }));
    };
    let asked = trex::window::Asked { numbers: placed, offsets: placed, binary, units: placed };
    let window = trex::window::read_file(path, select, unit, asked).map_err(|e| read_err(&shown, e))?;
    if window.binary && !binary {
        return Ok(None);
    }
    Ok(Some(from_window(window)))
}
