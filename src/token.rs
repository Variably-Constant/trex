//! The typed-token contract shared by the lexer and the engine.
//!
//! Every trex atom (`\N`, `\W`, `\Q`, `\I`, ...) names a
//! [`TokenKind`]. The lexer produces a flat `Vec<Token>` over the
//! input; the derivative engine consumes that slice. Bracket
//! pairing is recorded on the token itself ([`Token::mate`]) so
//! that matching a balanced, nestable group is a constant-time
//! jump rather than a recursive descent at match time.
//!
//! This module defines only the data contract. The lexer that fills it
//! ([`crate::lexer`]) and the engine that reads it ([`crate::engine`])
//! live in their own modules.

use std::num::NonZeroU32;

/// The three bracket pairs the lexer balances.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BracketKind {
    /// `(` and `)`.
    Paren,
    /// `[` and `]`.
    Square,
    /// `{` and `}`.
    Brace,
}

/// The typed class of a single token.
///
/// Each variant is the target of one surface atom. Promoting a
/// span to one of these classes is the lexer's job; once classed,
/// a whole number or a whole quoted string is one atom, which is
/// what lets a trex pattern stay on one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Integer or floating-point literal. Surface atom: `\N`.
    Number,
    /// Word or identifier run. Surface atom: `\W`.
    Word,
    /// Quoted string, including its delimiters, with escapes
    /// resolved by the lexer. A string closes on its own line, or a
    /// backslash carries it past the newline; a quote its line does not
    /// close is punctuation. Surface atom: `\Q`.
    Quoted,
    /// IPv4 or IPv6 address. Surface atom: `\I`.
    Ip,
    /// URL. Surface atom: `\U`.
    Url,
    /// Email address. Surface atom: `\E`.
    Email,
    /// Date or timestamp. Surface atom: `\T`.
    Timestamp,
    /// A semantic-version string, `MAJOR.MINOR.PATCH` with an optional
    /// `-prerelease` and/or `+build` suffix. Surface atom: `\V`.
    Version,
    /// A UUID / GUID: `8-4-4-4-12` hex digits. Surface atom: `\{uuid}`.
    Uuid,
    /// A MAC / EUI-48 hardware address: six `:`- or `-`-separated hex
    /// pairs. Surface atom: `\A`.
    Mac,
    /// A hex color literal, `#rgb` or `#rrggbb`. Surface atom: `\H`.
    HexColor,
    /// A CIDR block: an IPv4 address with a `/prefix`. Surface atom: `\C`.
    Cidr,
    /// A percentage, `N%` or `N.N%`. Surface atom: `\%`.
    Percent,
    /// A byte size with a unit, `10MB` / `1.5GiB` / `512KB`. Surface atom: `\Z`.
    ByteSize,
    /// A money amount, `$1,234.56` / `$5`. Surface atom: `\$`.
    Money,
    /// A hash digest: a run of exactly 32 / 40 / 64 hex characters with at
    /// least one hex letter (md5 / sha1 / sha256). Surface atom: `\D`.
    HashDigest,
    /// A hex run: 32 or more hex characters with at least one hex letter, at
    /// a length no hash digest has (a key, a dump, a digest of another
    /// size). Surface atom: `\{hex}`.
    Hex,
    /// A duration: one or more `number + time-unit` segments
    /// (`1500ms`, `2.5s`, `3h20m`). Surface atom: `\R`.
    Duration,
    /// A filesystem path (`/usr/bin/x`, `./rel`, `../up`, `~/home`,
    /// `C:\dir\file`, `\\server\share`). Surface atom: `\L`.
    Path,
    /// A JSON Web Token: three base64url segments separated by dots
    /// (`header.payload.signature`). Surface atom: `\{jwt}`.
    Jwt,
    /// A payment-card number, 13-19 digits, contiguous or grouped the way an
    /// issuer prints them (`4-4-4-4`, `4-6-5`, `4-6-4` or `4-4-4-4-3`, with
    /// one separator throughout, a space or a hyphen), that passes the Luhn
    /// check. Surface atom: `\{creditcard}`.
    CreditCard,
    /// A base64 / base64url blob: a `[A-Za-z0-9+/]` run of length a multiple of
    /// four and at least 16, with charset diversity (so a plain word is not one)
    /// and optional `=` padding. Surface atom: `\{base64}`.
    Base64,
    /// A geographic coordinate in decimal degrees, `lat,long` with both in range
    /// (lat -90..90, long -180..180), each carrying four fractional digits, and
    /// the pair on its own rather than inside a longer comma-separated run.
    /// Surface atom: `\{geo}`.
    Geo,
    /// A telephone number: international, a `+` then an ITU-T E.164 country
    /// calling code and 7-15 digits in all; or North American written
    /// nationally, `NPA-NXX-XXXX` hyphenated with an optional `1-` prefix.
    /// Surface atom: `\{phone}`.
    Phone,
    /// A physical quantity: a number, signed where nothing alphanumeric
    /// precedes the sign, then a unit symbol of [`crate::quantity`]'s table,
    /// attached or one space apart (`5kg`, `-40°C`, `3.2 GHz`, `40 %`).
    /// Surface atom: `\{quantity}`; `\{qty}` is the class of every kind a
    /// quantity predicate reads.
    Quantity,
    /// Run of insignificant whitespace. Surface atom: `\S`.
    /// Skipped between atoms in token-mode unless matched
    /// explicitly.
    Whitespace,
    /// A single punctuation token. Surface atom: `\P`.
    Punct,
    /// An opening bracket of the given kind. Its [`Token::mate`]
    /// points at the matching close.
    Open(BracketKind),
    /// A closing bracket of the given kind. Its [`Token::mate`]
    /// points back at the matching open.
    Close(BracketKind),
    /// A span matching a user-declared shape, identified by its id in the
    /// [`crate::custom::ShapeSet`] that lexed it. Surface atom: `\{name}`,
    /// the name that shape was declared under.
    Custom(u8),
    /// A token the lexer did not assign a more specific class.
    Other,
}

impl TokenKind {
    /// Whether this kind is skipped between atoms in token-mode.
    #[must_use]
    pub fn is_insignificant(self) -> bool {
        matches!(self, TokenKind::Whitespace)
    }

    /// The bracket a kind code stands for and whether it opens, or `None` for
    /// a code that is no bracket.
    ///
    /// Beside [`TokenKind::code`] so both directions of the encoding read from
    /// the one table, and derived from it rather than written out a second
    /// time. A reader holding codes and not kinds - the significant stream
    /// stores codes - needs this to match a close against the open it wants.
    #[must_use]
    pub fn bracket_of_code(code: u32) -> Option<(bool, BracketKind)> {
        for bk in [BracketKind::Paren, BracketKind::Square, BracketKind::Brace] {
            if TokenKind::Open(bk).code() == code {
                return Some((true, bk));
            }
            if TokenKind::Close(bk).code() == code {
                return Some((false, bk));
            }
        }
        None
    }

    /// A small, total, distinct integer code for this kind, used to
    /// carry the token-kind stream and the atom-kind table to a backend
    /// that cannot hold the Rust enum (the GPU kernel). The bracket kind
    /// is folded into the code so `Open(Paren)` and `Open(Square)` stay
    /// distinct.
    #[must_use]
    pub fn code(self) -> u32 {
        match self {
            TokenKind::Number => 0,
            TokenKind::Word => 1,
            TokenKind::Quoted => 2,
            TokenKind::Ip => 3,
            TokenKind::Url => 4,
            TokenKind::Email => 5,
            TokenKind::Timestamp => 6,
            TokenKind::Whitespace => 7,
            TokenKind::Punct => 8,
            TokenKind::Other => 9,
            TokenKind::Open(BracketKind::Paren) => 10,
            TokenKind::Open(BracketKind::Square) => 11,
            TokenKind::Open(BracketKind::Brace) => 12,
            TokenKind::Close(BracketKind::Paren) => 13,
            TokenKind::Close(BracketKind::Square) => 14,
            TokenKind::Close(BracketKind::Brace) => 15,
            // Codes for the richer typed kinds continue past the bracket
            // block. The GPU kernel compares these as opaque integers
            // (`kernels/scan.cu`), so a new code needs no kernel change.
            TokenKind::Version => 16,
            TokenKind::Uuid => 17,
            TokenKind::Mac => 18,
            TokenKind::HexColor => 19,
            TokenKind::Cidr => 20,
            TokenKind::Percent => 21,
            TokenKind::ByteSize => 22,
            TokenKind::Money => 23,
            TokenKind::HashDigest => 24,
            TokenKind::Duration => 25,
            TokenKind::Path => 26,
            TokenKind::Jwt => 27,
            TokenKind::CreditCard => 28,
            TokenKind::Base64 => 29,
            TokenKind::Geo => 30,
            TokenKind::Phone => 31,
            TokenKind::Quantity => 32,
            TokenKind::Hex => 33,
            // Custom codes start past the built-in block. `shape_class`
            // shifts a code left by 16, so the range stays inside a u32.
            TokenKind::Custom(id) => 34 + u32::from(id),
        }
    }

    /// The kind a code names: the inverse of [`Self::code`] on every kind.
    ///
    /// # Panics
    /// On a code no kind has, which only a stream written by something other
    /// than [`Self::code`] could carry.
    #[must_use]
    pub fn from_code(code: u32) -> TokenKind {
        match code {
            0 => TokenKind::Number,
            1 => TokenKind::Word,
            2 => TokenKind::Quoted,
            3 => TokenKind::Ip,
            4 => TokenKind::Url,
            5 => TokenKind::Email,
            6 => TokenKind::Timestamp,
            7 => TokenKind::Whitespace,
            8 => TokenKind::Punct,
            9 => TokenKind::Other,
            10 => TokenKind::Open(BracketKind::Paren),
            11 => TokenKind::Open(BracketKind::Square),
            12 => TokenKind::Open(BracketKind::Brace),
            13 => TokenKind::Close(BracketKind::Paren),
            14 => TokenKind::Close(BracketKind::Square),
            15 => TokenKind::Close(BracketKind::Brace),
            16 => TokenKind::Version,
            17 => TokenKind::Uuid,
            18 => TokenKind::Mac,
            19 => TokenKind::HexColor,
            20 => TokenKind::Cidr,
            21 => TokenKind::Percent,
            22 => TokenKind::ByteSize,
            23 => TokenKind::Money,
            24 => TokenKind::HashDigest,
            25 => TokenKind::Duration,
            26 => TokenKind::Path,
            27 => TokenKind::Jwt,
            28 => TokenKind::CreditCard,
            29 => TokenKind::Base64,
            30 => TokenKind::Geo,
            31 => TokenKind::Phone,
            32 => TokenKind::Quantity,
            33 => TokenKind::Hex,
            34..=289 => TokenKind::Custom((code - 34) as u8),
            _ => panic!("{code} is no token kind's code"),
        }
    }
}

/// One lexed token: a typed class plus the half-open byte span
/// `[start, end)` it covers in the input.
///
/// The bracket-pairing result rides on the token: for an
/// [`TokenKind::Open`] / [`TokenKind::Close`] token [`Token::mate`] is the
/// index of the partner token in the stream, and `None` for every other
/// kind.
///
/// A whole input's tokens are one flat vector that every consumer streams,
/// so the struct's width is bandwidth. The mate is stored as the partner's
/// index plus one in a [`NonZeroU32`], which puts the `None` in the zero
/// niche and costs four bytes rather than the sixteen an `Option<usize>`
/// takes.
impl TokenKind {
    /// The kind's name, as `\{name}` writes it in a pattern.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            TokenKind::Number => "number",
            TokenKind::Word => "word",
            TokenKind::Quoted => "quoted",
            TokenKind::Ip => "ip",
            TokenKind::Url => "url",
            TokenKind::Email => "email",
            TokenKind::Timestamp => "timestamp",
            TokenKind::Version => "version",
            TokenKind::Uuid => "uuid",
            TokenKind::Mac => "mac",
            TokenKind::HexColor => "hexcolor",
            TokenKind::Cidr => "cidr",
            TokenKind::Percent => "percent",
            TokenKind::ByteSize => "bytesize",
            TokenKind::Money => "money",
            TokenKind::HashDigest => "hash",
            TokenKind::Hex => "hex",
            TokenKind::Duration => "duration",
            TokenKind::Path => "path",
            TokenKind::Jwt => "jwt",
            TokenKind::CreditCard => "creditcard",
            TokenKind::Base64 => "base64",
            TokenKind::Geo => "geo",
            TokenKind::Phone => "phone",
            TokenKind::Quantity => "quantity",
            TokenKind::Whitespace => "whitespace",
            TokenKind::Punct => "punct",
            TokenKind::Open(_) => "open",
            TokenKind::Close(_) => "close",
            TokenKind::Custom(_) => "custom",
            TokenKind::Other => "other",
        }
    }

    /// The kind written under `name`, the inverse of [`Self::name`] for the
    /// kinds a name reaches.
    ///
    /// A bracket is named by its side rather than its shape, since that is
    /// what [`Self::name`] reports; a declared kind has no name here, because
    /// the name belongs to the declaration rather than to the enum.
    #[must_use]
    pub fn named(name: &str) -> Option<TokenKind> {
        [
            TokenKind::Number,
            TokenKind::Word,
            TokenKind::Quoted,
            TokenKind::Ip,
            TokenKind::Url,
            TokenKind::Email,
            TokenKind::Timestamp,
            TokenKind::Version,
            TokenKind::Uuid,
            TokenKind::Mac,
            TokenKind::HexColor,
            TokenKind::Cidr,
            TokenKind::Percent,
            TokenKind::ByteSize,
            TokenKind::Money,
            TokenKind::HashDigest,
            TokenKind::Hex,
            TokenKind::Duration,
            TokenKind::Path,
            TokenKind::Jwt,
            TokenKind::CreditCard,
            TokenKind::Base64,
            TokenKind::Geo,
            TokenKind::Phone,
            TokenKind::Quantity,
            TokenKind::Whitespace,
            TokenKind::Punct,
            TokenKind::Open(BracketKind::Paren),
            TokenKind::Close(BracketKind::Paren),
            TokenKind::Other,
        ]
        .into_iter()
        .find(|k| k.name() == name)
    }

    /// The single letter that names the kind as `\X` in a pattern, for the
    /// kinds that have one.
    #[must_use]
    pub fn escape(self) -> Option<char> {
        Some(match self {
            TokenKind::Number => 'N',
            TokenKind::Word => 'W',
            TokenKind::Quoted => 'Q',
            TokenKind::Ip => 'I',
            TokenKind::Url => 'U',
            TokenKind::Email => 'E',
            TokenKind::Timestamp => 'T',
            TokenKind::Punct => 'P',
            TokenKind::Whitespace => 'S',
            TokenKind::Version => 'V',
            TokenKind::HexColor => 'H',
            TokenKind::Cidr => 'C',
            TokenKind::ByteSize => 'Z',
            TokenKind::Percent => '%',
            TokenKind::Money => '$',
            TokenKind::HashDigest => 'D',
            TokenKind::Duration => 'R',
            TokenKind::Path => 'L',
            TokenKind::Uuid
            | TokenKind::Mac
            | TokenKind::Hex
            | TokenKind::Jwt
            | TokenKind::CreditCard
            | TokenKind::Base64
            | TokenKind::Geo
            | TokenKind::Phone
            | TokenKind::Quantity
            | TokenKind::Open(_)
            | TokenKind::Close(_)
            | TokenKind::Custom(_)
            | TokenKind::Other => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    /// The token's typed class.
    pub kind: TokenKind,
    /// Inclusive start byte offset into the input.
    pub start: u32,
    /// Exclusive end byte offset into the input.
    pub end: u32,
    /// The mate's index plus one, or `None`. Read it through
    /// [`Token::mate`] and write it through [`Token::set_mate`], which hold
    /// the encoding in one place; the field is named for what it stores so
    /// that reading it as an index does not compile.
    pub mate_plus_one: Option<NonZeroU32>,
}

impl Token {
    /// Construct a token with no bracket mate.
    ///
    /// # Panics
    /// When an offset does not fit the stored width, which needs an input
    /// over four gigabytes. Truncating instead would point a token at the
    /// wrong bytes.
    #[must_use]
    #[inline]
    pub fn new(kind: TokenKind, start: usize, end: usize) -> Self {
        Self {
            kind,
            start: u32::try_from(start).expect("a byte offset within the stored width"),
            end: u32::try_from(end).expect("a byte offset within the stored width"),
            mate_plus_one: None,
        }
    }

    /// The token's byte span, for indexing the input it was lexed from.
    #[must_use]
    #[inline]
    pub fn span(&self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }

    /// The start offset as a `usize`, for arithmetic and indexing against
    /// everything else, which counts bytes in the machine's width. The field
    /// is the storage and this is the view of it; the two never disagree,
    /// and a site that needs the wider one fails to compile rather than
    /// silently taking the narrower.
    #[must_use]
    #[inline]
    pub fn start(&self) -> usize {
        self.start as usize
    }

    /// The end offset as a `usize`. See [`Token::start`].
    #[must_use]
    #[inline]
    pub fn end(&self) -> usize {
        self.end as usize
    }

    /// Move the span by `by` bytes, for a token lexed from a chunk and
    /// rebased onto the whole input.
    ///
    /// # Panics
    /// When the shifted offset does not fit the stored width.
    #[inline]
    pub fn shift(&mut self, by: usize) {
        let shifted = |v: u32| -> u32 {
            u32::try_from(v as usize + by).expect("a byte offset within the stored width")
        };
        self.start = shifted(self.start);
        self.end = shifted(self.end);
    }

    /// The index of this token's matching bracket, or `None` when it has
    /// none.
    #[must_use]
    #[inline]
    pub fn mate(&self) -> Option<usize> {
        self.mate_plus_one.map(|m| m.get() as usize - 1)
    }

    /// Record this token's matching bracket, or clear it.
    ///
    /// # Panics
    /// When the index does not fit the stored width. A stream that long
    /// would need more than four billion tokens, which no input this lexer
    /// can hold produces, and silently storing a wrapped index would pair
    /// the wrong brackets.
    #[inline]
    pub fn set_mate(&mut self, index: Option<usize>) {
        self.mate_plus_one = index.map(|i| {
            let plus_one = u32::try_from(i + 1).expect("a token index within the stored width");
            NonZeroU32::new(plus_one).expect("one more than an index is never zero")
        });
    }

    /// The byte length of the token's span.
    #[must_use]
    pub fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    /// Whether the token's span is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Whether this token participates in token-mode matching
    /// (everything except insignificant whitespace).
    #[must_use]
    pub fn is_significant(&self) -> bool {
        !self.kind.is_insignificant()
    }
}

/// The class key of a token: a generic token-to-string mapping over the typed-token kinds, used by
/// the compression-based structure layer and the field axes. Numbers / strings / literals collapse
/// to a tag, short words stay literal, long words collapse to `#id`, brackets and punctuation stay
/// themselves. A substrate primitive over the typed tokens, independent of any domain.
#[must_use]
pub fn code_class(t: &Token, code: &[u8]) -> String {
    let text = || String::from_utf8_lossy(&code[t.span()]).into_owned();
    match t.kind {
        TokenKind::Punct => text(),
        TokenKind::Number => "#num".into(),
        TokenKind::Quoted => "#str".into(),
        TokenKind::Ip
        | TokenKind::Url
        | TokenKind::Email
        | TokenKind::Timestamp
        | TokenKind::Version
        | TokenKind::Uuid
        | TokenKind::Mac
        | TokenKind::HexColor
        | TokenKind::Cidr
        | TokenKind::Percent
        | TokenKind::ByteSize
        | TokenKind::Money
        | TokenKind::HashDigest
        | TokenKind::Hex
        | TokenKind::Duration
        | TokenKind::Path
        | TokenKind::Jwt
        | TokenKind::CreditCard
        | TokenKind::Base64
        | TokenKind::Geo
        | TokenKind::Phone
        | TokenKind::Quantity => "#lit".into(),
        TokenKind::Word => {
            let w = text();
            if w.len() <= 5 { w } else { "#id".into() }
        }
        TokenKind::Open(BracketKind::Paren) => "(".into(),
        TokenKind::Open(BracketKind::Square) => "[".into(),
        TokenKind::Open(BracketKind::Brace) => "{".into(),
        TokenKind::Close(BracketKind::Paren) => ")".into(),
        TokenKind::Close(BracketKind::Square) => "]".into(),
        TokenKind::Close(BracketKind::Brace) => "}".into(),
        TokenKind::Custom(id) => format!("#shape{id}"),
        TokenKind::Other | TokenKind::Whitespace => "#other".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_length_matches_offsets() {
        let t = Token::new(TokenKind::Number, 4, 7);
        assert_eq!(t.len(), 3);
        assert!(!t.is_empty());
        assert!(t.is_significant());
    }

    #[test]
    fn whitespace_is_insignificant() {
        let t = Token::new(TokenKind::Whitespace, 0, 1);
        assert!(!t.is_significant());
        assert!(t.kind.is_insignificant());
    }

    #[test]
    fn bracket_mate_defaults_none_then_sets() {
        let mut t = Token::new(TokenKind::Open(BracketKind::Paren), 0, 1);
        assert_eq!(t.mate(), None);
        t.set_mate(Some(9));
        assert_eq!(t.mate(), Some(9));
        // Index zero is a mate like any other; the stored value is what
        // carries the plus one.
        t.set_mate(Some(0));
        assert_eq!(t.mate(), Some(0));
        assert_eq!(t.mate_plus_one.map(NonZeroU32::get), Some(1));
        t.set_mate(None);
        assert_eq!(t.mate(), None);
    }

    #[test]
    fn a_token_is_no_wider_than_the_layout_it_was_shrunk_to() {
        // Two offsets and a mate at four bytes each, a kind at two: fourteen
        // of content at an alignment of four. The option costs the zero
        // niche rather than a word, which is what the plus-one encoding gains.
        assert_eq!(size_of::<Option<NonZeroU32>>(), 4);
        assert_eq!(size_of::<Token>(), 16);
    }

    /// Every bracket kind survives the trip out to a code and back, and no
    /// other kind answers as a bracket - which is what lets a reader holding
    /// codes match a close against its open.
    #[test]
    fn a_bracket_kind_code_reads_back_as_the_bracket_it_came_from() {
        for bk in [BracketKind::Paren, BracketKind::Square, BracketKind::Brace] {
            assert_eq!(
                TokenKind::bracket_of_code(TokenKind::Open(bk).code()),
                Some((true, bk)),
                "an open {bk:?} reads back"
            );
            assert_eq!(
                TokenKind::bracket_of_code(TokenKind::Close(bk).code()),
                Some((false, bk)),
                "a close {bk:?} reads back"
            );
        }
        for kind in [
            TokenKind::Number,
            TokenKind::Word,
            TokenKind::Quoted,
            TokenKind::Whitespace,
            TokenKind::Punct,
            TokenKind::Other,
            TokenKind::Timestamp,
        ] {
            assert_eq!(
                TokenKind::bracket_of_code(kind.code()),
                None,
                "{kind:?} is no bracket"
            );
        }
    }
}
