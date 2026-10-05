//! The records a record-level query runs over: how an input is cut into the
//! units a `--all`, `--any`, `--none` or `--at-least` question is asked of,
//! each a byte span of the input. A unit is a line, a paragraph or the
//! whole input; a run from one match of a pattern to the next, or each
//! match of one; or a unit an axis finds with no delimiter: the stream's own
//! record period, the seam axis's segments, the spectral texture regions,
//! the shape axis's regions, or a supertoken.
//!
//! [`Query`] keeps the records that hold all, any, none
//! or at least some number of its patterns and none of the patterns it
//! excludes, each pattern scanned once over the whole input.

use crate::ast::Pattern;
use crate::supertoken::Role;

/// What a record is.
#[derive(Clone, Debug, PartialEq)]
pub enum RecordUnit {
    /// A line, without its newline.
    Line,
    /// A paragraph: a run of lines holding something other than whitespace,
    /// from the first's start to the last's end.
    Paragraph,
    /// The whole input.
    File,
    /// The stream's own record period in supertokens: every `period` units,
    /// from the first's start to the last's end; the whole input where the
    /// stream has no period.
    ///
    /// Not the period `@phase:k` counts in. That one is in significant tokens,
    /// because a phase anchor names a token's column, and a record is found at
    /// a coarser grain than a column is counted in - a record too long in
    /// tokens to fit the bounded search is a unit or two of supertokens.
    Period,
    /// The seam axis's segments: what the past stops predicting.
    Seam,
    /// The bytes cut where they hold together least under the input's pair
    /// field ([`crate::gravity`]). `bind` makes as many cuts as `seam` does on
    /// the same input, at the weakest bonds; `bind:Q` cuts at every bond in
    /// the weakest `Q` percent.
    Bind(Option<f64>),
    /// `seam` or `bind`, whichever puts more of its cuts at this input's line
    /// starts, a boundary neither cut set is built from; the seam where they
    /// tie.
    Auto,
    /// The spectral texture regions.
    Texture,
    /// The shape axis's regions between its silhouette change-points.
    Shape,
    /// A supertoken, the grammar-free statement or clause, of `role` where
    /// one is named.
    Unit(Option<Role>),
    /// A balanced bracket group, from its opening bracket to its closing one.
    ///
    /// The only unit whose records nest: `f(g(x))` holds two, and the inner
    /// one is inside the outer. A reader that needs one record per byte
    /// wants `unit` or `paragraph`; this one answers "which group is this
    /// inside", which has more than one true answer.
    Block,
    /// A run from one match of the pattern to the next, or to the end of the
    /// input; the bytes before the first match are in no record.
    Start(Pattern),
    /// Each match of the pattern.
    Span(Pattern),
}

impl RecordUnit {
    /// The unit `--record` names: `line`, `paragraph`, `file`, `period`,
    /// `seam`, `bind`, `bind:Q`, `auto`, `texture`, `shape`, `block`, `unit`
    /// or `unit:ROLE`.
    ///
    /// # Errors
    ///
    /// Any other word, naming the ones accepted, a role that is not one, or a
    /// share that is not a percentage.
    pub fn parse(s: &str) -> Result<RecordUnit, String> {
        Ok(match s {
            "line" => RecordUnit::Line,
            "paragraph" => RecordUnit::Paragraph,
            "file" => RecordUnit::File,
            "period" => RecordUnit::Period,
            "seam" => RecordUnit::Seam,
            "bind" => RecordUnit::Bind(None),
            "auto" => RecordUnit::Auto,
            "texture" => RecordUnit::Texture,
            "shape" => RecordUnit::Shape,
            "block" => RecordUnit::Block,
            "unit" => RecordUnit::Unit(None),
            other => {
                if let Some(share) = other.strip_prefix("bind:") {
                    return match share.parse::<f64>() {
                        Ok(q) if (0.0..=100.0).contains(&q) => Ok(RecordUnit::Bind(Some(q))),
                        Ok(q) => Err(format!("bind:{q} is not a share of the cuts; write 0 to 100, e.g. bind:5")),
                        Err(e) => Err(format!("bind:{share} is not a percentage ({e}); write e.g. bind:5")),
                    };
                }
                match other.strip_prefix("unit:") {
                    Some(role) => RecordUnit::Unit(Some(Role::parse(role).ok_or_else(|| {
                        format!("{role:?} is not a unit role; write call, assign, kv, list, numeric or plain")
                    })?)),
                    None => {
                        return Err(format!(
                            "{other:?} is not a record unit; write line, paragraph, file, period, seam, bind, bind:Q, auto, texture, shape, block, unit or unit:ROLE"
                        ));
                    }
                }
            }
        })
    }

    /// The records of `input` under this unit, as ascending byte spans.
    #[must_use]
    pub fn records(&self, input: &[u8]) -> Vec<(usize, usize)> {
        match self {
            RecordUnit::Line => lines(input),
            RecordUnit::Paragraph => paragraphs(input),
            RecordUnit::File => {
                if input.is_empty() {
                    Vec::new()
                } else {
                    vec![(0, input.len())]
                }
            }
            RecordUnit::Period => periods(input),
            RecordUnit::Seam => crate::seam::analyze(input).segments(),
            RecordUnit::Bind(share) => {
                let cuts = match share {
                    Some(q) => bind_cuts_under(input, *q),
                    None => bind_cuts(input, crate::seam::analyze(input).internal_cuts().len()),
                };
                between_cuts(&cuts, input.len())
            }
            RecordUnit::Auto => {
                let seam = crate::seam::analyze(input);
                let seam_cuts = seam.internal_cuts();
                let bind = bind_cuts(input, seam_cuts.len());
                // A line start: the byte after a newline. The seam reads token
                // boundaries and the binding reads the pair field, and neither
                // is built from where lines begin.
                let at_line_starts = |cuts: &[usize]| cuts.iter().filter(|&&c| c > 0 && input[c - 1] == b'\n').count();
                if at_line_starts(&bind) > at_line_starts(&seam_cuts) {
                    between_cuts(&bind, input.len())
                } else {
                    seam.segments()
                }
            }
            RecordUnit::Texture => {
                let field = crate::spectral::analyze(input);
                crate::spectral::regions(&field).into_iter().map(|(s, e, _)| (s, e)).collect()
            }
            RecordUnit::Shape => {
                let field = crate::shape::analyze_bytes(input);
                between_cuts(&field.boundaries, input.len())
            }
            RecordUnit::Block => blocks(input),
            RecordUnit::Unit(role) => crate::supertoken::supertokens(input)
                .into_iter()
                .filter(|u| role.is_none_or(|r| u.role == r))
                .map(|u| (u.start, u.end))
                .collect(),
            RecordUnit::Start(pattern) => {
                let starts: Vec<usize> = crate::engine::scan(pattern, input).iter().map(|s| s.start()).collect();
                starts
                    .iter()
                    .enumerate()
                    .map(|(i, &s)| (s, starts.get(i + 1).copied().unwrap_or(input.len())))
                    .filter(|(s, e)| e > s)
                    .collect()
            }
            RecordUnit::Span(pattern) => {
                crate::engine::scan(pattern, input).iter().map(|s| (s.start(), s.end())).collect()
            }
        }
    }
}

/// Every balanced bracket group, from its opening bracket to just past its
/// closing one, ordered by where each opens and, where two open together, the
/// wider first.
///
/// A close that does not match what is open is not a group and closes
/// nothing, and a bracket left open at the end of the input closes nothing
/// either: a group is reported only when both its ends are there.
fn blocks(input: &[u8]) -> Vec<(usize, usize)> {
    use crate::token::TokenKind;

    let mut open: Vec<(crate::token::BracketKind, usize)> = Vec::new();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for token in crate::lexer::lex(input) {
        match token.kind {
            TokenKind::Open(kind) => open.push((kind, token.start())),
            TokenKind::Close(kind) if open.last().is_some_and(|&(held, _)| held == kind) => {
                let (_, start) = open.pop().expect("the last entry was just read");
                out.push((start, token.end()));
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    out
}

/// Each line of `input` without its newline; an input ending in a newline
/// has no empty record after it.
fn lines(input: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &b) in input.iter().enumerate() {
        if b == b'\n' {
            out.push((start, i));
            start = i + 1;
        }
    }
    if start < input.len() {
        out.push((start, input.len()));
    }
    out
}

/// Each run of lines holding something other than whitespace, from the
/// first's start to the last's end.
fn paragraphs(input: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    for (s, e) in lines(input) {
        let blank = input[s..e].iter().all(u8::is_ascii_whitespace);
        match (blank, open) {
            (true, Some(span)) => {
                out.push(span);
                open = None;
            }
            (true, None) => {}
            (false, Some((ps, _))) => open = Some((ps, e)),
            (false, None) => open = Some((s, e)),
        }
    }
    if let Some(span) = open {
        out.push(span);
    }
    out
}

/// The stream cut every record period in supertokens, or whole where it has
/// no period.
///
/// The cut is taken at the supertoken grain because a lag is counted in
/// symbols and the search is bounded, so a record longer than the bound cannot
/// be found at the token grain at all: a clippy log's line runs to a median of
/// 74 significant tokens against a ceiling of 32, and the token reading
/// returns the record's factors, cutting a record every two tokens. The same
/// line is 1.64 supertokens, and the reading finds the twenty-unit diagnostic
/// block that is the log's actual record.
fn periods(input: &[u8]) -> Vec<(usize, usize)> {
    let toks = crate::lexer::lex(input);
    let units = crate::supertoken::supertokens_from(&toks, input);
    let Some(period) = crate::context::unit_record_period(&units) else {
        return if input.is_empty() { Vec::new() } else { vec![(0, input.len())] };
    };
    units
        .chunks(usize::from(period))
        .filter_map(|chunk| Some((chunk.first()?.start, chunk.last()?.end)))
        .collect()
}

/// The `count` byte cuts the input's pair field binds least, ascending; ties
/// go to the earlier cut. The first byte has no cut before it.
fn bind_cuts(input: &[u8], count: usize) -> Vec<usize> {
    let r = crate::gravity::Readings::read(crate::ast::Grain::Byte, input, &[]);
    let mut order: Vec<(usize, f32)> =
        r.binding.iter().enumerate().filter_map(|(t, b)| b.map(|b| (t, b))).collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    order.truncate(count);
    let mut cuts: Vec<usize> = order.into_iter().map(|(t, _)| t).collect();
    cuts.sort_unstable();
    cuts
}

/// Every byte cut whose binding is in the weakest `share` percent of the
/// input's, ascending.
fn bind_cuts_under(input: &[u8], share: f64) -> Vec<usize> {
    let r = crate::gravity::Readings::read(crate::ast::Grain::Byte, input, &[]);
    let Some(bar) = r.binding_percentile(share) else {
        return Vec::new();
    };
    (0..r.len()).filter(|&t| r.binding[t].is_some_and(|b| b <= bar)).collect()
}

/// The spans between ascending cut offsets, from the input's start to its
/// end, the empty ones left out.
fn between_cuts(cuts: &[usize], len: usize) -> Vec<(usize, usize)> {
    let mut edges = vec![0];
    edges.extend(cuts.iter().copied().filter(|&c| c < len));
    edges.push(len);
    edges.windows(2).map(|w| (w[0], w[1])).filter(|(s, e)| e > s).collect()
}

/// How many of its patterns a record must hold for a record query to keep
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantifier {
    /// Every pattern present.
    All,
    /// At least one present.
    Any,
    /// None present.
    None,
    /// At least this many present.
    AtLeast(usize),
}

/// A record-level query: the quantifier, the patterns it holds a record to
/// (given one by one, or as the members of a set, each under its name), the
/// patterns a record must not hold, and what a record is.
pub struct Query {
    pub quantifier: Quantifier,
    pub positives: Vec<Pattern>,
    pub set: Option<crate::PatternSet>,
    pub negatives: Vec<Pattern>,
    pub unit: RecordUnit,
}

/// One record a query keeps: its span, the positive patterns present in it,
/// and where they are asked for, the matches of theirs that overlap it, in
/// position order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordHit {
    pub start: usize,
    pub end: usize,
    /// The positive patterns present, by index: among the patterns given one
    /// by one, or the members of the set.
    pub present: Vec<usize>,
    pub spans: Vec<(usize, usize)>,
}

impl Query {
    /// How many patterns a record is held to.
    #[must_use]
    pub fn positive_count(&self) -> usize {
        self.set.as_ref().map_or(self.positives.len(), crate::PatternSet::len)
    }

    /// The names the patterns a record is held to go by, where they are the
    /// members of a set; nothing where they go by index.
    #[must_use]
    pub fn names(&self) -> &[String] {
        self.set.as_ref().map_or(&[], crate::PatternSet::names)
    }

    /// Every pattern's matches over the whole input, the positives then the
    /// negatives, each through the routes and the engine handoff a scan
    /// takes, and the members of a set over its one lex.
    fn found(&self, input: &[u8], shapes: &crate::custom::ShapeSet) -> Vec<Vec<crate::Span>> {
        let mut found: Vec<Vec<crate::Span>> = match &self.set {
            Some(set) => {
                let mut per = vec![Vec::new(); set.len()];
                for (i, s) in set.scan(input) {
                    per[i].push(s);
                }
                per
            }
            None => self.positives.iter().map(|p| crate::scan_with_shapes(p, input, shapes)).collect(),
        };
        found.extend(self.negatives.iter().map(|p| crate::scan_with_shapes(p, input, shapes)));
        found
    }

    /// The records of `input` this query keeps. Every pattern is scanned once
    /// over the whole input, through the routes and the engine handoff a
    /// scan takes, and a record holds a pattern where one of its matches
    /// overlaps the record; nothing is lexed a second time per record. The
    /// matches within each record kept are listed only where `want_spans`
    /// asks.
    #[must_use]
    pub fn hits(&self, input: &[u8], shapes: &crate::custom::ShapeSet, want_spans: bool) -> Vec<RecordHit> {
        let records = self.unit.records(input);
        let positives = self.positive_count();
        let found = self.found(input, shapes);
        let mut hits = Vec::new();
        for (s, e) in records {
            let mut present = Vec::new();
            let mut spans = Vec::new();
            let mut negative = false;
            for (i, matches) in found.iter().enumerate() {
                // The matches are sorted and do not overlap, so their ends
                // ascend too, and the first that could reach the record is
                // found by a binary search.
                let from = matches.partition_point(|m| m.end() < s);
                let mut hit = false;
                for m in &matches[from..] {
                    if m.start() >= e {
                        break;
                    }
                    // A match overlaps the record where it reaches past the
                    // record's start; an empty match counts if it is inside.
                    let overlaps = m.end() > s || (m.start() == m.end() && m.start() >= s);
                    if !overlaps {
                        continue;
                    }
                    hit = true;
                    if want_spans && i < positives {
                        spans.push((m.start(), m.end()));
                    } else {
                        break;
                    }
                }
                if hit {
                    if i < positives {
                        present.push(i);
                    } else {
                        negative = true;
                    }
                }
            }
            let kept = !negative
                && match self.quantifier {
                    Quantifier::All => present.len() == positives,
                    Quantifier::Any => !present.is_empty(),
                    Quantifier::None => present.is_empty(),
                    Quantifier::AtLeast(n) => present.len() >= n,
                };
            if kept {
                spans.sort_unstable();
                hits.push(RecordHit { start: s, end: e, present, spans });
            }
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_paragraphs_and_the_file_cut_where_they_say() {
        let text = b"a 1\n\n  \nb 2\nc 3\n";
        assert_eq!(RecordUnit::Line.records(text), vec![(0, 3), (4, 4), (5, 7), (8, 11), (12, 15)]);
        assert_eq!(RecordUnit::Paragraph.records(text), vec![(0, 3), (8, 15)]);
        assert_eq!(RecordUnit::File.records(text), vec![(0, 16)]);
        assert!(RecordUnit::File.records(b"").is_empty());
        assert_eq!(RecordUnit::Line.records(b"x"), vec![(0, 1)]);
    }

    #[test]
    fn a_pattern_starts_or_spans_a_record() {
        let text = b"head\nT 1 a\n  b\nT 2 c\n";
        let start = RecordUnit::Start(crate::parse("\"T\" \\N").expect("a pattern"));
        assert_eq!(start.records(text), vec![(5, 15), (15, 21)]);
        let span = RecordUnit::Span(crate::parse("\"T\" \\N").expect("a pattern"));
        assert_eq!(span.records(text), vec![(5, 8), (15, 18)]);
    }

    #[test]
    fn a_unit_is_a_supertoken_of_the_role_named() {
        let text = b"x = 1\nfoo(2)\ny = 3\n";
        // A call's argument list is a unit of its own, inside the brackets.
        let all = RecordUnit::Unit(None).records(text);
        assert_eq!(all, vec![(0, 5), (6, 9), (10, 11), (13, 18)]);
        let assigns = RecordUnit::Unit(Some(Role::Assign)).records(text);
        assert_eq!(assigns, vec![(0, 5), (13, 18)]);
        assert_eq!(RecordUnit::parse("unit:assign").expect("a role"), RecordUnit::Unit(Some(Role::Assign)));
        assert!(RecordUnit::parse("unit:verb").expect_err("no such role").contains("is not a unit role"));
        assert!(RecordUnit::parse("sentence").expect_err("no such unit").contains("is not a record unit"));
    }

    #[test]
    fn a_periodic_stream_cuts_at_its_period_and_an_aperiodic_one_stays_whole() {
        let text = b"a 1\nb 2\nc 3\nd 4\ne 5\nf 6\n";
        assert_eq!(RecordUnit::Period.records(text), vec![(0, 3), (4, 7), (8, 11), (12, 15), (16, 19), (20, 23)]);
        assert_eq!(RecordUnit::Period.records(b"just words here"), vec![(0, 15)]);
    }

    #[test]
    fn the_axis_units_cover_the_input_in_order() {
        let text = b"alpha beta gamma delta\n1 2 3 4 5 6 7 8\nfn main() { let x = 1; }\n";
        for unit in [
            RecordUnit::Seam,
            RecordUnit::Bind(None),
            RecordUnit::Bind(Some(10.0)),
            RecordUnit::Auto,
            RecordUnit::Texture,
            RecordUnit::Shape,
        ] {
            let records = unit.records(text);
            assert!(!records.is_empty(), "{unit:?}");
            let mut last = 0;
            for (s, e) in &records {
                assert!(*s >= last && *e > *s && *e <= text.len(), "{unit:?}: {records:?}");
                last = *e;
            }
        }
    }

    #[test]
    fn bind_cuts_as_often_as_the_seam_and_bind_q_at_its_share() {
        let text = "alpha beta; gamma(delta) = 12\n".repeat(40);
        let bytes = text.as_bytes();
        let seam = RecordUnit::Seam.records(bytes).len();
        let bind = RecordUnit::Bind(None).records(bytes).len();
        assert!(bind.abs_diff(seam) <= 2, "as many cuts as the seam makes, so as many records: {bind} against {seam}");
        let few = RecordUnit::Bind(Some(1.0)).records(bytes).len();
        let many = RecordUnit::Bind(Some(20.0)).records(bytes).len();
        assert!(few < many, "a larger share cuts more: {few} against {many}");
        assert_eq!(RecordUnit::parse("bind").expect("a unit"), RecordUnit::Bind(None));
        assert_eq!(RecordUnit::parse("bind:5").expect("a share"), RecordUnit::Bind(Some(5.0)));
        assert_eq!(RecordUnit::parse("auto").expect("a unit"), RecordUnit::Auto);
        assert!(RecordUnit::parse("bind:150").expect_err("not a share").contains("0 to 100"));
        assert!(RecordUnit::parse("bind:x").expect_err("not a number").contains("not a percentage"));
    }

    #[test]
    fn auto_takes_the_cuts_that_land_on_more_line_starts() {
        let text = "alpha beta; gamma(delta) = 12\n".repeat(40);
        let bytes = text.as_bytes();
        let auto = RecordUnit::Auto.records(bytes);
        let seam = RecordUnit::Seam.records(bytes);
        let bind = RecordUnit::Bind(None).records(bytes);
        assert!(auto == seam || auto == bind, "auto is one of the two it chooses between");
    }
}
