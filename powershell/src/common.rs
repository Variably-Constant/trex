//! What the cmdlets share: the error records a failure is reported as, the
//! offsets a match is reported in, a parsed value as the .NET type that holds
//! it, and a pattern given as text or as a compiled object.

use pwrs::prelude::*;

/// A failure the caller can fix by changing an argument: a pattern that does
/// not parse, a declaration refused, a name nothing declares.
pub(crate) fn arg_err(id: &str, message: impl Into<String>) -> PsError {
    PsError::new(ErrorCategory::InvalidArgument, id, message.into())
}

/// A pattern that did not parse, naming the byte the parser stopped at.
pub(crate) fn parse_err(source: &str, e: &trex::ParseError) -> PsError {
    arg_err("TrexPatternError", format!("pattern error at byte {} of {source:?}: {}", e.pos, e.msg))
}

/// A declaration the atom set refused, with trex's own account of why.
pub(crate) fn shape_err(e: &trex::ShapeError) -> PsError {
    arg_err("TrexDeclarationError", e.msg.clone())
}

/// A file the scan could not read, reported against its path.
pub(crate) fn read_err(path: &str, e: impl std::fmt::Display) -> PsError {
    PsError::new(ErrorCategory::ReadError, "TrexRead", format!("{path}: {e}"))
}

/// What a cmdlet says of a file it was given that holds a NUL byte and so
/// goes unread.
pub(crate) fn binary_notice(path: &str) -> String {
    format!("{path} holds a NUL byte and is binary; -Binary reads it")
}

/// How many UTF-16 code units the UTF-8 bytes encode: one for every byte that
/// opens a character, and a second for a four-byte one, which UTF-16 writes
/// as a surrogate pair. A continuation byte opens nothing.
fn utf16_units(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0xBF => 0,
            0xF0..=0xFF => 2,
            _ => 1,
        })
        .sum()
}

/// Byte offsets of a UTF-8 text read as UTF-16 code-unit offsets, the unit
/// .NET indexes a string by, so `$text.Substring($m.Start, $m.Length)` is the
/// match. The cursor moves from the offset last asked to the next, forward or
/// back, counting only the bytes between, so a scan's matches and the
/// registers inside each cost one pass over the text; an ASCII text is its
/// own offsets.
pub(crate) struct Units<'a> {
    bytes: &'a [u8],
    ascii: bool,
    byte: usize,
    unit: usize,
}

impl<'a> Units<'a> {
    pub(crate) fn of(text: &'a [u8]) -> Self {
        Units { bytes: text, ascii: text.is_ascii(), byte: 0, unit: 0 }
    }

    /// The UTF-16 offset of byte offset `b`, which begins a character or is
    /// the text's end.
    pub(crate) fn at(&mut self, b: usize) -> usize {
        if self.ascii {
            return b;
        }
        if b >= self.byte {
            self.unit += utf16_units(&self.bytes[self.byte..b]);
        } else {
            self.unit -= utf16_units(&self.bytes[b..self.byte]);
        }
        self.byte = b;
        self.unit
    }
}

/// A decimal written out in digits: its sign, every digit as one integer,
/// and how many of the digits follow the point.
struct Digits {
    negative: bool,
    mantissa: u128,
    scale: u32,
}

/// The largest scale a `decimal` carries.
const DECIMAL_MAX_SCALE: usize = 28;

/// `text` read as an optionally signed decimal of up to 28 places whose
/// digits fit 128 bits, or none when it is another shape.
fn digits_of(text: &str) -> Option<Digits> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    if fraction.len() > DECIMAL_MAX_SCALE {
        return None;
    }
    let mut mantissa: u128 = 0;
    for b in whole.bytes().chain(fraction.bytes()) {
        if !b.is_ascii_digit() {
            return None;
        }
        mantissa = mantissa.checked_mul(10)?.checked_add(u128::from(b - b'0'))?;
    }
    Some(Digits { negative, mantissa, scale: fraction.len() as u32 })
}

/// The whole number `d` holds, where it has no places and fits a `long`.
fn whole_i64(d: &Digits) -> Option<i64> {
    if d.scale != 0 {
        return None;
    }
    if d.negative {
        if d.mantissa <= 1u128 << 63 { Some((d.mantissa as i128).wrapping_neg() as i64) } else { None }
    } else if d.mantissa <= i64::MAX as u128 {
        Some(d.mantissa as i64)
    } else {
        None
    }
}

/// `d` as a `decimal`, where its digits fit the 96 bits one carries.
fn decimal_of(d: &Digits) -> Option<PsDecimal> {
    if d.mantissa >> 96 != 0 {
        return None;
    }
    let lo = d.mantissa as u32 as i32;
    let mid = (d.mantissa >> 32) as u32 as i32;
    let hi = (d.mantissa >> 64) as u32 as i32;
    let sign = if d.negative && d.mantissa != 0 { 1u32 << 31 } else { 0 };
    Some(PsDecimal::from_bits(lo, mid, hi, (sign | (d.scale << 16)) as i32))
}

/// `text`, a number written out in digits, as the .NET value that holds it
/// exactly: a `long` when it is a whole number that fits one, a `decimal`
/// when its digits fit one, and a `double` past both, the one reading that
/// rounds.
pub(crate) fn number(text: &str) -> PsResult<PsObject> {
    if let Some(d) = digits_of(text) {
        if let Some(n) = whole_i64(&d) {
            return n.into_ps();
        }
        if let Some(dec) = decimal_of(&d) {
            return dec.into_ps();
        }
    }
    match text.parse::<f64>() {
        Ok(f) => f.into_ps(),
        Err(e) => Err(arg_err("TrexValue", format!("{text:?} is not a number: {e}"))),
    }
}

/// The ticks between 0001-01-01 and the Unix epoch, which .NET counts a
/// `DateTime` from.
const EPOCH_TICKS: i128 = 621_355_968_000_000_000;

/// The last tick a `DateTimeOffset` holds, 9999-12-31 23:59:59.9999999.
const MAX_TICKS: i128 = 3_155_378_975_999_999_999;

/// A decimal written with its repeating digits bracketed, `2.(3)` or
/// `0.1(6)`, as the `decimal` nearest it: as many places as a `decimal`
/// holds at its magnitude, the last one rounded, and a `double` where the
/// whole part alone is past what a `decimal` holds.
pub(crate) fn repeating(text: &str) -> PsResult<PsObject> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let Some((head, cycle)) = unsigned.strip_suffix(')').and_then(|t| t.split_once('(')) else {
        return number(text);
    };
    let (whole, fixed) = head.split_once('.').unwrap_or((head, ""));
    if cycle.is_empty() || !whole.bytes().chain(fixed.bytes()).chain(cycle.bytes()).all(|b| b.is_ascii_digit()) {
        return Err(arg_err("TrexValue", format!("{text:?} is not a repeating decimal")));
    }
    // One digit past the most places a decimal carries, so the last place
    // kept is rounded by the digit after it. A repeating tail is never
    // exactly half, so that one digit decides.
    let fraction: String = fixed.chars().chain(cycle.chars().cycle()).take(DECIMAL_MAX_SCALE + 1).collect();
    for scale in (0..=DECIMAL_MAX_SCALE).rev() {
        let Some(mut d) = digits_of(&format!("{whole}.{}", &fraction[..scale])) else {
            continue;
        };
        let Some(mantissa) = d.mantissa.checked_add(u128::from(fraction.as_bytes()[scale] >= b'5')) else {
            continue;
        };
        d.mantissa = mantissa;
        d.negative = negative;
        if let Some(dec) = decimal_of(&d) {
            return dec.into_ps();
        }
    }
    number(&format!("{}{whole}.{fraction}", if negative { "-" } else { "" }))
}

/// A duration of `ns` nanoseconds, written in digits with or without a
/// fraction, as a `TimeSpan`, which counts hundreds of them and drops what
/// is left; a duration past a `TimeSpan`'s range arrives as its nanoseconds.
pub(crate) fn duration(ns: &str) -> PsResult<PsObject> {
    let whole = match ns.split_once('.') {
        Some((w, _)) => w,
        None => ns,
    };
    let Some(d) = digits_of(whole) else {
        return number(ns);
    };
    let ticks = (d.mantissa / 100) as i128;
    let ticks = if d.negative { -ticks } else { ticks };
    if (i128::from(i64::MIN)..=i128::from(i64::MAX)).contains(&ticks) {
        PsTimeSpan::from_ticks(ticks as i64).into_ps()
    } else {
        number(ns)
    }
}

/// The value a register of `kind` bound, parsed as trex's own predicates read
/// it, as the .NET type [`typed_value`] gives it; `$null` where the text does
/// not parse as the kind or the kind carries no value.
pub(crate) fn value_of(kind: trex::token::TokenKind, text: &str) -> PsResult<PsObject> {
    match trex::typed::value_of(kind, text) {
        Some(v) => typed_value(kind, &v),
        None => Ok(PsObject::default()),
    }
}

/// The name a token's kind is written under, a declared shape or kind by the
/// name its declaration in `shapes` gave it.
pub(crate) fn kind_name(kind: trex::token::TokenKind, shapes: &trex::ShapeSet) -> String {
    match kind {
        trex::token::TokenKind::Custom(id) => match shapes.name_of(id) {
            Some(name) => name.to_string(),
            None => kind.name().to_string(),
        },
        other => other.name().to_string(),
    }
}

/// A value trex parsed from a token of `kind`, as the .NET type that holds
/// it: a whole number as `long`, any other number, a size, a percentage, an
/// amount of money or a quantity in its family's base unit as `decimal`, a
/// duration as `TimeSpan`, an instant as a UTC `DateTimeOffset`, an address,
/// a block or a text as the canonical text trex compares, and a version as
/// the dotted text semver orders it by. `$null` where a time names no date.
pub(crate) fn typed_value(kind: trex::token::TokenKind, v: &trex::typed::TypedValue) -> PsResult<PsObject> {
    use trex::typed::TypedValue as V;
    match v {
        V::Num(d) if kind == trex::token::TokenKind::Duration => duration(&d.to_text()),
        V::Num(d) => number(&d.to_text()),
        V::Quantity(_, base) => number(&base.to_text()),
        V::Instant(civil) => match civil.epoch(trex::Clock::current()) {
            Some((secs, nanos)) => {
                let ticks = i128::from(secs) * 10_000_000 + i128::from(nanos / 100) + EPOCH_TICKS;
                if (0..=MAX_TICKS).contains(&ticks) {
                    PsDateTimeOffset::new(ticks as i64, 0).into_ps()
                } else {
                    Ok(PsObject::default())
                }
            }
            None => Ok(PsObject::default()),
        },
        V::Ip(ip) => ip.to_string().into_ps(),
        V::Cidr(ip, bits) => format!("{ip}/{bits}").into_ps(),
        V::Version(version) => version.to_text().into_ps(),
        V::Text(s) => s.clone().into_ps(),
        V::Family(v6) => (if *v6 { "v6" } else { "v4" }).to_string().into_ps(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Offsets asked backward read as offsets asked forward, over characters
    /// of one, two, three and four bytes, the last a surrogate pair in
    /// UTF-16.
    #[test]
    fn utf16_offsets_read_alike_in_either_direction() {
        let text = "a\u{e9}\u{20ac}\u{1f600}b";
        let at = [(0, 0), (1, 1), (3, 2), (6, 3), (10, 5), (11, 6)];
        let mut units = Units::of(text.as_bytes());
        for &(byte, unit) in at.iter().rev() {
            assert_eq!(units.at(byte), unit, "backward to byte {byte}");
        }
        for &(byte, unit) in &at {
            assert_eq!(units.at(byte), unit, "forward to byte {byte}");
        }
        assert_eq!(text.encode_utf16().count(), 6);
    }
}
