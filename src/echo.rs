//! The echo axis - trex's recurrence substrate, the connection on the bundle.
//!
//! Every other axis is a one-point function: `magnitude` reads a value's
//! scale, `stress` its structural load, `spectral` its temporal texture,
//! `flow` its dynamics, `shape` its form, `orbit` its symmetry class, `seam`
//! its predictability boundary, `observation` its vantage-dependence - each a
//! local functional of a window around one position. Echo is the two-point
//! function: for each token, does this content occur ELSEWHERE, how often, how
//! far away, and how regularly? It is content-addressed and unbounded-range,
//! where `spectral` autocorrelates a bounded window; a log template recurring
//! every 2 KB, an identifier bound 40 times across a file, and a phrase that
//! returns 400 KB later are all echo and nothing else.
//!
//! Per token the axis reads four fields:
//!
//! - **count** - how many times this token's key occurs in the document;
//! - **back / forward lag** - the byte distance to the previous / next
//!   occurrence (no previous = **novel**, the first appearance);
//! - **period** - when a key recurs at least three times with regular
//!   spacing, the mean lag: the document-scale pitch of a repeating template;
//! - **strength** - the recurrence mass, `count - 1` (0 = unique).
//!
//! The key is the token's text quotiented by an [`OrbitGroup`], so recurrence
//! composes with the symmetry axis: under `Case`, `Foo` and `foo` are one
//! echo; under `Shape`, `1,22,3` and `4,55,6` are. Recurrence also lifts to
//! the supertoken tower ([`analyze_super`]): the same structural unit (role
//! plus silhouette) returning across a document is structural rhyme - a
//! repeated config block, a log template, a stanza.

use std::collections::HashMap;
use std::num::NonZeroU32;

use crate::orbit::{OrbitGroup, canonical};
use crate::token::{Token, TokenKind};

/// Tuning for the echo analysis.
#[derive(Clone, Copy, Debug)]
pub struct EchoConfig {
    /// The symmetry group the key is quotiented by: recurrence up to this
    /// equivalence. `Identity` (the default) is exact-text recurrence.
    pub orbit: OrbitGroup,
    /// Minimum byte length for a token to be keyed; shorter tokens get an
    /// unkeyed (zero-echo) frame.
    pub min_len: usize,
    /// Maximum coefficient of variation of a key's successive lags for the
    /// recurrence to count as periodic (the mean lag is then its period).
    pub max_period_cv: f32,
}

impl Default for EchoConfig {
    fn default() -> Self {
        EchoConfig { orbit: OrbitGroup::Identity, min_len: 1, max_period_cv: 0.3 }
    }
}

/// One token's echo reading.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EchoFrame {
    /// Byte offset where the token begins, at the width [`Token`] stores it.
    pub start: u32,
    /// Byte offset just past the token, at the same width.
    pub end: u32,
    /// Whether the token participates in the recurrence field (word / number /
    /// quoted / typed-literal kinds at or above the length floor). Unkeyed
    /// tokens (whitespace, punctuation, brackets) carry a zero frame.
    pub keyed: bool,
    /// Occurrences of this token's key in the document (1 = unique).
    pub count: u32,
    /// Byte distance back to the previous occurrence of the key, or `None`
    /// when this is the first (a novel token).
    ///
    /// Non-zero because two occurrences of one key begin at different bytes,
    /// which is the niche that keeps the `Option` free: a plain `Option<u32>`
    /// is eight bytes where this is four, over one frame a token.
    pub back_lag: Option<NonZeroU32>,
    /// Byte distance forward to the next occurrence, or `None` at the last,
    /// non-zero for the same reason as [`EchoFrame::back_lag`].
    pub fwd_lag: Option<NonZeroU32>,
    /// The key's recurrence period in bytes when its lags are regular, and zero
    /// where it has none.
    ///
    /// Zero rather than `None`, which saves the four bytes an `Option<f32>`
    /// spends on a discriminant over one frame a token. A period is only ever
    /// set from a mean lag the writer requires to be above zero, so zero was
    /// never a reading this could carry and reads as the absence it is.
    ///
    /// Zero rather than a NaN, which would be the other way to mark it and is
    /// wrong twice here: `Default` gives an unkeyed token a zero frame and
    /// `f32::default()` is zero, not NaN, so absence would have two spellings;
    /// and NaN is unequal to itself, so the derived `PartialEq` would report two
    /// frames with no period as different.
    pub period: f32,
    /// This occurrence's place among the key's, counting from one; zero for
    /// an unkeyed token.
    pub nth: u32,
}

impl EchoFrame {
    /// The first appearance of a keyed token: no prior occurrence.
    #[must_use]
    pub fn novel(&self) -> bool {
        self.keyed && self.back_lag.is_none()
    }

    /// A keyed token whose key occurs more than once in the document.
    #[must_use]
    pub fn echoed(&self) -> bool {
        self.keyed && self.count >= 2
    }

    /// The recurrence mass: occurrences beyond this one (0 = unique).
    #[must_use]
    pub fn strength(&self) -> f32 {
        self.count.saturating_sub(1) as f32
    }
}

/// The echo field over a token stream: one frame per token, aligned with the
/// input token slice, plus document-level summary readings.
#[derive(Clone, Debug)]
pub struct EchoField {
    /// One frame per input token (index-aligned with the lexed stream).
    pub frames: Vec<EchoFrame>,
    /// Keyed tokens in the stream.
    pub keyed: usize,
    /// Distinct keys among them.
    pub distinct: usize,
    /// Keyed tokens that are first appearances.
    pub novel: usize,
    /// Keyed tokens whose key recurs.
    pub echoed: usize,
}

impl EchoField {
    /// The fraction of keyed tokens that are first appearances - the
    /// document's novelty rate (1.0 = nothing ever repeats).
    #[must_use]
    pub fn novelty(&self) -> f32 {
        if self.keyed == 0 { 0.0 } else { self.novel as f32 / self.keyed as f32 }
    }

    /// The fraction of keyed tokens that recur - the document's echo rate.
    #[must_use]
    pub fn echo_rate(&self) -> f32 {
        if self.keyed == 0 { 0.0 } else { self.echoed as f32 / self.keyed as f32 }
    }
}

/// Whether a token kind participates in the recurrence field. Structure
/// (whitespace, punctuation, brackets) and unclassified spans recur by
/// grammar, not by content, so they are not keyed.
pub(crate) fn keyed_kind(kind: TokenKind) -> bool {
    !matches!(
        kind,
        TokenKind::Whitespace
            | TokenKind::Punct
            | TokenKind::Open(_)
            | TokenKind::Close(_)
            | TokenKind::Other
    )
}

/// The group number standing for a token that belongs to no group, so the
/// layout skips it. A stream long enough to reach it could not be indexed by
/// the `u32` that layout uses either.
const UNKEYED: u32 = u32::MAX;

/// Number the keyed tokens' keys in first-occurrence order, answering each
/// token's group and how many tokens each group holds. Marks the keyed
/// tokens on their frames on the way.
///
/// The key type is the caller's, so a rung whose representative is the token's
/// own bytes hands back a borrow of the input and one whose representative is
/// rewritten text hands back the text it made. The map holds a group number
/// rather than that group's occurrences, so a value is four bytes here instead
/// of a `Vec` whose buffer is a second allocation per distinct key - and text
/// where nearly every token is unique, identifiers carrying a serial number
/// being the usual shape of real source, reaches one distinct key per token.
fn group_tokens<K: Eq + std::hash::Hash>(
    tokens: &[Token],
    cfg: &EchoConfig,
    frames: &mut [EchoFrame],
    mut key_of: impl FnMut(usize) -> K,
) -> (Vec<u32>, Vec<u32>) {
    let n = tokens.len();
    let keyed = |t: &Token| keyed_kind(t.kind) && t.len() >= cfg.min_len;

    // Which keys can possibly occur twice, read before any of them is put in a
    // map. A key that occurs once wants a group holding only itself, and a map
    // large enough to hold one per distinct key misses cache on every probe -
    // which on text where nearly every token is unique is nearly every token.
    //
    // The counters saturate at two, and the reading is one-sided: a key
    // occurring twice increments the same counter twice and cannot read one, so
    // a counter reading one PROVES its key unique. Two different keys sharing a
    // counter both read two and both go on to the map, which costs work and
    // answers the same.
    // The hash a token's key carries, kept so the pass below need not build a
    // key it will not use. Under a rung that rewrites the text a key is a fresh
    // String, and a key proved unique is never looked up, so building it twice
    // would cost the rung the very work the skip saves.
    let counting = crate::trace::phase("echo: keying, counting the keys");
    let mut hashes: Vec<u32> = vec![0; n];
    // How many tokens this pass builds a key for. The grouping pass below
    // builds one only for the tokens its skip does not answer, so the two
    // counts divide the key's own cost out of the difference between them.
    let mut keyed_count = 0u64;
    let seen = Repeats::over(n, |table| {
        for (i, t) in tokens.iter().enumerate() {
            if keyed(t) {
                keyed_count += 1;
                let h = Repeats::index_of(&key_of(i));
                hashes[i] = h;
                table.saw(h);
            }
        }
    });
    crate::trace::counted("echo: tokens keyed", keyed_count);
    drop(counting);

    let _grouping = crate::trace::phase("echo: keying, grouping what repeats");
    // The crate's own hash with an avalanche over what it finishes with. Plain
    // fxhash measured 19% worse than SipHash here, on keys differing only in a
    // trailing serial number: hashbrown reads a bucket from one end of the hash
    // and a control byte from the other, and fxhash distributes one end and not
    // the other. The repeat table above already carries its index through the
    // same five steps for the same reason.
    let mut ids: HashMap<K, u32, crate::fxhash::FxFinalBuild> = HashMap::default();
    let mut group_of: Vec<u32> = vec![UNKEYED; n];
    let mut counts: Vec<u32> = Vec::new();
    // How far down this loop a token gets, carried locally and handed over
    // once. The loop walks every token whether or not its key can echo, so
    // what the map costs and what the walk costs are different questions and
    // the phase around them answers neither on its own.
    //
    // `inserted` divides the probes again: an entry that is written is a
    // different cost from one that is found, and a probe count alone reads them
    // as the same operation.
    let (mut walked, mut probed, mut inserted) = (0u64, 0u64, 0u64);
    for (i, t) in tokens.iter().enumerate() {
        walked += 1;
        if !keyed(t) {
            continue;
        }
        frames[i].keyed = true;
        if seen.once(hashes[i]) {
            // Its own group, and nothing to look up: the map never learns of a
            // key that cannot echo.
            group_of[i] = u32::try_from(counts.len()).expect("a group index within the stored width");
            counts.push(1);
            continue;
        }
        probed += 1;
        let fresh = u32::try_from(counts.len()).expect("a group index within the stored width");
        let g = *ids.entry(key_of(i)).or_insert(fresh);
        if g == fresh {
            inserted += 1;
            counts.push(0);
        }
        counts[g as usize] += 1;
        group_of[i] = g;
    }
    crate::trace::counted("echo: tokens the grouping loop walks", walked);
    crate::trace::counted("echo: tokens that reach the map", probed);
    // What the probe does, and over which key. Writing an entry and finding one
    // are different costs, and a probe count alone reads them as one operation.
    // The key's type is the other half: this is generic over `K`, and a rung
    // that rewrites the text hands it an owned key where an identity rung hands
    // it a borrow, so the row is named by the type each instantiation carries.
    crate::trace::counted("echo: map probes that write an entry", inserted);
    crate::trace::counted("echo: map probes that find one", probed - inserted);
    crate::trace::counted(std::any::type_name::<K>(), probed);
    (group_of, counts)
}

/// Saturating two-bit counters over the hashes of the keys, answering whether a
/// key was seen once or more than once.
///
/// Sized from the token count rather than to a figure of its own, and two bits
/// wide because "once, or more than once" is the whole question.
struct Repeats {
    /// Four counters a byte.
    slots: Vec<u8>,
    /// One less than the counter count, which is a power of two.
    mask: u64,
}

impl Repeats {
    /// Count every key `fill` names, over a table sized for `tokens` of them.
    fn over(tokens: usize, fill: impl FnOnce(&mut Self)) -> Self {
        let counters = tokens.saturating_mul(2).next_power_of_two().max(64);
        let mut table = Repeats {
            slots: vec![0u8; counters / 4],
            mask: (counters - 1) as u64,
        };
        fill(&mut table);
        table
    }

    /// The number this table counts a key under, which a caller keeps rather
    /// than rebuilding the key to ask twice.
    ///
    /// Carried through [`crate::fxhash::avalanche`], because the crate's hash is
    /// built for a map that takes its bucket from the high bits and leaves the
    /// low ones poorly mixed - and a counter is chosen by the low ones. It is
    /// kept to four bytes: a counter is chosen by twenty-two bits on the largest
    /// input this crate's token indices admit, so the other half of a word is
    /// memory traffic spent and never read.
    ///
    /// The named function is the one place those five steps live, so this table
    /// and the key map in `group_tokens` mix alike rather than by two copies
    /// that can drift apart.
    fn index_of<K: std::hash::Hash>(key: &K) -> u32 {
        use std::hash::BuildHasher;
        let mixed = crate::fxhash::avalanche(crate::fxhash::FxBuild::process().hash_one(key));
        (mixed & 0xffff_ffff) as u32
    }

    /// Where an index's counter is: the byte holding it, and its shift in it.
    fn at(&self, index: u32) -> (usize, u32) {
        let at = (u64::from(index) & self.mask) as usize;
        (at / 4, ((at % 4) * 2) as u32)
    }

    /// Record one appearance of a key counted under `index`, saturating at two.
    fn saw(&mut self, index: u32) {
        let (byte, shift) = self.at(index);
        let held = (self.slots[byte] >> shift) & 0b11;
        if held < 2 {
            self.slots[byte] += 1 << shift;
        }
    }

    /// Whether a key counted under `index` was seen exactly once, which is
    /// proof that it occurs once.
    fn once(&self, index: u32) -> bool {
        let (byte, shift) = self.at(index);
        (self.slots[byte] >> shift) & 0b11 == 1
    }
}

/// Analyze the echo field with the default configuration.
#[must_use]
pub fn analyze(tokens: &[Token], bytes: &[u8]) -> EchoField {
    analyze_with(tokens, bytes, &EchoConfig::default())
}

/// Lex `bytes` and analyze its echo field with the default configuration.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> EchoField {
    analyze(&crate::lexer::lex(bytes), bytes)
}

/// Analyze the echo field: key each participating token by its orbit-canonical
/// text, collect per-key occurrence lists, and read count / lags / period back
/// onto every token's frame.
#[must_use]
pub fn analyze_with(tokens: &[Token], bytes: &[u8], cfg: &EchoConfig) -> EchoField {
    // The parts this divides into, named so a share of the field is read rather
    // than reasoned about. It is the largest of the axis fields the set engine
    // builds, so which part carries the time decides where work on it goes.
    let framing = crate::trace::phase("echo: a frame a token");
    let mut frames: Vec<EchoFrame> =
        tokens
            .iter()
            .map(|t| EchoFrame { start: t.start, end: t.end, ..Default::default() })
            .collect();
    drop(framing);
    // The group each token's key belongs to, and how many tokens each group
    // holds. The identity rung's representative is the token's own literal
    // bytes, so its key is borrowed from the input: a rung that genuinely
    // rewrites the text owns its key, and only that rung allocates one.
    let keying = crate::trace::phase("echo: keying the tokens");
    let (group_of, counts) = match cfg.orbit {
        OrbitGroup::Identity => {
            group_tokens(tokens, cfg, &mut frames, |i| &bytes[tokens[i].span()])
        }
        g => group_tokens(tokens, cfg, &mut frames, |i| {
            canonical(&bytes[tokens[i].span()], g).into_bytes()
        }),
    };

    drop(keying);

    // Every group's occurrences, contiguous and in stream order: the counts
    // prefix-summed give each group its run, and one pass over the tokens
    // scatters each index into the run its group owns. Walking the tokens in
    // order is what leaves each run ascending.
    let laying = crate::trace::phase("echo: laying out the occurrences");
    let distinct = counts.len();
    let mut starts: Vec<u32> = Vec::with_capacity(distinct + 1);
    let mut acc = 0u32;
    for &c in &counts {
        starts.push(acc);
        acc += c;
    }
    starts.push(acc);
    let mut cursor: Vec<u32> = starts[..distinct].to_vec();
    let mut flat: Vec<u32> = vec![0; acc as usize];
    for (i, &g) in group_of.iter().enumerate() {
        if g == UNKEYED {
            continue;
        }
        let slot = &mut cursor[g as usize];
        flat[*slot as usize] = i as u32;
        *slot += 1;
    }

    drop(laying);

    let _reading = crate::trace::phase("echo: reading the runs back onto the frames");
    let mut keyed = 0usize;
    let mut novel = 0usize;
    let mut echoed = 0usize;
    for g in 0..distinct {
        let list = &flat[starts[g] as usize..starts[g + 1] as usize];
        let count = list.len() as u32;
        // Successive byte lags between occurrences, for the period test. The
        // run is contiguous, so a lag is read off it where it was collected.
        let lags = list.len().saturating_sub(1);
        let lag = |w: usize| {
            (tokens[list[w + 1] as usize].start - tokens[list[w] as usize].start) as f32
        };
        let period = (lags >= 2)
            .then(|| {
                let mut sum = 0.0f32;
                for w in 0..lags {
                    sum += lag(w);
                }
                let mean = sum / lags as f32;
                let mut spread = 0.0f32;
                for w in 0..lags {
                    let d = lag(w) - mean;
                    spread += d * d;
                }
                let var = spread / lags as f32;
                (mean > 0.0 && var.sqrt() / mean <= cfg.max_period_cv).then_some(mean)
            })
            .flatten();
        for (j, &slot) in list.iter().enumerate() {
            let i = slot as usize;
            let f = &mut frames[i];
            f.count = count;
            f.back_lag = (j > 0).then(|| {
                let lag = tokens[i].start - tokens[list[j - 1] as usize].start;
                NonZeroU32::new(lag).expect("two occurrences of a key begin at different bytes")
            });
            f.fwd_lag = (j + 1 < list.len()).then(|| {
                let lag = tokens[list[j + 1] as usize].start - tokens[i].start;
                NonZeroU32::new(lag).expect("two occurrences of a key begin at different bytes")
            });
            f.period = period.unwrap_or(0.0);
            f.nth = j as u32 + 1;
            keyed += 1;
            if j == 0 {
                novel += 1;
            }
            if count >= 2 {
                echoed += 1;
            }
        }
    }
    EchoField { frames, keyed, distinct, novel, echoed }
}

/// One recurring structural unit in the supertoken tower: echo lifted to the
/// layer above tokens. The key is the unit's role plus the shape-class
/// silhouette of its span, so two units that differ only in their identifiers
/// and values are the same structure - structural rhyme.
#[derive(Clone, Debug)]
pub struct SuperEcho {
    /// The unit's role label plus silhouette (the structural key).
    pub key: String,
    /// How many units in the document share it.
    pub count: u32,
    /// The recurrence period in bytes, when the spacing is regular.
    pub period: Option<f32>,
    /// Byte offset of the first occurrence.
    pub first: usize,
}

/// The letter one silhouette code spells in a structural-rhyme key.
///
/// The kind half of [`crate::shape::shape_class`] read back: its high sixteen
/// bits are a [`TokenKind`] code, and for punctuation its low sixteen bits are
/// the glyph's code point. A word is `W`, a number `N`, a quoted run `Q`,
/// punctuation and a bracket the glyph itself, and every other kind `T`; a
/// glyph whose low sixteen bits name no character is spelled U+FFFD, the
/// replacement character.
///
/// The key is read by a person - `kv:W:W` is a key beside a value, twice, the
/// middle colon being the punctuation's own glyph - so it spells the kinds
/// rather than printing the codes it compares on.
fn silhouette_letter(code: u32) -> char {
    let kind = code >> 16;
    if kind == TokenKind::Word.code() {
        return 'W';
    }
    if kind == TokenKind::Number.code() {
        return 'N';
    }
    if kind == TokenKind::Quoted.code() {
        return 'Q';
    }
    if kind == TokenKind::Punct.code() {
        return match char::from_u32(code & 0xFFFF) {
            Some(glyph) => glyph,
            None => char::REPLACEMENT_CHARACTER,
        };
    }
    // A bracket carries no glyph in its code, so it is spelled from the pair
    // the code names rather than read out of the low bits.
    match TokenKind::bracket_of_code(kind) {
        Some((true, crate::token::BracketKind::Paren)) => '(',
        Some((true, crate::token::BracketKind::Square)) => '[',
        Some((true, crate::token::BracketKind::Brace)) => '{',
        Some((false, crate::token::BracketKind::Paren)) => ')',
        Some((false, crate::token::BracketKind::Square)) => ']',
        Some((false, crate::token::BracketKind::Brace)) => '}',
        None => 'T',
    }
}

/// Recurring supertoken structures, most frequent first. Only structures that
/// actually recur are reported (a unique unit is not rhyme).
///
/// The structural key is the unit's role plus its token-kind silhouette (words
/// as `W`, numbers as `N`, quoted as `Q`, typed literals as `T`, punctuation
/// and brackets as themselves) - coarse enough that `alpha: one` and
/// `bravo: two` are the same structure, which byte-level shape classes are
/// not.
#[must_use]
pub fn analyze_super(bytes: &[u8]) -> Vec<SuperEcho> {
    analyze_super_with(bytes, &EchoConfig::default())
}

/// [`analyze_super`] with a period read under `cfg.max_period_cv`.
#[must_use]
pub fn analyze_super_with(bytes: &[u8], cfg: &EchoConfig) -> Vec<SuperEcho> {
    analyze_super_tokens(&crate::lexer::lex(bytes), bytes, cfg)
}

/// [`analyze_super_with`] over every token of `bytes` already lexed, as a
/// lex under declarations reads them.
#[must_use]
pub fn analyze_super_tokens(toks: &[Token], bytes: &[u8], cfg: &EchoConfig) -> Vec<SuperEcho> {
    let units = crate::supertoken::supertokens_from(toks, bytes);
    // The unit's silhouette comes from the shape axis, which is what computes
    // silhouettes. Folding it through the profile monoid also means the key
    // tracks that axis rather than a second, private idea of token shape.
    let ctx = crate::profile::AxisCtx::new(bytes);
    let mut occ: HashMap<String, Vec<usize>> = HashMap::new();
    // The units are in stream order and so are the tokens, so one cursor walks
    // both instead of rescanning the token stream per unit.
    let mut cursor = 0usize;
    for u in &units {
        while cursor < toks.len() && toks[cursor].start() < u.start {
            cursor += 1;
        }
        let lo = cursor;
        let mut hi = cursor;
        while hi < toks.len() && toks[hi].end() <= u.end {
            hi += 1;
        }
        let shape: crate::profile::ShapeProfile = crate::profile::fold_tokens(
            lo,
            toks[lo..hi].iter().filter(|t| t.is_significant()).copied().collect::<Vec<_>>().as_slice(),
            &ctx,
        );
        let mut key = String::from(u.role.label());
        key.push(':');
        for &code in &shape.silhouette {
            key.push(silhouette_letter(code));
        }
        occ.entry(key).or_default().push(u.start);
    }
    let mut out: Vec<SuperEcho> = occ
        .into_iter()
        .filter(|(_, starts)| starts.len() >= 2)
        .map(|(key, starts)| {
            let lags: Vec<f32> = starts.windows(2).map(|w| (w[1] - w[0]) as f32).collect();
            let period = (lags.len() >= 2)
                .then(|| {
                    let mean = lags.iter().sum::<f32>() / lags.len() as f32;
                    let var = lags.iter().map(|l| (l - mean) * (l - mean)).sum::<f32>()
                        / lags.len() as f32;
                    (mean > 0.0 && var.sqrt() / mean <= cfg.max_period_cv).then_some(mean)
                })
                .flatten();
            SuperEcho { key, count: starts.len() as u32, period, first: starts[0] }
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.first.cmp(&b.first)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame a token of the input, so this width multiplies by the token
    /// count: 2,650,000 of them on the comparison corpus. Pinned the way the
    /// lexer pins a token's and the engine pins a match's, because a field
    /// added here is written that many times and the phase that writes the table
    /// runs at memory bandwidth.
    #[test]
    fn a_frame_is_the_width_the_table_is_counted_at() {
        assert_eq!(size_of::<EchoFrame>(), 32, "a frame is {} bytes", size_of::<EchoFrame>());
    }

    #[test]
    fn novel_then_echoed() {
        // First "whale" is novel; the second echoes it with the right lag.
        let bytes = b"the whale swam and the whale sang";
        let field = analyze_bytes(bytes);
        let toks = crate::lexer::lex(bytes);
        let whales: Vec<usize> = (0..toks.len())
            .filter(|&i| &bytes[toks[i].span()] == b"whale")
            .collect();
        assert_eq!(whales.len(), 2);
        let (a, b) = (field.frames[whales[0]], field.frames[whales[1]]);
        assert!(a.novel() && a.echoed(), "first whale is novel and echoed: {a:?}");
        assert!(!b.novel() && b.echoed(), "second whale echoes: {b:?}");
        assert_eq!(a.count, 2);
        assert_eq!(b.back_lag, NonZeroU32::new(toks[whales[1]].start - toks[whales[0]].start));
        assert_eq!(a.fwd_lag, b.back_lag);
    }

    #[test]
    fn unique_token_is_novel_never_echoed() {
        let field = analyze_bytes(b"one two three");
        for f in field.frames.iter().filter(|f| f.keyed) {
            assert!(f.novel() && !f.echoed(), "{f:?}");
            assert_eq!(f.strength(), 0.0);
        }
        assert_eq!(field.novelty(), 1.0);
        assert_eq!(field.echo_rate(), 0.0);
    }

    #[test]
    fn orbit_quotient_folds_case() {
        // Exact keying sees two distinct keys; the case quotient sees one echo.
        let bytes = b"Whale and whale";
        let exact = analyze_bytes(bytes);
        assert_eq!(exact.echoed, 0);
        let folded = analyze_with(
            &crate::lexer::lex(bytes),
            bytes,
            &EchoConfig { orbit: OrbitGroup::Case, ..Default::default() },
        );
        assert_eq!(folded.echoed, 2, "Whale/whale are one key under Case");
    }

    #[test]
    fn regular_recurrence_has_a_period() {
        // "tick" every 20 bytes: a periodic echo; the filler words are not.
        let line = "tick aa bb cc dd ee ".repeat(6);
        let field = analyze_bytes(line.as_bytes());
        let toks = crate::lexer::lex(line.as_bytes());
        let tick = (0..toks.len())
            .find(|&i| &line.as_bytes()[toks[i].span()] == b"tick")
            .expect("tick present");
        let p = field.frames[tick].period;
        assert!(p > 0.0, "tick recurs regularly, so it carries a period");
        assert!((p - 20.0).abs() < 1.0, "period ~20 bytes, got {p}");
    }

    #[test]
    fn punctuation_is_not_keyed() {
        let field = analyze_bytes(b"a , b , c , d");
        let toks = crate::lexer::lex(b"a , b , c , d");
        for (i, t) in toks.iter().enumerate() {
            if t.kind == TokenKind::Punct {
                assert!(!field.frames[i].keyed);
                assert!(!field.frames[i].echoed());
            }
        }
    }

    #[test]
    fn super_echo_finds_structural_rhyme() {
        // Three key: value lines with different words: one recurring structure.
        // The key is asserted whole rather than by its prefix, because the
        // silhouette is the half that carries the structure and a prefix test
        // passes whatever the silhouette is spelled as.
        let bytes = b"alpha: one\nbravo: two\ndelta: six\n";
        let rhymes = analyze_super(bytes);
        assert!(
            rhymes.iter().any(|r| r.count == 3 && r.key == "kv:W:W"),
            "three kv units rhyme structurally as kv:W:W: {rhymes:?}"
        );
    }

    /// Every branch of the spelling, since the key is what a reader reads.
    #[test]
    fn a_silhouette_spells_its_kinds() {
        let letter = |kind: TokenKind, text: &[u8]| {
            silhouette_letter(crate::shape::shape_class(kind, text))
        };
        assert_eq!(letter(TokenKind::Word, b"alpha"), 'W');
        assert_eq!(letter(TokenKind::Number, b"42"), 'N');
        assert_eq!(letter(TokenKind::Quoted, b"\"bob\""), 'Q');
        // Punctuation keeps its own glyph, which is what puts the colon in
        // the middle of `kv:W:W` rather than a separator doing it.
        assert_eq!(letter(TokenKind::Punct, b":"), ':');
        assert_eq!(letter(TokenKind::Punct, b","), ',');
        // Outside ASCII too: the glyph is the character, not its first byte.
        assert_eq!(letter(TokenKind::Punct, "、".as_bytes()), '、');
        assert_eq!(letter(TokenKind::Punct, "│".as_bytes()), '│');
        // A bracket carries no glyph in its code and is spelled from the pair.
        assert_eq!(letter(TokenKind::Open(crate::token::BracketKind::Brace), b"{"), '{');
        assert_eq!(letter(TokenKind::Close(crate::token::BracketKind::Square), b"]"), ']');
        // Everything else is one letter, so two typed kinds share it.
        assert_eq!(letter(TokenKind::Ip, b"10.0.0.1"), 'T');
        assert_eq!(letter(TokenKind::Email, b"bob@x.com"), 'T');
    }

    #[test]
    fn empty_is_safe() {
        let field = analyze_bytes(b"");
        assert!(field.frames.is_empty());
        assert_eq!(field.novelty(), 0.0);
        assert!(analyze_super(b"").is_empty());
    }

    /// A key seen more than once never reads as seen once.
    ///
    /// The whole of the unique skip rests on this one direction: a key that
    /// reads as seen once is given its own group and never reaches the map, so
    /// a key that echoed and read as unique would be reported novel. The other
    /// direction is allowed to be wrong - two keys sharing a counter both read
    /// twice and both go to the map, which answers the same and only costs the
    /// lookup.
    #[test]
    fn a_key_seen_twice_never_reads_as_seen_once() {
        // Serial-numbered identifiers, the shape that fills this table in real
        // source and the shape whose hashes are closest together.
        let keys: Vec<String> = (0..20_000).map(|i| format!("value_{i}")).collect();
        let table = Repeats::over(keys.len(), |t| {
            for k in &keys {
                t.saw(Repeats::index_of(k));
                t.saw(Repeats::index_of(k));
            }
        });
        for k in &keys {
            assert!(
                !table.once(Repeats::index_of(k)),
                "{k} was seen twice and must not read as once"
            );
        }
    }

    /// Counted once reads as once, counted twice does not, and counted not at
    /// all does not either.
    ///
    /// Read over hashes chosen here rather than over keys, so it says what the
    /// counters do and nothing about how well any hash spreads. How much of a
    /// real text reaches the skip is a property of that text and is measured
    /// rather than asserted: on the engine-surface corpus it took the keying of
    /// the echo field from 89.2 ms to 63.8.
    #[test]
    fn a_counter_tells_once_from_more_than_once() {
        let table = Repeats::over(64, |t| {
            t.saw(11);
            t.saw(11);
            t.saw(22);
        });
        assert!(!table.once(11), "counted twice");
        assert!(table.once(22), "counted once");
        assert!(!table.once(33), "never counted");
    }

    /// A counter saturates rather than carrying into the counter beside it.
    ///
    /// Nine appearances must read the same as two, and a neighbor must be
    /// untouched - a carry out of one counter would put a key that echoed into
    /// a slot reading one, which is the reading that skips the map.
    #[test]
    fn a_counter_saturates_and_leaves_its_neighbors_alone() {
        let table = Repeats::over(64, |t| {
            for _ in 0..9 {
                t.saw(7);
            }
            t.saw(8);
        });
        assert!(!table.once(7), "nine appearances read as more than once");
        assert!(table.once(8), "its neighbor is untouched");
    }
}
