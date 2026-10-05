//! Log-template mining: the lines of an input grouped by their token-kind
//! silhouette, each group read as one template whose positions are the
//! literal text every line shares or a slot named by the kind that varies
//! there, with the lines each template covers; and the rarity of a line's
//! template, which `@shape:rare` reads at every token of the line.

use std::collections::HashMap;

use crate::token::{Token, TokenKind};

/// One position of a template: the text every line of the group has there,
/// or the kind of the token whose text varies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Literal(String),
    Slot(TokenKind),
}

/// A distinct record shape and the records that have it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    /// The positions, one per significant token of the record.
    pub cells: Vec<Cell>,
    /// The bytes between consecutive tokens in the first record of the group,
    /// which the readable spelling keeps so the template reads as the record
    /// did; one fewer than the cells.
    gaps: Vec<String>,
    /// The records with this template, zero-based, in input order.
    pub records: Vec<usize>,
    /// The kinds the group's records share, in order: the silhouette the
    /// mining grouped them by, which a comparison across two inputs needs
    /// and the cells alone no longer carry once a position is a literal.
    kinds: Vec<TokenKind>,
}

impl Template {
    /// How many records have this template.
    #[must_use]
    pub fn count(&self) -> usize {
        self.records.len()
    }

    /// Whether this template would accept the records of `other`: the same
    /// silhouette, and at every position a cell that admits the other's - the
    /// same literal, or a slot, which admits any text of the kind the
    /// silhouette already fixed.
    ///
    /// The reading `--against` marks a template shared by, and it is
    /// asymmetric on purpose. A log that has seen one name at a position
    /// accepts only that name and one that has seen several accepts them all,
    /// so the question is whether the other log would have found these
    /// records ordinary, not whether the two spellings agree.
    #[must_use]
    pub fn admits(&self, other: &Template) -> bool {
        self.kinds == other.kinds
            && self.cells.iter().zip(&other.cells).all(|(mine, theirs)| match (mine, theirs) {
                (Cell::Slot(_), _) => true,
                (Cell::Literal(a), Cell::Literal(b)) => a == b,
                (Cell::Literal(_), Cell::Slot(_)) => false,
            })
    }

    /// The template as the line reads: each literal as written, each slot as
    /// `<kind>`, the spacing of the group's first line between them.
    #[must_use]
    pub fn readable(&self) -> String {
        let mut out = String::new();
        for (i, cell) in self.cells.iter().enumerate() {
            if i > 0 {
                out.push_str(&self.gaps[i - 1]);
            }
            match cell {
                Cell::Literal(text) => out.push_str(text),
                Cell::Slot(kind) => {
                    out.push('<');
                    out.push_str(kind.name());
                    out.push('>');
                }
            }
        }
        out
    }

    /// The template as a pattern the scanner accepts as written: each
    /// literal quoted, each slot as its kind's atom, one space between. A
    /// declared kind has no atom without its declaration, and a bracket or
    /// an unclassified token has none at all; each is spelled `.`, which
    /// matches the token it stands for.
    #[must_use]
    pub fn pattern(&self) -> String {
        let parts: Vec<String> = self
            .cells
            .iter()
            .map(|cell| match cell {
                Cell::Literal(text) => quote(text),
                Cell::Slot(
                    TokenKind::Custom(_) | TokenKind::Open(_) | TokenKind::Close(_) | TokenKind::Other,
                ) => ".".to_string(),
                Cell::Slot(kind) => match kind.escape() {
                    Some(c) => format!("\\{c}"),
                    None => format!("\\{{{}}}", kind.name()),
                },
            })
            .collect();
        parts.join(" ")
    }
}

/// `text` as a quoted literal of the pattern language.
pub(crate) fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The cut under which a template is rare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rarity {
    /// Fewer lines than the mean template covers: the lines with a template
    /// divided by the distinct templates.
    Mean,
    /// Fewer than this many lines.
    Fewer(u32),
    /// Less than this share of the lines with a template, in hundredths of
    /// a percent.
    Share(u32),
}

impl Rarity {
    /// The cut as written after `@shape:rare<` or to `--cut`: a whole count
    /// of lines (`5`), or a share of the lines with a template as a
    /// percentage to hundredths (`1%`, `0.25%`).
    ///
    /// # Errors
    ///
    /// Anything else, a count of zero, and a share outside `(0%, 100%]`.
    pub fn parse(s: &str) -> Result<Rarity, String> {
        let number = |digits: &str| -> Result<u32, String> {
            match digits.parse::<u32>() {
                Ok(n) => Ok(n),
                Err(e) => Err(format!("{s:?}: {e}")),
            }
        };
        if let Some(share) = s.strip_suffix('%') {
            let (whole, frac) = match share.split_once('.') {
                Some((w, f)) => (w, f),
                None => (share, ""),
            };
            let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
            if (whole.is_empty() && frac.is_empty()) || !digits(whole) || !digits(frac) || frac.len() > 2
            {
                return Err(format!(
                    "{s:?} is not a share; write a percentage to hundredths, as in 1% or 0.25%"
                ));
            }
            let whole = if whole.is_empty() { 0 } else { number(whole)? };
            let frac = match frac.len() {
                0 => 0,
                1 => number(frac)? * 10,
                _ => number(frac)?,
            };
            let hundredths = whole
                .checked_mul(100)
                .and_then(|w| w.checked_add(frac))
                .ok_or_else(|| format!("{s:?} is too large a share"))?;
            if hundredths == 0 || hundredths > 10_000 {
                return Err(format!("{s:?}: a share is above 0% and at most 100%"));
            }
            return Ok(Rarity::Share(hundredths));
        }
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("{s:?} is not a count of lines or a share; write 5 or 1%"));
        }
        let n = number(s)?;
        if n == 0 {
            return Err(
                "a cut of 0 lines holds of nothing; write the count a rare template stays under"
                    .to_string(),
            );
        }
        Ok(Rarity::Fewer(n))
    }

    /// The cut's spelling after `@shape:rare`: nothing for the mean, `<5`,
    /// `<1%`.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Rarity::Mean => String::new(),
            Rarity::Fewer(n) => format!("<{n}"),
            Rarity::Share(h) if h % 100 == 0 => format!("<{}%", h / 100),
            Rarity::Share(h) if h % 10 == 0 => format!("<{}.{}%", h / 100, h % 100 / 10),
            Rarity::Share(h) => format!("<{}.{:02}%", h / 100, h % 100),
        }
    }
}

/// The templates of an input and which record, and which token, has which.
#[derive(Clone, Debug)]
pub struct Mining {
    /// Most records first, then by readable spelling, so the order is the
    /// same on every run.
    pub templates: Vec<Template>,
    /// Per record, the index of its template; `None` for a record with no
    /// significant token.
    pub record_template: Vec<Option<usize>>,
    /// Per token, the index of its record's template plus one; `None` for a
    /// token that is not significant or that no record holds.
    ///
    /// Four bytes per token rather than sixteen, which is what the plus-one
    /// encoding gains: the option costs the zero niche instead of a word, and
    /// this table is one entry per token of the input - 2,650,000 of them on
    /// the comparison corpus, so the width is 42 megabytes against 10. Read it
    /// through [`Mining::token_template`], which takes the one off again.
    token_template: Vec<Option<std::num::NonZeroU32>>,
    /// The records that have a template.
    covered: usize,
}

impl Mining {
    /// Mine `input` over its own lex.
    #[must_use]
    pub fn mine(input: &[u8]) -> Mining {
        Self::mine_tokens(&crate::lexer::lex(input), input)
    }

    /// Mine `input` as `toks` lexes it, over its lines; the token indexes the
    /// result carries are indexes into `toks`.
    #[must_use]
    pub fn mine_tokens(toks: &[Token], input: &[u8]) -> Mining {
        // Timed apart from the mining it feeds: this is a pass over every byte
        // of the input, where everything below walks tokens and records, so the
        // two scale with different things and a single figure over both hides
        // which one a change reached.
        let splitting = crate::trace::phase("templates: splitting the input into records");
        let records = crate::records::RecordUnit::Line.records(input);
        drop(splitting);
        Self::mine_records(toks, input, &records)
    }

    /// Mine `input` as `toks` lexes it, grouping the byte spans `records`
    /// rather than the lines.
    ///
    /// A record's template spans every significant token inside it, so a
    /// record of several lines has one template covering all of them rather
    /// than one per line. The spans are ascending and need not touch: a token
    /// between two records belongs to neither and is grouped by nothing.
    #[must_use]
    pub fn mine_records(toks: &[Token], input: &[u8], records: &[(usize, usize)]) -> Mining {
        // Each record's significant tokens: a token belongs to the record
        // whose span holds its start, and to none where no span does.
        // One array of token indexes and one of where each record's run begins,
        // rather than a vector a record. The cursor only ever advances and the
        // tokens are walked in order, so a record's tokens are already
        // consecutive here - which is what lets the run be recorded as a bound
        // instead of collected into a vector of its own. A vector a record is
        // 200,000 allocations that then grow by push over 2,650,000 of them.
        let assigning = crate::trace::phase("templates: assigning tokens to records");
        let mut flat: Vec<usize> = Vec::with_capacity(toks.len());
        let mut starts: Vec<usize> = Vec::with_capacity(records.len() + 1);
        starts.push(0);
        let mut at = 0;
        for (t, tok) in toks.iter().enumerate() {
            if !tok.is_significant() || records.is_empty() {
                continue;
            }
            while at + 1 < records.len() && tok.start() >= records[at + 1].0 {
                at += 1;
                starts.push(flat.len());
            }
            let (from, to) = records[at];
            if tok.start() >= from && tok.start() < to {
                flat.push(t);
            }
        }
        // The records the walk never reached end where it stopped.
        while starts.len() <= records.len() {
            starts.push(flat.len());
        }

        drop(assigning);

        // Group by silhouette. A group's cells start as its first record's
        // texts, and a position becomes a slot when a later record differs
        // there.
        let grouping = crate::trace::phase("templates: grouping by silhouette");
        let text = |t: usize| String::from_utf8_lossy(&input[toks[t].span()]);
        let mut groups: HashMap<Vec<TokenKind>, usize> = HashMap::new();
        let mut templates: Vec<Template> = Vec::new();
        let mut record_template: Vec<Option<usize>> = vec![None; records.len()];
        // What the grouping does, for the trace. A silhouette is a fresh
        // `Vec<TokenKind>` built and hashed for every record that holds a token,
        // so the key's own cost and the map's are separate questions and the
        // duration answers neither. The split between a silhouette already held
        // and a new one says which half of the match arm below is the common
        // one. Read once a call from registers the loop already carries.
        let watching = crate::trace::keeping();
        let (mut probed, mut fresh) = (0u64, 0u64);
        // One buffer for every record's silhouette, and the map probed by its
        // slice. A silhouette that the map already holds is dropped again the
        // moment it has been looked up, and the counters say that is almost all
        // of them - 199,996 of 200,000 on the comparison corpus - so building a
        // fresh vector per record allocates once a record to answer a question
        // that keeps nothing. The vector is cloned where the answer is a miss,
        // which is what the other four are.
        let mut key: Vec<TokenKind> = Vec::new();
        for r in 0..records.len() {
            let toks_of = &flat[starts[r]..starts[r + 1]];
            if toks_of.is_empty() {
                continue;
            }
            key.clear();
            key.extend(toks_of.iter().map(|&t| toks[t].kind));
            probed += u64::from(watching);
            let index = match groups.get(key.as_slice()) {
                Some(&index) => {
                    for (cell, &t) in templates[index].cells.iter_mut().zip(toks_of) {
                        let differs = matches!(cell, Cell::Literal(s) if *s != text(t));
                        if differs {
                            *cell = Cell::Slot(toks[t].kind);
                        }
                    }
                    index
                }
                None => {
                    fresh += u64::from(watching);
                    let cells = toks_of.iter().map(|&t| Cell::Literal(text(t).into_owned())).collect();
                    let gaps = toks_of
                        .windows(2)
                        .map(|w| {
                            String::from_utf8_lossy(&input[toks[w[0]].end()..toks[w[1]].start()])
                                .into_owned()
                        })
                        .collect();
                    templates.push(Template {
                        cells,
                        gaps,
                        records: Vec::new(),
                        kinds: key.clone(),
                    });
                    groups.insert(key.clone(), templates.len() - 1);
                    templates.len() - 1
                }
            };
            templates[index].records.push(r);
            record_template[r] = Some(index);
        }
        crate::trace::counted("templates: records mined", {
            u64::try_from(records.len()).expect("a record count within the counter's width")
        });
        crate::trace::counted("templates: silhouettes probed", probed);
        crate::trace::counted("templates: silhouettes already held", probed - fresh);
        crate::trace::counted("templates: silhouettes newly made", fresh);
        drop(grouping);

        // Most records first, then by spelling; the record and token tables
        // follow the templates to their new places.
        //
        // Timed because neither phase above covers it and it is the only other
        // act in the call. Its work is the template count, where the two above
        // are the token count and the record count, so a corpus with many
        // distinct silhouettes prices it differently from one with few. The
        // spelling comparison renders both sides, which is what the count of
        // templates below is read against.
        let ordering = crate::trace::phase("templates: ordering and laying out the tables");
        crate::trace::counted("templates: templates ordered", {
            u64::try_from(templates.len()).expect("a template count within the counter's width")
        });
        // The spelling each template is ordered by, rendered once. A template's
        // spelling does not change while the sort runs, and `readable` builds a
        // fresh string from every cell and gap, so rendering inside the
        // comparator would build one on both sides of every tie: at the template
        // counts real corpora carry, tens of thousands, that is a quarter of a
        // million renders to settle an order that as many of them describe.
        let spelling: Vec<String> = templates.iter().map(Template::readable).collect();
        let mut order: Vec<usize> = (0..templates.len()).collect();
        order.sort_by(|&a, &b| {
            templates[b].count().cmp(&templates[a].count()).then_with(|| spelling[a].cmp(&spelling[b]))
        });
        let mut rank = vec![0usize; templates.len()];
        for (new, &old) in order.iter().enumerate() {
            rank[old] = new;
        }
        let mut taken: Vec<Option<Template>> = templates.into_iter().map(Some).collect();
        let templates: Vec<Template> = order
            .iter()
            .map(|&old| taken[old].take().expect("each template moves exactly once"))
            .collect();
        let record_template: Vec<Option<usize>> =
            record_template.into_iter().map(|t| t.map(|old| rank[old])).collect();
        let mut token_template: Vec<Option<std::num::NonZeroU32>> = vec![None; toks.len()];
        for r in 0..records.len() {
            // Encoded once a record rather than once a token: every token of a
            // record carries its record's template.
            let held = record_template[r].map(|k| {
                let plus_one =
                    u32::try_from(k + 1).expect("a template index within the table's width");
                std::num::NonZeroU32::new(plus_one).expect("an index plus one is never zero")
            });
            for &t in &flat[starts[r]..starts[r + 1]] {
                token_template[t] = held;
            }
        }
        let covered = templates.iter().map(Template::count).sum();
        drop(ordering);
        Mining { templates, record_template, token_template, covered }
    }

    /// The records that have a template.
    #[must_use]
    pub fn covered(&self) -> usize {
        self.covered
    }

    /// Whether the template at `index` is rare under `cut`.
    #[must_use]
    pub fn is_rare(&self, index: usize, cut: Rarity) -> bool {
        let count = self.templates[index].count() as u64;
        let covered = self.covered as u64;
        match cut {
            // count < covered / templates, kept exact by cross-multiplying.
            Rarity::Mean => count * (self.templates.len() as u64) < covered,
            Rarity::Fewer(n) => count < u64::from(n),
            // count / covered < hundredths / 10,000, likewise.
            Rarity::Share(hundredths) => count * 10_000 < covered * u64::from(hundredths),
        }
    }

    /// The index of the template of token `t`'s line, or `None`
    /// for a token that is not significant.
    #[must_use]
    pub fn token_template(&self, t: usize) -> Option<usize> {
        self.token_template.get(t).copied().flatten().map(|k| k.get() as usize - 1)
    }

    /// Whether the record holding token `t` has a rare template.
    #[must_use]
    pub fn token_is_rare(&self, t: usize, cut: Rarity) -> bool {
        self.token_template(t).is_some_and(|i| self.is_rare(i, cut))
    }

    /// The mark each template of this mining carries against `other`: whether
    /// some template of `other` would accept its records.
    ///
    /// One pass over the other's templates per template of this one. Both
    /// sides are small - a log of a million lines mines to a few dozen shapes
    /// - so the pair is cheaper than the mining that produced either.
    #[must_use]
    pub fn shared_with(&self, other: &Mining) -> Vec<bool> {
        self.templates
            .iter()
            .map(|mine| other.templates.iter().any(|theirs| theirs.admits(mine)))
            .collect()
    }
}

/// The token-kind silhouette of one span of `input`: the kinds of the
/// significant tokens it holds, in order.
///
/// The same reading a template groups lines by, taken over a span rather than
/// a line, so two spans share a template exactly when this is equal. What it
/// deliberately drops is the text: `user bob logged in` and `user amy logged
/// in` are one silhouette, which is what makes it the unit a reviewer answers
/// for rather than a synonym for the match itself.
///
/// A span is lexed on its own here. A recognizer that would have run across
/// either end reads a different token, so a caller that has the whole input's
/// tokens should compare those instead; this is for a caller holding a span
/// and nothing else.
#[must_use]
pub fn silhouette(input: &[u8], span: std::ops::Range<usize>) -> Vec<TokenKind> {
    let Some(bytes) = input.get(span) else { return Vec::new() };
    crate::lexer::lex(bytes).iter().filter(|t| t.is_significant()).map(|t| t.kind).collect()
}

/// A silhouette as a reviewer reads it: the kind names in order.
#[must_use]
pub fn silhouette_name(shape: &[TokenKind]) -> String {
    if shape.is_empty() {
        return "no token".to_string();
    }
    shape.iter().map(|k| k.name()).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mine one input and answer with the template of its first record.
    fn one(input: &str) -> Mining {
        Mining::mine(input.as_bytes())
    }

    /// Whether the templates of `theirs` would accept the records of `mine`,
    /// template by template in `mine`'s order.
    fn marks(mine: &str, theirs: &str) -> Vec<bool> {
        one(mine).shared_with(&one(theirs))
    }

    #[test]
    fn a_template_is_shared_when_the_other_would_accept_its_records() {
        // A slot admits any text of its kind, so a log that has seen several
        // names at a position accepts one that has seen fewer.
        assert_eq!(marks("user bob logged in\n", "user amy logged in\nuser eve logged in\n"), vec![true]);
        // And not the other way: a log that has seen one name accepts only it.
        assert_eq!(marks("user amy logged in\nuser eve logged in\n", "user bob logged in\n"), vec![false]);
        // The same literal is accepted as itself.
        assert_eq!(marks("disk sda ok\n", "disk sda ok\n"), vec![true]);
        assert_eq!(marks("disk sda ok\n", "disk sdb ok\n"), vec![false]);
        // A different silhouette is never shared, however alike it reads.
        assert_eq!(marks("disk sda ok\n", "disk sda 200\n"), vec![false]);
        assert_eq!(marks("a b\n", "a b c\n"), vec![false]);
        // Nothing to compare against is nothing shared.
        assert_eq!(marks("a b\n", ""), vec![false]);
    }

    #[test]
    fn a_record_of_several_lines_is_one_template_spanning_them() {
        let input = b"job alpha\nstatus ok\n\njob bravo\nstatus ok\n";
        let toks = crate::lexer::lex(input);
        // By line the four lines are one shape and it says nothing: every one
        // of them is two words. The record is what carries the structure.
        let by_line = Mining::mine_tokens(&toks, input);
        assert_eq!(by_line.templates.len(), 1);
        assert_eq!(by_line.templates[0].readable(), "<word> <word>");
        let paragraphs = crate::records::RecordUnit::Paragraph.records(input);
        let by_paragraph = Mining::mine_records(&toks, input, &paragraphs);
        assert_eq!(by_paragraph.templates.len(), 1);
        assert_eq!(by_paragraph.templates[0].count(), 2);
        assert_eq!(by_paragraph.templates[0].records, vec![0, 1]);
        assert_eq!(by_paragraph.templates[0].readable(), "job <word>\nstatus ok");
        // A token in no record is grouped by nothing, so a span that skips
        // one leaves it out rather than folding it into a neighbor.
        let first_only = Mining::mine_records(&toks, input, &paragraphs[..1]);
        assert_eq!(first_only.covered(), 1);
        assert_eq!(first_only.templates[0].readable(), "job alpha\nstatus ok");
    }
}
