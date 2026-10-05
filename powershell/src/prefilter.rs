//! Presence prefilters: a Bloom, cuckoo or xor filter built over a corpus's
//! four-byte n-grams, which answers whether a literal might occur in it
//! without reading the corpus again. A no is exact, since a filter never
//! reports a literal that occurs as absent; a yes can be a false positive,
//! which an exact search settles. A literal shorter than one n-gram is never
//! rejected.

use pwrs::prelude::*;

use crate::axes::each_input;
use crate::common::arg_err;

/// Which filter a prefilter is.
#[psenum(name = "Trex.FilterKind")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FilterKind {
    /// A Bloom filter: each n-gram sets bits chosen by several hashes, and
    /// an unset bit proves absence.
    #[default]
    Bloom,
    /// A cuckoo filter: each n-gram a one-byte fingerprint in one of two
    /// buckets.
    Cuckoo,
    /// An xor filter: each n-gram's fingerprint the xor of three slots,
    /// filled by peeling.
    Xor,
}

impl FilterKind {
    /// The kind trex names `name`.
    fn named(name: &str) -> PsResult<FilterKind> {
        match name {
            "bloom" => Ok(FilterKind::Bloom),
            "cuckoo" => Ok(FilterKind::Cuckoo),
            "xor" => Ok(FilterKind::Xor),
            other => Err(PsError::new(
                ErrorCategory::NotImplemented,
                "TrexFilterKind",
                format!("trex reports a filter the module does not know: {other}"),
            )),
        }
    }
}

/// A filter built over a corpus.
enum Built {
    Bloom(trex::prefilter::BloomFilter),
    Cuckoo(trex::prefilter::CuckooFilter),
    Xor(trex::prefilter::XorFilter),
}

impl Built {
    fn over(kind: FilterKind, corpus: &[u8]) -> Built {
        match kind {
            FilterKind::Bloom => Built::Bloom(trex::prefilter::BloomFilter::build(corpus)),
            FilterKind::Cuckoo => Built::Cuckoo(trex::prefilter::CuckooFilter::build(corpus)),
            FilterKind::Xor => Built::Xor(trex::prefilter::XorFilter::build(corpus)),
        }
    }

    fn might_contain(&self, literal: &[u8]) -> bool {
        use trex::prefilter::Membership;
        match self {
            Built::Bloom(f) => f.might_contain(literal),
            Built::Cuckoo(f) => f.might_contain(literal),
            Built::Xor(f) => f.might_contain(literal),
        }
    }
}

/// A presence filter built over a corpus.
#[psclass(name = "Trex.Prefilter", mode = proxy)]
pub struct TrexPrefilter {
    /// Which filter it is.
    pub filter: FilterKind,
    /// The size of the corpus it was built over, in bytes as UTF-8.
    pub bytes: i64,
    #[psfield(skip)]
    built: Built,
}

impl TrexPrefilter {
    fn over(kind: FilterKind, corpus: &[u8]) -> Self {
        TrexPrefilter { filter: kind, bytes: corpus.len() as i64, built: Built::over(kind, corpus) }
    }
}

#[psmethods]
impl TrexPrefilter {
    /// Builds a Bloom filter over `corpus`; New-TrexPrefilter builds the
    /// others and reads files.
    pub fn new(corpus: String) -> PsResult<Self> {
        Ok(TrexPrefilter::over(FilterKind::Bloom, corpus.as_bytes()))
    }

    /// Whether `literal` might occur in the corpus: false is exact, true
    /// can be a false positive.
    pub fn might_contain(&self, literal: String) -> PsResult<bool> {
        Ok(self.built.might_contain(literal.as_bytes()))
    }
}

/// What a filter answers for one literal, beside what an exact search of
/// the corpus finds.
#[psclass(name = "Trex.LiteralTest")]
#[derive(Clone, Default)]
pub struct TrexLiteralTest {
    /// The literal asked about.
    pub literal: String,
    /// The filter that answered.
    pub filter: FilterKind,
    /// Whether the filter says the literal might occur; false is exact.
    pub might_occur: bool,
    /// Whether an exact search finds the literal in the corpus. A literal
    /// the filter says might occur that does not is a false positive.
    pub occurs: bool,
}

/// How one filter kept its contract over a corpus.
#[psclass(name = "Trex.FilterCheck")]
#[derive(Clone, Default)]
pub struct TrexFilterCheck {
    /// The filter probed.
    pub filter: FilterKind,
    /// The substrings of the corpus probed, each present by construction.
    pub present_probes: i64,
    /// The present probes the filter reported absent, which its contract
    /// forbids.
    pub false_negatives: i64,
    /// The byte strings probed that an exact search finds nowhere in the
    /// corpus.
    pub absent_probes: i64,
    /// The absent probes the filter rejected, each a scan it saves.
    pub absent_rejected: i64,
}

/// Adds one input to the one corpus a cmdlet reads, after a newline where
/// an input came before it.
fn gather(corpus: &mut Vec<u8>, text: &str) {
    if !corpus.is_empty() {
        corpus.push(b'\n');
    }
    corpus.extend_from_slice(text.as_bytes());
}

/// Builds a presence filter over a corpus, as an object whose MightContain
/// method answers whether a literal might occur in it.
///
/// The files named, or the strings piped in, are read as one corpus. The
/// filter keeps no copy of it, so the object leaves a yes unsettled, where
/// Test-TrexPrefilter settles each with an exact search.
///
/// # Examples
/// $seen = New-TrexPrefilter -Path ./access.log -Filter Xor
/// 'ERROR', 'CRITICAL' | Where-Object { $seen.MightContain($_) }
#[cmdlet(verb = "New", noun = "TrexPrefilter", alias = "New-TxPrefilter", default_parameter_set = "Path", output = ["Trex.Prefilter"])]
#[derive(Default)]
pub struct NewTrexPrefilter {
    /// The corpus files; wildcards expand, and every file is read into the
    /// one corpus.
    #[param(mandatory, position = 0, set = "Path")]
    pub path: Vec<String>,
    /// The corpus files, read as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The corpus as text; every string piped in is read into the one
    /// corpus.
    #[param(mandatory, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// Which filter to build: Bloom when absent.
    #[param]
    pub filter: Option<FilterKind>,
    corpus: Vec<u8>,
}

impl Cmdlet for NewTrexPrefilter {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let corpus = &mut self.corpus;
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| {
            gather(corpus, &input.text);
            Ok(())
        })
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        ps.write(TrexPrefilter::over(self.filter.unwrap_or(FilterKind::Bloom), &self.corpus))
    }
}

/// Tests literals against a presence filter built over a corpus, or with
/// -Verify every filter's contract over it.
///
/// Each literal gets a Trex.LiteralTest: whether the filter says it might
/// occur, and whether an exact search finds it, so a false positive reads as
/// one. -Verify probes all three filters with substrings of the corpus, which
/// each must report as present, and with byte strings found nowhere in it,
/// the share of which a filter rejects being the scanning it saves; it
/// writes a Trex.FilterCheck for each.
///
/// # Examples
/// Test-TrexPrefilter -Path ./access.log -Literal ERROR, CRITICAL
/// Test-TrexPrefilter -Path ./access.log -Verify
#[cmdlet(verb = "Test", noun = "TrexPrefilter", alias = "Test-TxPrefilter", default_parameter_set = "Path", output = ["Trex.LiteralTest", "Trex.FilterCheck"])]
#[derive(Default)]
pub struct TestTrexPrefilter {
    /// The corpus files; wildcards expand, and every file is read into the
    /// one corpus.
    #[param(mandatory, position = 0, set = "Path")]
    pub path: Vec<String>,
    /// The corpus files, read as written, as Get-ChildItem pipes them.
    #[param(mandatory, set = "LiteralPath", literal_path)]
    pub literal_path: Vec<String>,
    /// The corpus as text; every string piped in is read into the one
    /// corpus.
    #[param(mandatory, set = "Text", value_from_pipeline, allow_empty_string)]
    pub input_object: String,
    /// The literals to test.
    #[param]
    pub literal: Vec<String>,
    /// Which filter tests the literals: Bloom when absent.
    #[param]
    pub filter: Option<FilterKind>,
    /// Probes the contract of all three filters over the corpus.
    #[param]
    pub verify: bool,
    corpus: Vec<u8>,
}

impl Cmdlet for TestTrexPrefilter {
    fn begin(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        if self.literal.is_empty() && !self.verify {
            return Err(arg_err(
                "TrexPrefilterAsk",
                "Test-TrexPrefilter tests -Literal against a filter, or with -Verify every filter; give either",
            ));
        }
        Ok(())
    }

    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let corpus = &mut self.corpus;
        each_input(ps, &self.input_object, &self.path, &self.literal_path, |input| {
            gather(corpus, &input.text);
            Ok(())
        })
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        crate::notes::write(ps)?;
        if !self.literal.is_empty() {
            let kind = self.filter.unwrap_or(FilterKind::Bloom);
            let built = Built::over(kind, &self.corpus);
            for literal in &self.literal {
                let bytes = literal.as_bytes();
                let might_occur = built.might_contain(bytes);
                ps.write(TrexLiteralTest {
                    literal: literal.clone(),
                    filter: kind,
                    might_occur,
                    occurs: might_occur && trex::byte_simd::contains(&self.corpus, bytes),
                })?;
            }
        }
        if self.verify {
            for check in trex::prefilter::verify(&self.corpus) {
                ps.write(TrexFilterCheck {
                    filter: FilterKind::named(check.name)?,
                    present_probes: check.present as i64,
                    false_negatives: check.false_negatives as i64,
                    absent_probes: check.absent as i64,
                    absent_rejected: check.rejected as i64,
                })?;
            }
        }
        Ok(())
    }
}
