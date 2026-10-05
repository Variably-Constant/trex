//! The axes of recurrence: what an input repeats, token by token under a
//! symmetry, and template by template in the shape of its records.

use std::collections::BTreeMap;

use pwrs::prelude::*;

use crate::atoms::{TrexLibrary, atoms_for};
use crate::axes::{Input, char_start, each_input, frames_at, lexed, text_of, utf16_offsets};
use crate::common::arg_err;

/// A symmetry group named as the trex command names one: `identity`, `case`,
/// `notation`, `shape`, `e8`, `ip`, `url`, `time`, `path`, `fold`,
/// `numeric`, or a typed relation such as `subnet/24`, `domain` or `day`.
fn group_of(name: &str) -> PsResult<trex::orbit::OrbitGroup> {
    match trex::orbit::OrbitGroup::parse(name.trim()) {
        Some(g) => Ok(g),
        None => Err(arg_err(
            "TrexOrbitGroup",
            format!(
                "{name:?} names no group: use identity, case, notation, shape, e8, ip, url, time, path, fold, \
                 numeric, or a typed relation such as subnet/24, domain or day"
            ),
        )),
    }
}

/// One token's echo reading.
#[psclass(name = "Trex.EchoFrame", show = "{Text} at {Offset}")]
#[derive(Clone, Default)]
pub struct TrexEchoFrame {
    /// Where the token starts, in UTF-16 code units.
    pub offset: i64,
    /// The token's text.
    pub text: String,
    /// How many times the token's key occurs in the input.
    pub count: i64,
    /// Which occurrence of its key this is, counting from 1.
    pub nth: i64,
    /// Where the key's previous occurrence starts, in UTF-16 code units;
    /// `$null` at its first.
    pub previous: PsObject,
}

/// A key that recurs: a token's text, or the structure of a run of tokens.
#[psclass(name = "Trex.Echo", show = "{Key} ({Count})")]
#[derive(Clone, Default)]
pub struct TrexEcho {
    /// The key: the text of its first occurrence, or a structure's role and
    /// the kinds of its tokens.
    pub key: String,
    /// How many times it occurs.
    pub count: i64,
    /// The mean distance between occurrences in bytes, where the distances
    /// are regular; `$null` where they are not.
    pub period: PsObject,
    /// Where its first occurrence starts, in UTF-16 code units.
    pub offset: i64,
}

/// The echo axis over one input: which tokens recur, how often, and how
/// regularly.
#[psclass(name = "Trex.EchoReport")]
#[derive(Clone, Default)]
pub struct TrexEchoReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// The group the keys were read under.
    pub group: String,
    /// How many tokens the lexer read.
    pub tokens: i64,
    /// The tokens that take part: words, numbers, quoted runs and typed
    /// literals.
    pub keyed: i64,
    /// How many distinct keys they hold.
    pub distinct: i64,
    /// The keyed tokens that are the first occurrence of their key.
    pub novel: i64,
    /// The keyed tokens whose key occurs more than once.
    pub echoed: i64,
    /// The share of keyed tokens that are first occurrences, from 0 to 1.
    pub novelty: f32,
    /// The share of keyed tokens whose key recurs, from 0 to 1.
    pub echo_rate: f32,
    /// Every key that recurs, most occurrences first.
    pub echoes: Vec<TrexEcho>,
    /// The structures that recur among runs of tokens, most occurrences
    /// first, with -Structure.
    pub structures: Vec<TrexEcho>,
    /// Every keyed token's frame, with -Detail.
    pub frames: Vec<TrexEchoFrame>,
}

/// Reads the echo axis: which words, numbers and literals recur, how many
/// times, and whether they recur at a regular distance.
///
/// -Group reads the keys under a symmetry, so under `case` `Error` and
/// `ERROR` are one key, and under `subnet/24` so are all the addresses of
/// one network. -Structure adds the structures that recur among supertokens, so
/// `alpha: one` and `bravo: two` are one structure, a key beside a value. A
/// key recurring three or more times has a period where the spread of its
/// distances is at most -MaxPeriodCv of their mean, 0.3 by default.
///
/// # Examples
/// Measure-TrexEcho 'GET /a GET /b POST /a GET /c'
/// Measure-TrexEcho -Path ./app.log -Group case | Select-Object -ExpandProperty Echoes -First 5
/// Measure-TrexEcho -Path ./config.ini -Structure | Select-Object -ExpandProperty Structures
/// Measure-TrexEcho -Path ./app.log -MaxPeriodCv 0.1 | Where-Object Period
#[cmdlet(verb = "Measure", noun = "TrexEcho", alias = "Measure-TxEcho", default_parameter_set = "Text", output = ["Trex.EchoReport"])]
#[derive(Default)]
pub struct MeasureTrexEcho {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The symmetry the keys are read under: `identity`, exact text, when
    /// absent.
    #[param]
    pub group: Option<String>,
    /// Adds the structures that recur among supertokens.
    #[param]
    pub structure: bool,
    /// Adds every keyed token's frame to the report.
    #[param]
    pub detail: bool,
    /// How widely a key's distances may spread, as a fraction of their mean,
    /// for it to have a period: 0.3 when absent.
    #[param]
    pub max_period_cv: Option<f32>,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
    orbit: Option<(String, trex::orbit::OrbitGroup)>,
}

impl MeasureTrexEcho {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexEchoReport> {
        let Some((group_name, group)) = &self.orbit else {
            return Err(arg_err("TrexOrbitGroup", "the group was not read"));
        };
        let bytes = input.text.as_bytes();
        let toks = lexed(bytes, shapes, false);
        let defaults = trex::echo::EchoConfig::default();
        let cfg = trex::echo::EchoConfig {
            orbit: *group,
            max_period_cv: crate::axes::cut("MaxPeriodCv", self.max_period_cv, defaults.max_period_cv)?,
            ..defaults
        };
        let f = trex::echo::analyze_with(&toks, bytes, &cfg);
        let period = |p: f32| if p > 0.0 { p.into_ps() } else { Ok(PsObject::default()) };

        // A recurring key's first occurrence carries the key's count and period.
        let mut firsts: Vec<&trex::echo::EchoFrame> =
            f.frames.iter().filter(|r| r.echoed() && r.back_lag.is_none()).collect();
        firsts.sort_by(|a, b| b.count.cmp(&a.count).then(a.start.cmp(&b.start)));
        let starts: Vec<usize> = firsts.iter().map(|r| r.start as usize).collect();
        let mut echoes = Vec::with_capacity(firsts.len());
        for (r, offset) in firsts.iter().zip(utf16_offsets(bytes, &starts)) {
            echoes.push(TrexEcho {
                key: text_of(bytes, (r.start as usize, r.end as usize)),
                count: i64::from(r.count),
                period: period(r.period)?,
                offset,
            });
        }

        let mut structures = Vec::new();
        if self.structure {
            let rhymes = trex::echo::analyze_super_tokens(&toks, bytes, &cfg);
            let firsts: Vec<usize> = rhymes.iter().map(|r| r.first).collect();
            for (r, offset) in rhymes.iter().zip(utf16_offsets(bytes, &firsts)) {
                structures.push(TrexEcho {
                    key: r.key.clone(),
                    count: i64::from(r.count),
                    period: match r.period {
                        Some(p) => p.into_ps()?,
                        None => PsObject::default(),
                    },
                    offset,
                });
            }
        }

        let mut frames = Vec::new();
        if self.detail {
            let keyed: Vec<&trex::echo::EchoFrame> = f.frames.iter().filter(|r| r.keyed).collect();
            let starts: Vec<usize> = keyed.iter().map(|r| r.start as usize).collect();
            let offsets = utf16_offsets(bytes, &starts);
            for (r, &offset) in keyed.iter().zip(&offsets) {
                // The previous occurrence is an earlier keyed token, so its
                // offset is among those just found.
                let previous = match r.back_lag {
                    Some(lag) => {
                        let at = r.start as usize - lag.get() as usize;
                        match starts.binary_search(&at) {
                            Ok(k) => offsets[k].into_ps()?,
                            Err(_) => {
                                return Err(PsError::new(
                                    ErrorCategory::InvalidResult,
                                    "TrexEchoLag",
                                    format!("trex placed the occurrence before byte {} at byte {at}, which begins no keyed token", r.start),
                                ));
                            }
                        }
                    }
                    None => PsObject::default(),
                };
                frames.push(TrexEchoFrame {
                    offset,
                    text: text_of(bytes, (r.start as usize, r.end as usize)),
                    count: i64::from(r.count),
                    nth: i64::from(r.nth),
                    previous,
                });
            }
        }

        Ok(TrexEchoReport {
            length: input.length(),
            path: input.path,
            group: group_name.clone(),
            tokens: toks.len() as i64,
            keyed: f.keyed as i64,
            distinct: f.distinct as i64,
            novel: f.novel as i64,
            echoed: f.echoed as i64,
            novelty: f.novelty(),
            echo_rate: f.echo_rate(),
            echoes,
            structures,
            frames,
        })
    }
}

impl Cmdlet for MeasureTrexEcho {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let name = match &self.group {
            Some(g) => g.trim().to_string(),
            None => "identity".to_string(),
        };
        let group = group_of(&name)?;
        self.orbit = Some((name, group));
        self.shapes = Some(atoms_for(ps, &self.library)?);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shapes) = &self.shapes else {
            return Err(arg_err("TrexNoAtoms", "the atoms were not read"));
        };
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| {
            ps.write(self.report(input, shapes)?)
        })
    }
}

/// One token under a symmetry: its text and the orbit it belongs to.
#[psclass(name = "Trex.OrbitToken", show = "{Text} at {Offset}")]
#[derive(Clone, Default)]
pub struct TrexOrbitToken {
    /// Where the token starts, in UTF-16 code units.
    pub offset: i64,
    /// How many UTF-16 code units it spans.
    pub length: i64,
    /// The token's text.
    pub text: String,
    /// The orbit's representative: the form every token of the orbit takes
    /// under the group.
    pub orbit: String,
}

/// One orbit and the distinct texts that fold onto it.
#[psclass(name = "Trex.OrbitClass", show = "{Orbit}")]
#[derive(Clone, Default)]
pub struct TrexOrbitClass {
    /// The orbit's representative.
    pub orbit: String,
    /// The distinct texts in the input that fold onto it, sorted.
    pub forms: Vec<String>,
}

/// A run of an input: where it starts, how long it is, and its text.
#[psclass(name = "Trex.Segment", show = "{Text}")]
#[derive(Clone, Default)]
pub struct TrexSegment {
    /// Where the run starts, in UTF-16 code units.
    pub offset: i64,
    /// How many UTF-16 code units it spans.
    pub length: i64,
    /// The run's text.
    pub text: String,
}

/// The segments of `bytes` between cuts, in order: from the start to the
/// first cut, between each pair, and from the last to the end.
pub(crate) fn segments_between(bytes: &[u8], cuts: &[usize]) -> Vec<TrexSegment> {
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(cuts.len() + 1);
    let mut from = 0usize;
    for &c in cuts {
        spans.push((from, c));
        from = c;
    }
    spans.push((from, bytes.len()));
    segments_of(bytes, &spans)
}

/// The segments at byte spans given in order, each end inside a character
/// moved to the character's start.
pub(crate) fn segments_of(bytes: &[u8], spans: &[(usize, usize)]) -> Vec<TrexSegment> {
    let ends: Vec<usize> = spans.iter().flat_map(|&(s, e)| [s, e]).collect();
    let offsets = utf16_offsets(bytes, &ends);
    spans
        .iter()
        .enumerate()
        .map(|(k, &(s, e))| TrexSegment {
            offset: offsets[2 * k],
            length: offsets[2 * k + 1] - offsets[2 * k],
            text: text_of(bytes, (char_start(bytes, s), char_start(bytes, e))),
        })
        .collect()
}

/// The orbit axis over one input: its tokens read under a symmetry, and the
/// distinct texts each orbit folds together.
#[psclass(name = "Trex.OrbitReport")]
#[derive(Clone, Default)]
pub struct TrexOrbitReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// The group the tokens were read under.
    pub group: String,
    /// How many tokens were read, whitespace left out.
    pub tokens: i64,
    /// How many distinct texts the tokens hold.
    pub forms: i64,
    /// How many distinct orbits they fold onto.
    pub orbits: i64,
    /// Every orbit with the texts that fold onto it, by representative.
    pub classes: Vec<TrexOrbitClass>,
    /// The tokens in the same orbit as -SameAs, with -SameAs.
    pub matches: Vec<TrexOrbitToken>,
    /// The runs of one kind of character, letters, digits, whitespace or
    /// the rest, with -Detail.
    pub segments: Vec<TrexSegment>,
    /// Every token with its orbit, with -Detail.
    pub frames: Vec<TrexOrbitToken>,
}

/// Reads the orbit axis: each token as the representative of its orbit
/// under a symmetry, and how far the symmetry folds the input's vocabulary.
///
/// Under `shape`, the default, `cat`, `dog` and `bat` are one orbit, a
/// consonant, a vowel and a consonant; under `case`, so are `Error` and
/// `ERROR`, and under `numeric`, `1e3`, `0x3e8` and `1_000`. -SameAs lists
/// the tokens in the same orbit as a text: every token that equals it up to
/// the group.
///
/// # Examples
/// Measure-TrexOrbit 'Cat cat CAT dog' -Group case
/// Measure-TrexOrbit -Path ./app.log -Group numeric | Select-Object Forms, Orbits
/// (Measure-TrexOrbit -Path ./hosts.txt -Group subnet/24 -SameAs 10.0.0.1).Matches
#[cmdlet(verb = "Measure", noun = "TrexOrbit", alias = "Measure-TxOrbit", default_parameter_set = "Text", output = ["Trex.OrbitReport"])]
#[derive(Default)]
pub struct MeasureTrexOrbit {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The symmetry the tokens are read under: `shape` when absent.
    #[param]
    pub group: Option<String>,
    /// A text: the report lists every token in its orbit, each equal to it up
    /// to the group.
    #[param]
    pub same_as: Option<String>,
    /// Adds every token with its orbit and the runs of one kind of
    /// character.
    #[param]
    pub detail: bool,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
    orbit: Option<(String, trex::orbit::OrbitGroup)>,
}

impl MeasureTrexOrbit {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexOrbitReport> {
        let Some((group_name, group)) = &self.orbit else {
            return Err(arg_err("TrexOrbitGroup", "the group was not read"));
        };
        let bytes = input.text.as_bytes();
        let toks = lexed(bytes, shapes, true);
        let spans: Vec<(usize, usize)> = toks.iter().map(|t| (t.start(), t.end())).collect();
        let orbits: Vec<String> = spans.iter().map(|&(s, e)| trex::orbit::canonical(&bytes[s..e], *group)).collect();

        let mut table: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (k, &span) in spans.iter().enumerate() {
            let forms = table.entry(orbits[k].as_str()).or_default();
            let text = text_of(bytes, span);
            if !forms.contains(&text) {
                forms.push(text);
            }
        }
        let mut distinct_forms: Vec<&[u8]> = spans.iter().map(|&(s, e)| &bytes[s..e]).collect();
        distinct_forms.sort_unstable();
        distinct_forms.dedup();
        let classes: Vec<TrexOrbitClass> = table
            .into_iter()
            .map(|(orbit, mut forms)| {
                forms.sort();
                TrexOrbitClass { orbit: orbit.to_string(), forms }
            })
            .collect();

        let token = |k: usize, offset: i64, text: String| TrexOrbitToken {
            offset,
            length: text.encode_utf16().count() as i64,
            text,
            orbit: orbits[k].clone(),
        };
        let hits: Vec<usize> = match &self.same_as {
            Some(q) => {
                let key = trex::orbit::canonical(q.as_bytes(), *group);
                (0..spans.len()).filter(|&k| orbits[k] == key).collect()
            }
            None => Vec::new(),
        };
        let all: Vec<usize> = if self.detail { (0..spans.len()).collect() } else { Vec::new() };
        let segments =
            if self.detail { segments_between(bytes, &trex::orbit::shape_boundaries(bytes)) } else { Vec::new() };
        Ok(TrexOrbitReport {
            length: input.length(),
            path: input.path,
            group: group_name.clone(),
            tokens: spans.len() as i64,
            forms: distinct_forms.len() as i64,
            orbits: classes.len() as i64,
            classes,
            matches: frames_at(bytes, &spans, &hits, token),
            segments,
            frames: frames_at(bytes, &spans, &all, token),
        })
    }
}

impl Cmdlet for MeasureTrexOrbit {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let name = match &self.group {
            Some(g) => g.trim().to_string(),
            None => "shape".to_string(),
        };
        let group = group_of(&name)?;
        self.orbit = Some((name, group));
        self.shapes = Some(atoms_for(ps, &self.library)?);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shapes) = &self.shapes else {
            return Err(arg_err("TrexNoAtoms", "the atoms were not read"));
        };
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| {
            ps.write(self.report(input, shapes)?)
        })
    }
}

/// One token's shape reading.
#[psclass(name = "Trex.ShapeFrame", show = "{Text} at {Offset}")]
#[derive(Clone, Default)]
pub struct TrexShapeFrame {
    /// Where the token starts, in UTF-16 code units.
    pub offset: i64,
    /// The token's text.
    pub text: String,
    /// The token's shape class: its kind and silhouette as one number, equal
    /// for tokens of one shape.
    pub class: u32,
    /// The period, in tokens, at which the shapes around the token repeat;
    /// 0 where they do not.
    pub period: i32,
    /// How strongly the shapes repeat at that period, from 0 to 1: the share
    /// of the tokens in the window behind the token whose shape equals the
    /// shape one period before. A run whose strength reaches
    /// -TemplateStrength is a region.
    pub period_strength: f32,
    /// How new the run of shapes ending here is, from 0 to 1, 1 for its
    /// first sighting.
    pub novelty: f32,
}

/// A run of tokens whose shapes repeat at one period: a table, a list of
/// records, a template filled in again and again.
#[psclass(name = "Trex.ShapeRegion", show = "at {Offset}, {Length} long, period {Period}")]
#[derive(Clone, Default)]
pub struct TrexShapeRegion {
    /// Where the run starts, in UTF-16 code units.
    pub offset: i64,
    /// How many UTF-16 code units it spans.
    pub length: i64,
    /// The period its shapes repeat at, in tokens.
    pub period: i32,
}

/// The shape axis over one input: the template its tokens repeat, where
/// that repetition holds, and where it breaks.
#[psclass(name = "Trex.ShapeReport")]
#[derive(Clone, Default)]
pub struct TrexShapeReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// The group the token shapes were read under, empty for the lexer's
    /// own.
    pub group: String,
    /// How many tokens the axis read, whitespace left out.
    pub tokens: i64,
    /// The period of the strongest repetition, in tokens; 0 where nothing
    /// repeats.
    pub dominant_period: i32,
    /// How strongly the dominant period repeats, from 0 to 1.
    pub dominant_strength: f32,
    /// The runs whose shapes repeat with a strength of at least
    /// -TemplateStrength, 0.6 by default, each ending at the last token that
    /// repeats the kind one period before it.
    pub regions: Vec<TrexShapeRegion>,
    /// Where the silhouette breaks, in UTF-16 code units.
    pub change_points: Vec<i64>,
    /// Every token's frame, with -Detail.
    pub frames: Vec<TrexShapeFrame>,
}

/// Reads the shape axis: each token's shape, the period at which the shapes
/// repeat, and the regions that repeat like a table or a list of records.
///
/// A table reads as a region whether or not its columns line up, since the
/// period is of token shapes and not of bytes. -Group reads each token's
/// shape under a symmetry. A region is a run whose period repeats with a
/// strength of at least -TemplateStrength, 0.6 by default.
///
/// # Examples
/// Measure-TrexShape "id=1 ok`nid=2 ok`nid=3 fail"
/// Measure-TrexShape -Path ./data.csv | Select-Object DominantPeriod, Regions
/// Measure-TrexShape -Path ./data.csv -TemplateStrength 0.9 | Select-Object -ExpandProperty Regions
#[cmdlet(verb = "Measure", noun = "TrexShape", alias = "Measure-TxShape", default_parameter_set = "Text", output = ["Trex.ShapeReport"])]
#[derive(Default)]
pub struct MeasureTrexShape {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The symmetry each token's shape is read under; the lexer's own shape
    /// when absent.
    #[param]
    pub group: Option<String>,
    /// Adds every token's frame to the report.
    #[param]
    pub detail: bool,
    /// How strongly a run's period must repeat for it to be a region, from 0
    /// to 1: 0.6 when absent.
    #[param]
    pub template_strength: Option<f32>,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
    orbit: Option<(String, trex::orbit::OrbitGroup)>,
}

impl MeasureTrexShape {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexShapeReport> {
        let defaults = trex::shape::ShapeConfig::default();
        let cfg = trex::shape::ShapeConfig {
            template_strength: crate::axes::cut("TemplateStrength", self.template_strength, defaults.template_strength)?,
            ..defaults
        };
        let bytes = input.text.as_bytes();
        let toks = lexed(bytes, shapes, true);
        let f = match &self.orbit {
            Some((_, g)) => trex::shape::analyze_over_with(&toks, bytes, *g, &cfg),
            None => trex::shape::analyze_with(&toks, bytes, &cfg),
        };
        let dominant = f
            .frames
            .iter()
            .filter(|r| r.period > 0)
            .max_by(|a, b| a.period_strength.total_cmp(&b.period_strength))
            .map(|r| (i32::from(r.period), r.period_strength));
        let (dominant_period, dominant_strength) = dominant.unwrap_or((0, 0.0));
        let regions = f.shape_regions();
        let region_ends: Vec<usize> = regions.iter().flat_map(|&(s, e, _)| [s, e]).collect();
        let region_offsets = utf16_offsets(bytes, &region_ends);
        let frame = |i: usize, offset: i64, text: String| {
            let r = &f.frames[i];
            TrexShapeFrame {
                offset,
                text,
                class: r.class,
                period: i32::from(r.period),
                period_strength: r.period_strength,
                novelty: r.novelty,
            }
        };
        let all: Vec<usize> = if self.detail { (0..f.frames.len()).collect() } else { Vec::new() };
        Ok(TrexShapeReport {
            length: input.length(),
            path: input.path,
            group: match &self.orbit {
                Some((name, _)) => name.clone(),
                None => String::new(),
            },
            tokens: f.n_tokens as i64,
            dominant_period,
            dominant_strength,
            regions: regions
                .iter()
                .enumerate()
                .map(|(k, &(_, _, period))| TrexShapeRegion {
                    offset: region_offsets[2 * k],
                    length: region_offsets[2 * k + 1] - region_offsets[2 * k],
                    period: i32::from(period),
                })
                .collect(),
            change_points: utf16_offsets(bytes, &f.boundaries),
            frames: frames_at(bytes, &f.spans, &all, frame),
        })
    }
}

impl Cmdlet for MeasureTrexShape {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        self.orbit = match &self.group {
            Some(g) => Some((g.trim().to_string(), group_of(g)?)),
            None => None,
        };
        self.shapes = Some(atoms_for(ps, &self.library)?);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shapes) = &self.shapes else {
            return Err(arg_err("TrexNoAtoms", "the atoms were not read"));
        };
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| {
            ps.write(self.report(input, shapes)?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run between cuts covers the input end to end, the first from the
    /// start and the last to the end.
    #[test]
    fn segments_between_cuts_cover_the_input() {
        let bytes = b"ab 12";
        let segs = segments_between(bytes, &[2, 3]);
        let texts: Vec<&str> = segs.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["ab", " ", "12"]);
        assert_eq!(segs.iter().map(|s| s.length).sum::<i64>(), 5);
    }
}
