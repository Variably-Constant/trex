//! A compiled pattern and the matching every cmdlet shares.
//!
//! A pattern is compiled against the atoms in force when it is compiled: the
//! library a cmdlet was given, or the session's. A `Trex.Pattern` keeps the
//! atoms it was compiled against, so it matches the same way wherever it is
//! passed later, whatever the session has declared since.
//!
//! A declared shape or kind decides where tokens begin and end, so a pattern
//! compiled with one scans on the path that lexes with them; every other
//! pattern takes the routed scan, which may use the device.

use pwrs::prelude::*;

pub(crate) use trex::Engine;

use crate::atoms::{TrexLibrary, atoms_for};
use crate::common::{Offsets, arg_err, parse_err, units_of, value_of};

/// A pattern, the atoms it reads, and its source.
#[derive(Clone)]
pub(crate) struct Compiled {
    pub(crate) source: String,
    pub(crate) inner: trex::ast::Pattern,
    pub(crate) shapes: trex::ShapeSet,
}

impl Compiled {
    /// `source` compiled against `shapes`. With nothing declared and no
    /// directory to read a relative `@file` from this is the plain parse;
    /// otherwise it is the parse `--lib` makes, so a pattern reads alike on
    /// the command line and here.
    pub(crate) fn new(source: &str, shapes: trex::ShapeSet) -> PsResult<Self> {
        let declares = !shapes.is_empty() || !shapes.lets().is_empty() || shapes.base_dir().is_some();
        let inner = if declares {
            trex::parser::parse_with_shapes(source, &shapes).map_err(|e| parse_err(source, &e))?
        } else {
            trex::parse(source).map_err(|e| parse_err(source, &e))?
        };
        Ok(Compiled { source: source.to_string(), inner, shapes })
    }

    /// Whether the scan must lex with the declarations.
    fn shaped(&self) -> bool {
        !self.shapes.is_empty()
    }

    /// Every leftmost, non-overlapping match.
    pub(crate) fn spans(&self, bytes: &[u8]) -> Vec<trex::Span> {
        if self.shaped() {
            trex::scan_with_shapes(&self.inner, bytes, &self.shapes)
        } else {
            trex::scan_with_backend(&self.inner, bytes, trex::Backend::Auto).0
        }
    }

    /// `spans` with every register resolved, a register bound under a
    /// repetition keeping each binding.
    pub(crate) fn resolved(&self, bytes: &[u8], spans: &[trex::Span]) -> Vec<trex::Match> {
        if self.shaped() {
            trex::captures_with_shapes_and_lists(&self.inner, bytes, &self.shapes, spans)
        } else {
            trex::captures_with_lists(&self.inner, bytes, spans)
        }
    }

    /// Every match with its registers resolved.
    pub(crate) fn found(&self, bytes: &[u8]) -> Vec<trex::Match> {
        let spans = self.spans(bytes);
        self.resolved(bytes, &spans)
    }

    /// Every match with its registers resolved, the scan run as `engine`
    /// says, beside a note of how it ran where there is one to say: that
    /// the device ran or could not, the two grains' timing, or that declared
    /// atoms took the scan to the path that lexes with them, which none of
    /// the engine's choices carries. Every choice finds the same matches.
    pub(crate) fn found_by(&self, bytes: &[u8], engine: Engine) -> (Vec<trex::Match>, Option<String>) {
        let (spans, note) = self.spans_by(bytes, engine);
        (self.resolved(bytes, &spans), note)
    }

    /// Every leftmost, non-overlapping match, the scan run as `engine` says,
    /// beside the note [`Self::found_by`] gives of how it ran.
    pub(crate) fn spans_by(&self, bytes: &[u8], engine: Engine) -> (Vec<trex::Span>, Option<String>) {
        if engine.is_plain() {
            return (self.spans(bytes), None);
        }
        if self.shaped() {
            return (self.spans(bytes), Some(SHAPED_LEX.to_string()));
        }
        let (spans, ran) = trex::scan_engine(&self.inner, bytes, &engine);
        let note = ran_note(ran, spans.len());
        (spans, note)
    }

    /// The edits that replace the matches of `text`, found as `engine` says,
    /// with `template` rendered at each, the first `limit` of them where one
    /// is given, as every surface rewrites a file; beside the note of how the
    /// scan ran.
    pub(crate) fn template_edits_by(
        &self,
        text: &str,
        template: &trex::Template,
        limit: Option<usize>,
        engine: Engine,
    ) -> (Vec<trex::files::Edit>, Option<String>) {
        let bytes = text.as_bytes();
        let (edits, ran) = trex::rewrite::edits_by(&self.inner, template, bytes, &self.shapes, &engine, limit);
        let note = if self.shaped() && !engine.is_plain() {
            Some(SHAPED_LEX.to_string())
        } else {
            ran.and_then(|ran| ran_note(ran, edits.len()))
        };
        (edits, note)
    }

    /// Whether anything matches.
    pub(crate) fn is_match(&self, bytes: &[u8]) -> bool {
        if self.shaped() { !self.spans(bytes).is_empty() } else { trex::is_match(&self.inner, bytes) }
    }

    /// The first match, found without scanning past it where the pattern
    /// allows.
    pub(crate) fn first(&self, bytes: &[u8]) -> Option<trex::Span> {
        if self.shaped() { self.spans(bytes).into_iter().next() } else { trex::find(&self.inner, bytes) }
    }

    /// The text between the matches, at most `limit` pieces, the last
    /// holding whatever the cut stopped short of.
    pub(crate) fn split(&self, text: &str, limit: Option<usize>) -> Vec<String> {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        if limit == Some(0) {
            return out;
        }
        let mut at = 0usize;
        for span in self.spans(bytes) {
            if limit.is_some_and(|n| out.len() + 1 >= n) {
                break;
            }
            out.push(String::from_utf8_lossy(&bytes[at..span.start()]).into_owned());
            at = span.end();
        }
        out.push(String::from_utf8_lossy(&bytes[at..]).into_owned());
        out
    }

    /// `text` with each match, or the first `limit`, replaced by `template`
    /// rendered at it.
    pub(crate) fn replace(&self, text: &str, template: &str, limit: Option<usize>) -> PsResult<String> {
        let t = trex::Template::parse(template, &self.inner.capture_names())
            .map_err(|e| arg_err("TrexTemplateError", format!("template error at byte {}: {}", e.pos, e.msg)))?;
        let bytes = text.as_bytes();
        let out = if self.shaped() {
            let mut edits = trex::rewrite::edits_with_shapes(&self.inner, &t, bytes, &self.shapes);
            if let Some(n) = limit {
                edits.truncate(n);
            }
            splice(bytes, &edits)
        } else {
            match limit {
                Some(n) => trex::rewrite_n(&self.inner, &t, bytes, n),
                None => trex::rewrite(&self.inner, &t, bytes),
            }
        };
        String::from_utf8(out).map_err(|e| arg_err("TrexTemplateError", format!("the rewritten text is not UTF-8: {e}")))
    }

    /// The matches of `text` as objects, each beside the byte offset it
    /// starts at, which a caller that read a file turns into a line and a
    /// column; `path`, `line` and `column` are left for that caller to fill.
    pub(crate) fn matches(&self, text: &str, limit: Option<usize>) -> PsResult<Vec<(usize, TrexMatch)>> {
        let mut spans = self.spans(text.as_bytes());
        if let Some(n) = limit {
            spans.truncate(n);
        }
        self.matches_at(text, &spans)
    }

    /// [`Self::matches`] with the scan run as `engine` says, beside the note
    /// of how it ran.
    pub(crate) fn matches_by(&self, text: &str, limit: Option<usize>, engine: Engine) -> PsResult<(Placed, Option<String>)> {
        let (mut spans, note) = self.spans_by(text.as_bytes(), engine);
        if let Some(n) = limit {
            spans.truncate(n);
        }
        Ok((self.matches_at(text, &spans)?, note))
    }

    /// The matches of `text` at `spans`, found by this pattern, as
    /// [`Self::matches`] gives them.
    pub(crate) fn matches_at(&self, text: &str, spans: &[trex::Span]) -> PsResult<Vec<(usize, TrexMatch)>> {
        let bytes = text.as_bytes();
        let found = self.resolved(bytes, spans);
        let kinds = self.inner.capture_kinds();
        let mut units = units_of(bytes);
        let mut out = Vec::with_capacity(found.len());
        for m in &found {
            out.push((m.start, TrexMatch::of(text, &mut units, m, &kinds, &self.shapes)?));
        }
        Ok(out)
    }
}

/// Matches as objects, each beside the byte offset it starts at.
pub(crate) type Placed = Vec<(usize, TrexMatch)>;

/// What a scan of a pattern compiled with declared atoms says when an engine
/// choice was made: the choices do not carry the declarations.
const SHAPED_LEX: &str =
    "declared atoms decide this lex, which -Backend, -DualGrain and -ChunkSize do not carry; the scan lexed with them";

/// What a scan through an engine says of how it ran, `matches` the matches it
/// found; nothing for a chunked scan or a routed one on the CPU.
fn ran_note(ran: trex::Ran, matches: usize) -> Option<String> {
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    match ran {
        trex::Ran::Device => Some(format!("the device scanned, {matches} matches")),
        trex::Ran::DeviceDeclined => Some(
            "the device does not take this scan, since the pattern is outside what it runs, no device is present, or the build carries none; the CPU scanned".to_string(),
        ),
        trex::Ran::DualGrain(t) => Some(format!(
            "dual grain: {} chunks; byte grain busy {:.2} ms, token grain busy {:.2} ms, wall {:.2} ms; overlap at least {:.2} ms",
            t.chunks,
            ms(t.producer_busy),
            ms(t.consumer_busy),
            ms(t.wall),
            ms(t.overlap())
        )),
        trex::Ran::Routed(trex::BackendUsed::Split) => {
            Some(format!("split across the cores and the device, {matches} matches"))
        }
        trex::Ran::Chunked | trex::Ran::Routed(_) => None,
    }
}

/// `input` with each edit's bytes in its place and the gaps copied verbatim.
pub(crate) fn splice(input: &[u8], edits: &[trex::files::Edit]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut at = 0;
    for e in edits {
        out.extend_from_slice(&input[at..e.start]);
        out.extend_from_slice(&e.replacement);
        at = e.end;
    }
    out.extend_from_slice(&input[at..]);
    out
}

/// What one register bound.
#[psclass(name = "Trex.Capture", show = "{Name}={Text}")]
#[derive(Clone, Default)]
pub struct TrexCapture {
    /// The register's name.
    pub name: String,
    /// The text it bound; for a register bound under a repetition, the last
    /// binding.
    pub text: String,
    /// Where the text starts in the input, in UTF-16 code units.
    pub start: i64,
    /// How many UTF-16 code units the text spans.
    pub length: i64,
    /// The token kind the register binds, such as `ip` or `timestamp`, a
    /// declared shape or kind by the name its declaration gave it, or empty
    /// where it binds more than one kind.
    pub kind: String,
    /// The text parsed as its kind: a number as `long` or `decimal`, a
    /// duration as `TimeSpan`, an instant as `DateTimeOffset`, an address
    /// or a version as its canonical text; `$null` where it has none.
    pub value: PsObject,
    /// Every binding of a register bound under a repetition, oldest first.
    pub items: Vec<String>,
}

/// One match.
#[psclass(name = "Trex.Match")]
#[derive(Clone, Default)]
pub struct TrexMatch {
    /// The file the match is in, or empty for text.
    pub path: String,
    /// The line the match starts on, counting from 1.
    pub line_number: i64,
    /// The character the match starts at within its line, counting from 1.
    pub column: i64,
    /// Where the match starts in the input, in UTF-16 code units, so
    /// `$input.Substring($m.Start, $m.Length)` is the match.
    pub start: i64,
    /// How many UTF-16 code units the match spans.
    pub length: i64,
    /// The matched text.
    pub text: String,
    /// A hashtable of each register's name to the text it bound, so
    /// `$m.Captures.name` reads one.
    pub captures: PsObject,
    /// Each register the pattern bound, in the order the pattern writes
    /// them, with its span, its kind and its parsed value.
    pub groups: Vec<TrexCapture>,
    /// The pattern's name within a pattern set, or empty.
    pub pattern: String,
    /// The lines before the match's line, with -Context.
    pub pre_context: Vec<String>,
    /// The lines after the match's line, with -Context.
    pub post_context: Vec<String>,
    /// What the match is made of, a Trex.Explanation, with -Explain;
    /// `$null` otherwise.
    pub explanation: PsObject,
}

impl TrexMatch {
    /// This match of a part of a longer input, as a match of that input: its
    /// start and each register's moved by `units`, the UTF-16 code units of
    /// the input ahead of the part.
    #[must_use]
    pub(crate) fn moved(mut self, units: usize) -> Self {
        let by = units as i64;
        self.start += by;
        for g in &mut self.groups {
            g.start += by;
        }
        self
    }

    /// The match `m` of `text`, each register with the kind `kinds` gives it,
    /// a declared one named as its declaration in `shapes` names it.
    pub(crate) fn of(
        text: &str,
        units: &mut Offsets<'_>,
        m: &trex::Match,
        kinds: &[(String, Option<trex::token::TokenKind>)],
        shapes: &trex::ShapeSet,
    ) -> PsResult<Self> {
        let bytes = text.as_bytes();
        let slice = |s: usize, e: usize| String::from_utf8_lossy(&bytes[s..e]).into_owned();
        let start = units.at(m.start);
        let end = units.at(m.end);
        let mut captures = std::collections::HashMap::new();
        let mut groups = Vec::with_capacity(m.names().len());
        for (name, span) in m.names().iter().zip(m.captures()) {
            let (bs, be) = (span.start(), span.end());
            let items: Vec<String> = match m.list(name) {
                Some(all) => all.iter().map(|b| slice(b.start(), b.end())).collect(),
                None => Vec::new(),
            };
            let bound = slice(bs, be);
            let kind = kinds.iter().find(|(n, _)| n == name).and_then(|(_, k)| *k);
            let value = match kind {
                Some(k) => value_of(k, &bound)?,
                None => PsObject::default(),
            };
            let s = units.at(bs);
            let e = units.at(be);
            captures.insert(name.clone(), bound.clone());
            groups.push(TrexCapture {
                name: name.clone(),
                text: bound,
                start: s as i64,
                length: (e - s) as i64,
                kind: match kind {
                    Some(k) => shapes.kind_name(k),
                    None => String::new(),
                },
                value,
                items,
            });
        }
        Ok(TrexMatch {
            path: String::new(),
            line_number: 0,
            column: 0,
            start: start as i64,
            length: (end - start) as i64,
            text: slice(m.start, m.end),
            captures: captures.into_ps()?,
            groups,
            pattern: String::new(),
            pre_context: Vec::new(),
            post_context: Vec::new(),
            explanation: PsObject::default(),
        })
    }
}

/// One token a match spans, as an explanation reads it.
#[psclass(name = "Trex.ExplainedToken", show = "{Kind} {Text}")]
#[derive(Clone, Default)]
pub struct TrexExplainedToken {
    /// The token's kind.
    pub kind: String,
    /// The token's text.
    pub text: String,
}

/// One reading of an axis the pattern reads, at a token of the match.
#[psclass(name = "Trex.Reading", show = "{Axis} {Text}: {Value}")]
#[derive(Clone, Default)]
pub struct TrexReading {
    /// The axis read.
    pub axis: String,
    /// The token's text.
    pub text: String,
    /// The value the pattern compared, as the axis gives it.
    pub value: String,
    /// The values the reading gives, each under the name a report template
    /// uses after the dot in `${@axis.piece}`; empty for an axis that gives
    /// one value.
    pub parts: PsObject,
}

/// What a match is made of: the kinds of the tokens it spans, the check
/// each guarded kind passed, every axis the pattern read at them, and the
/// route the scan took.
#[psclass(name = "Trex.Explanation")]
#[derive(Clone, Default)]
pub struct TrexExplanation {
    /// The significant tokens the match spans, with their kinds.
    pub tokens: Vec<TrexExplainedToken>,
    /// The check each guarded token passed to be its kind, such as a card
    /// number's Luhn check.
    pub guards: Vec<String>,
    /// Each axis the pattern reads, at each token of the match.
    pub readings: Vec<TrexReading>,
    /// The rung of the scan ladder that answered the scan of the match's
    /// input, as the `trex` command's `--explain` names it.
    pub route: String,
}

/// An explanation as the object a match carries.
pub(crate) fn explanation_of(e: &trex::explain::Explanation) -> PsResult<PsObject> {
    let mut readings = Vec::with_capacity(e.readings.len());
    for r in &e.readings {
        let mut parts = std::collections::HashMap::new();
        for (name, value) in &r.parts {
            parts.insert(name.to_string(), value.clone());
        }
        readings.push(TrexReading {
            axis: r.axis.to_string(),
            text: r.text.clone(),
            value: r.value.clone(),
            parts: parts.into_ps()?,
        });
    }
    TrexExplanation {
        tokens: e.tokens.iter().map(|(kind, text)| TrexExplainedToken { kind: kind.clone(), text: text.clone() }).collect(),
        guards: e.guards.clone(),
        readings,
        route: e.route.clone(),
    }
    .into_ps()
}

/// A compiled TREX pattern and the atoms it was compiled against.
#[psclass(name = "Trex.Pattern", mode = proxy)]
pub struct TrexPattern {
    /// The pattern's source text.
    pub source: String,
    /// The registers the pattern binds, in the order it writes them.
    pub capture_names: Vec<String>,
    #[psfield(skip)]
    pub(crate) compiled: Compiled,
}

impl TrexPattern {
    pub(crate) fn of(compiled: Compiled) -> Self {
        TrexPattern {
            source: compiled.source.clone(),
            capture_names: compiled.inner.capture_names(),
            compiled,
        }
    }
}

#[psmethods]
impl TrexPattern {
    /// Compiles `source` with the atoms TREX ships and none declared;
    /// New-TrexPattern compiles against the session's or a library's.
    pub fn new(source: String) -> PsResult<Self> {
        Ok(TrexPattern::of(Compiled::new(&source, trex::ShapeSet::new())?))
    }

    /// Whether the pattern matches anywhere in `input`.
    pub fn is_match(&self, input: String) -> PsResult<bool> {
        Ok(self.compiled.is_match(input.as_bytes()))
    }

    /// The first match in `input`, or `$null`.
    pub fn find(&self, input: String) -> PsResult<Option<TrexMatch>> {
        let bytes = input.as_bytes();
        let Some(span) = self.compiled.first(bytes) else {
            return Ok(None);
        };
        let found = self.compiled.resolved(bytes, &[span]);
        let kinds = self.compiled.inner.capture_kinds();
        let mut units = units_of(bytes);
        match found.first() {
            Some(m) => Ok(Some(TrexMatch::of(&input, &mut units, m, &kinds, &self.compiled.shapes)?)),
            None => Ok(None),
        }
    }

    /// Every match in `input`.
    pub fn find_all(&self, input: String) -> PsResult<Vec<TrexMatch>> {
        Ok(self.compiled.matches(&input, None)?.into_iter().map(|(_, m)| m).collect())
    }

    /// `input` with each match replaced by `template` rendered at it:
    /// `${name}` a register, `${0}` the match, `${ip:octet1-2}` a typed
    /// slice.
    pub fn replace(&self, input: String, template: String) -> PsResult<String> {
        self.compiled.replace(&input, &template, None)
    }

    /// The text between the matches in `input`.
    pub fn split(&self, input: String) -> PsResult<Vec<String>> {
        Ok(self.compiled.split(&input, None))
    }
}

/// The pattern a cmdlet was given: a `Trex.Pattern`, used as compiled, or
/// source text, compiled here against the library given or the session's
/// atoms.
pub(crate) fn pattern_arg(
    ps: &Pipeline<'_>,
    given: &PsObject,
    library: &Option<PsProxy<TrexLibrary>>,
) -> PsResult<Compiled> {
    if given.is_null() {
        return Err(arg_err("TrexNoPattern", "a pattern is required"));
    }
    if given.type_name()? == "Trex.Pattern" {
        let proxy = PsProxy::<TrexPattern>::from_ps(given)?;
        return proxy.with(|p| p.compiled.clone());
    }
    let source = String::from_ps(given)?;
    Compiled::new(&source, atoms_for(ps, library)?)
}

/// Compiles a TREX pattern against the atoms in force, for reuse across
/// commands and as an object with IsMatch, Find, FindAll, Replace and Split
/// methods.
///
/// The pattern keeps the atoms it was compiled against, so it matches the
/// same way wherever it is passed later.
///
/// # Examples
/// $email = New-TrexPattern '\E:e'
/// $email.Find('ping bob@x.com').Captures.e
/// Select-TrexMatch $email -Path ./logs
#[cmdlet(verb = "New", noun = "TrexPattern", alias = "New-TxPattern", output = ["Trex.Pattern"])]
#[derive(Default)]
pub struct NewTrexPattern {
    /// The pattern's source text.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub pattern: String,
    /// The atoms to compile against, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
}

impl Cmdlet for NewTrexPattern {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let compiled = Compiled::new(&self.pattern, atoms_for(ps, &self.library)?)?;
        ps.write(TrexPattern::of(compiled))
    }
}
