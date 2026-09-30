//! The text a scan's report is made of, for every surface that writes one:
//! a match as a JSON object, its registers as JSON with each typed value
//! beside its text, an explanation as JSON, a match as the one-input report
//! writes it, and a line of an input with the matches in it painted; and the
//! reports built from them, each line handed to a sink the surface supplies,
//! so the command line and every module write one report the same way.

use crate::ast::Pattern;
use crate::engine::{Match, Span};
use crate::explain::{Explainer, Explanation};
use crate::files::LineIndex;
use crate::paint::{Painter, Role};
use crate::records::RecordUnit;
use crate::token::{Token, TokenKind};

/// Which member of a set made each match of a report, parallel to the
/// matches, and the set the names come from.
#[derive(Clone, Copy)]
pub struct Members<'a> {
    /// The set the scan ran.
    pub set: &'a crate::PatternSet,
    /// The member that made each match, in the report's order.
    pub of: &'a [usize],
}

impl Members<'_> {
    /// The name of the member that made match `k`.
    #[must_use]
    pub fn name(&self, k: usize) -> String {
        self.set.name(self.of[k])
    }
}

/// How a report explains its matches: an explainer for each pattern with a
/// match, and the route the scan took.
pub struct Explaining<'a> {
    explainers: Vec<Option<Explainer<'a>>>,
    route: &'a str,
}

impl<'a> Explaining<'a> {
    /// Explainers over `input` for the patterns whose matches the report
    /// writes: the first pattern where `members` is `None`, every member
    /// with a match otherwise. `count` is how many patterns the scan ran and
    /// `pattern` gives each by its place.
    #[must_use]
    pub fn over<'p>(
        count: usize,
        pattern: impl Fn(usize) -> &'p Pattern,
        input: &'a [u8],
        shapes: &crate::ShapeSet,
        members: Option<Members<'_>>,
        route: &'a str,
    ) -> Self {
        let mut explainers: Vec<Option<Explainer<'a>>> = (0..count).map(|_| None).collect();
        let mut present: Vec<usize> = members.map_or_else(|| vec![0], |ms| ms.of.to_vec());
        present.sort_unstable();
        present.dedup();
        for i in present {
            explainers[i] = Some(Explainer::new(pattern(i), input, shapes));
        }
        Explaining { explainers, route }
    }

    /// The explanation of match `k` of the report, under the pattern that
    /// made it.
    #[must_use]
    pub fn explain(&self, m: &Match, members: Option<Members<'_>>, k: usize) -> Explanation {
        let i = members.map_or(0, |ms| ms.of[k]);
        self.explainers[i].as_ref().expect("an explainer for every pattern with a match").explain(m, self.route)
    }
}

/// The members a match object carries beyond its span, text and captures:
/// `pattern`, the set member that made it, and `explain`, its explanation,
/// where the report has them.
#[must_use]
pub fn json_extras(
    m: &Match,
    members: Option<Members<'_>>,
    k: usize,
    explaining: Option<&Explaining<'_>>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(ms) = members {
        parts.push(format!("\"pattern\":\"{}\"", json_escape(&ms.name(k))));
    }
    if let Some(ex) = explaining {
        parts.push(explanation_json(&ex.explain(m, members, k)));
    }
    if parts.is_empty() { None } else { Some(parts.join(",")) }
}

/// The lines an explanation adds under a match in a text report: the kinds
/// it spans, the guard each guarded kind passed, each axis reading, and the
/// route.
#[must_use]
pub fn explanation_lines(e: &Explanation) -> Vec<String> {
    let tokens: Vec<String> = e.tokens.iter().map(|(kind, text)| format!("{kind} {text:?}")).collect();
    let mut out = vec![format!("  tokens: {}", tokens.join(", "))];
    for guard in &e.guards {
        out.push(format!("  guard: {guard}"));
    }
    for r in &e.readings {
        out.push(format!("  {}: {} {:?}", r.axis, r.value, r.text));
    }
    out.push(format!("  route: {}", e.route));
    out
}

/// The one-input text report, a line at a time into `out`: `[start..end]`
/// and the match's text, its captures and its member, each explanation under
/// its match; `no match` where there are none. The offsets are the ones
/// `index` gives, the whole input's where `input` is a window of it.
pub fn human_report(
    input: &[u8],
    matches: &[Match],
    index: &LineIndex,
    painter: &Painter,
    members: Option<Members<'_>>,
    explaining: Option<&Explaining<'_>>,
    out: &mut dyn FnMut(&str),
) {
    if matches.is_empty() {
        out("no match");
        return;
    }
    for (k, m) in matches.iter().enumerate() {
        let name = members.map(|ms| ms.name(k));
        out(&format!(
            "[{}..{}] {}",
            index.offset(m.start),
            index.offset(m.end),
            match_text(input, m, painter, name.as_deref())
        ));
        if let Some(ex) = explaining {
            for line in explanation_lines(&ex.explain(m, members, k)) {
                out(&line);
            }
        }
    }
}

/// The one-input JSON report: an array of match objects, each carrying its
/// member where the scan ran a set and its explanation where the report
/// explains, their offsets the ones `index` gives.
#[must_use]
pub fn json_report(
    input: &[u8],
    matches: &[Match],
    index: &LineIndex,
    members: Option<Members<'_>>,
    explaining: Option<&Explaining<'_>>,
    values: Option<&ValueView<'_>>,
) -> String {
    let mut out = String::from("[");
    for (k, m) in matches.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        let extra = json_extras(m, members, k, explaining);
        out.push_str(&match_json(input, m, None, index.offset(0), extra.as_deref(), values));
    }
    out.push(']');
    out
}

/// The lines report, a line at a time into `out`: with `invert` the lines no
/// match touches, each selected; otherwise every line, the touched ones
/// selected with their matches painted and the rest carried along.
/// `max_count` caps the selected lines.
// The eight are one report: where it came from, the input and its line index,
// the matches, the two flags that choose the lines, the painter and the sink.
#[allow(clippy::too_many_arguments)]
pub fn lines_report(
    prefix: Option<&str>,
    input: &[u8],
    matches: &[Match],
    index: &LineIndex,
    invert: bool,
    max_count: Option<usize>,
    painter: &Painter,
    out: &mut dyn FnMut(&str),
) {
    // Lexed here rather than taken from the scan, because a scan that
    // answered on a byte route never lexed at all: the kinds are what the
    // report needs, and only a report that paints them pays for them.
    let tokens = if painter.paints_kinds() { crate::lexer::lex(input) } else { Vec::new() };
    let spans: Vec<(usize, usize)> = matches
        .iter()
        .map(|m| (index.line_of(m.start), index.line_of(m.end.saturating_sub(1).max(m.start))))
        .collect();
    let mut from = 0usize;
    let mut selected_so_far = 0usize;
    for line in 0..index.lines() {
        while from < matches.len() && spans[from].1 < line {
            from += 1;
        }
        let mut to = from;
        while to < matches.len() && spans[to].0 <= line {
            to += 1;
        }
        let here = &matches[from..to];
        let touched = !here.is_empty();
        if invert && touched {
            continue;
        }
        let capped = max_count.is_some_and(|n| selected_so_far >= n);
        if invert && capped {
            break;
        }
        let selected = invert || (touched && !capped);
        if selected {
            selected_so_far += 1;
        }
        out(&painted_line(prefix, input, index, line, selected, if selected { here } else { &[] }, painter, &tokens));
    }
}

/// The header `trex head`, `trex tail` and `trex lines` print above an
/// input's lines where they read several, as the coreutils head and tail
/// print it.
#[must_use]
pub fn listing_header(name: &str, painter: &Painter) -> String {
    format!("==> {} <==", painter.paint(Role::Path, name))
}

/// The lines of a window numbered as `trex head -n`, `trex tail -n` and
/// `trex lines -n` print them, a line at a time into `out`: `N:text`, the
/// number the one `index` gives, which is the input's own where the window is
/// part of one, painted as a context line is. An empty window prints nothing.
pub fn numbered_lines(input: &[u8], index: &LineIndex, painter: &Painter, out: &mut dyn FnMut(&str)) {
    if input.is_empty() {
        return;
    }
    // As in the lines report: lexed only where the painter paints kinds.
    let tokens = if painter.paints_kinds() { crate::lexer::lex(input) } else { Vec::new() };
    for line in 0..index.lines() {
        let (s, e) = index.line_span(line);
        out(&format!(
            "{}{}{}",
            painter.paint(Role::Line, &index.number(line).to_string()),
            painter.paint(Role::Separator, ":"),
            painter.paint_kinds(input, s..e, &tokens)
        ));
    }
}

/// What a text report prints around each match: a count of lines on each
/// side, or the record of a unit that holds the match, on the sides asked
/// for.
pub struct Context<'a> {
    /// The lines printed ahead of each match's line.
    pub before: usize,
    /// The lines printed after it.
    pub after: usize,
    /// The unit whose record holding each match is printed in place of the
    /// counts, and whether its part ahead of the match and its part after
    /// are printed.
    pub record: Option<(&'a RecordUnit, bool, bool)>,
}

/// The text report with context, a line at a time into `out`: each match as
/// `line:col: text`, with the context's lines above its first line and below
/// it as `line-text`, a `--` between groups of lines that do not touch, and
/// no context line printed twice or where a match line will be. A prefix
/// names the input ahead of each line, an explainer puts the match's
/// explanation under its line, and the painter colors the path, the numbers,
/// the separators and the match.
// The nine are one report: where it came from, the input and its line index,
// the matches, what surrounds them, the painter, the members and the
// explainer that annotate them, and the sink.
#[allow(clippy::too_many_arguments)]
pub fn context_report(
    prefix: Option<&str>,
    input: &[u8],
    matches: &[Match],
    index: &LineIndex,
    context: &Context<'_>,
    painter: &Painter,
    members: Option<Members<'_>>,
    explaining: Option<&Explaining<'_>>,
    out: &mut dyn FnMut(&str),
) {
    let (before, after) = (context.before, context.after);
    let lines = index.lines();
    let match_lines: Vec<usize> = matches.iter().map(|m| index.line_of(m.start)).collect();
    // The records are read once per input, not once per match: every match
    // asks the same list which construct holds it.
    let records = context.record.map(|(unit, _, _)| unit.records(input));
    // As in the lines report: lexed here because a scan that answered on a
    // byte route never lexed, and only a report that paints kinds pays.
    let tokens = if painter.paints_kinds() { crate::lexer::lex(input) } else { Vec::new() };
    let mut last_printed: Option<usize> = None;
    let sep = |s: &str| painter.paint(Role::Separator, s);
    let context_line = |line: usize, out: &mut dyn FnMut(&str)| {
        let (s, e) = index.line_span(line);
        let text = painter.paint_kinds(input, s..e, &tokens);
        let number = painter.paint(Role::Line, &index.number(line).to_string());
        match prefix {
            Some(p) => out(&format!("{}{}{number}{}{text}", painter.paint(Role::Path, p), sep("-"), sep("-"))),
            None => out(&format!("{number}{}{text}", sep("-"))),
        }
    };
    for (k, (m, &line)) in matches.iter().zip(&match_lines).enumerate() {
        let (line1, col) = index.line_col(input, m.start);
        let (from, to) = match (context.record, &records) {
            // The innermost record holding the match, which is the last one
            // to open at or before it among those that close after it. A
            // match inside no record of the unit stands on its own line.
            (Some((_, ahead, behind)), Some(records)) => {
                let holding = records
                    .iter()
                    .filter(|&&(s, e)| s <= m.start && m.end <= e)
                    .max_by_key(|&&(s, e)| (s, std::cmp::Reverse(e)));
                match holding {
                    Some(&(s, e)) => (
                        if ahead { index.line_of(s) } else { line },
                        if behind { (index.line_of(e.saturating_sub(1)) + 1).min(lines) } else { line + 1 },
                    ),
                    None => (line, line + 1),
                }
            }
            _ => (line.saturating_sub(before), (line + after + 1).min(lines)),
        };
        if (before > 0 || after > 0 || context.record.is_some())
            && let Some(last) = last_printed
            && from > last + 1
        {
            out("--");
        }
        for l in from..line {
            if last_printed.is_some_and(|last| l <= last) || match_lines.binary_search(&l).is_ok() {
                continue;
            }
            context_line(l, &mut *out);
            last_printed = Some(l);
        }
        let place = format!(
            "{}{}{}{} ",
            painter.paint(Role::Line, &line1.to_string()),
            sep(":"),
            painter.paint(Role::Column, &col.to_string()),
            sep(":")
        );
        let name = members.map(|ms| ms.name(k));
        let text = match_text(input, m, painter, name.as_deref());
        match prefix {
            Some(p) => out(&format!("{}{}{place}{text}", painter.paint(Role::Path, p), sep(":"))),
            None => out(&format!("{place}{text}")),
        }
        if let Some(ex) = explaining {
            for said in explanation_lines(&ex.explain(m, members, k)) {
                out(&said);
            }
        }
        last_printed = Some(line);
        for l in line + 1..to {
            if match_lines.binary_search(&l).is_ok() {
                continue;
            }
            context_line(l, &mut *out);
            last_printed = Some(l);
        }
    }
}

/// What a report needs to write a typed register's value: which kind each
/// register binds, how a value is spelled, and the clock that places a
/// timestamp carrying no zone or no year.
///
/// The kinds come from the pattern and the values from each match, so this
/// is built once a scan and read once a register.
pub struct ValueView<'a> {
    /// Each register's name and the one kind it binds, or `None` where it
    /// binds no single kind.
    pub kinds: &'a [(String, Option<TokenKind>)],
    /// How a value is spelled.
    pub style: crate::typed::ValueStyle,
    /// The clock a timestamp with no zone or no year is placed by.
    pub clock: crate::Clock,
}

impl ValueView<'_> {
    /// The kind `name` binds, or `None` where it binds no single kind and so
    /// has no one value to report.
    fn kind_of(&self, name: &str) -> Option<TokenKind> {
        self.kinds.iter().find(|(n, _)| n == name).and_then(|(_, k)| *k)
    }
}

/// `s` escaped to stand between the quotes of a JSON string.
#[must_use]
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
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
    out
}

/// The match object every JSON report writes: its place where the scan ran
/// over named inputs, its span at `base` plus its own offsets for a match
/// found over a window of a longer input, its text, its captures, and
/// `extra` as further members when a report has them.
#[must_use]
pub fn match_json(
    input: &[u8],
    m: &Match,
    at: Option<(&str, usize, usize)>,
    base: usize,
    extra: Option<&str>,
    values: Option<&ValueView<'_>>,
) -> String {
    let span = String::from_utf8_lossy(&input[m.start..m.end]);
    let mut out = String::from("{");
    if let Some((path, line, col)) = at {
        out.push_str(&format!("\"path\":\"{}\",\"line\":{line},\"col\":{col},", json_escape(path)));
    }
    out.push_str(&format!(
        "\"start\":{},\"end\":{},\"text\":\"{}\",\"captures\":{}",
        m.start + base,
        m.end + base,
        json_escape(&span),
        captures_json(input, m, "", None, values)
    ));
    if let Some(members) = extra {
        out.push(',');
        out.push_str(members);
    }
    out.push('}');
    out
}

/// The registers of `m` under `prefix` as a JSON object: a register maps to
/// its text; one with registers nested under it (`pair.k`) to a match-shaped
/// object, its text and its children under `captures`; one bound under a
/// repetition to an array of its bindings, each a text or such an object.
/// Under `within`, one binding of a repetition is being shown and only what
/// lies inside it is: a child bound once in that turn is its text, one bound
/// more than once the array of them, so each turn pairs with its own.
#[must_use]
pub fn captures_json(
    input: &[u8],
    m: &Match,
    prefix: &str,
    within: Option<Span>,
    values: Option<&ValueView<'_>>,
) -> String {
    let names = m.names();
    let inside = |s: &Span| within.is_none_or(|w| s.start() >= w.start() && s.end() <= w.end());
    let text = |s: &Span| json_escape(&String::from_utf8_lossy(&input[s.range()]));
    let mut members: Vec<String> = Vec::new();
    for (name, span) in names.iter().zip(m.captures()) {
        let Some(short) = name.strip_prefix(prefix) else { continue };
        if short.is_empty() || short.contains('.') || (!prefix.is_empty() && !name.starts_with(prefix)) {
            continue;
        }
        let nested = format!("{name}.");
        let has_children = names.iter().any(|n| n.starts_with(&nested));
        // A typed register carries its parsed value beside its text. An
        // untyped one stays the bare string, so a report over patterns that
        // bind no typed kind is the one it would be with no values at all.
        let valued = |s: &Span| {
            let v = values?;
            let kind = v.kind_of(name)?;
            let txt = String::from_utf8_lossy(&input[s.range()]);
            let read = crate::typed::value_of(kind, &txt)?;
            Some(read.json(kind, v.style, v.clock))
        };
        let one = |s: &Span| match (has_children, valued(s)) {
            (true, Some(v)) => format!(
                "{{\"text\":\"{}\",\"value\":{v},\"captures\":{}}}",
                text(s),
                captures_json(input, m, &nested, Some(*s), values)
            ),
            (true, None) => format!(
                "{{\"text\":\"{}\",\"captures\":{}}}",
                text(s),
                captures_json(input, m, &nested, Some(*s), values)
            ),
            (false, Some(v)) => format!("{{\"text\":\"{}\",\"value\":{v}}}", text(s)),
            (false, None) => format!("\"{}\"", text(s)),
        };
        let value = match m.list(name) {
            Some(all) => {
                let mut items: Vec<String> = all.iter().filter(|s| inside(s)).map(one).collect();
                match (within, items.len()) {
                    (Some(_), 0) => "\"\"".to_string(),
                    (Some(_), 1) => items.remove(0),
                    _ => format!("[{}]", items.join(",")),
                }
            }
            None if inside(span) => one(span),
            None => "\"\"".to_string(),
        };
        members.push(format!("\"{}\":{value}", json_escape(short)));
    }
    format!("{{{}}}", members.join(","))
}

/// The explanation as the JSON member a match object carries it under,
/// `explain`: the kinds the match spans, the guard each guarded kind passed,
/// each axis reading, and the route that answered.
#[must_use]
pub fn explanation_json(e: &Explanation) -> String {
    let tokens: Vec<String> = e
        .tokens
        .iter()
        .map(|(kind, text)| format!("{{\"kind\":\"{}\",\"text\":\"{}\"}}", json_escape(kind), json_escape(text)))
        .collect();
    let guards: Vec<String> = e.guards.iter().map(|g| format!("\"{}\"", json_escape(g))).collect();
    let readings: Vec<String> = e
        .readings
        .iter()
        .map(|r| {
            format!(
                "{{\"axis\":\"{}\",\"text\":\"{}\",\"value\":\"{}\"}}",
                r.axis,
                json_escape(&r.text),
                json_escape(&r.value)
            )
        })
        .collect();
    format!(
        "\"explain\":{{\"tokens\":[{}],\"guards\":[{}],\"readings\":[{}],\"route\":\"{}\"}}",
        tokens.join(","),
        guards.join(","),
        readings.join(","),
        json_escape(&e.route)
    )
}

/// The match's text, painted as a match, its captures, and the set member
/// that made it where the scan ran a set, as the one-input report writes
/// them after the span.
#[must_use]
pub fn match_text(input: &[u8], m: &Match, painter: &Painter, pattern: Option<&str>) -> String {
    let quoted = format!("{:?}", String::from_utf8_lossy(&input[m.start..m.end]));
    let mut out = painter.paint(Role::Match, &quoted);
    if !m.captures().is_empty() {
        let caps: Vec<String> = m
            .names()
            .iter()
            .zip(m.captures())
            .map(|(k, s)| format!("{k}={:?}", String::from_utf8_lossy(&input[s.range()])))
            .collect();
        out.push_str("  captures: ");
        out.push_str(&caps.join(", "));
    }
    if let Some(name) = pattern {
        out.push_str("  pattern: ");
        out.push_str(name);
    }
    out
}

/// One line as the `-v` and `--passthru` reports write it: its text with
/// the matches in it painted, and where the report names its inputs, the
/// path and line number ahead of it, joined by `:` for a line the report
/// selects and by `-` for one it carries along.
///
/// The kinds paint what the matches do not, from `tokens`, which is empty
/// where the painter paints no kinds.
// The eight are one line and everything needed to paint it: where it came
// from, the input and its line index, which line, whether the report selected
// it or is carrying it along, the matches in it, the painter, and the tokens.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn painted_line(
    prefix: Option<&str>,
    input: &[u8],
    index: &LineIndex,
    line: usize,
    selected: bool,
    matches: &[Match],
    painter: &Painter,
    tokens: &[Token],
) -> String {
    let (s, e) = index.line_span(line);
    let mut text = String::new();
    let mut at = s;
    for m in matches {
        let (ms, me) = (m.start.max(s), m.end.min(e));
        if me <= ms || ms < at {
            continue;
        }
        // A match is the thing the reader asked for, so its own role paints
        // over whatever kinds its tokens carry rather than under them - and a
        // register painted by name paints over the match for the same
        // reason, being the narrower thing the reader named.
        text.push_str(&painter.paint_kinds(input, at..ms, tokens));
        let mut within = ms;
        if painter.palette().paints_captures() {
            for (name, span) in m.names().iter().zip(m.captures()) {
                let Some(style) = painter.palette().capture(name) else { continue };
                let sgr = style.sgr(painter.depth());
                let (cs, ce) = (span.start().max(within), span.end().min(me));
                if sgr.is_empty() || ce <= cs {
                    continue;
                }
                // An empty run is not painted: a register at the very start
                // of its match would otherwise emit an escape pair with
                // nothing between it, which is bytes that render as nothing
                // and read as a mistake to anything counting them.
                if within < cs {
                    let before = String::from_utf8_lossy(&input[within..cs]).into_owned();
                    text.push_str(&painter.paint(Role::Match, &before));
                }
                text.push_str(&format!("\x1b[{sgr}m{}\x1b[0m", String::from_utf8_lossy(&input[cs..ce])));
                within = ce;
            }
        }
        if within < me {
            text.push_str(&painter.paint(Role::Match, &String::from_utf8_lossy(&input[within..me])));
        }
        at = me;
    }
    text.push_str(&painter.paint_kinds(input, at..e, tokens));
    let Some(p) = prefix else {
        return text;
    };
    let sep = if selected { ":" } else { "-" };
    format!(
        "{}{}{}{}{text}",
        painter.paint(Role::Path, p),
        painter.paint(Role::Separator, sep),
        painter.paint(Role::Line, &index.number(line).to_string()),
        painter.paint(Role::Separator, sep)
    )
}
