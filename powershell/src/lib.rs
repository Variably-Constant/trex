//! trex as a PowerShell module: the pattern language over typed tokens as
//! cmdlets and objects, bound to the Rust directly.
//!
//! A pattern is trex source text, compiled when a cmdlet is called, or a
//! `Trex.Pattern` compiled once with New-TrexPattern and passed again. Either
//! reads the custom atoms in force: those declared for the session with
//! Register-TrexAtom and Import-TrexAtom, or a `Trex.Library` passed with
//! `-Library`.
//!
//! A string piped in is scanned one at a time, which composes with the rest
//! of a pipeline; a file named with `-Path` is read and scanned in one call,
//! so a large input crosses into trex once per file. A match reports its
//! offsets in UTF-16 code units, the unit a .NET string is indexed by, and
//! each register's text beside its value parsed as its kind.
//!
//! The Measure-Trex cmdlets read an input's structure with no pattern, one
//! axis each: magnitude, stress, flow, observation, echo, orbit, shape,
//! relation, spectral and seam. Each writes a report per input.
//!
//! Beside the patterns: New-TrexGrammar and Invoke-TrexGrammar parse text
//! against a grammar over the same tokens; New-TrexBpe learns a byte-pair
//! subword tokenizer and ConvertTo-TrexBpe splits text with one; and
//! New-TrexPrefilter and Test-TrexPrefilter answer whether a literal might
//! occur in a corpus without reading it again. Get-TrexFile lists the files
//! a scan would read, and Get-TrexLine an input's first lines, its last or a
//! range of them, as the trex command's head, tail and lines read them.
//!
//! A failure from a cmdlet is an error record, non-terminating unless the
//! cmdlet cannot go on; from a method it is an exception.

mod atoms;
mod axes;
mod bpe;
mod common;
mod explore;
mod follow;
mod grammar;
mod group;
mod index;
mod lines;
mod matching;
mod notes;
mod pattern;
mod prefilter;
mod query;
mod recurrence;
mod rules;
mod settings;
mod stream;
mod structure;
mod transform;
mod walk;
mod window;

pwrs::export_module! {
    name: "Trex",
    cmdlets: [
        pattern::NewTrexPattern,
        matching::SelectTrexMatch,
        matching::TestTrexMatch,
        walk::GetTrexFile,
        walk::GetTrexFileType,
        lines::GetTrexLine,
        stream::NewTrexStreamScanner,
        transform::EditTrexText,
        transform::ProtectTrexText,
        group::GroupTrexMatch,
        explore::GetTrexToken,
        explore::GetTrexRecord,
        explore::GetTrexRecordShape,
        explore::ConvertToTrexPattern,
        explore::ConvertFromTrexText,
        explore::ConvertToTrexLiteral,
        query::FindTrexRecord,
        index::NewTrexIndex,
        index::GetTrexIndex,
        rules::GetTrexRule,
        rules::InvokeTrexRule,
        rules::ConvertToTrexSarif,
        axes::MeasureTrexMagnitude,
        axes::MeasureTrexStress,
        axes::MeasureTrexFlow,
        axes::MeasureTrexObservation,
        axes::MeasureTrexGravity,
        axes::MeasureTrexContext,
        recurrence::MeasureTrexEcho,
        recurrence::MeasureTrexOrbit,
        recurrence::MeasureTrexShape,
        structure::MeasureTrexRelation,
        structure::MeasureTrexSpectral,
        structure::MeasureTrexSeam,
        grammar::NewTrexGrammar,
        grammar::InvokeTrexGrammar,
        bpe::NewTrexBpe,
        bpe::ImportTrexBpe,
        bpe::ExportTrexBpe,
        bpe::ConvertToTrexBpe,
        prefilter::NewTrexPrefilter,
        prefilter::TestTrexPrefilter,
        atoms::RegisterTrexAtom,
        atoms::ImportTrexAtom,
        atoms::GetTrexAtom,
        atoms::UnregisterTrexAtom,
        atoms::TestTrexAtom,
        atoms::NewTrexLibrary,
        settings::GetTrexClock,
        settings::SetTrexClock,
        settings::GetTrexInfo,
    ],
    classes: [
        pattern::TrexPattern,
        pattern::TrexMatch,
        pattern::TrexCapture,
        pattern::TrexExplainedToken,
        pattern::TrexReading,
        pattern::TrexExplanation,
        matching::TrexMatchCount,
        matching::TrexScanRoute,
        matching::TrexScanStats,
        walk::TrexFile,
        walk::TrexFileType,
        lines::TrexLine,
        stream::TrexStreamMatch,
        stream::TrexStreamScanner,
        explore::TrexToken,
        explore::TrexRecord,
        explore::TrexRecordShape,
        explore::TrexBuiltField,
        explore::TrexBuiltShape,
        explore::TrexBuiltPattern,
        query::TrexRecordHit,
        index::TrexIndexInfo,
        rules::TrexRule,
        rules::TrexRegion,
        rules::TrexFinding,
        axes::TrexMagnitudeFrame,
        axes::TrexMagnitudeReport,
        axes::TrexStressFrame,
        axes::TrexStressReport,
        axes::TrexFlowFrame,
        axes::TrexAnalyticFrame,
        axes::TrexFlowReport,
        axes::TrexObservationFrame,
        axes::TrexObservationReport,
        axes::TrexGravityFrame,
        axes::TrexGravityClass,
        axes::TrexGravityReport,
        axes::TrexContextMagnitude,
        axes::TrexContextStress,
        axes::TrexContextSpectral,
        axes::TrexContextEcho,
        axes::TrexContextObservation,
        axes::TrexContextSeam,
        axes::TrexContextFlow,
        axes::TrexContextFrame,
        axes::TrexContextAlignment,
        axes::TrexContextAgreement,
        axes::TrexContextReport,
        recurrence::TrexEchoFrame,
        recurrence::TrexEcho,
        recurrence::TrexEchoReport,
        recurrence::TrexOrbitToken,
        recurrence::TrexOrbitClass,
        recurrence::TrexSegment,
        recurrence::TrexOrbitReport,
        recurrence::TrexShapeFrame,
        recurrence::TrexShapeRegion,
        recurrence::TrexShapeReport,
        structure::TrexRelationFrame,
        structure::TrexRelationEdge,
        structure::TrexRelationChord,
        structure::TrexRelationReport,
        structure::TrexSpectralFrame,
        structure::TrexSpectralPeriod,
        structure::TrexTextureRegion,
        structure::TrexClassifiedRegion,
        structure::TrexSpectralReport,
        structure::TrexSeamFrame,
        structure::TrexSeamReport,
        grammar::TrexGrammar,
        grammar::TrexParseNode,
        grammar::TrexTiling,
        bpe::TrexBpe,
        prefilter::TrexPrefilter,
        prefilter::TrexLiteralTest,
        prefilter::TrexFilterCheck,
        atoms::TrexLibrary,
        atoms::TrexAtom,
        atoms::TrexAtomTest,
        settings::TrexClock,
        settings::TrexInfo,
    ],
    enums: [
        matching::SortKey,
        matching::Backend,
        matching::ColorDepth,
        matching::ValueSpelling,
        matching::DurationUnit,
        prefilter::FilterKind,
        atoms::AtomForm,
        settings::DateOrder,
        group::GroupOrder,
        group::PercentileMethod,
        rules::Severity,
        axes::FlowSignal,
        axes::FlowDirection,
        axes::TrexGrain,
        axes::TrexContextFold,
        structure::RelationKind,
        structure::Texture,
        structure::RegionKind,
    ],
    on_import: notes::keep_notes,
    on_remove: notes::release_notes,
}
