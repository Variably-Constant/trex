//! The analysis axes: readings of an input's structure that need no pattern,
//! one cmdlet an axis, carrying what the trex command's analysis commands
//! print.
//!
//! Each cmdlet writes one report an input: a string piped in, or each file
//! -Path names. A report holds the axis's summary readings and the points it
//! marks, each point the frame of its token or byte, so a point carries its
//! reading; -Detail adds every frame. Offsets are UTF-16 code units, as a
//! match's are, and a point inside a character is reported at the
//! character's offset. The readings are trex's own, as the
//! single-precision numbers it computes them in.
//!
//! An axis that reads tokens lexes with the atoms in force, the session's or
//! -Library's, so a declared shape or kind is one token of its own; with no
//! atoms declared its readings are the trex command's.

use pwrs::prelude::*;

use crate::atoms::{TrexLibrary, atoms_for};
use crate::common::{arg_err, units_of};
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

pub(crate) use trex::encoding::char_start;

/// UTF-16 offsets of byte offsets given in any order, found in one pass over
/// the text; a byte inside a character takes the character's offset.
pub(crate) fn utf16_offsets(bytes: &[u8], offsets: &[usize]) -> Vec<i64> {
    let starts: Vec<usize> = offsets.iter().map(|&b| char_start(bytes, b)).collect();
    let mut order: Vec<usize> = (0..starts.len()).collect();
    order.sort_by_key(|&i| starts[i]);
    let mut units = units_of(bytes);
    let mut out = vec![0i64; starts.len()];
    for i in order {
        out[i] = units.at(starts[i]) as i64;
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

/// The stream an axis reads: the bytes, the significant tokens, or the
/// supertokens they group into, named as a pattern's grain suffix names them.
#[psenum(name = "Trex.Grain")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TrexGrain {
    /// The bytes.
    #[default]
    Byte,
    /// The significant tokens, whitespace left out.
    Token,
    /// The supertokens: runs of tokens read as one unit with a structural role.
    Super,
}

/// A cut given to a parameter, or `default` where none is: a cut must be a
/// finite number, since `NaN` passes no point and an infinite cut every
/// point or none.
pub(crate) fn cut(parameter: &str, given: Option<f32>, default: f32) -> PsResult<f32> {
    match given {
        Some(v) if !v.is_finite() => Err(arg_err("TrexCut", format!("-{parameter} takes a finite number, not {v}"))),
        Some(v) => Ok(v),
        None => Ok(default),
    }
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
    /// The tokens whose magnitude differs from the one before by at least
    /// -JumpThreshold orders, three by default.
    pub jumps: Vec<TrexMagnitudeFrame>,
    /// The tokens whose magnitude is at least -OutlierSigma standard
    /// deviations from the input's mean, two and a half by default.
    pub outliers: Vec<TrexMagnitudeFrame>,
    /// Every token's frame, with -Detail.
    pub frames: Vec<TrexMagnitudeFrame>,
}

/// Reads the magnitude axis: each token's order of magnitude, its change
/// from the token before, and the energy of the window behind it.
///
/// A number's magnitude is log10 of its value and any other token's log2 of
/// its length. Jumps are the tokens where it changes by at least
/// -JumpThreshold orders from the token before, three by default; outliers
/// are the tokens at least -OutlierSigma standard deviations from the
/// input's mean, two and a half by default.
///
/// # Examples
/// Measure-TrexMagnitude 'size 12 then 12000000 then 3'
/// Measure-TrexMagnitude -Path ./metrics.log | Select-Object -ExpandProperty Jumps
/// (Measure-TrexMagnitude -Path ./metrics.log -Detail).Frames | Sort-Object Energy -Descending | Select-Object -First 3
/// Measure-TrexMagnitude -Path ./metrics.log -JumpThreshold 1.5 | Select-Object -ExpandProperty Jumps
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
    /// The change in orders of magnitude from the token before that makes a
    /// jump: three when absent.
    #[param]
    pub jump_threshold: Option<f32>,
    /// The standard deviations from the input's mean that make an outlier:
    /// two and a half when absent.
    #[param]
    pub outlier_sigma: Option<f32>,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
}

impl MeasureTrexMagnitude {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexMagnitudeReport> {
        let defaults = trex::magnitude::MagnitudeConfig::default();
        let cfg = trex::magnitude::MagnitudeConfig {
            jump_threshold: cut("JumpThreshold", self.jump_threshold, defaults.jump_threshold)?,
            outlier_sigma: cut("OutlierSigma", self.outlier_sigma, defaults.outlier_sigma)?,
            ..defaults
        };
        let bytes = input.text.as_bytes();
        let toks = lexed(bytes, shapes, true);
        let f = trex::magnitude::analyze_with(&toks, bytes, &cfg);
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
    /// The tokens at a local maximum of depth, at least -PeakMinDepth deep,
    /// two by default.
    pub peaks: Vec<TrexStressFrame>,
    /// The tokens where the depth starts to fall from a level at least
    /// -FractureMinDepth deep, two by default: where a nested structure
    /// begins to close.
    pub fractures: Vec<TrexStressFrame>,
    /// Every token's frame, with -Detail.
    pub frames: Vec<TrexStressFrame>,
}

/// Reads the stress axis: how deeply the paired brackets nest at each
/// token, how long the innermost has been held open, and how long all of
/// them together.
///
/// Peaks are the local maxima of depth at least -PeakMinDepth deep, and
/// fractures the tokens where the depth starts to fall from a level at least
/// -FractureMinDepth deep, both two by default. An unpaired bracket is a
/// character of the text and opens no level.
///
/// # Examples
/// Measure-TrexStress '{"a": [1, {"b": [2, 3]}]}'
/// Measure-TrexStress -Path ./config.json | Select-Object MaxDepth, PeakLoad
/// Measure-TrexStress -Path ./config.json -PeakMinDepth 4 | Select-Object -ExpandProperty Peaks
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
    /// The depth a local maximum must reach to be a peak: two when absent.
    #[param]
    pub peak_min_depth: Option<u32>,
    /// The depth a fall must start from to be a fracture: two when absent.
    #[param]
    pub fracture_min_depth: Option<u32>,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
}

/// A depth given to a parameter, or `default` where none is, within the
/// range a nesting depth is counted in.
fn depth(parameter: &str, given: Option<u32>, default: u16) -> PsResult<u16> {
    match given {
        Some(v) => u16::try_from(v).map_err(|_| arg_err("TrexDepth", format!("-{parameter} {v} is deeper than any nesting is counted, {}", u16::MAX))),
        None => Ok(default),
    }
}

impl MeasureTrexStress {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexStressReport> {
        let defaults = trex::stress::StressConfig::default();
        let cfg = trex::stress::StressConfig {
            peak_min_depth: depth("PeakMinDepth", self.peak_min_depth, defaults.peak_min_depth)?,
            fracture_min_depth: depth("FractureMinDepth", self.fracture_min_depth, defaults.fracture_min_depth)?,
        };
        let bytes = input.text.as_bytes();
        let toks = lexed(bytes, shapes, true);
        let f = trex::stress::analyze_with(&toks, bytes, &cfg);
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
    /// The signal's change per unit, averaged over the -Window units ending
    /// at this one, four by default.
    pub slope: f32,
    /// Which way the trend runs: Steady where the slope is within
    /// -SteadyBand of zero, 0.05 by default.
    pub direction: FlowDirection,
    /// How many consecutive tokens have held the same direction, rising or
    /// falling.
    pub momentum: i32,
}

/// One unit's analytic reading: the signal's position in its swing.
#[psclass(name = "Trex.AnalyticFrame", show = "{Text} at {Offset}, amplitude {Amplitude}")]
#[derive(Clone, Default)]
pub struct TrexAnalyticFrame {
    /// Where the unit starts, in UTF-16 code units.
    pub offset: i64,
    /// The unit's text.
    pub text: String,
    /// The envelope of the signal's swing around its running mean.
    pub amplitude: f32,
    /// The unit's position in the swing, in radians from -pi to pi.
    pub phase: f32,
    /// How fast the swing runs, in radians a unit: the phase's advance from
    /// the unit before.
    pub frequency: f32,
}

/// The flow axis over one input: the trend of a per-unit signal, how long
/// each run of it lasts, and where it turns.
#[psclass(name = "Trex.FlowReport")]
#[derive(Clone, Default)]
pub struct TrexFlowReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// How many tokens the input holds, whitespace left out.
    pub tokens: i64,
    /// The stream the trend was read over: Token, or Super for the
    /// supertokens.
    pub grain: TrexGrain,
    /// How many units the axis read at that grain.
    pub units: i64,
    /// The signal whose trend was read.
    pub signal: FlowSignal,
    /// The unit holding the longest run, a Trex.FlowFrame; `$null` for an
    /// input with no units.
    pub peak_momentum: PsObject,
    /// The units where the trend turns between rising and falling.
    pub reversals: Vec<TrexFlowFrame>,
    /// Every unit's frame, with -Detail.
    pub frames: Vec<TrexFlowFrame>,
    /// How many units past a unit its analytic reading uses, with
    /// -Analytic; `$null` without it. The last Latency units have no
    /// reading.
    pub latency: Option<i64>,
    /// Every unit's analytic reading that has one, with -Analytic.
    pub analytic: Vec<TrexAnalyticFrame>,
}

/// Reads the flow axis: the trend of a per-token signal, the magnitude by
/// default, how long it has run one way, and where it reverses.
///
/// -Signal Stress reads the trend of the nesting depth instead, and -Signal
/// Length that of the token lengths. -Grain Super reads it from supertoken to
/// supertoken. -Analytic adds the signal's position in its swing: its
/// envelope, its phase and how fast the swing runs.
///
/// # Examples
/// Measure-TrexFlow '1 10 100 1000 10 1'
/// Measure-TrexFlow -Path ./trace.log -Signal Stress | Select-Object -ExpandProperty Reversals
/// (Measure-TrexFlow -Path ./latency.txt -Analytic).Analytic | Sort-Object Amplitude -Descending | Select-Object -First 3
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
    /// The stream the trend is read over: Token when absent, or Super.
    #[param]
    pub grain: Option<TrexGrain>,
    /// Adds every unit's frame to the report.
    #[param]
    pub detail: bool,
    /// Adds every unit's analytic reading and the latency it carries.
    #[param]
    pub analytic: bool,
    /// How many units the slope is averaged over: four when absent.
    #[param]
    pub window: Option<u32>,
    /// How far from zero a slope reads as steady: 0.05 when absent.
    #[param]
    pub steady_band: Option<f32>,
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
        let grain = match self.grain {
            Some(TrexGrain::Byte) => {
                return Err(arg_err("TrexFlowGrain", "the flow axis reads tokens or supertokens; -Grain takes Token or Super"));
            }
            Some(g) => g,
            None => TrexGrain::Token,
        };
        let defaults = trex::flow::FlowConfig::default();
        let cfg = trex::flow::FlowConfig {
            window: self.window.map_or(defaults.window, |w| w as usize),
            steady_band: cut("SteadyBand", self.steady_band, defaults.steady_band)?,
        };
        let bytes = input.text.as_bytes();
        let sig = lexed(bytes, shapes, true);
        let (f, analytic) = match grain {
            TrexGrain::Super => {
                let toks = lexed(bytes, shapes, false);
                let f = trex::flow::analyze_supertokens_with(&toks, bytes, signal.trex(), &cfg);
                (f, self.analytic.then(|| trex::flow::analytic_supertokens(&toks, bytes, signal.trex())))
            }
            TrexGrain::Byte | TrexGrain::Token => {
                let f = trex::flow::analyze_with(&sig, bytes, signal.trex(), &cfg);
                (f, self.analytic.then(|| trex::flow::analytic(&sig, bytes, signal.trex())))
            }
        };
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
        // A unit's reading uses the signal `latency` units past it, so the
        // last `latency` units have none and carry no frame.
        let analytic_frames = match &analytic {
            Some(a) => {
                let read: Vec<usize> = (0..f.n_tokens.saturating_sub(a.latency)).collect();
                frames_at(bytes, &f.spans, &read, |i, offset, text| TrexAnalyticFrame {
                    offset,
                    text,
                    amplitude: a.amplitude[i],
                    phase: a.phase[i],
                    frequency: a.frequency[i],
                })
            }
            None => Vec::new(),
        };
        Ok(TrexFlowReport {
            length: input.length(),
            path: input.path,
            tokens: sig.len() as i64,
            grain,
            units: f.n_tokens as i64,
            signal,
            peak_momentum: one(peak)?,
            reversals: frames_at(bytes, &f.spans, &tokens_at(&f.spans, &f.reversals)?, frame),
            frames: frames_at(bytes, &f.spans, &all, frame),
            latency: analytic.as_ref().map(|a| a.latency as i64),
            analytic: analytic_frames,
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

/// One unit's observation reading: the entropy of the classes around it,
/// seen from behind, from ahead and from both sides.
#[psclass(name = "Trex.ObservationFrame", show = "at {Offset}, disagreement {Disagreement}")]
#[derive(Clone, Default)]
pub struct TrexObservationFrame {
    /// The unit's offset, in UTF-16 code units; each byte of a character
    /// written in several has a frame at the character's offset.
    pub offset: i64,
    /// The unit's text: at the byte grain the character the byte begins,
    /// empty for a byte inside a character written in several; the token or
    /// the supertoken otherwise.
    pub text: String,
    /// The class entropy of the window before the unit, from 0 to 1.
    pub causal: f32,
    /// The class entropy of the window after the unit, from 0 to 1.
    pub anticausal: f32,
    /// The class entropy of the window centered on the unit, from 0 to 1.
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
    /// The stream read: Byte, Token or Super for the supertokens.
    pub grain: TrexGrain,
    /// How many units the axis read at that grain.
    pub units: i64,
    /// The contested unit where the two sides differ most, a
    /// Trex.ObservationFrame; `$null` where none is contested.
    pub peak: PsObject,
    /// The units where the two sides differ most: local maxima of
    /// Disagreement at least -ContestedThreshold, 0.2 by default; at the
    /// byte grain at least -ContestedMinGap bytes apart, 8 by default, the
    /// stronger of two closer maxima kept.
    pub contested: Vec<TrexObservationFrame>,
    /// Every unit's frame, with -Detail.
    pub frames: Vec<TrexObservationFrame>,
}

/// Reads the observation axis: the entropy of the classes behind each unit,
/// ahead of it and around it, and the points where behind and ahead differ
/// most.
///
/// At the byte grain the entropies are of five byte classes, digits,
/// letters, whitespace, other characters and bytes that are not part of a
/// well-formed UTF-8 character, so they read the texture of the text and not
/// its words; a letter outside ASCII is a letter, and any other character
/// outside ASCII counts once. -Grain Token reads the sequence of
/// token kinds instead, and -Grain Super the sequence of supertoken roles,
/// each lexed with the atoms in force, the session's or -Library's; the byte
/// reading takes no atoms.
///
/// # Examples
/// Measure-TrexObservation 'key=value; key=other; odd'
/// Measure-TrexObservation -Path ./app.log | Select-Object -ExpandProperty Contested
/// Measure-TrexObservation -Path ./app.log -ContestedThreshold 0.4 | Select-Object -ExpandProperty Contested
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
    /// The stream read: Byte when absent, Token or Super.
    #[param]
    pub grain: Option<TrexGrain>,
    /// Adds every unit's frame to the report.
    #[param]
    pub detail: bool,
    /// The disagreement a contested point must reach, from 0 to 1: 0.2 when
    /// absent.
    #[param]
    pub contested_threshold: Option<f32>,
    /// The bytes between two contested points at the byte grain: 8 when
    /// absent.
    #[param]
    pub contested_min_gap: Option<u32>,
    /// The atoms that decide the lex at the Token and Super grains, in place
    /// of the session's; the byte grain reads bytes and takes none.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
}

/// Each byte's span as the character holding it, so a byte's frame is
/// reported at its character's offset.
fn character_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::with_capacity(text.len());
    for (s, c) in text.char_indices() {
        let e = s + c.len_utf8();
        spans.extend(std::iter::repeat_n((s, e), e - s));
    }
    spans
}

impl MeasureTrexObservation {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexObservationReport> {
        let grain = match self.grain {
            Some(g) => g,
            None => TrexGrain::Byte,
        };
        if grain == TrexGrain::Byte && self.library.is_some() {
            return Err(arg_err(
                "TrexObservationLibrary",
                "atoms decide how tokens are read, and the byte grain reads bytes; -Grain Token or Super reads them",
            ));
        }
        let defaults = trex::observation::ObservationConfig::default();
        let threshold = cut("ContestedThreshold", self.contested_threshold, defaults.contested_threshold)?;
        if !(0.0..=1.0).contains(&threshold) {
            return Err(arg_err(
                "TrexCut",
                format!("-ContestedThreshold {threshold} names no disagreement: a disagreement runs 0 to 1"),
            ));
        }
        if self.contested_min_gap.is_some() && grain != TrexGrain::Byte {
            return Err(arg_err(
                "TrexObservationGap",
                "-ContestedMinGap spaces the byte grain's contested points; the Token and Super grains keep every local maximum",
            ));
        }
        let cfg = trex::observation::ObservationConfig {
            contested_threshold: threshold,
            contested_min_gap: self.contested_min_gap.map_or(defaults.contested_min_gap, |g| g as usize),
            ..defaults
        };
        let bytes = input.text.as_bytes();
        let (f, spans) = match grain {
            TrexGrain::Byte => (trex::observation::analyze_with(bytes, &cfg), character_spans(&input.text)),
            TrexGrain::Token => {
                let sig = lexed(bytes, shapes, true);
                (trex::observation::analyze_tokens(&sig, &cfg), sig.iter().map(|t| (t.start(), t.end())).collect())
            }
            TrexGrain::Super => {
                let units = trex::supertoken::supertokens_from(&lexed(bytes, shapes, false), bytes);
                (trex::observation::analyze_supertokens(&units, &cfg), units.iter().map(|u| (u.start, u.end)).collect())
            }
        };
        let frame = |i: usize, offset: i64, text: String| {
            let r = &f.frames[i];
            let begins = grain != TrexGrain::Byte || spans[i].0 == i;
            TrexObservationFrame {
                offset,
                text: if begins { text } else { String::new() },
                causal: r.causal,
                anticausal: r.anticausal,
                centered: r.centered,
                disagreement: r.disagreement,
            }
        };
        let contested = frames_at(bytes, &spans, &f.contested, frame);
        let peak = contested.iter().max_by(|a, b| a.disagreement.total_cmp(&b.disagreement)).cloned();
        let all: Vec<usize> = if self.detail { (0..f.frames.len()).collect() } else { Vec::new() };
        Ok(TrexObservationReport {
            length: input.length(),
            path: input.path,
            grain,
            units: spans.len() as i64,
            peak: one(peak)?,
            frames: frames_at(bytes, &spans, &all, frame),
            contested,
        })
    }
}

impl Cmdlet for MeasureTrexObservation {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        self.shapes = Some(atoms_for(ps, &self.library)?);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shapes) = &self.shapes else {
            return Err(arg_err("TrexNoAtoms", "the atoms were not read"));
        };
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| ps.write(self.report(input, shapes)?))
    }
}

/// One unit's gravity reading: how its past pushes it away, how strongly the
/// input holds together across the cut before it, and its type's class.
#[psclass(name = "Trex.GravityFrame", show = "{Text} at {Offset}")]
#[derive(Clone, Default)]
pub struct TrexGravityFrame {
    /// The unit's offset, in UTF-16 code units; each byte of a character
    /// written in several has a frame at the character's offset.
    pub offset: i64,
    /// The unit's text: at the byte grain the character the byte begins,
    /// empty for a byte inside a character written in several; the token or
    /// the supertoken otherwise.
    pub text: String,
    /// The unit's mean potential against each unit before it within the
    /// field's reach, in bits: high where what came before pushes it away.
    /// `$null` at the first unit, which has nothing before it.
    pub strain: Option<f32>,
    /// The share of the input's strain readings below this one, in percent.
    pub strain_percentile: Option<f64>,
    /// The attraction across the cut before the unit, the mean log2 g over
    /// the pairs straddling it within 8 units, in bits per pair: low where the
    /// two sides hold together least. `$null` at the first unit, which has no
    /// cut before it.
    pub bound: Option<f32>,
    /// The share of the input's bound readings below this one, in percent.
    pub bound_percentile: Option<f64>,
    /// The gravity class of the unit's type; `$null` where no significant
    /// cell places it, so it is kin to nothing but itself.
    pub class: Option<i32>,
}

/// One gravity class: types the pair field places together, kin to one
/// another under @kin.
#[psclass(name = "Trex.GravityClass", show = "class {Class}, {Units} units")]
#[derive(Clone, Default)]
pub struct TrexGravityClass {
    /// The class's number, from 0 to 15.
    pub class: i32,
    /// Its types, most frequent first: at the byte grain each byte, as itself
    /// where it is printable ASCII, ' ' for a space and an escape otherwise;
    /// a token's kind with a punctuation mark's text; a supertoken's role and
    /// kinds.
    pub types: Vec<String>,
    /// How many of the grain's units are of its types.
    pub units: i64,
}

/// The pair field over one input: where each unit's past pushes it away,
/// where the input holds together least, and which types it treats alike.
#[psclass(name = "Trex.GravityReport")]
#[derive(Clone, Default)]
pub struct TrexGravityReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// The stream read: Token, Byte or Super for the supertokens.
    pub grain: TrexGrain,
    /// How many units the field read at that grain.
    pub units: i64,
    /// How many distinct types those units are of.
    pub types: i64,
    /// The units under the most strain, most first, -Top of them.
    pub strained: Vec<TrexGravityFrame>,
    /// The units before the cuts held together least, least first, -Top of
    /// them.
    pub weakest: Vec<TrexGravityFrame>,
    /// The classes the field places types in, ascending.
    pub classes: Vec<TrexGravityClass>,
    /// Every unit's frame, with -Detail.
    pub frames: Vec<TrexGravityFrame>,
}

/// Reads the pair field: how much more or less often each type of unit
/// follows another at each gap than chance alone would, and three readings
/// taken from it.
///
/// A unit's Strain is its mean potential against the units before it, high
/// where its past pushes it away; the Bound of the cut before it is the
/// attraction across that cut, low where the input holds together least; and
/// its Class groups the types the field treats alike. -Grain Token reads the
/// significant tokens, Byte the bytes and Super the supertokens, the token and
/// super grains lexed with the atoms in force, the session's or -Library's;
/// the byte grain takes no atoms. -Top sizes the lists of the most strained
/// units and the weakest cuts.
///
/// # Examples
/// Measure-TrexGravity 'let x = f(a) ; let y = g(b) ; print x'
/// Measure-TrexGravity -Path ./app.log | Select-Object -ExpandProperty Weakest
/// Measure-TrexGravity -Path ./app.log -Grain Byte -Detail | Sort-Object Strain -Descending | Select-Object -First 5
#[cmdlet(verb = "Measure", noun = "TrexGravity", alias = "Measure-TxGravity", default_parameter_set = "Text", output = ["Trex.GravityReport"])]
#[derive(Default)]
pub struct MeasureTrexGravity {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The stream read: Token when absent, Byte or Super.
    #[param]
    pub grain: Option<TrexGrain>,
    /// How many of the most strained units and the weakest cuts the report
    /// lists: 8 when absent.
    #[param]
    pub top: Option<u32>,
    /// Adds every unit's frame to the report.
    #[param]
    pub detail: bool,
    /// The atoms that decide the lex at the Token and Super grains, in place
    /// of the session's; the byte grain reads bytes and takes none.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
}

impl MeasureTrexGravity {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexGravityReport> {
        let grain = match self.grain {
            Some(g) => g,
            None => TrexGrain::Token,
        };
        if grain == TrexGrain::Byte && self.library.is_some() {
            return Err(arg_err(
                "TrexGravityLibrary",
                "atoms decide how tokens are read, and the byte grain reads bytes; -Grain Token or Super reads them",
            ));
        }
        let top = self.top.map_or(8, |t| t as usize);
        let bytes = input.text.as_bytes();
        let unit = match grain {
            TrexGrain::Byte => trex::ast::Grain::Byte,
            TrexGrain::Token => trex::ast::Grain::Token,
            TrexGrain::Super => trex::ast::Grain::Super,
        };
        let toks = if grain == TrexGrain::Byte { Vec::new() } else { lexed(bytes, shapes, false) };
        let r = trex::gravity::Readings::read(unit, bytes, &toks);
        // Each unit's span; a byte's is the character holding it, so a byte's
        // frame is reported at its character's offset.
        let spans: Vec<(usize, usize)> = match grain {
            TrexGrain::Byte => character_spans(&input.text),
            TrexGrain::Token | TrexGrain::Super => (0..r.len()).map(|u| r.span_of(u)).collect(),
        };
        let frame = |u: usize, offset: i64, text: String| {
            let reading = r.reading_of(u);
            let begins = grain != TrexGrain::Byte || spans[u].0 == u;
            TrexGravityFrame {
                offset,
                text: if begins { text } else { String::new() },
                strain: reading.strain,
                strain_percentile: reading.strain_percentile,
                bound: reading.bound,
                bound_percentile: reading.bound_percentile,
                class: r.class_of(r.type_of(u)).map(i32::from),
            }
        };
        let classes = r
            .classes()
            .into_iter()
            .map(|g| TrexGravityClass {
                class: i32::from(g.class),
                types: g.types.iter().map(|&t| r.type_label(t)).collect(),
                units: g.units as i64,
            })
            .collect();
        let all: Vec<usize> = if self.detail { (0..r.len()).collect() } else { Vec::new() };
        Ok(TrexGravityReport {
            length: input.length(),
            path: input.path,
            grain,
            units: r.len() as i64,
            types: r.types() as i64,
            strained: frames_at(bytes, &spans, &r.most_strained(top), frame),
            weakest: frames_at(bytes, &spans, &r.weakest_cuts(top), frame),
            classes,
            frames: frames_at(bytes, &spans, &all, frame),
        })
    }
}

impl Cmdlet for MeasureTrexGravity {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        self.shapes = Some(atoms_for(ps, &self.library)?);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shapes) = &self.shapes else {
            return Err(arg_err("TrexNoAtoms", "the atoms were not read"));
        };
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| ps.write(self.report(input, shapes)?))
    }
}

/// The context a token is read against, as `\N{>+1:F}` names it, and the unit
/// rung's two.
#[psenum(name = "Trex.ContextFold")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TrexContextFold {
    /// The window of tokens before the token.
    #[default]
    Window,
    /// The supertoken holding the token.
    Unit,
    /// The window of supertokens ending at the one holding the token.
    Units,
    /// The heads of the brackets enclosing the token.
    Enclosing,
    /// The earlier occurrences of the token's key.
    Echo,
    /// The tokens since the last spectral change point.
    Regime,
    /// The earlier tokens at the token's column of the record period.
    Phase,
    /// The values the token's key was bound to before.
    Key,
}

/// A context's scale: the orders of magnitude of the tokens it folds.
#[psclass(name = "Trex.ContextMagnitude", show = "mean {Mean}, spread {Spread}, max {Max}")]
#[derive(Clone, Default)]
pub struct TrexContextMagnitude {
    /// Their mean.
    pub mean: f32,
    /// Their standard deviation, the spread an `s` predicate counts in.
    pub spread: f32,
    /// The largest.
    pub max: f32,
}

/// A context's load: the bracket depth of the tokens it folds.
#[psclass(name = "Trex.ContextStress", show = "mean {Mean}, max {Max}")]
#[derive(Clone, Default)]
pub struct TrexContextStress {
    /// The mean depth.
    pub mean: f32,
    /// The deepest.
    pub max: i32,
}

/// A context's texture: the spectral reading of the tokens it folds.
#[psclass(name = "Trex.ContextSpectral", show = "entropy {Entropy}, period {Period}")]
#[derive(Clone, Default)]
pub struct TrexContextSpectral {
    /// The mean pooled byte-class entropy, from 0 to 1.
    pub entropy: f32,
    /// The byte period of the token whose period repeats most strongly; 0
    /// for none.
    pub period: i32,
    /// How strongly that period repeats, from 0 to 1.
    pub strength: f32,
}

/// A context's recurrence: how many of the tokens it folds are first
/// sightings and how many recur.
#[psclass(name = "Trex.ContextEcho", show = "novel {Novel}, echoed {Echoed}")]
#[derive(Clone, Default)]
pub struct TrexContextEcho {
    /// Tokens whose content occurs here for the first time.
    pub novel: i64,
    /// Tokens whose content recurs elsewhere in the input.
    pub echoed: i64,
}

/// A context's vantage: what the bytes before, after and around the tokens it
/// folds read, and how far before and after disagree.
#[psclass(name = "Trex.ContextObservation", show = "disagreement {Disagreement}, contested {Contested}")]
#[derive(Clone, Default)]
pub struct TrexContextObservation {
    /// The mean reading of the window before each token, from 0 to 1.
    pub causal: f32,
    /// The mean reading of the window after each token, from 0 to 1.
    pub anticausal: f32,
    /// The mean reading of the window centered on each token, from 0 to 1.
    pub centered: f32,
    /// The mean disagreement between before and after.
    pub disagreement: f32,
    /// The largest disagreement.
    pub disagreement_max: f32,
    /// Tokens spanning a contested point.
    pub contested: i64,
}

/// A context's segmentation: how strongly the tokens it folds align with
/// seams, read in both directions.
#[psclass(name = "Trex.ContextSeam", show = "strength {Strength}, cuts {Cuts}")]
#[derive(Clone, Default)]
pub struct TrexContextSeam {
    /// The mean forward branching entropy.
    pub forward: f32,
    /// The mean backward branching entropy.
    pub backward: f32,
    /// The mean seam strength.
    pub strength: f32,
    /// The strongest seam.
    pub strength_max: f32,
    /// Tokens starting a segment.
    pub cuts: i64,
}

/// A context's dynamics: the trend of magnitude over the tokens it folds.
#[psclass(name = "Trex.ContextFlow", show = "slope {Slope}, reversals {Reversals}")]
#[derive(Clone, Default)]
pub struct TrexContextFlow {
    /// The mean windowed slope.
    pub slope: f32,
    /// Tokens whose trend rises.
    pub rising: i64,
    /// Tokens whose trend falls.
    pub falling: i64,
    /// Tokens whose trend is steady.
    pub steady: i64,
    /// The longest run any token's trend has kept its direction.
    pub momentum: i32,
    /// Changes of direction between adjacent tokens.
    pub reversals: i64,
}

/// One token's context: how many tokens it folds and each axis's reading over
/// them.
#[psclass(name = "Trex.ContextFrame", show = "{Text} at {Offset}, over {Over}")]
#[derive(Clone, Default)]
pub struct TrexContextFrame {
    /// Where the token starts, in UTF-16 code units.
    pub offset: i64,
    /// The token's text.
    pub text: String,
    /// The token's column of the record period; `$null` in a stream with no
    /// period.
    pub phase: Option<i32>,
    /// How many tokens the context folds; 0 where it holds none, and then
    /// every axis below is `$null`.
    pub over: i64,
    /// Their scale, a Trex.ContextMagnitude.
    pub magnitude: PsObject,
    /// Their bracket depth, a Trex.ContextStress.
    pub stress: PsObject,
    /// Their texture, a Trex.ContextSpectral.
    pub spectral: PsObject,
    /// Their recurrence, a Trex.ContextEcho.
    pub echo: PsObject,
    /// Their vantage, a Trex.ContextObservation.
    pub observation: PsObject,
    /// Their segmentation, a Trex.ContextSeam.
    pub seam: PsObject,
    /// Their dynamics, a Trex.ContextFlow.
    pub flow: PsObject,
}

/// How consistently each lower grain's boundaries are at one offset from the
/// starts of supertokens of one role: the share at their role's most common
/// offset, from 0 to 1.
#[psclass(name = "Trex.ContextAlignment", show = "regime {Regime}, shape {Shape}, seam {Seam}")]
#[derive(Clone, Default)]
pub struct TrexContextAlignment {
    /// The spectral change points.
    pub regime: f32,
    /// The shape change points.
    pub shape: f32,
    /// The seam cuts.
    pub seam: f32,
}

/// One supertoken's role and how far each lower grain's nearest boundary is
/// from its start, in significant tokens: negative before it.
#[psclass(name = "Trex.ContextAgreement", show = "{Role} at {Offset}")]
#[derive(Clone, Default)]
pub struct TrexContextAgreement {
    /// Where the supertoken starts, in UTF-16 code units.
    pub offset: i64,
    /// Its text.
    pub text: String,
    /// Its role: call, assign, kv, list, numeric or plain.
    pub role: String,
    /// The nearest spectral change point; `$null` where there is none.
    pub regime: Option<i32>,
    /// The nearest shape change point; `$null` where there is none.
    pub shape: Option<i32>,
    /// The nearest seam cut; `$null` where there is none.
    pub seam: Option<i32>,
}

/// The context axis over one input: what each token is read against.
#[psclass(name = "Trex.ContextReport")]
#[derive(Clone, Default)]
pub struct TrexContextReport {
    /// The file read, empty for a string.
    pub path: String,
    /// The input's length in UTF-16 code units.
    pub length: i64,
    /// The context read.
    pub fold: TrexContextFold,
    /// How many tokens the axis read, whitespace left out.
    pub tokens: i64,
    /// How many supertokens they form.
    pub units: i64,
    /// The record period in tokens, the lag the stream's token shapes repeat
    /// at most; `$null` where none clears chance.
    pub period: Option<i32>,
    /// Every lag that clears chance as the period does, strongest first.
    pub live_periods: Vec<i32>,
    /// How consistently the lower grains' boundaries line up with the
    /// supertokens, a Trex.ContextAlignment.
    pub alignment: PsObject,
    /// Every supertoken's role and nearest boundaries, with -Detail.
    pub agreement: Vec<TrexContextAgreement>,
    /// Every token's context, with -Detail.
    pub frames: Vec<TrexContextFrame>,
}

/// Reads the context axis: each token read against what surrounds it.
///
/// -Fold names the context as `\N{>+1:F}` names it: Window, the tokens before
/// it; Unit, the supertoken holding it; Units, the window of supertokens
/// ending there; Enclosing, the heads of the brackets around it; Echo, its
/// key's earlier occurrences; Regime, the tokens since the texture last
/// changed; Phase, the earlier tokens at its column of the record period; Key,
/// the values its key was bound to before. Every frame carries each axis's
/// reading over the tokens its context folds. -TokenWindow and -UnitWindow set
/// the windows, 32 tokens and 8 supertokens when absent, and the lex reads the
/// atoms in force, the session's or -Library's.
///
/// # Examples
/// Measure-TrexContext 'latency = 100 ; latency = 120 ; latency = 12000 ;' -Fold Key -Detail
/// Measure-TrexContext -Path ./app.log | Select-Object Period, LivePeriods, Alignment
#[cmdlet(verb = "Measure", noun = "TrexContext", alias = "Measure-TxContext", default_parameter_set = "Text", output = ["Trex.ContextReport"])]
#[derive(Default)]
pub struct MeasureTrexContext {
    /// The text to read; each string piped in is read on its own.
    #[param(mandatory, position = 0, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Files or directories to read; wildcards expand.
    #[param(mandatory, set = "Path")]
    pub path: Vec<String>,
    /// Files or directories to read, as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The context read: Window when absent.
    #[param]
    pub fold: Option<TrexContextFold>,
    /// The significant tokens a window folds: 32 when absent. The window
    /// before a token is the one ending at the token before it.
    #[param]
    pub token_window: Option<u32>,
    /// The supertokens the unit window folds: 8 when absent.
    #[param]
    pub unit_window: Option<u32>,
    /// Adds every token's context and every supertoken's agreement to the
    /// report.
    #[param]
    pub detail: bool,
    /// The atoms that decide the lex, in place of the session's.
    #[param]
    pub library: Option<PsProxy<TrexLibrary>>,
    shapes: Option<trex::ShapeSet>,
}

impl MeasureTrexContext {
    fn report(&self, input: Input, shapes: &trex::ShapeSet) -> PsResult<TrexContextReport> {
        use trex::context::Fold;
        let fold = match self.fold {
            Some(f) => f,
            None => TrexContextFold::Window,
        };
        let which = match fold {
            TrexContextFold::Window => Fold::Window,
            TrexContextFold::Unit => Fold::Unit,
            TrexContextFold::Units => Fold::Units,
            TrexContextFold::Enclosing => Fold::Enclosing,
            TrexContextFold::Echo => Fold::Echo,
            TrexContextFold::Regime => Fold::Regime,
            TrexContextFold::Phase => Fold::Phase,
            TrexContextFold::Key => Fold::Key,
        };
        let defaults = trex::context::ContextConfig::default();
        let cfg = trex::context::ContextConfig {
            token_window: self.token_window.map_or(defaults.token_window, |w| w as usize),
            unit_window: self.unit_window.map_or(defaults.unit_window, |w| w as usize),
        };
        if cfg.token_window == 0 || cfg.unit_window == 0 {
            return Err(arg_err(
                "TrexContextWindow",
                "a window folds at least one unit: -TokenWindow and -UnitWindow take 1 or more",
            ));
        }
        let bytes = input.text.as_bytes();
        let toks = lexed(bytes, shapes, false);
        let c = trex::context::Contexts::read(&toks, bytes, &cfg);
        let sig: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].is_significant()).collect();
        let spans: Vec<(usize, usize)> = toks.iter().map(|t| (t.start(), t.end())).collect();
        let frame = |t: usize, offset: i64, text: String| -> PsResult<TrexContextFrame> {
            let p = c.at(which, t);
            let some = p.magnitude.count > 0;
            Ok(TrexContextFrame {
                offset,
                text,
                phase: c.phase_of(t).map(i32::from),
                over: i64::from(p.magnitude.count),
                magnitude: one(some.then(|| TrexContextMagnitude {
                    mean: p.magnitude.mean(),
                    spread: p.magnitude.std_dev(),
                    max: p.magnitude.max,
                }))?,
                stress: one(some.then(|| TrexContextStress {
                    mean: p.stress.mean_depth(),
                    max: i32::from(p.stress.max_depth),
                }))?,
                spectral: one(some.then(|| TrexContextSpectral {
                    entropy: p.spectral.mean_entropy(),
                    period: i32::from(p.spectral.period),
                    strength: p.spectral.period_strength,
                }))?,
                echo: one(some.then(|| TrexContextEcho {
                    novel: i64::from(p.echo.novel),
                    echoed: i64::from(p.echo.echoed),
                }))?,
                observation: one(some.then(|| TrexContextObservation {
                    causal: p.observation.mean_causal(),
                    anticausal: p.observation.mean_anticausal(),
                    centered: p.observation.mean_centered(),
                    disagreement: p.observation.mean_disagreement(),
                    disagreement_max: p.observation.disagreement_max,
                    contested: i64::from(p.observation.contested),
                }))?,
                seam: one(some.then(|| TrexContextSeam {
                    forward: p.seam.mean_fwd(),
                    backward: p.seam.mean_bwd(),
                    strength: p.seam.mean_strength(),
                    strength_max: p.seam.strength_max,
                    cuts: i64::from(p.seam.cuts),
                }))?,
                flow: one(some.then(|| TrexContextFlow {
                    slope: p.flow.mean_slope(),
                    rising: i64::from(p.flow.rising),
                    falling: i64::from(p.flow.falling),
                    steady: i64::from(p.flow.steady),
                    momentum: i32::from(p.flow.momentum_max),
                    reversals: i64::from(p.flow.reversals),
                }))?,
            })
        };
        let every: Vec<usize> = if self.detail { sig.clone() } else { Vec::new() };
        let frames = frames_at(bytes, &spans, &every, frame).into_iter().collect::<PsResult<Vec<_>>>()?;
        let unit_spans: Vec<(usize, usize)> = c.supers.units.iter().map(|u| (u.start, u.end)).collect();
        let units: Vec<usize> = if self.detail { (0..unit_spans.len()).collect() } else { Vec::new() };
        let agreement = frames_at(bytes, &unit_spans, &units, |u, offset, text| {
            let a = &c.field.agreement[u];
            TrexContextAgreement {
                offset,
                text,
                role: a.role.label().to_string(),
                regime: a.regime,
                shape: a.shape,
                seam: a.seam,
            }
        });
        let (regime, shape, seam) = c.field.alignment();
        Ok(TrexContextReport {
            length: input.length(),
            path: input.path,
            fold,
            tokens: sig.len() as i64,
            units: c.supers.units.len() as i64,
            period: c.related.period.map(i32::from),
            live_periods: c.live_periods.iter().map(|&p| i32::from(p)).collect(),
            alignment: TrexContextAlignment { regime, shape, seam }.into_ps()?,
            agreement,
            frames,
        })
    }
}

impl Cmdlet for MeasureTrexContext {
    fn begin(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        self.shapes = Some(atoms_for(ps, &self.library)?);
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shapes) = &self.shapes else {
            return Err(arg_err("TrexNoAtoms", "the atoms were not read"));
        };
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| ps.write(self.report(input, shapes)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every byte takes the offset of the character holding it, over
    /// characters of one, two, three and four bytes, the last a surrogate
    /// pair in UTF-16, and the text's end at its length.
    #[test]
    fn a_byte_inside_a_character_takes_the_character_s_offset() {
        let text = "a\u{e9}\u{20ac}\u{1f600}b";
        let all: Vec<usize> = (0..=text.len()).collect();
        assert_eq!(utf16_offsets(text.as_bytes(), &all), [0, 1, 1, 2, 2, 2, 3, 3, 3, 3, 5, 6]);
    }
}
