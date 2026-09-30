//! The analysis axes: readings of an input's structure that need no pattern,
//! one cmdlet an axis, carrying what the trex command's analysis commands
//! print.
//!
//! Each cmdlet writes one report an input: a string piped in, or each file
//! -Path names. A report holds the axis's summary readings and the points it
//! marks, each point the frame of the token or the byte it falls on, so a
//! point carries its reading; -Detail adds every frame. Offsets are UTF-16
//! code units, as a match's are. The readings are trex's own, as the
//! single-precision numbers it computes them in.
//!
//! An axis that reads tokens lexes with the atoms in force, the session's or
//! -Library's, so a declared shape or kind is one token of its own; with no
//! atoms declared its readings are the trex command's.

use pwrs::prelude::*;

use crate::atoms::{TrexLibrary, atoms_for};
use crate::common::{Units, arg_err};
use crate::matching::{files_of, read_text};

/// One input an axis reads: the file it came from, empty for a string, and
/// its text.
pub(crate) struct Input {
    pub(crate) path: String,
    pub(crate) text: String,
}

impl Input {
    /// The text's length in UTF-16 code units.
    pub(crate) fn length(&self) -> i64 {
        self.text.encode_utf16().count() as i64
    }
}

/// The inputs a cmdlet was given, handed to `each` one at a time: the string
/// piped in, or every file -Path or -LiteralPath names, walked and decoded as
/// a scan reads one. A file that cannot be read is an error record, and the
/// rest are still read.
pub(crate) fn each_input(
    ps: &Pipeline<'_>,
    text: &str,
    path: &[String],
    literal_path: &[String],
    mut each: impl FnMut(Input) -> PsResult<()>,
) -> PsResult<()> {
    if path.is_empty() && literal_path.is_empty() {
        return each(Input { path: String::new(), text: text.to_string() });
    }
    let (given, literal) = if literal_path.is_empty() { (path, false) } else { (literal_path, true) };
    for source in files_of(ps, given, literal, &trex::files::WalkOptions::default())? {
        if ps.stopping() {
            break;
        }
        let trex::files::Source::File(file) = source else {
            continue;
        };
        match read_text(&file, false) {
            Ok(Some(text)) => each(Input { path: file.display().to_string(), text })?,
            Ok(None) => {}
            Err(e) => ps.write_error(&e)?,
        }
    }
    Ok(())
}

/// The tokens of `bytes` as the atoms in `shapes` lex them: every token, or
/// only the significant ones, which leaves out whitespace.
pub(crate) fn lexed(bytes: &[u8], shapes: &trex::ShapeSet, significant: bool) -> Vec<trex::token::Token> {
    let toks = if shapes.is_empty() {
        trex::lexer::lex(bytes)
    } else {
        trex::lexer::lex_with_shapes(bytes, &trex::lexer::blob_runs(bytes), shapes, 0)
    };
    if significant { toks.into_iter().filter(|t| t.is_significant()).collect() } else { toks }
}

/// UTF-16 offsets of byte offsets given in any order, found in one pass over
/// the text.
pub(crate) fn utf16_offsets(bytes: &[u8], offsets: &[usize]) -> Vec<i64> {
    let mut order: Vec<usize> = (0..offsets.len()).collect();
    order.sort_by_key(|&i| offsets[i]);
    let mut units = Units::of(bytes);
    let mut out = vec![0i64; offsets.len()];
    for i in order {
        out[i] = units.at(offsets[i]) as i64;
    }
    out
}

/// The text of a byte span.
pub(crate) fn text_of(bytes: &[u8], (s, e): (usize, usize)) -> String {
    String::from_utf8_lossy(&bytes[s..e]).into_owned()
}

/// The index of the token that begins at byte `at`, among spans sorted by
/// where they begin. A point an axis marks is where a token begins, so a
/// byte that begins none is a reading this cannot place, reported rather
/// than dropped.
pub(crate) fn token_at(spans: &[(usize, usize)], at: usize) -> PsResult<usize> {
    let i = spans.partition_point(|&(s, _)| s < at);
    match spans.get(i) {
        Some(&(s, _)) if s == at => Ok(i),
        _ => Err(PsError::new(
            ErrorCategory::InvalidResult,
            "TrexAxisPoint",
            format!("trex marked byte {at}, which begins no token the axis read"),
        )),
    }
}

/// The indices of the tokens `points` mark, in the order given.
pub(crate) fn tokens_at(spans: &[(usize, usize)], points: &[usize]) -> PsResult<Vec<usize>> {
    points.iter().map(|&b| token_at(spans, b)).collect()
}

/// The frames of the tokens at `indices`, each built by `frame` from the
/// token's index, its UTF-16 offset and its text.
pub(crate) fn frames_at<F>(
    bytes: &[u8],
    spans: &[(usize, usize)],
    indices: &[usize],
    frame: impl Fn(usize, i64, String) -> F,
) -> Vec<F> {
    let starts: Vec<usize> = indices.iter().map(|&i| spans[i].0).collect();
    let offsets = utf16_offsets(bytes, &starts);
    indices.iter().zip(offsets).map(|(&i, offset)| frame(i, offset, text_of(bytes, spans[i]))).collect()
}

/// The frame at one index as a report's single-frame reading, `$null` where
/// there is none.
pub(crate) fn one<F: IntoPs>(frame: Option<F>) -> PsResult<PsObject> {
    match frame {
        Some(f) => f.into_ps(),
        None => Ok(PsObject::default()),
    }
}

/// One token's magnitude reading.
#[psclass(name = "Trex.MagnitudeFrame", show = "{Text} at {Offset}")]
#[derive(Clone, Default)]
pub struct TrexMagnitudeFrame {
    /// Where the token starts, in UTF-16 code units.
    pub offset: i64,
    /// The token's text.
    pub text: String,
    /// The token's order of magnitude: log10 of a number's absolute value,
    /// log2 of any other token's length in bytes.
    pub magnitude: f32,
    /// The change in magnitude from the token before.
    pub gradient: f32,
    /// The sum of squared magnitudes over the 16 tokens ending at this one.
    pub energy: f32,
}

/// The magnitude axis over one input: the order of magnitude each token
/// carries, where it jumps, and the tokens far from the input's mean.
#[psclass(name = "Trex.MagnitudeReport")]
#[derive(Clone, Default)]
pub struct TrexMagnitudeReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// How many tokens the axis read, whitespace left out.
    pub tokens: i64,
    /// The sum of squared magnitudes over the whole input.
    pub total_energy: f32,
    /// The token of greatest magnitude, a Trex.MagnitudeFrame; `$null` for an
    /// input with no tokens.
    pub peak: PsObject,
    /// The tokens whose magnitude differs from the one before by more than
    /// three orders.
    pub jumps: Vec<TrexMagnitudeFrame>,
    /// The tokens whose magnitude lies more than two and a half standard
    /// deviations from the input's mean.
    pub outliers: Vec<TrexMagnitudeFrame>,
    /// Every token's frame, with -Detail.
    pub frames: Vec<TrexMagnitudeFrame>,
}

/// Reads the magnitude axis: each token's order of magnitude, its change
/// from the token before, and the energy of the window behind it.
///
/// A number's magnitude is log10 of its value and any other token's log2 of
/// its length. Jumps are the tokens where it changes by more than three
/// orders from the token before; outliers are the tokens more than two and a
/// half standard deviations from the input's mean.
///
/// # Examples
/// Measure-TrexMagnitude 'size 12 then 12000000 then 3'
/// Measure-TrexMagnitude -Path ./metrics.log | Select-Object -ExpandProperty Jumps
/// (Measure-TrexMagnitude -Path ./metrics.log -Detail).Frames | Sort-Object Energy -Descending | Select-Object -First 3
#[cmdlet(verb = "Measure", noun = "TrexMagnitude", alias = "Measure-TxMagnitude", default_parameter_set = "Text", output = ["Trex.MagnitudeReport"])]
#[derive(Default)]
pub struct MeasureTrexMagnitude {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// Adds every token's frame to the report.
    #[param]
    pub detail: bool,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
}

impl MeasureTrexMagnitude {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexMagnitudeReport> {
        let bytes = input.text.as_bytes();
        let toks = lexed(bytes, shapes, true);
        let f = trex::magnitude::analyze(&toks, bytes);
        let frame = |i: usize, offset: i64, text: String| {
            let r = &f.frames[i];
            TrexMagnitudeFrame { offset, text, magnitude: r.magnitude, gradient: r.gradient, energy: r.energy }
        };
        let peak = f.frames.iter().enumerate().max_by(|a, b| a.1.magnitude.total_cmp(&b.1.magnitude)).map(|(i, _)| i);
        let peak = frames_at(bytes, &f.spans, &peak.into_iter().collect::<Vec<_>>(), frame).pop();
        let all: Vec<usize> = if self.detail { (0..f.frames.len()).collect() } else { Vec::new() };
        Ok(TrexMagnitudeReport {
            length: input.length(),
            path: input.path,
            tokens: f.n_tokens as i64,
            total_energy: f.total_energy,
            peak: one(peak)?,
            jumps: frames_at(bytes, &f.spans, &tokens_at(&f.spans, &f.jumps)?, frame),
            outliers: frames_at(bytes, &f.spans, &f.outliers(), frame),
            frames: frames_at(bytes, &f.spans, &all, frame),
        })
    }
}

impl Cmdlet for MeasureTrexMagnitude {
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

/// One token's stress reading.
#[psclass(name = "Trex.StressFrame", show = "{Text} at {Offset}, depth {Depth}")]
#[derive(Clone, Default)]
pub struct TrexStressFrame {
    /// Where the token starts, in UTF-16 code units.
    pub offset: i64,
    /// The token's text.
    pub text: String,
    /// How many paired brackets enclose the token.
    pub depth: i32,
    /// How many tokens the innermost open bracket has been held open.
    pub strain: f32,
    /// How many tokens every open bracket has been held open, summed.
    pub load: f32,
}

/// The stress axis over one input: how deeply the brackets nest at each
/// token, the peaks of that nesting, and where it releases at once.
#[psclass(name = "Trex.StressReport")]
#[derive(Clone, Default)]
pub struct TrexStressReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// How many tokens the axis read, whitespace left out.
    pub tokens: i64,
    /// The deepest nesting reached.
    pub max_depth: i32,
    /// The token carrying the greatest load, a Trex.StressFrame; `$null` for
    /// an input with no tokens.
    pub peak_load: PsObject,
    /// The tokens at a local maximum of depth, at depth 2 or more.
    pub peaks: Vec<TrexStressFrame>,
    /// The tokens where the depth starts to fall from a level 2 or more
    /// deep: where a nested structure begins to close.
    pub fractures: Vec<TrexStressFrame>,
    /// Every token's frame, with -Detail.
    pub frames: Vec<TrexStressFrame>,
}

/// Reads the stress axis: how deeply the paired brackets nest at each
/// token, how long the innermost has been held open, and how long all of
/// them together.
///
/// Peaks are the local maxima of depth, and fractures the tokens where the
/// depth starts to fall from a nested level. An unpaired bracket is a
/// character of the text and opens no level.
///
/// # Examples
/// Measure-TrexStress '{"a": [1, {"b": [2, 3]}]}'
/// Measure-TrexStress -Path ./config.json | Select-Object MaxDepth, PeakLoad
#[cmdlet(verb = "Measure", noun = "TrexStress", alias = "Measure-TxStress", default_parameter_set = "Text", output = ["Trex.StressReport"])]
#[derive(Default)]
pub struct MeasureTrexStress {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// Adds every token's frame to the report.
    #[param]
    pub detail: bool,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
}

impl MeasureTrexStress {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexStressReport> {
        let bytes = input.text.as_bytes();
        let toks = lexed(bytes, shapes, true);
        let f = trex::stress::analyze(&toks, bytes);
        let frame = |i: usize, offset: i64, text: String| {
            let r = &f.frames[i];
            TrexStressFrame { offset, text, depth: i32::from(r.depth), strain: r.strain, load: r.load }
        };
        let peak = f.frames.iter().enumerate().max_by(|a, b| a.1.load.total_cmp(&b.1.load)).map(|(i, _)| i);
        let peak = frames_at(bytes, &f.spans, &peak.into_iter().collect::<Vec<_>>(), frame).pop();
        let all: Vec<usize> = if self.detail { (0..f.frames.len()).collect() } else { Vec::new() };
        Ok(TrexStressReport {
            length: input.length(),
            path: input.path,
            tokens: f.n_tokens as i64,
            max_depth: i32::from(f.max_depth),
            peak_load: one(peak)?,
            peaks: frames_at(bytes, &f.spans, &tokens_at(&f.spans, &f.peaks)?, frame),
            fractures: frames_at(bytes, &f.spans, &tokens_at(&f.spans, &f.fractures)?, frame),
            frames: frames_at(bytes, &f.spans, &all, frame),
        })
    }
}

impl Cmdlet for MeasureTrexStress {
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

/// The per-token signal the flow axis reads the trend of.
#[psenum(name = "Trex.FlowSignal")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlowSignal {
    /// Each token's order of magnitude, as the magnitude axis reads it.
    #[default]
    Magnitude,
    /// Each token's nesting depth, as the stress axis reads it.
    Stress,
    /// Each token's length in bytes.
    Length,
}

impl FlowSignal {
    fn trex(self) -> trex::flow::Signal {
        match self {
            FlowSignal::Magnitude => trex::flow::Signal::Magnitude,
            FlowSignal::Stress => trex::flow::Signal::StressDepth,
            FlowSignal::Length => trex::flow::Signal::Length,
        }
    }
}

/// Which way a flow's trend runs at a token.
#[psenum(name = "Trex.FlowDirection")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlowDirection {
    /// Neither rising nor falling.
    #[default]
    Steady,
    /// Rising.
    Rising,
    /// Falling.
    Falling,
}

/// One token's flow reading.
#[psclass(name = "Trex.FlowFrame", show = "{Text} at {Offset}, {Direction}")]
#[derive(Clone, Default)]
pub struct TrexFlowFrame {
    /// Where the token starts, in UTF-16 code units.
    pub offset: i64,
    /// The token's text.
    pub text: String,
    /// The signal's change per token, averaged over the four tokens ending
    /// at this one.
    pub slope: f32,
    /// Which way the trend runs: Steady where the slope is within 0.05 of
    /// zero.
    pub direction: FlowDirection,
    /// How many consecutive tokens have held the same direction, rising or
    /// falling.
    pub momentum: i32,
}

/// The flow axis over one input: the trend of a per-token signal, how long
/// each run of it lasts, and where it turns.
#[psclass(name = "Trex.FlowReport")]
#[derive(Clone, Default)]
pub struct TrexFlowReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// How many tokens the axis read, whitespace left out.
    pub tokens: i64,
    /// The signal whose trend was read.
    pub signal: FlowSignal,
    /// The token where the longest run stands, a Trex.FlowFrame; `$null`
    /// for an input with no tokens.
    pub peak_momentum: PsObject,
    /// The tokens where the trend turns between rising and falling.
    pub reversals: Vec<TrexFlowFrame>,
    /// Every token's frame, with -Detail.
    pub frames: Vec<TrexFlowFrame>,
}

/// Reads the flow axis: the trend of a per-token signal, the magnitude by
/// default, how long it has run one way, and where it reverses.
///
/// -Signal Stress reads the trend of the nesting depth instead, and -Signal
/// Length that of the token lengths.
///
/// # Examples
/// Measure-TrexFlow '1 10 100 1000 10 1'
/// Measure-TrexFlow -Path ./trace.log -Signal Stress | Select-Object -ExpandProperty Reversals
#[cmdlet(verb = "Measure", noun = "TrexFlow", alias = "Measure-TxFlow", default_parameter_set = "Text", output = ["Trex.FlowReport"])]
#[derive(Default)]
pub struct MeasureTrexFlow {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The signal whose trend is read: Magnitude when absent.
    #[param]
    pub signal: Option<FlowSignal>,
    /// Adds every token's frame to the report.
    #[param]
    pub detail: bool,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
}

impl MeasureTrexFlow {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexFlowReport> {
        let signal = match self.signal {
            Some(s) => s,
            None => FlowSignal::Magnitude,
        };
        let bytes = input.text.as_bytes();
        let toks = lexed(bytes, shapes, true);
        let f = trex::flow::analyze(&toks, bytes, signal.trex());
        let frame = |i: usize, offset: i64, text: String| {
            let r = &f.frames[i];
            let direction = match r.direction {
                1 => FlowDirection::Rising,
                -1 => FlowDirection::Falling,
                _ => FlowDirection::Steady,
            };
            TrexFlowFrame { offset, text, slope: r.slope, direction, momentum: i32::from(r.momentum) }
        };
        let peak = f.frames.iter().enumerate().max_by_key(|(_, r)| r.momentum).map(|(i, _)| i);
        let peak = frames_at(bytes, &f.spans, &peak.into_iter().collect::<Vec<_>>(), frame).pop();
        let all: Vec<usize> = if self.detail { (0..f.frames.len()).collect() } else { Vec::new() };
        Ok(TrexFlowReport {
            length: input.length(),
            path: input.path,
            tokens: f.n_tokens as i64,
            signal,
            peak_momentum: one(peak)?,
            reversals: frames_at(bytes, &f.spans, &tokens_at(&f.spans, &f.reversals)?, frame),
            frames: frames_at(bytes, &f.spans, &all, frame),
        })
    }
}

impl Cmdlet for MeasureTrexFlow {
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

/// One byte's observation reading: the entropy of the byte classes around
/// it, seen from behind, from ahead and from both sides.
#[psclass(name = "Trex.ObservationFrame", show = "at {Offset}, disagreement {Disagreement}")]
#[derive(Clone, Default)]
pub struct TrexObservationFrame {
    /// Where the byte stands, in UTF-16 code units; each byte of a character
    /// written in several has a frame at the character's offset.
    pub offset: i64,
    /// The byte-class entropy of the window before the byte, from 0 to 1.
    pub causal: f32,
    /// The byte-class entropy of the window after the byte, from 0 to 1.
    pub anticausal: f32,
    /// The byte-class entropy of the window centered on the byte, from 0 to
    /// 1.
    pub centered: f32,
    /// How far the two sides differ: Causal less Anticausal, unsigned.
    pub disagreement: f32,
}

/// The observation axis over one input: where the text behind a point and
/// the text ahead of it read differently.
#[psclass(name = "Trex.ObservationReport")]
#[derive(Clone, Default)]
pub struct TrexObservationReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// The contested byte where the two sides differ most, a
    /// Trex.ObservationFrame; `$null` where none is contested.
    pub peak: PsObject,
    /// The bytes where the two sides differ most: local maxima of
    /// Disagreement above 0.2, at least 8 bytes apart.
    pub contested: Vec<TrexObservationFrame>,
    /// Every byte's frame, with -Detail.
    pub frames: Vec<TrexObservationFrame>,
}

/// Reads the observation axis: the entropy of the byte classes behind each
/// byte, ahead of it and around it, and the points where behind and ahead
/// differ most.
///
/// The entropies are of five byte classes, digits, letters, whitespace,
/// other ASCII and bytes above ASCII, so they read the texture of the text
/// and not its words. The axis reads bytes, so it takes no atoms.
///
/// # Examples
/// Measure-TrexObservation 'key=value; key=other; odd'
/// Measure-TrexObservation -Path ./app.log | Select-Object -ExpandProperty Contested
#[cmdlet(verb = "Measure", noun = "TrexObservation", alias = "Measure-TxObservation", default_parameter_set = "Text", output = ["Trex.ObservationReport"])]
#[derive(Default)]
pub struct MeasureTrexObservation {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// Adds every byte's frame to the report.
    #[param]
    pub detail: bool,
}

impl MeasureTrexObservation {
    fn report(&self, input: Input) -> PsResult<TrexObservationReport> {
        let bytes = input.text.as_bytes();
        let f = trex::observation::analyze(bytes);
        let frames_of = |at: &[usize]| -> Vec<TrexObservationFrame> {
            utf16_offsets(bytes, at)
                .into_iter()
                .zip(at)
                .map(|(offset, &b)| {
                    let r = &f.frames[b];
                    TrexObservationFrame {
                        offset,
                        causal: r.causal,
                        anticausal: r.anticausal,
                        centered: r.centered,
                        disagreement: r.disagreement,
                    }
                })
                .collect()
        };
        let contested = frames_of(&f.contested);
        let peak = contested.iter().max_by(|a, b| a.disagreement.total_cmp(&b.disagreement)).cloned();
        let all: Vec<usize> = if self.detail { (0..f.frames.len()).collect() } else { Vec::new() };
        Ok(TrexObservationReport {
            length: input.length(),
            path: input.path,
            peak: one(peak)?,
            frames: frames_of(&all),
            contested,
        })
    }
}

impl Cmdlet for MeasureTrexObservation {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| ps.write(self.report(input)?))
    }
}
