//! Pattern inference from examples: the most specific pattern every example
//! matches, read off an alignment of their token sequences. A position
//! where every example has the same text is that literal; one where the
//! kinds agree and the texts differ is the kind's atom; one where the kinds
//! differ is the class of the kinds seen; a run of one kind whose length
//! differs between examples is that kind repeated with the bounds seen; a
//! position some examples lack is optional. The pattern is verified against
//! every example before it is returned.

pub mod build;
pub mod marks;

use crate::token::TokenKind;

/// Why no pattern was inferred.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InferError {
    /// Fewer than two examples were given.
    TooFew(usize),
    /// The example at this index has no significant token.
    Empty(usize),
    /// The pattern built does not parse, which is a defect in its spelling.
    Unparsed { pattern: String, msg: String },
    /// The pattern built does not match the example at this index from its
    /// first token to its last.
    Unverified { example: usize, pattern: String },
    /// The pattern built still matches the counter-example at this index,
    /// and no range the examples support excludes it.
    Matched { counter: usize, pattern: String },
}

impl std::fmt::Display for InferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InferError::TooFew(n) => {
                write!(f, "inference needs at least two examples; {n} given")
            }
            InferError::Empty(i) => write!(f, "example {} has no token to read", i + 1),
            InferError::Unparsed { pattern, msg } => {
                write!(f, "the inferred pattern `{pattern}` does not parse: {msg}")
            }
            InferError::Unverified { example, pattern } => write!(
                f,
                "the inferred pattern `{pattern}` does not match example {} whole",
                example + 1
            ),
            InferError::Matched { counter, pattern } => write!(
                f,
                "the inferred pattern `{pattern}` still matches counter-example {}; \
                 nothing the examples have in common tells them apart",
                counter + 1
            ),
        }
    }
}

/// What the examples agree on at a position beyond their kinds, where their
/// texts do not all agree.
///
/// The two are ordered: a rung is tried before a range, because a rung still
/// names the text every example had, under a folding, while a range only
/// bounds the value. Nothing here is reached when the texts agree outright,
/// which is a literal and narrower than either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Narrowing {
    /// The texts differ but are one text under an orbit rung.
    Folded(crate::orbit::OrbitGroup, String),
    /// Every example's value is in one range of the kind's own units, spelled
    /// as the predicate body that says so: `200..299`, `in:10.0.0.0/8`.
    Range(String),
}

/// One position of the inferred pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    /// The kinds seen here, in the order first seen.
    pub kinds: Vec<TokenKind>,
    /// The text every example has here, when they all agree.
    pub text: Option<String>,
    /// What the texts agree on when they do not all agree, where anything
    /// does. A range here is a candidate, not a decision: it is printed only
    /// where a counter-example rules it in, which [`infer_against`] settles.
    pub narrowing: Option<Narrowing>,
    /// The fewest tokens an example has here.
    pub min: usize,
    /// The most tokens an example has here.
    pub max: usize,
}

impl Slot {
    /// The slot as the pattern language spells it, trying the forms from the
    /// most specific down: the text every example has, the text they share
    /// under an orbit rung, the range holding every value, the class of the
    /// kinds seen, and last the bare kind. Each step gives up exactly one
    /// thing the step above it kept. The repeat follows where the count
    /// varies, and a kind no atom names makes the slot `.`, which matches
    /// any token.
    #[must_use]
    pub fn spelled(&self) -> String {
        let atom = match (&self.text, &self.narrowing) {
            (Some(text), _) => crate::templates::quote(text),
            (None, Some(Narrowing::Folded(group, text))) => {
                format!("(?orbit:{} {})", group.label(), crate::templates::quote(text))
            }
            (None, narrowing) => {
                let atoms: Option<Vec<String>> = self.kinds.iter().map(|k| atom_of(*k)).collect();
                match atoms {
                    Some(atoms) if atoms.len() == 1 => match narrowing {
                        Some(Narrowing::Range(range)) => format!("{}{{{range}}}", atoms[0]),
                        _ => atoms[0].clone(),
                    },
                    Some(atoms) => format!("[{}]", atoms.join(" ")),
                    None => ".".to_string(),
                }
            }
        };
        match (self.min, self.max) {
            (1, 1) => atom,
            (0, 1) => format!("{atom}?"),
            (a, b) if a == b => format!("{atom}{{{a}}}"),
            (a, b) => format!("{atom}{{{a},{b}}}"),
        }
    }
}

/// The atom that names a kind, or `None` for a kind no atom names: a
/// declared kind without its declaration, a bracket, an unclassified token.
fn atom_of(kind: TokenKind) -> Option<String> {
    match kind {
        TokenKind::Custom(_) | TokenKind::Open(_) | TokenKind::Close(_) | TokenKind::Other => None,
        k => Some(match k.escape() {
            Some(c) => format!("\\{c}"),
            None => format!("\\{{{}}}", k.name()),
        }),
    }
}

/// The orbit rungs an inferred pattern may fold a position to, tried in this
/// order, narrowest first.
///
/// Every one of these folds two spellings of one thing, so the literal
/// printed under it still names what every example had. `shape` and `e8` are
/// deliberately absent: they fold spans that merely share a structure, so
/// `cat` and `dog` are one span under `shape`, and a pattern folded there
/// would match words no example resembled.
const FOLDING_RUNGS: [crate::orbit::OrbitGroup; 8] = [
    crate::orbit::OrbitGroup::Case,
    crate::orbit::OrbitGroup::Notation,
    crate::orbit::OrbitGroup::Numeric,
    crate::orbit::OrbitGroup::Ip,
    crate::orbit::OrbitGroup::Url,
    crate::orbit::OrbitGroup::Time,
    crate::orbit::OrbitGroup::Path,
    crate::orbit::OrbitGroup::Fold,
];

/// What one position's differing texts agree on: the rung folding them to
/// one text, else the range holding every value, else nothing.
///
/// A rung is tried first because it still names the text every example had,
/// where a range only bounds the value. Both are read only at a position
/// where every example has a token: a position some example lacks has no
/// value there to bound and no text there to fold.
fn narrowing_of(column: &Column) -> Option<Narrowing> {
    let texts: Option<Vec<&str>> = column.texts.iter().map(|t| t.as_deref()).collect();
    let texts = texts?;
    for &group in &FOLDING_RUNGS {
        let mut folded = texts.iter().map(|t| crate::orbit::canonical(t.as_bytes(), group));
        let first = folded.next()?;
        if folded.all(|f| f == first) {
            return Some(Narrowing::Folded(group, first));
        }
    }
    let [kind] = column.kinds[..] else { return None };
    range_of(kind, &texts).map(Narrowing::Range)
}

/// The range holding every text's value, as the predicate body `\K{...}`
/// takes, or `None` where the kind has no range form or the texts do not
/// share one.
///
/// Each kind rounds outward to a boundary its own units have rather than
/// stopping at the tightest span the examples showed, so the pattern
/// describes the class the examples came from: three status codes give
/// `200..299`, not `200..204`, which tomorrow's 206 would miss.
///
/// The range forms exist for these four kinds. A size or a duration has a
/// value and no natural boundary to round to - no unit says where the band
/// around `1.5GiB` ends - so neither takes a range.
fn range_of(kind: TokenKind, texts: &[&str]) -> Option<String> {
    match kind {
        TokenKind::Number => numeric_band(texts),
        TokenKind::Ip => address_block(texts),
        TokenKind::Timestamp => calendar_unit(texts),
        TokenKind::Version => version_line(texts),
        _ => None,
    }
}

/// The band of whole numbers holding every text, both ends rounded to the
/// leading digit place of the larger: `200 201 204` gives `200..299` and
/// `8080 8443` gives `8000..8999`.
///
/// Whole numbers only. A number written with a fraction has no digit place
/// to round to that is not an arbitrary choice of precision, so it takes no
/// band, and neither does one whose value prints as a power of ten, since a
/// band is written out in digits.
fn numeric_band(texts: &[&str]) -> Option<String> {
    let mut digits: Vec<String> = Vec::with_capacity(texts.len());
    for text in texts {
        let value = crate::typed::number_value(text)?.to_text();
        if value.contains(['.', 'e']) || value.starts_with('-') {
            return None;
        }
        digits.push(value);
    }
    // A longer digit string is the larger number, and two of one length
    // compare as their bytes, so this is the numeric order without a width
    // to parse into.
    let by_value = |a: &&String, b: &&String| a.len().cmp(&b.len()).then_with(|| a.cmp(b));
    let low = digits.iter().min_by(by_value)?;
    let high = digits.iter().max_by(by_value)?;
    let width = high.len();
    // A number shorter than the widest is below every band of the widest's
    // leading digit, so the floor is 0.
    let lead = |s: &String| if s.len() < width { '0' } else { s.as_bytes()[0] as char };
    let band = |first: char, rest: char| -> String {
        std::iter::once(first).chain(std::iter::repeat_n(rest, width - 1)).collect()
    };
    let floor = band(lead(low), '0');
    let floor = floor.trim_start_matches('0');
    Some(format!("{}..{}", if floor.is_empty() { "0" } else { floor }, band(lead(high), '9')))
}

/// The smallest CIDR block holding every address: the bits they share, with
/// the rest zeroed. Addresses of two families share no block.
fn address_block(texts: &[&str]) -> Option<String> {
    use std::net::IpAddr;

    let mut addresses: Vec<IpAddr> = Vec::with_capacity(texts.len());
    for text in texts {
        match crate::typed::value_of(TokenKind::Ip, text)? {
            crate::typed::TypedValue::Ip(address) => addresses.push(address),
            _ => return None,
        }
    }
    let first = *addresses.first()?;
    let octets = |a: &IpAddr| -> Vec<u8> {
        match a {
            IpAddr::V4(v4) => v4.octets().to_vec(),
            IpAddr::V6(v6) => v6.octets().to_vec(),
        }
    };
    let head = octets(&first);
    let rest: Vec<Vec<u8>> = addresses.iter().map(octets).collect();
    if rest.iter().any(|o| o.len() != head.len()) {
        return None;
    }
    let mut shared = 0usize;
    'bits: for (i, byte) in head.iter().enumerate() {
        for bit in (0..8).rev() {
            let mask = 1u8 << bit;
            if rest.iter().any(|o| (o[i] & mask) != (byte & mask)) {
                break 'bits;
            }
            shared += 1;
        }
    }
    let mut network = head;
    for (i, byte) in network.iter_mut().enumerate() {
        let kept = u32::try_from(shared.saturating_sub(i * 8).min(8)).unwrap_or(8);
        *byte &= u8::MAX.checked_shl(8 - kept).unwrap_or(0);
    }
    // The masked bytes came from this address's own family, so each copy is
    // exactly as wide as the buffer it fills.
    let address = match first {
        IpAddr::V4(_) => {
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&network);
            IpAddr::from(bytes)
        }
        IpAddr::V6(_) => {
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&network);
            IpAddr::from(bytes)
        }
    };
    Some(format!("in:{address}/{shared}"))
}

/// The smallest calendar unit holding every timestamp, as the field clauses
/// that name it: the hour where they share one, else the day, the month or
/// the year. Timestamps that share no year, or that carry no date, have no
/// unit in common.
fn calendar_unit(texts: &[&str]) -> Option<String> {
    let mut stamps: Vec<crate::typed::Civil> = Vec::with_capacity(texts.len());
    for text in texts {
        match crate::typed::value_of(TokenKind::Timestamp, text)? {
            crate::typed::TypedValue::Instant(civil) if civil.has_date => stamps.push(civil),
            _ => return None,
        }
    }
    let first = stamps.first()?;
    let year = first.year?;
    let agree = |read: &dyn Fn(&crate::typed::Civil) -> Option<i32>| {
        let head = read(first);
        head.is_some() && stamps.iter().all(|s| read(s) == head)
    };
    if !agree(&|s| s.year) {
        return None;
    }
    if !agree(&|s| Some(i32::from(s.month))) {
        return Some(format!("year={year}"));
    }
    let mut clauses = format!("year={year},month={}", first.month);
    if !agree(&|s| Some(i32::from(s.day))) {
        return Some(clauses);
    }
    clauses.push_str(&format!(",day={}", first.day));
    if first.has_time && agree(&|s| Some(i32::from(s.hour))) {
        clauses.push_str(&format!(",hour={}", first.hour));
    }
    Some(clauses)
}

/// The version line every version is on, as the field clauses that name
/// it: the minor line where they share one, else the major. Versions sharing
/// no major are on no line.
fn version_line(texts: &[&str]) -> Option<String> {
    let mut parts: Vec<Vec<String>> = Vec::with_capacity(texts.len());
    for text in texts {
        match crate::typed::value_of(TokenKind::Version, text)? {
            crate::typed::TypedValue::Version(v) => {
                parts.push(v.parts().iter().map(crate::typed::Decimal::to_text).collect());
            }
            _ => return None,
        }
    }
    let first = parts.first()?;
    let shared = |n: usize| first.len() > n && parts.iter().all(|p| p.len() > n && p[n] == first[n]);
    if !shared(0) {
        return None;
    }
    let mut clauses = format!("major={}", first[0]);
    if shared(1) {
        clauses.push_str(&format!(",minor={}", first[1]));
    }
    Some(clauses)
}

/// The pattern inferred from a set of examples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inferred {
    /// The positions, in order.
    pub slots: Vec<Slot>,
    pattern: String,
}

impl Inferred {
    /// The pattern, unanchored.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The pattern anchored to whole lines: `^` before its first token and
    /// `$` before its last.
    #[must_use]
    pub fn anchored(&self) -> String {
        anchor(&self.pattern)
    }
}

/// `pattern` with `^` before its first slot and `$` before its last.
fn anchor(pattern: &str) -> String {
    match pattern.rsplit_once(' ') {
        Some((head, last)) => format!("^ {head} $ {last}"),
        None => format!("^ $ {pattern}"),
    }
}

/// A token as the alignment reads it.
struct Tok {
    kind: TokenKind,
    text: String,
    start: usize,
    end: usize,
}

/// One aligned position while the examples are being folded in.
struct Column {
    kinds: Vec<TokenKind>,
    text: Option<String>,
    /// The text each example has here, `None` for an example with no token
    /// at this position. An alignment pairs one token per example with a
    /// column, so each entry is written at most once.
    ///
    /// This is what a narrowing is read from. [`Self::text`] is the text
    /// they all share and is `None` the moment two differ, which is the
    /// case a rung or a range exists to describe, so it cannot be the
    /// source.
    texts: Vec<Option<String>>,
}

impl Column {
    fn new(examples: usize, example: usize, tok: &Tok) -> Column {
        let mut texts = vec![None; examples];
        texts[example] = Some(tok.text.clone());
        Column { kinds: vec![tok.kind], text: Some(tok.text.clone()), texts }
    }

    /// Fold `tok` of `example` into this position.
    fn absorb(&mut self, example: usize, tok: &Tok) {
        if !self.kinds.contains(&tok.kind) {
            self.kinds.push(tok.kind);
        }
        if self.text.as_deref() != Some(tok.text.as_str()) {
            self.text = None;
        }
        self.texts[example] = Some(tok.text.clone());
    }

    /// Whether each example has a token here.
    fn present(&self) -> impl Iterator<Item = bool> + '_ {
        self.texts.iter().map(Option::is_some)
    }
}

/// One step of an alignment between the columns so far and an example.
enum Op {
    /// The column and the token are at the same position.
    Match(usize, usize),
    /// The column has no token in this example.
    Column(usize),
    /// The token has no column yet.
    Token(usize),
}

/// The longest common subsequence of the columns and the tokens, a column
/// and a token pairing when the token's kind is one the column has seen,
/// as the steps that walk both from start to end. Among the alignments with
/// the most pairings, the one pairing the most equal texts wins, so a word
/// both examples share stays a literal rather than pairing with a neighbor
/// of the same kind.
fn align(columns: &[Column], seq: &[Tok]) -> Vec<Op> {
    let (m, k) = (columns.len(), seq.len());
    // What pairing column `i` with token `j` adds: a kind match, and a text
    // match where the column is still one text and the token has it.
    let gain = |i: usize, j: usize| -> Option<(u32, u32)> {
        let column = &columns[i];
        let tok = &seq[j];
        if !column.kinds.contains(&tok.kind) {
            return None;
        }
        Some((1, u32::from(column.text.as_deref() == Some(tok.text.as_str()))))
    };
    let mut best = vec![vec![(0u32, 0u32); k + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=k {
            let mut here = best[i - 1][j].max(best[i][j - 1]);
            if let Some((kind, text)) = gain(i - 1, j - 1) {
                let paired = (best[i - 1][j - 1].0 + kind, best[i - 1][j - 1].1 + text);
                here = here.max(paired);
            }
            best[i][j] = here;
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (m, k);
    while i > 0 || j > 0 {
        let paired = if i > 0 && j > 0 {
            gain(i - 1, j - 1).map(|(kind, text)| (best[i - 1][j - 1].0 + kind, best[i - 1][j - 1].1 + text))
        } else {
            None
        };
        if paired == Some(best[i][j]) {
            ops.push(Op::Match(i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if i > 0 && (j == 0 || best[i - 1][j] >= best[i][j - 1]) {
            ops.push(Op::Column(i - 1));
            i -= 1;
        } else {
            ops.push(Op::Token(j - 1));
            j -= 1;
        }
    }
    ops.reverse();
    ops
}

/// Fold `example` into the columns: a matched token joins its column, and
/// in each stretch between matches the columns and tokens left over pair
/// up in order, a pair being one position whose kind differs between
/// examples; a column left over stays as a position this example lacks,
/// and a token left over opens a position the earlier examples lack.
fn fold(columns: Vec<Column>, example: usize, seq: &[Tok], examples: usize) -> Vec<Column> {
    let ops = align(&columns, seq);
    let mut taken: Vec<Option<Column>> = columns.into_iter().map(Some).collect();
    let mut out: Vec<Column> = Vec::with_capacity(taken.len() + seq.len());
    let mut gap_columns: Vec<Column> = Vec::new();
    let mut gap_tokens: Vec<usize> = Vec::new();
    let flush = |gap_columns: &mut Vec<Column>, gap_tokens: &mut Vec<usize>, out: &mut Vec<Column>| {
        let pairs = gap_columns.len().min(gap_tokens.len());
        for (p, mut column) in gap_columns.drain(..).enumerate() {
            if p < pairs {
                column.absorb(example, &seq[gap_tokens[p]]);
            }
            out.push(column);
        }
        for &j in gap_tokens.iter().skip(pairs) {
            out.push(Column::new(examples, example, &seq[j]));
        }
        gap_tokens.clear();
    };
    for op in ops {
        match op {
            Op::Match(i, j) => {
                flush(&mut gap_columns, &mut gap_tokens, &mut out);
                let mut column = taken[i].take().expect("each column is walked once");
                column.absorb(example, &seq[j]);
                out.push(column);
            }
            Op::Column(i) => gap_columns.push(taken[i].take().expect("each column is walked once")),
            Op::Token(j) => gap_tokens.push(j),
        }
    }
    flush(&mut gap_columns, &mut gap_tokens, &mut out);
    out
}

/// The slots the aligned columns spell: a literal, which is a text every
/// example has at the position, is a slot of its own; so is a class; and
/// consecutive positions of one kind set and no literal merge into one slot
/// whose bounds are the fewest and the most tokens an example has across
/// them, so a position some examples lack is a repeat from zero.
fn slots_of(columns: &[Column]) -> Vec<Slot> {
    let mut slots: Vec<Slot> = Vec::new();
    // The per-example token counts of the run the last slot merges, while
    // one is open.
    let mut run: Option<Vec<usize>> = None;
    for column in columns {
        let here: Vec<usize> = column.present().map(usize::from).collect();
        let all = column.present().all(|p| p);
        if let (true, Some(text)) = (all, &column.text) {
            slots.push(Slot {
                kinds: column.kinds.clone(),
                text: Some(text.clone()),
                narrowing: None,
                min: 1,
                max: 1,
            });
            run = None;
            continue;
        }
        let merges = match (&run, slots.last()) {
            (Some(_), Some(last)) => last.text.is_none() && last.kinds == column.kinds,
            _ => false,
        };
        if merges {
            let counts = run.as_mut().expect("a run is open when a slot merges");
            for (c, h) in counts.iter_mut().zip(&here) {
                *c += h;
            }
            let last = slots.last_mut().expect("a slot is open when a slot merges");
            last.min = counts.iter().copied().min().unwrap_or(0);
            last.max = counts.iter().copied().max().unwrap_or(0);
            last.narrowing = None;
        } else {
            // A narrowing describes one position's values, so it is read
            // only where the slot is that one position. A merged run holds
            // a varying number of tokens per example and no example's value
            // is at a known place in it.
            slots.push(Slot {
                kinds: column.kinds.clone(),
                text: None,
                narrowing: narrowing_of(column),
                min: here.iter().copied().min().unwrap_or(0),
                max: here.iter().copied().max().unwrap_or(0),
            });
            run = Some(here);
        }
    }
    slots
}

/// The most specific pattern every example matches.
///
/// With no counter-examples no position takes a value range, so this is
/// [`infer_against`] with nothing to tell the examples from.
///
/// # Errors
///
/// Fewer than two examples, an example with no token, or a pattern that
/// fails its own verification, which names the example it missed.
pub fn infer(examples: &[&[u8]]) -> Result<Inferred, InferError> {
    infer_against(examples, &[])
}

/// The most specific pattern every example matches and no counter-example
/// does.
///
/// A value range is printed only where a counter-example rules it in. With
/// nothing to tell the examples from there is no evidence a range means
/// anything - two numbers are a sample of two - so a position reports its
/// bare kind however many examples agree. Where counter-examples are given,
/// the ranges that exclude them are added one at a time, most excluded
/// first, so the pattern carries the ranges it needed and no others.
///
/// An orbit rung is not gated this way. A rung still covers exactly the
/// texts the examples showed, under a folding; a range covers values none of
/// them did, and that extrapolation is what the evidence is needed for.
///
/// # Errors
///
/// As [`infer`], and a counter-example the pattern still matches once every
/// range the examples support has been tried.
pub fn infer_against(examples: &[&[u8]], counters: &[&[u8]]) -> Result<Inferred, InferError> {
    if examples.len() < 2 {
        return Err(InferError::TooFew(examples.len()));
    }
    let mut sequences: Vec<Vec<Tok>> = Vec::with_capacity(examples.len());
    for (i, example) in examples.iter().enumerate() {
        let seq: Vec<Tok> = crate::lexer::lex(example)
            .iter()
            .filter(|t| t.is_significant())
            .map(|t| Tok {
                kind: t.kind,
                text: String::from_utf8_lossy(&example[t.span()]).into_owned(),
                start: t.start(),
                end: t.end(),
            })
            .collect();
        if seq.is_empty() {
            return Err(InferError::Empty(i));
        }
        sequences.push(seq);
    }
    let n = examples.len();
    let mut columns: Vec<Column> = sequences[0].iter().map(|t| Column::new(n, 0, t)).collect();
    for (e, seq) in sequences.iter().enumerate().skip(1) {
        columns = fold(columns, e, seq, n);
    }
    let mut slots = slots_of(&columns);
    // Every range starts switched off. Each is a candidate the examples
    // support, and it is used only where a counter-example needs it.
    let candidates: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s.narrowing, Some(Narrowing::Range(_))))
        .map(|(i, _)| i)
        .collect();
    let mut ranges: Vec<Option<Narrowing>> = candidates.iter().map(|&i| slots[i].narrowing.take()).collect();
    let spell = |slots: &[Slot]| slots.iter().map(Slot::spelled).collect::<Vec<_>>().join(" ");
    let missed = |slots: &[Slot]| -> Result<Vec<usize>, InferError> {
        let pattern = spell(slots);
        let parsed = match crate::parser::parse(&pattern) {
            Ok(p) => p,
            Err(e) => return Err(InferError::Unparsed { pattern, msg: e.msg }),
        };
        Ok(counters
            .iter()
            .enumerate()
            .filter(|(_, c)| !crate::engine::scan(&parsed, c).is_empty())
            .map(|(i, _)| i)
            .collect())
    };
    let mut hit = missed(&slots)?;
    while !hit.is_empty() {
        // The range excluding the most of the counter-examples still matched
        // is taken first, so a pattern never carries a range that narrowed
        // nothing.
        let mut best: Option<(usize, usize)> = None;
        for (c, slot) in candidates.iter().enumerate() {
            if ranges[c].is_none() {
                continue;
            }
            slots[*slot].narrowing = ranges[c].clone();
            let left = missed(&slots)?.len();
            slots[*slot].narrowing = None;
            if best.is_none_or(|(_, fewest)| left < fewest) {
                best = Some((c, left));
            }
        }
        let Some((c, left)) = best else { break };
        if left >= hit.len() {
            break;
        }
        slots[candidates[c]].narrowing = ranges[c].take();
        hit = missed(&slots)?;
    }
    let pattern = spell(&slots);
    if let Some(&counter) = hit.first() {
        return Err(InferError::Matched { counter, pattern });
    }
    let parsed = match crate::parser::parse(&pattern) {
        Ok(p) => p,
        Err(e) => return Err(InferError::Unparsed { pattern, msg: e.msg }),
    };
    for (i, (example, seq)) in examples.iter().zip(&sequences).enumerate() {
        let whole = seq[0].start..seq[seq.len() - 1].end;
        if !crate::engine::scan(&parsed, example).iter().any(|s| s.range() == whole) {
            return Err(InferError::Unverified { example: i, pattern });
        }
    }
    Ok(Inferred { slots, pattern })
}
