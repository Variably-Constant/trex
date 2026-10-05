//! Scanning input that arrives in pieces. A stream scanner takes each chunk
//! as it comes and hands back the matches no later chunk can change, then
//! at the end the rest, so the matches over a stream are the ones a scan of
//! the whole input finds, with only the undecided tail of the stream held.

use pwrs::prelude::*;

use crate::common::{arg_err, parse_err, units_of};

/// One match a stream scanner committed.
#[psclass(name = "Trex.StreamMatch")]
#[derive(Clone, Default)]
pub struct TrexStreamMatch {
    /// Where the match starts in the whole stream, in UTF-16 code units.
    pub start: i64,
    /// How many UTF-16 code units it spans.
    pub length: i64,
    /// The matched text.
    pub text: String,
    /// The pattern of a set that made the match, as it was given; empty for
    /// a scanner of one pattern.
    pub pattern: String,
}

/// A scan fed a stream a chunk at a time.
#[psclass(name = "Trex.StreamScanner", mode = proxy)]
pub struct TrexStreamScanner {
    /// The patterns scanned, as given, one a line.
    pub source: String,
    #[psfield(skip)]
    scanner: Option<trex::streaming::StreamScanner>,
    #[psfield(skip)]
    names: Vec<String>,
    #[psfield(skip)]
    held: Vec<u8>,
    #[psfield(skip)]
    held_at: usize,
    #[psfield(skip)]
    units_at: i64,
}

impl TrexStreamScanner {
    /// A scanner of `patterns`, each compiled with the atoms TREX ships: one
    /// pattern alone, or several as one set whose every match names its
    /// member.
    fn over(patterns: &[String]) -> PsResult<Self> {
        let mut compiled = Vec::with_capacity(patterns.len());
        for source in patterns {
            compiled.push(trex::parse(source).map_err(|e| parse_err(source, &e))?);
        }
        let scanner = match compiled.len() {
            0 => return Err(arg_err("TrexNoPattern", "a pattern is required")),
            1 => trex::streaming::StreamScanner::new(compiled.remove(0)),
            _ => trex::streaming::StreamScanner::over_set(trex::PatternSet::named(compiled, patterns.to_vec())),
        };
        Ok(TrexStreamScanner {
            source: patterns.join("\n"),
            scanner: Some(scanner),
            names: if patterns.len() > 1 { patterns.to_vec() } else { Vec::new() },
            held: Vec::new(),
            held_at: 0,
            units_at: 0,
        })
    }

    /// The matches `found` as the module writes them; then the held bytes
    /// before `cut`, which no later match can reach, are let go.
    fn committed(&mut self, found: Vec<(usize, trex::Span)>, cut: usize) -> PsResult<Vec<TrexStreamMatch>> {
        let mut units = units_of(&self.held);
        let mut out = Vec::with_capacity(found.len());
        for (member, span) in found {
            let (s, e) = (span.start() - self.held_at, span.end() - self.held_at);
            let (us, ue) = (units.at(s), units.at(e));
            out.push(TrexStreamMatch {
                start: self.units_at + us as i64,
                length: (ue - us) as i64,
                text: String::from_utf8_lossy(&self.held[s..e]).into_owned(),
                pattern: match self.names.get(member) {
                    Some(name) => name.clone(),
                    None => String::new(),
                },
            });
        }
        let released = cut.saturating_sub(self.held_at).min(self.held.len());
        self.units_at += units.at(released) as i64;
        self.held.drain(..released);
        self.held_at += released;
        Ok(out)
    }

    fn finished() -> PsError {
        PsError::new(ErrorCategory::InvalidOperation, "TrexStreamFinished", "the stream is finished; begin another scanner")
    }
}

#[psmethods]
impl TrexStreamScanner {
    /// A scanner of one pattern, compiled with the atoms TREX ships;
    /// New-TrexStreamScanner takes several as a set.
    pub fn new(pattern: String) -> PsResult<Self> {
        TrexStreamScanner::over(&[pattern])
    }

    /// Feeds the next chunk of the stream and gives back the matches no
    /// later chunk can change.
    pub fn push(&mut self, chunk: String) -> PsResult<Vec<TrexStreamMatch>> {
        let Some(scanner) = self.scanner.as_mut() else {
            return Err(TrexStreamScanner::finished());
        };
        self.held.extend_from_slice(chunk.as_bytes());
        scanner.push(chunk.as_bytes());
        let found = scanner.drain_committed_with_members();
        let cut = scanner.base();
        self.committed(found, cut)
    }

    /// Ends the stream and gives back every match not given yet.
    pub fn finish(&mut self) -> PsResult<Vec<TrexStreamMatch>> {
        let Some(scanner) = self.scanner.take() else {
            return Err(TrexStreamScanner::finished());
        };
        let found = scanner.finish_with_members();
        self.committed(found, usize::MAX)
    }
}

/// Begins a scan of input that arrives in pieces: a Trex.StreamScanner
/// whose Push method takes each chunk and gives back the matches no later
/// chunk can change, and whose Finish method gives back the rest.
///
/// The matches over the whole stream are the ones Select-TrexMatch finds in
/// it at once, each at its offset into the whole stream in UTF-16 code
/// units, and the scanner holds only the part of the stream a match may
/// still reach. Several patterns scan as one set, each match naming its
/// pattern. The patterns are compiled with the atoms TREX ships, since the
/// stream is lexed as it arrives, with no declarations in force.
///
/// # Examples
/// $s = New-TrexStreamScanner '\E'
/// Get-Content ./big.log -ReadCount 1000 | ForEach-Object { $s.Push(($_ -join "`n") + "`n") }
/// $s.Finish()
#[cmdlet(verb = "New", noun = "TrexStreamScanner", alias = "New-TxStreamScanner", output = ["Trex.StreamScanner"])]
#[derive(Default)]
pub struct NewTrexStreamScanner {
    /// The patterns to scan for, as TREX source text; several scan as one
    /// set.
    #[param(mandatory, position = 0)]
    pub pattern: Vec<String>,
}

impl Cmdlet for NewTrexStreamScanner {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        ps.write(TrexStreamScanner::over(&self.pattern)?)
    }
}
