//! The part of an input a head, a tail or a line range selects, and the
//! readers that fetch only that much of it.
//!
//! A head stops reading after the last record it keeps, a tail reads a file
//! backward from its end until it holds the records it keeps, and a range
//! reads up to its last record. Lines are found with the vector byte search,
//! and paragraphs from the lines around them, so neither reads past what it
//! selects. An input whose mark declares UTF-16 or UTF-32 is read the same
//! way in whole code units, and only its window is decoded. Every other
//! record unit is cut from the whole input, since its records are found from
//! all of it. A window keeps its offset in its input's text, so a report
//! over it gives the text's own byte offsets and, where they were counted,
//! its own line numbers.

use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;

use crate::byte_simd::{count_byte, count_byte_split, nth_byte, nth_byte_back};
use crate::encoding::{Encoding, Incremental, decode_units};
use crate::records::RecordUnit;

/// Which records of an input a command reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Select {
    /// The first `n` records.
    Head(usize),
    /// The last `n` records.
    Tail(usize),
    /// Records `from` through `to`, counted from one with both ends
    /// included; an absent `from` starts at the first record, and an absent
    /// `to` runs to the last.
    Range { from: Option<usize>, to: Option<usize> },
}

impl Select {
    /// The range `--lines` names: `A..B`, `A..`, `..B`, or `A` for record
    /// `A` alone.
    ///
    /// # Errors
    ///
    /// A bound that is not a whole number of one or more, no bound at all,
    /// or a start after the end.
    pub fn parse_range(s: &str) -> Result<Select, String> {
        let bound = |t: &str| -> Result<Option<usize>, String> {
            if t.is_empty() {
                return Ok(None);
            }
            match t.parse::<usize>() {
                Ok(0) => Err(format!("{s:?}: records count from 1, so 0 names none")),
                Ok(v) => Ok(Some(v)),
                Err(e) => Err(format!("{s:?}: {t:?} is not a record number ({e})")),
            }
        };
        let (from, to) = match s.trim().split_once("..") {
            Some((a, b)) => (bound(a.trim())?, bound(b.trim())?),
            None => {
                let one = bound(s.trim())?;
                (one, one)
            }
        };
        if from.is_none() && to.is_none() {
            return Err(format!("{s:?} names no record; write A..B, A.., ..B or A"));
        }
        if let (Some(a), Some(b)) = (from, to)
            && a > b
        {
            return Err(format!("{s:?}: the range starts at {a}, after its end at {b}"));
        }
        Ok(Select::Range { from, to })
    }

    /// Whether this keeps the last records of an input.
    #[must_use]
    pub fn is_tail(self) -> bool {
        match self {
            Select::Tail(..) => true,
            Select::Head(..) | Select::Range { .. } => false,
        }
    }

    /// Whether what this keeps runs to the input's end, so bytes appended to
    /// the input are inside it: a tail, and a range with no last record.
    #[must_use]
    pub fn runs_to_end(self) -> bool {
        match self {
            Select::Tail(..) | Select::Range { to: None, .. } => true,
            Select::Head(..) | Select::Range { to: Some(..), .. } => false,
        }
    }
}

/// What a read of a window is asked beyond the window itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Asked {
    /// Count the lines ahead of a file's tail, for a report that prints line
    /// numbers; a head, a range and a stream know theirs without it.
    pub numbers: bool,
    /// Place the tail of a file whose mark declares UTF-16 or UTF-32 in the
    /// text it decodes to, for a report that prints byte offsets or a follow
    /// that continues them: the text ahead of the tail is decoded in one pass
    /// across the cores. A tail whose lines are numbered is placed in the
    /// same pass, and every other window knows its place without it.
    pub offsets: bool,
    /// Read a window holding a NUL byte as text, decoded as a full read of
    /// the input decodes it, rather than handing it back marked binary.
    pub binary: bool,
    /// Count the UTF-16 code units of the text ahead of the window, for a
    /// surface that indexes text in UTF-16; a file's tail read with it has
    /// the lines ahead of it counted in the same pass.
    pub units: bool,
}

/// The bytes a selection took from an input, their offset in it, and
/// how many lines come before them where that was counted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Window {
    /// Whole records, the last with its newline where it has one.
    pub bytes: Vec<u8>,
    /// Where `bytes` begins in the input's text; `None` only for the tail of
    /// a file whose mark declares UTF-16 or UTF-32, where its place was not
    /// asked for.
    pub byte_base: Option<usize>,
    /// How many lines of the input come before `bytes`: always known for a
    /// head, a range and a stream, and for a file's tail where its lines
    /// were asked to be numbered.
    pub line_base: Option<usize>,
    /// How many UTF-16 code units the input's text holds before `bytes`,
    /// where they were asked to be counted.
    pub unit_base: Option<usize>,
    /// Whether the bytes read hold a NUL byte and no byte order mark declares
    /// them text, which marks the input binary: the window's own bytes where
    /// it was read alone, the whole input's where it was read whole.
    pub binary: bool,
    /// How long the input was, in its own bytes, when it was read, which is
    /// where following it as it grows picks up; for a stream, which is not
    /// followed, where the window ends in it.
    pub input_len: usize,
    /// The encoding the input's opening declared, which the bytes appended to
    /// it are decoded by.
    pub encoding: Encoding,
}

/// The byte range of `input` that `select` names under `unit`: whole
/// records, the last carrying its newline where it has one.
#[must_use]
pub fn range_of(input: &[u8], select: Select, unit: &RecordUnit) -> Range<usize> {
    if *unit == RecordUnit::Line {
        line_range(input, select)
    } else {
        record_range(input, select, &unit.records(input))
    }
}

/// `input`, a text already decoded, cut to what `select` names under `unit`,
/// with the lines and the UTF-16 code units before the cut counted.
#[must_use]
pub fn window_of(input: &[u8], select: Select, unit: &RecordUnit) -> Window {
    cut(input, select, unit, Count { units: true, chars: false }).window
}

/// [`window_of`], the text before the cut counted as `count` asks.
fn cut(input: &[u8], select: Select, unit: &RecordUnit, count: Count) -> Counted {
    let r = range_of(input, select, unit);
    let (lines, gone) = ahead(&input[..r.start], count);
    Counted {
        window: Window {
            line_base: Some(lines),
            unit_base: gone.units,
            byte_base: Some(r.start),
            bytes: input[r].to_vec(),
            binary: false,
            input_len: input.len(),
            encoding: Encoding::Utf8,
        },
        chars: gone.chars,
    }
}

/// The newlines of `prefix`, the text ahead of a window of text held in
/// memory whole, split across the cores from 4 MiB, and its UTF-16 code units
/// and characters where `count` asks for them.
fn ahead(prefix: &[u8], count: Count) -> (usize, Tally) {
    (count_byte_split(prefix, b'\n'), Tally::of(prefix, count))
}

/// The byte just past line `n` of `input`, its newline included, or the
/// input's end where it holds fewer; 0 for `n` of 0.
fn after_line(input: &[u8], n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    nth_byte(input, b'\n', n).map_or(input.len(), |at| at + 1)
}

/// Where the last `n` lines of `input` begin: after the newline ahead of
/// them, or at its start where it holds no more than `n`. A newline ending
/// the input ends its last line rather than opening an empty one.
fn tail_start(input: &[u8], n: usize) -> usize {
    if n == 0 {
        return input.len();
    }
    let back = n + usize::from(input.last() == Some(&b'\n'));
    nth_byte_back(input, b'\n', back).map_or(0, |at| at + 1)
}

fn line_range(input: &[u8], select: Select) -> Range<usize> {
    match select {
        Select::Head(n) => 0..after_line(input, n),
        Select::Tail(n) => tail_start(input, n)..input.len(),
        Select::Range { from, to } => {
            let start = after_line(input, from.map_or(0, |a| a - 1));
            let end = to.map_or(input.len(), |b| after_line(input, b));
            start..end.max(start)
        }
    }
}

/// The span from the first record `select` keeps to the end of the last,
/// over `records` ascending by start, the last record's newline with it;
/// empty where it keeps none.
fn record_range(input: &[u8], select: Select, records: &[(usize, usize)]) -> Range<usize> {
    let count = records.len();
    let (first, last) = match select {
        Select::Head(n) => (0, n.min(count)),
        Select::Tail(n) => (count.saturating_sub(n), count),
        Select::Range { from, to } => (from.map_or(0, |a| a - 1).min(count), to.map_or(count, |b| b.min(count))),
    };
    if first >= last {
        let at = if select.is_tail() { input.len() } else { 0 };
        return at..at;
    }
    let start = records[first].0;
    let end = records[first..last].iter().map(|r| r.1).fold(start, usize::max);
    let end = if input.get(end) == Some(&b'\n') { end + 1 } else { end };
    start..end
}

/// How much of a file the readers take in their first read; each read after
/// it takes twice the one before, so reaching far into a file costs a number
/// of reads that grows with the logarithm of the distance.
const FIRST_READ: usize = 64 * 1024;

/// How much of a file a count of the lines ahead of a tail reads at a time,
/// each block read and counted on a core of its own.
const COUNT_READ: usize = 16 * 1024 * 1024;

/// What a reader counts of the text ahead of a window beside its newlines:
/// its UTF-16 code units, and its characters.
#[derive(Clone, Copy, Debug, Default)]
struct Count {
    units: bool,
    chars: bool,
}

/// The UTF-16 code units and the characters of text gone by, each where it
/// is counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
    units: Option<usize>,
    chars: Option<usize>,
}

impl Tally {
    /// Nothing gone by yet, counted as `count` asks.
    fn new(count: Count) -> Tally {
        Tally { units: count.units.then_some(0), chars: count.chars.then_some(0) }
    }

    /// What `bytes` hold, counted as `count` asks.
    fn of(bytes: &[u8], count: Count) -> Tally {
        let mut tally = Tally::new(count);
        tally.add(bytes);
        tally
    }

    /// Count in `bytes`, UTF-8 text gone by.
    fn add(&mut self, bytes: &[u8]) {
        if let Some(units) = self.units.as_mut() {
            *units += crate::encoding::utf16_units(bytes);
        }
        if let Some(chars) = self.chars.as_mut() {
            *chars += chars_in(bytes);
        }
    }

    /// Count in `other`, the tally of text that went by after this one's.
    fn merge(&mut self, other: Tally) {
        if let (Some(units), Some(more)) = (self.units.as_mut(), other.units) {
            *units += more;
        }
        if let (Some(chars), Some(more)) = (self.chars.as_mut(), other.chars) {
            *chars += more;
        }
    }
}

/// How many characters the UTF-8 text `bytes` holds: the bytes that open
/// one, so a character cut between two pieces counts once, in the piece it
/// opens in.
fn chars_in(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count()
}

/// What a read of a window is asked: what [`Asked`] asks, and whether the
/// characters of the text ahead of the window are counted.
#[derive(Clone, Copy, Debug, Default)]
struct Ask {
    asked: Asked,
    chars: bool,
}

impl Ask {
    /// What the text ahead of the window is counted for beside its newlines.
    fn count(self) -> Count {
        Count { units: self.asked.units, chars: self.chars }
    }
}

/// A window, and the characters of its input's text ahead of it where they
/// were counted.
#[derive(Clone, Debug, Default)]
struct Counted {
    window: Window,
    chars: Option<usize>,
}

/// What an input holds ahead of a window: its newlines where they were
/// counted, and its UTF-16 code units and characters where they were.
#[derive(Clone, Copy, Debug, Default)]
struct Ahead {
    lines: Option<usize>,
    gone: Tally,
}

impl Ahead {
    /// Nothing ahead: a window at the input's start.
    const NONE: Ahead = Ahead { lines: Some(0), gone: Tally { units: Some(0), chars: Some(0) } };

    /// What `prefix`, the text a reader read ahead of a window, holds,
    /// counted in one pass as `count` asks.
    fn of(prefix: &[u8], count: Count) -> Ahead {
        Ahead { lines: Some(count_byte(prefix, b'\n')), gone: Tally::of(prefix, count) }
    }
}

/// A window of UTF-8 text at `byte_base` of it: a file's own bytes past any
/// mark, or a stream's text as it arrived.
fn local(bytes: Vec<u8>, byte_base: usize, before: Ahead, input_len: usize) -> Counted {
    Counted {
        window: Window {
            bytes,
            byte_base: Some(byte_base),
            line_base: before.lines,
            unit_base: before.gone.units,
            binary: false,
            input_len,
            encoding: Encoding::Utf8,
        },
        chars: before.gone.chars,
    }
}

/// The window `select` names under `unit` of `raw`, a whole input as read,
/// decoded as a full read decodes it; nothing but the mark where the input
/// is binary and is not to be read as text.
fn whole(raw: Vec<u8>, select: Select, unit: &RecordUnit, ask: Ask) -> Counted {
    let binary = crate::files::is_binary(&raw);
    let encoding = Encoding::declared(&raw).0;
    let input_len = raw.len();
    if binary && !ask.asked.binary {
        return Counted { window: Window { binary, input_len, encoding, ..Window::default() }, chars: None };
    }
    let counted = cut(&crate::encoding::decode(raw), select, unit, ask.count());
    Counted { window: Window { binary, input_len, encoding, ..counted.window }, chars: counted.chars }
}

/// Append up to `want` more bytes of `reader` to `buf`, fewer only at its
/// end, and give how many were appended.
fn read_more(reader: &mut impl Read, buf: &mut Vec<u8>, want: usize) -> io::Result<usize> {
    let before = buf.len();
    reader.by_ref().take(want as u64).read_to_end(buf)?;
    Ok(buf.len() - before)
}

/// The part of the file at `path` that `select` names under `unit`.
///
/// Lines and paragraphs are read from the end nearest the selection, and no
/// further than it needs: a head stops after its last record, a tail reads
/// backward from the file's end, and a range reads to its end. A file whose
/// mark declares UTF-8, or that opens with no mark, is read as UTF-8 this
/// way from past its mark; one whose mark declares UTF-16 or UTF-32 is read
/// the same way in whole code units, and only its window is decoded. Offsets
/// are those of the text, as a full read decodes it. `asked.numbers` asks for
/// a tail's line base, which counts every newline before the tail and so
/// reads the whole file ahead of it, across the cores and without scanning
/// it; a head and a range know theirs. `asked.units` counts the UTF-16 code
/// units ahead of the window in the same reads, and `asked.offsets` places a
/// UTF-16 or UTF-32 tail.
///
/// A unit whose records come from the whole input is read whole, decoded as
/// a full read decodes it, and cut in memory. A UTF-8 window holding a NUL
/// byte is binary, or BOM-less UTF-16, every ASCII character of which carries
/// one, since only a UTF-16 or UTF-32 mark declares a NUL text: it comes back
/// marked binary, or, where `asked.binary` asks for such input as text, the
/// file is read whole and decoded.
///
/// # Errors
///
/// The file cannot be opened, measured, positioned or read.
pub fn read_file(path: &Path, select: Select, unit: &RecordUnit, asked: Asked) -> io::Result<Window> {
    read_counted(path, select, unit, Ask { asked, chars: false }).map(|counted| counted.window)
}

/// [`read_file`], with the characters of the text ahead of the window counted
/// in the same reads, for a surface that indexes text by character: the
/// window, and the characters ahead of it, which a window handed back binary
/// and unread does not count. The window is placed in its text, as
/// `asked.offsets` places it, wherever its characters are counted.
///
/// # Errors
///
/// The file cannot be opened, measured, positioned or read.
pub fn read_file_with_chars(
    path: &Path,
    select: Select,
    unit: &RecordUnit,
    asked: Asked,
) -> io::Result<(Window, Option<usize>)> {
    read_counted(path, select, unit, Ask { asked, chars: true }).map(|counted| (counted.window, counted.chars))
}

/// [`read_file`], the text ahead of the window counted as `ask` asks.
fn read_counted(path: &Path, select: Select, unit: &RecordUnit, ask: Ask) -> io::Result<Counted> {
    let mut file = std::fs::File::open(path)?;
    let len = usize::try_from(file.metadata()?.len())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))?;
    let mut opening = Vec::with_capacity(4);
    read_more(&mut file, &mut opening, 4)?;
    let (encoding, mark) = Encoding::declared(&opening);
    if *unit == RecordUnit::Line || *unit == RecordUnit::Paragraph {
        if encoding != Encoding::Utf8 {
            return read_encoded(&mut file, len, encoding, mark, select, unit, ask);
        }
        file.seek(SeekFrom::Start(mark as u64))?;
        let mut counted = if *unit == RecordUnit::Line {
            read_lines(&mut file, mark, len, select, ask)?
        } else {
            read_paragraphs(&mut file, mark, len, select, ask)?
        };
        // Only a UTF-16 or UTF-32 mark declares a NUL text, as a full read
        // judges the file.
        if !counted.window.bytes.contains(&0) {
            return Ok(counted);
        }
        if !ask.asked.binary {
            counted.window.binary = true;
            return Ok(counted);
        }
    }
    file.seek(SeekFrom::Start(0))?;
    let mut raw = Vec::with_capacity(len);
    file.read_to_end(&mut raw)?;
    Ok(whole(raw, select, unit, ask))
}

/// The lines of an open file `select` names; `file` is at `origin`, where
/// its UTF-8 text begins past any mark, and is `len` bytes long. Offsets are
/// the text's, counted from `origin`.
fn read_lines(file: &mut std::fs::File, origin: usize, len: usize, select: Select, ask: Ask) -> io::Result<Counted> {
    match select {
        Select::Head(n) => {
            let mut bytes = Vec::new();
            read_through_line(file, &mut bytes, n)?;
            Ok(local(bytes, 0, Ahead::NONE, len))
        }
        Select::Range { from, to } => {
            let skip = from.map_or(0, |a| a - 1);
            let (byte_base, mut bytes, gone) = skip_lines(file, skip, ask.count())?;
            let input_len = match to {
                Some(b) => {
                    read_through_line(file, &mut bytes, b - skip)?;
                    len
                }
                None => {
                    file.read_to_end(&mut bytes)?;
                    origin + byte_base + bytes.len()
                }
            };
            Ok(local(bytes, byte_base, Ahead { lines: Some(skip), gone }, input_len))
        }
        Select::Tail(n) => {
            let (start, bytes) = read_tail_lines(file, origin, len, n)?;
            Ok(local(bytes, start - origin, ahead_of_tail(file, origin, start, ask)?, len))
        }
    }
}

/// What a file's text, from `origin` past any mark, holds ahead of a tail
/// that begins at `end`: nothing counted unless `ask` asks for its lines,
/// its units or its characters, and all of them in one pass where it does.
fn ahead_of_tail(file: &std::fs::File, origin: usize, end: usize, ask: Ask) -> io::Result<Ahead> {
    let count = ask.count();
    if !ask.asked.numbers && !count.units && !count.chars {
        return Ok(Ahead::default());
    }
    let (lines, gone) = count_lines_in_blocks(file, origin, end, COUNT_READ, count)?;
    Ok(Ahead { lines: Some(lines), gone })
}

/// Read on into `bytes`, which holds the start of the lines being read,
/// until it holds `n` newlines, and cut it just past the `n`th; the whole
/// rest of the file where it has fewer.
fn read_through_line(file: &mut std::fs::File, bytes: &mut Vec<u8>, n: usize) -> io::Result<()> {
    if n == 0 {
        bytes.clear();
        return Ok(());
    }
    let mut seen = 0usize;
    let mut from = 0usize;
    let mut want = FIRST_READ;
    loop {
        if let Some(at) = nth_byte(&bytes[from..], b'\n', n - seen) {
            bytes.truncate(from + at + 1);
            return Ok(());
        }
        seen += count_byte(&bytes[from..], b'\n');
        from = bytes.len();
        if read_more(file, bytes, want)? == 0 {
            return Ok(());
        }
        want = want.saturating_mul(2);
    }
}

/// Read past the first `skip` lines of an open file at its start, giving
/// where the next line begins, the bytes already read of it and beyond, and
/// the UTF-16 code units and characters of the lines skipped where `count`
/// asks for them.
fn skip_lines(file: &mut std::fs::File, skip: usize, count: Count) -> io::Result<(usize, Vec<u8>, Tally)> {
    let mut offset = 0usize;
    let mut buf = Vec::new();
    let mut skipped = Tally::new(count);
    if skip == 0 {
        return Ok((0, buf, skipped));
    }
    let mut seen = 0usize;
    let mut want = FIRST_READ;
    loop {
        let got = read_more(file, &mut buf, want)?;
        if let Some(at) = nth_byte(&buf, b'\n', skip - seen) {
            skipped.add(&buf[..=at]);
            buf.drain(..=at);
            return Ok((offset + at + 1, buf, skipped));
        }
        seen += count_byte(&buf, b'\n');
        skipped.add(&buf);
        offset += buf.len();
        buf.clear();
        if got == 0 {
            return Ok((offset, buf, skipped));
        }
        want = want.saturating_mul(2);
    }
}

/// The last `n` lines of the text an open file `len` bytes long holds from
/// `origin` past any mark, read backward from its end, and where they begin
/// in the file.
fn read_tail_lines(file: &mut std::fs::File, origin: usize, len: usize, n: usize) -> io::Result<(usize, Vec<u8>)> {
    if n == 0 || len <= origin {
        return Ok((len, Vec::new()));
    }
    let mut last = [0u8; 1];
    file.seek(SeekFrom::Start((len - 1) as u64))?;
    file.read_exact(&mut last)?;
    // A newline ending the file ends its last line rather than opening one.
    let want_newlines = n + usize::from(last[0] == b'\n');
    let mut start = len;
    let mut held: Vec<u8> = Vec::new();
    let mut seen = 0usize;
    let mut want = FIRST_READ;
    while start > origin {
        let from = start.saturating_sub(want).max(origin);
        let mut block = vec![0u8; start - from];
        file.seek(SeekFrom::Start(from as u64))?;
        file.read_exact(&mut block)?;
        if let Some(at) = nth_byte_back(&block, b'\n', want_newlines - seen) {
            block.drain(..=at);
            block.extend_from_slice(&held);
            return Ok((from + at + 1, block));
        }
        seen += count_byte(&block, b'\n');
        block.extend_from_slice(&held);
        held = block;
        start = from;
        want = want.saturating_mul(2);
    }
    Ok((origin, held))
}

/// How many newlines the bytes of an open file from `from` to `end` hold,
/// and their UTF-16 code units and characters where `count` asks, counted a
/// block of `block` bytes at a time across the cores.
fn count_lines_in_blocks(
    file: &std::fs::File,
    from: usize,
    end: usize,
    block: usize,
    count: Count,
) -> io::Result<(usize, Tally)> {
    let counts = in_blocks(file, from, end, block, "count lines before", |bytes| {
        (count_byte(bytes, b'\n'), Tally::of(bytes, count))
    })?;
    let mut lines = 0usize;
    let mut gone = Tally::new(count);
    for (l, tally) in counts {
        lines += l;
        gone.merge(tally);
    }
    Ok((lines, gone))
}

/// `each` over the bytes of an open file from `from` to `end`, cut into
/// blocks of `block` bytes, each block read at its own offset and taken on a
/// core of its own, so the reading splits across the cores with the work;
/// the answers in the order of the blocks. `rung` names the split in the
/// trace.
fn in_blocks<T: Default + Send>(
    file: &std::fs::File,
    from: usize,
    end: usize,
    block: usize,
    rung: &'static str,
    each: impl Fn(&[u8]) -> T + Sync,
) -> io::Result<Vec<T>> {
    let blocks = (end - from).div_ceil(block);
    if blocks <= 1 {
        let mut bytes = vec![0u8; end - from];
        read_exact_at(file, &mut bytes, from as u64)?;
        return Ok(vec![each(&bytes)]);
    }
    crate::trace::rung(rung, "blocks", blocks);
    let leaves = u32::try_from(blocks).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("{blocks} blocks ahead of the tail are more than a count reads: {e}"))
    })?;
    let mut answers: Vec<io::Result<T>> = (0..blocks).map(|_| Ok(T::default())).collect();
    // The byte scan's own profile, as `count_byte_across` takes it: a block
    // costs a read and one pass over its bytes.
    let plan = flynnel::JobPlan::set_profile(0, leaves, flynnel::DispatchProfile::Streaming);
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut answers, 1, |base, slots| {
        let mut held = vec![0u8; block];
        for (k, slot) in slots.iter_mut().enumerate() {
            let lo = from + (base + k) * block;
            let hi = (lo + block).min(end);
            let bytes = &mut held[..hi - lo];
            *slot = read_exact_at(file, bytes, lo as u64).map(|()| each(bytes));
        }
    });
    answers.into_iter().collect()
}

/// Fill `buf` from the file's bytes at `at`, without moving where another
/// reader of the same file reads from.
#[cfg(unix)]
fn read_exact_at(file: &std::fs::File, buf: &mut [u8], at: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, at)
}

/// Fill `buf` from the file's bytes at `at`, each read naming its own offset,
/// so readers on several cores share one handle.
#[cfg(windows)]
fn read_exact_at(file: &std::fs::File, mut buf: &mut [u8], mut at: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_read(buf, at) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the file ended before the lines ahead of its tail were counted",
                ));
            }
            Ok(n) => {
                buf = &mut buf[n..];
                at += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn read_exact_at(file: &std::fs::File, buf: &mut [u8], at: u64) -> io::Result<()> {
    compile_error!("counting the lines ahead of a tail reads a file at an offset, which trex reads on Unix and Windows")
}

/// The paragraphs of an open file `select` names; `file` is at
/// `origin`, where its UTF-8 text begins past any mark, and is `len` bytes
/// long. Offsets are the text's, counted from `origin`.
///
/// A head or a range reads forward until the records it has read hold one
/// past its last, so the last it keeps is whole; a tail reads backward
/// from the file's end, from a line start, until it holds one more record
/// than it keeps or a blank line ahead of its first, so the first it keeps
/// is whole.
fn read_paragraphs(file: &mut std::fs::File, origin: usize, len: usize, select: Select, ask: Ask) -> io::Result<Counted> {
    let unit = RecordUnit::Paragraph;
    if let Select::Tail(n) = select {
        let mut start = len;
        let mut held: Vec<u8> = Vec::new();
        let mut want = FIRST_READ;
        loop {
            let from = start.saturating_sub(want).max(origin);
            let mut block = vec![0u8; start - from];
            file.seek(SeekFrom::Start(from as u64))?;
            file.read_exact(&mut block)?;
            block.extend_from_slice(&held);
            held = block;
            start = from;
            // From a line start, so no line is read in part.
            let cut = if start == origin { 0 } else { nth_byte(&held, b'\n', 1).map_or(held.len(), |at| at + 1) };
            let records = unit.records(&held[cut..]);
            let whole = start == origin
                || records.len() > n
                || (records.len() == n && records.first().is_some_and(|r| r.0 > 0));
            if whole {
                let r = record_range(&held[cut..], select, &records);
                let at = start + cut + r.start;
                let before = ahead_of_tail(file, origin, at, ask)?;
                return Ok(local(held[cut + r.start..cut + r.end].to_vec(), at - origin, before, len));
            }
            want = want.saturating_mul(2);
        }
    }
    let last = match select {
        Select::Head(n) | Select::Tail(n) => Some(n),
        Select::Range { to, .. } => to,
    };
    let mut buf = Vec::new();
    let mut want = FIRST_READ;
    loop {
        let got = read_more(file, &mut buf, want)?;
        let records = unit.records(&buf);
        if got == 0 || last.is_some_and(|b| records.len() > b) {
            let r = record_range(&buf, select, &records);
            let before = Ahead::of(&buf[..r.start], ask.count());
            // Read to its end, the file is as long as what was read of it.
            let input_len = if got == 0 { origin + buf.len() } else { len };
            return Ok(local(buf[r.clone()].to_vec(), r.start, before, input_len));
        }
        want = want.saturating_mul(2);
    }
}

/// A reader of the text of `inner`, code units in `encoding` read from past
/// any mark, handing on their UTF-8 as a full read decodes it, and counting
/// the bytes of `inner` it has taken.
struct Decoding<R> {
    inner: R,
    decoder: Incremental,
    piece: Vec<u8>,
    text: Vec<u8>,
    at: usize,
    taken: usize,
    ended: bool,
}

impl<R: Read> Decoding<R> {
    fn new(inner: R, encoding: Encoding) -> Self {
        Decoding {
            inner,
            decoder: Incremental::after(encoding),
            piece: vec![0u8; FIRST_READ],
            text: Vec::new(),
            at: 0,
            taken: 0,
            ended: false,
        }
    }
}

impl<R: Read> Read for Decoding<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        while self.at == self.text.len() {
            if self.ended {
                return Ok(0);
            }
            let got = self.inner.read(&mut self.piece)?;
            self.taken += got;
            self.text = if got == 0 {
                self.ended = true;
                self.decoder.finish()
            } else {
                self.decoder.decode(&self.piece[..got])
            };
            self.at = 0;
        }
        let n = buf.len().min(self.text.len() - self.at);
        buf[..n].copy_from_slice(&self.text[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// The lines or paragraphs `select` names of an open file `len` bytes long
/// whose opening `mark` declares `encoding`, UTF-16 or UTF-32. A head or a
/// range reads forward through the decoder to its last record, as a stream
/// is read; a tail reads backward from the file's end in whole code units,
/// and only its window is decoded. Offsets are those of the text the file
/// decodes to, which a tail learns only where `asked` asks for its place.
fn read_encoded(
    file: &mut std::fs::File,
    len: usize,
    encoding: Encoding,
    mark: usize,
    select: Select,
    unit: &RecordUnit,
    ask: Ask,
) -> io::Result<Counted> {
    let mut counted = if let Select::Tail(n) = select {
        tail_encoded(file, len, encoding, mark, n, unit, ask)?
    } else {
        file.seek(SeekFrom::Start(mark as u64))?;
        let mut text = Decoding::new(&mut *file, encoding);
        let mut c = if *unit == RecordUnit::Line {
            stream_lines(&mut text, select, ask.count())?
        } else {
            stream_paragraphs(&mut text, select, ask.count())?
        };
        // Read to its end, the file is as long as what was read of it.
        c.window.input_len = if text.ended { mark + text.taken } else { len };
        c
    };
    counted.window.encoding = encoding;
    Ok(counted)
}

/// The last `n` records of an open file `len` bytes long whose opening
/// `mark` declares `encoding`, UTF-16 or UTF-32, read backward from the
/// file's end in whole code units and decoded alone. What the text ahead of
/// them decodes to is counted only where `ask` asks for their place, their
/// line numbers, their characters or, in UTF-32, their UTF-16 code units; a
/// UTF-16 tail's code units ahead are half its bytes ahead.
fn tail_encoded(
    file: &std::fs::File,
    len: usize,
    encoding: Encoding,
    mark: usize,
    n: usize,
    unit: &RecordUnit,
    ask: Ask,
) -> io::Result<Counted> {
    let width = encoding.unit_width();
    // A part of a unit at the end is no text, as a full read drops it.
    let end = mark + (len - mark) / width * width;
    let (start, bytes) = if *unit == RecordUnit::Line {
        let (start, raw) = tail_units(file, encoding, mark, end, n)?;
        (start, decode_units(&raw, encoding))
    } else {
        tail_paragraphs_encoded(file, encoding, mark, end, n)?
    };
    let asked = ask.asked;
    let decoded = if asked.numbers || asked.offsets || (asked.units && width == 4) || ask.chars {
        Some(count_encoded_ahead(file, encoding, mark, start, COUNT_READ)?)
    } else {
        None
    };
    let unit_base = match (asked.units, width) {
        (false, _) => None,
        (true, 2) => Some((start - mark) / 2),
        (true, _) => decoded.map(|d| d.units),
    };
    Ok(Counted {
        window: Window {
            bytes,
            byte_base: decoded.map(|d| d.bytes),
            line_base: decoded.map(|d| d.lines),
            unit_base,
            binary: false,
            input_len: len,
            encoding,
        },
        chars: if ask.chars { decoded.map(|d| d.chars) } else { None },
    })
}

/// The last `n` lines of the text an open file holds from `mark` to `end`,
/// code units in `encoding` read backward from `end` in whole units, as the
/// file's bytes, and where they begin; a newline ending the text ends its
/// last line rather than opening one.
fn tail_units(file: &std::fs::File, encoding: Encoding, mark: usize, end: usize, n: usize) -> io::Result<(usize, Vec<u8>)> {
    let width = encoding.unit_width();
    let newline = encoding.newline();
    if n == 0 || end == mark {
        return Ok((end, Vec::new()));
    }
    let mut last = vec![0u8; width];
    read_exact_at(file, &mut last, (end - width) as u64)?;
    let want_newlines = n + usize::from(last == newline);
    let mut start = end;
    let mut held: Vec<u8> = Vec::new();
    let mut seen = 0usize;
    let mut want = FIRST_READ;
    while start > mark {
        let from = mark + (start.saturating_sub(want).max(mark) - mark) / width * width;
        let mut block = vec![0u8; start - from];
        read_exact_at(file, &mut block, from as u64)?;
        if let Some(at) = nth_unit_back(&block, newline, want_newlines - seen) {
            block.drain(..at + width);
            block.extend_from_slice(&held);
            return Ok((from + at + width, block));
        }
        seen += count_units(&block, newline);
        block.extend_from_slice(&held);
        held = block;
        start = from;
        want = want.saturating_mul(2);
    }
    Ok((mark, held))
}

/// The last `n` paragraphs of the text an open file holds from `mark` to
/// `end`, code units in `encoding`, read backward from `end` in whole units
/// until what was read holds one more paragraph than the tail keeps or a
/// blank line ahead of its first, as [`read_paragraphs`] reads UTF-8: where
/// the tail begins in the file, and its text decoded.
fn tail_paragraphs_encoded(
    file: &std::fs::File,
    encoding: Encoding,
    mark: usize,
    end: usize,
    n: usize,
) -> io::Result<(usize, Vec<u8>)> {
    let unit = RecordUnit::Paragraph;
    let width = encoding.unit_width();
    let newline = encoding.newline();
    let mut start = end;
    let mut held: Vec<u8> = Vec::new();
    let mut want = FIRST_READ;
    loop {
        let from = mark + (start.saturating_sub(want).max(mark) - mark) / width * width;
        let mut block = vec![0u8; start - from];
        read_exact_at(file, &mut block, from as u64)?;
        block.extend_from_slice(&held);
        held = block;
        start = from;
        let text = decode_units(&held, encoding);
        // From a line start, so no line is read in part.
        let cut = if start == mark { 0 } else { nth_byte(&text, b'\n', 1).map_or(text.len(), |at| at + 1) };
        let records = unit.records(&text[cut..]);
        let whole = start == mark
            || records.len() > n
            || (records.len() == n && records.first().is_some_and(|r| r.0 > 0));
        if whole {
            let r = record_range(&text[cut..], Select::Tail(n), &records);
            // Each newline of the text is one newline unit of the bytes read,
            // so the tail's first line begins after the same count of them.
            let lines = count_byte(&text[..cut + r.start], b'\n');
            let at = if lines == 0 {
                start
            } else {
                let unit_at = nth_unit(&held, newline, lines)
                    .expect("each newline of the decoded text is a newline unit of the bytes read");
                start + unit_at + width
            };
            return Ok((at, text[cut + r.start..cut + r.end].to_vec()));
        }
        want = want.saturating_mul(2);
    }
}

/// What the text ahead of an encoded window decodes to.
#[derive(Clone, Copy, Debug, Default)]
struct Decoded {
    /// Its newlines.
    lines: usize,
    /// Its bytes as UTF-8.
    bytes: usize,
    /// Its UTF-16 code units.
    units: usize,
    /// Its characters.
    chars: usize,
}

/// What one block of code units decodes to on its own, and whether it ends
/// with a high surrogate or opens with a low one, which pair across the
/// boundary with the block beside it.
#[derive(Clone, Copy, Debug, Default)]
struct BlockText {
    decoded: Decoded,
    ends_high: bool,
    opens_low: bool,
}

/// What the text an open file holds from `mark` to `end`, code units in
/// `encoding`, decodes to, counted a block of `block` bytes at a time across
/// the cores; `block` is a whole number of units.
fn count_encoded_ahead(file: &std::fs::File, encoding: Encoding, mark: usize, end: usize, block: usize) -> io::Result<Decoded> {
    let blocks = in_blocks(file, mark, end, block, "decode before", |bytes| block_text(bytes, encoding))?;
    let mut total = Decoded::default();
    let mut high_before = false;
    for b in blocks {
        total.lines += b.decoded.lines;
        total.bytes += b.decoded.bytes;
        total.units += b.decoded.units;
        total.chars += b.decoded.chars;
        // A pair cut between two blocks is one four-byte character, where
        // each block alone reads its half as a three-byte U+FFFD.
        if high_before && b.opens_low {
            total.bytes -= 2;
            total.chars -= 1;
        }
        high_before = b.ends_high;
    }
    Ok(total)
}

/// What `bytes`, whole code units in `encoding`, decode to on their own, as
/// [`decode_units`] decodes them.
fn block_text(bytes: &[u8], encoding: Encoding) -> BlockText {
    match encoding {
        Encoding::Utf16Le => utf16_block(bytes, u16::from_le_bytes),
        Encoding::Utf16Be => utf16_block(bytes, u16::from_be_bytes),
        Encoding::Utf32Le => utf32_block(bytes, u32::from_le_bytes),
        Encoding::Utf32Be => utf32_block(bytes, u32::from_be_bytes),
        Encoding::Utf8 => BlockText {
            decoded: Decoded {
                lines: count_byte(bytes, b'\n'),
                bytes: bytes.len(),
                units: crate::encoding::utf16_units(bytes),
                chars: chars_in(bytes),
            },
            ..BlockText::default()
        },
    }
}

/// What `bytes`, whole UTF-16 code units in the byte order `unit` reads,
/// decode to on their own. Each byte order is its own loop, so `unit` is
/// inlined.
fn utf16_block(bytes: &[u8], unit: impl Fn([u8; 2]) -> u16) -> BlockText {
    let units = bytes.as_chunks::<2>().0;
    let high = |u: u16| (0xD800..0xDC00).contains(&u);
    let low = |u: u16| (0xDC00..0xE000).contains(&u);
    let mut decoded = Decoded { units: units.len(), chars: units.len(), ..Decoded::default() };
    // Outside the surrogates a unit is one character of one, two or three
    // bytes, counted without a branch; a block holding a surrogate is walked
    // unit by unit below, a pair being one character and a lone one U+FFFD.
    let mut surrogates = 0usize;
    for u in units.iter().map(|c| unit(*c)) {
        decoded.bytes += 1 + usize::from(u >= 0x80) + usize::from(u >= 0x800);
        decoded.lines += usize::from(u == 0x0A);
        surrogates += usize::from((0xD800..0xE000).contains(&u));
    }
    if surrogates == 0 {
        return BlockText { decoded, ends_high: false, opens_low: false };
    }
    decoded = Decoded { units: units.len(), ..Decoded::default() };
    let mut i = 0;
    while i < units.len() {
        let u = unit(units[i]);
        let paired = high(u) && units.get(i + 1).is_some_and(|next| low(unit(*next)));
        // A surrogate pair is one four-byte character, and a surrogate with no
        // partner reads as U+FFFD, three bytes, as String::from_utf16_lossy
        // reads it.
        decoded.bytes += if paired {
            4
        } else if u < 0x80 {
            1
        } else if u < 0x800 {
            2
        } else {
            3
        };
        decoded.lines += usize::from(u == 0x0A);
        decoded.chars += 1;
        i += if paired { 2 } else { 1 };
    }
    BlockText {
        decoded,
        ends_high: units.last().is_some_and(|u| high(unit(*u))),
        opens_low: units.first().is_some_and(|u| low(unit(*u))),
    }
}

/// What `bytes`, whole UTF-32 code units in the byte order `unit` reads,
/// decode to on their own, a value that is no character dropped. Each byte
/// order is its own loop, so `unit` is inlined.
fn utf32_block(bytes: &[u8], unit: impl Fn([u8; 4]) -> u32) -> BlockText {
    let mut decoded = Decoded::default();
    for c in bytes.as_chunks::<4>().0.iter().filter_map(|u| char::from_u32(unit(*u))) {
        decoded.bytes += c.len_utf8();
        decoded.units += c.len_utf16();
        decoded.lines += usize::from(c == '\n');
        decoded.chars += 1;
    }
    BlockText { decoded, ..BlockText::default() }
}

/// Where the newline unit whose newline byte is at `at` in `bytes`,
/// whole code units as wide as `newline`, begins: none where that byte is
/// part of a unit that is not a newline.
fn newline_unit_at(bytes: &[u8], newline: &[u8], at: usize) -> Option<usize> {
    let width = newline.len();
    let start = at.checked_sub(newline.iter().position(|&b| b == b'\n')?)?;
    (start.is_multiple_of(width) && bytes.get(start..start + width) == Some(newline)).then_some(start)
}

/// Where in `bytes`, whole code units as wide as `newline`, the `k`th newline
/// unit from the start is: each newline byte found by the vector search,
/// and kept if it is in a whole newline unit.
fn nth_unit(bytes: &[u8], newline: &[u8], k: usize) -> Option<usize> {
    let mut left = k;
    let mut from = 0;
    while left > 0 {
        let at = from + nth_byte(&bytes[from..], b'\n', 1)?;
        from = at + 1;
        if let Some(unit) = newline_unit_at(bytes, newline, at) {
            left -= 1;
            if left == 0 {
                return Some(unit);
            }
        }
    }
    None
}

/// Where in `bytes`, whole code units as wide as `newline`, the `k`th newline
/// unit from the end is, found as [`nth_unit`] finds one from the start.
fn nth_unit_back(bytes: &[u8], newline: &[u8], k: usize) -> Option<usize> {
    let mut left = k;
    let mut end = bytes.len();
    while left > 0 {
        let at = nth_byte_back(&bytes[..end], b'\n', 1)?;
        end = at;
        if let Some(unit) = newline_unit_at(bytes, newline, at) {
            left -= 1;
            if left == 0 {
                return Some(unit);
            }
        }
    }
    None
}

/// How many newline units `bytes`, whole code units as wide as `newline`,
/// hold, found as [`nth_unit`] finds them.
fn count_units(bytes: &[u8], newline: &[u8]) -> usize {
    let mut count = 0;
    let mut from = 0;
    while let Some(at) = nth_byte(&bytes[from..], b'\n', 1) {
        let at = from + at;
        from = at + 1;
        count += usize::from(newline_unit_at(bytes, newline, at).is_some());
    }
    count
}

/// The part of a stream `select` names under `unit`, reading no more of it
/// than a head or a range needs; a tail keeps only the lines or paragraphs
/// it may still keep as the stream goes by, at the offsets of its text past
/// any mark. A stream whose mark declares UTF-16 or UTF-32 is decoded as it
/// arrives, as a full read decodes it. A unit other than a line or a
/// paragraph, whose records are found from all of the input, reads the whole
/// stream. A UTF-8 window holding a NUL byte comes back marked binary, and
/// with no mark is decoded where `asked.binary` asks for such input as text;
/// the stream behind it has gone by, so only the window itself is decoded.
///
/// # Errors
///
/// The stream cannot be read.
pub fn read_stream(mut reader: impl Read, select: Select, unit: &RecordUnit, asked: Asked) -> io::Result<Window> {
    let ask = Ask { asked, chars: false };
    let mut opening = Vec::with_capacity(4);
    read_more(&mut reader, &mut opening, 4)?;
    let (encoding, mark) = Encoding::declared(&opening);
    if *unit != RecordUnit::Line && *unit != RecordUnit::Paragraph {
        let mut raw = opening;
        reader.read_to_end(&mut raw)?;
        return Ok(whole(raw, select, unit, ask).window);
    }
    let past_mark = opening.split_off(mark);
    if encoding != Encoding::Utf8 {
        let mut text = Decoding::new(past_mark.as_slice().chain(reader), encoding);
        let mut window = if *unit == RecordUnit::Line {
            stream_lines(&mut text, select, ask.count())?
        } else {
            stream_paragraphs(&mut text, select, ask.count())?
        }
        .window;
        window.encoding = encoding;
        return Ok(window);
    }
    let mut reader = past_mark.as_slice().chain(reader);
    let mut window = if *unit == RecordUnit::Line {
        stream_lines(&mut reader, select, ask.count())?
    } else {
        stream_paragraphs(&mut reader, select, ask.count())?
    }
    .window;
    if window.bytes.contains(&0) {
        window.binary = true;
        // Text a UTF-8 mark declared stays the bytes it is, as a full read
        // decodes it.
        if asked.binary && mark == 0 {
            window.bytes = crate::encoding::decode(std::mem::take(&mut window.bytes));
        }
    }
    Ok(window)
}

/// The paragraphs of a stream `select` names. A head or a range stops once
/// it has read one paragraph past its last; a tail keeps the last `n + 1`
/// paragraphs of what has arrived, since the last may still be growing. The
/// UTF-16 code units and the characters ahead of the window are counted
/// where `count` asks.
fn stream_paragraphs(reader: &mut impl Read, select: Select, count: Count) -> io::Result<Counted> {
    let unit = RecordUnit::Paragraph;
    let last = match select {
        Select::Head(n) => Some(n),
        Select::Range { to, .. } => to,
        Select::Tail(..) => None,
    };
    let mut held = Vec::new();
    let mut dropped = 0usize;
    let mut dropped_lines = 0usize;
    let mut gone = Tally::new(count);
    loop {
        let got = read_more(reader, &mut held, FIRST_READ)?;
        let records = unit.records(&held);
        if got == 0 || last.is_some_and(|b| records.len() > b) {
            let r = record_range(&held, select, &records);
            let line_base = dropped_lines + count_byte(&held[..r.start], b'\n');
            gone.add(&held[..r.start]);
            let before = Ahead { lines: Some(line_base), gone };
            return Ok(local(held[r.clone()].to_vec(), dropped + r.start, before, dropped + r.end));
        }
        if let Select::Tail(n) = select
            && records.len() > n + 1
        {
            let keep_from = records[records.len() - n - 1].0;
            dropped_lines += count_byte(&held[..keep_from], b'\n');
            gone.add(&held[..keep_from]);
            dropped += keep_from;
            held.drain(..keep_from);
        }
    }
}

/// The lines of a stream `select` names, the UTF-16 code units and the
/// characters ahead of the window counted where `count` asks.
fn stream_lines(reader: &mut impl Read, select: Select, count: Count) -> io::Result<Counted> {
    let mut gone = Tally::new(count);
    match select {
        Select::Head(n) => {
            let mut bytes = Vec::new();
            stream_through_line(reader, &mut bytes, n)?;
            let end = bytes.len();
            Ok(local(bytes, 0, Ahead::NONE, end))
        }
        Select::Range { from, to } => {
            let skip = from.map_or(0, |a| a - 1);
            let mut offset = 0usize;
            let mut bytes = Vec::new();
            let mut seen = 0usize;
            while seen < skip {
                if read_more(reader, &mut bytes, FIRST_READ)? == 0 {
                    let lines = seen + count_byte(&bytes, b'\n');
                    gone.add(&bytes);
                    let end = offset + bytes.len();
                    return Ok(local(Vec::new(), end, Ahead { lines: Some(lines), gone }, end));
                }
                if let Some(at) = nth_byte(&bytes, b'\n', skip - seen) {
                    gone.add(&bytes[..=at]);
                    bytes.drain(..=at);
                    offset += at + 1;
                    seen = skip;
                } else {
                    seen += count_byte(&bytes, b'\n');
                    gone.add(&bytes);
                    offset += bytes.len();
                    bytes.clear();
                }
            }
            match to {
                Some(b) => stream_through_line(reader, &mut bytes, b - skip)?,
                None => {
                    reader.read_to_end(&mut bytes)?;
                }
            }
            let end = offset + bytes.len();
            Ok(local(bytes, offset, Ahead { lines: Some(skip), gone }, end))
        }
        Select::Tail(n) => {
            let mut held = Vec::new();
            let mut dropped = 0usize;
            let mut dropped_lines = 0usize;
            loop {
                let got = read_more(reader, &mut held, FIRST_READ)?;
                // The last `n` lines of what has arrived are the most the tail
                // can still keep, whatever arrives after.
                let keep_from = tail_start(&held, n);
                if keep_from > 0 {
                    dropped_lines += count_byte(&held[..keep_from], b'\n');
                    gone.add(&held[..keep_from]);
                    dropped += keep_from;
                    held.drain(..keep_from);
                }
                if got == 0 {
                    let end = dropped + held.len();
                    return Ok(local(held, dropped, Ahead { lines: Some(dropped_lines), gone }, end));
                }
            }
        }
    }
}

/// Read a stream into `bytes`, which holds the start of the lines being
/// read, until it holds `n` newlines, cut just past the `n`th; the whole
/// stream where it has fewer.
fn stream_through_line(reader: &mut impl Read, bytes: &mut Vec<u8>, n: usize) -> io::Result<()> {
    if n == 0 {
        bytes.clear();
        return Ok(());
    }
    let mut seen = 0usize;
    let mut from = 0usize;
    loop {
        if let Some(at) = nth_byte(&bytes[from..], b'\n', n - seen) {
            bytes.truncate(from + at + 1);
            return Ok(());
        }
        seen += count_byte(&bytes[from..], b'\n');
        from = bytes.len();
        if read_more(reader, bytes, FIRST_READ)? == 0 {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(input: &[u8], select: Select, unit: &RecordUnit) -> String {
        String::from_utf8_lossy(&window_of(input, select, unit).bytes).into_owned()
    }

    #[test]
    fn a_head_and_a_tail_take_whole_lines_as_head_and_tail_print_them() {
        let input = b"one\ntwo\nthree\nfour\n";
        assert_eq!(text(input, Select::Head(2), &RecordUnit::Line), "one\ntwo\n");
        assert_eq!(text(input, Select::Tail(2), &RecordUnit::Line), "three\nfour\n");
        assert_eq!(text(input, Select::Head(9), &RecordUnit::Line), "one\ntwo\nthree\nfour\n");
        assert_eq!(text(input, Select::Tail(9), &RecordUnit::Line), "one\ntwo\nthree\nfour\n");
        assert_eq!(text(input, Select::Head(0), &RecordUnit::Line), "");
        assert_eq!(text(input, Select::Tail(0), &RecordUnit::Line), "");
        // With no newline at its end, the last line is still a line.
        assert_eq!(text(b"one\ntwo", Select::Tail(1), &RecordUnit::Line), "two");
        assert_eq!(text(b"one\ntwo", Select::Head(2), &RecordUnit::Line), "one\ntwo");
        // An empty line before the end is a line of its own.
        assert_eq!(text(b"a\nb\n\n", Select::Tail(1), &RecordUnit::Line), "\n");
    }

    #[test]
    fn a_range_takes_its_lines_and_knows_the_lines_before_it() {
        let input = b"one\ntwo\nthree\nfour\n";
        let w = window_of(input, Select::parse_range("2..3").expect("a range"), &RecordUnit::Line);
        assert_eq!(w.bytes, b"two\nthree\n");
        assert_eq!((w.byte_base, w.line_base), (Some(4), Some(1)));
        let w = window_of(input, Select::parse_range("3..").expect("a range"), &RecordUnit::Line);
        assert_eq!(w.bytes, b"three\nfour\n");
        assert_eq!(w.line_base, Some(2));
        let w = window_of(input, Select::parse_range("..1").expect("a range"), &RecordUnit::Line);
        assert_eq!(w.bytes, b"one\n");
        let w = window_of(input, Select::parse_range("9..").expect("a range"), &RecordUnit::Line);
        assert!(w.bytes.is_empty());
        let w = window_of(input, Select::Tail(1), &RecordUnit::Line);
        assert_eq!((w.byte_base, w.line_base), (Some(14), Some(3)));
    }

    #[test]
    fn a_range_that_names_nothing_or_runs_backward_is_refused() {
        for bad in ["", "..", "0..3", "3..2", "a..3", "2..b"] {
            assert!(Select::parse_range(bad).is_err(), "{bad:?} was accepted");
        }
        assert_eq!(Select::parse_range("4"), Ok(Select::Range { from: Some(4), to: Some(4) }));
    }

    #[test]
    fn a_record_unit_selects_whole_records() {
        let input = b"a one\na two\n\nb one\n\n\nc one\nc two\n";
        assert_eq!(text(input, Select::Head(1), &RecordUnit::Paragraph), "a one\na two\n");
        assert_eq!(text(input, Select::Tail(1), &RecordUnit::Paragraph), "c one\nc two\n");
        assert_eq!(text(input, Select::Range { from: Some(2), to: Some(2) }, &RecordUnit::Paragraph), "b one\n");
        let w = window_of(input, Select::Tail(2), &RecordUnit::Paragraph);
        assert_eq!(w.line_base, Some(3));
    }

    /// A file read through the readers holds the window an in-memory cut of
    /// the whole file holds, for every selection over lines and paragraphs,
    /// with reads that must grow past the first one.
    #[test]
    fn a_file_read_from_either_end_holds_what_the_whole_file_holds() {
        let dir = std::env::temp_dir().join(format!("trex-window-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the directory");
        let path = dir.join("lines.txt");
        let mut body = String::new();
        for i in 0..40_000 {
            body.push_str(&format!("line {i} of the file\n"));
            if i % 7 == 6 {
                body.push('\n');
            }
        }
        std::fs::write(&path, &body).expect("write the file");
        let input = body.as_bytes();
        let selects = [
            Select::Head(0),
            Select::Head(3),
            Select::Head(30_000),
            Select::Tail(1),
            Select::Tail(5_000),
            Select::Tail(50_000),
            Select::Range { from: Some(10), to: Some(20) },
            Select::Range { from: Some(35_000), to: None },
            Select::Range { from: None, to: Some(7) },
        ];
        let counted = Asked { numbers: true, units: true, ..Asked::default() };
        for unit in [RecordUnit::Line, RecordUnit::Paragraph] {
            for select in selects {
                let whole = window_of(input, select, &unit);
                let read = read_file(&path, select, &unit, counted).expect("read the file");
                assert_eq!(read, whole, "{select:?} over {unit:?}");
                // A stream is not measured, so it says where its window ends
                // rather than how long it was.
                let streamed = read_stream(input, select, &unit, counted).expect("read the stream");
                assert_eq!(
                    (&streamed.bytes, streamed.byte_base, streamed.line_base, streamed.unit_base),
                    (&whole.bytes, whole.byte_base, whole.line_base, whole.unit_base),
                    "{select:?} over {unit:?} from a stream"
                );
            }
        }
        let unnumbered = read_file(&path, Select::Tail(3), &RecordUnit::Line, Asked::default()).expect("read the file");
        assert_eq!(unnumbered.line_base, None);
        std::fs::remove_dir_all(&dir).expect("remove the directory");
    }

    /// The lines ahead of a tail, counted a block at a time on the cores, are
    /// the newlines a count of the whole prefix finds, whatever the blocks'
    /// size and wherever the prefix begins and ends.
    #[test]
    fn the_lines_counted_in_blocks_are_the_lines_the_prefix_holds() {
        let dir = std::env::temp_dir().join(format!("trex-window-count-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the directory");
        let path = dir.join("lines.txt");
        let body: Vec<u8> = (0..20_000u32).flat_map(|i| format!("{i}\n{}", " ".repeat((i % 13) as usize)).into_bytes()).collect();
        std::fs::write(&path, &body).expect("write the file");
        let file = std::fs::File::open(&path).expect("open the file");
        for from in [0, 3, 1000] {
            for end in [1000, 1001, 65_537, body.len()] {
                for block in [1000, 4096, 1 << 20] {
                    let both = Count { units: true, chars: true };
                    let counted = count_lines_in_blocks(&file, from, end, block, both).expect("count the lines");
                    let prefix = &body[from..end];
                    let chars = std::str::from_utf8(prefix).expect("the body is ASCII").chars().count();
                    let expected = (
                        count_byte(prefix, b'\n'),
                        Tally { units: Some(crate::encoding::utf16_units(prefix)), chars: Some(chars) },
                    );
                    assert_eq!(counted, expected, "{from}..{end} in blocks of {block}");
                }
            }
        }
        drop(file);
        std::fs::remove_dir_all(&dir).expect("remove the directory");
    }

    /// The text of `body` as a file opening with the mark of `encoding`.
    fn marked(body: &str, encoding: Encoding) -> Vec<u8> {
        match encoding {
            Encoding::Utf8 => [&[0xEF, 0xBB, 0xBF][..], body.as_bytes()].concat(),
            Encoding::Utf16Le => [0xFF, 0xFE].into_iter().chain(body.encode_utf16().flat_map(u16::to_le_bytes)).collect(),
            Encoding::Utf16Be => [0xFE, 0xFF].into_iter().chain(body.encode_utf16().flat_map(u16::to_be_bytes)).collect(),
            Encoding::Utf32Le => {
                [0xFF, 0xFE, 0x00, 0x00].into_iter().chain(body.chars().flat_map(|c| (c as u32).to_le_bytes())).collect()
            }
            Encoding::Utf32Be => {
                [0x00, 0x00, 0xFE, 0xFF].into_iter().chain(body.chars().flat_map(|c| (c as u32).to_be_bytes())).collect()
            }
        }
    }

    const MARKED: [Encoding; 5] =
        [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Utf32Le, Encoding::Utf32Be];

    /// A file opening with any mark, read from the end nearest its window,
    /// holds the window the whole file decoded and cut holds, at the same
    /// place in its text, for every selection over lines and paragraphs and
    /// with reads that must grow past the first; a stream of it holds the
    /// same window.
    #[test]
    fn a_marked_file_read_from_either_end_holds_what_the_whole_file_decoded_holds() {
        let dir = std::env::temp_dir().join(format!("trex-window-marked-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the directory");
        let mut body = String::new();
        for i in 0..12_000 {
            body.push_str(&format!("line {i} caf\u{E9} \u{4E2D}\u{6587} \u{1F600}\n"));
            if i % 7 == 6 {
                body.push('\n');
            }
        }
        let selects = [
            Select::Head(0),
            Select::Head(3),
            Select::Head(9_000),
            Select::Tail(1),
            Select::Tail(1_500),
            Select::Tail(20_000),
            Select::Range { from: Some(10), to: Some(20) },
            Select::Range { from: Some(11_000), to: None },
            Select::Range { from: None, to: Some(7) },
        ];
        let placed = Asked { numbers: true, offsets: true, units: true, ..Asked::default() };
        for encoding in MARKED {
            let raw = marked(&body, encoding);
            let path = dir.join(format!("{encoding:?}.txt"));
            std::fs::write(&path, &raw).expect("write the file");
            for unit in [RecordUnit::Line, RecordUnit::Paragraph] {
                for select in selects {
                    let whole = whole(raw.clone(), select, &unit, Ask { asked: placed, chars: false }).window;
                    let read = read_file(&path, select, &unit, placed).expect("read the file");
                    assert_eq!(read, whole, "{encoding:?}: {select:?} over {unit:?}");
                    let streamed = read_stream(raw.as_slice(), select, &unit, placed).expect("read the stream");
                    assert_eq!(
                        (&streamed.bytes, streamed.byte_base, streamed.line_base, streamed.unit_base, streamed.encoding),
                        (&whole.bytes, whole.byte_base, whole.line_base, whole.unit_base, whole.encoding),
                        "{encoding:?}: {select:?} over {unit:?} from a stream"
                    );
                }
            }
            // A tail not asked for its place holds the same text and knows
            // its place only where its text is UTF-8 or its units were asked
            // for in UTF-32.
            let tail = whole(raw.clone(), Select::Tail(5), &RecordUnit::Line, Ask { asked: placed, chars: false }).window;
            let bare = read_file(&path, Select::Tail(5), &RecordUnit::Line, Asked::default()).expect("read the file");
            assert_eq!(bare.bytes, tail.bytes, "{encoding:?}");
            let known = encoding == Encoding::Utf8;
            assert_eq!(bare.byte_base.is_some(), known, "{encoding:?}");
            let units = Asked { units: true, ..Asked::default() };
            let counted = read_file(&path, Select::Tail(5), &RecordUnit::Line, units).expect("read the file");
            assert_eq!(counted.unit_base, tail.unit_base, "{encoding:?}");
        }
        std::fs::remove_dir_all(&dir).expect("remove the directory");
    }

    /// What the text ahead of an encoded tail decodes to, counted a block at
    /// a time, is what the prefix decoded whole holds, wherever a surrogate
    /// pair or a lone surrogate is relative to the blocks' edges.
    #[test]
    fn the_text_ahead_counted_in_blocks_is_the_prefix_decoded_whole() {
        let dir = std::env::temp_dir().join(format!("trex-window-decode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the directory");
        let body = "a\u{1F600}b\n\u{1F600}\u{1F600}\nc\u{E9}\u{4E2D}\n\u{10FFFF}d\n";
        for encoding in MARKED {
            let mut raw = marked(body, encoding);
            let width = encoding.unit_width();
            if width == 2 {
                // A high surrogate with no partner, and a low one.
                let (lone_high, lone_low) = if encoding == Encoding::Utf16Le {
                    ([0x00, 0xD8], [0x00, 0xDC])
                } else {
                    ([0xD8, 0x00], [0xDC, 0x00])
                };
                raw.extend(lone_high);
                raw.extend(encoding.newline());
                raw.extend(lone_low);
                raw.extend(encoding.newline());
            }
            let path = dir.join(format!("{encoding:?}.txt"));
            std::fs::write(&path, &raw).expect("write the file");
            let file = std::fs::File::open(&path).expect("open the file");
            let mark = Encoding::declared(&raw).1;
            for end in (mark..=raw.len()).step_by(width) {
                let text = decode_units(&raw[mark..end], encoding);
                let expected = (
                    count_byte(&text, b'\n'),
                    text.len(),
                    crate::encoding::utf16_units(&text),
                    // A UTF-8 prefix may end inside a character, which reads
                    // as one U+FFFD, the one character its first byte opens.
                    String::from_utf8_lossy(&text).chars().count(),
                );
                for block in [width, 2 * width, 3 * width, 5 * width, 1 << 20] {
                    let counted = count_encoded_ahead(&file, encoding, mark, end, block).expect("count the text");
                    assert_eq!(
                        (counted.lines, counted.bytes, counted.units, counted.chars),
                        expected,
                        "{encoding:?}: {mark}..{end} in blocks of {block}"
                    );
                }
            }
            drop(file);
        }
        std::fs::remove_dir_all(&dir).expect("remove the directory");
    }

    /// An input that ends with a surrogate still waiting for its pair, or
    /// with part of a unit, reads at its end as a whole read reads it.
    #[test]
    fn an_encoded_stream_ends_as_the_whole_stream_decoded_ends() {
        for (raw, encoding) in [
            (vec![0xFF, 0xFE, b'a', 0x00, 0x3D, 0xD8], Encoding::Utf16Le),
            (vec![0xFF, 0xFE, b'a', 0x00, b'b'], Encoding::Utf16Le),
            (vec![0xFE, 0xFF, 0x00, b'a', 0xD8, 0x3D], Encoding::Utf16Be),
            (vec![0xFF, 0xFE, 0x00, 0x00, b'a', 0, 0, 0, b'b', 0], Encoding::Utf32Le),
        ] {
            let mark = Encoding::declared(&raw).1;
            let mut text = Vec::new();
            Decoding::new(&raw[mark..], encoding).read_to_end(&mut text).expect("read the stream");
            assert_eq!(text, crate::encoding::decode(raw.clone()), "{raw:02X?}");
        }
    }

    /// Ahead of a window of text beyond ASCII, the UTF-16 code units counted
    /// are those of the text a UTF-16 string of the whole input holds there,
    /// read from either end of a file or from a stream.
    #[test]
    fn the_units_ahead_of_a_window_are_the_whole_text_s() {
        let dir = std::env::temp_dir().join(format!("trex-window-units-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the directory");
        let path = dir.join("wide.txt");
        let body = "caf\u{E9} one\n\u{4E2D}\u{6587} two\n\u{1F600} three\nplain four\nlast five\n";
        std::fs::write(&path, body).expect("write the file");
        let asked = Asked { units: true, ..Asked::default() };
        for select in [Select::Tail(2), Select::Range { from: Some(3), to: None }, Select::Head(1)] {
            let read = read_file(&path, select, &RecordUnit::Line, asked).expect("read the file");
            let prefix = &body[..read.byte_base.expect("a UTF-8 window knows its place")];
            assert_eq!(read.unit_base, Some(prefix.encode_utf16().count()), "{select:?}");
            let streamed = read_stream(body.as_bytes(), select, &RecordUnit::Line, asked).expect("read the stream");
            assert_eq!(streamed.unit_base, read.unit_base, "{select:?} from a stream");
        }
        std::fs::remove_dir_all(&dir).expect("remove the directory");
    }

    /// Ahead of a window of a file opening with any mark, read from either
    /// end, the characters counted are those the whole file decoded holds
    /// there, and the window is the one a read that counts none holds.
    #[test]
    fn the_characters_ahead_of_a_window_are_the_whole_text_s() {
        let dir = std::env::temp_dir().join(format!("trex-window-chars-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the directory");
        let mut body = String::new();
        for i in 0..3_000 {
            body.push_str(&format!("line {i} caf\u{E9} \u{4E2D}\u{6587} \u{1F600}\n"));
            if i % 7 == 6 {
                body.push('\n');
            }
        }
        let selects = [
            Select::Head(3),
            Select::Tail(1),
            Select::Tail(500),
            Select::Range { from: Some(10), to: Some(20) },
            Select::Range { from: Some(2_000), to: None },
        ];
        for encoding in MARKED {
            let raw = marked(&body, encoding);
            let text = crate::encoding::decode(raw.clone());
            let path = dir.join(format!("{encoding:?}.txt"));
            std::fs::write(&path, &raw).expect("write the file");
            for unit in [RecordUnit::Line, RecordUnit::Paragraph] {
                for select in selects {
                    let (window, chars) =
                        read_file_with_chars(&path, select, &unit, Asked::default()).expect("read the file");
                    let at = window.byte_base.expect("a window whose characters are counted is placed");
                    let expected = std::str::from_utf8(&text[..at]).expect("decoded text is UTF-8").chars().count();
                    assert_eq!(chars, Some(expected), "{encoding:?}: {select:?} over {unit:?}");
                    let plain = read_file(&path, select, &unit, Asked::default()).expect("read the file");
                    assert_eq!(window.bytes, plain.bytes, "{encoding:?}: {select:?} over {unit:?}");
                }
            }
        }
        std::fs::remove_dir_all(&dir).expect("remove the directory");
    }

    /// A window holding a NUL byte comes back marked binary without the rest
    /// of the file read, or read whole and decoded where binary input is read
    /// as text; a file opening with a byte order mark is read as the text it
    /// declares, its tail placed in that text only where it is asked to be,
    /// and says which encoding its mark declared.
    #[test]
    fn a_nul_marks_a_window_binary_and_a_mark_is_read_as_the_text_it_declares() {
        let dir = std::env::temp_dir().join(format!("trex-window-enc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the directory");
        let bin = dir.join("tail.bin");
        std::fs::write(&bin, b"text\nmore\nzero\0here\n").expect("write the file");
        let marked = read_file(&bin, Select::Tail(1), &RecordUnit::Line, Asked::default()).expect("read the file");
        assert!(marked.binary);
        assert_eq!(marked.bytes, b"zero\0here\n");
        let as_text = Asked { binary: true, ..Asked::default() };
        let read = read_file(&bin, Select::Tail(1), &RecordUnit::Line, as_text).expect("read the file");
        assert!(read.binary);
        assert_eq!((read.bytes.as_slice(), read.line_base), (&b"zero\0here\n"[..], Some(2)));
        // A NUL outside the window is not read, so it marks nothing.
        let clean = read_file(&bin, Select::Head(2), &RecordUnit::Line, Asked::default()).expect("read the file");
        assert!(!clean.binary);

        let utf16 = dir.join("log.txt");
        let text = "one\ntwo \u{4E2D}\nthree\n";
        let mut raw = vec![0xFF, 0xFE];
        raw.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(&utf16, &raw).expect("write the file");
        let w = read_file(&utf16, Select::Tail(2), &RecordUnit::Line, Asked::default()).expect("read the file");
        assert_eq!(w.bytes, "two \u{4E2D}\nthree\n".as_bytes());
        assert_eq!((w.byte_base, w.line_base, w.binary), (None, None, false));
        assert_eq!((w.encoding, w.input_len), (Encoding::Utf16Le, raw.len()));
        let placed = Asked { offsets: true, ..Asked::default() };
        let w = read_file(&utf16, Select::Tail(2), &RecordUnit::Line, placed).expect("read the file");
        assert_eq!((w.byte_base, w.line_base), (Some(4), Some(1)));
        let streamed = read_stream(raw.as_slice(), Select::Head(1), &RecordUnit::Line, Asked::default())
            .expect("read the stream");
        assert_eq!(streamed.bytes, b"one\n");
        std::fs::remove_dir_all(&dir).expect("remove the directory");
    }

    #[test]
    fn a_tail_and_an_open_range_run_to_the_end() {
        assert!(Select::Tail(3).runs_to_end());
        assert!(Select::Range { from: Some(4), to: None }.runs_to_end());
        assert!(!Select::Head(3).runs_to_end());
        assert!(!Select::Range { from: None, to: Some(9) }.runs_to_end());
    }

    /// The newline units the byte search finds are the ones a walk over every
    /// unit finds, in text whose other units hold the newline byte, whole or
    /// straddling two of them.
    #[test]
    fn the_newline_units_found_by_the_byte_search_are_those_a_walk_finds() {
        let text = "a\n\u{0A41}\u{4E00}\n\u{010A}\u{0A0A}\n\n\u{10A0A}x\u{0A00}\n\u{0A}";
        for encoding in [Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Utf32Le, Encoding::Utf32Be] {
            let bytes = crate::encoding::encode(text.as_bytes(), encoding).into_owned();
            let newline = encoding.newline();
            let width = newline.len();
            let walked: Vec<usize> =
                bytes.chunks_exact(width).enumerate().filter(|(_, u)| *u == newline).map(|(i, _)| i * width).collect();
            assert_eq!(walked.len(), 6, "{encoding:?}");
            assert_eq!(count_units(&bytes, newline), walked.len(), "{encoding:?}");
            assert_eq!(nth_unit(&bytes, newline, 0), None);
            assert_eq!(nth_unit_back(&bytes, newline, 0), None);
            for k in 1..=walked.len() + 1 {
                assert_eq!(nth_unit(&bytes, newline, k), walked.get(k - 1).copied(), "{encoding:?} {k}");
                assert_eq!(nth_unit_back(&bytes, newline, k), walked.iter().rev().nth(k - 1).copied(), "{encoding:?} {k}");
            }
        }
    }
}
