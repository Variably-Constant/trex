//! Matches grouped by a rendered key: how many each key holds, and the typed
//! aggregates of the registers asked for. `trex count-by`, `top` and `uniq`,
//! Group-TrexMatch and Python's `group_by` all read their tables from here.
//!
//! A column is checked against the pattern before any input is read: the
//! register must be one the pattern binds to a single typed kind, the kind
//! must admit the aggregate (a sum takes numeric kinds, an order takes those
//! and timestamps, versions and addresses), and a linear percentile takes
//! numeric kinds only. A match that does not bind a column's register adds no
//! value to it, which is not the same as adding a zero.

use std::collections::HashMap;

use crate::token::TokenKind;
use crate::typed::{Agg, Aggregable, Collected, Percentile, aggregable, value_of};

/// One aggregate column: what it computes, the register it reads, and the
/// kind that register binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub agg: Agg,
    pub register: String,
    pub kind: TokenKind,
}

/// Why a column cannot be computed, found before any input is read. Each
/// surface words it in its own terms: a flag on the command line, a
/// parameter in PowerShell, a keyword in Python.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnError {
    /// The spec names more than one value or a slice of text, as `${a}/${b}`
    /// or `${u:host}`, rather than one register.
    NotOneRegister { agg: Agg, spec: String },
    /// The pattern binds no register of that name.
    NotBound { agg: Agg, register: String },
    /// The register binds a run, a repetition or an alternation, whose kind
    /// varies between matches.
    NoSingleKind { agg: Agg, register: String },
    /// The kind cannot carry the aggregate: no arithmetic on it, or no order.
    NotAdmitted { agg: Agg, register: String, kind: TokenKind },
    /// A linear percentile over a kind that cannot be interpolated.
    LinearOnOrdered { register: String, kind: TokenKind },
}

/// The single register `spec` names, written as a report reference
/// (`${size}`) or bare (`size`), or `None` where it names anything else.
/// Spaces around the spec and inside the braces are not part of the name.
#[must_use]
pub fn one_register(spec: &str) -> Option<String> {
    let spec = spec.trim();
    let body = match spec.strip_prefix("${") {
        Some(rest) => rest.strip_suffix('}')?,
        None => spec,
    };
    let name = body.trim();
    if name.is_empty() || name.contains(['$', '{', '}', ':', '|']) {
        return None;
    }
    Some(name.to_string())
}

/// The columns `asked` names, each an aggregate and the spec of the register
/// it reads, checked against `capture_kinds`, the kind each register of the
/// pattern binds, under the percentile method `pct`.
///
/// # Errors
///
/// The first column that cannot be computed, with why.
pub fn columns(
    asked: &[(Agg, String)],
    capture_kinds: &[(String, Option<TokenKind>)],
    pct: Percentile,
) -> Result<Vec<Column>, ColumnError> {
    let mut out = Vec::with_capacity(asked.len());
    for (agg, spec) in asked {
        let agg = *agg;
        let Some(register) = one_register(spec) else {
            return Err(ColumnError::NotOneRegister { agg, spec: spec.clone() });
        };
        let Some((_, bound)) = capture_kinds.iter().find(|(n, _)| *n == register) else {
            return Err(ColumnError::NotBound { agg, register });
        };
        let Some(kind) = *bound else {
            return Err(ColumnError::NoSingleKind { agg, register });
        };
        if !agg.admits(kind) {
            return Err(ColumnError::NotAdmitted { agg, register, kind });
        }
        if pct == Percentile::Linear && matches!(agg, Agg::Pct(_)) && aggregable(kind) != Aggregable::Numeric {
            return Err(ColumnError::LinearOnOrdered { register, kind });
        }
        out.push(Column { agg, register, kind });
    }
    Ok(out)
}

/// How a table's rows are ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Most frequent first, and by key among equal counts, so the order is
    /// the same on every run.
    Count,
    /// By key.
    Key,
}

/// One row of a table: a key, how many matches it holds, and for each
/// column the values its matches bound, `None` where the key's matches
/// bound none of them.
#[derive(Debug)]
pub struct Row<'a> {
    pub key: &'a str,
    pub count: u64,
    pub values: Vec<Option<&'a Collected>>,
}

/// Matches gathered under their keys, with the values each column reads.
#[derive(Debug, Default)]
pub struct Table {
    columns: Vec<Column>,
    counts: HashMap<String, u64>,
    gathered: HashMap<String, Vec<Collected>>,
    matched: u64,
}

impl Table {
    /// An empty table computing `columns`.
    #[must_use]
    pub fn new(columns: Vec<Column>) -> Self {
        Table { columns, ..Table::default() }
    }

    /// The columns this table computes.
    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Count `m`, found in `input`, under `key`, and keep each column's
    /// value where the match binds its register.
    pub fn add(&mut self, key: String, m: &crate::Match, input: &[u8]) {
        self.matched += 1;
        *self.counts.entry(key.clone()).or_insert(0) += 1;
        if self.columns.is_empty() {
            return;
        }
        let slot = self.gathered.entry(key).or_insert_with(|| vec![Collected::default(); self.columns.len()]);
        for (c, col) in self.columns.iter().enumerate() {
            let Some(text) = m.group(&col.register, input) else { continue };
            let Some(v) = value_of(col.kind, &String::from_utf8_lossy(text)) else { continue };
            slot[c].push(v);
        }
    }

    /// How many matches were counted.
    #[must_use]
    pub fn matched(&self) -> u64 {
        self.matched
    }

    /// How many distinct keys there are.
    #[must_use]
    pub fn keys(&self) -> usize {
        self.counts.len()
    }

    /// Every row, in `order`.
    #[must_use]
    pub fn rows(&self, order: Order) -> Vec<Row<'_>> {
        let mut keyed: Vec<(&str, u64)> = self.counts.iter().map(|(k, n)| (k.as_str(), *n)).collect();
        match order {
            Order::Count => keyed.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0))),
            Order::Key => keyed.sort_by(|a, b| a.0.cmp(b.0)),
        }
        keyed
            .into_iter()
            .map(|(key, count)| {
                let held = self.gathered.get(key);
                let values = (0..self.columns.len())
                    .map(|c| held.map(|v| &v[c]).filter(|v| !v.is_empty()))
                    .collect();
                Row { key, count, values }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed::{QuotientForm, ValueStyle};

    fn kinds(src: &str) -> Vec<(String, Option<TokenKind>)> {
        crate::parse(src).expect("the test pattern parses").capture_kinds()
    }

    /// Every match of `src` in `input`, with its registers resolved.
    fn found(src: &str, input: &[u8]) -> Vec<crate::Match> {
        let p = crate::parse(src).expect("the test pattern parses");
        crate::captures(&p, input, &crate::scan(&p, input))
    }

    fn column(agg: Agg, register: &str, kind: TokenKind) -> Column {
        Column { agg, register: register.to_string(), kind }
    }

    #[test]
    fn a_spec_names_one_register_written_bare_or_as_a_reference() {
        for spec in ["size", "${size}", " ${size} ", "${ size }", " size "] {
            assert_eq!(one_register(spec).as_deref(), Some("size"), "{spec:?}");
        }
        for spec in ["", "${}", "${size", "$size", "${a}/${b}", "${u:host}", "a|b"] {
            assert_eq!(one_register(spec), None, "{spec:?}");
        }
    }

    #[test]
    fn a_column_is_checked_against_the_kind_its_register_binds() {
        let k = kinds(r"\W:host \N:n \T:at \V:ver (\W \W):pair");
        let ask = |agg: Agg, spec: &str, pct: Percentile| columns(&[(agg, spec.to_string())], &k, pct);
        let near = Percentile::Nearest;
        assert_eq!(ask(Agg::Sum, "${n}", near), Ok(vec![column(Agg::Sum, "n", TokenKind::Number)]));
        assert_eq!(ask(Agg::Max, "at", near), Ok(vec![column(Agg::Max, "at", TokenKind::Timestamp)]));
        assert_eq!(ask(Agg::Pct(50), "ver", near), Ok(vec![column(Agg::Pct(50), "ver", TokenKind::Version)]));
        assert_eq!(
            ask(Agg::Pct(50), "n", Percentile::Linear),
            Ok(vec![column(Agg::Pct(50), "n", TokenKind::Number)])
        );
        assert_eq!(
            ask(Agg::Sum, "${n}/${n}", near),
            Err(ColumnError::NotOneRegister { agg: Agg::Sum, spec: "${n}/${n}".to_string() })
        );
        assert_eq!(
            ask(Agg::Sum, "size", near),
            Err(ColumnError::NotBound { agg: Agg::Sum, register: "size".to_string() })
        );
        assert_eq!(
            ask(Agg::Min, "pair", near),
            Err(ColumnError::NoSingleKind { agg: Agg::Min, register: "pair".to_string() })
        );
        assert_eq!(
            ask(Agg::Sum, "at", near),
            Err(ColumnError::NotAdmitted { agg: Agg::Sum, register: "at".to_string(), kind: TokenKind::Timestamp })
        );
        assert_eq!(
            ask(Agg::Max, "host", near),
            Err(ColumnError::NotAdmitted { agg: Agg::Max, register: "host".to_string(), kind: TokenKind::Word })
        );
        assert_eq!(
            ask(Agg::Pct(50), "ver", Percentile::Linear),
            Err(ColumnError::LinearOnOrdered { register: "ver".to_string(), kind: TokenKind::Version })
        );
    }

    #[test]
    fn a_table_counts_each_key_and_keeps_only_the_values_bound() {
        let input = b"a 3 b 5 a 4 c - a 2";
        let src = r"\W:k (\N:n)?";
        let sum_n = columns(&[(Agg::Sum, "n".to_string())], &kinds(src), Percentile::Nearest).expect("n sums");
        let mut table = Table::new(sum_n);
        for m in found(src, input) {
            let key = String::from_utf8_lossy(m.group("k", input).expect("every match binds k")).into_owned();
            table.add(key, &m, input);
        }
        assert_eq!(table.matched(), 5);
        assert_eq!(table.keys(), 3);

        let by_count: Vec<(&str, u64)> = table.rows(Order::Count).iter().map(|r| (r.key, r.count)).collect();
        assert_eq!(by_count, [("a", 3), ("b", 1), ("c", 1)]);
        let by_key: Vec<&str> = table.rows(Order::Key).iter().map(|r| r.key).collect();
        assert_eq!(by_key, ["a", "b", "c"]);

        let rows = table.rows(Order::Key);
        let sum = |row: &Row<'_>| {
            let held = row.values[0]?;
            held.report(
                Agg::Sum,
                TokenKind::Number,
                ValueStyle::new(),
                QuotientForm::Repetend,
                Percentile::Nearest,
                crate::Clock::current(),
            )
        };
        assert_eq!(sum(&rows[0]).as_deref(), Some("9"));
        assert_eq!(sum(&rows[1]).as_deref(), Some("5"));
        assert!(rows[2].values[0].is_none(), "c's match bound no n, which is not a zero");
    }

    #[test]
    fn a_table_without_columns_only_counts() {
        let input = b"x 1 y 2 x 3";
        let mut table = Table::new(Vec::new());
        for m in found(r"\W:k \N", input) {
            let key = String::from_utf8_lossy(m.group("k", input).expect("every match binds k")).into_owned();
            table.add(key, &m, input);
        }
        let rows = table.rows(Order::Count);
        let counted: Vec<(&str, u64)> = rows.iter().map(|r| (r.key, r.count)).collect();
        assert_eq!(counted, [("x", 2), ("y", 1)]);
        assert!(rows.iter().all(|r| r.values.is_empty()));
    }
}
