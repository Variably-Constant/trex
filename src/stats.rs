//! What scans read and found, as the `trex` command's `--stats`, PowerShell's
//! `-Stats` and Python's `trex.scan_stats()` report it: the counts each input
//! adds, and what TREX's trace kept of the lexes and of the rungs of the scan
//! ladder that answered.

use std::time::{Duration, Instant};

use crate::Match;
use crate::files::LineIndex;

/// How many inputs one rung of the scan ladder answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// The rung, as `--stats` names it.
    pub rung: String,
    /// How many inputs it answered.
    pub inputs: u64,
}

/// What scans read and found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanStats {
    /// The matches found, or under a record query the records kept.
    pub matches: u64,
    /// The lines those matches touch, each line once.
    pub matched_lines: u64,
    /// The inputs holding a match.
    pub files_with_matches: u64,
    /// The inputs scanned.
    pub files_searched: u64,
    /// The bytes the scans read.
    pub bytes_searched: u64,
    /// The time the scans took.
    pub searching: Duration,
    /// The time from the start of the work counted to its end.
    pub elapsed: Duration,
    /// The tokens the lexes produced.
    pub tokens_lexed: u64,
    /// The time those lexes took, summed over the cores that ran them, which
    /// comes out of `searching` rather than adding to it.
    pub lexing: Duration,
    /// The bytes handed to the device.
    pub device_bytes: u64,
    /// How many inputs each rung of the scan ladder answered, in the order
    /// first seen.
    pub routes: Vec<Route>,
}

impl ScanStats {
    /// Count one scanned input: `index` places `matches` in it, and the scan
    /// read `scanned` of its bytes. A match touches every line from the one it
    /// starts on to the one holding its last byte.
    pub fn count<'m>(&mut self, index: &LineIndex, scanned: usize, matches: impl IntoIterator<Item = &'m Match>) {
        let matches: Vec<&Match> = matches.into_iter().collect();
        self.add_input(scanned, !matches.is_empty());
        self.matches += matches.len() as u64;
        let untouched = crate::report::untouched_lines(matches.iter().copied(), index).len();
        self.matched_lines += index.lines().saturating_sub(untouched) as u64;
    }

    /// Count one queried input: `index` places `records`, the spans a query
    /// kept, and the scan read `scanned` of its bytes. A record covers every
    /// line from the one it starts on to the one holding its last byte.
    pub fn count_records(&mut self, index: &LineIndex, scanned: usize, records: &[(usize, usize)]) {
        self.add_input(scanned, !records.is_empty());
        self.matches += records.len() as u64;
        let mut covered = vec![false; index.lines()];
        for &(start, end) in records {
            let first = index.line_of(start);
            let last = index.line_of(end.saturating_sub(1).max(start));
            for slot in covered.iter_mut().take(last + 1).skip(first) {
                *slot = true;
            }
        }
        self.matched_lines += covered.iter().filter(|&&c| c).count() as u64;
    }

    fn add_input(&mut self, scanned: usize, matched: bool) {
        self.files_searched += 1;
        self.bytes_searched += scanned as u64;
        if matched {
            self.files_with_matches += 1;
        }
    }

    /// Take what TREX's trace kept since it was last taken: the tokens lexed
    /// and the time the lexes took, and each rung of the scan ladder that
    /// answered, summed by rung, with the bytes the device was handed. The
    /// trace keeps these only while it records, as a [`Counting`] or
    /// [`crate::trace::record`] has it do.
    pub fn read_trace(&mut self) {
        let (tokens, lexing) = crate::trace::take_lexing();
        self.tokens_lexed += tokens;
        self.lexing += lexing;
        for total in crate::trace::take_totals() {
            if total.ladder != "scan" {
                continue;
            }
            if total.rung.contains("device") {
                self.device_bytes += total.bytes;
            }
            match self.routes.iter_mut().find(|r| r.rung == total.rung) {
                Some(route) => route.inputs += total.calls,
                None => self.routes.push(Route { rung: total.rung, inputs: total.calls }),
            }
        }
    }

    /// `searching` less `lexing`: the time left for matching.
    #[must_use]
    pub fn matching(&self) -> Duration {
        self.searching.saturating_sub(self.lexing)
    }
}

/// Scans being counted: TREX's trace cleared and recording, and a clock
/// running, from [`Counting::start`] to [`Counting::finish`].
///
/// The trace counts everything TREX does in the process while it records, so
/// what a finished count reads from it is the counted scans' own whenever one
/// thread scans at a time.
pub struct Counting {
    started: Instant,
    recording: Option<crate::trace::Recording>,
}

impl Counting {
    /// Clear the trace, start it recording and start the clock.
    #[must_use]
    pub fn start() -> Counting {
        crate::trace::clear();
        Counting { started: Instant::now(), recording: Some(crate::trace::Recording::start()) }
    }

    /// Add the time since [`Counting::start`] to `stats` as searching and
    /// elapsed time, stop recording, and read what the trace kept into it.
    pub fn finish(mut self, stats: &mut ScanStats) {
        let took = self.started.elapsed();
        self.recording = None;
        stats.searching += took;
        stats.elapsed += took;
        stats.read_trace();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(pattern: &str, text: &[u8]) -> Vec<Match> {
        let pat = crate::parse(pattern).expect("valid pattern");
        crate::captures(&pat, text, &crate::scan(&pat, text))
    }

    #[test]
    fn an_input_counts_its_matches_the_lines_they_touch_and_its_bytes() {
        let text = b"a 1 2\nb\nc 3\n";
        let mut stats = ScanStats::default();
        stats.count(&LineIndex::new(text), text.len(), &found(r"\N", text));
        assert_eq!(
            (stats.matches, stats.matched_lines, stats.files_with_matches, stats.files_searched, stats.bytes_searched),
            (3, 2, 1, 1, 12)
        );
        stats.count(&LineIndex::new(b"none"), 4, &[]);
        assert_eq!((stats.files_with_matches, stats.files_searched, stats.bytes_searched), (1, 2, 16));
    }

    #[test]
    fn a_match_over_several_lines_touches_each_of_them_once() {
        let text = b"a\nb\nc d\n";
        let mut stats = ScanStats::default();
        stats.count(&LineIndex::new(text), text.len(), &found(r#""a" .* "c""#, text));
        assert_eq!((stats.matches, stats.matched_lines), (1, 3));
    }

    #[test]
    fn a_record_covers_its_lines_and_two_records_on_one_line_count_it_once() {
        let text = b"x = 1\nfoo(3)\ny = 3\n";
        let mut stats = ScanStats::default();
        stats.count_records(&LineIndex::new(text), text.len(), &[(0, 5), (13, 18)]);
        assert_eq!((stats.matches, stats.matched_lines, stats.files_with_matches), (2, 2, 1));
        let mut paired = ScanStats::default();
        paired.count_records(&LineIndex::new(text), text.len(), &[(0, 1), (4, 5)]);
        assert_eq!((paired.matches, paired.matched_lines), (2, 1));
    }

    // What the trace kept is not asserted here: the trace is the process's, and
    // the suite's other tests scan on other threads while this one counts. The
    // surfaces' own tests read it, each in a process of its own.
    #[test]
    fn a_count_times_the_scans_and_matching_is_what_lexing_leaves() {
        let text = b"order 42 shipped and 7 more";
        let mut stats = ScanStats::default();
        let counting = Counting::start();
        let matches = found(r"\N \W", text);
        counting.finish(&mut stats);
        stats.count(&LineIndex::new(text), text.len(), &matches);
        assert_eq!(stats.matches, 2);
        assert_eq!(stats.searching, stats.elapsed);
        assert_eq!(stats.matching(), stats.searching.saturating_sub(stats.lexing));
    }
}
