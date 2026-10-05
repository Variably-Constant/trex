//! GPU SIMT scan backend (the default `gpu` feature; opt out with
//! `--no-default-features`).
//!
//! The all-starts scan is data-parallel: the longest match anchored at
//! each token position is an independent computation. This backend maps
//! one anchor to one GPU thread, each running a bit-parallel NFA
//! simulation over the token-kind stream (see `kernels/scan.cu`), then
//! selects the leftmost, non-overlapping matches on the host exactly as
//! the CPU engine does. The result is identical to [`crate::scan`].
//!
//! The kernel handles the regular, alternation-free, capture-free subset
//! of patterns (typed-kind atoms and `.`, with concatenation and
//! quantifiers). [`gpu_eligible`] reports membership; every other
//! pattern, and the case of no device or a build without the `gpu`
//! feature, returns `None` from [`scan_gpu`] so the caller falls back to
//! the CPU engine.

use crate::ast::{Atom, Greed, Pattern};
use crate::engine::Span;
use crate::token::{Token, TokenKind};

// The device coder behind `trex compress --gpu`, compiled with the `compress`
// feature; its public items keep their `trex::gpu::` paths through the
// re-export.
#[cfg(feature = "compress")]
mod coder;
#[cfg(feature = "compress")]
pub use coder::*;

/// Whether `pattern` is in the device subset: typed-kind atoms, `.`, literals,
/// byte classes other than whitespace, absolute magnitude tests, spectral
/// tests, and one-token binds that every reference reads at one fixed
/// distance, with every literal and reference under one orbit group, composed
/// with concatenation and quantifiers. Alternation, any other bind, a bind
/// beside a spectral test, balance, field, content guard, relative magnitude,
/// byte patterns and set expressions are excluded, so the device and the CPU
/// never disagree on what the device accepts.
#[must_use]
pub fn gpu_eligible(pattern: &Pattern) -> bool {
    eligible_shape(pattern)
        && (!compares_classes(pattern) || crate::nfa::device_binds_fit(pattern))
        // A binding pattern's captures are resolved on the host by the linear
        // engine, which evaluates no spectral atom, so a bind and a spectral
        // test do not share a device pattern.
        && !(pattern.has_spectral() && pattern.binds())
}

/// Whether a bind, a register reference or a literal appears anywhere in
/// `pattern`: the atoms whose orbit groups must agree, and whose references
/// must be at fixed offsets, for the device to take the pattern.
fn compares_classes(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::Bind(..) | Pattern::Atom(Atom::RegisterEq(..) | Atom::Literal(..)) => true,
        Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(compares_classes),
        Pattern::Star(p, _)
        | Pattern::Plus(p, _)
        | Pattern::Opt(p, _)
        | Pattern::Repeat(p, _, _, _)
        | Pattern::Atomic(p) => compares_classes(p),
        _ => false,
    }
}

/// Whether the auto routers may place `pattern` on the device: it is in the device
/// subset and reads only each token's kind. A pattern that tests a
/// magnitude, a byte class, a literal or a register scans correctly on the
/// device, but a per-call scan measured 0.98x the CPU engine for magnitude
/// tests and 0.78x for back-references over 16 MB, so such a pattern reaches
/// the device only when a caller asks for it, through [`scan_gpu`] or tokens
/// held across scans ([`GpuTokens`]).
#[must_use]
pub fn gpu_auto_routes(pattern: &Pattern) -> bool {
    gpu_eligible(pattern) && !reads_token_properties(pattern)
}

/// Whether `pattern` reads a property of a token beyond its kind.
fn reads_token_properties(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::Bind(..)
        | Pattern::Atom(
            Atom::Magnitude(_)
            | Atom::KindMag(..)
            | Atom::KindPred(..)
            | Atom::Byte(_)
            | Atom::Literal(..)
            | Atom::RegisterEq(..)
            | Atom::RegisterRelated(..)
            | Atom::LiteralWithin(..)
            | Atom::RegisterWithin(..)
            | Atom::RegisterKin(..)
            | Atom::Since(..)
            | Atom::Spectral(_),
        ) => true,
        Pattern::Concat(v) | Pattern::Alt(v, _) => v.iter().any(reads_token_properties),
        Pattern::Star(p, _)
        | Pattern::Plus(p, _)
        | Pattern::Opt(p, _)
        | Pattern::Repeat(p, _, _, _)
        | Pattern::Atomic(p) => reads_token_properties(p),
        _ => false,
    }
}

/// The pattern shapes the device kernels implement, before [`gpu_eligible`]
/// checks where a binding pattern's references are.
fn eligible_shape(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::Empty => true,
        // Whitespace atoms are excluded: the device kernel runs over the
        // significant-token stream and never sees a whitespace token.
        Pattern::Atom(Atom::Kind(crate::token::TokenKind::Whitespace)) => false,
        // A user-declared shape decides token boundaries, and the device path
        // lexes for itself with no shape set, so its token stream would not
        // contain the token this atom names. Declining is the deliberate
        // answer; accepting would return a confidently wrong empty result.
        Pattern::Atom(Atom::Kind(crate::token::TokenKind::Custom(_))) => false,
        Pattern::Atom(Atom::Kind(_) | Atom::Any) => true,
        // An absolute magnitude test is a fixed threshold on one token, checked
        // against a magnitude uploaded per token. A relative one takes its
        // threshold from the tokens before, which the device does not carry.
        Pattern::Atom(Atom::Magnitude(p)) => p.scope().is_none(),
        Pattern::Atom(Atom::KindMag(k, p)) => {
            p.scope().is_none()
                && !matches!(k, crate::token::TokenKind::Whitespace | crate::token::TokenKind::Custom(_))
        }
        Pattern::Atom(Atom::RegisterEq(..) | Atom::Literal(..) | Atom::Spectral(_)) => true,
        Pattern::Atom(Atom::Byte(bc)) => crate::nfa::byte_class_bit(*bc).is_some(),
        Pattern::Atom(_) => false,
        // Atomic discards the lengths the body did not prefer, and the device
        // kernel has no preference order to discard them by. An edit-distance
        // group offers several lengths with a preference over them, which is
        // the same absence read the other way.
        Pattern::Atomic(_) | Pattern::Within(..) => false,
        Pattern::Concat(v) => v.iter().all(eligible_shape),
        Pattern::Bind(_, _, p) => eligible_shape(p),
        // Laziness changes which accepted length is reported, and the device
        // kernel has no preference order, so only greedy quantifiers are in
        // the subset.
        Pattern::Star(p, Greed::Greedy)
        | Pattern::Plus(p, Greed::Greedy)
        | Pattern::Opt(p, Greed::Greedy)
        | Pattern::Repeat(p, _, _, Greed::Greedy) => {
            eligible_shape(p)
        }
        Pattern::Star(_, Greed::Lazy)
        | Pattern::Plus(_, Greed::Lazy)
        | Pattern::Opt(_, Greed::Lazy)
        | Pattern::Repeat(_, _, _, Greed::Lazy)
        | Pattern::Alt(..)
        | Pattern::Balanced(..)
        | Pattern::Guard(..)
        | Pattern::Assert(..)
        | Pattern::Field(..)
        | Pattern::Anchor(_) => false,
    }
}

/// Scan `input` for `pattern` on the GPU, or `None` when the GPU path
/// does not apply: the pattern is outside the device subset, the `gpu`
/// feature is off, or no usable device is present. On `None` the caller
/// runs the CPU engine, which returns identical matches.
#[must_use]
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
pub fn scan_gpu(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
    if !gpu_eligible(pattern) {
        return None;
    }
    #[cfg(feature = "gpu")]
    {
        cuda::scan(pattern, input)
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

/// The kernel's per-anchor automaton run on the host over `kinds`, filling
/// `out` with the longest match end of every anchor in `lo..lo + out.len()`
/// across the host's cores: the share a split's host half computes, exposed
/// so it can be timed against another way of computing the same table.
///
/// `false` where the pattern is outside the device subset or the `gpu`
/// feature is off, in which case `out` is untouched.
#[must_use]
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
pub fn port_anchor_ends(pattern: &Pattern, kinds: &[u32], lo: usize, out: &mut [i32]) -> bool {
    #[cfg(feature = "gpu")]
    {
        let Some(nfa) = crate::nfa::compile_for_gpu(pattern) else {
            return false;
        };
        if nfa.reads_magnitude
            || nfa.binds_registers
            || nfa.class_group.is_some()
            || nfa.reads_bytes
            || nfa.reads_spectral
        {
            return false;
        }
        cuda::host_anchor_ends(&nfa, kinds, lo, out);
        true
    }
    #[cfg(not(feature = "gpu"))]
    {
        false
    }
}

/// A text's significant token kinds held on the device, so every scan over it
/// sends only its pattern's tables and a result buffer. The token spans stay on
/// the host, where the match selection reads them.
pub struct GpuTokens {
    #[cfg(feature = "gpu")]
    kinds: cuda::ResidentKinds,
    /// The same kind codes on the host, which the host's share of a split scan
    /// over these tokens reads.
    #[cfg(feature = "gpu")]
    host_kinds: Vec<u32>,
    spans: Vec<(u32, u32)>,
    #[cfg(feature = "gpu")]
    text: Option<HeldText>,
}

/// What a scan over held tokens needs on the host to resolve a literal's
/// class: the class ids the held tokens carry.
#[cfg(feature = "gpu")]
struct HeldText {
    classes: ClassIds,
}

impl GpuTokens {
    /// Lex `input` and hold every per-token property a pattern can read on the
    /// device - kind, magnitude, byte-class mask, and orbit class under
    /// `group` - with the class ids kept on the host, so literal, byte-class,
    /// magnitude and binding patterns scan the held tokens too. `None` when
    /// the `gpu` feature is off or no usable device is present.
    #[must_use]
    #[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
    pub fn upload_with_properties(input: &[u8], group: crate::orbit::OrbitGroup) -> Option<Self> {
        #[cfg(feature = "gpu")]
        {
            let toks = crate::parallel_lex::lex_parallel(input);
            let (kinds, spans) = significant_stream(&toks);
            let mags = significant_magnitudes(&toks, input);
            let masks = significant_byte_masks(&toks, input);
            let mut classes = ClassIds::new(group);
            let ids: Vec<u32> = toks
                .iter()
                .filter(|t| t.kind != TokenKind::Whitespace)
                .map(|t| classes.id_of(&input[t.start()..t.end()]))
                .collect();
            let held = cuda::upload_properties(&kinds, &mags, &masks, &ids, None)?;
            Some(Self { kinds: held, host_kinds: kinds, spans, text: Some(HeldText { classes }) })
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }

    /// [`Self::upload_with_properties`], holding each significant token's
    /// pooled spectral reading as well, so a pattern that tests the spectral
    /// field scans the held tokens too. The field is computed once here.
    #[must_use]
    #[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
    pub fn upload_with_spectral(input: &[u8], group: crate::orbit::OrbitGroup) -> Option<Self> {
        #[cfg(feature = "gpu")]
        {
            let toks = crate::parallel_lex::lex_parallel(input);
            let (kinds, spans) = significant_stream(&toks);
            let mags = significant_magnitudes(&toks, input);
            let masks = significant_byte_masks(&toks, input);
            let mut classes = ClassIds::new(group);
            let ids: Vec<u32> = toks
                .iter()
                .filter(|t| t.kind != TokenKind::Whitespace)
                .map(|t| classes.id_of(&input[t.start()..t.end()]))
                .collect();
            let (entropy, packed) = significant_spectral(&toks, &crate::spectral::analyze(input));
            let held = cuda::upload_properties(&kinds, &mags, &masks, &ids, Some((&entropy, &packed)))?;
            Some(Self { kinds: held, host_kinds: kinds, spans, text: Some(HeldText { classes }) })
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }

    /// Lex `input` and hold its significant token kinds on the device, or
    /// `None` when the `gpu` feature is off or no usable device is present.
    /// A pattern that tests a magnitude needs [`Self::upload_with_magnitudes`].
    #[must_use]
    #[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
    pub fn upload(input: &[u8]) -> Option<Self> {
        #[cfg(feature = "gpu")]
        {
            let (kinds, spans) = crate::parallel_lex::lex_significant_parallel(input);
            Some(Self { kinds: cuda::upload_kinds(&kinds, None)?, host_kinds: kinds, spans, text: None })
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }

    /// [`Self::upload`], holding each significant token's magnitude as well,
    /// so a pattern that tests one scans on the device too.
    #[must_use]
    pub fn upload_with_magnitudes(input: &[u8]) -> Option<Self> {
        Self::hold(&crate::parallel_lex::lex_parallel(input), Some(input))
    }

    /// Hold the significant token kinds of `toks` on the device, or `None`
    /// when the `gpu` feature is off or no usable device is present.
    #[must_use]
    pub fn from_tokens(toks: &[Token]) -> Option<Self> {
        Self::hold(toks, None)
    }

    /// Hold the significant kinds of `toks`, and their magnitudes when
    /// `input`, the bytes they were lexed from, is given.
    #[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
    fn hold(toks: &[Token], input: Option<&[u8]>) -> Option<Self> {
        #[cfg(feature = "gpu")]
        {
            let (kinds, spans) = significant_stream(toks);
            let mags = input.map(|bytes| significant_magnitudes(toks, bytes));
            Some(Self { kinds: cuda::upload_kinds(&kinds, mags.as_deref())?, host_kinds: kinds, spans, text: None })
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }

    /// Significant tokens held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// True when the text has no significant token.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Scan the held tokens for `pattern`: the matches [`scan_gpu`] returns on
    /// the same input, or `None` when the pattern is outside the device subset,
    /// reads a property these tokens were held without - a magnitude, a byte
    /// class, a literal, a register or the spectral reading, or an orbit group
    /// other than the one they were held under - or the device call fails.
    #[must_use]
    #[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
    pub fn scan(&self, pattern: &Pattern) -> Option<Vec<Span>> {
        if !gpu_eligible(pattern) {
            return None;
        }
        #[cfg(feature = "gpu")]
        {
            cuda::scan_resident(&self.kinds, &self.spans, self.text.as_ref(), pattern)
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }

    /// Scan the held tokens for `pattern` with the anchors split between the
    /// host's cores and the device, both at once and neither lexing or
    /// uploading: the device scans the kinds from its share's start out of the
    /// buffer it holds, the host runs the same per-anchor automaton over the
    /// kinds kept beside them, and one thread selects over the joined table.
    /// `host_per_mille` is the host's share of the anchors, clamped to leave
    /// each side at least one. The matches are [`Self::scan`]'s; `None` in the
    /// cases that returns `None`, and for a pattern that binds or reads any
    /// per-token property.
    #[must_use]
    #[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
    pub fn scan_split(&self, pattern: &Pattern, host_per_mille: u32) -> Option<Vec<Span>> {
        if !gpu_eligible(pattern) {
            return None;
        }
        #[cfg(feature = "gpu")]
        {
            cuda::scan_resident_split(&self.kinds, &self.host_kinds, &self.spans, pattern, host_per_mille)
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }

    /// [`Self::scan_split`] with its phases timed, for
    /// `benches/gpu_throughput`: the per-anchor ends both sides compute at
    /// once, and the host selection over the joined table. The spans are
    /// [`Self::scan_split`]'s, and `None` in the cases it returns `None`.
    #[must_use]
    #[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
    pub fn scan_split_phases(
        &self,
        pattern: &Pattern,
        host_per_mille: u32,
    ) -> Option<(Vec<Span>, SplitPhases)> {
        if !gpu_eligible(pattern) {
            return None;
        }
        #[cfg(feature = "gpu")]
        {
            cuda::scan_resident_split_timed(
                &self.kinds,
                &self.host_kinds,
                &self.spans,
                pattern,
                host_per_mille,
            )
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }
}

/// Which backend a caller wants for a scan. `Auto` places each call by what
/// this process has measured at the input's size: the CPU engine alone, or,
/// for a pattern that reads only token kinds when a device is present, the
/// input's anchors split between the host's cores and the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Choose per input (the default).
    Auto,
    /// Force the device; fall back to the CPU when the pattern is
    /// ineligible or no device is present.
    Gpu,
    /// Force the CPU; never probe or touch the device.
    Cpu,
}

/// The backend a scan actually ran on, so a caller can report the choice
/// and detect a forced-device fallback (`Gpu` requested, `Cpu` used).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendUsed {
    /// The SIMT device kernel ran.
    Gpu,
    /// The CPU engine ran.
    Cpu,
    /// The host's cores and the device each took a share of the input's
    /// anchors, at once.
    Split,
}

/// Where a device scan's wall time goes, in microseconds.
///
/// The device speedup falls as the corpus grows, which is the opposite of
/// what a per-anchor kernel over more anchors should do. The call is not all
/// kernel: it lexes on the cores, builds two host vectors the size of the
/// token stream, copies one to the device and one back, and selects on the
/// host. Any of those could be what grows. Reasoning about which has been
/// wrong repeatedly here, so it is reported instead.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuPhases {
    /// Significant tokens scanned, so a caller can read per-token costs.
    pub tokens: usize,
    /// Lexing the input on the cores. Common to both engines.
    pub lex_us: f64,
    /// Building the kind vector from a token stream: zero for the kind-only
    /// scan, whose lexer writes the vector itself.
    pub host_prep_us: f64,
    /// Copying the kind vector and the automaton tables to the device.
    pub upload_us: f64,
    /// The kernel itself, from launch to the synchronize that follows the
    /// download - the two cannot be separated without a second sync, and a
    /// sync between them would add a stall the launch does not otherwise have.
    pub kernel_and_download_us: f64,
    /// Leftmost non-overlapping selection over the per-anchor ends, on the
    /// host.
    pub select_us: f64,
}

/// Where a resident split's time goes, in microseconds
/// ([`GpuTokens::scan_split_phases`]).
///
/// The corpus is already lexed and resident, so neither a lex nor an upload
/// appears here: what remains is the per-anchor ends, computed by the host and
/// the device at once, and the host's leftmost non-overlapping selection over
/// the joined table. `wall_us` also covers allocating the ends array and
/// choosing the split point, so it exceeds the two phases by that much.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SplitPhases {
    /// Both halves of the per-anchor ends, from the split point to the join:
    /// the device over the kinds it holds and the host over the kinds beside
    /// them, run at the same time, so this is the slower of the two.
    pub ends_us: f64,
    /// Leftmost non-overlapping selection over the joined ends, on one host
    /// thread.
    pub select_us: f64,
    /// The whole call, from the gates to the selected spans.
    pub wall_us: f64,
}

/// Where a pipelined device scan's time goes, in microseconds
/// ([`scan_gpu_pipelined`]). The stages run on two threads, so their busy
/// times can sum past the wall time; the excess is time they ran at once.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PipelinePhases {
    /// Ranges the input was lexed in.
    pub partitions: usize,
    /// Significant tokens scanned.
    pub tokens: usize,
    /// The whole call, from the blob table to the last selection.
    pub wall_us: f64,
    /// The blob table over the whole input, computed once before the first
    /// range is lexed.
    pub blobs_us: f64,
    /// Lexing the ranges across cores, summed over the ranges.
    pub lex_us: f64,
    /// Uploads, kernels and downloads, summed over the device thread's
    /// windows.
    pub device_us: f64,
    /// Selections, summed over the device thread's windows.
    pub select_us: f64,
    /// Copying each range into the device thread's window and dropping the
    /// tokens it has finished with, summed over the windows. This is the
    /// thread's own memory traffic beside the lexer: the whole corpus's kinds
    /// and spans are copied in once, and each window shifts what it carries.
    pub carry_us: f64,
}

impl PipelinePhases {
    /// A lower bound on the time the stages ran at once: their busy times'
    /// excess over the wall time.
    #[must_use]
    pub fn overlap_us(&self) -> f64 {
        (self.blobs_us + self.lex_us + self.device_us + self.select_us + self.carry_us
            - self.wall_us)
            .max(0.0)
    }
}

/// Run the device scan with its phases timed, for `benches/gpu_throughput`.
///
/// Returns `None` in the cases [`scan_gpu`] does and for a pattern that tests a
/// magnitude, whose phases this does not time, so a caller that gets `Some`
/// here would have got a device scan there. The matches are spans: the probe
/// times kind-only patterns, which bind nothing.
#[must_use]
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
pub fn scan_gpu_phases(
    pattern: &Pattern,
    input: &[u8],
) -> Option<(Vec<crate::engine::Span>, GpuPhases)> {
    #[cfg(feature = "gpu")]
    {
        cuda::scan_phases(pattern, input)
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

/// The kind-only device scan run as a pipeline over `partitions` ranges of
/// the input, for `benches/gpu_throughput`: the ranges are lexed in order
/// while the device scans, and the host selects, the ranges before them, and
/// each stage's busy time is reported beside the wall time. Returns the spans
/// [`scan_gpu`] returns; `None` without the `gpu` feature or a device, for a
/// pattern the device scan does not take or whose longest match is unbounded,
/// and when a device call fails.
#[must_use]
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
pub fn scan_gpu_pipelined(
    pattern: &Pattern,
    input: &[u8],
    partitions: usize,
) -> Option<(Vec<crate::engine::Span>, PipelinePhases)> {
    #[cfg(feature = "gpu")]
    {
        cuda::scan_pipelined(pattern, input, partitions)
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

/// The kind-only device scan over the lexed chunks where the lexer left them,
/// for `benches/gpu_throughput`: each chunk's kinds go to their offset of the
/// device buffer and the selection reads each chunk's spans in place, so the
/// host never joins the chunks. Returns the spans [`scan_gpu`] returns; `None`
/// without the `gpu` feature or a device, for a pattern the kind-only scan
/// does not take, and when a device call fails.
#[must_use]
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
pub fn scan_gpu_parts(pattern: &Pattern, input: &[u8]) -> Option<Vec<crate::engine::Span>> {
    #[cfg(feature = "gpu")]
    {
        cuda::scan_parts(pattern, input)
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

/// The kind-only scan with its anchors split between the host's cores and the
/// device, both working one lex at once, for `benches/gpu_throughput`: the
/// split the automatic route places, run whatever that route's placement model
/// has learned. Returns the spans [`scan_gpu`] returns; `None` without the
/// `gpu` feature or a device, and for a pattern the split does not take.
#[must_use]
#[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
pub fn scan_gpu_split(pattern: &Pattern, input: &[u8]) -> Option<Vec<crate::engine::Span>> {
    #[cfg(feature = "gpu")]
    {
        cuda::scan_split_now(pattern, input)
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

/// Whether a usable CUDA device is present. Cheap and cached: the device
/// context is initialized once, on the first call, and the result is
/// reused on every later call. Always `false` without the `gpu` feature,
/// so the auto router resolves to the CPU on a non-GPU build.
#[must_use]
pub fn device_available() -> bool {
    #[cfg(feature = "gpu")]
    {
        cuda::device_present()
    }
    #[cfg(not(feature = "gpu"))]
    {
        false
    }
}

/// Scan `input` for `pattern` on the backend chosen by `backend`, and
/// report which one ran. `Auto` places a pattern [`gpu_auto_routes`] accepts,
/// when a device is present, by what this process has measured at the
/// input's size: the CPU engine alone, or the input's anchors split between
/// the host's cores and the device; everything else runs on the CPU. `Gpu`
/// forces the device and falls back to the CPU when it cannot run the
/// pattern or no device is found; `Cpu` never touches the device. The
/// matches are identical on every backend, so the choice is only speed.
#[must_use]
pub fn scan_with_backend(
    pattern: &Pattern,
    input: &[u8],
    backend: Backend,
) -> (Vec<Span>, BackendUsed) {
    match backend {
        Backend::Cpu => (crate::engine::scan(pattern, input), BackendUsed::Cpu),
        Backend::Gpu => match scan_gpu(pattern, input) {
            Some(m) => {
                crate::trace::rung("scan", "the device kernel", input.len());
                (m, BackendUsed::Gpu)
            }
            None => (crate::engine::scan(pattern, input), BackendUsed::Cpu),
        },
        Backend::Auto => {
            #[cfg(feature = "gpu")]
            {
                if gpu_auto_routes(pattern)
                    && device_available()
                    && let Some(placed) = cuda::scan_placed(pattern, input)
                {
                    return placed;
                }
            }
            (crate::engine::scan(pattern, input), BackendUsed::Cpu)
        }
    }
}

/// How a scan runs: on a backend, as the dual-grain pipeline, or fed in
/// chunks of a size. The command line's `--gpu`, `--cpu`, `--dual-grain` and
/// `--chunk-size`, PowerShell's `-Backend`, `-DualGrain` and `-ChunkSize`, and
/// Python's `backend=`, `dual_grain=` and `chunk_size=` each build one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Engine {
    /// The backend a whole-input scan is placed on.
    pub backend: Backend,
    /// Run the byte grain and the token grain as a pipeline on two threads.
    pub dual_grain: bool,
    /// Feed the input in chunks of this many bytes, one or more.
    pub chunk_size: Option<usize>,
}

impl Engine {
    /// The routed scan every call runs where none of the choices is made.
    pub const PLAIN: Engine = Engine { backend: Backend::Auto, dual_grain: false, chunk_size: None };

    /// Whether this is the routed scan, which a call asking only whether an
    /// input matches, or for its first matches, may stop early on.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.backend == Backend::Auto && !self.dual_grain && self.chunk_size.is_none()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Engine::PLAIN
    }
}

/// What a scan through an [`Engine`] ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ran {
    /// A forced device scan, which the device ran.
    Device,
    /// A forced device scan the device could not take, run on the CPU: the
    /// pattern is outside the device's subset, no device is present, or the
    /// build has no device backend.
    DeviceDeclined,
    /// The dual-grain pipeline, and how its two grains spent their time.
    DualGrain(crate::dual_grain::GrainTiming),
    /// The input fed in chunks of the engine's size.
    Chunked,
    /// The routed backend, and what it placed the scan on.
    Routed(BackendUsed),
}

/// Scan `input` for `pattern` as `engine` says, and report what ran: a forced
/// device scan first, then the dual-grain pipeline, then a chunked scan, and
/// otherwise the routed backend. Every way returns the matches
/// [`crate::scan`] returns.
///
/// # Panics
///
/// A chunk size of zero, which names no chunk.
#[must_use]
pub fn scan_engine(pattern: &Pattern, input: &[u8], engine: &Engine) -> (Vec<Span>, Ran) {
    if engine.backend == Backend::Gpu {
        let (spans, used) = scan_with_backend(pattern, input, Backend::Gpu);
        (spans, if used == BackendUsed::Gpu { Ran::Device } else { Ran::DeviceDeclined })
    } else if engine.dual_grain {
        let (spans, timing) = crate::dual_grain::scan_dual_grain(pattern, input);
        (spans, Ran::DualGrain(timing))
    } else if let Some(size) = engine.chunk_size {
        assert!(size > 0, "a chunk size of zero names no chunk");
        (crate::streaming::scan_chunked(pattern, input.chunks(size)), Ran::Chunked)
    } else {
        let (spans, used) = scan_with_backend(pattern, input, engine.backend);
        (spans, Ran::Routed(used))
    }
}

/// The significant-token stream: kind codes and byte spans, whitespace dropped.
///
/// The device kernel runs over significant tokens only, and `select` maps a
/// token index back to a byte span, so both arrays are indexed by position in
/// this stream rather than in the lexer's.
///
/// Both arrays are reserved for the whole token count. That over-reserves by
/// the whitespace share and costs one allocation each; growing them instead
/// copies everything written so far at every doubling.
///
/// The spans keep the `u32` width [`Token`] already stores them at. `start()`
/// and `end()` widen to `usize` for callers, and widening here would double the
/// array this pass writes - eight bytes a token against sixteen.
#[must_use]
pub fn significant_stream(toks: &[Token]) -> (Vec<u32>, Vec<(u32, u32)>) {
    let mut kinds = Vec::with_capacity(toks.len());
    let mut spans = Vec::with_capacity(toks.len());
    for t in toks {
        if t.kind != TokenKind::Whitespace {
            kinds.push(t.kind.code());
            spans.push((t.start, t.end));
        }
    }
    (kinds, spans)
}

/// The magnitude x100 of every significant token of `toks`, in the order
/// [`significant_stream`] writes them, computed as the CPU engine computes it
/// so a device compare against a threshold agrees with the engine's.
#[must_use]
pub fn significant_magnitudes(toks: &[Token], input: &[u8]) -> Vec<f32> {
    toks.iter()
        .filter(|t| t.kind != TokenKind::Whitespace)
        .map(|t| crate::magnitude::token_magnitude(t.kind, &input[t.start()..t.end()]) * 100.0)
        .collect()
}

/// Orbit class ids under one group, assigned in first-seen order. Two byte
/// strings share an id exactly when a comparison under the group finds them
/// equal: their bytes under the identity orbit, their canonical forms under any
/// other.
pub(crate) struct ClassIds {
    group: crate::orbit::OrbitGroup,
    ids: std::collections::HashMap<Vec<u8>, u32>,
}

impl ClassIds {
    pub(crate) fn new(group: crate::orbit::OrbitGroup) -> Self {
        Self { group, ids: std::collections::HashMap::new() }
    }

    /// The group the ids compare under.
    #[cfg(feature = "gpu")]
    pub(crate) fn group(&self) -> crate::orbit::OrbitGroup {
        self.group
    }

    /// The id of `bytes`, assigning the next one when its class is new. Under
    /// the identity orbit a class already seen is found without allocating.
    pub(crate) fn id_of(&mut self, bytes: &[u8]) -> u32 {
        let next = u32::try_from(self.ids.len()).expect("fewer than 2^32 distinct classes");
        match self.group {
            crate::orbit::OrbitGroup::Identity => match self.ids.get(bytes) {
                Some(&id) => id,
                None => {
                    self.ids.insert(bytes.to_vec(), next);
                    next
                }
            },
            g => *self.ids.entry(crate::orbit::canonical(bytes, g).into_bytes()).or_insert(next),
        }
    }

    /// The id of `bytes` when its class has one, assigning nothing.
    #[cfg(feature = "gpu")]
    pub(crate) fn find(&self, bytes: &[u8]) -> Option<u32> {
        match self.group {
            crate::orbit::OrbitGroup::Identity => self.ids.get(bytes).copied(),
            g => self.ids.get(crate::orbit::canonical(bytes, g).as_bytes()).copied(),
        }
    }
}

/// An orbit class id for every significant token of `toks`, in the order
/// [`significant_stream`] writes them, and one for each of `literals`. Two
/// share an id exactly when a comparison under `group` finds them equal: their
/// bytes under the identity orbit, their canonical forms under any other. A
/// literal no token equals still gets an id, which then no token carries.
#[must_use]
pub fn significant_classes(
    toks: &[Token],
    input: &[u8],
    group: crate::orbit::OrbitGroup,
    literals: &[&[u8]],
) -> (Vec<u32>, Vec<u32>) {
    let mut ids = ClassIds::new(group);
    let literal_ids: Vec<u32> = literals.iter().map(|&lit| ids.id_of(lit)).collect();
    let token_ids: Vec<u32> = toks
        .iter()
        .filter(|t| t.kind != TokenKind::Whitespace)
        .map(|t| ids.id_of(&input[t.start()..t.end()]))
        .collect();
    (token_ids, literal_ids)
}

/// The pooled spectral reading of every significant token of `toks`, in the
/// order [`significant_stream`] writes them, from `field`, the spectral field
/// of the input they were lexed from: the pooled entropy x100 as the set
/// engine compares it, and one word packing the dominant period in its low
/// sixteen bits, the [`crate::nfa::texture_id`] of the pooled texture in the
/// next four, and whether a change-point is inside the token in bit 20.
#[must_use]
pub fn significant_spectral(toks: &[Token], field: &crate::spectral::SpectralField) -> (Vec<f32>, Vec<u32>) {
    let mut entropy = Vec::with_capacity(toks.len());
    let mut packed = Vec::with_capacity(toks.len());
    for t in toks.iter().filter(|t| t.kind != TokenKind::Whitespace) {
        let sig = field.signature(t.start(), t.end());
        entropy.push(sig.entropy * 100.0);
        let texture = crate::nfa::texture_id(crate::spectral::texture_of(&sig));
        let onset = u32::from(field.boundary_in(t.start(), t.end()));
        packed.push(u32::from(sig.period) | (texture << 16) | (onset << 20));
    }
    (entropy, packed)
}

/// The byte-class mask of every significant token of `toks`, in the order
/// [`significant_stream`] writes them: a [`crate::nfa::byte_class_bit`] bit for
/// each byte class every one of the token's bytes holds, tested as the CPU
/// engine tests it.
#[must_use]
pub fn significant_byte_masks(toks: &[Token], input: &[u8]) -> Vec<u32> {
    use crate::ast::ByteClass;
    const CLASSES: [ByteClass; 6] =
        [ByteClass::Digit, ByteClass::Word, ByteClass::Hex, ByteClass::Alpha, ByteClass::Upper, ByteClass::Lower];
    toks.iter()
        .filter(|t| t.kind != TokenKind::Whitespace)
        .map(|t| {
            let bytes = &input[t.start()..t.end()];
            CLASSES
                .iter()
                .filter(|&&bc| crate::engine::byte_class_matches(bc, bytes))
                .map(|&bc| crate::nfa::byte_class_bit(bc).expect("every class but whitespace has a bit"))
                .fold(0, |mask, bit| mask | bit)
        })
        .collect()
}

// The kernel launch is the one `unsafe` site, guarded by the argument
// contract documented at the call. `deny` (set crate-wide) is what lets
// this module opt in; a `forbid` could not be relaxed.
#[cfg(feature = "gpu")]
#[allow(unsafe_code)]
mod cuda {
    use std::sync::{Arc, OnceLock};

    use cudarc::driver::{CudaContext, CudaFunction, LaunchConfig, PushKernelArg};
    use cudarc::nvrtc::Ptx;

    use super::Span;
    use crate::ast::Pattern;
    use crate::ends_simd::{next_match_from, next_match_from_scalar, takes_vector};
    use crate::token::Token;

    /// PTX compiled from `kernels/scan.cu` at build time (see `build.rs`).
    /// Loaded through the driver at runtime, so deployment needs only the
    /// NVIDIA driver.
    const PTX: &str = include_str!(env!("TREX_SCAN_PTX"));

    /// The device context and the loaded kernel, initialized once.
    struct Gpu {
        ctx: Arc<CudaContext>,
        func: CudaFunction,
    }

    /// Whether a usable device is present, without exposing the handle
    /// type to the parent module. Cached through [`gpu`].
    pub(super) fn device_present() -> bool {
        gpu().is_some()
    }

    /// Load a kernel, reporting `None` for every way a host can lack a device.
    ///
    /// Two failures reach here by different routes. A host with the CUDA
    /// library but no usable device returns `Err`. A host without the library
    /// at all PANICS inside cudarc's dynamic loader, before any error value
    /// exists, so the probe is caught rather than returned. Both mean the
    /// same thing to a caller: run on the CPU.
    ///
    /// The failover is silent. The panic hook is replaced across the probe so
    /// the loader's message does not reach stderr, and the reason is named on
    /// the trace ladder, which prints only under `TREX_TRACE`.
    ///
    /// The hook is process-wide, so a panic on another thread during the
    /// probe is silenced with it. The window is one kernel load, once per
    /// process, behind a `OnceLock`.
    ///
    /// `kernel` names which kernel was loaded, since a host can hold a device
    /// that takes one and not the other. Visible to the rest of `gpu` for the
    /// device coder's kernel.
    pub(super) fn load_or_cpu<T>(
        kernel: &str,
        load: fn() -> Result<T, cudarc::driver::DriverError>,
    ) -> Option<T> {
        let prior = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let probed = std::panic::catch_unwind(load);
        std::panic::set_hook(prior);
        match probed {
            Ok(Ok(loaded)) => Some(loaded),
            Ok(Err(driver)) => {
                crate::trace::rung("gpu", &format!("{kernel}: no usable device ({driver:?})"), 0);
                None
            }
            Err(panicked) => {
                let why = panicked
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panicked.downcast_ref::<&str>().copied())
                    .unwrap_or("the CUDA library could not be loaded");
                crate::trace::rung("gpu", &format!("{kernel}: no CUDA library ({why})"), 0);
                None
            }
        }
    }

    /// The process-wide GPU handle, or `None` when no device is usable, in
    /// which case every scan runs on the CPU.
    fn gpu() -> Option<&'static Gpu> {
        static G: OnceLock<Option<Gpu>> = OnceLock::new();
        G.get_or_init(|| load_or_cpu("scan", load_scan_kernel)).as_ref()
    }

    /// Device 0's context with the scan kernel loaded into it.
    fn load_scan_kernel() -> Result<Gpu, cudarc::driver::DriverError> {
        let ctx = CudaContext::new(0)?;
        let module = ctx.load_module(Ptx::from_src(PTX))?;
        let func = module.load_function("trex_scan")?;
        Ok(Gpu { ctx, func })
    }

    /// [`scan`] with a clock between its phases, for the phase report.
    ///
    /// A second copy of the same sequence rather than a timed wrapper: the
    /// phases are local bindings and there is no seam to hook. The bench
    /// compares this function's match set against [`scan`]'s, so the two
    /// drifting apart is caught rather than assumed away.
    ///
    /// Every device failure here is reported to stderr before it becomes
    /// `None`. `scan` maps them all to `None`, where the caller reads "no
    /// device" and falls back to the cores - so an out-of-memory or a kernel
    /// fault is indistinguishable from a machine with no GPU, and the
    /// fallback hides it. That is tolerable for a scan whose contract is
    /// that the device is optional; it is not tolerable for a function whose
    /// whole purpose is to say where the time went.
    pub(super) fn scan_phases(
        pattern: &Pattern,
        input: &[u8],
    ) -> Option<(Vec<crate::engine::Span>, super::GpuPhases)> {
        use std::time::Instant;

        let nfa = crate::nfa::compile_for_gpu(pattern)?;
        let g = gpu()?;
        if nfa.reads_magnitude
            || nfa.binds_registers
            || nfa.class_group.is_some()
            || nfa.reads_bytes
            || nfa.reads_spectral
        {
            eprintln!(
                "trex gpu phase probe: phases are timed for kind-only patterns, and this one reads a per-token property"
            );
            return None;
        }
        let mut ph = super::GpuPhases::default();

        // The closure runs as soon as the lex returns, so the clock read at
        // its entry is the lex.
        let t = Instant::now();
        crate::parallel_lex::lex_significant_parallel_held(input, |kinds, spans| {
            ph.lex_us = t.elapsed().as_secs_f64() * 1e6;
            let n = kinds.len();
            if n == 0 {
                return Some((Vec::new(), ph));
            }
            ph.tokens = n;
            phases_lexed(g, &nfa, kinds, spans, ph)
        })
    }

    /// [`scan_phases`] from the lexed stream on: upload, kernel, selection.
    fn phases_lexed(
        g: &Gpu,
        nfa: &crate::nfa::GpuNfa,
        kinds: &[u32],
        spans: &[(u32, u32)],
        mut ph: super::GpuPhases,
    ) -> Option<(Vec<Span>, super::GpuPhases)> {
        use std::time::Instant;

        let n = kinds.len();
        let stream = g.ctx.default_stream();
        let t = Instant::now();
        let staged = (|| {
            let d_kind = stream.clone_htod(kinds)?;
            let d_atom = stream.clone_htod(&nfa.atom_kind)?;
            let d_next = stream.clone_htod(&nfa.next_closure)?;
            let d_isatom = stream.clone_htod(&nfa.is_atom)?;
            let d_result = stream.alloc_zeros::<i32>(n)?;
            // The copies are queued on the stream, so the clock sees them
            // land only once something waits. Synchronizing here is what
            // makes this the upload rather than the cost of queueing it.
            stream.synchronize()?;
            Ok::<_, cudarc::driver::DriverError>((d_kind, d_atom, d_next, d_isatom, d_result))
        })();
        let (d_kind, d_atom, d_next, d_isatom, d_result) = match staged {
            Ok(v) => v,
            Err(e) => {
                // Debug rather than Display: cudarc's DriverError implements
                // only the former, and the CUDA status code it carries is
                // what names the failure.
                eprintln!("trex gpu phase probe: staging {n} tokens to the device failed: {e:?}");
                return None;
            }
        };
        ph.upload_us = t.elapsed().as_secs_f64() * 1e6;

        let n_i = n as i32;
        let nstates_i = nfa.nstates as i32;
        let start_closure = nfa.start_closure;
        let match_mask = nfa.match_mask;
        let block = 256u32;
        let cfg = LaunchConfig {
            grid_dim: (n.div_ceil(block as usize) as u32, 1, 1),
            block_dim: (block, 1, 1),
            shared_mem_bytes: 0,
        };

        let t = Instant::now();
        let ran = (|| {
            let mut builder = stream.launch_builder(&g.func);
            builder.arg(&d_kind);
            builder.arg(&n_i);
            builder.arg(&d_atom);
            builder.arg(&d_next);
            builder.arg(&d_isatom);
            builder.arg(&start_closure);
            builder.arg(&match_mask);
            builder.arg(&nstates_i);
            builder.arg(&d_result);
            // SAFETY: as in `scan` - the argument list matches `trex_scan`'s
            // parameters in order and type, and every device buffer is sized
            // to `n` or to `nstates`, which the kernel never indexes past.
            unsafe { builder.launch(cfg)? };
            let result: Vec<i32> = stream.clone_dtoh(&d_result)?;
            stream.synchronize()?;
            Ok::<_, cudarc::driver::DriverError>(result)
        })();
        let result = match ran {
            Ok(r) => r,
            Err(e) => {
                eprintln!("trex gpu phase probe: the kernel over {n} tokens failed: {e:?}");
                return None;
            }
        };
        ph.kernel_and_download_us = t.elapsed().as_secs_f64() * 1e6;

        let t = Instant::now();
        let out = select(spans, &result);
        ph.select_us = t.elapsed().as_secs_f64() * 1e6;
        Some((out, ph))
    }

    /// The kind-only scan as a pipeline over `partitions` ranges of the input
    /// between its safe boundaries. The calling thread lexes the ranges in
    /// order, each across cores, against the whole input's blob table, and
    /// hands each to a device thread as soon as it is lexed. The device thread
    /// appends the range to the tokens it carried from the window before,
    /// uploads the window, runs the kernel, reads the ends back and selects
    /// every anchor whose match cannot reach past the window: a match is at
    /// most `bounded_max_len` tokens long, so the window's last
    /// `bounded_max_len - 1` tokens carry into the next window and their
    /// anchors are selected there, and after the last range the carried tokens
    /// are scanned as a final window. `None` for a pattern the kernel does not
    /// take, a pattern whose longest match is unbounded, and a device failure,
    /// which is reported to stderr first.
    pub(super) fn scan_pipelined(
        pattern: &Pattern,
        input: &[u8],
        partitions: usize,
    ) -> Option<(Vec<Span>, super::PipelinePhases)> {
        use std::sync::mpsc::{TryRecvError, channel, sync_channel};
        use std::time::Instant;

        use crate::parallel_lex::SignificantWorkspace;

        let nfa = crate::nfa::compile_for_gpu(pattern)?;
        if nfa.reads_magnitude
            || nfa.binds_registers
            || nfa.class_group.is_some()
            || nfa.reads_bytes
            || nfa.reads_spectral
        {
            return None;
        }
        let max_len = crate::nfa::bounded_max_len(pattern)?;
        let g = gpu()?;
        let carry = max_len.saturating_sub(1);

        let wall = Instant::now();
        let mut ph = super::PipelinePhases::default();
        let t = Instant::now();
        let blobs = crate::lexer::blob_runs_parallel(input);
        ph.blobs_us = t.elapsed().as_secs_f64() * 1e6;
        let bounds = crate::parallel_lex::safe_boundaries(input, partitions.max(1));
        ph.partitions = bounds.len().saturating_sub(1);

        // One lexed range waits for the device thread at most, so the lexer
        // runs one range ahead of the device and no further.
        let (to_device, from_lexer) = sync_channel::<SignificantWorkspace>(1);
        let (to_lexer, from_device) = channel::<SignificantWorkspace>();
        let nfa_ref = &nfa;
        let (lex_us, staged) = std::thread::scope(|sc| {
            let device = sc.spawn(move || device_stage(g, nfa_ref, carry, from_lexer, to_lexer));
            let mut lex_us = 0.0;
            // Workspaces for the ranges that run before the device thread has
            // returned its first. Three covers the pipeline's depth - one
            // being lexed, one in the channel, one in the device's window -
            // so the whole scan allocates three rather than one a range.
            let mut spare: Vec<SignificantWorkspace> =
                (0..3).map(|_| SignificantWorkspace::default()).collect();
            for w in bounds.windows(2) {
                // A range's buffers come back once the device thread has
                // copied them into its window; until the first does, a range
                // takes one of the pool above. A workspace taken fresh for
                // every range costs the lex 2.5x to 2.8x at 16.5 MB over eight
                // ranges, where one kept across them costs nothing. With the
                // pool spent the lexer waits for a return rather than
                // allocating, which the channel one deep already bounds it to.
                // Either channel ends only when the device thread has stopped
                // on a failure, which its join below reports, so no further
                // range is lexed.
                let mut ws = match from_device.try_recv() {
                    Ok(ws) => ws,
                    Err(TryRecvError::Empty) => match spare.pop() {
                        Some(ws) => ws,
                        None => match from_device.recv() {
                            Ok(ws) => ws,
                            Err(std::sync::mpsc::RecvError) => break,
                        },
                    },
                    Err(TryRecvError::Disconnected) => break,
                };
                // Room at the front for the tokens the window before carries
                // in, which the device thread writes: it then reads a few
                // dozen slots of this buffer rather than all of it. Reading
                // every range instead cost the lex 1.72x to 2.16x, which
                // `examples/lex_contention`'s handed arm measures.
                let t = Instant::now();
                ws.kinds.clear();
                ws.spans.clear();
                ws.kinds.resize(carry, 0);
                ws.spans.resize(carry, (0, 0));
                crate::parallel_lex::lex_significant_range_onto(input, w[0], w[1], &blobs, &mut ws);
                lex_us += t.elapsed().as_secs_f64() * 1e6;
                if to_device.send(ws).is_err() {
                    // The device thread has stopped on a failure, which its
                    // join below reports, so no further range is lexed.
                    break;
                }
            }
            drop(to_device);
            (lex_us, device.join().expect("the pipeline's device thread returns"))
        });
        let staged = match staged {
            Ok(s) => s,
            Err(e) => {
                eprintln!("trex gpu pipeline: a window on the device failed: {e:?}");
                return None;
            }
        };
        ph.lex_us = lex_us;
        ph.tokens = staged.tokens;
        ph.device_us = staged.device_us;
        ph.select_us = staged.select_us;
        ph.carry_us = staged.carry_us;
        ph.wall_us = wall.elapsed().as_secs_f64() * 1e6;
        Some((staged.spans, ph))
    }

    /// What the pipeline's device thread hands back: the selected spans, the
    /// tokens it received, and its busy times in microseconds.
    struct Staged {
        spans: Vec<Span>,
        tokens: usize,
        device_us: f64,
        select_us: f64,
        carry_us: f64,
    }

    /// The device thread of [`scan_pipelined`]. Each lexed range joins the
    /// tokens carried from the window before; the window is uploaded, scanned
    /// and read back, its anchors are selected up to the first whose match
    /// could reach past the window, and the tokens from there on carry into
    /// the next window. A range's buffers go back to the lexer once copied.
    fn device_stage(
        g: &Gpu,
        nfa: &crate::nfa::GpuNfa,
        carry: usize,
        ranges: std::sync::mpsc::Receiver<crate::parallel_lex::SignificantWorkspace>,
        spent: std::sync::mpsc::Sender<crate::parallel_lex::SignificantWorkspace>,
    ) -> Result<Staged, cudarc::driver::DriverError> {
        use std::time::Instant;

        let stream = g.ctx.default_stream();
        let d_atom = stream.clone_htod(&nfa.atom_kind)?;
        let d_next = stream.clone_htod(&nfa.next_closure)?;
        let d_isatom = stream.clone_htod(&nfa.is_atom)?;
        let mut staged =
            Staged { spans: Vec::new(), tokens: 0, device_us: 0.0, select_us: 0.0, carry_us: 0.0 };
        // The tokens carried out of the window just scanned, which the next
        // window begins with. At most `carry` of them, one short of the
        // longest match, so this is a few dozen and never a range.
        let mut kinds: Vec<u32> = Vec::new();
        let mut spans: Vec<(u32, u32)> = Vec::new();
        // The selection's next anchor, as an index into the current window.
        let mut next = 0usize;
        let mut last = false;
        while !last {
            // A range arrives with `carry` slots reserved at its front for
            // what the window before carried in. Those are the only slots this
            // thread writes, and the window begins where they end when there
            // are fewer than `carry` of them - which shifts nothing and leaves
            // the first window, carrying none, beginning at `carry` exactly.
            let (mut window, start) = match ranges.recv() {
                Ok(ws) => {
                    let t = Instant::now();
                    let start = carry - kinds.len();
                    staged.tokens += ws.kinds.len() - carry;
                    let mut ws = ws;
                    ws.kinds[start..carry].copy_from_slice(&kinds);
                    ws.spans[start..carry].copy_from_slice(&spans);
                    staged.carry_us += t.elapsed().as_secs_f64() * 1e6;
                    (Some(ws), start)
                }
                // Every range has been sent: what the last window carried out
                // is the final window, and each of its anchors is final.
                Err(std::sync::mpsc::RecvError) => {
                    last = true;
                    (None, 0)
                }
            };
            let (wk, wsp): (&[u32], &[(u32, u32)]) = match &window {
                Some(ws) => (&ws.kinds[start..], &ws.spans[start..]),
                None => (&kinds, &spans),
            };
            let n = wk.len();
            if n == 0 {
                // A range that lexed to nothing, or no carry left at the end:
                // there is no window to scan, and the buffers go back so the
                // ranges after it still are.
                if let Some(ws) = window.take()
                    && let Err(returned) = spent.send(ws)
                {
                    drop(returned);
                }
                continue;
            }
            let t = Instant::now();
            let ends = window_ends(g, &stream, nfa, wk, &d_atom, &d_next, &d_isatom)?;
            staged.device_us += t.elapsed().as_secs_f64() * 1e6;

            let t = Instant::now();
            let horizon = if last { n } else { n.saturating_sub(carry) };
            while next < horizon {
                let best = ends[next];
                if best > next as i32 {
                    let e = best as usize;
                    staged.spans.push(Span { start: wsp[next].0, end: wsp[e - 1].1 });
                    next = e;
                } else {
                    next += 1;
                }
            }
            staged.select_us += t.elapsed().as_secs_f64() * 1e6;
            next -= horizon;

            let t = Instant::now();
            let (tail_kinds, tail_spans) = (wk[horizon..].to_vec(), wsp[horizon..].to_vec());
            kinds = tail_kinds;
            spans = tail_spans;
            staged.carry_us += t.elapsed().as_secs_f64() * 1e6;
            // The lexer takes the buffers back for its next range; once it has
            // lexed its last range it holds no receiver, and they are dropped
            // here instead.
            if let Some(ws) = window.take()
                && let Err(returned) = spent.send(ws)
            {
                drop(returned);
            }
        }
        Ok(staged)
    }

    /// The kernel's longest match end for every anchor of `kinds`, a window of
    /// the significant kind stream, with the automaton tables already on the
    /// device.
    #[allow(clippy::too_many_arguments)]
    fn window_ends(
        g: &Gpu,
        stream: &Arc<cudarc::driver::CudaStream>,
        nfa: &crate::nfa::GpuNfa,
        kinds: &[u32],
        d_atom: &cudarc::driver::CudaSlice<u32>,
        d_next: &cudarc::driver::CudaSlice<u64>,
        d_isatom: &cudarc::driver::CudaSlice<u32>,
    ) -> Result<Vec<i32>, cudarc::driver::DriverError> {
        let d_kind = stream.clone_htod(kinds)?;
        device_ends(g, stream, nfa, &d_kind, kinds.len(), d_atom, d_next, d_isatom)
    }

    /// [`device_ends`] over a view of kinds already on the device, read back
    /// straight into `out`: a caller holding a resident corpus scans a range
    /// of it without copying the kinds there or the ends back.
    #[allow(clippy::too_many_arguments)]
    fn device_view_ends(
        g: &Gpu,
        stream: &Arc<cudarc::driver::CudaStream>,
        nfa: &crate::nfa::GpuNfa,
        d_kind: &cudarc::driver::CudaView<'_, u32>,
        out: &mut [i32],
        d_atom: &cudarc::driver::CudaSlice<u32>,
        d_next: &cudarc::driver::CudaSlice<u64>,
        d_isatom: &cudarc::driver::CudaSlice<u32>,
    ) -> Result<(), cudarc::driver::DriverError> {
        let n = out.len();
        let d_result = stream.alloc_zeros::<i32>(n)?;
        let n_i = n as i32;
        let nstates_i = nfa.nstates as i32;
        let start_closure = nfa.start_closure;
        let match_mask = nfa.match_mask;
        let block = 256u32;
        let cfg = LaunchConfig {
            grid_dim: (n.div_ceil(block as usize) as u32, 1, 1),
            block_dim: (block, 1, 1),
            shared_mem_bytes: 0,
        };
        let mut builder = stream.launch_builder(&g.func);
        builder.arg(d_kind);
        builder.arg(&n_i);
        builder.arg(d_atom);
        builder.arg(d_next);
        builder.arg(d_isatom);
        builder.arg(&start_closure);
        builder.arg(&match_mask);
        builder.arg(&nstates_i);
        builder.arg(&d_result);
        // SAFETY: the argument list matches `trex_scan`'s parameters in order
        // and type, the view holds `n` kinds, and every other device buffer is
        // sized to `n` or to `nstates`, which the kernel never indexes past.
        unsafe { builder.launch(cfg)? };
        stream.memcpy_dtoh(&d_result, out)?;
        stream.synchronize()?;
        Ok(())
    }

    /// The kernel's longest match end for each of the `n` anchors of `d_kind`,
    /// significant kinds already on the device, with the automaton tables
    /// there too.
    #[allow(clippy::too_many_arguments)]
    fn device_ends(
        g: &Gpu,
        stream: &Arc<cudarc::driver::CudaStream>,
        nfa: &crate::nfa::GpuNfa,
        d_kind: &cudarc::driver::CudaSlice<u32>,
        n: usize,
        d_atom: &cudarc::driver::CudaSlice<u32>,
        d_next: &cudarc::driver::CudaSlice<u64>,
        d_isatom: &cudarc::driver::CudaSlice<u32>,
    ) -> Result<Vec<i32>, cudarc::driver::DriverError> {
        let d_result = stream.alloc_zeros::<i32>(n)?;
        let n_i = n as i32;
        let nstates_i = nfa.nstates as i32;
        let start_closure = nfa.start_closure;
        let match_mask = nfa.match_mask;
        let block = 256u32;
        let cfg = LaunchConfig {
            grid_dim: (n.div_ceil(block as usize) as u32, 1, 1),
            block_dim: (block, 1, 1),
            shared_mem_bytes: 0,
        };
        let mut builder = stream.launch_builder(&g.func);
        builder.arg(d_kind);
        builder.arg(&n_i);
        builder.arg(d_atom);
        builder.arg(d_next);
        builder.arg(d_isatom);
        builder.arg(&start_closure);
        builder.arg(&match_mask);
        builder.arg(&nstates_i);
        builder.arg(&d_result);
        // SAFETY: the argument list matches `trex_scan`'s parameters in order
        // and type, and every device buffer is sized to `n` or to `nstates`,
        // which the kernel never indexes past.
        unsafe { builder.launch(cfg)? };
        let ends: Vec<i32> = stream.clone_dtoh(&d_result)?;
        stream.synchronize()?;
        Ok(ends)
    }

    pub(super) fn scan(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
        let nfa = crate::nfa::compile_for_gpu(pattern)?;
        let g = gpu()?;
        if nfa.reads_magnitude
            || nfa.binds_registers
            || nfa.class_group.is_some()
            || nfa.reads_bytes
            || nfa.reads_spectral
        {
            return scan_props(g, &nfa, input);
        }

        // The lexer writes the significant kinds and spans across cores into
        // this thread's held buffers; a kind-only scan builds no token vector.
        crate::parallel_lex::lex_significant_parallel_held(input, |kinds, spans| match scan_lexed(g, &nfa, kinds, spans) {
            Ok(found) => Some(found),
            Err(e) => {
                eprintln!("trex gpu scan: a device call failed: {e:?}");
                None
            }
        })
    }

    /// [`scan`] from the lexed stream on: upload, kernel, selection.
    fn scan_lexed(
        g: &Gpu,
        nfa: &crate::nfa::GpuNfa,
        kinds: &[u32],
        spans: &[(u32, u32)],
    ) -> Result<Vec<Span>, cudarc::driver::DriverError> {
        let n = kinds.len();
        if n == 0 {
            return Ok(Vec::new());
        }

        let stream = g.ctx.default_stream();
        let d_kind = stream.clone_htod(kinds)?;
        let d_atom = stream.clone_htod(&nfa.atom_kind)?;
        let d_next = stream.clone_htod(&nfa.next_closure)?;
        let d_isatom = stream.clone_htod(&nfa.is_atom)?;
        let result = device_ends(g, &stream, nfa, &d_kind, n, &d_atom, &d_next, &d_isatom)?;
        Ok(select(spans, &result))
    }

    /// The split itself, for a caller that wants it rather than what the
    /// placement model chose: [`scan`]'s gate, then [`scan_split`].
    pub(super) fn scan_split_now(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
        let nfa = crate::nfa::compile_for_gpu(pattern)?;
        if nfa.reads_magnitude
            || nfa.binds_registers
            || nfa.class_group.is_some()
            || nfa.reads_bytes
            || nfa.reads_spectral
        {
            return None;
        }
        let g = gpu()?;
        Some(scan_split(g, &nfa, input))
    }

    /// [`scan`] over the lexed chunks where the lexer left them: each chunk's
    /// kinds are copied to their offset of one device buffer, and the
    /// selection reads each chunk's spans in place, so the kinds and spans are
    /// never joined on the host. `None` for a pattern [`scan`] hands to
    /// `scan_props`, and for a device failure, which is reported to stderr
    /// first.
    pub(super) fn scan_parts(pattern: &Pattern, input: &[u8]) -> Option<Vec<Span>> {
        let nfa = crate::nfa::compile_for_gpu(pattern)?;
        if nfa.reads_magnitude
            || nfa.binds_registers
            || nfa.class_group.is_some()
            || nfa.reads_bytes
            || nfa.reads_spectral
        {
            return None;
        }
        let g = gpu()?;
        crate::parallel_lex::lex_significant_parts_held(input, |parts| match scan_lexed_parts(g, &nfa, parts) {
            Ok(spans) => Some(spans),
            Err(e) => {
                eprintln!("trex gpu parts scan: a device call failed: {e:?}");
                None
            }
        })
    }

    /// [`scan_parts`] from the lexed chunks on: each chunk's kinds uploaded to
    /// its offset, the kernel, and the selection across the chunks' spans.
    fn scan_lexed_parts(
        g: &Gpu,
        nfa: &crate::nfa::GpuNfa,
        parts: &[crate::lexer::Significant],
    ) -> Result<Vec<Span>, cudarc::driver::DriverError> {
        let n: usize = parts.iter().map(|p| p.kinds.len()).sum();
        if n == 0 {
            return Ok(Vec::new());
        }
        let stream = g.ctx.default_stream();
        // SAFETY: the copies below write every element before the kernel
        // reads one, since the chunks' lengths sum to `n` and each chunk
        // writes its own offset range.
        let mut d_kind = unsafe { stream.alloc::<u32>(n) }?;
        let mut off = 0usize;
        for part in parts {
            let len = part.kinds.len();
            if len > 0 {
                stream.memcpy_htod(part.kinds.as_slice(), &mut d_kind.slice_mut(off..off + len))?;
            }
            off += len;
        }
        let d_atom = stream.clone_htod(&nfa.atom_kind)?;
        let d_next = stream.clone_htod(&nfa.next_closure)?;
        let d_isatom = stream.clone_htod(&nfa.is_atom)?;
        let ends = device_ends(g, &stream, nfa, &d_kind, n, &d_atom, &d_next, &d_isatom)?;
        Ok(select_parts(parts, &ends))
    }

    /// [`select`] over the lexed chunks' spans in place. `ends` holds the
    /// longest match end of every anchor of the chunks joined, and a match
    /// may end in a later chunk than the one it starts in.
    fn select_parts(parts: &[crate::lexer::Significant], ends: &[i32]) -> Vec<Span> {
        // Chosen once for the whole selection, chunks included, so the loop
        // below never tests which scan it is running.
        if takes_vector(ends) {
            select_parts_with(parts, ends, next_match_from)
        } else {
            select_parts_with(parts, ends, next_match_from_scalar)
        }
    }

    /// [`select_parts`] over one scan.
    fn select_parts_with(
        parts: &[crate::lexer::Significant],
        ends: &[i32],
        scan: impl Fn(&[i32], usize, usize) -> Option<usize>,
    ) -> Vec<Span> {
        let mut out = Vec::with_capacity(ends.len() / 2 + 1);
        // `a` is the next anchor, and `base` the joined index of the first
        // token of the chunk being read.
        let mut a = 0usize;
        let mut base = 0usize;
        for (p, part) in parts.iter().enumerate() {
            let end = base + part.spans.len();
            while let Some(m) = scan(ends, a, end) {
                let e = ends[m] as usize; // tokens [m, e) matched
                let last = if e <= end {
                    part.spans[e - 1 - base].1
                } else {
                    span_end_in(&parts[p + 1..], end, e - 1)
                };
                out.push(Span { start: part.spans[m - base].0, end: last });
                a = e;
            }
            // A chunk with no match left `a` where it was, and the next chunk
            // indexes its spans from `base`, so the walk moves to the chunk
            // boundary here rather than through a miss at a time.
            a = a.max(end);
            base = end;
        }
        out
    }

    /// The end of the token at joined index `i`, which is in `rest`, the
    /// chunks whose first token has joined index `base`.
    fn span_end_in(rest: &[crate::lexer::Significant], mut base: usize, i: usize) -> u32 {
        for part in rest {
            let len = part.spans.len();
            if i < base + len {
                return part.spans[i - base].1;
            }
            base += len;
        }
        panic!("token {i} is past the lexed chunks, whose tokens end at {base}");
    }

    /// Significant token kinds on the device, uploaded once, with the other
    /// per-token properties they were held with: magnitudes, and byte-class
    /// masks with orbit class ids. An empty text holds no buffer.
    pub(super) struct ResidentKinds {
        d_kind: Option<cudarc::driver::CudaSlice<u32>>,
        d_mag: Option<cudarc::driver::CudaSlice<f32>>,
        d_bytes: Option<cudarc::driver::CudaSlice<u32>>,
        d_class: Option<cudarc::driver::CudaSlice<u32>>,
        d_ent: Option<cudarc::driver::CudaSlice<f32>>,
        d_spec: Option<cudarc::driver::CudaSlice<u32>>,
        magnitudes: bool,
        spectral: bool,
    }

    /// Upload `kinds`, and `mags` when given, to the device to be held. A
    /// failed upload is reported before it becomes `None`.
    pub(super) fn upload_kinds(kinds: &[u32], mags: Option<&[f32]>) -> Option<ResidentKinds> {
        let g = gpu()?;
        if let Some(m) = mags {
            assert_eq!(m.len(), kinds.len(), "one magnitude per significant token");
        }
        let magnitudes = mags.is_some();
        if kinds.is_empty() {
            return Some(ResidentKinds {
                d_kind: None,
                d_mag: None,
                d_bytes: None,
                d_class: None,
                d_ent: None,
                d_spec: None,
                magnitudes,
                spectral: false,
            });
        }
        let stream = g.ctx.default_stream();
        let held = (|| {
            let d_kind = stream.clone_htod(kinds)?;
            let d_mag = match mags {
                Some(m) => Some(stream.clone_htod(m)?),
                None => None,
            };
            Ok::<_, cudarc::driver::DriverError>((d_kind, d_mag))
        })();
        match held {
            Ok((d_kind, d_mag)) => Some(ResidentKinds {
                d_kind: Some(d_kind),
                d_mag,
                d_bytes: None,
                d_class: None,
                d_ent: None,
                d_spec: None,
                magnitudes,
                spectral: false,
            }),
            Err(e) => {
                eprintln!("trex gpu: holding {} tokens on the device failed: {e:?}", kinds.len());
                None
            }
        }
    }

    /// Upload `kinds` with every per-token property - magnitudes, byte-class
    /// masks, orbit class ids, and the spectral reading when `spectral` is
    /// given, one each per kind - to be held. A failed upload is reported
    /// before it becomes `None`.
    pub(super) fn upload_properties(
        kinds: &[u32],
        mags: &[f32],
        masks: &[u32],
        ids: &[u32],
        spectral: Option<(&[f32], &[u32])>,
    ) -> Option<ResidentKinds> {
        let g = gpu()?;
        assert!(
            mags.len() == kinds.len() && masks.len() == kinds.len() && ids.len() == kinds.len(),
            "one magnitude, mask and class id per significant token"
        );
        if let Some((e, s)) = spectral {
            assert!(e.len() == kinds.len() && s.len() == kinds.len(), "one spectral reading per significant token");
        }
        if kinds.is_empty() {
            return Some(ResidentKinds {
                d_kind: None,
                d_mag: None,
                d_bytes: None,
                d_class: None,
                d_ent: None,
                d_spec: None,
                magnitudes: true,
                spectral: spectral.is_some(),
            });
        }
        let stream = g.ctx.default_stream();
        let held = (|| {
            let d_kind = stream.clone_htod(kinds)?;
            let d_mag = stream.clone_htod(mags)?;
            let d_bytes = stream.clone_htod(masks)?;
            let d_class = stream.clone_htod(ids)?;
            let (d_ent, d_spec) = match spectral {
                Some((e, s)) => (Some(stream.clone_htod(e)?), Some(stream.clone_htod(s)?)),
                None => (None, None),
            };
            Ok::<_, cudarc::driver::DriverError>((d_kind, d_mag, d_bytes, d_class, d_ent, d_spec))
        })();
        match held {
            Ok((d_kind, d_mag, d_bytes, d_class, d_ent, d_spec)) => Some(ResidentKinds {
                d_kind: Some(d_kind),
                d_mag: Some(d_mag),
                d_bytes: Some(d_bytes),
                d_class: Some(d_class),
                d_ent,
                d_spec,
                magnitudes: true,
                spectral: spectral.is_some(),
            }),
            Err(e) => {
                eprintln!("trex gpu: holding {} tokens and their properties on the device failed: {e:?}", kinds.len());
                None
            }
        }
    }

    /// `trex_scan_props` from the scan module, loaded once. A failed load is
    /// reported before it becomes `None`.
    fn scan_props_kernel() -> Option<&'static CudaFunction> {
        static F: OnceLock<Option<CudaFunction>> = OnceLock::new();
        F.get_or_init(|| {
            let g = gpu()?;
            let module = match g.ctx.load_module(Ptx::from_src(PTX)) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("trex gpu: loading the scan module for trex_scan_props failed: {e:?}");
                    return None;
                }
            };
            match module.load_function("trex_scan_props") {
                Ok(f) => Some(f),
                Err(e) => {
                    eprintln!("trex gpu: loading trex_scan_props failed: {e:?}");
                    None
                }
            }
        })
        .as_ref()
    }

    /// Launch the scan kernel over `n` device-held kinds and download each
    /// anchor's longest match end. `d_mag` carries the per-token magnitudes and
    /// is given exactly when the pattern tests one; `d_class` carries the
    /// per-token orbit classes and is given exactly when the pattern compares
    /// a literal or a register; `d_bytes` carries the per-token byte-class
    /// masks and is given exactly when the pattern tests a byte class.
    /// `lit_class` holds each state's literal class id, `u32::MAX` where the
    /// state is not a literal, and `None` stands for every state being so. A
    /// device failure is reported before it becomes `None`.
    #[allow(clippy::too_many_arguments)]
    fn run_scan(
        g: &Gpu,
        d_kind: &cudarc::driver::CudaSlice<u32>,
        d_mag: Option<&cudarc::driver::CudaSlice<f32>>,
        d_class: Option<&cudarc::driver::CudaSlice<u32>>,
        d_bytes: Option<&cudarc::driver::CudaSlice<u32>>,
        d_spectral: Option<(&cudarc::driver::CudaSlice<f32>, &cudarc::driver::CudaSlice<u32>)>,
        lit_class: Option<&[u32]>,
        nfa: &crate::nfa::GpuNfa,
        n: usize,
    ) -> Option<Vec<i32>> {
        assert_eq!(
            nfa.reads_spectral,
            d_spectral.is_some(),
            "a pattern that tests the spectral field scans with per-token readings, and only such a pattern does"
        );
        assert_eq!(
            nfa.reads_magnitude,
            d_mag.is_some(),
            "a pattern that tests a magnitude scans with per-token magnitudes, and only such a pattern does"
        );
        assert_eq!(
            nfa.class_group.is_some(),
            d_class.is_some(),
            "a pattern that compares a literal or a register scans with per-token classes, and only such a pattern does"
        );
        assert_eq!(
            nfa.reads_bytes,
            d_bytes.is_some(),
            "a pattern that tests a byte class scans with per-token masks, and only such a pattern does"
        );
        let literal_classes: Vec<u32> = match lit_class {
            Some(l) => l.to_vec(),
            None => vec![u32::MAX; nfa.nstates],
        };
        assert_eq!(literal_classes.len(), nfa.nstates, "one literal class per state");
        assert!(
            nfa.literals.iter().all(Option::is_none) || lit_class.is_some(),
            "a pattern with a literal scans with its literal classes resolved"
        );
        let n_i = n as i32;
        let nstates_i = nfa.nstates as i32;
        let start_closure = nfa.start_closure;
        let match_mask = nfa.match_mask;
        let reads_mag_i = i32::from(nfa.reads_magnitude);
        let block = 256u32;
        let cfg = LaunchConfig {
            grid_dim: (n.div_ceil(block as usize) as u32, 1, 1),
            block_dim: (block, 1, 1),
            shared_mem_bytes: 0,
        };
        let reads_spec_i = i32::from(nfa.reads_spectral);
        let props_func = if d_mag.is_some() || d_class.is_some() || d_bytes.is_some() || d_spectral.is_some() {
            Some(scan_props_kernel()?)
        } else {
            None
        };
        let stream = g.ctx.default_stream();
        let ran = (|| {
            let d_atom = stream.clone_htod(&nfa.atom_kind)?;
            let d_next = stream.clone_htod(&nfa.next_closure)?;
            let d_isatom = stream.clone_htod(&nfa.is_atom)?;
            let d_result = stream.alloc_zeros::<i32>(n)?;
            match props_func {
                Some(func) => {
                    let d_lo = stream.clone_htod(&nfa.mag_lo)?;
                    let d_hi = stream.clone_htod(&nfa.mag_hi)?;
                    let d_back = stream.clone_htod(&nfa.ref_back)?;
                    let d_need = stream.clone_htod(&nfa.need_mask)?;
                    let d_lit = stream.clone_htod(&literal_classes)?;
                    let d_ent_lo = stream.clone_htod(&nfa.ent_lo)?;
                    let d_ent_hi = stream.clone_htod(&nfa.ent_hi)?;
                    let d_period_need = stream.clone_htod(&nfa.period_need)?;
                    let d_period_val = stream.clone_htod(&nfa.period_val)?;
                    let d_texture_need = stream.clone_htod(&nfa.texture_need)?;
                    let d_onset_need = stream.clone_htod(&nfa.onset_need)?;
                    let spare_mag = stream.clone_htod(&[0.0f32])?;
                    let spare_class = stream.clone_htod(&[0u32])?;
                    let spare_bytes = stream.clone_htod(&[0u32])?;
                    let spare_ent = stream.clone_htod(&[0.0f32])?;
                    let spare_spec = stream.clone_htod(&[0u32])?;
                    let mut builder = stream.launch_builder(func);
                    builder.arg(d_kind);
                    builder.arg(&n_i);
                    if let Some(d) = d_mag {
                        builder.arg(d);
                    } else {
                        builder.arg(&spare_mag);
                    }
                    builder.arg(&reads_mag_i);
                    if let Some(d) = d_class {
                        builder.arg(d);
                    } else {
                        builder.arg(&spare_class);
                    }
                    if let Some(d) = d_bytes {
                        builder.arg(d);
                    } else {
                        builder.arg(&spare_bytes);
                    }
                    match d_spectral {
                        Some((e, s)) => {
                            builder.arg(e);
                            builder.arg(s);
                        }
                        None => {
                            builder.arg(&spare_ent);
                            builder.arg(&spare_spec);
                        }
                    }
                    builder.arg(&reads_spec_i);
                    builder.arg(&d_atom);
                    builder.arg(&d_lo);
                    builder.arg(&d_hi);
                    builder.arg(&d_back);
                    builder.arg(&d_need);
                    builder.arg(&d_lit);
                    builder.arg(&d_ent_lo);
                    builder.arg(&d_ent_hi);
                    builder.arg(&d_period_need);
                    builder.arg(&d_period_val);
                    builder.arg(&d_texture_need);
                    builder.arg(&d_onset_need);
                    builder.arg(&d_next);
                    builder.arg(&d_isatom);
                    builder.arg(&start_closure);
                    builder.arg(&match_mask);
                    builder.arg(&nstates_i);
                    builder.arg(&d_result);
                    // SAFETY: the argument list matches `trex_scan_props`'s
                    // parameters in order and type; the kinds, and each
                    // property the pattern reads, number `n`, an unread
                    // property's one-entry buffer is never indexed, and the
                    // per-state tables number `nstates`.
                    unsafe { builder.launch(cfg)? };
                }
                _ => {
                    let mut builder = stream.launch_builder(&g.func);
                    builder.arg(d_kind);
                    builder.arg(&n_i);
                    builder.arg(&d_atom);
                    builder.arg(&d_next);
                    builder.arg(&d_isatom);
                    builder.arg(&start_closure);
                    builder.arg(&match_mask);
                    builder.arg(&nstates_i);
                    builder.arg(&d_result);
                    // SAFETY: as in `scan` - the argument list matches
                    // `trex_scan`'s parameters in order and type; the kinds
                    // number `n` and the tables `nstates`, which the kernel
                    // never indexes past.
                    unsafe { builder.launch(cfg)? };
                }
            }
            let result: Vec<i32> = stream.clone_dtoh(&d_result)?;
            stream.synchronize()?;
            Ok::<_, cudarc::driver::DriverError>(result)
        })();
        match ran {
            Ok(result) => Some(result),
            Err(e) => {
                eprintln!("trex gpu: a scan over {n} device-held tokens failed: {e:?}");
                None
            }
        }
    }

    /// [`scan`] for a pattern that reads a per-token property - a magnitude, a
    /// byte class, a literal or a register: the properties it reads go up with
    /// the kinds for this call.
    fn scan_props(g: &Gpu, nfa: &crate::nfa::GpuNfa, input: &[u8]) -> Option<Vec<Span>> {
        let toks = crate::parallel_lex::lex_parallel(input);
        let (kinds, spans) = significant(&toks);
        let n = spans.len();
        if n == 0 {
            return Some(Vec::new());
        }
        let mags = nfa.reads_magnitude.then(|| super::significant_magnitudes(&toks, input));
        let masks = nfa.reads_bytes.then(|| super::significant_byte_masks(&toks, input));
        let spectral = nfa
            .reads_spectral
            .then(|| super::significant_spectral(&toks, &crate::spectral::analyze(input)));
        let (classes, lit_class) = match nfa.class_group {
            Some(group) => {
                let texts: Vec<&[u8]> = nfa.literals.iter().flatten().map(Vec::as_slice).collect();
                let (token_ids, literal_ids) = super::significant_classes(&toks, input, group, &texts);
                let mut per_state = vec![u32::MAX; nfa.nstates];
                let mut ids = literal_ids.into_iter();
                for (slot, lit) in per_state.iter_mut().zip(&nfa.literals) {
                    if lit.is_some() {
                        *slot = ids.next().expect("one class id per literal");
                    }
                }
                (Some(token_ids), Some(per_state))
            }
            None => (None, None),
        };
        let stream = g.ctx.default_stream();
        let held = (|| {
            let d_kind = stream.clone_htod(&kinds)?;
            let d_mag = match &mags {
                Some(m) => Some(stream.clone_htod(m)?),
                None => None,
            };
            let d_class = match &classes {
                Some(c) => Some(stream.clone_htod(c)?),
                None => None,
            };
            let d_bytes = match &masks {
                Some(b) => Some(stream.clone_htod(b)?),
                None => None,
            };
            let d_spectral = match &spectral {
                Some((e, s)) => Some((stream.clone_htod(e)?, stream.clone_htod(s)?)),
                None => None,
            };
            Ok::<_, cudarc::driver::DriverError>((d_kind, d_mag, d_class, d_bytes, d_spectral))
        })();
        let (d_kind, d_mag, d_class, d_bytes, d_spectral) = match held {
            Ok(v) => v,
            Err(e) => {
                eprintln!("trex gpu: uploading {n} tokens and their properties failed: {e:?}");
                return None;
            }
        };
        let result = run_scan(
            g,
            &d_kind,
            d_mag.as_ref(),
            d_class.as_ref(),
            d_bytes.as_ref(),
            d_spectral.as_ref().map(|(e, s)| (e, s)),
            lit_class.as_deref(),
            nfa,
            n,
        )?;
        Some(select(&spans, &result))
    }

    /// [`scan`] over tokens already on the device: only the pattern's tables
    /// and the result buffer cross the bus. `spans` holds one entry per held
    /// kind, and `text` is present when the tokens were held with every
    /// property. `None` for a pattern that reads a property the tokens were
    /// held without, or an orbit group other than `text`'s; a device failure
    /// is reported before it becomes `None`.
    pub(super) fn scan_resident(
        kinds: &ResidentKinds,
        spans: &[(u32, u32)],
        text: Option<&super::HeldText>,
        pattern: &Pattern,
    ) -> Option<Vec<Span>> {
        let nfa = crate::nfa::compile_for_gpu(pattern)?;
        let g = gpu()?;
        if (nfa.reads_magnitude && !kinds.magnitudes) || (nfa.reads_spectral && !kinds.spectral) {
            return None;
        }
        let needs_text = nfa.binds_registers || nfa.class_group.is_some() || nfa.reads_bytes;
        if needs_text && text.is_none() {
            return None;
        }
        if let (Some(group), Some(t)) = (nfa.class_group, text)
            && group != t.classes.group()
        {
            return None;
        }
        let Some(d_kind) = &kinds.d_kind else {
            return Some(Vec::new());
        };
        // A literal no held token equals takes an id past every token's, and
        // below the kernel's no-literal marker.
        let lit_class: Option<Vec<u32>> = match (nfa.class_group, text) {
            (Some(_), Some(t)) => Some(
                nfa.literals
                    .iter()
                    .map(|lit| match lit {
                        Some(bytes) => t.classes.find(bytes).unwrap_or(u32::MAX - 1),
                        None => u32::MAX,
                    })
                    .collect(),
            ),
            _ => None,
        };
        let d_mag = if nfa.reads_magnitude { kinds.d_mag.as_ref() } else { None };
        let d_class = if nfa.class_group.is_some() { kinds.d_class.as_ref() } else { None };
        let d_bytes = if nfa.reads_bytes { kinds.d_bytes.as_ref() } else { None };
        let d_spectral = if nfa.reads_spectral { kinds.d_ent.as_ref().zip(kinds.d_spec.as_ref()) } else { None };
        let result =
            run_scan(g, d_kind, d_mag, d_class, d_bytes, d_spectral, lit_class.as_deref(), &nfa, spans.len())?;
        Some(select(spans, &result))
    }

    /// [`scan_resident`] with the anchors split between the host's cores and
    /// the device, both over a corpus the device already holds: the device
    /// scans the kinds from `mid` on out of its own buffer, so nothing is
    /// uploaded, while the host runs the same per-anchor automaton over
    /// `host_kinds` up to `mid`. `host_per_mille` is the host's share of the
    /// anchors, clamped to leave each side at least one. `None` for a pattern
    /// outside the kind-only subset; a device failure is reported, and its
    /// share is scanned on the host.
    pub(super) fn scan_resident_split(
        kinds: &ResidentKinds,
        host_kinds: &[u32],
        spans: &[(u32, u32)],
        pattern: &Pattern,
        host_per_mille: u32,
    ) -> Option<Vec<Span>> {
        scan_resident_split_timed(kinds, host_kinds, spans, pattern, host_per_mille)
            .map(|(out, _)| out)
    }

    /// [`scan_resident_split`]'s implementation, reporting where the call
    /// spent itself. The two are one function so the timed path and the
    /// production path cannot drift apart.
    pub(super) fn scan_resident_split_timed(
        kinds: &ResidentKinds,
        host_kinds: &[u32],
        spans: &[(u32, u32)],
        pattern: &Pattern,
        host_per_mille: u32,
    ) -> Option<(Vec<Span>, super::SplitPhases)> {
        use std::time::Instant;

        let nfa = crate::nfa::compile_for_gpu(pattern)?;
        if nfa.reads_magnitude
            || nfa.binds_registers
            || nfa.class_group.is_some()
            || nfa.reads_bytes
            || nfa.reads_spectral
        {
            return None;
        }
        let g = gpu()?;
        let n = host_kinds.len();
        let Some(d_kind) = &kinds.d_kind else {
            return Some((Vec::new(), super::SplitPhases::default()));
        };
        let wall = Instant::now();
        let mut ph = super::SplitPhases::default();
        // Both halves write every slot, so the array is only allocated here,
        // never filled: a zeroed allocation of this size comes from the
        // operating system's own zero pages, where writing a sentinel over it
        // would cost a pass across the whole corpus.
        let mut ends = vec![0i32; n];
        if n < 2 {
            let t = Instant::now();
            host_anchor_ends(&nfa, host_kinds, 0, &mut ends);
            ph.ends_us = t.elapsed().as_secs_f64() * 1e6;
            let t = Instant::now();
            let out = select(spans, &ends);
            ph.select_us = t.elapsed().as_secs_f64() * 1e6;
            ph.wall_us = wall.elapsed().as_secs_f64() * 1e6;
            return Some((out, ph));
        }
        let mid = (n * host_per_mille as usize / 1000).clamp(1, n - 1);
        let ends_at = Instant::now();
        let (host_part, device_part) = ends.split_at_mut(mid);
        std::thread::scope(|s| {
            s.spawn(|| {
                match resident_suffix_ends(g, &nfa, d_kind, mid, device_part) {
                    Ok(()) => {
                        let base = i32::try_from(mid).expect("a token index within the kernel's i32 width");
                        for slot in device_part.iter_mut() {
                            if *slot >= 0 {
                                *slot += base;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "trex gpu: the device's share of a resident split failed and the host scanned it: {e:?}"
                        );
                        host_anchor_ends(&nfa, host_kinds, mid, device_part);
                    }
                }
            });
            host_anchor_ends(&nfa, host_kinds, 0, host_part);
        });
        ph.ends_us = ends_at.elapsed().as_secs_f64() * 1e6;
        let t = Instant::now();
        let out = select(spans, &ends);
        ph.select_us = t.elapsed().as_secs_f64() * 1e6;
        ph.wall_us = wall.elapsed().as_secs_f64() * 1e6;
        Some((out, ph))
    }

    /// The kernel's ends for the anchors from `mid` on of the kinds the device
    /// holds, written into `out` relative to `mid`, with the automaton tables
    /// uploaded for this call. `out` is the tail of the caller's ends array,
    /// so the ends cross the bus once and are not copied again.
    fn resident_suffix_ends(
        g: &Gpu,
        nfa: &crate::nfa::GpuNfa,
        d_kind: &cudarc::driver::CudaSlice<u32>,
        mid: usize,
        out: &mut [i32],
    ) -> Result<(), cudarc::driver::DriverError> {
        let stream = g.ctx.default_stream();
        let d_atom = stream.clone_htod(&nfa.atom_kind)?;
        let d_next = stream.clone_htod(&nfa.next_closure)?;
        let d_isatom = stream.clone_htod(&nfa.is_atom)?;
        let view = d_kind.slice(mid..mid + out.len());
        device_view_ends(g, &stream, nfa, &view, out, &d_atom, &d_next, &d_isatom)
    }

    /// The significant tokens' kind codes and byte spans, in one pass.
    ///
    /// The device needs the kinds and the host selection needs the spans,
    /// and both are read in token order. Writing them as two contiguous
    /// arrays means the selection reads forward through memory instead of
    /// dereferencing a pointer into the token vector for every match: at a
    /// few million tokens the token vector is far past the last level of
    /// cache, so each of those dereferences is a miss, and the misses are
    /// what make an O(n) pass behave worse than O(n) as the corpus grows.
    ///
    /// The spans keep the `u32` width [`Token`] already stores them at.
    /// `start()` and `end()` widen to `usize` for callers, and widening
    /// here would double the array this pass writes - eight bytes a token
    /// against sixteen - which is the pass's own bottleneck at a few
    /// million tokens.
    ///
    /// Both arrays are reserved for the whole token count. That
    /// over-reserves by the whitespace share and costs one allocation each;
    /// growing them instead copies everything written so far at every
    /// doubling.
    fn significant(toks: &[Token]) -> (Vec<u32>, Vec<(u32, u32)>) {
        super::significant_stream(toks)
    }

    /// Leftmost, non-overlapping selection over the per-anchor longest match
    /// ends, mapped back to byte spans.
    fn select(spans: &[(u32, u32)], result: &[i32]) -> Vec<Span> {
        // Chosen once, so a walk whose calls cross two anchors does not run a
        // test on each of them.
        if takes_vector(result) {
            select_with(spans, result, next_match_from)
        } else {
            select_with(spans, result, next_match_from_scalar)
        }
    }

    /// [`select`] over one scan.
    fn select_with(
        spans: &[(u32, u32)],
        result: &[i32],
        scan: impl Fn(&[i32], usize, usize) -> Option<usize>,
    ) -> Vec<Span> {
        let n = spans.len();
        // A match consumes at least one token, so the count cannot exceed
        // the token count, and for a two-token pattern over dense input it
        // approaches half of it. Reserving that up front trades one
        // allocation for the log n reallocations a growing vector performs,
        // each of which copies everything written so far.
        let mut out = Vec::with_capacity(n / 2 + 1);
        let mut a = 0usize;
        while let Some(m) = scan(result, a, n) {
            let e = result[m] as usize; // tokens [m, e) matched
            out.push(Span { start: spans[m].0, end: spans[e - 1].1 });
            a = e;
        }
        out
    }

    /// The kind-only scan placed by what this call site has measured at the
    /// input's log2 size: the CPU engine alone, or the anchors split between
    /// the host's cores and the device ([`scan_split`]). A size with either
    /// side unmeasured, and every thirty-second call at a size, runs both one
    /// after the other and returns the CPU engine's matches, which the split's
    /// equal by construction. `None` for a pattern the split does not take.
    pub(super) fn scan_placed(
        pattern: &Pattern,
        input: &[u8],
    ) -> Option<(Vec<Span>, super::BackendUsed)> {
        use flynnel::sched::call_site::Placement;
        let nfa = crate::nfa::compile_for_gpu(pattern)?;
        if nfa.reads_magnitude
            || nfa.binds_registers
            || nfa.class_group.is_some()
            || nfa.reads_bytes
            || nfa.reads_spectral
        {
            return None;
        }
        let g = gpu()?;
        let site = flynnel::sched::call_site::caller_site().get();
        let size = input.len().min(u32::MAX as usize) as u32;
        let cpu = || {
            let t = std::time::Instant::now();
            (crate::engine::scan(pattern, input), elapsed_ns(t))
        };
        let split = || {
            let t = std::time::Instant::now();
            (scan_split(g, &nfa, input), elapsed_ns(t))
        };
        match site.choose_placement(size) {
            Placement::Cpu => {
                let (m, ns) = cpu();
                site.record_placement(size, Some(ns), None);
                Some((m, super::BackendUsed::Cpu))
            }
            Placement::Backend => {
                let (m, ns) = split();
                site.record_placement(size, None, Some(ns));
                crate::trace::rung("scan", "split across the cores and the device", input.len());
                Some((m, super::BackendUsed::Split))
            }
            Placement::Race => {
                // One after the other, so neither clock carries the other's
                // load, in an order that alternates so neither always runs on
                // the other's warm caches.
                let cpu_first =
                    RACES.fetch_add(1, std::sync::atomic::Ordering::Relaxed).is_multiple_of(2);
                let ((m, cpu_ns), split_ns) = if cpu_first {
                    let c = cpu();
                    (c, split().1)
                } else {
                    let s = split().1;
                    (cpu(), s)
                };
                site.record_placement(size, Some(cpu_ns), Some(split_ns));
                Some((m, super::BackendUsed::Cpu))
            }
        }
    }

    /// Races run by [`scan_placed`], counted to alternate their order.
    static RACES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn elapsed_ns(t: std::time::Instant) -> u64 {
        t.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
    }

    /// The kind-only scan with its anchors split between the host's cores and
    /// the device, both at once. The host runs the kernel's per-anchor
    /// automaton over anchors `0..mid` ([`host_anchor_ends`]); the device runs
    /// the kernel over the kinds from `mid` on ([`device_anchor_ends`]). An
    /// anchor's attempt reads only tokens at or after it, so the device needs
    /// no token before `mid`, a match may run across `mid`, and the leftmost,
    /// non-overlapping selection runs over the joined table. `mid` is the
    /// host's share this call site has measured at this token count.
    fn scan_split(g: &'static Gpu, nfa: &crate::nfa::GpuNfa, input: &[u8]) -> Vec<Span> {
        crate::parallel_lex::lex_significant_parallel_held(input, |kinds, spans| {
            let n = kinds.len();
            let mut ends = vec![-1i32; n];
            if n < 2 {
                host_anchor_ends(nfa, kinds, 0, &mut ends);
                return select(spans, &ends);
            }
            let site = flynnel::sched::call_site::caller_site().get();
            let key = n.min(u32::MAX as usize) as u32;
            let share = site.split_cpu_share_per_mille_for(key) as usize;
            let mid = (n * share / 1000).clamp(1, n - 1);
            let (host_part, device_part) = ends.split_at_mut(mid);
            let (host_ns, device_ns) = std::thread::scope(|s| {
                let device = s.spawn(|| {
                    let t = std::time::Instant::now();
                    if let Err(e) = device_anchor_ends(g, nfa, &kinds[mid..], mid, device_part) {
                        eprintln!(
                            "trex gpu: the device's share of a split scan failed and the host scanned it: {e:?}"
                        );
                        host_anchor_ends(nfa, kinds, mid, device_part);
                    }
                    elapsed_ns(t)
                });
                let t = std::time::Instant::now();
                host_anchor_ends(nfa, kinds, 0, host_part);
                let host_ns = elapsed_ns(t);
                (host_ns, device.join().expect("the device share's thread returns"))
            });
            site.record_split_for(key, mid, host_ns, n - mid, device_ns);
            select(spans, &ends)
        })
    }

    /// Each anchor's longest match end over `suffix`, the kinds from token
    /// `mid` on, written into `out` as whole-stream indices: the scan kernel
    /// over the suffix alone, its ends offset by `mid`.
    fn device_anchor_ends(
        g: &Gpu,
        nfa: &crate::nfa::GpuNfa,
        suffix: &[u32],
        mid: usize,
        out: &mut [i32],
    ) -> Result<(), cudarc::driver::DriverError> {
        let n = suffix.len();
        let stream = g.ctx.default_stream();
        let d_kind = stream.clone_htod(suffix)?;
        let d_atom = stream.clone_htod(&nfa.atom_kind)?;
        let d_next = stream.clone_htod(&nfa.next_closure)?;
        let d_isatom = stream.clone_htod(&nfa.is_atom)?;
        let d_result = stream.alloc_zeros::<i32>(n)?;
        let n_i = n as i32;
        let nstates_i = nfa.nstates as i32;
        let start_closure = nfa.start_closure;
        let match_mask = nfa.match_mask;
        let block = 256u32;
        let cfg = LaunchConfig {
            grid_dim: (n.div_ceil(block as usize) as u32, 1, 1),
            block_dim: (block, 1, 1),
            shared_mem_bytes: 0,
        };
        let mut builder = stream.launch_builder(&g.func);
        builder.arg(&d_kind);
        builder.arg(&n_i);
        builder.arg(&d_atom);
        builder.arg(&d_next);
        builder.arg(&d_isatom);
        builder.arg(&start_closure);
        builder.arg(&match_mask);
        builder.arg(&nstates_i);
        builder.arg(&d_result);
        // SAFETY: the argument list matches `trex_scan`'s parameters in order
        // and type, and every device buffer is sized to `n` or to `nstates`,
        // which the kernel never indexes past.
        unsafe { builder.launch(cfg)? };
        let ends: Vec<i32> = stream.clone_dtoh(&d_result)?;
        stream.synchronize()?;
        let base = i32::try_from(mid).expect("a token index within the kernel's i32 width");
        for (slot, e) in out.iter_mut().zip(ends) {
            *slot = if e < 0 { -1 } else { e + base };
        }
        Ok(())
    }

    /// Each anchor's longest match end for anchors `first..first + out.len()`,
    /// by the scan kernel's per-anchor automaton run across the host's cores:
    /// the same tables and the same steps, so a host end and a device end at
    /// one anchor are equal.
    pub(super) fn host_anchor_ends(nfa: &crate::nfa::GpuNfa, kinds: &[u32], first: usize, out: &mut [i32]) {
        use flynnel::JobPlan;
        use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        let min_leaf = out.len().div_ceil(cores * 4).max(64);
        let plan = JobPlan::new(0, out.len().min(u32::MAX as usize) as u32)
            .with_leaf_shape(flynnel::LeafShape::PortCompute);
        for_each_chunk_indexed_min_leaf(&plan, out, min_leaf, |start, slots| {
            for (i, slot) in slots.iter_mut().enumerate() {
                *slot = anchor_end(nfa, kinds, first + start + i);
            }
        });
    }

    /// `trex_scan`'s loop in `kernels/scan.cu` for anchor `a`: the longest
    /// match end, or -1 when no match begins there.
    #[inline]
    fn anchor_end(nfa: &crate::nfa::GpuNfa, kinds: &[u32], a: usize) -> i32 {
        let n = kinds.len();
        let mut active = nfa.start_closure;
        let mut best = -1i32;
        for (k, &tk) in kinds.iter().enumerate().skip(a) {
            if active & nfa.match_mask != 0 {
                best = k as i32;
            }
            let mut next = 0u64;
            let mut m = active;
            while m != 0 {
                let pc = m.trailing_zeros() as usize;
                m &= m - 1;
                if nfa.is_atom[pc] != 0 {
                    let ak = nfa.atom_kind[pc];
                    if ak == u32::MAX || ak == tk {
                        next |= nfa.next_closure[pc];
                    }
                }
            }
            if next == 0 {
                return best;
            }
            active = next;
        }
        if active & nfa.match_mask != 0 { n as i32 } else { best }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Word and number lines with runs of numbers, so an unbounded
        /// pattern has long matches that a cut can split.
        fn split_corpus() -> Vec<u8> {
            let mut s = String::new();
            for i in 0..3000u32 {
                s.push_str(&format!("tag {} {} word {}\n", i % 7, i % 13, i));
                if i % 5 == 0 {
                    s.push_str("1 2 3 4 5 6 7 8\n");
                }
            }
            s.into_bytes()
        }

        /// The host port of the kernel's per-anchor loop, run over every
        /// anchor, selects the CPU engine's spans.
        #[test]
        fn host_anchor_ends_select_the_cpu_engines_spans() {
            let input = split_corpus();
            let (kinds, spans) = crate::parallel_lex::lex_significant_parallel(&input);
            for src in ["\\W \\N", "\\N+", "\\N{2,4}", "(\\N \\W)+", ". .", "\\W \\W \\W"] {
                let pat = crate::parser::parse(src).expect("pattern parses");
                let nfa = crate::nfa::compile_for_gpu(&pat).expect("a kind-only pattern compiles for the kernel");
                let mut ends = vec![-1i32; kinds.len()];
                host_anchor_ends(&nfa, &kinds, 0, &mut ends);
                assert_eq!(select(&spans, &ends), crate::engine::scan(&pat, &input), "{src}");
            }
        }

        /// A split at any share selects the CPU engine's spans, a match that
        /// runs across the cut included.
        #[test]
        fn a_split_at_any_share_selects_the_cpu_engines_spans() {
            let Some(g) = gpu() else {
                eprintln!("no CUDA device present; the split agreement test did not run");
                return;
            };
            let input = split_corpus();
            let (kinds, spans) = crate::parallel_lex::lex_significant_parallel(&input);
            let n = kinds.len();
            for src in ["\\W \\N", "\\N+", "(\\N \\W)+", "\\W \\W \\W"] {
                let pat = crate::parser::parse(src).expect("pattern parses");
                let nfa = crate::nfa::compile_for_gpu(&pat).expect("a kind-only pattern compiles for the kernel");
                let want = crate::engine::scan(&pat, &input);
                for mid in [1, n / 3, n / 2, n - 1] {
                    let mut ends = vec![-1i32; n];
                    let (host_part, device_part) = ends.split_at_mut(mid);
                    host_anchor_ends(&nfa, &kinds, 0, host_part);
                    device_anchor_ends(g, &nfa, &kinds[mid..], mid, device_part)
                        .expect("the device ran its share");
                    assert_eq!(select(&spans, &ends), want, "{src} split at {mid} of {n}");
                }
            }
        }

        /// A pipelined scan selects the CPU engine's spans at any range count,
        /// matches that cross a range's end included, and declines a pattern
        /// whose longest match is unbounded.
        #[test]
        fn a_pipelined_scan_selects_the_cpu_engines_spans() {
            if gpu().is_none() {
                eprintln!("no CUDA device present; the pipelined scan test did not run");
                return;
            }
            let input = split_corpus().repeat(4);
            // Ranges are cut at line starts, and every match of `\W \N \W`
            // spans a line end.
            for src in ["\\W \\N", "\\N{2,4}", "\\W \\N \\W", ". .", "\\N"] {
                let pat = crate::parser::parse(src).expect("pattern parses");
                let want = crate::engine::scan(&pat, &input);
                assert!(!want.is_empty(), "{src} matches nothing, so nothing would be compared");
                for partitions in [1, 2, 3, 7, 64] {
                    let (got, ph) = scan_pipelined(&pat, &input, partitions).expect("the device ran the pipeline");
                    assert_eq!(got, want, "{src} over {partitions} ranges");
                    assert!(ph.partitions >= 1, "{src} over {partitions} ranges lexed no range");
                }
            }
            let unbounded = crate::parser::parse("\\N+").expect("pattern parses");
            assert!(scan_pipelined(&unbounded, &input, 4).is_none(), "an unbounded pattern has no carry length");
        }

        /// A scan over the lexed chunks in place selects the CPU engine's
        /// spans on an input lexed in many chunks, matches that cross a
        /// chunk's end included, on an input lexed as one chunk, and on an
        /// empty input.
        #[test]
        fn a_parts_scan_selects_the_cpu_engines_spans() {
            if gpu().is_none() {
                eprintln!("no CUDA device present; the parts scan test did not run");
                return;
            }
            let many = split_corpus().repeat(4);
            let one = b"tag 1 2 word 3\ntag 4 5 word 6\n".to_vec();
            // Chunks are cut at line starts, and every match of `\W \N \W`
            // spans a line end.
            for src in ["\\W \\N", "\\N{2,4}", "\\W \\N \\W", ". .", "\\N", "\\N+"] {
                let pat = crate::parser::parse(src).expect("pattern parses");
                for input in [&many, &one] {
                    let want = crate::engine::scan(&pat, input);
                    assert!(!want.is_empty(), "{src} matches nothing in {} bytes", input.len());
                    let got = scan_parts(&pat, input).expect("the device ran the parts scan");
                    assert_eq!(got, want, "{src} over {} bytes", input.len());
                }
                assert_eq!(scan_parts(&pat, b""), Some(Vec::new()), "{src} over an empty input");
            }
        }

        /// A split over a corpus the device holds selects the CPU engine's
        /// spans at any host share, matches that cross the cut included, and
        /// on an empty input.
        #[test]
        fn a_resident_split_selects_the_cpu_engines_spans() {
            if gpu().is_none() {
                eprintln!("no CUDA device present; the resident split test did not run");
                return;
            }
            let input = split_corpus();
            let held = crate::gpu::GpuTokens::upload(&input).expect("the device holds the corpus");
            for src in ["\\W \\N", "\\N+", "\\N{2,4}", "\\W \\N \\W", ". .", "\\N"] {
                let pat = crate::parser::parse(src).expect("pattern parses");
                let want = crate::engine::scan(&pat, &input);
                assert!(!want.is_empty(), "{src} matches nothing, so nothing would be compared");
                for share in [1u32, 100, 500, 900, 999] {
                    let got = held.scan_split(&pat, share).expect("the device ran the resident split");
                    assert_eq!(got, want, "{src} at a host share of {share} per mille");
                }
            }
            let empty = crate::gpu::GpuTokens::upload(b"").expect("the device holds an empty corpus");
            let pat = crate::parser::parse("\\N").expect("pattern parses");
            assert_eq!(empty.scan_split(&pat, 500), Some(Vec::new()), "an empty corpus has no match");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::parser::parse;

    #[test]
    fn eligibility_matches_the_documented_subset() {
        // In the subset: typed atoms, dot, concatenation, quantifiers, and
        // absolute magnitude tests.
        for src in [
            "\\N",
            "\\W \\N",
            "\\N+",
            "\\W*",
            ".",
            ". .",
            "\\N{2,4}",
            "(\\N \\W)+",
            "\\M{>6}",
            "\\N{mag<3}",
            "\\W \\N{mag>=2}",
            "\\W:t",
            "\\W:x =x",
            "\\W:x \\N =case x",
            "\"lit\"",
            "\\d",
            "\\W:x \"lit\"",
        ] {
            assert!(gpu_eligible(&parse(src).unwrap()), "{src} should be eligible");
        }
        // Out of the subset: alternation, a bind no reference reads at one
        // offset, a literal and a reference under different groups, balance,
        // field, guard, relative magnitude, and byte patterns.
        for src in [
            "\\N{>+1}",
            "\\W+:x =x",
            "\\W:x \\N* =x",
            "\\W:x =case x \"lit\"",
            "\\N | \\W",
            "\\W\\B(.*)",
            "@2 \\W",
            ". ~\"END\"",
            "`[a-z]+`",
        ] {
            assert!(!gpu_eligible(&parse(src).unwrap()), "{src} should be ineligible");
        }
    }

    #[cfg(not(feature = "gpu"))]
    #[test]
    fn scan_gpu_is_none_without_the_feature() {
        // Without the feature the GPU path never applies; the caller
        // falls back to the CPU engine.
        assert!(scan_gpu(&parse("\\N \\W").unwrap(), b"12 kg").is_none());
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn the_device_agrees_with_the_cpu_over_many_tokens() {
        // The twenty-byte case below exercises the device path but not the
        // parts of it that only appear in bulk: the kind and span arrays
        // spanning many cache lines, runs of whitespace between matches, a
        // match ending at the final token, and a match count large enough
        // that the output vector is filled rather than merely started.
        let pat = parse("\\W \\N").expect("pattern parses");
        let mut input = String::new();
        for i in 0..20_000 {
            // Whitespace runs of varying width, so the significant
            // subsequence is not a fixed stride through the token vector
            // and a span read from the wrong index lands on the wrong text.
            input.push_str(match i % 4 {
                0 => "tag ",
                1 => "tag\t\t",
                2 => "tag \n ",
                _ => "tag   ",
            });
            input.push_str(&(i % 997).to_string());
            input.push(' ');
        }
        let bytes = input.as_bytes();
        let Some(device) = scan_gpu(&pat, bytes) else {
            // No CUDA device on this host: the path under test cannot run,
            // and reporting a pass would say it agreed when it never ran.
            eprintln!("no CUDA device present; device-agreement test did not run");
            return;
        };
        let cpu = crate::engine::scan(&pat, bytes);
        assert_eq!(device.len(), cpu.len(), "device and cpu must find the same number of matches");
        assert_eq!(device, cpu, "device and cpu must agree on every span");
        assert!(!cpu.is_empty(), "the corpus must actually match, or this asserts nothing");
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn the_device_agrees_with_the_cpu_on_magnitude_tests() {
        // Numbers across ten orders of magnitude, fractions below one, and
        // words of several lengths, so each threshold has tokens on both sides
        // of it and a magnitude read for the wrong token changes a match.
        let mut input = String::new();
        for i in 0..20_000u64 {
            input.push_str(match i % 3 {
                0 => "size ",
                1 => "tiny\t",
                _ => "enormousword \n ",
            });
            input.push_str(&(i * i * 37 % 10_000_019).to_string());
            input.push(' ');
            if i % 5 == 0 {
                input.push_str("0.004 ");
            }
        }
        let bytes = input.as_bytes();
        for src in [
            "\\N{mag>6}",
            "\\N{mag<3}",
            "\\M{>1}",
            "\\M{<=2} \\N{mag>=5}",
            "(\\W \\N{mag>4})+",
        ] {
            let pat = parse(src).expect("pattern parses");
            assert!(gpu_eligible(&pat), "{src} is in the device subset");
            let Some(device) = scan_gpu(&pat, bytes) else {
                eprintln!("no CUDA device present; magnitude agreement test did not run");
                return;
            };
            let cpu = crate::engine::scan(&pat, bytes);
            assert!(!cpu.is_empty(), "{src} must match the corpus, or this asserts nothing");
            assert_eq!(device, cpu, "{src}: device and cpu must agree on every span");
            let held = GpuTokens::upload_with_magnitudes(bytes).expect("the device ran the scan");
            assert_eq!(held.scan(&pat), Some(cpu), "{src}: held tokens and cpu must agree");
            let kinds_only = GpuTokens::upload(bytes).expect("the device ran the scan");
            assert_eq!(kinds_only.scan(&pat), None, "{src}: tokens held without magnitudes decline");
        }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn the_device_agrees_with_the_cpu_on_binds() {
        // Words repeating at one and two tokens' distance, in mixed case, with
        // numbers between, so every reference is found both true and false and
        // a class read from the wrong token changes a match or its capture.
        let words = ["alpha", "Alpha", "beta", "BETA", "gamma"];
        let mut input = String::new();
        for i in 0..20_000usize {
            input.push_str(words[i % 5]);
            input.push(' ');
            input.push_str(words[(i * 7 + i / 3) % 5]);
            input.push_str(if i % 4 == 0 { "\t" } else { " " });
            input.push_str(&(i % 13).to_string());
            input.push(' ');
            if i % 3 == 0 {
                input.push_str("Alpha 7 alpha gamma 7 gamma delta x DELTA beta beta ");
            }
        }
        let bytes = input.as_bytes();
        for src in ["\\W:x =x", "\\W:x =case x", "\\W:x \\N =x", "\\W:x \\W =case x", "(\\W:x =case x)+", "\\W:t \\N"] {
            let pat = parse(src).expect("pattern parses");
            assert!(gpu_eligible(&pat), "{src} is in the device subset");
            let Some(device) = scan_gpu(&pat, bytes) else {
                eprintln!("no CUDA device present; bind agreement test did not run");
                return;
            };
            let cpu = crate::engine::scan(&pat, bytes);
            assert!(!cpu.is_empty(), "{src} must match the corpus, or this asserts nothing");
            assert_eq!(device, cpu, "{src}: device and cpu must agree on every span");
            let identity = GpuTokens::upload_with_properties(bytes, crate::orbit::OrbitGroup::Identity)
                .expect("the device ran the scan");
            let case = GpuTokens::upload_with_properties(bytes, crate::orbit::OrbitGroup::Case)
                .expect("the device ran the scan");
            let held = identity.scan(&pat).or_else(|| case.scan(&pat));
            assert_eq!(held, Some(cpu), "{src}: tokens held with their properties agree with the cpu");
        }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn the_device_agrees_with_the_cpu_on_literals_and_byte_classes() {
        // A literal in three cases, hex-looking and mixed-case words, numbers
        // and two separators, so every literal and byte class is found true and
        // false and a class or mask read for the wrong token changes a match.
        let mut input = String::new();
        for i in 0..20_000usize {
            input.push_str(["the", "The", "THE", "cat", "x1f"][i % 5]);
            input.push(' ');
            input.push_str(&(i % 97).to_string());
            input.push_str(if i % 3 == 0 { " , " } else { " ; " });
            input.push_str(["ABC", "abc", "a_b", "Zz9"][i % 4]);
            input.push('\n');
        }
        let bytes = input.as_bytes();
        for src in [
            "\"the\" \\N",
            "(?orbit:case \"the\") \\N",
            "\",\" \\w",
            "\\d \";\"",
            "\\u",
            "\\l \\W",
            "\\a+",
            "\\W:x \"the\"",
        ] {
            let pat = parse(src).expect("pattern parses");
            assert!(gpu_eligible(&pat), "{src} is in the device subset");
            let Some(device) = scan_gpu(&pat, bytes) else {
                eprintln!("no CUDA device present; literal and byte-class agreement test did not run");
                return;
            };
            let cpu = crate::engine::scan(&pat, bytes);
            assert!(!cpu.is_empty(), "{src} must match the corpus, or this asserts nothing");
            assert_eq!(device, cpu, "{src}: device and cpu must agree on every span");
            let identity = GpuTokens::upload_with_properties(bytes, crate::orbit::OrbitGroup::Identity)
                .expect("the device ran the scan");
            let case = GpuTokens::upload_with_properties(bytes, crate::orbit::OrbitGroup::Case)
                .expect("the device ran the scan");
            let held = identity.scan(&pat).or_else(|| case.scan(&pat));
            assert_eq!(held, Some(cpu), "{src}: tokens held with their properties agree with the cpu");
        }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn the_device_agrees_with_the_cpu_on_hex_runs() {
        // Hex runs either side of every digest length, digests of each length,
        // a decimal run of a digest's length and a base64 blob on every line,
        // so the hex kind is found true and false beside each kind whose runs
        // it borders, and a kind code read for the wrong token changes a match.
        let digits = b"0123456789abcdef";
        let mut input = String::new();
        for i in 0..20_000usize {
            let len = [33usize, 39, 41, 52, 63, 65, 96, 32, 40, 64][i % 10];
            let run: String = (0..len).map(|j| char::from(digits[(i * 7 + j * 3) % 16])).collect();
            input.push_str(["key ", "sum ", "id\t"][i % 3]);
            input.push_str(&run);
            input.push_str(if i % 4 == 0 { " = " } else { " ; " });
            input.push_str(&"7".repeat(40));
            input.push_str(" aB3dEfGhIjKlMnOp\n");
        }
        let bytes = input.as_bytes();
        for src in ["\\{hex}", "\\W \\{hex}", "\\{hex} \";\"", "\\{hex} \\P \\N", "\\{hash}"] {
            let pat = parse(src).expect("pattern parses");
            assert!(gpu_eligible(&pat), "{src} is in the device subset");
            let Some(device) = scan_gpu(&pat, bytes) else {
                eprintln!("no CUDA device present; hex agreement test did not run");
                return;
            };
            let cpu = crate::engine::scan(&pat, bytes);
            assert!(!cpu.is_empty(), "{src} must match the corpus, or this asserts nothing");
            assert_eq!(device, cpu, "{src}: device and cpu must agree on every span");
            let held = GpuTokens::upload_with_properties(bytes, crate::orbit::OrbitGroup::Identity)
                .expect("the device ran the scan");
            assert_eq!(held.scan(&pat), Some(cpu), "{src}: tokens held with their properties agree with the cpu");
        }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn the_device_agrees_with_the_cpu_on_spectral_tests() {
        // Prose, base64 blobs, a fixed-width table and a run of one byte,
        // so entropy, period, texture and change-points each take both
        // values across the tokens.
        let mut input = String::new();
        for i in 0..400usize {
            input.push_str("the quick brown fox jumps over the lazy dog and the cat sat on the mat ");
            if i % 3 == 0 {
                let mut x = 0x2545_f491_4f6c_dd1du64 ^ (i as u64);
                for _ in 0..200 {
                    x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                    input.push(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"[((x >> 58) % 64) as usize] as char);
                }
                input.push(' ');
            }
            if i % 5 == 0 {
                for r in 0..12 {
                    input.push_str(&format!("{:03},{:03},{:03}\n", r, (r * 7) % 100, (r * 13) % 100));
                }
            }
            if i % 7 == 0 {
                input.push_str(&"x".repeat(150));
                input.push(' ');
            }
        }
        let bytes = input.as_bytes();
        for src in [
            "\\F{entropy>0.8}",
            "\\F{entropy<0.3} \\W",
            "\\F{period:any}",
            "\\F{texture:prose} \\W",
            "\\F{texture:data}",
            "\\F{onset}",
            "\\W \\F{entropy>0.8}",
        ] {
            let pat = parse(src).expect("pattern parses");
            assert!(gpu_eligible(&pat), "{src} is in the device subset");
            let Some(device) = scan_gpu(&pat, bytes) else {
                eprintln!("no CUDA device present; spectral agreement test did not run");
                return;
            };
            let cpu = crate::engine::scan(&pat, bytes);
            assert!(!cpu.is_empty(), "{src} must match the corpus, or this asserts nothing");
            assert_eq!(device, cpu, "{src}: device and cpu must agree on every span");
            let held = GpuTokens::upload_with_spectral(bytes, crate::orbit::OrbitGroup::Identity)
                .expect("the device ran the scan");
            assert_eq!(held.scan(&pat), Some(cpu), "{src}: tokens held with the spectral reading agree");
            let without = GpuTokens::upload_with_properties(bytes, crate::orbit::OrbitGroup::Identity)
                .expect("the device ran the scan");
            assert_eq!(without.scan(&pat), None, "{src}: tokens held without the reading decline");
        }
        assert!(!gpu_eligible(&parse("\\W:x \\F{onset}").unwrap()), "a bind beside a spectral test stays on the CPU");
    }

    #[test]
    fn every_backend_returns_the_cpu_match_set() {
        // The device path is verified byte-identical to the CPU, so all
        // three backends must return the same matches as a plain scan,
        // on a GPU build or not.
        let pat = parse("\\W \\N").unwrap();
        let input = b"tag 12 tag 34 tag 56";
        let baseline = crate::engine::scan(&pat, input);
        for b in [Backend::Auto, Backend::Gpu, Backend::Cpu] {
            let (m, _used) = scan_with_backend(&pat, input, b);
            assert_eq!(m, baseline, "{b:?} must match the CPU baseline");
        }
    }

    #[test]
    fn auto_matches_the_cpu_engine_while_its_placement_learns() {
        // Enough calls at one size for the placement to race both routes and
        // then take the one it measured faster, with the split's share moving.
        let pat = parse("\\W \\N").unwrap();
        let mut input = String::new();
        for i in 0..40_000u32 {
            input.push_str(&format!("tag {}\n", i % 1000));
        }
        let want = crate::engine::scan(&pat, input.as_bytes());
        for _ in 0..40 {
            let (m, _used) = scan_with_backend(&pat, input.as_bytes(), Backend::Auto);
            assert_eq!(m, want, "Auto returns the CPU engine's spans whichever route it took");
        }
    }

    #[test]
    fn auto_routes_only_kind_patterns_to_the_device() {
        for src in ["\\W \\N", "\\N+", ". ."] {
            assert!(gpu_auto_routes(&parse(src).unwrap()), "{src} reads only token kinds");
        }
        let input = "tag 12 the 34 ".repeat(2_000);
        for src in ["\\W:x =x", "\\N{mag>6}", "\"the\" \\W", "\\d", "\\W:t \\N"] {
            let pat = parse(src).unwrap();
            assert!(gpu_eligible(&pat), "{src} is in the device subset");
            assert!(!gpu_auto_routes(&pat), "{src} reads a token property");
            let (m, used) = scan_with_backend(&pat, input.as_bytes(), Backend::Auto);
            assert_eq!(used, BackendUsed::Cpu, "{src}: auto keeps a property-reading pattern on the cores");
            assert_eq!(m, crate::engine::scan(&pat, input.as_bytes()), "{src}: auto returns the CPU matches");
        }
    }

    #[cfg(not(feature = "gpu"))]
    #[test]
    fn without_a_device_every_backend_runs_on_the_cpu() {
        // No feature means no device: `device_available` is false and
        // every backend, including a forced `Gpu`, resolves to the CPU.
        assert!(!device_available());
        let pat = parse("\\W \\N").unwrap();
        let input = b"tag 12 tag 34";
        for b in [Backend::Auto, Backend::Gpu, Backend::Cpu] {
            let (_m, used) = scan_with_backend(&pat, input, b);
            assert_eq!(used, BackendUsed::Cpu, "no device: {b:?} must use the CPU");
        }
    }
}
