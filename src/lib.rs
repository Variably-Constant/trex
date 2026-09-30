//! trex: Token-Regular EXpression.
//!
//! trex is a regex-shaped pattern language whose alphabet is
//! typed tokens rather than raw bytes. A structure-aware lexer
//! turns input into a stream of typed atoms (numbers, words,
//! quoted strings, IPs, balanced bracket groups, punctuation),
//! and patterns are matched over that stream by an Antimirov
//! partial-derivative engine carrying a register environment.
//!
//! The result is a one-line, regex-terse surface that expresses
//! three things regular expressions cannot:
//!
//! - balanced, nestable bracket groups (the lexer pairs them, so
//!   `\B(...)` is a single primitive rather than an impossible
//!   recursion),
//! - long-distance binding (`:name` writes a register, `=name`
//!   requires an equal token later), evaluated by a register-set
//!   automaton with no catastrophic backtracking, unlike a
//!   backtracking backreference,
//! - content-addressed lookahead (`~"lit"`), answered by a
//!   presence prefilter rather than a positional scan.
//!
//! ## Dual-grain scanning
//!
//! Tokens are spans of bytes. trex runs two co-operating grains
//! over the same input at once: a token-grain engine that owns
//! structure (binding, balance, valency) and a byte-grain engine
//! that owns literal speed and sub-token detail. The two grains
//! are complementary, not redundant: each answers a different
//! question about the same bytes, and their results join. See
//! `wiki/content/docs/explanation/architecture.md` for the architecture.

// Crate-wide: no unsafe, with two deliberate exceptions, both under the
// same runtime-feature-detection safety contract. The hand-rolled SIMD
// byte search in `byte_simd` carries a module-scoped
// `#![allow(unsafe_code)]`; the entropy flag pass in `spectral` carries
// an item-scoped one on its dispatcher and its gated body. Calling a
// `#[target_feature]` function on a CPU without that feature is
// undefined behavior, and in both places an `is_x86_feature_detected!`
// probe immediately before the call is what discharges it. `deny` (not
// `forbid`) is what lets those opt in.
#![deny(unsafe_code)]

pub mod action;
pub mod ast;
pub mod byte_dfa;
pub mod byte_lex;
pub mod byte_nfa;
pub mod byte_simd;
pub mod bytepat;
pub mod bpe;
pub mod builder;
pub mod canon;
pub mod captures;
mod checksum;
pub mod context;
pub mod cursor;
pub mod curvature;
pub mod decoded;
pub mod custom;
pub mod dual_grain;
pub mod e8;
pub mod echo;
pub mod edit;
pub mod encoding;
pub mod ends_simd;
pub mod entanglement;
pub mod files;
pub mod flow;
pub mod follow;
mod fxhash;
pub mod gauge;
pub mod engine;
pub mod gpu;
pub mod geodesic;
pub mod grammar;
pub mod gravity;
pub mod holography;
pub mod isa;
pub mod kind_route;
pub mod lexer;
pub mod library;
pub mod magnitude;
pub mod nfa;
pub mod observation;
pub mod orbit;
pub mod paint;
pub mod parallel_lex;
pub mod parser;
pub mod pattern_set;
pub mod prefilter;
#[cfg(feature = "compress")]
pub mod prior_cache;
pub mod profile;
pub mod explain;
pub mod index;
pub mod infer;
pub mod quantity;
pub mod records;
pub mod templates;
pub mod relation;
pub mod report;
pub mod resonator;
pub mod rewrite;
pub mod rule_scan;
pub mod seam;
pub mod shape;
pub mod spectral;
pub mod streaming;
pub mod stress;
pub mod supertoken;
#[cfg(feature = "tandem")]
pub mod tandem;
pub mod token;
pub mod tokutil;
pub mod topology;
pub mod trace;
pub mod typed;
pub mod window;

/// Where the notes of the scheduler trex runs its parallel work on go: a
/// calibration it measured, a lever it read. A program embedding trex
/// installs a sink with [`notice::set_sink`]; with none installed they print
/// to the standard error.
pub use flynnel::notice;

pub use context::{
    Agreement, ContextConfig, ContextField, RelationContext, RelationReading, Window,
    WindowProfile,
};
pub use captures::{
    CaptureSlots, SlotCursor, captures_read, captures_read_at, captures_read_iter,
    static_captures_len, static_token_extent,
};
pub use cursor::{
    Cursor, MatchCursor, MatchRef, Split, capture_names, captures_at, captures_first, captures_iter,
    captures_len, escape, find, find_at, find_iter, is_match_at, shortest_match,
    shortest_match_at, split, split_at_depth, splitn,
};
pub use curvature::{Curvature, EdgeCurvature};
pub use dual_grain::{GrainTiming, scan_dual_grain};
pub use echo::{EchoConfig, EchoField, EchoFrame, SuperEcho};
pub use entanglement::Entanglement;
pub use custom::{LibTest, Precedence, Rule, Severity, ShapeError, ShapeSet, TestFailure, TokenShape};
pub use ast::EmptyLoop;
pub use builder::PatternBuilder;
pub use pattern_set::{PatternSet, SetMatches};
pub use engine::{
    INLINE_REGS, Match, Regs, Span, captures, captures_over, captures_over_with_lists, captures_with_empty_loop,
    captures_with_lists, captures_with_shapes, captures_with_shapes_and_lists, is_match, scan,
    scan_with_empty_loop, scan_with_shapes,
};
pub use parser::parse_with_empty_loop;
pub use flow::{
    AnalyticConfig, AnalyticField, FlowConfig, FlowField, FlowFrame, Signal, analytic,
    analytic_signal,
};
pub use gpu::{Backend, BackendUsed, device_available, gpu_eligible, scan_gpu, scan_with_backend};
pub use geodesic::Geodesic;
pub use grammar::{Grammar, GrammarError, Node};
pub use holography::Holography;
pub use magnitude::{MagnitudeConfig, MagnitudeField, MagnitudeFrame};
pub use observation::{ObservationConfig, ObservationField, ObservationFrame};
pub use orbit::{OrbitGroup, OrbitToken, canonical, same_orbit, tokenize};
pub use parser::{ParseError, parse, parse_with_inputs};
pub use typed::{
    Clock, date_order_day_first, now_override, set_date_order_day_first, set_now, set_tz_offset,
};
pub use profile::{
    AxisCtx, AxisProfile, EchoProfile, MagnitudeProfile, Profile, ShapeProfile, SpectralProfile,
    StressProfile, fold_profiles, fold_tokens,
};
pub use relation::{Chord, Edge, RelationField, RelationFrame, RelationKind};
pub use resonator::{
    Spectrum, analyze_symbols, over_byte_values, over_bytes, over_supertokens, over_tokens,
};
pub use rewrite::{
    Field, Keep, Mask, Matched, Reference, ReportAt, ReportField, ReportRule, Spanned, Template,
    TemplateError, redactions, redactions_with_shapes, rewrite, rewrite_first, rewrite_n, rewrite_n_with,
    rewrite_with, rewrite_with_backend,
};
pub use seam::{Recovery, SeamConfig, SeamField};
pub use shape::{RegionKind, ShapeConfig, ShapeField, ShapeFrame};
pub use spectral::{
    CodeTexture, SpectralField, SpectralFrame, Texture, analyze, code_regions, code_texture,
    high_entropy_runs, regions,
};
pub use streaming::{HeldStream, StreamScanner, scan_chunked};
pub use stress::{StressConfig, StressField, StressFrame};
pub use supertoken::{Role, SuperToken, supertokens};
pub use topology::Topology;

/// The crate version string, sourced from the package manifest.
///
/// Exposed so the binary and any embedding host report a single
/// authoritative version.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// The wiki pages whose Rust blocks describe this crate's API, compiled as
// doctests so they cannot drift from it.
//
// rustdoc already compiles a fenced rust block in a doc comment, and
// `#[doc = include_str!(..)]` points it at a file, so a page becomes a set of
// doctests with no extractor and no second build system. `cfg(doctest)` keeps
// the modules out of every other build, so this costs a test run and nothing
// else. The pages' front matter is inert: rustdoc renders it as text and
// compiles none of it.
//
// A block here is a claim about the API that nothing was checking. That is not
// hypothetical - the reference described a match's captures as a type it has
// never been - and an example a reader copies is worth less than nothing when
// it does not build.
#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/library-api.md")]
mod wiki_library_api {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/context.md")]
mod wiki_axis_context {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/shape.md")]
mod wiki_axis_shape {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}
