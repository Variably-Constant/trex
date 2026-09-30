//! Orbit-based tokenization: the symmetry axis, orthogonal to frequency (BPE)
//! and predictability (seam).
//!
//! BPE asks *where to merge* (co-occurrence frequency); seam asks *where to
//! cut* (predictability). This module asks a third, orthogonal question:
//! *what is the same?* A token is the canonical representative of its **orbit**
//! under a symmetry group `G`, and tokenization is the quotient map
//! `bytes -> bytes / G`. Two spans that differ only by a symmetry of `G` are
//! one token.
//!
//! The principle is already in `crate::canon` (a checkable zero is an
//! invariant under a group; the zero is the canonical orbit representative):
//! the notation orbit (`theta` / `\theta` / the glyph), the commutative orbit
//! (`a+b` / `b+a`), and the register-renaming orbit (code blocks up to
//! register permutation). This module lifts that into a tokenization axis with
//! a ladder of groups.
//!
//! The ladder: identity, case, notation (reusing `crate::canon`), within-class
//! **shape** (a word's consonant/vowel/digit pattern, the orbit under
//! substituting one symbol for another of the same class), and the deepest rung
//! **E8** - a span's 8-channel profile snapped onto the self-dual E8 root lattice
//! and quotiented by the reflection group `W(E8)` (`crate::e8`).
//!
//! E8 is the only rung whose equivalence is not a string rewrite: it folds spans
//! whose channel profiles are related by a lattice symmetry, so spans with the
//! same distributional structure collapse regardless of which channel carried it.
//!
//! Three uses, all from one `canonical` function:
//! - **collapse** (identity): symmetry-equivalent spans become one token;
//! - **boundary** (orbit-change): cut where the symmetry class shifts;
//! - **match** (equivariance): a query matches every span in its orbit.

use std::collections::BTreeMap;

/// An orbit-collapse table: each orbit representative mapped to the distinct
/// raw forms that fold onto it.
pub type CollapseTable = BTreeMap<String, Vec<String>>;

/// A symmetry group whose orbits define token identity. The E8 Weyl orbit is
/// the next rung and slots in here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbitGroup {
    /// No symmetry: the token is its literal bytes (standard tokenization).
    Identity,
    /// Case folding: `Cat` = `cat` = `CAT`.
    Case,
    /// Notation: `theta` = `\theta` = the Greek glyph (via `crate::canon`),
    /// which also folds case.
    Notation,
    /// Within-class substitution: a span's consonant/vowel/digit/other shape,
    /// so `cat` = `dog` = `bat` (all `CVC`). The phonotactic-shape orbit.
    Shape,
    /// The E8 WEYL orbit: the span's 8-channel profile snapped onto the E8 root
    /// lattice and quotiented by the reflection group `W(E8)`.
    ///
    /// The deepest rung, and the only one whose equivalence is not a string
    /// rewrite. Where `Shape` folds spans that differ by substituting one symbol
    /// for another of the same class, this folds spans whose channel profiles are
    /// related by a symmetry of the lattice - a coordinate permutation, an even
    /// sign change, or any reflection in a root. So it identifies spans with the
    /// same distributional structure regardless of which channel carried it,
    /// which is a generalization no byte-level rewrite can express.
    E8,
    /// An address in any written form: `::1` = `0:0:0:0:0:0:0:1`,
    /// `2001:DB8::1` = `2001:db8:0:0:0:0:0:1`.
    Ip,
    /// A URL under RFC 3986 syntax-based normalization: the scheme and host
    /// case, a default port, an empty path, percent-encoding case and
    /// unreserved characters fold; the query, fragment and a trailing slash
    /// are kept as written.
    Url,
    /// A timestamp as the instant it names, so one moment in two forms or two
    /// zones is one token.
    Time,
    /// A filesystem path under either separator: `C:\a\b` = `C:/a/b`.
    Path,
    /// Text with compatibility forms decomposed, combining marks removed and
    /// case folded: `Café` = `cafe` = the fullwidth form.
    Fold,
    /// A number in any notation: `1,000` = `1000` = `1e3` = `0x3e8`.
    Numeric,
    /// A typed relation as a rung: the orbit is the set of texts one
    /// projection reads alike, so `subnet/24` folds every address of a
    /// network onto that network, `domain` every address at a mail domain
    /// onto the domain, and `day` every timestamp of one calendar day.
    ///
    /// The relation the same name spells after `=` is the pairwise form of
    /// this: `=domain e` asks whether one token relates to a bound one, and
    /// `(?orbit:domain ...)` names the projection both sides go through. They
    /// answer alike everywhere except the calendar relations, where the
    /// pairwise reading does not hold an absent year against a written one
    /// and so is no equivalence; the rung keeps the year and is finer.
    Typed(crate::typed::Relation),
}

impl OrbitGroup {
    /// Parse a group name from the CLI, a `(?orbit:G ...)` scope or an
    /// `=case x` reference: one of the rungs above, or the name of a typed
    /// relation, which is a rung of its own, with the width after a slash
    /// where the relation takes one (`subnet/24`).
    #[must_use]
    pub fn parse(name: &str) -> Option<OrbitGroup> {
        if let Some((base, width)) = name.split_once('/') {
            // Read digit by digit rather than through a parse that would hand
            // back an error this function has no way to carry: a name that is
            // not a group is `None`, and that is all a caller here can use.
            if width.is_empty() || width.len() > 3 || !width.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let bits = width.bytes().fold(0u32, |a, b| a * 10 + u32::from(b - b'0'));
            if bits > 128 {
                return None;
            }
            return OrbitGroup::parse(base)?.with_prefix(bits as u8);
        }
        match name {
            "identity" => Some(OrbitGroup::Identity),
            "case" => Some(OrbitGroup::Case),
            "notation" => Some(OrbitGroup::Notation),
            "shape" => Some(OrbitGroup::Shape),
            "e8" => Some(OrbitGroup::E8),
            "ip" => Some(OrbitGroup::Ip),
            "url" => Some(OrbitGroup::Url),
            "time" => Some(OrbitGroup::Time),
            "path" => Some(OrbitGroup::Path),
            "fold" => Some(OrbitGroup::Fold),
            "numeric" => Some(OrbitGroup::Numeric),
            other => crate::typed::Relation::parse(other).map(OrbitGroup::Typed),
        }
    }

    /// The rung with a prefix length, for the one relation that takes one:
    /// `(?orbit:subnet/24 ...)`.
    #[must_use]
    pub fn with_prefix(self, bits: u8) -> Option<OrbitGroup> {
        match self {
            OrbitGroup::Typed(rel) => rel.with_prefix(bits).map(OrbitGroup::Typed),
            _ => None,
        }
    }

    /// A short, stable label. A typed rung carries its relation's name, so
    /// `subnet/24` and a bare `subnet` label alike, as the relation does.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            OrbitGroup::Identity => "identity",
            OrbitGroup::Case => "case",
            OrbitGroup::Notation => "notation",
            OrbitGroup::Shape => "shape",
            OrbitGroup::E8 => "e8",
            OrbitGroup::Ip => "ip",
            OrbitGroup::Url => "url",
            OrbitGroup::Time => "time",
            OrbitGroup::Path => "path",
            OrbitGroup::Fold => "fold",
            OrbitGroup::Numeric => "numeric",
            OrbitGroup::Typed(rel) => rel.label(),
        }
    }
}

/// The coarse symbol kind of a byte (alpha / digit / space / other), one level
/// above the vowel/consonant shape. The orbit-change boundary stratifies the
/// stream by this kind: a cut falls where the kind of symbol changes.
fn kind_char(b: u8) -> char {
    if b.is_ascii_alphabetic() {
        'A'
    } else if b.is_ascii_digit() {
        'D'
    } else if b.is_ascii_whitespace() {
        'S'
    } else {
        '.'
    }
}

/// The character shape class of a byte: vowel, consonant, digit, or other -
/// the alphabet the within-class substitution group permutes within.
#[must_use]
pub fn shape_char(b: u8) -> char {
    if b.is_ascii_digit() {
        'D'
    } else if matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'A' | b'E' | b'I' | b'O' | b'U') {
        'V'
    } else if b.is_ascii_alphabetic() {
        'C'
    } else {
        '.'
    }
}

/// The canonical representative of `span`'s orbit under `group`. Two spans are
/// the same token under `group` exactly when their canonical forms are equal.
#[must_use]
pub fn canonical(span: &[u8], group: OrbitGroup) -> String {
    let text = String::from_utf8_lossy(span);
    match group {
        OrbitGroup::Identity => text.into_owned(),
        OrbitGroup::Case => text.to_lowercase(),
        OrbitGroup::Notation => crate::canon::canon_symbol(&text),
        OrbitGroup::Shape => span.iter().map(|&b| shape_char(b)).collect(),
        OrbitGroup::E8 => {
            let c = crate::e8::canonicalize(&embed_e8(span));
            // Rendered from the doubled integer coordinates, so the representative is an exact
            // string: two spans produce the same token exactly when their canonical vectors are
            // equal, with no formatting-precision question in between.
            let mut s = String::with_capacity(40);
            s.push_str("E8[");
            for (i, x) in c.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push_str(&x.to_string());
            }
            s.push(']');
            s
        }
        // The typed rungs fold a representation onto the value it names; text
        // that is not of the type is its own representative.
        OrbitGroup::Ip => match crate::typed::parse_ip(&text) {
            Some(ip) => ip.to_string(),
            None => text.into_owned(),
        },
        OrbitGroup::Url => crate::typed::canonical_url(&text),
        OrbitGroup::Time => crate::typed::canonical_time(&text),
        OrbitGroup::Path => text.replace('\\', "/"),
        OrbitGroup::Fold => crate::typed::fold_text(&text),
        OrbitGroup::Numeric => crate::typed::canonical_number(&text),
        // The projection's value under the relation's name, and for text the
        // relation cannot read, that text behind a `?`. The two cannot
        // collide: a token spelled `domain:example.com` keys as
        // `?domain:example.com`, so a bare word never folds onto the domain
        // read out of an address - which across two inputs would be a join
        // reported where there is none.
        OrbitGroup::Typed(rel) => match rel.key(&text) {
            Some(key) => format!("{}:{key}", rel.label()),
            None => format!("?{text}"),
        },
    }
}

/// The eight channels a span is measured on before it meets the lattice.
///
/// Chosen to be a profile rather than a sequence: the E8 orbit quotients by coordinate
/// permutation among other symmetries, so what survives is the span's distributional shape, not
/// which channel held which count. Sequence information is the other rungs' job.
pub const E8_CHANNELS: usize = 8;

/// Embed a span as a doubled E8 lattice point.
///
/// Counts per channel (vowel, consonant, digit, space, punctuation, uppercase, high-bit, length
/// band), doubled so the value is exact in the lattice's integer coordinates, then snapped onto
/// E8. Counts are capped so one long span cannot dominate the profile - the orbit is meant to
/// describe shape, and an uncapped length term would make it describe size instead.
#[must_use]
pub fn embed_e8(span: &[u8]) -> crate::e8::E8Vec {
    const CAP: i64 = 12;
    let mut ch = [0i64; E8_CHANNELS];
    for &b in span {
        let i = match shape_char(b) {
            'V' => 0,
            'C' => 1,
            'D' => 2,
            _ if b == b' ' || b == b'\t' || b == b'\n' => 3,
            _ if b.is_ascii_punctuation() => 4,
            _ => 6,
        };
        ch[i] += 1;
        if b.is_ascii_uppercase() {
            ch[5] += 1;
        }
    }
    // Channel 7 is a LOG length band rather than the raw length, for the same reason the counts
    // are capped: a linear length term would swamp every other channel on long spans.
    ch[7] = (span.len() as f64 + 1.0).log2() as i64;
    let mut v = [0i64; 8];
    for i in 0..8 {
        v[i] = ch[i].min(CAP) * 2;
    }
    // SORT DESCENDING BEFORE SNAPPING. This is not cosmetic and it is not an optimization.
    //
    // `snap` repairs the even-sum condition by moving the coordinate it rounded hardest, and
    // when several coordinates tie it breaks the tie by POSITION. That makes snap
    // position-dependent, so two profiles that are coordinate permutations of each other (three
    // vowels vs three consonants - the same shape on a different channel) land on different
    // lattice points and never fold, which defeats the entire purpose of quotienting by a group
    // that contains those permutations. A test caught exactly that.
    //
    // Sorting applies the permutation part of the quotient FIRST, so snap sees one canonical
    // input per permutation class and its tie-break can no longer split an orbit. `canonicalize`
    // then handles the rest of the group - sign changes and root reflections.
    v.sort_unstable_by(|a, b| b.cmp(a));
    crate::e8::snap(&v)
}

/// Whether two spans lie in the same orbit under `group` (the equivariance
/// test - the basis of orbit-aware matching).
#[must_use]
pub fn same_orbit(a: &[u8], b: &[u8], group: OrbitGroup) -> bool {
    canonical(a, group) == canonical(b, group)
}

/// One orbit token: the source span, its raw text, and its orbit
/// representative under the active group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrbitToken {
    /// Inclusive start byte offset.
    pub start: usize,
    /// Exclusive end byte offset.
    pub end: usize,
    /// The literal span text.
    pub raw: String,
    /// The canonical orbit representative (the token's identity).
    pub orbit: String,
}

/// Tokenize `input` into orbit tokens: the base lexer's significant tokens,
/// each mapped to its orbit representative under `group`.
#[must_use]
pub fn tokenize(input: &[u8], group: OrbitGroup) -> Vec<OrbitToken> {
    crate::lexer::lex(input)
        .into_iter()
        .filter(crate::token::Token::is_significant)
        .map(|t| {
            let span = &input[t.span()];
            OrbitToken {
                start: t.start(),
                end: t.end(),
                raw: String::from_utf8_lossy(span).into_owned(),
                orbit: canonical(span, group),
            }
        })
        .collect()
}

/// The orbit-collapse table: each orbit representative to the distinct raw
/// forms that fold onto it (sorted), under `group`. The keys are the token
/// vocabulary; an entry with more than one value is a genuine collapse.
#[must_use]
pub fn collapse(input: &[u8], group: OrbitGroup) -> CollapseTable {
    let mut table: CollapseTable = BTreeMap::new();
    for t in tokenize(input, group) {
        let forms = table.entry(t.orbit).or_default();
        if !forms.contains(&t.raw) {
            forms.push(t.raw);
        }
    }
    for forms in table.values_mut() {
        forms.sort();
    }
    table
}

/// `(distinct raw forms, distinct orbits)` - the vocabulary reduction the
/// quotient achieves under `group`.
#[must_use]
pub fn collapse_stats(input: &[u8], group: OrbitGroup) -> (usize, usize) {
    let toks = tokenize(input, group);
    let mut raw: Vec<&str> = toks.iter().map(|t| t.raw.as_str()).collect();
    raw.sort_unstable();
    raw.dedup();
    let mut orb: Vec<&str> = toks.iter().map(|t| t.orbit.as_str()).collect();
    orb.sort_unstable();
    orb.dedup();
    (raw.len(), orb.len())
}

/// Orbit-change boundaries: the byte offsets where the per-byte shape class
/// changes - the stratification of the stream into maximal same-class runs.
/// This is the symmetry-axis boundary signal, orthogonal to frequency and
/// predictability: it cuts where the *kind* of symbol changes, not where the
/// statistics do.
#[must_use]
pub fn shape_boundaries(input: &[u8]) -> Vec<usize> {
    let mut cuts = Vec::new();
    let mut prev: Option<char> = None;
    for (i, &b) in input.iter().enumerate() {
        let c = kind_char(b);
        if prev.is_some_and(|p| p != c) {
            cuts.push(i);
        }
        prev = Some(c);
    }
    cuts
}

/// Every span (start, end, raw) in `input` whose orbit equals `query`'s orbit
/// under `group` - an equivariant match: the query matches up to the group,
/// not byte-for-byte.
#[must_use]
pub fn matches(input: &[u8], query: &[u8], group: OrbitGroup) -> Vec<OrbitToken> {
    let key = canonical(query, group);
    tokenize(input, group).into_iter().filter(|t| t.orbit == key).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The E8 rung is a genuine quotient map at the token level: equal spans agree, the
    /// representative is stable under repetition, and the output is a dominant lattice point.
    #[test]
    fn e8_orbit_is_a_stable_quotient_over_spans() {
        for s in [&b"hello"[..], b"WORLD", b"a1b2c3", b"   ", b"!!!", b""] {
            let a = canonical(s, OrbitGroup::E8);
            assert_eq!(a, canonical(s, OrbitGroup::E8), "canonical form is deterministic for {s:?}");
            assert!(a.starts_with("E8["), "the representative is rendered from lattice coordinates: {a}");
            let v = embed_e8(s);
            assert!(crate::e8::in_e8(&v), "the embedding lands on the lattice for {s:?}: {v:?}");
            assert!(crate::e8::is_dominant(&crate::e8::canonicalize(&v)), "the representative is dominant for {s:?}");
        }
    }

    /// What the rung BUYS, stated as a discrimination rather than a claim: spans whose channel
    /// profiles are lattice-symmetric fold together, while a span with a genuinely different
    /// profile does not. Without the second half this test would pass on a function that folds
    /// everything, which is the failure mode a quotient map is most prone to.
    #[test]
    fn e8_folds_symmetry_related_profiles_and_separates_unrelated_ones() {
        // Same profile with the counts on different channels: three vowels vs three consonants.
        // A coordinate permutation is in W(E8), so these are one orbit - and no other rung folds
        // them, which is exactly what makes this rung worth having.
        let vowels = b"aei";
        let consonants = b"bcd";
        assert_eq!(
            canonical(vowels, OrbitGroup::E8),
            canonical(consonants, OrbitGroup::E8),
            "channel-permuted profiles are one E8 orbit"
        );
        assert_ne!(
            canonical(vowels, OrbitGroup::Shape),
            canonical(consonants, OrbitGroup::Shape),
            "the SHAPE rung keeps them apart - so E8 is adding a fold, not duplicating one"
        );
        // A structurally different span must not fold in: a long mixed run has a different
        // profile shape, not a permuted one.
        assert_ne!(
            canonical(vowels, OrbitGroup::E8),
            canonical(b"a1! bcdefgh", OrbitGroup::E8),
            "an unrelated profile stays a different token - the quotient is not collapsing everything"
        );
    }

    /// The group round-trips through its own CLI vocabulary, so `--orbit e8` reaches the rung.
    #[test]
    fn e8_parses_and_labels() {
        assert_eq!(OrbitGroup::parse("e8"), Some(OrbitGroup::E8));
        assert_eq!(OrbitGroup::E8.label(), "e8");
        assert_eq!(OrbitGroup::parse(OrbitGroup::E8.label()), Some(OrbitGroup::E8), "label and parse are inverses");
    }

    #[test]
    fn notation_orbit_collapses_encodings() {
        // The shipped notation orbit, lifted to tokens: glyph, command, and
        // bare name are one token; case folds too.
        assert!(same_orbit("\u{03B8}".as_bytes(), b"theta", OrbitGroup::Notation));
        assert!(same_orbit(b"\\theta", b"Theta", OrbitGroup::Notation));
        assert!(!same_orbit(b"theta", b"phi", OrbitGroup::Notation));
    }

    #[test]
    fn shape_orbit_unifies_words_of_one_pattern() {
        // The phonotactic-shape orbit: distinct words, one orbit.
        assert_eq!(canonical(b"cat", OrbitGroup::Shape), "CVC");
        assert!(same_orbit(b"cat", b"dog", OrbitGroup::Shape));
        assert!(same_orbit(b"cat", b"bat", OrbitGroup::Shape));
        assert!(!same_orbit(b"cat", b"the", OrbitGroup::Shape)); // CVC vs CCV
    }

    #[test]
    fn typed_rungs_fold_representations_of_one_value() {
        assert!(same_orbit(b"::1", b"0:0:0:0:0:0:0:1", OrbitGroup::Ip));
        assert!(same_orbit(b"2001:DB8::1", b"2001:db8:0:0:0:0:0:1", OrbitGroup::Ip));
        assert!(!same_orbit(b"10.0.0.1", b"10.0.0.2", OrbitGroup::Ip));
        assert!(same_orbit(b"HTTP://Example.COM:80/a", b"http://example.com/a", OrbitGroup::Url));
        assert!(!same_orbit(b"http://example.com/a/", b"http://example.com/a", OrbitGroup::Url));
        assert!(same_orbit(b"2026-09-15T02:00:00+02:00", b"2026-09-15 00:00:00", OrbitGroup::Time));
        assert!(same_orbit(b"C:\\a\\b", b"C:/a/b", OrbitGroup::Path));
        assert!(same_orbit("Caf\u{E9}".as_bytes(), b"cafe", OrbitGroup::Fold));
        assert!(same_orbit(b"1,000", b"1e3", OrbitGroup::Numeric));
        assert!(!same_orbit(b"1,000", b"1001", OrbitGroup::Numeric));
        for name in ["ip", "url", "time", "path", "fold", "numeric"] {
            let g = OrbitGroup::parse(name).unwrap_or_else(|| panic!("{name} is a rung"));
            assert_eq!(g.label(), name);
        }
    }

    /// Each typed relation is a rung of the same name: the orbit is the set
    /// of texts one projection reads alike, and a text the projection cannot
    /// read is its own representative behind a `?`.
    #[test]
    fn a_typed_relation_is_a_rung_that_folds_what_its_projection_reads_alike() {
        let rung = |name: &str| OrbitGroup::parse(name).unwrap_or_else(|| panic!("{name} is a rung"));
        let subnet24 = rung("subnet").with_prefix(24).unwrap_or_else(|| panic!("subnet takes one"));
        assert!(same_orbit(b"10.0.0.7", b"10.0.0.201", subnet24));
        assert!(!same_orbit(b"10.0.0.7", b"10.0.1.201", subnet24));
        assert_eq!(canonical(b"10.0.0.7", subnet24), "subnet:10.0.0.0/24");
        assert!(rung("domain").with_prefix(24).is_none());
        assert!(same_orbit(b"bob@corp.example", b"amy@corp.example", rung("domain")));
        assert!(!same_orbit(b"bob@corp.example", b"amy@other.example", rung("domain")));
        assert!(same_orbit(b"2026-09-15T01:00:00", b"2026-09-15T23:00:00", rung("day")));
        // The rung keeps the year, so two days a year apart never fold - a
        // join across two logs must not report a match across years.
        assert!(!same_orbit(b"2026-09-15T01:00:00", b"2025-09-15T23:00:00", rung("day")));
        // A text the projection cannot read stands for itself, and cannot
        // collide with a projected key however it is spelled.
        assert_eq!(canonical(b"plainword", rung("domain")), "?plainword");
        assert_eq!(canonical(b"domain:corp.example", rung("domain")), "?domain:corp.example");
        assert!(!same_orbit(b"domain:corp.example", b"bob@corp.example", rung("domain")));
        assert!(same_orbit(b"alpha", b"alpha", rung("domain")));
        assert!(!same_orbit(b"alpha", b"bravo", rung("domain")));
        for name in ["subnet", "domain", "day", "host", "len", "magnitude"] {
            assert_eq!(rung(name).label(), name, "a rung labels as the relation it is");
            assert_eq!(OrbitGroup::parse(rung(name).label()), Some(rung(name)));
        }
        // The width is written after a slash wherever a rung is named, so a
        // CLI flag and a `(?orbit:...)` scope take one spelling.
        assert_eq!(OrbitGroup::parse("subnet/24"), Some(subnet24));
        assert_eq!(OrbitGroup::parse("subnet/129"), None);
        assert_eq!(OrbitGroup::parse("subnet/"), None);
        assert_eq!(OrbitGroup::parse("subnet/x"), None);
        assert_eq!(OrbitGroup::parse("domain/24"), None);
        assert_eq!(OrbitGroup::parse("case/24"), None);
    }

    #[test]
    fn case_orbit_folds_case_only() {
        assert!(same_orbit(b"Cat", b"CAT", OrbitGroup::Case));
        assert!(!same_orbit(b"cat", b"dog", OrbitGroup::Case));
    }

    #[test]
    fn collapse_reduces_vocabulary() {
        // Five CVC words plus a CCV word: shape collapses six raw forms to two
        // orbits.
        let (raw, orb) = collapse_stats(b"cat dog bat sat mat the", OrbitGroup::Shape);
        assert_eq!(raw, 6, "six distinct raw words");
        assert_eq!(orb, 2, "two shape orbits: CVC and CCV");
        // Identity collapses nothing.
        let (raw_i, orb_i) = collapse_stats(b"cat dog bat sat mat the", OrbitGroup::Identity);
        assert_eq!(raw_i, orb_i, "identity is the trivial quotient");
    }

    #[test]
    fn generalizes_to_unseen_strings() {
        // An unseen word maps to a KNOWN orbit (generalization, not a lookup):
        // `vat` was never in the text yet shares the CVC orbit of `cat`.
        let table = collapse(b"cat dog", OrbitGroup::Shape);
        assert!(table.contains_key("CVC"));
        assert_eq!(canonical(b"vat", OrbitGroup::Shape), "CVC");
    }

    #[test]
    fn shape_boundaries_cut_at_class_changes() {
        // `cat123dog`: letters | digits | letters - boundaries at the class
        // transitions (3 and 6).
        let cuts = shape_boundaries(b"cat123dog");
        assert_eq!(cuts, vec![3, 6]);
    }

    #[test]
    fn equivariant_match_finds_the_whole_orbit() {
        // A case query matches every case variant in the text.
        let hits = matches(b"the Cat and the CAT and a cat", b"cat", OrbitGroup::Case);
        assert_eq!(hits.len(), 3, "Cat, CAT, cat all match up to case: {hits:?}");
    }
}
