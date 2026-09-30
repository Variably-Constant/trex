//! The decoded content of a token that carries an encoding: a base64 blob's
//! bytes and a JSON Web Token's header and payload.
//!
//! A predicate on `\{base64}` or `\{jwt}` reads what the token encodes rather
//! than how it is written. Decoding is the whole cost, so it happens once per
//! token and only where a clause asks for it: a predicate that names only
//! header claims never decodes the payload, and a clause whose cheaper
//! neighbour has already failed never decodes at all
//! ([`crate::typed::TypedPred`] orders its clauses by what each costs).
//!
//! Nothing here requires the decoded bytes to be text. The byte readings
//! (entropy, texture, period) are over the bytes themselves, a glob reads
//! them lossily, and a sub-pattern lexes them as the scanner lexes any
//! input, so an encoding is never a reason for a clause to be unanswerable.

use crate::typed::Decimal;

/// The base64 value of one character in the standard alphabet and in the URL
/// alphabet, which differ only in the last two: `+/` against `-_`. `64` marks
/// a byte no alphabet holds.
const fn b64_value(c: u8) -> u8 {
    match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' | b'-' => 62,
        b'/' | b'_' => 63,
        _ => 64,
    }
}

/// The value of one hexadecimal digit.
const fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// The bytes a base64 or base64url run encodes, padded or not.
///
/// Both alphabets are read at once, since a run is in one of them and the two
/// disagree only on the two characters neither uses for the other's purpose.
/// A run whose length leaves one character over encodes no whole byte and is
/// not base64; padding beyond the run's end is ignored, and a character
/// outside both alphabets refuses the whole run.
#[must_use]
pub fn base64(input: &[u8]) -> Option<Vec<u8>> {
    let body: &[u8] = match input.iter().position(|&c| c == b'=') {
        Some(at) => &input[..at],
        None => input,
    };
    if body.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &c in body {
        let v = b64_value(c);
        if v == 64 {
            return None;
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

/// One decoded half of a JSON Web Token: the payload's bytes when `payload`,
/// else the header's. A token is three `.`-separated segments; the signature
/// is not decoded, since nothing here verifies one.
///
/// A half at a time, so a predicate naming only header parameters never
/// decodes the payload.
#[must_use]
pub fn jwt_half(text: &[u8], payload: bool) -> Option<Vec<u8>> {
    let mut parts = text.split(|&c| c == b'.');
    let header = parts.next()?;
    let claims = parts.next()?;
    parts.next()?;
    base64(if payload { claims } else { header })
}

/// The Shannon entropy of `bytes`, in bits per byte, to two decimal places:
/// `0` for one repeated byte and `8` where every byte value is equally
/// likely. The hundredths are the resolution the spectral axis already
/// reports its own entropy at.
///
/// Empty input has no distribution and reads as zero.
///
/// # Panics
///
/// Never: the entropy of a byte histogram is finite and non-negative, so its
/// two-decimal rendering is a decimal, and the expect below states that.
#[must_use]
pub fn bits_per_byte(bytes: &[u8]) -> Decimal {
    if bytes.is_empty() {
        return Decimal::from_u128(0);
    }
    let mut counts = [0u32; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    let total = bytes.len() as f64;
    let mut h = 0.0f64;
    for &c in &counts {
        if c > 0 {
            let p = f64::from(c) / total;
            h -= p * p.log2();
        }
    }
    Decimal::parse(&format!("{h:.2}")).expect("an entropy renders as a decimal")
}

/// The texture class of `bytes`, read by the spectral axis over the decoded
/// content as it reads it over an input.
#[must_use]
pub fn texture(bytes: &[u8]) -> crate::spectral::Texture {
    let field = crate::spectral::analyze(bytes);
    crate::spectral::texture_of(&field.signature(0, bytes.len()))
}

/// The dominant byte-period of `bytes`, or zero where none is strong enough.
/// The lag is bounded by half the content, since a period longer than that
/// repeats less than twice and is not one.
#[must_use]
pub fn period(bytes: &[u8]) -> u16 {
    crate::spectral::dominant_period(bytes, bytes.len() / 2).0
}

/// A JSON value as a member of an object carries it, in the text it was
/// written in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Json {
    /// A string, with its escapes resolved.
    Str(String),
    /// A number, as written.
    Num(String),
    Bool(bool),
    Null,
    /// An array or an object, as the text it spans: a claim holding one
    /// compares as that text.
    Nested(String),
}

impl Json {
    /// The value as text: a string's characters, a number as written, a
    /// literal's spelling, a nested value's own text.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Json::Str(s) | Json::Num(s) | Json::Nested(s) => s.clone(),
            Json::Bool(true) => "true".to_string(),
            Json::Bool(false) => "false".to_string(),
            Json::Null => "null".to_string(),
        }
    }
}

/// Skip whitespace from `i`.
fn spaces(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// Read a JSON string starting at the opening quote, returning its resolved
/// characters and the index past the closing quote.
fn string_at(b: &[u8], i: usize) -> Option<(String, usize)> {
    if b.get(i) != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'"' => return Some((out, j + 1)),
            b'\\' => {
                let e = *b.get(j + 1)?;
                j += 2;
                match e {
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'u' => {
                        let mut code = 0u32;
                        for k in 0..4 {
                            code = code * 16 + u32::from(hex_value(*b.get(j + k)?)?);
                        }
                        j += 4;
                        // A lone surrogate is no character and stands as the
                        // replacement, which is what a reader of the text
                        // sees.
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    }
                    other => out.push(char::from(other)),
                }
            }
            _ => {
                let (c, len) = crate::lexer::utf8_char_at(b, j);
                out.push(c);
                j += len;
            }
        }
    }
    None
}

/// The index past the value starting at `i`, whatever kind it is.
fn value_end(b: &[u8], i: usize) -> Option<usize> {
    match *b.get(i)? {
        b'"' => Some(string_at(b, i)?.1),
        b'{' | b'[' => {
            let (open, close) = if b[i] == b'{' { (b'{', b'}') } else { (b'[', b']') };
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => j = string_at(b, j)?.1,
                    c if c == open => {
                        depth += 1;
                        j += 1;
                    }
                    c if c == close => {
                        depth -= 1;
                        j += 1;
                        if depth == 0 {
                            return Some(j);
                        }
                    }
                    _ => j += 1,
                }
            }
            None
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                j += 1;
            }
            (j > i).then_some(j)
        }
    }
}

/// The value the top-level object in `json` holds under `key`, or `None`
/// where the text is not an object or holds no such member.
///
/// A hand-written reader rather than a JSON crate: the question is one member
/// of one object, the answer is wanted per token, and a claim's value is
/// compared as text or as a number, never as a tree.
#[must_use]
pub fn member(json: &[u8], key: &str) -> Option<Json> {
    let b = json;
    let mut i = spaces(b, 0);
    if b.get(i) != Some(&b'{') {
        return None;
    }
    i = spaces(b, i + 1);
    while i < b.len() && b[i] != b'}' {
        let (name, next) = string_at(b, i)?;
        i = spaces(b, next);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i = spaces(b, i + 1);
        let end = value_end(b, i)?;
        if name == key {
            let raw = &b[i..end];
            return Some(match raw[0] {
                b'"' => Json::Str(string_at(b, i)?.0),
                b'{' | b'[' => Json::Nested(String::from_utf8_lossy(raw).into_owned()),
                b't' => Json::Bool(true),
                b'f' => Json::Bool(false),
                b'n' => Json::Null,
                _ => Json::Num(String::from_utf8_lossy(raw).into_owned()),
            });
        }
        i = spaces(b, end);
        if b.get(i) == Some(&b',') {
            i = spaces(b, i + 1);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_alphabets_decode_padded_and_bare() {
        assert_eq!(base64(b"SGVsbG8gV29ybGQ=").as_deref(), Some(&b"Hello World"[..]));
        assert_eq!(base64(b"SGVsbG8gV29ybGQ").as_deref(), Some(&b"Hello World"[..]));
        assert_eq!(base64(b"eyJhbGciOiJub25lIn0").as_deref(), Some(&br#"{"alg":"none"}"#[..]));
        // The URL alphabet's last two characters, and the standard one's.
        assert_eq!(base64(b"-_8=").as_deref(), Some(&[0xfb, 0xff][..]));
        assert_eq!(base64(b"+/8=").as_deref(), Some(&[0xfb, 0xff][..]));
        assert_eq!(base64(b"A"), None, "one character encodes no whole byte");
        assert_eq!(base64(b"SGVs bG8="), None, "a byte outside both alphabets");
        assert_eq!(base64(b"").as_deref(), Some(&[][..]));
    }

    #[test]
    fn a_token_splits_into_its_decoded_halves() {
        let jwt = b"eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.";
        let header = jwt_half(jwt, false).expect("three segments");
        let payload = jwt_half(jwt, true).expect("three segments");
        assert_eq!(String::from_utf8_lossy(&header), r#"{"alg":"none"}"#);
        assert_eq!(String::from_utf8_lossy(&payload), r#"{"sub":"123","exp":1789430400}"#);
        assert!(jwt_half(b"eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMifQ", true).is_none(), "two segments");
    }

    #[test]
    fn a_members_value_reads_in_the_text_it_was_written_in() {
        let json = br#"{"alg":"none","n":42,"f":1.5,"ok":true,"no":false,"z":null,"a":[1,2],"o":{"k":"v"},"esc":"a\"b\nA"}"#;
        assert_eq!(member(json, "alg"), Some(Json::Str("none".into())));
        assert_eq!(member(json, "n"), Some(Json::Num("42".into())));
        assert_eq!(member(json, "f"), Some(Json::Num("1.5".into())));
        assert_eq!(member(json, "ok"), Some(Json::Bool(true)));
        assert_eq!(member(json, "no"), Some(Json::Bool(false)));
        assert_eq!(member(json, "z"), Some(Json::Null));
        assert_eq!(member(json, "a"), Some(Json::Nested("[1,2]".into())));
        assert_eq!(member(json, "o"), Some(Json::Nested(r#"{"k":"v"}"#.into())));
        assert_eq!(member(json, "esc"), Some(Json::Str("a\"b\nA".into())));
        assert_eq!(member(json, "missing"), None);
        assert_eq!(member(b"not an object", "x"), None);
        // A key inside a nested value is not a member of the object.
        assert_eq!(member(json, "k"), None);
        // A `\u` escape, and a lone surrogate standing as the replacement.
        assert_eq!(member("{\"k\":\"A\u{e9}\"}".as_bytes(), "k"), Some(Json::Str("A\u{e9}".into())));
        assert_eq!(member(br#"{"k":"\ud800"}"#, "k"), Some(Json::Str("\u{fffd}".into())));
    }

    #[test]
    fn the_byte_readings_read_the_decoded_bytes() {
        assert_eq!(bits_per_byte(b"").to_text(), "0");
        assert_eq!(bits_per_byte(b"aaaaaaaa").to_text(), "0");
        // Two values in equal measure carry one bit a byte, four carry two.
        assert_eq!(bits_per_byte(b"abababab").to_text(), "1");
        assert_eq!(bits_per_byte(b"abcdabcd").to_text(), "2");
        // A fixed-width record repeats its delimiter at a steady lag.
        let table: Vec<u8> = "abc,12,x\n".repeat(16).into_bytes();
        assert_eq!(period(&table), 9);
    }
}
