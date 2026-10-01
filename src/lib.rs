//! trex: Token-Regular EXpression.
//!
//! trex is a regex-shaped pattern language whose alphabet is
//! typed tokens rather than raw bytes. A structure-aware lexer
//! turns input into a stream of typed atoms (numbers, words,
//! quoted strings, IPs, balanced bracket groups, punctuation),
//! and patterns are matched over that stream by a single-pass
//! engine, with a set-reachability engine (the operational form of
//! the Antimirov partial derivative) for the constructs it routes
//! away; both carry a register environment.
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
//! Tokens are spans of bytes, so a scan has two stages over the
//! same input: the byte grain lexes the bytes into tokens and the
//! token grain matches the pattern over them. [`scan_dual_grain`]
//! runs the two on two threads as a producer and a consumer and
//! returns the matches [`scan`] returns. See
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

// The wiki pages that hold Rust examples, each compiled as doctests.
//
// rustdoc compiles and runs a fenced rust block in a doc comment, and
// `#[doc = include_str!(..)]` points one at a file, so a page becomes a set of
// doctests with no extractor and no second build system. `cfg(doctest)` keeps
// the modules out of every other build. A page's front matter, shortcodes and
// other fences are inert: rustdoc renders them as text and compiles none of
// them. tests/wiki_examples.rs fails when a page holding a rust block is not
// named here.
#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/pattern-syntax.md")]
mod wiki_pattern_syntax {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/matching.md")]
mod wiki_matching {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/windows.md")]
mod wiki_windows {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/rewriting.md")]
mod wiki_rewriting {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/redaction.md")]
mod wiki_redaction {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/aggregates.md")]
mod wiki_aggregates {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/records.md")]
mod wiki_records {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/building-patterns.md")]
mod wiki_building_patterns {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/pattern-files.md")]
mod wiki_pattern_files {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/tools.md")]
mod wiki_tools {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/context.md")]
mod wiki_axis_context {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/magnitude.md")]
mod wiki_axis_magnitude {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/stress.md")]
mod wiki_axis_stress {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/flow.md")]
mod wiki_axis_flow {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/observation.md")]
mod wiki_axis_observation {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/echo.md")]
mod wiki_axis_echo {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/orbit.md")]
mod wiki_axis_orbit {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/shape.md")]
mod wiki_axis_shape {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/spectral.md")]
mod wiki_axis_spectral {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/seam.md")]
mod wiki_axis_seam {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/reference/axes/relation.md")]
mod wiki_axis_relation {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/extract-fields.md")]
mod wiki_howto_extract_fields {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/rename-matches.md")]
mod wiki_howto_rename_matches {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/find-tables.md")]
mod wiki_howto_find_tables {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/match-up-to-a-symmetry.md")]
mod wiki_howto_match_up_to_a_symmetry {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/segment-without-delimiters.md")]
mod wiki_howto_segment_without_delimiters {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/find-outliers.md")]
mod wiki_howto_find_outliers {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/choose-a-backend.md")]
mod wiki_howto_choose_a_backend {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/declare-your-own-atoms.md")]
mod wiki_howto_declare_your_own_atoms {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/build-and-reuse-a-pattern.md")]
mod wiki_howto_build_and_reuse_a_pattern {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/query-records.md")]
mod wiki_howto_query_records {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/count-and-rank-matches.md")]
mod wiki_howto_count_and_rank_matches {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/redact-and-follow-logs.md")]
mod wiki_howto_redact_and_follow_logs {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/search-a-tree-of-files.md")]
mod wiki_howto_search_a_tree_of_files {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/index-a-tree.md")]
mod wiki_howto_index_a_tree {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/lint-with-rules.md")]
mod wiki_howto_lint_with_rules {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/test-a-pattern-file.md")]
mod wiki_howto_test_a_pattern_file {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/filter-by-typed-values.md")]
mod wiki_howto_filter_by_typed_values {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/scan-many-patterns-at-once.md")]
mod wiki_howto_scan_many_patterns_at_once {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/explain-why-a-pattern-matched.md")]
mod wiki_howto_explain_why_a_pattern_matched {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/match-inside-encoded-content.md")]
mod wiki_howto_match_inside_encoded_content {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/summarize-a-log-by-templates.md")]
mod wiki_howto_summarize_a_log_by_templates {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/scan-input-in-pieces.md")]
mod wiki_howto_scan_input_in_pieces {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/parse-with-a-token-grammar.md")]
mod wiki_howto_parse_with_a_token_grammar {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/split-run-together-words.md")]
mod wiki_howto_split_run_together_words {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/learn-a-subword-tokenizer.md")]
mod wiki_howto_learn_a_subword_tokenizer {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/find-deep-nesting.md")]
mod wiki_howto_find_deep_nesting {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/find-repeated-or-new-content.md")]
mod wiki_howto_find_repeated_or_new_content {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/find-rare-lines-and-out-of-order-times.md")]
mod wiki_howto_find_rare_lines_and_out_of_order_times {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/find-where-text-changes-kind.md")]
mod wiki_howto_find_where_text_changes_kind {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/how-to/pre-check-a-corpus-for-a-literal.md")]
mod wiki_howto_pre_check_a_corpus_for_a_literal {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/tutorial/getting-started.md")]
mod wiki_tutorial_getting_started {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/tutorial/first-patterns.md")]
mod wiki_tutorial_first_patterns {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/tutorial/binding-and-balance.md")]
mod wiki_tutorial_binding_and_balance {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/tutorial/axes-and-tools.md")]
mod wiki_tutorial_axes_and_tools {}

#[cfg(doctest)]
#[doc = include_str!("../wiki/content/docs/tutorial/beyond-regex.md")]
mod wiki_tutorial_beyond_regex {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}
