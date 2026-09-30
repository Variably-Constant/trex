//! Input transcoding: accept text in any UTF encoding, hand the lexer UTF-8.
//!
//! A BOM is an explicit declaration and selects UTF-32 LE/BE, UTF-16 LE/BE,
//! or UTF-8 (the BOM is stripped). Without a BOM the decoder is conservative
//! by construction, so a wrong guess cannot silently corrupt a scan:
//!
//! - bytes that are valid UTF-8 (which includes all ASCII and much binary)
//!   pass through unchanged - the decoder never guesses against valid UTF-8;
//! - otherwise the bytes are accepted as BOM-less UTF-16 only when a strict
//!   full decode succeeds - even length, no unpaired surrogates - and the
//!   decoded text is control-clean (no control chars beyond tab / newline /
//!   carriage return) with at least one letter;
//! - when exactly one endianness passes, it is used; when both pass, the
//!   ASCII null-parity of the raw bytes picks the endianness (an ASCII char
//!   in UTF-16 LE puts its zero byte at odd offsets, BE at even), and a text
//!   with no ASCII signal at all falls to little-endian, the convention of
//!   every mainstream BOM-less UTF-16 producer;
//! - anything else - odd length, invalid units, control-laden decodes, plain
//!   binary - is returned unchanged and scanned as the bytes it is.

use std::borrow::Cow;

/// Decode `raw` to UTF-8 text per the module rules. Total: never fails,
/// never panics; input that is not confidently text comes back unchanged.
#[must_use]
pub fn decode(raw: Vec<u8>) -> Vec<u8> {
    match recognized(&raw) {
        Recognized::Marked(encoding, mark) => decode_units(&raw[mark..], encoding),
        Recognized::Unmarked(_, text) => text.into_bytes(),
        Recognized::Bytes => raw,
    }
}

/// The text of `raw` as [`decode`] reads it, borrowed from `raw` wherever
/// the bytes past the mark are the text.
#[must_use]
pub fn text_of(raw: &[u8]) -> Cow<'_, [u8]> {
    match recognized(raw) {
        Recognized::Marked(Encoding::Utf8, mark) => Cow::Borrowed(&raw[mark..]),
        Recognized::Marked(encoding, mark) => Cow::Owned(decode_units(&raw[mark..], encoding)),
        Recognized::Unmarked(_, text) => Cow::Owned(text.into_bytes()),
        Recognized::Bytes => Cow::Borrowed(raw),
    }
}

/// The encoding [`decode`] reads `raw` in, and how many bytes of byte order
/// mark it reads past: UTF-8 and none for bytes it takes as they are.
#[must_use]
pub fn read_as(raw: &[u8]) -> (Encoding, usize) {
    match recognized(raw) {
        Recognized::Marked(encoding, mark) => (encoding, mark),
        Recognized::Unmarked(encoding, _) => (encoding, 0),
        Recognized::Bytes => (Encoding::Utf8, 0),
    }
}

/// How [`decode`] reads an input.
enum Recognized {
    /// A byte order mark declares the encoding and takes this many bytes.
    Marked(Encoding, usize),
    /// No mark, and a strict decode read UTF-16 text: its byte order, and
    /// the text.
    Unmarked(Encoding, String),
    /// The bytes are the text.
    Bytes,
}

/// How [`decode`] reads `raw`, per the module rules.
fn recognized(raw: &[u8]) -> Recognized {
    // Explicit BOMs first; UTF-32 LE before UTF-16 LE, whose BOM it extends.
    let (encoding, mark) = Encoding::declared(raw);
    if mark > 0 {
        return Recognized::Marked(encoding, mark);
    }
    // No BOM: never guess against NUL-free valid UTF-8. (UTF-16 ASCII is
    // itself valid UTF-8 - every other byte is a NUL - so the pass-through
    // rule requires the absence of NUL, which no real text file contains.)
    if !raw.contains(&0) && std::str::from_utf8(raw).is_ok() {
        return Recognized::Bytes;
    }
    // Detection needs evidence: below four chars any byte pair is as likely
    // binary as text, so short inputs stay raw.
    if raw.len() < 8 {
        return Recognized::Bytes;
    }
    let le = strict_utf16(raw, |c| u16::from_le_bytes(*c));
    let be = strict_utf16(raw, |c| u16::from_be_bytes(*c));
    match (le, be) {
        (Some(s), None) => Recognized::Unmarked(Encoding::Utf16Le, s),
        (None, Some(s)) => Recognized::Unmarked(Encoding::Utf16Be, s),
        (Some(l), Some(b)) => {
            // ASCII null-parity: an ASCII char's zero byte sits at odd
            // offsets in LE, even in BE. Require a decisive margin.
            let (even_nulls, odd_nulls) = null_parity(raw);
            if odd_nulls > even_nulls.saturating_mul(4) {
                Recognized::Unmarked(Encoding::Utf16Le, l)
            } else if even_nulls > odd_nulls.saturating_mul(4) {
                Recognized::Unmarked(Encoding::Utf16Be, b)
            } else {
                // No ASCII signal (a spaceless non-Latin line): little-endian
                // by convention.
                Recognized::Unmarked(Encoding::Utf16Le, l)
            }
        }
        (None, None) => Recognized::Bytes,
    }
}

/// `text`, UTF-8, in `encoding`: as it is for UTF-8, and read as a string
/// reads UTF-8, each ill-formed part one U+FFFD, for the others.
#[must_use]
pub(crate) fn encode(text: &[u8], encoding: Encoding) -> Cow<'_, [u8]> {
    let chars = || String::from_utf8_lossy(text);
    match encoding {
        Encoding::Utf8 => Cow::Borrowed(text),
        Encoding::Utf16Le => Cow::Owned(chars().encode_utf16().flat_map(u16::to_le_bytes).collect()),
        Encoding::Utf16Be => Cow::Owned(chars().encode_utf16().flat_map(u16::to_be_bytes).collect()),
        Encoding::Utf32Le => Cow::Owned(chars().chars().flat_map(|c| u32::from(c).to_le_bytes()).collect()),
        Encoding::Utf32Be => Cow::Owned(chars().chars().flat_map(|c| u32::from(c).to_be_bytes()).collect()),
    }
}

/// A stretch of an input's bytes and the UTF-8 text it decodes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    /// Bytes that are their own text, offset for offset.
    Same(usize),
    /// A code unit, a surrogate pair, an ill-formed part of UTF-8 or the
    /// trailing part of a unit, `bytes` long, and the `text` bytes it
    /// decodes to whole; no offset of the text falls inside them.
    Unit { bytes: usize, text: usize },
}

impl Run {
    fn bytes(self) -> usize {
        match self {
            Run::Same(n) | Run::Unit { bytes: n, .. } => n,
        }
    }

    fn text(self) -> usize {
        match self {
            Run::Same(n) | Run::Unit { text: n, .. } => n,
        }
    }
}

/// The runs an input's bytes past its mark make, from `at` on: in
/// `encoding` as [`decode_units`] reads them, and with `lossy` UTF-8 read as
/// a string reads it, each ill-formed part one U+FFFD.
struct Runs<'a> {
    units: &'a [u8],
    encoding: Encoding,
    lossy: bool,
    at: usize,
}

impl Iterator for Runs<'_> {
    type Item = Run;

    fn next(&mut self) -> Option<Run> {
        let rest = &self.units[self.at..];
        let run = match self.encoding {
            _ if rest.is_empty() => return None,
            Encoding::Utf8 if !self.lossy => Run::Same(rest.len()),
            Encoding::Utf8 => match rest.utf8_chunks().next() {
                Some(chunk) if chunk.valid().is_empty() => {
                    Run::Unit { bytes: chunk.invalid().len(), text: char::REPLACEMENT_CHARACTER.len_utf8() }
                }
                Some(chunk) => Run::Same(chunk.valid().len()),
                None => return None,
            },
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let order = if self.encoding == Encoding::Utf16Le { u16::from_le_bytes } else { u16::from_be_bytes };
                let unit = |i: usize| rest.get(i..i + 2).map(|c| order([c[0], c[1]]));
                let high = |u: u16| (0xD800..0xDC00).contains(&u);
                let low = |u: u16| (0xDC00..0xE000).contains(&u);
                match unit(0) {
                    None => Run::Unit { bytes: rest.len(), text: 0 },
                    Some(u) if high(u) && unit(2).is_some_and(low) => Run::Unit { bytes: 4, text: 4 },
                    Some(u) => Run::Unit {
                        bytes: 2,
                        text: char::from_u32(u32::from(u)).map_or(char::REPLACEMENT_CHARACTER.len_utf8(), char::len_utf8),
                    },
                }
            }
            Encoding::Utf32Le | Encoding::Utf32Be => {
                let order = if self.encoding == Encoding::Utf32Le { u32::from_le_bytes } else { u32::from_be_bytes };
                match rest.first_chunk::<4>() {
                    None => Run::Unit { bytes: rest.len(), text: 0 },
                    Some(c) => Run::Unit { bytes: 4, text: char::from_u32(order(*c)).map_or(0, char::len_utf8) },
                }
            }
        };
        self.at += run.bytes();
        Some(run)
    }
}

/// Where offsets of an input's decoded text fall among its bytes past the
/// mark, found by one walk over them, the offsets asked for in ascending
/// order.
pub(crate) struct Places<'a> {
    runs: Runs<'a>,
    /// The run the walk stands at the start of, while one is left.
    run: Option<Run>,
    /// Where that run starts, among the bytes and in the text.
    byte: usize,
    text: usize,
}

impl<'a> Places<'a> {
    /// The places of the text `units`, an input's bytes past its mark, make
    /// in `encoding`; with `lossy`, of that text read as a string reads
    /// UTF-8, each ill-formed part one U+FFFD.
    #[must_use]
    pub(crate) fn new(units: &'a [u8], encoding: Encoding, lossy: bool) -> Self {
        let mut runs = Runs { units, encoding, lossy, at: 0 };
        let run = runs.next();
        Places { runs, run, byte: 0, text: 0 }
    }

    /// The first byte whose text starts at `offset`: ahead of any unit that
    /// decodes to nothing there.
    pub(crate) fn earliest(&mut self, offset: usize) -> usize {
        self.find(offset, false)
    }

    /// The last byte whose text starts at `offset`: past every unit that
    /// decodes to nothing there.
    pub(crate) fn latest(&mut self, offset: usize) -> usize {
        self.find(offset, true)
    }

    fn find(&mut self, offset: usize, past_empty: bool) -> usize {
        while let Some(run) = self.run {
            let inside = offset > self.text && offset < self.text + run.text();
            if inside {
                return match run {
                    Run::Same(_) => self.byte + (offset - self.text),
                    Run::Unit { .. } => self.byte,
                };
            }
            let passed = offset > self.text || (past_empty && offset == self.text && run.text() == 0);
            if !passed {
                return self.byte;
            }
            self.byte += run.bytes();
            self.text += run.text();
            self.run = self.runs.next();
        }
        self.byte
    }
}

/// The UTF-8 text of `raw`, code units in `encoding` read from past any
/// mark, as [`decode`] reads a marked input: UTF-16 with an unpaired
/// surrogate read as U+FFFD, UTF-32 with a value that is no character
/// dropped, and a trailing part of a unit dropped.
#[must_use]
pub fn decode_units(raw: &[u8], encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => raw.to_vec(),
        Encoding::Utf16Le => utf16_text(raw, u16::from_le_bytes),
        Encoding::Utf16Be => utf16_text(raw, u16::from_be_bytes),
        Encoding::Utf32Le => utf32_text(raw, u32::from_le_bytes),
        Encoding::Utf32Be => utf32_text(raw, u32::from_be_bytes),
    }
}

/// The UTF-8 text of `raw`, UTF-16 code units in the byte order `unit`
/// reads, a surrogate with no partner read as U+FFFD and a trailing byte
/// dropped. Each byte order is its own loop, so `unit` is inlined.
fn utf16_text(raw: &[u8], unit: impl Fn([u8; 2]) -> u16) -> Vec<u8> {
    let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| unit(*c)).collect();
    String::from_utf16_lossy(&units).into_bytes()
}

/// The UTF-8 text of `raw`, UTF-32 code units in the byte order `unit`
/// reads, a value that is no character dropped and a trailing part of a unit
/// dropped. Each byte order is its own loop, so `unit` is inlined.
fn utf32_text(raw: &[u8], unit: impl Fn([u8; 4]) -> u32) -> Vec<u8> {
    raw.as_chunks::<4>().0.iter().filter_map(|c| char::from_u32(unit(*c))).collect::<String>().into_bytes()
}

/// The encoding a byte order mark declares an input to be in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8, declared by its mark or by the absence of any: the bytes are
    /// the text.
    #[default]
    Utf8,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
}

impl Encoding {
    /// The encoding `opening`, an input's first bytes, declares by a byte
    /// order mark, and how many bytes the mark takes; UTF-8 and none where it
    /// opens with no mark. UTF-32 LE is read before UTF-16 LE, whose mark it
    /// extends, as [`decode`] reads them.
    #[must_use]
    pub fn declared(opening: &[u8]) -> (Encoding, usize) {
        if opening.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) {
            (Encoding::Utf32Le, 4)
        } else if opening.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) {
            (Encoding::Utf32Be, 4)
        } else if opening.starts_with(&[0xFF, 0xFE]) {
            (Encoding::Utf16Le, 2)
        } else if opening.starts_with(&[0xFE, 0xFF]) {
            (Encoding::Utf16Be, 2)
        } else if opening.starts_with(&[0xEF, 0xBB, 0xBF]) {
            (Encoding::Utf8, 3)
        } else {
            (Encoding::Utf8, 0)
        }
    }

    /// How many bytes one code unit of this encoding takes.
    #[must_use]
    pub fn unit_width(self) -> usize {
        match self {
            Encoding::Utf8 => 1,
            Encoding::Utf16Le | Encoding::Utf16Be => 2,
            Encoding::Utf32Le | Encoding::Utf32Be => 4,
        }
    }

    /// The bytes of the code unit that is a newline in this encoding.
    #[must_use]
    pub fn newline(self) -> &'static [u8] {
        match self {
            Encoding::Utf8 => b"\n",
            Encoding::Utf16Le => &[0x0A, 0x00],
            Encoding::Utf16Be => &[0x00, 0x0A],
            Encoding::Utf32Le => &[0x0A, 0x00, 0x00, 0x00],
            Encoding::Utf32Be => &[0x00, 0x00, 0x00, 0x0A],
        }
    }
}

/// How many UTF-16 code units the UTF-8 text `text` makes: one for each
/// character, and a second for one beyond the Basic Multilingual Plane,
/// whose four-byte form is the only one opening with a byte at or above
/// 0xF0. What a surface that indexes text in UTF-16, as PowerShell's strings
/// are indexed, places a window of a longer text by.
#[must_use]
pub fn utf16_units(text: &[u8]) -> usize {
    text.iter().map(|&b| usize::from((b & 0xC0) != 0x80) + usize::from(b >= 0xF0)).sum()
}

/// Whether `bytes` could still grow into a longer byte order mark than the
/// one they hold, so the encoding they declare is not decided yet.
fn opens_a_longer_mark(bytes: &[u8]) -> bool {
    const MARKS: [&[u8]; 5] =
        [&[0xFF, 0xFE, 0x00, 0x00], &[0x00, 0x00, 0xFE, 0xFF], &[0xFF, 0xFE], &[0xFE, 0xFF], &[0xEF, 0xBB, 0xBF]];
    MARKS.iter().any(|mark| mark.len() > bytes.len() && mark.starts_with(bytes))
}

/// A decoder for an input that arrives a piece at a time, as a file followed
/// while it grows does: each piece decoded by the encoding the input's mark
/// declared, as [`decode`] decodes a whole input, with a code unit or a
/// surrogate pair cut between two pieces held until the rest arrives. An
/// input with no mark passes through as the bytes it is.
#[derive(Clone, Debug, Default)]
pub struct Incremental {
    /// The encoding, once the input's opening bytes have declared it.
    encoding: Option<Encoding>,
    /// Bytes not yet decoded: a mark still arriving, or a unit or a pair
    /// still incomplete.
    held: Vec<u8>,
}

impl Incremental {
    /// A decoder for an input read from its first byte, which learns the
    /// encoding from the mark the input opens with.
    #[must_use]
    pub fn from_start() -> Self {
        Incremental { encoding: None, held: Vec::new() }
    }

    /// A decoder for an input read from past its mark, in the encoding the
    /// mark declared.
    #[must_use]
    pub fn after(encoding: Encoding) -> Self {
        Incremental { encoding: Some(encoding), held: Vec::new() }
    }

    /// The text of `piece`, the input's next bytes, in UTF-8; what cannot be
    /// decoded until more arrives is held for the next piece.
    pub fn decode(&mut self, piece: &[u8]) -> Vec<u8> {
        self.held.extend_from_slice(piece);
        let encoding = match self.encoding {
            Some(e) => e,
            None => {
                if opens_a_longer_mark(&self.held) {
                    return Vec::new();
                }
                let (e, mark) = Encoding::declared(&self.held);
                self.held.drain(..mark);
                self.encoding = Some(e);
                e
            }
        };
        match encoding {
            Encoding::Utf8 => std::mem::take(&mut self.held),
            Encoding::Utf16Le => self.take_utf16(u16::from_le_bytes),
            Encoding::Utf16Be => self.take_utf16(u16::from_be_bytes),
            Encoding::Utf32Le | Encoding::Utf32Be => {
                let taken = self.held.len() / 4 * 4;
                let text = decode_units(&self.held[..taken], encoding);
                self.held.drain(..taken);
                text
            }
        }
    }

    /// The text of the whole UTF-16 code units held, in the byte order `unit`
    /// reads, less a high surrogate ending them, which waits for the low one.
    fn take_utf16(&mut self, unit: impl Fn([u8; 2]) -> u16) -> Vec<u8> {
        let mut units: Vec<u16> = self.held.as_chunks::<2>().0.iter().map(|c| unit(*c)).collect();
        if units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
            units.pop();
        }
        self.held.drain(..units.len() * 2);
        String::from_utf16_lossy(&units).into_bytes()
    }

    /// The text of what is still held once the input has ended, read as
    /// [`decode`] reads the end of a whole input: a surrogate still waiting
    /// for its pair is U+FFFD, a part of a unit is dropped, and an input too
    /// short to have declared its encoding is decoded whole.
    pub fn finish(&mut self) -> Vec<u8> {
        let held = std::mem::take(&mut self.held);
        match self.encoding {
            Some(encoding) => decode_units(&held, encoding),
            None => decode(held),
        }
    }
}

/// Strictly decode `raw` as BOM-less UTF-16 with the given unit reader.
/// Accepts only what real text satisfies: even length, no unpaired
/// surrogates, no control chars beyond tab / newline / carriage return, at
/// least one letter, and script coherence - at least 95% of the letters in
/// the two dominant scripts. Binary that survives the decode sprays letters
/// across unrelated scripts (an 8-byte PNG header reads as CJK + Gurmukhi +
/// Buhid), which no real document does.
fn strict_utf16(raw: &[u8], unit: fn(&[u8; 2]) -> u16) -> Option<String> {
    if raw.is_empty() || !raw.len().is_multiple_of(2) {
        return None;
    }
    let s: String =
        char::decode_utf16(raw.as_chunks::<2>().0.iter().map(unit)).collect::<Result<_, _>>().ok()?;
    if s.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r')) {
        return None;
    }
    if !s.chars().any(char::is_alphabetic) {
        return None;
    }
    // Script coherence over every script-bearing char - letters and
    // script-specific punctuation or marks - so gibberish like "Syriac
    // punct + Samaritan letter + CJK letter" cannot pass on its letters
    // alone. ASCII, whitespace, and pan-script punctuation (general and CJK
    // punctuation, fullwidth forms, currency) are Common and exempt.
    let mut counts: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut bucketed = 0usize;
    for c in s.chars() {
        let cp = c as u32;
        let common = c.is_ascii()
            || c.is_whitespace()
            || matches!(cp, 0x2000..=0x206F | 0x3000..=0x303F | 0xFF00..=0xFFEF | 0x20A0..=0x20CF);
        if common {
            continue;
        }
        bucketed += 1;
        *counts.entry(script_bucket(c)).or_default() += 1;
    }
    if bucketed > 0 {
        let mut by_count: Vec<usize> = counts.into_values().collect();
        by_count.sort_unstable_by(|a, b| b.cmp(a));
        // Evidence-scaled coherence: a short input can look two-script by
        // coincidence (an 8-byte PNG header reads as 2 CJK + 2 Gurmukhi
        // chars), so below 16 script-bearing chars the text must be
        // SINGLE-script; with more evidence a second script is allowed
        // (bilingual documents).
        let allowed = if bucketed < 16 { 1 } else { 2 };
        let top: usize = by_count.iter().take(allowed).sum();
        if top * 100 < bucketed * 95 {
            return None;
        }
    }
    Some(s)
}

/// A coarse script bucket for a letter: major scripts get one bucket across
/// their blocks (CJK spans many blocks, Latin several), and anything rarer
/// falls to a 2048-codepoint superblock - fine enough that binary-decoded
/// letter sprays land in many buckets, coarse enough that one language's
/// text lands in one.
fn script_bucket(c: char) -> u32 {
    let cp = c as u32;
    match cp {
        0x0041..=0x024F | 0x1E00..=0x1EFF => 1,               // Latin + extensions
        0x0370..=0x03FF | 0x1F00..=0x1FFF => 2,               // Greek
        0x0400..=0x052F => 3,                                 // Cyrillic
        0x0530..=0x058F => 4,                                 // Armenian
        0x0590..=0x05FF => 5,                                 // Hebrew
        0x0600..=0x077F | 0x08A0..=0x08FF => 6,               // Arabic
        0x0900..=0x0DFF => 7 + ((cp - 0x0900) >> 7),          // Indic, one per block
        0x0E00..=0x0E7F => 30,                                // Thai
        0x0E80..=0x0EFF => 31,                                // Lao
        0x0F00..=0x0FFF => 32,                                // Tibetan
        0x1000..=0x109F => 33,                                // Myanmar
        0x10A0..=0x10FF => 34,                                // Georgian
        0x1200..=0x139F => 35,                                // Ethiopic
        0x3040..=0x30FF | 0x31F0..=0x31FF => 36,              // Kana
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7FF => 37, // Hangul
        0x2E80..=0x2FDF | 0x3400..=0x9FFF | 0xF900..=0xFAFF => 38, // CJK
        _ => 1000 + (cp >> 11),                               // rare: superblock
    }
}

/// Zero-byte counts at (even, odd) offsets of `raw`.
fn null_parity(raw: &[u8]) -> (usize, usize) {
    let mut even = 0;
    let mut odd = 0;
    for (i, &b) in raw.iter().enumerate() {
        if b == 0 {
            if i.is_multiple_of(2) { even += 1 } else { odd += 1 }
        }
    }
    (even, odd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }
    fn utf16be(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_be_bytes).collect()
    }

    #[test]
    fn boms_select_the_encoding() {
        let text = "hi \u{4E2D}\u{6587}"; // "hi 中文"
        let mut le = vec![0xFF, 0xFE];
        le.extend(utf16le(text));
        assert_eq!(decode(le), text.as_bytes());
        let mut be = vec![0xFE, 0xFF];
        be.extend(utf16be(text));
        assert_eq!(decode(be), text.as_bytes());
        let mut u8bom = vec![0xEF, 0xBB, 0xBF];
        u8bom.extend(text.as_bytes());
        assert_eq!(decode(u8bom), text.as_bytes());
        let mut u32le = vec![0xFF, 0xFE, 0x00, 0x00];
        u32le.extend(text.chars().flat_map(|c| (c as u32).to_le_bytes()));
        assert_eq!(decode(u32le), text.as_bytes());
        let mut u32be = vec![0x00, 0x00, 0xFE, 0xFF];
        u32be.extend(text.chars().flat_map(|c| (c as u32).to_be_bytes()));
        assert_eq!(decode(u32be), text.as_bytes());
    }

    #[test]
    fn bomless_utf16_ascii_both_endians() {
        // ASCII text without a BOM: the null-parity signal is decisive.
        let text = "call f(42) ok\n";
        assert_eq!(decode(utf16le(text)), text.as_bytes());
        assert_eq!(decode(utf16be(text)), text.as_bytes());
    }

    #[test]
    fn bomless_utf16_cjk() {
        // CJK with ordinary whitespace: the newline's null byte still betrays
        // the endianness.
        let text = "\u{4E2D}\u{6587}\u{5206}\u{8BCD} ok\n";
        assert_eq!(decode(utf16le(text)), text.as_bytes());
        assert_eq!(decode(utf16be(text)), text.as_bytes());
        // A spaceless non-Latin line has no ASCII signal: LE by convention.
        let pure = "\u{4E2D}\u{6587}\u{5206}\u{8BCD}";
        assert_eq!(decode(utf16le(pure)), pure.as_bytes());
    }

    #[test]
    fn utf8_and_ascii_pass_untouched() {
        for text in ["plain ascii", "caf\u{E9} \u{4E2D}\u{6587}", ""] {
            assert_eq!(decode(text.as_bytes().to_vec()), text.as_bytes());
        }
    }

    /// An input decoded a piece at a time, cut at every byte, reads as the
    /// whole input decoded at once, for every mark and none, with a pair of
    /// surrogates among the text.
    #[test]
    fn an_input_decoded_in_pieces_reads_as_the_whole_decoded_at_once() {
        let text = "a \u{4E2D}\u{6587} \u{1F600} line\nnext\n";
        let mut inputs: Vec<Vec<u8>> = Vec::new();
        let mut le = vec![0xFF, 0xFE];
        le.extend(utf16le(text));
        inputs.push(le);
        let mut be = vec![0xFE, 0xFF];
        be.extend(utf16be(text));
        inputs.push(be);
        let mut u32le = vec![0xFF, 0xFE, 0x00, 0x00];
        u32le.extend(text.chars().flat_map(|c| (c as u32).to_le_bytes()));
        inputs.push(u32le);
        let mut u32be = vec![0x00, 0x00, 0xFE, 0xFF];
        u32be.extend(text.chars().flat_map(|c| (c as u32).to_be_bytes()));
        inputs.push(u32be);
        let mut u8bom = vec![0xEF, 0xBB, 0xBF];
        u8bom.extend(text.as_bytes());
        inputs.push(u8bom);
        inputs.push(text.as_bytes().to_vec());
        for input in &inputs {
            for piece in 1..=5 {
                let mut decoder = Incremental::from_start();
                let mut out = Vec::new();
                for chunk in input.chunks(piece) {
                    out.extend(decoder.decode(chunk));
                }
                assert_eq!(out, decode(input.clone()), "{:02X?} in pieces of {piece}", &input[..4]);
            }
        }
        // Read from past its mark, an input decodes in the encoding the mark
        // declared.
        let mut decoder = Incremental::after(Encoding::Utf16Le);
        let body = utf16le(text);
        let mut out = decoder.decode(&body[..3]);
        out.extend(decoder.decode(&body[3..]));
        assert_eq!(out, text.as_bytes());
    }

    #[test]
    fn utf16_units_count_what_a_utf16_string_holds() {
        for text in ["", "plain", "caf\u{E9}", "\u{4E2D}\u{6587}", "a\u{1F600}b\n", "\u{10FFFF}"] {
            assert_eq!(utf16_units(text.as_bytes()), text.encode_utf16().count(), "{text:?}");
        }
    }

    #[test]
    fn binary_is_never_misread() {
        // Odd length: cannot be UTF-16.
        assert_eq!(decode(vec![0xFF, 0x00, 0xFF]), vec![0xFF, 0x00, 0xFF]);
        // Even length but decodes to control chars: stays raw.
        let ctl = vec![0x07, 0x00, 0x08, 0x00, 0x61, 0x00, 0x07, 0x00];
        assert_eq!(decode(ctl.clone()), ctl);
        // An unpaired surrogate: strict decode fails, stays raw.
        let surr = vec![0x00, 0xD8, 0x61, 0x00];
        assert_eq!(decode(surr.clone()), surr);
        // PNG-style header bytes: control-clean when read as UTF-16 LE, but
        // the letters spray across unrelated scripts - the coherence gate
        // keeps it raw.
        let png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(decode(png.clone()), png);
        // Short binary below the evidence floor: stays raw.
        let tiny = vec![0x2D, 0x4E];
        assert_eq!(decode(tiny.clone()), tiny);
        // Longer structured binary (an LCG byte stream): stays raw.
        let mut x = 0x1234_5678_u32;
        let blob: Vec<u8> = (0..64)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect();
        assert_eq!(decode(blob.clone()), blob);
    }
}
