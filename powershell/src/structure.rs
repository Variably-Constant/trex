//! The axes of structure: how an input's tokens relate, how its byte
//! statistics change along it, and where its units begin.

use pwrs::prelude::*;

use crate::atoms::{TrexLibrary, atoms_for};
use crate::axes::{Input, each_input, frames_at, lexed, text_of, utf16_offsets};
use crate::common::arg_err;
use crate::recurrence::{TrexSegment, segments_of};

/// The relation an edge between two tokens carries.
#[psenum(name = "Trex.RelationKind")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RelationKind {
    /// From the word heading a bracket group to a token inside it.
    #[default]
    Encloses,
    /// From one operand of a binding punctuation to the other, in order.
    Operator,
    /// From a significant token to the next.
    Adjacent,
}

impl RelationKind {
    fn of(kind: trex::relation::RelationKind) -> Self {
        match kind {
            trex::relation::RelationKind::Encloses => RelationKind::Encloses,
            trex::relation::RelationKind::Operator => RelationKind::Operator,
            trex::relation::RelationKind::Adjacent => RelationKind::Adjacent,
        }
    }
}

/// One token's place in the enclosure tree.
#[psclass(name = "Trex.RelationFrame", show = "{Text} at {Offset}, depth {Depth}")]
#[derive(Clone, Default)]
pub struct TrexRelationFrame {
    /// Where the token starts, in UTF-16 code units.
    pub offset: i64,
    /// The token's text.
    pub text: String,
    /// How many brackets enclose it.
    pub depth: i32,
    /// The heads of the bracket groups enclosing it, outermost first.
    pub enclosure: Vec<String>,
}

/// One directed relation between two tokens.
#[psclass(name = "Trex.RelationEdge", show = "{From} {Kind} {To}")]
#[derive(Clone, Default)]
pub struct TrexRelationEdge {
    /// The relation the edge carries.
    pub kind: RelationKind,
    /// The text of the token it runs from.
    pub from: String,
    /// The text of the token it runs to.
    pub to: String,
    /// Where the token it runs from starts, in UTF-16 code units.
    pub from_offset: i64,
    /// Where the token it runs to starts, in UTF-16 code units.
    pub to_offset: i64,
}

/// One reuse: a later occurrence of repeated content joined to the earlier.
#[psclass(name = "Trex.RelationChord", show = "{From} to {To}")]
#[derive(Clone, Default)]
pub struct TrexRelationChord {
    /// The earlier occurrence's text.
    pub from: String,
    /// The later occurrence's text.
    pub to: String,
    /// The later occurrence's depth less the earlier's: positive where the
    /// reuse sits deeper.
    pub residual: i32,
    /// Where the earlier occurrence starts, in UTF-16 code units.
    pub from_offset: i64,
    /// Where the later occurrence starts, in UTF-16 code units.
    pub to_offset: i64,
}

/// The relation axis over one input: the graph its brackets, operators,
/// adjacency and reuse make, read as a tree with loops.
#[psclass(name = "Trex.RelationReport")]
#[derive(Clone, Default)]
pub struct TrexRelationReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// How many tokens the lexer read, whitespace included.
    pub tokens: i64,
    /// The deepest nesting of brackets.
    pub max_depth: i32,
    /// How many tokens sit inside at least one bracket group.
    pub nesting_load: i64,
    /// The edges from a bracket group's head to the tokens inside it.
    pub encloses: i64,
    /// The edges between the operands of a binding punctuation.
    pub operators: i64,
    /// The edges from each significant token to the next.
    pub adjacencies: i64,
    /// The reuses: later occurrences of repeated content joined to the
    /// earlier.
    pub reuse_chords: i64,
    /// The reuses whose two occurrences sit at different depths.
    pub scope_crossing_chords: i64,
    /// The reuses' depth differences summed without sign: 0 for a tree, and
    /// positive once a reuse crosses a scope.
    pub holonomy: i64,
    /// The same sum with its sign: positive where reuse flows inward,
    /// negative outward, 0 where the two cancel.
    pub net_holonomy: i64,
    /// The brackets the input holds, opens and closes, as the boundary
    /// reading counts them.
    pub boundary_events: i64,
    /// The balanced bracket groups.
    pub bulk_nodes: i64,
    /// What the brackets alone do not determine: the reuses.
    pub holographic_defect: i64,
    /// Whether every bracket is balanced, so that the brackets alone
    /// determine the nesting.
    pub boundary_closed: bool,
    /// The enclosure edges rebuilt from the brackets alone.
    pub rebuilt_edges: i64,
    /// The most negative edge curvature: the sharpest bottleneck, 0 for a
    /// graph with no edges.
    pub min_ricci: i32,
    /// The mean edge curvature.
    pub mean_ricci: f32,
    /// The edges of negative curvature: the bottlenecks.
    pub bridges: i64,
    /// The tokens that take part in at least one relation.
    pub nodes: i64,
    /// The relation graph's undirected edges.
    pub graph_edges: i64,
    /// Its connected components.
    pub components: i64,
    /// Its independent loops: edges less nodes plus components.
    pub cycle_rank: i64,
    /// Its Euler characteristic: nodes less edges.
    pub euler: i64,
    /// The longest run of tokens a single reuse joins in one step; 0 where
    /// nothing is reused.
    pub max_shortcut: i64,
    /// The most relation edges crossing any boundary between two tokens.
    pub peak_entanglement: i64,
    /// The fewest relation edges crossing a boundary between two tokens;
    /// `$null` where the input has no boundary inside it.
    pub minimal_cut: PsObject,
    /// Where that boundary falls, in UTF-16 code units: the start of the
    /// token after it; `$null` with MinimalCut.
    pub minimal_cut_offset: PsObject,
    /// The input's alpha-equivalence form, with -Canonical: the first word
    /// inside each `[` is a binder written `#`, a later use of it `^k` for
    /// the binder k scopes out, a name bound nowhere is kept, and the tokens
    /// are joined by spaces.
    pub canonical: String,
    /// Every relation edge, with -Detail.
    pub edges: Vec<TrexRelationEdge>,
    /// Every reuse, with -Detail.
    pub chords: Vec<TrexRelationChord>,
    /// The frame of every token inside a bracket group, with -Detail.
    pub frames: Vec<TrexRelationFrame>,
}

/// Reads the relation axis: the graph an input's brackets, binding
/// punctuation, adjacency and repeated content make, and what that graph's
/// tree and its loops measure.
///
/// A reuse joins a later occurrence of repeated content to the earlier, and
/// its depth difference is its holonomy: a tree has none, and a name bound
/// outside and used deeper has some. The curvature, topology, geodesic and
/// entanglement readings are of the same graph. -Canonical adds the input's
/// alpha-equivalence form, which reads alike for two texts that differ only
/// in the names their brackets bind.
///
/// # Examples
/// Measure-TrexRelation 'let x = f(y); g(x, [z (z)])'
/// Measure-TrexRelation -Path ./main.rs | Select-Object MaxDepth, Holonomy, CycleRank
/// Measure-TrexRelation '[ x ( x ) ]' -Canonical | Select-Object -ExpandProperty Canonical
#[cmdlet(verb = "Measure", noun = "TrexRelation", alias = "Measure-TxRelation", default_parameter_set = "Text", output = ["Trex.RelationReport"])]
#[derive(Default)]
pub struct MeasureTrexRelation {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// Adds the input's alpha-equivalence form.
    #[param]
    pub canonical: bool,
    /// Adds every edge, every reuse and every nested token's frame.
    #[param]
    pub detail: bool,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
}

impl MeasureTrexRelation {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexRelationReport> {
        use trex::relation::RelationKind as K;
        let bytes = input.text.as_bytes();
        let toks = lexed(bytes, shapes, false);
        let f = trex::relation::analyze(&toks, bytes);
        let h = trex::holography::analyze(&toks, bytes);
        let curvature = trex::curvature::analyze(&toks, bytes);
        let topology = trex::topology::analyze(&toks, bytes);
        let geodesic = trex::geodesic::analyze(&toks, bytes);
        let entanglement = trex::entanglement::analyze(&toks, bytes);
        let (minimal_cut, minimal_cut_offset) = match entanglement.min_cut() {
            Some((k, crossings)) => {
                let at = match f.spans.get(k) {
                    Some(&(s, _)) => s,
                    None => bytes.len(),
                };
                ((crossings as i64).into_ps()?, utf16_offsets(bytes, &[at])[0].into_ps()?)
            }
            None => (PsObject::default(), PsObject::default()),
        };

        let (mut edges, mut chords, mut frames) = (Vec::new(), Vec::new(), Vec::new());
        if self.detail {
            let starts: Vec<usize> = f.spans.iter().map(|&(s, _)| s).collect();
            let offsets = utf16_offsets(bytes, &starts);
            let word = |i: usize| text_of(bytes, f.spans[i]);
            for e in &f.edges {
                edges.push(TrexRelationEdge {
                    kind: RelationKind::of(e.kind),
                    from: word(e.from),
                    to: word(e.to),
                    from_offset: offsets[e.from],
                    to_offset: offsets[e.to],
                });
            }
            for c in &f.chords {
                chords.push(TrexRelationChord {
                    from: word(c.from),
                    to: word(c.to),
                    residual: c.residual,
                    from_offset: offsets[c.from],
                    to_offset: offsets[c.to],
                });
            }
            let nested: Vec<usize> = (0..f.frames.len()).filter(|&i| f.frames[i].depth > 0).collect();
            frames = frames_at(bytes, &f.spans, &nested, |i, offset, text| TrexRelationFrame {
                offset,
                text,
                depth: i32::from(f.frames[i].depth),
                enclosure: f.frames[i].enclosure.iter().map(|&head| word(head)).collect(),
            });
        }

        Ok(TrexRelationReport {
            length: input.length(),
            path: input.path,
            tokens: f.n_tokens as i64,
            max_depth: i32::from(f.max_depth()),
            nesting_load: f.nesting_load as i64,
            encloses: f.edges_of(K::Encloses).count() as i64,
            operators: f.edges_of(K::Operator).count() as i64,
            adjacencies: f.edges_of(K::Adjacent).count() as i64,
            reuse_chords: f.chords.len() as i64,
            scope_crossing_chords: f.twisted_chords() as i64,
            holonomy: i64::from(f.holonomy()),
            net_holonomy: i64::from(f.net_holonomy()),
            boundary_events: h.boundary.len() as i64,
            bulk_nodes: h.bulk_nodes as i64,
            holographic_defect: h.holographic_defect as i64,
            boundary_closed: h.boundary_closed,
            rebuilt_edges: h.reconstruct_bulk(&toks).len() as i64,
            min_ricci: curvature.min_ricci(),
            mean_ricci: curvature.mean_ricci(),
            bridges: curvature.bridges() as i64,
            nodes: topology.nodes as i64,
            graph_edges: topology.edges as i64,
            components: topology.components as i64,
            cycle_rank: topology.cycle_rank as i64,
            euler: topology.euler(),
            max_shortcut: geodesic.max_shortcut() as i64,
            peak_entanglement: entanglement.max_entanglement() as i64,
            minimal_cut,
            minimal_cut_offset,
            canonical: if self.canonical { trex::gauge::canonicalize(&toks, bytes) } else { String::new() },
            edges,
            chords,
            frames,
        })
    }
}

impl Cmdlet for MeasureTrexRelation {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
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

/// The texture a stretch of bytes has, by the classes of its bytes.
#[psenum(name = "Trex.Texture")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Texture {
    /// Letters dominate and punctuation is low: prose.
    #[default]
    Prose,
    /// Punctuation and symbols high beside identifiers: code.
    Code,
    /// Single symbols and digits mixed: mathematics.
    Math,
    /// High entropy or many bytes above ASCII: compressed, packed or binary
    /// data.
    Data,
    /// No class dominates.
    Mixed,
}

impl Texture {
    fn of(t: trex::spectral::Texture) -> Self {
        match t {
            trex::spectral::Texture::Prose => Texture::Prose,
            trex::spectral::Texture::Code => Texture::Code,
            trex::spectral::Texture::Math => Texture::Math,
            trex::spectral::Texture::Data => Texture::Data,
            trex::spectral::Texture::Mixed => Texture::Mixed,
        }
    }
}

/// What a region of an input is, by its texture and by whether its token
/// shapes repeat.
#[psenum(name = "Trex.RegionKind")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RegionKind {
    /// Token shapes that repeat at a strong period: a table or a list of
    /// records, whatever its bytes look like.
    #[default]
    Table,
    /// A high-entropy run: packed, encoded or encrypted bytes.
    Blob,
    /// Prose.
    Prose,
    /// Numbers.
    Numeric,
    /// Code whose shapes do not repeat strongly.
    Code,
    /// No texture dominates.
    Mixed,
}

/// One frame of the spectral reading: the byte statistics of a window.
#[psclass(name = "Trex.SpectralFrame", show = "{Texture} at {Offset}")]
#[derive(Clone, Default)]
pub struct TrexSpectralFrame {
    /// The last byte the frame reads, in UTF-16 code units.
    pub offset: i64,
    /// The window's texture.
    pub texture: Texture,
    /// The window's byte entropy, from 0 to 1.
    pub entropy: f32,
    /// The period its bytes repeat at, in bytes; 0 where none.
    pub period: i32,
    /// How new its runs of bytes are, from 0 to 1, 1 for a first sighting.
    pub novelty: f32,
    /// The shares of digits, letters, whitespace, punctuation and bytes
    /// above ASCII among its recent bytes, in that order.
    pub mix: Vec<f32>,
}

/// A byte period the spectral reading found, and how strongly it holds.
#[psclass(name = "Trex.SpectralPeriod", show = "{Period} ({Strength})")]
#[derive(Clone, Default)]
pub struct TrexSpectralPeriod {
    /// The period, in bytes.
    pub period: i32,
    /// Its strongest showing, from 0 to 1.
    pub strength: f32,
}

/// A stretch of an input between two change points, with its texture.
#[psclass(name = "Trex.TextureRegion", show = "{Texture} at {Offset}, {Length} long")]
#[derive(Clone, Default)]
pub struct TrexTextureRegion {
    /// Where the stretch starts, in UTF-16 code units.
    pub offset: i64,
    /// How many UTF-16 code units it spans.
    pub length: i64,
    /// Its texture.
    pub texture: Texture,
    /// Its byte entropy, from 0 to 1.
    pub entropy: f32,
    /// The period its bytes repeat at, in bytes; 0 where none.
    pub period: i32,
    /// How new its runs of bytes are, from 0 to 1.
    pub novelty: f32,
}

/// A region kind as the module writes it, with a table's period in tokens
/// beside it and 0 for every other kind.
pub(crate) fn region_kind(kind: trex::shape::RegionKind) -> (RegionKind, i32) {
    match kind {
        trex::shape::RegionKind::Table(p) => (RegionKind::Table, i32::from(p)),
        trex::shape::RegionKind::Blob => (RegionKind::Blob, 0),
        trex::shape::RegionKind::Prose => (RegionKind::Prose, 0),
        trex::shape::RegionKind::Numeric => (RegionKind::Numeric, 0),
        trex::shape::RegionKind::Code => (RegionKind::Code, 0),
        trex::shape::RegionKind::Mixed => (RegionKind::Mixed, 0),
    }
}

/// A region of an input classified by its texture and its token shapes.
#[psclass(name = "Trex.ClassifiedRegion", show = "{Kind} at {Offset}, {Length} long")]
#[derive(Clone, Default)]
pub struct TrexClassifiedRegion {
    /// Where the region starts, in UTF-16 code units.
    pub offset: i64,
    /// How many UTF-16 code units it spans.
    pub length: i64,
    /// What the region is.
    pub kind: RegionKind,
    /// A table's period in tokens; 0 for every other kind.
    pub period: i32,
}

/// The spectral axis over one input: its byte statistics along its length,
/// where they change, and the texture of each stretch between.
#[psclass(name = "Trex.SpectralReport")]
#[derive(Clone, Default)]
pub struct TrexSpectralReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// How many bytes each frame advances.
    pub hop: i64,
    /// Where the byte statistics change, in UTF-16 code units.
    pub change_points: Vec<i64>,
    /// The least byte entropy of any frame, from 0 to 1; `$null` for an
    /// input with no frames.
    pub entropy_minimum: PsObject,
    /// The mean byte entropy over the frames; `$null` for an input with no
    /// frames.
    pub entropy_mean: PsObject,
    /// The greatest byte entropy of any frame; `$null` for an input with no
    /// frames.
    pub entropy_maximum: PsObject,
    /// The distinct byte periods the frames found, strongest first.
    pub periods: Vec<TrexSpectralPeriod>,
    /// The input cut at its change points, each stretch with its texture.
    pub timeline: Vec<TrexTextureRegion>,
    /// The input's regions classified by texture and token shape, with
    /// -Classify.
    pub regions: Vec<TrexClassifiedRegion>,
    /// Every frame, with -Detail.
    pub frames: Vec<TrexSpectralFrame>,
}

/// Reads the spectral axis: the entropy, byte period, novelty and class mix
/// of an input's bytes along its length, where they change, and the
/// texture of each stretch between, prose, code, mathematics, data or
/// mixed.
///
/// -Classify adds the regions read by texture and by token shape together,
/// so a table whose columns do not line up still reads as a table. The axis
/// reads bytes, so it takes no atoms.
///
/// # Examples
/// Measure-TrexSpectral -Path ./mixed.txt | Select-Object -ExpandProperty Timeline
/// Measure-TrexSpectral -Path ./dump.bin -Classify | Select-Object -ExpandProperty Regions
#[cmdlet(verb = "Measure", noun = "TrexSpectral", alias = "Measure-TxSpectral", default_parameter_set = "Text", output = ["Trex.SpectralReport"])]
#[derive(Default)]
pub struct MeasureTrexSpectral {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// Adds the regions read by texture and token shape together.
    #[param]
    pub classify: bool,
    /// Adds every frame to the report.
    #[param]
    pub detail: bool,
}

impl MeasureTrexSpectral {
    fn report(&self, input: Input) -> PsResult<TrexSpectralReport> {
        let bytes = input.text.as_bytes();
        let f = trex::spectral::analyze(bytes);

        let (mut entropy_minimum, mut entropy_mean, mut entropy_maximum) =
            (PsObject::default(), PsObject::default(), PsObject::default());
        if !f.frames.is_empty() {
            let (mut lo, mut hi, mut sum) = (f32::INFINITY, f32::NEG_INFINITY, 0.0f32);
            for r in &f.frames {
                lo = lo.min(r.entropy);
                hi = hi.max(r.entropy);
                sum += r.entropy;
            }
            entropy_minimum = lo.into_ps()?;
            entropy_mean = (sum / f.frames.len() as f32).into_ps()?;
            entropy_maximum = hi.into_ps()?;
        }

        let mut periods: Vec<TrexSpectralPeriod> = Vec::new();
        for r in f.frames.iter().filter(|r| r.period != 0) {
            let period = i32::from(r.period);
            match periods.iter_mut().find(|p| p.period == period) {
                Some(p) => p.strength = p.strength.max(r.period_strength),
                None => periods.push(TrexSpectralPeriod { period, strength: r.period_strength }),
            }
        }
        periods.sort_by(|a, b| b.strength.total_cmp(&a.strength));

        let stretches = trex::spectral::regions(&f);
        let ends: Vec<usize> = stretches.iter().flat_map(|&(s, e, _)| [s, e]).collect();
        let at = utf16_offsets(bytes, &ends);
        let timeline = stretches
            .iter()
            .enumerate()
            .map(|(k, &(s, e, texture))| {
                let sig = f.signature(s, e);
                TrexTextureRegion {
                    offset: at[2 * k],
                    length: at[2 * k + 1] - at[2 * k],
                    texture: Texture::of(texture),
                    entropy: sig.entropy,
                    period: i32::from(sig.period),
                    novelty: sig.novelty,
                }
            })
            .collect();

        let mut regions = Vec::new();
        if self.classify {
            let classified = trex::shape::classified_regions(bytes);
            let ends: Vec<usize> = classified.iter().flat_map(|&(s, e, _)| [s, e]).collect();
            let at = utf16_offsets(bytes, &ends);
            for (k, &(_, _, kind)) in classified.iter().enumerate() {
                let (kind, period) = region_kind(kind);
                regions.push(TrexClassifiedRegion { offset: at[2 * k], length: at[2 * k + 1] - at[2 * k], kind, period });
            }
        }

        let mut frames = Vec::new();
        if self.detail && !bytes.is_empty() {
            let last: Vec<usize> =
                (0..f.frames.len()).map(|j| ((j + 1) * f.hop).min(f.len).saturating_sub(1)).collect();
            let at = utf16_offsets(bytes, &last);
            for (r, offset) in f.frames.iter().zip(at) {
                frames.push(TrexSpectralFrame {
                    offset,
                    texture: Texture::of(trex::spectral::texture_of(r)),
                    entropy: r.entropy,
                    period: i32::from(r.period),
                    novelty: r.novelty,
                    mix: r.texture_mix().to_vec(),
                });
            }
        }

        Ok(TrexSpectralReport {
            length: input.length(),
            path: input.path,
            hop: f.hop as i64,
            change_points: utf16_offsets(bytes, &f.boundaries),
            entropy_minimum,
            entropy_mean,
            entropy_maximum,
            periods,
            timeline,
            regions,
            frames,
        })
    }
}

impl Cmdlet for MeasureTrexSpectral {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| ps.write(self.report(input)?))
    }
}

/// One byte's seam reading: how uncertain the text is on either side of it.
#[psclass(name = "Trex.SeamFrame", show = "{Text} at {Offset}")]
#[derive(Clone, Default)]
pub struct TrexSeamFrame {
    /// Where the byte stands, in UTF-16 code units.
    pub offset: i64,
    /// The character the byte begins, empty for a byte inside a character
    /// written in several.
    pub text: String,
    /// The forward branching entropy at the byte: how uncertain what
    /// follows is, given what precedes.
    pub forward: f32,
    /// The backward branching entropy at the byte: how uncertain what
    /// precedes is, given what follows.
    pub backward: f32,
    /// The seam strength of a cut before the byte, from both readings
    /// together.
    pub boundary: f32,
}

/// The seam axis over one input: the units it divides into where the text
/// before stops predicting the text after.
#[psclass(name = "Trex.SeamReport")]
#[derive(Clone, Default)]
pub struct TrexSeamReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// The longest context the entropy was read over, in bytes.
    pub order: i32,
    /// The units, in order.
    pub segments: Vec<TrexSegment>,
    /// Every byte's frame, with -Detail.
    pub frames: Vec<TrexSeamFrame>,
}

/// Reads the seam axis: where an input divides into units, found where the
/// text before a point stops predicting the text after it, read in both
/// directions, with no dictionary.
///
/// -English reads with a model of English letter sequences, so even a short
/// string divides where English does not join its letters, and a unit may
/// be cut inside a token the lexer read. The axis reads bytes, so it takes
/// no atoms.
///
/// # Examples
/// Measure-TrexSeam 'thequickbrownfoxjumpsoverthelazydog' -English | Select-Object -ExpandProperty Segments
/// Measure-TrexSeam -Path ./blob.txt -Order 4
#[cmdlet(verb = "Measure", noun = "TrexSeam", alias = "Measure-TxSeam", default_parameter_set = "Text", output = ["Trex.SeamReport"])]
#[derive(Default)]
pub struct MeasureTrexSeam {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The longest context the entropy is read over, in bytes: 3 when
    /// absent.
    #[param]
    pub order: Option<u32>,
    /// How many confidence passes to run: one when absent.
    #[param]
    pub passes: Option<u32>,
    /// Reads with a model of English letter sequences.
    #[param]
    pub english: bool,
    /// Adds every byte's frame to the report.
    #[param]
    pub detail: bool,
}

impl MeasureTrexSeam {
    fn report(&self, input: Input) -> PsResult<TrexSeamReport> {
        let order = match self.order {
            Some(0) => return Err(arg_err("TrexSeamOrder", "-Order is a context length in bytes, 1 or more")),
            Some(k) => k as usize,
            None => 3,
        };
        let passes = match self.passes {
            Some(p) => p as usize,
            None => 0,
        };
        let cfg = trex::seam::SeamConfig { order, passes, ..trex::seam::SeamConfig::default() };
        let bytes = input.text.as_bytes();
        let f = if self.english {
            trex::seam::analyze_with_model(bytes, &trex::seam::english_model(), &cfg)
        } else {
            trex::seam::analyze_with(bytes, &cfg)
        };
        let mut frames = Vec::new();
        if self.detail {
            let all: Vec<usize> = (0..bytes.len()).collect();
            let at = utf16_offsets(bytes, &all);
            for (b, offset) in at.into_iter().enumerate() {
                let text = match bytes[b] {
                    0x80..=0xBF => String::new(),
                    _ => match input.text.get(b..).and_then(|rest| rest.chars().next()) {
                        Some(c) => c.to_string(),
                        None => String::new(),
                    },
                };
                frames.push(TrexSeamFrame {
                    offset,
                    text,
                    forward: f.fwd_entropy[b],
                    backward: f.bwd_entropy[b],
                    boundary: f.boundary[b],
                });
            }
        }
        Ok(TrexSeamReport {
            length: input.length(),
            order: order as i32,
            segments: segments_of(bytes, &f.segments()),
            path: input.path,
            frames,
        })
    }
}

impl Cmdlet for MeasureTrexSeam {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| ps.write(self.report(input)?))
    }
}
