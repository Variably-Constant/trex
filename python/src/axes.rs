//! `trex.axes`: readings of an input's structure that need no pattern, one
//! function an axis, each giving a report with the fields PowerShell's
//! Measure-Trex cmdlets write. An axis reads the text positionally, a str or
//! bytes, or `path=`, a file decoded as a scan reads one. Offsets are in the
//! input's units, characters of a str or a file's text and bytes of bytes,
//! and a point inside a character is reported at the character's offset; a text
//! taken from the input comes back in the input's type. The token axes take
//! `lib=`, the declarations that decide the lex, and observation and gravity
//! take it at their token and super grains; spectral, seam and the byte grain
//! of observation and gravity read bytes and take none.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use pyo3::IntoPyObjectExt;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyString};

use trex::encoding::{OffsetUnit, Offsets};

/// A frozen result class of `trex.axes`: a getter for each field, a
/// `__repr__` showing the fields `repr(...)` names, and `to_dict()` giving
/// every field by name.
macro_rules! axis_class {
    (
        $(#[$meta:meta])*
        $name:ident repr($($shown:ident),+) {
            $($(#[$fmeta:meta])* $field:ident: $ty:ty,)+
        }
    ) => {
        $(#[$meta])*
        #[pyclass(frozen, module = "trex.axes")]
        pub(crate) struct $name {
            $($(#[$fmeta])* #[pyo3(get)] $field: $ty,)+
        }

        #[pymethods]
        impl $name {
            /// The object as a plain dict of its fields, each object among
            /// them as a dict of its own.
            fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
                let d = PyDict::new(py);
                $(d.set_item(stringify!($field), plain((&self.$field).into_bound_py_any(py)?)?)?;)+
                Ok(d)
            }

            fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
                let shown = [$(format!(
                    concat!(stringify!($shown), "={}"),
                    (&self.$shown).into_bound_py_any(py)?.repr()?
                )),+];
                Ok(format!(concat!(stringify!($name), "({})"), shown.join(", ")))
            }
        }
    };
}

/// `value` with each object among it as its `to_dict()`, a list item by
/// item.
fn plain(value: Bound<'_, PyAny>) -> PyResult<Bound<'_, PyAny>> {
    let py = value.py();
    if value.is_instance_of::<PyList>() {
        let items = value.cast::<PyList>()?.iter().map(plain).collect::<PyResult<Vec<_>>>()?;
        return Ok(PyList::new(py, items)?.into_any());
    }
    if value.hasattr("to_dict")? {
        return value.call_method0("to_dict");
    }
    Ok(value)
}

/// One input an axis reads: its bytes, the file's name where it is one, and
/// whether it was bytes, which its offsets then count.
struct AxisInput {
    bytes: Vec<u8>,
    path: Option<String>,
    as_bytes: bool,
}

impl AxisInput {
    /// The text positionally, a str or bytes, or the file at `path`, decoded
    /// as a scan reads one and read as the str it decodes to.
    fn read(py: Python<'_>, text: Option<&Bound<'_, PyAny>>, path: Option<PathBuf>) -> PyResult<AxisInput> {
        let as_bytes = text.is_some_and(|t| t.is_instance_of::<PyBytes>());
        let (bytes, path) = crate::tools::one_input(py, text, path)?;
        let bytes = match (&path, String::from_utf8(bytes)) {
            (_, Ok(s)) => s.into_bytes(),
            (Some(_), Err(e)) => String::from_utf8_lossy(e.as_bytes()).into_owned().into_bytes(),
            (None, Err(e)) => e.into_bytes(),
        };
        Ok(AxisInput { bytes, path, as_bytes })
    }

    /// The unit the input's offsets count: bytes of bytes, characters of a
    /// str or a file's text.
    fn unit(&self) -> OffsetUnit {
        if self.as_bytes { OffsetUnit::Bytes } else { OffsetUnit::CodePoints }
    }

    /// The offsets of the input's bytes in its own units, from its start.
    fn units(&self) -> Offsets<'_> {
        Offsets::new(&self.bytes, self.unit())
    }

    /// The input's length in its own units.
    fn length(&self) -> usize {
        self.unit().count(&self.bytes)
    }

    /// Byte `b`, or where the character holding it starts when the input is
    /// text and `b` is inside one.
    fn snapped(&self, b: usize) -> usize {
        if self.as_bytes { b } else { trex::encoding::char_start(&self.bytes, b) }
    }

    /// The offsets in the input's units of byte offsets given in any order,
    /// found in one pass over the text; a byte inside a character takes the
    /// character's offset.
    fn offsets(&self, at: &[usize]) -> Vec<usize> {
        let starts: Vec<usize> = at.iter().map(|&b| self.snapped(b)).collect();
        let mut order: Vec<usize> = (0..starts.len()).collect();
        order.sort_by_key(|&i| starts[i]);
        let mut units = self.units();
        let mut out = vec![0usize; starts.len()];
        for i in order {
            out[i] = units.at(starts[i]);
        }
        out
    }

    /// `piece` of the input, in the input's type.
    fn text(&self, py: Python<'_>, piece: &[u8]) -> Py<PyAny> {
        if self.as_bytes {
            PyBytes::new(py, piece).into_any().unbind()
        } else {
            PyString::new(py, &String::from_utf8_lossy(piece)).into_any().unbind()
        }
    }

    /// The text of a byte span, in the input's type.
    fn piece(&self, py: Python<'_>, (s, e): (usize, usize)) -> Py<PyAny> {
        self.text(py, &self.bytes[s..e])
    }

    /// How long a byte span is in the input's units.
    fn span_length(&self, (s, e): (usize, usize)) -> usize {
        self.unit().count(&self.bytes[s..e])
    }

    /// The frames of the tokens at `indices`, each built by `frame` from the
    /// token's index, its offset in the input's units and its text.
    fn frames_at<T>(
        &self,
        py: Python<'_>,
        spans: &[(usize, usize)],
        indices: &[usize],
        frame: impl Fn(usize, usize, Py<PyAny>) -> PyResult<T>,
    ) -> PyResult<Vec<T>> {
        let starts: Vec<usize> = indices.iter().map(|&i| spans[i].0).collect();
        let offsets = self.offsets(&starts);
        indices.iter().zip(offsets).map(|(&i, offset)| frame(i, offset, self.piece(py, spans[i]))).collect()
    }
}

/// The tokens of `bytes` as the declarations in `shapes` lex them: every
/// token, or only the significant ones, which leaves out whitespace.
fn lexed(bytes: &[u8], shapes: &trex::ShapeSet, significant: bool) -> Vec<trex::token::Token> {
    let toks = if shapes.is_empty() {
        trex::lexer::lex(bytes)
    } else {
        trex::lexer::lex_with_shapes(bytes, &trex::lexer::blob_runs(bytes), shapes, 0)
    };
    if significant { toks.into_iter().filter(trex::token::Token::is_significant).collect() } else { toks }
}

/// The index of the token that begins at byte `at`, among spans sorted by
/// where they begin. A point an axis marks is where a token begins, so a
/// byte that begins none is a reading this cannot place, raised rather than
/// dropped.
fn token_at(spans: &[(usize, usize)], at: usize) -> PyResult<usize> {
    let i = spans.partition_point(|&(s, _)| s < at);
    match spans.get(i) {
        Some(&(s, _)) if s == at => Ok(i),
        _ => Err(PyRuntimeError::new_err(format!("trex marked byte {at}, which begins no token the axis read"))),
    }
}

/// The indices of the tokens `points` mark, in the order given.
fn tokens_at(spans: &[(usize, usize)], points: &[usize]) -> PyResult<Vec<usize>> {
    points.iter().map(|&b| token_at(spans, b)).collect()
}

/// Every index below `n` where `detail` asks for every frame, and none
/// otherwise.
fn all(detail: bool, n: usize) -> Vec<usize> {
    if detail { (0..n).collect() } else { Vec::new() }
}

/// A cut given to a keyword, or `default` where none is: a cut must be a
/// finite number, since `nan` passes no point and an infinite cut every
/// point or none.
fn cut(keyword: &str, given: Option<f32>, default: f32) -> PyResult<f32> {
    match given {
        Some(v) if !v.is_finite() => Err(PyValueError::new_err(format!("{keyword}= takes a finite number, not {v}"))),
        Some(v) => Ok(v),
        None => Ok(default),
    }
}

/// A symmetry group named as the `trex` command names one, with the name as
/// given.
fn group_of(name: &str) -> PyResult<(String, trex::orbit::OrbitGroup)> {
    let name = name.trim();
    match trex::orbit::OrbitGroup::parse(name) {
        Some(g) => Ok((name.to_string(), g)),
        None => Err(PyValueError::new_err(format!(
            "group={name:?} names no group: use identity, case, notation, shape, e8, ip, url, time, path, fold, \
             numeric, or a typed relation such as subnet/24, domain or day"
        ))),
    }
}

axis_class! {
    /// One token's magnitude reading.
    MagnitudeFrame repr(offset, text, magnitude) {
        /// Where the token starts, in the input's units.
        offset: usize,
        /// The token's text, in the input's type.
        text: Py<PyAny>,
        /// The token's order of magnitude: log10 of a number's absolute
        /// value, log2 of any other token's length in bytes.
        magnitude: f32,
        /// The change in magnitude from the token before.
        gradient: f32,
        /// The sum of squared magnitudes over the 16 tokens ending at this
        /// one.
        energy: f32,
    }
}

axis_class! {
    /// The magnitude axis over one input: the order of magnitude each token
    /// carries, where it jumps, and the tokens far from the input's mean.
    MagnitudeReport repr(path, tokens, total_energy) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// How many tokens the axis read, whitespace left out.
        tokens: usize,
        /// The sum of squared magnitudes over the whole input.
        total_energy: f32,
        /// The token of greatest magnitude; `None` for an input with no
        /// tokens.
        peak: Option<Py<MagnitudeFrame>>,
        /// The tokens whose magnitude differs from the one before by at least
        /// `jump_threshold` orders, three by default.
        jumps: Vec<Py<MagnitudeFrame>>,
        /// The tokens whose magnitude is at least `outlier_sigma` standard
        /// deviations from the input's mean, two and a half by default.
        outliers: Vec<Py<MagnitudeFrame>>,
        /// Every token's frame, with `detail=True`.
        frames: Vec<Py<MagnitudeFrame>>,
    }
}

/// The magnitude axis: each token's order of magnitude, its change from the
/// token before, and the energy of the window behind it. A number's
/// magnitude is log10 of its value and any other token's log2 of its
/// length. Jumps are the tokens where it changes by at least
/// `jump_threshold` orders from the token before, three by default;
/// outliers the tokens at least `outlier_sigma` standard deviations from
/// the input's mean, two and a half by default. `lib=` lexes under a
/// library's declarations; `detail=True` adds every token's frame.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, lib = None, detail = false, jump_threshold = None, outlier_sigma = None))]
pub(crate) fn magnitude(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    lib: Option<&Bound<'_, PyAny>>,
    detail: bool,
    jump_threshold: Option<f32>,
    outlier_sigma: Option<f32>,
) -> PyResult<MagnitudeReport> {
    let defaults = trex::magnitude::MagnitudeConfig::default();
    let cfg = trex::magnitude::MagnitudeConfig {
        jump_threshold: cut("jump_threshold", jump_threshold, defaults.jump_threshold)?,
        outlier_sigma: cut("outlier_sigma", outlier_sigma, defaults.outlier_sigma)?,
        ..defaults
    };
    let shapes = crate::shapes_of(lib)?;
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let f = py.detach(|| trex::magnitude::analyze_with(&lexed(bytes, &shapes, true), bytes, &cfg));
    let frame = |i: usize, offset: usize, text: Py<PyAny>| {
        let r = &f.frames[i];
        Py::new(py, MagnitudeFrame { offset, text, magnitude: r.magnitude, gradient: r.gradient, energy: r.energy })
    };
    let peak = f.frames.iter().enumerate().max_by(|a, b| a.1.magnitude.total_cmp(&b.1.magnitude)).map(|(i, _)| i);
    Ok(MagnitudeReport {
        length: input.length(),
        tokens: f.n_tokens,
        total_energy: f.total_energy,
        peak: input.frames_at(py, &f.spans, &Vec::from_iter(peak), frame)?.pop(),
        jumps: input.frames_at(py, &f.spans, &tokens_at(&f.spans, &f.jumps)?, frame)?,
        outliers: input.frames_at(py, &f.spans, &f.outliers(), frame)?,
        frames: input.frames_at(py, &f.spans, &all(detail, f.frames.len()), frame)?,
        path: input.path,
    })
}

axis_class! {
    /// One token's stress reading.
    StressFrame repr(offset, text, depth) {
        /// Where the token starts, in the input's units.
        offset: usize,
        /// The token's text, in the input's type.
        text: Py<PyAny>,
        /// How many paired brackets enclose the token.
        depth: i32,
        /// How many tokens the innermost open bracket has been held open.
        strain: f32,
        /// How many tokens every open bracket has been held open, summed.
        load: f32,
    }
}

axis_class! {
    /// The stress axis over one input: how deeply the brackets nest at each
    /// token, the peaks of that nesting, and where it releases.
    StressReport repr(path, tokens, max_depth) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// How many tokens the axis read, whitespace left out.
        tokens: usize,
        /// The deepest nesting reached.
        max_depth: i32,
        /// The token carrying the greatest load; `None` for an input with no
        /// tokens.
        peak_load: Option<Py<StressFrame>>,
        /// The tokens at a local maximum of depth, at least
        /// `peak_min_depth` deep, two by default.
        peaks: Vec<Py<StressFrame>>,
        /// The tokens where the depth starts to fall from a level at least
        /// `fracture_min_depth` deep, two by default: where a nested
        /// structure begins to close.
        fractures: Vec<Py<StressFrame>>,
        /// Every token's frame, with `detail=True`.
        frames: Vec<Py<StressFrame>>,
    }
}

/// The stress axis: how deeply the paired brackets nest at each token, how
/// long the innermost has been held open, and how long all of them
/// together. Peaks are the local maxima of depth at least `peak_min_depth`
/// deep, and fractures the tokens where the depth starts to fall from a
/// level at least `fracture_min_depth` deep, both two by default; an
/// unpaired bracket opens no level.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, lib = None, detail = false, peak_min_depth = None, fracture_min_depth = None))]
pub(crate) fn stress(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    lib: Option<&Bound<'_, PyAny>>,
    detail: bool,
    peak_min_depth: Option<u16>,
    fracture_min_depth: Option<u16>,
) -> PyResult<StressReport> {
    let defaults = trex::stress::StressConfig::default();
    let cfg = trex::stress::StressConfig {
        peak_min_depth: peak_min_depth.unwrap_or(defaults.peak_min_depth),
        fracture_min_depth: fracture_min_depth.unwrap_or(defaults.fracture_min_depth),
    };
    let shapes = crate::shapes_of(lib)?;
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let f = py.detach(|| trex::stress::analyze_with(&lexed(bytes, &shapes, true), bytes, &cfg));
    let frame = |i: usize, offset: usize, text: Py<PyAny>| {
        let r = &f.frames[i];
        Py::new(py, StressFrame { offset, text, depth: i32::from(r.depth), strain: r.strain, load: r.load })
    };
    let peak = f.frames.iter().enumerate().max_by(|a, b| a.1.load.total_cmp(&b.1.load)).map(|(i, _)| i);
    Ok(StressReport {
        length: input.length(),
        tokens: f.n_tokens,
        max_depth: i32::from(f.max_depth),
        peak_load: input.frames_at(py, &f.spans, &Vec::from_iter(peak), frame)?.pop(),
        peaks: input.frames_at(py, &f.spans, &tokens_at(&f.spans, &f.peaks)?, frame)?,
        fractures: input.frames_at(py, &f.spans, &tokens_at(&f.spans, &f.fractures)?, frame)?,
        frames: input.frames_at(py, &f.spans, &all(detail, f.frames.len()), frame)?,
        path: input.path,
    })
}

axis_class! {
    /// One unit's flow reading.
    FlowFrame repr(offset, text, direction) {
        /// Where the unit starts, in the input's units.
        offset: usize,
        /// The unit's text, in the input's type.
        text: Py<PyAny>,
        /// The signal's change per unit, averaged over the `window` units
        /// ending at this one, four by default.
        slope: f32,
        /// Which way the trend runs: `"rising"`, `"falling"`, or `"steady"`
        /// where the slope is within `steady_band` of zero, 0.05 by default.
        direction: String,
        /// How many consecutive units have held the same direction, rising
        /// or falling.
        momentum: i32,
    }
}

axis_class! {
    /// One unit's analytic reading: the signal's position in its swing.
    AnalyticFrame repr(offset, text, amplitude) {
        /// Where the unit starts, in the input's units.
        offset: usize,
        /// The unit's text, in the input's type.
        text: Py<PyAny>,
        /// The envelope of the signal's swing around its running mean.
        amplitude: f32,
        /// The unit's position in the swing, in radians from -pi to pi.
        phase: f32,
        /// How fast the swing runs, in radians a unit: the phase's advance
        /// from the unit before.
        frequency: f32,
    }
}

axis_class! {
    /// The flow axis over one input: the trend of a per-unit signal, how
    /// long each run of it lasts, and where it turns.
    FlowReport repr(path, tokens, signal) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// How many tokens the input holds, whitespace left out.
        tokens: usize,
        /// The stream the trend was read over: `"token"`, or `"super"` for
        /// the supertokens.
        grain: String,
        /// How many units the axis read at that grain.
        units: usize,
        /// The signal whose trend was read: `"magnitude"`, `"stress"` or
        /// `"length"`.
        signal: String,
        /// The unit holding the longest run; `None` for an input with no
        /// units.
        peak_momentum: Option<Py<FlowFrame>>,
        /// The units where the trend turns between rising and falling.
        reversals: Vec<Py<FlowFrame>>,
        /// Every unit's frame, with `detail=True`.
        frames: Vec<Py<FlowFrame>>,
        /// How many units past a unit its analytic reading uses, with
        /// `analytic=True`; `None` without it. The last `latency` units
        /// have no reading.
        latency: Option<usize>,
        /// Every unit's analytic reading that has one, with
        /// `analytic=True`.
        analytic: Vec<Py<AnalyticFrame>>,
    }
}

/// The flow axis: the trend of a per-token signal, how long it has run one
/// way, and where it reverses. `signal` is `"magnitude"`, each token's order
/// of magnitude as the magnitude axis reads it; `"stress"`, its nesting
/// depth as the stress axis reads it; or `"length"`, its length in bytes.
/// `grain="super"` reads it from supertoken to supertoken. `window` sets how
/// many units the slope is averaged over, four by default, and
/// `steady_band` how far from zero a slope reads as steady, 0.05 by
/// default. `analytic=True` adds the signal's position in its swing:
/// its envelope, its phase and how fast the swing runs.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, lib = None, signal = "magnitude", grain = "token", detail = false, analytic = false, window = None, steady_band = None))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn flow(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    lib: Option<&Bound<'_, PyAny>>,
    signal: &str,
    grain: &str,
    detail: bool,
    analytic: bool,
    window: Option<usize>,
    steady_band: Option<f32>,
) -> PyResult<FlowReport> {
    let read = match signal {
        "magnitude" => trex::flow::Signal::Magnitude,
        "stress" => trex::flow::Signal::StressDepth,
        "length" => trex::flow::Signal::Length,
        other => {
            return Err(PyValueError::new_err(format!(
                "signal takes 'magnitude', 'stress' or 'length', not {other:?}"
            )));
        }
    };
    let by_super = match grain {
        "token" => false,
        "super" => true,
        other => {
            return Err(PyValueError::new_err(format!(
                "grain takes 'token' or 'super', not {other:?}: the flow axis reads tokens or supertokens"
            )));
        }
    };
    let defaults = trex::flow::FlowConfig::default();
    let cfg = trex::flow::FlowConfig {
        window: window.unwrap_or(defaults.window),
        steady_band: cut("steady_band", steady_band, defaults.steady_band)?,
    };
    let shapes = crate::shapes_of(lib)?;
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let (tokens, f, reading) = py.detach(|| {
        let sig = lexed(bytes, &shapes, true);
        if by_super {
            let toks = lexed(bytes, &shapes, false);
            let f = trex::flow::analyze_supertokens_with(&toks, bytes, read, &cfg);
            let reading = analytic.then(|| trex::flow::analytic_supertokens(&toks, bytes, read));
            (sig.len(), f, reading)
        } else {
            let f = trex::flow::analyze_with(&sig, bytes, read, &cfg);
            let reading = analytic.then(|| trex::flow::analytic(&sig, bytes, read));
            (sig.len(), f, reading)
        }
    });
    let frame = |i: usize, offset: usize, text: Py<PyAny>| {
        let r = &f.frames[i];
        let direction = match r.direction {
            1 => "rising",
            -1 => "falling",
            _ => "steady",
        };
        Py::new(
            py,
            FlowFrame { offset, text, slope: r.slope, direction: direction.to_string(), momentum: i32::from(r.momentum) },
        )
    };
    let peak = f.frames.iter().enumerate().max_by_key(|(_, r)| r.momentum).map(|(i, _)| i);
    // A unit's reading uses the signal `latency` units past it, so the last
    // `latency` units have none and carry no frame.
    let analytic_frames = match &reading {
        Some(a) => {
            let read: Vec<usize> = (0..f.n_tokens.saturating_sub(a.latency)).collect();
            input.frames_at(py, &f.spans, &read, |i, offset, text| {
                Py::new(
                    py,
                    AnalyticFrame { offset, text, amplitude: a.amplitude[i], phase: a.phase[i], frequency: a.frequency[i] },
                )
            })?
        }
        None => Vec::new(),
    };
    Ok(FlowReport {
        length: input.length(),
        tokens,
        grain: grain.to_string(),
        units: f.n_tokens,
        signal: signal.to_string(),
        peak_momentum: input.frames_at(py, &f.spans, &Vec::from_iter(peak), frame)?.pop(),
        reversals: input.frames_at(py, &f.spans, &tokens_at(&f.spans, &f.reversals)?, frame)?,
        frames: input.frames_at(py, &f.spans, &all(detail, f.frames.len()), frame)?,
        latency: reading.as_ref().map(|a| a.latency),
        analytic: analytic_frames,
        path: input.path,
    })
}

axis_class! {
    /// One unit's observation reading: the entropy of the classes around
    /// it, seen from behind, from ahead and from both sides.
    ObservationFrame repr(offset, disagreement) {
        /// The unit's offset, in the input's units; each byte of a
        /// character written in several has a frame at the character's
        /// offset.
        offset: usize,
        /// The unit's text, in the input's type: at the byte grain the
        /// character the byte begins, `""` for a byte inside a character
        /// written in several, and of bytes the byte itself; the token or
        /// the supertoken otherwise.
        text: Py<PyAny>,
        /// The class entropy of the window before the unit, from 0 to 1.
        causal: f32,
        /// The class entropy of the window after the unit, from 0 to 1.
        anticausal: f32,
        /// The class entropy of the window centered on the unit, from 0 to 1.
        centered: f32,
        /// How far the two sides differ: causal less anticausal, unsigned.
        disagreement: f32,
    }
}

axis_class! {
    /// The observation axis over one input: where the text behind a point
    /// and the text ahead of it read differently.
    ObservationReport repr(path, length) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// The stream read: `"byte"`, `"token"` or `"super"` for the
        /// supertokens.
        grain: String,
        /// How many units the axis read at that grain.
        units: usize,
        /// The contested unit where the two sides differ most; `None` where
        /// none is contested.
        peak: Option<Py<ObservationFrame>>,
        /// The units where the two sides differ most: local maxima of
        /// disagreement at least `contested_threshold`, 0.2 by default; at
        /// the byte grain at least `contested_min_gap` bytes apart, 8 by
        /// default, the stronger of two closer maxima kept.
        contested: Vec<Py<ObservationFrame>>,
        /// Every unit's frame, with `detail=True`.
        frames: Vec<Py<ObservationFrame>>,
    }
}

/// The observation axis: the entropy of the classes behind each unit, ahead
/// of it and around it, and the points where behind and ahead differ most.
/// At the byte grain the classes are digits, letters, whitespace, other
/// characters and bytes that are not part of a well-formed UTF-8 character,
/// so the axis reads the texture of the text and not its words; a letter
/// outside ASCII is a letter, and any other character outside ASCII counts
/// once. `grain="token"` reads the sequence of token kinds
/// instead, and `grain="super"` the sequence of supertoken roles, each
/// lexed under `lib=`'s declarations where it is given; the byte grain reads
/// bytes and takes no library.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, lib = None, grain = "byte", detail = false, contested_threshold = None, contested_min_gap = None))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn observation(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    lib: Option<&Bound<'_, PyAny>>,
    grain: &str,
    detail: bool,
    contested_threshold: Option<f32>,
    contested_min_gap: Option<usize>,
) -> PyResult<ObservationReport> {
    use trex::ast::Grain;
    let Some(unit) = Grain::parse(grain) else {
        return Err(PyValueError::new_err(format!("grain takes 'byte', 'token' or 'super', not {grain:?}")));
    };
    if unit == Grain::Byte && lib.is_some() {
        return Err(PyValueError::new_err(
            "lib= decides how tokens are read, and the byte grain reads bytes; grain='token' or 'super' reads them",
        ));
    }
    let shapes = crate::shapes_of(lib)?;
    let defaults = trex::observation::ObservationConfig::default();
    let threshold = cut("contested_threshold", contested_threshold, defaults.contested_threshold)?;
    if !(0.0..=1.0).contains(&threshold) {
        return Err(PyValueError::new_err(format!(
            "contested_threshold={threshold} names no disagreement: a disagreement runs 0 to 1"
        )));
    }
    if contested_min_gap.is_some() && unit != Grain::Byte {
        return Err(PyValueError::new_err(
            "contested_min_gap= spaces the byte grain's contested points; the token and super grains keep every local maximum",
        ));
    }
    let cfg = trex::observation::ObservationConfig {
        contested_threshold: threshold,
        contested_min_gap: contested_min_gap.unwrap_or(defaults.contested_min_gap),
        ..defaults
    };
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    // The units' spans at the token and super grains; at the byte grain a
    // unit is the byte at its own index.
    let (f, spans) = py.detach(|| match unit {
        Grain::Byte => (trex::observation::analyze_with(bytes, &cfg), None),
        Grain::Token => {
            let sig = lexed(bytes, &shapes, true);
            let spans: Vec<(usize, usize)> = sig.iter().map(|t| (t.start(), t.end())).collect();
            (trex::observation::analyze_tokens(&sig, &cfg), Some(spans))
        }
        Grain::Super => {
            let units = trex::supertoken::supertokens_from(&lexed(bytes, &shapes, false), bytes);
            let spans: Vec<(usize, usize)> = units.iter().map(|u| (u.start, u.end)).collect();
            (trex::observation::analyze_supertokens(&units, &cfg), Some(spans))
        }
    });
    let chars = if input.as_bytes {
        None
    } else {
        Some(std::str::from_utf8(bytes).map_err(|e| PyRuntimeError::new_err(format!("the text read is not UTF-8: {e}")))?)
    };
    let frames_of = |at: &[usize]| -> PyResult<Vec<Py<ObservationFrame>>> {
        let starts: Vec<usize> = match &spans {
            Some(s) => at.iter().map(|&i| s[i].0).collect(),
            None => at.to_vec(),
        };
        input
            .offsets(&starts)
            .into_iter()
            .zip(at)
            .map(|(offset, &i)| {
                let r = &f.frames[i];
                let text = match (&spans, chars) {
                    (Some(s), _) => input.piece(py, s[i]),
                    (None, None) => PyBytes::new(py, &bytes[i..i + 1]).into_any().unbind(),
                    (None, Some(s)) => {
                        let c = match s.get(i..).and_then(|rest| rest.chars().next()) {
                            Some(c) => c.to_string(),
                            None => String::new(),
                        };
                        PyString::new(py, &c).into_any().unbind()
                    }
                };
                Py::new(
                    py,
                    ObservationFrame {
                        offset,
                        text,
                        causal: r.causal,
                        anticausal: r.anticausal,
                        centered: r.centered,
                        disagreement: r.disagreement,
                    },
                )
            })
            .collect()
    };
    let contested = frames_of(&f.contested)?;
    let peak = contested
        .iter()
        .max_by(|a, b| a.get().disagreement.total_cmp(&b.get().disagreement))
        .map(|p| p.clone_ref(py));
    Ok(ObservationReport {
        length: input.length(),
        grain: grain.to_string(),
        units: spans.as_ref().map_or(bytes.len(), Vec::len),
        peak,
        frames: frames_of(&all(detail, f.frames.len()))?,
        contested,
        path: input.path,
    })
}

axis_class! {
    /// One token's place in the enclosure tree.
    RelationFrame repr(offset, text, depth) {
        /// Where the token starts, in the input's units.
        offset: usize,
        /// The token's text, in the input's type.
        text: Py<PyAny>,
        /// How many brackets enclose it.
        depth: i32,
        /// The heads of the bracket groups enclosing it, outermost first, in
        /// the input's type.
        enclosure: Vec<Py<PyAny>>,
    }
}

axis_class! {
    /// One directed relation between two tokens.
    RelationEdge repr(kind, from_, to) {
        /// The relation the edge carries: `"encloses"`, from the word heading
        /// a bracket group to a token inside it; `"operator"`, from one
        /// operand of a binding punctuation to the other, in order; or
        /// `"adjacent"`, from a significant token to the next.
        kind: String,
        /// The text of the token it runs from, in the input's type.
        from_: Py<PyAny>,
        /// The text of the token it runs to, in the input's type.
        to: Py<PyAny>,
        /// Where the token it runs from starts, in the input's units.
        from_offset: usize,
        /// Where the token it runs to starts, in the input's units.
        to_offset: usize,
    }
}

axis_class! {
    /// One reuse: a later occurrence of repeated content joined to the
    /// earlier.
    RelationChord repr(from_, to, residual) {
        /// The earlier occurrence's text, in the input's type.
        from_: Py<PyAny>,
        /// The later occurrence's text, in the input's type.
        to: Py<PyAny>,
        /// The later occurrence's depth less the earlier's: positive where
        /// the reuse is deeper.
        residual: i32,
        /// Where the earlier occurrence starts, in the input's units.
        from_offset: usize,
        /// Where the later occurrence starts, in the input's units.
        to_offset: usize,
    }
}

axis_class! {
    /// The relation axis over one input: the graph its brackets, operators,
    /// adjacency and reuse make, read as a tree with loops.
    RelationReport repr(path, tokens, max_depth, holonomy, cycle_rank) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// How many tokens the lexer read, whitespace included.
        tokens: usize,
        /// The deepest nesting of brackets.
        max_depth: i32,
        /// How many tokens are inside at least one bracket group.
        nesting_load: usize,
        /// The edges from a bracket group's head to the tokens inside it.
        encloses: usize,
        /// The edges between the operands of a binding punctuation.
        operators: usize,
        /// The edges from each significant token to the next.
        adjacencies: usize,
        /// The reuses: later occurrences of repeated content joined to the
        /// earlier.
        reuse_chords: usize,
        /// The reuses whose two occurrences are at different depths.
        scope_crossing_chords: usize,
        /// The reuses' depth differences summed without sign: 0 for a tree,
        /// and positive once a reuse crosses a scope.
        holonomy: u32,
        /// The same sum with its sign: positive where reuse flows inward,
        /// negative outward, 0 where the two cancel.
        net_holonomy: i32,
        /// The brackets the input holds, opens and closes, as the boundary
        /// reading counts them.
        boundary_events: usize,
        /// The balanced bracket groups.
        bulk_nodes: usize,
        /// What the brackets alone do not determine: the reuses.
        holographic_defect: usize,
        /// Whether every bracket is balanced, so that the brackets alone
        /// determine the nesting.
        boundary_closed: bool,
        /// The enclosure edges rebuilt from the brackets alone.
        rebuilt_edges: usize,
        /// The most negative edge curvature: the sharpest bottleneck, 0 for
        /// a graph with no edges.
        min_ricci: i32,
        /// The mean edge curvature.
        mean_ricci: f32,
        /// The edges of negative curvature: the bottlenecks.
        bridges: usize,
        /// The tokens that take part in at least one relation.
        nodes: usize,
        /// The relation graph's undirected edges.
        graph_edges: usize,
        /// Its connected components.
        components: usize,
        /// Its independent loops: edges less nodes plus components.
        cycle_rank: usize,
        /// Its Euler characteristic: nodes less edges.
        euler: i64,
        /// The longest run of tokens a single reuse joins in one step; 0
        /// where nothing is reused.
        max_shortcut: usize,
        /// The most relation edges crossing any boundary between two tokens.
        peak_entanglement: usize,
        /// The fewest relation edges crossing a boundary between two tokens;
        /// `None` where the input has no boundary inside it.
        minimal_cut: Option<usize>,
        /// That boundary's offset, in the input's units: the start of the
        /// token after it; `None` with `minimal_cut`.
        minimal_cut_offset: Option<usize>,
        /// The input's alpha-equivalence form, with `canonical=True`: the
        /// first word inside each `[` is a binder written `#`, a later use
        /// of it `^k` for the binder k scopes out, a name bound nowhere is
        /// kept, and the tokens are joined by spaces; `None` otherwise.
        canonical: Option<String>,
        /// Every relation edge, with `detail=True`.
        edges: Vec<Py<RelationEdge>>,
        /// Every reuse, with `detail=True`.
        chords: Vec<Py<RelationChord>>,
        /// The frame of every token inside a bracket group, with
        /// `detail=True`.
        frames: Vec<Py<RelationFrame>>,
    }
}

/// The relation axis: the graph an input's brackets, binding punctuation,
/// adjacency and repeated content make, and what that graph's tree and its
/// loops measure. A reuse joins a later occurrence of repeated content to
/// the earlier, and its depth difference is its holonomy: a tree has none,
/// and a name bound outside and used deeper has some. The curvature,
/// topology, geodesic and entanglement readings are of the same graph.
/// `canonical=True` adds the input's alpha-equivalence form, which reads
/// alike for two texts that differ only in the names their brackets bind.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, lib = None, canonical = false, detail = false))]
pub(crate) fn relation(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    lib: Option<&Bound<'_, PyAny>>,
    canonical: bool,
    detail: bool,
) -> PyResult<RelationReport> {
    use trex::relation::RelationKind as K;
    let shapes = crate::shapes_of(lib)?;
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let (f, h, rebuilt_edges, curvature, topology, geodesic, entanglement, form) = py.detach(|| {
        let toks = lexed(bytes, &shapes, false);
        let h = trex::holography::analyze(&toks, bytes);
        let rebuilt_edges = h.reconstruct_bulk(&toks).len();
        (
            trex::relation::analyze(&toks, bytes),
            h,
            rebuilt_edges,
            trex::curvature::analyze(&toks, bytes),
            trex::topology::analyze(&toks, bytes),
            trex::geodesic::analyze(&toks, bytes),
            trex::entanglement::analyze(&toks, bytes),
            canonical.then(|| trex::gauge::canonicalize(&toks, bytes)),
        )
    });
    let (minimal_cut, minimal_cut_offset) = match entanglement.min_cut() {
        Some((k, crossings)) => {
            let at = match f.spans.get(k) {
                Some(&(s, _)) => s,
                None => bytes.len(),
            };
            (Some(crossings), Some(input.offsets(&[at])[0]))
        }
        None => (None, None),
    };

    let (mut edges, mut chords, mut frames) = (Vec::new(), Vec::new(), Vec::new());
    if detail {
        let starts: Vec<usize> = f.spans.iter().map(|&(s, _)| s).collect();
        let offsets = input.offsets(&starts);
        let word = |i: usize| input.piece(py, f.spans[i]);
        for e in &f.edges {
            let kind = match e.kind {
                K::Encloses => "encloses",
                K::Operator => "operator",
                K::Adjacent => "adjacent",
            };
            edges.push(Py::new(
                py,
                RelationEdge {
                    kind: kind.to_string(),
                    from_: word(e.from),
                    to: word(e.to),
                    from_offset: offsets[e.from],
                    to_offset: offsets[e.to],
                },
            )?);
        }
        for c in &f.chords {
            chords.push(Py::new(
                py,
                RelationChord {
                    from_: word(c.from),
                    to: word(c.to),
                    residual: c.residual,
                    from_offset: offsets[c.from],
                    to_offset: offsets[c.to],
                },
            )?);
        }
        let nested: Vec<usize> = (0..f.frames.len()).filter(|&i| f.frames[i].depth > 0).collect();
        frames = input.frames_at(py, &f.spans, &nested, |i, offset, text| {
            Py::new(
                py,
                RelationFrame {
                    offset,
                    text,
                    depth: i32::from(f.frames[i].depth),
                    enclosure: f.frames[i].enclosure.iter().map(|&head| word(head)).collect(),
                },
            )
        })?;
    }

    Ok(RelationReport {
        length: input.length(),
        tokens: f.n_tokens,
        max_depth: i32::from(f.max_depth()),
        nesting_load: f.nesting_load,
        encloses: f.edges_of(K::Encloses).count(),
        operators: f.edges_of(K::Operator).count(),
        adjacencies: f.edges_of(K::Adjacent).count(),
        reuse_chords: f.chords.len(),
        scope_crossing_chords: f.twisted_chords(),
        holonomy: f.holonomy(),
        net_holonomy: f.net_holonomy(),
        boundary_events: h.boundary.len(),
        bulk_nodes: h.bulk_nodes,
        holographic_defect: h.holographic_defect,
        boundary_closed: h.boundary_closed,
        rebuilt_edges,
        min_ricci: curvature.min_ricci(),
        mean_ricci: curvature.mean_ricci(),
        bridges: curvature.bridges(),
        nodes: topology.nodes,
        graph_edges: topology.edges,
        components: topology.components,
        cycle_rank: topology.cycle_rank,
        euler: topology.euler(),
        max_shortcut: geodesic.max_shortcut(),
        peak_entanglement: entanglement.max_entanglement(),
        minimal_cut,
        minimal_cut_offset,
        canonical: form,
        edges,
        chords,
        frames,
        path: input.path,
    })
}

/// The name a texture goes by.
fn texture_name(t: trex::spectral::Texture) -> &'static str {
    match t {
        trex::spectral::Texture::Prose => "prose",
        trex::spectral::Texture::Code => "code",
        trex::spectral::Texture::Math => "math",
        trex::spectral::Texture::Data => "data",
        trex::spectral::Texture::Mixed => "mixed",
    }
}

axis_class! {
    /// One frame of the spectral reading: the byte statistics of a window.
    SpectralFrame repr(offset, texture, entropy) {
        /// The last byte the frame reads, in the input's units.
        offset: usize,
        /// The window's texture: `"prose"`, `"code"`, `"math"`, `"data"` or
        /// `"mixed"`.
        texture: String,
        /// The window's byte entropy, from 0 to 1.
        entropy: f32,
        /// The period its bytes repeat at, in bytes; 0 where none.
        period: i32,
        /// How new its runs of bytes are, from 0 to 1, 1 for a first
        /// sighting.
        novelty: f32,
        /// The shares of digits, letters, whitespace, punctuation, and control
        /// bytes or bytes not part of a well-formed UTF-8 character, among its
        /// recent bytes, in that order.
        mix: Vec<f32>,
    }
}

axis_class! {
    /// A byte period the spectral reading found, and how strongly it holds.
    SpectralPeriod repr(period, strength) {
        /// The period, in bytes.
        period: i32,
        /// Its strongest showing, from 0 to 1.
        strength: f32,
    }
}

axis_class! {
    /// A stretch of an input between two change points, with its texture.
    TextureRegion repr(offset, length, texture) {
        /// Where the stretch starts, in the input's units.
        offset: usize,
        /// How many of the input's units it spans.
        length: usize,
        /// Its texture: `"prose"`, letters dominating and punctuation low;
        /// `"code"`, punctuation and symbols high beside identifiers;
        /// `"math"`, single symbols and digits mixed; `"data"`, high entropy,
        /// allowing for the multi-byte characters it holds, or many bytes not
        /// part of a well-formed UTF-8 character; or `"mixed"`, no class
        /// dominating.
        texture: String,
        /// Its byte entropy, from 0 to 1.
        entropy: f32,
        /// The period its bytes repeat at, in bytes; 0 where none.
        period: i32,
        /// How new its runs of bytes are, from 0 to 1.
        novelty: f32,
    }
}

axis_class! {
    /// A region of an input classified by its texture and its token shapes.
    ClassifiedRegion repr(offset, length, kind) {
        /// Where the region starts, in the input's units.
        offset: usize,
        /// How many of the input's units it spans.
        length: usize,
        /// What the region is: `"table"`, token shapes repeating at a strong
        /// period whatever its bytes look like; `"blob"`, a high-entropy run
        /// of packed, encoded or encrypted bytes; `"prose"`; `"numeric"`;
        /// `"code"` whose shapes do not repeat strongly; or `"mixed"`.
        kind: String,
        /// A table's period in tokens; 0 for every other kind.
        period: i32,
    }
}

axis_class! {
    /// The spectral axis over one input: its byte statistics along its
    /// length, where they change, and the texture of each stretch between.
    SpectralReport repr(path, length, hop) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// How many bytes each frame advances.
        hop: usize,
        /// Where the byte statistics change, in the input's units.
        change_points: Vec<usize>,
        /// The least byte entropy of any frame, from 0 to 1; `None` for an
        /// input with no frames.
        entropy_minimum: Option<f32>,
        /// The mean byte entropy over the frames; `None` for an input with
        /// no frames.
        entropy_mean: Option<f32>,
        /// The greatest byte entropy of any frame; `None` for an input with
        /// no frames.
        entropy_maximum: Option<f32>,
        /// The distinct byte periods the frames found, strongest first.
        periods: Vec<Py<SpectralPeriod>>,
        /// The input cut at its change points, each stretch with its
        /// texture.
        timeline: Vec<Py<TextureRegion>>,
        /// The input's regions classified by texture and token shape, with
        /// `classify=True`.
        regions: Vec<Py<ClassifiedRegion>>,
        /// Every frame, with `detail=True`.
        frames: Vec<Py<SpectralFrame>>,
    }
}

/// The spectral axis: the entropy, byte period, novelty and class mix of an
/// input's bytes along its length, where they change, and the texture of
/// each stretch between, prose, code, mathematics, data or mixed.
/// `classify=True` adds the regions read by texture and by token shape
/// together, so a table whose columns do not line up still reads as a
/// table. A change point is where the byte statistics move past their
/// running mean by more than `cp_threshold` mean deviations, 4 by default,
/// or `cp_floor` of the mean, 0.05 by default, at least `cp_min_gap` bytes
/// after the last, 4 by default. The axis reads bytes, so it takes no
/// library.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, classify = false, detail = false, cp_threshold = None, cp_floor = None, cp_min_gap = None))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn spectral(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    classify: bool,
    detail: bool,
    cp_threshold: Option<f32>,
    cp_floor: Option<f32>,
    cp_min_gap: Option<usize>,
) -> PyResult<SpectralReport> {
    let defaults = trex::spectral::SpectralConfig::default();
    let cfg = trex::spectral::SpectralConfig {
        cp_threshold: cut("cp_threshold", cp_threshold, defaults.cp_threshold)?,
        cp_floor: cut("cp_floor", cp_floor, defaults.cp_floor)?,
        cp_min_gap: cp_min_gap.unwrap_or(defaults.cp_min_gap),
        ..defaults
    };
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let (f, stretches, classified) = py.detach(|| {
        let f = trex::spectral::analyze_with(bytes, &cfg);
        let stretches = trex::spectral::regions(&f);
        let classified = if classify { trex::shape::classified_regions_with(bytes, &cfg) } else { Vec::new() };
        (f, stretches, classified)
    });

    let (mut entropy_minimum, mut entropy_mean, mut entropy_maximum) = (None, None, None);
    if !f.frames.is_empty() {
        let (mut lo, mut hi, mut sum) = (f32::INFINITY, f32::NEG_INFINITY, 0.0f32);
        for r in &f.frames {
            lo = lo.min(r.entropy);
            hi = hi.max(r.entropy);
            sum += r.entropy;
        }
        entropy_minimum = Some(lo);
        entropy_mean = Some(sum / f.frames.len() as f32);
        entropy_maximum = Some(hi);
    }

    let mut found: Vec<(i32, f32)> = Vec::new();
    for r in f.frames.iter().filter(|r| r.period != 0) {
        let period = i32::from(r.period);
        match found.iter_mut().find(|p| p.0 == period) {
            Some(p) => p.1 = p.1.max(r.period_strength),
            None => found.push((period, r.period_strength)),
        }
    }
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    let periods = found
        .into_iter()
        .map(|(period, strength)| Py::new(py, SpectralPeriod { period, strength }))
        .collect::<PyResult<Vec<_>>>()?;

    let ends: Vec<usize> = stretches.iter().flat_map(|&(s, e, _)| [s, e]).collect();
    let at = input.offsets(&ends);
    let timeline = stretches
        .iter()
        .enumerate()
        .map(|(k, &(s, e, texture))| {
            let sig = f.signature(s, e);
            Py::new(
                py,
                TextureRegion {
                    offset: at[2 * k],
                    length: at[2 * k + 1] - at[2 * k],
                    texture: texture_name(texture).to_string(),
                    entropy: sig.entropy,
                    period: i32::from(sig.period),
                    novelty: sig.novelty,
                },
            )
        })
        .collect::<PyResult<Vec<_>>>()?;

    let ends: Vec<usize> = classified.iter().flat_map(|&(s, e, _)| [s, e]).collect();
    let at = input.offsets(&ends);
    let regions = classified
        .iter()
        .enumerate()
        .map(|(k, &(_, _, kind))| {
            use trex::shape::RegionKind as R;
            let (kind, period) = match kind {
                R::Table(p) => ("table", i32::from(p)),
                R::Blob => ("blob", 0),
                R::Prose => ("prose", 0),
                R::Numeric => ("numeric", 0),
                R::Code => ("code", 0),
                R::Mixed => ("mixed", 0),
            };
            Py::new(
                py,
                ClassifiedRegion { offset: at[2 * k], length: at[2 * k + 1] - at[2 * k], kind: kind.to_string(), period },
            )
        })
        .collect::<PyResult<Vec<_>>>()?;

    let mut frames = Vec::new();
    if detail && !bytes.is_empty() {
        let last: Vec<usize> = (0..f.frames.len()).map(|j| ((j + 1) * f.hop).min(f.len).saturating_sub(1)).collect();
        for (r, offset) in f.frames.iter().zip(input.offsets(&last)) {
            frames.push(Py::new(
                py,
                SpectralFrame {
                    offset,
                    texture: texture_name(trex::spectral::texture_of(r)).to_string(),
                    entropy: r.entropy,
                    period: i32::from(r.period),
                    novelty: r.novelty,
                    mix: r.texture_mix().to_vec(),
                },
            )?);
        }
    }

    Ok(SpectralReport {
        length: input.length(),
        hop: f.hop,
        change_points: input.offsets(&f.boundaries),
        entropy_minimum,
        entropy_mean,
        entropy_maximum,
        periods,
        timeline,
        regions,
        frames,
        path: input.path,
    })
}

axis_class! {
    /// A run of an input: where it starts, how long it is, and its text.
    Segment repr(offset, length, text) {
        /// Where the run starts, in the input's units.
        offset: usize,
        /// How many of the input's units it spans.
        length: usize,
        /// The run's text, in the input's type.
        text: Py<PyAny>,
    }
}

/// The runs of `input` at byte spans given in order, each end inside a
/// character moved to the character's start.
fn segments_of(py: Python<'_>, input: &AxisInput, spans: &[(usize, usize)]) -> PyResult<Vec<Py<Segment>>> {
    let ends: Vec<usize> = spans.iter().flat_map(|&(s, e)| [s, e]).collect();
    let at = input.offsets(&ends);
    spans
        .iter()
        .enumerate()
        .map(|(k, &(s, e))| {
            let text = input.piece(py, (input.snapped(s), input.snapped(e)));
            Py::new(py, Segment { offset: at[2 * k], length: at[2 * k + 1] - at[2 * k], text })
        })
        .collect()
}

/// The runs of `input` between cuts, in order: from the start to the first
/// cut, between each pair, and from the last to the end.
fn segments_between(py: Python<'_>, input: &AxisInput, cuts: &[usize]) -> PyResult<Vec<Py<Segment>>> {
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(cuts.len() + 1);
    let mut from = 0usize;
    for &c in cuts {
        spans.push((from, c));
        from = c;
    }
    spans.push((from, input.bytes.len()));
    segments_of(py, input, &spans)
}

axis_class! {
    /// One byte's seam reading: how uncertain the text is on either side of
    /// it.
    SeamFrame repr(offset, text, boundary) {
        /// The byte's offset, in the input's units.
        offset: usize,
        /// The character the byte begins, `""` for a byte inside a
        /// character written in several; of bytes, the byte itself.
        text: Py<PyAny>,
        /// The forward branching entropy at the byte: how uncertain what
        /// follows is, given what precedes.
        forward: f32,
        /// The backward branching entropy at the byte: how uncertain what
        /// precedes is, given what follows.
        backward: f32,
        /// The seam strength of a cut before the byte, from both readings
        /// together.
        boundary: f32,
    }
}

axis_class! {
    /// The seam axis over one input: the units it divides into where the
    /// text before stops predicting the text after.
    SeamReport repr(path, length, order) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// The longest context the entropy was read over, in bytes.
        order: usize,
        /// The units, in order.
        segments: Vec<Py<Segment>>,
        /// Every byte's frame, with `detail=True`.
        frames: Vec<Py<SeamFrame>>,
    }
}

/// The seam axis: where an input divides into units, found where the text
/// before a point stops predicting the text after it, read in both
/// directions, with no dictionary. `order` is the longest context read, in
/// bytes, 3 by default; `passes` how many confidence passes to run, one by
/// default. `english=True` reads with a model of English letter
/// sequences, so even a short string divides where English does not join
/// its letters, and a unit may be cut inside a token the lexer read. A cut
/// needs a seam strength at least `cut_threshold` standard deviations above
/// the input's mean, 0.6 by default. The axis reads bytes, so it takes no
/// library.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, order = None, passes = None, english = false, detail = false, cut_threshold = None))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn seam(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    order: Option<usize>,
    passes: Option<usize>,
    english: bool,
    detail: bool,
    cut_threshold: Option<f32>,
) -> PyResult<SeamReport> {
    let order = match order {
        Some(0) => return Err(PyValueError::new_err("order= is a context length in bytes, 1 or more")),
        Some(k) => k,
        None => 3,
    };
    let defaults = trex::seam::SeamConfig::default();
    // No passes given is 0, which `auto_passes` reads as one pass.
    let cfg = trex::seam::SeamConfig {
        order,
        passes: passes.unwrap_or_default(),
        cut_threshold: cut("cut_threshold", cut_threshold, defaults.cut_threshold)?,
        ..defaults
    };
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let f = py.detach(|| {
        if english {
            trex::seam::analyze_with_model(bytes, &trex::seam::english_model(), &cfg)
        } else {
            trex::seam::analyze_with(bytes, &cfg)
        }
    });
    let mut frames = Vec::new();
    if detail {
        let chars = if input.as_bytes {
            None
        } else {
            Some(
                std::str::from_utf8(bytes)
                    .map_err(|e| PyRuntimeError::new_err(format!("the text read is not UTF-8: {e}")))?,
            )
        };
        let all: Vec<usize> = (0..bytes.len()).collect();
        for (b, offset) in input.offsets(&all).into_iter().enumerate() {
            let text = match chars {
                None => PyBytes::new(py, &bytes[b..b + 1]).into_any().unbind(),
                Some(s) => {
                    let c = match s.get(b..).and_then(|rest| rest.chars().next()) {
                        Some(c) => c.to_string(),
                        None => String::new(),
                    };
                    PyString::new(py, &c).into_any().unbind()
                }
            };
            frames.push(Py::new(
                py,
                SeamFrame { offset, text, forward: f.fwd_entropy[b], backward: f.bwd_entropy[b], boundary: f.boundary[b] },
            )?);
        }
    }
    Ok(SeamReport {
        length: input.length(),
        order,
        segments: segments_of(py, &input, &f.segments())?,
        frames,
        path: input.path,
    })
}

axis_class! {
    /// One token's echo reading.
    EchoFrame repr(offset, text, nth) {
        /// Where the token starts, in the input's units.
        offset: usize,
        /// The token's text, in the input's type.
        text: Py<PyAny>,
        /// How many times the token's key occurs in the input.
        count: u32,
        /// Which occurrence of its key this is, counting from 1.
        nth: u32,
        /// Where the key's previous occurrence starts, in the input's units;
        /// `None` at its first.
        previous: Option<usize>,
    }
}

axis_class! {
    /// A key that recurs: a token's text, or the structure of a run of
    /// tokens.
    Echo repr(key, count, offset) {
        /// The key: the text of its first occurrence, in the input's type,
        /// or a structure's role and the kinds of its tokens, a str.
        key: Py<PyAny>,
        /// How many times it occurs.
        count: u32,
        /// The mean distance between occurrences in bytes, where the
        /// distances are regular; `None` where they are not.
        period: Option<f32>,
        /// Where its first occurrence starts, in the input's units.
        offset: usize,
    }
}

axis_class! {
    /// The echo axis over one input: which tokens recur, how often, and how
    /// regularly.
    EchoReport repr(path, group, keyed, distinct, echo_rate) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// The group the keys were read under.
        group: String,
        /// How many tokens the lexer read.
        tokens: usize,
        /// The tokens that take part: words, numbers, quoted runs and typed
        /// literals.
        keyed: usize,
        /// How many distinct keys they hold.
        distinct: usize,
        /// The keyed tokens that are the first occurrence of their key.
        novel: usize,
        /// The keyed tokens whose key occurs more than once.
        echoed: usize,
        /// The share of keyed tokens that are first occurrences, from 0 to 1.
        novelty: f32,
        /// The share of keyed tokens whose key recurs, from 0 to 1.
        echo_rate: f32,
        /// Every key that recurs, most occurrences first.
        echoes: Vec<Py<Echo>>,
        /// The structures that recur among runs of tokens, most occurrences
        /// first, with `structure=True`.
        structures: Vec<Py<Echo>>,
        /// Every keyed token's frame, with `detail=True`.
        frames: Vec<Py<EchoFrame>>,
    }
}

/// The echo axis: which words, numbers and literals recur, how many times,
/// and whether they recur at a regular distance. `group` reads the keys
/// under a symmetry, `"identity"`, the exact text, by default: under
/// `"case"` `Error` and `ERROR` are one key, and under `"subnet/24"` so are
/// all the addresses of one network. `structure=True` adds the structures
/// that recur among supertokens, so `alpha: one` and `bravo: two` are one
/// structure, a key beside a value. A key recurring three or more times has a period
/// where the spread of its distances is at most `max_period_cv` of their
/// mean, 0.3 by default.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, lib = None, group = "identity", structure = false, detail = false, max_period_cv = None))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn echo(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    lib: Option<&Bound<'_, PyAny>>,
    group: &str,
    structure: bool,
    detail: bool,
    max_period_cv: Option<f32>,
) -> PyResult<EchoReport> {
    let (group, orbit) = group_of(group)?;
    let defaults = trex::echo::EchoConfig::default();
    let cfg = trex::echo::EchoConfig {
        orbit,
        max_period_cv: cut("max_period_cv", max_period_cv, defaults.max_period_cv)?,
        ..defaults
    };
    let shapes = crate::shapes_of(lib)?;
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let (tokens, f, rhymes) = py.detach(|| {
        let toks = lexed(bytes, &shapes, false);
        let f = trex::echo::analyze_with(&toks, bytes, &cfg);
        let rhymes = if structure { trex::echo::analyze_super_tokens(&toks, bytes, &cfg) } else { Vec::new() };
        (toks.len(), f, rhymes)
    });
    let span = |r: &trex::echo::EchoFrame| (r.start as usize, r.end as usize);

    // A recurring key's first occurrence carries the key's count and period.
    let mut firsts: Vec<&trex::echo::EchoFrame> =
        f.frames.iter().filter(|r| r.echoed() && r.back_lag.is_none()).collect();
    firsts.sort_by(|a, b| b.count.cmp(&a.count).then(a.start.cmp(&b.start)));
    let starts: Vec<usize> = firsts.iter().map(|r| r.start as usize).collect();
    let echoes = firsts
        .iter()
        .zip(input.offsets(&starts))
        .map(|(r, offset)| {
            let period = if r.period > 0.0 { Some(r.period) } else { None };
            Py::new(py, Echo { key: input.piece(py, span(r)), count: r.count, period, offset })
        })
        .collect::<PyResult<Vec<_>>>()?;

    let firsts: Vec<usize> = rhymes.iter().map(|r| r.first).collect();
    let structures = rhymes
        .iter()
        .zip(input.offsets(&firsts))
        .map(|(r, offset)| {
            let key = PyString::new(py, &r.key).into_any().unbind();
            Py::new(py, Echo { key, count: r.count, period: r.period, offset })
        })
        .collect::<PyResult<Vec<_>>>()?;

    let mut frames = Vec::new();
    if detail {
        let keyed: Vec<&trex::echo::EchoFrame> = f.frames.iter().filter(|r| r.keyed).collect();
        let starts: Vec<usize> = keyed.iter().map(|r| r.start as usize).collect();
        let offsets = input.offsets(&starts);
        for (r, &offset) in keyed.iter().zip(&offsets) {
            // The previous occurrence is an earlier keyed token, so its
            // offset is among those just found.
            let previous = match r.back_lag {
                Some(lag) => {
                    let at = r.start as usize - lag.get() as usize;
                    match starts.binary_search(&at) {
                        Ok(k) => Some(offsets[k]),
                        Err(_) => {
                            return Err(PyRuntimeError::new_err(format!(
                                "trex placed the occurrence before byte {} at byte {at}, which begins no keyed token",
                                r.start
                            )));
                        }
                    }
                }
                None => None,
            };
            frames.push(Py::new(
                py,
                EchoFrame { offset, text: input.piece(py, span(r)), count: r.count, nth: r.nth, previous },
            )?);
        }
    }

    Ok(EchoReport {
        length: input.length(),
        group,
        tokens,
        keyed: f.keyed,
        distinct: f.distinct,
        novel: f.novel,
        echoed: f.echoed,
        novelty: f.novelty(),
        echo_rate: f.echo_rate(),
        echoes,
        structures,
        frames,
        path: input.path,
    })
}

axis_class! {
    /// One token under a symmetry: its text and the orbit it belongs to.
    OrbitToken repr(offset, text, orbit) {
        /// Where the token starts, in the input's units.
        offset: usize,
        /// How many of the input's units it spans.
        length: usize,
        /// The token's text, in the input's type.
        text: Py<PyAny>,
        /// The orbit's representative: the form every token of the orbit
        /// takes under the group.
        orbit: String,
    }
}

axis_class! {
    /// One orbit and the distinct texts that fold onto it.
    OrbitClass repr(orbit, forms) {
        /// The orbit's representative.
        orbit: String,
        /// The distinct texts in the input that fold onto it, sorted, in the
        /// input's type.
        forms: Vec<Py<PyAny>>,
    }
}

axis_class! {
    /// The orbit axis over one input: its tokens read under a symmetry, and
    /// the distinct texts each orbit folds together.
    OrbitReport repr(path, group, tokens, forms, orbits) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// The group the tokens were read under.
        group: String,
        /// How many tokens were read, whitespace left out.
        tokens: usize,
        /// How many distinct texts the tokens hold.
        forms: usize,
        /// How many distinct orbits they fold onto.
        orbits: usize,
        /// Every orbit with the texts that fold onto it, by representative.
        classes: Vec<Py<OrbitClass>>,
        /// The tokens in the same orbit as `same_as`, with `same_as=`.
        matches: Vec<Py<OrbitToken>>,
        /// The runs of one kind of character, letters, digits, whitespace or
        /// the rest, with `detail=True`.
        segments: Vec<Py<Segment>>,
        /// Every token with its orbit, with `detail=True`.
        frames: Vec<Py<OrbitToken>>,
    }
}

/// The orbit axis: each token as the representative of its orbit under a
/// symmetry, and how far the symmetry folds the input's vocabulary. Under
/// `"shape"`, the default, `cat`, `dog` and `bat` are one orbit, a
/// consonant, a vowel and a consonant; under `"case"`, so are `Error` and
/// `ERROR`, and under `"numeric"`, `1e3`, `0x3e8` and `1_000`. `same_as=`, a
/// str or bytes, lists the tokens in the same orbit as it: every token that
/// equals it up to the group.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, lib = None, group = "shape", same_as = None, detail = false))]
pub(crate) fn orbit(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    lib: Option<&Bound<'_, PyAny>>,
    group: &str,
    same_as: Option<&Bound<'_, PyAny>>,
    detail: bool,
) -> PyResult<OrbitReport> {
    let (group, g) = group_of(group)?;
    let key = match same_as {
        Some(q) => Some(trex::orbit::canonical(crate::Input::of(q)?.bytes(), g)),
        None => None,
    };
    let shapes = crate::shapes_of(lib)?;
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let (spans, orbits, cuts) = py.detach(|| {
        let toks = lexed(bytes, &shapes, true);
        let spans: Vec<(usize, usize)> = toks.iter().map(|t| (t.start(), t.end())).collect();
        let orbits: Vec<String> = spans.iter().map(|&(s, e)| trex::orbit::canonical(&bytes[s..e], g)).collect();
        let cuts = if detail { trex::orbit::shape_boundaries(bytes) } else { Vec::new() };
        (spans, orbits, cuts)
    });

    let mut table: BTreeMap<&str, BTreeSet<&[u8]>> = BTreeMap::new();
    for (k, &(s, e)) in spans.iter().enumerate() {
        table.entry(orbits[k].as_str()).or_default().insert(&bytes[s..e]);
    }
    let forms: BTreeSet<&[u8]> = spans.iter().map(|&(s, e)| &bytes[s..e]).collect();
    let classes = table
        .into_iter()
        .map(|(orbit, texts)| {
            let forms = texts.into_iter().map(|t| input.text(py, t)).collect();
            Py::new(py, OrbitClass { orbit: orbit.to_string(), forms })
        })
        .collect::<PyResult<Vec<_>>>()?;

    let token = |k: usize, offset: usize, text: Py<PyAny>| {
        Py::new(py, OrbitToken { offset, length: input.span_length(spans[k]), text, orbit: orbits[k].clone() })
    };
    let hits: Vec<usize> = match &key {
        Some(key) => (0..spans.len()).filter(|&k| &orbits[k] == key).collect(),
        None => Vec::new(),
    };
    Ok(OrbitReport {
        length: input.length(),
        group,
        tokens: spans.len(),
        forms: forms.len(),
        orbits: classes.len(),
        classes,
        matches: input.frames_at(py, &spans, &hits, token)?,
        segments: if detail { segments_between(py, &input, &cuts)? } else { Vec::new() },
        frames: input.frames_at(py, &spans, &all(detail, spans.len()), token)?,
        path: input.path,
    })
}

axis_class! {
    /// One token's shape reading.
    ShapeFrame repr(offset, text, class_) {
        /// Where the token starts, in the input's units.
        offset: usize,
        /// The token's text, in the input's type.
        text: Py<PyAny>,
        /// The token's shape class: its kind and silhouette as one number,
        /// equal for tokens of one shape.
        class_: u32,
        /// The period, in tokens, at which the shapes around the token
        /// repeat; 0 where they do not.
        period: i32,
        /// How strongly the shapes repeat at that period, from 0 to 1: the
        /// share of the tokens in the window behind the token whose shape
        /// equals the shape one period before. A run whose strength reaches
        /// `template_strength` is a region.
        period_strength: f32,
        /// How new the run of shapes ending here is, from 0 to 1, 1 for its
        /// first sighting.
        novelty: f32,
    }
}

axis_class! {
    /// A run of tokens whose shapes repeat at one period: a table, a list of
    /// records, a template filled in again and again.
    ShapeRegion repr(offset, length, period) {
        /// Where the run starts, in the input's units.
        offset: usize,
        /// How many of the input's units it spans.
        length: usize,
        /// The period its shapes repeat at, in tokens.
        period: i32,
    }
}

axis_class! {
    /// The shape axis over one input: the template its tokens repeat, where
    /// that repetition holds, and where it breaks.
    ShapeReport repr(path, tokens, dominant_period, dominant_strength) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// The group the token shapes were read under; `None` for the
        /// lexer's own.
        group: Option<String>,
        /// How many tokens the axis read, whitespace left out.
        tokens: usize,
        /// The period of the strongest repetition, in tokens; 0 where
        /// nothing repeats.
        dominant_period: i32,
        /// How strongly the dominant period repeats, from 0 to 1.
        dominant_strength: f32,
        /// The runs whose shapes repeat with a strength of at least
        /// `template_strength`, 0.6 by default, each ending at the last
        /// token that repeats the kind one period before it.
        regions: Vec<Py<ShapeRegion>>,
        /// Where the silhouette breaks, in the input's units.
        change_points: Vec<usize>,
        /// Every token's frame, with `detail=True`.
        frames: Vec<Py<ShapeFrame>>,
    }
}

/// The shape axis: each token's shape, the period at which the shapes
/// repeat, and the regions that repeat like a table or a list of records. A
/// table reads as a region whether or not its columns line up, since the
/// period is of token shapes and not of bytes. `group=` reads each token's
/// shape under a symmetry, the lexer's own by default. A region is a run
/// whose period repeats with a strength of at least `template_strength`,
/// 0.6 by default.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, lib = None, group = None, detail = false, template_strength = None))]
pub(crate) fn shape(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    lib: Option<&Bound<'_, PyAny>>,
    group: Option<&str>,
    detail: bool,
    template_strength: Option<f32>,
) -> PyResult<ShapeReport> {
    let group = match group {
        Some(g) => Some(group_of(g)?),
        None => None,
    };
    let defaults = trex::shape::ShapeConfig::default();
    let cfg = trex::shape::ShapeConfig {
        template_strength: cut("template_strength", template_strength, defaults.template_strength)?,
        ..defaults
    };
    let shapes = crate::shapes_of(lib)?;
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let f = py.detach(|| {
        let toks = lexed(bytes, &shapes, true);
        match &group {
            Some((_, g)) => trex::shape::analyze_over_with(&toks, bytes, *g, &cfg),
            None => trex::shape::analyze_with(&toks, bytes, &cfg),
        }
    });
    let dominant = f
        .frames
        .iter()
        .filter(|r| r.period > 0)
        .max_by(|a, b| a.period_strength.total_cmp(&b.period_strength))
        .map(|r| (i32::from(r.period), r.period_strength));
    let (dominant_period, dominant_strength) = dominant.unwrap_or((0, 0.0));
    let found = f.shape_regions();
    let ends: Vec<usize> = found.iter().flat_map(|&(s, e, _)| [s, e]).collect();
    let at = input.offsets(&ends);
    let regions = found
        .iter()
        .enumerate()
        .map(|(k, &(_, _, period))| {
            Py::new(
                py,
                ShapeRegion { offset: at[2 * k], length: at[2 * k + 1] - at[2 * k], period: i32::from(period) },
            )
        })
        .collect::<PyResult<Vec<_>>>()?;
    let frame = |i: usize, offset: usize, text: Py<PyAny>| {
        let r = &f.frames[i];
        Py::new(
            py,
            ShapeFrame {
                offset,
                text,
                class_: r.class,
                period: i32::from(r.period),
                period_strength: r.period_strength,
                novelty: r.novelty,
            },
        )
    };
    Ok(ShapeReport {
        length: input.length(),
        group: group.map(|(name, _)| name),
        tokens: f.n_tokens,
        dominant_period,
        dominant_strength,
        regions,
        change_points: input.offsets(&f.boundaries),
        frames: input.frames_at(py, &f.spans, &all(detail, f.frames.len()), frame)?,
        path: input.path,
    })
}

axis_class! {
    /// One unit's gravity reading: how its past pushes it away, how strongly
    /// the input holds together across the cut before it, and its type's
    /// class.
    GravityFrame repr(offset, text, strain, bound) {
        /// The unit's offset, in the input's units; each byte of a
        /// character written in several has a frame at the character's
        /// offset.
        offset: usize,
        /// The unit's text, in the input's type: at the byte grain the
        /// character the byte begins, `""` for a byte inside a character
        /// written in several, and of bytes the byte itself; the token or
        /// the supertoken otherwise.
        text: Py<PyAny>,
        /// The unit's mean potential against each unit before it within the
        /// field's reach, in bits: high where what came before pushes it away.
        /// `None` at the first unit, which has nothing before it.
        strain: Option<f32>,
        /// The share of the input's strain readings below this one, in
        /// percent.
        strain_percentile: Option<f64>,
        /// The attraction across the cut before the unit, the mean `log2 g`
        /// over the pairs straddling it within 8 units, in bits per pair: low
        /// where the two sides hold together least. `None` at the first unit,
        /// which has no cut before it.
        bound: Option<f32>,
        /// The share of the input's bound readings below this one, in percent.
        bound_percentile: Option<f64>,
        /// The gravity class of the unit's type; `None` where no significant
        /// cell places it, so it is kin to nothing but itself.
        class_: Option<u16>,
    }
}

axis_class! {
    /// One gravity class: types the pair field places together, kin to one
    /// another under `@kin`.
    GravityClass repr(class_, units) {
        /// The class's number, from 0 to 15.
        class_: u16,
        /// Its types, most frequent first: at the byte grain each byte, as
        /// itself where it is printable ASCII, `' '` for a space and an escape
        /// otherwise; a token's kind with a punctuation mark's text; a
        /// supertoken's role and kinds.
        types: Vec<String>,
        /// How many of the grain's units are of its types.
        units: usize,
    }
}

axis_class! {
    /// The pair field over one input: where each unit's past pushes it away,
    /// where the input holds together least, and which types it treats alike.
    GravityReport repr(path, grain, units, types) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// The stream read: `"token"`, `"byte"` or `"super"` for the
        /// supertokens.
        grain: String,
        /// How many units the field read at that grain.
        units: usize,
        /// How many distinct types those units are of.
        types: usize,
        /// The units under the most strain, most first, `top` of them.
        strained: Vec<Py<GravityFrame>>,
        /// The units before the cuts held together least, least first, `top`
        /// of them.
        weakest: Vec<Py<GravityFrame>>,
        /// The classes the field places types in, ascending.
        classes: Vec<Py<GravityClass>>,
        /// Every unit's frame, with `detail=True`.
        frames: Vec<Py<GravityFrame>>,
    }
}

/// The pair field: how much more or less often each type of unit follows
/// another at each gap than chance alone would, and three readings taken
/// from it.
/// A unit's strain is its mean potential against the units before it, high
/// where its past pushes it away; the bound of the cut before it is the
/// attraction across that cut, low where the input holds together least; and
/// its class groups the types the field treats alike. `grain="token"` reads
/// the significant tokens, `"byte"` the bytes and `"super"` the supertokens,
/// the token and super grains lexed under `lib=`'s declarations where it is
/// given; the byte grain reads bytes and takes no library. `top=` sizes the
/// lists of the most strained units and the weakest cuts.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, lib = None, grain = "token", top = 8, detail = false))]
pub(crate) fn gravity(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    lib: Option<&Bound<'_, PyAny>>,
    grain: &str,
    top: usize,
    detail: bool,
) -> PyResult<GravityReport> {
    use trex::ast::Grain;
    let Some(unit) = Grain::parse(grain) else {
        return Err(PyValueError::new_err(format!("grain takes 'byte', 'token' or 'super', not {grain:?}")));
    };
    if unit == Grain::Byte && lib.is_some() {
        return Err(PyValueError::new_err(
            "lib= decides how tokens are read, and the byte grain reads bytes; grain='token' or 'super' reads them",
        ));
    }
    let shapes = crate::shapes_of(lib)?;
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let r = py.detach(|| {
        let toks = if unit == Grain::Byte { Vec::new() } else { lexed(bytes, &shapes, false) };
        trex::gravity::Readings::read(unit, bytes, &toks)
    });
    let chars = if input.as_bytes {
        None
    } else {
        Some(std::str::from_utf8(bytes).map_err(|e| PyRuntimeError::new_err(format!("the text read is not UTF-8: {e}")))?)
    };
    let frames_of = |at: &[usize]| -> PyResult<Vec<Py<GravityFrame>>> {
        let starts: Vec<usize> = at.iter().map(|&u| r.start_of(u)).collect();
        input
            .offsets(&starts)
            .into_iter()
            .zip(at)
            .map(|(offset, &u)| {
                let text = match (unit, chars) {
                    (Grain::Token | Grain::Super, _) => input.piece(py, r.span_of(u)),
                    (Grain::Byte, None) => PyBytes::new(py, &bytes[u..u + 1]).into_any().unbind(),
                    (Grain::Byte, Some(s)) => {
                        let c = match s.get(u..).and_then(|rest| rest.chars().next()) {
                            Some(c) => c.to_string(),
                            None => String::new(),
                        };
                        PyString::new(py, &c).into_any().unbind()
                    }
                };
                let reading = r.reading_of(u);
                Py::new(
                    py,
                    GravityFrame {
                        offset,
                        text,
                        strain: reading.strain,
                        strain_percentile: reading.strain_percentile,
                        bound: reading.bound,
                        bound_percentile: reading.bound_percentile,
                        class_: r.class_of(r.type_of(u)),
                    },
                )
            })
            .collect()
    };
    let classes = r
        .classes()
        .into_iter()
        .map(|g| {
            Py::new(
                py,
                GravityClass { class_: g.class, types: g.types.iter().map(|&t| r.type_label(t)).collect(), units: g.units },
            )
        })
        .collect::<PyResult<Vec<_>>>()?;
    let every: Vec<usize> = if detail { (0..r.len()).collect() } else { Vec::new() };
    Ok(GravityReport {
        length: input.length(),
        grain: grain.to_string(),
        units: r.len(),
        types: r.types(),
        strained: frames_of(&r.most_strained(top))?,
        weakest: frames_of(&r.weakest_cuts(top))?,
        classes,
        frames: frames_of(&every)?,
        path: input.path,
    })
}

axis_class! {
    /// A context's scale: the orders of magnitude of the tokens it folds.
    ContextMagnitude repr(mean, spread, max) {
        /// Their mean.
        mean: f32,
        /// Their standard deviation, the spread an `s` predicate counts in.
        spread: f32,
        /// The largest.
        max: f32,
    }
}

axis_class! {
    /// A context's load: the bracket depth of the tokens it folds.
    ContextStress repr(mean, max) {
        /// The mean depth.
        mean: f32,
        /// The deepest.
        max: u16,
    }
}

axis_class! {
    /// A context's texture: the spectral reading of the tokens it folds.
    ContextSpectral repr(entropy, period) {
        /// The mean pooled byte-class entropy, from 0 to 1.
        entropy: f32,
        /// The byte period of the token whose period repeats most strongly;
        /// 0 for none.
        period: u16,
        /// How strongly that period repeats, from 0 to 1.
        strength: f32,
    }
}

axis_class! {
    /// A context's recurrence: how many of the tokens it folds are first
    /// sightings and how many recur.
    ContextEcho repr(novel, echoed) {
        /// Tokens whose content occurs here for the first time.
        novel: u32,
        /// Tokens whose content recurs elsewhere in the input.
        echoed: u32,
    }
}

axis_class! {
    /// A context's vantage: what the bytes before, after and around the
    /// tokens it folds read, and how far before and after disagree.
    ContextObservation repr(disagreement, contested) {
        /// The mean reading of the window before each token, from 0 to 1.
        causal: f32,
        /// The mean reading of the window after each token, from 0 to 1.
        anticausal: f32,
        /// The mean reading of the window centered on each token, from 0 to 1.
        centered: f32,
        /// The mean disagreement between before and after.
        disagreement: f32,
        /// The largest disagreement.
        disagreement_max: f32,
        /// Tokens spanning a contested point.
        contested: u32,
    }
}

axis_class! {
    /// A context's segmentation: how strongly the tokens it folds align with
    /// seams, read in both directions.
    ContextSeam repr(strength, cuts) {
        /// The mean forward branching entropy.
        forward: f32,
        /// The mean backward branching entropy.
        backward: f32,
        /// The mean seam strength.
        strength: f32,
        /// The strongest seam.
        strength_max: f32,
        /// Tokens starting a segment.
        cuts: u32,
    }
}

axis_class! {
    /// A context's dynamics: the trend of magnitude over the tokens it folds.
    ContextFlow repr(slope, reversals) {
        /// The mean windowed slope.
        slope: f32,
        /// Tokens whose trend rises.
        rising: u32,
        /// Tokens whose trend falls.
        falling: u32,
        /// Tokens whose trend is steady.
        steady: u32,
        /// The longest run any token's trend has kept its direction.
        momentum: u16,
        /// Changes of direction between adjacent tokens.
        reversals: u32,
    }
}

axis_class! {
    /// One token's context: how many tokens it folds and each axis's reading
    /// over them.
    ContextFrame repr(offset, text, over) {
        /// Where the token starts, in the input's units.
        offset: usize,
        /// The token's text, in the input's type.
        text: Py<PyAny>,
        /// The token's column of the record period; `None` in a stream with no
        /// period.
        phase: Option<u16>,
        /// How many tokens the context folds; 0 where it holds none, and then
        /// every axis below is `None`.
        over: u32,
        /// Their scale.
        magnitude: Option<Py<ContextMagnitude>>,
        /// Their bracket depth.
        stress: Option<Py<ContextStress>>,
        /// Their texture.
        spectral: Option<Py<ContextSpectral>>,
        /// Their recurrence.
        echo: Option<Py<ContextEcho>>,
        /// Their vantage.
        observation: Option<Py<ContextObservation>>,
        /// Their segmentation.
        seam: Option<Py<ContextSeam>>,
        /// Their dynamics.
        flow: Option<Py<ContextFlow>>,
    }
}

axis_class! {
    /// How consistently each lower grain's boundaries are at one offset from
    /// the starts of supertokens of one role: the share at their role's most
    /// common offset, from 0 to 1.
    ContextAlignment repr(regime, shape, seam) {
        /// The spectral change points.
        regime: f32,
        /// The shape change points.
        shape: f32,
        /// The seam cuts.
        seam: f32,
    }
}

axis_class! {
    /// One supertoken's role and how far from its start, in significant
    /// tokens, each lower grain's nearest boundary is: negative before it.
    ContextAgreement repr(offset, role) {
        /// Where the supertoken starts, in the input's units.
        offset: usize,
        /// Its text, in the input's type.
        text: Py<PyAny>,
        /// Its role: `"call"`, `"assign"`, `"kv"`, `"list"`, `"numeric"` or
        /// `"plain"`.
        role: String,
        /// The nearest spectral change point; `None` where there is none.
        regime: Option<i32>,
        /// The nearest shape change point; `None` where there is none.
        shape: Option<i32>,
        /// The nearest seam cut; `None` where there is none.
        seam: Option<i32>,
    }
}

axis_class! {
    /// The context axis over one input: what each token is read against.
    ContextReport repr(path, fold, tokens, units, period) {
        /// The file read; `None` for a text.
        path: Option<String>,
        /// The input's length in its units.
        length: usize,
        /// The context read: `"window"`, `"unit"`, `"units"`, `"enclosing"`,
        /// `"echo"`, `"regime"`, `"phase"` or `"key"`.
        fold: String,
        /// How many tokens the axis read, whitespace left out.
        tokens: usize,
        /// How many supertokens they form.
        units: usize,
        /// The record period in tokens, the lag the stream's token shapes
        /// repeat at most; `None` where none clears chance.
        period: Option<u16>,
        /// Every lag that clears chance as the period does, strongest first.
        live_periods: Vec<u16>,
        /// How consistently the lower grains' boundaries align with the
        /// supertokens.
        alignment: Py<ContextAlignment>,
        /// Every supertoken's role and nearest boundaries, with `detail=True`.
        agreement: Vec<Py<ContextAgreement>>,
        /// Every token's context, with `detail=True`.
        frames: Vec<Py<ContextFrame>>,
    }
}

/// The context axis: each token read against what surrounds it. `fold=` names
/// the context as `\N{>+1:F}` names it: `"window"`, the tokens before it;
/// `"unit"`, the supertoken holding it; `"units"`, the window of supertokens
/// ending there; `"enclosing"`, the heads of the brackets around it; `"echo"`,
/// its key's earlier occurrences; `"regime"`, the tokens since the texture
/// last changed; `"phase"`, the earlier tokens at its column of the record
/// period; `"key"`, the values its key was bound to before. Every frame
/// carries each axis's reading over the tokens its context folds.
/// `token_window=` and `unit_window=` set the windows, 32 tokens and 8
/// supertokens by default, and `lib=` the declarations the lex reads.
#[pyfunction]
#[pyo3(signature = (text = None, *, path = None, lib = None, fold = "window", token_window = None, unit_window = None, detail = false))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn context(
    py: Python<'_>,
    text: Option<&Bound<'_, PyAny>>,
    path: Option<PathBuf>,
    lib: Option<&Bound<'_, PyAny>>,
    fold: &str,
    token_window: Option<usize>,
    unit_window: Option<usize>,
    detail: bool,
) -> PyResult<ContextReport> {
    let Some(which) = trex::context::Fold::parse(fold) else {
        return Err(PyValueError::new_err(format!(
            "fold takes {}, not {fold:?}",
            trex::context::Fold::NAMES.map(|n| format!("'{n}'")).join(", ")
        )));
    };
    let defaults = trex::context::ContextConfig::default();
    let cfg = trex::context::ContextConfig {
        token_window: token_window.unwrap_or(defaults.token_window),
        unit_window: unit_window.unwrap_or(defaults.unit_window),
    };
    if cfg.token_window == 0 || cfg.unit_window == 0 {
        return Err(PyValueError::new_err(
            "a window folds at least one unit: token_window= and unit_window= take 1 or more",
        ));
    }
    let shapes = crate::shapes_of(lib)?;
    let input = AxisInput::read(py, text, path)?;
    let bytes = &input.bytes[..];
    let (toks, c) = py.detach(|| {
        let toks = lexed(bytes, &shapes, false);
        let c = trex::context::Contexts::read(&toks, bytes, &cfg);
        (toks, c)
    });
    let sig: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].is_significant()).collect();
    let frames = if detail {
        let starts: Vec<usize> = sig.iter().map(|&t| toks[t].start()).collect();
        let at = input.offsets(&starts);
        sig.iter()
            .zip(at)
            .map(|(&t, offset)| {
                let p = c.at(which, t);
                let over = p.magnitude.count;
                let some = over > 0;
                Py::new(
                    py,
                    ContextFrame {
                        offset,
                        text: input.piece(py, (toks[t].start(), toks[t].end())),
                        phase: c.phase_of(t),
                        over,
                        magnitude: some
                            .then(|| {
                                Py::new(
                                    py,
                                    ContextMagnitude {
                                        mean: p.magnitude.mean(),
                                        spread: p.magnitude.std_dev(),
                                        max: p.magnitude.max,
                                    },
                                )
                            })
                            .transpose()?,
                        stress: some
                            .then(|| Py::new(py, ContextStress { mean: p.stress.mean_depth(), max: p.stress.max_depth }))
                            .transpose()?,
                        spectral: some
                            .then(|| {
                                Py::new(
                                    py,
                                    ContextSpectral {
                                        entropy: p.spectral.mean_entropy(),
                                        period: p.spectral.period,
                                        strength: p.spectral.period_strength,
                                    },
                                )
                            })
                            .transpose()?,
                        echo: some
                            .then(|| Py::new(py, ContextEcho { novel: p.echo.novel, echoed: p.echo.echoed }))
                            .transpose()?,
                        observation: some
                            .then(|| {
                                Py::new(
                                    py,
                                    ContextObservation {
                                        causal: p.observation.mean_causal(),
                                        anticausal: p.observation.mean_anticausal(),
                                        centered: p.observation.mean_centered(),
                                        disagreement: p.observation.mean_disagreement(),
                                        disagreement_max: p.observation.disagreement_max,
                                        contested: p.observation.contested,
                                    },
                                )
                            })
                            .transpose()?,
                        seam: some
                            .then(|| {
                                Py::new(
                                    py,
                                    ContextSeam {
                                        forward: p.seam.mean_fwd(),
                                        backward: p.seam.mean_bwd(),
                                        strength: p.seam.mean_strength(),
                                        strength_max: p.seam.strength_max,
                                        cuts: p.seam.cuts,
                                    },
                                )
                            })
                            .transpose()?,
                        flow: some
                            .then(|| {
                                Py::new(
                                    py,
                                    ContextFlow {
                                        slope: p.flow.mean_slope(),
                                        rising: p.flow.rising,
                                        falling: p.flow.falling,
                                        steady: p.flow.steady,
                                        momentum: p.flow.momentum_max,
                                        reversals: p.flow.reversals,
                                    },
                                )
                            })
                            .transpose()?,
                    },
                )
            })
            .collect::<PyResult<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let agreement = if detail {
        let units = &c.supers.units;
        let starts: Vec<usize> = units.iter().map(|u| u.start).collect();
        let at = input.offsets(&starts);
        units
            .iter()
            .zip(&c.field.agreement)
            .zip(at)
            .map(|((u, a), offset)| {
                Py::new(
                    py,
                    ContextAgreement {
                        offset,
                        text: input.piece(py, (u.start, u.end)),
                        role: a.role.label().to_string(),
                        regime: a.regime,
                        shape: a.shape,
                        seam: a.seam,
                    },
                )
            })
            .collect::<PyResult<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let (regime, shape, seam) = c.field.alignment();
    Ok(ContextReport {
        length: input.length(),
        fold: which.name().to_string(),
        tokens: sig.len(),
        units: c.supers.units.len(),
        period: c.related.period,
        live_periods: c.live_periods.clone(),
        alignment: Py::new(py, ContextAlignment { regime, shape, seam })?,
        agreement,
        frames,
        path: input.path,
    })
}

/// Adds the axis functions and their result classes to `m`, the `trex.axes`
/// module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(magnitude, m)?)?;
    m.add_function(wrap_pyfunction!(stress, m)?)?;
    m.add_function(wrap_pyfunction!(flow, m)?)?;
    m.add_function(wrap_pyfunction!(observation, m)?)?;
    m.add_function(wrap_pyfunction!(relation, m)?)?;
    m.add_function(wrap_pyfunction!(spectral, m)?)?;
    m.add_function(wrap_pyfunction!(seam, m)?)?;
    m.add_function(wrap_pyfunction!(echo, m)?)?;
    m.add_function(wrap_pyfunction!(orbit, m)?)?;
    m.add_function(wrap_pyfunction!(shape, m)?)?;
    m.add_function(wrap_pyfunction!(gravity, m)?)?;
    m.add_function(wrap_pyfunction!(context, m)?)?;
    m.add_class::<MagnitudeFrame>()?;
    m.add_class::<MagnitudeReport>()?;
    m.add_class::<StressFrame>()?;
    m.add_class::<StressReport>()?;
    m.add_class::<FlowFrame>()?;
    m.add_class::<AnalyticFrame>()?;
    m.add_class::<FlowReport>()?;
    m.add_class::<ObservationFrame>()?;
    m.add_class::<ObservationReport>()?;
    m.add_class::<RelationFrame>()?;
    m.add_class::<RelationEdge>()?;
    m.add_class::<RelationChord>()?;
    m.add_class::<RelationReport>()?;
    m.add_class::<SpectralFrame>()?;
    m.add_class::<SpectralPeriod>()?;
    m.add_class::<TextureRegion>()?;
    m.add_class::<ClassifiedRegion>()?;
    m.add_class::<SpectralReport>()?;
    m.add_class::<Segment>()?;
    m.add_class::<SeamFrame>()?;
    m.add_class::<SeamReport>()?;
    m.add_class::<EchoFrame>()?;
    m.add_class::<Echo>()?;
    m.add_class::<EchoReport>()?;
    m.add_class::<OrbitToken>()?;
    m.add_class::<OrbitClass>()?;
    m.add_class::<OrbitReport>()?;
    m.add_class::<ShapeFrame>()?;
    m.add_class::<ShapeRegion>()?;
    m.add_class::<ShapeReport>()?;
    m.add_class::<GravityFrame>()?;
    m.add_class::<GravityClass>()?;
    m.add_class::<GravityReport>()?;
    m.add_class::<ContextMagnitude>()?;
    m.add_class::<ContextStress>()?;
    m.add_class::<ContextSpectral>()?;
    m.add_class::<ContextEcho>()?;
    m.add_class::<ContextObservation>()?;
    m.add_class::<ContextSeam>()?;
    m.add_class::<ContextFlow>()?;
    m.add_class::<ContextFrame>()?;
    m.add_class::<ContextAlignment>()?;
    m.add_class::<ContextAgreement>()?;
    m.add_class::<ContextReport>()?;
    Ok(())
}
