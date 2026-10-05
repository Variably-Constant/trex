//! What a match is made of, for `--explain`: the kinds of the tokens it
//! spans, the guard each guarded kind passed to be that kind, the value of
//! every axis the pattern read at those tokens, and the rung of the scan
//! ladder that answered. The axes are read through the same analyses the
//! engine builds from, once per input, so an explanation reports the number
//! the predicate compared.

use std::collections::HashSet;
use std::sync::Arc;

use crate::ast::{AnchorKind, Atom, Grain, OtherInput, Pattern, Scope};
use crate::engine::Match;
use crate::orbit::OrbitGroup;
use crate::token::{Token, TokenKind};

/// One reading of an axis at a token of the match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    /// The axis read.
    pub axis: &'static str,
    /// The token's text.
    pub text: String,
    /// The value the predicate compared, as the axis states it.
    pub value: String,
    /// The values [`Self::value`] states, each under the name a template
    /// writes after the dot in `${@axis.piece}`. An axis whose reading is a
    /// single value carries none, because the value is already that piece.
    pub parts: Vec<(&'static str, String)>,
}

/// Every name a `${@...}` reference may write: an axis the explainer reads,
/// or one of the three the explanation carries whole rather than per token,
/// paired with the pieces its reading states.
///
/// A template is checked against this, so `${@entrpoy}` is a parse error
/// rather than a field that renders empty forever. A name that is here but
/// that the pattern never reads renders empty, which is the difference
/// between a misspelling and an axis this pattern had no use for.
pub const EXPLAIN_FIELDS: &[(&str, &[&str])] = &[
    ("kind", &[]),
    ("guard", &[]),
    ("route", &[]),
    ("magnitude", &[]),
    ("baseline", &["scope", "mean", "spread", "count"]),
    ("spectral", &["entropy", "period", "strength", "novelty", "texture"]),
    ("echo", &["count", "occurrence", "period", "rung"]),
    ("order", &[]),
    ("template", &["rarity", "cut", "template", "count", "covered"]),
    ("nesting", &["depth"]),
    ("seam", &["on", "grain", "cut"]),
    ("ambiguous", &["contested", "grain", "cut"]),
    ("gravity", &["strain", "bound", "class", "grain"]),
    ("super", &["role", "depth", "begins"]),
    ("phase", &["phase", "period"]),
    ("join", &["occurs", "other", "rung", "key"]),
    ("field", &["index"]),
];

/// The three names that read the explanation as a whole rather than one
/// axis at each token.
const WHOLE_MATCH_FIELDS: [&str; 3] = ["kind", "guard", "route"];

/// Whether `axis` names something a `${@...}` reference may write, and
/// `piece` a value that axis's reading states.
///
/// # Errors
///
/// The name of no axis, or a piece the named axis does not state, each
/// reported with what it could have been instead.
pub fn check_explain_field(axis: &str, piece: Option<&str>) -> Result<(), String> {
    let Some((_, pieces)) = EXPLAIN_FIELDS.iter().find(|(name, _)| *name == axis) else {
        let names: Vec<&str> = EXPLAIN_FIELDS.iter().map(|(name, _)| *name).collect();
        return Err(format!("${{@{axis}}} reads no axis; the axes are {}", names.join(", ")));
    };
    match piece {
        None => Ok(()),
        Some(p) if pieces.contains(&p) => Ok(()),
        Some(p) if pieces.is_empty() => {
            Err(format!("${{@{axis}}} is one value and takes no .{p} after it"))
        }
        Some(p) => {
            Err(format!(
                "${{@{axis}.{p}}} is not a reading {axis} gives; it gives {}",
                pieces.join(", ")
            ))
        }
    }
}

/// The explanation of one match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explanation {
    /// The kind and the text of each significant token the match spans.
    pub tokens: Vec<(String, String)>,
    /// What each guarded token passed to be its kind.
    pub guards: Vec<String>,
    /// Each axis the pattern reads, at each token of the match.
    pub readings: Vec<Reading>,
    /// The rung of the scan ladder that answered.
    pub route: String,
}

impl Explanation {
    /// What `${@axis}` or `${@axis.piece}` renders over this match: every
    /// reading of that axis across the tokens the match spans, joined with
    /// `, `, or the one at `at` where the reference carries an index.
    ///
    /// Empty where the pattern read no such axis, where the axis has no
    /// reading at any token the match spans, or where `at` is past the last
    /// reading - a template reports what the match has rather than failing
    /// over what it has not.
    #[must_use]
    pub fn field(&self, axis: &str, piece: Option<&str>, at: Option<usize>) -> String {
        let mut values: Vec<String> = if WHOLE_MATCH_FIELDS.contains(&axis) {
            match axis {
                "kind" => self.tokens.iter().map(|(k, _)| k.clone()).collect(),
                "guard" => self.guards.clone(),
                _ => vec![self.route.clone()],
            }
        } else {
            self.readings
                .iter()
                .filter(|r| r.axis == axis)
                .filter_map(|r| match piece {
                    None => Some(r.value.clone()),
                    Some(p) => {
                        r.parts.iter().find(|(name, _)| *name == p).map(|(_, v)| v.clone())
                    }
                })
                .collect()
        };
        match at {
            Some(i) if i < values.len() => values.swap_remove(i),
            Some(_) => String::new(),
            None => values.join(", "),
        }
    }
}

/// The rung that answered a scan, read from the rungs the trace kept:
/// the last one on the scan ladder.
#[must_use]
pub fn route_of(recorded: &[crate::trace::Rung]) -> String {
    recorded
        .iter()
        .rev()
        .find(|r| r.ladder == "scan")
        .map_or_else(|| "no route recorded".to_string(), |r| r.rung.clone())
}

/// The check a token passed to be its kind, for the kinds that check their
/// bytes beyond their shape.
fn guard_of(kind: TokenKind) -> Option<String> {
    let text = match kind {
        TokenKind::CreditCard => {
            "creditcard: 13 to 19 digits in an issuer's grouping, passing the Luhn check"
        }
        TokenKind::Jwt => "jwt: three base64url segments opening with the encoded JSON header",
        TokenKind::Geo => {
            "geo: a latitude within 90 degrees and a longitude within 180, each with four fractional digits, not inside a longer comma-separated run"
        }
        TokenKind::Phone => {
            "phone: an E.164 country code with 7 to 15 digits in all, or a North American number written nationally"
        }
        TokenKind::Base64 => {
            "base64: a length a multiple of four, at least sixteen, with charset diversity"
        }
        TokenKind::Custom(id) => {
            let name = crate::library::name_of(id)?;
            let entry = crate::library::entries().iter().find(|e| e.name == name)?;
            if !entry.is_guarded() {
                return None;
            }
            return Some(format!("{name}: {}", entry.what));
        }
        _ => return None,
    };
    Some(text.to_string())
}

/// The axes one pattern reads, gathered once from its atoms and anchors.
#[derive(Default)]
struct Axes {
    magnitude: bool,
    scopes: Vec<Scope>,
    spectral: bool,
    echo: Vec<OrbitGroup>,
    order: bool,
    rare: Vec<crate::templates::Rarity>,
    nested: bool,
    seam: bool,
    ambiguous: bool,
    gravity: Vec<Grain>,
    supertoken: bool,
    phase: bool,
    periods: Vec<crate::ast::PeriodRef>,
    field: bool,
    joins: Vec<(Arc<OtherInput>, OrbitGroup)>,
}

impl Axes {
    fn gather(pat: &Pattern, into: &mut Axes) {
        match pat {
            Pattern::Atom(a) => Self::atom(a, into),
            Pattern::Within(v, _) => {
                for e in v {
                    Self::atom(&e.atom, into);
                }
            }
            Pattern::Anchor(k) => Self::anchor(k, into),
            Pattern::Empty | Pattern::Guard(..) => {}
            Pattern::Field(_, p) => {
                into.field = true;
                Self::gather(p, into);
            }
            Pattern::Bind(_, _, p)
            | Pattern::Balanced(_, p)
            | Pattern::Assert(p, _, _)
            | Pattern::Star(p, _)
            | Pattern::Plus(p, _)
            | Pattern::Opt(p, _)
            | Pattern::Repeat(p, _, _, _)
            | Pattern::Atomic(p) => Self::gather(p, into),
            Pattern::Concat(v) | Pattern::Alt(v, _) => {
                for p in v {
                    Self::gather(p, into);
                }
            }
        }
    }

    fn atom(a: &Atom, into: &mut Axes) {
        match a {
            Atom::Magnitude(p) | Atom::KindMag(_, p) => {
                into.magnitude = true;
                if let Some(scope) = p.scope()
                    && !into.scopes.contains(scope)
                {
                    into.scopes.push(scope.clone());
                }
            }
            Atom::Spectral(_) => into.spectral = true,
            Atom::RegisterKin(_, g) => {
                if !into.gravity.contains(g) {
                    into.gravity.push(*g);
                }
            }
            Atom::Class(c) => {
                for m in c.members() {
                    Self::atom(m, into);
                }
            }
            _ => {}
        }
    }

    fn anchor(k: &AnchorKind, into: &mut Axes) {
        match k {
            AnchorKind::Novel | AnchorKind::Echoed => Self::rung(OrbitGroup::Identity, into),
            AnchorKind::Echo(_, g) => Self::rung(*g, into),
            AnchorKind::Order(_) => into.order = true,
            AnchorKind::Rare(cut) => {
                if !into.rare.contains(cut) {
                    into.rare.push(*cut);
                }
            }
            AnchorKind::Nested(_) => into.nested = true,
            AnchorKind::Seam(..) => into.seam = true,
            AnchorKind::Ambiguous(..) => into.ambiguous = true,
            AnchorKind::Gravity(_, g, ..) | AnchorKind::Kin(g, _) => {
                if !into.gravity.contains(g) {
                    into.gravity.push(*g);
                }
            }
            AnchorKind::SuperStart | AnchorKind::SuperRole(_) => into.supertoken = true,
            AnchorKind::Phase(_) => into.phase = true,
            AnchorKind::PhaseIn(_, period) => {
                if !into.periods.contains(period) {
                    into.periods.push(*period);
                }
            }
            AnchorKind::Joined { other, group, .. } => {
                if !into.joins.iter().any(|(o, g)| Arc::ptr_eq(o, other) && g == group) {
                    into.joins.push((Arc::clone(other), *group));
                }
            }
            AnchorKind::LineStart
            | AnchorKind::LineEnd
            | AnchorKind::InputStart
            | AnchorKind::InputEnd
            | AnchorKind::Resume
            | AnchorKind::ResetStart => {}
        }
    }

    fn rung(g: OrbitGroup, into: &mut Axes) {
        if !into.echo.contains(&g) {
            into.echo.push(g);
        }
    }
}

/// The analyses one input's explanations read, built once for every axis
/// the pattern names and for no other.
pub struct Explainer<'a> {
    input: &'a [u8],
    toks: Vec<Token>,
    /// The declarations the input was lexed under, with the library kinds
    /// the pattern names, which name a declared token's kind.
    shapes: crate::custom::ShapeSet,
    axes: Axes,
    context: Option<crate::context::ContextField>,
    relation: Option<crate::context::RelationContext>,
    spectral: Option<crate::spectral::SpectralField>,
    echo: Vec<(OrbitGroup, crate::echo::EchoField)>,
    order: Option<Vec<Option<bool>>>,
    templates: Option<crate::templates::Mining>,
    stress: Option<crate::stress::StressField>,
    seam: Vec<crate::engine::GrainCuts>,
    contested: Vec<crate::engine::GrainCuts>,
    gravity: Vec<crate::gravity::Readings>,
    /// The live periods, strongest first, and each token's index among the
    /// significant tokens, where a pattern names a period.
    bands: Option<(Vec<u16>, Vec<u32>)>,
    supers: Option<crate::supertoken::SuperContext>,
    fields: Option<Vec<u32>>,
    joins: Vec<(Arc<OtherInput>, OrbitGroup, HashSet<String>)>,
}

/// The field each token is in, from one, carried forward from the engine's
/// own index of the tokens that open a field.
///
/// Reading it from that index rather than counting commas again is what
/// keeps the number an explanation reports and the number `@k` matched on
/// the same number: there is one rule for where a field begins and both read
/// it. A token before the first field opens is in none.
fn field_at_token(input: &[u8], toks: &[Token]) -> Vec<u32> {
    let starts = crate::engine::field_start_index(input, toks);
    let mut here = 0;
    starts
        .into_iter()
        .map(|n| {
            if n != 0 {
                here = n;
            }
            here
        })
        .collect()
}

impl<'a> Explainer<'a> {
    /// Lex `input` as the scan lexed it and build the analyses the
    /// pattern's axes read.
    #[must_use]
    pub fn new(pattern: &Pattern, input: &'a [u8], shapes: &crate::custom::ShapeSet) -> Self {
        let shapes = shapes.with_library_shapes(&pattern.library_kinds());
        let toks = if shapes.is_empty() {
            crate::lexer::lex(input)
        } else {
            crate::lexer::lex_with_shapes(input, &crate::lexer::blob_runs(input), &shapes, 0)
        };
        let mut axes = Axes::default();
        Axes::gather(pattern, &mut axes);
        let reads_window = axes.scopes.contains(&Scope::Window);
        let reads_relation = axes.scopes.iter().any(|s| *s != Scope::Window) || axes.phase;
        let context = reads_window.then(|| crate::context::analyze(input));
        let relation = reads_relation.then(|| crate::context::relate_bytes(input));
        let spectral = axes.spectral.then(|| crate::spectral::analyze(input));
        let echo = axes
            .echo
            .iter()
            .map(|&g| {
                let cfg = crate::echo::EchoConfig { orbit: g, ..Default::default() };
                (g, crate::echo::analyze_with(&toks, input, &cfg))
            })
            .collect();
        let order = axes.order.then(|| {
            crate::engine::timestamp_order(&toks, input, crate::typed::Clock::current())
        });
        let templates =
            (!axes.rare.is_empty()).then(|| crate::templates::Mining::mine_tokens(&toks, input));
        let stress = axes.nested.then(|| crate::stress::analyze(&toks, input));
        let seam = if axes.seam { crate::engine::seam_lists(pattern, input, &toks) } else { Vec::new() };
        let contested =
            if axes.ambiguous { crate::engine::contested_lists(pattern, input, &toks) } else { Vec::new() };
        let gravity = axes.gravity.iter().map(|&g| crate::gravity::Readings::read(g, input, &toks)).collect();
        let bands = (!axes.periods.is_empty()).then(|| {
            let mut next = 0u32;
            let sig_index = toks
                .iter()
                .map(|t| {
                    if t.is_significant() {
                        next += 1;
                        next - 1
                    } else {
                        u32::MAX
                    }
                })
                .collect();
            (crate::context::live_periods(&toks, input), sig_index)
        });
        let supers = axes.supertoken.then(|| crate::supertoken::SuperContext::build(&toks, input));
        let fields = axes.field.then(|| field_at_token(input, &toks));
        let joins = axes
            .joins
            .iter()
            .map(|(other, group)| {
                let keys: HashSet<String> = other
                    .tokens
                    .iter()
                    .filter(|t| crate::echo::keyed_kind(t.kind))
                    .map(|t| crate::orbit::canonical(&other.bytes[t.span()], *group))
                    .collect();
                (Arc::clone(other), *group, keys)
            })
            .collect();
        Explainer {
            input,
            toks,
            shapes,
            axes,
            context,
            relation,
            spectral,
            echo,
            order,
            templates,
            stress,
            seam,
            contested,
            gravity,
            bands,
            supers,
            fields,
            joins,
        }
    }

    /// The kind and the text of each significant token wholly inside
    /// `at`, which is what an explanation of a match over `at` reads. A
    /// caller with no match of its own - a test line whose text the pattern
    /// did not take - asks over the whole input and gets the same reading.
    #[must_use]
    pub fn tokens(&self, at: std::ops::Range<usize>) -> Vec<(String, String)> {
        self.toks
            .iter()
            .filter(|t| t.is_significant() && t.start() >= at.start && t.end() <= at.end)
            .map(|t| (self.shapes.kind_name(t.kind), String::from_utf8_lossy(&self.input[t.span()]).into_owned()))
            .collect()
    }

    /// The explanation of `m`, with `route` the rung that answered its scan.
    #[must_use]
    pub fn explain(&self, m: &Match, route: &str) -> Explanation {
        let indexes: Vec<usize> = self
            .toks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.is_significant() && t.start() >= m.start && t.end() <= m.end)
            .map(|(i, _)| i)
            .collect();
        let text = |i: usize| String::from_utf8_lossy(&self.input[self.toks[i].span()]).into_owned();
        let tokens = self.tokens(m.start..m.end);
        let guards: Vec<String> = indexes.iter().filter_map(|&i| guard_of(self.toks[i].kind)).collect();
        let mut readings: Vec<Reading> = Vec::new();
        let mut read = |axis: &'static str,
                        i: usize,
                        value: String,
                        parts: Vec<(&'static str, String)>| {
            readings.push(Reading { axis, text: text(i), value, parts });
        };
        for &i in &indexes {
            let t = &self.toks[i];
            if self.axes.magnitude {
                let mag = crate::magnitude::token_magnitude(t.kind, &self.input[t.span()]);
                read("magnitude", i, format!("{mag:.2}"), Vec::new());
                for scope in &self.axes.scopes {
                    if let Some((value, parts)) = self.baseline(scope, i, m) {
                        read("baseline", i, value, parts);
                    }
                }
            }
            if let Some(field) = &self.spectral {
                let f = field.frame_at(t.start());
                let texture = crate::spectral::texture_of(&f).label();
                read(
                    "spectral",
                    i,
                    format!(
                        "entropy {:.2}, period {} (strength {:.2}), novelty {:.2}, texture {texture}",
                        f.entropy, f.period, f.period_strength, f.novelty,
                    ),
                    vec![
                        ("entropy", format!("{:.2}", f.entropy)),
                        ("period", f.period.to_string()),
                        ("strength", format!("{:.2}", f.period_strength)),
                        ("novelty", format!("{:.2}", f.novelty)),
                        ("texture", texture.to_string()),
                    ],
                );
            }
            for (g, field) in &self.echo {
                let (value, parts) = match field.frames.get(i) {
                    Some(fr) if fr.keyed => {
                        let period = if fr.period > 0.0 {
                            format!(", period {:.0} bytes", fr.period)
                        } else {
                            String::new()
                        };
                        (
                            format!("count {}, occurrence {} of {}{period} at the {} rung", fr.count, fr.nth, fr.count, g.label()),
                            vec![
                                ("count", fr.count.to_string()),
                                ("occurrence", fr.nth.to_string()),
                                (
                                    "period",
                                    if fr.period > 0.0 {
                                        format!("{:.0}", fr.period)
                                    } else {
                                        String::new()
                                    },
                                ),
                                ("rung", g.label().to_string()),
                            ],
                        )
                    }
                    _ => ("unkeyed".to_string(), Vec::new()),
                };
                read("echo", i, value, parts);
            }
            if let Some(order) = &self.order
                && t.kind == TokenKind::Timestamp
            {
                let value = match order.get(i).copied().flatten() {
                    Some(true) => "at or after the timestamp before it",
                    Some(false) => "before the timestamp before it",
                    None => "the first timestamp, with nothing to compare with",
                };
                read("order", i, value.to_string(), Vec::new());
            }
            if let Some(mining) = &self.templates {
                for cut in &self.axes.rare {
                    let (value, parts) = match mining.token_template(i) {
                        Some(k) => {
                            let tpl = &mining.templates[k];
                            let rarity = if mining.is_rare(k, *cut) { "rare" } else { "common" };
                            (
                                format!(
                                    "{rarity} under @shape:rare{}: `{}` on {} of {} lines",
                                    cut.label(),
                                    tpl.readable(),
                                    tpl.count(),
                                    mining.covered()
                                ),
                                vec![
                                    ("rarity", rarity.to_string()),
                                    ("cut", cut.label().to_string()),
                                    ("template", tpl.readable()),
                                    ("count", tpl.count().to_string()),
                                    ("covered", mining.covered().to_string()),
                                ],
                            )
                        }
                        None => ("no template".to_string(), Vec::new()),
                    };
                    read("template", i, value, parts);
                }
            }
            if let Some(field) = &self.stress {
                let depth = field.depth_at(t.start());
                read("nesting", i, format!("depth {depth}"), vec![("depth", depth.to_string())]);
            }
            for (g, cut, cuts) in &self.seam {
                let at = cuts.binary_search(&t.start()).is_ok();
                let on = if at { "on" } else { "not on" };
                let written = cut.map(crate::ast::AnchorCut::written).unwrap_or_default();
                let value = match cut {
                    Some(_) => format!("{on} at a {} cut {written}", unit_name(*g)),
                    None => format!("{on} at a {} cut", unit_name(*g)),
                };
                read(
                    "seam",
                    i,
                    value,
                    vec![("on", on.to_string()), ("grain", g.name().to_string()), ("cut", written)],
                );
            }
            for (g, cut, points) in &self.contested {
                let k = points.partition_point(|&x| x < t.start());
                let contested = points.get(k).is_some_and(|&x| x < t.end());
                let state = if contested { "contested" } else { "settled" };
                let written = cut.map(crate::ast::AnchorCut::written).unwrap_or_default();
                let value = match cut {
                    Some(_) => format!("{state} {written} at the {} grain", g.name()),
                    None => format!("{state} at the {} grain", g.name()),
                };
                read(
                    "ambiguous",
                    i,
                    value,
                    vec![("contested", state.to_string()), ("grain", g.name().to_string()), ("cut", written)],
                );
            }
            for r in &self.gravity {
                let grain = r.grain().name();
                let Some(u) = r.unit_at(t.start()) else {
                    read("gravity", i, format!("no unit at the {grain} grain"), Vec::new());
                    continue;
                };
                let strain = r.strain[u];
                let bound = r.unit_starting_at(t.start()).and_then(|b| r.binding[b]);
                let class = match r.class.get(r.type_of(u) as usize).copied() {
                    Some(crate::gravity::UNPLACED) | None => "unplaced".to_string(),
                    Some(c) => c.to_string(),
                };
                let strain_text = match strain {
                    Some(s) => format!("strain {s:.2} bits (percentile {:.0})", r.strain_rank(s)),
                    None => "nothing before it to strain against".to_string(),
                };
                let bound_text = match bound {
                    Some(b) => format!("bound {b:.2} bits per pair (percentile {:.0})", r.binding_rank(b)),
                    None => "no cut before it".to_string(),
                };
                read(
                    "gravity",
                    i,
                    format!("{strain_text}, {bound_text}, class {class} at the {grain} grain"),
                    vec![
                        ("strain", strain.map_or_else(String::new, |s| format!("{s:.2}"))),
                        ("bound", bound.map_or_else(String::new, |b| format!("{b:.2}"))),
                        ("class", class),
                        ("grain", grain.to_string()),
                    ],
                );
            }
            if let Some(supers) = &self.supers {
                let (value, parts) = match supers.unit_of(i) {
                    Some(u) => {
                        let begins = supers.starts_unit(i);
                        (
                            format!(
                                "{} supertoken at depth {}{}",
                                u.role.label(),
                                u.depth,
                                if begins { ", which this token begins" } else { "" }
                            ),
                            vec![
                                ("role", u.role.label().to_string()),
                                ("depth", u.depth.to_string()),
                                ("begins", begins.to_string()),
                            ],
                        )
                    }
                    None => ("in no supertoken".to_string(), Vec::new()),
                };
                read("super", i, value, parts);
            }
            if self.axes.phase
                && let Some(relation) = &self.relation
            {
                let (value, parts) = match relation.phase_of.get(i) {
                    Some(&p) if p != u16::MAX => {
                        (format!("phase {p}"), vec![("phase", p.to_string())])
                    }
                    _ => ("no period".to_string(), Vec::new()),
                };
                read("phase", i, value, parts);
            }
            if let Some((live, sig_index)) = &self.bands {
                for period in &self.axes.periods {
                    let named = match period {
                        crate::ast::PeriodRef::Length(len) => live.contains(len).then_some(*len),
                        crate::ast::PeriodRef::Rank(n) => {
                            usize::from(*n).checked_sub(1).and_then(|k| live.get(k).copied())
                        }
                    };
                    let (value, parts) = match (named, sig_index.get(i)) {
                        (Some(len), Some(&at)) if at != u32::MAX => {
                            let column = at % u32::from(len);
                            (
                                format!("phase {column} of the {len}-token period"),
                                vec![("phase", column.to_string()), ("period", len.to_string())],
                            )
                        }
                        (Some(_) | None, Some(_) | None) => match period {
                            crate::ast::PeriodRef::Length(len) => (format!("no live {len}-token period"), Vec::new()),
                            crate::ast::PeriodRef::Rank(n) => (format!("no live period #{n}"), Vec::new()),
                        },
                    };
                    read("phase", i, value, parts);
                }
            }
            if let Some(fields) = &self.fields {
                let (value, parts) = match fields.get(i).copied() {
                    Some(n) if n != 0 => {
                        (format!("field {n}"), vec![("index", n.to_string())])
                    }
                    _ => ("before the first field".to_string(), Vec::new()),
                };
                read("field", i, value, parts);
            }
            for (other, group, keys) in &self.joins {
                let (value, parts) = if crate::echo::keyed_kind(t.kind) {
                    let key = crate::orbit::canonical(&self.input[t.span()], *group);
                    let occurs = if keys.contains(&key) { "occurs in" } else { "absent from" };
                    // The key appears in the sentence only where the rung made
                    // one that is not the token's own text: under a typed rung
                    // it is the whole answer to why the join held, and under
                    // the identity rung it would repeat the text beside it.
                    let sentence = if key == text(i) {
                        format!("{occurs} {} at the {} rung", other.name, group.label())
                    } else {
                        format!("{occurs} {} at the {} rung, keyed {key}", other.name, group.label())
                    };
                    (
                        sentence,
                        vec![
                            ("occurs", occurs.to_string()),
                            ("other", other.name.clone()),
                            ("rung", group.label().to_string()),
                            ("key", key),
                        ],
                    )
                } else {
                    ("unkeyed".to_string(), Vec::new())
                };
                read("join", i, value, parts);
            }
        }
        Explanation { tokens, guards, readings, route: route.to_string() }
    }

    /// The magnitude baseline a relative predicate at token `i` compares
    /// against, under `scope`, as its mean, spread and size, with each of
    /// those under the name a template writes after the dot.
    fn baseline(
        &self,
        scope: &Scope,
        i: usize,
        m: &Match,
    ) -> Option<(String, Vec<(&'static str, String)>)> {
        let profile = match scope {
            Scope::Window => {
                let field = self.context.as_ref()?;
                let prev = (0..i).rev().find(|&j| self.toks[j].is_significant())?;
                &field.at_token.get(prev)?.magnitude
            }
            Scope::Phase => &self.relation.as_ref()?.at_token.get(i)?.phase.magnitude,
            Scope::Regime => &self.relation.as_ref()?.at_token.get(i)?.regime.magnitude,
            Scope::Echo => &self.relation.as_ref()?.at_token.get(i)?.echoing.magnitude,
            Scope::Enclosing => &self.relation.as_ref()?.at_token.get(i)?.enclosing.magnitude,
            Scope::Key(name) => {
                let k = m.names().iter().position(|n| n == name)?;
                let span = m.captures().get(k)?;
                let key = self.toks.partition_point(|t| t.start() < span.start());
                &self.relation.as_ref()?.value_history.get(key)?.magnitude
            }
        };
        Some((
            format!(
                "{}: mean {:.2}, spread {:.2}, over {}",
                scope_name(scope),
                profile.mean(),
                profile.std_dev(),
                profile.count
            ),
            vec![
                ("scope", scope_name(scope)),
                ("mean", format!("{:.2}", profile.mean())),
                ("spread", format!("{:.2}", profile.std_dev())),
                ("count", profile.count.to_string()),
            ],
        ))
    }
}

/// The unit each reading of a grain is taken at, as a cut names it.
fn unit_name(g: Grain) -> &'static str {
    match g {
        Grain::Byte => "byte",
        Grain::Token => "token",
        Grain::Super => "supertoken",
    }
}

fn scope_name(scope: &Scope) -> String {
    match scope {
        Scope::Window => "window".to_string(),
        Scope::Phase => "phase".to_string(),
        Scope::Regime => "regime".to_string(),
        Scope::Echo => "echo".to_string(),
        Scope::Enclosing => "enclosing".to_string(),
        Scope::Key(name) => format!("key {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declared_or_library_kind_is_named_by_its_declaration() {
        let mut shapes = crate::custom::ShapeSet::new();
        shapes.declare("ticket = `[A-Z]{2,4}-\\d{1,4}`", crate::custom::Precedence::Before).expect("a bounded shape");
        let pattern = crate::parser::parse_with_shapes(r"\{ticket}", &shapes).expect("valid pattern");
        let explainer = Explainer::new(&pattern, b"see AB-12 now", &shapes);
        assert_eq!(explainer.tokens(4..9), [("ticket".to_string(), "AB-12".to_string())]);

        let iban = b"pay GB82 WEST 1234 5698 7654 32";
        let pattern = crate::parse(r"\{iban}").expect("valid pattern");
        let explainer = Explainer::new(&pattern, iban, &crate::custom::ShapeSet::new());
        assert_eq!(explainer.tokens(4..iban.len()), [("iban".to_string(), "GB82 WEST 1234 5698 7654 32".to_string())]);
    }
}
