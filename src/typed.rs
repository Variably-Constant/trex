//! Typed value predicates: a kind atom's `{...}` body compared in the type's
//! own units.
//!
//! `\I{in:10.0.0.0/8}` is CIDR containment, `\V{>=2.0,<3}` semver precedence,
//! `\Z{>1GiB}` a byte count with the unit normalized, `\T{age<24h}` a
//! duration against the clock. The lexer has already recognized each token's
//! shape, so a predicate reads the token's bytes as that shape's value and
//! compares there; no clause compares text where the type has a value.
//!
//! A predicate is a conjunction of clauses, `{clause, clause}`. A clause is
//! `[field] op value`: `> >= < <= = !=` on an ordered value, `field:value` on
//! a text field with `*` and `?` globs, `a..b` an inclusive range, and the
//! containment forms `in:` and `contains:`. The field a kind reads by default
//! is its own value; a kind with no single value (a URL, an email) names a
//! field every time. A value never holds a comma, since commas separate the
//! clauses.
//!
//! Numbers are compared exactly at any length ([`Decimal`] is a digit string,
//! not a machine word), so a fifty-digit token compares as written rather than
//! being refused or rounded. Every reader here answers `None` for bytes that
//! are not the shape it reads; a token that does not read in the clause's
//! type does not satisfy the clause.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering as Atomic};

use crate::token::TokenKind;

/// The clock a scan reads: the instant `now`, and the offset given to a
/// timestamp written with no zone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    /// Seconds since the Unix epoch.
    pub now_secs: i64,
    /// The offset, in seconds east of UTC, given to a timestamp with no zone.
    pub tz_offset: i32,
}

/// `i64::MIN` means no override is set.
static NOW_OVERRIDE: AtomicI64 = AtomicI64::new(i64::MIN);
static TZ_OFFSET: AtomicI32 = AtomicI32::new(0);
static DAY_FIRST: AtomicBool = AtomicBool::new(true);

impl Clock {
    /// The clock a scan beginning now reads: the override where one is set,
    /// else the system clock, with the process's zone offset.
    #[must_use]
    pub fn current() -> Clock {
        let over = NOW_OVERRIDE.load(Atomic::Relaxed);
        let now_secs = if over == i64::MIN { system_now_secs() } else { over };
        Clock { now_secs, tz_offset: TZ_OFFSET.load(Atomic::Relaxed) }
    }

    /// A fixed clock, for a test or a replay.
    #[must_use]
    pub fn fixed(now_secs: i64, tz_offset: i32) -> Clock {
        Clock { now_secs, tz_offset }
    }
}

/// The system clock in seconds since the epoch, clamped to the seconds an
/// `i64` holds.
fn system_now_secs() -> i64 {
    let ceiling = i64::MAX as u64;
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(after) => after.as_secs().min(ceiling) as i64,
        Err(before) => -(before.duration().as_secs().min(ceiling) as i64),
    }
}

/// Fix `now` for every scan that begins after this call, in seconds since the
/// epoch; `None` returns to the system clock.
pub fn set_now(secs: Option<i64>) {
    NOW_OVERRIDE.store(secs.unwrap_or(i64::MIN), Atomic::Relaxed);
}

/// The zone offset, in seconds east of UTC, given to a timestamp written
/// with no zone. Zero, UTC, until set.
pub fn set_tz_offset(secs: i32) {
    TZ_OFFSET.store(secs, Atomic::Relaxed);
}

/// Which of the two leading fields of an all-numeric slash date is the day,
/// where both readings are valid dates. Day first until set: trex's other
/// slash date, Apache's `15/Sep/2026`, is day first, so the default follows
/// the tool's own convention rather than a region's.
pub fn set_date_order_day_first(day_first: bool) {
    DAY_FIRST.store(day_first, Atomic::Relaxed);
}

/// The instant [`set_now`] fixed, in seconds since the epoch, or `None` while
/// scans read the system clock.
#[must_use]
pub fn now_override() -> Option<i64> {
    match NOW_OVERRIDE.load(Atomic::Relaxed) {
        i64::MIN => None,
        secs => Some(secs),
    }
}

/// Whether an all-numeric slash date is read day first, as
/// [`set_date_order_day_first`] last set it.
#[must_use]
pub fn date_order_day_first() -> bool {
    DAY_FIRST.load(Atomic::Relaxed)
}

/// Parse a `--date-order` argument, `dmy` or `mdy`, as whether a slash date
/// is read day first.
#[must_use]
pub fn parse_date_order_arg(s: &str) -> Option<bool> {
    match s.trim() {
        "dmy" => Some(true),
        "mdy" => Some(false),
        _ => None,
    }
}

/// The clock a compiled program holds: read when the program is built and
/// refreshed at each scan the program is reused for, so every atom of one
/// scan reads the same instant.
pub(crate) struct ClockCell {
    now: AtomicI64,
    tz: AtomicI32,
}

impl ClockCell {
    pub(crate) fn now() -> Self {
        let c = Clock::current();
        ClockCell { now: AtomicI64::new(c.now_secs), tz: AtomicI32::new(c.tz_offset) }
    }

    pub(crate) fn refresh(&self) {
        let c = Clock::current();
        self.now.store(c.now_secs, Atomic::Relaxed);
        self.tz.store(c.tz_offset, Atomic::Relaxed);
    }

    pub(crate) fn get(&self) -> Clock {
        Clock { now_secs: self.now.load(Atomic::Relaxed), tz_offset: self.tz.load(Atomic::Relaxed) }
    }
}

impl Clone for ClockCell {
    fn clone(&self) -> Self {
        let c = self.get();
        ClockCell { now: AtomicI64::new(c.now_secs), tz: AtomicI32::new(c.tz_offset) }
    }
}

impl std::fmt::Debug for ClockCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.get())
    }
}

/// A signed run of ASCII digits as an integer, or `None` for any other text
/// or a value past what an `i64` holds.
fn parse_int(s: &str) -> Option<i64> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut v: i64 = 0;
    for b in digits.bytes() {
        v = v.checked_mul(10)?.checked_add(i64::from(b - b'0'))?;
    }
    Some(if neg { -v } else { v })
}

/// Parse a `--now` argument: a timestamp with a date in any form the lexer
/// recognizes, or a bare count of seconds since the epoch.
#[must_use]
pub fn parse_now_arg(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Some(secs) = parse_int(s) {
        return Some(secs);
    }
    let civil = parse_civil(s)?;
    civil.epoch(Clock::fixed(0, TZ_OFFSET.load(Atomic::Relaxed))).map(|(secs, _)| secs)
}

/// Parse a `--tz` argument, `Z`, `+02:00`, `-0530` or `+05`, to seconds east
/// of UTC.
#[must_use]
pub fn parse_tz_arg(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("z") || s.eq_ignore_ascii_case("utc") {
        return Some(0);
    }
    let (zone, end) = parse_zone(s.as_bytes(), 0)?;
    (end == s.len()).then_some(zone)
}

/// An exact decimal of any size: the sign, the significant digits with no
/// leading or trailing zero, and the power of ten they are scaled by, so the
/// value is the digits times ten to `exp` and zero has no digits. Comparison,
/// hashing and scaling cost the digits written however large the power, so
/// `1e999999` is one digit and an exponent; only arithmetic that lines two
/// values up writes the zeros between them.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Decimal {
    neg: bool,
    digits: String,
    exp: i64,
}

/// The most zeros a value's text writes between its digits and its point:
/// `1` and 21 zeros is written out, and `1` and 22 zeros is `1e22`.
const PLAIN_ZEROS: i128 = 21;

impl Decimal {
    /// `-12.50` reads as `-12.5`; `.5` and `5.` are accepted. Commas are not
    /// digits; a caller strips them where the type writes them.
    #[must_use]
    pub fn parse(s: &str) -> Option<Decimal> {
        let s = s.trim();
        let (neg, body) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        if int.is_empty() && frac.is_empty() {
            return None;
        }
        if !int.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(Decimal::normalized(neg, int, frac))
    }

    fn normalized(neg: bool, int: &str, frac: &str) -> Decimal {
        Decimal::scaled(neg, &format!("{int}{frac}"), -(frac.len() as i64))
    }

    /// The value `digits` times ten to `exp`, its leading zeros dropped and
    /// its trailing ones moved into the power.
    fn scaled(neg: bool, digits: &str, exp: i64) -> Decimal {
        let lead = digits.trim_start_matches('0');
        let significant = lead.trim_end_matches('0');
        if significant.is_empty() {
            return Decimal { neg: false, digits: String::new(), exp: 0 };
        }
        let moved = (lead.len() - significant.len()) as i64;
        Decimal { neg, digits: significant.to_string(), exp: exp + moved }
    }

    #[must_use]
    pub fn from_u128(v: u128) -> Decimal {
        Decimal::scaled(false, &v.to_string(), 0)
    }

    #[must_use]
    pub fn from_i64(v: i64) -> Decimal {
        Decimal::scaled(v < 0, &v.unsigned_abs().to_string(), 0)
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// The power of ten of the leading digit, or `None` for zero.
    fn lead_power(&self) -> Option<i128> {
        (!self.is_zero()).then(|| self.digits.len() as i128 - 1 + i128::from(self.exp))
    }

    /// The magnitudes compared, signs aside: the leading digit's power, then
    /// the digits, which end on no zero, so digits that are a prefix of
    /// another value's are the smaller.
    fn cmp_magnitude(a: &Decimal, b: &Decimal) -> Ordering {
        match (a.lead_power(), b.lead_power()) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.digits.as_bytes().cmp(b.digits.as_bytes())),
        }
    }

    /// Numeric order, exact.
    #[must_use]
    pub fn compare(&self, other: &Decimal) -> Ordering {
        match (self.neg, other.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => Self::cmp_magnitude(self, other),
            (true, true) => Self::cmp_magnitude(other, self),
        }
    }

    /// This value times an integer, exact. `by` is at most a sixty-four-bit
    /// value: the largest unit scaled here is a year of nanoseconds.
    #[must_use]
    pub fn times(&self, by: u128) -> Decimal {
        debug_assert!(by < 1u128 << 64, "a unit scale fits sixty-four bits");
        let mut out: Vec<u8> = Vec::with_capacity(self.digits.len() + 40);
        let mut carry: u128 = 0;
        for d in self.digits.bytes().rev() {
            let v = u128::from(d - b'0') * by + carry;
            out.push((v % 10) as u8);
            carry = v / 10;
        }
        while carry > 0 {
            out.push((carry % 10) as u8);
            carry /= 10;
        }
        out.reverse();
        let s: String = out.iter().map(|d| char::from(b'0' + d)).collect();
        Decimal::scaled(self.neg, &s, self.exp)
    }

    /// The value as text: written out where that puts at most
    /// [`PLAIN_ZEROS`] zeros between the digits and the point, and otherwise
    /// as the digits and the leading digit's power of ten, `1e999999` or
    /// `-2.5e-30`, so the text is as long as the value's digits.
    #[must_use]
    pub fn to_text(&self) -> String {
        self.written(PLAIN_ZEROS)
    }

    /// The value written out in digits, or `None` where that text would be
    /// longer than `longest` bytes. For a caller reading the text into a
    /// type of fixed width, which a value written longer cannot fit, so a
    /// huge value is never written out only to find it too long.
    #[must_use]
    pub fn plain_text(&self, longest: usize) -> Option<String> {
        let n = self.digits.len() as i128;
        let exp = i128::from(self.exp);
        let body = if exp >= 0 {
            n + exp
        } else if n + exp > 0 {
            n + 1
        } else {
            2 - exp
        };
        (i128::from(self.neg) + body <= longest as i128).then(|| self.written(i128::MAX))
    }

    /// The value as text, written out where that puts at most `plain_zeros`
    /// zeros between the digits and the point, and otherwise as the digits
    /// and the leading digit's power of ten.
    fn written(&self, plain_zeros: i128) -> String {
        let Some(lead) = self.lead_power() else {
            return "0".to_string();
        };
        let sign = if self.neg { "-" } else { "" };
        let exp = i128::from(self.exp);
        if (0..=plain_zeros).contains(&exp) {
            return format!("{sign}{}{}", self.digits, "0".repeat(exp as usize));
        }
        if exp < 0 {
            // How many digits come before the point; none do where this is
            // zero or less.
            let before = self.digits.len() as i128 + exp;
            if before > 0 {
                let (int, frac) = self.digits.split_at(before as usize);
                return format!("{sign}{int}.{frac}");
            }
            if -before <= plain_zeros {
                return format!("{sign}0.{}{}", "0".repeat((-before) as usize), self.digits);
            }
        }
        let (first, rest) = self.digits.split_at(1);
        if rest.is_empty() {
            format!("{sign}{first}e{lead}")
        } else {
            format!("{sign}{first}.{rest}e{lead}")
        }
    }

    /// The same magnitude with the opposite sign; zero stays zero.
    #[must_use]
    pub fn negated(&self) -> Decimal {
        Decimal { neg: !self.neg && !self.is_zero(), digits: self.digits.clone(), exp: self.exp }
    }

    /// The sum of two decimals of either sign, exact, or `None` where they
    /// are more than [`ALIGN_PLACES`] places apart, since the sum would write
    /// that many zeros that neither value has.
    #[must_use]
    pub fn add(&self, other: &Decimal) -> Option<Decimal> {
        Some(match (self.neg, other.neg) {
            (false, false) => add_positive(self, other)?,
            (true, true) => {
                let mut sum = add_positive(self, other)?;
                sum.neg = !sum.is_zero();
                sum
            }
            _ => {
                let (plus, minus) = if self.neg { (other, self) } else { (self, other) };
                match Decimal::cmp_magnitude(plus, minus) {
                    Ordering::Equal => Decimal::from_u128(0),
                    Ordering::Greater => sub_magnitude(plus, minus)?,
                    Ordering::Less => {
                        let mut d = sub_magnitude(minus, plus)?;
                        d.neg = !d.is_zero();
                        d
                    }
                }
            }
        })
    }
}

/// The most places apart two values may be for exact arithmetic to line them
/// up. Values further apart would be summed by writing more zeros than
/// either has, a megabyte past this, so their sum is not formed and an
/// aggregate asking for it reports no value.
const ALIGN_PLACES: i64 = 1_000_000;

/// Both values' digits written over the smaller of their two powers of ten,
/// and that power, so they add and subtract a place at a time; `None` where
/// the powers are more than [`ALIGN_PLACES`] apart. The zeros between a
/// value's digits and that power are written out, which is what exact
/// arithmetic over two values far apart in size costs.
fn aligned(a: &Decimal, b: &Decimal) -> Option<(Vec<u8>, Vec<u8>, i64)> {
    let exp = a.exp.min(b.exp);
    if a.exp.max(b.exp) - exp > ALIGN_PLACES {
        return None;
    }
    let place = |d: &Decimal| -> Vec<u8> {
        let mut v: Vec<u8> = d.digits.bytes().map(|c| c - b'0').collect();
        v.extend(std::iter::repeat_n(0, (d.exp - exp) as usize));
        v
    };
    Some((place(a), place(b), exp))
}

/// The magnitude of `a` less the magnitude of `b`, where `a` is at least as
/// large in magnitude, exact and non-negative; `None` as [`aligned`] gives.
fn sub_magnitude(a: &Decimal, b: &Decimal) -> Option<Decimal> {
    if b.is_zero() {
        return Some(Decimal { neg: false, ..a.clone() });
    }
    let (x, y, exp) = aligned(a, b)?;
    let mut out = vec![0u8; x.len()];
    let mut borrow = 0u8;
    let (mut i, mut j) = (x.len(), y.len());
    while i > 0 {
        i -= 1;
        let q = if j > 0 {
            j -= 1;
            y[j]
        } else {
            0
        };
        let p = x[i];
        if p >= q + borrow {
            out[i] = p - q - borrow;
            borrow = 0;
        } else {
            out[i] = p + 10 - q - borrow;
            borrow = 1;
        }
    }
    let s: String = out.iter().map(|d| char::from(b'0' + d)).collect();
    Some(Decimal::scaled(false, &s, exp))
}

/// A timestamp as written: its calendar fields, with the year, the date, the
/// time and the zone each possibly absent, placed on the clock only when a
/// comparison needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Civil {
    /// The year, or `None` where the form writes none (syslog).
    pub year: Option<i32>,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanos: u32,
    pub has_date: bool,
    pub has_time: bool,
    /// Seconds east of UTC as written; `None` for a timestamp with no zone.
    pub zone: Option<i32>,
}

impl Civil {
    /// Seconds since the epoch and the nanosecond part, placed with `clock`: a
    /// timestamp with no zone is read in the clock's offset, and one with no
    /// year in the clock's year, or the year before where that would put it
    /// more than a day past `now`. `None` for a time with no date.
    #[must_use]
    pub fn epoch(&self, clock: Clock) -> Option<(i64, u32)> {
        if !self.has_date {
            return None;
        }
        let offset = i64::from(self.zone.unwrap_or(clock.tz_offset));
        let time =
            i64::from(self.hour) * 3600 + i64::from(self.minute) * 60 + i64::from(self.second);
        let place = |year: i64| -> Option<i64> {
            let days = days_from_civil(year, u32::from(self.month), u32::from(self.day))?;
            Some(days * 86_400 + time - offset)
        };
        let secs = match self.year {
            Some(y) => place(i64::from(y))?,
            None => {
                let (this_year, _, _) = civil_from_days(clock.now_secs.div_euclid(86_400));
                match place(this_year) {
                    Some(s) if s > clock.now_secs + 86_400 => place(this_year - 1)?,
                    Some(s) => s,
                    None => place(this_year - 1)?,
                }
            }
        };
        Some((secs, self.nanos))
    }

    /// Seconds into the day, for a comparison between two times with no date.
    #[must_use]
    pub fn seconds_of_day(&self) -> Option<(i64, u32)> {
        if !self.has_time {
            return None;
        }
        let offset = i64::from(self.zone.unwrap_or(0));
        let s = i64::from(self.hour) * 3600 + i64::from(self.minute) * 60 + i64::from(self.second);
        Some((s - offset, self.nanos))
    }
}

/// Whether the date exists in the calendar; with no year, February keeps its
/// leap day.
fn valid_date(year: Option<i32>, month: u8, day: u8) -> bool {
    let m = u32::from(month);
    let limit = match year {
        Some(y) => days_in_month(i64::from(y), m),
        None if m == 2 => 29,
        None => days_in_month(2001, m),
    };
    (1..=12).contains(&m) && day >= 1 && u32::from(day) <= limit
}

/// Days since 1970-01-01 of a calendar date, or `None` for a date that does
/// not exist.
fn days_from_civil(y: i64, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// The calendar date of a count of days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// The month a three-letter English abbreviation names, any case.
#[must_use]
pub fn month_abbrev(s: &[u8]) -> Option<u8> {
    const NAMES: [&[u8; 3]; 12] = [
        b"jan", b"feb", b"mar", b"apr", b"may", b"jun", b"jul", b"aug", b"sep", b"oct", b"nov",
        b"dec",
    ];
    if s.len() < 3 {
        return None;
    }
    let low = [s[0] | 0x20, s[1] | 0x20, s[2] | 0x20];
    NAMES.iter().position(|n| **n == low).map(|i| i as u8 + 1)
}

/// One or two ASCII digits at `i` as a number, with the offset past them.
fn small_digits(s: &[u8], i: usize) -> Option<(u32, usize)> {
    let mut j = i;
    let mut v = 0u32;
    while j < s.len() && s[j].is_ascii_digit() && j - i < 2 {
        v = v * 10 + u32::from(s[j] - b'0');
        j += 1;
    }
    (j > i).then_some((v, j))
}

/// A slash-separated date read out of the text: its calendar fields, the
/// offset just past it, and which of the three digit runs holds each field.
pub(crate) struct SlashDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    /// Byte offset just past the date.
    pub end: usize,
    /// The date's three digit runs, in the order the text writes them, as
    /// `(year, month, day)` indices. `2026/09/15` is `(0, 1, 2)` and
    /// `15/09/2026` is `(2, 1, 0)`, so a caller that must point at the bytes
    /// of one field does not decide the order a second time.
    pub runs: (usize, usize, usize),
}

/// A slash-separated date at `i`, in every order logs write.
///
/// `YYYY/MM/DD` is the slash spelling of ISO 8601 and is told by its shape
/// alone. In the other two the year comes last, and the year's four digits
/// are the marker that tells a date from a run of figures: without them
/// `1/2/3` would be a date, and so would half the fractions in a document.
/// Which of the two leading fields is the day is decided by which reading is
/// a valid date - `15/09/2026` has no fifteenth month - and where both are,
/// as in `03/04/2026`, by [`set_date_order_day_first`].
///
/// Both readings span the same bytes, so a caller that wants only the extent
/// gets an answer that no setting can move.
pub(crate) fn slash_date(s: &[u8], i: usize) -> Option<SlashDate> {
    if let Some((year, j)) = fixed_digits(s, i, 4)
        && s.get(j) == Some(&b'/')
        && let Some((month, j)) = fixed_digits(s, j + 1, 2)
        && s.get(j) == Some(&b'/')
        && let Some((day, j)) = fixed_digits(s, j + 1, 2)
        && !s.get(j).is_some_and(u8::is_ascii_digit)
        && valid_date(Some(year as i32), month as u8, day as u8)
    {
        return Some(SlashDate {
            year: year as i32,
            month: month as u8,
            day: day as u8,
            end: j,
            runs: (0, 1, 2),
        });
    }
    let (first, j) = small_digits(s, i)?;
    if s.get(j) != Some(&b'/') {
        return None;
    }
    let (second, j) = small_digits(s, j + 1)?;
    if s.get(j) != Some(&b'/') {
        return None;
    }
    let (year, j) = fixed_digits(s, j + 1, 4)?;
    if s.get(j).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let year = year as i32;
    let (first, second) = (first as u8, second as u8);
    let day_leads = valid_date(Some(year), second, first);
    let month_leads = valid_date(Some(year), first, second);
    let day_first = match (day_leads, month_leads) {
        (true, true) => DAY_FIRST.load(Atomic::Relaxed),
        (true, false) => true,
        (false, true) => false,
        (false, false) => return None,
    };
    let (month, day) = if day_first { (second, first) } else { (first, second) };
    let runs = if day_first { (2, 1, 0) } else { (2, 0, 1) };
    Some(SlashDate { year, month, day, end: j, runs })
}

/// Read `count` ASCII digits at `i` as a number, with the offset past them.
fn fixed_digits(s: &[u8], i: usize, count: usize) -> Option<(u32, usize)> {
    if i + count > s.len() {
        return None;
    }
    let mut v = 0u32;
    for &b in &s[i..i + count] {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v * 10 + u32::from(b - b'0');
    }
    Some((v, i + count))
}

/// A clock time at `i`: `HH:MM`, `HH:MM:SS`, `HH:MM:SS.frac`, then an optional
/// zone. Returns the fields and the offset past what was read.
fn parse_time(s: &[u8], i: usize) -> Option<(u8, u8, u8, u32, Option<i32>, usize)> {
    let (hour, j) = fixed_digits(s, i, 2)?;
    if s.get(j) != Some(&b':') {
        return None;
    }
    let (minute, mut j) = fixed_digits(s, j + 1, 2)?;
    let mut second = 0;
    let mut nanos = 0u32;
    if s.get(j) == Some(&b':')
        && let Some((sec, k)) = fixed_digits(s, j + 1, 2)
    {
        second = sec;
        j = k;
        if s.get(j) == Some(&b'.') && s.get(j + 1).is_some_and(u8::is_ascii_digit) {
            let mut k = j + 1;
            let mut scale = 100_000_000u32;
            while k < s.len() && s[k].is_ascii_digit() {
                if scale > 0 {
                    nanos += u32::from(s[k] - b'0') * scale;
                    scale /= 10;
                }
                k += 1;
            }
            j = k;
        }
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let (zone, j) = match parse_zone(s, j) {
        Some((z, k)) => (Some(z), k),
        None => (None, j),
    };
    Some((hour as u8, minute as u8, second as u8, nanos, zone, j))
}

/// A zone at `i`: `Z`, `z`, `+HH:MM`, `-HHMM` or `+HH`, as seconds east of
/// UTC, with the offset past it. A single space before a four-digit numeric
/// zone is the Apache form.
fn parse_zone(s: &[u8], i: usize) -> Option<(i32, usize)> {
    match s.get(i) {
        Some(b'Z' | b'z') => Some((0, i + 1)),
        Some(b'+' | b'-') => {
            let sign = if s[i] == b'-' { -1 } else { 1 };
            let (hh, j) = fixed_digits(s, i + 1, 2)?;
            let (mm, j) = if s.get(j) == Some(&b':') {
                fixed_digits(s, j + 1, 2)?
            } else if let Some(r) = fixed_digits(s, j, 2) {
                r
            } else {
                (0, j)
            };
            if hh > 23 || mm > 59 {
                return None;
            }
            Some((sign * (hh as i32 * 3600 + mm as i32 * 60), j))
        }
        Some(b' ') if matches!(s.get(i + 1), Some(b'+' | b'-')) => {
            let (z, j) = parse_zone(s, i + 1)?;
            (j == i + 6).then_some((z, j))
        }
        _ => None,
    }
}

/// Every timestamp form the lexer recognizes, read as calendar fields:
/// `YYYY-MM-DD`, the slash dates [`slash_date`] reads,
/// `HH:MM[:SS[.frac]][zone]`, a date and a clock joined by `T` or a space,
/// syslog `Mon DD HH:MM:SS`, and Apache `DD/Mon/YYYY:HH:MM:SS[ +hhmm]`.
#[must_use]
pub fn parse_civil(text: &str) -> Option<Civil> {
    let s = text.as_bytes();
    let n = s.len();
    let mut civil = Civil {
        year: None,
        month: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        nanos: 0,
        has_date: false,
        has_time: false,
        zone: None,
    };
    let mut j = 0usize;
    if let Some(month) = month_abbrev(s).filter(|_| s.get(3) == Some(&b' ')) {
        // syslog: the month, one or two spaces, the day, a space, the clock.
        j = 4;
        if s.get(j) == Some(&b' ') {
            j += 1;
        }
        let (day, k) = match fixed_digits(s, j, 2) {
            Some(r) if s.get(r.1) == Some(&b' ') => r,
            _ => fixed_digits(s, j, 1)?,
        };
        if s.get(k) != Some(&b' ') {
            return None;
        }
        let (hour, minute, second, nanos, zone, k) = parse_time(s, k + 1)?;
        if k != n || !valid_date(None, month, day as u8) {
            return None;
        }
        civil.month = month;
        civil.day = day as u8;
        civil.has_date = true;
        civil.has_time = true;
        civil.hour = hour;
        civil.minute = minute;
        civil.second = second;
        civil.nanos = nanos;
        civil.zone = zone;
        return Some(civil);
    }
    if let Some((day, k)) = fixed_digits(s, 0, 2).filter(|(_, k)| s.get(*k) == Some(&b'/'))
        && let Some(month) = month_abbrev(&s[k + 1..]).filter(|_| s.get(k + 4) == Some(&b'/'))
    {
        // Apache: DD/Mon/YYYY:HH:MM:SS, an optional space and zone.
        let (year, k) = fixed_digits(s, k + 5, 4)?;
        if s.get(k) != Some(&b':') {
            return None;
        }
        let (hour, minute, second, nanos, zone, k) = parse_time(s, k + 1)?;
        if k != n || !valid_date(Some(year as i32), month, day as u8) {
            return None;
        }
        civil.year = Some(year as i32);
        civil.month = month;
        civil.day = day as u8;
        civil.has_date = true;
        civil.has_time = true;
        civil.hour = hour;
        civil.minute = minute;
        civil.second = second;
        civil.nanos = nanos;
        civil.zone = zone;
        return Some(civil);
    }
    let dashed = fixed_digits(s, 0, 4).filter(|(_, k)| s.get(*k) == Some(&b'-')).and_then(|(year, k)| {
        let (month, k) = fixed_digits(s, k + 1, 2)?;
        if s.get(k) != Some(&b'-') {
            return None;
        }
        let (day, k) = fixed_digits(s, k + 1, 2)?;
        valid_date(Some(year as i32), month as u8, day as u8)
            .then_some((year as i32, month as u8, day as u8, k))
    });
    let slashed = slash_date(s, 0).map(|d| (d.year, d.month, d.day, d.end));
    if let Some((year, month, day, k)) = dashed.or(slashed) {
        civil.year = Some(year);
        civil.month = month;
        civil.day = day;
        civil.has_date = true;
        j = k;
        if j == n {
            return Some(civil);
        }
        if !matches!(s.get(j), Some(b'T' | b't' | b' ')) {
            return None;
        }
        j += 1;
    }
    let (hour, minute, second, nanos, zone, k) = parse_time(s, j)?;
    if k != n {
        return None;
    }
    civil.has_time = true;
    civil.hour = hour;
    civil.minute = minute;
    civil.second = second;
    civil.nanos = nanos;
    civil.zone = zone;
    Some(civil)
}

/// A version as semver orders it: the numeric parts, and the pre-release
/// identifiers where there are any. Build metadata is not part of the order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    parts: Vec<Decimal>,
    pre: Option<Vec<String>>,
}

impl Version {
    /// `v1.2.3-beta.2+build.7`, `2.0`, `3`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = match s.strip_prefix(['v', 'V']) {
            Some(r) if r.starts_with(|c: char| c.is_ascii_digit()) => r,
            _ => s,
        };
        let s = match s.split_once('+') {
            Some((core, _build)) => core,
            None => s,
        };
        let (core, pre) = match s.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (s, None),
        };
        let mut parts = Vec::new();
        for p in core.split('.') {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            parts.push(Decimal::parse(p)?);
        }
        let pre = match pre {
            Some(p) => {
                if p.is_empty() {
                    return None;
                }
                Some(p.split('.').map(str::to_string).collect())
            }
            None => None,
        };
        Some(Version { parts, pre })
    }

    /// The i-th numeric part, zero where the version writes none.
    #[must_use]
    pub fn part(&self, i: usize) -> Decimal {
        match self.parts.get(i) {
            Some(p) => p.clone(),
            None => Decimal::from_u128(0),
        }
    }

    /// The pre-release identifiers joined by `.`, empty for a release.
    #[must_use]
    pub fn pre_text(&self) -> String {
        match &self.pre {
            Some(p) => p.join("."),
            None => String::new(),
        }
    }

    /// The numeric parts in written order, which decide semver precedence
    /// before the pre-release identifiers are looked at.
    #[must_use]
    pub fn parts(&self) -> &[Decimal] {
        &self.parts
    }

    /// The pre-release identifiers, absent for a release. A release outranks
    /// every pre-release of the same parts, so absent is not the same as an
    /// empty list.
    #[must_use]
    pub fn pre(&self) -> Option<&[String]> {
        self.pre.as_deref()
    }

    /// The version as semver orders it: the numeric parts joined by dots and
    /// the pre-release identifiers after a dash. Build metadata is absent,
    /// because it takes no part in the order and the parser drops it.
    #[must_use]
    pub fn to_text(&self) -> String {
        let core = self.parts.iter().map(Decimal::to_text).collect::<Vec<_>>().join(".");
        match &self.pre {
            Some(p) if !p.is_empty() => format!("{core}-{}", p.join(".")),
            _ => core,
        }
    }

    /// The version as the two lists that decide its order, so a consumer
    /// compares without parsing the text again. `pre` is absent for a
    /// release, which outranks every pre-release of the same parts.
    #[must_use]
    pub fn json(&self) -> String {
        let parts =
            self.parts.iter().map(Decimal::to_text).collect::<Vec<_>>().join(", ");
        match &self.pre {
            Some(p) => {
                let pre =
                    p.iter().map(|s| json_string(s)).collect::<Vec<_>>().join(", ");
                format!("{{\"parts\": [{parts}], \"pre\": [{pre}]}}")
            }
            None => format!("{{\"parts\": [{parts}]}}"),
        }
    }

    /// Semver precedence: numeric parts in order (a missing part is zero), a
    /// release above any of its pre-releases, pre-release identifiers
    /// numeric before alphanumeric, numbers by value, the rest by bytes, and
    /// a longer list above its prefix.
    #[must_use]
    pub fn compare(&self, other: &Version) -> Ordering {
        let n = self.parts.len().max(other.parts.len());
        for i in 0..n {
            match self.part(i).compare(&other.part(i)) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => {
                for (x, y) in a.iter().zip(b) {
                    let o = match (Decimal::parse(x), Decimal::parse(y)) {
                        (Some(p), Some(q)) => p.compare(&q),
                        (Some(_), None) => Ordering::Less,
                        (None, Some(_)) => Ordering::Greater,
                        (None, None) => x.as_bytes().cmp(y.as_bytes()),
                    };
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                a.len().cmp(&b.len())
            }
        }
    }
}

/// Whether `text` matches a glob: `*` any run of characters, `?` any one,
/// every other character itself, with ASCII case folded when `fold` is set.
#[must_use]
pub fn glob_match(pat: &str, text: &str, fold: bool) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let same = |a: char, b: char| if fold { a.eq_ignore_ascii_case(&b) } else { a == b };
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || same(p[pi], t[ti])) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// A dotted-quad IPv4 address, each octet in `0..=255`.
fn parse_ipv4(s: &str) -> Option<Ipv4Addr> {
    let mut octets = [0u8; 4];
    let mut n = 0;
    for part in s.split('.') {
        if n == 4 || part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let v = part.bytes().fold(0u32, |a, b| a * 10 + u32::from(b - b'0'));
        if v > 255 {
            return None;
        }
        octets[n] = v as u8;
        n += 1;
    }
    (n == 4).then_some(Ipv4Addr::from(octets))
}

/// An IPv6 address: eight hex groups of at most four digits, or fewer with one
/// `::` standing for the zero groups between.
fn parse_ipv6(s: &str) -> Option<Ipv6Addr> {
    let group = |g: &str| -> Option<u16> {
        if g.is_empty() || g.len() > 4 {
            return None;
        }
        let mut v = 0u16;
        for b in g.bytes() {
            let digit = (b as char).to_digit(16)?;
            v = (v << 4) | digit as u16;
        }
        Some(v)
    };
    let mut segs = [0u16; 8];
    match s.split_once("::") {
        Some((left, right)) => {
            if right.contains("::") {
                return None;
            }
            let l: Vec<&str> = if left.is_empty() { Vec::new() } else { left.split(':').collect() };
            let r: Vec<&str> = if right.is_empty() { Vec::new() } else { right.split(':').collect() };
            if l.len() + r.len() > 7 {
                return None;
            }
            for (i, g) in l.iter().enumerate() {
                segs[i] = group(g)?;
            }
            for (i, g) in r.iter().enumerate() {
                segs[8 - r.len() + i] = group(g)?;
            }
        }
        None => {
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() != 8 {
                return None;
            }
            for (i, g) in parts.iter().enumerate() {
                segs[i] = group(g)?;
            }
        }
    }
    Some(Ipv6Addr::from(segs))
}

/// An IP address of either family.
#[must_use]
pub fn parse_ip(s: &str) -> Option<IpAddr> {
    let s = s.trim();
    if s.contains(':') { parse_ipv6(s).map(IpAddr::V6) } else { parse_ipv4(s).map(IpAddr::V4) }
}

/// A field of a typed token that a clause reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// The token's own value: the number, the instant, the address, the
    /// version, the byte count, the duration, the amount.
    Value,
    /// A word's or a quoted string's length in characters.
    Len,
    /// How long before `now` a timestamp is.
    Age,
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Scheme,
    Host,
    Port,
    Path,
    Query,
    User,
    Domain,
    Major,
    Minor,
    Patch,
    /// A version's pre-release identifiers.
    Pre,
    /// A card's issuer, from its issuer identification number.
    Issuer,
    /// A card's last four digits.
    Last4,
    /// A phone number's country calling code.
    Cc,
    /// A CIDR block's prefix length.
    Prefix,
    /// A UUID's version digit.
    UuidVersion,
    /// A MAC address's first three octets.
    Oui,
    /// A hash digest's algorithm, by length.
    Algo,
    Dir,
    Name,
    Ext,
    Lat,
    Long,
    /// An address is inside a block (`in:`).
    In,
    /// A block holds an address (`contains:`).
    Contains,
    /// An address family, `v4` or `v6`; a quantity's family, `mass`,
    /// `length`, `temperature` and the rest of [`crate::quantity::Family`].
    Family,
    /// A quantity's unit symbol as written.
    Unit,
    /// A named parameter of a JSON Web Token's decoded header; the name is
    /// the clause's own key, empty for the header's whole text.
    Header,
    /// A named claim of a decoded payload; the name is the clause's own key,
    /// empty for the payload's whole text.
    Claim,
    /// The Shannon entropy of decoded content, in bits per byte.
    Bits,
    /// The texture class of decoded content.
    Texture,
    /// The dominant byte-period of decoded content.
    Period,
    /// Decoded content as text.
    Text,
    /// A declared sub-pattern that must match somewhere in decoded content.
    Match,
}

impl Field {
    fn name(self) -> &'static str {
        match self {
            Field::Value => "value",
            Field::Len => "len",
            Field::Age => "age",
            Field::Year => "year",
            Field::Month => "month",
            Field::Day => "day",
            Field::Hour => "hour",
            Field::Minute => "minute",
            Field::Second => "second",
            Field::Scheme => "scheme",
            Field::Host => "host",
            Field::Port => "port",
            Field::Path => "path",
            Field::Query => "query",
            Field::User => "user",
            Field::Domain => "domain",
            Field::Major => "major",
            Field::Minor => "minor",
            Field::Patch => "patch",
            Field::Pre => "pre",
            Field::Issuer => "issuer",
            Field::Last4 => "last4",
            Field::Cc => "cc",
            Field::Prefix => "prefix",
            Field::UuidVersion => "version",
            Field::Oui => "oui",
            Field::Algo => "algo",
            Field::Dir => "dir",
            Field::Name => "name",
            Field::Ext => "ext",
            Field::Lat => "lat",
            Field::Long => "long",
            Field::In => "in",
            Field::Contains => "contains",
            Field::Family => "family",
            Field::Unit => "unit",
            Field::Header => "header",
            Field::Claim => "payload",
            Field::Bits => "bits",
            Field::Texture => "texture",
            Field::Period => "period",
            Field::Text => "text",
            Field::Match => "match",
        }
    }

    /// What answering this field costs, so a predicate asks its cheapest
    /// clauses first and only a token that survives them is decoded. Ordering
    /// a conjunction changes nothing it answers.
    fn cost(self) -> u8 {
        match self {
            // Read from the token's own bytes.
            Field::Header | Field::Claim => 1,
            // One decode, then a walk of the decoded bytes.
            Field::Text => 2,
            Field::Bits => 3,
            Field::Period | Field::Texture => 4,
            // One decode, then a scan over the decoded bytes.
            Field::Match => 5,
            _ => 0,
        }
    }

    /// Whether text under this field compares with ASCII case folded: names
    /// in the DNS, a scheme, an issuer and an algorithm are case-insensitive
    /// by their standards; paths, queries, users and identifiers are not.
    fn folds(self) -> bool {
        matches!(
            self,
            Field::Host | Field::Domain | Field::Scheme | Field::Issuer | Field::Algo | Field::Oui
        )
    }
}

/// The type of value a field compares in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    Num,
    Bytes,
    Duration,
    Instant,
    Ip,
    Cidr,
    Version,
    Text,
    /// A number with a unit, compared in its family's base unit.
    Quantity,
    /// A JSON member's value, compared as whatever it holds: a number
    /// against a number, anything else as its text.
    Json,
    /// A declared sub-pattern, matched against decoded content.
    Sub,
}

/// The fields a kind has, with the type each reads, in the order the error
/// message lists them. A kind whose first field is `Value` reads it when a
/// clause names no field.
fn fields_of(kind: TokenKind) -> &'static [(Field, Ty)] {
    use Field as F;
    match kind {
        TokenKind::Number | TokenKind::Percent | TokenKind::Money => {
            &[(F::Value, Ty::Num), (F::In, Ty::Num)]
        }
        TokenKind::ByteSize => &[(F::Value, Ty::Bytes), (F::In, Ty::Bytes)],
        TokenKind::Duration => &[(F::Value, Ty::Duration), (F::In, Ty::Duration)],
        TokenKind::Quantity => &[
            (F::Value, Ty::Quantity),
            (F::Unit, Ty::Text),
            (F::Family, Ty::Text),
            (F::In, Ty::Quantity),
        ],
        TokenKind::Word | TokenKind::Quoted => &[(F::Len, Ty::Num), (F::In, Ty::Text)],
        TokenKind::Ip => &[(F::Value, Ty::Ip), (F::In, Ty::Cidr), (F::Family, Ty::Text)],
        TokenKind::Cidr => {
            &[(F::Value, Ty::Cidr), (F::In, Ty::Cidr), (F::Contains, Ty::Ip), (F::Prefix, Ty::Num)]
        }
        TokenKind::Url => &[
            (F::Scheme, Ty::Text),
            (F::Host, Ty::Text),
            (F::Port, Ty::Num),
            (F::Path, Ty::Text),
            (F::Query, Ty::Text),
            (F::In, Ty::Text),
        ],
        TokenKind::Email => &[(F::User, Ty::Text), (F::Domain, Ty::Text), (F::In, Ty::Text)],
        TokenKind::Timestamp => &[
            (F::Value, Ty::Instant),
            (F::Age, Ty::Duration),
            (F::Year, Ty::Num),
            (F::Month, Ty::Num),
            (F::Day, Ty::Num),
            (F::Hour, Ty::Num),
            (F::Minute, Ty::Num),
            (F::Second, Ty::Num),
        ],
        TokenKind::Version => &[
            (F::Value, Ty::Version),
            (F::Major, Ty::Num),
            (F::Minor, Ty::Num),
            (F::Patch, Ty::Num),
            (F::Pre, Ty::Text),
            (F::In, Ty::Version),
        ],
        TokenKind::CreditCard => &[(F::Issuer, Ty::Text), (F::Last4, Ty::Text), (F::Len, Ty::Num)],
        TokenKind::Phone => &[(F::Cc, Ty::Num)],
        TokenKind::Uuid => &[(F::UuidVersion, Ty::Num)],
        TokenKind::Mac => &[(F::Oui, Ty::Text)],
        TokenKind::HashDigest => &[(F::Algo, Ty::Text)],
        TokenKind::Path => {
            &[(F::Dir, Ty::Text), (F::Name, Ty::Text), (F::Ext, Ty::Text), (F::In, Ty::Text)]
        }
        TokenKind::Geo => &[(F::Lat, Ty::Num), (F::Long, Ty::Num)],
        // A token's decoded content. The named fields are what every token
        // of the kind has; a JWT's claims are named by the token itself, so
        // `parse_clause` reads any other name as one rather than refusing it.
        TokenKind::Jwt => &[
            (F::Claim, Ty::Json),
            (F::Header, Ty::Json),
            (F::Text, Ty::Text),
            (F::Match, Ty::Sub),
        ],
        TokenKind::Base64 => &[
            (F::Bits, Ty::Num),
            (F::Texture, Ty::Text),
            (F::Period, Ty::Num),
            (F::Text, Ty::Text),
            (F::Match, Ty::Sub),
        ],
        TokenKind::Punct
        | TokenKind::Whitespace
        | TokenKind::HexColor
        | TokenKind::Hex
        | TokenKind::Open(_)
        | TokenKind::Close(_)
        | TokenKind::Custom(_)
        | TokenKind::Other => &[],
    }
}

/// The registered header parameters of a JSON Web Token (RFC 7515), which a
/// bare name reads from the header; every other bare name is a payload
/// claim, and `header.x` or `payload.x` names a half outright.
const HEADER_PARAMETERS: &[&str] = &["alg", "typ", "kid", "cty", "crit", "enc", "zip", "jku", "x5u"];

/// The registered claims written as a NumericDate (RFC 7519): seconds since
/// the epoch, so they compare as instants and take the timestamp grammar.
const DATE_CLAIMS: &[&str] = &["exp", "nbf", "iat"];

/// The field a bare clause on this kind reads.
fn default_field(kind: TokenKind) -> Option<(Field, Ty)> {
    let first = *fields_of(kind).first()?;
    (first.0 == Field::Value).then_some(first)
}

/// How a clause compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    /// `field:value`: a glob on text, containment on an address, equality on
    /// a number.
    Colon,
}

impl Op {
    fn holds(self, o: Ordering) -> bool {
        match self {
            Op::Lt => o == Ordering::Less,
            Op::Le => o != Ordering::Greater,
            Op::Gt => o == Ordering::Greater,
            Op::Ge => o != Ordering::Less,
            Op::Eq | Op::Colon => o == Ordering::Equal,
            Op::Ne => o != Ordering::Equal,
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Eq => "=",
            Op::Ne => "!=",
            Op::Colon => ":",
        }
    }
}

/// A value a clause compares against, in the type it was written in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Value {
    Num(Decimal),
    Instant(Civil),
    /// The clock's instant, written `now`.
    Now,
    Ip(IpAddr),
    Cidr(IpAddr, u8),
    Version(Version),
    Text(String),
    /// `v4` / `v6`: the flag says whether the family is IPv6.
    Family(bool),
    /// A quantity, as its family and its value in the family's base unit.
    Quantity(crate::quantity::Family, Decimal),
    /// A set read from a file, written `@file` or `GROUP:@file`.
    Set(SetRef),
    /// A declared sub-pattern, named where the clause was written.
    Sub(SubRef),
}

/// A sub-pattern a clause matches against decoded content, resolved from the
/// declarations when the pattern was parsed and held under the name it was
/// written with, so two clauses naming one pattern are equal.
#[derive(Clone, Debug)]
pub struct SubRef {
    name: String,
    pattern: std::sync::Arc<crate::ast::Pattern>,
}

impl PartialEq for SubRef {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for SubRef {}

impl std::hash::Hash for SubRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Clause {
    field: Field,
    /// The name a `Header` or `Claim` field reads, empty for the half's own
    /// text and for every other field.
    key: String,
    op: Op,
    value: Value,
}

/// A set read from a file when the pattern was parsed, held once and
/// compared by the text that named it, so two predicates naming one file
/// the same way are equal.
#[derive(Clone, Debug)]
pub struct SetRef {
    spec: String,
    data: std::sync::Arc<SetData>,
}

impl PartialEq for SetRef {
    fn eq(&self, other: &Self) -> bool {
        self.spec == other.spec
    }
}

impl Eq for SetRef {}

impl std::hash::Hash for SetRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.spec.hash(state);
    }
}

/// The members of a set, in the type the field compares in.
#[derive(Debug)]
enum SetData {
    /// Text, each member as it compares: folded under the group or the
    /// field's case rule where one applies.
    Text { members: HashSet<String>, group: Option<crate::orbit::OrbitGroup>, fold: bool },
    /// Address blocks, the network bits of each keyed by its prefix length
    /// and family, so an address is looked up once per prefix length.
    Nets { by_prefix: Vec<(u8, bool, HashSet<u128>)> },
    /// Numbers, sorted, for a binary search.
    Nums(Vec<Decimal>),
    /// Versions, sorted by precedence, for a binary search.
    Versions(Vec<Version>),
    /// Quantities, sorted by family then value in the base unit, for a
    /// binary search.
    Quantities(Vec<(crate::quantity::Family, Decimal)>),
}

impl SetRef {
    /// Read the set `spec` names for a field of type `ty`: `@file`, or
    /// `GROUP:@file` for text folded under an orbit group. `base` is the
    /// directory a relative path is taken from; the current one when none.
    fn load(
        spec: &str,
        kind: TokenKind,
        ty: Ty,
        field: Field,
        shapes: &crate::custom::ShapeSet,
    ) -> Result<SetRef, String> {
        let base = shapes.base_dir();
        let (group, file) = match spec.strip_prefix('@') {
            Some(file) => (None, file),
            None => {
                let Some((name, file)) = spec.split_once(":@") else {
                    return Err(format!("{spec:?} is not a set: write @file or GROUP:@file"));
                };
                let Some(group) = crate::orbit::OrbitGroup::parse(name) else {
                    return Err(format!("{name:?} is not an orbit group; the groups are case, shape, notation, ip, url, time, path, fold and numeric, and every typed relation is one of the same name"));
                };
                (Some(group), file)
            }
        };
        if file.is_empty() {
            return Err("a set needs a file after @".to_string());
        }
        let path = match base {
            Some(dir) if std::path::Path::new(file).is_relative() => dir.join(file),
            _ => std::path::PathBuf::from(file),
        };
        let text = match std::fs::read(&path) {
            Ok(bytes) => String::from_utf8_lossy(&crate::encoding::decode(bytes)).into_owned(),
            Err(e) => return Err(format!("cannot read the set {}: {e}", path.display())),
        };
        if group.is_some() && ty != Ty::Text {
            return Err(format!("a group folds text; `{}` compares in another type", field.name()));
        }
        let data = match ty {
            Ty::Text => {
                let fold = field.folds();
                let members = lines_of(&text)
                    .map(|(_, line)| fold_member(line, group, fold))
                    .collect();
                SetData::Text { members, group, fold }
            }
            Ty::Ip | Ty::Cidr => {
                let mut by_prefix: Vec<(u8, bool, HashSet<u128>)> = Vec::new();
                for (n, line) in lines_of(&text) {
                    let (addr, bits) = match parse_cidr(line) {
                        Some(net) => net,
                        None => match parse_ip(line) {
                            Some(ip) => (ip, if ip.is_ipv6() { 128 } else { 32 }),
                            None => {
                                return Err(format!(
                                    "line {n} of {}: {line:?} is not an address or a block",
                                    path.display()
                                ));
                            }
                        },
                    };
                    let v6 = addr.is_ipv6();
                    let masked = ip_bits(addr) & prefix_mask(bits, v6);
                    match by_prefix.iter_mut().find(|(b, f, _)| *b == bits && *f == v6) {
                        Some((_, _, set)) => {
                            set.insert(masked);
                        }
                        None => by_prefix.push((bits, v6, HashSet::from([masked]))),
                    }
                }
                // Longer prefixes first, so the tightest block that holds an
                // address is the first one asked.
                by_prefix.sort_by_key(|b| std::cmp::Reverse(b.0));
                SetData::Nets { by_prefix }
            }
            Ty::Num | Ty::Bytes | Ty::Duration => {
                let mut nums = Vec::new();
                for (n, line) in lines_of(&text) {
                    let value = parse_value(kind, ty, field, Op::Eq, line, shapes)?;
                    match value {
                        Value::Num(d) => nums.push(d),
                        _ => return Err(format!("line {n} of {}: {line:?} is not a number", path.display())),
                    }
                }
                nums.sort_by(Decimal::compare);
                SetData::Nums(nums)
            }
            Ty::Quantity => {
                let mut members = Vec::new();
                for (n, line) in lines_of(&text) {
                    match crate::quantity::parse(line) {
                        Some(q) => members.push((q.family, q.base)),
                        None => {
                            return Err(format!(
                                "line {n} of {}: {line:?} is not a quantity (a number and a unit)",
                                path.display()
                            ));
                        }
                    }
                }
                members.sort_by(|(fa, a), (fb, b)| fa.cmp(fb).then_with(|| a.compare(b)));
                SetData::Quantities(members)
            }
            Ty::Version => {
                let mut versions = Vec::new();
                for (n, line) in lines_of(&text) {
                    match Version::parse(line) {
                        Some(v) => versions.push(v),
                        None => {
                            return Err(format!("line {n} of {}: {line:?} is not a version", path.display()));
                        }
                    }
                }
                versions.sort_by(Version::compare);
                SetData::Versions(versions)
            }
            Ty::Instant => return Err("a set of instants is not a comparison this field makes".to_string()),
            Ty::Json => {
                // A claim is compared as whatever it holds, so a set of them
                // is a set of the texts they were written as.
                let members = lines_of(&text).map(|(_, line)| line.to_string()).collect();
                SetData::Text { members, group: None, fold: false }
            }
            Ty::Sub => {
                return Err("`match` names one declared sub-pattern, not a set".to_string());
            }
        };
        Ok(SetRef { spec: spec.to_string(), data: std::sync::Arc::new(data) })
    }

    /// Whether the text is a member, folded as the members were.
    fn holds_text(&self, text: &str) -> bool {
        match &*self.data {
            SetData::Text { members, group, fold } => members.contains(&fold_member(text, *group, *fold)),
            SetData::Nets { .. } | SetData::Nums(_) | SetData::Versions(_) | SetData::Quantities(_) => false,
        }
    }

    /// Whether some block holds the address.
    fn holds_ip(&self, ip: IpAddr) -> bool {
        match &*self.data {
            SetData::Nets { by_prefix } => {
                let v6 = ip.is_ipv6();
                let bits = ip_bits(ip);
                by_prefix
                    .iter()
                    .any(|(len, f, set)| *f == v6 && set.contains(&(bits & prefix_mask(*len, v6))))
            }
            SetData::Text { .. } | SetData::Nums(_) | SetData::Versions(_) | SetData::Quantities(_) => false,
        }
    }

    /// Whether some block holds the whole of the block `net/bits`.
    fn holds_block(&self, net: IpAddr, bits: u8) -> bool {
        match &*self.data {
            SetData::Nets { by_prefix } => {
                let v6 = net.is_ipv6();
                let addr = ip_bits(net);
                by_prefix.iter().any(|(len, f, set)| {
                    *f == v6 && *len <= bits && set.contains(&(addr & prefix_mask(*len, v6)))
                })
            }
            SetData::Text { .. } | SetData::Nums(_) | SetData::Versions(_) | SetData::Quantities(_) => false,
        }
    }

    fn holds_num(&self, n: &Decimal) -> bool {
        match &*self.data {
            SetData::Nums(nums) => nums.binary_search_by(|x| x.compare(n)).is_ok(),
            SetData::Text { .. } | SetData::Nets { .. } | SetData::Versions(_) | SetData::Quantities(_) => false,
        }
    }

    fn holds_version(&self, v: &Version) -> bool {
        match &*self.data {
            SetData::Versions(versions) => versions.binary_search_by(|x| x.compare(v)).is_ok(),
            SetData::Text { .. } | SetData::Nets { .. } | SetData::Nums(_) | SetData::Quantities(_) => false,
        }
    }

    fn holds_quantity(&self, family: crate::quantity::Family, v: &Decimal) -> bool {
        match &*self.data {
            SetData::Quantities(members) => members
                .binary_search_by(|(f, x)| f.cmp(&family).then_with(|| x.compare(v)))
                .is_ok(),
            SetData::Text { .. } | SetData::Nets { .. } | SetData::Nums(_) | SetData::Versions(_) => false,
        }
    }
}

/// The lines of a set file that carry a member, with their one-based line
/// numbers: trimmed, with blank lines and lines opening with `#` skipped.
fn lines_of(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
}

/// A member as it compares: under the orbit group where one was named,
/// else with ASCII case folded where the field folds it.
fn fold_member(text: &str, group: Option<crate::orbit::OrbitGroup>, fold: bool) -> String {
    match group {
        Some(g) => crate::orbit::canonical(text.as_bytes(), g),
        None if fold => text.to_ascii_lowercase(),
        None => text.to_string(),
    }
}

/// The mask keeping the first `bits` of an address of the family.
fn prefix_mask(bits: u8, v6: bool) -> u128 {
    let width: u32 = if v6 { 128 } else { 32 };
    let keep = u32::from(bits).min(width);
    let all = u128::MAX >> (128 - width);
    if keep == 0 { 0 } else { all & !((1u128 << (width - keep)) - 1) }
}

/// A typed predicate: the clauses a token of the kind must all satisfy.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypedPred {
    kind: TokenKind,
    clauses: Vec<Clause>,
}

/// The multiplier a byte-size unit carries, decimal prefixes by powers of a
/// thousand and binary ones by powers of 1024.
fn byte_unit(unit: &str) -> Option<u128> {
    let u = unit.to_ascii_lowercase();
    let (prefix, binary) = match u.as_str() {
        "" | "b" => return Some(1),
        s if s.ends_with("ib") => (&s[..s.len() - 2], true),
        s if s.ends_with('b') => (&s[..s.len() - 1], false),
        _ => return None,
    };
    let power = match prefix {
        "k" => 1,
        "m" => 2,
        "g" => 3,
        "t" => 4,
        "p" => 5,
        "e" => 6,
        _ => return None,
    };
    Some(if binary { 1024u128.pow(power) } else { 1000u128.pow(power) })
}

/// A byte size, `1.5GiB`, `512KB`, `10B` or a bare count, as an exact count
/// of bytes.
pub(crate) fn parse_bytes(s: &str) -> Option<Decimal> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let value = Decimal::parse(num)?;
    Some(value.times(byte_unit(unit.trim())?))
}

/// Nanoseconds per duration unit. A day is twenty-four hours, a week seven
/// days, a year 365 days.
fn duration_unit(unit: &str) -> Option<u128> {
    Some(match unit {
        "ns" => 1,
        "us" => 1_000,
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        "m" => 60_000_000_000,
        "h" => 3_600_000_000_000,
        "d" => 86_400_000_000_000,
        "w" => 604_800_000_000_000,
        "y" => 31_536_000_000_000_000,
        _ => return None,
    })
}

/// A duration, one or more `number + unit` segments (`1500ms`, `3h20m`,
/// `2.5s`), as exact nanoseconds. Every segment needs a unit.
pub(crate) fn parse_duration(s: &str) -> Option<Decimal> {
    let b = s.trim().as_bytes();
    let mut total = Decimal::from_u128(0);
    let mut i = 0;
    let mut segments = 0;
    while i < b.len() {
        let start = i;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
            i += 1;
        }
        let num: String = b[start..i].iter().map(|&x| char::from(x)).collect();
        let unit_start = i;
        while i < b.len() && b[i].is_ascii_alphabetic() {
            i += 1;
        }
        let unit: String = b[unit_start..i].iter().map(|&x| char::from(x)).collect();
        if num.is_empty() || unit.is_empty() {
            return None;
        }
        let part = Decimal::parse(&num)?.times(duration_unit(&unit)?);
        total = add_positive(&total, &part)?;
        segments += 1;
    }
    (segments > 0).then_some(total)
}

/// The sum of two non-negative decimals, exact; `None` as [`aligned`] gives.
fn add_positive(a: &Decimal, b: &Decimal) -> Option<Decimal> {
    if a.is_zero() {
        return Some(Decimal { neg: false, ..b.clone() });
    }
    if b.is_zero() {
        return Some(Decimal { neg: false, ..a.clone() });
    }
    let (x, y, exp) = aligned(a, b)?;
    let mut out = Vec::with_capacity(x.len().max(y.len()) + 1);
    let mut carry = 0u8;
    let (mut i, mut j) = (x.len(), y.len());
    while i > 0 || j > 0 || carry > 0 {
        let p = if i > 0 {
            i -= 1;
            x[i]
        } else {
            0
        };
        let q = if j > 0 {
            j -= 1;
            y[j]
        } else {
            0
        };
        let s = p + q + carry;
        out.push(s % 10);
        carry = s / 10;
    }
    out.reverse();
    let s: String = out.iter().map(|d| char::from(b'0' + d)).collect();
    Some(Decimal::scaled(false, &s, exp))
}

fn parse_cidr(s: &str) -> Option<(IpAddr, u8)> {
    let (addr, bits) = s.trim().split_once('/')?;
    let addr = parse_ip(addr)?;
    let bits = parse_int(bits)?;
    let max = if addr.is_ipv6() { 128 } else { 32 };
    (0..=max).contains(&bits).then_some((addr, bits as u8))
}

/// The address as 128 bits, an IPv4 one in its low 32.
fn ip_bits(ip: IpAddr) -> u128 {
    match ip {
        IpAddr::V4(v4) => u128::from(u32::from(v4)),
        IpAddr::V6(v6) => u128::from(v6),
    }
}

/// The network holding `ip` at a prefix of `bits`: the address with its host
/// bits cleared, which is the same address for every member of the block and
/// so is what a key over a subnet is.
fn mask_ip(ip: IpAddr, bits: u8) -> IpAddr {
    let width: u32 = if ip.is_ipv6() { 128 } else { 32 };
    let keep = u32::from(bits).min(width);
    let all = u128::MAX >> (128 - width);
    let mask = if keep == 0 { 0 } else { all & !((1u128 << (width - keep)) - 1) };
    let net = ip_bits(ip) & mask;
    match ip {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::from(net as u32)),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::from(net)),
    }
}

/// Whether `ip` is inside the block.
fn ip_in_cidr(ip: IpAddr, net: IpAddr, bits: u8) -> bool {
    if ip.is_ipv6() != net.is_ipv6() {
        return false;
    }
    let width: u32 = if ip.is_ipv6() { 128 } else { 32 };
    let keep = u32::from(bits);
    if keep == 0 {
        return true;
    }
    let all = u128::MAX >> (128 - width);
    let mask = if keep >= width { all } else { all & !((1u128 << (width - keep)) - 1) };
    (ip_bits(ip) & mask) == (ip_bits(net) & mask)
}

/// The issuer a card's leading digits name, from the issuers' published
/// identification ranges, or `unknown`.
fn card_issuer(digits: &str) -> &'static str {
    let lead = |n: usize| -> u32 {
        match digits.get(..n) {
            Some(s) => s.bytes().fold(0u32, |a, b| a * 10 + u32::from(b - b'0')),
            None => 0,
        }
    };
    let (d1, d2, d3, d4, d6) = (lead(1), lead(2), lead(3), lead(4), lead(6));
    if d2 == 34 || d2 == 37 {
        return "amex";
    }
    if (622_126..=622_925).contains(&d6) || d4 == 6011 || (644..=649).contains(&d3) || d2 == 65 {
        return "discover";
    }
    if (3528..=3589).contains(&d4) {
        return "jcb";
    }
    if (300..=305).contains(&d3) || d4 == 3095 || d2 == 36 || d2 == 38 || d2 == 39 {
        return "diners";
    }
    if (51..=55).contains(&d2) || (2221..=2720).contains(&d4) {
        return "mastercard";
    }
    if d2 == 62 {
        return "unionpay";
    }
    if d1 == 4 {
        return "visa";
    }
    if d2 == 50 || (56..=58).contains(&d2) || d4 == 6304 || d4 == 6759 || (6761..=6763).contains(&d4)
    {
        return "maestro";
    }
    "unknown"
}

/// A token's field read in the type the clause compares in, and the same
/// value every output surface reports.
///
/// One parse serves both: a predicate comparing on a field and a report
/// naming that field's value read the token through [`read_field`], so a
/// reported value cannot disagree with the predicate that selected it.
///
/// Every arm holds the value exactly. `Num` and the base unit of `Quantity`
/// are arbitrary-length decimals, and an `Ip` is 128 bits, so rendering any
/// of them through a float is lossy and the renderer says which spellings
/// are not.
#[derive(Clone, Debug)]
pub enum TypedValue {
    Num(Decimal),
    Instant(Civil),
    Ip(IpAddr),
    Cidr(IpAddr, u8),
    Version(Version),
    Text(String),
    Family(bool),
    Quantity(crate::quantity::Family, Decimal),
}

/// The decoded views of one token, each read at most once and only where a
/// clause asks for it: a predicate naming only header parameters never
/// decodes the payload, and a predicate whose cheaper clauses have already
/// failed decodes nothing at all.
struct Decode {
    blob: Option<Option<Vec<u8>>>,
    header: Option<Option<Vec<u8>>>,
    payload: Option<Option<Vec<u8>>>,
}

impl Decode {
    fn new() -> Decode {
        Decode { blob: None, header: None, payload: None }
    }

    /// The bytes a base64 token encodes.
    fn blob(&mut self, txt: &str) -> Option<&[u8]> {
        self.blob.get_or_insert_with(|| crate::decoded::base64(txt.as_bytes())).as_deref()
    }

    /// One decoded half of a JSON Web Token.
    fn half(&mut self, txt: &str, payload: bool) -> Option<&[u8]> {
        let slot = if payload { &mut self.payload } else { &mut self.header };
        slot.get_or_insert_with(|| crate::decoded::jwt_half(txt.as_bytes(), payload)).as_deref()
    }

    /// The decoded content a clause on `kind` reads: a base64 token's bytes,
    /// or the half of a JSON Web Token the clause names.
    fn content(&mut self, kind: TokenKind, field: Field, txt: &str) -> Option<&[u8]> {
        match kind {
            TokenKind::Jwt => self.half(txt, field != Field::Header),
            _ => self.blob(txt),
        }
    }
}

/// How a value is spelled in a report.
///
/// The parsers here are exact and a JSON number is a double, so the three
/// differ in what they do about that rather than in what they mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ValueSpelling {
    /// A JSON number only where a double holds the value exactly; a string
    /// or a small object everywhere else. The default.
    #[default]
    Exact,
    /// A JSON number throughout, rounding where a double must.
    Natural,
    /// `{"kind": ..., "exact": ...}` for every value.
    Tagged,
}

/// The unit a duration's value is reported in.
///
/// The parser holds a duration as exact nanoseconds and every comparison is
/// made on those, so nanoseconds is what a report shows unless a caller asks
/// otherwise. The other two are a convenience for a reader, taken by an
/// exact decimal shift, so `1500ms` is `1.5` seconds and not `1.4999`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DurationUnit {
    #[default]
    Nanoseconds,
    Milliseconds,
    Seconds,
}

impl DurationUnit {
    /// Digits to shift a nanosecond count right by to reach this unit, for a
    /// caller rendering a duration itself rather than through
    /// [`TypedValue::json`].
    #[must_use]
    pub fn digits(self) -> i32 {
        match self {
            DurationUnit::Nanoseconds => 0,
            DurationUnit::Milliseconds => -6,
            DurationUnit::Seconds => -9,
        }
    }
}

/// How a report renders a value: the spelling, and the unit a duration is
/// shown in. Both are the caller's choice and neither changes what was
/// parsed.
#[derive(Clone, Copy, Debug, Default)]
pub struct ValueStyle {
    pub spelling: ValueSpelling,
    pub duration: DurationUnit,
}

impl ValueStyle {
    /// The default: exact, and a duration in the nanoseconds it is held in.
    #[must_use]
    pub fn new() -> ValueStyle {
        ValueStyle::default()
    }
}

/// An exact quotient of two decimals, in the form a reader asked for.
///
/// Division is not closed over terminating decimals, but it is exact over the
/// rationals, and every rational has a finite way to write it down: a
/// terminating decimal where the reduced denominator's only prime factors are
/// 2 and 5, and otherwise an eventually repeating one whose cycle is at most
/// `denominator - 1` digits long. Neither form rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum QuotientForm {
    /// `30`, `2.5`, `3.(3)`, `0.(142857)`: the digits, with the repeating
    /// ones bracketed where there are any.
    #[default]
    Repetend,
    /// `30`, `5/2`, `10/3`, `1/7`: the fraction in lowest terms.
    Rational,
}

/// The most fraction digits a dividend can carry before scaling it into a
/// `u128` overflows, which is the largest power of ten that type holds.
const MAX_SCALE_DIGITS: usize = 38;

/// The exact quotient of `sum` by `count`, written in `form`.
///
/// `None` for a count of zero, and for a sum too large to scale into a
/// `u128`, where the integer division this runs on cannot hold the dividend.
/// A caller is told there is no quotient rather than handed a wrong one.
/// Nothing here rounds.
///
/// The repetend is found by long division, remembering which remainder
/// produced each digit: a remainder seen twice begins the cycle, and a
/// remainder of zero ends the expansion. There are at most `denominator`
/// distinct remainders, so the loop is bounded by the divisor.
#[must_use]
pub fn exact_quotient(sum: &Decimal, count: u64, form: QuotientForm) -> Option<String> {
    if count == 0 {
        return None;
    }
    // Scale the dividend past its own fraction so the division runs over
    // integers, and carry that scale into the divisor to place the point.
    // A positive power is written out as zeros, which a u128 holds only up
    // to thirty-nine digits, so a longer one is refused before it is written.
    let (written, scaled_by) = if sum.exp >= 0 {
        if sum.digits.len() as i128 + i128::from(sum.exp) > 39 {
            return None;
        }
        (format!("{}{}", sum.digits, "0".repeat(sum.exp as usize)), 0)
    } else {
        (sum.digits.clone(), sum.exp.unsigned_abs() as usize)
    };
    if scaled_by > MAX_SCALE_DIGITS {
        return None;
    }
    let numerator: u128 = if written.is_empty() {
        0
    } else {
        match written.parse() {
            Ok(n) => n,
            Err(_wider_than_a_u128) => return None,
        }
    };
    // Bounded just above, so the width cast cannot truncate.
    let scale = 10u128.checked_pow(scaled_by as u32)?;
    let denominator = u128::from(count).checked_mul(scale)?;
    let sign = if sum.neg && numerator != 0 { "-" } else { "" };

    if form == QuotientForm::Rational {
        let g = gcd(numerator, denominator).max(1);
        let (n, d) = (numerator / g, denominator / g);
        return Some(if d == 1 { format!("{sign}{n}") } else { format!("{sign}{n}/{d}") });
    }

    let whole = numerator / denominator;
    let mut rem = numerator % denominator;
    if rem == 0 {
        return Some(format!("{sign}{whole}"));
    }
    let mut seen: Vec<(u128, usize)> = Vec::new();
    let mut frac: Vec<u8> = Vec::new();
    let cycle_at = loop {
        if let Some((_, at)) = seen.iter().find(|(r, _)| *r == rem) {
            break Some(*at);
        }
        seen.push((rem, frac.len()));
        rem = rem.checked_mul(10)?;
        // A remainder is below the denominator, so ten times it over the
        // denominator is a single digit and this cannot truncate.
        frac.push((rem / denominator) as u8);
        rem %= denominator;
        if rem == 0 {
            break None;
        }
    };
    let digits: String = frac.iter().map(|d| char::from(b'0' + d)).collect();
    Some(match cycle_at {
        Some(at) => format!("{sign}{whole}.{}({})", &digits[..at], &digits[at..]),
        None => format!("{sign}{whole}.{digits}"),
    })
}

/// The greatest common divisor, for reducing a fraction to lowest terms.
fn gcd(a: u128, b: u128) -> u128 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Which aggregate a column reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agg {
    Sum,
    Avg,
    Min,
    Max,
    /// The percentile at this fraction of the sorted values, as a percent, so
    /// 50 is the median and 95 the ninety-fifth.
    Pct(u32),
}

impl Agg {
    /// The flag that asks for it.
    #[must_use]
    pub fn flag(self) -> String {
        match self {
            Agg::Sum => "--sum".to_string(),
            Agg::Avg => "--avg".to_string(),
            Agg::Min => "--min".to_string(),
            Agg::Max => "--max".to_string(),
            Agg::Pct(p) => format!("--p{p}"),
        }
    }

    /// Whether a kind admitting only an order, and not arithmetic, can carry
    /// this aggregate. Summing a timestamp is meaningless; its minimum is not.
    #[must_use]
    pub fn needs_arithmetic(self) -> bool {
        matches!(self, Agg::Sum | Agg::Avg)
    }

    /// Whether `kind` can carry this aggregate at all.
    #[must_use]
    pub fn admits(self, kind: TokenKind) -> bool {
        match aggregable(kind) {
            Aggregable::Numeric => true,
            Aggregable::Ordered => !self.needs_arithmetic(),
            Aggregable::Neither => false,
        }
    }
}

/// The values one key collected under one aggregate, and the aggregate over
/// them.
///
/// Every value is kept rather than folded as it arrives, because a percentile
/// needs the whole sorted run and no number of them may be dropped: a
/// percentile over a sample is a different figure from a percentile, and
/// nothing in the output would say which had been printed.
#[derive(Clone, Debug, Default)]
pub struct Collected {
    values: Vec<TypedValue>,
}

impl Collected {
    /// Keep one more value.
    pub fn push(&mut self, v: TypedValue) {
        self.values.push(v);
    }

    /// How many values this holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// `agg` over these values, rendered, or `None` where there are none to
    /// report or the values cannot answer it.
    ///
    /// `style` renders the result the way a report renders any other value,
    /// so an aggregate and a value in the same table agree on units and
    /// spelling. `form` decides how an average writes an exact quotient.
    #[must_use]
    pub fn report(
        &self,
        agg: Agg,
        kind: TokenKind,
        style: ValueStyle,
        form: QuotientForm,
        how: Percentile,
        clock: Clock,
    ) -> Option<String> {
        if self.values.is_empty() {
            return None;
        }
        match agg {
            Agg::Sum => {
                Some(TypedValue::Num(self.total()?).json(kind, style, clock))
            }
            Agg::Avg => {
                // A duration averages in the unit the column reports, so the
                // scaling happens before the division rather than after it.
                let total = self.total()?;
                let scaled = if kind == TokenKind::Duration {
                    total.shift(style.duration.digits())
                } else {
                    total
                };
                // A vector's length is a count of things in memory, so it
                // fits a u64 on every platform this builds for.
                exact_quotient(&scaled, self.values.len() as u64, form)
            }
            Agg::Min | Agg::Max => {
                let sorted = self.sorted()?;
                let pick = if agg == Agg::Min { sorted.first() } else { sorted.last() };
                Some(pick?.json(kind, style, clock))
            }
            Agg::Pct(p) => self.percentile(p, kind, style, how, clock),
        }
    }

    /// The exact sum, or `None` where a value is ordered but not numeric, or
    /// where two values are too far apart to be summed exactly.
    fn total(&self) -> Option<Decimal> {
        let mut total = Decimal::from_u128(0);
        for v in &self.values {
            total = total.add(v.as_decimal()?)?;
        }
        Some(total)
    }

    /// The values in the kind's own order, or `None` where two of them do not
    /// compare, which is a report asking for an order the kind does not have.
    fn sorted(&self) -> Option<Vec<&TypedValue>> {
        let mut out: Vec<&TypedValue> = self.values.iter().collect();
        let mut comparable = true;
        out.sort_by(|a, b| match a.compare(b) {
            Some(o) => o,
            None => {
                comparable = false;
                Ordering::Equal
            }
        });
        comparable.then_some(out)
    }

    /// The value `agg` picks from these rather than computes: a minimum, a
    /// maximum, or a percentile that is one of the values under the method
    /// `how` names. `None` for a sum or an average, for a percentile between
    /// two values, where there are no values, and where two of them do not
    /// compare.
    ///
    /// It is the value [`Collected::report`] renders, handed back whole for
    /// a caller that converts it rather than prints it, since a rendering
    /// keeps an instant to the second and an address as its integer.
    #[must_use]
    pub fn picked(&self, agg: Agg, kind: TokenKind, how: Percentile) -> Option<&TypedValue> {
        if matches!(agg, Agg::Sum | Agg::Avg) {
            return None;
        }
        let sorted = self.sorted()?;
        match agg {
            Agg::Min => sorted.first().copied(),
            Agg::Max => sorted.last().copied(),
            Agg::Pct(p) => match rank(sorted.len(), p, how, kind)? {
                Rank::At(i) => Some(sorted[i]),
                Rank::Between(..) => None,
            },
            Agg::Sum | Agg::Avg => None,
        }
    }

    /// The percentile at `p` percent, by the method `how` names.
    fn percentile(
        &self,
        p: u32,
        kind: TokenKind,
        style: ValueStyle,
        how: Percentile,
        clock: Clock,
    ) -> Option<String> {
        let sorted = self.sorted()?;
        match rank(sorted.len(), p, how, kind)? {
            Rank::At(i) => Some(sorted[i].json(kind, style, clock)),
            Rank::Between(lower, frac) => {
                let a = sorted[lower].as_decimal()?;
                let b = sorted[lower + 1].as_decimal()?;
                // a + (b - a) * frac/100, as one exact quotient, with a
                // duration in the unit the column reports it in.
                let span = b.add(&a.negated())?;
                let scaled = a.times(100).add(&span.times(frac))?;
                let scaled = if kind == TokenKind::Duration {
                    scaled.shift(style.duration.digits())
                } else {
                    scaled
                };
                exact_quotient(&scaled, 100, QuotientForm::Repetend)
            }
        }
    }
}

/// A percentile's position in a sorted run of values.
enum Rank {
    /// On the value at this index.
    At(usize),
    /// Between the value at this index and the next, this many hundredths
    /// of the way from it.
    Between(usize, u128),
}

/// Where the percentile at `p` percent of `n` sorted values of `kind` is
/// under the method `how` names, or `None` where there are no values or `p`
/// is past 100.
fn rank(n: usize, p: u32, how: Percentile, kind: TokenKind) -> Option<Rank> {
    if n == 0 || p > 100 {
        return None;
    }
    let count = n as u128;
    let p = u128::from(p);
    // The position the interpolating methods read, p percent of the way from
    // the first value to the last, exact in hundredths. A fraction left over
    // places it strictly before the last value.
    let pos = p * (count - 1);
    let below = (pos / 100) as usize;
    let between = match pos % 100 {
        0 => Rank::At(below),
        frac => Rank::Between(below, frac),
    };
    Some(match how {
        Percentile::Lower => Rank::At(below),
        Percentile::Linear => between,
        Percentile::Hybrid if aggregable(kind) == Aggregable::Numeric => between,
        Percentile::Nearest | Percentile::Hybrid => {
            // The rank at p percent, one-based, so 100 names the last value
            // and any p above zero names at least the first.
            let at = (p * count).div_ceil(100).clamp(1, count);
            Rank::At((at - 1) as usize)
        }
    })
}

/// How a percentile between two observed values is decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Percentile {
    /// The value at `ceil(p * n)` in sorted order. Always a value that
    /// occurred, and defined for every ordered kind. The default.
    #[default]
    Nearest,
    /// Interpolated between the two neighbors. Only a kind whose values can
    /// be interpolated takes it; the rest are refused rather than quietly
    /// given another method.
    Linear,
    /// The observed value at or below the position.
    Lower,
    /// Linear where the kind can be interpolated, nearest where it cannot,
    /// for a table whose columns are of both sorts.
    Hybrid,
}

/// Which aggregates a kind admits.
///
/// Adding a timestamp, a version or an address is meaningless while ordering
/// them is not, so summing and averaging take a narrower set than the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aggregable {
    /// Summed, averaged, ordered and interpolated.
    Numeric,
    /// Ordered only: a minimum, a maximum and a percentile read, a sum does
    /// not, and there is nothing between two of them to interpolate.
    Ordered,
    /// Carries no value an aggregate can read.
    Neither,
}

/// What `kind` admits, which decides both the refusal and whether `linear`
/// has a meaning for it.
#[must_use]
pub fn aggregable(kind: TokenKind) -> Aggregable {
    match kind {
        TokenKind::Number
        | TokenKind::ByteSize
        | TokenKind::Duration
        | TokenKind::Money
        | TokenKind::Percent
        | TokenKind::Quantity => Aggregable::Numeric,
        TokenKind::Timestamp | TokenKind::Version | TokenKind::Ip => Aggregable::Ordered,
        _ => Aggregable::Neither,
    }
}

/// The largest integer a double holds exactly.
const EXACT_IN_A_DOUBLE: i64 = 1 << 53;

/// A JSON string with the six characters that must be escaped in one.
pub(crate) fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// An address as the decimal integer it orders by: 32 bits for v4, 128 for v6.
fn address_int(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => u32::from(*v4).to_string(),
        IpAddr::V6(v6) => u128::from(*v6).to_string(),
    }
}

impl TypedValue {
    /// The word the `tagged` spelling gives this value's kind.
    fn kind_word(&self, kind: TokenKind) -> &'static str {
        match self {
            TypedValue::Instant(_) => "timestamp",
            TypedValue::Ip(_) => "address",
            TypedValue::Cidr(_, _) => "block",
            TypedValue::Version(_) => "version",
            TypedValue::Family(_) => "family",
            TypedValue::Quantity(_, _) => "quantity",
            TypedValue::Text(_) => "text",
            TypedValue::Num(_) => match kind {
                TokenKind::Money => "money",
                TokenKind::Percent => "percent",
                TokenKind::ByteSize => "bytesize",
                TokenKind::Duration => "duration",
                _ => "number",
            },
        }
    }

    /// Whether a decimal of this kind keeps its meaning as a JSON number.
    ///
    /// Money and a percentage never do: they are written with fraction
    /// digits that a double cannot hold, and a rounded amount in a report is
    /// worse than none. Anything else does when it is a whole number inside
    /// a double's exact range.
    fn fits_a_double(text: &str, kind: TokenKind) -> bool {
        if matches!(kind, TokenKind::Money | TokenKind::Percent) {
            return false;
        }
        match text.parse::<i64>() {
            Ok(n) => n.abs() < EXACT_IN_A_DOUBLE,
            Err(_not_an_integer) => false,
        }
    }

    /// The exact text of this value, whatever spelling asked for it. Every
    /// arm renders from the parsed value rather than the token's text, so
    /// `1e3` and `1000` answer alike.
    fn exact_text(&self, kind: TokenKind, style: ValueStyle, clock: Clock) -> String {
        match self {
            TypedValue::Num(d) if kind == TokenKind::Duration => {
                d.shift(style.duration.digits()).to_text()
            }
            TypedValue::Num(d) => d.to_text(),
            TypedValue::Instant(civil) => match civil.epoch(clock) {
                Some((secs, _nanos)) => secs.to_string(),
                None => String::new(),
            },
            TypedValue::Ip(ip) => address_int(ip),
            TypedValue::Cidr(ip, bits) => format!("{}/{bits}", address_int(ip)),
            TypedValue::Version(v) => v.to_text(),
            TypedValue::Text(s) => s.clone(),
            TypedValue::Family(v6) => if *v6 { "v6" } else { "v4" }.to_string(),
            TypedValue::Quantity(family, base) => {
                format!("{} {}", base.to_text(), family.name())
            }
        }
    }

    /// This value as a JSON fragment, under `spelling`, for a token of
    /// `kind`. `clock` places a timestamp that carries no zone or no year.
    ///
    /// A timestamp that names a time and no date has no epoch second, so it
    /// renders as JSON `null` rather than as a guess.
    #[must_use]
    pub fn json(&self, kind: TokenKind, style: ValueStyle, clock: Clock) -> String {
        let spelling = style.spelling;
        let exact = self.exact_text(kind, style, clock);
        if exact.is_empty() {
            return "null".to_string();
        }
        if spelling == ValueSpelling::Tagged {
            return format!(
                "{{\"kind\": {}, \"exact\": {}}}",
                json_string(self.kind_word(kind)),
                json_string(&exact)
            );
        }
        match self {
            TypedValue::Num(d) => match spelling {
                ValueSpelling::Natural => d.as_f64().map_or(exact, |f| format!("{f}")),
                _ if TypedValue::fits_a_double(&exact, kind) => exact,
                _ => json_string(&exact),
            },
            TypedValue::Instant(_) => exact,
            TypedValue::Ip(ip) => match spelling {
                // A v4 address is 32 bits and a double holds it; a v6
                // address is 128 bits, more than a double holds exactly.
                ValueSpelling::Natural if ip.is_ipv4() => exact,
                _ => json_string(&exact),
            },
            TypedValue::Version(v) => v.json(),
            TypedValue::Quantity(family, base) => format!(
                "{{\"family\": {}, \"value\": {}}}",
                json_string(family.name()),
                match spelling {
                    ValueSpelling::Natural => base.as_f64().map_or_else(
                        || json_string(&base.to_text()),
                        |f| format!("{f}")
                    ),
                    _ if TypedValue::fits_a_double(&base.to_text(), kind) => base.to_text(),
                    _ => json_string(&base.to_text()),
                }
            ),
            TypedValue::Cidr(_, _) | TypedValue::Text(_) | TypedValue::Family(_) => {
                json_string(&exact)
            }
        }
    }
}

impl TypedValue {
    /// This value against another of the same kind, in the order the kind
    /// sorts by, or `None` where the two are not comparable.
    ///
    /// Two values of one kind always are; `None` says the caller mixed kinds,
    /// which a report cannot do because a register binds one kind.
    #[must_use]
    pub fn compare(&self, other: &TypedValue) -> Option<Ordering> {
        match (self, other) {
            (TypedValue::Num(a), TypedValue::Num(b)) => Some(a.compare(b)),
            (TypedValue::Version(a), TypedValue::Version(b)) => Some(a.compare(b)),
            (TypedValue::Ip(a), TypedValue::Ip(b)) => Some(a.cmp(b)),
            (TypedValue::Text(a), TypedValue::Text(b)) => Some(a.cmp(b)),
            (TypedValue::Instant(a), TypedValue::Instant(b)) => {
                let clock = Clock::current();
                match (a.epoch(clock), b.epoch(clock)) {
                    (Some(x), Some(y)) => Some(x.cmp(&y)),
                    // A time with no date places no instant, so it has no
                    // position among instants that do.
                    _ => None,
                }
            }
            (TypedValue::Quantity(fa, a), TypedValue::Quantity(fb, b)) if fa == fb => {
                Some(a.compare(b))
            }
            _ => None,
        }
    }

    /// The decimal this value sums and averages as, or `None` for a kind that
    /// is ordered but not numeric.
    ///
    /// A quantity answers its base unit, so a sum of quantities is a sum
    /// within one family; the caller keeps the family beside it.
    #[must_use]
    pub fn as_decimal(&self) -> Option<&Decimal> {
        match self {
            TypedValue::Num(d) | TypedValue::Quantity(_, d) => Some(d),
            _ => None,
        }
    }
}

/// The parsed value of a token of `kind` whose text is `txt`, or `None`
/// where the kind carries no value or the text does not parse as one.
///
/// This is the same read a predicate on `:value` makes, through the same
/// function, so a value in a report and a value in a comparison cannot
/// drift apart. A kind whose token is only text, and a duration written
/// without a unit, both answer `None` rather than a substitute.
#[must_use]
pub fn value_of(kind: TokenKind, txt: &str) -> Option<TypedValue> {
    read_field(kind, kind, Field::Value, txt)
}

/// The part of an email address on one side of the `@`.
fn email_part(txt: &str, user: bool) -> String {
    match txt.split_once('@') {
        Some((u, d)) => if user { u.to_string() } else { d.to_string() },
        None => String::new(),
    }
}

/// The field's value in a token of `kind` whose text is `txt`, or `None`
/// where the token does not carry it. `token` is the kind the token was
/// lexed as: the kind the predicate was written on, except under the
/// quantity class, where a byte size, a duration, a percentage or a
/// context-read kind reads as a quantity in its own units.
fn read_field(kind: TokenKind, token: TokenKind, field: Field, txt: &str) -> Option<TypedValue> {
    use Field as F;
    use crate::rewrite::{PathPart, UrlPart, path_field, url_part};
    let text = |s: String| Some(TypedValue::Text(s));
    let num = |s: &str| Decimal::parse(s).map(TypedValue::Num);
    // A set on a kind's own value reads the value; on a text kind it reads
    // the whole text.
    let field = match (kind, field) {
        (
            TokenKind::Number
            | TokenKind::Percent
            | TokenKind::Money
            | TokenKind::ByteSize
            | TokenKind::Duration
            | TokenKind::Version
            | TokenKind::Quantity,
            F::In,
        ) => F::Value,
        _ => field,
    };
    match (kind, field) {
        (TokenKind::Quantity, F::Value) => {
            crate::quantity::parse_as(token, txt).map(|q| TypedValue::Quantity(q.family, q.base))
        }
        (TokenKind::Quantity, F::Unit) => crate::quantity::parse_as(token, txt).map(|q| TypedValue::Text(q.unit)),
        (TokenKind::Quantity, F::Family) => {
            crate::quantity::parse_as(token, txt).map(|q| TypedValue::Text(q.family.name().to_string()))
        }
        (TokenKind::Number, F::Value) => number_value(txt).map(TypedValue::Num),
        (TokenKind::Percent, F::Value) => num(txt.trim_end_matches('%')),
        (TokenKind::Money, F::Value) => {
            let cleaned: String = txt.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
            num(&cleaned)
        }
        (TokenKind::ByteSize, F::Value) => parse_bytes(txt).map(TypedValue::Num),
        (TokenKind::Duration, F::Value) => parse_duration(txt).map(TypedValue::Num),
        (TokenKind::Word | TokenKind::Quoted, F::Len) => {
            Some(TypedValue::Num(Decimal::from_u128(txt.chars().count() as u128)))
        }
        (
            TokenKind::Word | TokenKind::Quoted | TokenKind::Email | TokenKind::Url | TokenKind::Path,
            F::In,
        ) => text(txt.to_string()),
        (TokenKind::Ip, F::Value | F::In) => parse_ip(txt).map(TypedValue::Ip),
        (TokenKind::Ip, F::Family) => parse_ip(txt).map(|ip| TypedValue::Family(ip.is_ipv6())),
        (TokenKind::Cidr, F::Value | F::In | F::Contains) => {
            parse_cidr(txt).map(|(a, b)| TypedValue::Cidr(a, b))
        }
        (TokenKind::Cidr, F::Prefix) => {
            parse_cidr(txt).map(|(_, b)| TypedValue::Num(Decimal::from_u128(u128::from(b))))
        }
        (TokenKind::Url, F::Scheme) => text(url_part(txt, UrlPart::Scheme)),
        (TokenKind::Url, F::Host) => text(url_part(txt, UrlPart::Host)),
        (TokenKind::Url, F::Port) => {
            let port = url_part(txt, UrlPart::Port);
            if port.is_empty() {
                // The scheme's default port, where the scheme has one.
                let scheme = url_part(txt, UrlPart::Scheme).to_ascii_lowercase();
                let default = match scheme.as_str() {
                    "http" | "ws" => 80,
                    "https" | "wss" => 443,
                    "ftp" => 21,
                    "ssh" | "sftp" => 22,
                    "smtp" => 25,
                    _ => return None,
                };
                return Some(TypedValue::Num(Decimal::from_u128(default)));
            }
            num(&port)
        }
        (TokenKind::Url, F::Path) => text(url_part(txt, UrlPart::Path)),
        (TokenKind::Url, F::Query) => text(url_part(txt, UrlPart::Query)),
        (TokenKind::Email, F::User) => text(email_part(txt, true)),
        (TokenKind::Email, F::Domain) => text(email_part(txt, false)),
        (TokenKind::Timestamp, F::Value | F::Age) => parse_civil(txt).map(TypedValue::Instant),
        (TokenKind::Timestamp, F::Year) => {
            parse_civil(txt)?.year.map(|y| TypedValue::Num(Decimal::from_i64(i64::from(y))))
        }
        (TokenKind::Timestamp, F::Month | F::Day) => {
            let c = parse_civil(txt)?;
            let v = if field == F::Month { c.month } else { c.day };
            c.has_date.then(|| TypedValue::Num(Decimal::from_u128(u128::from(v))))
        }
        (TokenKind::Timestamp, F::Hour | F::Minute | F::Second) => {
            let c = parse_civil(txt)?;
            let v = match field {
                F::Hour => c.hour,
                F::Minute => c.minute,
                _ => c.second,
            };
            c.has_time.then(|| TypedValue::Num(Decimal::from_u128(u128::from(v))))
        }
        (TokenKind::Version, F::Value) => Version::parse(txt).map(TypedValue::Version),
        (TokenKind::Version, F::Major) => Version::parse(txt).map(|v| TypedValue::Num(v.part(0))),
        (TokenKind::Version, F::Minor) => Version::parse(txt).map(|v| TypedValue::Num(v.part(1))),
        (TokenKind::Version, F::Patch) => Version::parse(txt).map(|v| TypedValue::Num(v.part(2))),
        (TokenKind::Version, F::Pre) => Version::parse(txt).map(|v| TypedValue::Text(v.pre_text())),
        (TokenKind::CreditCard, F::Issuer | F::Last4 | F::Len) => {
            let digits: String = txt.chars().filter(char::is_ascii_digit).collect();
            match field {
                F::Issuer => text(card_issuer(&digits).to_string()),
                F::Last4 => text(digits[digits.len().saturating_sub(4)..].to_string()),
                _ => Some(TypedValue::Num(Decimal::from_u128(digits.len() as u128))),
            }
        }
        (TokenKind::Phone, F::Cc) => {
            let digits: Vec<u8> =
                txt.bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect();
            let code = if txt.starts_with('+') {
                let len = crate::lexer::country_code_len(&digits)?;
                digits[..len].iter().fold(0u128, |a, &d| a * 10 + u128::from(d))
            } else {
                1
            };
            Some(TypedValue::Num(Decimal::from_u128(code)))
        }
        (TokenKind::Uuid, F::UuidVersion) => {
            let c = txt.as_bytes().get(14)?;
            let v = (*c as char).to_digit(16)?;
            Some(TypedValue::Num(Decimal::from_u128(u128::from(v))))
        }
        (TokenKind::Mac, F::Oui) => {
            let groups: Vec<String> =
                txt.split([':', '-']).take(3).map(|g| g.to_ascii_lowercase()).collect();
            text(groups.join(":"))
        }
        (TokenKind::HashDigest, F::Algo) => text(
            match txt.len() {
                32 => "md5",
                40 => "sha1",
                64 => "sha256",
                _ => "unknown",
            }
            .to_string(),
        ),
        (TokenKind::Path, F::Dir) => text(path_field(txt, PathPart::Dir)),
        (TokenKind::Path, F::Name) => text(path_field(txt, PathPart::Name)),
        (TokenKind::Path, F::Ext) => text(path_field(txt, PathPart::Ext)),
        (TokenKind::Geo, F::Lat | F::Long) => {
            let (lat, long) = txt.split_once(',')?;
            num(if field == F::Lat { lat } else { long })
        }
        _ => None,
    }
}

fn field_by_name(kind: TokenKind, name: &str) -> Option<(Field, Ty)> {
    fields_of(kind).iter().copied().find(|(f, _)| f.name() == name)
}

fn field_list(kind: TokenKind) -> String {
    let names: Vec<&str> = fields_of(kind).iter().map(|(f, _)| f.name()).collect();
    names.join(", ")
}

fn parse_value(
    kind: TokenKind,
    ty: Ty,
    field: Field,
    op: Op,
    raw: &str,
    shapes: &crate::custom::ShapeSet,
) -> Result<Value, String> {
    let raw = raw.trim();
    // `@file`, or `GROUP:@file`: the members are read now, once, and the
    // clause holds of a token that is one of them.
    if raw.starts_with('@') || raw.contains(":@") {
        if !matches!(op, Op::Colon | Op::Eq | Op::Ne) {
            return Err(format!("a set takes `{}:` `=` or `!=`, not an ordering", field.name()));
        }
        return SetRef::load(raw, kind, ty, field, shapes).map(Value::Set);
    }
    if ty == Ty::Sub {
        if !matches!(op, Op::Colon | Op::Eq | Op::Ne) {
            return Err("`match` takes `:` `=` or `!=`, not an ordering".to_string());
        }
        let Some(pattern) = shapes.let_of(raw).cloned().or_else(|| {
            shapes.consults_library().then(|| crate::library::let_of(raw).cloned()).flatten()
        }) else {
            return Err(format!(
                "`match` names a declared sub-pattern; {raw:?} is not one (declare it with `let {raw} = ...`)"
            ));
        };
        return Ok(Value::Sub(SubRef { name: raw.to_string(), pattern: std::sync::Arc::new(pattern) }));
    }
    if field == Field::Family && kind == TokenKind::Ip {
        return match raw {
            "v4" => Ok(Value::Family(false)),
            "v6" => Ok(Value::Family(true)),
            _ => Err(format!("{raw:?} is not an address family; the families are v4 and v6")),
        };
    }
    if ty == Ty::Text {
        return Ok(Value::Text(raw.to_string()));
    }
    if raw.is_empty() {
        return Err(format!("`{}` needs a value after `{}`", field.name(), op.glyph()));
    }
    match ty {
        Ty::Num => {
            let cleaned: String = raw.chars().filter(|c| !matches!(c, '$' | '%' | '_')).collect();
            Decimal::parse(&cleaned).map(Value::Num).ok_or_else(|| format!("{raw:?} is not a number"))
        }
        Ty::Bytes => parse_bytes(raw)
            .map(Value::Num)
            .ok_or_else(|| format!("{raw:?} is not a byte size (10B, 512KB, 1.5GiB)")),
        Ty::Duration => parse_duration(raw)
            .map(Value::Num)
            .ok_or_else(|| format!("{raw:?} is not a duration; write the unit: 500ms, 2s, 3h20m")),
        Ty::Instant => {
            if raw == "now" {
                return Ok(Value::Now);
            }
            parse_civil(raw).map(Value::Instant).ok_or_else(|| {
                format!("{raw:?} is not a timestamp (2026-09-01, 2026-09-01T12:00:00Z, 12:30)")
            })
        }
        Ty::Ip => parse_ip(raw).map(Value::Ip).ok_or_else(|| format!("{raw:?} is not an IP address")),
        Ty::Cidr => {
            if let Some((a, b)) = parse_cidr(raw) {
                return Ok(Value::Cidr(a, b));
            }
            // A bare address under `in:` is the block holding only it.
            if field == Field::In
                && let Some(ip) = parse_ip(raw)
            {
                let bits = if ip.is_ipv6() { 128 } else { 32 };
                return Ok(Value::Cidr(ip, bits));
            }
            Err(format!("{raw:?} is not a CIDR block (10.0.0.0/8)"))
        }
        Ty::Version => Version::parse(raw)
            .map(Value::Version)
            .ok_or_else(|| format!("{raw:?} is not a version (2.0, 1.2.3-beta)")),
        Ty::Quantity => crate::quantity::parse(raw).map(|q| Value::Quantity(q.family, q.base)).ok_or_else(|| {
            format!("{raw:?} is not a quantity; write a number and a unit: 5kg, -40\u{b0}C, 3.2GHz, 40%, 1GiB, 2h")
        }),
        // A claim is compared as whatever it was written as, so a value that
        // reads as a number compares numerically and anything else as text.
        Ty::Json => Ok(match Decimal::parse(raw) {
            Some(d) => Value::Num(d),
            None => Value::Text(raw.to_string()),
        }),
        Ty::Text | Ty::Sub => Ok(Value::Text(raw.to_string())),
    }
}

/// The field a clause on a JSON Web Token names, with the claim it reads and
/// the byte offset where its operator begins.
///
/// A token names its own claims, so a name the kind has no field for is one
/// rather than an error: a registered header parameter reads the header,
/// `header.x` and `payload.x` name a half outright, `header` and `payload`
/// alone are the half's whole text, and every other name is a payload claim.
/// `text` and `match` are the kind's own fields and are left to those.
fn jwt_field(c: &str) -> Option<(Field, String, Ty, usize)> {
    let end = c
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-'))
        .unwrap_or(c.len());
    let name = &c[..end];
    if name.is_empty() || name == "text" || name == "match" {
        return None;
    }
    let (field, key) = match name.split_once('.') {
        Some(("header", k)) => (Field::Header, k),
        Some(("payload", k)) => (Field::Claim, k),
        Some(_) => return None,
        None if name == "header" => (Field::Header, ""),
        None if name == "payload" => (Field::Claim, ""),
        None if HEADER_PARAMETERS.contains(&name) => (Field::Header, name),
        None => (Field::Claim, name),
    };
    // A half with no claim after it is that half's own text.
    let ty = if key.is_empty() {
        Ty::Text
    } else if DATE_CLAIMS.contains(&key) {
        Ty::Instant
    } else {
        Ty::Json
    };
    Some((field, key.to_string(), ty, end))
}

impl TypedPred {
    /// Parse the body between the braces of `\K{...}` for a token kind `K`.
    ///
    /// # Errors
    ///
    /// A clause the kind has no field for, an operator the field's type does
    /// not order by, or a value that does not read in the field's type.
    pub fn parse(kind: TokenKind, body: &str) -> Result<TypedPred, String> {
        TypedPred::parse_in(kind, body, &crate::custom::ShapeSet::new())
    }

    /// [`Self::parse`] against the declarations in force: a relative `@file`
    /// set is read from the set's own directory, and `match:` names a
    /// sub-pattern declared there or in the shipped library.
    ///
    /// # Errors
    ///
    /// As [`Self::parse`], and a set file that cannot be read or holds a
    /// line that is not a member, or a `match:` naming no sub-pattern.
    pub fn parse_in(
        kind: TokenKind,
        body: &str,
        shapes: &crate::custom::ShapeSet,
    ) -> Result<TypedPred, String> {
        let mut clauses = Vec::new();
        for raw in body.split(',') {
            let c = raw.trim();
            if c.is_empty() {
                return Err("an empty clause between the commas".to_string());
            }
            parse_clause(kind, c, &mut clauses, shapes)?;
        }
        if clauses.is_empty() {
            return Err("an empty predicate".to_string());
        }
        // Cheapest first, so a token is decoded only once the clauses
        // its own bytes answer have all held. A conjunction answers the same
        // whatever order it is asked in, and the sort is stable, so clauses
        // of equal cost stay in the order they were written.
        clauses.sort_by_key(|c| c.field.cost());
        Ok(TypedPred { kind, clauses })
    }

    /// The kind the predicate was written on.
    #[must_use]
    pub fn kind(&self) -> TokenKind {
        self.kind
    }

    /// The least value a token can carry and still satisfy this predicate,
    /// where a clause bounds its value below; `None` where none does.
    ///
    /// For an index over whole files rather than tokens: a file whose
    /// largest number is smaller than this holds no token the predicate
    /// accepts, so the file can be refused without being read. Only a clause
    /// on the value itself bounds it - a clause on a part, a host or a length
    /// says nothing about the value - and only `>` and `>=` bound it below,
    /// `>` conservatively read as `>=`, since a bound that is too low
    /// refuses fewer files and never the wrong one.
    #[must_use]
    pub fn least_value(&self) -> Option<f64> {
        self.clauses
            .iter()
            .filter(|c| c.field == Field::Value && matches!(c.op, Op::Gt | Op::Ge))
            .filter_map(|c| match &c.value {
                Value::Num(d) => d.as_f64(),
                Value::Quantity(_, d) => d.as_f64(),
                _ => None,
            })
            .max_by(f64::total_cmp)
    }

    /// The earliest instant a timestamp can carry and still satisfy this
    /// predicate, as seconds since the epoch, read against `clock`; `None`
    /// where no clause bounds it.
    ///
    /// Two clauses bound it: `\T{>TIME}` bounds the instant itself, and
    /// `\T{age<D}` bounds how long before now it may be, which is the same
    /// bound written from the other end. As [`Self::least_value`], a bound
    /// too early refuses fewer files and never the wrong one.
    #[must_use]
    pub fn earliest(&self, clock: Clock) -> Option<i64> {
        self.clauses
            .iter()
            .filter_map(|c| match (c.field, c.op, &c.value) {
                (Field::Value, Op::Gt | Op::Ge, Value::Instant(civil)) => {
                    civil.epoch(clock).map(|(secs, _)| secs)
                }
                (Field::Value, Op::Gt | Op::Ge, Value::Now) => Some(clock.now_secs),
                // An age below a duration is an instant after now less that
                // duration. The age is held in nanoseconds.
                (Field::Age, Op::Lt | Op::Le, Value::Num(d)) => {
                    let nanos = d.as_f64()?;
                    Some(clock.now_secs - (nanos / 1e9) as i64)
                }
                _ => None,
            })
            .max()
    }

    /// Whether a clause reads the clock: an age, `now`, or a timestamp placed
    /// on it.
    #[must_use]
    pub fn reads_clock(&self) -> bool {
        self.clauses
            .iter()
            .any(|c| c.field == Field::Age || matches!(c.value, Value::Now | Value::Instant(_)))
    }

    /// Whether a token of the predicate's kind with text `txt` satisfies every
    /// clause. `clock` is called at most once, and only by a clause that
    /// needs it.
    #[must_use]
    pub fn matches(&self, txt: &[u8], clock: impl Fn() -> Clock) -> bool {
        self.matches_as(self.kind, txt, clock)
    }

    /// [`Self::matches`] for a token lexed as `token`: the predicate's own
    /// kind, or under the quantity class a byte size, a duration, a
    /// percentage or a context-read kind, each read in its own units.
    #[must_use]
    pub fn matches_as(&self, token: TokenKind, txt: &[u8], clock: impl Fn() -> Clock) -> bool {
        let text = String::from_utf8_lossy(txt);
        let mut cached: Option<Clock> = None;
        let mut read_clock = || match cached {
            Some(c) => c,
            None => {
                let c = clock();
                cached = Some(c);
                c
            }
        };
        let mut decode = Decode::new();
        self.clauses
            .iter()
            .all(|c| clause_holds(self.kind, token, c, &text, &mut read_clock, &mut decode))
    }
}

fn parse_clause(
    kind: TokenKind,
    c: &str,
    out: &mut Vec<Clause>,
    shapes: &crate::custom::ShapeSet,
) -> Result<(), String> {
    if c == "v4" || c == "v6" {
        if kind != TokenKind::Ip {
            return Err(format!("`{c}` is an address family and this kind is not an address"));
        }
        out.push(Clause {
            field: Field::Family,
            key: String::new(),
            op: Op::Eq,
            value: Value::Family(c == "v6"),
        });
        return Ok(());
    }
    if fields_of(kind).is_empty() {
        return Err("this kind has no field a predicate can read".to_string());
    }
    let ident_end =
        c.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')).unwrap_or(c.len());
    let ident = &c[..ident_end];
    let after = &c[ident_end..];
    let jwt = if kind == TokenKind::Jwt { jwt_field(c) } else { None };
    let (field, key, ty, rest) = match jwt {
        Some((f, k, t, at)) => (f, k, t, &c[at..]),
        None => {
            let named = if !ident.is_empty() && after.starts_with(':') && !after.starts_with("::") {
                Some(field_by_name(kind, ident).ok_or_else(|| {
                    format!("no field `{ident}` on this kind; its fields are {}", field_list(kind))
                })?)
            } else if !ident.is_empty() && after.starts_with(['<', '>', '=', '!']) {
                field_by_name(kind, ident)
            } else {
                None
            };
            match named {
                Some((f, t)) => (f, String::new(), t, after),
                None => {
                    let (f, t) = default_field(kind).ok_or_else(|| {
                        format!("this kind has no single value; name a field: {}", field_list(kind))
                    })?;
                    (f, String::new(), t, c)
                }
            }
        }
    };
    if ty == Ty::Text || ty == Ty::Sub || matches!(field, Field::In | Field::Contains) {
        let (op, raw) = if let Some(r) = rest.strip_prefix(':') {
            (Op::Colon, r)
        } else if let Some(r) = rest.strip_prefix("!=") {
            (Op::Ne, r)
        } else if let Some(r) = rest.strip_prefix('=') {
            (Op::Eq, r)
        } else {
            return Err(format!(
                "`{}` takes `:` (a glob), `=` or `!=`; the ordering operators need a number, a time, a version, a size or a quantity",
                field.name()
            ));
        };
        let value = parse_value(kind, ty, field, op, raw, shapes)?;
        out.push(Clause { field, key, op, value });
        return Ok(());
    }
    if let Some(r) = rest.strip_prefix(':') {
        // `port:8080`, `year:2026`: equality written with the colon. A claim
        // is globbed instead, so `iss:*.internal` reads as a pattern and
        // `alg:none`, holding no wildcard, still reads as equality.
        let op = if ty == Ty::Json { Op::Colon } else { Op::Eq };
        let value = parse_value(kind, ty, field, op, r, shapes)?;
        out.push(Clause { field, key, op, value });
        return Ok(());
    }
    let (op, raw) = if let Some(r) = rest.strip_prefix(">=") {
        (Some(Op::Ge), r)
    } else if let Some(r) = rest.strip_prefix("<=") {
        (Some(Op::Le), r)
    } else if let Some(r) = rest.strip_prefix("!=") {
        (Some(Op::Ne), r)
    } else if let Some(r) = rest.strip_prefix('>') {
        (Some(Op::Gt), r)
    } else if let Some(r) = rest.strip_prefix('<') {
        (Some(Op::Lt), r)
    } else if let Some(r) = rest.strip_prefix('=') {
        (Some(Op::Eq), r)
    } else {
        (None, rest)
    };
    match op {
        Some(op) => {
            let value = parse_value(kind, ty, field, op, raw, shapes)?;
            out.push(Clause { field, key, op, value });
        }
        None => {
            let Some((lo, hi)) = raw.split_once("..") else {
                return Err(format!(
                    "a clause is `field:value`, an operator and a value, or a range `a..b`; {c:?} is none of those"
                ));
            };
            let lo = parse_value(kind, ty, field, Op::Ge, lo, shapes)?;
            let hi = parse_value(kind, ty, field, Op::Le, hi, shapes)?;
            out.push(Clause { field, key: key.clone(), op: Op::Ge, value: lo });
            out.push(Clause { field, key, op: Op::Le, value: hi });
        }
    }
    Ok(())
}

/// Whether one clause holds of the text of a token lexed as `token`.
fn clause_holds(
    kind: TokenKind,
    token: TokenKind,
    c: &Clause,
    text: &str,
    clock: &mut dyn FnMut() -> Clock,
    decode: &mut Decode,
) -> bool {
    if c.field.cost() > 0 {
        return decoded_holds(kind, c, text, clock, decode);
    }
    let Some(read) = read_field(kind, token, c.field, text) else {
        return false;
    };
    if let Value::Set(set) = &c.value {
        let held = match &read {
            TypedValue::Text(a) => set.holds_text(a),
            TypedValue::Ip(a) => set.holds_ip(*a),
            TypedValue::Cidr(net, bits) => set.holds_block(*net, *bits),
            TypedValue::Num(a) => set.holds_num(a),
            TypedValue::Version(a) => set.holds_version(a),
            TypedValue::Quantity(f, a) => set.holds_quantity(*f, a),
            TypedValue::Instant(_) | TypedValue::Family(_) => false,
        };
        return if c.op == Op::Ne { !held } else { held };
    }
    match (&read, &c.value) {
        (TypedValue::Num(a), Value::Num(b)) => c.op.holds(a.compare(b)),
        // Two quantities compare only within one family; across families
        // they are unequal and never ordered.
        (TypedValue::Quantity(fa, a), Value::Quantity(fb, b)) => {
            if fa != fb {
                return c.op == Op::Ne;
            }
            c.op.holds(a.compare(b))
        }
        (TypedValue::Instant(civil), Value::Num(dur)) if c.field == Field::Age => {
            let now = clock();
            let Some((secs, nanos)) = civil.epoch(now) else {
                return false;
            };
            c.op.holds(elapsed_nanos(now.now_secs, secs, nanos).compare(dur))
        }
        (TypedValue::Instant(civil), Value::Now) => {
            let now = clock();
            let Some((secs, nanos)) = civil.epoch(now) else {
                return false;
            };
            c.op.holds((secs, nanos).cmp(&(now.now_secs, 0)))
        }
        (TypedValue::Instant(a), Value::Instant(b)) => {
            if a.has_date && b.has_date {
                let now = clock();
                let (Some(x), Some(y)) = (a.epoch(now), b.epoch(now)) else {
                    return false;
                };
                c.op.holds(x.cmp(&y))
            } else if !a.has_date && !b.has_date {
                let (Some(x), Some(y)) = (a.seconds_of_day(), b.seconds_of_day()) else {
                    return false;
                };
                c.op.holds(x.cmp(&y))
            } else {
                false
            }
        }
        (TypedValue::Ip(a), Value::Ip(b)) => {
            if a.is_ipv6() != b.is_ipv6() {
                return c.op == Op::Ne;
            }
            c.op.holds(ip_bits(*a).cmp(&ip_bits(*b)))
        }
        (TypedValue::Ip(a), Value::Cidr(net, bits)) => ip_in_cidr(*a, *net, *bits),
        (TypedValue::Family(v6), Value::Family(want)) => v6 == want,
        (TypedValue::Cidr(a, abits), Value::Cidr(b, bbits)) => match c.field {
            // The token's block is inside the value's: at least as long a
            // prefix, and the same network under the value's prefix.
            Field::In => abits >= bbits && ip_in_cidr(*a, *b, *bbits),
            _ => match c.op {
                Op::Eq | Op::Colon => a == b && abits == bbits,
                Op::Ne => !(a == b && abits == bbits),
                _ => false,
            },
        },
        (TypedValue::Cidr(net, bits), Value::Ip(ip)) if c.field == Field::Contains => {
            ip_in_cidr(*ip, *net, *bits)
        }
        (TypedValue::Version(a), Value::Version(b)) => c.op.holds(a.compare(b)),
        (TypedValue::Text(a), Value::Text(b)) => {
            let fold = c.field.folds();
            match c.op {
                Op::Colon => glob_match(b, a, fold),
                Op::Eq => text_eq(a, b, fold),
                Op::Ne => !text_eq(a, b, fold),
                _ => false,
            }
        }
        _ => false,
    }
}

fn text_eq(a: &str, b: &str, fold: bool) -> bool {
    if fold { a.eq_ignore_ascii_case(b) } else { a == b }
}

/// Whether a clause reading decoded content holds. The content is decoded
/// here, which is the first point a clause needs it and the only point any
/// of them does: the views are shared and each is read at most once.
fn decoded_holds(
    kind: TokenKind,
    c: &Clause,
    text: &str,
    clock: &mut dyn FnMut() -> Clock,
    decode: &mut Decode,
) -> bool {
    let Some(content) = decode.content(kind, c.field, text) else {
        return false;
    };
    match c.field {
        Field::Match => {
            let Value::Sub(sub) = &c.value else {
                return false;
            };
            // A literal every match of the sub-pattern must hold, absent from
            // the content, settles the clause without the engine running.
            let found = !crate::prefilter::requires_absent(&sub.pattern, content)
                && crate::cursor::find(&sub.pattern, content).is_some();
            if c.op == Op::Ne { !found } else { found }
        }
        Field::Bits => num_holds(c, &crate::decoded::bits_per_byte(content)),
        Field::Period => {
            num_holds(c, &Decimal::from_u128(u128::from(crate::decoded::period(content))))
        }
        Field::Texture => text_holds(c, crate::decoded::texture(content).label()),
        Field::Text => text_holds(c, &String::from_utf8_lossy(content)),
        Field::Header | Field::Claim => {
            if c.key.is_empty() {
                return text_holds(c, &String::from_utf8_lossy(content));
            }
            match crate::decoded::member(content, &c.key) {
                Some(m) => claim_holds(c, &m, clock),
                None => false,
            }
        }
        _ => false,
    }
}

/// Whether a clause comparing a number holds of one read from content.
fn num_holds(c: &Clause, got: &Decimal) -> bool {
    match &c.value {
        Value::Num(want) => c.op.holds(got.compare(want)),
        Value::Set(set) => {
            let held = set.holds_num(got);
            if c.op == Op::Ne { !held } else { held }
        }
        _ => false,
    }
}

/// Whether a clause comparing text holds of text read from content. Decoded
/// content carries no case rule of its own, so nothing here folds.
fn text_holds(c: &Clause, got: &str) -> bool {
    match &c.value {
        Value::Text(want) => match c.op {
            Op::Colon => glob_match(want, got, false),
            Op::Eq => text_eq(got, want, false),
            Op::Ne => !text_eq(got, want, false),
            _ => false,
        },
        Value::Set(set) => {
            let held = set.holds_text(got);
            if c.op == Op::Ne { !held } else { held }
        }
        _ => false,
    }
}

/// Whether a clause holds of a claim's value, compared as what the claim
/// holds: a NumericDate against an instant or the clock, a number against a
/// number - including a number a token wrote as a string - and anything else
/// as its text.
fn claim_holds(c: &Clause, m: &crate::decoded::Json, clock: &mut dyn FnMut() -> Clock) -> bool {
    use crate::decoded::Json;
    let seconds = |v: &Decimal, secs: i64| c.op.holds(v.compare(&Decimal::from_i64(secs)));
    match (&c.value, m) {
        (Value::Now, Json::Num(n)) => match Decimal::parse(n) {
            Some(v) => seconds(&v, clock().now_secs),
            None => false,
        },
        (Value::Instant(civil), Json::Num(n)) => {
            let now = clock();
            match (civil.epoch(now), Decimal::parse(n)) {
                (Some((secs, _)), Some(v)) => seconds(&v, secs),
                _ => false,
            }
        }
        (Value::Num(want), Json::Num(n) | Json::Str(n)) => match Decimal::parse(n) {
            Some(v) => c.op.holds(v.compare(want)),
            None => false,
        },
        _ => text_holds(c, &m.text()),
    }
}

/// The signed difference `later - earlier` in nanoseconds, exact.
fn nanos_between(later: (i64, u32), earlier: (i64, u32)) -> Decimal {
    let a = i128::from(later.0) * 1_000_000_000 + i128::from(later.1);
    let b = i128::from(earlier.0) * 1_000_000_000 + i128::from(earlier.1);
    let d = a - b;
    Decimal::normalized(d < 0, &d.unsigned_abs().to_string(), "")
}

/// Whether the instant `txt` is the written distance from the instant
/// `bound`: their difference, in nanoseconds, compared against a signed
/// duration. Text that does not read as an instant, and an instant the clock
/// cannot place, satisfy nothing.
#[must_use]
pub fn since_holds(
    op: crate::ast::Cmp,
    signed: &crate::ast::Signed,
    bound: &[u8],
    txt: &[u8],
    clock: Clock,
) -> bool {
    let (Some(a), Some(b)) = (
        parse_civil(&String::from_utf8_lossy(bound)),
        parse_civil(&String::from_utf8_lossy(txt)),
    ) else {
        return false;
    };
    let (Some(from), Some(here)) = (a.epoch(clock), b.epoch(clock)) else {
        return false;
    };
    let want =
        if signed.negative { signed.nanos.negated() } else { signed.nanos.clone() };
    op.holds(nanos_between(here, from).compare(&want))
}

/// `now - then` in nanoseconds, as an exact decimal; negative for a time
/// ahead of the clock.
fn elapsed_nanos(now_secs: i64, secs: i64, nanos: u32) -> Decimal {
    let whole = i128::from(now_secs) - i128::from(secs);
    let total = whole * 1_000_000_000 - i128::from(nanos);
    Decimal::normalized(total < 0, &total.unsigned_abs().to_string(), "")
}

impl Decimal {
    /// This value as a float, or `None` where it is outside a float's range,
    /// too large for one to tell from infinity or too small to tell from zero.
    ///
    /// Lossy by nature - the decimal is exact and the float is not - so it
    /// is for a caller comparing against a summary that is itself
    /// approximate, never for deciding a match.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        if self.is_zero() {
            return Some(0.0);
        }
        let sign = if self.neg { "-" } else { "" };
        match format!("{sign}{}e{}", self.digits, self.exp).parse::<f64>() {
            Ok(v) if v.is_finite() && v != 0.0 => Some(v),
            Ok(_past_a_float) => None,
            Err(_not_a_float) => None,
        }
    }

    /// The float nearest this value: infinity past a float's range and zero
    /// below it. The rounding never reverses an order, so the floats of a
    /// file's values bound the floats of every value it holds, which is what
    /// a summary that refuses only certain misses needs.
    #[must_use]
    pub fn nearest_f64(&self) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        let sign = if self.neg { "-" } else { "" };
        match format!("{sign}{}e{}", self.digits, self.exp).parse::<f64>() {
            Ok(v) => v,
            Err(not_a_float) => unreachable!("digits and an exponent read as a float: {not_a_float}"),
        }
    }

    /// The base-ten logarithm of this value's magnitude, or `None` for zero.
    /// The leading seventeen digits are read as an integer and the places
    /// after them added to its logarithm, so it stays accurate at any power.
    #[must_use]
    pub fn log10(&self) -> Option<f64> {
        if self.is_zero() {
            return None;
        }
        let take = self.digits.len().min(17);
        let head = self.digits.bytes().take(take).fold(0u64, |a, b| a * 10 + u64::from(b - b'0'));
        Some((head as f64).log10() + (self.digits.len() - take) as f64 + self.exp as f64)
    }

    /// The order of magnitude: the power of ten of the leading digit, or
    /// `None` for zero.
    #[must_use]
    pub fn order(&self) -> Option<i64> {
        if self.is_zero() {
            return None;
        }
        self.exp.checked_add(self.digits.len() as i64 - 1)
    }

    /// The value times ten to the `by`, exact.
    #[must_use]
    pub fn shift(&self, by: i32) -> Decimal {
        if self.is_zero() {
            return self.clone();
        }
        Decimal { neg: self.neg, digits: self.digits.clone(), exp: self.exp + i64::from(by) }
    }
}

/// A number in any notation a document writes: thousands separators,
/// underscores, a currency or percent sign, a `0x`, `0b` or `0o` prefix, an
/// exponent.
#[must_use]
pub fn lenient_decimal(s: &str) -> Option<Decimal> {
    let cleaned: String =
        s.trim().chars().filter(|c| !matches!(c, ',' | '_' | '$' | '%')).collect();
    number_value(&cleaned)
}

/// The value of a number written as the lexer reads one, a sign allowed
/// before it: decimal digits with `_` between them, then a fraction and an
/// exponent (`1_000.5e-3`), or `0x`, `0b` or `0o` and digits of that radix
/// with `_` between them (`0xFFFF_FFFF`). Exact at any length. `None` for
/// text that is not such a number, and for an exponent of more than eighteen
/// digits, which is past the power of ten a [`Decimal`] holds.
#[must_use]
pub fn number_value(s: &str) -> Option<Decimal> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let radix = match body.as_bytes() {
        [b'0', b'x' | b'X', ..] => 16,
        [b'0', b'b' | b'B', ..] => 2,
        [b'0', b'o' | b'O', ..] => 8,
        _ => 10,
    };
    if radix != 10 {
        let digits = joined_digits(&body[2..], radix)?;
        return Some(Decimal::scaled(neg, &radix_to_decimal(&digits, radix), 0));
    }
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (&body[..at], parse_exponent(&body[at + 1..])?),
        None => (body, 0),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    let decimal = |part: &str| -> Option<String> {
        if part.is_empty() {
            return Some(String::new());
        }
        Some(joined_digits(part, 10)?.iter().map(|d| char::from(b'0' + d)).collect())
    };
    let (int, frac) = (decimal(int)?, decimal(frac)?);
    Some(Decimal::scaled(neg, &format!("{int}{frac}"), exponent - frac.len() as i64))
}

/// The digit values of `s` in `radix`, most significant first, with one `_`
/// allowed between two digits; `None` where `s` holds anything else or no
/// digit at all.
fn joined_digits(s: &str, radix: u32) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    let mut after_digit = false;
    for c in s.chars() {
        if c == '_' {
            if !after_digit {
                return None;
            }
            after_digit = false;
            continue;
        }
        out.push(c.to_digit(radix)? as u8);
        after_digit = true;
    }
    after_digit.then_some(out)
}

/// An exponent: an optional sign, then decimal digits, at most eighteen of
/// them once leading zeros are set aside.
fn parse_exponent(s: &str) -> Option<i64> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let significant = digits.trim_start_matches('0');
    if significant.len() > 18 {
        return None;
    }
    let v = significant.bytes().fold(0i64, |a, b| a * 10 + i64::from(b - b'0'));
    Some(if neg { -v } else { v })
}

/// The decimal digits of the number whose digit values in `radix` are
/// `digits`, most significant first, exact at any length. The value is built
/// in limbs of nine decimal digits, taking a step as many radix digits as fit
/// a 32-bit multiplier, so a step is one multiply-add a limb.
fn radix_to_decimal(digits: &[u8], radix: u32) -> String {
    const LIMB: u64 = 1_000_000_000;
    let per_step = match radix {
        16 => 7,
        8 => 10,
        2 => 31,
        _ => 1,
    };
    // Least significant limb first.
    let mut limbs: Vec<u64> = Vec::new();
    for step in digits.chunks(per_step) {
        let (mut scale, mut add) = (1u64, 0u64);
        for &d in step {
            scale *= u64::from(radix);
            add = add * u64::from(radix) + u64::from(d);
        }
        let mut carry = add;
        for limb in &mut limbs {
            let v = *limb * scale + carry;
            *limb = v % LIMB;
            carry = v / LIMB;
        }
        while carry > 0 {
            limbs.push(carry % LIMB);
            carry /= LIMB;
        }
    }
    match limbs.split_last() {
        None => "0".to_string(),
        Some((top, rest)) => {
            let mut s = top.to_string();
            for limb in rest.iter().rev() {
                s.push_str(&format!("{limb:09}"));
            }
            s
        }
    }
}

/// The canonical text of a number under the `numeric` orbit: `1,000`, `1e3`,
/// `1000.0` and `0x3e8` are one. Text that is not a number is itself.
#[must_use]
pub fn canonical_number(s: &str) -> String {
    match lenient_decimal(s) {
        Some(d) => d.to_text(),
        None => s.to_string(),
    }
}

/// A timestamp as the instant it names, so two forms of one moment fold:
/// seconds since the epoch and nanoseconds; a time with no date as seconds into
/// its day. Text that is not a timestamp is itself.
#[must_use]
pub fn canonical_time(s: &str) -> String {
    let Some(civil) = parse_civil(s) else {
        return s.to_string();
    };
    if civil.has_date {
        match civil.epoch(Clock::current()) {
            Some((secs, nanos)) => format!("{secs}.{nanos:09}"),
            None => s.to_string(),
        }
    } else {
        match civil.seconds_of_day() {
            Some((secs, nanos)) => format!("t{secs}.{nanos:09}"),
            None => s.to_string(),
        }
    }
}

/// Text with compatibility forms decomposed, combining marks removed and case
/// folded: `Café`, `cafe` and the fullwidth `Ｃａｆé` are one.
#[must_use]
pub fn fold_text(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

fn hex_val(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// Percent-encodings with their hex uppercased, and an encoded unreserved
/// character (a letter, a digit, `-` `.` `_` `~`) decoded.
fn normalize_pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if let Some(hex) = rest.strip_prefix('%')
            && hex.len() >= 2
            && let (Some(h), Some(l)) = (hex_val(hex.as_bytes()[0]), hex_val(hex.as_bytes()[1]))
        {
            let v = (h << 4) | l;
            if v.is_ascii_alphanumeric() || matches!(v, b'-' | b'.' | b'_' | b'~') {
                out.push(char::from(v));
            } else {
                out.push('%');
                out.push_str(&hex[..2].to_ascii_uppercase());
            }
            i += 3;
            continue;
        }
        let c = rest.chars().next().expect("a non-empty remainder begins with a character");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// A URL in the form RFC 3986's syntax-based normalization gives it: the
/// scheme and host lowercased, a port equal to the scheme's default dropped,
/// an empty path read as `/`, percent-encoding hex uppercased and unreserved
/// characters decoded. The query, the fragment and a trailing slash stay as
/// written. Text that is not a URL is itself.
#[must_use]
pub fn canonical_url(s: &str) -> String {
    let Some((scheme, rest)) = s.split_once("://") else {
        return s.to_string();
    };
    let scheme = scheme.to_ascii_lowercase();
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, after) = rest.split_at(auth_end);
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => (h, Some(p)),
        _ => (hostport, None),
    };
    let default_port: Option<i64> = match scheme.as_str() {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        "ssh" | "sftp" => Some(22),
        "smtp" => Some(25),
        _ => None,
    };
    let port = port.filter(|p| parse_int(p) != default_port);
    let (path, tail) = match after.find(['?', '#']) {
        Some(i) => after.split_at(i),
        None => (after, ""),
    };
    let path = if path.is_empty() { "/".to_string() } else { normalize_pct(path) };
    let mut out = format!("{scheme}://");
    if let Some(u) = userinfo {
        out.push_str(u);
        out.push('@');
    }
    out.push_str(&host.to_ascii_lowercase());
    if let Some(p) = port {
        out.push(':');
        out.push_str(p);
    }
    out.push_str(&path);
    out.push_str(&normalize_pct(tail));
    out
}

/// A relation a back-reference holds under: the later token equals the bound
/// one after both are projected through a typed field, so `=domain e` is the
/// same mail domain and `=subnet a` the same network.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Relation {
    /// The same network under a prefix length: the one written after a
    /// slash, or the family's default, /24 for IPv4 and /64 for IPv6.
    Subnet(Option<u8>),
    /// The same address family.
    Family,
    /// The same mail domain.
    Domain,
    /// The same mail user.
    User,
    /// The same URL host.
    Host,
    /// The same URL port, a scheme's default counted.
    Port,
    /// The same calendar day as written, each timestamp in its own zone.
    Day,
    /// The same day and hour as written.
    Hour,
    /// The same month as written.
    Month,
    /// The same year as written.
    Year,
    /// The same major version.
    Major,
    /// The same major and minor version.
    Minor,
    /// The same major, minor and patch.
    Patch,
    /// The same order of magnitude.
    Magnitude,
    /// The same card issuer.
    Issuer,
    /// The same last four card digits.
    Last4,
    /// The same country calling code.
    Cc,
    /// The same path extension.
    Ext,
    /// The same directory.
    Dir,
    /// The same file name.
    Name,
    /// The same first three octets of a MAC address.
    Oui,
    /// The same digest algorithm.
    Algo,
    /// The same length in characters.
    Len,
    /// The same CIDR prefix length.
    Prefix,
}

impl Relation {
    /// The relation a name denotes, or `None` when it names none.
    #[must_use]
    pub fn parse(name: &str) -> Option<Relation> {
        Some(match name {
            "subnet" => Relation::Subnet(None),
            "family" => Relation::Family,
            "domain" => Relation::Domain,
            "user" => Relation::User,
            "host" => Relation::Host,
            "port" => Relation::Port,
            "day" => Relation::Day,
            "hour" => Relation::Hour,
            "month" => Relation::Month,
            "year" => Relation::Year,
            "major" => Relation::Major,
            "minor" => Relation::Minor,
            "patch" => Relation::Patch,
            "magnitude" => Relation::Magnitude,
            "issuer" => Relation::Issuer,
            "last4" => Relation::Last4,
            "cc" => Relation::Cc,
            "ext" => Relation::Ext,
            "dir" => Relation::Dir,
            "name" => Relation::Name,
            "oui" => Relation::Oui,
            "algo" => Relation::Algo,
            "len" => Relation::Len,
            "prefix" => Relation::Prefix,
            _ => return None,
        })
    }

    /// The relation with a prefix length, for the one that takes one.
    #[must_use]
    pub fn with_prefix(self, bits: u8) -> Option<Relation> {
        match self {
            Relation::Subnet(_) => Some(Relation::Subnet(Some(bits))),
            _ => None,
        }
    }

    /// The relation's name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Relation::Subnet(_) => "subnet",
            Relation::Family => "family",
            Relation::Domain => "domain",
            Relation::User => "user",
            Relation::Host => "host",
            Relation::Port => "port",
            Relation::Day => "day",
            Relation::Hour => "hour",
            Relation::Month => "month",
            Relation::Year => "year",
            Relation::Major => "major",
            Relation::Minor => "minor",
            Relation::Patch => "patch",
            Relation::Magnitude => "magnitude",
            Relation::Issuer => "issuer",
            Relation::Last4 => "last4",
            Relation::Cc => "cc",
            Relation::Ext => "ext",
            Relation::Dir => "dir",
            Relation::Name => "name",
            Relation::Oui => "oui",
            Relation::Algo => "algo",
            Relation::Len => "len",
            Relation::Prefix => "prefix",
        }
    }

    /// The kind and field both sides are read through, for a relation that
    /// is a plain field equality.
    fn projection(self) -> Option<(TokenKind, Field)> {
        Some(match self {
            Relation::Family => (TokenKind::Ip, Field::Family),
            Relation::Domain => (TokenKind::Email, Field::Domain),
            Relation::User => (TokenKind::Email, Field::User),
            Relation::Host => (TokenKind::Url, Field::Host),
            Relation::Port => (TokenKind::Url, Field::Port),
            Relation::Major => (TokenKind::Version, Field::Major),
            Relation::Issuer => (TokenKind::CreditCard, Field::Issuer),
            Relation::Last4 => (TokenKind::CreditCard, Field::Last4),
            Relation::Cc => (TokenKind::Phone, Field::Cc),
            Relation::Ext => (TokenKind::Path, Field::Ext),
            Relation::Dir => (TokenKind::Path, Field::Dir),
            Relation::Name => (TokenKind::Path, Field::Name),
            Relation::Oui => (TokenKind::Mac, Field::Oui),
            Relation::Algo => (TokenKind::HashDigest, Field::Algo),
            Relation::Prefix => (TokenKind::Cidr, Field::Prefix),
            Relation::Subnet(_)
            | Relation::Day
            | Relation::Hour
            | Relation::Month
            | Relation::Year
            | Relation::Minor
            | Relation::Patch
            | Relation::Magnitude
            | Relation::Len => return None,
        })
    }

    /// The value this relation reads a text at: two texts are in the
    /// relation exactly when their keys are equal, so one projection serves
    /// the back-reference, the orbit rung and every table keyed on either.
    /// `None` where the text is not of the type the relation reads.
    ///
    /// The calendar relations are the exception and are keyed by
    /// [`calendar_key`], which is finer than [`related`](Self::related)
    /// because their pairwise reading is no equivalence.
    #[must_use]
    pub fn key(self, text: &str) -> Option<String> {
        match self {
            Relation::Subnet(bits) => {
                let ip = parse_ip(text)?;
                let width = match bits {
                    Some(w) => w,
                    None if ip.is_ipv6() => 64,
                    None => 24,
                };
                Some(format!("{}/{width}", mask_ip(ip, width)))
            }
            Relation::Day | Relation::Hour | Relation::Month | Relation::Year => {
                calendar_key(self, &parse_civil(text)?)
            }
            Relation::Minor | Relation::Patch => {
                let v = Version::parse(text)?;
                let parts = if self == Relation::Minor { 2 } else { 3 };
                let mut out = String::new();
                for i in 0..parts {
                    if i > 0 {
                        out.push('.');
                    }
                    out.push_str(&read_key(&TypedValue::Num(v.part(i)), false));
                }
                Some(out)
            }
            // Two zeros have no order of magnitude and are one value here, so
            // the absent order is a value of its own rather than no key.
            Relation::Magnitude => Some(
                lenient_decimal(text)?.order().map_or_else(|| "zero".to_string(), |o| o.to_string()),
            ),
            Relation::Len => Some(text.chars().count().to_string()),
            _ => {
                let (kind, field) = self.projection()?;
                if !lexes_as(kind, text) {
                    return None;
                }
                Some(read_key(&read_field(kind, kind, field, text)?, field.folds()))
            }
        }
    }

    /// Whether the token with text `txt` is in this relation to the bound
    /// text.
    #[must_use]
    pub fn related(self, bound: &[u8], txt: &[u8]) -> bool {
        let a = String::from_utf8_lossy(bound);
        let b = String::from_utf8_lossy(txt);
        if matches!(self, Relation::Day | Relation::Hour | Relation::Month | Relation::Year) {
            let (Some(x), Some(y)) = (parse_civil(&a), parse_civil(&b)) else {
                return false;
            };
            return same_calendar(self, &x, &y);
        }
        match (self.key(&a), self.key(&b)) {
            (Some(p), Some(q)) => p == q,
            _ => false,
        }
    }
}

/// Whether two timestamps share the calendar fields the relation names, as
/// written; a year one of them leaves out is not held against it.
fn same_calendar(rel: Relation, x: &Civil, y: &Civil) -> bool {
    if !x.has_date || !y.has_date {
        return false;
    }
    let years_agree = match (x.year, y.year) {
        (Some(p), Some(q)) => p == q,
        _ => true,
    };
    match rel {
        Relation::Year => matches!((x.year, y.year), (Some(p), Some(q)) if p == q),
        Relation::Month => years_agree && x.month == y.month,
        Relation::Day => years_agree && x.month == y.month && x.day == y.day,
        _ => {
            years_agree
                && x.month == y.month
                && x.day == y.day
                && x.has_time
                && y.has_time
                && x.hour == y.hour
        }
    }
}

/// Whether `text` is one token of `kind` and nothing besides.
///
/// A relation's projection is written for a token the lexer already read as
/// that kind, and several of the field readers answer for any text at all:
/// an address with no `@` has an empty domain and `card_issuer` names every
/// text `unknown`, so without this a document's whole vocabulary would fold
/// onto one key and every token would relate to every other.
fn lexes_as(kind: TokenKind, text: &str) -> bool {
    let toks = crate::lexer::lex(text.as_bytes());
    let mut significant = toks.iter().filter(|t| t.is_significant());
    match (significant.next(), significant.next()) {
        (Some(t), None) => t.kind == kind && t.start() == 0 && t.end() == text.len(),
        _ => false,
    }
}

/// One field reading as the text that decides its equality, folded where the
/// field folds.
///
/// Every arm renders what the field's comparison actually looks at rather
/// than the token's bytes: a number through its normalized decimal, so `1e3`
/// and `1000` key alike, and a folding field in lower case, so `Example.com`
/// and `example.com` do. Two readings are equal exactly when these are, so a
/// relation and a key over the same field cannot come to disagree.
fn read_key(v: &TypedValue, fold: bool) -> String {
    match v {
        // Rendered as zero whichever sign it was written with, because the
        // comparison treats the two zeros as one value and the text does not.
        TypedValue::Num(d) if d.is_zero() => "0".to_string(),
        TypedValue::Num(d) => d.to_text(),
        TypedValue::Text(s) if fold => s.to_ascii_lowercase(),
        TypedValue::Text(s) => s.clone(),
        TypedValue::Ip(ip) => ip.to_string(),
        TypedValue::Cidr(ip, bits) => format!("{ip}/{bits}"),
        TypedValue::Family(v6) => if *v6 { "v6" } else { "v4" }.to_string(),
        TypedValue::Version(v) => v.to_text(),
        TypedValue::Instant(c) => format!("{c:?}"),
        TypedValue::Quantity(family, base) => format!("{} {}", base.to_text(), family.name()),
    }
}

/// The calendar fields a relation names, as written, with a year the text
/// does not write kept as a value of its own.
///
/// Finer than [`same_calendar`], deliberately: that reading does not hold an
/// absent year against a written one, which relates `Sep 15` to two years
/// that do not relate to each other, so it is no equivalence and has no key.
/// A key that dropped the year instead would fold those two years together
/// and a join across inputs would report a match that is not one.
fn calendar_key(rel: Relation, c: &Civil) -> Option<String> {
    if !c.has_date {
        return None;
    }
    let (month, day, hour) = (c.month, c.day, c.hour);
    let year = match c.year {
        Some(y) => y.to_string(),
        // A year the text never wrote, and never the spelling of one.
        None => "....".to_string(),
    };
    Some(match rel {
        // `=year` holds only between two timestamps that both write one, so a
        // timestamp without a year has no key here rather than a marked one.
        Relation::Year => c.year?.to_string(),
        Relation::Month => format!("{year}-{month:02}"),
        Relation::Day => format!("{year}-{month:02}-{day:02}"),
        _ => {
            if !c.has_time {
                return None;
            }
            format!("{year}-{month:02}-{day:02}T{hour:02}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pred(kind: TokenKind, body: &str) -> TypedPred {
        TypedPred::parse(kind, body).unwrap_or_else(|e| panic!("{body}: {e}"))
    }

    /// The clock every test reads: 2026-09-15T00:00:00Z, UTC.
    const NOW: i64 = 1_789_430_400;

    fn holds(kind: TokenKind, body: &str, txt: &str) -> bool {
        pred(kind, body).matches(txt.as_bytes(), || Clock::fixed(NOW, 0))
    }

    fn d(s: &str) -> Decimal {
        Decimal::parse(s).unwrap_or_else(|| panic!("{s} is a decimal"))
    }

    #[test]
    fn decimals_compare_exactly_at_any_length() {
        assert_eq!(d("500").compare(&d("499.999")), Ordering::Greater);
        assert_eq!(d("0500.0").compare(&d("500")), Ordering::Equal);
        assert_eq!(d("-1").compare(&d("0.5")), Ordering::Less);
        assert_eq!(d("-2").compare(&d("-1.5")), Ordering::Less);
        let long = "9".repeat(60);
        assert_eq!(d(&long).compare(&d("1000")), Ordering::Greater);
        assert_eq!(d(&format!("{long}.1")).compare(&d(&long)), Ordering::Greater);
        assert_eq!(d("1.5").times(1024).to_text(), "1536");
        assert_eq!(d("0.25").times(1000).to_text(), "250");
        assert_eq!(d("2.5").times(1_000_000_000).to_text(), "2500000000");
        assert_eq!(d("0.0005").times(1000).to_text(), "0.5");
        let sum = |a: &str, b: &str| add_positive(&d(a), &d(b)).unwrap_or_else(|| panic!("{a} + {b} sums")).to_text();
        assert_eq!(sum("1.5", "2.75"), "4.25");
        assert_eq!(sum("999", "1"), "1000");
        assert!(Decimal::parse("1,000").is_none());
        assert!(Decimal::parse("").is_none());
        assert_eq!(parse_int("-42"), Some(-42));
        assert_eq!(parse_int("99999999999999999999"), None);
    }

    #[test]
    fn a_number_reads_its_value_from_every_form_the_lexer_reads_whole() {
        let v = |s: &str| number_value(s).unwrap_or_else(|| panic!("{s} reads")).to_text();
        assert_eq!(v("1000"), "1000");
        assert_eq!(v("1e3"), "1000");
        assert_eq!(v("1E3"), "1000");
        assert_eq!(v("2.5e-4"), "0.00025");
        assert_eq!(v("6.02e23"), "602000000000000000000000");
        assert_eq!(v("1e+10"), "10000000000");
        assert_eq!(v("0x3e8"), "1000");
        assert_eq!(v("0X3E8"), "1000");
        assert_eq!(v("0b1111101000"), "1000");
        assert_eq!(v("0o1750"), "1000");
        assert_eq!(v("0xFFFF_FFFF"), "4294967295");
        assert_eq!(v("1_000_000"), "1000000");
        assert_eq!(v("12_345.75"), "12345.75");
        assert_eq!(v("1_000.5e-3"), "1.0005");
        // A radix literal past 128 bits reads exactly: 16^40 is 2^160.
        assert_eq!(v(&format!("0x1{}", "0".repeat(40))), "1461501637330902918203684832716283019655932542976");
        for none in ["", "e3", "1e", "0x", "0x_1", "0b2", "1__0", "_1", "1_", "1e3a", "1,000", "abc"] {
            assert_eq!(number_value(none), None, "{none:?}");
        }
        // An exponent of nineteen digits is past the power a value holds.
        assert_eq!(number_value("1e1000000000000000000"), None);
    }

    #[test]
    fn a_value_is_written_out_until_more_than_twenty_one_zeros_would_follow_its_digits() {
        let v = |s: &str| number_value(s).unwrap_or_else(|| panic!("{s} reads")).to_text();
        assert_eq!(v("1e21"), format!("1{}", "0".repeat(21)));
        assert_eq!(v("1e22"), "1e22");
        assert_eq!(v("2.5e30"), "2.5e30");
        assert_eq!(v("-1.25e999999"), "-1.25e999999");
        assert_eq!(v("1e-22"), format!("0.{}1", "0".repeat(21)));
        assert_eq!(v("1e-23"), "1e-23");
        assert_eq!(v("1.5e-30"), "1.5e-30");
        // Zeros between digits are digits and are written out.
        assert_eq!(v(&format!("1{}1", "0".repeat(40))), format!("1{}1", "0".repeat(40)));
        assert_eq!(v("0"), "0");
        assert_eq!(v("0e999"), "0");
    }

    #[test]
    fn a_value_far_past_a_float_compares_and_adds_exactly() {
        let n = |s: &str| number_value(s).unwrap_or_else(|| panic!("{s} reads"));
        assert_eq!(n("1e999999").compare(&n("9e999998")), Ordering::Greater);
        assert_eq!(n("1e999999").compare(&n("10e999998")), Ordering::Equal);
        assert_eq!(n("1e999999"), n("10e999998"), "one value is one key");
        assert_eq!(n("1e-999999").compare(&n("0")), Ordering::Greater);
        assert_eq!(n("-1e999999").compare(&n("1")), Ordering::Less);
        let sum = |a: &str, b: &str| n(a).add(&n(b)).unwrap_or_else(|| panic!("{a} + {b} sums")).to_text();
        assert_eq!(sum("1e3", "1e-3"), "1000.001");
        assert_eq!(sum("6.02e23", "-6.02e23"), "0");
        assert_eq!(sum("1e25", "1"), format!("1{}1", "0".repeat(24)));
        // A million places apart is summed exactly; one place further is not.
        assert_eq!(sum("1e1000000", "1").len(), 1_000_001);
        assert_eq!(n("1e1000001").add(&n("1")), None);
        assert_eq!(n("1e1000001").add(&n("0")), Some(n("1e1000001")), "zero adds nothing to line up");
        assert_eq!(n("1e999999").shift(-999_996).to_text(), "1000");
        assert_eq!(n("1e999999").order(), Some(999_999));
        assert_eq!(n("2.5e3").as_f64(), Some(2500.0));
        assert_eq!(n("1e400").as_f64(), None);
        assert_eq!(n("1e-400").as_f64(), None);
        assert_eq!(n("1e400").nearest_f64(), f64::INFINITY);
        assert_eq!(n("1e-400").nearest_f64(), 0.0);
    }

    #[test]
    fn the_calendar_round_trips() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(2000, 3, 1), Some(11_017));
        assert_eq!(days_from_civil(2026, 1, 1), Some(20_454));
        assert_eq!(days_from_civil(2026, 2, 29), None);
        assert_eq!(days_from_civil(2024, 2, 29), Some(19_782));
        for days in [-1_000_000, -1, 0, 1, 11_017, 20_454, 3_000_000] {
            let (y, m, day) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, day), Some(days), "{days}");
        }
    }

    #[test]
    fn every_timestamp_form_reads() {
        let clock = Clock::fixed(NOW, 0);
        let epoch = |s: &str| parse_civil(s).unwrap_or_else(|| panic!("{s}")).epoch(clock);
        assert_eq!(epoch("2026-01-01"), Some((1_767_225_600, 0)));
        assert_eq!(epoch("2026-01-01T00:00:00Z"), Some((1_767_225_600, 0)));
        assert_eq!(epoch("2026-01-01T02:00:00+02:00"), Some((1_767_225_600, 0)));
        assert_eq!(epoch("2026-01-01T02:00:00+0200"), Some((1_767_225_600, 0)));
        assert_eq!(epoch("2026-01-01 00:00:00"), Some((1_767_225_600, 0)));
        assert_eq!(epoch("2026-01-01t00:00:00.250z"), Some((1_767_225_600, 250_000_000)));
        assert_eq!(epoch("01/Jan/2026:00:00:00 +0000"), Some((1_767_225_600, 0)));
        assert_eq!(epoch("01/Jan/2026:01:00:00 +0100"), Some((1_767_225_600, 0)));
        let plus_two = Clock::fixed(NOW, 7200);
        assert_eq!(
            parse_civil("2026-01-01T02:00:00").unwrap_or_else(|| panic!("reads")).epoch(plus_two),
            Some((1_767_225_600, 0))
        );
        // syslog takes the clock's year, 2026 here.
        assert_eq!(epoch("Jan  1 00:00:00"), Some((1_767_225_600, 0)));
        assert_eq!(epoch("Sep 15 00:00:00"), Some((NOW, 0)));
        // A syslog date more than a day past the clock is last year's.
        let (secs, _) = epoch("Dec 31 23:59:59").unwrap_or_else(|| panic!("places"));
        assert_eq!(civil_from_days(secs.div_euclid(86_400)).0, 2025);
        let clock_time = parse_civil("12:30").unwrap_or_else(|| panic!("reads"));
        assert_eq!(clock_time.seconds_of_day(), Some((45_000, 0)));
        assert!(clock_time.epoch(clock).is_none());
        for bad in ["2026-13-01", "2026-02-30", "25:00", "12:60", "Sep 32 00:00:00", "2026-01-01X"] {
            assert!(parse_civil(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn versions_order_as_semver_does() {
        let v = |s: &str| Version::parse(s).unwrap_or_else(|| panic!("{s} is a version"));
        assert_eq!(v("2.0").compare(&v("2.0.0")), Ordering::Equal);
        assert_eq!(v("v1.2.3").compare(&v("1.2.10")), Ordering::Less);
        assert_eq!(v("1.0.0-alpha").compare(&v("1.0.0")), Ordering::Less);
        assert_eq!(v("1.0.0-alpha.1").compare(&v("1.0.0-alpha.beta")), Ordering::Less);
        assert_eq!(v("1.0.0-beta.2").compare(&v("1.0.0-beta.11")), Ordering::Less);
        assert_eq!(v("1.0.0-rc.1").compare(&v("1.0.0-rc.1+build.5")), Ordering::Equal);
        assert_eq!(v("3").compare(&v("2.99.99")), Ordering::Greater);
    }

    #[test]
    fn globs_and_addresses() {
        assert!(glob_match("*.internal", "db.corp.internal", true));
        assert!(glob_match("*.INTERNAL", "db.corp.internal", true));
        assert!(!glob_match("*.INTERNAL", "db.corp.internal", false));
        assert!(glob_match("api-v?", "api-v2", false));
        assert!(!glob_match("api-v?", "api-v22", false));
        assert!(glob_match("*", "", false));
        assert!(glob_match("", "", false));
        assert!(glob_match("a*b*c", "axxbyyc", false));
        assert!(!glob_match("a*b*c", "axxbyy", false));
        assert_eq!(parse_ip("10.0.0.1"), Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert_eq!(parse_ip("::1"), Some(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert_eq!(
            parse_ip("2001:db8::ff00:42:8329"),
            Some(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0xff00, 0x42, 0x8329)))
        );
        assert_eq!(
            parse_ip("2001:0db8:85a3:0000:0000:8a2e:0370:7334"),
            Some(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0x85a3, 0, 0, 0x8a2e, 0x370, 0x7334)))
        );
        assert_eq!(parse_ip("fe80::"), Some(IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0))));
        for bad in ["256.1.1.1", "1.2.3", "1::2::3", "12345::1", "abcd", "1.2.3.4.5"] {
            assert!(parse_ip(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn numbers_sizes_durations_money_percent() {
        assert!(holds(TokenKind::Number, ">500", "501"));
        assert!(!holds(TokenKind::Number, ">500", "500"));
        assert!(holds(TokenKind::Number, "500..599", "503"));
        assert!(!holds(TokenKind::Number, "500..599", "600"));
        assert!(holds(TokenKind::Number, ">=1.5,<2", "1.75"));
        assert!(holds(TokenKind::Number, "!=200", "404"));
        assert!(holds(TokenKind::Number, "=200", "200.0"));
        assert!(holds(TokenKind::ByteSize, ">1GiB", "1.5GiB"));
        assert!(!holds(TokenKind::ByteSize, ">1GiB", "1GB"));
        assert!(holds(TokenKind::ByteSize, ">1000MB", "1.5GB"));
        assert!(holds(TokenKind::ByteSize, "=1024", "1KiB"));
        assert!(holds(TokenKind::Duration, ">500ms", "2.5s"));
        assert!(!holds(TokenKind::Duration, "<1h", "3h20m"));
        assert!(holds(TokenKind::Duration, ">=200m", "3h20m"));
        assert!(holds(TokenKind::Duration, "<1d", "23h"));
        assert!(holds(TokenKind::Money, ">1000", "$1,234.56"));
        assert!(!holds(TokenKind::Money, ">1000", "$999.99"));
        assert!(holds(TokenKind::Money, ">$1000", "$1,234.56"));
        assert!(holds(TokenKind::Percent, ">50", "75%"));
        assert!(holds(TokenKind::Percent, "<=50%", "12.5%"));
        assert!(holds(TokenKind::Word, "len>3", "hello"));
        assert!(!holds(TokenKind::Word, "len>5", "hello"));
        assert!(holds(TokenKind::Word, "len=4", "caf\u{E9}"));
        assert!(TypedPred::parse(TokenKind::Duration, ">500").is_err());
        assert!(TypedPred::parse(TokenKind::Word, ">5").is_err());
        assert!(TypedPred::parse(TokenKind::Number, "len>5").is_err());
        assert!(TypedPred::parse(TokenKind::Punct, ">5").is_err());
        assert!(TypedPred::parse(TokenKind::Number, "").is_err());
        assert!(TypedPred::parse(TokenKind::Number, ">5,").is_err());
    }

    #[test]
    fn addresses_and_blocks() {
        assert!(holds(TokenKind::Ip, "in:10.0.0.0/8", "10.20.30.40"));
        assert!(!holds(TokenKind::Ip, "in:10.0.0.0/8", "11.0.0.1"));
        assert!(holds(TokenKind::Ip, "in:192.168.1.0/24", "192.168.1.255"));
        assert!(!holds(TokenKind::Ip, "in:192.168.1.0/24", "192.168.2.1"));
        assert!(holds(TokenKind::Ip, "in:fe80::/10", "fe80::1"));
        assert!(!holds(TokenKind::Ip, "in:10.0.0.0/8", "fe80::1"));
        assert!(holds(TokenKind::Ip, "in:10.0.0.1", "10.0.0.1"));
        assert!(holds(TokenKind::Ip, "in:0.0.0.0/0", "8.8.8.8"));
        assert!(holds(TokenKind::Ip, "10.0.0.1..10.0.0.9", "10.0.0.5"));
        assert!(!holds(TokenKind::Ip, "10.0.0.1..10.0.0.9", "10.0.0.10"));
        assert!(holds(TokenKind::Ip, "v6", "::1"));
        assert!(holds(TokenKind::Ip, "v4", "1.2.3.4"));
        assert!(holds(TokenKind::Ip, "family:v6", "::1"));
        assert!(holds(TokenKind::Ip, "!=1.2.3.4", "::1"));
        assert!(holds(TokenKind::Cidr, "contains:10.1.2.3", "10.0.0.0/8"));
        assert!(!holds(TokenKind::Cidr, "contains:11.1.2.3", "10.0.0.0/8"));
        assert!(holds(TokenKind::Cidr, "in:10.0.0.0/8", "10.1.0.0/16"));
        assert!(!holds(TokenKind::Cidr, "in:10.1.0.0/16", "10.0.0.0/8"));
        assert!(holds(TokenKind::Cidr, "prefix>=24", "192.168.1.0/24"));
        assert!(holds(TokenKind::Cidr, "=10.0.0.0/8", "10.0.0.0/8"));
        assert!(TypedPred::parse(TokenKind::Ip, "in:10.0.0.0/33").is_err());
        assert!(TypedPred::parse(TokenKind::Ip, "host:x").is_err());
    }

    #[test]
    fn urls_emails_paths() {
        assert!(holds(TokenKind::Url, "host:*.internal", "https://db.corp.internal/x"));
        assert!(holds(TokenKind::Url, "host:*.INTERNAL", "https://db.corp.internal/x"));
        assert!(!holds(TokenKind::Url, "host:*.internal", "https://example.com/internal"));
        assert!(holds(TokenKind::Url, "scheme:https", "https://a.b/c"));
        assert!(holds(TokenKind::Url, "port=8443", "https://a.b:8443/c"));
        assert!(holds(TokenKind::Url, "port:443", "https://a.b/c"));
        assert!(holds(TokenKind::Url, "port>1024", "http://a.b:8080/"));
        assert!(holds(TokenKind::Url, "path:/api/*", "http://a.b/api/v1"));
        assert!(!holds(TokenKind::Url, "path:/API/*", "http://a.b/api/v1"));
        assert!(holds(TokenKind::Url, "query:*token=*", "http://a.b/c?x=1&token=abc"));
        assert!(holds(TokenKind::Email, "domain:example.com", "bob@example.com"));
        assert!(holds(TokenKind::Email, "domain:Example.COM", "bob@example.com"));
        assert!(!holds(TokenKind::Email, "domain:example.com", "bob@example.org"));
        assert!(holds(TokenKind::Email, "user:admin*", "admin-2@example.org"));
        assert!(holds(TokenKind::Email, "domain!=example.com", "bob@example.org"));
        assert!(holds(TokenKind::Path, "ext:log", "/var/log/app.log"));
        assert!(holds(TokenKind::Path, "name:app.*", "/var/log/app.log"));
        assert!(holds(TokenKind::Path, "dir:/var/*", "/var/log/app.log"));
        assert!(TypedPred::parse(TokenKind::Url, ">5").is_err());
        assert!(TypedPred::parse(TokenKind::Url, "host>5").is_err());
    }

    #[test]
    fn timestamps_and_ages() {
        assert!(holds(TokenKind::Timestamp, "age<24h", "2026-09-14T12:00:00Z"));
        assert!(!holds(TokenKind::Timestamp, "age<24h", "2026-09-13T12:00:00Z"));
        assert!(holds(TokenKind::Timestamp, "age>1y", "2024-01-01"));
        assert!(holds(TokenKind::Timestamp, "age<1h", "Sep 14 23:30:00"));
        assert!(holds(TokenKind::Timestamp, "2026-09-01..2026-09-15", "2026-09-14 08:00:00"));
        assert!(!holds(TokenKind::Timestamp, "2026-09-01..2026-09-14", "2026-09-14 08:00:00"));
        assert!(holds(TokenKind::Timestamp, ">2026-09-01", "15/Sep/2026:10:00:00 +0200"));
        assert!(holds(TokenKind::Timestamp, "<now", "2026-09-14"));
        assert!(!holds(TokenKind::Timestamp, ">now", "2026-09-14"));
        assert!(holds(TokenKind::Timestamp, "year=2026,month=9", "2026-09-14"));
        assert!(holds(TokenKind::Timestamp, "hour>=22", "23:15"));
        assert!(!holds(TokenKind::Timestamp, "hour>=22", "2026-09-14"));
        assert!(holds(TokenKind::Timestamp, ">12:00", "13:30"));
        assert!(!holds(TokenKind::Timestamp, ">2026-09-01", "13:30"));
        assert!(!holds(TokenKind::Timestamp, "age<24h", "13:30"));
    }

    #[test]
    fn versions_cards_phones_and_the_rest() {
        assert!(holds(TokenKind::Version, ">=2.0,<3", "2.5.1"));
        assert!(!holds(TokenKind::Version, ">=2.0,<3", "3.0.0"));
        assert!(!holds(TokenKind::Version, ">=2.0", "2.0.0-rc.1"));
        assert!(holds(TokenKind::Version, "major=1,minor>=4", "v1.4.2"));
        assert!(holds(TokenKind::Version, "pre:rc*", "1.0.0-rc.2"));
        assert!(holds(TokenKind::Version, "pre:", "1.0.0"));
        assert!(!holds(TokenKind::Version, "pre:", "1.0.0-rc.2"));
        assert!(holds(TokenKind::CreditCard, "issuer:visa", "4111 1111 1111 1111"));
        assert!(holds(TokenKind::CreditCard, "issuer:amex", "3782 822463 10005"));
        assert!(holds(TokenKind::CreditCard, "issuer:mastercard", "5555555555554444"));
        assert!(holds(TokenKind::CreditCard, "issuer:mastercard", "2223003122003222"));
        assert!(holds(TokenKind::CreditCard, "issuer:discover", "6011111111111117"));
        assert!(holds(TokenKind::CreditCard, "issuer:diners", "3056 930902 5904"));
        assert!(holds(TokenKind::CreditCard, "issuer:jcb", "3530111333300000"));
        assert!(holds(TokenKind::CreditCard, "last4:1111", "4111-1111-1111-1111"));
        assert!(holds(TokenKind::CreditCard, "len=16", "4111111111111111"));
        assert!(holds(TokenKind::Phone, "cc=44", "+44 20 7123 4567"));
        assert!(holds(TokenKind::Phone, "cc:+44", "+44 20 7123 4567"));
        assert!(holds(TokenKind::Phone, "cc=1", "212-555-1234"));
        assert!(!holds(TokenKind::Phone, "cc=1", "+44 20 7123 4567"));
        assert!(holds(TokenKind::Uuid, "version=4", "550e8400-e29b-41d4-a716-446655440000"));
        assert!(!holds(TokenKind::Uuid, "version=1", "550e8400-e29b-41d4-a716-446655440000"));
        assert!(holds(TokenKind::Mac, "oui:00:1a:2b", "00-1A-2B-3C-4D-5E"));
        assert!(holds(TokenKind::HashDigest, "algo:md5", "d41d8cd98f00b204e9800998ecf8427e"));
        assert!(holds(
            TokenKind::HashDigest,
            "algo:sha1",
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        ));
        assert!(holds(TokenKind::Geo, "lat>0,long<0", "37.7749,-122.4194"));
        assert!(!holds(TokenKind::Geo, "lat<0", "37.7749,-122.4194"));
    }

    #[test]
    fn a_now_argument_reads_any_form() {
        assert_eq!(parse_now_arg("1767225600"), Some(1_767_225_600));
        assert_eq!(parse_now_arg("2026-01-01T00:00:00Z"), Some(1_767_225_600));
        assert_eq!(parse_now_arg("2026-01-01"), Some(1_767_225_600));
        assert_eq!(parse_now_arg("nonsense"), None);
        assert_eq!(parse_tz_arg("+02:00"), Some(7200));
        assert_eq!(parse_tz_arg("-0530"), Some(-19_800));
        assert_eq!(parse_tz_arg("Z"), Some(0));
        assert_eq!(parse_tz_arg("+05"), Some(18_000));
        assert_eq!(parse_tz_arg("+5"), None);
        assert_eq!(parse_tz_arg("later"), None);
    }

    #[test]
    fn relations_hold_after_projecting_both_sides() {
        let rel = |name: &str| Relation::parse(name).unwrap_or_else(|| panic!("{name}"));
        let same = |name: &str, a: &str, b: &str| rel(name).related(a.as_bytes(), b.as_bytes());
        assert!(same("subnet", "10.0.0.1", "10.0.0.200"));
        assert!(!same("subnet", "10.0.0.1", "10.0.1.200"));
        let wide = rel("subnet").with_prefix(16).unwrap_or_else(|| panic!("subnet takes a width"));
        assert!(wide.related(b"10.0.0.1", b"10.0.1.200"));
        assert!(same("subnet", "fe80::1", "fe80::ffff"));
        assert!(!same("subnet", "fe80::1", "fe81::1"));
        assert!(!same("subnet", "10.0.0.1", "fe80::1"));
        assert!(rel("domain").with_prefix(8).is_none());
        assert!(same("domain", "bob@Example.com", "ann@example.COM"));
        assert!(!same("domain", "bob@example.com", "bob@example.org"));
        assert!(same("host", "https://Corp.internal/a", "http://corp.internal:8080/b"));
        assert!(same("port", "https://a.b/x", "http://c.d:443/y"));
        assert!(same("day", "2026-09-15T23:30:00+02:00", "2026-09-15 01:00:00"));
        assert!(!same("day", "2026-09-15T23:30:00+02:00", "2026-09-16T00:30:00+02:00"));
        assert!(same("day", "Sep 15 10:00:00", "2026-09-15"));
        assert!(same("hour", "2026-09-15T23:30:00", "2026-09-15T23:59:59"));
        assert!(!same("hour", "2026-09-15T23:30:00", "2026-09-15"));
        assert!(same("month", "2026-09-01", "2026-09-30"));
        assert!(same("year", "2026-01-01", "2026-12-31"));
        assert!(!same("year", "Sep 15 10:00:00", "2026-09-15"));
        assert!(same("major", "v1.4.2", "1.9.0"));
        assert!(!same("major", "1.4.2", "2.0.0"));
        assert!(same("minor", "1.4.2", "1.4.9"));
        assert!(!same("minor", "1.4.2", "1.5.0"));
        assert!(same("patch", "1.4.2", "1.4.2-rc.1"));
        assert!(same("magnitude", "1234", "9999"));
        assert!(!same("magnitude", "1234", "999"));
        assert!(same("magnitude", "$1,234.56", "5000"));
        assert!(same("magnitude", "0.05", "0.099"));
        assert!(same("issuer", "4111 1111 1111 1111", "4012888888881881"));
        assert!(same("cc", "+44 20 7123 4567", "+44 161 000 0000"));
        assert!(!same("ext", "/var/log/app.log", "C:\\logs\\other.LOG"));
        assert!(same("ext", "/var/log/app.log", "/tmp/x.log"));
        assert!(same("oui", "00-1A-2B-3C-4D-5E", "00:1a:2b:00:00:01"));
        assert!(same(
            "algo",
            "d41d8cd98f00b204e9800998ecf8427e",
            "c4ca4238a0b923820dcc509a6f75849b"
        ));
        assert!(same("len", "caf\u{E9}", "cafe"));
        assert!(!same("len", "cafe", "cafes"));
        assert!(same("family", "::1", "fe80::1"));
        assert!(same("prefix", "10.0.0.0/8", "11.0.0.0/8"));
        assert!(!same("domain", "not-an-email", "x@y.com"));
        // A field reader answers for a token its kind was read from, and
        // several answer for any text at all: without the kind test two bare
        // words would share an empty domain and every text would be an
        // `unknown` issuer, so each would relate to every other.
        assert!(!same("domain", "alpha", "bravo"));
        assert!(!same("issuer", "alpha", "bravo"));
        assert!(!same("host", "bob@x.com", "amy@x.com"));
    }

    #[test]
    fn a_relation_is_its_key_wherever_it_has_one() {
        let rel = |name: &str| Relation::parse(name).unwrap_or_else(|| panic!("{name}"));
        let key = |name: &str, text: &str| rel(name).key(text);
        // Every address of a network keys onto the network, which is what
        // lets a join across two inputs report an address neither log shares.
        assert_eq!(key("subnet", "10.0.0.7"), Some("10.0.0.0/24".to_string()));
        assert_eq!(key("subnet", "10.0.0.201"), Some("10.0.0.0/24".to_string()));
        assert_eq!(key("subnet", "10.0.1.7"), Some("10.0.1.0/24".to_string()));
        assert_eq!(
            rel("subnet").with_prefix(16).and_then(|r| r.key("10.0.1.7")),
            Some("10.0.0.0/16".to_string())
        );
        assert_eq!(key("subnet", "fe80::abcd"), Some("fe80::/64".to_string()));
        assert_eq!(key("domain", "bob@Example.com"), Some("example.com".to_string()));
        assert_eq!(key("magnitude", "1234"), Some("3".to_string()));
        assert_eq!(key("magnitude", "0"), Some("zero".to_string()));
        assert_eq!(key("len", "caf\u{E9}"), Some("4".to_string()));
        assert_eq!(key("minor", "1.4.2"), Some("1.4".to_string()));
        assert_eq!(key("domain", "not-an-email"), None);
        // The calendar rungs keep the year as written, and a year the text
        // never wrote is a value of its own rather than one that agrees with
        // whatever it meets. `=day` is the looser reading.
        assert_eq!(key("day", "2026-09-15T23:00:00"), Some("2026-09-15".to_string()));
        assert_eq!(key("day", "2025-09-15T01:00:00"), Some("2025-09-15".to_string()));
        assert_eq!(key("day", "Sep 15 10:00:00"), Some("....-09-15".to_string()));
        assert_eq!(key("hour", "2026-09-15T23:30:00"), Some("2026-09-15T23".to_string()));
        assert_eq!(key("month", "2026-09-30"), Some("2026-09".to_string()));
        assert_eq!(key("year", "2026-09-30"), Some("2026".to_string()));
        assert_eq!(key("year", "Sep 15 10:00:00"), None, "a year never written is no year");
        assert!(rel("day").related(b"Sep 15 10:00:00", b"2026-09-15"));
        assert_ne!(key("day", "Sep 15 10:00:00"), key("day", "2026-09-15"));
    }

    #[test]
    fn a_slash_date_is_read_in_the_order_the_calendar_allows() {
        let read = |s: &str| {
            slash_date(s.as_bytes(), 0).map(|d| (d.year, d.month, d.day, d.end, d.runs))
        };
        // Year first by shape; year last decided by which reading is a
        // valid date; day first where both are, which is the default.
        assert_eq!(read("2026/09/15"), Some((2026, 9, 15, 10, (0, 1, 2))));
        assert_eq!(read("09/15/2026"), Some((2026, 9, 15, 10, (2, 0, 1))));
        assert_eq!(read("15/09/2026"), Some((2026, 9, 15, 10, (2, 1, 0))));
        assert_eq!(read("3/4/2026"), Some((2026, 4, 3, 8, (2, 1, 0))));
        // The four-digit year is what tells a date from a run of figures.
        assert_eq!(read("1/2/3"), None);
        assert_eq!(read("13/13/2026"), None);
        assert_eq!(read("2026/13/01"), None);
        assert_eq!(read("2026/09/151"), None);
        assert_eq!(read("2026-09-15"), None, "the dashed form is not this one's");
        // The whole timestamp reads through it, clock and all.
        let civil = parse_civil("15/09/2026 10:11:12").unwrap_or_else(|| panic!("a timestamp"));
        assert_eq!((civil.year, civil.month, civil.day), (Some(2026), 9, 15));
        assert_eq!((civil.hour, civil.minute, civil.second), (10, 11, 12));
        assert_eq!(parse_now_arg("2026/01/01"), Some(1_767_225_600));
        assert_eq!(parse_date_order_arg("dmy"), Some(true));
        assert_eq!(parse_date_order_arg("mdy"), Some(false));
        assert_eq!(parse_date_order_arg("ymd"), None);
    }

    #[test]
    fn canonical_forms_fold_representations() {
        assert_eq!(canonical_number("1,000"), "1000");
        assert_eq!(canonical_number("1e3"), "1000");
        assert_eq!(canonical_number("1000.0"), "1000");
        assert_eq!(canonical_number("0x3e8"), "1000");
        assert_eq!(canonical_number("2.5E-2"), "0.025");
        assert_eq!(canonical_number("$1_000"), "1000");
        assert_eq!(canonical_number("0b1111101000"), "1000");
        assert_eq!(canonical_number("0o1750"), "1000");
        assert_eq!(canonical_number("1e999999"), "1e999999");
        assert_eq!(canonical_number("10e999998"), "1e999999");
        assert_eq!(canonical_number("word"), "word");
        assert_eq!(canonical_url("HTTP://Example.COM:80/a%2fb"), "http://example.com/a%2Fb");
        assert_eq!(canonical_url("https://a.b"), "https://a.b/");
        assert_eq!(canonical_url("https://a.b:8443/x?q=1#f"), "https://a.b:8443/x?q=1#f");
        assert_eq!(canonical_url("http://a.b/%7Euser/%41"), "http://a.b/~user/A");
        assert_eq!(canonical_url("http://a.b/x/"), "http://a.b/x/");
        assert_eq!(canonical_url("plain"), "plain");
        assert_eq!(
            canonical_time("2026-09-15T02:00:00+02:00"),
            canonical_time("2026-09-15T00:00:00Z")
        );
        assert_eq!(canonical_time("2026-09-15T00:00:00Z"), "1789430400.000000000");
        assert_eq!(canonical_time("12:30"), "t45000.000000000");
        assert_eq!(canonical_time("word"), "word");
        assert_eq!(fold_text("Caf\u{E9}"), "cafe");
        assert_eq!(fold_text("\u{FF23}\u{FF41}\u{FF46}\u{E9}"), "cafe");
        assert_eq!(fold_text("na\u{EF}ve"), "naive");
        let dec = |s: &str| Decimal::parse(s).unwrap_or_else(|| panic!("{s} reads"));
        assert_eq!(dec("1234").order(), Some(3));
        assert_eq!(dec("0.05").order(), Some(-2));
        assert_eq!(dec("0").order(), None);
        assert_eq!(dec("12.5").shift(2).to_text(), "1250");
        assert_eq!(dec("12.5").shift(-3).to_text(), "0.0125");
    }

    /// Every typed kind reads its value in its base unit, and the three
    /// spellings render it.
    #[test]
    fn every_kind_reports_its_value_in_its_base_unit() {
        let clock = Clock { now_secs: 1_789_430_400, tz_offset: 0 };
        let json = |kind: TokenKind, txt: &str, how: ValueSpelling| {
            let style = ValueStyle { spelling: how, ..ValueStyle::new() };
            value_of(kind, txt)
                .unwrap_or_else(|| panic!("{txt} reads as a value"))
                .json(kind, style, clock)
        };
        let exact = |kind: TokenKind, txt: &str| json(kind, txt, ValueSpelling::Exact);
        let in_unit = |txt: &str, unit: DurationUnit| {
            let style = ValueStyle { duration: unit, ..ValueStyle::new() };
            value_of(TokenKind::Duration, txt)
                .unwrap_or_else(|| panic!("{txt} reads as a duration"))
                .json(TokenKind::Duration, style, clock)
        };

        // A byte size reads as bytes, KB as 1000 and KiB as 1024; a day is
        // 24h and a week 7d.
        assert_eq!(exact(TokenKind::ByteSize, "4KB"), "4000");
        assert_eq!(exact(TokenKind::ByteSize, "4KiB"), "4096");
        // A duration is held and reported as exact nanoseconds, which is what
        // every comparison is made on; the other two units are a shift for a
        // reader and stay exact, so 1500ms is 1.5 seconds and not 1.4999.
        assert_eq!(exact(TokenKind::Duration, "90s"), "90000000000");
        assert_eq!(exact(TokenKind::Duration, "2d"), "172800000000000");
        assert_eq!(exact(TokenKind::Duration, "1w"), "604800000000000");
        assert_eq!(in_unit("90s", DurationUnit::Seconds), "90");
        assert_eq!(in_unit("90s", DurationUnit::Milliseconds), "90000");
        assert_eq!(in_unit("1500ms", DurationUnit::Seconds), "\"1.5\"");
        assert_eq!(in_unit("2.5s", DurationUnit::Seconds), "\"2.5\"");
        assert_eq!(in_unit("3h20m", DurationUnit::Seconds), "12000");

        // A number as itself, an address as the integer it orders by, a
        // timestamp as its epoch second, a version as the lists that decide
        // its order with build metadata absent.
        assert_eq!(exact(TokenKind::Number, "1234"), "1234");
        assert_eq!(exact(TokenKind::Ip, "192.168.1.7"), "\"3232235783\"");
        assert_eq!(exact(TokenKind::Timestamp, "2026-09-15T00:00:00Z"), "1789430400");
        assert_eq!(
            exact(TokenKind::Version, "v1.2.3-beta.2+build.7"),
            "{\"parts\": [1, 2, 3], \"pre\": [\"beta\", \"2\"]}"
        );

        // Why exact is the default, as a test rather than a claim: a double
        // holds 2^53 and no more, and money carries the digits a double
        // loses first. Under exact both stay strings that compare as
        // written; under natural both become JSON numbers.
        let big = "9007199254740993";
        assert_eq!(exact(TokenKind::Number, big), format!("\"{big}\""));
        assert_eq!(exact(TokenKind::Money, "$12.50"), "\"12.5\"");
        assert_eq!(exact(TokenKind::Percent, "99.95%"), "\"99.95\"");
        assert_eq!(json(TokenKind::Money, "$12.50", ValueSpelling::Natural), "12.5");

        // Tagged names the kind beside the exact text, whatever the kind.
        assert_eq!(
            json(TokenKind::ByteSize, "4KB", ValueSpelling::Tagged),
            "{\"kind\": \"bytesize\", \"exact\": \"4000\"}"
        );
        assert_eq!(
            json(TokenKind::Money, "$12.50", ValueSpelling::Tagged),
            "{\"kind\": \"money\", \"exact\": \"12.5\"}"
        );

        // A kind that carries no value, and text that does not parse as the
        // kind, both answer None rather than a substitute.
        assert!(value_of(TokenKind::Word, "hello").is_none());
        assert!(value_of(TokenKind::Number, "hello").is_none());
        // A duration with no unit is a parse error upstream, so there is no
        // value to report for one.
        assert!(value_of(TokenKind::Duration, "90").is_none());
    }

    /// An average of exact decimals is exact. Division is not closed over
    /// terminating decimals, but every rational has a finite way to write it.
    #[test]
    fn a_quotient_is_exact_in_both_forms() {
        let d = |s: &str| Decimal::parse(s).unwrap_or_else(|| panic!("{s} reads"));
        let rep = |s: &str, n: u64| exact_quotient(&d(s), n, QuotientForm::Repetend);
        let rat = |s: &str, n: u64| exact_quotient(&d(s), n, QuotientForm::Rational);

        // Terminating, because the reduced denominator is only 2s and 5s.
        assert_eq!(rep("120", 4).as_deref(), Some("30"));
        assert_eq!(rep("10", 4).as_deref(), Some("2.5"));
        assert_eq!(rep("1", 8).as_deref(), Some("0.125"));

        // Repeating, bracketed, and exact: 10/3 is not 3.333 rounded, it is
        // three point three recurring, which is what the brackets say.
        assert_eq!(rep("10", 3).as_deref(), Some("3.(3)"));
        assert_eq!(rep("1", 7).as_deref(), Some("0.(142857)"));
        assert_eq!(rep("22", 7).as_deref(), Some("3.(142857)"));
        // A cycle that starts after some settled digits.
        assert_eq!(rep("1", 6).as_deref(), Some("0.1(6)"));

        // A dividend carrying its own fraction scales into the division.
        assert_eq!(rep("12.5", 2).as_deref(), Some("6.25"));
        assert_eq!(rep("0.1", 3).as_deref(), Some("0.0(3)"));

        // The same values as fractions in lowest terms.
        assert_eq!(rat("120", 4).as_deref(), Some("30"));
        assert_eq!(rat("10", 4).as_deref(), Some("5/2"));
        assert_eq!(rat("10", 3).as_deref(), Some("10/3"));
        assert_eq!(rat("1", 7).as_deref(), Some("1/7"));

        // Sign is carried, and zero has none.
        assert_eq!(rep("-10", 3).as_deref(), Some("-3.(3)"));
        assert_eq!(rep("0", 3).as_deref(), Some("0"));

        // No matches is no quotient, rather than a quotient of nothing.
        assert!(exact_quotient(&d("10"), 0, QuotientForm::Repetend).is_none());
    }

    /// The six aggregates, and the four percentile methods over one run of
    /// values chosen so the methods disagree.
    #[test]
    fn the_aggregates_read_in_the_kinds_base_unit() {
        let clock = Clock { now_secs: 1_789_430_400, tz_offset: 0 };
        let collect = |kind: TokenKind, texts: &[&str]| {
            let mut c = Collected::default();
            for t in texts {
                c.push(value_of(kind, t).unwrap_or_else(|| panic!("{t} reads")));
            }
            c
        };
        let report = |c: &Collected, agg: Agg, kind: TokenKind, how: Percentile| {
            c.report(agg, kind, ValueStyle::new(), QuotientForm::Repetend, how, clock)
        };

        // Byte sizes sum and average in bytes, on KB of 1000.
        let sizes = collect(TokenKind::ByteSize, &["1KB", "2KB", "3KB", "4KB"]);
        assert_eq!(report(&sizes, Agg::Sum, TokenKind::ByteSize, Percentile::Nearest).as_deref(), Some("10000"));
        assert_eq!(report(&sizes, Agg::Avg, TokenKind::ByteSize, Percentile::Nearest).as_deref(), Some("2500"));
        assert_eq!(report(&sizes, Agg::Min, TokenKind::ByteSize, Percentile::Nearest).as_deref(), Some("1000"));
        assert_eq!(report(&sizes, Agg::Max, TokenKind::ByteSize, Percentile::Nearest).as_deref(), Some("4000"));

        // An average that does not divide evenly stays exact.
        let three = collect(TokenKind::Number, &["1", "2", "4"]);
        assert_eq!(report(&three, Agg::Avg, TokenKind::Number, Percentile::Nearest).as_deref(), Some("2.(3)"));

        // Four values, so the methods differ: nearest and lower name an
        // observed value, linear interpolates between two, and hybrid takes
        // linear here because a number can be interpolated.
        let n = TokenKind::Number;
        let vals = collect(n, &["10", "20", "30", "40"]);
        assert_eq!(report(&vals, Agg::Pct(50), n, Percentile::Nearest).as_deref(), Some("20"));
        assert_eq!(report(&vals, Agg::Pct(50), n, Percentile::Lower).as_deref(), Some("20"));
        assert_eq!(report(&vals, Agg::Pct(50), n, Percentile::Linear).as_deref(), Some("25"));
        assert_eq!(report(&vals, Agg::Pct(50), n, Percentile::Hybrid).as_deref(), Some("25"));
        assert_eq!(report(&vals, Agg::Pct(95), n, Percentile::Nearest).as_deref(), Some("40"));
        assert_eq!(report(&vals, Agg::Pct(100), n, Percentile::Nearest).as_deref(), Some("40"));

        // A version orders but does not add: min and max read, and hybrid
        // takes nearest for it rather than refusing or interpolating.
        let v = TokenKind::Version;
        let vers = collect(v, &["v1.2.0", "v1.10.0", "v1.3.0"]);
        assert_eq!(report(&vers, Agg::Min, v, Percentile::Nearest).as_deref(), Some("{\"parts\": [1, 2, 0]}"));
        assert_eq!(report(&vers, Agg::Max, v, Percentile::Nearest).as_deref(), Some("{\"parts\": [1, 10, 0]}"));
        assert_eq!(report(&vers, Agg::Pct(50), v, Percentile::Hybrid).as_deref(), Some("{\"parts\": [1, 3, 0]}"));
        // Summing one is not a thing a version can do.
        assert!(report(&vers, Agg::Sum, v, Percentile::Nearest).is_none());

        // Which aggregates each kind admits, which is what the refusal reads.
        assert!(Agg::Sum.admits(TokenKind::ByteSize));
        assert!(!Agg::Sum.admits(TokenKind::Version));
        assert!(Agg::Min.admits(TokenKind::Version));
        assert!(Agg::Min.admits(TokenKind::Timestamp));
        assert!(!Agg::Min.admits(TokenKind::Word));
        assert!(!Agg::Sum.admits(TokenKind::Word));

        // No values is no aggregate, rather than a zero that reads as a sum.
        assert!(report(&Collected::default(), Agg::Sum, n, Percentile::Nearest).is_none());
    }

    /// Lower names the value at or below the position linear reads, which is
    /// not always the value nearest names; a linear duration reads in the
    /// unit the column reports; and a picked value comes back whole.
    #[test]
    fn each_percentile_method_and_a_picked_value_read_as_named() {
        let clock = Clock { now_secs: 1_789_430_400, tz_offset: 0 };
        let collect = |kind: TokenKind, texts: &[&str]| {
            let mut c = Collected::default();
            for t in texts {
                c.push(value_of(kind, t).unwrap_or_else(|| panic!("{t} reads")));
            }
            c
        };
        let n = TokenKind::Number;
        let vals = collect(n, &["10", "20", "30", "40"]);
        let at = |p: u32, how: Percentile| {
            vals.report(Agg::Pct(p), n, ValueStyle::new(), QuotientForm::Repetend, how, clock)
        };
        // p60 of four is at position 1.8 of 0 to 3: nearest takes rank
        // ceil(2.4), lower the value below 1.8, linear eight tenths of the
        // way from 20 to 30.
        assert_eq!(at(60, Percentile::Nearest).as_deref(), Some("30"));
        assert_eq!(at(60, Percentile::Lower).as_deref(), Some("20"));
        assert_eq!(at(60, Percentile::Linear).as_deref(), Some("28"));
        assert_eq!(at(60, Percentile::Hybrid).as_deref(), Some("28"));
        assert_eq!(at(0, Percentile::Nearest).as_deref(), Some("10"));
        assert_eq!(at(100, Percentile::Lower).as_deref(), Some("40"));
        assert!(at(101, Percentile::Nearest).is_none());

        // The picked value is the one the report renders, and an
        // interpolated percentile picks none.
        let picked = |agg: Agg, how: Percentile| match vals.picked(agg, n, how) {
            Some(TypedValue::Num(d)) => Some(d.to_text()),
            Some(other) => panic!("a number picks a number, not {other:?}"),
            None => None,
        };
        assert_eq!(picked(Agg::Pct(60), Percentile::Lower).as_deref(), Some("20"));
        assert_eq!(picked(Agg::Pct(60), Percentile::Nearest).as_deref(), Some("30"));
        assert_eq!(picked(Agg::Max, Percentile::Nearest).as_deref(), Some("40"));
        assert!(picked(Agg::Pct(60), Percentile::Linear).is_none());
        assert!(picked(Agg::Sum, Percentile::Nearest).is_none());

        // An interpolated duration reads in the unit the column reports, as
        // an observed one does.
        let d = TokenKind::Duration;
        let ms = ValueStyle { duration: DurationUnit::Milliseconds, ..ValueStyle::new() };
        let spans = collect(d, &["1s", "2s"]);
        let p50 = |how| spans.report(Agg::Pct(50), d, ms, QuotientForm::Repetend, how, clock);
        assert_eq!(p50(Percentile::Linear).as_deref(), Some("1500"));
        assert_eq!(p50(Percentile::Nearest).as_deref(), Some("1000"));

        // An instant renders to the second and is picked with its fraction.
        let t = TokenKind::Timestamp;
        let stamps = collect(t, &["2026-09-27T10:00:00.250Z", "2026-09-27T09:00:00.750Z"]);
        let style = ValueStyle::new();
        let earliest = stamps.report(Agg::Min, t, style, QuotientForm::Repetend, Percentile::Nearest, clock);
        assert_eq!(earliest.as_deref(), Some("1790499600"));
        match stamps.picked(Agg::Min, t, Percentile::Nearest) {
            Some(TypedValue::Instant(civil)) => {
                assert_eq!(civil.epoch(clock), Some((1_790_499_600, 750_000_000)));
            }
            other => panic!("a timestamp picks an instant, not {other:?}"),
        }

        // An address renders as its integer and is picked as the address.
        let ip = TokenKind::Ip;
        let hosts = collect(ip, &["10.0.0.10", "10.0.0.9"]);
        let lowest = hosts.report(Agg::Min, ip, style, QuotientForm::Repetend, Percentile::Nearest, clock);
        assert_eq!(lowest.as_deref(), Some("\"167772169\""));
        match hosts.picked(Agg::Min, ip, Percentile::Nearest) {
            Some(TypedValue::Ip(addr)) => assert_eq!(addr.to_string(), "10.0.0.9"),
            other => panic!("an address picks an address, not {other:?}"),
        }
    }

    /// A timestamp naming a time and no date places no instant, so it
    /// reports JSON null rather than one guessed from the clock.
    #[test]
    fn a_time_without_a_date_reports_no_value() {
        let clock = Clock { now_secs: 1_789_430_400, tz_offset: 0 };
        let v = value_of(TokenKind::Timestamp, "12:30").expect("a time reads");
        assert_eq!(v.json(TokenKind::Timestamp, ValueStyle::new(), clock), "null");
    }
}
