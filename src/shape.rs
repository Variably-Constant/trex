//! The shape axis - TREX's structural substrate, the dual of `spectral`.
//!
//! The spectral axis (`crate::spectral`) reads the temporal character of the
//! byte stream; the shape axis reads its structural form. Two spans have the
//! same shape when they share a silhouette - the same sequence of token-classes
//! and word-shapes - regardless of content. `foo(a, b)` and `bar(x, y)` are the
//! same shape `W ( W , W )`; `1,22,3` and `444,5,66` are the same shape
//! `N , N , N`. A silhouette is a grammar production, so a shape token is a
//! grammar-free grammar.
//!
//! The load-bearing signal is the **shape period**: the spectral period
//! (`spectral::dominant_period`) autocorrelates bytes and so finds only
//! fixed-width structure; a ragged table (varying field widths) defeats it
//! because the delimiter byte-offsets shift every row. Shape autocorrelates
//! silhouettes, where width variation has already collapsed (a field is one
//! `Number` token whatever its width), so the row period survives. Structural
//! periodicity is invisible to every other axis.
//!
//! Token-grain, byte-span-keyed: every frame carries the token's byte span, so
//! the byte, spectral and supertoken layers query this field by byte offset
//! exactly as they query the spectral field. Documented in
//! `wiki/content/docs/reference/axes/shape.md`.

use crate::token::{Token, TokenKind};

/// Knobs for the shape reader. `Default` suits general token input.
#[derive(Clone, Copy, Debug)]
pub struct ShapeConfig {
    /// Largest token-lag the period search considers.
    pub max_lag: usize,
    /// Trailing token window the period autocorrelation runs over.
    pub period_window: usize,
    /// Recompute the period every `period_hop` tokens (held between), so total
    /// periodicity work stays under a fixed op budget on large inputs.
    pub period_hop: usize,
    /// Silhouette n-gram order for novelty / change-point.
    pub novelty_k: usize,
    /// Sliding window (in n-grams) the novelty count map spans.
    pub novelty_window: usize,
    /// A shape-period run is a template when its strength reaches this.
    pub template_strength: f32,
    /// Minimum tokens between two change-points.
    pub cp_min_gap: usize,
}

impl Default for ShapeConfig {
    fn default() -> Self {
        Self {
            max_lag: 64,
            period_window: 256,
            period_hop: 8,
            novelty_k: 3,
            novelty_window: 4096,
            template_strength: 0.6,
            cp_min_gap: 4,
        }
    }
}

/// How far the token counts of a table's lines may spread, as a share of
/// their mean, where its template repeats every one or two tokens.
const ROW_SPREAD: f32 = 0.5;

/// One token's silhouette as a comparable `u32`: the `TokenKind` code in the
/// high bits, plus - for `Word` - the [`crate::tokutil::shape`] class, and for
/// `Punct` - the [`glyph`]. Two tokens with the same code are the same shape.
#[must_use]
pub fn shape_class(kind: TokenKind, tok_bytes: &[u8]) -> u32 {
    let base = kind.code() << 16;
    match kind {
        TokenKind::Word => {
            let s = crate::tokutil::shape(&String::from_utf8_lossy(tok_bytes));
            base | word_shape_code(s)
        }
        TokenKind::Punct => base | glyph(tok_bytes),
        _ => base,
    }
}

/// A punctuation token's glyph: the low sixteen bits of the code point of its
/// first character, so `、` and `。` are two glyphs as `,` and `.` are, or its
/// first byte where that byte begins no well-formed character.
fn glyph(tok: &[u8]) -> u32 {
    let Some(chunk) = tok.utf8_chunks().next() else { return 0 };
    match chunk.valid().chars().next() {
        Some(c) => u32::from(c) & 0xFFFF,
        None => chunk.invalid().first().map_or(0, |&b| u32::from(b)),
    }
}

/// Map a [`crate::tokutil::shape`] class string to a small code (0..6).
fn word_shape_code(s: &str) -> u32 {
    match s {
        "Pascal" => 1,
        "snake" => 2,
        "camel" => 3,
        "SCREAM" => 4,
        "short" => 5,
        _ => 0, // "word"
    }
}

/// One token's structural reading.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShapeFrame {
    /// The token's [`shape_class`] code.
    pub class: u32,
    /// Dominant shape-period of the neighborhood, in tokens (`0` = none).
    pub period: u16,
    /// Normalized silhouette-autocorrelation peak `[0,1]`.
    pub period_strength: f32,
    /// Shape n-gram surprise `[0,1]` (`1` = first sighting of this template).
    pub novelty: f32,
}

/// The structural side table, keyed by byte offset (like `SpectralField`).
#[derive(Clone, Debug, Default)]
pub struct ShapeField {
    /// Token count.
    pub n_tokens: usize,
    /// Byte span per token - the byte-offset key.
    pub spans: Vec<(usize, usize)>,
    /// One frame per token.
    pub frames: Vec<ShapeFrame>,
    /// Shape-change-point byte offsets (silhouette-break cuts), sorted.
    pub boundaries: Vec<usize>,
    /// The template-strength threshold this field was built with.
    template_strength: f32,
}

impl ShapeField {
    /// The token index covering `byte` (the last token whose span starts at or
    /// before `byte`), or `None` if the field is empty.
    fn token_at(&self, byte: usize) -> Option<usize> {
        if self.spans.is_empty() {
            return None;
        }
        let i = self.spans.partition_point(|&(s, _)| s <= byte);
        Some(i.saturating_sub(1))
    }

    /// The shape-class of the token covering `byte` (`0` if empty).
    #[must_use]
    pub fn class_at(&self, byte: usize) -> u32 {
        self.token_at(byte)
            .and_then(|i| self.frames.get(i))
            .map_or(0, |f| f.class)
    }

    /// The dominant shape-period and its strength at `byte`.
    #[must_use]
    pub fn period_at(&self, byte: usize) -> (u16, f32) {
        self.token_at(byte)
            .and_then(|i| self.frames.get(i))
            .map_or((0, 0.0), |f| (f.period, f.period_strength))
    }

    /// Is `byte` inside a strong shape-period (template) run?
    #[must_use]
    pub fn in_template(&self, byte: usize) -> bool {
        self.period_at(byte).1 >= self.template_strength
    }

    /// The periodic / template regions as byte spans plus their period: maximal
    /// runs of tokens whose period strength clears the template threshold, each
    /// ending at the last token that repeats the kind one period before it. The
    /// structural map a tabular / template consumer reads.
    #[must_use]
    pub fn shape_regions(&self) -> Vec<(usize, usize, u16)> {
        self.template_runs()
            .into_iter()
            .map(|(first, last, period)| (self.spans[first].0, self.spans[last].1, period))
            .collect()
    }

    /// The runs of tokens whose period strength clears the template threshold,
    /// as first and last token index with the period the run opened at.
    ///
    /// The reader looks back over a window, so the tokens just past a
    /// repeating block inherit its period at full strength. Each run's end is
    /// therefore trimmed back to the last token whose kind equals the kind one
    /// period earlier, the mirror of how [`Self::template_spans`] carries a
    /// run's start back over the periods that already repeat.
    fn template_runs(&self) -> Vec<(usize, usize, u16)> {
        let kind = |i: usize| self.frames[i].class >> 16;
        let trim = |(first, last, period): (usize, usize, u16)| {
            let lag = usize::from(period);
            let mut last = last;
            while last > first && !(last >= lag && kind(last) == kind(last - lag)) {
                last -= 1;
            }
            (first, last, period)
        };
        let mut out: Vec<(usize, usize, u16)> = Vec::new();
        let mut run: Option<(usize, usize, u16)> = None;
        for (i, f) in self.frames.iter().enumerate() {
            if f.period_strength >= self.template_strength && f.period > 0 {
                match run.as_mut() {
                    Some(r) => r.1 = i,
                    None => run = Some((i, i, f.period)),
                }
            } else if let Some(r) = run.take() {
                out.push(trim(r));
            }
        }
        if let Some(r) = run.take() {
            out.push(trim(r));
        }
        out
    }

    /// [`Self::shape_regions`] with each run reaching back over the periods
    /// before it that already repeat: a whole period is taken while every
    /// token's kind equals the kind one period later, or one period earlier
    /// where the later one is past the input's end, and the period holds a
    /// kind other than a word. The kind rather than the whole class, so two
    /// words of different shapes in one column, `alpha` and `beta`, carry the
    /// run back; a kind other than a word, since every word is one kind and
    /// text of words alone repeats at any period. The reader finds a period
    /// only once its trailing window holds a few repeats, so a run begins rows
    /// after the repetition does, and at the input's end it may be the last
    /// row alone. Byte spans with their period, ascending by start.
    #[must_use]
    pub fn template_spans(&self) -> Vec<(usize, usize, u16)> {
        self.extended_runs(|_, _, _| true)
    }

    /// [`Self::template_spans`] of the runs a table is made of. Their tokens
    /// hold a kind other than a word, since words alone repeat at any period
    /// and a sentence alternating short and long words is no table. And a run
    /// that repeats every one or two tokens has regular rows, lines holding
    /// about as many tokens as each other: a period that short holds no
    /// columns - one kind over and over, or a value and a separator in turn -
    /// so the rows have to be lines, as a list of numbers is and a paragraph
    /// of clauses and commas is not. `input` is the text the field was read
    /// from, which holds the line breaks.
    #[must_use]
    pub fn table_spans(&self, input: &[u8]) -> Vec<(usize, usize, u16)> {
        let word = TokenKind::Word.code();
        self.extended_runs(|first, last, period| {
            (first..=last).any(|i| self.frames[i].class >> 16 != word)
                && (period > 2 || self.rows_are_regular(input, first, last))
        })
    }

    /// Whether the lines holding tokens `first..=last` hold about as many
    /// tokens as each other: two lines or more, the standard deviation of the
    /// tokens a line holds at most [`ROW_SPREAD`] of their mean. A line ends
    /// at a `\n` or `\r` between two tokens, and the first and last lines
    /// count the tokens the run reaches in them.
    fn rows_are_regular(&self, input: &[u8], first: usize, last: usize) -> bool {
        let mut rows: Vec<f32> = Vec::new();
        let mut tokens = 1u32;
        for j in first + 1..=last {
            if input[self.spans[j - 1].1..self.spans[j].0].iter().any(|&b| b == b'\n' || b == b'\r') {
                rows.push(tokens as f32);
                tokens = 1;
            } else {
                tokens += 1;
            }
        }
        rows.push(tokens as f32);
        if rows.len() < 2 {
            return false;
        }
        let n = rows.len() as f32;
        let mean = rows.iter().sum::<f32>() / n;
        let variance = rows.iter().map(|r| (r - mean).powi(2)).sum::<f32>() / n;
        variance.sqrt() <= ROW_SPREAD * mean
    }

    /// The template runs `keep` takes by first and last token and period, each
    /// reaching back over the periods before it that already repeat.
    fn extended_runs(&self, keep: impl Fn(usize, usize, u16) -> bool) -> Vec<(usize, usize, u16)> {
        let n = self.frames.len();
        let kind = |i: usize| self.frames[i].class >> 16;
        let repeats = |j: usize, lag: usize| {
            if j + lag < n {
                kind(j) == kind(j + lag)
            } else {
                j >= lag && kind(j) == kind(j - lag)
            }
        };
        let word = TokenKind::Word.code();
        let mut out: Vec<(usize, usize, u16)> = self
            .template_runs()
            .into_iter()
            .filter(|&(first, last, period)| keep(first, last, period))
            .map(|(first, last, period)| {
                let lag = usize::from(period);
                let mut first = first;
                while first > 0 {
                    let j = first - 1;
                    if !repeats(j, lag) || (j..(j + lag).min(n)).all(|i| kind(i) == word) {
                        break;
                    }
                    first = j;
                }
                (self.spans[first].0, self.spans[last].1, period)
            })
            .collect();
        out.sort_unstable_by_key(|&(s, _, _)| s);
        out
    }
}

/// Analyze a token stream with the default configuration.
#[must_use]
pub fn analyze(tokens: &[Token], bytes: &[u8]) -> ShapeField {
    analyze_with(tokens, bytes, &ShapeConfig::default())
}

/// Tokenize `bytes` (the significant-token lexer) and analyze- the convenience
/// path for consumers that hold only bytes.
#[must_use]
pub fn analyze_bytes(bytes: &[u8]) -> ShapeField {
    let toks = crate::tokutil::lex_sig(bytes);
    analyze(&toks, bytes)
}

/// The shape-class for a token measured over a chosen orbit quotient: the
/// silhouette is the token's [`crate::orbit::canonical`] form under `group`
/// (folded with the `TokenKind`), so two tokens that are the same up to the
/// group share a class. `Identity` -> the literal token (exact-repeat
/// structure); `Case` -> case-folded (`The` = `the`); `Notation` ->
/// notation-folded; `Shape` -> the C/V/D phonotactic pattern. This is the shape
/// axis reading the structure of an orbit the caller picks, instead of the
/// built-in word-shape silhouette of [`shape_class`].
#[must_use]
pub fn shape_class_over(kind: TokenKind, tok_bytes: &[u8], group: crate::orbit::OrbitGroup) -> u32 {
    let base = kind.code() << 16;
    let canon = crate::orbit::canonical(tok_bytes, group);
    // 16-bit FNV-1a of the canonical form: same orbit -> same low bits.
    let mut h: u32 = 2166136261;
    for &b in canon.as_bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(16777619);
    }
    base | ((h ^ (h >> 16)) & 0xFFFF)
}

/// Build the field (spans, novelty, periodicity, change-points) from a
/// precomputed silhouette-class per token - the shared core of [`analyze_with`]
/// (built-in word-shape silhouette) and [`analyze_over_with`] (orbit-quotient
/// silhouette).
fn build_field(tokens: &[Token], classes: &[u32], cfg: &ShapeConfig) -> ShapeField {
    let n = tokens.len();
    let mut field = ShapeField {
        n_tokens: n,
        spans: Vec::with_capacity(n),
        frames: Vec::with_capacity(n),
        boundaries: Vec::new(),
        template_strength: cfg.template_strength,
    };
    if n == 0 {
        return field;
    }
    for t in tokens {
        field.spans.push((t.start(), t.end()));
    }

    // 5.3 novelty + 5.4 change-point: rolling silhouette n-gram surprise.
    let k = cfg.novelty_k.max(1);
    let mut counts: std::collections::HashMap<u64, u32> =
        std::collections::HashMap::with_capacity(cfg.novelty_window.min(n) + 1);
    // The window drops its oldest n-gram every token; a deque pops it in O(1).
    let mut ring: std::collections::VecDeque<u64> = std::collections::VecDeque::with_capacity(cfg.novelty_window + 1);
    let mut last_cut: isize = -(cfg.cp_min_gap as isize);

    // 5.2 periodicity: recomputed every `period_hop` tokens, held between.
    let mut cur_period: u16 = 0;
    let mut cur_strength: f32 = 0.0;

    let mut frames: Vec<ShapeFrame> = Vec::with_capacity(n);
    for i in 0..n {
        // novelty of the k-gram ending at i.
        let novelty = if i + 1 >= k {
            let mut h: u64 = 1469598103934665603;
            for &c in &classes[i + 1 - k..=i] {
                h = (h ^ u64::from(c)).wrapping_mul(1099511628211);
            }
            let prev = *counts.get(&h).unwrap_or(&0);
            let entry = counts.entry(h).or_insert(0);
            *entry += 1;
            ring.push_back(h);
            if ring.len() > cfg.novelty_window
                && let Some(old) = ring.pop_front()
                && let Some(c) = counts.get_mut(&old)
            {
                *c = c.saturating_sub(1);
            }
            1.0 / (1.0 + prev as f32)
        } else {
            1.0
        };

        // change-point: a novelty spike past 0.5 after the min gap is a break.
        if novelty > 0.5 && (i as isize - last_cut) >= cfg.cp_min_gap as isize && i > 0 {
            field.boundaries.push(tokens[i].start());
            last_cut = i as isize;
        }

        // periodicity over a trailing silhouette window, recomputed on the hop.
        if i % cfg.period_hop == 0 || i + 1 == n {
            let lo = i.saturating_sub(cfg.period_window);
            let (p, s) = dominant_shape_period(&classes[lo..=i], cfg.max_lag);
            cur_period = p;
            cur_strength = s;
        }

        frames.push(ShapeFrame {
            class: classes[i],
            period: cur_period,
            period_strength: cur_strength,
            novelty,
        });
    }
    field.frames = frames;
    field
}

/// The full one-pass reader over the built-in word-shape silhouette.
#[must_use]
pub fn analyze_with(tokens: &[Token], bytes: &[u8], cfg: &ShapeConfig) -> ShapeField {
    // 5.1 silhouette: one shape-class per token.
    let classes: Vec<u32> = tokens
        .iter()
        .map(|t| shape_class(t.kind, &bytes[t.span()]))
        .collect();
    build_field(tokens, &classes, cfg)
}

/// The one-pass reader over a chosen orbit quotient, the composition axis. The
/// shape axis measures period / novelty / change-points of the silhouette
/// `group` produces: `Identity` recovers exact-token structure, `Case` folds
/// case before measuring (the structural period of the case-folded stream),
/// `Notation` / `Shape` fold notation / phonotactic shape. Orbit is the
/// pre-transform; shape is the measurement that composes over it.
#[must_use]
pub fn analyze_over_with(
    tokens: &[Token],
    bytes: &[u8],
    group: crate::orbit::OrbitGroup,
    cfg: &ShapeConfig,
) -> ShapeField {
    let classes: Vec<u32> = tokens
        .iter()
        .map(|t| shape_class_over(t.kind, &bytes[t.span()], group))
        .collect();
    build_field(tokens, &classes, cfg)
}

/// [`analyze_over_with`] with the default configuration.
#[must_use]
pub fn analyze_over(tokens: &[Token], bytes: &[u8], group: crate::orbit::OrbitGroup) -> ShapeField {
    analyze_over_with(tokens, bytes, group, &ShapeConfig::default())
}

/// Tokenize `bytes` and analyzeover a chosen orbit quotient - the convenience
/// path for consumers that hold only bytes.
#[must_use]
pub fn analyze_bytes_over(bytes: &[u8], group: crate::orbit::OrbitGroup) -> ShapeField {
    let toks = crate::tokutil::lex_sig(bytes);
    analyze_over(&toks, bytes, group)
}

/// Categorical autocorrelation over shape-classes: for each token-lag the
/// fraction of positions whose class equals the class `lag` back. The dominant
/// period is the arg-max with prominence (>= 0.5 match); equality-based because
/// shape-classes are categorical, not numeric - that is what lets a ragged
/// table (same silhouette, different widths) still align.
fn dominant_shape_period(win: &[u32], max_lag: usize) -> (u16, f32) {
    let n = win.len();
    if n < 4 {
        return (0, 0.0);
    }
    let hi = max_lag.min(n / 2);
    let mut best_lag = 0usize;
    let mut best = 0.0f32;
    for lag in 1..=hi {
        // Branchless equality count over the two overlapping slices, accumulated
        // in u32, the lane width of the data: an indexless loop with a u32
        // accumulator vectorizes (eight lanes per AVX2 step) where a usize one
        // widens every element and stays scalar. The window never nears
        // u32::MAX elements.
        let mut matches = 0u32;
        for (a, b) in win[lag..].iter().zip(&win[..n - lag]) {
            matches += u32::from(a == b);
        }
        let matches = matches as usize;
        let frac = matches as f32 / (n - lag) as f32;
        if frac > best {
            best = frac;
            best_lag = lag;
        }
    }
    if best < 0.5 {
        (0, best)
    } else {
        (best_lag as u16, best)
    }
}

/// A region's classification, fusing the spectral texture (byte-grain) with the
/// tokens and the shape period (token-grain): a region of encoded tokens is a
/// `Blob`, one carrying a strong shape period is a `Table` (a tabular / record
/// template) regardless of byte texture, and the rest take their spectral
/// texture. The two-axis composition - shape x spectral - that names a data
/// table with neither a delimiter nor a grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// A strong shape-period template (the `u16` is the period in tokens).
    Table(u16),
    /// Encoded or opaque: base64, hash and hex tokens or high-entropy runs over
    /// most of the region, or a spectral high-entropy texture.
    Blob,
    /// Spectral prose texture.
    Prose,
    /// Spectral numeric texture.
    Numeric,
    /// Spectral code texture, no strong period.
    Code,
    /// Spectral mixed texture.
    Mixed,
}

impl RegionKind {
    /// The kind's name, without the period a table carries.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            RegionKind::Table(_) => "table",
            RegionKind::Blob => "blob",
            RegionKind::Prose => "prose",
            RegionKind::Numeric => "numeric",
            RegionKind::Code => "code",
            RegionKind::Mixed => "mixed",
        }
    }

    /// Every kind's name, for a caller listing what it accepts.
    pub const NAMES: [&'static str; 6] = ["table", "blob", "prose", "numeric", "code", "mixed"];

    /// Whether this kind is the one `name` calls for.
    ///
    /// A table is named by `table` whatever its period, because the period is
    /// a property of the table found and not of the kind asked for: a caller
    /// keeping the tables cannot know their periods in advance, and one that
    /// wants a particular period reads it off the region.
    #[must_use]
    pub fn named(self, name: &str) -> bool {
        self.label() == name
    }
}

/// Whether an input whose dominant kind is `kind` is kept by the texture
/// filters `asked`, each a kind's name and whether it keeps (`true`) or
/// drops (`false`) an input of that kind.
///
/// A kind named to drop drops. Where only drops are given every other input
/// is kept, since the caller asked to remove something rather than to select
/// something; where any keep is given the keeps are the whole of what passes.
#[must_use]
pub fn keeps_texture(asked: &[(String, bool)], kind: Option<RegionKind>) -> bool {
    let named = |name: &str| kind.is_some_and(|k| k.named(name));
    if asked.iter().any(|(name, keeps)| !keeps && named(name)) {
        return false;
    }
    match asked.iter().any(|(_, keeps)| *keeps) {
        true => asked.iter().any(|(name, keeps)| *keeps && named(name)),
        false => true,
    }
}

/// Classify `input` into regions by fusing the spectral region texture with the
/// lexer's tokens and the shape period. A spectral region more than half of
/// whose bytes the lexer reads as encoded - base64, hash and hex tokens and
/// the runs its blob gate collapses - is a `Blob`, wrapped base64 and a list
/// of digests included. Otherwise one more than half of whose bytes are in
/// table runs ([`ShapeField::table_spans`]: shape-period templates holding a
/// kind other than a word, each reaching back over the rows its period
/// already repeated, with lines of about one length where the period is one
/// or two tokens) is a `Table` with the period of the run covering most of
/// it. The rest keep their texture. The cross-cutting consumer of the shape
/// axis - a tabular block is named structurally, where the byte-period alone
/// reads only "data". `shape` calls `spectral` here (token grain over byte
/// grain), never the reverse.
#[must_use]
pub fn classified_regions(input: &[u8]) -> Vec<(usize, usize, RegionKind)> {
    classified_regions_with(input, &crate::spectral::SpectralConfig::default())
}

/// [`classified_regions`] with the spectral regions split at the
/// change-points `spectral` places.
#[must_use]
pub fn classified_regions_with(input: &[u8], spectral: &crate::spectral::SpectralConfig) -> Vec<(usize, usize, RegionKind)> {
    let toks = crate::tokutil::lex_sig(input);
    let templates = analyze(&toks, input).table_spans(input);
    let encoded = encoded_spans(input, &toks);
    crate::spectral::code_regions_with(input, spectral)
        .into_iter()
        .map(|(s, e, tex)| {
            // The templates' union inside the region, and the one covering most.
            let mut covered = 0usize;
            let mut reach = s;
            let mut widest: Option<(usize, u16)> = None;
            for &(ts, te, p) in &templates {
                let (lo, hi) = (ts.max(s), te.min(e));
                if lo >= hi {
                    continue;
                }
                covered += hi.saturating_sub(lo.max(reach));
                reach = reach.max(hi);
                if widest.is_none_or(|(w, _)| hi - lo > w) {
                    widest = Some((hi - lo, p));
                }
            }
            let period = widest.filter(|_| 2 * covered > e - s).map(|(_, p)| p);
            let kind = match period {
                _ if 2 * covered_by(&encoded, s, e) > e - s => RegionKind::Blob,
                Some(p) => RegionKind::Table(p),
                None => match tex {
                    crate::spectral::CodeTexture::Blob => RegionKind::Blob,
                    crate::spectral::CodeTexture::Prose => RegionKind::Prose,
                    crate::spectral::CodeTexture::Numeric => RegionKind::Numeric,
                    crate::spectral::CodeTexture::Mixed => RegionKind::Mixed,
                    crate::spectral::CodeTexture::Code => RegionKind::Code,
                },
            };
            (s, e, kind)
        })
        .collect()
}

/// The bytes the lexer reads as encoded or opaque: its base64, hash and hex
/// tokens and the high-entropy runs its blob gate collapses, merged,
/// ascending. A region most of whose bytes these cover is a blob whatever
/// its byte texture, so base64 of English text, whose letters read as prose,
/// still reads as encoded.
fn encoded_spans(input: &[u8], toks: &[Token]) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = toks
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Base64 | TokenKind::HashDigest | TokenKind::Hex))
        .map(|t| (t.start(), t.end()))
        .chain(crate::lexer::blob_runs(input))
        .collect();
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (s, e) in spans {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// How many bytes of `[s, e)` the ascending, disjoint `spans` cover.
fn covered_by(spans: &[(usize, usize)], s: usize, e: usize) -> usize {
    let first = spans.partition_point(|&(_, end)| end <= s);
    spans[first..].iter().take_while(|&&(start, _)| start < e).map(|&(start, end)| end.min(e) - start.max(s)).sum()
}

/// The kind covering the most bytes of `input`, or `None` for an input with
/// no region at all.
///
/// Bytes rather than region count, because a file is named by what most of it
/// is: a source file holding one long base64 line and forty short code
/// regions is code by count and could be a blob by bytes, and it is the bytes
/// a reader means when they call a file one thing. A table reports the period
/// of the widest table in it, since the period belongs to the region rather
/// than to the file and one had to be chosen.
#[must_use]
pub fn dominant_kind(input: &[u8]) -> Option<RegionKind> {
    let regions = classified_regions(input);
    // Held by name rather than by kind, so every table counts toward one
    // total whatever period each carries.
    let mut totals: Vec<(&'static str, usize, RegionKind, usize)> = Vec::new();
    for (s, e, kind) in regions {
        let span = e.saturating_sub(s);
        match totals.iter_mut().find(|(name, _, _, _)| *name == kind.label()) {
            Some((_, bytes, widest, widest_span)) => {
                *bytes += span;
                if span > *widest_span {
                    *widest = kind;
                    *widest_span = span;
                }
            }
            None => totals.push((kind.label(), span, kind, span)),
        }
    }
    totals.into_iter().max_by_key(|&(_, bytes, _, _)| bytes).map(|(_, _, kind, _)| kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(s: &str) -> ShapeField {
        analyze_bytes(s.as_bytes())
    }

    #[test]
    fn ragged_csv_has_strong_shape_period() {
        // Field widths vary, so the byte period finds nothing; the shape period
        // (N , N , N per row) is strong.
        let f = field("1,22,3\n444,5,66\n7,888,9\n12,3,456\n");
        let strong = f.frames.iter().any(|fr| fr.period > 0 && fr.period_strength >= 0.6);
        assert!(strong, "ragged CSV should show a strong shape period");
        assert!(!f.shape_regions().is_empty(), "should report a template region");
    }

    #[test]
    fn prose_has_no_shape_period() {
        let f = field("the quick brown fox jumps over the lazy dog and then rests");
        let any_template = f.frames.iter().any(|fr| fr.period_strength >= 0.8 && fr.period > 1);
        assert!(!any_template, "free prose should not read as a strong template");
    }

    #[test]
    fn repeated_idiom_is_low_novelty() {
        // The repeated `self.x = x;` idiom recurs - later sightings score low.
        let f = field("self.a = a; self.b = b; self.c = c; self.d = d;");
        let tail: f32 = f.frames.iter().rev().take(4).map(|fr| fr.novelty).sum::<f32>() / 4.0;
        assert!(tail < 0.6, "a repeated template should have low tail novelty, got {tail}");
    }

    #[test]
    fn a_texture_filter_keeps_by_name_and_drops_first() {
        let asked = |list: &[(&str, bool)]| list.iter().map(|(n, k)| ((*n).to_string(), *k)).collect::<Vec<_>>();
        let table = Some(RegionKind::Table(7));
        assert!(keeps_texture(&[], None));
        assert!(keeps_texture(&asked(&[("table", true)]), table));
        assert!(!keeps_texture(&asked(&[("prose", true)]), table));
        assert!(!keeps_texture(&asked(&[("prose", true)]), None));
        assert!(!keeps_texture(&asked(&[("table", false)]), table));
        assert!(keeps_texture(&asked(&[("blob", false)]), table));
        assert!(keeps_texture(&asked(&[("blob", false)]), None));
        assert!(!keeps_texture(&asked(&[("table", true), ("table", false)]), table));
    }

    #[test]
    fn empty_is_safe() {
        let f = field("");
        assert_eq!(f.n_tokens, 0);
        assert!(f.frames.is_empty());
        assert!(f.shape_regions().is_empty());
        assert_eq!(f.class_at(0), 0);
        assert!(!f.in_template(0));
    }

    #[test]
    fn class_at_maps_byte_to_silhouette() {
        let f = field("foo(a, b)");
        // The first token is the Word `foo`; its class is non-zero and stable.
        assert_ne!(f.class_at(0), 0);
        // `foo` and a same-shaped word elsewhere share a class.
        let g = field("bar(x, y)");
        assert_eq!(f.class_at(0), g.class_at(0), "same silhouette -> same class");
    }

    #[test]
    fn classified_regions_names_a_table() {
        // The fused classifier (shape x spectral) names a ragged CSV `Table`,
        // where the byte-period alone would read only "data".
        let csv = "name,age,score\nalice,30,95\nbob,25,88\ncarol,41,73\ndan,38,91\n";
        let regions = classified_regions(csv.as_bytes());
        assert!(
            regions.iter().any(|&(_, _, k)| matches!(k, RegionKind::Table(_))),
            "ragged CSV should classify as a Table region, got {regions:?}"
        );
    }

    #[test]
    fn a_short_template_run_in_prose_is_no_table() {
        let prose = "# Reading a directory\n\nThe walk reports what it found rather than what it was asked for. A filter that\nsilently drops a file reads exactly the same as a directory that never held one, and\nthe reader cannot tell the two apart afterwards.\n";
        assert!(
            !field(prose).shape_regions().is_empty(),
            "the prose holds a template run, which is what the rule must not take for a table"
        );
        assert!(
            !classified_regions(prose.as_bytes()).iter().any(|&(_, _, k)| matches!(k, RegionKind::Table(_))),
            "prose with a short template run reads by its texture"
        );
        let words = "queue drained\nnothing to report\n";
        assert!(!classified_regions(words.as_bytes()).iter().any(|&(_, _, k)| matches!(k, RegionKind::Table(_))));
    }

    #[test]
    fn a_table_whose_period_is_found_at_its_last_row_is_a_table() {
        for table in ["alpha 10\nbeta 20\ngamma 300\ndelta 4000\nepsilon 5\n", "alpha 1000B 80ms\nalpha 2000B 80ms\nalpha 4000B 80ms\nbeta 8000B 300ms\n"] {
            assert!(
                classified_regions(table.as_bytes()).iter().all(|&(_, _, k)| matches!(k, RegionKind::Table(_))),
                "{table:?} reads as one table"
            );
        }
    }

    fn all_tables(input: &str) -> bool {
        classified_regions(input.as_bytes()).iter().all(|&(_, _, k)| matches!(k, RegionKind::Table(_)))
    }

    #[test]
    fn a_column_of_numbers_one_to_a_line_is_a_table() {
        // A period of one token holds no columns, so the rows are the lines,
        // one number each; a line may end at a carriage return alone.
        let column: String = (1..=40).map(|i| format!("{}\n", i * 37 % 1000)).collect();
        assert!(all_tables(&column), "{column:?}");
        let cr = column.replace('\n', "\r");
        assert!(all_tables(&cr), "{cr:?}");
    }

    #[test]
    fn a_list_run_along_one_line_is_no_table() {
        // A value and a separator in turn is a template of period two, and with
        // no line break it has no rows: a list, not a table. Broken into lines
        // of three, the same values are one.
        let list: String = (1..=60).map(|i| format!("{}, ", i * 37 % 1000)).collect();
        let f = field(&list);
        assert!(!f.template_spans().is_empty(), "the list repeats, which is what the row rule weighs");
        assert!(f.table_spans(list.as_bytes()).is_empty(), "{list:?}");
        let rows: String = (1..=60).map(|i| format!("{},{}", i * 37 % 1000, if i % 3 == 0 { "\n" } else { " " })).collect();
        assert!(all_tables(&rows), "{rows:?}");
    }

    #[test]
    fn digests_one_to_a_line_read_as_a_blob_though_they_repeat_as_a_table() {
        // SHA-256 digests from the parallel corpus's manifest.
        let digests = "e5dd0024dd43d92434808c8e1df3c09870a2832744283232aa1dc16c737237b1\n\
            756d56532a8ba4f64281743a9aa5ce68a6a1d25962e722c80c334cab62232fca\n\
            730e4c677c1913d48d81535890070c6ca04b9768fc5edccb7600d1d251ea15ea\n\
            ac4fb8cbb647821b1ffddfa312804f941ca08de57d3cdbcc92f890498cee64f5\n\
            72d31464e6ef9df69c740c364e18099be00dbdca22a12bd04ed919f7e664e6c3\n\
            c6a7202fa3e271d49d0d3a45f75b2b01b687f67698765d04e7e6abbdc93cf0c1\n\
            0c28e709ef02d738012348236e52c0f2ce0b9dad70f567dd73ac7769c08af03d\n\
            f21e2b588e7fb5766c405084872a1a31c79677d5cf49ebfc3b62b3b5863e59ee\n\
            959ad168d33c61cab403f00f22e8faf36a2138d567a5d513fae827d4c9565d12\n\
            4eba41e47c256a732446527a2889eaf7e88bb959ce250905eb5d98fa48fb6b20\n\
            74e6d06c9d5daceafb13535c47e9f28ed95bc9cf08a611bba485499b11e85ab2\n\
            2bb471318d836aa874fecbcf2dd3da00c6bedee70ec20c141f9ca02bc25abe07\n\
            6aab6defb79d1c856cb8fdfb98b9dc93f6144f1d4bf25faad0647c5282ee28ff\n\
            1e0700dba02c4e8baeafc5ab588b7b38c10caa19dbced125fa7472e1651a95e0\n\
            d109b20a1ef2dcc601d3fc3dca2b7d06f423b72d3a0fa650afb4cef5fa8e9a18\n\
            31f54ed24f1733a2029ebbbe08ed8343dd18c06de0356e5e3fc611fe700bd345\n";
        assert!(!field(digests).table_spans(digests.as_bytes()).is_empty(), "the digests are a table by shape alone");
        let regions = classified_regions(digests.as_bytes());
        assert!(regions.iter().all(|&(_, _, k)| k == RegionKind::Blob), "{regions:?}");
    }

    #[test]
    fn a_punctuation_class_is_its_character() {
        let class = |s: &str| shape_class(TokenKind::Punct, s.as_bytes());
        assert_ne!(class("、"), class("。"), "two characters sharing a first byte");
        assert_ne!(class("─"), class("│"), "two box-drawing characters");
        assert_eq!(class(","), (TokenKind::Punct.code() << 16) | u32::from(b','));
    }

    #[test]
    fn orbit_case_fold_finds_the_true_period() {
        // The composition axis: a case-varied repeated phrase "the cat sat". The
        // The identity orbit sees the case-cycle (period 6); the case orbit folds
        // case and recovers the true phrase period (3). Orbit is the
        // pre-transform, shape the measurement that composes over it.
        let s: &[u8] = b"the cat sat THE CAT SAT the cat sat THE CAT SAT";
        let toks = crate::tokutil::lex_sig(s);
        let id = analyze_over(&toks, s, crate::orbit::OrbitGroup::Identity);
        let ca = analyze_over(&toks, s, crate::orbit::OrbitGroup::Case);
        let id_p = id.frames.last().map_or(0, |f| f.period);
        let ca_p = ca.frames.last().map_or(0, |f| f.period);
        assert!(
            ca_p > 0 && ca_p < id_p,
            "case orbit should find a tighter period ({ca_p}) than identity ({id_p})"
        );
    }

    #[test]
    fn orbit_identity_matches_default_silhouette_periodicity() {
        // Sanity: over the Identity orbit the field is well-formed and
        // non-empty on structured input (the literal-token silhouette).
        let s: &[u8] = b"a,1,b,2,a,1,b,2,a,1,b,2";
        let toks = crate::tokutil::lex_sig(s);
        let f = analyze_over(&toks, s, crate::orbit::OrbitGroup::Identity);
        assert_eq!(f.n_tokens, toks.len());
        assert!(f.frames.iter().any(|fr| fr.period > 0));
    }
}
