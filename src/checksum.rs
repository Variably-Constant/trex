//! The checks the shipped library's guards apply. A shape says what a token
//! looks like; where its standard defines a check digit, a checksum or a
//! hash, the guard says whether the bytes are one.

/// The Luhn (mod-10) check over digit values, most significant first.
pub(crate) fn luhn_values(digits: &[u8]) -> bool {
    let mut sum = 0u32;
    for (k, &d) in digits.iter().rev().enumerate() {
        let mut v = u32::from(d);
        if k % 2 == 1 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    !digits.is_empty() && sum.is_multiple_of(10)
}

/// The CRC-32 a GitHub token's last six characters carry, over the thirty
/// before them.
///
/// GitHub's token format is a prefix, thirty base62 characters of random
/// data and six more holding the CRC-32 of those thirty, which is what lets
/// a scanner tell a token from a string that merely looks like one. Only
/// the `gh?_` forms are checked here: the fine-grained `github_pat_` form
/// is laid out differently, and a guard that cannot verify a token must
/// pass it rather than refuse it, since a refusal is a secret reported as
/// ordinary text.
pub(crate) fn github_token(text: &[u8]) -> bool {
    let Some(body) = text.strip_prefix(b"ghp_").or_else(|| {
        [b"gho_", b"ghu_", b"ghs_", b"ghr_"].iter().find_map(|p| text.strip_prefix(*p))
    }) else {
        // Not a form this check knows, which the shape has already
        // accepted: `github_pat_` passes here.
        return true;
    };
    if body.len() != 36 {
        return false;
    }
    let (random, checksum) = body.split_at(30);
    let Some(carried) = base62_value(checksum) else {
        return false;
    };
    carried == u64::from(crc32(random))
}

/// The value six base62 characters hold, most significant first, or `None`
/// for a byte outside the alphabet. Six characters reach past a `u32`, so
/// the value is read wider than the checksum it is compared against and a
/// token carrying too large a number fails rather than wrapping into a
/// value that matches.
fn base62_value(text: &[u8]) -> Option<u64> {
    let mut value = 0u64;
    for &b in text {
        let digit = match b {
            b'0'..=b'9' => u64::from(b - b'0'),
            b'A'..=b'Z' => u64::from(b - b'A') + 10,
            b'a'..=b'z' => u64::from(b - b'a') + 36,
            _ => return None,
        };
        value = value.checked_mul(62)?.checked_add(digit)?;
    }
    Some(value)
}

/// CRC-32 as IEEE 802.3 defines it and zlib computes it: the reflected
/// polynomial, all ones in and all ones out.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let take = crc & 1;
            crc >>= 1;
            if take == 1 {
                crc ^= 0xEDB8_8320;
            }
        }
    }
    !crc
}

/// [`luhn_values`] over ASCII digits; any other byte fails.
pub(crate) fn luhn(text: &[u8]) -> bool {
    if !text.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let values: Vec<u8> = text.iter().map(|b| b - b'0').collect();
    luhn_values(&values)
}

/// ISO 7064 mod 97-10 over `text` with its first four characters moved to
/// the end and letters read as 10 to 35, true where the remainder is 1.
pub(crate) fn mod97_is_one(text: &[u8]) -> bool {
    if text.len() < 5 {
        return false;
    }
    let (head, tail) = text.split_at(4);
    let mut rem: u32 = 0;
    for &b in tail.iter().chain(head) {
        let v = match b {
            b'0'..=b'9' => u32::from(b - b'0'),
            b'A'..=b'Z' => u32::from(b - b'A') + 10,
            b'a'..=b'z' => u32::from(b - b'a') + 10,
            _ => return false,
        };
        rem = if v >= 10 { (rem * 100 + v) % 97 } else { (rem * 10 + v) % 97 };
    }
    rem == 1
}

/// The IBAN length each country registers with SWIFT, by ISO 3166 code.
const IBAN_LENGTHS: &[(&str, u8)] = &[
    ("AD", 24), ("AE", 23), ("AL", 28), ("AT", 20), ("AZ", 28), ("BA", 20), ("BE", 16),
    ("BG", 22), ("BH", 22), ("BI", 27), ("BR", 29), ("BY", 28), ("CH", 21), ("CR", 22),
    ("CY", 28), ("CZ", 24), ("DE", 22), ("DJ", 27), ("DK", 18), ("DO", 28), ("EE", 20),
    ("EG", 29), ("ES", 24), ("FI", 18), ("FK", 18), ("FO", 18), ("FR", 27), ("GB", 22),
    ("GE", 22), ("GI", 23), ("GL", 18), ("GR", 27), ("GT", 28), ("HR", 21), ("HU", 28),
    ("IE", 22), ("IL", 23), ("IQ", 23), ("IS", 26), ("IT", 27), ("JO", 30), ("KW", 30),
    ("KZ", 20), ("LB", 28), ("LC", 32), ("LI", 21), ("LT", 20), ("LU", 20), ("LV", 21),
    ("LY", 25), ("MC", 27), ("MD", 24), ("ME", 22), ("MK", 19), ("MN", 20), ("MR", 27),
    ("MT", 31), ("MU", 30), ("NI", 28), ("NL", 18), ("NO", 15), ("OM", 23), ("PK", 24),
    ("PL", 28), ("PS", 29), ("PT", 25), ("QA", 29), ("RO", 24), ("RS", 22), ("RU", 33),
    ("SA", 24), ("SC", 31), ("SD", 18), ("SE", 24), ("SI", 19), ("SK", 24), ("SM", 27),
    ("SO", 23), ("ST", 25), ("SV", 28), ("TL", 23), ("TN", 24), ("TR", 26), ("UA", 29),
    ("VA", 22), ("VG", 24), ("XK", 20), ("YE", 30),
];

/// An IBAN as written, spaces allowed between groups: a registered
/// country, that country's length, two check digits, and mod 97-10 at 1.
pub(crate) fn iban(text: &[u8]) -> bool {
    let compact: Vec<u8> = text.iter().copied().filter(|b| *b != b' ').collect();
    if compact.len() < 5 {
        return false;
    }
    let country = &compact[..2];
    let Some(&(_, want)) = IBAN_LENGTHS.iter().find(|(cc, _)| cc.as_bytes() == country) else {
        return false;
    };
    compact.len() == usize::from(want)
        && compact[2..4].iter().all(u8::is_ascii_digit)
        && mod97_is_one(&compact)
}

/// The GS1 check over every digit including the last: from the right,
/// weights 1 and 3 alternating, a total divisible by ten. EAN-8, UPC-A,
/// EAN-13 and GTIN-14 all check this way.
pub(crate) fn gs1(digits: &[u8]) -> bool {
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let mut sum = 0u32;
    for (k, &b) in digits.iter().rev().enumerate() {
        let v = u32::from(b - b'0');
        sum += if k % 2 == 1 { v * 3 } else { v };
    }
    sum.is_multiple_of(10)
}

/// An ISBN as written, hyphens and spaces allowed: ten characters checked
/// mod 11 with `X` as ten, or thirteen digits under 978 or 979 checked as
/// an EAN-13.
pub(crate) fn isbn(text: &[u8]) -> bool {
    let compact: Vec<u8> = text.iter().copied().filter(|b| *b != b'-' && *b != b' ').collect();
    match compact.len() {
        10 => {
            let mut sum = 0u32;
            for (i, &b) in compact.iter().enumerate() {
                let v = match b {
                    b'0'..=b'9' => u32::from(b - b'0'),
                    b'X' | b'x' if i == 9 => 10,
                    _ => return false,
                };
                sum += v * (10 - u32::try_from(i).expect("ten positions"));
            }
            sum.is_multiple_of(11)
        }
        13 => (compact.starts_with(b"978") || compact.starts_with(b"979")) && gs1(&compact),
        _ => false,
    }
}

/// A vehicle identification number: seventeen characters without I, O or
/// Q, whose ninth is the check digit of the weighted transliteration.
pub(crate) fn vin(text: &[u8]) -> bool {
    const WEIGHTS: [u32; 17] = [8, 7, 6, 5, 4, 3, 2, 10, 0, 9, 8, 7, 6, 5, 4, 3, 2];
    if text.len() != 17 {
        return false;
    }
    let mut sum = 0u32;
    for (&b, &w) in text.iter().zip(WEIGHTS.iter()) {
        let v = match b {
            b'0'..=b'9' => u32::from(b - b'0'),
            b'A' | b'J' => 1,
            b'B' | b'K' | b'S' => 2,
            b'C' | b'L' | b'T' => 3,
            b'D' | b'M' | b'U' => 4,
            b'E' | b'N' | b'V' => 5,
            b'F' | b'W' => 6,
            b'G' | b'P' | b'X' => 7,
            b'H' | b'Y' => 8,
            b'R' | b'Z' => 9,
            _ => return false,
        };
        sum += v * w;
    }
    let want = match sum % 11 {
        10 => b'X',
        d => b'0' + u8::try_from(d).expect("a digit"),
    };
    text[8] == want
}

/// An ISIN: two letters, nine alphanumerics and a check digit, Luhn over
/// the letters expanded to two digits each.
pub(crate) fn isin(text: &[u8]) -> bool {
    if text.len() != 12 {
        return false;
    }
    let mut digits = Vec::with_capacity(24);
    for &b in text {
        match b {
            b'0'..=b'9' => digits.push(b - b'0'),
            b'A'..=b'Z' => {
                let v = b - b'A' + 10;
                digits.push(v / 10);
                digits.push(v % 10);
            }
            _ => return false,
        }
    }
    luhn_values(&digits)
}

/// Keccak-f[1600] round constants.
const KECCAK_RC: [u64; 24] = [
    0x0000_0000_0000_0001, 0x0000_0000_0000_8082, 0x8000_0000_0000_808a, 0x8000_0000_8000_8000,
    0x0000_0000_0000_808b, 0x0000_0000_8000_0001, 0x8000_0000_8000_8081, 0x8000_0000_0000_8009,
    0x0000_0000_0000_008a, 0x0000_0000_0000_0088, 0x0000_0000_8000_8009, 0x0000_0000_8000_000a,
    0x0000_0000_8000_808b, 0x8000_0000_0000_008b, 0x8000_0000_0000_8089, 0x8000_0000_0000_8003,
    0x8000_0000_0000_8002, 0x8000_0000_0000_0080, 0x0000_0000_0000_800a, 0x8000_0000_8000_000a,
    0x8000_0000_8000_8081, 0x8000_0000_0000_8080, 0x0000_0000_8000_0001, 0x8000_0000_8000_8008,
];
/// Keccak rho rotations, in pi order.
const KECCAK_ROT: [u32; 24] = [
    1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44,
];
/// Keccak pi lane order.
const KECCAK_PI: [usize; 24] = [
    10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4, 15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1,
];

fn keccak_f(st: &mut [u64; 25]) {
    for rc in KECCAK_RC {
        let mut parity = [0u64; 5];
        for (x, p) in parity.iter_mut().enumerate() {
            *p = st[x] ^ st[x + 5] ^ st[x + 10] ^ st[x + 15] ^ st[x + 20];
        }
        for x in 0..5 {
            let t = parity[(x + 4) % 5] ^ parity[(x + 1) % 5].rotate_left(1);
            for y in 0..5 {
                st[x + 5 * y] ^= t;
            }
        }
        let mut carried = st[1];
        for (&lane, &rot) in KECCAK_PI.iter().zip(KECCAK_ROT.iter()) {
            let next = st[lane];
            st[lane] = carried.rotate_left(rot);
            carried = next;
        }
        for y in 0..5 {
            let mut row = [0u64; 5];
            row.copy_from_slice(&st[5 * y..5 * y + 5]);
            for x in 0..5 {
                st[5 * y + x] = row[x] ^ (!row[(x + 1) % 5] & row[(x + 2) % 5]);
            }
        }
        st[0] ^= rc;
    }
}

/// Keccak-256 of `data`: the pre-standard padding Ethereum uses, not
/// SHA3-256's.
pub(crate) fn keccak256(data: &[u8]) -> [u8; 32] {
    const RATE: usize = 136;
    let mut st = [0u64; 25];
    let absorb = |st: &mut [u64; 25], block: &[u8]| {
        for (lane, bytes) in st.iter_mut().zip(block.as_chunks::<8>().0) {
            *lane ^= u64::from_le_bytes(*bytes);
        }
        keccak_f(st);
    };
    let (blocks, rest) = data.as_chunks::<RATE>();
    for block in blocks {
        absorb(&mut st, block);
    }
    let mut last = [0u8; RATE];
    last[..rest.len()].copy_from_slice(rest);
    last[rest.len()] ^= 0x01;
    last[RATE - 1] ^= 0x80;
    absorb(&mut st, &last);
    let mut out = [0u8; 32];
    for (bytes, lane) in out.as_chunks_mut::<8>().0.iter_mut().zip(st.iter()) {
        *bytes = lane.to_le_bytes();
    }
    out
}

/// An Ethereum address as written after `0x`: forty hex digits, and where
/// the letters mix case, the EIP-55 case of the address's own hash.
pub(crate) fn ethereum_address(hex: &[u8]) -> bool {
    if hex.len() != 40 || !hex.iter().all(u8::is_ascii_hexdigit) {
        return false;
    }
    let has_upper = hex.iter().any(u8::is_ascii_uppercase);
    let has_lower = hex.iter().any(u8::is_ascii_lowercase);
    if !(has_upper && has_lower) {
        return true;
    }
    let lower: Vec<u8> = hex.iter().map(u8::to_ascii_lowercase).collect();
    let hash = keccak256(&lower);
    hex.iter().enumerate().all(|(i, &b)| {
        if !b.is_ascii_alphabetic() {
            return true;
        }
        let nibble = if i % 2 == 0 { hash[i / 2] >> 4 } else { hash[i / 2] & 0x0f };
        (nibble >= 8) == b.is_ascii_uppercase()
    })
}

const SHA256_K: [u32; 64] = [
    0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1, 0x923f_82a4,
    0xab1c_5ed5, 0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3, 0x72be_5d74, 0x80de_b1fe,
    0x9bdc_06a7, 0xc19b_f174, 0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f,
    0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da, 0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7,
    0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967, 0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc,
    0x5338_0d13, 0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85, 0xa2bf_e8a1, 0xa81a_664b,
    0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070, 0x19a4_c116,
    0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
    0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208, 0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7,
    0xc671_78f2,
];

/// SHA-256 of `data`.
pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let mut padded = data.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    let bits = u64::try_from(data.len()).expect("a length in bytes") * 8;
    padded.extend_from_slice(&bits.to_be_bytes());
    for block in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (word, bytes) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_be_bytes(*bytes);
        }
        for t in 16..64 {
            let s0 = w[t - 15].rotate_right(7) ^ w[t - 15].rotate_right(18) ^ (w[t - 15] >> 3);
            let s1 = w[t - 2].rotate_right(17) ^ w[t - 2].rotate_right(19) ^ (w[t - 2] >> 10);
            w[t] = w[t - 16].wrapping_add(s0).wrapping_add(w[t - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for (&k, &wt) in SHA256_K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(k).wrapping_add(wt);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (bytes, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h.iter()) {
        *bytes = word.to_be_bytes();
    }
    out
}

const BASE58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// `text` decoded from base58, or None where a byte is outside the alphabet.
fn base58_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    for &b in text {
        let v = u32::try_from(BASE58.iter().position(|&c| c == b)?).expect("under fifty-eight");
        let mut carry = v;
        for byte in out.iter_mut().rev() {
            let acc = u32::from(*byte) * 58 + carry;
            *byte = (acc & 0xff) as u8;
            carry = acc >> 8;
        }
        while carry > 0 {
            out.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let zeros = text.iter().take_while(|&&b| b == b'1').count();
    let mut lead = vec![0u8; zeros];
    lead.extend(out);
    Some(lead)
}

/// A base58check string: twenty-five decoded bytes whose last four are the
/// first four of the double SHA-256 of the rest, the check a Bitcoin legacy
/// address carries.
pub(crate) fn base58check(text: &[u8]) -> bool {
    let Some(bytes) = base58_decode(text) else {
        return false;
    };
    if bytes.len() != 25 {
        return false;
    }
    let hash = sha256(&sha256(&bytes[..21]));
    hash[..4] == bytes[21..]
}

const BECH32_CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn bech32_polymod(values: impl Iterator<Item = u32>) -> u32 {
    const GEN: [u32; 5] = [0x3b6a_57b2, 0x2650_8e6d, 0x1ea1_19fa, 0x3d42_33dd, 0x2a14_62b3];
    let mut chk: u32 = 1;
    for v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ v;
        for (i, g) in GEN.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

/// A bech32 or bech32m string in one case: a human-readable part, the `1`
/// separator, data from the charset and a checksum that verifies under
/// either constant.
pub(crate) fn bech32(text: &[u8]) -> bool {
    if text.len() > 90 || text.iter().any(|b| !b.is_ascii_graphic()) {
        return false;
    }
    let has_upper = text.iter().any(u8::is_ascii_uppercase);
    let has_lower = text.iter().any(u8::is_ascii_lowercase);
    if has_upper && has_lower {
        return false;
    }
    let lower: Vec<u8> = text.iter().map(u8::to_ascii_lowercase).collect();
    let Some(sep) = lower.iter().rposition(|&b| b == b'1') else {
        return false;
    };
    if sep < 1 || sep + 7 > lower.len() {
        return false;
    }
    let (hrp, data) = (&lower[..sep], &lower[sep + 1..]);
    let mut values = Vec::with_capacity(data.len());
    for &b in data {
        let Some(v) = BECH32_CHARSET.iter().position(|&c| c == b) else {
            return false;
        };
        values.push(u32::try_from(v).expect("under thirty-two"));
    }
    let expanded = hrp
        .iter()
        .map(|&b| u32::from(b) >> 5)
        .chain(std::iter::once(0))
        .chain(hrp.iter().map(|&b| u32::from(b) & 31))
        .chain(values);
    let chk = bech32_polymod(expanded);
    chk == 1 || chk == 0x2bc8_30a3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn luhn_accepts_the_published_examples() {
        assert!(luhn(b"79927398713"));
        assert!(!luhn(b"79927398710"));
        assert!(luhn(b"490154203237518"), "the IMEI example");
        assert!(!luhn(b""));
        assert!(!luhn(b"12a4"));
    }

    #[test]
    fn iban_checks_the_registry_length_and_mod_97() {
        assert!(iban(b"GB82WEST12345698765432"));
        assert!(iban(b"GB82 WEST 1234 5698 7654 32"));
        assert!(iban(b"DE89370400440532013000"));
        assert!(!iban(b"GB82WEST12345698765433"), "one digit off");
        assert!(!iban(b"DE8937040044053201300"), "one short for Germany");
        assert!(!iban(b"ZZ82WEST12345698765432"), "no such country");
        assert!(!iban(b"GBA2WEST12345698765432"), "check digits must be digits");
    }

    #[test]
    fn gs1_covers_ean_13_upc_a_and_isbn_13() {
        assert!(gs1(b"4006381333931"));
        assert!(gs1(b"036000291452"));
        assert!(gs1(b"96385074"), "an EAN-8");
        assert!(!gs1(b"4006381333932"));
        assert!(isbn(b"9780306406157"));
        assert!(isbn(b"978-0-306-40615-7"));
        assert!(!isbn(b"9780306406158"));
        assert!(!isbn(b"9770306406157"), "not a book prefix");
    }

    #[test]
    fn isbn_10_checks_mod_11_with_x_as_ten() {
        assert!(isbn(b"0306406152"));
        assert!(isbn(b"0-306-40615-2"));
        assert!(isbn(b"080442957X"));
        assert!(!isbn(b"0306406153"));
        assert!(!isbn(b"0X06406152"), "X is only the check digit");
    }

    #[test]
    fn vin_checks_the_ninth_character() {
        assert!(vin(b"1HGCM82633A004352"));
        assert!(vin(b"11111111111111111"), "the all-ones VIN checks at 1");
        assert!(!vin(b"1HGCM82634A004352"));
        assert!(!vin(b"1HGCM82633A00435"), "sixteen characters");
        assert!(!vin(b"1HGCM82633I004352"), "I is not a VIN character");
    }

    #[test]
    fn isin_luhn_runs_over_the_expanded_letters() {
        assert!(isin(b"US0378331005"), "Apple");
        assert!(isin(b"US5949181045"), "Microsoft");
        assert!(isin(b"GB0002634946"), "BAE Systems");
        assert!(!isin(b"US0378331006"));
        assert!(!isin(b"US037833100"));
    }

    #[test]
    fn keccak_256_and_sha_256_match_their_published_vectors() {
        assert_eq!(hex(&keccak256(b"")), "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");
        assert_eq!(hex(&keccak256(b"abc")), "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(
            hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            "the two-block FIPS example"
        );
    }

    #[test]
    fn ethereum_addresses_check_eip_55_case() {
        assert!(ethereum_address(b"5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"));
        assert!(ethereum_address(b"fB6916095ca1df60bB79Ce92cE3Ea74c37c5d359"));
        assert!(ethereum_address(b"5aaeb6053f3e94c9b9a09f33669435e7ef1beaed"), "all lowercase carries no check");
        assert!(ethereum_address(b"5AAEB6053F3E94C9B9A09F33669435E7EF1BEAED"), "all uppercase carries no check");
        assert!(!ethereum_address(b"5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAeD"), "one letter's case flipped");
        assert!(!ethereum_address(b"5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAe"));
    }

    #[test]
    fn bitcoin_addresses_check_base58check_and_bech32() {
        assert!(base58check(b"1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2"));
        assert!(base58check(b"3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"));
        assert!(!base58check(b"1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN3"));
        assert!(!base58check(b"1BvBMSEYstWetqTFn5Au4m4GFg7xJaNV0N"), "0 is outside base58");
        assert!(bech32(b"bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"));
        assert!(bech32(b"BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4"));
        assert!(bech32(b"bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0"), "a bech32m taproot address");
        assert!(!bech32(b"bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdx"));
        assert!(!bech32(b"bc1Qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"), "mixed case");
    }
}
