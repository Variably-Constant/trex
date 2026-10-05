//! An input read as it arrives: a file followed as it grows, the part read
//! and every byte the file gains after it scanned as one stream, or the
//! strings piped into Select-TrexMatch, scanned as the lines of one input.
//! For Select-TrexMatch each match is handed back once nothing that arrives
//! later can change it, placed at the input's own line, byte offset and
//! UTF-16 offset; for Edit-TrexText and Protect-TrexText -Follow each line of
//! the rewritten text is.

use std::path::PathBuf;

use pwrs::prelude::*;

use crate::matching::Scanning;
use crate::pattern::Compiled;
use crate::window::Read;

/// The matches a followed file settled at once: the text they were found
/// in, placed as the file places it, and each match with the member of a set
/// that made it.
pub(crate) struct Batch {
    pub(crate) read: Read,
    pub(crate) found: Vec<(Option<usize>, trex::Match)>,
}

/// One file -Follow scans as it grows, or the one input strings piped in
/// make, which has no path.
pub(crate) struct FollowedScan {
    pub(crate) path: PathBuf,
    pub(crate) shown: String,
    /// Where following the file picks up: its length when the part was read.
    offset: usize,
    decoder: trex::encoding::Incremental,
    /// The opening bytes of a UTF-8 character a piece ended inside, held
    /// until the rest arrives.
    carry: Vec<u8>,
    stream: trex::HeldStream,
    /// The UTF-16 code units of the file's text up to what has been pushed.
    units_through: usize,
    /// How many matches have been written, for -MaxCount.
    pub(crate) written: usize,
    /// Which members of a set have written a match, for -SingleMatch.
    pub(crate) members_seen: Vec<bool>,
}

impl FollowedScan {
    /// The file at `path` scanned from `read`, the part of it read, which is
    /// pushed with [`Self::push_text`], on.
    pub(crate) fn new(path: PathBuf, shown: String, read: &Read, sc: &Scanning) -> PsResult<FollowedScan> {
        let members = match sc {
            Scanning::One(_) => 1,
            Scanning::Set(set) => set.len(),
        };
        Ok(FollowedScan {
            path,
            shown,
            offset: read.input_len,
            decoder: trex::encoding::Incremental::after(read.encoding),
            carry: Vec::new(),
            stream: trex::HeldStream::new(stream_scanner(sc), read.base()?, read.line_base),
            units_through: read.units_ahead()?,
            written: 0,
            members_seen: vec![false; members],
        })
    }

    /// Where following the file picks up.
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Scan `text`, the file's next text, and the matches it settles.
    pub(crate) fn push_text(&mut self, text: &str, sc: &Scanning) -> Option<Batch> {
        self.units_through += trex::encoding::utf16_units(text.as_bytes());
        let committed = self.stream.push(text.as_bytes());
        let held = self.stream.held().to_vec();
        batch_of(committed, held, self.stream.base(), self.stream.lines_before(), self.units_through, sc)
    }

    /// Scan `bytes`, what the file gained, decoded as its opening declared,
    /// and the matches they settle; a character cut between two pieces is
    /// held until the rest arrives.
    pub(crate) fn push_bytes(&mut self, bytes: &[u8], sc: &Scanning) -> Option<Batch> {
        let mut text = std::mem::take(&mut self.carry);
        text.extend_from_slice(&self.decoder.decode(bytes));
        let whole = complete_prefix(&text);
        self.carry = text.split_off(whole);
        let text = String::from_utf8_lossy(&text).into_owned();
        self.push_text(&text, sc)
    }

    /// The input ended: the matches its stream held until then, the stream
    /// left holding nothing.
    pub(crate) fn finish(&mut self, sc: &Scanning) -> Option<Batch> {
        let fresh = trex::HeldStream::new(stream_scanner(sc), 0, Some(0));
        let ended = std::mem::replace(&mut self.stream, fresh).finish();
        batch_of(ended.matches, ended.held, ended.base, ended.lines_before, self.units_through, sc)
    }

    /// The file was truncated, replaced or removed: the matches its stream
    /// held until then, and the stream begun again at the start of the file
    /// now under its name.
    pub(crate) fn restart(&mut self, sc: &Scanning) -> Option<Batch> {
        let batch = self.finish(sc);
        self.units_through = 0;
        self.carry.clear();
        self.decoder = trex::encoding::Incremental::from_start();
        batch
    }
}

/// The edits a followed rewrite makes at the matches a push committed: from
/// the text held, the committed spans over it, and the UTF-16 code units of
/// the file ahead of the held text where they are counted, to edits of the
/// held text.
pub(crate) type EditsAt<'a> =
    dyn FnMut(&[u8], &[trex::Span], Option<usize>) -> PsResult<Vec<trex::files::Edit>> + 'a;

/// One file -Follow rewrites as it grows, which writes the rewritten text a
/// line at a time: each line once nothing that arrives later can change it.
pub(crate) struct FollowedEdit {
    pub(crate) path: PathBuf,
    pub(crate) shown: String,
    /// Where following the file picks up: its length when the part was read.
    offset: usize,
    decoder: trex::encoding::Incremental,
    /// The opening bytes of a UTF-8 character a piece ended inside, held
    /// until the rest arrives.
    carry: Vec<u8>,
    /// The file's text edited as it arrives, its offsets counted from the
    /// part read, and the matches edited, which -MaxCount caps.
    stream: trex::EditStream,
    /// The UTF-16 code units of the file's text up to what has been pushed,
    /// where the part read had those ahead of it counted.
    units_through: Option<usize>,
    /// The rewritten text of a line whose end has not arrived.
    pending: String,
}

impl FollowedEdit {
    /// The file at `path` rewritten for `c` from `read`, the part of it read,
    /// which is pushed with [`Self::push_text`], on.
    pub(crate) fn new(path: PathBuf, shown: String, read: &Read, c: &Compiled) -> FollowedEdit {
        FollowedEdit {
            path,
            shown,
            offset: read.input_len,
            decoder: trex::encoding::Incremental::after(read.encoding),
            carry: Vec::new(),
            // The stream's offsets only say how far it has written, so they
            // count from the part read; the edits place a match by its UTF-16
            // units.
            stream: trex::EditStream::new(edit_scanner(c), 0, 0),
            units_through: read.unit_base,
            pending: String::new(),
        }
    }

    /// Whether a match can be rewritten before the file ends.
    pub(crate) fn commits_early(&self) -> bool {
        self.stream.commits_early()
    }

    /// Where following the file picks up.
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Rewrite `text`, the file's next text, replacing no more than `limit`
    /// matches in all: the lines it completes.
    pub(crate) fn push_text(
        &mut self,
        text: &str,
        limit: Option<usize>,
        edits: &mut EditsAt<'_>,
    ) -> PsResult<Vec<String>> {
        if let Some(units) = self.units_through.as_mut() {
            *units += trex::encoding::utf16_units(text.as_bytes());
        }
        let units_through = self.units_through;
        let mut placed = |held: &[u8], _base: usize, spans: &[trex::Span]| {
            edits(held, spans, units_through.map(|u| u - trex::encoding::utf16_units(held)))
        };
        let out = self.stream.push(text.as_bytes(), limit, &mut placed)?;
        self.lines_of(&out)
    }

    /// Rewrite `bytes`, what the file gained, decoded as its opening
    /// declared; a character cut between two pieces is held until the rest
    /// arrives.
    pub(crate) fn push_bytes(
        &mut self,
        bytes: &[u8],
        limit: Option<usize>,
        edits: &mut EditsAt<'_>,
    ) -> PsResult<Vec<String>> {
        let mut text = std::mem::take(&mut self.carry);
        text.extend_from_slice(&self.decoder.decode(bytes));
        let whole = complete_prefix(&text);
        self.carry = text.split_off(whole);
        let text = String::from_utf8(text).map_err(|e| {
            PsError::new(ErrorCategory::InvalidData, "TrexFollow", format!("{}: {e}", self.shown))
        })?;
        self.push_text(&text, limit, edits)
    }

    /// The file was truncated, replaced or removed: the rest of its text
    /// rewritten, every line of it, the last whether or not its end arrived,
    /// and the stream begun again at the start of the file now under its
    /// name. That file is a new input, whose -MaxCount
    /// count starts again unless `keep_count` carries the old one's on.
    pub(crate) fn restart(
        &mut self,
        c: &Compiled,
        limit: Option<usize>,
        keep_count: bool,
        edits: &mut EditsAt<'_>,
    ) -> PsResult<Vec<String>> {
        let units_through = self.units_through;
        let mut placed = |held: &[u8], _base: usize, spans: &[trex::Span]| {
            edits(held, spans, units_through.map(|u| u - trex::encoding::utf16_units(held)))
        };
        let ended = std::mem::replace(&mut self.stream, trex::EditStream::new(edit_scanner(c), 0, 0));
        let (out, edited) = ended.finish(limit, &mut placed)?;
        if keep_count {
            self.stream = trex::EditStream::new(edit_scanner(c), 0, edited);
        }
        let mut lines = self.lines_of(&out)?;
        if !self.pending.is_empty() {
            let last = std::mem::take(&mut self.pending);
            lines.push(last.strip_suffix('\r').unwrap_or(&last).to_string());
        }
        self.units_through = self.units_through.map(|_| 0);
        self.carry.clear();
        self.decoder = trex::encoding::Incremental::from_start();
        Ok(lines)
    }

    /// The lines `out`, the next of the rewritten text, completes, each
    /// without its ending; a line whose end has not arrived is kept for the
    /// next.
    fn lines_of(&mut self, out: &[u8]) -> PsResult<Vec<String>> {
        let out = std::str::from_utf8(out)
            .map_err(|e| PsError::new(ErrorCategory::InvalidData, "TrexFollow", format!("{}: {e}", self.shown)))?;
        self.pending.push_str(out);
        let Some(last) = self.pending.rfind('\n') else {
            return Ok(Vec::new());
        };
        let done: String = self.pending.drain(..=last).collect();
        Ok(done[..last].split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line).to_string()).collect())
    }
}

/// The stream scanner a followed rewrite runs: the pattern under its
/// declarations.
fn edit_scanner(c: &Compiled) -> trex::StreamScanner {
    trex::StreamScanner::with_shapes(c.inner.clone(), c.shapes.clone())
}

/// The stream scanner a followed file is scanned by: the one pattern under
/// its declarations, or the set.
fn stream_scanner(sc: &Scanning) -> trex::StreamScanner {
    match sc {
        Scanning::One(c) => trex::StreamScanner::with_shapes(c.inner.clone(), c.shapes.clone()),
        Scanning::Set(set) => trex::StreamScanner::over_set((**set).clone()),
    }
}

/// `committed`, spans over `held` with the member that made each, as a
/// batch: the held text placed at `base`, after `lines_before` lines and
/// before whatever of the `units_through` code units the held text does not
/// make up, and each span's registers resolved over it.
fn batch_of(
    committed: Vec<(usize, trex::Span)>,
    held: Vec<u8>,
    base: usize,
    lines_before: Option<usize>,
    units_through: usize,
    sc: &Scanning,
) -> Option<Batch> {
    if committed.is_empty() {
        return None;
    }
    let units_ahead = units_through - trex::encoding::utf16_units(&held);
    let mut found: Vec<(Option<usize>, trex::Match)> = Vec::with_capacity(committed.len());
    match sc {
        Scanning::One(c) => {
            let spans: Vec<trex::Span> = committed.iter().map(|&(_, s)| s).collect();
            let resolved = trex::captures_with_shapes_and_lists(&c.inner, &held, &c.shapes, &spans);
            found.extend(resolved.into_iter().map(|m| (None, m)));
        }
        Scanning::Set(set) => {
            let mut members: Vec<usize> = committed.iter().map(|&(member, _)| member).collect();
            members.sort_unstable();
            members.dedup();
            let mut placed: Vec<(usize, usize, trex::Match)> = Vec::with_capacity(committed.len());
            for member in members {
                let at: Vec<usize> = (0..committed.len()).filter(|&k| committed[k].0 == member).collect();
                let spans: Vec<trex::Span> = at.iter().map(|&k| committed[k].1).collect();
                let pattern = &set.patterns()[member];
                let resolved = trex::captures_with_shapes_and_lists(pattern, &held, set.shapes(), &spans);
                placed.extend(at.into_iter().zip(resolved).map(|(k, m)| (k, member, m)));
            }
            placed.sort_by_key(|&(k, _, _)| k);
            found.extend(placed.into_iter().map(|(_, member, m)| (Some(member), m)));
        }
    }
    let end = base + held.len();
    let read = Read {
        text: String::from_utf8_lossy(&held).into_owned(),
        byte_base: Some(base),
        line_base: lines_before,
        unit_base: Some(units_ahead),
        input_len: end,
        encoding: trex::encoding::Encoding::Utf8,
    };
    Some(Batch { read, found })
}

pub(crate) use trex::encoding::complete_prefix;
