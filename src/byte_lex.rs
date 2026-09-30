//! The lexer's recognizers as byte automata.
//!
//! A byte-grain engine has to agree with the lexer about where a token ends,
//! because a trex atom is a whole typed token: an automaton that read `1.5` as
//! two numbers and a point would match patterns the engine does not. So each
//! recognizer here is built to the shape the lexer's own scanner has, and held
//! to it by a test that lexes a corpus and asks the automaton about every token
//! the lexer found.
//!
//! Twelve recognizers and the gate between them. The rest follow the same shape,
//! and the blob gate does not follow at all - it reads a spectral entropy
//! history over the preceding bytes, which is not a property any automaton over
//! the byte stream can decide.
//!
//! Each shape is a `_piece` the standalone recognizer and the gate are both
//! built from, so a recognizer and the branch that stands for it in the gate
//! cannot come to disagree.

use crate::byte_nfa::{Builder, ByteClass, ByteNfa, Piece};

/// Which recognizer accepted, as the automata report it.
///
/// A lower value wins outright, whatever the two recognizers reach, which is
/// what [`crate::byte_nfa::ByteNfa::recognize`] does with it. So the value is
/// the recognizer's place in `lexer::try_typed_token_chain`: that function calls
/// its own order precedence, and the plain number and word sit last because the
/// lexer reads them only where the chain returned nothing.
///
/// The numbering is load-bearing rather than a tie-break, so a test can tell it
/// from its reverse. Swapping the number and the byte size reads `10MB` as the
/// number `10`, because a lower id no longer has to reach further to win.
/// The whole chain is spelled out, including the recognizers not built here, so
/// that building one never renumbers another. A gap is a recognizer this module
/// does not have yet, not a gap left for room; the numbers are read off the
/// chain rather than chosen. Where a kind has two branches - a phone written
/// internationally or nationally, an address written as v6 or v4 - the kind
/// takes the first of them.
pub mod id {
    /// A URL: a scheme, then `://`, then the rest of the run.
    pub const URL: u32 = 0;
    /// A JSON Web Token, anchored by its `eyJ` header.
    pub const JWT: u32 = 1;
    /// A telephone number, international or North American.
    pub const PHONE: u32 = 2;
    /// A UUID: `8-4-4-4-12` hex digits joined by dashes.
    pub const UUID: u32 = 4;
    /// A MAC address: six two-hex-digit groups on one separator.
    pub const MAC: u32 = 5;
    /// A hash digest: a hex run of exactly 32, 40 or 64 with a hex letter.
    pub const HASH_DIGEST: u32 = 6;
    /// A CIDR block: an IPv4 address and a `/prefix`.
    pub const CIDR: u32 = 7;
    /// An email address.
    pub const EMAIL: u32 = 8;
    /// An IP address, v6 or v4.
    pub const IP: u32 = 9;
    /// A payment-card number that passes the Luhn check.
    pub const CREDIT_CARD: u32 = 11;
    /// A decimal-degree coordinate pair.
    pub const GEO: u32 = 12;
    /// A money amount.
    pub const MONEY: u32 = 13;
    /// A semantic-version string.
    pub const VERSION: u32 = 14;
    /// A byte size: a number and a unit ending in `b`.
    pub const BYTE_SIZE: u32 = 15;
    /// A percentage: a number and a `%`.
    pub const PERCENT: u32 = 16;
    /// A duration: a run of number-and-time-unit segments.
    pub const DURATION: u32 = 17;
    /// A physical quantity: a number and a unit symbol.
    pub const QUANTITY: u32 = 18;
    /// A hex color literal: `#` and three or six hex digits.
    pub const HEX_COLOR: u32 = 19;
    /// A filesystem path.
    pub const PATH: u32 = 20;
    /// A base64 blob.
    pub const BASE64: u32 = 21;
    /// A date or timestamp.
    pub const TIMESTAMP: u32 = 22;
    /// A numeric run, which the lexer reads where the chain returned nothing.
    pub const NUMBER: u32 = 23;
    /// A word: letters, digits and underscore, led by anything but a digit.
    pub const WORD: u32 = 24;
}

/// A numeric run: digits, then at most one decimal point which must sit
/// between digits.
///
/// The shape the lexer's own scanner has. A point with nothing after it is not
/// part of the number, and a second point ends it - so `1.` reads one byte and
/// `12.34.56` reads five.
#[must_use]
pub fn number() -> ByteNfa {
    let mut b = Builder::new();
    let all = number_piece(&mut b);
    b.accept(all, id::NUMBER)
}

/// The number's shape, which a byte size begins with too.
fn number_piece(b: &mut Builder) -> Piece {
    let digit = ByteClass::range(b'0', b'9');
    let lead = b.class(digit);
    let whole = b.plus(lead);
    let point = b.class(ByteClass::just(b'.'));
    let after = b.class(digit);
    let after_run = b.plus(after);
    let frac = b.then(point, after_run);
    let maybe_frac = b.maybe(frac);
    b.then(whole, maybe_frac)
}

/// The class a word is made of: letters, digits and underscore.
///
/// A digit continues a word though it cannot lead one, which is why `x1` is one
/// word rather than a word beside a number, and why `1x` is a number beside a
/// word rather than one word.
fn word_byte() -> ByteClass {
    ByteClass::range(b'a', b'z')
        .union(ByteClass::range(b'A', b'Z'))
        .union(ByteClass::range(b'0', b'9'))
        .union(ByteClass::just(b'_'))
}

/// A word: a letter or underscore, then any run of word bytes.
///
/// A digit continues a word but cannot lead one, and that asymmetry is the
/// whole of how the lexer divides a digit from a letter beside it: `x1` is one
/// word, because the letter leads and the digit continues, while `1x` is a
/// number and then a word, because the digit leads nothing.
#[must_use]
pub fn word() -> ByteNfa {
    let mut b = Builder::new();
    let all = word_piece(&mut b);
    b.accept(all, id::WORD)
}

/// The word's shape.
fn word_piece(b: &mut Builder) -> Piece {
    let lead = b.class(word_lead());
    let rest = b.class(word_byte());
    let tail = b.star(rest);
    b.then(lead, tail)
}

/// The class a word may begin with: a letter or underscore, and not a digit.
fn word_lead() -> ByteClass {
    ByteClass::range(b'a', b'z').union(ByteClass::range(b'A', b'Z')).union(ByteClass::just(b'_'))
}

/// The magnitudes a byte size may carry, either case.
fn magnitude() -> ByteClass {
    let mut c = ByteClass::none();
    for m in b"kmgtpeKMGTPE" {
        c.add(*m);
    }
    c
}

/// Letters and digits, which is what a byte size may not be followed by.
fn alphanumeric() -> ByteClass {
    ByteClass::range(b'a', b'z')
        .union(ByteClass::range(b'A', b'Z'))
        .union(ByteClass::range(b'0', b'9'))
}

/// The letters, either case, which a scheme begins with.
///
/// Narrower than [`word_lead`] by the underscore, because a scheme is letters
/// where a word is letters or an underscore.
fn letter() -> ByteClass {
    ByteClass::range(b'a', b'z').union(ByteClass::range(b'A', b'Z'))
}

/// What a scheme carries after its first letter: letters, digits, and the three
/// marks a scheme name admits.
fn scheme_byte() -> ByteClass {
    alphanumeric()
        .union(ByteClass::just(b'+'))
        .union(ByteClass::just(b'.'))
        .union(ByteClass::just(b'-'))
}

/// What a URL's body runs over: every byte but the ones that end it.
///
/// The five ASCII whitespace bytes, the double quote and the two angle
/// brackets end it, and everything else continues it - including the bytes
/// above ASCII, which are not whitespace and so are body.
fn url_body_byte() -> ByteClass {
    let mut stop = ByteClass::none();
    for b in b" \t\n\x0c\r\"<>" {
        stop.add(*b);
    }
    stop.negate()
}

/// The base64url alphabet a token's segments are spelled in: letters, digits,
/// and the two marks that stand where base64 puts `+` and `/`.
fn base64url_byte() -> ByteClass {
    alphanumeric().union(ByteClass::just(b'-')).union(ByteClass::just(b'_'))
}

/// The hex digits, either case.
fn hex() -> ByteClass {
    ByteClass::range(b'0', b'9')
        .union(ByteClass::range(b'a', b'f'))
        .union(ByteClass::range(b'A', b'F'))
}

/// The hex digits that are letters, either case.
fn hex_letter() -> ByteClass {
    ByteClass::range(b'a', b'f').union(ByteClass::range(b'A', b'F'))
}

/// A hash digest: a hex run of exactly 32, 40 or 64 bytes holding at least one
/// hex letter, with no alphanumeric or underscore after it.
///
/// The shape `lexer::try_hash` has. The letter is what keeps a long decimal run
/// from reading as a digest, and it is a condition over the whole run rather
/// than anything about a position in it - so the recognizer is the meet of the
/// three lengths with a run that holds a letter somewhere. Spelled as one piece
/// it would be an alternation over every position the letter could occupy.
///
/// The trailing condition rides the shape, and the meet carries it through: a
/// condition met with an unconditional acceptance is that condition again.
#[must_use]
pub fn hash_digest() -> ByteNfa {
    let follow = alphanumeric().union(ByteClass::just(b'_')).negate();
    let shape = {
        let mut b = Builder::new();
        let mut lengths = exactly(&mut b, hex(), 32);
        for len in [40usize, 64] {
            let next = exactly(&mut b, hex(), len);
            lengths = b.or(lengths, next);
        }
        b.accept_if(lengths, id::HASH_DIGEST, follow, true)
    };
    let holds_a_letter = {
        let mut b = Builder::new();
        let before = b.class(hex());
        let head = b.star(before);
        let letter = b.class(hex_letter());
        let after = b.class(hex());
        let tail = b.star(after);
        let led = b.then(head, letter);
        let all = b.then(led, tail);
        b.accept(all, id::HASH_DIGEST)
    };
    shape.intersect(&holds_a_letter, id::HASH_DIGEST)
}

/// The bytes a base64 body is made of.
fn base64_byte() -> ByteClass {
    alphanumeric().union(ByteClass::just(b'+')).union(ByteClass::just(b'/'))
}

/// What a base64 run may not be followed by, which is any byte that would have
/// been part of it.
///
/// This is what lets the recognizer accept at every valid length and still agree
/// with a scanner that reads the body greedily and tests one candidate: a
/// reading that stopped early is followed by a body byte or by a `=`, and either
/// refuses it.
fn base64_follow() -> ByteClass {
    base64_byte().union(ByteClass::just(b'=')).negate()
}

/// A base64 run holding at least one byte the class holds.
///
/// The shape every "somewhere in this run" condition takes: any body, the byte
/// that matters, any body again.
fn base64_holding(class: ByteClass) -> ByteNfa {
    let mut b = Builder::new();
    let before = b.class(base64_byte());
    let head = b.star(before);
    let wanted = b.class(class);
    let after = b.class(base64_byte());
    let tail = b.star(after);
    let led = b.then(head, wanted);
    let all = b.then(led, tail);
    b.accept(all, id::BASE64)
}

/// A body of `lead` bytes and then any number of four more, then `pad` padding
/// bytes.
///
/// Every length that is at least sixteen and a multiple of four once the padding
/// is counted, spelled as three shapes rather than as a counter: with one `=`
/// the body is fifteen more than a multiple of four, with two it is fourteen.
fn base64_shape(b: &mut Builder, lead: usize, pad: usize) -> Piece {
    let head = exactly(b, base64_byte(), lead);
    let four = exactly(b, base64_byte(), 4);
    let more = b.star(four);
    let body = b.then(head, more);
    if pad == 0 {
        return body;
    }
    let tail = exactly(b, ByteClass::just(b'='), pad);
    b.then(body, tail)
}

/// A base64 blob: a run of base64 bytes whose length with its padding is at
/// least sixteen and a multiple of four, and which is padded or carries a
/// base64-only character or mixes a digit with both cases.
///
/// The shape `lexer::try_base64` has. The last condition is what keeps a plain
/// word or a camelCase identifier from reading as base64, and it is three
/// conditions over the whole run at once - so it is three automata met together,
/// which is the construction this recognizer exists on. A padded run needs none
/// of it, because the padding is itself what makes the run base64.
#[must_use]
pub fn base64() -> ByteNfa {
    let unpadded = {
        let mut b = Builder::new();
        let shape = base64_shape(&mut b, 16, 0);
        b.accept_if(shape, id::BASE64, base64_follow(), true)
    };
    let marked = base64_holding(ByteClass::just(b'+').union(ByteClass::just(b'/')));
    let mixed = base64_holding(ByteClass::range(b'0', b'9'))
        .intersect(&base64_holding(ByteClass::range(b'A', b'Z')), id::BASE64)
        .intersect(&base64_holding(ByteClass::range(b'a', b'z')), id::BASE64);
    let by_mark = unpadded.intersect(&marked, id::BASE64);
    let by_mix = unpadded.intersect(&mixed, id::BASE64);

    let mut b = Builder::new();
    let one = base64_shape(&mut b, 15, 1);
    let one = b.accepting_if(one, id::BASE64, base64_follow(), true);
    let two = base64_shape(&mut b, 14, 2);
    let two = b.accepting_if(two, id::BASE64, base64_follow(), true);
    let mark = b.adopt(&by_mark);
    let mix = b.adopt(&by_mix);
    let padded = b.or(one, two);
    let plain = b.or(mark, mix);
    let all = b.or(padded, plain);
    b.build(all)
}

/// A run of exactly `n` bytes the class holds, which is how every fixed-width
/// group in a UUID, a MAC or a hex color is spelled.
fn exactly(b: &mut Builder, class: ByteClass, n: usize) -> Piece {
    let mut run = b.class(class);
    for _ in 1..n {
        let next = b.class(class);
        run = b.then(run, next);
    }
    run
}

/// A UUID: `8-4-4-4-12` hex digits joined by dashes, with no alphanumeric or
/// dash after it.
///
/// The shape `lexer::try_uuid` has. No hex letter is required anywhere, so a
/// UUID of nothing but zeros is one.
#[must_use]
pub fn uuid() -> ByteNfa {
    let mut b = Builder::new();
    let all = uuid_piece(&mut b);
    let follow = alphanumeric().union(ByteClass::just(b'-')).negate();
    b.accept_if(all, id::UUID, follow, true)
}

/// The UUID's shape.
fn uuid_piece(b: &mut Builder) -> Piece {
    let mut all = exactly(b, hex(), 8);
    for len in [4usize, 4, 4, 12] {
        let sep = b.class(ByteClass::just(b'-'));
        let group = exactly(b, hex(), len);
        let joined = b.then(sep, group);
        all = b.then(all, joined);
    }
    all
}

/// The separators a MAC address may be written with.
fn mac_separators() -> ByteClass {
    ByteClass::just(b':').union(ByteClass::just(b'-'))
}

/// A MAC address: six two-hex-digit groups joined by one separator, `:` or `-`
/// throughout, with no alphanumeric or separator after it.
///
/// The shape `lexer::try_mac` has. The scanner fixes the separator from the
/// byte after the first pair and then requires it at every joint, so the two
/// spellings are separate branches here rather than one branch over a class of
/// both - a class would admit `aa:bb-cc:dd-ee:ff`, which the lexer refuses.
#[must_use]
pub fn mac() -> ByteNfa {
    let mut b = Builder::new();
    let all = mac_piece(&mut b);
    let follow = alphanumeric().union(mac_separators()).negate();
    b.accept_if(all, id::MAC, follow, true)
}

/// The MAC's shape: one branch a spelling.
fn mac_piece(b: &mut Builder) -> Piece {
    let colon = mac_groups(b, b':');
    let hyphen = mac_groups(b, b'-');
    b.or(colon, hyphen)
}

/// Six two-hex-digit groups joined by `sep`.
fn mac_groups(b: &mut Builder, sep: u8) -> Piece {
    let mut all = exactly(b, hex(), 2);
    for _ in 1..6 {
        let joiner = b.class(ByteClass::just(sep));
        let group = exactly(b, hex(), 2);
        let joined = b.then(joiner, group);
        all = b.then(all, joined);
    }
    all
}

/// A hex color: `#` then exactly three or six hex digits, with no alphanumeric
/// or underscore after it.
///
/// The shape `lexer::try_hexcolor` has. Three and six are separate branches
/// because the scanner reads the whole hex run and then requires its length to
/// be one of the two, so `#abcd` is not a three-digit color with a digit left
/// over: the trailing condition refuses that reading and four is not a length.
#[must_use]
pub fn hexcolor() -> ByteNfa {
    let mut b = Builder::new();
    let all = hexcolor_piece(&mut b);
    let follow = alphanumeric().union(ByteClass::just(b'_')).negate();
    b.accept_if(all, id::HEX_COLOR, follow, true)
}

/// The hex color's shape.
fn hexcolor_piece(b: &mut Builder) -> Piece {
    let hash = b.class(ByteClass::just(b'#'));
    let six = exactly(b, hex(), 6);
    let three = exactly(b, hex(), 3);
    let digits = b.or(six, three);
    b.then(hash, digits)
}

/// A byte size: a number, an optional magnitude with an optional binary `i`,
/// then a required `b`, and nothing alphanumeric after it.
///
/// The shape `lexer::try_bytesize` has, which is case sensitive in one place and
/// not in two others: the magnitude and the closing `b` are read either case and
/// the binary `i` is lower case only, so `1.5GiB` is a byte size and `1.5GIB` is
/// a number beside a word. The `b` is required rather than implied by the
/// magnitude, which is what leaves a bare `10M` a number beside a word as well.
#[must_use]
pub fn bytesize() -> ByteNfa {
    let mut b = Builder::new();
    let all = bytesize_piece(&mut b);
    b.accept_if(all, id::BYTE_SIZE, alphanumeric().negate(), true)
}

/// The single-letter time units, lower case being the conventional form the
/// scanner reads.
fn duration_letter() -> ByteClass {
    let mut c = ByteClass::none();
    for u in b"smhdwy" {
        c.add(*u);
    }
    c
}

/// The lead byte of the two-letter time units `ns`, `us` and `ms`, each of
/// which ends in `s`.
fn duration_pair_lead() -> ByteClass {
    let mut c = ByteClass::none();
    for u in b"num" {
        c.add(*u);
    }
    c
}

/// A duration: one or more `number + time unit` segments run together, and
/// nothing alphanumeric after them.
///
/// The shape `lexer::try_duration` has. The segments carry no separator, which
/// is what makes `3h20m` one duration, and the scanner's refusal on a letter
/// after a unit is the same trailing condition the whole run carries: `5string`
/// is not a short duration followed by a word, it is not a duration at all,
/// because the byte that refuses the longest reading refuses every shorter one.
#[must_use]
pub fn duration() -> ByteNfa {
    let mut b = Builder::new();
    let all = duration_piece(&mut b);
    b.accept_if(all, id::DURATION, alphanumeric().negate(), true)
}

/// The duration's shape, without the acceptance that carries its trailing
/// condition.
fn duration_piece(b: &mut Builder) -> Piece {
    let num = number_piece(b);
    let lead = b.class(duration_pair_lead());
    let tail = b.class(ByteClass::just(b's'));
    let pair = b.then(lead, tail);
    let one = b.class(duration_letter());
    let unit = b.or(pair, one);
    let segment = b.then(num, unit);
    b.plus(segment)
}

/// A percentage: a number, then `%`.
///
/// The shape `lexer::try_percent` has, which unlike the byte size's carries no
/// trailing condition at all - the `%` is not a byte anything else continues
/// through, so there is nothing for a run into more letters to fuse with.
#[must_use]
pub fn percent() -> ByteNfa {
    let mut b = Builder::new();
    let all = percent_piece(&mut b);
    b.accept(all, id::PERCENT)
}

/// The percentage's shape.
fn percent_piece(b: &mut Builder) -> Piece {
    let num = number_piece(b);
    let sign = b.class(ByteClass::just(b'%'));
    b.then(num, sign)
}

/// A URL: a scheme, then `://`, then the rest of the run.
///
/// The shape `lexer::try_url` has. The `://` is the marker and the whole of why
/// this never takes a plain dotted word: a scheme on its own is a word, and a
/// word before a colon is a word and then a colon. The body must take at least
/// one byte, so a scheme and its marker alone are not a URL.
///
/// It carries no trailing condition. The body runs to the byte that ends it and
/// there is no longer reading to refuse, which is what a trailing condition is
/// for.
#[must_use]
pub fn url() -> ByteNfa {
    let mut b = Builder::new();
    let all = url_piece(&mut b);
    b.accept(all, id::URL)
}

/// The URL's shape.
fn url_piece(b: &mut Builder) -> Piece {
    let lead = b.class(letter());
    let more = b.class(scheme_byte());
    let more_run = b.star(more);
    let scheme = b.then(lead, more_run);
    let colon = b.class(ByteClass::just(b':'));
    let first_slash = b.class(ByteClass::just(b'/'));
    let second_slash = b.class(ByteClass::just(b'/'));
    let opener = b.then(colon, first_slash);
    let opener = b.then(opener, second_slash);
    let opened = b.then(scheme, opener);
    let byte = b.class(url_body_byte());
    let body = b.plus(byte);
    b.then(opened, body)
}

/// A JSON Web Token: three `.`-separated base64url segments, led by `eyJ`.
///
/// The shape `lexer::try_jwt` has. The `eyJ` is the marker and the reason this
/// takes no ordinary dotted identifier: it is the base64url of a JSON object's
/// opening `{"`, so every token whose header is a JSON object begins with it
/// and almost nothing else does.
///
/// It carries no trailing condition, as `lexer::try_jwt` carries none. The
/// third segment runs to the byte that ends it, and a byte that ends it is a
/// byte no segment could have continued through.
#[must_use]
pub fn jwt() -> ByteNfa {
    let mut b = Builder::new();
    let all = jwt_piece(&mut b);
    b.accept(all, id::JWT)
}

/// The token's shape.
fn jwt_piece(b: &mut Builder) -> Piece {
    let e = b.class(ByteClass::just(b'e'));
    let y = b.class(ByteClass::just(b'y'));
    let j = b.class(ByteClass::just(b'J'));
    let ey = b.then(e, y);
    let marker = b.then(ey, j);
    let rest = b.class(base64url_byte());
    let rest_run = b.star(rest);
    let header = b.then(marker, rest_run);

    let first_dot = b.class(ByteClass::just(b'.'));
    let payload = jwt_segment(b);
    let second_dot = b.class(ByteClass::just(b'.'));
    let signature = jwt_segment(b);

    let through_first = b.then(header, first_dot);
    let through_payload = b.then(through_first, payload);
    let through_second = b.then(through_payload, second_dot);
    b.then(through_second, signature)
}

/// One base64url segment: at least one of its bytes.
fn jwt_segment(b: &mut Builder) -> Piece {
    let byte = b.class(base64url_byte());
    b.plus(byte)
}

/// The byte size's shape, without the acceptance that carries its trailing
/// condition.
fn bytesize_piece(b: &mut Builder) -> Piece {
    let num = number_piece(b);
    let mag = b.class(magnitude());
    let binary = b.class(ByteClass::just(b'i'));
    let maybe_binary = b.maybe(binary);
    let with_binary = b.then(mag, maybe_binary);
    let maybe_mag = b.maybe(with_binary);
    let unit = b.class(ByteClass::just(b'b').union(ByteClass::just(b'B')));
    let head = b.then(num, maybe_mag);
    b.then(head, unit)
}

/// Every recognizer in one automaton, which is the gate between them.
///
/// A run enters every branch at once and [`crate::byte_nfa::ByteNfa::recognize`]
/// settles which one owns it: the branch of highest precedence that accepts at
/// all, and then that branch's own reach. A letter or underscore leads only the
/// word, so there the first byte settles it - `x1` is one word and `1x` is a
/// number and then a word. Where a digit leads several branches, precedence
/// does: `10MB` is a byte size because the byte size outranks the number, not
/// because it reaches further, and `10MBx` is the number `10` because the byte
/// size does not accept there at all.
///
/// That last case is why the branches are alternated rather than tried in turn
/// and compared: the reading the gate falls back to is one the automaton is
/// still carrying, not one that has to be recomputed after a rule rejected the
/// first answer.
#[must_use]
pub fn tokens() -> ByteNfa {
    let mut b = Builder::new();

    let num = number_piece(&mut b);
    let num = b.accepting(num, id::NUMBER);

    let wrd = word_piece(&mut b);
    let wrd = b.accepting(wrd, id::WORD);

    let size = bytesize_piece(&mut b);
    let size = b.accepting_if(size, id::BYTE_SIZE, alphanumeric().negate(), true);

    let pct = percent_piece(&mut b);
    let pct = b.accepting(pct, id::PERCENT);

    let span = duration_piece(&mut b);
    let span = b.accepting_if(span, id::DURATION, alphanumeric().negate(), true);

    let uid = uuid_piece(&mut b);
    let uid_follow = alphanumeric().union(ByteClass::just(b'-')).negate();
    let uid = b.accepting_if(uid, id::UUID, uid_follow, true);

    let hardware = mac_piece(&mut b);
    let hardware_follow = alphanumeric().union(mac_separators()).negate();
    let hardware = b.accepting_if(hardware, id::MAC, hardware_follow, true);

    let color = hexcolor_piece(&mut b);
    let color_follow = alphanumeric().union(ByteClass::just(b'_')).negate();
    let color = b.accepting_if(color, id::HEX_COLOR, color_follow, true);

    let digest = hash_digest();
    let digest = b.adopt(&digest);

    let blob = base64();
    let blob = b.adopt(&blob);

    let link = url_piece(&mut b);
    let link = b.accepting(link, id::URL);

    let token = jwt_piece(&mut b);
    let token = b.accepting(token, id::JWT);

    let mut all = num;
    for piece in [wrd, size, pct, span, uid, hardware, color, digest, blob, link, token] {
        all = b.or(all, piece);
    }
    b.build(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind;

    /// Source of the shape the comparison corpus has, plus the cases where a
    /// number's end is decided by something other than running out of digits,
    /// plus a line of each kind the gate reads that the comparison corpus does
    /// not carry. The last of those is what lets the gate test assert a count
    /// of every kind rather than of the kinds one corpus happened to hold.
    fn corpus() -> Vec<u8> {
        let mut s = String::new();
        for i in 0..200 {
            s.push_str(&format!("let value_{i} = {} ;\n", i * 37));
            s.push_str(&format!("call_{i}(alpha, {}.{}, beta) ;\n", i, i + 1));
            s.push_str(&format!("size_{i} = {}KB ;\n", i * 3));
            s.push_str(&format!("share_{i} = {}.{}% ;\n", i % 100, i));
            s.push_str(&format!("wait_{i} = {}ms ;\n", i * 7));
            s.push_str(&format!("id_{i} = {i:08x}-{i:04x}-{i:04x}-{i:04x}-{i:012x} ;\n"));
            let o = i % 256;
            s.push_str(&format!("mac_{i} = {o:02x}:{o:02x}:{o:02x}:{o:02x}:{o:02x}:{o:02x} ;\n"));
            s.push_str(&format!("tint_{i} = #{:06x} ;\n", i * 1234));
            s.push_str(&format!("link_{i} = http://h{i}.example/p/{i} ;\n"));
            s.push_str(&format!("auth_{i} = eyJh{i:04x}.eyJz{i:04x}.Sfl-{i:04x} ;\n"));
            match i % 3 {
                0 => s.push_str(&format!("sum_{i} = a{i:031x} ;\n")),
                1 => s.push_str(&format!("sum_{i} = b{i:039x} ;\n")),
                _ => s.push_str(&format!("sum_{i} = c{i:063x} ;\n")),
            }
            s.push_str(&format!("blob_{i} = aB{i:012x}Yz ;\n"));
        }
        s.push_str("edge 1. 2.. .3 4.5.6 007 0 9999999999 x1 1x\n");
        s.push_str("sizes 10B 512KB 1.5GiB 2TB 10MB 3b 10M 10MBx 1.5GIB 10iB\n");
        s.push_str("shares 50% 3.5% 100% 50%x .5% 0%\n");
        s.push_str("spans 1500ms 2.5s 3h20m 90s 5m 5us 5ns 5x 5string 3h20 5msx\n");
        s.push_str("fixed 550e8400-e29b-41d4-a716-446655440000 aa:bb:cc:dd:ee:ff\n");
        s.push_str("more aa-bb-cc-dd-ee-ff #abc #abcdef #abcd aa:bb-cc:dd-ee:ff\n");
        s.into_bytes()
    }

    /// The property the byte grain rests on: the automaton ends a number where
    /// the lexer ends it, on every number the lexer finds.
    ///
    /// Asked of the lexer's own output rather than of a list written here, so
    /// the corpus decides the cases rather than my expectations of it.
    #[test]
    fn the_automaton_ends_a_number_where_the_lexer_does() {
        let input = corpus();
        let toks = crate::lexer::lex(&input);
        let nfa = number();
        let mut seen = 0usize;
        for t in &toks {
            if t.kind != TokenKind::Number {
                continue;
            }
            seen += 1;
            let at = t.start();
            let got = nfa.recognize(&input[at..]);
            assert_eq!(
                got,
                Some((t.end() - at, id::NUMBER)),
                "the lexer read {:?} at {at} and the automaton read {got:?}",
                String::from_utf8_lossy(&input[at..t.end()])
            );
        }
        assert!(seen > 400, "the corpus holds numbers to check: {seen}");
    }

    /// The byte size ends a run where `lexer::try_bytesize` ends it, in the
    /// cases where the two could most easily differ.
    ///
    /// Case is the one the shape does not make obvious: a magnitude and the
    /// closing `b` are read either case and the binary `i` is lower case only,
    /// so the same six bytes are a byte size written one way and a number beside
    /// a word written the other.
    #[test]
    fn the_byte_size_reads_the_units_the_lexer_reads() {
        let nfa = bytesize();
        let cases: [(&[u8], Option<usize>); 11] = [
            (b"10B", Some(3)),
            (b"512KB", Some(5)),
            (b"1.5GiB", Some(6)),
            (b"2TB", Some(3)),
            (b"3b", Some(2)),
            (b"10MB x", Some(4)),
            (b"1.5GIB", None),
            (b"10MBx", None),
            (b"10MB7", None),
            (b"10M", None),
            (b"10iB", None),
        ];
        for (run, want) in cases {
            assert_eq!(
                nfa.recognize(run).map(|(end, _)| end),
                want,
                "{}",
                String::from_utf8_lossy(run)
            );
        }
    }

    /// A URL needs its marker and a body, and the bytes that end a body are not
    /// all whitespace.
    ///
    /// The marker is what separates a URL from the word its scheme would
    /// otherwise be, and the body is why a scheme and marker alone are not one.
    /// The quote and the angle brackets are the cases the shape does not make
    /// obvious: they end a body though a run would otherwise carry straight
    /// through them.
    #[test]
    fn a_url_needs_its_marker_and_a_body() {
        let nfa = url();
        let cases: [(&[u8], Option<usize>); 9] = [
            (b"http://x.com/p", Some(14)),
            (b"https://a.b", Some(11)),
            (b"git+ssh://h/r.git", Some(17)),
            (b"http://x y", Some(8)),
            (b"http://x\"y", Some(8)),
            (b"http://x<y", Some(8)),
            (b"http://", None),
            (b"http:/x", None),
            (b"://x", None),
        ];
        for (run, want) in cases {
            assert_eq!(
                nfa.recognize(run).map(|(end, _)| end),
                want,
                "{}",
                String::from_utf8_lossy(run)
            );
        }
    }

    /// The automaton ends a URL where the lexer ends it, asked of the lexer's
    /// own output rather than of a list written here.
    ///
    /// The four shapes are the ones whose ends differ: a URL ended by
    /// whitespace, one ended by the quote around it, one ended by the angle
    /// bracket around it, and one whose scheme carries a mark.
    #[test]
    fn the_automaton_ends_a_url_where_the_lexer_does() {
        let mut s = String::new();
        for i in 0..40 {
            s.push_str(&format!("see http://host{i}.example/p/{i}?q={i} ;\n"));
            s.push_str(&format!("src=\"https://cdn{i}.example/a.js\" ;\n"));
            s.push_str(&format!("<ftp://files{i}.example/x> ;\n"));
            s.push_str(&format!("git+ssh://git{i}.example/r.git ;\n"));
        }
        let input = s.into_bytes();
        let toks = crate::lexer::lex(&input);
        let nfa = url();
        let mut seen = 0usize;
        for t in &toks {
            if t.kind != TokenKind::Url {
                continue;
            }
            seen += 1;
            let at = t.start();
            let got = nfa.recognize(&input[at..]);
            assert_eq!(
                got,
                Some((t.end() - at, id::URL)),
                "the lexer read {:?} at {at} and the automaton read {got:?}",
                String::from_utf8_lossy(&input[at..t.end()])
            );
        }
        assert!(seen > 100, "the input holds URLs to check: {seen}");
    }

    /// A token needs its marker and all three segments.
    ///
    /// Two dots are the shape and `eyJ` is the marker, and the marker is the
    /// half worth testing: `abc.def.ghi` has the shape exactly and is not a
    /// token, which is the whole of why this never takes a dotted identifier.
    #[test]
    fn a_token_needs_its_marker_and_three_segments() {
        let nfa = jwt();
        let cases: [(&[u8], Option<usize>); 8] = [
            (b"eyJa.b.c", Some(8)),
            (b"eyJhbGci.eyJzdWIi.SflKxw", Some(24)),
            (b"eyJa-b_c.d.e", Some(12)),
            (b"eyJa.b.c.d", Some(8)),
            (b"abc.def.ghi", None),
            (b"eyJa.b.", None),
            (b"eyJa.b", None),
            (b"eyj.b.c", None),
        ];
        for (run, want) in cases {
            assert_eq!(
                nfa.recognize(run).map(|(end, _)| end),
                want,
                "{}",
                String::from_utf8_lossy(run)
            );
        }
    }

    /// The automaton ends a token where the lexer ends it, asked of the lexer's
    /// own output rather than of a list written here.
    #[test]
    fn the_automaton_ends_a_token_where_the_lexer_does() {
        let mut s = String::new();
        for i in 0..40 {
            s.push_str(&format!("auth = eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ{i}9.Sfl-Kx_w{i} ;\n"));
        }
        let input = s.into_bytes();
        let toks = crate::lexer::lex(&input);
        let nfa = jwt();
        let mut seen = 0usize;
        for t in &toks {
            if t.kind != TokenKind::Jwt {
                continue;
            }
            seen += 1;
            let at = t.start();
            let got = nfa.recognize(&input[at..]);
            assert_eq!(
                got,
                Some((t.end() - at, id::JWT)),
                "the lexer read {:?} at {at} and the automaton read {got:?}",
                String::from_utf8_lossy(&input[at..t.end()])
            );
        }
        assert_eq!(seen, 40, "the input holds one token a line");
    }

    /// The percentage ends where `lexer::try_percent` ends it, and the contrast
    /// with the byte size is what the test is for: a `%` is not a byte a word
    /// runs through, so a letter after one ends a percentage rather than
    /// refusing it.
    #[test]
    fn a_percentage_needs_no_trailing_condition_where_a_byte_size_does() {
        let pct = percent();
        let cases: [(&[u8], Option<usize>); 5] = [
            (b"50%", Some(3)),
            (b"3.5%", Some(4)),
            (b"0%", Some(2)),
            (b"50%x", Some(3)),
            (b"50", None),
        ];
        for (run, want) in cases {
            assert_eq!(
                pct.recognize(run).map(|(end, _)| end),
                want,
                "{}",
                String::from_utf8_lossy(run)
            );
        }
        assert_eq!(
            bytesize().recognize(b"10MBx"),
            None,
            "the byte size refuses the letter the percentage admits"
        );
    }

    /// The duration ends where `lexer::try_duration` ends it, including the
    /// runs it refuses outright.
    ///
    /// The refusals are the half worth asserting. A letter after a unit does
    /// not shorten the reading, it removes it: `5string` is not `5s` and then a
    /// word, and `3h20` is not `3h` and then a number, because the byte that
    /// refuses the longest acceptance refuses the shorter ones for the same
    /// reason. A recognizer that returned the short reading would put a
    /// duration where the lexer puts a number.
    #[test]
    fn the_duration_reads_the_segments_the_lexer_reads() {
        let nfa = duration();
        let cases: [(&[u8], Option<usize>); 11] = [
            (b"1500ms", Some(6)),
            (b"2.5s", Some(4)),
            (b"3h20m", Some(5)),
            (b"90s", Some(3)),
            (b"5m", Some(2)),
            (b"5us", Some(3)),
            (b"5ns", Some(3)),
            (b"5x", None),
            (b"5string", None),
            (b"3h20", None),
            (b"5msx", None),
        ];
        for (run, want) in cases {
            assert_eq!(
                nfa.recognize(run).map(|(end, _)| end),
                want,
                "{}",
                String::from_utf8_lossy(run)
            );
        }
    }

    /// The lexer takes a shorter reading where precedence says to, which is why
    /// [`crate::byte_nfa::ByteNfa::recognize`] is not a longest match.
    ///
    /// Forty hex bytes, a `+`, then seven more base64 bytes. The digest
    /// recognizer accepts the first forty, because `+` is neither alphanumeric
    /// nor an underscore and so passes its trailing guard; base64 would accept
    /// all forty-eight, its run crossing the `+` that ended the digest's, the
    /// length a multiple of four and the `+` itself the character that makes it
    /// base64 rather than a word. The chain reaches the digest first, so the
    /// lexer reads forty - and a gate holding both recognizers and taking the
    /// longer of them would read forty-eight and disagree.
    ///
    /// Both recognizers are in the gate now, so this asks the gate as well as
    /// the lexer. It is the one input in this module where the two rules give
    /// different answers, and the assertion is what says which rule is
    /// implemented rather than merely passing under either.
    #[test]
    fn the_lexer_takes_the_shorter_reading_where_precedence_says_so() {
        let mut s = String::from("blob a");
        for _ in 0..39 {
            s.push('0');
        }
        s.push_str("+AAAAAAA end");
        let input = s.into_bytes();
        let toks = crate::lexer::lex(&input);
        let read: Vec<(TokenKind, usize)> =
            toks.iter().map(|t| (t.kind, t.end() - t.start())).collect();
        assert_eq!(
            read,
            [
                (TokenKind::Word, 4),
                (TokenKind::Whitespace, 1),
                (TokenKind::HashDigest, 40),
                (TokenKind::Punct, 1),
                (TokenKind::Word, 7),
                (TokenKind::Whitespace, 1),
                (TokenKind::Word, 3),
            ],
            "the lexer no longer prefers the digest to the longer base64 run"
        );
        let digest = toks
            .iter()
            .find(|t| t.kind == TokenKind::HashDigest)
            .expect("the lexer read a digest in it");
        assert_eq!(
            tokens().recognize(&input[digest.start()..]),
            Some((40, id::HASH_DIGEST)),
            "the gate took the forty-eight byte base64 run the lexer passed over"
        );
    }

    /// The base64 blob reads what `lexer::try_base64` reads, and refuses the
    /// runs that are only shaped like it.
    ///
    /// The charset condition is the half a shape cannot carry: sixteen lower
    /// case letters are the right length and are a word, and the same sixteen
    /// with a digit and a capital among them are a blob.
    #[test]
    fn the_base64_blob_needs_its_length_and_its_charset() {
        let nfa = base64();
        assert_eq!(nfa.recognize(b"aB1dEfGhIjKlMnOp"), Some((16, id::BASE64)), "a mixed run");
        assert_eq!(nfa.recognize(b"abcdefghijklmnop"), None, "one case and no digit");
        assert_eq!(nfa.recognize(b"abcdefghijklmno+"), Some((16, id::BASE64)), "a base64 byte");
        assert_eq!(nfa.recognize(b"abcdefghijklmno="), Some((16, id::BASE64)), "one pad");
        assert_eq!(nfa.recognize(b"abcdefghijklmn=="), Some((16, id::BASE64)), "two pads");
        assert_eq!(nfa.recognize(b"aB1dEfGhIjKlMnO"), None, "fifteen is no length");
        assert_eq!(nfa.recognize(b"aB1dEfGhIjKlMnOpQrSt"), Some((20, id::BASE64)), "the next one");
        assert_eq!(nfa.recognize(b"aB1dEfGhIjKlMnOpQr"), None, "eighteen is no length");
    }

    /// The fixed-width shapes end where their scanners end them, and refuse the
    /// near misses a length alone would admit.
    ///
    /// Each refusal is a different mechanism. A short UUID group has no
    /// acceptance to refuse - the shape simply does not complete. A mixed MAC is
    /// refused by there being two branches rather than one over a class of both
    /// separators. And `#abcd` is refused by the trailing condition, which is
    /// the only one of the three that needs a byte the match does not cover.
    #[test]
    fn the_fixed_shapes_read_what_their_scanners_read() {
        let u = uuid();
        assert_eq!(u.recognize(b"550e8400-e29b-41d4-a716-446655440000"), Some((36, id::UUID)));
        assert_eq!(u.recognize(b"00000000-0000-0000-0000-000000000000"), Some((36, id::UUID)));
        assert_eq!(u.recognize(b"550e8400-e29b-41d4-a716-44665544000"), None, "a short group");

        let m = mac();
        assert_eq!(m.recognize(b"aa:bb:cc:dd:ee:ff"), Some((17, id::MAC)));
        assert_eq!(m.recognize(b"aa-bb-cc-dd-ee-ff"), Some((17, id::MAC)));
        assert_eq!(m.recognize(b"aa:bb-cc:dd-ee:ff"), None, "one separator throughout");

        let c = hexcolor();
        assert_eq!(c.recognize(b"#abc"), Some((4, id::HEX_COLOR)));
        assert_eq!(c.recognize(b"#abcdef"), Some((7, id::HEX_COLOR)));
        assert_eq!(c.recognize(b"#abcd"), None, "four is not a length a color has");
    }

    /// The digest reads the three lengths the lexer reads, and refuses a run
    /// that holds no hex letter - which is the condition no single piece could
    /// carry and the reason this recognizer is a meet.
    ///
    /// The three refusals are worth separating. A decimal run of the right
    /// length is refused by the condition; a run of thirty-nine is refused by
    /// there being no such length; and a run of forty-one is refused by the
    /// trailing condition, because the fortieth byte is a hex digit and so the
    /// forty-byte reading cannot stand either.
    #[test]
    fn the_digest_needs_a_letter_and_one_of_three_lengths() {
        let nfa = hash_digest();
        let md5 = "d41d8cd98f00b204e9800998ecf8427e";
        let sha1 = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        let sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(nfa.recognize(md5.as_bytes()), Some((32, id::HASH_DIGEST)));
        assert_eq!(nfa.recognize(sha1.as_bytes()), Some((40, id::HASH_DIGEST)));
        assert_eq!(nfa.recognize(sha256.as_bytes()), Some((64, id::HASH_DIGEST)));
        assert_eq!(
            nfa.recognize(format!("{sha1} rest").as_bytes()),
            Some((40, id::HASH_DIGEST)),
            "a boundary after it"
        );
        assert_eq!(nfa.recognize("0".repeat(40).as_bytes()), None, "no hex letter in it");
        assert_eq!(nfa.recognize(&sha1.as_bytes()[..39]), None, "thirty-nine is not a length");
        assert_eq!(nfa.recognize(format!("{sha1}0").as_bytes()), None, "a hex byte runs it on");
    }

    /// What the gate is asked is a position the lexer begins a token at, and
    /// asking it anywhere else reads runs the lexer never cut.
    ///
    /// This is the boundary every other test here sits inside, and it is worth a
    /// test of its own because the tests that pass do not show it: they walk the
    /// lexer's own tokens and ask about each one's start, so they never ask at a
    /// byte the lexer is in the middle of something at. A caller scanning raw
    /// bytes does not know where the tokens begin - that is what it is scanning
    /// to find out - so it asks everywhere, and inside a quoted string every
    /// recognizer here answers as if the quote were not there.
    ///
    /// The lexer takes the whole string as one token and nothing inside it. The
    /// gate carries no quote rule, so it reads the contents. Neither is wrong
    /// about its own question; they are different questions, and a prefilter
    /// built on this one without a quote reader in front of it would claim
    /// matches inside strings.
    #[test]
    fn the_gate_reads_inside_a_string_where_the_lexer_reads_one_token() {
        let input = b"let s = \"hello world\" ;".to_vec();
        let toks = crate::lexer::lex(&input);
        let quoted = toks
            .iter()
            .find(|t| t.kind == TokenKind::Quoted)
            .expect("the lexer took the string as one token");
        assert_eq!(
            &input[quoted.start()..quoted.end()],
            b"\"hello world\"",
            "the whole string, quotes included"
        );

        let inside = quoted.start() + 1;
        assert_eq!(
            tokens().recognize(&input[inside..]),
            Some((5, id::WORD)),
            "the gate reads `hello` where the lexer is inside a string"
        );
        assert!(
            !toks.iter().any(|t| t.start() == inside),
            "the lexer begins no token there, which is the whole of the difference"
        );
    }

    /// The gate reads every kind where the lexer reads it, and calls each by
    /// the same name.
    ///
    /// This is the claim the byte grain stands or falls on: not that a
    /// recognizer can be written, but that several of them together divide a
    /// stream the way the lexer divides it. Asked of the lexer's output, so the
    /// corpus chooses the cases.
    #[test]
    fn the_gate_divides_a_stream_the_way_the_lexer_divides_it() {
        let input = corpus();
        let toks = crate::lexer::lex(&input);
        let nfa = tokens();
        let (mut words, mut numbers) = (0usize, 0usize);
        let (mut sizes, mut shares, mut spans) = (0usize, 0usize, 0usize);
        let (mut uuids, mut macs, mut colors) = (0usize, 0usize, 0usize);
        let (mut digests, mut blobs) = (0usize, 0usize);
        let (mut links, mut webtokens) = (0usize, 0usize);
        for t in &toks {
            let want = match t.kind {
                TokenKind::Number => {
                    numbers += 1;
                    id::NUMBER
                }
                TokenKind::Word => {
                    words += 1;
                    id::WORD
                }
                TokenKind::ByteSize => {
                    sizes += 1;
                    id::BYTE_SIZE
                }
                TokenKind::Percent => {
                    shares += 1;
                    id::PERCENT
                }
                TokenKind::Duration => {
                    spans += 1;
                    id::DURATION
                }
                TokenKind::Uuid => {
                    uuids += 1;
                    id::UUID
                }
                TokenKind::Mac => {
                    macs += 1;
                    id::MAC
                }
                TokenKind::HexColor => {
                    colors += 1;
                    id::HEX_COLOR
                }
                TokenKind::HashDigest => {
                    digests += 1;
                    id::HASH_DIGEST
                }
                TokenKind::Base64 => {
                    blobs += 1;
                    id::BASE64
                }
                TokenKind::Url => {
                    links += 1;
                    id::URL
                }
                TokenKind::Jwt => {
                    webtokens += 1;
                    id::JWT
                }
                _ => continue,
            };
            let at = t.start();
            assert_eq!(
                nfa.recognize(&input[at..]),
                Some((t.end() - at, want)),
                "the lexer read {:?} at {at} as {:?}",
                String::from_utf8_lossy(&input[at..t.end()]),
                t.kind
            );
        }
        assert!(
            words > 400
                && numbers > 400
                && sizes > 100
                && shares > 100
                && spans > 100
                && uuids > 100
                && macs > 100
                && colors > 100
                && digests > 100
                && blobs > 100
                && links > 100
                && webtokens > 100,
            "the corpus holds every kind the gate reads: {words}, {numbers}, {sizes}, \
             {shares}, {spans}, {uuids}, {macs}, {colors}, {digests}, {blobs}, {links} \
             and {webtokens}"
        );
    }

    /// Where a digit sits beside a letter, the gate divides the run where the
    /// lexer divides it.
    ///
    /// The corpus test covers this at scale; this one names the runs the gate
    /// divides and asserts the division itself, so widening a lead class to
    /// admit a digit fails here with the input that caused it in view - `1x`
    /// would come back one word of two bytes where the lexer reads a number of
    /// one.
    ///
    /// `3b` is the run no lead class can divide: it begins on a digit like a
    /// number and reaches further than one, so it is a byte size only because
    /// the longest acceptance decides between the two branches that both
    /// started.
    #[test]
    fn a_digit_beside_a_letter_divides_where_the_lexer_divides_it() {
        let input = b"x1 1x a_2 3b".to_vec();
        let toks = crate::lexer::lex(&input);
        let nfa = tokens();
        let mut read = Vec::new();
        for t in &toks {
            let want = match t.kind {
                TokenKind::Number => id::NUMBER,
                TokenKind::Word => id::WORD,
                TokenKind::ByteSize => id::BYTE_SIZE,
                _ => continue,
            };
            let at = t.start();
            assert_eq!(
                nfa.recognize(&input[at..]),
                Some((t.end() - at, want)),
                "the lexer read {:?} at {at} as {:?}",
                String::from_utf8_lossy(&input[at..t.end()]),
                t.kind
            );
            read.push(String::from_utf8_lossy(&input[at..t.end()]).into_owned());
        }
        assert_eq!(read, ["x1", "1", "x", "a_2", "3b"], "the runs the three branches reach");
    }
}
